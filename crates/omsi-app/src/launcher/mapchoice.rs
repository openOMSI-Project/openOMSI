//! The map step, as Omsi-Hub has it: the maps as tiles, each with the picture the map brings
//! for OMSI's own map choice (`picture.jpg`, 370 x 280), on a wide sheet - Hamburg is known
//! by its harbour sooner than by its name in a row - or, for whoever knows the map already,
//! as a list with the number of tours and the year, on a narrow sheet beside the map itself.
//! A switch goes between the two, and this computer remembers which one was wanted. A tile
//! chooses the map and nothing else: Next goes on, and the step after it has the map as its
//! ground.
//!
//! What openOMSI's own map choice offered stays: the friendly names, the mod and new marks,
//! the entry points (how many there are, and which one the bus is put down at), a search,
//! the way to the folders (Setup), and on a server the server's map, which is not changed
//! here.

use super::drive;
use super::flow::{self, Step};
use super::theme::*;
use super::ui::{id_of, ButtonKind, Ui};
use super::{Launcher, Page};
use glam::Vec2;
use omsi_launcher_lib as core;
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Instant;

/// What a map's folder says beyond its global.cfg: how many tours its timetable has for the
/// player, and the year it plays in (when anything says so).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Facts {
    pub tours: usize,
    pub year: Option<i32>,
}

/// What the thread reading the maps' folders sends back.
enum Read {
    Facts(String, Facts),
    /// A map's picture, or none when it has no picture of its own.
    Picture(String, Option<image::RgbaImage>),
}

/// The step's own state: the view, the search, and what is read of the maps' folders.
pub struct MapChoiceView {
    /// Tiles (the maps' pictures, a wide sheet) or the list beside the map.
    pub tiles: bool,
    pub filter: String,
    /// Tours and year by map file, as they come in.
    facts: HashMap<String, Facts>,
    /// The pictures on the GPU by map file (texture, width, height), the ones decoded but not
    /// yet uploaded, and the maps that have none.
    textures: HashMap<String, (usize, u32, u32)>,
    pending: Vec<(String, image::RgbaImage)>,
    bare: HashSet<String>,
    rx: Option<Receiver<Read>>,
    /// The maps (and the OMSI folder) the reading was started for.
    asked: u64,
    /// The pictures went with the GPU (a game, a lost device): they are read again.
    pictures_gone: bool,
    /// When the step was drawn last: coming back to it brings the chosen map into view.
    seen: Option<Instant>,
}

impl MapChoiceView {
    pub fn new() -> MapChoiceView {
        MapChoiceView { tiles: read_view(), filter: String::new(), facts: HashMap::new(), textures: HashMap::new(), pending: Vec::new(), bare: HashSet::new(), rx: None, asked: 0, pictures_gone: false, seen: None }
    }

    /// The tours and the year of a map, once its folder is read (the year is the one a date
    /// for the map could default to).
    pub fn facts(&self, file: &str) -> Option<Facts> {
        self.facts.get(file).copied()
    }

    /// Everything on the GPU goes with it; the pictures are read again when the step is next
    /// drawn (their numbers would point at the new device's other textures).
    pub fn drop_gpu(&mut self) {
        self.textures.clear();
        self.pending.clear();
        self.pictures_gone = true;
    }

    /// The pictures decoded since the last frame go to the GPU (`mod.rs`, where the device is).
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, gpu: &mut omsi_ui::Gpu) {
        self.poll();
        for (file, img) in std::mem::take(&mut self.pending) {
            // (one already up: the same picture read twice, before and after the GPU went)
            if self.textures.contains_key(&file) {
                continue;
            }
            let (w, h) = img.dimensions();
            let id = gpu.add_image(device, queue, w, h, img.as_raw());
            self.textures.insert(file, (id, w, h));
        }
    }

    /// Take in what the reading thread has sent.
    fn poll(&mut self) {
        let Some(rx) = self.rx.as_ref() else { return };
        while let Ok(r) = rx.try_recv() {
            match r {
                Read::Facts(file, f) => {
                    self.facts.insert(file, f);
                }
                Read::Picture(file, Some(img)) => self.pending.push((file, img)),
                Read::Picture(file, None) => {
                    self.bare.insert(file);
                }
            }
        }
    }

    /// Read the maps' pictures and facts on a thread of its own, when the list (or the OMSI
    /// folder) is not the one read last; after the GPU went, the pictures again.
    fn want(&mut self, root: &str, maps: &[core::MapInfo]) {
        if maps.is_empty() {
            return;
        }
        let key = maps_key(root, maps);
        let again = std::mem::take(&mut self.pictures_gone);
        if key == self.asked && !again {
            return;
        }
        let facts_too = key != self.asked;
        self.asked = key;
        let skip: HashSet<String> = self.textures.keys().chain(self.bare.iter()).cloned().collect();
        let base = PathBuf::from(root);
        let list: Vec<(String, PathBuf)> = maps.iter().map(|m| (m.file.clone(), map_dir(&base, &m.file))).collect();
        let (tx, rx) = channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            // the pictures first: they are what the tiles are (a reading started anew drops
            // this one's receiver, and the sending stops it)
            for (file, dir) in list.iter().filter(|x| !skip.contains(&x.0)) {
                if tx.send(Read::Picture(file.clone(), read_picture(dir))).is_err() {
                    return;
                }
            }
            if !facts_too {
                return;
            }
            let mut keys = Vec::new();
            let mut shipped: Option<HashMap<String, i32>> = None;
            for (file, dir) in &list {
                let tt = omsi_cfg::resolve_path(dir, "TTData");
                let key = format!("{FACTS_KEY}{}", dir.display());
                let stamp = core::index::folder_stamp(&[dir.clone(), tt.clone(), base.join("Situations")]);
                keys.push(key.clone());
                let f: Facts = core::index::cached(&key, stamp, || {
                    let shipped = shipped.get_or_insert_with(|| situation_years(&base));
                    (Facts { tours: count_tours(&tt), year: year_of(dir, shipped) }, Vec::new())
                });
                if tx.send(Read::Facts(file.clone(), f)).is_err() {
                    return;
                }
            }
            core::index::save(FACTS_KEY, Some(&keys));
        });
    }
}

/// The content index's prefix for the maps' facts.
const FACTS_KEY: &str = "mapfacts|";

/// The view this computer had last (`~/.openomsi/launcher-map-view.txt`; the tiles when none).
fn view_file() -> PathBuf {
    core::data_dir().join("launcher-map-view.txt")
}

fn read_view() -> bool {
    std::fs::read_to_string(view_file()).map(|t| t.trim() != "list").unwrap_or(true)
}

fn write_view(tiles: bool) {
    let _ = std::fs::write(view_file(), if tiles { "tiles\n" } else { "list\n" });
}

fn maps_key(root: &str, maps: &[core::MapInfo]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    root.hash(&mut h);
    for m in maps {
        m.file.hash(&mut h);
    }
    h.finish().max(1)
}

/// A map's folder, found the way the map picture finds it: in the content roots first (a
/// mod's map), else under the OMSI folder.
fn map_dir(root: &Path, file: &str) -> PathBuf {
    let global = omsi_cfg::find_in_roots(file).map(|(_, p)| p).unwrap_or_else(|| omsi_cfg::resolve_path(root, file));
    global.parent().map(Path::to_path_buf).unwrap_or(global)
}

/// The map's own picture for OMSI's map choice, no bigger than twice a tile; `None` when the
/// map has none (or it cannot be read).
fn read_picture(dir: &Path) -> Option<image::RgbaImage> {
    let path = omsi_cfg::resolve_path(dir, "picture.jpg");
    let bytes = omsi_cfg::vfs::read(&path).ok()?;
    let img = match image::load_from_memory(&bytes) {
        Ok(i) => i,
        Err(e) => {
            log::warn!("map picture {}: {e}", path.display());
            return None;
        }
    };
    let img = if img.width() > 740 || img.height() > 560 { img.thumbnail(740, 560) } else { img };
    Some(img.to_rgba8())
}

/// The tours of a map's timetable (`TTData/*.ttl`).
fn count_tours(tt: &Path) -> usize {
    let Some(entries) = omsi_cfg::vfs::list_dir(tt) else { return 0 };
    let lines: Vec<omsi_timetable::Line> = entries
        .iter()
        .filter(|(n, dir)| !dir && n.to_string_lossy().to_ascii_lowercase().ends_with(".ttl"))
        .filter_map(|(n, _)| omsi_timetable::Line::load(&tt.join(n)).ok())
        .collect();
    tours_for_player(&lines)
}

/// The tours a player can drive, counted as Omsi-Hub counts them: those with trips on a line
/// the player may drive (`[userallowed]`); a map that allows none, all with trips.
fn tours_for_player(lines: &[omsi_timetable::Line]) -> usize {
    let with_trips = |l: &omsi_timetable::Line| l.tours.iter().filter(|t| !t.trips.is_empty()).count();
    let allowed: usize = lines.iter().filter(|l| l.user_allowed).map(with_trips).sum();
    if allowed > 0 {
        allowed
    } else {
        lines.iter().map(with_trips).sum()
    }
}

/// The year a map plays in: where the game left it last (`laststn.osn`, and the copies
/// Omsi-Hub keeps of OMSI's own), else a situation OMSI ships for it, else a year in its
/// folder's name ("Vienna_2005_Line_24A"). Chrono maps belong to a period: Berlin-Spandau is
/// 1988, HafenCity 2016.
fn year_of(dir: &Path, shipped: &HashMap<String, i32>) -> Option<i32> {
    let folder = dir.file_name()?.to_string_lossy().to_string();
    for name in ["laststn.osn", "laststn.osn.voor-omsi-enhancer", "laststn.osn.voor-omsi-career"] {
        let p = dir.join(name);
        if !omsi_cfg::vfs::is_file(&p) {
            continue;
        }
        if let Some((_, y)) = omsi_cfg::CfgFile::read(&p).ok().as_ref().and_then(situation_place) {
            return Some(y);
        }
    }
    shipped.get(&folder.to_lowercase()).copied().or_else(|| year_in_name(&folder))
}

/// The years of the situations in OMSI's `Situations` folder, by the map's folder (lower
/// case); the first one in name order counts.
fn situation_years(root: &Path) -> HashMap<String, i32> {
    let mut files: Vec<PathBuf> = omsi_cfg::vfs::read_dir_paths(&root.join("Situations")).into_iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("osn"))).collect();
    files.sort();
    let mut out = HashMap::new();
    for f in files {
        if let Some((folder, y)) = omsi_cfg::CfgFile::read(&f).ok().as_ref().and_then(situation_place) {
            out.entry(folder.to_lowercase()).or_insert(y);
        }
    }
    out
}

/// Where and when a situation plays: its map's folder and the year. Not one Omsi-Hub wrote
/// itself (it carries the date of the last drive, not the map's).
fn situation_place(f: &omsi_cfg::CfgFile) -> Option<(String, i32)> {
    let (mut map, mut year, mut own) = (None, None, false);
    let mut r = f.reader();
    while let Some(k) = r.next_keyword() {
        match k.as_str() {
            "name" => {
                let n = r.str().trim().to_lowercase();
                own = n.starts_with("omsi enhancer") || n.starts_with("omsi career") || n.starts_with("omsi-hub");
            }
            "description" => {
                r.until("[end]");
            }
            "map" => map = map_folder(r.str()),
            "time" => year = Some(r.i32()).filter(|y| (1900..=2100).contains(y)),
            _ => {}
        }
    }
    if own {
        return None;
    }
    Some((map?, year?))
}

/// The map's folder in a situation's `[map]` line (`maps\Grundorf\global.cfg`).
fn map_folder(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split(['\\', '/']).map(str::trim).filter(|p| !p.is_empty()).collect();
    match parts.iter().position(|p| p.eq_ignore_ascii_case("maps")) {
        Some(k) => parts.get(k + 1).map(|s| s.to_string()),
        None if parts.len() >= 2 => Some(parts[parts.len() - 2].to_string()),
        None => None,
    }
}

/// A year standing in a name on its own: four digits between 1900 and 2099, no more digits
/// either side ("Hamburg109" has none, "Bad_Huegelsdorf_2020" has 2020).
fn year_in_name(name: &str) -> Option<i32> {
    let b = name.as_bytes();
    let mut k = 0;
    while k < b.len() {
        if !b[k].is_ascii_digit() {
            k += 1;
            continue;
        }
        let start = k;
        while k < b.len() && b[k].is_ascii_digit() {
            k += 1;
        }
        if k - start == 4 {
            if let Ok(y) = name[start..k].parse::<i32>() {
                if (1900..=2099).contains(&y) {
                    return Some(y);
                }
            }
        }
    }
    None
}

// --- what the step shows ----------------------------------------------------------------

/// A map as the step shows it.
#[derive(Clone, Debug, Default)]
struct Item {
    file: String,
    name: String,
    starts: usize,
    modded: bool,
    fresh: bool,
    facts: Option<Facts>,
    picture: Option<(usize, u32, u32)>,
    /// It has no picture of its own (as opposed to one still being read).
    bare: bool,
}

/// The search: in the name the step shows and in the map's folder.
fn matches(q: &str, name: &str, file: &str) -> bool {
    let q = q.trim().to_lowercase();
    q.is_empty() || name.to_lowercase().contains(&q) || file.to_lowercase().contains(&q)
}

fn display_name(m: &core::MapInfo) -> String {
    if m.friendly.is_empty() {
        m.name.clone()
    } else {
        m.friendly.clone()
    }
}

fn items(l: &Launcher) -> Vec<Item> {
    let v = &l.mapchoice;
    l.state
        .maps
        .iter()
        .filter(|m| matches(&v.filter, &display_name(m), &m.file))
        .map(|m| Item {
            file: m.file.clone(),
            name: display_name(m),
            starts: m.entry_points.len(),
            modded: m.installed,
            fresh: l.state.fresh.contains_key(&m.file),
            facts: v.facts.get(&m.file).copied(),
            picture: v.textures.get(&m.file).copied(),
            bare: v.bare.contains(&m.file),
        })
        .collect()
}

/// "212 tours · 2022", as a tile and the sheet's head say it.
fn caption(f: Option<Facts>) -> String {
    let Some(f) = f else { return "…".into() };
    let tours = match f.tours {
        0 => omsi_ui::tr("No timetable").into_owned(),
        1 => omsi_ui::tr("1 tour").into_owned(),
        n => omsi_ui::tr("%{n} tours").replace("%{n}", &n.to_string()),
    };
    match f.year {
        Some(y) => format!("{tours} · {y}"),
        None => tours,
    }
}

/// The line under the sheet's title: the chosen map and what it is, or the question.
fn sub_line(l: &Launcher) -> String {
    match l.state.map() {
        Some(m) => match l.mapchoice.facts(&m.file) {
            Some(f) => format!("{} · {}", display_name(m), caption(Some(f))),
            None => display_name(m),
        },
        None => omsi_ui::tr("Where are you driving today?").into_owned(),
    }
}

/// The step: the tiles on a wide sheet, or the list on a narrow one with the map beside it.
pub fn draw(l: &mut Launcher, window: Rect) {
    let now = Instant::now();
    let entering = l.mapchoice.seen.is_none_or(|t| now.duration_since(t).as_secs_f32() > 0.5);
    l.mapchoice.seen = Some(now);
    l.mapchoice.want(&l.state.config.root, &l.state.maps);
    l.mapchoice.poll();
    // (the chosen map is read for its picture meanwhile: the next step stands on it)
    l.mapview.want(drive::map_look(l));
    if l.mapchoice.tiles {
        tiles_view(l, window, entering);
    } else {
        list_view(l, window, entering);
    }
}

/// The space between two tiles, the narrowest a tile gets, and the height of its caption.
const TILE_GAP: f32 = 14.0;
const TILE_MIN: f32 = 236.0;
const CAPTION_H: f32 = 70.0;

/// The grid in `w` points: tiles to a row, a tile's width and its picture's height (OMSI's
/// map pictures are 370 x 280).
fn grid(w: f32) -> (usize, f32, f32) {
    let cols = (((w + TILE_GAP) / (TILE_MIN + TILE_GAP)).floor() as usize).clamp(1, 6);
    let tw = (w - TILE_GAP * (cols - 1) as f32) / cols as f32;
    (cols, tw, (tw * 280.0 / 370.0).round())
}

fn tiles_view(l: &mut Launcher, window: Rect, entering: bool) {
    flow::ground_picture(l, window);
    let size = l.ui.size;
    let r = flow::wide_rect(size, true);
    flow::sheet(l, r);
    // the grid in the middle, no wider than four or five tiles read well; the head over it
    let gw = (r.w - 112.0).min(1120.0).max(200.0);
    let gx = r.x + (r.w - gw) * 0.5;
    let sub = sub_line(l);
    let body = flow::sheet_head(l, Rect::new(gx - 20.0, r.y, gw + 40.0, r.h), "map", "Maps", &sub);
    let y = body.y;
    switch_and_search(l, Rect::new(gx, y, gw, 32.0));
    l.ui.p().rect(Rect::new(r.x, y + 46.0, r.w, 1.0), HAIRLINE);
    let foot_r = Rect::new(gx - 18.0, y, gw + 36.0, r.bottom() - y);
    let above = flow::sheet_foot(l, foot_r, &foot_text(l));
    // where the bus starts, in the foot's right end
    let foot_y = above.bottom();
    entry_choice(l, Rect::new(gx + gw - 340.0, foot_y + 7.0, 340.0, 32.0));
    let area = Rect::new(gx - 4.0, y + 60.0, gw + 16.0, (above.bottom() - y - 72.0).max(60.0));
    let items = items(l);
    if empty_note(l, area, items.len()) {
        flow_actions(l, flow::EDGE_IN);
        return;
    }
    let chosen = l.state.choice.map.clone();
    let locked = drive::joined_server_name(l).is_some();
    let (cols, _, pic_h) = grid(gw);
    if entering {
        if let Some(k) = items.iter().position(|it| it.file == chosen) {
            let row = (k / cols) as f32;
            l.ui.scroll_to("mapchoice-tiles", row * (pic_h + CAPTION_H + TILE_GAP), pic_h + CAPTION_H + 8.0, area.h);
        }
    }
    let mut pick = None;
    l.ui.scroll_area("mapchoice-tiles", area, &mut |ui, v| {
        let (h, p) = tile_grid(ui, Rect::new(v.x + 4.0, v.y, gw, v.h), &items, &chosen, locked);
        pick = p;
        h
    });
    if let Some(k) = pick {
        choose(l, &items[k].file);
    }
    flow_actions(l, flow::EDGE_IN);
}

/// The tiles from the top of `v` down; returns their height and the tile clicked.
fn tile_grid(ui: &mut Ui, v: Rect, items: &[Item], chosen: &str, locked: bool) -> (f32, Option<usize>) {
    let (cols, tw, pic_h) = grid(v.w);
    let th = pic_h + CAPTION_H;
    // (room above the first row for a tile lifted under the mouse)
    let top = 6.0;
    let mut pick = None;
    for (k, it) in items.iter().enumerate() {
        let r = Rect::new(v.x + (k % cols) as f32 * (tw + TILE_GAP), v.y + top + (k / cols) as f32 * (th + TILE_GAP), tw, th);
        if !ui.rect_visible(r) {
            continue;
        }
        super::tour::anchor(if it.file == chosen { "map-tile" } else { "map-tile-any" }, r);
        if tile(ui, k, r, pic_h, it, it.file == chosen, locked) {
            pick = Some(k);
        }
    }
    let rows = items.len().div_ceil(cols.max(1));
    (top + rows as f32 * (th + TILE_GAP), pick)
}

/// One map as a tile: its picture, its name and what it holds; the chosen one blue.
fn tile(ui: &mut Ui, k: usize, r: Rect, pic_h: f32, it: &Item, on: bool, locked: bool) -> bool {
    let id = id_of(&format!("mapchoice-tile-{k}"));
    // (under the mouse it moves as the start's tiles do, `Ui::tile`; on a server's map step the
    // maps are the server's and stand still)
    let (h, held, clicked) = ui.interact(id, r);
    let t = ui.tile_from(id, r, RADIUS, if locked { (false, false, false) } else { (h, held, clicked) });
    let base = r;
    let r = t.r;
    let pic_h = pic_h * t.grown();
    let pic = Rect::new(r.x, r.y, r.w, pic_h);
    let cap = Rect::new(r.x, r.y + pic_h, r.w, r.h - pic_h);
    ui.tile_shadow(&t);
    if on {
        ui.p().shadow(r.inset(-3.0), RADIUS + 3.0, 18.0, accent().alpha(0.4));
    }
    // the picture, its lower corners under the caption (the corners are rounded all round)
    match it.picture {
        Some((tex, w, hh)) => ui.tile_photo(&t, Rect::new(pic.x, pic.y, pic.w, pic.h + RADIUS), RADIUS, tex, w, hh),
        None => {
            ui.p().rounded(Rect::new(pic.x, pic.y, pic.w, pic.h + RADIUS), RADIUS, Color::rgba(14, 19, 30, 1.0));
            if it.bare {
                ui.icon("map", pic.center(), 40.0, TEXT_FAINT);
            }
        }
    }
    let fill = if on { accent() } else { FIELD.mix(HOVER, t.hover) };
    ui.p().rect(Rect::new(cap.x, cap.y, cap.w, RADIUS), fill);
    ui.p().rounded(cap, RADIUS, fill);
    ui.tile_light(&t, 0.12);
    if on {
        ui.p().rounded_border(r, RADIUS, 2.0, accent());
    } else {
        ui.tile_edge(&t, 1.0, EDGE);
    }
    // (the words' room is the tile's where it lies: they cut the same under the mouse)
    let cap_w = base.w;
    // the marks on the picture: a mod's map, one that came while the launcher was open
    let mut mx = pic.x + 10.0;
    for (word, bg, ink) in [(it.modded, ("MOD", Color::rgba(9, 12, 24, 0.82), Color::WHITE)), (it.fresh, ("NEW", accent(), on_accent()))].into_iter().filter(|x| x.0).map(|x| x.1) {
        let word = omsi_ui::tr(word).into_owned();
        let w = ui.width(&word, 10.5, Weight::Bold) + 14.0;
        let m = Rect::new(mx, pic.y + 10.0, w, 20.0);
        ui.p().rounded(m, 5.0, bg);
        ui.text_in(&word, m, 10.5, Weight::Bold, ink, Align::Center);
        mx += w + 6.0;
    }
    let (ink, soft) = if on { (on_accent(), on_accent().alpha(0.86)) } else { (TEXT, TEXT_DIM) };
    ui.text_in(&it.name, Rect::new(cap.x + 16.0, cap.y + 12.0, cap_w - 32.0, 22.0), 15.0, Weight::Bold, ink, Align::Left);
    // what it holds, and how many places the bus can start at on the right
    let starts = it.starts.to_string();
    let sw = ui.width(&starts, 12.5, Weight::Medium);
    let sr = Rect::new(cap.right() - 16.0 - sw - 18.0, cap.y + 38.0, sw + 18.0, 20.0);
    if it.starts > 0 {
        ui.icon("location_on", Vec2::new(sr.x + 6.0, sr.center().y), 13.0, soft);
        ui.text_in(&starts, Rect::new(sr.x + 16.0, sr.y, sw + 2.0, sr.h), 12.5, Weight::Medium, soft, Align::Left);
        ui.tooltip(sr, "Places the bus can start at");
    }
    ui.text_in(&caption(it.facts), Rect::new(cap.x + 16.0, cap.y + 38.0, (cap_w - 16.0 - sw - 18.0 - 24.0).max(20.0), 20.0), 12.5, Weight::Regular, soft, Align::Left);
    if locked && !on {
        ui.p().rounded(r, RADIUS, Color::rgba(9, 12, 24, 0.55));
    }
    t.clicked
}

fn list_view(l: &mut Launcher, window: Rect, entering: bool) {
    let size = l.ui.size;
    let s = flow::sheet_rect(size);
    // what the sheet leaves of the map: the map's names are kept in it
    let clear = Rect::new(s.right() + 20.0, flow::SHEET_TOP, (size.x - s.right() - 40.0).max(160.0), (size.y - flow::SHEET_TOP - flow::ACTION_BOTTOM - flow::ACTION_H - 20.0).max(160.0));
    l.map_background(window);
    flow::sheet(l, s);
    let sub = sub_line(l);
    let body = flow::sheet_head(l, s, "map", "Maps", &sub);
    let y = body.y;
    switch_and_search(l, Rect::new(s.x + 20.0, y, s.w - 40.0, 32.0));
    l.ui.p().rect(Rect::new(s.x, y + 46.0, s.w, 1.0), HAIRLINE);
    let above = flow::sheet_foot(l, Rect::new(s.x, y, s.w, s.bottom() - y), &foot_text(l));
    // where the bus starts, over the foot
    let ey = above.bottom() - ROW - 12.0;
    let has_entry = entry_choice(l, Rect::new(s.x + 20.0, ey, s.w - 40.0, ROW));
    let list_bottom = if has_entry { ey - 12.0 } else { above.bottom() - 8.0 };
    let items = items(l);
    let heads_y = y + 58.0;
    let area = Rect::new(s.x + 12.0, heads_y + 24.0, s.w - 24.0, (list_bottom - heads_y - 24.0).max(40.0));
    if !empty_note(l, area, items.len()) {
        let cols = columns(area.w - 8.0);
        for (k, head) in ["Map", "Tours", "Era"].iter().enumerate() {
            l.ui.text_in(&omsi_ui::tr(head).to_uppercase(), Rect::new(area.x + cols[k], heads_y, 120.0f32.min(area.w - cols[k]), 16.0), 10.0, Weight::Bold, TEXT_DIM, Align::Left);
        }
        if cols[3] > 0.0 {
            let c = Vec2::new(area.x + cols[3] + 7.0, heads_y + 8.0);
            l.ui.icon("location_on", c, 13.0, TEXT_DIM);
            l.ui.tooltip(Rect::new(c.x - 10.0, c.y - 10.0, 20.0, 20.0), "Starts");
        }
        let chosen = l.state.choice.map.clone();
        let locked = drive::joined_server_name(l).is_some();
        if entering {
            if let Some(k) = items.iter().position(|it| it.file == chosen) {
                l.ui.scroll_to("mapchoice-list", k as f32 * LIST_ROW, LIST_ROW, area.h);
            }
        }
        let mut pick = None;
        l.ui.scroll_area("mapchoice-list", area, &mut |ui, v| {
            let (h, p) = list_rows(ui, v, &items, &chosen, locked);
            pick = p;
            h
        });
        if let Some(k) = pick {
            choose(l, &items[k].file);
        }
    }
    // the map's names, off the sheet; the map takes the mouse last
    drive::map_labels(l, clear, &[s]);
    flow_actions(l, s.right() + 14.0);
    l.map_interact(window, clear);
}

/// A row of the list.
const LIST_ROW: f32 = 38.0;
/// The width of the list's number columns.
const NUMBER_COL: f32 = 54.0;

/// Where the list's columns begin in a row `w` wide: the name, the tours, the year, and the
/// entry points (0: no room for them).
fn columns(w: f32) -> [f32; 4] {
    let starts = w >= 320.0;
    let n = if starts { 3.0 } else { 2.0 };
    let first = w - n * NUMBER_COL;
    [36.0, first, first + NUMBER_COL, if starts { first + 2.0 * NUMBER_COL } else { 0.0 }]
}

/// The rows from the top of `v` down; returns their height and the row clicked.
fn list_rows(ui: &mut Ui, v: Rect, items: &[Item], chosen: &str, locked: bool) -> (f32, Option<usize>) {
    let w = v.w - 8.0;
    let cols = columns(w);
    let mut pick = None;
    for (k, it) in items.iter().enumerate() {
        let rr = Rect::new(v.x, v.y + k as f32 * LIST_ROW, w, LIST_ROW - 2.0);
        if !ui.rect_visible(rr) {
            continue;
        }
        let on = it.file == chosen;
        super::tour::anchor(if on { "map-tile" } else { "map-tile-any" }, rr);
        if ui.row(&format!("mapchoice-row-{k}"), rr, on) && !locked {
            pick = Some(k);
        }
        flow::radio(ui, Vec2::new(rr.x + 16.0, rr.center().y), on);
        let ink = if on { on_accent() } else if locked { TEXT_DIM } else { TEXT };
        let soft = if on { on_accent() } else { TEXT_SOFT };
        // the name, and after it the marks
        let marks: Vec<String> = [(it.modded, "MOD"), (it.fresh, "NEW")].iter().filter(|x| x.0).map(|x| omsi_ui::tr(x.1).into_owned()).collect();
        let marks_w: f32 = marks.iter().map(|m| ui.width(m, 9.5, Weight::Bold) + 14.0).sum();
        let name_w = (cols[1] - cols[0] - 10.0 - marks_w).max(30.0);
        let nw = ui.width(&it.name, 14.0, Weight::Bold).min(name_w);
        ui.text_in(&it.name, Rect::new(rr.x + cols[0], rr.y, name_w, rr.h), 14.0, Weight::Bold, ink, Align::Left);
        let mut mx = rr.x + cols[0] + nw + 6.0;
        for m in &marks {
            let mw = ui.width(m, 9.5, Weight::Bold) + 10.0;
            let mr = Rect::new(mx, rr.center().y - 8.0, mw, 16.0);
            ui.p().rounded(mr, 4.0, if on { Color::WHITE.alpha(0.22) } else { Color::WHITE.alpha(0.1) });
            ui.text_in(m, mr, 9.5, Weight::Bold, if on { on_accent() } else { TEXT_SOFT }, Align::Center);
            mx += mw + 4.0;
        }
        let (tours, year) = match it.facts {
            Some(f) => (f.tours.to_string(), f.year.map(|y| y.to_string()).unwrap_or_else(|| "-".into())),
            None => ("…".into(), "…".into()),
        };
        ui.text_in(&tours, Rect::new(rr.x + cols[1], rr.y, NUMBER_COL - 6.0, rr.h), 14.0, Weight::Bold, ink, Align::Left);
        ui.text_in(&year, Rect::new(rr.x + cols[2], rr.y, NUMBER_COL - 6.0, rr.h), 14.0, Weight::Regular, soft, Align::Left);
        if cols[3] > 0.0 {
            ui.text_in(&it.starts.to_string(), Rect::new(rr.x + cols[3], rr.y, NUMBER_COL - 6.0, rr.h), 14.0, Weight::Regular, soft, Align::Left);
        }
    }
    (items.len() as f32 * LIST_ROW, pick)
}

/// The row under the head: the view switch on the left; the search on the right - or, on a
/// server, whose map it is and the way back to driving alone.
fn switch_and_search(l: &mut Launcher, r: Rect) {
    super::tour::anchor("map-views", Rect::new(r.x, r.y, 156.0, r.h));
    if let Some(t) = view_switch(&mut l.ui, Rect::new(r.x, r.y, 156.0, r.h), l.mapchoice.tiles) {
        l.mapchoice.tiles = t;
        // (the chosen map into view in the other one too)
        l.mapchoice.seen = None;
        write_view(t);
    }
    let rest = Rect::new(r.x + 168.0, r.y, (r.w - 168.0).max(0.0), r.h);
    if let Some(server) = drive::joined_server_name(l) {
        let bw = (l.ui.width("Leave the server", 13.0, Weight::Bold) + 44.0).min(rest.w * 0.5);
        let b = Rect::new(rest.right() - bw, r.y, bw, r.h);
        if l.ui.button("mapchoice-leave", b, "Leave the server", Some("logout"), ButtonKind::Normal) {
            l.state.leave_server();
        }
        let t = Rect::new(rest.x, r.y, (b.x - rest.x - 10.0).max(0.0), r.h);
        l.ui.icon("lock", Vec2::new(t.x + 9.0, t.center().y), 14.0, TEXT_DIM);
        let line = omsi_ui::tr("%{server} chooses the map").replace("%{server}", &server);
        l.ui.text_in(&line, Rect::new(t.x + 24.0, t.y, (t.w - 24.0).max(0.0), t.h), 12.5, Weight::Medium, TEXT_SOFT, Align::Left);
        l.ui.tooltip(t, "On a server the map is the server's: it comes with the server when the game joins.");
        return;
    }
    let sw = rest.w.min(260.0);
    if sw >= 90.0 {
        l.ui.text_input("mapchoice-filter", Rect::new(rest.right() - sw, r.y, sw, r.h), &mut l.mapchoice.filter, "Search…", Some("search"));
    }
}

/// The two views as one control, as Omsi-Hub draws it: a pill with the chosen half lit.
/// Returns the view clicked when it is another one.
fn view_switch(ui: &mut Ui, r: Rect, tiles: bool) -> Option<bool> {
    ui.p().rounded(r, r.h * 0.5, Color::WHITE.alpha(0.02));
    ui.p().rounded_border(r, r.h * 0.5, 1.0, EDGE);
    let half = (r.w - 6.0) * 0.5;
    let mut out = None;
    for (k, (label, icon)) in [("List", "view_list"), ("Tiles", "grid_view")].iter().enumerate() {
        let cell = Rect::new(r.x + 3.0 + k as f32 * half, r.y + 3.0, half, r.h - 6.0);
        let on = (k == 1) == tiles;
        let (h, _, clicked) = ui.interact(id_of(&format!("mapchoice-view-{k}")), cell);
        if on {
            ui.p().rounded(cell, cell.h * 0.5, Color::WHITE.alpha(0.1));
        } else if h {
            ui.p().rounded(cell, cell.h * 0.5, Color::WHITE.alpha(0.04));
        }
        let c = if on { TEXT } else if h { TEXT_SOFT } else { TEXT_DIM };
        let word = omsi_ui::tr(label).into_owned();
        let tw = ui.width(&word, 12.5, Weight::Bold);
        let x0 = cell.center().x - (tw + 20.0) * 0.5;
        ui.icon(icon, Vec2::new(x0 + 7.0, cell.center().y), 14.0, c);
        ui.text_in(&word, Rect::new(x0 + 20.0, cell.y, tw + 2.0, cell.h), 12.5, if on { Weight::Bold } else { Weight::Medium }, c, Align::Left);
        if clicked && !on {
            out = Some(k == 1);
        }
    }
    out
}

/// The foot's line: how many maps there are (and how many the search leaves).
fn foot_text(l: &Launcher) -> String {
    let n = l.state.maps.len();
    if n == 0 && l.state.loading_content {
        return omsi_ui::tr("Reading the OMSI folder…").into_owned();
    }
    let shown = l.state.maps.iter().filter(|m| matches(&l.mapchoice.filter, &display_name(m), &m.file)).count();
    if shown < n {
        omsi_ui::tr("%{k} of %{n} maps").replace("%{k}", &shown.to_string()).replace("%{n}", &n.to_string())
    } else {
        omsi_ui::tr("%{n} maps installed").replace("%{n}", &n.to_string())
    }
}

/// Where the bus is put down on the chosen map: one of its entry points, as in OMSI 2, or
/// the automatic one (nearest to the duty's first stop; a free drive: the map's first). The
/// same choice as the duty step's and the orange marks on the map. Returns whether it is
/// there (a map with entry points, not on a server).
fn entry_choice(l: &mut Launcher, r: Rect) -> bool {
    if drive::joined_server_name(l).is_some() {
        return false;
    }
    let Some(m) = l.state.map() else { return false };
    if m.entry_points.is_empty() {
        return false;
    }
    let free = l.state.choice.free;
    let mut labels = vec![if free { "Automatic (the map's first)".to_string() } else { "Automatic (nearest to the first stop)".to_string() }];
    labels.extend(m.entry_points.iter().map(|e| if e.name.is_empty() { super::drive::entry_name(e.index) } else { e.name.clone() }));
    // (the choice is the entry's place in the list; 0 = automatic here)
    let mut es = if l.state.choice.entry < 0 { 0 } else { (l.state.choice.entry as usize + 1).min(labels.len() - 1) };
    let lw = l.ui.width("Start at", 13.0, Weight::Medium) + 14.0;
    super::tour::anchor("map-start", r);
    l.ui.label(Rect::new(r.x, r.y, lw, r.h), "Start at");
    if l.ui.select("mapchoice-entry", Rect::new(r.x + lw, r.y, r.w - lw, r.h), &mut es, &labels) {
        l.state.choice.entry = es as i32 - 1;
        l.state.touched();
    }
    l.ui.tooltip(Rect::new(r.x, r.y, lw, r.h), "Where the bus is put down. The orange marks on the map are the same places: click one to take it.");
    true
}

/// No maps (yet), or none the search finds: a word in the middle of `area` instead of the
/// grid. Returns whether there was nothing to show.
fn empty_note(l: &mut Launcher, area: Rect, shown: usize) -> bool {
    if shown > 0 {
        return false;
    }
    let (title, text) = if !l.state.maps.is_empty() {
        ("No map matches the search.", "")
    } else if l.state.loading_content {
        ("Reading the OMSI folder…", "")
    } else {
        ("No maps found.", "The maps are read from the maps folder of the OMSI 2 folder and from the content folder. Check installed folders shows where they are looked for.")
    };
    let c = area.center();
    l.ui.icon("map", Vec2::new(c.x, c.y - 46.0), 34.0, TEXT_FAINT);
    l.ui.text_in(title, Rect::new(area.x, c.y - 22.0, area.w, 24.0), 16.0, Weight::Bold, TEXT_SOFT, Align::Center);
    if !text.is_empty() {
        let w = area.w.min(460.0);
        l.ui.paragraph(text, Vec2::new(c.x - w * 0.5, c.y + 8.0), w, 12.5, Weight::Regular, TEXT_DIM);
    }
    true
}

/// A map clicked: it is the map (the step after this one stands on it).
fn choose(l: &mut Launcher, file: &str) {
    l.state.select_map(file);
}

/// Back to the mode, the way to the folders (openOMSI's Setup), and on.
fn flow_actions(l: &mut Launcher, from_x: f32) {
    let (back, next, extra) = flow::actions(l, from_x, true, "Next step", "play_arrow", &[("Check installed folders", "")]);
    if back {
        if let Some(p) = flow::previous_of(l, Step::Map) {
            l.drive.step = p;
        }
    }
    if extra == Some(0) {
        l.go(Page::Setup);
    }
    if next {
        if l.state.map().is_none() {
            l.state.set_status("Choose a map first.", true);
        } else if let Some(n) = flow::next_of(l, Step::Map) {
            l.drive.step = n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(allowed: bool, tours: &[usize]) -> omsi_timetable::Line {
        omsi_timetable::Line {
            user_allowed: allowed,
            tours: tours.iter().map(|n| omsi_timetable::Tour { trips: vec![Default::default(); *n], ..Default::default() }).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn tours_are_counted_as_the_player_has_them() {
        // the lines the player may drive, their tours with trips
        assert_eq!(tours_for_player(&[line(true, &[3, 0, 2]), line(false, &[4, 4]), line(true, &[1])]), 3);
        // a map that allows no line: every tour with trips
        assert_eq!(tours_for_player(&[line(false, &[3, 0]), line(false, &[1, 1])]), 3);
        assert_eq!(tours_for_player(&[]), 0);
    }

    #[test]
    fn a_year_in_a_name_stands_on_its_own() {
        assert_eq!(year_in_name("Vienna_2005_Line_24A"), Some(2005));
        assert_eq!(year_in_name("Bad_Huegelsdorf_2020"), Some(2020));
        assert_eq!(year_in_name("Hamburg109"), None);
        assert_eq!(year_in_name("HamburgLi20"), None);
        assert_eq!(year_in_name("Map 12345"), None);
        assert_eq!(year_in_name("Linie 1850 und 1988"), Some(1988));
    }

    #[test]
    fn a_situation_says_its_map_and_year() {
        let f = omsi_cfg::CfgFile::from_str("x.osn", "[name]\r\nLinie 5\r\n\r\n[description]\r\n[map]\r\nnot this\r\n[end]\r\n\r\n[map]\r\nmaps\\Berlin-Spandau\\global.cfg\r\n\r\n[time]\r\n1988\r\n152\r\n7\r\n30\r\n0.000000\r\n");
        assert_eq!(situation_place(&f), Some(("Berlin-Spandau".to_string(), 1988)));
        // one Omsi-Hub wrote carries the last drive's date, not the map's
        let own = omsi_cfg::CfgFile::from_str("y.osn", "[name]\r\nOMSI Enhancer\r\n\r\n[map]\r\nmaps\\Grundorf\\global.cfg\r\n\r\n[time]\r\n2026\r\n1\r\n");
        assert_eq!(situation_place(&own), None);
        assert_eq!(map_folder("maps/Grundorf/global.cfg").as_deref(), Some("Grundorf"));
        assert_eq!(map_folder("MAPS\\Hohenkirchen - Herrenhof\\global.cfg").as_deref(), Some("Hohenkirchen - Herrenhof"));
    }

    #[test]
    fn the_year_comes_from_the_last_situation_before_the_name() {
        let dir = std::env::temp_dir().join(format!("openomsi-mapchoice-{}", std::process::id())).join("Vienna_2005_Line_24A");
        std::fs::create_dir_all(&dir).unwrap();
        let none = HashMap::new();
        assert_eq!(year_of(&dir, &none), Some(2005));
        let shipped: HashMap<String, i32> = [("vienna_2005_line_24a".to_string(), 2004)].into_iter().collect();
        assert_eq!(year_of(&dir, &shipped), Some(2004));
        std::fs::write(dir.join("laststn.osn"), "[map]\r\nmaps\\Vienna_2005_Line_24A\\global.cfg\r\n\r\n[time]\r\n2006\r\n100\r\n").unwrap();
        assert_eq!(year_of(&dir, &shipped), Some(2006));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn the_caption_says_tours_and_year() {
        // (in the interface's language, which another test may have set: the words through
        // `tr` here as well)
        let t = |s: &str| omsi_ui::tr(s).into_owned();
        assert_eq!(caption(None), "…");
        assert_eq!(caption(Some(Facts { tours: 212, year: Some(2022) })), format!("{} · 2022", t("%{n} tours").replace("%{n}", "212")));
        assert_eq!(caption(Some(Facts { tours: 1, year: None })), t("1 tour"));
        assert_eq!(caption(Some(Facts { tours: 0, year: Some(2005) })), format!("{} · 2005", t("No timetable")));
    }

    #[test]
    fn the_search_looks_at_the_name_and_the_folder() {
        assert!(matches("", "Grundorf", "maps/Grundorf/global.cfg"));
        assert!(matches("hafen", "HafenCity - Hamburg Modern", "maps/HafenCityHamburg/global.cfg"));
        assert!(matches("li20", "Hamburg Line 20", "maps/HamburgLi20/global.cfg"));
        assert!(!matches("berlin", "Grundorf", "maps/Grundorf/global.cfg"));
    }

    #[test]
    fn the_grid_fills_its_width() {
        for w in [300.0, 916.0, 1120.0, 1800.0] {
            let (cols, tw, pic_h) = grid(w);
            assert!(cols >= 1 && cols <= 6);
            assert!(tw >= TILE_MIN.min(w) - 0.5, "{w}: {tw}");
            assert!((cols as f32 * tw + (cols - 1) as f32 * TILE_GAP - w).abs() < 0.5);
            assert!((pic_h - tw * 280.0 / 370.0).abs() <= 0.5);
        }
        assert_eq!(grid(1120.0).0, 4);
        // a narrow list keeps its name column, giving the entry points up first
        assert!(columns(330.0)[3] > 0.0);
        assert_eq!(columns(260.0)[3], 0.0);
        assert!(columns(260.0)[1] - columns(260.0)[0] >= 100.0);
    }

    fn some_maps() -> Vec<Item> {
        (0..7).map(|k| Item { file: format!("maps/m{k}/global.cfg"), name: format!("Map {k}"), starts: k, modded: k == 2, fresh: k == 3, facts: (k % 2 == 0).then_some(Facts { tours: 10 * k, year: Some(2000 + k as i32) }), ..Default::default() }).collect()
    }

    /// A frame of the grid (or the list) in a scrolling area, clicked at `at` when given.
    fn frame(ui: &mut Ui, tiles: bool, items: &[Item], locked: bool) -> Option<usize> {
        ui.begin(Vec2::new(1440.0, 900.0), 1.0, 1.0 / 60.0);
        let mut pick = None;
        let area = Rect::new(100.0, 100.0, 1140.0, 600.0);
        let name = if tiles { "mapchoice-tiles" } else { "mapchoice-list" };
        ui.scroll_area(name, area, &mut |ui, v| {
            let (h, p) = if tiles { tile_grid(ui, Rect::new(v.x, v.y, 1120.0, v.h), items, "maps/m1/global.cfg", locked) } else { list_rows(ui, v, items, "maps/m1/global.cfg", locked) };
            pick = p;
            h
        });
        pick
    }

    fn click(tiles: bool, id: &str, locked: bool) -> Option<usize> {
        let items = some_maps();
        let mut ui = Ui::new();
        frame(&mut ui, tiles, &items, locked);
        let r = *ui.drawn.get(&id_of(id)).unwrap_or_else(|| panic!("{id} is not drawn"));
        ui.input.mouse = r.center();
        ui.input.pressed = true;
        ui.input.down = true;
        frame(&mut ui, tiles, &items, locked);
        ui.input.pressed = false;
        ui.input.down = false;
        ui.input.released = true;
        frame(&mut ui, tiles, &items, locked)
    }

    #[test]
    fn a_tile_or_a_row_chooses_its_map() {
        assert_eq!(click(true, "mapchoice-tile-4", false), Some(4));
        assert_eq!(click(false, "mapchoice-row-5", false), Some(5));
        // on a server the map is the server's
        assert_eq!(click(true, "mapchoice-tile-4", true), None);
        assert_eq!(click(false, "mapchoice-row-5", true), None);
    }

    #[test]
    fn the_tiles_lie_four_to_a_row_and_stay_apart() {
        let items = some_maps();
        let mut ui = Ui::new();
        frame(&mut ui, true, &items, false);
        let rects: Vec<Rect> = (0..items.len()).map(|k| *ui.drawn.get(&id_of(&format!("mapchoice-tile-{k}"))).unwrap()).collect();
        assert_eq!(rects[0].y, rects[3].y);
        assert!(rects[4].y > rects[3].bottom());
        for k in 1..4 {
            assert!(rects[k].x >= rects[k - 1].right() + TILE_GAP - 0.5);
        }
    }

    #[test]
    fn the_switch_says_which_view_was_clicked() {
        let mut ui = Ui::new();
        let r = Rect::new(10.0, 10.0, 156.0, 32.0);
        let run = |ui: &mut Ui| {
            ui.begin(Vec2::new(400.0, 200.0), 1.0, 1.0 / 60.0);
            view_switch(ui, r, true)
        };
        run(&mut ui);
        let list = *ui.drawn.get(&id_of("mapchoice-view-0")).unwrap();
        ui.input.mouse = list.center();
        ui.input.pressed = true;
        ui.input.down = true;
        run(&mut ui);
        ui.input.pressed = false;
        ui.input.down = false;
        ui.input.released = true;
        assert_eq!(run(&mut ui), Some(false));
        // the view it is on already is no change
        let tiles = *ui.drawn.get(&id_of("mapchoice-view-1")).unwrap();
        ui.input.released = false;
        run(&mut ui);
        ui.input.mouse = tiles.center();
        ui.input.pressed = true;
        ui.input.down = true;
        run(&mut ui);
        ui.input.pressed = false;
        ui.input.down = false;
        ui.input.released = true;
        assert_eq!(run(&mut ui), None);
    }
}
