//! The company's clock: the company has a "now" - its date and the minute of that day - and
//! time is simulated step by step instead of a day being closed at once (Luc: simulate to the
//! next day, or two or three, with all that happens on the way, and also by hours and
//! minutes).
//!
//! Simulating moves the clock through an event timeline. A day's own events are written when
//! the clock first enters it (`script`, from the day's plan and its draws): the night's notes
//! told in the morning (the sick calls, a bus going to its service), the morning's
//! disruptions at their times, the tours leaving and coming back, the buses breaking down on
//! the way (`Broken`), building work and workshop jobs finishing in the afternoon of their
//! last day, the last day of someone leaving. Besides them come the tenders' auctions
//! (opening, the rivals' bids as they come, closing: `concessions`), and at midnight the day's
//! close (`day::close_day`, the month's bookings on a month's last day). Each event is
//! applied when its time comes and told in the feed; the important ones wait for the player's
//! decision (`Ask`) - or the dispatcher's, when the player left it to him or simulates
//! quickly.
//!
//! The day's money is still booked at its close (the modelled day is settled as a whole); a
//! breakdown's decision counts there: a rental bus ordered saves the trips after it arrives
//! (`breakdowns`).
//!
//! While the game runs on the company's map, the game's clock leads (`follow`): the company's
//! day goes along with it, and what the game reports of the company's buses comes in as it
//! comes (`tell_live`); it is booked at the day's close.
//!
//! The other parts of the company with things happening in its time hook in at `on_minute`
//! (called at every full hour and wherever a simulation stops).

use super::concessions as cn;
use super::dates;
use super::day::{DayReport, LiveEvent, Note, Plan};
use super::depot::Area;
use super::economy;
use super::model::{BookingKind, Cents, Company};
use super::network;
use super::plan::{self, BusOf, DayPlan, Disruption, Who};
use super::rng::Rng;
use super::staff::StaffNote;
use crate::LineInfo;
use serde::{Deserialize, Serialize};

/// Minutes in a day.
pub const DAY: i64 = 1440;
/// The morning "to the morning" goes to.
pub const MORNING: i64 = 5 * 60;
/// How many of the feed's lines are kept.
pub const FEED_KEPT: usize = 300;
/// Minutes until a rental bus ordered for a broken-down one takes over its trips.
pub const RENTAL_MINUTES: i32 = 45;
/// When building work and workshop jobs are finished on their last day, and someone leaving
/// has their last hour.
pub const BUILT_AT: i64 = 16 * 60;
pub const JOB_DONE_AT: i64 = 15 * 60 + 30;
pub const LAST_DAY_AT: i64 = 17 * 60;

// --- the time -------------------------------------------------------------------------------

/// The company's now: minutes since 1970-01-01 00:00 of its calendar.
pub fn now(c: &Company) -> i64 {
    moment(&c.date, c.clock.minute as i64)
}

/// A date's minute as the clock counts.
pub fn moment(date: &str, minute: i64) -> i64 {
    dates::parse(date).unwrap_or(0) * DAY + minute
}

/// The first day the company may be moved to (`move_to`): its last day booked or closed
/// (None: any).
pub fn earliest_date(c: &Company) -> Option<String> {
    c.ledger.iter().map(|b| b.date.as_str()).chain(c.history.iter().map(|h| h.date.as_str())).filter_map(dates::parse).max().map(dates::fmt)
}

/// Move the company to another day (the company's settings): its clock to that day's
/// midnight, the day's events made anew. Nothing between is simulated - the days skipped bring
/// no money and no wear - and offers, deliveries and contracts keep their own dates. Back only
/// as far as its last day booked or closed: the books would not add up before it.
pub fn move_to(c: &mut Company, date: &str) -> Result<(), &'static str> {
    let Some(d) = dates::parse(date) else { return Err("That is no date.") };
    if earliest_date(c).and_then(|e| dates::parse(&e)).is_some_and(|e| d < e) {
        return Err("The company's books go further: it cannot go back before its last booking.");
    }
    c.date = dates::fmt(d);
    c.clock.minute = 0;
    c.clock.today = None;
    c.clock.ask = None;
    c.clock.target = None;
    Ok(())
}

pub fn date_of(m: i64) -> String {
    dates::fmt(m.div_euclid(DAY))
}

pub fn minute_of(m: i64) -> i64 {
    m.rem_euclid(DAY)
}

/// "08:12".
pub fn hhmm(m: i64) -> String {
    let x = minute_of(m);
    format!("{:02}:{:02}", x / 60, x % 60)
}

/// What the pages offer to simulate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    Minutes(i64),
    /// To the next 05:00.
    Morning,
    /// To the next midnight: the day is closed ("Simulate to tomorrow").
    Midnight,
    Days(i64),
    /// To the next time it is this minute of the day.
    Until(i64),
}

/// Where a step from now goes.
pub fn target(c: &Company, step: Step) -> i64 {
    let now = now(c);
    let next = |m: i64| {
        let at = now.div_euclid(DAY) * DAY + m.rem_euclid(DAY);
        if at > now {
            at
        } else {
            at + DAY
        }
    };
    match step {
        Step::Minutes(n) => now + n.max(1),
        Step::Morning => next(MORNING),
        Step::Midnight => (now.div_euclid(DAY) + 1) * DAY,
        Step::Days(n) => now + n.max(1) * DAY,
        Step::Until(m) => next(m),
    }
}

// --- what the company keeps -------------------------------------------------------------------

/// The clock as the company keeps it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Clock {
    /// Minutes into the company's day (`Company::date`; 0 = its midnight, the day before it
    /// closed).
    #[serde(default)]
    pub minute: u32,
    /// What happened, the newest last.
    #[serde(default)]
    pub feed: Vec<FeedItem>,
    /// Today's own events (written when the clock entered the day).
    #[serde(default)]
    pub today: Option<DayScript>,
    /// Something waits for the player's decision; the simulation stopped there, on its way to
    /// `target`.
    #[serde(default)]
    pub ask: Option<Ask>,
    #[serde(default)]
    pub target: Option<i64>,
    /// The dispatcher decides breakdowns by himself (the simulation does not wait for them).
    #[serde(default)]
    pub dispatcher: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// The ordinary run of the day (a tour leaving).
    #[default]
    Minor,
    Info,
    Good,
    Warn,
    Bad,
}

/// A line of the feed: its text as an English key with `%{name}` places, and what goes in
/// them ("amount" in cents and "date" are written as the page writes money and days).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct FeedItem {
    pub at: i64,
    pub text: String,
    #[serde(default)]
    pub args: Vec<(String, String)>,
    #[serde(default)]
    pub level: Level,
}

/// A day's own events.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct DayScript {
    pub date: String,
    pub events: Vec<Timed>,
    /// The buses that break down on their tours today (the day's close takes them from here).
    #[serde(default)]
    pub breaks: Vec<Broken>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Timed {
    /// The minute of the day.
    pub at: i64,
    pub what: What,
    #[serde(default)]
    pub done: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum What {
    Out { number: String, tour: String, bus: String, driver: String },
    In { number: String, tour: String },
    NotRunning { number: String, tour: String, why: String },
    Late { name: String, minutes: i32, number: String, tour: String },
    NoStart { bus: String, cost: Cents },
    /// What fell out in the morning, for the dispatcher (the planning).
    Morning { count: usize },
    Breakdown { index: usize },
    Built { area: String, level: u32 },
    JobDone { bus: String, job: String },
    LastDay { name: String },
    /// The night's notes, told in the morning.
    Told { text: String, args: Vec<(String, String)>, level: Level },
}

/// A bus that breaks down on its tour, and what was decided for its trips.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Broken {
    pub vehicle: u32,
    pub bus: String,
    pub line: String,
    pub at: i32,
    #[serde(default)]
    pub choice: Option<Choice>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    /// A rental bus takes over its trips after `RENTAL_MINUTES`.
    Rental,
    /// Its trips after the breakdown are dropped.
    Drop,
}

/// What waits for the player.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Ask {
    /// Today's `breaks[index]`.
    Breakdown { index: usize },
    /// A tender opened, or the player's bid was beaten.
    Tender { id: u32, outbid: bool },
    /// The morning's disruptions (the planning has them).
    Morning { count: usize },
    /// Something another part of the company tells (`on_minute`).
    Notice { text: String, args: Vec<(String, String)> },
}

/// What a simulation came to.
#[derive(Clone, Debug, Default)]
pub struct Run {
    /// The days closed on the way.
    pub reports: Vec<DayReport>,
    /// It stopped for a decision (`Clock::ask`).
    pub stopped: bool,
    /// Lines added to the feed.
    pub told: usize,
}

/// What the simulation needs from outside: the timetable and the night.
pub trait World {
    /// The company map's lines on a date.
    fn lines(&mut self, c: &Company, date: &str) -> Result<Vec<LineInfo>, String>;
    /// The night after the company's day (`store::close_day`'s work): the date moves on.
    fn close_day(&mut self, c: &mut Company, lines: &[LineInfo]) -> Result<DayReport, String>;
}

fn arg(k: &str, v: impl ToString) -> (String, String) {
    (k.to_string(), v.to_string())
}

fn tell(c: &mut Company, run: &mut Run, at: i64, text: &str, args: Vec<(String, String)>, level: Level) {
    c.clock.feed.push(FeedItem { at, text: text.to_string(), args, level });
    run.told += 1;
    if c.clock.feed.len() > FEED_KEPT {
        let extra = c.clock.feed.len() - FEED_KEPT;
        c.clock.feed.drain(..extra);
    }
}

// --- a day's events ---------------------------------------------------------------------------

/// The buses of a day's tours with the span they are out (id, from, to).
fn spans(tours: impl Iterator<Item = (u32, i32, i32)>) -> Vec<(u32, i32, i32)> {
    let mut out: Vec<(u32, i32, i32)> = Vec::new();
    for (id, a, b) in tours {
        match out.iter_mut().find(|s| s.0 == id) {
            Some(s) => {
                s.1 = s.1.min(a);
                s.2 = s.2.max(b);
            }
            None => out.push((id, a, b)),
        }
    }
    out.sort_by_key(|s| s.0);
    out
}

/// The buses that break down on their way (drawn from the company, the date and the bus: the
/// same day draws the same), and when.
pub fn roll_breakdowns(c: &Company, date: &str, spans: &[(u32, i32, i32)]) -> Vec<(u32, i32)> {
    let r = economy::rules(c.difficulty);
    let day = dates::parse(date).unwrap_or(0);
    let mut out = Vec::new();
    for &(id, a, b) in spans {
        let Some(v) = c.vehicle(id) else { continue };
        let overdue = if v.km > v.next_service_km { 2.0 } else { 1.0 };
        let p = 0.004 * r.breakdown_factor * (1.0 + (100.0 - v.condition) / 25.0) * overdue;
        let mut rng = Rng::of(&[&c.id, "breakdown", &id.to_string()], day);
        if rng.chance(p) {
            out.push((id, rng.int(a as i64, b.max(a) as i64) as i32));
        }
    }
    out
}

/// The breakdowns the day's close counts (vehicle, minute, and from when a rental bus runs its
/// trips): today's script's when the clock went through the day, else drawn as it would have;
/// and the buses the game reported broken down.
pub fn breakdowns(c: &Company, plan: &Plan, date: &str, live_broken: &[u32]) -> Vec<(u32, i32, Option<i32>)> {
    let sp = spans(plan.tours.iter().filter_map(|t| Some((t.bus?, t.tour.from(), t.tour.to()))));
    let mut out: Vec<(u32, i32, Option<i32>)> = match c.clock.today.as_ref().filter(|s| s.date == date) {
        Some(s) => s.breaks.iter().map(|b| (b.vehicle, b.at, (b.choice == Some(Choice::Rental)).then_some(b.at + RENTAL_MINUTES))).collect(),
        None => roll_breakdowns(c, date, &sp).into_iter().map(|(v, at)| (v, at, None)).collect(),
    };
    let day = dates::parse(date).unwrap_or(0);
    for id in live_broken {
        if out.iter().any(|x| x.0 == *id) {
            continue;
        }
        let (a, b) = sp.iter().find(|s| s.0 == *id).map(|s| (s.1, s.2)).unwrap_or((6 * 60, 18 * 60));
        let mut rng = Rng::of(&[&c.id, "live-breakdown", &id.to_string()], day);
        out.push((*id, rng.int(a as i64, b.max(a) as i64) as i32, None));
    }
    out
}

fn bus_label(c: &Company, b: Option<BusOf>) -> String {
    match b {
        Some(BusOf::Own(id)) => c.vehicle(id).map(|v| v.number.clone()).unwrap_or_default(),
        Some(BusOf::Rental) => "rental".into(),
        None => String::new(),
    }
}

fn who_label(c: &Company, w: Option<Who>) -> String {
    match w {
        Some(Who::Staff(id)) => c.employee(id).map(|e| e.name.clone()).unwrap_or_default(),
        Some(Who::Agency) => "agency".into(),
        Some(Who::Player) => "you".into(),
        None => String::new(),
    }
}

/// The night's notes as the morning tells them (when, what, how much it matters).
fn told_notes(c: &Company, date: &str) -> Vec<(i64, String, Vec<(String, String)>, Level)> {
    let mut out = Vec::new();
    let Some(r) = c.last_report.as_ref().filter(|r| dates::add(&r.date, 1) == date) else { return out };
    for n in &r.notes {
        let x = match n {
            Note::Service { number } => (0, "Bus %{bus} goes to its service today", vec![arg("bus", number)], Level::Info),
            Note::Returned { number, name } => (0, "%{bus} %{name} went back", vec![arg("bus", number), arg("name", name)], Level::Info),
            Note::Month { month, result } => (0, "The month %{month} was closed: %{amount}", vec![arg("month", month), arg("amount", result)], if *result >= 0 { Level::Good } else { Level::Warn }),
            Note::Built { area } => (0, "Building work finished: %{area}", vec![arg("area", area)], Level::Good),
            Note::BayWait { number } => (6 * 60, "Bus %{bus} waits for a free workshop bay", vec![arg("bus", number)], Level::Warn),
            Note::Won { number, until } => (0, "The concession of line %{n} runs until %{date}", vec![arg("n", number), arg("date", until)], Level::Good),
            Note::Ended { number } => (0, "Line %{n} is no longer the company's: its concession ended", vec![arg("n", number)], Level::Bad),
            Note::NotStarted { number, charge, .. } => (0, "Line %{n} is not in service yet: the authority charged %{amount}", vec![arg("n", number), arg("amount", charge)], Level::Bad),
            Note::Accident { number, cost } => (0, "Bus %{bus} had an accident yesterday: damage %{amount}", vec![arg("bus", number), arg("amount", cost)], Level::Bad),
            Note::Breakdown { .. } | Note::LoanPaid { .. } | Note::Lost { .. } => continue,
        };
        out.push((x.0, x.1.to_string(), x.2, x.3));
    }
    for s in &r.staff {
        let x = match s {
            StaffNote::Sick { name, until } => (5 * 60 + 30, "%{name} called in sick, until %{date}", vec![arg("name", name), arg("date", until)], Level::Warn),
            StaffNote::Holiday { name, until } => (0, "%{name} is on holiday until %{date}", vec![arg("name", name), arg("date", until)], Level::Minor),
            StaffNote::Resigned { name, until } => (9 * 60, "%{name} handed in their notice: leaves on %{date}", vec![arg("name", name), arg("date", until)], Level::Warn),
            StaffNote::Unhappy { name } => (10 * 60, "%{name} is unhappy", vec![arg("name", name)], Level::Warn),
            StaffNote::Left { name } => (0, "%{name} left the company", vec![arg("name", name)], Level::Info),
        };
        out.push((x.0, x.1.to_string(), x.2, x.3));
    }
    out
}

/// A day's own events, from its plan (`plan::day_plan` of the company's day).
pub fn script(c: &Company, dp: &DayPlan) -> DayScript {
    let date = dp.date.clone();
    let mut ev: Vec<Timed> = Vec::new();
    let mut push = |at: i64, what: What| ev.push(Timed { at: at.clamp(0, DAY - 1), what, done: false });
    for (at, text, args, level) in told_notes(c, &date) {
        push(at, What::Told { text, args, level });
    }
    // the morning
    let first = dp.tours.iter().map(|t| t.tour.from() as i64).min().unwrap_or(6 * 60);
    let mut morning: Option<i64> = None;
    for d in &dp.disruptions {
        match *d {
            Disruption::Late { employee, minutes } => {
                let duty = dp.tours.iter().flat_map(|t| t.duties.iter().map(move |d| (t, d))).filter(|(_, d)| d.who == Some(Who::Staff(employee))).min_by_key(|(_, d)| d.from);
                let name = c.employee(employee).map(|e| e.name.clone()).unwrap_or_default();
                let (at, number, tour) = match duty {
                    Some((t, d)) => (d.from as i64, t.tour.number.clone(), t.tour.tour.clone()),
                    None => continue,
                };
                morning = Some(morning.map_or(at, |m| m.min(at)));
                push(at, What::Late { name, minutes, number, tour });
            }
            Disruption::Breakdown { vehicle, cost } => {
                let at = (first - 30).max(4 * 60);
                morning = Some(morning.map_or(at, |m| m.min(at)));
                push(at, What::NoStart { bus: c.vehicle(vehicle).map(|v| v.number.clone()).unwrap_or_default(), cost });
            }
        }
    }
    if let Some(at) = morning {
        push(at, What::Morning { count: dp.disruptions.len() });
    }
    // the tours
    for t in dp.tours.iter().filter(|t| !t.by_player) {
        let (number, tour) = (t.tour.number.clone(), t.tour.tour.clone());
        if t.covered() {
            push(t.tour.from() as i64, What::Out { number: number.clone(), tour: tour.clone(), bus: bus_label(c, t.bus), driver: who_label(c, t.duties.first().and_then(|d| d.who)) });
            push(t.tour.to() as i64, What::In { number, tour });
        } else {
            let why = t.bus_problem.or_else(|| t.duties.iter().find_map(|d| d.problem)).map(|p| p.label()).unwrap_or(if t.bus.is_none() { "No bus" } else { "No driver" });
            push(t.tour.from() as i64, What::NotRunning { number, tour, why: why.to_string() });
        }
    }
    // the breakdowns on the way
    let sp = spans(dp.tours.iter().filter(|t| t.covered()).filter_map(|t| match t.bus {
        Some(BusOf::Own(id)) => Some((id, t.tour.from(), t.tour.to())),
        _ => None,
    }));
    let mut breaks = Vec::new();
    for (vehicle, at) in roll_breakdowns(c, &date, &sp) {
        let line = dp.tours.iter().filter(|t| t.bus == Some(BusOf::Own(vehicle))).find(|t| t.tour.from() <= at && at <= t.tour.to()).or_else(|| dp.tours.iter().find(|t| t.bus == Some(BusOf::Own(vehicle)))).map(|t| t.tour.number.clone()).unwrap_or_default();
        let bus = c.vehicle(vehicle).map(|v| v.number.clone()).unwrap_or_default();
        breaks.push(Broken { vehicle, bus, line, at, choice: None });
        let index = breaks.len() - 1;
        push(at as i64, What::Breakdown { index });
    }
    // the depot, the people
    for w in c.site.works.iter().filter(|w| w.until == date) {
        push(BUILT_AT, What::Built { area: w.area.key().to_string(), level: w.level });
    }
    for j in c.site.jobs.iter().filter(|j| j.until.as_deref() == Some(date.as_str())) {
        push(JOB_DONE_AT, What::JobDone { bus: c.vehicle(j.vehicle).map(|v| v.number.clone()).unwrap_or_default(), job: j.kind.label().to_string() });
    }
    for e in c.staff.iter().filter(|e| e.notice_until.as_deref() == Some(date.as_str())) {
        push(LAST_DAY_AT, What::LastDay { name: e.name.clone() });
    }
    ev.sort_by_key(|e| e.at);
    DayScript { date, events: ev, breaks }
}

/// The company enters its day: the timetable read, the tender market refreshed, the day's
/// events written.
fn begin_day(c: &mut Company, w: &mut dyn World) -> Result<(), String> {
    if c.clock.today.as_ref().is_some_and(|s| s.date == c.date) {
        return Ok(());
    }
    let date = c.date.clone();
    let lines = w.lines(c, &date)?;
    cn::refresh(c, &lines);
    let tours = network::in_service_only(network::tours_of_day(c, &lines, &date));
    let dp = plan::day_plan(c, &date, tours, &[], &[], false);
    c.clock.today = Some(script(c, &dp));
    Ok(())
}

// --- what comes next, and its doing -------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Due {
    Script(usize),
    Open(u32),
    Bid(u32),
    Close(u32),
}

/// The first thing due up to `limit` (with things of the past not done yet first).
fn next_due(c: &Company, limit: i64) -> Option<(i64, Due)> {
    let day0 = moment(&c.date, 0);
    let mut best: Option<(i64, Due)> = None;
    let mut consider = |at: i64, d: Due| {
        if at <= limit && best.is_none_or(|b| at < b.0) {
            best = Some((at, d));
        }
    };
    if let Some(s) = c.clock.today.as_ref().filter(|s| s.date == c.date) {
        if let Some((i, e)) = s.events.iter().enumerate().find(|(_, e)| !e.done) {
            consider(day0 + e.at, Due::Script(i));
        }
    }
    for t in c.concessions.tenders.iter().filter(|t| t.open() && t.closes_at > 0) {
        if !t.opened {
            consider(t.opens_at, Due::Open(t.id));
            continue;
        }
        if let Some(p) = cn::bids(c, t, t.closes_at).get(t.told) {
            consider(p.at, Due::Bid(t.id));
        }
        consider(t.closes_at, Due::Close(t.id));
    }
    best
}

/// The dispatcher's choice for a breakdown: a rental bus while the cash allows.
fn dispatcher_choice(c: &Company, index: usize) -> Choice {
    let cost = rental_cost(c, index);
    if c.cash >= cost * 5 {
        Choice::Rental
    } else {
        Choice::Drop
    }
}

/// What a rental bus for the rest of a broken-down bus's day costs.
pub fn rental_cost(c: &Company, index: usize) -> Cents {
    let r = economy::rules(c.difficulty);
    let kind = c.clock.today.as_ref().and_then(|s| s.breaks.get(index)).and_then(|b| c.vehicle(b.vehicle)).map(|v| v.kind).unwrap_or_default();
    economy::rent_per_day(kind, &r, c.price_index)
}

/// Decide a breakdown: the rental bus ordered (and paid), or its trips dropped.
pub fn decide(c: &mut Company, index: usize, choice: Choice, run: &mut Run) {
    let at = now(c);
    let Some(b) = c.clock.today.as_ref().and_then(|s| s.breaks.get(index)).cloned() else { return };
    if b.choice.is_some() {
        return;
    }
    match choice {
        Choice::Rental => {
            let cost = rental_cost(c, index);
            c.book(BookingKind::Rent, -cost, format!("Rental bus for {} (line {})", b.bus, b.line), false);
            tell(c, run, at, "A rental bus takes over the trips of bus %{bus} on line %{n} from %{time}", vec![arg("bus", &b.bus), arg("n", &b.line), arg("time", hhmm(moment(&c.date, (b.at + RENTAL_MINUTES) as i64))), arg("amount", cost)], Level::Info);
        }
        Choice::Drop => tell(c, run, at, "The trips of bus %{bus} on line %{n} are dropped for the rest of the day", vec![arg("bus", &b.bus), arg("n", &b.line)], Level::Warn),
    }
    if let Some(x) = c.clock.today.as_mut().and_then(|s| s.breaks.get_mut(index)) {
        x.choice = Some(choice);
    }
}

fn apply(c: &mut Company, due: Due, quick: bool, run: &mut Run) {
    let at = now(c);
    let asks = !quick;
    match due {
        Due::Script(i) => {
            let Some(e) = c.clock.today.as_mut().and_then(|s| s.events.get_mut(i)) else { return };
            e.done = true;
            let when = moment(&c.date, e.at);
            let what = e.what.clone();
            match what {
                What::Out { number, tour, bus, driver } => {
                    let text = match (bus.as_str(), driver.as_str()) {
                        ("rental", _) => "Line %{n} tour %{tour} sets off with a rental bus",
                        (_, "agency") => "Line %{n} tour %{tour} sets off with bus %{bus} and an agency driver",
                        (_, "you") => "Line %{n} tour %{tour} sets off with bus %{bus}: your duty",
                        _ => "Line %{n} tour %{tour} sets off with bus %{bus}, %{name}",
                    };
                    tell(c, run, when, text, vec![arg("n", number), arg("tour", tour), arg("bus", bus), arg("name", driver)], Level::Minor);
                }
                What::In { number, tour } => tell(c, run, when, "Line %{n} tour %{tour} is back in the depot", vec![arg("n", number), arg("tour", tour)], Level::Minor),
                What::NotRunning { number, tour, why } => tell(c, run, when, "Line %{n} tour %{tour} does not run: %{why}", vec![arg("n", number), arg("tour", tour), arg("why", why)], Level::Warn),
                What::Late { name, minutes, number, tour } => tell(c, run, when, "%{name} comes %{min} minutes late for line %{n} tour %{tour}", vec![arg("name", name), arg("min", minutes), arg("n", number), arg("tour", tour)], Level::Warn),
                What::NoStart { bus, cost } => tell(c, run, when, "Bus %{bus} does not start this morning: towed, %{amount}", vec![arg("bus", bus), arg("amount", cost)], Level::Warn),
                What::Morning { count } => {
                    if asks && !c.clock.dispatcher {
                        c.clock.ask = Some(Ask::Morning { count });
                    }
                }
                What::Breakdown { index } => {
                    let Some(b) = c.clock.today.as_ref().and_then(|s| s.breaks.get(index)).cloned() else { return };
                    tell(c, run, when, "Bus %{bus} broke down on line %{n}", vec![arg("bus", &b.bus), arg("n", &b.line)], Level::Bad);
                    if asks && !c.clock.dispatcher {
                        c.clock.ask = Some(Ask::Breakdown { index });
                    } else {
                        let choice = dispatcher_choice(c, index);
                        decide(c, index, choice, run);
                    }
                }
                What::Built { area, level } => {
                    let Some(a) = Area::from_key(&area) else { return };
                    if c.site.works.iter().any(|w| w.area == a && w.level == level) {
                        c.site.works.retain(|w| !(w.area == a && w.level == level));
                        c.site.set_level(a, level);
                    }
                    tell(c, run, when, "Building work finished: %{area}", vec![arg("area", a.label())], Level::Good);
                }
                What::JobDone { bus, job } => tell(c, run, when, "Bus %{bus}: %{job} done, back on the road tomorrow", vec![arg("bus", bus), arg("job", job)], Level::Info),
                What::LastDay { name } => tell(c, run, when, "%{name} works their last day today", vec![arg("name", name)], Level::Info),
                What::Told { text, args, level } => tell(c, run, when, &text, args, level),
            }
        }
        Due::Open(id) => {
            let Some(t) = c.concessions.tenders.iter_mut().find(|t| t.id == id) else { return };
            t.opened = true;
            let (when, number, closes, renewal) = (t.opens_at, t.number.clone(), t.closes_at, t.renewal);
            let text = if renewal { "The renewal of line %{n} is out to tender: bids until %{time}" } else { "Line %{n} is out to tender: bids until %{time}" };
            tell(c, run, when, text, vec![arg("n", number), arg("time", hhmm(closes))], Level::Info);
            if asks {
                c.clock.ask = Some(Ask::Tender { id, outbid: false });
            }
        }
        Due::Bid(id) => {
            let Some(t) = c.concessions.tenders.iter().find(|t| t.id == id).cloned() else { return };
            let placed = cn::bids(c, &t, at);
            let mut outbid = false;
            for k in t.told..placed.len() {
                let p = placed[k];
                match p.who {
                    super::auction::Who::Player => tell(c, run, p.at, "You bid %{amount} for line %{n}", vec![arg("amount", p.amount), arg("n", &t.number)], Level::Info),
                    who => {
                        let name = cn::bidder_name(c, &t, who);
                        let beaten = k > 0 && placed[k - 1].who == super::auction::Who::Player;
                        outbid |= beaten;
                        tell(c, run, p.at, if beaten { "%{who} bids %{amount} for line %{n}: your bid is beaten" } else { "%{who} bids %{amount} for line %{n}" }, vec![arg("who", name), arg("amount", p.amount), arg("n", &t.number)], if beaten { Level::Warn } else { Level::Minor });
                    }
                }
            }
            if let Some(x) = c.concessions.tenders.iter_mut().find(|x| x.id == id) {
                x.told = placed.len();
            }
            if outbid && asks {
                c.clock.ask = Some(Ask::Tender { id, outbid: true });
            }
        }
        Due::Close(id) => {
            let Some(t) = c.concessions.tenders.iter().find(|t| t.id == id).cloned() else { return };
            // (what was bid before the close is told first)
            let placed = cn::bids(c, &t, t.closes_at);
            if placed.len() > t.told {
                apply(c, Due::Bid(id), true, run);
            }
            let Some((ev, amount)) = cn::close(c, id) else { return };
            let mine = !t.offers.is_empty();
            match ev {
                cn::Event::Won { until, .. } => tell(c, run, t.closes_at, "Line %{n} won for %{amount}: the concession runs until %{date}", vec![arg("n", &t.number), arg("amount", amount), arg("date", until)], Level::Good),
                cn::Event::Lost { winner, .. } if winner.is_empty() => tell(c, run, t.closes_at, "Nobody bid for line %{n}", vec![arg("n", &t.number)], Level::Minor),
                cn::Event::Lost { winner, .. } => {
                    let level = if mine || t.renewal { Level::Bad } else { Level::Minor };
                    let text = if t.renewal { "Line %{n} went to %{who} for %{amount}: your concession ends with its term" } else { "Line %{n} went to %{who} for %{amount}" };
                    tell(c, run, t.closes_at, text, vec![arg("n", &t.number), arg("who", winner), arg("amount", amount)], level);
                }
                cn::Event::Ended { .. } => {}
            }
        }
    }
}

/// The hook for the other parts of the company with things that happen in its time (the
/// dealer's deliveries and offers running out): called at every full hour the clock passes and
/// wherever a simulation stops, with the company's now. What it returns is told in the feed;
/// an item of `Level::Bad` stops the simulation for the player (`Ask::Notice`).
pub fn on_minute(c: &mut Company, now: i64) -> Vec<FeedItem> {
    let mut out = Vec::new();
    // the dealer's orders whose delivery time has come: the buses join the fleet
    for d in super::dealer::tick(c, &super::dealer::moment(now)) {
        out.push(FeedItem { at: now, text: "Delivered: %{name} (%{numbers})".into(), args: vec![("name".into(), d.name.clone()), ("numbers".into(), d.numbers.join(", "))], level: Level::Good });
    }
    out
}

fn hook(c: &mut Company, at: i64, quick: bool, run: &mut Run) {
    for item in on_minute(c, at) {
        if item.level == Level::Bad && !quick && c.clock.ask.is_none() {
            c.clock.ask = Some(Ask::Notice { text: item.text.clone(), args: item.args.clone() });
        }
        c.clock.feed.push(item);
        run.told += 1;
    }
    if c.clock.feed.len() > FEED_KEPT {
        let extra = c.clock.feed.len() - FEED_KEPT;
        c.clock.feed.drain(..extra);
    }
}

// --- simulating ---------------------------------------------------------------------------------

/// Simulate until `to` (minutes): every event on the way applied in its order, the days
/// closed at their midnights. Stops early for a decision (`Clock::ask`), unless `quick`: then
/// the dispatcher decides and nothing waits (the quick "Close the day", the game's clock).
/// Nothing moves while a decision waits and the run is not quick.
pub fn advance(c: &mut Company, to: i64, w: &mut dyn World, quick: bool) -> Result<Run, String> {
    let mut run = Run::default();
    if c.clock.ask.is_some() {
        if !quick {
            run.stopped = true;
            return Ok(run);
        }
        answer(c, None, &mut run);
    }
    loop {
        let now = now(c);
        if now >= to {
            break;
        }
        begin_day(c, w)?;
        let day0 = moment(&c.date, 0);
        let midnight = day0 + DAY;
        let limit = to.min(midnight).min((now.div_euclid(60) + 1) * 60);
        if let Some((at, due)) = next_due(c, limit) {
            if at > now {
                c.clock.minute = (at - day0) as u32;
            }
            apply(c, due, quick, &mut run);
            if quick {
                if let Some(Ask::Breakdown { index }) = c.clock.ask.clone() {
                    let choice = dispatcher_choice(c, index);
                    decide(c, index, choice, &mut run);
                }
                c.clock.ask = None;
            }
            if c.clock.ask.is_some() {
                c.clock.target = Some(to);
                run.stopped = true;
                return Ok(run);
            }
            continue;
        }
        c.clock.minute = (limit - day0) as u32;
        hook(c, limit, quick, &mut run);
        if c.clock.ask.is_some() {
            c.clock.target = Some(to);
            run.stopped = true;
            return Ok(run);
        }
        if limit == midnight {
            let date = c.date.clone();
            let lines = w.lines(c, &date)?;
            let report = w.close_day(c, &lines)?;
            c.clock.minute = 0;
            c.clock.today = None;
            tell(c, &mut run, midnight, "The day %{date} was closed: %{amount}", vec![arg("date", &date), arg("amount", report.result)], if report.result >= 0 { Level::Good } else { Level::Warn });
            run.reports.push(report);
        }
    }
    c.clock.target = None;
    Ok(run)
}

/// The player's answer to what waits (`choice`: for a breakdown; None there leaves it to the
/// dispatcher). Returns where the simulation was going.
pub fn answer(c: &mut Company, choice: Option<Choice>, run: &mut Run) -> Option<i64> {
    let ask = c.clock.ask.take()?;
    if let Ask::Breakdown { index } = ask {
        let choice = choice.unwrap_or_else(|| dispatcher_choice(c, index));
        decide(c, index, choice, run);
    }
    c.clock.target.take()
}

/// The game's clock leads while it runs on the company's map: the company's day moves along to
/// the game's minute of the day (forward only, not past the day; quick).
pub fn follow(c: &mut Company, game_minute: i64, w: &mut dyn World) -> Result<Run, String> {
    let to = moment(&c.date, game_minute.clamp(0, DAY - 1));
    if to <= now(c) {
        return Ok(Run::default());
    }
    advance(c, to, w, true)
}

/// What the game reported of the company's buses, told as it comes (booked at the day's close).
pub fn tell_live(c: &mut Company, events: &[LiveEvent]) -> usize {
    let mut run = Run::default();
    let at = now(c);
    for ev in events {
        match ev {
            LiveEvent::Trip { line, tour, passengers, delay, completed, .. } => {
                let text = if *completed { "Line %{n} tour %{tour}: a trip run in the game, %{pax} passengers, %{late} min off the timetable" } else { "Line %{n} tour %{tour}: a trip broken off in the game" };
                tell(c, &mut run, at, text, vec![arg("n", line), arg("tour", tour), arg("pax", passengers), arg("late", format!("{:+.0}", delay / 60.0))], Level::Minor);
            }
            LiveEvent::Breakdown { vehicle } => {
                let bus = c.vehicle(*vehicle).map(|v| v.number.clone()).unwrap_or_default();
                tell(c, &mut run, at, "Bus %{bus} broke down in the game", vec![arg("bus", bus)], Level::Bad);
            }
        }
    }
    run.told
}

/// The feed's lines of the company's day so far, the newest first.
pub fn feed_of_day(c: &Company) -> impl Iterator<Item = &FeedItem> {
    let day0 = moment(&c.date, 0);
    c.clock.feed.iter().rev().take_while(move |f| f.at >= day0)
}

#[cfg(test)]
mod tests {
    use super::super::concessions::tests::line;
    use super::super::market::{self, MarketBus, Payment};
    use super::super::staff::{applicants, hire};
    use super::super::{day, depot, found, Founding};
    use super::*;
    use crate::company::model::{Difficulty, Licence};

    /// The timetable of every day the same; the night as the store runs it.
    struct Fake(Vec<LineInfo>);

    impl World for Fake {
        fn lines(&mut self, _: &Company, _: &str) -> Result<Vec<LineInfo>, String> {
            Ok(self.0.clone())
        }

        fn close_day(&mut self, c: &mut Company, lines: &[LineInfo]) -> Result<DayReport, String> {
            network::refresh_lines(c, lines);
            let date = c.date.clone();
            let tours = network::tours_of_day(c, lines, &date);
            let r = day::close_day(c, tours, &[]);
            Ok(depot::after_close(c, r, lines))
        }
    }

    fn company(buses: usize, people: usize) -> (Company, Fake) {
        let lines = vec![line("Linie5", "5", 3, true), line("Linie7", "7", 2, true)];
        let mut c = found(&Founding { name: "Uhr".into(), difficulty: Difficulty::Realistic, date: "2024-03-04".into(), ..Default::default() }, "Luc");
        c.cash += 5_000_000_00;
        network::add_line(&mut c, &lines[0], None).unwrap();
        // (in service from the start, the dispatcher's own filling the roster's gaps)
        network::start_service(&mut c, "Linie5", 0).unwrap();
        c.planning.auto = true;
        let bus = MarketBus { file: "Vehicles/Citaro/Citaro.bus".into(), name: "Citaro".into(), ..Default::default() };
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
        }
        (c, Fake(lines))
    }

    #[test]
    fn steps_go_where_they_say() {
        let (mut c, _) = company(0, 0);
        c.clock.minute = 14 * 60 + 10;
        let now = now(&c);
        assert_eq!(target(&c, Step::Minutes(15)), now + 15);
        assert_eq!(hhmm(target(&c, Step::Morning)), "05:00");
        assert_eq!(date_of(target(&c, Step::Morning)), "2024-03-05");
        assert_eq!(target(&c, Step::Midnight), moment("2024-03-05", 0));
        assert_eq!(target(&c, Step::Days(2)), now + 2 * DAY);
        assert_eq!(target(&c, Step::Until(16 * 60)), moment("2024-03-04", 16 * 60));
        assert_eq!(target(&c, Step::Until(9 * 60)), moment("2024-03-05", 9 * 60));
        c.clock.minute = 0;
        assert_eq!(date_of(target(&c, Step::Morning)), "2024-03-04", "from midnight: this morning");
    }

    #[test]
    fn the_company_moves_to_another_day_but_not_before_its_books() {
        let (mut c, _) = company(1, 1);
        let start = c.date.clone();
        move_to(&mut c, "2030-01-15").unwrap();
        assert_eq!((c.date.as_str(), c.clock.minute), ("2030-01-15", 0));
        // (the founding's capital is booked on its first day: back to it, not before)
        assert_eq!(earliest_date(&c), Some(start.clone()));
        move_to(&mut c, &start).unwrap();
        assert!(move_to(&mut c, &dates::add(&start, -1)).is_err());
        assert!(move_to(&mut c, "nonsense").is_err());
    }

    #[test]
    fn a_day_runs_through_its_events_in_order_and_closes_at_midnight() {
        let (mut c, mut w) = company(3, 6);
        let run = advance(&mut c, moment("2024-03-04", 12 * 60), &mut w, true).unwrap();
        assert!(run.reports.is_empty() && !run.stopped);
        assert_eq!((c.date.as_str(), c.clock.minute), ("2024-03-04", 12 * 60));
        let feed = &c.clock.feed;
        assert!(feed.len() >= 3, "{feed:?}");
        assert!(feed.windows(2).all(|f| f[0].at <= f[1].at), "in order");
        assert!(feed.iter().all(|f| f.at <= now(&c)));
        // the three tours left at six (the line's first trips), none came back before nine
        assert_eq!(feed.iter().filter(|f| f.text.starts_with("Line %{n} tour %{tour} sets off") || f.text.contains("does not run")).count(), 3);
        assert!(feed.iter().filter(|f| f.text.contains("is back")).all(|f| minute_of(f.at) >= 9 * 60));
        let s = c.clock.today.as_ref().unwrap();
        assert!(s.events.iter().all(|e| e.done == (e.at <= 12 * 60)));
        // on to midnight: the day closes, the next one begins
        let to = target(&c, Step::Midnight);
        let run = advance(&mut c, to, &mut w, true).unwrap();
        assert_eq!(run.reports.len(), 1);
        assert_eq!((c.date.as_str(), c.clock.minute), ("2024-03-05", 0));
        assert_eq!(c.history.len(), 1);
        assert!(c.clock.feed.last().unwrap().text.starts_with("The day"));
    }

    #[test]
    fn catching_up_days_at_once_is_the_same_as_step_by_step() {
        let (a0, mut w) = company(2, 5);
        let mut a = a0.clone();
        let mut b = a0;
        let to = moment("2024-03-07", 10 * 60);
        let run = advance(&mut a, to, &mut w, true).unwrap();
        assert_eq!(run.reports.len(), 3);
        let mut n = 0;
        while now(&b) < to {
            let next = (now(&b) + 5 * 60 + 7).min(to);
            n += advance(&mut b, next, &mut w, true).unwrap().reports.len();
        }
        assert_eq!(n, 3);
        assert_eq!((a.date.as_str(), a.clock.minute), ("2024-03-07", 10 * 60));
        assert_eq!(a, b);
    }

    #[test]
    fn a_breakdown_waits_for_the_player_and_a_rental_bus_saves_its_trips() {
        let (mut c, mut w) = company(3, 6);
        advance(&mut c, moment("2024-03-04", 5 * 60), &mut w, false).unwrap();
        // a bus out on a tour breaks down at eight (and nothing else stops the day)
        let s = c.clock.today.as_mut().unwrap();
        s.breaks.clear();
        s.events.retain(|e| !matches!(e.what, What::Breakdown { .. } | What::Morning { .. }));
        let number = s.events.iter().find_map(|e| if let What::Out { bus, .. } = &e.what { Some(bus.clone()) } else { None }).unwrap();
        let bus = c.fleet.iter().find(|v| v.number == number).unwrap().clone();
        let s = c.clock.today.as_mut().unwrap();
        s.breaks.push(Broken { vehicle: bus.id, bus: bus.number.clone(), line: "5".into(), at: 8 * 60, choice: None });
        s.events.push(Timed { at: 8 * 60, what: What::Breakdown { index: 0 }, done: false });
        s.events.sort_by_key(|e| e.at);
        let mut rental = c.clone();
        let run = advance(&mut c, moment("2024-03-04", 12 * 60), &mut w, false).unwrap();
        assert!(run.stopped);
        assert_eq!(c.clock.ask, Some(Ask::Breakdown { index: 0 }));
        assert_eq!(hhmm(now(&c)), "08:00");
        // nothing moves while it waits
        assert!(advance(&mut c, moment("2024-03-04", 12 * 60), &mut w, false).unwrap().stopped);
        assert_eq!(hhmm(now(&c)), "08:00");
        let mut dropped = c.clone();
        let cash = c.cash;
        let mut r = Run::default();
        assert_eq!(answer(&mut c, Some(Choice::Rental), &mut r), Some(moment("2024-03-04", 12 * 60)));
        assert_eq!(cash - c.cash, rental_cost(&c, 0));
        answer(&mut dropped, Some(Choice::Drop), &mut r);
        let end = moment("2024-03-05", 0);
        let a = advance(&mut c, end, &mut w, true).unwrap().reports.remove(0);
        let b = advance(&mut dropped, end, &mut w, true).unwrap().reports.remove(0);
        assert!(a.dropped < b.dropped, "{} {}", a.dropped, b.dropped);
        assert!(a.notes.iter().any(|n| matches!(n, Note::Breakdown { number, .. } if *number == bus.number)));
        // quick: the dispatcher decides by himself
        let run = advance(&mut rental, end, &mut w, true).unwrap();
        assert!(!run.stopped && rental.clock.ask.is_none());
        assert_eq!(rental.clock.today, None);
    }

    #[test]
    fn a_tender_runs_for_hours_with_the_rivals_bidding_as_the_clock_goes() {
        let (mut c, mut w) = company(1, 2);
        c.clock.dispatcher = true;
        advance(&mut c, moment("2024-03-04", 9 * 60), &mut w, true).unwrap();
        c.concessions.tenders.clear();
        let id = cn::apply(&mut c, &w.0[1]).unwrap();
        let t = c.concessions.tenders.iter().find(|t| t.id == id).unwrap().clone();
        assert_eq!(t.opens_at, now(&c));
        // it opens: the player is asked
        let run = advance(&mut c, t.closes_at + 60, &mut w, false).unwrap();
        assert!(run.stopped);
        assert_eq!(c.clock.ask, Some(Ask::Tender { id, outbid: false }));
        // the least bid; a rival beats it soon and the simulation stops for it
        let least = cn::min_bid(&c, c.concessions.tenders.iter().find(|t| t.id == id).unwrap());
        cn::bid(&mut c, id, least).unwrap();
        let mut r = Run::default();
        let to = answer(&mut c, None, &mut r).unwrap();
        let mut stops = 0;
        while c.concessions.tenders.iter().find(|t| t.id == id).unwrap().open() {
            let run = advance(&mut c, to, &mut w, false).unwrap();
            if run.stopped {
                stops += 1;
                assert!(matches!(c.clock.ask, Some(Ask::Tender { outbid: true, .. })));
                answer(&mut c, None, &mut r);
            }
            assert!(stops < 20);
        }
        let t = c.concessions.tenders.iter().find(|t| t.id == id).unwrap();
        assert!(matches!(t.outcome, Some(cn::Outcome::Lost { .. }) | Some(cn::Outcome::Won { .. })));
        if matches!(t.outcome, Some(cn::Outcome::Lost { .. })) {
            assert!(stops >= 1, "a lost auction told the player he was beaten");
        }
        // it closed at its minute, the same day, and the feed told the bids as they came
        assert_eq!(c.date, "2024-03-04");
        assert!(now(&c) >= t.closes_at);
        let bids: Vec<&FeedItem> = c.clock.feed.iter().filter(|f| f.text.contains("bids %{amount}") || f.text.starts_with("You bid")).collect();
        assert!(bids.len() >= 2 && bids.iter().all(|f| f.at >= t.opens_at && f.at < t.closes_at));
        assert!(c.clock.feed.iter().any(|f| f.at == t.closes_at && f.text.starts_with("Line %{n}")));
    }
}
