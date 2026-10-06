//! The Line editor: lines of the player's own on a map. The stops are clicked on the map in
//! the order the bus calls at them, and the way between two of them is found over the map's
//! roads (the lanes the game drives, on their own side of the road); the way back is made
//! from the stops across the road. A line has a number, a name, a colour, the depot whose
//! buses drive it, the texts its displays show and a timetable of first and last departures
//! and the interval between them.
//!
//! A line is kept as a file of its own in `~/.openomsi/lines/<map>/` - the map's own
//! timetable (and the OMSI 2 folder) is not touched. "Drive it" starts a free drive on the
//! map at the entry point nearest the line's first stop.

use super::mapview::{self, MapView};
use super::theme::*;
use super::ui::{id_of, ButtonKind, Ui};
use super::{Launcher, Page};
use glam::{DVec2, Vec2};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The colours a line can have.
const COLOURS: [[u8; 3]; 8] = [[41, 98, 240], [226, 50, 60], [40, 168, 90], [240, 168, 0], [140, 60, 200], [0, 160, 150], [230, 40, 130], [120, 130, 140]];

/// The days a timetable row is for.
const DAYS: [&str; 3] = ["Mon – Fri", "Saturday", "Sunday"];

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct Stop {
    pub name: String,
    pub x: f64,
    pub y: f64,
}

impl Stop {
    fn at(&self) -> DVec2 {
        DVec2::new(self.x, self.y)
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct Service {
    pub on: bool,
    /// First and last departure (minutes after midnight) and the minutes between them.
    pub first: i32,
    pub last: i32,
    pub every: i32,
}

impl Service {
    /// How many trips the row makes.
    fn trips(&self) -> i32 {
        if !self.on || self.every <= 0 || self.last < self.first {
            0
        } else {
            (self.last - self.first) / self.every + 1
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct UserLine {
    pub number: String,
    pub name: String,
    pub colour: [u8; 3],
    pub depot: String,
    /// The stops, outbound and back.
    pub stops: [Vec<Stop>; 2],
    /// The displays' texts, outbound and back (the destination first).
    pub texts: [Vec<String>; 2],
    pub services: Vec<Service>,
}

impl UserLine {
    fn new(number: usize) -> UserLine {
        UserLine {
            number: number.to_string(),
            name: format!("{} {number}", omsi_ui::tr("Line")),
            colour: COLOURS[(number + 7) % COLOURS.len()],
            depot: String::new(),
            stops: [Vec::new(), Vec::new()],
            texts: [vec![String::new(); 4], vec![String::new(); 4]],
            services: vec![
                Service { on: true, first: 5 * 60, last: 23 * 60, every: 20 },
                Service { on: true, first: 6 * 60, last: 23 * 60, every: 30 },
                Service { on: true, first: 8 * 60, last: 22 * 60, every: 60 },
            ],
        }
    }

    /// The file it is kept in (by its number).
    fn file_name(&self) -> String {
        let n: String = self.number.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
        format!("line_{n}.json")
    }
}

/// The way back: the outbound stops in reverse, each taken at its twin across the road (the
/// stop of the same name nearest to it, within 150 m), else at itself.
pub fn way_back(out: &[Stop], all: &[(String, DVec2)]) -> Vec<Stop> {
    out.iter()
        .rev()
        .map(|s| {
            all.iter()
                .filter(|(n, p)| n.eq_ignore_ascii_case(&s.name) && (*p - s.at()).length() > 0.5 && (*p - s.at()).length() < 150.0)
                .min_by(|a, b| (a.1 - s.at()).length().total_cmp(&(b.1 - s.at()).length()))
                .map(|(n, p)| Stop { name: n.clone(), x: p.x, y: p.y })
                .unwrap_or_else(|| s.clone())
        })
        .collect()
}

#[derive(Default)]
pub struct LinesView {
    pub map: MapView,
    /// The map chosen (its place in the launcher's list) and the lines read for it.
    map_index: Option<usize>,
    loaded_for: Option<String>,
    lines: Vec<UserLine>,
    /// The lines as they are on the disk (what Save would change).
    saved: Vec<UserLine>,
    selected: usize,
    /// 0 outbound, 1 back.
    dir: usize,
    /// 0 stops, 1 displays, 2 timetable.
    tab: usize,
    /// The way between each two stops, by direction (found when the stops change).
    ways: [Vec<Option<Vec<DVec2>>>; 2],
    ways_for: [Vec<Stop>; 2],
    /// When each leg was found (the interface's clock): a new leg is drawn growing from its
    /// first stop to its second.
    born: [Vec<f32>; 2],
    /// The map's picture this frame and its texture in the interface.
    pub rect: Option<Rect>,
    pub tex: Option<usize>,
    pub gen: u64,
}

fn lines_dir(map: &str) -> PathBuf {
    let m: String = map.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    omsi_launcher_lib::data_dir().join("lines").join(m)
}

fn read_lines(map: &str) -> Vec<UserLine> {
    let mut v: Vec<UserLine> = std::fs::read_dir(lines_dir(map))
        .map(|d| d.flatten().filter_map(|e| std::fs::read_to_string(e.path()).ok()).filter_map(|t| serde_json::from_str(&t).ok()).collect())
        .unwrap_or_default();
    v.sort_by(|a, b| super::drive::natural(&a.number).cmp(&super::drive::natural(&b.number)));
    v
}

/// The map the editor shows (its `global.cfg`) as the map view wants it.
fn map_look(l: &Launcher, i: usize) -> Option<mapview::Look> {
    let m = l.state.maps.get(i)?;
    let global = omsi_cfg::resolve_path(std::path::Path::new(&l.state.config.root), &m.file);
    Some(mapview::Look { map: m.file.clone(), global, date: l.state.choice.date.clone(), trip: String::new(), entry: -1 })
}

fn map_name(l: &Launcher, i: usize) -> String {
    l.state.maps.get(i).map(|m| if m.friendly.is_empty() { m.name.clone() } else { m.friendly.clone() }).unwrap_or_default()
}

pub fn draw(l: &mut Launcher, area: Rect) {
    // the map: the one chosen for driving, the first time
    if l.lines.map_index.is_none() && !l.state.maps.is_empty() {
        l.lines.map_index = Some(l.state.maps.iter().position(|m| m.file == l.state.choice.map).unwrap_or(0));
        l.lines.map.show_entries = false;
    }
    let Some(mi) = l.lines.map_index else {
        l.page_title(area, "Line editor", "Reading the maps…");
        return;
    };
    let folder = l.state.maps[mi].name.clone();
    if l.lines.loaded_for.as_deref() != Some(folder.as_str()) {
        l.lines.lines = read_lines(&folder);
        l.lines.saved = l.lines.lines.clone();
        l.lines.selected = 0;
        l.lines.loaded_for = Some(folder.clone());
        if l.lines.lines.is_empty() {
            l.lines.lines.push(UserLine::new(1));
        }
    }
    if let Some(look) = map_look(l, mi) {
        l.lines.map.want(look);
    }
    l.lines.selected = l.lines.selected.min(l.lines.lines.len().saturating_sub(1));

    let body = l.page_title(area, "Line editor", "Click the stops in order: the way between them is found over the roads. The way back is made from the stops across the road.");
    if l.ui.button("lines-back", Rect::new(area.right() - 100.0, area.y + 6.0, 100.0, 34.0), "Back", Some("chevron_left"), ButtonKind::Normal) {
        l.go(Page::Home);
    }
    let gap = 12.0;
    let left = Rect::new(body.x, body.y, 250.0, body.h);
    let right = Rect::new(body.right() - 290.0, body.y, 290.0, body.h);
    let map_r = Rect::new(left.right() + gap, body.y, right.x - left.right() - gap * 2.0, body.h);

    // the map first: the panels lie beside it and take the mouse before it
    map_background(l, map_r);
    ways(l);
    let hovered = overlay(l, map_r);
    // (the panels slide in from their sides when the page opens)
    let a = appear(l.page_t, 0.05, 0.45);
    left_panel(l, Rect::new(left.x - 40.0 * (1.0 - a), left.y, left.w, left.h), mi);
    right_panel(l, Rect::new(right.x + 40.0 * (1.0 - a), right.y, right.w, right.h));
    // a click on the map: the stop under it joins the line (at its end)
    let p = mapview::Pointer {
        at: l.ui.input.mouse,
        pressed: l.ui.input.pressed,
        released: l.ui.input.released,
        down: l.ui.input.down,
        wheel: l.ui.input.wheel.y,
        blocked: l.ui.over_ui || !map_r.contains(l.ui.input.mouse),
    };
    l.lines.map.think(map_r, map_r, l.ui.scale, p);
    if let Some(at) = l.lines.map.take_click() {
        // a stop of the map's, else a stop of the line's own where the click is on a road
        let picked = match hovered {
            Some(i) => Some(l.lines.map.bus_stops()[i].clone()),
            None => l.lines.map.on_road(at, 25.0).map(|q| {
                let n = l.lines.lines[l.lines.selected].stops[l.lines.dir].len() + 1;
                (format!("{} {n}", omsi_ui::tr("Stop")), q)
            }),
        };
        if let Some((name, at)) = picked {
            let (sel, dir) = (l.lines.selected, l.lines.dir);
            let line = &mut l.lines.lines[sel];
            let last = line.stops[dir].last().map(|s| s.at());
            if last.map(|q| (q - at).length() > 0.5).unwrap_or(true) {
                line.stops[dir].push(Stop { name: name.clone(), x: at.x, y: at.y });
                // the destination follows the last stop until it is typed
                let t = &mut line.texts[dir];
                if t.first().map(|d| d.is_empty() || line.stops[dir].len() > 1).unwrap_or(true) {
                    if let Some(d) = t.first_mut() {
                        *d = name;
                    }
                }
            }
        }
    }
}

/// The map's picture, or a word while it is read.
fn map_background(l: &mut Launcher, r: Rect) {
    l.lines.rect = Some(r);
    let status = l.lines.map.status();
    match (l.lines.tex, status.is_empty()) {
        (Some(tex), true) => l.ui.image(r, tex, RADIUS),
        _ => {
            l.ui.p().rounded(r, RADIUS, BACKDROP());
            let t = if status.is_empty() { "Loading…" } else { status };
            l.ui.text_in(t, Rect::new(r.x, r.center().y - 12.0, r.w, 24.0), 13.5, Weight::Regular, TEXT_FAINT(), Align::Center);
        }
    }
    l.ui.p().rounded_border(r, RADIUS, 1.0, EDGE());
}

/// The ways between the stops, found again for a direction whose stops changed.
fn ways(l: &mut Launcher) {
    let now = l.ui.time;
    let v = &mut l.lines;
    let Some(line) = v.lines.get(v.selected) else { return };
    for dir in 0..2 {
        let stops = line.stops[dir].clone();
        if v.ways_for[dir] == stops {
            continue;
        }
        // (only the legs that changed are looked for again)
        let old = std::mem::take(&mut v.ways[dir]);
        let old_born = std::mem::take(&mut v.born[dir]);
        let old_stops = std::mem::replace(&mut v.ways_for[dir], stops.clone());
        let legs: Vec<(Option<Vec<DVec2>>, f32)> = stops
            .windows(2)
            .enumerate()
            .map(|(k, w)| {
                if old_stops.get(k) == Some(&w[0]) && old_stops.get(k + 1) == Some(&w[1]) {
                    if let Some(Some(r)) = old.get(k) {
                        return (Some(r.clone()), old_born.get(k).copied().unwrap_or(now));
                    }
                }
                (v.map.route(w[0].at(), w[1].at()), now)
            })
            .collect();
        v.born[dir] = legs.iter().map(|l| l.1).collect();
        v.ways[dir] = legs.into_iter().map(|l| l.0).collect();
    }
}

/// The first `share` (0..1) of a polyline's length.
fn cut(pts: &[Vec2], share: f32) -> Vec<Vec2> {
    if share >= 1.0 {
        return pts.to_vec();
    }
    let total: f32 = pts.windows(2).map(|s| (s[1] - s[0]).length()).sum();
    let mut left = total * share.max(0.0);
    let mut out = Vec::new();
    for s in pts.windows(2) {
        out.push(s[0]);
        let d = (s[1] - s[0]).length();
        if d >= left {
            out.push(s[0] + (s[1] - s[0]) * (left / d.max(1e-6)));
            return out;
        }
        left -= d;
    }
    out
}

/// The point `d` along a polyline.
fn at_length(pts: &[Vec2], d: f32) -> Option<Vec2> {
    let mut left = d;
    for s in pts.windows(2) {
        let len = (s[1] - s[0]).length();
        if len >= left {
            return Some(s[0] + (s[1] - s[0]) * (left / len.max(1e-6)));
        }
        left -= len;
    }
    pts.last().copied()
}

/// Over the map: every stop of it as a dot, the line's way and its stops numbered. Returns
/// the map's stop under the mouse.
fn overlay(l: &mut Launcher, r: Rect) -> Option<usize> {
    let v = &l.lines;
    let Some(line) = v.lines.get(v.selected).cloned() else { return None };
    let stops: Vec<(String, DVec2)> = v.map.bus_stops().to_vec();
    let col = Color::rgba(line.colour[0], line.colour[1], line.colour[2], 1.0);
    let ways = v.ways[v.dir].clone();
    let other = v.ways[1 - v.dir].clone();
    let dir = v.dir;
    let proj = |p: DVec2| l.lines.map.project(p);
    let pts_of = |w: &[DVec2]| -> Vec<Vec2> { w.iter().map(|p| proj(*p)).collect() };
    let now = l.ui.time;
    let born = v.born[v.dir].clone();
    let mut lines_px: Vec<(Vec<Vec2>, bool)> = Vec::new();
    for w in other.iter().flatten() {
        lines_px.push((pts_of(w), false));
    }
    // a leg just found grows from its first stop to its second
    let mut whole: Vec<Vec2> = Vec::new();
    for (k, w) in ways.iter().enumerate() {
        let Some(w) = w else { continue };
        let pts = pts_of(w);
        whole.extend(pts.iter().copied());
        let g = appear(now - born.get(k).copied().unwrap_or(0.0), 0.0, 0.7);
        lines_px.push((cut(&pts, g), true));
    }
    let dots: Vec<Vec2> = stops.iter().map(|s| proj(s.1)).collect();
    let chosen: Vec<Vec2> = line.stops[dir].iter().map(|s| proj(s.at())).collect();
    let mouse = l.ui.input.mouse;
    let over = r.contains(mouse) && !l.ui.over_ui;
    l.ui.push_clip(r, RADIUS);
    for (pts, this) in &lines_px {
        for s in pts.windows(2) {
            l.ui.p().line(s[0], s[1], if *this { 5.0 } else { 3.0 }, if *this { col } else { col.alpha(0.35) });
        }
    }
    // the map's stops (hovered: bigger, named)
    let mut hovered = None;
    if over {
        let mut best = 14.0f32;
        for (i, d) in dots.iter().enumerate() {
            let dist = (*d - mouse).length();
            if dist < best {
                best = dist;
                hovered = Some(i);
            }
        }
    }
    for (i, d) in dots.iter().enumerate() {
        if !r.contains(*d) {
            continue;
        }
        let h = hovered == Some(i);
        l.ui.p().circle(*d, if h { 6.0 } else { 3.2 }, if h { TEXT() } else { TEXT_SOFT().alpha(0.75) });
        l.ui.p().circle(*d, if h { 3.5 } else { 1.6 }, RAIL());
    }
    // a bus running the line, there and back again, at an even pace on the screen
    let total: f32 = whole.windows(2).map(|s| (s[1] - s[0]).length()).sum();
    let grown = born.iter().all(|b| now - b > 0.7);
    if total > 40.0 && grown {
        let speed = 120.0;
        let cycle = total / speed;
        let ph = (now / cycle) % 2.0;
        let d = if ph < 1.0 { ph } else { 2.0 - ph } * total;
        if let Some(p) = at_length(&whole, d) {
            l.ui.p().circle(p, 11.0, col.alpha(0.25));
            l.ui.p().circle(p, 8.0, RAIL());
            l.ui.icon("directions_bus", p, 12.0, TEXT());
        }
    }
    // the line's stops, numbered (a stop just added pops up)
    for (k, c) in chosen.iter().enumerate() {
        // (a stop at the end of a leg comes when the leg has grown to it)
        let pop = match k.checked_sub(1).and_then(|i| born.get(i)) {
            Some(b) => {
                let x = ((now - b - 0.6) / 0.35).clamp(0.0, 1.0);
                if x <= 0.0 {
                    continue;
                }
                x + (x * std::f32::consts::PI).sin() * 0.35
            }
            None => 1.0,
        };
        l.ui.p().circle(*c, 9.0 * pop, TEXT());
        l.ui.p().circle(*c, 7.5 * pop, col);
        l.ui.text_in(&format!("{}", k + 1), Rect::new(c.x - 9.0, c.y - 9.0, 18.0, 18.0), 9.5, Weight::Bold, ON_ACCENT(), Align::Center);
    }
    if let Some(i) = hovered {
        let d = dots[i];
        let name = stops[i].0.clone();
        let w = l.ui.width(&name, 12.0, Weight::Medium) + 18.0;
        let tag = Rect::new(d.x + 10.0, d.y - 26.0, w, 22.0);
        l.ui.p().rounded(tag, 11.0, PANEL().alpha(0.95));
        l.ui.text_in(&name, tag, 12.0, Weight::Medium, TEXT(), Align::Center);
        l.ui.cursor = winit::window::CursorIcon::Pointer;
    }
    // a leg no road joins: said once, over the map
    let missing = ways.iter().filter(|w| w.is_none()).count();
    if missing > 0 {
        let t = omsi_ui::tr("No road joins some of the stops: they are joined straight").to_string();
        let w = l.ui.width(&t, 12.0, Weight::Medium) + 30.0;
        let tag = Rect::new(r.center().x - w * 0.5, r.y + 12.0, w, 26.0);
        l.ui.p().rounded(tag, 13.0, PANEL().alpha(0.95));
        l.ui.text_in(&t, tag, 12.0, Weight::Medium, WARN(), Align::Center);
        for (k, w) in ways.iter().enumerate() {
            if w.is_none() && k + 1 < chosen.len() {
                l.ui.p().line(chosen[k], chosen[k + 1], 2.0, WARN().alpha(0.8));
            }
        }
    }
    // the map's own buttons: zoom and back to the whole map are the wheel and a drag
    if l.lines.map.bus_stops().is_empty() && l.lines.map.status().is_empty() {
        l.ui.text_in("This map's timetable names no bus stops", Rect::new(r.x, r.bottom() - 40.0, r.w, 24.0), 12.5, Weight::Medium, TEXT_DIM(), Align::Center);
    }
    l.ui.pop_clip();
    hovered
}

/// The lines of the map, the chosen one's number, name, colour and depot, and the buttons.
fn left_panel(l: &mut Launcher, r: Rect, mi: usize) {
    l.ui.panel(r);
    let inner = Rect::new(r.x + 14.0, r.y + 14.0, r.w - 28.0, r.h - 28.0);
    let mut y = inner.y;
    l.ui.label(Rect::new(inner.x, y, inner.w, 18.0), "Your lines");
    y += 22.0;
    let names: Vec<String> = (0..l.state.maps.len()).map(|i| map_name(l, i)).collect();
    let mut sel = mi;
    if l.ui.select("lines-map", Rect::new(inner.x, y, inner.w, ROW), &mut sel, &names) && sel != mi {
        l.lines.map_index = Some(sel);
        l.lines.loaded_for = None;
        l.lines.ways = Default::default();
        l.lines.ways_for = Default::default();
    }
    y += ROW + 10.0;
    // the lines
    let n = l.lines.lines.len();
    let mut pick = None;
    for k in 0..n {
        let line = l.lines.lines[k].clone();
        let rr = Rect::new(inner.x, y, inner.w, 34.0);
        if y + 34.0 > inner.bottom() - 300.0 {
            break;
        }
        let on = k == l.lines.selected;
        if l.ui.row(&format!("lines-line-{k}"), rr, on) {
            pick = Some(k);
        }
        let chip = Rect::new(rr.x + 8.0, rr.y + 8.0, 34.0, 18.0);
        l.ui.p().rounded(chip, 4.0, Color::rgba(line.colour[0], line.colour[1], line.colour[2], 1.0));
        l.ui.text_in(&line.number, chip, 11.5, Weight::Bold, Color::rgba(255, 255, 255, 1.0), Align::Center);
        l.ui.text_in(&line.name, Rect::new(rr.x + 50.0, rr.y, rr.w - 56.0, rr.h), 12.5, if on { Weight::Bold } else { Weight::Medium }, TEXT(), Align::Left);
        y += 38.0;
    }
    if let Some(k) = pick {
        l.lines.selected = k;
        l.lines.dir = 0;
    }
    if l.ui.button("lines-new", Rect::new(inner.x, y, inner.w, 32.0), "New line", Some("add"), ButtonKind::Ghost) {
        let next = l.lines.lines.iter().filter_map(|x| x.number.trim().parse::<usize>().ok()).max().unwrap_or(0) + 1;
        l.lines.lines.push(UserLine::new(next));
        l.lines.selected = l.lines.lines.len() - 1;
        l.lines.dir = 0;
    }
    y += 44.0;
    // the chosen line's own fields
    let sel = l.lines.selected;
    if sel >= l.lines.lines.len() {
        return;
    }
    l.ui.text_in("Number", Rect::new(inner.x, y, 70.0, 14.0), 10.5, Weight::Medium, TEXT_DIM(), Align::Left);
    l.ui.text_in("Name", Rect::new(inner.x + 80.0, y, inner.w - 80.0, 14.0), 10.5, Weight::Medium, TEXT_DIM(), Align::Left);
    y += 16.0;
    let line = &mut l.lines.lines[sel];
    l.ui.text_input("lines-number", Rect::new(inner.x, y, 70.0, 32.0), &mut line.number, "1", None);
    l.ui.text_input("lines-name", Rect::new(inner.x + 80.0, y, inner.w - 80.0, 32.0), &mut line.name, "Line 1", None);
    y += 42.0;
    let cw = inner.w / COLOURS.len() as f32;
    for (k, c) in COLOURS.iter().enumerate() {
        let centre = Vec2::new(inner.x + cw * (k as f32 + 0.5), y + 12.0);
        let rr = Rect::new(centre.x - 12.0, centre.y - 12.0, 24.0, 24.0);
        let (h, _, clicked) = l.ui.interact(id_of(&format!("lines-colour-{k}")), rr);
        let on = l.lines.lines[sel].colour == *c;
        if on {
            l.ui.p().circle(centre, 12.5, TEXT());
        } else if h {
            l.ui.p().circle(centre, 12.0, TEXT_DIM());
        }
        l.ui.p().circle(centre, 10.0, Color::rgba(c[0], c[1], c[2], 1.0));
        if clicked {
            l.lines.lines[sel].colour = *c;
        }
    }
    y += 34.0;
    l.ui.text_in("Driven by the buses of", Rect::new(inner.x, y, inner.w, 14.0), 10.5, Weight::Medium, TEXT_DIM(), Align::Left);
    y += 16.0;
    let mut hofs: Vec<String> = l.state.vehicles.iter().flat_map(|v| v.hofs.iter().cloned()).collect();
    hofs.sort_by_key(|h| h.to_lowercase());
    hofs.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    let own = l.state.maps.get(mi).map(|m| m.hof.clone()).unwrap_or_default();
    if l.lines.lines[sel].depot.is_empty() && !own.is_empty() {
        l.lines.lines[sel].depot = own.clone();
    }
    if !hofs.iter().any(|h| h.eq_ignore_ascii_case(&l.lines.lines[sel].depot)) && !l.lines.lines[sel].depot.is_empty() {
        hofs.insert(0, l.lines.lines[sel].depot.clone());
    }
    if !hofs.is_empty() {
        let mut hs = hofs.iter().position(|h| h.eq_ignore_ascii_case(&l.lines.lines[sel].depot)).unwrap_or(0);
        if l.ui.select("lines-depot", Rect::new(inner.x, y, inner.w, ROW), &mut hs, &hofs) {
            l.lines.lines[sel].depot = hofs[hs].clone();
        }
    }

    // the buttons at the foot
    let by = inner.bottom() - 32.0 - 8.0 - 36.0;
    let dirty = l.lines.lines != l.lines.saved;
    if dirty {
        l.ui.text_in("Not saved yet", Rect::new(inner.x, by - 22.0, inner.w, 16.0), 11.0, Weight::Medium, WARN(), Align::Left);
    }
    if l.ui.button("lines-save", Rect::new(inner.x, by, inner.w, 36.0), "Save line", Some("save"), ButtonKind::Primary) {
        save(l, mi);
    }
    let half = (inner.w - 8.0) * 0.5;
    if l.ui.button("lines-delete", Rect::new(inner.x, inner.bottom() - 32.0, half, 32.0), "Delete line", Some("delete"), ButtonKind::Normal) {
        let line = l.lines.lines.remove(sel);
        let _ = std::fs::remove_file(lines_dir(&l.state.maps[mi].name).join(line.file_name()));
        l.lines.saved.retain(|s| s.file_name() != line.file_name());
        if l.lines.lines.is_empty() {
            l.lines.lines.push(UserLine::new(1));
        }
        l.lines.selected = 0;
        l.state.set_status(format!("{} {}", omsi_ui::tr("Line deleted:"), line.name), false);
    }
    if l.ui.button("lines-drive", Rect::new(inner.x + half + 8.0, inner.bottom() - 32.0, half, 32.0), "Drive it", Some("play_arrow"), ButtonKind::Normal) {
        drive(l, mi);
    }
}

/// The line written to its file (the way back made from the stops across the road when it
/// has none yet).
fn save(l: &mut Launcher, mi: usize) {
    let sel = l.lines.selected;
    let all = l.lines.map.bus_stops().to_vec();
    let line = &mut l.lines.lines[sel];
    if line.stops[1].is_empty() && line.stops[0].len() >= 2 {
        line.stops[1] = way_back(&line.stops[0], &all);
        if line.texts[1].first().map(|t| t.is_empty()).unwrap_or(true) {
            if let (Some(d), Some(first)) = (line.texts[1].first_mut(), line.stops[1].last()) {
                *d = first.name.clone();
            }
        }
    }
    let dir = lines_dir(&l.state.maps[mi].name);
    let line = line.clone();
    let res = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join(line.file_name()), serde_json::to_string_pretty(&line).unwrap_or_default()));
    match res {
        Ok(()) => {
            // (a line renumbered: its old file goes)
            if let Some(old) = l.lines.saved.get(sel).filter(|o| o.file_name() != line.file_name()) {
                let _ = std::fs::remove_file(dir.join(old.file_name()));
            }
            l.lines.saved = l.lines.lines.clone();
            l.state.set_status(format!("{} {} → {}", omsi_ui::tr("Line saved:"), line.name, dir.display()), false);
        }
        Err(e) => l.state.set_status(e.to_string(), true),
    }
}

/// A free drive on the line's map, put down at the entry point nearest its first stop.
fn drive(l: &mut Launcher, mi: usize) {
    let sel = l.lines.selected;
    let first = l.lines.lines[sel].stops[0].first().map(|s| s.at());
    let file = l.state.maps[mi].file.clone();
    if l.state.choice.map != file {
        l.state.select_map(&file);
    }
    l.state.choice.free = true;
    if let Some(e) = first.and_then(|p| l.lines.map.nearest_entry(p)) {
        l.state.choice.entry = e as i32;
    }
    l.state.touched();
    l.drive.tab = 2;
    l.go(Page::Drive);
}

/// The right panel: the stops, the displays and the timetable, outbound or back.
fn right_panel(l: &mut Launcher, r: Rect) {
    l.ui.panel(r);
    let inner = Rect::new(r.x + 14.0, r.y + 14.0, r.w - 28.0, r.h - 28.0);
    let mut tab = l.lines.tab;
    if l.ui.segmented("lines-tab", Rect::new(inner.x, inner.y, inner.w, 32.0), &mut tab, &["Stops", "Displays", "Timetable"]) {
        l.lines.tab = tab;
    }
    let mut y = inner.y + 42.0;
    if l.lines.tab != 2 {
        let mut dir = l.lines.dir;
        if l.ui.segmented("lines-dir", Rect::new(inner.x, y, inner.w, 30.0), &mut dir, &["Outbound", "Return"]) {
            l.lines.dir = dir;
        }
        y += 40.0;
    }
    let sel = l.lines.selected;
    if sel >= l.lines.lines.len() {
        return;
    }
    let body = Rect::new(inner.x, y, inner.w, inner.bottom() - y);
    match l.lines.tab {
        0 => stops_tab(l, body),
        1 => displays_tab(l, body),
        _ => timetable_tab(l, body),
    }
}

fn stops_tab(l: &mut Launcher, r: Rect) {
    let (sel, dir) = (l.lines.selected, l.lines.dir);
    let stops = l.lines.lines[sel].stops[dir].clone();
    let lengths: Vec<f64> = l.lines.ways[dir].iter().map(|w| w.as_ref().map(|p| p.windows(2).map(|s| (s[1] - s[0]).length()).sum()).unwrap_or(0.0)).collect();
    if stops.is_empty() {
        let t = if dir == 0 { "Click the line's first stop on the map." } else { "Click the stops of the way back, or leave them: saved, the way back is made from the stops across the road." };
        l.ui.paragraph(t, Vec2::new(r.x, r.y), r.w, 12.0, Weight::Regular, TEXT_DIM());
        if dir == 1 && l.lines.lines[sel].stops[0].len() >= 2 {
            if l.ui.button("lines-way-back", Rect::new(r.x, r.y + 60.0, r.w, 32.0), "Make the way back", Some("swap_horiz"), ButtonKind::Normal) {
                let all = l.lines.map.bus_stops().to_vec();
                let back = way_back(&l.lines.lines[sel].stops[0], &all);
                l.lines.lines[sel].stops[1] = back;
            }
        }
        return;
    }
    let mut remove = None;
    let total: f64 = lengths.iter().sum();
    let list = Rect::new(r.x, r.y, r.w, r.h - 30.0);
    l.ui.scroll_area("lines-stops", list, &mut |ui: &mut Ui, view: Rect| {
        let mut y = view.y;
        for (k, s) in stops.iter().enumerate() {
            let row = Rect::new(view.x, y, view.w - 10.0, 30.0);
            ui.p().circle(Vec2::new(row.x + 10.0, row.center().y), 7.0, ACCENT());
            ui.text_in(&format!("{}", k + 1), Rect::new(row.x + 3.0, row.y, 14.0, row.h), 9.0, Weight::Bold, ON_ACCENT(), Align::Center);
            ui.text_in(&s.name, Rect::new(row.x + 24.0, row.y, row.w - 56.0, row.h), 12.0, Weight::Medium, TEXT(), Align::Left);
            if ui.icon_button(&format!("lines-stop-x-{k}"), Vec2::new(row.right() - 12.0, row.center().y), 11.0, "close", "Remove the stop") {
                remove = Some(k);
            }
            y += 30.0;
            if let Some(m) = lengths.get(k) {
                ui.text_in(&format!("{:.0} m", m), Rect::new(row.x + 24.0, y - 4.0, row.w, 14.0), 10.0, Weight::Regular, TEXT_FAINT(), Align::Left);
                y += 12.0;
            }
        }
        y - view.y
    });
    if let Some(k) = remove {
        l.lines.lines[sel].stops[dir].remove(k);
    }
    l.ui.text_in(&format!("{} {} · {:.1} km", stops.len(), omsi_ui::tr("stops"), total / 1000.0), Rect::new(r.x, r.bottom() - 22.0, r.w, 18.0), 11.5, Weight::Medium, TEXT_DIM(), Align::Left);
}

fn displays_tab(l: &mut Launcher, r: Rect) {
    let (sel, dir) = (l.lines.selected, l.lines.dir);
    let number = l.lines.lines[sel].number.clone();
    let dest = l.lines.lines[sel].texts[dir].first().cloned().unwrap_or_default();
    // the destination display: amber dots on black
    let d = Rect::new(r.x, r.y, r.w, 54.0);
    l.ui.p().rounded(d, 6.0, Color::rgba(8, 8, 8, 1.0));
    l.ui.p().rounded_border(d, 6.0, 1.0, EDGE());
    let amber = Color::rgba(255, 170, 30, 1.0);
    l.ui.text_in(&number, Rect::new(d.x + 10.0, d.y, 54.0, d.h), 24.0, Weight::Bold, amber, Align::Left);
    l.ui.text_in(&dest.to_uppercase(), Rect::new(d.x + 64.0, d.y, d.w - 72.0, d.h), 15.0, Weight::Bold, amber, Align::Center);
    let mut y = d.bottom() + 12.0;
    let labels = ["Destination", "Via", "Via", "Line text"];
    for (k, label) in labels.iter().enumerate() {
        l.ui.text_in(label, Rect::new(r.x, y, r.w, 14.0), 10.5, Weight::Medium, TEXT_DIM(), Align::Left);
        y += 16.0;
        let texts = &mut l.lines.lines[sel].texts[dir];
        while texts.len() <= k {
            texts.push(String::new());
        }
        l.ui.text_input(&format!("lines-text-{dir}-{k}"), Rect::new(r.x, y, r.w, 32.0), &mut texts[k], "", None);
        y += 40.0;
    }
}

fn timetable_tab(l: &mut Launcher, r: Rect) {
    let sel = l.lines.selected;
    let mut y = r.y;
    let mut total = 0;
    for (k, day) in DAYS.iter().enumerate() {
        let s = &mut l.lines.lines[sel].services[k];
        let mut on = s.on;
        if l.ui.toggle(&format!("lines-day-{k}"), Rect::new(r.x, y, r.w, 24.0), &mut on, day) {
            s.on = on;
        }
        y += 30.0;
        if s.on {
            let half = (r.w - 8.0) * 0.5;
            l.ui.text_in("First", Rect::new(r.x, y, half, 14.0), 10.5, Weight::Medium, TEXT_DIM(), Align::Left);
            l.ui.text_in("Last", Rect::new(r.x + half + 8.0, y, half, 14.0), 10.5, Weight::Medium, TEXT_DIM(), Align::Left);
            y += 16.0;
            let mut first = s.first;
            let mut last = s.last;
            if l.ui.time_field(&format!("lines-first-{k}"), Rect::new(r.x, y, half, 32.0), &mut first) {
                l.lines.lines[sel].services[k].first = first;
            }
            if l.ui.time_field(&format!("lines-last-{k}"), Rect::new(r.x + half + 8.0, y, half, 32.0), &mut last) {
                l.lines.lines[sel].services[k].last = last;
            }
            y += 40.0;
            let s = &mut l.lines.lines[sel].services[k];
            let mut every = s.every as f32;
            if l.ui.slider(&format!("lines-every-{k}"), Rect::new(r.x, y, r.w, ROW), &mut every, 5.0, 120.0, 5.0, "Every", &|v| format!("{v:.0} min")) {
                s.every = every as i32;
            }
            y += ROW + 2.0;
            let n = s.trips();
            total += n;
            l.ui.text_in(&format!("{n} {}", omsi_ui::tr("trips")), Rect::new(r.x, y, r.w, 14.0), 10.5, Weight::Regular, TEXT_FAINT(), Align::Left);
            y += 22.0;
        }
        y += 6.0;
    }
    l.ui.text_in(&format!("{total} {}", omsi_ui::tr("trips in all, each way")), Rect::new(r.x, r.bottom() - 20.0, r.w, 16.0), 11.0, Weight::Medium, TEXT_DIM(), Align::Left);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_way_back_takes_the_stop_across_the_road() {
        let all = vec![("Markt".to_string(), DVec2::new(0.0, 0.0)), ("Markt".to_string(), DVec2::new(0.0, 12.0)), ("Bahnhof".to_string(), DVec2::new(500.0, 0.0)), ("Markt".to_string(), DVec2::new(0.0, 900.0))];
        let out = vec![Stop { name: "Markt".into(), x: 0.0, y: 0.0 }, Stop { name: "Bahnhof".into(), x: 500.0, y: 0.0 }];
        let back = way_back(&out, &all);
        assert_eq!(back[0].name, "Bahnhof");
        assert_eq!((back[0].x, back[0].y), (500.0, 0.0));
        // the twin across the road, not the far stop of the same name
        assert_eq!((back[1].x, back[1].y), (0.0, 12.0));
    }

    #[test]
    fn a_service_counts_its_trips() {
        let s = Service { on: true, first: 6 * 60, last: 8 * 60, every: 20 };
        assert_eq!(s.trips(), 7);
        assert_eq!(Service { on: false, ..s.clone() }.trips(), 0);
        assert_eq!(Service { last: 5 * 60, ..s }.trips(), 0);
    }

    #[test]
    fn a_line_survives_its_file() {
        let mut l = UserLine::new(7);
        l.stops[0].push(Stop { name: "Markt".into(), x: 1.5, y: -2.0 });
        let back: UserLine = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(back, l);
        assert_eq!(l.file_name(), "line_7.json");
    }
}
