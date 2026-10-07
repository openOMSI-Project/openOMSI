//! The company's own lines: a line the player makes in the line editor for the company, pays
//! for and then runs (Luc: "deze lijn moet ook geld kosten ... gebaseerd op realistische
//! factoren"), and how the time of day shapes it - the rush hours and the quiet ones, and
//! which bus each of them wants.
//!
//! The reference is a German commercial line service (`eigenwirtschaftlicher Linienverkehr`)
//! under the Passenger Transport Act (PBefG). The figures are rounded reference values for
//! "Realistic"; "Easy" costs about half and draws more passengers, "Hard" about 1.4 times and
//! draws fewer (`costs`).
//!
//! **Once, when the line is confirmed** (`Estimate::one_off`):
//! - *The line licence* (§§ 9, 13, 42 PBefG): the authority examines the application and
//!   hears the municipalities, the chamber of commerce and the other operators (§ 14). The
//!   fee under the states' fee ordinances is a few hundred euros, the application itself (route
//!   plan, timetable, the operator's proof of reliability and means) costs the operator more:
//!   €2,000 and €150 per route kilometre.
//! - *Timetable and passenger information*: a printed timetable in a display case and the
//!   line's plate on the stop's sign, about €180 a stop; live departure displays
//!   (`LineDesign::live_displays`) at the transfer stops - an LED or TFT display with its mast,
//!   power and data link costs cities €8,000-20,000 installed: €12,000 each; producing the
//!   timetable (planning, printing, the timetable book and the online data) €1,200 per group
//!   of days it runs on.
//! - *The fare association* (Verkehrsverbund): joining with the company's first own line -
//!   the revenue-sharing contract, its tariff in the buses' ticket machines - €6,000; each
//!   later line is taken into the tariff for €800.
//! - *Launch marketing*: flyers to the households along the line, advertisements, an opening
//!   day: €1,000 and €2 per passenger expected on a working day.
//!
//! A route changed later is approved anew (§ 13 PBefG for a changed licence): €600 and the
//! passenger information of every stop that is new to the line (`change_fee`).
//!
//! **Every month**: the stop usage fee to the road authority (cities and the owners of bus
//! stations charge for their stops, here a flat €25 a stop, `month_end`); the fare
//! association's share of the line's fares for its sales, clearing and administration, 3-10 %
//! in German associations, 6 % here (`association_share`, booked with the fares); the
//! licence, the yearly fee spread over the months (`concessions::LICENCE`, booked there); and
//! running the line - fuel or power and maintenance per vehicle-kilometre (`economy`), the
//! drivers' hours and the buses. Every tour has its depot runs (`add_depot_runs`): empty
//! kilometres and driver time to the first stop and back, earning nothing; the demand model
//! counts the trips with passengers only.
//!
//! **Demand** (`potential`, `forecast`): a working day brings about 110 boardings a stop at a
//! bus every 15 minutes, less where the stops stand closer than about 400 m (their catchments
//! overlap) and a little more where they stand wider, less on a line shorter than 3 km
//! (people walk or cycle); every other line calling at a stop
//! adds a third of that (people change there; three lines at the most count). The service
//! draws passengers with an elasticity of 0.4 to the headway (TRL 593 "The demand for public
//! transport", 2004: 0.4 in the short run), the fare with -0.3. The day's passengers spread
//! over the hours as the German mobility survey (MiD 2017) has public transport's: a sharp
//! peak in the morning, a broad one in the afternoon on working days, the middle of the day
//! on Saturdays (60 % of a working day) and Sundays (40 %). In the rush hours two thirds ride
//! one way, and at its busiest point a trip carries about 60 % of its boardings at once: a bus
//! too small for that leaves people behind (passengers lost, and the reputation with them,
//! `carried`), a bus too big for the quiet hours burns fuel and money for nothing.
//!
//! **The kinds of service** (`crate::service::ServiceKind`) change who pays and who rides:
//! - *School transport* carries pupils - about 45 a stop on a school day, before eight and
//!   from noon on (`SCHOOL_HOURLY`), whatever the fare (they have passes) - and is paid by the
//!   school authority per trip: €45 and €3 a kilometre (German districts pay some €2.50 to €4
//!   a kilometre for their school runs). It runs on about 190 school days a year, and the
//!   authority is strict: a late trip costs three times the contract's penalty, a dropped one
//!   twice (`SCHOOL_LATE`, `SCHOOL_DROP`).
//! - *Weekend and leisure trips* carry people out for the day - 55 % of a regular line's riders
//!   at the stops, Saturdays and Sundays, from the late morning to the evening
//!   (`LEISURE_HOURLY`) - at one and a half times the fare (day tickets, visitors); a tourist
//!   board pays half the authority's money per kilometre; rain keeps people at home
//!   (`outing_weather`).
//! - *On demand* runs a trip only when somebody booked it: 30 % of a regular line's riders,
//!   who pay the fare and €1.50 for the booking. A trip is booked when at least one of its
//!   riders comes (`booking`); the drivers and buses stand by all the same.

use super::concessions;
use super::dates;
use super::economy;
use super::market;
use super::model::{BookingKind, BusKind, BusSize, Cents, Company, CompanyLine, Difficulty, Drive, Vehicle};
use super::network::{PlannedTrip, TourOfDay};
use super::rng::Rng;
use crate::lines::{self, LineDesign, PlannedTour, StopRef};
use crate::service::{BusFacts, LineVehicles, ServiceKind};
use crate::LineInfo;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// --- the costs ---------------------------------------------------------------------------------

/// What making and running an own line costs, for one difficulty (see the module's text).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineCosts {
    pub licence_base: Cents,
    pub licence_per_km: Cents,
    pub info_per_stop: Cents,
    pub live_display: Cents,
    pub timetable_per_group: Cents,
    pub association_entry: Cents,
    pub association_line: Cents,
    pub marketing_base: Cents,
    pub marketing_per_passenger: Cents,
    pub change_base: Cents,
    /// A stop, a month.
    pub stop_fee: Cents,
    /// The fare association's share of the fares.
    pub association_share: f64,
    /// Passengers against Realistic's.
    pub demand: f64,
}

const REALISTIC: LineCosts = LineCosts {
    licence_base: 2_000_00,
    licence_per_km: 150_00,
    info_per_stop: 180_00,
    live_display: 12_000_00,
    timetable_per_group: 1_200_00,
    association_entry: 6_000_00,
    association_line: 800_00,
    marketing_base: 1_000_00,
    marketing_per_passenger: 2_00,
    change_base: 600_00,
    stop_fee: 25_00,
    association_share: 0.06,
    demand: 1.0,
};

/// The costs of a difficulty: Realistic's, about half on Easy, 1.4 times on Hard.
pub fn costs(d: Difficulty) -> LineCosts {
    let scaled = |f: f64, share: f64, demand: f64| {
        let s = |c: Cents| ((c as f64 * f / 10_00 as f64).round() as Cents) * 10_00;
        LineCosts {
            licence_base: s(REALISTIC.licence_base),
            licence_per_km: s(REALISTIC.licence_per_km),
            info_per_stop: s(REALISTIC.info_per_stop),
            live_display: s(REALISTIC.live_display),
            timetable_per_group: s(REALISTIC.timetable_per_group),
            association_entry: s(REALISTIC.association_entry),
            association_line: s(REALISTIC.association_line),
            marketing_base: s(REALISTIC.marketing_base),
            marketing_per_passenger: (REALISTIC.marketing_per_passenger as f64 * f).round() as Cents,
            change_base: s(REALISTIC.change_base),
            stop_fee: s(REALISTIC.stop_fee),
            association_share: share,
            demand,
        }
    };
    match d {
        Difficulty::Easy => scaled(0.5, 0.03, 1.2),
        Difficulty::Realistic => REALISTIC,
        Difficulty::Hard => scaled(1.4, 0.09, 0.9),
    }
}

// --- the time of day ---------------------------------------------------------------------------

/// The day types (the line editor's `lines::DAY_GROUPS`, in that order).
pub const DAY_TYPES: [&str; 3] = ["Working days", "Saturdays", "Sundays and holidays"];

/// Days of each type in an average month: 250 working days, 52 Saturdays and 63 Sundays and
/// public holidays a year.
pub const MONTH_DAYS: [f64; 3] = [250.0 / 12.0, 52.0 / 12.0, 63.0 / 12.0];

/// A day type's passengers against a working day's.
pub const DAY_LEVEL: [f64; 3] = [1.0, 0.6, 0.4];

/// How a day's passengers spread over its hours (per cent, made to add up in `share`): public
/// transport in the German mobility survey MiD 2017.
const HOURLY: [[f64; 24]; 3] = [
    [0.5, 0.2, 0.1, 0.1, 0.3, 1.5, 5.0, 9.5, 7.0, 5.0, 4.5, 4.8, 5.5, 6.5, 6.5, 7.5, 8.0, 7.5, 5.5, 4.0, 2.8, 2.2, 1.6, 1.0],
    [0.8, 0.4, 0.2, 0.1, 0.2, 0.6, 1.5, 2.5, 4.0, 6.0, 8.0, 9.0, 9.0, 8.5, 8.0, 7.5, 7.0, 6.5, 5.5, 4.5, 3.5, 3.0, 2.2, 1.5],
    [0.8, 0.5, 0.3, 0.2, 0.2, 0.4, 1.0, 1.5, 2.5, 4.0, 6.5, 8.0, 8.0, 8.0, 8.5, 8.5, 8.5, 8.0, 7.0, 5.5, 4.5, 3.5, 2.5, 1.5],
];

/// The share of a day type's passengers in an hour.
pub fn share(day: usize, hour: usize) -> f64 {
    let d = &HOURLY[day.min(2)];
    d[hour % 24] / d.iter().sum::<f64>()
}

/// The parts of the day the estimate shows.
pub const BANDS: [&str; 5] = ["Morning peak", "Daytime", "Afternoon peak", "Evening", "Night"];
/// When each begins and ends (hours; the night from 21:00 to 6:00).
pub const BAND_HOURS: [(usize, usize); 5] = [(6, 9), (9, 15), (15, 18), (18, 21), (21, 6)];

pub fn band_of(hour: usize) -> usize {
    match hour % 24 {
        6..=8 => 0,
        9..=14 => 1,
        15..=17 => 2,
        18..=20 => 3,
        _ => 4,
    }
}

/// A rush hour (6-9, 15-18).
pub fn is_peak(hour: usize) -> bool {
    matches!(band_of(hour), 0 | 2)
}

// --- the kinds of service --------------------------------------------------------------------------

/// The school authority's contract per school trip (Realistic, founding day's prices): a part
/// for the bus and its driver kept for the school's times, and a part per kilometre.
pub const SCHOOL_TRIP_BASE: Cents = 45_00;
pub const SCHOOL_TRIP_PER_KM: Cents = 3_00;
/// School days a year (about 190 in the German states and the Netherlands).
pub const SCHOOL_DAYS: f64 = 190.0;
/// Pupils a stop brings on a school day (both ways together).
pub const PUPILS_PER_STOP: f64 = 45.0;
/// The school authority is strict: a late school trip costs this many times the contract's
/// penalty for a late trip, a dropped one this many times the penalty for a dropped one.
pub const SCHOOL_LATE: Cents = 3;
pub const SCHOOL_DROP: Cents = 2;
/// A weekend line's fare against the single ticket (day tickets, family tickets, visitors at
/// the full fare), the share of the authority's payment per kilometre a tourist board pays for
/// it, and its riders against a regular line's.
pub const LEISURE_FARE: f64 = 1.5;
pub const LEISURE_COMPENSATION: f64 = 0.5;
pub const LEISURE_DEMAND: f64 = 0.55;
/// `outing_weather` over a year: summers drier than winters.
pub const OUTING_WEATHER: f64 = 0.92;
/// An on-demand line: the booking fee on top of the fare, and its riders against a regular
/// line's (a thin area, the late hours).
pub const BOOKING_FEE: Cents = 1_50;
pub const ON_DEMAND_DEMAND: f64 = 0.3;

/// How the pupils spread over a school day: to school before eight, home from noon on.
const SCHOOL_HOURLY: [f64; 24] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 9.0, 34.0, 5.0, 1.0, 1.0, 1.0, 9.0, 16.0, 12.0, 8.0, 3.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
/// How people out for the day spread over it: out in the late morning, back in the afternoon.
const LEISURE_HOURLY: [f64; 24] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 1.5, 4.0, 8.0, 11.0, 11.0, 9.0, 9.0, 10.0, 10.0, 10.0, 8.0, 5.0, 3.0, 1.5, 1.0, 0.5, 0.0];

/// A day type's passengers against a working day's, for a line of `kind` (a school line has
/// none at the weekend, a weekend line none on working days).
pub fn level_of(kind: ServiceKind, day: usize) -> f64 {
    let day = day.min(2);
    match kind {
        ServiceKind::School => [1.0, 0.0, 0.0][day],
        ServiceKind::Leisure => [0.0, 0.85, 1.0][day],
        ServiceKind::OnDemand => [1.0, 0.6, 0.45][day],
        ServiceKind::Regular => DAY_LEVEL[day],
    }
}

/// The share of a day type's passengers in an hour, for a line of `kind`.
pub fn share_of(kind: ServiceKind, day: usize, hour: usize) -> f64 {
    let row = |r: &[f64; 24]| r[hour % 24] / r.iter().sum::<f64>();
    match kind {
        ServiceKind::School => row(&SCHOOL_HOURLY),
        ServiceKind::Leisure => row(&LEISURE_HOURLY),
        ServiceKind::Regular | ServiceKind::OnDemand => share(day, hour),
    }
}

/// Days of each type in an average month that a line of `kind` runs (a school line on the
/// school days only).
pub fn month_days(kind: ServiceKind, day: usize) -> f64 {
    match kind {
        ServiceKind::School if day == 0 => SCHOOL_DAYS / 12.0,
        ServiceKind::School => 0.0,
        _ => MONTH_DAYS[day.min(2)],
    }
}

/// What a school trip of `km` brings: the school authority's contract (indexed as the
/// authority's payments are; more on Easy, less on Hard).
pub fn school_trip_pay(c: &Company, km: f64) -> Cents {
    let d = match c.difficulty {
        Difficulty::Easy => 1.25,
        Difficulty::Realistic => 1.0,
        Difficulty::Hard => 0.85,
    };
    ((SCHOOL_TRIP_BASE as f64 + SCHOOL_TRIP_PER_KM as f64 * km.max(0.0)) * c.contract_index * d).round() as Cents
}

/// The weather of a day as people out for it feel it: a sunny day draws a quarter more, a wet
/// one 40 % fewer. Drawn from the company and the date (the same day has the same weather);
/// May to September are drier.
pub fn outing_weather(c: &Company, date: &str) -> f64 {
    let day = dates::parse(date).unwrap_or(0);
    let month = dates::civil_from_days(day).1;
    let wet = if (5..=9).contains(&month) { 0.25 } else { 0.45 };
    let x = Rng::of(&[&c.id, "weather"], day).f64();
    if x < wet {
        0.6
    } else if x < wet + 0.35 {
        1.0
    } else {
        1.25
    }
}

/// An on-demand trip whose riders average `boardings`: the chance somebody booked it (at
/// least one booking, the bookings coming at random), and its passengers when they did.
pub fn booking(boardings: f64) -> (f64, f64) {
    let b = boardings.max(0.0);
    let p = 1.0 - (-b).exp();
    (p, if p > 1e-9 { b / p } else { 1.0 })
}

/// What a trip of a line of `kind` brings: (fares, the payment for it). `per_passenger` is what
/// a passenger pays on the line, `per_km` the authority's payment a kilometre, `school` the
/// school authority's for the trip (`school_trip_pay`).
pub fn trip_income(kind: ServiceKind, pax: f64, km: f64, per_passenger: f64, per_km: f64, school: Cents) -> (Cents, Cents) {
    let (pax, km) = (pax.max(0.0), km.max(0.0));
    match kind {
        ServiceKind::Regular => ((pax * per_passenger).round() as Cents, (km * per_km).round() as Cents),
        ServiceKind::School => (0, school),
        ServiceKind::Leisure => ((pax * per_passenger * LEISURE_FARE).round() as Cents, (km * per_km * LEISURE_COMPENSATION).round() as Cents),
        ServiceKind::OnDemand => ((pax * (per_passenger + BOOKING_FEE as f64)).round() as Cents, (km * per_km).round() as Cents),
    }
}

/// The kind of service of a company line (a map's line, or an own line confirmed before there
/// were kinds: regular).
pub fn kind_of(cl: &CompanyLine) -> ServiceKind {
    cl.plan.as_ref().map(|p| p.service).unwrap_or_default()
}

/// The buses a company line asks for (None: any).
pub fn vehicles_of<'a>(c: &'a Company, line: &str) -> Option<&'a LineVehicles> {
    c.lines.iter().find(|x| x.name.eq_ignore_ascii_case(line.trim()))?.plan.as_ref().map(|p| &p.vehicles).filter(|v| !v.open())
}

/// The bus of the fleet may run the line's tours: the line asks for no buses in particular, or
/// it is one of them.
pub fn line_allows(c: &Company, line: &str, v: &Vehicle) -> bool {
    vehicles_of(c, line).is_none_or(|w| w.allows(&BusFacts::of_fleet(&v.bus, &v.name, v.kind.size)))
}

/// The fleet has a bus the line may run with (whether it is free or not).
pub fn fleet_has_bus_for(c: &Company, line: &str) -> bool {
    c.fleet.iter().any(|v| line_allows(c, line, v))
}

/// The size a tour wants made one of the sizes a line asks for (`allowed`, none: any): the
/// smallest of them that carries as much, else the biggest.
pub fn fit_size(size: BusSize, allowed: &[BusSize]) -> BusSize {
    if allowed.is_empty() || allowed.contains(&size) {
        return size;
    }
    let mut by_room: Vec<BusSize> = allowed.to_vec();
    by_room.sort_by(|a, b| capacity(*a).total_cmp(&capacity(*b)));
    by_room.iter().copied().find(|s| capacity(*s) >= capacity(size)).unwrap_or(*by_room.last().unwrap())
}

/// The day type of a date: 0 a working day, 1 Saturday, 2 Sunday.
pub fn day_type(date: &str) -> usize {
    match dates::parse(date).map(dates::weekday).unwrap_or(0) {
        5 => 1,
        6 => 2,
        _ => 0,
    }
}

// --- buses and their loads -----------------------------------------------------------------------

/// Places a bus offers as planned: its seats and standing room at four people a square metre
/// (VDV's planning value).
pub fn capacity(size: BusSize) -> f64 {
    match size {
        BusSize::Midi => 45.0,
        BusSize::Solo => 70.0,
        BusSize::Double => 95.0,
        BusSize::Articulated => 105.0,
    }
}

/// Crammed (six a square metre) a bus carries this much more; who does not fit waits for the
/// next one, or stays away.
pub const CRUSH: f64 = 1.3;
/// The share of a trip's boardings on board at once at its busiest point.
pub const ON_BOARD: f64 = 0.6;
/// In the rush hours two thirds ride one way: the busy direction carries 1.3 times the
/// average.
pub const PEAK_DIRECTION: f64 = 1.3;

/// The sizes by what they carry, smallest first.
pub const SIZES: [BusSize; 4] = [BusSize::Midi, BusSize::Solo, BusSize::Double, BusSize::Articulated];

/// The most people on board of a trip with `boardings` leaving in `hour`.
pub fn load(boardings: f64, hour: usize) -> f64 {
    boardings.max(0.0) * ON_BOARD * if is_peak(hour) { PEAK_DIRECTION } else { 1.0 }
}

/// The smallest bus a load fits into as planned (the double-decker is the player's own
/// choice: a midibus, a solo or an articulated bus).
pub fn size_for(load: f64) -> BusSize {
    [BusSize::Midi, BusSize::Solo, BusSize::Articulated].into_iter().find(|s| capacity(*s) >= load).unwrap_or(BusSize::Articulated)
}

/// The bigger of two sizes (by what they carry).
pub fn bigger(a: BusSize, b: BusSize) -> BusSize {
    if capacity(b) > capacity(a) {
        b
    } else {
        a
    }
}

/// What a trip with `boardings` wanting to ride in `hour` carries in a bus of `size`: the
/// passengers it takes, those left behind, and whether it was crowded beyond its planned
/// places.
pub fn carried(boardings: f64, hour: usize, size: BusSize) -> (f64, f64, bool) {
    let l = load(boardings, hour);
    let max = capacity(size) * CRUSH;
    if l <= max {
        return (boardings.max(0.0), 0.0, l > capacity(size));
    }
    let left = boardings.max(0.0) * (l - max) / l;
    (boardings.max(0.0) - left, left, true)
}

// --- the line's route --------------------------------------------------------------------------

/// The other lines of a map at each stop: by the stop's object and by its name (a stop and
/// the one across the road share their name).
#[derive(Clone, Debug, Default)]
pub struct StopLines {
    by_id: HashMap<i64, HashSet<String>>,
    by_name: HashMap<String, HashSet<String>>,
}

impl StopLines {
    /// The lines of a map's timetable (`lines`), `skip` left out (the line itself, by its
    /// timetable name). A line's every trip counts, not only today's.
    pub fn of(lines: &[LineInfo], skip: &str) -> StopLines {
        let mut s = StopLines::default();
        for l in lines.iter().filter(|l| !l.name.eq_ignore_ascii_case(skip.trim())) {
            for stop in l.tours.iter().flat_map(|t| t.trips.iter()).flat_map(|t| t.stops.iter()) {
                s.by_id.entry(stop.id).or_default().insert(l.name.clone());
                let name = stop.name.trim().to_lowercase();
                if !name.is_empty() {
                    s.by_name.entry(name).or_default().insert(l.name.clone());
                }
            }
        }
        s
    }

    /// How many other lines call at a stop (or the one of its name).
    pub fn count(&self, s: &StopRef) -> usize {
        let mut all: HashSet<&String> = HashSet::new();
        if let Some(x) = self.by_id.get(&s.id) {
            all.extend(x.iter());
        }
        if let Some(x) = self.by_name.get(&s.name.trim().to_lowercase()) {
            all.extend(x.iter());
        }
        all.len()
    }
}

/// What of a line's route counts for its passengers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shape {
    /// Its stops (a stop and the one of its name across the road once).
    pub stops: usize,
    /// Kilometres of each direction with stops, and of the longer.
    pub dir_km: Vec<f64>,
    pub route_km: f64,
    /// Kilometres from stop to stop on average.
    pub spacing_km: f64,
    /// For each of its stops the other lines calling there.
    pub transfers: Vec<usize>,
    /// The stops' objects (both directions).
    pub stop_ids: Vec<i64>,
}

impl Shape {
    /// Stops where another line calls.
    pub fn transfer_stops(&self) -> usize {
        self.transfers.iter().filter(|n| **n > 0).count()
    }

    /// The connections at its stops (three lines at a stop at the most count).
    pub fn connections(&self) -> usize {
        self.transfers.iter().map(|n| (*n).min(3)).sum()
    }
}

/// The shape of a line: `others` tells how many other lines call at a stop.
pub fn shape_of(l: &LineDesign, others: &dyn Fn(&StopRef) -> usize) -> Shape {
    let mut names: Vec<String> = Vec::new();
    let mut transfers = Vec::new();
    let mut stop_ids: Vec<i64> = Vec::new();
    let mut dir_km = Vec::new();
    let mut spacing = Vec::new();
    for d in l.directions.iter().filter(|d| d.stops.len() >= 2) {
        let km = d.legs.iter().take(d.stops.len() - 1).map(|g| g.length.max(0.0) as f64).sum::<f64>() / 1000.0;
        dir_km.push(km);
        spacing.push(km / (d.stops.len() - 1) as f64);
        for s in &d.stops {
            if !stop_ids.contains(&s.id) {
                stop_ids.push(s.id);
            }
            let key = if s.name.trim().is_empty() { format!("#{}", s.id) } else { s.name.trim().to_lowercase() };
            if !names.contains(&key) {
                names.push(key);
                transfers.push(others(s));
            }
        }
    }
    let route_km = dir_km.iter().copied().fold(0.0, f64::max);
    let spacing_km = if spacing.is_empty() { 0.0 } else { spacing.iter().sum::<f64>() / spacing.len() as f64 };
    Shape { stops: names.len(), dir_km, route_km, spacing_km, transfers, stop_ids }
}

// --- demand ------------------------------------------------------------------------------------

/// Boardings a stop brings on a working day at the reference service.
pub const BOARDINGS_PER_STOP: f64 = 110.0;
/// The reference service: a bus every 15 minutes.
pub const REF_HEADWAY: f64 = 15.0;
/// The stop spacing whose catchments just touch (km).
pub const GOOD_SPACING: f64 = 0.4;
/// What another line at a stop adds, of a stop's boardings.
pub const TRANSFER: f64 = 0.35;
/// The elasticities: of the service (to the headway), and of the fare (against €1.10).
pub const HEADWAY_ELASTICITY: f64 = 0.4;
pub const FARE_ELASTICITY: f64 = 0.3;
pub const REF_FARE: f64 = 110.0;

/// A line shorter than this (km) loses riders to walking and cycling.
pub const SHORT_LINE: f64 = 3.0;

/// Boardings a working day (a school day; a Sunday for a weekend line) brings at the reference
/// service for a line of `kind`: a school line its pupils (they ride on passes, whatever the
/// fare and however often the bus comes), the others the stops' riders (`potential`) at their
/// share and their fare.
pub fn potential_of(kind: ServiceKind, shape: &Shape, k: &LineCosts, fare: Cents, live_displays: bool) -> f64 {
    match kind {
        ServiceKind::Regular => potential(shape, k, fare, live_displays),
        ServiceKind::School => {
            if shape.stops < 2 {
                return 0.0;
            }
            let short = (shape.route_km / SHORT_LINE).clamp(0.3, 1.0);
            PUPILS_PER_STOP * shape.stops as f64 * short * k.demand
        }
        ServiceKind::Leisure => potential(shape, k, (fare as f64 * LEISURE_FARE).round() as Cents, live_displays) * LEISURE_DEMAND,
        ServiceKind::OnDemand => potential(shape, k, fare + BOOKING_FEE, live_displays) * ON_DEMAND_DEMAND,
    }
}

/// Boardings a working day brings at the reference service, both directions.
pub fn potential(shape: &Shape, k: &LineCosts, fare: Cents, live_displays: bool) -> f64 {
    if shape.stops < 2 {
        return 0.0;
    }
    let short = (shape.route_km / SHORT_LINE).clamp(0.3, 1.0);
    let spacing = (shape.spacing_km / GOOD_SPACING).clamp(0.5, 1.25);
    let stops = shape.stops as f64 * spacing + shape.connections() as f64 * TRANSFER;
    let fare = (REF_FARE / fare.max(1) as f64).powf(FARE_ELASTICITY);
    // (live departure information at the transfer stops: a couple of per cent more)
    let info = if live_displays && shape.transfer_stops() > 0 { 1.02 } else { 1.0 };
    BOARDINGS_PER_STOP * stops * short * k.demand * fare * info
}

/// What a service of `departures` an hour in each direction draws, against the reference.
pub fn service(departures: u32) -> f64 {
    if departures == 0 {
        return 0.0;
    }
    let headway = 60.0 / departures as f64;
    (REF_HEADWAY / headway).powf(HEADWAY_ELASTICITY).clamp(0.25, 1.35)
}

/// A line's passengers through the day: per day type and hour, the departures (all
/// directions), the boardings, and the boardings of a trip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Forecast {
    pub trips: [[u32; 24]; 3],
    pub riders: [[f64; 24]; 3],
    pub per_trip: [[f64; 24]; 3],
}

impl Forecast {
    pub fn day(&self, day: usize) -> f64 {
        self.riders[day].iter().sum()
    }

    /// Boardings in a part of the day (`BANDS`) of a day type.
    pub fn band(&self, day: usize, band: usize) -> f64 {
        (0..24).filter(|h| band_of(*h) == band).map(|h| self.riders[day][h]).sum()
    }
}

/// The forecast of a line with a working day's `potential` (see `potential_of`): its kind's
/// days and hours (a school line's pupils come whether the bus comes every ten minutes or
/// every thirty, as long as one comes).
pub fn forecast(l: &LineDesign, potential: f64) -> Forecast {
    let mut f = Forecast::default();
    let dirs = lines::run_minutes(l).len().clamp(1, 2) as u32;
    let kind = l.service;
    for day in 0..3 {
        // (the departures of the day patterns, or of the timetable's table)
        let all = lines::hourly_departures(l, day);
        for h in 0..24 {
            let per_dir = all[h].div_ceil(dirs);
            let drawn = if kind == ServiceKind::School { f64::from(u8::from(per_dir > 0)) } else { service(per_dir) };
            let riders = potential * level_of(kind, day) * share_of(kind, day, h) * drawn;
            f.trips[day][h] = all[h];
            f.riders[day][h] = riders;
            f.per_trip[day][h] = if f.trips[day][h] > 0 { riders / f.trips[day][h] as f64 } else { 0.0 };
        }
    }
    f
}

/// The size a tour of the line runs with: the biggest its trips want - their band's size, or
/// (a band on "auto", or no bands) the smallest bus their load fits into.
pub fn tour_size(l: &LineDesign, t: &PlannedTour, f: &Forecast) -> BusSize {
    let bands = l.days.get(t.day).map(|p| p.clean_bands()).unwrap_or_default();
    let mut size = BusSize::Midi;
    for trip in &t.trips {
        let hour = (trip.departure / 60.0).floor().max(0.0) as usize % 24;
        let wanted = trip.band.and_then(|b| bands.get(b)).and_then(|b| b.size).unwrap_or_else(|| size_for(load(f.per_trip[t.day.min(2)][hour], hour)));
        size = bigger(size, wanted);
    }
    size
}

// --- the estimate ------------------------------------------------------------------------------

/// A part of the day on a working day, as the estimate shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BandFigure {
    pub passengers: f64,
    /// The most on board of a trip, and the bus that fits it.
    pub load: f64,
    pub needed: Option<BusSize>,
    /// The bus most of its trips run with.
    pub size: Option<BusSize>,
    /// Trips fuller than their bus's places, and trips with a bus more than twice too big.
    pub crowded: usize,
    pub oversized: usize,
    pub trips: usize,
}

/// What a line costs and brings, as the line editor shows it while the player makes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Estimate {
    /// What is paid once (interface texts, amounts).
    pub one_off: Vec<(&'static str, Cents)>,
    /// A month: income positive, costs negative.
    pub monthly: Vec<(&'static str, Cents)>,
    /// Passengers a day, per day type.
    pub passengers: [f64; 3],
    /// A working day's parts.
    pub bands: [BandFigure; 5],
    /// Tours and vehicle-kilometres a day with passengers, and empty (the depot runs), per
    /// day type.
    pub tours: [usize; 3],
    pub km: [f64; 3],
    pub empty_km: [f64; 3],
    /// Buses out at once on a working day: at the most in the rush hours, in the middle of the
    /// day.
    pub buses_peak: usize,
    pub buses_offpeak: usize,
    /// Buses needed at the peak, per size.
    pub fleet: Vec<(BusSize, usize)>,
}

impl Estimate {
    pub fn one_off_total(&self) -> Cents {
        self.one_off.iter().map(|x| x.1).sum()
    }

    pub fn revenue(&self) -> Cents {
        self.monthly.iter().filter(|x| x.1 > 0).map(|x| x.1).sum()
    }

    pub fn costs(&self) -> Cents {
        -self.monthly.iter().filter(|x| x.1 < 0).map(|x| x.1).sum::<Cents>()
    }

    /// The month's result.
    pub fn result(&self) -> Cents {
        self.monthly.iter().map(|x| x.1).sum()
    }
}

/// Productive hours of a full-time driver a month (39 hours a week, less holidays, sickness
/// and training: about 1,680 hours a year at the wheel).
pub const DRIVER_HOURS: f64 = 140.0;
/// The depot run (Betriebsfahrt) before a tour's first trip and after its last: empty, no
/// line, no passengers - kilometres and driver time that earn nothing. Where the depot lies
/// on the map is not known: 3 km each way, 8 minutes.
pub const DEPOT_RUN_KM: f64 = 3.0;
pub const DEPOT_RUN_MIN: i32 = 8;
/// Minutes a tour takes besides its trips and depot runs: the checks before it leaves.
pub const TOUR_EXTRA: f64 = 10.0;

/// The own lines' tours of a day with their depot runs (their timetable has none): an empty
/// run from the depot to the first stop before the first trip, and back after the last. They
/// carry no passengers (`PlannedTrip::counts`), so the day costs their kilometres and the
/// driver's time and books nothing for them.
pub fn add_depot_runs(c: &Company, tours: &mut [TourOfDay]) {
    for t in tours.iter_mut() {
        let own = c.lines.iter().any(|l| l.plan.is_some() && l.name.eq_ignore_ascii_case(&t.line));
        let (Some(first), Some(last)) = (t.trips.first().cloned(), t.trips.last().cloned()) else { continue };
        if !own || !first.counts() {
            continue;
        }
        let run = |from: &str, to: &str, dep: i32| PlannedTrip { name: "Betriebsfahrt".into(), line: String::new(), from: from.into(), to: to.into(), dep, arr: dep + DEPOT_RUN_MIN, km: DEPOT_RUN_KM, stops: 0, empty: true };
        t.trips.insert(0, run("Depot", &first.from, first.dep - DEPOT_RUN_MIN));
        t.trips.push(run(&last.to, "Depot", last.arr));
    }
}

/// The estimate of `l` for company `c` (`shape` from `shape_of`).
pub fn estimate(c: &Company, l: &LineDesign, shape: &Shape) -> Estimate {
    let k = costs(c.difficulty);
    let r = economy::rules(c.difficulty);
    let pi = c.price_index;
    let kind = l.service;
    let mut e = Estimate::default();
    let f = forecast(l, potential_of(kind, shape, &k, r.fare, l.live_displays));
    let run = lines::run_minutes(l);
    let plan = lines::tour_plan(l);
    let allowed = l.vehicles.sizes();
    // the tours: their size, kilometres and hours
    struct T {
        day: usize,
        size: BusSize,
        from: f32,
        to: f32,
        km: f64,
    }
    let tours: Vec<T> = plan
        .iter()
        .map(|t| {
            let (from, to) = t.span(&run);
            let km = t.trips.iter().map(|x| shape.dir_km.get(x.dir).copied().unwrap_or(0.0)).sum();
            T { day: t.day.min(2), size: fit_size(tour_size(l, t, &f), &allowed), from, to, km }
        })
        .collect();
    for day in 0..3 {
        e.passengers[day] = f.day(day);
        e.tours[day] = tours.iter().filter(|t| t.day == day).count();
        e.km[day] = tours.iter().filter(|t| t.day == day).map(|t| t.km).sum();
        e.empty_km[day] = e.tours[day] as f64 * 2.0 * DEPOT_RUN_KM;
    }
    // a working day's parts: passengers, loads, the buses the trips run with
    for t in plan.iter().filter(|t| t.day == 0) {
        let size = fit_size(tour_size(l, t, &f), &allowed);
        for trip in &t.trips {
            let hour = (trip.departure / 60.0).floor().max(0.0) as usize % 24;
            let b = &mut e.bands[band_of(hour)];
            let ld = load(f.per_trip[0][hour], hour);
            b.trips += 1;
            if ld > capacity(size) {
                b.crowded += 1;
            }
            if capacity(size) > 2.0 * ld && size != size_for(ld) {
                b.oversized += 1;
            }
        }
    }
    for (bi, b) in e.bands.iter_mut().enumerate() {
        b.passengers = f.band(0, bi);
        b.load = (0..24).filter(|h| band_of(*h) == bi && f.trips[0][*h] > 0).map(|h| load(f.per_trip[0][h], h)).fold(0.0, f64::max);
        b.needed = (b.trips > 0).then(|| size_for(b.load));
        let mut count: Vec<(BusSize, usize)> = Vec::new();
        for t in plan.iter().filter(|t| t.day == 0) {
            let size = fit_size(tour_size(l, t, &f), &allowed);
            let n = t.trips.iter().filter(|x| band_of((x.departure / 60.0).floor().max(0.0) as usize % 24) == bi).count();
            if n > 0 {
                match count.iter_mut().find(|x| x.0 == size) {
                    Some(x) => x.1 += n,
                    None => count.push((size, n)),
                }
            }
        }
        b.size = count.iter().max_by_key(|x| x.1).map(|x| x.0);
    }
    // buses out at once on a working day (every five minutes), in all and per size
    let pull = DEPOT_RUN_MIN as f32;
    let out_at = |m: f32, size: Option<BusSize>| tours.iter().filter(|t| t.day == 0 && t.from - pull <= m && m < t.to + pull && size.is_none_or(|s| s == t.size)).count();
    let steps = |a: f32, b: f32| (0..((b - a) / 5.0) as usize).map(move |i| a + i as f32 * 5.0);
    e.buses_peak = steps(0.0, 1440.0).filter(|m| is_peak((*m / 60.0) as usize)).map(|m| out_at(m, None)).max().unwrap_or(0);
    e.buses_offpeak = steps(9.0 * 60.0 + 30.0, 14.0 * 60.0 + 30.0).map(|m| out_at(m, None)).max().unwrap_or(0);
    for s in SIZES {
        let n = steps(0.0, 1440.0).map(|m| out_at(m, Some(s))).max().unwrap_or(0);
        if n > 0 {
            e.fleet.push((s, n));
        }
    }

    // once
    let number_of_groups = (0..l.days.len().max(3)).filter(|d| lines::runs_on_group(l, *d)).count() as Cents;
    let has_own = c.lines.iter().any(|x| x.own);
    let scale = |a: Cents| (a as f64 * pi).round() as Cents;
    e.one_off.push(("Licence application", scale(k.licence_base + (k.licence_per_km as f64 * shape.route_km).round() as Cents)));
    e.one_off.push(("Stop timetables", scale(k.info_per_stop * shape.stops as Cents)));
    if l.live_displays && shape.transfer_stops() > 0 {
        e.one_off.push(("Live displays", scale(k.live_display * shape.transfer_stops() as Cents)));
    }
    e.one_off.push(("Timetable", scale(k.timetable_per_group * number_of_groups.max(1))));
    e.one_off.push(if has_own { ("Tariff integration", scale(k.association_line)) } else { ("Association entry", scale(k.association_entry)) });
    e.one_off.push(("Launch marketing", scale(k.marketing_base + (k.marketing_per_passenger as f64 * e.passengers[0]).round() as Cents)));

    // a month (a school line on the school days only; an on-demand line drives a trip only
    // when somebody booked it - its drivers and buses stand by all the same)
    let month = |a: [f64; 3]| (0..3).map(|d| a[d] * month_days(kind, d)).sum::<f64>();
    let pax = month(e.passengers);
    let mut booked = [1.0f64; 3];
    if kind == ServiceKind::OnDemand {
        for (d, b) in booked.iter_mut().enumerate() {
            let p: Vec<f64> = plan.iter().filter(|t| t.day.min(2) == d).flat_map(|t| t.trips.iter()).map(|x| booking(f.per_trip[d][(x.departure / 60.0).floor().max(0.0) as usize % 24]).0).collect();
            if !p.is_empty() {
                *b = p.iter().sum::<f64>() / p.len() as f64;
            }
        }
    }
    let km = (0..3).map(|d| e.km[d] * booked[d] * month_days(kind, d)).sum::<f64>();
    let per_km = economy::compensation_per_km(&r, c.reputation, c.contract_index);
    match kind {
        ServiceKind::School => {
            let a_day: f64 = plan.iter().filter(|t| t.day == 0).flat_map(|t| t.trips.iter()).map(|x| school_trip_pay(c, shape.dir_km.get(x.dir).copied().unwrap_or(0.0)) as f64).sum();
            e.monthly.push(("School contract", (a_day * month_days(kind, 0)).round() as Cents));
        }
        _ => {
            let (label, per) = match kind {
                ServiceKind::Leisure => ("Fares", r.fare as f64 * LEISURE_FARE * OUTING_WEATHER),
                ServiceKind::OnDemand => ("Fares and booking fees", (r.fare + BOOKING_FEE) as f64),
                _ => ("Fares", r.fare as f64),
            };
            let fares = (pax * per).round() as Cents;
            e.monthly.push((label, fares));
            e.monthly.push(("Association share", -(fares as f64 * k.association_share).round() as Cents));
            let (what, share) = if kind == ServiceKind::Leisure { ("Tourism grant per km", LEISURE_COMPENSATION) } else { ("Payment per km", 1.0) };
            e.monthly.push((what, (km * per_km * share).round() as Cents));
        }
    }
    let (mut energy, mut upkeep, mut hours) = (0.0, 0.0, 0.0);
    for t in &tours {
        let bus = BusKind { size: t.size, drive: Drive::Diesel };
        let days = month_days(kind, t.day);
        // (with the depot runs: empty kilometres, and the driver's time on them)
        let km = (t.km + 2.0 * DEPOT_RUN_KM) * booked[t.day];
        energy += km * economy::energy_per_km(bus, pi) * days;
        upkeep += km * economy::maintenance_per_km(bus, 3.0, pi) * days;
        hours += ((t.to - t.from) as f64 + 2.0 * DEPOT_RUN_MIN as f64 + TOUR_EXTRA) / 60.0 * days;
    }
    e.monthly.push(("Fuel", -energy.round() as Cents));
    e.monthly.push(("Maintenance", -upkeep.round() as Cents));
    let per_hour = economy::employer_cost(economy::market_wage(45.0, pi)) as f64 / DRIVER_HOURS;
    e.monthly.push(("Drivers", -(hours * per_hour).round() as Cents));
    let buses: f64 = e
        .fleet
        .iter()
        .map(|(s, n)| {
            let kind = BusKind { size: *s, drive: Drive::Diesel };
            let lease = economy::lease_monthly(economy::new_price(kind, &r, pi), &r);
            *n as f64 * (lease + economy::insurance_per_month(kind, pi)) as f64 + *n as f64 * 150_00 as f64 * pi
        })
        .sum();
    e.monthly.push(("Buses", -buses.round() as Cents));
    e.monthly.push(("Stop fees", -scale(k.stop_fee * shape.stops as Cents)));
    e.monthly.push(("Line licence", -scale(concessions::LICENCE)));
    e
}

// --- confirming, changing, running ---------------------------------------------------------------

/// What the company keeps of an own line it confirmed: the registry's line, its route as
/// approved, what it pays monthly per stop, its passengers through the day and the size of
/// bus each tour wants.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct OwnPlan {
    /// The line editor's id of it (`lines::LineDesign::id`).
    pub line_id: u64,
    /// The stops as approved (`route_key`), and every stop's object.
    pub route: u64,
    pub stop_ids: Vec<i64>,
    pub stops: u32,
    pub live_displays: bool,
    /// Boardings of a trip leaving in each hour, per day type.
    pub per_trip: [[f32; 24]; 3],
    /// The bus size each tour wants (by the tour's number).
    pub sizes: Vec<(String, BusSize)>,
    pub confirmed: String,
    /// What it cost to make and to change.
    pub paid: Cents,
    /// Its kind of service, and the buses it asks for (a file of an older version: regular,
    /// any bus).
    pub service: ServiceKind,
    pub vehicles: LineVehicles,
}

/// The route as a number: its stops, direction by direction (FNV-1a).
pub fn route_key(l: &LineDesign) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |v: i64| {
        for b in v.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for d in &l.directions {
        for s in &d.stops {
            eat(s.id);
            eat(((s.tile[0] as i64) << 32) ^ s.tile[1] as i64);
        }
        eat(-1);
    }
    h
}

/// The company line that is the line editor's line `id`.
pub fn line_of<'a>(c: &'a Company, id: u64) -> Option<&'a CompanyLine> {
    c.lines.iter().find(|x| x.own && x.plan.as_ref().is_some_and(|p| p.line_id == id))
}

/// "Ring · Markt – Bahnhof": the line's name and where it goes.
pub fn caption(l: &LineDesign) -> String {
    let mut to: Vec<String> = Vec::new();
    for d in l.directions.iter().filter(|d| d.stops.len() >= 2) {
        let x = d.destination();
        if !x.is_empty() && !to.contains(&x) {
            to.push(x);
        }
    }
    let to = to.join(" – ");
    [l.name.trim(), to.trim()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
}

fn plan_of(c: &Company, l: &LineDesign, shape: &Shape) -> OwnPlan {
    let k = costs(c.difficulty);
    let f = forecast(l, potential_of(l.service, shape, &k, economy::rules(c.difficulty).fare, l.live_displays));
    let allowed = l.vehicles.sizes();
    let mut per_trip = [[0.0f32; 24]; 3];
    for (d, row) in per_trip.iter_mut().enumerate() {
        for (h, v) in row.iter_mut().enumerate() {
            *v = f.per_trip[d][h] as f32;
        }
    }
    OwnPlan {
        line_id: l.id,
        route: route_key(l),
        stop_ids: shape.stop_ids.clone(),
        stops: shape.stops as u32,
        live_displays: l.live_displays,
        per_trip,
        sizes: lines::tour_plan(l).iter().map(|t| (t.number.clone(), fit_size(tour_size(l, t, &f), &allowed))).collect(),
        confirmed: c.date.clone(),
        paid: 0,
        service: l.service,
        vehicles: l.vehicles.clone(),
    }
}

/// Confirm and pay a line the player made for the company (`stem`: the timetable name it is
/// written as, `lines::stems`): its one-off costs booked, and the company runs it from today.
/// A line the company cannot pay for in cash is not confirmed.
/// The sizes of bus a line asks for: its kinds of bus and its time bands' buses.
pub fn sizes_asked(l: &LineDesign) -> Vec<BusSize> {
    let mut v: Vec<BusSize> = l.vehicles.classes.iter().map(|k| k.size()).collect();
    v.extend(l.days.iter().filter(|p| p.on).flat_map(|p| p.bands.iter().filter_map(|b| b.size)));
    v.dedup();
    v
}

/// The company's level allows every size of bus the line asks for (an articulated bus, a
/// double-decker: `market::kind_allowed`).
pub fn sizes_allowed(c: &Company, l: &LineDesign) -> Result<(), &'static str> {
    sizes_asked(l).into_iter().try_for_each(|s| market::size_allowed(c, s))
}

pub fn confirm(c: &mut Company, l: &LineDesign, stem: &str, shape: &Shape) -> Result<Cents, &'static str> {
    if !lines::problems(l).is_empty() {
        return Err("The line is not finished yet.");
    }
    sizes_allowed(c, l)?;
    if line_of(c, l.id).is_some() || c.lines.iter().any(|x| x.name.eq_ignore_ascii_case(stem)) {
        return Err("The company runs this line already.");
    }
    let e = estimate(c, l, shape);
    let total = e.one_off_total();
    if c.cash < total {
        return Err("The company cannot pay for the line: take a loan on the Finances page, or make it smaller.");
    }
    let number = l.number.trim().to_string();
    // (the line's public title in its bookings: the launch's advertising is for it)
    let named = if l.title.trim().is_empty() { format!("Line {number}") } else { format!("Line {number} \"{}\"", l.title.trim()) };
    for (what, amount) in &e.one_off {
        c.book(BookingKind::Concession, -amount, format!("{named}: {what}"), false);
    }
    let mut plan = plan_of(c, l, shape);
    plan.paid = total;
    c.lines.push(CompanyLine {
        name: stem.to_string(),
        number: number.clone(),
        numbers: vec![number],
        own: true,
        colour: l.colour.clone(),
        caption: caption(l),
        added: c.date.clone(),
        tours: e.tours[0] as u32,
        km: e.km[0],
        plan: Some(plan),
        // (a line runs only once it is planned: `network::start_service`)
        service_from: None,
        fare: None,
        demand: Default::default(),
        title: l.title.trim().to_string(),
        hof: String::new(),
        pending: None,
    });
    Ok(total)
}

/// What saving an edited company line costs: the route approved anew when its stops changed,
/// with the passenger information of the stops new to it and the live displays newly asked
/// for (None: nothing to pay).
pub fn change_fee(c: &Company, l: &LineDesign, shape: &Shape) -> Option<Cents> {
    // (against what is approved: a change waiting for its day is)
    let cl = line_of(c, l.id)?;
    let p = cl.pending.as_ref().map(|x| &x.plan).or(cl.plan.as_ref())?;
    let k = costs(c.difficulty);
    let mut fee = 0;
    if route_key(l) != p.route {
        let new = shape.stop_ids.iter().filter(|id| !p.stop_ids.contains(id)).count() as Cents;
        fee += k.change_base + k.info_per_stop * new;
    }
    if l.live_displays && !p.live_displays {
        fee += k.live_display * shape.transfer_stops() as Cents;
    }
    (fee > 0).then(|| (fee as f64 * c.price_index).round() as Cents)
}

/// A company line edited in the line editor and saved: the change paid (`change_fee`), and
/// what the company keeps of it made anew (its timetable name, number, colour, passengers and
/// tours). Returns what was paid.
pub fn apply_change(c: &mut Company, l: &LineDesign, stem: &str, shape: &Shape) -> Result<Cents, &'static str> {
    let (fee, change) = changed(c, l, stem, shape, "")?;
    let Some(cl) = c.lines.iter_mut().find(|x| x.own && x.plan.as_ref().is_some_and(|p| p.line_id == l.id)) else { return Err("The company does not run this line.") };
    cl.pending = None;
    let old = cl.name.clone();
    put(cl, change);
    // (a new number is a new timetable name: the roster follows it)
    super::plan::rename_line(c, &old, stem);
    Ok(fee)
}

/// A change of an own line in service, saved for a later day (Luc: "lijnen moeten ook
/// aanpasbaar zijn als ze al actief zijn"): what the company's line will be from `from` on,
/// and the tours whose buses and drivers are given anew then. Until that day the line runs
/// as it does (the timetable keeps it too: `lines::LineDesign::live`); the change is paid
/// when it is saved.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Pending {
    pub from: String,
    pub name: String,
    pub number: String,
    pub colour: String,
    pub caption: String,
    pub title: String,
    pub plan: OwnPlan,
    pub tours: u32,
    pub km: f64,
    pub replan: Vec<String>,
}

/// What a change makes of the company's line, paid (`change_fee`).
fn changed(c: &mut Company, l: &LineDesign, stem: &str, shape: &Shape, from: &str) -> Result<(Cents, Pending), &'static str> {
    let Some(cl) = line_of(c, l.id) else { return Err("The company does not run this line.") };
    let old = cl.pending.as_ref().map(|x| x.plan.clone()).or_else(|| cl.plan.clone()).unwrap_or_default();
    sizes_allowed(c, l)?;
    let fee = change_fee(c, l, shape).unwrap_or(0);
    if c.cash < fee {
        return Err("The company cannot pay for the change: take a loan on the Finances page.");
    }
    let number = l.number.trim().to_string();
    if fee > 0 {
        c.book(BookingKind::Concession, -fee, format!("Line {number}: route approved anew"), false);
    }
    let fresh = plan_of(c, l, shape);
    let e = estimate(c, l, shape);
    let plan = OwnPlan { confirmed: old.confirmed, paid: old.paid + fee, ..fresh };
    let title = l.title.trim().to_string();
    Ok((fee, Pending { from: from.to_string(), name: stem.to_string(), number, colour: l.colour.clone(), caption: caption(l), title, plan, tours: e.tours[0] as u32, km: e.km[0], replan: Vec::new() }))
}

/// A change into the company's line.
fn put(cl: &mut CompanyLine, x: Pending) {
    cl.plan = Some(x.plan);
    cl.name = x.name;
    if !cl.numbers.contains(&x.number) {
        cl.numbers.insert(0, x.number.clone());
    }
    cl.number = x.number;
    cl.colour = x.colour;
    cl.caption = x.caption;
    if !x.title.is_empty() {
        cl.title = x.title;
    }
    cl.tours = x.tours;
    cl.km = x.km;
}

/// Save a change of an own line for the day `from` (tomorrow at the soonest): paid now, the
/// company's line (and the roster's `replan` tours) as they are until then (`take_effect`).
/// A change waiting already gives way to it. Returns what was paid.
pub fn schedule_change(c: &mut Company, l: &LineDesign, stem: &str, shape: &Shape, from: &str, replan: Vec<String>) -> Result<Cents, &'static str> {
    if dates::between(&c.date, from) < 1 {
        return Err("A change of a line in service begins tomorrow at the soonest.");
    }
    let (fee, mut change) = changed(c, l, stem, shape, from)?;
    change.replan = replan;
    let Some(cl) = c.lines.iter_mut().find(|x| x.own && x.plan.as_ref().is_some_and(|p| p.line_id == l.id)) else { return Err("The company does not run this line.") };
    cl.pending = Some(change);
    Ok(fee)
}

/// The own lines (their line editor ids) whose change waiting is due on the company's day.
pub fn due(c: &Company) -> Vec<u64> {
    c.lines.iter().filter(|x| x.pending.as_ref().is_some_and(|p| dates::between(&p.from, &c.date) >= 0)).filter_map(|x| x.plan.as_ref().map(|p| p.line_id)).collect()
}

/// A change waiting takes effect: the company's line is the changed one, the roster follows
/// its new name, and the tours it changed or dropped lose their buses and drivers (to be
/// given anew). Returns the line's name, and those tours.
pub fn take_effect(c: &mut Company, line_id: u64) -> Option<(String, Vec<String>)> {
    let cl = c.lines.iter_mut().find(|x| x.own && x.plan.as_ref().is_some_and(|p| p.line_id == line_id))?;
    let x = cl.pending.take()?;
    let (old, replan) = (cl.name.clone(), x.replan.clone());
    put(cl, x);
    let name = cl.name.clone();
    super::plan::rename_line(c, &old, &name);
    super::plan::forget_tours(c, &name, &replan);
    Some((name, replan))
}

/// The bus size a tour of the company's asks for: an own line's tour the size its plan gives
/// it, else what its depot group names (`TourOfDay::wants`).
pub fn wanted(c: &Company, t: &TourOfDay) -> Option<BusSize> {
    c.lines
        .iter()
        .find(|x| x.name.eq_ignore_ascii_case(&t.line))
        .and_then(|x| x.plan.as_ref())
        .and_then(|p| p.sizes.iter().find(|s| s.0 == t.tour.trim()).map(|s| s.1))
        .or_else(|| t.wants())
}

/// The boardings of an own line's trip leaving at `minute` on `date` (None: no own line with
/// a plan - the economy's average then).
pub fn trip_boardings(cl: &CompanyLine, date: &str, minute: i32) -> Option<f64> {
    let p = cl.plan.as_ref()?;
    let hour = (minute.rem_euclid(1440) / 60) as usize;
    // (a weekend line runs on a working day only when it is a public holiday: Sunday's)
    let day = match (p.service, day_type(date)) {
        (ServiceKind::Leisure, 0) => 2,
        (_, d) => d,
    };
    Some(p.per_trip[day][hour] as f64)
}

/// The fare association's share of a line's fares (only the company's own lines are its).
pub fn association_share(c: &Company, cl: &CompanyLine) -> f64 {
    if cl.own && cl.plan.is_some() {
        costs(c.difficulty).association_share
    } else {
        0.0
    }
}

/// On a month's last day: the stop fees of the own lines.
pub fn month_end(c: &mut Company) {
    let k = costs(c.difficulty);
    let fees: Vec<(String, Cents)> = c.lines.iter().filter_map(|l| l.plan.as_ref().map(|p| (l.number.clone(), (k.stop_fee as f64 * p.stops as f64 * c.price_index).round() as Cents))).collect();
    for (n, fee) in fees {
        c.book(BookingKind::Concession, -fee, format!("Line {n}: stop fees"), false);
    }
}

/// A size of bus the fleet has too few of for the day's tours: the most tours asking for
/// that size or a bigger one out at once, and the buses of that size or bigger the company
/// has (a bigger bus can drive a tour asking for a smaller one).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shortfall {
    pub size: BusSize,
    pub needed: usize,
    pub have: usize,
}

/// The sizes the fleet lacks for the day's tours at their busiest.
pub fn shortfall(c: &Company, tours: &[TourOfDay]) -> Vec<Shortfall> {
    let mut out = Vec::new();
    // (the biggest first; a smaller size only when it lacks more than the bigger ones)
    let mut worst = 0;
    for size in SIZES.into_iter().rev() {
        let mine: Vec<(i32, i32)> = tours.iter().filter(|t| wanted(c, t).is_some_and(|w| capacity(w) >= capacity(size))).map(|t| (t.from(), t.to())).collect();
        let needed = mine.iter().map(|&(a, _)| mine.iter().filter(|&&(x, z)| x <= a && a < z).count()).max().unwrap_or(0);
        let have = c.fleet.iter().filter(|v| capacity(v.kind.size) >= capacity(size) && v.held_on(&c.date)).count();
        if needed > have && needed - have > worst {
            worst = needed - have;
            out.push(Shortfall { size, needed, have });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::{found, Founding};
    use super::*;
    use crate::lines::{DayPattern, Direction, Leg, TimeBand};

    fn stop(id: i64, name: &str) -> StopRef {
        StopRef { id, name: name.into(), ..Default::default() }
    }

    /// A line of `n` stops 400 m apart, out and back, a bus every 15 minutes all day.
    fn line(n: usize) -> LineDesign {
        let dir = |rev: bool| {
            let mut stops: Vec<StopRef> = (0..n).map(|i| stop(i as i64 + 1, &format!("Stop {i}"))).collect();
            if rev {
                stops.reverse();
            }
            let legs = (0..n - 1).map(|_| Leg { length: 400.0, ok: true, ..Default::default() }).collect();
            let mut d = Direction { stops, legs, ..Default::default() };
            d.refresh_times();
            d
        };
        let p = |days: u16| DayPattern { days, first: 5.0 * 60.0, last: 23.0 * 60.0, headway: 15.0, ..Default::default() };
        LineDesign { id: 9, number: "9".into(), name: "Ring".into(), ai_group: "Busse".into(), directions: vec![dir(false), dir(true)], days: vec![p(31), p(32), p(192)], ..Default::default() }
    }

    fn company(d: Difficulty) -> Company {
        found(&Founding { name: "Stadtbus".into(), difficulty: d, date: "2024-05-06".into(), ..Default::default() }, "Luc")
    }

    #[test]
    fn each_kind_of_service_has_its_riders_and_its_money() {
        let c = company(Difficulty::Realistic);
        let mut l = line(20);
        let s = shape_of(&l, &|_| 0);
        // school transport: pupils before eight and after noon, paid per trip by the school
        l.service = ServiceKind::School;
        l.days = lines::days_for(ServiceKind::School);
        let school = estimate(&c, &l, &s);
        assert!(school.monthly.iter().any(|x| x.0 == "School contract" && x.1 > 0));
        assert!(!school.monthly.iter().any(|x| x.0 == "Fares" || x.0 == "Payment per km"));
        assert_eq!((school.passengers[1], school.passengers[2], school.tours[1], school.tours[2]), (0.0, 0.0, 0, 0));
        let f = forecast(&l, potential_of(ServiceKind::School, &s, &costs(Difficulty::Realistic), 110, false));
        assert!(f.riders[0][7] > 100.0 && f.riders[0][10] == 0.0, "{} {}", f.riders[0][7], f.riders[0][10]);
        assert!((month_days(ServiceKind::School, 0) - 190.0 / 12.0).abs() < 1e-9 && month_days(ServiceKind::School, 2) == 0.0);
        // weekend trips: none on working days, a grant per km instead of the authority's money
        l.service = ServiceKind::Leisure;
        l.days = lines::days_for(ServiceKind::Leisure);
        let leisure = estimate(&c, &l, &s);
        assert_eq!(leisure.passengers[0], 0.0);
        assert!(leisure.passengers[2] > 0.0 && leisure.monthly.iter().any(|x| x.0 == "Tourism grant per km" && x.1 > 0));
        // on demand: the fare and the booking fee, fuel only for the trips booked
        l.service = ServiceKind::OnDemand;
        l.days = lines::days_for(ServiceKind::OnDemand);
        let od = estimate(&c, &l, &s);
        assert!(od.monthly.iter().any(|x| x.0 == "Fares and booking fees" && x.1 > 0));
        let all = LineDesign { service: ServiceKind::Regular, ..l.clone() };
        let fuel = |e: &Estimate| e.monthly.iter().find(|x| x.0 == "Fuel").unwrap().1;
        assert!(fuel(&od) > fuel(&estimate(&c, &all, &s)), "less fuel (a cost: negative)");
        // a trip's money by its kind
        assert_eq!(trip_income(ServiceKind::Regular, 10.0, 5.0, 110.0, 200.0, 0), (11_00, 10_00));
        assert_eq!(trip_income(ServiceKind::School, 40.0, 5.0, 110.0, 200.0, 60_00), (0, 60_00));
        assert_eq!(trip_income(ServiceKind::Leisure, 10.0, 5.0, 110.0, 200.0, 0), (16_50, 5_00));
        assert_eq!(trip_income(ServiceKind::OnDemand, 2.0, 5.0, 110.0, 200.0, 0), (5_20, 10_00));
        assert_eq!(school_trip_pay(&c, 10.0), 75_00);
        assert!(school_trip_pay(&company(Difficulty::Easy), 10.0) > school_trip_pay(&company(Difficulty::Hard), 10.0));
        // bookings: a trip nobody rides is never booked, one with two riders mostly
        assert_eq!(booking(0.0).0, 0.0);
        let (p, riders) = booking(2.0);
        assert!((p - 0.8647).abs() < 1e-3 && (riders * p - 2.0).abs() < 1e-9);
        // the weather of a day out: the same day the same
        let w = outing_weather(&c, "2024-07-06");
        assert!(w == outing_weather(&c, "2024-07-06") && [0.6, 1.0, 1.25].contains(&w));
        // the sizes a line's kinds of bus allow
        assert_eq!(fit_size(BusSize::Articulated, &[BusSize::Midi, BusSize::Solo]), BusSize::Solo);
        assert_eq!(fit_size(BusSize::Midi, &[BusSize::Solo]), BusSize::Solo);
        assert_eq!(fit_size(BusSize::Double, &[]), BusSize::Double);
        // confirmed, the company keeps its kind and its buses; a weekend line's holiday Monday
        // has Sunday's riders
        let mut c = company(Difficulty::Realistic);
        l.service = ServiceKind::Leisure;
        l.days = lines::days_for(ServiceKind::Leisure);
        l.vehicles.classes.push(crate::service::VehicleClass::Coach);
        confirm(&mut c, &l, "oo_9", &s).unwrap();
        let cl = line_of(&c, 9).unwrap();
        assert_eq!((kind_of(cl), cl.plan.as_ref().unwrap().vehicles.classes.len()), (ServiceKind::Leisure, 1));
        assert!(cl.plan.as_ref().unwrap().sizes.iter().all(|x| x.1 == BusSize::Solo));
        assert!(trip_boardings(cl, "2024-05-06", 12 * 60).unwrap() > 0.0);
    }

    #[test]
    fn the_costs_follow_the_difficulty() {
        let (e, r, h) = (costs(Difficulty::Easy), costs(Difficulty::Realistic), costs(Difficulty::Hard));
        assert_eq!(r.licence_base, 2_000_00);
        assert_eq!(e.licence_base, 1_000_00);
        assert_eq!(h.licence_base, 2_800_00);
        assert!(e.association_share < r.association_share && r.association_share < h.association_share);
        assert!(e.demand > r.demand && r.demand > h.demand);
        // a twenty-stop line of 7.6 km on Realistic: some €20,000-30,000 to start
        let c = company(Difficulty::Realistic);
        let l = line(20);
        let s = shape_of(&l, &|_| 0);
        assert_eq!((s.stops, (s.route_km * 10.0).round()), (20, 76.0));
        let est = estimate(&c, &l, &s);
        assert!((15_000_00..35_000_00).contains(&est.one_off_total()), "{}", est.one_off_total());
        assert!(est.one_off.iter().any(|x| x.0 == "Association entry"));
        // the same line costs less on Easy and more on Hard
        assert!(estimate(&company(Difficulty::Easy), &l, &s).one_off_total() < est.one_off_total());
        assert!(estimate(&company(Difficulty::Hard), &l, &s).one_off_total() > est.one_off_total());
    }

    #[test]
    fn passengers_follow_the_hour_the_stops_and_the_service() {
        let l = line(20);
        let s = shape_of(&l, &|_| 0);
        let k = costs(Difficulty::Realistic);
        let pot = potential(&s, &k, 110, false);
        assert!((pot - 2200.0).abs() < 1.0, "{pot}");
        let f = forecast(&l, pot);
        // the morning peak hour carries more than the middle of the day, the night least
        assert!(f.riders[0][7] > 1.5 * f.riders[0][11]);
        assert!(f.riders[0][7] > 10.0 * f.riders[0][5] / 2.0);
        assert_eq!(f.riders[0][2], 0.0, "no bus, no passenger");
        // Saturday and Sunday bring fewer
        assert!(f.day(0) > f.day(1) && f.day(1) > f.day(2));
        // other lines at the stops bring more; a denser service too, but less than in step
        let busy = shape_of(&l, &|s| if s.id <= 3 { 2 } else { 0 });
        assert_eq!((busy.transfer_stops(), busy.connections()), (3, 6));
        assert!(potential(&busy, &k, 110, false) > pot);
        assert!(service(6) > service(4) && service(6) < 1.5 * service(4));
        assert!((service(4) - 1.0).abs() < 1e-9);
        // a short line draws fewer: people walk
        let short = shape_of(&line(4), &|_| 0);
        assert!(potential(&short, &k, 110, false) < 4.0 * BOARDINGS_PER_STOP * 0.5);
        // stops too close together add less
        let mut tight = l.clone();
        for d in &mut tight.directions {
            for g in &mut d.legs {
                g.length = 150.0;
            }
        }
        assert!(potential(&shape_of(&tight, &|_| 0), &k, 110, false) < pot);
    }

    #[test]
    fn the_bus_fits_the_load_and_a_full_one_leaves_people_behind() {
        assert_eq!(size_for(30.0), BusSize::Midi);
        assert_eq!(size_for(60.0), BusSize::Solo);
        assert_eq!(size_for(100.0), BusSize::Articulated);
        assert_eq!(size_for(500.0), BusSize::Articulated);
        assert_eq!(bigger(BusSize::Double, BusSize::Articulated), BusSize::Articulated);
        // 100 boarding at 7: 78 on board at once - crowded in a solo bus (70), not left behind
        let (taken, left, crowded) = carried(100.0, 7, BusSize::Solo);
        assert!(crowded && left == 0.0 && taken == 100.0);
        // 200: 156 on board, a solo bus takes 91 - the rest waits
        let (taken, left, _) = carried(200.0, 7, BusSize::Solo);
        assert!(left > 0.0 && (taken + left - 200.0).abs() < 1e-9 && load(taken, 7) <= 91.0 + 1e-6);
        assert_eq!(carried(150.0, 7, BusSize::Articulated).1, 0.0);
    }

    #[test]
    fn rush_hours_get_more_buses_and_the_auto_size_follows_the_load() {
        let c = company(Difficulty::Realistic);
        let mut l = line(20);
        let flat = estimate(&c, &l, &shape_of(&l, &|_| 0));
        l.days[0].bands = lines::default_bands(0);
        let banded = estimate(&c, &l, &shape_of(&l, &|_| 0));
        // every 10 minutes in the rush hours: more buses then than in the middle of the day
        assert!(banded.buses_peak > banded.buses_offpeak, "{} {}", banded.buses_peak, banded.buses_offpeak);
        assert!(banded.passengers[0] > 0.0 && banded.bands[0].passengers > banded.bands[4].passengers);
        assert!(flat.buses_peak >= flat.buses_offpeak);
        // a small line wants small buses on "auto"; a band asking for articulated buses gets them
        assert_eq!(banded.bands[1].needed, Some(BusSize::Midi));
        l.days[0].bands[1] = TimeBand { size: Some(BusSize::Articulated), ..l.days[0].bands[1] };
        let big = estimate(&c, &l, &shape_of(&l, &|_| 0));
        assert!(big.fleet.iter().any(|x| x.0 == BusSize::Articulated));
        assert!(big.bands[1].oversized > 0);
        // the articulated buses cost more a month than the small ones
        assert!(big.result() < banded.result());
    }

    #[test]
    fn a_line_in_service_changes_on_its_day_and_the_roster_follows() {
        use super::super::plan::{self, Who};
        let mut c = company(Difficulty::Realistic);
        let l = line(12);
        let s = shape_of(&l, &|_| 0);
        confirm(&mut c, &l, "oo_9", &s).unwrap();
        // a roster for tours 1 and 2 on Monday
        plan::set_bus(&mut c, 0, "oo_9", "1", Some(7));
        plan::set_driver(&mut c, 0, "oo_9", "2", 0, Some(Who::Player));
        // a new number and a stop more, from the day after tomorrow
        let mut later = l.clone();
        later.number = "19".into();
        later.directions[0].stops.push(stop(99, "Neu"));
        later.directions[0].fit_legs();
        let s2 = shape_of(&later, &|_| 0);
        let fee = change_fee(&c, &later, &s2).unwrap();
        let today = c.date.clone();
        assert!(schedule_change(&mut c, &later, "oo_19", &s2, &today, vec!["2".into()]).is_err(), "not today");
        let from = dates::add(&c.date, 2);
        let cash = c.cash;
        assert_eq!(schedule_change(&mut c, &later, "oo_19", &s2, &from, vec!["2".into()]), Ok(fee));
        assert_eq!(c.cash, cash - fee, "paid when saved");
        // until then the line is as it was; its fee is not asked twice
        assert_eq!((line_of(&c, 9).unwrap().name.as_str(), line_of(&c, 9).unwrap().number.as_str()), ("oo_9", "9"));
        assert_eq!(change_fee(&c, &later, &s2), None);
        assert!(due(&c).is_empty());
        c.date = dates::add(&c.date, 1);
        assert!(due(&c).is_empty());
        // its day: the line is the new one, the roster follows its name, tour 2 is given anew
        c.date = from.clone();
        assert_eq!(due(&c), vec![9]);
        let (name, replan) = take_effect(&mut c, 9).unwrap();
        assert_eq!((name.as_str(), replan), ("oo_19", vec!["2".to_string()]));
        let cl = line_of(&c, 9).unwrap();
        assert!(cl.pending.is_none() && cl.number == "19" && cl.numbers.contains(&"9".to_string()));
        assert_eq!(plan::roster(&c, 0, "oo_19", "1").and_then(|r| r.bus), Some(7));
        assert!(plan::roster(&c, 0, "oo_19", "2").is_none());
        assert!(due(&c).is_empty() && take_effect(&mut c, 9).is_none());
    }

    #[test]
    fn a_line_asks_only_for_the_buses_the_level_opens() {
        use crate::service::VehicleClass;
        let mut c = company(Difficulty::Realistic);
        c.progress.xp = 0;
        let mut l = line(12);
        let s = shape_of(&l, &|_| 0);
        // an articulated bus for the rush hour: not at the first level
        l.days[0].bands = vec![lines::TimeBand { size: Some(BusSize::Articulated), ..Default::default() }];
        assert_eq!(sizes_allowed(&c, &l), Err("Articulated buses open at a higher company level."));
        assert_eq!(confirm(&mut c.clone(), &l, "oo_9", &s), Err("Articulated buses open at a higher company level."));
        // a double-decker among its kinds of bus neither
        l.days[0].bands.clear();
        l.vehicles.classes = vec![VehicleClass::Solo, VehicleClass::Double];
        assert_eq!(sizes_allowed(&c, &l), Err("Double-deckers open at a higher company level."));
        // at level 4 both are open
        c.progress.xp = super::super::levels::LEVEL_XP[3];
        assert_eq!(sizes_allowed(&c, &l), Ok(()));
    }

    #[test]
    fn a_line_is_confirmed_paid_and_changed() {
        let mut c = company(Difficulty::Realistic);
        let l = line(12);
        let s = shape_of(&l, &|_| 0);
        let cost = estimate(&c, &l, &s).one_off_total();
        // not without the money
        let mut poor = c.clone();
        poor.cash = cost - 1;
        assert!(confirm(&mut poor, &l, "oo_9", &s).is_err());
        assert!(poor.lines.is_empty() && poor.cash == cost - 1);
        // paid: booked, and the company runs it
        let cash = c.cash;
        assert_eq!(confirm(&mut c, &l, "oo_9", &s), Ok(cost));
        assert_eq!(c.cash, cash - cost);
        assert!(c.ledger.iter().any(|b| b.kind == BookingKind::Concession && b.text.contains("Licence")));
        let cl = line_of(&c, 9).unwrap().clone();
        assert!(cl.own && cl.name == "oo_9" && cl.caption.starts_with("Ring"));
        assert!(confirm(&mut c, &l, "oo_9", &s).is_err(), "only once");
        // a line without stops yet: no passengers, no buses, only the fixed costs
        let empty = LineDesign { id: 77, days: l.days.clone(), ..Default::default() };
        let e0 = estimate(&c, &empty, &shape_of(&empty, &|_| 0));
        assert_eq!((e0.passengers[0], e0.buses_peak, e0.tours[0]), (0.0, 0, 0));
        // its tours ask for a size; the second line pays the association less
        let p = cl.plan.as_ref().unwrap();
        assert_eq!(p.sizes.len(), lines::tour_plan(&l).len());
        assert!(estimate(&c, &line(12), &s).one_off.iter().any(|x| x.0 == "Tariff integration"));
        // the timetable changed only: free; a stop more: the change fee
        let mut later = l.clone();
        later.days[0].headway = 10.0;
        assert_eq!(change_fee(&c, &later, &shape_of(&later, &|_| 0)), None);
        later.directions[0].stops.push(stop(99, "Neu"));
        later.directions[0].fit_legs();
        let s2 = shape_of(&later, &|_| 0);
        let fee = change_fee(&c, &later, &s2).unwrap();
        assert_eq!(fee, 600_00 + 180_00);
        let cash = c.cash;
        assert_eq!(apply_change(&mut c, &later, "oo_9", &s2), Ok(fee));
        assert_eq!(c.cash, cash - fee);
        assert_eq!(change_fee(&c, &later, &s2), None, "approved now");
        // the stop fees at the month's end
        let cash = c.cash;
        month_end(&mut c);
        assert_eq!(cash - c.cash, 25_00 * 13);
        // a line with a public title: its launch is advertised under it, and the company keeps it
        let mut d = company(Difficulty::Realistic);
        let titled = LineDesign { id: 10, title: "Shuttleverkehr Altenfeld - Wurzbach".into(), ..line(12) };
        confirm(&mut d, &titled, "oo_10", &shape_of(&titled, &|_| 0)).unwrap();
        assert!(d.ledger.iter().any(|b| b.text.starts_with("Line 9 \"Shuttleverkehr Altenfeld - Wurzbach\": Launch marketing")));
        assert_eq!(line_of(&d, 10).unwrap().title, "Shuttleverkehr Altenfeld - Wurzbach");
    }

    #[test]
    fn the_planning_knows_the_sizes_and_what_the_fleet_lacks() {
        let mut c = company(Difficulty::Realistic);
        // (articulated buses: from the second level)
        c.progress.xp = super::super::levels::LEVEL_XP[1];
        let mut l = line(12);
        l.days[0].bands = vec![TimeBand { from: 360.0, to: 540.0, headway: 10.0, size: Some(BusSize::Articulated) }];
        let s = shape_of(&l, &|_| 0);
        confirm(&mut c, &l, "oo_9", &s).unwrap();
        let t = TourOfDay { line: "oo_9".into(), number: "9".into(), tour: "1".into(), ai_group: String::new(), trips: vec![super::super::network::PlannedTrip { dep: 360, arr: 400, stops: 12, ..Default::default() }], unplanned: false };
        assert_eq!(wanted(&c, &t), Some(BusSize::Articulated));
        let lack = shortfall(&c, &[t.clone(), TourOfDay { tour: "2".into(), ..t.clone() }]);
        assert_eq!(lack, vec![Shortfall { size: BusSize::Articulated, needed: 2, have: 0 }]);
        // a map line's tour: its depot group's
        assert_eq!(wanted(&c, &TourOfDay { line: "Linie 5".into(), ai_group: "Gelenkbus".into(), ..t.clone() }), Some(BusSize::Articulated));
        // its tours get their depot runs: empty, no passengers
        let mut day = vec![t.clone()];
        add_depot_runs(&c, &mut day);
        assert_eq!(day[0].trips.len(), 3);
        assert!(!day[0].trips[0].counts() && !day[0].trips[2].counts());
        assert_eq!((day[0].from(), day[0].to()), (360 - DEPOT_RUN_MIN, 400 + DEPOT_RUN_MIN));
        assert!((day[0].km() - 2.0 * DEPOT_RUN_KM).abs() < 1e-9);
        // its passengers by the hour of the day
        let cl = line_of(&c, 9).unwrap();
        assert!(trip_boardings(cl, "2024-05-06", 7 * 60).unwrap() > 0.0);
        assert_eq!(trip_boardings(cl, "2024-05-06", 3 * 60), Some(0.0));
        assert_eq!(day_type("2024-05-11"), 1);
        assert_eq!(day_type("2024-05-12"), 2);
    }

    #[test]
    fn the_map_lines_at_a_stop_count_once() {
        use crate::{StopInfo, TourInfo, TripInfo};
        let trip = |stops: &[(i64, &str)]| TripInfo { name: String::new(), index: 1, line: String::new(), from: String::new(), terminus: String::new(), departure: 0.0, arrival: 0.0, stops: stops.iter().map(|(id, n)| StopInfo { name: n.to_string(), id: *id, arr: 0.0, dep: 0.0 }).collect(), km: 1.0 };
        let li = |name: &str, stops: &[(i64, &str)]| LineInfo { name: name.into(), user_allowed: true, termini: Vec::new(), tours: vec![TourInfo { number: "1".into(), ai_group: String::new(), first: 0.0, last: 0.0, days: String::new(), runs: true, next_run: None, trips: vec![trip(stops)] }] };
        let lines = vec![li("A", &[(1, "Markt"), (2, "Bahnhof")]), li("B", &[(11, "Markt")]), li("oo_9", &[(1, "Markt")])];
        let s = StopLines::of(&lines, "oo_9");
        // by its object and across the road by its name
        assert_eq!(s.count(&stop(1, "Markt")), 2);
        assert_eq!(s.count(&stop(2, "Bahnhof")), 1);
        assert_eq!(s.count(&stop(5, "Kirche")), 0);
    }
}
