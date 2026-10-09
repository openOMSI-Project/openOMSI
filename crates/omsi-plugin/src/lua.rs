//! Lua plugins: `plugins/<name>.lua`, or a folder `plugins/<name>/main.lua`, run in an
//! embedded Lua 5.4. Every function of the API registry (`crate::api`) is bound as
//! `omsi.<name>` (a dotted name is a nested table: `ui.set` is `omsi.ui.set`); events,
//! timers and watches are the API's runtime, which calls the plugin's functions back through
//! this binding. `omsi.data` is a table saved between sessions (`<name>.save.lua`). A changed
//! file is loaded again while the game runs.
//!
//! Each plugin has its own Lua state with the safe libraries only: no `io`, no `os` beyond
//! the clock, no C modules, no `dofile`, no `string.dump` and no binary chunks anywhere (Lua
//! has no bytecode checker: a crafted binary chunk can break out of the VM), at most 256 MB
//! of memory; `require` finds modules through the plugin's [`LuaSource`] - its own folder on
//! disk. A call that runs longer than 50 ms is stopped and the plugin switched off; a plugin
//! whose handlers keep failing is switched off for the session.

use crate::api::{self, runtime, Binding, CallError, Ctx, PluginState, Value};
use crate::ui::{self, SharedUi};
use crate::PluginIo;
use mlua::{ChunkMode, Function, HookTriggers, Lua, LuaOptions, MultiValue, StdLib, Table, VmState};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

const PRELUDE: &str = include_str!("prelude.lua");

/// Most memory one plugin's Lua state may hold.
const MEMORY_LIMIT: usize = 256 << 20;
/// Deepest table a plugin passes to the API (a table that holds itself is cut there).
const MAX_DEPTH: usize = 32;
/// Most values in what a plugin passes to the API in one argument.
const MAX_VALUES: usize = 200_000;

/// Where a Lua plugin's code comes from: a folder on disk, or (an `.oop`) an archive.
pub trait LuaSource {
    /// The plugin's main chunk: its text and the name errors give it (`main.lua`).
    fn main(&self) -> Result<(String, String), String>;
    /// A module by its `require` name (`util`, `lib.json`): its text and name.
    fn module(&self, name: &str) -> Option<(String, String)>;
    /// The newest change of its files: a change loads the plugin again. None: never.
    fn stamp(&self) -> Option<SystemTime> {
        None
    }
}

/// A plugin's files on disk: a one-file plugin, or a folder with `main.lua` and its modules.
pub struct DiskSource {
    pub path: PathBuf,
}

impl DiskSource {
    fn is_main(&self) -> bool {
        self.path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("main.lua"))
    }

    fn folder(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }
}

impl LuaSource for DiskSource {
    fn main(&self) -> Result<(String, String), String> {
        let text = std::fs::read(&self.path).map_err(|e| e.to_string())?;
        let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok((String::from_utf8_lossy(&text).into_owned(), name))
    }

    fn module(&self, name: &str) -> Option<(String, String)> {
        // (`require("a.b")` is `a/b.lua` or `a/b/init.lua`, never out of the folder)
        let rel = api::paths::clean(&name.replace('.', "/")).ok().filter(|r| !r.is_empty())?;
        for file in [format!("{rel}.lua"), format!("{rel}/init.lua")] {
            if let Some(p) = crate::resolve_path(self.folder(), &file).filter(|p| p.is_file()) {
                let text = std::fs::read(&p).ok()?;
                return Some((String::from_utf8_lossy(&text).into_owned(), file));
            }
        }
        None
    }

    /// The plugin file's change time; for a folder plugin the newest `.lua` in it (its data
    /// folder left out).
    fn stamp(&self) -> Option<SystemTime> {
        if !self.is_main() {
            return std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        }
        fn walk(dir: &Path, best: &mut Option<SystemTime>, top: bool) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if !(top && p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(DATA_DIR))) {
                        walk(&p, best, false);
                    }
                } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lua")) && !is_save(&p) {
                    if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                        *best = Some(best.map_or(t, |b| b.max(t)));
                    }
                }
            }
        }
        let mut best = None;
        walk(self.folder(), &mut best, true);
        best
    }
}

/// A folder plugin's data folder (`storage`, `files`), in its own folder.
const DATA_DIR: &str = "data";

/// Every Lua plugin of a plugins folder: top-level `*.lua` files and `<folder>/main.lua`.
/// A one-file plugin's saved data (`<name>.save.lua`) is no plugin.
pub fn find_lua(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_default());
    let mut out = Vec::new();
    for p in entries {
        if p.is_dir() {
            if let Some(main) = crate::resolve_path(&p, "main.lua").filter(|m| m.is_file()) {
                out.push(main);
            }
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lua")) && !is_save(&p) {
            out.push(p);
        }
    }
    out
}

/// A plugin's saved data (`data.save.lua`, `<name>.save.lua`), not its code.
fn is_save(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().ends_with(".save.lua"))
}

/// What a Lua plugin is made from.
pub struct LuaSpec {
    pub name: String,
    pub source: Box<dyn LuaSource>,
    /// Where `omsi.data` is kept.
    pub save_file: PathBuf,
    /// Its data folder (`storage`, `files`).
    pub data_dir: PathBuf,
    /// Its own files (images, sounds, `plugin.read`); None: it has none on disk.
    pub folder: Option<PathBuf>,
    /// The permissions it declared; None: all, as a plain `.lua` file has.
    pub perms: Option<std::collections::HashSet<api::Perm>>,
}

impl LuaSpec {
    /// A plugin file of a plugins folder.
    pub fn disk(path: &Path) -> LuaSpec {
        let is_main = path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("main.lua"));
        let name_src = if is_main { path.parent().unwrap_or(path) } else { path };
        let name = name_src.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "plugin".into());
        let folder = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let (save_file, data_dir) = if is_main { (path.with_file_name("data.save.lua"), folder.join(DATA_DIR)) } else { (path.with_extension("save.lua"), path.with_extension("data")) };
        LuaSpec { name, source: Box::new(DiskSource { path: path.to_path_buf() }), save_file, data_dir, folder: Some(folder), perms: None }
    }
}

/// One Lua plugin.
pub struct LuaPlugin {
    /// The plugin's name: its file name, or its folder's for a `main.lua`.
    pub name: String,
    pub path: PathBuf,
    spec: LuaSpec,
    binding: Option<Box<LuaBinding>>,
    state: Box<PluginState>,
    slot: Slot,
    /// Newest change time of the plugin's files when it was loaded.
    stamp: Option<SystemTime>,
    last_check: Option<Instant>,
    pub disabled: bool,
}

/// Each plugin's mark on its panels (a plugin loaded again keeps its own).
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

/// A new plugin's mark on its panels (every kind of plugin counts on from the same).
pub(crate) fn next_owner() -> u64 {
    NEXT_OWNER.fetch_add(1, Ordering::Relaxed)
}

/// The game, the plugin's state and its binding while the plugin runs: set only for the
/// length of a call into it, so the API's functions it calls reach them.
#[derive(Clone, Copy)]
struct Raw {
    io: *mut (dyn PluginIo + 'static),
    state: *mut PluginState,
    binding: *const LuaBinding,
}

type Slot = Rc<Cell<Option<Raw>>>;

impl LuaPlugin {
    /// Load and start the plugin (its top level runs, then the `start` event).
    pub fn load(path: &Path, io: &mut dyn PluginIo) -> Result<LuaPlugin, String> {
        Self::load_with_ui(path, io, SharedUi::default())
    }

    /// [`LuaPlugin::load`], with the panels it shows in `ui` (shared by every plugin).
    pub fn load_with_ui(path: &Path, io: &mut dyn PluginIo, ui: SharedUi) -> Result<LuaPlugin, String> {
        Self::start_spec(LuaSpec::disk(path), path, io, runtime::Hub::with_ui(ui))
    }

    /// Load a plugin of any source (`path`: what it is known by in the log).
    pub fn start_spec(spec: LuaSpec, path: &Path, io: &mut dyn PluginIo, hub: runtime::SharedHub) -> Result<LuaPlugin, String> {
        let owner = NEXT_OWNER.fetch_add(1, Ordering::Relaxed);
        let mut state = PluginState::new(&spec.name, "Lua plugin", "lua", owner, hub, spec.folder.clone(), spec.data_dir.clone());
        state.perms = spec.perms.clone();
        let mut p = LuaPlugin { name: spec.name.clone(), path: path.to_path_buf(), spec, binding: None, state: Box::new(state), slot: Rc::new(Cell::new(None)), stamp: None, last_check: None, disabled: false };
        p.start(io)?;
        Ok(p)
    }

    /// The plugin's state (its events, timers, storage...).
    pub fn state(&self) -> &PluginState {
        &self.state
    }

    fn start(&mut self, io: &mut dyn PluginIo) -> Result<(), String> {
        self.stamp = self.spec.source.stamp();
        let (source, chunk) = self.spec.source.main()?;
        let binding = LuaBinding::new(self).map_err(|e| e.to_string())?;
        self.binding = Some(Box::new(binding));
        let r = self.with_ctx(io, |ctx, b| {
            b.depth.set(1);
            b.deadline.set(Some(Instant::now() + Duration::from_millis(runtime::LOAD_BUDGET_MS)));
            let top = b.lua.load(&source).set_name(format!("@{chunk}")).set_mode(ChunkMode::Text).exec().map_err(|e| b.error(e));
            b.depth.set(0);
            b.deadline.set(None);
            top?;
            runtime::start(ctx)
        });
        if let Err(e) = r {
            log::warn!("{} {}", self.state.tag, e.msg);
            io.message(&format!("Lua plugin {}: {}", self.name, e.msg.lines().next().unwrap_or("")), 8.0);
            // (what its top level showed before it failed would stay for good)
            self.with_ctx(io, |ctx, _| {
                runtime::finish(ctx);
            });
            self.binding = None;
            return Err(format!("{} did not start", self.name));
        }
        Ok(())
    }

    /// Run `f` with the game, the state and the binding reachable from Lua.
    fn with_ctx<R>(&mut self, io: &mut dyn PluginIo, f: impl FnOnce(&mut Ctx<'_>, &LuaBinding) -> R) -> R {
        let b: &LuaBinding = self.binding.as_deref().expect("a plugin runs with its Lua state");
        // SAFETY: the lifetime is erased only for the length of this call; the slot is
        // cleared before `io` goes out of reach
        let io_ptr: *mut (dyn PluginIo + '_) = io;
        let io_ptr = unsafe { std::mem::transmute::<*mut (dyn PluginIo + '_), *mut (dyn PluginIo + 'static)>(io_ptr) };
        let state: *mut PluginState = &mut *self.state;
        let raw = Raw { io: io_ptr, state, binding: b };
        self.slot.set(Some(raw));
        // SAFETY: the three live while `self` and `io` are borrowed here
        let mut ctx = unsafe { Ctx::from_raw(raw.io, raw.state, raw.binding as *const dyn Binding) };
        let r = f(&mut ctx, b);
        self.slot.set(None);
        r
    }

    /// One frame: a reload when the files changed, then the runtime's frame (the events,
    /// timers, watches and `frame`).
    pub fn frame(&mut self, io: &mut dyn PluginIo) {
        if self.last_check.is_none_or(|t| t.elapsed() > Duration::from_secs(1)) {
            self.last_check = Some(Instant::now());
            let stamp = self.spec.source.stamp();
            if stamp.is_some() && stamp != self.stamp {
                log::info!("{} changed: loading it again", self.state.tag);
                self.stop(io);
                self.reset_state();
                match self.start(io) {
                    Ok(()) => io.message(&format!("Lua plugin {} reloaded", self.name), 3.0),
                    Err(e) => log::warn!("{} {e}", self.state.tag),
                }
            }
        }
        if self.disabled || self.binding.is_none() {
            return;
        }
        self.with_ctx(io, |ctx, _| runtime::frame(ctx));
        self.disabled = self.state.disabled;
        if let Some(b) = self.binding.as_ref() {
            b.sweep();
        }
    }

    /// Whether the plugin listens to `event` (through `on`, or its global `on_<event>`).
    pub fn hears(&self, event: &str) -> bool {
        !self.disabled && (self.state.listens(event) || self.binding.as_ref().is_some_and(|b| b.has_global_handler(event)))
    }

    /// One event now, outside the frame (`Plugins::emit`).
    pub fn emit(&mut self, io: &mut dyn PluginIo, event: &str, args: Vec<Value>) {
        if self.disabled || self.binding.is_none() {
            return;
        }
        self.with_ctx(io, |ctx, _| runtime::emit_now(ctx, event, args));
        self.disabled = self.state.disabled;
    }

    /// A fresh state for a plugin loaded again (its owner mark stays).
    fn reset_state(&mut self) {
        let old = &self.state;
        let mut s = PluginState::new(&self.spec.name, "Lua plugin", "lua", old.owner, old.hub.clone(), self.spec.folder.clone(), self.spec.data_dir.clone());
        s.perms = self.spec.perms.clone();
        self.state = Box::new(s);
        self.disabled = false;
    }

    /// The `stop` event, then `omsi.data` and the storage are saved.
    pub fn stop(&mut self, io: &mut dyn PluginIo) {
        if self.binding.is_some() {
            self.with_ctx(io, |ctx, b| {
                runtime::stop(ctx);
                if let Err(e) = b.call_internal("_save") {
                    let (io, s) = ctx.both();
                    s.record_error(io, &e);
                }
            });
        }
        self.binding = None;
    }
}

/// The Lua side of a plugin: its state, its functions the runtime keeps (callbacks), and
/// the time budget of a call.
pub struct LuaBinding {
    lua: Lua,
    /// id -> function, and function -> id.
    by_id: Table,
    by_fn: Table,
    counts: RefCell<HashMap<u64, u32>>,
    /// Ids handed out during the call that nothing kept (forgotten after it).
    fresh: RefCell<Vec<u64>>,
    next: Cell<u64>,
    depth: Cell<u32>,
    deadline: Rc<Cell<Option<Instant>>>,
    overran: Rc<Cell<bool>>,
    globals: Table,
}

impl LuaBinding {
    /// A Lua state with the safe libraries and the `omsi` table.
    fn new(p: &LuaPlugin) -> mlua::Result<LuaBinding> {
        let libs = StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE | StdLib::PACKAGE | StdLib::OS;
        let lua = Lua::new_with(libs, LuaOptions::default())?;
        lua.set_memory_limit(MEMORY_LIMIT)?;
        let g = lua.globals();
        sandbox(&lua, &g)?;
        // a call that runs past its deadline is stopped (and keeps being stopped, should
        // the plugin catch the error, until it gives up)
        let deadline: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
        let overran = Rc::new(Cell::new(false));
        {
            let (deadline, overran) = (deadline.clone(), overran.clone());
            lua.set_hook(HookTriggers::new().every_nth_instruction(10_000), move |_, _| match deadline.get() {
                Some(d) if Instant::now() > d => {
                    overran.set(true);
                    Err(mlua::Error::runtime(format!("the plugin ran longer than {} ms and was stopped", runtime::CALL_BUDGET_MS)))
                }
                _ => Ok(VmState::Continue),
            });
        }
        let b = LuaBinding { by_id: lua.create_table()?, by_fn: lua.create_table()?, counts: RefCell::new(HashMap::new()), fresh: RefCell::new(Vec::new()), next: Cell::new(1), depth: Cell::new(0), deadline, overran, globals: g.clone(), lua };
        b.bind(p)?;
        Ok(b)
    }

    /// `omsi`: every function of the registry, nested by its dots, and the binding's own.
    fn bind(&self, p: &LuaPlugin) -> mlua::Result<()> {
        let lua = &self.lua;
        let omsi = lua.create_table()?;
        omsi.set("version", env!("CARGO_PKG_VERSION"))?;
        omsi.set("name", p.name.clone())?;
        for f in api::registry() {
            let mut t = omsi.clone();
            let mut parts = f.name.split('.').peekable();
            while let Some(part) = parts.next() {
                if parts.peek().is_none() {
                    t.set(part, self.function(f, p.slot.clone())?)?;
                } else {
                    t = match t.raw_get::<Option<Table>>(part)? {
                        Some(sub) => sub,
                        None => {
                            let sub = lua.create_table()?;
                            t.set(part, sub.clone())?;
                            sub
                        }
                    };
                }
            }
        }
        omsi.get::<Table>("ui")?.set("version", ui::VERSION)?;
        // print goes to the log too
        self.globals.set("print", omsi.get::<Function>("log")?)?;
        // saved data, only the plugin's own file
        let save = p.spec.save_file.clone();
        omsi.set("_read_data", lua.create_function(move |_, ()| Ok(std::fs::read_to_string(&save).ok()))?)?;
        let save = p.spec.save_file.clone();
        omsi.set(
            "_write_data",
            lua.create_function(move |_, text: Option<String>| {
                let r = match text {
                    Some(t) => std::fs::write(&save, t),
                    None => match std::fs::remove_file(&save) {
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        r => r,
                    },
                };
                r.map_err(|e| mlua::Error::runtime(format!("saving {}: {e}", save.display())))
            })?,
        )?;
        self.globals.set("omsi", omsi)?;
        require(lua, &self.globals, p.slot.clone(), source_ptr(p))?;
        lua.load(PRELUDE).set_name("=omsi").set_mode(ChunkMode::Text).exec()
    }

    /// The Lua function of a registry entry.
    fn function(&self, f: &'static api::ApiFn, slot: Slot) -> mlua::Result<Function> {
        self.lua.create_function(move |lua, args: MultiValue| {
            let Some(raw) = slot.get() else {
                return Err(mlua::Error::runtime(format!("omsi.{}: called outside the game's frame", f.name)));
            };
            // SAFETY: the slot is set only while the plugin's call runs (`with_ctx`)
            let b = unsafe { &*raw.binding };
            let mut vals = Vec::with_capacity(args.len());
            for a in args.iter() {
                vals.push(b.to_value(a).map_err(mlua::Error::runtime)?);
            }
            let mut ctx = unsafe { Ctx::from_raw(raw.io, raw.state, raw.binding as *const dyn Binding) };
            let out = api::call_fn(&mut ctx, f, vals).map_err(|e| mlua::Error::runtime(e.0))?;
            match out {
                Value::List(items) if f.multi => items.into_iter().map(|v| b.to_lua(lua, v)).collect::<mlua::Result<MultiValue>>(),
                v => Ok(MultiValue::from_iter([b.to_lua(lua, v)?])),
            }
        })
    }

    /// A Lua value as the API's.
    pub fn to_value(&self, v: &mlua::Value) -> Result<Value, String> {
        let mut budget = MAX_VALUES;
        convert(v, 0, &mut budget, &mut |f| self.id_of(f))
    }

    fn id_of(&self, f: &Function) -> u64 {
        if let Ok(Some(id)) = self.by_fn.raw_get::<Option<i64>>(f.clone()) {
            return id as u64;
        }
        let id = self.next.get();
        self.next.set(id + 1);
        let _ = self.by_fn.raw_set(f.clone(), id as i64);
        let _ = self.by_id.raw_set(id as i64, f.clone());
        self.fresh.borrow_mut().push(id);
        id
    }

    /// An API value for Lua.
    pub fn to_lua(&self, lua: &Lua, v: Value) -> mlua::Result<mlua::Value> {
        Ok(match v {
            Value::Nil => mlua::Value::Nil,
            Value::Bool(b) => mlua::Value::Boolean(b),
            Value::Int(i) => mlua::Value::Integer(i),
            Value::Num(n) => mlua::Value::Number(n),
            Value::Str(s) => mlua::Value::String(lua.create_string(s)?),
            Value::Bytes(b) => mlua::Value::String(lua.create_string(b)?),
            Value::List(l) => {
                let t = lua.create_table_with_capacity(l.len(), 0)?;
                for (i, x) in l.into_iter().enumerate() {
                    t.raw_set(i + 1, self.to_lua(lua, x)?)?;
                }
                mlua::Value::Table(t)
            }
            Value::Map(m) => {
                let t = lua.create_table_with_capacity(0, m.len())?;
                for (k, x) in m {
                    t.raw_set(k, self.to_lua(lua, x)?)?;
                }
                mlua::Value::Table(t)
            }
            Value::Callback(id) => self.by_id.raw_get::<mlua::Value>(id as i64)?,
        })
    }

    fn error(&self, e: mlua::Error) -> CallError {
        CallError { msg: e.to_string(), fatal: self.overran.get() }
    }

    /// Run `f` as a call into the plugin: the outermost one gets the time budget.
    fn guarded<R>(&self, f: impl FnOnce() -> mlua::Result<R>) -> Result<R, CallError> {
        let outer = self.depth.get() == 0;
        if outer {
            self.overran.set(false);
            self.deadline.set(Some(Instant::now() + Duration::from_millis(runtime::CALL_BUDGET_MS)));
        }
        self.depth.set(self.depth.get() + 1);
        let r = f();
        self.depth.set(self.depth.get() - 1);
        if outer {
            self.deadline.set(None);
        }
        r.map_err(|e| self.error(e))
    }

    /// One of the prelude's own functions (`omsi._save`).
    fn call_internal(&self, name: &str) -> Result<(), CallError> {
        self.guarded(|| self.globals.get::<Table>("omsi")?.get::<Function>(name)?.call::<()>(()))
    }

    /// Forget the functions handed out in the last call that nothing kept.
    fn sweep(&self) {
        let fresh = std::mem::take(&mut *self.fresh.borrow_mut());
        let counts = self.counts.borrow();
        for id in fresh {
            if !counts.contains_key(&id) {
                self.forget(id);
            }
        }
    }

    fn forget(&self, id: u64) {
        if let Ok(f) = self.by_id.raw_get::<mlua::Value>(id as i64) {
            let _ = self.by_fn.raw_set(f, mlua::Value::Nil);
        }
        let _ = self.by_id.raw_set(id as i64, mlua::Value::Nil);
    }
}

impl Binding for LuaBinding {
    fn invoke(&self, cb: u64, args: Vec<Value>) -> Result<Value, CallError> {
        let f: Function = match self.by_id.raw_get::<Option<Function>>(cb as i64) {
            Ok(Some(f)) => f,
            _ => return Err(CallError::new(format!("callback {cb} is gone"))),
        };
        let lua = &self.lua;
        let args = args.into_iter().map(|v| self.to_lua(lua, v)).collect::<mlua::Result<MultiValue>>().map_err(|e| self.error(e))?;
        let r: mlua::Value = self.guarded(|| f.call(args))?;
        self.to_value(&r).map_err(CallError::new)
    }

    fn retain(&self, cb: u64) {
        *self.counts.borrow_mut().entry(cb).or_insert(0) += 1;
    }

    fn release(&self, cb: u64) {
        let mut counts = self.counts.borrow_mut();
        if let Some(n) = counts.get_mut(&cb) {
            *n -= 1;
            if *n == 0 {
                counts.remove(&cb);
                drop(counts);
                self.forget(cb);
            }
        }
    }

    /// A global function `on_<event>`: `on_frame`, `on_start`, ...
    fn global_handler(&self, event: &str, args: &[Value]) -> Result<bool, CallError> {
        let f = match self.globals.raw_get::<mlua::Value>(format!("on_{event}")) {
            Ok(mlua::Value::Function(f)) => f,
            _ => return Ok(false),
        };
        let lua = &self.lua;
        let args = args.iter().cloned().map(|v| self.to_lua(lua, v)).collect::<mlua::Result<MultiValue>>().map_err(|e| self.error(e))?;
        self.guarded(|| f.call::<()>(args))?;
        Ok(true)
    }

    fn has_global_handler(&self, event: &str) -> bool {
        matches!(self.globals.raw_get::<mlua::Value>(format!("on_{event}")), Ok(mlua::Value::Function(_)))
    }
}

/// The safe libraries only: `os` the clock, no `dofile`/`loadfile`, `load` of text only, no
/// `string.dump`, `require` of the plugin's own modules (see `require`).
fn sandbox(lua: &Lua, g: &Table) -> mlua::Result<()> {
    let os: Table = g.get("os")?;
    let safe_os = lua.create_table()?;
    for k in ["clock", "time", "date", "difftime"] {
        safe_os.set(k, os.get::<mlua::Value>(k)?)?;
    }
    g.set("os", safe_os)?;
    g.set("dofile", mlua::Value::Nil)?;
    g.set("loadfile", mlua::Value::Nil)?;
    // (Lua has no bytecode checker: a binary chunk can break the VM's memory, and
    // `string.dump` is the way to make one. It is kept as a name, for plugins that test
    // `load(string.dump(f))`, but gives no code: only the mark of a binary chunk, which
    // `load` refuses)
    g.get::<Table>("string")?.set("dump", lua.create_function(|lua, _: MultiValue| lua.create_string("\x1bLua"))?)?;
    let package: Table = g.get("package")?;
    package.set("path", "")?;
    package.set("cpath", "")?;
    package.set("loadlib", mlua::Value::Nil)?;
    // `require("os")` hands out `package.loaded.os`, the whole library with `execute`
    // and `remove` (#1715): the loaded table keeps the safe libraries only, `os` the clock
    let loaded: Table = package.get("loaded")?;
    let names: Vec<String> = loaded.pairs::<String, mlua::Value>().filter_map(|kv| kv.ok().map(|(k, _)| k)).collect();
    for k in names {
        if !matches!(k.as_str(), "_G" | "table" | "string" | "math" | "utf8" | "coroutine" | "package") {
            loaded.set(k, mlua::Value::Nil)?;
        }
    }
    loaded.set("os", g.get::<Table>("os")?)?;
    lua.load("local raw = load; load = function(chunk, name, _, env) return raw(chunk, name, 't', env) end").set_mode(ChunkMode::Text).exec()
}

/// The source behind a plugin, for its `require` (the plugin outlives its Lua state).
fn source_ptr(p: &LuaPlugin) -> *const dyn LuaSource {
    &*p.spec.source
}

/// `require`: `package.preload`, then the plugin's source - never the disk's search path or
/// a C library. Modules load as text only.
fn require(lua: &Lua, g: &Table, slot: Slot, source: *const dyn LuaSource) -> mlua::Result<()> {
    let package: Table = g.get("package")?;
    let searchers: Table = package.get("searchers")?;
    let searcher = lua.create_function(move |lua, name: String| {
        // (the slot is set whenever the plugin runs, so its source is alive: it is the
        // plugin's own field, and the Lua state is dropped before it)
        let _ = slot.get();
        // SAFETY: as said above
        let src = unsafe { &*source };
        match src.module(&name) {
            Some((text, file)) => {
                let f = lua.load(text).set_name(format!("@{file}")).set_mode(ChunkMode::Text).into_function()?;
                Ok((mlua::Value::Function(f), mlua::Value::String(lua.create_string(&file)?)))
            }
            None => Ok((mlua::Value::String(lua.create_string(format!("\n\tno module '{name}' in the plugin"))?), mlua::Value::Nil)),
        }
    })?;
    // [1] package.preload stays; the file searchers go
    for i in (2..=searchers.raw_len()).rev() {
        searchers.raw_set(i, mlua::Value::Nil)?;
    }
    searchers.raw_set(2, searcher)?;
    Ok(())
}

/// A Lua value as the API's: `to_value` without a binding (functions become nil).
pub fn to_value(v: &mlua::Value) -> Result<Value, String> {
    let mut budget = MAX_VALUES;
    convert(v, 0, &mut budget, &mut |_| 0)
}

/// Integers stay integers, a table with the keys 1..n is a list and any other a map (its
/// keys sorted, numbers written as texts), a function is its callback id, a text that is no
/// UTF-8 is bytes. A table deeper than `MAX_DEPTH` is cut there (nil).
fn convert(v: &mlua::Value, depth: usize, budget: &mut usize, cb: &mut dyn FnMut(&Function) -> u64) -> Result<Value, String> {
    if *budget == 0 {
        return Err(format!("a table of more than {MAX_VALUES} values"));
    }
    *budget -= 1;
    Ok(match v {
        mlua::Value::Nil => Value::Nil,
        mlua::Value::Boolean(b) => Value::Bool(*b),
        mlua::Value::Integer(i) => Value::Int(*i),
        mlua::Value::Number(n) => Value::Num(*n),
        mlua::Value::String(s) => match s.to_str() {
            Ok(t) => Value::Str(t.to_string()),
            Err(_) => Value::Bytes(s.as_bytes().to_vec()),
        },
        mlua::Value::Function(f) => Value::Callback(cb(f)),
        mlua::Value::Table(t) => {
            if depth >= MAX_DEPTH {
                return Ok(Value::Nil);
            }
            let n = t.raw_len();
            let mut pairs: Vec<(mlua::Value, mlua::Value)> = Vec::new();
            for kv in t.clone().pairs::<mlua::Value, mlua::Value>() {
                pairs.push(kv.map_err(|e| e.to_string())?);
            }
            let is_list = pairs.len() == n && pairs.iter().all(|(k, _)| matches!(k, mlua::Value::Integer(i) if *i >= 1 && (*i as usize) <= n));
            if is_list {
                let mut out = vec![Value::Nil; n];
                for (k, x) in &pairs {
                    if let mlua::Value::Integer(i) = k {
                        out[*i as usize - 1] = convert(x, depth + 1, budget, cb)?;
                    }
                }
                Value::List(out)
            } else {
                let mut out = Vec::with_capacity(pairs.len());
                for (k, x) in &pairs {
                    let key = match k {
                        mlua::Value::String(s) => s.to_string_lossy().to_string(),
                        mlua::Value::Integer(i) => i.to_string(),
                        mlua::Value::Number(n) => crate::api::value::lua_number(*n),
                        mlua::Value::Boolean(b) => b.to_string(),
                        _ => continue,
                    };
                    out.push((key, convert(x, depth + 1, budget, cb)?));
                }
                out.sort_by(|a, b| a.0.cmp(&b.0));
                Value::Map(out)
            }
        }
        _ => Value::Nil,
    })
}

/// The io used outside the frame (loading and the way out): no vehicle.
pub struct NoVehicle;

impl PluginIo for NoVehicle {
    fn system(&mut self, _: &str) -> Option<f32> {
        None
    }
    fn set_system(&mut self, _: &str, _: f32) {}
    fn has_vehicle(&self) -> bool {
        false
    }
    fn var(&mut self, _: &str) -> Option<f32> {
        None
    }
    fn set_var(&mut self, _: &str, _: f32) {}
    fn string(&mut self, _: &str) -> Option<String> {
        None
    }
    fn set_string(&mut self, _: &str, _: &str) {}
    fn fire(&mut self, _: &str, _: bool) {}
}
