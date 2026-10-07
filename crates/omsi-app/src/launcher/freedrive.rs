//! Free drive as Omsi-Hub has it: a map, a place to start, the day and a bus - nothing is
//! booked and nobody keeps the time. The place to start is the step of its own (Luc: "bij
//! het beginpunt zien welke lijnen er zijn vanaf die halte en of het een beginpunt of een
//! tussenstop is - gecategoriseerd"): the day's stops in two groups - the starting points,
//! where trips begin, and the intermediate stops buses only call at - each with the plates of
//! the lines that serve it and how many buses do, and the map's own entry points, which are
//! where the game puts a bus down. A stop chosen puts the bus at the entry point nearest to
//! it, and the map marks both.
//!
//! The switch at the top makes it a drive along a line instead ("Choose a line yourself"): a
//! line and one of its routes - a direction, or a variant of one - and the bus starts at the
//! route's first stop with the game knowing the line, so the navigator draws the way and the
//! IBIS can be typed for it. Still nothing is booked: the game drives it as a free drive
//! (`--free-line`, see `Schedule::free_line_duty`).

use super::daytime::chips;
use super::flow::radio;
use super::ownlines;
use super::state::Choice;
use super::theme::*;
use super::ui::{ButtonKind, Ui};
use super::Launcher;
use glam::{DVec2, Vec2};
use omsi_launcher_lib::lines::{is_own_file, own_line_of, OwnLine};
use omsi_launcher_lib::{LineInfo, TripInfo};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

#[derive(Default)]
pub struct FreeView {
    /// The list of places shown: 0 the starting points, 1 the intermediate stops, 2 the map's
    /// entry points.
    pub tab: usize,
    /// What the lists are narrowed to.
    pub filter: String,
    /// The lines are listed again over the one chosen ("Other line").
    pub pick_line: bool,
    /// The entry point chosen on the map is to be brought into the list's view.
    reveal: bool,
    /// The day's stops, and the timetable they were counted from (map, date, its lines, and
    /// whether another was being read: a day's lines may come as many as the day's before).
    stops: (Vec<StartStop>, Vec<StartStop>),
    stops_for: (String, String, usize, bool),
}

/// A stop of the day's timetable as the start point lists it: by its name (its platforms
/// and directions are one place to start at), with its map objects, the lines calling there
/// in number order, and how many trips begin there and how many call on their way.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StartStop {
    pub name: String,
    pub ids: Vec<i64>,
    pub lines: Vec<String>,
    pub begins: usize,
    pub passes: usize,
}

/// One way a line runs: a route (its trip file), where it begins and ends, how long it is,
/// and how many buses take it on the day.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub trip: String,
    pub line: String,
    pub from: String,
    pub terminus: String,
    pub stops: usize,
    pub km: f64,
    pub runs: usize,
}

/// The number a trip's displays show; a depot run has none.
fn plate_of(trip: &TripInfo) -> String {
    trip.line.trim().to_string()
}

/// The lines a player may drive when the map marks any (`[userallowed]`, as OMSI's timetable
/// dialog lists them), else all.
fn drivable(lines: &[LineInfo]) -> Vec<&LineInfo> {
    // (the player's own lines are marked so too: whether the map marks any is the map's say,
    // or a map that marks none lost all its lines to the first line of the player's)
    let marked = lines.iter().any(|l| l.user_allowed && !is_own_file(&l.name));
    lines.iter().filter(|l| !marked || l.user_allowed).collect()
}

/// The trips that leave on the day: those of the tours that run then (all of them when none
/// does - the date is then one the timetable does not know, and every trip is a guess).
fn trips_of_day<'a>(lines: &[&'a LineInfo]) -> Vec<(&'a LineInfo, &'a TripInfo)> {
    let any_runs = lines.iter().any(|l| l.tours.iter().any(|t| t.runs));
    let mut out = Vec::new();
    for &l in lines {
        for t in l.tours.iter().filter(|t| t.runs || !any_runs) {
            out.extend(t.trips.iter().map(|trip| (l, trip)));
        }
    }
    out
}

fn by_number(list: &mut [String]) {
    list.sort_by(|a, b| super::drive::natural(a).cmp(&super::drive::natural(b)));
}

/// The day's stops as two lists: the starting points (trips begin there; the most departures
/// first) and the intermediate stops (buses only call there; the most buses first). A trip
/// counts as often as the tours have it: that is how often a bus really leaves.
pub fn start_stops(lines: &[LineInfo]) -> (Vec<StartStop>, Vec<StartStop>) {
    let mut all: Vec<StartStop> = Vec::new();
    let mut at: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (_, trip) in trips_of_day(&drivable(lines)) {
        let plate = plate_of(trip);
        for (i, stop) in trip.stops.iter().enumerate() {
            let name = stop.name.trim();
            if name.is_empty() {
                continue;
            }
            let k = *at.entry(name.to_string()).or_insert_with(|| {
                all.push(StartStop { name: name.to_string(), ..Default::default() });
                all.len() - 1
            });
            let s = &mut all[k];
            if !s.ids.contains(&stop.id) {
                s.ids.push(stop.id);
            }
            if !plate.is_empty() && !s.lines.contains(&plate) {
                s.lines.push(plate.clone());
            }
            if i == 0 {
                s.begins += 1;
            } else {
                s.passes += 1;
            }
        }
    }
    for s in &mut all {
        by_number(&mut s.lines);
    }
    let (mut begin, mut via): (Vec<StartStop>, Vec<StartStop>) = all.into_iter().partition(|s| s.begins > 0);
    begin.sort_by(|a, b| b.begins.cmp(&a.begins).then_with(|| a.name.cmp(&b.name)));
    via.sort_by(|a, b| b.passes.cmp(&a.passes).then_with(|| a.name.cmp(&b.name)));
    (begin, via)
}

/// The lines a free drive can follow, with the numbers their displays show (the line's own
/// name when its trips show none) and their termini.
pub fn line_rows(lines: &[LineInfo]) -> Vec<(String, Vec<String>, String)> {
    drivable(lines)
        .into_iter()
        .filter(|l| l.tours.iter().any(|t| !t.trips.is_empty()))
        .map(|l| {
            let mut plates: Vec<String> = Vec::new();
            for t in l.tours.iter().flat_map(|t| t.trips.iter()) {
                let p = plate_of(t);
                if !p.is_empty() && !plates.contains(&p) {
                    plates.push(p);
                }
            }
            by_number(&mut plates);
            if plates.is_empty() {
                plates.push(l.name.clone());
            }
            (l.name.clone(), plates, l.termini.join(" · "))
        })
        .collect()
}

/// The routes of a line, the most driven first (and the service trips last): each trip file
/// once, with how many buses take it on the day. A route of fewer than two stops goes nowhere and is left out.
pub fn routes_of(line: &LineInfo) -> Vec<Route> {
    let mut out: Vec<Route> = Vec::new();
    for (_, trip) in trips_of_day(&[line]) {
        if trip.stops.len() < 2 {
            continue;
        }
        match out.iter_mut().find(|r| r.trip == trip.name) {
            Some(r) => r.runs += 1,
            None => out.push(Route { trip: trip.name.clone(), line: plate_of(trip), from: trip.from.clone(), terminus: trip.terminus.clone(), stops: trip.stops.len(), km: trip.km, runs: 1 }),
        }
    }
    // (a route only on other days: still a way the line runs)
    for trip in line.tours.iter().flat_map(|t| t.trips.iter()) {
        if trip.stops.len() >= 2 && !out.iter().any(|r| r.trip == trip.name) {
            out.push(Route { trip: trip.name.clone(), line: plate_of(trip), from: trip.from.clone(), terminus: trip.terminus.clone(), stops: trip.stops.len(), km: trip.km, runs: 0 });
        }
    }
    // (the service trips without a number on their displays last)
    out.sort_by(|a, b| a.line.is_empty().cmp(&b.line.is_empty()).then_with(|| b.runs.cmp(&a.runs)).then_with(|| a.terminus.cmp(&b.terminus)).then_with(|| a.from.cmp(&b.from)));
    out
}

/// The line and route a free drive follows, when the player chose one (`State::duty`).
pub(super) fn free_route(c: &Choice) -> Option<(String, String)> {
    (c.free && c.own_line && !c.free_line.trim().is_empty() && !c.free_route.trim().is_empty()).then(|| (c.free_line.clone(), c.free_route.clone()))
}

/// The chosen route in the day's timetable: its line and its trip.
fn find_route<'a>(lines: &'a [LineInfo], line: &str, route: &str) -> Option<(&'a LineInfo, &'a TripInfo)> {
    let l = lines.iter().find(|x| x.name == line)?;
    let t = l.tours.iter().flat_map(|t| t.trips.iter()).find(|t| t.name == route)?;
    Some((l, t))
}

/// The bar's plate in a free drive: the number the chosen route's displays show.
pub(super) fn chosen_plate(l: &Launcher) -> Option<String> {
    let (line, route) = free_route(&l.state.choice)?;
    let (li, t) = find_route(&l.state.lines, &line, &route)?;
    Some(if plate_of(t).is_empty() { li.name.clone() } else { plate_of(t) })
}

/// What the map shows in a free drive: the route of the line followed (the map alone when
/// there is none), and the entry point chosen.
pub(super) fn map_look(l: &Launcher) -> super::mapview::Look {
    let mut look = super::drive::map_look(l);
    let route = free_route(&l.state.choice);
    look.trips = route.as_ref().map(|r| vec![r.1.clone()]).unwrap_or_default();
    // (along a line the bus goes to the entry point nearest the route's first stop: none is
    // ringed)
    look.entry = if route.is_some() { -1 } else { l.state.choice.free_entry };
    look
}

/// An entry point was clicked on the map in a free drive (`map_interact` put it into the
/// duty's `entry`, which was `before`): that is where the free drive's bus starts now - not
/// at a stop, and not along a line. The duty's own start stays as it was.
pub(super) fn entry_clicked(l: &mut Launcher, before: i32) {
    let c = &mut l.state.choice;
    c.free_entry = c.entry;
    c.entry = before;
    c.free_stop.clear();
    c.own_line = false;
    l.free.tab = 2;
    l.free.filter.clear();
    l.free.reveal = true;
    l.state.touched();
}

/// The day's stops, counted again when the timetable read is another one.
fn refresh(l: &mut Launcher) {
    let key = (l.state.lines_for.0.clone(), l.state.lines_for.1.clone(), l.state.lines.len(), l.state.loading_lines);
    if l.free.stops_for != key {
        l.free.stops = start_stops(&l.state.lines);
        l.free.stops_for = key;
    }
}

/// Where a stop stands on the map: the middle of those of its platforms the map has placed.
fn stop_place(l: &Launcher, name: &str) -> Option<DVec2> {
    let (b, v) = &l.free.stops;
    let s = b.iter().chain(v.iter()).find(|s| s.name == name)?;
    let places: Vec<DVec2> = s.ids.iter().filter_map(|id| l.mapview.object_place(*id)).collect();
    (!places.is_empty()).then(|| places.iter().fold(DVec2::ZERO, |a, p| a + *p) / places.len() as f64)
}

/// The entry point a stop chosen to start at puts the bus at: the nearest to it, once the map
/// is read (the choice keeps it in `free_entry`, so a start before the map is read again
/// still has it).
fn resolve_stop(l: &mut Launcher) -> Option<f64> {
    let c = &l.state.choice;
    if !c.free || c.own_line || c.free_stop.is_empty() {
        return None;
    }
    let at = stop_place(l, &c.free_stop)?;
    let (e, metres) = l.mapview.nearest_entry(at)?;
    if e as i32 != l.state.choice.free_entry {
        l.state.choice.free_entry = e as i32;
        l.state.touched();
    }
    Some(metres)
}

/// An entry point's name, as the map's list numbers it.
fn entry_name(l: &Launcher, e: i32) -> Option<String> {
    let x = l.state.map()?.entry_points.get(usize::try_from(e).ok()?)?;
    Some(if x.name.is_empty() { format!("{} {}", omsi_ui::tr("entry"), x.index + 1) } else { x.name.clone() })
}

/// The start in a few words, under the sheet's title.
pub(super) fn start_line(l: &Launcher) -> String {
    let c = &l.state.choice;
    if c.own_line {
        return match free_route(c).and_then(|(line, route)| find_route(&l.state.lines, &line, &route)) {
            Some((li, t)) => omsi_ui::tr("Line %{line} to %{terminus}").replace("%{line}", &if plate_of(t).is_empty() { li.name.clone() } else { plate_of(t) }).replace("%{terminus}", t.terminus.trim()),
            None => omsi_ui::tr("Choose a line yourself").into_owned(),
        };
    }
    if !c.free_stop.is_empty() {
        return c.free_stop.clone();
    }
    entry_name(l, c.free_entry).unwrap_or_else(|| omsi_ui::tr("Automatic").into_owned())
}

/// Where the bus will stand, said in full: the sheet's foot.
fn start_foot(l: &Launcher, metres: Option<f64>) -> String {
    let c = &l.state.choice;
    if c.own_line {
        return match free_route(c).and_then(|(line, route)| find_route(&l.state.lines, &line, &route)) {
            Some((_, t)) => omsi_ui::tr("Your bus starts at the entry point nearest to %{stop}, the route's first stop. Nothing is booked: drive the line as you like.").replace("%{stop}", t.from.trim()),
            None => omsi_ui::tr("Choose a line, then the way it goes: the bus starts at its first stop.").into_owned(),
        };
    }
    if !c.free_stop.is_empty() {
        return match (entry_name(l, c.free_entry), metres) {
            (Some(e), Some(m)) => omsi_ui::tr("Your bus stands at %{entry}, the entry point nearest to %{stop} (%{distance} away).").replace("%{entry}", &e).replace("%{stop}", &c.free_stop).replace("%{distance}", &distance(m)),
            _ => omsi_ui::tr("Your bus stands at the entry point nearest to %{stop}.").replace("%{stop}", &c.free_stop),
        };
    }
    match entry_name(l, c.free_entry) {
        Some(e) => omsi_ui::tr("Your bus stands at %{entry}. The orange marks on the map are the entry points: click one to start there.").replace("%{entry}", &e),
        None => omsi_ui::tr("Your bus stands at the map's first entry point. Choose a stop, or click an orange mark on the map.").into_owned(),
    }
}

fn distance(m: f64) -> String {
    if m >= 1000.0 {
        format!("{:.1} km", m / 1000.0)
    } else {
        format!("{:.0} m", (m / 10.0).round() * 10.0)
    }
}

/// The yellow plate of a line number, drawn on a `Ui` (inside a list's closure).
fn plate(ui: &mut Ui, at: Vec2, line: &str, h: f32) -> f32 {
    let px = h * 0.6;
    let w = (ui.width(line, px, Weight::Black) + h * 0.7).max(h * 1.8);
    let r = Rect::new(at.x, at.y, w, h);
    ui.p().rounded(r, 5.0, LINE);
    ui.text_in(line, r, px, Weight::Black, ON_LINE, Align::Center);
    w
}

/// Plates side by side within `w`; what does not fit is counted ("+3").
fn plates(ui: &mut Ui, at: Vec2, w: f32, lines: &[String], h: f32, ink: Color) {
    let mut x = at.x;
    for (k, line) in lines.iter().enumerate() {
        let need = (ui.width(line, h * 0.6, Weight::Black) + h * 0.7).max(h * 1.8);
        let rest = lines.len() - k;
        if x + need > at.x + w - if rest > 1 { 30.0 } else { 0.0 } {
            ui.text_in(&format!("+{rest}"), Rect::new(x, at.y, 30.0, h), 11.5, Weight::Bold, ink, Align::Left);
            return;
        }
        x += plate(ui, Vec2::new(x, at.y), line, h) + 5.0;
    }
}

/// The sheet's foot: a hairline, an info mark and the start in full, on up to three lines.
/// Returns the space above it.
fn foot(l: &mut Launcher, s: Rect, from_y: f32, text: &str) -> Rect {
    let tw = s.w - 62.0;
    let th = l.ui.paragraph_height(text, tw, 12.5, Weight::Regular).min(54.0);
    let h = (th + 26.0).max(46.0);
    let y = s.bottom() - h;
    l.ui.p().rect(Rect::new(s.x, y, s.w, 1.0), HAIRLINE);
    l.ui.icon("info", Vec2::new(s.x + 26.0, y + 22.0), 14.0, TEXT_DIM);
    l.ui.push_clip(Rect::new(s.x + 40.0, y + 6.0, tw + 4.0, h - 10.0), 0.0);
    l.ui.paragraph(text, Vec2::new(s.x + 42.0, y + 13.0), tw, 12.5, Weight::Regular, TEXT_SOFT);
    l.ui.pop_clip();
    Rect::new(s.x, from_y, s.w, (y - from_y).max(0.0))
}

/// Over the sheet's foot, quietly: that in the game the city map sets a destination of one's
/// own, to drive to with the navigator (`nav_pins`). Returns the space above it.
fn pins_hint(l: &mut Launcher, r: Rect) -> Rect {
    // (both translate it)
    let text = "In the game, the city map (Shift+M) sets a destination: right-click it, or use its pin tool.";
    let (x, w) = (r.x + 18.0, r.w - 36.0);
    let th = l.ui.paragraph_height(text, w - 24.0, 12.0, Weight::Regular).min(34.0);
    let y = r.bottom() - th - 12.0;
    l.ui.icon("location_on", Vec2::new(x + 8.0, y + 8.0), 14.0, TEXT_DIM);
    l.ui.push_clip(Rect::new(x + 20.0, y - 2.0, w - 20.0, th + 6.0), 0.0);
    l.ui.paragraph(text, Vec2::new(x + 24.0, y), w - 24.0, 12.0, Weight::Regular, TEXT_DIM);
    l.ui.pop_clip();
    Rect::new(r.x, r.y, r.w, (y - 10.0 - r.y).max(0.0))
}

/// The start point sheet under its head (`flow` draws the sheet, the head and the actions):
/// the switch between a free start and a line, and the list that goes with it.
pub(super) fn start_panel(l: &mut Launcher, s: Rect, body: Rect) {
    refresh(l);
    let metres = resolve_stop(l);
    let text = start_foot(l, metres);
    let rest = foot(l, s, body.y, &text);
    let rest = pins_hint(l, rest);
    let x = body.x + 18.0;
    let w = body.w - 36.0;
    let mut y = body.y;
    let mut kind = usize::from(l.state.choice.own_line);
    let switch = Rect::new(x, y, w, 34.0);
    super::tour::anchor("free-line", Rect::new(switch.x + switch.w * 0.5, switch.y, switch.w * 0.5, switch.h));
    if l.ui.segmented("free-kind", switch, &mut kind, &["Free", "Choose a line yourself"]) {
        l.state.choice.own_line = kind == 1;
        l.free.filter.clear();
        l.free.pick_line = false;
        l.state.touched();
    }
    l.ui.tooltip(switch, "Free: the bus starts where you choose, without a line. Choose a line yourself: it starts at the first stop of the route you pick, and the navigator and the IBIS know the line - nothing is booked either way.");
    y += 46.0;
    let area = Rect::new(x, y, w, (rest.bottom() - y - 10.0).max(60.0));
    if l.state.choice.own_line {
        line_body(l, area);
    } else {
        super::tour::anchor("free-starts", area);
        place_body(l, area);
    }
}

/// A quiet line where a list has nothing to show.
fn empty_note(l: &mut Launcher, r: Rect, text: &str) {
    l.ui.paragraph(text, Vec2::new(r.x + 4.0, r.y + 18.0), r.w - 8.0, 13.0, Weight::Regular, TEXT_DIM);
}

/// The column heads over a list: on the left, and on the right.
fn heads(ui: &mut Ui, x: f32, y: f32, w: f32, left: &str, right: &str) {
    ui.text_in(&omsi_ui::tr(left).to_uppercase(), Rect::new(x + 36.0, y, w * 0.6, 16.0), 10.0, Weight::Bold, TEXT_DIM, Align::Left);
    if !right.is_empty() {
        ui.text_in(&omsi_ui::tr(right).to_uppercase(), Rect::new(x + w * 0.4, y, w * 0.6 - 14.0, 16.0), 10.0, Weight::Bold, TEXT_DIM, Align::Right);
    }
}

/// Free: the starting points, the intermediate stops or the entry points - chips to pick
/// the list, a search, and the list.
fn place_body(l: &mut Launcher, r: Rect) {
    let (begin, via) = std::mem::take(&mut l.free.stops);
    let entries: Vec<String> = l.state.map().map(|m| m.entry_points.iter().map(|e| if e.name.is_empty() { format!("{} {}", omsi_ui::tr("entry"), e.index + 1) } else { e.name.clone() }).collect()).unwrap_or_default();
    // (a map without a timetable has no stops: its entry points are all there is)
    if begin.is_empty() && via.is_empty() && !l.state.loading_lines && l.free.tab < 2 {
        l.free.tab = 2;
    }
    let labels = vec![
        omsi_ui::tr("Starting points (%{n})").replace("%{n}", &begin.len().to_string()),
        omsi_ui::tr("Intermediate stops (%{n})").replace("%{n}", &via.len().to_string()),
        omsi_ui::tr("Entry points (%{n})").replace("%{n}", &entries.len().to_string()),
    ];
    let mut y = r.y;
    let (pick, h) = chips(&mut l.ui, "free-tab", r.x, y, r.w, &labels, l.free.tab);
    if let Some(k) = pick {
        l.free.tab = k;
    }
    y += h + 12.0;
    l.ui.text_input("free-filter", Rect::new(r.x, y, r.w, 34.0), &mut l.free.filter, if l.free.tab == 2 { "Search entry points…" } else { "Search stops…" }, Some("search"));
    y += 46.0;
    let q = l.free.filter.trim().to_lowercase();
    let list = Rect::new(r.x - 6.0, y + 22.0, r.w + 12.0, (r.bottom() - y - 22.0).max(40.0));
    let c = &l.state.choice;
    let (chosen_stop, chosen_entry) = (c.free_stop.clone(), if c.free_stop.is_empty() { c.free_entry } else { -2 });
    if l.free.tab < 2 {
        let stops: Vec<&StartStop> = (if l.free.tab == 0 { begin.iter() } else { via.iter() }).filter(|s| q.is_empty() || s.name.to_lowercase().contains(&q) || s.lines.iter().any(|x| x.to_lowercase() == q)).collect();
        heads(&mut l.ui, list.x + 6.0, y, list.w - 12.0, "Bus stop", if l.free.tab == 0 { "Departures" } else { "Buses calling" });
        if stops.is_empty() {
            let note = if l.state.loading_lines { "Reading the timetable…" } else if q.is_empty() { "The timetable has no such stops on this day." } else { "No stop of that name." };
            empty_note(l, list, note);
        } else {
            let mut picked = None;
            let begins = l.free.tab == 0;
            l.ui.scroll_area(&format!("free-stops-{}", l.free.tab), list, &mut |ui, v| {
                const ROW_H: f32 = 54.0;
                for (k, s) in stops.iter().enumerate() {
                    let rr = Rect::new(v.x + 6.0, v.y + k as f32 * ROW_H, v.w - 18.0, ROW_H - 3.0);
                    if !ui.rect_visible(rr) {
                        continue;
                    }
                    let on = s.name == chosen_stop;
                    if ui.row(&format!("free-stop-{}", s.name), rr, on) {
                        picked = Some(s.name.clone());
                    }
                    radio(ui, Vec2::new(rr.x + 16.0, rr.y + 17.0), on);
                    let ink = if on { on_accent() } else { TEXT };
                    let count = if begins { s.begins } else { s.passes };
                    ui.text_in(&s.name, Rect::new(rr.x + 36.0, rr.y + 7.0, rr.w - 36.0 - 52.0, 20.0), 14.0, Weight::Bold, ink, Align::Left);
                    ui.text_in(&count.to_string(), Rect::new(rr.right() - 52.0, rr.y + 7.0, 44.0, 20.0), 14.0, Weight::Bold, ink, Align::Right);
                    if s.lines.is_empty() {
                        ui.text_in("service trips", Rect::new(rr.x + 36.0, rr.y + 29.0, rr.w - 44.0, 17.0), 11.5, Weight::Regular, if on { on_accent() } else { TEXT_DIM }, Align::Left);
                    } else {
                        plates(ui, Vec2::new(rr.x + 36.0, rr.y + 29.0), rr.w - 44.0, &s.lines, 17.0, if on { on_accent() } else { TEXT_SOFT });
                    }
                }
                stops.len() as f32 * ROW_H
            });
            if let Some(name) = picked {
                let c = &mut l.state.choice;
                c.free_stop = name;
                l.state.touched();
                resolve_stop(l);
            }
        }
    } else {
        heads(&mut l.ui, list.x + 6.0, y, list.w - 12.0, "Entry point", "");
        let mut rows: Vec<(i32, String)> = vec![(-1, omsi_ui::tr("Automatic (the map's first)").into_owned())];
        rows.extend(entries.iter().enumerate().map(|(k, n)| (k as i32, n.clone())));
        rows.retain(|(_, n)| q.is_empty() || n.to_lowercase().contains(&q));
        if std::mem::take(&mut l.free.reveal) {
            if let Some(k) = rows.iter().position(|(e, _)| *e == chosen_entry) {
                l.ui.scroll_to("free-entries", k as f32 * 40.0, 40.0, list.h);
            }
        }
        let mut picked = None;
        l.ui.scroll_area("free-entries", list, &mut |ui, v| {
            const ROW_H: f32 = 40.0;
            for (k, (e, name)) in rows.iter().enumerate() {
                let rr = Rect::new(v.x + 6.0, v.y + k as f32 * ROW_H, v.w - 18.0, ROW_H - 3.0);
                if !ui.rect_visible(rr) {
                    continue;
                }
                let on = *e == chosen_entry;
                if ui.row(&format!("free-entry-{e}"), rr, on) {
                    picked = Some(*e);
                }
                radio(ui, Vec2::new(rr.x + 16.0, rr.center().y), on);
                ui.text_in(name, Rect::new(rr.x + 36.0, rr.y, rr.w - 44.0, rr.h), 13.5, if *e < 0 { Weight::Medium } else { Weight::Bold }, if on { on_accent() } else { TEXT }, Align::Left);
            }
            rows.len() as f32 * ROW_H
        });
        if let Some(e) = picked {
            let c = &mut l.state.choice;
            c.free_entry = e;
            c.free_stop.clear();
            l.state.touched();
        }
    }
    l.free.stops = (begin, via);
}

/// Along a line: the lines to choose from, or the routes of the one chosen.
fn line_body(l: &mut Launcher, r: Rect) {
    let chosen = l.state.choice.free_line.clone();
    let line = l.state.lines.iter().find(|x| x.name == chosen).map(|x| (routes_of(x), line_rows(std::slice::from_ref(x)).into_iter().next().map(|r| r.1).unwrap_or_else(|| vec![x.name.clone()]), own_line_of(&x.name, &l.state.own_lines)));
    match line {
        Some((routes, plates, own)) if !l.free.pick_line => route_list(l, r, &routes, &plates, own.as_ref()),
        _ => line_list(l, r),
    }
}

/// A line of the free drive's list: its timetable name, plates and termini, how many routes
/// it has, and what the line editor knows of it when it is the player's.
type LineRow = (String, Vec<String>, String, usize, Option<OwnLine>);

/// The lines, each with its plates and termini, and how many routes it has: the map's, or
/// the player's own (a switch over them).
fn line_list(l: &mut Launcher, r: Rect) {
    let mut y = r.y;
    let all: Vec<LineRow> = line_rows(&l.state.lines)
        .into_iter()
        .map(|(name, plates, termini)| {
            let n = l.state.lines.iter().find(|x| x.name == name).map(|x| routes_of(x).len()).unwrap_or(0);
            let own = own_line_of(&name, &l.state.own_lines);
            (name, plates, termini, n, own)
        })
        .collect();
    let (mine, map): (Vec<LineRow>, Vec<LineRow>) = all.into_iter().partition(|x| x.4.is_some());
    match ownlines::switch(&mut l.ui, "free-line-source", Rect::new(r.x, y, r.w, 30.0), l.state.choice.my_lines, (map.len(), mine.len())) {
        ownlines::Switched::To(m) => {
            l.state.choice.my_lines = m;
            l.free.filter.clear();
            l.state.touched();
        }
        ownlines::Switched::Hint => l.state.set_status(omsi_ui::tr(ownlines::NONE_YET), false),
        ownlines::Switched::No => {}
    }
    y += 40.0;
    let showing_mine = ownlines::showing_mine(l.state.choice.my_lines, mine.len());
    l.ui.text_input("free-line-filter", Rect::new(r.x, y, r.w, 34.0), &mut l.free.filter, if showing_mine { "Search your lines…" } else { "Search lines…" }, Some("search"));
    y += 46.0;
    let q = l.free.filter.trim().to_lowercase();
    let rows: Vec<LineRow> = (if showing_mine { mine } else { map })
        .into_iter()
        .filter(|(name, plates, termini, _, own)| {
            q.is_empty() || name.to_lowercase().contains(&q) || plates.iter().any(|p| p.to_lowercase().contains(&q)) || termini.to_lowercase().contains(&q) || own.as_ref().is_some_and(|o| o.caption().to_lowercase().contains(&q))
        })
        .collect();
    let list = Rect::new(r.x - 6.0, y + 22.0, r.w + 12.0, (r.bottom() - y - 22.0).max(40.0));
    heads(&mut l.ui, list.x - 20.0, y, list.w + 14.0, "Line", "Routes");
    if rows.is_empty() {
        let note = if l.state.loading_lines { "Reading the timetable…" } else if q.is_empty() { "This map has no timetable: drive free instead." } else { "No line like that." };
        empty_note(l, list, note);
        return;
    }
    let chosen = l.state.choice.free_line.clone();
    let mut picked = None;
    l.ui.scroll_area("free-lines", list, &mut |ui, v| {
        const ROW_H: f32 = 54.0;
        for (k, (name, pl, termini, n, own)) in rows.iter().enumerate() {
            let rr = Rect::new(v.x + 6.0, v.y + k as f32 * ROW_H, v.w - 18.0, ROW_H - 3.0);
            if !ui.rect_visible(rr) {
                continue;
            }
            let on = *name == chosen;
            if ui.row(&format!("free-line-{name}"), rr, on) {
                picked = Some(name.clone());
            }
            // (a line of the player's: its plate in its colour, its name and where it goes)
            let t = match own {
                Some(o) => {
                    ownlines::plate(ui, Vec2::new(rr.x + 10.0, rr.y + 7.0), &o.number, &o.colour, 20.0);
                    let c = o.caption();
                    if c.is_empty() { termini.clone() } else { c }
                }
                None => {
                    plates(ui, Vec2::new(rr.x + 10.0, rr.y + 7.0), rr.w - 70.0, pl, 20.0, if on { on_accent() } else { TEXT_SOFT });
                    if termini.is_empty() { name.clone() } else { termini.clone() }
                }
            };
            ui.text_in(&n.to_string(), Rect::new(rr.right() - 52.0, rr.y + 7.0, 44.0, 20.0), 14.0, Weight::Bold, if on { on_accent() } else { TEXT }, Align::Right);
            ui.text_in(&t, Rect::new(rr.x + 10.0, rr.y + 31.0, rr.w - 20.0, 16.0), 11.5, Weight::Regular, if on { on_accent() } else { TEXT_DIM }, Align::Left);
        }
        rows.len() as f32 * ROW_H
    });
    if let Some(name) = picked {
        // the line, with its most driven route to begin with
        let first = l.state.lines.iter().find(|x| x.name == name).and_then(|x| routes_of(x).into_iter().next()).map(|x| x.trip).unwrap_or_default();
        let c = &mut l.state.choice;
        c.free_line = name;
        c.free_route = first;
        l.free.pick_line = false;
        l.free.filter.clear();
        l.state.touched();
    }
}

/// The routes of the chosen line (its plates `pl`): where each goes and from where, and how
/// often it runs.
fn route_list(l: &mut Launcher, r: Rect, routes: &[Route], pl: &[String], own: Option<&OwnLine>) {
    let mut y = r.y;
    let other = Rect::new(r.right() - 116.0, y, 116.0, 32.0);
    match own {
        // (a line of the player's: its plate in its colour, and its name)
        Some(o) => {
            let w = ownlines::plate(&mut l.ui, Vec2::new(r.x, y + 3.0), &o.number, &o.colour, 26.0);
            let at = r.x + w + 10.0;
            l.ui.text_in(&o.name, Rect::new(at, y, (other.x - at - 10.0).max(0.0), 32.0), 14.0, Weight::Bold, TEXT, Align::Left);
        }
        None => plates(&mut l.ui, Vec2::new(r.x, y + 3.0), other.x - r.x - 10.0, pl, 26.0, TEXT_SOFT),
    }
    if l.ui.button("free-other-line", other, "Other line", Some("alt_route"), ButtonKind::Normal) {
        l.free.pick_line = true;
        l.free.filter.clear();
    }
    y += 44.0;
    let list = Rect::new(r.x - 6.0, y + 22.0, r.w + 12.0, (r.bottom() - y - 22.0).max(40.0));
    heads(&mut l.ui, list.x + 6.0, y, list.w - 12.0, "Direction", "Today");
    let chosen = l.state.choice.free_route.clone();
    let mut picked = None;
    l.ui.scroll_area("free-routes", list, &mut |ui, v| {
        const ROW_H: f32 = 56.0;
        for (k, route) in routes.iter().enumerate() {
            let rr = Rect::new(v.x + 6.0, v.y + k as f32 * ROW_H, v.w - 18.0, ROW_H - 3.0);
            if !ui.rect_visible(rr) {
                continue;
            }
            let on = route.trip == chosen;
            if ui.row(&format!("free-route-{}", route.trip), rr, on) {
                picked = Some(route.trip.clone());
            }
            radio(ui, Vec2::new(rr.x + 16.0, rr.y + 17.0), on);
            let ink = if on { on_accent() } else { TEXT };
            let mut x = rr.x + 36.0;
            // (a route of another number than the line's - a night or short variant - says so)
            if !route.line.is_empty() && pl.len() > 1 {
                x += plate(ui, Vec2::new(x, rr.y + 7.0), &route.line, 18.0) + 6.0;
            }
            ui.text_in(&format!("→ {}", route.terminus.trim()), Rect::new(x, rr.y + 6.0, rr.right() - 52.0 - x, 20.0), 14.0, Weight::Bold, ink, Align::Left);
            let runs = if route.runs == 0 { "-".to_string() } else { format!("{}×", route.runs) };
            ui.text_in(&runs, Rect::new(rr.right() - 52.0, rr.y + 6.0, 44.0, 20.0), 13.5, Weight::Bold, ink, Align::Right);
            let what = omsi_ui::tr("from %{stop} · %{n} stops · %{km} km").replace("%{stop}", route.from.trim()).replace("%{n}", &route.stops.to_string()).replace("%{km}", &format!("{:.1}", route.km));
            ui.text_in(&what, Rect::new(rr.x + 36.0, rr.y + 30.0, rr.w - 44.0, 17.0), 11.5, Weight::Regular, if on { on_accent() } else { TEXT_DIM }, Align::Left);
        }
        routes.len() as f32 * ROW_H
    });
    if let Some(t) = picked {
        l.state.choice.free_route = t;
        l.state.touched();
    }
}

/// The free drive's marks on the map: the stop chosen to start at (its "H", and its name),
/// or the names along the route followed, its first stop marked as the start.
pub(super) fn map_marks(l: &mut Launcher, map: Rect, avoid: &[Rect]) {
    refresh(l);
    resolve_stop(l);
    let inside = |r: &Rect| r.x >= map.x + 4.0 && r.right() <= map.right() - 4.0 && r.y >= map.y + 4.0 && r.bottom() <= map.bottom() - 4.0;
    let hits = |a: &Rect, b: &Rect| a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom();
    let mut taken: Vec<Rect> = avoid.to_vec();
    let c = l.state.choice.clone();
    // the entry point under the mouse, else the one the bus stands at, in the markers' amber
    // (not for a stop chosen - the stop is marked, the foot names the entry point - nor for a
    // line, whose bus goes to the entry point nearest its first stop)
    let chosen = if free_route(&c).is_some() || !c.free_stop.is_empty() { -1 } else { c.free_entry };
    if let Some(i) = l.mapview.hovered().or_else(|| l.mapview.shown_of(chosen)) {
        if let (Some(name), Some(at)) = (l.mapview.entry_name(i).map(str::to_string), l.mapview.entry_at(i)) {
            if map.contains(at) {
                let name = if name.chars().count() > 32 { name.chars().take(31).collect::<String>() + "…" } else { name };
                let w = l.ui.width(&name, 11.5, Weight::Bold) + 14.0;
                let right = Rect::new(at.x + 12.0, at.y - 28.0, w, 20.0);
                let rr = if inside(&right) { right } else { Rect::new(at.x - 12.0 - w, at.y - 28.0, w, 20.0) };
                if inside(&rr) {
                    l.ui.p().rounded(rr, 4.0, accent());
                    l.ui.text_in(&name, rr.pad(7.0, 0.0), 11.5, Weight::Bold, on_accent(), Align::Left);
                    taken.push(rr);
                }
            }
        }
    }
    if let Some((line, route)) = free_route(&c) {
        let Some((_, trip)) = find_route(&l.state.lines, &line, &route) else { return };
        let trip = trip.clone();
        for (place, k) in l.mapview.placed_stops().iter().map(|s| (s.at, s.stop)).collect::<Vec<_>>() {
            let Some(st) = trip.stops.get(k) else { continue };
            let at = l.mapview.project(place);
            if !map.contains(at) {
                continue;
            }
            let first = k == 0;
            let name = if first { format!("{} · {}", omsi_ui::tr("Start"), st.name.trim()) } else { st.name.trim().to_string() };
            let px = if first { 12.0 } else { 11.0 };
            let w = l.ui.width(&name, px, Weight::Bold) + 20.0;
            let h = if first { 22.0 } else { 18.0 };
            let right = Rect::new(at.x + 9.0, at.y - h * 0.5, w, h);
            let left = Rect::new(at.x - 9.0 - w, at.y - h * 0.5, w, h);
            let Some(rr) = [right, left].into_iter().find(|r| inside(r) && (first || !taken.iter().any(|t| hits(t, r)))) else { continue };
            l.ui.p().rounded(rr, 4.0, if first { accent() } else { Color::rgba(10, 10, 10, 0.84) });
            l.ui.text_in(&name, rr.pad(8.0, 0.0), px, if first { Weight::Bold } else { Weight::Medium }, if first { on_accent() } else { TEXT_SOFT }, Align::Left);
            taken.push(rr);
        }
        return;
    }
    if c.own_line || c.free_stop.is_empty() {
        return;
    }
    let Some(place) = stop_place(l, &c.free_stop) else { return };
    let at = l.mapview.project(place);
    if !map.contains(at) {
        return;
    }
    // the stop as the game's maps mark the one the bus heads for (`stop_signs`, in the style
    // the settings choose)
    let style = crate::stop_signs::Style::from_setting(l.state.settings.get("stop_style").and_then(|v| v.as_str()).unwrap_or("de"));
    crate::stop_signs::draw(l.ui.p(), style, crate::stop_signs::Kind::Next, at, 16.0);
    let w = l.ui.width(&c.free_stop, 12.0, Weight::Bold) + 20.0;
    let right = Rect::new(at.x + 16.0, at.y - 11.0, w, 22.0);
    let left = Rect::new(at.x - 16.0 - w, at.y - 11.0, w, 22.0);
    if let Some(rr) = [right, left].into_iter().find(|r| inside(r)) {
        l.ui.p().rounded(rr, 4.0, Color::rgba(10, 10, 10, 0.88));
        l.ui.text_in(&c.free_stop, rr.pad(8.0, 0.0), 12.0, Weight::Bold, TEXT, Align::Left);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omsi_launcher_lib::{StopInfo, TourInfo};

    fn trip(name: &str, line: &str, stops: &[&str]) -> TripInfo {
        TripInfo {
            name: name.into(),
            index: 1,
            line: line.into(),
            from: stops[0].into(),
            terminus: stops[stops.len() - 1].into(),
            departure: 0.0,
            arrival: 600.0,
            stops: stops.iter().enumerate().map(|(k, s)| StopInfo { name: s.to_string(), id: k as i64 + s.len() as i64 * 100, arr: 0.0, dep: 0.0 }).collect(),
            km: 4.2,
        }
    }

    fn tour(runs: bool, trips: Vec<TripInfo>) -> TourInfo {
        TourInfo { number: "1".into(), ai_group: String::new(), first: 0.0, last: 0.0, days: String::new(), runs, next_run: None, trips }
    }

    fn line(name: &str, allowed: bool, tours: Vec<TourInfo>) -> LineInfo {
        LineInfo { name: name.into(), user_allowed: allowed, termini: vec!["Zoo".into()], tours }
    }

    /// Line 5 runs Hbf - Markt - Zoo and back twice a day, line 7 Hbf - Park once, and a
    /// depot run leaves the depot for the Hbf; on another day line 5 goes to the Park.
    fn day() -> Vec<LineInfo> {
        vec![
            line("5", true, vec![
                tour(true, vec![trip("5a", "5", &["Hbf", "Markt", "Zoo"]), trip("5b", "5", &["Zoo", "Markt", "Hbf"]), trip("5a", "5", &["Hbf", "Markt", "Zoo"])]),
                tour(true, vec![trip("depot", "", &["Depot", "Hbf"]), trip("5a", "5", &["Hbf", "Markt", "Zoo"])]),
                tour(false, vec![trip("5p", "5", &["Hbf", "Park"])]),
            ]),
            line("7", true, vec![tour(true, vec![trip("7a", "7", &["Hbf", "Park"])])]),
            line("99", false, vec![tour(true, vec![trip("99a", "99", &["Ring", "Markt"])])]),
        ]
    }

    #[test]
    fn stops_are_starting_points_or_intermediate_with_their_lines() {
        let (begin, via) = start_stops(&day());
        let names = |l: &[StartStop]| l.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        // the most departures first; the depot is a starting point of service trips only
        assert_eq!(names(&begin), ["Hbf", "Depot", "Zoo"]);
        assert_eq!((begin[0].begins, begin[0].passes, begin[0].lines.clone()), (4, 2, vec!["5".to_string(), "7".to_string()]));
        assert!(begin[1].lines.is_empty());
        // (line 99 is not one a player may drive, and the Park only on another day)
        assert_eq!(names(&via), ["Markt", "Park"]);
        assert_eq!((via[0].passes, via[0].lines.clone()), (4, vec!["5".to_string()]));
        assert_eq!(via[1].lines, ["7"]);
    }

    #[test]
    fn a_line_has_its_routes_the_most_driven_first() {
        let lines = day();
        let routes = routes_of(&lines[0]);
        let r: Vec<(&str, usize)> = routes.iter().map(|r| (r.trip.as_str(), r.runs)).collect();
        // the Park's only on its day, and the depot run last (a way the line's buses go, but
        // without a number on its displays)
        assert_eq!(r, [("5a", 3), ("5b", 1), ("5p", 0), ("depot", 1)]);
        assert_eq!((routes[0].from.as_str(), routes[0].terminus.as_str(), routes[0].stops), ("Hbf", "Zoo", 3));
        let rows = line_rows(&lines);
        assert_eq!(rows.iter().map(|r| (r.0.as_str(), r.1.clone())).collect::<Vec<_>>(), [("5", vec!["5".to_string()]), ("7", vec!["7".to_string()])]);
    }

    #[test]
    fn a_line_of_the_players_does_not_hide_the_maps_unmarked_lines() {
        // a map that marks no line: all of them, and the player's own (always marked) too
        let mut lines = day();
        for l in &mut lines {
            l.user_allowed = false;
        }
        lines.push(line("oo_42", true, vec![tour(true, vec![trip("oo_42_a", "42", &["Kirche", "Markt"])])]));
        let names = |v: Vec<&LineInfo>| v.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(drivable(&lines)), ["5", "7", "99", "oo_42"]);
        assert_eq!(line_rows(&lines).last().map(|r| r.1.clone()), Some(vec!["42".to_string()]));
        // a map that marks some: those, and the player's
        lines[1].user_allowed = true;
        assert_eq!(names(drivable(&lines)), ["7", "oo_42"]);
    }

    #[test]
    fn a_line_is_followed_only_in_a_free_drive_with_one_chosen() {
        let mut c = Choice { free: true, own_line: true, free_line: "5".into(), free_route: "5a".into(), ..Default::default() };
        assert_eq!(free_route(&c), Some(("5".into(), "5a".into())));
        c.own_line = false;
        assert_eq!(free_route(&c), None, "the switch is on Free");
        c.own_line = true;
        c.free = false;
        assert_eq!(free_route(&c), None, "a duty is no free drive");
        c.free = true;
        c.free_route.clear();
        assert_eq!(free_route(&c), None, "no route chosen yet");
    }

    #[test]
    fn a_free_drive_along_a_line_goes_to_the_game_with_its_route() {
        let lines = day();
        let (_, t) = find_route(&lines, "5", "5b").unwrap();
        assert_eq!((t.from.as_str(), t.terminus.as_str()), ("Zoo", "Hbf"));
        assert!(find_route(&lines, "7", "5a").is_none(), "a route is looked for in its own line");
    }

    #[test]
    fn distances_are_rounded_as_a_driver_says_them() {
        assert_eq!(distance(84.0), "80 m");
        assert_eq!(distance(1240.0), "1.2 km");
    }
}
