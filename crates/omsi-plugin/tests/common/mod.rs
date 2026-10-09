//! A fake game for the plugin API's tests: every part of `PluginIo` with a little state, and
//! what the plugins changed recorded.
#![allow(dead_code)]

use omsi_plugin::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Game {
    pub vehicle: bool,
    pub vars: HashMap<String, f32>,
    pub strings: HashMap<String, String>,
    pub fired: Vec<(String, bool)>,
    pub messages: Vec<String>,
    pub events: Vec<GameEvent>,
    pub events_ex: Vec<(&'static str, Vec<api::Value>)>,
    pub info: Vec<(&'static str, InfoValue)>,
    pub keys: Vec<(String, bool)>,
    pub held: Vec<String>,
    pub pos: [f64; 4],
    pub doors: Vec<f32>,
    pub indicator: u8,
    pub engine: bool,
    pub gear: f32,
    pub passengers: usize,
    pub duty: Option<Duty>,
    pub clock: Clock,
    pub weather: Weather,
    pub light: Option<Light>,
    pub cars: Vec<AiCar>,
    pub paused: bool,
    pub view: String,
    /// What the plugins asked the game to do, in order.
    pub log: RefCell<Vec<String>>,
    pub sounds: RefCell<Vec<(PathBuf, Sound)>>,
    pub lan_sent: Vec<(String, u32, String)>,
}

impl Game {
    /// A bus on a duty at noon, on a map with a little traffic.
    pub fn new() -> Game {
        let stop = |id: i64, name: &str, t: f64, x: f64| Stop { id, name: name.into(), arr: t, dep: t + 20.0, pos: Some([x, 0.0, 0.0]), stops: true };
        let trip = Trip { name: "Spandau".into(), line: "136".into(), terminus: "Rathaus".into(), departure: 43200.0, end: 44400.0, stops: vec![stop(11, "Zoo", 43200.0, 0.0), stop(12, "Markt", 43500.0, 300.0), stop(13, "Rathaus", 44400.0, 900.0)] };
        let mut g = Game {
            vehicle: true,
            pos: [100.0, 50.0, 2.0, 90.0],
            doors: vec![0.0, 0.0],
            gear: 1.0,
            passengers: 3,
            view: "driver".into(),
            clock: Clock { time: 43200.0, year: 2026, month: 10, day: 8, day_of_year: 281, weekday: 3, run_time: 10.0 },
            weather: Weather { name: "Sunny".into(), visibility_m: 20000.0, temperature: 18.0, clouds: "-1".into(), ..Default::default() },
            duty: Some(Duty { line: "136".into(), tour: "3".into(), trip: 0, trips: 4, next: 1, at_stop: false, trip_done: false, delay: 30.0, current: trip }),
            cars: vec![AiCar { id: 7, kind: "car", name: "VW Golf".into(), pos: [130.0, 50.0, 2.0, 90.0], speed_kmh: 30.0, ..Default::default() }, AiCar { id: 9, kind: "timetable_bus", name: "MAN NL".into(), pos: [600.0, 50.0, 2.0, 0.0], line: Some("137".into()), ..Default::default() }],
            ..Default::default()
        };
        for (k, v) in [("Velocity", 0.0), ("door_0", 0.0), ("cockpit_hupe", 0.0), ("bremse_feststell", 1.0), ("engine_tank_content", 180.0)] {
            g.vars.insert(k.into(), v);
        }
        g.strings.insert("IBIS_terminus_name".into(), "Rathaus".into());
        g
    }

    pub fn did(&self, what: impl Into<String>) {
        self.log.borrow_mut().push(what.into());
    }
}

impl PluginIo for Game {
    fn system(&mut self, name: &str) -> Option<f32> {
        match name {
            "Time" => Some(self.clock.time as f32),
            "SunAlt" => Some(35.0),
            _ => None,
        }
    }
    fn set_system(&mut self, _: &str, _: f32) {}
    fn has_vehicle(&self) -> bool {
        self.vehicle
    }
    fn var(&mut self, name: &str) -> Option<f32> {
        self.vars.get(name).copied()
    }
    fn set_var(&mut self, name: &str, v: f32) {
        self.vars.insert(name.into(), v);
    }
    fn string(&mut self, name: &str) -> Option<String> {
        self.strings.get(name).cloned()
    }
    fn set_string(&mut self, name: &str, s: &str) {
        self.strings.insert(name.into(), s.into());
    }
    fn fire(&mut self, t: &str, down: bool) {
        self.fired.push((t.into(), down));
    }
    fn dt(&self) -> f32 {
        0.5
    }
    fn vehicle_name(&self) -> Option<String> {
        Some("MAN SD202".into())
    }
    fn vehicle_manufacturer_model(&self) -> Option<(String, String)> {
        Some(("MAN".into(), "SD202".into()))
    }
    fn position(&self) -> Option<[f64; 4]> {
        self.vehicle.then_some(self.pos)
    }
    fn message(&mut self, text: &str, _: f32) {
        self.messages.push(text.into());
    }
    fn info(&self) -> Vec<(&'static str, InfoValue)> {
        self.info.clone()
    }
    fn command(&mut self, what: &str) -> bool {
        self.did(format!("command {what}"));
        what == "refuel"
    }
    fn var_names(&self) -> (Vec<String>, Vec<String>) {
        let mut v: Vec<String> = self.vars.keys().cloned().collect();
        v.sort();
        (v, self.strings.keys().cloned().collect())
    }
    fn keys(&self) -> Vec<(String, bool)> {
        self.keys.clone()
    }
    fn events(&self) -> Vec<GameEvent> {
        self.events.clone()
    }
    fn events_ex(&self) -> Vec<(&'static str, Vec<api::Value>)> {
        self.events_ex.clone()
    }
    fn bus_velocity(&self) -> Option<[f64; 3]> {
        self.vehicle.then_some([1.0, 2.0, 0.0])
    }
    fn bus_acceleration(&self) -> Option<[f64; 3]> {
        self.vehicle.then_some([0.1, -1.5, 0.0])
    }
    fn bus_orientation(&self) -> Option<[f64; 3]> {
        self.vehicle.then_some([self.pos[3], 1.0, -0.5])
    }
    fn bus_mass(&self) -> Option<f64> {
        Some(11500.0)
    }
    fn bus_km(&self) -> Option<(f64, f64)> {
        Some((123456.7, 12.5))
    }
    fn bus_doors(&self) -> Vec<f32> {
        self.doors.clone()
    }
    fn bus_doors_open(&self) -> Option<bool> {
        Some(self.doors.iter().any(|d| *d > 0.05))
    }
    fn bus_door_count(&self) -> usize {
        self.doors.len()
    }
    fn bus_door_key(&mut self, n: usize) -> bool {
        self.did(format!("door {n}"));
        true
    }
    fn bus_indicator(&self) -> Option<u8> {
        Some(self.indicator)
    }
    fn bus_set_indicator(&mut self, want: u8) -> bool {
        self.indicator = want;
        true
    }
    fn bus_headlights(&self) -> Option<u8> {
        Some(2)
    }
    fn bus_interior_light(&self) -> Option<f32> {
        Some(1.0)
    }
    fn bus_set_interior_light(&mut self, on: bool) -> bool {
        self.did(format!("interior {on}"));
        true
    }
    fn bus_engine(&self) -> Option<(bool, Option<f32>, bool)> {
        Some((self.engine, Some(if self.engine { 750.5 } else { 0.0 }), true))
    }
    fn bus_start_up(&mut self) -> Option<String> {
        self.engine = true;
        Some("Starting up".into())
    }
    fn bus_gear(&self) -> Option<f32> {
        Some(self.gear)
    }
    fn bus_shift(&mut self, gear: i32) -> bool {
        self.gear = gear as f32;
        true
    }
    fn bus_dirt(&self) -> Option<f32> {
        Some(0.25)
    }
    fn bus_damage(&mut self) -> Option<(u32, f32, Option<f32>)> {
        Some((1, 136000.0, Some(30.0)))
    }
    fn bus_controls(&self) -> Option<[f32; 4]> {
        Some([0.5, 0.0, 0.0, -0.25])
    }
    fn bus_steer(&self) -> Option<(f32, f32)> {
        Some((-10.0, 45.0))
    }
    fn bus_action(&mut self, name: &str, down: bool) -> bool {
        self.did(format!("action {name} {down}"));
        if name == "horn" {
            self.vars.insert("cockpit_hupe".into(), down as u8 as f32);
        }
        true
    }
    fn bus_trailers(&self) -> Option<usize> {
        Some(0)
    }
    fn bus_destinations(&self) -> (Vec<Terminus>, Option<i64>) {
        (vec![Terminus { code: 1, name: "Rathaus".into(), all_exit: false }, Terminus { code: 0, name: "Betriebsfahrt".into(), all_exit: true }], Some(0))
    }
    fn bus_set_destination(&mut self, index: usize) -> bool {
        self.did(format!("destination {index}"));
        index < 2
    }
    fn bus_set_line(&mut self, line: &str) -> bool {
        self.did(format!("line {line}"));
        true
    }
    fn bus_ident(&self) -> Option<(String, String)> {
        Some(("2711".into(), "Vehicles/MAN_SD202/SD202.bus".into()))
    }
    fn bus_passengers(&self) -> Option<(usize, usize, usize)> {
        Some((self.passengers, self.passengers.min(2), self.passengers.saturating_sub(2)))
    }
    fn bus_tickets(&self) -> (Vec<Ticket>, Option<(String, f32)>) {
        (vec![Ticket { name: "Einzelfahrschein".into(), price: 2.1, day_ticket: false }], Some(("Einzelfahrschein".into(), 2.1)))
    }
    fn bus_sales(&self) -> Option<(u32, f64)> {
        Some((4, 8.4))
    }
    fn bus_wheels(&self) -> Vec<Wheel> {
        vec![Wheel { axle: 0, side: 0, rpm: 10.0, radius: 0.5, suspension: 0.01, driven: false }]
    }
    fn bus_triggers(&self) -> Vec<String> {
        vec!["bus_doorfront0".into()]
    }
    fn bus_sound(&mut self, event: &str) -> bool {
        self.did(format!("sound {event}"));
        true
    }
    fn traffic_list(&self) -> Vec<AiCar> {
        self.cars.clone()
    }
    fn traffic_counts(&self) -> Option<[usize; 4]> {
        Some([2, 1, 5, 10])
    }
    fn traffic_density(&self) -> Option<(usize, f32)> {
        Some((30, 1.0))
    }
    fn traffic_set_density(&mut self, cars: Option<usize>, share: Option<f32>) -> bool {
        self.did(format!("density {cars:?} {share:?}"));
        true
    }
    fn traffic_remove(&mut self, id: u64) -> bool {
        let n = self.cars.len();
        self.cars.retain(|c| c.id != id);
        n != self.cars.len()
    }
    fn traffic_clear(&mut self) -> Option<usize> {
        Some(1)
    }
    fn traffic_buses(&self) -> Vec<AiBus> {
        vec![AiBus { id: 9, line: "137".into(), tour: "1".into(), trip: "x".into(), delay: 60.0, ..Default::default() }]
    }
    fn traffic_light_ahead(&self, _: f64) -> Option<Light> {
        self.light.clone()
    }
    fn people_counts(&self) -> Option<[usize; 3]> {
        Some([20, 5, 3])
    }
    fn people_list(&self) -> Vec<Person> {
        vec![Person { id: 1, pos: [101.0, 50.0, 2.0], state: "seated", aboard: true, ..Default::default() }, Person { id: 2, pos: [900.0, 0.0, 0.0], state: "waiting", stop: Some(13), ..Default::default() }]
    }
    fn people_stops(&self) -> Vec<PaxStop> {
        vec![PaxStop { id: 13, name: "Rathaus".into(), pos: [900.0, 0.0, 0.0], waiting: 5 }]
    }
    fn people_density(&self) -> Option<f32> {
        Some(1.0)
    }
    fn people_set_density(&mut self, v: f32) -> bool {
        self.did(format!("people {v}"));
        true
    }
    fn duty(&self) -> Option<Duty> {
        self.duty.clone()
    }
    fn duty_trips(&self) -> Vec<Trip> {
        self.duty.iter().map(|d| d.current.clone()).collect()
    }
    fn duty_skip_next(&mut self) -> Option<String> {
        let d = self.duty.as_mut()?;
        let name = d.current.stops.get(d.next)?.name.clone();
        d.next += 1;
        Some(name)
    }
    fn duty_skip_to(&mut self, index: usize) -> bool {
        match self.duty.as_mut() {
            Some(d) if index < d.current.stops.len() => {
                d.next = index;
                true
            }
            _ => false,
        }
    }
    fn duty_start(&mut self, line: &str, tour: &str, trip: usize, stop: usize) -> Result<(), String> {
        if line != "136" {
            return Err(format!("no line {line}"));
        }
        self.did(format!("duty {line}/{tour} {trip} {stop}"));
        Ok(())
    }
    fn duty_end(&mut self) -> bool {
        self.duty.take().is_some()
    }
    fn timetable_lines(&self) -> Vec<Line> {
        vec![Line { name: "136".into(), user_allowed: true, tours: vec![Tour { number: "3".into(), today: true, trips: 4 }] }]
    }
    fn timetable_stops(&self, _: &str, _: &str, trip: Option<usize>) -> Vec<(usize, usize, String, f64)> {
        let t = trip.unwrap_or(0);
        vec![(t, 0, "Zoo".into(), 43200.0), (t, 1, "Markt".into(), 43500.0)]
    }
    fn timetable_stop_names(&self) -> Vec<(i64, String)> {
        vec![(11, "Zoo".into())]
    }
    fn map_info(&self) -> Option<MapInfo> {
        Some(MapInfo { name: "Spandau".into(), friendly_name: "Berlin-Spandau".into(), path: "maps/Berlin-Spandau/global.cfg".into(), left_hand_traffic: false, tile_size: 300.0 })
    }
    fn map_tiles(&self) -> Vec<Tile> {
        vec![Tile { x: 0, y: 0, file: "tile_0_0.map".into(), loaded: true }]
    }
    fn map_stops(&self) -> Vec<MapStop> {
        vec![MapStop { id: 11, name: "Zoo".into(), pos: [0.0, 0.0, 0.0, 90.0] }]
    }
    fn map_object(&self, id: i64) -> Option<[f64; 4]> {
        (id == 11).then_some([0.0, 0.0, 0.0, 90.0])
    }
    fn map_objects_near(&self, _: f64, _: f64, _: f64) -> Vec<(i64, [f64; 4])> {
        vec![(11, [0.0, 0.0, 0.0, 90.0]), (12, [10.0, 0.0, 0.0, 0.0])]
    }
    fn map_ground(&self, _: f64, _: f64) -> (Option<f64>, Option<f64>) {
        (Some(2.5), Some(2.0))
    }
    fn map_entrypoints(&self) -> Vec<Entry> {
        vec![Entry { index: 0, name: "Depot".into(), pos: Some([5.0, 5.0, 0.0, 180.0]) }]
    }
    fn map_teleport(&mut self, at: [f64; 3], heading: f64) -> bool {
        self.pos = [at[0], at[1], at[2], heading];
        true
    }
    fn map_teleport_entry(&mut self, index: usize) -> bool {
        index == 0
    }
    fn map_place_on_road(&mut self, _: f64, _: f64) -> bool {
        true
    }
    fn map_lane(&self, _: f64, _: f64) -> Option<Lane> {
        Some(Lane { index: 4, distance: 1.0, speed_limit_kmh: 50.0, name: "Street".into(), has_light: false })
    }
    fn clock(&self) -> Option<Clock> {
        Some(self.clock.clone())
    }
    fn set_time(&mut self, s: f64) -> bool {
        self.clock.time = s;
        true
    }
    fn set_date(&mut self, y: i32, m: u32, d: u32) -> bool {
        (self.clock.year, self.clock.month, self.clock.day) = (y, m, d);
        true
    }
    fn time_speed(&self) -> Option<f64> {
        Some(1.0)
    }
    fn set_time_speed(&mut self, x: f64) -> bool {
        self.did(format!("time speed {x}"));
        true
    }
    fn paused(&self) -> bool {
        self.paused
    }
    fn set_paused(&mut self, on: bool) -> bool {
        self.paused = on;
        true
    }
    fn season(&self) -> Option<(Option<String>, bool)> {
        Some((None, false))
    }
    fn weather(&self) -> Option<Weather> {
        Some(self.weather.clone())
    }
    fn set_weather(&mut self, w: &WeatherChange) -> Result<(), String> {
        if let Some(t) = w.temperature {
            self.weather.temperature = t;
        }
        if let Some(k) = w.precip_kind {
            self.weather.precip_kind = k;
        }
        Ok(())
    }
    fn weather_presets(&self) -> Vec<(String, String)> {
        vec![("Weather/Rain.owt".into(), "Rain".into())]
    }
    fn set_weather_preset(&mut self, file: Option<&str>, _: f32) -> Result<(), String> {
        self.weather.name = file.unwrap_or("map").into();
        Ok(())
    }
    fn camera(&self) -> Option<Camera> {
        Some(Camera { view: self.view.clone(), pos: [0.0, -10.0, 2.0], yaw: 0.0, pitch: 0.0, fov: 60.0, zoom: 1.0, width: 1920, height: 1080, ..Default::default() })
    }
    fn set_view(&mut self, name: &str) -> bool {
        self.view = name.into();
        true
    }
    fn set_free_camera(&mut self, _: [f64; 3], _: f32, _: f32) -> bool {
        self.view = "free".into();
        true
    }
    fn set_zoom(&mut self, _: f32) -> bool {
        true
    }
    fn set_look(&mut self, _: f32, _: f32) -> bool {
        true
    }
    fn key_held(&self, name: &str) -> bool {
        self.held.iter().any(|k| k == name)
    }
    fn keys_held(&self) -> Vec<String> {
        self.held.clone()
    }
    fn mouse(&self) -> Option<Mouse> {
        Some(Mouse { x: 200.0, y: 100.0, left: true, ..Default::default() })
    }
    fn controllers(&self) -> Vec<Controller> {
        vec![Controller { name: "Wheel".into(), gamepad: false, axes: vec![0.0, 1.0], buttons: 12 }]
    }
    fn bindings(&self, vehicle: bool) -> Vec<(String, String)> {
        vec![(if vehicle { "horn" } else { "sim_pause" }.into(), "P".into())]
    }
    fn sound_play(&mut self, file: &Path, s: &Sound) -> Option<u64> {
        self.sounds.borrow_mut().push((file.to_path_buf(), s.clone()));
        Some(self.sounds.borrow().len() as u64)
    }
    fn sound_set(&mut self, _: u64, _: &Sound) -> bool {
        true
    }
    fn sound_stop(&mut self, id: u64) {
        self.did(format!("stop sound {id}"));
    }
    fn sound_playing(&self, _: u64) -> bool {
        true
    }
    fn volume(&self) -> Option<f32> {
        Some(0.8)
    }
    fn settings(&self) -> Vec<(&'static str, InfoValue)> {
        vec![("graphics", InfoValue::Text("enhanced".into())), ("units", InfoValue::Text("metric".into()))]
    }
    fn stats(&self) -> Vec<(&'static str, InfoValue)> {
        vec![("km", InfoValue::Num(12.5))]
    }
    fn screenshot(&mut self) -> Option<String> {
        Some("Screenshots/omsi_1.png".into())
    }
    fn notify(&mut self, text: &str, kind: u8, _: f32) -> bool {
        self.did(format!("notify {kind} {text}"));
        true
    }
    fn game_action(&mut self, name: &str) -> bool {
        self.did(format!("game action {name}"));
        true
    }
    fn fps(&self) -> Option<f32> {
        Some(60.0)
    }
    fn lan(&self) -> Option<(bool, u32, String)> {
        Some((true, 1, "Host".into()))
    }
    fn lan_players(&self) -> Vec<LanPlayer> {
        vec![LanPlayer { id: 2, name: "Anna".into(), bus: "x.bus".into(), ..Default::default() }]
    }
    fn lan_send(&mut self, plugin: &str, to: u32, text: &str) -> Result<(), String> {
        self.lan_sent.push((plugin.into(), to, text.into()));
        Ok(())
    }
    fn lan_chat(&mut self, text: &str) -> Result<(), String> {
        self.did(format!("chat {text}"));
        Ok(())
    }
}

/// A fresh plugins folder for one test.
pub fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("omsi-api-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Load one plugin of this source and run `frames` frames of `game` with it.
pub fn run(tag: &str, src: &str, game: &mut Game, frames: usize) -> Plugins {
    let d = dir(tag);
    std::fs::write(d.join(format!("{tag}.lua")), src).unwrap();
    let mut p = Plugins::load(&[d], &HostConfig::default());
    assert_eq!(p.lua.len(), 1, "{tag} did not load: {:?}", game.messages);
    for _ in 0..frames {
        p.frame(game);
    }
    p
}
