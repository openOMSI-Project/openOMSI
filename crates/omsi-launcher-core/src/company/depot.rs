//! The depot: the company's yard and its buildings, and the workshop's jobs.
//!
//! As the Busbetrieb-Simulator's depot, it is built up area by area: parking spaces (they set
//! how many buses the company can keep), workshop bays (how many buses can be worked on at
//! once), a washing bay (how clean the fleet is, which the passengers and the authority see),
//! a diesel station and charging points (what is not filled or charged at the depot is
//! bought dearer outside), and offices with rest rooms (the staff's satisfaction). Every level
//! of an area costs money and days to build and something every month to keep; what the
//! company was founded with is the rented yard `economy::depot_per_month` already pays for.
//!
//! The workshop takes jobs - a service, a repair, an overhaul - one bus a bay; a job waits
//! until a bay is free, and the bus is out of service only while it is worked on. The nightly
//! part (`after_day`) is called after the day's close: building work finished, jobs started,
//! the fleet's cleanliness, the fuel and power bought outside, the staff rooms.
//!
//! Figures (Realistic, net, mid-2020s Germany, rounded): a parking space on an asphalt yard
//! with lights and fencing about €7,500 (twelve a level); a workshop bay with a pit and lifting
//! gear in an existing hall €250,000; a manual wash bay €120,000, a wash gantry €350,000; a
//! diesel station with a tank and a pump €150,000, another pump €40,000; a 150 kW depot charger
//! with its share of the grid connection €60,000; a staff building €180,000-320,000. Diesel at a
//! public station costs operators about 12 % more than their own tank; public fast charging
//! about 45 % more than depot charging.

use super::dates;
use super::day::{DayReport, Note};
use super::economy;
use super::market;
use super::model::{BookingKind, BusSize, Cents, Company, Difficulty, Drive};
use serde::{Deserialize, Serialize};

/// The parts of the depot that are built.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Area {
    Parking,
    Workshop,
    Wash,
    Fuel,
    Charging,
    Offices,
}

impl Area {
    pub const ALL: [Area; 6] = [Area::Parking, Area::Workshop, Area::Wash, Area::Fuel, Area::Charging, Area::Offices];

    pub fn label(self) -> &'static str {
        match self {
            Area::Parking => "Parking spaces",
            Area::Workshop => "Workshop bays",
            Area::Wash => "Washing bay",
            Area::Fuel => "Diesel station",
            Area::Charging => "Charging points",
            Area::Offices => "Offices and rest rooms",
        }
    }

    /// Its name in the phone's JSON and in a saved order.
    pub fn key(self) -> &'static str {
        match self {
            Area::Parking => "parking",
            Area::Workshop => "workshop",
            Area::Wash => "wash",
            Area::Fuel => "fuel",
            Area::Charging => "charging",
            Area::Offices => "offices",
        }
    }

    pub fn from_key(s: &str) -> Option<Area> {
        Area::ALL.into_iter().find(|a| a.key() == s)
    }

    /// The launcher's icon for it.
    pub fn icon(self) -> &'static str {
        match self {
            Area::Parking => "local_parking",
            Area::Workshop => "construction",
            Area::Wash => "water_drop",
            Area::Fuel => "inventory_2",
            Area::Charging => "bolt",
            Area::Offices => "groups",
        }
    }

    /// The highest level it can be built to.
    pub fn max(self) -> u32 {
        match self {
            Area::Parking => 8,
            Area::Workshop => 6,
            Area::Wash => 2,
            Area::Fuel => 3,
            Area::Charging => 10,
            Area::Offices => 3,
        }
    }

    /// The level a company is founded with (the rented yard).
    pub fn start(self) -> u32 {
        match self {
            Area::Parking | Area::Workshop | Area::Fuel | Area::Offices => 1,
            Area::Wash | Area::Charging => 0,
        }
    }
}

/// Parking spaces a level of the yard has.
pub const SPACES_PER_LEVEL: u32 = 12;
/// Buses that can stand on rented spaces in the street when the yard is full, and what one
/// such space costs a day.
pub const OUTSIDE_MAX: usize = 6;
pub const OUTSIDE_PER_DAY: Cents = 30_00;
/// Buses a diesel pump fills a night, and a charging point charges.
pub const BUSES_PER_PUMP: usize = 15;
pub const BUSES_PER_CHARGER: usize = 2;
/// What fuel and power cost more when bought outside the depot.
pub const DIESEL_OUTSIDE: f64 = 0.12;
pub const POWER_OUTSIDE: f64 = 0.45;

/// One level of an area: what building it costs (Realistic, at founding), what it costs a
/// month to keep, and how many days the work takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub cost: Cents,
    pub upkeep: Cents,
    pub days: i64,
}

/// The step that brings an area to `level` (1 = its first).
pub fn step(area: Area, level: u32) -> Step {
    let (cost, upkeep, days) = match (area, level) {
        (Area::Parking, _) => (90_000_00, 400_00, 14),
        (Area::Workshop, _) => (250_000_00, 2_500_00, 30),
        (Area::Wash, 1) => (120_000_00, 900_00, 21),
        (Area::Wash, _) => (350_000_00, 1_800_00, 35),
        (Area::Fuel, 1) => (150_000_00, 600_00, 21),
        (Area::Fuel, _) => (40_000_00, 200_00, 7),
        (Area::Charging, _) => (60_000_00, 250_00, 10),
        (Area::Offices, 1) => (180_000_00, 700_00, 45),
        (Area::Offices, 2) => (260_000_00, 1_100_00, 45),
        (Area::Offices, _) => (320_000_00, 1_400_00, 50),
    };
    Step { cost, upkeep, days }
}

/// Building costs against the reference, by difficulty.
fn cost_factor(d: Difficulty) -> f64 {
    match d {
        Difficulty::Easy => 0.7,
        Difficulty::Realistic => 1.0,
        Difficulty::Hard => 1.15,
    }
}

/// Building work under way: the level it brings and its last day.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Works {
    pub area: Area,
    pub level: u32,
    pub until: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// The regular service (its work is paid with the maintenance per kilometre).
    Service,
    /// Body and running gear put right: the condition a service brings.
    Repair,
    /// A general overhaul: nearly as new.
    Overhaul,
    /// Painted in one of the company's liveries (`livery::paint`; not one of `ALL`, the jobs
    /// ordered as they are).
    Paint,
}

impl JobKind {
    pub const ALL: [JobKind; 3] = [JobKind::Service, JobKind::Repair, JobKind::Overhaul];

    pub fn label(self) -> &'static str {
        match self {
            JobKind::Service => "Service",
            JobKind::Repair => "Repair",
            JobKind::Overhaul => "Overhaul",
            JobKind::Paint => "Painting",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            JobKind::Service => "service",
            JobKind::Repair => "repair",
            JobKind::Overhaul => "overhaul",
            JobKind::Paint => "paint",
        }
    }

    pub fn from_key(s: &str) -> Option<JobKind> {
        JobKind::ALL.into_iter().find(|k| k.key() == s)
    }

    /// Workshop days it takes.
    pub fn days(self) -> i64 {
        match self {
            JobKind::Service | JobKind::Paint => 1,
            JobKind::Repair => 2,
            JobKind::Overhaul => 5,
        }
    }
}

/// A job of the workshop: waiting for a bay (`started` None) or in hand until `until`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Job {
    pub id: u32,
    pub vehicle: u32,
    pub kind: JobKind,
    pub ordered: String,
    #[serde(default)]
    pub started: Option<String>,
    #[serde(default)]
    pub until: Option<String>,
    /// What it costs (booked when it starts).
    pub cost: Cents,
    /// The company's livery a painting puts on the bus.
    #[serde(default)]
    pub livery: Option<String>,
}

/// The depot as it is built.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Site {
    pub parking: u32,
    pub workshop: u32,
    pub wash: u32,
    pub fuel: u32,
    pub charging: u32,
    pub offices: u32,
    #[serde(default)]
    pub works: Vec<Works>,
    #[serde(default)]
    pub jobs: Vec<Job>,
    /// How clean the fleet is, 0 to 100.
    #[serde(default = "clean_start")]
    pub clean: f64,
    #[serde(default)]
    pub job_counter: u32,
}

fn clean_start() -> f64 {
    80.0
}

impl Default for Site {
    fn default() -> Self {
        Site {
            parking: Area::Parking.start(),
            workshop: Area::Workshop.start(),
            wash: Area::Wash.start(),
            fuel: Area::Fuel.start(),
            charging: Area::Charging.start(),
            offices: Area::Offices.start(),
            works: Vec::new(),
            jobs: Vec::new(),
            clean: clean_start(),
            job_counter: 0,
        }
    }
}

impl Site {
    pub fn level(&self, a: Area) -> u32 {
        match a {
            Area::Parking => self.parking,
            Area::Workshop => self.workshop,
            Area::Wash => self.wash,
            Area::Fuel => self.fuel,
            Area::Charging => self.charging,
            Area::Offices => self.offices,
        }
    }

    pub fn set_level(&mut self, a: Area, v: u32) {
        let v = v.min(a.max());
        match a {
            Area::Parking => self.parking = v,
            Area::Workshop => self.workshop = v,
            Area::Wash => self.wash = v,
            Area::Fuel => self.fuel = v,
            Area::Charging => self.charging = v,
            Area::Offices => self.offices = v,
        }
    }

    pub fn spaces(&self) -> usize {
        (self.parking * SPACES_PER_LEVEL) as usize
    }

    pub fn bays(&self) -> usize {
        self.workshop as usize
    }

    /// Buses the washing bay washes a night: none, a manual bay's ten, a gantry's forty.
    pub fn washes(&self) -> usize {
        match self.wash {
            0 => 0,
            1 => 10,
            _ => 40,
        }
    }

    pub fn diesel_served(&self) -> usize {
        self.fuel as usize * BUSES_PER_PUMP
    }

    pub fn electric_served(&self) -> usize {
        self.charging as usize * BUSES_PER_CHARGER
    }

    /// The staff the offices and rest rooms have room for.
    pub fn staff_room(&self) -> usize {
        match self.offices {
            0 => 0,
            1 => 20,
            2 => 50,
            _ => 120,
        }
    }

    /// The building work on an area, if any.
    pub fn works_on(&self, a: Area) -> Option<&Works> {
        self.works.iter().find(|w| w.area == a)
    }

    /// The job of a bus, waiting or in hand.
    pub fn job_of(&self, vehicle: u32) -> Option<&Job> {
        self.jobs.iter().find(|j| j.vehicle == vehicle)
    }
}

/// Whether the company may build an area now: the workshop and the charging points open with
/// the company's levels (`levels::Feature`), the rest at once.
pub fn area_allowed(c: &Company, area: Area) -> bool {
    use super::levels::{unlocked, Feature};
    match area {
        Area::Workshop => unlocked(c, Feature::Workshop),
        Area::Charging => unlocked(c, Feature::ElectricBuses),
        _ => true,
    }
}

/// What building the next level of an area costs the company now (None: it is at its top).
pub fn next_cost(c: &Company, area: Area) -> Option<(Cents, i64)> {
    let next = c.site.level(area) + 1;
    if next > area.max() {
        return None;
    }
    let s = step(area, next);
    let cost = ((s.cost as f64 * cost_factor(c.difficulty) * c.price_index / 100.0).round() as Cents) * 100;
    let days = if c.difficulty == Difficulty::Easy { (s.days + 1) / 2 } else { s.days };
    Some((cost, days))
}

/// What the depot's buildings cost a month beyond the rented yard (the levels built since).
pub fn upkeep_month(c: &Company) -> Cents {
    let mut sum = 0;
    for a in Area::ALL {
        for lv in (a.start() + 1)..=c.site.level(a) {
            sum += step(a, lv).upkeep;
        }
    }
    (sum as f64 * c.price_index).round() as Cents
}

/// Build the next level of an area: paid now, in use once the work is done.
pub fn build(c: &mut Company, area: Area) -> Result<(), &'static str> {
    if !area_allowed(c, area) {
        return Err("Your company cannot build this yet.");
    }
    if c.site.works_on(area).is_some() {
        return Err("This is being built already.");
    }
    let Some((cost, days)) = next_cost(c, area) else { return Err("This is as big as it gets.") };
    if c.cash < cost {
        return Err("Not enough cash.");
    }
    let level = c.site.level(area) + 1;
    c.book(BookingKind::Construction, -cost, format!("{} {}", area.label(), level), false);
    let until = dates::add(&c.date, days - 1);
    c.site.works.push(Works { area, level, until });
    Ok(())
}

/// The buses the company holds today.
fn held(c: &Company) -> usize {
    c.fleet.iter().filter(|v| v.held_on(&c.date)).count()
}

/// Whether there is room for another bus: the yard's spaces and a few rented ones in the
/// street (the market asks before a bus is bought, leased or rented).
pub fn room(c: &Company) -> Result<(), &'static str> {
    // (the halls the company's levels open add places: `levels::extra_places`)
    if held(c) >= c.site.spaces() + super::levels::extra_places(c) as usize + OUTSIDE_MAX {
        return Err("The depot has no room for another bus: build more parking spaces.");
    }
    Ok(())
}

/// Buses that stand outside the yard tonight (over its spaces).
pub fn outside(c: &Company) -> usize {
    outside_on(c, &c.date)
}

/// Buses that stand outside the yard on `day`: those held and not in the workshop (a bus in
/// a bay needs no space) over the yard's spaces.
pub fn outside_on(c: &Company, day: &str) -> usize {
    let parked = c.fleet.iter().filter(|v| v.held_on(day) && !v.in_workshop(day)).count();
    parked.saturating_sub(c.site.spaces())
}

/// What a job on a bus costs now.
pub fn job_cost(c: &Company, vehicle: u32, kind: JobKind) -> Cents {
    let Some(v) = c.vehicle(vehicle) else { return 0 };
    let size = match v.kind.size {
        BusSize::Midi => 0.8,
        BusSize::Solo => 1.0,
        BusSize::Articulated | BusSize::Double => 1.35,
    };
    let age = dates::years_between(&v.built, &c.date);
    let euros = match kind {
        JobKind::Service => 0.0,
        JobKind::Repair => (market::serviced_condition(age) - v.condition).max(5.0) * 120.0 * size,
        JobKind::Overhaul => economy::reference_price(v.kind) as f64 / 100.0 * 0.06,
        JobKind::Paint => return super::livery::paint_cost(c, v),
    };
    ((euros * c.price_index).round() as Cents) * 100
}

/// Order a job for a bus: it starts tomorrow when a bay is free, else when one is.
pub fn order(c: &mut Company, vehicle: u32, kind: JobKind) -> Result<u32, &'static str> {
    let Some(v) = c.vehicle(vehicle) else { return Err("This bus is not in the fleet.") };
    if !v.held_on(&c.date) {
        return Err("This bus is not in the fleet.");
    }
    if c.site.job_of(vehicle).is_some() {
        return Err("The workshop has a job for this bus already.");
    }
    let cost = job_cost(c, vehicle, kind);
    if cost > 0 && c.cash < cost {
        return Err("Not enough cash.");
    }
    c.site.job_counter += 1;
    let id = c.site.job_counter;
    c.site.jobs.push(Job { id, vehicle, kind, ordered: c.date.clone(), started: None, until: None, cost, livery: None });
    let tomorrow = dates::add(&c.date, 1);
    start_jobs(c, &tomorrow);
    Ok(id)
}

/// Order the painting of a bus in a livery of the company's (`livery::paint` checks it): as
/// any job, from tomorrow when a bay is free, paid when it starts.
pub fn order_paint(c: &mut Company, vehicle: u32, livery: &str, cost: Cents) -> Result<u32, &'static str> {
    let Some(v) = c.vehicle(vehicle) else { return Err("This bus is not in the fleet.") };
    if !v.held_on(&c.date) {
        return Err("This bus is not in the fleet.");
    }
    if c.site.job_of(vehicle).is_some() {
        return Err("The workshop has a job for this bus already.");
    }
    if c.cash < cost {
        return Err("Not enough cash.");
    }
    c.site.job_counter += 1;
    let id = c.site.job_counter;
    c.site.jobs.push(Job { id, vehicle, kind: JobKind::Paint, ordered: c.date.clone(), started: None, until: None, cost, livery: Some(livery.to_string()) });
    let tomorrow = dates::add(&c.date, 1);
    start_jobs(c, &tomorrow);
    Ok(id)
}

/// Take back a job that has not started.
pub fn cancel(c: &mut Company, id: u32) -> Result<(), &'static str> {
    match c.site.jobs.iter().position(|j| j.id == id) {
        Some(i) if c.site.jobs[i].started.is_none() => {
            c.site.jobs.remove(i);
            Ok(())
        }
        Some(_) => Err("The work has started."),
        None => Err("There is no such job."),
    }
}

/// The bays in use on `day`: buses in the workshop (a breakdown, the service the night sends
/// a bus to, a job of ours).
pub fn bays_used(c: &Company, day: &str) -> usize {
    c.fleet.iter().filter(|v| v.held_on(day) && v.in_workshop(day)).count()
}

/// Start the waiting jobs that find a free bay on `day`, in the order they were ordered.
fn start_jobs(c: &mut Company, day: &str) {
    let mut free = c.site.bays().saturating_sub(bays_used(c, day));
    let waiting: Vec<u32> = c.site.jobs.iter().filter(|j| j.started.is_none()).map(|j| j.id).collect();
    for id in waiting {
        if free == 0 {
            break;
        }
        let Some(j) = c.site.jobs.iter().find(|j| j.id == id).cloned() else { continue };
        let Some(v) = c.fleet.iter_mut().find(|v| v.id == j.vehicle) else {
            c.site.jobs.retain(|x| x.id != id);
            continue;
        };
        if v.in_workshop(day) {
            // (in the workshop already, for a breakdown: the job waits for it to be out)
            continue;
        }
        let until = dates::add(day, j.kind.days() - 1);
        v.workshop_until = Some(until.clone());
        let age = dates::years_between(&v.built, day);
        match j.kind {
            JobKind::Service => {
                v.condition = v.condition.max(market::serviced_condition(age));
                v.next_service_km = ((v.km / market::SERVICE_KM).floor() + 1.0) * market::SERVICE_KM;
            }
            JobKind::Repair => v.condition = v.condition.max(market::serviced_condition(age)),
            JobKind::Overhaul => {
                v.condition = v.condition.max(96.0);
                v.next_service_km = ((v.km / market::SERVICE_KM).floor() + 1.0) * market::SERVICE_KM;
            }
            JobKind::Paint => {
                if let Some(name) = j.livery.clone().filter(|n| !n.trim().is_empty()) {
                    v.livery = name.clone();
                    v.house_livery = Some(name);
                }
            }
        }
        let text = format!("{} {} ({})", v.number, v.name, j.kind.label());
        if let Some(x) = c.site.jobs.iter_mut().find(|x| x.id == id) {
            x.started = Some(day.to_string());
            x.until = Some(until);
        }
        // (the dealer's free first service, and repairs under his warranty, cost nothing)
        let free_of_charge = match j.kind {
            JobKind::Service => super::dealer::take_free_service(c, j.vehicle),
            JobKind::Repair => super::dealer::under_warranty(c, j.vehicle, day),
            _ => false,
        };
        let kind = if j.kind == JobKind::Paint { BookingKind::Livery } else { BookingKind::Repair };
        c.book(kind, if free_of_charge { 0 } else { -j.cost }, text, false);
        free -= 1;
    }
}

/// What the night did at the depot, for the day's report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Night {
    /// Areas whose building work was finished (their labels).
    pub built: Vec<Area>,
    /// Buses that wait for a free bay (their numbers).
    pub waiting: Vec<String>,
    /// Bought outside the depot: fuel and power, street parking.
    pub energy_outside: Cents,
    pub parking_outside: Cents,
    pub upkeep: Cents,
}

/// The night at the depot after the day `date` was closed (the company's date is the next
/// day already; what is booked here is booked on `date`).
pub fn after_day(c: &mut Company, date: &str) -> Night {
    let mut night = Night::default();
    let tomorrow = c.date.clone();
    let keep = std::mem::replace(&mut c.date, date.to_string());

    // building work finished
    let done: Vec<Works> = c.site.works.iter().filter(|w| dates::between(&w.until, date) >= 0).cloned().collect();
    c.site.works.retain(|w| dates::between(&w.until, date) < 0);
    for w in done {
        c.site.set_level(w.area, w.level);
        night.built.push(w.area);
    }

    // fuel and power bought outside, for the share of the fleet the depot cannot serve
    let fleet: Vec<_> = c.fleet.iter().filter(|v| v.held_on(date)).cloned().collect();
    let energy_today: Cents = -c.ledger.iter().filter(|b| b.date == date && b.kind == BookingKind::Energy).map(|b| b.amount).sum::<Cents>();
    if energy_today > 0 && !fleet.is_empty() {
        let weight = |d: Drive| fleet.iter().filter(|v| v.kind.drive == d).map(|v| economy::energy_per_km(v.kind, 1.0)).sum::<f64>();
        let (wd, we) = (weight(Drive::Diesel), weight(Drive::Electric));
        let all = (wd + we).max(1e-9);
        let n = |d: Drive| fleet.iter().filter(|v| v.kind.drive == d).count();
        let (nd, ne) = (n(Drive::Diesel), n(Drive::Electric));
        let short = |n: usize, served: usize| if n == 0 { 0.0 } else { n.saturating_sub(served) as f64 / n as f64 };
        let extra = energy_today as f64 * (wd / all * short(nd, c.site.diesel_served()) * DIESEL_OUTSIDE + we / all * short(ne, c.site.electric_served()) * POWER_OUTSIDE);
        let extra = extra.round() as Cents;
        if extra > 0 {
            c.book(BookingKind::Energy, -extra, "Fuel and power bought outside the depot", false);
            night.energy_outside = extra;
        }
    }

    // street parking for the buses the yard has no space for
    let out = outside_on(c, &tomorrow);
    if out > 0 {
        let cost = (OUTSIDE_PER_DAY as f64 * out as f64 * c.price_index).round() as Cents;
        c.book(BookingKind::Depot, -cost, format!("Street parking ({out})"), false);
        night.parking_outside = cost;
    }

    // the fleet's cleanliness: every day takes some, the washing bay gives back
    if !fleet.is_empty() {
        let w = (c.site.washes() as f64 / fleet.len() as f64).min(1.0);
        let dirt = if out > 0 { 5.0 + 3.0 * out as f64 / fleet.len() as f64 } else { 5.0 };
        // (without a washing bay the cleaners' buckets keep it from going below this)
        let floor = 35.0;
        c.site.clean = ((c.site.clean - dirt).max(floor) * (1.0 - w) + 100.0 * w).clamp(0.0, 100.0);
        c.reputation = (c.reputation + (c.site.clean - 60.0) / 40.0 * 0.08).clamp(0.0, 100.0);
    }

    // the staff's rooms: room enough lifts them a little, too few is felt
    let staff_n = c.staff.iter().filter(|e| e.employed_on(date)).count();
    if staff_n > 0 {
        let room = c.site.staff_room();
        let lift = if staff_n <= room { 0.03 * c.site.offices as f64 } else { -0.05 };
        for e in c.staff.iter_mut().filter(|e| e.employed_on(date)) {
            e.satisfaction = (e.satisfaction + lift).clamp(0.0, 100.0);
        }
    }

    // the month's upkeep of what was built
    if dates::last_of_month(date) {
        let up = upkeep_month(c);
        if up > 0 {
            c.book(BookingKind::Depot, -up, "Upkeep of the depot's buildings", false);
            night.upkeep = up;
        }
    }

    // the workshop: jobs done, buses over the bays wait, waiting jobs start
    c.site.jobs.retain(|j| j.until.as_deref().is_none_or(|u| dates::between(u, date) < 0));
    c.site.jobs.retain(|j| c.fleet.iter().any(|v| v.id == j.vehicle));
    let ours: Vec<u32> = c.site.jobs.iter().filter(|j| j.started.is_some()).map(|j| j.vehicle).collect();
    let used = bays_used(c, &tomorrow);
    if used > c.site.bays() {
        let mut over = used - c.site.bays();
        let bays = c.site.bays();
        for v in c.fleet.iter_mut().filter(|v| v.held_on(&tomorrow) && v.in_workshop(&tomorrow) && !ours.contains(&v.id)).skip(bays) {
            if over == 0 {
                break;
            }
            if let Some(u) = v.workshop_until.clone() {
                v.workshop_until = Some(dates::add(&u, 1));
                night.waiting.push(v.number.clone());
                over -= 1;
            }
        }
    }
    start_jobs(c, &tomorrow);

    c.date = keep;
    night
}

/// After the day's close (`day::close_day`): the night at the depot and of the concessions
/// (`lines`: the timetable's lines of the day closed), what they booked counted into the
/// day's report and its record, and what they did told in its notes.
pub fn after_close(c: &mut Company, mut report: DayReport, lines: &[crate::LineInfo]) -> DayReport {
    let date = report.date.clone();
    let month = dates::month_of(&date);
    let before = c.month(&month);
    let night = after_day(c, &date);
    let km: Vec<(String, f64)> = report.lines.iter().map(|l| (l.line.clone(), l.km)).collect();
    let events = super::concessions::after_day(c, &date, &km, lines);
    let after = c.month(&month);
    for k in BookingKind::ALL.iter().filter(|k| !k.is_capital()) {
        let d = after.get(*k) - before.get(*k);
        if d > 0 {
            report.income += d;
        } else {
            report.expenses -= d;
        }
    }
    report.result = report.income - report.expenses;
    report.cash = c.cash;
    for a in night.built {
        report.notes.push(Note::Built { area: a.label().to_string() });
    }
    for number in night.waiting {
        report.notes.push(Note::BayWait { number });
    }
    for e in events {
        report.notes.push(match e {
            super::concessions::Event::Won { number, until } => Note::Won { number, until },
            super::concessions::Event::Lost { number, winner } => Note::Lost { number, winner },
            super::concessions::Event::Ended { number } => Note::Ended { number },
        });
    }
    if let Some(h) = c.history.last_mut().filter(|h| h.date == date) {
        h.cash = report.cash;
        h.income = report.income;
        h.expenses = report.expenses;
        h.result = report.result;
    }
    c.last_report = Some(report.clone());
    report
}

#[cfg(test)]
mod tests {
    use super::super::market::{self, MarketBus, Payment};
    use super::super::model::BusKind;
    use super::super::{found, Founding};
    use super::*;

    fn company(d: Difficulty, buses: usize) -> Company {
        let mut c = found(&Founding { name: "Depot".into(), difficulty: d, date: "2024-03-04".into(), ..Default::default() }, "Luc");
        c.cash += 10_000_000_00;
        // (level 2: the workshop may be built - the levels' gates are tested in `market`)
        c.progress.xp = super::super::levels::LEVEL_XP[1];
        let bus = MarketBus { file: "Vehicles/Citaro/Citaro.bus".into(), name: "Citaro".into(), ..Default::default() };
        for _ in 0..buses {
            market::buy_new(&mut c, &bus, Payment::Cash, "").unwrap();
        }
        c
    }

    /// Close `n` days of the depot only (the company's day moves on, as the day's close does).
    fn nights(c: &mut Company, n: usize) {
        for _ in 0..n {
            let date = c.date.clone();
            c.date = dates::add(&date, 1);
            after_day(c, &date);
        }
    }

    #[test]
    fn a_company_starts_with_the_rented_yard_and_its_upkeep_is_nothing() {
        let c = company(Difficulty::Realistic, 0);
        assert_eq!((c.site.spaces(), c.site.bays(), c.site.washes(), c.site.staff_room()), (12, 1, 0, 20));
        assert_eq!(upkeep_month(&c), 0);
        // (an old file without a site reads the same yard)
        let s: Site = serde_json::from_str("{\"parking\":1,\"workshop\":1,\"wash\":0,\"fuel\":1,\"charging\":0,\"offices\":1}").unwrap();
        assert_eq!(s, Site::default());
    }

    #[test]
    fn building_costs_by_difficulty_takes_its_days_and_then_costs_upkeep() {
        let easy = company(Difficulty::Easy, 0);
        let hard = company(Difficulty::Hard, 0);
        let mut c = company(Difficulty::Realistic, 0);
        assert_eq!(next_cost(&c, Area::Wash), Some((120_000_00, 21)));
        assert_eq!(next_cost(&easy, Area::Wash), Some((84_000_00, 11)));
        assert_eq!(next_cost(&hard, Area::Wash), Some((138_000_00, 21)));
        let cash = c.cash;
        build(&mut c, Area::Wash).unwrap();
        assert_eq!(c.cash, cash - 120_000_00);
        assert_eq!(build(&mut c, Area::Wash), Err("This is being built already."));
        assert!(c.ledger.last().is_some_and(|b| b.kind == BookingKind::Construction && b.kind.is_capital()));
        nights(&mut c, 20);
        assert_eq!(c.site.wash, 0, "not done before its 21st day");
        nights(&mut c, 1);
        assert_eq!(c.site.wash, 1);
        assert_eq!(upkeep_month(&c), 900_00);
        // the month's end books it
        while !dates::last_of_month(&c.date) {
            nights(&mut c, 1);
        }
        let month = dates::month_of(&c.date);
        nights(&mut c, 1);
        assert_eq!(c.month(&month).get(BookingKind::Depot), -900_00);
        // a top level is the top
        c.site.wash = 2;
        assert_eq!(build(&mut c, Area::Wash), Err("This is as big as it gets."));
        let mut poor = company(Difficulty::Hard, 0);
        poor.cash = 10;
        assert_eq!(build(&mut poor, Area::Workshop), Err("Not enough cash."));
    }

    #[test]
    fn the_yard_limits_the_fleet_and_buses_outside_cost_street_parking() {
        let mut c = company(Difficulty::Realistic, 12);
        assert_eq!(outside(&c), 0);
        let bus = MarketBus { file: "Vehicles/Citaro/Citaro.bus".into(), name: "Citaro".into(), ..Default::default() };
        for _ in 0..OUTSIDE_MAX {
            market::buy_new(&mut c, &bus, Payment::Cash, "").unwrap();
        }
        assert_eq!(outside(&c), OUTSIDE_MAX);
        assert!(room(&c).is_err());
        assert!(market::buy_new(&mut c, &bus, Payment::Cash, "").is_err());
        assert!(market::rent(&mut c, &bus, 2, "").is_err());
        let cash = c.cash;
        nights(&mut c, 1);
        assert_eq!(cash - c.cash, OUTSIDE_PER_DAY * OUTSIDE_MAX as Cents);
        // more spaces, more room
        c.site.parking = 2;
        assert!(room(&c).is_ok());
    }

    #[test]
    fn jobs_wait_for_a_free_bay_and_the_bus_is_out_only_while_worked_on() {
        let mut c = company(Difficulty::Realistic, 3);
        let ids: Vec<u32> = c.fleet.iter().map(|v| v.id).collect();
        for &id in &ids {
            c.fleet.iter_mut().find(|v| v.id == id).unwrap().condition = 50.0;
        }
        let tomorrow = dates::add(&c.date, 1);
        let a = order(&mut c, ids[0], JobKind::Repair).unwrap();
        let b = order(&mut c, ids[1], JobKind::Service).unwrap();
        assert_eq!(order(&mut c, ids[1], JobKind::Repair), Err("The workshop has a job for this bus already."));
        // one bay: the first starts tomorrow, the second waits and its bus still runs
        let ja = c.site.jobs.iter().find(|j| j.id == a).unwrap().clone();
        assert_eq!(ja.started.as_deref(), Some(tomorrow.as_str()));
        assert!(c.site.jobs.iter().find(|j| j.id == b).unwrap().started.is_none());
        assert!(c.vehicle(ids[0]).unwrap().in_workshop(&tomorrow));
        assert!(!c.vehicle(ids[1]).unwrap().in_workshop(&tomorrow));
        assert!(c.vehicle(ids[0]).unwrap().condition > 80.0);
        assert!(c.ledger.last().is_some_and(|x| x.kind == BookingKind::Repair && x.amount < 0));
        // the repair takes two days (tomorrow and the day after); then the service gets the bay
        nights(&mut c, 2);
        assert!(c.site.jobs.iter().find(|j| j.id == b).unwrap().started.is_none());
        assert!(c.vehicle(ids[0]).unwrap().in_workshop(&c.date));
        nights(&mut c, 1);
        assert!(c.site.jobs.iter().all(|j| j.id != a), "the repair is done");
        assert!(!c.vehicle(ids[0]).unwrap().in_workshop(&c.date));
        let jb = c.site.jobs.iter().find(|j| j.id == b).unwrap();
        assert_eq!(jb.started.as_deref(), Some(c.date.as_str()));
        // with two bays both start at once
        let mut two = company(Difficulty::Realistic, 2);
        two.site.workshop = 2;
        let v: Vec<u32> = two.fleet.iter().map(|v| v.id).collect();
        order(&mut two, v[0], JobKind::Service).unwrap();
        order(&mut two, v[1], JobKind::Overhaul).unwrap();
        assert!(two.site.jobs.iter().all(|j| j.started.is_some()));
        assert!(cancel(&mut two, 1).is_err());
    }

    #[test]
    fn the_washing_bay_keeps_the_fleet_clean_and_the_tank_saves_on_fuel() {
        let mut dirty = company(Difficulty::Realistic, 10);
        let mut clean = dirty.clone();
        clean.site.wash = 1;
        nights(&mut dirty, 30);
        nights(&mut clean, 30);
        assert!(dirty.site.clean <= 40.0 && clean.site.clean > 90.0, "{} {}", dirty.site.clean, clean.site.clean);
        assert!(clean.reputation > dirty.reputation);
        // fuel: twenty diesel buses, one pump for fifteen of them - a quarter of it outside
        let mut c = company(Difficulty::Realistic, 0);
        let bus = MarketBus { file: "Vehicles/Citaro/Citaro.bus".into(), name: "Citaro".into(), ..Default::default() };
        c.site.parking = 2;
        for _ in 0..20 {
            market::buy_new(&mut c, &bus, Payment::Cash, "").unwrap();
        }
        c.book(BookingKind::Energy, -1_000_00, "line", false);
        let date = c.date.clone();
        c.date = dates::add(&date, 1);
        let n = after_day(&mut c, &date);
        assert_eq!(n.energy_outside, 30_00);
        // and an electric bus without a charger charges outside, dearer still
        let mut e = company(Difficulty::Realistic, 0);
        // (electric buses open at level 3)
        e.progress.xp = super::super::levels::LEVEL_XP[2];
        let ebus = MarketBus { file: "Vehicles/E/E.bus".into(), name: "eCitaro".into(), kind: BusKind { size: BusSize::Solo, drive: Drive::Electric }, ..Default::default() };
        market::buy_new(&mut e, &ebus, Payment::Cash, "").unwrap();
        e.book(BookingKind::Energy, -1_000_00, "line", false);
        let date = e.date.clone();
        e.date = dates::add(&date, 1);
        assert_eq!(after_day(&mut e, &date).energy_outside, 450_00);
        let mut f = e.clone();
        f.site.charging = 1;
        f.book(BookingKind::Energy, -1_000_00, "line", false);
        let date = f.date.clone();
        f.date = dates::add(&date, 1);
        assert_eq!(after_day(&mut f, &date).energy_outside, 0);
    }
}
