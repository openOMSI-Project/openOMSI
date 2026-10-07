//! `.hof` depot files: termini, bus stop display strings, IBIS info system.

use omsi_cfg::CfgFile;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Terminus {
    pub code: i32,
    /// Texture change id.
    pub texture_id: String,
    /// Bus stop where everybody leaves (None for `_allexit`).
    pub terminus_stop: Option<String>,
    pub all_exit: bool,
    pub strings: Vec<String>,
}

impl Terminus {
    /// Name used for destination text while preserving the original HOF string indices.
    pub fn display_name(&self) -> String {
        let first = self.strings.iter().find(|s| !s.trim().is_empty()).map(String::as_str);
        if first.is_some_and(|s| is_destination_image_path(s)) {
            return self.texture_id.trim().to_string();
        }
        first.map(str::trim).filter(|s| !s.is_empty()).unwrap_or_else(|| self.texture_id.trim()).to_string()
    }

    /// Name used by the destination menu: the identifier on the second `[addterminus]` line.
    pub fn menu_name(&self) -> String {
        let first = self.strings.iter().find(|s| !s.trim().is_empty()).map(String::as_str).unwrap_or("").trim();
        if !first.is_empty() && (first.eq_ignore_ascii_case("no") || is_destination_image_path(first) || has_route_label(&self.texture_id)) {
            let id = self.texture_id.trim();
            if !id.is_empty() {
                return id.to_string();
            }
        }
        self.display_name()
    }
}

fn is_destination_image_path(s: &str) -> bool {
    let s = s.trim().to_ascii_lowercase();
    s.ends_with(".bmp") || s.ends_with(".tga") || s.ends_with(".png")
}

fn has_route_label(s: &str) -> bool {
    let Some((prefix, _)) = s.trim().split_once(':') else { return false };
    !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit() || c.is_ascii_alphabetic())
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BusStop {
    pub ident: String,
    pub strings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct InfoTrip {
    pub code: String,
    pub name: String,
    pub route: String,
    pub line: String,
    pub extra: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hof {
    pub path: PathBuf,
    pub name: String,
    pub service_trip: String,
    pub global_strings: Vec<String>,
    pub string_count_terminus: usize,
    pub string_count_busstop: usize,
    pub termini: Vec<Terminus>,
    pub bus_stops: Vec<BusStop>,
    pub info_trips: Vec<InfoTrip>,
    pub info_busstop_lists: Vec<Vec<String>>,
    pub info_busstops: Vec<Vec<String>>,
}

impl Hof {
    pub fn load(path: &Path) -> Result<Hof, omsi_cfg::CfgError> {
        let f = CfgFile::read(path)?;
        Ok(Self::parse(&f))
    }

    /// Whether the IBIS trips carry line `line` (a map's line name, "11-11s"): a trip coded
    /// line x 100 + route, or one named after the line ("91.06A -> MASSY GARE").
    pub fn has_line(&self, line: &str) -> bool {
        let l = line.trim().to_ascii_lowercase();
        if l.is_empty() {
            return false;
        }
        let digits: String = l.chars().take_while(|c| c.is_ascii_digit()).collect();
        let number = digits.parse::<i32>().ok().filter(|n| *n > 0);
        let boundary = |rest: &str| rest.chars().next().is_none_or(|c| !c.is_ascii_alphanumeric());
        self.info_trips.iter().any(|t| {
            let code = omsi_cfg::parse_i32(&t.code);
            let head = t.name.split("->").next().unwrap_or("").trim().to_ascii_lowercase();
            let named = !head.is_empty()
                && (l.strip_prefix(head.as_str()).is_some_and(boundary)
                    || (l.ends_with(|c: char| c.is_ascii_digit())
                        && head.strip_prefix(l.as_str()).is_some_and(|r| r.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))));
            named || number.is_some_and(|n| code >= 100 && code / 100 == n)
        })
    }

    /// Only the `[name]` of a depot file ("" when it has none), kept for the session: the
    /// searches by name below read every depot file of every vehicle folder, and parsing
    /// each whole (termini, stops, the IVU trips) made a big installation's start take
    /// minutes.
    pub fn read_name(path: &Path) -> Option<String> {
        type Names = std::collections::HashMap<PathBuf, Option<String>>;
        static CACHE: std::sync::OnceLock<std::sync::Mutex<(u64, Names)>> = std::sync::OnceLock::new();
        let cache = CACHE.get_or_init(Default::default);
        let generation = omsi_cfg::content_generation();
        {
            let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
            if c.0 != generation {
                *c = (generation, Names::new());
            }
            if let Some(n) = c.1.get(path) {
                return n.clone();
            }
        }
        let name = CfgFile::read(path).ok().map(|f| {
            let mut r = f.reader().with_rule(omsi_cfg::KeywordRule::TrimEnd);
            while let Some(k) = r.next_keyword() {
                if k == "name" {
                    return r.str().to_string();
                }
            }
            String::new()
        });
        cache.lock().unwrap_or_else(|e| e.into_inner()).1.insert(path.to_path_buf(), name.clone());
        name
    }

    pub fn parse(f: &CfgFile) -> Hof {
        let mut h = Hof { path: f.path.clone(), string_count_terminus: 0, string_count_busstop: 0, ..Default::default() };
        // `stringcount_terminus` / `stringcount_busstop` are bare (unbracketed) directives.
        for (i, l) in f.lines.iter().enumerate() {
            let w = l.trim_end();
            if w.eq_ignore_ascii_case("stringcount_terminus") {
                h.string_count_terminus = f.lines.get(i + 1).map(|s| omsi_cfg::parse_i64(s).max(0) as usize).unwrap_or(0);
            } else if w.eq_ignore_ascii_case("stringcount_busstop") {
                h.string_count_busstop = f.lines.get(i + 1).map(|s| omsi_cfg::parse_i64(s).max(0) as usize).unwrap_or(0);
            }
        }
        // the stock depot files are spreadsheet exports: every line ends in tabs, which the
        // original cuts off (with spaces and quotes) before looking at it
        let mut r = f.reader().with_rule(omsi_cfg::KeywordRule::TrimEnd);
        while let Some(k) = r.next_keyword() {
            match k.as_str() {
                "name" => h.name = r.str().to_string(),
                "servicetrip" => h.service_trip = r.str().to_string(),
                "global_strings" => {
                    let n = r.usize();
                    h.global_strings = (0..n).map(|_| r.str().to_string()).collect();
                }
                "addterminus" | "addterminus_allexit" => {
                    // Despite the SDK comment, no shipped file carries a separate terminus
                    // station line: every record is code, ident, then stringcount strings
                    // (verified over all stock .hof files). The ident doubles as the station.
                    let all_exit = k.ends_with("allexit");
                    let code = r.i32();
                    let texture_id = r.str().to_string();
                    let terminus_stop = if all_exit { None } else { Some(texture_id.clone()) };
                    let strings: Vec<String> = (0..h.string_count_terminus).map(|_| r.str().to_string()).collect();
                    h.termini.push(Terminus { code, texture_id, terminus_stop, all_exit, strings });
                }
                "addterminus_list" => {
                    // One row per terminus, tab separated: a flag column (`{ALLEX}` or
                    // empty), the code, the ident and the display strings - every one of the
                    // 3 549 rows of the stock and installed depot files has the flag column,
                    // empty ones included. Reading the code from the flag column gave every
                    // terminus without `{ALLEX}` the code 0 and its code as ident: typed
                    // destination codes were "wrong", a route found no terminus and the
                    // displays fell back to the first (empty) entry.
                    for l in r.until("[end]") {
                        if l.trim().is_empty() {
                            continue;
                        }
                        let cols: Vec<&str> = l.split('\t').collect();
                        let all_exit = cols[0].trim().eq_ignore_ascii_case("{ALLEX}");
                        let code = omsi_cfg::parse_i32(cols.get(1).unwrap_or(&"0"));
                        let texture_id = cols.get(2).unwrap_or(&"").trim().to_string();
                        let terminus_stop = if all_exit { None } else { Some(texture_id.clone()) };
                        let mut strings: Vec<String> = cols.iter().skip(3).map(|s| s.to_string()).collect();
                        if h.string_count_terminus > 0 {
                            strings.resize(h.string_count_terminus, String::new());
                        }
                        h.termini.push(Terminus { code, texture_id, terminus_stop, all_exit, strings });
                    }
                }
                "addbusstop" => {
                    let ident = r.str().to_string();
                    let strings = (0..h.string_count_busstop).map(|_| r.str().to_string()).collect();
                    h.bus_stops.push(BusStop { ident, strings });
                }
                "addbusstop_list" => {
                    for l in r.until("[end]") {
                        if l.trim().is_empty() {
                            continue;
                        }
                        let mut cols = l.split('\t');
                        let ident = cols.next().unwrap_or("").trim().to_string();
                        h.bus_stops.push(BusStop { ident, strings: cols.map(|s| s.to_string()).collect() });
                    }
                }
                "infosystem_trip" => {
                    let code = r.str().to_string();
                    let name = r.str().to_string();
                    let route = r.str().to_string();
                    let line = r.str().to_string();
                    h.info_trips.push(InfoTrip { code, name, route, line, extra: Vec::new() });
                    // every trip has a stop list, empty until one follows (THof.LoadFromFile
                    // 0x7ea142), so that the lists stay in step with the trips
                    h.info_busstop_lists.push(Vec::new());
                }
                "infosystem_busstop_list" => {
                    // the list of the trip read last (0x7ea16f: DynArrayHigh of the trips);
                    // pushed as one more list, a trip without one (the IVU data routes of
                    // some depot files) gave every later trip the stops of the one before
                    let n = r.usize();
                    let list: Vec<String> = (0..n).map(|_| r.str().to_string()).collect();
                    if let Some(last) = h.info_busstop_lists.last_mut() {
                        *last = list;
                    }
                }
                "infosystem_busstop" => h.info_busstops.push((0..3).map(|_| r.str().to_string()).collect()),
                _ => {}
            }
        }
        h
    }

    pub fn terminus_by_code(&self, code: i32) -> Option<&Terminus> {
        self.termini.iter().find(|t| t.code == code)
    }

    /// The depot file as OMSI reads it (`\r\n` line ends; the caller picks the code page):
    /// `[name]`, `[servicetrip]`, `[global_strings]`, the two string counts with `comments`'
    /// notes on the strings, then the termini, the stops, the IBIS trips with their stop lists
    /// and the IBIS stops. What [`Hof::parse`] reads of it is this `Hof` again (`path` aside).
    ///
    /// A record (`[addterminus]`, `[addbusstop]`) is read with its line ends cut off, so a
    /// text ending in spaces - a centred sign padded on the right, as the spreadsheet exports
    /// have them - keeps them only in a list (`[addterminus_list]`, `[addbusstop_list]`):
    /// termini and stops are written as a list when one of their texts needs it (and none
    /// holds a tab, which a list cannot).
    pub fn to_text(&self, comments: &Comments) -> String {
        let mut o = String::with_capacity(4096 + self.termini.len() * 160);
        let line = |o: &mut String, s: &str| {
            o.push_str(&one_line(s));
            o.push_str("\r\n");
        };
        for c in &comments.head {
            line(&mut o, &comment(c));
        }
        if !comments.head.is_empty() {
            o.push_str("\r\n");
        }
        o.push_str("[name]\r\n");
        line(&mut o, &self.name);
        o.push_str("\r\n");
        if !self.service_trip.is_empty() {
            o.push_str("[servicetrip]\r\n");
            line(&mut o, &self.service_trip);
            o.push_str("\r\n");
        }
        if !self.global_strings.is_empty() {
            o.push_str(&format!("[global_strings]\r\n{}\r\n", self.global_strings.len()));
            for s in &self.global_strings {
                line(&mut o, s);
            }
            o.push_str("\r\n");
        }
        // (a file without the count reads every column of its lists: as many as the widest)
        let nt = if self.string_count_terminus > 0 { self.string_count_terminus } else { self.termini.iter().map(|t| t.strings.len()).max().unwrap_or(0) };
        let nb = if self.string_count_busstop > 0 { self.string_count_busstop } else { self.bus_stops.iter().map(|b| b.strings.len()).max().unwrap_or(0) };
        let notes = |o: &mut String, prefix: &str, notes: &[String]| {
            let mut any = false;
            for (k, n) in notes.iter().enumerate().filter(|(_, n)| !n.trim().is_empty()) {
                o.push_str(&format!("\t{prefix}string{k}:\t{}\r\n", one_line(n.trim())));
                any = true;
            }
            if any {
                o.push_str("\r\n");
            }
        };
        o.push_str(&format!("stringcount_terminus\r\n{nt}\r\n\r\n"));
        notes(&mut o, "", &comments.terminus);
        o.push_str(&format!("stringcount_busstop\r\n{nb}\r\n\r\n"));
        // (not `string0:` alone: that is how the terminus strings' notes are told)
        notes(&mut o, "busstop ", &comments.busstop);
        let fit = |s: &[String], n: usize| -> Vec<String> {
            let mut v: Vec<String> = s.iter().take(n).map(|x| one_line(x)).collect();
            v.resize(n, String::new());
            v
        };
        let list = |texts: Vec<&String>, names: Vec<&String>| texts.iter().any(|t| t.trim_end() != t.as_str()) && !texts.iter().chain(names.iter()).any(|t| t.contains('\t'));
        if list(self.termini.iter().flat_map(|t| t.strings.iter().take(nt)).collect(), self.termini.iter().map(|t| &t.texture_id).collect()) {
            o.push_str("[addterminus_list]\r\n");
            for t in &self.termini {
                let flag = if t.all_exit { "{ALLEX}" } else { "" };
                o.push_str(&format!("{flag}\t{}\t{}", t.code, one_line(t.texture_id.trim())));
                for s in fit(&t.strings, nt) {
                    o.push('\t');
                    o.push_str(&s);
                }
                o.push_str("\r\n");
            }
            o.push_str("[end]\r\n\r\n");
        } else {
            for t in &self.termini {
                o.push_str(if t.all_exit { "[addterminus_allexit]\r\n" } else { "[addterminus]\r\n" });
                o.push_str(&format!("{}\r\n", t.code));
                line(&mut o, &t.texture_id);
                for s in fit(&t.strings, nt) {
                    line(&mut o, &s);
                }
                o.push_str("\r\n");
            }
        }
        if list(self.bus_stops.iter().flat_map(|b| b.strings.iter().take(nb)).collect(), self.bus_stops.iter().map(|b| &b.ident).collect()) {
            o.push_str("[addbusstop_list]\r\n");
            for b in &self.bus_stops {
                o.push_str(&one_line(b.ident.trim()));
                for s in fit(&b.strings, nb) {
                    o.push('\t');
                    o.push_str(&s);
                }
                o.push_str("\r\n");
            }
            o.push_str("[end]\r\n\r\n");
        } else {
            for b in &self.bus_stops {
                o.push_str("[addbusstop]\r\n");
                line(&mut o, &b.ident);
                for s in fit(&b.strings, nb) {
                    line(&mut o, &s);
                }
                o.push_str("\r\n");
            }
        }
        for (k, t) in self.info_trips.iter().enumerate() {
            o.push_str("[infosystem_trip]\r\n");
            for s in [&t.code, &t.name, &t.route, &t.line] {
                line(&mut o, s);
            }
            o.push_str("\r\n");
            if let Some(stops) = self.info_busstop_lists.get(k).filter(|l| !l.is_empty()) {
                o.push_str(&format!("[infosystem_busstop_list]\r\n{}\r\n", stops.len()));
                for s in stops {
                    line(&mut o, s);
                }
                o.push_str("\r\n");
            }
        }
        for b in &self.info_busstops {
            o.push_str("[infosystem_busstop]\r\n");
            for k in 0..3 {
                line(&mut o, b.get(k).map(String::as_str).unwrap_or(""));
            }
            o.push_str("\r\n");
        }
        o
    }
}

/// The comment lines [`Hof::to_text`] writes: at the head of the file, and the notes on what
/// each terminus string and each stop string is for (`string0: IBIS-Display`, as the stock
/// files have them - what the line editor knows the strings by, `linehof::notes`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Comments {
    pub head: Vec<String>,
    pub terminus: Vec<String>,
    pub busstop: Vec<String>,
}

/// A text as one line of a depot file: no line breaks in it.
fn one_line(s: &str) -> String {
    if s.contains(['\r', '\n']) {
        s.replace("\r\n", " ").replace(['\r', '\n'], " ")
    } else {
        s.to_string()
    }
}

/// A comment line: indented, and so never read as a keyword or a string count.
fn comment(s: &str) -> String {
    let s = one_line(s.trim());
    if s.is_empty() {
        String::new()
    } else {
        format!("\t{s}")
    }
}

/// The folder the depot files of a bus in `bus_dir` are read from: its own when it has any
/// there (in any content root), else its pack's - the folder right under `Vehicles` - when
/// that has some, as the bus list and the launcher's depot tiles have them.
pub fn depot_dir(bus_dir: &Path) -> PathBuf {
    if has_own_depot_files(bus_dir) {
        return bus_dir.to_path_buf();
    }
    let comps: Vec<std::path::Component> = bus_dir.components().collect();
    if let Some(i) = comps.iter().rposition(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case("vehicles")) {
        if comps.len() > i + 2 {
            let pack: PathBuf = comps[..i + 2].iter().collect();
            if has_own_depot_files(&pack) {
                return pack;
            }
        }
    }
    bus_dir.to_path_buf()
}

/// Whether `dir` has depot files of its own, in any content root (the shared `HOFs/` folder,
/// which `depot_files` adds for every vehicle, aside: it would make every bus seem to have
/// some, and a pack's never be read).
fn has_own_depot_files(dir: &Path) -> bool {
    omsi_cfg::mirrored_dirs(dir).iter().any(|d| omsi_cfg::vfs::read_dir_paths(d).iter().any(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("hof"))))
}

/// The `.hof` files available to a vehicle. Files next to the vehicle come first, merged
/// over all content roots (a mod's depot beside the installation's; archives read in place
/// too), followed by the shared top-level `HOFs/` folder. A vehicle-local file hides a
/// shared file of the same name; within each group files are sorted by name.
pub fn depot_files(dir: &Path) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut local: Vec<PathBuf> = Vec::new();
    for d in omsi_cfg::mirrored_dirs(dir) {
        for p in omsi_cfg::vfs::read_dir_paths(&d) {
            if !p.extension().map(|e| e.eq_ignore_ascii_case("hof")).unwrap_or(false) {
                continue;
            }
            if seen.insert(p.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase()) {
                local.push(p);
            }
        }
    }
    local.sort_by_key(|f| f.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase());

    let mut shared: Vec<PathBuf> = omsi_cfg::read_dir_merged("HOFs")
        .into_iter()
        .filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("hof")).unwrap_or(false))
        .filter(|p| seen.insert(p.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase()))
        .collect();
    shared.sort_by_key(|f| f.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase());

    local.extend(shared);
    local
}

/// The depot file of `dir` called `name`: by its file name (without `.hof`) or its `[name]`.
pub fn depot_in(dir: &Path, name: &str) -> Option<Hof> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let files = depot_files(dir);
    if let Some(f) = files.iter().find(|f| f.file_stem().map(|s| s.to_string_lossy().trim().eq_ignore_ascii_case(name)).unwrap_or(false)) {
        if let Ok(h) = Hof::load(f) {
            return Some(h);
        }
    }
    files
        .iter()
        .filter(|f| Hof::read_name(f).is_some_and(|n| n.trim().eq_ignore_ascii_case(name)))
        .find_map(|f| Hof::load(f).ok())
}

/// The words of a depot or map name that tell one place from another: four letters or
/// more, not a year or a number, not a word every depot file has ("Linie 20", "Hof").
fn place_words(s: &str) -> Vec<String> {
    const COMMON: [&str; 14] = ["linie", "line", "lines", "depot", "omsi", "maps", "version", "final", "neue", "update", "addon", "fixed", "standard", "default"];
    s.split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| w.chars().count() >= 4 && !w.chars().all(|c| c.is_ascii_digit()) && !COMMON.contains(&w.as_str()))
        .collect()
}

/// Of `names` (the depot files a bus has), the one that belongs to the place `hints` name
/// (the map's depot names, its title, its folder): the one sharing the most of their
/// words, the longer words counting more; the first of equals. None when none shares one.
///
/// OMSI asks the driver which of the bus's depot files to use; taking the first of them
/// when none is called exactly as the map wants put a bus on Hamburg's Linie 20 with the
/// Grundorf depot of its folder - no line and no destination its IBIS knew (#896).
pub fn closest_name(names: &[&str], hints: &[&str]) -> Option<usize> {
    let wanted: Vec<String> = hints.iter().flat_map(|h| place_words(h)).collect();
    let mut best: Option<(usize, usize)> = None;
    for (i, n) in names.iter().enumerate() {
        let mut words = place_words(n);
        words.dedup();
        let score: usize = words.iter().filter(|w| wanted.contains(w)).map(|w| w.chars().count()).sum();
        if score > 0 && best.is_none_or(|(_, b)| score > b) {
            best = Some((i, score));
        }
    }
    best.map(|(i, _)| i)
}

/// A depot file of the player's own (openOMSI's depot editor's, `oo_…`), given to a bus for the
/// drives that name it: never taken for another one in its place.
pub fn is_players(path: &Path) -> bool {
    path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.get(..3)).is_some_and(|p| p.eq_ignore_ascii_case("oo_"))
}

/// The depot file of `dir` that belongs to the place `hints` name (see [`closest_name`]),
/// by its file name or its `[name]` (one of the player's own only by its name: `is_players`).
pub fn depot_like(dir: &Path, hints: &[&str]) -> Option<Hof> {
    let files: Vec<PathBuf> = depot_files(dir).into_iter().filter(|f| !is_players(f)).collect();
    let names: Vec<String> = files
        .iter()
        .map(|f| {
            let stem = f.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            match Hof::read_name(f) {
                Some(n) => format!("{stem} {n}"),
                None => stem,
            }
        })
        .collect();
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    closest_name(&refs, hints).and_then(|i| Hof::load(&files[i]).ok())
}

/// The depot file called `name` in the shared `HOFs/` folder or any vehicle folder of any content root (`Vehicles/*/`).
///
/// A depot file belongs to a map, not to a bus model: it lists the map's termini, stops and
/// IBIS codes. A mod bus brings only the depot of the map it was made on (the O530 Citaro
/// pack has Grundorf.hof alone), and on another map every code of the timetable was then
/// unknown to its IBIS - no line and no destination on any display. OMSI players copy the
/// map's .hof into such a folder; this finds the copy that is already installed with
/// another bus.
pub fn depot_anywhere(name: &str) -> Option<Hof> {
    // (asked for every type of AI bus without the map's depot: the file found is kept for
    // the session, and a big installation's thousands of folders are gone through once)
    type Found = std::collections::HashMap<String, Option<PathBuf>>;
    static FOUND: std::sync::OnceLock<std::sync::Mutex<(u64, Found)>> = std::sync::OnceLock::new();
    let found = FOUND.get_or_init(Default::default);
    let key = name.trim().to_ascii_lowercase();
    let generation = omsi_cfg::content_generation();
    let known = {
        let mut f = found.lock().unwrap_or_else(|e| e.into_inner());
        if f.0 != generation {
            *f = (generation, Found::new());
        }
        f.1.get(&key).cloned()
    };
    if let Some(path) = known {
        return path.and_then(|p| Hof::load(&p).ok());
    }
    // every vehicle folder once over all roots (depot_in looks at each root's copy)
    let mut dirs: Vec<PathBuf> = omsi_cfg::read_dir_merged("Vehicles").into_iter().filter(|d| omsi_cfg::vfs::is_dir(d)).collect();
    dirs.sort_by_key(|d| d.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase());
    let h = dirs.iter().find_map(|d| depot_in(d, name));
    found.lock().unwrap_or_else(|e| e.into_inner()).1.insert(key, h.as_ref().map(|h| h.path.clone()));
    h
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_depot_file_carries_a_line_by_its_trip_codes_or_names() {
        let trip = |code: &str, name: &str| InfoTrip { code: code.into(), name: name.into(), ..Default::default() };
        let global = Hof { info_trips: vec![trip("1101", "11 -> PETIT VILTAIN"), trip("1401", "14 -> MOULON")], ..Default::default() };
        let inter = Hof { info_trips: vec![trip("001", "91.06A -> MASSY GARE"), trip("005", "91.06C -> MASSY GARE")], ..Default::default() };
        assert!(global.has_line("11-11s") && global.has_line("14"));
        assert!(!global.has_line("1") && !global.has_line("91.06C") && !global.has_line(""));
        assert!(inter.has_line("91.06C") && inter.has_line("91.06") && !inter.has_line("11-11s"));
    }

    use super::*;

    /// #896: the bus's own depot of the map's place, not the first of its folder.
    #[test]
    fn closest_depot_name_is_the_maps_place() {
        let names = ["Grundorf", "Hamburg Linie 20", "Spandau 2019"];
        assert_eq!(closest_name(&names, &["Hamburg_Linie_20", "Linie 20"]), Some(1));
        assert_eq!(closest_name(&names, &["Spandau 1986"]), Some(2));
        assert_eq!(closest_name(&names, &["Berlin-Spandau"]), Some(2));
        assert_eq!(closest_name(&names, &["Thüringer Wald"]), None);
        assert_eq!(closest_name(&["Berlin X10", "Spandau"], &["Berlin-Spandau"]), Some(1));
        assert_eq!(closest_name(&["Linie 20"], &["Linie 7"]), None);
    }

    #[test]
    fn shared_depots_are_available_but_vehicle_copy_wins() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("openomsi-shared-hof-{stamp}"));
        let bus = root.join("Vehicles/TestBus");
        let shared = root.join("HOFs");
        std::fs::create_dir_all(&bus).unwrap();
        std::fs::create_dir_all(&shared).unwrap();

        let local_name = format!("local-{stamp}.hof");
        let shared_name = format!("shared-{stamp}.hof");
        let same_name = format!("same-{stamp}.hof");
        std::fs::write(bus.join(&local_name), "[name]\nLocal\n").unwrap();
        std::fs::write(shared.join(&shared_name), "[name]\nShared\n").unwrap();
        std::fs::write(bus.join(&same_name), "[name]\nLocal duplicate\n").unwrap();
        std::fs::write(shared.join(&same_name), "[name]\nShared duplicate\n").unwrap();

        omsi_cfg::add_content_root(root.clone());
        let files = depot_files(&bus);
        let named = |p: &Path, name: &str| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n == name);
        let local_i = files.iter().position(|p| named(p, &local_name)).unwrap();
        let shared_i = files.iter().position(|p| named(p, &shared_name)).unwrap();
        let duplicate = files.iter().find(|p| named(p, &same_name)).unwrap();
        assert!(local_i < shared_i);
        assert_eq!(duplicate, &bus.join(&same_name));
        assert_eq!(depot_in(&bus, "Shared").map(|h| h.name), Some("Shared".into()));

        omsi_cfg::remove_content_root(&root);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn terminus_list_columns() {
        let text = "stringcount_terminus\r\n3\r\n\r\n[addterminus_list]\r\n{ALLEX}\t13\tBetriebsfahrt\tBETRIEBSFAHRT\t\tBETRIEBSFAHRT\t\t\r\n\t282\tU Ruhleben\tRUHLEBEN\tU-BAHNHOF\tRUHLEBEN  \t\t\t\r\n[end]\r\n";
        let h = Hof::parse(&CfgFile::from_str("test.hof", text));
        assert_eq!(h.termini.len(), 2);
        assert_eq!((h.termini[0].code, h.termini[0].texture_id.as_str(), h.termini[0].all_exit), (13, "Betriebsfahrt", true));
        assert_eq!(h.termini[0].strings, vec!["BETRIEBSFAHRT", "", "BETRIEBSFAHRT"]);
        assert_eq!((h.termini[1].code, h.termini[1].texture_id.as_str(), h.termini[1].all_exit), (282, "U Ruhleben", false));
        assert_eq!(h.termini[1].terminus_stop.as_deref(), Some("U Ruhleben"));
        assert_eq!(h.termini[1].strings, vec!["RUHLEBEN", "U-BAHNHOF", "RUHLEBEN  "]);
        assert_eq!(h.terminus_by_code(282).map(|t| t.texture_id.as_str()), Some("U Ruhleben"));
    }

    /// The head of a stock depot file as OMSI ships it: comments, notes on the strings, the
    /// spreadsheet's trailing tabs, an "everybody out" terminus, umlauts, an IBIS trip with
    /// its stops and one without (Windows-1252 when written).
    const STOCK: &str = "\t########################\r\n\t\tHOF-Datei\r\n\r\n\tEnthält die diversen Informationen zum Einsatz für diesen Hof.\r\n\r\n\
        [name]\r\nGrundorf\r\n\r\n[servicetrip]\r\nBetriebsfahrt\r\n\r\n[global_strings]\t\r\n4\t\r\nGrundorf\t\r\n\r\nGrundorf\r\n4\r\n\r\n\
        stringcount_terminus\r\n8\r\n\r\n\tstring0:\tIBIS-Display & Rollband-Textur\r\n\tstring5:\tIBIS2-Display (Klarname in Groß-/Kleinschreibung), max 20 Zeichen\r\n\r\n\
        stringcount_busstop\r\n4\r\n\r\n\
        [addterminus_allexit]\r\n13\r\nBetriebsfahrt\r\nBETRIEBSFAHRT\r\n                \r\n BETRIEBSFAHRT\r\n BETRIEBSFAHRT\r\nBetriebsfahrt.tga\r\nBetriebsfahrt\r\n\r\n\r\n................\r\n\r\n\
        [addterminus]\r\n103\r\nThalesstr\r\nTHALESSTRASSE\r\n THALESSTRASSE\r\n- FERNSEHTURM -\r\n THALESSTRASSE\r\nGru_Thalesstr.tga\r\nThalesstraße\r\n\r\n\r\n................\r\n\r\n\
        [addbusstop]\r\nBauernhof\r\nNORDS. BAUERNHOF\r\nNordspitze\r\nBauernhof\r\nNordsp. Bauernhof\r\n....................\r\n\r\n\
        [addbusstop]\r\nGaussdorf\r\nGAUSSDORF\r\nGaussdorf\r\n\r\nGaussdorf\r\n\r\n\
        [infosystem_trip]\r\n7601\r\nBAUERNHOF-KRANKENHAUS\r\n105\r\nTML\r\n\r\n................\r\n\r\n\
        [infosystem_busstop_list]\r\n2\r\nBauernhof\r\nGaussdorf\r\n\r\n\
        [infosystem_trip]\r\n455900\r\nIVU\r\n81\r\n455\r\n\r\n\
        [infosystem_busstop]\r\nGAUSSDORF\r\nGaussdorf\r\nMitte\r\n";

    fn same(a: &Hof, b: &Hof) {
        let (mut a, mut b) = (a.clone(), b.clone());
        a.path = PathBuf::new();
        b.path = PathBuf::new();
        assert_eq!(a, b);
    }

    #[test]
    fn a_depot_file_written_reads_back_as_it_was() {
        let h = Hof::parse(&CfgFile::from_str("Grundorf.hof", STOCK));
        assert_eq!((h.termini.len(), h.bus_stops.len(), h.info_trips.len(), h.info_busstops.len()), (2, 2, 2, 1));
        assert_eq!(h.global_strings, vec!["Grundorf", "", "Grundorf", "4"]);
        let notes = Comments { head: vec!["openOMSI depot file".into()], terminus: vec!["IBIS-Display".into(), String::new(), "Front, 2. Zeile".into()], busstop: vec!["IBIS".into()] };
        let text = h.to_text(&notes);
        let back = Hof::parse(&CfgFile::from_str("x.hof", &text));
        same(&h, &back);
        assert!(text.contains("[addterminus_allexit]\r\n13\r\nBetriebsfahrt\r\n"), "{text}");
        assert!(text.contains("\tstring2:\tFront, 2. Zeile\r\n") && text.contains("\tbusstop string0:\tIBIS\r\n"));
        // written again: the same text
        assert_eq!(back.to_text(&notes), text);
        // in Windows-1252, as OMSI reads it
        let page = omsi_cfg::codepage::CodePage::Windows1252.encoding();
        let (bytes, _, lossy) = page.encode(&text);
        assert!(!lossy);
        assert!(bytes.windows(2).any(|w| w == [b'a', 0xDF]), "Thalesstraße in one byte");
        same(&h, &Hof::parse(&CfgFile::from_bytes("x.hof", &bytes)));
    }

    #[test]
    fn texts_padded_on_the_right_are_written_as_a_list() {
        let text = "stringcount_terminus\r\n3\r\nstringcount_busstop\r\n2\r\n\r\n[addterminus_list]\r\n{ALLEX}\t13\tBetriebsfahrt\tBETRIEBSFAHRT\t\tBETRIEBSFAHRT\t\t\r\n\t282\tU Ruhleben\tRUHLEBEN\tU-BAHNHOF\tRUHLEBEN  \t\t\t\r\n[end]\r\n\
            [addbusstop_list]\r\nRuhleben\tRUHLEBEN  \tU Ruhleben\r\n[end]\r\n";
        let h = Hof::parse(&CfgFile::from_str("Spandau.hof", text));
        let out = h.to_text(&Comments::default());
        assert!(out.contains("[addterminus_list]\r\n{ALLEX}\t13\tBetriebsfahrt\t") && out.contains("[addbusstop_list]"), "{out}");
        let back = Hof::parse(&CfgFile::from_str("x.hof", &out));
        same(&h, &back);
        assert_eq!(back.termini[1].strings[2], "RUHLEBEN  ");
        // nothing padded: records
        let mut plain = h.clone();
        plain.termini[1].strings[2] = "RUHLEBEN".into();
        plain.bus_stops[0].strings[0] = "RUHLEBEN".into();
        let out = plain.to_text(&Comments::default());
        assert!(out.contains("[addterminus]\r\n282\r\nU Ruhleben\r\n") && !out.contains("_list]"), "{out}");
        same(&plain, &Hof::parse(&CfgFile::from_str("x.hof", &out)));
        // a line break typed into a text is no second line
        plain.termini[1].strings[0] = "RUH\r\nLEBEN".into();
        let back = Hof::parse(&CfgFile::from_str("x.hof", &plain.to_text(&Comments::default())));
        assert_eq!(back.termini[1].strings[0], "RUH LEBEN");
        assert_eq!(back.termini.len(), 2);
    }

    #[test]
    fn a_pack_buses_depot_files_are_the_packs() {
        let base = std::env::temp_dir().join(format!("omsi_hof_dir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let put = |rel: &str| {
            let p = base.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "[name]\r\nX\r\n").unwrap();
        };
        put("Vehicles/Pack/Grundorf.hof");
        put("Vehicles/Pack/Solo/solo.bus");
        put("Vehicles/Pack/Own/Spandau.hof");
        put("Vehicles/Lone/Sub/x.bus");
        assert_eq!(depot_dir(&base.join("Vehicles/Pack/Solo")), base.join("Vehicles/Pack"));
        assert_eq!(depot_dir(&base.join("Vehicles/Pack/Own")), base.join("Vehicles/Pack/Own"));
        assert_eq!(depot_dir(&base.join("Vehicles/Pack")), base.join("Vehicles/Pack"));
        assert_eq!(depot_dir(&base.join("Vehicles/Lone/Sub")), base.join("Vehicles/Lone/Sub"), "no depot file anywhere: its own folder");
        // the player's own given to a bus is taken by its name only, never for the place
        let mine = base.join("Vehicles/Lone/Sub/oo_Grundorf_mine.hof");
        std::fs::write(&mine, "[name]\r\nGrundorf - mine\r\n").unwrap();
        assert!(is_players(&mine) && !is_players(&base.join("Vehicles/Pack/Grundorf.hof")));
        assert!(depot_like(&base.join("Vehicles/Lone/Sub"), &["Grundorf"]).is_none());
        assert!(depot_in(&base.join("Vehicles/Lone/Sub"), "oo_Grundorf_mine").is_some());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Every depot file of an installation (`OMSI_HOF_DIR`, e.g. OMSI 2's `Vehicles`) written
    /// and read back: what is read is what was there. Run by hand:
    /// `OMSI_HOF_DIR=... cargo test -p omsi-vehicle -- --ignored every_installed`.
    #[test]
    #[ignore]
    fn every_installed_depot_file_reads_back() {
        let Some(dir) = std::env::var_os("OMSI_HOF_DIR") else { return };
        fn walk(d: &Path, out: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("hof")) {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(Path::new(&dir), &mut files);
        assert!(!files.is_empty());
        for f in &files {
            let bytes = std::fs::read(f).unwrap();
            let h = Hof::parse(&CfgFile::from_bytes(f, &bytes));
            let text = h.to_text(&Comments::default());
            let back = Hof::parse(&CfgFile::from_str(f, &text));
            let mut want = h.clone();
            // (a file without a count is written with one: its widest list)
            if want.string_count_terminus == 0 {
                want.string_count_terminus = back.string_count_terminus;
                for t in &mut want.termini {
                    t.strings.resize(back.string_count_terminus, String::new());
                }
            }
            if want.string_count_busstop == 0 {
                want.string_count_busstop = back.string_count_busstop;
            }
            for b in &mut want.bus_stops {
                b.strings.resize(back.string_count_busstop, String::new());
            }
            want.info_busstop_lists.truncate(want.info_trips.len());
            same(&want, &back);
        }
        eprintln!("{} depot files read back", files.len());
    }

    #[test]
    fn legacy_hof_destination_name_falls_back_to_ident() {
        let text = "stringcount_terminus\n6\n[addterminus]\n71910\n71 Eden Tunnel\n\n\n\n\nLegacyRoute\\71Y_1.bmp\n71Y\n";
        let h = Hof::parse(&CfgFile::from_str("legacy.hof", text));
        let t = h.terminus_by_code(71910).unwrap();
        assert_eq!(t.strings[0], "");
        assert_eq!(t.strings[4], "LegacyRoute\\71Y_1.bmp");
        assert_eq!(t.strings[5], "71Y");
        assert_eq!(t.display_name(), "71 Eden Tunnel");
        assert_eq!(t.menu_name(), "71 Eden Tunnel");
    }

    #[test]
    fn normal_hof_menu_name_keeps_display_text() {
        let t = Terminus { texture_id: "910".into(), strings: vec!["AEC".into()], ..Default::default() };
        assert_eq!(t.menu_name(), "AEC");
    }

    #[test]
    fn route_label_menu_name_uses_the_ident() {
        let t = Terminus { texture_id: "66: South Valley Railway Station Circular".into(), strings: vec!["S.VALLEY STN CIR".into()], ..Default::default() };
        assert_eq!(t.menu_name(), "66: South Valley Railway Station Circular");
    }

    /// #667: a trip without a stop list (an IVU data route) keeps the lists of the trips
    /// after it on their own trips.
    #[test]
    fn stop_lists_belong_to_the_trip_before_them() {
        let text = "[infosystem_trip]\r\n45581\r\nZOB-HOHENECK\r\n81\r\n455\r\n\r\n\
            [infosystem_busstop_list]\r\n2\r\nZOB\r\nHoheneck\r\n\r\n\
            [infosystem_trip]\r\n455900\r\nIVU\r\n81\r\n455\r\n\r\n\
            [infosystem_trip]\r\n45503\r\nHBF-BERGERFUERTH\r\n3\r\n455\r\n\r\n\
            [infosystem_busstop_list]\r\n3\r\nHauptbahnhof\r\nMarkt\r\nBergerfuerth\r\n";
        let h = Hof::parse(&CfgFile::from_str("test.hof", text));
        assert_eq!(h.info_trips.len(), 3);
        assert_eq!(h.info_busstop_lists.len(), 3);
        assert_eq!(h.info_busstop_lists[0], vec!["ZOB", "Hoheneck"]);
        assert!(h.info_busstop_lists[1].is_empty());
        assert_eq!(h.info_busstop_lists[2], vec!["Hauptbahnhof", "Markt", "Bergerfuerth"]);
    }
}
