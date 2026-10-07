//! The bus step's picker, after Omsi-Hub's: the buses as tiles in a wide sheet - the makers,
//! then a maker's models, then a model's versions - each tile with a photo of the bus drawn by
//! the game's own renderer (see `busphoto`), a breadcrumb back up, a search and the starred
//! buses alone. A version chosen, the sheet makes way for the bus itself: it stands on the
//! ground, turned by the mouse (see `showroom`), with its livery, depot file, fleet number and
//! plate on a narrow sheet beside it; its depot file opens the bus's depot files as tiles in the
//! wide sheet, and a bus without the map's one is asked about (see `hof`).
//!
//! On the way in the bus that fits the duty best is offered in a dialog, as Omsi-Hub offers it:
//! the one whose depot files know the duty's trips, of the map's own fleet before any other (a
//! bus the map's depot runs is of the place and the time). It is said what openOMSI would take,
//! and the choice is the player's. A free drive has no trips to fit: there the bus the map's
//! depot runs most is marked, and nothing is asked.
//!
//! A player's line that asks for buses of its own (the line editor's Kind tab: kinds of
//! bus, makers, models) has them marked "this line" and put first, a maker's or a model's tile
//! with one of them in it too; the bus offered is one of them (`mark_line`).
//!
//! OMSI keeps the maker in `[friendlyname]`'s first line and the model and version together in
//! its second, " - " between them ("Gelenkbus - 18C - 3 Tuerer"): the first part is the model,
//! the rest the version.

use super::busphoto;
use super::theme::*;
use super::ui::{id_of, ButtonKind, Key, Ui};
use super::Launcher;
use glam::Vec2;
use omsi_launcher_lib::{display_bus_name, vehicle_type_label, VehicleInfo};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

/// One bus file as the picker shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bus {
    pub file: String,
    pub group: String,
    pub model: String,
    pub version: String,
    /// Its maker and type as OMSI names it ("MAN NL202 - 2 Tuerer").
    pub name: String,
    /// Its liveries, its own among them.
    pub liveries: usize,
    pub fresh: bool,
    pub installed: bool,
    pub incomplete: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub name: String,
    /// Its buses (places in `Tree::buses`), in order.
    pub buses: Vec<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    pub name: String,
    pub models: Vec<Model>,
}

/// The buses in their three levels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tree {
    pub buses: Vec<Bus>,
    pub groups: Vec<Group>,
}

impl Tree {
    fn group(&self, name: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.name == name)
    }
    fn model(&self, group: &str, model: &str) -> Option<&Model> {
        self.group(group)?.models.iter().find(|m| m.name == model)
    }
    fn find(&self, file: &str) -> Option<usize> {
        self.buses.iter().position(|b| b.file == file)
    }
}

/// Where the tiles are: the makers, a maker's models, or a model's versions.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Level {
    #[default]
    Groups,
    Models(String),
    Versions(String, String),
}

/// What a tile opens.
#[derive(Clone, Debug, PartialEq)]
pub enum To {
    Group(String),
    Model(String, String),
    Bus(usize),
}

/// One tile of the grid.
#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    pub to: To,
    pub title: String,
    pub sub: String,
    /// The bus whose photo it shows (a group's or a model's: the chosen one in it, the one
    /// offered, or its first).
    pub bus: usize,
    pub chosen: bool,
    /// A version's star, on or off; None on a group's or model's tile.
    pub star: Option<bool>,
    /// A starred bus is in it.
    pub starred: bool,
    pub badge: Option<(&'static str, Color)>,
}

impl Tile {
    fn key(&self, tree: &Tree) -> String {
        match &self.to {
            To::Group(g) => format!("g-{g}"),
            To::Model(g, m) => format!("m-{g}-{m}"),
            To::Bus(i) => format!("b-{}", tree.buses[*i].file),
        }
    }
}

/// What was clicked on the grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Click {
    Open(usize),
    Star(usize),
    /// The "3D" mark on a version's photo: the bus in the showroom.
    Look(usize),
}

/// The bus that fits the duty best.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub file: String,
    /// The share of the duty's trips its depot file knows; None when none of its depot files
    /// knows any and it was taken for being the map's own.
    pub fit: Option<f32>,
    /// That depot file's name.
    pub depot: String,
    /// A free drive's: the bus the map's depot runs most.
    pub free: bool,
}

#[derive(Default)]
struct Recommend {
    /// The duty it was worked out for.
    key: String,
    rx: Option<Receiver<Option<Pick>>>,
    pick: Option<Pick>,
    /// The depot files read, by bus folder (kept for the session: a big installation's are
    /// read once).
    depots: Arc<Mutex<HashMap<PathBuf, Arc<Vec<Depot>>>>>,
}

#[derive(Default)]
pub struct BusPickView {
    /// The chosen bus in the showroom rather than the tiles.
    showroom: bool,
    level: Level,
    search: String,
    only_favourites: bool,
    /// The starred buses (#524), by file (lower case, '/'): the file the old bus list keeps
    /// them in, `~/.openomsi/favourite-buses.txt`.
    favourites: Option<BTreeSet<String>>,
    tree: Arc<Tree>,
    tree_key: (usize, u64, usize),
    /// `ui.time` of the last frame the step was drawn: a gap is the step entered anew.
    seen: f32,
    /// The grid is to be scrolled to the chosen tile (a level opened).
    rescroll: bool,
    recommend: Recommend,
    /// The duties whose bus was offered and answered, or that the player went looking for a
    /// bus for himself: the dialog is not put in the way again.
    answered: HashSet<String>,
    offering: bool,
    /// The bus's depot files: the tiles after the bus, and the dialog when it lacks the map's
    /// (see `hof`).
    pub(super) depots: super::hof::DepotView,
    /// How high the bus sheet's part under the livery came out last frame (it scrolls when
    /// that is more than it has room for).
    sheet_content: f32,
    /// The buses of the player's own line the duty is on, when it asks for some.
    line: LineBuses,
}

/// The buses a player's line asks for: what they were worked out for, the line's number, and
/// the bus files (`fav_key`).
#[derive(Default)]
struct LineBuses {
    key: String,
    number: String,
    buses: Arc<HashSet<String>>,
}

/// The word on the tiles of a line's buses.
const THIS_LINE: &str = "THIS LINE";

// --- the tree -------------------------------------------------------------------------------

/// Natural order: DL9 before DL10, case not counting.
pub(super) fn name_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let x: String = std::iter::from_fn(|| a.next_if(|c| c.is_ascii_digit())).collect();
                let y: String = std::iter::from_fn(|| b.next_if(|c| c.is_ascii_digit())).collect();
                let (x, y) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let o = x.len().cmp(&y.len()).then_with(|| x.cmp(y));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}

/// The model and the version of a bus: `[friendlyname]`'s second line split at its first
/// " - "; without one, the version is the bus's own paint, or its file's name.
pub fn split_type(type_name: &str, file: &str, default_paint: &str) -> (String, String) {
    let label = vehicle_type_label(type_name, Path::new(file));
    let parts: Vec<&str> = label.split(" - ").map(str::trim).filter(|s| !s.is_empty()).collect();
    let model = parts.first().map(|s| s.to_string()).unwrap_or_else(|| label.clone());
    let version = if parts.len() > 1 {
        parts[1..].join(" · ")
    } else if !default_paint.trim().is_empty() {
        display_bus_name(default_paint)
    } else {
        display_bus_name(&Path::new(file).file_stem().unwrap_or_default().to_string_lossy())
    };
    (model, version)
}

/// The buses in their groups and models, those `allowed` only (a server's buses when one is
/// joined).
pub fn build_tree(vehicles: &[VehicleInfo], allowed: Option<&HashSet<String>>, fresh: &HashSet<String>) -> Tree {
    let mut tree = Tree::default();
    let mut groups: HashMap<String, (String, HashMap<String, (String, Vec<usize>)>)> = HashMap::new();
    for v in vehicles {
        if !allowed.map(|a| a.contains(&fav_key(&v.file))).unwrap_or(true) {
            continue;
        }
        let maker = v.manufacturer.trim();
        let group = if maker.is_empty() { display_bus_name(&v.folder) } else { display_bus_name(maker) };
        let (model, version) = split_type(&v.type_name, &v.file, &v.default_paint);
        let k = tree.buses.len();
        tree.buses.push(Bus { file: v.file.clone(), group: group.clone(), model: model.clone(), version, name: display_bus_name(&v.name), liveries: v.paints.len() + 1, fresh: fresh.contains(&v.file), installed: v.installed, incomplete: !v.missing_packs.is_empty() });
        let g = groups.entry(group.to_lowercase()).or_insert_with(|| (group.clone(), HashMap::new()));
        g.1.entry(model.to_lowercase()).or_insert_with(|| (model.clone(), Vec::new())).1.push(k);
    }
    for (_, (gname, models)) in groups {
        let mut g = Group { name: gname, models: Vec::new() };
        for (_, (mname, mut buses)) in models {
            // (two files of one version: the pack tells them apart, and the file if that
            // does not)
            for pass in 0..2 {
                let mut seen: HashMap<String, usize> = HashMap::new();
                for &b in &buses {
                    *seen.entry(tree.buses[b].version.to_lowercase()).or_default() += 1;
                }
                for &b in &buses {
                    if seen[&tree.buses[b].version.to_lowercase()] > 1 {
                        let f = tree.buses[b].file.replace('\\', "/");
                        let extra = if pass == 0 { f.split('/').nth(1).unwrap_or_default().to_string() } else { Path::new(&f).file_stem().unwrap_or_default().to_string_lossy().into_owned() };
                        let b = &mut tree.buses[b];
                        b.version = format!("{} · {}", b.version, display_bus_name(&extra));
                    }
                }
            }
            buses.sort_by(|a, b| name_cmp(&tree.buses[*a].version, &tree.buses[*b].version).then_with(|| tree.buses[*a].file.cmp(&tree.buses[*b].file)));
            g.models.push(Model { name: mname, buses });
        }
        g.models.sort_by(|a, b| name_cmp(&a.name, &b.name));
        tree.groups.push(g);
    }
    tree.groups.sort_by(|a, b| name_cmp(&a.name, &b.name));
    tree
}

/// The search's words in a bus: its name, maker, model, version or file.
fn matches(b: &Bus, q: &str) -> bool {
    q.is_empty() || [&b.name, &b.group, &b.model, &b.version].iter().any(|s| s.to_lowercase().contains(q)) || display_bus_name(&b.file).to_lowercase().contains(q)
}

/// "%{count} versions", or "one version" (one is not "1 versions").
fn count_text(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        omsi_ui::tr(one).into_owned()
    } else {
        omsi_ui::tr(many).replace("%{count}", &n.to_string())
    }
}

/// The tiles of a level (or of a search, across them all): what each opens, its words, its
/// photo's bus and its marks.
pub fn tiles(tree: &Tree, level: &Level, q: &str, favs: Option<&BTreeSet<String>>, chosen: &str, best: Option<&Pick>) -> Vec<Tile> {
    let fav = |b: usize| favs.is_some_and(|f| f.contains(&fav_key(&tree.buses[b].file)));
    let starred = |b: usize| fav(b) || tree.buses[b].file == chosen;
    // (the starred ones only: a family with one of them, and in it only those - #524)
    let keep = |b: usize| favs.is_none() || starred(b);
    let best_file = best.map(|p| p.file.as_str());
    let best_word = if best.is_some_and(|p| p.free) { "drives here most" } else { "fits best" };
    let with_best = |text: String, is: bool| if is { format!("{text} · {}", omsi_ui::tr(best_word)) } else { text };
    let pick = |buses: &[usize]| -> usize {
        buses.iter().copied().find(|b| tree.buses[*b].file == chosen).or_else(|| buses.iter().copied().find(|b| Some(tree.buses[*b].file.as_str()) == best_file)).unwrap_or(buses[0])
    };
    let bus_tile = |b: usize, title: String, sub: String| {
        let bus = &tree.buses[b];
        let badge = if bus.incomplete { Some(("PARTS MISSING", WARN)) } else if bus.fresh { Some(("NEW", OK)) } else if bus.installed { Some(("MOD", TEXT_SOFT)) } else { None };
        Tile { to: To::Bus(b), title, sub: with_best(sub, Some(bus.file.as_str()) == best_file), bus: b, chosen: bus.file == chosen, star: Some(fav(b)), starred: fav(b), badge }
    };
    if !q.is_empty() {
        let mut hits: Vec<usize> = (0..tree.buses.len()).filter(|b| matches(&tree.buses[*b], q) && keep(*b)).collect();
        hits.sort_by(|a, b| name_cmp(&tree.buses[*a].name, &tree.buses[*b].name));
        return hits.into_iter().map(|b| bus_tile(b, format!("{} {}", tree.buses[b].group, tree.buses[b].model), format!("{} · {}", tree.buses[b].version, count_text(tree.buses[b].liveries, "one livery", "%{count} liveries")))).collect();
    }
    let versions = |buses: &[usize]| -> Vec<usize> { buses.iter().copied().filter(|b| keep(*b)).collect() };
    match level {
        Level::Groups => tree.groups.iter().filter_map(|g| {
            let buses: Vec<usize> = g.models.iter().flat_map(|m| versions(&m.buses)).collect();
            if buses.is_empty() {
                return None;
            }
            let b = pick(&buses);
            let has = |f: &str| buses.iter().any(|x| tree.buses[*x].file == f);
            Some(Tile { to: To::Group(g.name.clone()), title: g.name.clone(), sub: with_best(count_text(buses.len(), "one version", "%{count} versions"), best_file.is_some_and(has)), bus: b, chosen: has(chosen), star: None, starred: buses.iter().any(|x| fav(*x)), badge: None })
        }).collect(),
        Level::Models(g) => tree.group(g).map(|g| g.models.iter().filter_map(|m| {
            let buses = versions(&m.buses);
            if buses.is_empty() {
                return None;
            }
            let has = |f: &str| buses.iter().any(|x| tree.buses[*x].file == f);
            Some(Tile { to: To::Model(g.name.clone(), m.name.clone()), title: m.name.clone(), sub: with_best(count_text(buses.len(), "one version", "%{count} versions"), best_file.is_some_and(has)), bus: pick(&buses), chosen: has(chosen), star: None, starred: buses.iter().any(|x| fav(*x)), badge: None })
        }).collect()).unwrap_or_default(),
        Level::Versions(g, m) => tree.model(g, m).map(|m| versions(&m.buses).into_iter().map(|b| bus_tile(b, tree.buses[b].version.clone(), count_text(tree.buses[b].liveries, "one livery", "%{count} liveries"))).collect()).unwrap_or_default(),
    }
}

/// The buses a player's line asks for (`line`, by `fav_key`) marked "this line" on their tiles
/// and put first - a maker's or a model's tile with one of them in it too, showing one of them
/// unless it shows the chosen bus -, the order otherwise kept.
pub fn mark_line(tree: &Tree, list: &mut [Tile], line: &HashSet<String>, chosen: &str) {
    if line.is_empty() {
        return;
    }
    let has = |b: usize| line.contains(&fav_key(&tree.buses[b].file));
    for t in list.iter_mut() {
        let inside: Vec<usize> = match &t.to {
            To::Bus(b) => vec![*b],
            To::Group(g) => tree.group(g).map(|g| g.models.iter().flat_map(|m| m.buses.iter().copied()).collect()).unwrap_or_default(),
            To::Model(g, m) => tree.model(g, m).map(|m| m.buses.clone()).unwrap_or_default(),
        };
        let Some(first) = inside.iter().copied().find(|b| has(*b)) else { continue };
        if !matches!(t.badge, Some(("PARTS MISSING", _))) {
            t.badge = Some((THIS_LINE, accent_2()));
        }
        if !has(t.bus) && tree.buses[t.bus].file != chosen {
            t.bus = first;
        }
    }
    list.sort_by_key(|t| !inside_line(tree, t, line));
}

/// A tile is (or holds) one of a line's buses.
fn inside_line(tree: &Tree, t: &Tile, line: &HashSet<String>) -> bool {
    let has = |b: &usize| line.contains(&fav_key(&tree.buses[*b].file));
    match &t.to {
        To::Bus(b) => has(b),
        To::Group(g) => tree.group(g).is_some_and(|g| g.models.iter().flat_map(|m| m.buses.iter()).any(has)),
        To::Model(g, m) => tree.model(g, m).is_some_and(|m| m.buses.iter().any(has)),
    }
}

/// The player's own line the duty is on - a tour of it, or a free drive along it - with the
/// buses it asks for (None: a line of the map's, or one that asks for none).
fn own_line_buses(s: &super::state::State) -> Option<(String, omsi_launcher_lib::service::LineVehicles)> {
    let name = if s.choice.free { s.choice.own_line.then(|| s.choice.free_line.clone())? } else { s.choice.line.clone()? };
    let o = omsi_launcher_lib::lines::own_line_of(&name, &s.own_lines)?;
    (!o.vehicles.open()).then(|| (o.number.clone(), o.vehicles))
}

// --- the bus that fits the duty ------------------------------------------------------------

/// A depot file as the offer needs it: its name, the trips its IBIS knows and its termini.
#[derive(Clone, Debug, Default)]
pub struct Depot {
    pub name: String,
    pub trips: HashSet<String>,
    pub termini: HashSet<String>,
}

/// A name to compare: lower case, letters and digits only, the accents off.
pub fn fold(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'ß' => out.push_str("ss"),
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => out.push('a'),
            'è' | 'é' | 'ê' | 'ë' => out.push('e'),
            'ì' | 'í' | 'î' | 'ï' => out.push('i'),
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' => out.push('o'),
            'ù' | 'ú' | 'û' | 'ü' => out.push('u'),
            c if c.is_alphanumeric() => out.push(c),
            _ => {}
        }
    }
    out
}

pub fn depot_of(h: &omsi_vehicle::Hof) -> Depot {
    let name = if h.name.trim().is_empty() { h.path.file_stem().unwrap_or_default().to_string_lossy().into_owned() } else { h.name.trim().to_string() };
    let trips = h.info_trips.iter().map(|t| fold(&t.name)).filter(|s| !s.is_empty()).collect();
    let mut termini = HashSet::new();
    for t in &h.termini {
        termini.insert(fold(&t.texture_id));
        termini.insert(fold(&t.strings.join(" ")));
        termini.extend(t.strings.iter().map(|s| fold(s)));
    }
    termini.remove("");
    Depot { name, trips, termini }
}

/// How many of the trips (name, terminus) a depot file knows: one its IBIS has under its name,
/// or one whose terminus it can show.
pub fn knows(d: &Depot, trips: &[(String, String)]) -> usize {
    trips.iter().filter(|(name, end)| d.trips.contains(&fold(name)) || (!end.is_empty() && d.termini.contains(&fold(end)))).count()
}

/// How many of each bus file the map's own fleet has: its depot groups' type groups and the
/// vehicles of the groups that run from a depot.
pub fn fleet_of(ai: &omsi_map::ailists::AiLists, date: Option<i32>) -> HashMap<String, f32> {
    let mut out: HashMap<String, f32> = HashMap::new();
    for g in ai.groups.iter().filter(|g| g.is_depot || g.hof.is_some()) {
        for v in &g.vehicles {
            *out.entry(fav_key(v.file.trim())).or_default() += v.weight.max(0.0);
        }
        for t in &g.typgroups {
            let n = t.entries.iter().filter(|e| date.map(|d| omsi_map::typgroup_entry_valid(e, d)).unwrap_or(true)).count();
            *out.entry(fav_key(t.file.trim())).or_default() += n.max(1) as f32;
        }
    }
    out
}

/// A bus weighed for the duty.
#[derive(Clone, Debug, Default)]
pub struct Candidate {
    pub file: String,
    pub name: String,
    pub fit: f32,
    pub depot: String,
    /// How many of it the map's fleet has.
    pub wagons: f32,
}

/// The bus to offer: of those whose depot file knows the duty's trips, the map's own before
/// the others, then the one that knows most, then the one the map has most of; a free drive
/// (or a duty no depot file knows) the bus the map has most of.
pub fn rank(c: &[Candidate], free: bool) -> Option<Pick> {
    let by_name = |a: &Candidate, b: &Candidate| name_cmp(&b.name, &a.name).then_with(|| b.file.cmp(&a.file));
    if !free {
        let fitting: Vec<&Candidate> = c.iter().filter(|x| x.fit > 0.0).collect();
        let own: Vec<&Candidate> = fitting.iter().copied().filter(|x| x.wagons > 0.0).collect();
        let pool = if own.is_empty() { fitting } else { own };
        if let Some(b) = pool.into_iter().max_by(|a, b| a.fit.total_cmp(&b.fit).then(a.wagons.total_cmp(&b.wagons)).then_with(|| by_name(a, b))) {
            return Some(Pick { file: b.file.clone(), fit: Some(b.fit), depot: b.depot.clone(), free: false });
        }
    }
    c.iter().filter(|x| x.wagons > 0.0).max_by(|a, b| a.wagons.total_cmp(&b.wagons).then_with(|| by_name(a, b))).map(|b| Pick { file: b.file.clone(), fit: None, depot: String::new(), free })
}

/// The depot files of a bus: those of its folder, else those of its pack's (as the bus list
/// reads them).
fn depots_of(root: &Path, file: &str, cache: &Mutex<HashMap<PathBuf, Arc<Vec<Depot>>>>) -> Arc<Vec<Depot>> {
    let path = omsi_cfg::resolve_path(root, file);
    let dir = path.parent().unwrap_or(root).to_path_buf();
    if let Some(d) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&dir) {
        return d.clone();
    }
    let pack = file.replace('\\', "/").split('/').take(2).collect::<Vec<_>>().join("/");
    let mut files = omsi_vehicle::hof::depot_files(&dir);
    if files.is_empty() && !pack.is_empty() {
        files = omsi_vehicle::hof::depot_files(&omsi_cfg::resolve_path(root, &pack));
    }
    let depots = Arc::new(files.iter().filter_map(|f| omsi_vehicle::Hof::load(f).ok()).map(|h| depot_of(&h)).collect::<Vec<_>>());
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(dir, depots.clone());
    depots
}

/// Work out the bus to offer (on a thread: depot files are read, some are megabytes); of
/// `only` (a player's line's buses) when it names any.
#[allow(clippy::too_many_arguments)]
fn recommend(root: PathBuf, map: String, date: String, free: bool, trips: Vec<(String, String)>, buses: Vec<(String, String)>, only: Arc<HashSet<String>>, cache: Arc<Mutex<HashMap<PathBuf, Arc<Vec<Depot>>>>>) -> Option<Pick> {
    let buses: Vec<(String, String)> = if only.is_empty() { buses } else { buses.into_iter().filter(|(f, _)| only.contains(&fav_key(f))).collect() };
    let t0 = std::time::Instant::now();
    let map_dir = omsi_cfg::resolve_path(&root, &map).parent()?.to_path_buf();
    let date = omsi_map::ailists::date_code(&date);
    let chrono = date.map(|d| omsi_map::active_chrono_dirs(&map_dir, d)).unwrap_or_default();
    let ai = omsi_map::ailists::ailists_with_chrono(&map_dir, &chrono);
    let fleet = fleet_of(&ai, date);
    let weigh = |(file, name): &(String, String)| -> Candidate {
        let wagons = fleet.get(&fav_key(file)).copied().unwrap_or(0.0);
        let mut c = Candidate { file: file.clone(), name: name.clone(), fit: 0.0, depot: String::new(), wagons };
        if !free && !trips.is_empty() {
            for d in depots_of(&root, file, &cache).iter() {
                let fit = knows(d, &trips) as f32 / trips.len() as f32;
                if fit > c.fit {
                    (c.fit, c.depot) = (fit, d.name.clone());
                }
            }
        }
        c
    };
    // the map's own buses first: when one of them fits, the others need not be read
    let (own, rest): (Vec<&(String, String)>, Vec<&(String, String)>) = buses.iter().partition(|(f, _)| fleet.get(&fav_key(f)).is_some_and(|w| *w > 0.0));
    let mut c: Vec<Candidate> = own.into_iter().map(weigh).collect();
    if !free && !trips.is_empty() && !c.iter().any(|x| x.fit > 0.0) {
        c.extend(rest.into_iter().map(weigh));
    }
    let pick = rank(&c, free || trips.is_empty());
    log::info!("bus picker: {} offered for the duty ({} buses weighed in {:.1} s)", pick.as_ref().map(|p| format!("{} ({:.0} %)", p.file, p.fit.unwrap_or(0.0) * 100.0)).unwrap_or_else(|| "no bus".into()), c.len(), t0.elapsed().as_secs_f32());
    pick
}

/// The duty's trips (name, terminus), and whether there are none to fit (a free drive, or no
/// duty chosen yet).
fn duty_trips(l: &Launcher) -> (bool, Vec<(String, String)>) {
    let s = &l.state;
    if s.choice.free {
        return (true, Vec::new());
    }
    let mut trips = Vec::new();
    if s.choice.composed {
        for leg in s.composed_legs().unwrap_or(&[]) {
            let Some((line, tour, first, count)) = omsi_launcher_lib::compose::Block::parse(leg) else { continue };
            if let Some(t) = s.lines.iter().find(|x| x.name == line).and_then(|x| x.tours.iter().find(|t| t.number == tour)) {
                trips.extend(t.trips.iter().filter(|x| x.index >= first && x.index < first + count).map(|x| (x.name.clone(), x.terminus.clone())));
            }
        }
    } else if let Some(t) = s.tour() {
        trips.extend(t.trips.iter().skip(s.first_trip().unwrap_or(0)).map(|x| (x.name.clone(), x.terminus.clone())));
    }
    (trips.is_empty(), trips)
}

// --- the favourites -------------------------------------------------------------------------

/// The starred buses (#524): the file the old bus list (`drive`, the phone's) keeps them in,
/// one bus file a line, so that a star set in one is in the other.
fn favourites_file() -> PathBuf {
    omsi_launcher_lib::data_dir().join("favourite-buses.txt")
}

fn fav_key(file: &str) -> String {
    file.replace('\\', "/").to_lowercase()
}

fn read_favourites() -> BTreeSet<String> {
    std::fs::read_to_string(favourites_file()).map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(fav_key).collect()).unwrap_or_default()
}

fn toggle_favourite(v: &mut BusPickView, file: &str) {
    let f = v.favourites.get_or_insert_with(read_favourites);
    let k = fav_key(file);
    if !f.remove(&k) {
        f.insert(k);
    }
    let text: String = f.iter().map(|l| format!("{l}\n")).collect();
    if let Err(e) = std::fs::write(favourites_file(), text) {
        log::warn!("favourite buses not saved: {e}");
    }
}

// --- each frame -----------------------------------------------------------------------------

/// Once a frame on the bus step, before it is drawn: the tree kept up with the buses, the bus
/// offered worked out, the dialog opened on the way in, the photos taken.
pub(super) fn frame(l: &mut Launcher) {
    l.buspick.seen = l.ui.time;
    // the tree, when the buses (or those a server allows) changed
    let allowed: Option<HashSet<String>> = l.state.host_vehicles().map(|v| v.iter().map(|f| fav_key(f)).collect());
    if let Some(a) = allowed.as_ref() {
        if !a.contains(&fav_key(&l.state.choice.bus)) {
            if let Some(first) = l.state.vehicles.iter().find(|v| a.contains(&fav_key(&v.file))).map(|v| v.file.clone()) {
                l.state.select_bus(&first);
            }
        }
    }
    let allowed_key = allowed.as_ref().map(|a| a.iter().fold(0u64, |h, f| h ^ id_of(f))).unwrap_or(u64::MAX);
    let key = (l.state.vehicles.len(), allowed_key, l.state.fresh.len());
    if key != l.buspick.tree_key {
        let fresh: HashSet<String> = l.state.fresh.keys().cloned().collect();
        l.buspick.tree = Arc::new(build_tree(&l.state.vehicles, allowed.as_ref(), &fresh));
        l.buspick.tree_key = key;
    }
    // the buses of the player's own line the duty is on, when it asks for some (`busclass`)
    l.busclasses.want(&l.state.vehicles);
    let want = own_line_buses(&l.state);
    let line_key = want.as_ref().map(|(n, w)| format!("{n}|{}|{}|{w:?}", l.state.vehicles.len(), l.busclasses.busy())).unwrap_or_default();
    if line_key != l.buspick.line.key {
        l.buspick.line = match want {
            Some((number, w)) => LineBuses { key: line_key, number, buses: Arc::new(l.busclasses.matching(&l.state.vehicles, &w)) },
            None => LineBuses { key: line_key, ..Default::default() },
        };
    }
    // the bus to offer, worked out again when the duty (or the buses) changed
    let (free, trips) = duty_trips(l);
    let duty = format!("{}|{}|{}|{}|{}|{}", l.state.choice.map, l.state.choice.date, free, trips.iter().map(|t| t.0.as_str()).collect::<Vec<_>>().join(","), l.buspick.tree.buses.len(), l.buspick.line.key);
    let v = &mut l.buspick;
    if v.recommend.key != duty && !l.state.choice.map.is_empty() && !v.tree.buses.is_empty() {
        v.recommend.key = duty.clone();
        v.recommend.pick = None;
        let (tx, rx) = std::sync::mpsc::channel();
        let (root, map, date, cache) = (PathBuf::from(&l.state.config.root), l.state.choice.map.clone(), l.state.choice.date.clone(), v.recommend.depots.clone());
        let buses: Vec<(String, String)> = v.tree.buses.iter().map(|b| (b.file.clone(), b.name.clone())).collect();
        let only = v.line.buses.clone();
        std::thread::spawn(move || {
            let _ = tx.send(recommend(root, map, date, free, trips, buses, only, cache));
        });
        v.recommend.rx = Some(rx);
    }
    if let Some(Ok(p)) = v.recommend.rx.as_ref().map(|rx| rx.try_recv()) {
        v.recommend.pick = p;
        v.recommend.rx = None;
    }
    // (no dialog offers the duty's bus any more - "This bus is ready for you": the player picks
    // one himself; the bus openOMSI would take is only marked on its tile and sheet)
    v.offering = false;
    // (the bus in the showroom is gone: a mod taken out, a server joined)
    if (l.buspick.showroom || l.buspick.depots.open) && l.state.bus().is_none() {
        l.buspick.showroom = false;
        l.buspick.depots.open = false;
    }
    // the bus's depot files, and the dialog when it lacks the map's (after the bus offered)
    let offering = l.buspick.offering;
    super::hof::frame(l, offering);
    busphoto::work(l);
}

/// The chosen bus is in the showroom (rather than the tiles).
pub(super) fn showing(l: &Launcher) -> bool {
    l.buspick.showroom
}

/// Where the bus stands in the showroom, in a window `size` big with the bus's sheet at
/// `sheet`: right of the sheet to the window's margin, from the sheet's top down to the line
/// over the actions (the duty in a word above the main action) - the part nothing lies over.
/// The showroom frames the bus in its middle (see `showroom::frame`).
pub(super) fn stage(size: Vec2, sheet: Rect) -> Rect {
    use super::flow::{ACTION_BOTTOM, ACTION_H, EDGE_IN, SHEET_TOP};
    // (the duty's line sits 32 px over the action, its room 14 more)
    let bottom = size.y - ACTION_BOTTOM - ACTION_H - 32.0 - 14.0;
    let x = sheet.right() + EDGE_IN;
    Rect::new(x, SHEET_TOP, (size.x - EDGE_IN - x).max(48.0), (bottom - SHEET_TOP).max(48.0))
}

/// The bus offered for the duty (or the dialog of a bus without the map's depot file) lies
/// over the step.
pub(super) fn offering(l: &Launcher) -> bool {
    l.buspick.offering || super::hof::asking(l)
}

/// The chosen bus's depot files, as tiles in the wide sheet.
fn open_depots(l: &mut Launcher) {
    if l.state.bus().is_some() {
        l.buspick.depots.open = true;
        l.buspick.showroom = false;
        l.ui.scroll.remove(&id_of("hof-grid"));
        l.ui.scroll.remove(&(id_of("hof-grid") ^ 0xabc));
        acted(&mut l.buspick);
    }
}

/// The player looked for a bus himself: the dialog is not put in his way again for this duty.
fn acted(v: &mut BusPickView) {
    let k = v.recommend.key.clone();
    v.answered.insert(k);
    v.offering = false;
}

fn go_to(l: &mut Launcher, level: Level) {
    l.buspick.level = level;
    l.buspick.showroom = false;
    l.buspick.depots.open = false;
    l.buspick.rescroll = true;
    acted(&mut l.buspick);
}

/// The chosen bus in the showroom ("View in 3D").
pub(super) fn show_bus(l: &mut Launcher) {
    if l.state.bus().is_some() {
        l.buspick.showroom = true;
        l.buspick.depots.open = false;
        // (a bus in view: the dialog of its depot file may come now)
        l.buspick.depots.deferred = false;
        acted(&mut l.buspick);
    }
}

/// One level up: the showroom to its bus's versions, the versions to the models, the models to
/// the makers, a search to the tiles. False at the top: Back goes to the step before.
pub(super) fn back(l: &mut Launcher) -> bool {
    let v = &mut l.buspick;
    if v.depots.open {
        v.depots.open = false;
        v.showroom = true;
        return true;
    }
    if v.showroom {
        let tree = v.tree.clone();
        let level = tree.find(&l.state.choice.bus).map(|b| Level::Versions(tree.buses[b].group.clone(), tree.buses[b].model.clone())).unwrap_or_default();
        go_to(l, level);
        return true;
    }
    if !v.search.is_empty() {
        v.search.clear();
        return true;
    }
    match v.level.clone() {
        Level::Groups => false,
        Level::Models(_) => {
            go_to(l, Level::Groups);
            true
        }
        Level::Versions(g, _) => {
            go_to(l, Level::Models(g));
            true
        }
    }
}

/// The sheet's title and the line under it.
pub(super) fn heading(l: &Launcher) -> (String, String) {
    let v = &l.buspick;
    if v.depots.open {
        return super::hof::heading();
    }
    if v.showroom {
        let tree = &v.tree;
        return match tree.find(&l.state.choice.bus) {
            Some(b) => (tree.buses[b].model.clone(), format!("{} · {}", tree.buses[b].group, tree.buses[b].version)),
            None => (l.state.bus().map(|b| display_bus_name(&b.name)).unwrap_or_default(), String::new()),
        };
    }
    if !v.search.trim().is_empty() {
        return (omsi_ui::tr("Buses").into_owned(), omsi_ui::tr("Which bus are you taking out?").into_owned());
    }
    match &v.level {
        Level::Groups => (omsi_ui::tr("Buses").into_owned(), omsi_ui::tr("Which bus are you taking out?").into_owned()),
        Level::Models(g) => (g.clone(), omsi_ui::tr("Which model?").into_owned()),
        Level::Versions(_, m) => (m.clone(), omsi_ui::tr("Which version? Gearbox, doors and cab differ.").into_owned()),
    }
}

/// The grid's foot: how many buses, and which fits best.
pub(super) fn foot(l: &Launcher) -> String {
    let v = &l.buspick;
    if v.depots.open {
        return super::hof::foot(l);
    }
    let mut n = omsi_ui::tr("%{n} buses installed").replace("%{n}", &v.tree.buses.len().to_string());
    if !v.line.buses.is_empty() {
        n = format!("{n} · {}", omsi_ui::tr("%{n} for line %{line}").replace("%{n}", &v.line.buses.len().to_string()).replace("%{line}", &v.line.number));
    }
    match v.recommend.pick.as_ref().and_then(|p| v.tree.find(&p.file).map(|b| (p, b))) {
        Some((p, b)) => {
            let line = if p.free { "%{bus} drives here most" } else { "%{bus} fits this duty best" };
            format!("{n} · {}", omsi_ui::tr(line).replace("%{bus}", &v.tree.buses[b].name))
        }
        None => n,
    }
}

// --- the tiles ------------------------------------------------------------------------------

const TILE_W: f32 = 240.0;
const TILE_GAP: f32 = 16.0;
/// Under a tile's photo: its name and the line under it.
const TILE_TEXT_H: f32 = 64.0;

/// The grid's columns and a tile's size in a grid `w` wide: (columns, tile width, photo
/// height, tile height).
pub fn grid_layout(w: f32) -> (usize, f32, f32, f32) {
    let n = (((w - 8.0 + TILE_GAP) / (TILE_W + TILE_GAP)).floor() as usize).max(1);
    let tw = ((w - 8.0 - TILE_GAP * (n as f32 - 1.0)) / n as f32).max(80.0);
    let ph = (tw * 0.6).round();
    (n, tw, ph, ph + TILE_TEXT_H)
}

/// The first letters of a name's first two words ("BHD MAN": BM), for a tile without a photo.
pub fn initials(name: &str) -> String {
    name.split(|c: char| c.is_whitespace() || c == '_' || c == '-').filter(|w| !w.is_empty()).take(2).filter_map(|w| w.chars().next()).flat_map(char::to_uppercase).collect()
}

/// The tiles in rows, scrolled: a photo (or the bus's initials while it is drawn), the name
/// and the line under it; on a version's tile its star, and the "3D" mark that shows it in the
/// showroom. `photo` is asked only for the tiles on screen. Returns what was clicked.
pub fn grid(ui: &mut Ui, r: Rect, tiles: &[Tile], keys: &[String], busy: Option<usize>, photo: &mut dyn FnMut(&Tile) -> Option<(usize, u32, u32)>) -> Option<Click> {
    let (n, tw, ph, th) = grid_layout(r.w);
    let mut click = None;
    ui.scroll_area("bus-grid", r, &mut |ui, v| {
        for (k, t) in tiles.iter().enumerate() {
            // (room above the first row for a tile risen under the mouse)
            let tile = Rect::new(v.x + (k % n) as f32 * (tw + TILE_GAP), v.y + 6.0 + (k / n) as f32 * (th + TILE_GAP), tw, th);
            if !ui.rect_visible(tile) {
                continue;
            }
            let pic = photo(t);
            if let Some(c) = draw_tile(ui, tile, ph, t, &keys[k], pic, busy == Some(t.bus)) {
                click = Some(match c {
                    0 => Click::Open(k),
                    1 => Click::Star(k),
                    _ => Click::Look(k),
                });
            }
        }
        tiles.len().div_ceil(n) as f32 * (th + TILE_GAP) + 10.0
    });
    click
}

/// One tile. Returns 0 when it was clicked, 1 its star, 2 its "3D" mark.
fn draw_tile(ui: &mut Ui, tile: Rect, ph: f32, t: &Tile, key: &str, pic: Option<(usize, u32, u32)>, busy: bool) -> Option<u8> {
    let id = id_of(&format!("bus-tile-{key}"));
    // (under the mouse it moves as the start's tiles do, `Ui::tile`: its parts are laid out in
    // the tile as drawn, the star's and the "3D" mark's hit areas where it lies)
    let m = ui.tile(id, tile, SHEET_RADIUS);
    let (h, clicked, base) = (m.hovered, m.clicked, tile);
    let tile = m.r;
    let ph = ph * m.grown();
    let fill = if t.chosen { accent() } else { FIELD.mix(HOVER, m.hover) };
    ui.tile_shadow(&m);
    if t.chosen {
        ui.p().shadow(tile.inset(-3.0), SHEET_RADIUS + 3.0, 18.0, accent().alpha(0.4));
    }
    ui.p().rounded(tile, SHEET_RADIUS, fill);
    let photo = Rect::new(tile.x, tile.y, tile.w, ph);
    // the photo, its lower corners under the name's part (a layer rounds all four)
    let under = Rect::new(tile.x, tile.y, tile.w, ph + SHEET_RADIUS);
    match pic {
        Some((tex, w, hh)) => ui.tile_photo(&m, under, SHEET_RADIUS, tex, w, hh),
        None => ui.p().rounded_gradient(under, SHEET_RADIUS, Color::rgba(38, 48, 70, 1.0), Color::rgba(24, 31, 46, 1.0)),
    }
    ui.p().rect(Rect::new(tile.x, tile.y + ph, tile.w, SHEET_RADIUS), fill);
    ui.p().rounded(Rect::new(tile.x, tile.y + ph, tile.w, tile.h - ph), SHEET_RADIUS, fill);
    ui.tile_light(&m, 0.12);
    if pic.is_none() {
        let mono = Rect::new(photo.center().x - 28.0, photo.center().y - 28.0, 56.0, 56.0);
        ui.p().rounded(mono, RADIUS, Color::WHITE.alpha(0.09));
        ui.text_in(&initials(&t.title), mono, 20.0, Weight::Bold, TEXT, Align::Center);
        if busy {
            // (this one is being photographed)
            let c = Vec2::new(photo.right() - 20.0, photo.bottom() - 18.0);
            let a = ui.time * 5.0;
            ui.p().arc(c, 6.0, 8.0, a, a + 4.2, TEXT_SOFT);
        }
    }
    if t.chosen {
        ui.p().rounded_border(tile, SHEET_RADIUS, 2.0, accent());
    } else {
        ui.tile_edge(&m, 1.0, EDGE);
    }
    let ink = if t.chosen { on_accent() } else { TEXT };
    let soft = if t.chosen { on_accent().alpha(0.85) } else { TEXT_SOFT };
    // (the words' room is the tile's where it lies: they cut the same under the mouse)
    ui.text_in(&t.title, Rect::new(tile.x + 16.0, tile.y + ph + 10.0, base.w - 32.0, 22.0), 15.0, Weight::Bold, ink, Align::Left);
    ui.text_in(&t.sub, Rect::new(tile.x + 16.0, tile.y + ph + 34.0, base.w - 32.0, 18.0), 12.5, Weight::Regular, soft, Align::Left);
    // the marks on the photo: a version's state, its star, its way into the showroom
    if let Some((word, c)) = t.badge {
        let tw = ui.width(word, 10.0, Weight::Bold) + 14.0;
        let b = Rect::new(photo.x + 12.0, photo.bottom() - 30.0, tw, 18.0);
        ui.p().rounded(b, 5.0, Color::rgba(9, 12, 24, 0.8));
        ui.text_in(word, b, 10.0, Weight::Bold, c, Align::Center);
    }
    let mut out = clicked.then_some(0);
    match t.star {
        Some(on) => {
            let hit = Vec2::new(base.x + 22.0, base.y + 22.0);
            let c = m.at(hit);
            let r = Rect::new(c.x - 15.0, c.y - 15.0, 30.0, 30.0);
            let (hs, _, cs) = ui.interact(id_of(&format!("bus-tile-star-{key}")), Rect::new(hit.x - 15.0, hit.y - 15.0, 30.0, 30.0));
            if on || hs || h {
                ui.p().circle(c, 15.0, Color::rgba(9, 12, 24, 0.72));
                ui.icon("star", c, 17.0, if on { accent() } else if hs { TEXT } else { Color::WHITE.alpha(0.45) });
            }
            ui.tooltip(r, if on { "Remove from the favourites" } else { "Add to the favourites" });
            if cs {
                out = Some(1);
            }
        }
        None if t.starred => {
            let c = Vec2::new(photo.x + 22.0, photo.y + 22.0);
            ui.p().circle(c, 13.0, Color::rgba(9, 12, 24, 0.72));
            ui.icon("star", c, 14.0, accent());
        }
        None => {}
    }
    if pic.is_some() && matches!(t.to, To::Bus(_)) {
        let hit = Vec2::new(base.right() - 22.0, base.y + 22.0);
        let c = m.at(hit);
        let r = Rect::new(c.x - 15.0, c.y - 15.0, 30.0, 30.0);
        let (hs, _, cs) = ui.interact(id_of(&format!("bus-tile-3d-{key}")), Rect::new(hit.x - 15.0, hit.y - 15.0, 30.0, 30.0));
        ui.p().circle(c, 15.0, if hs { accent() } else { Color::rgba(9, 12, 24, 0.72) });
        ui.text_in("3D", r, 10.5, Weight::Bold, if hs { on_accent() } else { Color::WHITE }, Align::Center);
        ui.tooltip(r, "View in 3D");
        if cs {
            out = Some(2);
        }
    }
    out
}

/// The way back up: "Buses › MAN › NL202", every part but the last a link. Returns the part
/// clicked.
pub fn crumbs(ui: &mut Ui, r: Rect, parts: &[String]) -> Option<usize> {
    let mut x = r.x;
    let px = ui.wpx(13.0);
    // (too long for the room: the parts on the way are cut, all alike, before the last one is)
    let widths: Vec<f32> = parts.iter().enumerate().map(|(k, p)| ui.width(p, px, if k + 1 == parts.len() { Weight::Bold } else { Weight::Medium }) + 1.0).collect();
    let joins = 20.0 * parts.len().saturating_sub(1) as f32;
    let last_w = widths.last().copied().unwrap_or(0.0).min(r.w * 0.5);
    let way: Vec<f32> = widths.iter().take(parts.len().saturating_sub(1)).copied().collect();
    let room = (r.w - joins - last_w).max(0.0);
    let mut cap = way.iter().copied().fold(0.0, f32::max);
    while way.iter().map(|w| w.min(cap)).sum::<f32>() > room && cap > 36.0 {
        cap -= 4.0;
    }
    let mut clicked = None;
    for (k, p) in parts.iter().enumerate() {
        let last = k + 1 == parts.len();
        let w = if last { widths[k] } else { widths[k].min(cap) }.min((r.right() - x).max(0.0));
        let cell = Rect::new(x, r.y, w, r.h);
        if last {
            ui.text_in(p, cell, px, Weight::Bold, TEXT, Align::Left);
        } else {
            let (h, _, c) = ui.interact(id_of(&format!("bus-crumb-{k}")), cell);
            ui.text_in(p, cell, px, Weight::Medium, if h { accent_2() } else { accent() }, Align::Left);
            if h {
                ui.p().rect(Rect::new(cell.x, cell.bottom() - 2.0, cell.w, 1.0), accent_2());
            }
            if c {
                clicked = Some(k);
            }
            ui.icon("chevron_right", Vec2::new(x + w + 10.0, r.center().y), 14.0, TEXT_FAINT);
        }
        x += w + 20.0;
    }
    clicked
}

/// The tiles in the wide sheet: the way back up and the search above them, the grid, and in
/// the foot the photos made of every bus.
pub(super) fn browse(l: &mut Launcher, r: Rect, foot: Rect) {
    if l.buspick.depots.open {
        depot_browse(l, r, foot);
        return;
    }
    let tree = l.buspick.tree.clone();
    // above the tiles: the crumbs on the left, the search and the stars on the right
    let row = Rect::new(r.x, r.y, r.w, ROW);
    let fav_w = l.ui.width(&omsi_ui::tr("Favourites only"), 13.0, Weight::Regular) + 50.0;
    let search = Rect::new(row.right() - fav_w - 16.0 - 260.0, row.y, 260.0, ROW);
    if l.ui.text_input("bus-search", search, &mut l.buspick.search, "Search buses…", Some("search")) {
        l.ui.scroll.remove(&id_of("bus-grid"));
        l.ui.scroll.remove(&(id_of("bus-grid") ^ 0xabc));
        acted(&mut l.buspick);
    }
    let mut only = l.buspick.only_favourites;
    if l.ui.toggle("bus-only-favourites", Rect::new(row.right() - fav_w, row.y + 7.0, fav_w, 22.0), &mut only, "Favourites only") {
        l.buspick.only_favourites = only;
        l.buspick.rescroll = true;
    }
    let q = display_bus_name(l.buspick.search.trim()).to_lowercase();
    let mut parts = vec![omsi_ui::tr("Buses").into_owned()];
    if !q.is_empty() {
        parts.push(omsi_ui::tr("Search: %{q}").replace("%{q}", l.buspick.search.trim()));
    } else {
        match &l.buspick.level {
            Level::Groups => {}
            Level::Models(g) => parts.push(g.clone()),
            Level::Versions(g, m) => parts.extend([g.clone(), m.clone()]),
        }
    }
    if let Some(k) = crumbs(&mut l.ui, Rect::new(row.x, row.y + 8.0, (search.x - row.x - 24.0).max(60.0), 20.0), &parts) {
        if !q.is_empty() {
            l.buspick.search.clear();
        }
        let level = match (k, l.buspick.level.clone()) {
            (0, _) => Level::Groups,
            (1, Level::Versions(g, _)) => Level::Models(g),
            (_, level) => level,
        };
        go_to(l, level);
    }
    let grid_r = Rect::new(r.x - 4.0, row.bottom() + 14.0, r.w + 8.0, (r.bottom() - row.bottom() - 14.0).max(60.0));
    super::tour::anchor("bus-crumbs", Rect::new(row.x, row.y + 4.0, (search.x - row.x - 24.0).max(60.0), 28.0));
    super::tour::anchor("bus-grid", grid_r);
    l.ui.p().rect(Rect::new(foot.x, row.bottom() + 7.0, foot.w, 1.0), HAIRLINE);
    // the tiles of the level
    if l.buspick.only_favourites && l.buspick.favourites.is_none() {
        l.buspick.favourites = Some(read_favourites());
    }
    let favs_all = l.buspick.favourites.get_or_insert_with(read_favourites).clone();
    let favs = (l.buspick.only_favourites && !favs_all.is_empty()).then_some(&favs_all);
    let chosen = l.state.choice.bus.clone();
    let best = l.buspick.recommend.pick.clone();
    let mut list = tiles(&tree, &l.buspick.level, &q, favs, &chosen, best.as_ref());
    let line = l.buspick.line.buses.clone();
    mark_line(&tree, &mut list, &line, &chosen);
    // (the stars shown are every starred bus's, also with the list not limited to them)
    for t in list.iter_mut() {
        let has = |b: usize| favs_all.contains(&fav_key(&tree.buses[b].file));
        match &t.to {
            To::Bus(b) => {
                t.star = Some(has(*b));
                t.starred = has(*b);
            }
            To::Group(g) => t.starred = tree.group(g).is_some_and(|g| g.models.iter().flat_map(|m| m.buses.iter()).any(|b| has(*b))),
            To::Model(g, m) => t.starred = tree.model(g, m).is_some_and(|m| m.buses.iter().any(|b| has(*b))),
        }
    }
    if list.is_empty() {
        let text = if l.state.loading_content || tree.buses.is_empty() && l.state.vehicles.is_empty() { "Reading the buses…" } else { "No buses found. Try another search." };
        l.ui.text_in(text, Rect::new(grid_r.x, grid_r.y + 30.0, grid_r.w, 24.0), 13.5, Weight::Regular, TEXT_DIM, Align::Center);
    }
    if std::mem::take(&mut l.buspick.rescroll) {
        l.ui.scroll.remove(&id_of("bus-grid"));
        l.ui.scroll.remove(&(id_of("bus-grid") ^ 0xabc));
        if let Some(k) = list.iter().position(|t| t.chosen) {
            let (n, _, _, th) = grid_layout(grid_r.w);
            l.ui.scroll_to("bus-grid", (k / n) as f32 * (th + TILE_GAP), th + 8.0, grid_r.h);
        }
    }
    let keys: Vec<String> = list.iter().map(|t| t.key(&tree)).collect();
    let root = l.state.config.root.clone();
    let paint = l.state.choice.paint.clone();
    let now = l.ui.time;
    let busy = l.showroom.photos.busy_with().and_then(|f| tree.find(f));
    let Launcher { ui, showroom, .. } = l;
    let photos = &mut showroom.photos;
    let click = grid(ui, grid_r, &list, &keys, busy, &mut |t: &Tile| {
        let bus = &tree.buses[t.bus];
        // (the chosen bus in the livery chosen for it, the others in their own)
        let p = if matches!(t.to, To::Bus(_)) && bus.file == chosen { paint.as_str() } else { "" };
        photos.get(&root, &bus.file, p, now)
    });
    match click {
        Some(Click::Open(k)) => match list[k].to.clone() {
            To::Group(g) => go_to(l, Level::Models(g)),
            To::Model(g, m) => go_to(l, Level::Versions(g, m)),
            To::Bus(b) => {
                l.state.select_bus(&tree.buses[b].file);
                l.buspick.level = Level::Versions(tree.buses[b].group.clone(), tree.buses[b].model.clone());
                show_bus(l);
            }
        },
        Some(Click::Star(k)) => toggle_favourite(&mut l.buspick, &tree.buses[list[k].bus].file),
        Some(Click::Look(k)) => {
            let b = list[k].bus;
            l.state.select_bus(&tree.buses[b].file);
            l.buspick.level = Level::Versions(tree.buses[b].group.clone(), tree.buses[b].model.clone());
            show_bus(l);
        }
        None => {}
    }
    // in the foot, on the right: a photo of every bus, made now
    let files: Vec<String> = tree.buses.iter().map(|b| b.file.clone()).collect();
    let progress = l.showroom.photos.progress();
    let label = match progress {
        Some(_) => omsi_ui::tr("Stop").into_owned(),
        None => omsi_ui::tr("Update bus pictures").into_owned(),
    };
    let bw = l.ui.width(&label, 13.0, Weight::Medium) + 46.0;
    let b = Rect::new(foot.right() - 18.0 - bw, foot.y + 7.0, bw, 32.0);
    if let Some((done, total)) = progress {
        let t = omsi_ui::tr("Pictures: %{done} of %{total}").replace("%{done}", &done.to_string()).replace("%{total}", &total.to_string());
        let tw = l.ui.width(&t, 12.5, Weight::Regular);
        l.ui.text_in(&t, Rect::new(b.x - 14.0 - tw, foot.y, tw, foot.h), 12.5, Weight::Regular, TEXT_DIM, Align::Left);
    }
    if l.ui.button("bus-photos", b, &label, Some(if progress.is_some() { "stop_circle" } else { "photo_camera" }), ButtonKind::Ghost) {
        if progress.is_some() {
            l.showroom.photos.stop_all();
        } else {
            l.showroom.photos.queue_all(&root, &files);
        }
    }
    l.ui.tooltip(b, "openOMSI draws a picture of every bus from its own 3D model, so that you see which bus you are choosing. It does them one by one while the launcher is idle.");
}

/// The chosen bus's depot files in the wide sheet: the way back up (the bus's crumbs, then
/// "Depot file"), the map's depot file on the right, and the tiles (`hof::cards`).
fn depot_browse(l: &mut Launcher, r: Rect, foot: Rect) {
    let tree = l.buspick.tree.clone();
    let row = Rect::new(r.x, r.y, r.w, ROW);
    let b = tree.find(&l.state.choice.bus);
    let mut parts = vec![omsi_ui::tr("Buses").into_owned()];
    if let Some(b) = b {
        let bus = &tree.buses[b];
        parts.extend([bus.group.clone(), bus.model.clone(), bus.version.clone()]);
    }
    parts.push(omsi_ui::tr("Depot file").into_owned());
    let line = super::hof::map_line(l).unwrap_or_default();
    let lw = if line.is_empty() { 0.0 } else { l.ui.width(&line, 12.5, Weight::Regular).min(r.w * 0.45) };
    if lw > 0.0 {
        l.ui.icon("map", Vec2::new(row.right() - lw - 12.0, row.y + 18.0), 14.0, TEXT_DIM);
        l.ui.text_in(&line, Rect::new(row.right() - lw, row.y + 8.0, lw, 20.0), 12.5, Weight::Regular, TEXT_DIM, Align::Left);
    }
    if let Some(k) = crumbs(&mut l.ui, Rect::new(row.x, row.y + 8.0, (row.w - lw - 48.0).max(60.0), 20.0), &parts) {
        match (k, b) {
            (1, Some(b)) => go_to(l, Level::Models(tree.buses[b].group.clone())),
            (2, Some(b)) => go_to(l, Level::Versions(tree.buses[b].group.clone(), tree.buses[b].model.clone())),
            (3, Some(_)) => show_bus(l),
            _ => go_to(l, Level::Groups),
        }
        return;
    }
    l.ui.p().rect(Rect::new(foot.x, row.bottom() + 7.0, foot.w, 1.0), HAIRLINE);
    super::hof::cards(l, Rect::new(r.x, row.bottom() + 20.0, r.w, (r.bottom() - row.bottom() - 20.0).max(60.0)));
}

// --- the chosen bus in the showroom ---------------------------------------------------------

/// Beside the bus in the showroom: the way back to the tiles, its star, whether it fits the
/// duty, its livery, depot file, fleet number and plate, and what its maker says of it.
pub(super) fn bus_sheet(l: &mut Launcher, r: Rect) {
    let Some(vehicle) = l.state.bus().cloned() else { return };
    let tree = l.buspick.tree.clone();
    let b = tree.find(&vehicle.file);
    let mut y = r.y;
    // the crumbs, and the star at their end
    if let Some(b) = b {
        let bus = &tree.buses[b];
        let parts = [omsi_ui::tr("Buses").into_owned(), bus.group.clone(), bus.model.clone(), bus.version.clone()];
        if let Some(k) = crumbs(&mut l.ui, Rect::new(r.x, y, r.w - 40.0, 20.0), &parts) {
            let level = match k {
                0 => Level::Groups,
                1 => Level::Models(bus.group.clone()),
                _ => Level::Versions(bus.group.clone(), bus.model.clone()),
            };
            go_to(l, level);
            return;
        }
    }
    let on = l.buspick.favourites.get_or_insert_with(read_favourites).contains(&fav_key(&vehicle.file));
    let sr = Rect::new(r.right() - 28.0, y - 4.0, 28.0, 28.0);
    let (hs, _, cs) = l.ui.interact(id_of("bus-star-chosen"), sr);
    l.ui.icon("star", sr.center(), 18.0, if on { accent() } else if hs { TEXT_SOFT } else { Color::WHITE.alpha(0.25) });
    l.ui.tooltip(sr, if on { "Remove from the favourites" } else { "Add to the favourites" });
    if cs {
        toggle_favourite(&mut l.buspick, &vehicle.file);
    }
    y += 32.0;
    // (the player's line asks for buses of its own: whether this is one)
    if !l.buspick.line.buses.is_empty() {
        let mine = l.buspick.line.buses.contains(&fav_key(&vehicle.file));
        let text = if mine { "One of line %{line}'s buses" } else { "Not one of the buses line %{line} asks for" };
        l.ui.icon(if mine { "check_circle" } else { "info" }, Vec2::new(r.x + 9.0, y + 10.0), 16.0, if mine { accent_2() } else { WARN });
        l.ui.text_in(&omsi_ui::tr(text).replace("%{line}", &l.buspick.line.number), Rect::new(r.x + 26.0, y, r.w - 26.0, 20.0), 12.5, Weight::Medium, if mine { accent_2() } else { WARN }, Align::Left);
        y += 30.0;
    }
    if let Some(p) = l.buspick.recommend.pick.as_ref().filter(|p| p.file == vehicle.file) {
        let line = if p.free { "Drives on this map most" } else { "Fits this duty best" };
        l.ui.icon("check_circle", Vec2::new(r.x + 9.0, y + 10.0), 16.0, OK);
        let text = if p.depot.is_empty() { omsi_ui::tr(line).into_owned() } else { format!("{} · {}", omsi_ui::tr(line), p.depot) };
        l.ui.text_in(&text, Rect::new(r.x + 26.0, y, r.w - 26.0, 20.0), 12.5, Weight::Medium, OK, Align::Left);
        y += 30.0;
    }
    // the livery
    let paints: Vec<String> = std::iter::once(super::drive::default_livery_label(&vehicle).to_string()).chain(vehicle.paints.iter().cloned()).collect();
    let mut paint_sel = vehicle.paints.iter().position(|p| *p == l.state.choice.paint).map(|i| i + 1).unwrap_or(0);
    l.ui.label(Rect::new(r.x, y, r.w, 22.0), "Livery");
    if paints.len() > 1 {
        l.ui.text_in(&format!("{} / {}", paint_sel + 1, paints.len()), Rect::new(r.right() - 70.0, y, 70.0, 22.0), 11.5, Weight::Regular, TEXT_DIM, Align::Right);
    }
    // (a livery of one's own: the livery studio on this bus, from the livery shown)
    if l.ui.icon_button("paint-own", Vec2::new(r.right() - 84.0, y + 11.0), 11.0, "livery_fill", "Paint a livery of your own") {
        let paint = l.state.choice.paint.clone();
        super::livery::open(l, Some(vehicle.file.clone()), Some(paint));
        return;
    }
    y += 28.0;
    let arrows = paints.len() > 1;
    let selector = Rect::new(r.x, y, r.w - if arrows { 88.0 } else { 0.0 }, ROW);
    let mut changed = l.ui.select("paint", selector, &mut paint_sel, &paints);
    if arrows {
        let prev = Rect::new(selector.right() + 8.0, y, ROW, ROW);
        let next = Rect::new(prev.right() + 8.0, y, ROW, ROW);
        if l.ui.button("paint-previous", prev, "", Some("chevron_left"), ButtonKind::Normal) {
            paint_sel = (paint_sel + paints.len() - 1) % paints.len();
            changed = true;
        }
        if l.ui.button("paint-next", next, "", Some("chevron_right"), ButtonKind::Normal) {
            paint_sel = (paint_sel + 1) % paints.len();
            changed = true;
        }
        l.ui.tooltip(prev, "Preview previous livery");
        l.ui.tooltip(next, "Preview next livery");
    }
    if changed {
        l.state.choice.paint = if paint_sel == 0 { String::new() } else { vehicle.paints[paint_sel - 1].clone() };
        l.state.touched();
    }
    y += ROW + 14.0;
    // under the livery (which stays in view: the options say what it gives), in a part of the
    // sheet that scrolls when it holds more than there is room for: the bus options, the
    // depot file (a field that opens the bus's depot files, `hof`), the fleet number and the
    // plate, and what the maker says of the bus; the way back to the tiles under it all
    let back = Rect::new(r.x, r.bottom() - ROW, r.w, ROW);
    let region = Rect::new(r.x - 4.0, y, r.w + 8.0, (back.y - 12.0 - y).max(0.0));
    let id = id_of("bus-sheet");
    let off = l.ui.scroll.get(&id).copied().unwrap_or(0.0);
    // (room for the scroll bar when it scrolls)
    let w = if l.buspick.sheet_content > region.h + 0.5 { r.w - 10.0 } else { r.w };
    let top = region.y + 4.0 - off;
    let mut y = top;
    l.ui.push_clip(region, 0.0);
    let root = l.state.config.root.clone();
    if let Some(cat) = l.state.bus_options.catalogue(&root, &vehicle.file).filter(|c| !c.options.is_empty()) {
        let (picks, paint) = (l.state.bus_options.picks(&vehicle.file), l.state.choice.paint.clone());
        let section = super::busoptions::Section { cat: &cat, livery: &paint, picks: &picks, technical_open: l.state.bus_options.technical_open };
        let (h, edit) = super::busoptions::section(&mut l.ui, r.x, y, w, &section);
        if let Some(e) = edit {
            l.state.edit_bus_options(&vehicle.file, e);
        }
        y += h + 10.0;
        l.ui.p().rect(Rect::new(r.x, y, w, 1.0), HAIRLINE);
        y += 14.0;
    }
    // the font of its destination displays
    {
        let sample = super::displayfont::sample_of(&l.state);
        let (h, edit) = super::displayfont::section(&mut l.ui, r.x, y, w, &mut l.state.display_fonts, &root, &vehicle.file, &sample);
        if let Some(e) = edit {
            l.state.display_fonts.edit(&root, &vehicle.file, e);
        }
        y += h + 6.0;
        l.ui.p().rect(Rect::new(r.x, y, w, 1.0), HAIRLINE);
        y += 14.0;
    }
    let field = |y: f32| Rect::new(r.x + 116.0, y, w - 116.0, ROW);
    let (used, open) = super::hof::field(l, r.x, y, w);
    y += used + 8.0;
    if !vehicle.numbers.is_empty() {
        let options: Vec<String> = vehicle.numbers.iter().map(|(number, plate)| if plate.trim().is_empty() { number.clone() } else { format!("{number}  ({})", plate.trim()) }).collect();
        let mut sel = vehicle.numbers.iter().position(|(number, _)| *number == l.state.choice.number).unwrap_or(0);
        l.ui.label(Rect::new(r.x, y, 110.0, ROW), "Fleet number");
        if l.ui.select("number", field(y), &mut sel, &options) {
            l.state.choice.number = vehicle.numbers[sel].0.clone();
            l.state.touched();
        }
        y += ROW + 8.0;
    }
    let mut plate = l.state.choice.plate.clone();
    l.ui.label(Rect::new(r.x, y, 110.0, ROW), "Number plate");
    if l.ui.text_input("plate", field(y), &mut plate, "Automatic", Some("badge")) {
        l.state.choice.plate = plate;
        l.state.touched();
    }
    y += ROW + 18.0;
    l.ui.p().rect(Rect::new(r.x, y - 8.0, w, 1.0), HAIRLINE);
    y += 4.0;
    if !vehicle.missing_packs.is_empty() {
        let m = omsi_ui::tr("Parts missing: needs %{packs}").replace("%{packs}", &vehicle.missing_packs.join(", "));
        y += l.ui.paragraph(&m, Vec2::new(r.x, y), w, 12.5, Weight::Regular, WARN) + 10.0;
    }
    let description = vehicle.description.replace('\t', " ").lines().map(str::trim).collect::<Vec<_>>().join("\n").trim().to_string();
    if !description.is_empty() {
        y += l.ui.paragraph(&description, Vec2::new(r.x, y), w, 12.0, Weight::Regular, TEXT_DIM) + 10.0;
    }
    y += l.ui.paragraph(&vehicle.file, Vec2::new(r.x, y), w, 10.5, Weight::Regular, TEXT_FAINT);
    l.ui.pop_clip();
    let content = y - top + 12.0;
    l.buspick.sheet_content = content;
    l.ui.scroll_keep("bus-sheet", region, content);
    if open {
        open_depots(l);
        return;
    }
    if l.ui.button("bus-choose-another", back, "Choose another bus", Some("grid_view"), ButtonKind::Normal) {
        let level = b.map(|b| Level::Versions(tree.buses[b].group.clone(), tree.buses[b].model.clone())).unwrap_or_default();
        go_to(l, level);
    }
}

// --- the bus offered ------------------------------------------------------------------------

/// The dialog over the step: which bus openOMSI would take for the duty, and why. Returns
/// Some(true) when it is taken, Some(false) when the player picks one himself.
pub fn offer_dialog(ui: &mut Ui, bus: &str, depot: &str, text: &str) -> Option<bool> {
    let size = ui.size;
    let full = Rect::new(0.0, 0.0, size.x, size.y);
    ui.solid(full);
    ui.p().rect(full, Color::rgba(4, 6, 14, 0.62));
    let w = (size.x - 48.0).min(470.0);
    let th = ui.paragraph_height(text, w - 56.0, 13.5, Weight::Regular);
    let h = 26.0 + 30.0 + 12.0 + 20.0 + 16.0 + th + 26.0 + 44.0 + 26.0;
    let r = Rect::new((size.x - w) * 0.5, (size.y - h) * 0.5, w, h);
    ui.p().shadow(r.inset(-2.0), SHEET_RADIUS, 30.0, Color::rgba(0, 0, 0, 0.5));
    ui.panel(r);
    let x = r.x + 28.0;
    let mut y = r.y + 26.0;
    ui.text_in("This bus is ready for you", Rect::new(x, y, w - 56.0, 30.0), 21.0, Weight::Bold, TEXT, Align::Left);
    y += 42.0;
    let bw = ui.width(bus, 13.5, Weight::Bold).min(w - 56.0);
    ui.text_in(bus, Rect::new(x, y, bw, 20.0), 13.5, Weight::Bold, TEXT_SOFT, Align::Left);
    if !depot.is_empty() {
        ui.text_in(&format!(" · {depot}"), Rect::new(x + bw, y, (w - 56.0 - bw).max(0.0), 20.0), 13.5, Weight::Regular, TEXT_DIM, Align::Left);
    }
    y += 36.0;
    ui.paragraph(text, Vec2::new(x, y), w - 56.0, 13.5, Weight::Regular, TEXT_SOFT);
    let by = r.bottom() - 26.0 - 44.0;
    let take = omsi_ui::tr("Take this one");
    let tw = ui.width(&take, 14.5, Weight::Bold) + 48.0;
    let mine = omsi_ui::tr("Pick one myself");
    let mw = ui.width(&mine, 14.5, Weight::Bold) + 40.0;
    let take_r = Rect::new(r.right() - 28.0 - tw, by, tw, 44.0);
    let mine_r = Rect::new(take_r.x - 12.0 - mw, by, mw, 44.0);
    let keys = ui.input.keys.clone();
    if ui.button("bus-offer-mine", mine_r, &mine, None, ButtonKind::Normal) || keys.contains(&Key::Escape) {
        return Some(false);
    }
    if ui.button("bus-offer-take", take_r, &take, None, ButtonKind::Primary) || keys.contains(&Key::Enter) {
        return Some(true);
    }
    None
}

/// The offer over the step (drawn last, with the mouse and the keys its own) - or, once it is
/// answered, the dialog of a bus without the map's depot file.
pub(super) fn offer(l: &mut Launcher) {
    if !l.buspick.offering {
        if super::hof::dialog(l) {
            open_depots(l);
        }
        return;
    }
    let Some(p) = l.buspick.recommend.pick.clone() else { return };
    let name = l.buspick.tree.find(&p.file).map(|b| l.buspick.tree.buses[b].name.clone()).unwrap_or_else(|| display_bus_name(&p.file));
    let text = match p.fit {
        Some(f) if f >= 0.995 => omsi_ui::tr("It knows every destination of this duty. Take it out, or pick one yourself.").into_owned(),
        Some(f) => omsi_ui::tr("It knows %{percent}% of the destinations of this duty - the best of what is installed. Take it out, or pick one yourself.").replace("%{percent}", &format!("{:.0}", f * 100.0)),
        None => omsi_ui::tr("openOMSI picked it for this duty: the map's own depot runs it. Take it out, or pick one yourself.").into_owned(),
    };
    match offer_dialog(&mut l.ui, &name, &p.depot, &text) {
        Some(true) => {
            l.state.select_bus(&p.file);
            if let Some(b) = l.buspick.tree.find(&p.file) {
                l.buspick.level = Level::Versions(l.buspick.tree.buses[b].group.clone(), l.buspick.tree.buses[b].model.clone());
            }
            show_bus(l);
            log::info!("bus picker: the bus offered was taken ({})", p.file);
        }
        Some(false) => {
            acted(&mut l.buspick);
            // (he picks one himself: the depot file is asked about once he has one in view)
            l.buspick.depots.deferred = true;
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus(maker: &str, type_name: &str, file: &str, paints: usize) -> VehicleInfo {
        VehicleInfo { name: format!("{maker} {type_name}"), manufacturer: maker.into(), type_name: type_name.into(), file: file.into(), folder: file.split('/').nth(1).unwrap_or_default().into(), description: String::new(), default_paint: String::new(), paints: (0..paints).map(|k| format!("P{k}")).collect(), hofs: Vec::new(), installed: false, missing_packs: Vec::new(), numbers: Vec::new() }
    }

    fn fleet() -> Vec<VehicleInfo> {
        vec![
            bus("MAN", "NL202 - 2 Tuerer", "Vehicles/MAN_NL202/NL202_2T.bus", 3),
            bus("MAN", "NL202 - 3 Tuerer", "Vehicles/MAN_NL202/NL202_3T.bus", 0),
            bus("MAN", "SD200", "Vehicles/MAN_SD200/SD200.bus", 1),
            bus("Mercedes-Benz", "O530 - Gelenkbus - 3 Tuerer", "Vehicles/O530/O530G.bus", 2),
            bus("", "", "Vehicles/Solaris/U12.bus", 0),
        ]
    }

    #[test]
    fn the_second_name_line_is_model_and_version() {
        assert_eq!(split_type("Gelenkbus - 18C - 3 Tuerer", "x/y.bus", ""), ("Gelenkbus".into(), "18C · 3 Tuerer".into()));
        assert_eq!(split_type("SD200", "x/SD200_1.bus", "BVG"), ("SD200".into(), "BVG".into()), "no version: the bus's own paint");
        assert_eq!(split_type("SD200", "x/SD200_1.bus", ""), ("SD200".into(), "SD200 1".into()), "nor a paint: the file's name");
    }

    #[test]
    fn a_lines_buses_are_marked_and_put_first() {
        let vehicles = vec![bus("MAN", "NL202 - 2 Tuerer", "Vehicles/MAN_NL202/a.bus", 1), bus("Mercedes-Benz", "Sprinter - City", "Vehicles/Sprinter/s.bus", 1), bus("Solaris", "Urbino 12", "Vehicles/Urbino/u.bus", 1)];
        let t = build_tree(&vehicles, None, &HashSet::new());
        let line: HashSet<String> = [fav_key("Vehicles/Sprinter/s.bus")].into_iter().collect();
        let mut top = tiles(&t, &Level::Groups, "", None, "", None);
        mark_line(&t, &mut top, &line, "");
        // the maker with the line's bus first, marked; the others as they were
        assert_eq!(top.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), ["Mercedes-Benz", "MAN", "Solaris"]);
        assert_eq!(top[0].badge.map(|b| b.0), Some(THIS_LINE));
        assert!(top[1].badge.is_none());
        let mut versions = tiles(&t, &Level::Versions("Mercedes-Benz".into(), "Sprinter".into()), "", None, "", None);
        mark_line(&t, &mut versions, &line, "");
        assert_eq!(versions[0].badge.map(|b| b.0), Some(THIS_LINE));
        // a line that asks for none marks none
        let mut plain = tiles(&t, &Level::Groups, "", None, "", None);
        mark_line(&t, &mut plain, &HashSet::new(), "");
        assert!(plain.iter().all(|x| x.badge.is_none()) && plain[0].title == "MAN");
    }

    #[test]
    fn the_buses_fall_into_makers_models_and_versions() {
        let t = build_tree(&fleet(), None, &HashSet::new());
        let names: Vec<&str> = t.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["MAN", "Mercedes-Benz", "Solaris"], "a bus without a maker goes under its folder");
        let man = t.group("MAN").unwrap();
        assert_eq!(man.models.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["NL202", "SD200"]);
        let nl = &man.models[0];
        assert_eq!(nl.buses.iter().map(|b| t.buses[*b].version.as_str()).collect::<Vec<_>>(), ["2 Tuerer", "3 Tuerer"]);
        assert_eq!(t.buses[0].liveries, 4, "its own paint and its three");
        // (a server's buses alone)
        let allowed: HashSet<String> = ["vehicles/man_sd200/sd200.bus".to_string()].into();
        let t = build_tree(&fleet(), Some(&allowed), &HashSet::new());
        assert_eq!(t.buses.len(), 1);
    }

    #[test]
    fn two_files_of_one_version_are_told_apart_by_their_pack() {
        let mut v = fleet();
        v.push(bus("MAN", "NL202 - 2 Tuerer", "Vehicles/MAN_NL202_Repaint/NL202_2T.bus", 0));
        let t = build_tree(&v, None, &HashSet::new());
        let nl = t.model("MAN", "NL202").unwrap();
        let versions: Vec<&str> = nl.buses.iter().map(|b| t.buses[*b].version.as_str()).collect();
        assert_eq!(versions, ["2 Tuerer · MAN NL202", "2 Tuerer · MAN NL202 Repaint", "3 Tuerer"]);
    }

    #[test]
    fn natural_order_counts_numbers_as_numbers() {
        let mut v = vec!["DL10", "DL9", "dl1", "DL09b"];
        v.sort_by(|a, b| name_cmp(a, b));
        assert_eq!(v, ["dl1", "DL9", "DL09b", "DL10"]);
    }

    #[test]
    fn each_level_has_its_tiles_and_marks_the_chosen_and_the_best() {
        let t = build_tree(&fleet(), None, &HashSet::new());
        let best = Pick { file: "Vehicles/MAN_SD200/SD200.bus".into(), fit: Some(1.0), depot: "Berlin".into(), free: false };
        let chosen = "Vehicles/MAN_NL202/NL202_3T.bus";
        let top = tiles(&t, &Level::Groups, "", None, chosen, Some(&best));
        assert_eq!(top.len(), 3);
        assert!(top[0].chosen && !top[1].chosen);
        assert_eq!(t.buses[top[0].bus].file, chosen, "a maker's tile shows the chosen bus in it");
        assert!(top[0].sub.ends_with("fits best"), "{}", top[0].sub);
        let models = tiles(&t, &Level::Models("MAN".into()), "", None, chosen, Some(&best));
        assert_eq!(models.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), ["NL202", "SD200"]);
        assert_eq!(models[0].sub, "2 versions");
        assert_eq!(models[1].sub, "one version · fits best");
        let versions = tiles(&t, &Level::Versions("MAN".into(), "NL202".into()), "", None, chosen, None);
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].sub, "4 liveries");
        assert_eq!(versions[1].sub, "one livery");
        assert!(versions[1].chosen && versions[1].star == Some(false));
        // a search goes across the levels
        let found = tiles(&t, &Level::Groups, "gelenk", None, chosen, None);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].to, To::Bus(t.find("Vehicles/O530/O530G.bus").unwrap()));
        // the starred ones only (and the chosen one)
        let favs: BTreeSet<String> = ["vehicles/o530/o530g.bus".to_string()].into();
        let top = tiles(&t, &Level::Groups, "", Some(&favs), chosen, None);
        assert_eq!(top.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), ["MAN", "Mercedes-Benz"]);
        assert_eq!(top[0].sub, "one version", "of MAN only the chosen bus");
    }

    fn depot(trips: &[&str], termini: &[&str]) -> Depot {
        Depot { name: "D".into(), trips: trips.iter().map(|s| fold(s)).collect(), termini: termini.iter().map(|s| fold(s)).collect() }
    }

    #[test]
    fn a_depot_knows_a_trip_by_its_name_or_its_terminus() {
        let d = depot(&["4 Liman-ZS"], &["Ahlheim Bf", "Grundorf Kirche"]);
        let trips = vec![("4 liman - zs".to_string(), "Novi Sad".to_string()), ("1 Ahl".to_string(), "Ahlheim, Bf.".to_string()), ("2".to_string(), "Spandau".to_string())];
        assert_eq!(knows(&d, &trips), 2);
        assert_eq!(fold("Straße Süd-Ost"), "strassesudost");
    }

    #[test]
    fn the_map_s_own_fitting_bus_is_offered_before_a_better_stranger() {
        let c = |file: &str, fit: f32, wagons: f32| Candidate { file: file.into(), name: file.into(), fit, depot: format!("{file}.hof"), wagons };
        let list = vec![c("a", 1.0, 0.0), c("b", 0.5, 3.0), c("c", 0.5, 7.0), c("d", 0.0, 20.0)];
        let p = rank(&list, false).unwrap();
        assert_eq!((p.file.as_str(), p.fit), ("c", Some(0.5)), "of the map's own, the one it has most of");
        // none of the map's own fits: the best of the others
        let list = vec![c("a", 0.4, 0.0), c("b", 0.9, 0.0), c("d", 0.0, 20.0)];
        assert_eq!(rank(&list, false).unwrap().file, "b");
        // nothing fits: the map's own most common, said plainly
        let list = vec![c("a", 0.0, 2.0), c("d", 0.0, 20.0)];
        assert_eq!(rank(&list, false).unwrap(), Pick { file: "d".into(), fit: None, depot: String::new(), free: false });
        // a free drive: the bus the map has most of
        assert_eq!(rank(&[c("a", 1.0, 2.0), c("d", 0.0, 20.0)], true).unwrap().file, "d");
        assert_eq!(rank(&[c("a", 0.0, 0.0)], true), None);
    }

    #[test]
    fn the_map_s_fleet_counts_its_depot_s_buses() {
        use omsi_map::ailists::{AiGroup, AiLists, AiTypGroup, AiVehicleEntry, DepotEntry};
        let entry = |n: &str, to: Option<i32>| DepotEntry { number: n.into(), to, ..Default::default() };
        let depot = AiGroup { name: "BVG".into(), hof: Some("Spandau 1986".into()), is_depot: true, typgroups: vec![AiTypGroup { file: "Vehicles\\MAN_SD200\\SD200.bus".into(), entries: vec![entry("1001", None), entry("1002", None), entry("1003", Some(19850101))] }], ..Default::default() };
        let cars = AiGroup { name: "Cars".into(), vehicles: vec![AiVehicleEntry { file: "Vehicles\\Car\\car.ovh".into(), weight: 1.0, number: None, registration: None }], ..Default::default() };
        let ai = AiLists { groups: vec![depot, cars], default_group: 0 };
        assert_eq!(fleet_of(&ai, None).get("vehicles/man_sd200/sd200.bus"), Some(&3.0));
        // (on the day: a bus taken out of service before it is not counted)
        let fleet = fleet_of(&ai, Some(19890530));
        assert_eq!(fleet.get("vehicles/man_sd200/sd200.bus"), Some(&2.0));
        assert_eq!(fleet.get("vehicles/car/car.ovh"), None, "a car group is no depot");
    }

    #[test]
    fn initials_of_a_name() {
        assert_eq!(initials("BHD MAN"), "BM");
        assert_eq!(initials("MAN"), "M");
        assert_eq!(initials("citybus_kajosoft"), "CK");
    }

    fn frame_grid(ui: &mut Ui, tiles: &[Tile], keys: &[String]) -> Option<Click> {
        ui.begin(Vec2::new(1100.0, 700.0), 1.0, 1.0 / 60.0);
        grid(ui, Rect::new(0.0, 0.0, 1100.0, 700.0), tiles, keys, None, &mut |_| None)
    }

    fn click_on(ui: &mut Ui, at: Vec2, tiles: &[Tile], keys: &[String]) -> Option<Click> {
        ui.input.mouse = at;
        ui.input.pressed = true;
        ui.input.down = true;
        frame_grid(ui, tiles, keys);
        ui.input.pressed = false;
        ui.input.down = false;
        ui.input.released = true;
        let c = frame_grid(ui, tiles, keys);
        ui.input.released = false;
        c
    }

    #[test]
    fn a_tile_opens_and_its_star_is_a_switch_of_its_own() {
        let t = build_tree(&fleet(), None, &HashSet::new());
        let list = tiles(&t, &Level::Versions("MAN".into(), "NL202".into()), "", None, "", None);
        let keys: Vec<String> = list.iter().map(|x| x.key(&t)).collect();
        let mut ui = Ui::new();
        frame_grid(&mut ui, &list, &keys);
        let second = *ui.drawn.get(&id_of(&format!("bus-tile-{}", keys[1]))).expect("the second tile is drawn");
        let (n, tw, _, _) = grid_layout(1100.0);
        assert_eq!(n, 4);
        assert!((second.x - (tw + TILE_GAP)).abs() < 0.5, "side by side");
        assert_eq!(click_on(&mut ui, second.center(), &list, &keys), Some(Click::Open(1)));
        let star = *ui.drawn.get(&id_of(&format!("bus-tile-star-{}", keys[0]))).expect("a version has a star");
        assert_eq!(click_on(&mut ui, star.center(), &list, &keys), Some(Click::Star(0)), "the star, not the tile");
        // (a maker's tile has no star to click)
        let top = tiles(&t, &Level::Groups, "", None, "", None);
        let keys: Vec<String> = top.iter().map(|x| x.key(&t)).collect();
        frame_grid(&mut ui, &top, &keys);
        assert!(!ui.drawn.contains_key(&id_of(&format!("bus-tile-star-{}", keys[0]))));
    }

    #[test]
    fn the_crumbs_lead_back_up_but_not_to_where_one_is() {
        let parts = ["Buses".to_string(), "MAN".to_string(), "NL202".to_string()];
        let mut ui = Ui::new();
        ui.begin(Vec2::new(800.0, 100.0), 1.0, 1.0 / 60.0);
        crumbs(&mut ui, Rect::new(0.0, 0.0, 600.0, 20.0), &parts);
        assert!(ui.drawn.contains_key(&id_of("bus-crumb-0")) && ui.drawn.contains_key(&id_of("bus-crumb-1")));
        assert!(!ui.drawn.contains_key(&id_of("bus-crumb-2")), "the last part is where one is");
    }

    #[test]
    fn the_offer_is_taken_or_left() {
        let mut ui = Ui::new();
        for (button, want) in [("bus-offer-take", true), ("bus-offer-mine", false)] {
            ui.begin(Vec2::new(1200.0, 800.0), 1.0, 1.0 / 60.0);
            assert_eq!(offer_dialog(&mut ui, "MAN SD200", "Berlin", "It knows every destination."), None);
            let r = *ui.drawn.get(&id_of(button)).unwrap();
            ui.input.mouse = r.center();
            ui.input.pressed = true;
            ui.input.down = true;
            ui.begin(Vec2::new(1200.0, 800.0), 1.0, 1.0 / 60.0);
            offer_dialog(&mut ui, "MAN SD200", "Berlin", "It knows every destination.");
            ui.input.pressed = false;
            ui.input.down = false;
            ui.input.released = true;
            ui.begin(Vec2::new(1200.0, 800.0), 1.0, 1.0 / 60.0);
            assert_eq!(offer_dialog(&mut ui, "MAN SD200", "Berlin", "It knows every destination."), Some(want));
            ui.input.released = false;
        }
        ui.input.keys.push(Key::Escape);
        ui.begin(Vec2::new(1200.0, 800.0), 1.0, 1.0 / 60.0);
        assert_eq!(offer_dialog(&mut ui, "MAN SD200", "", "x"), Some(false), "Escape leaves it");
    }
}
