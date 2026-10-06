//! The Drive page: three steps in a column on the left - the bus, the day and the weather,
//! the map and the duty - and a stage on the right: the bus on its turntable for the first
//! two, the map itself for the third (see `mapview`: it is dragged, zoomed and clicked, not
//! looked at), with the roadbook beside it. Under the stage a foot sums the choice up and
//! holds the buttons that go on.
//!
//! The order is what a player does: what to drive, then when, then where.

use super::state::{hhmm, trip_index_at};
use super::theme::*;
use super::ui::{id_of, ButtonKind};
use super::Launcher;
use glam::{DVec2, Vec2};
use omsi_launcher_lib::{display_bus_name, vehicle_type_label, WeatherInfo};
use omsi_ui::paint::Align;
use omsi_ui::{Rect, Weight};

#[derive(Clone)]
struct BusVariant {
    file: String,
    name: String,
    variant: String,
    fresh: bool,
    installed: bool,
    paints: usize,
    incomplete: bool,
}

#[derive(Clone)]
struct BusManufacturer {
    key: String,
    name: String,
    variants: Vec<BusVariant>,
}

#[derive(Default)]
pub struct DriveView {
    /// The tab: 0 the vehicle and the environment, 1 the map and the duty.
    pub tab: usize,
    pub bus_filter: String,
    /// The original OMSI manufacturer/type hierarchy, cached between content updates.
    bus_manufacturers: std::sync::Arc<Vec<BusManufacturer>>,
    bus_manufacturers_key: (usize, u64, usize),
    expanded_manufacturer: Option<String>,
    bus_list_initialized: bool,
    vehicle_settings_open: bool,
    pub line_filter: String,
    /// The tour list's search (tour number, trip name, line, stop) and whether tours whose
    /// last trip has left are listed.
    pub tour_filter: String,
    pub show_ended: bool,
    /// The line list was put where the chosen line is (once: the first time it has rows).
    line_scrolled: bool,
    /// The roadbook's sidebar: open or shut, and whether the player shut it by hand (which
    /// is what stops a chosen tour from opening it again).
    pub book_open: bool,
    book_shut: bool,
    /// The duty (line, tour, first trip) the roadbook was last scrolled to: it opens on the
    /// player's first trip, not on the morning's trips before it.
    book_scrolled: Option<(String, String, usize)>,
    /// The buses marked with a star (#524), by file (lower case, '/'), read once from
    /// `~/.openomsi/favourite-buses.txt`; and whether the list shows only them.
    favourites: Option<std::collections::BTreeSet<String>>,
    only_favourites: bool,
}

fn favourites_file() -> std::path::PathBuf {
    omsi_launcher_lib::data_dir().join("favourite-buses.txt")
}

fn fav_key(file: &str) -> String {
    file.replace('\\', "/").to_lowercase()
}

/// The starred buses of the file (one bus file a line).
fn read_favourites() -> std::collections::BTreeSet<String> {
    std::fs::read_to_string(favourites_file()).map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(fav_key).collect()).unwrap_or_default()
}

fn write_favourites(f: &std::collections::BTreeSet<String>) {
    let text: String = f.iter().map(|l| format!("{l}\n")).collect();
    if let Err(e) = std::fs::write(favourites_file(), text) {
        log::warn!("favourite buses not saved: {e}");
    }
}

/// The starred buses (by file, lower case, '/'), read once (the gallery shares them).
pub(super) fn favourites(d: &mut DriveView) -> std::collections::BTreeSet<String> {
    d.favourites.get_or_insert_with(read_favourites).clone()
}

/// A bus starred, or its star taken away again.
pub(super) fn toggle_favourite(d: &mut DriveView, file: &str) {
    let f = d.favourites.get_or_insert_with(read_favourites);
    let k = fav_key(file);
    if !f.remove(&k) {
        f.insert(k);
    }
    write_favourites(f);
}

/// The three steps, in the order a player decides: what to drive, the day it is driven on,
/// and where (the `drive:N` of `OMSI_LAUNCHER_PAGE` is the step's index).
const STEPS: [&str; 3] = ["Bus", "Day & weather", "Map & duty"];
/// Between the column and the stage, and between the stage's picture and its foot.
const GAP: f32 = 16.0;
/// The steps' row above the column.
const HEAD_H: f32 = 44.0;
/// The foot under the stage: what the choice comes to and the buttons that go on.
const FOOT_H: f32 = 76.0;
/// The foot's buttons.
const GO_H: f32 = 44.0;

/// The page's boxes, worked out once a frame: the column on the left (the steps above its
/// panel), the stage on the right (the bus or the map, its foot under it) and, on the map's
/// step, the roadbook beside the map. Nothing is drawn over anything else, so nothing shows
/// through a panel and no label runs under one.
struct Layout {
    steps: Rect,
    panel: Rect,
    /// The picture: the bus, or the map.
    view: Rect,
    /// The part of the picture nothing lies on (where the map frames the route).
    clear: Rect,
    book: Option<Rect>,
    foot: Rect,
}

fn layout(l: &Launcher, area: Rect) -> Layout {
    let col_w = (area.w * 0.32).clamp(340.0, 440.0);
    let steps = Rect::new(area.x, area.y, col_w, HEAD_H);
    let panel = Rect::new(area.x, area.y + HEAD_H + 12.0, col_w, (area.h - HEAD_H - 12.0).max(200.0));
    let stage = Rect::new(area.x + col_w + GAP, area.y, (area.w - col_w - GAP).max(260.0), area.h);
    let foot = Rect::new(stage.x, stage.bottom() - FOOT_H, stage.w, FOOT_H);
    let view = Rect::new(stage.x, stage.y, stage.w, (stage.h - FOOT_H - 12.0).max(140.0));
    if l.drive.tab != 2 || !l.drive.book_open {
        return Layout { steps, panel, view, clear: view, book: None, foot };
    }
    let book_w = (view.w * 0.36).clamp(290.0, 380.0);
    let book = Rect::new(view.right() - book_w, view.y, book_w, view.h);
    if view.w - book_w - 12.0 >= 360.0 {
        // room for both: the map beside the roadbook
        let map = Rect::new(view.x, view.y, view.w - book_w - 12.0, view.h);
        Layout { steps, panel, view: map, clear: map, book: Some(book), foot }
    } else {
        // a narrow window: the roadbook lies over the map's right side, the route is framed
        // in what it leaves
        let clear = Rect::new(view.x, view.y, (book.x - view.x - 8.0).max(120.0), view.h);
        Layout { steps, panel, view, clear, book: Some(book), foot }
    }
}

pub fn draw(l: &mut Launcher, area: Rect) {
    l.drive.tab = l.drive.tab.min(STEPS.len() - 1);
    let tab = l.drive.tab;
    // a duty chosen already (the last one, read back): its roadbook is open from the start,
    // unless the player put it away
    if tab == 2 && !l.drive.book_shut && !l.drive.book_open && l.state.tour().is_some() && !l.state.choice.free {
        l.drive.book_open = true;
        l.state.load_ibis();
    }
    let lay = layout(l, area);

    // the stage: the bus on its turntable, or the map (drawn first: what lies on it - the
    // panels, the roadbook button - takes the mouse before it)
    if tab == 2 {
        l.mapview.want(map_look(l));
        l.map_background(lay.view);
    } else {
        l.preview_full(lay.view, 0.5);
    }
    l.ui.p().rounded_border(lay.view, RADIUS, 1.0, EDGE());

    steps(l, lay.steps);
    l.ui.panel(lay.panel);
    let inner = Rect::new(lay.panel.x + 18.0, lay.panel.y + 16.0, lay.panel.w - 36.0, lay.panel.h - 32.0);
    match tab {
        0 => step_bus(l, inner),
        1 => step_time(l, inner),
        _ => duty_panel(l, inner),
    }

    if tab == 2 {
        let legend_r = legend(l, lay.clear);
        let handle = match lay.book {
            Some(b) => {
                book_panel(l, b);
                None
            }
            None => Some(book_handle(l, lay.clear)),
        };
        // (the names and times the map's pixels cannot say, kept off the legend and the
        // roadbook's button, and inside the map)
        let avoid: Vec<Rect> = [legend_r, handle].into_iter().flatten().collect();
        map_labels(l, lay.clear, &avoid);
    }
    foot(l, lay.foot, tab);
    if tab == 2 {
        l.map_interact(lay.view, lay.clear);
    } else {
        l.showroom_pointer(lay.view);
    }
}

/// The icon a weather file deserves: what it says about itself.
fn weather_icon_of(w: &WeatherInfo) -> &'static str {
    if w.snow || w.precip.starts_with("snow") {
        "weather_snowy"
    } else if w.precip.starts_with("rain") {
        "rainy"
    } else if w.fog_m < 1500.0 {
        "foggy"
    } else if w.clouds.to_lowercase().contains("overcast") {
        "cloud"
    } else if w.clouds.to_lowercase().contains("cumulus") {
        "partly_cloudy_day"
    } else {
        "wb_sunny"
    }
}

/// The three steps: a number and a name each, the current one underlined. Any step can be
/// gone to at any time - the order is only the one that reads best.
fn steps(l: &mut Launcher, r: Rect) {
    let w = r.w / STEPS.len() as f32;
    l.ui.p().rect(Rect::new(r.x, r.bottom() - 1.0, r.w, 1.0), EDGE());
    let sel = l.ui.anim(id_of("drive-tab"), l.drive.tab as f32, 0.08);
    l.ui.p().rect(Rect::new(r.x + w * sel, r.bottom() - 2.0, w, 2.0), ACCENT());
    for (k, name) in STEPS.iter().enumerate() {
        let cell = Rect::new(r.x + w * k as f32, r.y, w, r.h - 2.0);
        l.ui.solid(cell);
        let (h, _, clicked) = l.ui.interact(id_of(&format!("drive-tab-{k}")), cell);
        if clicked {
            l.drive.tab = k;
        }
        let on = l.drive.tab == k;
        let c = if on { TEXT() } else if h { TEXT_SOFT() } else { TEXT_DIM() };
        let dot = Vec2::new(cell.x + 14.0, cell.center().y);
        l.ui.p().circle(dot, 10.0, if on { ACCENT() } else { TRACK() });
        l.ui.text_in(&format!("{}", k + 1), Rect::new(dot.x - 10.0, dot.y - 10.0, 20.0, 20.0), 11.5, Weight::Bold, if on { ON_ACCENT() } else { TEXT_SOFT() }, Align::Center);
        l.ui.text_in(name, Rect::new(cell.x + 30.0, cell.y, (cell.w - 34.0).max(20.0), cell.h), 12.5, if on { Weight::Bold } else { Weight::Medium }, c, Align::Left);
    }
}

/// Where the bus is put down and what it drives there: the map, the duty, the place to start
/// (all of it also clickable on the map itself). `body` is the panel's inside.
fn duty_panel(l: &mut Launcher, body: Rect) {
    let mut y = body.y;
    // the map (on a server: the server's, not to be changed)
    if let Some(name) = joined_server_name(l) {
        let m = l.state.map().map(|m| if m.friendly.is_empty() { m.name.clone() } else { m.friendly.clone() }).unwrap_or_else(|| l.state.choice.map.clone());
        l.ui.label(Rect::new(body.x, y, 62.0, ROW), "Map");
        let fr = Rect::new(body.x + 62.0, y, body.w - 62.0, ROW);
        l.ui.solid(fr);
        l.ui.p().rounded(fr, RADIUS, FIELD());
        l.ui.icon("lock", Vec2::new(fr.x + 16.0, fr.center().y), 15.0, TEXT_DIM());
        l.ui.text_in(&format!("{m} · {name}"), Rect::new(fr.x + 32.0, fr.y, fr.w - 40.0, fr.h), 12.5, Weight::Regular, TEXT_SOFT(), Align::Left);
        y += ROW + 8.0;
        // (on a server: which one, and the way back to driving alone)
        let leave = Rect::new(body.x, y, body.w, ROW);
        if l.ui.button("leave-server", leave, "Leave the server", Some("logout"), ButtonKind::Normal) {
            l.state.leave_server();
        }
        l.ui.tooltip(leave, "Back to driving alone: the map, the clock and the weather are your own again");
        y += ROW + 12.0;
    } else {
        let maps: Vec<(String, String)> = l.state.maps.iter().map(|m| (m.file.clone(), format!("{}{}{}", if l.state.fresh.contains_key(&m.file) { "★ NEW · " } else { "" }, if m.friendly.is_empty() { &m.name } else { &m.friendly }, if m.installed { "  (mod)" } else { "" }))).collect();
        let mut sel = maps.iter().position(|m| m.0 == l.state.choice.map).unwrap_or(0);
        l.ui.label(Rect::new(body.x, y, 62.0, ROW), "Map");
        if l.ui.select("map", Rect::new(body.x + 62.0, y, body.w - 62.0, ROW), &mut sel, &maps.iter().map(|m| m.1.clone()).collect::<Vec<_>>()) {
            if let Some((f, _)) = maps.get(sel) {
                let f = f.clone();
                l.state.select_map(&f);
            }
        }
        y += ROW + 10.0;
    }
    let mut free = l.state.choice.free;
    if l.ui.toggle("free", Rect::new(body.x, y, body.w, ROW), &mut free, "Free drive (no timetable duty)") {
        l.state.choice.free = free;
        l.state.touched();
    }
    y += ROW + 10.0;
    // where to start, at the panel's foot: one of the map's entry points, as in OMSI 2;
    // Automatic takes the one nearest to the duty's first stop by road (a free drive: the
    // map's first)
    let start_y = body.bottom() - ROW;
    entry_select(l, body, start_y, free);
    if free {
        l.ui.paragraph("Free driving: the bus starts where you chose, with the traffic and the timetable's buses around it, but no line of your own. Pick the place below or click an entry point on the map.", Vec2::new(body.x, y), body.w, 12.5, Weight::Regular, TEXT_DIM());
        return;
    }
    // what the two lists share: the lines take about two fifths, the tours the rest
    let lists = (start_y - 30.0 - y - 26.0 - 40.0 - 8.0 - 26.0 - 40.0 - 8.0).max(200.0);
    let lines_h = (lists * 0.42).clamp(110.0, 300.0);
    // lines: a list that scrolls, with the search above it
    l.ui.heading(Rect::new(body.x, y, body.w, 24.0), "Line", None);
    y += 26.0;
    l.ui.text_input("line-filter", Rect::new(body.x, y, body.w, 34.0), &mut l.drive.line_filter, "Filter lines…", Some("search"));
    y += 40.0;
    let q = l.drive.line_filter.to_lowercase();
    let lines: Vec<(String, String, usize)> = l.state.lines.iter()
        .filter(|x| x.user_allowed)
        .filter(|x| q.is_empty() || x.name.to_lowercase().contains(&q))
        .map(|x| (x.name.clone(), x.termini.join(" · "), x.tours.len()))
        .collect();
    let chosen = l.state.choice.line.clone();
    let mut pick_line = None;
    let loading = l.state.loading_lines;
    const LINE_H: f32 = 50.0;
    // (the list opens on the line the duty already has, not on the alphabet)
    if !l.drive.line_scrolled && !lines.is_empty() {
        l.drive.line_scrolled = true;
        if let Some(k) = lines.iter().position(|x| chosen.as_deref() == Some(x.0.as_str())) {
            l.ui.scroll_to("line-list", k as f32 * LINE_H, LINE_H, lines_h);
        }
    }
    let list = Rect::new(body.x - 4.0, y, body.w + 8.0, lines_h);
    l.ui.scroll_area("line-list", list, &mut |ui, v| {
        if lines.is_empty() {
            ui.text_in(if loading { "Reading the timetable…" } else { "No lines on this date." }, Rect::new(v.x + 10.0, v.y + 6.0, v.w, 34.0), 12.5, Weight::Regular, TEXT_DIM(), Align::Left);
        }
        for (k, (name, termini, tours)) in lines.iter().enumerate() {
            let rr = Rect::new(v.x + 4.0, v.y + k as f32 * LINE_H, v.w - 12.0, LINE_H - 4.0);
            if !ui.rect_visible(rr) {
                continue;
            }
            let on = chosen.as_deref() == Some(name.as_str());
            if ui.row(&format!("line-{name}"), rr, on) {
                pick_line = Some(name.clone());
            }
            // (a long line name, as Ahlheim's "Eichenhoehe TA11 Mo-Do Schule", is cut)
            let count = format!("{tours}");
            let cw = ui.width(&count, 11.5, Weight::Bold) + 8.0;
            let bw = (ui.width(name, 12.5, Weight::Bold) + 14.0).clamp(34.0, (rr.w - cw - 40.0).max(34.0));
            let badge = Rect::new(rr.x + 10.0, rr.y + 6.0, bw, 20.0);
            ui.p().rounded(badge, 4.0, if on { ACCENT() } else { TRACK() });
            ui.text_in(name, badge.pad(6.0, 0.0), 12.0, Weight::Bold, if on { ON_ACCENT() } else { TEXT() }, Align::Center);
            ui.icon("event", Vec2::new(rr.right() - cw - 10.0, badge.center().y), 13.0, TEXT_FAINT());
            ui.text_in(&count, Rect::new(rr.right() - cw - 2.0, badge.y, cw, badge.h), 11.5, Weight::Bold, TEXT_DIM(), Align::Right);
            ui.tooltip(Rect::new(rr.right() - cw - 18.0, badge.y, cw + 18.0, badge.h), "Tours of this line on the chosen day");
            ui.text_in(termini, Rect::new(rr.x + 10.0, rr.y + 29.0, rr.w - 20.0, 14.0), 11.0, Weight::Regular, TEXT_DIM(), Align::Left);
        }
        lines.len() as f32 * LINE_H + 4.0
    });
    if let Some(n) = pick_line {
        if l.state.choice.line.as_deref() != Some(n.as_str()) {
            l.state.choice.line = Some(n);
            l.state.choice.tour = None;
            l.state.touched();
        }
    }
    y += lines_h + 12.0;
    // tours: the same, and choosing one brings the roadbook up with it
    let heading = Rect::new(body.x, y, body.w, 24.0);
    l.ui.heading(heading, "Tour", None);
    y += 28.0;
    let Some(line) = l.state.line().cloned() else {
        l.ui.paragraph("Pick a line first. As in OMSI, the start time and date then say where in the tour the bus is: the trip under way, or the next to leave.", Vec2::new(body.x, y), body.w, 12.5, Weight::Regular, TEXT_DIM());
        return;
    };
    l.ui.text_input("tour-filter", Rect::new(body.x, y, body.w, 34.0), &mut l.drive.tour_filter, "Filter tours: number, route, stop…", Some("search"));
    y += 40.0;
    let now = l.state.choice.time as f64 * 60.0;
    let q = l.drive.tour_filter.trim().to_lowercase();
    let mut ended_count = 0;
    let mut tours: Vec<(&omsi_launcher_lib::TourInfo, bool, Option<usize>)> = Vec::new();
    for t in &line.tours {
        let (ended, trip) = tour_trip(t, now, &q);
        if trip.is_none() && !t.trips.is_empty() {
            continue;
        }
        if ended {
            ended_count += 1;
            if !l.drive.show_ended {
                continue;
            }
        }
        tours.push((t, ended, trip));
    }
    let departure = |t: &(&omsi_launcher_lib::TourInfo, bool, Option<usize>)| t.2.and_then(|k| t.0.trips.get(k)).map_or(f64::MAX, |x| x.departure);
    tours.sort_by(|a, b| {
        b.0.runs.cmp(&a.0.runs).then(a.1.cmp(&b.1)).then_with(|| {
            if q.is_empty() {
                std::cmp::Ordering::Equal
            } else {
                departure(a).total_cmp(&departure(b))
            }
        }).then_with(|| natural(&a.0.number).cmp(&natural(&b.0.number)))
    });
    let mut show_ended = l.drive.show_ended;
    let toggle_w = 190.0_f32.min(body.w * 0.6);
    if l.ui.toggle("tour-ended", Rect::new(heading.right() - toggle_w, heading.y, toggle_w, heading.h), &mut show_ended, &format!("{} ({ended_count})", omsi_ui::tr("Ended tours"))) {
        l.drive.show_ended = show_ended;
    }
    l.ui.tooltip(Rect::new(heading.right() - toggle_w, heading.y, toggle_w, heading.h), "Tours whose last trip has already left at the chosen time");
    let searching = !q.is_empty();
    let tours: Vec<(String, usize, String, bool, Option<String>, Option<omsi_launcher_lib::TripInfo>, String, String, bool)> = tours.iter().map(|&(t, ended, k)| {
        let trip = k.and_then(|i| t.trips.get(i)).cloned();
        let from = t.trips.first().map(|x| x.from.clone()).unwrap_or_default();
        let terminus = t.trips.last().map(|x| x.terminus.clone()).unwrap_or_default();
        (t.number.clone(), t.trips.len(), t.days.clone(), t.runs, t.next_run.clone(), trip, from, terminus, ended)
    }).collect();
    let chosen_t = l.state.choice.tour.clone();
    let mut pick = None;
    const TOUR_H: f32 = 70.0;
    let list = Rect::new(body.x - 4.0, y, body.w + 8.0, (start_y - 30.0 - y).max(TOUR_H));
    l.ui.scroll_area("tour-list", list, &mut |ui, v| {
        if tours.is_empty() {
            let why = if searching { "No tour has a trip still to come that matches." } else { "No tour left at this time: show the ended tours, or start earlier." };
            ui.text_in(why, Rect::new(v.x + 10.0, v.y + 6.0, v.w, 34.0), 12.5, Weight::Regular, TEXT_DIM(), Align::Left);
        }
        for (k, (num, trips, days, runs, next, trip, from, terminus, ended)) in tours.iter().enumerate() {
            let rr = Rect::new(v.x + 4.0, v.y + k as f32 * TOUR_H, v.w - 12.0, TOUR_H - 4.0);
            if !ui.rect_visible(rr) {
                continue;
            }
            let on = chosen_t.as_deref() == Some(num.as_str());
            if ui.row(&format!("tour-{num}"), rr, on) {
                pick = Some((num.clone(), *runs, next.clone(), trip.clone()));
            }
            let live = *runs && !*ended;
            let c = if live { TEXT() } else { TEXT_FAINT() };
            // (the tour's name as the map writes it and OMSI lists it: "1", "Mo-Fr 1"),
            // the trip a start now would take on the right
            ui.text_in(num, Rect::new(rr.x + 10.0, rr.y + 6.0, rr.w - 120.0, 18.0), 13.5, Weight::Bold, c, Align::Left);
            if let Some(x) = trip {
                ui.text_in(&format!("{} - {}", hhmm(x.departure), hhmm(x.arrival)), Rect::new(rr.right() - 112.0, rr.y + 6.0, 102.0, 18.0), 12.0, Weight::Medium, if live { ACCENT() } else { TEXT_FAINT() }, Align::Right);
            }
            let route = match trip {
                Some(x) if searching => format!("{} · {} → {}", x.name, x.from, x.terminus),
                _ if from.is_empty() && terminus.is_empty() => String::new(),
                _ => format!("{from} → {terminus}"),
            };
            ui.text_in(&route, Rect::new(rr.x + 10.0, rr.y + 26.0, rr.w - 20.0, 16.0), 11.5, Weight::Medium, if live { TEXT_SOFT() } else { TEXT_FAINT() }, Align::Left);
            let mut sub = format!("{trips} trips · {days}");
            if let Some(x) = trip {
                sub = format!("{sub} · {} {}", trip_duration(x.departure, x.arrival), omsi_ui::tr("a trip"));
            }
            if *ended {
                sub = format!("{trips} trips · {days} · ended");
            }
            if !*runs {
                sub = match next {
                    Some(n) => format!("{trips} trips · {days} · runs {n}"),
                    None => format!("{trips} trips · never within a year"),
                };
            }
            ui.text_in(&sub, Rect::new(rr.x + 10.0, rr.y + 45.0, rr.w - 20.0, 16.0), 11.0, Weight::Regular, TEXT_DIM(), Align::Left);
        }
        tours.len() as f32 * TOUR_H + 4.0
    });
    if let Some((num, runs, next, trip)) = pick {
        // a tour of another day moves the date to the next day it runs (OMSI lists only
        // the day's tours)
        if !runs {
            if let Some(n) = next {
                l.state.choice.date = n;
                l.state.load_lines();
            }
        }
        l.state.choice.tour = Some(num);
        l.state.touched();
        // a search starts the tour at the trip it found
        if let Some(x) = trip.filter(|_| searching) {
            l.state.pick_trip(x.index, x.departure);
        }
        // the roadbook has something to say now
        l.state.load_ibis();
        if !l.drive.book_shut {
            l.drive.book_open = true;
        }
    }
}

/// Whether all of a tour's trips have left at `now`, and the trip a start then takes: the one
/// under way or the next, or with a search (`q`, lower case) the first still to come that
/// matches it by name, line or stop (a tour whose number matches keeps the usual one).
fn tour_trip(t: &omsi_launcher_lib::TourInfo, now: f64, q: &str) -> (bool, Option<usize>) {
    let ended = t.runs && t.trips.iter().all(|x| x.departure < now - 120.0);
    let first = trip_index_at(t, now);
    let has = |s: &str| s.to_lowercase().contains(q);
    if q.is_empty() || has(&t.number) {
        return (ended, first);
    }
    let from = if ended { 0 } else { first.unwrap_or(0) };
    let hit = t.trips.iter().enumerate().skip(from).find(|(_, x)| {
        has(&x.name) || has(&x.line) || has(&x.from) || has(&x.terminus) || x.stops.iter().any(|s| has(&s.name))
    });
    (ended, hit.map(|(k, _)| k))
}

/// The entry point the bus starts at, at `y` in the panel; the one under the mouse on the map
/// named above it.
fn entry_select(l: &mut Launcher, body: Rect, y: f32, free: bool) {
    let Some(m) = l.state.map().cloned() else { return };
    let mut labels = vec![if free { "Automatic (the map's first)".to_string() } else { "Automatic (nearest to the first stop)".to_string() }];
    labels.extend(m.entry_points.iter().map(|e| if e.name.is_empty() { format!("entry {}", e.index + 1) } else { e.name.clone() }));
    // (the choice is the entry's place in the list; 0 = automatic here)
    let mut es = if l.state.choice.entry < 0 { 0 } else { (l.state.choice.entry as usize + 1).min(labels.len() - 1) };
    l.ui.p().rect(Rect::new(body.x, y - 14.0, body.w, 1.0), EDGE());
    l.ui.label(Rect::new(body.x, y, 70.0, ROW), "Start at");
    if l.ui.select("entry", Rect::new(body.x + 70.0, y, body.w - 70.0, ROW), &mut es, &labels) {
        l.state.choice.entry = es as i32 - 1;
        l.state.touched();
    }
    l.ui.tooltip(Rect::new(body.x, y, 70.0, ROW), "Where the bus is put down. The orange marks on the map are the same places: click one to take it.");
}

/// The roadbook beside the map: the chosen tour's trips and their stops, and the IBIS.
fn book_panel(l: &mut Launcher, r: Rect) {
    l.ui.panel(r);
    l.ui.heading(Rect::new(r.x + 16.0, r.y + 8.0, r.w - 76.0, 30.0), "Roadbook", Some("receipt_long"));
    let shut = Rect::new(r.right() - 44.0, r.y + 10.0, 30.0, 30.0);
    if l.ui.button("book-shut", shut, "", Some("close"), ButtonKind::Ghost) {
        l.drive.book_open = false;
        l.drive.book_shut = true;
    }
    l.ui.tooltip(shut, "Put the roadbook away (the button on the map brings it back)");
    step_roadbook(l, Rect::new(r.x + 16.0, r.y + 46.0, r.w - 32.0, r.h - 60.0));
}

/// The roadbook put away: a button in the map's top right corner. Returns where it is.
fn book_handle(l: &mut Launcher, map: Rect) -> Rect {
    let r = Rect::new(map.right() - 138.0, map.y + 12.0, 126.0, 34.0);
    l.ui.solid(r);
    if l.ui.button("book-open", r, "Roadbook", Some("receipt_long"), ButtonKind::Normal) {
        l.drive.book_open = true;
        l.drive.book_shut = false;
    }
    l.ui.tooltip(r, "The trips of the chosen tour, where they call, and what to type into the IBIS");
    r
}

/// The foot under the stage: what the choice comes to on the left, the buttons on the right -
/// on to the next step, or into the game on the last one, and back into the last game.
fn foot(l: &mut Launcher, f: Rect, tab: usize) {
    l.ui.panel(f);
    let running = l.state.instances.iter().filter(|i| i.running).count();
    let pad = 16.0;
    let by = f.y + (f.h - GO_H) * 0.5;
    // the buttons, from the right
    let go_w = 212.0;
    let go = Rect::new(f.right() - pad - go_w, by, go_w, GO_H);
    let mut left_of = go.x;
    if tab < 2 {
        let next = STEPS[tab + 1];
        if l.ui.button("drive-next", go, &format!("{} {}", omsi_ui::tr("Next:"), omsi_ui::tr(next)), Some("arrow_forward"), ButtonKind::Primary) {
            l.drive.tab = tab + 1;
        }
    } else {
        let label = if running > 0 && l.state.second_armed.map(|t| t.elapsed().as_secs() < 6).unwrap_or(false) {
            "Start another game"
        } else if (l.state.choice.free || l.state.choice.line.is_none()) && l.state.joined_server.is_none() {
            "Drive"
        } else {
            "Start the duty"
        };
        if l.ui.button("launch", go, label, Some("play_arrow"), ButtonKind::Primary) {
            start(l);
        }
    }
    // where the last game on this map was left (`laststn.osn`): a second way in on every
    // step, and - when the map keeps more than one (save slots, #341) - which of them
    if l.state.joined_server.is_none() && l.state.has_last_situation() {
        let saves: Vec<String> = l.state.saved_situations().iter().map(|s| s.name.clone()).collect();
        let narrow = f.w < 760.0;
        if saves.len() > 1 {
            let bw = if narrow { GO_H } else { 128.0 };
            let b = Rect::new(left_of - 10.0 - bw, by, bw, GO_H);
            let s = Rect::new(b.x - 8.0 - 150.0, by, 150.0, GO_H);
            let mut pick = l.state.save_pick.min(saves.len() - 1);
            if l.ui.select("continue-which", s, &mut pick, &saves) {
                l.state.save_pick = pick;
            }
            if l.ui.button("continue", b, if narrow { "" } else { "Continue" }, Some("history"), ButtonKind::Normal) {
                l.state.launch_last_situation();
            }
            l.ui.tooltip(b, "Continue where you left off");
            left_of = s.x;
        } else {
            let bw = if narrow { GO_H } else { 196.0 };
            let b = Rect::new(left_of - 10.0 - bw, by, bw, GO_H);
            if l.ui.button("continue", b, if narrow { "" } else { "Continue last game" }, Some("history"), ButtonKind::Normal) {
                l.state.launch_last_situation();
            }
            l.ui.tooltip(b, "Continue where you left off on this map");
            left_of = b.x;
        }
    }
    // the choice in words, left of the buttons
    let text_w = (left_of - f.x - pad * 2.0).max(0.0);
    if text_w < 60.0 {
        return;
    }
    let icon = ["directions_bus", "partly_cloudy_day", "map"][tab];
    l.ui.icon(icon, Vec2::new(f.x + pad + 12.0, f.center().y), 22.0, TEXT_DIM());
    let tx = f.x + pad + 34.0;
    let tw = (text_w - 34.0).max(0.0);
    let (title, sub) = if tab < 2 {
        let bus = l.state.bus().map(|b| display_bus_name(&b.name)).unwrap_or_else(|| "No bus chosen".into());
        let paint = paint_line(l);
        (if paint.is_empty() { bus } else { format!("{bus} · {paint}") }, start_line(l))
    } else {
        let (duty, when) = duty_of(l);
        let place = duty_place(l);
        (duty, if when.is_empty() { place } else { format!("{when} · {place}") })
    };
    let warn = l.state.bus().filter(|b| !b.missing_packs.is_empty()).map(|b| omsi_ui::tr("Parts missing: needs %{packs}").replace("%{packs}", &b.missing_packs.join(", ")));
    let lines = 2 + (warn.is_some() || running > 0) as usize;
    let y0 = f.center().y - lines as f32 * 9.5;
    l.ui.text_in(&title, Rect::new(tx, y0, tw, 20.0), 13.5, Weight::Bold, TEXT(), Align::Left);
    l.ui.text_in(&sub, Rect::new(tx, y0 + 20.0, tw, 18.0), 11.5, Weight::Regular, TEXT_DIM(), Align::Left);
    if let Some(w) = warn {
        l.ui.text_in(&w, Rect::new(tx, y0 + 38.0, tw, 18.0), 11.5, Weight::Medium, WARN(), Align::Left);
    } else if running > 0 {
        let note = format!("{running} game{} running - see Sessions", if running > 1 { "s" } else { "" });
        let nr = Rect::new(tx, y0 + 38.0, l.ui.width(&note, 11.5, Weight::Medium).min(tw), 18.0);
        let (h, _, clicked) = l.ui.interact(id_of("running-note"), nr);
        if clicked {
            l.go(super::Page::Sessions);
        }
        l.ui.text_in(&note, nr, 11.5, Weight::Medium, if h { TEXT() } else { OK() }, Align::Left);
    }
}

/// The livery the bus would wear.
fn paint_line(l: &Launcher) -> String {
    match l.state.bus() {
        Some(bus) if l.state.choice.paint.is_empty() => default_livery_label(bus).to_string(),
        Some(_) => l.state.choice.paint.clone(),
        None => String::new(),
    }
}

/// The chosen day and weather in one line.
fn start_line(l: &Launcher) -> String {
    // on a server: its clock and its weather, whatever this machine has chosen
    if let Some(i) = l.state.joined_server.as_ref().and_then(|a| l.state.server_info.get(a)).and_then(|x| x.1.as_ref().ok()) {
        let weather = if i.weather.is_empty() {
            omsi_ui::tr("the map's (the server's)").into_owned()
        } else {
            format!("{} {}", i.weather, omsi_ui::tr("(the server's)"))
        };
        return format!("{} {} · {weather}", i.time, omsi_ui::tr("(the server's clock)"));
    }
    let weather = match l.state.choice.weather.strip_prefix("metar:") {
        Some(code) => format!("at {code}"),
        None if l.state.choice.weather == "cycle" => omsi_ui::tr("Weather cycle").into_owned(),
        None if crate::weather_model::is_natural(Some(&l.state.choice.weather)) || l.state.choice.weather.is_empty() => omsi_ui::tr("Natural weather").into_owned(),
        None if crate::weather_setup::custom_weather(Some(&l.state.choice.weather)).is_some() => {
            let c = crate::weather_setup::custom_weather(Some(&l.state.choice.weather)).unwrap();
            format!("{} · {}", omsi_ui::tr("Custom"), custom_weather_summary(&c))
        }
        None => l.state.weathers.iter().find(|w| w.file == l.state.choice.weather).map(|w| w.name.clone()).unwrap_or_else(|| "the map's weather".into()),
    };
    let (yy, mm, dd) = super::ui::parse_date(&l.state.choice.date);
    format!("{:02}:{:02}, {dd} {} {yy} · {weather}", l.state.choice.time / 60, l.state.choice.time % 60, super::ui::MONTHS[(mm as usize).clamp(1, 12) - 1])
}

/// The duty in words: the line and the tour, and when the trip the game would take runs.
fn duty_of(l: &Launcher) -> (String, String) {
    let map = l.state.map().map(|m| if m.friendly.is_empty() { m.name.clone() } else { m.friendly.clone() }).unwrap_or_else(|| "-".into());
    match (&l.state.choice.line, &l.state.choice.tour, l.state.choice.free) {
        (_, _, true) | (None, _, _) => (format!("Free drive · {map}"), String::new()),
        (Some(line), Some(t), _) => {
            let trip = l.state.tour().and_then(|t| t.trips.get(l.state.first_trip().unwrap_or(0)));
            let when = trip.map(|x| format!("{} - {} · {:.1} km · {} → {}", hhmm(x.departure), hhmm(x.arrival), x.km, if x.from.is_empty() { "?" } else { &x.from }, x.terminus)).unwrap_or_default();
            (format!("Line {line} · tour {t} · {map}"), when)
        }
        (Some(line), None, _) => (format!("Line {line} · choose a tour · {map}"), String::new()),
    }
}

/// Where the bus is put down.
fn duty_place(l: &Launcher) -> String {
    match l.state.choice.entry {
        e if e < 0 => "starting point: automatic".to_string(),
        e => l
            .state
            .map()
            .and_then(|m| m.entry_points.get(e as usize))
            .map(|x| format!("starting at {}", if x.name.is_empty() { format!("entry {}", x.index + 1) } else { x.name.clone() }))
            .unwrap_or_else(|| "starting point: automatic".into()),
    }
}

/// How much map the picture holds, in the map's bottom left corner. Returns where it is.
fn legend(l: &mut Launcher, map: Rect) -> Option<Rect> {
    let (roads, stops, entries) = l.mapview.counts()?;
    let t = format!("{roads} {}  ·  {stops} {}  ·  {entries} {}", omsi_ui::tr("roads"), omsi_ui::tr("stops"), omsi_ui::tr("entry points"));
    let w = (l.ui.width(&t, 11.0, Weight::Regular) + 20.0).min(map.w - 24.0);
    let bar = Rect::new(map.x + 12.0, map.bottom() - 32.0, w, 22.0);
    l.ui.solid(bar);
    l.ui.p().rounded(bar, 5.0, SHADE().alpha(0.85));
    l.ui.text_in(&t, bar.pad(10.0, 0.0), 11.0, Weight::Regular, TEXT_SOFT(), Align::Left);
    Some(bar)
}

/// The names and times the map's own pixels cannot say: where the chosen trip calls, and
/// which entry point the mouse is on. Each name goes right of its point, or left of it at the
/// map's right edge; one that would sit on another name or on `avoid` is left out, and none
/// leaves the map.
fn map_labels(l: &mut Launcher, map: Rect, avoid: &[Rect]) {
    let hits = |a: &Rect, b: &Rect| a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom();
    let inside = |r: &Rect| r.x >= map.x + 4.0 && r.right() <= map.right() - 4.0 && r.y >= map.y + 4.0 && r.bottom() <= map.bottom() - 4.0;
    let mut taken: Vec<Rect> = avoid.to_vec();
    // the entry point first (it is what the mouse is on), then the stops in their order
    if let Some(i) = l.mapview.hovered().or_else(|| l.mapview.shown_of(l.state.choice.entry)) {
        if let (Some(name), Some(at)) = (l.mapview.entry_name(i).map(str::to_string), l.mapview.entry_at(i)) {
            if map.contains(at) {
                let name = if name.chars().count() > 32 { name.chars().take(31).collect::<String>() + "…" } else { name };
                let w = l.ui.width(&name, 11.5, Weight::Medium) + 14.0;
                let right = Rect::new(at.x + 12.0, at.y - 28.0, w, 20.0);
                let rr = if inside(&right) { right } else { Rect::new(at.x - 12.0 - w, at.y - 28.0, w, 20.0) };
                if inside(&rr) {
                    l.ui.p().rounded(rr, 4.0, ACCENT());
                    l.ui.text_in(&name, rr.pad(7.0, 0.0), 11.5, Weight::Bold, ON_ACCENT(), Align::Left);
                    taken.push(rr);
                }
            }
        }
    }
    let stops: Vec<(DVec2, usize)> = l.mapview.stops_placed().to_vec();
    if stops.is_empty() {
        return;
    }
    let trip = l.state.tour().and_then(|t| t.trips.get(l.state.first_trip().unwrap_or(0))).cloned();
    for (place, k) in stops {
        let Some(st) = trip.as_ref().and_then(|t| t.stops.get(k)) else { continue };
        let at = l.mapview.project(place);
        if !map.contains(at) {
            continue;
        }
        let time = hhmm(st.arr);
        let w = l.ui.width(&st.name, 11.0, Weight::Medium) + l.ui.width(&time, 11.0, Weight::Bold) + 22.0;
        let right = Rect::new(at.x + 9.0, at.y - 9.0, w, 18.0);
        let left = Rect::new(at.x - 9.0 - w, at.y - 9.0, w, 18.0);
        let Some(rr) = [right, left].into_iter().find(|r| inside(r) && !taken.iter().any(|t| hits(t, r))) else { continue };
        l.ui.p().rounded(rr, 4.0, RAIL().alpha(0.88));
        let tw = l.ui.width(&time, 11.0, Weight::Bold);
        l.ui.text_in(&time, Rect::new(rr.x + 6.0, rr.y, tw + 2.0, rr.h), 11.0, Weight::Bold, ACCENT(), Align::Left);
        l.ui.text_in(&st.name, Rect::new(rr.x + 12.0 + tw, rr.y, rr.w - tw - 16.0, rr.h), 11.0, Weight::Medium, TEXT_SOFT(), Align::Left);
        taken.push(rr);
    }
}

/// OMSI takes the manufacturer and the complete type from [friendlyname]. The
/// vehicle folder and rendering configuration do not define this hierarchy.
fn build_bus_manufacturers(vehicles: &[omsi_launcher_lib::VehicleInfo], allowed: Option<&std::collections::HashSet<String>>, fresh: &std::collections::HashSet<String>) -> Vec<BusManufacturer> {
    let mut grouped = std::collections::BTreeMap::<String, BusManufacturer>::new();
    for vehicle in vehicles {
        if !allowed.map(|a| a.contains(&vehicle.file.replace('\\', "/").to_lowercase())).unwrap_or(true) {
            continue;
        }
        let maker = vehicle.manufacturer.trim();
        let key = maker.to_lowercase();
        let group = grouped.entry(key.clone()).or_insert_with(|| BusManufacturer {
            key, name: if maker.is_empty() { "Unknown manufacturer".into() } else { display_bus_name(maker) }, variants: Vec::new(),
        });
        let type_name = vehicle_type_label(&vehicle.type_name, std::path::Path::new(&vehicle.file));
        group.variants.push(BusVariant {
            file: vehicle.file.clone(), name: display_bus_name(&vehicle.name), variant: type_name,
            fresh: fresh.contains(&vehicle.file), installed: vehicle.installed,
            paints: vehicle.paints.len(), incomplete: !vehicle.missing_packs.is_empty(),
        });
    }
    let mut manufacturers: Vec<BusManufacturer> = grouped.into_values().collect();
    for maker in &mut manufacturers {
        // Distinct .bus files remain selectable even when add-ons repeat a friendly
        // type name. Show the pack, and the file only if the pack also repeats it.
        let mut counts = std::collections::HashMap::<String, usize>::new();
        for variant in &maker.variants { *counts.entry(variant.variant.to_lowercase()).or_default() += 1; }
        for variant in &mut maker.variants {
            if counts[&variant.variant.to_lowercase()] > 1 {
                let folder = variant.file.replace('\\', "/").split('/').nth(1).unwrap_or_default().to_string();
                variant.variant = format!("{} · {}", variant.variant, display_bus_name(&folder));
            }
        }
        let mut counts = std::collections::HashMap::<String, usize>::new();
        for variant in &maker.variants { *counts.entry(variant.variant.to_lowercase()).or_default() += 1; }
        for variant in &mut maker.variants {
            if counts[&variant.variant.to_lowercase()] > 1 {
                let stem = std::path::Path::new(&variant.file).file_stem().unwrap_or_default().to_string_lossy();
                variant.variant = format!("{} · {}", variant.variant, display_bus_name(&stem));
            }
        }
        maker.variants.sort_by(|a, b| bus_name_cmp(&a.variant, &b.variant).then_with(|| a.file.cmp(&b.file)));
    }
    manufacturers.sort_by(|a, b| bus_name_cmp(&a.name, &b.name).then_with(|| a.key.cmp(&b.key)));
    manufacturers
}

/// Sort numeric runs wherever they occur: DL9 precedes DL10; case does not change
/// a manufacturer's position.
fn bus_name_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let a = a.to_lowercase();
    let b = b.to_lowercase();
    let mut a = a.chars().peekable();
    let mut b = b.chars().peekable();
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let x: String = std::iter::from_fn(|| a.next_if(|c| c.is_ascii_digit())).collect();
                let y: String = std::iter::from_fn(|| b.next_if(|c| c.is_ascii_digit())).collect();
                let x = x.trim_start_matches('0');
                let y = y.trim_start_matches('0');
                let order = x.len().cmp(&y.len()).then_with(|| x.cmp(y));
                if order != Ordering::Equal { return order; }
            }
            (Some(x), Some(y)) => {
                let order = x.cmp(&y);
                if order != Ordering::Equal { return order; }
                a.next(); b.next();
            }
        }
    }
}

pub(super) fn default_livery_label(vehicle: &omsi_launcher_lib::VehicleInfo) -> &str {
    if vehicle.default_paint.trim().is_empty() { "Default paint" } else { vehicle.default_paint.trim() }
}

/// How many liveries a bus type with `repaints` has, as its Livery list counts them (its own
/// paint and the repaints): beside each type in the list, as a family says how many types it
/// has (#717).
pub(super) fn liveries_text(repaints: usize) -> String {
    let n = repaints + 1;
    format!("{n} {}", omsi_ui::tr(if n == 1 { "livery" } else { "liveries" }))
}

/// A type of a bus family in its dropdown: the type and its liveries.
fn variant_option(variant: &BusVariant) -> String {
    format!("{} · {}", variant.variant, liveries_text(variant.paints))
}

fn variant_matches(variant: &BusVariant, q: &str) -> bool {
    q.is_empty() || variant.name.to_lowercase().contains(q) || variant.variant.to_lowercase().contains(q) || display_bus_name(&variant.file).to_lowercase().contains(q)
}

fn manufacturer_matches(model: &BusManufacturer, q: &str) -> bool {
    q.is_empty() || model.name.to_lowercase().contains(q) || model.variants.iter().any(|v| variant_matches(v, q))
}

fn step_bus(l: &mut Launcher, r: Rect) {
    l.ui.heading(Rect::new(r.x, r.y, r.w, 24.0), "Choose a bus", None);
    let search = Rect::new(r.x, r.y + 32.0, r.w, ROW);
    let search_changed = l.ui.text_input("bus-filter", search, &mut l.drive.bus_filter, "Search buses…", Some("search"));
    if search_changed {
        l.ui.scroll.remove(&id_of("bus-model-list"));
        l.ui.scroll.remove(&(id_of("bus-model-list") ^ 0xabc));
    }
    let q = display_bus_name(l.drive.bus_filter.trim()).to_lowercase();
    let norm = |f: &str| f.replace('\\', "/").to_lowercase();
    let allowed: Option<std::collections::HashSet<String>> = l.state.host_vehicles().map(|v| v.iter().map(|f| norm(f)).collect());
    if let Some(a) = allowed.as_ref() {
        if !a.contains(&norm(&l.state.choice.bus)) {
            if let Some(first) = l.state.vehicles.iter().find(|v| a.contains(&norm(&v.file))).map(|v| v.file.clone()) {
                l.state.select_bus(&first);
            }
        }
    }
    let allowed_key = allowed.as_ref().map(|a| a.iter().fold(0u64, |hash, file| hash ^ id_of(file))).unwrap_or(u64::MAX);
    let key = (l.state.vehicles.len(), allowed_key, l.state.fresh.len());
    if key != l.drive.bus_manufacturers_key || (l.drive.bus_manufacturers.is_empty() && !l.state.vehicles.is_empty()) {
        let fresh = l.state.fresh.keys().cloned().collect();
        l.drive.bus_manufacturers = std::sync::Arc::new(build_bus_manufacturers(&l.state.vehicles, allowed.as_ref(), &fresh));
        l.drive.bus_manufacturers_key = key;
    }
    let models = l.drive.bus_manufacturers.clone();
    let chosen = l.state.choice.bus.clone();
    let favs = l.drive.favourites.get_or_insert_with(read_favourites).clone();
    let is_fav = |file: &str| favs.contains(&fav_key(file));
    // (the starred ones only: a family with one of them, and in it only those - #524)
    let only = l.drive.only_favourites && !favs.is_empty();

    let visible: Vec<&BusManufacturer> = models.iter().filter(|m| manufacturer_matches(m, &q) && (!only || m.variants.iter().any(|v| is_fav(&v.file)))).collect();
    let count = format!("{} {}", visible.len(), omsi_ui::tr(if visible.len() == 1 { "manufacturer" } else { "manufacturers" }));
    l.ui.text_in(&count, Rect::new(r.x, search.bottom() + 6.0, r.w, 20.0), 11.5, Weight::Regular, TEXT_DIM(), Align::Left);
    let mut only_now = l.drive.only_favourites;
    let fav_w = l.ui.width(&omsi_ui::tr("Favourites only"), 13.0, Weight::Regular) + 50.0;
    if l.ui.toggle("bus-only-favourites", Rect::new(r.right() - fav_w, search.bottom() + 4.0, fav_w, 22.0), &mut only_now, "Favourites only") {
        l.drive.only_favourites = only_now;
        l.ui.scroll.remove(&id_of("bus-model-list"));
    }
    let list_y = search.bottom() + 32.0;
    let settings_h = if l.drive.vehicle_settings_open { 164.0 } else { 0.0 };
    let list = Rect::new(r.x, list_y, r.w, (r.bottom() - list_y - 132.0 - settings_h).max(100.0));
    l.ui.p().rounded(list, RADIUS, FIELD());
    l.ui.p().rounded_border(list, RADIUS, 1.0, EDGE());
    if !l.drive.bus_list_initialized && !models.is_empty() {
        if let Some(index) = models.iter().position(|m| m.variants.iter().any(|v| v.file == chosen)) {
            let model = &models[index];
            l.drive.expanded_manufacturer = Some(model.key.clone());
            let selected_y = 6.0 + index as f32 * 58.0;
            l.ui.scroll_to("bus-model-list", selected_y, if model.variants.len() > 1 { 120.0 } else { 54.0 }, list.h);
        }
        l.drive.bus_list_initialized = true;
    }
    if search_changed && !q.is_empty() {
        l.drive.expanded_manufacturer = visible.first().map(|m| m.key.clone());
    }
    let expanded = l.drive.expanded_manufacturer.clone();
    let mut toggle = None;
    let mut pick = None;
    let mut star: Option<String> = None;
    let loading = l.state.loading_content;
    l.ui.scroll_area("bus-model-list", list, &mut |ui, view| {
        let mut y = view.y + 6.0;
        if visible.is_empty() {
            ui.text_in(if loading { "Reading the buses…" } else { "No buses found. Try another search." }, Rect::new(view.x + 12.0, y, view.w - 24.0, 50.0), 12.5, Weight::Regular, TEXT_DIM(), Align::Left);
        }
        for model in &visible {
            let open = model.variants.len() > 1 && expanded.as_deref() == Some(model.key.as_str());
            let row = Rect::new(view.x + 6.0, y, view.w - 18.0, 54.0);
            // Keep the scroll extent, but avoid shaping text and emitting geometry for
            // invisible families. The expanded dropdown retains its widget state.
            if !open && !ui.rect_visible(row) {
                y += 58.0;
                continue;
            }
            let selected = model.variants.iter().find(|v| v.file == chosen);
            if ui.row(&format!("bus-family-{}", model.key), row, selected.is_some()) {
                if model.variants.len() == 1 { pick = Some(model.variants[0].file.clone()); }
                else { toggle = Some(model.key.clone()); }
            }
            ui.icon("directions_bus", Vec2::new(row.x + 20.0, row.y + 23.0), 20.0, if selected.is_some() { ACCENT() } else { TEXT_DIM() });
            let title = Rect::new(row.x + 42.0, row.y + 7.0, row.w - 78.0, 20.0);
            ui.text_in(&model.name, title, 13.0, Weight::Medium, TEXT(), Align::Left);
            ui.tooltip(title, &model.name);
            let subtitle = if model.variants.len() == 1 { format!("{} · {}", omsi_ui::tr(&model.variants[0].variant), liveries_text(model.variants[0].paints)) }
            else if let Some(v) = selected { format!("{} · {} {}", omsi_ui::tr(&v.variant), model.variants.len(), omsi_ui::tr("models")) }
            else { format!("{} {}", model.variants.len(), omsi_ui::tr("models")) };
            let subtitle = if selected.is_some_and(|v| v.incomplete) { format!("{subtitle} · {}", omsi_ui::tr("PARTS MISSING")) }
            else if selected.is_some_and(|v| v.fresh) { format!("{subtitle} · {}", omsi_ui::tr("NEW")) }
            else if selected.is_some_and(|v| v.installed) { format!("{subtitle} · {}", omsi_ui::tr("MOD")) } else { subtitle };
            ui.text_in(&subtitle, Rect::new(title.x, row.y + 29.0, title.w, 17.0), 11.5, Weight::Regular, TEXT_DIM(), Align::Left);
            ui.icon(if model.variants.len() == 1 { if selected.is_some() { "check" } else { "chevron_right" } } else if open { "expand_less" } else { "expand_more" }, Vec2::new(row.right() - 18.0, row.center().y), 18.0, if selected.is_some() { ACCENT() } else { TEXT_DIM() });
            // the star: a bus of its own is starred here, a family's types in its list below
            let starred = model.variants.iter().any(|v| is_fav(&v.file));
            let sr = Rect::new(row.right() - 62.0, row.center().y - 13.0, 26.0, 26.0);
            if model.variants.len() == 1 {
                let (hs, _, cs) = ui.interact(id_of(&format!("bus-star-{}", model.key)), sr);
                if cs {
                    star = Some(model.variants[0].file.clone());
                }
                ui.icon("star", sr.center(), 16.0, if starred { ACCENT() } else if hs { TEXT_SOFT() } else { LIFT().alpha(0.16) });
                ui.tooltip(sr, if starred { "Remove from the favourites" } else { "Add to the favourites" });
            } else if starred {
                ui.icon("star", sr.center(), 14.0, ACCENT().alpha(0.8));
            }
            y += 58.0;
            if open {
                let variants: Vec<&BusVariant> = model.variants.iter().filter(|variant| (q.is_empty() || model.name.to_lowercase().contains(&q) || variant.file == chosen || variant_matches(variant, &q)) && (!only || variant.file == chosen || is_fav(&variant.file))).collect();
                let selected_index = variants.iter().position(|variant| variant.file == chosen);
                let mut options: Vec<String> = variants.iter().map(|variant| variant_option(variant)).collect();
                let offset = if selected_index.is_none() { options.insert(0, "Choose a bus".into()); 1 } else { 0 };
                let mut sel = selected_index.unwrap_or(0);
                ui.label(Rect::new(view.x + 38.0, y, view.w - 50.0, 22.0), "Type / variant");
                y += 26.0;
                let dropdown = Rect::new(view.x + 38.0, y, view.w - 50.0 - 36.0, ROW);
                if !options.is_empty() && ui.select(&format!("bus-type-{}", model.key), dropdown, &mut sel, &options) {
                    if let Some(variant) = sel.checked_sub(offset).and_then(|index| variants.get(index)) { pick = Some(variant.file.clone()); }
                }
                // the star of the type chosen
                if let Some(variant) = selected {
                    let sr = Rect::new(dropdown.right() + 6.0, y, 30.0, ROW);
                    let on = is_fav(&variant.file);
                    let (hs, _, cs) = ui.interact(id_of(&format!("bus-star-type-{}", model.key)), sr);
                    if cs {
                        star = Some(variant.file.clone());
                    }
                    ui.icon("star", sr.center(), 18.0, if on { ACCENT() } else if hs { TEXT_SOFT() } else { LIFT().alpha(0.2) });
                    ui.tooltip(sr, if on { "Remove from the favourites" } else { "Add to the favourites" });
                }
                if let Some(variant) = selected {
                    ui.tooltip(dropdown, &format!("{}\n{}\n{}", variant.name, variant.file, liveries_text(variant.paints)));
                }
                y += ROW + 10.0;
            }
        }
        y - view.y + 4.0
    });
    if let Some(key) = toggle {
        l.drive.expanded_manufacturer = if l.drive.expanded_manufacturer.as_ref() == Some(&key) { None } else { Some(key.clone()) };
        if l.drive.expanded_manufacturer.is_some() {
            if let Some(index) = visible.iter().position(|maker| maker.key == key) {
                l.ui.scroll_to("bus-model-list", 6.0 + index as f32 * 58.0, 120.0, list.h);
            }
        }
    }
    if let Some(file) = pick { l.state.select_bus(&file); }
    if let Some(file) = star {
        let f = l.drive.favourites.get_or_insert_with(read_favourites);
        let k = fav_key(&file);
        if !f.remove(&k) {
            f.insert(k);
        }
        write_favourites(f);
    }

    let mut y = list.bottom() + 16.0;
    if let Some(vehicle) = l.state.bus().cloned() {
        let paints: Vec<String> = std::iter::once(default_livery_label(&vehicle).to_string()).chain(vehicle.paints.iter().cloned()).collect();
        let mut paint_sel = vehicle.paints.iter().position(|p| *p == l.state.choice.paint).map(|i| i + 1).unwrap_or(0);
        l.ui.label(Rect::new(r.x, y, r.w, 22.0), "Livery");
        if paints.len() > 1 { l.ui.text_in(&format!("{} / {}", paint_sel + 1, paints.len()), Rect::new(r.right() - 70.0, y, 70.0, 22.0), 11.5, Weight::Regular, TEXT_DIM(), Align::Right); }
        y += 28.0;
        let has_arrows = paints.len() > 1;
        let selector = Rect::new(r.x, y, r.w - if has_arrows { 88.0 } else { 0.0 }, ROW);
        let mut changed = l.ui.select("paint", selector, &mut paint_sel, &paints);
        if has_arrows {
            let prev = Rect::new(selector.right() + 8.0, y, ROW, ROW);
            let next = Rect::new(prev.right() + 8.0, y, ROW, ROW);
            if l.ui.button("paint-previous", prev, "", Some("chevron_left"), ButtonKind::Normal) { paint_sel = (paint_sel + paints.len() - 1) % paints.len(); changed = true; }
            if l.ui.button("paint-next", next, "", Some("chevron_right"), ButtonKind::Normal) { paint_sel = (paint_sel + 1) % paints.len(); changed = true; }
            l.ui.tooltip(prev, "Preview previous livery");
            l.ui.tooltip(next, "Preview next livery");
        }
        if changed {
            l.state.choice.paint = if paint_sel == 0 { String::new() } else { vehicle.paints[paint_sel - 1].clone() };
            l.state.touched();
        }
        y += ROW + 14.0;
        let settings = Rect::new(r.x, y, r.w, 32.0);
        if l.ui.row("vehicle-settings-toggle", settings, false) { l.drive.vehicle_settings_open = !l.drive.vehicle_settings_open; }
        l.ui.icon(if l.drive.vehicle_settings_open { "expand_less" } else { "expand_more" }, Vec2::new(settings.x + 12.0, settings.center().y), 18.0, TEXT_DIM());
        l.ui.text_in("Vehicle settings & details", Rect::new(settings.x + 30.0, settings.y, settings.w - 30.0, settings.h), 12.5, Weight::Medium, TEXT_DIM(), Align::Left);
        y += 38.0;
        if l.drive.vehicle_settings_open {
            let details = Rect::new(r.x, y, r.w, (r.bottom() - y).max(0.0));
            let mut hof_pick = None;
            let mut number_pick = None;
            let number_options: Vec<String> = vehicle.numbers.iter().map(|(number, plate)| if plate.trim().is_empty() { number.clone() } else { format!("{number}  ({})", plate.trim()) }).collect();
            let mut number_sel = vehicle.numbers.iter().position(|(number, _)| *number == l.state.choice.number).unwrap_or(0);
            let mut plate = l.state.choice.plate.clone();
            let auto = l.state.default_hof();
            let mut hof_options = vec![format!("Automatic ({auto})")];
            hof_options.extend(vehicle.hofs.iter().cloned());
            let mut hof_sel = if l.state.choice.hof_manual { vehicle.hofs.iter().position(|h| h.eq_ignore_ascii_case(&l.state.choice.hof)).map(|i| i + 1).unwrap_or(0) } else { 0 };
            let mut plate_changed = false;
            l.ui.scroll_area("bus-details", details, &mut |ui, view| {
                let mut y = view.y;
                let field_w = view.w - 8.0;
                ui.label(Rect::new(view.x, y, 110.0, ROW), "Depot file");
                if ui.select("hof", Rect::new(view.x + 110.0, y, field_w - 110.0, ROW), &mut hof_sel, &hof_options) { hof_pick = Some(hof_sel); }
                y += ROW + 8.0;
                if !number_options.is_empty() {
                    ui.label(Rect::new(view.x, y, 110.0, ROW), "Fleet number");
                    if ui.select("number", Rect::new(view.x + 110.0, y, field_w - 110.0, ROW), &mut number_sel, &number_options) { number_pick = Some(number_sel); }
                    y += ROW + 8.0;
                }
                ui.label(Rect::new(view.x, y, 110.0, ROW), "Number plate");
                plate_changed = ui.text_input("plate", Rect::new(view.x + 110.0, y, field_w - 110.0, ROW), &mut plate, "Automatic", Some("badge"));
                y += ROW + 16.0;
                if !vehicle.missing_packs.is_empty() {
                    y += ui.paragraph(&omsi_ui::tr("Parts missing: needs %{packs}").replace("%{packs}", &vehicle.missing_packs.join(", ")), Vec2::new(view.x, y), field_w, 12.5, Weight::Regular, WARN()) + 12.0;
                }
                let description = vehicle.description.replace('\t', " ").lines().map(str::trim).collect::<Vec<_>>().join("\n").trim().to_string();
                if !description.is_empty() { y += ui.paragraph(&description, Vec2::new(view.x, y), field_w, 12.0, Weight::Regular, TEXT_DIM()) + 12.0; }
                y += ui.paragraph(&vehicle.file, Vec2::new(view.x, y), field_w, 10.5, Weight::Regular, TEXT_FAINT());
                y - view.y + 8.0
            });
            if let Some(sel) = hof_pick {
                l.state.choice.hof_manual = sel != 0;
                l.state.choice.hof = if sel == 0 { auto } else { vehicle.hofs[sel - 1].clone() };
                l.state.touched();
            }
            if let Some(sel) = number_pick {
                l.state.choice.number = vehicle.numbers[sel].0.clone();
                l.state.touched();
            }
            if plate_changed { l.state.choice.plate = plate; l.state.touched(); }
        }
    }
}

pub(super) fn natural(s: &str) -> (u64, String) {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    (digits.parse().unwrap_or(u64::MAX), s.to_string())
}

/// Time from the departure to the arrival of one trip.
fn trip_duration(first: f64, last: f64) -> String {
    let minutes = ((last - first).max(0.0) / 60.0).round() as i64;
    let hours = minutes / 60;
    let remaining = minutes % 60;
    let count = |n: i64, one: &str, many: &str| format!("{n} {}", omsi_ui::tr(if n == 1 { one } else { many }));
    match (hours, remaining) {
        (0, m) => count(m, "minute", "minutes"),
        (h, 0) => count(h, "hour", "hours"),
        (h, m) => format!("{} {}", count(h, "hour", "hours"), count(m, "minute", "minutes")),
    }
}

/// The name of the server the Drive page is joined to (see the Multiplayer page).
fn joined_server_name(l: &Launcher) -> Option<String> {
    let a = l.state.joined_server.as_ref()?;
    let entry = l.state.servers.iter().find(|s| &s.address == a);
    let info = l.state.server_info.get(a).and_then(|x| x.1.as_ref().ok());
    Some(entry.map(|e| e.name.clone()).filter(|n| !n.is_empty()).or_else(|| info.map(|i| i.name.clone())).unwrap_or_else(|| a.clone()))
}

fn step_time(l: &mut Launcher, r: Rect) {
    let mut y = r.y;
    if let Some(name) = joined_server_name(l) {
        // the server's world: nothing to choose here
        let info = l.state.joined_server.as_ref().and_then(|a| l.state.server_info.get(a)).and_then(|x| x.1.as_ref().ok()).cloned();
        l.ui.heading(Rect::new(r.x, y, r.w, 28.0), &format!("Set by {name}"), Some("lock"));
        y += 36.0;
        let rows = [("Time", info.as_ref().map(|i| i.time.clone()).unwrap_or_default()), ("Weather", info.as_ref().map(|i| if i.weather.is_empty() { "the map's".to_string() } else { i.weather.clone() }).unwrap_or_default())];
        for (k, v) in rows {
            l.ui.text_in(k, Rect::new(r.x, y, 110.0, 22.0), 13.0, Weight::Regular, TEXT_DIM(), Align::Left);
            l.ui.text_in(&v, Rect::new(r.x + 110.0, y, r.w - 110.0, 22.0), 13.0, Weight::Regular, TEXT(), Align::Left);
            y += 26.0;
        }
        l.ui.paragraph("On a server the map, the time, the date and the weather are the same for everybody: the server keeps the world's clock. You choose your bus and your duty.", Vec2::new(r.x, y + 8.0), r.w, 12.5, Weight::Regular, TEXT_DIM());
        return;
    }
    let col = (r.w - 12.0) * 0.5;
    l.ui.label(Rect::new(r.x, y, col, 20.0), "Time");
    l.ui.label(Rect::new(r.x + col + 12.0, y, col, 20.0), "Date");
    y += 22.0;
    let mut t = l.state.choice.time;
    if l.ui.time_field("time", Rect::new(r.x, y, col, 44.0), &mut t) {
        l.state.choice.time = t;
        l.state.touched();
    }
    let mut d = l.state.choice.date.clone();
    if l.ui.date_field("date", Rect::new(r.x + col + 12.0, y, col, 44.0), &mut d) {
        l.state.choice.date = d;
        l.state.choice.season = "auto".into();
        l.state.load_lines();
        l.state.touched();
    }
    y += 54.0;
    // the computer's clock in one click - not while the launcher follows it already
    // (Settings: start at the real time / on today's date), where a button would do nothing
    let follows = |k: &str| l.state.settings.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
    let (own_time, own_date) = (!follows("use_real_time"), !follows("use_real_date"));
    if own_time && l.ui.button("current-time", Rect::new(r.x, y, col, 34.0), "Current time", None, ButtonKind::Normal) {
        if let Some((_, _, _, h, m)) = omsi_launcher_lib::local_now() {
            l.state.choice.time = h * 60 + m;
            l.state.touched();
        }
    }
    if own_date && l.ui.button("current-date", Rect::new(r.x + col + 12.0, y, col, 34.0), "Current date", None, ButtonKind::Normal) {
        if let Some((yy, mo, d, _, _)) = omsi_launcher_lib::local_now() {
            l.state.choice.date = format!("{yy:04}-{mo:02}-{d:02}");
            l.state.choice.season = "auto".into();
            l.state.load_lines();
            l.state.touched();
        }
    }
    if own_time || own_date {
        y += 44.0;
    }
    let seasons = ["auto", "spring", "summer", "autumn", "winter"];
    let mut s = seasons.iter().position(|x| *x == l.state.choice.season).unwrap_or(0);
    if l.ui.segmented("season", Rect::new(r.x, y, r.w, 34.0), &mut s, &["By date", "Spring", "Summer", "Autumn", "Winter"]) {
        l.state.choice.season = seasons[s].to_string();
        if s > 0 {
            let month = ["", "04", "07", "10", "01"][s];
            let date = l.state.choice.date.clone();
            let (yy, dd) = (date.get(0..4).unwrap_or("1989").to_string(), date.get(8..10).unwrap_or("15").to_string());
            l.state.choice.date = format!("{yy}-{month}-{dd}");
            l.state.load_lines();
        }
        let w = l.state.choice.weather.clone();
        if let Some(wi) = l.state.weathers.iter().find(|x| x.file == w).cloned() {
            if !l.state.weather_fits(&wi) {
                l.state.choice.weather.clear();
            }
        }
        l.state.touched();
    }
    y += 46.0;
    let mut traffic = l.state.choice.traffic;
    if l.ui.slider("traffic", Rect::new(r.x, y, r.w, 34.0), &mut traffic, 0.0, 120.0, 1.0, "Cars around", &|v| format!("{v:.0}")) {
        l.state.choice.traffic = traffic;
        l.state.touched();
    }
    y += 40.0;
    let mut v = l.state.choice.passengers;
    if l.ui.toggle("pax", Rect::new(r.x, y, col, 32.0), &mut v, "Passengers") {
        l.state.choice.passengers = v;
        l.state.touched();
    }
    let mut v = l.state.choice.schedule;
    if l.ui.toggle("sched", Rect::new(r.x + col + 12.0, y, col, 32.0), &mut v, "Timetable buses") {
        l.state.choice.schedule = v;
        l.state.touched();
    }
    y += 36.0;
    let mut v = l.state.choice.autostart;
    if l.ui.toggle("autostart", Rect::new(r.x, y, r.w, 32.0), &mut v, "Put the bus into service on start (Shift+U)") {
        l.state.choice.autostart = v;
        l.state.touched();
    }
    y += 36.0;
    let mut v = l.state.choice.on_foot;
    if l.ui.toggle("onfoot", Rect::new(r.x, y, r.w, 32.0), &mut v, "Start on foot (place a bus from the game menu)") {
        l.state.choice.on_foot = v;
        l.state.touched();
    }
    y += 44.0;
    // weather cards
    l.ui.heading(Rect::new(r.x, y, r.w, 28.0), "Weather", Some("partly_cloudy_day"));
    y += 30.0;
    if selected_weather_as_custom(&l.state.config.root,&l.state.choice.weather).is_some() {
        if l.ui.button("weather-edit-current",Rect::new(r.x,y,r.w,34.0),"Edit selected weather as custom",Some("tune"),ButtonKind::Normal){
            if let Some(custom)=selected_weather_as_custom(&l.state.config.root,&l.state.choice.weather){
                l.state.choice.weather=custom;
                l.state.touched();
            }
        }
        y+=42.0;
    }
    let custom_now = crate::weather_setup::custom_weather(Some(&l.state.choice.weather));
    let custom_file = custom_now.clone().unwrap_or_default().encode();
    let custom_meta = custom_now.as_ref()
        .map(custom_weather_summary)
        .unwrap_or_else(|| "Set visibility, wind, clouds, rain, temperature and road state".into());
    let mut items: Vec<(String, String, String, String, bool)> = vec![
        // (no weather chosen: the physical model, weather_model.rs)
        (String::new(), "Natural weather".into(), "Develops by itself through the day and the season".into(), "wb_sunny".into(), false),
        (custom_file, "Custom weather".into(), custom_meta, "tune".into(), false),
    ];
    // OMSI 2's current weather: an airport's METAR report, fetched when the game starts
    let metar = l.state.choice.weather.strip_prefix("metar:").map(str::to_string);
    // (the airport nearest the map, not Berlin's for every map: Novi Sad got Berlin's rain)
    let home = nearest_airport(&l.state.config.root, &l.state.choice.map);
    items.push((format!("metar:{}", metar.clone().unwrap_or_else(|| home.clone())), "Current weather".into(), format!("METAR of {} (fetched at the start)", metar.clone().unwrap_or_else(|| home.clone())), "public".into(), false));
    // the weather going on from one to another through the day
    items.push(("cycle".into(), "Weather cycle".into(), "Changes every 25-60 minutes, as the month allows".into(), "autorenew".into(), false));
    for w in l.state.weathers.clone() {
        if !l.state.weather_fits(&w) {
            continue;
        }
        let vis = if w.fog_m >= 20000.0 { "clear air".to_string() } else { format!("{:.0} m", w.fog_m) };
        items.push((w.file.clone(), w.name.clone(), format!("{:.0} °C · {} · {vis}", w.temp, w.precip), weather_icon_of(&w).into(), l.state.fresh.contains_key(&w.file)));
    }
    if let Some(code) = metar.as_ref() {
        let root = std::path::PathBuf::from(&l.state.config.root);
        let list = crate::weather_setup::metar_airports(&root);

        let mut airport = code.to_uppercase().chars().take(4).collect::<String>();

        l.ui.label(Rect::new(r.x, y, 110.0, ROW), "Airport");

        let button_w = 150.0;
        let gap = 8.0;
        let input_w = r.w - 110.0 - button_w - gap;

        if l.ui.text_input(
            "metar-airport",
            Rect::new(r.x + 110.0, y, input_w, ROW),
            &mut airport,
            "ICAO",
            None,
        ) {
            airport = airport
                .chars()
                .filter(|c| c.is_ascii_alphabetic())
                .take(4)
                .collect::<String>()
                .to_uppercase();

            l.state.choice.weather = if airport.is_empty() {
                "metar:".into()
            } else {
                format!("metar:{airport}")
            };
            l.state.touched();
        }
        let labels: Vec<String> = list.iter().map(|a| a.1.clone()).collect();
        let mut sel = list
            .iter()
            .position(|a| a.0.eq_ignore_ascii_case(&airport))
            .unwrap_or(0);

        if l.ui.select(
            "metar-airport-list",
            Rect::new(r.x + 110.0 + input_w + gap, y, button_w, ROW),
            &mut sel,
            &labels,
        ) {
            if let Some(a) = list.get(sel) {
                l.state.choice.weather = format!("metar:{}", a.0);
                l.state.touched();
            }
        }

        y += ROW + 8.0;
    }
    // When Custom weather is selected, the weather cards turn into the editor. The value is
    // still just `choice.weather`, so it survives launcher restarts and goes to the game
    // through the normal `--weather` argument.
    if let Some(mut custom) = crate::weather_setup::custom_weather(Some(&l.state.choice.weather)) {
        let area = Rect::new(r.x - 4.0, y, r.w + 8.0, r.bottom() - y);
        let mut changed = false;
        let mut presets = false;
        l.ui.scroll_area("custom-weather-editor", area, &mut |ui, v| {
            let mut yy = v.y + 4.0;
            if ui.button("weather-presets", Rect::new(v.x + 4.0, yy, v.w - 12.0, 34.0), "Choose a weather preset", Some("arrow_back"), ButtonKind::Normal) {
                presets = true;
            }
            yy += 44.0;

            changed |= ui.slider("custom-vis", Rect::new(v.x + 4.0, yy, v.w - 12.0, 34.0), &mut custom.visibility_m, 50.0, 50_000.0, 50.0, "Visibility", &|x| if x >= 49_950.0 { "unlimited".into() } else if x >= 1000.0 { format!("{:.1} km", x / 1000.0) } else { format!("{x:.0} m") });
            yy += 40.0;
            changed |= ui.slider("custom-bright", Rect::new(v.x + 4.0, yy, v.w - 12.0, 34.0), &mut custom.brightness, 0.0, 1.5, 0.05, "Brightness", &|x| format!("{:.0} %", x * 100.0));
            yy += 40.0;
            changed |= ui.slider("custom-wdir", Rect::new(v.x + 4.0, yy, v.w - 12.0, 34.0), &mut custom.wind_dir, 0.0, 355.0, 5.0, "Wind direction", &|x| format!("{x:.0}°"));
            yy += 40.0;
            changed |= ui.slider("custom-wspeed", Rect::new(v.x + 4.0, yy, v.w - 12.0, 34.0), &mut custom.wind_speed, 0.0, 40.0, 0.5, "Wind speed", &|x| format!("{x:.1} m/s"));
            yy += 40.0;
            changed |= ui.slider("custom-temp", Rect::new(v.x + 4.0, yy, v.w - 12.0, 34.0), &mut custom.temp_c, -30.0, 45.0, 1.0, "Temperature", &|x| format!("{x:.0} °C"));
            yy += 40.0;
            let temp_for_dew = custom.temp_c;
            changed |= ui.slider("custom-hum", Rect::new(v.x + 4.0, yy, v.w - 12.0, 34.0), &mut custom.humidity, 0.0, 100.0, 1.0, "Humidity", &|x| format!("{x:.0} % · dew {:.0} °C", crate::weather_setup::dew_point_c(temp_for_dew, x)));
            yy += 44.0;

            ui.label(Rect::new(v.x + 4.0, yy, 130.0, 32.0), "Cloud type");
            let cloud_labels: Vec<String> = crate::weather_setup::CUSTOM_CLOUDS.iter().map(|x| (*x).to_string()).collect();
            let mut cloud = custom.cloud;
            if ui.select("custom-cloud", Rect::new(v.x + 134.0, yy, v.w - 142.0, 32.0), &mut cloud, &cloud_labels) {
                custom.cloud = cloud;
                changed = true;
            }
            yy += 44.0;

            ui.label(Rect::new(v.x + 4.0, yy, 130.0, 32.0), "Precipitation");
            let precip_labels: Vec<String> = crate::weather_setup::CUSTOM_PRECIP.iter().map(|x| (*x).to_string()).collect();
            let mut precip = custom.precip.clamp(0, 2) as usize;
            if ui.select("custom-precip", Rect::new(v.x + 134.0, yy, v.w - 142.0, 32.0), &mut precip, &precip_labels) {
                custom.precip = precip as i32;
                changed = true;
            }
            yy += 40.0;
            changed |= ui.slider("custom-intensity", Rect::new(v.x + 4.0, yy, v.w - 12.0, 34.0), &mut custom.precip_intensity, 0.0, 255.0, 1.0, "Precipitation intensity", &|x| format!("{x:.0} / 255"));
            yy += 40.0;
            changed |= ui.slider("custom-wet", Rect::new(v.x + 4.0, yy, v.w - 12.0, 34.0), &mut custom.road_wetness, 0.0, 1.0, 0.05, "Road wetness", &|x| format!("{:.0} %", x * 100.0));
            yy += 40.0;

            let mut snow = custom.snow_cover;
            if ui.toggle("custom-snow", Rect::new(v.x + 4.0, yy, (v.w - 20.0) * 0.5, 32.0), &mut snow, "Snow cover") {
                custom.snow_cover = snow;
                changed = true;
            }
            let mut snow_road = custom.snow_on_road;
            if ui.toggle("custom-snow-road", Rect::new(v.x + 12.0 + (v.w - 20.0) * 0.5, yy, (v.w - 20.0) * 0.5, 32.0), &mut snow_road, "Snow on road") {
                custom.snow_on_road = snow_road;
                changed = true;
            }
            yy += 44.0;
            yy - v.y
        });
        if presets {
            l.state.choice.weather.clear();
            l.state.touched();
        } else if changed {
            custom.normalize();
            l.state.choice.weather = custom.encode();
            l.state.touched();
        }
        return;
    }

    let chosen = l.state.choice.weather.clone();
    let mut pick = None;
    let area = Rect::new(r.x - 4.0, y, r.w + 8.0, r.bottom() - y);
    l.ui.scroll_area("weather", area, &mut |ui, v| {
        let cw = (v.w - 20.0) / 2.0;
        let ch = 62.0;
        for (k, (file, name, meta, icon, fresh)) in items.iter().enumerate() {
            let (cx, cy) = ((k % 2) as f32, (k / 2) as f32);
            let rr = Rect::new(v.x + 4.0 + cx * (cw + 8.0), v.y + cy * (ch + 8.0), cw, ch);
            let on = *file == chosen;
            let id = id_of(&format!("w-{file}"));
            let (h, _, clicked) = ui.interact(id, rr);
            if clicked {
                pick = Some(file.clone());
            }
            let t = ui.anim(id, if h { 1.0 } else { 0.0 }, 0.08);
            ui.p().rounded(rr, 6.0, if on { SELECTED() } else { FIELD().mix(HOVER(), t) });
            ui.p().rounded_border(rr, 6.0, 1.0, if on { ACCENT().alpha(0.6) } else { EDGE() });
            ui.icon(icon, Vec2::new(rr.x + 24.0, rr.center().y), 22.0, if on { TEXT() } else { TEXT_DIM() });
            let tw = ui.text_in(name, Rect::new(rr.x + 46.0, rr.y + 10.0, rr.w - 56.0, 20.0), 13.0, Weight::Bold, TEXT(), Align::Left);
            if *fresh {
                ui.badge(Vec2::new(rr.x + 50.0 + tw, rr.y + 12.0), "NEW", OK());
            }
            ui.text_in(meta, Rect::new(rr.x + 46.0, rr.y + 33.0, rr.w - 56.0, 18.0), 11.0, Weight::Regular, TEXT_DIM(), Align::Left);
        }
        ((items.len() + 1) / 2) as f32 * (ch + 8.0) + 4.0
    });
    if let Some(f) = pick {
        l.state.choice.weather = f;
        l.state.touched();
    }
}

fn step_roadbook(l: &mut Launcher, r: Rect) {
    let (Some(line), Some(tour), false) = (l.state.line().cloned(), l.state.tour().cloned(), l.state.choice.free) else {
        let map = l.state.map().map(|m| if m.friendly.is_empty() { m.name.clone() } else { m.friendly.clone() }).unwrap_or_default();
        l.ui.paragraph(&format!("No duty chosen: free driving on {map}. Pick a line and a tour under Route to see the roadbook here."), Vec2::new(r.x, r.y), r.w, 13.0, Weight::Regular, TEXT_DIM());
        ibis_box(l, Rect::new(r.x, r.y + 60.0, r.w, 160.0));
        return;
    };
    let from = l.state.first_trip().unwrap_or(0);
    l.ui.text_in(&format!("Line {} · tour {} · from {}", line.name, tour.number, hhmm(l.state.choice.time as f64 * 60.0)), Rect::new(r.x, r.y, r.w, 22.0), 14.0, Weight::Bold, TEXT(), Align::Left);
    l.ui.text_in("Click a trip to start the tour there.", Rect::new(r.x, r.y + 20.0, r.w, 16.0), 11.5, Weight::Regular, TEXT_DIM(), Align::Left);
    let trips: Vec<omsi_launcher_lib::TripInfo> = tour.trips.clone();
    let ibis_h = 150.0;
    let list = Rect::new(r.x - 4.0, r.y + 40.0, r.w + 8.0, r.h - 40.0 - ibis_h - 10.0);
    let mut start_at = None;
    let key = (line.name.clone(), tour.number.clone(), from);
    if l.drive.book_scrolled.as_ref() != Some(&key) {
        // (each earlier trip is one row of 52; the first trip goes to the top)
        l.ui.scroll_to("roadbook", from as f32 * 52.0, list.h, list.h);
        l.drive.book_scrolled = Some(key);
    }
    l.ui.scroll_area("roadbook", list, &mut |ui, v| {
        let mut y = v.y;
        for (i, t) in trips.iter().enumerate() {
            let head = Rect::new(v.x + 4.0, y, v.w - 12.0, 46.0);
            if i < from {
                if ui.row(&format!("roadbook-trip-{i}"), head, false) {
                    start_at = Some((t.index, t.departure));
                }
                ui.text_in(&format!("{} · {} → {}", omsi_ui::tr("Earlier"), if t.from.is_empty() { "?" } else { &t.from }, t.terminus), Rect::new(head.x + 10.0, head.y + 4.0, head.w - 20.0, 20.0), 13.0, Weight::Bold, TEXT_FAINT(), Align::Left);
                ui.text_in(&format!("{} - {} · {}", hhmm(t.departure), hhmm(t.arrival), if t.line.is_empty() { "depot run".to_string() } else { format!("line {}", t.line) }), Rect::new(head.x + 10.0, head.y + 24.0, head.w - 20.0, 18.0), 11.0, Weight::Regular, TEXT_FAINT(), Align::Left);
                y += 52.0;
                continue;
            }
            let k = i - from;
            if k == 0 {
                ui.p().rounded(head, 6.0, SELECTED());
            } else if ui.row(&format!("roadbook-trip-{i}"), head, false) {
                start_at = Some((t.index, t.departure));
            }
            ui.text_in(&format!("{} · {} → {}", if k == 0 { "Your first trip" } else { "Then" }, if t.from.is_empty() { "?" } else { &t.from }, t.terminus), Rect::new(head.x + 10.0, head.y + 4.0, head.w - 20.0, 20.0), 13.0, Weight::Bold, TEXT(), Align::Left);
            ui.text_in(&format!("{} - {} · {:.1} km · {}{}", hhmm(t.departure), hhmm(t.arrival), t.km, if t.line.is_empty() { "depot run".to_string() } else { format!("line {}", t.line) }, format!(" · {}", t.name)), Rect::new(head.x + 10.0, head.y + 24.0, head.w - 20.0, 18.0), 11.0, Weight::Regular, TEXT_DIM(), Align::Left);
            y += 52.0;
            let n = t.stops.len();
            for (s, st) in t.stops.iter().enumerate() {
                let rr = Rect::new(v.x + 4.0, y, v.w - 12.0, 24.0);
                // the timeline: a line with a dot per stop
                let cx = rr.x + 60.0;
                if s + 1 < n {
                    ui.p().rect(Rect::new(cx - 1.0, rr.center().y, 2.0, 24.0), LIFT().alpha(0.12));
                }
                let end = s == 0 || s + 1 == n;
                ui.p().circle(Vec2::new(cx, rr.center().y), if end { 4.0 } else { 3.0 }, if end { TEXT() } else { TEXT_FAINT() });
                ui.text_in(&hhmm(st.arr), Rect::new(rr.x + 6.0, rr.y, 42.0, rr.h), 12.0, Weight::Condensed, if end { TEXT() } else { TEXT_SOFT() }, Align::Left);
                ui.text_in(&st.name, Rect::new(cx + 14.0, rr.y, rr.w - 140.0, rr.h), 12.5, if end { Weight::Bold } else { Weight::Regular }, TEXT(), Align::Left);
                if s == 0 {
                    ui.text_in(&format!("dep {}", hhmm(st.dep)), Rect::new(rr.right() - 80.0, rr.y, 76.0, rr.h), 11.0, Weight::Medium, TEXT_DIM(), Align::Right);
                }
                y += 24.0;
            }
            y += 12.0;
        }
        y - v.y
    });
    if let Some((index, dep)) = start_at {
        l.state.pick_trip(index, dep);
    }
    ibis_box(l, Rect::new(r.x, r.bottom() - ibis_h, r.w, ibis_h));
}

pub(super) fn ibis_box(l: &mut Launcher, r: Rect) {
    l.ui.p().rounded(r, 6.0, FIELD());
    let inner = l.ui.heading(Rect::new(r.x + 12.0, r.y + 10.0, r.w - 24.0, r.h - 20.0), "IBIS", Some("keyboard"));
    let Some((_, info)) = l.state.ibis.clone() else {
        l.ui.paragraph("Pick a line to see what to type into the IBIS. Shift+U in the game types it for you and puts the bus into service.", Vec2::new(inner.x, inner.y - 4.0), inner.w, 12.0, Weight::Regular, TEXT_DIM());
        return;
    };
    match info {
        Err(e) => {
            l.ui.paragraph(&e, Vec2::new(inner.x, inner.y - 4.0), inner.w, 12.0, Weight::Regular, DANGER());
        }
        Ok(i) if i.routes.is_empty() => {
            l.ui.paragraph(&format!("Depot file {} has no entries for this line. Type line {} and the terminus code by hand, or press Shift+U.", i.hof, i.line_code), Vec2::new(inner.x, inner.y - 4.0), inner.w, 12.0, Weight::Regular, TEXT_DIM());
        }
        Ok(i) => {
            let mut y = inner.y - 2.0;
            for rt in i.routes.iter().take(3) {
                let code = rt.code.clone();
                let (lc, rc) = if code.len() > 2 { (code[..code.len() - 2].to_string(), code[code.len() - 2..].to_string()) } else { (i.line_code.clone(), code.clone()) };
                let name = if rt.name.is_empty() { rt.terminus.clone() } else { rt.name.clone() };
                l.ui.text_in(&name, Rect::new(inner.x, y, inner.w - 170.0, 22.0), 12.5, Weight::Medium, TEXT(), Align::Left);
                let mut x = inner.right() - 160.0;
                for (label, v) in [("line", lc), ("route", rc)] {
                    l.ui.text_in(label, Rect::new(x, y, 34.0, 22.0), 11.0, Weight::Regular, TEXT_DIM(), Align::Left);
                    let cw = l.ui.width(&v, 13.0, Weight::Black) + 12.0;
                    let cr = Rect::new(x + 34.0, y + 2.0, cw, 18.0);
                    l.ui.p().rounded(cr, 4.0, SELECTED());
                    l.ui.text_in(&v, cr, 12.5, Weight::Bold, TEXT(), Align::Center);
                    x += 80.0;
                }
                y += 26.0;
            }
            l.ui.text_in(&format!("Depot file {} · Shift+U in the game types it for you", i.hof), Rect::new(inner.x, y + 2.0, inner.w, 18.0), 11.0, Weight::Regular, TEXT_FAINT(), Align::Left);
        }
    }
}

/// What the map picture should show, from the current choice: the map, the trip of the
/// chosen tour that a start now would begin with (`State::first_trip`, the same one the
/// launch choice takes), and the entry point the player picked.
fn map_look(l: &Launcher) -> super::mapview::Look {
    let file = l.state.choice.map.clone();
    // (the map list came from the content roots; ask them the same way, case-insensitively,
    // so a mod's map is found wherever its root sits)
    let global = omsi_cfg::find_in_roots(&file)
        .map(|(_, p)| p)
        .unwrap_or_else(|| omsi_cfg::resolve_path(std::path::Path::new(&l.state.config.root), &file));
    let trip = l.state.tour().and_then(|t| t.trips.get(l.state.first_trip().unwrap_or(0))).map(|t| t.name.clone()).unwrap_or_default();
    super::mapview::Look { map: file, global, date: l.state.choice.date.clone(), trip, entry: l.state.choice.entry }
}

/// The phone's Start: as the desktop's.
pub(super) fn start_from_phone(l: &mut Launcher) {
    start(l);
}

fn start(l: &mut Launcher) {
    if l.state.bus().is_none() || l.state.map().is_none() {
        l.state.set_status("Choose a bus and a map first.", true);
        return;
    }
    if l.state.choice.lan_mode == "join" && !l.state.join.0 {
        let t = l.state.join.1.clone();
        l.state.set_status(format!("LAN: {t}"), true);
        return;
    }
    let running = l.state.instances.iter().filter(|i| i.running).count();
    // a second game on one computer is for testing LAN play, not something to do by
    // accident: with one running, the button asks for a second click
    if running > 0 && l.state.second_armed.map(|t| t.elapsed().as_secs() >= 6).unwrap_or(true) {
        l.state.second_armed = Some(std::time::Instant::now());
        l.state.set_status("A game is running already (its window may be behind this one - see Sessions). Click again to start another one anyway.", true);
        return;
    }
    l.state.second_armed = None;
    l.state.choice.save();
    l.state.launch();
    if l.state.choice.lan_mode != "off" {
        l.go(super::Page::Sessions);
    }
}

/// The airport of OMSI's METAR list (`Weather/ICAO.txt`) nearest to where map `map` lies
/// (its `timezone.txt`), else Berlin's; read once per map.
pub(super) fn custom_weather_summary(c:&crate::weather_setup::CustomWeather)->String{
    let precip=match c.precip{
        1=>format!("rain {:.0}%",c.precip_intensity/255.0*100.0),
        2=>format!("snow {:.0}%",c.precip_intensity/255.0*100.0),
        _=>"dry".to_string(),
    };
    let vis=if c.visibility_m>=49_950.0{
        "clear visibility".to_string()
    }else if c.visibility_m>=1000.0{
        format!("{:.1} km",c.visibility_m/1000.0)
    }else{
        format!("{:.0} m",c.visibility_m)
    };
    format!("{:.0} °C · {:.0}% RH · {precip} · {vis}",c.temp_c,c.humidity)
}

pub(super) fn selected_weather_as_custom(root:&str,file:&str)->Option<String>{
    if file.is_empty()||file=="cycle"||file.to_ascii_lowercase().starts_with("metar:")||crate::weather_setup::custom_weather(Some(file)).is_some(){return None}
    let path=omsi_cfg::resolve_path(std::path::Path::new(root),file);
    let w=omsi_content::weather::Weather::load(&path).ok()?;
    let wet=(w.ground_wet[0]/255.0).clamp(0.0,1.0);
    Some(crate::weather_setup::CustomWeather::from_weather(&w,1.0,wet).encode())
}

pub(crate) fn nearest_airport(root: &str, map: &str) -> String {
    static CACHE: std::sync::Mutex<Option<hashbrown::HashMap<String, String>>> = std::sync::Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = cache.get_or_insert_with(Default::default);
    if let Some(c) = cache.get(map) {
        return c.clone();
    }
    // (the airports of the list round Europe and a little beyond, where OMSI's maps lie)
    const AIRPORTS: &[(&str, f64, f64)] = &[
        ("BKPR", 42.57, 21.04), ("EBBR", 50.90, 4.48), ("EDDH", 53.63, 9.99), ("EDDM", 48.35, 11.79),
        ("EDDN", 49.50, 11.08), ("EDDP", 51.42, 12.24), ("EDDS", 48.69, 9.22), ("EDDB", 52.37, 13.52),
        ("EDOP", 53.43, 11.78), ("EETN", 59.41, 24.83), ("EFHF", 60.25, 25.04), ("EGAA", 54.66, -6.22),
        ("EGCC", 53.35, -2.27), ("EGLL", 51.47, -0.45), ("EGPH", 55.95, -3.37), ("EHAM", 52.31, 4.76),
        ("EIDW", 53.42, -6.27), ("EKBI", 55.74, 9.15), ("ELLX", 49.63, 6.21), ("ENBR", 60.29, 5.22),
        ("ENGM", 60.19, 11.10), ("ESKN", 58.79, 16.91), ("EPKK", 50.08, 19.78), ("EPWA", 52.17, 20.97),
        ("EVRA", 56.92, 23.97), ("EYVI", 54.63, 25.29), ("LBSF", 42.70, 23.41), ("LDZA", 45.74, 16.07),
        ("LEAL", 38.28, -0.56), ("LEBB", 43.30, -2.91), ("LEMD", 40.47, -3.56), ("LEPA", 39.55, 2.74),
        ("LFBD", 44.83, -0.72), ("LFLY", 45.73, 5.08), ("LFMD", 43.54, 6.95), ("LFPG", 49.01, 2.55),
        ("LGAV", 37.94, 23.94), ("LHBP", 47.44, 19.26), ("LICJ", 38.18, 13.10), ("LIMC", 45.63, 8.72),
        ("LIRA", 41.80, 12.59), ("LOWI", 47.26, 11.34), ("LOWL", 48.23, 14.19), ("LOWW", 48.11, 16.57),
        ("LPPT", 38.78, -9.14), ("LQSA", 43.82, 18.33), ("LSGG", 46.24, 6.11), ("LSZH", 47.46, 8.55),
        ("LTAC", 40.13, 32.99), ("LWSK", 41.96, 21.62), ("LYBE", 44.82, 20.31), ("LYTV", 42.40, 18.72),
        ("LZIB", 48.17, 17.21), ("UKKK", 50.40, 30.45), ("ULLI", 59.80, 30.26), ("UMKK", 54.89, 20.59),
        ("UMMM", 53.88, 28.03), ("UUEE", 55.97, 37.41), ("USSS", 56.74, 60.80),
    ];
    let dir = std::path::Path::new(map).parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
    let tz = omsi_cfg::resolve_path(std::path::Path::new(root), &format!("{dir}/timezone.txt"));
    let code = omsi_map::TimeZone::load(&tz)
        .ok()
        .and_then(|t| t.lat_lon())
        .and_then(|(lat, lon)| {
            AIRPORTS
                .iter()
                .map(|(c, a, o)| (c, (a - lat).powi(2) + ((o - lon) * lat.to_radians().cos()).powi(2)))
                .min_by(|x, y| x.1.total_cmp(&y.1))
                .map(|x| x.0.to_string())
        })
        .unwrap_or_else(|| "EDDB".into());
    cache.insert(map.to_string(), code.clone());
    code
}

#[cfg(test)]
mod vehicle_picker_tests {
    use super::*;

    fn vehicle(folder: &str, maker: &str, name: &str, file: &str) -> omsi_launcher_lib::VehicleInfo {
        omsi_launcher_lib::VehicleInfo {
            name: format!("{maker} {name}"), manufacturer: maker.into(), type_name: name.into(),
            folder: folder.into(), file: format!("Vehicles/{folder}/{file}.bus"),
            description: String::new(), paints: vec!["Paint".into()], hofs: vec![],
            installed: false, missing_packs: vec![], numbers: vec![], default_paint: "Beige".into(),
        }
    }

    #[test]
    fn a_tour_search_finds_the_next_trip_of_a_route_and_skips_the_ones_gone() {
        let trip = |index: usize, name: &str, departure: f64| omsi_launcher_lib::TripInfo {
            name: name.into(),
            index,
            line: "9106".into(),
            from: "Massy".into(),
            terminus: "Cormeilles".into(),
            departure,
            arrival: departure + 1800.0,
            stops: Vec::new(),
            km: 10.0,
        };
        let tour = |trips: Vec<omsi_launcher_lib::TripInfo>| omsi_launcher_lib::TourInfo {
            number: "12".into(),
            ai_group: String::new(),
            first: 0.0,
            last: 0.0,
            days: "Mon-Fri".into(),
            runs: true,
            next_run: None,
            trips,
        };
        let h = |x: f64| x * 3600.0;
        let morning_b = tour(vec![trip(1, "9106B_HC_MASSY", h(7.0)), trip(2, "9106A_HC_CORMEILLES", h(8.0)), trip(3, "9106A_HC_MASSY", h(13.0))]);
        assert_eq!(tour_trip(&morning_b, h(12.0), "9106b"), (false, None));
        assert_eq!(tour_trip(&morning_b, h(12.0), ""), (false, Some(2)));
        let later_b = tour(vec![trip(1, "9106A_HC_MASSY", h(11.5)), trip(2, "9106B_HC_CORMEILLES", h(12.5)), trip(3, "9106B_HC_MASSY", h(14.0))]);
        assert_eq!(tour_trip(&later_b, h(12.0), "9106b"), (false, Some(1)));
        assert_eq!(tour_trip(&later_b, h(12.0), "12"), (false, Some(1)));
        assert_eq!(tour_trip(&later_b, h(15.0), ""), (true, Some(2)));
        assert_eq!(tour_trip(&later_b, h(15.0), "9106b"), (true, Some(1)));
    }

    #[test]
    fn empty_vehicle_types_use_the_file_name() {
        for empty in ["", "   "] {
            let manufacturers = build_bus_manufacturers(
                &[vehicle("Pack", "MAN", empty, "NL_202")],
                None,
                &Default::default(),
            );
            assert_eq!(manufacturers[0].variants[0].variant, "NL 202");
        }
    }

    #[test]
    fn omsi_manufacturer_groups_dl_and_lions_city_across_packs() {
        let vehicles = vec![
            vehicle("MAN_DL05", "MAN", "DL05", "dl05"),
            vehicle("MAN_DL05", "MAN", "DL07", "dl07"),
            vehicle("MAN_DL05", "MAN", "DL08", "dl08"),
            vehicle("MAN_DL05", "MAN", "DL09", "dl09"),
            vehicle("MAN_LC_MVG", "MAN", "Lion's City (MVG)", "mvg"),
            vehicle("MAN_LC_GUE", "MAN", "Lion's City G (ORN)", "orn"),
        ];
        let manufacturers = build_bus_manufacturers(&vehicles, None, &Default::default());
        assert_eq!(manufacturers.len(), 1);
        assert_eq!(manufacturers[0].name, "MAN");
        assert_eq!(manufacturers[0].variants.len(), 6);
        assert!(manufacturers[0].variants.iter().any(|v| v.variant == "DL07"));
        assert!(manufacturers[0].variants.iter().any(|v| v.variant == "Lion's City G (ORN)"));
        assert!(manufacturer_matches(&manufacturers[0], "orn"));
        assert!(!manufacturer_matches(&manufacturers[0], "not a bus"));
    }

    #[test]
    fn author_defined_manufacturer_and_complete_type_are_preserved() {
        let vehicles = vec![
            vehicle("Pack", "Mercedes-Benz Release", "MB_C2_E6_GN_BVG_Leasing 2", "leasing"),
            vehicle("Pack", "Mercedes-Benz", "O530", "o530"),
        ];
        let manufacturers = build_bus_manufacturers(&vehicles, None, &Default::default());
        assert_eq!(manufacturers.len(), 2);
        let maker = manufacturers.iter().find(|m| m.name == "Mercedes-Benz Release").unwrap();
        assert_eq!(maker.variants[0].variant, "MB C2 E6 GN BVG Leasing 2");
    }

    #[test]
    fn duplicate_names_remain_selectable_and_host_filter_is_respected() {
        let vehicles = vec![
            vehicle("MAN", "MAN", "NL202", "en92"),
            vehicle("MAN", "MAN", "NL202", "en93"),
            vehicle("OtherPack", "MAN", "NL202", "en92"),
        ];
        let manufacturers = build_bus_manufacturers(&vehicles, None, &Default::default());
        let labels: std::collections::HashSet<_> = manufacturers[0].variants.iter().map(|v| &v.variant).collect();
        assert_eq!(labels.len(), 3);
        let allowed = std::collections::HashSet::from([vehicles[1].file.to_lowercase()]);
        let filtered = build_bus_manufacturers(&vehicles, Some(&allowed), &Default::default());
        assert_eq!(filtered[0].variants.len(), 1);
        assert_eq!(filtered[0].variants[0].file, vehicles[1].file);
    }

    /// Each type in a family's dropdown says how many liveries it has, as the Livery list
    /// beside counts them: its own paint and its repaints (#717).
    #[test]
    fn each_bus_type_says_how_many_liveries_it_has() {
        let mut one = vehicle("MAN_SD200", "MAN", "SD77", "sd77");
        one.paints.clear();
        let mut many = vehicle("MAN_SD200", "MAN", "SD78", "sd78");
        many.paints = vec!["BVG".into(), "Werbung".into(), "Neu".into()];
        let manufacturers = build_bus_manufacturers(&[one, many], None, &Default::default());
        let options: Vec<String> = manufacturers[0].variants.iter().map(variant_option).collect();
        assert_eq!(options, vec!["SD77 · 1 livery".to_string(), "SD78 · 4 liveries".to_string()]);
    }

    #[test]
    fn numbers_in_type_names_sort_naturally_and_default_livery_has_its_omsi_name() {
        assert_eq!(bus_name_cmp("DL9", "DL10"), std::cmp::Ordering::Less);
        assert_eq!(bus_name_cmp("MAN", "man"), std::cmp::Ordering::Equal);
        let mut vehicle = vehicle("MAN", "MAN", "DL07", "dl07");
        assert_eq!(default_livery_label(&vehicle), "Beige");
        vehicle.default_paint.clear();
        assert_eq!(default_livery_label(&vehicle), "Default paint");
    }
}
