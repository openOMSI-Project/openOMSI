//! Installing mods as background jobs.
//!
//! A mod is a folder or a `.zip`. Installing it happens in four steps, each of which can be
//! followed on the Mods page and cancelled:
//!
//! 1. **Plan** - from the archive's central directory (or the folder listing) alone, work
//!    out what goes where: OMSI-style folders (`Vehicles`, `maps`, `Sceneryobjects` ...) are
//!    merged into the same folders of the content folder, a lone bus / map / object folder is
//!    put under the right folder, and everything else (read-mes, screenshots) is left out.
//!    A pack that only adds paints or textures to a bus that is not installed is kept aside
//!    in `Mods/waiting` and installed by itself once the bus is there.
//! 2. **Check** - the unpacked size of the planned files plus a margin must fit on the disk
//!    of the content folder; otherwise the job stops with the numbers before anything is
//!    written.
//! 3. **Unpack / copy** into `<content>/.install-staging/<pid>-<job>`, on the same volume
//!    as the content folder, with the free space watched while it runs. The files of a
//!    folder in the Mods inbox are hard-linked there instead (no space, no time), so the
//!    staging folder never holds the only copy of anything.
//! 4. **Move** into place: a new folder is renamed into the content folder in one step,
//!    files for an existing folder are renamed over it one by one. Only then are the
//!    linked inbox files removed from the inbox.
//!
//! The staging folder is removed whatever happens (done, failed, cancelled, or the launcher
//! killed half-way: stale ones of launchers that died - and the old `~/.openomsi/unzip`
//! - are removed on start).
//!
//! Every path inside a source goes through `safe_rel` before it is planned, so no entry
//! (`..\..\x` in a zip made on Windows included) can be written outside the staging folder.
//!
//! **In place** (`InstallMode::InPlace`): a `.zip` laid out like OMSI 2 is not unpacked at
//! all. It is put into `<content>/Archives/` - hard-linked when it is on the same disk
//! (no space, no time), moved when it came through the Mods inbox, else copied through the
//! staging folder after the same free-space check - and the game mounts every archive
//! there as a content root (`omsi_cfg::vfs`), as the launcher's own lists do. The plan is
//! still made, for the report and to check that the archive's folders are where the game
//! looks for them. `InstallMode::Auto` (the page's default) unpacks what fits on the disk
//! and uses an archive in place when its unpacked size does not.

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Keep at least this much free on the disk after an install (bytes).
const MIN_MARGIN: u64 = 512 * 1024 * 1024;
/// Stop unpacking when the disk gets this full while a job runs (something else writes).
const ABORT_BELOW: u64 = 1024 * 1024 * 1024;
/// Folder of the running jobs' staging areas, inside the content folder.
pub const STAGING: &str = ".install-staging";
/// Packs waiting for the bus they belong to.
pub const WAITING: &str = "waiting";
/// Archives used in place, inside the content folder (the game mounts every `.zip` there).
pub const ARCHIVES: &str = "Archives";

/// How an archive is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum InstallMode {
    /// Unpack (or copy) the files into the content folder.
    #[default]
    Extract,
    /// Put the archive into `<content>/Archives` and let the game read it as a content root.
    InPlace,
    /// Unpack when the unpacked files fit on the disk, else use the archive in place.
    Auto,
}

impl InstallMode {
    pub fn parse(s: &str) -> InstallMode {
        match s.trim().to_ascii_lowercase().as_str() {
            "inplace" | "in-place" | "mount" => InstallMode::InPlace,
            "auto" => InstallMode::Auto,
            _ => InstallMode::Extract,
        }
    }
}

/// What a job has done so far, as the page shows it.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Progress {
    pub id: u64,
    pub source: String,
    pub name: String,
    /// queued, planning, checking, unpacking, copying, moving, done, failed, cancelled
    pub state: String,
    pub mode: InstallMode,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// Free bytes on the content folder's disk when the job checked, and what it needs.
    pub free_bytes: u64,
    pub needed_bytes: u64,
    pub message: String,
    pub report: Vec<String>,
    pub warnings: Vec<String>,
    /// What was put into the content folder: `Vehicles/Foo`, `maps/Bar`, `Sceneryobjects`.
    pub installed: Vec<String>,
    /// Packs put into `Mods/waiting` (their names).
    pub kept_aside: Vec<String>,
    pub from_inbox: bool,
    pub started: u64,
    pub finished: Option<u64>,
}

pub struct Job {
    pub id: u64,
    pub source: PathBuf,
    pub mode: InstallMode,
    /// The source lies in the Mods inbox: its files may be linked instead of copied (and
    /// removed from the inbox once installed), and what is left of it goes to
    /// `Mods/installed`.
    pub from_inbox: bool,
    cancel: AtomicBool,
    progress: Mutex<Progress>,
    bytes_done: AtomicU64,
    files_done: AtomicU64,
    /// Inbox files hard-linked (not copied) into the staging folder. They stay where they
    /// are until the install is in place, and are removed from the inbox only then.
    linked: Mutex<Vec<PathBuf>>,
    /// Tests: wait for the cancel after this many files.
    stall_at: u64,
}

impl Job {
    pub fn snapshot(&self) -> Progress {
        let mut p = self.progress.lock().unwrap().clone();
        p.bytes_done = self.bytes_done.load(Ordering::Relaxed);
        p.files_done = self.files_done.load(Ordering::Relaxed);
        p
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn set(&self, f: impl FnOnce(&mut Progress)) {
        f(&mut self.progress.lock().unwrap());
    }

    fn state(&self, s: &str, msg: impl Into<String>) {
        let msg = msg.into();
        self.set(|p| {
            p.state = s.into();
            p.message = msg;
        });
    }

    fn is_active(&self) -> bool {
        self.progress.lock().unwrap().finished.is_none()
    }
}

static JOBS: Mutex<Vec<Arc<Job>>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// One install at a time: two unpacking at once would both pass the space check.
static RUNNING: Mutex<()> = Mutex::new(());

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// All jobs of this launcher, newest first.
pub fn jobs() -> Vec<Progress> {
    let mut v: Vec<Progress> = JOBS.lock().unwrap().iter().map(|j| j.snapshot()).collect();
    v.sort_by(|a, b| b.id.cmp(&a.id));
    v
}

pub fn cancel(id: u64) -> bool {
    match JOBS.lock().unwrap().iter().find(|j| j.id == id) {
        Some(j) if j.is_active() => {
            j.cancel();
            true
        }
        _ => false,
    }
}

/// Forget finished jobs (the page's "clear").
pub fn clear_finished() {
    JOBS.lock().unwrap().retain(|j| j.is_active());
}

/// Is `source` being installed right now?
pub fn is_busy(source: &Path) -> bool {
    JOBS.lock().unwrap().iter().any(|j| j.is_active() && j.source == source)
}

/// Queue an install; it runs on its own thread. `content` is the content folder.
pub fn start(content: PathBuf, root: Option<PathBuf>, source: PathBuf, mode: InstallMode, from_inbox: bool) -> Arc<Job> {
    start_inner(content, root, source, mode, from_inbox, u64::MAX)
}

fn start_inner(content: PathBuf, root: Option<PathBuf>, source: PathBuf, mode: InstallMode, from_inbox: bool, stall_at: u64) -> Arc<Job> {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let name = source.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| source.display().to_string());
    let job = Arc::new(Job {
        id,
        source: source.clone(),
        mode,
        from_inbox,
        cancel: AtomicBool::new(false),
        progress: Mutex::new(Progress { id, source: source.to_string_lossy().to_string(), name, state: "queued".into(), mode, from_inbox, started: now_secs(), message: "waiting for the install before it".into(), ..Default::default() }),
        bytes_done: AtomicU64::new(0),
        files_done: AtomicU64::new(0),
        linked: Mutex::new(Vec::new()),
        stall_at,
    });
    JOBS.lock().unwrap().push(job.clone());
    let j = job.clone();
    std::thread::spawn(move || {
        let _one = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        let result = if j.cancelled() {
            Err(anyhow!(Cancelled))
        } else {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&j, &content, root.as_deref()))).unwrap_or_else(|_| Err(anyhow!("the install stopped with an internal error")))
        };
        // (an inbox source's files were only linked into it: removing it loses nothing)
        let staging = staging_dir(&content, j.id);
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_dir(content.join(STAGING));
        // (whatever was written, the game's lookups start afresh)
        omsi_cfg::content_changed();
        match result {
            Ok(()) => j.set(|p| {
                p.state = "done".into();
                p.finished = Some(now_secs());
            }),
            Err(e) if e.downcast_ref::<Cancelled>().is_some() => j.set(|p| {
                p.state = "cancelled".into();
                p.message = "cancelled - nothing was installed, the unpacked files were removed".into();
                p.finished = Some(now_secs());
            }),
            Err(e) => j.set(|p| {
                p.state = "failed".into();
                p.message = format!("{e:#}");
                p.report.push(format!("failed: {e:#}"));
                p.finished = Some(now_secs());
            }),
        }
    });
    job
}

/// Run an install on this thread and return how it ended (the CLI).
pub fn run_blocking(content: PathBuf, root: Option<PathBuf>, source: PathBuf, mode: InstallMode, cancel_after: Option<std::time::Duration>, verbose: bool) -> Progress {
    let job = start(content, root, source, mode, false);
    let t0 = std::time::Instant::now();
    let mut last = String::new();
    loop {
        let p = job.snapshot();
        if verbose {
            let line = format!("{} {} {}/{} files {:.1}/{:.1} MB {}", p.state, p.name, p.files_done, p.files_total, p.bytes_done as f64 / 1e6, p.bytes_total as f64 / 1e6, p.message);
            if line != last {
                eprintln!("{line}");
                last = line;
            }
        }
        if p.finished.is_some() {
            return p;
        }
        if let Some(c) = cancel_after {
            if t0.elapsed() >= c {
                job.cancel();
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[derive(Debug)]
struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}
impl std::error::Error for Cancelled {}

fn staging_dir(content: &Path, id: u64) -> PathBuf {
    content.join(STAGING).join(format!("{}-{id}", std::process::id()))
}

// ---------------------------------------------------------------------------------------
// disk space and processes

/// Free bytes for this user on the volume of `path` (the nearest existing ancestor).
pub fn free_space(path: &Path) -> Option<u64> {
    let mut p = path.to_path_buf();
    while !p.exists() {
        if !p.pop() {
            return None;
        }
    }
    free_space_impl(&p)
}

#[cfg(unix)]
fn free_space_impl(p: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    Some(s.f_bavail as u64 * s.f_frsize as u64)
}

#[cfg(windows)]
fn free_space_impl(p: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    }
    let wide: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
    let (mut a, mut t, mut f) = (0u64, 0u64, 0u64);
    (unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut a, &mut t, &mut f) } != 0).then_some(a)
}

#[cfg(not(any(unix, windows)))]
fn free_space_impl(_p: &Path) -> Option<u64> {
    None
}

/// Is the process `pid` still running?
#[cfg(unix)]
pub fn pid_alive(pid: u32) -> bool {
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
pub fn pid_alive(pid: u32) -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn GetExitCodeProcess(h: isize, code: *mut u32) -> i32;
        fn CloseHandle(h: isize) -> i32;
    }
    const QUERY_LIMITED: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let h = OpenProcess(QUERY_LIMITED, 0, pid);
        if h == 0 {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code) != 0 && code == STILL_ACTIVE;
        CloseHandle(h);
        ok
    }
}

#[cfg(not(any(unix, windows)))]
pub fn pid_alive(_pid: u32) -> bool {
    true
}

pub fn gb(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.1} GB", b as f64 / (1u64 << 30) as f64)
    } else if b >= 1 << 20 {
        format!("{:.0} MB", b as f64 / (1u64 << 20) as f64)
    } else {
        format!("{} KB", b.div_ceil(1024))
    }
}

/// Remove what interrupted installs left behind: the old `unzip` folder of earlier
/// launchers, and staging folders of launchers that are no longer running. Returns what
/// was removed.
pub fn cleanup_stale(data_dir: &Path, content: Option<&Path>) -> Vec<String> {
    let mut out = Vec::new();
    let legacy = data_dir.join("unzip");
    if legacy.exists() {
        let size = tree_size(&legacy);
        if std::fs::remove_dir_all(&legacy).is_ok() {
            out.push(format!("removed {} of unpacked files an earlier install left in {}", gb(size), legacy.display()));
        }
    }
    let Some(content) = content else { return out };
    let dir = content.join(STAGING);
    let Ok(rd) = std::fs::read_dir(&dir) else { return out };
    let active: Vec<u64> = JOBS.lock().unwrap().iter().filter(|j| j.is_active()).map(|j| j.id).collect();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let (pid, id) = match name.split_once('-') {
            Some((p, i)) => (p.parse::<u32>().unwrap_or(0), i.parse::<u64>().unwrap_or(0)),
            None => (0, 0),
        };
        let mine = pid == std::process::id();
        let stale = if mine { !active.contains(&id) } else { pid == 0 || !pid_alive(pid) };
        if stale {
            let size = tree_size(&e.path());
            if std::fs::remove_dir_all(e.path()).is_ok() {
                out.push(format!("removed {} of partial files of an interrupted install ({})", gb(size), e.path().display()));
            }
        }
    }
    let _ = std::fs::remove_dir(&dir);
    out
}

fn tree_size(p: &Path) -> u64 {
    let Ok(md) = std::fs::symlink_metadata(p) else { return 0 };
    if !md.is_dir() {
        return md.len();
    }
    std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| tree_size(&e.path())).sum()).unwrap_or(0)
}

// ---------------------------------------------------------------------------------------
// the plan

/// One file of the source.
#[derive(Debug, Clone)]
struct Entry {
    /// Path inside the source, `/`-separated, no leading slash.
    rel: String,
    size: u64,
    /// Index in the archive (zip sources).
    index: usize,
}

/// A folder of the source as a tree of names.
#[derive(Default, Debug)]
struct Node {
    dirs: BTreeMap<String, Node>,
    /// (file name, entry index)
    files: Vec<(String, usize)>,
}

impl Node {
    fn insert(&mut self, rel: &str, entry: usize) {
        let mut parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
        let Some(file) = parts.pop() else { return };
        let mut n = self;
        for p in parts {
            n = n.dirs.entry(p.to_string()).or_default();
        }
        n.files.push((file.to_string(), entry));
    }

    fn get(&self, path: &str) -> Option<&Node> {
        let mut n = self;
        for p in path.split('/').filter(|p| !p.is_empty()) {
            n = n.dirs.get(p)?;
        }
        Some(n)
    }

    /// Every entry index below this node.
    fn all(&self, out: &mut Vec<usize>) {
        out.extend(self.files.iter().map(|f| f.1));
        for d in self.dirs.values() {
            d.all(out);
        }
    }

    fn has_ext_deep(&self, exts: &[&str]) -> bool {
        self.files.iter().any(|(f, _)| has_ext(f, exts)) || self.dirs.values().any(|d| d.has_ext_deep(exts))
    }
}

fn has_ext(name: &str, exts: &[&str]) -> bool {
    name.rsplit_once('.').map(|(_, x)| exts.iter().any(|e| x.eq_ignore_ascii_case(e))).unwrap_or(false)
}

/// A path inside a source (`/` or `\` separated) as a relative `/`-separated path, or None
/// when it could lead out of the folder it is unpacked into: absolute, with a drive or a
/// `:` (a Windows drive-relative path or stream), with a `..`, or a NUL. `zip`'s own check
/// reads `\` as a plain character on macOS and Linux, so `..\..\x` passes it and only
/// turns into a way out once the separators are unified - which is why this runs after.
fn safe_rel(name: &str) -> Option<String> {
    let name = name.replace('\\', "/");
    if name.starts_with('/') || name.contains('\0') {
        return None;
    }
    let mut parts = Vec::new();
    for p in name.split('/') {
        match p {
            "" | "." => continue,
            ".." => return None,
            p if p.contains(':') => return None,
            p => parts.push(p),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// An error unless `rel` is a `safe_rel` path: checked again right before anything is
/// written, so that nothing lands outside its folder whatever the plan says.
/// For `mods`: whether `rel` is a plain path inside the content folder.
pub(crate) fn check_rel_pub(rel: &str) -> Result<()> {
    check_rel(rel)
}

fn check_rel(rel: &str) -> Result<()> {
    if safe_rel(rel).as_deref() != Some(rel) {
        return Err(anyhow!("refused to write {rel:?}: it is not a plain path inside the mod"));
    }
    Ok(())
}

/// Where a downloaded plugin is put instead of `Plugins` (see `plan`).
pub const PLUGINS_HELD: &str = "plugins-not-enabled";

/// Files that are never installed.
fn is_junk(rel: &str) -> bool {
    rel.split('/').any(|p| p == "__MACOSX" || p == ".DS_Store" || p.eq_ignore_ascii_case("thumbs.db") || p.eq_ignore_ascii_case("desktop.ini") || p.starts_with("._"))
}

/// The canonical spelling of a content folder name, if `name` is one.
pub fn content_folder_of(name: &str) -> Option<&'static str> {
    omsi_cfg::CONTENT_FOLDERS.iter().copied().find(|f| f.eq_ignore_ascii_case(name))
}

/// Content folders that never lie inside a bus, map or object folder (unlike `Texture`,
/// `Sound`, `Fonts` or `Scripts`, which a bus folder has too).
const PACK_ONLY: &[&str] = &["Vehicles", "maps", "Sceneryobjects", "Splines", "Humans", "Trains", "Drivers", "TicketPacks", "Plugins", "Situations", "Weather", "Money", "Announcements", "Inputs"];

/// What a folder is, judged by the files directly in it.
fn guess_kind(n: &Node) -> Option<&'static str> {
    if n.files.iter().any(|(f, _)| f.eq_ignore_ascii_case("global.cfg")) {
        return Some("maps");
    }
    for (ext, kind) in [("bus", "Vehicles"), ("ovh", "Vehicles"), ("zug", "Trains"), ("sco", "Sceneryobjects"), ("sli", "Splines"), ("hum", "Humans"), ("owt", "Weather"), ("otp", "TicketPacks"), ("oft", "Fonts"), ("odr", "Drivers"), ("osn", "Situations"), ("dll", "Plugins"), ("opl", "Plugins")] {
        if n.files.iter().any(|(f, _)| has_ext(f, &[ext])) {
            return Some(kind);
        }
    }
    None
}

/// Where a part of the source goes.
#[derive(Debug, Clone)]
struct Mapping {
    /// Folder inside the source ("" = its root).
    src: String,
    /// Destination relative to the content folder.
    dest: String,
    /// What it is, for the report.
    what: String,
    /// Entries (with their path below `src`).
    files: Vec<(usize, String)>,
    bytes: u64,
    /// Kept aside: a repaint / add-on for a bus that is not installed.
    aside: bool,
}

#[derive(Debug, Default)]
struct Plan {
    maps: Vec<Mapping>,
    notes: Vec<String>,
    warnings: Vec<String>,
}

/// Lookup of what is installed already (content folder and the OMSI 2 folder).
struct Installed<'a> {
    content: &'a Path,
    root: Option<&'a Path>,
}

impl Installed<'_> {
    /// The installed spelling of `Vehicles/<name>` (content folder first), if any.
    fn vehicle(&self, name: &str) -> Option<String> {
        // the content folder, the archives used in place there, the OMSI folder
        let archives = self.content.join(ARCHIVES);
        let mounted: Vec<PathBuf> = omsi_cfg::vfs::mounts().iter().map(|m| m.path().to_path_buf()).filter(|p| p.starts_with(&archives)).collect();
        let bases: Vec<&Path> = std::iter::once(self.content).chain(mounted.iter().map(|p| p.as_path())).chain(self.root).collect();
        for base in bases {
            for (n, is_dir) in omsi_cfg::vfs::list_dir(&base.join("Vehicles")).unwrap_or_default() {
                let n = n.to_string_lossy().to_string();
                if is_dir && n.eq_ignore_ascii_case(name) {
                    return Some(n);
                }
            }
        }
        None
    }
}

fn collect(node: &Node, src: &str, entries: &[Entry]) -> (Vec<(usize, String)>, u64) {
    let mut idx = Vec::new();
    node.all(&mut idx);
    let mut bytes = 0;
    let files = idx
        .into_iter()
        .map(|i| {
            bytes += entries[i].size;
            let below = entries[i].rel.strip_prefix(src).unwrap_or(&entries[i].rel).trim_start_matches('/').to_string();
            (i, below)
        })
        .collect();
    (files, bytes)
}

fn join(a: &str, b: &str) -> String {
    if a.is_empty() {
        b.to_string()
    } else {
        format!("{a}/{b}")
    }
}

/// Work out what goes where.
fn plan(entries: &[Entry], source_name: &str, installed: &Installed) -> Plan {
    let mut tree = Node::default();
    for (i, e) in entries.iter().enumerate() {
        tree.insert(&e.rel, i);
    }
    let mut plan = Plan::default();
    let mut used: HashSet<usize> = HashSet::new();
    // level by level: a folder that *is* a thing (a bus, a map ...) is taken whole; the
    // OMSI folders among a folder's children are merged; stop below the first level that
    // gave anything
    let mut level: Vec<String> = vec![String::new()];
    for _ in 0..5 {
        let mut next = Vec::new();
        let mut found = false;
        for path in &level {
            let Some(node) = tree.get(path) else { continue };
            // a folder holding bus files is a bus even with `Sound` / `Texture` in it; one
            // holding `Vehicles` or `Sceneryobjects` is a pack whatever lies beside them
            let pack = node.dirs.keys().any(|d| PACK_ONLY.iter().any(|f| f.eq_ignore_ascii_case(d)));
            if let Some(kind) = guess_kind(node).filter(|k| *k == "maps" || !pack) {
                let name = if path.is_empty() { source_name.to_string() } else { path.rsplit('/').next().unwrap_or(path).to_string() };
                let (files, bytes) = collect(node, path, entries);
                used.extend(files.iter().map(|f| f.0));
                let dest = format!("{kind}/{name}");
                plan.maps.push(Mapping { src: path.clone(), dest, what: kind_label(kind).into(), files, bytes, aside: false });
                found = true;
                continue;
            }
            for (child, sub) in &node.dirs {
                let cpath = join(path, child);
                match content_folder_of(child) {
                    Some("Vehicles") => {
                        found = true;
                        // each bus folder separately: a folder without a vehicle file only
                        // adds to a bus (paints, textures), which must be installed
                        for (bus, bn) in &sub.dirs {
                            let bpath = join(&cpath, bus);
                            let (files, bytes) = collect(bn, &bpath, entries);
                            used.extend(files.iter().map(|f| f.0));
                            if bn.has_ext_deep(&["bus", "ovh", "zug", "sco"]) {
                                plan.maps.push(Mapping { src: bpath, dest: format!("Vehicles/{bus}"), what: "vehicle".into(), files, bytes, aside: false });
                            } else {
                                match installed.vehicle(bus) {
                                    Some(have) => plan.maps.push(Mapping { src: bpath, dest: format!("Vehicles/{have}"), what: format!("add-on for {have} (paints / textures)"), files, bytes, aside: false }),
                                    None => {
                                        plan.warnings.push(format!("{bus}: only paints or textures for a bus that is not installed (Vehicles/{bus}) - kept aside in Mods/{WAITING} and installed by itself once that bus is installed"));
                                        plan.maps.push(Mapping { src: bpath, dest: format!("Vehicles/{bus}"), what: format!("add-on for {bus}, which is not installed"), files, bytes, aside: true });
                                    }
                                }
                            }
                        }
                        let loose: Vec<(usize, String)> = sub.files.iter().map(|(f, i)| (*i, f.clone())).collect();
                        if !loose.is_empty() {
                            plan.notes.push(format!("{} loose file(s) directly in {cpath} left out", loose.len()));
                        }
                    }
                    Some(canon) => {
                        found = true;
                        let (files, bytes) = collect(sub, &cpath, entries);
                        used.extend(files.iter().map(|f| f.0));
                        plan.maps.push(Mapping { src: cpath, dest: canon.to_string(), what: format!("{canon} folder"), files, bytes, aside: false });
                    }
                    None => next.push(cpath),
                }
            }
        }
        if found || next.is_empty() {
            break;
        }
        level = next;
    }
    // plugins are native code that the game loads by itself: one from a download is put
    // aside, and only the player moving it into `Plugins` makes it run
    for m in plan.maps.iter_mut() {
        let lower = m.dest.to_ascii_lowercase();
        if lower == "plugins" || lower.starts_with("plugins/") {
            m.dest = format!("Mods/{PLUGINS_HELD}{}", &m.dest["plugins".len()..]);
            plan.warnings.push(format!(
                "{}: a plugin runs its own program code inside the game, so it was not enabled - it is in Mods/{PLUGINS_HELD}; move it into Plugins yourself if you trust where it came from",
                if m.src.is_empty() { source_name } else { &m.src }
            ));
        }
    }
    let unused = entries.len() - used.len();
    if unused > 0 && !plan.maps.is_empty() {
        let mut sample: Vec<&str> = entries.iter().enumerate().filter(|(i, _)| !used.contains(i)).map(|(_, e)| e.rel.as_str()).take(4).collect();
        sample.sort();
        plan.notes.push(format!("{unused} file(s) that belong to no OMSI folder were left out (e.g. {})", sample.join(", ")));
    }
    plan
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "maps" => "map",
        "Vehicles" => "vehicle",
        "Trains" => "train",
        "Sceneryobjects" => "scenery objects",
        "Splines" => "splines",
        "Humans" => "people",
        "Weather" => "weather",
        "TicketPacks" => "ticket pack",
        "Fonts" => "fonts",
        "Drivers" => "driver",
        "Situations" => "situation",
        "Plugins" => "plugin",
        _ => "content",
    }
}

// ---------------------------------------------------------------------------------------
// the source

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArchiveKind {
    Zip,
    SevenZip,
    Rar,
}

fn archive_kind(path: &Path) -> Option<ArchiveKind> {
    if !path.is_file() {
        return None;
    }
    match path.extension()?.to_string_lossy().to_ascii_lowercase().as_str() {
        "zip" => Some(ArchiveKind::Zip),
        "7z" => Some(ArchiveKind::SevenZip),
        "rar" => Some(ArchiveKind::Rar),
        _ => None,
    }
}

/// Expand formats the game cannot mount into the install job's own staging area. Header
/// sizes are checked against the content volume before any data is written; the later
/// install step hard-links these staged files into their final layout.
fn unpack_archive(job: &Job, content: &Path, src: &Path, dest: &Path, kind: ArchiveKind) -> Result<()> {
    let (files, bytes) = match kind {
        ArchiveKind::SevenZip => {
            let reader = sevenz_rust2::ArchiveReader::open(src, sevenz_rust2::Password::empty())
                .with_context(|| format!("{} is not a readable 7z archive", src.display()))?;
            let entries = &reader.archive().files;
            let files = entries.iter().filter(|e| e.has_stream && !e.is_directory).count() as u64;
            let bytes = entries.iter().filter(|e| e.has_stream && !e.is_directory).fold(0u64, |sum, e| sum.saturating_add(e.size));
            (files, bytes)
        }
        ArchiveKind::Rar => {
            let archive = unrar_rs::RarArchive::open(std::fs::File::open(src)?)
                .with_context(|| format!("{} is not a readable RAR archive", src.display()))?;
            let members: Vec<_> = archive.entries().collect();
            if members.iter().any(|m| m.unpacked_size.is_none() && !m.is_directory) {
                return Err(anyhow!("{} is a multi-volume RAR archive; add all parts and start with the first .rar file", src.display()));
            }
            let files = members.iter().filter(|m| !m.is_directory && !m.is_symlink && !m.is_hardlink).count() as u64;
            let bytes = members.iter().filter(|m| !m.is_directory && !m.is_symlink && !m.is_hardlink).fold(0u64, |sum, m| sum.saturating_add(m.unpacked_size.unwrap_or(0)));
            (files, bytes)
        }
        ArchiveKind::Zip => unreachable!(),
    };
    let free = free_space(content).unwrap_or(u64::MAX);
    let needed = bytes.saturating_add(MIN_MARGIN);
    job.set(|p| {
        p.files_total = files;
        p.bytes_total = bytes;
        p.free_bytes = free;
        p.needed_bytes = needed;
    });
    if needed > free {
        return Err(anyhow!("not enough disk space to unpack {}: {} plus {} kept free needs {}, but only {} is free on {}", src.display(), gb(bytes), gb(MIN_MARGIN), gb(needed), gb(free), content.display()));
    }
    if job.cancelled() {
        return Err(anyhow!(Cancelled));
    }
    std::fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
    match kind {
        ArchiveKind::SevenZip => {
            let mut done = 0u64;
            sevenz_rust2::decompress_file_with_extract_fn(src, dest, |entry, reader, target| {
                if job.cancelled() {
                    return Err(sevenz_rust2::Error::Io(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled"), "install cancelled".into()));
                }
                let wrote = sevenz_rust2::default_entry_extract_fn(entry, reader, target)?;
                if wrote && entry.has_stream && !entry.is_directory {
                    done += 1;
                    job.files_done.store(done, Ordering::Relaxed);
                    if free_space(content).is_some_and(|left| left < ABORT_BELOW) {
                        return Err(sevenz_rust2::Error::Io(std::io::Error::new(std::io::ErrorKind::Other, "free disk space fell below 1 GB"), "install stopped".into()));
                    }
                }
                Ok(wrote)
            }).with_context(|| format!("unpacking {} (the archive may be damaged or encrypted)", src.display()))?;
        }
        ArchiveKind::Rar => {
            let mut archive = unrar_rs::RarArchive::open(std::fs::File::open(src)?)
                .with_context(|| format!("opening {}", src.display()))?;
            let members: Vec<_> = archive.entries().collect();
            for (index, member) in members.iter().enumerate() {
                if job.cancelled() {
                    return Err(anyhow!(Cancelled));
                }
                if member.is_directory || member.is_symlink || member.is_hardlink {
                    continue;
                }
                let Some(rel) = safe_rel(&member.name) else {
                    job.set(|p| p.warnings.push(format!("unsafe RAR path refused: {}", member.name)));
                    continue;
                };
                let out = dest.join(&rel);
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut file = std::fs::File::create(&out)?;
                let n = archive.by_index(index)?.copy_to(&mut file)
                    .with_context(|| format!("unpacking {} from {}", rel, src.display()))?;
                job.files_done.fetch_add(1, Ordering::Relaxed);
                job.bytes_done.fetch_add(n, Ordering::Relaxed);
                check_space(content)?;
            }
        }
        ArchiveKind::Zip => unreachable!(),
    }
    if job.cancelled() {
        return Err(anyhow!(Cancelled));
    }
    Ok(())
}

enum Source {
    /// A folder's files as they are on disk, by `Entry::index` (the name in `Entry::rel`
    /// may differ: `\` read as a separator).
    Folder(Vec<PathBuf>),
    Zip(zip::ZipArchive<std::io::BufReader<std::fs::File>>),
}

/// The files of a folder, with their paths on disk, and how many were left out because
/// their names cannot be installed safely.
fn list_folder(dir: &Path, job: &Job) -> Result<(Vec<Entry>, Vec<PathBuf>, usize)> {
    let mut found = Vec::new();
    let mut refused = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if job.cancelled() {
            return Err(anyhow!(Cancelled));
        }
        for e in std::fs::read_dir(&d).with_context(|| format!("reading {}", d.display()))?.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(dir) {
                let Some(rel) = safe_rel(&rel.to_string_lossy()) else {
                    refused += 1;
                    continue;
                };
                if !is_junk(&rel) {
                    let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                    found.push((rel, size, p));
                }
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    let mut entries = Vec::with_capacity(found.len());
    let mut paths = Vec::with_capacity(found.len());
    for (index, (rel, size, p)) in found.into_iter().enumerate() {
        entries.push(Entry { rel, size, index });
        paths.push(p);
    }
    Ok((entries, paths, refused))
}

/// The read buffer an archive is opened with. The zip reader looks for the end of the
/// archive's table of contents from the back of the file in 2 KB steps, seeking before
/// each: every step emptied a 64 KB buffer and read it anew. The table of contents of a
/// 30 000-file map took 3.2 s here (0.06 s with 8 KB), and on a phone, whose shared storage
/// is read through a slow layer, minutes: the install stood at "reading the archive's table
/// of contents", 0 of 0 files, for every mod.
const ZIP_BUFFER: usize = 1 << 13;

fn list_zip(z: &mut zip::ZipArchive<std::io::BufReader<std::fs::File>>, job: &Job) -> Result<(Vec<Entry>, Vec<String>)> {
    let mut out = Vec::new();
    let mut problems = Vec::new();
    let (mut encrypted, mut unsupported, mut unsafe_names) = (0usize, BTreeMap::<String, usize>::new(), 0usize);
    let n = z.len();
    job.set(|p| p.files_total = n as u64);
    for i in 0..n {
        if i % 512 == 0 {
            if job.cancelled() {
                return Err(anyhow!(Cancelled));
            }
            job.files_done.store(i as u64, Ordering::Relaxed);
        }
        let f = z.by_index_raw(i).with_context(|| format!("reading entry {i} of the archive"))?;
        if f.is_dir() {
            continue;
        }
        let Some(rel) = f.enclosed_name().and_then(|name| safe_rel(&name.to_string_lossy())) else {
            unsafe_names += 1;
            continue;
        };
        if is_junk(&rel) {
            continue;
        }
        if f.encrypted() {
            encrypted += 1;
        }
        let method = f.compression();
        let supported = matches!(method, zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated);
        if !supported {
            *unsupported.entry(format!("{method:?}")).or_default() += 1;
        }
        out.push(Entry { rel, size: f.size(), index: i });
    }
    job.files_done.store(0, Ordering::Relaxed);
    if encrypted > 0 {
        problems.push(format!("{encrypted} file(s) in the archive are password protected - unpack it with the password first and install the folder"));
    }
    for (m, c) in unsupported {
        problems.push(format!("{c} file(s) are packed with {m}, which the launcher cannot unpack - unpack the archive with the system's tool and install the folder"));
    }
    if unsafe_names > 0 {
        problems.push(format!("{unsafe_names} file name(s) point outside the archive and were refused"));
    }
    Ok((out, problems))
}

// ---------------------------------------------------------------------------------------
// the job

fn run(job: &Job, content: &Path, root: Option<&Path>) -> Result<()> {
    // (the unit tests leave the user's own data folder alone)
    let data_dir = if cfg!(test) { content.join(".test-data") } else { crate::data_dir() };
    for line in cleanup_stale(&data_dir, Some(content)) {
        job.set(|p| p.report.push(line.clone()));
    }
    let src = job.source.clone();
    let archive = archive_kind(&src);
    let is_zip = archive == Some(ArchiveKind::Zip);
    let staged_archive = matches!(archive, Some(ArchiveKind::SevenZip | ArchiveKind::Rar));
    if archive.is_none() && !src.is_dir() {
        return Err(anyhow!("{} is neither a folder nor a supported mod archive (.zip, .7z, .rar)", src.display()));
    }
    if src.starts_with(content) && !job.from_inbox && !src.starts_with(content.join("Mods")) {
        return Err(anyhow!("{} is already inside the content folder", src.display()));
    }
    // (the name becomes a folder name: `...zip` has the stem `..`)
    let source_name = if archive.is_some() { src.file_stem() } else { src.file_name() }.map(|s| s.to_string_lossy().to_string()).filter(|n| safe_rel(n).as_deref() == Some(n.as_str())).unwrap_or_else(|| "mod".into());
    let (mut source, entries) = if staged_archive {
        let unpacked = staging_dir(content, job.id).join("source");
        job.state("unpacking", "checking the archive and unpacking it into staging");
        unpack_archive(job, content, &src, &unpacked, archive.unwrap())?;
        let (entries, paths, refused) = list_folder(&unpacked, job)?;
        if refused > 0 {
            job.set(|p| p.warnings.push(format!("{refused} file(s) whose names cannot be installed safely were left out")));
        }
        (Source::Folder(paths), entries)
    } else if is_zip {
        job.state("planning", "reading the archive's table of contents");
        let f = std::fs::File::open(&src).with_context(|| format!("opening {}", src.display()))?;
        let mut z = zip::ZipArchive::new(std::io::BufReader::with_capacity(ZIP_BUFFER, f)).with_context(|| format!("{} is not a readable zip archive", src.display()))?;
        let (entries, problems) = list_zip(&mut z, job)?;
        if !problems.is_empty() {
            return Err(anyhow!("{}: {}", src.display(), problems.join("; ")));
        }
        (Source::Zip(z), entries)
    } else {
        job.state("planning", "listing the folder");
        let (entries, paths, refused) = list_folder(&src, job)?;
        if refused > 0 {
            job.set(|p| p.warnings.push(format!("{refused} file(s) whose names cannot be installed safely (a '..', '\\' or ':' in them) were left out")));
        }
        (Source::Folder(paths), entries)
    };
    if entries.is_empty() {
        return Err(anyhow!("{} is empty", src.display()));
    }
    let installed = Installed { content, root };
    let plan = plan(&entries, &source_name, &installed);
    if plan.maps.is_empty() {
        return Err(anyhow!("could not tell what {} is: no Vehicles / maps / Sceneryobjects ... folders and no .bus / .sco / .sli / global.cfg files in it", src.display()));
    }
    for m in &plan.maps {
        let line = format!("{} -> {}{}/ ({} files, {}){}", if m.src.is_empty() { source_name.as_str() } else { m.src.as_str() }, if m.aside { format!("Mods/{WAITING}/{source_name}/") } else { String::new() }, m.dest, m.files.len(), gb(m.bytes), if m.what.is_empty() { String::new() } else { format!(" - {}", m.what) });
        job.set(|p| p.report.push(line.clone()));
    }
    job.set(|p| {
        p.report.extend(plan.notes.iter().cloned());
        p.warnings.extend(plan.warnings.iter().cloned());
    });
    // 2. does it fit?
    job.state("checking", "checking the free disk space");
    let total_files: u64 = plan.maps.iter().map(|m| m.files.len() as u64).sum();
    let total_bytes: u64 = plan.maps.iter().map(|m| m.bytes).sum();
    // an inbox folder's files are hard-linked into the staging folder (no space, and the
    // originals stay until the install is in place); everything else is written anew
    let moving = (job.from_inbox && !is_zip) || staged_archive;
    let needed = if moving { 0 } else { total_bytes + (total_bytes / 20).max(MIN_MARGIN) };
    let free = free_space(content).unwrap_or(u64::MAX);
    job.set(|p| {
        p.files_total = total_files;
        p.bytes_total = total_bytes;
        p.free_bytes = free;
        p.needed_bytes = needed;
    });
    // unpack, or use the archive in place?
    let layout = if is_zip { in_place_layout(&src, &plan) } else { Err(anyhow!("only a .zip archive can be used in place; this archive must be unpacked")) };
    let mode = if staged_archive {
        InstallMode::Extract
    } else { match job.mode {
        InstallMode::Auto if is_zip && needed > free => match &layout {
            Ok(()) => {
                let line = format!("{} unpacks to {}, which does not fit ({} free): using the archive in place instead", source_name, gb(total_bytes), gb(free));
                job.set(|p| p.report.push(line.clone()));
                InstallMode::InPlace
            }
            Err(e) => {
                let line = format!("the archive cannot be used in place either: {e:#}");
                job.set(|p| p.warnings.push(line.clone()));
                InstallMode::Extract
            }
        },
        InstallMode::Auto => InstallMode::Extract,
        m => m,
    }};
    job.set(|p| p.mode = mode);
    if mode == InstallMode::InPlace {
        layout?;
        drop(source);
        return place_archive(job, content, &src, &plan);
    }
    if needed > free {
        return Err(anyhow!(
            "not enough disk space: {} unpacks to {} ({} files), and with {} kept free that needs {}, but only {} is free on the disk of {}. Nothing was unpacked - free some space and try again.",
            source_name,
            gb(total_bytes),
            total_files,
            gb(needed - total_bytes),
            gb(needed),
            gb(free),
            content.display()
        ));
    }
    // 3. unpack / copy into the staging folder
    let staging = staging_dir(content, job.id);
    std::fs::create_dir_all(&staging).with_context(|| format!("creating {}", staging.display()))?;
    job.state(if is_zip { "unpacking" } else if moving { "moving" } else { "copying" }, format!("{} files, {}", total_files, gb(total_bytes)));
    let mut buf = vec![0u8; 1 << 20];
    let mut since_check = 0u64;
    for (mi, m) in plan.maps.iter().enumerate() {
        let base = staging.join(format!("{mi}"));
        for (idx, rel) in &m.files {
            if job.cancelled() {
                return Err(anyhow!(Cancelled));
            }
            check_rel(rel)?;
            let out = base.join(rel);
            if let Some(d) = out.parent() {
                std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
            }
            // two entries of the same name: the later one wins, and it must not write
            // through a link to an inbox file
            if std::fs::symlink_metadata(&out).is_ok() {
                std::fs::remove_file(&out).with_context(|| format!("replacing {}", out.display()))?;
            }
            let e = &entries[*idx];
            match &mut source {
                Source::Zip(z) => {
                    let mut f = z.by_index(e.index).with_context(|| format!("reading {} from the archive", e.rel))?;
                    let mut w = std::fs::File::create(&out).with_context(|| format!("writing {}", out.display()))?;
                    loop {
                        let n = f.read(&mut buf).with_context(|| format!("unpacking {} (the archive may be damaged)", e.rel))?;
                        if n == 0 {
                            break;
                        }
                        w.write_all(&buf[..n]).with_context(|| format!("writing {}", out.display()))?;
                        job.bytes_done.fetch_add(n as u64, Ordering::Relaxed);
                        since_check += n as u64;
                        if since_check > 64 << 20 {
                            since_check = 0;
                            if job.cancelled() {
                                return Err(anyhow!(Cancelled));
                            }
                            check_space(content)?;
                        }
                    }
                }
                Source::Folder(paths) => {
                    let from = &paths[e.index];
                    if moving && std::fs::hard_link(from, &out).is_ok() {
                        job.linked.lock().unwrap().push(from.clone());
                        job.bytes_done.fetch_add(e.size, Ordering::Relaxed);
                    } else {
                        let mut r = std::fs::File::open(from).with_context(|| format!("reading {}", from.display()))?;
                        let mut w = std::fs::File::create(&out).with_context(|| format!("writing {}", out.display()))?;
                        loop {
                            let n = r.read(&mut buf)?;
                            if n == 0 {
                                break;
                            }
                            w.write_all(&buf[..n]).with_context(|| format!("writing {}", out.display()))?;
                            job.bytes_done.fetch_add(n as u64, Ordering::Relaxed);
                            since_check += n as u64;
                            if since_check > 64 << 20 {
                                since_check = 0;
                                if job.cancelled() {
                                    return Err(anyhow!(Cancelled));
                                }
                                check_space(content)?;
                            }
                        }
                    }
                }
            }
            if job.files_done.fetch_add(1, Ordering::Relaxed) + 1 == job.stall_at {
                while !job.cancelled() {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
    }
    drop(source);
    if job.cancelled() {
        return Err(anyhow!(Cancelled));
    }
    // 4. into place
    job.state("moving", "moving the files into the content folder");
    let mut installed_items = Vec::new();
    let mut aside_items = Vec::new();
    // (the folders this install made: they go again when the mod is taken out of
    // Mods/installed, see `uninstall_removed`)
    let mut created = Vec::new();
    for (mi, m) in plan.maps.iter().enumerate() {
        let from = staging.join(format!("{mi}"));
        if !from.exists() {
            continue;
        }
        check_rel(&m.dest)?;
        let dest = if m.aside { content.join("Mods").join(WAITING).join(&source_name).join(&m.dest) } else { case_path(content, &m.dest) };
        if !m.aside && !dest.exists() && m.dest.contains('/') {
            if let Ok(rel) = dest.strip_prefix(content) {
                created.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
        let replaced = move_into(&from, &dest).with_context(|| format!("moving into {}", dest.display()))?;
        if replaced > 0 {
            let line = format!("{}: {replaced} existing file(s) replaced", m.dest);
            job.set(|p| p.report.push(line.clone()));
        }
        if m.aside {
            aside_items.push(format!("{source_name} ({})", m.dest));
        } else {
            installed_items.push(m.dest.clone());
        }
    }
    // the linked inbox files are installed now: they leave the inbox, and what is left of
    // it goes to Mods/installed
    for original in std::mem::take(&mut *job.linked.lock().unwrap()) {
        let _ = std::fs::remove_file(&original);
    }
    if job.from_inbox {
        let done = content.join("Mods").join("installed");
        let _ = std::fs::create_dir_all(&done);
        let target = done.join(src.file_name().unwrap_or_default());
        let _ = std::fs::remove_dir_all(&target);
        let _ = std::fs::remove_file(&target);
        if std::fs::rename(&src, &target).is_err() {
            if src.is_dir() {
                let _ = std::fs::remove_dir_all(&src);
            }
        } else {
            note_installed(content, &src.file_name().unwrap_or_default().to_string_lossy(), &created);
        }
    }
    // (noted as one mod: its own folders, see `mods`)
    crate::mods::record(content, &source_name, &created);
    let summary = match (installed_items.is_empty(), aside_items.is_empty()) {
        (false, true) => format!("installed {} - it is in the lists now", installed_items.join(", ")),
        (false, false) => format!("installed {}; kept aside: {}", installed_items.join(", "), aside_items.join(", ")),
        (true, _) => format!("nothing installed; kept aside: {}", aside_items.join(", ")),
    };
    job.set(|p| {
        p.installed = installed_items.clone();
        p.kept_aside = aside_items.clone();
        p.message = summary.clone();
        p.report.push(summary.clone());
    });
    Ok(())
}

/// Where a mod moved to `Mods/installed` keeps the list of the folders its install made
/// (`<name>.txt`, one content-relative path a line).
pub const RECORDS: &str = ".installed-records";
/// Where the folders of a mod taken out of `Mods/installed` go (nothing is deleted: moved
/// back into the content folder, or the mod into `Mods`, it is there again).
pub const UNINSTALLED: &str = "uninstalled";

/// Note the folders the install of `name` made (with those of an earlier install of it that
/// are still there).
fn note_installed(content: &Path, name: &str, created: &[String]) {
    let dir = content.join("Mods").join(RECORDS);
    let file = dir.join(format!("{name}.txt"));
    let mut all: Vec<String> = std::fs::read_to_string(&file).map(|t| t.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && content.join(l).exists()).collect()).unwrap_or_default();
    for c in created {
        if !all.contains(c) {
            all.push(c.clone());
        }
    }
    if all.is_empty() {
        let _ = std::fs::remove_file(&file);
        return;
    }
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(&file, all.join("\n") + "\n");
}

/// A mod the player deleted from `Mods/installed` is uninstalled (#819): the folders its
/// install made (a bus's own folder) are moved into `Mods/uninstalled/<name>`, so the
/// lists no longer show it - and nothing is lost. Folders a mod only added to (an existing
/// bus, the shared `Sceneryobjects`) stay. Returns the mods uninstalled.
pub fn uninstall_removed(content: &Path) -> Vec<String> {
    // (not while an install runs: a mod installed again leaves Mods/installed for a moment)
    if !content.join("Mods").join(RECORDS).is_dir() || jobs().iter().any(|j| j.finished.is_none()) {
        return Vec::new();
    }
    uninstall_removed_now(content)
}

fn uninstall_removed_now(content: &Path) -> Vec<String> {
    let records = content.join("Mods").join(RECORDS);
    let Ok(rd) = std::fs::read_dir(&records) else { return Vec::new() };
    let mut done = Vec::new();
    for e in rd.flatten() {
        let file = e.path();
        let Some(name) = file.file_name().map(|n| n.to_string_lossy().to_string()).and_then(|n| n.strip_suffix(".txt").map(str::to_string)) else { continue };
        let installed = content.join("Mods").join("installed").join(&name);
        if installed.exists() || installed.is_symlink() {
            continue;
        }
        let list: Vec<String> = std::fs::read_to_string(&file).unwrap_or_default().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        let mut left = Vec::new();
        for rel in list {
            // (only plain paths inside the content folder, never Mods itself)
            if check_rel(&rel).is_err() || rel.to_ascii_lowercase().starts_with("mods/") {
                continue;
            }
            let from = content.join(&rel);
            if !from.exists() {
                continue;
            }
            let to = content.join("Mods").join(UNINSTALLED).join(&name).join(&rel);
            if to.exists() {
                let _ = std::fs::remove_dir_all(&to);
            }
            let moved = to.parent().map(|p| std::fs::create_dir_all(p).is_ok()).unwrap_or(false) && std::fs::rename(&from, &to).is_ok();
            if !moved {
                left.push(rel);
            }
        }
        if left.is_empty() {
            let _ = std::fs::remove_file(&file);
            done.push(name);
        } else {
            let _ = std::fs::write(&file, left.join("\n") + "\n");
        }
    }
    done
}

/// Whether the game finds what `plan` says is in the zip at `src` when the archive is
/// mounted as it is: the mount starts at the archive's folder holding OMSI's content
/// folders, so every part of the plan must lie right there (a lone bus folder, which the
/// plan puts under `Vehicles`, must be unpacked).
fn in_place_layout(src: &Path, plan: &Plan) -> Result<()> {
    let archive = omsi_cfg::vfs::ZipArchive::open(src).with_context(|| format!("reading {}", src.display()))?;
    let prefix = archive.prefix().trim_end_matches('/').to_lowercase();
    let mut wrong: Vec<String> = Vec::new();
    for m in plan.maps.iter().filter(|m| !m.aside) {
        let want = if prefix.is_empty() { m.dest.to_lowercase() } else { format!("{prefix}/{}", m.dest.to_lowercase()) };
        if m.src.to_lowercase() != want {
            wrong.push(format!("{} (the game would look for it at {})", if m.src.is_empty() { "the archive itself" } else { m.src.as_str() }, want));
        }
    }
    if wrong.is_empty() {
        Ok(())
    } else {
        Err(anyhow!("the archive is not laid out like an OMSI 2 folder, so it has to be unpacked: {}", wrong.join("; ")))
    }
}

/// Step 3/4 for an archive used in place: into `<content>/Archives/<name>.zip`, linked,
/// moved (from the inbox) or copied through the staging folder, and mounted for the lists.
fn place_archive(job: &Job, content: &Path, src: &Path, plan: &Plan) -> Result<()> {
    let name = src.file_name().map(|n| n.to_string_lossy().to_string()).filter(|n| safe_rel(n).as_deref() == Some(n.as_str()) && !n.contains('/')).ok_or_else(|| anyhow!("{} has a name that cannot be used", src.display()))?;
    let archives = content.join(ARCHIVES);
    std::fs::create_dir_all(&archives).with_context(|| format!("creating {}", archives.display()))?;
    let dest = archives.join(&name);
    let size = std::fs::metadata(src).map(|m| m.len()).unwrap_or(0);
    job.set(|p| {
        p.files_total = 1;
        p.bytes_total = size;
    });
    let same = |a: &Path, b: &Path| -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let (Ok(x), Ok(y)) = (std::fs::metadata(a), std::fs::metadata(b)) {
                return x.dev() == y.dev() && x.ino() == y.ino();
            }
            false
        }
        #[cfg(not(unix))]
        {
            std::fs::canonicalize(a).ok() == std::fs::canonicalize(b).ok() && a.exists()
        }
    };
    let how;
    if dest.exists() && same(src, &dest) {
        how = "already in place";
    } else if job.from_inbox && std::fs::rename(src, &dest).is_ok() {
        how = "moved from the Mods folder";
    } else {
        // through the staging folder, so that the game never mounts half an archive
        let staging = staging_dir(content, job.id);
        std::fs::create_dir_all(&staging).with_context(|| format!("creating {}", staging.display()))?;
        let tmp = staging.join(&name);
        if std::fs::hard_link(src, &tmp).is_ok() {
            how = "linked: no disk space used, the archive also stays where it was";
        } else {
            let needed = size + (size / 20).max(MIN_MARGIN);
            let free = free_space(content).unwrap_or(u64::MAX);
            job.set(|p| {
                p.free_bytes = free;
                p.needed_bytes = needed;
            });
            if needed > free {
                return Err(anyhow!("not enough disk space to copy {} ({}) into {}: with {} kept free that needs {}, but only {} is free. Nothing was copied.", name, gb(size), archives.display(), gb(needed - size), gb(needed), gb(free)));
            }
            job.state("copying", format!("copying the archive ({})", gb(size)));
            let mut r = std::fs::File::open(src).with_context(|| format!("reading {}", src.display()))?;
            let mut w = std::fs::File::create(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
            let mut buf = vec![0u8; 1 << 20];
            let mut since_check = 0u64;
            loop {
                let n = r.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                w.write_all(&buf[..n]).with_context(|| format!("writing {}", tmp.display()))?;
                job.bytes_done.fetch_add(n as u64, Ordering::Relaxed);
                since_check += n as u64;
                if since_check > 64 << 20 {
                    since_check = 0;
                    if job.cancelled() {
                        return Err(anyhow!(Cancelled));
                    }
                    check_space(content)?;
                }
            }
            w.flush()?;
            how = "copied";
        }
        if job.cancelled() {
            return Err(anyhow!(Cancelled));
        }
        job.state("moving", "putting the archive into place");
        std::fs::rename(&tmp, &dest).with_context(|| format!("moving into {}", dest.display()))?;
        if job.from_inbox {
            // the inbox copy is in place now (it was linked or copied)
            let done = content.join("Mods").join("installed");
            let _ = std::fs::create_dir_all(&done);
            let _ = std::fs::remove_file(done.join(&name));
            if std::fs::rename(src, done.join(&name)).is_err() {
                let _ = std::fs::remove_file(src);
            }
        }
    }
    job.bytes_done.store(size, Ordering::Relaxed);
    job.files_done.store(1, Ordering::Relaxed);
    if !cfg!(test) {
        crate::mount_archive(&dest);
    }
    crate::mods::record(content, &name, &[format!("{ARCHIVES}/{name}")]);
    let items: Vec<String> = plan.maps.iter().filter(|m| !m.aside).map(|m| m.dest.clone()).collect();
    let summary = format!("{} used in place ({how}) as {}/{} - {} in the lists now; the game reads it without unpacking", name, ARCHIVES, name, if items.is_empty() { "its content is".to_string() } else { format!("{} are", items.join(", ")) });
    job.set(|p| {
        p.installed = items.iter().map(|i| format!("{i} (in {ARCHIVES}/{name})")).collect();
        p.message = summary.clone();
        p.report.push(summary.clone());
    });
    Ok(())
}

/// What the page shows before an install: how big a source unpacks, what is free, and
/// whether it could be used in place.
#[derive(Debug, Clone, Serialize, Default)]
pub struct SourceInfo {
    pub is_archive: bool,
    pub is_zip: bool,
    pub files: u64,
    pub unpacked_bytes: u64,
    pub archive_bytes: u64,
    pub needed_bytes: u64,
    pub free_bytes: u64,
    pub fits: bool,
    /// Why it cannot be used in place (empty when it can).
    pub in_place: String,
    pub in_place_ok: bool,
    /// The mode `auto` will take.
    pub suggested: InstallMode,
}

/// Look at a source without installing it.
pub fn inspect(content: &Path, root: Option<&Path>, src: &Path) -> Result<SourceInfo> {
    let archive = archive_kind(src);
    let is_zip = archive == Some(ArchiveKind::Zip);
    let free = free_space(content).unwrap_or(u64::MAX);
    if archive.is_none() {
        return Ok(SourceInfo { free_bytes: free, fits: true, in_place: "only a .zip archive can be used in place".into(), suggested: InstallMode::Extract, ..Default::default() });
    }
    let job = Job { id: 0, source: src.to_path_buf(), mode: InstallMode::Auto, from_inbox: false, cancel: AtomicBool::new(false), progress: Mutex::new(Progress::default()), bytes_done: AtomicU64::new(0), files_done: AtomicU64::new(0), linked: Mutex::new(Vec::new()), stall_at: u64::MAX };
    let archive_bytes = std::fs::metadata(src).map(|m| m.len()).unwrap_or(0);
    if !is_zip {
        let (files, unpacked) = match archive.unwrap() {
            ArchiveKind::SevenZip => {
                let a = sevenz_rust2::ArchiveReader::open(src, sevenz_rust2::Password::empty()).with_context(|| format!("{} is not a readable 7z archive", src.display()))?;
                let entries = &a.archive().files;
                (entries.iter().filter(|e| e.has_stream && !e.is_directory).count() as u64,
                    entries.iter().filter(|e| e.has_stream && !e.is_directory).fold(0u64, |n, e| n.saturating_add(e.size)))
            }
            ArchiveKind::Rar => {
                let a = unrar_rs::RarArchive::open(std::fs::File::open(src)?)
                    .with_context(|| format!("{} is not a readable RAR archive", src.display()))?;
                let members: Vec<_> = a.entries().collect();
                let regular = members.iter().filter(|m| !m.is_directory && !m.is_symlink && !m.is_hardlink);
                let entries: Vec<_> = regular.collect();
                if entries.iter().any(|m| m.unpacked_size.is_none()) {
                    return Err(anyhow!("multi-volume RAR archive; add all parts and start with the first .rar file"));
                }
                (entries.len() as u64, entries.iter().fold(0u64, |n, m| n.saturating_add(m.unpacked_size.unwrap_or(0))))
            }
            ArchiveKind::Zip => unreachable!(),
        };
        let needed = unpacked.saturating_add(MIN_MARGIN);
        return Ok(SourceInfo { is_archive: true, is_zip: false, files, unpacked_bytes: unpacked, archive_bytes, needed_bytes: needed, free_bytes: free, fits: needed <= free, in_place: "7z and RAR archives must be unpacked".into(), in_place_ok: false, suggested: InstallMode::Extract });
    }
    let f = std::fs::File::open(src).with_context(|| format!("opening {}", src.display()))?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::with_capacity(ZIP_BUFFER, f)).with_context(|| format!("{} is not a readable zip archive", src.display()))?;
    let (entries, _) = list_zip(&mut z, &job)?;
    let source_name = src.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "mod".into());
    let plan = plan(&entries, &source_name, &Installed { content, root });
    let files: u64 = plan.maps.iter().map(|m| m.files.len() as u64).sum();
    let unpacked: u64 = plan.maps.iter().map(|m| m.bytes).sum();
    let needed = unpacked + (unpacked / 20).max(MIN_MARGIN);
    let layout = if plan.maps.is_empty() { Err(anyhow!("nothing in it the game would use")) } else { in_place_layout(src, &plan) };
    let fits = needed <= free;
    let in_place_ok = layout.is_ok();
    Ok(SourceInfo { is_archive: true, is_zip, files, unpacked_bytes: unpacked, archive_bytes, needed_bytes: needed, free_bytes: free, fits, in_place: layout.err().map(|e| format!("{e:#}")).unwrap_or_default(), in_place_ok, suggested: if !fits && in_place_ok { InstallMode::InPlace } else { InstallMode::Extract } })
}

fn check_space(content: &Path) -> Result<()> {
    if let Some(free) = free_space(content) {
        if free < ABORT_BELOW {
            return Err(anyhow!("the disk is almost full ({} free) - stopped unpacking and removed the partial files", gb(free)));
        }
    }
    Ok(())
}

/// `rel` under `base`, taking the spelling of folders that exist already (a mod's
/// `vehicles/foo/texture` goes into `Vehicles/Foo/Texture` on a case-sensitive disk).
fn case_path(base: &Path, rel: &str) -> PathBuf {
    let mut cur = base.to_path_buf();
    for comp in rel.split('/').filter(|c| !c.is_empty()) {
        let direct = cur.join(comp);
        if direct.exists() {
            cur = direct;
            continue;
        }
        let found = std::fs::read_dir(&cur).ok().and_then(|rd| rd.flatten().map(|e| e.file_name()).find(|n| n.to_string_lossy().eq_ignore_ascii_case(comp)));
        cur = match found {
            Some(n) => cur.join(n),
            None => direct,
        };
    }
    cur
}

/// Move the tree `from` to `to`: in one rename when `to` does not exist, else merged file
/// by file (files replace files of the same name). Returns how many files were replaced.
fn move_into(from: &Path, to: &Path) -> Result<usize> {
    if !to.exists() {
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p)?;
        }
        if std::fs::rename(from, to).is_ok() {
            return Ok(0);
        }
    }
    std::fs::create_dir_all(to)?;
    let mut replaced = 0;
    for e in std::fs::read_dir(from)?.flatten() {
        let name = e.file_name();
        let src = e.path();
        let dest = case_path(to, &name.to_string_lossy());
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            if dest.exists() && !dest.is_dir() {
                std::fs::remove_file(&dest)?;
            }
            replaced += move_into(&src, &dest)?;
        } else {
            if dest.is_dir() {
                // (a file of a mod never takes the place of a whole installed folder)
                return Err(anyhow!("{} is a folder; the mod's file of that name was not installed over it", dest.display()));
            } else if dest.exists() {
                replaced += 1;
            }
            if std::fs::rename(&src, &dest).is_err() {
                // a new file, never written through whatever `dest` was (a link)
                let _ = std::fs::remove_file(&dest);
                std::fs::copy(&src, &dest).with_context(|| format!("copying to {}", dest.display()))?;
            }
        }
    }
    Ok(replaced)
}

/// Packs in `Mods/waiting` whose buses are installed now.
pub fn waiting_ready(content: &Path, root: Option<&Path>) -> Vec<PathBuf> {
    let installed = Installed { content, root };
    let Ok(rd) = std::fs::read_dir(content.join("Mods").join(WAITING)) else { return Vec::new() };
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            let buses: Vec<String> = std::fs::read_dir(p.join("Vehicles")).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
            !buses.is_empty() && buses.iter().all(|b| installed.vehicle(b).is_some())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(names: &[&str]) -> Vec<Entry> {
        names.iter().enumerate().map(|(i, n)| Entry { rel: n.to_string(), size: 10, index: i }).collect()
    }

    fn dests(p: &Plan) -> Vec<(String, bool)> {
        let mut v: Vec<(String, bool)> = p.maps.iter().map(|m| (m.dest.clone(), m.aside)).collect();
        v.sort();
        v
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("omsi-launcher-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn plans() {
        let content = tmp("plans");
        std::fs::create_dir_all(content.join("Vehicles/MAN_SD202")).unwrap();
        let inst = Installed { content: &content, root: None };
        // an OMSI-style pack one folder down, with a read-me
        let p = plan(&entries(&["Pack/Vehicles/Foo/foo.bus", "Pack/Vehicles/Foo/Model/model.cfg", "Pack/Sceneryobjects/X/x.sco", "Pack/readme.txt"]), "Pack", &inst);
        assert_eq!(dests(&p), vec![("Sceneryobjects".into(), false), ("Vehicles/Foo".into(), false)]);
        assert!(p.notes.iter().any(|n| n.contains("1 file")), "{:?}", p.notes);
        // a lone bus folder: its Sound / Script / Texture are not the content folders
        let p = plan(&entries(&["Foo/foo.bus", "Foo/Sound/a.wav", "Foo/Script/a.osc", "Foo/Texture/a.dds"]), "Foo", &inst);
        assert_eq!(dests(&p), vec![("Vehicles/Foo".into(), false)]);
        assert_eq!(p.maps[0].files.len(), 4);
        assert!(p.maps[0].files.iter().any(|f| f.1 == "Sound/a.wav"));
        // a bus folder at the top of the archive
        let p = plan(&entries(&["foo.bus", "Model/model.cfg", "Sound/sound.cfg", "Script/main.osc", "Texture/a.dds"]), "MyBus", &inst);
        assert_eq!(dests(&p), vec![("Vehicles/MyBus".into(), false)]);
        // a map
        let p = plan(&entries(&["Ahlheim/global.cfg", "Ahlheim/tile_0_0.map", "Ahlheim/texture/a.dds"]), "AhlheimV5", &inst);
        assert_eq!(dests(&p), vec![("maps/Ahlheim".into(), false)]);
        // repaints: for an installed bus (any case) merged, for a missing one kept aside
        let p = plan(&entries(&["Vehicles/man_sd202/Texture/Repaint/x.dds", "Vehicles/man_sd202/Model/x.cti", "Vehicles/O305/Texture/y.dds"]), "Repaints", &inst);
        assert_eq!(dests(&p), vec![("Vehicles/MAN_SD202".into(), false), ("Vehicles/O305".into(), true)]);
        assert!(p.warnings.iter().any(|w| w.contains("O305")));
        // nothing recognisable
        let p = plan(&entries(&["docs/a.pdf", "b.txt"]), "Junk", &inst);
        assert!(p.maps.is_empty());
        let _ = std::fs::remove_dir_all(&content);
    }

    fn write_zip(path: &Path, files: &[(&str, usize)]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, size) in files {
            z.start_file(*name, o).unwrap();
            let data: Vec<u8> = (0..*size).map(|i| (i * 7 % 251) as u8).collect();
            z.write_all(&data).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn install_zip_moves_into_place_and_cleans_up() {
        let dir = tmp("zip");
        let content = dir.join("content");
        omsi_cfg::ensure_content_layout(&content).unwrap();
        let zip = dir.join("Foo Bus.zip");
        write_zip(&zip, &[("Foo/foo.bus", 100), ("Foo/Model/model.cfg", 2000), ("Foo/Texture/a.dds", 300_000), ("readme.txt", 10)]);
        let p = run_blocking(content.clone(), None, zip.clone(), InstallMode::Extract, None, false);
        assert_eq!(p.state, "done", "{p:?}");
        assert_eq!(p.installed, vec!["Vehicles/Foo".to_string()]);
        assert_eq!(std::fs::metadata(content.join("Vehicles/Foo/Texture/a.dds")).unwrap().len(), 300_000);
        assert!(!content.join(STAGING).exists(), "staging removed");
        // again: merged over the existing folder
        let p = run_blocking(content.clone(), None, zip.clone(), InstallMode::Extract, None, false);
        assert_eq!(p.state, "done", "{p:?}");
        assert!(p.report.iter().any(|l| l.contains("3 existing file(s) replaced")), "{:?}", p.report);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_zip_is_used_in_place() {
        let dir = tmp("inplace");
        let content = dir.join("content");
        omsi_cfg::ensure_content_layout(&content).unwrap();
        // laid out like OMSI 2 (one folder down, with a read-me beside it)
        let zip = dir.join("Big Map.zip");
        write_zip(&zip, &[("OMSI 2/maps/Big/global.cfg", 100), ("OMSI 2/Sceneryobjects/Big/x.sco", 2000), ("readme.txt", 10)]);
        let p = run_blocking(content.clone(), None, zip.clone(), InstallMode::InPlace, None, false);
        assert_eq!(p.state, "done", "{p:?}");
        assert_eq!(p.mode, InstallMode::InPlace);
        let placed = content.join(ARCHIVES).join("Big Map.zip");
        assert_eq!(std::fs::metadata(&placed).unwrap().len(), std::fs::metadata(&zip).unwrap().len());
        assert!(zip.exists(), "a linked or copied archive stays where it was");
        assert!(!content.join("maps/Big").exists(), "nothing unpacked");
        assert!(!content.join(STAGING).exists(), "staging removed");
        assert!(p.installed.iter().any(|i| i.starts_with("maps (in Archives/Big Map.zip)")), "{:?}", p.installed);
        // the game's reader finds the map in it
        let archive = omsi_cfg::vfs::ZipArchive::open(&placed).unwrap();
        assert_eq!(archive.prefix(), "OMSI 2/");
        // again: already there
        let p = run_blocking(content.clone(), None, zip.clone(), InstallMode::InPlace, None, false);
        assert_eq!(p.state, "done", "{p:?}");
        // a lone bus folder cannot be read in place: explicit in-place fails, auto unpacks
        let lone = dir.join("Foo Bus.zip");
        write_zip(&lone, &[("Foo/foo.bus", 100), ("Foo/Texture/a.dds", 3000)]);
        let p = run_blocking(content.clone(), None, lone.clone(), InstallMode::InPlace, None, false);
        assert_eq!(p.state, "failed", "{p:?}");
        assert!(p.message.contains("unpacked"), "{}", p.message);
        let p = run_blocking(content.clone(), None, lone.clone(), InstallMode::Auto, None, false);
        assert_eq!(p.state, "done", "{p:?}");
        assert_eq!(p.mode, InstallMode::Extract);
        assert!(content.join("Vehicles/Foo/foo.bus").exists());
        // what the page is told before
        let info = inspect(&content, None, &zip).unwrap();
        assert!(info.is_zip && info.in_place_ok && info.unpacked_bytes == 2100, "{info:?}");
        let info = inspect(&content, None, &lone).unwrap();
        assert!(!info.in_place_ok, "{info:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn downloaded_plugins_are_not_enabled() {
        let dir = tmp("plugins");
        let content = dir.join("content");
        omsi_cfg::ensure_content_layout(&content).unwrap();
        let zip = dir.join("Pack.zip");
        write_zip(&zip, &[("Vehicles/Foo/foo.bus", 10), ("Plugins/evil.dll", 100), ("Plugins/evil.opl", 10)]);
        let p = run_blocking(content.clone(), None, zip, InstallMode::Extract, None, false);
        assert_eq!(p.state, "done", "{p:?}");
        assert!(content.join("Vehicles/Foo/foo.bus").exists());
        assert!(!content.join("Plugins/evil.dll").exists(), "never where the game loads plugins");
        assert!(content.join("Mods").join(PLUGINS_HELD).join("evil.dll").exists());
        assert!(p.warnings.iter().any(|w| w.contains("not enabled")), "{:?}", p.warnings);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_never_replaces_an_installed_folder() {
        let dir = tmp("file-over-folder");
        let (from, to) = (dir.join("from"), dir.join("to"));
        std::fs::create_dir_all(to.join("Bus")).unwrap();
        std::fs::write(to.join("Bus").join("keep.bus"), b"x").unwrap();
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join("Bus"), b"a file").unwrap();
        assert!(move_into(&from, &to).is_err());
        assert!(to.join("Bus").join("keep.bus").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_removes_partial_files() {
        let dir = tmp("cancel");
        let content = dir.join("content");
        omsi_cfg::ensure_content_layout(&content).unwrap();
        let zip = dir.join("Big.zip");
        let files: Vec<(String, usize)> = (0..4).map(|i| (format!("Vehicles/Big/Texture/t{i}.dds"), 100)).collect();
        let refs: Vec<(&str, usize)> = files.iter().map(|(n, s)| (n.as_str(), *s)).collect();
        let mut with_bus = refs.clone();
        with_bus.push(("Vehicles/Big/big.bus", 10));
        write_zip(&zip, &with_bus);
        // Pause after a file is staged: a fast disk must not finish before the test
        // requests cancellation, and cancellation must clean up actual partial files.
        let job = start_inner(content.clone(), None, zip, InstallMode::Extract, false, 1);
        wait_for(&job, 1);
        assert_eq!(job.snapshot().files_done, 1);
        assert!(staging_dir(&content, job.id).exists());
        job.cancel();
        let p = wait_done(&job);
        assert_eq!(p.state, "cancelled", "{p:?}");
        assert!(!content.join("Vehicles/Big").exists());
        assert!(!content.join(STAGING).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An inbox folder of `n` textures and a bus file.
    fn inbox_bus(content: &Path, n: usize) -> PathBuf {
        let src = content.join("Mods").join("Big Bus");
        std::fs::create_dir_all(src.join("Texture")).unwrap();
        std::fs::write(src.join("big.bus"), b"[friendlyname]").unwrap();
        std::fs::write(src.join("readme.txt"), b"read me").unwrap();
        for i in 0..n {
            std::fs::write(src.join("Texture").join(format!("t{i}.dds")), [7u8; 64]).unwrap();
        }
        src
    }

    fn wait_for(job: &Job, files: u64) {
        while job.snapshot().files_done < files && job.snapshot().finished.is_none() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn wait_done(job: &Job) -> Progress {
        while job.snapshot().finished.is_none() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        job.snapshot()
    }

    #[cfg(unix)]
    fn links(p: &Path) -> u64 {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(p).unwrap().nlink()
    }

    /// A bus installed from the inbox and then deleted from Mods/installed leaves the
    /// lists: its folder goes to Mods/uninstalled (#819).
    #[test]
    fn a_bus_deleted_from_installed_is_uninstalled() {
        let dir = tmp("uninstall");
        let content = dir.join("content");
        omsi_cfg::ensure_content_layout(&content).unwrap();
        // a bus that was there before, which the mod only adds to, stays
        std::fs::create_dir_all(content.join("Vehicles/Old")).unwrap();
        let src = inbox_bus(&content, 3);
        let p = wait_done(&start_inner(content.clone(), None, src, InstallMode::Extract, true, 0));
        assert_eq!(p.state, "done", "{p:?}");
        assert!(content.join("Vehicles/Big Bus/big.bus").exists());
        assert!(content.join("Mods/installed/Big Bus").exists());
        // still in Mods/installed: nothing happens
        assert!(uninstall_removed_now(&content).is_empty());
        assert!(content.join("Vehicles/Big Bus/big.bus").exists());
        std::fs::remove_dir_all(content.join("Mods/installed/Big Bus")).unwrap();
        assert_eq!(uninstall_removed_now(&content), ["Big Bus"]);
        assert!(!content.join("Vehicles/Big Bus").exists());
        assert!(content.join("Mods").join(UNINSTALLED).join("Big Bus/Vehicles/Big Bus/big.bus").exists());
        assert!(content.join("Vehicles/Old").exists());
        // once only
        assert!(uninstall_removed_now(&content).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cancelled_inbox_folder_keeps_its_files() {
        let dir = tmp("inbox");
        let content = dir.join("content");
        omsi_cfg::ensure_content_layout(&content).unwrap();
        let src = inbox_bus(&content, 3000);
        let job = start_inner(content.clone(), None, src.clone(), InstallMode::Extract, true, 1500);
        wait_for(&job, 1500);
        assert_eq!(std::fs::read_dir(src.join("Texture")).unwrap().count(), 3000, "linked, not moved: the inbox is whole while the job runs");
        #[cfg(unix)]
        assert_eq!(links(&src.join("big.bus")), 2, "big.bus is linked into the staging folder");
        job.cancel();
        let p = wait_done(&job);
        assert_eq!(p.state, "cancelled", "{p:?}");
        assert_eq!(std::fs::read_dir(src.join("Texture")).unwrap().count(), 3000, "every file is there");
        #[cfg(unix)]
        assert_eq!(links(&src.join("big.bus")), 1, "the staged link is gone");
        assert!(src.join("big.bus").exists());
        assert!(!content.join("Vehicles/Big Bus").exists());
        assert!(!content.join(STAGING).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_killed_inbox_install_loses_nothing() {
        let dir = tmp("killed");
        let content = dir.join("content");
        omsi_cfg::ensure_content_layout(&content).unwrap();
        let src = inbox_bus(&content, 400);
        let job = start_inner(content.clone(), None, src.clone(), InstallMode::Extract, true, 200);
        wait_for(&job, 200);
        // the launcher "dies" half-way: its staging folder now belongs to a process that is
        // gone, and the next launcher's start-up removes it
        let staging = staging_dir(&content, job.id);
        let dead = content.join(STAGING).join(format!("999999-{}", job.id));
        std::fs::rename(&staging, &dead).unwrap();
        let removed = cleanup_stale(&dir.join("data"), Some(&content));
        assert!(removed.iter().any(|l| l.contains("999999")), "{removed:?}");
        assert!(!dead.exists());
        assert_eq!(std::fs::read_dir(src.join("Texture")).unwrap().count(), 400, "every inbox file is still there");
        assert_eq!(std::fs::read(src.join("big.bus")).unwrap(), b"[friendlyname]");
        job.cancel();
        wait_done(&job);
        assert_eq!(std::fs::read_dir(src.join("Texture")).unwrap().count(), 400);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_inbox_folder_is_installed_and_leaves_the_inbox() {
        let dir = tmp("inbox-done");
        let content = dir.join("content");
        omsi_cfg::ensure_content_layout(&content).unwrap();
        let src = inbox_bus(&content, 20);
        let job = start(content.clone(), None, src.clone(), InstallMode::Extract, true);
        let p = wait_done(&job);
        assert_eq!(p.state, "done", "{p:?}");
        assert_eq!(p.installed, vec!["Vehicles/Big Bus".to_string()]);
        assert_eq!(std::fs::read_dir(content.join("Vehicles/Big Bus/Texture")).unwrap().count(), 20);
        #[cfg(unix)]
        assert_eq!(links(&content.join("Vehicles/Big Bus/big.bus")), 1, "the inbox copy is gone");
        assert!(!src.exists(), "the inbox folder was cleared");
        // the installed files left the inbox; only the empty folders went to Mods/installed
        assert!(content.join("Mods/installed/Big Bus").is_dir());
        assert!(!content.join("Mods/installed/Big Bus/Texture/t0.dds").exists());
        assert!(!content.join(STAGING).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn safe_names() {
        assert_eq!(safe_rel("Vehicles/Foo/foo.bus").as_deref(), Some("Vehicles/Foo/foo.bus"));
        assert_eq!(safe_rel("Vehicles\\Foo\\./foo.bus").as_deref(), Some("Vehicles/Foo/foo.bus"));
        assert_eq!(safe_rel("Foo//bar/").as_deref(), Some("Foo/bar"));
        for bad in ["..\\..\\..\\evil", "a/..\\..\\b", "../x", "a/../b", "/etc/passwd", "\\\\server\\share\\x", "C:\\x", "C:x", "Foo/C:evil/x.bus", "a\0b", "", "./."] {
            assert_eq!(safe_rel(bad), None, "{bad:?}");
        }
        assert!(check_rel("Vehicles/Foo").is_ok());
        assert!(check_rel("Vehicles/../Foo").is_err());
        assert!(check_rel("maps/..").is_err());
    }

    #[test]
    fn backslash_traversal_in_a_zip_is_refused() {
        let dir = tmp("zipslip");
        let content = dir.join("content");
        omsi_cfg::ensure_content_layout(&content).unwrap();
        // files are unpacked into dir/content/.install-staging/<pid>-<job>/0: four levels up
        // is `dir`, so this name would land in `dir/evil.txt`
        let zip = dir.join("Evil Map.zip");
        write_zip(&zip, &[("global.cfg", 10), ("..\\..\\..\\..\\evil.txt", 10), ("tile_0_0.map", 10)]);
        let p = run_blocking(content.clone(), None, zip, InstallMode::Extract, None, false);
        assert_eq!(p.state, "failed", "{p:?}");
        assert!(p.message.contains("refused"), "{}", p.message);
        assert!(!dir.join("evil.txt").exists(), "nothing written outside the staging folder");
        assert!(!content.join("maps/Evil Map").exists());
        assert!(!content.join(STAGING).exists());
        // the same inside a bus folder
        let zip = dir.join("Evil Bus.zip");
        write_zip(&zip, &[("Vehicles/Foo/foo.bus", 10), ("Vehicles/Foo/..\\..\\..\\..\\evil.txt", 10)]);
        let p = run_blocking(content.clone(), None, zip, InstallMode::Extract, None, false);
        assert_eq!(p.state, "failed", "{p:?}");
        assert!(!dir.join("evil.txt").exists());
        // a zip made on Windows with `\` separators still installs
        let zip = dir.join("Win Bus.zip");
        write_zip(&zip, &[("Vehicles\\Bar\\bar.bus", 10), ("Vehicles\\Bar\\Texture\\a.dds", 100)]);
        let p = run_blocking(content.clone(), None, zip, InstallMode::Extract, None, false);
        assert_eq!(p.state, "done", "{p:?}");
        assert_eq!(std::fs::metadata(content.join("Vehicles/Bar/Texture/a.dds")).unwrap().len(), 100);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn backslash_names_in_a_folder_stay_inside() {
        let dir = tmp("folderslip");
        let content = dir.join("content");
        omsi_cfg::ensure_content_layout(&content).unwrap();
        let src = dir.join("Odd Bus");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("odd.bus"), b"[friendlyname]").unwrap();
        // one file whose name is a whole Windows path, one that tries to climb out
        std::fs::write(src.join("Texture\\a.dds"), [1u8; 32]).unwrap();
        std::fs::write(src.join("..\\..\\..\\..\\evil.txt"), b"x").unwrap();
        let p = run_blocking(content.clone(), None, src.clone(), InstallMode::Extract, None, false);
        assert_eq!(p.state, "done", "{p:?}");
        assert!(p.warnings.iter().any(|w| w.contains("1 file(s)")), "{:?}", p.warnings);
        assert_eq!(std::fs::metadata(content.join("Vehicles/Odd Bus/Texture/a.dds")).unwrap().len(), 32);
        assert!(!dir.join("evil.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_staging_is_removed() {
        let dir = tmp("stale");
        let content = dir.join("content");
        let data = dir.join("data");
        std::fs::create_dir_all(content.join(STAGING).join("999999-3/0")).unwrap();
        std::fs::write(content.join(STAGING).join("999999-3/0/x"), b"partial").unwrap();
        std::fs::create_dir_all(data.join("unzip/Old")).unwrap();
        let out = cleanup_stale(&data, Some(&content));
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(!content.join(STAGING).exists());
        assert!(!data.join("unzip").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn free_space_is_known() {
        assert!(free_space(&std::env::temp_dir()).unwrap_or(0) > 0);
    }
}
