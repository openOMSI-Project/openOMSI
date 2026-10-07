//! The vehicle market: new buses (every bus installed, at the price of its kind), used ones
//! (offers of the week, older and worn), leasing and short-term rental, and selling. What
//! kind a bus is - its size and its drive - is guessed from its files: an articulated bus has
//! a trailer section (`[couple_back]`), an electric one says so in its name or in the names
//! of its scripts and sounds, a double-decker is tall and a midibus short.

use super::dates;
use super::economy::{self, Rules};
use super::finance;
use super::model::{BookingKind, BusKind, BusSize, Cents, Company, Drive, Tenure, Vehicle};
use super::rng::Rng;
use serde::{Deserialize, Serialize};

/// A bus as the market offers it: an installed bus.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct MarketBus {
    /// The bus file (`Vehicles/.../x.bus`).
    pub file: String,
    pub name: String,
    pub kind: BusKind,
    /// Its liveries, and the name of its own.
    pub paints: Vec<String>,
    pub default_paint: String,
}

/// The words that make a bus electric (in its name, file or the names of its scripts).
const ELECTRIC: [&str; 18] = [
    "electr", "elektr", "e-bus", "ebus", "ecitaro", "e-citaro", "e_citaro", "battery", "batterie", "akku", "trolley", "o-bus", "e-motor", "emotor", "elmotor", "lionscity_e", "urbino_e", "e-urbino",
];

/// Lower-case words of a text (`\b` of Omsi-Hub's patterns).
fn words(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// The names of scripts and parts only an electric bus has (a diesel bus has its 24 V
/// "elektrik" and "batterie" scripts too, and many share an "articulation" script with their
/// articulated sisters: those say nothing).
const ELECTRIC_PARTS: [&str; 15] = ["elektromotor", "e-motor", "e_motor", "emotor", "elmotor", "fahrmotor", "traction", "traktion", "pantograph", "stromabnehmer", "hv_batt", "hvbatt", "hochvolt", "e-antrieb", "eantrieb"];

/// A bus's kind from what is known of it: its names (its name, type, maker and file), the
/// names of its scripts and sounds (`parts`, only for the drive), whether a trailer section
/// is coupled behind it, and its length and height in metres. Omsi-Hub's `vormVanNaam` for
/// the names: "Gelenk", "articulated", "18C", a lone "G" an articulated bus; "Doppeldeck",
/// "DD" a double-decker; "Midi", "10C", "O530K", "kurz" a midibus.
pub fn guess_kind(texts: &[&str], parts: &[&str], trailer: bool, length: Option<f32>, height: Option<f32>) -> BusKind {
    let all = texts.join(" ").to_lowercase();
    let w: Vec<String> = texts.iter().flat_map(|t| words(t)).collect();
    let has = |x: &str| w.iter().any(|y| y == x);
    let parts = parts.join(" ").to_lowercase();
    let electric = ELECTRIC.iter().any(|k| all.contains(k)) || ELECTRIC_PARTS.iter().any(|k| parts.contains(k)) || has("ev") || has("bev") || has("obus") || has("e") && (all.contains("citaro") || all.contains("urbino") || all.contains("lion"));
    let size = if trailer || all.contains("gelenk") || all.contains("artic") || all.contains("18c") || all.contains("19c") || has("g") || has("gn") {
        BusSize::Articulated
    } else if all.contains("doppeldeck") || all.contains("double deck") || all.contains("doubledeck") || all.contains("double-deck") || has("dd") || height.is_some_and(|h| h > 3.9) {
        BusSize::Double
    } else if all.contains("midi") || all.contains("10c") || all.contains("o530k") || all.contains("kurz") || length.is_some_and(|l| l > 4.0 && l < 10.6) {
        BusSize::Midi
    } else {
        BusSize::Solo
    };
    BusKind { size, drive: if electric { Drive::Electric } else { Drive::Diesel } }
}

/// The kind of an installed bus, from its file (`Vehicles/.../x.bus`) and its name.
pub fn kind_of(file: &str, name: &str) -> BusKind {
    let path = crate::resolve_content(file).unwrap_or_else(|_| std::path::PathBuf::from(file));
    let Ok(v) = omsi_vehicle::Vehicle::load(&path) else { return guess_kind(&[name, file], &[], false, None, None) };
    let texts: Vec<String> = vec![name.to_string(), file.to_string(), v.type_name.clone(), v.manufacturer.clone()];
    let file_names = |ps: &[std::path::PathBuf]| ps.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).collect::<Vec<_>>();
    let mut parts = file_names(&v.scripts.scripts);
    parts.extend(file_names(&v.scripts.constfiles));
    parts.extend(file_names(&v.scripts.varlists));
    if let Some(s) = &v.sound {
        parts.push(s.clone());
    }
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    let bb = v.bounding_box;
    guess_kind(&refs, &parts, v.couple_back.is_some(), bb.map(|b| b[1]), bb.map(|b| b[2]))
}

/// The market's buses: every bus installed (reads each bus file - on a thread of its own).
pub fn market_of(vehicles: &[crate::VehicleInfo]) -> Vec<MarketBus> {
    let mut out: Vec<MarketBus> = vehicles
        .iter()
        .map(|v| MarketBus { file: v.file.clone(), name: crate::display_bus_name(&v.name), kind: kind_of(&v.file, &v.name), paints: v.paints.clone(), default_paint: v.default_paint.clone() })
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// A used bus offered this week.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UsedOffer {
    pub no: u32,
    pub bus: MarketBus,
    pub built: String,
    pub km: f64,
    pub condition: f64,
    pub price: Cents,
}

impl UsedOffer {
    pub fn age_years(&self, today: &str) -> f64 {
        dates::years_between(&self.built, today)
    }
}

/// How a bus is paid for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Payment {
    Cash,
    /// The bank lends the price (see `finance`).
    Loan,
}

/// Kilometres between two services.
pub const SERVICE_KM: f64 = 30_000.0;

/// The condition a service brings a bus of that age back to.
pub fn serviced_condition(age_years: f64) -> f64 {
    (100.0 - 1.5 * age_years.max(0.0)).max(55.0)
}

/// The used buses on offer in the company's week: the same all week, a new set on Monday,
/// those bought gone. Older buses have more kilometres and a worse condition; the price is
/// their book value with the dealer's margin.
pub fn used_offers(c: &Company, market: &[MarketBus]) -> Vec<UsedOffer> {
    if market.is_empty() {
        return Vec::new();
    }
    let r = economy::rules(c.difficulty);
    let week = dates::week_of(&c.date);
    let mut rng = Rng::of(&[&c.id, "used"], week);
    let mut out = Vec::new();
    for no in 0..r.used_offers as u32 {
        let Some(bus) = rng.pick(market).cloned() else { break };
        let age = rng.range(2.0, 15.0);
        let km = (age * rng.range(48_000.0, 72_000.0) / 1000.0).round() * 1000.0;
        let condition = (95.0 - age * 3.5 + rng.range(-15.0, 10.0)).clamp(25.0, 92.0).round();
        let value = economy::book_value(economy::new_price(bus.kind, &r, c.price_index), age, km, condition);
        let price = ((value as f64 * r.used_markup / 500_00 as f64).round() as Cents) * 500_00;
        // (counted from the week's Monday: the same offer all week)
        let built = dates::fmt(week - (age * 365.25).round() as i64);
        let taken = c.taken.week == week && c.taken.used.contains(&no);
        if !taken {
            out.push(UsedOffer { no, bus, built, km, condition, price });
        }
    }
    out
}

/// A bus into the fleet: its number, plate and first service.
pub(crate) fn add_vehicle(c: &mut Company, bus: &MarketBus, built: String, km: f64, condition: f64, tenure: Tenure, livery: &str) -> u32 {
    c.counters.vehicle += 1;
    let id = c.counters.vehicle;
    let number = c.next_fleet_number();
    let plate = c.plate_for(&number);
    let next_service_km = ((km / SERVICE_KM).floor() + 1.0) * SERVICE_KM;
    c.fleet.push(Vehicle {
        id,
        number,
        plate,
        bus: bus.file.clone(),
        name: bus.name.clone(),
        kind: bus.kind,
        livery: livery.to_string(),
        house_livery: None,
        built,
        km,
        condition,
        next_service_km,
        tenure,
        acquired: c.date.clone(),
        workshop_until: None,
        breakdowns: 0,
    });
    id
}

/// Pay `amount` for something: from the cash, or with a loan of it.
fn pay(c: &mut Company, amount: Cents, pay: Payment, what: &str) -> Result<(), &'static str> {
    match pay {
        Payment::Cash if c.cash < amount => Err("Not enough cash."),
        Payment::Cash => Ok(()),
        // (the bus itself is the bank's security)
        Payment::Loan => finance::take_loan(c, amount, what, amount).map(|_| ()),
    }
}

/// What a new bus costs today, and the grant on it.
pub fn new_offer(c: &Company, bus: &MarketBus) -> (Cents, Cents) {
    let r = economy::rules(c.difficulty);
    let price = economy::new_price(bus.kind, &r, c.price_index);
    (price, economy::grant(bus.kind, price, &r, c.price_index))
}

/// Whether the company's level allows a bus of this kind: articulated, electric and
/// double-decker buses open with its levels (`levels::Feature`).
pub fn kind_allowed(c: &Company, kind: BusKind) -> Result<(), &'static str> {
    use super::levels::{unlocked, Feature};
    if kind.size == BusSize::Articulated && !unlocked(c, Feature::ArticulatedBuses) {
        return Err("Articulated buses open at a higher company level.");
    }
    if kind.size == BusSize::Double && !unlocked(c, Feature::DoubleDeckers) {
        return Err("Double-deckers open at a higher company level.");
    }
    if kind.drive == Drive::Electric && !unlocked(c, Feature::ElectricBuses) {
        return Err("Electric buses open at a higher company level.");
    }
    Ok(())
}

/// A size of bus the company's level allows (a diesel one: the drive is the bus's own).
pub fn size_allowed(c: &Company, size: BusSize) -> Result<(), &'static str> {
    kind_allowed(c, BusKind { size, drive: Drive::Diesel })
}

/// Buy a new bus. Returns its id.
pub fn buy_new(c: &mut Company, bus: &MarketBus, how: Payment, livery: &str) -> Result<u32, &'static str> {
    super::depot::room(c)?;
    kind_allowed(c, bus.kind)?;
    let (price, grant) = new_offer(c, bus);
    pay(c, price - grant, how, &bus.name)?;
    c.book(BookingKind::Purchase, -price, format!("{} (new)", bus.name), false);
    c.book(BookingKind::Subsidy, grant, bus.name.clone(), false);
    let date = c.date.clone();
    Ok(add_vehicle(c, bus, date, 0.0, 100.0, Tenure::Owned { paid: price - grant, new_value: price }, livery))
}

/// Buy one of the week's used offers.
pub fn buy_used(c: &mut Company, offer: &UsedOffer, how: Payment, livery: &str) -> Result<u32, &'static str> {
    super::depot::room(c)?;
    kind_allowed(c, offer.bus.kind)?;
    let week = dates::week_of(&c.date);
    if c.taken.week == week && c.taken.used.contains(&offer.no) {
        return Err("This bus has been sold.");
    }
    pay(c, offer.price, how, &offer.bus.name)?;
    if c.taken.week != week {
        c.taken = super::model::Taken { week, used: Vec::new(), applicants: Vec::new() };
    }
    c.taken.used.push(offer.no);
    c.book(BookingKind::Purchase, -offer.price, format!("{} (used, {:.0} km)", offer.bus.name, offer.km), false);
    let r = economy::rules(c.difficulty);
    let new_value = economy::new_price(offer.bus.kind, &r, c.price_index);
    Ok(add_vehicle(c, &offer.bus, offer.built.clone(), offer.km, offer.condition, Tenure::Owned { paid: offer.price, new_value }, livery))
}

/// What leasing a new bus costs: the monthly rate, the term in months and the residual.
pub fn lease_offer(c: &Company, bus: &MarketBus) -> (Cents, u32, Cents) {
    let r = economy::rules(c.difficulty);
    let price = economy::new_price(bus.kind, &r, c.price_index);
    (economy::lease_monthly(price, &r), r.lease_months, (price as f64 * r.lease_residual).round() as Cents)
}

/// Lease a new bus: no price now, a rate at every month's end (the first month pro rata).
pub fn lease(c: &mut Company, bus: &MarketBus, livery: &str) -> Result<u32, &'static str> {
    super::depot::room(c)?;
    kind_allowed(c, bus.kind)?;
    let (monthly, months, residual) = lease_offer(c, bus);
    // (the leasing company wants to see a month's rate in the bank)
    if c.cash < monthly {
        return Err("Not enough cash.");
    }
    let until = dates::add(&c.date, (months as f64 * 30.44).round() as i64);
    let date = c.date.clone();
    Ok(add_vehicle(c, bus, date, 0.0, 100.0, Tenure::Leased { monthly, until, residual }, livery))
}

/// Rent a bus for `days` days from today (paid by the day at each day's close).
pub fn rent(c: &mut Company, bus: &MarketBus, days: u32, livery: &str) -> Result<u32, &'static str> {
    super::depot::room(c)?;
    kind_allowed(c, bus.kind)?;
    let r = economy::rules(c.difficulty);
    let daily = economy::rent_per_day(bus.kind, &r, c.price_index);
    if days == 0 {
        return Err("Rent it for a day at least.");
    }
    if c.cash < daily * days as Cents {
        return Err("Not enough cash.");
    }
    let until = dates::add(&c.date, days as i64 - 1);
    // (a rented bus is a few years old and kept well)
    let built = dates::add(&c.date, -3 * 365);
    Ok(add_vehicle(c, bus, built, 150_000.0, 85.0, Tenure::Rented { daily, until }, livery))
}

/// What a bus of the fleet is worth now (owned ones; leased and rented are not ours).
pub fn value_of(c: &Company, v: &Vehicle) -> Cents {
    match &v.tenure {
        Tenure::Owned { new_value, .. } => economy::book_value(*new_value, v.age_years(&c.date), v.km, v.condition),
        _ => 0,
    }
}

/// What selling or giving back a bus brings (negative: what it costs).
pub fn sale_offer(c: &Company, v: &Vehicle) -> Cents {
    let r = economy::rules(c.difficulty);
    match &v.tenure {
        Tenure::Owned { .. } => economy::sale_price(value_of(c, v), &r),
        // giving a lease back early costs three months' rates
        Tenure::Leased { monthly, .. } => -3 * monthly,
        Tenure::Rented { .. } => 0,
    }
}

/// Sell a bus (or give a leased or rented one back). Returns what it brought.
pub fn sell(c: &mut Company, id: u32) -> Result<Cents, &'static str> {
    let Some(v) = c.vehicle(id).cloned() else { return Err("This bus is not in the fleet.") };
    let amount = sale_offer(c, &v);
    if amount < 0 && c.cash < -amount {
        return Err("Not enough cash.");
    }
    let text = format!("{} {}", v.number, v.name);
    match v.tenure {
        Tenure::Owned { .. } => c.book(BookingKind::Sale, amount, text, false),
        Tenure::Leased { .. } => c.book(BookingKind::Lease, amount, format!("{text} (returned early)"), false),
        Tenure::Rented { .. } => {}
    }
    c.fleet.retain(|x| x.id != id);
    Ok(amount)
}

/// Send a bus to the workshop for a service tomorrow: back the day after, in the condition
/// a service brings (the work is paid with the maintenance per kilometre).
pub fn service(c: &mut Company, id: u32) -> Result<(), &'static str> {
    let today = c.date.clone();
    let Some(v) = c.fleet.iter_mut().find(|v| v.id == id) else { return Err("This bus is not in the fleet.") };
    if v.in_workshop(&dates::add(&today, 1)) {
        return Err("It is in the workshop already.");
    }
    v.workshop_until = Some(dates::add(&today, 1));
    v.condition = v.condition.max(serviced_condition(dates::years_between(&v.built, &today)));
    v.next_service_km = ((v.km / SERVICE_KM).floor() + 1.0) * SERVICE_KM;
    Ok(())
}

/// Give a bus another of its liveries.
pub fn set_livery(c: &mut Company, id: u32, livery: &str) {
    if let Some(v) = c.fleet.iter_mut().find(|v| v.id == id) {
        v.livery = livery.to_string();
    }
}

/// The rules of the company's difficulty (a shorthand for the pages).
pub fn rules_of(c: &Company) -> Rules {
    economy::rules(c.difficulty)
}

#[cfg(test)]
mod tests {
    use super::super::{found, Founding};
    use super::*;
    use crate::company::model::Difficulty;

    pub(crate) fn bus(name: &str, size: BusSize, drive: Drive) -> MarketBus {
        MarketBus { file: format!("Vehicles/{name}/{name}.bus"), name: name.into(), kind: BusKind { size, drive }, paints: vec!["Red".into(), "Blue".into()], default_paint: "Red".into() }
    }

    #[test]
    fn a_kind_is_guessed_from_the_files() {
        let k = |t: &[&str]| guess_kind(t, &[], false, None, None);
        assert_eq!(k(&["MAN Lion's City", "Vehicles/MAN_NL/MAN_NL.bus"]), BusKind { size: BusSize::Solo, drive: Drive::Diesel });
        assert_eq!(guess_kind(&["MAN NG"], &[], true, Some(12.0), None).size, BusSize::Articulated);
        assert_eq!(k(&["SD200 Doppeldecker"]).size, BusSize::Double);
        assert_eq!(k(&["Citaro K"]).size, BusSize::Solo);
        assert_eq!(k(&["O530K kurz"]).size, BusSize::Midi);
        assert_eq!(guess_kind(&["Sprinter City"], &[], false, Some(8.5), Some(2.9)).size, BusSize::Midi);
        assert_eq!(guess_kind(&["Bus"], &[], false, Some(11.0), Some(4.2)).size, BusSize::Double);
        assert_eq!(k(&["Citaro G"]).size, BusSize::Articulated);
        // electric from the name or a script that only an electric bus has
        assert_eq!(k(&["eCitaro"]).drive, Drive::Electric);
        assert_eq!(k(&["Solaris Urbino 12 electric"]).drive, Drive::Electric);
        assert_eq!(guess_kind(&["Urbino"], &["main_elektromotor.osc"], false, None, None).drive, Drive::Electric);
        assert_eq!(k(&["Trolleybus Gelenk"]), BusKind { size: BusSize::Articulated, drive: Drive::Electric });
        // a solo diesel's scripts: its 24 V electrics, its battery, the articulation script it
        // shares with its articulated sister and its Euro 5 sound say nothing
        let citybus = guess_kind(&["Citybus by Kajosoft", "Vehicles/Citybus 628c 628g LF by Kajosoft/628c_LF_5hp500.bus"], &["elec.osc", "elektrik.osc", "batterie.osc", "articulation_varlist.txt", "Sound_926LA_E5_z.cfg", "engine.osc"], false, Some(11.95), Some(2.55));
        assert_eq!(citybus, BusKind { size: BusSize::Solo, drive: Drive::Diesel });
    }

    fn company(d: Difficulty) -> Company {
        let mut c = found(&Founding { name: "Stadtbus".into(), short: "SB".into(), difficulty: d, date: "2024-03-04".into(), ..Default::default() }, "Luc");
        // (every kind of bus open: the levels' gates have a test of their own)
        c.progress.xp = super::super::levels::LEVEL_XP[9];
        c
    }

    /// A new company buys solo diesel buses only: articulated, electric and double-decker
    /// buses open with its levels.
    #[test]
    fn the_company_level_opens_the_bigger_and_electric_buses() {
        let mut c = found(&Founding { name: "Klein".into(), short: "KL".into(), difficulty: Difficulty::Realistic, date: "2024-03-04".into(), ..Default::default() }, "Luc");
        assert!(buy_new(&mut c, &bus("Citaro", BusSize::Solo, Drive::Diesel), Payment::Cash, "").is_ok());
        assert_eq!(buy_new(&mut c, &bus("Citaro G", BusSize::Articulated, Drive::Diesel), Payment::Cash, ""), Err("Articulated buses open at a higher company level."));
        assert_eq!(lease(&mut c, &bus("eCitaro", BusSize::Solo, Drive::Electric), ""), Err("Electric buses open at a higher company level."));
        c.progress.xp = super::super::levels::LEVEL_XP[1];
        assert!(buy_new(&mut c, &bus("Citaro G", BusSize::Articulated, Drive::Diesel), Payment::Cash, "").is_ok());
    }

    #[test]
    fn buying_new_and_used_takes_the_money_and_adds_the_bus() {
        let mut c = company(Difficulty::Realistic);
        let cash = c.cash;
        let solo = bus("Citaro", BusSize::Solo, Drive::Diesel);
        let id = buy_new(&mut c, &solo, Payment::Cash, "Blue").unwrap();
        assert_eq!(c.cash, cash - 280_000_00);
        let v = c.vehicle(id).unwrap();
        assert_eq!((v.number.as_str(), v.plate.as_str(), v.livery.as_str(), v.km, v.next_service_km), ("101", "SB-K 101", "Blue", 0.0, SERVICE_KM));
        // an electric bus with its grant (40 % of the extra cost on Realistic)
        let e = bus("eCitaro", BusSize::Solo, Drive::Electric);
        let before = c.cash;
        buy_new(&mut c, &e, Payment::Cash, "").unwrap();
        assert_eq!(before - c.cash, 550_000_00 - 108_000_00);
        // the used market: the same offers all week, others the next
        let market = vec![solo.clone(), e.clone()];
        let offers = used_offers(&c, &market);
        assert_eq!(offers.len(), 6);
        assert!(offers.iter().all(|o| o.price > 0 && o.km > 50_000.0 && (25.0..=92.0).contains(&o.condition)));
        c.date = dates::add(&c.date, 2);
        assert_eq!(used_offers(&c, &market), offers);
        let o = offers[0].clone();
        let before = c.cash;
        let id = buy_used(&mut c, &o, Payment::Cash, "").unwrap();
        assert_eq!(c.cash, before - o.price);
        assert_eq!(c.vehicle(id).unwrap().number, "103");
        assert_eq!(used_offers(&c, &market).len(), 5);
        assert!(buy_used(&mut c, &o, Payment::Cash, "").is_err());
        c.date = dates::add(&c.date, 7);
        assert_ne!(used_offers(&c, &market)[0], o);
    }

    #[test]
    fn without_the_cash_the_bank_may_lend() {
        let mut c = company(Difficulty::Hard);
        let art = bus("Citaro G", BusSize::Articulated, Drive::Electric);
        c.cash = 10_000_00;
        assert_eq!(buy_new(&mut c, &art, Payment::Cash, ""), Err("Not enough cash."));
        // Hard: the bank lends at most €250k without a fleet
        assert!(buy_new(&mut c, &art, Payment::Loan, "").is_err());
        let solo = bus("Citaro", BusSize::Solo, Drive::Diesel);
        c.cash = 400_000_00;
        let id = buy_new(&mut c, &solo, Payment::Loan, "").unwrap();
        assert_eq!(c.loans.len(), 1);
        assert_eq!(c.cash, 400_000_00);
        assert!(c.vehicle(id).is_some());
    }

    #[test]
    fn leasing_renting_and_selling() {
        let mut c = company(Difficulty::Realistic);
        let solo = bus("Citaro", BusSize::Solo, Drive::Diesel);
        let cash = c.cash;
        let l = lease(&mut c, &solo, "").unwrap();
        assert_eq!(c.cash, cash);
        assert!(matches!(c.vehicle(l).unwrap().tenure, Tenure::Leased { monthly, .. } if (3_400_00..3_800_00).contains(&monthly)));
        let r = rent(&mut c, &solo, 3, "").unwrap();
        assert!(matches!(&c.vehicle(r).unwrap().tenure, Tenure::Rented { daily: 360_00, until } if until == "2024-03-06"));
        assert!(c.vehicle(r).unwrap().held_on("2024-03-06") && !c.vehicle(r).unwrap().held_on("2024-03-07"));
        // giving the lease back early costs three rates
        let before = c.cash;
        let got = sell(&mut c, l).unwrap();
        assert!(got < 0 && c.cash == before + got);
        // a bus bought and sold the same day brings 85 % of its price
        let b = buy_new(&mut c, &solo, Payment::Cash, "").unwrap();
        assert_eq!(sell(&mut c, b).unwrap(), 238_000_00);
        assert!(c.vehicle(b).is_none());
        // a service: in the workshop tomorrow
        let s = buy_new(&mut c, &solo, Payment::Cash, "").unwrap();
        service(&mut c, s).unwrap();
        assert!(c.vehicle(s).unwrap().in_workshop("2024-03-05") && !c.vehicle(s).unwrap().in_workshop("2024-03-06"));
        assert!(service(&mut c, s).is_err());
    }
}
