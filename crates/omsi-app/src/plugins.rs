//! The OMSI plugins of the content roots' `plugins` folders (see `omsi_plugin`), driven
//! every frame with the player's bus as OMSI drives them: system
//! variables, then the bus's variables, string variables and triggers.

use omsi_plugin::{HostConfig, InfoValue, PluginIo, Plugins};
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
    if let Some(t) = app.traffic.as_ref() {
        v.push(("traffic", Num(t.cars.len() as f64)));
    }
    if let Some(p) = app.player.as_ref() {
        v.push(("speed", Num(p.vehicle.physics.velocity_kmh().abs() as f64)));
        v.push(("delay", Num(p.vehicle.host.tt_delay as f64)));
    }
    if let Some(d) = app.duty.as_ref() {
        v.push(("line", Text(d.line.trim().to_string())));
        v.push(("tour", Text(d.tour.trim().to_string())));
        if let Some(trip) = d.trips.get(d.trip_index) {
            v.push(("trip", Num(d.trip_index as f64 + 1.0)));
            v.push(("trips", Num(d.trips.len() as f64)));
            v.push(("terminus", Text(trip.terminus.trim().to_string())));
            if let Some(s) = trip.stops.get(d.next_stop) {
                v.push(("next_stop", Text(s.name.trim().to_string())));
                v.push(("next_stop_arrival", Num(s.arr)));
                v.push(("next_stop_departure", Num(s.dep)));
            }
        }
    }
    v
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
        self.vehicle.as_ref()?.var(name)
    }

    fn set_var(&mut self, name: &str, v: f32) {
        if let Some(veh) = self.vehicle.as_mut() {
            veh.set_var(name, v);
        }
    }

    fn string(&mut self, name: &str) -> Option<String> {
        let veh = self.vehicle.as_ref()?;
        let i = veh.ty.program.str_var(name)?;
        veh.state.str_vars.get(i as usize).cloned()
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
