//! Concessions: the right to run a line of the map's timetable for a term, won in a tender
//! against other operators (the Busbetrieb-Simulator's map concessions; Omsi-Hub's
//! `Concessie` and `schrijfIn`).
//!
//! Every four weeks the authority puts some of the map's lines out to tender, one after the
//! other on the days of the round's first week; a line of the map can also be applied for on
//! the Lines page (its tender opens at once). A tender is an auction of a few hours of the
//! company's time (`auction`): the map's other operators - made up, each with a character,
//! the same ones every time (`Rival`) - and the player offer a sum for the concession, the
//! authority weighs it with their quality (reputation and punctuality; the incumbent knows
//! the line), and the best weighed offer when it closes wins and pays it. The player may
//! also buy the line at once for the buy-out price. A won line is run from the next day for
//! the term; towards its end the line is put out again (the renewal), and a concession not
//! won again ends: the line goes to the winner. On an easy economy map lines are taken on
//! directly and renewed by themselves; on Realistic and Hard adding a map line is applying
//! for its concession.
//!
//! The player's own lines from the line editor need no concession: the company runs them on
//! its own account and pays the authority's licence for each month instead (`LICENCE`).
//!
//! Concessions of older files were bid as a price per kilometre (`Concession::price`): the
//! day's close pays every line the reference per kilometre, and `after_day` books the
//! difference; a concession won in an auction runs at the reference.
//!
//! The company's clock (`clock`) opens and closes the tenders at their minutes and tells the
//! rivals' bids as they come; `after_day` closes what the night finds still open.

use super::auction::{self, Bidder, Character, Lot, Placed, Seen, Who};
use super::clock;
use super::dates;
use super::economy;
use super::model::{BookingKind, Cents, Company, Difficulty};
use super::network;
use super::rng::Rng;
use super::specials;
use crate::LineInfo;
use serde::{Deserialize, Serialize};

/// The authority's licence for one of the player's own lines, a month.
pub const LICENCE: Cents = 350_00;
/// A tender round lasts this many weeks.
pub const ROUND_WEEKS: i64 = 4;
/// An auction runs this many hours (from, to).
pub const HOURS: (i64, i64) = (2, 6);
/// A concession is put out again this many days before it ends.
pub const RENEW_BEFORE: i64 = 42;
/// The share of the term's compensation a line is worth to the auction (its `value`).
pub const VALUE_SHARE: f64 = 0.05;
/// The least first bid, the buy-out price and the least step, as shares of the value.
pub const RESERVE: f64 = 0.4;
pub const BUY_OUT: f64 = 1.4;
pub const STEP: f64 = 0.03;

/// A concession the company holds.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Concession {
    /// The timetable's line (its `.ttl` name) and the number shown.
    pub line: String,
    pub number: String,
    pub from: String,
    /// The last day it runs.
    pub until: String,
    /// The compensation per kilometre, of the authority's reference (1 = the reference; older
    /// files bid it).
    pub price: f64,
    /// Given without a tender (a line run before concessions, an easy economy).
    #[serde(default)]
    pub direct: bool,
}

/// A line's week, from the timetable of its seven days.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Week {
    pub tours: u32,
    /// Trips with passengers.
    pub trips: u32,
    pub km: f64,
    /// The most tours of one day (the buses the line asks for).
    pub peak: u32,
}

/// A bidder's last bid when a tender closed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RivalBid {
    pub name: String,
    #[serde(default)]
    pub price: f64,
    pub quality: f64,
    pub score: f64,
    #[serde(default)]
    pub amount: Cents,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    Won {
        score: f64,
        rivals: Vec<RivalBid>,
        /// What the concession cost.
        #[serde(default)]
        amount: Cents,
        /// Bought at the buy-out price.
        #[serde(default)]
        bought: bool,
    },
    Lost {
        score: f64,
        winner: String,
        rivals: Vec<RivalBid>,
        #[serde(default)]
        amount: Cents,
    },
    /// No bid of the player's (`winner` empty: nobody bid).
    NoBid {
        winner: String,
        #[serde(default)]
        amount: Cents,
    },
}

/// A line put out to tender.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Tender {
    pub id: u32,
    pub line: String,
    pub number: String,
    pub caption: String,
    /// The days it opens and closes on (see `opens_at`, `closes_at`).
    pub offered: String,
    pub closes: String,
    /// The term, in weeks from the day after it closes (a renewal: from the old one's end).
    pub weeks: u32,
    /// The tours and kilometres of the day it was offered.
    pub day_tours: u32,
    pub day_km: f64,
    /// Its week, once the timetable of the seven days was read.
    #[serde(default)]
    pub week: Option<Week>,
    /// A concession of the company's put out again.
    #[serde(default)]
    pub renewal: bool,
    /// The company asked for it (the lines page's "Apply").
    #[serde(default)]
    pub applied: bool,
    #[serde(default)]
    pub fee_paid: bool,
    #[serde(default)]
    pub outcome: Option<Outcome>,
    /// When the auction runs, on the company's clock (`clock::now`).
    #[serde(default)]
    pub opens_at: i64,
    #[serde(default)]
    pub closes_at: i64,
    /// What the line is worth to the auction (see `VALUE_SHARE`).
    #[serde(default)]
    pub value: Cents,
    /// The rivals taking part (`Rival::id`).
    #[serde(default)]
    pub bidders: Vec<u32>,
    /// The player's bids: when, how much.
    #[serde(default)]
    pub offers: Vec<(i64, Cents)>,
    /// The clock told its opening, and this many of its bids.
    #[serde(default)]
    pub opened: bool,
    #[serde(default)]
    pub told: usize,
}

impl Tender {
    /// Not decided yet (announced or running).
    pub fn open(&self) -> bool {
        self.outcome.is_none()
    }

    /// Taking bids at `now`.
    pub fn running(&self, now: i64) -> bool {
        self.open() && self.opens_at <= now && now < self.closes_at
    }

    pub fn lot(&self) -> Lot {
        Lot { opens: self.opens_at, closes: self.closes_at, reserve: reserve(self.value), step: step(self.value) }
    }

    pub fn buy_out(&self) -> Cents {
        round(self.value as f64 * BUY_OUT)
    }
}

/// One of the map's other operators.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Rival {
    pub id: u32,
    pub name: String,
    pub character: Character,
    pub quality: f64,
}

/// The company's concessions and the market of tenders.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Concessions {
    pub held: Vec<Concession>,
    pub tenders: Vec<Tender>,
    /// The round the market was last offered in (`week / ROUND_WEEKS`).
    pub round: i64,
    pub counter: u32,
    /// The map's other operators (made once, kept).
    #[serde(default)]
    pub rivals: Vec<Rival>,
}

/// How a difficulty runs its tenders.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rules {
    /// How many rivals bid (from, to).
    pub rivals: (i64, i64),
    /// How far the rivals go, against their character's range.
    pub factor: f64,
    /// The fee for taking part: a fixed part and one per tour of the line's day.
    pub fee_base: Cents,
    pub fee_per_tour: Cents,
    pub term_weeks: u32,
    /// Lines offered each round.
    pub offers: usize,
    /// Map lines may be taken on without a tender, and concessions renew by themselves.
    pub direct: bool,
}

pub fn rules(d: Difficulty) -> Rules {
    match d {
        Difficulty::Easy => Rules { rivals: (1, 2), factor: 0.8, fee_base: 0, fee_per_tour: 0, term_weeks: 104, offers: 4, direct: true },
        Difficulty::Realistic => Rules { rivals: (2, 3), factor: 1.0, fee_base: 1_500_00, fee_per_tour: 150_00, term_weeks: 52, offers: 3, direct: false },
        Difficulty::Hard => Rules { rivals: (3, 4), factor: 1.15, fee_base: 3_000_00, fee_per_tour: 250_00, term_weeks: 52, offers: 3, direct: false },
    }
}

/// Map lines may be added without a concession's tender (an easy economy).
pub fn may_add_directly(c: &Company) -> bool {
    rules(c.difficulty).direct
}

/// The operators a map draws its rivals from (made up).
const NAMES: [&str; 16] = [
    "Regiobus Mitte",
    "Krüger Omnibus",
    "Nordbus Linienverkehr",
    "Stadtlinie GmbH",
    "Becker Reisen",
    "Linienverkehr Süd",
    "Busbetrieb Hansen",
    "Weststadt Verkehr",
    "Ostland Mobil",
    "Talbus",
    "Kraftverkehr Lindner",
    "Omnibus Petersen",
    "Verkehrsbetrieb Auental",
    "Schulz & Söhne Reisen",
    "Kreisbus Nord",
    "Vogt Linienbus",
];

/// The map's rivals, made once per company from its map: six operators, an aggressive and a
/// cautious one, a big and a small one among them. Returns whether they were made now.
pub fn ensure_rivals(c: &mut Company) -> bool {
    if !c.concessions.rivals.is_empty() {
        return false;
    }
    let mut rng = Rng::of(&[&c.id, &c.map, "rivals"], 0);
    let mut names = NAMES.to_vec();
    let mut chars = Character::ALL.to_vec();
    chars.push(Character::ALL[rng.int(0, 3) as usize]);
    chars.push(Character::ALL[rng.int(0, 3) as usize]);
    for (k, ch) in chars.into_iter().enumerate() {
        let name = names.remove(rng.int(0, names.len() as i64 - 1) as usize);
        let (lo, hi) = ch.quality();
        c.concessions.rivals.push(Rival { id: k as u32 + 1, name: name.to_string(), character: ch, quality: rng.range(lo, hi).round() });
    }
    true
}

pub fn rival(c: &Company, id: u32) -> Option<&Rival> {
    c.concessions.rivals.iter().find(|r| r.id == id)
}

/// How the authority sees the company's quality for a tender (the incumbent knows the line).
pub fn quality(c: &Company, renewal: bool) -> f64 {
    (0.7 * c.reputation + 0.3 * c.punctuality + if renewal { 8.0 } else { 0.0 }).clamp(0.0, 100.0)
}

/// What the company's bids count for in a tender (`auction::weight`).
pub fn weight(c: &Company, t: &Tender) -> f64 {
    auction::weight(quality(c, t.renewal))
}

/// The fee for taking part in a tender (a tenth of the line's worth at most).
pub fn fee(c: &Company, t: &Tender) -> Cents {
    let r = rules(c.difficulty);
    let f = ((r.fee_base + r.fee_per_tour * t.day_tours as Cents) as f64 * c.price_index).round() as Cents;
    if t.value > 0 && f > 0 {
        f.min(round(t.value as f64 * 0.1))
    } else {
        f
    }
}

fn round(x: f64) -> Cents {
    ((x / auction::ROUND as f64).round() as Cents * auction::ROUND).max(auction::ROUND)
}

pub fn reserve(value: Cents) -> Cents {
    round(value as f64 * RESERVE)
}

pub fn step(value: Cents) -> Cents {
    round(value as f64 * STEP)
}

/// What a line is worth to the auction: its share of the term's compensation at the reference
/// (from the kilometres of a day, six such days a week), at least 2 000 €.
pub fn value_of(c: &Company, day_km: f64, weeks: u32) -> Cents {
    value_at(day_km, weeks, reference_per_km(c))
}

fn value_at(day_km: f64, weeks: u32, per_km: f64) -> Cents {
    round(day_km.max(20.0) * 6.0 * weeks as f64 * per_km * VALUE_SHARE).max(2_000_00)
}

/// The authority's reference compensation per kilometre now (cents).
pub fn reference_per_km(c: &Company) -> f64 {
    economy::compensation_per_km(&economy::rules(c.difficulty), 50.0, c.contract_index)
}

/// What a line's week brings in at a price: the authority's payment and the fares.
pub fn week_revenue(c: &Company, w: &Week, price: f64) -> Cents {
    let r = economy::rules(c.difficulty);
    let comp = w.km * reference_per_km(c) * price;
    let fares = w.km * r.passengers_per_km * r.fare as f64;
    (comp + fares).round() as Cents
}

/// A line's week from the timetable of seven days (`days`: each day's lines).
pub fn week_of(days: &[Vec<LineInfo>], line: &str) -> Week {
    let mut w = Week::default();
    for day in days {
        let Some(l) = day.iter().find(|l| l.name.eq_ignore_ascii_case(line)) else { continue };
        let runs: Vec<_> = l.tours.iter().filter(|t| t.runs).collect();
        w.tours += runs.len() as u32;
        w.peak = w.peak.max(runs.len() as u32);
        for t in runs.iter().flat_map(|t| t.trips.iter()) {
            w.km += t.km;
            if t.stops.len() >= 3 {
                w.trips += 1;
            }
        }
    }
    w
}

/// A timetable of the map is a line of the company's to run (not depot runs, empty runs,
/// specials, nor other operators' traffic - a timetable the player may not drive: `specials`).
/// The Lines page offers only these (it offered the others too, and Apply refused them, Luc).
pub fn is_line(c: &Company, l: &LineInfo) -> bool {
    specials::line_kind(l, &[&c.depot]).line()
}

fn day_figures(l: &LineInfo) -> (u32, f64) {
    let runs: Vec<_> = l.tours.iter().filter(|t| t.runs).collect();
    (runs.len() as u32, runs.iter().flat_map(|t| t.trips.iter()).map(|t| t.km).sum())
}

/// The held concession of a line.
pub fn of_line<'a>(c: &'a Company, line: &str) -> Option<&'a Concession> {
    c.concessions.held.iter().find(|h| h.line.eq_ignore_ascii_case(line))
}

/// The open tender of a line (announced or running).
pub fn open_tender<'a>(c: &'a Company, line: &str) -> Option<&'a Tender> {
    c.concessions.tenders.iter().find(|t| t.open() && t.line.eq_ignore_ascii_case(line))
}

/// Lines run before there were concessions (or added directly) get one, and concessions of
/// lines no longer run are given up (not those won for tomorrow); the map's rivals are made,
/// and tenders of older files (bid by the day) become auctions of the next hours. Returns
/// whether anything changed.
pub fn ensure(c: &mut Company) -> bool {
    let r = rules(c.difficulty);
    let mut changed = ensure_rivals(c);
    let lines = c.lines.clone();
    let today = c.date.clone();
    let n = c.concessions.held.len();
    c.concessions.held.retain(|h| lines.iter().any(|l| !l.own && l.name.eq_ignore_ascii_case(&h.line)) || dates::between(&today, &h.from) > 0);
    changed |= n != c.concessions.held.len();
    for l in lines.iter().filter(|l| !l.own) {
        if of_line(c, &l.name).is_none() {
            let until = dates::add(&c.date, r.term_weeks as i64 * 7 - 1);
            c.concessions.held.push(Concession { line: l.name.clone(), number: l.number.clone(), from: c.date.clone(), until, price: 1.0, direct: true });
            changed = true;
        }
    }
    let now = clock::now(c);
    let per_km = reference_per_km(c);
    let pool: Vec<u32> = c.concessions.rivals.iter().map(|r| r.id).collect();
    for t in c.concessions.tenders.iter_mut().filter(|t| t.open() && t.closes_at == 0) {
        t.value = value_at(t.day_km, t.weeks, per_km);
        t.opens_at = now;
        t.closes_at = now + 4 * 60;
        t.closes = clock::date_of(t.closes_at);
        t.bidders = pool.iter().copied().take(r.rivals.1 as usize).collect();
        changed = true;
    }
    changed
}

/// A new tender of a line, its auction opening at `opens_at` (minutes) for a few hours.
fn new_tender(c: &mut Company, l: &LineInfo, opens_at: i64, renewal: bool) -> u32 {
    ensure_rivals(c);
    let r = rules(c.difficulty);
    c.concessions.counter += 1;
    let id = c.concessions.counter;
    let mut rng = Rng::of(&[&c.id, "tender", &l.name], id as i64);
    let closes_at = opens_at + rng.int(HOURS.0 * 4, HOURS.1 * 4) * 15;
    let (day_tours, day_km) = day_figures(l);
    let mut pool: Vec<u32> = c.concessions.rivals.iter().map(|r| r.id).collect();
    let n = (rng.int(r.rivals.0, r.rivals.1).max(1) as usize).min(pool.len());
    let mut bidders = Vec::new();
    for _ in 0..n {
        bidders.push(pool.remove(rng.int(0, pool.len() as i64 - 1) as usize));
    }
    let value = value_of(c, day_km, r.term_weeks);
    c.concessions.tenders.push(Tender {
        id,
        line: l.name.clone(),
        number: specials::number_of(l, &[&c.depot]),
        caption: specials::caption_of(l, &[&c.depot]),
        offered: clock::date_of(opens_at),
        closes: clock::date_of(closes_at),
        weeks: r.term_weeks,
        day_tours,
        day_km,
        week: None,
        renewal,
        applied: false,
        fee_paid: false,
        outcome: None,
        opens_at,
        closes_at,
        value,
        bidders,
        offers: Vec::new(),
        opened: false,
        told: 0,
    });
    id
}

/// The market as the day `lines` were read for has it: a new round of tenders every four
/// weeks (map lines the company does not run, the player's own left out), opening one after
/// the other on the next days; renewals of the concessions ending soon; old results dropped.
/// Returns whether anything changed.
pub fn refresh(c: &mut Company, lines: &[LineInfo]) -> bool {
    let r = rules(c.difficulty);
    let mut changed = ensure(c);
    changed |= mend_tenders(c, lines);
    let today = c.date.clone();
    let now = clock::now(c);
    // renewals: on an easy economy they renew by themselves
    let ending: Vec<Concession> = c.concessions.held.iter().filter(|h| dates::between(&today, &h.until) <= RENEW_BEFORE).cloned().collect();
    for h in ending {
        if r.direct {
            if let Some(x) = c.concessions.held.iter_mut().find(|x| x.line == h.line) {
                x.until = dates::add(&x.until, r.term_weeks as i64 * 7);
                changed = true;
            }
            continue;
        }
        if open_tender(c, &h.line).is_some() {
            continue;
        }
        let Some(l) = lines.iter().find(|l| l.name.eq_ignore_ascii_case(&h.line)) else { continue };
        // (a fortnight before it ends, a week from now at the soonest)
        let day = dates::add(&h.until, -14);
        let day = if dates::between(&today, &day) < 7 { dates::add(&today, 7) } else { day };
        let mut rng = Rng::of(&[&c.id, "renewal", &h.line], dates::parse(&day).unwrap_or(0));
        new_tender(c, l, clock::moment(&day, 9 * 60 + rng.int(0, 24) * 15), true);
        changed = true;
    }
    // a new round
    let round = dates::week_of(&today) / ROUND_WEEKS;
    if c.concessions.round != round {
        c.concessions.round = round;
        changed = true;
        c.concessions.tenders.retain(|t| t.open() || dates::between(&t.closes, &today) < 56);
        let mut free: Vec<&LineInfo> = lines
            .iter()
            .filter(|l| !crate::lines::is_own_file(&l.name))
            .filter(|l| !c.lines.iter().any(|x| x.name.eq_ignore_ascii_case(&l.name)))
            .filter(|l| open_tender(c, &l.name).is_none())
            // (a line of its own: not the map's depot runs, empty runs, specials or other
            // traffic - see `specials`)
            .filter(|l| is_line(c, l))
            .collect();
        let mut rng = Rng::of(&[&c.id, "round"], round);
        let mut chosen = Vec::new();
        while chosen.len() < r.offers && !free.is_empty() {
            let k = rng.int(0, free.len() as i64 - 1) as usize;
            chosen.push(free.remove(k.min(free.len() - 1)).clone());
        }
        for (k, l) in chosen.into_iter().enumerate() {
            // one a day, in the office hours, half an hour from now at the soonest
            let at = clock::moment(&dates::add(&today, k as i64), 9 * 60 + rng.int(0, 24) * 15).max(now + 30);
            new_tender(c, &l, at, false);
        }
    }
    changed
}

/// Open tenders as `specials` reads their timetables now. Older versions offered timetables
/// that are no line of their own and named a whole-day timetable after its first trip - in Bad
/// Hügelsdorf the depot run "X" that opens "Montag - Freitag" - and the file kept both: such
/// tenders are withdrawn, the others get the number and caption of their passenger trips.
/// Returns whether anything changed.
fn mend_tenders(c: &mut Company, lines: &[LineInfo]) -> bool {
    let depot = c.depot.clone();
    let of = |name: &str| lines.iter().find(|l| l.name.eq_ignore_ascii_case(name));
    let before = c.concessions.tenders.len();
    c.concessions.tenders.retain(|t| !t.open() || of(&t.line).is_none_or(|l| specials::line_kind(l, &[&depot]).line()));
    let mut changed = before != c.concessions.tenders.len();
    for t in c.concessions.tenders.iter_mut().filter(|t| t.open()) {
        let Some(l) = of(&t.line) else { continue };
        let (number, caption) = (specials::number_of(l, &[&depot]), specials::caption_of(l, &[&depot]));
        if t.number != number || t.caption != caption {
            (t.number, t.caption) = (number, caption);
            changed = true;
        }
    }
    changed
}

/// Apply for a map line's concession (the lines page): its open tender, or a new one whose
/// auction opens now. Returns the tender's id.
pub fn apply(c: &mut Company, l: &LineInfo) -> Result<u32, &'static str> {
    if c.lines.iter().any(|x| x.name.eq_ignore_ascii_case(&l.name)) {
        return Err("The company runs this line already.");
    }
    if !is_line(c, l) {
        return Err("This timetable carries no passengers of its own: depot runs, empty runs or other traffic.");
    }
    if let Some(t) = open_tender(c, &l.name) {
        let id = t.id;
        if let Some(t) = c.concessions.tenders.iter_mut().find(|t| t.id == id) {
            t.applied = true;
        }
        return Ok(id);
    }
    let now = clock::now(c);
    let id = new_tender(c, l, now, false);
    if let Some(t) = c.concessions.tenders.iter_mut().find(|t| t.id == id) {
        t.applied = true;
    }
    Ok(id)
}

/// The rivals of a tender as its auction knows them: their limits drawn per tender (secret),
/// when they open, who keeps quiet until the end.
pub fn bidders(c: &Company, t: &Tender) -> Vec<Bidder> {
    let r = rules(c.difficulty);
    let mut nobody_opened = true;
    let mut out = Vec::new();
    for (k, id) in t.bidders.iter().enumerate() {
        let Some(rv) = rival(c, *id) else { continue };
        let mut rng = Rng::of(&[&c.id, "bidder", &rv.name], t.id as i64);
        let (lo, hi) = rv.character.limit();
        // (in whole hundreds, as the bids go)
        let limit = (t.value as f64 * rng.range(lo, hi) * r.factor / auction::ROUND as f64).floor() as Cents * auction::ROUND;
        let quiet = rng.chance(rv.character.quiet());
        let len = (t.closes_at - t.opens_at).max(30);
        let quiet_until = quiet.then(|| t.closes_at - rng.int(2, 12));
        // (the first that does not keep quiet opens; the others now and then)
        let opens = !quiet && (nobody_opened || rng.chance(0.4));
        if opens {
            nobody_opened = false;
        }
        let opening = opens.then(|| t.opens_at + rng.int(3, (len / 4).max(4)));
        out.push(Bidder { character: rv.character, limit, weight: auction::weight(rv.quality), seed: rng.next_u64() ^ k as u64, opening, quiet_until });
    }
    out
}

/// The tender's bids from its opening until `until` (minutes).
pub fn bids(c: &Company, t: &Tender, until: i64) -> Vec<Placed> {
    if t.value <= 0 || until < t.opens_at {
        return Vec::new();
    }
    auction::replay(&t.lot(), &bidders(c, t), &t.offers, weight(c, t), until)
}

/// The name a bid was placed under (the player's: empty).
pub fn bidder_name(c: &Company, t: &Tender, who: Who) -> String {
    match who {
        Who::Player => String::new(),
        Who::Rival(i) => t.bidders.get(i).and_then(|id| rival(c, *id)).map(|r| r.name.clone()).unwrap_or_default(),
    }
}

/// The least the company has to bid now to lead.
pub fn min_bid(c: &Company, t: &Tender) -> Cents {
    let placed = bids(c, t, clock::now(c));
    let rivals = bidders(c, t);
    auction::to_lead(auction::best_score(&placed, &rivals, weight(c, t)), weight(c, t), &t.lot())
}

/// The company's chance to win with a bid of `amount` now, as the player can reckon it.
pub fn chance(c: &Company, t: &Tender, amount: Cents) -> f64 {
    if amount >= t.buy_out() {
        return 1.0;
    }
    let placed = bids(c, t, clock::now(c));
    let seen: Vec<Seen> = t
        .bidders
        .iter()
        .enumerate()
        .filter_map(|(i, id)| {
            let r = rival(c, *id)?;
            let last = placed.iter().rev().find(|p| p.who == Who::Rival(i)).map(|p| p.amount);
            Some(Seen { character: r.character, weight: auction::weight(r.quality), last })
        })
        .collect();
    auction::chance(amount, weight(c, t), &seen, t.value, rules(c.difficulty).factor, &t.lot())
}

/// The fee with the first bid.
fn pay_fee(c: &mut Company, id: u32) -> Result<(), &'static str> {
    let Some(t) = c.concessions.tenders.iter().find(|t| t.id == id).cloned() else { return Err("There is no such tender.") };
    if !t.fee_paid {
        let f = fee(c, &t);
        if c.cash < f {
            return Err("Not enough cash.");
        }
        c.book(BookingKind::Concession, -f, format!("Tender line {}", t.number), false);
        if let Some(x) = c.concessions.tenders.iter_mut().find(|x| x.id == id) {
            x.fee_paid = true;
        }
    }
    Ok(())
}

/// Bid a sum on a running tender, now: it has to lead (`min_bid`), and the fee is paid with
/// the first bid. (The sum is paid when the tender is won.)
pub fn bid(c: &mut Company, id: u32, amount: Cents) -> Result<(), &'static str> {
    let now = clock::now(c);
    let Some(t) = c.concessions.tenders.iter().find(|t| t.id == id).cloned() else { return Err("There is no such tender.") };
    if now < t.opens_at && t.open() {
        return Err("This tender has not opened yet.");
    }
    if !t.running(now) {
        return Err("This tender is closed.");
    }
    if amount < min_bid(c, &t) {
        return Err("Another bid leads with more: bid at least the least shown.");
    }
    if amount >= t.buy_out() {
        return Err("That is the buy-out price: buy the line instead.");
    }
    if c.cash < amount + if t.fee_paid { 0 } else { fee(c, &t) } {
        return Err("Not enough cash.");
    }
    pay_fee(c, id)?;
    if let Some(x) = c.concessions.tenders.iter_mut().find(|x| x.id == id) {
        x.offers.retain(|o| o.0 != now);
        x.offers.push((now, amount));
    }
    Ok(())
}

/// Buy the line at once for the buy-out price (the fee too, if not paid yet): the tender is
/// won and closed; the line is run from tomorrow.
pub fn buy_out(c: &mut Company, id: u32) -> Result<Event, &'static str> {
    let now = clock::now(c);
    let Some(t) = c.concessions.tenders.iter().find(|t| t.id == id).cloned() else { return Err("There is no such tender.") };
    if !t.running(now) {
        return Err(if now < t.opens_at && t.open() { "This tender has not opened yet." } else { "This tender is closed." });
    }
    let price = t.buy_out();
    if c.cash < price + if t.fee_paid { 0 } else { fee(c, &t) } {
        return Err("Not enough cash.");
    }
    pay_fee(c, id)?;
    c.book(BookingKind::Concession, -price, format!("Concession line {} (bought)", t.number), false);
    let rivals = last_bids(c, &t, &bids(c, &t, now));
    let ev = take(c, &t);
    let score = price as f64 * weight(c, &t);
    let today = c.date.clone();
    if let Some(x) = c.concessions.tenders.iter_mut().find(|x| x.id == id) {
        x.outcome = Some(Outcome::Won { score, rivals, amount: price, bought: true });
        x.closes_at = now;
        x.closes = today;
    }
    Ok(ev)
}

/// The bidders' last bids, best first.
fn last_bids(c: &Company, t: &Tender, placed: &[Placed]) -> Vec<RivalBid> {
    let mut out: Vec<RivalBid> = t
        .bidders
        .iter()
        .enumerate()
        .filter_map(|(i, id)| {
            let r = rival(c, *id)?;
            let amount = placed.iter().rev().find(|p| p.who == Who::Rival(i))?.amount;
            Some(RivalBid { name: r.name.clone(), price: 0.0, quality: r.quality, score: amount as f64 * auction::weight(r.quality), amount })
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out
}

/// A tender won: the concession from tomorrow (the line taken on in the night), or a renewal
/// added to the old term.
fn take(c: &mut Company, t: &Tender) -> Event {
    let tomorrow = dates::add(&c.date, 1);
    if let Some(h) = c.concessions.held.iter_mut().find(|h| h.line.eq_ignore_ascii_case(&t.line)) {
        h.until = dates::add(&h.until, t.weeks as i64 * 7);
        h.price = 1.0;
        h.direct = false;
        return Event::Won { number: t.number.clone(), until: h.until.clone() };
    }
    let until = dates::add(&tomorrow, t.weeks as i64 * 7 - 1);
    c.concessions.held.push(Concession { line: t.line.clone(), number: t.number.clone(), from: tomorrow, until: until.clone(), price: 1.0, direct: false });
    Event::Won { number: t.number.clone(), until }
}

/// Close a tender at its minute: the best weighed bid wins; the company pays its bid when it
/// is the winner. Returns what came of it and for how much (None: closed already).
pub fn close(c: &mut Company, id: u32) -> Option<(Event, Cents)> {
    let t = c.concessions.tenders.iter().find(|t| t.id == id && t.open())?.clone();
    let placed = bids(c, &t, t.closes_at);
    let rivals = last_bids(c, &t, &placed);
    let mine = placed.iter().rev().find(|p| p.who == Who::Player).map(|p| p.amount as f64 * weight(c, &t)).unwrap_or(0.0);
    let (outcome, event, amount) = match placed.last() {
        Some(p) if p.who == Who::Player => {
            c.book(BookingKind::Concession, -p.amount, format!("Concession line {}", t.number), false);
            let ev = take(c, &t);
            (Outcome::Won { score: mine, rivals, amount: p.amount, bought: false }, ev, p.amount)
        }
        Some(p) => {
            let winner = bidder_name(c, &t, p.who);
            let ev = Event::Lost { number: t.number.clone(), winner: winner.clone() };
            let o = if t.offers.is_empty() { Outcome::NoBid { winner, amount: p.amount } } else { Outcome::Lost { score: mine, winner, rivals, amount: p.amount } };
            (o, ev, p.amount)
        }
        None => (Outcome::NoBid { winner: String::new(), amount: 0 }, Event::Lost { number: t.number.clone(), winner: String::new() }, 0),
    };
    if let Some(x) = c.concessions.tenders.iter_mut().find(|x| x.id == id) {
        x.outcome = Some(outcome);
    }
    Some((event, amount))
}

/// What a closed tender, a concession that ended or the licence came to, for the report.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Won { number: String, until: String },
    Lost { number: String, winner: String },
    Ended { number: String },
}

/// The night after the day `date` was closed (the company's date is the next day already;
/// what is booked here is booked on `date`): tenders that should have closed that day and did
/// not (the clock closes them at their minute) are decided, lines won are taken on for
/// tomorrow, concessions that ended given up, older concessions' prices settled with the
/// authority for the day's kilometres (`km`: each company line's), and on a month's end the
/// own lines' licences. `lines`: the timetable's lines of `date`.
pub fn after_day(c: &mut Company, date: &str, km: &[(String, f64)], lines: &[LineInfo]) -> Vec<Event> {
    let mut events = Vec::new();
    let tomorrow = c.date.clone();
    let keep = std::mem::replace(&mut c.date, date.to_string());
    let r = rules(c.difficulty);

    // older concessions' prices: the close paid the reference for every kilometre
    let per_km = economy::compensation_per_km(&economy::rules(c.difficulty), c.reputation, c.contract_index);
    for (line, k) in km {
        let Some(h) = of_line(c, line).cloned() else { continue };
        let delta = (k * per_km * (h.price - 1.0)).round() as Cents;
        if delta != 0 {
            c.book(BookingKind::Compensation, delta, format!("Line {} (concession price)", h.number), false);
        }
    }

    // the tenders that should have closed today
    let end = clock::moment(&tomorrow, 0);
    let due: Vec<(u32, bool)> = c.concessions.tenders.iter().filter(|t| t.open() && t.closes_at > 0 && t.closes_at <= end).map(|t| (t.id, t.renewal || t.applied || !t.offers.is_empty())).collect();
    for (id, tell) in due {
        if let Some((ev, _)) = close(c, id) {
            if tell || matches!(ev, Event::Won { .. }) {
                events.push(ev);
            }
        }
    }

    // lines won, run from tomorrow
    let new: Vec<Concession> = c.concessions.held.iter().filter(|h| h.from == tomorrow && !c.lines.iter().any(|l| l.name.eq_ignore_ascii_case(&h.line))).cloned().collect();
    for h in new {
        match lines.iter().find(|l| l.name.eq_ignore_ascii_case(&h.line)) {
            Some(l) => {
                let _ = network::add_line(c, l, None);
            }
            None => {
                let t = c.concessions.tenders.iter().rev().find(|t| t.line.eq_ignore_ascii_case(&h.line));
                let (caption, tours, day_km) = t.map(|t| (t.caption.clone(), t.day_tours, t.day_km)).unwrap_or_default();
                c.lines.push(super::model::CompanyLine { name: h.line.clone(), number: h.number.clone(), numbers: vec![h.number.clone()], caption, added: tomorrow.clone(), tours, km: day_km, ..Default::default() });
            }
        }
        if let Some(l) = c.lines.iter_mut().find(|l| l.name.eq_ignore_ascii_case(&h.line)) {
            l.added = tomorrow.clone();
        }
    }

    // concessions that ended today
    let ended: Vec<Concession> = c.concessions.held.iter().filter(|h| dates::between(&h.until, date) >= 0).cloned().collect();
    for h in ended {
        if r.direct {
            if let Some(x) = c.concessions.held.iter_mut().find(|x| x.line == h.line) {
                x.until = dates::add(&x.until, r.term_weeks as i64 * 7);
            }
            continue;
        }
        c.concessions.held.retain(|x| x.line != h.line);
        network::remove_line(c, &h.line);
        events.push(Event::Ended { number: h.number.clone() });
    }

    // the licences of the own lines
    if dates::last_of_month(date) {
        let own: Vec<String> = c.lines.iter().filter(|l| l.own).map(|l| l.number.clone()).collect();
        for n in own {
            let fee = (LICENCE as f64 * c.price_index).round() as Cents;
            c.book(BookingKind::Concession, -fee, format!("Line licence {n}"), false);
        }
    }

    c.date = keep;
    events
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::model::CompanyLine;
    use super::super::{found, Founding};
    use super::*;
    use crate::{StopInfo, TourInfo, TripInfo};

    /// A map line of `tours` tours of four trips each, from 06:00 hourly (ten kilometres each).
    pub(crate) fn line(name: &str, number: &str, tours: usize, runs: bool) -> LineInfo {
        let trip = |k: usize| TripInfo {
            name: format!("{name} {k}"),
            index: k + 1,
            line: number.into(),
            from: "A".into(),
            terminus: "B".into(),
            departure: 21_600.0 + k as f64 * 3600.0,
            arrival: 23_400.0 + k as f64 * 3600.0,
            stops: (0..5).map(|s| StopInfo { name: format!("S{s}"), id: s, arr: 0.0, dep: 0.0 }).collect(),
            km: 10.0,
        };
        LineInfo {
            name: name.into(),
            user_allowed: true,
            termini: vec!["A".into(), "B".into()],
            tours: (0..tours).map(|t| TourInfo { number: (t + 1).to_string(), ai_group: String::new(), first: 0.0, last: 0.0, days: "daily".into(), runs, next_run: None, trips: (0..4).map(trip).collect() }).collect(),
        }
    }

    fn company(d: Difficulty) -> Company {
        found(&Founding { name: "Tender".into(), difficulty: d, date: "2024-03-04".into(), ..Default::default() }, "Luc")
    }

    /// The nights from today until `until` (inclusive), as the day's close runs them.
    fn nights_until(c: &mut Company, until: &str, lines: &[LineInfo]) -> Vec<Event> {
        let mut out = Vec::new();
        while dates::between(&c.date, until) >= 0 {
            let date = c.date.clone();
            c.date = dates::add(&date, 1);
            out.extend(after_day(c, &date, &[], lines));
        }
        out
    }

    #[test]
    fn a_round_offers_lines_the_company_does_not_run_one_a_day() {
        let lines = vec![line("Linie5", "5", 3, true), line("Linie7", "7", 2, true), line("oo_12", "12", 1, true), line("Linie9", "9", 4, true)];
        let mut c = company(Difficulty::Realistic);
        network::add_line(&mut c, &lines[0], None).unwrap();
        assert!(refresh(&mut c, &lines));
        // (the line run already is grandfathered, the player's own one is never offered)
        assert!(of_line(&c, "Linie5").is_some_and(|h| h.direct && h.price == 1.0));
        let offered: Vec<&Tender> = c.concessions.tenders.iter().collect();
        assert_eq!(offered.len(), 2);
        assert!(offered.iter().all(|t| t.line != "Linie5" && t.line != "oo_12"));
        // each runs a few hours, on days one after the other, with rivals of the map
        for (k, t) in offered.iter().enumerate() {
            let hours = (t.closes_at - t.opens_at) as f64 / 60.0;
            assert!((2.0..=6.0).contains(&hours), "{hours}");
            assert_eq!(clock::date_of(t.opens_at), dates::add(&c.date, k as i64));
            assert!(!t.bidders.is_empty() && t.bidders.iter().all(|id| rival(&c, *id).is_some()));
            assert!(t.value >= 2_000_00 && t.buy_out() > t.value && reserve(t.value) < t.value);
        }
        assert_eq!(c.concessions.rivals.len(), 6);
        assert!(!refresh(&mut c, &lines), "the same round again changes nothing");
        // the week: five weekdays of 3 tours, a weekend without
        let mut days: Vec<Vec<LineInfo>> = (0..5).map(|_| vec![line("Linie5", "5", 3, true)]).collect();
        days.push(vec![line("Linie5", "5", 3, false)]);
        let w = week_of(&days, "Linie5");
        assert_eq!((w.tours, w.trips, w.peak), (15, 60, 3));
        assert!((w.km - 600.0).abs() < 1e-9);
    }

    #[test]
    fn a_bid_that_leads_at_the_close_wins_and_the_line_is_run_for_the_term() {
        let lines = vec![line("Linie7", "7", 2, true)];
        let mut c = company(Difficulty::Realistic);
        c.cash = 1_000_000_00;
        c.clock.minute = 10 * 60;
        let id = apply(&mut c, &lines[0]).unwrap();
        assert_eq!(apply(&mut c, &lines[0]), Ok(id), "one tender a line");
        let t = c.concessions.tenders[0].clone();
        assert!(t.running(clock::now(&c)) && t.applied);
        assert_eq!(bid(&mut c, id, reserve(t.value) - auction::ROUND), Err("Another bid leads with more: bid at least the least shown."));
        let cash = c.cash;
        // a bid past every rival's limit (below the buy-out)
        let high = t.buy_out() - auction::ROUND;
        bid(&mut c, id, high).unwrap();
        assert_eq!(cash - c.cash, fee(&c, &c.concessions.tenders[0]), "the fee with the first bid");
        assert!(fee(&c, &t) > 0 && fee(&c, &t) <= t.value / 10 + auction::ROUND, "a tenth of the worth at most");
        assert!(chance(&c, &c.concessions.tenders[0], high) > 0.5);
        let (ev, paid) = close(&mut c, id).unwrap();
        assert!(matches!(&ev, Event::Won { number, .. } if number == "7"));
        assert_eq!(paid, high);
        assert!(matches!(c.concessions.tenders[0].outcome, Some(Outcome::Won { amount, bought: false, .. }) if amount == high));
        // the line comes in the night, from tomorrow on
        assert!(!c.lines.iter().any(|l| l.name == "Linie7"));
        ensure(&mut c);
        assert!(of_line(&c, "Linie7").is_some(), "won for tomorrow: kept");
        let today = c.date.clone();
        nights_until(&mut c, &today, &lines);
        assert!(c.lines.iter().any(|l| l.name == "Linie7"));
        let h = of_line(&c, "Linie7").unwrap().clone();
        assert_eq!((h.from.as_str(), h.price), (c.date.as_str(), 1.0));
        assert_eq!(dates::between(&h.from, &h.until), 52 * 7 - 1);
        // the renewal comes up; without a bid the line ends with the term
        c.date = dates::add(&h.until, -RENEW_BEFORE);
        refresh(&mut c, &lines);
        let renewal = open_tender(&c, "Linie7").unwrap().clone();
        assert!(renewal.renewal);
        let ev = nights_until(&mut c, &h.until.clone(), &lines);
        assert!(ev.iter().any(|e| matches!(e, Event::Ended { number } if number == "7")));
        assert!(!c.lines.iter().any(|l| l.name == "Linie7"));
    }

    #[test]
    fn buying_out_is_certain_and_a_low_bid_loses_more_often() {
        let lines = vec![line("Linie7", "7", 2, true)];
        let mut c = company(Difficulty::Hard);
        c.cash = 1_000_000_00;
        c.clock.minute = 9 * 60;
        let id = apply(&mut c, &lines[0]).unwrap();
        let price = c.concessions.tenders[0].buy_out();
        let cash = c.cash;
        buy_out(&mut c, id).unwrap();
        let t = &c.concessions.tenders[0];
        assert!(matches!(t.outcome, Some(Outcome::Won { bought: true, amount, .. }) if amount == price));
        assert_eq!(cash - c.cash, price + fee(&c, t));
        assert_eq!(buy_out(&mut c, id).err(), Some("This tender is closed."));
        // the least bid against the rivals: lost more often than a high one, and a lost one
        // says who won and for how much
        let mut won = [0; 2];
        for n in 0..60 {
            for (k, share) in [0.45, 1.25].into_iter().enumerate() {
                let mut c = company(Difficulty::Realistic);
                c.id = format!("c{n}");
                c.cash = 1_000_000_00;
                c.clock.minute = 9 * 60;
                let id = apply(&mut c, &lines[0]).unwrap();
                let t = c.concessions.tenders[0].clone();
                let amount = round(t.value as f64 * share).max(min_bid(&c, &t));
                bid(&mut c, id, amount).unwrap();
                close(&mut c, id).unwrap();
                match &c.concessions.tenders[0].outcome {
                    Some(Outcome::Won { .. }) => won[k] += 1,
                    Some(Outcome::Lost { winner, amount: a, .. }) => assert!(!winner.is_empty() && *a > amount),
                    o => panic!("{o:?}"),
                }
            }
        }
        assert!(won[0] < won[1] && won[0] < 50 && won[1] > 30, "{won:?}");
    }

    #[test]
    fn depot_runs_and_specials_are_never_offered() {
        let mut depot = line("Betriebsfahrten", "X", 2, true);
        for t in depot.tours.iter_mut().flat_map(|t| t.trips.iter_mut()) {
            t.terminus = "Betriebsfahrt".into();
        }
        let shuttle = line("FFF Shuttle", "", 1, true);
        let lines = vec![depot.clone(), shuttle.clone(), line("Linie7", "7", 2, true)];
        let mut c = company(Difficulty::Hard);
        refresh(&mut c, &lines);
        assert_eq!(c.concessions.tenders.iter().map(|t| t.line.as_str()).collect::<Vec<_>>(), vec!["Linie7"]);
        assert!(apply(&mut c, &depot).is_err() && apply(&mut c, &shuttle).is_err());
    }

    #[test]
    fn tenders_of_older_files_are_read_again() {
        // Bad Hügelsdorf: the weekday timetable opens with the depot run X; an older version
        // named its tender "Line X" after it, and offered other operators' traffic too
        let mut day = line("Montag - Freitag", "301", 2, true);
        let first = &mut day.tours[0].trips[0];
        (first.line, first.name, first.terminus) = ("X".into(), "(leer) Betriebshof VBBH - Hildegardplatz".into(), "Betriebsfahrt".into());
        for t in day.tours.iter_mut().flat_map(|t| t.trips.iter_mut()).skip(1) {
            t.terminus = "Hauptbahnhof                              301".into();
        }
        let mut other = line("251", "251", 1, true);
        other.user_allowed = false;
        let lines = vec![day.clone(), other.clone()];
        let mut c = company(Difficulty::Realistic);
        c.clock.minute = 9 * 60;
        let now = clock::now(&c);
        let a = new_tender(&mut c, &day, now, false);
        let b = new_tender(&mut c, &other, now, false);
        for t in c.concessions.tenders.iter_mut() {
            (t.number, t.caption) = ("X".into(), "Betriebsfahrt – Hauptbahnhof                              301".into());
        }
        assert!(mend_tenders(&mut c, &lines));
        let t = c.concessions.tenders.iter().find(|t| t.id == a).unwrap();
        assert_eq!((t.number.as_str(), t.caption.as_str()), ("301", "Hauptbahnhof"));
        assert!(c.concessions.tenders.iter().all(|t| t.id != b), "no line of its own: withdrawn");
        assert!(!mend_tenders(&mut c, &lines), "nothing more to mend");
    }

    #[test]
    fn an_easy_economy_takes_lines_directly_and_own_lines_pay_a_licence() {
        let lines = vec![line("Linie5", "5", 3, true)];
        let mut c = company(Difficulty::Easy);
        assert!(may_add_directly(&c));
        network::add_line(&mut c, &lines[0], None).unwrap();
        c.lines.push(CompanyLine { name: "oo_1".into(), number: "1".into(), own: true, ..Default::default() });
        ensure(&mut c);
        assert_eq!(c.concessions.held.len(), 1, "the own line has none");
        let until = of_line(&c, "Linie5").unwrap().until.clone();
        nights_until(&mut c, &until, &lines);
        assert!(c.lines.iter().any(|l| l.name == "Linie5"), "renewed by itself");
        assert!(dates::between(&c.date, &of_line(&c, "Linie5").unwrap().until) > 300);
        // the month's licence
        let mut d = company(Difficulty::Realistic);
        d.lines.push(CompanyLine { name: "oo_1".into(), number: "1".into(), own: true, ..Default::default() });
        d.date = "2024-03-31".into();
        let cash = d.cash;
        let date = d.date.clone();
        d.date = dates::add(&date, 1);
        after_day(&mut d, &date, &[], &[]);
        assert_eq!(cash - d.cash, LICENCE);
        assert!(!may_add_directly(&d));
    }
}
