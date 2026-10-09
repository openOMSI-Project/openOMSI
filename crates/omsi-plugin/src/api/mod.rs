//! The plugin API registry: every function a plugin can call, once, whatever its language.
//!
//! A function is an [`ApiFn`]: its name (`"var"`, `"ui.set"`, `"traffic.list"`), its group,
//! parameters and documentation - the documentation of docs/PLUGINS.md and of
//! `docs/plugin-api.json` is generated from these entries, so a function cannot be
//! documented differently from what it does - and the Rust function that does it. Lua binds
//! every entry as `omsi.<name>` (a dot is a nested table: `omsi.ui.set`); a WASM plugin calls
//! `openomsi.call("<name>", args)`.
//!
//! The events (`omsi.on`), timers, watches and messages between plugins live in the
//! language-neutral [`runtime`], which calls the plugin's functions back through its
//! [`Binding`]. Values cross as [`Value`]s.

pub mod docs;
pub mod events;
mod fns;
pub mod json;
pub mod paths;
pub mod runtime;
pub mod value;

pub use runtime::{Binding, CallError, PluginState};
pub use value::Value;

use crate::PluginIo;

/// The version of the function interface a plugin binds to (`api_abi` of an `.oop`).
pub const ABI: i64 = 1;
/// The openOMSI version the functions and events marked `since` it came with.
pub const VERSION: &str = "0.2.22";

/// One parameter of a function, for the documentation and the argument checks.
#[derive(Debug, Clone, Copy)]
pub struct Param {
    pub name: &'static str,
    /// "number", "integer", "string", "bool", "table", "function" or "any".
    pub ty: &'static str,
    pub optional: bool,
}

/// A required parameter.
pub const fn p(name: &'static str, ty: &'static str) -> Param {
    Param { name, ty, optional: false }
}

/// An optional parameter.
pub const fn o(name: &'static str, ty: &'static str) -> Param {
    Param { name, ty, optional: true }
}

/// What a plugin must have declared to call a function (an `.oop`'s `permissions`; a plain
/// `.lua` plugin has them all, as it always had).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Perm {
    /// Reading the game: every plugin may.
    None,
    /// Panels, notifications and on-screen messages.
    Ui,
    /// The plugin's own data folder and key/value storage.
    Storage,
    /// Changing the player's bus: variables, triggers, doors, the destination.
    VehicleWrite,
    /// Changing the AI traffic and the people.
    TrafficWrite,
    /// Changing the world: weather, time, pause, teleport, the duty, game menu actions.
    WorldWrite,
    /// The camera and the view.
    Camera,
    /// Playing sounds.
    Audio,
    /// UDP to other programs on this computer.
    NetworkLocal,
    /// Messages to the other players of a LAN game.
    Lan,
}

impl Perm {
    /// The name an `.oop` header lists it under.
    pub fn as_str(self) -> &'static str {
        match self {
            Perm::None => "",
            Perm::Ui => "ui",
            Perm::Storage => "storage",
            Perm::VehicleWrite => "vehicle_write",
            Perm::TrafficWrite => "traffic_write",
            Perm::WorldWrite => "world_write",
            Perm::Camera => "camera",
            Perm::Audio => "audio",
            Perm::NetworkLocal => "network_local",
            Perm::Lan => "lan",
        }
    }

    pub fn parse(s: &str) -> Option<Perm> {
        Self::ALL.into_iter().find(|p| p.as_str() == s)
    }

    pub const ALL: [Perm; 9] = [Perm::Ui, Perm::Storage, Perm::VehicleWrite, Perm::TrafficWrite, Perm::WorldWrite, Perm::Camera, Perm::Audio, Perm::NetworkLocal, Perm::Lan];
}

/// The handler of a function.
pub type Handler = fn(&mut Ctx<'_>, Args) -> Result<Value, ApiError>;

/// One function of the API.
pub struct ApiFn {
    /// `"var"`, `"ui.set"`: dots make nested tables in Lua.
    pub name: &'static str,
    /// The docs section: "vehicle", "game", "ui", ...
    pub group: &'static str,
    pub params: &'static [Param],
    /// What it gives back, for people.
    pub returns: &'static str,
    /// One paragraph, the user-facing documentation.
    pub doc: &'static str,
    pub since: &'static str,
    pub perm: Perm,
    /// The result is several values: a `Value::List` it returns is spread over Lua's
    /// return values (`x, y, z, heading = omsi.position()`; an empty list is none at all).
    /// A WASM plugin gets the list as it is.
    pub multi: bool,
    pub call: Handler,
}

impl std::fmt::Debug for ApiFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiFn").field("name", &self.name).field("group", &self.group).finish()
    }
}

/// A function failed: the message a Lua plugin sees as its error (`omsi.var: argument 1
/// (name) must be a string`).
#[derive(Debug, Clone, PartialEq)]
pub struct ApiError(pub String);

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for ApiError {
    fn from(s: String) -> ApiError {
        ApiError(s)
    }
}

impl From<&str> for ApiError {
    fn from(s: &str) -> ApiError {
        ApiError(s.to_string())
    }
}

/// The arguments of a call, with the checks every function shares.
pub struct Args {
    pub v: Vec<Value>,
    pub f: &'static ApiFn,
}

impl Args {
    pub fn new(f: &'static ApiFn, v: Vec<Value>) -> Args {
        Args { v, f }
    }

    pub fn len(&self) -> usize {
        self.v.len()
    }

    pub fn is_empty(&self) -> bool {
        self.v.is_empty()
    }

    /// Argument `i` (from 0), nil when not given.
    pub fn get(&self, i: usize) -> &Value {
        self.v.get(i).unwrap_or(&Value::Nil)
    }

    /// Take argument `i` out (nil when not given).
    pub fn take(&mut self, i: usize) -> Value {
        self.v.get_mut(i).map(std::mem::take).unwrap_or(Value::Nil)
    }

    fn bad(&self, i: usize, want: &str) -> ApiError {
        let name = self.f.params.get(i).map_or("?", |p| p.name);
        ApiError(format!("omsi.{}: argument {} ({name}) must be {want}, not {}", self.f.name, i + 1, self.get(i).kind()))
    }

    pub fn num(&self, i: usize) -> Result<f64, ApiError> {
        self.get(i).as_f64().ok_or_else(|| self.bad(i, "a number"))
    }

    pub fn opt_num(&self, i: usize) -> Result<Option<f64>, ApiError> {
        match self.get(i) {
            Value::Nil => Ok(None),
            v => v.as_f64().map(Some).ok_or_else(|| self.bad(i, "a number")),
        }
    }

    pub fn int(&self, i: usize) -> Result<i64, ApiError> {
        match self.get(i) {
            Value::Num(n) if n.is_finite() => Ok(n.floor() as i64),
            v => v.as_i64().ok_or_else(|| self.bad(i, "a whole number")),
        }
    }

    pub fn opt_int(&self, i: usize) -> Result<Option<i64>, ApiError> {
        match self.get(i) {
            Value::Nil => Ok(None),
            _ => self.int(i).map(Some),
        }
    }

    /// A text; numbers are written as Lua writes them (as Lua's own string functions do).
    pub fn str(&self, i: usize) -> Result<String, ApiError> {
        self.get(i).to_text().ok_or_else(|| self.bad(i, "a string"))
    }

    pub fn opt_str(&self, i: usize) -> Result<Option<String>, ApiError> {
        match self.get(i) {
            Value::Nil => Ok(None),
            _ => self.str(i).map(Some),
        }
    }

    /// A flag: nil is `default`, anything else is Lua's truth.
    pub fn flag(&self, i: usize, default: bool) -> bool {
        match self.get(i) {
            Value::Nil => default,
            v => v.truthy(),
        }
    }

    pub fn callback(&self, i: usize) -> Result<u64, ApiError> {
        match self.get(i) {
            Value::Callback(id) => Ok(*id),
            _ => Err(self.bad(i, "a function")),
        }
    }

    /// A table (a list or a map), or nil.
    pub fn opt_table(&self, i: usize) -> Result<Option<&Value>, ApiError> {
        match self.get(i) {
            Value::Nil => Ok(None),
            v if v.is_table() => Ok(Some(v)),
            _ => Err(self.bad(i, "a table")),
        }
    }

    pub fn table(&self, i: usize) -> Result<&Value, ApiError> {
        self.opt_table(i)?.ok_or_else(|| self.bad(i, "a table"))
    }
}

/// What a function works on while it runs: the game, the calling plugin and the way back
/// into the plugin's own code. It holds them as pointers so that a function that calls the
/// plugin back (`emit` running the plugin's handlers, which call further functions) does not
/// keep a borrow alive across that call: [`Ctx::io`] and [`Ctx::state`] borrow the context
/// itself, so neither can be held over [`Ctx::invoke`].
pub struct Ctx<'a> {
    io: *mut (dyn PluginIo + 'a),
    state: *mut PluginState,
    binding: *const (dyn Binding + 'a),
    _life: std::marker::PhantomData<&'a mut ()>,
}

impl<'a> Ctx<'a> {
    /// # Safety
    /// The three must stay valid for `'a`, and nothing else may use them while a reference
    /// that this context handed out is alive (the bindings make one context per call into
    /// the API, from pointers set only for the length of the plugin's call).
    pub unsafe fn from_raw(io: *mut (dyn PluginIo + 'a), state: *mut PluginState, binding: *const (dyn Binding + 'a)) -> Ctx<'a> {
        Ctx { io, state, binding, _life: std::marker::PhantomData }
    }

    /// A context over borrowed parts (the runtime's own calls, the tests).
    pub fn new(io: &'a mut dyn PluginIo, state: &'a mut PluginState, binding: &'a dyn Binding) -> Ctx<'a> {
        Ctx { io, state, binding, _life: std::marker::PhantomData }
    }

    pub fn io(&mut self) -> &mut dyn PluginIo {
        // SAFETY: see `from_raw`; the borrow of `self` keeps it the only one
        unsafe { &mut *self.io }
    }

    pub fn state(&mut self) -> &mut PluginState {
        // SAFETY: as `io`
        unsafe { &mut *self.state }
    }

    /// Both at once (a function that reads the game into the plugin's own state).
    pub fn both(&mut self) -> (&mut dyn PluginIo, &mut PluginState) {
        // SAFETY: as `io`; the two are different objects
        unsafe { (&mut *self.io, &mut *self.state) }
    }

    /// Call one of the plugin's functions.
    pub fn invoke(&mut self, cb: u64, args: Vec<Value>) -> Result<Value, CallError> {
        // SAFETY: as `io`; the binding builds the nested calls' contexts from the same
        // pointers, while nothing of this one is borrowed
        unsafe { (*self.binding).invoke(cb, args) }
    }

    /// The plugin keeps a callback (a handler, a timer): it must not be forgotten.
    pub fn retain(&mut self, cb: u64) {
        unsafe { (*self.binding).retain(cb) }
    }

    /// The plugin no longer keeps a callback.
    pub fn release(&mut self, cb: u64) {
        unsafe { (*self.binding).release(cb) }
    }

    /// The language's own handler of an event (Lua's `on_<event>`).
    pub fn global(&mut self, event: &str, args: &[Value]) -> Result<bool, CallError> {
        unsafe { (*self.binding).global_handler(event, args) }
    }

    pub fn has_global(&mut self, event: &str) -> bool {
        unsafe { (*self.binding).has_global_handler(event) }
    }

    /// Run the plugin's handlers of an event (and its global `on_<event>` in Lua).
    pub fn emit(&mut self, event: &str, args: Vec<Value>) -> Result<(), CallError> {
        runtime::dispatch(self, event, args)
    }
}

/// Every function of the API, by name.
pub fn registry() -> &'static [&'static ApiFn] {
    use std::sync::OnceLock;
    static ALL: OnceLock<Vec<&'static ApiFn>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut v: Vec<&'static ApiFn> = fns::GROUPS.iter().flat_map(|g| g.iter()).collect();
        v.sort_by_key(|f| f.name);
        v
    })
}

/// A function by its name.
pub fn find(name: &str) -> Option<&'static ApiFn> {
    let r = registry();
    r.binary_search_by(|f| f.name.cmp(name)).ok().map(|i| r[i])
}

/// Call a function by name with the permission check (for bindings that call by name,
/// such as WASM).
pub fn call(ctx: &mut Ctx<'_>, name: &str, args: Vec<Value>) -> Result<Value, ApiError> {
    let f = find(name).ok_or_else(|| ApiError(format!("no function \"{name}\"")))?;
    call_fn(ctx, f, args)
}

/// Call a function with the permission check.
pub fn call_fn(ctx: &mut Ctx<'_>, f: &'static ApiFn, args: Vec<Value>) -> Result<Value, ApiError> {
    if f.perm != Perm::None && !ctx.state().allows(f.perm) {
        return Err(ApiError(format!("omsi.{}: the plugin has no \"{}\" permission", f.name, f.perm.as_str())));
    }
    (f.call)(ctx, Args::new(f, args))
}

/// The whole registry as JSON (PLUGIN_SPEC section 1), for `docs/plugin-api.json` and tools.
pub fn manifest() -> Value {
    let functions = registry()
        .iter()
        .map(|f| {
            let params = f.params.iter().map(|p| Value::map([("name", p.name.into()), ("type", p.ty.into()), ("optional", p.optional.into())])).collect::<Vec<_>>();
            Value::map([
                ("name", f.name.into()),
                ("group", f.group.into()),
                ("since", f.since.into()),
                ("doc", f.doc.into()),
                ("params", Value::List(params)),
                ("returns", f.returns.into()),
                ("multi", f.multi.into()),
                ("permission", Value::opt((f.perm != Perm::None).then(|| f.perm.as_str()))),
            ])
        })
        .collect::<Vec<_>>();
    let events = events::EVENTS
        .iter()
        .map(|e| Value::map([("name", e.name.into()), ("args", Value::List(e.args.iter().map(|a| Value::from(*a)).collect())), ("doc", e.doc.into()), ("since", e.since.into())]))
        .collect::<Vec<_>>();
    Value::map([("abi", ABI.into()), ("version", VERSION.into()), ("functions", Value::List(functions)), ("events", Value::List(events))])
}
