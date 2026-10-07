//! The fleet map: the company's map with every line it runs drawn in its colour, and every
//! bus of today's plan as a marker where it is at the clock (Luc's own wish, beside the
//! Busbetrieb-Simulator's features; Omsi-Hub's `Vlootkaart`).
//!
//! The map is the launcher's own (`mapview`, the roads the game's city map draws), asked for
//! the routes of every trip of the company's lines; the buses are placed by
//! `company::fleetmap` from the timetable at the clock, behind by the delay the game reported
//! for their tour (the live hook's file, while the game runs) or by the small one the model
//! shows. The clock follows the time of day - the game's, estimated from when it was started,
//! while it runs on the company's map - or runs by itself, ten or sixty times as fast. A bus
//! clicked shows its card: line, tour, duty, driver, delay, passengers and condition.

use super::super::mapview::{Dot, Look, Pointer};
use super::super::ownlines;
use super::super::theme::*;
use super::super::ui::ButtonKind;
use super::super::Launcher;
use super::kit;
use super::{data, grade, line_plate, meter, plate};
use glam::{DVec2, Vec2};
use omsi_launcher_lib as core;
use omsi_launcher_lib::company::fleetmap::{self as fm, State, TripShape};
use omsi_launcher_lib::company::{self as co, Company};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};
use std::collections::HashMap;
use std::time::Instant;

/// The colours of map lines without one of their own.
const PALETTE: [&str; 8] = ["#4f8ff7", "#34b26e", "#e8aa46", "#c867d8", "#4cc3c9", "#e8705f", "#a3c94c", "#7f8cf2"];

/// A tour of today with its trips as the map moves its bus.
struct TourShape {
    /// The company line (its timetable name), the number shown, the tour.
    line: String,
    number: String,
    tour: String,
    colour: Color,
    trips: Vec<TripShape>,
    km: Vec<f64>,
}

#[derive(Default)]
pub struct FleetMap {
    /// Seconds of the day the map shows, and how fast it runs (seconds of the day a second;
    /// 0 stopped). `live`: it follows the time of day.
    clock: f64,
    speed: f64,
    live: bool,
    started: bool,
    last: Option<Instant>,
    /// The tour picked: (line, tour).
    picked: Option<(String, String)>,
    tours: Vec<TourShape>,
    built_for: Option<(u64, String, String, usize)>,
    revision: u64,
    lines: Vec<(Vec<DVec2>, Color, f32)>,
    legend: Vec<(co::CompanyLine, Color)>,
    /// The fleet map's layer is on the map.
    on: bool,
    /// The delays the game reported (line number, tour) → seconds, and when they were read.
    delays: HashMap<(String, String), f64>,
    delays_at: Option<Instant>,
}

/// The fleet map's tab is left: the map is the duty's again.
pub fn leave(l: &mut Launcher) {
    if l.company.map.on {
        l.company.map.on = false;
        l.company.map.built_for = None;
        l.mapview.editor_off();
    }
}

fn colour_of_line(c: &Company, line: &str) -> Color {
    let k = c.lines.iter().position(|x| x.name.eq_ignore_ascii_case(line)).unwrap_or(0);
    match c.lines.get(k) {
        Some(x) if !x.colour.trim().is_empty() => ownlines::colour_of(&x.colour),
        _ => ownlines::colour_of(PALETTE[k % PALETTE.len()]),
    }
}

/// The time of day the map follows (seconds): the game's while it runs on the company's map
/// (from the time it was started at and how long it has run), else the company's clock.
pub(super) fn time_of_day(l: &Launcher, c: &Company) -> (f64, bool) {
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let same = |m: &str| m.replace('\\', "/").eq_ignore_ascii_case(&c.map.replace('\\', "/"));
    if let Some(i) = l.state.instances.iter().find(|i| i.running && same(&i.map)) {
        let start = l.state.choice.time as f64 * 60.0;
        return ((start + unix.saturating_sub(i.started) as f64).rem_euclid(86_400.0), true);
    }
    let now = omsi_launcher_lib::company::clock::now(c);
    (omsi_launcher_lib::company::clock::minute_of(now) as f64 * 60.0, false)
}

fn hhmmss(s: f64) -> String {
    let s = s.rem_euclid(86_400.0) as i64;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

fn hhmm(s: f64) -> String {
    let m = (s / 60.0).floor() as i64;
    format!("{:02}:{:02}", m.div_euclid(60).rem_euclid(24), m.rem_euclid(60))
}

/// "+2:10", "−0:45".
fn offset(s: f64) -> String {
    let a = s.abs().round() as i64;
    format!("{}{}:{:02}", if s < 0.0 { "−" } else { "+" }, a / 60, a % 60)
}

/// Build the tours' shapes from the map's routes (once per read of the map and the company's
/// day), and the lines as the map draws them.
fn build(l: &mut Launcher, c: &Company, lines: &[core::LineInfo]) {
    let key = (l.mapview.reads(), c.map.clone(), c.date.clone(), c.lines.len());
    if l.company.map.built_for.as_ref() == Some(&key) {
        return;
    }
    l.company.map.built_for = Some(key);
    let tracks: HashMap<String, (Vec<fm::P>, Vec<Option<fm::P>>)> = l
        .mapview
        .trip_tracks()
        .into_iter()
        .map(|(name, pts, stops)| (name, (pts.iter().map(|p| [p.x, p.y]).collect(), stops.iter().map(|s| s.map(|p| [p.x, p.y])).collect())))
        .collect();
    // (a trip's shape is the same on every tour that drives it: made once)
    let mut shapes: HashMap<String, TripShape> = HashMap::new();
    let mut tours = Vec::new();
    let mut drawn: Vec<(Vec<DVec2>, Color, f32)> = Vec::new();
    let mut legend = Vec::new();
    for cl in &c.lines {
        let Some(line) = lines.iter().find(|x| x.name.eq_ignore_ascii_case(&cl.name)) else { continue };
        let colour = colour_of_line(c, &cl.name);
        legend.push((cl.clone(), colour));
        let mut seen: Vec<String> = Vec::new();
        for t in line.tours.iter().filter(|t| t.runs) {
            let mut trips = Vec::new();
            let mut km = Vec::new();
            for trip in &t.trips {
                let base = shapes.entry(trip.name.clone()).or_insert_with(|| {
                    let (route, places) = tracks.get(&trip.name).cloned().unwrap_or_default();
                    let mut places = places;
                    places.resize(trip.stops.len(), None);
                    TripShape::new(trip, (route.len() >= 2).then_some(route.as_slice()), places)
                });
                // (this tour's times on the shape)
                let mut s = base.clone();
                s.dep = trip.departure;
                s.arr = trip.arrival.max(trip.departure);
                s.times = trip.stops.iter().enumerate().map(|(k, x)| if k == 0 { trip.departure.max(x.dep) } else { x.arr }).collect();
                s.empty = trip.stops.len() < 3;
                trips.push(s);
                km.push(trip.km);
                if !seen.contains(&trip.name) {
                    seen.push(trip.name.clone());
                    if let Some(tr) = &base.track {
                        drawn.push((tr.points.iter().map(|p| DVec2::new(p[0], p[1])).collect(), colour, 4.0));
                    }
                }
            }
            tours.push(TourShape { line: cl.name.clone(), number: cl.number.clone(), tour: t.number.clone(), colour, trips, km });
        }
    }
    let m = &mut l.company.map;
    m.tours = tours;
    m.lines = drawn;
    m.legend = legend;
    m.revision = m.revision.wrapping_add(1) | (1 << 40);
}

/// The delays the game reported for the company's tours (the live hook's file), read every
/// few seconds while the game runs.
fn live_delays(l: &mut Launcher, c: &Company, in_game: bool) {
    let m = &mut l.company.map;
    if !in_game {
        m.delays.clear();
        return;
    }
    if m.delays_at.is_some_and(|t| t.elapsed().as_secs_f32() < 5.0) {
        return;
    }
    m.delays_at = Some(Instant::now());
    let Ok(text) = std::fs::read_to_string(co::store::live_file(&data(), &c.id)) else { return };
    for ev in text.lines().filter_map(|x| serde_json::from_str::<co::day::LiveEvent>(x).ok()) {
        if let co::day::LiveEvent::Trip { line, tour, delay, .. } = ev {
            m.delays.insert((line.to_lowercase(), tour.trim().to_string()), delay);
        }
    }
}

/// A bus of the map this frame.
struct Marker {
    k: usize,
    place: fm::Place,
    delay: f64,
    at: DVec2,
}

pub fn draw(l: &mut Launcher, area: Rect) {
    let Some(c) = l.company.company.clone() else { return };
    let Some(today) = l.company.today.as_ref().filter(|t| t.map == c.map && t.date == c.date) else {
        l.ui.text_in("Reading the timetable…", Rect::new(area.x, area.y, area.w, 24.0), kit::ROWS, Weight::Regular, TEXT_DIM, Align::Left);
        return;
    };
    let lines = today.lines.clone();
    // the map: every trip of the company's lines today
    let mut trips: Vec<String> = Vec::new();
    for cl in &c.lines {
        if let Some(line) = lines.iter().find(|x| x.name.eq_ignore_ascii_case(&cl.name)) {
            for t in line.tours.iter().filter(|t| t.runs).flat_map(|t| t.trips.iter()) {
                if !trips.contains(&t.name) {
                    trips.push(t.name.clone());
                }
            }
        }
    }
    trips.sort();
    let global = omsi_cfg::find_in_roots(&c.map).map(|(_, p)| p).unwrap_or_else(|| omsi_cfg::resolve_path(std::path::Path::new(&l.state.config.root), &c.map));
    l.mapview.want(Look { map: c.map.clone(), global, date: c.date.clone(), trips, entry: -1 });
    l.map_background(area);
    l.ui.p().rounded_border(area, RADIUS, 1.0, EDGE);
    l.company.map.on = true;
    build(l, &c, &lines);
    let rev = l.company.map.revision;
    let drawn = l.company.map.lines.clone();
    l.mapview.company_lines(rev, || drawn);

    // the clock
    let (now, in_game) = time_of_day(l, &c);
    {
        let m = &mut l.company.map;
        if !m.started {
            m.started = true;
            m.live = true;
            m.clock = now;
        }
        let dt = m.last.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0).min(1.0);
        m.last = Some(Instant::now());
        if m.live {
            m.clock = now;
        } else if m.speed > 0.0 {
            m.clock = (m.clock + dt * m.speed).rem_euclid(86_400.0);
        }
    }
    l.ui.keep_moving();
    live_delays(l, &c, in_game);

    // the buses
    let plan = l.company.plan.clone();
    let clock = l.company.map.clock;
    let mut markers: Vec<Marker> = Vec::new();
    for (k, t) in l.company.map.tours.iter().enumerate() {
        let tp = plan.as_ref().and_then(|p| p.tours.iter().find(|x| x.tour.line.eq_ignore_ascii_case(&t.line) && x.tour.tour == t.tour));
        // (a tour without a bus is dropped today: it has no marker)
        if tp.is_some_and(|p| !p.covered()) {
            continue;
        }
        let refs: Vec<&TripShape> = t.trips.iter().collect();
        let first = fm::place_at(&refs, clock, 0.0);
        let delay = match l.company.map.delays.get(&(t.number.to_lowercase(), t.tour.trim().to_string())) {
            Some(d) => *d,
            None => {
                let bus = tp.and_then(|p| p.bus).and_then(|b| c.vehicle(b));
                let driver = tp.and_then(|p| p.duties.iter().find(|d| (d.start..d.end).contains(&first.trip)).and_then(|d| d.driver)).and_then(|id| c.employee(id));
                fm::delay_of(&c.date, &format!("{}/{}", t.line, t.tour), first.trip, driver.map(|e| e.experience).unwrap_or(60.0), bus.map(|v| v.condition).unwrap_or(90.0))
            }
        };
        let place = fm::place_at(&refs, clock, delay);
        // (in the depot until half an hour before it leaves, and after its day)
        let shown = match place.state {
            State::Depot => t.trips.first().is_some_and(|f| f.dep - clock <= 1800.0),
            State::Done => false,
            _ => true,
        };
        if let (true, Some(p)) = (shown, place.at) {
            markers.push(Marker { k, place, delay, at: DVec2::new(p[0], p[1]) });
        }
    }
    let picked = l.company.map.picked.clone();
    let mut dots: Vec<Dot> = Vec::with_capacity(markers.len());
    for m in &markers {
        let t = &l.company.map.tours[m.k];
        let on = picked.as_ref().is_some_and(|p| p.0 == t.line && p.1 == t.tour);
        let fill = if m.place.state == State::Trip { t.colour } else { t.colour.mix(TEXT_FAINT, 0.55) };
        dots.push(Dot { at: m.at, fill, r: if on { 7.5 } else { 5.5 }, ring: on.then_some(TEXT) });
    }
    l.mapview.editor_dots(dots);

    // the panel over the map's left side
    let panel = Rect::new(area.x + 12.0, area.y + 12.0, 360.0f32.min(area.w * 0.42), (area.h - 24.0).max(0.0));
    let window = Rect::new(panel.right(), area.y, (area.right() - panel.right()).max(1.0), area.h);
    side(l, panel, &c, &markers, in_game);

    // the bus under the mouse: its number beside it; a click picks it
    let mouse = l.ui.input.mouse;
    let over = area.contains(mouse) && !panel.contains(mouse) && l.mapview.network().is_some();
    let mut hover: Option<usize> = None;
    if over {
        hover = markers.iter().enumerate().map(|(i, m)| (i, (l.mapview.project(m.at) - mouse).length())).filter(|x| x.1 < 12.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0);
    }
    l.ui.push_clip(area, RADIUS);
    for (i, m) in markers.iter().enumerate() {
        let t = &l.company.map.tours[m.k];
        let on = picked.as_ref().is_some_and(|p| p.0 == t.line && p.1 == t.tour);
        if !(on || hover == Some(i)) {
            continue;
        }
        let p = l.mapview.project(m.at);
        let text = format!("{} / {}", t.number, t.tour);
        let w = l.ui.width(&text, kit::NOTE, Weight::Bold) + 16.0;
        let r = Rect::new(p.x + 12.0, p.y - 11.0, w, 22.0);
        l.ui.p().rounded(r, 6.0, ON_MAP);
        l.ui.text_in(&text, r, kit::NOTE, Weight::Bold, TEXT, Align::Center);
    }
    l.ui.pop_clip();
    if hover.is_some() {
        l.ui.cursor = winit::window::CursorIcon::Pointer;
    }
    // (the map lies on the page's sheet, which counts as interface everywhere: only the
    // panel over it blocks; a dialog over the page takes the mouse away itself)
    let p = Pointer { at: mouse, pressed: l.ui.input.pressed, released: l.ui.input.released, down: l.ui.input.down, wheel: l.ui.input.wheel.y, blocked: !area.contains(mouse) || panel.contains(mouse) };
    let scale = l.ui.scale;
    l.mapview.think(area, window, scale, p);
    if l.mapview.take_click_at().is_some() {
        l.company.map.picked = hover.map(|i| {
            let t = &l.company.map.tours[markers[i].k];
            (t.line.clone(), t.tour.clone())
        });
    }
}

/// The panel: the clock and its speed, the lines with their buses on the road, and the card
/// of the bus picked.
fn side(l: &mut Launcher, r: Rect, c: &Company, markers: &[Marker], in_game: bool) {
    l.ui.panel(r);
    let inner = Rect::new(r.x + 16.0, r.y + 14.0, r.w - 32.0, r.h - 28.0);
    let mut y = inner.y;
    l.ui.text_in(&omsi_ui::tr("Fleet map").to_uppercase(), Rect::new(inner.x, y, inner.w, 14.0), kit::CAPS, Weight::Bold, TEXT_DIM, Align::Left);
    if l.company.map.live {
        let t = if in_game { "Live: the game's time" } else { "Live: the company's time" };
        let w = l.ui.width(&omsi_ui::tr(t), 12.0, Weight::Bold) + 16.0;
        kit::tag(&mut l.ui, Vec2::new(inner.right() - w, y - 3.0), &omsi_ui::tr(t), if in_game { OK } else { accent_2() });
    }
    y += 24.0;
    let clock = l.company.map.clock;
    l.ui.text_in(&hhmmss(clock), Rect::new(inner.x, y, inner.w, 34.0), 28.0, Weight::Bold, TEXT, Align::Left);
    y += 40.0;
    // play, x10, x60 and back to now
    let bw = (inner.w - 3.0 * 6.0) / 4.0;
    let m = &l.company.map;
    let (live, speed) = (m.live, m.speed);
    let playing = !live && speed > 0.0;
    let b = |k: usize| Rect::new(inner.x + k as f32 * (bw + 6.0), y, bw, 36.0);
    if l.ui.button("fleetmap-play", b(0), "", Some(if playing || live { "pause" } else { "play_arrow" }), ButtonKind::Ghost) {
        let m = &mut l.company.map;
        if m.live || m.speed > 0.0 {
            m.live = false;
            m.speed = 0.0;
        } else {
            m.speed = 1.0;
        }
    }
    l.ui.tooltip(b(0), if playing || live { "Stop the clock" } else { "Run the clock" });
    for (k, s) in [(1usize, 10.0), (2, 60.0)] {
        let on = !live && (speed - s).abs() < 0.5;
        if l.ui.button(&format!("fleetmap-x{s}"), b(k), &format!("×{s:.0}"), None, if on { ButtonKind::Primary } else { ButtonKind::Ghost }) {
            let m = &mut l.company.map;
            m.live = false;
            m.speed = s;
        }
    }
    if l.ui.button("fleetmap-now", b(3), "", Some("my_location"), if live { ButtonKind::Primary } else { ButtonKind::Ghost }) {
        let m = &mut l.company.map;
        m.live = true;
        m.speed = 0.0;
    }
    l.ui.tooltip(b(3), "Back to now");
    l.ui.tooltip(b(1), "Ten times as fast");
    l.ui.tooltip(b(2), "Sixty times as fast");
    y += 46.0;
    // how many are where
    let on_road = markers.iter().filter(|m| matches!(m.place.state, State::Trip | State::Empty)).count();
    let pausing = markers.iter().filter(|m| m.place.state == State::Pause).count();
    let total = l.company.map.tours.len();
    let t = omsi_ui::tr("On the road: %{n}  ·  at a terminus: %{p}  ·  tours today: %{t}").replace("%{n}", &on_road.to_string()).replace("%{p}", &pausing.to_string()).replace("%{t}", &total.to_string());
    y += l.ui.paragraph(&t, Vec2::new(inner.x, y), inner.w, kit::NOTE, Weight::Regular, TEXT_SOFT) + 12.0;
    let picked = l.company.map.picked.clone();
    let card_h = 300.0;
    let legend_h = (inner.bottom() - y - if picked.is_some() { card_h + 12.0 } else { 0.0 }).max(0.0);
    // the lines
    let legend = l.company.map.legend.clone();
    let tours: Vec<(String, usize)> = markers.iter().map(|m| (l.company.map.tours[m.k].line.clone(), 1)).collect();
    if legend.is_empty() {
        l.ui.paragraph("The company runs no line on this day.", Vec2::new(inner.x, y), inner.w, 14.0, Weight::Regular, TEXT_SOFT);
    } else if legend_h > 30.0 {
        l.ui.scroll_area("fleetmap-legend", Rect::new(inner.x, y, inner.w, legend_h), &mut |ui, v| {
            let rh = 40.0;
            for (k, (line, colour)) in legend.iter().enumerate() {
                let r = Rect::new(v.x, v.y + k as f32 * rh, v.w - 8.0, rh - 4.0);
                if !ui.rect_visible(r) {
                    continue;
                }
                ui.p().rounded(Rect::new(r.x, r.y + 8.0, 4.0, 14.0), 2.0, *colour);
                let w = line_plate(ui, Vec2::new(r.x + 12.0, r.y + 5.0), line, 20.0);
                let caption = if line.caption.is_empty() { line.name.clone() } else { line.caption.clone() };
                ui.text_in(&caption, Rect::new(r.x + w + 20.0, r.y, r.w - w - 60.0, r.h), kit::NOTE, Weight::Medium, TEXT_SOFT, Align::Left);
                let n = tours.iter().filter(|t| t.0 == line.name).count();
                ui.text_in(&n.to_string(), Rect::new(r.right() - 34.0, r.y, 34.0, r.h), kit::NOTE, Weight::Bold, if n > 0 { TEXT } else { TEXT_FAINT }, Align::Right);
            }
            legend.len() as f32 * rh
        });
    }
    // the bus picked
    if let Some((line, tour)) = picked {
        let card = Rect::new(inner.x - 4.0, inner.bottom() - card_h, inner.w + 8.0, card_h);
        let m = markers.iter().find(|m| {
            let t = &l.company.map.tours[m.k];
            t.line == line && t.tour == tour
        });
        bus_card(l, card, c, &line, &tour, m);
    }
}

fn bus_card(l: &mut Launcher, r: Rect, c: &Company, line: &str, tour: &str, m: Option<&Marker>) {
    l.ui.p().rounded(r, RADIUS, FIELD);
    let inner = Rect::new(r.x + 12.0, r.y + 10.0, r.w - 24.0, r.h - 20.0);
    if l.ui.icon_button("fleetmap-card-close", Vec2::new(inner.right() - 8.0, inner.y + 10.0), 13.0, "close", "Close") {
        l.company.map.picked = None;
        return;
    }
    let Some(t) = l.company.map.tours.iter().find(|t| t.line == line && t.tour == tour) else { return };
    let (number, colour, trips, km) = (t.number.clone(), t.colour, t.trips.clone(), t.km.clone());
    let cl = c.lines.iter().find(|x| x.name == line).cloned();
    let w = match &cl {
        Some(x) => line_plate(&mut l.ui, Vec2::new(inner.x, inner.y), x, 22.0),
        None => plate(&mut l.ui, Vec2::new(inner.x, inner.y), &number, 22.0),
    };
    l.ui.p().rounded(Rect::new(inner.x + w + 8.0, inner.y + 4.0, 4.0, 14.0), 2.0, colour);
    l.ui.text_in(&format!("{} {}", omsi_ui::tr("Tour"), tour), Rect::new(inner.x + w + 18.0, inner.y, inner.w - w - 40.0, 22.0), 15.5, Weight::Bold, TEXT, Align::Left);
    let mut y = inner.y + 32.0;
    let tp = l.company.plan.as_ref().and_then(|p| p.tours.iter().find(|x| x.tour.line.eq_ignore_ascii_case(line) && x.tour.tour == tour)).cloned();
    let Some(m) = m else {
        let first = trips.first().map(|f| f.dep).unwrap_or(0.0);
        let t = if l.company.map.clock < first { omsi_ui::tr("In the depot: leaves at %{time}").replace("%{time}", &hhmm(first)) } else { omsi_ui::tr("Its day is done.").into_owned() };
        l.ui.paragraph(&t, Vec2::new(inner.x, y), inner.w, 14.0, Weight::Regular, TEXT_SOFT);
        return;
    };
    let trip = trips.get(m.place.trip);
    let next = trip.and_then(|t| m.place.next_stop.and_then(|k| t.names.get(k)));
    let what = match m.place.state {
        State::Trip => omsi_ui::tr("On its way to %{stop}").replace("%{stop}", &trip.and_then(|t| t.names.last()).cloned().unwrap_or_default()),
        State::Empty => omsi_ui::tr("Empty run").into_owned(),
        State::Pause => omsi_ui::tr("At the terminus until %{time}").replace("%{time}", &hhmm(m.place.next_time.unwrap_or(0.0))),
        State::Depot => omsi_ui::tr("Leaves the depot at %{time}").replace("%{time}", &hhmm(m.place.next_time.unwrap_or(0.0))),
        State::Done => omsi_ui::tr("Its day is done.").into_owned(),
    };
    l.ui.text_in(&what, Rect::new(inner.x, y, inner.w, 18.0), 14.0, Weight::Medium, TEXT_SOFT, Align::Left);
    y += 26.0;
    if let (Some(n), Some(tm)) = (next, m.place.next_time) {
        let t = omsi_ui::tr("Next: %{stop} at %{time}").replace("%{stop}", n).replace("%{time}", &hhmm(tm));
        l.ui.text_in(&t, Rect::new(inner.x, y, inner.w, 18.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
        y += 26.0;
    }
    let row = |l: &mut Launcher, y: f32, label: &str, value: &str, c: Color| {
        l.ui.text_in(label, Rect::new(inner.x, y, inner.w * 0.45, 22.0), kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Left);
        l.ui.text_in(value, Rect::new(inner.x + inner.w * 0.4, y, inner.w * 0.6, 22.0), kit::NOTE, Weight::Medium, c, Align::Right);
    };
    let bus = tp.as_ref().and_then(|p| p.bus).and_then(|b| c.vehicle(b));
    let duty = tp.as_ref().and_then(|p| p.duties.iter().position(|d| (d.start..d.end).contains(&m.place.trip)));
    let driver = tp.as_ref().and_then(|p| duty.and_then(|k| p.duties.get(k)).and_then(|d| d.driver)).and_then(|id| c.employee(id));
    let by_player = tp.as_ref().is_some_and(|p| p.by_player);
    row(l, y, &omsi_ui::tr("Bus"), &bus.map(|v| format!("{} {}", v.number, v.name)).unwrap_or_else(|| "—".into()), TEXT);
    y += 26.0;
    let duty_text = match (duty, tp.as_ref()) {
        (Some(k), Some(p)) => format!("{} / {}", k + 1, p.duties.len()),
        _ => "—".into(),
    };
    row(l, y, &omsi_ui::tr("Duty"), &duty_text, TEXT);
    y += 26.0;
    let driver_text = if by_player { omsi_ui::tr("You").into_owned() } else { driver.map(|e| e.name.clone()).unwrap_or_else(|| "—".into()) };
    row(l, y, &omsi_ui::tr("Driver"), &driver_text, TEXT);
    y += 26.0;
    let late = m.delay > 180.0;
    row(l, y, &omsi_ui::tr("Delay"), &offset(m.delay), if late { WARN } else if m.delay < -60.0 { EARLY_SOFT } else { OK });
    y += 26.0;
    let pax = match (trip, km.get(m.place.trip)) {
        (Some(t), Some(k)) if !t.empty => {
            let r = co::economy::rules(c.difficulty);
            format!("≈ {:.0}", co::economy::passengers_for(*k, (t.dep / 60.0) as i32, &r))
        }
        _ => "—".into(),
    };
    row(l, y, &omsi_ui::tr("Passengers this trip"), &pax, TEXT);
    y += 26.0;
    if let Some(v) = bus {
        row(l, y, &omsi_ui::tr("Condition"), &format!("{:.0} %", v.condition), grade(v.condition));
        meter(&mut l.ui, Rect::new(inner.x, y + 24.0, inner.w, 4.0), v.condition / 100.0, grade(v.condition));
    }
}
