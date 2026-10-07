//! The player's own depot files (`.hof`), made in the launcher's depot editor: destinations
//! with their texts for every display, the IBIS's stops and routes, the special and service
//! trips - each file for a map, kept in `~/.openomsi/depots` and never in the OMSI 2 folder.
//!
//! A line of the line editor may choose one (`LineDesign::depot_file`): its destinations, stops
//! and routes are written into it as into the map's depot files (`linehof`: the block at its end
//! between the line editor's comment lines, `assign` and `write_lines`), and every bus that
//! drives the line is given it before the drive starts (`for_duty`) - a copy in openOMSI's
//! content folder, in the folder the bus's depot files are read from (`depot::target_for`),
//! kept the same as the player's (`give`, `refresh`) - and the game is told to take it: `--hof`
//! by its file name, which `situation::find_hof` looks for first. A depot file chosen on the
//! bus step is given the same way to any bus, for a special or a service trip.
//!
//! The player's own destinations of the line editor are destinations of the map's depot file of
//! the player's own (`migrate` moved the registry's list there; `own_destinations`,
//! `keep_destination`).
//!
//! The file is an ordinary depot file OMSI reads as well (`Hof::to_text`); what only the editor
//! knows - the map it is for, which destinations are special trips offered for any bus - is in
//! comment lines at its head (`META_MAP`, `META_SPECIAL`).

use crate::depot::Refusal;
use crate::linehof::{self, Coding, Column, Depot, Role, Taken};
use crate::lines::{self, LineDesign, OwnDestination, Registry};
use omsi_cfg::codepage::CodePage;
use omsi_vehicle::hof::{BusStop, Comments, Terminus};
use omsi_vehicle::Hof;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// What the file name of every depot file of the player's own begins with: none of a map's or a
/// bus's is called so.
pub const PREFIX: &str = "oo_";
/// The comment lines at a file's head with what only the editor knows.
const META_MAP: &str = "openOMSI depot file, map:";
const META_SPECIAL: &str = "openOMSI special trips:";
const META_NOTE: &str = "Made in openOMSI's depot editor; the line editor writes its lines at the end.";

/// The special and service trips the editor offers to add with a click: the destination's
/// name, and whether nobody boards (`[addterminus_allexit]`).
pub const SPECIALS: [(&str, bool); 6] = [("Betriebsfahrt", true), ("Leerfahrt", true), ("Dienstfahrt", true), ("Nicht einsteigen", true), ("Sonderfahrt", false), ("Schulbus", false)];

/// Where the player's own depot files are kept.
pub fn dir() -> PathBuf {
    crate::data_dir().join("depots")
}

// --- one depot file --------------------------------------------------------------------------

/// A depot file as the editor has it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Doc {
    pub hof: Hof,
    /// What each terminus string is for (`string0: IBIS-Display`; "" where it says nothing),
    /// and each stop string.
    pub notes: Vec<String>,
    pub stop_notes: Vec<String>,
    /// The folder of the map it is for ("": any map).
    pub map: String,
    /// The codes of the destinations offered as special trips for any bus.
    pub specials: Vec<i32>,
    /// The comment lines at the head of the file it was made from (who made the map's), kept.
    pub head: Vec<String>,
}

/// The notes OMSI's own depot files have on their strings, in English: the layout of a new
/// file (the line editor knows the strings by them, `linehof::Role::from_note`).
const STOCK_NOTES: [&str; 8] = ["IBIS display", "Front, line 1", "Front, line 2", "Side", "Roller blind texture", "IBIS 2 (name as written), max 20 characters", "Extra sign", "Picture (Krueger bitmap)"];
const STOCK_STOP_NOTES: [&str; 4] = ["IBIS display", "Line 1", "Line 2", "Name as written"];

/// Where a problem of a depot file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    File,
    Terminus(usize),
    Stop(usize),
    Trip(usize),
}

/// A problem of a depot file, said plainly.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub at: Place,
    /// The game trips over it (else it is only worth knowing).
    pub serious: bool,
    /// An interface text, `%{..}` taking `args`.
    pub text: &'static str,
    pub args: Vec<(&'static str, String)>,
}

impl Issue {
    pub fn english(&self) -> String {
        self.args.iter().fold(self.text.to_string(), |t, (k, v)| t.replace(&format!("%{{{k}}}"), v))
    }
}

/// The most characters a text of `role` (its file's note `note`) shows on a display: the
/// note's when it says, else what OMSI's displays take.
pub fn limit(role: Role, note: &str) -> Option<usize> {
    linehof::note_max(note).or(match role {
        Role::IbisDisplay => Some(16),
        Role::FrontTop | Role::FrontBottom | Role::Side | Role::ClearName => Some(20),
        _ => None,
    })
}

/// The characters of `text` the code page of `coding` cannot hold (each once): written, they
/// become `?`.
pub fn unwritable(text: &str, coding: Coding) -> String {
    let Coding::Page(page) = coding else { return String::new() };
    let mut out = String::new();
    let mut buf = [0u8; 4];
    for c in text.chars() {
        if !c.is_ascii() && !out.contains(c) && page.encoding().encode(c.encode_utf8(&mut buf)).2 {
            out.push(c);
        }
    }
    out
}

impl Doc {
    /// A new, empty depot file called `name` for the map `map`, laid out as OMSI's own (eight
    /// texts a destination, four a stop).
    pub fn blank(name: &str, map: &str) -> Doc {
        let hof = Hof { name: name.trim().to_string(), string_count_terminus: STOCK_NOTES.len(), string_count_busstop: STOCK_STOP_NOTES.len(), ..Default::default() };
        Doc { hof, notes: STOCK_NOTES.iter().map(|s| s.to_string()).collect(), stop_notes: STOCK_STOP_NOTES.iter().map(|s| s.to_string()).collect(), map: map.trim().to_string(), ..Default::default() }
    }

    /// A depot file's text as the editor has it: without the line editor's blocks (`blocks`).
    pub fn from_text(text: &str) -> Doc {
        let (own, _) = split_blocks(text);
        let hof = Hof::parse(&omsi_cfg::CfgFile::from_str("depot.hof", &own));
        let notes = linehof::notes(&own, hof.string_count_terminus);
        let mut stop_notes = vec![String::new(); hof.string_count_busstop];
        let mut map = String::new();
        let mut specials = Vec::new();
        let mut head = Vec::new();
        let mut before = true;
        for line in own.lines() {
            let t = line.trim();
            if t.starts_with('[') {
                before = false;
            }
            if let Some(m) = t.strip_prefix(META_MAP) {
                map = m.trim().to_string();
            } else if let Some(s) = t.strip_prefix(META_SPECIAL) {
                specials = s.split([' ', ',']).filter_map(|c| c.trim().parse::<i32>().ok()).collect();
            } else if let Some((k, note)) = t.strip_prefix("busstop string").and_then(|r| r.split_once(':')) {
                if let Some(slot) = k.trim().parse::<usize>().ok().and_then(|k| stop_notes.get_mut(k)) {
                    *slot = note.trim().to_string();
                }
            } else if before && !t.is_empty() && t != META_NOTE && head.len() < 40 {
                head.push(t.to_string());
            }
        }
        let mut d = Doc { hof, notes, stop_notes, map, specials, head };
        d.tidy();
        d
    }

    /// The file's text (`\r\n` line ends), without blocks of the line editor.
    pub fn to_text(&self) -> String {
        let mut head = vec![format!("{META_MAP} {}", self.map.trim())];
        let mut codes: Vec<i32> = self.specials.iter().copied().filter(|c| self.hof.termini.iter().any(|t| t.code == *c)).collect();
        codes.sort_unstable();
        codes.dedup();
        if !codes.is_empty() {
            head.push(format!("{META_SPECIAL} {}", codes.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" ")));
        }
        head.push(META_NOTE.to_string());
        head.extend(self.head.iter().cloned());
        self.hof.to_text(&Comments { head, terminus: self.notes.clone(), busstop: self.stop_notes.clone() })
    }

    /// Everything in step with the counts: as many texts a destination and a stop as the file
    /// says, a stop list for every route, a destination's stop as the game reads it.
    pub fn tidy(&mut self) {
        let (nt, nb) = (self.hof.string_count_terminus, self.hof.string_count_busstop);
        self.notes.resize(nt, String::new());
        self.stop_notes.resize(nb, String::new());
        for t in &mut self.hof.termini {
            t.strings.resize(nt, String::new());
            t.texture_id = t.texture_id.trim().to_string();
            t.terminus_stop = (!t.all_exit).then(|| t.texture_id.clone());
        }
        for b in &mut self.hof.bus_stops {
            b.strings.resize(nb, String::new());
            b.ident = b.ident.trim().to_string();
        }
        self.hof.info_busstop_lists.resize(self.hof.info_trips.len(), Vec::new());
    }

    /// The file as the line editor sees it (what each text is for, how its columns write).
    pub fn depot(&self) -> Depot {
        let mut d = Depot::from_hof(self.hof.clone(), self.notes.clone());
        // (a file without destinations yet: each text as its note says)
        if self.hof.termini.is_empty() {
            for (k, r) in d.roles.clone().iter().enumerate() {
                let note = self.notes.get(k).map(String::as_str).unwrap_or("");
                d.cols[k] = match r {
                    Role::RollerBlind | Role::Bitmap | Role::ExtraSign => Column::Empty,
                    Role::ClearName => Column::Text { upper: false, max: limit(*r, note).unwrap_or(20), centred: false },
                    _ => Column::Text { upper: true, max: limit(*r, note).unwrap_or(16), centred: false },
                };
            }
        }
        d
    }

    /// The labels of the terminus texts: what each is for (an interface text), or the file's
    /// own note.
    pub fn labels(&self) -> Vec<(Role, String)> {
        self.depot().labels()
    }

    /// The destination called `name` (in any case).
    pub fn destination(&self, name: &str) -> Option<usize> {
        let n = name.trim();
        (!n.is_empty()).then(|| self.hof.termini.iter().position(|t| t.texture_id.trim().eq_ignore_ascii_case(n))).flatten()
    }

    /// A destination code: `wanted` when it is free, else the first free one
    /// (`linehof::terminus_code`).
    pub fn free_code(&self, wanted: i32) -> i32 {
        let taken: HashSet<i32> = self.hof.termini.iter().map(|t| t.code).collect();
        linehof::terminus_code(wanted, &taken)
    }

    /// A route code: `wanted` when free, else the next free one after the highest.
    pub fn free_route(&self, wanted: u32) -> u32 {
        let taken: HashSet<u32> = self.hof.info_trips.iter().filter_map(|t| t.code.trim().parse().ok()).collect();
        if wanted > 0 && !taken.contains(&wanted) {
            return wanted;
        }
        (taken.iter().max().copied().unwrap_or(100) + 1..).find(|c| !taken.contains(c)).unwrap_or(1)
    }

    /// A destination is a special or service trip offered for any bus: marked so, or one where
    /// nobody boards.
    pub fn is_special(&self, i: usize) -> bool {
        self.hof.termini.get(i).is_some_and(|t| t.all_exit || self.specials.contains(&t.code))
    }

    pub fn set_special(&mut self, i: usize, on: bool) {
        let Some(code) = self.hof.termini.get(i).map(|t| t.code) else { return };
        self.specials.retain(|c| *c != code);
        if on {
            self.specials.push(code);
        }
    }

    /// A new destination `name` at the end: a free code, its texts made from the name as the
    /// file writes its own (`Depot::sign_defaults`). Returns its place.
    pub fn add_destination(&mut self, name: &str, all_exit: bool, special: bool) -> usize {
        let name = name.trim();
        let strings = self.depot().sign_defaults(name);
        let wanted = if special { 900 } else { 0 };
        let code = self.free_code(wanted);
        self.hof.termini.push(Terminus { code, texture_id: name.to_string(), terminus_stop: (!all_exit).then(|| name.to_string()), all_exit, strings });
        if special {
            self.specials.push(code);
        }
        self.tidy();
        self.hof.termini.len() - 1
    }

    /// A new IBIS stop `name`: its IBIS text made as the file writes its stops. Returns its place.
    pub fn add_stop(&mut self, name: &str) -> usize {
        let d = self.depot();
        let display = d.stop_display(name);
        let strings = d.stop_strings(name, &display);
        self.hof.bus_stops.push(BusStop { ident: name.trim().to_string(), strings });
        self.tidy();
        self.hof.bus_stops.len() - 1
    }

    /// How many texts a destination has: every destination's list made as long (new texts
    /// empty), the notes with it.
    pub fn set_string_count(&mut self, n: usize) {
        self.hof.string_count_terminus = n.clamp(1, 32);
        self.tidy();
    }

    /// The routes that go to destination `code`.
    pub fn routes_to(&self, code: i32) -> usize {
        self.hof.info_trips.iter().filter(|t| omsi_cfg::parse_i32(&t.route) == code && !t.route.trim().is_empty()).count()
    }

    /// What is wrong with the file, or worth knowing: the game's problems first. `maps_depot`
    /// is the name of the map's own depot file ("" when unknown), `coding` how it is written.
    pub fn issues(&self, maps_depot: &str, coding: Coding) -> Vec<Issue> {
        let mut out = Vec::new();
        let issue = |at, serious, text, args: Vec<(&'static str, String)>| Issue { at, serious, text, args };
        let h = &self.hof;
        if h.name.trim().is_empty() {
            out.push(issue(Place::File, true, "The depot file has no name", vec![]));
        } else if !maps_depot.trim().is_empty() && h.name.trim().eq_ignore_ascii_case(maps_depot.trim()) {
            out.push(issue(Place::File, false, "It has the name of the map's own depot file: a bus that has both may take the map's. Give it a name of its own", vec![]));
        }
        if h.termini.is_empty() {
            out.push(issue(Place::File, false, "No destinations yet", vec![]));
        }
        if !h.service_trip.trim().is_empty() && self.destination(&h.service_trip).is_none() {
            out.push(issue(Place::File, false, "The service trip names %{name}, which is no destination of the file", vec![("name", h.service_trip.trim().to_string())]));
        }
        let bad = unwritable(&self.to_text(), coding);
        if !bad.is_empty() {
            out.push(issue(Place::File, false, "These characters cannot be written in the file's code page and become ?: %{chars}", vec![("chars", bad)]));
        }
        // the destinations
        let labels = self.labels();
        let mut codes: HashMap<i32, usize> = HashMap::new();
        let mut names: HashMap<String, usize> = HashMap::new();
        for (i, t) in h.termini.iter().enumerate() {
            let name = t.texture_id.trim().to_string();
            let at = Place::Terminus(i);
            if t.code < 0 {
                out.push(issue(at, true, "%{name}: code %{code} is below 0", vec![("name", name.clone()), ("code", t.code.to_string())]));
            } else if t.code >= 1000 {
                out.push(issue(at, false, "%{name}: the IBIS types three digits; code %{code} can only be chosen by a timetable", vec![("name", name.clone()), ("code", t.code.to_string())]));
            }
            match codes.get(&t.code) {
                Some(first) => out.push(issue(at, true, "Code %{code} is given twice (%{first} and %{name}): the IBIS takes the first", vec![("code", t.code.to_string()), ("first", h.termini[*first].texture_id.trim().to_string()), ("name", name.clone())])),
                None => {
                    codes.insert(t.code, i);
                }
            }
            if name.is_empty() {
                out.push(issue(at, true, "Destination %{code} has no name: no trip can go there", vec![("code", t.code.to_string())]));
                continue;
            }
            if names.insert(name.to_lowercase(), i).is_some() {
                out.push(issue(at, false, "%{name} is there twice: a trip to it takes the first", vec![("name", name.clone())]));
            }
            let texts: Vec<usize> = labels.iter().enumerate().filter(|(_, (r, _))| matches!(r, Role::IbisDisplay | Role::FrontTop | Role::FrontBottom | Role::Side | Role::ClearName | Role::Other)).map(|(k, _)| k).collect();
            if !t.all_exit && !texts.is_empty() && texts.iter().all(|k| t.strings.get(*k).is_none_or(|s| s.trim().is_empty())) {
                out.push(issue(at, false, "%{name} shows nothing on the displays", vec![("name", name.clone())]));
            }
            for (k, (role, _)) in labels.iter().enumerate() {
                let note = self.notes.get(k).map(String::as_str).unwrap_or("");
                let Some(max) = limit(*role, note) else { continue };
                let n = t.strings.get(k).map(|s| s.trim_end().chars().count()).unwrap_or(0);
                if n > max {
                    out.push(issue(at, false, "%{name}: text %{k} has %{n} characters, its display shows %{max}", vec![("name", name.clone()), ("k", (k + 1).to_string()), ("n", n.to_string()), ("max", max.to_string())]));
                }
            }
        }
        // the IBIS's stops
        let mut stops: HashSet<String> = HashSet::new();
        for (i, b) in h.bus_stops.iter().enumerate() {
            let name = b.ident.trim();
            if name.is_empty() {
                out.push(issue(Place::Stop(i), true, "A stop without a name: no route can call at it", vec![]));
            } else if !stops.insert(name.to_lowercase()) {
                out.push(issue(Place::Stop(i), false, "The stop %{name} is there twice: the IBIS takes the first", vec![("name", name.to_string())]));
            }
            let n = b.strings.first().map(|s| s.trim_end().chars().count()).unwrap_or(0);
            if n > 16 {
                out.push(issue(Place::Stop(i), false, "%{name}: the IBIS text has %{n} characters, the IBIS shows 16", vec![("name", name.to_string()), ("n", n.to_string())]));
            }
        }
        // the routes
        let mut routes: HashSet<String> = HashSet::new();
        for (i, t) in h.info_trips.iter().enumerate() {
            let at = Place::Trip(i);
            let code = t.code.trim().to_string();
            if code.parse::<u32>().is_err() {
                out.push(issue(at, true, "Route %{code} is no number: the IBIS cannot type it", vec![("code", code.clone())]));
            } else if !routes.insert(code.clone()) {
                out.push(issue(at, true, "Route %{code} is there twice: the IBIS takes the first", vec![("code", code.clone())]));
            }
            let dest = omsi_cfg::parse_i32(&t.route);
            if t.route.trim().is_empty() || !h.termini.iter().any(|x| x.code == dest) {
                out.push(issue(at, true, "Route %{code} goes to destination %{dest}, which the file does not have: the displays stay blank", vec![("code", code.clone()), ("dest", t.route.trim().to_string())]));
            }
            let list = h.info_busstop_lists.get(i).map(Vec::as_slice).unwrap_or(&[]);
            if list.is_empty() {
                out.push(issue(at, false, "Route %{code} has no stops: the IBIS shows none", vec![("code", code.clone())]));
            }
            if let Some(s) = list.iter().find(|s| !s.trim().is_empty() && !stops.contains(&s.trim().split('#').next().unwrap_or("").trim().to_lowercase()) && !stops.contains(&s.trim().to_lowercase())) {
                out.push(issue(at, false, "Route %{code} calls at %{stop}, which is no stop of the file: the IBIS shows nothing there", vec![("code", code.clone()), ("stop", s.trim().to_string())]));
            }
        }
        out.sort_by_key(|i| !i.serious);
        out
    }
}

// --- the line editor's blocks ----------------------------------------------------------------

/// `text` without the line editor's blocks (of any map), and each block's map and what is in it
/// (as `linehof::put_block` takes it back).
pub fn split_blocks(text: &str) -> (String, Vec<(String, String)>) {
    const BEGIN: &str = "--- openOMSI line editor, map ";
    const BEGIN_END: &str = ": begin (written again on every save) ---";
    let mut rest = String::with_capacity(text.len());
    let mut blocks: Vec<(String, String)> = Vec::new();
    let mut inside: Option<(String, String, String)> = None;
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        if let Some((_, end, body)) = inside.as_mut() {
            if t == end.as_str() {
                let (map, _, body) = inside.take().unwrap();
                blocks.push((map, body));
            } else if !(body.is_empty() && t.is_empty()) {
                body.push_str(line);
            }
            continue;
        }
        if let Some(map) = t.strip_prefix(BEGIN).and_then(|m| m.strip_suffix(BEGIN_END)) {
            let end = linehof::markers(map).1;
            // (the empty line `put_block` put before it)
            for nl in ["\r\n\r\n", "\n\n"] {
                if rest.ends_with(nl) {
                    rest.truncate(rest.len() - nl.len() / 2);
                    break;
                }
            }
            inside = Some((map.trim().to_string(), end, String::new()));
            continue;
        }
        rest.push_str(line);
    }
    // (a block cut off: what there is of it)
    if let Some((map, _, body)) = inside {
        blocks.push((map, body));
    }
    (rest, blocks)
}

/// The text of `doc` with the line editor's `blocks` after it.
pub fn compose(doc: &Doc, blocks: &[(String, String)]) -> String {
    blocks.iter().fold(doc.to_text(), |t, (map, body)| linehof::put_block(&t, map, body))
}

/// What the line editor wrote into a depot file of `doc`'s layout (a block's text) as a depot
/// file of its own: the destinations, stops and routes of the player's lines.
pub fn block_entries(doc: &Doc, body: &str) -> Hof {
    let text = format!("stringcount_terminus\r\n{}\r\nstringcount_busstop\r\n{}\r\n\r\n{body}", doc.hof.string_count_terminus, doc.hof.string_count_busstop);
    Hof::parse(&omsi_cfg::CfgFile::from_str("block.hof", &text))
}

// --- the files -------------------------------------------------------------------------------

/// One of the player's depot files, as the lists show it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    /// Its file name without `.hof`: what a line names and the game is given (`--hof`).
    pub key: String,
    pub name: String,
    /// The folder of the map it is for ("": any).
    pub map: String,
    /// Its destinations: name, texts and code (the line editor's `OwnDestination`).
    pub termini: Vec<OwnDestination>,
    /// Its special and service trips: code and name.
    pub specials: Vec<(i32, String)>,
}

/// A depot file read: as the editor has it, how it was written, and the line editor's blocks.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub doc: Doc,
    pub coding: Coding,
    pub blocks: Vec<(String, String)>,
}

/// A depot file's bytes as the editor has them (any depot file: one of the player's, the map's
/// or a bus's to start from).
pub fn read(bytes: &[u8]) -> Loaded {
    let (text, coding) = linehof::decode(bytes);
    let (_, blocks) = split_blocks(&text);
    Loaded { doc: Doc::from_text(&text), coding, blocks }
}

pub fn load(path: &Path) -> Result<Loaded, String> {
    omsi_cfg::vfs::read(path).map(|b| read(&b)).map_err(|e| format!("{}: {e}", path.display()))
}

fn is_hof(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("hof"))
}

/// The player's depot files in `dir`, by name.
pub fn list(dir: &Path) -> Vec<Entry> {
    let mut out: Vec<Entry> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_hof(p))
        .filter_map(|p| {
            let l = load(&p).ok()?;
            let key = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let d = &l.doc;
            let termini = d.hof.termini.iter().map(|t| OwnDestination { name: t.texture_id.clone(), sign: t.strings.clone(), code: t.code }).collect();
            let specials = d.hof.termini.iter().enumerate().filter(|(i, _)| d.is_special(*i)).map(|(_, t)| (t.code, t.texture_id.clone())).collect();
            Some(Entry { name: if d.hof.name.trim().is_empty() { key.clone() } else { d.hof.name.trim().to_string() }, key, map: d.map.clone(), termini, specials, path: p })
        })
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.key.cmp(&b.key)));
    out
}

/// Of `entries`, those for the map whose folder is `map` (and those for any map).
pub fn for_map(entries: &[Entry], map: &str) -> Vec<Entry> {
    entries.iter().filter(|e| e.map.trim().is_empty() || e.map.trim().eq_ignore_ascii_case(map.trim())).cloned().collect()
}

/// The player's depot file `key` in `dir` (by its file name, in any case).
pub fn path_of(dir: &Path, key: &str) -> Option<PathBuf> {
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).find(|p| is_hof(p) && p.file_stem().is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(key)))
}

/// `bytes` written to `path` through a file beside it (a write cut off leaves the old file).
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let part = PathBuf::from(format!("{}.openomsi-part", path.display()));
    std::fs::write(&part, bytes).map_err(|e| format!("{}: {e}", part.display()))?;
    std::fs::rename(&part, path).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("{}: {e}", path.display())
    })
}

/// Write `doc` to `path` in `coding`, the line editor's blocks the file has now kept after it.
/// Returns the bytes written.
pub fn save(path: &Path, doc: &Doc, coding: Coding) -> Result<Vec<u8>, String> {
    let blocks = std::fs::read(path).map(|b| split_blocks(&linehof::decode(&b).0).1).unwrap_or_default();
    let bytes = linehof::encode(&compose(doc, &blocks), coding);
    write_file(path, &bytes)?;
    Ok(bytes)
}

/// A new depot file `doc` in `dir`, its file name made from its name (one of its own). Returns
/// where it lies.
pub fn create(dir: &Path, doc: &Doc, coding: Coding) -> Result<PathBuf, String> {
    let stem = format!("{PREFIX}{}", lines::safe_name(&doc.hof.name));
    let path = (1..1000).map(|k| if k == 1 { dir.join(format!("{stem}.hof")) } else { dir.join(format!("{stem}_{k}.hof")) }).find(|p| path_of(dir, &p.file_stem().unwrap_or_default().to_string_lossy()).is_none()).ok_or("no free file name")?;
    let bytes = linehof::encode(&doc.to_text(), coding);
    write_file(&path, &bytes)?;
    Ok(path)
}

/// The player's depot file at `path` put aside (into `deleted` beside it, so that it can be had
/// back), and its copies beside buses in the content folder `content` gone with it.
pub fn delete(path: &Path, content: Option<&Path>) -> Result<(), String> {
    let file = path.file_name().unwrap_or_default().to_os_string();
    let aside = path.parent().map(|d| d.join("deleted")).ok_or("no folder")?;
    std::fs::create_dir_all(&aside).map_err(|e| e.to_string())?;
    let to = aside.join(format!("{}-{}", crate::lines::now_secs(), file.to_string_lossy()));
    std::fs::rename(path, &to).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(c) = content {
        for p in copies_in(c, &file.to_string_lossy()) {
            let _ = std::fs::remove_file(&p);
        }
        omsi_cfg::content_changed();
    }
    Ok(())
}

/// The map's depot file of the player's own: the first for `map` in `dir`, else a new one
/// called `name` made from `base` (the bytes of the map's depot file; None: an empty one).
pub fn ensure_for_map(dir: &Path, map: &str, name: &str, base: Option<&[u8]>) -> Result<PathBuf, String> {
    if let Some(e) = list(dir).into_iter().find(|e| e.map.trim().eq_ignore_ascii_case(map.trim())) {
        return Ok(e.path);
    }
    create_from(dir, name, map, base)
}

/// A new depot file of the player's in `dir` called `name`, for the map `map`: a copy of `base`
/// (a depot file's bytes: the map's, a bus's - without the line editor's blocks, its special
/// trips marked anew), or an empty one laid out as OMSI's own.
pub fn create_from(dir: &Path, name: &str, map: &str, base: Option<&[u8]>) -> Result<PathBuf, String> {
    let (mut doc, coding) = match base {
        Some(b) => {
            let l = read(b);
            (l.doc, l.coding)
        }
        None => (Doc::blank(name, map), Coding::Page(CodePage::Windows1252)),
    };
    doc.hof.name = name.trim().to_string();
    doc.map = map.trim().to_string();
    doc.specials.clear();
    create(dir, &doc, coding)
}

// --- the player's own destinations (the line editor's) ---------------------------------------

/// The destinations of `entries` (the player's depot files of the map) that `known` - the
/// names of the destinations the line's depot file has, lowercased - lacks: what the line
/// editor offers as "Mine: …", each name once.
pub fn own_destinations(entries: &[Entry], known: &HashSet<String>) -> Vec<OwnDestination> {
    let mut seen: HashSet<String> = known.clone();
    let mut out = Vec::new();
    for e in entries {
        for t in &e.termini {
            let k = t.name.trim().to_lowercase();
            if !k.is_empty() && seen.insert(k) {
                out.push(t.clone());
            }
        }
    }
    out.sort_by_key(|d| d.name.to_lowercase());
    out
}

/// Keep the destination `d` in the player's depot file at `path`: a new one with its texts (the
/// empty ones made from its name) and its code when that is free (else a free one); one of that
/// name keeps its own. Returns its code. The copies beside buses (`content`) follow.
pub fn keep_destination(path: &Path, d: &OwnDestination, content: Option<&Path>) -> Result<i32, String> {
    let l = load(path)?;
    let mut doc = l.doc;
    if let Some(i) = doc.destination(&d.name) {
        return Ok(doc.hof.termini[i].code);
    }
    let i = doc.add_destination(&d.name, false, false);
    for (k, s) in d.sign.iter().enumerate() {
        if !s.trim().is_empty() {
            if let Some(x) = doc.hof.termini[i].strings.get_mut(k) {
                *x = s.clone();
            }
        }
    }
    if d.code > 0 && !doc.hof.termini.iter().any(|t| t.code == d.code) {
        doc.hof.termini[i].code = d.code;
    }
    let code = doc.hof.termini[i].code;
    let bytes = save(path, &doc, l.coding)?;
    if let Some(c) = content {
        refresh(c, &path.file_name().unwrap_or_default().to_string_lossy(), &bytes);
    }
    Ok(code)
}

/// The destination `name` out of the player's depot file at `path`. Returns whether it was
/// there.
pub fn drop_destination(path: &Path, name: &str, content: Option<&Path>) -> Result<bool, String> {
    let l = load(path)?;
    let mut doc = l.doc;
    let Some(i) = doc.destination(name) else { return Ok(false) };
    let code = doc.hof.termini.remove(i).code;
    doc.specials.retain(|c| *c != code);
    let bytes = save(path, &doc, l.coding)?;
    if let Some(c) = content {
        refresh(c, &path.file_name().unwrap_or_default().to_string_lossy(), &bytes);
    }
    Ok(true)
}

/// The player's own destinations of a registry made before the depot editor
/// (`Registry::destinations`) moved into the map's depot file of the player's own - made, when
/// there is none, from `base` (the map's depot file's bytes) and called `name`: one list of
/// them. Returns the file and how many moved; None when there were none.
pub fn migrate(reg: &mut Registry, dir: &Path, name: &str, base: Option<&[u8]>) -> Result<Option<(PathBuf, usize)>, String> {
    if reg.destinations.is_empty() {
        return Ok(None);
    }
    let path = ensure_for_map(dir, &reg.map, name, base)?;
    let n = reg.destinations.len();
    for d in reg.destinations.clone() {
        keep_destination(&path, &d, None)?;
    }
    reg.destinations.clear();
    Ok(Some((path, n)))
}

// --- the lines -------------------------------------------------------------------------------

/// The lines of `reg` that are written and chose the depot file `key`.
fn lines_of<'a>(reg: &'a Registry, key: &str) -> Vec<&'a LineDesign> {
    reg.lines.iter().filter(|l| lines::written(l) && l.depot_file.trim().eq_ignore_ascii_case(key.trim())).collect()
}

/// A depot file of the player's as the line editor of map `map` sees it (without its block).
pub fn depot_at(path: &Path, map: &str) -> Option<Depot> {
    let b = std::fs::read(path).ok()?;
    let (text, _) = linehof::decode(&b);
    let doc = Doc::from_text(&linehof::strip_block(&text, map));
    Some(doc.depot())
}

/// Before the registry is saved, after `linehof::prepare`: the codes of the lines that chose a
/// depot file of the player's own given in it too - each kept where it is free there, in the
/// map's depot files (`bases`: the copies of their depot groups' files, `groups`) and of every
/// other line; else a new one free in all of them, so that each file the line is written into
/// has it once.
pub fn assign(reg: &mut Registry, dir: &Path, groups: &HashMap<String, String>, bases: &[PathBuf]) {
    let mut keys: Vec<String> = Vec::new();
    for l in reg.lines.iter().filter(|l| lines::written(l) && !l.depot_file.trim().is_empty()) {
        if !keys.iter().any(|k| k.eq_ignore_ascii_case(l.depot_file.trim())) {
            keys.push(l.depot_file.trim().to_string());
        }
    }
    for key in keys {
        let Some(path) = path_of(dir, &key) else { continue };
        let Some(depot) = depot_at(&path, &reg.map) else { continue };
        let ids: Vec<u64> = lines_of(reg, &key).iter().map(|l| l.id).collect();
        let mut taken = Taken::default();
        taken.add(&depot.hof);
        // the map's depot files of these lines
        let mut names: Vec<String> = Vec::new();
        for l in reg.lines.iter().filter(|l| ids.contains(&l.id)) {
            if let Some(n) = linehof::depot_of(l, groups) {
                if !names.iter().any(|x| x.eq_ignore_ascii_case(&n)) {
                    names.push(n);
                }
            }
        }
        for n in &names {
            for c in linehof::copies(n, bases) {
                if let Some(d) = Depot::load(&c.source, &reg.map) {
                    taken.add(&d.hof);
                }
            }
        }
        // and every other line's
        for l in reg.lines.iter().filter(|l| !ids.contains(&l.id)) {
            for d in &l.directions {
                if d.terminus_code > 0 {
                    taken.termini.insert(d.terminus_code);
                }
                if d.ibis_route > 0 {
                    taken.routes.insert(d.ibis_route);
                }
            }
        }
        linehof::assign_lines(reg, &ids, &depot, &taken);
    }
}

/// After the registry is saved: each depot file of the player's own in `dir` given the lines of
/// the map `reg.map` that chose it (`linehof::block`, at its end; none: the block taken out),
/// and its copies beside buses in the content folder (`content`) made the same again. Returns
/// how many files changed and the first that could not be written.
pub fn write_lines(reg: &Registry, dir: &Path, content: Option<&Path>) -> (usize, Option<String>) {
    let mut changed = 0;
    let mut error = None;
    for e in list(dir) {
        let Ok(b) = std::fs::read(&e.path) else { continue };
        let (text, coding) = linehof::decode(&b);
        let lines = lines_of(reg, &e.key);
        let had = split_blocks(&text).1.iter().any(|(m, _)| m.eq_ignore_ascii_case(&reg.map));
        if lines.is_empty() && !had {
            continue;
        }
        let block = if lines.is_empty() {
            String::new()
        } else {
            let depot = Doc::from_text(&linehof::strip_block(&text, &reg.map)).depot();
            linehof::block(&lines, &depot)
        };
        let new = linehof::put_block(&text, &reg.map, &block);
        if new == text {
            continue;
        }
        let bytes = linehof::encode(&new, coding);
        match write_file(&e.path, &bytes) {
            Ok(()) => {
                changed += 1;
                if let Some(c) = content {
                    refresh(c, &e.path.file_name().unwrap_or_default().to_string_lossy(), &bytes);
                }
            }
            Err(err) => {
                error.get_or_insert(err);
            }
        }
    }
    (changed, error)
}

// --- the buses -------------------------------------------------------------------------------

/// Every copy called `file` beside a bus in the content folder `content` (`Vehicles/<pack>/` and
/// a folder in it).
fn copies_in(content: &Path, file: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut look = |d: &Path| {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(file)) {
                out.push(p);
            }
        }
    };
    for pack in std::fs::read_dir(content.join("Vehicles")).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        look(&pack);
        for sub in std::fs::read_dir(&pack).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            look(&sub);
        }
    }
    out
}

/// The copies of a player's depot file (`file`, its file name) beside buses in the content
/// folder made the same as it (`bytes`) again. Returns how many changed.
pub fn refresh(content: &Path, file: &str, bytes: &[u8]) -> usize {
    let mut n = 0;
    for p in copies_in(content, file) {
        if std::fs::read(&p).is_ok_and(|b| b != bytes) && write_file(&p, bytes).is_ok() {
            n += 1;
        }
    }
    if n > 0 {
        omsi_cfg::content_changed();
    }
    n
}

/// Put a copy of the player's depot file `source` into `target_dir` under its own file name -
/// over the copy put there before (made the same again), never over another file: `beside` are
/// every content root's copies of the bus's folder, and a file of that name in one of them is
/// not ours; never into the vehicles of the OMSI 2 folder `original`. Returns where it lies and
/// whether it was written.
pub fn give_into(source: &Path, target_dir: &Path, beside: &[PathBuf], original: Option<&Path>) -> Result<(PathBuf, bool), Refusal> {
    let file = source.file_name().unwrap_or_default().to_string_lossy().into_owned();
    if !is_hof(source) {
        return Err(Refusal::Unreadable(format!("{file} is no .hof file")));
    }
    if original.is_some_and(|o| crate::depot::lies_in(target_dir, &o.join("Vehicles"))) {
        return Err(Refusal::Original);
    }
    let target = target_dir.join(&file);
    for d in beside {
        if crate::depot::lies_in(d, target_dir) && crate::depot::lies_in(target_dir, d) {
            continue;
        }
        if let Some((n, _)) = omsi_cfg::vfs::list_dir(d).unwrap_or_default().into_iter().find(|(n, dir)| !dir && n.to_string_lossy().eq_ignore_ascii_case(&file)) {
            return Err(Refusal::Taken(n.to_string_lossy().into_owned()));
        }
    }
    let bytes = std::fs::read(source).map_err(|e| Refusal::Unreadable(e.to_string()))?;
    if std::fs::read(&target).is_ok_and(|b| b == bytes) {
        return Ok((target, false));
    }
    write_file(&target, &bytes).map_err(Refusal::Failed)?;
    Ok((target, true))
}

/// Give the bus `bus_file` (`Vehicles/<pack>/x.bus`, as the lists name it) the player's depot
/// file `key`: a copy in openOMSI's content folder, in the folder the bus's depot files are read
/// from. Returns where it lies.
pub fn give(bus_file: &str, key: &str) -> Result<PathBuf, Refusal> {
    let source = path_of(&dir(), key).ok_or(Refusal::NoSource)?;
    let content = crate::content_dir().ok_or(Refusal::NoFolder)?;
    let target_dir = crate::depot::target_for(bus_file).ok_or(Refusal::NoFolder)?;
    let rel = target_dir.strip_prefix(&content).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
    let beside = crate::depot::copies_of(&rel);
    let original = crate::root().ok();
    let r = give_into(&source, &target_dir, &beside, original.as_deref());
    match &r {
        Ok((p, true)) => {
            crate::log_line(&format!("depot: the player's {} given to {bus_file} ({})", source.display(), p.display()));
            crate::depot::record(p, &source);
            omsi_cfg::content_changed();
        }
        Ok(_) => {}
        Err(e) => crate::log_line(&format!("depot: the player's {} not given to {bus_file}: {e}", source.display())),
    }
    r.map(|(p, _)| p)
}

/// The player's depot file a drive is to have (its key): the one the duty names (`hof`, chosen
/// on the bus step) when it is one of `keys` (the player's), else the one its line chose - a
/// line of the player's own (`line`, the timetable line's name) of the registry `reg`.
pub fn wanted(keys: &[String], reg: Option<&Registry>, line: Option<&str>, hof: Option<&str>) -> Option<String> {
    let mine = |k: &str| keys.iter().find(|x| x.eq_ignore_ascii_case(k.trim())).cloned();
    if let Some(k) = hof.and_then(mine) {
        return Some(k);
    }
    let (reg, line) = (reg?, line?.trim());
    if !lines::is_own_file(line) {
        return None;
    }
    let stems = lines::stems(reg);
    let l = reg.lines.iter().find(|l| stems.get(&l.id).is_some_and(|s| s.eq_ignore_ascii_case(line)))?;
    mine(&l.depot_file)
}

/// Before a drive starts: the player's depot file it is to have (`wanted`) given to its bus and
/// named for the game (`--hof`, by its file name: the bus's own files are looked at by it
/// first). Returns what was done, for the log.
pub fn for_duty(d: &mut crate::Duty) -> Option<String> {
    if d.tutorial.is_some() || d.situation.as_deref().is_some_and(|s| !s.trim().is_empty()) || d.bus.trim().is_empty() {
        return None;
    }
    let keys: Vec<String> = list(&dir()).into_iter().map(|e| e.key).collect();
    if keys.is_empty() {
        return None;
    }
    let folder = lines::map_folder(&d.map);
    let path = lines::registry_path(&folder);
    let reg = (!folder.is_empty() && path.is_file()).then(|| lines::load_registry(&path));
    let key = wanted(&keys, reg.as_ref(), d.line.as_deref(), d.hof.as_deref())?;
    Some(match give(&d.bus, &key) {
        Ok(p) => {
            d.hof = Some(key.clone());
            format!("the drive has the player's depot file {key} ({})", p.display())
        }
        Err(e) => format!("the player's depot file {key} could not be given to {}: {e}", d.bus),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linehof::tests::{groups, registry, GRUNDORF};

    /// A folder of its own under the system's temporary folder, gone at the end.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Scratch {
            let d = std::env::temp_dir().join(format!("openomsi-owndepot-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Scratch(d)
        }
        fn file(&self, rel: &str, bytes: &[u8]) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, bytes).unwrap();
            p
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn cp1252(text: &str) -> Vec<u8> {
        linehof::encode(text, Coding::Page(CodePage::Windows1252))
    }

    #[test]
    fn a_depot_file_is_kept_as_it_was_written() {
        let mut l = read(&cp1252(GRUNDORF));
        assert!(matches!(l.coding, Coding::Page(_)));
        assert_eq!(l.doc.hof.termini.len(), 5);
        assert_eq!(l.doc.notes[5], "IBIS2-Display (Klarname in Groß-/Kleinschreibung), max 20 Zeichen");
        l.doc.map = "Grundorf".into();
        l.doc.hof.name = "Grundorf - mine".into();
        let k = l.doc.add_destination("Sonderfahrt", false, true);
        assert!(l.doc.is_special(k) && l.doc.is_special(0), "marked, and the one where nobody boards");
        let text = l.doc.to_text();
        let back = Doc::from_text(&text);
        assert_eq!(back, l.doc);
        assert_eq!((back.map.as_str(), back.specials.as_slice()), ("Grundorf", [l.doc.hof.termini[k].code].as_slice()));
        // the line editor sees it as the stock file: what each text is for
        assert_eq!(back.depot().roles, Doc::from_text(GRUNDORF).depot().roles);
        // with a block of the line editor's: kept apart, put back after
        let with = linehof::put_block(&text, "Grundorf", "[addterminus]\r\n901\r\nMarkt\r\n");
        let (rest, blocks) = split_blocks(&with);
        assert_eq!(rest, text);
        assert_eq!(blocks, vec![("Grundorf".to_string(), "[addterminus]\r\n901\r\nMarkt\r\n".to_string())]);
        assert_eq!(compose(&back, &blocks), with);
        assert_eq!(Doc::from_text(&with), back, "the block is not the editor's");
        assert_eq!(block_entries(&back, &blocks[0].1).termini[0].texture_id, "Markt");
    }

    #[test]
    fn a_new_depot_file_is_laid_out_as_omsis_own() {
        let mut d = Doc::blank("Grundorf - mine", "Grundorf");
        let i = d.add_destination("Marktplatz Süd", false, false);
        let s = &d.hof.termini[i].strings;
        assert_eq!(s.len(), 8);
        assert_eq!((s[0].as_str(), s[1].as_str(), s[4].as_str(), s[5].as_str()), ("MARKTPLATZ SÜD", "MARKTPLATZ SÜD", "", "Marktplatz Süd"));
        assert_eq!(d.hof.termini[i].code, 901);
        let b = d.add_destination("Betriebsfahrt", true, true);
        assert_eq!(d.hof.termini[b].code, 900, "a special trip's code from 900");
        assert!(d.hof.termini[b].all_exit && d.hof.termini[b].terminus_stop.is_none());
        let st = d.add_stop("Kirche am Markt");
        assert_eq!(d.hof.bus_stops[st].strings[0], "KIRCHE AM MARKT");
        d.set_string_count(9);
        assert!(d.hof.termini.iter().all(|t| t.strings.len() == 9) && d.notes.len() == 9);
        assert_eq!(d.free_route(0), 101);
    }

    #[test]
    fn what_is_wrong_is_said_plainly() {
        let mut d = Doc::blank("Grundorf", "Grundorf");
        let a = d.add_destination("Markt", false, false);
        let b = d.add_destination("Bahnhof", false, false);
        d.hof.termini[b].code = d.hof.termini[a].code;
        let c = d.add_destination("", false, false);
        let long = d.add_destination("Albert", false, false);
        d.hof.termini[long].strings[0] = "ALBERT-EINSTEIN-STRASSE".into();
        let quiet = d.add_destination("Leer", false, false);
        for s in &mut d.hof.termini[quiet].strings {
            s.clear();
        }
        d.hof.info_trips.push(omsi_vehicle::hof::InfoTrip { code: "4201".into(), name: "A-B".into(), route: "777".into(), line: "42".into(), extra: vec![] });
        d.hof.info_busstop_lists.push(vec!["Nirgendwo".into()]);
        d.hof.termini[a].texture_id = "Markt Ő".into();
        let issues = d.issues("Grundorf", Coding::Page(CodePage::Windows1252));
        let says = |at: Place, part: &str| issues.iter().any(|i| i.at == at && i.english().contains(part));
        assert!(says(Place::File, "name of the map's own depot file"));
        assert!(says(Place::Terminus(b), "is given twice"));
        assert!(says(Place::Terminus(c), "has no name"));
        assert!(says(Place::Terminus(long), "text 1 has 23 characters, its display shows 16"));
        assert!(says(Place::Terminus(quiet), "shows nothing"));
        assert!(says(Place::Trip(0), "destination 777"));
        assert!(says(Place::Trip(0), "Nirgendwo"));
        assert!(says(Place::File, ": Ő"));
        assert!(issues.first().is_some_and(|i| i.serious), "the game's problems first");
        assert!(Doc::blank("Mine", "X").issues("Grundorf", Coding::Utf16).iter().all(|i| !i.serious));
    }

    #[test]
    fn the_player_keeps_destinations_of_his_own_in_the_maps_depot_file() {
        let s = Scratch::new("mine");
        let dir = s.0.join("depots");
        let mut reg = Registry { map: "Grundorf".into(), ..Default::default() };
        reg.keep_destination(OwnDestination { name: "Shuttle Wurzbach".into(), sign: vec!["SHUTTLE".into()], code: 950 });
        reg.keep_destination(OwnDestination { name: "Krankenhaus".into(), ..Default::default() });
        let (path, n) = migrate(&mut reg, &dir, "Grundorf - mine", Some(&cp1252(GRUNDORF))).unwrap().unwrap();
        assert_eq!(n, 2);
        assert!(reg.destinations.is_empty(), "one list: the depot file's");
        assert!(path.file_name().unwrap().to_string_lossy().starts_with("oo_Grundorf"));
        let e = list(&dir);
        assert_eq!((e.len(), e[0].name.as_str(), e[0].map.as_str()), (1, "Grundorf - mine", "Grundorf"));
        let shuttle = e[0].termini.iter().find(|t| t.name == "Shuttle Wurzbach").unwrap();
        assert_eq!((shuttle.code, shuttle.sign[0].as_str()), (950, "SHUTTLE"));
        assert_eq!(e[0].termini.iter().filter(|t| t.name == "Krankenhaus").count(), 1, "the map's own is not made twice");
        // offered as "Mine": what the map's file lacks
        let known: HashSet<String> = Doc::from_text(GRUNDORF).hof.termini.iter().map(|t| t.texture_id.to_lowercase()).collect();
        assert_eq!(own_destinations(&for_map(&e, "grundorf"), &known).iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Shuttle Wurzbach"]);
        assert!(for_map(&e, "Spandau").is_empty());
        // kept and dropped there
        assert_eq!(keep_destination(&path, &OwnDestination { name: "Rathaus".into(), code: 950, ..Default::default() }, None).unwrap(), 901, "950 is the shuttle's");
        assert!(drop_destination(&path, "shuttle wurzbach", None).unwrap());
        assert!(!drop_destination(&path, "Shuttle Wurzbach", None).unwrap());
        // nothing to move a second time; the same file for the map
        assert_eq!(migrate(&mut reg, &dir, "x", None).unwrap(), None);
        assert_eq!(ensure_for_map(&dir, "Grundorf", "y", None).unwrap(), path);
        // the file keeps its code page
        assert!(std::fs::read(&path).unwrap().windows(3).any(|w| w == [b'o', 0xDF, b'-']), "Groß- in Windows-1252");
    }

    /// A line that chose the player's depot file: its destination, stops and route go into it
    /// with codes free there and in the map's depot files, the file's own entries stay, and
    /// the copies beside buses follow.
    #[test]
    fn a_lines_displays_go_into_the_players_depot_file() {
        let s = Scratch::new("lines");
        let (dir, content, omsi) = (s.0.join("depots"), s.0.join("content"), s.0.join("omsi"));
        s.file("omsi/Vehicles/MAN_NL/Grundorf.hof", &cp1252(GRUNDORF));
        let bases = vec![content.clone(), omsi.clone()];
        // the player's file: the map's, with route 4201 and destination 901 taken by his own
        let mut doc = read(&cp1252(GRUNDORF)).doc;
        doc.hof.name = "Grundorf - mine".into();
        doc.map = "Grundorf".into();
        let k = doc.add_destination("Sportplatz", false, false);
        assert_eq!(doc.hof.termini[k].code, 901);
        doc.hof.info_trips.push(omsi_vehicle::hof::InfoTrip { code: "4201".into(), name: "X".into(), route: "901".into(), line: "42".into(), extra: vec![] });
        doc.tidy();
        let path = create(&dir, &doc, Coding::Page(CodePage::Windows1252)).unwrap();
        let key = path.file_stem().unwrap().to_string_lossy().into_owned();
        // a copy of it beside a bus already, from before
        let copy = s.file(&format!("content/Vehicles/O530/{key}.hof"), b"old");
        let mut reg = registry();
        reg.lines[0].depot_file = key.clone();
        let plan = linehof::prepare(&mut reg, &groups(), &bases);
        assign(&mut reg, &dir, &groups(), &bases);
        let l = &reg.lines[0];
        let codes = (l.directions[0].terminus_code, l.directions[0].ibis_route, l.directions[1].ibis_route);
        assert_eq!(codes, (902, 4202, 4203), "free in the player's file and the map's");
        let (n, err) = linehof::write(&content, Some(&omsi), &reg, &groups(), &plan);
        assert_eq!((n, err), (1, None));
        assert_eq!(write_lines(&reg, &dir, Some(&content)), (1, None));
        let mine = load(&path).unwrap();
        assert_eq!(mine.doc.hof.termini.len(), 6, "its own entries as they were");
        assert_eq!(mine.blocks.len(), 1);
        let h = Hof::parse(&omsi_cfg::CfgFile::from_str("x.hof", &linehof::decode(&std::fs::read(&path).unwrap()).0));
        assert!(h.termini.iter().any(|t| t.texture_id == "Marktplatz Süd" && t.code == 902));
        assert!(h.info_trips.iter().any(|t| t.code == "4202" && t.route == "902") && h.info_trips.iter().any(|t| t.code == "4203" && t.route == "107"));
        assert_eq!(std::fs::read(&copy).unwrap(), std::fs::read(&path).unwrap(), "the bus's copy follows");
        // the map's file has the same codes
        let map = Hof::load(&content.join("Vehicles/MAN_NL/Grundorf.hof")).unwrap();
        assert!(map.termini.iter().any(|t| t.texture_id == "Marktplatz Süd" && t.code == 902));
        // saved again: nothing changes
        let before = reg.clone();
        linehof::prepare(&mut reg, &groups(), &bases);
        assign(&mut reg, &dir, &groups(), &bases);
        assert_eq!(reg, before);
        assert_eq!(write_lines(&reg, &dir, Some(&content)), (0, None));
        // the editor saves the file: the line's block stays
        let mut edited = mine.doc.clone();
        edited.hof.termini[k].strings[0] = "SPORT".into();
        save(&path, &edited, mine.coding).unwrap();
        assert_eq!(load(&path).unwrap().blocks, mine.blocks);
        // the line chooses the map's again: its block goes
        reg.lines[0].depot_file.clear();
        assert_eq!(write_lines(&reg, &dir, Some(&content)), (1, None));
        assert!(load(&path).unwrap().blocks.is_empty());
        assert_eq!(load(&path).unwrap().doc, edited);
    }

    #[test]
    fn a_bus_is_given_the_players_depot_file_and_it_is_kept_the_same() {
        let s = Scratch::new("give");
        let omsi = s.0.join("omsi");
        let source = s.file("depots/oo_Mine.hof", &cp1252("[name]\r\nMine\r\n"));
        s.file("omsi/Vehicles/MAN_NL/Grundorf.hof", b"[name]\r\nGrundorf\r\n");
        let bus = s.0.join("omsi/Vehicles/MAN_NL");
        let target = s.0.join("content/Vehicles/MAN_NL");
        let (p, written) = give_into(&source, &target, &[bus.clone(), target.clone()], Some(&omsi)).unwrap();
        assert_eq!((p.clone(), written), (target.join("oo_Mine.hof"), true));
        assert_eq!(std::fs::read(&p).unwrap(), std::fs::read(&source).unwrap());
        assert!(!bus.join("oo_Mine.hof").exists(), "the OMSI 2 folder's bus is not touched");
        // again: nothing to write; changed: written again
        assert!(!give_into(&source, &target, std::slice::from_ref(&bus), Some(&omsi)).unwrap().1);
        std::fs::write(&source, cp1252("[name]\r\nMine 2\r\n")).unwrap();
        assert!(give_into(&source, &target, std::slice::from_ref(&bus), Some(&omsi)).unwrap().1);
        assert_eq!(std::fs::read(&p).unwrap(), std::fs::read(&source).unwrap());
        // all the copies beside buses follow a change
        let other = s.file("content/Vehicles/Pack/Solo/oo_mine.hof", b"old");
        std::fs::write(&source, cp1252("[name]\r\nMine 3\r\n")).unwrap();
        assert_eq!(refresh(&s.0.join("content"), "oo_Mine.hof", &std::fs::read(&source).unwrap()), 2);
        assert_eq!(std::fs::read(&other).unwrap(), std::fs::read(&source).unwrap());
        // never over a file of that name in another root, never into the OMSI 2 folder
        s.file("omsi/Vehicles/Solaris/oo_Mine.hof", b"someone's");
        assert_eq!(give_into(&source, &s.0.join("content/Vehicles/Solaris"), &[s.0.join("omsi/Vehicles/Solaris")], Some(&omsi)), Err(Refusal::Taken("oo_Mine.hof".into())));
        assert_eq!(give_into(&source, &s.0.join("omsi/Vehicles/X"), &[], Some(&omsi)), Err(Refusal::Original));
        // put aside: its copies go with it
        delete(&source, Some(&s.0.join("content"))).unwrap();
        assert!(!source.exists() && !other.exists() && !p.exists());
        assert_eq!(std::fs::read_dir(s.0.join("depots/deleted")).unwrap().count(), 1);
    }

    #[test]
    fn the_drive_has_the_file_the_duty_or_its_line_chose() {
        let keys = vec!["oo_Mine".to_string(), "oo_Specials".to_string()];
        let mut reg = registry();
        let stem = lines::stems(&reg)[&reg.lines[0].id].clone();
        assert_eq!(wanted(&keys, Some(&reg), Some(&stem), Some("Grundorf")), None, "a line of the map's depot file");
        reg.lines[0].depot_file = "OO_mine".into();
        assert_eq!(wanted(&keys, Some(&reg), Some(&stem), Some("Grundorf")).as_deref(), Some("oo_Mine"));
        assert_eq!(wanted(&keys, Some(&reg), Some(&stem), Some("oo_specials")).as_deref(), Some("oo_Specials"), "the one chosen on the bus step");
        assert_eq!(wanted(&keys, None, None, Some("oo_Specials")).as_deref(), Some("oo_Specials"), "any bus, no line: a special trip");
        assert_eq!(wanted(&keys, Some(&reg), Some("Linie 7"), None), None, "a line of the map's");
        reg.lines[0].depot_file = "oo_gone".into();
        assert_eq!(wanted(&keys, Some(&reg), Some(&stem), None), None, "a file no longer there");
    }
}
