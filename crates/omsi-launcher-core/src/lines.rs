//! The player's own lines: composed in the launcher's line editor, kept in a registry per map
//! (`~/.openomsi/lines/<map folder>.json`) and written out as ordinary OMSI timetable files,
//! so that the player and the timetable's AI buses drive them like any line of the map.
//!
//! The registry is the source of truth - stable ids, the stops, the lanes between them, the
//! times and the day patterns - and the files are made from it again on every save (and after
//! a reset of the map's timetable). It is also what a later bus company builds on: a line
//! there is a line here.
//!
//! What a line becomes in the map's `TTData` (see `export`):
//! * `<stem>.ttl` - `[userallowed]`, and one tour per bus of each day pattern;
//! * `<stem>_a.ttp` / `<stem>_b.ttp` - one trip per direction: its stops, the terminus, the
//!   line number, a profile with the time to every stop;
//! * `<stem>_a.ttr` / `<stem>_b.ttr` - the lanes the trip drives (the trip names it);
//! * `StnLinks.cfg` - a link for every pair of stops the map has none for (the map's own are
//!   never changed: other lines drive them), `Busstops.cfg` - the stops the map does not name.
//!
//! * `oo_vehicles.json` - the buses each line asks for (`service::LineVehicles`): the game
//!   takes its timetable buses for the line's trips from them (`read_line_buses`).
//!
//! A line's timetable is either made from its day patterns (first and last departure and how
//! often, or time bands) or - `LineDesign::table_on` - a table of trips with a time at every
//! stop, each the player's to change (`TableTrip`, after City Bus Manager's): a trip of other
//! times than its direction's gets a profile of its own in its trip file.
//!
//! Every file name starts with `oo_`, so none can take the place of one of the map's. What the
//! destination displays and the IBIS need of a line goes into the depot files of its buses
//! (`linehof`). A line's kind of service (`service::ServiceKind`) is written as its tours' day
//! masks: a school line's run on school days only, a weekend line's on Saturdays, Sundays and
//! public holidays.

use crate::service::{HolidayPeriod, LineVehicles, ServiceKind};
use omsi_timetable::{BusStopEntry, Line, StnLink, StnLinkEntry, Tour, TourTrip, Track, TrackEntry, Trip, TripProfile};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The registry's format; a newer one is read as far as it is understood.
pub const REGISTRY_VERSION: u32 = 1;
/// What every file a player's line writes begins with.
pub const FILE_PREFIX: &str = "oo_";
/// The list of the files the line editor wrote into a `TTData` folder (they are deleted
/// before it writes again, so a line renamed or deleted leaves nothing behind).
pub const MANIFEST: &str = "openomsi-lines.txt";
/// The buses each line asks for, as the game reads them (a timetable line's file stem, lower
/// case, to its `LineVehicles`; only lines that chose some).
pub const VEHICLES_FILE: &str = "oo_vehicles.json";

/// The days of a pattern (bits 0 - 6 Monday to Sunday, 7 public holidays): working days,
/// Saturday, Sunday and public holidays.
pub const DAY_GROUPS: [(&str, u16); 3] = [("Mon - Fri", 0b0001_1111), ("Saturday", 0b0010_0000), ("Sunday", 0b1100_0000)];
/// Both school bits: a tour runs in the school holidays (bit 8, as the game reads it) and on
/// school days (bit 9) alike - the game takes a tour only when its mask has the day's bit
/// and the school bit of the date.
pub const SCHOOL_BITS: i32 = 0b11_0000_0000;

/// The speed model for the running times: an average on the road between two stops and the
/// time a stop takes (braking, the doors, pulling out).
pub const CRUISE_MS: f32 = 8.3;
pub const STOP_S: f32 = 20.0;

/// The registry of one map.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Registry {
    pub version: u32,
    /// The map's folder (`maps/<folder>`), and its `global.cfg` as the launcher names it.
    pub map: String,
    pub global: String,
    pub next_id: u64,
    pub lines: Vec<LineDesign>,
    /// The depot files (by the name the depot groups give them) the line editor wrote its
    /// block into last time: a line deleted, or moved to another depot, takes its block out.
    pub depots: Vec<String>,
    /// The school holidays of a map whose calendar (`Holidays.txt`) has none (empty: the
    /// defaults, `service::default_school_holidays`).
    pub school_holidays: Vec<HolidayPeriod>,
    /// The player's own destinations on this map (`OwnDestination`), as the line editor kept
    /// them before the depot editor: they are destinations of the map's depot file of the
    /// player's own now (`owndepot::migrate` moves them there, and this stays empty).
    pub destinations: Vec<OwnDestination>,
}

impl Default for Registry {
    fn default() -> Self {
        Registry { version: REGISTRY_VERSION, map: String::new(), global: String::new(), next_id: 1, lines: Vec::new(), depots: Vec::new(), school_holidays: Vec::new(), destinations: Vec::new() }
    }
}

/// A player's line.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct LineDesign {
    /// Stable for the life of the line (the file names follow the number; this does not).
    pub id: u64,
    /// What the plate and the destination displays show.
    pub number: String,
    pub name: String,
    /// `#rrggbb`.
    pub colour: String,
    /// The `ailists.cfg` depot group whose buses drive it.
    pub ai_group: String,
    /// Outbound, then the way back (a line of one direction runs it round).
    pub directions: Vec<Direction>,
    /// One per `DAY_GROUPS` entry.
    pub days: Vec<DayPattern>,
    /// Seconds since 1970.
    pub created: u64,
    pub modified: u64,
    /// The bus company (its id) the line was made for; empty: a line of the player's own.
    /// The line editor shows a company's lines only when it works for that company.
    pub company: String,
    /// Kept out of the timetable: a company's line not confirmed and paid for yet
    /// (`company::ownline::confirm`).
    pub draft: bool,
    /// Dynamic passenger information (live departure displays) at the line's transfer stops
    /// (what a company pays for when it confirms the line).
    pub live_displays: bool,
    /// What kind of service it is: on which days it runs, and - in a company - what pays for
    /// it (`service`).
    pub service: ServiceKind,
    /// The buses that run it (none chosen: any of its depot group's).
    pub vehicles: LineVehicles,
    /// The timetable as a table of trips (`TableTrip`), and whether it is the line's (else the
    /// day patterns make its trips).
    pub table: Vec<TableTrip>,
    pub table_on: bool,
    /// What the line is called in public - on its card and in its advertising ("Shuttleverkehr
    /// Altenfeld - Wurzbach"); empty: its name.
    pub title: String,
    /// The player's own depot file (`owndepot`, by its key) the line's destinations, IBIS
    /// stops and routes go into, and which every bus that drives the line is given; empty:
    /// the map's (its depot group's) alone. The map's depot files get the line as well, for
    /// the timetable's buses.
    pub depot_file: String,
    /// A change of a company's line in service waiting for its day (Luc: "lijnen moeten ook
    /// aanpasbaar zijn als ze al actief zijn"): the line as the map's timetable keeps it until
    /// `pending_from`, the change's first day - today's tours run as they are. None: the line
    /// is in the timetable as it is here.
    pub live: Option<Box<LineDesign>>,
    pub pending_from: String,
}

impl Default for LineDesign {
    fn default() -> Self {
        LineDesign {
            id: 0,
            number: String::new(),
            name: String::new(),
            colour: "#2a75f7".into(),
            ai_group: String::new(),
            directions: vec![Direction::default()],
            days: default_days(),
            created: 0,
            modified: 0,
            company: String::new(),
            draft: false,
            live_displays: false,
            service: ServiceKind::Regular,
            vehicles: LineVehicles::default(),
            table: Vec::new(),
            table_on: false,
            title: String::new(),
            depot_file: String::new(),
            live: None,
            pending_from: String::new(),
        }
    }
}

impl Registry {
    /// The registry as the map's timetable has it: a line with a change waiting for its day
    /// as it runs until then (`LineDesign::live`).
    pub fn as_timetable(&self) -> Registry {
        let mut r = self.clone();
        for l in r.lines.iter_mut() {
            if let Some(live) = l.live.take() {
                *l = LineDesign { id: l.id, live: None, pending_from: String::new(), ..*live };
            }
        }
        r
    }
}

/// How a line's tours change from `old` to `new` (by their numbers): kept as they were,
/// changed (other trips or times), new, and gone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TourDiff {
    pub kept: Vec<String>,
    pub changed: Vec<String>,
    pub added: Vec<String>,
    pub gone: Vec<String>,
}

impl TourDiff {
    /// The tours whose buses and drivers are given anew: the changed and the gone.
    pub fn replan(&self) -> Vec<String> {
        self.changed.iter().chain(self.gone.iter()).cloned().collect()
    }
}

pub fn tour_diff(old: &LineDesign, new: &LineDesign) -> TourDiff {
    let (a, b) = (tour_plan(old), tour_plan(new));
    let (ra, rb) = (run_minutes(old), run_minutes(new));
    let same = |x: &PlannedTour, y: &PlannedTour| {
        x.day == y.day && x.trips.len() == y.trips.len() && x.trips.iter().zip(&y.trips).all(|(p, q)| p.dir == q.dir && (p.departure - q.departure).abs() < 0.5 && (p.minutes(&ra) - q.minutes(&rb)).abs() < 0.5)
    };
    let mut d = TourDiff::default();
    for t in &b {
        match a.iter().find(|x| x.number == t.number) {
            Some(x) if same(x, t) => d.kept.push(t.number.clone()),
            Some(_) => d.changed.push(t.number.clone()),
            None => d.added.push(t.number.clone()),
        }
    }
    d.gone = a.iter().filter(|x| !b.iter().any(|t| t.number == x.number)).map(|x| x.number.clone()).collect();
    d
}

/// The map's timetable changes from `old` to `new`: its tours, its stops or where they go, or
/// its number (the files' names).
pub fn timetable_differs(old: &LineDesign, new: &LineDesign) -> bool {
    let d = tour_diff(old, new);
    let stops = |l: &LineDesign| l.directions.iter().map(|d| (d.stops.iter().map(|s| (s.tile, s.id)).collect::<Vec<_>>(), d.destination())).collect::<Vec<_>>();
    !(d.changed.is_empty() && d.added.is_empty() && d.gone.is_empty()) || stops(old) != stops(new) || old.number.trim() != new.number.trim()
}

/// A destination of the player's own (Luc: "dat je eerst je eigen bestemmingen opslaat in de
/// editor"): what a direction's displays show - its name (the trip's terminus, "Shuttleverkehr
/// Altenfeld - Wurzbach"), the display texts (one per string of the depot file; empty: made
/// from the name, `linehof::Depot::sign_defaults`) and its terminus code in the depot file (0:
/// given when a line with it is saved). Kept per map in the registry and offered wherever a
/// destination is chosen.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct OwnDestination {
    pub name: String,
    pub sign: Vec<String>,
    pub code: i32,
}

/// A stop of a direction: the map object the timetable names (its tile, as object ids repeat
/// across the tiles of some maps), its name and where it stands (world metres).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct StopRef {
    pub tile: [i32; 2],
    pub id: i64,
    pub name: String,
    pub at: [f64; 2],
    /// What the IBIS shows for it (empty: the depot file's own name for the stop, else one
    /// made from its name; see `linehof`).
    pub ibis: String,
}

/// A lane a leg drives: the game's own `LaneKey` and its direction, and its length.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(default)]
pub struct LaneStep {
    pub tile: [i32; 2],
    pub id: i64,
    pub path: u16,
    pub reversed: bool,
    pub length: f32,
}

/// The way from one stop to the next.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Leg {
    /// Points the player dragged the leg through (world metres), in order.
    pub vias: Vec<[f64; 2]>,
    pub steps: Vec<LaneStep>,
    /// Metres from stop to stop.
    pub length: f32,
    /// A way over the roads was found (a leg without one keeps the line from being saved).
    pub ok: bool,
    /// Where the stops lie on the first and the last lane (along it, and beside it), as
    /// `StnLinks.cfg` keeps it.
    pub from_s: f32,
    pub to_s: f32,
    pub from_lat: f32,
    pub to_lat: f32,
}

/// One direction of a line.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Direction {
    /// The destination (a terminus of the map's depot file, or free text): the last stop's
    /// name when empty.
    pub terminus: String,
    pub stops: Vec<StopRef>,
    /// `stops.len() - 1` of them.
    pub legs: Vec<Leg>,
    /// Minutes from the first stop to every stop (the first 0).
    pub times: Vec<f32>,
    /// The times were set by hand (else they follow the legs: `auto_times`).
    pub manual_times: bool,
    /// The destination displays when the destination is no terminus of the depot file (it
    /// gets an `[addterminus]` of its own): one text per string of the depot file, an empty
    /// one taking the default made from the destination (`linehof::Depot::sign_defaults`).
    pub sign: Vec<String>,
    /// That new terminus's code in the depot file (given when the line is saved).
    pub terminus_code: i32,
    /// The IBIS route code, the line's number × 100 and two digits (0: given when the line is
    /// saved; one the depot file has already is given anew).
    pub ibis_route: u32,
}

/// When a line runs on the days of one `DAY_GROUPS` entry: buses from the first to the last
/// departure every `headway` minutes, each standing at least `layover` at the end - or, with
/// `bands`, a headway of its own for each part of the day (the rush hours denser than the
/// quiet ones), each band with the size of bus it wants.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct DayPattern {
    pub on: bool,
    pub days: u16,
    /// Minutes after midnight.
    pub first: f32,
    pub last: f32,
    pub headway: f32,
    pub layover: f32,
    /// The time bands (none: `first` to `last` every `headway`).
    pub bands: Vec<TimeBand>,
}

impl Default for DayPattern {
    fn default() -> Self {
        DayPattern { on: true, days: DAY_GROUPS[0].1, first: 6.0 * 60.0, last: 22.0 * 60.0, headway: 20.0, layover: 5.0, bands: Vec::new() }
    }
}

/// A part of the day with its own headway: departures from `from` every `headway` minutes,
/// the last before `to` (minutes after midnight; a band ends where the next begins).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct TimeBand {
    pub from: f32,
    pub to: f32,
    pub headway: f32,
    /// The bus size the band wants (None: the company's demand model chooses, see
    /// `company::ownline`).
    pub size: Option<crate::company::BusSize>,
}

impl Default for TimeBand {
    fn default() -> Self {
        TimeBand { from: 6.0 * 60.0, to: 9.0 * 60.0, headway: 15.0, size: None }
    }
}

/// The latest a band may end (the timetable's day ends at midnight).
pub const DAY_END: f32 = 24.0 * 60.0;

/// The bands a day group starts from when the player asks for them: the working days with
/// their two rush hours, Saturday's busy middle of the day, Sunday's quiet one.
pub fn default_bands(group: usize) -> Vec<TimeBand> {
    let b = |from: f32, to: f32, headway: f32| TimeBand { from: from * 60.0, to: to * 60.0, headway, size: None };
    match group {
        0 => vec![b(5.0, 6.0, 30.0), b(6.0, 9.0, 10.0), b(9.0, 15.0, 15.0), b(15.0, 18.0, 10.0), b(18.0, 21.0, 20.0), b(21.0, 24.0, 30.0)],
        1 => vec![b(6.0, 9.0, 30.0), b(9.0, 18.0, 20.0), b(18.0, 24.0, 30.0)],
        _ => vec![b(8.0, 20.0, 30.0), b(20.0, 24.0, 60.0)],
    }
}

impl DayPattern {
    /// The bands in order of the day, each ending where the next begins and before midnight
    /// (what the player typed, made to fit).
    pub fn clean_bands(&self) -> Vec<TimeBand> {
        let mut v: Vec<TimeBand> = self.bands.iter().copied().filter(|b| b.headway >= 1.0).collect();
        v.sort_by(|a, b| a.from.total_cmp(&b.from));
        for i in 0..v.len() {
            let next = v.get(i + 1).map(|n| n.from).unwrap_or(DAY_END);
            v[i].to = v[i].to.min(next).min(DAY_END);
        }
        v.retain(|b| b.to > b.from);
        v
    }

    /// Every departure of a direction (minutes after midnight), with the band it is in.
    pub fn departures(&self) -> Vec<(f32, Option<usize>)> {
        let mut out = Vec::new();
        if !self.on {
            return out;
        }
        if self.bands.is_empty() {
            if self.headway < 1.0 || self.last < self.first {
                return out;
            }
            let mut t = self.first;
            while t <= self.last + 1e-3 && out.len() < 2000 {
                out.push((t, None));
                t += self.headway;
            }
            return out;
        }
        for (i, b) in self.clean_bands().iter().enumerate() {
            let mut t = b.from;
            while t < b.to - 1e-3 && out.len() < 2000 {
                out.push((t, Some(i)));
                t += b.headway;
            }
        }
        out
    }

    /// It runs (it has a departure).
    pub fn runs(&self) -> bool {
        !self.departures().is_empty()
    }
}

/// Working days every 20 minutes, Saturdays every 30, Sundays every 60.
pub fn default_days() -> Vec<DayPattern> {
    vec![
        DayPattern { days: DAY_GROUPS[0].1, ..Default::default() },
        DayPattern { days: DAY_GROUPS[1].1, first: 7.0 * 60.0, headway: 30.0, ..Default::default() },
        DayPattern { days: DAY_GROUPS[2].1, first: 8.0 * 60.0, last: 21.0 * 60.0, headway: 60.0, ..Default::default() },
    ]
}

/// The tour mask a regular line's pattern's days are written as (`ServiceKind::mask` for the
/// other kinds).
pub fn mask_of(days: u16) -> i32 {
    days as i32 & 0xff | SCHOOL_BITS
}

/// The timetable a line of `kind` starts from when the player chooses the kind: a school
/// line's trips before school and after it, a weekend line's every hour in the daytime, an
/// on-demand line's every hour from morning to night; a regular line's as a new line has it.
pub fn days_for(kind: ServiceKind) -> Vec<DayPattern> {
    let b = |from: f32, to: f32, headway: f32| TimeBand { from: from * 60.0, to: to * 60.0, headway, size: None };
    let mut days = default_days();
    match kind {
        ServiceKind::Regular => {}
        ServiceKind::School => {
            days[0] = DayPattern { days: DAY_GROUPS[0].1, layover: 5.0, bands: vec![b(6.5, 8.25, 15.0), b(12.75, 16.5, 30.0)], ..Default::default() };
            days[1].on = false;
            days[2].on = false;
        }
        ServiceKind::Leisure => {
            days[0].on = false;
            days[1] = DayPattern { days: DAY_GROUPS[1].1, first: 9.0 * 60.0, last: 19.0 * 60.0, headway: 60.0, ..Default::default() };
            days[2] = DayPattern { days: DAY_GROUPS[2].1, first: 9.0 * 60.0, last: 19.0 * 60.0, headway: 60.0, ..Default::default() };
        }
        ServiceKind::OnDemand => {
            days[0] = DayPattern { days: DAY_GROUPS[0].1, first: 6.0 * 60.0, last: 22.0 * 60.0, headway: 60.0, ..Default::default() };
            days[1] = DayPattern { days: DAY_GROUPS[1].1, first: 7.0 * 60.0, last: 22.0 * 60.0, headway: 60.0, ..Default::default() };
            days[2] = DayPattern { days: DAY_GROUPS[2].1, first: 8.0 * 60.0, last: 21.0 * 60.0, headway: 60.0, ..Default::default() };
        }
    }
    days
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// --- the registry on disk --------------------------------------------------------------------

/// The map folder of a map file as the launcher names it (`maps/Grundorf/global.cfg` →
/// `Grundorf`).
pub fn map_folder(map_file: &str) -> String {
    let p = Path::new(&map_file.replace('\\', "/")).to_path_buf();
    p.parent().and_then(|d| d.file_name()).map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Where the registry of a map lies (in `~/.openomsi/lines`).
pub fn registry_path(map_folder: &str) -> PathBuf {
    crate::data_dir().join("lines").join(format!("{}.json", safe_name(map_folder)))
}

/// The registry in `path` (an empty one when there is none, or it cannot be read).
pub fn load_registry(path: &Path) -> Registry {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice::<Registry>(&b).unwrap_or_else(|_| {
            // (kept aside, so that the next save does not write an empty registry over it)
            let _ = std::fs::copy(path, path.with_extension(format!("json.broken-{}", now_secs())));
            Registry::default()
        }),
        Err(_) => Registry::default(),
    }
}

/// The registry written to `path` (through a file beside it: a crash halfway leaves the old).
pub fn save_registry(path: &Path, reg: &Registry) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_vec_pretty(reg).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

impl Registry {
    /// A new, empty line with the next id (and the number after the highest).
    pub fn add_line(&mut self, ai_group: &str) -> &mut LineDesign {
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        let number = (self.lines.iter().filter_map(|l| l.number.trim().parse::<u32>().ok()).max().unwrap_or(0) + 1).to_string();
        let t = now_secs();
        self.lines.push(LineDesign { id, number, ai_group: ai_group.to_string(), created: t, modified: t, ..Default::default() });
        self.lines.last_mut().unwrap()
    }

    pub fn line(&self, id: u64) -> Option<&LineDesign> {
        self.lines.iter().find(|l| l.id == id)
    }

    pub fn line_mut(&mut self, id: u64) -> Option<&mut LineDesign> {
        self.lines.iter_mut().find(|l| l.id == id)
    }

    /// The player's own destination of that name (in any case).
    pub fn destination(&self, name: &str) -> Option<&OwnDestination> {
        self.destinations.iter().find(|d| d.name.trim().eq_ignore_ascii_case(name.trim()))
    }

    /// Keep a destination of the player's own (one of the same name taken over); the empty
    /// name is none.
    pub fn keep_destination(&mut self, d: OwnDestination) {
        let name = d.name.trim().to_string();
        if name.is_empty() {
            return;
        }
        self.destinations.retain(|x| !x.name.trim().eq_ignore_ascii_case(&name));
        self.destinations.push(OwnDestination { name, ..d });
        self.destinations.sort_by_key(|x| x.name.to_lowercase());
    }
}

impl Direction {
    /// Give the direction one of the player's own destinations: its name, its display texts
    /// and - when it has one - its terminus code.
    pub fn take_destination(&mut self, d: &OwnDestination) {
        self.terminus = d.name.trim().to_string();
        self.sign = d.sign.clone();
        if d.code > 0 {
            self.terminus_code = d.code;
        }
    }
}

/// A name with the characters a file name cannot hold taken out.
pub fn safe_name(s: &str) -> String {
    let t: String = s.trim().chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '+' | '.') { c } else { '_' }).collect();
    let t = t.trim_matches('.').to_string();
    if t.is_empty() { "line".into() } else { t }
}

// --- times ------------------------------------------------------------------------------------

/// Seconds a leg of `length` metres takes (`CRUISE_MS`, plus `STOP_S` for the stop it ends at).
pub fn leg_seconds(length: f32) -> f32 {
    STOP_S + length.max(0.0) / CRUISE_MS
}

/// Minutes from the first stop to every stop, from the legs' lengths, in whole minutes (the
/// sum rounded, so the rounding does not pile up).
pub fn auto_times(legs: &[Leg]) -> Vec<f32> {
    let mut out = vec![0.0f32];
    let mut acc = 0.0f32;
    for l in legs {
        acc += leg_seconds(l.length);
        let m = (acc / 60.0).round().max(out.last().copied().unwrap_or(0.0));
        out.push(m);
    }
    // (a trip takes at least a minute)
    if out.len() > 1 && out.last().copied().unwrap_or(0.0) < 1.0 {
        *out.last_mut().unwrap() = 1.0;
    }
    out
}

impl Direction {
    /// Its times again from the legs, unless they were set by hand (and then only made to fit
    /// the stops there are).
    pub fn refresh_times(&mut self) {
        let n = self.stops.len();
        if !self.manual_times || self.times.len() != n {
            self.times = auto_times(&self.legs);
            self.times.resize(n, self.times.last().copied().unwrap_or(0.0));
            if n == 0 {
                self.times.clear();
            }
            self.manual_times = false;
        }
    }

    /// Minutes from the first stop to the last.
    pub fn minutes(&self) -> f32 {
        self.times.last().copied().unwrap_or(0.0)
    }

    /// The whole trip made to take `total` minutes: every stop's time scaled with it.
    pub fn set_total(&mut self, total: f32) {
        let old = self.minutes();
        if total < 1.0 || old <= 0.0 {
            return;
        }
        for t in &mut self.times {
            *t = (*t * total / old).round();
        }
        if let Some(l) = self.times.last_mut() {
            *l = total.round();
        }
        self.manual_times = true;
    }

    /// Stop `i` (and every stop after it) `by` minutes later; never before the stop ahead.
    pub fn shift_from(&mut self, i: usize, by: f32) {
        if i == 0 || i >= self.times.len() {
            return;
        }
        let floor = self.times[i - 1];
        let d = (self.times[i] + by).max(floor) - self.times[i];
        for t in &mut self.times[i..] {
            *t += d;
        }
        self.manual_times = true;
    }

    /// The legs fit the stops (one fewer), new ones empty.
    pub fn fit_legs(&mut self) {
        let n = self.stops.len().saturating_sub(1);
        self.legs.resize(n, Leg::default());
    }

    /// The destination it shows: the terminus chosen, else the last stop's name.
    pub fn destination(&self) -> String {
        if self.terminus.trim().is_empty() {
            self.stops.last().map(|s| s.name.trim().to_string()).unwrap_or_default()
        } else {
            self.terminus.trim().to_string()
        }
    }
}

/// The way back of `out`: its stops from the last to the first, each the stop of the same
/// name across the road (the nearest within `reach` metres, of `all` the map's stops), else
/// the same one.
pub fn opposite_stops(out: &[StopRef], all: &[StopRef], reach: f64) -> Vec<StopRef> {
    let dist = |a: &StopRef, b: &StopRef| ((a.at[0] - b.at[0]).powi(2) + (a.at[1] - b.at[1]).powi(2)).sqrt();
    out.iter()
        .rev()
        .map(|s| {
            let name = s.name.trim().to_lowercase();
            all.iter()
                .filter(|o| (o.id, o.tile) != (s.id, s.tile) && !name.is_empty() && o.name.trim().to_lowercase() == name && dist(o, s) <= reach)
                .min_by(|a, b| dist(a, s).total_cmp(&dist(b, s)))
                .cloned()
                .unwrap_or_else(|| s.clone())
        })
        .collect()
}

// --- tours from a pattern ------------------------------------------------------------------

/// A trip of a bus's day: the direction, its departure (minutes after midnight) and the time
/// band it is in (an index into the pattern's `clean_bands`); a table's trip its own minutes
/// and the profile of its trip file it runs with (0: the direction's times).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Planned {
    pub dir: usize,
    pub departure: f32,
    pub band: Option<usize>,
    /// Minutes the trip takes (0: its direction's, `run_minutes`).
    pub run: f32,
    pub profile: usize,
}

impl Planned {
    /// Minutes it takes: its own, else its direction's (`run`, from `run_minutes`).
    pub fn minutes(&self, run: &[f32]) -> f32 {
        if self.run > 0.0 { self.run } else { run.get(self.dir).copied().unwrap_or(1.0) }.max(1.0)
    }
}

/// A bus standing longer than this at the end (minutes) goes back to the depot between its
/// trips: the day of a bus that is wanted only in the rush hours is two tours, not one.
pub const PARK: f32 = 90.0;

/// The buses a pattern needs and the trips of each, in order: every direction leaves at the
/// pattern's departures, and each departure is given to a bus ready at that end (`run`
/// minutes of its trip and the `layover` after it), else to a new bus. A bus so drives back
/// and forth; with one direction it goes round. Without time bands the bus that has stood
/// longest takes the departure; with them the bus that came out first does, so that the buses
/// added for a rush hour are free again after it (`tours` sends them back to the depot).
pub fn blocks(run: &[f32], p: &DayPattern) -> Vec<Vec<Planned>> {
    // (a line without a direction yet has no buses)
    if run.is_empty() {
        return Vec::new();
    }
    let dirs = run.len().clamp(1, 2);
    let banded = !p.bands.is_empty();
    let mut deps: Vec<Planned> = Vec::new();
    for dir in 0..dirs {
        deps.extend(p.departures().into_iter().take(4000 / dirs).map(|(departure, band)| Planned { dir, departure, band, run: 0.0, profile: 0 }));
    }
    chain(deps, run, dirs, p.layover, banded)
}

/// Each trip (of `dirs` directions) given to a bus ready at the end it leaves from (its minutes
/// and `layover` after its trip before), else to a new bus: the one that has stood longest,
/// or - `first_out` - the one that came out first.
fn chain(mut deps: Vec<Planned>, run: &[f32], dirs: usize, layover: f32, first_out: bool) -> Vec<Vec<Planned>> {
    deps.sort_by(|a, b| a.departure.total_cmp(&b.departure).then(a.dir.cmp(&b.dir)));
    // (where a direction leaves from and where it ends: the two ends of the line, or the one
    // end of a line that goes round)
    let from = |d: usize| if dirs == 1 { 0 } else { d };
    let to = |d: usize| if dirs == 1 { 0 } else { 1 - d };
    struct Bus {
        at: usize,
        free: f32,
        trips: Vec<Planned>,
    }
    let mut buses: Vec<Bus> = Vec::new();
    for d in deps {
        let mut ready_buses = buses.iter().enumerate().filter(|(_, b)| b.at == from(d.dir) && b.free <= d.departure + 1e-3);
        let ready = if first_out { ready_buses.next().map(|(i, _)| i) } else { ready_buses.min_by(|a, b| a.1.free.total_cmp(&b.1.free)).map(|(i, _)| i) };
        let i = ready.unwrap_or_else(|| {
            buses.push(Bus { at: from(d.dir), free: 0.0, trips: Vec::new() });
            buses.len() - 1
        });
        let b = &mut buses[i];
        b.trips.push(d);
        b.at = to(d.dir);
        b.free = d.departure + d.minutes(run) + layover.max(0.0);
    }
    buses.into_iter().map(|b| b.trips).collect()
}

/// The tours of a pattern: each bus's day (`blocks`), cut where - with time bands - it
/// stands longer than `PARK` at the end (it goes back to the depot and out again later, as a
/// tour of its own: a bus for a rush hour only).
pub fn tours(run: &[f32], p: &DayPattern) -> Vec<Vec<Planned>> {
    let all = blocks(run, p);
    if p.bands.is_empty() {
        return all;
    }
    cut_parked(all, run)
}

/// Each bus's day cut where it stands longer than `PARK` at the end, in order of the tours'
/// first departures.
fn cut_parked(all: Vec<Vec<Planned>>, run: &[f32]) -> Vec<Vec<Planned>> {
    let mut out = Vec::new();
    for bus in all {
        let mut cur: Vec<Planned> = Vec::new();
        for t in bus {
            if let Some(last) = cur.last() {
                let back = last.departure + last.minutes(run);
                if t.departure - back > PARK {
                    out.push(std::mem::take(&mut cur));
                }
            }
            cur.push(t);
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out.sort_by(|a, b| a[0].departure.total_cmp(&b[0].departure));
    out
}

/// A tour of a line as it is written: its number, the day group (an index into the line's
/// `days`) and its trips.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedTour {
    pub number: String,
    pub day: usize,
    pub trips: Vec<Planned>,
}

impl PlannedTour {
    /// When its first trip leaves and its last arrives (minutes), with the run times of the
    /// directions.
    pub fn span(&self, run: &[f32]) -> (f32, f32) {
        let a = self.trips.first().map(|t| t.departure).unwrap_or(0.0);
        let z = self.trips.iter().map(|t| t.departure + t.minutes(run)).fold(a, f32::max);
        (a, z)
    }
}

/// Minutes each direction of a line takes from its first stop to its last (one entry per
/// direction with stops).
pub fn run_minutes(l: &LineDesign) -> Vec<f32> {
    l.directions.iter().filter(|d| d.stops.len() >= 2).map(|d| fitted_times(d).last().copied().unwrap_or(1.0).max(1.0)).collect()
}

/// Every tour of a line, numbered on through the day groups as the `.ttl` has them (the
/// groups its kind of service does not run on left out: a school line has no weekend tours) -
/// of its table when it has one on (`table_tours`).
pub fn tour_plan(l: &LineDesign) -> Vec<PlannedTour> {
    if l.table_on {
        return table_tours(l);
    }
    let run = run_minutes(l);
    let mut out: Vec<PlannedTour> = Vec::new();
    for (day, p) in l.days.iter().enumerate() {
        if !l.service.allows(day) {
            continue;
        }
        for trips in tours(&run, p) {
            out.push(PlannedTour { number: (out.len() + 1).to_string(), day, trips });
        }
    }
    out
}

// --- the timetable as a table -------------------------------------------------------------------

/// A trip of a line's timetable table (`LineDesign::table`, after City Bus Manager's: every time
/// the player's to change): its direction, its group of days (an index into `DAY_GROUPS`; the
/// line's kind of service decides which run, and a school line's on school days only, as its
/// masks say) and its time at every stop (minutes after midnight).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct TableTrip {
    pub dir: usize,
    pub day: usize,
    pub times: Vec<f32>,
}

impl TableTrip {
    pub fn departure(&self) -> f32 {
        self.times.first().copied().unwrap_or(0.0)
    }

    /// Minutes from its first stop to its last.
    pub fn minutes(&self) -> f32 {
        self.times.last().copied().unwrap_or(0.0) - self.departure()
    }

    /// The first stop it would reach before it left the stop before (None: its times run
    /// forward along the stops, as the game needs them to).
    pub fn first_fall(&self) -> Option<usize> {
        (1..self.times.len()).find(|&i| self.times[i] < self.times[i - 1] - 1e-3)
    }

    /// The whole trip `by` minutes later (earlier when negative; not before midnight).
    pub fn shift(&mut self, by: f32) {
        let by = by.max(-self.departure());
        for t in &mut self.times {
            *t += by;
        }
    }

    /// Its times from its departure on (the shape its trip file's profile keeps).
    pub fn relative(&self) -> Vec<f32> {
        let a = self.departure();
        self.times.iter().map(|t| t - a).collect()
    }
}

/// Minutes from the first stop to every stop of a direction: its own (set by hand, or made
/// from its legs when they were saved), else made from its legs now.
pub fn fitted_times(d: &Direction) -> Vec<f32> {
    let mut times = d.times.clone();
    if times.len() != d.stops.len() {
        times = auto_times(&d.legs);
        times.resize(d.stops.len(), times.last().copied().unwrap_or(0.0));
    }
    times
}

/// A table trip's times as its direction's stops are now (a stop added or taken out since the
/// table was made: from its departure on as the direction has the times).
pub fn trip_times(l: &LineDesign, t: &TableTrip) -> Vec<f32> {
    match l.directions.get(t.dir) {
        Some(d) if t.times.len() != d.stops.len() => fitted_times(d).iter().map(|x| t.departure() + x).collect(),
        _ => t.times.clone(),
    }
}

/// The table the day patterns make: every departure of every group of days the line's kind of
/// service runs on, in each direction, with the direction's times to the stops - where a table
/// starts, the player changing it trip by trip after.
pub fn table_from_patterns(l: &LineDesign) -> Vec<TableTrip> {
    let dirs: Vec<(usize, Vec<f32>)> = l.directions.iter().enumerate().filter(|(_, d)| d.stops.len() >= 2).map(|(k, d)| (k, fitted_times(d))).take(2).collect();
    let mut out = Vec::new();
    for (day, p) in l.days.iter().enumerate().filter(|(k, _)| l.service.allows(*k)) {
        for (dir, rel) in &dirs {
            for (dep, _) in p.departures() {
                out.push(TableTrip { dir: *dir, day, times: rel.iter().map(|x| dep + x).collect() });
            }
        }
    }
    out.sort_by(|a, b| a.day.cmp(&b.day).then(a.departure().total_cmp(&b.departure())).then(a.dir.cmp(&b.dir)));
    out
}

/// The trips of the table the line drives: of the groups of days its kind runs on and of a
/// direction it has, fitted to the stops (`trip_times`), in order of departure.
pub fn table_trips(l: &LineDesign) -> Vec<TableTrip> {
    let dirs = l.directions.iter().filter(|d| d.stops.len() >= 2).count().min(2);
    let mut out: Vec<TableTrip> = l
        .table
        .iter()
        .filter(|t| t.dir < dirs && t.day < DAY_GROUPS.len() && l.service.allows(t.day) && !t.times.is_empty())
        .map(|t| TableTrip { times: trip_times(l, t), ..t.clone() })
        .collect();
    out.sort_by(|a, b| a.day.cmp(&b.day).then(a.departure().total_cmp(&b.departure())).then(a.dir.cmp(&b.dir)));
    out
}

fn same_shape(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.05)
}

/// The profiles of a direction's trip file: its own times first, then every other shape the
/// table's trips of that direction have (a trip of the table runs with the profile of its
/// times: `Planned::profile`).
pub fn table_profiles(l: &LineDesign, dir: usize) -> Vec<Vec<f32>> {
    let Some(d) = l.directions.get(dir) else { return Vec::new() };
    let mut out = vec![fitted_times(d)];
    if l.table_on {
        for t in table_trips(l).iter().filter(|t| t.dir == dir) {
            let rel = t.relative();
            if !out.iter().any(|p| same_shape(p, &rel)) {
                out.push(rel);
            }
        }
    }
    out
}

/// The tours of a line's table: per group of days its trips given to buses as the day
/// patterns' are (`chain`, each bus standing at least the group's layover at the end), and a
/// bus that stands longer than `PARK` sent back to the depot in between.
pub fn table_tours(l: &LineDesign) -> Vec<PlannedTour> {
    let trips = table_trips(l);
    let dirs = l.directions.iter().filter(|d| d.stops.len() >= 2).count().clamp(1, 2);
    let profiles: Vec<Vec<Vec<f32>>> = (0..dirs).map(|d| table_profiles(l, d)).collect();
    let mut out: Vec<PlannedTour> = Vec::new();
    for day in 0..DAY_GROUPS.len() {
        let deps: Vec<Planned> = trips
            .iter()
            .filter(|t| t.day == day)
            .map(|t| {
                let rel = t.relative();
                let profile = profiles.get(t.dir).and_then(|p| p.iter().position(|x| same_shape(x, &rel))).unwrap_or(0);
                Planned { dir: t.dir, departure: t.departure(), band: None, run: t.minutes().max(1.0), profile }
            })
            .collect();
        if deps.is_empty() {
            continue;
        }
        let layover = l.days.get(day).map(|p| p.layover).unwrap_or(5.0);
        let mut buses = cut_parked(chain(deps, &[], dirs, layover, false), &[]);
        buses.sort_by(|a, b| a[0].departure.total_cmp(&b[0].departure));
        for trips in buses {
            out.push(PlannedTour { number: (out.len() + 1).to_string(), day, trips });
        }
    }
    out
}

/// Departures a group of days has in each hour, all directions together: the table's trips,
/// or the day pattern's departures in every direction.
pub fn hourly_departures(l: &LineDesign, day: usize) -> [u32; 24] {
    let mut out = [0u32; 24];
    if !l.service.allows(day) {
        return out;
    }
    let hour = |t: f32| (t / 60.0).floor().max(0.0) as usize % 24;
    if l.table_on {
        for t in table_trips(l).iter().filter(|t| t.day == day) {
            out[hour(t.departure())] += 1;
        }
    } else if let Some(p) = l.days.get(day) {
        let dirs = run_minutes(l).len().clamp(1, 2) as u32;
        for (t, _) in p.departures() {
            out[hour(t)] += dirs;
        }
    }
    out
}

/// The line runs on the group of days at all (its kind runs then, and it has a trip).
pub fn runs_on_group(l: &LineDesign, day: usize) -> bool {
    hourly_departures(l, day).iter().any(|n| *n > 0)
}

// --- the files --------------------------------------------------------------------------------

/// The file stem of each line: `oo_<number>`, with its id after it when another line of the
/// registry has the same number.
pub fn stems(reg: &Registry) -> HashMap<u64, String> {
    let mut count: HashMap<String, usize> = HashMap::new();
    for l in &reg.lines {
        *count.entry(safe_name(&l.number).to_lowercase()).or_default() += 1;
    }
    reg.lines
        .iter()
        .map(|l| {
            let n = safe_name(&l.number);
            let stem = if count.get(&n.to_lowercase()).copied().unwrap_or(0) > 1 { format!("{FILE_PREFIX}{n}_{}", l.id) } else { format!("{FILE_PREFIX}{n}") };
            (l.id, stem)
        })
        .collect()
}

/// The trip file of direction `dir` of a line with file stem `stem`.
pub fn trip_name(stem: &str, dir: usize) -> String {
    format!("{stem}_{}", if dir == 0 { "a" } else { "b" })
}

// --- the player's lines in the launcher's lists ----------------------------------------------

/// A player's line as the launcher's line lists show it, among the map's: the timetable line
/// it was written as, and what the line editor knows of it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OwnLine {
    /// 0: a line of the editor's whose registry is gone (only its files are left).
    pub id: u64,
    /// The `.ttl` file's stem: the timetable line's name (`LineInfo::name`).
    pub file: String,
    pub number: String,
    pub name: String,
    /// `#rrggbb`.
    pub colour: String,
    /// Where its directions go, outbound first, each once.
    pub destinations: Vec<String>,
    /// Its kind of service and the buses it asks for.
    pub service: ServiceKind,
    pub vehicles: LineVehicles,
    /// The player's own depot file every bus that drives it is given (`LineDesign::
    /// depot_file`; empty: none).
    pub depot_file: String,
}

impl OwnLine {
    /// "Ring · Markt – Bahnhof": its name and where it goes (what of it there is).
    pub fn caption(&self) -> String {
        let to = self.destinations.join(" – ");
        [self.name.trim(), to.trim()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
    }
}

/// The timetable line `name` is a file the line editor wrote (`oo_…`, in any case).
pub fn is_own_file(name: &str) -> bool {
    name.trim().get(..FILE_PREFIX.len()).is_some_and(|p| p.eq_ignore_ascii_case(FILE_PREFIX))
}

/// The lines of the registry that are in the map's timetable (the drafts `problems` keeps
/// out have no files to drive), by the file stem each was written as.
pub fn own_lines(reg: &Registry) -> Vec<OwnLine> {
    let stems = stems(reg);
    reg.lines
        .iter()
        .filter(|l| written(l))
        .map(|l| {
            let mut destinations: Vec<String> = Vec::new();
            for d in l.directions.iter().filter(|d| d.stops.len() >= 2) {
                let to = d.destination();
                if !to.is_empty() && !destinations.contains(&to) {
                    destinations.push(to);
                }
            }
            OwnLine { id: l.id, file: stems[&l.id].clone(), number: l.number.trim().to_string(), name: l.name.trim().to_string(), colour: l.colour.clone(), destinations, service: l.service, vehicles: l.vehicles.clone(), depot_file: l.depot_file.trim().to_string() }
        })
        .collect()
}

/// The player's line the timetable line `name` is: the registry's (`own`), else - a file of
/// the editor's the registry no longer has - one made from the file name (`oo_42` → 42).
/// None for a line of the map's.
pub fn own_line_of(name: &str, own: &[OwnLine]) -> Option<OwnLine> {
    let name = name.trim();
    if let Some(o) = own.iter().find(|o| o.file.eq_ignore_ascii_case(name)) {
        return Some(o.clone());
    }
    is_own_file(name).then(|| OwnLine { file: name.to_string(), number: name[FILE_PREFIX.len()..].to_string(), colour: LineDesign::default().colour, ..Default::default() })
}

/// The player's lines of the map whose `global.cfg` the launcher names `map_file`, from its
/// registry (none when it has none).
pub fn own_lines_of_map(map_file: &str) -> Vec<OwnLine> {
    let folder = map_folder(map_file);
    if folder.is_empty() {
        return Vec::new();
    }
    let path = registry_path(&folder);
    if !path.is_file() {
        return Vec::new();
    }
    own_lines(&load_registry(&path))
}

/// What saving the registry writes into a map's `TTData`: whole files by name, and the
/// links and stops to add to the map's `StnLinks.cfg` and `Busstops.cfg`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Export {
    pub files: Vec<(String, String)>,
    pub links: Vec<(StnLink, String, String)>,
    pub stops: Vec<BusStopEntry>,
    /// Tours per line id (how many buses its patterns need).
    pub tours: HashMap<u64, usize>,
}

/// Why a line cannot be saved yet: the text (an interface text, `%{n}` for the number) and
/// the number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Problem {
    pub text: &'static str,
    pub n: usize,
}

impl Problem {
    pub fn english(&self) -> String {
        self.text.replace("%{n}", &self.n.to_string())
    }
}

/// Why a line cannot be saved yet (none: it can).
pub fn problems(l: &LineDesign) -> Vec<Problem> {
    let mut out = Vec::new();
    let p = |text, n| Problem { text, n };
    if l.number.trim().is_empty() {
        out.push(p("The line needs a number", 0));
    }
    if l.ai_group.trim().is_empty() {
        out.push(p("Choose the depot whose buses drive the line", 0));
    }
    for (k, d) in l.directions.iter().enumerate() {
        if d.stops.len() < 2 {
            out.push(p(if k == 0 { "The outbound direction needs at least two stops" } else { "The way back needs at least two stops" }, 0));
            continue;
        }
        let bad = d.legs.iter().filter(|g| !g.ok).count() + (d.stops.len() - 1).saturating_sub(d.legs.len());
        if bad > 0 {
            out.push(p(if k == 0 { "%{n} leg(s) of the outbound direction have no way over the roads" } else { "%{n} leg(s) of the way back have no way over the roads" }, bad));
        }
    }
    let runs = if l.table_on { !table_trips(l).is_empty() } else { l.days.iter().enumerate().any(|(k, d)| l.service.allows(k) && d.runs()) };
    if !runs {
        out.push(p("The line runs on no day", 0));
    }
    // (the game needs a trip's times to run forward along its stops)
    let falling = if l.table_on { table_trips(l).iter().filter(|t| t.first_fall().is_some()).count() } else { 0 };
    if falling > 0 {
        out.push(p("%{n} trip(s) of the table reach a stop before they left the one before", falling));
    }
    out
}

/// The line goes into the map's timetable: it can be saved (`problems`) and is no company's
/// draft.
pub fn written(l: &LineDesign) -> bool {
    !l.draft && problems(l).is_empty()
}

/// The files of every line of the registry that can be saved (`problems` empty). `raw_tiles`
/// is `global.cfg`'s `[map]` list (a lane's tile is its place there); `map_links` the pairs
/// of stops the map has a link for, `map_stops` the stops its `Busstops.cfg` names.
pub fn export(reg: &Registry, raw_tiles: &[(i32, i32)], map_links: &HashSet<(i64, i64)>, map_stops: &HashSet<i64>) -> Result<Export, String> {
    let tile_index = |t: [i32; 2]| raw_tiles.iter().position(|x| *x == (t[0], t[1]));
    let stems = stems(reg);
    let mut out = Export::default();
    let mut links_done: HashSet<(i64, i64)> = map_links.clone();
    let mut stops_done: HashSet<i64> = map_stops.clone();
    let mut buses: std::collections::BTreeMap<String, LineVehicles> = std::collections::BTreeMap::new();
    for l in reg.lines.iter().filter(|l| written(l)) {
        let stem = &stems[&l.id];
        let number = l.number.trim().to_string();
        for (dir, d) in l.directions.iter().enumerate() {
            let name = trip_name(stem, dir);
            // (the direction's times, and every other shape of its table's trips: a profile each)
            let profiles: Vec<TripProfile> = table_profiles(l, dir)
                .iter()
                .enumerate()
                .map(|(k, times)| {
                    let total = times.last().copied().unwrap_or(1.0).max(1.0);
                    let man_dep_time = (1..d.stops.len().saturating_sub(1)).map(|i| (i as i32, times.get(i).copied().unwrap_or(0.0))).collect();
                    TripProfile { name: if k == 0 { "standard".into() } else { format!("table {k}") }, factor: total, man_dep_time, ..Default::default() }
                })
                .collect();
            let trip = Trip {
                name: name.clone(),
                display_name: name.clone(),
                terminus: d.destination(),
                line: number.clone(),
                stations: d.stops.iter().map(|s| s.id).collect(),
                profiles,
                ..Default::default()
            };
            out.files.push((format!("{name}.ttp"), trip.to_text()));
            let legs = &d.legs[..d.legs.len().min(d.stops.len().saturating_sub(1))];
            // the track: every leg's lanes, the one two legs share once
            let mut entries: Vec<TrackEntry> = Vec::new();
            let mut last: Option<([i32; 2], i64, u16, bool)> = None;
            for g in legs {
                for s in &g.steps {
                    let key = (s.tile, s.id, s.path, s.reversed);
                    if last == Some(key) {
                        continue;
                    }
                    last = Some(key);
                    let ti = tile_index(s.tile).ok_or_else(|| format!("line {number}: tile {},{} is not in the map's global.cfg", s.tile[0], s.tile[1]))?;
                    entries.push(TrackEntry { values: vec![s.id as f64, s.path as f64, ti as f64, 0.0, s.length as f64, 0.0] });
                }
            }
            out.files.push((format!("{name}.ttr"), Track { path: PathBuf::new(), entries }.to_text()));
            // the links the map lacks, and the stops it does not name
            for (k, g) in legs.iter().enumerate() {
                let (a, b) = (&d.stops[k], &d.stops[k + 1]);
                if !links_done.insert((a.id, b.id)) {
                    continue;
                }
                let mut entries = Vec::new();
                for s in &g.steps {
                    let ti = tile_index(s.tile).ok_or_else(|| format!("line {number}: tile {},{} is not in the map's global.cfg", s.tile[0], s.tile[1]))?;
                    entries.push(StnLinkEntry { values: [s.id as f64, s.path as f64, ti as f64, s.length as f64, -1.0, 0.0, 0.0] });
                }
                let last = entries.len().saturating_sub(1) as f64;
                let link = StnLink { length: g.length as f64, from_id: a.id, to_id: b.id, params: [g.from_lat as f64, g.to_lat as f64, g.from_s as f64, g.to_s as f64, 0.0, last], entries };
                out.links.push((link, a.name.clone(), b.name.clone()));
            }
            for s in &d.stops {
                if stops_done.insert(s.id) {
                    let group = tile_index(s.tile).unwrap_or(0) as i32;
                    out.stops.push(BusStopEntry { name: s.name.clone(), group, object_id: s.id, params: [0.0, 0.0, 0.0] });
                }
            }
        }
        // the tours: the buses of every day pattern, numbered on through the day groups
        let tours: Vec<Tour> = tour_plan(l)
            .into_iter()
            .map(|t| Tour {
                number: t.number,
                ai_group: l.ai_group.trim().to_string(),
                extra: l.service.mask(l.days[t.day].days).to_string(),
                trips: t.trips.iter().map(|x| TourTrip { trip: trip_name(stem, x.dir), profile: x.profile as i32, departure: x.departure }).collect(),
            })
            .collect();
        out.tours.insert(l.id, tours.len());
        let line = Line { path: PathBuf::new(), name: stem.clone(), user_allowed: true, priority: 1, tours };
        out.files.push((format!("{stem}.ttl"), line.to_text()));
        if !l.vehicles.open() {
            buses.insert(stem.to_lowercase(), l.vehicles.clone());
        }
    }
    if !buses.is_empty() {
        out.files.push((VEHICLES_FILE.to_string(), serde_json::to_string_pretty(&buses).map_err(|e| e.to_string())?));
    }
    Ok(out)
}

/// The buses the lines of a `TTData` folder ask for (`VEHICLES_FILE`; none when it has none):
/// by the timetable line's file stem, lower case.
pub fn read_line_buses(ttdata: &Path) -> HashMap<String, LineVehicles> {
    let Ok(b) = omsi_cfg::vfs::read(&ttdata.join(VEHICLES_FILE)) else { return HashMap::new() };
    serde_json::from_slice::<HashMap<String, LineVehicles>>(&b).map(|m| m.into_iter().map(|(k, v)| (k.to_lowercase(), v)).collect()).unwrap_or_default()
}

/// The text of a `TTData` file as the map has it, without what the line editor added.
fn map_part(dir: &Path, file: &str) -> String {
    let b = std::fs::read(dir.join(file)).unwrap_or_default();
    let text = omsi_cfg::codepage::detect(&b).encoding().decode(&b).0.into_owned();
    omsi_timetable::write::replace_block(&text, "")
}

/// The pairs of stops the map's own `StnLinks.cfg` in `dir` links, and the stops its
/// `Busstops.cfg` names (what the line editor added left out).
pub fn map_has(dir: &Path) -> (HashSet<(i64, i64)>, HashSet<i64>) {
    let links = omsi_timetable::parse_stnlinks(&omsi_cfg::CfgFile::from_str(dir.join("StnLinks.cfg"), &map_part(dir, "StnLinks.cfg")));
    let stops = omsi_timetable::parse_busstops(&omsi_cfg::CfgFile::from_str(dir.join("Busstops.cfg"), &map_part(dir, "Busstops.cfg")));
    (links.iter().map(|l| (l.from_id, l.to_id)).collect(), stops.iter().map(|s| s.object_id).collect())
}

/// Write `e` into the `TTData` folder `dir`: the files written last time go first (see
/// `MANIFEST`), then the new ones, and the blocks of `StnLinks.cfg` and `Busstops.cfg`.
/// `keep` is called for a file of the map's before it is changed (its `.orig`).
pub fn write_export(dir: &Path, e: &Export, keep: &dyn Fn(&Path) -> Result<(), String>) -> Result<usize, String> {
    let manifest = dir.join(MANIFEST);
    for name in std::fs::read_to_string(&manifest).unwrap_or_default().lines().map(str::trim).filter(|n| !n.is_empty()) {
        // (only what the editor writes: a manifest edited by hand deletes nothing else)
        if name.starts_with(FILE_PREFIX) && !name.contains(['/', '\\']) {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }
    let mut written = Vec::new();
    for (name, text) in &e.files {
        omsi_timetable::write::write_text(&dir.join(name), text).map_err(|x| format!("{name}: {x}"))?;
        written.push(name.clone());
    }
    let links: String = e.links.iter().map(|(l, a, b)| l.to_text(a, b)).collect();
    let stops: String = e.stops.iter().map(|s| s.to_text()).collect();
    for (file, block) in [("StnLinks.cfg", links), ("Busstops.cfg", stops)] {
        let p = dir.join(file);
        if p.is_file() || !block.is_empty() {
            keep(&p)?;
        }
        omsi_timetable::write::update_block(&p, &block).map_err(|x| format!("{file}: {x}"))?;
    }
    std::fs::write(&manifest, written.join("\r\n")).map_err(|x| format!("{MANIFEST}: {x}"))?;
    Ok(written.len())
}

/// Every line of the registry that can be saved, written into the map's `TTData` (the
/// content folder's copy, see `ttstore`). Returns how many lines and files.
pub fn export_to_map(content: &Path, map_dir: &Path, reg: &Registry) -> Result<(usize, usize), String> {
    let folder = map_dir.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = crate::ttstore::ttdata_dir(content, map_dir, &folder)?;
    let raw_tiles = omsi_map::GlobalCfg::load(&map_dir.join("global.cfg")).map(|g| g.raw_tiles).map_err(|e| format!("global.cfg: {e}"))?;
    let (links, stops) = map_has(&dir);
    // (a line with a change waiting for its day: as it runs until then)
    let e = export(&reg.as_timetable(), &raw_tiles, &links, &stops)?;
    let files = write_export(&dir, &e, &crate::ttstore::keep_original)?;
    Ok((e.tours.len(), files))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop(id: i64, name: &str, x: f64) -> StopRef {
        StopRef { tile: [0, 0], id, name: name.into(), at: [x, 0.0], ..Default::default() }
    }

    fn leg(len: f32, ids: &[i64]) -> Leg {
        Leg { steps: ids.iter().map(|i| LaneStep { tile: [0, 0], id: *i, path: 0, reversed: false, length: len / ids.len() as f32 }).collect(), length: len, ok: true, from_s: 2.0, to_s: 5.0, from_lat: 2.5, to_lat: 2.5, vias: Vec::new() }
    }

    /// A line of three stops out and three back over five lanes, every day.
    fn line(reg: &mut Registry) -> u64 {
        let l = reg.add_line("Busses");
        l.number = "42".into();
        l.name = "Ring".into();
        let out = Direction { terminus: "Markt".into(), stops: vec![stop(1, "Bahnhof", 0.0), stop(2, "Kirche", 500.0), stop(3, "Markt", 900.0)], legs: vec![leg(500.0, &[10, 11]), leg(400.0, &[11, 12])], ..Default::default() };
        let back = Direction { stops: vec![stop(4, "Markt", 900.0), stop(5, "Kirche", 500.0), stop(6, "Bahnhof", 0.0)], legs: vec![leg(400.0, &[13]), leg(500.0, &[14])], ..Default::default() };
        l.directions = vec![out, back];
        for d in &mut l.directions {
            d.refresh_times();
        }
        l.id
    }

    #[test]
    fn a_change_waits_for_its_day_and_says_which_tours_it_touches() {
        let mut reg = Registry::default();
        let id = line(&mut reg);
        let l = reg.line_mut(id).unwrap();
        l.days = vec![DayPattern { days: DAY_GROUPS[0].1, first: 360.0, last: 600.0, headway: 30.0, ..Default::default() }, DayPattern { on: false, days: DAY_GROUPS[1].1, ..Default::default() }, DayPattern { on: false, days: DAY_GROUPS[2].1, ..Default::default() }];
        let old = l.clone();
        // the colour only: the timetable stays
        let mut new = old.clone();
        new.colour = "#ff0000".into();
        assert!(!timetable_differs(&old, &new));
        assert!(tour_diff(&old, &new).replan().is_empty());
        // the evening later: the buses run longer - their tours change
        new.days[0].last = 720.0;
        let d = tour_diff(&old, &new);
        assert!(timetable_differs(&old, &new));
        assert!(!d.changed.is_empty() || !d.added.is_empty(), "{d:?}");
        assert_eq!(tour_diff(&old, &old.clone()).kept.len(), tour_plan(&old).len());
        // the rush hour's extra buses taken off: their tours go
        let mut busy = old.clone();
        busy.days[0].bands = vec![TimeBand { from: 360.0, to: 480.0, headway: 2.0, size: None }, TimeBand { from: 480.0, to: 600.0, headway: 30.0, size: None }];
        let fewer = old.clone();
        let d = tour_diff(&busy, &fewer);
        assert!(tour_plan(&busy).len() > tour_plan(&fewer).len());
        assert!(!d.gone.is_empty() && d.replan().len() >= d.gone.len(), "{d:?}");
        // another number: the files' names change
        let mut renamed = old.clone();
        renamed.number = "43".into();
        assert!(timetable_differs(&old, &renamed));
        // waiting: the timetable is written as it was
        let l = reg.line_mut(id).unwrap();
        *l = LineDesign { live: Some(Box::new(old.clone())), pending_from: "2024-03-06".into(), ..fewer.clone() };
        let t = reg.as_timetable();
        assert_eq!(t.lines[0].days, old.days);
        assert!(t.lines[0].live.is_none() && t.lines[0].id == id);
        let e = export(&t, &[(0, 0)], &HashSet::new(), &HashSet::new()).unwrap();
        let plain = {
            let mut r = Registry::default();
            r.lines.push(old.clone());
            export(&r, &[(0, 0)], &HashSet::new(), &HashSet::new()).unwrap()
        };
        assert_eq!(e.tours.len(), plain.tours.len());
    }

    #[test]
    fn the_table_starts_from_the_patterns_and_every_time_can_change() {
        let mut reg = Registry::default();
        let id = line(&mut reg);
        let l = reg.line_mut(id).unwrap();
        l.days = vec![DayPattern { days: DAY_GROUPS[0].1, first: 360.0, last: 480.0, headway: 30.0, ..Default::default() }, DayPattern { on: false, days: DAY_GROUPS[1].1, ..Default::default() }, DayPattern { on: false, days: DAY_GROUPS[2].1, ..Default::default() }];
        // the generator: every departure in both directions, the direction's times to the stops
        l.table = table_from_patterns(l);
        assert_eq!(l.table.len(), 2 * 5);
        assert_eq!(l.table[0].times, vec![360.0, 361.0, 362.0]);
        let patterns = tour_plan(l);
        l.table_on = true;
        // made into the same buses and tours as the patterns make
        let table = tour_plan(l);
        assert_eq!(table.len(), patterns.len());
        assert_eq!(table.iter().map(|t| t.trips.len()).sum::<usize>(), 10);
        assert!(table.iter().flat_map(|t| t.trips.iter()).all(|x| x.profile == 0));
        // a trip later as a whole keeps its direction's profile; one stop's time changed makes
        // a profile of its own
        l.table[2].shift(3.0);
        l.table[4].times[2] += 2.0;
        assert_eq!(table_profiles(l, 0).len(), 2);
        let plan = tour_plan(l);
        assert!(plan.iter().flat_map(|t| t.trips.iter()).any(|x| x.profile == 1 && (x.run - 4.0).abs() < 1e-3));
        let e = export(&reg, &[(0, 0)], &HashSet::new(), &HashSet::new()).unwrap();
        let ttp = &e.files.iter().find(|f| f.0 == "oo_42_a.ttp").unwrap().1;
        assert_eq!(ttp.matches("[profile]").count(), 2);
        let ttl = &e.files.iter().find(|f| f.0 == "oo_42.ttl").unwrap().1;
        assert!(ttl.contains("oo_42_a\r\n1\r\n"), "{ttl}");
        // the departures by the hour
        let l = reg.line_mut(id).unwrap();
        assert_eq!(hourly_departures(l, 0)[6], 4);
        assert!(runs_on_group(l, 0) && !runs_on_group(l, 1));
        // a stop reached before the one before: said, and not written
        l.table[1].times[2] = l.table[1].times[1] - 1.0;
        assert!(table_trips(l).iter().any(|t| t.first_fall() == Some(2)));
        assert!(problems(l).iter().any(|p| p.text.contains("reach a stop before") && p.n == 1));
        // a weekend trip of a school line is kept in the table, not driven
        l.table[1].times[2] = l.table[1].times[1] + 1.0;
        l.service = ServiceKind::School;
        l.table.push(TableTrip { dir: 0, day: 1, times: vec![600.0, 601.0, 602.0] });
        assert!(table_trips(l).iter().all(|t| t.day == 0));
        // a stop added since: the trip follows its direction's times from its departure
        l.directions[0].stops.push(stop(7, "Ende", 1200.0));
        l.directions[0].fit_legs();
        l.directions[0].legs[2] = leg(300.0, &[15]);
        l.directions[0].refresh_times();
        assert_eq!(trip_times(l, &l.table[0].clone()).len(), 4);
    }

    #[test]
    fn own_destinations_are_kept_and_given_to_a_direction() {
        let mut reg = Registry::default();
        reg.keep_destination(OwnDestination { name: " Shuttleverkehr Altenfeld - Wurzbach ".into(), sign: vec!["Shuttle".into(), "Altenfeld - Wurzbach".into()], code: 950 });
        reg.keep_destination(OwnDestination { name: "Betriebshof".into(), ..Default::default() });
        reg.keep_destination(OwnDestination { name: "".into(), ..Default::default() });
        assert_eq!(reg.destinations.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Betriebshof", "Shuttleverkehr Altenfeld - Wurzbach"]);
        // the same name again takes the place of the one before
        reg.keep_destination(OwnDestination { name: "betriebshof".into(), code: 960, ..Default::default() });
        assert_eq!(reg.destinations.len(), 2);
        let mut d = Direction::default();
        d.take_destination(reg.destination("shuttleverkehr altenfeld - wurzbach").unwrap());
        assert_eq!((d.terminus.as_str(), d.sign.len(), d.terminus_code), ("Shuttleverkehr Altenfeld - Wurzbach", 2, 950));
    }

    #[test]
    fn a_lines_kind_is_written_as_its_days_and_its_buses_for_the_game() {
        use crate::service::VehicleClass;
        let mut reg = Registry::default();
        let id = line(&mut reg);
        let l = reg.line_mut(id).unwrap();
        l.service = ServiceKind::School;
        l.days = days_for(ServiceKind::School);
        l.vehicles.classes.push(VehicleClass::Coach);
        // a school line: working days only, its tours on school days only
        let plan = tour_plan(reg.line(id).unwrap());
        assert!(!plan.is_empty() && plan.iter().all(|t| t.day == 0));
        let base = std::env::temp_dir().join(format!("omsi_lines_kind_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("TTData");
        std::fs::create_dir_all(&dir).unwrap();
        let masks = |reg: &Registry| -> HashSet<String> {
            let e = export(reg, &[(0, 0)], &HashSet::new(), &HashSet::new()).unwrap();
            write_export(&dir, &e, &|_| Ok(())).unwrap();
            let data = omsi_timetable::TimetableData::load(&base);
            data.lines.iter().find(|l| l.name == "oo_42").unwrap().tours.iter().map(|t| t.extra.clone()).collect()
        };
        assert_eq!(masks(&reg), ["543"].into_iter().map(String::from).collect());
        // the buses it asks for, for the game
        let buses = read_line_buses(&dir);
        assert_eq!(buses["oo_42"].classes, vec![VehicleClass::Coach]);
        // a weekend line, its working days on or not: Saturdays, Sundays and holidays
        let l = reg.line_mut(id).unwrap();
        l.service = ServiceKind::Leisure;
        l.days = default_days();
        assert_eq!(masks(&reg), ["800", "960"].into_iter().map(String::from).collect());
        // any bus again: the file is gone
        reg.line_mut(id).unwrap().vehicles = LineVehicles::default();
        masks(&reg);
        assert!(read_line_buses(&dir).is_empty() && !dir.join(VEHICLES_FILE).exists());
        let _ = std::fs::remove_dir_all(&base);
        // a school line whose working days are off runs on no day
        let l = reg.line_mut(id).unwrap();
        l.service = ServiceKind::School;
        l.days[0].on = false;
        assert!(problems(reg.line(id).unwrap()).iter().any(|p| p.text == "The line runs on no day"));
        // a registry of before: regular, any bus, the default holidays
        let old: Registry = serde_json::from_str(r#"{"version":1,"lines":[{"id":7,"number":"1"}]}"#).unwrap();
        assert!(old.lines[0].service == ServiceKind::Regular && old.lines[0].vehicles.open() && old.school_holidays.is_empty());
    }

    #[test]
    fn the_times_follow_the_legs_and_can_be_set_by_hand() {
        let legs = vec![leg(500.0, &[1]), leg(400.0, &[2]), leg(10.0, &[3])];
        // 80.2 s, 68.2 s, 21.2 s: 1, 2.5 (rounded 2), 2.8 (3)
        assert_eq!(auto_times(&legs), vec![0.0, 1.0, 2.0, 3.0]);
        let mut d = Direction { stops: vec![stop(1, "a", 0.0), stop(2, "b", 0.0), stop(3, "c", 0.0), stop(4, "d", 0.0)], legs, ..Default::default() };
        d.refresh_times();
        d.set_total(6.0);
        assert_eq!(d.times, vec![0.0, 2.0, 4.0, 6.0]);
        d.shift_from(2, -5.0);
        // (never before the stop ahead)
        assert_eq!(d.times, vec![0.0, 2.0, 2.0, 4.0]);
        assert!(d.manual_times);
        // set by hand, they stay when the legs change
        d.refresh_times();
        assert_eq!(d.times, vec![0.0, 2.0, 2.0, 4.0]);
    }

    #[test]
    fn the_buses_go_back_and_forth() {
        // 15 minutes each way, 5 at the end: a round takes 40, so every 20 minutes needs two
        // buses - one starting at each end, each taking the other's departures from there on
        let p = DayPattern { on: true, days: 31, first: 360.0, last: 480.0, headway: 20.0, layover: 5.0, ..Default::default() };
        let b = blocks(&[15.0, 15.0], &p);
        let trips: usize = b.iter().map(|x| x.len()).sum();
        assert_eq!(trips, 14);
        assert_eq!(b.len(), 2);
        // 10 minutes of layover: the 6:20 at each end finds no bus back yet, four buses
        assert_eq!(blocks(&[15.0, 15.0], &DayPattern { layover: 10.0, ..p.clone() }).len(), 4);
        for bus in &b {
            // a bus alternates the directions, and is never late for its next trip
            for w in bus.windows(2) {
                assert_ne!(w[0].dir, w[1].dir);
                assert!(w[1].departure >= w[0].departure + 20.0 - 1e-3);
            }
        }
        // one direction: the buses go round
        let round = blocks(&[25.0], &DayPattern { headway: 10.0, ..p.clone() });
        assert_eq!(round.len(), 3);
        assert!(round.iter().flatten().all(|t| t.dir == 0));
        // a pattern that is off has no buses
        assert!(blocks(&[15.0, 15.0], &DayPattern { on: false, ..p }).is_empty());
    }

    #[test]
    fn time_bands_add_buses_for_the_rush_hour_and_send_them_back() {
        // every 30 minutes, every 10 from 7 to 9, every 30 again until 12; 20 minutes a way
        let b = |from: f32, to: f32, headway: f32| TimeBand { from, to, headway, size: None };
        let p = DayPattern { bands: vec![b(420.0, 540.0, 10.0), b(360.0, 420.0, 30.0), b(540.0, 720.0, 30.0)], ..Default::default() };
        // (the bands in order of the day, each to where the next begins)
        let clean = p.clean_bands();
        assert_eq!(clean.iter().map(|x| (x.from, x.to)).collect::<Vec<_>>(), vec![(360.0, 420.0), (420.0, 540.0), (540.0, 720.0)]);
        let deps = p.departures();
        assert_eq!(deps.len(), 2 + 12 + 6);
        assert_eq!(deps[2], (420.0, Some(1)));
        let run = [20.0, 20.0];
        let buses = blocks(&run, &p);
        let t = tours(&run, &p);
        // the rush hour needs more buses than the quiet hours (a round of 50 minutes: two buses
        // at a 30-minute headway, six at 10)
        assert!(buses.len() >= 5, "{}", buses.len());
        // the buses added for it run in the rush hour only; the first ones go on all morning
        let peak_only = t.iter().filter(|x| x.iter().all(|y| y.band == Some(1))).count();
        assert!(peak_only >= 3, "{peak_only}");
        assert!(t.iter().any(|x| x.first().unwrap().departure < 420.0 && x.last().unwrap().departure >= 600.0));
        // no tour leaves a trip out, none takes a trip twice
        assert_eq!(t.iter().map(|x| x.len()).sum::<usize>(), 2 * deps.len());
        // a bus standing more than `PARK` goes back to the depot: two tours
        let gap = DayPattern { bands: vec![b(360.0, 420.0, 30.0), b(420.0, 900.0, 240.0)], ..Default::default() };
        assert!(tours(&run, &gap).len() > blocks(&run, &gap).len());
        // a line without stops yet has no tours
        assert!(tours(&[], &p).is_empty());
        // without bands nothing changes
        let plain = DayPattern::default();
        assert_eq!(tours(&run, &plain), blocks(&run, &plain));
        assert!(plain.departures().iter().all(|d| d.1.is_none()));
        // and a line's tours are numbered on through its day groups
        let mut reg = Registry::default();
        let id = line(&mut reg);
        let l = reg.line_mut(id).unwrap();
        l.days[0].bands = default_bands(0);
        let plan = tour_plan(l);
        assert!(plan.iter().enumerate().all(|(k, x)| x.number == (k + 1).to_string()));
        assert!(plan.iter().any(|x| x.day == 2));
    }

    #[test]
    fn a_companys_draft_is_kept_out_of_the_timetable() {
        let mut reg = Registry::default();
        let id = line(&mut reg);
        reg.line_mut(id).unwrap().draft = true;
        assert!(problems(reg.line(id).unwrap()).is_empty() && !written(reg.line(id).unwrap()));
        assert!(export(&reg, &[(0, 0)], &HashSet::new(), &HashSet::new()).unwrap().files.is_empty());
        assert!(own_lines(&reg).is_empty());
    }

    #[test]
    fn the_way_back_takes_the_stops_across_the_road() {
        let all = vec![stop(1, "Bahnhof", 0.0), stop(11, "Bahnhof", 12.0), stop(2, "Kirche", 500.0), stop(3, "Markt", 900.0), stop(31, "Markt", 2000.0)];
        let out = vec![all[0].clone(), all[2].clone(), all[3].clone()];
        let back: Vec<i64> = opposite_stops(&out, &all, 150.0).iter().map(|s| s.id).collect();
        // Markt across the road is too far: the same stop; Kirche has none; Bahnhof has one
        assert_eq!(back, vec![3, 2, 11]);
    }

    #[test]
    fn a_line_is_written_and_reads_back() {
        let mut reg = Registry::default();
        let id = line(&mut reg);
        // the map links 1 -> 2 already and names stop 1
        let map_links: HashSet<(i64, i64)> = [(1, 2)].into_iter().collect();
        let map_stops: HashSet<i64> = [1].into_iter().collect();
        let e = export(&reg, &[(5, 5), (0, 0)], &map_links, &map_stops).unwrap();
        let base = std::env::temp_dir().join(format!("omsi_lines_export_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("TTData");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Busstops.cfg"), "[busstop]\r\nBahnhof\r\n1\r\n1\r\n0.0\r\n0\r\n0\r\n").unwrap();
        assert_eq!(write_export(&dir, &e, &|_| Ok(())).unwrap(), 5);
        let data = omsi_timetable::TimetableData::load(&base);
        let l = data.lines.iter().find(|l| l.name == "oo_42").expect("the line");
        assert!(l.user_allowed);
        assert_eq!(l.tours.len(), e.tours[&id]);
        assert!(l.tours.iter().all(|t| t.ai_group == "Busses"));
        // the days: working days, Saturdays, Sundays and holidays - with both school bits
        let masks: HashSet<String> = l.tours.iter().map(|t| t.extra.clone()).collect();
        assert_eq!(masks, ["799", "800", "960"].into_iter().map(String::from).collect());
        let a = data.trip("oo_42_a").expect("the outbound trip");
        assert_eq!((a.display_name.as_str(), a.terminus.as_str(), a.line.as_str()), ("oo_42_a", "Markt", "42"));
        assert_eq!(a.stations, vec![1, 2, 3]);
        assert_eq!(a.profiles[0].man_dep_time, vec![(1, 1.0)]);
        // the way back shows its last stop
        assert_eq!(data.trip("oo_42_b").unwrap().terminus, "Bahnhof");
        // the track: lane 11 once where the two legs meet; the tile is its place in the list
        let track = data.tracks.iter().find(|t| t.path.file_stem().unwrap() == "oo_42_a").unwrap();
        assert_eq!(track.entries.iter().map(|x| x.values[0] as i64).collect::<Vec<_>>(), vec![10, 11, 12]);
        assert!(track.entries.iter().all(|x| x.values[2] == 1.0));
        // links: not 1 -> 2 (the map's), the three others; stops: all but 1
        let pairs: HashSet<(i64, i64)> = data.stn_links.iter().map(|l| (l.from_id, l.to_id)).collect();
        assert_eq!(pairs, [(2, 3), (4, 5), (5, 6)].into_iter().collect());
        assert_eq!(data.bus_stops.len(), 6);
        // written again with the line gone: its files and its block are gone, the map's stop stays
        reg.lines.clear();
        let e = export(&reg, &[(5, 5), (0, 0)], &map_links, &map_stops).unwrap();
        write_export(&dir, &e, &|_| Ok(())).unwrap();
        let data = omsi_timetable::TimetableData::load(&base);
        assert!(data.lines.is_empty() && data.trips.is_empty() && data.tracks.is_empty() && data.stn_links.is_empty());
        assert_eq!(data.bus_stops.iter().map(|s| s.object_id).collect::<Vec<_>>(), vec![1]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_lists_know_the_players_lines_by_their_files() {
        let mut reg = Registry::default();
        let id = line(&mut reg);
        // a draft (no stops yet) has no file, so it is no line to drive
        reg.add_line("Busses").number = "7".into();
        let own = own_lines(&reg);
        assert_eq!(own.len(), 1);
        let o = &own[0];
        assert_eq!((o.id, o.file.as_str(), o.number.as_str(), o.name.as_str()), (id, "oo_42", "42", "Ring"));
        // the outbound goes to its terminus, the way back to its last stop
        assert_eq!(o.destinations, vec!["Markt".to_string(), "Bahnhof".to_string()]);
        assert_eq!(o.caption(), "Ring · Markt – Bahnhof");
        // the timetable's line of that file, in any case; a line of the map's is none
        assert_eq!(own_line_of("OO_42", &own).map(|x| x.id), Some(id));
        assert_eq!(own_line_of("Montag - Freitag", &own), None);
        assert_eq!(own_line_of("Zoo_3", &own), None);
        // a file of the editor's without its registry: the number from the file name
        let orphan = own_line_of("oo_9", &[]).unwrap();
        assert_eq!((orphan.id, orphan.number.as_str(), orphan.caption().as_str()), (0, "9", ""));
        assert!(is_own_file(" oo_1") && !is_own_file("o") && !is_own_file("Hoo_1"));
        // a line of one direction that goes round names its end once
        let mut round = reg.clone();
        let l = round.line_mut(id).unwrap();
        l.directions.truncate(1);
        l.directions[0].terminus.clear();
        assert_eq!(own_lines(&round)[0].destinations, vec!["Markt".to_string()]);
    }

    #[test]
    fn a_line_with_a_gap_is_not_written() {
        let mut reg = Registry::default();
        let id = line(&mut reg);
        reg.line_mut(id).unwrap().directions[1].legs[0].ok = false;
        assert_eq!(problems(reg.line(id).unwrap()).len(), 1);
        let e = export(&reg, &[(0, 0)], &HashSet::new(), &HashSet::new()).unwrap();
        assert!(e.files.is_empty());
    }

    #[test]
    fn the_registry_keeps_its_ids() {
        let path = std::env::temp_dir().join(format!("omsi_lines_reg_{}", std::process::id())).join("Dorf.json");
        let mut reg = Registry { map: "Dorf".into(), ..Default::default() };
        let a = line(&mut reg);
        let b = reg.add_line("Busses").id;
        assert_ne!(a, b);
        assert_eq!(reg.line(b).unwrap().number, "43");
        save_registry(&path, &reg).unwrap();
        let back = load_registry(&path);
        assert_eq!(back, reg);
        // the same number twice: the files tell them apart by the id
        let mut two = back.clone();
        two.line_mut(b).unwrap().number = "42".into();
        let s = stems(&two);
        assert_eq!((s[&a].as_str(), s[&b].as_str()), ("oo_42_1", "oo_42_2"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        // (a file of another version still reads)
        let old: Registry = serde_json::from_str(r#"{"version":1,"map":"X","lines":[{"id":7,"number":"1"}]}"#).unwrap();
        assert_eq!(old.lines[0].days.len(), 3);
    }
}
