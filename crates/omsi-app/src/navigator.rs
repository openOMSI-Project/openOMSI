//! The route navigator: a small tilted 3D map in a corner of the screen, after the Route
//! Advisor of Euro Truck Simulator 2, made for a bus driver's duty.
//!
//! * The map turns with the bus and zooms out with speed; the camera looks over the bus
//!   from behind and above, so the road ahead fills the picture.
//! * Roads are drawn from the traffic network's lanes (dark casing, grey surface); the
//!   trip's route from the first stop to the last lies on them with arrows, coloured by
//!   how busy each stretch is (blue empty, green, yellow, red, dark red jammed), and its
//!   stops as markers (the last is a chequered flag).
//! * Leaving the route is noticed after a moment: a way back is searched on the lanes
//!   (Dijkstra, towards any lane of the route still ahead) and drawn instead - the route
//!   is recalculated as often as the driver goes wrong.
//! * Traffic: every AI vehicle near the bus is on the map, and roads whose cars crawl or
//!   stand are tinted amber or deep red (smoothed over seconds, so one car at a red light
//!   is no jam); the route itself takes the colour where it runs into one, and the header
//!   says how long it costs.
//! * The header: speed, the line, passengers aboard, day and time; the next stop with its
//!   distance and whether the bus is early or late; the speed limit sign and signals ahead
//!   on the map.
//!
//! Everything is drawn with `omsi-ui` into a texture of its own (4x MSAA) that the game
//! shows as a premultiplied overlay. Roads are built once per area and kept on the GPU;
//! their width is in metres near the camera and in pixels far away, so zooming rebuilds
//! nothing.

use glam::{DMat3, DVec2, DVec3, Mat4, Vec2, Vec3};
use hashbrown::HashMap;
use omsi_render::{Renderer, Scene, TextureId};
use omsi_sim::traffic::{LaneKey, LaneKind, Network};
use omsi_ui::paint::Align;
use omsi_ui::{Atlas, Color, Draw, Fonts, Gpu, Layer, Painter, Rect, Weight};

use crate::traffic::Traffic;

// --- colours (sRGB) -------------------------------------------------------------------

// neutral dark, half transparent, calm
const NAV_REDRAW_S: f32 = 1.0 / 30.0;
const PANEL: Color = Color::rgba(10, 10, 10, 0.70);
// (the bars under the texts darken whatever the opacity setting leaves of the panel: at a
// third the cab showed through behind the next stop)
const BAR: Color = Color::rgba(0, 0, 0, 0.55);
// (the launcher's map picture draws with these too, so the two maps cannot look apart)
pub(crate) const ROAD_CASING: Color = Color::rgba(30, 30, 30, 0.9);
pub(crate) const ROAD: Color = Color::rgba(92, 92, 92, 1.0);
pub(crate) const ROAD_MAIN: Color = Color::rgba(112, 112, 112, 1.0);
pub(crate) const ROUTE: Color = Color::rgba(214, 48, 40, 1.0);
const DOT: Color = Color::rgba(70, 140, 255, 1.0);
/// The public transport's dots: trolleybuses, buses, trams, and the text on their line tags.
const TROLLEY: Color = Color::rgba(46, 184, 92, 1.0);
const BUS: Color = Color::rgba(226, 58, 52, 1.0);
const TRAM: Color = Color::rgba(240, 190, 30, 1.0);
const LINE_TEXT: Color = Color::rgba(15, 15, 15, 1.0);
/// The other players of a LAN session or server: an arrow the way they face, with their name.
const PLAYER: Color = Color::rgba(190, 96, 255, 1.0);

/// What a traffic vehicle is on the map: a trolleybus (its model has trolley poles to
/// raise, `cp_SHTANGALEV` or `shtanga_lev_rot`), a tram, a bus (one on a timetable or
/// showing a line), or any other car; and the line it shows, if it is public transport.
fn traffic_kind(c: &crate::traffic::AiCar) -> (Color, Option<String>) {
    let v = &c.vehicle;
    let line = v
        .ty
        .program
        .str_var("SetLineTo")
        .and_then(|i| v.state.str_vars.get(i as usize))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "0");
    let trolley = v.var("cp_SHTANGALEV").is_some() || v.var("shtanga_lev_rot").is_some();
    let color = if c.is_rail() {
        TRAM
    } else if trolley {
        TROLLEY
    } else if c.bus.is_some() || line.is_some() {
        BUS
    } else {
        return (DOT, None);
    };
    (color, line)
}
/// A bus, trolleybus or tram on the map as a pictogram, not a dot: a body with a cut front and
/// a windscreen, turned to the way it heads (`heading`: degrees clockwise from north), a dark
/// edge round it. `min_px` keeps it readable from far away; the line number is drawn beside it.
fn vehicle_icon(p: &mut Painter, at: Vec3, heading: f64, color: Color, long: bool, min_px: f32) {
    let h = heading.to_radians() as f32;
    let (sn, cs) = h.sin_cos();
    // local: x to the right, y forward; turned onto the world (x east, y north)
    let turn = |v: Vec2| Vec2::new(v.x * cs + v.y * sn, -v.x * sn + v.y * cs);
    let l = if long { 1.5 } else { 1.0 };
    let body = |k: f32| -> Vec<Vec2> {
        [(-0.34, -1.0), (0.34, -1.0), (0.34, 0.55), (0.2, 1.0), (-0.2, 1.0), (-0.34, 0.55)].iter().map(|&(x, y)| turn(Vec2::new(x * k, y * l * k))).collect()
    };
    let (m, px) = (6.0, min_px);
    p.world_shape(at, &body(1.28), m, px * 1.28, Color::rgba(8, 8, 8, 0.92));
    p.world_shape(at, &body(1.0), m, px, color);
    let glass: Vec<Vec2> = [(-0.24, 0.5), (0.24, 0.5), (0.15, 0.8), (-0.15, 0.8)].iter().map(|&(x, y)| turn(Vec2::new(x, y * l))).collect();
    p.world_shape(at, &glass, m, px, Color::rgba(240, 244, 248, 0.9));
}

/// The route by how busy its roads are: empty, light, busy, heavy, jammed.
const LEVEL: [Color; 5] = [
    Color::rgba(46, 116, 240, 1.0),
    Color::rgba(56, 178, 86, 1.0),
    Color::rgba(236, 192, 40, 1.0),
    Color::rgba(224, 56, 44, 1.0),
    Color::rgba(122, 16, 22, 1.0),
];
/// The arrows on it, in a colour that stands out from each.
const ARROW: [Color; 5] = [
    Color::rgba(236, 244, 255, 1.0),
    Color::rgba(12, 66, 28, 1.0),
    Color::rgba(92, 58, 0, 1.0),
    Color::rgba(150, 240, 150, 1.0),
    Color::rgba(255, 206, 80, 1.0),
];
/// The part of the route already driven (city map).
const DRIVEN: Color = Color::rgba(62, 70, 86, 1.0);
const STREET: Color = Color::rgba(178, 178, 178, 1.0);

/// How busy a road is (0 empty … 4 jammed) from its congestion score.
fn level(score: f32) -> usize {
    match score {
        s if s < 0.12 => 0,
        s if s < 0.40 => 1,
        s if s < 0.60 => 2,
        s if s < 0.80 => 3,
        _ => 4,
    }
}
const TEXT: Color = Color::rgba(235, 235, 235, 1.0);
// (the second texts - units, the day, the times - bright enough to read on a lit cab)
const TEXT_DIM: Color = Color::rgba(178, 178, 178, 1.0);
const LATE: Color = Color::rgba(235, 85, 70, 1.0);
const EARLY: Color = Color::rgba(90, 160, 240, 1.0);
const ON_TIME: Color = Color::rgba(110, 200, 120, 1.0);
const WARN: Color = Color::rgba(235, 170, 60, 1.0);
const STOP_REQUEST: Color = Color::hex(0xF0A030);

pub(crate) fn stop_requested(vehicle: &omsi_sim::vehicle::VehicleInstance) -> bool {
    ["haltewunsch", "haltewunschlampe"].iter().any(|name| vehicle.var(name).is_some_and(|v| v > 0.5))
}

fn stop_request_icon(ui: &mut Painter, atlas: &mut Atlas, requested: bool, mut row: Rect, scale: f32) -> Rect {
    if requested {
        let size = 32.0 * scale;
        ui.icon(atlas, "stop_request", Vec2::new(row.right() - size * 0.5, row.center().y), size, STOP_REQUEST);
        row.w -= size + 6.0 * scale;
    }
    row
}

/// Vertical field of view and tilt of the map camera (degrees).
const FOV: f32 = 40.0;
const PITCH: f64 = 52.0;
/// Roads are built for this far around the bus (m) and again when it has gone half-way.
const ROAD_RADIUS: f64 = 1300.0;
/// Off the route for this long (s) before a way back is looked for, and between tries.
const OFF_ROUTE_AFTER: f32 = 2.0;
const REROUTE_EVERY: f32 = 2.5;

/// One stop of the trip for the map.
#[derive(Debug, Clone)]
pub struct NavStop {
    /// The stop's map object (its place is looked up in the navigator's map when the
    /// timetable does not know it: its tile is not loaded yet).
    pub object_id: i64,
    pub position: DVec3,
    pub name: String,
    /// Planned arrival (s of the day).
    pub arrival: f64,
}

/// Another player of the session as the maps show them.
#[derive(Debug, Clone, PartialEq)]
pub struct NavPlayer {
    /// Where they are: their bus, or on foot where they walk (riding in someone's bus: that bus).
    pub position: DVec3,
    /// Compass heading (degrees, 0 = +y, clockwise).
    pub heading: f64,
    pub name: String,
}

/// What the navigator is told each frame.
pub struct NavFrame<'a> {
    pub traffic: Option<&'a Traffic>,
    /// The other players of a LAN session or server, on both maps (#1011, #1080).
    pub players: Vec<NavPlayer>,
    pub bus: DVec3,
    /// Compass heading (degrees, 0 = +y, clockwise).
    pub heading: f64,
    pub speed_kmh: f32,
    /// Outside weather and cabin air temperatures (°C).
    pub outside_temp: f32,
    pub inside_temp: f32,
    /// Line, terminus, and the trip's stops from the next one on (the next is first).
    pub line: Option<String>,
    pub terminus: Option<String>,
    pub stops: Vec<NavStop>,
    /// How late the bus is (s, negative early), when on a duty.
    pub delay: Option<f64>,
    pub passengers: Option<usize>,
    /// The vehicle script's latched stop request or illuminated request lamp.
    pub stop_requested: bool,
    /// Seconds of the day and weekday (0 = Monday).
    pub time: f64,
    pub weekday: i32,
    pub language: &'a str,
    /// Window size in physical pixels.
    pub screen: (f32, f32),
    /// The player's interface size (`Settings::ui_scale`): the panel and the city map's
    /// texts and buttons grow with it.
    pub ui_scale: f32,
    /// The interface grows with a tall window (`Settings::ui_scale_window`); off, the panel
    /// is held to 480 px, as before.
    pub follow_window: bool,
    pub dt: f32,
    /// Where the information bar is (`ui::Ui::info_rect`): a navigator along the top keeps
    /// below it.
    pub info_rect: Option<[f32; 4]>,
}

/// The texts, per language.
struct Words {
    kmh: &'static str,
    days: [&'static str; 7],
    off_route: &'static str,
    rerouting: &'static str,
    recalculated: &'static str,
    jam: &'static str,
    slow: &'static str,
    no_duty: &'static str,
    last_stop: &'static str,
    on_time: &'static str,
}

fn words(lang: &str) -> Words {
    match lang.to_ascii_uppercase().as_str() {
        "DEU" | "DE" | "GER" => Words { kmh: "km/h", days: ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"], off_route: "Abseits der Route", rerouting: "Route wird neu berechnet", recalculated: "Route neu berechnet", jam: "Stau", slow: "Zähfließend", no_duty: "Freie Fahrt", last_stop: "Endhaltestelle", on_time: "pünktlich" },
        "FRA" | "FR" => Words { kmh: "km/h", days: ["Lun", "Mar", "Mer", "Jeu", "Ven", "Sam", "Dim"], off_route: "Hors itinéraire", rerouting: "Recalcul de l'itinéraire", recalculated: "Itinéraire recalculé", jam: "Bouchon", slow: "Ralentissement", no_duty: "Conduite libre", last_stop: "Terminus", on_time: "à l'heure" },
        "RUS" | "RU" => Words { kmh: "км/ч", days: ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"], off_route: "Вне маршрута", rerouting: "Перестроение маршрута", recalculated: "Маршрут перестроен", jam: "Пробка", slow: "Затруднено", no_duty: "Свободная езда", last_stop: "Конечная", on_time: "по графику" },
        _ => Words { kmh: "km/h", days: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"], off_route: "Off route", rerouting: "Recalculating route", recalculated: "Route recalculated", jam: "Traffic jam", slow: "Slow traffic", no_duty: "Free drive", last_stop: "Final stop", on_time: "on time" },
    }
}

/// The route being followed: the lanes, how far along the bus is, and whether it was
/// replaced by a way back after leaving it.
#[derive(Default)]
struct Route {
    key: String,
    lanes: Vec<usize>,
    complete: bool,
    generation: u64,
    /// Index into `lanes` of the lane the bus is on, and how far along it.
    progress: usize,
    s: f32,
    on_route: bool,
    off_for: f32,
    retry_in: f32,
    /// Seconds the "recalculated" note still shows.
    note: f32,
    /// Bumped whenever `lanes` or `progress` change (the cached route mesh).
    version: u64,
    /// `lanes` is a way to the next stop found by the navigator itself (the trip's own
    /// route is not there yet: its tiles still load); the trip's replaces it.
    provisional: bool,
    /// The bus has been on the route: before that, the way to it is simply the way to the
    /// next stop (found at once, and no "recalculating").
    joined: bool,
    /// `lanes` begins with a way to the route (the bus is looked for on it too).
    approach: bool,
}

/// Roads around a centre, built into vertex buffer 0.
struct Roads {
    anchor: DVec2,
    lanes_seen: usize,
    verts: usize,
    built_at: f32,
}

pub struct Navigator {
    /// The current small-map texture in Scene::overlays, for the cockpit display.
    pub panel_overlay: Option<usize>,
    pub cockpit_display: bool,
    drawn_at: f32,
    pub enabled: bool,
    /// The next stops with their times under the map (Shift+N cycles map, map and
    /// schedule, off).
    pub schedule: bool,
    /// Smoothed speed (m/s) for the time to the next stop.
    speed_avg: f32,
    pub opacity: f32,
    pub corner: String,
    /// Where the navigator was dragged to (#940): its top-left corner as a share of the
    /// room the window leaves it across and down (`navigator_corner = at x,y`); None: the
    /// corner.
    pub at: Option<[f32; 2]>,
    /// A press on the navigator: where it was (cursor and the panel's top-left) and whether
    /// it has moved far enough to be a drag rather than a click.
    panel_drag: Option<([f32; 2], [f32; 2], bool)>,
    /// The room the window leaves the navigator (width, height) when it was last placed.
    panel_room: [f32; 2],
    /// The city map (a click on the navigator or Shift+M) and where the navigator is on the
    /// screen.
    pub city: CityMap,
    panel_rect: [f32; 4],
    origin_x: f32,
    gpu: Option<Gpu>,
    fonts: Fonts,
    atlas: Atlas,
    target: Option<(TextureId, u32, u32)>,
    /// The map's own lanes when there is no traffic system.
    own_net: Option<std::sync::Arc<Network>>,
    /// The whole map's road network and object places (read in the background at the
    /// start, see `World::navigation_map`): routes, roads and stops beyond the loaded
    /// tiles. Its version changes when it arrives.
    global: Option<std::sync::Arc<Network>>,
    stop_pos: std::sync::Arc<HashMap<i64, DVec3>>,
    /// Street names of the map's lanes (from its street name signs).
    streets: Option<std::sync::Arc<Streets>>,
    #[allow(clippy::type_complexity)]
    building: Option<std::sync::mpsc::Receiver<(Network, HashMap<i64, DVec3>, Streets)>>,
    pub global_version: u64,
    roads: Option<Roads>,
    route: Route,
    /// The route mesh (buffer 1): the route and traffic versions and the anchor it was
    /// built for, and its size.
    route_mesh: (u64, u64, DVec2, u32, usize),
    /// Per lane of the traffic: how congested it is (0 free … 1 standing), smoothed.
    congestion: HashMap<usize, f32>,
    /// The same for the route's lanes ahead (in the route's network), and a version that
    /// changes when any of their levels does (the route is drawn again).
    route_jam: HashMap<usize, f32>,
    jam_version: u64,
    congestion_t: f32,
    /// Map camera: distance, heading, both smoothed.
    zoom: f64,
    cam_heading: f64,
    time: f32,
    /// Distance along the route to the next stop (m), refreshed now and then.
    next_dist: Option<f64>,
    dist_t: f32,
    /// The next turn on the route: direction (-1 left, 1 right, 2 back), how sharp
    /// (degrees), how far (m) and the street it turns into.
    /// OMSI 2's route arrows are wanted (see `turn_hint`).
    pub arrows: bool,
    /// The other (AI) vehicles are drawn on the maps (the `nav_ai` setting).
    pub show_ai: bool,
    /// How far the panel has faded in (Shift+N fades it in and out rather than cutting).
    shown: f32,
    /// The trip's next stops (place, name, the bus's heading there) and where the bus is,
    /// for the route arrows.
    stop_spots: Vec<(DVec3, String, f64, i64)>,
    bus_at: DVec3,
    next_turn: Option<(i32, f32, f64, Option<String>)>,
    /// The street the bus is on.
    street_here: Option<String>,
    /// Seconds of delay the jams ahead on the route cost.
    jam_cost: f32,
    first: bool,
}

/// The duty as the navigator shows it: line, terminus, the stops from the next one on,
/// and the current trip (a key that changes with it, and its name).
#[allow(clippy::type_complexity)]
pub fn duty_parts(duty: Option<&crate::schedule::PlayerDuty>) -> (Option<String>, Option<String>, Vec<NavStop>, Option<(String, String)>) {
    let Some(d) = duty else { return (None, None, Vec::new(), None) };
    let Some(trip) = d.trips.get(d.trip_index) else { return (None, None, Vec::new(), None) };
    let line = if trip.line.trim().is_empty() { d.line.trim() } else { trip.line.trim() };
    let stops = trip
        .stops
        .iter()
        .skip(d.next_stop)
        .filter(|s| s.stops)
        .map(|s| NavStop { object_id: s.object_id, position: s.position.unwrap_or(DVec3::ZERO), name: s.name.clone(), arrival: s.arr })
        .collect();
    (Some(line.to_string()), Some(trip.terminus.clone()), stops, Some((format!("{}/{}", d.trip_index, trip.name), trip.name.clone())))
}

/// Screen position of a world point (relative to the anchor) in target pixels.
fn project(vp: Mat4, viewport: [f32; 4], p: Vec3) -> Option<Vec2> {
    let c = vp * p.extend(1.0);
    if c.w <= 0.1 {
        return None;
    }
    let n = c.truncate() / c.w;
    Some(Vec2::new(viewport[0] + (n.x * 0.5 + 0.5) * viewport[2], viewport[1] + (0.5 - n.y * 0.5) * viewport[3]))
}

fn angle_diff(a: f64, b: f64) -> f64 {
    let mut d = (b - a) % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d < -180.0 {
        d += 360.0;
    }
    d
}

fn ease(dt: f32, tau: f32) -> f64 {
    (1.0 - (-dt / tau.max(1e-3)).exp()) as f64
}

/// A `navigator_corner` of the form `at x,y` (where the navigator was dragged to, #940).
pub(crate) fn placed_at(corner: &str) -> Option<[f32; 2]> {
    let (x, y) = corner.trim().strip_prefix("at")?.trim().split_once(',')?;
    let (x, y) = (x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?);
    (x.is_finite() && y.is_finite()).then(|| [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)])
}

/// The GPS is a separate render texture composed after the scene's post AA.
fn map_samples(format: wgpu::TextureFormat) -> u32 {
    if format.guaranteed_format_features(wgpu::Features::empty()).flags.sample_count_supported(4) { 4 } else { 1 }
}

impl Navigator {
    pub fn new(enabled: bool, opacity: f32, corner: &str) -> Navigator {
        Navigator {
            panel_overlay: None,
            cockpit_display: false,
            drawn_at: f32::MIN,
            enabled,
            schedule: false,
            speed_avg: 8.0,
            opacity: opacity.clamp(0.2, 1.0),
            corner: corner.to_string(),
            at: placed_at(corner),
            panel_drag: None,
            panel_room: [0.0; 2],
            city: CityMap::default(),
            panel_rect: [0.0; 4],
            origin_x: 0.0,
            gpu: None,
            fonts: Fonts::new(),
            atlas: Atlas::new(1024),
            target: None,
            own_net: None,
            global: None,
            stop_pos: Default::default(),
            streets: None,
            building: None,
            global_version: 0,
            roads: None,
            route: Route::default(),
            route_mesh: (u64::MAX, 0, DVec2::ZERO, 0, 0),
            congestion: HashMap::new(),
            route_jam: HashMap::new(),
            jam_version: 0,
            congestion_t: 0.0,
            zoom: 120.0,
            cam_heading: 0.0,
            time: 0.0,
            next_dist: None,
            arrows: false,
            show_ai: true,
            shown: if enabled { 1.0 } else { 0.0 },
            stop_spots: Vec::new(),
            bus_at: DVec3::ZERO,
            next_turn: None,
            street_here: None,
            dist_t: 0.0,
            jam_cost: 0.0,
            first: true,
        }
    }

    /// Read the whole map's road network on a worker (once per session).
    pub fn start_map(&mut self, world: std::sync::Arc<crate::scene::World>) {
        if self.building.is_some() || self.global.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("navigator map".into())
            .spawn(move || {
                let m = world.navigation_map();
                let mut net = Network { lanes: m.lanes, ..Default::default() };
                net.link(1.5);
                confirm_road_surfaces(&mut net, &m.road_surfaces);
                probe_lanes(&net);
                let streets = build_streets(&net, &m.signs);
                let _ = tx.send((net, m.places, streets));
            })
            .ok();
        self.building = Some(rx);
    }

    /// The map's network given at once (an offscreen picture reads it on its own thread).
    pub fn set_map(&mut self, map: crate::scene::NavigationMap) {
        let mut net = Network { lanes: map.lanes, ..Default::default() };
        net.link(1.5);
        confirm_road_surfaces(&mut net, &map.road_surfaces);
        probe_lanes(&net);
        self.streets = Some(std::sync::Arc::new(build_streets(&net, &map.signs)));
        let global = std::sync::Arc::new(net);
        self.global = Some(global);
        self.stop_pos = std::sync::Arc::new(map.places);
        self.global_version += 1;
    }

    /// The places of every object on the map (stops beyond the loaded tiles), once read.
    pub fn places(&self) -> Option<&HashMap<i64, DVec3>> {
        self.global.as_ref().map(|_| &*self.stop_pos)
    }

    /// The whole map's network, once read.
    pub fn map_net(&self) -> Option<&Network> {
        self.global.as_deref()
    }

    /// The map's lanes, for a session without a traffic system (added as tiles stream in).
    pub fn add_lanes(&mut self, lanes: Vec<omsi_sim::traffic::Lane>) {
        if lanes.is_empty() {
            return;
        }
        match self.own_net.as_mut() {
            Some(n) => {
                // (only `draw` borrows it, for the length of a frame)
                if let Some(n) = std::sync::Arc::get_mut(n) {
                    n.extend(lanes, omsi_sim::traffic::LINK_TOLERANCE);
                    n.build_grid();
                }
            }
            None => {
                let mut n = Network { lanes, ..Default::default() };
                n.link(omsi_sim::traffic::LINK_TOLERANCE);
                n.build_grid();
                self.own_net = Some(std::sync::Arc::new(n));
            }
        }
    }

    /// Does the route of trip `key` need (re)building? (a new trip, or tiles brought the
    /// lanes a partial route was missing)
    pub fn wants_route(&self, key: &str, generation: u64) -> bool {
        if self.global.is_some() {
            return self.route.key != key || self.route.generation != self.global_version + (1 << 40);
        }
        self.route.key != key || (!self.route.complete && self.route.generation != generation)
    }

    /// The route of trip `key`: its lanes as the timetable drives them.
    pub fn set_route(&mut self, key: &str, lanes: Vec<usize>, complete: bool, generation: u64) {
        let same_trip = self.route.key == key;
        self.route.key = key.to_string();
        self.route.complete = complete;
        self.route.generation = generation;
        if same_trip && !self.route.lanes.is_empty() && !self.route.on_route && !self.route.provisional {
            // a partial route grew while the driver was off it: keep the way back
            return;
        }
        if self.route.provisional && lanes.is_empty() {
            return;
        }
        self.route.provisional = false;
        self.route.lanes = lanes;
        if !same_trip {
            self.route.progress = 0;
            self.route.on_route = false;
            self.route.off_for = 0.0;
        }
        self.route.version += 1;
    }

    /// No duty: nothing to follow.
    pub fn clear_route(&mut self) {
        if !self.route.key.is_empty() || !self.route.lanes.is_empty() {
            self.route = Route { version: self.route.version + 1, ..Route::default() };
        }
    }


    /// Where the bus is along the route, and a way back when it left it.
    fn follow(&mut self, f: &NavFrame) {
        let global = self.global.clone();
        let Some(net) = global.as_deref().or(f.traffic.map(|t| &t.net)) else { return };
        let r = &mut self.route;
        if r.lanes.is_empty() {
            // no route (yet): the way to the next stop, looked for now and then
            let Some(stop) = f.stops.first() else { return };
            r.retry_in -= f.dt;
            if r.retry_in > 0.0 {
                return;
            }
            r.retry_in = REROUTE_EVERY * 2.0;
            let targets: Vec<usize> = lanes_near(net, stop.position.truncate(), 40.0)
                .into_iter()
                .filter(|&i| net.lanes[i].kind == LaneKind::Street && net.lanes[i].nearest_point(stop.position).map(|p| p.1 < 20.0).unwrap_or(false))
                .collect();
            if let Some((mut path, k)) = way_back(net, f.bus, f.heading, &targets, 30_000.0) {
                path.push(targets[k]);
                log::info!("navigator: no route of the trip here yet; {} lanes to the next stop '{}'", path.len(), stop.name.trim());
                r.lanes = path;
                r.progress = 0;
                r.s = 0.0;
                r.version += 1;
                r.provisional = true;
            }
            return;
        }
        r.note = (r.note - f.dt).max(0.0);
        // where the next stop is on the route: the way to the route must not skip it
        let stop_at = f.stops.first().and_then(|st| {
            (r.progress..r.lanes.len().min(r.progress + 1500)).find(|&k| {
                net.lanes.get(r.lanes[k]).and_then(|l| l.nearest_point(st.position)).map(|p| p.1 < 25.0).unwrap_or(false)
            })
        });
        // the nearest route lane near the bus that runs its way, a little back to a lot ahead
        // (before the bus first reached the route: only the stretch up to the next stop)
        let (from, to) = match (r.joined, stop_at) {
            (false, Some(k)) if r.approach => (r.progress.saturating_sub(3), (k + 2).min(r.lanes.len())),
            (false, Some(k)) => (k.saturating_sub(40).max(r.progress), (k + 2).min(r.lanes.len())),
            _ => (r.progress.saturating_sub(3), (r.progress + 60).min(r.lanes.len())),
        };
        let mut best: Option<(usize, f32, f64)> = None;
        for k in from..to {
            let Some(l) = net.lanes.get(r.lanes[k]) else { continue };
            let Some((s, d)) = l.nearest_point(f.bus) else { continue };
            // (a wide road, a stop bay, a lane change or a turn in progress: the bus is still
            // on its route well beyond the lane's middle; at 11 m and 75 deg it counted as off
            // and the navigator recalculated for ever)
            if d > 16.0 {
                continue;
            }
            let (_, h) = l.at(s);
            if angle_diff(f.heading, h as f64).abs() > 100.0 {
                continue;
            }
            // (ahead of where the bus was preferred: a route crossing itself)
            let score = d + if k < r.progress { 4.0 } else { 0.0 } + (k.saturating_sub(r.progress) as f64) * 0.05;
            if best.map(|b| score < b.2).unwrap_or(true) {
                best = Some((k, s, score));
            }
        }
        match best {
            Some((k, s, _)) => {
                if k != r.progress {
                    r.version += 1;
                }
                r.progress = k;
                r.s = s;
                r.on_route = true;
                r.joined = true;
                r.off_for = 0.0;
            }
            None => {
                r.on_route = false;
                r.off_for += f.dt;
            }
        }
        // the way to the route: at once before the bus first reached it (from the depot or
        // wherever it starts), after a moment off it once driving it
        if r.on_route || (r.joined && r.off_for < OFF_ROUTE_AFTER) {
            return;
        }
        r.retry_in -= f.dt;
        if r.retry_in > 0.0 {
            return;
        }
        r.retry_in = REROUTE_EVERY;
        let base = r.progress.min(r.lanes.len() - 1);
        // targets: the route from where the bus left it up to the next stop (joining it
        // later would skip that stop), or on for 120 lanes when no stop is ahead
        let (lo, hi) = match stop_at {
            Some(k) => (k.saturating_sub(150).max(base), k + 1),
            None => (base, (base + 120).min(r.lanes.len())),
        };
        // (from the depot to the first stop may be a long way; back onto the route is not)
        let way = way_back(net, f.bus, f.heading, &r.lanes[lo..hi], if r.joined { 6000.0 } else { 30_000.0 });
        if way.is_none() && omsi_cfg::flags::OMSI_DEBUG_NAV.is_set() {
            let info: Vec<_> = r.lanes[lo..hi].iter().map(|&l| (l, net.lanes[l].kind, net.lanes[l].name.clone(), net.lanes.iter().filter(|x| x.next.contains(&l)).count(), net.lanes[l].start())).collect();
            log::info!("navigator: off the route for {:.1} s and no way back found; targets {info:?}", r.off_for);
        }
        // the route's own lanes before a stop may be cut off from the road network (the
        // map links nothing into the start of line 31's trip at Maulbeerallee): then a
        // road past the stop running the route's way, else the route just after the stop
        let max = if r.joined { 6000.0 } else { 30_000.0 };
        let way = way.map(|(p, j)| (p, lo + j)).or_else(|| {
            let k = stop_at?;
            let st = f.stops.first()?;
            let (_, h) = net.lanes[r.lanes[k]].nearest_point(st.position).map(|(s, _)| net.lanes[r.lanes[k]].at(s))?;
            let near: Vec<usize> = lanes_near(net, st.position.truncate(), 40.0)
                .into_iter()
                .filter(|&i| {
                    let l = &net.lanes[i];
                    l.kind == LaneKind::Street
                        && l.nearest_point(st.position).map(|(s, d)| d < 20.0 && angle_diff(l.at(s).1 as f64, h as f64).abs() < 60.0).unwrap_or(false)
                })
                .collect();
            let (mut path, j) = way_back(net, f.bus, f.heading, &near, max)?;
            path.push(near[j]);
            log::info!("navigator: the route before stop '{}' cannot be reached; led onto the road past it", st.name.trim());
            Some((path, k + 1))
        })
            .or_else(|| {
                let k = stop_at?;
                let hi = (k + 120).min(r.lanes.len());
                (k + 1 < hi).then_some(())?;
                way_back(net, f.bus, f.heading, &r.lanes[k + 1..hi], max).map(|(p, j)| (p, k + 1 + j))
            });
        if let Some((path, join)) = way {
            let rest = r.lanes[join.min(r.lanes.len())..].to_vec();
            log::info!(
                "navigator: {} of {} lanes joins the route {} lanes on{}",
                if r.joined { "a way back" } else { "the way to the route" },
                path.len(),
                join,
                stop_at.map(|k| format!(" (the next stop is on route lane {k})")).unwrap_or_default()
            );
            let mut lanes = path;
            lanes.extend(rest);
            // (drawn from where the bus is on its first lane, not from that lane's start)
            r.s = lanes.first().and_then(|&l| net.lanes.get(l)).and_then(|l| l.nearest_point(f.bus)).map(|p| p.0).unwrap_or(0.0);
            r.lanes = lanes;
            r.progress = 0;
            r.version += 1;
            if r.joined {
                r.note = 4.0;
                r.on_route = true;
                r.off_for = 0.0;
            } else {
                r.approach = true;
            }
        }
    }

    /// How congested the lanes near the bus are: a lane with cars on it is light traffic,
    /// more so the fuller it is; cars crawling or standing in a row make it heavy or a jam
    /// (one car waiting at a light is not a jam). Smoothed over seconds.
    fn update_congestion(&mut self, f: &NavFrame) {
        self.congestion_t -= f.dt;
        if self.congestion_t > 0.0 {
            return;
        }
        let step = 0.25f32;
        self.congestion_t = step;
        let Some(t) = f.traffic else { return };
        let mut by_lane: HashMap<usize, (f32, u32)> = HashMap::new();
        for c in &t.cars {
            if c.gone || (c.vehicle.position - f.bus).truncate().length() > 1200.0 {
                continue;
            }
            let e = by_lane.entry(c.state.lane).or_insert((0.0, 0));
            e.0 += c.state.speed.max(0.0);
            e.1 += 1;
        }
        let k = ease(step, 4.0) as f32;
        for v in self.congestion.values_mut() {
            *v -= *v * k;
        }
        for (lane, (sum, n)) in by_lane {
            let Some(l) = t.net.lanes.get(lane) else { continue };
            let expected = (l.speed_limit_kmh.clamp(20.0, 70.0) / 3.6) * 0.75;
            let slow = (1.0 - (sum / n as f32) / expected).clamp(0.0, 1.0);
            let full = (n as f32 * 7.5 / l.length().max(25.0)).min(1.0);
            let light = 0.22 + 0.33 * full;
            let jam = slow * (n as f32 / 3.0).min(1.0);
            let score = if jam > 0.2 { light.max(0.3 + 0.7 * jam) } else { light };
            let v = self.congestion.entry(lane).or_insert(0.0);
            *v += (score - *v) * k;
        }
        self.congestion.retain(|_, v| *v > 0.03);
        // the same for the route ahead, in the route's own network
        let global = self.global.clone();
        let jam = match global.as_deref() {
            Some(g) => congestion_on(g, &t.net, &self.congestion),
            None => self.congestion.clone(),
        };
        let net = global.as_deref().unwrap_or(&t.net);
        let r = &self.route;
        let mut route_jam = HashMap::new();
        let mut cost = 0.0;
        for &l in r.lanes.iter().skip(r.progress).take(600) {
            let Some(&c) = jam.get(&l) else { continue };
            route_jam.insert(l, c);
            // the time the jams ahead cost
            if let Some(lane) = net.lanes.get(l) {
                if c > 0.6 {
                    let v_free = lane.speed_limit_kmh.clamp(20.0, 70.0) / 3.6;
                    let v = (v_free * (1.0 - c)).max(1.2);
                    cost += lane.length() / v - lane.length() / v_free;
                }
            }
        }
        self.jam_cost = cost;
        let changed = route_jam.len() != self.route_jam.len() || route_jam.iter().any(|(l, c)| self.route_jam.get(l).map(|o| level(*o) != level(*c)).unwrap_or(true));
        self.route_jam = route_jam;
        if changed {
            self.jam_version += 1;
        }
    }

    /// The first real turn on the route within 1.5 km: a lane whose heading changes by more
    /// than 35 degrees from its start to its end (a junction's curve), or a sharp bend
    /// between two lanes. Gentle curves of a road are not turns.
    fn turn_ahead(&self, net: &Network) -> Option<(i32, f32, f64, Option<String>)> {
        let r = &self.route;
        if !r.on_route {
            return None;
        }
        let mut acc = -(r.s as f64);
        let mut prev_end: Option<f32> = None;
        for (j, &l) in r.lanes.iter().enumerate().skip(r.progress) {
            let lane = net.lanes.get(l)?;
            let len = lane.length();
            if acc > 1500.0 {
                break;
            }
            let (h0, h1) = (lane.start_heading(), lane.end_heading());
            let mut d = omsi_sim::traffic::wrap_deg(h1 - h0);
            if let Some(pe) = prev_end {
                d += omsi_sim::traffic::wrap_deg(h0 - pe);
            }
            // (a short junction lane turns within a few metres; a long road curve does not count)
            if d.abs() > 35.0 && (len < 60.0 || d.abs() > 70.0) && acc + len as f64 > 0.0 {
                let dir = if d.abs() > 150.0 { 2 } else if d > 0.0 { 1 } else { -1 };
                // the street it turns into: the first named lane after the turn
                let street = r.lanes.iter().skip(j + 1).take(4).chain(std::iter::once(&l)).find_map(|&x| self.street_of(x)).map(str::to_string);
                return Some((dir, d.abs(), acc.max(0.0), street));
            }
            prev_end = Some(h1);
            acc += len as f64;
        }
        None
    }

    /// The street a lane of the route's network belongs to (the map's network only).
    fn street_of(&self, lane: usize) -> Option<&str> {
        self.global.as_ref()?;
        let st = self.streets.as_deref()?;
        let i = *st.of_lane.get(lane)?;
        st.names.get(i as usize).map(String::as_str)
    }

    /// Distance along the route to `stop` (m).
    fn route_distance(&self, net: &Network, stop: DVec3) -> Option<f64> {
        let r = &self.route;
        let mut acc = -(r.s as f64);
        for &l in r.lanes.iter().skip(r.progress) {
            let lane = net.lanes.get(l)?;
            if let Some((s, d)) = lane.nearest_point(stop) {
                if d < 25.0 && (acc + s as f64) >= -5.0 {
                    return Some((acc + s as f64).max(0.0));
                }
            }
            acc += lane.length() as f64;
            if acc > 30_000.0 {
                break;
            }
        }
        None
    }

    /// Advance, draw into the texture and put it on the screen.
    pub fn frame_at(
        &mut self,
        renderer: &Renderer,
        scene: &mut Scene,
        f: &NavFrame,
        origin_x: f32,
    ) {
        for rect in [&mut self.panel_rect, &mut self.city.rect] {
            rect[0] -= self.origin_x;
            rect[2] -= self.origin_x;
        }
        if self.origin_x != origin_x {
            self.panel_drag = None;
            self.city.drag = None;
        }
        let start = scene.overlays.len();
        self.frame(renderer, scene, f);
        crate::ui::shift_overlays(scene, start, origin_x);
        for rect in [&mut self.panel_rect, &mut self.city.rect] {
            rect[0] += origin_x;
            rect[2] += origin_x;
        }
        self.origin_x = origin_x;
    }

    pub fn frame(&mut self, renderer: &Renderer, scene: &mut Scene, f: &NavFrame) {
        self.panel_overlay = None;
        // (with the navigator off the route is still followed for OMSI 2's arrows)
        if !self.enabled && !self.city.open && !self.arrows && self.shown < 0.01 {
            return;
        }
        let target = if self.enabled { 1.0 } else { 0.0 };
        self.shown += (target - self.shown) * (1.0 - (-f.dt * 8.0).exp());
        if (self.shown - target).abs() < 0.005 {
            self.shown = target;
        }
        self.time += f.dt;
        if let Some(rx) = self.building.as_ref() {
            if let Ok((net, pos, streets)) = rx.try_recv() {
                log::info!("navigator: the map's road network is there ({} lanes, {} streets named)", net.lanes.len(), streets.names.len());
                self.streets = Some(std::sync::Arc::new(streets));
                self.global = Some(std::sync::Arc::new(net));
                self.stop_pos = std::sync::Arc::new(pos);
                self.global_version += 1;
                self.building = None;
                // routes and roads again on the whole map
                self.roads = None;
                self.route = Route { version: self.route.version + 1, ..Route::default() };
                self.route_mesh.0 = u64::MAX;
                self.route_jam.clear();
            }
        }
        // stops whose tiles are not loaded: their place from the map
        if omsi_cfg::flags::OMSI_DEBUG_NAV.is_set() && self.time < 0.15 {
            log::info!("navigator: stops {:?}", f.stops.iter().map(|s| (s.name.clone(), s.object_id, s.position != DVec3::ZERO, self.stop_pos.contains_key(&s.object_id))).collect::<Vec<_>>());
        }
        let f2;
        let f = if f.stops.iter().any(|s| s.position == DVec3::ZERO) {
            f2 = NavFrame { stops: f.stops.iter().filter_map(|s| if s.position == DVec3::ZERO { self.stop_pos.get(&s.object_id).map(|p| NavStop { position: *p, ..s.clone() }) } else { Some(s.clone()) }).collect(), ..f.clone_ref() };
            &f2
        } else {
            f
        };
        self.follow(f);
        self.bus_at = f.bus;
        self.stop_spots = f.stops.iter().take(3).map(|st| (st.position, st.name.clone(), f.heading, st.object_id)).collect();
        if omsi_cfg::flags::OMSI_DEBUG_NAV.is_set() && (self.time % 1.0) < f.dt {
            log::info!("navigator: route {} lanes (complete {}, provisional {}, at {}, on it {}, off for {:.1} s), {} stops ahead, next {:?}, key {:?}", self.route.lanes.len(), self.route.complete, self.route.provisional, self.route.progress, self.route.on_route, self.route.off_for, f.stops.len(), f.stops.first().map(|s| (s.name.clone(), s.position.x.round(), s.position.y.round())), self.route.key);
        }
        self.update_congestion(f);
        // camera: zoomed out with speed, turned with the bus (both eased)
        let want = (110.0 + f.speed_kmh as f64 * 2.2).clamp(110.0, 280.0);
        if self.first {
            self.zoom = want;
            self.cam_heading = f.heading;
        }
        self.zoom += (want - self.zoom) * ease(f.dt, 1.8);
        self.cam_heading += angle_diff(self.cam_heading, f.heading) * ease(f.dt, 0.3);
        let v = f.speed_kmh.abs() / 3.6;
        self.speed_avg += (v - self.speed_avg) * ease(f.dt, 20.0) as f32;
        self.dist_t -= f.dt;
        if self.dist_t <= 0.0 || self.first {
            self.dist_t = 0.4;
            let global = self.global.clone();
            let net = global.as_deref().or(f.traffic.map(|t| &t.net));
            self.next_dist = match (f.stops.first(), net) {
                (Some(s), Some(n)) if !self.route.lanes.is_empty() => self.route_distance(n, s.position).or_else(|| Some((s.position - f.bus).truncate().length())),
                (Some(s), _) => Some((s.position - f.bus).truncate().length()),
                _ => None,
            };
            self.next_turn = net.and_then(|n| self.turn_ahead(n));
            self.street_here = if self.route.on_route {
                self.route.lanes.get(self.route.progress).and_then(|&l| self.street_of(l))
            } else {
                global.as_deref().and_then(|g| g.nearest_lane_near(f.bus, LaneKind::Street)).filter(|l| l.2 < 10.0).and_then(|l| self.street_of(l.0))
            }
                .map(str::to_string);
        }
        self.first = false;
        if !self.enabled && self.shown < 0.01 {
            self.panel_rect = [0.0; 4];
            // (followed for the route arrows alone, with the city map shut: nothing to draw)
            if self.city.open {
                self.city(renderer, scene, f);
            }
            return;
        }

        // --- size and place on the screen: small, a corner of its own
        let (sw, sh) = f.screen;
        // (a third of the window's height however tall it is - held to 480 px, it was a
        // sixth of a 4K screen's - but 300 px at least, where its smallest texts were 8 px
        // high on a 720p window; made larger, still short enough to fit the window with its
        // schedule)
        let base = (sh * 0.33).max(300.0);
        let base = if f.follow_window { base } else { base.min(480.0) };
        let pw = (base * f.ui_scale).min((sh * 0.7).max(300.0)).round();
        let map_h = (pw * 0.62).round();
        let s = pw / 360.0;
        let bars = (34.0 + 46.0) * s;
        let sched = if self.schedule { (f.stops.len().clamp(1, 5) as f32 * 22.0 + 12.0) * s } else { 0.0 };
        let ph = (map_h + bars + sched).round();
        let (w, h) = (pw as u32, ph as u32);
        let (x0, y0) = self.panel_origin((sw, sh), (pw, ph), crate::platform::touch_controls(), f.info_rect);

        if self.gpu.is_none() {
            self.gpu = Some(Gpu::new(&renderer.device, renderer.format(), map_samples(renderer.format()), self.atlas.size));
        }
        let resized = self.target.map(|t| (t.1, t.2) != (w, h)).unwrap_or(true);
        if resized {
            if let Some((t, _, _)) = self.target.take() {
                renderer.free_texture(scene, t);
                scene.premultiplied.remove(&t);
            }
            let t = renderer.add_render_texture(scene, w, h);
            scene.premultiplied.insert(t);
            self.target = Some((t, w, h));
        }
        let (tex, _, _) = self.target.unwrap();
        let Some(view) = renderer.texture_view(scene, tex) else { return };
        if !self.city.open && (resized || self.time - self.drawn_at >= NAV_REDRAW_S) {
            self.drawn_at = self.time;
            self.draw(renderer, &view, (w, h), map_h, f);
        }
        // (the small navigator steps aside while the city map is open)
        if !self.city.open {
            self.panel_overlay = Some(scene.overlays.len());
            scene.overlays.push((tex, [x0, y0, x0 + pw, y0 + ph]));
        }
        self.panel_rect = [x0, y0, x0 + pw, y0 + ph];
        if self.city.open {
            self.city(renderer, scene, f);
        }
    }

    fn draw(&mut self, renderer: &Renderer, target: &wgpu::TextureView, size: (u32, u32), map_h: f32, f: &NavFrame) {
        let (pw, ph) = (size.0 as f32, size.1 as f32);
        let s = pw / 360.0;
        let wd = words(f.language);
        self.atlas.begin_frame();
        let panel = Rect::new(0.0, 0.0, pw, ph);
        let radius = 7.0 * s;
        let top_h = (34.0 * s).round();
        let map = Rect::new(0.0, top_h, pw, map_h);
        let vp = [map.x, map.y, map.w, map.h];

        // --- roads (buffer 0): rebuilt when the bus went far or tiles brought lanes
        let own = self.own_net.clone();
        let global = self.global.clone();
        let net = global.as_deref().or(f.traffic.map(|t| &t.net)).or(own.as_deref());
        let lanes_now = net.map(|n| n.lanes.len()).unwrap_or(0);
        let rebuild = match &self.roads {
            None => lanes_now > 0,
            Some(r) => (r.anchor - f.bus.truncate()).length() > ROAD_RADIUS * 0.45 || (r.lanes_seen != lanes_now && self.time - r.built_at > 1.5),
        };
        let mut road_verts = None;
        if rebuild {
            if let Some(net) = net {
                let anchor = f.bus.truncate();
                let mut p = Painter::new();
                let t0 = std::time::Instant::now();
                build_roads(&mut p, net, anchor);
                if omsi_cfg::flags::OMSI_DEBUG_NAV.is_set() {
                    log::info!("navigator: roads around ({:.0}, {:.0}) in {:.1} ms: {} vertices", anchor.x, anchor.y, t0.elapsed().as_secs_f64() * 1000.0, p.verts.len());
                }
                road_verts = Some(p.verts);
                self.roads = Some(Roads { anchor, lanes_seen: lanes_now, verts: 0, built_at: self.time });
            }
        }

        // --- the map camera (world coordinates relative to the anchor)
        let anchor = self.roads.as_ref().map(|r| r.anchor).unwrap_or(f.bus.truncate());
        let rel = |p: DVec3| Vec3::new((p.x - anchor.x) as f32, (p.y - anchor.y) as f32, 0.0);
        let hd = self.cam_heading.to_radians();
        let fwd = DVec2::new(hd.sin(), hd.cos());
        let look_at = f.bus.truncate() - anchor + fwd * self.zoom * 0.28;
        let pitch = PITCH.to_radians();
        let back = fwd * self.zoom * pitch.cos();
        let eye = DVec3::new(look_at.x - back.x, look_at.y - back.y, self.zoom * pitch.sin());
        let view = Mat4::look_at_rh(eye.as_vec3(), Vec3::new(look_at.x as f32, look_at.y as f32, 0.0), Vec3::Z);
        let clip_panel = [0.0, 0.0, pw, ph];
        let map_layer = Layer::world(view, FOV.to_radians(), vp, [map.x, map.y, map.right(), map.bottom()], 0.0, 1.0);
        let vpm = map_layer.view_proj;

        // --- the route (buffer 1): rebuilt when it or the lane the bus is on changed
        let mut route_verts = None;
        let route_net = global.as_deref().or(f.traffic.map(|t| &t.net));
        if let (Some(rn), true) = (route_net, !self.route.lanes.is_empty()) {
            let first = self.route.lanes.get(self.route.progress.min(self.route.lanes.len().saturating_sub(1))).copied();
            let bus_lane = first.map(|l| lane_from_right(rn, l, f.bus)).unwrap_or(0);
            if self.route_mesh.0 != self.route.version || self.route_mesh.1 != self.jam_version || self.route_mesh.2 != anchor || self.route_mesh.4 != bus_lane {
                let mut p = Painter::new();
                // (one lane wide: a little narrower than the lane, so that it never spills onto the next)
                let style = RouteStyle { extra_m: -0.9, min_px: 4.0, arrows: Some((28.0, 900.0)), max_len: 12_000.0, near: Some((anchor, ROAD_RADIUS * 1.6)) };
                build_route(&mut p, rn, &self.route.lanes[self.route.progress.min(self.route.lanes.len())..], anchor, self.route.s, &self.route_jam, &style, bus_lane);
                self.route_mesh = (self.route.version, self.jam_version, anchor, p.len(), bus_lane);
                route_verts = Some(p.verts);
            }
        } else if self.route_mesh.3 != 0 {
            route_verts = Some(Vec::new());
            self.route_mesh = (self.route.version, self.jam_version, anchor, 0, 0);
        }

        // --- background (half transparent), traffic, markers, text
        let mut bg = Painter::new();
        bg.rounded(panel, radius, if self.cockpit_display { Color::rgba(10, 10, 10, 1.0) } else { PANEL });
        let n_bg = bg.len();

        let mut dy = Painter::new();
        // other roads where the traffic is heavy or stands
        if let Some(net) = f.traffic.map(|t| &t.net) {
            for (&lane, &c) in &self.congestion {
                let lv = level(c);
                if lv < 3 {
                    continue;
                }
                let Some(l) = net.lanes.get(lane) else { continue };
                if (l.start() - f.bus).truncate().length() > 900.0 {
                    continue;
                }
                let pts: Vec<Vec3> = l.points.iter().map(|p| rel(*p)).collect();
                dy.ribbon(&pts, l.width.max(2.5) * 0.6, 2.0, LEVEL[lv].alpha(0.8), true);
            }
        }
        let n_traffic = dy.len();
        // the other vehicles: blue dots, as the other drivers in ETS2; the public transport in
        // its own colours (trolleybus, bus, tram), a little bigger, its line beside it
        let mut lines: Vec<(DVec3, Color, String)> = Vec::new();
        if let Some(t) = f.traffic.filter(|_| self.show_ai) {
            for c in &t.cars {
                if c.gone || (c.vehicle.position - f.bus).truncate().length() > self.zoom * 3.5 + 150.0 {
                    continue;
                }
                let (color, line) = traffic_kind(c);
                if color == DOT {
                    dy.world_disc(rel(c.vehicle.position), 1.7, 3.6, Color::rgba(8, 8, 8, 0.9));
                    dy.world_disc(rel(c.vehicle.position), 1.2, 2.6, color);
                } else {
                    vehicle_icon(&mut dy, rel(c.vehicle.position), c.vehicle.heading, color, c.is_rail(), 7.0);
                }
                if let Some(l) = line {
                    lines.push((c.vehicle.position, color, l));
                }
            }
        }
        let n_world = dy.len();

        let mut ui = Painter::new();
        // the far end of the map fades into the panel
        ui.gradient(Rect::new(map.x, map.y, map.w, map.h * 0.3), Color::rgba(10, 10, 10, 0.75), Color::rgba(10, 10, 10, 0.0));
        // the next turn: an arrow and how far, top left of the map
        if let Some((dir, angle, dist, street)) = self.next_turn.as_ref() {
            let icon = match *dir {
                2 => "u_turn_left",
                -1 if *angle < 60.0 => "turn_slight_left",
                1 if *angle < 60.0 => "turn_slight_right",
                -1 => "turn_left",
                _ => "turn_right",
            };
            let t = if *dist >= 1000.0 { format!("{:.1} km", dist / 1000.0) } else { format!("{:.0} m", ((dist / 10.0).round() * 10.0).max(10.0)) };
            let tw = self.fonts.width(&t, 14.0 * s, Weight::Bold);
            // with the street it turns into, when the map names it
            let street = street.as_deref().map(|n| self.fonts.fit(n, 12.0 * s, Weight::Medium, map.w * 0.62 - 50.0 * s - tw));
            let sw_ = street.as_deref().map(|n| self.fonts.width(n, 12.0 * s, Weight::Medium) + 10.0 * s).unwrap_or(0.0);
            let b = Rect::new(map.x + 8.0 * s, map.y + 8.0 * s, 44.0 * s + tw + sw_, 34.0 * s);
            ui.rounded(b, 5.0 * s, Color::rgba(10, 10, 10, 0.85));
            ui.icon(&mut self.atlas, icon, Vec2::new(b.x + 18.0 * s, b.center().y), 24.0 * s, TEXT);
            ui.text_in(&mut self.atlas, &self.fonts, &t, 14.0 * s, Weight::Bold, Rect::new(b.x + 34.0 * s, b.y, tw + 4.0, b.h), Align::Left, TEXT);
            if let Some(n) = street.as_deref() {
                ui.text_in(&mut self.atlas, &self.fonts, n, 12.0 * s, Weight::Medium, Rect::new(b.x + 42.0 * s + tw, b.y, sw_, b.h), Align::Left, TEXT_DIM);
            }
        }
        // the street the bus is on, bottom middle of the map
        if let Some(n) = self.street_here.as_deref() {
            let px = 11.5 * s;
            let n = self.fonts.fit(n, px, Weight::Medium, map.w * 0.7);
            let w = self.fonts.width(&n, px, Weight::Medium) + 14.0 * s;
            let r = Rect::new(map.center().x - w * 0.5, map.bottom() - 24.0 * s, w, 18.0 * s);
            ui.rounded(r, 9.0 * s, Color::rgba(10, 10, 10, 0.8));
            ui.text_in(&mut self.atlas, &self.fonts, &n, px, Weight::Medium, r, Align::Center, STREET);
        }
        // Stops ahead use a bus badge so they read as stops, not generic route dots.
        let n_stops = f.stops.len();
        let markers = spaced_markers(f.stops.iter().enumerate().filter_map(|(k, st)| project(vpm, vp, rel(st.position)).filter(|p| map.contains(*p)).map(|p| (k, p))), 20.0 * s);
        for (k, sp) in markers.into_iter().rev() {
            if k + 1 == n_stops {
                ui.icon(&mut self.atlas, "sports_score", sp - Vec2::new(0.0, 12.0 * s), 14.0 * s, TEXT);
            }
            let next = k == 0;
            let badge = if next { 8.0 } else { 6.5 } * s;
            ui.circle(sp, badge + 1.5 * s, Color::rgba(10, 10, 10, 0.95));
            let fill = if next { Color::rgba(45, 116, 205, 1.0) } else if k + 1 == n_stops { ROUTE } else { Color::rgba(76, 91, 112, 0.98) };
            ui.circle(sp, badge, fill);
            ui.icon(&mut self.atlas, "directions_bus", sp, if next { 12.5 } else { 10.5 } * s, TEXT);
        }
        // the public transport's lines: a tag in its colour above its dot (the nearest first,
        // none on top of another)
        lines.sort_by(|a, b| (a.0 - f.bus).length().total_cmp(&(b.0 - f.bus).length()));
        let mut taken: Vec<Rect> = Vec::new();
        // the other players: an arrow each, their name above it (before the lines' tags)
        for pl in &f.players {
            let Some(p) = project(vpm, vp, rel(pl.position)).filter(|p| map.contains(*p)) else { continue };
            let a = (angle_diff(self.cam_heading, pl.heading) as f32).to_radians();
            arrow(&mut ui, p, a, 7.5 * s, 1.25, Color::rgba(10, 10, 10, 0.8), PLAYER);
            let px = 9.5 * s;
            let name = self.fonts.fit(&pl.name, px, Weight::Bold, 90.0 * s);
            let w = self.fonts.width(&name, px, Weight::Bold) + 8.0 * s;
            let r = Rect::new(p.x - w * 0.5, p.y - 23.0 * s, w, 13.0 * s);
            if taken.iter().any(|o| rects_overlap(o, &r)) {
                continue;
            }
            taken.push(r);
            ui.rounded(Rect::new(r.x - 1.0 * s, r.y - 1.0 * s, r.w + 2.0 * s, r.h + 2.0 * s), 4.0 * s, Color::rgba(10, 10, 10, 0.9));
            ui.rounded(r, 3.5 * s, PLAYER);
            ui.text_in(&mut self.atlas, &self.fonts, &name, px, Weight::Bold, r, Align::Center, LINE_TEXT);
        }
        for (pos, color, l) in &lines {
            let Some(p) = project(vpm, vp, rel(*pos)).filter(|p| map.contains(*p)) else { continue };
            let px = 9.5 * s;
            let l = self.fonts.fit(l, px, Weight::Bold, 40.0 * s);
            let w = self.fonts.width(&l, px, Weight::Bold) + 8.0 * s;
            let r = Rect::new(p.x - w * 0.5, p.y - 19.0 * s, w, 13.0 * s);
            if taken.iter().any(|o| o.x < r.right() && r.x < o.right() && o.y < r.bottom() && r.y < o.bottom()) {
                continue;
            }
            taken.push(r);
            ui.rounded(Rect::new(r.x - 1.0 * s, r.y - 1.0 * s, r.w + 2.0 * s, r.h + 2.0 * s), 4.0 * s, Color::rgba(10, 10, 10, 0.9));
            ui.rounded(r, 3.5 * s, *color);
            ui.text_in(&mut self.atlas, &self.fonts, &l, px, Weight::Bold, r, Align::Center, LINE_TEXT);
        }
        // the bus: a plain white arrow
        if let Some(bp) = project(vpm, vp, rel(f.bus)) {
            let a = (angle_diff(self.cam_heading, f.heading) as f32).to_radians();
            arrow(&mut ui, bp, a, 9.0 * s, 1.25, Color::rgba(10, 10, 10, 0.8), TEXT);
        }

        // top bar: speed (and the limit) · line ……… game time
        let top = Rect::new(0.0, 0.0, pw, top_h);
        ui.rect(top, BAR);
        let pad = 11.0 * s;
        let base = top.y + top.h * 0.5 + self.fonts.cap_height(17.0 * s, Weight::Bold) * 0.5;
        let mut x = pad;
        x += ui.text(&mut self.atlas, &self.fonts, &format!("{:.0}", f.speed_kmh.abs()), 17.0 * s, Weight::Bold, Vec2::new(x, base), Align::Left, TEXT);
        x += 4.0 * s;
        x += ui.text(&mut self.atlas, &self.fonts, wd.kmh, 12.0 * s, Weight::Medium, Vec2::new(x, base), Align::Left, TEXT_DIM);
        let limit = net.and_then(|n| {
            let lane = if self.route.on_route { self.route.lanes.get(self.route.progress).copied() } else { None };
            let lane = lane.or_else(|| n.nearest_lane_near(f.bus, LaneKind::Street).filter(|l| l.2 < 8.0).map(|l| l.0))?;
            let v = n.lanes.get(lane)?.speed_limit_kmh;
            (v > 1.0 && v < 200.0).then_some(v)
        });
        if let Some(v) = limit {
            x += 10.0 * s;
            let c = Vec2::new(x + 10.0 * s, top.center().y);
            ui.circle(c, 10.5 * s, Color::rgba(200, 40, 40, 1.0));
            ui.circle(c, 8.3 * s, Color::rgba(235, 235, 235, 1.0));
            let t = format!("{:.0}", (v / 5.0).round() * 5.0);
            let px = if t.len() > 2 { 7.5 } else { 9.0 } * s;
            ui.text(&mut self.atlas, &self.fonts, &t, px, Weight::Black, Vec2::new(c.x, c.y + self.fonts.cap_height(px, Weight::Black) * 0.5), Align::Center, Color::rgba(15, 15, 15, 1.0));
        }
        let hh = (f.time / 3600.0) as i32 % 24;
        let mm = ((f.time % 3600.0) / 60.0) as i32;
        let time_text = format!("{hh:02}:{mm:02}");
        let day_text = wd.days[f.weekday.clamp(0, 6) as usize];
        let time_w = self.fonts.width(&time_text, 14.0 * s, Weight::Bold);
        let day_w = self.fonts.width(day_text, 12.0 * s, Weight::Medium);
        ui.text(&mut self.atlas, &self.fonts, &time_text, 14.0 * s, Weight::Bold, Vec2::new(pw - pad, base), Align::Right, TEXT);
        ui.text(&mut self.atlas, &self.fonts, day_text, 12.0 * s, Weight::Medium, Vec2::new(pw - pad - time_w - 5.0 * s, base), Align::Right, TEXT_DIM);

        // Temperatures stay in the navigator header on every bus. The simulator always keeps
        // Cabinair_Temp, while scripts that model heating/air conditioning can overwrite it.
        let temp = format!("EXT {:.0}°C · INT {:.0}°C", f.outside_temp, f.inside_temp);
        let center = match f.line.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
            Some(line) => format!("{temp} · {line}"),
            None => temp,
        };
        let left_edge = (if limit.is_some() { x + 24.0 * s } else { x }) + 6.0 * s;
        let right_edge = pw - pad - time_w - 5.0 * s - day_w - 6.0 * s;
        if right_edge > left_edge {
            let rect = Rect::new(left_edge, top.y, right_edge - left_edge, top.h);
            let center = self.fonts.fit(&center, 11.5 * s, Weight::Bold, rect.w);
            ui.text_in(&mut self.atlas, &self.fonts, &center, 11.5 * s, Weight::Bold, rect, Align::Center, TEXT_DIM);
        }

        // bottom bar: the next stop; its distance, the time to it, the planned time and
        // whether the bus is early or late
        let bottom = Rect::new(0.0, map.bottom(), pw, 46.0 * s);
        ui.rect(bottom, BAR);
        let stop_row = if f.stops.is_empty() {
            Rect::new(pad, bottom.y, pw - 2.0 * pad, bottom.h)
        } else {
            Rect::new(pad, bottom.y + 4.0 * s, pw - 2.0 * pad, 20.0 * s)
        };
        let stop_row = stop_request_icon(&mut ui, &mut self.atlas, f.stop_requested, stop_row, s);
        let note = if self.route.note > 0.0 {
            Some((wd.recalculated, ON_TIME))
        } else if self.route.joined && !self.route.lanes.is_empty() && !self.route.on_route && self.route.off_for > OFF_ROUTE_AFTER {
            // (no way back found after a while: it says so instead of recalculating for ever)
            Some((if self.route.off_for < OFF_ROUTE_AFTER + 20.0 { wd.rerouting } else { wd.off_route }, WARN))
        } else {
            None
        };
        // what the traffic on the route ahead costs, as the module promises: a jam from a
        // minute on, slow traffic from half of one
        let jam_note = (self.jam_cost >= 30.0).then(|| {
            let what = if self.jam_cost >= 60.0 { wd.jam } else { wd.slow };
            (format!("{what} +{:.0} min", (self.jam_cost / 60.0).max(1.0).round()), if self.jam_cost >= 60.0 { LATE } else { WARN })
        });
        match f.stops.first() {
            Some(st) => {
                let name = if n_stops == 1 { format!("{} · {}", st.name.trim(), wd.last_stop) } else { st.name.trim().to_string() };
                ui.text_in(&mut self.atlas, &self.fonts, &name, 13.5 * s, Weight::Bold, stop_row, Align::Left, TEXT);
                let mut parts = Vec::new();
                if let Some(d) = self.next_dist {
                    parts.push(if d >= 1000.0 { format!("{:.1} km", d / 1000.0) } else { format!("{:.0} m", (d / 10.0).round() * 10.0) });
                    let secs = d / (self.speed_avg.max(5.0) as f64);
                    parts.push(if secs < 60.0 { "<1 min".to_string() } else { format!("{:.0} min", (secs / 60.0).round()) });
                }
                parts.push(format!("{:02}:{:02}", (st.arrival / 3600.0) as i32 % 24, ((st.arrival % 3600.0) / 60.0) as i32));
                let line2 = parts.join("  ·  ");
                let y2 = Rect::new(pad, bottom.y + 24.0 * s, pw - 2.0 * pad, 18.0 * s);
                match note {
                    Some((t, c)) => {
                        ui.text_in(&mut self.atlas, &self.fonts, t, 12.5 * s, Weight::Medium, y2, Align::Left, c);
                    }
                    None => match &jam_note {
                        Some((t, c)) => {
                            ui.text_in(&mut self.atlas, &self.fonts, &format!("{line2}  ·  {t}"), 12.5 * s, Weight::Medium, y2, Align::Left, *c);
                        }
                        None => {
                            ui.text_in(&mut self.atlas, &self.fonts, &line2, 12.5 * s, Weight::Medium, y2, Align::Left, TEXT_DIM);
                        }
                    },
                }
                if let Some(d) = f.delay {
                    let (txt, c) = if d > 59.0 {
                        (format!("+{}:{:02}", (d / 60.0) as i32, (d % 60.0) as i32), LATE)
                    } else if d < -59.0 {
                        (format!("−{}:{:02}", (-d / 60.0) as i32, (-d % 60.0) as i32), EARLY)
                    } else {
                        (wd.on_time.to_string(), ON_TIME)
                    };
                    ui.text_in(&mut self.atlas, &self.fonts, &txt, 12.5 * s, Weight::Bold, y2, Align::Right, c);
                }
            }
            None => {
                let t = f.terminus.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| wd.no_duty.to_string());
                ui.text_in(&mut self.atlas, &self.fonts, &t, 13.0 * s, Weight::Medium, stop_row, Align::Left, TEXT_DIM);
            }
        }
        // the schedule: the next stops with their planned times
        if self.schedule && !f.stops.is_empty() {
            let mut y = bottom.bottom() + 6.0 * s;
            ui.rect(Rect::new(pad, bottom.bottom(), pw - 2.0 * pad, 1.0), Color::WHITE.alpha(0.06));
            let late = f.delay.unwrap_or(0.0);
            for st in f.stops.iter().take(5) {
                let r = Rect::new(pad, y, pw - 2.0 * pad, 22.0 * s);
                let planned = format!("{:02}:{:02}", (st.arrival / 3600.0) as i32 % 24, ((st.arrival % 3600.0) / 60.0) as i32);
                ui.text_in(&mut self.atlas, &self.fonts, &planned, 12.5 * s, Weight::Bold, r, Align::Left, TEXT_DIM);
                ui.text_in(&mut self.atlas, &self.fonts, st.name.trim(), 13.0 * s, Weight::Medium, Rect::new(r.x + 46.0 * s, r.y, r.w - 100.0 * s, r.h), Align::Left, TEXT);
                // when the bus will be there at this lateness
                let exp = st.arrival + late;
                let e = format!("{:02}:{:02}", (exp / 3600.0).rem_euclid(24.0) as i32, ((exp.rem_euclid(3600.0)) / 60.0) as i32);
                ui.text_in(&mut self.atlas, &self.fonts, &e, 12.5 * s, Weight::Medium, r, Align::Right, if late > 59.0 { LATE } else if late < -59.0 { EARLY } else { TEXT_DIM });
                y += 22.0 * s;
            }
        }

        // --- to the GPU
        let (Some(gpu), device, queue) = (self.gpu.as_mut(), &renderer.device, &renderer.queue) else { return };
        if let Some(v) = road_verts {
            if let Some(r) = self.roads.as_mut() {
                r.verts = v.len();
            }
            gpu.upload(device, queue, 0, &v);
        }
        if let Some(v) = route_verts {
            gpu.upload(device, queue, 1, &v);
        }
        let mut all = bg.verts;
        all.extend(dy.verts);
        let n_ui_start = all.len() as u32;
        all.extend(ui.verts);
        gpu.upload(device, queue, 2, &all);
        gpu.upload_atlas(queue, &mut self.atlas);
        // (the opacity setting is the background's: the map and the text stay solid)
        let flat = Layer::flat(clip_panel, radius, 1.0);
        let backdrop = Layer::flat(clip_panel, radius, if self.cockpit_display { self.opacity } else { crate::ui::backdrop(self.opacity).min(1.0) });
        let mut layers = [flat, map_layer, backdrop];
        for l in layers.iter_mut() {
            l.opacity *= self.shown;
        }
        let roads_n = self.roads.as_ref().map(|r| r.verts as u32).unwrap_or(0);
        let draws = [
            Draw { buffer: 2, range: 0..n_bg, layer: 2, texture: 0 },
            Draw { buffer: 0, range: 0..roads_n, layer: 1, texture: 0 },
            Draw { buffer: 2, range: n_bg..n_bg + n_traffic, layer: 1, texture: 0 },
            Draw { buffer: 1, range: 0..self.route_mesh.3, layer: 1, texture: 0 },
            Draw { buffer: 2, range: n_bg + n_traffic..n_bg + n_world, layer: 1, texture: 0 },
            Draw { buffer: 2, range: n_ui_start..all.len() as u32, layer: 0, texture: 0 },
        ];
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("navigator") });
        gpu.render(device, queue, &mut enc, target, size, Some(wgpu::Color::TRANSPARENT), &layers, &draws);
        queue.submit([enc.finish()]);
    }
}

/// Congestion of the traffic's lanes carried over to the same paths of `net`.
fn congestion_on(net: &Network, traffic: &Network, c: &HashMap<usize, f32>) -> HashMap<usize, f32> {
    let mut out = HashMap::new();
    for (&l, &v) in c {
        let Some(lane) = traffic.lanes.get(l) else { continue };
        let Some(key) = lane.key else { continue };
        if let Some(cands) = net.by_key.get(&key) {
            for &g in cands {
                if net.lanes[g].reversed == lane.reversed {
                    out.insert(g, v);
                }
            }
        }
    }
    out
}

impl<'a> NavFrame<'a> {
    fn clone_ref(&self) -> NavFrame<'a> {
        NavFrame { traffic: self.traffic, players: self.players.clone(), bus: self.bus, heading: self.heading, speed_kmh: self.speed_kmh, outside_temp: self.outside_temp, inside_temp: self.inside_temp, line: self.line.clone(), terminus: self.terminus.clone(), stops: self.stops.clone(), delay: self.delay, passengers: self.passengers, time: self.time, weekday: self.weekday, language: self.language, screen: self.screen, ui_scale: self.ui_scale, follow_window: self.follow_window, dt: self.dt, stop_requested: self.stop_requested, info_rect: self.info_rect }
    }
}

/// Street lanes that stand for drawn roads, including editor-only paths corroborated by
/// road surfaces. Uncorroborated helpers must not make a phantom road on the GPS.
fn visible_road_lanes(net: &Network) -> Vec<(usize, &omsi_sim::traffic::Lane)> {
    let mut seen = hashbrown::HashSet::<(LaneKey, u32)>::new();
    net.lanes
        .iter()
        .enumerate()
        .filter(|(_, l)| l.kind == LaneKind::Street && !l.invisible && l.points.len() >= 2)
        .filter(|(_, l)| {
            if let Some(key) = l.key {
                // A two-way [path] has the same geometry in both directions. Keep one
                // casing and surface; retain separate [path]s, which may be real lanes.
                return seen.insert((key, l.source));
            }
            true
        })
        .collect()
}

/// The roads a city map draws, from a whole map's lanes as they were read off the tiles:
/// every path linked to its neighbours, the editor-only ones confirmed by the map's own
/// asphalt, then grouped into carriageways with their real widths. The navigator's city map
/// and the launcher's map picture draw the same roads this way, so the two cannot drift
/// apart; the second value is the linked network the caller may walk (a trip's own lanes).
pub(crate) fn city_roads(lanes: Vec<omsi_sim::traffic::Lane>, surfaces: &[(Vec<DVec3>, f32)]) -> (Vec<MapRoad>, Network) {
    let mut net = Network { lanes, ..Default::default() };
    net.link(1.5);
    confirm_road_surfaces(&mut net, surfaces);
    let roads = road_geometry(&net);
    (roads, net)
}

/// A road of the city map: the centre line of a carriageway, as wide as the map makes it.
pub(crate) struct MapRoad {
    pub points: Vec<DVec3>,
    pub width: f32,
    /// A road cars drive far along (the map's own speed limits say so): drawn brighter.
    pub main: bool,
}

/// Some maps separate the asphalt mesh from their editor-only traffic splines. Use the
/// actual road footprint to distinguish those paths from invisible scenery helpers. This
/// changes only the navigator's copy of the network; driving still uses the original data.
fn confirm_road_surfaces(net: &mut Network, surfaces: &[(Vec<DVec3>, f32)]) {
    let mut segments = Vec::new();
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (pts, width) in surfaces {
        for ab in pts.windows(2) {
            let (a, b) = (ab[0], ab[1]);
            let margin = *width as f64 * 0.5 + 0.5;
            let lo = a.truncate().min(b.truncate()) - DVec2::splat(margin);
            let hi = a.truncate().max(b.truncate()) + DVec2::splat(margin);
            let (x0, y0) = Network::grid_cell(lo.extend(0.0));
            let (x1, y1) = Network::grid_cell(hi.extend(0.0));
            let i = segments.len();
            segments.push((a, b, margin));
            for x in x0..=x1 { for y in y0..=y1 { grid.entry((x, y)).or_default().push(i); } }
        }
    }
    for lane in net.lanes.iter_mut().filter(|l| l.invisible && l.kind == LaneKind::Street) {
        let n = (lane.length() / 8.0).ceil().max(1.0) as usize;
        let mut covered = 0;
        for k in 0..n {
            let (p, _) = lane.at(lane.length() * (k as f32 + 0.5) / n as f32);
            let on_surface = grid.get(&Network::grid_cell(p)).map(|ids| ids.iter().any(|&i| {
                let (a, b, margin) = segments[i];
                let ab = (b - a).truncate();
                let t = ((p - a).truncate().dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                let q = a.lerp(b, t);
                (p - q).truncate().length() <= margin && (p.z - q.z).abs() <= 2.0
            })).unwrap_or(false);
            if on_surface { covered += 1; }
        }
        if covered * 4 >= n * 3 { lane.invisible = false; }
    }
    // Crossings made of objects can have no asphalt spline. Keep helpers joining two
    // corroborated roads; an isolated invisible helper still does not become a street.
    let mut prev = vec![Vec::new(); net.lanes.len()];
    for (i, lane) in net.lanes.iter().enumerate().filter(|(_, l)| l.kind == LaneKind::Street) {
        for &j in &lane.next {
            if net.lanes.get(j).map(|l| l.kind == LaneKind::Street).unwrap_or(false) { prev[j].push(i); }
        }
    }
    let mut seen = vec![false; net.lanes.len()];
    let mut keep = Vec::new();
    for seed in 0..net.lanes.len() {
        if seen[seed] || !net.lanes[seed].invisible || net.lanes[seed].kind != LaneKind::Street { continue; }
        let mut queue = vec![seed];
        let mut component = Vec::new();
        let mut anchors = hashbrown::HashSet::new();
        while let Some(i) = queue.pop() {
            if seen[i] { continue; }
            seen[i] = true;
            component.push(i);
            for &j in net.lanes[i].next.iter().chain(prev[i].iter()) {
                let Some(l) = net.lanes.get(j).filter(|l| l.kind == LaneKind::Street) else { continue };
                if l.invisible {
                    if !seen[j] { queue.push(j); }
                } else {
                    anchors.insert((l.key, l.source, if l.key.is_none() { j } else { 0 }));
                }
            }
        }
        if anchors.len() >= 2 { keep.extend(component); }
    }
    for i in keep { net.lanes[i].invisible = false; }
}

/// Traffic paths are lane centres, not separate streets. Adjacent paths on one spline
/// describe one carriageway; combine their footprints without filling a median. Align
/// opposing directions before averaging so bends, mirrors and ramps retain their shape.
fn road_geometry(net: &Network) -> Vec<MapRoad> {
    let mut roads = Vec::new();
    let mut splines = std::collections::BTreeMap::<((i32, i32), i64), Vec<&omsi_sim::traffic::Lane>>::new();
    for (_, lane) in visible_road_lanes(net) {
        if let Some(key) = lane.key.filter(|_| lane.source == 1) {
            splines.entry((key.tile, key.id)).or_default().push(lane);
        } else {
            roads.push(MapRoad { points: lane.points.clone(), width: lane.width.max(2.6), main: lane.speed_limit_kmh >= 55.0 });
        }
    }
    for mut lanes in splines.into_values() {
        lanes.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        let mut start = 0;
        while start < lanes.len() {
            let a = lanes[start];
            let lo = a.offset - a.width.max(2.6) * 0.5;
            let mut hi = a.offset + a.width.max(2.6) * 0.5;
            let mut end = start + 1;
            while end < lanes.len() {
                let b = lanes[end];
                if b.points.len() != a.points.len() || b.offset - b.width.max(2.6) * 0.5 > hi + 0.65 { break; }
                let mid = a.points.len() / 2;
                let za = a.points[if a.reversed { a.points.len() - 1 - mid } else { mid }].z;
                let zb = b.points[if b.reversed { b.points.len() - 1 - mid } else { mid }].z;
                if (za - zb).abs() > 1.5 { break; }
                hi = hi.max(b.offset + b.width.max(2.6) * 0.5);
                end += 1;
            }
            let b = lanes[end - 1];
            let span = b.offset - a.offset;
            let t = if span.abs() > 0.01 { (((lo + hi) * 0.5 - a.offset) / span) as f64 } else { 0.0 };
            let point = |l: &omsi_sim::traffic::Lane, i: usize| l.points[if l.reversed { l.points.len() - 1 - i } else { i }];
            let points = (0..a.points.len()).map(|i| point(a, i).lerp(point(b, i), t)).collect();
            roads.push(MapRoad { points, width: hi - lo, main: lanes[start..end].iter().any(|l| l.speed_limit_kmh >= 55.0) });
            start = end;
        }
    }
    // Bridge only explicit graph connections within the map's placement tolerance.
    for lane in net.lanes.iter().filter(|l| l.kind == LaneKind::Street && !l.invisible && l.points.len() >= 2) {
        for &j in &lane.next {
            let Some(next) = net.lanes.get(j).filter(|l| l.kind == LaneKind::Street && !l.invisible && l.points.len() >= 2) else { continue };
            let distance = lane.end().distance(next.start());
            if distance > 0.05 && distance <= 2.0 {
                roads.push(MapRoad { points: vec![lane.end(), next.start()], width: lane.width.min(next.width).max(2.6), main: lane.speed_limit_kmh >= 55.0 && next.speed_limit_kmh >= 55.0 });
            }
        }
    }
    roads
}

/// An arrow at `at` pointing `angle` (radians, clockwise from up on the screen), `k` px from
/// its middle to its tip, on a dark outline `grow` times its size: the bus and the players.
fn arrow(ui: &mut Painter, at: Vec2, angle: f32, k: f32, grow: f32, dark: Color, fill: Color) {
    let rot = |v: Vec2| Vec2::new(v.x * angle.cos() - v.y * angle.sin(), v.x * angle.sin() + v.y * angle.cos());
    let tip = at + rot(Vec2::new(0.0, -1.0) * k);
    let l = at + rot(Vec2::new(-0.7, 0.8) * k);
    let m = at + rot(Vec2::new(0.0, 0.4) * k);
    let r = at + rot(Vec2::new(0.7, 0.8) * k);
    let g = |p: Vec2| at + (p - at) * grow;
    ui.tri(g(tip), g(l), g(m), dark, dark, dark);
    ui.tri(g(tip), g(m), g(r), dark, dark, dark);
    ui.tri(tip, l, m, fill, fill, fill);
    ui.tri(tip, m, r, fill, fill, fill);
}

fn rects_overlap(a: &Rect, b: &Rect) -> bool {
    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
}

/// Stop positions stay on the route. Move only their labels, or omit a label if none of
/// the nearby positions fits. Call in route order so the next stop has first choice.
fn stop_label_rect(p: Vec2, width: f32, s: f32, win: Rect, taken: &[Rect]) -> Option<Rect> {
    let h = 20.0 * s;
    let gap = 10.0 * s;
    let positions = [
        Vec2::new(p.x + gap, p.y - h * 0.5),
        Vec2::new(p.x - gap - width, p.y - h * 0.5),
        Vec2::new(p.x + gap, p.y - h - gap),
        Vec2::new(p.x + gap, p.y + gap),
        Vec2::new(p.x - gap - width, p.y - h - gap),
        Vec2::new(p.x - gap - width, p.y + gap),
    ];
    positions.into_iter().map(|q| Rect::new(q.x, q.y, width, h)).find(|r| {
        r.x >= 8.0 * s && r.right() <= win.right() - 8.0 * s && r.y >= 50.0 * s && r.bottom() <= win.bottom() - 8.0 * s
            && !taken.iter().any(|t| rects_overlap(t, r))
    })
}

fn spaced_markers(points: impl IntoIterator<Item = (usize, Vec2)>, gap: f32) -> Vec<(usize, Vec2)> {
    let mut kept: Vec<(usize, Vec2)> = Vec::new();
    for (k, p) in points {
        if kept.iter().all(|(_, q)| p.distance(*q) >= gap) { kept.push((k, p)); }
    }
    kept
}

/// Street lanes within `ROAD_RADIUS` of `anchor`: casings first, then surfaces (so that a
/// junction's surfaces cover each other's casings).
fn build_roads(p: &mut Painter, net: &Network, anchor: DVec2) {
    let rel = |q: DVec3| Vec3::new((q.x - anchor.x) as f32, (q.y - anchor.y) as f32, 0.0);
    let lanes: Vec<(MapRoad, Vec<Vec3>)> = road_geometry(net)
        .into_iter()
        .filter(|l| l.points.iter().any(|q| (q.truncate() - anchor).length() < ROAD_RADIUS))
        .map(|l| { let pts = simplify(&l.points.iter().map(|q| rel(*q)).collect::<Vec<_>>(), 0.12); (l, pts) })
        .collect();
    for (l, pts) in &lanes {
        p.ribbon(pts, l.width + 1.6, 3.6, ROAD_CASING, true);
    }
    for (l, pts) in &lanes {
        p.ribbon(pts, l.width + 0.2, 2.4, if l.main { ROAD_MAIN } else { ROAD }, true);
    }
}

/// `pts` with the points dropped that lie within `tol` metres of the line through their
/// neighbours kept (Douglas-Peucker): a lane is sampled every metre or two, a ribbon needs
/// only its bends. The launcher's map picture draws its roads with this too.
pub(crate) fn simplify(pts: &[Vec3], tol: f32) -> Vec<Vec3> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (pa, pb) = (pts[a].truncate(), pts[b].truncate());
        let ab = pb - pa;
        let len = ab.length().max(1e-6);
        let mut worst = (0.0f32, 0usize);
        for k in a + 1..b {
            let d = (pts[k].truncate() - pa).perp_dot(ab).abs() / len;
            if d > worst.0 {
                worst = (d, k);
            }
        }
        if worst.0 > tol {
            keep[worst.1] = true;
            stack.push((a, worst.1));
            stack.push((worst.1, b));
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// Lanes with a point within `radius` of `c` (by the network's 50 m grid).
fn lanes_near(net: &Network, c: DVec2, radius: f64) -> Vec<usize> {
    let mut seen = hashbrown::HashSet::new();
    let (cx, cy) = Network::grid_cell(c.extend(0.0));
    let r = (radius / 50.0).ceil() as i32;
    if net.grid.is_empty() {
        return (0..net.lanes.len()).filter(|&i| net.lanes[i].points.iter().any(|q| (q.truncate() - c).length() < radius)).collect();
    }
    for gx in cx - r..=cx + r {
        for gy in cy - r..=cy + r {
            if let Some(v) = net.grid.get(&(gx, gy)) {
                seen.extend(v.iter().copied());
            }
        }
    }
    let mut v: Vec<usize> = seen.into_iter().collect();
    v.sort_unstable();
    v
}

/// `OMSI_NAV_PROBE=x,y[,r]`: the lanes of the map's network that start or end within r
/// metres (25) of a point - how they link, to see why a route cannot reach a place.
fn probe_lanes(net: &Network) {
    let Some(v) = omsi_cfg::flags::OMSI_NAV_PROBE.var() else { return };
    let f: Vec<f64> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    if f.len() < 2 {
        return;
    }
    let c = DVec2::new(f[0], f[1]);
    let r = f.get(2).copied().unwrap_or(25.0);
    for (i, l) in net.lanes.iter().enumerate() {
        let (s, e) = (l.start(), l.end());
        if (s.truncate() - c).length() > r && (e.truncate() - c).length() > r && l.nearest_point(DVec3::new(c.x, c.y, s.z)).map(|p| p.1 > r).unwrap_or(true) {
            continue;
        }
        let prev = net.prev.get(i).cloned().unwrap_or_default();
        log::info!(
            "nav probe: lane {i} {:?} {:?} key {:?} rev {} start ({:.1}, {:.1}, {:.1}) h {:.0} end ({:.1}, {:.1}, {:.1}) h {:.0} len {:.1} next {:?} prev {:?}",
            l.kind, l.name, l.key, l.reversed, s.x, s.y, s.z, l.start_heading(), e.x, e.y, e.z, l.end_heading(), l.length(), l.next, prev
        );
    }
}

/// How a route is drawn: its width (lane width plus `extra_m`, at least `min_px`), arrows
/// every so many metres for so far, how much of it, and only near a point.
struct RouteStyle {
    extra_m: f32,
    min_px: f32,
    arrows: Option<(f32, f32)>,
    max_len: f32,
    near: Option<(DVec2, f64)>,
}

/// Which lane of `lane`'s road (the lanes beside it going the same way) the bus at `bus`
/// is in, counted from the kerb (0 = the kerb lane: the rightmost, the leftmost on a
/// left-hand-traffic map).
fn lane_from_right(net: &Network, lane: usize, bus: DVec3) -> usize {
    let Some(mut cur) = net.lanes.get(lane).map(|_| lane) else { return 0 };
    let kerb = |l: &omsi_sim::traffic::Lane| if net.left_hand { l.left } else { l.right };
    let away = |l: &omsi_sim::traffic::Lane| if net.left_hand { l.right } else { l.left };
    for _ in 0..6 {
        match kerb(&net.lanes[cur]) {
            Some(n) if n < net.lanes.len() && net.lanes[n].kind == LaneKind::Street => cur = n,
            _ => break,
        }
    }
    let (mut best, mut best_d, mut k) = (0usize, f64::MAX, 0usize);
    loop {
        if let Some((_, d)) = net.lanes[cur].nearest_point(bus) {
            if d < best_d {
                best_d = d;
                best = k;
            }
        }
        match away(&net.lanes[cur]) {
            Some(n) if n < net.lanes.len() && net.lanes[n].kind == LaneKind::Street && k < 6 => {
                cur = n;
                k += 1;
            }
            _ => break,
        }
    }
    best
}

/// The route from the bus on: one band, coloured stretch by stretch by how busy the road
/// is (`jam`: congestion per lane), with arrows along it in a colour that stands out.
#[allow(clippy::too_many_arguments)]
fn build_route(p: &mut Painter, net: &Network, lanes: &[usize], anchor: DVec2, s0: f32, jam: &HashMap<usize, f32>, style: &RouteStyle, bus_lane: usize) {
    let rel = |q: DVec3| Vec3::new((q.x - anchor.x) as f32, (q.y - anchor.y) as f32, 0.0);
    // runs of one colour: the lanes' points joined up, the first lane cut where the bus is
    let mut runs: Vec<(Vec<Vec3>, f32, usize)> = Vec::new();
    let mut arrows: Vec<(DVec3, f32, usize, f32)> = Vec::new();
    let mut total = 0.0f32;
    // the lane to show: of a road with several lanes each way, the one a driver takes - the
    // leftmost before a left turn, the rightmost before a right turn, and otherwise the one
    // the bus is driving in (`bus_lane`, counted from the right). Always the rightmost, the
    // route ran along the kerb on Berlin's three-lane roads, where the cars are parked; the
    // timetable's own track often runs in the middle lane.
    let turn_after = |k: usize| -> i32 {
        let mut acc = 0.0f32;
        for &j in lanes.iter().skip(k + 1).take(40) {
            let Some(l) = net.lanes.get(j) else { break };
            let d = omsi_sim::traffic::wrap_deg(l.end_heading() - l.start_heading());
            if d.abs() > 35.0 && l.length() < 60.0 {
                return if d > 0.0 { 1 } else { -1 };
            }
            // a lane that has no neighbours ends the stretch of several lanes
            if l.left.is_none() && l.right.is_none() {
                break;
            }
            acc += l.length();
            if acc > 250.0 {
                break;
            }
        }
        0
    };
    let shown = |k: usize, l: usize| -> usize {
        let side = turn_after(k);
        let step = |cur: usize, left: bool| -> Option<usize> {
            let n = if left { net.lanes[cur].left } else { net.lanes[cur].right }?;
            (n < net.lanes.len() && net.lanes[n].kind == LaneKind::Street).then_some(n)
        };
        let mut cur = l;
        // before a turn the lane on its side; straight on the bus's own, counted from the kerb
        // (the right, or the left on a left-hand-traffic map)
        let to_left = if side == 0 { net.left_hand } else { side < 0 };
        for _ in 0..6 {
            match step(cur, to_left) {
                Some(n) => cur = n,
                None => break,
            }
        }
        if side == 0 {
            for _ in 0..bus_lane {
                match step(cur, !net.left_hand) {
                    Some(n) => cur = n,
                    None => break,
                }
            }
        }
        cur
    };
    for (k, &l) in lanes.iter().enumerate() {
        let Some(lane) = net.lanes.get(l) else { break };
        let route_l = l;
        let l = shown(k, l);
        let lane = net.lanes.get(l).unwrap_or(lane);
        if let Some((c, r)) = style.near {
            if (c - lane.start().truncate()).length() > r {
                break;
            }
        }
        let from = if k == 0 { s0 } else { 0.0 };
        let mut pts: Vec<Vec3> = Vec::new();
        if k == 0 {
            pts.push(rel(lane.at(s0).0));
        }
        for (q, d) in lane.points.iter().zip(&lane.dist) {
            if *d > from + 0.05 || k > 0 {
                pts.push(rel(*q));
            }
        }
        let lv = level(jam.get(&route_l).copied().unwrap_or(0.0));
        let w = lane.width.max(2.6);
        match runs.last_mut() {
            Some(run) if run.2 == lv => {
                run.0.extend(pts);
                run.1 = run.1.max(w);
            }
            _ => {
                // (a new colour starts where the last one ended)
                let mut start = runs.last().and_then(|r| r.0.last().copied()).map(|q| vec![q]).unwrap_or_default();
                start.extend(pts);
                runs.push((start, w, lv));
            }
        }
        // arrows at even steps along each lane (they stay put while the bus drives)
        if let Some((every, reach)) = style.arrows {
            let len = lane.length();
            // (never more than a few dozen a lane: zoomed far in on the city map the step
            // came out a few millimetres and the loop ran for minutes - the game froze)
            if total < reach && len > every * 0.4 && every > 0.5 {
                let n = (len / every).round().clamp(1.0, 64.0);
                let step = len / n;
                for i in 0..n as usize {
                    let at = step * (i as f32 + 0.5);
                    if at > from + 4.0 && total + at - from < reach {
                        let (q, h) = lane.at(at);
                        arrows.push((q, h, lv, w));
                    }
                }
            }
        }
        total += lane.length() - from;
        if total > style.max_len {
            break;
        }
    }
    for (pts, w, lv) in &runs {
        p.ribbon(pts, w + style.extra_m, style.min_px, LEVEL[*lv], true);
    }
    for (q, h, lv, w) in arrows {
        let hr = h.to_radians();
        let d = Vec2::new(hr.sin(), hr.cos());
        let n = Vec2::new(-d.y, d.x);
        let (tip, bl, br) = (d * 0.55, -d * 0.45 + n * 0.75, -d * 0.45 - n * 0.75);
        let t = -d * 0.42;
        let (sm, spx) = ((w + style.extra_m) * 0.5 * 0.8, style.min_px * 0.5 * 1.05);
        let c = ARROW[lv];
        p.world_shape(rel(q), &[tip, bl, bl + t, tip + t], sm, spx, c);
        p.world_shape(rel(q), &[tip, br, br + t, tip + t], sm, spx, c);
    }
}

/// The street names of the map: per lane of its network the street it belongs to, and
/// where to write each name on the city map. OMSI maps have no street names of their own,
/// but their street name signs carry them: a sign names the road its plate runs along,
/// and the name is carried on along that road until it turns or meets another name.
pub struct Streets {
    names: Vec<String>,
    /// Per lane: index into `names`, or `u32::MAX`.
    of_lane: Vec<u32>,
    /// A place to write a name: the point, the road's direction (radians, world, from +x
    /// anticlockwise) and the name.
    labels: Vec<(DVec2, f32, u32)>,
}

/// The heading of a sign whose plate runs along its road, relative to the road (degrees;
/// settled on the stock Verkehrszeichen_MC signs of Berlin-Spandau, see `build_streets`).
const SIGN_ALONG: f64 = 90.0;

fn build_streets(net: &Network, signs: &[(DVec3, f64, String)]) -> Streets {
    let t0 = std::time::Instant::now();
    let n = net.lanes.len();
    let mut names: Vec<String> = Vec::new();
    let mut index: HashMap<String, u32> = HashMap::new();
    let mut of_lane = vec![u32::MAX; n];
    // lanes into each lane, to carry names backwards
    let mut prev: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, l) in net.lanes.iter().enumerate() {
        for &j in &l.next {
            if j < n {
                prev[j].push(i);
            }
        }
    }
    let straight = |l: &omsi_sim::traffic::Lane| l.kind == LaneKind::Street && omsi_sim::traffic::wrap_deg(l.end_heading() - l.start_heading()).abs() < 30.0;
    let debug = omsi_cfg::flags::OMSI_DEBUG_NAV.is_set();
    let mut hist = [0u32; 12];
    let mut seeds: Vec<(usize, u32)> = Vec::new();
    for (pos, rot, name) in signs {
        let id = *index.entry(name.clone()).or_insert_with(|| {
            names.push(name.clone());
            (names.len() - 1) as u32
        });
        // the straight street lanes near the sign, and how well each runs along its plate
        let mut best: Option<(usize, f64)> = None;
        for i in lanes_near(net, pos.truncate(), 30.0) {
            let l = &net.lanes[i];
            if !straight(l) || l.length() < 12.0 {
                continue;
            }
            let Some((s, d)) = l.nearest_point(DVec3::new(pos.x, pos.y, l.start().z)) else { continue };
            if d > 22.0 {
                continue;
            }
            let (_, h) = l.at(s);
            // (the direction of a road does not matter: modulo 180)
            let off = (angle_diff(*rot, h as f64).abs() - SIGN_ALONG).abs();
            let off = off.min(180.0 - off);
            if debug && d < 12.0 {
                let raw = angle_diff(*rot, h as f64).rem_euclid(180.0);
                hist[((raw / 15.0) as usize).min(11)] += 1;
            }
            let score = d + off * 0.5;
            if off < 35.0 && best.map(|b| score < b.1).unwrap_or(true) {
                best = Some((i, score));
            }
        }
        if let Some((i, _)) = best {
            seeds.push((i, id));
        }
    }
    if debug {
        if let Some((pos, rot, name)) = signs.first() {
            let near = lanes_near(net, pos.truncate(), 30.0);
            let best = near.iter().filter_map(|&i| net.lanes[i].nearest_point(*pos).map(|p| (i, p.1, net.lanes[i].kind, net.lanes[i].length()))).min_by(|a, b| a.1.total_cmp(&b.1));
            log::info!("navigator: first sign '{name}' at {pos:?} heading {rot}: {} lanes near, nearest {best:?}", near.len());
        }
        // which way a plate runs: two signs of one name far apart lie along their street
        let mut along = [0u32; 12];
        for (i, (p, r, n)) in signs.iter().enumerate() {
            for (q, _, m) in &signs[i + 1..] {
                let d = (*q - *p).truncate();
                if n == m && d.length() > 150.0 && d.length() < 700.0 {
                    let h = d.x.atan2(d.y).to_degrees();
                    let raw = angle_diff(*r, h).rem_euclid(180.0);
                    along[((raw / 15.0) as usize).min(11)] += 1;
                }
            }
        }
        log::info!("navigator: sign heading minus the line to another sign of its name (mod 180): {along:?}");
        log::info!("navigator: sign heading minus road heading (mod 180, 15-degree bins): {hist:?}");
    }
    // carry each name along its road: the seed lane, its other direction, and on through
    // straight continuations for up to 1.5 km each way
    for &(seed, id) in &seeds {
        if of_lane[seed] != u32::MAX {
            continue;
        }
        let mut queue = vec![(seed, 0.0f32)];
        while let Some((i, far)) = queue.pop() {
            if of_lane[i] != u32::MAX && i != seed {
                continue;
            }
            of_lane[i] = id;
            let l = &net.lanes[i];
            if let Some(k) = l.key {
                for &j in net.by_key.get(&k).map(|v| v.as_slice()).unwrap_or(&[]) {
                    if of_lane[j] == u32::MAX {
                        of_lane[j] = id;
                    }
                }
            }
            if far > 1500.0 {
                continue;
            }
            for &j in l.next.iter() {
                let m = &net.lanes[j];
                if of_lane[j] == u32::MAX && straight(m) && omsi_sim::traffic::wrap_deg(m.start_heading() - l.end_heading()).abs() < 20.0 {
                    queue.push((j, far + m.length()));
                }
            }
            for &j in &prev[i] {
                let m = &net.lanes[j];
                if of_lane[j] == u32::MAX && straight(m) && omsi_sim::traffic::wrap_deg(l.start_heading() - m.end_heading()).abs() < 20.0 {
                    queue.push((j, far + m.length()));
                }
            }
        }
    }
    // where to write them: the middle of long named lanes, a name every 350 m at most
    let mut order: Vec<usize> = (0..n).filter(|&i| of_lane[i] != u32::MAX && !net.lanes[i].reversed && net.lanes[i].length() > 30.0).collect();
    order.sort_by(|a, b| net.lanes[*b].length().total_cmp(&net.lanes[*a].length()));
    let mut labels: Vec<(DVec2, f32, u32)> = Vec::new();
    for i in order {
        let l = &net.lanes[i];
        let (q, h) = l.at(l.length() * 0.5);
        let q = q.truncate();
        if labels.iter().any(|(p, _, id)| *id == of_lane[i] && (*p - q).length() < 350.0) {
            continue;
        }
        let hr = (h as f64).to_radians();
        labels.push((q, (hr.cos()).atan2(hr.sin()) as f32, of_lane[i]));
    }
    let named = of_lane.iter().filter(|&&x| x != u32::MAX).count();
    log::info!("navigator: {} street name signs, {} names, {} of {} lanes named, {} labels, {:.0} ms", signs.len(), names.len(), named, n, labels.len(), t0.elapsed().as_secs_f64() * 1000.0);
    Streets { names, of_lane, labels }
}

/// A way from where the bus is back onto `ahead` (the route from the lane it was last
/// on): the lanes to drive up to the route, and the index in `ahead` of the route lane it
/// reaches (the route goes on from there). Dijkstra over
/// the street lanes, from the lane under the bus that runs its way, to the first route
/// lane reached (the search stops at 6 km).
pub(crate) fn way_back(net: &Network, bus: DVec3, heading: f64, ahead: &[usize], max_cost: f32) -> Option<(Vec<usize>, usize)> {
    use std::cmp::Ordering;
    use std::collections::BinaryHeap;
    // the lane under the bus going its way; else (a depot, a car park, the grass) the
    // nearest street lane that does not point back at the bus, else any near
    let cands: Vec<(usize, f64, f64)> = lanes_near(net, bus.truncate(), 90.0)
        .into_iter()
        .filter_map(|i| {
            let l = net.lanes.get(i)?;
            if l.kind != LaneKind::Street {
                return None;
            }
            let (s, d) = l.nearest_point(bus)?;
            let (_, h) = l.at(s);
            Some((i, d, angle_diff(heading, h as f64).abs()))
        })
        .collect();
    let pick = |max_d: f64, max_a: f64| cands.iter().filter(|c| c.1 < max_d && c.2 < max_a).min_by(|a, b| (a.1 + a.2 * 0.1).total_cmp(&(b.1 + b.2 * 0.1))).map(|c| c.0);
    let start = pick(14.0, 70.0).or_else(|| pick(80.0, 100.0)).or_else(|| pick(80.0, 181.0));
    if omsi_cfg::flags::OMSI_DEBUG_NAV.is_set() {
        log::info!("navigator: way search from {:?} ({} street lanes within 90 m) to {} route lanes", start, cands.len(), ahead.len());
    }
    let start = start?;
    let targets: HashMap<usize, usize> = ahead.iter().take(120).enumerate().map(|(k, &l)| (l, k)).collect();
    #[derive(PartialEq)]
    struct Node(f32, usize);
    impl Eq for Node {}
    impl Ord for Node {
        fn cmp(&self, o: &Self) -> Ordering {
            o.0.total_cmp(&self.0)
        }
    }
    impl PartialOrd for Node {
        fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
            Some(self.cmp(o))
        }
    }
    let mut dist: HashMap<usize, f32> = HashMap::new();
    let mut parent: HashMap<usize, usize> = HashMap::new();
    let mut heap = BinaryHeap::new();
    dist.insert(start, 0.0);
    heap.push(Node(0.0, start));
    while let Some(Node(cost, lane)) = heap.pop() {
        if cost > dist.get(&lane).copied().unwrap_or(f32::INFINITY) {
            continue;
        }
        if lane != start {
            if let Some(&k) = targets.get(&lane) {
                let mut path = vec![lane];
                let mut c = lane;
                while let Some(&p) = parent.get(&c) {
                    path.push(p);
                    c = p;
                }
                path.reverse();
                // (the route lane reached is where the route goes on)
                path.pop();
                return Some((path, k));
            }
        }
        if cost > max_cost {
            break;
        }
        let Some(l) = net.lanes.get(lane) else { continue };
        for &n in &l.next {
            let Some(nl) = net.lanes.get(n) else { continue };
            if nl.kind != LaneKind::Street {
                continue;
            }
            // turning back is the last resort
            let u_turn = angle_diff(l.end_heading() as f64, nl.start_heading() as f64).abs() > 150.0;
            let c = cost + nl.length() + if u_turn { 400.0 } else { 0.0 };
            if c < dist.get(&n).copied().unwrap_or(f32::INFINITY) {
                dist.insert(n, c);
                parent.insert(n, lane);
                heap.push(Node(c, n));
            }
        }
    }
    None
}

/// Rotation of a 2D direction by a compass heading (for tests).
#[allow(dead_code)]
fn heading_vec(h: f64) -> DVec2 {
    let m = DMat3::from_rotation_z(-h.to_radians());
    m.transform_vector2(DVec2::Y)
}

/// The city map: the whole map from above in a large window over the game (after ETS2's
/// map screen, but not full-screen) - every road, the trip's route, its stops with their
/// names, the bus and the traffic. Dragged to move, the wheel zooms at the cursor, Escape
/// or a click outside closes it.
#[derive(Default)]
pub struct CityMap {
    pub open: bool,
    /// Where the window is on the screen (physical pixels).
    pub rect: [f32; 4],
    /// Centre of the view (world) and metres per pixel.
    center: DVec2,
    mpp: f64,
    /// The view stays on the bus until the map is dragged.
    follow: bool,
    drag: Option<(f32, f32)>,
    target: Option<(TextureId, u32, u32)>,
    /// The roads mesh (buffer 3): the map version it was built for, its size and anchor.
    roads: Option<(u64, u32, DVec2)>,
    /// The route mesh (buffer 4): the route, traffic, arrow spacing and roads versions it
    /// was built for, and its size.
    route: ((u64, u64, u32, u64), u32),
    /// The map's extent (world), for the zoom limits.
    extent: (DVec2, DVec2),
    /// "Centre on the bus" and zoom buttons (window pixels).
    buttons: Vec<(Rect, u8)>,
}

impl Navigator {
    /// Open or close the city map.
    /// Where OMSI 2's dynamic route arrows stand on the route ahead (the `nav_arrows`
    /// setting): every junction lane of the next `reach` metres with the way through it
    /// (`L`, `R`, or `dn` for straight on), and the stops of the trip ahead with their
    /// names (`busstop`). Each: a key that stays the same while it is ahead, the place,
    /// the heading, the kind and its text.
    /// `stop_pose` gives a stop object's place and heading where its tile is loaded.
    pub fn arrow_spots(&self, traffic: Option<&Network>, reach: f64, stop_pose: &dyn Fn(i64) -> Option<(DVec3, f64)>) -> Vec<(u64, DVec3, f64, &'static str, String)> {
        let mut out = Vec::new();
        if !self.arrows {
            return out;
        }
        let global = self.global.clone();
        let Some(net) = global.as_deref().or(traffic) else { return out };
        let r = &self.route;
        if !r.on_route {
            return out;
        }
        let mut acc = -(r.s as f64);
        let mut prev_end: Option<f32> = None;
        for (j, &l) in r.lanes.iter().enumerate().skip(r.progress) {
            let Some(lane) = net.lanes.get(l) else { break };
            let len = lane.length();
            if acc > reach {
                break;
            }
            let (h0, h1) = (lane.start_heading(), lane.end_heading());
            let mut d = omsi_sim::traffic::wrap_deg(h1 - h0);
            if let Some(pe) = prev_end {
                d += omsi_sim::traffic::wrap_deg(h0 - pe);
            }
            let junction = net.crossings.get(l).map(|c| !c.is_empty()).unwrap_or(false) || lane.turn != 0;
            let turn = d.abs() > 35.0 && (len < 60.0 || d.abs() > 70.0);
            if (turn || junction) && acc + len as f64 > 5.0 {
                let kind = if !turn { "dn" } else if d > 0.0 { "R" } else { "L" };
                // ten metres on from where the path begins, as OMSI puts it (the original:
                // the path's start moved by (0, 0, 10) in its own frame); the mesh hangs
                // 6-11 m over that point
                let (p, _) = lane.at(10.0f32.min(len * 0.7));
                // facing the driver coming in: the path's heading where it begins
                let h = h0;
                // the street the route goes on into (the junction's own lanes have no name)
                let text = r.lanes.iter().skip(j).take(5).find_map(|&x| self.street_of(x)).map(str::to_string).unwrap_or_default();
                out.push((l as u64, p, h as f64, kind, text));
            }
            prev_end = Some(h1);
            acc += len as f64;
        }
        // The stop's helper: Omsi.exe puts `routearrows_busstop.sco` on the stop object
        // itself, at its place and with its rotation (0x61fc04: the station record's
        // position +0x3c and quaternion +0x54) - where and how the mapper set the stop down,
        // not turned to the bus as it comes.
        for (k, (p, name, h, id)) in self.stop_spots.iter().enumerate() {
            let (p, h) = stop_pose(*id).unwrap_or((*p, *h));
            let d = (p - self.bus_at).truncate().length();
            if d < reach && k < 2 {
                out.push(((1u64 << 40) + p.x.to_bits().rotate_left(7) ^ p.y.to_bits() ^ h.to_bits(), p, h, "busstop", name.clone()));
            }
        }
        out
    }

    pub fn toggle_map(&mut self) {
        self.city.open = !self.city.open;
        if self.city.open {
            self.city.follow = true;
            if self.city.mpp <= 0.0 {
                // For repeatable offscreen GPS comparisons at a chosen zoom.
                self.city.mpp = omsi_cfg::flags::OMSI_NAV_MAP_MPP.parse::<f64>().filter(|v| v.is_finite() && (0.25..=20.0).contains(v)).unwrap_or(2.5);
            }
        }
        self.city.drag = None;
    }

    pub fn map_open(&self) -> bool {
        self.city.open
    }

    /// Where the small navigator is on the screen, when it is shown.
    pub fn screen_rect(&self) -> Option<[f32; 4]> {
        let r = self.panel_rect;
        (self.enabled && !self.city.open && r[2] > r[0]).then_some(r)
    }

    /// The point (physical pixels) is on the small navigator.
    pub fn over_panel(&self, x: f32, y: f32) -> bool {
        let r = self.panel_rect;
        self.enabled && x >= r[0] && y >= r[1] && x < r[2] && y < r[3]
    }

    /// Where the small navigator (`size`) goes on a screen of `screen`: its corner, the top
    /// middle with the on-screen controls (`touch`), below the information bar (`info`) where
    /// they would lie over each other - unless it was dragged somewhere, which it keeps
    /// (kept inside the window), by the mouse or a finger.
    fn panel_origin(&mut self, screen: (f32, f32), size: (f32, f32), touch: bool, info: Option<[f32; 4]>) -> (f32, f32) {
        let ((sw, sh), (pw, ph)) = (screen, size);
        let margin = (sh * 0.018).max(10.0).round();
        // (with the on-screen controls the corners are theirs: the top middle)
        let right = self.corner.contains("right");
        let top = self.corner.contains("top") || touch;
        // ("top-center": a phone's, between its on-screen buttons)
        let x0 = if self.corner.contains("center") || touch { ((sw - pw) * 0.5).round() } else if right { sw - margin - pw } else { margin };
        let y0 = if top { margin } else { sh - margin - ph };
        // (below the information bar where they would lie over each other: in its rows on a
        // phone it took the navigator's place in the top middle, #1164)
        let y0 = match info {
            Some(i) if top && x0 < i[2] && x0 + pw > i[0] => y0.max((i[3] + margin * 0.5).round()),
            _ => y0,
        };
        // (dragged somewhere else: there - on a phone too, where it had to stay in the
        // middle it covered, #1138)
        let room = [(sw - pw).max(0.0), (sh - ph).max(0.0)];
        self.panel_room = room;
        match self.at {
            Some(a) => ((a[0].clamp(0.0, 1.0) * room[0]).round(), (a[1].clamp(0.0, 1.0) * room[1]).round()),
            None => (x0, y0),
        }
    }

    /// The mouse button went down on the small navigator: a click opens the city map, a
    /// drag moves the navigator (see [`Navigator::panel_move`]).
    pub fn panel_press(&mut self, x: f32, y: f32) {
        self.panel_drag = Some((
            [x, y],
            [self.panel_rect[0] - self.origin_x, self.panel_rect[1]],
            false,
        ));
    }

    /// The cursor moved with the button held on the navigator: past a few pixels it follows
    /// the cursor. True while it is being dragged.
    pub fn panel_move(&mut self, x: f32, y: f32) -> bool {
        let Some((from, rect, moved)) = self.panel_drag.as_mut() else { return false };
        let (dx, dy) = (x - from[0], y - from[1]);
        if !*moved && dx.hypot(dy) < 6.0 {
            return false;
        }
        *moved = true;
        let room = self.panel_room;
        let share = |p: f32, r: f32| if r > 0.0 { (p / r).clamp(0.0, 1.0) } else { 0.0 };
        self.at = Some([share(rect[0] + dx, room[0]), share(rect[1] + dy, room[1])]);
        true
    }

    /// The button came up after a press on the navigator: Some(true) when it was dragged
    /// (the place is then the setting's, [`Navigator::placement`]), Some(false) for a click.
    pub fn panel_release(&mut self) -> Option<bool> {
        self.panel_drag.take().map(|d| d.2)
    }

    /// The setting `navigator_corner` for where the navigator is now.
    pub fn placement(&self) -> String {
        match self.at {
            Some(a) => format!("at {:.3},{:.3}", a[0], a[1]),
            None => self.corner.clone(),
        }
    }

    fn map_hit(&self, x: f32, y: f32) -> bool {
        let r = self.city.rect;
        x >= r[0] && y >= r[1] && x < r[2] && y < r[3]
    }

    /// A mouse press while the map is open: on it, a drag or a button; outside, it closes.
    pub fn map_press(&mut self, x: f32, y: f32) {
        if !self.map_hit(x, y) {
            self.city.open = false;
            return;
        }
        let local = Vec2::new(x - self.city.rect[0], y - self.city.rect[1]);
        let hit = self.city.buttons.iter().find(|(r, _)| r.contains(local)).map(|b| b.1);
        match hit {
            Some(0) => self.city.follow = true,
            Some(1) => self.city.mpp = (self.city.mpp / 1.6).max(0.25),
            Some(2) => self.city.mpp = (self.city.mpp * 1.6).min(self.max_mpp()),
            Some(3) => self.city.open = false,
            _ => self.city.drag = Some((x, y)),
        }
    }

    /// The world point (x, y) under the window point, when the map is open and the point
    /// is on it.
    pub fn map_point(&self, x: f32, y: f32) -> Option<DVec2> {
        if !self.city.open || !self.map_hit(x, y) {
            return None;
        }
        let r = self.city.rect;
        let (w, h) = ((r[2] - r[0]) as f64, (r[3] - r[1]) as f64);
        let (lx, ly) = ((x - r[0]) as f64 - w * 0.5, (y - r[1]) as f64 - h * 0.5);
        Some(self.city.center + DVec2::new(lx, -ly) * self.city.mpp)
    }

    pub fn map_release(&mut self) {
        self.city.drag = None;
    }

    pub fn map_move(&mut self, x: f32, y: f32) {
        if let Some((px, py)) = self.city.drag {
            let (dx, dy) = ((x - px) as f64, (y - py) as f64);
            if dx.abs() + dy.abs() > 0.5 {
                self.city.follow = false;
            }
            self.city.center.x -= dx * self.city.mpp;
            self.city.center.y += dy * self.city.mpp;
            self.city.drag = Some((x, y));
        }
    }

    /// The wheel over the map: zoom, keeping the point under the cursor where it is.
    pub fn map_wheel(&mut self, amount: f32, x: f32, y: f32) {
        let r = self.city.rect;
        let (w, h) = ((r[2] - r[0]) as f64, (r[3] - r[1]) as f64);
        let (lx, ly) = ((x - r[0]) as f64 - w * 0.5, (y - r[1]) as f64 - h * 0.5);
        let before = self.city.center + DVec2::new(lx, -ly) * self.city.mpp;
        let k = (1.0 - amount as f64 * 0.15).clamp(0.6, 1.6);
        self.city.mpp = (self.city.mpp * k).clamp(0.25, self.max_mpp());
        let after = self.city.center + DVec2::new(lx, -ly) * self.city.mpp;
        self.city.center += before - after;
        if amount.abs() > 0.0 && (lx.abs() > 40.0 || ly.abs() > 40.0) {
            self.city.follow = false;
        }
    }

    fn max_mpp(&self) -> f64 {
        let (lo, hi) = self.city.extent;
        let span = (hi - lo).max_element().max(2000.0);
        let r = self.city.rect;
        span / ((r[2] - r[0]).max(200.0) as f64) * 1.2
    }

    /// Draw the city map and lay it over the picture.
    fn city(&mut self, renderer: &Renderer, scene: &mut Scene, f: &NavFrame) {
        self.atlas.begin_frame();
        let (sw, sh) = f.screen;
        let (w, h) = ((sw * 0.8).round(), (sh * 0.82).round());
        let (x0, y0) = (((sw - w) * 0.5).round(), ((sh - h) * 0.5).round());
        self.city.rect = [x0, y0, x0 + w, y0 + h];
        let (tw, th) = (w as u32, h as u32);
        if self.gpu.is_none() {
            self.gpu = Some(Gpu::new(&renderer.device, renderer.format(), map_samples(renderer.format()), self.atlas.size));
        }
        if self.city.target.map(|t| (t.1, t.2) != (tw, th)).unwrap_or(true) {
            if let Some((t, _, _)) = self.city.target.take() {
                renderer.free_texture(scene, t);
                scene.premultiplied.remove(&t);
            }
            let t = renderer.add_render_texture(scene, tw, th);
            scene.premultiplied.insert(t);
            self.city.target = Some((t, tw, th));
        }
        if self.city.follow {
            self.city.center = f.bus.truncate();
        }
        let s = (h / 760.0).clamp(0.95, 2.0) * f.ui_scale;
        let global = self.global.clone();
        let net = global.as_deref().or(f.traffic.map(|t| &t.net));
        // roads of the whole map (buffer 3), once per map version
        let mut roads_verts = None;
        if let Some(n) = net {
            let version = self.global_version * 1_000_000 + n.lanes.len() as u64;
            if self.city.roads.map(|r| r.0 != version).unwrap_or(true) {
                let (mut lo, mut hi) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
                let road_lanes = road_geometry(n);
                for l in &road_lanes {
                    for p in &l.points {
                        lo = lo.min(p.truncate());
                        hi = hi.max(p.truncate());
                    }
                }
                if lo.x == f64::MAX {
                    lo = f.bus.truncate();
                    hi = lo;
                }
                let anchor = (lo + hi) * 0.5;
                self.city.extent = (lo, hi);
                let rel = |q: DVec3| Vec3::new((q.x - anchor.x) as f32, (q.y - anchor.y) as f32, 0.0);
                let mut p = Painter::new();
                for pass in 0..2 {
                    for l in &road_lanes {
                        let pts = simplify(&l.points.iter().map(|q| rel(*q)).collect::<Vec<_>>(), 0.12);
                        if pass == 0 {
                            p.ribbon(&pts, l.width + 2.0, 2.4, Color::rgba(26, 26, 26, 1.0), true);
                        } else {
                            p.ribbon(&pts, l.width, 1.4, if l.main { ROAD_MAIN } else { ROAD }, true);
                        }
                    }
                }
                self.city.roads = Some((version, p.len(), anchor));
                roads_verts = Some(p.verts);
            }
        }
        let anchor = self.city.roads.map(|r| r.2).unwrap_or(f.bus.truncate());
        let rel = |q: DVec3| Vec3::new((q.x - anchor.x) as f32, (q.y - anchor.y) as f32, 0.0);
        // the whole route (buffer 4): the part driven grey, the rest by how busy it is,
        // with arrows about 90 pixels apart (built again when the zoom changes that much)
        let mut route_verts = None;
        let every = 2f64.powf((90.0 * s as f64 * self.city.mpp).log2().round()) as f32;
        let key = (self.route.version, self.jam_version, every.to_bits(), self.city.roads.map(|r| r.0).unwrap_or(0));
        if self.city.route.0 != key {
            let mut p = Painter::new();
            if let Some(n) = net {
                let r = &self.route;
                let done = r.progress.min(r.lanes.len());
                for &l in &r.lanes[..done] {
                    let Some(lane) = n.lanes.get(l) else { continue };
                    let pts: Vec<Vec3> = lane.points.iter().map(|q| rel(*q)).collect();
                    p.ribbon(&pts, lane.width.max(3.0) + 2.0, 5.0, DRIVEN, true);
                }
                let style = RouteStyle { extra_m: 0.0, min_px: 6.0, arrows: Some((every, f32::MAX)), max_len: f32::MAX, near: None };
                let bus_lane = r.lanes.get(done).map(|&l| lane_from_right(n, l, f.bus)).unwrap_or(0);
                build_route(&mut p, n, &r.lanes[done..], anchor, r.s, &self.route_jam, &style, bus_lane);
            }
            self.city.route = (key, p.len());
            route_verts = Some(p.verts);
        }
        // the view: north up, `mpp` metres a pixel
        let c = self.city.center - anchor;
        let (hw, hh) = (w as f64 * 0.5 * self.city.mpp, h as f64 * 0.5 * self.city.mpp);
        let proj = Mat4::orthographic_rh((c.x - hw) as f32, (c.x + hw) as f32, (c.y - hh) as f32, (c.y + hh) as f32, -1000.0, 1000.0);
        let vp = [0.0, 0.0, w, h];
        let world = Layer { view_proj: proj, viewport: vp, clip: [0.0, 0.0, w, h], radius: 10.0 * s, opacity: 1.0, px_scale: self.city.mpp as f32 };
        let to_screen = |q: DVec3| -> Vec2 {
            let d = q.truncate() - self.city.center;
            Vec2::new((w as f64 * 0.5 + d.x / self.city.mpp) as f32, (h as f64 * 0.5 - d.y / self.city.mpp) as f32)
        };
        let win = Rect::new(0.0, 0.0, w, h);
        let mut bg = Painter::new();
        // (the opacity setting the map's ground too; the roads, names and header stay solid)
        bg.rounded(win, 10.0 * s, Color::rgba(15, 15, 15, crate::ui::backdrop(self.opacity).min(1.0)));
        let n_bg = bg.len();
        // traffic: blue dots, the public transport in its colours with its line (as on the
        // small map)
        let mut dots = Painter::new();
        let mut lines: Vec<(DVec3, Color, String)> = Vec::new();
        if let Some(t) = f.traffic.filter(|_| self.show_ai) {
            for car in t.cars.iter().filter(|c| !c.gone) {
                let (color, line) = traffic_kind(car);
                if color == DOT {
                    dots.world_disc(rel(car.vehicle.position), 2.2, 3.4, Color::rgba(8, 8, 8, 0.9));
                    dots.world_disc(rel(car.vehicle.position), 1.5, 2.3, color);
                } else {
                    vehicle_icon(&mut dots, rel(car.vehicle.position), car.vehicle.heading, color, car.is_rail(), 7.5);
                }
                if let Some(l) = line {
                    lines.push((car.vehicle.position, color, l));
                }
            }
        }
        let n_dots = dots.len();
        // stops with their names, the bus, the frame and the header
        let mut ui = Painter::new();
        let n_stops = f.stops.len();
        // street names along their roads, where there is room (stops' names go first)
        let markers = spaced_markers(f.stops.iter().enumerate().map(|(k, st)| (k, to_screen(st.position))).filter(|(_, p)| win.contains(*p) && p.y > 50.0 * s), 20.0 * s);
        let mut taken: Vec<Rect> = markers.iter().map(|(_, p)| Rect::new(p.x - 9.0 * s, p.y - 9.0 * s, 18.0 * s, 18.0 * s)).collect();
        taken.push(Rect::new(0.0, 0.0, w, 50.0 * s));
        let mut stop_labels = Vec::new();
        for &(k, p) in &markers {
            if self.city.mpp >= 4.0 && k != 0 && k + 1 != n_stops { continue; }
            let st = &f.stops[k];
            let weight = if k == 0 { Weight::Bold } else { Weight::Medium };
            let name = format!("{}  {:02}:{:02}", st.name.trim(), (st.arrival / 3600.0) as i32 % 24, ((st.arrival % 3600.0) / 60.0) as i32);
            let name = self.fonts.fit(&name, 12.5 * s, weight, (300.0 * s).min(w * 0.3));
            let lw = self.fonts.width(&name, 12.5 * s, weight) + 14.0 * s;
            if let Some(r) = stop_label_rect(p, lw, s, win, &taken) {
                taken.push(r);
                stop_labels.push((k, name, r));
            }
        }
        // the public transport's line tags, above their dots, where there is room
        for (pos, color, l) in &lines {
            let p = to_screen(*pos);
            if !win.contains(p) {
                continue;
            }
            let px = 11.0 * s;
            let l = self.fonts.fit(l, px, Weight::Bold, 50.0 * s);
            let tw = self.fonts.width(&l, px, Weight::Bold) + 9.0 * s;
            let r = Rect::new(p.x - tw * 0.5, p.y - 22.0 * s, tw, 15.0 * s);
            if taken.iter().any(|o| o.x < r.right() && r.x < o.right() && o.y < r.bottom() && r.y < o.bottom()) {
                continue;
            }
            taken.push(r);
            ui.rounded(Rect::new(r.x - 1.0 * s, r.y - 1.0 * s, r.w + 2.0 * s, r.h + 2.0 * s), 4.5 * s, Color::rgba(10, 10, 10, 0.9));
            ui.rounded(r, 4.0 * s, *color);
            ui.text_in(&mut self.atlas, &self.fonts, &l, px, Weight::Bold, r, Align::Center, LINE_TEXT);
        }
        if let (Some(st), true) = (self.streets.clone(), self.city.mpp < 3.2 && self.global.is_some()) {
            let px = 12.0 * s;
            for (q, a, id) in &st.labels {
                let p = to_screen(q.extend(0.0));
                if !win.pad(60.0 * s, 30.0 * s).contains(p) {
                    continue;
                }
                let name = &st.names[*id as usize];
                let tw = self.fonts.width(name, px, Weight::Medium);
                // (upright: turned at most a quarter either way)
                let mut ang = -*a;
                if ang > std::f32::consts::FRAC_PI_2 {
                    ang -= std::f32::consts::PI;
                } else if ang <= -std::f32::consts::FRAC_PI_2 {
                    ang += std::f32::consts::PI;
                }
                // beside the road, not on it
                let side = Vec2::new(-ang.sin(), ang.cos()) * (px * 0.9 + 2.0 * s);
                let c = p + side;
                let (hx, hy) = ((ang.cos() * tw * 0.5).abs() + (ang.sin() * px * 0.6).abs(), (ang.sin() * tw * 0.5).abs() + (ang.cos() * px * 0.6).abs());
                let bb = Rect::new(c.x - hx, c.y - hy, hx * 2.0, hy * 2.0);
                if taken.iter().any(|t| rects_overlap(t, &bb)) {
                    continue;
                }
                taken.push(bb);
                let halo = Color::rgba(15, 15, 15, 0.9);
                for o in [Vec2::new(1.0, 0.0), Vec2::new(-1.0, 0.0), Vec2::new(0.0, 1.0), Vec2::new(0.0, -1.0)] {
                    ui.text_rotated(&mut self.atlas, &self.fonts, name, px, Weight::Medium, c + o * s, ang, halo);
                }
                ui.text_rotated(&mut self.atlas, &self.fonts, name, px, Weight::Medium, c, ang, STREET);
            }
        }
        for (k, p) in markers.into_iter().rev() {
            let next = k == 0;
            if self.city.mpp < 4.0 || next || k + 1 == n_stops {
                let badge = if next { 7.0 } else { 5.5 } * s;
                ui.circle(p, badge + 1.5 * s, Color::rgba(8, 8, 8, 0.95));
                let fill = if next { Color::rgba(45, 116, 205, 1.0) } else if k + 1 == n_stops { ROUTE } else { Color::rgba(76, 91, 112, 0.98) };
                ui.circle(p, badge, fill);
                ui.icon(&mut self.atlas, "directions_bus", p, if next { 11.5 } else { 9.5 } * s, TEXT);
            } else {
                ui.circle(p, 5.0 * s, Color::rgba(12, 12, 12, 0.95));
                ui.circle(p, 3.2 * s, TEXT_DIM);
            }
        }
        for (k, name, r) in stop_labels {
            ui.rounded(r, 4.0 * s, Color::rgba(12, 12, 12, 0.85));
            ui.text_in(&mut self.atlas, &self.fonts, &name, 12.5 * s, if k == 0 { Weight::Bold } else { Weight::Medium }, r.pad(6.0 * s, 0.0), Align::Left, if k == 0 { TEXT } else { TEXT_DIM });
        }
        // the other players: an arrow the way they face and their name (as on the small map)
        for pl in &f.players {
            let p = to_screen(pl.position);
            if !win.contains(p) || p.y < 44.0 * s {
                continue;
            }
            arrow(&mut ui, p, (pl.heading as f32).to_radians(), 8.5 * s, 1.3, Color::rgba(10, 10, 10, 0.9), PLAYER);
            let px = 11.0 * s;
            let name = self.fonts.fit(&pl.name, px, Weight::Bold, 140.0 * s);
            let tw = self.fonts.width(&name, px, Weight::Bold) + 9.0 * s;
            let r = Rect::new(p.x - tw * 0.5, p.y - 27.0 * s, tw, 15.0 * s);
            if taken.iter().any(|o| rects_overlap(o, &r)) {
                continue;
            }
            taken.push(r);
            ui.rounded(Rect::new(r.x - 1.0 * s, r.y - 1.0 * s, r.w + 2.0 * s, r.h + 2.0 * s), 4.5 * s, Color::rgba(10, 10, 10, 0.9));
            ui.rounded(r, 4.0 * s, PLAYER);
            ui.text_in(&mut self.atlas, &self.fonts, &name, px, Weight::Bold, r, Align::Center, LINE_TEXT);
        }
        arrow(&mut ui, to_screen(f.bus), (f.heading as f32).to_radians(), 10.0 * s, 1.3, Color::rgba(10, 10, 10, 0.9), TEXT);
        // header: the line and where it goes, the next stop; buttons on the right
        // (opaque: the route and the stops showed through behind its text)
        let head = Rect::new(0.0, 0.0, w, 44.0 * s);
        ui.rect(head, Color::rgba(18, 18, 18, 0.96));
        let pad = 16.0 * s;
        let wd = words(f.language);
        let title = match (&f.line, &f.terminus) {
            (Some(l), Some(t)) => format!("{}  ›  {}", l.trim(), t.trim()),
            _ => wd.no_duty.to_string(),
        };
        let title_w = ui.text_in(&mut self.atlas, &self.fonts, &title, 15.0 * s, Weight::Bold, Rect::new(pad, head.y, w * 0.4, head.h), Align::Left, TEXT);
        if let Some(st) = f.stops.first() {
            let d = self.next_dist.map(|d| if d >= 1000.0 { format!("{:.1} km", d / 1000.0) } else { format!("{:.0} m", d) }).unwrap_or_default();
            let t = format!("{}  ·  {}  ·  {:02}:{:02}", st.name.trim(), d, (st.arrival / 3600.0) as i32 % 24, ((st.arrival % 3600.0) / 60.0) as i32);
            ui.text_in(&mut self.atlas, &self.fonts, &t, 13.5 * s, Weight::Medium, Rect::new(pad + title_w + 24.0 * s, head.y, w * 0.45, head.h), Align::Left, TEXT_DIM);
        }
        self.city.buttons.clear();
        let bs = 30.0 * s;
        let mut bx = w - pad - bs;
        for (icon, id) in [("close", 3u8), ("zoom_out", 2), ("zoom_in", 1), ("my_location", 0)] {
            let r = Rect::new(bx, head.y + (head.h - bs) * 0.5, bs, bs);
            ui.rounded(r, 5.0 * s, if id == 0 && self.city.follow { Color::rgba(60, 60, 60, 1.0) } else { Color::rgba(34, 34, 34, 1.0) });
            ui.icon(&mut self.atlas, icon, r.center(), 18.0 * s, TEXT);
            self.city.buttons.push((r, id));
            bx -= bs + 8.0 * s;
        }
        // a scale bar, bottom left
        let nice = [10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0];
        let metres = nice.iter().copied().find(|m| m / self.city.mpp > 70.0 * s as f64).unwrap_or(5000.0);
        let len = (metres / self.city.mpp) as f32;
        let by = h - 22.0 * s;
        ui.rect(Rect::new(pad, by, len, 2.0 * s), TEXT_DIM);
        ui.text(&mut self.atlas, &self.fonts, &if metres >= 1000.0 { format!("{:.0} km", metres / 1000.0) } else { format!("{metres:.0} m") }, 12.0 * s, Weight::Medium, Vec2::new(pad + len + 8.0 * s, by + 4.0 * s), Align::Left, TEXT_DIM);
        ui.rounded_border(win, 10.0 * s, 1.0, Color::WHITE.alpha(0.08));

        // --- to the GPU
        let (tex, _, _) = self.city.target.unwrap();
        let Some(view) = renderer.texture_view(scene, tex) else { return };
        let (Some(gpu), device, queue) = (self.gpu.as_mut(), &renderer.device, &renderer.queue) else { return };
        if let Some(v) = roads_verts {
            gpu.upload(device, queue, 3, &v);
        }
        if let Some(v) = route_verts {
            gpu.upload(device, queue, 4, &v);
        }
        let mut all = bg.verts;
        all.extend(dots.verts);
        let n_ui = all.len() as u32;
        all.extend(ui.verts);
        gpu.upload(device, queue, 5, &all);
        gpu.upload_atlas(queue, &mut self.atlas);
        let flat = Layer::flat([0.0, 0.0, w, h], 10.0 * s, 1.0);
        let layers = [flat, world];
        let roads_n = self.city.roads.map(|r| r.1).unwrap_or(0);
        let draws = [
            Draw { buffer: 5, range: 0..n_bg, layer: 0, texture: 0 },
            Draw { buffer: 3, range: 0..roads_n, layer: 1, texture: 0 },
            Draw { buffer: 4, range: 0..self.city.route.1, layer: 1, texture: 0 },
            Draw { buffer: 5, range: n_bg..n_bg + n_dots, layer: 1, texture: 0 },
            Draw { buffer: 5, range: n_ui..all.len() as u32, layer: 0, texture: 0 },
        ];
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("city map") });
        gpu.render(device, queue, &mut enc, &view, (tw, th), Some(wgpu::Color::TRANSPARENT), &layers, &draws);
        queue.submit([enc.finish()]);
        let _ = n_dots;
        scene.overlays.push((tex, [x0, y0, x0 + w, y0 + h]));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn centre_panel_map_click_and_drag_use_window_coordinates() {
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        n.origin_x = 1920.0;
        n.panel_rect = [1930.0, 600.0, 2290.0, 890.0];
        n.panel_room = [1240.0, 610.0];
        assert!(!n.over_panel(100.0, 700.0));
        assert!(n.over_panel(2020.0, 700.0));
        n.panel_press(2020.0, 700.0);
        n.panel_move(2630.0, 405.0);
        assert_eq!(n.at, Some([0.5, 0.5]));
        n.city.open = true;
        n.city.rect = [2100.0, 100.0, 3700.0, 900.0];
        n.city.center = DVec2::new(100.0, 200.0);
        n.city.mpp = 2.0;
        assert_eq!(n.map_point(2900.0, 500.0), Some(n.city.center));
        assert_eq!(n.map_point(1000.0, 500.0), None);
    }
    /// A press and a small wobble is a click (the city map), a longer move drags the
    /// navigator, and the place reads back from the setting it is saved as (#940).
    #[test]
    fn the_navigator_is_dragged_by_the_mouse() {
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        assert!(n.at.is_none());
        n.panel_rect = [10.0, 600.0, 370.0, 890.0];
        n.panel_room = [1240.0, 610.0];
        n.panel_press(100.0, 700.0);
        assert!(!n.panel_move(103.0, 702.0));
        assert_eq!(n.panel_release(), Some(false));
        n.panel_press(100.0, 700.0);
        assert!(n.panel_move(100.0 + 610.0, 700.0 - 295.0));
        assert_eq!(n.panel_release(), Some(true));
        let at = n.at.unwrap();
        assert!((at[0] - 0.5).abs() < 1e-3 && (at[1] - 0.5).abs() < 1e-3, "{at:?}");
        let back = placed_at(&n.placement()).unwrap();
        assert!((back[0] - 0.5).abs() < 1e-3 && (back[1] - 0.5).abs() < 1e-3);
        // (dragged past the window's edge: held inside it)
        n.panel_press(100.0, 700.0);
        n.panel_move(5000.0, -5000.0);
        assert_eq!(n.at, Some([1.0, 0.0]));
        assert_eq!(placed_at("bottom-right"), None);
    }

    /// With the on-screen controls the navigator stands in the top middle, under the
    /// information bar when that is on - and where a finger dragged it, once it has been
    /// (#1138), as the mouse places it on a computer; the bar does not move a navigator in a
    /// corner it does not reach (#1164).
    #[test]
    fn the_navigator_keeps_where_it_was_dragged_on_a_phone_and_clear_of_the_information_bar() {
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        let (screen, size) = ((1280.0, 720.0), (300.0, 250.0));
        assert_eq!(n.panel_origin(screen, size, true, None), (490.0, 13.0));
        assert_eq!(n.panel_origin(screen, size, true, Some([311.0, 20.0, 898.0, 69.0])), (490.0, 76.0));
        assert_eq!(n.panel_origin(screen, size, false, Some([311.0, 20.0, 898.0, 69.0])), (13.0, 457.0));
        n.corner = "top-left".into();
        assert_eq!(n.panel_origin(screen, size, false, Some([330.0, 20.0, 950.0, 69.0])), (13.0, 13.0));
        assert_eq!(n.panel_origin(screen, size, false, Some([300.0, 20.0, 980.0, 69.0])), (13.0, 76.0));
        // a finger's drag: there, on the phone as on the computer
        n.panel_rect = [490.0, 13.0, 790.0, 263.0];
        n.panel_press(600.0, 100.0);
        assert!(n.panel_move(600.0 - 490.0, 100.0 + 457.0));
        assert_eq!(n.panel_release(), Some(true));
        assert_eq!(n.panel_origin(screen, size, true, None), (0.0, 470.0));
        assert_eq!(n.panel_origin(screen, size, false, None), (0.0, 470.0));
    }

    use super::*;
    use omsi_sim::traffic::Lane;

    #[test]
    fn stop_request_icon_reserves_space_only_while_active() {
        let mut atlas = Atlas::new(256);
        for scale in [0.75, 1.0, 2.0] {
            // Both the next-stop row and the free-drive row leave room for the symbol.
            for height in [20.0, 46.0] {
                let row = Rect::new(11.0 * scale, 300.0 * scale, 300.0 * scale, height * scale);
                for requested in [false, true, false] {
                    let mut ui = Painter::new();
                    let text = stop_request_icon(&mut ui, &mut atlas, requested, row, scale);
                    assert_eq!((text.x, text.y, text.h), (row.x, row.y, row.h));
                    if requested {
                        assert!(!ui.verts.is_empty(), "the stop sign must render");
                        assert!(ui.verts.iter().all(|v| v.color == STOP_REQUEST.0));
                        assert!(ui.verts.iter().all(|v| v.pos[0] > text.right() && v.pos[0] <= row.right()));
                        assert!((text.w - (row.w - 38.0 * scale)).abs() < 0.01);
                    } else {
                        assert!(ui.verts.is_empty());
                        assert_eq!(text.w, row.w);
                    }
                }
            }
        }
    }

    fn straight(a: (f64, f64), b: (f64, f64)) -> Lane {
        omsi_sim::traffic::LaneBuilder::polyline(vec![DVec3::new(a.0, a.1, 0.0), DVec3::new(b.0, b.1, 0.0)], LaneKind::Street, 3.0)
    }

    /// A square block: the route goes north then east; the bus turned west by mistake at
    /// the first corner and must be led round the block back onto the route.
    #[test]
    fn a_way_back_joins_the_route_ahead() {
        let lanes = vec![
            straight((0.0, 0.0), (0.0, 100.0)),     // 0 north (route)
            straight((0.0, 100.0), (100.0, 100.0)), // 1 east (route)
            straight((100.0, 100.0), (200.0, 100.0)), // 2 east (route)
            straight((0.0, 100.0), (-100.0, 100.0)), // 3 west (the mistake)
            straight((-100.0, 100.0), (-100.0, 200.0)), // 4 north
            straight((-100.0, 200.0), (100.0, 200.0)),  // 5 east
            straight((100.0, 200.0), (100.0, 100.0)),   // 6 south, back to the route's corner
        ];
        let mut net = Network { lanes, ..Default::default() };
        net.link(1.5);
        // (square corners: joined by hand, `link` wants a junction's curves)
        for (a, n) in [(0, vec![1, 3]), (1, vec![2]), (3, vec![4]), (4, vec![5]), (5, vec![6]), (6, vec![2])] {
            net.lanes[a].next = n;
        }
        let route = [0usize, 1, 2];
        let (path, join) = way_back(&net, DVec3::new(-40.0, 100.0, 0.0), 270.0, &route[1..], 6000.0).expect("a way");
        assert_eq!(path.first(), Some(&3));
        assert!(path.contains(&6), "{path:?}");
        // round the block back to the route's second corner: it goes on east on lane 2
        assert_eq!(route[1..][join], 2, "{path:?} join {join}");
        assert!(heading_vec(90.0).x > 0.99);
    }

    #[test]
    fn projection_puts_the_look_at_point_in_the_middle() {
        let view = Mat4::look_at_rh(Vec3::new(0.0, -50.0, 80.0), Vec3::ZERO, Vec3::Z);
        let l = Layer::world(view, 0.7, [0.0, 0.0, 400.0, 300.0], [0.0; 4], 0.0, 1.0);
        let p = project(l.view_proj, [0.0, 0.0, 400.0, 300.0], Vec3::ZERO).unwrap();
        assert!((p - Vec2::new(200.0, 150.0)).length() < 0.5, "{p}");
        let ahead = project(l.view_proj, [0.0, 0.0, 400.0, 300.0], Vec3::new(0.0, 30.0, 0.0)).unwrap();
        assert!(ahead.y < 150.0, "ahead is up the picture: {ahead}");
    }

    #[test]
    fn editor_only_lanes_do_not_become_gps_roads() {
        let visible = straight((0.0, 0.0), (100.0, 0.0));
        let mut hidden = straight((0.0, 30.0), (100.0, 30.0));
        hidden.invisible = true;
        let net = Network { lanes: vec![visible.clone(), hidden], ..Default::default() };
        let drawn = visible_road_lanes(&net);
        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0].1.points, visible.points);
    }

    #[test]
    fn separate_asphalt_meshes_corroborate_invisible_traffic_splines() {
        let mut on_road = straight((0.0, 0.0), (100.0, 0.0));
        on_road.invisible = true;
        let mut helper = straight((0.0, 25.0), (100.0, 25.0));
        helper.invisible = true;
        let mut bridge = straight((0.0, 0.0), (100.0, 0.0));
        bridge.points.iter_mut().for_each(|p| p.z = 8.0);
        bridge.invisible = true;
        let mut net = Network { lanes: vec![on_road, helper, bridge], ..Default::default() };
        let surface = (vec![DVec3::ZERO, DVec3::new(100.0, 0.0, 0.0)], 8.0);
        confirm_road_surfaces(&mut net, &[surface.clone()]);
        assert!(!net.lanes[0].invisible);
        assert!(net.lanes[1].invisible);
        assert!(net.lanes[2].invisible, "asphalt below a bridge must not corroborate its paths");
        assert_eq!(road_geometry(&net).len(), 1, "surface evidence must not draw a second road");
    }

    #[test]
    fn adjacent_spline_lanes_form_carriageways_without_filling_the_median() {
        let mut lanes = Vec::new();
        for (path, offset) in [-10.0f32, -7.0, 7.0, 10.0].into_iter().enumerate() {
            let mut lane = straight((offset as f64, 0.0), (offset as f64, 100.0));
            lane.source = 1;
            lane.key = Some(LaneKey { tile: (0, 0), id: 42, path: path as u16 });
            lane.offset = offset;
            if offset < 0.0 { lane.points.reverse(); lane.reversed = true; }
            lanes.push(lane);
        }
        let net = Network { lanes, ..Default::default() };
        let roads = road_geometry(&net);
        assert_eq!(roads.len(), 2);
        assert_eq!(roads[0].width, 6.0);
        assert_eq!(roads[1].width, 6.0);
        assert_eq!(roads[0].points, vec![DVec3::new(-8.5, 0.0, 0.0), DVec3::new(-8.5, 100.0, 0.0)]);
        assert_eq!(roads[1].points[0].x, 8.5);
        // Display geometry must not replace or move the graph used by routing.
        assert_eq!(net.lanes.len(), 4);
        assert_eq!(net.lanes[0].points[0].y, 100.0);
    }

    #[test]
    fn opposite_directions_draw_once_even_when_reverse_is_first() {
        let mut a = straight((0.0, 0.0), (0.0, 100.0));
        a.key = Some(LaneKey { tile: (0, 0), id: 42, path: 0 });
        let mut b = a.clone();
        b.points.reverse();
        b.reversed = true;
        let net = Network { lanes: vec![b, a], ..Default::default() };
        assert_eq!(visible_road_lanes(&net).len(), 1);
    }

    #[test]
    fn junction_helpers_between_roads_remain_connected() {
        let mut lanes = vec![straight((0.0, 0.0), (0.0, 40.0)), straight((0.0, 40.0), (0.0, 50.0)), straight((0.0, 50.0), (0.0, 60.0)), straight((0.0, 60.0), (0.0, 100.0)), straight((30.0, 0.0), (30.0, 100.0))];
        lanes[0].next = vec![1]; lanes[1].next = vec![2]; lanes[2].next = vec![3];
        for i in [1, 2, 4] { lanes[i].invisible = true; }
        let mut net = Network { lanes, ..Default::default() };
        confirm_road_surfaces(&mut net, &[]);
        assert!(!net.lanes[1].invisible && !net.lanes[2].invisible);
        assert!(net.lanes[4].invisible);
        assert_eq!(road_geometry(&net).len(), 4);
    }

    #[test]
    fn placement_gaps_are_bridged_only_where_the_graph_connects() {
        let mut lanes = vec![straight((0.0, 0.0), (0.0, 40.0)), straight((0.0, 41.0), (0.0, 80.0)), straight((2.0, 41.0), (2.0, 80.0))];
        lanes[0].next = vec![1];
        let roads = road_geometry(&Network { lanes, ..Default::default() });
        assert_eq!(roads.len(), 4);
        assert_eq!(roads[3].points, vec![DVec3::new(0.0, 40.0, 0.0), DVec3::new(0.0, 41.0, 0.0)]);
    }

    #[test]
    fn paved_areas_without_driving_paths_do_not_draw_streets() {
        let mut net = Network::default();
        confirm_road_surfaces(&mut net, &[(vec![DVec3::ZERO, DVec3::new(100.0, 0.0, 0.0)], 20.0)]);
        assert!(road_geometry(&net).is_empty());
    }

    #[test]
    fn stacked_paths_on_one_spline_are_not_merged() {
        let mut a = straight((0.0, 0.0), (0.0, 100.0));
        a.source = 1;
        a.key = Some(LaneKey { tile: (0, 0), id: 42, path: 0 });
        let mut b = a.clone();
        b.key.as_mut().unwrap().path = 1;
        b.points.iter_mut().for_each(|p| p.z = 8.0);
        let roads = road_geometry(&Network { lanes: vec![a, b], ..Default::default() });
        assert_eq!(roads.len(), 2);
        assert_eq!(roads[0].points[0].z, 0.0);
        assert_eq!(roads[1].points[0].z, 8.0);
    }

    #[test]
    fn crowded_stop_markers_and_labels_do_not_overlap() {
        let p = Vec2::new(200.0, 200.0);
        let markers = spaced_markers([(0, p), (1, p + Vec2::X * 5.0), (2, p + Vec2::X * 35.0)], 20.0);
        assert_eq!(markers.iter().map(|m| m.0).collect::<Vec<_>>(), vec![0, 2]);
        let win = Rect::new(0.0, 0.0, 600.0, 400.0);
        let first = stop_label_rect(p, 150.0, 1.0, win, &[]).unwrap();
        let second = stop_label_rect(p + Vec2::X * 35.0, 150.0, 1.0, win, &[first]).unwrap();
        assert!(!rects_overlap(&first, &second));
        let edge = stop_label_rect(Vec2::new(590.0, 200.0), 150.0, 1.0, win, &[]).unwrap();
        assert!(edge.right() < 600.0);
        assert!(stop_label_rect(p, 150.0, 1.0, win, &[win]).is_none());
    }

    #[test]
    fn a_vehicle_icon_is_whole_triangles_of_edge_body_and_windscreen() {
        let mut bus = Painter::new();
        vehicle_icon(&mut bus, Vec3::ZERO, 90.0, Color::WHITE, false, 7.0);
        // an edge and a body of six corners (four triangles each), a windscreen of four (two)
        assert_eq!(bus.verts.len(), (4 + 4 + 2) * 3);
        // a tram is longer, not made of more
        let mut tram = Painter::new();
        vehicle_icon(&mut tram, Vec3::ZERO, 0.0, Color::WHITE, true, 7.0);
        assert_eq!(tram.verts.len(), bus.verts.len());
    }
}
