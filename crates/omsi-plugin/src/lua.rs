//! Lua plugins: `plugins/<name>.lua`, or a folder `plugins/<name>/main.lua`, run in an
//! embedded Lua 5.4. Unlike a DLL plugin a Lua plugin lists nothing up front: it reads and
//! writes the player's bus by name through the `omsi` table (see docs/PLUGINS.md), hears
//! events (`start`, `frame`, `vehicle`, `stop`, and what happened in the game: `crash`,
//! `pedestrian`, `stops_skipped`), keeps timers and watches, and has an
//! `omsi.data` table saved between sessions. It can put panels and notifications of its own
//! on the screen (`omsi.ui`, see `ui`). A changed file is loaded again while the game runs.
//!
//! Each plugin has its own Lua state with the safe libraries only: no `io`, no `os`
//! beyond the clock, no C modules and no `dofile`; `require` finds modules in the
//! plugin's own folder. The one way out to other programs is `omsi.send`: UDP datagrams
//! to this computer only. A call that runs longer than a second is stopped, and a plugin
//! whose handlers keep failing is switched off for the session.

use crate::ui::{self, SharedUi};
use crate::PluginIo;
use mlua::{Function, HookTriggers, Lua, LuaOptions, MultiValue, StdLib, Table, Value, VmState};
use std::cell::{Cell, RefCell};
use std::net::{Ipv4Addr, UdpSocket};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

const PRELUDE: &str = include_str!("prelude.lua");

/// Longest one call into a plugin may run.
const CALL_BUDGET: Duration = Duration::from_secs(1);
/// Failed calls after which a plugin is switched off.
const MAX_ERRORS: u32 = 10;
/// Most `omsi.send` messages of a plugin in one second, so a plugin cannot flood a program
/// on this computer.
const SEND_PER_SECOND: u32 = 100;
/// Longest `omsi.send` message: the same on every system (macOS takes UDP datagrams of at
/// most 9 KB by default, Windows and Linux about 64 KB).
const SEND_MAX: usize = 8 * 1024;
/// The game's multiplayer ports (`omsi_net::DEFAULT_PORT` and the `PORT_RANGE` after it): a
/// plugin's messages must not reach a session hosted on this computer.
const MULTIPLAYER_PORTS: std::ops::Range<u16> = 27015..27025;

/// The game's side while a plugin runs: set only for the length of a call.
type IoSlot = Rc<Cell<Option<*mut (dyn PluginIo + 'static)>>>;

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

/// One Lua plugin.
pub struct LuaPlugin {
    /// The plugin's name: its file name, or its folder's for a `main.lua`.
    pub name: String,
    pub path: PathBuf,
    /// Where `omsi.data` is kept.
    data_path: PathBuf,
    lua: Option<Lua>,
    io: IoSlot,
    deadline: Rc<Cell<Option<Instant>>>,
    /// Newest change time of the plugin's files when it was loaded.
    stamp: Option<SystemTime>,
    last_check: Option<Instant>,
    vehicle: Option<String>,
    errors: u32,
    pub disabled: bool,
    /// The on-screen panels of every plugin (`omsi.ui`), and this plugin's mark on its own.
    ui: SharedUi,
    owner: u64,
    /// Whether the panels had the mouse at this plugin's last frame (`ui_focus`).
    focus_seen: bool,
}

/// Each plugin's mark on its panels (a plugin loaded again keeps its own).
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

impl LuaPlugin {
    /// Load and start the plugin (its top level runs, then the `start` event).
    pub fn load(path: &Path, io: &mut dyn PluginIo) -> Result<LuaPlugin, String> {
        Self::load_with_ui(path, io, SharedUi::default())
    }

    /// [`LuaPlugin::load`], with the panels it shows in `ui` (shared by every plugin).
    pub fn load_with_ui(
        path: &Path,
        io: &mut dyn PluginIo,
        ui: SharedUi,
    ) -> Result<LuaPlugin, String> {
        let is_main = path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("main.lua"));
        let name_src = if is_main { path.parent().unwrap_or(path) } else { path };
        let name = name_src.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "plugin".into());
        let data_path = if is_main { path.with_file_name("data.save.lua") } else { path.with_extension("save.lua") };
        let mut p = LuaPlugin {
            name,
            path: path.to_path_buf(),
            data_path,
            lua: None,
            io: Rc::new(Cell::new(None)),
            deadline: Rc::new(Cell::new(None)),
            stamp: None,
            last_check: None,
            vehicle: None,
            errors: 0,
            disabled: false,
            ui,
            owner: NEXT_OWNER.fetch_add(1, Ordering::Relaxed),
            focus_seen: false,
        };
        p.start(io)?;
        Ok(p)
    }

    fn folder(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }

    /// Newest change time of the plugin's `.lua` files (its folder's, for a `main.lua`).
    fn newest_stamp(&self) -> Option<SystemTime> {
        let is_main = self.path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("main.lua"));
        if !is_main {
            return std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        }
        fn walk(dir: &Path, best: &mut Option<SystemTime>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, best);
                } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lua")) && !is_save(&p) {
                    if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                        *best = Some(best.map_or(t, |b| b.max(t)));
                    }
                }
            }
        }
        let mut best = None;
        walk(self.folder(), &mut best);
        best
    }

    fn start(&mut self, io: &mut dyn PluginIo) -> Result<(), String> {
        self.stamp = self.newest_stamp();
        let source = std::fs::read(&self.path).map_err(|e| e.to_string())?;
        let lua = self.new_state().map_err(|e| e.to_string())?;
        self.lua = Some(lua);
        self.vehicle = io.vehicle_name().filter(|_| io.has_vehicle());
        let chunk_name = format!("@{}", self.path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default());
        let vehicle = self.vehicle.clone();
        let ok = self.call(io, |lua| {
            lua.load(&source[..]).set_name(chunk_name).exec()?;
            emit(lua, "start", ())?;
            if let Some(v) = vehicle {
                emit(lua, "vehicle", v)?;
            }
            Ok(())
        });
        if !ok {
            self.lua = None;
            // (what its top level showed before it failed would stay for good)
            self.ui.borrow_mut().remove_owner(self.owner);
            return Err(format!("{} did not start", self.name));
        }
        Ok(())
    }

    /// A Lua state with the safe libraries and the `omsi` table.
    fn new_state(&self) -> mlua::Result<Lua> {
        let libs = StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE | StdLib::PACKAGE | StdLib::OS;
        let lua = Lua::new_with(libs, LuaOptions::default())?;
        let g = lua.globals();

        // os: the clock only
        let os: Table = g.get("os")?;
        let safe_os = lua.create_table()?;
        for k in ["clock", "time", "date", "difftime"] {
            safe_os.set(k, os.get::<Value>(k)?)?;
        }
        g.set("os", safe_os)?;
        g.set("dofile", Value::Nil)?;
        g.set("loadfile", Value::Nil)?;
        // require: Lua modules of the plugin's folder, no C libraries
        let package: Table = g.get("package")?;
        let dir = self.folder().to_string_lossy().replace('\\', "/");
        package.set("path", format!("{dir}/?.lua;{dir}/?/init.lua"))?;
        package.set("cpath", "")?;
        package.set("loadlib", Value::Nil)?;
        lua.load("package.searchers[4] = nil; package.searchers[3] = nil").exec()?;
        // `require("os")` hands out `package.loaded.os`, the whole library with `execute`
        // and `remove` (#1715): the loaded table keeps the safe libraries only, `os` the
        // clock. `load` reads text only - a binary chunk can break the VM's memory.
        let loaded: Table = package.get("loaded")?;
        let names: Vec<String> = loaded.pairs::<String, Value>().filter_map(|kv| kv.ok().map(|(k, _)| k)).collect();
        for k in names {
            if !matches!(k.as_str(), "_G" | "table" | "string" | "math" | "utf8" | "coroutine" | "package") {
                loaded.set(k, Value::Nil)?;
            }
        }
        loaded.set("os", g.get::<Table>("os")?)?;
        lua.load("local raw = load; load = function(chunk, name, _, env) return raw(chunk, name, 't', env) end").exec()?;

        // a call that runs past its deadline is stopped
        let deadline = self.deadline.clone();
        lua.set_hook(HookTriggers::new().every_nth_instruction(10_000), move |_, _| match deadline.get() {
            Some(d) if Instant::now() > d => Err(mlua::Error::runtime("the plugin ran longer than a second and was stopped")),
            _ => Ok(VmState::Continue),
        });

        let omsi = lua.create_table()?;
        omsi.set("version", env!("CARGO_PKG_VERSION"))?;
        omsi.set("name", self.name.clone())?;
        let name = self.name.clone();
        let join = |args: MultiValue| -> String {
            args.iter().map(|v| v.to_string().unwrap_or_else(|_| format!("{v:?}"))).collect::<Vec<_>>().join("\t")
        };
        omsi.set(
            "log",
            lua.create_function(move |_, args: MultiValue| {
                log::info!("[lua {name}] {}", join(args));
                Ok(())
            })?,
        )?;
        let name = self.name.clone();
        omsi.set(
            "warn",
            lua.create_function(move |_, args: MultiValue| {
                log::warn!("[lua {name}] {}", join(args));
                Ok(())
            })?,
        )?;
        // print goes to the log too
        g.set("print", omsi.get::<Function>("log")?)?;

        let io = self.io.clone();
        let with = move |f: &mut dyn FnMut(&mut dyn PluginIo)| {
            if let Some(p) = io.get() {
                // SAFETY: the slot holds the frame's io only while `call` runs, which is
                // the only time Lua code runs
                f(unsafe { &mut *p });
            }
        };
        macro_rules! func {
            ($name:literal, $args:ty, |$io:ident, $a:pat_param| $ret:ty => $body:expr) => {{
                let with = with.clone();
                omsi.set(
                    $name,
                    lua.create_function(move |_, $a: $args| {
                        let mut out: Option<$ret> = None;
                        with(&mut |$io: &mut dyn PluginIo| out = Some($body));
                        Ok(out.unwrap_or_default())
                    })?,
                )?;
            }};
        }
        func!("has_vehicle", (), |io, _a| bool => io.has_vehicle());
        func!("vehicle", (), |io, _a| Option<String> => io.vehicle_name().filter(|_| io.has_vehicle()));
        func!("vehicle_manufacturer", (), |io, _a| Option<String> => io.vehicle_manufacturer_model().filter(|_| io.has_vehicle()).map(|(m, _)| m));
        func!("vehicle_model", (), |io, _a| Option<String> => io.vehicle_manufacturer_model().filter(|_| io.has_vehicle()).map(|(_, m)| m));
        func!("var", String, |io, n| Option<f32> => if io.has_vehicle() { io.var(&n) } else { None });
        func!("set_var", (String, f32), |io, (n, v)| bool => io.has_vehicle() && io.var(&n).is_some() && { io.set_var(&n, v); true });
        func!("str", String, |io, n| Option<String> => if io.has_vehicle() { io.string(&n) } else { None });
        func!("set_str", (String, String), |io, (n, s)| bool => io.has_vehicle() && io.string(&n).is_some() && { io.set_string(&n, &s); true });
        func!("sys", String, |io, n| Option<f32> => io.system(&n));
        func!("press", String, |io, n| () => io.fire(&n, true));
        func!("release", String, |io, n| () => io.fire(&n, false));
        func!("trigger", String, |io, n| () => { io.fire(&n, true); io.fire(&n, false) });
        func!("message", (String, Option<f32>), |io, (t, s)| () => io.message(&t, s.unwrap_or(5.0)));
        // position: x, y, z, heading - four numbers, or nothing on foot
        let with2 = with.clone();
        omsi.set(
            "position",
            lua.create_function(move |_, ()| {
                let mut out = None;
                with2(&mut |io: &mut dyn PluginIo| out = io.position());
                Ok(match out {
                    Some([x, y, z, h]) => MultiValue::from_iter([x, y, z, h].map(Value::Number)),
                    None => MultiValue::new(),
                })
            })?,
        )?;

        // what the game is doing: a table of numbers, text and flags
        let with3 = with.clone();
        omsi.set(
            "info",
            lua.create_function(move |lua, ()| {
                let t = lua.create_table()?;
                let mut pairs = Vec::new();
                with3(&mut |io: &mut dyn PluginIo| pairs = io.info());
                for (k, v) in pairs {
                    t.set(k, to_lua(lua, v)?)?;
                }
                Ok(t)
            })?,
        )?;
        func!("command", String, |io, c| bool => io.command(&c));
        // the other vehicles around: a list of {id, kind, name, x, y, z, heading}
        let with6 = with.clone();
        omsi.set(
            "others",
            lua.create_function(move |lua, radius: Option<f64>| {
                let mut list = Vec::new();
                with6(&mut |io: &mut dyn PluginIo| list = io.others(radius.unwrap_or(300.0)));
                let out = lua.create_table()?;
                for (i, o) in list.into_iter().enumerate() {
                    let t = lua.create_table()?;
                    t.set("id", o.id)?;
                    t.set("kind", o.kind)?;
                    t.set("name", o.name)?;
                    t.set("x", o.pos[0])?;
                    t.set("y", o.pos[1])?;
                    t.set("z", o.pos[2])?;
                    t.set("heading", o.pos[3])?;
                    out.set(i + 1, t)?;
                }
                Ok(out)
            })?,
        )?;
        func!("other_var", (u64, String), |io, (id, n)| Option<f32> => io.other_var(id, &n));
        func!("set_other_var", (u64, String, f32), |io, (id, n, v)| bool => io.set_other_var(id, &n, v));
        let with4 = with.clone();
        omsi.set(
            "vars",
            lua.create_function(move |lua, kind: Option<String>| {
                let mut names = (Vec::new(), Vec::new());
                with4(&mut |io: &mut dyn PluginIo| names = io.var_names());
                let list = if kind.as_deref() == Some("str") { names.1 } else { names.0 };
                lua.create_sequence_from(list)
            })?,
        )?;
        let with5 = with.clone();
        omsi.set(
            "_keys",
            lua.create_function(move |lua, ()| {
                let mut keys = Vec::new();
                with5(&mut |io: &mut dyn PluginIo| keys = io.keys());
                let t = lua.create_table()?;
                for (i, (k, down)) in keys.into_iter().enumerate() {
                    let e = lua.create_table()?;
                    e.set(1, k)?;
                    e.set(2, down)?;
                    t.set(i + 1, e)?;
                }
                Ok(t)
            })?,
        )?;

        // omsi.send(port, text): one UDP datagram to another program on this computer
        let sender = RefCell::new(Sender::default());
        omsi.set(
            "send",
            lua.create_function(move |_, (port, text): (i64, mlua::String)| {
                Ok(match sender.borrow_mut().send(port, &text.as_bytes()) {
                    Ok(()) => (true, None),
                    Err(why) => (false, Some(why)),
                })
            })?,
        )?;

        // saved data, only the plugin's own file
        let data_path = self.data_path.clone();
        omsi.set("_read_data", lua.create_function(move |_, ()| Ok(std::fs::read_to_string(&data_path).ok()))?)?;
        let data_path = self.data_path.clone();
        omsi.set(
            "_write_data",
            lua.create_function(move |_, text: Option<String>| {
                let r = match text {
                    Some(t) => std::fs::write(&data_path, t),
                    None => match std::fs::remove_file(&data_path) {
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        r => r,
                    },
                };
                r.map_err(|e| mlua::Error::runtime(format!("saving {}: {e}", data_path.display())))
            })?,
        )?;
        omsi.set("ui", self.ui_table(&lua)?)?;
        g.set("omsi", omsi)?;
        drop(g);
        lua.load(PRELUDE).set_name("=omsi").exec()?;
        Ok(lua)
    }

    /// `omsi.ui`: the plugin's panels and notifications on the screen (see `ui`). They live
    /// in the state the plugins share with the game, so a panel set by the file's top level
    /// - before the game's first frame - is there as well.
    fn ui_table(&self, lua: &Lua) -> mlua::Result<Table> {
        let t = lua.create_table()?;
        t.set("version", ui::VERSION)?;
        let owner = self.owner;
        let shared = self.ui.clone();
        t.set(
            "set",
            lua.create_function(move |_, (id, panel): (Value, Value)| {
                let result = match (ui_id(&id), panel) {
                    (Err(e), _) => Err(e),
                    (Ok(id), Value::Table(p)) => {
                        ui::parse_panel(&p).and_then(|p| shared.borrow_mut().set(owner, &id, p))
                    }
                    (Ok(_), _) => Err("the panel is a table".to_string()),
                };
                Ok(match result {
                    Ok(()) => (true, None),
                    Err(why) => (false, Some(why)),
                })
            })?,
        )?;
        let shared = self.ui.clone();
        t.set(
            "remove",
            lua.create_function(move |_, id: Value| {
                Ok(ui_id(&id).is_ok_and(|id| shared.borrow_mut().remove(owner, &id)))
            })?,
        )?;
        let shared = self.ui.clone();
        t.set(
            "clear",
            lua.create_function(move |_, ()| {
                shared.borrow_mut().clear(owner);
                Ok(())
            })?,
        )?;
        let shared = self.ui.clone();
        t.set(
            "toast",
            lua.create_function(move |_, (text, opts): (Value, Option<Table>)| {
                Ok(match ui::parse_toast(&text, opts.as_ref()) {
                    Ok(spec) => {
                        shared.borrow_mut().toast(owner, spec);
                        (true, None)
                    }
                    Err(why) => (false, Some(why)),
                })
            })?,
        )?;
        let shared = self.ui.clone();
        t.set(
            "focus",
            lua.create_function(move |_, on: Value| {
                Ok(shared
                    .borrow_mut()
                    .set_focus(owner, on.as_boolean().unwrap_or(!on.is_nil())))
            })?,
        )?;
        let shared = self.ui.clone();
        t.set(
            "focused",
            lua.create_function(move |_, ()| Ok(shared.borrow().focused()))?,
        )?;
        let shared = self.ui.clone();
        t.set(
            "screen",
            lua.create_function(move |_, ()| {
                let [w, h, scale] = shared.borrow().screen();
                Ok((w, h, scale))
            })?,
        )?;
        Ok(t)
    }

    /// Run `f` with the game's io reachable from Lua; false (logged) when it failed.
    fn call(&mut self, io: &mut dyn PluginIo, f: impl FnOnce(&Lua) -> mlua::Result<()>) -> bool {
        let Some(lua) = self.lua.as_ref() else { return false };
        // SAFETY: the lifetime is erased only for the length of this call; the slot is
        // cleared before `io` goes out of reach
        let ptr: *mut (dyn PluginIo + '_) = io;
        self.io.set(Some(unsafe { std::mem::transmute::<*mut (dyn PluginIo + '_), *mut (dyn PluginIo + 'static)>(ptr) }));
        self.deadline.set(Some(Instant::now() + CALL_BUDGET));
        let r = f(lua);
        self.deadline.set(None);
        self.io.set(None);
        match r {
            Ok(()) => true,
            Err(e) => {
                self.errors += 1;
                log::warn!("[lua {}] {e}", self.name);
                io.message(&format!("Lua plugin {}: {}", self.name, first_line(&e.to_string())), 8.0);
                if self.errors >= MAX_ERRORS {
                    log::warn!("[lua {}] {MAX_ERRORS} errors: switched off until it changes or the game restarts", self.name);
                    self.disabled = true;
                    // (its buttons would answer nothing any more)
                    self.ui.borrow_mut().remove_owner(self.owner);
                }
                false
            }
        }
    }

    /// One frame: a reload when the files changed, the `vehicle` event when the player's
    /// bus changed, then timers, watches and `frame`.
    pub fn frame(&mut self, io: &mut dyn PluginIo) {
        if self.last_check.is_none_or(|t| t.elapsed() > Duration::from_secs(1)) {
            self.last_check = Some(Instant::now());
            let stamp = self.newest_stamp();
            if stamp.is_some() && stamp != self.stamp {
                log::info!("[lua {}] changed: loading it again", self.name);
                self.stop(io);
                self.errors = 0;
                self.disabled = false;
                match self.start(io) {
                    Ok(()) => io.message(&format!("Lua plugin {} reloaded", self.name), 3.0),
                    Err(e) => log::warn!("[lua {}] {e}", self.name),
                }
            }
        }
        if self.disabled || self.lua.is_none() {
            return;
        }
        let now = io.vehicle_name().filter(|_| io.has_vehicle());
        if now != self.vehicle {
            self.vehicle = now.clone();
            self.call(io, |lua| emit(lua, "vehicle", now));
        }
        for e in io.events() {
            if self.disabled {
                return;
            }
            self.call(io, |lua| {
                let args = e.args.into_iter().map(|v| to_lua(lua, v)).collect::<mlua::Result<MultiValue>>()?;
                emit(lua, e.name, args)
            });
        }
        // the panels getting or losing the mouse (`ui_focus`), and the clicks on this
        // plugin's panels (`ui_click`)
        let focused = self.ui.borrow().focused();
        if focused != self.focus_seen {
            self.focus_seen = focused;
            self.call(io, |lua| emit(lua, "ui_focus", focused));
        }
        let clicks = self.ui.borrow_mut().take_clicks(self.owner);
        for c in clicks {
            if self.disabled {
                return;
            }
            self.call(io, |lua| emit(lua, "ui_click", (c.panel, c.element)));
        }
        let dt = io.dt();
        self.call(io, |lua| lua.globals().get::<Table>("omsi")?.get::<Function>("_tick")?.call::<()>(dt));
    }

    /// The `stop` event, then `omsi.data` is saved.
    pub fn stop(&mut self, io: &mut dyn PluginIo) {
        if self.lua.is_some() {
            self.deadline.set(None);
            self.call(io, |lua| emit(lua, "stop", ()));
            self.call(io, |lua| lua.globals().get::<Table>("omsi")?.get::<Function>("_save")?.call::<()>(()));
        }
        self.lua = None;
        // (a plugin loaded again starts with no panels: it shows its own again)
        self.ui.borrow_mut().remove_owner(self.owner);
    }
}

/// A plugin's `omsi.send`: its socket, opened on the first message, and the messages of
/// the current second.
#[derive(Default)]
struct Sender {
    socket: Option<UdpSocket>,
    second: Option<Instant>,
    sent: u32,
}

impl Sender {
    /// Send `data` to `127.0.0.1:port`, never waiting: with nobody listening it is lost,
    /// as UDP is. Err says why it was not sent.
    fn send(&mut self, port: i64, data: &[u8]) -> Result<(), String> {
        let port = u16::try_from(port).ok().filter(|p| *p >= 1024).ok_or("the port must be 1024-65535")?;
        if MULTIPLAYER_PORTS.contains(&port) {
            return Err(format!("ports {}-{} are the game's multiplayer", MULTIPLAYER_PORTS.start, MULTIPLAYER_PORTS.end - 1));
        }
        if data.len() > SEND_MAX {
            return Err(format!("a message is at most 8 KB ({} bytes given)", data.len()));
        }
        if self.second.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
            self.second = Some(Instant::now());
            self.sent = 0;
        }
        if self.sent >= SEND_PER_SECOND {
            return Err(format!("more than {SEND_PER_SECOND} messages in a second"));
        }
        if self.socket.is_none() {
            let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
            socket.set_nonblocking(true).map_err(|e| e.to_string())?;
            self.socket = Some(socket);
        }
        let socket = self.socket.as_ref().expect("opened above");
        socket.send_to(data, (Ipv4Addr::LOCALHOST, port)).map_err(|e| e.to_string())?;
        // (only what went out counts)
        self.sent += 1;
        Ok(())
    }
}

fn emit(lua: &Lua, event: &str, args: impl mlua::IntoLuaMulti) -> mlua::Result<()> {
    let emit: Function = lua.globals().get::<Table>("omsi")?.get("emit")?;
    let mut a = args.into_lua_multi(lua)?;
    a.push_front(Value::String(lua.create_string(event)?));
    emit.call::<()>(a)
}

/// A value of `omsi.info()` or of an event, as Lua sees it.
fn to_lua(lua: &Lua, v: crate::InfoValue) -> mlua::Result<Value> {
    Ok(match v {
        crate::InfoValue::Num(n) => Value::Number(n),
        crate::InfoValue::Text(s) => Value::String(lua.create_string(s)?),
        crate::InfoValue::Bool(b) => Value::Boolean(b),
    })
}

/// A panel's id as `omsi.ui` takes it: a text, or a number written as one.
fn ui_id(v: &Value) -> Result<String, String> {
    match v {
        Value::String(s) => s
            .to_str()
            .map(|s| s.to_string())
            .map_err(|_| "the panel id is no UTF-8".to_string()),
        Value::Integer(i) => Ok(i.to_string()),
        _ => Err("the panel id is a text".to_string()),
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
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
