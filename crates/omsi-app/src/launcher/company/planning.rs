//! The company's planning (Omsi-Hub's Planning page, `busbedrijf-planning.md`): a strip of
//! the next seven days, and the chosen day as a Gantt chart - the tours grouped by line, each
//! tour's duties as blocks coloured by who drives them, the bus beside the tour. A driver or
//! a bus is given by tapping it in the side list and then the duty or the tour, or by
//! dragging it there; tapping a duty or a tour shows who could take it. What is given goes
//! into the weekly roster, which repeats every week (`co::plan`); what fell out on the
//! company's own day is filled for that day only. The tools fill the roster as the
//! dispatcher would, clear the day, or repeat it on the other working days. "Drive this
//! duty" opens the Drive page with exactly that duty (as the Career tab's "My duties" does).
//! A line that asks for buses of its own (the line editor's Kind tab) is given only those:
//! the dispatcher takes none other, and a bus that is none of them is refused.

use super::super::flow::Step;
use super::super::theme::*;
use super::super::ui::{id_of, ButtonKind, Key, Ui};
use super::super::{Launcher, Page};
use super::kit::{self, Foot};
use super::{act, day_label, eur, line_plate, section, Dialog};
use glam::Vec2;
use omsi_launcher_lib as core;
use omsi_launcher_lib::company::plan::{self as pl, BusOf, DayPlan, DayTour, Disruption, Fill, Problem, Source, Warn, Who};
use omsi_launcher_lib::company::staff::Block;
use omsi_launcher_lib::company::{self as co, Company};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};
use std::sync::mpsc::{channel, Receiver, Sender};

/// A day's timetable as it was read: map, date, its lines.
type Read = (String, String, Result<Vec<core::LineInfo>, String>);

/// A driver or a bus taken up to be given (tapped, or dragged).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Arm {
    Driver(Who),
    Bus(u32),
}

#[derive(Clone, PartialEq, Debug)]
enum Sel {
    Tour(String, String),
    Duty(String, String, usize),
}

pub struct PlanningView {
    tx: Sender<Read>,
    rx: Receiver<Read>,
    days: Vec<Read>,
    asked: Vec<(String, String)>,
    /// The day shown: 0 the company's day, up to 6.
    pub(super) day: usize,
    sel: Option<Sel>,
    arm: Option<Arm>,
    /// Pressed on a driver or a bus of the list: dragged while the button is held.
    drag: Option<Arm>,
    /// A duty or a tour carried to a driver or a bus of the list (Luc: everything in the
    /// planning can be dragged): taken up from the open duties at once, from the chart once
    /// the mouse moved off where it was pressed (`press`) - a tap there still chooses it.
    carry: Option<(usize, Option<usize>)>,
    press: Option<(Vec2, usize, Option<usize>)>,
    /// Where the carry was taken up (its label shows once the mouse is off it).
    carry_from: Vec2,
    /// The list's drivers and buses where they are drawn this frame (to drop on).
    targets: Vec<(Rect, Arm)>,
    /// "Clear the day" pressed once: pressed again it clears.
    clear_armed: bool,
    /// The plans made: date, the company's generation they were made for, the plan.
    cache: Vec<(String, u64, DayPlan)>,
    /// The timetable's revision the days were read at (`CompanyView::timetable`: the line
    /// editor wrote it since - an own line's tours were missing until it was read again).
    read_at: u64,
    /// A line to show (from the Lines page): its group scrolled into view once.
    pub(super) focus: Option<String>,
}

impl Default for PlanningView {
    fn default() -> Self {
        let (tx, rx) = channel();
        PlanningView { tx, rx, days: Vec::new(), asked: Vec::new(), day: 0, sel: None, arm: None, drag: None, carry: None, press: None, carry_from: Vec2::ZERO, targets: Vec::new(), clear_armed: false, cache: Vec::new(), read_at: 0, focus: None }
    }
}

impl PlanningView {
    /// Forget the week read: it is read again (the timetable was written).
    fn forget(&mut self, revision: u64) {
        self.days.clear();
        self.asked.clear();
        self.cache.clear();
        self.read_at = revision;
        // (a read still on its way is of the old timetable: a fresh channel leaves it)
        let (tx, rx) = channel();
        (self.tx, self.rx) = (tx, rx);
    }
}

/// Where a block or a bus is in the chart (for dropping on it).
struct Hit {
    r: Rect,
    tour: usize,
    duty: Option<usize>,
}

fn hhmm(minutes: i32) -> String {
    super::super::state::hhmm(minutes as f64 * 60.0)
}

fn length(minutes: i32) -> String {
    format!("{} h {:02}", minutes / 60, minutes % 60)
}

/// The colours drivers are told apart by (calm, and readable under dark ink).
const DRIVERS: [(u8, u8, u8); 10] = [(110, 168, 230), (96, 196, 170), (196, 150, 214), (226, 184, 110), (140, 190, 120), (226, 150, 130), (150, 206, 240), (200, 200, 120), (180, 160, 230), (120, 200, 210)];
const INK: Color = Color::rgba(14, 18, 28, 1.0);

fn driver_colour(c: &Company, id: u32) -> Color {
    let k = c.staff.iter().position(|e| e.id == id).unwrap_or(id as usize);
    let (r, g, b) = DRIVERS[k % DRIVERS.len()];
    Color::rgba(r, g, b, 1.0)
}

fn first_name(c: &Company, id: u32) -> String {
    c.employee(id).map(|e| e.name.split_whitespace().next().unwrap_or(&e.name).to_string()).unwrap_or_else(|| "?".into())
}

fn who_name(c: &Company, w: Option<Who>) -> String {
    match w {
        Some(Who::Staff(id)) => c.employee(id).map(|e| e.name.clone()).unwrap_or_else(|| "?".into()),
        Some(Who::Player) => omsi_ui::tr("You").into_owned(),
        Some(Who::Agency) => omsi_ui::tr("Agency driver").into_owned(),
        None => omsi_ui::tr("Nobody").into_owned(),
    }
}

fn source_text(s: Source) -> &'static str {
    match s {
        Source::Roster => "fixed in the roster",
        Source::Auto => "the dispatcher's suggestion",
        Source::Dispatcher => "your choice for today",
        Source::Central => "filled by the central",
        Source::None => "",
    }
}

fn problem_colour(p: Option<Problem>) -> Color {
    match p {
        None | Some(Problem::Unassigned) => WARN,
        Some(_) => DANGER.lighten(0.2),
    }
}

fn line_title(t: &DayTour, duty: Option<usize>) -> String {
    let mut s = format!("{} {}  ·  {} {}", omsi_ui::tr("Line"), t.tour.number, omsi_ui::tr("Tour"), t.tour.tour);
    if let Some(k) = duty {
        s.push_str(&format!("  ·  {} {}", omsi_ui::tr("Duty"), k + 1));
    }
    s
}

// --- the data --------------------------------------------------------------------------------

/// Take in the timetables read, and ask for those of the week not read yet.
pub(super) fn work(l: &mut Launcher, c: &Company) {
    let revision = l.company.timetable;
    let v = &mut l.company.planning;
    if v.read_at != revision {
        v.forget(revision);
    }
    while let Ok(r) = v.rx.try_recv() {
        v.days.retain(|d| !(d.0 == r.0 && d.1 == r.1));
        v.days.push(r);
    }
    let week: Vec<(String, String)> = (0..7).map(|k| (c.map.clone(), co::dates::add(&c.date, k))).collect();
    v.days.retain(|d| week.iter().any(|w| w.0 == d.0 && w.1 == d.1));
    v.asked.retain(|a| week.contains(a));
    let missing: Vec<(String, String)> = week.into_iter().filter(|w| !v.asked.contains(w)).collect();
    if missing.is_empty() {
        return;
    }
    v.asked.extend(missing.iter().cloned());
    let tx = v.tx.clone();
    std::thread::spawn(move || {
        for (map, date) in missing {
            let r = core::list_lines(&map, &date).map_err(|e| format!("{e:#}"));
            if tx.send((map, date, r)).is_err() {
                break;
            }
        }
    });
}

/// The plan of a day of the strip (None: its timetable is still being read).
pub(super) fn plan_of(l: &mut Launcher, c: &Company, date: &str) -> Option<Result<DayPlan, String>> {
    let generation = l.company.generation;
    let v = &mut l.company.planning;
    if let Some(p) = v.cache.iter().find(|x| x.0 == date && x.1 == generation) {
        return Some(Ok(p.2.clone()));
    }
    let read = v.days.iter().find(|d| d.0 == c.map && d.1 == date)?;
    match &read.2 {
        Err(e) => Some(Err(e.clone())),
        Ok(lines) => {
            let tours = co::network::tours_of_day(c, lines, date);
            let p = pl::day_plan(c, date, tours, &[], &[], false);
            v.cache.retain(|x| x.1 == generation && x.0 != date);
            v.cache.push((date.to_string(), generation, p.clone()));
            Some(Ok(p))
        }
    }
}

/// Tomorrow's plan, for the overview's warnings (None: its timetable is still being read).
pub(super) fn tomorrow(l: &mut Launcher, c: &Company) -> Option<co::day::Plan> {
    work(l, c);
    let date = co::dates::add(&c.date, 1);
    plan_of(l, c, &date)?.ok().map(|p| p.to_plan())
}

/// The blocks a driver has on the plan's day, without one duty (to see whether it fits).
fn blocks_without(p: &DayPlan, id: u32, skip: (usize, usize)) -> Vec<Block> {
    p.duties_of(Who::Staff(id))
        .into_iter()
        .filter(|x| *x != skip)
        .map(|(i, k)| {
            let t = &p.tours[i];
            let d = &t.duties[k];
            Block { key: pl::duty_key(&t.tour.line, &t.tour.tour, k), from: d.from, to: d.to, from_stop: t.tour.trips[d.start].from.clone(), to_stop: t.tour.trips[d.end - 1].to.clone() }
        })
        .collect()
}

fn block_of(t: &DayTour, k: usize) -> Block {
    let d = &t.duties[k];
    Block { key: pl::duty_key(&t.tour.line, &t.tour.tour, k), from: d.from, to: d.to, from_stop: t.tour.trips[d.start].from.clone(), to_stop: t.tour.trips[d.end - 1].to.clone() }
}

/// What a driver would make of a duty: a word and its colour.
fn fit_of(c: &Company, p: &DayPlan, ti: usize, k: usize, id: u32) -> (String, Color, bool) {
    let Some(e) = c.employee(id) else { return ("?".into(), TEXT_FAINT, false) };
    if !e.employed_on(&p.date) {
        return (omsi_ui::tr("Not employed then").into_owned(), TEXT_FAINT, false);
    }
    if e.sick_until.as_deref().is_some_and(|u| co::dates::between(&p.date, u) >= 0) {
        return (omsi_ui::tr("Ill").into_owned(), DANGER.lighten(0.2), false);
    }
    if e.holiday_until.as_deref().is_some_and(|u| co::dates::between(&p.date, u) >= 0) {
        return (omsi_ui::tr("On holiday").into_owned(), TEXT_DIM, false);
    }
    let t = &p.tours[ti];
    let work = blocks_without(p, id, (ti, k));
    let bus = t.bus_of(c);
    match pl::fits_bus(e, &bus, &work, &block_of(t, k), p.today) {
        Err(Problem::Licence | Problem::NoEndorsement | Problem::NoTypeTraining) => {
            let why = co::licences::lack(e, bus.0, bus.1.as_deref()).map(|x| super::people::lack_text(c, &x)).unwrap_or_else(|| omsi_ui::tr("No licence for this bus").into_owned());
            (why, DANGER.lighten(0.2), false)
        }
        Err(_) => (omsi_ui::tr("Clashes with other work").into_owned(), DANGER.lighten(0.2), false),
        Ok(w) => match w.first() {
            Some(Warn::Overtime(m)) => (omsi_ui::tr("Overtime %{t}").replace("%{t}", &length(*m)), WARN, true),
            Some(Warn::Transfer(m)) => (omsi_ui::tr("%{n} min to change stops").replace("%{n}", &m.to_string()), WARN, true),
            Some(Warn::Experience) => (omsi_ui::tr("Little experience for this bus").into_owned(), WARN, true),
            None => (omsi_ui::tr("Fits").into_owned(), OK, true),
        },
    }
}

/// A duty's problem in words; for a driver not qualified for the bus, what they lack
/// ("No type training for the Citaro").
fn problem_text(c: &Company, p: &DayPlan, ti: usize, k: usize, problem: Option<Problem>) -> String {
    let t = &p.tours[ti];
    if matches!(problem, Some(Problem::Licence | Problem::NoEndorsement | Problem::NoTypeTraining)) {
        let rostered = pl::roster(c, p.weekday, &t.tour.line, &t.tour.tour).and_then(|r| r.duties.get(k).copied().flatten());
        if let Some(e) = rostered.and_then(|w| if let Who::Staff(id) = w { c.employee(id) } else { None }) {
            let (kind, bus) = t.bus_of(c);
            if let Some(x) = co::licences::lack(e, kind, bus.as_deref()) {
                return format!("{}: {}", first_name(c, e.id), super::people::lack_text(c, &x));
            }
        }
    }
    omsi_ui::tr(problem.map(Problem::label).unwrap_or("No driver")).into_owned()
}

/// Where a bus is on the plan's day besides this tour: (word, colour, free).
fn bus_fit(c: &Company, p: &DayPlan, ti: usize, id: u32) -> (String, Color, bool) {
    let Some(v) = c.vehicle(id) else { return ("?".into(), TEXT_FAINT, false) };
    if !v.held_on(&p.date) {
        return (omsi_ui::tr("Gone back then").into_owned(), TEXT_FAINT, false);
    }
    if v.in_workshop(&p.date) || v.condition < 20.0 {
        return (omsi_ui::tr("In the workshop").into_owned(), DANGER.lighten(0.2), false);
    }
    let t = &p.tours[ti];
    if !co::ownline::line_allows(c, &t.tour.line, v) {
        return (omsi_ui::tr("Not one of the line's buses").into_owned(), DANGER.lighten(0.2), false);
    }
    let (a, b) = (t.tour.from(), t.tour.to());
    let other = p.tours.iter().enumerate().find(|(i, x)| *i != ti && x.bus == Some(BusOf::Own(id)) && a < x.tour.to() + co::staff::BUS_MARGIN && x.tour.from() < b + co::staff::BUS_MARGIN);
    if let Some((_, x)) = other {
        return (omsi_ui::tr("On tour %{t} then").replace("%{t}", &format!("{}/{}", x.tour.number, x.tour.tour)), WARN, false);
    }
    if t.tour.wants().is_some_and(|w| w != v.kind.size) {
        return (omsi_ui::tr("Another size than the tour asks for").into_owned(), WARN, true);
    }
    (omsi_ui::tr("Free").into_owned(), OK, true)
}

// --- giving --------------------------------------------------------------------------------

/// Give a duty its driver: into the roster, or - for one that fell out today with somebody
/// fixed for it - for today only.
fn give_driver(l: &mut Launcher, p: &DayPlan, ti: usize, k: usize, who: Option<Who>) {
    let Some(c) = l.company.company.as_ref() else { return };
    let t = &p.tours[ti];
    let (line, tour) = (t.tour.line.clone(), t.tour.tour.clone());
    let rostered = pl::roster(c, p.weekday, &line, &tour).and_then(|r| r.duties.get(k).copied().flatten());
    let today_only = p.today && t.duties[k].problem.is_some_and(|x| x != Problem::Unassigned) && rostered.is_some() && who != Some(Who::Player);
    let wd = p.weekday;
    if today_only {
        let fill = match who {
            Some(Who::Staff(id)) => Some(Fill::Colleague { id }),
            Some(Who::Agency) => Some(Fill::Agency),
            _ => Some(Fill::Drop),
        };
        let key = pl::duty_key(&line, &tour, k);
        act(l, |c| {
            pl::set_fill(c, &key, fill);
            Ok(())
        });
    } else {
        act(l, |c| {
            pl::set_driver(c, wd, &line, &tour, k, who);
            Ok(())
        });
    }
}

fn give_bus(l: &mut Launcher, p: &DayPlan, ti: usize, bus: Option<u32>) {
    let Some(c) = l.company.company.as_ref() else { return };
    let t = &p.tours[ti];
    let (line, tour) = (t.tour.line.clone(), t.tour.tour.clone());
    // (a line that asks for buses of its own runs with none other)
    if let Some((v, want)) = bus.and_then(|id| c.vehicle(id)).zip(co::ownline::vehicles_of(c, &line)).filter(|(v, _)| !co::ownline::line_allows(c, &line, v)) {
        let text = omsi_ui::tr("Line %{n} runs with %{buses}: bus %{bus} is none of them.").replace("%{n}", &t.tour.number).replace("%{buses}", &want.summary(&|s| omsi_ui::tr(s).into_owned())).replace("%{bus}", &v.number);
        kit::show(l, kit::Popup::new("directions_bus", "Not one of the line's buses", text, omsi_ui::tr("Buy or lease one of them, or choose other buses for the line in the line editor."), None));
        return;
    }
    let rostered = pl::roster(c, p.weekday, &line, &tour).and_then(|r| r.bus);
    let today_only = p.today && t.bus_problem.is_some_and(|x| x != Problem::Unassigned) && rostered.is_some();
    let wd = p.weekday;
    if today_only {
        let fill = Some(bus.map(|id| Fill::Bus { id }).unwrap_or(Fill::Drop));
        let key = pl::tour_key(&line, &tour);
        act(l, |c| {
            pl::set_fill(c, &key, fill);
            Ok(())
        });
    } else {
        act(l, |c| {
            pl::set_bus(c, wd, &line, &tour, bus);
            Ok(())
        });
    }
}

fn set_fill(l: &mut Launcher, key: String, fill: Option<Fill>) {
    act(l, |c| {
        pl::set_fill(c, &key, fill);
        Ok(())
    });
}

/// What was taken up given to what is under it.
fn give(l: &mut Launcher, p: &DayPlan, arm: Arm, hit: &Hit) {
    match (arm, hit.duty) {
        (Arm::Driver(w), Some(k)) => give_driver(l, p, hit.tour, k, Some(w)),
        (Arm::Bus(id), None) => give_bus(l, p, hit.tour, Some(id)),
        // (a bus dropped on a duty: its tour's)
        (Arm::Bus(id), Some(_)) => give_bus(l, p, hit.tour, Some(id)),
        (Arm::Driver(_), None) => {}
    }
}

/// "Drive this duty": the Drive page with exactly this duty - its line and tour, its first
/// trip and as many trips as it has (`--duty-leg`), the company's map, the plan's day, the
/// depot and the tour's bus in its paint. The player starts it there himself.
pub(super) fn drive(l: &mut Launcher, c: &Company, p: &DayPlan, ti: usize, k: usize) {
    let t = &p.tours[ti];
    let d = &t.duties[k];
    // the trip's place in its tour as the game counts it (1 the first): the day's timetable
    // has the tour's trips; the plan has them in the order they leave
    let index = l.company.planning.days.iter().find(|x| x.0 == c.map && x.1 == p.date).and_then(|x| x.2.as_ref().ok()).and_then(|lines| {
        let line = lines.iter().find(|x| x.name.eq_ignore_ascii_case(&t.tour.line))?;
        let tour = line.tours.iter().find(|x| x.number.trim() == t.tour.tour.trim())?;
        let mut trips: Vec<&core::TripInfo> = tour.trips.iter().collect();
        trips.sort_by(|a, b| a.departure.partial_cmp(&b.departure).unwrap_or(std::cmp::Ordering::Equal));
        trips.get(d.start).map(|x| x.index)
    });
    let Some(first) = index else {
        kit::show(l, kit::Popup::new("timer", "Not yet", omsi_ui::tr("The timetable of the day is still being read."), omsi_ui::tr("Try again in a moment."), None));
        return;
    };
    let (line, tour) = (t.tour.line.clone(), t.tour.tour.clone());
    let time = t.tour.trips[d.start].dep;
    let ch = &mut l.state.choice;
    ch.map = c.map.clone();
    ch.entry = -1;
    ch.date = p.date.clone();
    ch.free = false;
    ch.own_line = false;
    ch.composed = true;
    ch.line = Some(line.clone());
    ch.tour = Some(tour.clone());
    ch.time = time;
    ch.start_trip = Some((line.clone(), tour.clone(), first, time));
    ch.legs = vec![format!("{line}|{tour}|{first}|{}", d.end - d.start)];
    if let Some(v) = t.bus.and_then(|b| if let BusOf::Own(id) = b { c.vehicle(id) } else { None }).filter(|v| !v.bus.trim().is_empty()) {
        ch.bus = v.bus.clone();
        ch.paint = v.house_livery.clone().filter(|h| !h.trim().is_empty()).unwrap_or_else(|| v.livery.clone());
        ch.plate = v.plate.clone();
    }
    // (the line's own depot file, else the company's)
    let hof = c.lines.iter().find(|x| x.name.eq_ignore_ascii_case(&line)).map(|x| x.hof_or(&c.depot).to_string()).unwrap_or_else(|| c.depot.clone());
    if !hof.trim().is_empty() {
        ch.hof = hof;
        ch.hof_manual = true;
    }
    l.state.touched();
    l.state.load_lines();
    l.state.load_ibis();
    l.go(Page::Drive);
    l.drive.step = Step::Duty;
    l.state.set_status(omsi_ui::tr("Your duty is set: check the bus and start the duty when you are ready.").into_owned(), false);
}

// --- the page ------------------------------------------------------------------------------

pub fn draw(l: &mut Launcher, area: Rect) {
    let Some(c) = l.company.company.clone() else { return };
    work(l, &c);
    let day = l.company.planning.day.min(6);
    let date = co::dates::add(&c.date, day as i64);
    week_strip(l, Rect::new(area.x, area.y, area.w, 60.0), &c);
    let plan = plan_of(l, &c, &date);
    let ty = area.y + 74.0;
    tools(l, Rect::new(area.x, ty, area.w, 40.0), &c, &date);
    let body = Rect::new(area.x, ty + 40.0 + 14.0, area.w, (area.bottom() - ty - 40.0 - 14.0).max(0.0));
    let side_w = (body.w * 0.28).clamp(320.0, 440.0);
    let chart = Rect::new(body.x, body.y, (body.w - side_w - 16.0).max(200.0), body.h);
    let side = Rect::new(chart.right() + 16.0, body.y, side_w, body.h);
    match plan {
        None => {
            let inner = section(&mut l.ui, chart, "Duties");
            l.ui.text_in("Reading the timetable…", Rect::new(inner.x, inner.y, inner.w, 24.0), kit::BODY, Weight::Regular, TEXT_SOFT, Align::Left);
        }
        Some(Err(e)) => {
            let inner = section(&mut l.ui, chart, "Duties");
            l.ui.paragraph(&e, Vec2::new(inner.x, inner.y), inner.w, kit::BODY, Weight::Regular, DANGER.lighten(0.2));
        }
        Some(Ok(p)) => {
            let hits = gantt(l, chart, &c, &p);
            // (pressed on a duty or a tour's bus in the chart: taken up once the mouse moves)
            let m = l.ui.input.mouse;
            if l.ui.input.pressed && l.company.planning.drag.is_none() && l.company.planning.carry.is_none() {
                if let Some(h) = hits.iter().find(|h| h.r.contains(m)) {
                    l.company.planning.press = Some((m, h.tour, h.duty));
                }
            }
            if let Some((at, ti, duty)) = l.company.planning.press {
                if !l.ui.input.down {
                    l.company.planning.press = None;
                } else if (m - at).length() > 6.0 {
                    l.company.planning.press = None;
                    l.company.planning.carry = Some((ti, duty));
                    l.company.planning.carry_from = at;
                    // (the list to drop it on: the day's, not a duty's own)
                    l.company.planning.sel = None;
                }
            }
            l.company.planning.targets.clear();
            side_panel(l, side, &c, &p);
            // a duty or a tour carried to a driver or a bus of the list and let go
            if let Some((ti, duty)) = l.company.planning.carry {
                if l.ui.input.released {
                    l.company.planning.carry = None;
                    let target = l.company.planning.targets.iter().find(|t| t.0.contains(m)).map(|t| t.1);
                    match (target, duty) {
                        (Some(Arm::Driver(w)), Some(k)) => give_driver(l, &p, ti, k, Some(w)),
                        (Some(Arm::Bus(id)), _) => give_bus(l, &p, ti, Some(id)),
                        _ => {}
                    }
                } else if l.ui.input.down {
                    if (m - l.company.planning.carry_from).length() > 6.0 {
                        carried(&mut l.ui, &p, ti, duty);
                    }
                } else {
                    l.company.planning.carry = None;
                }
            }
            // a driver or a bus dragged here and let go
            if l.ui.input.released {
                if let Some(arm) = l.company.planning.drag.take() {
                    let at = l.ui.input.mouse;
                    if let Some(h) = hits.iter().find(|h| h.r.contains(at)) {
                        give(l, &p, arm, h);
                    }
                }
            } else if l.ui.input.down {
                if let Some(arm) = l.company.planning.drag {
                    if !side.contains(l.ui.input.mouse) {
                        ghost(&mut l.ui, &c, arm);
                    }
                }
            } else {
                l.company.planning.drag = None;
            }
        }
    }
    if l.ui.input.keys.contains(&Key::Escape) {
        l.company.planning.arm = None;
        l.company.planning.sel = None;
    }
}

/// The duty or tour being carried, at the mouse.
fn carried(ui: &mut Ui, p: &DayPlan, ti: usize, duty: Option<usize>) {
    let Some(t) = p.tours.get(ti) else { return };
    let text = line_title(t, duty);
    let m = ui.input.mouse;
    let w = (ui.width(&text, 13.5, Weight::Bold) + 22.0).min(360.0);
    let r = Rect::new(m.x + 10.0, m.y + 6.0, w, 28.0);
    ui.p().shadow(r, 6.0, 10.0, Color::rgba(0, 0, 0, 0.4));
    ui.p().rounded(r, 6.0, accent());
    ui.text_in(&text, r, 13.5, Weight::Bold, on_accent(), Align::Center);
}

/// The driver or bus being dragged, at the mouse.
fn ghost(ui: &mut Ui, c: &Company, arm: Arm) {
    let (text, fill, ink) = match arm {
        Arm::Driver(Who::Staff(id)) => (first_name(c, id), driver_colour(c, id), INK),
        Arm::Driver(Who::Player) => (omsi_ui::tr("You").into_owned(), accent(), on_accent()),
        Arm::Driver(Who::Agency) => (omsi_ui::tr("Agency").into_owned(), TEXT_FAINT, TEXT),
        Arm::Bus(id) => (c.vehicle(id).map(|v| v.number.clone()).unwrap_or_default(), FIELD, TEXT),
    };
    let m = ui.input.mouse;
    let w = ui.width(&text, 13.5, Weight::Bold) + 22.0;
    let r = Rect::new(m.x + 10.0, m.y + 6.0, w, 28.0);
    ui.p().shadow(r, 6.0, 10.0, Color::rgba(0, 0, 0, 0.4));
    ui.p().rounded(r, 6.0, fill);
    ui.text_in(&text, r, 13.5, Weight::Bold, ink, Align::Center);
}

/// The next seven days, each with whether it is covered.
fn week_strip(l: &mut Launcher, r: Rect, c: &Company) {
    let gap = 8.0;
    let w = (r.w - gap * 6.0) / 7.0;
    for k in 0..7usize {
        let date = co::dates::add(&c.date, k as i64);
        let cell = Rect::new(r.x + k as f32 * (w + gap), r.y, w, r.h);
        let on = l.company.planning.day == k;
        l.ui.card(cell);
        if l.ui.row(&format!("plan-day-{k}"), cell, on) && !on {
            let v = &mut l.company.planning;
            v.day = k;
            v.sel = None;
            v.clear_armed = false;
        }
        let ink = if on { on_accent() } else { TEXT };
        let dim = if on { on_accent() } else { TEXT_DIM };
        let Some(d) = co::dates::parse(&date) else { continue };
        let (_, m, dd) = co::dates::civil_from_days(d);
        let top = if k == 0 { omsi_ui::tr("Company day").to_uppercase() } else { omsi_ui::tr(super::WEEKDAYS[co::dates::weekday(d) as usize]).to_uppercase() };
        l.ui.text_in(&top, Rect::new(cell.x + 12.0, cell.y + 8.0, cell.w - 34.0, 16.0), kit::CAPS, Weight::Bold, dim, Align::Left);
        let big = format!("{} {} {}", omsi_ui::tr(super::WEEKDAYS[co::dates::weekday(d) as usize]), dd, omsi_ui::tr(super::MONTHS[(m as usize).clamp(1, 12) - 1]));
        l.ui.text_in(&big, Rect::new(cell.x + 12.0, cell.y + 27.0, cell.w - 24.0, 24.0), 15.5, Weight::Bold, ink, Align::Left);
        let status = plan_of(l, c, &date).and_then(|p| p.ok()).map(|p| p.counts());
        if let Some((tours, _, open, no_bus)) = status {
            let col = if tours == 0 { TEXT_FAINT } else if open + no_bus == 0 { OK } else { WARN };
            l.ui.p().circle(Vec2::new(cell.right() - 15.0, cell.y + 16.0), 5.0, col);
            let tip = if tours == 0 { omsi_ui::tr("No tours this day").into_owned() } else if open + no_bus == 0 { omsi_ui::tr("Every tour planned").into_owned() } else { omsi_ui::tr("%{n} open").replace("%{n}", &(open + no_bus).to_string()) };
            l.ui.tooltip(cell, &tip);
        }
    }
}

/// A weekday's full name (0 Monday).
fn weekday_name(d: u8) -> String {
    const NAMES: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    omsi_ui::tr(NAMES[d as usize % 7]).into_owned()
}

/// Weekdays in words: "Tuesday to Friday" (three or more in a row), else "Monday, Thursday".
fn weekdays_text(days: &[u8]) -> String {
    let in_row = days.windows(2).all(|w| w[1] == w[0] + 1);
    match days {
        [a, .., b] if days.len() >= 3 && in_row => omsi_ui::tr("%{from} to %{to}").replace("%{from}", &weekday_name(*a)).replace("%{to}", &weekday_name(*b)),
        _ => days.iter().map(|d| weekday_name(*d)).collect::<Vec<_>>().join(", "),
    }
}

/// What "Fill the roster" left open, in words ("" when nothing).
pub(super) fn open_text(f: &pl::Filled) -> String {
    let mut parts = Vec::new();
    let open = f.unqualified + f.busy;
    if open > 0 {
        let why = match (f.unqualified, f.busy) {
            (_, 0) => omsi_ui::tr("nobody is qualified for their bus").into_owned(),
            (0, _) => omsi_ui::tr("nobody qualified is free").into_owned(),
            (q, b) => omsi_ui::tr("%{q} with nobody qualified for the bus, %{b} with nobody free").replace("%{q}", &q.to_string()).replace("%{b}", &b.to_string()),
        };
        parts.push(omsi_ui::tr("%{n} duties still open: %{why}.").replace("%{n}", &open.to_string()).replace("%{why}", &why));
    }
    if f.no_bus > 0 {
        parts.push(omsi_ui::tr("%{n} tours still without a bus: none of the fleet is free for them.").replace("%{n}", &f.no_bus.to_string()));
    }
    parts.join(" ")
}

/// The tools of the day: what the roster is, fill, clear, repeat; and the dispatcher's
/// switch.
fn tools(l: &mut Launcher, r: Rect, c: &Company, date: &str) {
    let wd = pl::weekday_of(date);
    let mut x = r.right();
    let fw = Foot::width(&l.ui, "Fill the roster", Some("autorenew"));
    x -= fw;
    let fill = l.ui.button("plan-fill", Rect::new(x, r.y, fw, r.h), "Fill the roster", Some("autorenew"), ButtonKind::Normal);
    l.ui.tooltip(Rect::new(x, r.y, fw, r.h), "Fix in the roster what the dispatcher would take: free buses, and free drivers by the working-time rules. Only what is planned runs.");
    let clear_label = if l.company.planning.clear_armed { "Press again" } else { "Clear the day" };
    let cw = Foot::width(&l.ui, "Clear the day", Some("delete"));
    x -= cw + 8.0;
    let clear = l.ui.button("plan-clear", Rect::new(x, r.y, cw, r.h), clear_label, Some("delete"), ButtonKind::Normal);
    l.ui.tooltip(Rect::new(x, r.y, cw, r.h), "Take every bus and driver of this weekday out of the roster");
    let (repeat_label, to): (&str, Vec<u8>) = if wd < 5 { ("Repeat Mon–Fri", (0..5).collect()) } else { ("Repeat on the weekend", vec![5, 6]) };
    let rw = Foot::width(&l.ui, repeat_label, Some("content_copy"));
    x -= rw + 8.0;
    let repeat = l.ui.button("plan-repeat", Rect::new(x, r.y, rw, r.h), repeat_label, Some("content_copy"), ButtonKind::Normal);
    l.ui.tooltip(Rect::new(x, r.y, rw, r.h), "This weekday's roster on the other days too (theirs is replaced)");
    x -= 8.0;
    // (a fleet without the buses the tours ask for at their busiest: said instead)
    let lack = l.company.planning.days.iter().find(|d| d.0 == c.map && d.1 == date).and_then(|d| d.2.as_ref().ok()).map(|lines| co::ownline::shortfall(c, &co::network::tours_of_day(c, lines, date))).unwrap_or_default();
    let (text, colour) = match lack.first() {
        Some(s) => (
            omsi_ui::tr("At the busiest the tours ask for %{n} × %{bus} (or bigger); the fleet has %{have}.")
                .replace("%{n}", &s.needed.to_string())
                .replace("%{bus}", &omsi_ui::tr(co::BusKind { size: s.size, drive: co::Drive::Diesel }.label()))
                .replace("%{have}", &s.have.to_string()),
            WARN,
        ),
        None => (omsi_ui::tr("%{day}: the roster of this weekday repeats every week.").replace("%{day}", &day_label(date)), TEXT_DIM),
    };
    l.ui.paragraph(&text, Vec2::new(r.x, r.y + 2.0), (x - r.x - 12.0).max(0.0), kit::NOTE + 0.5, Weight::Regular, colour);
    // (each tool says what it did - or, when it could do nothing, why, in a popup)
    let day = weekday_name(wd);
    if fill {
        if let Some(lines) = l.company.planning.days.iter().find(|d| d.0 == c.map && d.1 == date).and_then(|d| d.2.as_ref().ok()).cloned() {
            let d = date.to_string();
            if let Some(f) = act(l, |c| {
                let tours = co::network::tours_of_day(c, &lines, &d);
                Ok(pl::fill_day_told(c, &d, tours))
            }) {
                let open = open_text(&f);
                if f.duties + f.buses > 0 {
                    let done = omsi_ui::tr("Filled: %{d} duties given to drivers, %{b} buses given.").replace("%{d}", &f.duties.to_string()).replace("%{b}", &f.buses.to_string());
                    l.state.set_status(if open.is_empty() { done } else { format!("{done} {open}") }, false);
                } else if open.is_empty() {
                    let text = omsi_ui::tr("Every tour of %{day} has its bus and its drivers already.").replace("%{day}", &day);
                    kit::show(l, kit::Popup::new("event", "Nothing to fill", text, "", None));
                } else {
                    let go = if f.unqualified > 0 { kit::Go::Licences } else if f.busy > 0 { kit::Go::Hire } else { kit::Go::Dealer };
                    let unlock = omsi_ui::tr("Hire drivers, train them for the buses on the Staff page, or get more buses.");
                    kit::show(l, kit::Popup::new("event", "Nothing could be filled", open, unlock, Some(go)));
                }
            }
        }
    }
    if clear {
        if l.company.planning.clear_armed {
            l.company.planning.clear_armed = false;
            if let Some(gone) = act(l, |c| Ok(pl::clear_day(c, wd))) {
                if gone.is_empty() {
                    let text = omsi_ui::tr("The roster of %{day} is empty already.").replace("%{day}", &day);
                    kit::show(l, kit::Popup::new("delete", "Nothing to clear", text, "", None));
                } else {
                    let t = omsi_ui::tr("The day is cleared: %{d} duties and %{b} buses taken off the roster of %{day}.").replace("%{d}", &gone.duties.to_string()).replace("%{b}", &gone.buses.to_string()).replace("%{day}", &day);
                    l.state.set_status(t, false);
                }
            }
        } else {
            l.company.planning.clear_armed = true;
        }
    }
    if repeat {
        if let Some(days) = act(l, |c| Ok(pl::copy_day(c, wd, &to))) {
            if days.is_empty() {
                let text = omsi_ui::tr("The roster of %{day} is empty: there is nothing to repeat. Fill it first (\"Fill the roster\" does it for you), then repeat it on the other days.").replace("%{day}", &day);
                kit::show(l, kit::Popup::new("content_copy", "Nothing to repeat", text, "", None));
            } else {
                let key = if days.len() == 1 { "The roster of %{day} is now on %{days} too." } else { "The roster of %{day} is now on %{days} too (%{n} days)." };
                let t = omsi_ui::tr(key).replace("%{day}", &day).replace("%{days}", &weekdays_text(&days)).replace("%{n}", &days.len().to_string());
                l.state.set_status(t, false);
            }
        }
    }
}

/// The day's tours as a Gantt chart: grouped by line, the bus left of each tour, its duties
/// as blocks on the hours. Returns where the duties and buses are.
fn gantt(l: &mut Launcher, r: Rect, c: &Company, p: &DayPlan) -> Vec<Hit> {
    l.ui.card(r);
    let inner = Rect::new(r.x + 16.0, r.y + 12.0, r.w - 28.0, r.h - 18.0);
    if p.tours.is_empty() {
        l.ui.paragraph("No tours on this day: the company runs no line yet, or its lines do not run on this day.", Vec2::new(inner.x, inner.y + 4.0), inner.w, kit::BODY, Weight::Regular, TEXT_SOFT);
        return Vec::new();
    }
    let left = 260.0;
    let t0 = p.tours.iter().map(|t| t.tour.from()).min().unwrap_or(300).div_euclid(60) * 60;
    let t1 = (p.tours.iter().map(|t| t.tour.to()).max().unwrap_or(1440) + 59).div_euclid(60) * 60;
    let span = (t1 - t0).max(60) as f32;
    let gx = inner.x + left;
    let gw = (inner.w - left - 14.0).max(60.0);
    let ppm = (gw / span).min(64.0 / 60.0);
    let x_of = move |m: i32| gx + (m - t0) as f32 * ppm;
    // the hours
    let head = 30.0;
    let every = if ppm * 60.0 >= 36.0 { 1 } else if ppm * 60.0 >= 18.0 { 2 } else { 3 };
    let hours: Vec<i32> = (t0 / 60..=t1 / 60).filter(|h| h % every == 0).collect();
    for &h in &hours {
        let x = x_of(h * 60);
        l.ui.text_in(&format!("{:02}", h % 24), Rect::new(x - 16.0, inner.y, 32.0, 18.0), 12.5, Weight::Bold, TEXT_DIM, Align::Center);
    }
    let armed = l.company.planning.arm;
    if let Some(a) = armed {
        let who = match a {
            Arm::Driver(w) => who_name(c, Some(w)),
            Arm::Bus(id) => c.vehicle(id).map(|v| format!("{} {}", v.number, v.name)).unwrap_or_default(),
        };
        let hint = match a {
            Arm::Driver(_) => omsi_ui::tr("Tap a duty to give it to %{who} (Esc: stop)."),
            Arm::Bus(_) => omsi_ui::tr("Tap a tour's bus to give it %{who} (Esc: stop)."),
        }
        .replace("%{who}", &who);
        l.ui.text_in(&hint, Rect::new(inner.x, inner.y - 2.0, left - 8.0, 20.0), kit::NOTE, Weight::Bold, accent_2(), Align::Left);
    }
    let rows = Rect::new(inner.x, inner.y + head - 4.0, inner.w, inner.h - head + 4.0);
    // the lines in the company's order, each with its tours
    let mut groups: Vec<(Option<co::CompanyLine>, Vec<usize>)> = c.lines.iter().map(|x| (Some(x.clone()), Vec::new())).collect();
    for (i, t) in p.tours.iter().enumerate() {
        match groups.iter_mut().find(|g| g.0.as_ref().is_some_and(|x| x.name.eq_ignore_ascii_case(&t.tour.line))) {
            Some(g) => g.1.push(i),
            None => groups.push((None, vec![i])),
        }
    }
    groups.retain(|g| !g.1.is_empty());
    let sel = l.company.planning.sel.clone();
    let mut hits: Vec<Hit> = Vec::new();
    let mut clicked: Option<(usize, Option<usize>)> = None;
    let mut start: Option<String> = None;
    let mut stop: Option<String> = None;
    let focus = l.company.planning.focus.take();
    let mut focus_y: Option<f32> = None;
    l.ui.scroll_area("plan-gantt", rows, &mut |ui, v| {
        let mut y = v.y + 4.0;
        for (line, idx) in &groups {
            let hr = Rect::new(v.x, y, v.w - 10.0, 38.0);
            if line.as_ref().is_some_and(|cl| focus.as_deref() == Some(cl.name.as_str())) {
                focus_y = Some(y - v.y);
            }
            let idle = idx.iter().all(|&i| p.tours[i].tour.unplanned);
            if ui.rect_visible(hr) {
                let mut x = hr.x;
                let covered = idx.iter().filter(|&&i| p.tours[i].covered()).count();
                let state_w = 330.0f32.min(hr.w * 0.45);
                if let Some(cl) = line {
                    x += line_plate(ui, Vec2::new(hr.x, hr.y + 6.0), cl, 24.0) + 12.0;
                    let caption = if cl.caption.is_empty() { cl.name.clone() } else { cl.caption.clone() };
                    ui.text_in(&caption, Rect::new(x, hr.y, (hr.w - state_w - (x - hr.x) - 12.0).max(40.0), hr.h), kit::ROWS, Weight::Bold, TEXT, Align::Left);
                    // its service: not yet (the button that starts it), or since when
                    let sr = Rect::new(hr.right() - state_w, hr.y + 3.0, state_w, 32.0);
                    if cl.service_from.is_none() || idle {
                        let label = omsi_ui::tr("Start service…");
                        let bw = Foot::width(ui, &label, Some("play_arrow"));
                        let b = Rect::new(sr.right() - bw, sr.y, bw, sr.h);
                        if ui.button(&format!("plan-start-{}", cl.name), b, &label, Some("play_arrow"), ButtonKind::Primary) {
                            start = Some(cl.name.clone());
                        }
                        ui.tooltip(b, "Its tours run from the moment you choose, as they are planned; what is not covered then is dropped with the contract's penalty");
                        let note = omsi_ui::tr("Not in service · %{c} of %{t} tours planned").replace("%{c}", &covered.to_string()).replace("%{t}", &idx.len().to_string());
                        ui.text_in(&note, Rect::new(sr.x - 40.0, sr.y, sr.w - bw - 14.0 + 40.0, sr.h), kit::NOTE, Weight::Medium, TEXT_DIM, Align::Right);
                    } else {
                        let since = cl.service_from.map(|s| if s <= 0 { String::new() } else { format!("{} {}", day_label(&co::clock::date_of(s)), co::clock::hhmm(s)) }).unwrap_or_default();
                        let text = if since.is_empty() { omsi_ui::tr("In service").into_owned() } else { omsi_ui::tr("In service since %{when}").replace("%{when}", &since) };
                        let cov = omsi_ui::tr("%{c} of %{t} tours covered").replace("%{c}", &covered.to_string()).replace("%{t}", &idx.len().to_string());
                        ui.text_in(&format!("{text}  ·  {cov}"), Rect::new(sr.x - 60.0, sr.y, sr.w + 24.0, sr.h), kit::NOTE, Weight::Medium, if covered == idx.len() { OK } else { WARN }, Align::Right);
                        if ui.icon_button(&format!("plan-stop-{}", cl.name), Vec2::new(sr.right() - 12.0, sr.center().y), 14.0, "pause", "Take the line out of service") {
                            stop = Some(cl.name.clone());
                        }
                    }
                }
            }
            y += 42.0;
            for &ti in idx {
                let t = &p.tours[ti];
                let row = Rect::new(v.x, y, v.w - 10.0, 38.0);
                y += 40.0;
                if !ui.rect_visible(row) {
                    continue;
                }
                ui.p().rect(Rect::new(row.x, row.bottom() + 1.0, row.w, 1.0), HAIRLINE);
                if t.tour.unplanned {
                    // (a line not in service: planned here, not run)
                    ui.p().rounded(row, 6.0, Color::WHITE.alpha(0.025));
                }
                for &h in &hours {
                    ui.p().rect(Rect::new(x_of(h * 60), row.y, 1.0, row.h), HAIRLINE.alpha(0.5));
                }
                let tour_on = sel == Some(Sel::Tour(t.tour.line.clone(), t.tour.tour.clone()));
                ui.text_in(&format!("{} {}", omsi_ui::tr("Tour"), t.tour.tour), Rect::new(row.x + 2.0, row.y, 96.0, row.h), 14.0, Weight::Bold, if t.tour.unplanned { TEXT_SOFT } else { TEXT }, Align::Left);
                // the bus
                let chip = Rect::new(row.x + 98.0, row.y + 6.0, left - 110.0, 26.0);
                let (h, _, click) = ui.interact(id_of(&format!("plan-bus-{ti}")), chip);
                if click {
                    clicked = Some((ti, None));
                }
                hits.push(Hit { r: chip, tour: ti, duty: None });
                if t.by_player {
                    ui.text_in(&omsi_ui::tr("You drove it."), chip, 13.0, Weight::Bold, accent_2(), Align::Left);
                } else {
                    let (text, fill, ink) = match t.bus {
                        Some(BusOf::Own(id)) => (c.vehicle(id).map(|x| format!("{}  {}", x.number, x.name)).unwrap_or_default(), if t.bus_from == Source::Auto { FIELD } else { HOVER }, if t.bus_from == Source::Auto { TEXT_SOFT } else { TEXT }),
                        Some(BusOf::Rental) => (omsi_ui::tr("Rental bus").into_owned(), HOVER, TEXT),
                        None => (omsi_ui::tr("No bus").into_owned(), problem_colour(t.bus_problem).alpha(0.12), problem_colour(t.bus_problem)),
                    };
                    ui.p().rounded(chip, 6.0, if h { fill.lighten(0.08) } else { fill });
                    if t.bus.is_none() || t.bus_problem.is_some() {
                        ui.p().rounded_border(chip, 6.0, 1.0, problem_colour(t.bus_problem));
                    }
                    if tour_on {
                        ui.p().rounded_border(chip, 6.0, 2.0, accent());
                    }
                    ui.text_in(&text, Rect::new(chip.x + 8.0, chip.y, chip.w - 12.0, chip.h), 13.0, Weight::Bold, ink, Align::Left);
                    ui.tooltip(chip, &omsi_ui::tr("The tour's bus: tap a bus in the list, then here - or drag it here"));
                }
                // the duties
                for (k, d) in t.duties.iter().enumerate() {
                    let br = Rect::new(x_of(d.from), row.y + 5.0, ((d.to - d.from) as f32 * ppm).max(6.0), 28.0);
                    let (h, _, click) = ui.interact(id_of(&format!("plan-duty-{ti}-{k}")), br);
                    if click {
                        clicked = Some((ti, Some(k)));
                    }
                    hits.push(Hit { r: br, tour: ti, duty: Some(k) });
                    let faint = if t.bus.is_none() && !t.by_player { 0.35 } else { 1.0 };
                    let (fill, ink, label) = if t.by_player {
                        (accent().alpha(0.5), on_accent(), omsi_ui::tr("You").into_owned())
                    } else {
                        match d.who {
                            Some(Who::Staff(id)) => {
                                let col = driver_colour(c, id);
                                (if d.from_ == Source::Auto { col.alpha(0.55) } else { col }, INK, first_name(c, id))
                            }
                            Some(Who::Player) => (accent(), on_accent(), omsi_ui::tr("You").into_owned()),
                            Some(Who::Agency) => (TEXT_FAINT, TEXT, omsi_ui::tr("Agency").into_owned()),
                            None => (problem_colour(d.problem).alpha(0.12), problem_colour(d.problem), problem_text(c, p, ti, k, d.problem)),
                        }
                    };
                    ui.p().rounded(br, 5.0, fill.alpha(faint * if h { 0.85 } else { 1.0 }));
                    if d.who.is_none() && !t.by_player {
                        ui.p().rounded_border(br, 5.0, 1.0, problem_colour(d.problem).alpha(faint));
                    } else if matches!(d.from_, Source::Dispatcher | Source::Central) {
                        ui.p().rounded_border(br, 5.0, 1.5, WARN);
                    }
                    // a late driver's first minutes
                    if let Some(late) = d.late {
                        let lw = ((late.until.min(d.to) - d.from) as f32 * ppm).max(3.0);
                        let col = if late.cover.is_some() { TEXT_DIM } else { DANGER };
                        ui.p().rounded(Rect::new(br.x, br.y, lw, br.h), 5.0, col.alpha(0.6));
                    }
                    if !d.warn.is_empty() {
                        ui.p().circle(Vec2::new(br.right() - 5.0, br.y + 5.0), 3.0, WARN);
                    }
                    if sel == Some(Sel::Duty(t.tour.line.clone(), t.tour.tour.clone(), k)) {
                        ui.p().rounded_border(br.inset(-2.0), 6.0, 2.0, TEXT);
                    }
                    // its depot and empty runs, shaded: run with it, without passengers
                    for x in t.tour.trips[d.start..d.end].iter().filter(|x| x.empty) {
                        let a = x_of(x.dep.max(d.from));
                        let b = x_of(x.arr.min(d.to)).max(a + 2.0);
                        ui.p().rect(Rect::new(a, br.y + br.h - 6.0, b - a, 6.0), Color::rgba(0, 0, 0, 0.45));
                        ui.p().rect(Rect::new(a, br.y + br.h - 6.0, b - a, 1.0), TEXT_FAINT.alpha(0.6));
                    }
                    if br.w > 34.0 {
                        ui.text_in(&label, Rect::new(br.x + 6.0, br.y, br.w - 10.0, br.h), 12.5, Weight::Bold, ink, Align::Left);
                    }
                    let tip = format!("{} – {}  ·  {}", hhmm(d.from), hhmm(d.to), who_name(c, if t.by_player { Some(Who::Player) } else { d.who }));
                    ui.tooltip(br, &tip);
                }
            }
            y += 8.0;
        }
        y - v.y
    });
    if let Some(fy) = focus_y {
        l.ui.scroll_to("plan-gantt", fy, 40.0, rows.h);
    }
    if let Some(name) = start {
        l.company.dialog = Some(Dialog::Service { line: name, when: 0, date: c.date.clone(), gaps: false });
    }
    if let Some(name) = stop {
        act(l, |c| co::network::stop_service(c, &name));
    }
    if let Some((ti, duty)) = clicked {
        let t = &p.tours[ti];
        match (l.company.planning.arm, duty) {
            (Some(Arm::Driver(w)), Some(k)) if !t.by_player => give_driver(l, p, ti, k, Some(w)),
            (Some(Arm::Bus(id)), _) if !t.by_player => give_bus(l, p, ti, Some(id)),
            (_, Some(k)) => {
                let s = Sel::Duty(t.tour.line.clone(), t.tour.tour.clone(), k);
                let v = &mut l.company.planning;
                v.sel = if v.sel.as_ref() == Some(&s) { None } else { Some(s) };
            }
            (_, None) => {
                let s = Sel::Tour(t.tour.line.clone(), t.tour.tour.clone());
                let v = &mut l.company.planning;
                v.sel = if v.sel.as_ref() == Some(&s) { None } else { Some(s) };
            }
        }
    }
    hits
}

// --- the side ------------------------------------------------------------------------------

fn side_panel(l: &mut Launcher, r: Rect, c: &Company, p: &DayPlan) {
    let sel = l.company.planning.sel.clone();
    let find = |line: &str, tour: &str| p.tours.iter().position(|t| t.tour.line == line && t.tour.tour == tour);
    match sel {
        Some(Sel::Duty(line, tour, k)) => match find(&line, &tour).filter(|&i| k < p.tours[i].duties.len()) {
            Some(ti) => duty_panel(l, r, c, p, ti, k),
            None => day_panel(l, r, c, p),
        },
        Some(Sel::Tour(line, tour)) => match find(&line, &tour) {
            Some(ti) => tour_panel(l, r, c, p, ti),
            None => day_panel(l, r, c, p),
        },
        None => day_panel(l, r, c, p),
    }
}

/// A list row with a name, what it says on the right, and a swatch; returns (clicked,
/// pressed on it).
fn pick_row(ui: &mut Ui, name: &str, r: Rect, on: bool, swatch: Option<Color>, text: &str, right: &str, right_c: Color) -> (bool, bool) {
    let pressed = ui.hover(r) && ui.input.pressed;
    let clicked = ui.row(name, r, on);
    let mut x = r.x + 10.0;
    if let Some(s) = swatch {
        ui.p().rounded(Rect::new(x, r.y + r.h * 0.5 - 6.0, 12.0, 12.0), 3.0, s);
        x += 20.0;
    }
    let ink = if on { on_accent() } else { TEXT };
    let rw = ui.width(right, 13.0, Weight::Regular).min(r.w * 0.5);
    ui.text_in(text, Rect::new(x, r.y, r.right() - x - rw - 16.0, r.h), 14.0, Weight::Medium, ink, Align::Left);
    ui.text_in(right, Rect::new(r.right() - rw - 10.0, r.y, rw, r.h), 13.0, Weight::Regular, if on { on_accent() } else { right_c }, Align::Right);
    (clicked, pressed)
}

/// Nothing chosen: the day in short, what fell out, and the drivers and buses to give.
fn day_panel(l: &mut Launcher, r: Rect, c: &Company, p: &DayPlan) {
    let inner = section(&mut l.ui, r, if p.today { "The company's day" } else { "This day" });
    let (tours, covered, open, no_bus) = p.counts();
    let rows = p.open_rows();
    let arm = l.company.planning.arm;
    // what the morning brought, and what is open
    let mut morning: Vec<(String, Color)> = Vec::new();
    for d in &p.disruptions {
        match *d {
            Disruption::Late { employee, minutes } => morning.push((omsi_ui::tr("%{name} comes %{n} minutes late.").replace("%{name}", &who_name(c, Some(Who::Staff(employee)))).replace("%{n}", &minutes.to_string()), WARN)),
            Disruption::Breakdown { vehicle, cost } => morning.push((
                omsi_ui::tr("Bus %{n} did not start: towed to the workshop (%{amount}).").replace("%{n}", &c.vehicle(vehicle).map(|v| v.number.clone()).unwrap_or_default()).replace("%{amount}", &eur(cost)),
                DANGER.lighten(0.2),
            )),
        }
    }
    let staff: Vec<(u32, String, Color, String, Color)> = c
        .staff
        .iter()
        .map(|e| {
            let mins: i32 = p.duties_of(Who::Staff(e.id)).iter().map(|&(i, k)| p.tours[i].duties[k].to - p.tours[i].duties[k].from).sum();
            let (state, col) = if !e.employed_on(&p.date) {
                (omsi_ui::tr("Not employed then").into_owned(), TEXT_FAINT)
            } else if e.sick_until.as_deref().is_some_and(|u| co::dates::between(&p.date, u) >= 0) {
                (omsi_ui::tr("Ill").into_owned(), DANGER.lighten(0.2))
            } else if e.holiday_until.as_deref().is_some_and(|u| co::dates::between(&p.date, u) >= 0) {
                (omsi_ui::tr("On holiday").into_owned(), TEXT_DIM)
            } else if mins == 0 {
                (omsi_ui::tr("Free").into_owned(), TEXT_DIM)
            } else {
                (length(mins), if mins > co::staff::DAY_TARGET { WARN } else { TEXT_SOFT })
            };
            (e.id, e.name.clone(), driver_colour(c, e.id), state, col)
        })
        .collect();
    let buses: Vec<(u32, String, String, Color)> = c
        .fleet
        .iter()
        .map(|v| {
            let n = p.tours.iter().filter(|t| t.bus == Some(BusOf::Own(v.id))).count();
            let (state, col) = if !v.held_on(&p.date) {
                (omsi_ui::tr("Gone back then").into_owned(), TEXT_FAINT)
            } else if v.in_workshop(&p.date) || p.disruptions.iter().any(|d| matches!(d, Disruption::Breakdown { vehicle, .. } if *vehicle == v.id)) {
                (omsi_ui::tr("In the workshop").into_owned(), DANGER.lighten(0.2))
            } else if n == 0 {
                (omsi_ui::tr("Free").into_owned(), TEXT_DIM)
            } else {
                (omsi_ui::tr("%{n} tours").replace("%{n}", &n.to_string()), TEXT_SOFT)
            };
            (v.id, format!("{}  {}", v.number, v.name), state, col)
        })
        .collect();
    let open_rows: Vec<(usize, Option<usize>, String, String, Color)> = rows
        .iter()
        .map(|o| {
            let t = &p.tours[o.tour];
            let what = match o.duty {
                _ if o.late => omsi_ui::tr("Late start").into_owned(),
                Some(k) => problem_text(c, p, o.tour, k, o.problem),
                None => omsi_ui::tr(o.problem.map(Problem::label).unwrap_or("No driver")).into_owned(),
            };
            let fill = if o.filled {
                match (o.duty, o.late) {
                    (None, _) => match t.bus {
                        Some(BusOf::Rental) => omsi_ui::tr("Rental bus").into_owned(),
                        Some(BusOf::Own(id)) => c.vehicle(id).map(|v| v.number.clone()).unwrap_or_default(),
                        None => String::new(),
                    },
                    (Some(k), true) => t.duties[k].late.and_then(|x| x.cover).map(|id| who_name(c, Some(Who::Staff(id)))).unwrap_or_default(),
                    (Some(k), false) => who_name(c, t.duties[k].who),
                }
            } else {
                omsi_ui::tr("Trips dropped").into_owned()
            };
            (o.tour, o.duty, format!("{}  ·  {}", line_title(t, o.duty), what), fill, if o.filled { TEXT_SOFT } else { problem_colour(o.problem) })
        })
        .collect();
    let mut tapped: Option<Arm> = None;
    let mut pressed: Option<Arm> = None;
    let mut open_pick: Option<(usize, Option<usize>)> = None;
    let mut open_press: Option<(usize, Option<usize>)> = None;
    let mut targets: Vec<(Rect, Arm)> = Vec::new();
    // (a duty or tour carried: the drivers and buses it could go to light up under the mouse)
    let carrying = l.company.planning.carry.map(|c| c.1.is_some());
    let clip = inner;
    let summary = omsi_ui::tr("%{c} of %{t} tours covered").replace("%{c}", &covered.to_string()).replace("%{t}", &tours.to_string());
    let gaps = if open + no_bus == 0 { omsi_ui::tr("Nothing open.").into_owned() } else { omsi_ui::tr("%{d} duties without a driver, %{b} tours without a bus.").replace("%{d}", &open.to_string()).replace("%{b}", &no_bus.to_string()) };
    l.ui.scroll_area("plan-side", inner, &mut |ui, v| {
        let mut y = v.y;
        ui.text_in(&summary, Rect::new(v.x, y, v.w, 24.0), kit::HEAD, Weight::Bold, TEXT, Align::Left);
        y += 28.0;
        ui.text_in(&gaps, Rect::new(v.x, y, v.w, 20.0), kit::NOTE, Weight::Regular, if open + no_bus == 0 { OK } else { WARN }, Align::Left);
        y += 36.0;
        let caps = |ui: &mut Ui, y: f32, t: &str| kit::caps(ui, Rect::new(v.x, y, v.w, 16.0), t);
        if !morning.is_empty() {
            caps(ui, y, "This morning");
            y += 24.0;
            for (t, col) in &morning {
                ui.p().circle(Vec2::new(v.x + 4.0, y + 8.0), 3.0, *col);
                let h = ui.paragraph(t, Vec2::new(v.x + 14.0, y), v.w - 24.0, kit::NOTE, Weight::Regular, TEXT_SOFT);
                y += h.max(18.0) + 6.0;
            }
            y += 8.0;
        }
        if !open_rows.is_empty() {
            caps(ui, y, "Open duties");
            y += 24.0;
            for (k, (ti, duty, title, fill, col)) in open_rows.iter().enumerate() {
                let rr = Rect::new(v.x, y, v.w - 8.0, 50.0);
                if ui.hover(rr) && ui.input.pressed {
                    open_press = Some((*ti, *duty));
                }
                if ui.row(&format!("plan-open-{k}"), rr, false) {
                    open_pick = Some((*ti, *duty));
                }
                ui.icon("open_with", Vec2::new(rr.right() - 14.0, rr.y + 25.0), 16.0, TEXT_FAINT);
                ui.text_in(title, Rect::new(rr.x + 8.0, rr.y + 4.0, rr.w - 16.0, 21.0), 13.5, Weight::Bold, TEXT, Align::Left);
                ui.text_in(&format!("→ {fill}"), Rect::new(rr.x + 8.0, rr.y + 26.0, rr.w - 16.0, 20.0), 13.5, Weight::Regular, *col, Align::Left);
                y += 52.0;
            }
            y += 8.0;
        }
        caps(ui, y, "Drivers");
        y += 22.0;
        ui.text_in(&omsi_ui::tr("Tap one, then a duty - or drag it there; or drag a duty here."), Rect::new(v.x, y, v.w, 20.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
        y += 26.0;
        let me = Rect::new(v.x, y, v.w - 8.0, 34.0);
        let mut target = |ui: &mut Ui, r: Rect, a: Arm| {
            let fits = match (carrying, a) {
                (Some(true), _) => true,
                (Some(false), Arm::Bus(_)) => true,
                _ => false,
            };
            if fits {
                let (x0, y0) = (r.x.max(clip.x), r.y.max(clip.y));
                let shown = Rect::new(x0, y0, r.right().min(clip.right()) - x0, r.bottom().min(clip.bottom()) - y0);
                if shown.w > 0.0 && shown.h > 0.0 {
                    targets.push((shown, a));
                    if ui.hover(r) {
                        ui.p().rounded_border(r, RADIUS, 2.0, accent());
                    }
                }
            }
        };
        target(ui, me, Arm::Driver(Who::Player));
        let (cl, pr) = pick_row(ui, "plan-me", me, arm == Some(Arm::Driver(Who::Player)), Some(accent()), &omsi_ui::tr("You"), &omsi_ui::tr("Your own duties"), TEXT_DIM);
        if cl {
            tapped = Some(Arm::Driver(Who::Player));
        }
        if pr {
            pressed = Some(Arm::Driver(Who::Player));
        }
        y += 36.0;
        for (id, name, col, state, state_c) in &staff {
            let rr = Rect::new(v.x, y, v.w - 8.0, 34.0);
            let a = Arm::Driver(Who::Staff(*id));
            target(ui, rr, a);
            let (cl, pr) = pick_row(ui, &format!("plan-driver-{id}"), rr, arm == Some(a), Some(*col), name, state, *state_c);
            if cl {
                tapped = Some(a);
            }
            if pr {
                pressed = Some(a);
            }
            y += 36.0;
        }
        if staff.is_empty() {
            ui.text_in(&omsi_ui::tr("Nobody on the payroll yet."), Rect::new(v.x, y, v.w, 20.0), kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Left);
            y += 22.0;
        }
        y += 10.0;
        caps(ui, y, "Buses");
        y += 24.0;
        for (id, name, state, state_c) in &buses {
            let rr = Rect::new(v.x, y, v.w - 8.0, 34.0);
            let a = Arm::Bus(*id);
            target(ui, rr, a);
            let (cl, pr) = pick_row(ui, &format!("plan-bus-pick-{id}"), rr, arm == Some(a), None, name, state, *state_c);
            if cl {
                tapped = Some(a);
            }
            if pr {
                pressed = Some(a);
            }
            y += 36.0;
        }
        if buses.is_empty() {
            ui.text_in(&omsi_ui::tr("No bus in the fleet yet."), Rect::new(v.x, y, v.w, 20.0), kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Left);
            y += 22.0;
        }
        y - v.y + 8.0
    });
    let mouse = l.ui.input.mouse;
    let view = &mut l.company.planning;
    view.targets = targets;
    if let Some(a) = pressed {
        view.drag = Some(a);
    }
    if let Some(o) = open_press {
        view.carry = Some(o);
        view.carry_from = mouse;
    }
    if let Some(a) = tapped {
        view.arm = if view.arm == Some(a) { None } else { Some(a) };
        view.drag = None;
    }
    if let Some((ti, duty)) = open_pick {
        // (a tap, not a carry)
        view.carry = None;
        let t = &p.tours[ti];
        view.sel = Some(match duty {
            Some(k) => Sel::Duty(t.tour.line.clone(), t.tour.tour.clone(), k),
            None => Sel::Tour(t.tour.line.clone(), t.tour.tour.clone()),
        });
    }
}

/// A duty chosen: its times, who drives it and why, who else could; on the company's day
/// what to do when it is open, and "Drive this duty".
fn duty_panel(l: &mut Launcher, r: Rect, c: &Company, p: &DayPlan, ti: usize, k: usize) {
    let t = p.tours[ti].clone();
    let d = t.duties[k].clone();
    let inner = section(&mut l.ui, r, "Duty");
    if l.ui.icon_button("plan-sel-close", Vec2::new(r.right() - 24.0, r.y + 22.0), 16.0, "close", "Back to the day") {
        l.company.planning.sel = None;
        return;
    }
    let key = pl::duty_key(&t.tour.line, &t.tour.tour, k);
    let fill_now = c.planning.date == c.date && c.planning.fills.iter().any(|f| f.key == key);
    let late_key = pl::late_key(&t.tour.line, &t.tour.tour, k);
    let late_fill = c.planning.date == c.date && c.planning.fills.iter().any(|f| f.key == late_key);
    let first = &t.tour.trips[d.start];
    let last = &t.tour.trips[d.end - 1];
    let mut facts: Vec<(String, Color)> = vec![
        (format!("{} – {}  ·  {}  ·  {} {}", hhmm(d.from), hhmm(d.to), length(d.to - d.from), d.end - d.start, omsi_ui::tr("trips")), TEXT_SOFT),
        (format!("{} → {}", first.from, last.to), TEXT_DIM),
    ];
    let who = if t.by_player { omsi_ui::tr("You drove it.").into_owned() } else { format!("{}: {}", omsi_ui::tr("Driven by"), who_name(c, d.who)) };
    facts.push((who, TEXT));
    if !source_text(d.from_).is_empty() && d.who.is_some() {
        facts.push((omsi_ui::tr(source_text(d.from_)).into_owned(), TEXT_DIM));
    }
    if let Some(pb) = d.problem {
        facts.push((format!("{}: {}", omsi_ui::tr("Open because"), omsi_ui::tr(pb.label())), problem_colour(Some(pb))));
    }
    for w in &d.warn {
        let s = match w {
            Warn::Overtime(m) => omsi_ui::tr("Overtime %{t}").replace("%{t}", &length(*m)),
            Warn::Transfer(m) => omsi_ui::tr("%{n} min to change stops").replace("%{n}", &m.to_string()),
            Warn::Experience => omsi_ui::tr("Little experience for this bus").into_owned(),
        };
        facts.push((s, WARN));
    }
    if let Some(late) = d.late {
        let s = match late.cover {
            Some(id) => omsi_ui::tr("The driver is late until %{t}: %{name} drives the first trips.").replace("%{t}", &hhmm(late.until)).replace("%{name}", &who_name(c, Some(Who::Staff(id)))),
            None => omsi_ui::tr("The driver is late until %{t}: the trips before are dropped.").replace("%{t}", &hhmm(late.until)),
        };
        facts.push((s, if late.cover.is_some() { WARN } else { DANGER.lighten(0.2) }));
    }
    let mut y = inner.y;
    l.ui.text_in(&line_title(&t, Some(k)), Rect::new(inner.x, y, inner.w - 20.0, 24.0), 16.0, Weight::Bold, TEXT, Align::Left);
    y += 32.0;
    for (s, col) in &facts {
        let h = l.ui.paragraph(s, Vec2::new(inner.x, y), inner.w, kit::NOTE + 0.5, Weight::Regular, *col);
        y += h.max(19.0) + 5.0;
    }
    y += 8.0;
    // on the company's day: drive it, and what to do while it is open
    let foot_h = if p.today && !t.by_player { 50.0 } else { 0.0 };
    if p.today && !t.by_player {
        let b = Rect::new(inner.x, inner.bottom() - 40.0, inner.w, 40.0);
        if l.ui.button("plan-drive", b, "Drive this duty", Some("play_arrow"), ButtonKind::Primary) {
            drive(l, c, p, ti, k);
            return;
        }
    }
    let mut options: Vec<(String, Option<Fill>, bool)> = Vec::new();
    if p.today && !t.by_player {
        if d.problem.is_some() || d.who.is_none() {
            options.push((omsi_ui::tr("An agency driver for today").into_owned(), Some(Fill::Agency), true));
            options.push((omsi_ui::tr("Drop its trips today").into_owned(), Some(Fill::Drop), true));
            if fill_now {
                options.push((omsi_ui::tr("Leave it to the central").into_owned(), None, true));
            }
        }
        if d.late.is_some() {
            options.push((omsi_ui::tr("Drop the first trips").into_owned(), Some(Fill::Drop), false));
            if late_fill {
                options.push((omsi_ui::tr("Let the central cover the first trips").into_owned(), None, false));
            }
        }
    }
    let cands: Vec<(Option<Who>, String, String, Color, Option<Color>)> = {
        let mut v: Vec<(Option<Who>, String, String, Color, Option<Color>)> = Vec::new();
        v.push((Some(Who::Player), omsi_ui::tr("You").into_owned(), omsi_ui::tr("Drive it yourself").into_owned(), TEXT_DIM, Some(accent())));
        let mut people: Vec<(bool, u32, String, String, Color)> = c
            .staff
            .iter()
            .map(|e| {
                let (s, col, ok) = fit_of(c, p, ti, k, e.id);
                (ok, e.id, e.name.clone(), s, col)
            })
            .collect();
        people.sort_by(|a, b| b.0.cmp(&a.0).then(a.2.cmp(&b.2)));
        for (_, id, name, s, col) in people {
            v.push((Some(Who::Staff(id)), name, s, col, Some(driver_colour(c, id))));
        }
        v.push((None, omsi_ui::tr("Nobody").into_owned(), String::new(), TEXT_DIM, None));
        v
    };
    let list = Rect::new(inner.x, y, inner.w, (inner.bottom() - foot_h - y).max(0.0));
    let current = if t.by_player { Some(Who::Player) } else { d.who };
    let mut pick: Option<Option<Who>> = None;
    let mut opt: Option<(Option<Fill>, bool)> = None;
    l.ui.scroll_area("plan-duty-side", list, &mut |ui, v| {
        let mut yy = v.y;
        if !options.is_empty() {
            kit::caps(ui, Rect::new(v.x, yy, v.w, 16.0), "Today");
            yy += 24.0;
            for (n, (label, fill, whole)) in options.iter().enumerate() {
                let rr = Rect::new(v.x, yy, v.w - 8.0, 34.0);
                if pick_row(ui, &format!("plan-opt-{n}"), rr, false, None, label, "", TEXT_DIM).0 {
                    opt = Some((*fill, *whole));
                }
                yy += 36.0;
            }
            yy += 10.0;
        }
        kit::caps(ui, Rect::new(v.x, yy, v.w, 16.0), "Who drives it");
        yy += 24.0;
        for (n, (w, name, s, col, sw)) in cands.iter().enumerate() {
            let rr = Rect::new(v.x, yy, v.w - 8.0, 34.0);
            if pick_row(ui, &format!("plan-cand-{n}"), rr, *w == current && w.is_some(), *sw, name, s, *col).0 {
                pick = Some(*w);
            }
            yy += 32.0;
        }
        yy - v.y + 8.0
    });
    if let Some(w) = pick {
        if !t.by_player {
            give_driver(l, p, ti, k, w);
        }
    }
    if let Some((fill, whole)) = opt {
        set_fill(l, if whole { key } else { late_key }, fill);
    }
}

/// A tour chosen: its bus and which others could run it; on the company's day a rental bus
/// or dropping it.
fn tour_panel(l: &mut Launcher, r: Rect, c: &Company, p: &DayPlan, ti: usize) {
    let t = p.tours[ti].clone();
    let inner = section(&mut l.ui, r, "Tour");
    if l.ui.icon_button("plan-sel-close", Vec2::new(r.right() - 24.0, r.y + 22.0), 16.0, "close", "Back to the day") {
        l.company.planning.sel = None;
        return;
    }
    let mut y = inner.y;
    l.ui.text_in(&line_title(&t, None), Rect::new(inner.x, y, inner.w - 20.0, 24.0), 16.0, Weight::Bold, TEXT, Align::Left);
    y += 32.0;
    let mut facts: Vec<(String, Color)> = vec![(format!("{} – {}  ·  {} km  ·  {} {}", hhmm(t.tour.from()), hhmm(t.tour.to()), t.tour.km().round(), t.duties.len(), omsi_ui::tr("duties")), TEXT_SOFT)];
    let bus = match t.bus {
        Some(BusOf::Own(id)) => c.vehicle(id).map(|v| format!("{} {}", v.number, v.name)).unwrap_or_default(),
        Some(BusOf::Rental) => omsi_ui::tr("Rental bus").into_owned(),
        None => omsi_ui::tr("No bus").into_owned(),
    };
    facts.push((format!("{}: {}", omsi_ui::tr("Bus"), bus), TEXT));
    if t.bus.is_some() && !source_text(t.bus_from).is_empty() {
        facts.push((omsi_ui::tr(source_text(t.bus_from)).into_owned(), TEXT_DIM));
    }
    if let Some(pb) = t.bus_problem {
        facts.push((format!("{}: {}", omsi_ui::tr("Open because"), omsi_ui::tr(pb.label())), problem_colour(Some(pb))));
    }
    if let Some(w) = t.tour.wants() {
        facts.push((omsi_ui::tr("The tour asks for: %{kind}").replace("%{kind}", &omsi_ui::tr(co::BusKind { size: w, drive: co::Drive::Diesel }.label())), TEXT_DIM));
    }
    for (s, col) in &facts {
        let h = l.ui.paragraph(s, Vec2::new(inner.x, y), inner.w, kit::NOTE + 0.5, Weight::Regular, *col);
        y += h.max(19.0) + 5.0;
    }
    y += 8.0;
    let key = pl::tour_key(&t.tour.line, &t.tour.tour);
    let fill_now = c.planning.date == c.date && c.planning.fills.iter().any(|f| f.key == key);
    let mut options: Vec<(String, Option<Fill>)> = Vec::new();
    if p.today && !t.by_player && (t.bus.is_none() || t.bus_problem.is_some()) {
        let rent = co::economy::rent_per_day(co::BusKind { size: t.tour.wants().unwrap_or_default(), drive: co::Drive::Diesel }, &co::economy::rules(c.difficulty), c.price_index);
        options.push((omsi_ui::tr("Rent a bus for today (%{amount})").replace("%{amount}", &eur(rent)), Some(Fill::Rental)));
        options.push((omsi_ui::tr("Drop the tour today").into_owned(), Some(Fill::Drop)));
        if fill_now {
            options.push((omsi_ui::tr("Leave it to the central").into_owned(), None));
        }
    }
    let mut cands: Vec<(bool, Option<u32>, String, String, Color)> = c
        .fleet
        .iter()
        .map(|v| {
            let (s, col, ok) = bus_fit(c, p, ti, v.id);
            (ok, Some(v.id), format!("{}  {}", v.number, v.name), s, col)
        })
        .collect();
    cands.sort_by(|a, b| b.0.cmp(&a.0));
    cands.push((true, None, omsi_ui::tr("No bus").into_owned(), String::new(), TEXT_DIM));
    let current = match t.bus {
        Some(BusOf::Own(id)) => Some(id),
        _ => None,
    };
    let list = Rect::new(inner.x, y, inner.w, (inner.bottom() - y).max(0.0));
    let mut pick: Option<Option<u32>> = None;
    let mut opt: Option<Option<Fill>> = None;
    let by_player = t.by_player;
    l.ui.scroll_area("plan-tour-side", list, &mut |ui, v| {
        let mut yy = v.y;
        if !options.is_empty() {
            kit::caps(ui, Rect::new(v.x, yy, v.w, 16.0), "Today");
            yy += 24.0;
            for (n, (label, fill)) in options.iter().enumerate() {
                let rr = Rect::new(v.x, yy, v.w - 8.0, 34.0);
                if pick_row(ui, &format!("plan-topt-{n}"), rr, false, None, label, "", TEXT_DIM).0 {
                    opt = Some(*fill);
                }
                yy += 36.0;
            }
            yy += 10.0;
        }
        if !by_player {
            kit::caps(ui, Rect::new(v.x, yy, v.w, 16.0), "Which bus runs it");
            yy += 24.0;
            for (n, (_, id, name, s, col)) in cands.iter().enumerate() {
                let rr = Rect::new(v.x, yy, v.w - 8.0, 34.0);
                if pick_row(ui, &format!("plan-bcand-{n}"), rr, id.is_some() && *id == current, None, name, s, *col).0 {
                    pick = Some(*id);
                }
                yy += 36.0;
            }
        }
        yy - v.y + 8.0
    });
    if let Some(b) = pick {
        give_bus(l, p, ti, b);
    }
    if let Some(fill) = opt {
        set_fill(l, key, fill);
    }
}

// --- putting a line into service ----------------------------------------------------------------

/// "Start service from": now, from tomorrow, or from a date; what the next seven days of the
/// line are planned like, and - with gaps - the player's word that he starts with them.
pub(super) fn service_dialog(l: &mut Launcher) {
    let Some(Dialog::Service { line, when, date, gaps }) = &l.company.dialog else { return };
    let (line, when, date, gaps) = (line.clone(), *when, date.clone(), *gaps);
    let Some(c) = l.company.company.clone() else { return };
    let Some(cl) = c.lines.iter().find(|x| x.name == line).cloned() else {
        l.company.dialog = None;
        return;
    };
    work(l, &c);
    let title = omsi_ui::tr("Start the service of line %{n}").replace("%{n}", &cl.number);
    let f = kit::frame(l, 760.0, 640.0, "play_arrow", &title);
    let inner = f.body;
    let mut y = inner.y;
    y += l.ui.paragraph("From the moment you choose, the line's tours run as they are planned. A tour without its bus or a driver is dropped then, with the contract's penalty - plan first, then start.", Vec2::new(inner.x, y), inner.w, kit::BODY, Weight::Regular, TEXT_SOFT) + 16.0;
    kit::caps(&mut l.ui, Rect::new(inner.x, y, inner.w, 16.0), "From when");
    y += 26.0;
    let labels: Vec<String> = ["Now", "From tomorrow", "From a date"].iter().map(|s| omsi_ui::tr(s).into_owned()).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let mut w = when;
    l.ui.segmented("plan-service-when", Rect::new(inner.x, y, inner.w.min(520.0), 40.0), &mut w, &refs);
    let mut d = date.clone();
    if w == 2 {
        l.ui.date_field("plan-service-date", Rect::new(inner.x + inner.w.min(520.0) + 14.0, y, (inner.w - inner.w.min(520.0) - 14.0).max(160.0), 40.0), &mut d);
    }
    y += 56.0;
    let from = match w {
        0 => co::clock::now(&c),
        1 => co::clock::moment(&co::dates::add(&c.date, 1), 0),
        _ => co::clock::moment(&d, 0).max(co::clock::now(&c)),
    };
    // the next seven days of the line as planned
    kit::caps(&mut l.ui, Rect::new(inner.x, y, inner.w, 16.0), "The line's week as it is planned");
    y += 26.0;
    let mut short = 0usize;
    let mut reading = false;
    let start_day = co::clock::date_of(from);
    for k in 0..7 {
        let day = co::dates::add(&start_day, k);
        let row = Rect::new(inner.x, y, inner.w, 30.0);
        l.ui.text_in(&day_label(&day), Rect::new(row.x, row.y, 220.0, row.h), kit::ROWS, Weight::Medium, TEXT, Align::Left);
        let (text, colour) = match plan_of(l, &c, &day) {
            None => {
                reading = true;
                (omsi_ui::tr("Reading the timetable…").into_owned(), TEXT_DIM)
            }
            Some(Err(_)) => (omsi_ui::tr("The timetable could not be read.").into_owned(), WARN),
            Some(Ok(p)) => {
                let mine: Vec<&DayTour> = p.tours.iter().filter(|t| t.tour.line == line && co::clock::moment(&day, t.tour.from() as i64) >= from).collect();
                let covered = mine.iter().filter(|t| t.covered()).count();
                short += mine.len() - covered;
                if mine.is_empty() {
                    (omsi_ui::tr("No tours").into_owned(), TEXT_DIM)
                } else {
                    let t = omsi_ui::tr("%{c} of %{t} tours planned").replace("%{c}", &covered.to_string()).replace("%{t}", &mine.len().to_string());
                    (t, kit::share_colour(covered as f64 / mine.len() as f64, false))
                }
            }
        };
        l.ui.text_in(&text, Rect::new(row.x + 230.0, row.y, row.w - 230.0, row.h), kit::ROWS, Weight::Bold, colour, Align::Left);
        l.ui.p().rect(Rect::new(row.x, row.bottom(), row.w, 1.0), HAIRLINE);
        y += 32.0;
    }
    y += 12.0;
    let mut g = gaps;
    if short > 0 {
        let t = omsi_ui::tr("%{n} tours of these days have no bus or no driver yet: they would be dropped, each with its penalty.").replace("%{n}", &short.to_string());
        y += l.ui.paragraph(&t, Vec2::new(inner.x, y), inner.w, kit::BODY, Weight::Medium, WARN) + 8.0;
        l.ui.toggle("plan-service-gaps", Rect::new(inner.x, y, inner.w, ROW), &mut g, "Start all the same, with these gaps");
    }
    let mut foot = Foot::new(&f);
    let go = foot.right(l, "plan-service-go", "Start service", Some("play_arrow"), ButtonKind::Primary);
    if foot.right(l, "plan-service-cancel", "Cancel", None, ButtonKind::Normal) || f.close {
        l.company.dialog = None;
        return;
    }
    if go {
        if reading {
            kit::show(l, kit::Popup::new("timer", "Not yet", omsi_ui::tr("The line's week is still being read."), omsi_ui::tr("Try again in a moment."), None));
        } else if short > 0 && !g {
            kit::show(l, kit::Popup::new("event", "Not planned yet", omsi_ui::tr("Some of the line's tours have no bus or no driver: they would be dropped, each with the contract's penalty."), omsi_ui::tr("Give them buses and drivers on the planning (\"Fill the roster\" does it for you) - or choose to start with the gaps."), None));
        } else if let Some(at) = act(l, |c| co::network::start_service(c, &line, from)) {
            l.company.dialog = None;
            l.state.set_status(omsi_ui::tr("Line %{n} is in service from %{when}.").replace("%{n}", &cl.number).replace("%{when}", &format!("{} {}", day_label(&co::clock::date_of(at)), co::clock::hhmm(at))), false);
            return;
        }
    }
    l.company.dialog = Some(Dialog::Service { line, when: w, date: d, gaps: g });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tools_say_what_they_did() {
        assert_eq!(weekdays_text(&[1, 2, 3, 4]), "Tuesday to Friday");
        assert_eq!(weekdays_text(&[0, 1, 3, 4]), "Monday, Tuesday, Thursday, Friday");
        assert_eq!(weekdays_text(&[6]), "Sunday");
        let f = pl::Filled { duties: 9, buses: 4, unqualified: 2, busy: 0, no_bus: 0 };
        assert_eq!(open_text(&f), "2 duties still open: nobody is qualified for their bus.");
        assert_eq!(open_text(&pl::Filled::default()), "");
        let keys = [
            "%{from} to %{to}",
            "Filled: %{d} duties given to drivers, %{b} buses given.",
            "%{n} duties still open: %{why}.",
            "%{q} with nobody qualified for the bus, %{b} with nobody free",
            "The day is cleared: %{d} duties and %{b} buses taken off the roster of %{day}.",
            "The roster of %{day} is now on %{days} too (%{n} days).",
            "The roster of %{day} is now on %{days} too.",
            "Nothing to repeat",
            "Nothing to clear",
            "Nothing could be filled",
            "Train drivers",
        ];
        for lang in ["nl", "de", "fr", "ru", "uk", "pl"] {
            for k in keys {
                assert!(crate::_rust_i18n_try_translate(lang, k).is_some(), "{lang}: {k}");
            }
        }
    }
}
