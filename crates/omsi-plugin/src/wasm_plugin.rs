//! A WebAssembly plugin in the game: the module (`wasm::WasmPlugin`) bound to the plugin API's
//! registry and runtime, as a Lua plugin is - the same functions by the same names, the same
//! events, timers, watches, storage and permissions.
//!
//! One difference: a callback the runtime calls while the module itself runs (the module's
//! `emit` reaching its own handlers) cannot enter the module again there - it is run right
//! after the module's call returns, in order. Its result is not seen by the caller (nil).

use crate::api::runtime::{self, Binding, CallError, PluginState, SharedHub};
use crate::api::{self, json, Ctx, Perm, Value};
use crate::wasm::{Dispatch, WasmPlugin};
use crate::PluginIo;
use std::cell::{Cell, RefCell};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// What a WebAssembly plugin is made from.
pub struct WasmSpec {
    pub name: String,
    pub wasm: Vec<u8>,
    /// Its data folder (`storage`, `files`).
    pub data_dir: PathBuf,
    /// Its own files on disk (images, sounds, `plugin.read`), if any.
    pub folder: Option<PathBuf>,
    /// The permissions it declared; None: all.
    pub perms: Option<HashSet<Perm>>,
}

/// The game and the plugin's state while the plugin runs: set only for the length of a call
/// into it (see `LuaPlugin`'s `Raw`).
#[derive(Clone, Copy)]
struct Raw {
    io: *mut (dyn PluginIo + 'static),
    state: *mut PluginState,
}

/// The module and what the runtime asks of it.
struct WasmBinding {
    module: RefCell<WasmPlugin>,
    raw: Cell<Option<Raw>>,
    /// Callbacks called while the module ran, to run when it has returned.
    pending: RefCell<VecDeque<(u64, Vec<Value>)>>,
}

impl WasmBinding {
    /// The registry, reached from the module's `openomsi.call`.
    fn dispatch(&self) -> Option<Disp<'_>> {
        self.raw.get().map(|raw| Disp { raw, binding: self })
    }

    /// Run `f` in the module, then the callbacks that came up meanwhile.
    fn run(&self, f: impl FnOnce(&mut WasmPlugin, &mut dyn Dispatch)) {
        let Some(mut d) = self.dispatch() else { return };
        match self.module.try_borrow_mut() {
            Ok(mut m) => f(&mut m, &mut d),
            Err(_) => return,
        }
        // (taken one at a time: a callback may queue more)
        loop {
            let Some((cb, args)) = self.pending.borrow_mut().pop_front() else { break };
            let text = json::encode(&Value::List(args));
            let Ok(mut m) = self.module.try_borrow_mut() else { break };
            m.callback(&mut d, cb as i64, &text);
        }
    }

    fn disabled(&self) -> Option<String> {
        self.module.try_borrow().ok().and_then(|m| m.disabled.clone())
    }
}

impl Binding for WasmBinding {
    fn invoke(&self, cb: u64, args: Vec<Value>) -> Result<Value, CallError> {
        if self.module.try_borrow_mut().is_err() {
            self.pending.borrow_mut().push_back((cb, args));
            return Ok(Value::Nil);
        }
        self.pending.borrow_mut().push_back((cb, args));
        self.run(|_, _| {});
        // (not `fatal`, which reads "ran longer than 50 ms": the module said why it stopped,
        // and `WasmHost::check` switches the plugin off)
        match self.disabled() {
            Some(why) => Err(CallError::new(why)),
            None => Ok(Value::Nil),
        }
    }
}

/// The module's view of the API.
struct Disp<'a> {
    raw: Raw,
    binding: &'a WasmBinding,
}

impl Dispatch for Disp<'_> {
    fn call(&mut self, name: &str, args: &str) -> Result<String, String> {
        let args = match json::decode(args, true) {
            Ok(Value::List(v)) => v,
            Ok(_) => return Err(format!("{name}: the arguments must be a JSON array")),
            Err(e) => return Err(format!("{name}: the arguments are not JSON: {e}")),
        };
        // SAFETY: the pointers are set by `WasmHost::with_ctx` for the length of the call
        // into the module that is calling here, on this thread (as Lua's binding does)
        let mut ctx = unsafe { Ctx::from_raw(self.raw.io, self.raw.state, self.binding as &dyn Binding as *const dyn Binding) };
        api::call(&mut ctx, name, args).map(|v| json::encode(&v)).map_err(|e| e.0)
    }

    fn log(&mut self, level: i32, text: &str) {
        // SAFETY: as `call`
        let tag = unsafe { (*self.raw.state).tag.clone() };
        match level {
            ..=0 => log::debug!("{tag} {text}"),
            1 => log::info!("{tag} {text}"),
            2 => log::warn!("{tag} {text}"),
            _ => log::error!("{tag} {text}"),
        }
    }
}

/// One WebAssembly plugin.
pub struct WasmHost {
    pub name: String,
    pub path: PathBuf,
    binding: Box<WasmBinding>,
    state: Box<PluginState>,
    pub disabled: bool,
}

impl WasmHost {
    /// Load the module and start it (`oop_start`, then the `start` event).
    pub fn start(spec: WasmSpec, path: &Path, io: &mut dyn PluginIo, hub: SharedHub) -> Result<WasmHost, String> {
        let module = WasmPlugin::load(&spec.name, &spec.wasm)?;
        let owner = crate::lua::next_owner();
        let mut state = PluginState::new(&spec.name, "WASM plugin", "wasm", owner, hub, spec.folder.clone(), spec.data_dir.clone());
        state.perms = spec.perms.clone();
        let binding = WasmBinding { module: RefCell::new(module), raw: Cell::new(None), pending: RefCell::new(VecDeque::new()) };
        let mut p = WasmHost { name: spec.name.clone(), path: path.to_path_buf(), binding: Box::new(binding), state: Box::new(state), disabled: false };
        let r = p.with_ctx(io, |ctx, b| {
            b.run(|m, d| m.start(d));
            if let Some(why) = b.disabled() {
                return Err(CallError::new(why));
            }
            runtime::start(ctx)
        });
        if let Err(e) = r {
            log::warn!("{} {}", p.state.tag, e.msg);
            io.message(&format!("WASM plugin {}: {}", p.name, e.msg.lines().next().unwrap_or("")), 8.0);
            p.with_ctx(io, |ctx, _| runtime::finish(ctx));
            return Err(format!("{} did not start", p.name));
        }
        Ok(p)
    }

    /// The plugin's state (its events, timers, storage...).
    pub fn state(&self) -> &PluginState {
        &self.state
    }

    fn with_ctx<R>(&mut self, io: &mut dyn PluginIo, f: impl FnOnce(&mut Ctx<'_>, &WasmBinding) -> R) -> R {
        // SAFETY: the lifetime is erased only for the length of this call; the pointers are
        // cleared before `io` goes out of reach
        let io_ptr: *mut (dyn PluginIo + '_) = io;
        let io_ptr = unsafe { std::mem::transmute::<*mut (dyn PluginIo + '_), *mut (dyn PluginIo + 'static)>(io_ptr) };
        let raw = Raw { io: io_ptr, state: &mut *self.state };
        let b: &WasmBinding = &self.binding;
        b.raw.set(Some(raw));
        // SAFETY: both live while `self` and `io` are borrowed here
        let mut ctx = unsafe { Ctx::from_raw(raw.io, raw.state, b as &dyn Binding as *const dyn Binding) };
        let r = f(&mut ctx, b);
        b.raw.set(None);
        r
    }

    fn check(&mut self) {
        if let Some(why) = self.binding.disabled() {
            if !self.disabled {
                log::warn!("{} stopped: {why}", self.state.tag);
            }
            self.state.disabled = true;
        }
        self.disabled = self.state.disabled;
    }

    /// One frame: the runtime's events, timers, watches and `frame`.
    pub fn frame(&mut self, io: &mut dyn PluginIo) {
        if self.disabled {
            return;
        }
        self.with_ctx(io, |ctx, _| runtime::frame(ctx));
        self.check();
    }

    /// Whether the plugin listens to `event`.
    pub fn hears(&self, event: &str) -> bool {
        !self.disabled && self.state.listens(event)
    }

    /// One event now, outside the frame.
    pub fn emit(&mut self, io: &mut dyn PluginIo, event: &str, args: Vec<Value>) {
        if self.disabled {
            return;
        }
        self.with_ctx(io, |ctx, _| runtime::emit_now(ctx, event, args));
        self.check();
    }

    /// The `stop` event, then the module's `oop_stop`; the storage is saved.
    pub fn stop(&mut self, io: &mut dyn PluginIo) {
        self.with_ctx(io, |ctx, b| {
            runtime::stop(ctx);
            b.run(|m, d| m.stop(d));
        });
    }
}
