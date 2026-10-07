//! The bus's depot files (`.hof`), after Omsi-Hub's (its yard tiles on the bus step and
//! `HofDialog.tsx`): which of them the bus has and what each is for - the map it belongs to,
//! how many destinations it knows, whether it is the chosen map's - with the one in use marked
//! and the one for the chosen map first, and a tile that gives the bus the map's depot file
//! when it lacks it (`omsi_launcher_lib::depot`, which puts the copy into openOMSI's own
//! content folder). A bus without the chosen map's depot file is always asked about: a dialog
//! over the step says so, offers the bus's own files that know the map best, and the map's
//! file to add.
//!
//! What the game does with a bus that lacks the file is said as it is: `situation::find_hof`
//! takes the bus's own depot file of the same place first, then borrows the map's from another
//! vehicle folder (`depot_anywhere`), then the bus's first. The analysis follows that order, so
//! the tile marked "in use" is the file the drive will have.

use super::buspick::{fold, name_cmp};
use super::theme::*;
use super::ui::{ease_in_out_cubic, ease_out_cubic, id_of, ButtonKind, Key, Ui};
use super::Launcher;
use glam::Vec2;
use omsi_launcher_lib as core;
use omsi_launcher_lib::depot::{Refusal, Source};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

// --- what a bus's depot files are for -------------------------------------------------------

/// One depot file of the bus, as its tile shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub path: PathBuf,
    pub file: String,
    /// Its `[name]`, or its file name without `.hof` when it has none.
    pub name: String,
    /// What the game is given (`--hof`) to take this one.
    pub key: String,
    /// How many destinations it has (see [`destinations`]).
    pub destinations: usize,
    /// How many of the chosen map's destinations it knows; None when the map's depot file was
    /// not found to compare with.
    pub known: Option<usize>,
    /// The map it belongs to, as far as can be told.
    pub map: Option<String>,
    /// It is the chosen map's depot file.
    pub maps: bool,
    /// openOMSI added it (`depot::added`).
    pub added: bool,
}

/// What a drive takes when the bus is given the map's depot file and lacks it.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Fallback {
    /// One of its own (the one of the same place, else its first).
    Own(PathBuf),
    /// The map's, from another vehicle folder (this one).
    Borrowed(String),
    #[default]
    Nothing,
}

/// The depot file a drive with the bus has.
#[derive(Clone, Debug, PartialEq)]
pub enum Use {
    Row(usize),
    Borrowed(String),
    Nothing,
}

/// How a depot file stands to the chosen map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fit {
    /// It is the map's own.
    Maps,
    /// It knows (nearly) every destination of the map.
    Fits,
    /// It knows some of them (known, of all).
    Partly(usize, usize),
    /// It knows none of them.
    Other,
    /// There is nothing to compare it with.
    Unknown,
}

/// The bus's depot files, worked out for a bus on a map.
#[derive(Clone, Debug, Default)]
pub struct Menu {
    /// bus|map|depot file|content generation it was worked out for.
    pub key: String,
    pub bus: String,
    /// Its depot files, the one for the chosen map first.
    pub rows: Vec<Row>,
    /// The chosen map's depot file ("" for a map that names none) and the map's name.
    pub want: String,
    pub map: String,
    /// How many destinations the map's depot file has (None: no copy found to count).
    pub total: Option<usize>,
    /// The copy that would be added.
    pub source: Option<Source>,
    /// Its `[name]` as the bus list reads it, and how many destinations it has.
    pub source_name: String,
    pub source_destinations: usize,
    /// Why nothing can be added (the bus has it, there is no copy, a file is in the way).
    pub refusal: Option<Refusal>,
    /// Where a copy would go.
    pub target: Option<PathBuf>,
    pub fallback: Fallback,
    /// The tile for the chosen map (the first): None when none fits it.
    pub recommended: Option<usize>,
    /// The player's own depot files for the map (`owndepot`): any bus is given the one chosen.
    pub own: Vec<core::owndepot::Entry>,
}

impl Menu {
    /// The bus lacks the chosen map's depot file.
    pub fn lacks(&self) -> bool {
        !self.want.trim().is_empty() && !self.rows.iter().any(|r| r.maps)
    }

    /// The depot file a drive takes when the game is given `chosen` (`--hof`).
    pub fn used(&self, chosen: &str) -> Use {
        if let Some(i) = row_of(&self.rows, chosen) {
            return Use::Row(i);
        }
        match &self.fallback {
            Fallback::Own(p) => self.rows.iter().position(|r| &r.path == p).map(Use::Row).unwrap_or(Use::Nothing),
            Fallback::Borrowed(f) => Use::Borrowed(f.clone()),
            Fallback::Nothing => Use::Nothing,
        }
    }
}

/// A depot file's destinations to compare: the idents of its termini other than the
/// "everybody out" ones (a service trip, which every depot file has), folded.
pub fn destinations(h: &omsi_vehicle::Hof) -> HashSet<String> {
    h.termini.iter().filter(|t| !t.all_exit).map(|t| fold(&t.texture_id)).filter(|s| !s.is_empty()).collect()
}

/// What the game is given (`--hof`) to take each of the files named `names` (their `[name]`
/// or file name) with the file names `stems`: the name, unless another has the same one - then
/// the file name, which the game looks at first (`depot_in`).
pub fn keys_for(names: &[String], stems: &[String]) -> Vec<String> {
    names.iter().zip(stems).map(|(n, s)| if names.iter().filter(|o| o.trim().eq_ignore_ascii_case(n.trim())).count() > 1 { s.clone() } else { n.clone() }).collect()
}

/// The row of the depot file `chosen` (`--hof`): by what the game is given, its name or its
/// file name.
pub fn row_of(rows: &[Row], chosen: &str) -> Option<usize> {
    let c = chosen.trim();
    if c.is_empty() {
        return None;
    }
    let stem = |r: &Row| Path::new(&r.file).file_stem().map(|s| s.to_string_lossy().trim().to_string()).unwrap_or_default();
    rows.iter().position(|r| r.key.trim().eq_ignore_ascii_case(c)).or_else(|| rows.iter().position(|r| stem(r).eq_ignore_ascii_case(c))).or_else(|| rows.iter().position(|r| r.name.trim().eq_ignore_ascii_case(c)))
}

/// How a depot file stands to the chosen map, whose depot file has `total` destinations.
pub fn fit_of(row: &Row, total: Option<usize>) -> Fit {
    if row.maps {
        return Fit::Maps;
    }
    match (row.known, total) {
        (Some(k), Some(t)) if t > 0 && k * 10 >= t * 9 => Fit::Fits,
        (Some(k), Some(t)) if k > 0 => Fit::Partly(k, t),
        (Some(_), Some(t)) if t > 0 => Fit::Other,
        _ => Fit::Unknown,
    }
}

/// The map a depot file belongs to, as far as can be told: a map whose ailists name it, else
/// the map whose name shares the most words of a place with it. `maps` are each map's name
/// and depot file.
pub fn map_of(name: &str, stem: &str, maps: &[(String, String)]) -> Option<String> {
    let says = |h: &str| !h.trim().is_empty() && (h.trim().eq_ignore_ascii_case(name.trim()) || h.trim().eq_ignore_ascii_case(stem.trim()));
    if let Some((label, _)) = maps.iter().find(|(_, h)| says(h)) {
        return Some(label.clone());
    }
    let labels: Vec<&str> = maps.iter().map(|(l, _)| l.as_str()).collect();
    omsi_vehicle::hof::closest_name(&labels, &[name, stem]).map(|i| maps[i].0.clone())
}

/// The row of `rows` that belongs to the place `hints` name (`depot_like`: by its file name
/// and its name).
fn like(rows: &[Row], hints: &[&str]) -> Option<usize> {
    let labels: Vec<String> = rows.iter().map(|r| format!("{} {}", Path::new(&r.file).file_stem().unwrap_or_default().to_string_lossy(), r.name)).collect();
    let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    omsi_vehicle::hof::closest_name(&refs, hints)
}

/// What a drive takes when the bus (its depot files `rows`, in the game's order: by file name)
/// lacks the map's `want`: its own of the same place, else the map's borrowed from the folder
/// `borrowed`, else its own named like the map (`map_hints`), else its first
/// (`situation::find_hof`).
pub fn fallback_of(rows: &[Row], want: &str, map_hints: &[&str], borrowed: Option<String>) -> Fallback {
    if let Some(i) = like(rows, &[want]) {
        return Fallback::Own(rows[i].path.clone());
    }
    if let Some(f) = borrowed {
        return Fallback::Borrowed(f);
    }
    if let Some(i) = like(rows, map_hints) {
        return Fallback::Own(rows[i].path.clone());
    }
    rows.first().map(|r| Fallback::Own(r.path.clone())).unwrap_or_default()
}

/// The row to recommend for the chosen map (whose depot file has `total` destinations): its own
/// depot file, else the one that knows most of its destinations (a quarter at least), else the
/// one of the same place (`like`).
pub fn recommend(rows: &[Row], like: Option<usize>, total: Option<usize>) -> Option<usize> {
    if let Some(i) = rows.iter().position(|r| r.maps) {
        return Some(i);
    }
    let enough = |k: usize| k > 0 && total.is_none_or(|t| k * 4 >= t);
    let best = rows.iter().enumerate().filter(|(_, r)| r.known.is_some_and(enough)).max_by(|(a, x), (b, y)| x.known.cmp(&y.known).then(b.cmp(a)));
    best.map(|(i, _)| i).or(like)
}

/// The tiles' order: the recommended one first, then those that know most of the map's
/// destinations, the rest by name.
pub fn order(mut rows: Vec<Row>, first: Option<usize>) -> Vec<Row> {
    let head = first.filter(|i| *i < rows.len()).map(|i| rows.remove(i));
    rows.sort_by(|a, b| b.maps.cmp(&a.maps).then(b.known.unwrap_or(0).cmp(&a.known.unwrap_or(0))).then_with(|| name_cmp(&a.name, &b.name)));
    head.into_iter().chain(rows).collect()
}

/// The bus's own depot files the dialog offers instead of the map's: those that know a good
/// part of its destinations (a quarter at least - one of ninety-one is no choice), the best
/// first, three at most; without the map's file to compare with, the recommended one.
pub fn choices(m: &Menu) -> Vec<usize> {
    let enough = |k: usize| k > 0 && m.total.is_none_or(|t| k * 4 >= t);
    let mut v: Vec<usize> = (0..m.rows.len()).filter(|i| m.rows[*i].known.is_some_and(enough)).collect();
    v.sort_by(|a, b| m.rows[*b].known.cmp(&m.rows[*a].known).then(a.cmp(b)));
    if v.is_empty() && m.total.is_none() {
        v.extend(m.recommended);
    }
    v.truncate(3);
    v
}

/// A place to show in a line: from the content folder's own name on (`openOMSI\Vehicles\Bus`),
/// not the whole way from the drive.
pub fn short_place(p: &Path) -> String {
    let parts: Vec<String> = p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    match parts.iter().rposition(|c| c.eq_ignore_ascii_case("Vehicles")) {
        Some(i) if i > 0 => parts[i - 1..].join(std::path::MAIN_SEPARATOR_STR),
        _ => p.display().to_string(),
    }
}

/// The dialog comes over the step: the bus lacks the map's depot file, it was not answered for
/// this map and bus, nothing else is asked (the bus offered for the duty), and the player is
/// not choosing a bus himself (he said he would, and has not opened one yet).
pub fn should_ask(lacks: bool, answered: bool, offering: bool, deferred: bool) -> bool {
    lacks && !answered && !offering && !deferred
}

/// The tiles in a row `w` wide: (columns, tile width).
pub fn card_layout(w: f32) -> (usize, f32) {
    let n = (((w + CARD_GAP) / (CARD_W + CARD_GAP)).floor() as usize).max(1);
    (n, ((w - CARD_GAP * (n as f32 - 1.0)) / n as f32).max(120.0))
}

const CARD_W: f32 = 300.0;
const CARD_H: f32 = 128.0;
const CARD_GAP: f32 = 16.0;

// --- working it out (on a thread: depot files are read, some are megabytes) -----------------

struct Job {
    key: String,
    root: PathBuf,
    bus: String,
    map: String,
    want: String,
    map_label: String,
    map_hints: Vec<String>,
    /// Every map's name and depot file.
    maps: Vec<(String, String)>,
}

fn same(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().replace('\\', "/").eq_ignore_ascii_case(&b.to_string_lossy().replace('\\', "/"))
}

fn analyse(j: Job, cache: &Mutex<HashMap<String, Arc<Vec<Source>>>>) -> Menu {
    let t0 = std::time::Instant::now();
    // the bus's depot files as the bus list and the bus picker read them: its folder's, else
    // its pack's
    let bus_path = omsi_cfg::resolve_path(&j.root, &j.bus);
    let dir = bus_path.parent().map(Path::to_path_buf).unwrap_or_else(|| j.root.clone());
    let pack = j.bus.replace('\\', "/").split('/').take(2).collect::<Vec<_>>().join("/");
    // (the player's own given to it are tiles of their own: `Menu::own`)
    let theirs = |d: &Path| omsi_vehicle::hof::depot_files(d).into_iter().filter(|f| !omsi_vehicle::hof::is_players(f)).collect::<Vec<_>>();
    let mut files = theirs(&dir);
    if files.is_empty() && !pack.is_empty() {
        files = theirs(&omsi_cfg::resolve_path(&j.root, &pack));
    }
    let added = core::depot::added();
    let hofs: Vec<omsi_vehicle::Hof> = files.iter().map(|f| omsi_vehicle::Hof::load(f).unwrap_or_else(|_| omsi_vehicle::Hof { path: f.clone(), ..Default::default() })).collect();
    let stems: Vec<String> = files.iter().map(|f| f.file_stem().unwrap_or_default().to_string_lossy().trim().to_string()).collect();
    let names: Vec<String> = hofs.iter().zip(&stems).map(|(h, s)| if h.name.trim().is_empty() { s.clone() } else { h.name.trim().to_string() }).collect();
    let keys = keys_for(&names, &stems);
    let dests: Vec<HashSet<String>> = hofs.iter().map(destinations).collect();
    let maps_flags: Vec<bool> = files.iter().map(|f| core::depot::answers_to(f, &j.want)).collect();
    let own = maps_flags.iter().position(|m| *m);
    // the map's depot file elsewhere on this computer: the copy to add, and what to compare with
    let sources: Arc<Vec<Source>> = if j.want.trim().is_empty() {
        Arc::default()
    } else {
        let k = format!("{}|{}|{}", j.want.trim().to_lowercase(), j.map.to_lowercase(), omsi_cfg::content_generation());
        let hit = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&k).cloned();
        hit.unwrap_or_else(|| {
            let s = Arc::new(core::depot::find_sources(&j.want, &j.map));
            cache.lock().unwrap_or_else(|e| e.into_inner()).insert(k, s.clone());
            s
        })
    };
    let others: Vec<Source> = sources.iter().filter(|s| !files.iter().any(|f| same(f, &s.path))).cloned().collect();
    let source = core::depot::best(&others).map(|i| others[i].clone());
    let source_hof = source.as_ref().and_then(|s| omsi_vehicle::Hof::load(&s.path).ok());
    let reference: Option<HashSet<String>> = own.map(|i| dests[i].clone()).or_else(|| source_hof.as_ref().map(destinations));
    let rows: Vec<Row> = (0..files.len())
        .map(|i| Row {
            path: files[i].clone(),
            file: files[i].file_name().unwrap_or_default().to_string_lossy().into_owned(),
            name: names[i].clone(),
            key: keys[i].clone(),
            destinations: dests[i].len(),
            known: reference.as_ref().map(|r| dests[i].intersection(r).count()),
            map: if maps_flags[i] { Some(j.map_label.clone()).filter(|m| !m.is_empty()) } else { map_of(&names[i], &stems[i], &j.maps) },
            maps: maps_flags[i],
            added: added.iter().any(|a| same(a, &files[i])),
        })
        .collect();
    let lacks = !j.want.trim().is_empty() && own.is_none();
    let hints: Vec<&str> = j.map_hints.iter().map(String::as_str).filter(|h| !h.trim().is_empty()).collect();
    // (`depot_anywhere` goes through the vehicle folders by name and takes the first that has it)
    let borrowed = if lacks {
        let mut v: Vec<&Source> = others.iter().filter(|s| !s.from_map).collect();
        v.sort_by_key(|s| s.folder.to_lowercase());
        v.first().map(|s| s.folder.clone())
    } else {
        None
    };
    let fallback = if lacks { fallback_of(&rows, &j.want, &hints, borrowed) } else { Fallback::Nothing };
    let same_place = if j.want.trim().is_empty() { like(&rows, &hints) } else { like(&rows, &[j.want.as_str()]).or_else(|| like(&rows, &hints)) };
    let first = recommend(&rows, same_place, reference.as_ref().map(|r| r.len()));
    let rows = order(rows, first);
    let target = core::depot::target_for(&j.bus);
    let existing: Vec<(String, String)> = rows.iter().map(|r| (r.file.clone(), r.name.clone())).collect();
    let refusal = if j.want.trim().is_empty() {
        None
    } else if let Some(r) = rows.iter().find(|r| r.maps) {
        Some(Refusal::Has(r.file.clone()))
    } else {
        match &source {
            None => Some(Refusal::NoSource),
            Some(_) if target.is_none() => Some(Refusal::NoFolder),
            Some(s) => core::depot::check(&s.file, &j.want, &existing).err(),
        }
    };
    let source_name = source.as_ref().and_then(|s| omsi_vehicle::Hof::read_name(&s.path)).unwrap_or_default().trim().to_string();
    log::info!(
        "depot files: {} has {} ({}), the map's {:?} {} ({:.2} s)",
        j.bus,
        rows.len(),
        rows.iter().map(|r| r.file.as_str()).collect::<Vec<_>>().join(", "),
        j.want,
        match (&source, lacks) {
            (_, false) => "beside it".to_string(),
            (Some(s), true) => format!("missing, a copy in {}", s.path.display()),
            (None, true) => "missing, no copy found".to_string(),
        },
        t0.elapsed().as_secs_f32()
    );
    Menu {
        key: j.key,
        bus: j.bus,
        total: reference.as_ref().map(|r| r.len()),
        source_destinations: source_hof.as_ref().map(|h| destinations(h).len()).unwrap_or(0),
        source,
        source_name,
        refusal,
        target,
        fallback,
        recommended: first.map(|_| 0),
        own: core::owndepot::for_map(&core::owndepot::list(&core::owndepot::dir()), &core::lines::map_folder(&j.map)),
        want: j.want,
        map: j.map_label,
        rows,
    }
}

// --- the view -------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// Asked on the way: the bus lacks the map's depot file.
    Missing,
    /// From the tile: adding it, said in full before it is done.
    Add,
}

#[derive(Clone, Debug)]
struct Dialog {
    mode: Mode,
    /// `ui.time` it opened (its way in).
    at: f32,
    /// map|bus|depot file it is about.
    about: String,
}

#[derive(Default)]
pub struct DepotView {
    /// The depot files are shown in the wide sheet (the bus step's level after the bus).
    pub open: bool,
    /// The player said he would pick a bus himself: the dialog waits until one is in view.
    pub deferred: bool,
    menu: Option<Arc<Menu>>,
    asked: String,
    rx: Option<Receiver<Menu>>,
    sources: Arc<Mutex<HashMap<String, Arc<Vec<Source>>>>>,
    /// The chosen map's depot file and what it was worked out for (map, date, content
    /// generation): reading the ailists every frame would cost.
    want: String,
    want_for: (String, String, u64),
    dialog: Option<Dialog>,
    /// map|bus|depot file of the dialogs answered: not asked again this session.
    answered: HashSet<String>,
    seen: f32,
    /// What the last adding came to (shown over the tiles a while), an error or not, and when.
    note: Option<(String, bool, f32)>,
}

/// The menu worked out for the bus, map and depot file chosen now, if it is ready - also one
/// of before the content changed, while it is worked out again (the tiles and the dialog do not
/// blink away for it).
fn current(l: &Launcher) -> Option<Arc<Menu>> {
    let v = &l.buspick.depots;
    let what = v.asked.rsplit_once('|').map(|(w, _)| w).unwrap_or("");
    v.menu.clone().filter(|m| m.key.rsplit_once('|').is_some_and(|(w, _)| w == what))
}

fn map_label(m: &core::MapInfo) -> String {
    if m.friendly.trim().is_empty() { m.name.trim().to_string() } else { m.friendly.trim().to_string() }
}

/// Once a frame on the bus step: the depot files worked out again when the bus, the map or
/// the content changed, and the dialog opened for a bus that lacks the map's.
pub(super) fn frame(l: &mut Launcher, offering: bool) {
    let now = l.ui.time;
    let v = &mut l.buspick.depots;
    if now - v.seen > 0.2 {
        v.deferred = false;
    }
    v.seen = now;
    let Some(bus) = l.state.bus().map(|b| b.file.clone()) else { return };
    let map = l.state.choice.map.clone();
    if map.is_empty() {
        return;
    }
    let generation = omsi_cfg::content_generation();
    let wk = (map.clone(), l.state.choice.date.clone(), generation);
    if l.buspick.depots.want_for != wk {
        let w = l.state.map_hof();
        let v = &mut l.buspick.depots;
        v.want = w;
        v.want_for = wk;
    }
    let want = l.buspick.depots.want.clone();
    let key = format!("{bus}|{map}|{want}|{generation}");
    if l.buspick.depots.asked != key {
        let m = l.state.map();
        let job = Job {
            key: key.clone(),
            root: PathBuf::from(&l.state.config.root),
            bus: bus.clone(),
            map: map.clone(),
            want: want.clone(),
            map_label: m.map(map_label).unwrap_or_default(),
            map_hints: m.map(|m| vec![m.name.clone(), m.friendly.clone(), m.file.trim_end_matches("/global.cfg").rsplit('/').next().unwrap_or("").to_string()]).unwrap_or_default(),
            maps: l.state.maps.iter().map(|m| (map_label(m), m.hof.clone())).collect(),
        };
        let v = &mut l.buspick.depots;
        let cache = v.sources.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let k = job.key.clone();
            // (an odd file of some mod that panics the reading: the tiles say there is nothing
            // rather than "reading" for ever)
            let m = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| analyse(job, &cache))).unwrap_or_else(|_| Menu { key: k, ..Default::default() });
            let _ = tx.send(m);
        });
        v.rx = Some(rx);
        v.asked = key;
    }
    let v = &mut l.buspick.depots;
    if let Some(Ok(m)) = v.rx.as_ref().map(|rx| rx.try_recv()) {
        if m.key == v.asked {
            v.menu = Some(Arc::new(m));
        }
        v.rx = None;
    }
    let Some(m) = current(l) else { return };
    let about = format!("{map}|{bus}|{}", m.want.trim().to_lowercase());
    // (a depot file of the player's own is given to the bus: nothing to ask)
    let lacks = m.lacks() && own_in_use(l, &m).is_none();
    let v = &mut l.buspick.depots;
    match &v.dialog {
        None if should_ask(lacks, v.answered.contains(&about), offering, v.deferred) => {
            log::info!("depot files: {bus} lacks {:?}, the depot file of the map - asked", m.want);
            v.dialog = Some(Dialog { mode: Mode::Missing, at: now, about });
        }
        // (the bus or the map changed under it)
        Some(d) if d.about != about || (d.mode == Mode::Missing && !m.lacks()) => v.dialog = None,
        _ => {}
    }
}

/// A dialog of the depot files lies over the step.
pub(super) fn asking(l: &Launcher) -> bool {
    l.buspick.depots.dialog.is_some()
}

/// What adding came to: on the status line, and over the tiles for a while.
fn note(l: &mut Launcher, text: String, err: bool) {
    l.state.set_status(text.clone(), err);
    l.buspick.depots.note = Some((text, err, l.ui.time));
}

/// Make `row` the depot file of the drive: "automatic" (following the map and the date) when
/// it is the one openOMSI would take anyway, the player's own choice otherwise.
fn select(l: &mut Launcher, row: &Row) {
    let auto = l.state.default_hof();
    let is_auto = row.key.trim().eq_ignore_ascii_case(auto.trim()) || row.name.trim().eq_ignore_ascii_case(auto.trim());
    l.state.choice.hof_manual = !is_auto;
    l.state.choice.hof = if is_auto { auto } else { row.key.clone() };
    l.state.touched();
}

/// The player's own depot file of `m` the drive gives the bus (`State::own_depot_in_use`): its
/// place among `m.own`, and whether the driven line chose it.
fn own_in_use(l: &Launcher, m: &Menu) -> Option<(usize, bool)> {
    let keys: Vec<String> = m.own.iter().map(|e| e.key.clone()).collect();
    let (key, by_line) = l.state.own_depot_in_use(&keys)?;
    keys.iter().position(|k| *k == key).map(|i| (i, by_line))
}

/// Make the player's own depot file `key` the drive's: given to the bus, whichever it is.
fn select_own(l: &mut Launcher, key: &str) {
    l.state.choice.hof_manual = true;
    l.state.choice.hof = key.to_string();
    l.state.touched();
}

fn refusal_text(r: &Refusal) -> String {
    match r {
        Refusal::Has(f) => omsi_ui::tr("This bus has it already: %{file}").replace("%{file}", f),
        Refusal::Taken(f) => omsi_ui::tr("A file called %{file} lies beside this bus already, and is never written over").replace("%{file}", f),
        Refusal::NoSource => omsi_ui::tr("No copy of it on this computer").into_owned(),
        Refusal::NoFolder => omsi_ui::tr("openOMSI has no folder of its own to put it in").into_owned(),
        Refusal::Original => omsi_ui::tr("That would be in the OMSI 2 folder, which openOMSI does not write to").into_owned(),
        Refusal::Unreadable(w) => omsi_ui::tr("The copy found cannot be read: %{why}").replace("%{why}", w),
        Refusal::Failed(w) => omsi_ui::tr("Writing it failed: %{why}").replace("%{why}", w),
    }
}

/// Give the bus the map's depot file (see `depot::add_for_bus`). Returns whether it was added.
fn add(l: &mut Launcher, m: &Menu) -> bool {
    let Some(src) = m.source.clone() else { return false };
    let bus_name = l.state.bus().map(|b| omsi_launcher_lib::display_bus_name(&b.name)).unwrap_or_default();
    match core::depot::add_for_bus(&m.bus, &m.want, &src.path) {
        Ok(p) => {
            log::info!("depot files: {} added for {}", p.display(), m.bus);
            // the bus's list has it at once (the lists are read again when the poll sees the
            // folder change); every version of the pack that reads the same folder gets it
            if let Some(cur) = l.state.bus().cloned() {
                for v in l.state.vehicles.iter_mut().filter(|v| v.folder.eq_ignore_ascii_case(&cur.folder) && v.hofs == cur.hofs) {
                    if !v.hofs.iter().any(|h| h.eq_ignore_ascii_case(&m.source_name)) {
                        v.hofs.push(m.source_name.clone());
                    }
                }
            }
            // (the map's depot file is what the player wanted: the drive takes it)
            l.state.choice.hof_manual = false;
            l.state.choice.hof = l.state.default_hof();
            l.state.touched();
            note(l, omsi_ui::tr("%{file} added beside %{bus}").replace("%{file}", &src.file).replace("%{bus}", &bus_name), false);
            true
        }
        Err(e) => {
            note(l, omsi_ui::tr("Not added: %{why}").replace("%{why}", &refusal_text(&e)), true);
            false
        }
    }
}

// --- the dialog -----------------------------------------------------------------------------

/// What the dialog says and offers.
#[derive(Clone, Debug, Default)]
pub struct Ask {
    pub title: String,
    pub body: String,
    /// What the drive takes without it.
    pub now: Option<String>,
    /// What would be added, or why nothing can be.
    pub offer: Option<String>,
    pub offer_warn: bool,
    /// The bus's own depot files to take instead: name and what it knows.
    pub choices: Vec<(String, String)>,
    /// Where the copy goes.
    pub note: Option<String>,
    pub place: Option<String>,
    /// The add button's word; None when nothing can be added (the main button opens the
    /// depot files then).
    pub add: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Answer {
    NotNow,
    Choose(usize),
    Add,
    Open,
}

const DIALOG_IN: f32 = 0.32;
const GLOW_S: f32 = 1.4;

/// The dialog over the step, `t` (0..1) of its way in, `glow` (0..1, or past it) of the light
/// crossing its top edge once. Returns what was answered.
pub fn dialog_ui(ui: &mut Ui, a: &Ask, t: f32, glow: f32) -> Option<Answer> {
    let size = ui.size;
    let full = Rect::new(0.0, 0.0, size.x, size.y);
    ui.solid(full);
    let e = ease_out_cubic(t);
    ui.p().rect(full, Color::rgba(4, 6, 14, 0.62 * e));
    let w = (size.x - 32.0).min(540.0);
    let pad = if w < 400.0 { 20.0 } else { 28.0 };
    let tw = w - 2.0 * pad;
    let title_h = ui.paragraph_height(&a.title, tw, 21.0, Weight::Bold);
    let body_h = ui.paragraph_height(&a.body, tw, 13.5, Weight::Regular);
    let now_h = a.now.as_ref().map(|s| ui.paragraph_height(s, tw - 24.0, 12.5, Weight::Regular) + 14.0).unwrap_or(0.0);
    let offer_h = a.offer.as_ref().map(|s| ui.paragraph_height(s, tw, 13.5, Weight::Medium) + 16.0).unwrap_or(0.0);
    let choices_h = if a.choices.is_empty() { 0.0 } else { 30.0 + a.choices.len() as f32 * 50.0 + 6.0 };
    let note_h = a.note.as_ref().map(|s| ui.paragraph_height(s, tw - 24.0, 12.0, Weight::Regular) + 10.0).unwrap_or(0.0) + if a.place.is_some() { 18.0 } else { 0.0 };
    let h = (26.0 + title_h + 12.0 + body_h + 12.0 + now_h + offer_h + choices_h + note_h + 18.0 + 44.0 + 24.0).min(size.y - 24.0);
    // (it comes up a little as the ground darkens; without animations it is there at once)
    let r = Rect::new((size.x - w) * 0.5, ((size.y - h) * 0.5).max(12.0) + 22.0 * (1.0 - e), w, h);
    ui.p().shadow(r.inset(-2.0), SHEET_RADIUS, 30.0, Color::rgba(0, 0, 0, 0.5 * e));
    ui.panel(r);
    // a light along the top edge, once, as Omsi-Hub's dialogs have it
    if (0.0..1.0).contains(&glow) {
        ui.keep_moving();
        let k = ease_in_out_cubic(glow);
        let bw = r.w * 0.34;
        let x = r.x - bw + (r.w + bw) * k;
        ui.push_clip(r, SHEET_RADIUS);
        ui.p().gradient_h(Rect::new(x, r.y, bw * 0.5, 3.0), accent().alpha(0.0), accent());
        ui.p().gradient_h(Rect::new(x + bw * 0.5, r.y, bw * 0.5, 3.0), accent(), accent().alpha(0.0));
        ui.pop_clip();
    }
    let x = r.x + pad;
    let mut y = r.y + 26.0;
    y += ui.paragraph(&a.title, Vec2::new(x, y), tw, 21.0, Weight::Bold, TEXT) + 12.0;
    y += ui.paragraph(&a.body, Vec2::new(x, y), tw, 13.5, Weight::Regular, TEXT_SOFT) + 12.0;
    if let Some(s) = &a.now {
        ui.icon("info", Vec2::new(x + 8.0, y + 8.0), 15.0, TEXT_DIM);
        y += ui.paragraph(s, Vec2::new(x + 24.0, y), tw - 24.0, 12.5, Weight::Regular, TEXT_DIM) + 14.0;
    }
    if let Some(s) = &a.offer {
        y += ui.paragraph(s, Vec2::new(x, y), tw, 13.5, Weight::Medium, if a.offer_warn { WARN } else { TEXT }) + 16.0;
    }
    let mut answer = None;
    if !a.choices.is_empty() {
        ui.text(&omsi_ui::tr("Or take one of its own").to_uppercase(), Vec2::new(x, y + 14.0), 11.0, Weight::Bold, TEXT_DIM, Align::Left);
        y += 30.0;
        for (k, (name, sub)) in a.choices.iter().enumerate() {
            let row = Rect::new(x - 8.0, y, tw + 16.0, 46.0);
            if ui.row(&format!("hof-ask-choice-{k}"), row, false) {
                answer = Some(Answer::Choose(k));
            }
            ui.icon("garage", Vec2::new(row.x + 22.0, row.center().y), 18.0, TEXT_SOFT);
            ui.text_in(name, Rect::new(row.x + 44.0, row.y + 5.0, row.w - 80.0, 20.0), 13.5, Weight::Bold, TEXT, Align::Left);
            ui.text_in(sub, Rect::new(row.x + 44.0, row.y + 24.0, row.w - 80.0, 17.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
            ui.icon("chevron_right", Vec2::new(row.right() - 18.0, row.center().y), 16.0, TEXT_FAINT);
            y += 50.0;
        }
        y += 6.0;
    }
    if let Some(s) = &a.note {
        ui.icon("folder_open", Vec2::new(x + 8.0, y + 8.0), 14.0, TEXT_FAINT);
        y += ui.paragraph(s, Vec2::new(x + 24.0, y), tw - 24.0, 12.0, Weight::Regular, TEXT_FAINT) + 4.0;
        if let Some(p) = &a.place {
            ui.text_in(p, Rect::new(x + 24.0, y, tw - 24.0, 16.0), 11.0, Weight::Regular, TEXT_FAINT, Align::Left);
        }
    }
    // the buttons: not now, and adding it (or the depot files, when nothing can be added)
    let by = r.bottom() - 24.0 - 44.0;
    let later = omsi_ui::tr("Not now");
    let lw = ui.width(&later, 14.5, Weight::Bold) + 40.0;
    let mut main = a.add.clone().unwrap_or_else(|| omsi_ui::tr("Choose a depot file").into_owned());
    // (the whole words where they fit, "Add it" in a narrow window)
    if a.add.is_some() && ui.width(&main, 14.5, Weight::Bold) + 70.0 + 12.0 + lw > tw {
        main = omsi_ui::tr("Add it").into_owned();
    }
    let mw = ui.width(&main, 14.5, Weight::Bold) + 70.0;
    let main_r = Rect::new(r.right() - pad - mw, by, mw, 44.0);
    let later_r = Rect::new(main_r.x - 12.0 - lw, by, lw, 44.0);
    let keys = ui.input.keys.clone();
    if ui.button("hof-ask-later", later_r, &later, None, ButtonKind::Normal) || keys.contains(&Key::Escape) {
        answer = Some(Answer::NotNow);
    }
    let icon = if a.add.is_some() { "add" } else { "garage" };
    if ui.button("hof-ask-main", main_r, &main, Some(icon), ButtonKind::Primary) || keys.contains(&Key::Enter) {
        answer = Some(if a.add.is_some() { Answer::Add } else { Answer::Open });
    }
    answer
}

/// The words of the dialog for the menu `m`, the bus called `bus`, the drive taking `used`.
fn ask_of(m: &Menu, mode: Mode, bus: &str, used: &Use) -> (Ask, Vec<usize>) {
    let tr = |s: &str| omsi_ui::tr(s).into_owned();
    let (title, body) = match mode {
        Mode::Missing => (tr("This bus does not know this map"), tr("%{bus} does not carry %{hof}, the depot file of %{map}. A bus's IBIS and destination displays know only the codes and destinations of the depot files beside it.").replace("%{bus}", bus).replace("%{hof}", &m.want).replace("%{map}", &m.map)),
        Mode::Add => (tr("Add the map's depot file"), tr("%{hof} is the depot file of %{map}: with it beside the bus, its IBIS and destination displays know the map's codes and destinations.").replace("%{hof}", &m.want).replace("%{map}", &m.map)),
    };
    let now = match used {
        Use::Borrowed(f) => tr("For this drive openOMSI borrows it from %{folder}, which carries it. Beside this bus it is the bus's own, in its list of depot files.").replace("%{folder}", f),
        Use::Row(i) => {
            let r = &m.rows[*i];
            match (r.known, m.total) {
                (Some(k), Some(t)) => tr("Without it the bus takes %{file}, which knows %{known} of the %{total} destinations here.").replace("%{file}", &r.name).replace("%{known}", &k.to_string()).replace("%{total}", &t.to_string()),
                _ => tr("Without it the bus takes %{file}.").replace("%{file}", &r.name),
            }
        }
        Use::Nothing => tr("Without it the IBIS takes none of the map's codes, and the destination displays stay blank."),
    };
    let (offer, warn, add) = match (&m.source, &m.refusal) {
        (Some(s), None) => {
            let text = if s.from_map { tr("Add %{file} beside this bus? It came with the map and knows %{n} destinations.") } else { tr("Add %{file} beside this bus? It is the copy %{folder} carries, and knows %{n} destinations.") };
            (text.replace("%{file}", &s.file).replace("%{folder}", &s.folder).replace("%{n}", &m.source_destinations.to_string()), false, Some(tr("Add the map's depot file")))
        }
        (_, Some(Refusal::NoSource)) => (tr("No copy of %{hof} was found on this computer: no other bus carries it, and the map's folder has none.").replace("%{hof}", &m.want), true, None),
        (_, Some(r)) => (refusal_text(r), true, None),
        (None, None) => (tr("No copy of %{hof} was found on this computer: no other bus carries it, and the map's folder has none.").replace("%{hof}", &m.want), true, None),
    };
    let picks = if mode == Mode::Missing { choices(m) } else { Vec::new() };
    let choices: Vec<(String, String)> = picks.iter().map(|i| (m.rows[*i].name.clone(), fit_text(fit_of(&m.rows[*i], m.total), m.rows[*i].destinations).0)).collect();
    let note = add.is_some().then(|| tr("The copy goes into openOMSI's own folder, beside this bus. Nothing in OMSI 2 is changed or written over."));
    let place = add.is_some().then(|| m.target.as_deref().map(short_place)).flatten();
    (Ask { title, body, now: Some(now), offer: Some(offer), offer_warn: warn, choices, note, place, add }, picks)
}

/// The dialog over the step, drawn last. Returns true when the depot files are to be shown.
pub(super) fn dialog(l: &mut Launcher) -> bool {
    let Some(d) = l.buspick.depots.dialog.clone() else { return false };
    let Some(m) = current(l) else { return false };
    let bus = l.state.bus().map(|b| omsi_launcher_lib::display_bus_name(&b.name)).unwrap_or_default();
    let used = m.used(&l.state.choice.hof);
    let (a, picks) = ask_of(&m, d.mode, &bus, &used);
    let age = l.ui.time - d.at;
    let motion = l.ui.motion;
    let t = if motion { (age / DIALOG_IN).clamp(0.0, 1.0) } else { 1.0 };
    if t < 1.0 {
        l.ui.keep_moving();
    }
    let glow = if motion { age / GLOW_S } else { 1.0 };
    let answer = dialog_ui(&mut l.ui, &a, t, glow);
    let mut open = false;
    let v = &mut l.buspick.depots;
    match answer {
        Some(Answer::NotNow) => {
            v.answered.insert(d.about);
            v.dialog = None;
        }
        Some(Answer::Choose(k)) => {
            v.answered.insert(d.about);
            v.dialog = None;
            if let Some(row) = picks.get(k).and_then(|i| m.rows.get(*i)).cloned() {
                select(l, &row);
            }
        }
        Some(Answer::Add) => {
            if add(l, &m) {
                let v = &mut l.buspick.depots;
                v.answered.insert(d.about);
                v.dialog = None;
            }
        }
        Some(Answer::Open) => {
            v.answered.insert(d.about);
            v.dialog = None;
            open = true;
        }
        None => {}
    }
    open
}

// --- the field on the bus's sheet -----------------------------------------------------------

/// What the drive takes, in words for a field: its name, and how it stands to the map (the
/// words and their colour, and an icon).
fn standing(m: Option<&Menu>, chosen: &str) -> (String, String, Color, &'static str) {
    let Some(m) = m else { return (chosen.to_string(), omsi_ui::tr("Reading the depot files…").into_owned(), TEXT_DIM, "schedule") };
    match m.used(chosen) {
        Use::Row(i) => {
            let r = &m.rows[i];
            let fit = fit_of(r, m.total);
            let (text, c) = fit_text(fit, r.destinations);
            // (on the bus's own sheet a file of another map is a warning: the drive takes it)
            let (c, icon) = match fit {
                Fit::Maps | Fit::Fits => (OK, "check_circle"),
                Fit::Partly(..) | Fit::Other => (WARN, "warning"),
                Fit::Unknown => (c, "info"),
            };
            (r.name.clone(), text, c, icon)
        }
        Use::Borrowed(f) => (m.want.clone(), omsi_ui::tr("Borrowed from %{folder} for this drive").replace("%{folder}", &f), WARN, "warning"),
        Use::Nothing => (omsi_ui::tr("None").into_owned(), omsi_ui::tr("This bus has no depot file").into_owned(), WARN, "warning"),
    }
}

/// The words of a fit, and their colour: green for the map's, amber for a file that knows a
/// good part of it (a choice with a cost), quiet for the files of other maps - they are not
/// wrong, only not for here.
fn fit_text(f: Fit, destinations: usize) -> (String, Color) {
    match f {
        Fit::Maps => (omsi_ui::tr("This map's depot file").into_owned(), OK),
        Fit::Fits => (omsi_ui::tr("Fits this map").into_owned(), OK),
        Fit::Partly(k, t) => (omsi_ui::tr("Knows %{known} of %{total} destinations here").replace("%{known}", &k.to_string()).replace("%{total}", &t.to_string()), if k * 4 >= t { WARN } else { TEXT_DIM }),
        Fit::Other => (omsi_ui::tr("None of this map's destinations").into_owned(), TEXT_DIM),
        Fit::Unknown => (count_text(destinations), TEXT_DIM),
    }
}

fn count_text(n: usize) -> String {
    if n == 1 {
        omsi_ui::tr("one destination").into_owned()
    } else {
        omsi_ui::tr("%{n} destinations").replace("%{n}", &n.to_string())
    }
}

/// On the bus's sheet beside the showroom: the depot file the drive takes, as a field that
/// opens the depot files, and under it how it stands to the map. Returns the height used and
/// whether it was clicked.
pub(super) fn field(l: &mut Launcher, x: f32, y: f32, w: f32) -> (f32, bool) {
    let m = current(l);
    let chosen = l.state.choice.hof.clone();
    let own = m.as_deref().and_then(|m| own_in_use(l, m).map(|(i, _)| m.own[i].name.clone()));
    let (name, text, c, icon) = match own {
        Some(name) => (name, omsi_ui::tr("Your depot file: given to this bus when you drive").into_owned(), OK, "check_circle"),
        None => standing(m.as_deref(), &chosen),
    };
    crate::mt::protect([name.as_str()]);
    l.ui.label(Rect::new(x, y, 110.0, ROW), "Depot file");
    let f = Rect::new(x + 116.0, y, w - 116.0, ROW);
    let id = id_of("hof-field");
    let (h, _, clicked) = l.ui.interact(id, f);
    let t = l.ui.anim(id, if h { 1.0 } else { 0.0 }, 0.08);
    l.ui.p().rounded(f, RADIUS, FIELD.mix(HOVER, t));
    l.ui.p().rounded_border(f, RADIUS, 1.0, EDGE.mix(Color::WHITE.alpha(0.16), t));
    l.ui.icon("garage", Vec2::new(f.x + 18.0, f.center().y), 16.0, TEXT_SOFT);
    l.ui.text_in(&name, Rect::new(f.x + 34.0, f.y, f.w - 62.0, f.h), 13.0, Weight::Medium, TEXT, Align::Left);
    // (the chevron steps the way it goes under the mouse)
    l.ui.icon("chevron_right", Vec2::new(f.right() - 16.0 + 2.5 * t, f.center().y), 16.0, if h { TEXT } else { TEXT_DIM });
    l.ui.tooltip(f, "The depot files of this bus: which one the drive takes, and adding the map's");
    let sy = y + ROW + 4.0;
    l.ui.icon(icon, Vec2::new(f.x + 8.0, sy + 9.0), 13.0, c);
    l.ui.text_in(&text, Rect::new(f.x + 20.0, sy, f.w - 20.0, 18.0), 11.5, Weight::Medium, c, Align::Left);
    (ROW + 26.0, clicked)
}

// --- the tiles ------------------------------------------------------------------------------

/// One tile of the depot files.
#[derive(Clone, Debug, Default)]
struct Card {
    key: String,
    title: String,
    file: String,
    line: String,
    tag: Option<(String, Color)>,
    extra: Option<String>,
    chosen: bool,
    icon: &'static str,
    enabled: bool,
    /// The tile that adds one: an outline, not a surface.
    add: bool,
}

/// One tile. Returns whether it was clicked.
fn draw_card(ui: &mut Ui, base: Rect, c: &Card) -> bool {
    let id = id_of(&format!("hof-card-{}", c.key));
    let m = if c.enabled { ui.tile(id, base, SHEET_RADIUS) } else { ui.card_tile(id, base, SHEET_RADIUS) };
    let r = m.r;
    let k = if c.enabled { m.hover } else { 0.0 };
    // (the one in use takes the route's blue over a moment when another is chosen: the mark
    // moves from tile to tile rather than jumping)
    let s = ui.anim(id ^ 0x5e1e_c7ed, if c.chosen { 1.0 } else { 0.0 }, 0.12);
    let rest = if c.add { accent().alpha(0.05 + 0.07 * k) } else { FIELD.mix(HOVER, k) };
    let fill = rest.mix(accent(), s);
    ui.tile_shadow(&m);
    if s > 0.01 {
        ui.p().shadow(r.inset(-3.0), SHEET_RADIUS + 3.0, 18.0, accent().alpha(0.4 * s));
    }
    ui.p().rounded(r, SHEET_RADIUS, fill);
    if c.enabled {
        ui.tile_light(&m, 0.10);
    }
    if s > 0.5 {
        ui.p().rounded_border(r, SHEET_RADIUS, 2.0, accent());
    } else if c.add {
        ui.p().rounded_border(r, SHEET_RADIUS, 1.0, accent().alpha(if c.enabled { 0.45 + 0.4 * k } else { 0.2 }));
    } else {
        ui.tile_edge(&m, 1.0, EDGE);
    }
    let dim = !c.enabled && !c.chosen;
    let ink = if dim { TEXT_DIM } else { TEXT }.mix(on_accent(), s);
    let soft = if dim { TEXT_FAINT } else { TEXT_SOFT }.mix(on_accent().alpha(0.85), s);
    let faint = TEXT_FAINT.mix(on_accent().alpha(0.7), s);
    let ib = Rect::new(r.x + 16.0, r.y + 16.0, 40.0, 40.0);
    let box_c = if c.add { accent().alpha(if dim { 0.08 } else { 0.18 }) } else { Color::WHITE.alpha(0.06) };
    ui.p().rounded(ib, RADIUS, box_c.mix(Color::WHITE.alpha(0.18), s));
    ui.icon(c.icon, ib.center(), 22.0, if c.add && !dim { accent_2() } else { soft }.mix(on_accent(), s));
    let tx = ib.right() + 14.0;
    let right = base.x + base.w - 16.0 + (r.x - base.x);
    // in use: a mark in the corner
    let mut title_w = (right - tx).max(0.0);
    if c.chosen {
        let word = omsi_ui::tr("In use").into_owned();
        let ww = ui.width(&word, 11.0, Weight::Bold);
        let pill = Rect::new(right - ww - 30.0, r.y + 16.0, ww + 30.0, 22.0);
        ui.p().rounded(pill, 11.0, Color::WHITE.alpha(0.18));
        ui.icon("check", Vec2::new(pill.x + 12.0, pill.center().y), 14.0, on_accent());
        ui.text_in(&word, Rect::new(pill.x + 22.0, pill.y, ww + 4.0, pill.h), 11.0, Weight::Bold, on_accent(), Align::Left);
        title_w = (pill.x - 8.0 - tx).max(0.0);
    }
    ui.text_in(&c.title, Rect::new(tx, r.y + 15.0, title_w, 22.0), 15.0, Weight::Bold, ink, Align::Left);
    ui.text_in(&c.file, Rect::new(tx, r.y + 37.0, (right - tx).max(0.0), 18.0), 11.5, Weight::Regular, faint, Align::Left);
    ui.text_in(&c.line, Rect::new(r.x + 16.0, r.y + 68.0, (right - r.x - 16.0).max(0.0), 18.0), 12.5, Weight::Regular, soft, Align::Left);
    let mut x = r.x + 16.0;
    let ty = r.y + 94.0;
    if let Some((t, col)) = &c.tag {
        let col = if c.chosen { on_accent() } else if dim { TEXT_DIM } else { *col };
        let w = ui.width(t, 11.0, Weight::Bold) + 16.0;
        let pill = Rect::new(x, ty, w.min((right - x).max(0.0)), 20.0);
        ui.p().rounded(pill, 6.0, col.alpha(if c.chosen { 0.2 } else { 0.14 }));
        ui.text_in(t, pill.pad(8.0, 0.0), 11.0, Weight::Bold, col, Align::Left);
        x = pill.right() + 10.0;
    }
    if let Some(e) = &c.extra {
        ui.text_in(e, Rect::new(x, ty, (right - x).max(0.0), 20.0), 11.5, Weight::Regular, faint, Align::Left);
    }
    m.clicked && c.enabled
}

/// What a click on the tiles asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pick {
    Row(usize),
    /// Adding the map's depot file (said in full first).
    Add,
    /// One of the player's own depot files (`Menu::own`).
    Own(usize),
}

/// The tiles of the menu `m`, with the drive taking `used` (`auto`: openOMSI chose it) - or
/// `own`, one of the player's own depot files (and whether the driven line chose it): given to
/// the bus, it is what the drive takes.
fn cards_of(m: &Menu, used: &Use, auto: bool, own: Option<(usize, bool)>) -> Vec<(Card, Option<Pick>)> {
    let tr = |s: &str| omsi_ui::tr(s).into_owned();
    let mut out = Vec::new();
    // the player's own, first: any bus is given the one chosen
    for (k, e) in m.own.iter().enumerate() {
        let (chosen, by_line) = match own {
            Some((i, line)) if i == k => (true, line),
            _ => (false, false),
        };
        let specials: Vec<String> = e.specials.iter().take(4).map(|(c, n)| format!("{c} {n}")).collect();
        let more = if e.specials.len() > 4 { " …" } else { "" };
        out.push((
            Card {
                key: format!("own-{}", e.key.to_lowercase()),
                title: tr("Your depot file '%{name}'").replace("%{name}", &e.name),
                file: format!("{}.hof", e.key),
                line: format!("{} · {}", if chosen { tr("Given to this bus when you drive") } else { tr("Given to this bus when chosen") }, count_text(e.termini.len())),
                tag: Some((if by_line { tr("The line chose it") } else { tr("Yours") }, accent_2())),
                extra: (!specials.is_empty()).then(|| tr("Special trips: %{list}").replace("%{list}", &(specials.join(", ") + more))),
                chosen,
                icon: "departure_board",
                enabled: true,
                add: false,
            },
            Some(Pick::Own(k)),
        ));
    }
    // the map's depot file borrowed from another bus: it is what the drive takes
    if let Use::Borrowed(f) = used {
        out.push((
            Card {
                key: "borrowed".into(),
                title: m.want.clone(),
                file: tr("from %{folder}").replace("%{folder}", f),
                line: match m.total {
                    Some(t) => format!("{} · {}", m.map, count_text(t)),
                    None => m.map.clone(),
                },
                tag: Some((tr("Borrowed for this drive"), WARN)),
                extra: None,
                chosen: own.is_none(),
                icon: "garage",
                enabled: m.source.is_some() && m.refusal.is_none(),
                add: false,
            },
            Some(Pick::Add),
        ));
    }
    for (i, r) in m.rows.iter().enumerate() {
        let (tag, col) = fit_text(fit_of(r, m.total), r.destinations);
        let line = match &r.map {
            Some(map) => format!("{map} · {}", count_text(r.destinations)),
            None => count_text(r.destinations),
        };
        let chosen = own.is_none() && *used == Use::Row(i);
        let mut extra = Vec::new();
        if m.recommended == Some(i) && !r.maps {
            extra.push(tr("best for this map"));
        }
        if chosen && auto {
            extra.push(tr("automatic"));
        }
        if r.added {
            extra.push(tr("added by openOMSI"));
        }
        // (a fit that only counts destinations would say the line above again)
        let tag = (fit_of(r, m.total) != Fit::Unknown).then_some((tag, col));
        out.push((Card { key: r.file.to_lowercase(), title: r.name.clone(), file: r.file.clone(), line, tag, extra: (!extra.is_empty()).then(|| extra.join(" · ")), chosen, icon: "garage", enabled: true, add: false }, Some(Pick::Row(i))));
    }
    // and one added - always there when the map names a depot file: when nothing can be added it
    // says why (Omsi-Hub: a tile that is sometimes there and sometimes not is worse)
    if !m.want.trim().is_empty() {
        let (file, line, enabled, tag) = match (&m.source, &m.refusal) {
            (Some(s), None) => (
                if s.from_map { format!("{} · {}", s.file, tr("from the map's folder")) } else { format!("{} · {}", s.file, tr("from %{folder}").replace("%{folder}", &s.folder)) },
                format!("{} · {}", m.want, count_text(m.source_destinations)),
                true,
                None,
            ),
            (_, Some(Refusal::Has(f))) => (f.clone(), tr("This bus has the depot file of this map"), false, Some((tr("Nothing to add"), TEXT_DIM))),
            (_, Some(r)) => (m.want.clone(), refusal_text(r), false, None),
            (None, None) => (m.want.clone(), refusal_text(&Refusal::NoSource), false, None),
        };
        out.push((Card { key: "add".into(), title: tr("Add the map's depot file"), file, line, tag, extra: None, chosen: false, icon: "add", enabled, add: true }, enabled.then_some(Pick::Add)));
    }
    out
}

/// The depot files as tiles in the wide sheet (`r`): over them a word when the bus lacks the
/// map's file (or what adding it came to), and the tiles - the one for the map first, the one
/// in use marked, the one that adds last.
pub(super) fn cards(l: &mut Launcher, r: Rect) {
    let Some(m) = current(l) else {
        l.ui.text_in("Reading the depot files…", Rect::new(r.x, r.y + 30.0, r.w, 24.0), 13.5, Weight::Regular, TEXT_DIM, Align::Center);
        return;
    };
    let used = m.used(&l.state.choice.hof);
    let auto = !l.state.choice.hof_manual;
    let own = own_in_use(l, &m);
    let mut top = r.y;
    // a word over the tiles: what adding came to (a while), else that the map's file is missing
    let now = l.ui.time;
    let note = l.buspick.depots.note.clone().filter(|(_, _, at)| now - *at < 8.0);
    let strip = Rect::new(r.x, top, r.w - 8.0, 44.0);
    let mut open_add = false;
    if let Some((text, err, _)) = note {
        let c = if err { WARN } else { OK };
        l.ui.p().rounded(strip, RADIUS, c.alpha(0.10));
        l.ui.p().rounded_border(strip, RADIUS, 1.0, c.alpha(0.3));
        l.ui.icon(if err { "warning" } else { "check_circle" }, Vec2::new(strip.x + 22.0, strip.center().y), 17.0, c);
        l.ui.text_in(&text, Rect::new(strip.x + 42.0, strip.y, strip.w - 54.0, strip.h), 13.0, Weight::Medium, TEXT, Align::Left);
        top = strip.bottom() + 14.0;
    } else if let Some((i, by_line)) = own {
        // the player's own depot file: what the bus is given, said over the tiles
        l.ui.p().rounded(strip, RADIUS, OK.alpha(0.08));
        l.ui.p().rounded_border(strip, RADIUS, 1.0, OK.alpha(0.3));
        l.ui.icon("check_circle", Vec2::new(strip.x + 22.0, strip.center().y), 17.0, OK);
        let says = if by_line { omsi_ui::tr("Your line chose your depot file %{name}: it is given to this bus when you drive.") } else { omsi_ui::tr("Your depot file %{name} is given to this bus when you drive.") };
        let text = says.replace("%{name}", &m.own[i].name);
        crate::mt::protect([text.as_str()]);
        l.ui.text_in(&text, Rect::new(strip.x + 42.0, strip.y, strip.w - 54.0, strip.h), 13.0, Weight::Medium, TEXT, Align::Left);
        top = strip.bottom() + 14.0;
    } else if m.lacks() {
        l.ui.p().rounded(strip, RADIUS, WARN.alpha(0.08));
        l.ui.p().rounded_border(strip, RADIUS, 1.0, WARN.alpha(0.3));
        l.ui.icon("warning", Vec2::new(strip.x + 22.0, strip.center().y), 17.0, WARN);
        let can = m.source.is_some() && m.refusal.is_none();
        let word = omsi_ui::tr("Add it").into_owned();
        let bw = l.ui.width(&word, 13.0, Weight::Bold) + 52.0;
        let text = omsi_ui::tr("This bus does not carry %{hof}, the depot file of this map.").replace("%{hof}", &m.want);
        l.ui.text_in(&text, Rect::new(strip.x + 42.0, strip.y, strip.w - 54.0 - if can { bw + 12.0 } else { 0.0 }, strip.h), 13.0, Weight::Medium, TEXT, Align::Left);
        if can && l.ui.button("hof-strip-add", Rect::new(strip.right() - bw - 6.0, strip.y + 6.0, bw, strip.h - 12.0), &word, Some("add"), ButtonKind::Primary) {
            open_add = true;
        }
        top = strip.bottom() + 14.0;
    }
    let list = cards_of(&m, &used, auto, own);
    let area = Rect::new(r.x - 4.0, top, r.w + 8.0, (r.bottom() - top).max(60.0));
    if m.rows.is_empty() {
        l.ui.text_in("This bus has no depot files of its own.", Rect::new(area.x + 4.0, area.y + 4.0, area.w, 20.0), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
    }
    let shift = if m.rows.is_empty() { 30.0 } else { 0.0 };
    let (n, cw) = card_layout(area.w - 16.0);
    let mut picked = None;
    l.ui.scroll_area("hof-grid", Rect::new(area.x, area.y + shift, area.w, area.h - shift), &mut |ui, v| {
        for (k, (card, pick)) in list.iter().enumerate() {
            let base = Rect::new(v.x + 4.0 + (k % n) as f32 * (cw + CARD_GAP), v.y + 6.0 + (k / n) as f32 * (CARD_H + CARD_GAP), cw, CARD_H);
            if !ui.rect_visible(base) {
                continue;
            }
            if draw_card(ui, base, card) {
                picked = *pick;
            }
        }
        list.len().div_ceil(n) as f32 * (CARD_H + CARD_GAP) + 10.0
    });
    match picked {
        Some(Pick::Row(i)) => {
            if let Some(row) = m.rows.get(i).cloned() {
                select(l, &row);
            }
        }
        Some(Pick::Add) => open_add = true,
        Some(Pick::Own(k)) => {
            if let Some(e) = m.own.get(k) {
                select_own(l, &e.key);
            }
        }
        None => {}
    }
    if open_add && m.source.is_some() && m.refusal.is_none() {
        let about = format!("{}|{}|{}", l.state.choice.map, m.bus, m.want.trim().to_lowercase());
        l.buspick.depots.dialog = Some(Dialog { mode: Mode::Add, at: l.ui.time, about });
    }
}

/// The sheet's title and the line under it, on the depot files.
pub(super) fn heading() -> (String, String) {
    (omsi_ui::tr("Depot file").into_owned(), omsi_ui::tr("The depot file decides which destinations the IBIS and the displays know.").into_owned())
}

/// Over the tiles on the right, the map's depot file.
pub(super) fn map_line(l: &Launcher) -> Option<String> {
    let m = current(l)?;
    Some(if m.want.trim().is_empty() {
        omsi_ui::tr("%{map} names no depot file of its own").replace("%{map}", &m.map)
    } else {
        omsi_ui::tr("The depot file of %{map}: %{hof}").replace("%{map}", &m.map).replace("%{hof}", &m.want)
    })
}

/// The sheet's foot on the depot files: how many, and which one the drive takes.
pub(super) fn foot(l: &Launcher) -> String {
    let Some(m) = current(l) else { return omsi_ui::tr("Reading the depot files…").into_owned() };
    let n = if m.rows.len() == 1 { omsi_ui::tr("one depot file beside this bus").into_owned() } else { omsi_ui::tr("%{n} depot files beside this bus").replace("%{n}", &m.rows.len().to_string()) };
    if let Some((i, by_line)) = own_in_use(l, &m) {
        let how = if by_line { omsi_ui::tr("your line's choice") } else { omsi_ui::tr("your choice") };
        return format!("{n} · {} ({how})", omsi_ui::tr("In use: %{hof}").replace("%{hof}", &m.own[i].name));
    }
    let in_use = match m.used(&l.state.choice.hof) {
        Use::Row(i) => m.rows[i].name.clone(),
        Use::Borrowed(_) => m.want.clone(),
        Use::Nothing => return n,
    };
    let how = if l.state.choice.hof_manual { omsi_ui::tr("your choice") } else { omsi_ui::tr("automatic") };
    format!("{n} · {} ({how})", omsi_ui::tr("In use: %{hof}").replace("%{hof}", &in_use))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(file: &str, name: &str, known: Option<usize>, maps: bool) -> Row {
        Row { path: PathBuf::from(format!("Vehicles/Bus/{file}")), file: file.into(), name: name.into(), key: name.into(), destinations: 10, known, map: None, maps, added: false }
    }

    fn menu(rows: Vec<Row>, want: &str, total: Option<usize>) -> Menu {
        Menu { key: "k".into(), bus: "Vehicles/Bus/bus.bus".into(), rows, want: want.into(), map: "Hamburg".into(), total, ..Default::default() }
    }

    #[test]
    fn the_destinations_are_the_termini_but_the_service_trip() {
        let text = "stringcount_terminus\r\n1\r\n\r\n[addterminus_list]\r\n{ALLEX}\t13\tBetriebsfahrt\tBETRIEBSFAHRT\r\n\t282\tU Ruhleben\tRUHLEBEN\r\n\t283\tRathaus Spandau\tRATHAUS\r\n\t284\tu ruhleben\tRUHLEBEN\r\n[end]\r\n";
        let h = omsi_vehicle::Hof::parse(&omsi_cfg::CfgFile::from_str("x.hof", text));
        let d = destinations(&h);
        assert_eq!(d.len(), 2, "{d:?}");
        assert!(d.contains("uruhleben") && d.contains("rathausspandau"));
    }

    #[test]
    fn two_files_of_one_name_are_given_by_their_file_name() {
        let names = vec!["Hamburg".to_string(), "hamburg".to_string(), "Grundorf".to_string()];
        let stems = vec!["HH_2014".to_string(), "HH_2022".to_string(), "Grundorf".to_string()];
        assert_eq!(keys_for(&names, &stems), vec!["HH_2014", "HH_2022", "Grundorf"]);
    }

    #[test]
    fn the_file_in_use_is_found_by_what_the_game_is_given() {
        let mut rows = vec![row("HH_2014.hof", "Hamburg", None, false), row("Grundorf.hof", "Grundorf", None, false)];
        rows[0].key = "HH_2014".into();
        assert_eq!(row_of(&rows, "hh_2014"), Some(0));
        assert_eq!(row_of(&rows, "Hamburg"), Some(0), "by its name too");
        assert_eq!(row_of(&rows, " grundorf "), Some(1));
        assert_eq!(row_of(&rows, "Spandau"), None);
        assert_eq!(row_of(&rows, ""), None);
    }

    #[test]
    fn a_file_fits_by_how_many_of_the_maps_destinations_it_knows() {
        assert_eq!(fit_of(&row("a.hof", "A", Some(3), true), Some(40)), Fit::Maps);
        assert_eq!(fit_of(&row("a.hof", "A", Some(37), false), Some(40)), Fit::Fits);
        assert_eq!(fit_of(&row("a.hof", "A", Some(12), false), Some(40)), Fit::Partly(12, 40));
        assert_eq!(fit_of(&row("a.hof", "A", Some(0), false), Some(40)), Fit::Other);
        assert_eq!(fit_of(&row("a.hof", "A", None, false), None), Fit::Unknown);
    }

    #[test]
    fn a_file_belongs_to_the_map_that_names_it_or_shares_its_place() {
        let maps = vec![("Grundorf".to_string(), "Grundorf".to_string()), ("Berlin-Spandau".to_string(), "Spandau 1986".to_string()), ("Hamburg Linie 20".to_string(), String::new())];
        assert_eq!(map_of("Spandau 1986", "Spandau_86", &maps).as_deref(), Some("Berlin-Spandau"));
        assert_eq!(map_of("Spandau 2019", "SP19", &maps).as_deref(), Some("Berlin-Spandau"), "by its place");
        assert_eq!(map_of("HH20", "Hamburg_Linie_20_2022", &maps).as_deref(), Some("Hamburg Linie 20"));
        assert_eq!(map_of("Ahlheim", "Ahlheim", &maps), None);
    }

    #[test]
    fn a_bus_without_the_maps_file_takes_its_own_of_the_place_else_borrows_it() {
        let rows = vec![row("Grundorf.hof", "Grundorf", None, false), row("HH_2014.hof", "Hamburg Linie 20 (2014)", None, false)];
        assert_eq!(fallback_of(&rows, "Hamburg Linie 20 (2022)", &["Hamburg"], Some("MAN_NL".into())), Fallback::Own(rows[1].path.clone()));
        assert_eq!(fallback_of(&rows, "Spandau 1986", &["Berlin-Spandau"], Some("SD200".into())), Fallback::Borrowed("SD200".into()));
        assert_eq!(fallback_of(&rows, "Spandau 1986", &["Grundorf"], None), Fallback::Own(rows[0].path.clone()), "named like the map");
        assert_eq!(fallback_of(&rows, "Spandau 1986", &["Ahlheim"], None), Fallback::Own(rows[0].path.clone()), "its first");
        assert_eq!(fallback_of(&[], "Spandau 1986", &[], None), Fallback::Nothing);
        let mut m = menu(rows, "Spandau 1986", None);
        m.fallback = Fallback::Borrowed("SD200".into());
        assert_eq!(m.used("Spandau 1986"), Use::Borrowed("SD200".into()));
        assert_eq!(m.used("Grundorf"), Use::Row(0), "a file of its own chosen is the one taken");
    }

    #[test]
    fn the_file_for_the_map_comes_first() {
        let rows = vec![row("A.hof", "Alpha", Some(2), false), row("B.hof", "Bravo", Some(9), false), row("C.hof", "Charlie", Some(40), true), row("D.hof", "Delta", Some(0), false)];
        assert_eq!(recommend(&rows, None, Some(40)), Some(2), "the map's own");
        let ordered = order(rows.clone(), Some(2));
        assert_eq!(ordered.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Charlie", "Bravo", "Alpha", "Delta"]);
        let without: Vec<Row> = rows.into_iter().filter(|r| !r.maps).collect();
        assert_eq!(recommend(&without, Some(2), Some(20)), Some(1), "the one that knows most");
        assert_eq!(recommend(&without, Some(2), Some(91)), Some(2), "nine of ninety-one is too few: the one of the same place");
        let none = vec![row("A.hof", "Alpha", Some(0), false), row("B.hof", "Bravo", None, false)];
        assert_eq!(recommend(&none, Some(1), None), Some(1), "the one of the same place");
        assert_eq!(recommend(&none, None, None), None);
    }

    #[test]
    fn the_dialog_offers_the_buses_own_that_know_the_map_best() {
        let rows = vec![row("A.hof", "A", Some(12), false), row("B.hof", "B", Some(0), false), row("C.hof", "C", Some(30), false), row("D.hof", "D", Some(15), false), row("E.hof", "E", Some(10), false), row("F.hof", "F", Some(1), false)];
        let m = menu(rows, "Hamburg", Some(40));
        assert!(m.lacks());
        assert_eq!(choices(&m), vec![2, 3, 0], "the best three of a quarter or more");
        let mut none = menu(vec![row("A.hof", "A", None, false)], "Hamburg", None);
        none.recommended = Some(0);
        assert_eq!(choices(&none), vec![0]);
        let mut few = menu(vec![row("A.hof", "A", Some(1), false)], "Hamburg", Some(91));
        few.recommended = Some(0);
        assert!(choices(&few).is_empty(), "one of ninety-one is no choice");
        assert_eq!(short_place(Path::new("C:/Games/openOMSI/Vehicles/MAN_NL")), ["openOMSI", "Vehicles", "MAN_NL"].join(std::path::MAIN_SEPARATOR_STR));
        assert!(!menu(vec![row("A.hof", "A", None, true)], "Hamburg", None).lacks());
        assert!(!menu(vec![], "", None).lacks(), "a map that names no depot file lacks nothing");
    }

    #[test]
    fn a_bus_without_the_maps_file_is_always_asked_about_once() {
        assert!(should_ask(true, false, false, false));
        assert!(!should_ask(false, false, false, false), "it has it");
        assert!(!should_ask(true, true, false, false), "answered for this map and bus");
        assert!(!should_ask(true, false, true, false), "the bus offered is asked first");
        assert!(!should_ask(true, false, false, true), "the player picks a bus himself");
    }

    #[test]
    fn the_tiles_mark_the_one_in_use_and_end_with_adding() {
        let mut m = menu(vec![row("A.hof", "Alpha", Some(30), false), row("B.hof", "Bravo", Some(0), false)], "Hamburg", Some(40));
        m.recommended = Some(0);
        m.source = Some(Source { path: PathBuf::from("Vehicles/O530/HH.hof"), file: "HH.hof".into(), folder: "O530".into(), from_map: false, size: 1 });
        let list = cards_of(&m, &Use::Row(0), true, None);
        assert_eq!(list.len(), 3);
        assert!(list[0].0.chosen && !list[1].0.chosen);
        assert_eq!(list[2].1, Some(Pick::Add));
        assert!(list[2].0.add && list[2].0.enabled);
        // borrowed: a tile of its own, first and in use; nothing to add when it is refused
        m.refusal = Some(Refusal::Taken("HH.hof".into()));
        let list = cards_of(&m, &Use::Borrowed("O530".into()), true, None);
        assert_eq!(list.len(), 4);
        assert!(list[0].0.chosen && !list[0].0.enabled);
        assert_eq!(list[3].1, None, "the adding tile says why, and does nothing");
        // a map that names no depot file: no adding tile
        let plain = menu(vec![row("A.hof", "Alpha", None, false)], "", None);
        assert_eq!(cards_of(&plain, &Use::Row(0), false, None).len(), 1);
    }

    /// The player's own depot files are tiles of their own, first; the one the drive gives the
    /// bus is the one in use, and says so.
    #[test]
    fn the_players_own_depot_files_are_offered_for_any_bus() {
        let mut m = menu(vec![row("A.hof", "Alpha", Some(30), true)], "Alpha", Some(30));
        let own = |key: &str, name: &str| core::owndepot::Entry { key: key.into(), name: name.into(), specials: vec![(13, "Betriebsfahrt".into()), (900, "Sonderfahrt".into())], ..Default::default() };
        m.own = vec![own("oo_Mine", "Mine"), own("oo_Specials", "Specials")];
        let list = cards_of(&m, &Use::Row(0), true, Some((1, true)));
        assert_eq!(list.len(), 4, "and the tile that adds the map's");
        assert_eq!((list[0].1, list[1].1, list[2].1), (Some(Pick::Own(0)), Some(Pick::Own(1)), Some(Pick::Row(0))));
        assert!(!list[0].0.chosen && list[1].0.chosen && !list[2].0.chosen, "the bus's own is not what the drive takes");
        assert!(list[1].0.title.contains("Specials") && list[1].0.line.starts_with("Given to this bus when you drive"));
        assert_eq!(list[1].0.tag.as_ref().map(|t| t.0.as_str()), Some("The line chose it"));
        assert!(list[1].0.extra.as_deref().is_some_and(|e| e.contains("900 Sonderfahrt")));
        // none chosen: the bus's file is in use again
        assert!(cards_of(&m, &Use::Row(0), true, None)[2].0.chosen);
    }

    #[test]
    fn the_tiles_lay_out_in_columns() {
        assert_eq!(card_layout(1000.0).0, 3);
        assert_eq!(card_layout(300.0).0, 1);
        let (n, w) = card_layout(1400.0);
        assert!((n as f32 * w + (n as f32 - 1.0) * CARD_GAP - 1400.0).abs() < 0.5);
    }

    fn frame_dialog(ui: &mut Ui, a: &Ask) -> Option<Answer> {
        ui.begin(Vec2::new(1440.0, 900.0), 1.0, 1.0 / 60.0);
        dialog_ui(ui, a, 1.0, 1.0)
    }

    fn click(ui: &mut Ui, name: &str, a: &Ask) -> Option<Answer> {
        frame_dialog(ui, a);
        let r = *ui.drawn.get(&id_of(name)).expect("drawn");
        ui.input.mouse = r.center();
        ui.input.pressed = true;
        ui.input.down = true;
        frame_dialog(ui, a);
        ui.input.pressed = false;
        ui.input.down = false;
        ui.input.released = true;
        let answer = frame_dialog(ui, a);
        ui.input.released = false;
        answer
    }

    #[test]
    fn the_dialog_adds_chooses_or_waits() {
        let a = Ask { title: "T".into(), body: "B".into(), choices: vec![("Alpha".into(), "Knows 30 of 40".into()), ("Bravo".into(), "x".into())], add: Some("Add it".into()), ..Default::default() };
        let mut ui = Ui::new();
        assert_eq!(click(&mut ui, "hof-ask-main", &a), Some(Answer::Add));
        assert_eq!(click(&mut ui, "hof-ask-later", &a), Some(Answer::NotNow));
        assert_eq!(click(&mut ui, "hof-ask-choice-1", &a), Some(Answer::Choose(1)));
        // nothing to add: the main button shows the depot files
        let none = Ask { add: None, ..a.clone() };
        assert_eq!(click(&mut ui, "hof-ask-main", &none), Some(Answer::Open));
        // Escape is "not now"
        ui.begin(Vec2::new(1440.0, 900.0), 1.0, 1.0 / 60.0);
        ui.input.keys.push(Key::Escape);
        assert_eq!(dialog_ui(&mut ui, &a, 1.0, 1.0), Some(Answer::NotNow));
    }

    #[test]
    fn the_dialog_fits_a_small_window() {
        let a = Ask { title: "This bus does not know this map".into(), body: "x ".repeat(200), now: Some("y ".repeat(80)), offer: Some("z ".repeat(60)), choices: vec![("A".into(), "a".into()); 3], note: Some("n".into()), add: Some("Add the depot file of this map to this bus".into()), ..Default::default() };
        let mut ui = Ui::new();
        ui.begin(Vec2::new(420.0, 700.0), 1.0, 1.0 / 60.0);
        dialog_ui(&mut ui, &a, 1.0, 1.0);
        let (main, later) = (ui.drawn[&id_of("hof-ask-main")], ui.drawn[&id_of("hof-ask-later")]);
        assert!(main.x >= 0.0 && main.right() <= 420.0 && main.bottom() <= 700.0, "{main:?}");
        assert!(later.x >= 0.0, "a long word gives way to a short one: {later:?}");
    }
}
