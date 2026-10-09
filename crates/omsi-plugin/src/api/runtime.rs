//! The plugins' runtime, the same for every language: event handlers (`on`/`off`/`emit`),
//! timers (`after`/`every`), watches, the events the game sends and those worked out from
//! what changed between two frames, the messages between plugins and each plugin's
//! errors. A binding ([`Binding`]) only has to call the plugin's functions back.

use super::{events, Ctx, Perm, Value};
use crate::ui::SharedUi;
use crate::PluginIo;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

/// Failed calls after which a plugin is switched off.
pub const MAX_ERRORS: u32 = 10;
/// Longest one call into a plugin may run: past it the plugin is stopped and switched off.
pub const CALL_BUDGET_MS: u64 = 50;
/// The same for loading it (its top level and the `start` event).
pub const LOAD_BUDGET_MS: u64 = 1000;
/// Messages kept for a plugin between two of its frames (the oldest go).
const MAX_MAIL: usize = 256;

/// How a binding calls a plugin's own functions. Its methods take `&self`: a call into the
/// plugin can come back into the API and from there call the plugin again (`emit` inside a
/// handler), so the binding is reached from several frames of the stack at once.
pub trait Binding {
    /// Call callback `cb` with `args`.
    fn invoke(&self, cb: u64, args: Vec<Value>) -> Result<Value, CallError>;
    /// The runtime keeps `cb` (a handler, a timer, a watch).
    fn retain(&self, _cb: u64) {}
    /// The runtime dropped `cb` (once per `retain`).
    fn release(&self, _cb: u64) {}
    /// A handler of the language's own for `event` besides `on` (Lua's global
    /// `on_<event>`): called after the others; Ok(false) when there is none.
    fn global_handler(&self, _event: &str, _args: &[Value]) -> Result<bool, CallError> {
        Ok(false)
    }
    /// Whether there is such a handler.
    fn has_global_handler(&self, _event: &str) -> bool {
        false
    }
}

/// A call into a plugin failed.
#[derive(Debug, Clone, PartialEq)]
pub struct CallError {
    pub msg: String,
    /// It ran past its time budget: the plugin is switched off at once.
    pub fatal: bool,
}

impl CallError {
    pub fn new(msg: impl Into<String>) -> CallError {
        CallError { msg: msg.into(), fatal: false }
    }
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

/// A message between plugins (`plugin.send`, `plugin.broadcast`), delivered as the `message`
/// event in the receiver's next frame.
#[derive(Debug, Clone)]
pub struct Mail {
    pub from: String,
    /// None: every plugin but the sender.
    pub to: Option<String>,
    pub topic: String,
    pub data: Value,
}

/// What the plugins share: the panels on the screen, the messages between them and who is
/// loaded.
#[derive(Default)]
pub struct Hub {
    pub ui: SharedUi,
    pub mail: RefCell<Vec<Mail>>,
    /// The loaded plugins' names, in load order.
    pub names: RefCell<Vec<String>>,
    /// Events for one plugin (by name) in its next frame: (plugin, event, arguments).
    pub direct: RefCell<Vec<(String, &'static str, Vec<Value>)>>,
}

pub type SharedHub = Rc<Hub>;

/// One event now, outside the frame (see `Plugins::emit`).
pub fn emit_now(ctx: &mut Ctx<'_>, event: &str, args: Vec<Value>) {
    if !ctx.state().disabled {
        unit(ctx, |c| dispatch(c, event, args));
    }
}

impl Hub {
    pub fn with_ui(ui: SharedUi) -> SharedHub {
        Rc::new(Hub { ui, ..Default::default() })
    }

    /// The messages waiting for plugin `name` (taken out).
    pub fn take_mail(&self, name: &str) -> Vec<Mail> {
        let mut all = self.mail.borrow_mut();
        if all.is_empty() {
            return Vec::new();
        }
        let (mine, rest): (Vec<Mail>, Vec<Mail>) = std::mem::take(&mut *all).into_iter().partition(|m| m.to.as_deref().is_some_and(|to| to.eq_ignore_ascii_case(name)));
        *all = rest;
        mine
    }

    /// Post a message: a broadcast becomes one copy per other loaded plugin.
    pub fn post(&self, m: Mail) {
        let mut all = self.mail.borrow_mut();
        let copies: Vec<Mail> = match &m.to {
            Some(_) => vec![m],
            None => self.names.borrow().iter().filter(|n| !n.eq_ignore_ascii_case(&m.from)).map(|n| Mail { to: Some(n.clone()), ..m.clone() }).collect(),
        };
        for c in copies {
            if all.len() >= MAX_MAIL {
                all.remove(0);
            }
            all.push(c);
        }
    }
}

/// A timer of `after`/`every`.
#[derive(Debug, Clone)]
struct Timer {
    at: f64,
    every: Option<f64>,
    cb: u64,
}

/// A watch: `cb(new, old)` when the value changes.
#[derive(Debug, Clone)]
struct Watch {
    kind: WatchKind,
    name: String,
    cb: u64,
    last: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchKind {
    Var,
    Str,
    Sys,
    /// A key of `omsi.info()`.
    Info,
}

impl WatchKind {
    pub fn parse(s: &str) -> Option<WatchKind> {
        Some(match s {
            "var" => WatchKind::Var,
            "str" => WatchKind::Str,
            "sys" => WatchKind::Sys,
            "info" => WatchKind::Info,
            _ => return None,
        })
    }
}

/// A key combination of `input.hotkey`.
#[derive(Debug, Clone)]
pub struct Hotkey {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub cb: u64,
}

/// One plugin's state, whatever its language.
pub struct PluginState {
    /// The plugin's name: its file name, or its folder's for a `main.lua`.
    pub name: String,
    /// "Lua plugin", "WASM plugin": how messages on the screen name it.
    pub label: &'static str,
    /// Its log tag: `[lua <name>]`.
    pub tag: String,
    /// Its mark on its panels and sounds.
    pub owner: u64,
    /// The permissions it declared; None: all (a plain `.lua` file, as ever).
    pub perms: Option<HashSet<Perm>>,
    /// Where its own files are (`plugin.read`, images, sounds): None for none.
    pub folder: Option<PathBuf>,
    /// Its data folder (`storage`, `files`), made when first written.
    pub data_dir: PathBuf,
    pub hub: SharedHub,
    pub clock: f64,
    handlers: HashMap<String, Vec<u64>>,
    timers: BTreeMap<u64, Timer>,
    watches: BTreeMap<u64, Watch>,
    pub hotkeys: BTreeMap<u64, Hotkey>,
    next_id: u64,
    /// The player's vehicle at the last frame (`vehicle` event).
    pub vehicle: Option<String>,
    /// `omsi.info()` at the last frame that looked (next_stop, view, duty events).
    last_info: Vec<(&'static str, crate::InfoValue)>,
    /// What the derived events last saw (see `events::Probe`).
    pub(crate) probes: events::ProbeState,
    pub focus_seen: bool,
    pub errors: u32,
    pub disabled: bool,
    /// The key/value storage, read on first use.
    pub storage: Option<super::fns::storage::Store>,
    /// `omsi.send`'s socket and rate.
    pub sender: super::fns::core::Sender,
    /// The plugin's settings (`plugin.settings`).
    pub settings: super::fns::plugin::Settings,
    /// Sounds it started (stopped when it stops).
    pub voices: Vec<u64>,
}

impl PluginState {
    pub fn new(name: &str, label: &'static str, tag: &str, owner: u64, hub: SharedHub, folder: Option<PathBuf>, data_dir: PathBuf) -> PluginState {
        PluginState {
            name: name.to_string(),
            label,
            tag: format!("[{tag} {name}]"),
            owner,
            perms: None,
            folder,
            data_dir,
            hub,
            clock: 0.0,
            handlers: HashMap::new(),
            timers: BTreeMap::new(),
            watches: BTreeMap::new(),
            hotkeys: BTreeMap::new(),
            next_id: 1,
            vehicle: None,
            last_info: Vec::new(),
            probes: Default::default(),
            focus_seen: false,
            errors: 0,
            disabled: false,
            storage: None,
            sender: Default::default(),
            settings: Default::default(),
            voices: Vec::new(),
        }
    }

    pub fn allows(&self, p: Perm) -> bool {
        self.perms.as_ref().is_none_or(|s| s.contains(&p))
    }

    pub fn ui(&self) -> std::cell::RefMut<'_, crate::ui::UiState> {
        self.hub.ui.borrow_mut()
    }

    fn id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Whether the plugin has a handler for `event` through `on`.
    pub fn listens(&self, event: &str) -> bool {
        self.handlers.get(event).is_some_and(|l| !l.is_empty())
    }

    /// A failed call: logged, shown, counted; switched off at `MAX_ERRORS` or when it ran
    /// past its budget.
    pub fn record_error(&mut self, io: &mut dyn PluginIo, e: &CallError) {
        self.errors += 1;
        log::warn!("{} {}", self.tag, e.msg);
        io.message(&format!("{} {}: {}", self.label, self.name, e.msg.lines().next().unwrap_or("")), 8.0);
        if e.fatal {
            log::warn!("{} ran longer than {CALL_BUDGET_MS} ms in one call: switched off until it changes or the game restarts", self.tag);
            self.disabled = true;
        } else if self.errors >= MAX_ERRORS {
            log::warn!("{} {MAX_ERRORS} errors: switched off until it changes or the game restarts", self.tag);
            self.disabled = true;
        }
        if self.disabled {
            // (its buttons would answer nothing any more)
            self.hub.ui.borrow_mut().remove_owner(self.owner);
        }
    }
}

// --- what the API's `on`, `after`, `watch`... do -----------------------------------------

pub fn on(ctx: &mut Ctx<'_>, event: &str, cb: u64) {
    ctx.state().handlers.entry(event.to_string()).or_default().push(cb);
    ctx.retain(cb);
}

pub fn off(ctx: &mut Ctx<'_>, event: &str, cb: u64) {
    let mut gone = 0;
    if let Some(list) = ctx.state().handlers.get_mut(event) {
        let n = list.len();
        list.retain(|&c| c != cb);
        gone = n - list.len();
    }
    for _ in 0..gone {
        ctx.release(cb);
    }
}

pub fn after(ctx: &mut Ctx<'_>, seconds: f64, every: bool, cb: u64) -> u64 {
    let s = ctx.state();
    let id = s.id();
    let at = s.clock + seconds;
    s.timers.insert(id, Timer { at, every: every.then_some(seconds), cb });
    ctx.retain(cb);
    id
}

pub fn watch(ctx: &mut Ctx<'_>, kind: WatchKind, name: String, cb: u64) -> u64 {
    let s = ctx.state();
    let id = s.id();
    s.watches.insert(id, Watch { kind, name, cb, last: Value::Nil });
    ctx.retain(cb);
    id
}

pub fn hotkey(ctx: &mut Ctx<'_>, key: Hotkey) -> u64 {
    let cb = key.cb;
    let s = ctx.state();
    let id = s.id();
    s.hotkeys.insert(id, key);
    ctx.retain(cb);
    id
}

/// Stop a timer, a watch or a hotkey; false when there was none of that id.
pub fn cancel(ctx: &mut Ctx<'_>, id: u64) -> bool {
    let s = ctx.state();
    let cb = s.timers.remove(&id).map(|t| t.cb).or_else(|| s.watches.remove(&id).map(|w| w.cb)).or_else(|| s.hotkeys.remove(&id).map(|h| h.cb));
    match cb {
        Some(cb) => {
            ctx.release(cb);
            true
        }
        None => false,
    }
}

/// Run every handler of `event` in the order they were added, then the language's own
/// (`on_<event>`); the first that fails stops the rest and is the error.
pub fn dispatch(ctx: &mut Ctx<'_>, event: &str, args: Vec<Value>) -> Result<(), CallError> {
    if let Some(list) = ctx.state().handlers.get(event).filter(|l| !l.is_empty()).cloned() {
        for cb in list {
            ctx.invoke(cb, args.clone())?;
        }
    }
    ctx.global(event, &args)?;
    Ok(())
}

/// Whether the plugin hears `event` at all (through `on` or its own global handler).
pub fn hears(ctx: &mut Ctx<'_>, event: &str) -> bool {
    ctx.state().listens(event) || ctx.has_global(event)
}

// --- the plugin's life -------------------------------------------------------------------

/// After the plugin's top level ran: the `start` event, then `vehicle` with the vehicle the
/// player drives.
pub fn start(ctx: &mut Ctx<'_>) -> Result<(), CallError> {
    let vehicle = {
        let io = ctx.io();
        io.vehicle_name().filter(|_| io.has_vehicle())
    };
    ctx.state().vehicle = vehicle.clone();
    let name = ctx_name(ctx);
    let hub = ctx.state().hub.clone();
    hub.names.borrow_mut().retain(|n| *n != name);
    hub.names.borrow_mut().push(name);
    dispatch(ctx, "start", Vec::new())?;
    if let Some(v) = vehicle {
        dispatch(ctx, "vehicle", vec![Value::Str(v)])?;
    }
    Ok(())
}

fn ctx_name(ctx: &mut Ctx<'_>) -> String {
    ctx.state().name.clone()
}

/// One unit of a frame: its error is recorded; false when the plugin is off now.
fn unit(ctx: &mut Ctx<'_>, f: impl FnOnce(&mut Ctx<'_>) -> Result<(), CallError>) -> bool {
    if let Err(e) = f(ctx) {
        let (io, s) = ctx.both();
        s.record_error(io, &e);
    }
    !ctx.state().disabled
}

/// One frame of a plugin: the `vehicle` event when the player's vehicle changed, the
/// game's events, the messages of other plugins, the panels' focus, clicks and changes,
/// then timers, watches, keys, the events worked out from what changed, and `frame`.
pub fn frame(ctx: &mut Ctx<'_>) {
    if ctx.state().disabled {
        return;
    }
    let now = {
        let io = ctx.io();
        io.vehicle_name().filter(|_| io.has_vehicle())
    };
    if now != ctx.state().vehicle {
        ctx.state().vehicle = now.clone();
        if !unit(ctx, |c| dispatch(c, "vehicle", vec![Value::opt(now)])) {
            return;
        }
    }
    for e in ctx.io().events() {
        let args = e.args.into_iter().map(Value::from).collect();
        if !unit(ctx, |c| dispatch(c, e.name, args)) {
            return;
        }
    }
    for e in ctx.io().events_ex() {
        if !unit(ctx, |c| dispatch(c, e.0, e.1)) {
            return;
        }
    }
    let name = ctx_name(ctx);
    let direct: Vec<(&'static str, Vec<Value>)> = {
        let hub = ctx.state().hub.clone();
        let mut all = hub.direct.borrow_mut();
        let mine = all.iter().filter(|d| d.0.eq_ignore_ascii_case(&name)).map(|d| (d.1, d.2.clone())).collect();
        all.retain(|d| !d.0.eq_ignore_ascii_case(&name));
        mine
    };
    for (event, args) in direct {
        if !unit(ctx, |c| dispatch(c, event, args)) {
            return;
        }
    }
    let mail = ctx.state().hub.take_mail(&name);
    for m in mail {
        if !unit(ctx, |c| dispatch(c, "message", vec![Value::Str(m.from), Value::Str(m.topic), m.data])) {
            return;
        }
    }
    // the panels getting or losing the mouse (`ui_focus`), and the clicks on this plugin's
    // panels (`ui_click`) and the changes of its sliders, boxes and fields (`ui_change`)
    let focused = ctx.state().ui().focused();
    if focused != ctx.state().focus_seen {
        ctx.state().focus_seen = focused;
        if !unit(ctx, |c| dispatch(c, "ui_focus", vec![Value::Bool(focused)])) {
            return;
        }
    }
    let owner = ctx.state().owner;
    let clicks = ctx.state().ui().take_clicks(owner);
    for c in clicks {
        if c.panel == super::fns::plugin::SETTINGS_PANEL {
            super::fns::plugin::settings_click(ctx, c.element.as_deref());
            continue;
        }
        if !unit(ctx, |x| dispatch(x, "ui_click", vec![Value::Str(c.panel), Value::opt(c.element)])) {
            return;
        }
    }
    let changes = ctx.state().ui().take_changes(owner);
    for c in changes {
        if c.panel == super::fns::plugin::SETTINGS_PANEL {
            if !unit(ctx, |x| super::fns::plugin::setting_changed(x, &c.element, c.value)) {
                return;
            }
            continue;
        }
        if !unit(ctx, |x| dispatch(x, "ui_change", vec![Value::Str(c.panel), Value::Str(c.element), c.value])) {
            return;
        }
    }
    let dt = ctx.io().dt() as f64;
    unit(ctx, |c| tick(c, dt));
}

/// The timers due, the watches, the keys, the events from what changed, then `frame`.
fn tick(ctx: &mut Ctx<'_>, dt: f64) -> Result<(), CallError> {
    let clock = {
        let s = ctx.state();
        s.clock += dt;
        s.clock
    };
    // timers due, in id order (a timer may add or cancel others)
    let due: Vec<u64> = ctx.state().timers.iter().filter(|(_, t)| t.at <= clock).map(|(id, _)| *id).collect();
    for id in due {
        let Some(t) = ctx.state().timers.get_mut(&id) else { continue };
        let cb = t.cb;
        let once = match t.every {
            Some(every) => {
                t.at += every;
                if t.at <= clock {
                    t.at = clock + every;
                }
                false
            }
            None => true,
        };
        if once {
            ctx.state().timers.remove(&id);
        }
        let r = ctx.invoke(cb, Vec::new());
        if once {
            ctx.release(cb);
        }
        r?;
    }
    // watches
    if !ctx.state().watches.is_empty() {
        let ids: Vec<u64> = ctx.state().watches.keys().copied().collect();
        for id in ids {
            let Some((kind, name)) = ctx.state().watches.get(&id).map(|w| (w.kind, w.name.clone())) else { continue };
            let v = read_watch(ctx, kind, &name);
            let Some(w) = ctx.state().watches.get_mut(&id) else { continue };
            if !same(&v, &w.last) {
                let old = std::mem::replace(&mut w.last, v.clone());
                let cb = w.cb;
                if !(old.is_nil() && v.is_nil()) {
                    ctx.invoke(cb, vec![v, old])?;
                }
            }
        }
    }
    // the keys pressed since the last frame
    let keys = ctx.io().keys();
    for (k, down) in keys {
        if down && !ctx.state().hotkeys.is_empty() {
            hotkeys(ctx, &k)?;
        }
        dispatch(ctx, "key", vec![Value::Str(k), Value::Bool(down)])?;
    }
    info_events(ctx)?;
    events::probe(ctx)?;
    dispatch(ctx, "frame", vec![Value::Num(dt)])
}

/// Values compared as Lua compares them (`~=`): a number is a number, whatever its kind.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Num(y)) | (Value::Num(y), Value::Int(x)) => *x as f64 == *y,
        _ => a == b,
    }
}

fn read_watch(ctx: &mut Ctx<'_>, kind: WatchKind, name: &str) -> Value {
    let io = ctx.io();
    match kind {
        WatchKind::Var => Value::opt(if io.has_vehicle() { io.var(name).map(|v| v as f64) } else { None }),
        WatchKind::Str => Value::opt(if io.has_vehicle() { io.string(name) } else { None }),
        WatchKind::Sys => Value::opt(io.system(name).map(|v| v as f64)),
        WatchKind::Info => io.info_value(name).map_or(Value::Nil, Value::from),
    }
}

/// `input.hotkey`: the combinations that key `k` completes.
fn hotkeys(ctx: &mut Ctx<'_>, k: &str) -> Result<(), CallError> {
    let held = |io: &mut dyn PluginIo, a: &str, b: &str| io.key_held(a) || io.key_held(b);
    let (ctrl, shift, alt) = {
        let io = ctx.io();
        (held(io, "ControlLeft", "ControlRight"), held(io, "ShiftLeft", "ShiftRight"), held(io, "AltLeft", "AltRight"))
    };
    let hits: Vec<u64> = ctx.state().hotkeys.values().filter(|h| h.key.eq_ignore_ascii_case(k) && h.ctrl == ctrl && h.shift == shift && h.alt == alt).map(|h| h.cb).collect();
    for cb in hits {
        ctx.invoke(cb, vec![Value::Str(k.to_string())])?;
    }
    Ok(())
}

/// `next_stop`, `view` and `duty`: what `omsi.info()` says changed (worked out only when the
/// plugin listens to one of them).
fn info_events(ctx: &mut Ctx<'_>) -> Result<(), CallError> {
    if !(hears(ctx, "next_stop") || hears(ctx, "view") || hears(ctx, "duty")) {
        return Ok(());
    }
    let now = ctx.io().info();
    let last = std::mem::take(&mut ctx.state().last_info);
    let get = |l: &[(&'static str, crate::InfoValue)], k: &str| -> Value { l.iter().find(|(key, _)| *key == k).map_or(Value::Nil, |(_, v)| v.clone().into()) };
    let text = |v: Value| -> String { v.to_text().unwrap_or_default() };
    let (stop, stop_old) = (get(&now, "next_stop"), get(&last, "next_stop"));
    // (also to a stop of the same name: the two sides of a road often share one)
    let r = (|| {
        if !stop.is_nil() && (!same(&stop, &stop_old) || !same(&get(&now, "next_stop_number"), &get(&last, "next_stop_number"))) {
            dispatch(ctx, "next_stop", vec![stop.clone(), stop_old.clone()])?;
        }
        let (view, view_old) = (get(&now, "view"), get(&last, "view"));
        if !same(&view, &view_old) {
            dispatch(ctx, "view", vec![view, view_old])?;
        }
        let duty = |l: &[(&'static str, crate::InfoValue)]| format!("{}/{}", text(get(l, "line")), text(get(l, "tour")));
        if duty(&now) != duty(&last) {
            dispatch(ctx, "duty", vec![get(&now, "line"), get(&now, "tour")])?;
        }
        Ok(())
    })();
    ctx.state().last_info = now;
    r
}

/// The plugin stops: the `stop` event, then its storage is written and its sounds end.
pub fn stop(ctx: &mut Ctx<'_>) {
    // (a plugin switched off hears it too: it may want to save what it has)
    if let Err(e) = dispatch(ctx, "stop", Vec::new()) {
        let (io, s) = ctx.both();
        s.record_error(io, &e);
    }
    finish(ctx);
}

/// What goes when a plugin goes, whether it stopped or failed to start: its storage is
/// written, its sounds stop, its panels go and it leaves the list of plugins.
pub fn finish(ctx: &mut Ctx<'_>) {
    super::fns::storage::flush(ctx.state());
    let voices = std::mem::take(&mut ctx.state().voices);
    for v in voices {
        ctx.io().sound_stop(v);
    }
    let owner = ctx.state().owner;
    ctx.state().ui().remove_owner(owner);
    let name = ctx_name(ctx);
    ctx.state().hub.names.borrow_mut().retain(|n| *n != name);
}
