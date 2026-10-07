//! The company day: who drives what, and "Close the day".
//!
//! Omsi-Hub's company day: OMSI runs one to one, and a whole company's day cannot be played,
//! so a day is a step the player takes. Its tours (those of the company's lines that run on
//! the date, `network::tours_of_day`) are given buses and drivers (`plan::day_plan`: the
//! weekly roster, the dispatcher's own, and what fell out in the morning), and the
//! close settles it: what the player drove himself counts measured, from the trip reports the
//! game writes (`crate::TripRun`); what the game reported of the company's buses while it ran
//! counts measured too (`record_live`, the hook for the live company of the next phase); the
//! rest is modelled - passengers from the line and the hour, fares and the authority's payment
//! per kilometre, energy, maintenance, breakdowns, late trips, and penalties for what was
//! dropped. Then the night: wear and services, the staff (`staff::after_day`), and on a
//! month's last day the wages, leases, insurance, depot and loan rates.

use super::dates;
use super::economy;
use super::finance;
use super::market;
use super::model::{BookingKind, BusKind, BusSize, Cents, Company, DayRecord, Drive, Tenure, HISTORY_KEPT};
use super::network::{self, line_of_number, TourOfDay};
use super::rng::Rng;
use super::staff::{self, Block, StaffNote, BUS_MARGIN, WEEK_DAYS};
use crate::service::ServiceKind;
use crate::TripRun;
use serde::{Deserialize, Serialize};

// --- the live hook ---------------------------------------------------------------------------

/// What the game reports of the company while it runs (the next phase: the company's buses
/// drive the map as AI and the player's own duties are known).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LiveEvent {
    /// A trip of a company tour, driven by one of its buses (`vehicle`) or by the player.
    Trip {
        /// The company line's number (as the displays show it) and the tour.
        line: String,
        tour: String,
        #[serde(default)]
        vehicle: Option<u32>,
        km: f64,
        passengers: u32,
        /// Seconds off the timetable at its end (negative early).
        delay: f64,
        completed: bool,
        /// When the trip left (minutes of the day, as the timetable has it): the close leaves
        /// that trip out of the model; without it the whole tour counts as reported.
        #[serde(default)]
        dep: Option<i32>,
    },
    /// A bus of the fleet broke down in the game.
    Breakdown { vehicle: u32 },
}

/// The hook the game calls with what happened (kept until the day is closed, which books it
/// as measured and leaves those tours out of the model).
pub fn record_live(c: &mut Company, ev: LiveEvent) {
    c.live.push(ev);
}

// --- the plan of a day -----------------------------------------------------------------------

/// A duty of a tour: its trips (indices) and who drives it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct DutyPlan {
    pub start: usize,
    pub end: usize,
    pub from: i32,
    pub to: i32,
    pub driver: Option<u32>,
    /// The driver comes late: the trips that leave before this minute are dropped.
    #[serde(default)]
    pub dropped_before: Option<i32>,
}

/// A tour of the day with its bus and duties.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct TourPlan {
    pub tour: TourOfDay,
    pub bus: Option<u32>,
    pub duties: Vec<DutyPlan>,
    /// The player drove it (a trip report says so), or the game reported it live.
    pub by_player: bool,
    pub live: bool,
}

impl TourPlan {
    pub fn covered(&self) -> bool {
        self.by_player || self.live || (self.bus.is_some() && self.duties.iter().all(|d| d.driver.is_some()))
    }

    /// Run in part (a bus, but not a driver for every duty).
    pub fn partly(&self) -> bool {
        !self.covered() && self.bus.is_some() && self.duties.iter().any(|d| d.driver.is_some())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Plan {
    pub tours: Vec<TourPlan>,
}

impl Plan {
    /// Tours of lines in service not covered (a line not planned yet runs nothing).
    pub fn uncovered(&self) -> usize {
        self.tours.iter().filter(|t| !t.tour.unplanned && !t.covered()).count()
    }

    /// A line's tours in service: (covered, all).
    pub fn coverage_of(&self, line: &str) -> (usize, usize) {
        let mine: Vec<&TourPlan> = self.tours.iter().filter(|t| !t.tour.unplanned && t.tour.line.eq_ignore_ascii_case(line)).collect();
        (mine.iter().filter(|t| t.covered()).count(), mine.len())
    }

    /// A line's tours as planned, whether it is in service or not: (covered, all).
    pub fn planned_of(&self, line: &str) -> (usize, usize) {
        let mine: Vec<&TourPlan> = self.tours.iter().filter(|t| t.tour.line.eq_ignore_ascii_case(line)).collect();
        (mine.iter().filter(|t| t.covered()).count(), mine.len())
    }

    /// Tours (in service) without a bus, and duties without a driver (of tours with one).
    pub fn short_of(&self) -> (usize, usize) {
        let open = || self.tours.iter().filter(|t| !t.tour.unplanned && !t.by_player && !t.live);
        let buses = open().filter(|t| t.bus.is_none()).count();
        let drivers = open().filter(|t| t.bus.is_some()).flat_map(|t| t.duties.iter()).filter(|d| d.driver.is_none()).count();
        (buses, drivers)
    }
}

/// Whether a tour is one of `keys` (a line's number or name, and the tour).
fn is_one_of(t: &TourOfDay, keys: &[(String, String)]) -> bool {
    keys.iter().any(|(l, n)| (t.number.eq_ignore_ascii_case(l) || t.line.eq_ignore_ascii_case(l) || t.trips.iter().any(|x| x.line.eq_ignore_ascii_case(l))) && t.tour.trim() == n.trim())
}

/// Give the day's tours buses and drivers: the buses that are there (held, not in the
/// workshop, fit to drive) one tour after the other with `BUS_MARGIN` between, the bus of the
/// size a tour's group names first; the drivers that are there (employed, not ill or on
/// holiday, under five days this week) each duty by the working-time rules, a driver who
/// works already today first while their day stays under eight hours, those who drive a big
/// bus without a warning first. Tours the player drove (`player`) or the game reported
/// (`live`) need neither.
pub fn assign(c: &Company, tours: Vec<TourOfDay>, player: &[(String, String)], live: &[(String, String)]) -> Plan {
    let date = c.date.clone();
    let mut buses: Vec<(u32, BusSize, i32)> = c.fleet.iter().filter(|v| v.held_on(&date) && !v.in_workshop(&date) && v.condition >= 20.0).map(|v| (v.id, v.kind.size, i32::MIN)).collect();
    let mut people: Vec<(u32, Vec<Block>)> = c.staff.iter().filter(|e| e.employed_on(&date) && !e.absent(&date) && e.week_days < WEEK_DAYS).map(|e| (e.id, Vec::new())).collect();
    let mut plans: Vec<TourPlan> = Vec::new();
    let mut tours = tours;
    tours.sort_by_key(|t| t.from());
    for t in tours {
        let duties: Vec<DutyPlan> = network::duties_of(&t)
            .into_iter()
            .map(|r| DutyPlan { from: t.trips[r.start].dep, to: t.trips[r.clone()].iter().map(|x| x.arr).max().unwrap_or(0), start: r.start, end: r.end, driver: None, dropped_before: None })
            .collect();
        let by_player = is_one_of(&t, player);
        let is_live = !by_player && is_one_of(&t, live);
        let mut plan = TourPlan { tour: t, bus: None, duties, by_player, live: is_live };
        if !by_player && !is_live {
            let from = plan.tour.from();
            let wants = plan.tour.wants();
            // (free in time and one of the line's buses; the size asked for first, then the one
            // free the latest: the others stay free for later tours)
            let line = plan.tour.line.clone();
            let best = buses
                .iter_mut()
                .filter(|b| (b.2 == i32::MIN || b.2 + BUS_MARGIN <= from) && c.vehicle(b.0).is_some_and(|v| super::ownline::line_allows(c, &line, v)))
                .max_by_key(|b| (wants.is_none_or(|w| w == b.1), b.2, std::cmp::Reverse(b.0)));
            if let Some(b) = best {
                b.2 = plan.tour.to();
                plan.bus = Some(b.0);
            }
        }
        plans.push(plan);
    }
    // the drivers, duty by duty in the order they begin
    let mut order: Vec<(usize, usize)> = plans.iter().enumerate().filter(|(_, p)| p.bus.is_some() && !p.by_player && !p.live).flat_map(|(i, p)| (0..p.duties.len()).map(move |k| (i, k))).collect();
    order.sort_by_key(|&(i, k)| plans[i].duties[k].from);
    for (i, k) in order {
        let p = &plans[i];
        let d = &p.duties[k];
        let vehicle = p.bus.and_then(|b| c.vehicle(b));
        let size = vehicle.map(|v| v.kind.size).unwrap_or_default();
        let block = Block {
            key: format!("{}/{}/{}", p.tour.line, p.tour.tour, k),
            from: d.from,
            to: d.to,
            from_stop: p.tour.trips[d.start].from.clone(),
            to_stop: p.tour.trips[d.end - 1].to.clone(),
        };
        let mut best: Option<(usize, (bool, bool, i32, u32, i64))> = None;
        for (pi, (id, blocks)) in people.iter().enumerate() {
            let Some(e) = c.employee(*id) else { continue };
            // (qualified for the bus: the licence, the endorsements, the model's type training)
            if !staff::may_drive(e, size) || vehicle.is_some_and(|v| super::licences::lack(e, v.kind, Some(&v.bus)).is_some()) {
                continue;
            }
            let k = staff::check(blocks, &block, e.last_end, None);
            if !k.allowed() {
                continue;
            }
            let worked: i32 = staff::work_minutes(blocks);
            let fills = worked > 0 && k.overtime == 0;
            let score = (staff::qualified(e, size), fills, -worked, WEEK_DAYS - e.week_days, (e.experience * 10.0) as i64);
            if best.as_ref().is_none_or(|b| score > b.1) {
                best = Some((pi, score));
            }
        }
        if let Some((pi, _)) = best {
            people[pi].1.push(block);
            plans[i].duties[k].driver = Some(people[pi].0);
        }
    }
    Plan { tours: plans }
}

// --- the close -------------------------------------------------------------------------------

/// A company line's day.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct LineDay {
    pub line: String,
    pub number: String,
    pub colour: String,
    pub tours: u32,
    pub covered: u32,
    pub trips: u32,
    pub dropped: u32,
    pub late: u32,
    pub km: f64,
    pub passengers: u32,
    pub revenue: Cents,
    /// What the day brought and cost the line: the fares (the association's share off), the
    /// authority's payment for its kilometres, energy and maintenance, the penalties.
    #[serde(default)]
    pub fares: Cents,
    #[serde(default)]
    pub compensation: Cents,
    #[serde(default)]
    pub running: Cents,
    #[serde(default)]
    pub penalty: Cents,
    /// Not in service yet (not planned): its tours did not run.
    #[serde(default)]
    pub unplanned: bool,
}

/// Something the day's report tells besides the figures.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Note {
    /// A bus broke down on its tour and is in the workshop until `until`.
    Breakdown { number: String, until: String, cost: Cents },
    /// A bus is due for its service: in the workshop tomorrow.
    Service { number: String },
    /// A leased or rented bus went back.
    Returned { number: String, name: String },
    LoanPaid { purpose: String },
    /// The month was closed: wages, leases, insurance, depot and loan rates were booked.
    Month { month: String, result: Cents },
    /// Building work at the depot was finished (the area's label).
    Built { area: String },
    /// A bus waits for a free workshop bay.
    BayWait { number: String },
    /// A concession was won (from tomorrow, or renewed) until `until`.
    Won { number: String, until: String },
    /// A tender was lost to another operator.
    Lost { number: String, winner: String },
    /// A concession ended: the line is no longer the company's.
    Ended { number: String },
    /// A concession line not in service yet `days` after it was taken on: the authority
    /// charged for the day.
    NotStarted { number: String, days: i64, charge: Cents },
    /// A driver of the company had an accident with the bus: the damage (`incidents`).
    Accident { number: String, cost: Cents },
}

/// What a closed day came to.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct DayReport {
    pub date: String,
    pub tours: u32,
    pub covered: u32,
    pub by_player: u32,
    /// Trips with passengers: planned, dropped, late.
    pub trips: u32,
    pub dropped: u32,
    pub late: u32,
    pub km: f64,
    pub passengers: u32,
    pub income: Cents,
    pub expenses: Cents,
    pub result: Cents,
    pub cash: Cents,
    pub penalties: Cents,
    /// The player's own trips booked (measured), and what they earned.
    pub measured: u32,
    pub measured_revenue: Cents,
    pub punctuality: Option<f64>,
    pub reputation: f64,
    pub reputation_change: f64,
    pub lines: Vec<LineDay>,
    pub notes: Vec<Note>,
    pub staff: Vec<StaffNote>,
    /// Trips of the own lines fuller than their bus's places, and the passengers they left
    /// behind (`ownline::carried`).
    #[serde(default)]
    pub crowded: u32,
    #[serde(default)]
    pub left_behind: u32,
    /// The tours in service not covered, and what they lacked: tours without a bus, duties
    /// without a driver.
    #[serde(default)]
    pub uncovered: u32,
    #[serde(default)]
    pub short_buses: u32,
    #[serde(default)]
    pub short_drivers: u32,
    /// Accidents, fines, complaints and passengers taken ill on the modelled trips.
    #[serde(default)]
    pub incidents: super::incidents::Tally,
}

/// Maps named alike (`maps/Grundorf/global.cfg`, with either slash and any case).
fn same_map(a: &str, b: &str) -> bool {
    let n = |s: &str| s.trim().replace('\\', "/").to_lowercase();
    n(a) == n(b)
}

/// Wear a kilometre takes off a bus's condition.
fn wear_per_km(r: &economy::Rules) -> f64 {
    0.0012 * (0.7 + 0.3 * r.breakdown_factor)
}

/// The sums of a closing day, per line and in all.
#[derive(Default)]
struct Sums {
    fares: Cents,
    compensation: Cents,
    energy: f64,
    maintenance: f64,
    penalty: Cents,
}

/// Close the company's day: `tours` are its tours of the day (`network::tours_of_day` of the
/// timetable read for `c.date`), `trips` the player's trip reports (all of them; those of
/// the company's map and lines not booked yet count). Moves the company to the next day.
pub fn close_day(c: &mut Company, tours: Vec<TourOfDay>, trips: &[TripRun]) -> DayReport {
    // (only what is planned runs: a line not in service books nothing)
    let tours = network::in_service_only(tours);
    // (the passengers answer the fares: `fares`)
    super::fares::day(c);
    let r = economy::rules(c.difficulty);
    let date = c.date.clone();
    let month = dates::month_of(&date);
    let before = c.month(&month);
    let mut rng = Rng::of(&[&c.id, "day"], dates::parse(&date).unwrap_or(0));
    let mut report = DayReport { date: date.clone(), ..Default::default() };
    let day_end = super::clock::moment(&date, 24 * 60 - 1);
    let mut lines: Vec<LineDay> = c.lines.iter().map(|l| LineDay { line: l.name.clone(), number: l.number.clone(), colour: l.colour.clone(), unplanned: !network::in_service(l, day_end), ..Default::default() }).collect();
    let per_passenger: Vec<f64> = c.lines.iter().map(|l| super::fares::per_passenger(c, l)).collect();
    // (what kind of service each line is: a school line is paid per trip by the school
    // authority, a weekend line's riders answer the weather, an on-demand trip runs only when
    // it was booked - `ownline`)
    let kinds: Vec<ServiceKind> = c.lines.iter().map(super::ownline::kind_of).collect();
    let weather = super::ownline::outing_weather(c, &date);
    let mut bookings = Rng::of(&[&c.id, "bookings"], dates::parse(&date).unwrap_or(0));
    let mut sums: Vec<Sums> = lines.iter().map(|_| Sums::default()).collect();
    let line_index = |c: &Company, number: &str| line_of_number(c, number).and_then(|l| c.lines.iter().position(|x| x.name == l.name));
    let comp_km = economy::compensation_per_km(&r, c.reputation, c.contract_index);
    let mut on_time = 0u32;
    let mut timed = 0u32;

    // 1. what the player drove himself (measured)
    let mut player: Vec<(String, String)> = Vec::new();
    let mut seen = c.trips_seen;
    let mine: Vec<&TripRun> = trips.iter().filter(|t| t.time > c.trips_seen && !t.free && same_map(&t.map, &c.map) && line_of_number(c, &t.line).is_some()).collect();
    for t in mine {
        seen = seen.max(t.time);
        let Some(li) = line_index(c, &t.line) else { continue };
        let km = t.metres / 1000.0;
        let km = if km.is_finite() && km > 0.0 && km <= (t.seconds / 3600.0 * 100.0).max(10.0) { km } else { 0.0 };
        let (fares, comp) = super::ownline::trip_income(kinds[li], t.passengers.max(0) as f64, km, per_passenger[li], comp_km, super::ownline::school_trip_pay(c, km));
        let vehicle = c.fleet.iter_mut().find(|v| v.bus.eq_ignore_ascii_case(&t.bus));
        let (kind, age) = match vehicle {
            Some(v) => {
                v.km += km;
                v.condition = (v.condition - km * wear_per_km(&r)).max(0.0);
                (v.kind, dates::years_between(&v.built, &date))
            }
            None => (BusKind { size: BusSize::Solo, drive: Drive::Diesel }, 5.0),
        };
        let s = &mut sums[li];
        s.energy += km * economy::energy_per_km(kind, c.price_index);
        s.maintenance += km * economy::maintenance_per_km(kind, age, c.price_index);
        let late = t.average.is_some_and(|a| a > 180.0) || (t.stops > 0 && t.late * 4 > t.stops);
        if t.timed() {
            timed += 1;
            if !late {
                on_time += 1;
            }
        }
        let l = &mut lines[li];
        l.km += km;
        l.passengers += t.passengers.max(0) as u32;
        l.revenue += fares + comp;
        (l.fares, l.compensation) = (l.fares + fares, l.compensation + comp);
        report.measured += 1;
        report.measured_revenue += fares + comp;
        c.book(BookingKind::Fares, fares, format!("Line {} (own trip)", l.number), true);
        c.book(BookingKind::Compensation, comp, format!("Line {} (own trip)", l.number), true);
        // (its fines, its quality bonus and its experience: `levels`)
        super::levels::book_trip(c, t, &l.number);
        if !t.tour.trim().is_empty() {
            player.push((t.line.clone(), t.tour.clone()));
        }
    }
    c.trips_seen = seen;

    // 2. what the game reported live (measured)
    let mut live: Vec<(String, String)> = Vec::new();
    let mut live_trips: Vec<(String, String, Option<i32>)> = Vec::new();
    let mut live_broken: Vec<u32> = Vec::new();
    for ev in std::mem::take(&mut c.live) {
        match ev {
            LiveEvent::Trip { line, tour, vehicle, km, passengers, delay, completed: _, dep } => {
                live_trips.push((line.clone(), tour.clone(), dep));
                let Some(li) = line_index(c, &line) else { continue };
                let (fares, comp) = super::ownline::trip_income(kinds[li], passengers as f64, km, per_passenger[li], comp_km, super::ownline::school_trip_pay(c, km));
                if let Some(v) = vehicle.and_then(|id| c.fleet.iter_mut().find(|v| v.id == id)) {
                    v.km += km.max(0.0);
                    v.condition = (v.condition - km.max(0.0) * wear_per_km(&r)).max(0.0);
                    let age = dates::years_between(&v.built, &date);
                    sums[li].energy += km.max(0.0) * economy::energy_per_km(v.kind, c.price_index);
                    sums[li].maintenance += km.max(0.0) * economy::maintenance_per_km(v.kind, age, c.price_index);
                }
                timed += 1;
                if delay <= 180.0 {
                    on_time += 1;
                }
                let l = &mut lines[li];
                l.km += km.max(0.0);
                l.passengers += passengers;
                l.revenue += fares + comp;
                (l.fares, l.compensation) = (l.fares + fares, l.compensation + comp);
                c.book(BookingKind::Fares, fares, format!("Line {} (live)", l.number), true);
                c.book(BookingKind::Compensation, comp, format!("Line {} (live)", l.number), true);
                if !live.iter().any(|x| x.0 == line && x.1 == tour) {
                    live.push((line, tour));
                }
            }
            LiveEvent::Breakdown { vehicle } => live_broken.push(vehicle),
        }
    }

    // 3. the plan (the roster, the dispatcher's own, what fell out in the morning: see
    // `plan`), and what breaks down on the way (the trips after it are dropped)
    let day_plan = super::plan::day_plan(c, &date, tours, &player, &live, true);
    let (plan, morning) = super::plan::settle_morning(c, &day_plan);
    report.notes.extend(morning);
    let (short_buses, short_drivers) = plan.short_of();
    (report.uncovered, report.short_buses, report.short_drivers) = (plan.uncovered() as u32, short_buses as u32, short_drivers as u32);
    // (as the company's clock went through the day: `clock::breakdowns`; a rental bus ordered
    // runs the trips from when it came)
    let cut = super::clock::breakdowns(c, &plan, &date, &live_broken);

    // 4. the modelled trips (what else befalls them, by the drivers' courses: `incidents`)
    let trained = super::incidents::trained(c);
    let mut befalls = Rng::of(&[&c.id, "incidents"], dates::parse(&date).unwrap_or(0));
    let mut tally = super::incidents::Tally::default();
    let mut bus_km: Vec<(u32, f64)> = Vec::new();
    let mut worked: Vec<(u32, i32, i32)> = Vec::new();
    for tp in &plan.tours {
        let Some(li) = lines.iter().position(|l| l.line.eq_ignore_ascii_case(&tp.tour.line)) else { continue };
        lines[li].tours += 1;
        report.tours += 1;
        if tp.covered() {
            lines[li].covered += 1;
            report.covered += 1;
        }
        if tp.by_player {
            report.by_player += 1;
            continue;
        }
        // (a tour the game ran live: the trips it reported are booked already, the rest of
        // the day is the model's)
        let reported = |trip: &network::PlannedTrip| tp.live && live_trips.iter().any(|(l, n, dep)| is_one_of(&tp.tour, &[(l.clone(), n.clone())]) && dep.is_none_or(|d| (d - trip.dep).abs() <= 2));
        let v = tp.bus.and_then(|b| c.vehicle(b)).cloned();
        let broken_at = tp.bus.and_then(|b| cut.iter().find(|x| x.0 == b)).map(|x| (x.1, x.2));
        for d in &tp.duties {
            let driver = d.driver.and_then(|id| if id == super::plan::AGENCY { Some(super::plan::agency_driver()) } else { c.employee(id).cloned() });
            if let Some(e) = driver.as_ref().filter(|e| e.id != super::plan::AGENCY) {
                match worked.iter_mut().find(|w| w.0 == e.id) {
                    Some(w) => {
                        w.1 += d.to - d.from;
                        w.2 = w.2.max(d.to);
                    }
                    None => worked.push((e.id, d.to - d.from, d.to)),
                }
            }
            for trip in &tp.tour.trips[d.start..d.end] {
                let counts = trip.counts();
                let kind = kinds.get(li).copied().unwrap_or_default();
                // (an on-demand trip nobody booked stays in the depot: no kilometres, no money,
                // no penalty; one that was booked carries those who booked it)
                let mut booked = None;
                if kind == ServiceKind::OnDemand && counts {
                    let base = c.lines.get(li).and_then(|cl| super::ownline::trip_boardings(cl, &date, trip.dep)).unwrap_or(0.0);
                    let (p, riders) = super::ownline::booking(base);
                    if !bookings.chance(p) {
                        continue;
                    }
                    booked = Some(riders);
                }
                if counts {
                    lines[li].trips += 1;
                    report.trips += 1;
                }
                if reported(trip) {
                    continue;
                }
                let run = v.is_some() && driver.is_some() && broken_at.is_none_or(|(at, back)| trip.dep < at || back.is_some_and(|b| trip.dep >= b)) && d.dropped_before.is_none_or(|t| trip.dep >= t);
                if !run {
                    if counts {
                        lines[li].dropped += 1;
                        report.dropped += 1;
                        let strict = if kind == ServiceKind::School { super::ownline::SCHOOL_DROP } else { 1 };
                        sums[li].penalty += (r.drop_per_trip + (trip.km * r.drop_per_km as f64).round() as Cents) * strict;
                    }
                    continue;
                }
                let (Some(v), Some(e)) = (&v, &driver) else { continue };
                let age = dates::years_between(&v.built, &date);
                lines[li].km += trip.km;
                match bus_km.iter_mut().find(|b| b.0 == v.id) {
                    Some(b) => b.1 += trip.km,
                    None => bus_km.push((v.id, trip.km)),
                }
                sums[li].energy += trip.km * economy::energy_per_km(v.kind, c.price_index);
                sums[li].maintenance += trip.km * economy::maintenance_per_km(v.kind, age, c.price_index);
                if !counts {
                    continue;
                }
                // (an own line: its passengers by the hour, and what its bus can take)
                let own = c.lines.get(li).filter(|cl| cl.plan.is_some());
                let base = booked.or_else(|| own.and_then(|cl| super::ownline::trip_boardings(cl, &date, trip.dep))).unwrap_or_else(|| economy::passengers_for(trip.km, trip.dep, &r));
                let mut pax = base * rng.range(0.8, 1.2) * (0.85 + 0.3 * c.reputation / 100.0) * (0.95 + 0.1 * e.skills.service / 100.0) * c.lines.get(li).map(|l| l.demand.share()).unwrap_or(1.0);
                match kind {
                    ServiceKind::Leisure => pax *= weather,
                    ServiceKind::OnDemand => pax = pax.max(1.0),
                    _ => {}
                }
                if own.is_some() {
                    let (taken, left, crowded) = super::ownline::carried(pax, (trip.dep.rem_euclid(1440) / 60) as usize, v.kind.size);
                    pax = taken;
                    report.crowded += crowded as u32;
                    report.left_behind += left.round() as u32;
                }
                let pax = pax.round().max(0.0) as u32;
                let (fares, comp) = super::ownline::trip_income(kind, pax as f64, trip.km, per_passenger[li], comp_km, super::ownline::school_trip_pay(c, trip.km));
                let skilled = trained.get(&e.id).copied().unwrap_or_default();
                let fares = (fares as f64 * super::incidents::fare_factor(&skilled)).round() as Cents;
                super::incidents::trip(&mut befalls, e, &skilled, &v.number, v.kind.size, pax, c.price_index, &mut tally);
                sums[li].fares += fares;
                sums[li].compensation += comp;
                lines[li].passengers += pax;
                lines[li].revenue += fares + comp;
                let p_late = 0.04 + 0.12 * (1.0 - e.skills.punctuality / 100.0) + 0.10 * (1.0 - v.condition / 100.0);
                timed += 1;
                if rng.chance(p_late) {
                    lines[li].late += 1;
                    report.late += 1;
                    sums[li].penalty += r.late_per_trip * if kind == ServiceKind::School { super::ownline::SCHOOL_LATE } else { 1 };
                } else {
                    on_time += 1;
                }
            }
        }
    }
    for (li, s) in sums.iter().enumerate() {
        let text = format!("Line {}", lines[li].number);
        c.book(BookingKind::Fares, s.fares, text.clone(), false);
        // (the fare association keeps its share of an own line's fares)
        let share = c.lines.get(li).map(|cl| super::ownline::association_share(c, cl)).unwrap_or(0.0);
        let kept = (s.fares as f64 * share).round() as Cents;
        if kept > 0 {
            c.book(BookingKind::Fares, -kept, format!("Line {}: fare association's share", lines[li].number), false);
            lines[li].revenue -= kept;
        }
        c.book(BookingKind::Compensation, s.compensation, text.clone(), false);
        c.book(BookingKind::Energy, -s.energy.round() as Cents, text.clone(), false);
        c.book(BookingKind::Maintenance, -s.maintenance.round() as Cents, text.clone(), false);
        c.book(BookingKind::Penalty, -s.penalty, text, false);
        report.penalties += s.penalty;
        let l = &mut lines[li];
        l.fares += s.fares - kept;
        l.compensation += s.compensation;
        l.running += (s.energy + s.maintenance).round() as Cents;
        l.penalty += s.penalty;
    }
    for (number, cost) in &tally.accidents {
        c.book(BookingKind::Repair, -cost, format!("Accident damage, bus {number}"), false);
        report.notes.push(Note::Accident { number: number.clone(), cost: *cost });
    }
    if tally.fine_cost > 0 {
        c.book(BookingKind::Fine, -tally.fine_cost, format!("Traffic fines of the drivers ({})", tally.fines), false);
    }
    // a concession not in service after its grace: the authority charges for the day
    for (li, cl) in c.lines.clone().iter().enumerate() {
        if let Some((days, charge)) = network::late_start(c, cl, &date) {
            c.book(BookingKind::Penalty, -charge, format!("Line {}: service not started", cl.number), false);
            report.penalties += charge;
            lines[li].penalty += charge;
            report.notes.push(Note::NotStarted { number: cl.number.clone(), days, charge });
        }
    }

    // 5. rentals by the day
    let rented: Vec<(Cents, String)> = c.fleet.iter().filter(|v| v.held_on(&date)).filter_map(|v| match &v.tenure {
        Tenure::Rented { daily, .. } => Some((*daily, format!("{} {}", v.number, v.name))),
        _ => None,
    }).collect();
    for (daily, text) in rented {
        c.book(BookingKind::Rent, -daily, text, false);
    }

    // 6. the buses tonight: kilometres, wear, breakdowns, services
    let tomorrow = dates::add(&date, 1);
    let mut repairs: Vec<(Cents, String)> = Vec::new();
    let price_index = c.price_index;
    // (a bus under the dealer's warranty is repaired at the dealer's cost)
    let warranted = super::dealer::warranted(c, &date);
    // (the company's own mechanics have a broken-down bus back a day sooner)
    let mechanics = super::training::mechanics(c, &date);
    for v in c.fleet.iter_mut() {
        if let Some((_, km)) = bus_km.iter().find(|b| b.0 == v.id) {
            v.km += km;
            v.condition = (v.condition - km * wear_per_km(&r)).max(0.0);
        }
        if cut.iter().any(|x| x.0 == v.id) {
            let days = rng.int(1, 3);
            let days = if mechanics > 0 { (days - 1).max(1) } else { days };
            let until = dates::add(&date, days);
            let size = match v.kind.size {
                BusSize::Midi => 0.8,
                BusSize::Solo => 1.0,
                _ => 1.35,
            };
            let cost = ((rng.range(800.0, 5_000.0) * size * price_index).round() as Cents) * 100;
            v.workshop_until = Some(until.clone());
            v.breakdowns += 1;
            v.condition = (v.condition - 5.0).max(0.0);
            let cost = if warranted.contains(&v.id) { 0 } else { cost };
            repairs.push((cost, format!("{} {}", v.number, v.name)));
            report.notes.push(Note::Breakdown { number: v.number.clone(), until, cost });
        } else if v.km >= v.next_service_km && !v.in_workshop(&tomorrow) && v.held_on(&tomorrow) {
            v.workshop_until = Some(tomorrow.clone());
            v.condition = v.condition.max(market::serviced_condition(dates::years_between(&v.built, &date)));
            v.next_service_km = ((v.km / market::SERVICE_KM).floor() + 1.0) * market::SERVICE_KM;
            report.notes.push(Note::Service { number: v.number.clone() });
        }
    }
    for (cost, text) in repairs {
        c.book(BookingKind::Repair, -cost, text, false);
    }

    // 7. the people tonight
    report.staff = staff::after_day(c, &worked, &mut rng);

    // 8. the month's end
    if dates::last_of_month(&date) {
        for e in c.staff.clone() {
            c.book(BookingKind::Wages, -staff::wage_for_month(&e, &month), e.name.clone(), false);
        }
        let first = dates::parse(&format!("{month}-01")).unwrap_or(0);
        let last = dates::parse(&date).unwrap_or(first);
        let len = (last - first + 1).max(1) as f64;
        let held_days = |acquired: &str, until: Option<&str>| {
            let a = dates::parse(acquired).unwrap_or(first).max(first);
            let b = until.and_then(dates::parse).unwrap_or(last).min(last);
            (b - a + 1).max(0) as f64
        };
        let mut monthly: Vec<(BookingKind, Cents, String)> = Vec::new();
        let mut depot_buses = 0;
        for v in &c.fleet {
            let text = format!("{} {}", v.number, v.name);
            match &v.tenure {
                Tenure::Leased { monthly: m, until, .. } => {
                    let days = held_days(&v.acquired, Some(until));
                    monthly.push((BookingKind::Lease, -(*m as f64 * days / len).round() as Cents, text.clone()));
                    monthly.push((BookingKind::Insurance, -(economy::insurance_per_month(v.kind, c.price_index) as f64 * days / len).round() as Cents, text));
                    depot_buses += 1;
                }
                Tenure::Owned { .. } => {
                    let days = held_days(&v.acquired, None);
                    monthly.push((BookingKind::Insurance, -(economy::insurance_per_month(v.kind, c.price_index) as f64 * days / len).round() as Cents, text));
                    depot_buses += 1;
                }
                Tenure::Rented { .. } => {}
            }
        }
        monthly.push((BookingKind::Depot, -economy::depot_per_month(depot_buses, c.price_index), c.depot.clone()));
        super::ownline::month_end(c);
        super::adverts::month_end(c);
        for (k, a, t) in monthly {
            c.book(k, a, t, false);
        }
        for purpose in finance::pay_rates(c) {
            report.notes.push(Note::LoanPaid { purpose });
        }
        c.price_index *= 1.0 + r.inflation / 12.0;
        c.contract_index *= 1.0 + r.indexation / 12.0;
        if date.ends_with("-12-31") {
            for e in c.staff.iter_mut() {
                e.holiday_left = staff::HOLIDAYS;
            }
        }
        report.notes.push(Note::Month { month: month.clone(), result: c.month(&month).result() });
    }

    // 9. reputation and punctuality
    let punctuality = (timed > 0).then(|| on_time as f64 / timed as f64 * 100.0);
    let rep0 = c.reputation;
    let mut change = -(0.25 * report.dropped as f64).min(5.0);
    // (full buses on the own lines, and people left at the stop)
    change -= (0.01 * report.crowded as f64 + 0.002 * report.left_behind as f64).min(1.5);
    if let Some(p) = punctuality {
        if report.dropped == 0 && p >= 90.0 {
            change += 0.3;
        } else if p < 75.0 {
            change -= 0.3;
        }
        c.punctuality = c.punctuality * 0.85 + p * 0.15;
    }
    change += tally.reputation();
    report.incidents = tally;
    c.reputation = (c.reputation + change).clamp(0.0, 100.0);
    report.punctuality = punctuality;
    report.reputation = c.reputation;
    report.reputation_change = c.reputation - rep0;

    // the company's progress: the day's experience, the courses that end, what its own
    // mechanics and eco drivers saved (`levels::day_closed`)
    let day_passengers: u32 = lines.iter().map(|l| l.passengers).sum();
    super::levels::day_closed(c, &date, report.covered, report.dropped, punctuality, day_passengers);

    // 10. the day's figures, and on to the next
    let after = c.month(&month);
    for k in super::model::BookingKind::ALL.iter().filter(|k| !k.is_capital()) {
        let delta = after.get(*k) - before.get(*k);
        if delta > 0 {
            report.income += delta;
        } else {
            report.expenses -= delta;
        }
    }
    report.result = report.income - report.expenses;
    report.cash = c.cash;
    report.km = lines.iter().map(|l| l.km).sum();
    report.passengers = lines.iter().map(|l| l.passengers).sum();
    report.lines = lines;
    c.history.push(DayRecord {
        date: date.clone(),
        cash: c.cash,
        income: report.income,
        expenses: report.expenses,
        result: report.result,
        tours: report.tours,
        dropped_tours: report.tours - report.covered,
        trips: report.trips,
        dropped_trips: report.dropped,
        km: report.km,
        passengers: report.passengers,
        punctuality,
    });
    if c.history.len() > HISTORY_KEPT {
        let extra = c.history.len() - HISTORY_KEPT;
        c.history.drain(..extra);
    }
    c.date = tomorrow.clone();
    if dates::parse(&tomorrow).map(dates::weekday) == Some(0) {
        for e in c.staff.iter_mut() {
            e.week_days = 0;
        }
    }
    let gone: Vec<(String, String)> = c.fleet.iter().filter(|v| !v.held_on(&tomorrow)).map(|v| (v.number.clone(), v.name.clone())).collect();
    c.fleet.retain(|v| v.held_on(&tomorrow));
    for (number, name) in gone {
        report.notes.push(Note::Returned { number, name });
    }
    c.last_report = Some(report.clone());
    report
}

#[cfg(test)]
mod tests {
    use super::super::market::{self, Payment};
    use super::super::network::tests::trip;
    use super::super::staff::{applicants, hire};
    use super::super::{found, Founding};
    use super::*;
    use crate::company::model::{CompanyLine, Difficulty};

    fn tour(no: &str, from: i32, n: i32) -> TourOfDay {
        TourOfDay { line: "Linie5".into(), number: "5".into(), tour: no.into(), ai_group: String::new(), trips: (0..n).map(|k| trip(from + k * 60, from + 50 + k * 60, 12)).collect(), unplanned: false }
    }

    fn company(d: Difficulty, buses: usize, people: usize) -> Company {
        let mut c = found(&Founding { name: "Tag".into(), difficulty: d, date: "2024-03-04".into(), map: "maps/Grundorf/global.cfg".into(), ..Default::default() }, "Luc");
        // (in service from the start, and the dispatcher's own filling what the roster leaves
        // free - as "Fill the roster" does when asked)
        c.lines.push(CompanyLine { name: "Linie5".into(), number: "5".into(), numbers: vec!["5".into()], added: c.date.clone(), service_from: Some(0), ..Default::default() });
        c.planning.auto = true;
        let bus = market::MarketBus { file: "Vehicles/Citaro/Citaro.bus".into(), name: "Citaro".into(), ..Default::default() };
        // (money enough for the buses on any difficulty)
        c.cash += 5_000_000_00;
        for _ in 0..buses {
            market::buy_new(&mut c, &bus, Payment::Cash, "").unwrap();
        }
        // (enough applicants: another market each round)
        let id = c.id.clone();
        while c.staff.len() < people {
            for a in applicants(&c) {
                if c.staff.len() < people && a.licence == crate::company::model::Licence::D {
                    hire(&mut c, &a).unwrap();
                }
            }
            c.id.push('x');
            c.taken = Default::default();
        }
        c.id = id;
        for e in c.staff.iter_mut() {
            e.sick_until = None;
            e.holiday_until = None;
        }
        c
    }

    #[test]
    fn a_school_line_is_paid_per_trip_and_strict() {
        let mut c = company(Difficulty::Realistic, 2, 4);
        c.lines[0].plan = Some(super::super::ownline::OwnPlan { service: ServiceKind::School, per_trip: [[20.0; 24]; 3], ..Default::default() });
        let date = c.date.clone();
        let tours = vec![tour("1", 6 * 60, 10), tour("2", 6 * 60 + 30, 10), tour("3", 8 * 60, 6)];
        let r = close_day(&mut c, tours, &[]);
        let of = |k: BookingKind| c.ledger.iter().filter(|b| b.date == date && b.kind == k && b.text == "Line 5").map(|b| b.amount).sum::<Cents>();
        // no fares: the school authority's contract for each of the 20 trips run (€45 and €3 a km)
        assert_eq!(of(BookingKind::Fares), 0);
        assert_eq!(of(BookingKind::Compensation), 20 * (45_00 + (15.0 * 3_00 as f64) as Cents));
        assert!(r.passengers > 0);
        // a dropped trip costs twice the contract's penalty, a late one three times
        assert_eq!(r.penalties - r.late as Cents * 15_00 * 3, 2 * (6 * 80_00 + (6.0 * 15.0 * 2_00 as f64).round() as Cents));
    }

    #[test]
    fn an_on_demand_line_runs_only_what_was_booked() {
        let mut c = company(Difficulty::Realistic, 2, 4);
        let plan = |b: f32| Some(super::super::ownline::OwnPlan { service: ServiceKind::OnDemand, per_trip: [[b; 24]; 3], ..Default::default() });
        let tours = vec![tour("1", 6 * 60, 10), tour("2", 6 * 60 + 30, 10), tour("3", 8 * 60, 6)];
        // nobody books: nothing runs, nothing is dropped - not even the tour without a bus
        c.lines[0].plan = plan(0.0);
        let quiet = close_day(&mut c.clone(), tours.clone(), &[]);
        assert_eq!((quiet.trips, quiet.dropped, quiet.passengers, quiet.penalties), (0, 0, 0, 0));
        // everybody books: every trip runs, and the tour without a bus is dropped
        c.lines[0].plan = plan(30.0);
        let busy = close_day(&mut c, tours, &[]);
        assert_eq!((busy.trips, busy.dropped), (26, 6));
        assert!(busy.passengers >= 20);
    }

    #[test]
    fn free_buses_and_drivers_cover_what_they_can() {
        // three tours, two buses, drivers for all: one tour without a bus
        let c = company(Difficulty::Realistic, 2, 6);
        let tours = vec![tour("1", 6 * 60, 8), tour("2", 6 * 60 + 20, 8), tour("3", 7 * 60, 8)];
        let plan = assign(&c, tours.clone(), &[], &[]);
        assert_eq!(plan.uncovered(), 1);
        assert_eq!(plan.short_of(), (1, 0));
        assert!(plan.tours.iter().filter(|t| t.bus.is_some()).all(|t| t.duties.iter().all(|d| d.driver.is_some())));
        // a bus does a later tour after an earlier one
        let later = vec![tour("1", 6 * 60, 3), tour("2", 10 * 60, 3)];
        let plan = assign(&company(Difficulty::Realistic, 1, 2), later, &[], &[]);
        assert_eq!(plan.uncovered(), 0);
        assert_eq!(plan.tours[0].bus, plan.tours[1].bus);
        // no drivers: nothing covered; the player's own tour needs neither
        let none = company(Difficulty::Realistic, 2, 0);
        let plan = assign(&none, tours.clone(), &[("5".into(), "2".into())], &[]);
        assert_eq!(plan.uncovered(), 2);
        assert!(plan.tours.iter().find(|t| t.tour.tour == "2").unwrap().covered());
        assert_eq!(plan.coverage_of("Linie5"), (1, 3));
        // one driver: a long tour's two duties cannot both be theirs (over ten hours)
        let long = vec![tour("1", 5 * 60, 18)];
        let plan = assign(&company(Difficulty::Realistic, 1, 1), long, &[], &[]);
        assert_eq!(plan.tours[0].duties.len(), 2);
        assert!(plan.tours[0].partly());
    }

    #[test]
    fn only_a_planned_line_in_service_runs() {
        // a line of two tours, at 06:00 and at 14:00
        let mut li = super::super::concessions::tests::line("Linie5", "5", 2, true);
        for t in li.tours[1].trips.iter_mut() {
            (t.departure, t.arrival) = (t.departure + 8.0 * 3600.0, t.arrival + 8.0 * 3600.0);
        }
        let lines = vec![li];
        let fares_of = |c: &Company, date: &str| c.ledger.iter().filter(|b| b.date == date && matches!(b.kind, BookingKind::Fares | BookingKind::Penalty)).count();
        // not planned yet: its tours are shown for the planning, but nothing runs, nothing is
        // booked or penalised
        let mut c = company(Difficulty::Realistic, 2, 4);
        c.lines[0].service_from = None;
        let date = c.date.clone();
        let tours = network::tours_of_day(&c, &lines, &date);
        assert!(tours.len() == 2 && tours.iter().all(|t| t.unplanned));
        let r = close_day(&mut c, tours, &[]);
        assert_eq!((r.tours, r.trips, r.dropped, r.penalties), (0, 0, 0, 0));
        assert!(r.lines[0].unplanned && r.lines[0].revenue == 0);
        assert_eq!(fares_of(&c, &date), 0);
        // planned and in service, but a tour without a bus: dropped, with the contract's penalty
        let mut c = company(Difficulty::Realistic, 0, 4);
        let date = c.date.clone();
        let tours = network::tours_of_day(&c, &lines, &date);
        assert!(tours.iter().all(|t| !t.unplanned));
        let r = close_day(&mut c, tours, &[]);
        assert_eq!((r.tours, r.covered, r.uncovered, r.short_buses), (2, 0, 2, 2));
        assert!(r.dropped == 8 && r.penalties > 0 && r.lines[0].penalty == r.penalties);
        // in service from noon: the morning's tour waits, the afternoon's runs
        let mut c = company(Difficulty::Realistic, 2, 4);
        let date = c.date.clone();
        network::start_service(&mut c, "Linie5", super::super::clock::moment(&date, 12 * 60)).unwrap();
        let tours = network::tours_of_day(&c, &lines, &date);
        assert_eq!(tours.iter().map(|t| t.unplanned).collect::<Vec<_>>(), vec![true, false]);
        let r = close_day(&mut c, tours, &[]);
        assert_eq!((r.tours, r.covered), (1, 1));
        assert!(r.lines[0].fares > 0 && r.lines[0].compensation > 0 && r.lines[0].running > 0);
        // and the next day both run
        let date = c.date.clone();
        assert!(network::tours_of_day(&c, &lines, &date).iter().all(|t| !t.unplanned));
    }

    #[test]
    fn a_concession_not_started_after_its_grace_is_charged() {
        for (d, charged) in [(Difficulty::Easy, false), (Difficulty::Realistic, true), (Difficulty::Hard, true)] {
            let mut c = company(d, 1, 2);
            c.lines[0].service_from = None;
            c.lines[0].km = 400.0;
            c.lines[0].added = dates::add(&c.date, -10);
            let r = close_day(&mut c, Vec::new(), &[]);
            let note = r.notes.iter().any(|n| matches!(n, Note::NotStarted { days: 10, charge, .. } if *charge > 0));
            assert_eq!(note, charged, "{d:?}");
            assert_eq!(r.penalties > 0, charged);
        }
        // within its grace, or an own line: nothing
        let mut c = company(Difficulty::Hard, 1, 2);
        c.lines[0].service_from = None;
        c.lines[0].km = 400.0;
        c.lines[0].added = dates::add(&c.date, -2);
        assert_eq!(close_day(&mut c, Vec::new(), &[]).penalties, 0);
        c.lines[0].own = true;
        c.lines[0].added = dates::add(&c.date, -20);
        assert_eq!(close_day(&mut c, Vec::new(), &[]).penalties, 0);
    }

    #[test]
    fn closing_a_day_books_it_and_moves_on() {
        let mut c = company(Difficulty::Realistic, 2, 4);
        let tours = vec![tour("1", 6 * 60, 10), tour("2", 6 * 60 + 30, 10), tour("3", 8 * 60, 6)];
        let cash = c.cash;
        let r = close_day(&mut c, tours, &[]);
        assert_eq!(c.date, "2024-03-05");
        assert_eq!((r.tours, r.covered), (3, 2));
        // the tour without a bus was dropped, with its penalty
        assert_eq!(r.dropped, 6);
        assert_eq!(r.penalties - r.late as Cents * 15_00, 6 * 80_00 + (6.0 * 15.0 * 2_00 as f64).round() as Cents);
        assert!(r.passengers > 0 && r.km > 0.0 && r.income > 0 && r.expenses > 0);
        assert_eq!(c.cash - cash, r.result);
        assert_eq!(c.history.len(), 1);
        assert!(c.reputation < 50.0);
        // the buses ran their kilometres; the drivers worked
        assert!(c.fleet.iter().all(|v| v.km > 0.0 && v.condition < 100.0));
        assert!(c.staff.iter().filter(|e| e.days_worked == 1).count() >= 2);
        assert!(c.last_report.is_some());
        // the same day closed again on a copy draws the same
        let mut a = company(Difficulty::Realistic, 2, 4);
        let mut b = a.clone();
        let t = vec![tour("1", 6 * 60, 10)];
        assert_eq!(close_day(&mut a, t.clone(), &[]), close_day(&mut b, t, &[]));
    }

    #[test]
    fn drivers_trained_in_ticket_sales_bring_more_fares() {
        use super::super::training::{Course, CourseKind};
        let plain = company(Difficulty::Realistic, 2, 4);
        let mut trained = plain.clone();
        for (k, e) in plain.staff.iter().enumerate() {
            let x = Course { id: k as u32 + 1, kind: CourseKind::Ticketing, employee: Some(e.id), name: e.name.clone(), from: plain.date.clone(), until: plain.date.clone(), cost: 0, done: true, subject: String::new() };
            trained.progress.courses.push(x);
        }
        let month = plain.date[..7].to_string();
        let tours = vec![tour("1", 6 * 60, 10), tour("2", 6 * 60 + 30, 10)];
        let (mut a, mut b) = (plain, trained);
        let (ra, rb) = (close_day(&mut a, tours.clone(), &[]), close_day(&mut b, tours, &[]));
        assert_eq!(ra.passengers, rb.passengers);
        let (fa, fb) = (a.month(&month).get(BookingKind::Fares) as f64, b.month(&month).get(BookingKind::Fares) as f64);
        assert!(fa > 0.0 && fb > fa * 1.03 && fb < fa * 1.05, "{fa} {fb}");
        // what else befell the trips is told in the report, and the same on both
        assert_eq!(ra.incidents, rb.incidents);
    }

    #[test]
    fn the_players_own_trips_are_measured() {
        let mut c = company(Difficulty::Realistic, 0, 0);
        let run = TripRun { time: 1_700_000_000, map: "maps\\Grundorf\\global.cfg".into(), line: "5".into(), tour: "1".into(), stops: 12, passengers: 30, seconds: 1800.0, metres: 9000.0, completed: true, ..Default::default() };
        let elsewhere = TripRun { map: "maps/Other/global.cfg".into(), time: 1_700_000_001, ..run.clone() };
        let r = close_day(&mut c, vec![tour("1", 6 * 60, 4)], &[run.clone(), elsewhere]);
        assert_eq!(r.measured, 1);
        assert_eq!((r.tours, r.covered, r.by_player), (1, 1, 1));
        assert_eq!(r.dropped, 0);
        assert!(c.ledger.iter().any(|b| b.measured && b.kind == BookingKind::Fares && b.amount == 30 * 1_10));
        assert_eq!(c.trips_seen, 1_700_000_000);
        // booked once
        let r = close_day(&mut c, vec![], &[run]);
        assert_eq!(r.measured, 0);
        // the live hook counts too
        record_live(&mut c, LiveEvent::Trip { line: "5".into(), tour: "2".into(), vehicle: None, km: 8.0, passengers: 20, delay: 30.0, completed: true, dep: None });
        let r = close_day(&mut c, vec![tour("2", 6 * 60, 4)], &[]);
        assert_eq!((r.covered, r.dropped), (1, 0));
        assert!(c.live.is_empty());
    }

    #[test]
    fn a_month_end_books_the_wages_and_the_fixed_costs() {
        let mut c = company(Difficulty::Realistic, 1, 2);
        c.date = "2024-03-31".into();
        let r = close_day(&mut c, vec![], &[]);
        let m = c.month("2024-03");
        assert!(m.get(BookingKind::Wages) < 0 && m.get(BookingKind::Insurance) < 0 && m.get(BookingKind::Depot) < 0);
        assert!(r.notes.iter().any(|n| matches!(n, Note::Month { .. })));
        assert!(c.price_index > 1.0);
        assert_eq!(c.date, "2024-04-01");
        // a rented bus goes back after its last day
        let bus = market::MarketBus { file: "Vehicles/Citaro/Citaro.bus".into(), name: "Citaro".into(), ..Default::default() };
        market::rent(&mut c, &bus, 1, "").unwrap();
        let n = c.fleet.len();
        let r = close_day(&mut c, vec![], &[]);
        assert_eq!(c.fleet.len(), n - 1);
        assert!(r.notes.iter().any(|x| matches!(x, Note::Returned { .. })));
        assert!(c.month("2024-04").get(BookingKind::Rent) < 0);
    }

    #[test]
    fn hard_days_break_more_buses_than_easy_ones() {
        let count = |d: Difficulty| {
            let mut c = company(d, 4, 10);
            let mut n = 0;
            for _ in 0..120 {
                let tours = vec![tour("1", 6 * 60, 8), tour("2", 6 * 60 + 10, 8)];
                for v in c.fleet.iter_mut() {
                    v.condition = 25.0;
                    v.workshop_until = None;
                }
                let r = close_day(&mut c, tours, &[]);
                n += r.notes.iter().filter(|x| matches!(x, Note::Breakdown { .. })).count();
                for e in c.staff.iter_mut() {
                    e.sick_until = None;
                    e.holiday_until = None;
                    e.week_days = 0;
                    e.last_end = None;
                }
            }
            n
        };
        assert!(count(Difficulty::Hard) > count(Difficulty::Easy));
    }
}
