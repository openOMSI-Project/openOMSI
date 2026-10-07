//! Depot files a bus does not carry. A `.hof` belongs to a map - its termini, its stops, its
//! IBIS codes - but OMSI reads it from the bus's folder, and a bus that came without the map's
//! knows no destination there: its IBIS takes none of the codes and the displays stay blank.
//! Players copy the map's file into the bus's folder by hand; Omsi-Hub does it for them
//! (`hofTool.ts`), and so does this: it finds the map's file where it is installed - beside
//! another bus, or in the map's own folder - and puts a copy beside the bus.
//!
//! The copy goes into openOMSI's content folder, in the bus's folder there, and not into the
//! OMSI 2 folder: the game reads every content root's copy of a vehicle folder together, the
//! content folder's first (`omsi_vehicle::hof::depot_files`, `omsi_cfg::mirrored_dirs`), so
//! the bus has the file as if it lay in its own folder while nothing of the player's OMSI 2
//! is touched. It is never written over a file, and what was added is noted
//! (`depots-added.txt`) so that it can be told apart from the bus's own.

use std::path::{Path, PathBuf};

/// A copy of a depot file found on this computer.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub path: PathBuf,
    /// Its file name, which the copy keeps.
    pub file: String,
    /// The vehicle folder it lies in, or the map's folder.
    pub folder: String,
    /// It lies in the map's folder (it came with the map) rather than beside a bus.
    pub from_map: bool,
    pub size: u64,
}

/// Why a depot file cannot be added beside a bus.
#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    /// The bus has a depot file answering to that name already (its file name).
    Has(String),
    /// A file of that name lies beside the bus already - another depot file under the same
    /// name, someone's own: it is never written over (nor hidden by a copy in front of it).
    Taken(String),
    /// No copy of it on this computer.
    NoSource,
    /// No content folder to put it in.
    NoFolder,
    /// The place lies in the OMSI 2 folder's vehicles, which openOMSI does not write to.
    Original,
    /// The copy found cannot be read, or is no depot file.
    Unreadable(String),
    /// Writing it failed.
    Failed(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Has(file) => write!(f, "the bus has it already ({file})"),
            Refusal::Taken(file) => write!(f, "a file called {file} lies beside the bus already"),
            Refusal::NoSource => write!(f, "no copy of it on this computer"),
            Refusal::NoFolder => write!(f, "no content folder to put it in"),
            Refusal::Original => write!(f, "that would be inside the OMSI 2 folder's vehicles"),
            Refusal::Unreadable(why) => write!(f, "the copy found cannot be read: {why}"),
            Refusal::Failed(why) => write!(f, "writing it failed: {why}"),
        }
    }
}

fn is_hof(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("hof"))
}

fn file_name(p: &Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

/// Whether a depot file is called `name` as the game asks for one (`--hof`, an
/// `[aigroup_depot]`): by its file name without `.hof`, or by its `[name]`
/// (`omsi_vehicle::hof::depot_in`).
pub fn answers_to(path: &Path, name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }
    path.file_stem().is_some_and(|s| s.to_string_lossy().trim().eq_ignore_ascii_case(name)) || omsi_vehicle::Hof::read_name(path).is_some_and(|n| n.trim().eq_ignore_ascii_case(name))
}

/// The depot files lying directly in `dir` (a folder or a mounted archive's).
fn hofs_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = omsi_cfg::vfs::read_dir_paths(dir).into_iter().filter(|p| is_hof(p)).collect();
    v.sort();
    v
}

fn size_of(p: &Path) -> u64 {
    omsi_cfg::vfs::read(p).map(|b| b.len() as u64).unwrap_or(0)
}

/// Every copy of the depot file called `name`: beside the buses of `folders` (each vehicle
/// folder with its copies in every content root, as the lists read them; a file of a
/// higher-priority root hides the one of the same name lower down) and in the map's folder
/// `map_dir` or a folder in it (some maps bring their depot file there).
pub fn sources_in(name: &str, folders: &[(String, Vec<PathBuf>)], map_dir: Option<&Path>) -> Vec<Source> {
    let mut out = Vec::new();
    if name.trim().is_empty() {
        return out;
    }
    for (folder, copies) in folders {
        let mut seen = std::collections::HashSet::new();
        for d in copies {
            for p in hofs_in(d) {
                if seen.insert(file_name(&p).to_ascii_lowercase()) && answers_to(&p, name) {
                    out.push(Source { file: file_name(&p), folder: folder.clone(), from_map: false, size: size_of(&p), path: p });
                }
            }
        }
    }
    if let Some(m) = map_dir {
        let folder = file_name(m);
        let mut dirs = vec![m.to_path_buf()];
        if let Some(list) = omsi_cfg::vfs::list_dir(m) {
            let mut subs: Vec<PathBuf> = list.into_iter().filter(|(_, d)| *d).map(|(n, _)| m.join(n)).collect();
            subs.sort();
            dirs.extend(subs);
        }
        for d in dirs {
            for p in hofs_in(&d).into_iter().filter(|p| answers_to(p, name)) {
                out.push(Source { file: file_name(&p), folder: folder.clone(), from_map: true, size: size_of(&p), path: p });
            }
        }
    }
    out
}

/// Of the copies found, the one to give a bus: the file name most buses carry it under, and of
/// those the version most of them have (the same size) - what players pass on to each other is
/// the map's file as it is used; the map's own copy when no bus has it. On equal counts the
/// first by folder. None when there is none.
pub fn best(sources: &[Source]) -> Option<usize> {
    let count = |f: &dyn Fn(&Source) -> bool| sources.iter().filter(|s| !s.from_map && f(s)).count();
    let pool: Vec<usize> = if sources.iter().any(|s| !s.from_map) { (0..sources.len()).filter(|i| !sources[*i].from_map).collect() } else { (0..sources.len()).collect() };
    pool.into_iter().max_by(|&a, &b| {
        let (x, y) = (&sources[a], &sources[b]);
        let by_name = |s: &Source| count(&|o: &Source| o.file.eq_ignore_ascii_case(&s.file));
        let by_size = |s: &Source| count(&|o: &Source| o.file.eq_ignore_ascii_case(&s.file) && o.size == s.size);
        by_name(x).cmp(&by_name(y)).then(by_size(x).cmp(&by_size(y))).then_with(|| y.folder.to_lowercase().cmp(&x.folder.to_lowercase())).then_with(|| y.file.to_lowercase().cmp(&x.file.to_lowercase()))
    })
}

/// The folder (relative to a content root, '/') a bus's depot files are read from, and where
/// one added for it goes: the bus file's own when it has any there, else its pack's when that
/// has some (the bus list's rule: `read_vehicle_folder`), else its own.
pub fn depot_folder(bus_file: &str, own_has: bool, pack_has: bool) -> String {
    let f = bus_file.replace('\\', "/");
    let own = f.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
    let pack = f.split('/').take(2).collect::<Vec<_>>().join("/");
    if !own_has && pack_has && !pack.is_empty() {
        pack
    } else {
        own
    }
}

/// Whether a copy called `file`, answering to `name`, can be put beside a bus whose files
/// there are `existing` (each file's name and its `[name]`; every content root's copy of the
/// folder): not when one of them answers to the name already, and never over (or in front of)
/// a file of the same name.
pub fn check(file: &str, name: &str, existing: &[(String, String)]) -> Result<(), Refusal> {
    let name = name.trim();
    if let Some((f, _)) = existing.iter().find(|(f, n)| !name.is_empty() && (Path::new(f).file_stem().is_some_and(|s| s.to_string_lossy().trim().eq_ignore_ascii_case(name)) && is_hof(Path::new(f)) || n.trim().eq_ignore_ascii_case(name))) {
        return Err(Refusal::Has(f.clone()));
    }
    if let Some((f, _)) = existing.iter().find(|(f, _)| f.eq_ignore_ascii_case(file)) {
        return Err(Refusal::Taken(f.clone()));
    }
    Ok(())
}

/// Whether `p` lies in `dir` (names compared without regard to case, as Windows does).
pub(crate) fn lies_in(p: &Path, dir: &Path) -> bool {
    let low = |x: &Path| x.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect::<Vec<_>>();
    let (p, d) = (low(p), low(dir));
    p.len() >= d.len() && p[..d.len()] == d[..]
}

/// Put a copy of the depot file `source` (answering to `name`) into `target_dir` under its own
/// file name. `beside` are every content root's copies of the bus's folder - a file of that
/// name in any of them is the bus's, and a copy in front of it would hide it - and `original`
/// the OMSI 2 folder, whose vehicles are never written to. The file is written under a name
/// the game does not read as a depot file and given its own name when it is whole: a copy cut
/// off half-way is no half depot file in the bus's list.
pub fn copy_into(source: &Path, target_dir: &Path, beside: &[PathBuf], name: &str, original: Option<&Path>) -> Result<PathBuf, Refusal> {
    let file = file_name(source);
    if !is_hof(source) {
        return Err(Refusal::Unreadable(format!("{file} is no .hof file")));
    }
    if original.is_some_and(|o| lies_in(target_dir, &o.join("Vehicles"))) {
        return Err(Refusal::Original);
    }
    let mut existing: Vec<(String, String)> = Vec::new();
    for d in beside.iter().chain(std::iter::once(&target_dir.to_path_buf())) {
        for (n, is_dir) in omsi_cfg::vfs::list_dir(d).unwrap_or_default() {
            let p = d.join(&n);
            if !is_dir {
                let inner = if is_hof(&p) { omsi_vehicle::Hof::read_name(&p).unwrap_or_default() } else { String::new() };
                existing.push((n.to_string_lossy().into_owned(), inner));
            }
        }
    }
    check(&file, name, &existing)?;
    let bytes = omsi_cfg::vfs::read(source).map_err(|e| Refusal::Unreadable(e.to_string()))?;
    let h = omsi_vehicle::Hof::parse(&omsi_cfg::CfgFile::from_bytes(source, &bytes));
    if h.termini.is_empty() && h.name.trim().is_empty() {
        return Err(Refusal::Unreadable(format!("{file} has no [name] and no termini")));
    }
    std::fs::create_dir_all(target_dir).map_err(|e| Refusal::Failed(e.to_string()))?;
    let target = target_dir.join(&file);
    let part = target_dir.join(format!("{file}.openomsi-part"));
    if let Err(e) = std::fs::write(&part, &bytes) {
        let _ = std::fs::remove_file(&part);
        return Err(Refusal::Failed(e.to_string()));
    }
    // (looked at once more right before: a file that came meanwhile is not written over -
    // `rename` would replace it)
    if target.exists() {
        let _ = std::fs::remove_file(&part);
        return Err(Refusal::Taken(file));
    }
    if let Err(e) = std::fs::rename(&part, &target) {
        let _ = std::fs::remove_file(&part);
        return Err(Refusal::Failed(e.to_string()));
    }
    // the lists and the depot files' names are read again: the bus has one more
    omsi_cfg::content_changed();
    Ok(target)
}

/// What openOMSI added, one `<added>\t<copied from>` a line.
fn record_path() -> PathBuf {
    super::data_dir().join("depots-added.txt")
}

/// Note that openOMSI added `added` (a copy of `from`) beside a bus, once.
pub(crate) fn record(added: &Path, from: &Path) {
    use std::io::Write;
    let known = std::fs::read_to_string(record_path()).unwrap_or_default();
    let line = added.display().to_string();
    if known.lines().any(|l| l.split('\t').next().is_some_and(|a| a.trim().eq_ignore_ascii_case(&line))) {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(record_path()) {
        let _ = writeln!(f, "{}\t{}", added.display(), from.display());
    }
}

/// The depot files openOMSI added beside buses (that are still there).
pub fn added() -> Vec<PathBuf> {
    std::fs::read_to_string(record_path()).unwrap_or_default().lines().filter_map(|l| l.split('\t').next()).map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).filter(|p| p.is_file()).collect()
}

/// Every copy of the depot file called `name` on this computer: beside the installed buses
/// and in the map `map` (`maps/<folder>/global.cfg`).
pub fn find_sources(name: &str, map: &str) -> Vec<Source> {
    let map_dir = super::resolve_content(map).ok().and_then(|p| p.parent().map(Path::to_path_buf));
    sources_in(name, &super::merged_folders("Vehicles"), map_dir.as_deref())
}

/// The content folder's copy of a vehicle folder (`Vehicles/<pack>/...`, relative), and
/// every content root's copies of it.
pub(crate) fn copies_of(rel: &str) -> Vec<PathBuf> {
    let Ok(root) = super::root() else { return Vec::new() };
    let dir = omsi_cfg::resolve_path(&root, rel);
    let mut v = omsi_cfg::mirrored_dirs(&dir);
    for b in super::bases() {
        let p = omsi_cfg::resolve_path(&b, rel);
        if omsi_cfg::vfs::is_dir(&p) && !v.contains(&p) {
            v.push(p);
        }
    }
    v.retain(|d| omsi_cfg::vfs::is_dir(d));
    v
}

/// Where a depot file added for the bus `bus_file` (`Vehicles/<pack>/x.bus`, as the lists
/// name it) would go: the content folder's copy of the folder its depot files are read from.
pub fn target_for(bus_file: &str) -> Option<PathBuf> {
    let content = super::content_dir()?;
    let f = bus_file.replace('\\', "/");
    let own = f.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
    let pack = f.split('/').take(2).collect::<Vec<_>>().join("/");
    let has = |rel: &str| copies_of(rel).iter().any(|d| !hofs_in(d).is_empty());
    let own_has = has(&own);
    let rel = depot_folder(&f, own_has, !own_has && has(&pack));
    Some(content.join(rel))
}

/// Give the bus `bus_file` a copy of the depot file `source`, which answers to `name`: in
/// openOMSI's content folder, in the folder the bus's depot files are read from. Returns where
/// it lies now.
pub fn add_for_bus(bus_file: &str, name: &str, source: &Path) -> Result<PathBuf, Refusal> {
    let content = super::content_dir().ok_or(Refusal::NoFolder)?;
    let target_dir = target_for(bus_file).ok_or(Refusal::NoFolder)?;
    let rel = target_dir.strip_prefix(&content).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
    let beside = copies_of(&rel);
    let original = super::root().ok();
    let r = copy_into(source, &target_dir, &beside, name, original.as_deref());
    match &r {
        Ok(p) => {
            super::log_line(&format!("depot: {} added beside {bus_file} (a copy of {})", p.display(), source.display()));
            record(p, source);
        }
        Err(e) => super::log_line(&format!("depot: {} not added beside {bus_file}: {e}", source.display())),
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own under the system's temporary folder, gone at the end.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Scratch {
            let d = std::env::temp_dir().join(format!("openomsi-depot-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Scratch(d)
        }
        fn file(&self, rel: &str, text: &str) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, text).unwrap();
            p
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn hof(name: &str, termini: &[&str]) -> String {
        let mut t = format!("[name]\r\n{name}\r\n\r\nstringcount_terminus\r\n2\r\n\r\n");
        for (k, x) in termini.iter().enumerate() {
            t.push_str(&format!("[addterminus]\r\n{}\r\n{x}\r\n{x}\r\n{x}\r\n\r\n", k + 1));
        }
        t
    }

    fn src(file: &str, folder: &str, from_map: bool, size: u64) -> Source {
        Source { path: PathBuf::from(format!("{folder}/{file}")), file: file.into(), folder: folder.into(), from_map, size }
    }

    #[test]
    fn a_depot_file_answers_to_its_file_name_and_its_name() {
        let s = Scratch::new("answers");
        let p = s.file("HH20_2022.hof", &hof("Hamburg Linie 20", &["Rathaus"]));
        assert!(answers_to(&p, "HH20_2022"));
        assert!(answers_to(&p, "hamburg linie 20 "));
        assert!(!answers_to(&p, "Hamburg"));
        assert!(!answers_to(&p, ""));
    }

    #[test]
    fn the_maps_depot_file_is_found_beside_other_buses_and_in_the_map() {
        let s = Scratch::new("sources");
        s.file("omsi/Vehicles/MAN_NL/HH20.hof", &hof("Hamburg Linie 20", &["Rathaus"]));
        s.file("omsi/Vehicles/MAN_NL/Grundorf.hof", &hof("Grundorf", &["Bahnhof"]));
        s.file("omsi/Vehicles/O530/hh20.hof", &hof("Hamburg Linie 20", &["Rathaus"]));
        s.file("omsi/Vehicles/O530/Other.hof", &hof("Spandau", &["Ruhleben"]));
        s.file("content/Vehicles/O530/HH20.hof", &hof("Hamburg Linie 20", &["Rathaus", "Wandsbek"]));
        s.file("omsi/maps/HH20/Depot/Map.hof", &hof("Hamburg Linie 20", &["Rathaus"]));
        s.file("omsi/maps/HH20/Grundorf.hof", &hof("Grundorf", &["Bahnhof"]));
        let folders = vec![
            ("MAN_NL".to_string(), vec![s.0.join("omsi/Vehicles/MAN_NL")]),
            // (the content folder's copy first: its HH20.hof hides the installation's hh20.hof)
            ("O530".to_string(), vec![s.0.join("content/Vehicles/O530"), s.0.join("omsi/Vehicles/O530")]),
        ];
        let found = sources_in("Hamburg Linie 20", &folders, Some(&s.0.join("omsi/maps/HH20")));
        let names: Vec<(String, String, bool)> = found.iter().map(|x| (x.folder.clone(), x.file.clone(), x.from_map)).collect();
        assert_eq!(names, vec![("MAN_NL".into(), "HH20.hof".into(), false), ("O530".into(), "HH20.hof".into(), false), ("HH20".into(), "Map.hof".into(), true)]);
        assert!(found[1].path.starts_with(s.0.join("content")), "the higher root's copy, not the one it hides");
        assert!(sources_in("", &folders, None).is_empty());
        assert!(sources_in("Ahlheim", &folders, None).is_empty());
    }

    #[test]
    fn the_copy_most_buses_carry_is_taken_and_the_maps_own_without_one() {
        let s = vec![src("HH20_2014.hof", "A", false, 10), src("HH20.hof", "B", false, 20), src("HH20.hof", "C", false, 20), src("HH20.hof", "D", false, 21), src("Map.hof", "HH20", true, 30)];
        assert_eq!(best(&s), Some(1), "the name three carry, the size two of them have, the first folder");
        let only_map = vec![src("Map.hof", "HH20", true, 30)];
        assert_eq!(best(&only_map), Some(0));
        assert_eq!(best(&[]), None);
        let tie = vec![src("b.hof", "Zeta", false, 1), src("a.hof", "Alpha", false, 1)];
        assert_eq!(best(&tie), Some(1), "equal counts: the first by folder");
    }

    #[test]
    fn a_depot_file_goes_where_the_bus_reads_them() {
        assert_eq!(depot_folder("Vehicles/MAN_NL/NL202.bus", true, false), "Vehicles/MAN_NL");
        assert_eq!(depot_folder("Vehicles\\Pack\\Solo\\x.bus", false, true), "Vehicles/Pack", "its pack's, where its depot files are");
        assert_eq!(depot_folder("Vehicles/Pack/Solo/x.bus", true, true), "Vehicles/Pack/Solo");
        assert_eq!(depot_folder("Vehicles/Pack/Solo/x.bus", false, false), "Vehicles/Pack/Solo");
    }

    #[test]
    fn nothing_is_added_over_a_file_or_twice() {
        let existing = vec![("Grundorf.hof".to_string(), "Grundorf".to_string()), ("HH20.hof".to_string(), "Hamburg 2014".to_string()), ("readme.txt".to_string(), String::new())];
        assert_eq!(check("HH20.hof", "Hamburg Linie 20", &existing), Err(Refusal::Taken("HH20.hof".into())), "another file under the same name is someone's own");
        assert_eq!(check("Spandau.hof", "Grundorf", &existing), Err(Refusal::Has("Grundorf.hof".into())), "the bus has one of that name");
        assert_eq!(check("Spandau.hof", "grundorf", &existing), Err(Refusal::Has("Grundorf.hof".into())));
        assert_eq!(check("readme.txt", "Spandau", &existing), Err(Refusal::Taken("readme.txt".into())));
        assert_eq!(check("HH20_2022.hof", "Hamburg Linie 20", &existing), Ok(()));
    }

    #[test]
    fn the_copy_goes_into_the_content_folder_and_never_over_a_file() {
        let s = Scratch::new("copy");
        let omsi = s.0.join("omsi");
        let source = s.file("omsi/Vehicles/O530/HH20.hof", &hof("Hamburg Linie 20", &["Rathaus", "Wandsbek"]));
        s.file("omsi/Vehicles/MAN_NL/Grundorf.hof", &hof("Grundorf", &["Bahnhof"]));
        let bus = s.0.join("omsi/Vehicles/MAN_NL");
        let target = s.0.join("content/Vehicles/MAN_NL");
        let before: Vec<String> = std::fs::read_dir(&bus).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        let put = copy_into(&source, &target, std::slice::from_ref(&bus), "Hamburg Linie 20", Some(&omsi)).unwrap();
        assert_eq!(put, target.join("HH20.hof"));
        assert_eq!(std::fs::read(&put).unwrap(), std::fs::read(&source).unwrap(), "the same file");
        let after: Vec<String> = std::fs::read_dir(&bus).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(before, after, "the OMSI 2 folder's bus is not touched");
        let left: Vec<String> = std::fs::read_dir(&target).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, vec!["HH20.hof".to_string()], "no half copy left behind");
        // a second time: the bus has it now
        assert_eq!(copy_into(&source, &target, std::slice::from_ref(&bus), "Hamburg Linie 20", Some(&omsi)), Err(Refusal::Has("HH20.hof".into())));
        // a file of that name beside the bus in the installation is never hidden
        let other = s.file("omsi/Vehicles/Solaris/HH20.hof", &hof("Hamburg 2014", &["Rathaus"]));
        let _ = other;
        let solaris = s.0.join("omsi/Vehicles/Solaris");
        assert_eq!(copy_into(&source, &s.0.join("content/Vehicles/Solaris"), std::slice::from_ref(&solaris), "Hamburg Linie 20", Some(&omsi)), Err(Refusal::Taken("HH20.hof".into())));
        assert!(!s.0.join("content/Vehicles/Solaris").exists());
        // never into the OMSI 2 folder's vehicles
        assert_eq!(copy_into(&source, &s.0.join("OMSI/vehicles/X"), &[], "Hamburg Linie 20", Some(&s.0.join("omsi"))), Err(Refusal::Original));
        // nor anything that is no depot file
        let text = s.file("omsi/maps/readme.txt", "x");
        assert!(matches!(copy_into(&text, &s.0.join("content/Vehicles/X"), &[], "x", Some(&omsi)), Err(Refusal::Unreadable(_))));
        let empty = s.file("omsi/maps/empty.hof", "\r\n");
        assert!(matches!(copy_into(&empty, &s.0.join("content/Vehicles/X"), &[], "empty", Some(&omsi)), Err(Refusal::Unreadable(_))));
    }
}
