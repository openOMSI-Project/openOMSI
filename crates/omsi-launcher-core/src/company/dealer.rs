//! The bus dealer (Luc: "a better dealer"): the installed buses as a showroom by maker, model
//! and version; the day's special offers (new buses
//! from stock at a discount, demonstrators, a batch of used buses another operator sells) and
//! the used market, both new every company day; haggling with the dealer; a contract that is
//! signed before anything is booked, and the bus that comes on its delivery day. Or, for who
//! wants buses quickly, the quick buy: a model, a number, the list price, at once. The
//! company's setting `BuyingMode` says which of the two the dealer shows first.
//!
//! Every installed bus is to be had new at any company date (Luc: the model years were too
//! complicated - "laat die bouwjaren maar los"); the used market offers buses of some age.
//!
//! Time: what happens at a moment - an offer that expires, a bus that is delivered - is kept
//! as "YYYY-MM-DD HH:MM". The company clock calls `tick` with its time; until it does, the
//! pages call it with `now_of` (the company's day at noon).
//!
//! Everything here is plain functions over plain data; the dice are the company's own
//! (`Rng`): the same day draws the same offers and the same answers, also after a restart.

use super::dates;
use super::economy;
use super::finance;
use super::levels;
use super::market::{self, MarketBus, Payment};
use super::model::{BookingKind, BusKind, BusSize, Cents, Company, Difficulty, Drive, Tenure};
use super::rng::Rng;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// --- moments ---------------------------------------------------------------------------------

/// Minutes since 1970 of "YYYY-MM-DD HH:MM" (or "YYYY-MM-DDTHH:MM"; a bare date is its
/// first minute). None: not a moment.
pub fn minutes_of(at: &str) -> Option<i64> {
    let at = at.trim();
    let (d, t) = at.split_once([' ', 'T']).unwrap_or((at, "00:00"));
    let day = dates::parse(d)?;
    let (h, m) = t.trim().split_once(':')?;
    let (h, m): (i64, i64) = (h.parse().ok()?, m.get(..2).unwrap_or(m).parse().ok()?);
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return None;
    }
    Some(day * 1440 + h * 60 + m)
}

/// A moment as "YYYY-MM-DD HH:MM".
pub fn moment(minutes: i64) -> String {
    let m = minutes.rem_euclid(1440);
    format!("{} {:02}:{:02}", dates::fmt(minutes.div_euclid(1440)), m / 60, m % 60)
}

/// A day at a minute of it.
pub fn at(date: &str, minute: i64) -> String {
    moment(dates::parse(date).unwrap_or(0) * 1440 + minute)
}

/// `at` moved by `minutes` (a moment that cannot be read stays as it is).
pub fn later(at: &str, minutes: i64) -> String {
    minutes_of(at).map(|m| moment(m + minutes)).unwrap_or_else(|| at.to_string())
}

/// The day of a moment.
pub fn day_of(at: &str) -> String {
    at.trim().get(..10).unwrap_or(at).to_string()
}

/// The company's moment now, as its clock has it (`clock::now`).
pub fn now_of(c: &Company) -> String {
    moment(super::clock::now(c))
}

/// `a` is at or before `b`.
fn not_after(a: &str, b: &str) -> bool {
    matches!((minutes_of(a), minutes_of(b)), (Some(a), Some(b)) if a <= b)
}

/// The year of a date or moment.
pub fn year_of(date: &str) -> i32 {
    date.trim().get(..4).and_then(|y| y.parse().ok()).unwrap_or(2000)
}

// --- the showroom -----------------------------------------------------------------------------

/// A bus as the dealer shows it: the market's bus, its family (maker, model, version: the bus
/// step's tree) and what its cabin holds. (An older file's "years" are passed over.)
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Listing {
    pub bus: MarketBus,
    pub maker: String,
    pub model: String,
    pub version: String,
    pub seats: Option<u32>,
    pub standing: Option<u32>,
}

/// What a bus's files say beyond its kind: seats and standing places (its passenger cabin
/// and its trailer's: a seat has a height, a standing place none), and the names of its
/// scripts and sounds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Specs {
    pub seats: Option<u32>,
    pub standing: Option<u32>,
    pub parts: Vec<String>,
}

/// Read a bus's specs from its files (None of a count: no cabin).
pub fn read_specs(file: &str) -> Specs {
    let path = crate::resolve_content(file).unwrap_or_else(|_| PathBuf::from(file));
    let Ok(v) = omsi_vehicle::Vehicle::load(&path) else { return Specs::default() };
    let mut s = Specs::default();
    let mut count = |v: &omsi_vehicle::Vehicle| {
        if let Some(cab) = v.passenger_cabin.as_ref() {
            if let Ok(c) = omsi_vehicle::cabin::PassengerCabin::load(&omsi_cfg::resolve_path(v.dir(), cab)) {
                let seats = c.pass_positions.iter().filter(|p| p.height > 0.01).count() as u32;
                let standing = c.pass_positions.len() as u32 - seats;
                *s.seats.get_or_insert(0) += seats;
                *s.standing.get_or_insert(0) += standing;
            }
        }
    };
    count(&v);
    if let Some(back) = v.couple_back_path() {
        if let Ok(t) = omsi_vehicle::Vehicle::load(&back) {
            count(&t);
        }
    }
    let names = |ps: &[PathBuf]| ps.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).collect::<Vec<_>>();
    s.parts = names(&v.scripts.scripts);
    s.parts.extend(names(&v.scripts.constfiles));
    if let Some(snd) = &v.sound {
        s.parts.push(snd.clone());
    }
    s
}

/// A listing of an installed bus: `family` is its maker, model and version as the bus step
/// groups them.
pub fn listing_of(bus: MarketBus, family: (String, String, String), specs: &Specs) -> Listing {
    Listing { bus, maker: family.0, model: family.1, version: family.2, seats: specs.seats, standing: specs.standing }
}

/// What the showroom's filters ask.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub search: String,
    pub size: Option<BusSize>,
    pub drive: Option<Drive>,
    /// The highest new price (0: any).
    pub max_price: Cents,
}

impl Filter {
    pub fn fits(&self, l: &Listing, c: &Company) -> bool {
        let q = self.search.trim().to_lowercase();
        if !q.is_empty() && ![&l.bus.name, &l.maker, &l.model, &l.version, &l.bus.file].iter().any(|s| s.to_lowercase().contains(&q)) {
            return false;
        }
        if self.size.is_some_and(|s| s != l.bus.kind.size) || self.drive.is_some_and(|d| d != l.bus.kind.drive) {
            return false;
        }
        if self.max_price > 0 && list_price(c, l.bus.kind) > self.max_price {
            return false;
        }
        true
    }
}

/// A new bus's list price today.
pub fn list_price(c: &Company, kind: BusKind) -> Cents {
    economy::new_price(kind, &economy::rules(c.difficulty), c.price_index)
}

/// What painting a bus in another livery than its own costs.
pub fn painting_cost(c: &Company) -> Cents {
    round_to(3_500_00 as f64 * c.price_index, 100_00)
}

fn round_to(c: f64, step: Cents) -> Cents {
    ((c / step as f64).round() as Cents) * step
}

// --- the dealer's terms by difficulty -------------------------------------------------------

/// What a difficulty makes of the dealer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Terms {
    /// The dealer's margin on a new bus (of the list price), and the extra on a used one.
    pub margin: f64,
    pub used_margin: f64,
    /// How long he listens: rounds, and his patience (1: average).
    pub rounds: u32,
    pub patience: f64,
    /// Days from signing to a new bus's delivery (one not in stock).
    pub delivery_days: i64,
    /// Warranty in months: a new bus's, a used one's, and what an extended warranty adds.
    pub warranty_new: u32,
    pub warranty_used: u32,
    /// Days the dealer will not talk to a company that pushed too hard.
    pub sulk_days: i64,
}

pub fn terms(d: Difficulty) -> Terms {
    match d {
        Difficulty::Easy => Terms { margin: 0.16, used_margin: 0.06, rounds: 7, patience: 1.4, delivery_days: 5, warranty_new: 24, warranty_used: 12, sulk_days: 1 },
        Difficulty::Realistic => Terms { margin: 0.11, used_margin: 0.05, rounds: 5, patience: 1.0, delivery_days: 14, warranty_new: 24, warranty_used: 6, sulk_days: 2 },
        Difficulty::Hard => Terms { margin: 0.07, used_margin: 0.04, rounds: 4, patience: 0.75, delivery_days: 28, warranty_new: 12, warranty_used: 3, sulk_days: 4 },
    }
}

/// Months of warranty an extended warranty adds.
pub const EXTRA_WARRANTY_MONTHS: u32 = 12;

/// The days from signing to the delivery of a new bus of `maker`'s (the same model takes the
/// same time that day): from stock two, else the difficulty's with a spread; a faster
/// delivery halves it.
pub fn delivery_days(c: &Company, key: &str, stock: bool, fast: bool) -> i64 {
    let base = if stock {
        2
    } else {
        let mut rng = Rng::of(&[&c.id, "delivery", key], dates::parse(&c.date).unwrap_or(0));
        (terms(c.difficulty).delivery_days as f64 * rng.range(0.8, 1.4)).round() as i64
    };
    if fast {
        (base / 2).max(1)
    } else {
        base
    }
}

// --- the offers of the day and the used market ----------------------------------------------

/// What an offer is.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OfferKind {
    /// A used bus of the used market.
    Used,
    /// New buses from the dealer's stock at a discount (delivered in two days).
    Discount,
    /// The dealer's demonstrator: nearly new, a few thousand kilometres.
    Demonstrator,
    /// Several used buses of one model that another operator sells.
    Batch,
}

impl OfferKind {
    pub fn label(self) -> &'static str {
        match self {
            OfferKind::Used => "Used",
            OfferKind::Discount => "Special offer",
            OfferKind::Demonstrator => "Demonstrator",
            OfferKind::Batch => "Fleet sale",
        }
    }
}

/// An offer of the dealer's (a used bus, or one of the day's).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Offer {
    /// "u<day>-<n>" (the used market) or "o<day>-<n>" (the day's offers).
    pub id: String,
    pub kind: OfferKind,
    pub listing: Listing,
    /// Buses left of it.
    pub count: u32,
    /// When they were built (a new one: today), their kilometres and condition.
    pub built: String,
    pub km: f64,
    pub condition: f64,
    /// The price of one, and what one costs otherwise (new: its list price; used: its value).
    pub price: Cents,
    pub reference: Cents,
    /// Who sells it.
    pub seller: String,
    pub published: String,
    pub expires: String,
}

impl Offer {
    /// A new bus (delivered from stock), not a used one.
    pub fn is_new(&self) -> bool {
        self.kind == OfferKind::Discount
    }

    pub fn age_years(&self, today: &str) -> f64 {
        dates::years_between(&self.built, today)
    }

    /// The saving against what it costs otherwise, 0..1.
    pub fn saving(&self) -> f64 {
        if self.reference <= 0 {
            0.0
        } else {
            (1.0 - self.price as f64 / self.reference as f64).max(0.0)
        }
    }
}

/// Operators that sell their old buses (made up).
const OPERATORS: [&str; 10] = [
    "Stadtwerke Lindenau",
    "Verkehrsbetriebe Ostheim",
    "Regiobus Mittelland",
    "Kreisverkehr Altenburg",
    "Busbetrieb Nordheide",
    "Stadtverkehr Weißenfels",
    "Overland Vervoer Achterhoek",
    "Transports Urbains de Valmont",
    "Rheintal Mobil",
    "Bergland Linien",
];

/// The dealer of a maker's buses.
pub fn dealer_name(maker: &str) -> String {
    if maker.trim().is_empty() {
        "Bus dealer".to_string()
    } else {
        format!("{} dealer", maker.trim())
    }
}

/// The used bus centre (where the used market is).
pub const USED_CENTRE: &str = "Used bus centre";

/// How many days an offer runs back: the used market's buses stay four days, the day's offers
/// up to five.
const OFFER_DAYS: i64 = 5;
const USED_DAYS: i64 = 4;

fn used_price(c: &Company, kind: BusKind, age: f64, km: f64, condition: f64) -> (Cents, Cents) {
    let r = economy::rules(c.difficulty);
    let value = economy::book_value(economy::new_price(kind, &r, c.price_index), age, km, condition);
    (round_to(value as f64 * r.used_markup, 500_00), round_to(value as f64, 100_00))
}

/// A used bus `ages` years old (the least, the most). None: no such bus.
fn used_build(today: i64, rng: &mut Rng, ages: (f64, f64)) -> Option<(String, f64)> {
    let year = dates::civil_from_days(today).0;
    let (lo, hi) = (year - ages.1.round() as i32, year - ages.0.round() as i32);
    if hi < lo {
        return None;
    }
    let built_year = rng.int(lo as i64, hi as i64) as i32;
    let day = dates::days_from_civil(built_year, rng.int(1, 12) as u32, rng.int(1, 28) as u32).min(today - 200);
    let age = (today - day) as f64 / 365.25;
    Some((dates::fmt(day), age))
}

/// The used market's buses put up on `day`: each stays four days or until sold.
fn used_of_day(c: &Company, listings: &[Listing], day: i64) -> Vec<Offer> {
    let pool: Vec<&Listing> = listings.iter().collect();
    if pool.is_empty() {
        return Vec::new();
    }
    let r = economy::rules(c.difficulty);
    let mut rng = Rng::of(&[&c.id, "used-market"], day);
    let n = (r.used_offers / 2).max(2);
    let date = dates::fmt(day);
    let mut out = Vec::new();
    for k in 0..n {
        let Some(l) = rng.pick(&pool).copied() else { break };
        let Some((built, age)) = used_build(day, &mut rng, (2.0, 15.0)) else { continue };
        let km = (age * rng.range(45_000.0, 72_000.0) / 1000.0).round() * 1000.0;
        let condition = (95.0 - age * 3.0 + rng.range(-15.0, 10.0)).clamp(20.0, 92.0).round();
        let (price, value) = used_price(c, l.bus.kind, age, km, condition);
        out.push(Offer {
            id: format!("u{day}-{k}"),
            kind: OfferKind::Used,
            listing: l.clone(),
            count: 1,
            built,
            km,
            condition,
            price,
            reference: value,
            seller: USED_CENTRE.to_string(),
            published: at(&date, 7 * 60),
            expires: at(&dates::add(&date, USED_DAYS), 7 * 60),
        });
    }
    out
}

/// The special offers put up on `day`: two or three of a discount on new buses from stock, a
/// demonstrator, a batch of used buses another operator sells.
fn offers_of_day(c: &Company, listings: &[Listing], day: i64) -> Vec<Offer> {
    let new: Vec<&Listing> = listings.iter().collect();
    let used = new.clone();
    let mut rng = Rng::of(&[&c.id, "offers"], day);
    let date = dates::fmt(day);
    let r = economy::rules(c.difficulty);
    let easy = if c.difficulty == Difficulty::Easy { 0.03 } else { 0.0 };
    let n = rng.int(2, 3);
    let mut out = Vec::new();
    for k in 0..n {
        let roll = rng.f64();
        let days = rng.int(2, OFFER_DAYS);
        let published = at(&date, rng.int(7, 10) * 60);
        let expires = at(&dates::add(&date, days), 18 * 60);
        let id = format!("o{day}-{k}");
        if roll < 0.4 {
            let Some(l) = rng.pick(&new).copied() else { continue };
            let list = economy::new_price(l.bus.kind, &r, c.price_index);
            let off = rng.range(0.06, 0.14) + easy;
            out.push(Offer { id, kind: OfferKind::Discount, listing: l.clone(), count: rng.int(1, 4) as u32, built: date.clone(), km: 0.0, condition: 100.0, price: round_to(list as f64 * (1.0 - off), 100_00), reference: list, seller: dealer_name(&l.maker), published, expires });
        } else if roll < 0.7 {
            let Some(l) = rng.pick(&new).copied() else { continue };
            let list = economy::new_price(l.bus.kind, &r, c.price_index);
            let months = rng.int(4, 14);
            let built = dates::add(&date, -(months as f64 * 30.44).round() as i64);
            let km = (rng.range(5_000.0, 35_000.0) / 100.0).round() * 100.0;
            out.push(Offer {
                id,
                kind: OfferKind::Demonstrator,
                listing: l.clone(),
                count: 1,
                built,
                km,
                condition: rng.range(93.0, 98.0).round(),
                price: round_to(list as f64 * (rng.range(0.74, 0.84) - easy), 100_00),
                reference: list,
                seller: dealer_name(&l.maker),
                published,
                expires,
            });
        } else {
            let Some(l) = rng.pick(&used).copied() else { continue };
            let Some((built, age)) = used_build(day, &mut rng, (6.0, 14.0)) else { continue };
            let km = (age * rng.range(50_000.0, 68_000.0) / 1000.0).round() * 1000.0;
            let condition = (90.0 - age * 2.8 + rng.range(-8.0, 6.0)).clamp(30.0, 90.0).round();
            let (price, value) = used_price(c, l.bus.kind, age, km, condition);
            let seller = rng.pick(&OPERATORS).copied().unwrap_or(OPERATORS[0]).to_string();
            out.push(Offer { id, kind: OfferKind::Batch, listing: l.clone(), count: rng.int(3, 6) as u32, built, km, condition, price: round_to(price as f64 * 0.88, 500_00), reference: value, seller, published, expires });
        }
    }
    out
}

/// What of an offer is sold already.
fn taken_of(c: &Company, id: &str) -> u32 {
    c.dealer.taken.iter().filter(|t| t.id == id).map(|t| t.count).sum()
}

fn open_at(o: &Offer, now: &str) -> bool {
    not_after(&o.published, now) && !not_after(&o.expires, now)
}

/// The used market at `now`: the buses of the last days not sold and not expired.
pub fn used_market(c: &Company, listings: &[Listing], now: &str) -> Vec<Offer> {
    gather(c, listings, now, USED_DAYS, used_of_day)
}

/// The special offers open at `now`, the newest first.
pub fn day_offers(c: &Company, listings: &[Listing], now: &str) -> Vec<Offer> {
    gather(c, listings, now, OFFER_DAYS, offers_of_day)
}

fn gather(c: &Company, listings: &[Listing], now: &str, back: i64, of_day: fn(&Company, &[Listing], i64) -> Vec<Offer>) -> Vec<Offer> {
    let Some(today) = dates::parse(&day_of(now)) else { return Vec::new() };
    let mut out = Vec::new();
    for day in (today - back..=today).rev() {
        for mut o in of_day(c, listings, day) {
            o.count = o.count.saturating_sub(taken_of(c, &o.id));
            if o.count > 0 && open_at(&o, now) {
                out.push(o);
            }
        }
    }
    out
}

/// An offer by its id, if it is still open at `now`.
pub fn offer(c: &Company, listings: &[Listing], id: &str, now: &str) -> Option<Offer> {
    day_offers(c, listings, now).into_iter().chain(used_market(c, listings, now)).find(|o| o.id == id)
}

// --- haggling -----------------------------------------------------------------------------------

/// Something more than the price to haggle for.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Extra {
    /// The first service in the company's workshop is the dealer's.
    FreeService,
    /// Delivered in half the time.
    FastDelivery,
    /// Painted in the livery chosen at no cost.
    Painting,
    /// A year more of warranty.
    Warranty,
    /// The model's type training for two drivers on delivery (`licences::introduction`).
    Introduction,
}

impl Extra {
    pub const ALL: [Extra; 5] = [Extra::FreeService, Extra::FastDelivery, Extra::Painting, Extra::Warranty, Extra::Introduction];

    pub fn label(self) -> &'static str {
        match self {
            Extra::FreeService => "Free first service",
            Extra::FastDelivery => "Faster delivery",
            Extra::Painting => "Livery painting included",
            Extra::Warranty => "A year more warranty",
            Extra::Introduction => "Driver introduction",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Extra::FreeService => "construction",
            Extra::FastDelivery => "speed",
            Extra::Painting => "palette",
            Extra::Warranty => "badge",
            Extra::Introduction => "key",
        }
    }
}

/// What an extra costs the dealer for one bus of `list`.
pub fn extra_value(c: &Company, e: Extra, list: Cents) -> Cents {
    match e {
        Extra::FreeService => round_to(1_200_00 as f64 * c.price_index, 100_00),
        Extra::FastDelivery => round_to(list as f64 * 0.01, 100_00),
        Extra::Painting => painting_cost(c),
        Extra::Warranty => round_to(list as f64 * 0.02, 100_00),
        Extra::Introduction => 2 * super::licences::course_cost(c, super::training::CourseKind::TypeTraining),
    }
}

/// What is talked about.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Quote {
    /// "new:<bus file>", or the offer's id.
    pub key: String,
    pub maker: String,
    pub kind: BusKind,
    /// The price of one asked at the start.
    pub list: Cents,
    pub used: bool,
    /// From the dealer's stock (a special offer: less to give).
    pub stock: bool,
    pub count: u32,
}

impl Quote {
    pub fn new_bus(c: &Company, l: &Listing, count: u32) -> Quote {
        Quote { key: format!("new:{}", l.bus.file), maker: l.maker.clone(), kind: l.bus.kind, list: list_price(c, l.bus.kind), used: false, stock: false, count: count.max(1) }
    }

    pub fn of_offer(o: &Offer, count: u32) -> Quote {
        Quote { key: o.id.clone(), maker: o.listing.maker.clone(), kind: o.listing.bus.kind, list: o.price, used: !o.is_new(), stock: o.is_new(), count: count.clamp(1, o.count.max(1)) }
    }
}

/// A move of the company's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Move {
    /// "Can you do something on the price?"
    AskDiscount,
    /// A price for one bus.
    Offer(Cents),
    AskExtra(Extra),
    /// Take what is on the table.
    Accept,
}

/// The dealer's answer.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    /// The price agreed (for one).
    Accepted(Cents),
    /// He comes down to this.
    Discount(Cents),
    /// He meets the offer halfway: this.
    Counter(Cents),
    /// Not a cent less.
    Firm,
    /// That offer is no offer: he asks this still.
    TooLow(Cents),
    ExtraGranted(Extra),
    ExtraRefused(Extra),
    /// His last word: this, or nothing.
    LastOffer(Cents),
    /// He has had enough: no talks until then.
    BrokeOff(String),
}

/// A talk with the dealer about one thing on one day (the next day it starts anew; reopened
/// the same day it goes on where it stopped).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Talk {
    pub quote: Quote,
    pub day: String,
    /// What he asks for one now, and the least he would take (not shown).
    pub asking: Cents,
    pub floor: Cents,
    pub rounds: u32,
    pub max_rounds: u32,
    /// 0..: at 0 he breaks off.
    pub patience: f64,
    pub extras: Vec<Extra>,
    pub refused: Vec<Extra>,
    pub replies: Vec<Reply>,
    /// Agreed (the price is `asking`), or his last word was said, or he broke off.
    pub agreed: bool,
    pub closed: bool,
}

impl Talk {
    pub fn rounds_left(&self) -> u32 {
        self.max_rounds.saturating_sub(self.rounds)
    }

    /// What is off the list price so far, 0..1.
    pub fn discount(&self) -> f64 {
        if self.quote.list <= 0 {
            0.0
        } else {
            1.0 - self.asking as f64 / self.quote.list as f64
        }
    }
}

/// How much the dealer can give on `q`, of its price: his margin, less on a bus in demand
/// (electric ones) or from stock; for one bus alone much less than for an order of several
/// (from five a bulk discount, from ten a fleet order's - see `bulk`); more for a customer of
/// long standing with him (see `Standing`) and a little for the company's reputation and level.
pub fn room(c: &Company, q: &Quote) -> f64 {
    let t = terms(c.difficulty);
    let mut room = if q.used { t.margin * 0.6 + t.used_margin } else { t.margin };
    room *= match (q.kind.drive, q.kind.size) {
        (Drive::Electric, _) => 0.7,
        (_, BusSize::Double) => 0.9,
        (_, BusSize::Midi) => 1.1,
        _ => 1.0,
    };
    if q.stock {
        room *= 0.4;
    }
    let (share, bulk_bonus) = bulk(q.count);
    room = room * share + bulk_bonus;
    room += Standing::of(relation(c, &q.maker).points).room();
    room += (c.reputation - 50.0) / 50.0 * 0.02;
    room += levels::level(c) as f64 * 0.003;
    room.clamp(0.01, 0.35)
}

// --- the company's standing with a dealer ------------------------------------------------------

/// The company's standing with one maker's dealer (Luc: a dealer gives more to a customer
/// who keeps coming back): what it bought of him, what it spent, and the points that make
/// its standing - `POINTS_NEW` for a new bus, `POINTS_USED` for a used one, `POINTS_BROKE`
/// off when he broke off a talk.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Relation {
    pub maker: String,
    #[serde(default)]
    pub bought: u32,
    #[serde(default)]
    pub spent: Cents,
    #[serde(default)]
    pub points: f64,
}

const POINTS_NEW: f64 = 10.0;
const POINTS_USED: f64 = 6.0;
const POINTS_BROKE: f64 = 12.0;

/// What the company is to a dealer, by its points with him.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Standing {
    New,
    Known,
    Regular,
    Partner,
}

impl Standing {
    pub fn of(points: f64) -> Standing {
        if points >= 150.0 {
            Standing::Partner
        } else if points >= 60.0 {
            Standing::Regular
        } else if points >= 20.0 {
            Standing::Known
        } else {
            Standing::New
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Standing::New => "New customer",
            Standing::Known => "Known customer",
            Standing::Regular => "Regular customer",
            Standing::Partner => "Preferred partner",
        }
    }

    /// What it adds to the room he has for a discount.
    pub fn room(self) -> f64 {
        match self {
            Standing::New => 0.0,
            Standing::Known => 0.015,
            Standing::Regular => 0.03,
            Standing::Partner => 0.05,
        }
    }

    /// The points the next standing begins at (none above a partner).
    pub fn next_at(self) -> Option<f64> {
        match self {
            Standing::New => Some(20.0),
            Standing::Known => Some(60.0),
            Standing::Regular => Some(150.0),
            Standing::Partner => None,
        }
    }
}

/// The company's standing with `maker`'s dealer: a new customer when it never bought of him.
pub fn relation(c: &Company, maker: &str) -> Relation {
    c.dealer.relations.iter().find(|r| r.maker.eq_ignore_ascii_case(maker)).cloned().unwrap_or(Relation { maker: maker.to_string(), ..Default::default() })
}

fn relation_mut<'a>(c: &'a mut Company, maker: &str) -> &'a mut Relation {
    let k = match c.dealer.relations.iter().position(|r| r.maker.eq_ignore_ascii_case(maker)) {
        Some(k) => k,
        None => {
            c.dealer.relations.push(Relation { maker: maker.to_string(), ..Default::default() });
            c.dealer.relations.len() - 1
        }
    };
    &mut c.dealer.relations[k]
}

/// A purchase of `count` buses (new or used) for `spent` in the company's standing with
/// `maker`'s dealer.
fn bought_of(c: &mut Company, maker: &str, count: u32, new: bool, spent: Cents) {
    let r = relation_mut(c, maker);
    r.bought += count;
    r.spent += spent;
    r.points += count as f64 * if new { POINTS_NEW } else { POINTS_USED };
}

/// What `count` buses at a time do to the dealer's room: a share of his margin and a bonus on
/// top. One bus alone gets half of it, two a little more, three or four all of it; from five
/// a bulk discount, from ten a fleet order's.
pub fn bulk(count: u32) -> (f64, f64) {
    match count {
        0 | 1 => (0.5, 0.0),
        2 => (0.8, 0.0),
        3 | 4 => (1.0, 0.0),
        5..=9 => (1.0, 0.025),
        _ => (1.0, 0.05),
    }
}

/// The step of the order `count` buses make, and from how many buses the next begins (for
/// the dealer's page).
pub fn bulk_step(count: u32) -> (&'static str, Option<u32>) {
    match count {
        0 | 1 => ("One bus: little room for a discount", Some(2)),
        2..=4 => ("A small order", Some(5)),
        5..=9 => ("A bulk order: a bulk discount", Some(10)),
        _ => ("A fleet order: a fleet discount", None),
    }
}

/// The dealer's discount on a quick buy, without a talk: a part of what he could give, so
/// that the order's size and the company's standing with him count there too (a talk gets
/// more).
pub fn quick_discount(c: &Company, q: &Quote) -> f64 {
    room(c, q) * 0.4
}

/// The price of one of `count` new `l` on a quick buy: the list price less the quick
/// discount, rounded to a hundred euros.
pub fn quick_price(c: &Company, l: &Listing, count: u32) -> Cents {
    let q = Quote::new_bus(c, l, count);
    round_to(q.list as f64 * (1.0 - quick_discount(c, &q)), 100_00).min(q.list)
}

/// Talking to `maker`'s dealer is off until this moment (he broke off).
pub fn sulking(c: &Company, maker: &str, now: &str) -> Option<String> {
    c.dealer.breaks.iter().find(|b| b.0.eq_ignore_ascii_case(maker) && !not_after(&b.1, now)).map(|b| b.1.clone())
}

/// Open a talk about `q` (or go on with today's): None when the dealer will not talk now.
pub fn open_talk(c: &Company, q: &Quote, now: &str) -> Result<Talk, &'static str> {
    if sulking(c, &q.maker, now).is_some() {
        return Err("The dealer does not want to talk to you for now.");
    }
    let day = day_of(now);
    if let Some(t) = c.dealer.talks.iter().find(|t| t.quote.key == q.key && t.day == day && t.quote.count == q.count) {
        return Ok(t.clone());
    }
    let t = terms(c.difficulty);
    let mut rng = Rng::of(&[&c.id, "floor", &q.key], dates::parse(&day).unwrap_or(0));
    let give = room(c, q) * rng.range(0.8, 1.15);
    let floor = round_to(q.list as f64 * (1.0 - give), 100_00).min(q.list);
    Ok(Talk { quote: q.clone(), day, asking: q.list, floor, rounds: 0, max_rounds: t.rounds, patience: t.patience, extras: Vec::new(), refused: Vec::new(), replies: Vec::new(), agreed: false, closed: false })
}

/// The dealer's answer to a move, kept in the talk (and the talk in the company, so that it
/// goes on the same day).
pub fn respond(c: &mut Company, talk: &mut Talk, mv: Move, now: &str) -> Reply {
    if talk.closed {
        // (his last word can still be taken; a talk he broke off cannot)
        let broke = matches!(talk.replies.last(), Some(Reply::BrokeOff(_)));
        if mv == Move::Accept && !talk.agreed && !broke {
            talk.agreed = true;
            talk.replies.push(Reply::Accepted(talk.asking));
            keep_talk(c, talk);
            return Reply::Accepted(talk.asking);
        }
        return talk.replies.last().cloned().unwrap_or(Reply::Firm);
    }
    let day = dates::parse(&talk.day).unwrap_or(0);
    let mut rng = Rng::of(&[&c.id, "talk", &talk.quote.key], day * 64 + talk.rounds as i64);
    talk.rounds += 1;
    talk.patience -= 0.1;
    let gap = (talk.asking - talk.floor).max(0);
    let list = talk.quote.list.max(1);
    let mut reply = match mv {
        Move::Accept => {
            talk.agreed = true;
            talk.closed = true;
            Reply::Accepted(talk.asking)
        }
        Move::AskDiscount => {
            if gap >= 100_00 && rng.chance(0.45 + 0.3 * talk.patience.clamp(0.0, 1.0)) {
                let step = round_to(gap as f64 * rng.range(0.25, 0.5), 100_00).max(100_00);
                talk.asking -= step.min(gap);
                Reply::Discount(talk.asking)
            } else {
                talk.patience -= if gap < 100_00 { 0.25 } else { 0.1 };
                Reply::Firm
            }
        }
        Move::Offer(x) => {
            if x >= talk.asking {
                talk.agreed = true;
                talk.closed = true;
                Reply::Accepted(talk.asking)
            } else if x >= talk.floor {
                let p = if gap == 0 { 1.0 } else { (x - talk.floor) as f64 / gap as f64 };
                if rng.chance(0.1 + 0.75 * p.powf(0.7)) {
                    talk.asking = x;
                    talk.agreed = true;
                    talk.closed = true;
                    Reply::Accepted(x)
                } else {
                    let counter = (x as f64 + (talk.asking - x) as f64 * rng.range(0.35, 0.65)) as Cents;
                    let counter = (((counter + 100_00 - 1) / 100_00) * 100_00).clamp(talk.floor, talk.asking);
                    talk.asking = counter;
                    Reply::Counter(counter)
                }
            } else {
                // (a low offer costs patience, the lower the more)
                let below = (talk.floor - x) as f64 / list as f64;
                talk.patience -= 0.2 + 2.0 * below;
                let counter = round_to(talk.asking as f64 - gap as f64 * rng.range(0.1, 0.3), 100_00).clamp(talk.floor, talk.asking);
                talk.asking = counter;
                Reply::TooLow(counter)
            }
        }
        Move::AskExtra(e) => {
            let cost = extra_value(c, e, list);
            if talk.extras.contains(&e) {
                Reply::ExtraGranted(e)
            } else if gap >= cost && rng.chance(0.35 + 0.45 * talk.patience.clamp(0.0, 1.0)) {
                talk.extras.push(e);
                talk.refused.retain(|x| *x != e);
                // (what the extra costs him is off what he can still give)
                talk.floor = (talk.floor + cost).min(talk.asking);
                Reply::ExtraGranted(e)
            } else {
                talk.patience -= 0.15;
                if !talk.refused.contains(&e) {
                    talk.refused.push(e);
                }
                Reply::ExtraRefused(e)
            }
        }
    };
    if !talk.closed {
        if talk.patience <= 0.0 {
            let until = later(now, terms(c.difficulty).sulk_days * 1440);
            c.dealer.breaks.retain(|b| !b.0.eq_ignore_ascii_case(&talk.quote.maker));
            c.dealer.breaks.push((talk.quote.maker.clone(), until.clone()));
            // (and he remembers it)
            let r = relation_mut(c, &talk.quote.maker);
            r.points = (r.points - POINTS_BROKE).max(0.0);
            talk.closed = true;
            talk.asking = talk.quote.list;
            talk.extras.clear();
            reply = Reply::BrokeOff(until);
        } else if talk.rounds >= talk.max_rounds && !matches!(reply, Reply::Accepted(_)) {
            talk.closed = true;
            reply = Reply::LastOffer(talk.asking);
        }
    }
    talk.replies.push(reply.clone());
    keep_talk(c, talk);
    reply
}

/// The talk kept in the company (one per thing and day).
fn keep_talk(c: &mut Company, talk: &Talk) {
    c.dealer.talks.retain(|t| !(t.quote.key == talk.quote.key && t.day == talk.day));
    c.dealer.talks.push(talk.clone());
}

// --- the contract -------------------------------------------------------------------------------

/// How a contract is paid.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PayWay {
    #[default]
    Cash,
    /// The bank lends the amount (see `finance`).
    Loan,
    /// Leased: a rate every month, no price now (new buses only).
    Lease,
}

impl PayWay {
    pub fn label(self) -> &'static str {
        match self {
            PayWay::Cash => "Cash",
            PayWay::Loan => "Bank loan",
            PayWay::Lease => "Leasing",
        }
    }
}

/// A contract of purchase: who sells and buys, what, how many, at what price, paid how,
/// delivered when, with what warranty and extras - and the signature that makes it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Contract {
    /// Its number (given when it is signed).
    pub no: u32,
    pub seller: String,
    pub buyer: String,
    pub listing: Listing,
    pub livery: String,
    pub count: u32,
    /// A new bus (else used: built, km and condition as they are).
    pub new: bool,
    pub built: String,
    pub km: f64,
    pub condition: f64,
    /// The offer it buys (its id), if any.
    pub offer: Option<String>,
    /// Per bus: the list price, the agreed price, the grant and the painting.
    pub list: Cents,
    pub price: Cents,
    pub grant: Cents,
    pub painting: Cents,
    pub pay: PayWay,
    /// Days from signing to delivery (0: at once).
    pub delivery_days: i64,
    pub warranty_months: u32,
    pub extras: Vec<Extra>,
    /// The name signed with, the strokes drawn (0..1 in the signature field), and when.
    pub signed_by: String,
    #[serde(default)]
    pub strokes: Vec<Vec<[f32; 2]>>,
    pub signed_at: String,
}

impl Contract {
    /// The price of all the buses with their painting.
    pub fn total(&self) -> Cents {
        (self.price + self.painting) * self.count as Cents
    }

    pub fn grants(&self) -> Cents {
        self.grant * self.count as Cents
    }

    /// What is paid when it is signed (cash or loan): the total less the grants.
    pub fn due(&self) -> Cents {
        self.total() - self.grants()
    }

    /// A leased bus's monthly rate, the term and its residual value (of the agreed price).
    pub fn lease(&self, c: &Company) -> (Cents, u32, Cents) {
        let r = economy::rules(c.difficulty);
        let base = self.price + self.painting;
        (economy::lease_monthly(base, &r), r.lease_months, (base as f64 * r.lease_residual).round() as Cents)
    }

    /// When it is delivered, signed at `now`: a new bus at eight in the morning of its day.
    pub fn delivery(&self, now: &str) -> String {
        if self.delivery_days <= 0 {
            now.to_string()
        } else {
            at(&dates::add(&day_of(now), self.delivery_days), 8 * 60)
        }
    }

    pub fn is_signed(&self) -> bool {
        !self.signed_by.trim().is_empty() || self.strokes.iter().any(|s| s.len() > 1)
    }
}

/// The contract for `count` new buses of a listing at `price` each (from a talk, the list
/// price, or a special offer from stock).
pub fn draft_new(c: &Company, l: &Listing, count: u32, price: Cents, extras: &[Extra], livery: &str, stock: Option<&Offer>) -> Contract {
    let r = economy::rules(c.difficulty);
    let t = terms(c.difficulty);
    let key = format!("new:{}", l.bus.file);
    let fast = extras.contains(&Extra::FastDelivery);
    let painting = if livery.is_empty() || extras.contains(&Extra::Painting) { 0 } else { painting_cost(c) };
    Contract {
        seller: dealer_name(&l.maker),
        buyer: c.name.clone(),
        listing: l.clone(),
        livery: livery.to_string(),
        count: count.max(1),
        new: true,
        built: c.date.clone(),
        km: 0.0,
        condition: 100.0,
        offer: stock.map(|o| o.id.clone()),
        list: list_price(c, l.bus.kind),
        price,
        grant: economy::grant(l.bus.kind, price, &r, c.price_index),
        painting,
        pay: PayWay::Cash,
        delivery_days: delivery_days(c, &key, stock.is_some(), fast),
        warranty_months: t.warranty_new + if extras.contains(&Extra::Warranty) { EXTRA_WARRANTY_MONTHS } else { 0 },
        extras: extras.to_vec(),
        ..Default::default()
    }
}

/// The contract for `count` buses of a used offer (or a demonstrator, or a batch) at `price`
/// each: delivered at once. A special offer of new buses is `draft_new`'s.
pub fn draft_offer(c: &Company, o: &Offer, count: u32, price: Cents, extras: &[Extra], livery: &str) -> Contract {
    if o.is_new() {
        return draft_new(c, &o.listing, count.min(o.count), price, extras, livery, Some(o));
    }
    let t = terms(c.difficulty);
    let painting = if livery.is_empty() || extras.contains(&Extra::Painting) { 0 } else { painting_cost(c) };
    // (a demonstrator keeps what is left of its new warranty)
    let base = if o.kind == OfferKind::Demonstrator { t.warranty_new.saturating_sub((o.age_years(&c.date) * 12.0).round() as u32).max(t.warranty_used) } else { t.warranty_used };
    Contract {
        seller: o.seller.clone(),
        buyer: c.name.clone(),
        listing: o.listing.clone(),
        livery: livery.to_string(),
        count: count.clamp(1, o.count.max(1)),
        new: false,
        built: o.built.clone(),
        km: o.km,
        condition: o.condition,
        offer: Some(o.id.clone()),
        list: o.price,
        price,
        grant: 0,
        painting,
        pay: PayWay::Cash,
        delivery_days: 0,
        warranty_months: base + if extras.contains(&Extra::Warranty) { EXTRA_WARRANTY_MONTHS } else { 0 },
        extras: extras.iter().copied().filter(|e| *e != Extra::FastDelivery).collect(),
        ..Default::default()
    }
}

/// A signed contract waiting for its delivery.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Order {
    pub contract: Contract,
    pub delivery: String,
}

/// What `sign` did: the buses that joined the fleet now, or the order that brings them.
#[derive(Clone, Debug, PartialEq)]
pub enum Signed {
    Delivered(Vec<u32>),
    Ordered { no: u32, delivery: String },
}

/// Whether there is room at the depot for `n` more buses (with those ordered).
pub fn room_for(c: &Company, n: usize) -> Result<(), &'static str> {
    let held = c.fleet.iter().filter(|v| v.held_on(&c.date)).count();
    let pending: usize = c.dealer.orders.iter().map(|o| o.contract.count as usize).sum();
    let cap = c.site.spaces() + levels::extra_places(c) as usize + super::depot::OUTSIDE_MAX;
    if held + pending + n > cap {
        return Err("The depot has no room for so many buses: build more parking spaces.");
    }
    Ok(())
}

fn can_have(c: &Company, l: &Listing) -> Result<(), &'static str> {
    market::kind_allowed(c, l.bus.kind)
}

/// Pay `amount` (with `security` the bank's for a loan).
fn pay(c: &mut Company, amount: Cents, how: Payment, what: &str, security: Cents) -> Result<(), &'static str> {
    match how {
        Payment::Cash if c.cash < amount => Err("Not enough cash."),
        Payment::Cash => Ok(()),
        Payment::Loan => finance::take_loan(c, amount, what, security).map(|_| ()),
    }
}

/// Sign a contract at `now`: it is paid (or the lease agreed) and booked, and its buses join
/// the fleet - at once when they are used or in stock with no time to wait, else on their
/// delivery day (`tick`).
pub fn sign(c: &mut Company, k: &Contract, listings: &[Listing], now: &str) -> Result<Signed, &'static str> {
    if !k.is_signed() {
        return Err("Sign the contract first.");
    }
    if k.count == 0 {
        return Err("Choose how many buses.");
    }
    can_have(c, &k.listing)?;
    room_for(c, k.count as usize)?;
    if let Some(id) = &k.offer {
        let Some(o) = offer(c, listings, id, now) else { return Err("This offer has ended or is sold.") };
        if o.count < k.count {
            return Err("Not that many buses are left of this offer.");
        }
    }
    let name = k.listing.bus.name.clone();
    match k.pay {
        PayWay::Cash => pay(c, k.due(), Payment::Cash, &name, k.total())?,
        PayWay::Loan => pay(c, k.due(), Payment::Loan, &name, k.total())?,
        PayWay::Lease => {
            if !k.new {
                return Err("Only new buses can be leased.");
            }
            let (monthly, _, _) = k.lease(c);
            // (the leasing company wants to see a month's rates in the bank)
            if c.cash < monthly * k.count as Cents {
                return Err("Not enough cash.");
            }
        }
    }
    let mut k = k.clone();
    c.dealer.counter += 1;
    k.no = c.dealer.counter;
    k.signed_at = now.to_string();
    if k.pay != PayWay::Lease {
        let what = if k.new { format!("{} × {} (new, contract {})", k.count, name, k.no) } else { format!("{} × {} (used, contract {})", k.count, name, k.no) };
        c.book(BookingKind::Purchase, -k.total(), what, false);
        c.book(BookingKind::Subsidy, k.grants(), name.clone(), false);
    }
    if let Some(id) = &k.offer {
        let day = id.get(1..).and_then(|s| s.split('-').next()).and_then(|d| d.parse().ok()).unwrap_or(0);
        c.dealer.taken.push(Taken { id: id.clone(), day, count: k.count });
    }
    c.dealer.bought += k.count;
    bought_of(c, &k.listing.maker, k.count, k.new, k.total());
    c.dealer.contracts.push(k.clone());
    if c.dealer.contracts.len() > CONTRACTS_KEPT {
        c.dealer.contracts.remove(0);
    }
    c.dealer.talks.retain(|t| !(t.quote.key == format!("new:{}", k.listing.bus.file) || k.offer.as_deref() == Some(t.quote.key.as_str())));
    if k.delivery_days <= 0 {
        return Ok(Signed::Delivered(deliver(c, &k)));
    }
    let delivery = k.delivery(now);
    let no = k.no;
    c.dealer.orders.push(Order { contract: k, delivery: delivery.clone() });
    Ok(Signed::Ordered { no, delivery })
}

/// Sign a purchase paid with a loan: the loan contract and the purchase are signed together,
/// or neither is (the purchase is kept as paid by the bank).
pub fn sign_financed(c: &mut Company, k: &Contract, loan: &super::finance::LoanContract, listings: &[Listing], now: &str) -> Result<(u32, Signed), &'static str> {
    super::finance::with_loan(c, loan, k.total(), |c| {
        let mut paid = k.clone();
        paid.pay = PayWay::Cash;
        let done = sign(c, &paid, listings, now)?;
        if let Some(last) = c.dealer.contracts.last_mut() {
            last.pay = PayWay::Loan;
        }
        Ok(done)
    })
}

/// How many signed contracts the company keeps.
pub const CONTRACTS_KEPT: usize = 60;

/// A contract's buses into the fleet, with their warranty and free service. Returns their ids.
fn deliver(c: &mut Company, k: &Contract) -> Vec<u32> {
    let r = economy::rules(c.difficulty);
    let new_value = economy::new_price(k.listing.bus.kind, &r, c.price_index);
    let built = if k.new { c.date.clone() } else { k.built.clone() };
    let mut ids = Vec::new();
    for _ in 0..k.count {
        let tenure = match k.pay {
            PayWay::Lease => {
                let (monthly, months, residual) = k.lease(c);
                Tenure::Leased { monthly, until: dates::add(&c.date, (months as f64 * 30.44).round() as i64), residual }
            }
            _ => Tenure::Owned { paid: k.price + k.painting - k.grant, new_value },
        };
        let id = market::add_vehicle(c, &k.listing.bus, built.clone(), k.km, k.condition, tenure, &k.livery);
        let until = dates::add(&c.date, (k.warranty_months as f64 * 30.44).round() as i64);
        c.dealer.warranties.push(Warranty { vehicle: id, until });
        if k.extras.contains(&Extra::FreeService) {
            c.dealer.free_services.push(id);
        }
        ids.push(id);
    }
    // (the model's introduction: two drivers learn it from the dealer's man)
    if k.extras.contains(&Extra::Introduction) {
        super::licences::introduction(c, &k.listing.bus.file);
    }
    ids
}

/// A delivery `tick` made.
#[derive(Clone, Debug, PartialEq)]
pub struct Delivered {
    pub contract: u32,
    pub name: String,
    pub numbers: Vec<String>,
    pub ids: Vec<u32>,
}

/// What the dealer does by `now` (the company clock calls it; the pages too until it does):
/// the orders due are delivered, and what is over is forgotten (offers sold days ago, talks
/// of other days, a sulk that ended, warranties that ran out).
pub fn tick(c: &mut Company, now: &str) -> Vec<Delivered> {
    // (a company from before the dealers kept their customers: its contracts tell its standing)
    if c.dealer.relations.is_empty() && !c.dealer.contracts.is_empty() {
        for k in c.dealer.contracts.clone() {
            bought_of(c, &k.listing.maker, k.count, k.new, k.total());
        }
    }
    let mut out = Vec::new();
    let due: Vec<Order> = c.dealer.orders.iter().filter(|o| not_after(&o.delivery, now)).cloned().collect();
    c.dealer.orders.retain(|o| !not_after(&o.delivery, now));
    for o in due {
        let ids = deliver(c, &o.contract);
        let numbers = ids.iter().filter_map(|id| c.vehicle(*id).map(|v| v.number.clone())).collect();
        out.push(Delivered { contract: o.contract.no, name: o.contract.listing.bus.name.clone(), numbers, ids });
    }
    let today = dates::parse(&day_of(now)).unwrap_or(0);
    let date = day_of(now);
    c.dealer.taken.retain(|t| t.day >= today - 14);
    c.dealer.talks.retain(|t| t.day == date);
    c.dealer.breaks.retain(|b| !not_after(&b.1, now));
    c.dealer.warranties.retain(|w| dates::between(&date, &w.until) >= 0 && c.fleet.iter().any(|v| v.id == w.vehicle));
    c.dealer.free_services.retain(|id| c.fleet.iter().any(|v| v.id == *id));
    out
}

/// The quick buy: `count` new buses of a listing at the quick price (the list price less the
/// dealer's discount without a talk, `quick_price`), paid now (cash or loan), in the fleet at
/// once. Returns their ids.
pub fn quick_buy(c: &mut Company, l: &Listing, count: u32, how: Payment, livery: &str) -> Result<Vec<u32>, &'static str> {
    let mut k = draft_new(c, l, count, quick_price(c, l, count), &[], livery, None);
    k.delivery_days = 0;
    k.signed_by = c.name.clone();
    k.pay = if how == Payment::Loan { PayWay::Loan } else { PayWay::Cash };
    let now = now_of(c);
    match sign(c, &k, &[], &now)? {
        Signed::Delivered(ids) => Ok(ids),
        Signed::Ordered { .. } => Ok(Vec::new()),
    }
}

/// The quick buy of an offer at its price: `count` of it, in the fleet at once (new ones from
/// stock too).
pub fn quick_buy_offer(c: &mut Company, o: &Offer, count: u32, how: Payment, livery: &str, listings: &[Listing]) -> Result<Vec<u32>, &'static str> {
    let mut k = draft_offer(c, o, count, o.price, &[], livery);
    k.delivery_days = 0;
    k.signed_by = c.name.clone();
    k.pay = if how == Payment::Loan { PayWay::Loan } else { PayWay::Cash };
    let now = now_of(c);
    match sign(c, &k, listings, &now)? {
        Signed::Delivered(ids) => Ok(ids),
        Signed::Ordered { .. } => Ok(Vec::new()),
    }
}

/// A bus still under the dealer's warranty on `date`: its repairs are free.
pub fn under_warranty(c: &Company, vehicle: u32, date: &str) -> bool {
    c.dealer.warranties.iter().any(|w| w.vehicle == vehicle && dates::between(date, &w.until) >= 0)
}

/// The buses under warranty on `date`.
pub fn warranted(c: &Company, date: &str) -> Vec<u32> {
    c.dealer.warranties.iter().filter(|w| dates::between(date, &w.until) >= 0).map(|w| w.vehicle).collect()
}

/// A bus's first service is the dealer's: true once (and it is used up).
pub fn take_free_service(c: &mut Company, vehicle: u32) -> bool {
    let had = c.dealer.free_services.contains(&vehicle);
    c.dealer.free_services.retain(|v| *v != vehicle);
    had
}

// --- the test drive -----------------------------------------------------------------------------

/// A test drive under way: the game runs a free drive with this bus, which is not the
/// company's time (the company clock and the books leave it out).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct TestDrive {
    pub bus: String,
    pub name: String,
    pub livery: String,
    /// The offer it was for, if any.
    pub offer: Option<String>,
    pub started: String,
}

/// Mark a test drive as begun.
pub fn start_test_drive(c: &mut Company, bus: &str, name: &str, livery: &str, offer: Option<String>, now: &str) {
    c.dealer.test_drive = Some(TestDrive { bus: bus.to_string(), name: name.to_string(), livery: livery.to_string(), offer, started: now.to_string() });
}

/// The test drive is over (the player is back at the dealer).
pub fn end_test_drive(c: &mut Company) -> Option<TestDrive> {
    c.dealer.test_drive.take()
}

// --- what the company keeps -----------------------------------------------------------------------

/// How the dealer is shown first: the quick buy, or haggling and a contract.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BuyingMode {
    Simple,
    #[default]
    Advanced,
}

impl BuyingMode {
    pub const ALL: [BuyingMode; 2] = [BuyingMode::Simple, BuyingMode::Advanced];

    pub fn label(self) -> &'static str {
        match self {
            BuyingMode::Simple => "Simple",
            BuyingMode::Advanced => "Advanced",
        }
    }
}

/// An offer's buses bought.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Taken {
    pub id: String,
    pub day: i64,
    pub count: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Warranty {
    pub vehicle: u32,
    pub until: String,
}

/// The company's dealings with the dealer.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct DealerState {
    #[serde(default)]
    pub mode: BuyingMode,
    #[serde(default)]
    pub taken: Vec<Taken>,
    #[serde(default)]
    pub orders: Vec<Order>,
    #[serde(default)]
    pub talks: Vec<Talk>,
    /// Dealers (by maker) that broke off, and until when.
    #[serde(default)]
    pub breaks: Vec<(String, String)>,
    /// Buses bought of the dealers, all together.
    #[serde(default)]
    pub bought: u32,
    /// The company's standing with each maker's dealer.
    #[serde(default)]
    pub relations: Vec<Relation>,
    #[serde(default)]
    pub warranties: Vec<Warranty>,
    #[serde(default)]
    pub free_services: Vec<u32>,
    /// The signed contracts (the last `CONTRACTS_KEPT`).
    #[serde(default)]
    pub contracts: Vec<Contract>,
    #[serde(default)]
    pub test_drive: Option<TestDrive>,
    /// The last contract number given.
    #[serde(default)]
    pub counter: u32,
    /// The loan contracts the company signed with the bank (the last sixty; see `finance`).
    #[serde(default)]
    pub loan_contracts: Vec<super::finance::LoanContract>,
}

#[cfg(test)]
mod tests {
    use super::super::{found, Founding};
    use super::*;

    fn company(d: Difficulty, date: &str) -> Company {
        let mut c = found(&Founding { name: "Stadtbus".into(), short: "SB".into(), difficulty: d, date: date.into(), ..Default::default() }, "Luc");
        c.progress.xp = levels::LEVEL_XP[9];
        c
    }

    fn listing(name: &str, maker: &str, size: BusSize, drive: Drive) -> Listing {
        Listing {
            bus: MarketBus { file: format!("Vehicles/{name}/{name}.bus"), name: name.into(), kind: BusKind { size, drive }, paints: vec!["Red".into()], default_paint: "Red".into() },
            maker: maker.into(),
            model: name.into(),
            version: "3 doors".into(),
            seats: Some(32),
            standing: Some(60),
        }
    }

    #[test]
    fn moments_are_read_and_written() {
        assert_eq!(minutes_of("1970-01-02 01:30"), Some(1440 + 90));
        assert_eq!(minutes_of("1970-01-02"), Some(1440));
        assert_eq!(minutes_of("1970-01-02T00:05:30"), Some(1445));
        assert_eq!(minutes_of("2024-02-30 10:00"), None);
        assert_eq!(later("2024-02-28 23:30", 60), "2024-02-29 00:30");
        assert_eq!(at("2024-03-04", 8 * 60), "2024-03-04 08:00");
        assert!(not_after("2024-03-04 08:00", "2024-03-04 12:00") && !not_after("2024-03-05 08:00", "2024-03-04 12:00"));
    }

    #[test]
    fn every_bus_is_to_be_had_new_at_any_date() {
        // (no model years: a company of 1985 buys an eCitaro new, and an old one's used ones
        // are of some age)
        let c = company(Difficulty::Realistic, "1985-06-01");
        let ls = vec![listing("SD202", "MAN", BusSize::Double, Drive::Diesel), listing("eCitaro", "Mercedes-Benz", BusSize::Solo, Drive::Electric), listing("Citaro", "Mercedes-Benz", BusSize::Solo, Drive::Diesel)];
        let f = Filter::default();
        assert!(ls.iter().all(|l| f.fits(l, &c)));
        assert!(Filter { search: "citaro".into(), size: Some(BusSize::Solo), ..Default::default() }.fits(&ls[2], &c));
        assert!(!Filter { drive: Some(Drive::Diesel), ..Default::default() }.fits(&ls[1], &c));
        assert!(!Filter { max_price: 200_000_00, ..Default::default() }.fits(&ls[2], &c));
        let mut c2 = c.clone();
        assert!(quick_buy(&mut c2, &ls[1], 1, Payment::Cash, "").is_ok());
        let mut seen = 0;
        for d in 0..20 {
            let mut c = c.clone();
            c.date = dates::add(&c.date, d);
            for o in used_market(&c, &ls, &now_of(&c)) {
                let age = 1985 - year_of(&o.built);
                assert!((1..=16).contains(&age), "{age}");
                assert!(o.km > 50_000.0 && o.price > 0);
                seen += 1;
            }
        }
        assert!(seen > 0);
    }

    #[test]
    fn the_offers_change_every_day_and_expire() {
        let c = company(Difficulty::Realistic, "2024-03-04");
        let ls = vec![listing("Citaro", "Mercedes-Benz", BusSize::Solo, Drive::Diesel), listing("Lion's City", "MAN", BusSize::Solo, Drive::Diesel), listing("Urbino 18", "Solaris", BusSize::Articulated, Drive::Diesel)];
        let now = now_of(&c);
        let today = day_offers(&c, &ls, &now);
        assert!(!today.is_empty());
        // the same at the same moment, others the next day
        assert_eq!(day_offers(&c, &ls, &now), today);
        let mut next = c.clone();
        next.date = dates::add(&c.date, 1);
        let tomorrow = day_offers(&next, &ls, &now_of(&next));
        assert_ne!(tomorrow, today);
        // every offer open now, of a kind that fits
        let n = minutes_of(&now).unwrap();
        for o in today.iter().chain(used_market(&c, &ls, &now).iter()) {
            assert!(minutes_of(&o.published).unwrap() <= n && minutes_of(&o.expires).unwrap() > n);
            assert!(o.count >= 1 && o.price > 0);
            match o.kind {
                OfferKind::Discount => assert!(o.price < o.reference && o.km == 0.0),
                OfferKind::Demonstrator => assert!(o.price < o.reference && o.km < 40_000.0 && o.condition >= 93.0),
                OfferKind::Batch => assert!(o.count >= 3 && o.km > 100_000.0),
                OfferKind::Used => assert_eq!(o.seller, USED_CENTRE),
            }
        }
        // ten days later none of today's is open
        let mut later_c = c.clone();
        later_c.date = dates::add(&c.date, 10);
        let ids: Vec<String> = today.iter().map(|o| o.id.clone()).collect();
        assert!(day_offers(&later_c, &ls, &now_of(&later_c)).iter().all(|o| !ids.contains(&o.id)));
    }

    /// A talk, made the same way every time.
    fn haggle(c: &mut Company, q: &Quote, moves: &[Move]) -> (Talk, Vec<Reply>) {
        let now = now_of(c);
        let mut t = open_talk(c, q, &now).unwrap();
        let mut out = Vec::new();
        for m in moves {
            out.push(respond(c, &mut t, *m, &now));
            if t.closed {
                break;
            }
        }
        (t, out)
    }

    #[test]
    fn haggling_can_bring_the_price_down_but_not_below_the_floor() {
        let mut c = company(Difficulty::Realistic, "2024-03-04");
        let l = listing("Citaro", "Mercedes-Benz", BusSize::Solo, Drive::Diesel);
        let q = Quote::new_bus(&c, &l, 1);
        assert_eq!(q.list, 280_000_00);
        // the room: a margin of some per cent, more for several buses and a customer of standing
        let r1 = room(&c, &q);
        assert!((0.03..0.2).contains(&r1), "{r1}");
        assert!(room(&c, &Quote { count: 4, ..q.clone() }) > r1 + 0.04);
        assert!(room(&c, &Quote { kind: BusKind { size: BusSize::Solo, drive: Drive::Electric }, ..q.clone() }) < room(&c, &q));
        // asking for a discount a few times: lower, never under the floor
        let (t, replies) = haggle(&mut c, &q, &[Move::AskDiscount, Move::AskDiscount, Move::AskDiscount]);
        assert!(t.asking <= q.list && t.asking >= t.floor && t.floor < q.list);
        assert!(replies.iter().all(|r| matches!(r, Reply::Discount(_) | Reply::Firm | Reply::LastOffer(_))));
        // the talk goes on the same day where it stopped
        let again = open_talk(&c, &q, &now_of(&c)).unwrap();
        assert_eq!((again.rounds, again.asking), (t.rounds, t.asking));
        // an offer at the floor is taken or met: the price is never below it
        let mut c2 = company(Difficulty::Realistic, "2024-03-05");
        let q2 = Quote::new_bus(&c2, &l, 1);
        let floor = open_talk(&c2, &q2, &now_of(&c2)).unwrap().floor;
        let (t2, r2) = haggle(&mut c2, &q2, &[Move::Offer(floor), Move::Offer(floor), Move::Offer(floor), Move::Offer(floor), Move::Accept]);
        assert!(t2.agreed || t2.closed);
        assert!(t2.asking >= floor && t2.asking <= q2.list);
        assert!(r2.iter().any(|r| matches!(r, Reply::Accepted(_) | Reply::Counter(_) | Reply::LastOffer(_))));
        // the rounds run out: his last word
        let mut c3 = company(Difficulty::Hard, "2024-03-06");
        let q3 = Quote::new_bus(&c3, &l, 1);
        let (t3, r3) = haggle(&mut c3, &q3, &[Move::AskDiscount; 10]);
        assert!(t3.closed && t3.rounds <= t3.max_rounds);
        assert!(matches!(r3.last(), Some(Reply::LastOffer(_)) | Some(Reply::BrokeOff(_))));
        // his last word can still be taken (not once he broke off)
        let mut t3 = t3;
        let now = now_of(&c3);
        let taken = respond(&mut c3, &mut t3, Move::Accept, &now);
        match r3.last() {
            Some(Reply::LastOffer(p)) => assert!(taken == Reply::Accepted(*p) && t3.agreed),
            _ => assert!(!t3.agreed),
        }
    }

    #[test]
    fn pushing_too_hard_makes_the_dealer_break_off() {
        let mut c = company(Difficulty::Hard, "2024-03-04");
        let l = listing("Citaro", "Mercedes-Benz", BusSize::Solo, Drive::Diesel);
        let q = Quote::new_bus(&c, &l, 1);
        let (t, replies) = haggle(&mut c, &q, &[Move::Offer(100_000_00), Move::Offer(100_000_00), Move::Offer(100_000_00)]);
        assert!(t.closed && !t.agreed);
        let Some(Reply::BrokeOff(until)) = replies.last() else { panic!("{replies:?}") };
        assert_eq!(day_of(until), "2024-03-08");
        // no talks with that maker's dealer until then; the quick buy still sells at list
        assert!(open_talk(&c, &q, &now_of(&c)).is_err());
        assert!(sulking(&c, "mercedes-benz", &now_of(&c)).is_some());
        let other = listing("Lion's City", "MAN", BusSize::Solo, Drive::Diesel);
        assert!(open_talk(&c, &Quote::new_bus(&c, &other, 1), &now_of(&c)).is_ok());
        assert!(quick_buy(&mut c, &l, 1, Payment::Cash, "").is_ok());
        c.date = "2024-03-09".into();
        let now = now_of(&c);
        tick(&mut c, &now);
        assert!(open_talk(&c, &q, &now_of(&c)).is_ok() && c.dealer.breaks.is_empty());
    }

    #[test]
    fn one_bus_gets_less_off_than_an_order_and_a_regular_customer_more() {
        let mut c = company(Difficulty::Realistic, "2024-03-04");
        let l = listing("Citaro", "Mercedes-Benz", BusSize::Solo, Drive::Diesel);
        let at = |c: &Company, n: u32| room(c, &Quote::new_bus(c, &l, n));
        // one bus alone: about half of what three get; five a bulk discount, ten a fleet order's
        assert!(at(&c, 1) < at(&c, 2) && at(&c, 2) < at(&c, 3));
        assert!(at(&c, 1) < at(&c, 3) * 0.75);
        assert!(at(&c, 5) > at(&c, 4) + 0.02 && at(&c, 10) > at(&c, 5) + 0.02);
        assert_eq!(bulk_step(1).1, Some(2));
        assert_eq!(bulk_step(7), ("A bulk order: a bulk discount", Some(10)));
        // the quick buy: a part of it, a bus of ten cheaper than one alone
        assert!(quick_price(&c, &l, 10) < quick_price(&c, &l, 1));
        // buying of him makes a known, then a regular customer of the company, with more room
        assert_eq!(Standing::of(relation(&c, "Mercedes-Benz").points), Standing::New);
        let before = at(&c, 3);
        c.cash = 10_000_000_00;
        assert!(quick_buy(&mut c, &l, 2, Payment::Cash, "").is_ok());
        assert_eq!(Standing::of(relation(&c, "mercedes-benz").points), Standing::Known);
        assert!(at(&c, 3) > before);
        assert!(quick_buy(&mut c, &l, 4, Payment::Cash, "").is_ok());
        let r = relation(&c, "Mercedes-Benz");
        assert_eq!((r.bought, Standing::of(r.points)), (6, Standing::Regular));
        assert!(r.spent > 0);
        // another maker's dealer does not know the company
        assert_eq!(relation(&c, "MAN").bought, 0);
        // a talk he broke off costs standing
        let q = Quote::new_bus(&c, &l, 1);
        let (_, replies) = haggle(&mut c, &q, &[Move::Offer(1_00); 12]);
        assert!(matches!(replies.last(), Some(Reply::BrokeOff(_))), "{replies:?}");
        assert!(relation(&c, "Mercedes-Benz").points < r.points);
    }

    #[test]
    fn a_company_from_before_gets_its_standing_from_its_contracts() {
        let mut c = company(Difficulty::Easy, "2024-03-04");
        let l = listing("Lion's City", "MAN", BusSize::Solo, Drive::Diesel);
        let mut k = draft_new(&c, &l, 3, 250_000_00, &[], "", None);
        k.signed_by = "Luc".into();
        let now = now_of(&c);
        sign(&mut c, &k, &[], &now).unwrap();
        c.dealer.relations.clear();
        tick(&mut c, &now);
        assert_eq!(relation(&c, "MAN").bought, 3);
    }

    #[test]
    fn extras_are_granted_from_what_the_dealer_can_give() {
        let mut c = company(Difficulty::Easy, "2024-03-04");
        let l = listing("Citaro", "Mercedes-Benz", BusSize::Solo, Drive::Diesel);
        let q = Quote::new_bus(&c, &l, 2);
        let now = now_of(&c);
        let mut t = open_talk(&c, &q, &now).unwrap();
        let floor = t.floor;
        let mut granted = 0;
        for e in Extra::ALL {
            if respond(&mut c, &mut t, Move::AskExtra(e), &now) == Reply::ExtraGranted(e) {
                granted += 1;
            }
        }
        assert!(granted >= 1, "{:?}", t.replies);
        assert_eq!(t.extras.len(), granted);
        assert!(t.floor > floor && t.floor <= t.asking);
        // they go into the contract: painting free, the warranty longer, delivery faster
        let k = draft_new(&c, &l, 2, t.asking, &[Extra::Painting, Extra::Warranty, Extra::FastDelivery], "Blue", None);
        let plain = draft_new(&c, &l, 2, t.asking, &[], "Blue", None);
        assert_eq!(k.painting, 0);
        assert_eq!(plain.painting, painting_cost(&c));
        assert_eq!(k.warranty_months, plain.warranty_months + EXTRA_WARRANTY_MONTHS);
        assert!(k.delivery_days < plain.delivery_days);
    }

    #[test]
    fn a_signed_contract_is_paid_and_its_buses_come_on_their_day() {
        let mut c = company(Difficulty::Realistic, "2024-03-04");
        let l = listing("Citaro", "Mercedes-Benz", BusSize::Solo, Drive::Diesel);
        let mut k = draft_new(&c, &l, 2, 260_000_00, &[Extra::FreeService], "", None);
        // unsigned: nothing
        let now0 = now_of(&c);
        assert_eq!(sign(&mut c, &k, &[], &now0), Err("Sign the contract first."));
        k.signed_by = "Luc Ruigrok".into();
        let cash = c.cash;
        let now = now_of(&c);
        let Ok(Signed::Ordered { no, delivery }) = sign(&mut c, &k, &[], &now) else { panic!() };
        assert_eq!(no, 1);
        assert_eq!(c.cash, cash - 520_000_00);
        assert!(c.fleet.is_empty() && c.dealer.orders.len() == 1);
        assert_eq!(c.dealer.bought, 2);
        assert_eq!(c.dealer.contracts[0].signed_by, "Luc Ruigrok");
        // not yet the day before; on its morning it comes
        let day = day_of(&delivery);
        assert!(tick(&mut c, &later(&delivery, -60)).is_empty());
        let d = tick(&mut c, &at(&day, 9 * 60));
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].numbers, vec!["101".to_string(), "102".to_string()]);
        assert!(c.dealer.orders.is_empty() && c.fleet.len() == 2);
        let id = c.fleet[0].id;
        assert!(under_warranty(&c, id, &day) && !under_warranty(&c, id, &dates::add(&day, 800)));
        assert!(take_free_service(&mut c, id) && !take_free_service(&mut c, id));
        // a lease: no price now, a rate a month
        let mut lk = draft_new(&c, &l, 1, 270_000_00, &[], "", None);
        lk.pay = PayWay::Lease;
        lk.delivery_days = 0;
        lk.strokes = vec![vec![[0.1, 0.5], [0.4, 0.4], [0.8, 0.6]]];
        let cash = c.cash;
        let Ok(Signed::Delivered(ids)) = sign(&mut c, &lk, &[], &now) else { panic!() };
        assert_eq!(c.cash, cash);
        assert!(matches!(c.vehicle(ids[0]).unwrap().tenure, Tenure::Leased { .. }));
        // a purchase on a loan: both signed, or neither
        let mut bk = draft_new(&c, &l, 1, 270_000_00, &[], "", None);
        bk.signed_by = "Luc".into();
        bk.pay = PayWay::Loan;
        bk.delivery_days = 0;
        let loan = super::super::finance::LoanContract { signed_by: "Luc".into(), ..super::super::finance::draft_loan(&c, bk.due(), 48, "Citaro", bk.total()) };
        let (debt, cash) = (c.debt(), c.cash);
        let (_, done) = sign_financed(&mut c, &bk, &loan, &[], &now).unwrap();
        assert!(matches!(done, Signed::Delivered(_)));
        assert_eq!((c.debt(), c.cash), (debt + bk.due(), cash));
        assert_eq!(c.dealer.contracts.last().unwrap().pay, PayWay::Loan);
        // (an articulated bus a company of the first level may not buy: no loan either)
        let mut low = c.clone();
        low.progress.xp = 0;
        let before = low.clone();
        let big = listing("Citaro G", "Mercedes-Benz", BusSize::Articulated, Drive::Diesel);
        let mut gk = draft_new(&low, &big, 1, 400_000_00, &[], "", None);
        gk.signed_by = "Luc".into();
        gk.pay = PayWay::Loan;
        let loan = super::super::finance::LoanContract { signed_by: "Luc".into(), ..super::super::finance::draft_loan(&low, 10_000_00, 12, "x", 0) };
        assert!(sign_financed(&mut low, &gk, &loan, &[], &now).is_err());
        assert_eq!(low, before);
    }


    #[test]
    fn the_introduction_teaches_two_drivers_the_new_model() {
        use super::super::licences::{qualified_drivers, type_key};
        let mut c = company(Difficulty::Realistic, "2024-03-04");
        c.cash += 1_000_000_00;
        for k in 0..3 {
            let a = super::super::staff::applicants(&c)[k].clone();
            super::super::staff::hire(&mut c, &a).unwrap();
        }
        let l = listing("Urbino", "Solaris", BusSize::Solo, Drive::Diesel);
        assert_eq!(extra_value(&c, Extra::Introduction, 260_000_00), 500_00);
        let now = now_of(&c);
        // without it: nobody may drive the new model
        let mut plain = draft_new(&c, &l, 1, 260_000_00, &[], "", None);
        (plain.signed_by, plain.delivery_days) = ("Luc".into(), 0);
        let mut first = c.clone();
        let Ok(Signed::Delivered(ids)) = sign(&mut first, &plain, &[], &now) else { panic!() };
        let v = first.vehicle(ids[0]).unwrap().clone();
        assert_eq!(qualified_drivers(&first, v.kind, &v.bus), (0, 3));
        // with it: the two most experienced drivers
        let mut k = draft_new(&c, &l, 1, 260_000_00, &[Extra::Introduction], "", None);
        (k.signed_by, k.delivery_days) = ("Luc".into(), 0);
        let Ok(Signed::Delivered(ids)) = sign(&mut c, &k, &[], &now) else { panic!() };
        let v = c.vehicle(ids[0]).unwrap().clone();
        assert_eq!(qualified_drivers(&c, v.kind, &v.bus), (2, 3));
        let least = c.staff.iter().min_by(|a, b| a.experience.total_cmp(&b.experience)).unwrap();
        assert!(!least.types.contains(&type_key(&v.bus)));
    }

    #[test]
    fn an_offer_is_bought_once_and_used_ones_come_at_once() {
        let mut c = company(Difficulty::Realistic, "2024-03-04");
        let ls = vec![listing("Citaro", "Mercedes-Benz", BusSize::Solo, Drive::Diesel)];
        let now = now_of(&c);
        let used = used_market(&c, &ls, &now);
        let o = used[0].clone();
        let mut k = draft_offer(&c, &o, 1, o.price, &[], "");
        k.signed_by = "Luc".into();
        let Ok(Signed::Delivered(ids)) = sign(&mut c, &k, &ls, &now) else { panic!() };
        let v = c.vehicle(ids[0]).unwrap();
        assert_eq!((v.built.as_str(), v.km), (o.built.as_str(), o.km));
        assert!(used_market(&c, &ls, &now).iter().all(|x| x.id != o.id));
        assert_eq!(sign(&mut c, &k, &ls, &now), Err("This offer has ended or is sold."));
        // a used bus cannot be leased
        let o2 = used_market(&c, &ls, &now)[0].clone();
        let mut k2 = draft_offer(&c, &o2, 1, o2.price, &[], "");
        k2.signed_by = "Luc".into();
        k2.pay = PayWay::Lease;
        assert_eq!(sign(&mut c, &k2, &ls, &now), Err("Only new buses can be leased."));
        // the quick buy of an offer and of a model: at once
        let ids = quick_buy_offer(&mut c, &o2, 1, Payment::Cash, "", &ls).unwrap();
        assert_eq!(ids.len(), 1);
        c.cash = 5_000_000_00;
        let before = c.fleet.len();
        let cash = c.cash;
        let one = quick_price(&c, &ls[0], 3);
        assert!(one < 280_000_00);
        let ids = quick_buy(&mut c, &ls[0], 3, Payment::Cash, "Red").unwrap();
        assert_eq!(c.fleet.len(), before + 3);
        assert_eq!(cash - c.cash, 3 * (one + painting_cost(&c)));
        assert_eq!(c.vehicle(ids[2]).unwrap().livery, "Red");
    }

    #[test]
    fn there_is_no_buying_beyond_the_depot() {
        let mut c = company(Difficulty::Easy, "2024-03-04");
        let l = listing("Citaro", "Mercedes-Benz", BusSize::Solo, Drive::Diesel);
        let cap = c.site.spaces() + levels::extra_places(&c) as usize + super::super::depot::OUTSIDE_MAX;
        assert!(room_for(&c, cap).is_ok() && room_for(&c, cap + 1).is_err());
        let mut k = draft_new(&c, &l, 2, 250_000_00, &[], "", None);
        k.signed_by = "Luc".into();
        let now = now_of(&c);
        sign(&mut c, &k, &[], &now).unwrap();
        // (the ones ordered count)
        assert!(room_for(&c, cap - 2).is_ok() && room_for(&c, cap - 1).is_err());
    }

    #[test]
    fn a_test_drive_is_marked_and_ended() {
        let mut c = company(Difficulty::Realistic, "2024-03-04");
        let now = now_of(&c);
        start_test_drive(&mut c, "Vehicles/Citaro/Citaro.bus", "Citaro", "Red", None, &now);
        assert!(c.dealer.test_drive.is_some());
        assert_eq!(end_test_drive(&mut c).map(|t| t.name), Some("Citaro".to_string()));
        assert!(c.dealer.test_drive.is_none());
    }
}
