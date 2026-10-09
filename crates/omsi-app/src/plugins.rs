//! The OMSI plugins of the content roots' `plugins` folders (see `omsi_plugin`), driven
//! every frame with the player's bus as OMSI drives them: system
//! variables, then the bus's variables, string variables and triggers.

use omsi_plugin::{GameEvent, HostConfig, InfoValue, PluginIo, Plugins};
use omsi_script::Host;
use omsi_script::SysVar;

/// Load every plugin of every content root (`OMSI_NO_PLUGINS=1` leaves them out).
pub(crate) fn load() -> Plugins {
    if omsi_cfg::flags::OMSI_NO_PLUGINS.is_set() {
        return Plugins::default();
    }
    // (never from content another machine sent: a LAN host's mods are data only)
    let dirs: Vec<std::path::PathBuf> = omsi_cfg::content_roots()
        .iter()
        .filter(|r| !omsi_cfg::is_sandbox(r))
        .filter_map(|r| omsi_plugin::resolve_path(r, "plugins"))
        .filter(|d| d.is_dir())
        .collect();
    if dirs.is_empty() {
        return Plugins::default();
    }
    Plugins::load(&dirs, &HostConfig::detect())
}

/// The game's side of a plugin frame: the whole game (the plugins taken out of it for the
/// length of their frame), and what the plugins asked for that waits until after it.
pub(crate) struct Io<'a> {
    pub app: Option<&'a mut crate::App>,
    /// Seconds since the last frame.
    pub dt: f32,
    /// A plugin's `omsi.message` without a game (the tests): the game shows it at once.
    pub message: Option<(String, f32)>,
    /// `omsi.info()`, worked out the first time a plugin asks in the frame (and again after
    /// a plugin changed what it says: the time, the duty, the place).
    info: std::cell::RefCell<Option<Vec<(&'static str, InfoValue)>>>,
    /// `omsi.command`: game menu lines to run after the frame.
    pub commands: Vec<String>,
    /// Keys pressed and let go since the last frame.
    pub keys: Vec<(String, bool)>,
    /// What happened since the last frame (see [`queue_event`]).
    pub events: Vec<GameEvent>,
    pub events_ex: Vec<(&'static str, Vec<omsi_plugin::api::Value>)>,
}

impl<'a> Io<'a> {
    pub(crate) fn new(app: &'a mut crate::App, dt: f32, keys: Vec<(String, bool)>, events: Vec<GameEvent>, events_ex: Vec<(&'static str, Vec<omsi_plugin::api::Value>)>) -> Io<'a> {
        Io { app: Some(app), dt, message: None, info: Default::default(), commands: Vec::new(), keys, events, events_ex }
    }

    /// An io without a game, with `info` as `omsi.info()` (the tests).
    #[cfg(test)]
    fn snapshot(info: Vec<(&'static str, InfoValue)>) -> Io<'static> {
        Io { app: None, dt: 0.0, message: None, info: std::cell::RefCell::new(Some(info)), commands: Vec::new(), keys: Vec::new(), events: Vec::new(), events_ex: Vec::new() }
    }

    fn app(&self) -> Option<&crate::App> {
        self.app.as_deref()
    }

    fn app_mut(&mut self) -> Option<&mut crate::App> {
        self.invalidate();
        self.app.as_deref_mut()
    }

    /// What `omsi.info()` says may have changed.
    fn invalidate(&self) {
        if self.app.is_some() {
            *self.info.borrow_mut() = None;
        }
    }

    fn veh(&self) -> Option<&omsi_sim::VehicleInstance> {
        self.app()?.player.as_ref().map(|p| &p.vehicle)
    }

    fn veh_mut(&mut self) -> Option<&mut omsi_sim::VehicleInstance> {
        self.app.as_deref_mut()?.player.as_mut().map(|p| &mut p.vehicle)
    }

    fn player_mut(&mut self) -> Option<&mut crate::player::Player> {
        self.app.as_deref_mut()?.player.as_mut()
    }

    /// The vehicles of `omsi.others`: the AI traffic, and the other players' buses by
    /// `(1 << 48) | their id`.
    fn other(&mut self, id: u64) -> Option<&mut omsi_sim::VehicleInstance> {
        let app = self.app.as_deref_mut()?;
        if id >> 48 == 1 {
            return app.net.remotes.remotes.get_mut(&((id & 0xFFFF_FFFF) as u32)).map(|r| r.vehicle_mut());
        }
        app.session.traffic.as_mut()?.cars.iter_mut().find(|c| c.id == id).map(|c| &mut c.vehicle)
    }
}

/// The most events kept for the plugins' next frame (the game paused, say): the oldest go.
const MAX_EVENTS: usize = 64;

/// Keep an event for the plugins' next frame (`App::plugin_events`).
pub(crate) fn queue_event(events: &mut Vec<GameEvent>, name: &'static str, args: Vec<InfoValue>) {
    if events.len() >= MAX_EVENTS {
        events.remove(0);
    }
    events.push(GameEvent { name, args });
}

/// Keep an event whose values are tables for the plugins' next frame.
pub(crate) fn queue_event_ex(events: &mut Vec<(&'static str, Vec<omsi_plugin::api::Value>)>, name: &'static str, args: Vec<omsi_plugin::api::Value>) {
    if events.len() >= MAX_EVENTS {
        events.remove(0);
    }
    events.push((name, args));
}

/// What the plugins' events last saw of the game: the pause and the menu, the people in
/// the player's bus, the players of the LAN session.
#[derive(Default)]
pub(crate) struct Seen {
    paused: bool,
    menu: bool,
    /// The people in the player's bus, by id, and the stop each waited at.
    riders: Option<hashbrown::HashMap<u32, Option<i64>>>,
    players: Option<hashbrown::HashMap<u32, String>>,
}

impl crate::App {
    /// What happened that the plugins hear about but cannot see by looking: the game
    /// pausing (they hear it at once, then stand still) and going on, the menu, people
    /// getting in and out of the bus, players joining and leaving, buttons of the
    /// controllers, the bus hitting AI cars (`plugin_impacts`).
    pub(crate) fn plugin_watch(&mut self, plugins: &mut Plugins) {
        use omsi_plugin::api::Value;
        let menu = self.menus.game_menu.is_some();
        let mut seen = std::mem::take(&mut self.integrations.plugin_seen);
        if self.paused != seen.paused {
            seen.paused = self.paused;
            if self.paused {
                let mut io = Io::new(self, 0.0, Vec::new(), Vec::new(), Vec::new());
                if menu && !seen.menu {
                    plugins.emit(&mut io, "menu_open", Vec::new());
                }
                plugins.emit(&mut io, "pause", Vec::new());
                seen.menu = menu;
            } else {
                queue_event_ex(&mut self.integrations.plugin_events_ex, "resume", Vec::new());
            }
        }
        if menu != seen.menu && !self.paused {
            seen.menu = menu;
            queue_event_ex(&mut self.integrations.plugin_events_ex, if menu { "menu_open" } else { "menu_close" }, Vec::new());
        }
        let ev = &mut self.integrations.plugin_events_ex;
        // people in and out of the player's bus: who is in it now, against the last frame
        if plugins.hears("passenger_board") || plugins.hears("passenger_alight") {
            use omsi_sim::people::{BusId, State};
            let now: hashbrown::HashMap<u32, Option<i64>> = self
                .session
                .humans
                .as_ref()
                .map(|h| h.people.iter().filter(|p| p.inside(BusId::Player)).map(|p| (p.id, match &p.state { State::Pax(x) => x.stop, _ => None })).collect())
                .unwrap_or_default();
            if let Some(before) = seen.riders.as_ref() {
                let mut boarded: hashbrown::HashMap<Option<i64>, i64> = hashbrown::HashMap::new();
                for (id, stop) in &now {
                    if !before.contains_key(id) {
                        *boarded.entry(*stop).or_default() += 1;
                    }
                }
                for (stop, n) in boarded {
                    queue_event_ex(ev, "passenger_board", vec![Value::Int(n), Value::opt(stop)]);
                }
                // (somebody gone from the people altogether was taken away, not let out)
                let people: hashbrown::HashSet<u32> = self.session.humans.as_ref().map(|h| h.people.iter().map(|p| p.id).collect()).unwrap_or_default();
                let out = before.keys().filter(|id| !now.contains_key(*id) && people.contains(*id)).count();
                if out > 0 {
                    queue_event_ex(ev, "passenger_alight", vec![Value::Int(out as i64)]);
                }
            }
            seen.riders = Some(now);
        } else {
            seen.riders = None;
        }
        if plugins.hears("lan_join") || plugins.hears("lan_leave") {
            let now: hashbrown::HashMap<u32, String> = self.net.lan.as_ref().map(|l| l.peers().filter(|p| p.has_info).map(|p| (p.pose.id, p.pose.name.clone())).collect()).unwrap_or_default();
            if let Some(before) = seen.players.as_ref() {
                for (id, name) in &now {
                    if !before.contains_key(id) {
                        queue_event_ex(ev, "lan_join", vec![Value::Int(*id as i64), Value::Str(name.clone())]);
                    }
                }
                for (id, name) in before {
                    if !now.contains_key(id) {
                        queue_event_ex(ev, "lan_leave", vec![Value::Int(*id as i64), Value::Str(name.clone())]);
                    }
                }
            }
            seen.players = Some(now);
        } else {
            seen.players = None;
        }
        if plugins.hears("controller_button") {
            if let Some(c) = self.input.controllers.as_ref() {
                for (dev, n, down) in &c.raw_buttons {
                    queue_event_ex(ev, "controller_button", vec![Value::Str(dev.clone()), Value::Int(*n as i64), Value::Bool(*down)]);
                }
            }
        }
        for (name, text) in std::mem::take(&mut self.net.remotes.plugin_chat) {
            if plugins.hears("lan_chat") {
                queue_event_ex(ev, "lan_chat", vec![Value::Str(name), Value::Str(text)]);
            }
        }
        self.integrations.plugin_seen = seen;
    }

    /// A message of a plugin on another player's game (`lan.send`): to the plugin of the
    /// same name here. True when the text was one.
    pub(crate) fn plugin_lan_message(&mut self, from: u32, text: &str) -> bool {
        let Some(rest) = text.strip_prefix(crate::plugin_io::world::LAN_PREFIX) else { return false };
        let (name, msg) = rest.split_once(' ').unwrap_or((rest, ""));
        if let Some(p) = self.integrations.plugins.as_ref().filter(|p| p.has(name)) {
            use omsi_plugin::api::Value;
            p.post(name, "lan_message", vec![Value::Int(from as i64), Value::Str(msg.to_string())]);
        }
        true
    }
}

/// The player's bus hit AI cars this frame (before the hits are handed to the traffic):
/// an `ai_collision` event each, for the plugins.
pub(crate) fn plugin_impacts(veh: &omsi_sim::VehicleInstance, integ: &mut crate::app::Integrations) {
    if veh.dynamic_impacts.is_empty() || !integ.plugins.as_ref().is_some_and(|x| x.hears("ai_collision")) {
        return;
    }
    use omsi_plugin::api::Value;
    for i in veh.dynamic_impacts.iter().filter(|i| i.obstacle_id <= -2) {
        let args = vec![Value::Int(-i.obstacle_id - 2), Value::from(i.energy / 1000.0)];
        queue_event_ex(&mut integ.plugin_events_ex, "ai_collision", args);
    }
}

/// A game value kept as `f32`, for Lua as the number it reads as (2.1, not
/// 2.0999999046325684).
pub(crate) fn num_f32(x: f32) -> InfoValue {
    InfoValue::Num(x.to_string().parse().unwrap_or(x as f64))
}

/// The game menu lines a plugin may run with `omsi.command` (those that do something at
/// once, not the ones that open a list).
pub(crate) const PLUGIN_COMMANDS: [&str; 14] = ["refuel", "wash", "repair", "shot", "save", "load", "weather", "later", "earlier", "info", "timetable", "reset", "couple", "uncouple"];

/// What the game is doing, for `omsi.info()`.
pub(crate) fn game_info(app: &crate::App) -> Vec<(&'static str, InfoValue)> {
    use InfoValue::{Bool, Num, Text};
    let mut v: Vec<(&'static str, InfoValue)> = Vec::new();
    v.push(("map", Text(app.world.as_ref().map(|w| w.global.name.clone()).unwrap_or_default())));
    v.push(("clock", Num(app.clock.time)));
    v.push(("day", Num(app.clock.day_of_year as f64)));
    v.push(("year", Num(app.clock.year as f64)));
    v.push(("view", Text(app.view.clone())));
    v.push(("paused", Bool(app.paused)));
    v.push(("on_foot", Bool(app.session.on_foot.is_some())));
    v.push(("multiplayer", Bool(app.net.lan.is_some())));
    // the situation the game started from (the launcher's "continue": `laststn.osn`)
    // (relative to the OMSI folder, `/`-separated, whether the launcher passed it absolute or not)
    if let Some(s) = app.args.situation.as_ref() {
        let p = std::path::Path::new(s);
        let rel = p.strip_prefix(&app.args.root).unwrap_or(p);
        v.push(("situation", Text(rel.to_string_lossy().replace('\\', "/"))));
    }
    // this session's, as the personnel file counts them
    v.push(("crashes", Num(app.session.career.crashes[0] as f64)));
    v.push(("heavy_crashes", Num(app.session.career.crashes[3] as f64)));
    v.push(("pedestrians_hit", Num(app.session.career.crashes[1] as f64)));
    if let Some(t) = app.session.traffic.as_ref() {
        v.push(("traffic", Num(t.cars.len() as f64)));
    }
    if let Some(w) = app.world.as_ref() {
        v.push(("map_path", Text(w.global.path.to_string_lossy().into_owned())));
    }
    v.push(("version", Text(crate::startup::VERSION.to_string())));
    if let Some(p) = app.player.as_ref() {
        let veh = &p.vehicle;
        v.push(("speed", Num(veh.physics.velocity_kmh().abs() as f64)));
        v.push(("delay", Num(veh.host.tt_delay as f64)));
        // Tile coordinates from global.cfg and metres within the tile (x east, y north).
        let ((tx, ty), (lx, ly)) = omsi_map::world_to_tile_local(veh.position.x, veh.position.y);
        v.push(("tile_x", Num(tx as f64)));
        v.push(("tile_y", Num(ty as f64)));
        v.push(("tile_pos_x", Num(lx)));
        v.push(("tile_pos_y", Num(ly)));
        v.push(("heading", Num(veh.heading.rem_euclid(360.0))));
        v.push(("vehicle_manufacturer", Text(veh.ty.def.manufacturer.trim().to_string())));
        v.push(("vehicle_model", Text(veh.ty.def.type_name.trim().to_string())));
        // the terminus the bus shows (the hof entry its scripts chose; none for an
        // `[addterminus_allexit]` one), which is not always the timetable's
        let shown = match (veh.var("target_index_int"), veh.host.hof.as_ref()) {
            (Some(i), Some(hof)) if i.is_finite() && i >= 0.0 => hof.termini.get(i.round() as usize).filter(|t| !t.all_exit).map(|t| t.texture_id.trim().to_string()),
            _ => None,
        };
        v.push(("destination", Text(shown.unwrap_or_default())));
        v.push(("passengers", Num(app.session.humans.as_ref().map(|h| h.riding()).unwrap_or(0) as f64)));
    }
    if let Some(d) = app.session.duty.as_ref() {
        v.push(("line", Text(d.line.trim().to_string())));
        v.push(("tour", Text(d.tour.trim().to_string())));
        if let Some(trip) = d.trips.get(d.trip_index) {
            v.push(("trip", Num(d.trip_index as f64 + 1.0)));
            v.push(("trips", Num(d.trips.len() as f64)));
            v.push(("terminus", Text(trip.terminus.trim().to_string())));
            v.push(("trip_name", Text(trip.name.trim().to_string())));
            v.push(("stops", Num(trip.stops.len() as f64)));
            // the bus reached the trip's last stop: the trip is over, though the duty moves on to
            // the next one only a minute before it leaves
            v.push(("trip_done", Bool(d.trip_done())));
            let bus = app.player.as_ref().map(|p| p.vehicle.position);
            stop_info(&mut v, &trip.stops, d.next_stop, d.at_stop(), bus);
        }
    }
    v
}

/// `omsi.info()`'s keys of the next stop and the one before it: `stops` the trip's, `next`
/// the index of the next one, `bus` where the player's bus is.
fn stop_info(v: &mut Vec<(&'static str, InfoValue)>, stops: &[crate::schedule::PlannedStop], next: usize, at_stop: bool, bus: Option<glam::DVec3>) {
    use InfoValue::{Bool, Num, Text};
    // straight-line metres from the bus, where the stop's place is known
    let distance = |s: &crate::schedule::PlannedStop| {
        let (bus, stop) = (bus?, s.position?);
        Some((stop.x - bus.x).hypot(stop.y - bus.y))
    };
    if let Some(s) = stops.get(next) {
        v.push(("next_stop", Text(s.name.trim().to_string())));
        v.push(("next_stop_number", Num(next as f64 + 1.0)));
        v.push(("next_stop_arrival", Num(s.arr)));
        v.push(("next_stop_departure", Num(s.dep)));
        // the map's object ID of the stop, as the timetable and the map files name it:
        // outside tools match the stop by it, the name can occur twice
        v.push(("next_stop_id", Num(s.object_id as f64)));
        v.push(("at_stop", Bool(at_stop)));
        if let Some(m) = distance(s) {
            v.push(("next_stop_distance", Num(m)));
        }
    }
    // the last stop before the next one the trip calls at (passing stations left out)
    if let Some(s) = stops.iter().take(next).rev().find(|s| s.stops) {
        v.push(("previous_stop", Text(s.name.trim().to_string())));
        v.push(("previous_stop_id", Num(s.object_id as f64)));
        if let Some(m) = distance(s) {
            v.push(("previous_stop_distance", Num(m)));
        }
    }
}

impl Io<'_> {
    /// The `omsi.info()` key of an `.opl` list's `openomsi_<key>` name (any case).
    fn game_key(name: &str) -> Option<&str> {
        name.get(..9).filter(|p| p.eq_ignore_ascii_case("openomsi_")).map(|_| &name[9..])
    }

    /// The value of `omsi.info()` an `.opl` list names as `openomsi_<key>` (any case).
    fn game_value(&self, name: &str) -> Option<InfoValue> {
        let key = Self::game_key(name)?;
        if self.info.borrow().is_none() {
            let _ = self.info();
        }
        self.info.borrow().as_ref()?.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.clone())
    }

    fn game_number(&self, name: &str) -> Option<f32> {
        let value = match self.game_value(name)? {
            InfoValue::Num(n) => n as f32,
            InfoValue::Bool(b) => u8::from(b) as f32,
            InfoValue::Text(_) | InfoValue::Nil => return None,
        };
        value.is_finite().then_some(value)
    }

    fn game_string(&self, name: &str) -> Option<String> {
        match self.game_value(name)? {
            InfoValue::Text(t) => Some(t),
            _ => None,
        }
    }
}

use crate::plugin_io::{bus as pbus, duty as pduty, world as pworld};
use omsi_plugin as op;

impl PluginIo for Io<'_> {
    fn system(&mut self, name: &str) -> Option<f32> {
        let v = SysVar::from_name(name)?;
        self.veh_mut().map(|veh| veh.host.sys_var(v))
    }

    fn set_system(&mut self, name: &str, v: f32) {
        // the clock, the weather and the input are the game's own; a plugin writing them
        // is told nothing, as the scripts' S.S. writes are not honoured either
        log::debug!("plugin wrote system variable {name} = {v} (kept as it is)");
    }

    fn has_vehicle(&self) -> bool {
        self.veh().is_some()
    }

    fn var(&mut self, name: &str) -> Option<f32> {
        if let Some(v) = self.veh()?.var(name) {
            return Some(v);
        }
        // A script variable takes precedence over the read-only game snapshot.
        self.game_number(name)
    }

    fn set_var(&mut self, name: &str, v: f32) {
        // (the game's values are read-only: no script variable is made under their names)
        if Self::game_key(name).is_some() {
            return;
        }
        if let Some(veh) = self.veh_mut() {
            veh.set_var(name, v);
        }
    }

    fn string(&mut self, name: &str) -> Option<String> {
        let veh = self.veh()?;
        if let Some(i) = veh.ty.program.str_var(name) {
            return veh.state.str_vars.get(i as usize).cloned();
        }
        self.game_string(name)
    }

    fn set_string(&mut self, name: &str, s: &str) {
        if let Some(veh) = self.veh_mut() {
            if let Some(i) = veh.ty.program.str_var(name) {
                if let Some(slot) = veh.state.str_vars.get_mut(i as usize) {
                    *slot = s.to_string();
                }
            }
        }
    }

    /// A key down fires the trigger, a key up `<trigger>_off` (OMSI's keyboard event
    /// handler the original, which the plugin frame calls with the new state).
    fn fire(&mut self, trigger: &str, down: bool) {
        if let Some(veh) = self.veh_mut() {
            if down {
                veh.trigger(trigger);
            } else {
                veh.trigger(&format!("{trigger}_off"));
            }
        }
    }

    fn dt(&self) -> f32 {
        self.dt
    }

    fn vehicle_name(&self) -> Option<String> {
        self.veh().map(|v| format!("{} {}", v.ty.def.manufacturer, v.ty.def.type_name).trim().to_string())
    }

    fn vehicle_manufacturer_model(&self) -> Option<(String, String)> {
        self.veh().map(|v| (v.ty.def.manufacturer.trim().to_string(), v.ty.def.type_name.trim().to_string()))
    }

    fn position(&self) -> Option<[f64; 4]> {
        self.veh().map(|v| [v.position.x, v.position.y, v.position.z, v.heading])
    }

    fn message(&mut self, text: &str, seconds: f32) {
        match self.app.as_deref_mut() {
            Some(app) => app.service_msg = Some((text.to_string(), seconds)),
            None => self.message = Some((text.to_string(), seconds)),
        }
    }

    fn info(&self) -> Vec<(&'static str, InfoValue)> {
        let mut cache = self.info.borrow_mut();
        if cache.is_none() {
            *cache = Some(self.app().map(game_info).unwrap_or_default());
        }
        cache.clone().unwrap_or_default()
    }

    fn info_value(&self, key: &str) -> Option<InfoValue> {
        if self.info.borrow().is_none() {
            let _ = self.info();
        }
        self.info.borrow().as_ref()?.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())
    }

    fn command(&mut self, what: &str) -> bool {
        let what = what.trim().to_ascii_lowercase();
        if !PLUGIN_COMMANDS.contains(&what.as_str()) || self.commands.len() >= 8 {
            return false;
        }
        self.commands.push(what);
        true
    }

    fn var_names(&self) -> (Vec<String>, Vec<String>) {
        match self.veh() {
            Some(v) => (v.ty.program.var_names.clone(), v.ty.program.str_var_names.clone()),
            None => (Vec::new(), Vec::new()),
        }
    }

    fn keys(&self) -> Vec<(String, bool)> {
        self.keys.clone()
    }

    fn events(&self) -> Vec<GameEvent> {
        self.events.clone()
    }

    fn events_ex(&self) -> Vec<(&'static str, Vec<omsi_plugin::api::Value>)> {
        self.events_ex.clone()
    }

    fn others(&self, radius: f64) -> Vec<omsi_plugin::Other> {
        let (Some(app), Some(me)) = (self.app(), self.veh().map(|v| v.position)) else {
            return Vec::new();
        };
        let near = |v: &omsi_sim::VehicleInstance| (v.position.x - me.x).powi(2) + (v.position.y - me.y).powi(2) <= radius * radius;
        let other = |id: u64, kind: &'static str, v: &omsi_sim::VehicleInstance| omsi_plugin::Other { id, kind, name: format!("{} {}", v.ty.def.manufacturer, v.ty.def.type_name).trim().to_string(), pos: [v.position.x, v.position.y, v.position.z, v.heading] };
        let mut out: Vec<omsi_plugin::Other> = app.session.traffic.as_ref().map(|t| t.cars.iter().filter(|c| near(&c.vehicle)).map(|c| other(c.id, "ai", &c.vehicle)).collect()).unwrap_or_default();
        out.extend(app.net.remotes.remotes.iter().filter(|(_, r)| near(r.vehicle())).map(|(id, r)| other((1u64 << 48) | *id as u64, "player", r.vehicle())));
        out
    }

    fn other_var(&mut self, id: u64, name: &str) -> Option<f32> {
        self.other(id)?.var(name)
    }

    fn set_other_var(&mut self, id: u64, name: &str, v: f32) -> bool {
        self.other(id).is_some_and(|o| o.set_var(name, v))
    }

    // --- the player's bus ---
    fn bus_velocity(&self) -> Option<[f64; 3]> {
        pbus::velocity(self.app()?)
    }
    fn bus_acceleration(&self) -> Option<[f64; 3]> {
        self.veh().map(|v| [v.physics.a_trans.x as f64, v.physics.a_trans.y as f64, v.physics.a_trans.z as f64])
    }
    fn bus_orientation(&self) -> Option<[f64; 3]> {
        self.veh().map(|v| [v.heading.rem_euclid(360.0), v.pitch as f64, v.bank as f64])
    }
    fn bus_mass(&self) -> Option<f64> {
        self.veh().map(|v| v.physics.mass_kg as f64)
    }
    fn bus_km(&self) -> Option<(f64, f64)> {
        let app = self.app()?;
        Some((pbus::veh(app)?.odometer_km(), app.session.career.metres / 1000.0))
    }
    fn bus_doors(&self) -> Vec<f32> {
        self.app().map(pbus::doors).unwrap_or_default()
    }
    fn bus_doors_open(&self) -> Option<bool> {
        pbus::doors_open(self.app()?)
    }
    fn bus_door_count(&self) -> usize {
        self.veh().map_or(0, |v| crate::player::door_keys(&v.ty).len())
    }
    fn bus_door_key(&mut self, n: usize) -> bool {
        self.app_mut().is_some_and(|a| pbus::door_key(a, n))
    }
    fn bus_indicator(&self) -> Option<u8> {
        self.veh().map(crate::lan::indicator)
    }
    fn bus_set_indicator(&mut self, want: u8) -> bool {
        let Some(p) = self.player_mut() else { return false };
        if crate::lan::indicator(&p.vehicle) != want {
            p.toggle_indicator(want);
        }
        true
    }
    fn bus_headlights(&self) -> Option<u8> {
        pbus::headlights(self.app()?)
    }
    fn bus_interior_light(&self) -> Option<f32> {
        self.veh().map(|v| v.interior_light())
    }
    fn bus_set_interior_light(&mut self, on: bool) -> bool {
        self.player_mut().is_some_and(|p| !p.set_saloon_lights(on).is_empty())
    }
    fn bus_engine(&self) -> Option<(bool, Option<f32>, bool)> {
        pbus::engine(self.app()?)
    }
    fn bus_start_up(&mut self) -> Option<String> {
        self.player_mut().map(|p| p.start_up())
    }
    fn bus_gear(&self) -> Option<f32> {
        pbus::gear(self.app()?)
    }
    fn bus_shift(&mut self, gear: i32) -> bool {
        self.player_mut().is_some_and(|p| p.shift_gate_to(gear))
    }
    fn bus_dirt(&self) -> Option<f32> {
        self.veh().map(|v| v.dirt)
    }
    fn bus_damage(&mut self) -> Option<(u32, f32, Option<f32>)> {
        let v = self.veh_mut()?;
        let minutes = v.repair_minutes().filter(|m| *m > 0.0);
        Some((v.crashes, v.last_impact, minutes))
    }
    fn bus_controls(&self) -> Option<[f32; 4]> {
        self.veh().map(|v| {
            let c = &v.physics.controls;
            [c.throttle, c.brake, c.clutch, c.steering]
        })
    }
    fn bus_steer(&self) -> Option<(f32, f32)> {
        self.veh().map(|v| (v.physics.steer_deg, v.physics.max_steer_deg))
    }
    fn bus_action(&mut self, name: &str, down: bool) -> bool {
        self.player_mut().is_some_and(|p| p.action(name, down))
    }
    fn bus_trailers(&self) -> Option<usize> {
        self.veh().map(|v| v.trailers.len())
    }
    fn bus_destinations(&self) -> (Vec<op::Terminus>, Option<i64>) {
        self.app().map(pbus::destinations).unwrap_or_default()
    }
    fn bus_set_destination(&mut self, index: usize) -> bool {
        self.app_mut().is_some_and(|a| pbus::set_destination(a, index))
    }
    fn bus_set_line(&mut self, line: &str) -> bool {
        let Some(app) = self.app_mut() else { return false };
        if app.player.is_none() || line.trim().is_empty() {
            return false;
        }
        crate::game_lists::set_route_by_hand(app, line);
        true
    }
    fn bus_ident(&self) -> Option<(String, String)> {
        let app = self.app()?;
        let v = pbus::veh(app)?;
        let file = v.ty.def.path.strip_prefix(&app.args.root).unwrap_or(&v.ty.def.path).to_string_lossy().replace('\\', "/");
        Some((v.number(), file))
    }
    fn bus_passengers(&self) -> Option<(usize, usize, usize)> {
        pbus::passengers(self.app()?)
    }
    fn bus_tickets(&self) -> (Vec<op::Ticket>, Option<(String, f32)>) {
        self.app().map(pbus::tickets).unwrap_or_default()
    }
    fn bus_sales(&self) -> Option<(u32, f64)> {
        self.app()?.session.humans.as_ref().map(|h| (h.tickets_sold, h.ticket_cash as f64))
    }
    fn bus_wheels(&self) -> Vec<op::Wheel> {
        self.app().map(pbus::wheels).unwrap_or_default()
    }
    fn bus_triggers(&self) -> Vec<String> {
        self.veh().map(|v| v.ty.program.trigger_names()).unwrap_or_default()
    }
    fn bus_sound(&mut self, event: &str) -> bool {
        self.app_mut().is_some_and(|a| pbus::sound(a, event))
    }

    // --- traffic and people ---
    fn traffic_list(&self) -> Vec<op::AiCar> {
        self.app().map(pbus::traffic_list).unwrap_or_default()
    }
    fn traffic_counts(&self) -> Option<[usize; 4]> {
        let (cars, buses, dormant, parked) = self.app()?.session.traffic.as_ref()?.counts();
        Some([cars, buses, dormant, parked])
    }
    fn traffic_density(&self) -> Option<(usize, f32)> {
        let t = self.app()?.session.traffic.as_ref()?;
        Some((t.target, t.unsched_factor))
    }
    fn traffic_set_density(&mut self, cars: Option<usize>, share: Option<f32>) -> bool {
        let Some(app) = self.app_mut() else { return false };
        if app.net.lan.as_ref().is_some_and(|l| l.role == omsi_net::Role::Client) {
            return false;
        }
        let Some(t) = app.session.traffic.as_mut() else { return false };
        if let Some(n) = cars {
            t.target = n;
            app.args.traffic = n;
        }
        if let Some(s) = share {
            t.unsched_factor = s;
        }
        true
    }
    fn traffic_remove(&mut self, id: u64) -> bool {
        self.app_mut().is_some_and(|a| pbus::traffic_remove(a, id))
    }
    fn traffic_clear(&mut self) -> Option<usize> {
        pbus::traffic_clear(self.app_mut()?)
    }
    fn traffic_buses(&self) -> Vec<op::AiBus> {
        self.app().map(pduty::ai_buses).unwrap_or_default()
    }
    fn traffic_light_ahead(&self, reach: f64) -> Option<op::Light> {
        pbus::light_ahead(self.app()?, reach)
    }
    fn people_counts(&self) -> Option<[usize; 3]> {
        let (w, s, a) = self.app()?.session.humans.as_ref()?.counts();
        Some([w, s, a])
    }
    fn people_list(&self) -> Vec<op::Person> {
        self.app().map(pbus::people).unwrap_or_default()
    }
    fn people_stops(&self) -> Vec<op::PaxStop> {
        self.app().map(pbus::pax_stops).unwrap_or_default()
    }
    fn people_density(&self) -> Option<f32> {
        self.app().map(|a| a.settings.pax_density)
    }
    fn people_set_density(&mut self, v: f32) -> bool {
        let Some(app) = self.app_mut() else { return false };
        app.settings.pax_density = v;
        true
    }

    // --- duty, timetable, map ---
    fn duty(&self) -> Option<op::Duty> {
        pduty::duty(self.app()?)
    }
    fn duty_trips(&self) -> Vec<op::Trip> {
        self.app().map(pduty::trips).unwrap_or_default()
    }
    fn duty_skip_next(&mut self) -> Option<String> {
        pduty::skip_next(self.app_mut()?)
    }
    fn duty_skip_to(&mut self, index: usize) -> bool {
        self.app_mut().is_some_and(|a| pduty::skip_to(a, index))
    }
    fn duty_start(&mut self, line: &str, tour: &str, trip: usize, stop: usize) -> Result<(), String> {
        pduty::start(self.app_mut().ok_or("no game")?, line, tour, trip, stop)
    }
    fn duty_end(&mut self) -> bool {
        self.app_mut().is_some_and(pduty::finish)
    }
    fn timetable_lines(&self) -> Vec<op::Line> {
        self.app().map(pduty::lines).unwrap_or_default()
    }
    fn timetable_stops(&self, line: &str, tour: &str, trip: Option<usize>) -> Vec<(usize, usize, String, f64)> {
        self.app().map(|a| pduty::tour_stops(a, line, tour, trip)).unwrap_or_default()
    }
    fn timetable_stop_names(&self) -> Vec<(i64, String)> {
        self.app().and_then(|a| a.session.schedule.as_ref()).map(|s| s.data.bus_stops.iter().map(|b| (b.object_id, b.name.trim().to_string())).collect()).unwrap_or_default()
    }
    fn map_info(&self) -> Option<op::MapInfo> {
        pduty::map_info(self.app()?)
    }
    fn map_tiles(&self) -> Vec<op::Tile> {
        self.app().map(pduty::tiles).unwrap_or_default()
    }
    fn map_tile_at(&self, x: f64, y: f64) -> Option<((i32, i32), (f64, f64))> {
        self.app()?.world.as_ref()?;
        Some(omsi_map::world_to_tile_local(x, y))
    }
    fn map_from_tile(&self, tx: i32, ty: i32, lx: f64, ly: f64) -> Option<(f64, f64)> {
        self.app()?.world.as_ref()?;
        Some(omsi_map::tile_local_to_world(tx, ty, lx, ly))
    }
    fn map_stops(&self) -> Vec<op::MapStop> {
        self.app().map(pduty::stops).unwrap_or_default()
    }
    fn map_object(&self, id: i64) -> Option<[f64; 4]> {
        pduty::object(self.app()?, id)
    }
    fn map_objects_near(&self, x: f64, y: f64, r: f64) -> Vec<(i64, [f64; 4])> {
        self.app().map(|a| pduty::objects_near(a, x, y, r)).unwrap_or_default()
    }
    fn map_ground(&self, x: f64, y: f64) -> (Option<f64>, Option<f64>) {
        self.app().and_then(|a| a.world.as_ref()).map(|w| (w.ground_height(x, y), w.ground_terrain(x, y))).unwrap_or_default()
    }
    fn map_entrypoints(&self) -> Vec<op::Entry> {
        self.app().map(pduty::entrypoints).unwrap_or_default()
    }
    fn map_teleport(&mut self, at: [f64; 3], heading: f64) -> bool {
        self.app_mut().is_some_and(|a| pduty::teleport(a, at, heading))
    }
    fn map_teleport_entry(&mut self, index: usize) -> bool {
        self.app_mut().is_some_and(|a| pduty::teleport_entry(a, index))
    }
    fn map_place_on_road(&mut self, x: f64, y: f64) -> bool {
        self.app_mut().is_some_and(|a| pduty::place_on_road(a, x, y))
    }
    fn map_lane(&self, x: f64, y: f64) -> Option<op::Lane> {
        pduty::lane(self.app()?, x, y)
    }

    // --- time and weather ---
    fn clock(&self) -> Option<op::Clock> {
        self.app().map(pworld::clock)
    }
    fn set_time(&mut self, seconds: f64) -> bool {
        self.app_mut().is_some_and(|a| pworld::set_time(a, seconds))
    }
    fn set_date(&mut self, y: i32, m: u32, d: u32) -> bool {
        self.app_mut().is_some_and(|a| pworld::set_date(a, y, m, d))
    }
    fn time_speed(&self) -> Option<f64> {
        self.app().map(|a| a.time_speed())
    }
    fn set_time_speed(&mut self, x: f64) -> bool {
        let Some(app) = self.app_mut() else { return false };
        if app.net.lan.is_some() || app.real_time_locked() {
            return false;
        }
        app.settings.time_speed = x;
        true
    }
    fn paused(&self) -> bool {
        self.app().is_some_and(|a| a.paused)
    }
    fn set_paused(&mut self, on: bool) -> bool {
        let Some(app) = self.app_mut() else { return false };
        if app.net.lan.is_some() {
            return false;
        }
        if app.paused != on {
            app.toggle_pause();
        }
        app.paused == on
    }
    fn season(&self) -> Option<(Option<String>, bool)> {
        self.app()?;
        Some((omsi_texture::season_folder(), crate::scene::SNOW_WEATHER.load(std::sync::atomic::Ordering::Relaxed)))
    }
    fn weather(&self) -> Option<op::Weather> {
        pworld::weather(self.app()?)
    }
    fn set_weather(&mut self, w: &op::WeatherChange) -> Result<(), String> {
        pworld::set_weather(self.app_mut().ok_or("no game")?, w)
    }
    fn weather_presets(&self) -> Vec<(String, String)> {
        pworld::presets()
    }
    fn set_weather_preset(&mut self, file: Option<&str>, seconds: f32) -> Result<(), String> {
        pworld::set_preset(self.app_mut().ok_or("no game")?, file, seconds)
    }

    // --- camera and input ---
    fn camera(&self) -> Option<op::Camera> {
        pworld::camera(self.app()?)
    }
    fn set_view(&mut self, name: &str) -> bool {
        self.app_mut().is_some_and(|a| pworld::set_view(a, name))
    }
    fn set_free_camera(&mut self, at: [f64; 3], yaw: f32, pitch: f32) -> bool {
        self.app_mut().is_some_and(|a| pworld::set_free_camera(a, at, yaw, pitch))
    }
    fn set_zoom(&mut self, k: f32) -> bool {
        let Some(app) = self.app_mut() else { return false };
        let view = app.view.clone();
        app.cam.view_zoom.insert(view, k);
        true
    }
    fn set_look(&mut self, yaw: f32, pitch: f32) -> bool {
        let Some(app) = self.app_mut() else { return false };
        app.cam.look = (yaw, pitch);
        true
    }
    fn key_held(&self, name: &str) -> bool {
        self.app().is_some_and(|a| pworld::key_held(a, name))
    }
    fn keys_held(&self) -> Vec<String> {
        self.app().map(pworld::keys_held).unwrap_or_default()
    }
    fn mouse(&self) -> Option<op::Mouse> {
        let i = &self.app()?.input;
        Some(op::Mouse { x: i.cursor.0, y: i.cursor.1, left: i.buttons_held.0, right: i.buttons_held.1, middle: i.mmb_held })
    }
    fn controllers(&self) -> Vec<op::Controller> {
        self.app().map(pworld::controllers).unwrap_or_default()
    }
    fn bindings(&self, vehicle: bool) -> Vec<(String, String)> {
        self.app().map(|a| pworld::bindings(a, vehicle)).unwrap_or_default()
    }

    // --- sound ---
    fn sound_play(&mut self, file: &std::path::Path, s: &op::Sound) -> Option<u64> {
        pworld::sound_play(self.app.as_deref_mut()?, file, s)
    }
    fn sound_set(&mut self, id: u64, s: &op::Sound) -> bool {
        self.app.as_deref_mut().is_some_and(|a| pworld::sound_set(a, id, s))
    }
    fn sound_stop(&mut self, id: u64) {
        if let Some(a) = self.app().and_then(|a| a.sound.audio.as_ref()) {
            a.stop(id);
        }
    }
    fn sound_playing(&self, id: u64) -> bool {
        self.app().and_then(|a| a.sound.audio.as_ref()).is_some_and(|a| a.is_playing(id))
    }
    fn volume(&self) -> Option<f32> {
        self.app().map(|a| a.settings.volume)
    }

    // --- the game ---
    fn settings(&self) -> Vec<(&'static str, InfoValue)> {
        self.app().map(pworld::settings).unwrap_or_default()
    }
    fn stats(&self) -> Vec<(&'static str, InfoValue)> {
        self.app().map(pworld::stats).unwrap_or_default()
    }
    fn screenshot(&mut self) -> Option<String> {
        let app = self.app.as_deref_mut()?;
        app.take_screenshot();
        app.perf.shot.as_ref().map(|s| s.0.to_string_lossy().into_owned())
    }
    fn notify(&mut self, text: &str, kind: u8, seconds: f32) -> bool {
        self.app.as_deref_mut().is_some_and(|a| pworld::notify(a, text, kind, seconds))
    }
    fn game_action(&mut self, name: &str) -> bool {
        self.app_mut().is_some_and(|a| a.game_action(name))
    }
    fn fps(&self) -> Option<f32> {
        self.app().map(|a| a.perf.fps)
    }
    fn menu_open(&self) -> bool {
        self.app().is_some_and(|a| a.menus.game_menu.is_some())
    }

    // --- LAN ---
    fn lan(&self) -> Option<(bool, u32, String)> {
        pworld::lan(self.app()?)
    }
    fn lan_players(&self) -> Vec<op::LanPlayer> {
        self.app().map(pworld::lan_players).unwrap_or_default()
    }
    fn lan_send(&mut self, plugin: &str, to: u32, text: &str) -> Result<(), String> {
        pworld::lan_send(self.app.as_deref_mut().ok_or("no game")?, plugin, to, text)
    }
    fn lan_chat(&mut self, text: &str) -> Result<(), String> {
        pworld::lan_chat(self.app.as_deref_mut().ok_or("no game")?, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(info: Vec<(&'static str, InfoValue)>) -> Io<'static> {
        Io::snapshot(info)
    }

    /// A ticket's price or a jolt's acceleration reaches Lua as the number it reads as.
    #[test]
    fn f32_values_reach_lua_as_they_read() {
        assert_eq!(num_f32(2.1), InfoValue::Num(2.1));
        assert_eq!(num_f32(-5.4), InfoValue::Num(-5.4));
        assert_eq!(num_f32(38.0), InfoValue::Num(38.0));
    }

    #[test]
    fn game_info_names_require_prefix_and_ignore_ascii_case() {
        let io = snapshot(vec![("heading", InfoValue::Num(93.0))]);
        assert_eq!(io.game_number("OPENOMSI_Heading"), Some(93.0));
        for name in ["heading", "openomsi", "openomsi_", "openomsi_unknown", "openomsí_heading", "💡💡💡heading"] {
            assert_eq!(io.game_value(name), None, "{name}");
        }
    }

    #[test]
    fn game_info_callbacks_keep_number_boolean_and_text_types() {
        let io = snapshot(vec![
            ("speed", InfoValue::Num(42.5)),
            ("paused", InfoValue::Bool(true)),
            ("on_foot", InfoValue::Bool(false)),
            ("destination", InfoValue::Text("Žďár nad Sázavou".into())),
            ("map_path", InfoValue::Text(String::new())),
        ]);
        assert_eq!(io.game_number("openomsi_speed"), Some(42.5));
        assert_eq!(io.game_number("openomsi_paused"), Some(1.0));
        assert_eq!(io.game_number("openomsi_on_foot"), Some(0.0));
        assert_eq!(io.game_string("openomsi_destination").as_deref(), Some("Žďár nad Sázavou"));
        assert_eq!(io.game_string("openomsi_map_path"), Some(String::new()));
        assert_eq!(io.game_number("openomsi_destination"), None);
        assert_eq!(io.game_string("openomsi_speed"), None);
        assert_eq!(io.game_string("openomsi_paused"), None);
    }

    #[test]
    fn game_info_numbers_reject_non_finite_values_and_float_overflow() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
            let io = snapshot(vec![("speed", InfoValue::Num(value))]);
            assert_eq!(io.game_number("openomsi_speed"), None);
        }
    }

    #[test]
    fn legacy_vehicle_callbacks_still_require_a_player_vehicle() {
        let mut io = snapshot(vec![("clock", InfoValue::Num(32400.0)), ("map_path", InfoValue::Text("maps/example/global.cfg".into()))]);
        assert_eq!(io.var("openomsi_clock"), None);
        assert_eq!(io.string("openomsi_map_path"), None);
    }

    /// The next stop and the one before it, by object ID, with their distances: the stop
    /// before skips a passing station, and a stop with no known place has no distance.
    #[test]
    fn info_names_the_stops_on_either_side_of_the_bus() {
        let stop = |id: i64, name: &str, x: f64, stops: bool| crate::schedule::PlannedStop {
            object_id: id,
            name: name.into(),
            arr: 0.0,
            dep: 0.0,
            position: (x >= 0.0).then_some(glam::DVec3::new(x, 0.0, 5.0)),
            dir: Default::default(),
            stops,
        };
        let stops = vec![stop(11, "A", 0.0, true), stop(12, "B", 100.0, false), stop(13, "C", -1.0, true)];
        let mut v = Vec::new();
        stop_info(&mut v, &stops, 2, true, Some(glam::DVec3::new(30.0, 40.0, 0.0)));
        let get = |k: &str| v.iter().find(|(key, _)| *key == k).map(|(_, x)| x.clone());
        assert_eq!(get("next_stop"), Some(InfoValue::Text("C".into())));
        assert_eq!(get("next_stop_id"), Some(InfoValue::Num(13.0)));
        assert_eq!(get("at_stop"), Some(InfoValue::Bool(true)));
        assert_eq!(get("next_stop_distance"), None);
        assert_eq!(get("previous_stop"), Some(InfoValue::Text("A".into())));
        assert_eq!(get("previous_stop_id"), Some(InfoValue::Num(11.0)));
        assert_eq!(get("previous_stop_distance"), Some(InfoValue::Num(50.0)));
        // at the first stop there is none before it
        let mut v = Vec::new();
        stop_info(&mut v, &stops, 0, false, None);
        assert!(v.iter().all(|(k, _)| !k.starts_with("previous_stop") && *k != "next_stop_distance"));
    }
}
