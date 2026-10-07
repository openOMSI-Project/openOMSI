//! Advertising on the company's buses (Luc: "reclames die je op de bus kunt zetten die je meer
//! geld opleveren als je zo'n contract aangaat ... gekoppeld aan levels ... afgesloten met een
//! contract met ondertekening"): advertisers offer contracts for a number of buses over a
//! number of months at a monthly fee a bus; the company signs one as it signs the dealer's
//! (the same paper, the same pen), and the money comes in at every month's end.
//!
//! The kinds, after what German operators sell (Verkehrsmittelwerbung, Realistic, net,
//! founding day's prices, a bus a month): a poster on the rear (*Heckwerbung*) €90, the side
//! panels (*Seitenflächen*) €220, a full wrap (*Ganzgestaltung*, the advertiser pays the foil)
//! €600. A rear advert opens at level 2, side panels at 4, full wraps at 6 (`levels::Feature`);
//! an offer of a kind not open yet is shown locked.
//!
//! What an advertiser pays follows what the buses are seen by: the passengers a bus carries a
//! day (the closed days' history), the company's reputation, and the difficulty (Easy 1.2,
//! Hard 0.85 times). The week's offers are drawn from the company and the week, as the
//! market's are. A bus carries one advert at a time; a contract ended early costs three months'
//! fee (or what was left, if less).

use super::dates;
use super::levels::{self, Feature};
use super::model::{BookingKind, Cents, Company, Difficulty};
use super::rng::Rng;
use serde::{Deserialize, Serialize};

/// Where on the bus an advert goes.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum AdKind {
    #[default]
    Rear,
    Sides,
    FullWrap,
}

impl AdKind {
    pub const ALL: [AdKind; 3] = [AdKind::Rear, AdKind::Sides, AdKind::FullWrap];

    pub fn label(self) -> &'static str {
        match self {
            AdKind::Rear => "Rear advert",
            AdKind::Sides => "Side panels",
            AdKind::FullWrap => "Full wrap",
        }
    }

    /// The level feature that opens it.
    pub fn feature(self) -> Feature {
        match self {
            AdKind::Rear => Feature::RearAdverts,
            AdKind::Sides => Feature::SideAdverts,
            AdKind::FullWrap => Feature::FullWraps,
        }
    }

    /// Why it cannot be signed yet (the company's popup says what opens it: `kit::refusal`).
    pub fn locked_reason(self) -> &'static str {
        match self {
            AdKind::Rear => "Rear adverts open at a higher company level.",
            AdKind::Sides => "Side adverts open at a higher company level.",
            AdKind::FullWrap => "Full wraps open at a higher company level.",
        }
    }

    /// What an advertiser pays for it a bus a month (Realistic, founding day's prices).
    pub fn base(self) -> Cents {
        match self {
            AdKind::Rear => 90_00,
            AdKind::Sides => 220_00,
            AdKind::FullWrap => 600_00,
        }
    }

    /// How many buses and how many months its offers ask for.
    fn sizes(self) -> (&'static [u32], &'static [u32]) {
        match self {
            AdKind::Rear => (&[1, 2, 3, 4, 6], &[6, 12]),
            AdKind::Sides => (&[1, 2, 3, 4], &[12, 24]),
            AdKind::FullWrap => (&[1, 1, 2], &[12, 24, 36]),
        }
    }
}

/// The advertisers (made up) and what they sell.
pub const ADVERTISERS: [(&str, &str); 16] = [
    ("Brauerei Hügelquell", "Brewery"),
    ("Autohaus Lindner", "Car dealer"),
    ("Möbelhaus Krone", "Furniture store"),
    ("Bäckerei Sonnenkorn", "Bakery"),
    ("Radio Welle 7", "Radio station"),
    ("Zahnarztpraxis Dr. Berger", "Dental practice"),
    ("Fitnessstudio Pulsar", "Gym"),
    ("Kinopalast Aurora", "Cinema"),
    ("Elektro Brandt", "Electronics store"),
    ("Pizzeria Bella Vista", "Restaurant"),
    ("Tierpark Waldhaus", "Zoo"),
    ("Hotel Am Markt", "Hotel"),
    ("Optik Klarsicht", "Optician"),
    ("Baumarkt Hammerwerk", "DIY store"),
    ("Stadtfest Sommerklang", "Festival"),
    ("Fahrschule Grünlicht", "Driving school"),
];

/// An advertiser's offer this week.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct AdOffer {
    /// Its number in the week (`week * 100 + k`: taken offers are known by it).
    pub no: u32,
    pub advertiser: String,
    pub trade: String,
    pub kind: AdKind,
    pub buses: u32,
    pub months: u32,
    /// A bus a month.
    pub per_bus: Cents,
}

impl AdOffer {
    pub fn monthly(&self) -> Cents {
        self.per_bus * self.buses as Cents
    }

    pub fn total(&self) -> Cents {
        self.monthly() * self.months as Cents
    }
}

/// An advertising contract: the offer as signed, the buses that carry it, from when to when,
/// and the signature.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct AdContract {
    /// Given when it is signed.
    pub no: u32,
    pub offer: u32,
    pub advertiser: String,
    pub trade: String,
    pub kind: AdKind,
    pub per_bus: Cents,
    pub months: u32,
    pub buses: Vec<u32>,
    pub from: String,
    pub until: String,
    pub signed_by: String,
    pub strokes: Vec<Vec<[f32; 2]>>,
    pub signed_at: String,
    /// Months paid so far, and when it ended (ran out or ended early).
    pub paid_months: u32,
    pub ended: Option<String>,
}

impl AdContract {
    pub fn is_signed(&self) -> bool {
        !self.signed_by.trim().is_empty() || self.strokes.iter().any(|s| s.len() > 1)
    }

    pub fn monthly(&self) -> Cents {
        self.per_bus * self.buses.len() as Cents
    }

    pub fn running(&self) -> bool {
        self.ended.is_none()
    }

    pub fn months_left(&self) -> u32 {
        self.months.saturating_sub(self.paid_months)
    }
}

/// The company's advertising: its contracts, and the offers of the week signed already.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Adverts {
    pub contracts: Vec<AdContract>,
    pub counter: u32,
    pub taken: Vec<u32>,
}

fn factor(d: Difficulty) -> f64 {
    match d {
        Difficulty::Easy => 1.2,
        Difficulty::Realistic => 1.0,
        Difficulty::Hard => 0.85,
    }
}

/// What the buses are seen by: the passengers a bus carried a day over the last closed days
/// (up to thirty), against the 250 that make the base price; 0.7 to 1.4.
pub fn reach(c: &Company) -> f64 {
    let days: Vec<_> = c.history.iter().rev().take(30).collect();
    let buses = c.fleet.iter().filter(|v| v.held_on(&c.date)).count().max(1);
    if days.is_empty() {
        return 0.8;
    }
    let per_bus = days.iter().map(|d| d.passengers as f64).sum::<f64>() / days.len() as f64 / buses as f64;
    (0.6 + 0.4 * per_bus / 250.0).clamp(0.7, 1.4)
}

/// An advertiser's price for an advert of `kind` on a bus of the company a month.
pub fn price(c: &Company, kind: AdKind) -> Cents {
    let rep = 0.85 + 0.3 * c.reputation.clamp(0.0, 100.0) / 100.0;
    let euros = kind.base() as f64 / 100.0 * reach(c) * rep * factor(c.difficulty) * c.price_index;
    (euros.round() as Cents) * 100
}

/// The week's offers: two, and one more at levels 3, 5 and 7; each of a kind (one not open
/// yet too - it is shown locked), drawn from the company and the week. Those signed this
/// week are gone.
pub fn offers(c: &Company) -> Vec<AdOffer> {
    let week = dates::week_of(&c.date);
    let mut rng = Rng::of(&[&c.id, "adverts"], week);
    let level = levels::level(c);
    let n = 2 + [3, 5, 7].iter().filter(|l| level >= **l).count();
    let mut out = Vec::new();
    for k in 0..n {
        // (the kinds open now most of the time; the next one now and then, to look forward to)
        let open: Vec<AdKind> = AdKind::ALL.iter().copied().filter(|a| levels::unlocked(c, a.feature())).collect();
        let next = AdKind::ALL.iter().copied().find(|a| !levels::unlocked(c, a.feature()));
        let kind = match (open.is_empty(), next) {
            (true, Some(nx)) => nx,
            (_, Some(nx)) if rng.chance(0.25) => nx,
            _ => *rng.pick(&open).unwrap_or(&AdKind::Rear),
        };
        let (who, what) = *rng.pick(&ADVERTISERS).unwrap_or(&ADVERTISERS[0]);
        let (sizes, terms) = kind.sizes();
        let buses = *rng.pick(sizes).unwrap_or(&1);
        let months = *rng.pick(terms).unwrap_or(&12);
        let noise = rng.range(0.9, 1.1);
        let per_bus = ((price(c, kind) as f64 * noise / 100.0).round() as Cents) * 100;
        let no = (week.rem_euclid(1_000_000) as u32) * 100 + k as u32;
        if c.adverts.taken.contains(&no) {
            continue;
        }
        out.push(AdOffer { no, advertiser: who.to_string(), trade: what.to_string(), kind, buses, months, per_bus });
    }
    out
}

/// The contract running on a bus (a bus carries one advert at a time).
pub fn advert_of(c: &Company, vehicle: u32) -> Option<&AdContract> {
    c.adverts.contracts.iter().find(|k| k.running() && k.buses.contains(&vehicle))
}

/// The buses that could carry a new advert: the company's, without one, those that run most
/// first (by their kilometres).
pub fn free_buses(c: &Company) -> Vec<u32> {
    let mut v: Vec<&super::model::Vehicle> = c.fleet.iter().filter(|v| v.held_on(&c.date) && advert_of(c, v.id).is_none()).collect();
    v.sort_by(|a, b| b.km.total_cmp(&a.km).then(a.id.cmp(&b.id)));
    v.into_iter().map(|v| v.id).collect()
}

/// `date` moved on by `n` months (the day kept, or the month's last).
pub fn add_months(date: &str, n: u32) -> String {
    let Some(d) = dates::parse(date) else { return date.to_string() };
    let (y, m, day) = dates::civil_from_days(d);
    let months = y as i64 * 12 + (m as i64 - 1) + n as i64;
    let (y2, m2) = ((months / 12) as i32, (months % 12) as u32 + 1);
    dates::fmt(dates::days_from_civil(y2, m2, day.min(dates::days_in_month(y2, m2))))
}

/// The contract of an offer, to sign: the buses that would carry it chosen, from today.
pub fn draft(c: &Company, o: &AdOffer) -> Result<AdContract, &'static str> {
    if !levels::unlocked(c, o.kind.feature()) {
        return Err(o.kind.locked_reason());
    }
    let free = free_buses(c);
    if free.len() < o.buses as usize {
        return Err("The company has too few buses without an advert for this contract.");
    }
    Ok(AdContract {
        no: 0,
        offer: o.no,
        advertiser: o.advertiser.clone(),
        trade: o.trade.clone(),
        kind: o.kind,
        per_bus: o.per_bus,
        months: o.months,
        buses: free.into_iter().take(o.buses as usize).collect(),
        from: c.date.clone(),
        until: add_months(&c.date, o.months),
        signed_by: String::new(),
        strokes: Vec::new(),
        signed_at: String::new(),
        paid_months: 0,
        ended: None,
    })
}

/// Sign a contract (`draft`'s, signed): the company's, its buses carry the advert from now.
/// Returns its number.
pub fn sign(c: &mut Company, k: &AdContract) -> Result<u32, &'static str> {
    if !k.is_signed() {
        return Err("Sign the contract first.");
    }
    if !levels::unlocked(c, k.kind.feature()) {
        return Err(k.kind.locked_reason());
    }
    if c.adverts.taken.contains(&k.offer) {
        return Err("This offer is signed already.");
    }
    if k.buses.is_empty() || k.buses.iter().any(|id| c.vehicle(*id).is_none_or(|v| !v.held_on(&c.date)) || advert_of(c, *id).is_some()) {
        return Err("The company has too few buses without an advert for this contract.");
    }
    c.adverts.counter += 1;
    let no = c.adverts.counter;
    let date = c.date.clone();
    c.adverts.contracts.push(AdContract { no, signed_at: date, ended: None, paid_months: 0, ..k.clone() });
    c.adverts.taken.push(k.offer);
    // (the week's taken offers are kept for a while, no longer)
    if c.adverts.taken.len() > 200 {
        let extra = c.adverts.taken.len() - 200;
        c.adverts.taken.drain(..extra);
    }
    Ok(no)
}

/// What ending a contract early costs: three months' fee, or what was left if less.
pub fn penalty(k: &AdContract) -> Cents {
    k.monthly() * k.months_left().min(3) as Cents
}

/// End a contract early: its penalty booked, its buses free again. Returns the penalty.
pub fn end_early(c: &mut Company, no: u32) -> Result<Cents, &'static str> {
    let Some(k) = c.adverts.contracts.iter().find(|k| k.no == no && k.running()).cloned() else { return Err("There is no such contract.") };
    let fee = penalty(&k);
    if c.cash < fee {
        return Err("Not enough cash.");
    }
    c.book(BookingKind::Advertising, -fee, format!("Advert contract {} ({}) ended early", k.no, k.advertiser), false);
    let date = c.date.clone();
    if let Some(x) = c.adverts.contracts.iter_mut().find(|x| x.no == no) {
        x.ended = Some(date);
    }
    Ok(fee)
}

/// On a month's last day: every running contract's fee for its buses still in the fleet, and
/// those whose months are paid end.
pub fn month_end(c: &mut Company) {
    let date = c.date.clone();
    let running: Vec<AdContract> = c.adverts.contracts.iter().filter(|k| k.running()).cloned().collect();
    for k in running {
        // (a contract signed this month pays from the next one on)
        if k.paid_months == 0 && dates::month_of(&k.signed_at) == dates::month_of(&date) {
            continue;
        }
        let carried = k.buses.iter().filter(|id| c.vehicle(**id).is_some_and(|v| v.held_on(&date))).count() as Cents;
        let fee = k.per_bus * carried;
        c.book(BookingKind::Advertising, fee, format!("Advert contract {} ({})", k.no, k.advertiser), false);
        if let Some(x) = c.adverts.contracts.iter_mut().find(|x| x.no == k.no) {
            x.paid_months += 1;
            if x.paid_months >= x.months {
                x.ended = Some(date.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::market::{self, MarketBus, Payment};
    use super::super::{found, Founding};
    use super::*;

    fn company(level_xp: i64, buses: usize) -> Company {
        let mut c = found(&Founding { name: "Werbung".into(), date: "2024-03-04".into(), ..Default::default() }, "Luc");
        c.cash += 5_000_000_00;
        let bus = MarketBus { file: "Vehicles/Citaro/Citaro.bus".into(), name: "Citaro".into(), ..Default::default() };
        for _ in 0..buses {
            market::buy_new(&mut c, &bus, Payment::Cash, "").unwrap();
        }
        c.progress.xp = level_xp;
        c
    }

    fn signed(mut k: AdContract) -> AdContract {
        k.signed_by = "Luc".into();
        k
    }

    #[test]
    fn the_offers_follow_the_level_and_the_week() {
        let start = company(0, 3);
        assert_eq!(levels::level(&start), 1);
        let o = offers(&start);
        assert_eq!(o.len(), 2);
        assert_eq!(o, offers(&start), "the same week, the same offers");
        // at level 1 nothing is open: the rear adverts are shown, locked
        assert!(o.iter().all(|x| x.kind == AdKind::Rear));
        assert_eq!(draft(&start, &o[0]), Err("Rear adverts open at a higher company level."));
        // at level 7: more offers, the kinds open
        let big = company(levels::LEVEL_XP[6], 6);
        assert_eq!(offers(&big).len(), 5);
        assert!(levels::unlocked(&big, Feature::FullWraps) && levels::unlocked(&big, Feature::SideAdverts));
        // the price: a rear advert about €90 a bus, a full wrap more
        assert!((70_00..=110_00).contains(&price(&start, AdKind::Rear)), "{}", price(&start, AdKind::Rear));
        assert!(price(&start, AdKind::FullWrap) > 4 * price(&start, AdKind::Rear));
        let mut famous = start.clone();
        famous.reputation = 100.0;
        assert!(price(&famous, AdKind::Sides) > price(&start, AdKind::Sides));
    }

    #[test]
    fn a_contract_is_signed_paid_monthly_and_ends() {
        let mut c = company(levels::LEVEL_XP[1], 3);
        let o = AdOffer { no: 77, advertiser: "Bäckerei Sonnenkorn".into(), trade: "Bakery".into(), kind: AdKind::Rear, buses: 2, months: 2, per_bus: 90_00 };
        let k = draft(&c, &o).unwrap();
        assert_eq!((k.buses.len(), k.until.as_str()), (2, "2024-05-04"));
        assert_eq!(sign(&mut c, &k), Err("Sign the contract first."));
        let no = sign(&mut c, &signed(k.clone())).unwrap();
        assert_eq!(sign(&mut c, &signed(k.clone())), Err("This offer is signed already."));
        assert!(offers(&c).iter().all(|x| x.no != 77));
        let bus = c.adverts.contracts[0].buses[0];
        assert_eq!(advert_of(&c, bus).map(|x| x.no), Some(no));
        assert_eq!(free_buses(&c).len(), 1);
        // a full wrap is not open at level 2; three buses are too many now
        let full = AdOffer { kind: AdKind::FullWrap, buses: 1, no: 78, ..o.clone() };
        assert_eq!(draft(&c, &full), Err("Full wraps open at a higher company level."));
        assert!(draft(&c, &AdOffer { buses: 2, no: 79, ..o.clone() }).is_err());
        // the month signed in pays nothing; the next two months pay, then it ends
        c.date = "2024-03-31".into();
        month_end(&mut c);
        assert_eq!(c.adverts.contracts[0].paid_months, 0);
        let cash = c.cash;
        c.date = "2024-04-30".into();
        month_end(&mut c);
        assert_eq!(c.cash - cash, 180_00);
        assert!(c.ledger.last().is_some_and(|b| b.kind == BookingKind::Advertising && !b.kind.is_capital()));
        c.date = "2024-05-31".into();
        month_end(&mut c);
        assert!(!c.adverts.contracts[0].running());
        assert!(advert_of(&c, bus).is_none() && free_buses(&c).len() == 3);
    }

    #[test]
    fn ending_early_costs_up_to_three_months() {
        let mut c = company(levels::LEVEL_XP[3], 2);
        let o = AdOffer { no: 5, advertiser: "Radio Welle 7".into(), trade: "Radio station".into(), kind: AdKind::Sides, buses: 1, months: 24, per_bus: 200_00 };
        let k = signed(draft(&c, &o).unwrap());
        let no = sign(&mut c, &k).unwrap();
        assert_eq!(penalty(&c.adverts.contracts[0]), 600_00);
        let cash = c.cash;
        assert_eq!(end_early(&mut c, no), Ok(600_00));
        assert_eq!(c.cash, cash - 600_00);
        assert_eq!(end_early(&mut c, no), Err("There is no such contract."));
        // nearly over: what is left
        let k = AdContract { per_bus: 100_00, buses: vec![1], months: 12, paid_months: 11, ..Default::default() };
        assert_eq!(penalty(&k), 100_00);
        assert_eq!(add_months("2024-01-31", 1), "2024-02-29");
        assert_eq!(add_months("2024-11-15", 3), "2025-02-15");
    }
}
