//! WebAssembly plugins (`kind = "wasm"` in a `.oop`): a module built for wasm32-unknown-unknown,
//! usually from Rust with the openOMSI plugin SDK, run by an interpreter (wasmi) on every platform
//! the game runs on - the same file on Windows, macOS, Linux and Android.
//!
//! The module reaches the game through one import, `openomsi.call(name, args)`, which calls a
//! function of the plugin API by the name Lua plugins know it under (`"var"`, `"ui.set"`), its
//! arguments and result passed as JSON. The game calls the module back through `oop_callback(id,
//! args)` for the callbacks it registered (`{"$cb": id}` in an argument). See the plugin spec:
//! docs/PLUGINS.md, "WebAssembly plugins".
//!
//! A module has no file system, network or clock of its own: what it can do is what the API lets
//! it. A call into the module that runs too long (its fuel, about 50 ms of work, burnt) is stopped
//! and the plugin disabled, as a Lua plugin's runaway loop is; its memory is held to 256 MB.

use wasmi::{Caller, Config, Engine, Extern, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

/// The ABI version a module's `oop_abi` must return.
pub const ABI: i32 = 1;

/// Fuel for one call into a module: wasmi burns about one unit per instruction, and runs some
/// hundreds of millions a second, so this is in the tens of milliseconds on a phone.
const FUEL_PER_CALL: u64 = 20_000_000;

/// Memory a module may grow to.
const MEMORY_LIMIT: usize = 256 << 20;

/// The largest name or JSON text a module may hand over in one call.
const MAX_MESSAGE: usize = 16 << 20;

/// The game's side of a WebAssembly plugin's calls: the plugin API by name.
pub trait Dispatch {
    /// Calls API function `name` with `args` (a JSON array, as the module sent it). The
    /// result is JSON text; `Err` is a message the module gets through `last_error`.
    fn call(&mut self, name: &str, args: &str) -> Result<String, String>;
    /// A line for the log from the module (0 debug, 1 info, 2 warn, 3 error).
    fn log(&mut self, level: i32, text: &str) {
        match level {
            ..=0 => log::debug!("{text}"),
            1 => log::info!("{text}"),
            2 => log::warn!("{text}"),
            _ => log::error!("{text}"),
        }
    }
}

struct HostState {
    limits: StoreLimits,
    /// The dispatcher of the call in progress: set by [`WasmPlugin::enter`] for as long as the
    /// module runs and cleared before it returns, never kept beyond that.
    dispatch: Option<*mut (dyn Dispatch + 'static)>,
    last_error: String,
}

/// One loaded module.
pub struct WasmPlugin {
    pub name: String,
    store: Store<HostState>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    /// (checked to be there; the host never frees what it handed over)
    #[allow(dead_code)]
    free: TypedFunc<(i32, i32), ()>,
    callback: Option<TypedFunc<(i64, i32, i32), ()>>,
    start: Option<TypedFunc<(), ()>>,
    stop: Option<TypedFunc<(), ()>>,
    /// Why the plugin stopped working (a trap, its fuel burnt): it is called no more.
    pub disabled: Option<String>,
}

impl WasmPlugin {
    /// Compiles and instantiates `wasm`; nothing of it runs yet ([`WasmPlugin::start`] does).
    pub fn load(name: &str, wasm: &[u8]) -> Result<WasmPlugin, String> {
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm).map_err(|e| format!("{name}: not a usable WebAssembly module: {e}"))?;
        let state = HostState {
            limits: StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT).instances(1).memories(1).tables(4).build(),
            dispatch: None,
            last_error: String::new(),
        };
        let mut store = Store::new(&engine, state);
        store.limiter(|s| &mut s.limits);
        let mut linker = <Linker<HostState>>::new(&engine);
        link(&mut linker).map_err(|e| format!("{name}: {e}"))?;
        // (the start section runs with fuel too: a module looping there must not hang the game)
        store.set_fuel(FUEL_PER_CALL).map_err(|e| e.to_string())?;
        let instance = linker.instantiate_and_start(&mut store, &module).map_err(|e| format!("{name}: {e}"))?;
        let memory = instance.get_memory(&store, "memory").ok_or_else(|| format!("{name}: the module exports no memory"))?;
        let abi = instance
            .get_typed_func::<(), i32>(&store, "oop_abi")
            .map_err(|_| format!("{name}: no oop_abi export - not an openOMSI plugin (build it with the openOMSI plugin SDK)"))?
            .call(&mut store, ())
            .map_err(|e| format!("{name}: oop_abi: {e}"))?;
        if abi != ABI {
            return Err(format!("{name}: made for plugin ABI {abi}, this openOMSI runs ABI {ABI} - update {}", if abi > ABI { "openOMSI" } else { "the plugin" }));
        }
        let typed = |f: &str| format!("{name}: {f} is missing or has the wrong type");
        Ok(WasmPlugin {
            name: name.to_string(),
            alloc: instance.get_typed_func(&store, "oop_alloc").map_err(|_| typed("oop_alloc"))?,
            free: instance.get_typed_func(&store, "oop_free").map_err(|_| typed("oop_free"))?,
            callback: instance.get_typed_func(&store, "oop_callback").ok(),
            start: instance.get_typed_func(&store, "oop_start").ok(),
            stop: instance.get_typed_func(&store, "oop_stop").ok(),
            memory,
            store,
            disabled: None,
        })
    }

    /// Runs the module's `oop_start` (it registers its handlers there).
    pub fn start(&mut self, dispatch: &mut dyn Dispatch) {
        if let Some(f) = self.start {
            self.enter(dispatch, "oop_start", |store| f.call(store, ()));
        }
    }

    /// Calls the module's callback `id` with `args` (a JSON array).
    pub fn callback(&mut self, dispatch: &mut dyn Dispatch, id: i64, args: &str) {
        let Some(f) = self.callback else { return };
        if self.disabled.is_some() {
            return;
        }
        let text = args;
        let Some(ptr) = self.write_buffer(dispatch, text.as_bytes()) else { return };
        let len = text.len() as i32;
        // (the module frees the arguments: whoever is handed a buffer frees it, as the
        // module does with a call's result - freed here as well, the SDK's allocator aborted
        // on the second free)
        self.enter(dispatch, "oop_callback", |store| f.call(store, (id, ptr, len)));
    }

    /// Runs the module's `oop_stop`, if it has one (the game ends or the plugin goes).
    pub fn stop(&mut self, dispatch: &mut dyn Dispatch) {
        if let Some(f) = self.stop {
            self.enter(dispatch, "oop_stop", |store| f.call(store, ()));
        }
    }

    /// Copies `bytes` into a buffer the module allocates.
    fn write_buffer(&mut self, dispatch: &mut dyn Dispatch, bytes: &[u8]) -> Option<i32> {
        let alloc = self.alloc;
        let len = bytes.len() as i32;
        let ptr = self.enter(dispatch, "oop_alloc", |store| alloc.call(store, len))?;
        if self.memory.write(&mut self.store, ptr as u32 as usize, bytes).is_err() {
            self.disable(format!("oop_alloc returned {ptr:#x}, outside its memory"));
            return None;
        }
        Some(ptr)
    }

    /// Runs `f` in the module with `dispatch` reachable from its imports and a fresh fuel
    /// budget; a trap disables the plugin.
    fn enter<R>(
        &mut self,
        dispatch: &mut dyn Dispatch,
        what: &str,
        f: impl FnOnce(&mut Store<HostState>) -> Result<R, wasmi::Error>,
    ) -> Option<R> {
        if self.disabled.is_some() {
            return None;
        }
        // SAFETY: the pointer is only dereferenced by the imports while `f` runs (they are
        // called from inside it, on this thread) and is cleared below before `dispatch`'s
        // borrow ends; the lifetime is erased only to keep it in the store meanwhile.
        let erased: *mut (dyn Dispatch + '_) = dispatch;
        self.store.data_mut().dispatch = Some(unsafe { std::mem::transmute::<*mut (dyn Dispatch + '_), *mut (dyn Dispatch + 'static)>(erased) });
        let _ = self.store.set_fuel(FUEL_PER_CALL);
        let result = f(&mut self.store);
        self.store.data_mut().dispatch = None;
        match result {
            Ok(r) => Some(r),
            Err(e) => {
                let why = if e.as_trap_code() == Some(wasmi::TrapCode::OutOfFuel) {
                    format!("{what} ran too long (an endless loop?)")
                } else {
                    format!("{what}: {e}")
                };
                self.disable(why);
                None
            }
        }
    }

    fn disable(&mut self, why: String) {
        log::warn!("wasm plugin {}: {why}; the plugin is stopped", self.name);
        self.disabled = Some(why);
    }
}

/// The module's memory and allocator, from inside an import.
fn guest_memory(caller: &Caller<'_, HostState>) -> Result<Memory, wasmi::Error> {
    match caller.get_export("memory") {
        Some(Extern::Memory(m)) => Ok(m),
        _ => Err(wasmi::Error::new("the module exports no memory")),
    }
}

fn read_bytes(caller: &Caller<'_, HostState>, ptr: i32, len: i32) -> Result<Vec<u8>, wasmi::Error> {
    let len = usize::try_from(len).ok().filter(|&l| l <= MAX_MESSAGE).ok_or_else(|| wasmi::Error::new("a message of a bad length"))?;
    let mut buf = vec![0; len];
    guest_memory(caller)?.read(caller, ptr as u32 as usize, &mut buf).map_err(|_| wasmi::Error::new("a pointer outside the module's memory"))?;
    Ok(buf)
}

fn read_text(caller: &Caller<'_, HostState>, ptr: i32, len: i32) -> Result<String, wasmi::Error> {
    String::from_utf8(read_bytes(caller, ptr, len)?).map_err(|_| wasmi::Error::new("a text that is not UTF-8"))
}

/// Hands `bytes` to the module in a buffer of its allocator: `(ptr << 32) | len`.
fn hand_over(caller: &mut Caller<'_, HostState>, bytes: &[u8]) -> Result<i64, wasmi::Error> {
    let alloc = match caller.get_export("oop_alloc") {
        Some(Extern::Func(f)) => f.typed::<i32, i32>(&*caller)?,
        _ => return Err(wasmi::Error::new("the module exports no oop_alloc")),
    };
    let ptr = alloc.call(&mut *caller, bytes.len() as i32)?;
    guest_memory(caller)?.write(&mut *caller, ptr as u32 as usize, bytes).map_err(|_| wasmi::Error::new("oop_alloc returned a pointer outside the module's memory"))?;
    Ok((i64::from(ptr as u32) << 32) | bytes.len() as i64)
}

fn with_dispatch<R>(caller: &mut Caller<'_, HostState>, f: impl FnOnce(&mut dyn Dispatch) -> R) -> Result<R, wasmi::Error> {
    let ptr = caller.data().dispatch.ok_or_else(|| wasmi::Error::new("the game is not in a call"))?;
    // SAFETY: see `WasmPlugin::enter` - set for exactly the duration of the call we are in.
    Ok(f(unsafe { &mut *ptr }))
}

fn link(linker: &mut Linker<HostState>) -> Result<(), wasmi::Error> {
    linker.func_wrap("openomsi", "call", |mut caller: Caller<'_, HostState>, name_ptr: i32, name_len: i32, args_ptr: i32, args_len: i32| -> Result<i64, wasmi::Error> {
        let name = read_text(&caller, name_ptr, name_len)?;
        let args = read_text(&caller, args_ptr, args_len)?;
        let result = with_dispatch(&mut caller, |d| d.call(&name, &args))?;
        match result {
            Ok(v) => hand_over(&mut caller, v.as_bytes()),
            Err(e) => {
                caller.data_mut().last_error = e;
                Ok(-1)
            }
        }
    })?;
    linker.func_wrap("openomsi", "last_error", |mut caller: Caller<'_, HostState>| -> Result<i64, wasmi::Error> {
        let text = std::mem::take(&mut caller.data_mut().last_error);
        hand_over(&mut caller, text.as_bytes())
    })?;
    linker.func_wrap("openomsi", "log", |mut caller: Caller<'_, HostState>, level: i32, ptr: i32, len: i32| -> Result<(), wasmi::Error> {
        let text = read_text(&caller, ptr, len)?;
        with_dispatch(&mut caller, |d| d.log(level, &text))
    })?;
    Ok(())
}
