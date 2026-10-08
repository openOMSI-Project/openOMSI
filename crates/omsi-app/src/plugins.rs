//! The OMSI plugins of the content roots' `plugins` folders (see `omsi_plugin`), driven
//! every frame with the player's bus as OMSI drives them: system
//! variables, then the bus's variables, string variables and triggers.

use omsi_plugin::{GameEvent, HostConfig, InfoValue, PluginIo, Plugins};
use omsi_script::Host;
use omsi_script::SysVar;

/// Load every plugin of every content root (`OMSI_NO_PLUGINS=1` leaves them out).
pub(crate) fn load() -> Plugins {
    if omsi_cfg::env::var_os("OMSI_NO_PLUGINS").is_some() {
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

/// The game's side of a plugin frame: the player's bus, when there is one.
pub(crate) struct Io<'a> {
    pub vehicle: Option<&'a mut omsi_sim::VehicleInstance>,
    /// `omsi.others`: the AI traffic and the other LAN players' buses, by id.
    pub others: Vec<(u64, &'static str, &'a mut omsi_sim::VehicleInstance)>,
    /// Seconds since the last frame.
    pub dt: f32,
    /// A plugin's `omsi.message`, shown when the frame is done.
    pub message: Option<(String, f32)>,
    /// `omsi.info()`: taken before the frame (see [`game_info`]).
    pub info: Vec<(&'static str, InfoValue)>,
    /// `omsi.command`: game menu lines to run after the frame.
    pub commands: Vec<String>,
    /// Keys pressed and let go since the last frame.
    pub keys: Vec<(String, bool)>,
    /// What happened since the last frame (see [`queue_event`]).
    pub events: Vec<GameEvent>,
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
    v.push(("on_foot", Bool(app.on_foot.is_some())));
    v.push(("multiplayer", Bool(app.lan.is_some())));
    // the situation the game started from (the launcher's "continue": `laststn.osn`)
    // (relative to the OMSI folder, `/`-separated, whether the launcher passed it absolute or not)
    if let Some(s) = app.args.situation.as_ref() {
        let p = std::path::Path::new(s);
        let rel = p.strip_prefix(&app.args.root).unwrap_or(p);
        v.push(("situation", Text(rel.to_string_lossy().replace('\\', "/"))));
    }
    // this session's, as the personnel file counts them
    v.push(("crashes", Num(app.career.crashes[0] as f64)));
    v.push(("heavy_crashes", Num(app.career.crashes[3] as f64)));
    v.push(("pedestrians_hit", Num(app.career.crashes[1] as f64)));
    if let Some(t) = app.traffic.as_ref() {
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
        v.push(("passengers", Num(app.humans.as_ref().map(|h| h.riding()).unwrap_or(0) as f64)));
    }
    if let Some(d) = app.duty.as_ref() {
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
            if let Some(s) = trip.stops.get(d.next_stop) {
                v.push(("next_stop", Text(s.name.trim().to_string())));
                v.push(("next_stop_number", Num(d.next_stop as f64 + 1.0)));
                v.push(("next_stop_arrival", Num(s.arr)));
                v.push(("next_stop_departure", Num(s.dep)));
            }
        }
    }
    v
}

impl Io<'_> {
    /// The `omsi.info()` key of an `.opl` list's `openomsi_<key>` name (any case).
    fn game_key(name: &str) -> Option<&str> {
        name.get(..9).filter(|p| p.eq_ignore_ascii_case("openomsi_")).map(|_| &name[9..])
    }

    /// The value of `omsi.info()` an `.opl` list names as `openomsi_<key>` (any case).
    fn game_value(&self, name: &str) -> Option<&InfoValue> {
        let key = Self::game_key(name)?;
        self.info.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
    }

    fn game_number(&self, name: &str) -> Option<f32> {
        let value = match self.game_value(name)? {
            InfoValue::Num(n) => *n as f32,
            InfoValue::Bool(b) => u8::from(*b) as f32,
            InfoValue::Text(_) => return None,
        };
        value.is_finite().then_some(value)
    }

    fn game_string(&self, name: &str) -> Option<String> {
        match self.game_value(name)? {
            InfoValue::Text(t) => Some(t.clone()),
            _ => None,
        }
    }
}

impl PluginIo for Io<'_> {
    fn system(&mut self, name: &str) -> Option<f32> {
        let v = SysVar::from_name(name)?;
        self.vehicle.as_mut().map(|veh| veh.host.sys_var(v))
    }

    fn set_system(&mut self, name: &str, v: f32) {
        // the clock, the weather and the input are the game's own; a plugin writing them
        // is told nothing, as the scripts' S.S. writes are not honoured either
        log::debug!("plugin wrote system variable {name} = {v} (kept as it is)");
    }

    fn has_vehicle(&self) -> bool {
        self.vehicle.is_some()
    }

    fn var(&mut self, name: &str) -> Option<f32> {
        if let Some(v) = self.vehicle.as_ref()?.var(name) {
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
        if let Some(veh) = self.vehicle.as_mut() {
            veh.set_var(name, v);
        }
    }

    fn string(&mut self, name: &str) -> Option<String> {
        let veh = self.vehicle.as_ref()?;
        if let Some(i) = veh.ty.program.str_var(name) {
            return veh.state.str_vars.get(i as usize).cloned();
        }
        self.game_string(name)
    }

    fn set_string(&mut self, name: &str, s: &str) {
        if let Some(veh) = self.vehicle.as_mut() {
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
        if let Some(veh) = self.vehicle.as_mut() {
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
        self.vehicle.as_ref().map(|v| format!("{} {}", v.ty.def.manufacturer, v.ty.def.type_name).trim().to_string())
    }

    fn vehicle_manufacturer_model(&self) -> Option<(String, String)> {
        self.vehicle.as_ref().map(|v| (v.ty.def.manufacturer.trim().to_string(), v.ty.def.type_name.trim().to_string()))
    }

    fn position(&self) -> Option<[f64; 4]> {
        self.vehicle.as_ref().map(|v| [v.position.x, v.position.y, v.position.z, v.heading])
    }

    fn message(&mut self, text: &str, seconds: f32) {
        self.message = Some((text.to_string(), seconds));
    }

    fn info(&self) -> Vec<(&'static str, InfoValue)> {
        self.info.clone()
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
        match self.vehicle.as_ref() {
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

    fn others(&self, radius: f64) -> Vec<omsi_plugin::Other> {
        let Some(me) = self.vehicle.as_ref().map(|v| v.position) else {
            return Vec::new();
        };
        self.others
            .iter()
            .filter(|(_, _, v)| (v.position.x - me.x).powi(2) + (v.position.y - me.y).powi(2) <= radius * radius)
            .map(|(id, kind, v)| omsi_plugin::Other {
                id: *id,
                kind,
                name: format!("{} {}", v.ty.def.manufacturer, v.ty.def.type_name).trim().to_string(),
                pos: [v.position.x, v.position.y, v.position.z, v.heading],
            })
            .collect()
    }

    fn other_var(&mut self, id: u64, name: &str) -> Option<f32> {
        self.others.iter().find(|o| o.0 == id)?.2.var(name)
    }

    fn set_other_var(&mut self, id: u64, name: &str, v: f32) -> bool {
        self.others.iter_mut().find(|o| o.0 == id).is_some_and(|o| o.2.set_var(name, v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(info: Vec<(&'static str, InfoValue)>) -> Io<'static> {
        Io { vehicle: None, others: Vec::new(), dt: 0.0, message: None, info, commands: Vec::new(), keys: Vec::new(), events: Vec::new() }
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
}
