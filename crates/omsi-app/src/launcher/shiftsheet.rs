//! The duty step's sheet, after Omsi-Hub's: the duties to choose from as a list of
//! departures, arrivals and lengths, and the chosen one opened up into its trips - when each
//! leaves, the line's plate, where it goes and for how long, the break before it, and where
//! the bus goes on with another tour - so that a duty is chosen by what it is and not by its
//! numbers. A work shift (`omsi_launcher_lib::compose`) is asked for by its length and its
//! part of the day; a tour - one bus's trips on one line, OMSI's own duty - by its line, and
//! its opened row is where the trip it starts with is picked. The rest of the step - the map
//! with the duty's route, the roadbook and the actions - is `flow::step_duty`; the map's own
//! buttons and its scale are drawn here.

use super::drive;
use super::ownlines;
use super::state::{hhmm, DAYPARTS};
use super::theme::*;
use super::ui::{id_of, ButtonKind, Ui};
use super::Launcher;
use glam::Vec2;
use omsi_launcher_lib::compose::{Block, ComposedDuty, Leg};
use omsi_launcher_lib::{LineInfo, TourInfo};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

/// What the sheet keeps between frames.
#[derive(Default)]
pub struct ShiftView {
    /// The chosen row the list was last brought into view for: once each time the choice
    /// changes (a shift read back from the last launch, a tour picked from the line), not
    /// on every frame - the player scrolls it after that.
    shown: Option<Vec<String>>,
}

/// The sheet's inner margin, and a row's height.
const PAD: f32 = 20.0;
const ROW_H: f32 = 38.0;
/// Where a row's columns begin: the first after the radio, the others as shares of the
/// row's width (Omsi-Hub's departure, arrival and length).
const FIRST_COL: f32 = 42.0;
const COL_2: f32 = 0.44;
const COL_3: f32 = 0.68;
/// The opened row's trips: their time, their line's plate and where they go, from the row's
/// left edge.
const TRIP_TIME: f32 = 46.0;
const TRIP_PLATE: f32 = 100.0;
const TRIP_TO: f32 = 152.0;
/// The opened row's ground, a step lighter than the sheet; the range's track.
const DETAIL: Color = Color::rgba(34, 39, 51, 1.0);
const TRACK: Color = Color::rgba(41, 47, 58, 1.0);
/// The list's name (its scrolling is kept by it).
const LIST: &str = "shift-list";

// --- what a duty is made of ----------------------------------------------------------------

/// One trip of a duty as the sheet lists it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct TripLine {
    pub departure: f64,
    pub arrival: f64,
    /// The line its displays show (empty: a run without a line, to or from the depot).
    pub line: String,
    pub terminus: String,
    pub stops: usize,
    /// How long the bus stands before it leaves (s; nothing before a duty's first trip).
    pub pause: f64,
    /// The first trip of another tour: the line (`.ttl`) and the tour the bus goes on with.
    /// The game changes over by itself (`--duty-leg`) where OMSI made the player do it.
    pub takes_over: Option<(String, String)>,
}

/// A duty as a row of the list: what tells it apart, what its three columns say, and its
/// trips.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Card {
    /// A shift's `--duty-leg`s; a tour's number.
    pub key: Vec<String>,
    pub cells: [String; 3],
    /// The third column is a length (a clock in front of it).
    pub clock: bool,
    /// A tour that does not run on the chosen day: the days it does (the row is quiet).
    pub note: String,
    pub trips: Vec<TripLine>,
}

/// The lines `trips` show, each once, in the order they are driven.
fn lines_of(trips: &[TripLine]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for t in trips.iter().filter(|t| !t.line.is_empty()) {
        if !out.contains(&t.line.as_str()) {
            out.push(&t.line);
        }
    }
    out
}

/// A work shift as a row: its legs' trips looked up in the day's timetable for their stops,
/// the breaks at the termini, and where it goes on with another tour.
pub(super) fn shift_card(d: &ComposedDuty, lines: &[LineInfo]) -> Card {
    let mut trips = Vec::with_capacity(d.legs.len());
    for (k, leg) in d.legs.iter().enumerate() {
        let prev = k.checked_sub(1).map(|j| &d.legs[j]);
        // (a later trip of the same tour is a part of its own for the game, but the same bus
        // for the driver: only another tour is said)
        let same_tour = prev.is_some_and(|p| p.line == leg.line && p.tour == leg.tour);
        let stops = lines.iter().find(|x| x.name == leg.line).and_then(|x| x.tours.iter().find(|t| t.number == leg.tour)).and_then(|t| t.trips.iter().find(|x| x.index == leg.index)).map(|t| t.stops.len()).unwrap_or(0);
        trips.push(TripLine {
            departure: leg.departure,
            arrival: leg.arrival,
            line: leg.shown.clone(),
            terminus: leg.terminus.clone(),
            stops,
            pause: prev.map(|p| (leg.departure - p.arrival).max(0.0)).unwrap_or(0.0),
            takes_over: (prev.is_some() && !same_tour).then(|| (leg.line.clone(), leg.tour.clone())),
        });
    }
    Card { key: d.blocks().iter().map(Block::arg).collect(), cells: [hhmm(d.start()), hhmm(d.end()), length(d.seconds())], clock: true, note: String::new(), trips }
}

/// A tour as a row: its number, when it leaves the depot and when it is back, and its trips.
pub(super) fn tour_card(t: &TourInfo) -> Card {
    let mut trips = Vec::with_capacity(t.trips.len());
    for (k, x) in t.trips.iter().enumerate() {
        let pause = k.checked_sub(1).map(|j| (x.departure - t.trips[j].arrival).max(0.0)).unwrap_or(0.0);
        trips.push(TripLine { departure: x.departure, arrival: x.arrival, line: x.line.clone(), terminus: x.terminus.clone(), stops: x.stops.len(), pause, takes_over: None });
    }
    let first = t.trips.first().map(|x| x.departure).unwrap_or(t.first);
    let last = t.trips.last().map(|x| x.arrival).unwrap_or(t.last);
    Card { key: vec![t.number.clone()], cells: [t.number.clone(), hhmm(first), hhmm(last)], clock: false, note: if t.runs { String::new() } else { super::drive::days_text(&t.days) }, trips }
}

/// The shift `legs` (its `--duty-leg`s) stands for, read back from the day's timetable: the
/// one chosen at the last launch is shown even when the list has been drawn anew since. None
/// when one of its tours or trips is not in the timetable.
pub(super) fn shift_of_legs(legs: &[String], lines: &[LineInfo]) -> Option<ComposedDuty> {
    let mut out = ComposedDuty::default();
    for arg in legs {
        let (line, tour, first, count) = Block::parse(arg)?;
        let t = lines.iter().find(|x| x.name == line)?.tours.iter().find(|t| t.number == tour)?;
        let trips: Vec<_> = t.trips.iter().filter(|x| x.index >= first && x.index < first + count).collect();
        if trips.len() != count {
            return None;
        }
        for x in trips {
            let shown = if x.line.is_empty() { line.clone() } else { x.line.clone() };
            out.legs.push(Leg { line: line.clone(), tour: tour.clone(), index: x.index, trip: x.name.clone(), shown, from: x.from.clone(), terminus: x.terminus.clone(), departure: x.departure, arrival: x.arrival });
        }
    }
    (!out.legs.is_empty()).then_some(out)
}

/// Whole minutes.
fn minutes(seconds: f64) -> i64 {
    (seconds.max(0.0) / 60.0).round() as i64
}

/// "2h 05m": a length as Omsi-Hub writes it, in the interface's language ("2u 05m").
pub(super) fn length(seconds: f64) -> String {
    let m = minutes(seconds);
    omsi_ui::tr("%{h}h %{mm}m").replace("%{h}", &(m / 60).to_string()).replace("%{mm}", &format!("{:02}", m % 60))
}

/// "3 trips · 67 stops · 2 lines", each in the singular where it is one (a lone trip read
/// "1 trips" in Omsi-Hub once, which reads as a mistake and not as a number).
pub(super) fn summary(trips: usize, stops: usize, lines: usize) -> String {
    let n = |n: usize, one: &str, many: &str| omsi_ui::tr(if n == 1 { one } else { many }).replace("%{n}", &n.to_string());
    let mut parts = vec![n(trips, "%{n} trip", "%{n} trips"), n(stops, "%{n} stop", "%{n} stops")];
    if lines > 0 {
        parts.push(n(lines, "%{n} line", "%{n} lines"));
    }
    parts.join(" · ")
}

// --- the list ------------------------------------------------------------------------------

/// How tall the opened row of `c` is (`first`, a tour's: see `duty_list`).
fn detail_height(c: &Card, first: Option<usize>) -> f32 {
    trip_top(c, c.trips.len(), first) + 10.0
}

/// Whether the break before trip `k` is said: between two trips the duty drives (the ones
/// of a tour before the trip it starts with are not driven, and a day of eight-minute
/// breaks at the termini buried the trips that are).
fn break_shown(c: &Card, k: usize, first: Option<usize>) -> bool {
    k > first.unwrap_or(0) && minutes(c.trips[k].pause) >= 1
}

/// Where trip `k` of the opened row `c` begins, from the opened row's top (`k` past the last:
/// where the trips end).
fn trip_top(c: &Card, k: usize, first: Option<usize>) -> f32 {
    let mut y = 30.0 + if first.is_some() { 18.0 } else { 0.0 };
    for (j, t) in c.trips.iter().enumerate() {
        if break_shown(c, j, first) {
            y += 18.0;
        }
        if j == k {
            return y;
        }
        y += 24.0 + if t.takes_over.is_some() { 20.0 } else { 0.0 };
    }
    y
}

/// The duties as a list in `r`: a row each - its radio and its three columns - and the
/// chosen one opened into its trips. `first` (a tour's): the trip the duty starts with - its
/// trips can be clicked, and the ones before it are left out of the drive and drawn quiet.
/// `empty` is what an empty list says. Returns the row clicked and the trip clicked.
pub(super) fn duty_list(ui: &mut Ui, r: Rect, cards: &[Card], chosen: Option<usize>, first: Option<usize>, empty: &str) -> (Option<usize>, Option<usize>) {
    let mut pick = None;
    let mut trip = None;
    ui.scroll_area(LIST, r, &mut |ui, v| {
        if cards.is_empty() {
            return ui.paragraph(empty, Vec2::new(v.x + 14.0, v.y + 8.0), v.w - 36.0, 12.5, Weight::Regular, TEXT_DIM) + 16.0;
        }
        let mut y = v.y;
        for (k, c) in cards.iter().enumerate() {
            let rr = Rect::new(v.x, y, v.w - 12.0, ROW_H - 2.0);
            let on = chosen == Some(k);
            if ui.rect_visible(rr) {
                if ui.row(&format!("{LIST}-{k}"), rr, on) {
                    pick = Some(k);
                }
                row_cells(ui, rr, c, on);
            }
            y += ROW_H;
            if on {
                let d = Rect::new(rr.x, y - 2.0, rr.w, detail_height(c, first));
                if ui.rect_visible(d) {
                    trip = detail(ui, d, c, first).or(trip);
                }
                y += d.h;
            }
        }
        y - v.y + 4.0
    });
    (pick, trip)
}

/// A row's radio and its three columns.
fn row_cells(ui: &mut Ui, rr: Rect, c: &Card, on: bool) {
    radio(ui, Vec2::new(rr.x + 16.0, rr.center().y), on);
    let quiet = !c.note.is_empty();
    let ink = if on { on_accent() } else if quiet { TEXT_FAINT } else { TEXT };
    let (x2, x3) = (rr.x + rr.w * COL_2, rr.x + rr.w * COL_3);
    let x1 = rr.x + FIRST_COL;
    let w1 = ui.text_in(&c.cells[0], Rect::new(x1, rr.y, x2 - x1 - 8.0, rr.h), 14.0, Weight::Bold, ink, Align::Left);
    if quiet {
        let at = x1 + w1 + 7.0;
        ui.text_in(&c.note, Rect::new(at, rr.y, (x2 - at - 8.0).max(0.0), rr.h), 11.0, Weight::Medium, if on { on_accent() } else { TEXT_FAINT }, Align::Left);
    }
    ui.text_in(&c.cells[1], Rect::new(x2, rr.y, x3 - x2 - 8.0, rr.h), 14.0, Weight::Bold, ink, Align::Left);
    let mut x = x3;
    if c.clock {
        ui.icon("schedule", Vec2::new(x + 6.0, rr.center().y), 13.0, if on { on_accent() } else { TEXT_DIM });
        x += 18.0;
    }
    let (weight, c3) = if c.clock { (Weight::Medium, if on { on_accent() } else { TEXT_SOFT }) } else { (Weight::Bold, ink) };
    ui.text_in(&c.cells[2], Rect::new(x, rr.y, (rr.right() - x - 8.0).max(0.0), rr.h), if c.clock { 13.5 } else { 14.0 }, weight, c3, Align::Left);
}

/// The opened row: how much the duty is, then its trips with the breaks between them. A
/// tour's trips are buttons (the one the duty starts with marked); returns the one clicked.
fn detail(ui: &mut Ui, d: Rect, c: &Card, first: Option<usize>) -> Option<usize> {
    ui.p().rect(d, DETAIL);
    ui.p().rect(Rect::new(d.x, d.bottom() - 1.0, d.w, 1.0), HAIRLINE);
    let text_w = d.w - TRIP_TIME - 10.0;
    let mut y = d.y + 8.0;
    // (what is driven: a tour from the trip it starts with)
    let driven = &c.trips[first.unwrap_or(0).min(c.trips.len())..];
    let head = summary(driven.len(), driven.iter().map(|t| t.stops).sum(), lines_of(driven).len()).to_uppercase();
    ui.text_in(&head, Rect::new(d.x + TRIP_TIME, y, text_w, 20.0), 11.0, Weight::Medium, TEXT_DIM, Align::Left);
    y += 22.0;
    if first.is_some() {
        ui.text_in("Click a trip to start the tour there.", Rect::new(d.x + TRIP_TIME, y - 2.0, text_w, 16.0), 11.5, Weight::Regular, TEXT_FAINT, Align::Left);
        y += 18.0;
    }
    let mut clicked = None;
    for (k, t) in c.trips.iter().enumerate() {
        if break_shown(c, k, first) {
            let text = omsi_ui::tr("%{time} break").replace("%{time}", &length(t.pause));
            ui.text_in(&text, Rect::new(d.x + TRIP_PLATE - 8.0, y, d.w - TRIP_PLATE, 18.0), 11.5, Weight::Regular, TEXT_DIM, Align::Left);
            y += 18.0;
        }
        let (before, start) = first.map(|f| (k < f, k == f)).unwrap_or((false, false));
        if first.is_some() {
            let lr = Rect::new(d.x + 6.0, y, d.w - 12.0, 24.0);
            let (h, _, hit) = ui.interact(id_of(&format!("{LIST}-trip-{k}")), lr);
            if start {
                ui.p().rounded(lr, 6.0, accent().alpha(0.3));
                ui.icon("play_arrow", Vec2::new(d.x + 26.0, lr.center().y), 15.0, accent_2());
            } else if h {
                ui.p().rounded(lr, 6.0, Color::WHITE.alpha(0.05));
            }
            if hit && !start {
                clicked = Some(k);
            }
        }
        let (time_ink, ink, soft) = if before { (TEXT_FAINT, TEXT_FAINT, TEXT_FAINT) } else if start { (TEXT, TEXT, TEXT_SOFT) } else { (TEXT_DIM, TEXT, TEXT_DIM) };
        ui.text_in(&hhmm(t.departure), Rect::new(d.x + TRIP_TIME, y, 50.0, 24.0), 12.5, Weight::Regular, time_ink, Align::Left);
        plate(ui, Vec2::new(d.x + TRIP_PLATE, y + 3.0), &t.line, 18.0, before);
        ui.text_in(&t.terminus, Rect::new(d.x + TRIP_TO, y, d.w - TRIP_TO - 74.0, 24.0), 12.5, Weight::Bold, ink, Align::Left);
        ui.text_in(&length(t.arrival - t.departure), Rect::new(d.right() - 72.0, y, 58.0, 24.0), 12.0, Weight::Regular, soft, Align::Right);
        y += 24.0;
        if let Some((line, tour)) = &t.takes_over {
            let text = omsi_ui::tr("On with line %{line}, tour %{tour}").replace("%{line}", line).replace("%{tour}", tour);
            ui.icon("sync_alt", Vec2::new(d.x + TRIP_PLATE - 1.0, y + 9.0), 12.0, TEXT_DIM);
            ui.text_in(&text, Rect::new(d.x + TRIP_PLATE + 10.0, y, d.w - TRIP_PLATE - 20.0, 18.0), 11.5, Weight::Regular, TEXT_DIM, Align::Left);
            y += 20.0;
        }
    }
    clicked
}

/// The round mark of a row: a ring, and a dot in it when chosen (as `flow`'s map list).
fn radio(ui: &mut Ui, c: Vec2, on: bool) {
    // (the Ui's own: its dot pops in when the row is chosen, as on the other steps)
    ui.radio(c, on);
}

/// A line's plate as a pill, `h` high at `at` (a run without a line: a grey dash). Returns
/// its width.
fn plate(ui: &mut Ui, at: Vec2, line: &str, h: f32, quiet: bool) -> f32 {
    let px = h * 0.64;
    let text = if line.is_empty() { "–" } else { line };
    let w = (ui.width(text, px, Weight::Bold) + h * 0.8).clamp(h * 1.4, TRIP_TO - TRIP_PLATE - 8.0);
    let r = Rect::new(at.x, at.y, w, h);
    let (fill, ink) = if line.is_empty() { (Color::WHITE.alpha(0.14), TEXT_SOFT) } else { (LINE, ON_LINE) };
    let k = if quiet { 0.45 } else { 1.0 };
    ui.p().rounded(r, h * 0.5, fill.alpha(k));
    ui.text_in(text, r.pad(4.0, 0.0), px, Weight::Bold, ink.alpha(k), Align::Center);
    w
}

/// A control's name above it: small capitals, quiet.
fn caps(ui: &mut Ui, r: Rect, text: &str) {
    ui.text_in(&omsi_ui::tr(text).to_uppercase(), r, 10.5, Weight::Bold, TEXT_DIM, Align::Left);
}

/// Omsi-Hub's range: a thin track and the blue knob on it - dragged, clicked, or turned with
/// the wheel a `step` at a time. Returns true when the value moved.
fn range(ui: &mut Ui, name: &str, r: Rect, value: &mut f32, min: f32, max: f32, step: f32) -> bool {
    let id = id_of(name);
    let (h, held, _) = ui.interact(id, r.pad(-8.0, 0.0));
    let before = *value;
    if held {
        let t = ((ui.input.mouse.x - r.x) / r.w.max(1.0)).clamp(0.0, 1.0);
        *value = ((min + t * (max - min)) / step).round() * step;
        ui.cursor = winit::window::CursorIcon::Grabbing;
    } else if h && ui.input.wheel.y.abs() > 0.0 && !ui.input.touch {
        *value += ui.input.wheel.y.signum() * step;
        ui.input.wheel.y = 0.0;
    }
    *value = value.clamp(min, max);
    let frac = ((*value - min) / (max - min).max(1e-6)).clamp(0.0, 1.0);
    let shown = ui.anim(id ^ 3, frac, 0.06);
    let cy = r.center().y;
    ui.p().rounded(Rect::new(r.x, cy - 2.0, r.w, 4.0), 2.0, TRACK);
    ui.p().circle(Vec2::new(r.x + r.w * shown, cy), if h || held { 9.0 } else { 8.0 }, accent());
    *value != before
}

/// Omsi-Hub's chips: a word each, side by side (on to a new row where they do not fit), the
/// chosen one filled blue. Returns the one clicked and the height they took.
fn chips(ui: &mut Ui, name: &str, at: Vec2, max_w: f32, labels: &[&str], chosen: usize) -> (Option<usize>, f32) {
    let (h, gap) = (26.0, 5.0);
    let (mut x, mut y) = (at.x, at.y);
    let mut clicked = None;
    for (k, label) in labels.iter().enumerate() {
        let w = ui.width(label, 12.0, Weight::Medium) + 22.0;
        if x + w > at.x + max_w && x > at.x {
            x = at.x;
            y += h + gap;
        }
        let r = Rect::new(x, y, w, h);
        let (hov, _, hit) = ui.interact(id_of(&format!("{name}-{k}")), r);
        let on = k == chosen;
        if on {
            ui.p().rounded(r, h * 0.5, accent());
        } else {
            ui.p().rounded(r, h * 0.5, if hov { HOVER } else { Color::CLEAR });
            ui.p().rounded_border(r, h * 0.5, 1.0, Color::WHITE.alpha(if hov { 0.2 } else { 0.1 }));
        }
        ui.text_in(label, r, 12.0, if on { Weight::Bold } else { Weight::Medium }, if on { on_accent() } else if hov { TEXT } else { TEXT_SOFT }, Align::Center);
        if hit && !on {
            clicked = Some(k);
        }
        x += w + gap;
    }
    (clicked, y + h - at.y)
}

/// Bring the chosen row into view once each time it changes (`scope`: the line its tours are
/// of): the row and its trips, or - a tour whose trip to start with lies further down than
/// the list is tall - that trip near the list's top, with the drive after it in view.
fn keep_in_view(l: &mut Launcher, scope: &str, list: Rect, cards: &[Card], chosen: Option<usize>, first: Option<usize>) {
    let key = chosen.and_then(|k| cards.get(k)).map(|c| std::iter::once(scope.to_string()).chain(c.key.iter().cloned()).collect());
    if l.drive.shift.shown == key {
        return;
    }
    l.drive.shift.shown = key;
    let Some(k) = chosen else { return };
    let top = k as f32 * ROW_H;
    let c = &cards[k];
    let (y, h) = match first {
        Some(f) => {
            let trip = top + ROW_H - 2.0 + trip_top(c, f, first);
            if trip + 24.0 - top <= list.h { (top, trip + 24.0 - top) } else { (trip - 2.0 * 24.0, list.h) }
        }
        None => (top, (ROW_H + detail_height(c, None)).min(list.h)),
    };
    l.ui.scroll_to(LIST, y, h, list.h);
}

// --- the sheet -----------------------------------------------------------------------------

/// The line the sheet's title names: the chosen duty's first, as its displays show it.
fn chosen_line(l: &Launcher) -> Option<String> {
    drive::duty_trips(l).iter().map(|t| t.line.clone()).find(|x| !x.is_empty()).or_else(|| l.state.choice.line.clone().filter(|_| !l.state.choice.composed))
}

/// The sheet's title and the line under it: "Shifts · 35", what to do.
pub(super) fn head(l: &Launcher) -> (String, String) {
    let composed = l.state.choice.composed;
    let what = omsi_ui::tr(if composed { "Shifts" } else { "Tours" }).into_owned();
    let title = match chosen_line(l) {
        Some(line) => format!("{what} · {line}"),
        None => what,
    };
    let sub = if composed { "Select a shift to continue" } else { "Pick a line, a tour and the trip to start with" };
    (title, omsi_ui::tr(sub).into_owned())
}

/// The sheet's foot: the line, and how many duties there are to choose from.
pub(super) fn foot(l: &mut Launcher) -> String {
    if l.state.loading_lines {
        return omsi_ui::tr("Reading the timetable…").into_owned();
    }
    let line = chosen_line(l).map(|x| format!("{x} · ")).unwrap_or_default();
    let count = |n: usize, one: &str, many: &str| if n == 1 { omsi_ui::tr(one).into_owned() } else { omsi_ui::tr(many).replace("%{n}", &n.to_string()) };
    if l.state.choice.composed {
        let n = l.state.composed_duties().len();
        return format!("{line}{}", count(n, "one shift available", "%{n} shifts available"));
    }
    match l.state.line() {
        Some(x) => format!("{line}{}", count(x.tours.iter().filter(|t| t.runs).count(), "one tour on this day", "%{n} tours on this day")),
        None => count(l.state.lines.iter().filter(|x| x.user_allowed).count(), "one line on this day", "%{n} lines on this day"),
    }
}

/// The sheet between its head and its foot: a shift's length, part of the day and list, or
/// a tour's line and list; under them where the bus is put down.
pub(super) fn body(l: &mut Launcher, body: Rect) {
    let x = body.x + PAD;
    let w = body.w - 2.0 * PAD;
    let y = body.y + server(l, Rect::new(x, body.y, w, ROW));
    // where to start, at the sheet's foot: one of the map's entry points, as in OMSI 2;
    // Automatic takes the one nearest to the duty's first stop by road
    let start_y = body.bottom() - ROW - 12.0;
    let below = if l.state.map().is_some() { start_y - 14.0 } else { body.bottom() };
    if l.state.choice.composed {
        shifts(l, body, y, below);
    } else {
        tours(l, body, y, below);
    }
    if l.state.map().is_some() {
        drive::entry_select(l, Rect::new(x, body.y, w, body.h), start_y, false);
    }
}

/// On a server: whose map it is, and the way back to driving alone. Returns its height.
fn server(l: &mut Launcher, r: Rect) -> f32 {
    let Some(name) = drive::joined_server_name(l) else { return 0.0 };
    let m = l.state.map().map(|m| if m.friendly.is_empty() { m.name.clone() } else { m.friendly.clone() }).unwrap_or_else(|| l.state.choice.map.clone());
    let leave_w = 150.0;
    l.ui.icon("lock", Vec2::new(r.x + 8.0, r.center().y), 15.0, TEXT_DIM);
    l.ui.text_in(&format!("{m} · {name}"), Rect::new(r.x + 22.0, r.y, r.w - leave_w - 28.0, r.h), 12.5, Weight::Regular, TEXT_SOFT, Align::Left);
    let leave = Rect::new(r.right() - leave_w, r.y + 2.0, leave_w, r.h - 4.0);
    if l.ui.button("shift-leave-server", leave, "Leave the server", Some("logout"), ButtonKind::Ghost) {
        l.state.leave_server();
    }
    l.ui.tooltip(leave, "Back to driving alone: the map, the clock and the weather are your own again");
    r.h + 10.0
}

/// The shift: its length, its part of the day, and the shifts the timetable gives for them,
/// from `y` down to `bottom`.
fn shifts(l: &mut Launcher, body: Rect, mut y: f32, bottom: f32) {
    let x = body.x + PAD;
    let w = body.w - 2.0 * PAD;
    caps(&mut l.ui, Rect::new(x, y, w * 0.6, 18.0), "Shift length");
    let mut minutes = l.state.choice.duty_minutes.clamp(30, 480) as f32;
    l.ui.text_in(&length(minutes as f64 * 60.0), Rect::new(x + w * 0.4, y, w * 0.6, 18.0), 14.5, Weight::Bold, TEXT, Align::Right);
    if range(&mut l.ui, "shift-length", Rect::new(x, y + 20.0, w, 22.0), &mut minutes, 30.0, 480.0, 15.0) {
        l.state.choice.duty_minutes = minutes as i32;
        l.state.touched();
    }
    y += 50.0;
    caps(&mut l.ui, Rect::new(x, y, w, 18.0), "Time of day");
    let names: Vec<&str> = DAYPARTS.iter().map(|d| d.0).collect();
    let part = l.state.choice.daypart.min(DAYPARTS.len() - 1);
    let (clicked, h) = chips(&mut l.ui, "shift-daypart", Vec2::new(x, y + 22.0), w, &names, part);
    super::tour::anchor("shift-length", Rect::new(x, y - 50.0, w, 72.0 + h));
    l.ui.tooltip(Rect::new(x, y + 22.0, w, h), "When the shift's first trip leaves");
    if let Some(k) = clicked {
        l.state.choice.daypart = k;
        l.state.touched();
    }
    y += 22.0 + h + 14.0;
    // the shifts: the ones drawn for the length and the part of the day, and the one chosen
    // (read back from the last launch, or of another length) above them while it holds
    let duties = l.state.composed_duties().to_vec();
    let chosen_legs = l.state.composed_legs().map(|x| x.to_vec());
    let mut sources = duties;
    let mut cards: Vec<Card> = sources.iter().map(|d| shift_card(d, &l.state.lines)).collect();
    let mut chosen = chosen_legs.as_ref().and_then(|legs| cards.iter().position(|c| &c.key == legs));
    if chosen.is_none() {
        if let Some(d) = chosen_legs.as_ref().and_then(|legs| shift_of_legs(legs, &l.state.lines)) {
            cards.insert(0, shift_card(&d, &l.state.lines));
            sources.insert(0, d);
            chosen = Some(0);
        }
    }
    let list = list_frame(l, body, y, bottom, ["Departure", "Arrival", "Duration"]);
    super::tour::anchor("shift-list", Rect::new(list.x, y + 4.0, list.w, list.bottom() - y - 4.0));
    keep_in_view(l, "", list, &cards, chosen, None);
    let empty = if l.state.loading_lines { "Reading the timetable…" } else { "No shift of this length begins at this time of day. Try another length or part of the day." };
    let (pick, _) = duty_list(&mut l.ui, list, &cards, chosen, None, empty);
    if let Some(k) = pick.filter(|k| Some(*k) != chosen) {
        l.state.pick_composed(&sources[k]);
        // (the roadbook beside the map has something to say now)
        l.state.load_ibis();
    }
}

/// The tour: the line, and its tours on the day - the chosen one opened, its trips to pick
/// the first from - from `y` down to `bottom`.
fn tours(l: &mut Launcher, body: Rect, mut y: f32, bottom: f32) {
    let x = body.x + PAD;
    let w = body.w - 2.0 * PAD;
    // the map's lines or the player's own (the line editor's), as chosen last: a switch on
    // the row of the list's name
    let (map_rows, my_rows) = {
        let own = &l.state.own_lines;
        let (map, mine) = ownlines::split(l.state.lines.iter().filter(|x| x.user_allowed), own);
        let rows = |v: &[&LineInfo]| -> Vec<(String, String)> { v.iter().map(|x| (x.name.clone(), ownlines::option_label(x, own))).collect() };
        (rows(&map), rows(&mine))
    };
    caps(&mut l.ui, Rect::new(x, y, w * 0.3, 18.0), "Line");
    let sw = (w * 0.7).min(300.0);
    let switch = Rect::new(x + w - sw, y - 5.0, sw, 28.0);
    match ownlines::switch(&mut l.ui, "shift-line-source", switch, l.state.choice.my_lines, (map_rows.len(), my_rows.len())) {
        ownlines::Switched::To(mine) => {
            l.state.choice.my_lines = mine;
            l.state.touched();
        }
        ownlines::Switched::Hint => l.state.set_status(omsi_ui::tr(ownlines::NONE_YET), false),
        ownlines::Switched::No => {}
    }
    y += 30.0;
    let lines = if ownlines::showing_mine(l.state.choice.my_lines, my_rows.len()) { my_rows } else { map_rows };
    let chosen_line = l.state.choice.line.clone();
    if lines.is_empty() {
        let why = if l.state.loading_lines { "Reading the timetable…" } else { "No lines on this date." };
        l.ui.text_in(why, Rect::new(x, y, w, ROW), 12.5, Weight::Regular, TEXT_DIM, Align::Left);
    } else {
        // (a dropdown: typing in it searches it, as the line filter did; a line of the
        // player's with its plate, its name and where it goes)
        let at = chosen_line.as_ref().and_then(|c| lines.iter().position(|x| &x.0 == c));
        let mut options: Vec<String> = lines.iter().map(|(_, label)| label.clone()).collect();
        let offset = usize::from(at.is_none());
        if at.is_none() {
            options.insert(0, omsi_ui::tr("Choose a line…").into_owned());
        }
        let mut sel = at.map(|k| k + offset).unwrap_or(0);
        if l.ui.select("shift-line", Rect::new(x, y, w, ROW), &mut sel, &options) {
            if let Some((n, _)) = sel.checked_sub(offset).and_then(|k| lines.get(k)) {
                if chosen_line.as_deref() != Some(n.as_str()) {
                    l.state.choice.line = Some(n.clone());
                    l.state.choice.tour = None;
                    l.state.touched();
                }
            }
        }
    }
    y += ROW + 14.0;
    // the line's tours, the ones running on the day first (OMSI lists only those)
    let mut tours: Vec<&TourInfo> = l.state.line().map(|x| x.tours.iter().collect()).unwrap_or_default();
    tours.sort_by(|a, b| b.runs.cmp(&a.runs).then_with(|| drive::natural(&a.number).cmp(&drive::natural(&b.number))));
    let cards: Vec<Card> = tours.iter().map(|t| tour_card(t)).collect();
    let picks: Vec<(String, bool, Option<String>, Vec<(usize, f64)>)> = tours.iter().map(|t| (t.number.clone(), t.runs, t.next_run.clone(), t.trips.iter().map(|x| (x.index, x.departure)).collect())).collect();
    let chosen = l.state.choice.tour.as_ref().and_then(|n| picks.iter().position(|p| &p.0 == n));
    let first = chosen.map(|_| l.state.first_trip().unwrap_or(0));
    let list = list_frame(l, body, y, bottom, ["Tour", "Departure", "Arrival"]);
    keep_in_view(l, chosen_line.as_deref().unwrap_or(""), list, &cards, chosen, first);
    let empty = if chosen_line.is_none() { "Pick a line first. As in OMSI, the start time and date then say where in the tour the bus is: the trip under way, or the next to leave." } else { "Reading the timetable…" };
    let (pick, trip) = duty_list(&mut l.ui, list, &cards, chosen, first, empty);
    if let Some((num, runs, next, _)) = pick.filter(|k| Some(*k) != chosen).and_then(|k| picks.get(k)).cloned() {
        // a tour of another day moves the date to the next day it runs (OMSI lists only the
        // day's tours)
        if !runs {
            if let Some(n) = next {
                l.state.choice.date = n;
                l.state.load_lines();
            }
        }
        l.state.choice.tour = Some(num);
        l.state.touched();
        l.state.load_ibis();
    } else if let (Some(t), Some(k), Some(line)) = (trip, chosen, chosen_line) {
        // the trip the tour starts with: the clock goes to its departure, as OMSI's own
        // timetable dialog sets it
        if let Some(&(index, departure)) = picks[k].3.get(t) {
            let time = (departure / 60.0).floor() as i32;
            l.state.choice.time = time;
            l.state.choice.start_trip = Some((line, picks[k].0.clone(), index, time));
            l.state.touched();
        }
    }
}

/// The hairline over the list and its column heads. Returns the list's rect.
fn list_frame(l: &mut Launcher, body: Rect, y: f32, bottom: f32, heads: [&str; 3]) -> Rect {
    l.ui.p().rect(Rect::new(body.x, y, body.w, 1.0), HAIRLINE);
    let list = Rect::new(body.x + 12.0, y + 38.0, body.w - 18.0, (bottom - y - 38.0).max(ROW_H));
    let row_w = list.w - 12.0;
    let xs = [list.x + FIRST_COL, list.x + row_w * COL_2, list.x + row_w * COL_3];
    for (k, head) in heads.iter().enumerate() {
        caps(&mut l.ui, Rect::new(xs[k], y + 12.0, 110.0, 18.0), head);
    }
    list
}

/// "Find other shifts": the shifts are drawn from the timetable by chance, so another seed
/// draws others of the same length and time.
pub(super) fn other_shifts(l: &mut Launcher) {
    l.state.compose_seed = l.state.compose_seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
}

// --- on the map ----------------------------------------------------------------------------

/// The map's own buttons, as Omsi-Hub's: the route framed again, in, out - in a column whose
/// top right corner is `at` - and, left of them, the roadbook's while it is put away.
/// Returns where they are (no name is written over them).
pub(super) fn map_tools(l: &mut Launcher, at: Vec2, book_handle: bool) -> Vec<Rect> {
    let cell = 32.0;
    let group = Rect::new(at.x - cell, at.y, cell, cell * 3.0);
    l.ui.solid(group);
    l.ui.p().shadow(group.inset(-1.0), RADIUS, 14.0, Color::rgba(0, 0, 0, 0.35));
    l.ui.p().rounded(group, RADIUS, PANEL);
    l.ui.p().rounded_border(group, RADIUS, 1.0, EDGE);
    let tools: [(&str, &str); 3] = [("near_me", "Centre the map on the route"), ("add", "Zoom in"), ("remove", "Zoom out")];
    for (k, (icon, tip)) in tools.iter().enumerate() {
        let r = Rect::new(group.x, group.y + k as f32 * cell, cell, cell);
        if k > 0 {
            l.ui.p().rect(Rect::new(r.x + 1.0, r.y, r.w - 2.0, 1.0), HAIRLINE);
        }
        let (h, _, clicked) = l.ui.interact(id_of(&format!("map-tool-{k}")), r);
        l.ui.icon(icon, r.center(), 17.0, if h { TEXT } else { TEXT_SOFT });
        l.ui.tooltip(r, tip);
        if clicked {
            match k {
                0 => l.mapview.refit(),
                1 => l.mapview.zoom_by(1.0 / 1.5),
                _ => l.mapview.zoom_by(1.5),
            }
        }
    }
    let mut taken = vec![group];
    if book_handle {
        // (the roadbook's own button, where `drive::book_handle` puts it in the rect given)
        let right = group.x - 10.0;
        taken.push(drive::book_handle(l, Rect::new(0.0, at.y - 12.0, right + 12.0, cell * 3.0 + 12.0)));
    }
    taken
}

/// The map's scale in its bottom left corner, from `at` on: a round length about as long as
/// a thumb, and its name.
pub(super) fn scale_bar(l: &mut Launcher, at: Vec2) {
    if !l.mapview.status().is_empty() {
        return;
    }
    let mpp = l.mapview.metres_per_point();
    if !(mpp > 0.0) {
        return;
    }
    let (metres, w) = scale_of(mpp, 160.0);
    let c = TEXT_FAINT;
    l.ui.p().rect(Rect::new(at.x, at.y - 0.5, w, 1.5), c);
    for x in [at.x, at.x + w] {
        l.ui.p().rect(Rect::new(x - 0.5, at.y - 4.0, 1.5, 8.0), c);
    }
    let text = if metres >= 1000.0 { format!("{} km", metres / 1000.0) } else { format!("{metres} m") };
    l.ui.text_in(&text, Rect::new(at.x + w + 6.0, at.y - 9.0, 80.0, 18.0), 11.0, Weight::Medium, TEXT_DIM, Align::Left);
}

/// The roundest length that is at most `most` points long at `mpp` metres a point, and its
/// length in points.
fn scale_of(mpp: f64, most: f32) -> (f64, f32) {
    const NICE: [f64; 15] = [5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0, 20000.0, 50000.0, 100000.0, 200000.0];
    let metres = NICE.iter().copied().take_while(|m| (m / mpp) as f32 <= most).last().unwrap_or(NICE[0]);
    (metres, (metres / mpp) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use omsi_launcher_lib::{StopInfo, TripInfo};

    fn trip(index: usize, line: &str, from: &str, to: &str, dep: f64, arr: f64, stops: usize) -> TripInfo {
        let stops = (0..stops).map(|k| StopInfo { name: format!("{to} {k}"), id: k as i64, arr: dep + k as f64 * 60.0, dep: dep + k as f64 * 60.0 }).collect();
        TripInfo { name: format!("{line} {to}"), index, line: line.into(), from: from.into(), terminus: to.into(), departure: dep, arrival: arr, stops, km: 5.0 }
    }

    fn tour(number: &str, runs: bool, trips: Vec<TripInfo>) -> TourInfo {
        TourInfo { number: number.into(), ai_group: String::new(), first: 0.0, last: 0.0, days: "Sat".into(), runs, next_run: None, trips }
    }

    /// Line 35's tour 1 drives two trips and line 25's tour 3 a third one: the shift of all
    /// three, as `compose` makes it.
    fn timetable() -> (Vec<LineInfo>, ComposedDuty) {
        let h = 3600.0;
        let lines = vec![
            LineInfo { name: "35".into(), user_allowed: true, termini: vec![], tours: vec![tour("1", true, vec![trip(1, "35", "A", "B", 11.0 * h, 11.5 * h, 12), trip(2, "35", "B", "A", 11.75 * h, 12.25 * h, 11)])] },
            LineInfo { name: "25".into(), user_allowed: true, termini: vec![], tours: vec![tour("3", true, vec![trip(4, "25", "C", "A", 10.0 * h, 10.5 * h, 9), trip(5, "25", "A", "C", 12.5 * h, 13.0 * h, 10)])] },
        ];
        let leg = |line: &str, tour: &str, t: &TripInfo| Leg { line: line.into(), tour: tour.into(), index: t.index, trip: t.name.clone(), shown: t.line.clone(), from: t.from.clone(), terminus: t.terminus.clone(), departure: t.departure, arrival: t.arrival };
        let duty = ComposedDuty { legs: vec![leg("35", "1", &lines[0].tours[0].trips[0]), leg("35", "1", &lines[0].tours[0].trips[1]), leg("25", "3", &lines[1].tours[0].trips[1])] };
        (lines, duty)
    }

    #[test]
    fn a_shift_tells_its_breaks_and_where_it_goes_on_with_another_tour() {
        let (lines, duty) = timetable();
        let c = shift_card(&duty, &lines);
        assert_eq!(c.key, vec!["35|1|1|2".to_string(), "25|3|5|1".to_string()]);
        assert_eq!(c.trips.iter().map(|t| minutes(t.pause)).collect::<Vec<_>>(), vec![0, 15, 15]);
        // only the trip of the other tour is taken over
        assert_eq!(c.trips.iter().map(|t| t.takes_over.clone()).collect::<Vec<_>>(), vec![None, None, Some(("25".to_string(), "3".to_string()))]);
        assert_eq!(c.trips.iter().map(|t| t.stops).sum::<usize>(), 33);
        assert_eq!(lines_of(&c.trips), vec!["35", "25"]);
        assert_eq!(c.cells[0], "11:00");
        assert_eq!(c.cells[1], "13:00");
        assert!(c.clock);
    }

    #[test]
    fn a_later_trip_of_the_same_tour_is_no_change_of_tour() {
        let (lines, mut duty) = timetable();
        // tour 1's first trip, then (after its second, which another driver takes) tour 3's
        duty.legs.remove(1);
        duty.legs.push(duty.legs[0].clone());
        duty.legs[2].index = 2;
        let c = shift_card(&duty, &lines);
        assert_eq!(c.key.len(), 3);
        assert_eq!(c.trips.iter().map(|t| t.takes_over.is_some()).collect::<Vec<_>>(), vec![false, true, true]);
        duty.legs[2] = duty.legs[1].clone();
        duty.legs[2].index = 9;
        assert_eq!(shift_card(&duty, &lines).trips[2].takes_over, None);
    }

    #[test]
    fn the_chosen_shift_is_read_back_from_its_legs() {
        let (lines, duty) = timetable();
        let c = shift_card(&duty, &lines);
        let back = shift_of_legs(&c.key, &lines).expect("the shift's tours are in the timetable");
        assert_eq!(back, duty);
        assert_eq!(shift_card(&back, &lines), c);
        // a tour gone from the timetable (another date): nothing to show
        assert_eq!(shift_of_legs(&["35|1|1|3".to_string()], &lines), None);
        assert_eq!(shift_of_legs(&["99|1|1|1".to_string()], &lines), None);
    }

    #[test]
    fn a_tour_on_another_day_is_quiet_and_says_when_it_runs() {
        let (lines, _) = timetable();
        let mut t = lines[1].tours[0].clone();
        let c = tour_card(&t);
        assert!(c.note.is_empty());
        assert_eq!(c.cells, ["3".to_string(), "10:00".to_string(), "13:00".to_string()]);
        assert_eq!(minutes(c.trips[1].pause), 120);
        assert!(c.trips.iter().all(|x| x.takes_over.is_none()));
        t.runs = false;
        assert_eq!(tour_card(&t).note, "Sat");
    }

    #[test]
    fn lengths_and_counts_are_written_as_omsi_hub_writes_them() {
        let as_written = |h: &str, mm: &str| omsi_ui::tr("%{h}h %{mm}m").replace("%{h}", h).replace("%{mm}", mm);
        assert_eq!(length(7500.0), as_written("2", "05"));
        assert_eq!(length(0.0), as_written("0", "00"));
        // one of a thing is said in the singular; no lines (a tour of depot runs) not at all
        let n = |key: &str, n: &str| omsi_ui::tr(key).replace("%{n}", n);
        assert_eq!(summary(1, 2, 1), format!("{} · {} · {}", n("%{n} trip", "1"), n("%{n} stops", "2"), n("%{n} line", "1")));
        assert_eq!(summary(3, 1, 2), format!("{} · {} · {}", n("%{n} trips", "3"), n("%{n} stop", "1"), n("%{n} lines", "2")));
        assert_eq!(summary(2, 5, 0), format!("{} · {}", n("%{n} trips", "2"), n("%{n} stops", "5")));
    }

    #[test]
    fn the_scale_is_a_round_length_that_fits() {
        assert_eq!(scale_of(5.0, 160.0), (500.0, 100.0));
        assert_eq!(scale_of(1.0, 160.0).0, 100.0);
        assert_eq!(scale_of(10.0, 160.0).0, 1000.0);
        assert!(scale_of(0.01, 160.0).1 <= 1000.0);
    }

    /// One frame of the list of `cards` in a sheet-sized rect.
    fn frame(ui: &mut Ui, cards: &[Card], chosen: Option<usize>, first: Option<usize>) -> (Option<usize>, Option<usize>) {
        ui.begin(Vec2::new(1440.0, 900.0), 1.0, 1.0 / 60.0);
        duty_list(ui, Rect::new(30.0, 300.0, 380.0, 520.0), cards, chosen, first, "")
    }

    /// Click `name`: the mouse goes down over it and comes up again. Returns what the list
    /// answered on the frame it came up.
    fn click(cards: &[Card], chosen: Option<usize>, first: Option<usize>, name: &str) -> (Option<usize>, Option<usize>) {
        let mut ui = Ui::new();
        frame(&mut ui, cards, chosen, first);
        let r = *ui.drawn.get(&id_of(name)).unwrap_or_else(|| panic!("{name} is not drawn"));
        ui.input.mouse = r.center();
        ui.input.pressed = true;
        ui.input.down = true;
        frame(&mut ui, cards, chosen, first);
        ui.input.pressed = false;
        ui.input.down = false;
        ui.input.released = true;
        frame(&mut ui, cards, chosen, first)
    }

    #[test]
    fn a_row_and_a_trip_of_the_opened_one_are_clicked() {
        let (lines, duty) = timetable();
        let shift = shift_card(&duty, &lines);
        let tour = tour_card(&lines[0].tours[0]);
        let cards = vec![shift.clone(), tour.clone(), shift];
        assert_eq!(click(&cards, Some(0), None, "shift-list-2"), (Some(2), None));
        // a shift's trips are no buttons; a tour's are
        let mut ui = Ui::new();
        frame(&mut ui, &cards, Some(0), None);
        assert!(!ui.drawn.contains_key(&id_of("shift-list-trip-1")));
        assert_eq!(click(&cards, Some(1), Some(0), "shift-list-trip-1"), (None, Some(1)));
        // the trip it starts with already: nothing to change
        assert_eq!(click(&cards, Some(1), Some(1), "shift-list-trip-1"), (None, None));
    }

    #[test]
    fn the_opened_row_is_as_tall_as_its_trips_are_drawn() {
        let (lines, duty) = timetable();
        let c = tour_card(&lines[1].tours[0]);
        let cards = vec![c.clone()];
        let mut ui = Ui::new();
        frame(&mut ui, &cards, Some(0), Some(0));
        // the second trip's button stands where `trip_top` says (under the row, after the
        // break before it), so the list scrolls to the trip and not beside it
        let r = ui.drawn[&id_of("shift-list-trip-1")];
        assert_eq!(r.y, 300.0 + ROW_H - 2.0 + trip_top(&c, 1, Some(0)));
        assert!(trip_top(&c, 2, Some(0)) < detail_height(&c, Some(0)));
        // a break before the trip a tour starts with, or before one it does not drive, is not said
        assert_eq!(trip_top(&c, 1, Some(1)), trip_top(&c, 1, Some(0)) - 18.0);
        let s = shift_card(&duty, &lines);
        assert_eq!(detail_height(&s, None), 30.0 + 3.0 * 24.0 + 2.0 * 18.0 + 20.0 + 10.0);
    }
}
