//! [`PluginIo`]: the game's side of the plugins. The first eight methods are what an OMSI
//! `.opl` plugin needs and every game implements them; the rest feed the Lua and WASM
//! plugins' API (`crate::api`) and default to "the game has none of it" - a test's fake
//! game implements what it tests, the real game (omsi-app's `plugins.rs`) all of it.
//!
//! Coordinates are the map's: metres, x east, y north, z up; headings in degrees clockwise
//! from north. Speeds in km/h unless a name says otherwise.

use crate::api::Value;
use std::path::Path;

/// The game's side of a frame.
pub trait PluginIo {
    fn system(&mut self, name: &str) -> Option<f32>;
    fn set_system(&mut self, name: &str, v: f32);
    fn has_vehicle(&self) -> bool;
    fn var(&mut self, name: &str) -> Option<f32>;
    fn set_var(&mut self, name: &str, v: f32);
    fn string(&mut self, name: &str) -> Option<String>;
    fn set_string(&mut self, name: &str, s: &str);
    /// A trigger's key went down (`true`) or came up.
    fn fire(&mut self, trigger: &str, down: bool);
    /// Seconds of game time since the last frame (timers).
    fn dt(&self) -> f32 {
        0.0
    }
    /// The player's vehicle's name.
    fn vehicle_name(&self) -> Option<String> {
        None
    }
    /// The player's vehicle's manufacturer and model apart, as its `[friendlyname]` has them
    /// (the name is the two joined).
    fn vehicle_manufacturer_model(&self) -> Option<(String, String)> {
        None
    }
    /// The player's vehicle: x, y, z and heading in degrees.
    fn position(&self) -> Option<[f64; 4]> {
        None
    }
    /// A line of text on the screen for `seconds`.
    fn message(&mut self, _text: &str, _seconds: f32) {}
    /// What the game is doing, as (key, value) pairs for `omsi.info()`: the map, the clock,
    /// the duty, the view... A game works it out once a frame, when first asked.
    fn info(&self) -> Vec<(&'static str, InfoValue)> {
        Vec::new()
    }
    /// One value of [`PluginIo::info`] (a game keeps the frame's table and reads it there).
    fn info_value(&self, key: &str) -> Option<InfoValue> {
        self.info().into_iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }
    /// A game action by its game-menu id (`refuel`, `shot`, ...), run after the frame.
    /// False when the game does not know it.
    fn command(&mut self, _what: &str) -> bool {
        false
    }
    /// The names of the player's bus's script variables and string variables.
    fn var_names(&self) -> (Vec<String>, Vec<String>) {
        (Vec::new(), Vec::new())
    }
    /// Keys pressed (true) and let go since the last frame, by winit's key name.
    fn keys(&self) -> Vec<(String, bool)> {
        Vec::new()
    }
    /// The other vehicles within `radius` m of the player's (`omsi.others`): the AI traffic
    /// and the other LAN players' buses.
    fn others(&self, _radius: f64) -> Vec<Other> {
        Vec::new()
    }
    /// A variable of one of [`PluginIo::others`] by its id.
    fn other_var(&mut self, _id: u64, _name: &str) -> Option<f32> {
        None
    }
    /// Writes a variable of one of [`PluginIo::others`] (an AI vehicle's; another player's
    /// bus takes its values from the network again).
    fn set_other_var(&mut self, _id: u64, _name: &str, _v: f32) -> bool {
        false
    }
    /// What happened in the game since the last plugin frame, each sent to the plugins as an
    /// event (`crash`, `pedestrian`, `stops_skipped`, `service`, `trip_done`, `jolt`,
    /// `ticket_sold`, ...): things a plugin could not see by looking, as they are over
    /// before it could look. Every plugin of the frame gets them all.
    fn events(&self) -> Vec<GameEvent> {
        Vec::new()
    }
    /// The same for events whose values are not plain numbers and texts.
    fn events_ex(&self) -> Vec<(&'static str, Vec<Value>)> {
        Vec::new()
    }

    // --- the player's bus ---------------------------------------------------------------
    /// The bus's velocity in the world, m/s (x east, y north, z up).
    fn bus_velocity(&self) -> Option<[f64; 3]> {
        None
    }
    /// Its acceleration in its own frame, m/s² without gravity: across (right +), along
    /// (forward +), up.
    fn bus_acceleration(&self) -> Option<[f64; 3]> {
        None
    }
    /// Heading, pitch (nose up +) and bank (right side down +), degrees.
    fn bus_orientation(&self) -> Option<[f64; 3]> {
        None
    }
    fn bus_mass(&self) -> Option<f64> {
        None
    }
    /// The bus's odometer and the kilometres driven this session.
    fn bus_km(&self) -> Option<(f64, f64)> {
        None
    }
    /// Each door leaf's position, front to back (0 shut, 1 open), as the scripts' `door_<n>`.
    fn bus_doors(&self) -> Vec<f32> {
        Vec::new()
    }
    /// Whether any door is open, by the passengers' flags where the bus has them.
    fn bus_doors_open(&self) -> Option<bool> {
        None
    }
    /// The doorways the door keys work (front to back).
    fn bus_door_count(&self) -> usize {
        0
    }
    /// Press and let go the key of doorway `n` (1..), or of all doors (0).
    fn bus_door_key(&mut self, _n: usize) -> bool {
        false
    }
    /// 0 off, 1 left, 2 right, 3 hazard lights.
    fn bus_indicator(&self) -> Option<u8> {
        None
    }
    fn bus_set_indicator(&mut self, _want: u8) -> bool {
        false
    }
    /// 0 off, 1 side lights, 2 dipped, 3 high beam.
    fn bus_headlights(&self) -> Option<u8> {
        None
    }
    /// The brightest passenger-room light, 0..1.
    fn bus_interior_light(&self) -> Option<f32> {
        None
    }
    fn bus_set_interior_light(&mut self, _on: bool) -> bool {
        false
    }
    /// The engine: running, its rpm where the bus shows one, and the electrics on.
    fn bus_engine(&self) -> Option<(bool, Option<f32>, bool)> {
        None
    }
    /// The automatic start-up (or shut-down) of the bus; what it says it does.
    fn bus_start_up(&mut self) -> Option<String> {
        None
    }
    /// The gear engaged (-1 reverse, 0 neutral), where the bus has one.
    fn bus_gear(&self) -> Option<f32> {
        None
    }
    /// Put a manual gearbox's lever in `gear` (-1 reverse, 0 neutral).
    fn bus_shift(&mut self, _gear: i32) -> bool {
        false
    }
    /// The dirt on the bus, 0..1.
    fn bus_dirt(&self) -> Option<f32> {
        None
    }
    /// Crashes of the bus, the energy of the last (J) and the minutes a repair would take.
    fn bus_damage(&mut self) -> Option<(u32, f32, Option<f32>)> {
        None
    }
    /// What the bus drives with: throttle, brake, clutch (0..1) and steering (-1..1, right +).
    fn bus_controls(&self) -> Option<[f32; 4]> {
        None
    }
    /// The front wheels' angle and the most they turn, degrees (right +).
    fn bus_steer(&self) -> Option<(f32, f32)> {
        None
    }
    /// A key action of the vehicle (`[vehicles]` of keyboard.cfg: `horn`,
    /// `parking_brake_toggle`, `kw_scheinwerfer_toggle`, ...), down or up.
    fn bus_action(&mut self, _name: &str, _down: bool) -> bool {
        false
    }
    /// Parts coupled behind the bus (an articulated bus's rear counts).
    fn bus_trailers(&self) -> Option<usize> {
        None
    }
    /// The destinations of the bus's depot file and the one shown (an index into them).
    fn bus_destinations(&self) -> (Vec<Terminus>, Option<i64>) {
        (Vec::new(), None)
    }
    fn bus_set_destination(&mut self, _index: usize) -> bool {
        false
    }
    /// Set the line (route) shown, as typed into the IBIS.
    fn bus_set_line(&mut self, _line: &str) -> bool {
        false
    }
    /// The fleet number and the `.bus` file (relative to the game folder).
    fn bus_ident(&self) -> Option<(String, String)> {
        None
    }
    /// Passengers aboard: all, seated, standing.
    fn bus_passengers(&self) -> Option<(usize, usize, usize)> {
        None
    }
    /// The tickets on sale and the one a passenger at the desk asks for.
    fn bus_tickets(&self) -> (Vec<Ticket>, Option<(String, f32)>) {
        (Vec::new(), None)
    }
    /// Tickets sold this session and the money taken.
    fn bus_sales(&self) -> Option<(u32, f64)> {
        None
    }
    fn bus_wheels(&self) -> Vec<Wheel> {
        Vec::new()
    }
    /// The names of the bus's script triggers.
    fn bus_triggers(&self) -> Vec<String> {
        Vec::new()
    }
    /// Play the bus's own sound of this trigger (an event name of its sound files).
    fn bus_sound(&mut self, _event: &str) -> bool {
        false
    }

    // --- traffic and people -------------------------------------------------------------
    fn traffic_list(&self) -> Vec<AiCar> {
        Vec::new()
    }
    /// Cars driving, timetable buses, cars asleep out of range, parked cars.
    fn traffic_counts(&self) -> Option<[usize; 4]> {
        None
    }
    /// How many cars the traffic keeps around the camera, and the share of cars that do not
    /// run to a timetable.
    fn traffic_density(&self) -> Option<(usize, f32)> {
        None
    }
    fn traffic_set_density(&mut self, _cars: Option<usize>, _share: Option<f32>) -> bool {
        false
    }
    fn traffic_remove(&mut self, _id: u64) -> bool {
        false
    }
    /// Remove every car that does not run to a timetable; how many went.
    fn traffic_clear(&mut self) -> Option<usize> {
        None
    }
    /// The timetable buses on the road.
    fn traffic_buses(&self) -> Vec<AiBus> {
        Vec::new()
    }
    /// The traffic light the player's bus comes to within `reach` m.
    fn traffic_light_ahead(&self, _reach: f64) -> Option<Light> {
        None
    }
    /// Walking, waiting at stops, riding a bus.
    fn people_counts(&self) -> Option<[usize; 3]> {
        None
    }
    fn people_list(&self) -> Vec<Person> {
        Vec::new()
    }
    /// The stops people wait at near the camera.
    fn people_stops(&self) -> Vec<PaxStop> {
        Vec::new()
    }
    /// The people setting (0..3, 1 the map's own).
    fn people_density(&self) -> Option<f32> {
        None
    }
    fn people_set_density(&mut self, _v: f32) -> bool {
        false
    }

    // --- duty and timetable -------------------------------------------------------------
    fn duty(&self) -> Option<Duty> {
        None
    }
    /// The trips of the duty, each with its stops.
    fn duty_trips(&self) -> Vec<Trip> {
        Vec::new()
    }
    /// Skip the next stop; its name.
    fn duty_skip_next(&mut self) -> Option<String> {
        None
    }
    /// Make stop `index` (from 0) of the trip the next one.
    fn duty_skip_to(&mut self, _index: usize) -> bool {
        false
    }
    /// Take a duty: a line, a tour, its trip (from 0 in departure order) and the stop to
    /// start at (from 0 among the stops it calls at).
    fn duty_start(&mut self, _line: &str, _tour: &str, _trip: usize, _stop: usize) -> Result<(), String> {
        Err("the game has no timetable".into())
    }
    fn duty_end(&mut self) -> bool {
        false
    }
    fn timetable_lines(&self) -> Vec<Line> {
        Vec::new()
    }
    /// The stops of a tour's trips, or of one trip (from 0 in departure order): (trip,
    /// station, name, departure).
    fn timetable_stops(&self, _line: &str, _tour: &str, _trip: Option<usize>) -> Vec<(usize, usize, String, f64)> {
        Vec::new()
    }
    /// The timetable's stops: (map object id, name).
    fn timetable_stop_names(&self) -> Vec<(i64, String)> {
        Vec::new()
    }

    // --- the map ------------------------------------------------------------------------
    fn map_info(&self) -> Option<MapInfo> {
        None
    }
    fn map_tiles(&self) -> Vec<Tile> {
        Vec::new()
    }
    /// The tile of a map point and the metres in it (the map's own tile size and, on a
    /// `[worldcoordinates]` map, its scale).
    fn map_tile_at(&self, _x: f64, _y: f64) -> Option<((i32, i32), (f64, f64))> {
        None
    }
    /// The map point of a place in a tile.
    fn map_from_tile(&self, _tx: i32, _ty: i32, _lx: f64, _ly: f64) -> Option<(f64, f64)> {
        None
    }
    /// The bus stops of the tiles loaded.
    fn map_stops(&self) -> Vec<MapStop> {
        Vec::new()
    }
    /// A map object's place by its id: x, y, z, heading.
    fn map_object(&self, _id: i64) -> Option<[f64; 4]> {
        None
    }
    /// The objects within `r` m of x, y: (id, [x, y, z, heading]).
    fn map_objects_near(&self, _x: f64, _y: f64, _r: f64) -> Vec<(i64, [f64; 4])> {
        Vec::new()
    }
    /// The ground at x, y: the road or the terrain, and the terrain alone (loaded tiles).
    fn map_ground(&self, _x: f64, _y: f64) -> (Option<f64>, Option<f64>) {
        (None, None)
    }
    fn map_entrypoints(&self) -> Vec<Entry> {
        Vec::new()
    }
    /// Move the player's bus (z 0: onto the highest ground there).
    fn map_teleport(&mut self, _at: [f64; 3], _heading: f64) -> bool {
        false
    }
    fn map_teleport_entry(&mut self, _index: usize) -> bool {
        false
    }
    /// Put the bus on the street nearest to x, y, along it.
    fn map_place_on_road(&mut self, _x: f64, _y: f64) -> bool {
        false
    }
    /// The traffic lane nearest to x, y.
    fn map_lane(&self, _x: f64, _y: f64) -> Option<Lane> {
        None
    }

    // --- time and weather ---------------------------------------------------------------
    fn clock(&self) -> Option<Clock> {
        None
    }
    fn set_time(&mut self, _seconds: f64) -> bool {
        false
    }
    fn set_date(&mut self, _y: i32, _m: u32, _d: u32) -> bool {
        false
    }
    fn time_speed(&self) -> Option<f64> {
        None
    }
    fn set_time_speed(&mut self, _x: f64) -> bool {
        false
    }
    fn paused(&self) -> bool {
        false
    }
    fn set_paused(&mut self, _on: bool) -> bool {
        false
    }
    /// The season's texture folder (none: the base textures) and whether snow lies.
    fn season(&self) -> Option<(Option<String>, bool)> {
        None
    }
    fn weather(&self) -> Option<Weather> {
        None
    }
    fn set_weather(&mut self, _w: &WeatherChange) -> Result<(), String> {
        Err("the game has no weather".into())
    }
    /// The weather files installed: (file, name).
    fn weather_presets(&self) -> Vec<(String, String)> {
        Vec::new()
    }
    /// Change to a weather file (None: the map's own) over `seconds`.
    fn set_weather_preset(&mut self, _file: Option<&str>, _seconds: f32) -> Result<(), String> {
        Err("the game has no weather".into())
    }

    // --- camera and input ---------------------------------------------------------------
    fn camera(&self) -> Option<Camera> {
        None
    }
    /// Switch the view: driver, pax, outside, free, map, ego, or a `view_*` action.
    fn set_view(&mut self, _name: &str) -> bool {
        false
    }
    /// Place the free camera (the view becomes `free`).
    fn set_free_camera(&mut self, _at: [f64; 3], _yaw: f32, _pitch: f32) -> bool {
        false
    }
    /// The zoom of the current view (its field of view times this).
    fn set_zoom(&mut self, _k: f32) -> bool {
        false
    }
    /// Turn the head (driver, passenger view) or the outside camera: yaw, pitch degrees.
    fn set_look(&mut self, _yaw: f32, _pitch: f32) -> bool {
        false
    }
    /// Whether a key is held now (winit's name: `KeyW`, `ShiftLeft`).
    fn key_held(&self, _name: &str) -> bool {
        false
    }
    fn keys_held(&self) -> Vec<String> {
        Vec::new()
    }
    fn mouse(&self) -> Option<Mouse> {
        None
    }
    fn controllers(&self) -> Vec<Controller> {
        Vec::new()
    }
    /// Key bindings: (action, key) of the game's (`vehicle` false) or the vehicles' keys.
    fn bindings(&self, _vehicle: bool) -> Vec<(String, String)> {
        Vec::new()
    }

    // --- sound --------------------------------------------------------------------------
    /// Play a WAV file; its voice id.
    fn sound_play(&mut self, _file: &Path, _s: &Sound) -> Option<u64> {
        None
    }
    fn sound_set(&mut self, _id: u64, _s: &Sound) -> bool {
        false
    }
    fn sound_stop(&mut self, _id: u64) {}
    fn sound_playing(&self, _id: u64) -> bool {
        false
    }
    /// The game's volume setting, 0..1.
    fn volume(&self) -> Option<f32> {
        None
    }

    // --- the game -----------------------------------------------------------------------
    /// The settings a plugin may read: graphics, language, interface size, units...
    fn settings(&self) -> Vec<(&'static str, InfoValue)> {
        Vec::new()
    }
    /// This session's counts as the personnel file has them.
    fn stats(&self) -> Vec<(&'static str, InfoValue)> {
        Vec::new()
    }
    /// Take a screenshot (the next frame); the file it goes to.
    fn screenshot(&mut self) -> Option<String> {
        None
    }
    /// A notification card of the game: kind 0 info, 1 warning, 2 alert.
    fn notify(&mut self, _text: &str, _kind: u8, _seconds: f32) -> bool {
        false
    }
    /// A game action of keyboard.cfg's `[game]` (`sim_pause`, `view_set_map`, ...).
    fn game_action(&mut self, _name: &str) -> bool {
        false
    }
    fn fps(&self) -> Option<f32> {
        None
    }
    fn menu_open(&self) -> bool {
        false
    }

    // --- LAN ----------------------------------------------------------------------------
    /// This game in a LAN session: (host, own id, own name).
    fn lan(&self) -> Option<(bool, u32, String)> {
        None
    }
    fn lan_players(&self) -> Vec<LanPlayer> {
        Vec::new()
    }
    /// Send a plugin's message to player `to` (the same plugin there gets it).
    fn lan_send(&mut self, _plugin: &str, _to: u32, _text: &str) -> Result<(), String> {
        Err("no LAN session".into())
    }
    /// A line in the session's chat.
    fn lan_chat(&mut self, _text: &str) -> Result<(), String> {
        Err("no LAN session".into())
    }
}

/// One of [`PluginIo::events`]: an event of that name, called with these values.
#[derive(Debug, Clone, PartialEq)]
pub struct GameEvent {
    pub name: &'static str,
    pub args: Vec<InfoValue>,
}

/// One of [`PluginIo::others`].
#[derive(Debug, Clone, PartialEq)]
pub struct Other {
    /// Stable while the vehicle is there: AI cars by their id, LAN players by theirs.
    pub id: u64,
    /// "ai" or "player".
    pub kind: &'static str,
    /// Manufacturer and type, as `omsi.vehicle()` gives the player's.
    pub name: String,
    /// x, y, z and heading in degrees, as `omsi.position()`.
    pub pos: [f64; 4],
}

/// A value of [`PluginIo::info`] or of an event.
#[derive(Debug, Clone, PartialEq)]
pub enum InfoValue {
    Num(f64),
    Text(String),
    Bool(bool),
    Nil,
}

/// A destination of the bus's depot file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Terminus {
    pub code: i64,
    /// What the display shows.
    pub name: String,
    /// The bus lets everybody out there (no destination of its own).
    pub all_exit: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ticket {
    pub name: String,
    pub price: f32,
    pub day_ticket: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Wheel {
    pub axle: usize,
    /// 0 left, 1 right.
    pub side: usize,
    pub rpm: f32,
    pub radius: f32,
    pub suspension: f32,
    pub driven: bool,
}

/// An AI vehicle.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AiCar {
    pub id: u64,
    /// "car", "taxi", "bus", "truck", "timetable_bus", "tram", "bicycle", "plane".
    pub kind: &'static str,
    pub name: String,
    pub pos: [f64; 4],
    pub speed_kmh: f64,
    pub max_speed_kmh: f64,
    /// Why it waits or slows: "lead", "light", "yield", "people", ... ("" driving freely).
    pub why: &'static str,
    /// Seconds it has stood still.
    pub standing: f32,
    pub braking: bool,
    /// 0 none, 1 left, 2 right.
    pub blinker: u8,
    /// The line of a timetable bus.
    pub line: Option<String>,
}

/// A timetable bus on the road.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AiBus {
    pub id: u64,
    pub line: String,
    pub tour: String,
    pub trip: String,
    pub terminus: String,
    pub departure: f64,
    pub next_stop_id: Option<i64>,
    pub at_stop: bool,
    pub trip_done: bool,
    pub delay: f64,
    pub x: f64,
    pub y: f64,
    pub number: String,
}

/// A traffic light ahead.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Light {
    /// "red", "red_yellow", "green", "green_yellow", "yellow", "dark".
    pub aspect: &'static str,
    /// Seconds to its next change.
    pub change_in: f32,
    /// Metres from the bus to it.
    pub distance: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Person {
    pub id: u32,
    pub pos: [f64; 3],
    /// "strolling", "idle", "standing", "waiting", "to_bus", "boarding", "riding", "seated",
    /// "leaving", "to_stop".
    pub state: &'static str,
    /// In the player's bus.
    pub aboard: bool,
    /// In a bus of the AI.
    pub in_ai_bus: Option<u64>,
    pub stop: Option<i64>,
    pub destination: Option<String>,
    pub ticket: Option<String>,
    /// 0 none, 1..3 how bad the ride was (3: they leave).
    pub complaint: u8,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaxStop {
    pub id: i64,
    pub name: String,
    pub pos: [f64; 3],
    pub waiting: usize,
}

/// A stop of a trip.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stop {
    pub id: i64,
    pub name: String,
    /// Seconds since midnight.
    pub arr: f64,
    pub dep: f64,
    pub pos: Option<[f64; 3]>,
    /// False: the bus passes it without stopping.
    pub stops: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Trip {
    pub name: String,
    pub line: String,
    pub terminus: String,
    pub departure: f64,
    pub end: f64,
    pub stops: Vec<Stop>,
}

/// The player's duty now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Duty {
    pub line: String,
    pub tour: String,
    /// The trip in the duty (from 0) and how many it has.
    pub trip: usize,
    pub trips: usize,
    /// The next stop's index in the trip (from 0).
    pub next: usize,
    pub at_stop: bool,
    pub trip_done: bool,
    /// Seconds late (early negative).
    pub delay: f64,
    /// The trip now.
    pub current: Trip,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Line {
    pub name: String,
    /// The player may drive it.
    pub user_allowed: bool,
    pub tours: Vec<Tour>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tour {
    pub number: String,
    /// It runs today.
    pub today: bool,
    pub trips: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MapInfo {
    pub name: String,
    pub friendly_name: String,
    pub path: String,
    pub left_hand_traffic: bool,
    pub tile_size: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tile {
    pub x: i32,
    pub y: i32,
    pub file: String,
    pub loaded: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MapStop {
    pub id: i64,
    pub name: String,
    pub pos: [f64; 4],
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    pub index: usize,
    pub name: String,
    pub pos: Option<[f64; 4]>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lane {
    pub index: usize,
    pub distance: f64,
    pub speed_limit_kmh: f32,
    pub name: String,
    pub has_light: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Clock {
    /// Seconds since midnight.
    pub time: f64,
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub day_of_year: i32,
    /// 0 Monday .. 6 Sunday.
    pub weekday: i32,
    /// Seconds played (never wraps).
    pub run_time: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Weather {
    pub name: String,
    pub visibility_m: f32,
    pub wind_dir: f32,
    pub wind_ms: f32,
    pub temperature: f32,
    /// g/m³ and per cent.
    pub humidity_abs: f32,
    pub humidity_rel: f32,
    pub pressure: f32,
    pub clouds: String,
    pub cloud_base_m: f32,
    /// 0 none, 1 rain, 2 snow; 0..1.
    pub precip_kind: i32,
    pub precip_rate: f32,
    pub snow_cover: bool,
    pub snow_on_road: bool,
    /// How wet the roads are, 0..1.
    pub wetness: f32,
    /// A change of weather is coming in.
    pub changing: bool,
    /// The weather follows a real station (METAR): it cannot be set.
    pub locked: bool,
}

/// What `weather.set` changes (none: left as it is).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WeatherChange {
    pub visibility_m: Option<f32>,
    pub wind_dir: Option<f32>,
    pub wind_ms: Option<f32>,
    pub temperature: Option<f32>,
    pub pressure: Option<f32>,
    pub clouds: Option<String>,
    pub cloud_base_m: Option<f32>,
    pub precip_kind: Option<i32>,
    pub precip_rate: Option<f32>,
    pub snow_cover: Option<bool>,
    pub snow_on_road: Option<bool>,
    pub wetness: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Camera {
    pub view: String,
    pub pos: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    /// Vertical field of view, degrees.
    pub fov: f32,
    pub in_cab: bool,
    pub zoom: f32,
    /// The head turned (driver, passenger) or the outside camera swung: yaw, pitch.
    pub look: (f32, f32),
    /// The picture's size in pixels.
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mouse {
    /// Pixels of the window.
    pub x: f32,
    pub y: f32,
    pub left: bool,
    pub right: bool,
    pub middle: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Controller {
    pub name: String,
    pub gamepad: bool,
    /// Each axis, -1..1 or 0..1 as the device gives it.
    pub axes: Vec<f32>,
    pub buttons: usize,
}

/// How a plugin's sound plays.
#[derive(Debug, Clone, PartialEq)]
pub struct Sound {
    pub volume: f32,
    pub pitch: f32,
    pub looping: bool,
    /// None: everywhere alike (2D).
    pub at: Option<[f64; 3]>,
    /// It moves with the player's bus (`at` then is the bus's place).
    pub on_bus: bool,
    /// Metres it is heard at full volume.
    pub range: f32,
}

impl Default for Sound {
    fn default() -> Sound {
        Sound { volume: 1.0, pitch: 1.0, looping: false, at: None, on_bus: false, range: 5.0 }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LanPlayer {
    pub id: u32,
    pub name: String,
    pub host: bool,
    /// The `.bus` file (empty: no vehicle).
    pub bus: String,
    pub line: String,
    pub tour: String,
    pub pos: [f64; 4],
    pub speed_kmh: f32,
    pub on_foot: bool,
    pub passengers: u32,
}
