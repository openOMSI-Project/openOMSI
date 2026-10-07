//! OMSI plugins, driven the way OMSI drives them:
//!
//! * the original finds every `*.opl` under `<OMSI>\plugins` (recursively) and reads
//!   `[dll]` (the library, relative to `plugins\`), `[varlist]`, `[stringvarlist]`,
//!   `[systemvarlist]` and `[triggers]` - each a count, then that many names.
//! * the original loads the library and looks up `PluginStart`, `PluginFinalize` (both
//!   required: without them the plugin is not loaded) and `AccessVariable`,
//!   `AccessTrigger`, `AccessSystemVariable`, `AccessStringVariable` (each optional, a
//!   warning when missing), then calls `PluginStart(AOwner)`. `PluginFinalize` is called
//!   when the game ends.
//! * Every frame, for each plugin in turn:
//!   every system variable of its list (`AccessSystemVariable(index, var value: Single,
//!   var write: Boolean)`, the value written back when `write` comes back true); then,
//!   with a player vehicle, its vehicle variables (`AccessVariable`, the same shape), its
//!   string variables (`AccessStringVariable(index, PWideChar, var write)`: a buffer of
//!   length + 1 wide characters holding the text and its terminating zero, read back when
//!   `write` is set) and its triggers (`AccessTrigger(index, var active: Boolean)`, starting
//!   from false each frame: a change from the last frame's state is a key going down -
//!   the trigger fires - or coming up - `<trigger>_off`). Names nobody knows are skipped.
//!   All are `stdcall`; the index is the position in the plugin's own list (a Word).
//!
//! A plugin is a Windows DLL, almost always 32-bit. A library the running process can load
//! (same system and architecture) is loaded in-process; any other runs in
//! `omsi-plugin-host`, a small program built for 32-bit Windows that loads the DLL and
//! answers over its standard input and output (started directly on Windows and through
//! Wine elsewhere). The frame is one round trip.

pub mod lua;
pub mod ui;

use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// One `.opl` file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Opl {
    /// `[dll]`: the library, relative to the plugins folder.
    pub dll: String,
    pub vars: Vec<String>,
    pub string_vars: Vec<String>,
    pub system_vars: Vec<String>,
    pub triggers: Vec<String>,
}

/// Read an `.opl` file's text: the tags OMSI knows, each list a count and that many
/// lines (a count that is no number reads as 0, as `StrToInt` would refuse it).
pub fn parse_opl(text: &str) -> Opl {
    let mut o = Opl::default();
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));
    while let Some(l) = lines.next() {
        let list = match l.trim() {
            "[dll]" => {
                o.dll = lines.next().unwrap_or("").trim().to_string();
                continue;
            }
            "[varlist]" => &mut o.vars,
            "[stringvarlist]" => &mut o.string_vars,
            "[systemvarlist]" => &mut o.system_vars,
            "[triggers]" => &mut o.triggers,
            _ => continue,
        };
        let n: usize = lines.next().and_then(|c| c.trim().parse().ok()).unwrap_or(0);
        for _ in 0..n {
            match lines.next() {
                Some(name) => list.push(name.trim().to_string()),
                None => break,
            }
        }
    }
    o
}

/// Every `.opl` under `dir` (a folder's subfolders before its files, names compared without
/// case - the order FindFilesRecursive gives).
pub fn find_opls(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_default());
    for p in entries.iter().filter(|p| p.is_dir()) {
        out.extend(find_opls(p));
    }
    for p in entries.iter().filter(|p| p.is_file()) {
        if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("opl")) {
            out.push(p.clone());
        }
    }
    out
}

/// A file name as Windows finds it: the path as given, or else the entry of each folder
/// that equals it without regard to case.
pub fn resolve_path(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = base.to_path_buf();
    for part in rel.split(['\\', '/']).filter(|s| !s.is_empty()) {
        let direct = cur.join(part);
        if direct.exists() {
            cur = direct;
            continue;
        }
        let found = std::fs::read_dir(&cur).ok()?.flatten().map(|e| e.path()).find(|p| {
            p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(part))
        })?;
        cur = found;
    }
    Some(cur)
}

/// What one frame hands a plugin and what it gives back.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    /// (index in the plugin's list, value); values written back replace them.
    pub system: Vec<(u16, f32)>,
    pub vars: Vec<(u16, f32)>,
    pub strings: Vec<(u16, String)>,
    /// Indices of the triggers asked; `triggers_active` comes back in the same order.
    pub triggers: Vec<u16>,
}

/// The answers of a frame: per entry of the `Frame`, whether the plugin wrote it and the value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reply {
    pub system: Vec<Option<f32>>,
    pub vars: Vec<Option<f32>>,
    pub strings: Vec<Option<String>>,
    pub triggers_active: Vec<bool>,
}

/// Which of the optional procedures the library has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Procs {
    pub variable: bool,
    pub trigger: bool,
    pub system: bool,
    pub string: bool,
}

type StartFn = unsafe extern "system" fn(*mut std::ffi::c_void);
type FinalizeFn = unsafe extern "system" fn();
type AccessFloatFn = unsafe extern "system" fn(u16, *mut f32, *mut u8);
type AccessStringFn = unsafe extern "system" fn(u16, *mut u16, *mut u8);
type AccessTriggerFn = unsafe extern "system" fn(u16, *mut u8);

/// A plugin library loaded into this process.
pub struct Library {
    _lib: libloading::Library,
    start: StartFn,
    finalize: FinalizeFn,
    variable: Option<AccessFloatFn>,
    trigger: Option<AccessTriggerFn>,
    system: Option<AccessFloatFn>,
    string: Option<AccessStringFn>,
}

impl Library {
    /// Load the library and look its procedures up.
    pub fn load(path: &Path) -> Result<Library, String> {
        // SAFETY: loading a library runs its initialisers; that is what a plugin is for
        let lib = unsafe { libloading::Library::new(path) }.map_err(|e| {
            // (libloading 0.9 keeps the system's own reason - dlerror's text - in `source`)
            use std::error::Error as _;
            match e.source() {
                Some(why) => format!("LoadLibrary failed: {e}: {why}"),
                None => format!("LoadLibrary failed: {e}"),
            }
        })?;
        unsafe {
            let start = *lib.get::<StartFn>(b"PluginStart\0").map_err(|_| "procedure \"PluginStart\" not found".to_string())?;
            let finalize = *lib.get::<FinalizeFn>(b"PluginFinalize\0").map_err(|_| "procedure \"PluginFinalize\" not found".to_string())?;
            let variable = lib.get::<AccessFloatFn>(b"AccessVariable\0").ok().map(|s| *s);
            let trigger = lib.get::<AccessTriggerFn>(b"AccessTrigger\0").ok().map(|s| *s);
            let system = lib.get::<AccessFloatFn>(b"AccessSystemVariable\0").ok().map(|s| *s);
            let string = lib.get::<AccessStringFn>(b"AccessStringVariable\0").ok().map(|s| *s);
            Ok(Library { _lib: lib, start, finalize, variable, trigger, system, string })
        }
    }

    pub fn procs(&self) -> Procs {
        Procs { variable: self.variable.is_some(), trigger: self.trigger.is_some(), system: self.system.is_some(), string: self.string.is_some() }
    }

    /// `PluginStart(AOwner)`; there is no Delphi application to own the plugin's forms here.
    pub fn start(&self) {
        unsafe { (self.start)(std::ptr::null_mut()) }
    }

    pub fn finalize(&self) {
        unsafe { (self.finalize)() }
    }

    /// One frame's calls, in OMSI's order.
    pub fn frame(&self, f: &Frame) -> Reply {
        let mut r = Reply::default();
        let float = |func: Option<AccessFloatFn>, list: &[(u16, f32)]| -> Vec<Option<f32>> {
            list.iter()
                .map(|&(i, v)| {
                    let Some(func) = func else { return None };
                    let (mut value, mut write) = (v, 0u8);
                    unsafe { func(i, &mut value, &mut write) };
                    (write != 0).then_some(value)
                })
                .collect()
        };
        r.system = float(self.system, &f.system);
        r.vars = float(self.variable, &f.vars);
        r.strings = f
            .strings
            .iter()
            .map(|(i, s)| {
                let func = self.string?;
                // (OMSI hands over exactly length + 1 characters; a plugin writing a
                // longer text overran the heap there. Here the buffer has room to spare,
                // zeroed, and only up to its first zero is read back)
                let mut buf: Vec<u16> = s.encode_utf16().collect();
                let size = (buf.len() + 1).max(STRING_BUFFER);
                buf.resize(size, 0);
                let mut write = 0u8;
                unsafe { func(*i, buf.as_mut_ptr(), &mut write) };
                if write == 0 {
                    return None;
                }
                let end = buf.iter().position(|&c| c == 0).unwrap_or(size);
                Some(String::from_utf16_lossy(&buf[..end]))
            })
            .collect();
        r.triggers_active = f
            .triggers
            .iter()
            .map(|&i| {
                let Some(func) = self.trigger else { return false };
                let mut active = 0u8;
                unsafe { func(i, &mut active) };
                active != 0
            })
            .collect();
        r
    }
}

/// Wide characters a plugin's string variable buffer has at least (see `frame`).
const STRING_BUFFER: usize = 4096;

// --- the host protocol: little-endian, one request and one answer at a time ---

pub mod wire {
    use super::*;

    pub const START: u8 = 1;
    pub const FRAME: u8 = 2;
    pub const FINALIZE: u8 = 3;

    pub fn put_u8(w: &mut impl Write, v: u8) -> std::io::Result<()> {
        w.write_all(&[v])
    }
    pub fn put_u16(w: &mut impl Write, v: u16) -> std::io::Result<()> {
        w.write_all(&v.to_le_bytes())
    }
    pub fn put_f32(w: &mut impl Write, v: f32) -> std::io::Result<()> {
        w.write_all(&v.to_le_bytes())
    }
    pub fn put_str(w: &mut impl Write, s: &str) -> std::io::Result<()> {
        let u: Vec<u16> = s.encode_utf16().collect();
        put_u16(w, u.len().min(u16::MAX as usize) as u16)?;
        for c in u.iter().take(u16::MAX as usize) {
            put_u16(w, *c)?;
        }
        Ok(())
    }
    pub fn get_u8(r: &mut impl Read) -> std::io::Result<u8> {
        let mut b = [0u8; 1];
        r.read_exact(&mut b)?;
        Ok(b[0])
    }
    pub fn get_u16(r: &mut impl Read) -> std::io::Result<u16> {
        let mut b = [0u8; 2];
        r.read_exact(&mut b)?;
        Ok(u16::from_le_bytes(b))
    }
    pub fn get_f32(r: &mut impl Read) -> std::io::Result<f32> {
        let mut b = [0u8; 4];
        r.read_exact(&mut b)?;
        Ok(f32::from_le_bytes(b))
    }
    pub fn get_str(r: &mut impl Read) -> std::io::Result<String> {
        let n = get_u16(r)? as usize;
        let mut u = Vec::with_capacity(n);
        for _ in 0..n {
            u.push(get_u16(r)?);
        }
        Ok(String::from_utf16_lossy(&u))
    }

    pub fn put_frame(w: &mut impl Write, f: &Frame) -> std::io::Result<()> {
        put_u16(w, f.system.len() as u16)?;
        for (i, v) in &f.system {
            put_u16(w, *i)?;
            put_f32(w, *v)?;
        }
        put_u16(w, f.vars.len() as u16)?;
        for (i, v) in &f.vars {
            put_u16(w, *i)?;
            put_f32(w, *v)?;
        }
        put_u16(w, f.strings.len() as u16)?;
        for (i, s) in &f.strings {
            put_u16(w, *i)?;
            put_str(w, s)?;
        }
        put_u16(w, f.triggers.len() as u16)?;
        for i in &f.triggers {
            put_u16(w, *i)?;
        }
        Ok(())
    }

    pub fn get_frame(r: &mut impl Read) -> std::io::Result<Frame> {
        let mut f = Frame::default();
        for _ in 0..get_u16(r)? {
            f.system.push((get_u16(r)?, get_f32(r)?));
        }
        for _ in 0..get_u16(r)? {
            f.vars.push((get_u16(r)?, get_f32(r)?));
        }
        for _ in 0..get_u16(r)? {
            f.strings.push((get_u16(r)?, get_str(r)?));
        }
        for _ in 0..get_u16(r)? {
            f.triggers.push(get_u16(r)?);
        }
        Ok(f)
    }

    pub fn put_reply(w: &mut impl Write, r: &Reply) -> std::io::Result<()> {
        for v in r.system.iter().chain(&r.vars) {
            put_u8(w, v.is_some() as u8)?;
            put_f32(w, v.unwrap_or(0.0))?;
        }
        for s in &r.strings {
            put_u8(w, s.is_some() as u8)?;
            put_str(w, s.as_deref().unwrap_or(""))?;
        }
        for a in &r.triggers_active {
            put_u8(w, *a as u8)?;
        }
        Ok(())
    }

    pub fn get_reply(rd: &mut impl Read, f: &Frame) -> std::io::Result<Reply> {
        let mut r = Reply::default();
        let mut float = |n: usize| -> std::io::Result<Vec<Option<f32>>> {
            (0..n).map(|_| Ok(if get_u8(rd)? != 0 { Some(get_f32(rd)?) } else { get_f32(rd)?; None })).collect()
        };
        r.system = float(f.system.len())?;
        r.vars = float(f.vars.len())?;
        for _ in 0..f.strings.len() {
            let w = get_u8(rd)? != 0;
            let s = get_str(rd)?;
            r.strings.push(w.then_some(s));
        }
        for _ in 0..f.triggers.len() {
            r.triggers_active.push(get_u8(rd)? != 0);
        }
        Ok(r)
    }
}

/// A plugin library in `omsi-plugin-host`.
pub struct Remote {
    child: Child,
    to: BufWriter<ChildStdin>,
    from: BufReader<ChildStdout>,
    procs: Procs,
}

impl Remote {
    /// Start `host` (with `runner`, e.g. `wine`, in front when given) on the library and
    /// have it call `PluginStart`.
    pub fn spawn(runner: Option<&Path>, host: &Path, dll: &Path) -> Result<Remote, String> {
        let mut cmd = match runner {
            Some(r) => {
                let mut c = Command::new(r);
                c.arg(host);
                c
            }
            None => Command::new(host),
        };
        cmd.arg(dll).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        if let Some(dir) = dll.parent() {
            cmd.current_dir(dir);
        }
        let mut child = cmd.spawn().map_err(|e| format!("could not start {}: {e}", host.display()))?;
        let to = BufWriter::new(child.stdin.take().ok_or("no stdin")?);
        let from = BufReader::new(child.stdout.take().ok_or("no stdout")?);
        let mut r = Remote { child, to, from, procs: Procs::default() };
        let answer = (|| -> std::io::Result<(u8, u8)> {
            wire::put_u8(&mut r.to, wire::START)?;
            r.to.flush()?;
            Ok((wire::get_u8(&mut r.from)?, wire::get_u8(&mut r.from)?))
        })();
        match answer {
            Ok((1, flags)) => {
                r.procs = Procs { variable: flags & 1 != 0, trigger: flags & 2 != 0, system: flags & 4 != 0, string: flags & 8 != 0 };
                Ok(r)
            }
            Ok(_) => {
                let _ = r.child.kill();
                Err(format!("the host could not load {}", dll.display()))
            }
            Err(e) => {
                let _ = r.child.kill();
                Err(format!("the host for {} ended: {e}", dll.display()))
            }
        }
    }

    pub fn procs(&self) -> Procs {
        self.procs
    }

    pub fn frame(&mut self, f: &Frame) -> std::io::Result<Reply> {
        wire::put_u8(&mut self.to, wire::FRAME)?;
        wire::put_frame(&mut self.to, f)?;
        self.to.flush()?;
        wire::get_reply(&mut self.from, f)
    }

    pub fn finalize(&mut self) {
        let _ = wire::put_u8(&mut self.to, wire::FINALIZE).and_then(|_| self.to.flush());
        let _ = wire::get_u8(&mut self.from);
        let _ = self.child.wait();
    }
}

/// How a plugin runs.
pub enum Backend {
    Local(Library),
    Remote(Remote),
}

/// A loaded plugin: its `.opl` and its library.
pub struct Plugin {
    pub opl_path: PathBuf,
    pub opl: Opl,
    backend: Backend,
    procs: Procs,
    /// Each trigger's state after the last frame.
    trigger_state: Vec<bool>,
    failed: bool,
}

/// Where to look for the out-of-process host and what runs it.
#[derive(Debug, Clone, Default)]
pub struct HostConfig {
    /// `omsi-plugin-host` built for 32-bit Windows (`omsi-plugin-host32.exe`).
    pub host32: Option<PathBuf>,
    /// The program that runs Windows executables here (`wine`); none on Windows.
    pub runner: Option<PathBuf>,
}

impl HostConfig {
    /// The host next to the running program and, off Windows, `wine` from the path.
    pub fn detect() -> HostConfig {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
        let host32 = std::env::var_os("OMSI_PLUGIN_HOST32")
            .map(PathBuf::from)
            .or_else(|| exe_dir.map(|d| d.join("omsi-plugin-host32.exe")))
            .filter(|p| p.is_file());
        let runner = if cfg!(windows) {
            None
        } else {
            std::env::var_os("OMSI_WINE").map(PathBuf::from).or_else(|| {
                // the path, then where Homebrew and the Wine app bundles put it (a game
                // started from Finder gets a path without /opt/homebrew/bin)
                let from_path: Vec<PathBuf> = std::env::var_os("PATH")
                    .map(|paths| std::env::split_paths(&paths).map(|d| d.join("wine")).collect())
                    .unwrap_or_default();
                from_path
                    .into_iter()
                    .chain(
                        [
                            "/opt/homebrew/bin/wine",
                            "/usr/local/bin/wine",
                            "/Applications/Wine Stable.app/Contents/Resources/wine/bin/wine",
                            "/Applications/Wine Devel.app/Contents/Resources/wine/bin/wine",
                            "/Applications/Wine Staging.app/Contents/Resources/wine/bin/wine",
                            "/usr/bin/wine",
                        ]
                        .map(PathBuf::from),
                    )
                    .find(|p| p.is_file())
            })
        };
        HostConfig { host32, runner }
    }
}

impl Plugin {
    /// Load the plugin an `.opl` describes: in this process when its library loads here,
    /// else in the 32-bit host.
    pub fn load(opl_path: &Path, plugins_dir: &Path, hosts: &HostConfig) -> Result<Plugin, String> {
        let text = std::fs::read(opl_path).map_err(|e| e.to_string())?;
        let opl = parse_opl(&String::from_utf8_lossy(&text));
        if opl.dll.is_empty() {
            return Err("no [dll]".into());
        }
        let dll = resolve_path(plugins_dir, &opl.dll).ok_or_else(|| format!("{} not found", opl.dll))?;
        let (backend, procs) = match Library::load(&dll) {
            Ok(lib) => {
                lib.start();
                let p = lib.procs();
                (Backend::Local(lib), p)
            }
            Err(local) => {
                let Some(host) = &hosts.host32 else {
                    return Err(format!("{local}; a 32-bit Windows plugin needs omsi-plugin-host32.exe next to the game"));
                };
                if !cfg!(windows) && hosts.runner.is_none() {
                    return Err(format!("{local}; a Windows plugin needs Wine here (or OMSI_WINE)"));
                }
                let r = Remote::spawn(hosts.runner.as_deref(), host, &dll)?;
                let p = r.procs();
                (Backend::Remote(r), p)
            }
        };
        for (has, name) in [(procs.variable, "AccessVariable"), (procs.trigger, "AccessTrigger"), (procs.system, "AccessSystemVariable"), (procs.string, "AccessStringVariable")] {
            if !has {
                log::warn!("Loading plugin {}: procedure \"{name}\" not found!", opl.dll);
            }
        }
        let n = opl.triggers.len();
        Ok(Plugin { opl_path: opl_path.to_path_buf(), opl, backend, procs, trigger_state: vec![false; n], failed: false })
    }

    pub fn procs(&self) -> Procs {
        self.procs
    }

    /// Run one frame. `system` / `var` / `string` read a value by name (None: no such
    /// variable, skipped); the writes and trigger edges come back through `set_*` and `fire`.
    pub fn frame(&mut self, io: &mut dyn PluginIo) {
        if self.failed {
            return;
        }
        let mut f = Frame::default();
        let mut sys_names = Vec::new();
        if self.procs.system {
            for (i, n) in self.opl.system_vars.iter().enumerate() {
                if let Some(v) = io.system(n) {
                    f.system.push((i as u16, v));
                    sys_names.push(n.clone());
                }
            }
        }
        let vehicle = io.has_vehicle();
        let (mut var_names, mut str_names, mut trig_idx) = (Vec::new(), Vec::new(), Vec::new());
        if vehicle {
            if self.procs.variable {
                for (i, n) in self.opl.vars.iter().enumerate() {
                    if let Some(v) = io.var(n) {
                        f.vars.push((i as u16, v));
                        var_names.push(n.clone());
                    }
                }
            }
            if self.procs.string {
                for (i, n) in self.opl.string_vars.iter().enumerate() {
                    if let Some(s) = io.string(n) {
                        f.strings.push((i as u16, s));
                        str_names.push(n.clone());
                    }
                }
            }
            if self.procs.trigger {
                for i in 0..self.opl.triggers.len() {
                    f.triggers.push(i as u16);
                    trig_idx.push(i);
                }
            }
        }
        let reply = match &mut self.backend {
            Backend::Local(lib) => lib.frame(&f),
            Backend::Remote(r) => match r.frame(&f) {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("plugin {}: the host stopped answering ({e}); it is left out from now on", self.opl.dll);
                    self.failed = true;
                    return;
                }
            },
        };
        for (n, v) in sys_names.iter().zip(&reply.system) {
            if let Some(v) = v {
                io.set_system(n, *v);
            }
        }
        for (n, v) in var_names.iter().zip(&reply.vars) {
            if let Some(v) = v {
                io.set_var(n, *v);
            }
        }
        for (n, s) in str_names.iter().zip(&reply.strings) {
            if let Some(s) = s {
                io.set_string(n, s);
            }
        }
        for (&i, &active) in trig_idx.iter().zip(&reply.triggers_active) {
            if self.trigger_state[i] != active {
                io.fire(&self.opl.triggers[i], active);
                self.trigger_state[i] = active;
            }
        }
    }

    /// `PluginFinalize`.
    pub fn finalize(&mut self) {
        match &mut self.backend {
            Backend::Local(lib) => lib.finalize(),
            Backend::Remote(r) => r.finalize(),
        }
    }
}

/// The game's side of a frame.
pub trait PluginIo {
    fn system(&mut self, name: &str) -> Option<f32>;
    fn set_system(&mut self, name: &str, v: f32);
    fn has_vehicle(&self) -> bool;
    fn var(&mut self, name: &str) -> Option<f32>;
    fn set_var(&mut self, name: &str, v: f32);
    fn string(&mut self, name: &str) -> Option<String>;
    fn set_string(&mut self, name: &str, s: &str);
    /// A trigger's key went down (`true`) or came up.
    fn fire(&mut self, trigger: &str, down: bool);
    /// Seconds of game time since the last frame (Lua plugins' timers).
    fn dt(&self) -> f32 {
        0.0
    }
    /// The player's vehicle's name (Lua plugins).
    fn vehicle_name(&self) -> Option<String> {
        None
    }
    /// The player's vehicle's manufacturer and model apart, as its `[friendlyname]` has them
    /// (Lua plugins; the name is the two joined).
    fn vehicle_manufacturer_model(&self) -> Option<(String, String)> {
        None
    }
    /// The player's vehicle: x, y, z and heading in degrees (Lua plugins).
    fn position(&self) -> Option<[f64; 4]> {
        None
    }
    /// A line of text on the screen for `seconds` (Lua plugins).
    fn message(&mut self, _text: &str, _seconds: f32) {}
    /// What the game is doing, as (key, value) pairs for `omsi.info()` (Lua plugins): the
    /// map, the clock, the duty, the view... Values are numbers or text.
    fn info(&self) -> Vec<(&'static str, InfoValue)> {
        Vec::new()
    }
    /// A game action by its game-menu id (`refuel`, `shot`, ...), run after the frame
    /// (Lua plugins). False when the game does not know it.
    fn command(&mut self, _what: &str) -> bool {
        false
    }
    /// The names of the player's bus's script variables and string variables (Lua plugins).
    fn var_names(&self) -> (Vec<String>, Vec<String>) {
        (Vec::new(), Vec::new())
    }
    /// Keys pressed (true) and let go since the last frame, by winit's key name (Lua plugins).
    fn keys(&self) -> Vec<(String, bool)> {
        Vec::new()
    }
    /// The other vehicles within `radius` m of the player's (Lua plugins' `omsi.others`):
    /// the AI traffic and the other LAN players' buses.
    fn others(&self, _radius: f64) -> Vec<Other> {
        Vec::new()
    }
    /// A variable of one of [`PluginIo::others`] by its id.
    fn other_var(&mut self, _id: u64, _name: &str) -> Option<f32> {
        None
    }
    /// Writes a variable of one of [`PluginIo::others`] (an AI vehicle's; another player's
    /// bus takes its values from the network again).
    fn set_other_var(&mut self, _id: u64, _name: &str, _v: f32) -> bool {
        false
    }
    /// What happened in the game since the last plugin frame, each sent to Lua plugins as an
    /// event (`crash`, `pedestrian`, `stops_skipped`): things `omsi.info()` cannot show, as
    /// they are over before a plugin could look. Every plugin of the frame gets them all.
    fn events(&self) -> Vec<GameEvent> {
        Vec::new()
    }
}

/// One of [`PluginIo::events`]: a Lua event of that name, called with these values.
#[derive(Debug, Clone, PartialEq)]
pub struct GameEvent {
    pub name: &'static str,
    pub args: Vec<InfoValue>,
}

/// One of [`PluginIo::others`].
#[derive(Debug, Clone, PartialEq)]
pub struct Other {
    /// Stable while the vehicle is there: AI cars by their id, LAN players by theirs.
    pub id: u64,
    /// "ai" or "player".
    pub kind: &'static str,
    /// Manufacturer and type, as `omsi.vehicle()` gives the player's.
    pub name: String,
    /// x, y, z and heading in degrees, as `omsi.position()`.
    pub pos: [f64; 4],
}

/// A value of [`PluginIo::info`].
#[derive(Debug, Clone, PartialEq)]
pub enum InfoValue {
    Num(f64),
    Text(String),
    Bool(bool),
}

/// Every plugin of the plugins folders.
#[derive(Default)]
pub struct Plugins {
    pub loaded: Vec<Plugin>,
    /// The Lua plugins (`plugins/*.lua`, `plugins/<name>/main.lua`).
    pub lua: Vec<lua::LuaPlugin>,
    /// What the Lua plugins show on the screen (`omsi.ui`), for the game to draw.
    pub ui: ui::SharedUi,
}

impl Plugins {
    /// Load the plugins of each `plugins` folder given (the first folder's copy of an
    /// `.opl` name wins).
    pub fn load(dirs: &[PathBuf], hosts: &HostConfig) -> Plugins {
        let mut loaded: Vec<Plugin> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for dir in dirs {
            for opl in find_opls(dir) {
                let key = opl.strip_prefix(dir).unwrap_or(&opl).to_string_lossy().to_ascii_lowercase();
                if !seen.insert(key) {
                    continue;
                }
                match Plugin::load(&opl, dir, hosts) {
                    Ok(p) => {
                        log::info!("plugin {} loaded ({} variables, {} strings, {} system variables, {} triggers)", p.opl.dll, p.opl.vars.len(), p.opl.string_vars.len(), p.opl.system_vars.len(), p.opl.triggers.len());
                        loaded.push(p);
                    }
                    Err(e) => log::warn!("Could not load plugin {}: {e}", opl.display()),
                }
            }
        }
        let mut lua = Vec::new();
        let ui = ui::SharedUi::default();
        let mut seen = std::collections::HashSet::new();
        for dir in dirs {
            for path in lua::find_lua(dir) {
                let key = path.strip_prefix(dir).unwrap_or(&path).to_string_lossy().to_ascii_lowercase();
                if !seen.insert(key) {
                    continue;
                }
                match lua::LuaPlugin::load_with_ui(&path, &mut lua::NoVehicle, ui.clone()) {
                    Ok(p) => {
                        log::info!("Lua plugin {} loaded ({})", p.name, path.display());
                        lua.push(p);
                    }
                    Err(e) => log::warn!("Could not load Lua plugin {}: {e}", path.display()),
                }
            }
        }
        Plugins { loaded, lua, ui }
    }

    pub fn is_empty(&self) -> bool {
        self.loaded.is_empty() && self.lua.is_empty()
    }

    pub fn frame(&mut self, io: &mut dyn PluginIo) {
        for p in &mut self.loaded {
            p.frame(io);
        }
        for p in &mut self.lua {
            p.frame(io);
        }
    }

    pub fn finalize(&mut self) {
        for p in &mut self.loaded {
            p.finalize();
        }
        self.loaded.clear();
        for p in &mut self.lua {
            p.stop(&mut lua::NoVehicle);
        }
        self.lua.clear();
    }
}

impl Drop for Plugins {
    fn drop(&mut self) {
        self.finalize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opl_lists() {
        let o = parse_opl("[dll]\r\nsub\\my.dll\r\n\r\n[varlist]\r\n2\r\nVelocity\r\nthrottle\r\n[systemvarlist]\r\n1\r\nTime\r\n[stringvarlist]\r\n1\r\nIBIS_terminus_name\r\n[triggers]\r\n1\r\nbus_doorfront0\r\n");
        assert_eq!(o.dll, "sub\\my.dll");
        assert_eq!(o.vars, ["Velocity", "throttle"]);
        assert_eq!(o.system_vars, ["Time"]);
        assert_eq!(o.string_vars, ["IBIS_terminus_name"]);
        assert_eq!(o.triggers, ["bus_doorfront0"]);
    }

    #[test]
    fn wire_round_trip() {
        let f = Frame { system: vec![(0, 1.5)], vars: vec![(1, 2.0), (3, -1.0)], strings: vec![(0, "Zoo €".into())], triggers: vec![0, 2] };
        let mut buf = Vec::new();
        wire::put_frame(&mut buf, &f).unwrap();
        assert_eq!(wire::get_frame(&mut buf.as_slice()).unwrap(), f);
        let r = Reply { system: vec![None], vars: vec![Some(3.0), None], strings: vec![Some("ab".into())], triggers_active: vec![true, false] };
        let mut buf = Vec::new();
        wire::put_reply(&mut buf, &r).unwrap();
        assert_eq!(wire::get_reply(&mut buf.as_slice(), &f).unwrap(), r);
    }
}
