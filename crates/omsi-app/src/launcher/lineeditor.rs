//! The line editor: a line of the player's own on any map. The map fills the middle of the
//! page with every bus stop on it; the player clicks the stops in order and the way between
//! each two is found over the roads (`lineroute`) - kerb side, lane changes, loose ends, as the
//! game's buses will drive it. "Reverse" makes the way back from the stops across the road.
//! A leg is dragged through a point of the player's choosing; a stop is put in after the one
//! chosen in the list, moved up or down, or taken out there.
//!
//! On the left the player's lines of the map and the line's own settings (number, name,
//! colour, the depot whose buses drive it); on the right a direction's stops with the minutes
//! to each, its destination, and the timetable: per group of days the first and the last
//! departure, how often, and how long a bus stands at the end - made into tours, a bus going
//! back and forth (`core::lines::blocks`).
//!
//! Saved, the line goes into the registry (`~/.openomsi/lines/<map>.json`, the source of
//! truth) and from there into the map's timetable files (`core::lines::export_to_map`), so
//! that the player drives it from the Drive page like any line and the timetable's buses
//! drive it too.
//!
//! The Kind tab says what kind of service the line is - regular, school transport, weekend
//! and leisure trips, on demand (`core::service`): on which days it runs and, in a company,
//! who pays for it - and which buses run it, by kind (a minibus, a coach…) and by maker,
//! model or version from the installed buses (`busclass`). The game drives the line with such
//! buses where the map's depots have them; the bus step puts them first.
//!
//! Opened from the bus company's Lines page it works for the company (`ForCompany`): the
//! company's map, colours and depot, only its lines; a running estimate over the map of what
//! the line costs once and a month, its passengers through the day and the buses it needs
//! (`core::company::ownline`); and "Confirm and pay", after which the company runs it. A
//! company line changed later pays its route's approval anew.

use super::lineroute::{Anchor, Router};
use super::mapview::{Dot, Look, Pointer};
use super::theme::*;
use super::ui::{ButtonKind, Key, Ui};
use super::Launcher;
use glam::{DVec2, Vec2};
use omsi_launcher_lib as core;
use omsi_launcher_lib::linehof;
use omsi_launcher_lib::owndepot;
use omsi_launcher_lib::company::{self as co, ownline, BusKind, BusSize, Cents, Drive};
use omsi_launcher_lib::lines::{self as reg, Direction, LineDesign, Registry, StopRef, TimeBand, DAY_GROUPS};
use omsi_launcher_lib::service::{self, BusPick, ServiceKind, VehicleClass};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The colours a line may have.
const PALETTE: [&str; 8] = ["#2a75f7", "#e5484d", "#30a46c", "#f5a524", "#8e4ec6", "#12a594", "#e93d82", "#7a8ca6"];
/// How near the mouse must come to a stop, a dragged point or a leg (points).
const STOP_HIT: f32 = 10.0;
const VIA_HIT: f32 = 9.0;
const LEG_HIT: f32 = 6.0;
/// A stop of the same name across the road is at most this far (metres).
const ACROSS: f64 = 150.0;

/// What the mouse is dragging over the map.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    /// A point a leg is dragged through: (leg, its place among the leg's points).
    Via(usize, usize),
    /// A new point, pulled out of a leg.
    New(usize),
}

#[derive(Default)]
pub struct LineEditorView {
    /// The map (an index into the map list) and the map file the registry was read for.
    map: usize,
    loaded: Option<String>,
    map_dir: PathBuf,
    folder: String,
    global: PathBuf,
    reg: Registry,
    reg_path: PathBuf,
    /// The line shown, its direction (0 out, 1 back) and the stop chosen in its list.
    sel: Option<u64>,
    dir: usize,
    sel_stop: Option<usize>,
    /// Changed and not saved.
    dirty: bool,
    /// The right panel: 0 the stops, 1 the displays, 2 the timetable, 3 the kind of service.
    tab: usize,
    /// The map's calendar (`Holidays.txt`), for the school holidays a school line keeps.
    map_calendar: omsi_map::Calendar,
    /// The installed buses by maker and model (for how many buses), and the maker, model and
    /// version chosen to add to the line's buses (each 0: none, or all).
    catalogue: Option<(usize, Arc<Catalogue>)>,
    pick: (usize, usize, usize),
    /// The timetable's table over the map: open, its group of days, its first trip shown,
    /// the cell chosen (a trip of `LineDesign::table`, a stop) and what is typed into it, and
    /// "Fill from the patterns" pressed once.
    table_open: bool,
    table_day: usize,
    table_col: usize,
    table_sel: Option<(usize, usize)>,
    table_text: String,
    table_fill_armed: bool,
    /// A destination of the player's own being typed (the Displays tab).
    dest_new: String,
    /// The stops' names from the map's `Busstops.cfg`.
    names: HashMap<i64, String>,
    /// The depot groups with buses (`ailists.cfg`) and each one's depot file.
    groups: Vec<(String, String)>,
    /// The depot files of the depot groups as the line editor sees them, without its own
    /// block (by name, lowercased; read once) - and the player's own (`own:` and its key).
    depots: HashMap<String, Option<Arc<linehof::Depot>>>,
    /// The player's own depot files for the map (`owndepot`), read once (again after one
    /// changed), and the destinations of them offered as "Mine" (for the line's depot file).
    own: Option<Vec<owndepot::Entry>>,
    mine: Option<(String, Vec<reg::OwnDestination>)>,
    kept: Option<Kept>,
    /// What moving the player's own destinations into his depot file came to (said once).
    note: Option<(String, bool)>,
    /// "Open in the depot editor" was pressed: the depot editor next, on that file.
    go_depots: Option<String>,
    /// The sign's preview lit white rather than amber.
    white: bool,
    /// The picker for any colour of the line open; the display texts the depot file leaves
    /// empty shown too.
    colour_open: bool,
    more_texts: bool,
    /// The router of the map's network (the network's address, to see it is still that one),
    /// and the map's stops as the line editor offers them.
    router: Option<(usize, Router)>,
    stops: Vec<StopRef>,
    /// Each direction's legs as drawn (world points), and for which line they were made.
    shapes: [Vec<Vec<DVec2>>; 2],
    shapes_for: Option<u64>,
    /// Bumped whenever what the map draws of the line changes.
    revision: u64,
    drag: Option<Drag>,
    delete_armed: bool,
    hover_stop: Option<usize>,
    /// "Drive it" was pressed: the Drive page next.
    go_drive: bool,
    /// Working for a bus company (its Lines page opened the editor).
    company: Option<ForCompany>,
    /// What the company's buttons asked for (done after the panels).
    company_act: Option<CompanyAct>,
    /// A size of bus the company's level keeps locked was asked for: its popup (after the
    /// panels).
    level_ask: Option<BusSize>,
    /// A change of a line in service asked about (`change_dialog`).
    change_ask: Option<ChangeAsk>,
}

/// Where the shown line's own destinations are kept (`LineEditorView::kept`): for which line
/// (its depot group and depot file), the player's depot file (its place and name), its
/// destinations the map's depot file lacks, and the names of all of them (lowercased).
struct Kept {
    for_line: String,
    target: Option<(PathBuf, String)>,
    mine: Vec<reg::OwnDestination>,
    names: std::collections::HashSet<String>,
}

/// What the line editor keeps while it works for a bus company.
pub struct ForCompany {
    id: String,
    name: String,
    map: String,
    colours: [String; 2],
    /// The depot file its buses carry (the depot group of that file is the line's).
    depot: String,
    /// What to show once the map is read: a new line (None), or the company's line.
    start: Option<Option<u64>>,
    /// The other lines at the map's stops, for the line shown (its own left out).
    stop_lines: Option<(u64, ownline::StopLines)>,
    /// The figures of the line shown, for its revision, and the estimate's details open.
    cache: Option<Figures>,
    details: bool,
}

/// The company's figures of the line shown.
#[derive(Clone)]
struct Figures {
    key: (u64, u64, Cents, usize),
    shape: ownline::Shape,
    est: ownline::Estimate,
    /// The company runs it already, and what saving it now costs.
    confirmed: bool,
    fee: Option<Cents>,
    cash: Cents,
    /// What the company paid to start it (and to change it since).
    paid: Cents,
}

/// A change of a company's line in service, asked about before it is saved: what it costs,
/// the tours it touches, and from which day (tomorrow at the soonest: today's tours run as
/// they are).
#[derive(Clone)]
struct ChangeAsk {
    fee: Cents,
    diff: reg::TourDiff,
    today: String,
    from: String,
}

#[derive(Clone, Debug, PartialEq)]
enum CompanyAct {
    Confirm,
    SaveChange,
    /// The change asked about saved for its day.
    Schedule(String),
    /// A change waiting for its day taken back: the line stays as it runs.
    Withdraw,
    Discard,
}

/// The sizes a time band can ask for ("auto" first: the demand model's choice).
const SIZES: [Option<BusSize>; 5] = [None, Some(BusSize::Midi), Some(BusSize::Solo), Some(BusSize::Articulated), Some(BusSize::Double)];

fn size_label(s: Option<BusSize>) -> String {
    match s {
        None => omsi_ui::tr("Auto").into_owned(),
        Some(size) => omsi_ui::tr(BusKind { size, drive: Drive::Diesel }.label()).into_owned(),
    }
}

/// Maps named alike (`maps/Grundorf/global.cfg`, either slash, any case).
fn same_map(a: &str, b: &str) -> bool {
    let n = |s: &str| s.trim().replace('\\', "/").to_lowercase();
    n(a) == n(b)
}

/// The line editor opened for the bus company: its map, a new line of its own (`line`
/// None) or one of its lines.
pub fn open_for_company(l: &mut Launcher, line: Option<u64>) {
    let Some(c) = l.company.company.as_ref() else { return };
    let v = &mut l.pages.lines;
    v.company = Some(ForCompany { id: c.id.clone(), name: c.name.clone(), map: c.map.clone(), colours: c.colours.clone(), depot: c.depot.clone(), start: Some(line), stop_lines: None, cache: None, details: false });
    v.company_act = None;
    v.loaded = None;
    l.go(super::Page::Lines);
}

fn colour_of(hex: &str) -> Color {
    let h = hex.trim().trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).unwrap_or(0x2a75f7);
    Color::rgba((v >> 16) as u8, (v >> 8) as u8, v as u8, 1.0)
}

fn dv(a: [f64; 2]) -> DVec2 {
    DVec2::new(a[0], a[1])
}

/// The ground points of a leg the router could not route: straight through its points.
fn straight(d: &Direction, k: usize) -> Vec<DVec2> {
    let mut v = vec![dv(d.stops[k].at)];
    v.extend(d.legs[k].vias.iter().map(|p| dv(*p)));
    v.push(dv(d.stops[k + 1].at));
    v
}

/// Route every leg of a direction again (each from where the one before ended), its times
/// with them; returns the legs as drawn.
fn route_direction(router: &Router, d: &mut Direction) -> Vec<Vec<DVec2>> {
    d.fit_legs();
    let mut shapes = Vec::new();
    let mut start: Option<Anchor> = None;
    for k in 0..d.legs.len() {
        let (a, b) = (dv(d.stops[k].at), dv(d.stops[k + 1].at));
        let vias: Vec<DVec2> = d.legs[k].vias.iter().map(|p| dv(*p)).collect();
        // (from where the leg before ended; failing that, from any lane of the stop)
        let r = router.leg(a, b, &vias, start).or_else(|| start.and_then(|_| router.leg(a, b, &vias, None)));
        let leg = &mut d.legs[k];
        match r {
            Some(r) => {
                leg.steps = router.steps(&r);
                leg.length = r.length;
                leg.ok = !leg.steps.is_empty();
                (leg.from_s, leg.to_s, leg.from_lat, leg.to_lat) = (r.from.s, r.to.s, r.from.lateral.abs(), r.to.lateral.abs());
                shapes.push(router.points(&r.lanes, r.from.s, r.to.s));
                start = Some(Anchor { beside: None, ..r.to });
            }
            None => {
                leg.steps.clear();
                leg.ok = false;
                leg.length = (b - a).length() as f32;
                shapes.push(straight(d, k));
                start = None;
            }
        }
    }
    d.refresh_times();
    shapes
}

/// A saved direction's legs as drawn, from the lanes it keeps.
fn stored_shapes(router: &Router, d: &Direction) -> Vec<Vec<DVec2>> {
    (0..d.legs.len().min(d.stops.len().saturating_sub(1)))
        .map(|k| {
            let g = &d.legs[k];
            let lanes = router.lanes_of(&g.steps);
            if g.ok && !lanes.is_empty() {
                router.points(&lanes, g.from_s, g.to_s)
            } else {
                straight(d, k)
            }
        })
        .collect()
}

/// The distance from `p` to a polyline (all in picture points).
fn to_polyline(p: Vec2, pts: &[Vec2]) -> f32 {
    let mut best = f32::MAX;
    for w in pts.windows(2) {
        let ab = w[1] - w[0];
        let t = ((p - w[0]).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
        best = best.min((w[0] + ab * t - p).length());
    }
    best
}

impl LineEditorView {
    /// It works for a bus company.
    pub fn for_company(&self) -> bool {
        self.company.is_some()
    }

    /// Back to the player's own lines (the next draw reads the map again).
    pub fn leave_company(&mut self) {
        if self.company.take().is_some() {
            self.loaded = None;
            self.company_act = None;
        }
    }

    /// A line the list shows: the company's when working for one, else the player's own.
    fn shows(&self, l: &LineDesign) -> bool {
        match &self.company {
            Some(fc) => l.company == fc.id,
            None => l.company.is_empty(),
        }
    }

    fn first_shown(&self) -> Option<u64> {
        self.reg.lines.iter().find(|l| self.shows(l)).map(|l| l.id)
    }

    fn line(&self) -> Option<&LineDesign> {
        self.sel.and_then(|id| self.reg.line(id))
    }

    fn line_mut(&mut self) -> Option<&mut LineDesign> {
        let id = self.sel?;
        self.reg.line_mut(id)
    }

    /// Something of the line changed: unsaved, and drawn again.
    fn touched(&mut self) {
        self.dirty = true;
        self.revision += 1;
    }

    /// The direction `dir` of the shown line routed again.
    fn reroute(&mut self, dir: usize) {
        let Some((_, router)) = self.router.as_ref() else { return };
        let Some(id) = self.sel else { return };
        let Some(l) = self.reg.lines.iter_mut().find(|l| l.id == id) else { return };
        if let Some(d) = l.directions.get_mut(dir) {
            self.shapes[dir] = route_direction(router, d);
        }
        self.touched();
    }

    /// Read the map's registry, names and depot groups (another map was chosen).
    fn load_map(&mut self, file: &str, root: &Path, date: &str) {
        self.loaded = Some(file.to_string());
        self.global = omsi_cfg::find_in_roots(file).map(|(_, p)| p).unwrap_or_else(|| omsi_cfg::resolve_path(root, file));
        self.map_dir = self.global.parent().map(Path::to_path_buf).unwrap_or_default();
        self.folder = reg::map_folder(file);
        self.reg_path = reg::registry_path(&self.folder);
        self.reg = reg::load_registry(&self.reg_path);
        self.reg.map = self.folder.clone();
        self.reg.global = file.to_string();
        self.sel = self.first_shown();
        (self.dir, self.sel_stop, self.dirty, self.delete_armed, self.drag) = (0, None, false, false, None);
        self.shapes_for = None;
        self.router = None;
        self.stops.clear();
        self.map_calendar = omsi_map::Calendar::load(&self.map_dir.join("Holidays.txt")).unwrap_or_default();
        self.names = omsi_timetable::TimetableData::load(&self.map_dir).bus_stops.into_iter().filter(|b| !b.name.trim().is_empty()).map(|b| (b.object_id, b.name.trim().to_string())).collect();
        let chrono = omsi_map::date_code(date).map(|c| omsi_map::active_chrono_dirs(&self.map_dir, c)).unwrap_or_default();
        let ai = omsi_map::ailists::ailists_with_chrono(&self.map_dir, &chrono);
        self.groups = ai.groups.iter().filter(|g| g.is_depot && !g.typgroups.is_empty() && !g.name.trim().is_empty()).map(|g| (g.name.trim().to_string(), g.hof.clone().unwrap_or_default())).collect();
        self.depots.clear();
        self.own_changed();
        self.revision += 1;
        // the player's own destinations of before the depot editor: into his depot file of the
        // map (made from the map's), one list
        if !self.reg.destinations.is_empty() {
            let base = self.map_depot_bytes(file);
            let name = omsi_ui::tr("%{map} - mine").replace("%{map}", &self.folder);
            self.note = Some(match owndepot::migrate(&mut self.reg, &owndepot::dir(), &name, base.as_deref()) {
                Ok(Some((path, n))) => {
                    let _ = reg::save_registry(&self.reg_path, &self.reg);
                    let name = owndepot::load(&path).map(|l| l.doc.hof.name).unwrap_or(name);
                    (omsi_ui::tr("Your %{n} own destination(s) are in your depot file %{name} now: Editor, Depot editor").replace("%{n}", &n.to_string()).replace("%{name}", &name), false)
                }
                Ok(None) => (String::new(), false),
                Err(e) => (format!("{}: {e}", omsi_ui::tr("Your own destinations could not be moved into a depot file")), true),
            });
            self.own_changed();
        }
    }

    /// The bytes of the map's depot file (the first depot group's that names one), as the
    /// player's own depot file starts from: the copy most buses carry.
    fn map_depot_bytes(&self, map_file: &str) -> Option<Vec<u8>> {
        let hof = self.groups.iter().map(|g| g.1.trim()).find(|h| !h.is_empty())?;
        let found = core::depot::find_sources(hof, map_file);
        core::depot::best(&found).and_then(|i| omsi_cfg::vfs::read(&found[i].path).ok())
    }

    /// A depot file of the player's changed (or might have): read again, and the bus step's
    /// tiles with them.
    pub(super) fn own_changed(&mut self) {
        self.own = None;
        self.mine = None;
        self.kept = None;
        self.depots.retain(|k, _| !k.starts_with("own:"));
    }

    /// The player's own depot files for the map.
    fn own_entries(&mut self) -> &[owndepot::Entry] {
        let folder = self.folder.clone();
        self.own.get_or_insert_with(|| owndepot::for_map(&owndepot::list(&owndepot::dir()), &folder))
    }

    /// The depot file of the shown line as the line editor sees it: the player's own when the
    /// line chose one, else its depot group's.
    fn line_depot(&mut self) -> Option<Arc<linehof::Depot>> {
        let (group, key) = self.line().map(|l| (l.ai_group.clone(), l.depot_file.trim().to_string()))?;
        if let Some(path) = (!key.is_empty()).then(|| owndepot::path_of(&owndepot::dir(), &key)).flatten() {
            let folder = self.folder.clone();
            return self.depots.entry(format!("own:{}", key.to_lowercase())).or_insert_with(|| owndepot::depot_at(&path, &folder).map(Arc::new)).clone();
        }
        self.depot_of(&group)
    }

    /// The destinations of the player's own depot files the shown line's depot file lacks:
    /// what the line editor offers as "Mine" (`owndepot::own_destinations`).
    fn mine(&mut self) -> Vec<reg::OwnDestination> {
        let for_line = self.line().map(|l| format!("{}|{}", l.ai_group.to_lowercase(), l.depot_file.trim().to_lowercase())).unwrap_or_default();
        if let Some((_, m)) = self.mine.as_ref().filter(|(k, _)| *k == for_line) {
            return m.clone();
        }
        let known: std::collections::HashSet<String> = self.line_depot().map(|d| d.hof.termini.iter().map(|t| t.texture_id.trim().to_lowercase()).collect()).unwrap_or_default();
        let m = owndepot::own_destinations(self.own_entries(), &known);
        self.mine = Some((for_line, m.clone()));
        m
    }

    /// Where the shown line's own destinations are kept (the Displays tab): the player's depot
    /// file the line chose, else the first for the map (None: there is none yet) - worked out
    /// once (again after a change).
    fn kept(&mut self) -> &Kept {
        let (group, key) = self.line().map(|l| (l.ai_group.clone(), l.depot_file.trim().to_string())).unwrap_or_default();
        let for_line = format!("{}|{}", group.to_lowercase(), key.to_lowercase());
        if self.kept.as_ref().is_none_or(|k| k.for_line != for_line) {
            let map_known: std::collections::HashSet<String> = self.depot_of(&group).map(|d| d.hof.termini.iter().map(|t| t.texture_id.trim().to_lowercase()).collect()).unwrap_or_default();
            let own = self.own_entries();
            let target = own.iter().find(|e| !key.is_empty() && e.key.eq_ignore_ascii_case(&key)).or_else(|| own.iter().find(|e| !e.map.trim().is_empty())).or(own.first());
            let kept = Kept {
                for_line,
                target: target.map(|e| (e.path.clone(), e.name.clone())),
                mine: target.map(|e| e.termini.iter().filter(|t| !t.name.trim().is_empty() && !map_known.contains(&t.name.trim().to_lowercase())).cloned().collect()).unwrap_or_default(),
                names: target.map(|e| e.termini.iter().map(|t| t.name.trim().to_lowercase()).collect()).unwrap_or_default(),
            };
            self.kept = Some(kept);
        }
        self.kept.as_ref().unwrap()
    }

    /// The depot file a depot group names (`ailists.cfg`).
    fn hof_of(&self, group: &str) -> String {
        self.groups.iter().find(|g| g.0.eq_ignore_ascii_case(group)).map(|g| g.1.trim().to_string()).unwrap_or_default()
    }

    /// A depot group's depot file as the line editor sees it - without the block it wrote,
    /// so that a destination it added is still a new one (read once).
    fn depot_of(&mut self, group: &str) -> Option<Arc<linehof::Depot>> {
        let hof = self.hof_of(group);
        if hof.is_empty() {
            return None;
        }
        let folder = self.folder.clone();
        self.depots
            .entry(hof.to_lowercase())
            .or_insert_with(|| {
                let (bases, _) = core::depot_roots();
                linehof::first_depot(&hof, &bases, &folder).map(Arc::new)
            })
            .clone()
    }

    /// The termini of the shown line's depot file: (what the trip names, what the display says).
    fn termini(&mut self) -> Vec<(String, String)> {
        let Some(d) = self.line_depot() else { return Vec::new() };
        d.hof
            .termini
            .iter()
            .filter(|t| !t.all_exit && !t.texture_id.trim().is_empty())
            .map(|t| (t.texture_id.trim().to_string(), t.strings.first().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| t.texture_id.trim().to_string())))
            .collect()
    }

    /// Before the registry is saved: the codes of the lines in their depot files given - the
    /// map's and the player's own they chose (`owndepot::assign`) - and what is to be written
    /// there (`core::linehof::prepare`).
    fn prepare_depots(&mut self) -> (HashMap<String, String>, linehof::Plan, Option<PathBuf>) {
        let groups = linehof::group_depots(&self.map_dir);
        let (bases, original) = core::depot_roots();
        let plan = linehof::prepare(&mut self.reg, &groups, &bases);
        owndepot::assign(&mut self.reg, &owndepot::dir(), &groups, &bases);
        (groups, plan, original)
    }

    /// The depot files written (after the registry and the timetable): the destinations,
    /// routes and stops of the lines for the displays and the IBIS - in the map's depot files
    /// and in the player's own the lines chose (and the copies of those beside buses). The
    /// first error.
    fn write_depots(&mut self, content: &Path, prepared: &(HashMap<String, String>, linehof::Plan, Option<PathBuf>)) -> Option<String> {
        let (groups, plan, original) = prepared;
        let (n, err) = linehof::write(content, original.as_deref(), &self.reg, groups, plan);
        let (own, own_err) = owndepot::write_lines(&self.reg, &owndepot::dir(), Some(content));
        log::info!("line editor: {n} depot file(s) of the map's and {own} of the player's own written for the displays and the IBIS");
        self.depots.clear();
        self.own_changed();
        err.or(own_err)
    }
}

pub fn draw(l: &mut Launcher, area: Rect) {
    // (working for the company: its popups over the editor, and the question when a change
    // of a line in service begins)
    if l.pages.lines.company.is_some() {
        super::company::over_line_editor(l, |l| {
            if l.pages.lines.change_ask.is_some() {
                let i = super::company::mask(&mut l.ui);
                draw_page(l, area);
                l.ui.input = i;
                change_dialog(l, area);
            } else {
                draw_page(l, area);
            }
        });
    } else {
        draw_page(l, area);
    }
}

/// The question before a change of a line in service is saved: from which day, what it
/// costs, and which tours get their buses and drivers anew then.
fn change_dialog(l: &mut Launcher, area: Rect) {
    let Some(mut ask) = l.pages.lines.change_ask.clone() else { return };
    let number = l.pages.lines.line().map(|x| x.number.trim().to_string()).unwrap_or_default();
    let ui = &mut l.ui;
    ui.p().rect(area, Color::rgba(0, 0, 0, 0.45));
    let w = 560.0f32.min(area.w - 32.0);
    let h = 400.0;
    let r = Rect::new(area.center().x - w * 0.5, area.center().y - h * 0.5, w, h);
    ui.panel(r);
    let x = r.x + 24.0;
    let iw = w - 48.0;
    let mut y = r.y + 20.0;
    ui.icon("route", Vec2::new(x + 10.0, y + 12.0), 20.0, accent_2());
    ui.text_in(&omsi_ui::tr("Change line %{n} in service").replace("%{n}", &number), Rect::new(x + 30.0, y, iw - 30.0, 24.0), 17.0, Weight::Bold, TEXT, Align::Left);
    y += 36.0;
    y += ui.paragraph("The line runs: today's tours stay as they are. The new timetable begins on the day you choose; the tours it changes get their buses and drivers anew then.", Vec2::new(x, y), iw, 12.5, Weight::Regular, TEXT_SOFT) + 14.0;
    let row = |ui: &mut Ui, y: f32, label: &str, value: &str, c: Color| {
        ui.text_in(&omsi_ui::tr(label), Rect::new(x, y, iw * 0.45, 22.0), 13.0, Weight::Medium, TEXT_SOFT, Align::Left);
        ui.text_in(value, Rect::new(x + iw * 0.45, y, iw * 0.55, 22.0), 13.0, Weight::Bold, c, Align::Right);
        ui.p().rect(Rect::new(x, y + 26.0, iw, 1.0), HAIRLINE);
    };
    let fee = if ask.fee > 0 { super::company::eur(ask.fee) } else { omsi_ui::tr("none").into_owned() };
    row(ui, y, "Route approved anew", &fee, TEXT);
    y += 34.0;
    let d = &ask.diff;
    let tours = omsi_ui::tr("%{k} kept, %{c} changed, %{a} new, %{g} dropped").replace("%{k}", &d.kept.len().to_string()).replace("%{c}", &d.changed.len().to_string()).replace("%{a}", &d.added.len().to_string()).replace("%{g}", &d.gone.len().to_string());
    row(ui, y, "Tours", &tours, TEXT);
    y += 34.0;
    let replan = d.replan().len() + d.added.len();
    let words = omsi_ui::tr("%{n} tours to plan anew").replace("%{n}", &replan.to_string());
    row(ui, y, "On the planning", &words, if replan > 0 { WARN } else { TEXT });
    y += 42.0;
    // from which day: tomorrow at the soonest, two months at the most
    ui.text_in(&omsi_ui::tr("From"), Rect::new(x, y, iw * 0.3, 32.0), 13.0, Weight::Medium, TEXT_SOFT, Align::Left);
    let days = co::dates::between(&ask.today, &ask.from);
    let dx = x + iw - 250.0;
    if ui.icon_button("le-change-earlier", Vec2::new(dx + 14.0, y + 16.0), 14.0, "chevron_left", "A day earlier") && days > 1 {
        ask.from = co::dates::add(&ask.from, -1);
    }
    let when = if days == 1 { format!("{}  ({})", super::company::day_label(&ask.from), omsi_ui::tr("tomorrow")) } else { super::company::day_label(&ask.from) };
    ui.text_in(&when, Rect::new(dx + 32.0, y, 186.0, 32.0), 14.0, Weight::Bold, TEXT, Align::Center);
    if ui.icon_button("le-change-later", Vec2::new(dx + 236.0, y + 16.0), 14.0, "chevron_right", "A day later") && days < 60 {
        ask.from = co::dates::add(&ask.from, 1);
    }
    let by = r.bottom() - 20.0 - ROW;
    let bw = (iw - GAP) * 0.5;
    let label = if ask.fee > 0 { omsi_ui::tr("Pay %{amount} and save").replace("%{amount}", &super::company::eur(ask.fee)) } else { omsi_ui::tr("Save the change").into_owned() };
    let go = ui.button("le-change-save", Rect::new(x + bw + GAP, by, bw, ROW), &label, Some("check_circle"), ButtonKind::Primary);
    let cancel = ui.button("le-change-cancel", Rect::new(x, by, bw, ROW), "Cancel", None, ButtonKind::Normal) || ui.input.keys.contains(&Key::Escape);
    let v = &mut l.pages.lines;
    if cancel {
        v.change_ask = None;
    } else if go {
        v.change_ask = None;
        v.company_act = Some(CompanyAct::Schedule(ask.from.clone()));
    } else {
        v.change_ask = Some(ask);
    }
}

fn draw_page(l: &mut Launcher, area: Rect) {
    let maps: Vec<(String, String)> = l.state.maps.iter().map(|m| (m.friendly.clone(), m.file.clone())).collect();
    if maps.is_empty() {
        l.ui.paragraph("No maps found.", Vec2::new(area.x, area.y), area.w, 14.0, Weight::Regular, TEXT_DIM);
        return;
    }
    {
        let v = &mut l.pages.lines;
        // (working for the bus company: its map, and no other)
        let company_map = v.company.as_ref().map(|f| f.map.clone());
        if let Some(cm) = company_map {
            match maps.iter().position(|m| same_map(&m.1, &cm)) {
                Some(i) => v.map = i,
                None => {
                    v.leave_company();
                    l.state.set_status(omsi_ui::tr("The company's map is not installed.").into_owned(), true);
                    return;
                }
            }
        } else if v.loaded.is_none() {
            v.map = maps.iter().position(|m| m.1 == l.state.choice.map).unwrap_or(0);
        }
        v.map = v.map.min(maps.len() - 1);
        if v.loaded.as_deref() != Some(maps[v.map].1.as_str()) {
            let root = PathBuf::from(&l.state.config.root);
            v.load_map(&maps[v.map].1.clone(), &root, &l.state.choice.date);
        }
    }
    company_start(l);
    company_figures(l);
    // (a phone, or a window too narrow for the map and both panels: the lines only)
    if area.w < 900.0 {
        narrow(l, area);
        return;
    }
    let side = (area.w * 0.25).clamp(280.0, 340.0);
    let left = Rect::new(area.x, area.y, side, area.h);
    let right = Rect::new(area.right() - side, area.y, side, area.h);
    let map_r = Rect::new(left.right() + GAP, area.y, right.x - left.right() - 2.0 * GAP, area.h);
    // the map: the chosen one, nothing of a duty on it
    let file = maps[l.pages.lines.map].1.clone();
    let look = Look { map: file.clone(), global: l.pages.lines.global.clone(), date: l.state.choice.date.clone(), trips: Vec::new(), entry: -1 };
    l.mapview.want(look);
    l.map_background(map_r);
    // (the sheet under the page counts as interface everywhere: only what lies over the map
    // - the panels' lists, its buttons - keeps the mouse from it)
    l.ui.over_ui = false;
    take_network(l);
    let mut status: Option<(String, bool)> = None;
    left_panel(l, left, &maps, &mut status);
    right_panel(l, right, &mut status);
    if l.pages.lines.table_open {
        table_layer(l, map_r);
    } else {
        map_layer(l, map_r);
        map_buttons(l, map_r);
        estimate_card(l, map_r);
        map_pointer(l, map_r);
    }
    l.ui.over_ui = true;
    if let Some(act) = l.pages.lines.company_act.take() {
        company_action(l, act, &mut status);
    }
    if status.is_none() {
        status = l.pages.lines.note.take();
    }
    if let Some(size) = l.pages.lines.level_ask.take() {
        super::company::size_locked(l, size);
    }
    if let Some((s, e)) = status.filter(|s| !s.0.is_empty()) {
        l.state.set_status(s, e);
    }
    if let Some(key) = l.pages.lines.go_depots.take() {
        l.pages.depots.show(&key, true);
        l.go(super::Page::Depots);
        return;
    }
    if std::mem::take(&mut l.pages.lines.go_drive) {
        l.go(super::Page::Drive);
        l.drive.step = super::flow::Step::Start;
    }
}

/// The map's network once it is read: the router over it and the stops with their names; the
/// shown line's legs as drawn.
fn take_network(l: &mut Launcher) {
    let Some((net, stops, left_hand)) = l.mapview.network() else { return };
    let v = &mut l.pages.lines;
    let key = Arc::as_ptr(&net) as usize;
    if v.router.as_ref().map(|r| r.0) != Some(key) {
        let router = Router::new(net, left_hand);
        log::info!("line editor: {} stops on the map, {} loose lane ends joined", stops.len(), router.joined());
        v.stops = stops
            .iter()
            .map(|s| {
                let name = v.names.get(&s.id).cloned().filter(|n| !n.is_empty()).or_else(|| Some(s.name.clone()).filter(|n| !n.is_empty())).unwrap_or_else(|| format!("Stop {}", s.id));
                StopRef { tile: [s.tile.0, s.tile.1], id: s.id, name, at: [s.at.x, s.at.y], ..Default::default() }
            })
            .collect();
        v.router = Some((key, router));
        v.shapes_for = None;
    }
    if v.shapes_for != v.sel {
        v.shapes_for = v.sel;
        v.shapes = Default::default();
        if let (Some((_, router)), Some(line)) = (v.router.as_ref(), v.sel.and_then(|id| v.reg.line(id))) {
            for (k, d) in line.directions.iter().enumerate().take(2) {
                v.shapes[k] = stored_shapes(router, d);
            }
        }
        v.revision += 1;
    }
}

// --- the left panel: the lines and the line's settings ---------------------------------------

fn left_panel(l: &mut Launcher, r: Rect, maps: &[(String, String)], status: &mut Option<(String, bool)>) {
    let figures = l.pages.lines.company.as_ref().and_then(|f| f.cache.clone()).filter(|f| Some(f.key.0) == l.pages.lines.sel);
    let Launcher { ui, pages, state, .. } = l;
    let v = &mut pages.lines;
    ui.card(r);
    let inner = Rect::new(r.x + 16.0, r.y + 12.0, r.w - 32.0, r.h - 24.0);
    let body = ui.heading(inner, "Your lines", Some("route"));
    let names: Vec<String> = maps.iter().map(|m| m.0.clone()).collect();
    let mut m = v.map;
    if let Some(fc) = v.company.as_ref() {
        // (the company's map: its lines are made there)
        let text = format!("{}  ·  {}", fc.name, names.get(v.map).cloned().unwrap_or_default());
        ui.p().rounded(Rect::new(body.x, body.y, body.w, ROW), RADIUS, FIELD);
        ui.icon("garage", Vec2::new(body.x + 18.0, body.y + ROW * 0.5), 17.0, accent_2());
        ui.text_in(&text, Rect::new(body.x + 36.0, body.y, body.w - 44.0, ROW), 13.0, Weight::Medium, TEXT, Align::Left);
    } else if ui.select("le-map", Rect::new(body.x, body.y, body.w, ROW), &mut m, &names) && m != v.map {
        if v.dirty {
            *status = Some(("The changes to the line on the map before were not saved".into(), true));
        }
        v.map = m;
        v.loaded = None;
        return;
    }
    // the lines of the map
    let list_y = body.y + ROW + 10.0;
    let rows: Vec<(u64, String, String, Color, bool, ServiceKind)> = v.reg.lines.iter().filter(|x| v.shows(x)).map(|x| (x.id, x.number.clone(), x.name.clone(), colour_of(&x.colour), x.draft && v.company.is_some(), x.service)).collect();
    let list_h = ((rows.len().max(1) as f32) * 40.0).min(r.h * 0.28);
    let sel = v.sel;
    let mut pick = None;
    ui.scroll_area("le-lines", Rect::new(body.x - 6.0, list_y, body.w + 12.0, list_h), &mut |ui, a| {
        for (i, (id, number, name, c, draft, kind)) in rows.iter().enumerate() {
            let rr = Rect::new(a.x + 6.0, a.y + i as f32 * 40.0, a.w - 12.0, 36.0);
            if ui.row(&format!("le-line-{id}"), rr, Some(*id) == sel) {
                pick = Some(*id);
            }
            let plate = Rect::new(rr.x + 8.0, rr.y + 7.0, (ui.width(number, 13.0, Weight::Black) + 16.0).max(40.0), 22.0);
            ui.p().rounded(plate, 5.0, LINE);
            ui.text_in(number, plate, 13.0, Weight::Black, ON_LINE, Align::Center);
            ui.p().rounded(Rect::new(plate.right() + 8.0, rr.y + 12.0, 4.0, 12.0), 2.0, *c);
            let badge_w = if *draft { 70.0 } else { 0.0 };
            // (a line of another kind than regular: its kind's mark)
            let kind_w = if *kind != ServiceKind::Regular { 22.0 } else { 0.0 };
            ui.text_in(name, Rect::new(plate.right() + 20.0, rr.y, rr.right() - plate.right() - 24.0 - badge_w - kind_w, rr.h), 13.0, Weight::Medium, TEXT, Align::Left);
            if kind_w > 0.0 {
                let at = Rect::new(rr.right() - badge_w - kind_w - 4.0, rr.y, kind_w, rr.h);
                ui.icon(kind.icon(), at.center(), 15.0, if Some(*id) == sel { on_accent() } else { accent_2() });
                ui.tooltip(at, kind.label());
            }
            if *draft {
                ui.badge(Vec2::new(rr.right() - badge_w + 4.0, rr.y + 10.0), &omsi_ui::tr("draft").to_uppercase(), if Some(*id) == sel { on_accent() } else { WARN });
            }
        }
        rows.len() as f32 * 40.0
    });
    if rows.is_empty() {
        let none = if v.company.is_some() { "The company has no line of its own yet" } else { "No lines of your own on this map yet" };
        ui.text_in(none, Rect::new(body.x, list_y, body.w, 36.0), 12.5, Weight::Regular, TEXT_DIM, Align::Left);
    }
    if let Some(id) = pick {
        if v.sel != Some(id) {
            (v.sel, v.dir, v.sel_stop, v.delete_armed) = (Some(id), 0, None, false);
        }
    }
    let mut y = list_y + list_h + 8.0;
    if ui.button("le-new", Rect::new(body.x, y, body.w, ROW), "New line", Some("add"), ButtonKind::Normal) {
        new_line(v);
        *status = Some(("Now click the line's stops on the map, in the order the bus calls at them".into(), false));
    }
    y += ROW + 14.0;
    if v.line().is_none() {
        return;
    }
    // the line's own settings
    ui.p().rect(Rect::new(body.x, y - 7.0, body.w, 1.0), HAIRLINE);
    ui.text_in(&omsi_ui::tr("Number").to_uppercase(), Rect::new(body.x, y, 90.0, 16.0), 10.5, Weight::Bold, TEXT_DIM, Align::Left);
    ui.text_in(&omsi_ui::tr("Name").to_uppercase(), Rect::new(body.x + 100.0, y, 120.0, 16.0), 10.5, Weight::Bold, TEXT_DIM, Align::Left);
    y += 18.0;
    let groups = v.groups.clone();
    let mut changed = false;
    {
        let line = v.line_mut().unwrap();
        changed |= ui.text_input("le-number", Rect::new(body.x, y, 90.0, ROW), &mut line.number, "42", None);
        changed |= ui.text_input("le-name", Rect::new(body.x + 100.0, y, body.w - 100.0, ROW), &mut line.name, "Name", None);
    }
    y += ROW + 12.0;
    // its colour (the company's two first, working for one)
    let current = v.line().map(|x| x.colour.clone()).unwrap_or_default();
    let palette: Vec<String> = match &v.company {
        Some(fc) => fc.colours.iter().cloned().chain(PALETTE.iter().take(6).map(|s| s.to_string())).collect(),
        None => PALETTE.iter().map(|s| s.to_string()).collect(),
    };
    let sw = ((body.w - 8.0 * 8.0) / 9.0).min(30.0);
    for (k, hex) in palette.iter().enumerate() {
        let c = Rect::new(body.x + k as f32 * (sw + 8.0), y, sw, sw);
        let on = current.eq_ignore_ascii_case(hex);
        let hover = ui.hover(c);
        ui.p().rounded(c, sw * 0.5, colour_of(hex));
        if on || hover {
            ui.p().rounded_border(c.inset(-3.0), sw * 0.5 + 3.0, 2.0, if on { TEXT } else { TEXT_DIM });
        }
        let (_, _, clicked) = ui.interact(super::ui::id_of(&format!("le-colour-{k}")), c);
        if clicked && !on {
            v.line_mut().unwrap().colour = hex.clone();
            changed = true;
        }
    }
    // any other colour: the picker under the swatches (the colour chosen in it fills the last
    // one)
    let c = Rect::new(body.x + palette.len() as f32 * (sw + 8.0), y, sw, sw);
    let own = !palette.iter().any(|h| h.eq_ignore_ascii_case(&current));
    let hover = ui.hover(c);
    let cur = crate::accent::parse_hex(&current).unwrap_or(0x2a75f7);
    ui.p().rounded(c, sw * 0.5, if own { colour_of(&current) } else { FIELD });
    if own || hover || v.colour_open {
        ui.p().rounded_border(c.inset(-3.0), sw * 0.5 + 3.0, 2.0, if own || v.colour_open { TEXT } else { TEXT_DIM });
    }
    ui.icon("palette", c.center(), 16.0, if own { crate::accent::Shades::of(cur).on } else if hover { TEXT } else { TEXT_SOFT });
    ui.tooltip(c, "Any colour");
    if ui.interact(super::ui::id_of("le-colour-any"), c).2 {
        v.colour_open = !v.colour_open;
    }
    y += sw + 16.0;
    if v.colour_open {
        let r = Rect::new(body.x, y - 4.0, body.w.min(360.0), super::accent_pick::PICKER_H);
        if let Some(rgb) = super::accent_pick::picker(ui, "le-colour-pick", r, cur) {
            v.line_mut().unwrap().colour = crate::accent::hex(rgb);
            changed = true;
        }
        y += super::accent_pick::PICKER_H + 12.0;
    }
    // the depot whose buses drive it
    ui.text_in(&omsi_ui::tr("Driven by the buses of").to_uppercase(), Rect::new(body.x, y, body.w, 16.0), 10.5, Weight::Bold, TEXT_DIM, Align::Left);
    y += 18.0;
    if groups.is_empty() {
        let line = v.line_mut().unwrap();
        changed |= ui.text_input("le-group", Rect::new(body.x, y, body.w, ROW), &mut line.ai_group, "Depot group (ailists.cfg)", None);
        y += ROW + 4.0;
        ui.paragraph("This map's ailists.cfg has no depot group with buses: the timetable's buses cannot drive the line (you can).", Vec2::new(body.x, y), body.w, 11.5, Weight::Regular, WARN);
    } else {
        let names: Vec<String> = groups.iter().map(|g| g.0.clone()).collect();
        let line = v.line_mut().unwrap();
        let mut gi = names.iter().position(|n| n.eq_ignore_ascii_case(&line.ai_group)).unwrap_or(0);
        if line.ai_group.trim().is_empty() || !names.iter().any(|n| n.eq_ignore_ascii_case(&line.ai_group)) {
            line.ai_group = names[gi].clone();
            changed = true;
        }
        if ui.select("le-group", Rect::new(body.x, y, body.w, ROW), &mut gi, &names) {
            line.ai_group = names[gi].clone();
            changed = true;
        }
    }
    // working for the company: live departure displays at the transfer stops
    if let Some(f) = figures.as_ref() {
        y += ROW + 12.0;
        let n = f.shape.transfer_stops();
        let label = omsi_ui::tr("Live displays at %{n} transfer stops").replace("%{n}", &n.to_string());
        let line = v.line_mut().unwrap();
        let mut on = line.live_displays;
        if ui.toggle("le-live", Rect::new(body.x, y, body.w, ROW), &mut on, &label) {
            line.live_displays = on;
            changed = true;
        }
    }
    if changed {
        v.touched();
    }
    if v.company.is_some() {
        let problems = v.line().map(reg::problems).unwrap_or_default();
        company_foot(ui, v, state, body, r.bottom() - 12.0 - ROW, &problems, figures.as_ref(), status);
        return;
    }
    // at the foot: save, and delete (two presses)
    let foot = r.bottom() - 12.0 - ROW;
    let problems = v.line().map(reg::problems).unwrap_or_default();
    if let Some(p) = problems.first() {
        let p = omsi_ui::tr(p.text).replace("%{n}", &p.n.to_string());
        let h = ui.paragraph_height(&p, body.w, 11.5, Weight::Regular);
        ui.paragraph(&p, Vec2::new(body.x, foot - ROW - 14.0 - h), body.w, 11.5, Weight::Regular, WARN);
    }
    let half = (body.w - GAP) * 0.5;
    let label = if v.dirty { "Save line" } else { "Saved" };
    if ui.button("le-save", Rect::new(body.x, foot - ROW - 8.0, body.w, ROW), label, Some("save"), if v.dirty { ButtonKind::Primary } else { ButtonKind::Normal }) && v.dirty {
        *status = Some(save(v, state, &problems));
    }
    if ui.button("le-delete", Rect::new(body.x, foot, half, ROW), if v.delete_armed { "Press again" } else { "Delete line" }, Some("delete"), ButtonKind::Normal) {
        if v.delete_armed {
            *status = Some(delete(v, state));
        } else {
            v.delete_armed = true;
            *status = Some(("Press \"Delete line\" again: the line and its files go".into(), false));
        }
    }
    if ui.button("le-drive", Rect::new(body.x + half + GAP, foot, half, ROW), "Drive it", Some("play_arrow"), ButtonKind::Normal) {
        *status = Some(drive_it(v, state));
    }
}

/// The registry written, and every line of it that can be into the map's timetable.
fn save(v: &mut LineEditorView, state: &mut super::state::State, problems: &[reg::Problem]) -> (String, bool) {
    if let Some(line) = v.line_mut() {
        line.modified = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    }
    let prepared = v.prepare_depots();
    if let Err(e) = reg::save_registry(&v.reg_path, &v.reg) {
        return (format!("Not saved: {e}"), true);
    }
    v.dirty = false;
    let Some(content) = core::content_dir() else { return ("Saved in your lines, but there is no content folder to write the timetable into".into(), true) };
    match reg::export_to_map(&content, &v.map_dir, &v.reg) {
        Ok(_) => {
            let depot_error = v.write_depots(&content, &prepared);
            omsi_cfg::content_changed();
            state.load_lines();
            let number = v.line().map(|x| x.number.clone()).unwrap_or_default();
            if let Some(e) = depot_error {
                (format!("{}: {e}", omsi_ui::tr("The line is saved, but a depot file could not be written (displays and IBIS)")), true)
            } else if let Some(p) = problems.first() {
                (omsi_ui::tr("Line %{n} kept as a draft: %{why}").replace("%{n}", &number).replace("%{why}", &omsi_ui::tr(p.text).replace("%{n}", &p.n.to_string())), false)
            } else {
                (omsi_ui::tr("Line %{n} saved: it is in the map's timetable now (the next game start picks it up)").replace("%{n}", &number), false)
            }
        }
        Err(e) => (format!("{}: {e}", omsi_ui::tr("The timetable files could not be written")), true),
    }
}

/// The shown line out of the registry and its files out of the map's timetable.
fn delete(v: &mut LineEditorView, state: &mut super::state::State) -> (String, bool) {
    let Some(id) = v.sel else { return (String::new(), false) };
    let number = v.line().map(|x| x.number.clone()).unwrap_or_default();
    v.reg.lines.retain(|x| x.id != id);
    v.sel = v.first_shown();
    (v.dir, v.sel_stop, v.delete_armed, v.dirty) = (0, None, false, false);
    v.shapes_for = None;
    v.revision += 1;
    let prepared = v.prepare_depots();
    if let Err(e) = reg::save_registry(&v.reg_path, &v.reg) {
        return (format!("Not deleted: {e}"), true);
    }
    if let Some(content) = core::content_dir() {
        if let Err(e) = reg::export_to_map(&content, &v.map_dir, &v.reg) {
            return (format!("{}: {e}", omsi_ui::tr("The timetable files could not be written")), true);
        }
        if let Some(e) = v.write_depots(&content, &prepared) {
            return (format!("{}: {e}", omsi_ui::tr("A depot file could not be written (displays and IBIS)")), true);
        }
        omsi_cfg::content_changed();
        state.load_lines();
    }
    (omsi_ui::tr("Line %{n} deleted").replace("%{n}", &number), false)
}

/// The Drive page set to drive the shown line: a free drive along its first direction on its
/// map (it must be saved: the game reads the files).
fn drive_it(v: &mut LineEditorView, state: &mut super::state::State) -> (String, bool) {
    let Some(line) = v.line() else { return (String::new(), false) };
    if v.dirty || !reg::problems(line).is_empty() {
        return ("Save the line first: the game drives what is saved".into(), true);
    }
    let stem = reg::stems(&v.reg).get(&line.id).cloned().unwrap_or_default();
    let file = v.reg.global.clone();
    let c = &mut state.choice;
    if c.map != file {
        c.map = file;
        c.entry = -1;
    }
    (c.free, c.composed, c.own_line) = (true, false, true);
    // (the line lists open on the player's own lines)
    c.my_lines = true;
    c.free_line = stem.clone();
    c.free_route = reg::trip_name(&stem, 0);
    state.touched();
    state.load_lines();
    v.go_drive = true;
    (String::new(), false)
}

// --- the right panel: a direction's stops, and the timetable ----------------------------------

fn right_panel(l: &mut Launcher, r: Rect, status: &mut Option<(String, bool)>) {
    l.busclasses.want(&l.state.vehicles);
    let fleet = l.company.company.as_ref().filter(|_| l.pages.lines.company.is_some()).map(|c| c.fleet.iter().map(|x| (x.bus.clone(), x.name.clone(), x.kind.size)).collect::<Vec<_>>());
    let date = match (l.pages.lines.company.is_some(), l.company.company.as_ref()) {
        (true, Some(c)) => c.date.clone(),
        _ => l.state.choice.date.clone(),
    };
    // (working for the company: the sizes of bus its level does not open yet)
    let locked = if l.pages.lines.company.is_some() { super::company::sizes_locked(l) } else { Vec::new() };
    let Launcher { ui, pages, state, busclasses, .. } = l;
    let facts = ServiceFacts { vehicles: &state.vehicles, classes: busclasses, fleet, date, locked };
    let v = &mut pages.lines;
    ui.card(r);
    let inner = Rect::new(r.x + 16.0, r.y + 12.0, r.w - 32.0, r.h - 24.0);
    let Some(line) = v.line() else {
        ui.heading(inner, "The line", Some("route"));
        ui.paragraph("Choose one of your lines, or make a new one.", Vec2::new(inner.x, inner.y + 30.0), inner.w, 13.0, Weight::Regular, TEXT_DIM);
        return;
    };
    let two = line.directions.len() > 1;
    let mut tab = v.tab;
    if ui.segmented("le-tab", Rect::new(inner.x, inner.y, inner.w, ROW), &mut tab, &["Stops", "Displays", "Timetable", "Kind"]) {
        v.tab = tab;
    }
    let body = Rect::new(inner.x, inner.y + ROW + 12.0, inner.w, inner.h - ROW - 12.0);
    if v.tab == 3 {
        service_tab(ui, v, body, &facts);
        return;
    }
    if v.tab == 2 {
        timetable_tab(ui, v, body, &facts.locked);
        return;
    }
    if v.tab == 1 {
        displays_tab(ui, v, body, status);
        return;
    }
    let mut dir = v.dir;
    if ui.segmented("le-dir", Rect::new(body.x, body.y, body.w, ROW), &mut dir, &["Outbound", "Return"]) {
        (v.dir, v.sel_stop) = (dir, None);
        v.revision += 1;
    }
    let mut y = body.y + ROW + 12.0;
    if v.dir == 1 && !two {
        ui.paragraph("Without a way back the line goes round: the bus starts again at the first stop.", Vec2::new(body.x, y), body.w, 12.5, Weight::Regular, TEXT_DIM);
        y += 50.0;
        if ui.button("le-reverse-new", Rect::new(body.x, y, body.w, ROW), "Reverse: make the way back", Some("sync_alt"), ButtonKind::Primary) {
            reverse(v, status);
        }
        return;
    }
    // where it goes: a terminus of the depot's file, one of the player's own destinations, or
    // what is typed
    let termini = v.termini();
    let mine = v.mine();
    let d_idx = v.dir;
    let mut changed = false;
    {
        let line = v.line_mut().unwrap();
        let d = &mut line.directions[d_idx];
        ui.text_in(&omsi_ui::tr("Destination").to_uppercase(), Rect::new(body.x, y, body.w, 16.0), 10.5, Weight::Bold, TEXT_DIM, Align::Left);
        y += 18.0;
        let last = d.stops.last().map(|s| s.name.clone()).unwrap_or_default();
        let mut options = vec![omsi_ui::tr("Last stop: %{name}").replace("%{name}", &last)];
        options.extend(termini.iter().map(|t| t.1.clone()));
        options.extend(mine.iter().map(|m| omsi_ui::tr("Mine: %{name}").replace("%{name}", &m.name)));
        let mut ti = termini
            .iter()
            .position(|t| t.0.eq_ignore_ascii_case(d.terminus.trim()))
            .map(|i| i + 1)
            .or_else(|| mine.iter().position(|m| m.name.eq_ignore_ascii_case(d.terminus.trim())).map(|i| i + 1 + termini.len()))
            .unwrap_or(0);
        let half = (body.w - GAP) * 0.5;
        if ui.select("le-terminus", Rect::new(body.x, y, half, ROW), &mut ti, &options) {
            if ti == 0 {
                d.terminus.clear();
            } else if ti <= termini.len() {
                d.terminus = termini[ti - 1].0.clone();
            } else if let Some(m) = mine.get(ti - 1 - termini.len()) {
                d.take_destination(m);
            }
            changed = true;
        }
        changed |= ui.text_input("le-terminus-text", Rect::new(body.x + half + GAP, y, half, ROW), &mut d.terminus, "or type it", None);
    }
    y += ROW + 12.0;
    // the stops, the way to each between them
    let foot_h = 2.0 * ROW + 16.0;
    let list = Rect::new(body.x - 6.0, y, body.w + 12.0, body.bottom() - y - foot_h);
    let (rows, legs, times, total) = {
        let d = &v.line().unwrap().directions[d_idx];
        (d.stops.iter().map(|s| s.name.clone()).collect::<Vec<_>>(), d.legs.iter().map(|g| (g.ok, g.length, g.vias.len())).collect::<Vec<_>>(), d.times.clone(), d.minutes())
    };
    let sel_stop = v.sel_stop;
    let colour = colour_of(&v.line().unwrap().colour);
    enum Act {
        Pick(usize),
        Up(usize),
        Down(usize),
        Remove(usize),
        Shift(usize, f32),
        ClearVias(usize),
    }
    let mut act = None;
    let rh = 34.0;
    let lh = 20.0;
    ui.scroll_area(&format!("le-stops-{d_idx}"), list, &mut |ui, a| {
        let mut yy = a.y;
        if rows.is_empty() {
            ui.paragraph("Click the stops on the map in the order the bus calls at them.", Vec2::new(a.x + 6.0, yy + 4.0), a.w - 12.0, 12.5, Weight::Regular, TEXT_DIM);
            return 60.0;
        }
        for (i, name) in rows.iter().enumerate() {
            let rr = Rect::new(a.x + 6.0, yy, a.w - 12.0, rh);
            if ui.row(&format!("le-stop-{d_idx}-{i}"), rr, sel_stop == Some(i)) {
                act = Some(Act::Pick(i));
            }
            let dot = Vec2::new(rr.x + 14.0, rr.center().y);
            ui.p().circle(dot, if i == 0 || i + 1 == rows.len() { 6.0 } else { 4.5 }, colour);
            ui.text_in(name, Rect::new(rr.x + 28.0, rr.y, rr.w - 196.0, rr.h), 12.5, Weight::Medium, TEXT, Align::Left);
            let t = times.get(i).copied().unwrap_or(0.0);
            ui.text_in(&format!("{t:.0}'"), Rect::new(rr.right() - 164.0, rr.y, 34.0, rr.h), 12.0, Weight::Bold, TEXT_SOFT, Align::Right);
            let cy = rr.center().y;
            if i > 0 {
                if ui.icon_button(&format!("le-later-{d_idx}-{i}"), Vec2::new(rr.right() - 88.0, cy), 10.0, "add", "A minute later") {
                    act = Some(Act::Shift(i, 1.0));
                }
                if ui.icon_button(&format!("le-earlier-{d_idx}-{i}"), Vec2::new(rr.right() - 112.0, cy), 10.0, "remove", "A minute earlier") {
                    act = Some(Act::Shift(i, -1.0));
                }
            }
            if sel_stop == Some(i) {
                if i > 0 && ui.icon_button(&format!("le-up-{d_idx}-{i}"), Vec2::new(rr.right() - 54.0, cy), 10.0, "keyboard_arrow_up", "Earlier in the line") {
                    act = Some(Act::Up(i));
                }
                if i + 1 < rows.len() && ui.icon_button(&format!("le-down-{d_idx}-{i}"), Vec2::new(rr.right() - 32.0, cy), 10.0, "keyboard_arrow_down", "Later in the line") {
                    act = Some(Act::Down(i));
                }
            }
            if ui.icon_button(&format!("le-x-{d_idx}-{i}"), Vec2::new(rr.right() - 10.0, cy), 10.0, "close", "Take the stop out") {
                act = Some(Act::Remove(i));
            }
            yy += rh;
            if let Some((ok, len, vias)) = legs.get(i).copied() {
                let lr = Rect::new(a.x + 34.0, yy, a.w - 40.0, lh);
                ui.p().rect(Rect::new(a.x + 19.5, yy - 4.0, 1.5, lh + 8.0), colour.alpha(0.5));
                let text = if ok { format!("{:.0} m", len) } else { omsi_ui::tr("no way over the roads").to_string() };
                ui.text_in(&text, lr, 11.0, Weight::Regular, if ok { TEXT_DIM } else { DANGER }, Align::Left);
                if vias > 0 && ui.icon_button(&format!("le-novia-{d_idx}-{i}"), Vec2::new(lr.right() - 10.0, lr.center().y), 9.0, "restart_alt", "Straighten the leg (its dragged points go)") {
                    act = Some(Act::ClearVias(i));
                }
                yy += lh;
            }
        }
        yy - a.y
    });
    let mut route = false;
    match act {
        Some(Act::Pick(i)) => v.sel_stop = if v.sel_stop == Some(i) { None } else { Some(i) },
        Some(Act::Up(i)) | Some(Act::Down(i)) => {
            let j = if matches!(act, Some(Act::Up(_))) { i - 1 } else { i + 1 };
            let d = &mut v.line_mut().unwrap().directions[d_idx];
            d.stops.swap(i, j);
            for g in &mut d.legs {
                g.vias.clear();
            }
            v.sel_stop = Some(j);
            route = true;
        }
        Some(Act::Remove(i)) => {
            let d = &mut v.line_mut().unwrap().directions[d_idx];
            d.stops.remove(i);
            // (the legs either side become one, without their dragged points)
            if i < d.legs.len() {
                d.legs.remove(i);
            } else if !d.legs.is_empty() {
                d.legs.pop();
            }
            if i > 0 && i - 1 < d.legs.len() {
                d.legs[i - 1].vias.clear();
            }
            v.sel_stop = None;
            route = true;
        }
        Some(Act::Shift(i, by)) => {
            v.line_mut().unwrap().directions[d_idx].shift_from(i, by);
            changed = true;
        }
        Some(Act::ClearVias(i)) => {
            v.line_mut().unwrap().directions[d_idx].legs[i].vias.clear();
            route = true;
        }
        None => {}
    }
    // the foot: the whole trip's minutes, the times from the roads again, the way back
    let fy = body.bottom() - foot_h + 8.0;
    ui.text_in(&omsi_ui::tr("%{n} min from the first stop to the last").replace("%{n}", &format!("{total:.0}")), Rect::new(body.x, fy, body.w - 70.0, ROW), 12.5, Weight::Medium, TEXT_SOFT, Align::Left);
    if ui.icon_button("le-total-less", Vec2::new(body.right() - 52.0, fy + ROW * 0.5), 12.0, "remove", "The whole trip a minute shorter") && total > 1.0 {
        v.line_mut().unwrap().directions[d_idx].set_total(total - 1.0);
        changed = true;
    }
    if ui.icon_button("le-total-more", Vec2::new(body.right() - 14.0, fy + ROW * 0.5), 12.0, "add", "The whole trip a minute longer") {
        v.line_mut().unwrap().directions[d_idx].set_total(total + 1.0);
        changed = true;
    }
    let half = (body.w - GAP) * 0.5;
    let by = fy + ROW + 8.0;
    if ui.button("le-times-auto", Rect::new(body.x, by, half, ROW), "Times from the roads", Some("schedule"), ButtonKind::Normal) {
        let d = &mut v.line_mut().unwrap().directions[d_idx];
        d.manual_times = false;
        d.refresh_times();
        changed = true;
    }
    if ui.button("le-reverse", Rect::new(body.x + half + GAP, by, half, ROW), if two { "Reverse again" } else { "Reverse" }, Some("sync_alt"), ButtonKind::Normal) {
        reverse(v, status);
    }
    if route {
        v.reroute(d_idx);
    } else if changed {
        v.touched();
    }
}

/// The way back made anew: the outbound stops from the last to the first, each the stop of
/// the same name across the road, routed.
fn reverse(v: &mut LineEditorView, status: &mut Option<(String, bool)>) {
    let all = v.stops.clone();
    let Some(line) = v.line_mut() else { return };
    if line.directions[0].stops.len() < 2 {
        *status = Some(("Give the outbound direction its stops first".into(), true));
        return;
    }
    // (the way back keeps the route code it had on the IBIS)
    let ibis_route = line.directions.get(1).map(|d| d.ibis_route).unwrap_or(0);
    let back = Direction { stops: reg::opposite_stops(&line.directions[0].stops, &all, ACROSS), ibis_route, ..Default::default() };
    line.directions.truncate(1);
    line.directions.push(back);
    (v.dir, v.sel_stop) = (1, None);
    v.reroute(1);
    let bad = v.line().map(|x| x.directions[1].legs.iter().filter(|g| !g.ok).count()).unwrap_or(0);
    *status = Some(if bad > 0 {
        (omsi_ui::tr("The way back is made; %{n} leg(s) have no way over the roads (red): drag them, or change a stop").replace("%{n}", &bad.to_string()), true)
    } else {
        ("The way back is made from the stops across the road".into(), false)
    });
}

/// A destination matrix as a bus shows it: the line number on the left, the destination in one
/// or two lines beside it, lit dots on black (amber, or white).
pub(super) fn led_sign(ui: &mut Ui, r: Rect, line: &str, l1: &str, l2: &str, white: bool) {
    let lit = if white { Color::rgba(236, 242, 255, 1.0) } else { Color::rgba(255, 172, 28, 1.0) };
    let black = Color::rgba(12, 12, 14, 1.0);
    ui.p().rounded(r.inset(-3.0), 9.0, Color::rgba(48, 52, 60, 1.0));
    ui.p().rounded(r, 6.0, black);
    let inner = r.inset(7.0);
    // (a text too wide for its place gets smaller, as the matrix's narrower font)
    let fit = |ui: &Ui, s: &str, px: f32, w: f32| {
        let width = ui.width(s, px, Weight::Bold);
        if width > w { (px * w / width).max(7.0) } else { px }
    };
    let line = line.trim();
    let nw = if line.is_empty() { 0.0 } else { (ui.width(line, 30.0, Weight::Black) + 10.0).max(inner.h * 0.9) };
    if !line.is_empty() {
        ui.text_in(line, Rect::new(inner.x, inner.y, nw, inner.h), 30.0, Weight::Black, lit, Align::Center);
    }
    let dr = Rect::new(inner.x + nw + 6.0, inner.y, inner.w - nw - 6.0, inner.h);
    if l2.is_empty() {
        let px = fit(ui, l1, 21.0, dr.w);
        ui.text_in(l1, dr, px, Weight::Bold, lit, Align::Center);
    } else {
        let half = dr.h * 0.5;
        let px = fit(ui, l1, 15.0, dr.w).min(fit(ui, l2, 15.0, dr.w));
        ui.text_in(l1, Rect::new(dr.x, dr.y, dr.w, half), px, Weight::Bold, lit, Align::Center);
        ui.text_in(l2, Rect::new(dr.x, dr.y + half, dr.w, half), px, Weight::Bold, lit, Align::Center);
    }
    // the dots: a dark grid over what is lit
    let dark = black.alpha(0.62);
    let step = 2.6;
    let mut x = inner.x;
    while x < inner.right() {
        ui.p().rect(Rect::new(x, inner.y, 1.0, inner.h), dark);
        x += step;
    }
    let mut y = inner.y;
    while y < inner.bottom() {
        ui.p().rect(Rect::new(inner.x, y, inner.w, 1.0), dark);
        y += step;
    }
}

/// The depot file the shown line has (`r`'s top, `r.w` wide; a label and a field): the map's,
/// one of the player's own, or a new one of his own made from the map's - and the way to the
/// depot editor for one of his.
fn depot_choice(ui: &mut Ui, v: &mut LineEditorView, r: Rect, group: &str, chosen: &str, status: &mut Option<(String, bool)>) {
    ui.text_in(&omsi_ui::tr("Depot file").to_uppercase(), Rect::new(r.x, r.y, r.w, 16.0), 10.5, Weight::Bold, TEXT_DIM, Align::Left);
    let y = r.y + 18.0;
    let hof = v.hof_of(group);
    let own: Vec<(String, String)> = v.own_entries().iter().map(|e| (e.key.clone(), e.name.clone())).collect();
    let mut options = vec![if hof.is_empty() { omsi_ui::tr("The map's (it names none)").into_owned() } else { omsi_ui::tr("The map's: %{hof}").replace("%{hof}", &hof) }];
    options.extend(own.iter().map(|(_, name)| omsi_ui::tr("Mine: %{name}").replace("%{name}", name)));
    options.push(omsi_ui::tr("A new one of my own, from the map's").into_owned());
    crate::mt::protect(options.iter().map(String::as_str));
    let at = own.iter().position(|(key, _)| key.eq_ignore_ascii_case(chosen));
    let mut k = at.map(|i| i + 1).unwrap_or(0);
    let open = at.is_some();
    let fw = if open { r.w - ROW - 6.0 } else { r.w };
    if ui.select("le-depot-file", Rect::new(r.x, y, fw, ROW), &mut k, &options) {
        let key = if k == 0 {
            Ok(String::new())
        } else if let Some((key, _)) = own.get(k - 1) {
            Ok(key.clone())
        } else {
            let base = v.map_depot_bytes(&v.reg.global.clone());
            let name = omsi_ui::tr("%{map} - mine").replace("%{map}", &v.folder);
            owndepot::create_from(&owndepot::dir(), &name, &v.folder, base.as_deref()).map(|p| {
                *status = Some((omsi_ui::tr("Your depot file %{name} is made from the map's: the line's destinations go into it when it is saved").replace("%{name}", &name), false));
                p.file_stem().unwrap_or_default().to_string_lossy().into_owned()
            })
        };
        match key {
            Ok(key) => {
                v.line_mut().unwrap().depot_file = key;
                v.own_changed();
                v.touched();
            }
            Err(e) => *status = Some((e, true)),
        }
    }
    if open && ui.icon_button("le-depot-open", Vec2::new(r.right() - ROW * 0.5, y + ROW * 0.5), 13.0, "open_in_new", "Open it in the depot editor") {
        v.go_depots = Some(chosen.to_string());
    }
}

/// What the buses show of a direction, and what the IBIS knows of it (`core::linehof`): the
/// sign as it lights up; a destination the depot file lacks with its texts, one per string of
/// the file (each empty one made from the destination, as the file writes its own); the IBIS
/// route code; what the IBIS calls each stop. All of it written into the depot files when the
/// line is saved.
///
/// Over it the depot file the line has: the map's (its depot group's), or one of the player's
/// own (`owndepot`) - the line's destinations, stops and routes go into that one as well, and
/// every bus that drives the line is given it.
fn displays_tab(ui: &mut Ui, v: &mut LineEditorView, body: Rect, status: &mut Option<(String, bool)>) {
    let (group, two, number, chosen) = {
        let l = v.line().unwrap();
        (l.ai_group.clone(), l.directions.len() > 1, l.number.trim().to_string(), l.depot_file.trim().to_string())
    };
    let mut y = body.y;
    depot_choice(ui, v, Rect::new(body.x, y, body.w, 0.0), &group, &chosen, status);
    y += 18.0 + ROW + 6.0;
    let hint = if chosen.is_empty() { "Choose one of your own: every bus that drives the line is given it." } else { "Every bus that drives the line is given it; the timetable's buses keep the map's (it has the line too)." };
    y += ui.paragraph(hint, Vec2::new(body.x, y), body.w, 11.0, Weight::Regular, TEXT_FAINT) + 8.0;
    let mut dir = v.dir;
    if ui.segmented("le-dir-d", Rect::new(body.x, y, body.w, ROW), &mut dir, &["Outbound", "Return"]) {
        (v.dir, v.sel_stop) = (dir, None);
        v.revision += 1;
    }
    y += ROW + 12.0;
    if v.dir == 1 && !two {
        ui.paragraph("Without a way back the line goes round: the bus starts again at the first stop.", Vec2::new(body.x, y), body.w, 12.5, Weight::Regular, TEXT_DIM);
        return;
    }
    let d_idx = v.dir;
    let Some(depot) = v.line_depot() else {
        let hof = v.hof_of(&group);
        let text = if hof.is_empty() { omsi_ui::tr("The depot group has no depot file: the buses' displays and IBIS cannot know the line.").to_string() } else { omsi_ui::tr("The depot file %{hof} was not found beside any bus: the displays and the IBIS cannot know the line.").replace("%{hof}", &hof) };
        ui.paragraph(&text, Vec2::new(body.x, y), body.w, 12.5, Weight::Regular, WARN);
        return;
    };
    let d = v.line().unwrap().directions[d_idx].clone();
    let dest = d.destination();
    if d.stops.len() < 2 || dest.is_empty() {
        ui.paragraph("Give the direction its stops first: the destination is its last one, or the one chosen with the stops.", Vec2::new(body.x, y), body.w, 12.5, Weight::Regular, TEXT_DIM);
        return;
    }
    let (strings, new) = depot.sign_of(&d);
    let (l1, l2) = depot.front(&strings);
    // the sign (a click lights it the other colour)
    let sign = Rect::new(body.x + 3.0, y + 3.0, body.w - 6.0, 58.0);
    led_sign(ui, sign, &number, &l1, &l2, v.white);
    if ui.interact(super::ui::id_of("le-sign"), sign).2 {
        v.white = !v.white;
    }
    y += 70.0;
    let what = if new { omsi_ui::tr("New destination: %{name}") } else { omsi_ui::tr("%{name}, from the depot file") };
    ui.text_in(&what.replace("%{name}", &dest), Rect::new(body.x, y, body.w - 28.0, 20.0), 12.0, Weight::Medium, if new { accent_2() } else { TEXT_SOFT }, Align::Left);
    let mut changed = false;
    let mut sign_vals = d.sign.clone();
    if new && !d.sign.iter().all(|s| s.is_empty()) && ui.icon_button("le-sign-reset", Vec2::new(body.right() - 10.0, y + 10.0), 10.0, "restart_alt", "The texts made from the destination again") {
        sign_vals.clear();
        changed = true;
    }
    y += 26.0;
    // the rest scrolls: the texts, the IBIS route, the stops' names
    let defaults = if new { depot.sign_defaults(&dest) } else { Vec::new() };
    sign_vals.resize(defaults.len(), String::new());
    let labels: Vec<String> = depot
        .labels()
        .iter()
        .enumerate()
        .map(|(k, (role, note))| if note.is_empty() { omsi_ui::tr(role.label()).replace("%{n}", &(k + 1).to_string()) } else { note.chars().take(48).collect() })
        .collect();
    let line_no = linehof::line_number(&number);
    let mut route = if d.ibis_route > 0 { d.ibis_route.to_string() } else { String::new() };
    let route_hint = (line_no * 100 + d_idx as u32 + 1).to_string();
    let taken: std::collections::HashSet<u32> = depot.hof.info_trips.iter().filter_map(|t| t.code.trim().parse().ok()).collect();
    let stops: Vec<(String, String)> = d.stops.iter().map(|s| (s.name.clone(), depot.stop_display(&s.name))).collect();
    let mut stop_vals: Vec<String> = d.stops.iter().map(|s| s.ibis.clone()).collect();
    let area = Rect::new(body.x - 6.0, y, body.w + 12.0, body.bottom() - y);
    let head = |ui: &mut Ui, text: &str, r: Rect| {
        ui.text_in(&omsi_ui::tr(text).to_uppercase(), r, 10.5, Weight::Bold, TEXT_DIM, Align::Left);
    };
    let mut route_changed = false;
    let mut texts_changed = false;
    let mut stops_changed = false;
    // (the player's own destinations, in his depot file of the map: this one kept, one typed,
    // one taken out - those the map's depot file lacks)
    let (target, mine, is_mine) = {
        let k = v.kept();
        (k.target.clone(), k.mine.clone(), k.names.contains(&dest.trim().to_lowercase()))
    };
    let where_kept = match target.as_ref() {
        Some((_, name)) => omsi_ui::tr("In your depot file %{name} (Editor, Depot editor); chosen under Stops, Destination.").replace("%{name}", name),
        None => omsi_ui::tr("Kept in a depot file of your own for this map, made from the map's when you keep the first; chosen under Stops, Destination.").into_owned(),
    };
    crate::mt::protect(mine.iter().map(|m| m.name.as_str()).chain([where_kept.as_str()]));
    // (the texts the depot file fills for a destination first; those its destinations leave
    // empty, and its textures, folded away under them)
    let in_use: Vec<bool> = (0..defaults.len()).map(|k| matches!(depot.cols.get(k), Some(linehof::Column::Text { .. })) && (!defaults[k].trim().is_empty() || !sign_vals[k].is_empty())).collect();
    let unused = in_use.iter().filter(|u| !**u).count();
    let more = v.more_texts;
    let mut more_toggle = false;
    let mut dest_new = v.dest_new.clone();
    let (mut keep_this, mut keep_new, mut drop_mine) = (false, false, None);
    ui.scroll_area(&format!("le-displays-{d_idx}"), area, &mut |ui, a| {
        let x = a.x + 6.0;
        let w = a.w - 12.0;
        let mut yy = a.y + 2.0;
        if new {
            head(ui, "Display texts", Rect::new(x, yy, w, 16.0));
            yy += 20.0;
            let lw = (w * 0.36).min(130.0);
            for (k, def) in defaults.iter().enumerate() {
                if !in_use[k] && !more {
                    continue;
                }
                ui.text_in(&labels.get(k).cloned().unwrap_or_default(), Rect::new(x, yy, lw - 8.0, ROW - 4.0), 11.5, Weight::Medium, if in_use[k] { TEXT_SOFT } else { TEXT_FAINT }, Align::Left);
                // (empty: the default, shown greyed)
                texts_changed |= ui.text_input(&format!("le-sign-{d_idx}-{k}"), Rect::new(x + lw, yy, w - lw, ROW - 4.0), &mut sign_vals[k], if def.trim().is_empty() { "-" } else { def.as_str() }, None);
                yy += ROW + 2.0;
            }
            if unused > 0 {
                let label = if more { omsi_ui::tr("Hide the %{n} empty texts") } else { omsi_ui::tr("%{n} more texts, empty for this depot file") };
                if ui.button(&format!("le-sign-more-{d_idx}"), Rect::new(x, yy, w, ROW - 8.0), &label.replace("%{n}", &unused.to_string()), Some(if more { "expand_less" } else { "expand_more" }), ButtonKind::Ghost) {
                    more_toggle = true;
                }
                yy += ROW - 2.0;
            }
            yy += 6.0;
        }
        head(ui, "IBIS route", Rect::new(x, yy, w, 16.0));
        yy += 20.0;
        route_changed |= ui.text_input(&format!("le-route-{d_idx}"), Rect::new(x, yy, 110.0, ROW - 4.0), &mut route, &route_hint, None);
        let code: u32 = route.trim().parse().unwrap_or(0);
        let (hint, colour) = if route.trim().is_empty() {
            (omsi_ui::tr("Given when the line is saved").to_string(), TEXT_DIM)
        } else if code / 100 != line_no || code % 100 == 0 {
            (omsi_ui::tr("Not a route of line %{n} (%{n}01 - %{n}99): another is given on saving").replace("%{n}", &line_no.to_string()), WARN)
        } else if taken.contains(&code) {
            (omsi_ui::tr("The depot file has this code already: another is given on saving").to_string(), WARN)
        } else {
            (omsi_ui::tr("Line %{l}, route %{r} on the IBIS").replace("%{l}", &line_no.to_string()).replace("%{r}", &format!("{:02}", code % 100)), TEXT_DIM)
        };
        let hh = ui.paragraph(&hint, Vec2::new(x + 120.0, yy + 1.0), w - 120.0, 11.0, Weight::Regular, colour);
        yy += (ROW + 4.0).max(hh + 6.0);
        head(ui, "Stop names on the IBIS", Rect::new(x, yy, w, 16.0));
        yy += 20.0;
        // (one row a stop: its name, and what the IBIS calls it)
        let lw = (w * 0.36).min(130.0);
        for (i, (name, def)) in stops.iter().enumerate() {
            ui.text_in(name, Rect::new(x, yy, lw - 8.0, ROW - 4.0), 11.5, Weight::Medium, TEXT_SOFT, Align::Left);
            stops_changed |= ui.text_input(&format!("le-ibis-{d_idx}-{i}"), Rect::new(x + lw, yy, w - lw, ROW - 4.0), &mut stop_vals[i], def, None);
            yy += ROW + 2.0;
        }
        // the player's own destinations, offered with the depot file's wherever a destination
        // is chosen
        yy += 6.0;
        head(ui, "My destinations", Rect::new(x, yy, w, 16.0));
        yy += 22.0;
        if !is_mine && ui.button("le-dest-keep", Rect::new(x, yy, w, ROW - 6.0), "Keep this destination as mine", Some("save"), ButtonKind::Ghost) {
            keep_this = true;
        }
        if !is_mine {
            yy += ROW;
        }
        for (k, m) in mine.iter().enumerate() {
            let rr = Rect::new(x, yy, w, 30.0);
            ui.p().rounded(rr, RADIUS, FIELD);
            ui.text_in(&m.name, Rect::new(rr.x + 10.0, rr.y, rr.w - 90.0, rr.h), 12.5, Weight::Medium, TEXT, Align::Left);
            if m.code > 0 {
                ui.text_in(&m.code.to_string(), Rect::new(rr.right() - 80.0, rr.y, 44.0, rr.h), 11.5, Weight::Regular, TEXT_DIM, Align::Right);
            }
            if ui.icon_button(&format!("le-dest-x-{k}"), Vec2::new(rr.right() - 14.0, rr.center().y), 10.0, "close", "Take it out of my destinations") {
                drop_mine = Some(k);
            }
            yy += 34.0;
        }
        let bw = 78.0;
        ui.text_input("le-dest-new", Rect::new(x, yy, w - bw - 6.0, ROW - 4.0), &mut dest_new, "A destination of my own", None);
        if ui.button("le-dest-add", Rect::new(x + w - bw, yy, bw, ROW - 4.0), "Keep", Some("add"), ButtonKind::Normal) && !dest_new.trim().is_empty() {
            keep_new = true;
        }
        yy += ROW + 2.0;
        yy += ui.paragraph(&where_kept, Vec2::new(x, yy), w, 11.0, Weight::Regular, TEXT_FAINT);
        yy - a.y + 8.0
    });
    v.dest_new = dest_new;
    if more_toggle {
        v.more_texts = !v.more_texts;
    }
    if keep_this || keep_new || drop_mine.is_some() {
        let path = match target.as_ref() {
            Some((path, _)) => Ok(path.clone()),
            None => {
                let base = v.map_depot_bytes(&v.reg.global.clone());
                owndepot::ensure_for_map(&owndepot::dir(), &v.folder, &omsi_ui::tr("%{map} - mine").replace("%{map}", &v.folder), base.as_deref())
            }
        };
        let content = core::content_dir();
        let done = path.and_then(|p| {
            let file = owndepot::load(&p).map(|l| l.doc.hof.name).unwrap_or_default();
            if keep_this {
                // (a new one with the texts given here, else the depot file's own)
                let (sign, code) = if new { (sign_vals.clone(), d.terminus_code) } else { (strings.clone(), depot.terminus(&dest).map(|t| t.code).unwrap_or(0)) };
                owndepot::keep_destination(&p, &reg::OwnDestination { name: dest.clone(), sign, code }, content.as_deref()).map(|c| omsi_ui::tr("%{name} kept in your depot file %{file} (code %{code})").replace("%{name}", &dest).replace("%{code}", &c.to_string()).replace("%{file}", &file))
            } else if keep_new {
                let name = std::mem::take(&mut v.dest_new).trim().to_string();
                owndepot::keep_destination(&p, &reg::OwnDestination { name: name.clone(), ..Default::default() }, content.as_deref()).map(|c| omsi_ui::tr("%{name} kept in your depot file %{file} (code %{code})").replace("%{name}", &name).replace("%{code}", &c.to_string()).replace("%{file}", &file))
            } else {
                let name = drop_mine.and_then(|k| mine.get(k)).map(|m| m.name.clone()).unwrap_or_default();
                owndepot::drop_destination(&p, &name, content.as_deref()).map(|_| omsi_ui::tr("%{name} taken out of your depot file %{file}").replace("%{name}", &name).replace("%{file}", &file))
            }
        });
        crate::mt::protect(done.as_ref().ok().map(String::as_str));
        *status = Some(match done {
            Ok(text) => (text, false),
            Err(e) => (e, true),
        });
        v.own_changed();
        // (the bus step's tiles read the player's depot files again)
        omsi_cfg::content_changed();
    }
    if texts_changed || changed {
        // (all empty: the defaults, which follow the destination)
        if sign_vals.iter().all(|s| s.is_empty()) {
            sign_vals.clear();
        }
        v.line_mut().unwrap().directions[d_idx].sign = sign_vals;
        changed = true;
    }
    if route_changed {
        v.line_mut().unwrap().directions[d_idx].ibis_route = route.trim().parse().unwrap_or(0);
        changed = true;
    }
    if stops_changed {
        let dd = &mut v.line_mut().unwrap().directions[d_idx];
        for (s, val) in dd.stops.iter_mut().zip(stop_vals) {
            s.ibis = val;
        }
        changed = true;
    }
    if changed {
        v.touched();
    }
}

/// When the line runs: per group of days the first and last departure and how often - or time
/// bands, each part of the day with a headway of its own (the rush hours denser) and, for a
/// company, the bus it wants - and how long a bus stands at the end; how many buses that
/// takes.
fn timetable_tab(ui: &mut Ui, v: &mut LineEditorView, body: Rect, locked: &[BusSize]) {
    let run: Vec<f32> = v.line().map(reg::run_minutes).unwrap_or_default();
    let for_company = v.company.is_some();
    let kind = v.line().map(|x| x.service).unwrap_or_default();
    // the timetable as a table (every time the player's to change), or as the patterns make it
    let table_on = v.line().is_some_and(|x| x.table_on);
    let half = (body.w - GAP) * 0.5;
    let mut on = table_on;
    if ui.toggle("le-table-on", Rect::new(body.x, body.y, half, ROW), &mut on, "As a table") {
        let line = v.line_mut().unwrap();
        line.table_on = on;
        if on && line.table.is_empty() {
            line.table = reg::table_from_patterns(line);
        }
        v.touched();
    }
    if ui.button("le-table-open", Rect::new(body.x + half + GAP, body.y, half, ROW), "Open the table", Some("grid_view"), ButtonKind::Normal) {
        v.table_open = true;
    }
    let mut top = body.y + ROW + 8.0;
    if table_on {
        top += ui.paragraph("The table is the line's timetable: a change to the patterns below fills that group of days anew at once; the times typed into the other groups stay.", Vec2::new(body.x, top), body.w, 11.5, Weight::Regular, accent_2()) + 6.0;
    }
    let body = Rect::new(body.x, top, body.w, (body.bottom() - top).max(40.0));
    let mut changed = false;
    let line = v.line_mut().unwrap();
    if line.days.len() < DAY_GROUPS.len() {
        line.days = reg::default_days();
    }
    // (the departures each group had: a group whose patterns change them gets its trips in the
    // table anew)
    let before: Vec<Vec<f32>> = line.days.iter().map(|p| p.departures().iter().map(|d| d.0).collect()).collect();
    let days = &mut line.days;
    // (a size the company's level keeps locked says so, and asks for its popup)
    let sizes: Vec<String> = SIZES
        .iter()
        .map(|s| match s {
            Some(x) if locked.contains(x) => format!("{}  ·  {}", size_label(*s), omsi_ui::tr("locked")),
            _ => size_label(*s),
        })
        .collect();
    let mut level_ask = None;
    ui.scroll_area("le-timetable", Rect::new(body.x - 6.0, body.y, body.w + 12.0, body.h), &mut |ui, a| {
        let x = a.x + 6.0;
        let w = a.w - 12.0;
        let mut y = a.y;
        for (k, (name, _)) in DAY_GROUPS.iter().enumerate() {
            let p = &mut days[k];
            // (a group its kind of service does not run on: a school line's weekend)
            if !kind.allows(k) {
                ui.text_in(name, Rect::new(x, y, w * 0.4, ROW), 13.0, Weight::Regular, TEXT_FAINT, Align::Left);
                let why = omsi_ui::tr("%{kind} does not run then").replace("%{kind}", &omsi_ui::tr(kind.label()));
                ui.text_in(&why, Rect::new(x + w * 0.4, y, w * 0.6, ROW), 11.5, Weight::Regular, TEXT_FAINT, Align::Right);
                y += ROW + 4.0;
                ui.p().rect(Rect::new(x, y, w, 1.0), HAIRLINE);
                y += 10.0;
                continue;
            }
            changed |= ui.toggle(&format!("le-day-{k}"), Rect::new(x, y, w, ROW), &mut p.on, name);
            y += ROW + 4.0;
            if p.on {
                let mut mode = usize::from(!p.bands.is_empty());
                if ui.segmented(&format!("le-mode-{k}"), Rect::new(x, y, w, ROW - 4.0), &mut mode, &["One headway", "Time bands"]) {
                    p.bands = if mode == 1 { reg::default_bands(k) } else { Vec::new() };
                    changed = true;
                }
                y += ROW + 4.0;
                if p.bands.is_empty() {
                    let half = (w - GAP) * 0.5;
                    ui.text_in("First", Rect::new(x, y, half, 16.0), 10.5, Weight::Bold, TEXT_DIM, Align::Left);
                    ui.text_in("Last", Rect::new(x + half + GAP, y, half, 16.0), 10.5, Weight::Bold, TEXT_DIM, Align::Left);
                    y += 18.0;
                    let (mut first, mut last) = (p.first.round() as i32, p.last.round() as i32);
                    if ui.time_field(&format!("le-first-{k}"), Rect::new(x, y, half, ROW), &mut first) {
                        p.first = first as f32;
                        changed = true;
                    }
                    if ui.time_field(&format!("le-last-{k}"), Rect::new(x + half + GAP, y, half, ROW), &mut last) {
                        p.last = last as f32;
                        changed = true;
                    }
                    y += ROW + 6.0;
                    changed |= ui.slider(&format!("le-every-{k}"), Rect::new(x, y, w, 28.0), &mut p.headway, 5.0, 120.0, 5.0, "Every", &|x| format!("{x:.0} min"));
                    y += 30.0;
                } else {
                    // the bands: from - to, how often, the bus (a company's)
                    let tw = (w - 16.0 - 30.0) * 0.5;
                    let mut remove = None;
                    for (i, b) in p.bands.iter_mut().enumerate() {
                        let (mut from, mut to) = (b.from.round() as i32, if b.to >= reg::DAY_END { 0 } else { b.to.round() as i32 });
                        if ui.time_field(&format!("le-band-from-{k}-{i}"), Rect::new(x, y, tw, ROW - 4.0), &mut from) {
                            b.from = from as f32;
                            changed = true;
                        }
                        ui.text_in("–", Rect::new(x + tw, y, 16.0, ROW - 4.0), 14.0, Weight::Medium, TEXT_DIM, Align::Center);
                        if ui.time_field(&format!("le-band-to-{k}-{i}"), Rect::new(x + tw + 16.0, y, tw, ROW - 4.0), &mut to) {
                            // (00:00 at the end of a band is midnight)
                            b.to = if to == 0 { reg::DAY_END } else { to as f32 };
                            changed = true;
                        }
                        if ui.icon_button(&format!("le-band-x-{k}-{i}"), Vec2::new(x + w - 12.0, y + (ROW - 4.0) * 0.5), 10.0, "close", "Take the band out") {
                            remove = Some(i);
                        }
                        y += ROW;
                        let sw = if for_company { w * 0.56 } else { w };
                        changed |= ui.slider(&format!("le-band-every-{k}-{i}"), Rect::new(x, y, sw, 28.0), &mut b.headway, 5.0, 60.0, 5.0, "Every", &|x| format!("{x:.0} min"));
                        if for_company {
                            let mut si = SIZES.iter().position(|s| *s == b.size).unwrap_or(0);
                            if ui.select(&format!("le-band-size-{k}-{i}"), Rect::new(x + sw + 8.0, y - 2.0, w - sw - 8.0, 32.0), &mut si, &sizes) {
                                match SIZES[si] {
                                    Some(s) if locked.contains(&s) => level_ask = Some(s),
                                    s => {
                                        b.size = s;
                                        changed = true;
                                    }
                                }
                            }
                        }
                        y += 38.0;
                    }
                    if let Some(i) = remove {
                        p.bands.remove(i);
                        changed = true;
                    }
                    if ui.button(&format!("le-band-add-{k}"), Rect::new(x, y, w, ROW - 6.0), "Add a band", Some("add"), ButtonKind::Ghost) {
                        let from = p.bands.iter().map(|b| b.to).fold(0.0, f32::max).min(reg::DAY_END - 60.0);
                        p.bands.push(TimeBand { from, to: (from + 120.0).min(reg::DAY_END), headway: 30.0, size: None });
                        changed = true;
                    }
                    y += ROW;
                }
                changed |= ui.slider(&format!("le-layover-{k}"), Rect::new(x, y, w, 28.0), &mut p.layover, 0.0, 30.0, 1.0, "Stands", &|x| format!("{x:.0} min"));
                y += 30.0;
                let (buses, tours, trips) = if run.is_empty() {
                    (0, 0, 0)
                } else {
                    let t = reg::tours(&run, p);
                    (reg::blocks(&run, p).len(), t.len(), t.iter().map(|b| b.len()).sum::<usize>())
                };
                let text = if tours > buses {
                    omsi_ui::tr("%{b} buses in %{n} tours (some for the rush hours only), %{t} trips").replace("%{n}", &tours.to_string())
                } else {
                    omsi_ui::tr("%{b} buses, %{t} trips").into_owned()
                };
                ui.text_in(&text.replace("%{b}", &buses.to_string()).replace("%{t}", &trips.to_string()), Rect::new(x, y, w, 18.0), 11.5, Weight::Regular, TEXT_DIM, Align::Left);
                y += 26.0;
            }
            ui.p().rect(Rect::new(x, y, w, 1.0), HAIRLINE);
            y += 10.0;
        }
        let note = "Each group of days becomes tours of its own; the buses go back and forth, each standing at the end for at least as long as set. With time bands the buses added for the rush hours go back to the depot after them.";
        let h = ui.paragraph(note, Vec2::new(x, y), w, 11.5, Weight::Regular, TEXT_FAINT);
        y - a.y + h + 12.0
    });
    if changed {
        let line = v.line_mut().unwrap();
        if line.table_on {
            let moved: Vec<usize> = (0..line.days.len()).filter(|&k| before.get(k).is_none_or(|b| *b != line.days[k].departures().iter().map(|d| d.0).collect::<Vec<f32>>())).collect();
            if !moved.is_empty() {
                let fresh = reg::table_from_patterns(line);
                line.table.retain(|t| !moved.contains(&t.day));
                line.table.extend(fresh.into_iter().filter(|t| moved.contains(&t.day)));
                line.table.sort_by(|a, b| a.day.cmp(&b.day).then(a.departure().total_cmp(&b.departure())).then(a.dir.cmp(&b.dir)));
                (v.table_sel, v.table_text) = (None, String::new());
            }
        }
        v.touched();
    }
    if level_ask.is_some() {
        v.level_ask = level_ask;
    }
}

// --- the kind of service and the buses ----------------------------------------------------------

/// The installed buses by maker and model, as a line's buses are chosen from them: (maker, its
/// models - (model, its versions - (bus file, version))), in the bus picker's order.
type Catalogue = Vec<(String, Vec<(String, Vec<(String, String)>)>)>;

fn catalogue(vehicles: &[core::VehicleInfo]) -> Catalogue {
    use super::buspick::name_cmp;
    let mut out: Catalogue = Vec::new();
    for v in vehicles {
        let (maker, _) = service::maker_model(&v.manufacturer, &v.type_name, &v.file, &v.folder);
        let (model, version) = super::buspick::split_type(&v.type_name, &v.file, &v.default_paint);
        let m = match out.iter().position(|x| x.0.eq_ignore_ascii_case(&maker)) {
            Some(k) => k,
            None => {
                out.push((maker.clone(), Vec::new()));
                out.len() - 1
            }
        };
        let models = &mut out[m].1;
        let k = match models.iter().position(|x| x.0.eq_ignore_ascii_case(&model)) {
            Some(k) => k,
            None => {
                models.push((model.clone(), Vec::new()));
                models.len() - 1
            }
        };
        models[k].1.push((v.file.clone(), version));
    }
    out.sort_by(|a, b| name_cmp(&a.0, &b.0));
    for (_, models) in out.iter_mut() {
        models.sort_by(|a, b| name_cmp(&a.0, &b.0));
        for (_, versions) in models.iter_mut() {
            versions.sort_by(|a, b| name_cmp(&a.1, &b.1));
        }
    }
    out
}

/// The bus a line's choice gets from the catalogue's maker `m`, its model `k` (0: all of
/// them) and its version `j` (0: all of them): with the bus files it covers now.
fn pick_of(cat: &Catalogue, m: usize, k: usize, j: usize) -> Option<BusPick> {
    let (maker, models) = cat.get(m)?;
    if k == 0 {
        let files = models.iter().flat_map(|x| x.1.iter().map(|v| v.0.clone())).collect();
        return Some(BusPick { maker: maker.clone(), label: maker.clone(), files, ..Default::default() });
    }
    let (model, versions) = models.get(k - 1)?;
    if j == 0 {
        let files = versions.iter().map(|v| v.0.clone()).collect();
        return Some(BusPick { maker: maker.clone(), model: model.clone(), label: format!("{maker} {model}"), files, ..Default::default() });
    }
    let (file, version) = versions.get(j - 1)?;
    Some(BusPick { maker: maker.clone(), model: model.clone(), file: file.clone(), files: vec![file.clone()], label: format!("{maker} {model} · {version}") })
}

/// The timetable a line of `kind` starts from: `lines::days_for`, and - for a company's
/// regular line - the rush hours in it, as a new company line has them.
fn typical_days(kind: ServiceKind, company: bool) -> Vec<reg::DayPattern> {
    let mut days = reg::days_for(kind);
    if company && kind == ServiceKind::Regular {
        for (k, p) in days.iter_mut().enumerate() {
            p.bands = reg::default_bands(k);
        }
    }
    days
}

/// `YYYYMMDD` as the company's pages write a day ("Tue 5 Mar 1989").
fn day_of_code(code: i32) -> String {
    super::company::day_label(&format!("{:04}-{:02}-{:02}", code / 10000, code / 100 % 100, code % 100))
}

/// What the Kind tab knows besides the line: the installed buses and their kinds, the
/// company's fleet (file, name, size) when it works for one, and the date the launcher has.
struct ServiceFacts<'a> {
    vehicles: &'a [core::VehicleInfo],
    classes: &'a super::busclass::BusClasses,
    fleet: Option<Vec<(String, String, BusSize)>>,
    date: String,
    /// The sizes of bus the company's level keeps locked (none outside a company).
    locked: Vec<BusSize>,
}

/// The line's kind of service (and with a school line the school holidays it keeps), and the
/// buses that run it: kinds of bus, and buses by maker, model or version.
fn service_tab(ui: &mut Ui, v: &mut LineEditorView, body: Rect, f: &ServiceFacts) {
    if v.catalogue.as_ref().map(|c| c.0) != Some(f.vehicles.len()) {
        v.catalogue = Some((f.vehicles.len(), Arc::new(catalogue(f.vehicles))));
    }
    let cat = v.catalogue.as_ref().map(|c| c.1.clone()).unwrap_or_default();
    let Some(line) = v.line() else { return };
    let kind = line.service;
    let want = line.vehicles.clone();
    let matching = f.classes.matching(f.vehicles, &want).len();
    let in_fleet = f.fleet.as_ref().map(|fl| fl.iter().filter(|(file, name, size)| want.allows(&service::BusFacts::of_fleet(file, name, *size))).count());
    let cal = service::DayCalendar::new(&v.map_calendar, &v.reg.school_holidays);
    let mut periods = if v.reg.school_holidays.is_empty() { service::default_school_holidays() } else { v.reg.school_holidays.clone() };
    let (mut new_kind, mut typical, mut take_suits, mut periods_changed) = (None, false, false, false);
    let (mut toggle, mut remove, mut add): (Option<VehicleClass>, Option<usize>, Option<BusPick>) = (None, None, None);
    let mut pick = v.pick;
    let makers: Vec<String> = std::iter::once(omsi_ui::tr("Choose a brand").into_owned()).chain(cat.iter().map(|m| m.0.clone())).collect();
    let mut title = v.line().map(|x| x.title.clone()).unwrap_or_default();
    let mut title_changed = false;
    let mut level_ask = None;
    ui.scroll_area("le-service", Rect::new(body.x - 6.0, body.y, body.w + 12.0, body.h), &mut |ui, a| {
        let x = a.x + 6.0;
        let w = a.w - 12.0;
        let mut y = a.y;
        let caps = |ui: &mut Ui, y: f32, text: &str| ui.text_in(&omsi_ui::tr(text).to_uppercase(), Rect::new(x, y, w, 16.0), 10.5, Weight::Bold, TEXT_DIM, Align::Left);
        // what it is called in public: on its card and in its advertising
        caps(ui, y, "Public title");
        y += 20.0;
        title_changed |= ui.text_input("le-title", Rect::new(x, y, w, ROW - 4.0), &mut title, "Shuttleverkehr Altenfeld - Wurzbach", None);
        y += ROW + 6.0;
        caps(ui, y, "Kind of service");
        y += 20.0;
        for k in ServiceKind::ALL {
            let r = Rect::new(x, y, w, 34.0);
            let on = k == kind;
            if ui.row(&format!("le-kind-{k:?}"), r, on) && !on {
                new_kind = Some(k);
            }
            ui.icon(k.icon(), Vec2::new(r.x + 16.0, r.center().y), 17.0, if on { on_accent() } else { accent_2() });
            ui.text_in(k.label(), Rect::new(r.x + 34.0, r.y, r.w - 40.0, r.h), 13.0, Weight::Medium, if on { on_accent() } else { TEXT }, Align::Left);
            y += 36.0;
        }
        y += 4.0;
        y += ui.paragraph(kind.note(), Vec2::new(x, y), w, 11.5, Weight::Regular, TEXT_DIM) + 6.0;
        if ui.button("le-kind-typical", Rect::new(x, y, w, ROW - 6.0), "Typical timetable for this kind", Some("schedule"), ButtonKind::Ghost) {
            typical = true;
        }
        y += ROW + 2.0;
        // a school line: the school holidays it keeps
        if kind == ServiceKind::School {
            ui.p().rect(Rect::new(x, y, w, 1.0), HAIRLINE);
            y += 10.0;
            caps(ui, y, "School holidays");
            y += 20.0;
            if cal.map_school() {
                y += ui.paragraph("From the map's calendar (Holidays.txt), as the game keeps them.", Vec2::new(x, y), w, 11.5, Weight::Regular, TEXT_DIM) + 4.0;
            } else {
                y += ui.paragraph("This map's calendar names no school holidays: these hold for your lines here (the game itself runs every working day as a school day).", Vec2::new(x, y), w, 11.5, Weight::Regular, TEXT_DIM) + 6.0;
                let (nw, dw) = (w - 2.0 * 62.0 - 40.0, 62.0);
                let mut out = None;
                for (i, p) in periods.iter_mut().enumerate() {
                    periods_changed |= ui.text_input(&format!("le-hol-name-{i}"), Rect::new(x, y, nw, ROW - 6.0), &mut p.name, "Name", None);
                    periods_changed |= ui.text_input(&format!("le-hol-from-{i}"), Rect::new(x + nw + 6.0, y, dw, ROW - 6.0), &mut p.from, "MM-DD", None);
                    periods_changed |= ui.text_input(&format!("le-hol-to-{i}"), Rect::new(x + nw + 12.0 + dw, y, dw, ROW - 6.0), &mut p.to, "MM-DD", None);
                    if ui.icon_button(&format!("le-hol-x-{i}"), Vec2::new(x + w - 10.0, y + (ROW - 6.0) * 0.5), 10.0, "close", "Take the period out") {
                        out = Some(i);
                    }
                    if !p.valid() {
                        ui.p().rect(Rect::new(x + nw + 6.0, y + ROW - 7.0, 2.0 * dw + 6.0, 2.0), WARN);
                    }
                    y += ROW - 2.0;
                }
                if let Some(i) = out {
                    periods.remove(i);
                    periods_changed = true;
                }
                let half = (w - GAP) * 0.5;
                if ui.button("le-hol-add", Rect::new(x, y, half, ROW - 6.0), "Add a period", Some("add"), ButtonKind::Ghost) {
                    periods.push(service::HolidayPeriod::new("", "", ""));
                    periods_changed = true;
                }
                if ui.button("le-hol-default", Rect::new(x + half + GAP, y, half, ROW - 6.0), "Default holidays", Some("restart_alt"), ButtonKind::Ghost) {
                    periods = service::default_school_holidays();
                    periods_changed = true;
                }
                y += ROW + 2.0;
            }
            let next = omsi_map::date_code(&f.date).and_then(|c| cal.next_school_holidays(c));
            if let Some((name, from, to)) = next {
                let name = if name.trim().is_empty() { "–".to_string() } else { omsi_ui::tr(name.trim()).into_owned() };
                let text = omsi_ui::tr("Next: %{name}, %{from} to %{to}").replace("%{name}", &name).replace("%{from}", &day_of_code(from)).replace("%{to}", &day_of_code(to));
                y += ui.paragraph(&text, Vec2::new(x, y), w, 11.5, Weight::Regular, TEXT_SOFT) + 6.0;
            }
        }
        // the buses that run it
        ui.p().rect(Rect::new(x, y, w, 1.0), HAIRLINE);
        y += 10.0;
        caps(ui, y, "Buses on this line");
        y += 22.0;
        let (mut px, mut py) = (x, y);
        for c in VehicleClass::ALL {
            let label = omsi_ui::tr(c.label()).into_owned();
            // (a kind the company's level keeps locked: with its lock, and its popup)
            let shut = f.locked.contains(&c.size());
            let pw = ui.width(&label, 12.5, Weight::Medium) + 26.0 + if shut { 18.0 } else { 0.0 };
            if px > x && px + pw > x + w {
                px = x;
                py += 34.0;
            }
            let r = Rect::new(px, py, pw, 28.0);
            let on = want.classes.contains(&c);
            let (h, _, clicked) = ui.interact(super::ui::id_of(&format!("le-class-{c:?}")), r);
            ui.p().rounded(r, 14.0, if on { accent() } else if h { HOVER } else { FIELD });
            if !on {
                ui.p().rounded_border(r, 14.0, 1.0, EDGE);
            }
            if shut {
                ui.icon("lock", Vec2::new(r.x + 16.0, r.center().y), 12.0, TEXT_FAINT);
                ui.text_in(&label, Rect::new(r.x + 18.0, r.y, r.w - 18.0, r.h), 12.5, Weight::Medium, if on { on_accent() } else { TEXT_DIM }, Align::Center);
                ui.tooltip(r, &omsi_ui::tr("Your company's level does not open these buses yet"));
            } else {
                ui.text_in(&label, r, 12.5, Weight::Medium, if on { on_accent() } else { TEXT }, Align::Center);
            }
            if clicked {
                // (a locked kind may still be taken off a line that asks for it)
                if shut && !on {
                    level_ask = Some(c.size());
                } else {
                    toggle = Some(c);
                }
            }
            px += pw + 6.0;
        }
        y = py + 38.0;
        if !kind.suits().is_empty() && want.classes != kind.suits() {
            let names: Vec<String> = kind.suits().iter().map(|c| omsi_ui::tr(c.label()).into_owned()).collect();
            let text = omsi_ui::tr("Suits this kind: %{buses}").replace("%{buses}", &names.join(", "));
            let bw = 96.0;
            ui.paragraph(&text, Vec2::new(x, y), w - bw - 8.0, 11.5, Weight::Regular, TEXT_DIM);
            if ui.button("le-class-suits", Rect::new(x + w - bw, y - 4.0, bw, 28.0), "Take these", None, ButtonKind::Ghost) {
                take_suits = true;
            }
            y += 34.0;
        }
        for (i, p) in want.buses.iter().enumerate() {
            let r = Rect::new(x, y, w, 30.0);
            ui.p().rounded(r, RADIUS, FIELD);
            ui.icon("directions_bus", Vec2::new(r.x + 14.0, r.center().y), 15.0, accent_2());
            ui.text_in(&p.label, Rect::new(r.x + 28.0, r.y, r.w - 56.0, r.h), 12.5, Weight::Medium, TEXT, Align::Left);
            if ui.icon_button(&format!("le-bus-x-{i}"), Vec2::new(r.right() - 14.0, r.center().y), 10.0, "close", "Take it out") {
                remove = Some(i);
            }
            y += 34.0;
        }
        // a maker, a model of it, a version of that
        let mut m = pick.0;
        if ui.select("le-pick-maker", Rect::new(x, y, w, ROW - 4.0), &mut m, &makers) && m != pick.0 {
            pick = (m, 0, 0);
        }
        y += ROW;
        if let Some((_, models)) = pick.0.checked_sub(1).and_then(|k| cat.get(k)) {
            let names: Vec<String> = std::iter::once(omsi_ui::tr("All its models").into_owned()).chain(models.iter().map(|x| x.0.clone())).collect();
            let mut k = pick.1;
            if ui.select("le-pick-model", Rect::new(x, y, w, ROW - 4.0), &mut k, &names) && k != pick.1 {
                pick = (pick.0, k, 0);
            }
            y += ROW;
            if let Some((_, versions)) = pick.1.checked_sub(1).and_then(|k| models.get(k)) {
                let names: Vec<String> = std::iter::once(omsi_ui::tr("All its versions").into_owned()).chain(versions.iter().map(|x| x.1.clone())).collect();
                let mut j = pick.2;
                if ui.select("le-pick-version", Rect::new(x, y, w, ROW - 4.0), &mut j, &names) {
                    pick.2 = j;
                }
                y += ROW;
            }
            if ui.button("le-pick-add", Rect::new(x, y, w, ROW - 6.0), "Add to the line", Some("add"), ButtonKind::Normal) {
                add = pick_of(&cat, pick.0 - 1, pick.1, pick.2);
            }
            y += ROW;
        }
        // what that comes to
        let text = if want.open() {
            omsi_ui::tr("Any bus of its depot group drives it. Choose kinds of bus or buses by name to have the line driven with them.").into_owned()
        } else if f.classes.busy() {
            omsi_ui::tr("Reading the buses…").into_owned()
        } else {
            omsi_ui::tr("%{n} of the installed buses fit. The timetable's buses on the line are such buses where the map's depots have them.").replace("%{n}", &matching.to_string())
        };
        y += ui.paragraph(&text, Vec2::new(x, y), w, 11.5, Weight::Regular, if !want.open() && matching == 0 && !f.classes.busy() { WARN } else { TEXT_DIM }) + 6.0;
        if let (Some(n), false) = (in_fleet, want.open()) {
            let (text, c) = if n == 0 { (omsi_ui::tr("None of the company's buses fits: the planning cannot put one on its tours.").into_owned(), WARN) } else { (omsi_ui::tr("%{n} of the company's buses fit.").replace("%{n}", &n.to_string()), TEXT_SOFT) };
            y += ui.paragraph(&text, Vec2::new(x, y), w, 11.5, Weight::Regular, c) + 6.0;
        }
        y - a.y + 12.0
    });
    v.pick = pick;
    let for_company = v.company.is_some();
    let mut changed = false;
    if title_changed {
        v.line_mut().unwrap().title = title;
        changed = true;
    }
    if periods_changed {
        v.reg.school_holidays = periods;
        changed = true;
    }
    let line = v.line_mut().unwrap();
    if let Some(k) = new_kind {
        // (a timetable the player left as it came follows the kind)
        if line.days == typical_days(line.service, for_company) {
            line.days = typical_days(k, for_company);
        }
        line.service = k;
        changed = true;
    }
    if typical {
        line.days = typical_days(line.service, for_company);
        changed = true;
    }
    if let Some(c) = toggle {
        line.vehicles.toggle_class(c);
        changed = true;
    }
    if take_suits {
        // (what the level keeps locked stays out)
        line.vehicles.classes = line.service.suits().iter().copied().filter(|k| !f.locked.contains(&k.size())).collect();
        changed = true;
    }
    if let Some(i) = remove {
        line.vehicles.buses.remove(i);
        changed = true;
    }
    if let Some(p) = add.filter(|p| !line.vehicles.buses.iter().any(|x| x.maker == p.maker && x.model == p.model && x.file == p.file)) {
        line.vehicles.buses.push(p);
        changed = true;
    }
    if changed {
        v.touched();
    }
    if level_ask.is_some() {
        v.level_ask = level_ask;
    }
}

// --- the timetable as a table -------------------------------------------------------------------

/// A time of the table as it is shown: "07:05" (a trip past midnight from 00:00 on).
fn hm(minutes: f32) -> String {
    let m = minutes.round() as i32;
    format!("{:02}:{:02}", m.div_euclid(60).rem_euclid(24), m.rem_euclid(60))
}

/// A time typed into the table: "7:05", "07.05", "0705" or "705" (None: not a time).
fn parse_hm(s: &str) -> Option<i32> {
    let s = s.trim();
    let (h, m) = match s.split_once([':', '.', 'h']) {
        Some((h, m)) => (h.trim().parse::<i32>().ok()?, m.trim().parse::<i32>().ok()?),
        None if s.len() >= 3 && s.chars().all(|c| c.is_ascii_digit()) => (s[..s.len() - 2].parse().ok()?, s[s.len() - 2..].parse().ok()?),
        None => return None,
    };
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some(h * 60 + m)
}

/// What the table's tools asked for (done after it is drawn).
enum TableAct {
    Select(usize, usize),
    /// A time typed for the cell chosen, and on to the next stop.
    Typed(i32, bool),
    Nudge(f32),
    Add,
    Copy(f32),
    Shift(f32),
    Delete,
    Fill,
    UseTable(bool),
    Dir(usize),
    Day(usize),
    Page(i32),
    Close,
}

/// The timetable as a table over the map (after City Bus Manager's): the stops of a direction
/// down, its trips of a group of days across, every departure a cell to type a time into
/// ("0705", the arrow keys a minute later or earlier); a trip added, copied 10 to 30 minutes
/// later, moved as a whole or taken out; filled again from the day patterns. A stop reached
/// before the one before it is marked, and the line is not saved so.
fn table_layer(l: &mut Launcher, map_r: Rect) {
    let Launcher { ui, pages, .. } = l;
    let v = &mut pages.lines;
    let Some(line) = v.line().cloned() else {
        v.table_open = false;
        return;
    };
    let kind = line.service;
    if !kind.allows(v.table_day) {
        v.table_day = (0..DAY_GROUPS.len()).find(|k| kind.allows(*k)).unwrap_or(0);
    }
    let (dir, day) = (v.dir.min(line.directions.len().saturating_sub(1)), v.table_day);
    let r = Rect::new(map_r.x + 8.0, map_r.y + 8.0, map_r.w - 16.0, map_r.h - 16.0);
    ui.panel(r);
    let x = r.x + 16.0;
    let w = r.w - 32.0;
    let mut y = r.y + 12.0;
    let mut act: Option<TableAct> = None;
    // the head: the line, the direction, the days, closing
    let title = omsi_ui::tr("Timetable of line %{n}").replace("%{n}", line.number.trim());
    ui.text_in(&title, Rect::new(x, y, 190.0, ROW - 4.0), 15.0, Weight::Bold, TEXT, Align::Left);
    if ui.icon_button("le-table-close", Vec2::new(r.right() - 24.0, y + (ROW - 4.0) * 0.5), 14.0, "close", "Back to the map") {
        act = Some(TableAct::Close);
    }
    let mut d = dir;
    let seg_w = ((w - 200.0 - 40.0) * 0.45).max(150.0);
    if ui.segmented("le-table-dir", Rect::new(x + 200.0, y, seg_w, ROW - 4.0), &mut d, &["Outbound", "Return"]) {
        act = Some(TableAct::Dir(d));
    }
    let labels: Vec<&str> = DAY_GROUPS.iter().map(|g| g.0).collect();
    let off: Vec<usize> = (0..DAY_GROUPS.len()).filter(|k| !kind.allows(*k)).collect();
    let mut dd = day;
    if ui.segmented_some("le-table-day", Rect::new(x + 210.0 + seg_w, y, (w - 250.0 - seg_w).max(150.0), ROW - 4.0), &mut dd, &labels, &off) {
        act = Some(TableAct::Day(dd));
    }
    y += ROW + 4.0;
    // whether it is the line's timetable, and filling it from the patterns
    let mut on = line.table_on;
    if ui.toggle("le-table-use", Rect::new(x, y, 260.0f32.min(w * 0.45), ROW - 4.0), &mut on, "The table is the line's timetable") {
        act = Some(TableAct::UseTable(on));
    }
    let fw = 230.0f32.min(w * 0.45);
    // (its kind of service between: on which days its trips run)
    let kw = w - 260.0f32.min(w * 0.45) - fw - 20.0;
    if kw > 60.0 {
        let kr = Rect::new(x + 260.0f32.min(w * 0.45) + 10.0, y, kw, ROW - 4.0);
        let label = omsi_ui::tr(kind.label());
        let lw = ui.width(&label, 12.0, Weight::Medium).min(kw - 24.0);
        ui.icon(kind.icon(), Vec2::new(kr.center().x - lw * 0.5 - 10.0, kr.center().y), 15.0, accent_2());
        ui.text_in(&label, Rect::new(kr.center().x - lw * 0.5 + 2.0, kr.y, lw + 4.0, kr.h), 12.0, Weight::Medium, accent_2(), Align::Left);
        ui.tooltip(kr, kind.note());
    }
    let fill_label = if v.table_fill_armed { "Press again: the table anew" } else { "Fill from the patterns" };
    if ui.button("le-table-fill", Rect::new(x + w - fw, y, fw, ROW - 4.0), fill_label, Some("restart_alt"), ButtonKind::Ghost) {
        act = Some(TableAct::Fill);
    }
    y += ROW + 2.0;
    // the trips of the direction and the days, in order of departure
    let mut cols: Vec<usize> = (0..line.table.len()).filter(|&i| line.table[i].dir == dir && line.table[i].day == day).collect();
    cols.sort_by(|a, b| line.table[*a].departure().total_cmp(&line.table[*b].departure()));
    let sel = v.table_sel.filter(|(i, _)| cols.contains(i));
    // the tools for the trip chosen
    let bh = ROW - 8.0;
    let mut bx = x;
    if ui.button("le-table-add", Rect::new(bx, y, 120.0, bh), "Add a trip", Some("add"), ButtonKind::Normal) {
        act = Some(TableAct::Add);
    }
    bx += 128.0;
    if sel.is_some() {
        ui.text_in("Copy", Rect::new(bx, y, 44.0, bh), 12.0, Weight::Medium, TEXT_DIM, Align::Left);
        bx += 44.0;
        for m in [10.0, 15.0, 20.0, 30.0] {
            let br = Rect::new(bx, y, 46.0, bh);
            if ui.button(&format!("le-table-copy-{m}"), br, &format!("+{m:.0}"), None, ButtonKind::Ghost) {
                act = Some(TableAct::Copy(m));
            }
            ui.tooltip(br, &omsi_ui::tr("A copy of the trip, %{n} minutes later").replace("%{n}", &format!("{m:.0}")));
            bx += 48.0;
        }
        bx += 10.0;
        ui.text_in("Move", Rect::new(bx, y, 46.0, bh), 12.0, Weight::Medium, TEXT_DIM, Align::Left);
        bx += 46.0;
        for (m, label) in [(-5.0, "-5"), (-1.0, "-1"), (1.0, "+1"), (5.0, "+5")] {
            let br = Rect::new(bx, y, 40.0, bh);
            if ui.button(&format!("le-table-shift-{label}"), br, label, None, ButtonKind::Ghost) {
                act = Some(TableAct::Shift(m));
            }
            ui.tooltip(br, "The whole trip so many minutes earlier or later");
            bx += 42.0;
        }
        if ui.icon_button("le-table-delete", Vec2::new(bx + 18.0, y + bh * 0.5), 12.0, "delete", "Take the trip out") {
            act = Some(TableAct::Delete);
        }
    } else {
        ui.text_in("Choose a time in the table: type it (0705), the arrow keys a minute later or earlier.", Rect::new(bx, y, x + w - bx, bh), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
    }
    y += ROW;
    // the grid: the stops down, the trips across
    let stops: Vec<String> = line.directions.get(dir).map(|d| d.stops.iter().map(|s| s.name.clone()).collect()).unwrap_or_default();
    let foot_h = 46.0;
    let grid = Rect::new(x, y, w, r.bottom() - 12.0 - foot_h - y);
    let name_w = 160.0f32.min(w * 0.3);
    let col_w = 62.0;
    let per_page = (((grid.w - name_w - 8.0) / col_w).floor() as usize).max(1);
    let first = v.table_col.min(cols.len().saturating_sub(per_page));
    let shown: Vec<usize> = cols.iter().copied().skip(first).take(per_page).collect();
    let head_h = 28.0;
    // (the pages of trips: a few at a time)
    if cols.len() > per_page {
        if ui.icon_button("le-table-prev", Vec2::new(grid.x + 12.0, grid.y + head_h * 0.5), 12.0, "chevron_left", "Earlier trips") {
            act = Some(TableAct::Page(-(per_page as i32)));
        }
        if ui.icon_button("le-table-next", Vec2::new(grid.x + 40.0, grid.y + head_h * 0.5), 12.0, "chevron_right", "Later trips") {
            act = Some(TableAct::Page(per_page as i32));
        }
        let span = format!("{}–{} / {}", first + 1, first + shown.len(), cols.len());
        ui.text_in(&span, Rect::new(grid.x + 56.0, grid.y, name_w - 60.0, head_h), 11.5, Weight::Medium, TEXT_DIM, Align::Left);
    }
    for (c, &i) in shown.iter().enumerate() {
        let cx = grid.x + name_w + c as f32 * col_w;
        let hr = Rect::new(cx + 2.0, grid.y + 2.0, col_w - 4.0, head_h - 4.0);
        let chosen = sel.is_some_and(|s| s.0 == i);
        let (h, _, clicked) = ui.interact(super::ui::id_of(&format!("le-table-head-{i}")), hr);
        ui.p().rounded(hr, 6.0, if chosen { accent() } else if h { HOVER } else { FIELD });
        let label = omsi_ui::tr("Trip %{n}").replace("%{n}", &(first + c + 1).to_string());
        ui.text_in(&label, hr, 11.5, Weight::Bold, if chosen { on_accent() } else { TEXT_SOFT }, Align::Center);
        if clicked {
            act = Some(TableAct::Select(i, sel.map(|s| s.1).unwrap_or(0)));
        }
    }
    if cols.is_empty() {
        let text = if line.table.is_empty() { "The table is empty: fill it from the patterns, or add a trip." } else { "No trip of this direction on these days: add one." };
        ui.text_in(text, Rect::new(grid.x, grid.y + head_h + 8.0, grid.w, 24.0), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
    }
    let rows = Rect::new(grid.x - 6.0, grid.y + head_h, grid.w + 12.0, grid.h - head_h);
    let row_h = 28.0;
    let mut text = v.table_text.clone();
    let table = line.table.clone();
    ui.scroll_area(&format!("le-table-rows-{dir}-{day}"), rows, &mut |ui, a| {
        let gx = a.x + 6.0;
        for (s, name) in stops.iter().enumerate() {
            let ry = a.y + s as f32 * row_h;
            if s % 2 == 1 {
                ui.p().rect(Rect::new(gx, ry, a.w - 12.0, row_h), Color::WHITE.alpha(0.025));
            }
            ui.text_in(name, Rect::new(gx + 4.0, ry, name_w - 10.0, row_h), 12.0, Weight::Medium, TEXT_SOFT, Align::Left);
            for (c, &i) in shown.iter().enumerate() {
                let t = &table[i];
                let cell = Rect::new(gx + name_w + c as f32 * col_w + 2.0, ry + 2.0, col_w - 4.0, row_h - 4.0);
                let Some(&time) = t.times.get(s) else { continue };
                // (a stop reached before the one before it)
                let falls = s > 0 && t.times.get(s - 1).is_some_and(|p| time < *p - 1e-3);
                if sel == Some((i, s)) {
                    let placeholder = hm(time);
                    ui.text_input("le-table-cell", cell, &mut text, &placeholder, None);
                    if ui.input.keys.contains(&Key::Enter) {
                        act = Some(match parse_hm(&text) {
                            Some(m) => TableAct::Typed(m, true),
                            None => TableAct::Select(i, (s + 1).min(stops.len().saturating_sub(1))),
                        });
                    } else if let Some(m) = parse_hm(&text).filter(|_| text.trim().len() >= 4) {
                        act = Some(TableAct::Typed(m, false));
                    } else if ui.input.keys.contains(&Key::Up) {
                        act = Some(TableAct::Nudge(1.0));
                    } else if ui.input.keys.contains(&Key::Down) {
                        act = Some(TableAct::Nudge(-1.0));
                    }
                    continue;
                }
                let (h, _, clicked) = ui.interact(super::ui::id_of(&format!("le-table-cell-{i}-{s}")), cell);
                if falls {
                    ui.p().rounded(cell, 5.0, WARN.alpha(0.28));
                } else if h || sel.is_some_and(|x| x.0 == i) {
                    ui.p().rounded(cell, 5.0, HOVER);
                }
                ui.text_in(&hm(time), cell, 12.5, if s == 0 { Weight::Bold } else { Weight::Regular }, if falls { WARN } else { TEXT }, Align::Center);
                if clicked {
                    act = Some(TableAct::Select(i, s));
                }
            }
        }
        stops.len() as f32 * row_h + 8.0
    });
    v.table_text = text;
    // the foot: how many trips, buses and tours; what is wrong
    let falls: Vec<(usize, usize)> = cols.iter().enumerate().filter_map(|(k, &i)| line.table[i].first_fall().map(|s| (k, s))).collect();
    let fy = r.bottom() - 12.0 - foot_h;
    let (note, colour) = if let Some(&(k, s)) = falls.first() {
        let stop = stops.get(s).cloned().unwrap_or_default();
        (omsi_ui::tr("Trip %{n} reaches %{stop} before it left the stop before: the line is not saved so.").replace("%{n}", &(k + 1).to_string()).replace("%{stop}", &stop), WARN)
    } else if !line.table_on {
        (omsi_ui::tr("The day patterns make the line's timetable: switch the table on to make it this one.").into_owned(), TEXT_DIM)
    } else {
        let tours = reg::table_tours(&line).into_iter().filter(|t| t.day == day).collect::<Vec<_>>();
        (omsi_ui::tr("%{t} trips on these days, driven in %{n} tours").replace("%{t}", &line.table.iter().filter(|t| t.day == day).count().to_string()).replace("%{n}", &tours.len().to_string()), TEXT_SOFT)
    };
    ui.paragraph(&note, Vec2::new(x, fy + 4.0), w, 12.0, Weight::Regular, colour);
    let away = line.table.iter().filter(|t| !kind.allows(t.day)).count();
    if away > 0 {
        let text = omsi_ui::tr("%{n} trips on days %{kind} does not run: kept, not driven.").replace("%{n}", &away.to_string()).replace("%{kind}", &omsi_ui::tr(kind.label()));
        ui.text_in(&text, Rect::new(x, fy + 26.0, w, 18.0), 11.5, Weight::Regular, TEXT_DIM, Align::Left);
    }
    // what was asked for
    let Some(act) = act else { return };
    let headway = line.days.get(day).map(|p| if p.bands.is_empty() { p.headway } else { p.clean_bands().first().map(|b| b.headway).unwrap_or(p.headway) }).unwrap_or(30.0).max(1.0);
    let mut changed = true;
    match act {
        TableAct::Close => {
            v.table_open = false;
            changed = false;
        }
        TableAct::Dir(d) => {
            (v.dir, v.table_sel, v.table_col) = (d, None, 0);
            changed = false;
        }
        TableAct::Day(k) => {
            (v.table_day, v.table_sel, v.table_col) = (k, None, 0);
            changed = false;
        }
        TableAct::Page(by) => {
            v.table_col = (first as i32 + by).max(0) as usize;
            changed = false;
        }
        TableAct::Select(i, s) => {
            v.table_sel = Some((i, s));
            v.table_text.clear();
            ui.focus = Some(super::ui::id_of("le-table-cell"));
            changed = false;
        }
        TableAct::UseTable(on) => {
            let line = v.line_mut().unwrap();
            line.table_on = on;
            if on && line.table.is_empty() {
                line.table = reg::table_from_patterns(line);
            }
        }
        TableAct::Fill => {
            if v.table_fill_armed || line.table.is_empty() {
                let l2 = v.line_mut().unwrap();
                l2.table = reg::table_from_patterns(l2);
                (v.table_sel, v.table_col, v.table_fill_armed) = (None, 0, false);
            } else {
                v.table_fill_armed = true;
                changed = false;
            }
        }
        TableAct::Add => {
            // (after the last trip, a headway later; the first from the pattern's first departure)
            let after = cols.last().map(|&i| line.table[i].clone());
            let rel = line.directions.get(dir).map(reg::fitted_times).unwrap_or_default();
            let first_dep = line.days.get(day).map(|p| p.departures().first().map(|x| x.0).unwrap_or(p.first)).unwrap_or(360.0);
            let trip = match after {
                Some(mut t) => {
                    t.shift(headway);
                    t
                }
                None => reg::TableTrip { dir, day, times: rel.iter().map(|x| first_dep + x).collect() },
            };
            let line = v.line_mut().unwrap();
            line.table.push(trip);
            v.table_sel = Some((line.table.len() - 1, 0));
            v.table_col = cols.len().saturating_sub(per_page - 1);
        }
        TableAct::Copy(m) => {
            if let Some((i, s)) = sel {
                let line = v.line_mut().unwrap();
                let mut t = line.table[i].clone();
                t.shift(m);
                line.table.push(t);
                v.table_sel = Some((line.table.len() - 1, s));
            }
        }
        TableAct::Shift(m) => {
            if let Some((i, _)) = sel {
                v.line_mut().unwrap().table[i].shift(m);
            }
        }
        TableAct::Delete => {
            if let Some((i, _)) = sel {
                v.line_mut().unwrap().table.remove(i);
                v.table_sel = None;
            }
        }
        TableAct::Nudge(m) => {
            if let Some((i, s)) = sel {
                if let Some(t) = v.line_mut().unwrap().table[i].times.get_mut(s) {
                    *t = (*t + m).max(0.0);
                }
                v.table_text.clear();
            }
        }
        TableAct::Typed(m, next) => {
            if let Some((i, s)) = sel {
                if let Some(t) = v.line_mut().unwrap().table[i].times.get_mut(s) {
                    // (a time after midnight in a trip of the evening is the next day's)
                    let before = *t;
                    let mut m = m as f32;
                    if before >= 1440.0 || before - m > 720.0 {
                        m += 1440.0;
                    }
                    *t = m;
                }
                v.table_text.clear();
                if next {
                    v.table_sel = Some((i, (s + 1).min(stops.len().saturating_sub(1))));
                    ui.focus = Some(super::ui::id_of("le-table-cell"));
                }
            }
        }
    }
    if changed {
        v.table_fill_armed = false;
        v.touched();
    }
}

// --- working for the bus company ---------------------------------------------------------------

/// A new line in the registry, chosen: the player's own, or - working for the company - the
/// company's draft in its colour, driven by its depot's buses.
fn new_line(v: &mut LineEditorView) {
    let group = match &v.company {
        Some(fc) => company_group(&v.groups, &fc.depot),
        None => v.groups.first().map(|g| g.0.clone()).unwrap_or_default(),
    };
    let n = v.reg.lines.len();
    let line = v.reg.add_line(&group);
    line.colour = PALETTE[n % PALETTE.len()].to_string();
    line.name = format!("{} {}", omsi_ui::tr("Line"), line.number);
    if let Some(fc) = &v.company {
        line.company = fc.id.clone();
        line.draft = true;
        line.colour = fc.colours[0].clone();
        // (the company's lines start with the rush hours in their timetable)
        for (k, p) in line.days.iter_mut().enumerate() {
            p.bands = reg::default_bands(k);
        }
    }
    let id = line.id;
    (v.sel, v.dir, v.sel_stop) = (Some(id), 0, None);
    v.touched();
}

/// The depot group whose depot file is the company's (else the first).
fn company_group(groups: &[(String, String)], depot: &str) -> String {
    let file = |s: &str| {
        let name = s.trim().replace('\\', "/").rsplit('/').next().unwrap_or("").to_lowercase();
        name.strip_suffix(".hof").map(str::to_string).unwrap_or(name)
    };
    groups.iter().find(|g| !depot.trim().is_empty() && file(&g.1) == file(depot)).or(groups.first()).map(|g| g.0.clone()).unwrap_or_default()
}

/// The company's wish once its map is read: a new line, or the line it asked for.
fn company_start(l: &mut Launcher) {
    let v = &mut l.pages.lines;
    if v.loaded.is_none() {
        return;
    }
    let Some(start) = v.company.as_mut().and_then(|f| f.start.take()) else { return };
    match start {
        Some(id) if v.reg.line(id).is_some() => (v.sel, v.dir, v.sel_stop) = (Some(id), 0, None),
        Some(_) => {}
        None => {
            new_line(v);
            l.state.set_status(omsi_ui::tr("Click the line's stops on the map; the estimate shows what it costs and brings.").into_owned(), false);
        }
    }
}

/// The company's figures of the line shown (made again when the line or the company's money
/// changed): its shape with the other lines at its stops, the estimate, and what saving costs.
fn company_figures(l: &mut Launcher) {
    let Launcher { pages, company, .. } = l;
    let company = &*company;
    let v = &mut pages.lines;
    let (Some(fc), Some(c)) = (v.company.as_mut(), company.company.as_ref()) else { return };
    let Some(line) = v.sel.and_then(|id| v.reg.lines.iter().find(|x| x.id == id)) else {
        fc.cache = None;
        return;
    };
    if fc.stop_lines.as_ref().map(|s| s.0) != Some(line.id) {
        if let Some(lines) = super::company::map_lines(company) {
            let stem = reg::stems(&v.reg).get(&line.id).cloned().unwrap_or_default();
            fc.stop_lines = Some((line.id, ownline::StopLines::of(lines, &stem)));
        }
    }
    let known = fc.stop_lines.as_ref().is_some_and(|s| s.0 == line.id);
    let key = (line.id, v.revision * 2 + known as u64, c.cash, c.lines.len());
    if fc.cache.as_ref().is_some_and(|f| f.key == key) {
        return;
    }
    let sl = fc.stop_lines.as_ref().filter(|s| s.0 == line.id).map(|s| &s.1);
    let shape = ownline::shape_of(line, &|s| sl.map(|x| x.count(s)).unwrap_or(0));
    let est = ownline::estimate(c, line, &shape);
    let plan = ownline::line_of(c, line.id).and_then(|x| x.plan.as_ref());
    let (confirmed, paid) = (plan.is_some(), plan.map(|p| p.paid).unwrap_or(0));
    let fee = if confirmed { ownline::change_fee(c, line, &shape) } else { None };
    fc.cache = Some(Figures { key, shape, est, confirmed, fee, cash: c.cash, paid });
}

/// The foot of the left panel working for the company: a draft is confirmed and paid (or
/// saved, or deleted); a line the company runs is saved - paying its route's approval anew
/// when the stops changed - or its changes undone.
#[allow(clippy::too_many_arguments)]
fn company_foot(ui: &mut Ui, v: &mut LineEditorView, state: &mut super::state::State, body: Rect, foot: f32, problems: &[reg::Problem], f: Option<&Figures>, status: &mut Option<(String, bool)>) {
    let half = (body.w - GAP) * 0.5;
    let Some(f) = f else {
        ui.paragraph("Reading the company…", Vec2::new(body.x, foot - 30.0), body.w, 11.5, Weight::Regular, TEXT_DIM);
        return;
    };
    let total = f.est.one_off_total();
    let why: Option<String> = if let Some(p) = problems.first() {
        Some(omsi_ui::tr(p.text).replace("%{n}", &p.n.to_string()))
    } else if !f.confirmed && f.cash < total {
        Some(omsi_ui::tr("The company has %{cash}: not enough for the line. A loan on the Finances page helps.").replace("%{cash}", &super::company::eur(f.cash)))
    } else if f.confirmed && f.fee.is_some_and(|x| x > f.cash) {
        Some(omsi_ui::tr("The company has %{cash}: not enough for the change.").replace("%{cash}", &super::company::eur(f.cash)))
    } else {
        None
    };
    if let Some(w) = &why {
        let h = ui.paragraph_height(w, body.w, 11.5, Weight::Regular);
        ui.paragraph(w, Vec2::new(body.x, foot - ROW - 14.0 - h), body.w, 11.5, Weight::Regular, WARN);
    }
    if !f.confirmed {
        let label = omsi_ui::tr("Confirm and pay %{amount}").replace("%{amount}", &super::company::eur(total));
        if ui.button("le-confirm", Rect::new(body.x, foot - ROW - 8.0, body.w, ROW), &label, Some("check_circle"), if why.is_none() { ButtonKind::Primary } else { ButtonKind::Normal }) {
            match why {
                Some(w) => *status = Some((w, true)),
                None => v.company_act = Some(CompanyAct::Confirm),
            }
        }
        if ui.button("le-save", Rect::new(body.x, foot, half, ROW), if v.dirty { "Save draft" } else { "Saved" }, Some("save"), ButtonKind::Normal) && v.dirty {
            *status = Some(save(v, state, problems));
        }
        if ui.button("le-delete", Rect::new(body.x + half + GAP, foot, half, ROW), if v.delete_armed { "Press again" } else { "Delete draft" }, Some("delete"), ButtonKind::Normal) {
            if v.delete_armed {
                *status = Some(delete(v, state));
            } else {
                v.delete_armed = true;
                *status = Some(("Press \"Delete draft\" again: the draft goes".into(), false));
            }
        }
        return;
    }
    let label = match f.fee {
        Some(fee) if v.dirty => omsi_ui::tr("Pay %{amount} and save").replace("%{amount}", &super::company::eur(fee)),
        _ if v.dirty => omsi_ui::tr("Save line").into_owned(),
        _ => omsi_ui::tr("Saved").into_owned(),
    };
    if ui.button("le-save", Rect::new(body.x, foot - ROW - 8.0, body.w, ROW), &label, Some("save"), if v.dirty && why.is_none() { ButtonKind::Primary } else { ButtonKind::Normal }) && v.dirty {
        match why.clone() {
            Some(w) => *status = Some((w, true)),
            None => v.company_act = Some(CompanyAct::SaveChange),
        }
    }
    let waiting = v.line().map(|x| x.pending_from.clone()).filter(|d| !d.is_empty());
    if v.dirty {
        if ui.button("le-discard", Rect::new(body.x, foot, body.w, ROW), "Undo the changes", Some("restart_alt"), ButtonKind::Normal) {
            v.company_act = Some(CompanyAct::Discard);
        }
    } else if let Some(d) = waiting {
        // (a change waiting for its day: said over the buttons, and to be taken back)
        if why.is_none() {
            let t = omsi_ui::tr("The new timetable begins on %{date}.").replace("%{date}", &super::company::day_label(&d));
            ui.text_in(&t, Rect::new(body.x, foot - ROW - 34.0, body.w, 20.0), 11.5, Weight::Medium, accent_2(), Align::Left);
        }
        if ui.button("le-withdraw", Rect::new(body.x, foot, body.w, ROW), "Keep the line as it runs", Some("restart_alt"), ButtonKind::Normal) {
            v.company_act = Some(CompanyAct::Withdraw);
        }
    } else {
        ui.paragraph("The company runs this line. A changed route is approved anew; a change of a line in service begins on a day you choose. The line is given up on the company's Lines page.", Vec2::new(body.x, foot), body.w, 11.5, Weight::Regular, TEXT_DIM);
    }
}

/// What the company's buttons asked for: the line confirmed and paid, a change paid and
/// saved, or the changes undone.
fn company_action(l: &mut Launcher, act: CompanyAct, status: &mut Option<(String, bool)>) {
    let Some(id) = l.pages.lines.sel else { return };
    if act == CompanyAct::Discard {
        let v = &mut l.pages.lines;
        let (map, global) = (v.reg.map.clone(), v.reg.global.clone());
        v.reg = reg::load_registry(&v.reg_path);
        (v.reg.map, v.reg.global) = (map, global);
        if v.reg.line(id).is_none() {
            v.sel = v.first_shown();
        }
        (v.dirty, v.sel_stop, v.drag, v.delete_armed) = (false, None, None, false);
        v.shapes_for = None;
        v.revision += 1;
        *status = Some(("The changes are undone".into(), false));
        return;
    }
    let v = &l.pages.lines;
    let Some(line) = v.reg.line(id).cloned() else { return };
    let Some(f) = v.company.as_ref().and_then(|f| f.cache.clone()).filter(|f| f.key.0 == id) else { return };
    let stem = reg::stems(&v.reg).get(&id).cloned().unwrap_or_default();
    // (the line as the map's timetable has it now: saved, or waiting for its change's day)
    let live = reg::load_registry(&v.reg_path).line(id).map(|x| x.live.as_deref().cloned().unwrap_or_else(|| x.clone()));
    let Some(c) = l.company.company.as_ref() else { return };
    let running = ownline::line_of(c, id).filter(|x| x.service_from.is_some()).map(|x| x.name.clone());
    let diff = live.as_ref().filter(|old| reg::timetable_differs(old, &line)).map(|old| reg::tour_diff(old, &line));
    match act {
        CompanyAct::Withdraw => {
            withdraw(l, id, status);
            return;
        }
        // a line in service whose timetable changes: asked first from when
        CompanyAct::SaveChange if running.is_some() && diff.is_some() => {
            let today = c.date.clone();
            let from = Some(line.pending_from.clone()).filter(|d| co::dates::between(&today, d) >= 1).unwrap_or_else(|| co::dates::add(&today, 1));
            l.pages.lines.change_ask = Some(ChangeAsk { fee: f.fee.unwrap_or(0), diff: diff.unwrap_or_default(), today, from });
            return;
        }
        CompanyAct::Schedule(from) => {
            let (Some(old), Some(d)) = (live, diff) else { return };
            let replan = d.replan();
            let Some(paid) = super::company::act(l, |c| ownline::schedule_change(c, &line, &stem, &f.shape, &from, replan.clone())) else { return };
            let Launcher { pages, state, .. } = l;
            let v = &mut pages.lines;
            if let Some(x) = v.reg.line_mut(id) {
                (x.draft, x.live, x.pending_from) = (false, Some(Box::new(old)), from.clone());
            }
            v.dirty = true;
            let (text, err) = save(v, state, &[]);
            super::company::reload_timetable(l);
            let n = line.number.trim().to_string();
            let mut t = omsi_ui::tr("Line %{n}: the new timetable begins on %{date}; today's tours run as they are.").replace("%{n}", &n).replace("%{date}", &super::company::day_label(&from));
            if paid > 0 {
                t = format!("{t} {}", omsi_ui::tr("Its route is approved for %{amount}.").replace("%{amount}", &super::company::eur(paid)));
            }
            *status = Some(if err { (text, true) } else { (t, false) });
            return;
        }
        _ => {}
    }
    let paid = if act == CompanyAct::Confirm {
        super::company::act(l, |c| ownline::confirm(c, &line, &stem, &f.shape))
    } else {
        // (a change for now: the tours it changes lose their buses and drivers)
        let replan = diff.as_ref().map(|d| d.replan()).unwrap_or_default();
        super::company::act(l, |c| {
            let paid = ownline::apply_change(c, &line, &stem, &f.shape)?;
            co::plan::forget_tours(c, &stem, &replan);
            Ok(paid)
        })
    };
    // (refused: `act` said why)
    let Some(paid) = paid else { return };
    if let Some(x) = l.pages.lines.reg.line_mut(id) {
        (x.live, x.pending_from) = (None, String::new());
    }
    let Launcher { pages, state, .. } = l;
    let v = &mut pages.lines;
    if let Some(x) = v.reg.line_mut(id) {
        x.draft = false;
    }
    v.dirty = true;
    let (text, err) = save(v, state, &[]);
    super::company::reload_timetable(l);
    let number = line.number.trim().to_string();
    *status = Some(if err {
        (text, true)
    } else if act == CompanyAct::Confirm {
        (omsi_ui::tr("Line %{n} is the company's now: %{amount} paid. Its tours are on the Planning page.").replace("%{n}", &number).replace("%{amount}", &super::company::eur(paid)), false)
    } else if paid > 0 {
        (omsi_ui::tr("Line %{n} saved: its new route is approved for %{amount}.").replace("%{n}", &number).replace("%{amount}", &super::company::eur(paid)), false)
    } else {
        (text, false)
    });
}

/// A change waiting for its day taken back: the line stays as it runs (what was paid for
/// it stays paid).
fn withdraw(l: &mut Launcher, id: u64, status: &mut Option<(String, bool)>) {
    let Some(live) = l.pages.lines.reg.line(id).and_then(|x| x.live.as_deref().cloned()) else { return };
    let done = super::company::act(l, |c| {
        if let Some(x) = c.lines.iter_mut().find(|x| x.plan.as_ref().is_some_and(|p| p.line_id == id)) {
            x.pending = None;
        }
        Ok(())
    });
    if done.is_none() {
        return;
    }
    let Launcher { pages, state, .. } = l;
    let v = &mut pages.lines;
    if let Some(x) = v.reg.line_mut(id) {
        *x = LineDesign { id, live: None, pending_from: String::new(), ..live };
    }
    v.dirty = true;
    v.shapes_for = None;
    v.revision += 1;
    let (text, err) = save(v, state, &[]);
    *status = Some(if err { (text, true) } else { (omsi_ui::tr("The change is taken back: the line stays as it runs.").into_owned(), false) });
}

/// A change waiting for its day takes effect in the map's timetable: the line written as it
/// is now (the company's side: `ownline::take_effect`). The line editor open on the map
/// takes it too.
pub fn take_effect_in_timetable(l: &mut Launcher, map: &str, id: u64) -> Result<(), String> {
    let folder = reg::map_folder(map);
    let path = reg::registry_path(&folder);
    let mut r = reg::load_registry(&path);
    let flip = |x: &mut LineDesign| {
        x.live = None;
        x.pending_from.clear();
    };
    let Some(x) = r.line_mut(id) else { return Ok(()) };
    flip(x);
    if l.pages.lines.reg_path == path {
        if let Some(x) = l.pages.lines.reg.line_mut(id) {
            flip(x);
        }
    }
    reg::save_registry(&path, &r)?;
    let content = core::content_dir().ok_or("no content folder")?;
    let global = omsi_cfg::find_in_roots(map).map(|(_, p)| p).unwrap_or_else(|| omsi_cfg::resolve_path(Path::new(&l.state.config.root), map));
    let map_dir = global.parent().map(Path::to_path_buf).unwrap_or_default();
    reg::export_to_map(&content, &map_dir, &r)?;
    omsi_cfg::content_changed();
    l.state.load_lines();
    Ok(())
}

/// The estimate over the map's foot: what the line costs once and brings a month, its
/// passengers and its buses; opened, every item and the parts of a working day.
fn estimate_card(l: &mut Launcher, map_r: Rect) {
    let Some(fc) = l.pages.lines.company.as_ref() else { return };
    let Some(f) = fc.cache.clone().filter(|f| Some(f.key.0) == l.pages.lines.sel) else { return };
    let mut details = fc.details;
    let eur = super::company::eur;
    let e = &f.est;
    let ui = &mut l.ui;
    let w = (map_r.w - 24.0).min(800.0);
    let strip = Rect::new(map_r.x + 12.0, map_r.bottom() - 12.0 - 66.0, w, 66.0);
    ui.panel(strip);
    let cols = 4.0;
    let cw = (strip.w - 44.0) / cols;
    let result = e.result();
    let fleet: Vec<String> = e.fleet.iter().map(|(s, n)| format!("{n} × {}", size_label(Some(*s)))).collect();
    // (a school line's pupils on a school day; a weekend line's passengers on a Sunday)
    let kind = l.pages.lines.line().map(|x| x.service).unwrap_or_default();
    let n = |x: f64| format!("{x:.0}");
    let riders: (&str, String, String) = match kind {
        ServiceKind::School => ("Pupils a school day", n(e.passengers[0]), omsi_ui::tr("on school days only").into_owned()),
        ServiceKind::Leisure => ("Passengers a Sunday", n(e.passengers[2]), omsi_ui::tr("Saturday %{s}").replace("%{s}", &n(e.passengers[1]))),
        _ => ("Passengers a day", n(e.passengers[0]), omsi_ui::tr("Sat %{s} · Sun %{u}").replace("%{s}", &n(e.passengers[1])).replace("%{u}", &n(e.passengers[2]))),
    };
    let figures: [(&str, String, String, Color); 4] = [
        (if f.confirmed { "Paid to start" } else { "To start" }, eur(if f.confirmed { f.paid } else { e.one_off_total() }), if f.confirmed { omsi_ui::tr("the company runs it").into_owned() } else { omsi_ui::tr("licence, stops, tariff, launch").into_owned() }, TEXT),
        ("A month", eur(result), format!("{} {}  ·  {} {}", omsi_ui::tr("in"), eur(e.revenue()), omsi_ui::tr("out"), eur(e.costs())), if result >= 0 { OK } else { WARN }),
        (riders.0, riders.1, riders.2, TEXT),
        ("Buses peak / midday", format!("{} / {}", e.buses_peak, e.buses_offpeak), if fleet.is_empty() { omsi_ui::tr("none yet").into_owned() } else { fleet.join(", ") }, TEXT),
    ];
    for (k, (label, value, under, c)) in figures.iter().enumerate() {
        let x = strip.x + 14.0 + k as f32 * cw;
        ui.text_in(&omsi_ui::tr(label).to_uppercase(), Rect::new(x, strip.y + 8.0, cw - 10.0, 14.0), 10.0, Weight::Bold, TEXT_DIM, Align::Left);
        ui.text_in(value, Rect::new(x, strip.y + 22.0, cw - 10.0, 22.0), 16.0, Weight::Bold, *c, Align::Left);
        ui.text_in(under, Rect::new(x, strip.y + 44.0, cw - 10.0, 16.0), 11.0, Weight::Regular, TEXT_DIM, Align::Left);
    }
    if ui.icon_button("le-estimate-more", Vec2::new(strip.right() - 20.0, strip.center().y), 13.0, if details { "expand_more" } else { "expand_less" }, "The costs in detail") {
        details = !details;
    }
    if details {
        let h = (map_r.h - strip.h - 40.0).min(330.0);
        let r = Rect::new(strip.x, strip.y - 8.0 - h, strip.w, h);
        ui.panel(r);
        let gap = 18.0;
        let cw = (r.w - 32.0 - 2.0 * gap) / 3.0;
        let head = |ui: &mut Ui, text: &str, x: f32| ui.text_in(&omsi_ui::tr(text).to_uppercase(), Rect::new(x, r.y + 12.0, cw, 14.0), 10.0, Weight::Bold, TEXT_DIM, Align::Left);
        let item = |ui: &mut Ui, x: f32, y: f32, text: &str, amount: Cents, strong: bool| {
            let weight = if strong { Weight::Bold } else { Weight::Regular };
            ui.text_in(&omsi_ui::tr(text), Rect::new(x, y, cw - 74.0, 18.0), 11.5, weight, if strong { TEXT } else { TEXT_SOFT }, Align::Left);
            ui.text_in(&eur(amount), Rect::new(x + cw - 74.0, y, 74.0, 18.0), 11.5, weight, if amount < 0 { TEXT_SOFT } else { TEXT }, Align::Right);
        };
        // once
        let x0 = r.x + 16.0;
        head(ui, "Once", x0);
        let mut y = r.y + 32.0;
        for (text, amount) in &e.one_off {
            item(ui, x0, y, text, *amount, false);
            y += 20.0;
        }
        ui.p().rect(Rect::new(x0, y + 2.0, cw, 1.0), HAIRLINE);
        item(ui, x0, y + 6.0, "Together", e.one_off_total(), true);
        // a month
        let x1 = x0 + cw + gap;
        head(ui, "A month", x1);
        let mut y = r.y + 32.0;
        for (text, amount) in &e.monthly {
            item(ui, x1, y, text, *amount, false);
            y += 20.0;
        }
        ui.p().rect(Rect::new(x1, y + 2.0, cw, 1.0), HAIRLINE);
        item(ui, x1, y + 6.0, "Result", result, true);
        // a working day's parts: passengers, the bus, and whether it fits
        let x2 = x1 + cw + gap;
        head(ui, "A working day", x2);
        let mut y = r.y + 32.0;
        for (k, b) in e.bands.iter().enumerate() {
            let (a, z) = ownline::BAND_HOURS[k];
            ui.text_in(&format!("{}  {:02}–{:02}", omsi_ui::tr(ownline::BANDS[k]), a, z), Rect::new(x2, y, cw - 60.0, 18.0), 12.0, Weight::Medium, TEXT_SOFT, Align::Left);
            ui.text_in(&format!("{:.0}", b.passengers), Rect::new(x2 + cw - 60.0, y, 60.0, 18.0), 12.0, Weight::Bold, TEXT, Align::Right);
            y += 18.0;
            let (note, c) = if b.trips == 0 {
                (omsi_ui::tr("no buses").into_owned(), TEXT_FAINT)
            } else if b.crowded > 0 {
                (omsi_ui::tr("%{bus}: full on %{n} trips, %{need} would fit").replace("%{bus}", &size_label(b.size)).replace("%{n}", &b.crowded.to_string()).replace("%{need}", &size_label(b.needed)), WARN)
            } else if b.oversized > b.trips / 2 {
                (omsi_ui::tr("%{bus}: bigger than needed, %{need} would do").replace("%{bus}", &size_label(b.size)).replace("%{need}", &size_label(b.needed)), TEXT_DIM)
            } else {
                (omsi_ui::tr("%{bus}, up to %{n} on board").replace("%{bus}", &size_label(b.size)).replace("%{n}", &format!("{:.0}", b.load)), TEXT_DIM)
            };
            ui.text_in(&note, Rect::new(x2 + 8.0, y, cw - 8.0, 16.0), 11.0, Weight::Regular, c, Align::Left);
            y += 22.0;
        }
        let foot = omsi_ui::tr("Passengers from the stops, the transfers at them and the service; the rush hours busiest. A month: the drivers, buses and running at full cost.");
        ui.paragraph(&foot, Vec2::new(x0, r.bottom() - 36.0), r.w - 32.0, 10.5, Weight::Regular, TEXT_FAINT);
    }
    if let Some(fc) = l.pages.lines.company.as_mut() {
        fc.details = details;
    }
}

// --- the map ----------------------------------------------------------------------------------

/// What the map shows of the line: its legs (the direction worked on bright, the other faint,
/// a leg without a way red and straight) and the stops (the map's quiet, the line's in its
/// colour, the chosen ringed), with names beside the ends and the stop under the mouse.
fn map_layer(l: &mut Launcher, map_r: Rect) {
    let v = &l.pages.lines;
    let line = v.line();
    let colour = line.map(|x| colour_of(&x.colour)).unwrap_or(accent());
    let rev = v.revision;
    let shapes = v.shapes.clone();
    let dirs = line.map(|x| x.directions.clone()).unwrap_or_default();
    let cur = v.dir;
    l.mapview.editor_lines(rev, || {
        let mut out = Vec::new();
        for (k, legs) in shapes.iter().enumerate() {
            if k == cur {
                continue;
            }
            for s in legs {
                out.push((s.clone(), colour.alpha(0.35), 4.0));
            }
        }
        if let Some(legs) = shapes.get(cur) {
            for (i, s) in legs.iter().enumerate() {
                let ok = dirs.get(cur).and_then(|d| d.legs.get(i)).map(|g| g.ok).unwrap_or(false);
                out.push((s.clone(), if ok { colour } else { DANGER }, if ok { 5.0 } else { 3.0 }));
            }
        }
        out
    });
    let mut dots = Vec::with_capacity(v.stops.len() + 32);
    for s in &v.stops {
        dots.push(Dot { at: dv(s.at), fill: Color::rgba(149, 157, 176, 0.9), r: 3.2, ring: None });
    }
    for (k, d) in dirs.iter().enumerate().filter(|(k, _)| *k != cur) {
        let _ = k;
        for s in &d.stops {
            dots.push(Dot { at: dv(s.at), fill: colour.alpha(0.5), r: 4.0, ring: None });
        }
    }
    if let Some(d) = dirs.get(cur) {
        let n = d.stops.len();
        for (i, s) in d.stops.iter().enumerate() {
            let end = i == 0 || i + 1 == n;
            dots.push(Dot { at: dv(s.at), fill: if end { TEXT } else { colour }, r: if end { 6.5 } else { 5.0 }, ring: (v.sel_stop == Some(i)).then_some(LINE) });
        }
        for g in &d.legs {
            for p in &g.vias {
                dots.push(Dot { at: dv(*p), fill: TEXT, r: 3.5, ring: Some(colour) });
            }
        }
    }
    if let Some(i) = v.hover_stop {
        if let Some(s) = v.stops.get(i) {
            dots.push(Dot { at: dv(s.at), fill: TEXT, r: 4.5, ring: Some(LINE) });
        }
    }
    // the names: the ends of the direction and the stop under the mouse
    let mut labels: Vec<(DVec2, String, bool)> = Vec::new();
    if let Some(d) = dirs.get(cur) {
        for s in [d.stops.first(), d.stops.last()].into_iter().flatten() {
            labels.push((dv(s.at), s.name.clone(), false));
        }
    }
    if let Some(s) = v.hover_stop.and_then(|i| v.stops.get(i)) {
        labels.push((dv(s.at), s.name.clone(), true));
    }
    l.mapview.editor_dots(dots);
    if l.mapview.network().is_none() {
        return;
    }
    l.ui.push_clip(map_r, RADIUS);
    for (at, name, hover) in labels {
        let p = l.mapview.project(at);
        let w = l.ui.width(&name, 12.0, Weight::Medium) + 16.0;
        let r = Rect::new(p.x + 10.0, p.y - 11.0, w, 22.0);
        l.ui.p().rounded(r, 11.0, if hover { PANEL } else { ON_MAP });
        l.ui.text_in(&name, r, 12.0, Weight::Medium, TEXT, Align::Center);
    }
    if l.pages.lines.line().is_some_and(|x| x.directions.get(cur).is_some_and(|d| d.stops.is_empty())) {
        let hint = omsi_ui::tr("Click the line's first stop").to_string();
        let w = l.ui.width(&hint, 13.0, Weight::Medium) + 28.0;
        let r = Rect::new(map_r.center().x - w * 0.5, map_r.y + 14.0, w, 30.0);
        l.ui.p().rounded(r, 15.0, ON_MAP);
        l.ui.text_in(&hint, r, 13.0, Weight::Medium, TEXT, Align::Center);
    }
    l.ui.pop_clip();
}

/// Zoom and the whole map, in the map's corner.
fn map_buttons(l: &mut Launcher, map_r: Rect) {
    let x = map_r.right() - 26.0;
    for (k, (icon, tip)) in [("add", "Zoom in"), ("remove", "Zoom out"), ("near_me", "The whole map")].iter().enumerate() {
        let c = Vec2::new(x, map_r.y + 26.0 + k as f32 * 40.0);
        l.ui.solid(Rect::new(c.x - 17.0, c.y - 17.0, 34.0, 34.0));
        l.ui.p().circle(c, 17.0, ON_MAP);
        if l.ui.icon_button(&format!("le-zoom-{k}"), c, 14.0, icon, tip) {
            match k {
                0 => l.mapview.zoom_by(1.0 / 1.5),
                1 => l.mapview.zoom_by(1.5),
                _ => l.mapview.refit(),
            }
        }
    }
}

/// The mouse over the map: a stop clicked goes into the direction (after the stop chosen in
/// the list, else at its end) or is chosen there; a dragged point of a leg moves, a leg
/// dragged gets a new one; anything else moves and zooms the map.
fn map_pointer(l: &mut Launcher, map_r: Rect) {
    let mouse = l.ui.input.mouse;
    let over = map_r.contains(mouse) && !l.ui.over_ui;
    let ready = l.mapview.network().is_some();
    // what is under the mouse (the picture as it was drawn this frame)
    let (hover_stop, hover_via, hover_leg) = {
        let v = &l.pages.lines;
        let mut stop = None;
        let mut best = STOP_HIT;
        if over && ready {
            for (i, s) in v.stops.iter().enumerate() {
                let d = (l.mapview.project(dv(s.at)) - mouse).length();
                if d < best {
                    best = d;
                    stop = Some(i);
                }
            }
        }
        let mut via = None;
        let mut leg = None;
        if let (true, Some(d)) = (over && ready, v.line().and_then(|x| x.directions.get(v.dir))) {
            for (k, g) in d.legs.iter().enumerate() {
                for (j, p) in g.vias.iter().enumerate() {
                    if (l.mapview.project(dv(*p)) - mouse).length() < VIA_HIT {
                        via = Some((k, j));
                    }
                }
            }
            if stop.is_none() && via.is_none() {
                for (k, s) in v.shapes.get(v.dir).map(|x| x.as_slice()).unwrap_or(&[]).iter().enumerate() {
                    let pts: Vec<Vec2> = s.iter().map(|p| l.mapview.project(*p)).collect();
                    if to_polyline(mouse, &pts) < LEG_HIT && k < d.legs.len() {
                        leg = Some(k);
                    }
                }
            }
        }
        (stop, via, leg)
    };
    if l.pages.lines.hover_stop != hover_stop {
        l.pages.lines.hover_stop = hover_stop;
    }
    let has_line = l.pages.lines.line().is_some();
    if has_line && (hover_stop.is_some() || hover_via.is_some() || hover_leg.is_some()) && l.pages.lines.drag.is_none() {
        l.ui.cursor = winit::window::CursorIcon::Pointer;
    }
    // a drag of a leg or of one of its points starts, goes on, ends
    if l.ui.input.pressed && over && has_line {
        l.pages.lines.drag = match (hover_via, hover_leg) {
            (Some((k, j)), _) => Some(Drag::Via(k, j)),
            (None, Some(k)) => Some(Drag::New(k)),
            _ => None,
        };
    }
    let dragging = l.pages.lines.drag;
    if dragging.is_some() {
        l.ui.cursor = winit::window::CursorIcon::Grabbing;
        let at = l.mapview.world_of(mouse);
        if l.ui.input.released {
            l.pages.lines.drag = None;
            let v = &mut l.pages.lines;
            let d_idx = v.dir;
            if let Some(d) = v.line_mut().and_then(|x| x.directions.get_mut(d_idx)) {
                match dragging {
                    Some(Drag::Via(k, j)) => {
                        if let Some(p) = d.legs.get_mut(k).and_then(|g| g.vias.get_mut(j)) {
                            *p = [at.x, at.y];
                        }
                    }
                    Some(Drag::New(k)) => {
                        if k < d.legs.len() {
                            // (in its place along the leg: after the points nearer its start)
                            let a = dv(d.stops[k].at);
                            let along = |p: DVec2| (p - a).length();
                            let g = &mut d.legs[k];
                            let pos = g.vias.iter().position(|p| along(dv(*p)) > along(at)).unwrap_or(g.vias.len());
                            g.vias.insert(pos, [at.x, at.y]);
                        }
                    }
                    None => {}
                }
            }
            v.reroute(d_idx);
        } else {
            // (where the point goes, while it is held)
            let p = l.mapview.project(at);
            l.ui.p().circle(p, 6.0, TEXT);
            l.ui.p().circle(p, 3.5, accent());
        }
    }
    let p = Pointer { at: mouse, pressed: l.ui.input.pressed, released: l.ui.input.released, down: l.ui.input.down, wheel: l.ui.input.wheel.y, blocked: l.ui.over_ui || !map_r.contains(mouse) || dragging.is_some() };
    let scale = l.ui.scale;
    l.mapview.think(map_r, map_r, scale, p);
    // a click on a stop
    if let Some(at) = l.mapview.take_click_at() {
        if !has_line {
            return;
        }
        let v = &mut l.pages.lines;
        let stop = v.stops.iter().enumerate().map(|(i, s)| (i, (l.mapview.project(dv(s.at)) - at).length())).filter(|x| x.1 < STOP_HIT).min_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0);
        let Some(i) = stop else { return };
        let s = v.stops[i].clone();
        let d_idx = v.dir;
        let sel = v.sel_stop;
        let Some(line) = v.line_mut() else { return };
        if d_idx >= line.directions.len() {
            return;
        }
        let d = &mut line.directions[d_idx];
        if let Some(k) = d.stops.iter().position(|x| x.id == s.id && x.tile == s.tile) {
            // (a stop of the line: chosen in the list)
            v.sel_stop = Some(k);
            v.revision += 1;
            return;
        }
        let at_k = match sel {
            Some(k) if k + 1 < d.stops.len() => k + 1,
            _ => d.stops.len(),
        };
        d.stops.insert(at_k, s);
        // (the leg it lands in is two legs now, its dragged points gone)
        d.fit_legs();
        if at_k > 0 && at_k < d.stops.len() - 1 {
            d.legs.insert(at_k - 1, Default::default());
            d.legs.truncate(d.stops.len() - 1);
            if let Some(g) = d.legs.get_mut(at_k) {
                g.vias.clear();
            }
        }
        v.sel_stop = if sel.is_some() { Some(at_k) } else { None };
        v.reroute(d_idx);
    }
}

/// A narrow window (a phone): the lines of the map, to read; the editor wants a wide one.
fn narrow(l: &mut Launcher, area: Rect) {
    let v = &l.pages.lines;
    let mut y = area.y;
    let h = l.ui.paragraph("The line editor needs a wider window: open it on a computer. Your lines of this map:", Vec2::new(area.x, y), area.w, 13.0, Weight::Regular, TEXT_DIM);
    y += h + 12.0;
    let lines: Vec<(String, String, usize)> = v.reg.lines.iter().map(|x| (x.number.clone(), x.name.clone(), x.directions.iter().map(|d| d.stops.len()).max().unwrap_or(0))).collect();
    for (number, name, stops) in lines {
        let plate = Rect::new(area.x, y, (l.ui.width(&number, 13.0, Weight::Black) + 16.0).max(40.0), 24.0);
        l.ui.p().rounded(plate, 5.0, LINE);
        l.ui.text_in(&number, plate, 13.0, Weight::Black, ON_LINE, Align::Center);
        l.ui.text_in(&format!("{name} · {}", omsi_ui::tr("%{n} stops").replace("%{n}", &stops.to_string())), Rect::new(plate.right() + 10.0, y, area.w - plate.w - 10.0, 24.0), 13.0, Weight::Medium, TEXT, Align::Left);
        y += 32.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_and_distances() {
        let c = colour_of("#2a75f7");
        assert_eq!(c, Color::rgba(42, 117, 247, 1.0));
        let pts = [Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)];
        assert!((to_polyline(Vec2::new(5.0, 3.0), &pts) - 3.0).abs() < 1e-4);
    }

    fn vehicle(maker: &str, type_name: &str, file: &str) -> core::VehicleInfo {
        core::VehicleInfo { name: format!("{maker} {type_name}"), manufacturer: maker.into(), type_name: type_name.into(), file: file.into(), folder: file.split('/').nth(1).unwrap_or_default().into(), description: String::new(), default_paint: String::new(), paints: Vec::new(), hofs: Vec::new(), installed: false, missing_packs: Vec::new(), numbers: Vec::new() }
    }

    #[test]
    fn a_lines_buses_are_chosen_by_maker_model_and_version() {
        let vs = vec![vehicle("Setra", "S 415 UL - 2 doors", "Vehicles/Setra/a.bus"), vehicle("Setra", "S 415 UL - 3 doors", "Vehicles/Setra/b.bus"), vehicle("MAN", "Lion's City", "Vehicles/MAN/c.bus")];
        let cat = catalogue(&vs);
        assert_eq!(cat.iter().map(|m| m.0.as_str()).collect::<Vec<_>>(), ["MAN", "Setra"]);
        // the maker: all its buses; a model; one version
        let maker = pick_of(&cat, 1, 0, 0).unwrap();
        assert_eq!((maker.label.as_str(), maker.files.len(), maker.model.as_str()), ("Setra", 2, ""));
        let model = pick_of(&cat, 1, 1, 0).unwrap();
        assert_eq!((model.label.as_str(), model.files.len()), ("Setra S 415 UL", 2));
        let one = pick_of(&cat, 1, 1, 2).unwrap();
        assert_eq!((one.file.as_str(), one.label.as_str()), ("Vehicles/Setra/b.bus", "Setra S 415 UL · 3 doors"));
        assert!(pick_of(&cat, 5, 0, 0).is_none());
        // a kind's typical timetable: the company's regular line with its rush hours
        assert!(typical_days(ServiceKind::Regular, true).iter().all(|d| !d.bands.is_empty()));
        assert!(typical_days(ServiceKind::School, false)[1..].iter().all(|d| !d.on));
    }

    #[test]
    fn the_tables_times_are_typed_and_shown() {
        assert_eq!(parse_hm("7:05"), Some(425));
        assert_eq!(parse_hm("07.05"), Some(425));
        assert_eq!(parse_hm("0705"), Some(425));
        assert_eq!(parse_hm("705"), Some(425));
        assert_eq!(parse_hm("23:59"), Some(1439));
        assert_eq!(parse_hm("24:00"), None);
        assert_eq!(parse_hm("7:65"), None);
        assert_eq!(parse_hm("x"), None);
        assert_eq!(parse_hm("07"), None);
        assert_eq!(hm(425.0), "07:05");
        // (a trip past midnight from 00:00 on)
        assert_eq!(hm(1445.0), "00:05");
    }

    #[test]
    fn the_kinds_and_my_duties_are_translated() {
        let mut keys: Vec<&str> = ServiceKind::ALL.iter().flat_map(|k| [k.label(), k.note()]).collect();
        keys.extend(VehicleClass::ALL.iter().map(|c| c.label()));
        keys.extend(["School contract", "Fares and booking fees", "Tourism grant per km", "My duties", "Up next", "THIS LINE"]);
        keys.extend(["Public title", "Fill from the patterns", "Trip %{n}", "My destinations", "As advertised", "%{n} trip(s) of the table reach a stop before they left the one before"]);
        // (the sizes a company's level keeps locked)
        keys.extend(["locked", "Your company's level does not open these buses yet"]);
        // (a change of a line in service)
        keys.extend(["Change line %{n} in service", "Route approved anew", "%{k} kept, %{c} changed, %{a} new, %{g} dropped", "%{n} tours to plan anew", "Save the change", "Keep the line as it runs", "The new timetable begins on %{date}.", "A change of a line in service begins tomorrow at the soonest.", "New timetable from %{date}", "Line %{n} runs its new timetable from today: %{d} duties and %{b} buses given anew."]);
        for p in service::default_school_holidays() {
            keys.push(Box::leak(p.name.into_boxed_str()));
        }
        for lang in ["nl", "de", "fr", "ru", "uk", "pl"] {
            for k in &keys {
                assert!(crate::_rust_i18n_try_translate(lang, k).is_some(), "{lang}: {k}");
            }
        }
        assert_eq!(crate::_rust_i18n_try_translate("nl", "School transport").as_deref(), Some("Leerlingenvervoer"));
        // (and each kind's mark is one of the interface's icons)
        assert!(ServiceKind::ALL.iter().all(|k| omsi_ui::icons::svg(k.icon()).is_some()));
    }
}