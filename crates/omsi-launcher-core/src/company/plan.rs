//! The planning: who drives what, day by day (Omsi-Hub's `rooster.ts`, `planregels.ts`,
//! `uitval.ts` and `invulling.ts`; the design `busbedrijf-planning.md`).
//!
//! The company's lines give it tours every day (`network::tours_of_day`), each cut into
//! duties (`network::duties_of`, Omsi-Hub's `knipOmloop`). The roster is fixed per weekday
//! and repeats every week: a bus for a tour, a driver - or the player himself - for each
//! duty. Only what is planned runs: "Fill the roster" asks the dispatcher to fill what the
//! roster leaves free (the free buses, the free drivers by the working-time rules,
//! `staff::check`) and fixes it in the roster for the player to see; a line runs only once it
//! is put into service (`network::start_service`). What falls
//! out on the day is open: someone ill or late, a bus that does not start in the morning,
//! the player's own duty not driven. The dispatcher's choice for it counts (a colleague, an
//! agency driver, another bus, a rental bus, or dropping its trips); without one the
//! central fills what came suddenly - a spare colleague, else an agency driver, a free bus,
//! else a rental bus - and the rest is dropped, with the contract's penalty.
//!
//! The morning is drawn from the company and the date alone (`disruptions`), so the planning
//! page and "Close the day" see the same day. The plan of the company's own day is also
//! written for the game (`LivePlan`): while the player drives, the company's tours on the
//! map run with the company's buses as AI, and what they drive comes back through
//! `day::record_live`.

use super::dates;
use super::day::{DutyPlan, LiveEvent, Note, Plan, TourPlan};
use super::economy;
use super::market::{self, MarketBus};
use super::model::{BookingKind, BusKind, BusSize, Cents, Company, Difficulty, Drive, Employee, Licence, Skills};
use super::network::{self, TourOfDay};
use super::rng::Rng;
use super::staff::{self, Block, BUS_MARGIN, WEEK_DAYS};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The driver id an agency driver stands under in a closing day's plan.
pub const AGENCY: u32 = u32::MAX;
/// The driver id the player stands under in a plan that is not being closed (his duty is
/// covered: he drives it).
pub const PLAYER: u32 = u32::MAX - 1;
/// An agency driver's hour, the agency's margin included (founding day's prices).
pub const AGENCY_HOUR: Cents = 42_00;

/// Agency drivers a day at most: one, and one more for every five on the payroll.
pub fn agency_max(c: &Company) -> usize {
    1 + c.staff.len() / 5
}

// --- the roster ------------------------------------------------------------------------------

/// Who drives a duty.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    Staff(u32),
    /// The player himself.
    Player,
    /// A driver of an agency, for the day.
    Agency,
}

/// A tour of the weekly roster: on a weekday, its bus and who drives each of its duties.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct RosterTour {
    /// 0 Monday .. 6 Sunday.
    pub weekday: u8,
    /// The company line (its timetable name) and the tour.
    pub line: String,
    pub tour: String,
    #[serde(default)]
    pub bus: Option<u32>,
    #[serde(default)]
    pub duties: Vec<Option<Who>>,
}

impl RosterTour {
    fn is_empty(&self) -> bool {
        self.bus.is_none() && self.duties.iter().all(Option::is_none)
    }
}

/// The dispatcher's choice for something open today.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fill {
    Colleague { id: u32 },
    Agency,
    Bus { id: u32 },
    Rental,
    Drop,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FillChoice {
    /// `tour_key`, `duty_key` or `late_key`.
    pub key: String,
    pub fill: Fill,
}

/// The company's planning as it is saved.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Planning {
    #[serde(default)]
    pub week: Vec<RosterTour>,
    /// The dispatcher fills what the roster leaves free - only for "Fill the roster"
    /// (`fill_day`), which shows the player what it would take and fixes it in the roster: the
    /// day itself runs what is planned, nothing behind the player's back. (Not kept: an older
    /// file's "auto" is read as off.)
    #[serde(skip)]
    pub auto: bool,
    /// The day the dispatcher's choices are for (they hold for that day only).
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub fills: Vec<FillChoice>,
}

impl Default for Planning {
    fn default() -> Self {
        Planning { week: Vec::new(), auto: false, date: String::new(), fills: Vec::new() }
    }
}

pub fn weekday_of(date: &str) -> u8 {
    dates::parse(date).map(|d| dates::weekday(d) as u8).unwrap_or(0)
}

pub fn tour_key(line: &str, tour: &str) -> String {
    format!("{line}|{tour}")
}

pub fn duty_key(line: &str, tour: &str, duty: usize) -> String {
    format!("{line}|{tour}|{duty}")
}

/// The first piece of a duty whose driver comes late.
pub fn late_key(line: &str, tour: &str, duty: usize) -> String {
    format!("{line}|{tour}|{duty}|late")
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// The roster's tour on a weekday.
pub fn roster<'a>(c: &'a Company, weekday: u8, line: &str, tour: &str) -> Option<&'a RosterTour> {
    c.planning.week.iter().find(|r| r.weekday == weekday && same(&r.line, line) && same(&r.tour, tour))
}

fn roster_mut<'a>(c: &'a mut Company, weekday: u8, line: &str, tour: &str) -> &'a mut RosterTour {
    let at = match c.planning.week.iter().position(|r| r.weekday == weekday && same(&r.line, line) && same(&r.tour, tour)) {
        Some(k) => k,
        None => {
            c.planning.week.push(RosterTour { weekday, line: line.to_string(), tour: tour.to_string(), bus: None, duties: Vec::new() });
            c.planning.week.len() - 1
        }
    };
    &mut c.planning.week[at]
}

fn prune(c: &mut Company) {
    c.planning.week.retain(|r| !r.is_empty());
}

/// Give a tour of the roster its bus (None: none fixed).
pub fn set_bus(c: &mut Company, weekday: u8, line: &str, tour: &str, bus: Option<u32>) {
    roster_mut(c, weekday, line, tour).bus = bus;
    prune(c);
}

/// Give a duty of the roster its driver (None: none fixed).
pub fn set_driver(c: &mut Company, weekday: u8, line: &str, tour: &str, duty: usize, who: Option<Who>) {
    let r = roster_mut(c, weekday, line, tour);
    if r.duties.len() <= duty {
        r.duties.resize(duty + 1, None);
    }
    r.duties[duty] = who;
    while r.duties.last() == Some(&None) {
        r.duties.pop();
    }
    prune(c);
}

/// Clear a weekday of the roster.
pub fn clear_day(c: &mut Company, weekday: u8) -> Rostered {
    let gone = rostered(c, weekday);
    c.planning.week.retain(|r| r.weekday != weekday);
    gone
}

/// A line renamed (an own line's new number: its timetable name follows): its roster too.
pub fn rename_line(c: &mut Company, old: &str, new: &str) {
    if old.eq_ignore_ascii_case(new) {
        return;
    }
    for r in c.planning.week.iter_mut().filter(|r| r.line.eq_ignore_ascii_case(old)) {
        r.line = new.to_string();
    }
}

/// The roster's entries of a line's `tours` taken out, every weekday (a changed timetable:
/// their buses and drivers are given anew). Returns how many went.
pub fn forget_tours(c: &mut Company, line: &str, tours: &[String]) -> usize {
    let before = c.planning.week.len();
    c.planning.week.retain(|r| !(r.line.eq_ignore_ascii_case(line) && tours.iter().any(|t| t.trim() == r.tour.trim())));
    before - c.planning.week.len()
}

/// What a weekday's roster holds: the duties given to someone, the buses given.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rostered {
    pub duties: usize,
    pub buses: usize,
}

impl Rostered {
    pub fn is_empty(&self) -> bool {
        self.duties + self.buses == 0
    }
}

pub fn rostered(c: &Company, weekday: u8) -> Rostered {
    let days = c.planning.week.iter().filter(|r| r.weekday == weekday);
    days.fold(Rostered::default(), |n, r| Rostered { duties: n.duties + r.duties.iter().flatten().count(), buses: n.buses + usize::from(r.bus.is_some()) })
}

/// The roster of one weekday on others too (Monday's on the other working days): returns
/// the days it went to (none when the weekday's roster is empty - the others are kept then).
pub fn copy_day(c: &mut Company, from: u8, to: &[u8]) -> Vec<u8> {
    if rostered(c, from).is_empty() {
        return Vec::new();
    }
    let src: Vec<RosterTour> = c.planning.week.iter().filter(|r| r.weekday == from).cloned().collect();
    let days: Vec<u8> = to.iter().copied().filter(|d| *d != from).collect();
    for &d in &days {
        clear_day(c, d);
        c.planning.week.extend(src.iter().cloned().map(|mut r| {
            r.weekday = d;
            r
        }));
    }
    days
}

/// The dispatcher's choice for something open on the company's day (None: the central's).
pub fn set_fill(c: &mut Company, key: &str, fill: Option<Fill>) {
    if c.planning.date != c.date {
        c.planning.date = c.date.clone();
        c.planning.fills.clear();
    }
    c.planning.fills.retain(|f| f.key != key);
    if let Some(fill) = fill {
        c.planning.fills.push(FillChoice { key: key.to_string(), fill });
    }
}

fn fill_of(c: &Company, date: &str, key: &str) -> Option<Fill> {
    (c.planning.date == date).then(|| c.planning.fills.iter().find(|f| f.key == key).map(|f| f.fill)).flatten()
}

/// Fill the roster of `date`'s weekday with what the dispatcher would take (the free buses
/// and drivers by the rules, those fixed already kept): returns how many were added.
pub fn fill_day(c: &mut Company, date: &str, tours: Vec<TourOfDay>) -> usize {
    let f = fill_day_told(c, date, tours);
    f.duties + f.buses
}

/// What "Fill the roster" did, and what it left open and why.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Filled {
    /// Duties given to drivers, buses given to tours.
    pub duties: usize,
    pub buses: usize,
    /// Duties still without a driver: for which nobody employed is qualified (the licence,
    /// an endorsement, the type training), and for which the qualified are not free.
    pub unqualified: usize,
    pub busy: usize,
    /// Tours still without a bus.
    pub no_bus: usize,
}

/// `fill_day`, told.
pub fn fill_day_told(c: &mut Company, date: &str, tours: Vec<TourOfDay>) -> Filled {
    fill(c, date, tours, None)
}

/// "Fill the roster" for one line's tours only (a changed line planned anew), the day's
/// other tours weighed as they are.
pub fn fill_line(c: &mut Company, date: &str, tours: Vec<TourOfDay>, line: &str) -> Filled {
    fill(c, date, tours, Some(line))
}

fn fill(c: &mut Company, date: &str, tours: Vec<TourOfDay>, only: Option<&str>) -> Filled {
    let ours = |t: &DayTour| only.is_none_or(|l| t.tour.line.eq_ignore_ascii_case(l));
    let mut probe = c.clone();
    probe.planning.auto = true;
    let p = day_plan(&probe, date, tours, &[], &[], false);
    let wd = weekday_of(date);
    let mut f = Filled::default();
    let staff: Vec<&Employee> = c.staff.iter().filter(|e| e.employed_on(date)).collect();
    for t in &p.tours {
        if t.by_player || !ours(t) {
            continue;
        }
        // (a tour not in service is not open)
        let open = !t.tour.unplanned;
        match (t.bus, t.bus_from) {
            (Some(BusOf::Own(_)), Source::Auto) => f.buses += 1,
            (None, _) if open => f.no_bus += 1,
            _ => {}
        }
        let bus = t.bus_of(&probe);
        for d in &t.duties {
            match (d.who, d.from_) {
                (Some(Who::Staff(_)), Source::Auto) => f.duties += 1,
                (None, _) if !open => {}
                (None, _) if staff.iter().all(|e| super::licences::lack(e, bus.0, bus.1.as_deref()).is_some()) => f.unqualified += 1,
                (None, _) => f.busy += 1,
                _ => {}
            }
        }
    }
    for t in &p.tours {
        if t.by_player || !ours(t) {
            continue;
        }
        if let (Some(BusOf::Own(id)), Source::Auto) = (t.bus, t.bus_from) {
            set_bus(c, wd, &t.tour.line, &t.tour.tour, Some(id));
        }
        for (k, d) in t.duties.iter().enumerate() {
            if let (Some(w @ Who::Staff(_)), Source::Auto) = (d.who, d.from_) {
                set_driver(c, wd, &t.tour.line, &t.tour.tour, k, Some(w));
            }
        }
    }
    f
}

// --- the morning -----------------------------------------------------------------------------

/// What befalls the company in the morning of a day (drawn from the company and the date:
/// the same day draws the same).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Disruption {
    /// Someone comes `minutes` late to their first duty.
    Late { employee: u32, minutes: i32 },
    /// A bus does not start: in the workshop for the day; the tow and the parts cost `cost`.
    Breakdown { vehicle: u32, cost: Cents },
}

pub fn disruptions(c: &Company, date: &str) -> Vec<Disruption> {
    let r = economy::rules(c.difficulty);
    let day = dates::parse(date).unwrap_or(0);
    let late_factor = match c.difficulty {
        Difficulty::Easy => 0.5,
        Difficulty::Realistic => 1.0,
        Difficulty::Hard => 1.4,
    };
    let mut out = Vec::new();
    for e in c.staff.iter().filter(|e| e.employed_on(date) && !e.absent(date)) {
        let mut rng = Rng::of(&[&c.id, "late", &e.id.to_string()], day);
        let p = (0.03 + if e.satisfaction < 50.0 { 0.02 } else { 0.0 }) * late_factor * (1.5 - 0.5 * e.reliability);
        if rng.chance(p) {
            out.push(Disruption::Late { employee: e.id, minutes: 20 + 10 * rng.int(0, 4) as i32 });
        }
    }
    for v in c.fleet.iter().filter(|v| v.held_on(date) && !v.in_workshop(date) && v.condition >= 20.0) {
        let mut rng = Rng::of(&[&c.id, "start", &v.id.to_string()], day);
        let p = (0.01 + (70.0 - v.condition).max(0.0) / 70.0 * 0.06) * (0.5 + 0.5 * r.breakdown_factor);
        if rng.chance(p) {
            let cost = (rng.range(200.0, 600.0) * c.price_index).round() as Cents * 100;
            out.push(Disruption::Breakdown { vehicle: v.id, cost });
        }
    }
    out
}

// --- the plan of a day -----------------------------------------------------------------------

/// How a bus or a driver came to a tour or a duty.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    #[default]
    None,
    Roster,
    /// The dispatcher filled what the roster left free.
    Auto,
    /// The dispatcher's choice for the day.
    Dispatcher,
    /// The central filled what fell out.
    Central,
}

/// Why a tour has no bus, or a duty no driver.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Problem {
    Sick,
    Holiday,
    /// Not employed that day, gone, or five days worked this week.
    Away,
    /// The licence does not cover the bus.
    Licence,
    /// Drives something else then, or the working-time rules forbid it.
    Conflict,
    /// The bus did not start this morning.
    Breakdown,
    Workshop,
    /// No longer the company's bus.
    Gone,
    /// On another tour then.
    BusBusy,
    /// Nobody, or no bus, planned (only what is planned runs).
    Unassigned,
    /// The player's own duty, not driven.
    Player,
    /// The bus fixed for it is none of the buses its line asks for (`ownline::line_allows`).
    NotLineBus,
    /// The fleet has none of the buses its line asks for.
    NoLineBus,
    /// No endorsement for the bus (articulated, double-decker, electric: `licences`).
    NoEndorsement,
    /// No type training for the bus's model.
    NoTypeTraining,
}

impl Problem {
    /// Came suddenly: the central fills it by itself.
    pub fn sudden(self) -> bool {
        matches!(self, Problem::Sick | Problem::Breakdown | Problem::Player)
    }

    pub fn label(self) -> &'static str {
        match self {
            Problem::Sick => "Ill",
            Problem::Holiday => "On holiday",
            Problem::Away => "Not available",
            Problem::Licence => "No licence for this bus",
            Problem::Conflict => "Clashes with other work",
            Problem::Breakdown => "Did not start",
            Problem::Workshop => "In the workshop",
            Problem::Gone => "No longer in the fleet",
            Problem::BusBusy => "On another tour then",
            Problem::Unassigned => "Not planned",
            Problem::Player => "Your duty, not driven",
            Problem::NotLineBus => "Not one of the line's buses",
            Problem::NoLineBus => "The fleet has none of the line's buses",
            Problem::NoEndorsement => "No licence for this kind of bus",
            Problem::NoTypeTraining => "No type training for this bus",
        }
    }
}

/// What a fitting assignment warns of.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Warn {
    /// Minutes to change to another stop.
    Transfer(i32),
    /// Minutes over the day's eight hours.
    Overtime(i32),
    /// Little experience for a big bus.
    Experience,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum BusOf {
    Own(u32),
    /// A bus rented for the day.
    Rental,
}

/// A driver late for a duty's beginning: open until `until`, unless `cover` drives it.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct LatePiece {
    pub until: i32,
    pub cover: Option<u32>,
    pub from_: Source,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct DayDuty {
    /// Its trips (indices into the tour's trips) and its time.
    pub start: usize,
    pub end: usize,
    pub from: i32,
    pub to: i32,
    pub who: Option<Who>,
    pub from_: Source,
    /// What made it open (it stays open while `who` is None).
    pub problem: Option<Problem>,
    pub late: Option<LatePiece>,
    pub warn: Vec<Warn>,
}

impl DayDuty {
    pub fn open(&self) -> bool {
        self.who.is_none()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct DayTour {
    pub tour: TourOfDay,
    pub bus: Option<BusOf>,
    pub bus_from: Source,
    pub bus_problem: Option<Problem>,
    pub duties: Vec<DayDuty>,
    /// The player drove it (a trip report says so), or the game reported it live.
    pub by_player: bool,
    pub live: bool,
}

impl DayTour {
    pub fn covered(&self) -> bool {
        self.by_player || (self.bus.is_some() && self.duties.iter().all(|d| d.who.is_some()))
    }

    pub fn key(&self) -> String {
        tour_key(&self.tour.line, &self.tour.tour)
    }

    /// The bus it runs with as a driver's qualification sees it: its kind and its file (a
    /// rental or no bus yet: the size it asks for, no model).
    pub fn bus_of(&self, c: &Company) -> (BusKind, Option<String>) {
        match self.bus.and_then(|b| if let BusOf::Own(id) = b { c.vehicle(id) } else { None }) {
            Some(v) => (v.kind, Some(v.bus.clone())),
            None => (BusKind { size: self.size(c), drive: Drive::Diesel }, None),
        }
    }

    /// The size of bus it runs with (or asks for).
    pub fn size(&self, c: &Company) -> BusSize {
        match self.bus {
            Some(BusOf::Own(id)) => c.vehicle(id).map(|v| v.kind.size).unwrap_or_default(),
            _ => super::ownline::wanted(c, &self.tour).unwrap_or_default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct DayPlan {
    pub date: String,
    pub weekday: u8,
    /// The company's own day (the morning is known, the dispatcher's choices count).
    pub today: bool,
    pub tours: Vec<DayTour>,
    pub disruptions: Vec<Disruption>,
}

/// Something open, or something that was open and got filled, for the list of open duties.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenRow {
    pub tour: usize,
    /// None: the tour's bus.
    pub duty: Option<usize>,
    /// The late piece of the duty.
    pub late: bool,
    pub key: String,
    pub problem: Option<Problem>,
    /// Who or what fills it now (None: its trips are dropped).
    pub filled: bool,
    pub from_: Source,
}

impl DayPlan {
    /// Tours, tours covered, duties open (of tours with a bus), and tours without a bus.
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let mine: Vec<&DayTour> = self.tours.iter().filter(|t| !t.by_player).collect();
        let covered = self.tours.iter().filter(|t| t.covered()).count();
        let open = mine.iter().filter(|t| t.bus.is_some()).flat_map(|t| t.duties.iter()).filter(|d| d.open()).count();
        let no_bus = mine.iter().filter(|t| t.bus.is_none()).count();
        (self.tours.len(), covered, open, no_bus)
    }

    /// What fell out or stayed free, filled or not (the dispatcher's list).
    pub fn open_rows(&self) -> Vec<OpenRow> {
        let mut out = Vec::new();
        for (ti, t) in self.tours.iter().enumerate() {
            if t.by_player {
                continue;
            }
            if t.bus_problem.is_some() || t.bus.is_none() {
                out.push(OpenRow { tour: ti, duty: None, late: false, key: t.key(), problem: t.bus_problem.or(Some(Problem::Unassigned)), filled: t.bus.is_some(), from_: t.bus_from });
            }
            if t.bus.is_none() {
                continue;
            }
            for (k, d) in t.duties.iter().enumerate() {
                if d.problem.is_some() || d.who.is_none() {
                    out.push(OpenRow { tour: ti, duty: Some(k), late: false, key: duty_key(&t.tour.line, &t.tour.tour, k), problem: d.problem.or(Some(Problem::Unassigned)), filled: d.who.is_some(), from_: d.from_ });
                }
                if let Some(l) = d.late {
                    out.push(OpenRow { tour: ti, duty: Some(k), late: true, key: late_key(&t.tour.line, &t.tour.tour, k), problem: None, filled: l.cover.is_some(), from_: l.from_ });
                }
            }
        }
        out
    }

    /// The duties of a driver, (tour, duty), in the order they begin.
    pub fn duties_of(&self, who: Who) -> Vec<(usize, usize)> {
        let mut v: Vec<(usize, usize)> = self.tours.iter().enumerate().flat_map(|(ti, t)| t.duties.iter().enumerate().filter(|(_, d)| d.who == Some(who)).map(move |(k, _)| (ti, k))).collect();
        v.sort_by_key(|&(ti, k)| self.tours[ti].duties[k].from);
        v
    }

    /// The plan as the day's close takes it.
    pub fn to_plan(&self) -> Plan {
        Plan {
            tours: self
                .tours
                .iter()
                .map(|t| TourPlan {
                    tour: t.tour.clone(),
                    bus: match t.bus {
                        Some(BusOf::Own(id)) => Some(id),
                        _ => None,
                    },
                    duties: t
                        .duties
                        .iter()
                        .map(|d| DutyPlan {
                            start: d.start,
                            end: d.end,
                            from: d.from,
                            to: d.to,
                            driver: match d.who {
                                Some(Who::Staff(id)) => Some(id),
                                Some(Who::Agency) => Some(AGENCY),
                                Some(Who::Player) => Some(PLAYER),
                                None => None,
                            },
                            dropped_before: d.late.filter(|l| l.cover.is_none()).map(|l| l.until),
                        })
                        .collect(),
                    by_player: t.by_player,
                    live: t.live,
                })
                .collect(),
        }
    }
}

/// Whether a tour is one of `keys` (a line's number or name, and the tour).
fn is_one_of(t: &TourOfDay, keys: &[(String, String)]) -> bool {
    keys.iter().any(|(l, n)| (same(&t.number, l) || same(&t.line, l) || t.trips.iter().any(|x| same(&x.line, l))) && t.tour.trim() == n.trim())
}

/// When each bus is out: (id, (from, to) of its tours).
#[derive(Default)]
struct Buses(Vec<(u32, Vec<(i32, i32)>)>);

impl Buses {
    fn free(&self, id: u32, from: i32, to: i32) -> bool {
        self.0.iter().find(|b| b.0 == id).is_none_or(|b| b.1.iter().all(|&(a, z)| to + BUS_MARGIN <= a || z + BUS_MARGIN <= from))
    }

    fn take(&mut self, id: u32, from: i32, to: i32) {
        match self.0.iter_mut().find(|b| b.0 == id) {
            Some(b) => b.1.push((from, to)),
            None => self.0.push((id, vec![(from, to)])),
        }
    }

    /// When it was last out before `from` (the latest end; MIN if never).
    fn last_end(&self, id: u32, from: i32) -> i32 {
        self.0.iter().find(|b| b.0 == id).and_then(|b| b.1.iter().filter(|x| x.1 <= from).map(|x| x.1).max()).unwrap_or(i32::MIN)
    }
}

/// What each driver drives: (id, blocks).
#[derive(Default)]
struct Work(Vec<(u32, Vec<Block>)>);

impl Work {
    fn of(&self, id: u32) -> &[Block] {
        self.0.iter().find(|w| w.0 == id).map(|w| w.1.as_slice()).unwrap_or(&[])
    }

    fn push(&mut self, id: u32, b: Block) {
        match self.0.iter_mut().find(|w| w.0 == id) {
            Some(w) => w.1.push(b),
            None => self.0.push((id, vec![b])),
        }
    }
}

fn block_of(t: &TourOfDay, d: &DayDuty, key: String) -> Block {
    Block { key, from: d.from, to: d.to, from_stop: t.trips[d.start].from.clone(), to_stop: t.trips[d.end - 1].to.clone() }
}

fn covers(u: &Option<String>, date: &str) -> bool {
    u.as_deref().is_some_and(|u| dates::between(date, u) >= 0)
}

/// Why someone cannot drive on `date` at all (None: they can).
fn unavailable(e: &Employee, date: &str, today: bool) -> Option<Problem> {
    if !e.employed_on(date) {
        Some(Problem::Away)
    } else if covers(&e.sick_until, date) {
        Some(Problem::Sick)
    } else if covers(&e.holiday_until, date) {
        Some(Problem::Holiday)
    } else if today && e.week_days >= WEEK_DAYS {
        Some(Problem::Away)
    } else {
        None
    }
}

/// How well a bus of `size` fits a tour asking for `wants`: the size asked for (or none
/// asked), a bigger bus (it carries the load), a smaller one.
fn size_fit(wants: Option<BusSize>, size: BusSize) -> u8 {
    match wants {
        None => 2,
        Some(w) if w == size => 2,
        Some(w) if super::ownline::capacity(size) >= super::ownline::capacity(w) => 1,
        Some(_) => 0,
    }
}

/// How a duty (or a piece) on a bus (`DayTour::bus_of`) fits a driver: qualified for it - the
/// licence, the endorsements, the model's type training (`licences`) - and then as `fits`.
pub fn fits_bus(e: &Employee, bus: &(BusKind, Option<String>), work: &[Block], b: &Block, today: bool) -> Result<Vec<Warn>, Problem> {
    match super::licences::lack(e, bus.0, bus.1.as_deref()) {
        Some(super::licences::Lack::Licence) => Err(Problem::Licence),
        Some(super::licences::Lack::Endorsement(_)) => Err(Problem::NoEndorsement),
        Some(super::licences::Lack::Type(_)) => Err(Problem::NoTypeTraining),
        None => fits(e, bus.0.size, work, b, today),
    }
}

/// How a duty (or a piece) fits a driver: allowed, and the warnings.
pub fn fits(e: &Employee, size: BusSize, work: &[Block], b: &Block, today: bool) -> Result<Vec<Warn>, Problem> {
    if !staff::may_drive(e, size) {
        return Err(Problem::Licence);
    }
    let k = staff::check(work, b, if today { e.last_end } else { None }, None);
    if !k.allowed() {
        return Err(Problem::Conflict);
    }
    let mut w = Vec::new();
    if let Some(t) = k.transfer {
        w.push(Warn::Transfer(t));
    }
    if k.overtime > 0 {
        w.push(Warn::Overtime(k.overtime));
    }
    if !staff::qualified(e, size) {
        w.push(Warn::Experience);
    }
    Ok(w)
}

/// The plan of `date`: its tours (`network::tours_of_day` of that date's timetable) given
/// buses and drivers - the roster's first, then the dispatcher's own when `auto` is on; on
/// the company's own day the morning and what is open handled. `player` and `live` are the
/// tours the player drove and the game reported (they need nothing of the model);
/// `closing`: the day is being closed, so the player's duties he did not drive are open.
pub fn day_plan(c: &Company, date: &str, tours: Vec<TourOfDay>, player: &[(String, String)], live: &[(String, String)], closing: bool) -> DayPlan {
    let today = date == c.date;
    let wd = weekday_of(date);
    let dis = if today { disruptions(c, date) } else { Vec::new() };
    let broken: Vec<u32> = dis.iter().filter_map(|d| if let Disruption::Breakdown { vehicle, .. } = d { Some(*vehicle) } else { None }).collect();
    let usable = |id: u32| c.vehicle(id).is_some_and(|v| v.held_on(date) && !v.in_workshop(date) && v.condition >= 20.0) && !broken.contains(&id);
    let mut tours = tours;
    tours.sort_by_key(|t| t.from());
    let mut out: Vec<DayTour> = tours
        .into_iter()
        .map(|t| {
            let duties = network::duties_of(&t)
                .into_iter()
                .map(|r| DayDuty { from: t.trips[r.start].dep, to: t.trips[r.clone()].iter().map(|x| x.arr).max().unwrap_or(0), start: r.start, end: r.end, ..Default::default() })
                .collect();
            let by_player = is_one_of(&t, player);
            let live = !by_player && is_one_of(&t, live);
            DayTour { tour: t, duties, by_player, live, ..Default::default() }
        })
        .collect();

    // 1. the buses: the roster's, then the dispatcher's own, then what fell out
    let mut buses = Buses::default();
    for t in out.iter_mut().filter(|t| !t.by_player) {
        let Some(id) = roster(c, wd, &t.tour.line, &t.tour.tour).and_then(|r| r.bus) else { continue };
        let (from, to) = (t.tour.from(), t.tour.to());
        let problem = match c.vehicle(id) {
            None => Some(Problem::Gone),
            Some(v) if !v.held_on(date) => Some(Problem::Gone),
            Some(_) if broken.contains(&id) => Some(Problem::Breakdown),
            Some(v) if v.in_workshop(date) || v.condition < 20.0 => Some(Problem::Workshop),
            Some(v) if !super::ownline::line_allows(c, &t.tour.line, v) => Some(Problem::NotLineBus),
            Some(_) if !buses.free(id, from, to) => Some(Problem::BusBusy),
            Some(_) => None,
        };
        match problem {
            None => {
                buses.take(id, from, to);
                t.bus = Some(BusOf::Own(id));
                t.bus_from = Source::Roster;
            }
            p => t.bus_problem = p,
        }
    }
    let rostered_bus: Vec<u32> = c.planning.week.iter().filter(|r| r.weekday == wd).filter_map(|r| r.bus).collect();
    for t in out.iter_mut().filter(|t| !t.by_player && t.bus.is_none() && t.bus_problem.is_none()) {
        // (a line that asks for buses the fleet has none of says so)
        let none = if super::ownline::fleet_has_bus_for(c, &t.tour.line) { Problem::Unassigned } else { Problem::NoLineBus };
        if !c.planning.auto {
            t.bus_problem = Some(none);
            continue;
        }
        let (from, to, wants) = (t.tour.from(), t.tour.to(), super::ownline::wanted(c, &t.tour));
        // (free in time and one of the line's buses; the size asked for first, a bus no roster
        // tour has, then the one free the latest: the others stay free for later tours)
        let best = c
            .fleet
            .iter()
            .filter(|v| usable(v.id) && buses.free(v.id, from, to) && super::ownline::line_allows(c, &t.tour.line, v))
            .max_by_key(|v| (size_fit(wants, v.kind.size), !rostered_bus.contains(&v.id), buses.last_end(v.id, from), std::cmp::Reverse(v.id)));
        match best {
            Some(v) => {
                buses.take(v.id, from, to);
                t.bus = Some(BusOf::Own(v.id));
                t.bus_from = Source::Auto;
            }
            None => t.bus_problem = Some(none),
        }
    }
    if today {
        for t in out.iter_mut().filter(|t| !t.by_player && t.bus.is_none()) {
            let (from, to, wants) = (t.tour.from(), t.tour.to(), super::ownline::wanted(c, &t.tour));
            let rent = economy::rent_per_day(BusKind { size: wants.unwrap_or_default(), drive: Drive::Diesel }, &economy::rules(c.difficulty), c.price_index);
            match fill_of(c, date, &t.key()) {
                Some(Fill::Bus { id }) if usable(id) && buses.free(id, from, to) => {
                    buses.take(id, from, to);
                    t.bus = Some(BusOf::Own(id));
                    t.bus_from = Source::Dispatcher;
                }
                Some(Fill::Rental) => {
                    t.bus = Some(BusOf::Rental);
                    t.bus_from = Source::Dispatcher;
                }
                Some(Fill::Drop) => t.bus_from = Source::Dispatcher,
                _ if t.bus_problem.is_some_and(Problem::sudden) => {
                    let best = c.fleet.iter().filter(|v| usable(v.id) && buses.free(v.id, from, to) && super::ownline::line_allows(c, &t.tour.line, v)).max_by(|a, b| {
                        (size_fit(wants, a.kind.size), a.condition).partial_cmp(&(size_fit(wants, b.kind.size), b.condition)).unwrap_or(std::cmp::Ordering::Equal)
                    });
                    if let Some(v) = best {
                        buses.take(v.id, from, to);
                        t.bus = Some(BusOf::Own(v.id));
                        t.bus_from = Source::Central;
                    } else if c.cash >= rent {
                        t.bus = Some(BusOf::Rental);
                        t.bus_from = Source::Central;
                    }
                }
                _ => {}
            }
        }
    }

    // 2. the drivers: the roster's, in the order the duties begin
    let mut work = Work::default();
    let mut order: Vec<(usize, usize)> = out.iter().enumerate().filter(|(_, t)| !t.by_player).flat_map(|(i, t)| (0..t.duties.len()).map(move |k| (i, k))).collect();
    order.sort_by_key(|&(i, k)| out[i].duties[k].from);
    for &(i, k) in &order {
        let Some(who) = roster(c, wd, &out[i].tour.line, &out[i].tour.tour).and_then(|r| r.duties.get(k).copied().flatten()) else { continue };
        let bus = out[i].bus_of(c);
        let t = &out[i];
        let block = block_of(&t.tour, &t.duties[k], duty_key(&t.tour.line, &t.tour.tour, k));
        let d = &mut out[i].duties[k];
        match who {
            Who::Player if closing => d.problem = Some(Problem::Player),
            Who::Player | Who::Agency => {
                d.who = Some(who);
                d.from_ = Source::Roster;
            }
            Who::Staff(id) => {
                let check = match c.employee(id) {
                    None => Err(Problem::Away),
                    Some(e) => match unavailable(e, date, today) {
                        Some(p) => Err(p),
                        None => fits_bus(e, &bus, work.of(id), &block, today),
                    },
                };
                match check {
                    Ok(w) => {
                        work.push(id, block);
                        d.who = Some(who);
                        d.from_ = Source::Roster;
                        d.warn = w;
                    }
                    Err(p) => d.problem = Some(p),
                }
            }
        }
    }
    // the dispatcher's own for what the roster left free (tours with a bus, or reported live)
    let people: Vec<&Employee> = c.staff.iter().filter(|e| unavailable(e, date, today).is_none()).collect();
    let pick = |work: &Work, bus: &(BusKind, Option<String>), block: &Block| -> Option<(u32, Vec<Warn>)> {
        let mut best: Option<((bool, bool, i32, u32, i64), u32, Vec<Warn>)> = None;
        for e in &people {
            let Ok(w) = fits_bus(e, bus, work.of(e.id), block, today) else { continue };
            let worked = staff::work_minutes(work.of(e.id));
            let fills = worked > 0 && !w.iter().any(|x| matches!(x, Warn::Overtime(_)));
            let score = (staff::qualified(e, bus.0.size), fills, -worked, WEEK_DAYS.saturating_sub(e.week_days), (e.experience * 10.0) as i64);
            if best.as_ref().is_none_or(|b| score > b.0) {
                best = Some((score, e.id, w));
            }
        }
        best.map(|b| (b.1, b.2))
    };
    for &(i, k) in &order {
        let t = &out[i];
        if t.duties[k].who.is_some() || t.duties[k].problem.is_some() || (t.bus.is_none() && !t.live) {
            continue;
        }
        if !c.planning.auto {
            out[i].duties[k].problem = Some(Problem::Unassigned);
            continue;
        }
        let bus = t.bus_of(c);
        let block = block_of(&t.tour, &t.duties[k], duty_key(&t.tour.line, &t.tour.tour, k));
        match pick(&work, &bus, &block) {
            Some((id, w)) => {
                work.push(id, block);
                let d = &mut out[i].duties[k];
                d.who = Some(Who::Staff(id));
                d.from_ = Source::Auto;
                d.warn = w;
            }
            None => out[i].duties[k].problem = Some(Problem::Unassigned),
        }
    }
    // what is open today: the dispatcher's choice, else the central for what came suddenly
    if today {
        let mut agency = 0usize;
        for &(i, k) in &order {
            let t = &out[i];
            if t.duties[k].who.is_some() || (t.bus.is_none() && !t.live) {
                continue;
            }
            let bus = t.bus_of(c);
            let key = duty_key(&t.tour.line, &t.tour.tour, k);
            let block = block_of(&t.tour, &t.duties[k], key.clone());
            let problem = t.duties[k].problem;
            let (who, from_) = match fill_of(c, date, &key) {
                Some(Fill::Colleague { id }) => match c.employee(id).filter(|e| unavailable(e, date, today).is_none()).map(|e| fits_bus(e, &bus, work.of(id), &block, today)) {
                    Some(Ok(_)) => (Some(Who::Staff(id)), Source::Dispatcher),
                    _ => (None, Source::None),
                },
                Some(Fill::Agency) if agency < agency_max(c) => (Some(Who::Agency), Source::Dispatcher),
                Some(Fill::Drop) => (None, Source::Dispatcher),
                _ if problem.is_some_and(Problem::sudden) => match pick(&work, &bus, &block) {
                    Some((id, _)) => (Some(Who::Staff(id)), Source::Central),
                    None if agency < agency_max(c) => (Some(Who::Agency), Source::Central),
                    None => (None, Source::None),
                },
                _ => (None, Source::None),
            };
            if let Some(Who::Staff(id)) = who {
                work.push(id, block);
            }
            if who == Some(Who::Agency) {
                agency += 1;
            }
            let d = &mut out[i].duties[k];
            d.who = who;
            d.from_ = from_;
        }
        // late in the morning: the first duty of the day begins without them
        for dz in &dis {
            let Disruption::Late { employee, minutes } = *dz else { continue };
            let mine: Vec<(usize, usize)> = order.iter().copied().filter(|&(i, k)| out[i].duties[k].who == Some(Who::Staff(employee))).collect();
            let Some(&(i, k)) = mine.first() else { continue };
            let t = &out[i];
            let d = &t.duties[k];
            let until = d.from + minutes;
            if !t.tour.trips[d.start..d.end].iter().any(|x| x.dep < until) {
                continue;
            }
            let key = late_key(&t.tour.line, &t.tour.tour, k);
            let piece = Block { key: key.clone(), from: d.from, to: until.min(d.to), from_stop: t.tour.trips[d.start].from.clone(), to_stop: String::new() };
            let bus = t.bus_of(c);
            let (cover, from_) = match fill_of(c, date, &key) {
                Some(Fill::Colleague { id }) if c.employee(id).is_some_and(|e| id != employee && unavailable(e, date, today).is_none() && fits_bus(e, &bus, work.of(id), &piece, today).is_ok()) => (Some(id), Source::Dispatcher),
                Some(_) => (None, Source::Dispatcher),
                None => match pick(&work, &bus, &piece) {
                    Some((id, _)) if id != employee => (Some(id), Source::Central),
                    _ => (None, Source::None),
                },
            };
            if let Some(id) = cover {
                work.push(id, piece);
            }
            out[i].duties[k].late = Some(LatePiece { until, cover, from_ });
        }
    }
    DayPlan { date: date.to_string(), weekday: wd, today, tours: out, disruptions: dis }
}

// --- the player's own duties ---------------------------------------------------------------------

/// A duty planned for the player himself ("My duties"): its day, its tour and duty in that
/// day's plan, its line and times, and the bus it is planned with.
#[derive(Clone, Debug, PartialEq)]
pub struct MyDuty {
    pub date: String,
    /// The tour (an index into the day plan's tours) and its duty.
    pub tour: usize,
    pub duty: usize,
    /// The company line (its timetable name), the number shown, the tour's number.
    pub line: String,
    pub number: String,
    pub tour_no: String,
    pub from: i32,
    pub to: i32,
    pub bus: Option<BusOf>,
    /// The trips with passengers it drives.
    pub trips: usize,
}

/// The player's duties in `plans` (the days of the planning, in any order), the next first:
/// those of the company's day that are over (`now`, minutes of `today`) or that he drove
/// already left out.
pub fn my_duties(plans: &[DayPlan], today: &str, now: i32) -> Vec<MyDuty> {
    let mut out: Vec<MyDuty> = Vec::new();
    for p in plans {
        // (days before the company's are done)
        if dates::between(today, &p.date) < 0 {
            continue;
        }
        for (ti, k) in p.duties_of(Who::Player) {
            let t = &p.tours[ti];
            let d = &t.duties[k];
            if t.by_player || (p.date == today && d.to <= now) {
                continue;
            }
            let trips = t.tour.trips[d.start..d.end].iter().filter(|x| x.counts()).count();
            out.push(MyDuty { date: p.date.clone(), tour: ti, duty: k, line: t.tour.line.clone(), number: t.tour.number.clone(), tour_no: t.tour.tour.clone(), from: d.from, to: d.to, bus: t.bus, trips });
        }
    }
    out.sort_by_key(|d| (dates::parse(&d.date).unwrap_or(0), d.from));
    out
}

/// A stand-in for an agency driver in the day's model (average skills).
pub fn agency_driver() -> Employee {
    Employee {
        id: AGENCY,
        name: "Agency driver".into(),
        age: 40,
        experience: 40.0,
        licence: Licence::D,
        wage: 0,
        reliability: 0.8,
        skills: Skills { driving: 60.0, punctuality: 60.0, service: 50.0 },
        satisfaction: 60.0,
        hired: String::new(),
        notice_until: None,
        training_until: None,
        resigned: false,
        sick_until: None,
        holiday_until: None,
        holiday_left: 0,
        week_days: 0,
        last_end: None,
        days_worked: 0,
        // (the agency sends drivers for what they are sent to)
        endorsements: super::licences::Endorsement::ALL.to_vec(),
        types: Vec::new(),
    }
}

/// The morning of a closing day booked: the buses that did not start (towed, in the
/// workshop for the day), the rental buses rented (they go back after the day), the agency
/// drivers paid. Returns the plan the close runs, and what to tell.
pub fn settle_morning(c: &mut Company, dp: &DayPlan) -> (Plan, Vec<Note>) {
    let mut plan = dp.to_plan();
    let mut notes = Vec::new();
    let date = c.date.clone();
    for d in &dp.disruptions {
        let Disruption::Breakdown { vehicle, cost } = *d else { continue };
        let Some(v) = c.fleet.iter_mut().find(|v| v.id == vehicle) else { continue };
        v.breakdowns += 1;
        if !v.in_workshop(&date) {
            v.workshop_until = Some(date.clone());
        }
        let (number, text) = (v.number.clone(), format!("{} {}", v.number, v.name));
        c.book(BookingKind::Repair, -cost, text, false);
        notes.push(Note::Breakdown { number, until: date.clone(), cost });
    }
    for (ti, t) in dp.tours.iter().enumerate() {
        if t.bus != Some(BusOf::Rental) {
            continue;
        }
        let wants = super::ownline::wanted(c, &t.tour);
        let model = c.fleet.iter().find(|v| wants.is_none_or(|w| w == v.kind.size) && !v.bus.is_empty()).or(c.fleet.iter().find(|v| !v.bus.is_empty()));
        let bus = match model {
            Some(v) => MarketBus { file: v.bus.clone(), name: v.name.clone(), kind: v.kind, ..Default::default() },
            None => MarketBus { name: "Rental bus".into(), kind: BusKind { size: wants.unwrap_or_default(), drive: Drive::Diesel }, ..Default::default() },
        };
        if let Ok(id) = market::rent(c, &bus, 1, "") {
            plan.tours[ti].bus = Some(id);
        }
    }
    let hour = AGENCY_HOUR as f64 * c.price_index;
    for t in &dp.tours {
        for d in t.duties.iter().filter(|d| d.who == Some(Who::Agency)) {
            let cost = ((d.to - d.from).max(0) as f64 / 60.0 * hour).round() as Cents;
            c.book(BookingKind::Wages, -cost, format!("Agency driver, line {} tour {}", t.tour.number, t.tour.tour), false);
        }
    }
    (plan, notes)
}

// --- the game's side -------------------------------------------------------------------------

/// A company tour as the game runs it while the player drives: with this bus of the fleet
/// (its file, paint, number and plate), or not at all (`dropped`: no bus, no driver).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct LiveTour {
    /// The timetable's line (`.ttl` name), the number shown, and the tour.
    pub line: String,
    pub number: String,
    pub tour: String,
    #[serde(default)]
    pub vehicle: Option<u32>,
    #[serde(default)]
    pub bus: String,
    #[serde(default)]
    pub paint: String,
    #[serde(default)]
    pub fleet_number: String,
    #[serde(default)]
    pub plate: String,
    #[serde(default)]
    pub dropped: bool,
    /// The depot file the bus carries on this line (empty: the plan's `depot`).
    #[serde(default)]
    pub hof: String,
}

/// The company's day for the game (`companies/<id>.liveplan.json`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct LivePlan {
    pub company: String,
    pub map: String,
    pub date: String,
    /// The depot file its buses carry (their destination lists) and the economy (the
    /// passengers a trip carries).
    #[serde(default)]
    pub depot: String,
    #[serde(default)]
    pub difficulty: Difficulty,
    pub tours: Vec<LiveTour>,
}

impl LivePlan {
    pub fn tour(&self, line: &str, tour: &str) -> Option<&LiveTour> {
        self.tours.iter().find(|t| same(&t.line, line) && same(&t.tour, tour))
    }

    /// What the game reports of a company trip its AI ran to the end: its kilometres, the
    /// passengers the model gives a trip of that length and hour (a depot run carries none),
    /// and how late it ended (s). None: not a tour the company runs.
    pub fn trip_event(&self, line: &str, tour: &str, dep: i32, km: f64, stops: usize, delay: f64) -> Option<LiveEvent> {
        let t = self.tour(line, tour).filter(|t| !t.dropped)?;
        let km = if km.is_finite() { km.clamp(0.0, 200.0) } else { 0.0 };
        let passengers = if stops >= 3 { economy::passengers_for(km, dep, &economy::rules(self.difficulty)).round().max(0.0) as u32 } else { 0 };
        Some(LiveEvent::Trip { line: t.line.clone(), tour: t.tour.clone(), vehicle: t.vehicle, km, passengers, delay, completed: true, dep: Some(dep) })
    }

    /// Whether it is the plan of the map in `map_dir` (a map's folder) on `date`.
    pub fn is_for(&self, map_dir: &Path, date: &str) -> bool {
        let folder = |p: &str| p.replace('\\', "/").trim_end_matches("/global.cfg").rsplit('/').next().unwrap_or("").to_lowercase();
        let here = map_dir.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
        !here.is_empty() && folder(&self.map) == here && self.date == date
    }
}

/// The plan of the company's day for the game: each tour with its bus, and those dropped.
pub fn live_plan(c: &Company, dp: &DayPlan) -> LivePlan {
    let tours = dp
        .tours
        .iter()
        // (a line not in service runs nothing, not even in the game)
        .filter(|t| !t.by_player && !t.tour.unplanned)
        .map(|t| {
            let hof = c.lines.iter().find(|l| l.name.eq_ignore_ascii_case(&t.tour.line)).map(|l| l.hof.trim().to_string()).unwrap_or_default();
            let mut lt = LiveTour { line: t.tour.line.clone(), number: t.tour.number.clone(), tour: t.tour.tour.clone(), hof, ..Default::default() };
            match t.bus {
                Some(BusOf::Own(id)) => {
                    if let Some(v) = c.vehicle(id) {
                        lt.vehicle = Some(id);
                        lt.bus = v.bus.clone();
                        lt.paint = v.house_livery.clone().filter(|h| !h.trim().is_empty()).unwrap_or_else(|| v.livery.clone());
                        lt.fleet_number = v.number.clone();
                        lt.plate = v.plate.clone();
                    }
                }
                Some(BusOf::Rental) => {}
                None => lt.dropped = true,
            }
            lt.dropped |= t.bus.is_some() && t.duties.iter().all(|d| d.who.is_none());
            lt
        })
        .collect();
    LivePlan { company: c.id.clone(), map: c.map.clone(), date: c.date.clone(), depot: c.depot.clone(), difficulty: c.difficulty, tours }
}

pub fn live_plan_file(data: &Path, id: &str) -> PathBuf {
    super::store::dir(data).join(format!("{id}.liveplan.json"))
}

/// Written whole (the game reads it when its timetable is loaded and at midnight).
pub fn save_live_plan(data: &Path, p: &LivePlan) -> anyhow::Result<()> {
    let d = super::store::dir(data);
    std::fs::create_dir_all(&d)?;
    let path = live_plan_file(data, &p.company);
    let tmp = d.join(format!("{}.liveplan.json.tmp", p.company));
    std::fs::write(&tmp, serde_json::to_string_pretty(p)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Every company's plan for the game (those a company file still stands beside).
pub fn live_plans(data: &Path) -> Vec<LivePlan> {
    let Ok(rd) = std::fs::read_dir(super::store::dir(data)) else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            let id = n.strip_suffix(".liveplan.json")?.to_string();
            super::store::path_of(data, &id).exists().then_some(())?;
            serde_json::from_str::<LivePlan>(&std::fs::read_to_string(e.path()).ok()?).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::day::{close_day, record_live, LiveEvent};
    use super::super::market::Payment;
    use super::super::network::tests::trip;
    use super::super::network::PlannedTrip;
    use super::super::staff::{applicants, hire};
    use super::super::{found, Founding};
    use super::*;
    use crate::company::model::CompanyLine;

    fn tour(no: &str, from: i32, n: i32) -> TourOfDay {
        TourOfDay { line: "Linie5".into(), number: "5".into(), tour: no.into(), ai_group: String::new(), trips: (0..n).map(|k| trip(from + k * 60, from + 50 + k * 60, 12)).collect(), unplanned: false }
    }

    fn company(buses: usize, people: usize) -> Company {
        // (Easy: the fewest late mornings and breakdowns; the tests that want them make them)
        let mut c = found(&Founding { name: "Plan".into(), difficulty: Difficulty::Easy, date: "2024-03-04".into(), map: "maps/Grundorf/global.cfg".into(), ..Default::default() }, "Luc");
        // (in service from the start, and the dispatcher's own filling what the roster leaves
        // free - as "Fill the roster" does when asked)
        c.lines.push(CompanyLine { name: "Linie5".into(), number: "5".into(), numbers: vec!["5".into()], added: c.date.clone(), service_from: Some(0), ..Default::default() });
        c.planning.auto = true;
        let bus = MarketBus { file: "Vehicles/Citaro/Citaro.bus".into(), name: "Citaro".into(), ..Default::default() };
        c.cash += 5_000_000_00;
        for _ in 0..buses {
            market::buy_new(&mut c, &bus, Payment::Cash, "").unwrap();
        }
        let id = c.id.clone();
        while c.staff.len() < people {
            for a in applicants(&c) {
                if c.staff.len() < people && a.licence == Licence::D {
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
            e.satisfaction = 80.0;
            e.reliability = 1.0;
            e.experience = 50.0;
        }
        c
    }

    /// A company whose morning brings nothing (its id chosen so).
    fn quiet(buses: usize, people: usize) -> Company {
        let mut c = company(buses, people);
        let base = c.id.clone();
        for n in 0.. {
            c.id = format!("{base}{n}");
            if disruptions(&c, &c.date).is_empty() {
                return c;
            }
        }
        unreachable!()
    }

    #[test]
    fn a_line_runs_with_its_own_buses_only() {
        use crate::service::{LineVehicles, VehicleClass};
        let mut c = quiet(1, 2);
        let wd = weekday_of(&c.date);
        // the line asks for minibuses: the solo bus is none of them, and the planning says so
        c.lines[0].plan = Some(super::super::ownline::OwnPlan { vehicles: LineVehicles { classes: vec![VehicleClass::Minibus], ..Default::default() }, ..Default::default() });
        let tours = vec![tour("1", 6 * 60, 3)];
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        assert_eq!((p.tours[0].bus, p.tours[0].bus_problem), (None, Some(Problem::NoLineBus)));
        // fixed in the roster all the same: not one of the line's
        let solo = c.fleet[0].id;
        set_bus(&mut c, wd, "Linie5", "1", Some(solo));
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        assert_eq!((p.tours[0].bus, p.tours[0].bus_problem), (None, Some(Problem::NotLineBus)));
        set_bus(&mut c, wd, "Linie5", "1", None);
        // a minibus bought: the dispatcher takes it
        let mini = MarketBus { file: "Vehicles/Sprinter/Sprinter.bus".into(), name: "Mercedes-Benz Sprinter".into(), kind: BusKind { size: BusSize::Midi, drive: Drive::Diesel }, ..Default::default() };
        market::buy_new(&mut c, &mini, Payment::Cash, "").unwrap();
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        assert_eq!(p.tours[0].bus, Some(BusOf::Own(c.fleet[1].id)));
        // "Fill the roster" gives it the bus, but nobody had the type training for the new model
        let mut f = c.clone();
        assert_eq!(fill_day(&mut f, &c.date.clone(), tours.clone()), 1);
        let p = day_plan(&f, &f.date, tours.clone(), &[], &[], false);
        assert_eq!(p.tours[0].duties[0].problem, Some(Problem::Unassigned));
        // given by hand all the same: the planning says why not
        let e1 = c.staff[0].id;
        set_driver(&mut f, wd, "Linie5", "1", 0, Some(Who::Staff(e1)));
        let p = day_plan(&f, &f.date, tours.clone(), &[], &[], false);
        assert_eq!(p.tours[0].duties[0].problem, Some(Problem::NoTypeTraining));
        // trained: and it fills both, the bus and the driver
        let key = super::super::licences::type_key("Vehicles/Sprinter/Sprinter.bus");
        for e in c.staff.iter_mut() {
            e.types.push(key.clone());
        }
        assert_eq!(fill_day(&mut c.clone(), &c.date.clone(), tours), 2);
        // a line that asks for nothing takes any bus
        c.lines[0].plan = None;
        assert!(super::super::ownline::line_allows(&c, "Linie5", &c.fleet[0]));
    }

    #[test]
    fn a_lines_own_depot_file_and_title_go_with_it() {
        let mut c = quiet(1, 1);
        let p = day_plan(&c, &c.date, vec![tour("1", 6 * 60, 3)], &[], &[], false);
        // the company's depot file, unless the line has its own
        assert_eq!(live_plan(&c, &p).tours[0].hof, "");
        assert_eq!(c.lines[0].hof_or("Grundorf"), "Grundorf");
        c.lines[0].hof = "Spandau".into();
        assert_eq!(live_plan(&c, &p).tours[0].hof, "Spandau");
        assert_eq!(c.lines[0].hof_or("Grundorf"), "Spandau");
        // what it is called in public: its title, else where it goes
        c.lines[0].caption = "Markt – Bahnhof".into();
        assert_eq!(c.lines[0].public_name(), "Markt – Bahnhof");
        c.lines[0].title = " Shuttleverkehr Altenfeld - Wurzbach ".into();
        assert_eq!(c.lines[0].public_name(), "Shuttleverkehr Altenfeld - Wurzbach");
        // (an older file has neither)
        let old: CompanyLine = serde_json::from_str(r#"{"name":"5","number":"5","own":false,"added":"2024-03-04"}"#).unwrap();
        assert!(old.title.is_empty() && old.hof.is_empty());
    }

    #[test]
    fn my_duties_come_next_first() {
        let mut c = quiet(2, 1);
        let wd = weekday_of(&c.date);
        set_driver(&mut c, wd, "Linie5", "2", 0, Some(Who::Player));
        set_driver(&mut c, wd, "Linie5", "1", 0, Some(Who::Player));
        let tours = vec![tour("1", 6 * 60, 3), tour("2", 14 * 60, 3)];
        let today = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        // (the roster is the weekday's: next week's same day has them too)
        let next_week = dates::add(&c.date, 7);
        let later = day_plan(&c, &next_week, tours.clone(), &[], &[], false);
        let mine = my_duties(&[later.clone(), today.clone()], &c.date, 0);
        assert_eq!(mine.len(), 4);
        assert_eq!((mine[0].date.as_str(), mine[0].tour_no.as_str(), mine[0].from, mine[0].trips), (c.date.as_str(), "1", 6 * 60, 3));
        assert_eq!((mine[1].tour_no.as_str(), mine[2].date.as_str()), ("2", next_week.as_str()));
        assert!(mine[0].bus.is_some() && mine[0].number == "5");
        // at noon the morning's duty is over
        let mine = my_duties(&[today.clone()], &c.date, 12 * 60);
        assert_eq!(mine.iter().map(|d| d.tour_no.as_str()).collect::<Vec<_>>(), vec!["2"]);
        // a tour he drove already is no duty to drive
        let driven = day_plan(&c, &c.date, tours.clone(), &[("5".into(), "2".into())], &[], false);
        assert!(my_duties(&[driven], &c.date, 12 * 60).is_empty());
        // days before the company's are done; nobody's duties are his
        assert!(my_duties(&[today], &dates::add(&c.date, 1), 0).is_empty());
        let nobody = day_plan(&quiet(2, 1), &c.date, tours, &[], &[], false);
        assert!(my_duties(&[nobody], &c.date, 0).is_empty());
    }

    #[test]
    fn a_tour_is_cut_into_duties_of_at_most_nine_and_a_half_hours() {
        // 5:00 to 23:00, a cut after a layover near the middle; 9½ h at most a duty
        let c = quiet(1, 3);
        let p = day_plan(&c, &c.date, vec![tour("1", 5 * 60, 18)], &[], &[], false);
        let d = &p.tours[0].duties;
        assert_eq!(d.len(), 2);
        assert!(d.iter().all(|x| x.to - x.from <= staff::DUTY_MAX));
        assert_eq!((d[0].start, d[1].end), (0, 18));
        assert_eq!(d[0].end, d[1].start);
        // a cut only before a trip with passengers, after the bus stood: a depot run at the
        // start goes with the first duty
        let mut trips: Vec<PlannedTrip> = vec![trip(280, 295, 2)];
        trips.extend((0..18).map(|k| trip(300 + k * 60, 350 + k * 60, 12)));
        let t = TourOfDay { trips, ..tour("2", 0, 0) };
        let p = day_plan(&c, &c.date, vec![t], &[], &[], false);
        assert_eq!(p.tours[0].duties[0].start, 0);
        assert!(p.tours[0].tour.trips[p.tours[0].duties[1].start].counts());
    }

    #[test]
    fn the_roster_comes_first_and_the_dispatcher_fills_the_rest_by_the_rules() {
        let mut c = quiet(2, 3);
        let wd = weekday_of(&c.date);
        let (b1, b2) = (c.fleet[0].id, c.fleet[1].id);
        let (e1, e2) = (c.staff[0].id, c.staff[1].id);
        // the second bus and the second driver fixed for tour 1
        set_bus(&mut c, wd, "Linie5", "1", Some(b2));
        set_driver(&mut c, wd, "Linie5", "1", 0, Some(Who::Staff(e2)));
        let tours = vec![tour("1", 6 * 60, 6), tour("2", 6 * 60 + 30, 6)];
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        let t1 = p.tours.iter().find(|t| t.tour.tour == "1").unwrap();
        assert_eq!((t1.bus, t1.bus_from), (Some(BusOf::Own(b2)), Source::Roster));
        assert_eq!((t1.duties[0].who, t1.duties[0].from_), (Some(Who::Staff(e2)), Source::Roster));
        let t2 = p.tours.iter().find(|t| t.tour.tour == "2").unwrap();
        assert_eq!((t2.bus, t2.bus_from), (Some(BusOf::Own(b1)), Source::Auto));
        assert!(matches!(t2.duties[0].who, Some(Who::Staff(id)) if id != e2));
        assert_eq!(p.counts(), (2, 2, 0, 0));
        // the same driver on two duties at the same time: the second clashes
        set_driver(&mut c, wd, "Linie5", "2", 0, Some(Who::Staff(e2)));
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        let t2 = p.tours.iter().find(|t| t.tour.tour == "2").unwrap();
        assert_eq!(t2.duties[0].problem, Some(Problem::Conflict));
        // without the dispatcher's own, it stays open
        c.planning.auto = false;
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        assert!(p.tours.iter().find(|t| t.tour.tour == "2").unwrap().duties[0].open());
        c.planning.auto = true;
        // the rest between two duties of one driver: 20 minutes at least
        set_driver(&mut c, wd, "Linie5", "2", 0, None);
        let short = vec![tour("1", 6 * 60, 3), tour("2", 6 * 60 + 3 * 60 + 5, 3)];
        set_driver(&mut c, wd, "Linie5", "2", 0, Some(Who::Staff(e2)));
        let p = day_plan(&c, &c.date, short, &[], &[], false);
        assert_eq!(p.tours[1].duties[0].problem, Some(Problem::Conflict));
        // a D1 licence drives no solo bus
        c.staff[0].licence = Licence::D1;
        set_driver(&mut c, wd, "Linie5", "2", 0, Some(Who::Staff(e1)));
        let p = day_plan(&c, &c.date, vec![tour("2", 12 * 60, 3)], &[], &[], false);
        assert_eq!(p.tours[0].duties[0].problem, Some(Problem::Licence));
        // the night: yesterday's last duty ended at 23:00 - not before 10:00 today
        c.staff[0].licence = Licence::D;
        c.staff[0].last_end = Some(23 * 60);
        let p = day_plan(&c, &c.date, vec![tour("2", 6 * 60, 3)], &[], &[], false);
        assert_eq!(p.tours[0].duties[0].problem, Some(Problem::Conflict));
        // on holiday: open, and not sudden (the central leaves it)
        c.staff[0].last_end = None;
        c.staff[0].holiday_until = Some(c.date.clone());
        c.staff[1].holiday_until = Some(c.date.clone());
        c.staff[2].holiday_until = Some(c.date.clone());
        let p = day_plan(&c, &c.date, vec![tour("2", 12 * 60, 3)], &[], &[], false);
        assert_eq!(p.tours[0].duties[0].problem, Some(Problem::Holiday));
        assert!(p.tours[0].duties[0].open());
    }

    #[test]
    fn a_driver_without_the_endorsement_is_not_planned_on_the_bus() {
        use super::super::licences::{type_key, Endorsement};
        let mut c = quiet(1, 2);
        let wd = weekday_of(&c.date);
        // the company's bus an articulated one: nobody has the endorsement
        c.fleet[0].kind = BusKind { size: BusSize::Articulated, drive: Drive::Diesel };
        for e in c.staff.iter_mut() {
            e.endorsements.clear();
            e.types = vec![type_key(&c.fleet[0].bus)];
        }
        let e1 = c.staff[0].id;
        let tours = vec![tour("1", 6 * 60, 3)];
        let b1 = c.fleet[0].id;
        set_bus(&mut c, wd, "Linie5", "1", Some(b1));
        set_driver(&mut c, wd, "Linie5", "1", 0, Some(Who::Staff(e1)));
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        assert_eq!(p.tours[0].duties[0].problem, Some(Problem::NoEndorsement));
        // the dispatcher gives it to nobody, and says why
        set_driver(&mut c, wd, "Linie5", "1", 0, None);
        assert_eq!(fill_day(&mut c.clone(), &c.date.clone(), tours.clone()), 0);
        let f = fill_day_told(&mut c.clone(), &c.date.clone(), tours.clone());
        assert_eq!((f.duties, f.unqualified, f.busy), (0, 1, 0));
        // with the endorsement: planned
        c.staff[0].endorsements.push(Endorsement::Articulated);
        assert_eq!(fill_day(&mut c.clone(), &c.date.clone(), tours), 1);
    }

    #[test]
    fn filling_the_roster_keeps_the_rules_and_clearing_empties_it() {
        let mut c = quiet(2, 2);
        let wd = weekday_of(&c.date);
        // a long tour (two duties) and a short one at the same time: two buses, but only two
        // drivers - one of them may not drive two duties of over ten hours together
        let tours = vec![tour("1", 5 * 60, 18), tour("2", 6 * 60, 4)];
        let today = c.date.clone();
        let n = fill_day(&mut c, &today, tours.clone());
        assert!(n >= 3);
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        assert!(p.tours.iter().all(|t| t.bus_from == Source::Roster));
        // every duty that has a driver is the roster's, and nobody works over ten hours
        for e in &c.staff {
            let mine = p.duties_of(Who::Staff(e.id));
            let minutes: i32 = mine.iter().map(|&(i, k)| p.tours[i].duties[k].to - p.tours[i].duties[k].from).sum();
            assert!(minutes <= staff::DAY_MAX, "{minutes}");
        }
        assert!(p.tours.iter().flat_map(|t| t.duties.iter()).filter(|d| d.who.is_some()).all(|d| d.from_ == Source::Roster));
        // the roster repeats: next week's Monday is the same
        let next = dates::add(&c.date, 7);
        let mut later = c.clone();
        later.date = next.clone();
        let q = day_plan(&later, &next, tours.clone(), &[], &[], false);
        assert_eq!(q.tours.iter().map(|t| t.bus).collect::<Vec<_>>(), p.tours.iter().map(|t| t.bus).collect::<Vec<_>>());
        // filled again: nothing more to give, and what is open said
        let again = fill_day_told(&mut c.clone(), &today, tours.clone());
        assert_eq!((again.duties, again.buses), (0, 0));
        let open = p.tours.iter().flat_map(|t| t.duties.iter()).filter(|d| d.who.is_none()).count();
        assert_eq!(again.unqualified + again.busy, open);
        assert_eq!(again.no_bus, p.tours.iter().filter(|t| t.bus.is_none()).count());
        // copied to Tuesday, cleared on Monday: each says what it did
        let held = rostered(&c, wd);
        assert!(held.buses >= 1 && held.duties >= 1);
        assert_eq!(copy_day(&mut c, wd, &[wd, (wd + 1) % 7]), vec![(wd + 1) % 7]);
        assert_eq!(clear_day(&mut c, wd), held);
        assert!(c.planning.week.iter().all(|r| r.weekday == (wd + 1) % 7));
        let p = day_plan(&c, &c.date, tours, &[], &[], false);
        assert!(p.tours.iter().all(|t| t.bus_from != Source::Roster));
        // an empty day is not repeated: the others keep theirs
        assert!(copy_day(&mut c, wd, &[(wd + 1) % 7]).is_empty());
        assert_eq!(rostered(&c, (wd + 1) % 7), held);
        assert!(clear_day(&mut c, wd).is_empty());
    }

    #[test]
    fn what_falls_out_is_filled_or_dropped() {
        let mut c = quiet(2, 3);
        let wd = weekday_of(&c.date);
        let (b1, e1) = (c.fleet[0].id, c.staff[0].id);
        set_bus(&mut c, wd, "Linie5", "1", Some(b1));
        set_driver(&mut c, wd, "Linie5", "1", 0, Some(Who::Staff(e1)));
        let tours = vec![tour("1", 6 * 60, 4)];
        // ill: the central takes a spare colleague
        c.staff[0].sick_until = Some(c.date.clone());
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        let d = &p.tours[0].duties[0];
        assert_eq!((d.problem, d.from_), (Some(Problem::Sick), Source::Central));
        assert!(matches!(d.who, Some(Who::Staff(id)) if id != e1));
        assert_eq!(p.open_rows().len(), 1);
        assert!(p.open_rows()[0].filled);
        // the dispatcher drops it instead
        set_fill(&mut c, &duty_key("Linie5", "1", 0), Some(Fill::Drop));
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        assert!(p.tours[0].duties[0].open());
        // or takes an agency driver
        set_fill(&mut c, &duty_key("Linie5", "1", 0), Some(Fill::Agency));
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        assert_eq!(p.tours[0].duties[0].who, Some(Who::Agency));
        // nobody spare: an agency driver by the central
        set_fill(&mut c, &duty_key("Linie5", "1", 0), None);
        for e in c.staff.iter_mut().skip(1) {
            e.sick_until = Some(e.hired.clone());
            e.holiday_until = Some(dates::add(&e.hired, 30));
        }
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        assert_eq!((p.tours[0].duties[0].who, p.tours[0].duties[0].from_), (Some(Who::Agency), Source::Central));
        // the choices are for the day: tomorrow they are gone
        set_fill(&mut c, &duty_key("Linie5", "1", 0), Some(Fill::Drop));
        let mut t = c.clone();
        t.date = dates::add(&c.date, 1);
        assert!(fill_of(&t, &t.date, &duty_key("Linie5", "1", 0)).is_none());
        // a bus that did not start: another free bus, else a rental bus
        let mut c = quiet(1, 2);
        let b = c.fleet[0].id;
        set_bus(&mut c, wd, "Linie5", "1", Some(b));
        c.fleet[0].workshop_until = Some(c.date.clone());
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        assert_eq!(p.tours[0].bus_problem, Some(Problem::Workshop));
        // (the workshop is known the evening before: not the central's)
        assert_eq!(p.tours[0].bus, None);
        set_fill(&mut c, &tour_key("Linie5", "1"), Some(Fill::Rental));
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        assert_eq!(p.tours[0].bus, Some(BusOf::Rental));
        // the close rents it for the day and gives it back
        let n = c.fleet.len();
        let r = close_day(&mut c, tours.clone(), &[]);
        assert_eq!(r.covered, 1);
        assert_eq!(c.fleet.len(), n);
        assert!(c.month(&dates::month_of(&r.date)).get(BookingKind::Rent) < 0);
    }

    #[test]
    fn a_late_driver_and_a_morning_breakdown() {
        // a company whose morning has a late driver and a bus that does not start
        let mut c = company(3, 4);
        for e in c.staff.iter_mut() {
            e.satisfaction = 10.0;
            e.reliability = 0.0;
        }
        for v in c.fleet.iter_mut() {
            v.condition = 21.0;
        }
        c.difficulty = Difficulty::Hard;
        let base = c.id.clone();
        let mut found_one = false;
        for n in 0..4000 {
            c.id = format!("{base}{n}");
            let d = disruptions(&c, &c.date);
            if d.iter().any(|x| matches!(x, Disruption::Late { .. })) && d.iter().any(|x| matches!(x, Disruption::Breakdown { .. })) {
                found_one = true;
                break;
            }
        }
        assert!(found_one);
        let dis = disruptions(&c, &c.date);
        // the same morning again
        assert_eq!(dis, disruptions(&c.clone(), &c.date));
        let Some(Disruption::Late { employee, minutes }) = dis.iter().find(|x| matches!(x, Disruption::Late { .. })).copied() else { unreachable!() };
        let Some(Disruption::Breakdown { vehicle, .. }) = dis.iter().find(|x| matches!(x, Disruption::Breakdown { .. })).copied() else { unreachable!() };
        let wd = weekday_of(&c.date);
        set_bus(&mut c, wd, "Linie5", "1", Some(vehicle));
        set_driver(&mut c, wd, "Linie5", "1", 0, Some(Who::Staff(employee)));
        let tours = vec![tour("1", 6 * 60, 6)];
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        let t = &p.tours[0];
        // the broken bus: the central's free one instead
        assert_eq!(t.bus_problem, Some(Problem::Breakdown));
        assert!(matches!(t.bus, Some(BusOf::Own(id)) if id != vehicle) || t.bus == Some(BusOf::Rental));
        assert_eq!(t.bus_from, Source::Central);
        // the late driver still drives, from `minutes` on; the piece before is covered or not
        let d = &t.duties[0];
        assert_eq!(d.who, Some(Who::Staff(employee)));
        let late = d.late.unwrap();
        assert_eq!(late.until, 6 * 60 + minutes);
        // nobody covers it: its trips before are dropped at the close
        set_fill(&mut c, &late_key("Linie5", "1", 0), Some(Fill::Drop));
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        assert_eq!(p.tours[0].duties[0].late.unwrap().cover, None);
        assert_eq!(p.to_plan().tours[0].duties[0].dropped_before, Some(6 * 60 + minutes));
        let before = p.tours[0].tour.trips.iter().filter(|x| x.dep < 6 * 60 + minutes).count() as u32;
        let r = close_day(&mut c, tours, &[]);
        assert!(r.dropped >= before);
        assert!(r.notes.iter().any(|n| matches!(n, Note::Breakdown { .. })));
    }

    #[test]
    fn the_players_duty_is_his_or_open_at_the_close() {
        let mut c = quiet(1, 2);
        let wd = weekday_of(&c.date);
        set_driver(&mut c, wd, "Linie5", "1", 0, Some(Who::Player));
        let tours = vec![tour("1", 6 * 60, 4)];
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        assert_eq!(p.tours[0].duties[0].who, Some(Who::Player));
        assert!(p.to_plan().tours[0].covered());
        // closing without his trips: the central's spare colleague drives it
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], true);
        let d = &p.tours[0].duties[0];
        assert_eq!(d.problem, Some(Problem::Player));
        assert!(matches!(d.who, Some(Who::Staff(_))));
        // driven: nothing to fill
        let p = day_plan(&c, &c.date, tours, &[("5".into(), "1".into())], &[], true);
        assert!(p.tours[0].by_player && p.tours[0].covered());
    }

    #[test]
    fn the_live_company_is_planned_recorded_and_settled_for_the_rest() {
        let mut c = quiet(2, 4);
        let tours = vec![tour("1", 6 * 60, 6), tour("2", 6 * 60 + 30, 6)];
        // the game's plan: both tours with the fleet's buses, the house livery if there is one
        c.fleet[0].house_livery = Some("House".into());
        let p = day_plan(&c, &c.date, tours.clone(), &[], &[], false);
        let lp = live_plan(&c, &p);
        assert_eq!(lp.tours.len(), 2);
        assert!(lp.tours.iter().all(|t| t.vehicle.is_some() && !t.dropped && t.bus.ends_with(".bus")));
        assert!(lp.tours.iter().any(|t| t.paint == "House"));
        assert!(lp.is_for(Path::new("C:/OMSI 2/maps/Grundorf"), &c.date));
        assert!(!lp.is_for(Path::new("C:/OMSI 2/maps/Ahlheim"), &c.date));
        assert!(lp.tour("Linie5", "2").is_some());
        // written for the game and read back
        let data = std::env::temp_dir().join(format!("openomsi-liveplan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        super::super::store::save(&data, &c).unwrap();
        save_live_plan(&data, &lp).unwrap();
        assert_eq!(live_plans(&data), vec![lp.clone()]);
        // the game drove the first three trips of tour 1 and reported them
        let v = lp.tour("Linie5", "1").unwrap().vehicle;
        for k in 0..3 {
            let ev = lp.trip_event("linie5", "1", 6 * 60 + k * 60, 15.0, 12, 40.0).unwrap();
            assert!(matches!(&ev, LiveEvent::Trip { vehicle, passengers, dep, .. } if *vehicle == v && *passengers > 0 && *dep == Some(6 * 60 + k * 60)));
            super::super::store::append_live(&data, &c.id, &ev).unwrap();
        }
        // (a tour not the company's is none of its business)
        assert!(lp.trip_event("Linie7", "1", 400, 10.0, 12, 0.0).is_none());
        super::super::store::take_live(&data, &mut c);
        assert_eq!(c.live.len(), 3);
        let _ = std::fs::remove_dir_all(&data);
        let km_before = c.vehicle(v.unwrap()).unwrap().km;
        let r = close_day(&mut c, tours.clone(), &[]);
        // measured: the three live trips; the model the other nine - nothing dropped
        assert!(c.ledger.iter().filter(|b| b.measured && b.kind == BookingKind::Fares).count() >= 3);
        assert_eq!(r.trips, 12);
        assert_eq!(r.dropped, 0);
        assert_eq!(r.covered, 2);
        // the bus ran the live kilometres and the modelled ones
        assert!(c.vehicle(v.unwrap()).unwrap().km >= km_before + 45.0 + 3.0 * 15.0 - 1.0);
        // an event without its departure takes the whole tour, as before
        record_live(&mut c, LiveEvent::Trip { line: "5".into(), tour: "2".into(), vehicle: None, km: 8.0, passengers: 20, delay: 30.0, completed: true, dep: None });
        let r = close_day(&mut c, tours, &[]);
        assert_eq!(r.covered, 2);
        assert_eq!(r.trips, 6 + 6);
    }
}
