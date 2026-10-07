//! The route navigator: a small tilted 3D map in a corner of the screen, after the Route
//! Advisor of Euro Truck Simulator 2, made for a bus driver's duty.
//!
//! * The map turns with the bus and zooms out with speed; the camera looks over the bus
//!   from behind and above, so the road ahead fills the picture.
//! * Roads are drawn from the traffic network's lanes (dark casing, grey surface); the
//!   trip's route from the first stop to the last lies on them with arrows, coloured by
//!   how busy each stretch is (blue empty, green, yellow, red, dark red jammed), and its
//!   stops as the stop sign the player chose (`stop_signs`: the next larger, the last ringed).
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
//! * The player's own pins (`nav_pins`), set on the city map with its pin tool or a
//!   right-click: on a free drive the route goes through them to a destination of the
//!   player's choosing; on a duty they are a diversion for the navigation alone, back onto
//!   the trip's route after the last. Followed, recalculated and drawn as a trip's route is.
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

use crate::companion::Stage;
use crate::traffic::Traffic;

// --- colours (sRGB) -------------------------------------------------------------------

// Omsi-Hub's night blue, half transparent, calm (its overlay's glass)
const NAV_REDRAW_S: f32 = 1.0 / 30.0;
const PANEL: Color = Color::rgba(20, 26, 38, 0.70);
// (the bars under the texts darken whatever the opacity setting leaves of the panel: at a
// third the cab showed through behind the next stop)
const BAR: Color = Color::rgba(6, 9, 16, 0.55);
// (the launcher's map picture draws with these too, so the two maps cannot look apart)
pub(crate) const ROAD_CASING: Color = Color::rgba(30, 30, 30, 0.9);
pub(crate) const ROAD: Color = Color::rgba(92, 92, 92, 1.0);
pub(crate) const ROAD_MAIN: Color = Color::rgba(112, 112, 112, 1.0);
pub(crate) const ROUTE: Color = Color::rgba(214, 48, 40, 1.0);
const DOT: Color = Color::rgba(70, 140, 255, 1.0);
/// The public transport's dots: trolleybuses, buses, trams (their line tags in the same
/// colours: `stop_signs::draw_chip`).
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
// (punctuality in Omsi-Hub's colours, as the duty board writes it)
const LATE: Color = crate::nav_duty::LATE_INK;
const EARLY: Color = crate::nav_duty::EARLY_INK;
const ON_TIME: Color = crate::nav_duty::ON_TIME_INK;
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
    /// The player's duty (its trips, legs and progress), for the duty board under the map
    /// and the duty sheet beside the city map (`nav_duty`).
    pub duty: Option<&'a crate::schedule::PlayerDuty>,
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
        "NLD" | "NL" => Words { kmh: "km/u", days: ["Ma", "Di", "Wo", "Do", "Vr", "Za", "Zo"], off_route: "Buiten de route", rerouting: "Route wordt herberekend", recalculated: "Route herberekend", jam: "File", slow: "Langzaam verkeer", no_duty: "Vrij rijden", last_stop: "Eindhalte", on_time: "op tijd" },
        "UKR" | "UK" | "UA" => Words { kmh: "км/год", days: ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Нд"], off_route: "Поза маршрутом", rerouting: "Перебудова маршруту", recalculated: "Маршрут перебудовано", jam: "Затор", slow: "Повільний рух", no_duty: "Вільна поїздка", last_stop: "Кінцева", on_time: "за графіком" },
        "POL" | "PL" => Words { kmh: "km/h", days: ["Pn", "Wt", "Śr", "Cz", "Pt", "So", "Nd"], off_route: "Poza trasą", rerouting: "Wyznaczanie nowej trasy", recalculated: "Trasa wyznaczona ponownie", jam: "Korek", slow: "Wolny ruch", no_duty: "Jazda swobodna", last_stop: "Przystanek końcowy", on_time: "punktualnie" },
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
    /// The route ends this far along its last lane (the player's own destination, `nav_pins`),
    /// not at that lane's end.
    end: Option<f32>,
}

/// The key of the route to the player's own destination on a free drive (no trip's key is
/// ever this).
const DEST_KEY: &str = "\u{1}destination";

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
    /// The duty board under the map (`nav_duty`; Shift+N cycles map, map and board, off).
    pub schedule: bool,
    /// The board is wanted under the map (the `nav_board` setting, and its handle on the
    /// panel): it opens by itself on a duty and is a step of Shift+N; off, the map alone.
    pub board: bool,
    /// The board's handle was pressed: the setting to keep (taken by [`Navigator::take_board`]).
    board_changed: Option<bool>,
    /// The panel's size as the player dragged it, as shares of the window's height (the
    /// `nav_rect` setting's); None: its own size, from the interface's and `size`.
    pub custom: Option<[f32; 2]>,
    /// The place or the size changed in the game: `nav_rect` to keep ([`Navigator::take_rect`]).
    rect_changed: bool,
    /// How the small navigator was laid out last (its own pixels): the handle's place.
    layout: Option<crate::nav_panel::Layout>,
    /// What the mouse is over on the small navigator (the handle lit, the edge marked).
    panel_hover: Option<HoverPart>,
    /// The window (physical pixels) and the interface's pixels a point, when last placed.
    window: [f32; 2],
    unit: f32,
    /// A duty has been seen: the board opened by itself for it once.
    duty_seen: bool,
    /// Smoothed speed (m/s) for the time to the next stop.
    speed_avg: f32,
    pub opacity: f32,
    pub corner: String,
    /// Where the navigator was dragged to (#940): its top-left corner as a share of the
    /// room the window leaves it across and down (`navigator_corner = at x,y`); None: the
    /// corner.
    pub at: Option<[f32; 2]>,
    /// A press on the navigator: where it was, the panel as it was then, what it took hold of
    /// (the panel, or edges to size it by), and whether it has moved far enough to be a drag
    /// rather than a click.
    panel_drag: Option<PanelGrab>,
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
    /// The navigator's own size on top of the interface's (`nav_scale`): the small panel and
    /// the city map's panels, texts and buttons. Ctrl + the wheel over either changes it, and
    /// the city map's - and + (the caller keeps it: [`Navigator::take_resized`]).
    pub size: f32,
    /// The wheel's notches towards the next step of the size (a touchpad gives parts of one).
    size_wheel: f32,
    /// The size changed in the game, not yet kept as the setting.
    resized: Option<f32>,
    /// When the size last changed (the navigator's clock): it says so for a moment.
    size_at: f32,
    /// While the duty waits to be signed for, the small navigator is the sign-on page from
    /// edge to edge (`nav_signon`, [`crate::nav_signon::Room::Panel`]): the page as it is laid
    /// out and where a press on it does what. None: the map.
    signing: Option<PanelPage>,
    /// The pairing QR code ([`crate::companion::pair_qr`]), and the address, code and time it
    /// was asked for (it is asked again when they change, and now and then).
    qr: (String, f32, Option<std::sync::Arc<crate::nav_signon::Qr>>),
    /// The player's own destination and waypoints, set on the city map (`nav_pins`): on a free
    /// drive the route goes to them, on a duty they are a diversion for the navigation only.
    pins: crate::nav_pins::Pins,
    /// The timetable's stops by their map objects, for the pins' names (given once).
    stop_names: Option<Vec<(i64, String)>>,
}

/// A press held on the small navigator: where the cursor was, the panel then (x0, y0, x1,
/// y1, the window's pixels without the origin), what it holds, and whether it has moved yet.
#[derive(Debug, Clone, Copy)]
struct PanelGrab {
    from: [f32; 2],
    rect: [f32; 4],
    grip: crate::nav_panel::Grip,
    moved: bool,
}

/// The small navigator as the sign-on page: the page as it fits, where a press does what
/// (the panel's pixels), and the stage it was laid out at.
struct PanelPage {
    fit: crate::nav_signon::Fitted,
    hits: Vec<(Rect, crate::nav_signon::Action)>,
    stage: Stage,
}

/// Each notch of Ctrl + the wheel, and each press of the city map's - and +, changes the
/// navigator's size by this much (the setting's own steps).
const SIZE_STEP: f32 = 0.05;
/// How long the new size shows after it changed (s).
const SIZE_NOTE: f32 = 1.4;

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

pub(crate) fn angle_diff(a: f64, b: f64) -> f64 {
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
            board: true,
            board_changed: None,
            custom: None,
            rect_changed: false,
            layout: None,
            panel_hover: None,
            window: [0.0; 2],
            unit: 1.0,
            duty_seen: false,
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
            // (Omsi-Hub's typeface, as the launcher: the duty board is Omsi-Hub's overlay)
            fonts: Fonts::hanken(),
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
            size: 1.0,
            size_wheel: 0.0,
            resized: None,
            size_at: f32::MIN,
            signing: None,
            qr: (String::new(), f32::MIN, None),
            pins: crate::nav_pins::Pins::default(),
            stop_names: None,
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
                    n.extend(lanes, 1.5);
                    n.build_grid();
                }
            }
            None => {
                let mut n = Network { lanes, ..Default::default() };
                n.link(1.5);
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
        // (the player's own pins: while a diversion of this trip runs its route stays, and the
        // trip's lanes are what it goes back onto; a free drive's destination ends when a duty
        // begins, a diversion with its trip)
        if self.pins.diverts(key) {
            if self.pins.rest.is_empty() && !lanes.is_empty() {
                self.pins.rest = lanes;
                self.pins.dirty = true;
            }
            self.route.key = key.to_string();
            self.route.complete = complete;
            self.route.generation = generation;
            return;
        }
        if !self.pins.list.is_empty() || self.pins.diversion() {
            self.pins.finish();
        }
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
        self.route.end = None;
        if !same_trip {
            self.route.progress = 0;
            self.route.on_route = false;
            self.route.off_for = 0.0;
        }
        self.route.version += 1;
    }

    /// No duty: nothing to follow but the player's own destination (`nav_pins`). A diversion
    /// ends with its trip.
    pub fn clear_route(&mut self) {
        if self.pins.diversion() {
            self.pins.finish();
        }
        if self.route.key == DEST_KEY && !self.pins.list.is_empty() {
            return;
        }
        if !self.route.key.is_empty() || !self.route.lanes.is_empty() {
            self.route = Route { version: self.route.version + 1, ..Route::default() };
        }
    }


    /// Where the bus is along the route, and a way back when it left it.
    fn follow(&mut self, f: &NavFrame) {
        let global = self.global.clone();
        let Some(net) = global.as_deref().or(f.traffic.map(|t| &t.net)) else { return };
        // the player's own pins: what the city map asked, the one reached, a new plan
        self.pins_step(net, f);
        let pinned = self.pins.steers();
        let r = &mut self.route;
        if r.lanes.is_empty() && pinned {
            // (no way through the pins from here - no road near the bus yet: tried again now
            // and then)
            r.retry_in -= f.dt;
            if r.retry_in <= 0.0 {
                r.retry_in = REROUTE_EVERY * 2.0;
                self.plan_pins(net, f, true);
            }
            return;
        }
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
        // (before the bus first reached the route: only the stretch up to the next stop; the
        // pins' route begins where the bus is)
        let (from, to) = match (r.joined || pinned, stop_at) {
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
        if pinned {
            // (off the pins' route: a new one from the bus through the pins still ahead)
            self.plan_pins(net, f, true);
            return;
        }
        let base = r.progress.min(r.lanes.len() - 1);
        // targets: the route from where the bus left it up to the next stop (joining it
        // later would skip that stop), or on for 120 lanes when no stop is ahead
        let (lo, hi) = match stop_at {
            Some(k) => (k.saturating_sub(150).max(base), k + 1),
            None => (base, (base + 120).min(r.lanes.len())),
        };
        // (from the depot to the first stop may be a long way; back onto the route is not)
        let way = way_back(net, f.bus, f.heading, &r.lanes[lo..hi], if r.joined { 6000.0 } else { 30_000.0 });
        if way.is_none() && omsi_cfg::env::var_os("OMSI_DEBUG_NAV").is_some() {
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

    /// The timetable's stops have not been given yet (they name the player's pins).
    pub fn wants_stop_names(&self) -> bool {
        self.stop_names.is_none()
    }

    /// The timetable's stops by their map objects (`Schedule::stop_names`): a pin near one is
    /// called after it.
    pub fn set_stop_names(&mut self, names: impl IntoIterator<Item = (i64, String)>) {
        self.stop_names = Some(names.into_iter().collect());
    }

    /// The player's pins, once a frame: what the city map asked of them, the one the bus
    /// reached, and a new route through them when they changed.
    fn pins_step(&mut self, net: &Network, f: &NavFrame) {
        use crate::nav_pins::Note;
        self.pins.tick(f.dt);
        for op in std::mem::take(&mut self.pins.ops) {
            self.pin_op(net, f, op);
        }
        // at the destination: the route and the pins go after a moment
        if let Some(t) = self.pins.arrived.as_mut() {
            *t -= f.dt;
            if *t <= 0.0 {
                self.end_pins(net, f);
            }
            return;
        }
        if let Some(Note::Reached(n)) = self.pins.pass(f.bus) {
            log::info!("navigator: via {n} of the player's pins reached");
            if self.pins.list.is_empty() {
                self.diversion_done(net, f);
            }
        }
        if self.pins.dirty {
            self.plan_pins(net, f, false);
        }
    }

    /// Do what the city map asked of the pins.
    fn pin_op(&mut self, net: &Network, f: &NavFrame, op: crate::nav_pins::Op) {
        use crate::nav_pins::{self as pins, Note, Op, Pin};
        match op {
            Op::Add(p) => {
                // (set while the last destination still says "arrived": a new drive)
                if self.pins.arrived.is_some() {
                    self.end_pins(net, f);
                }
                let Some(at) = pins::snap(net, p, pins::SNAP_REACH) else {
                    self.pins.say(Note::NoRoad);
                    return;
                };
                if self.pins.list.is_empty() {
                    // a new set: a diversion of the trip whose route this is, or a free drive's
                    // destination; a diversion goes back onto the trip's route ahead
                    let key = &self.route.key;
                    self.pins.trip = (!key.is_empty() && key != DEST_KEY).then(|| key.clone());
                    self.pins.passed = 0;
                    let r = &self.route;
                    self.pins.rest = if self.pins.trip.is_some() && !r.provisional { r.lanes[r.progress.min(r.lanes.len())..].to_vec() } else { Vec::new() };
                }
                self.pins.made += 1;
                let name = self.pin_name(net, at.at, self.pins.made);
                let pin = Pin { at: at.at, name, n: self.pins.made };
                let diversion = self.pins.diversion();
                match pins::add(&mut self.pins.list, pin, diversion) {
                    Some(_) => self.pins.dirty = true,
                    None => self.pins.say(Note::Full),
                }
            }
            Op::Move(k, to, from) => {
                let Some(pin) = self.pins.list.get(k).cloned() else { return };
                match pins::snap(net, to, pins::SNAP_REACH) {
                    Some(at) => {
                        let name = self.pin_name(net, at.at, pin.n);
                        self.pins.list[k] = Pin { at: at.at, name, n: pin.n };
                        self.pins.dirty = true;
                    }
                    None => {
                        self.pins.list[k].at = from;
                        self.pins.say(Note::NoRoad);
                    }
                }
            }
            Op::Remove(k) => {
                if k < self.pins.list.len() {
                    self.pins.list.remove(k);
                    self.pins.dirty = true;
                }
                if self.pins.list.is_empty() {
                    self.end_pins(net, f);
                }
            }
            Op::Shift(k, up) => {
                if pins::shift(&mut self.pins.list, k, up) {
                    self.pins.dirty = true;
                }
            }
            Op::Clear => self.end_pins(net, f),
        }
    }

    /// A pin's name at `at`: the stop nearest within 200 m, else the street, else "Point n".
    fn pin_name(&self, net: &Network, at: DVec2, n: u32) -> String {
        let stops: Vec<(DVec2, String)> = self.stop_names.iter().flatten().filter_map(|(id, name)| self.stop_pos.get(id).map(|p| (p.truncate(), name.clone()))).collect();
        let street = crate::nav_pins::snap(net, at, 15.0).and_then(|s| self.street_of(s.lane));
        crate::nav_pins::name(at, &stops, street, n)
    }

    /// A route through the pins from the bus (`recalculated`: because the bus left it), for
    /// the navigator to follow as it follows a trip's. A diversion keeps the trip's route when
    /// no way leads through its pins.
    fn plan_pins(&mut self, net: &Network, f: &NavFrame, recalculated: bool) {
        use crate::nav_pins::Note;
        self.pins.dirty = false;
        if !self.pins.steers() {
            return;
        }
        let Some(lane) = start_lane(net, f.bus, f.heading) else { return };
        let (s, d) = net.lanes[lane].nearest_point(f.bus).unwrap_or((0.0, f64::MAX));
        let at: Vec<DVec2> = self.pins.list.iter().map(|p| p.at).collect();
        let rest = Some(self.pins.rest.as_slice()).filter(|r| self.pins.diversion() && !r.is_empty());
        let plan = crate::nav_pins::plan(net, (lane, s), &at, rest);
        if plan.marks.iter().any(Option::is_none) && !recalculated {
            self.pins.say(Note::NoWay);
        }
        if plan.lanes.is_empty() {
            // (no way from here to the first pin: a diversion keeps the trip's route, a free
            // drive has none for now - tried again now and then)
            if !self.pins.diversion() && !self.route.lanes.is_empty() {
                self.route = Route { key: DEST_KEY.to_string(), version: self.route.version + 1, retry_in: REROUTE_EVERY * 2.0, ..Route::default() };
            }
            self.pins.marks = plan.marks;
            self.pins.join = None;
            return;
        }
        log::info!("navigator: {} lanes through {} of the player's pins{}", plan.lanes.len(), at.len(), if plan.join.is_some() { ", back onto the trip's route" } else { "" });
        self.pins.marks = plan.marks;
        self.pins.join = plan.join;
        let r = &mut self.route;
        if !self.pins.diversion() {
            r.key = DEST_KEY.to_string();
            r.complete = true;
        }
        // (a bus standing off the roads - a depot - is not on it yet: no "recalculated" then)
        let near = d < 16.0;
        if recalculated && r.joined && near {
            r.note = 4.0;
        }
        r.lanes = plan.lanes;
        r.progress = 0;
        r.s = plan.s0;
        r.end = plan.end;
        r.on_route = near;
        r.joined = near;
        r.approach = false;
        r.provisional = false;
        r.off_for = 0.0;
        r.retry_in = REROUTE_EVERY;
        r.version += 1;
        // (the distances on the card and in the header at once)
        self.dist_t = 0.0;
    }

    /// How far along the route each pin is, now.
    fn pin_distances(&mut self, net: &Network) {
        let r = &self.route;
        self.pins.dist = self.pins.marks.iter().map(|m| m.and_then(|m| crate::nav_pins::along(net, &r.lanes, r.progress, r.s, m))).collect();
    }

    /// The pins gone (cleared, or the destination reached a moment ago): a free drive's route
    /// goes with them; a diversion's goes back to the trip's, unless the bus is on it again.
    fn end_pins(&mut self, net: &Network, f: &NavFrame) {
        if self.pins.diversion() {
            let rest = std::mem::take(&mut self.pins.rest);
            let rejoined = self.pins.join.is_some_and(|j| self.route.progress >= j);
            if !rejoined && !rest.is_empty() {
                self.back_to_trip(net, f, rest);
            }
        } else if self.route.key == DEST_KEY {
            self.route = Route { version: self.route.version + 1, ..Route::default() };
        }
        self.pins.finish();
    }

    /// A diversion's last via was reached: on along the trip's route it rejoins - or, when no
    /// way led back onto it, the trip's route again (the bus finds its way back to it).
    fn diversion_done(&mut self, net: &Network, f: &NavFrame) {
        let rest = std::mem::take(&mut self.pins.rest);
        if self.pins.join.is_none() && !rest.is_empty() {
            self.back_to_trip(net, f, rest);
        }
        self.pins.finish();
    }

    /// The trip's route `rest` again: where the bus is on it, else a way back to it is looked
    /// for at once (as after leaving it).
    fn back_to_trip(&mut self, net: &Network, f: &NavFrame, rest: Vec<usize>) {
        let at = crate::nav_pins::locate(net, &rest, f.bus, f.heading);
        let r = &mut self.route;
        r.lanes = rest;
        r.end = None;
        r.joined = true;
        r.approach = false;
        r.provisional = false;
        r.version += 1;
        match at {
            Some((k, s)) => {
                r.progress = k;
                r.s = s;
                r.on_route = true;
                r.off_for = 0.0;
            }
            None => {
                r.progress = 0;
                r.s = 0.0;
                r.on_route = false;
                r.off_for = OFF_ROUTE_AFTER;
                r.retry_in = 0.0;
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
                // (the player's pins stay where they are; their way, on the new network - and
                // a diversion goes back onto the trip's route as the new network has it)
                if !self.pins.list.is_empty() {
                    self.pins.rest.clear();
                    self.pins.marks.clear();
                    self.pins.join = None;
                    self.pins.dirty = true;
                }
            }
        }
        // stops whose tiles are not loaded: their place from the map
        if omsi_cfg::env::var_os("OMSI_DEBUG_NAV").is_some() && self.time < 0.15 {
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
        if omsi_cfg::env::var_os("OMSI_DEBUG_NAV").is_some() && (self.time % 1.0) < f.dt {
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
            if let Some(n) = net {
                self.pin_distances(n);
            }
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
            self.signing = None;
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
        // (the interface's pixels a point here: the smallest panel and its scale's range)
        let unit = base * f.ui_scale / crate::nav_panel::WIDTH;
        self.unit = unit;
        self.window = [sw, sh];
        // (the navigator's own size on top of the interface's; the cockpit's display is the
        // bus's, as large as it is)
        let size = if self.cockpit_display { 1.0 } else { self.size };
        let margin = (sh * 0.018).max(10.0).round();
        // (with the on-screen controls the corners are theirs: the top middle)
        let touch = crate::platform::touch_controls();
        // the size the player dragged it to (by its corners and edges): what is in it is laid
        // out for that shape; else its own, as wide as the interface makes it
        let custom = self
            .custom
            .filter(|_| !self.cockpit_display && !touch)
            .map(|c| crate::nav_panel::clamp_size(c[0] * sh, c[1] * sh, self.min_size(), [sw, sh]));
        let pw = match custom {
            Some((w, _)) => w,
            None => (base * f.ui_scale * size).min((sh * 0.7).max(300.0)).min((sw * 0.9).max(300.0)).round(),
        };
        // the duty board under the map (Shift+N's second step, the handle on the bar): as
        // many stops ahead as there is room for, four at most
        let duty = f.duty.and_then(|d| crate::nav_duty::DutyState::of(d, f.time));
        // (on a duty the board opens by itself, once - Omsi-Hub's duty overlay was there
        // whenever a duty ran - unless the player wants the map alone; Shift+N and the
        // handle put it away)
        if duty.is_some() && !self.duty_seen {
            self.duty_seen = true;
            self.schedule = self.board;
        }
        // (until the duty is signed for the whole navigator is the sign-on page - Omsi-Hub's
        // duty panel was "sign on first" and nothing else until then; signed, the map and the
        // board. Shift+N's map alone leaves it out, as it leaves out the board)
        self.signing = None;
        if self.schedule && duty.is_some() {
            let companion = crate::companion::state();
            if crate::nav_signon::waiting(&companion) {
                let (s, room) = match custom {
                    Some((cw, ch)) => (crate::nav_panel::scale(cw, ch, unit * size, false).0, ch),
                    None => (pw / crate::nav_panel::WIDTH, sh - 2.0 * margin),
                };
                self.signing = Some(self.panel_page(&companion, duty.as_ref(), f.time, pw, s, room, custom.is_some()));
            }
        }
        let handle = duty.is_some() || self.schedule;
        let (ph, board) = if let Some(p) = self.signing.as_ref() {
            self.layout = None;
            (p.fit.height.round(), Vec::new())
        } else if let Some((cw, ch)) = custom {
            // (the navigator's own size on top: its texts as large as they fit in the shape)
            let (s, beside) = crate::nav_panel::scale(cw, ch, unit * size, self.schedule);
            let (rows, bh) = if self.schedule { crate::nav_duty::board_within(duty.as_ref(), s, crate::nav_panel::board_room(ch, s, beside)) } else { (Vec::new(), 0.0) };
            let shown = !rows.is_empty();
            self.layout = Some(crate::nav_panel::layout(cw, ch, s, shown.then_some(bh), beside, handle));
            (ch, rows)
        } else {
            let s = pw / crate::nav_panel::WIDTH;
            let map_h = (pw * 0.62).round();
            let bars = (crate::nav_panel::HEAD * s).round() + (crate::nav_panel::NEXT * s).round();
            let room = sh - 2.0 * margin - map_h - bars;
            let (rows, sched) = if self.schedule { crate::nav_duty::board_fitting(duty.as_ref(), s, room) } else { (Vec::new(), 0.0) };
            let ph = (map_h + bars + sched).round();
            self.layout = Some(crate::nav_panel::layout(pw, ph, s, (!rows.is_empty()).then_some(sched), false, handle));
            (ph, rows)
        };
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
            match self.layout {
                Some(lay) if self.signing.is_none() => self.draw(renderer, &view, (w, h), &lay, f, &board),
                _ => self.draw_signon(renderer, &view, (w, h)),
            }
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

    /// Draw the small navigator into its texture (`size` pixels), laid out as `lay`.
    fn draw(&mut self, renderer: &Renderer, target: &wgpu::TextureView, size: (u32, u32), lay: &crate::nav_panel::Layout, f: &NavFrame, board: &[crate::nav_duty::Row]) {
        let reach = self.map_zoom(lay) * 3.5 + 150.0;
        let vehicles = map_vehicles(f.traffic.filter(|_| self.show_ai), f.bus, reach);
        let p = self.paint_panel((size.0 as f32, size.1 as f32), lay, f, board, &vehicles);
        let (Some(gpu), device, queue) = (self.gpu.as_mut(), &renderer.device, &renderer.queue) else { return };
        if let Some(v) = p.roads {
            if let Some(r) = self.roads.as_mut() {
                r.verts = v.len();
            }
            gpu.upload(device, queue, 0, &v);
        }
        if let Some(v) = p.route {
            gpu.upload(device, queue, 1, &v);
        }
        let (n_bg, n_traffic) = (p.bg.len(), p.traffic.len());
        let n_world = n_traffic + p.vehicles.len();
        let mut all = p.bg.verts;
        all.extend(p.traffic.verts);
        all.extend(p.vehicles.verts);
        let n_ui_start = all.len() as u32;
        all.extend(p.ui.verts);
        gpu.upload(device, queue, 2, &all);
        gpu.upload_atlas(queue, &mut self.atlas);
        let clip_panel = [0.0, 0.0, size.0 as f32, size.1 as f32];
        // (the opacity setting is the background's: the map and the text stay solid)
        let flat = Layer::flat(clip_panel, p.radius, 1.0);
        let backdrop = Layer::flat(clip_panel, p.radius, if self.cockpit_display { self.opacity } else { crate::ui::backdrop(self.opacity).min(1.0) });
        let mut layers = [flat, p.map_layer, backdrop];
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

    /// The map camera's distance for the map's part of the panel: the eased zoom, made
    /// larger as the map is taller than the design's (a tall map shows more of the way, the
    /// things on it as large as ever).
    fn map_zoom(&self, lay: &crate::nav_panel::Layout) -> f64 {
        let design = 0.62 * crate::nav_panel::WIDTH * lay.s;
        self.zoom * (lay.map.h / design.max(1.0)).clamp(0.5, 3.0) as f64
    }

    /// The small navigator as painted, before it goes to the GPU (`size` pixels, laid out as
    /// `lay`), with the other `vehicles` on the map.
    fn paint_panel(&mut self, size: (f32, f32), lay: &crate::nav_panel::Layout, f: &NavFrame, board: &[crate::nav_duty::Row], vehicles: &[MapVehicle]) -> PanelPaint {
        let (pw, ph) = size;
        let s = lay.s;
        let wd = words(f.language);
        self.atlas.begin_frame();
        let panel = Rect::new(0.0, 0.0, pw, ph);
        let radius = 7.0 * s;
        let map = lay.map;
        let zoom = self.map_zoom(lay);
        let vp = [map.x, map.y, map.w, map.h.max(1.0)];

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
                if omsi_cfg::env::var_os("OMSI_DEBUG_NAV").is_some() {
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
        let look_at = f.bus.truncate() - anchor + fwd * zoom * 0.28;
        let pitch = PITCH.to_radians();
        let back = fwd * zoom * pitch.cos();
        let eye = DVec3::new(look_at.x - back.x, look_at.y - back.y, zoom * pitch.sin());
        let view = Mat4::look_at_rh(eye.as_vec3(), Vec3::new(look_at.x as f32, look_at.y as f32, 0.0), Vec3::Z);
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
                let style = RouteStyle { extra_m: -0.9, min_px: 4.0, arrows: Some((28.0, 900.0)), max_len: 12_000.0, near: Some((anchor, ROAD_RADIUS * 1.6)), end: self.route.end };
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
        // the other vehicles: small cars in blue, as the other drivers in ETS2; the public
        // transport as buses in its own colours (trolleybus, bus, tram), its line above it -
        // turned as they head, a dot with a tip where they would be too small to make out
        let mut cars = Painter::new();
        let mpp_here = zoom as f32 * map_layer.px_scale;
        paint_vehicles(&mut cars, vehicles, &rel, mpp_here, s);
        let mut lines: Vec<(DVec3, Color, String, [DVec3; 2])> = vehicles.iter().filter_map(|v| v.line.clone().map(|l| (v.at, v.color, l, v.drawn_ends(mpp_here, s)))).collect();

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
        // Stops as the stop sign the player chose (`stop_signs`): the next one larger, the
        // terminus ringed, the last two served faded behind the bus.
        let n_stops = f.stops.len();
        let signs = crate::stop_signs::style();
        let markers = spaced_markers(f.stops.iter().enumerate().filter_map(|(k, st)| project(vpm, vp, rel(st.position)).filter(|p| map.contains(*p)).map(|p| (k, p))), 20.0 * s);
        let served = crate::stop_signs::served(f.duty, &self.stop_pos);
        for q in served.iter().rev().take(2).filter_map(|q| project(vpm, vp, rel(*q))).filter(|p| map.contains(*p) && markers.iter().all(|(_, m)| m.distance(*p) >= 14.0 * s)) {
            crate::stop_signs::draw(&mut ui, signs, crate::stop_signs::Kind::Passed, q, 13.5 * s);
        }
        for (k, sp) in markers.into_iter().rev() {
            crate::stop_signs::draw(&mut ui, signs, crate::stop_signs::Kind::ahead(k, n_stops), sp, 13.5 * s);
        }
        // the player's own pins (`nav_pins`): the destination's flag, the vias numbered
        for k in (0..self.pins.list.len()).rev() {
            let Some(p) = project(vpm, vp, rel(self.pins.list[k].at.extend(0.0))).filter(|p| map.contains(*p)) else { continue };
            match self.pins.role(k) {
                crate::nav_pins::Role::Destination => crate::nav_pins::draw_destination(&mut ui, &mut self.atlas, p, 7.0 * s, self.pins.arrived.is_some(), 1.0),
                crate::nav_pins::Role::Via(n) => crate::nav_pins::draw_via(&mut ui, &mut self.atlas, &self.fonts, p, n, 7.0 * s, 1.0),
            }
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
        for (pos, color, l, ends) in &lines {
            let Some(p) = project(vpm, vp, rel(*pos)).filter(|p| map.contains(*p)) else { continue };
            let px = 9.5 * s;
            // (a rounded chip in its kind's colour, the duty's own line on its yellow plate -
            // above the bus's end that is higher on the map, not over the bus)
            let own = f.line.as_deref().is_some_and(|o| o.trim().eq_ignore_ascii_case(l.trim()));
            let l = self.fonts.fit(l, px, Weight::Bold, 40.0 * s);
            let w = self.fonts.width(&l, px, Weight::Bold) + crate::stop_signs::CHIP_PAD * s;
            let top = ends.iter().filter_map(|e| project(vpm, vp, rel(*e))).fold(p.y, |t, e| t.min(e.y));
            let r = Rect::new(p.x - w * 0.5, top.min(p.y - 6.0 * s) - 15.0 * s, w, 13.0 * s);
            if r.y < map.y || taken.iter().any(|o| o.x < r.right() && r.x < o.right() && o.y < r.bottom() && r.y < o.bottom()) {
                continue;
            }
            taken.push(r);
            crate::stop_signs::draw_chip(&mut ui, &mut self.atlas, &self.fonts, r, &l, px, *color, own, s);
        }
        // the bus: a plain white arrow
        if let Some(bp) = project(vpm, vp, rel(f.bus)) {
            let a = (angle_diff(self.cam_heading, f.heading) as f32).to_radians();
            arrow(&mut ui, bp, a, 9.0 * s, 1.25, Color::rgba(10, 10, 10, 0.8), TEXT);
        }

        // top bar: speed (and the limit) · line ……… game time
        let top = lay.head;
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

        // bottom bar: the next stop (or the player's own destination); its distance, the time
        // to it, the planned time and whether the bus is early or late
        let next = lay.next_text();
        self.paint_next(&mut ui, f, next, s, board);
        // (the board's handle at the bar's right end: a chevron - down to show the board,
        // up to put it away; beside the map, towards it)
        if let Some(h) = lay.handle {
            ui.rect(Rect::new(next.right(), lay.next.y, lay.next.right() - next.right(), lay.next.h), BAR);
            let on = self.schedule;
            let lit = self.panel_hover == Some(HoverPart::Handle);
            ui.rounded(h, 6.0 * s, Color::WHITE.alpha(if lit { 0.2 } else { 0.08 }));
            ui.rounded_border(h, 6.0 * s, 1.0, Color::WHITE.alpha(if lit { 0.35 } else { 0.16 }));
            let icon = match (on, lay.beside) {
                (true, true) => "chevron_right",
                (true, false) => "expand_less",
                (false, _) => "expand_more",
            };
            ui.icon(&mut self.atlas, icon, h.center(), h.w * 0.8, if on { TEXT_DIM } else { crate::nav_duty::NOW });
        }
        // the duty board (Omsi-Hub's duty overlay): the trip, its stops behind and ahead
        // with their times, and what comes after it - under the bar, or beside the map
        if let Some(area) = lay.board.filter(|_| self.schedule && !board.is_empty()) {
            if lay.beside {
                ui.rect(Rect::new(area.x, area.y, 1.0, area.h), Color::WHITE.alpha(0.1));
            }
            crate::nav_duty::draw_board(&mut crate::nav_duty::Pen { p: &mut ui, atlas: &mut self.atlas, fonts: &self.fonts }, board, area, s);
        }
        // (the size just changed with Ctrl + the wheel: the new one, over the map)
        self.size_note(&mut ui, Vec2::new(map.center().x, map.y + 22.0 * s), s);
        // (the mouse over an edge or a corner: a mark along it, where a drag sizes the panel)
        if let Some(HoverPart::Grip(crate::nav_panel::Grip::Size { left, right, top, bottom })) = self.panel_hover {
            let (t, c) = (3.0 * s.max(1.0), crate::nav_duty::NOW.alpha(0.9));
            let (len_x, len_y) = ((pw * 0.25).min(80.0 * s), (ph * 0.25).min(80.0 * s));
            let (x0, x1) = if left { (0.0, len_x) } else if right { (pw - len_x, pw) } else { (pw * 0.5 - len_x, pw * 0.5 + len_x) };
            let (y0, y1) = if top { (0.0, len_y) } else if bottom { (ph - len_y, ph) } else { (ph * 0.5 - len_y, ph * 0.5 + len_y) };
            if top || bottom {
                ui.rect(Rect::new(x0, if top { 0.0 } else { ph - t }, x1 - x0, t), c);
            }
            if left || right {
                ui.rect(Rect::new(if left { 0.0 } else { pw - t }, y0, t, y1 - y0), c);
            }
        }

        PanelPaint { bg, traffic: dy, vehicles: cars, ui, roads: road_verts, route: route_verts, map_layer, radius }
    }
}

/// The small navigator as painted, before it goes to the GPU: the background, the jams on
/// the map, the other vehicles (both in the map's world, relative to the roads' anchor),
/// what lies over it, the roads' and the route's meshes when they were built again, the map's
/// view and the panel's corner radius.
struct PanelPaint {
    bg: Painter,
    traffic: Painter,
    vehicles: Painter,
    ui: Painter,
    roads: Option<Vec<omsi_ui::Vertex>>,
    route: Option<Vec<omsi_ui::Vertex>>,
    map_layer: Layer,
    radius: f32,
}

/// What the mouse is over on the small navigator: the board's handle, or what a press
/// there takes hold of.
#[derive(Debug, Clone, Copy, PartialEq)]
enum HoverPart {
    Handle,
    Grip(crate::nav_panel::Grip),
}

/// Another vehicle as the maps draw it: the middle of its body (world), its heading, its
/// length and width (m), its colour (cars blue, the public transport by kind), whether it is
/// public transport (drawn as a bus) and the line it shows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MapVehicle {
    pub at: DVec3,
    pub heading: f64,
    pub len: f32,
    pub width: f32,
    pub color: Color,
    pub bus: bool,
    pub line: Option<String>,
}

/// The traffic's vehicles within `reach` metres of `bus`, as the maps draw them.
fn map_vehicles(traffic: Option<&Traffic>, bus: DVec3, reach: f64) -> Vec<MapVehicle> {
    let Some(t) = traffic else { return Vec::new() };
    t.cars
        .iter()
        .filter(|c| !c.gone && (c.vehicle.position - bus).truncate().length() <= reach)
        .map(|c| {
            let (color, line) = traffic_kind(c);
            let h = c.vehicle.heading.to_radians();
            let (front, rear) = (c.state.front.max(0.5), c.state.rear.max(0.5));
            // (the body's middle: its origin is where its front and rear are measured from)
            let mid = c.vehicle.position.truncate() + DVec2::new(h.sin(), h.cos()) * ((front - rear) * 0.5) as f64;
            MapVehicle { at: mid.extend(c.vehicle.position.z), heading: c.vehicle.heading, len: front + rear, width: (c.half_width * 2.0).max(1.2), color, bus: color != DOT, line }
        })
        .collect()
}

/// The other vehicles onto the map `p` (its world, `rel` giving a world point relative to its
/// anchor), at `mpp` metres a pixel where they stand and scale `s`: each turned as it heads, at
/// its own size or - when that is small - a little larger, with a dark outline; a dot with a tip
/// where even that would be too small.
fn paint_vehicles(p: &mut Painter, vehicles: &[MapVehicle], rel: &dyn Fn(DVec3) -> Vec3, mpp: f32, s: f32) {
    use crate::nav_panel::{self as np, Tone};
    let outline = Color::rgba(8, 8, 8, 0.9);
    // (the cars first: the buses over them)
    for pass in [false, true] {
        for v in vehicles.iter().filter(|v| v.bus == pass) {
            let at = rel(v.at);
            match vehicle_size(v, mpp, s) {
                Some((_, width)) => {
                    // (metres, but at least `min_px` long: a scale of at least that a metre;
                    // wide enough to make out as a bus or a car)
                    let px_m = min_len(v, s) / v.len.max(1.0);
                    let parts = np::vehicle_shape(v.len, width, v.bus);
                    let edge = (1.1 * mpp).clamp(0.25, 0.9);
                    p.world_shape(at, &np::turned(&np::grown(&parts[0].1, edge), v.heading as f32), 1.0, px_m, outline);
                    for (tone, pts) in &parts {
                        let c = match tone {
                            Tone::Body => v.color,
                            Tone::Glass => Color::rgba(18, 24, 34, 0.9),
                            Tone::Roof => v.color.lighten(0.28),
                            Tone::Joint => v.color.darken(0.45),
                        };
                        p.world_shape(at, &np::turned(pts, v.heading as f32), 1.0, px_m, c);
                    }
                }
                None => {
                    let (disc, tip) = np::dot_shape();
                    let r = if v.bus { 3.4 } else { 2.6 } * s;
                    let turn = |pts: &[Vec2]| np::turned(pts, v.heading as f32);
                    p.world_shape(at, &turn(&np::grown(&tip, 0.3)), 0.0, r, outline);
                    p.world_shape(at, &turn(&disc), 0.0, r + 1.2 * s, outline);
                    p.world_shape(at, &turn(&disc), 0.0, r, v.color);
                    p.world_shape(at, &turn(&tip), 0.0, r, v.color);
                }
            }
        }
    }
}

/// The least length a vehicle is drawn at as its shape (pixels at scale `s`).
fn min_len(v: &MapVehicle, s: f32) -> f32 {
    (if v.bus { 15.0 } else { 9.0 }) * s
}

/// How a vehicle is drawn at `mpp` metres a pixel and scale `s`: as its shape - how many
/// times its own size, and its width (m) as drawn - or None: as a dot.
fn vehicle_size(v: &MapVehicle, mpp: f32, s: f32) -> Option<(f32, f32)> {
    let min_px = min_len(v, s);
    (crate::nav_panel::glyph(v.len, mpp, min_px) == crate::nav_panel::Glyph::Shape).then(|| {
        let k = (min_px / v.len.max(1.0) * mpp).max(1.0);
        let min_w = if v.bus { 6.0 } else { 5.0 } * s;
        (k, v.width.max(min_w * mpp / k))
    })
}

impl MapVehicle {
    /// Its front and its back as drawn at `mpp` metres a pixel and scale `s` (a dot: its
    /// middle), for the line's tag to stand above.
    fn drawn_ends(&self, mpp: f32, s: f32) -> [DVec3; 2] {
        let half = vehicle_size(self, mpp, s).map(|(k, _)| self.len * k * 0.5).unwrap_or(0.0) as f64;
        let h = self.heading.to_radians();
        let ahead = DVec3::new(h.sin(), h.cos(), 0.0) * half;
        [self.at + ahead, self.at - ahead]
    }
}

impl Navigator {
    /// The small navigator's bottom bar `bottom` at scale `s`: the next stop with its distance,
    /// the time to it and its planned time, and how the bus stands against the timetable
    /// (unless the duty board under it - `board` - says so on its chip); on a diversion of the
    /// player's own, that and its next via in place of the way to the stop. On a free drive
    /// the player's own destination where the next stop goes, with how far, how long and when
    /// there, and the next via on the right; else what the drive is. A note of the pins' or of
    /// the route's takes the second line for a moment.
    fn paint_next(&mut self, ui: &mut Painter, f: &NavFrame, bottom: Rect, s: f32, board: &[crate::nav_duty::Row]) {
        use crate::nav_pins as pins;
        let wd = words(f.language);
        let pad = 11.0 * s;
        ui.rect(bottom, BAR);
        let dest = self.pins.list.last().filter(|_| !self.pins.diversion());
        let stop_row = if f.stops.is_empty() && dest.is_none() {
            Rect::new(bottom.x + pad, bottom.y, bottom.w - 2.0 * pad, bottom.h)
        } else {
            Rect::new(bottom.x + pad, bottom.y + 4.0 * s, bottom.w - 2.0 * pad, 20.0 * s)
        };
        let stop_row = stop_request_icon(ui, &mut self.atlas, f.stop_requested, stop_row, s);
        let y2 = Rect::new(bottom.x + pad, bottom.y + 24.0 * s, bottom.w - 2.0 * pad, 18.0 * s);
        // (the pins' note first - a via reached, arrived - then the route's)
        let note: Option<(String, Color)> = if let Some((n, _)) = self.pins.note {
            Some((n.text(), if n.good() { ON_TIME } else { WARN }))
        } else if self.route.note > 0.0 {
            Some((wd.recalculated.to_string(), ON_TIME))
        } else if self.route.joined && !self.route.lanes.is_empty() && !self.route.on_route && self.route.off_for > OFF_ROUTE_AFTER {
            // (no way back found after a while: it says so instead of recalculating for ever)
            Some(((if self.route.off_for < OFF_ROUTE_AFTER + 20.0 { wd.rerouting } else { wd.off_route }).to_string(), WARN))
        } else {
            None
        };
        // what the traffic on the route ahead costs, as the module promises: a jam from a
        // minute on, slow traffic from half of one
        let jam_note = (self.jam_cost >= 30.0).then(|| {
            let what = if self.jam_cost >= 60.0 { wd.jam } else { wd.slow };
            (format!("{what} +{:.0} min", (self.jam_cost / 60.0).max(1.0).round()), if self.jam_cost >= 60.0 { LATE } else { WARN })
        });
        // the next via of the player's own, and how far along the route it is
        let via = self.pins.list.first().filter(|_| self.pins.steers() && (self.pins.diversion() || self.pins.list.len() > 1)).map(|_| {
            let label = self.pins.role(0).label();
            match self.pins.dist.first().copied().flatten() {
                Some(d) => format!("{label} · {}", pins::distance_text(d)),
                None => label,
            }
        });
        match (f.stops.first(), dest) {
            (Some(st), _) => {
                let name = if f.stops.len() == 1 { format!("{} · {}", st.name.trim(), wd.last_stop) } else { st.name.trim().to_string() };
                ui.text_in(&mut self.atlas, &self.fonts, &name, 13.5 * s, Weight::Bold, stop_row, Align::Left, TEXT);
                let mut parts = Vec::new();
                // (a diversion of the player's own: where it goes first; the timetable's next
                // stop stays the next stop)
                let diverted = via.is_some() && self.pins.diversion();
                if let Some(v) = via.clone().filter(|_| diverted) {
                    parts.push(omsi_ui::tr("Diversion").into_owned());
                    parts.push(v);
                } else if let Some(d) = self.next_dist {
                    parts.push(pins::distance_text(d));
                    parts.push(pins::eta_text(pins::eta(d, self.speed_avg)));
                }
                parts.push(format!("{:02}:{:02}", (st.arrival / 3600.0) as i32 % 24, ((st.arrival % 3600.0) / 60.0) as i32));
                let line2 = parts.join("  ·  ");
                match (&note, &jam_note) {
                    (Some((t, c)), _) => {
                        ui.text_in(&mut self.atlas, &self.fonts, t, 12.5 * s, Weight::Medium, y2, Align::Left, *c);
                    }
                    (None, Some((t, c))) => {
                        ui.text_in(&mut self.atlas, &self.fonts, &format!("{line2}  ·  {t}"), 12.5 * s, Weight::Medium, y2, Align::Left, *c);
                    }
                    (None, None) => {
                        ui.text_in(&mut self.atlas, &self.fonts, &line2, 12.5 * s, Weight::Medium, y2, Align::Left, if diverted { crate::nav_duty::NOW } else { TEXT_DIM });
                    }
                }
                // (OMSI 2's punctuality, as the duty board's; the board's chip says it when it
                // is there)
                let on_board = board.iter().any(|r| matches!(r, crate::nav_duty::Row::Head { .. }));
                if let Some(d) = f.delay.filter(|_| !on_board) {
                    let (txt, c) = match crate::nav_duty::punctuality(d) {
                        crate::nav_duty::Punctuality::Late => (crate::nav_duty::offset(d), LATE),
                        crate::nav_duty::Punctuality::Early => (crate::nav_duty::offset(d), EARLY),
                        crate::nav_duty::Punctuality::OnTime => (wd.on_time.to_string(), ON_TIME),
                    };
                    ui.text_in(&mut self.atlas, &self.fonts, &txt, 12.5 * s, Weight::Bold, y2, Align::Right, c);
                }
            }
            (None, Some(pin)) => {
                // the player's own destination, where a duty's next stop is: its flag and name;
                // how far, how long and when there
                let arrived = self.pins.arrived.is_some();
                ui.icon(&mut self.atlas, if arrived { "check" } else { "sports_score" }, Vec2::new(stop_row.x + 8.0 * s, stop_row.center().y), 16.0 * s, if arrived { ON_TIME } else { pins::PIN_LIGHT });
                let name_r = Rect::new(stop_row.x + 21.0 * s, stop_row.y, (stop_row.w - 21.0 * s).max(0.0), stop_row.h);
                ui.text_in(&mut self.atlas, &self.fonts, &pin.name, 13.5 * s, Weight::Bold, name_r, Align::Left, TEXT);
                let total = self.pins.dist.last().copied().flatten().filter(|_| self.pins.dist.len() == self.pins.list.len());
                let line2 = total
                    .map(|d| {
                        let secs = pins::eta(d, self.speed_avg);
                        format!("{}  ·  {}  ·  {}", pins::distance_text(d), pins::eta_text(secs), crate::nav_duty::hhmm(f.time + secs))
                    })
                    .unwrap_or_default();
                match (&note, &jam_note) {
                    (Some((t, c)), _) => {
                        ui.text_in(&mut self.atlas, &self.fonts, t, 12.5 * s, Weight::Medium, y2, Align::Left, *c);
                    }
                    (None, jam) => {
                        // (the next via on the right, the way to the destination left of it)
                        let mut left = y2;
                        if let Some(v) = via.as_deref() {
                            let vw = self.fonts.width(v, 12.5 * s, Weight::Bold) + 2.0 * s;
                            ui.text_in(&mut self.atlas, &self.fonts, v, 12.5 * s, Weight::Bold, y2, Align::Right, pins::PIN_LIGHT);
                            left.w = (left.w - vw - 10.0 * s).max(0.0);
                        }
                        let (t, c) = match jam {
                            Some((j, c)) if !line2.is_empty() => (format!("{line2}  ·  {j}"), *c),
                            _ => (line2, TEXT_DIM),
                        };
                        ui.text_in(&mut self.atlas, &self.fonts, &t, 12.5 * s, Weight::Medium, left, Align::Left, c);
                    }
                }
            }
            (None, None) => {
                let t = f.terminus.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| wd.no_duty.to_string());
                ui.text_in(&mut self.atlas, &self.fonts, &t, 13.0 * s, Weight::Medium, stop_row, Align::Left, TEXT_DIM);
            }
        }
    }
}

// --- the sign-on page as the navigator, and its size ----------------------------------------

impl Navigator {
    /// The pairing QR code for a device ([`crate::companion::pair_qr`]): asked again when the
    /// address or the code changes, and every few seconds (the companion may make it later).
    fn pair_qr(&mut self, st: &crate::companion::CompanionState) -> Option<std::sync::Arc<crate::nav_signon::Qr>> {
        let key = qr_key(st);
        if self.qr.0 != key || (self.time - self.qr.1).abs() > 3.0 {
            let qr = crate::companion::pair_qr().and_then(|(side, dark)| crate::nav_signon::Qr::new(side, dark)).map(std::sync::Arc::new);
            // (the same code again keeps its `Arc`: the page is no different)
            let qr = match (&self.qr.2, qr) {
                (Some(old), Some(new)) if **old == *new => Some(old.clone()),
                (_, new) => new,
            };
            self.qr = (key, self.time, qr);
        }
        self.qr.2.clone()
    }

    /// The small navigator as the sign-on page, `pw` pixels wide at scale `s` and at most
    /// `room` tall - all of it when `fill` (a panel the player sized: its height is theirs).
    #[allow(clippy::too_many_arguments)]
    fn panel_page(&mut self, st: &crate::companion::CompanionState, duty: Option<&crate::nav_duty::DutyState>, time: f64, pw: f32, s: f32, room: f32, fill: bool) -> PanelPage {
        use crate::nav_signon as signon;
        self.city.phone.follow(st);
        if let Some(r) = self.city.phone.wants(st) {
            crate::companion::request(r);
        }
        let qr = self.pair_qr(st);
        let page = signon::page_in(st, &self.city.phone, duty, time, signon::Room::Panel, qr.as_ref());
        let mut fit = signon::fit_panel(page, pw, s, room, &self.fonts);
        if fill {
            fit.height = room.round();
        }
        // (a page that is taller than the window scrolls: the wheel over it)
        self.city.phone.scroll = self.city.phone.scroll.clamp(0.0, signon::panel_scroll_max(&fit)).round();
        let hits = signon::panel_hits(&fit, self.city.phone.scroll);
        PanelPage { fit, hits, stage: st.stage }
    }

    /// The small navigator as the sign-on page, painted for a panel `pw` x `ph`: its
    /// background (drawn at the opacity setting's), its head, and its items (to be clipped to
    /// the part under the head, which they scroll in: the last).
    fn paint_signon(&mut self, pw: f32, ph: f32) -> Option<(Painter, Painter, Painter, Rect)> {
        use crate::nav_signon as signon;
        let sp = self.signing.take()?;
        let panel = Rect::new(0.0, 0.0, pw, ph);
        let scroll = self.city.phone.scroll;
        let mut bg = Painter::new();
        bg.rounded(panel, 7.0 * (pw / 360.0), if self.cockpit_display { Color::rgba(10, 10, 10, 1.0) } else { crate::nav_duty::SHEET.alpha(0.86) });
        let mut ui = Painter::new();
        signon::draw_panel_head(&mut crate::nav_duty::Pen { p: &mut ui, atlas: &mut self.atlas, fonts: &self.fonts }, &sp.fit, panel, scroll);
        let mut list = Painter::new();
        signon::draw_panel_items(&mut crate::nav_duty::Pen { p: &mut list, atlas: &mut self.atlas, fonts: &self.fonts }, &sp.fit, panel, scroll);
        self.size_note(&mut ui, Vec2::new(pw * 0.5, ph * 0.5), sp.fit.s);
        let view = signon::panel_view(&sp.fit, panel);
        self.signing = Some(sp);
        Some((bg, ui, list, view))
    }

    /// Draw the small navigator as the sign-on page into its texture: its background (the
    /// opacity setting's, as the map's), the page's head and items.
    fn draw_signon(&mut self, renderer: &Renderer, target: &wgpu::TextureView, size: (u32, u32)) {
        let (pw, ph) = (size.0 as f32, size.1 as f32);
        self.atlas.begin_frame();
        let Some((bg, ui, list, view)) = self.paint_signon(pw, ph) else { return };
        let radius = 7.0 * (pw / 360.0);
        let (Some(gpu), device, queue) = (self.gpu.as_mut(), &renderer.device, &renderer.queue) else { return };
        let n_bg = bg.len();
        let mut all = bg.verts;
        all.extend(list.verts);
        let n_list = all.len() as u32;
        all.extend(ui.verts);
        gpu.upload(device, queue, 2, &all);
        gpu.upload_atlas(queue, &mut self.atlas);
        let clip = [0.0, 0.0, pw, ph];
        let backdrop = Layer::flat(clip, radius, if self.cockpit_display { self.opacity } else { crate::ui::backdrop(self.opacity).min(1.0) });
        let mut layers = [Layer::flat(clip, radius, 1.0), backdrop, Layer::flat([view.x, view.y, view.right(), view.bottom()], radius, 1.0)];
        for l in layers.iter_mut() {
            l.opacity *= self.shown;
        }
        let draws = [
            Draw { buffer: 2, range: 0..n_bg, layer: 1, texture: 0 },
            Draw { buffer: 2, range: n_bg..n_list, layer: 2, texture: 0 },
            Draw { buffer: 2, range: n_list..all.len() as u32, layer: 0, texture: 0 },
        ];
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("navigator sign-on") });
        gpu.render(device, queue, &mut enc, target, size, Some(wgpu::Color::TRANSPARENT), &layers, &draws);
        queue.submit([enc.finish()]);
    }

    /// The size, for a moment after it changed: "110 %" in a dark pill centred on `at`.
    fn size_note(&mut self, ui: &mut Painter, at: Vec2, s: f32) {
        let t = self.time - self.size_at;
        if !(0.0..SIZE_NOTE).contains(&t) {
            return;
        }
        let a = ((SIZE_NOTE - t) / 0.3).min(1.0);
        let text = size_text(self.size);
        let px = 14.0 * s;
        let w = self.fonts.width(&text, px, Weight::Bold) + 40.0 * s;
        let r = Rect::new(at.x - w * 0.5, at.y - 15.0 * s, w, 30.0 * s);
        ui.rounded(r, 15.0 * s, Color::rgba(10, 12, 18, 0.9 * a));
        ui.icon(&mut self.atlas, "zoom_in", Vec2::new(r.x + 16.0 * s, r.center().y), 16.0 * s, TEXT.alpha(a));
        ui.text_in(&mut self.atlas, &self.fonts, &text, px, Weight::Bold, Rect::new(r.x + 28.0 * s, r.y, r.w - 34.0 * s, r.h), Align::Center, TEXT.alpha(a));
    }

    /// Make the navigator larger (`steps` > 0) or smaller by the setting's steps, within its
    /// range; the caller keeps the new size ([`Navigator::take_resized`]).
    pub fn resize_by(&mut self, steps: i32) {
        let to = omsi_launcher_lib::nav_scale(Some(self.size as f64 + (steps as f64) * SIZE_STEP as f64)) as f32;
        if (to - self.size).abs() > 1e-4 {
            // (a panel the player sized keeps its size: its texts grow or shrink in it, as far
            // as they fit - see `nav_panel::scale`)
            self.size = to;
            self.drawn_at = f32::MIN;
            self.resized = Some(to);
        }
        // (also at the end of the range: the note says where it stands)
        self.size_at = self.time;
    }

    /// A size changed in the game since this was last asked: the setting to keep it as.
    pub fn take_resized(&mut self) -> Option<f32> {
        self.resized.take()
    }

    /// Ctrl + the wheel: the notches, whole steps of the size as they add up.
    fn size_wheel(&mut self, amount: f32) {
        // (the other way: what was left of the last notches counts no more)
        if self.size_wheel * amount < 0.0 {
            self.size_wheel = 0.0;
        }
        self.size_wheel += amount;
        let steps = self.size_wheel.trunc();
        if steps != 0.0 {
            self.size_wheel -= steps;
            self.resize_by(steps as i32);
        }
    }

    /// The mouse wheel at (`x`, `y`): the city map takes it while it is open (Ctrl: the
    /// size); over the small navigator Ctrl + the wheel sizes it, and the wheel alone scrolls
    /// its sign-on page when that is taller than the window (`panel`: the small navigator is
    /// on the screen there, not in VR's cab). True when it was the navigator's.
    pub fn wheel(&mut self, amount: f32, x: f32, y: f32, ctrl: bool, panel: bool) -> bool {
        if self.city.open {
            if ctrl {
                self.size_wheel(amount);
            } else {
                self.map_wheel(amount, x, y);
            }
            return true;
        }
        if !panel || !self.over_panel(x, y) {
            return false;
        }
        if ctrl {
            self.size_wheel(amount);
            return true;
        }
        // (the sign-on page taller than the window: the wheel scrolls it)
        let Some(fit) = self.signing.as_ref().map(|p| &p.fit).filter(|f| crate::nav_signon::panel_scroll_max(f) > 0.0) else { return false };
        let (by, max) = (amount * 48.0 * fit.s, crate::nav_signon::panel_scroll_max(fit));
        self.city.phone.scroll = (self.city.phone.scroll - by).clamp(0.0, max);
        self.drawn_at = f32::MIN;
        true
    }
}

/// What the pairing QR code is made of, as far as the navigator sees: where and with which
/// code a device pairs.
fn qr_key(st: &crate::companion::CompanionState) -> String {
    format!("{:?} {:?} {}", st.listening, st.addresses.first(), st.pairing_code)
}

/// The navigator's size as it is said: "110 %".
fn size_text(size: f32) -> String {
    format!("{:.0} %", size * 100.0)
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
        NavFrame { traffic: self.traffic, players: self.players.clone(), bus: self.bus, heading: self.heading, speed_kmh: self.speed_kmh, outside_temp: self.outside_temp, inside_temp: self.inside_temp, line: self.line.clone(), terminus: self.terminus.clone(), stops: self.stops.clone(), delay: self.delay, duty: self.duty, passengers: self.passengers, time: self.time, weekday: self.weekday, language: self.language, screen: self.screen, ui_scale: self.ui_scale, follow_window: self.follow_window, dt: self.dt, stop_requested: self.stop_requested, info_rect: self.info_rect }
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
pub(crate) fn lanes_near(net: &Network, c: DVec2, radius: f64) -> Vec<usize> {
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
    let Ok(v) = omsi_cfg::env::var("OMSI_NAV_PROBE") else { return };
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
/// every so many metres for so far, how much of it, only near a point, and where on its last
/// lane it ends (the player's own destination) when not at that lane's end.
struct RouteStyle {
    extra_m: f32,
    min_px: f32,
    arrows: Option<(f32, f32)>,
    max_len: f32,
    near: Option<(DVec2, f64)>,
    end: Option<f32>,
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
        // (the last lane cut where the route ends: the player's destination)
        let until = style.end.filter(|_| k + 1 == lanes.len()).map(|e| e.max(from)).unwrap_or(f32::MAX);
        let mut pts: Vec<Vec3> = Vec::new();
        if k == 0 {
            pts.push(rel(lane.at(s0).0));
        }
        for (q, d) in lane.points.iter().zip(&lane.dist) {
            if (*d > from + 0.05 || k > 0) && *d < until - 0.05 {
                pts.push(rel(*q));
            }
        }
        if until < lane.length() {
            pts.push(rel(lane.at(until).0));
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
                    if at > from + 4.0 && at < until - 2.0 && total + at - from < reach {
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
    let debug = omsi_cfg::env::var_os("OMSI_DEBUG_NAV").is_some();
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

/// The lane a way from the bus begins on: the one under it going its way; else (a depot, a
/// car park, the grass) the nearest street lane that does not point back at the bus, else
/// any within 80 m.
pub(crate) fn start_lane(net: &Network, bus: DVec3, heading: f64) -> Option<usize> {
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
    pick(14.0, 70.0).or_else(|| pick(80.0, 100.0)).or_else(|| pick(80.0, 181.0))
}

/// A way from where the bus is back onto `ahead` (the route from the lane it was last
/// on): the lanes to drive up to the route, and the index in `ahead` of the route lane it
/// reaches (the route goes on from there). Dijkstra over
/// the street lanes, from the lane under the bus that runs its way, to the first route
/// lane reached (the search stops at 6 km).
pub(crate) fn way_back(net: &Network, bus: DVec3, heading: f64, ahead: &[usize], max_cost: f32) -> Option<(Vec<usize>, usize)> {
    use std::cmp::Ordering;
    use std::collections::BinaryHeap;
    let start = start_lane(net, bus, heading);
    if omsi_cfg::env::var_os("OMSI_DEBUG_NAV").is_some() {
        log::info!("navigator: way search from {:?} to {} route lanes", start, ahead.len());
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
/// or a click outside closes it. A click with its pin tool - or a right-click - sets one of
/// the player's own pins (`nav_pins`), which a drag moves and its card lists.
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
    /// The left column: the duty sheet (`nav_duty`) or the sign-on page (`nav_signon`), each
    /// with its button in the header (a second press puts the column away). The player's
    /// choice holds while the companion's stage is the one it was made at; otherwise the
    /// sign-on page shows until the duty is signed - the map's whole view then, not a
    /// column - and the sheet after (Omsi-Hub's duty panel said "sign on first" until then).
    /// What it showed last, at which stage.
    left: Option<(Left, Stage)>,
    shown: Left,
    stage: Stage,
    /// The column: where it is (window pixels, empty when not shown), how far the sheet's
    /// list is scrolled (points), the trip and stop it last scrolled to, and the interface
    /// scale it was drawn at.
    sheet: Rect,
    sheet_scroll: f32,
    sheet_at: Option<(usize, usize, bool)>,
    sheet_scale: f32,
    /// The sign-on page: its own state, where a press does what (window pixels), and the
    /// part of it that scrolls.
    phone: crate::nav_signon::Phone,
    phone_hits: Vec<(Rect, crate::nav_signon::Action)>,
    phone_view: Rect,
    /// The column dragged up or down (a finger has no wheel): the cursor's height before.
    column_drag: Option<f32>,
    /// The pin tool (the header's pin button): a click or a tap on the map sets a pin there.
    pin_tool: bool,
    /// A press on the map with the pin tool on: where (window pixels), and whether it has
    /// moved since (a drag of the map then, no pin).
    tap: Option<(f32, f32, bool)>,
    /// A pin being dragged to another place.
    pin_drag: Option<PinDrag>,
    /// The pins' card (the window's own pixels; empty when not shown) and where a press on it
    /// does what.
    card: Rect,
    card_hits: Vec<(Rect, crate::nav_pins::CardHit)>,
}

/// A pin dragged on the city map: which, where the press began and where the cursor is now
/// (the window's own pixels), and whether it has moved far enough to be a drag.
#[derive(Debug, Clone, Copy)]
struct PinDrag {
    k: usize,
    from: Vec2,
    at: Vec2,
    moved: bool,
}

/// The city map as painted, before it goes to the GPU: the ground, the traffic's dots, what
/// lies over the map (and the column's list, clipped to `list_clip`), the roads' and the
/// route's meshes when they were built again (the map's world, relative to `anchor`), and the
/// view they are seen through.
struct CityPaint {
    bg: Painter,
    dots: Painter,
    ui: Painter,
    list: Painter,
    list_clip: [f32; 4],
    roads: Option<Vec<omsi_ui::Vertex>>,
    route: Option<Vec<omsi_ui::Vertex>>,
    world: Layer,
    #[cfg_attr(not(test), allow(dead_code))]
    anchor: DVec2,
}

/// What the city map's left column shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Left {
    #[default]
    Nothing,
    Duty,
    SignOn,
}

/// What the left column shows at the companion's `stage`, with a duty or without: the
/// player's `choice` while it was made at this stage, else the sign-on page until the duty is
/// signed and then the duty sheet (nothing on a free drive). Until the duty is signed the
/// sign-on page is the map's whole view, so a driver who has no duty to sign for and has not
/// signed on gets the map (the badge's amber dot says signing on can be done).
fn left_page(choice: Option<(Left, Stage)>, stage: Stage, duty: bool) -> Left {
    let want = match choice {
        Some((l, at)) if at == stage => l,
        _ if stage == Stage::SignOn && !duty => Left::Nothing,
        _ if stage != Stage::OnDuty => Left::SignOn,
        _ if duty => Left::Duty,
        _ => Left::Nothing,
    };
    if want == Left::Duty && !duty { Left::Nothing } else { want }
}

impl CityMap {
    /// A press on the button of `page`: that page in the left column, or the column put away
    /// when it shows already.
    fn choose(&mut self, page: Left) {
        let next = if self.shown == page { Left::Nothing } else { page };
        self.left = Some((next, self.stage));
        self.shown = next;
    }
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
                self.city.mpp = omsi_cfg::env::var("OMSI_NAV_MAP_MPP").ok().and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && (0.25..=20.0).contains(v)).unwrap_or(2.5);
            }
        }
        self.city.drag = None;
        self.city.column_drag = None;
        self.city.tap = None;
        self.city.pin_drag = None;
        // (the pin tool is put away with the map)
        if !self.city.open {
            self.city.pin_tool = false;
        }
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
            Some(a) => crate::nav_panel::place(a, pw, ph, [sw, sh]).into(),
            None => (x0, y0),
        }
    }

    /// What a press at (`x`, `y`) does on the small navigator while it is the sign-on page: a
    /// key of its keypad, a button.
    fn panel_action(&self, x: f32, y: f32) -> Option<crate::nav_signon::Action> {
        let p = self.signing.as_ref()?;
        let local = Vec2::new(x - self.panel_rect[0], y - self.panel_rect[1]);
        p.hits.iter().find(|h| h.0.contains(local)).map(|h| h.1)
    }

    /// The mouse button went down on the small navigator: a click opens the city map, a
    /// drag moves the navigator (see [`Navigator::panel_move`]). While it is the sign-on page
    /// a press on a key or a button works it there and then (as on the city map), and is no
    /// click nor drag.
    /// A press on the board's handle shows or hides the board (the setting kept, see
    /// [`Navigator::take_board`]); on an edge or a corner a drag sizes the panel by it.
    pub fn panel_press(&mut self, x: f32, y: f32) {
        if let Some(a) = self.panel_action(x, y) {
            self.panel_drag = None;
            self.phone_act(a);
            // (drawn again at once: the digit shows in its box)
            self.drawn_at = f32::MIN;
            return;
        }
        if self.over_handle(x, y) {
            self.panel_drag = None;
            self.toggle_board();
            return;
        }
        let r = self.panel_rect;
        let grip = self.grip_at(x, y).unwrap_or(crate::nav_panel::Grip::Move);
        self.panel_drag = Some(PanelGrab { from: [x, y], rect: [r[0] - self.origin_x, r[1], r[2] - self.origin_x, r[3]], grip, moved: false });
    }

    /// What a press at (`x`, `y`) takes hold of on the small navigator: its edges only where
    /// it can be sized (not on the cockpit's display, not with the on-screen controls).
    fn grip_at(&self, x: f32, y: f32) -> Option<crate::nav_panel::Grip> {
        let grip = crate::nav_panel::grip_at(self.panel_rect, x, y, self.edge())?;
        Some(if self.cockpit_display || crate::platform::touch_controls() { crate::nav_panel::Grip::Move } else { grip })
    }

    /// How far in from its border an edge of the small navigator can be taken hold of.
    fn edge(&self) -> f32 {
        (7.0 * self.unit).clamp(6.0, 14.0)
    }

    /// The smallest the player can make the small navigator (pixels).
    fn min_size(&self) -> [f32; 2] {
        let u = self.unit.max(0.5);
        [crate::nav_panel::MIN_SIZE[0] * u, crate::nav_panel::MIN_SIZE[1] * u]
    }

    /// The point (physical pixels) is on the duty board's handle.
    fn over_handle(&self, x: f32, y: f32) -> bool {
        let Some(l) = self.layout.filter(|_| self.signing.is_none() && self.enabled && !self.city.open) else { return false };
        let local = Vec2::new(x - self.panel_rect[0], y - self.panel_rect[1]);
        l.handle.is_some_and(|h| h.inset(-4.0 * l.s).contains(local))
    }

    /// The board under the map shown or put away (its handle): kept as the player's choice,
    /// for Shift+N and the next game too.
    pub fn toggle_board(&mut self) {
        self.schedule = !self.schedule;
        self.board = self.schedule;
        self.board_changed = Some(self.board);
        self.drawn_at = f32::MIN;
    }

    /// The board wanted under the map or not (the game's settings): shown or put away at once.
    pub fn set_board(&mut self, on: bool) {
        self.board = on;
        self.schedule = on;
        self.drawn_at = f32::MIN;
    }

    /// The board was shown or put away on the panel: the `nav_board` setting to keep.
    pub fn take_board(&mut self) -> Option<bool> {
        self.board_changed.take()
    }

    /// The cursor moved with the button held on the navigator: past a few pixels the panel
    /// follows it, or the edges it holds do. True while it is being dragged.
    pub fn panel_move(&mut self, x: f32, y: f32) -> bool {
        use crate::nav_panel::{self as np, Grip};
        let Some(g) = self.panel_drag.as_mut() else { return false };
        let (dx, dy) = (x - g.from[0], y - g.from[1]);
        if !g.moved && dx.hypot(dy) < if g.grip == Grip::Move { 6.0 } else { 2.0 } {
            return false;
        }
        g.moved = true;
        let g = *g;
        match g.grip {
            Grip::Move => {
                let room = self.panel_room;
                let share = |p: f32, r: f32| if r > 0.0 { (p / r).clamp(0.0, 1.0) } else { 0.0 };
                self.at = Some([share(g.rect[0] + dx, room[0]), share(g.rect[1] + dy, room[1])]);
            }
            sides => {
                let r = np::resized(g.rect, sides, dx, dy, self.min_size(), self.window);
                let (w, h) = (r[2] - r[0], r[3] - r[1]);
                let sh = self.window[1].max(1.0);
                self.custom = Some([w / sh, h / sh]);
                self.at = Some(np::share(Vec2::new(r[0], r[1]), w, h, self.window));
                self.drawn_at = f32::MIN;
            }
        }
        true
    }

    /// The button came up after a press on the navigator: Some(true) when it was dragged
    /// (moved or sized: the place and size are then kept, [`Navigator::take_rect`]),
    /// Some(false) for a click.
    pub fn panel_release(&mut self) -> Option<bool> {
        let g = self.panel_drag.take()?;
        self.rect_changed |= g.moved;
        Some(g.moved)
    }

    /// The mouse at (`x`, `y`) (no button held, or the navigator held): what it is over on the
    /// small navigator - the handle lit, an edge marked - and the cursor for it (as
    /// `App::set_cursor_kind` has them); None where it is not over the navigator.
    pub fn hover(&mut self, x: f32, y: f32) -> Option<u8> {
        let part = if let Some(g) = self.panel_drag.filter(|g| g.moved) {
            Some(HoverPart::Grip(g.grip))
        } else if !self.over_panel(x, y) || self.city.open {
            None
        } else if self.over_handle(x, y) {
            Some(HoverPart::Handle)
        } else {
            self.grip_at(x, y).map(HoverPart::Grip)
        };
        if part != self.panel_hover {
            self.panel_hover = part;
            self.drawn_at = f32::MIN;
        }
        part.map(|p| match p {
            HoverPart::Handle => 1,
            HoverPart::Grip(crate::nav_panel::Grip::Move) if self.panel_drag.is_some_and(|g| g.moved) => 3,
            HoverPart::Grip(crate::nav_panel::Grip::Move) => 0,
            HoverPart::Grip(g) => g.cursor(),
        })
    }

    /// The `nav_rect` setting for where the navigator is and how large the player made it.
    pub fn rect_setting(&self) -> String {
        let wide = |p: [f32; 2]| [p[0] as f64, p[1] as f64];
        omsi_launcher_lib::nav_rect_text(self.at.map(wide), self.custom.map(wide))
    }

    /// The place and size from the `nav_rect` setting (a place there wins over the corner).
    pub fn set_rect(&mut self, setting: &str) {
        let (at, size) = omsi_launcher_lib::nav_rect_parts(setting);
        let narrow = |p: [f64; 2]| [p[0] as f32, p[1] as f32];
        if at.is_some() {
            self.at = at.map(narrow);
        }
        self.custom = size.map(narrow);
    }

    /// The navigator was moved or sized in the game: the `nav_rect` setting to keep.
    pub fn take_rect(&mut self) -> Option<String> {
        std::mem::take(&mut self.rect_changed).then(|| self.rect_setting())
    }

    /// The navigator as it came: in the bottom-left corner at its own size, at 100 %, the
    /// board under the map (shown again when a duty runs).
    pub fn reset_panel(&mut self) {
        self.at = None;
        self.custom = None;
        self.corner = "bottom-left".into();
        self.size = 1.0;
        self.board = true;
        self.schedule = self.duty_seen;
        self.panel_drag = None;
        self.drawn_at = f32::MIN;
    }

    fn map_hit(&self, x: f32, y: f32) -> bool {
        let r = self.city.rect;
        x >= r[0] && y >= r[1] && x < r[2] && y < r[3]
    }

    /// A mouse press while the map is open: on it, a drag or a button; outside, it closes.
    pub fn map_press(&mut self, x: f32, y: f32) {
        if !self.map_hit(x, y) {
            self.city.open = false;
            self.city.pin_tool = false;
            return;
        }
        let local = Vec2::new(x - self.city.rect[0], y - self.city.rect[1]);
        let hit = self.city.buttons.iter().find(|(r, _)| r.contains(local)).map(|b| b.1);
        // (a pin under the press, not one under the column or the card)
        let pin = (hit.is_none() && !self.city.sheet.contains(local) && !self.city.card.contains(local) && !self.full_view()).then(|| self.pin_under(local)).flatten();
        match hit {
            Some(0) => self.city.follow = true,
            Some(1) => self.city.mpp = (self.city.mpp / 1.6).max(0.25),
            Some(2) => self.city.mpp = (self.city.mpp * 1.6).min(self.max_mpp()),
            Some(3) => self.city.open = false,
            Some(4) => self.city.choose(Left::Duty),
            Some(5) => self.city.choose(Left::SignOn),
            // (the sign-on page's whole view: the map instead, until the stage changes - the
            // badge brings the page back)
            Some(6) => self.city.choose(Left::Nothing),
            Some(7) => self.resize_by(-1),
            Some(8) => self.resize_by(1),
            // the pin tool: a click (a tap) on the map sets a pin
            Some(9) => self.city.pin_tool = !self.city.pin_tool,
            // the pins' card: its buttons (and no drag of the map under it)
            _ if self.city.card.contains(local) => {
                use crate::nav_pins::{CardHit, Op};
                if let Some(h) = self.city.card_hits.iter().find(|(r, _)| r.contains(local)).map(|h| h.1) {
                    self.pins.ops.push(match h {
                        CardHit::Remove(k) => Op::Remove(k),
                        CardHit::Up(k) => Op::Shift(k, true),
                        CardHit::Down(k) => Op::Shift(k, false),
                        CardHit::Clear => Op::Clear,
                    });
                }
            }
            // (the column is no handle to drag the map by; on the sign-on page its keys and
            // buttons work)
            _ if self.city.sheet.contains(local) => {
                let on_page = self.city.shown == Left::SignOn && self.city.phone_view.contains(local);
                match self.city.phone_hits.iter().find(|(r, _)| on_page && r.contains(local)).map(|h| h.1) {
                    Some(a) => self.phone_act(a),
                    None => self.city.column_drag = Some(y),
                }
            }
            // (the sign-on page's whole view: no map to drag under its header)
            _ if self.full_view() => {}
            // a pin: dragged to another place
            _ if pin.is_some() => self.city.pin_drag = pin.map(|k| PinDrag { k, from: local, at: local, moved: false }),
            _ => {
                self.city.drag = Some((x, y));
                // (with the pin tool a press that comes up where it went down sets a pin;
                // the header is no place for one)
                if self.city.pin_tool && local.y > 44.0 * self.city.sheet_scale.max(0.5) {
                    self.city.tap = Some((x, y, false));
                }
            }
        }
    }

    /// A right-click on the city map: a pin where it is (as a click with the pin tool), or the
    /// pin under it taken away. Not on the header, the column or the card.
    pub fn map_right_press(&mut self, x: f32, y: f32) {
        use crate::nav_pins::Op;
        if !self.city.open || !self.map_hit(x, y) || self.full_view() {
            return;
        }
        let local = Vec2::new(x - self.city.rect[0], y - self.city.rect[1]);
        if local.y < 44.0 * self.city.sheet_scale.max(0.5) || self.city.sheet.contains(local) || self.city.card.contains(local) {
            return;
        }
        match self.pin_under(local) {
            Some(k) => self.pins.ops.push(Op::Remove(k)),
            None => {
                if let Some(p) = self.map_point(x, y) {
                    self.pins.ops.push(Op::Add(p));
                }
            }
        }
    }

    /// Where a world point is in the city map's window (its own pixels).
    fn city_spot(&self, p: DVec2) -> Vec2 {
        let r = self.city.rect;
        let (w, h) = ((r[2] - r[0]) as f64, (r[3] - r[1]) as f64);
        let d = p - self.city.center;
        Vec2::new((w * 0.5 + d.x / self.city.mpp) as f32, (h * 0.5 - d.y / self.city.mpp) as f32)
    }

    /// The pin whose marker is under `local` (the window's own pixels): a via's disc, the
    /// destination's head or its point; the one drawn on top first.
    fn pin_under(&self, local: Vec2) -> Option<usize> {
        use crate::nav_pins::{destination_head, Role, DEST_R, VIA_R};
        let s = self.city.sheet_scale.max(0.5);
        (0..self.pins.list.len()).find(|&k| {
            let p = self.city_spot(self.pins.list[k].at);
            match self.pins.role(k) {
                Role::Destination => p.distance(local) < 10.0 * s || destination_head(p, DEST_R * s).distance(local) < (DEST_R + 4.0) * s,
                Role::Via(_) => p.distance(local) < (VIA_R + 5.0) * s,
            }
        })
    }

    /// A key for the sign-on page's keypad: it takes the digits, Backspace and Delete while
    /// it asks for the number or the code - on the city map, or on the small navigator while
    /// that is the page. True when the key was used (the caller then gives it to nothing
    /// else).
    pub fn page_key(&mut self, code: winit::keyboard::KeyCode) -> bool {
        if !self.keypad_shown() {
            return false;
        }
        let Some(a) = crate::nav_signon::key_action(code) else { return false };
        self.phone_act(a);
        self.drawn_at = f32::MIN;
        true
    }

    /// The sign-on page's keypad is on the screen: the city map's page, or the small
    /// navigator as the page, asking for the number or the code.
    pub fn keypad_shown(&self) -> bool {
        if self.city.open {
            return self.city.shown == Left::SignOn && self.city.stage == Stage::SignOn;
        }
        self.enabled && self.signing.as_ref().is_some_and(|p| p.stage == Stage::SignOn)
    }

    /// The city map is the sign-on page, its whole view (no map to point at).
    fn full_view(&self) -> bool {
        self.city.shown == Left::SignOn && self.city.stage != Stage::OnDuty
    }

    /// Do what a press on the sign-on page asks: on the page, and of the game (through the
    /// companion, whose state the phones share).
    fn phone_act(&mut self, a: crate::nav_signon::Action) {
        use crate::nav_signon::Call;
        let st = crate::companion::state();
        self.city.phone.follow(&st);
        let Some(call) = self.city.phone.press(a, &st) else { return };
        match call {
            Call::SignOn { number, code } => {
                let answer = crate::companion::sign_on(&number, code.as_deref());
                self.city.phone.attempted(answer);
            }
            Call::SignOff => crate::companion::sign_off(),
            Call::Free => {
                crate::companion::drive_free();
            }
            Call::Ask(r) => crate::companion::request(r),
            Call::NewCode => crate::companion::new_pairing_code(),
            Call::Forget => crate::companion::forget_devices(),
            Call::Reveal(on) => crate::companion::reveal(on),
        }
    }

    /// The world point (x, y) under the window point, when the map is open and the point
    /// is on it.
    pub fn map_point(&self, x: f32, y: f32) -> Option<DVec2> {
        if !self.city.open || !self.map_hit(x, y) || self.full_view() {
            return None;
        }
        let r = self.city.rect;
        let (w, h) = ((r[2] - r[0]) as f64, (r[3] - r[1]) as f64);
        let (lx, ly) = ((x - r[0]) as f64 - w * 0.5, (y - r[1]) as f64 - h * 0.5);
        Some(self.city.center + DVec2::new(lx, -ly) * self.city.mpp)
    }

    pub fn map_release(&mut self) {
        use crate::nav_pins::Op;
        // a pin dragged: to where it was let go (snapped onto a road in the next frame; back
        // where it was with none near)
        // (moved as far as the cursor went: it may have taken the flag by its head)
        if let Some(d) = self.city.pin_drag.take().filter(|d| d.moved) {
            let mpp = self.city.mpp;
            if let Some(pin) = self.pins.list.get_mut(d.k) {
                let from = pin.at;
                let to = from + DVec2::new((d.at.x - d.from.x) as f64 * mpp, -(d.at.y - d.from.y) as f64 * mpp);
                pin.at = to;
                self.pins.ops.push(Op::Move(d.k, to, from));
            }
        }
        // the pin tool: a click that did not drag the map sets a pin
        if let Some((x, y, false)) = self.city.tap.take() {
            if let Some(p) = self.map_point(x, y) {
                self.pins.ops.push(Op::Add(p));
            }
        }
        self.city.drag = None;
        self.city.column_drag = None;
    }

    pub fn map_move(&mut self, x: f32, y: f32) {
        // a pin dragged follows the cursor (past a few pixels: a press alone moves nothing)
        if let Some(d) = self.city.pin_drag.as_mut() {
            let local = Vec2::new(x - self.city.rect[0], y - self.city.rect[1]);
            if d.moved || local.distance(d.from) > 5.0 {
                d.moved = true;
                d.at = local;
            }
            return;
        }
        if let Some((tx, ty, moved)) = self.city.tap.as_mut() {
            if (x - *tx).hypot(y - *ty) > 8.0 {
                *moved = true;
            }
        }
        // the left column dragged: its list or page follows the cursor (kept in range when it
        // is drawn)
        if let Some(py) = self.city.column_drag {
            let scroll = if self.city.shown == Left::SignOn { &mut self.city.phone.scroll } else { &mut self.city.sheet_scroll };
            *scroll = (*scroll - (y - py)).max(0.0);
            self.city.column_drag = Some(y);
            return;
        }
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
        // (over the pins' card: nothing to zoom)
        if self.city.card.contains(Vec2::new(x - r[0], y - r[1])) {
            return;
        }
        // over the left column: the duty sheet's list or the sign-on page scrolls (kept in
        // range when it is drawn)
        if self.city.sheet.contains(Vec2::new(x - r[0], y - r[1])) {
            let by = amount * 48.0 * self.city.sheet_scale.max(0.5);
            let scroll = if self.city.shown == Left::SignOn { &mut self.city.phone.scroll } else { &mut self.city.sheet_scroll };
            *scroll = (*scroll - by).max(0.0);
            return;
        }
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
        // (the navigator's own size on top of the interface's: the panels, texts and buttons)
        let s = (h / 760.0).clamp(0.95, 2.0) * f.ui_scale * if self.cockpit_display { 1.0 } else { self.size };
        // the left column: the duty sheet (Omsi-Hub's live duty) or the sign-on page
        // (Omsi-Hub's phone), as `left_page` has it
        let duty = f.duty.and_then(|d| crate::nav_duty::DutyState::of(d, f.time));
        let companion = crate::companion::state();
        let left = left_page(self.city.left, companion.stage, duty.is_some());
        self.city.stage = companion.stage;
        self.city.shown = left;
        self.city.phone.follow(&companion);
        self.city.sheet_scale = s;
        // until the duty is signed the sign-on page is the whole view, the map behind it
        if self.full_view() {
            self.city_signon(renderer, scene, f, &companion, duty.as_ref(), [x0, y0, w, h], s);
            return;
        }
        let paint = self.paint_city(f, &companion, duty.as_ref(), left, w, h, s);
        self.city_to_gpu(renderer, scene, paint, [x0, y0, w, h], s);
    }

    /// The city map painted for a window `w` x `h` at scale `s`, its left column showing
    /// `left`: the roads and the route (new meshes when they changed, in the map's world
    /// relative to its anchor), the ground, the traffic, and everything over them - the stops,
    /// the street names, the player's own pins, the bus, the column, the pins' card, the
    /// header and the scale. Where a press does what is kept for [`Navigator::map_press`].
    #[allow(clippy::too_many_arguments)]
    fn paint_city(&mut self, f: &NavFrame, companion: &crate::companion::CompanionState, duty: Option<&crate::nav_duty::DutyState>, left: Left, w: f32, h: f32, s: f32) -> CityPaint {
        let head_h = 44.0 * s;
        let sheet = if left == Left::Nothing { Rect::default() } else { Rect::new(12.0 * s, head_h + 12.0 * s, (340.0 * s).min(w * 0.42), h - head_h - 24.0 * s) };
        self.city.sheet = sheet;
        self.city.sheet_scale = s;
        // the pins' card (`nav_pins`): in the left column when that shows nothing else, else at
        // the right edge
        let card = self.pin_card(f);
        let room = h - head_h - 48.0 * s;
        let card_rect = if !crate::nav_pins::card_shown(&card) {
            Rect::default()
        } else if sheet.w > 0.0 {
            let cw = (320.0 * s).min(w * 0.34);
            Rect::new(w - 12.0 * s - cw, head_h + 12.0 * s, cw, crate::nav_pins::card_height(&card, s, room))
        } else {
            Rect::new(12.0 * s, head_h + 12.0 * s, (340.0 * s).min(w * 0.42), crate::nav_pins::card_height(&card, s, room))
        };
        self.city.card = card_rect;
        if self.city.follow {
            // (the bus in the middle of what the sheet - or the card in its place - leaves of
            // the map)
            let side = if sheet.w > 0.0 { sheet.right() } else { card_rect.right() };
            self.city.center = f.bus.truncate() - DVec2::new(side as f64 * 0.5 * self.city.mpp, 0.0);
        }
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
                let style = RouteStyle { extra_m: 0.0, min_px: 6.0, arrows: Some((every, f32::MAX)), max_len: f32::MAX, near: None, end: r.end };
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
        bg.rounded(win, 10.0 * s, Color::rgba(12, 16, 26, crate::ui::backdrop(self.opacity).min(1.0)));
        // traffic: blue dots, the public transport in its colours with its line (as on the
        // small map)
        let mut dots = Painter::new();
        let vehicles = map_vehicles(f.traffic.filter(|_| self.show_ai), f.bus, f64::INFINITY);
        paint_vehicles(&mut dots, &vehicles, &rel, self.city.mpp as f32, s);
        let lines: Vec<(DVec3, Color, String, [DVec3; 2])> = vehicles.iter().filter_map(|v| v.line.clone().map(|l| (v.at, v.color, l, v.drawn_ends(self.city.mpp as f32, s)))).collect();
        // stops with their names, the bus, the frame and the header
        let mut ui = Painter::new();
        let n_stops = f.stops.len();
        // street names along their roads, where there is room (stops' names go first)
        let markers = spaced_markers(f.stops.iter().enumerate().map(|(k, st)| (k, to_screen(st.position))).filter(|(_, p)| win.contains(*p) && p.y > 50.0 * s), 20.0 * s);
        let mut taken: Vec<Rect> = markers.iter().map(|(_, p)| Rect::new(p.x - 9.0 * s, p.y - 9.0 * s, 18.0 * s, 18.0 * s)).collect();
        taken.push(Rect::new(0.0, 0.0, w, 50.0 * s));
        if sheet.w > 0.0 {
            taken.push(sheet.inset(-6.0 * s));
        }
        if card_rect.w > 0.0 {
            taken.push(card_rect.inset(-6.0 * s));
        }
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
        for (pos, color, l, ends) in &lines {
            let p = to_screen(*pos);
            if !win.contains(p) {
                continue;
            }
            let px = 11.0 * s;
            let own = f.line.as_deref().is_some_and(|o| o.trim().eq_ignore_ascii_case(l.trim()));
            let l = self.fonts.fit(l, px, Weight::Bold, 50.0 * s);
            let tw = self.fonts.width(&l, px, Weight::Bold) + crate::stop_signs::CHIP_PAD * s;
            // (above the bus's higher end, not over the bus)
            let top = ends.iter().map(|e| to_screen(*e).y).fold(p.y, f32::min);
            let r = Rect::new(p.x - tw * 0.5, top.min(p.y - 7.0 * s) - 17.0 * s, tw, 15.0 * s);
            if taken.iter().any(|o| o.x < r.right() && r.x < o.right() && o.y < r.bottom() && r.y < o.bottom()) {
                continue;
            }
            taken.push(r);
            crate::stop_signs::draw_chip(&mut ui, &mut self.atlas, &self.fonts, r, &l, px, *color, own, s);
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
        // the stops as the sign the player chose (`stop_signs`): the ones served faded under
        // everything, the ones ahead over their names (the next one's edge reaches past where
        // its name begins), far out the stops between the next and the terminus as dots
        let signs = crate::stop_signs::style();
        let far = self.city.mpp >= 4.0;
        for q in crate::stop_signs::served(f.duty, &self.stop_pos).into_iter().map(&to_screen).filter(|q| win.contains(*q) && q.y > 50.0 * s && markers.iter().all(|(_, m)| m.distance(*q) >= 14.0 * s)) {
            crate::stop_signs::draw(&mut ui, signs, crate::stop_signs::Kind::Passed, q, if far { 6.0 } else { 13.0 } * s);
        }
        for (k, name, r) in stop_labels {
            crate::stop_signs::draw_label(&mut ui, &mut self.atlas, &self.fonts, r, &name, 12.5 * s, signs, k == 0, s);
        }
        for (k, p) in markers.into_iter().rev() {
            let kind = crate::stop_signs::Kind::ahead(k, n_stops);
            let dot = far && kind == crate::stop_signs::Kind::Ahead;
            crate::stop_signs::draw(&mut ui, signs, kind, p, if dot { 6.0 } else { 13.0 } * s);
        }
        // the player's own pins (`nav_pins`): the destination's flag, the vias numbered - the
        // one being dragged where the cursor has it
        let dragged = self.city.pin_drag.filter(|d| d.moved);
        for k in (0..self.pins.list.len()).rev() {
            let here = dragged.filter(|d| d.k == k);
            let p = to_screen(self.pins.list[k].at.extend(0.0)) + here.map(|d| d.at - d.from).unwrap_or(Vec2::ZERO);
            if !win.pad(30.0 * s, 60.0 * s).contains(p) {
                continue;
            }
            let alpha = if here.is_some() { 0.8 } else { 1.0 };
            match self.pins.role(k) {
                crate::nav_pins::Role::Destination => crate::nav_pins::draw_destination(&mut ui, &mut self.atlas, p, crate::nav_pins::DEST_R * s, self.pins.arrived.is_some(), alpha),
                crate::nav_pins::Role::Via(n) => crate::nav_pins::draw_via(&mut ui, &mut self.atlas, &self.fonts, p, n, crate::nav_pins::VIA_R * s, alpha),
            }
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
        // the duty sheet: its frame and head here, its list in a layer of its own clipped to
        // it (the list scrolls)
        let mut list = Painter::new();
        let mut list_clip = [0.0; 4];
        self.city.phone_hits.clear();
        self.city.phone_view = Rect::default();
        if left == Left::SignOn {
            // the sign-on page: its head here, its items in the clipped layer (they scroll)
            if let Some(r) = self.city.phone.wants(companion) {
                crate::companion::request(r);
            }
            let qr = self.pair_qr(companion);
            let page = crate::nav_signon::page_in(companion, &self.city.phone, duty, f.time, crate::nav_signon::Room::Column, qr.as_ref());
            let view = crate::nav_signon::draw_head(&mut crate::nav_duty::Pen { p: &mut ui, atlas: &mut self.atlas, fonts: &self.fonts }, &page, sheet, s);
            let pad = crate::nav_signon::PAD * s;
            let (placed, height) = crate::nav_signon::layout(&page.items, view.w - 2.0 * pad, s, &self.fonts);
            let content = height + 18.0 * s;
            let max = (content - view.h).max(0.0);
            self.city.phone.scroll = self.city.phone.scroll.clamp(0.0, max);
            let origin = Vec2::new(view.x + pad, view.y + 6.0 * s - self.city.phone.scroll);
            crate::nav_signon::draw_items(&mut crate::nav_duty::Pen { p: &mut list, atlas: &mut self.atlas, fonts: &self.fonts }, &page.items, &placed, origin, view, s);
            if max > 0.0 {
                let thumb = (view.h * view.h / content).max(24.0 * s);
                let y = view.y + (view.h - thumb) * self.city.phone.scroll / max;
                ui.rounded(Rect::new(view.right() - 6.0 * s, y, 3.0 * s, thumb), 1.5 * s, Color::WHITE.alpha(0.18));
            }
            self.city.phone_hits = crate::nav_signon::hits(&page.items, &placed, s).into_iter().map(|(r, a)| (Rect::new(r.x + origin.x, r.y + origin.y, r.w, r.h), a)).collect();
            self.city.phone_view = view;
            list_clip = [view.x, view.y, view.right(), view.bottom()];
        }
        if let (Left::Duty, Some(d)) = (left, duty) {
            let view = crate::nav_duty::draw_sheet_head(&mut crate::nav_duty::Pen { p: &mut ui, atlas: &mut self.atlas, fonts: &self.fonts }, d, sheet, f.time, s);
            let rows = crate::nav_duty::sheet(d);
            let content = (crate::nav_duty::sheet_height(&rows) + 12.0) * s;
            let max = (content - view.h).max(0.0);
            // (it follows the duty: when the trip or the stop changes it scrolls there, a
            // third down the list, as Omsi-Hub's live duty keeps the trip under way in view)
            let key = (d.focus(), d.next_stop, d.done);
            if self.city.sheet_at != Some(key) {
                self.city.sheet_at = Some(key);
                self.city.sheet_scroll = crate::nav_duty::sheet_focus(&rows) * s - view.h * 0.3;
            }
            self.city.sheet_scroll = self.city.sheet_scroll.clamp(0.0, max);
            crate::nav_duty::draw_sheet_list(&mut crate::nav_duty::Pen { p: &mut list, atlas: &mut self.atlas, fonts: &self.fonts }, &rows, view, self.city.sheet_scroll, s);
            if max > 0.0 {
                let thumb = (view.h * view.h / content).max(24.0 * s);
                let y = view.y + (view.h - thumb) * self.city.sheet_scroll / max;
                ui.rounded(Rect::new(view.right() - 6.0 * s, y, 3.0 * s, thumb), 1.5 * s, Color::WHITE.alpha(0.18));
            }
            list_clip = [view.x, view.y, view.right(), view.bottom()];
        }
        // the pins' card, and a note of theirs under the header (in the middle of the map the
        // column and the card leave)
        self.city.card_hits.clear();
        if card_rect.w > 0.0 {
            self.city.card_hits = crate::nav_pins::draw_card(&mut crate::nav_duty::Pen { p: &mut ui, atlas: &mut self.atlas, fonts: &self.fonts }, &card, card_rect, s);
        }
        if let Some((note, t)) = self.pins.note {
            let card_left = card_rect.w > 0.0 && card_rect.x < w * 0.5;
            let lo = sheet.right().max(if card_left { card_rect.right() } else { 0.0 });
            let hi = if card_rect.w > 0.0 && !card_left { card_rect.x } else { w };
            crate::nav_pins::draw_note(&mut crate::nav_duty::Pen { p: &mut ui, atlas: &mut self.atlas, fonts: &self.fonts }, note, Vec2::new((lo + hi) * 0.5, head_h + 34.0 * s), s, (t / 0.3).min(1.0));
        }
        // header: the line and where it goes, the next stop; the size and the buttons on the
        // right (the pin tool's; the duty sheet's only on a duty; the sign-on page's always)
        let buttons: &[(&str, u8)] = if f.duty.is_some() { &[("close", 3), ("zoom_out", 2), ("zoom_in", 1), ("my_location", 0), ("location_on", 9), ("view_list", 4), ("badge", 5)] } else { &[("close", 3), ("zoom_out", 2), ("zoom_in", 1), ("my_location", 0), ("location_on", 9), ("badge", 5)] };
        self.city_head(&mut ui, f, w, s, buttons, left, companion.stage, true);
        let pad = 16.0 * s;
        // a scale bar, bottom left
        let nice = [10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0];
        let metres = nice.iter().copied().find(|m| m / self.city.mpp > 70.0 * s as f64).unwrap_or(5000.0);
        let len = (metres / self.city.mpp) as f32;
        let by = h - 22.0 * s;
        // (right of the duty sheet when it is there)
        let sx = if sheet.w > 0.0 { sheet.right() + pad } else { pad };
        ui.rect(Rect::new(sx, by, len, 2.0 * s), TEXT_DIM);
        ui.text(&mut self.atlas, &self.fonts, &if metres >= 1000.0 { format!("{:.0} km", metres / 1000.0) } else { format!("{metres:.0} m") }, 12.0 * s, Weight::Medium, Vec2::new(sx + len + 8.0 * s, by + 4.0 * s), Align::Left, TEXT_DIM);
        ui.rounded_border(win, 10.0 * s, 1.0, Color::WHITE.alpha(0.08));
        CityPaint { bg, dots, ui, list, list_clip, roads: roads_verts, route: route_verts, world, anchor }
    }

    /// The city map as [`Navigator::paint_city`] painted it, into its texture and over the
    /// picture at `win` (x, y, width, height), at scale `s`.
    fn city_to_gpu(&mut self, renderer: &Renderer, scene: &mut Scene, paint: CityPaint, win: [f32; 4], s: f32) {
        let [x0, y0, w, h] = win;
        let (tw, th) = (w as u32, h as u32);
        let CityPaint { bg, dots, ui, list, list_clip, roads: roads_verts, route: route_verts, world, .. } = paint;
        let (n_bg, n_dots) = (bg.len(), dots.len());
        let Some((tex, _, _)) = self.city.target else { return };
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
        let n_list = all.len() as u32;
        all.extend(list.verts);
        gpu.upload(device, queue, 5, &all);
        gpu.upload_atlas(queue, &mut self.atlas);
        let flat = Layer::flat([0.0, 0.0, w, h], 10.0 * s, 1.0);
        let layers = [flat, world, Layer::flat(list_clip, 0.0, 1.0)];
        let roads_n = self.city.roads.map(|r| r.1).unwrap_or(0);
        let draws = [
            Draw { buffer: 5, range: 0..n_bg, layer: 0, texture: 0 },
            Draw { buffer: 3, range: 0..roads_n, layer: 1, texture: 0 },
            Draw { buffer: 4, range: 0..self.city.route.1, layer: 1, texture: 0 },
            Draw { buffer: 5, range: n_bg..n_bg + n_dots, layer: 1, texture: 0 },
            Draw { buffer: 5, range: n_ui..n_list, layer: 0, texture: 0 },
            Draw { buffer: 5, range: n_list..all.len() as u32, layer: 2, texture: 0 },
        ];
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("city map") });
        gpu.render(device, queue, &mut enc, &view, (tw, th), Some(wgpu::Color::TRANSPARENT), &layers, &draws);
        queue.submit([enc.finish()]);
        scene.overlays.push((tex, [x0, y0, x0 + w, y0 + h]));
    }

    /// The city map's header along the top of a view `w` pixels wide (opaque: the route and
    /// the stops showed through behind its text): the line and where it goes, the next stop
    /// (`next_stop`: on the map, not over the sign-on page), the navigator's size with its -
    /// and +, and the `buttons` on the right - lit for what shows (`left`), the sign-on
    /// page's with an amber dot while the duty is not signed and the page is put away. The
    /// buttons go in `self.city.buttons`.
    #[allow(clippy::too_many_arguments)]
    fn city_head(&mut self, ui: &mut Painter, f: &NavFrame, w: f32, s: f32, buttons: &[(&str, u8)], left: Left, stage: Stage, next_stop: bool) {
        let head = Rect::new(0.0, 0.0, w, 44.0 * s);
        ui.rect(head, Color::rgba(20, 26, 38, 0.97));
        let pad = 16.0 * s;
        let wd = words(f.language);
        let title = match (&f.line, &f.terminus) {
            (Some(l), Some(t)) => format!("{}  ›  {}", l.trim(), t.trim()),
            _ => wd.no_duty.to_string(),
        };
        let title_w = ui.text_in(&mut self.atlas, &self.fonts, &title, 15.0 * s, Weight::Bold, Rect::new(pad, head.y, w * 0.4, head.h), Align::Left, TEXT);
        let bs = 30.0 * s;
        self.city.buttons.clear();
        let mut bx = w - pad - bs;
        for &(icon, id) in buttons {
            let r = Rect::new(bx, head.y + (head.h - bs) * 0.5, bs, bs);
            let on = (id == 0 && self.city.follow) || (id == 4 && left == Left::Duty) || (id == 5 && left == Left::SignOn) || (id == 9 && self.city.pin_tool);
            ui.rounded(r, 5.0 * s, if on { crate::nav_duty::accent() } else { Color::rgba(36, 44, 62, 1.0) });
            ui.icon(&mut self.atlas, icon, r.center(), 18.0 * s, TEXT);
            // (an amber dot while the duty is not signed and the page is put away)
            if id == 5 && !on && stage != Stage::OnDuty {
                ui.circle(Vec2::new(r.right() - 4.0 * s, r.y + 4.0 * s), 4.0 * s, crate::nav_duty::NOW);
            }
            self.city.buttons.push((r, id));
            bx -= bs + 8.0 * s;
        }
        // the navigator's size: - 100 % +, left of the buttons (Ctrl + the wheel does it too)
        let text = size_text(self.size);
        let tw = self.fonts.width("200 %", 12.5 * s, Weight::Bold).max(self.fonts.width(&text, 12.5 * s, Weight::Bold)) + 12.0 * s;
        let pill = Rect::new(bx + bs - 8.0 * s - (2.0 * bs + tw), head.y + (head.h - bs) * 0.5, 2.0 * bs + tw, bs);
        ui.rounded(pill, 5.0 * s, Color::rgba(36, 44, 62, 1.0));
        let (minus, plus) = (Rect::new(pill.x, pill.y, bs, bs), Rect::new(pill.right() - bs, pill.y, bs, bs));
        let ends = (omsi_launcher_lib::nav_scale(Some(0.0)) as f32, omsi_launcher_lib::nav_scale(Some(9.0)) as f32);
        for (r, icon, id, end) in [(minus, "remove", 7u8, ends.0), (plus, "add", 8, ends.1)] {
            // (greyed at the end of the range)
            let ink = if (self.size - end).abs() < 1e-3 { TEXT_DIM.alpha(0.5) } else { TEXT };
            ui.icon(&mut self.atlas, icon, r.center(), 18.0 * s, ink);
            self.city.buttons.push((r, id));
        }
        let lit = (0.0..SIZE_NOTE).contains(&(self.time - self.size_at));
        ui.text_in(&mut self.atlas, &self.fonts, &text, 12.5 * s, Weight::Bold, Rect::new(minus.right(), pill.y, tw, bs), Align::Center, if lit { TEXT } else { TEXT_DIM });
        // the next stop - or the player's own destination, or the diversion's next via
        let text = if !next_stop {
            None
        } else if let Some((t, c)) = self.pins_head() {
            Some((t, c))
        } else {
            f.stops.first().map(|st| {
                let d = self.next_dist.map(|d| if d >= 1000.0 { format!("{:.1} km", d / 1000.0) } else { format!("{:.0} m", d) }).unwrap_or_default();
                (format!("{}  ·  {}  ·  {:02}:{:02}", st.name.trim(), d, (st.arrival / 3600.0) as i32 % 24, ((st.arrival % 3600.0) / 60.0) as i32), TEXT_DIM)
            })
        };
        if let Some((t, c)) = text {
            // (up to the size and the buttons, not under them)
            let x = pad + title_w + 24.0 * s;
            let room = (pill.x - 16.0 * s - x).max(0.0);
            ui.text_in(&mut self.atlas, &self.fonts, &t, 13.5 * s, Weight::Medium, Rect::new(x, head.y, room.min(w * 0.45), head.h), Align::Left, c);
        }
    }

    /// The pins' card as the city map shows it (`nav_pins::draw_card`): each pin with how far
    /// along the route and how long to it, to the destination when there too; with no pins
    /// (the tool on) a diversion's when the navigator follows a trip.
    fn pin_card(&self, f: &NavFrame) -> crate::nav_pins::Card {
        use crate::nav_pins as pins;
        let rows: Vec<pins::CardRow> = self
            .pins
            .list
            .iter()
            .enumerate()
            .map(|(k, p)| {
                let dist = self.pins.dist.get(k).copied().flatten();
                pins::CardRow { role: self.pins.role(k), name: p.name.clone(), dist, eta: dist.map(|d| pins::eta(d, self.speed_avg)) }
            })
            .collect();
        let total = rows.last().filter(|_| !self.pins.diversion()).and_then(|r| r.dist).map(|d| {
            let e = pins::eta(d, self.speed_avg);
            (d, e, f.time + e)
        });
        let on_trip = !self.route.key.is_empty() && self.route.key != DEST_KEY;
        pins::Card { diversion: self.pins.diversion() || (self.pins.list.is_empty() && on_trip), rows, total, arrived: self.pins.arrived.is_some(), tool: self.city.pin_tool }
    }

    /// What the city map's header says of the player's own pins: on a free drive the
    /// destination with how far and how long; on a duty the diversion and its next via.
    fn pins_head(&self) -> Option<(String, Color)> {
        use crate::nav_pins as pins;
        let first = self.pins.list.first()?;
        if self.pins.arrived.is_some() {
            return Some((format!("{}  ·  {}", self.pins.list.last()?.name, omsi_ui::tr("You have arrived")), ON_TIME));
        }
        let dist = |k: usize| self.pins.dist.get(k).copied().flatten();
        if self.pins.diversion() {
            let d = dist(0).map(|d| format!("  ·  {}", pins::distance_text(d))).unwrap_or_default();
            return Some((format!("{}  ·  {}{d}", omsi_ui::tr("Diversion"), first.name), crate::nav_duty::NOW));
        }
        let last = self.pins.list.len() - 1;
        let parts = match dist(last) {
            Some(d) => format!("  ·  {}  ·  {}", pins::distance_text(d), pins::eta_text(pins::eta(d, self.speed_avg))),
            None => String::new(),
        };
        Some((format!("{}{parts}", self.pins.list[last].name), pins::PIN_LIGHT))
    }

    /// The city map's whole view as the sign-on page, painted for a window `w` x `h` at scale
    /// `s`: its background, the cards (to be clipped to the body under the header, which they
    /// scroll in) and the header with the scroll bar; and the body. Where a press does what is
    /// kept for [`Navigator::map_press`].
    #[allow(clippy::too_many_arguments)]
    fn paint_city_signon(&mut self, f: &NavFrame, companion: &crate::companion::CompanionState, duty: Option<&crate::nav_duty::DutyState>, w: f32, h: f32, s: f32) -> (Painter, Painter, Painter, Rect) {
        use crate::nav_signon as signon;
        let head_h = (44.0 * s).round();
        let body = Rect::new(0.0, head_h, w, (h - head_h).max(0.0));
        // (the whole body is the page's: a press there is no drag of the map, the wheel and a
        // drag scroll it)
        self.city.sheet = body;
        if let Some(r) = self.city.phone.wants(companion) {
            crate::companion::request(r);
        }
        let qr = self.pair_qr(companion);
        let page = signon::page_in(companion, &self.city.phone, duty, f.time, signon::Room::Full, qr.as_ref());
        let full = signon::full(&page, body, s, &self.fonts);
        let max = (full.height - body.h).max(0.0);
        self.city.phone.scroll = self.city.phone.scroll.clamp(0.0, max);
        // (scrolled by whole pixels: the QR code's modules stay sharp)
        let scroll = self.city.phone.scroll.round();
        let mut bg = Painter::new();
        bg.rounded(Rect::new(0.0, 0.0, w, h), 10.0 * s, Color::rgba(12, 16, 26, crate::ui::backdrop(self.opacity).min(1.0)));
        // (a soft light behind the cards: the page is what the view is about now)
        bg.radial(body.center(), body.w.min(body.h) * 0.5, crate::nav_duty::accent().alpha(0.08), Color::rgba(12, 16, 26, 0.0));
        let mut list = Painter::new();
        signon::draw_full(&mut crate::nav_duty::Pen { p: &mut list, atlas: &mut self.atlas, fonts: &self.fonts }, &page, &full, scroll, body, s);
        self.city.phone_hits = signon::full_hits(&page, &full, scroll, s);
        self.city.phone_view = body;
        let mut ui = Painter::new();
        if max > 0.0 {
            let thumb = (body.h * body.h / full.height).max(24.0 * s);
            let y = body.y + (body.h - thumb) * scroll / max;
            ui.rounded(Rect::new(body.right() - 8.0 * s, y, 3.0 * s, thumb), 1.5 * s, Color::WHITE.alpha(0.18));
        }
        // the header: the duty's line, the size, the map instead of the page, closing
        self.city_head(&mut ui, f, w, s, &[("close", 3), ("map", 6)], Left::SignOn, companion.stage, false);
        ui.rounded_border(Rect::new(0.0, 0.0, w, h), 10.0 * s, 1.0, Color::WHITE.alpha(0.08));
        (bg, list, ui, body)
    }

    /// The city map as the sign-on page, its whole view (while the duty waits to be signed
    /// for): the header, and under it the page in a card with the devices' card beside it -
    /// scrolled with the wheel or a drag when they are taller than the view. The window is
    /// at `win` (x, y, width, height), its interface at scale `s`.
    #[allow(clippy::too_many_arguments)]
    fn city_signon(&mut self, renderer: &Renderer, scene: &mut Scene, f: &NavFrame, companion: &crate::companion::CompanionState, duty: Option<&crate::nav_duty::DutyState>, win: [f32; 4], s: f32) {
        let [x0, y0, w, h] = win;
        let (bg, list, ui, body) = self.paint_city_signon(f, companion, duty, w, h, s);
        let (tw, th) = (w as u32, h as u32);
        let Some((tex, _, _)) = self.city.target else { return };
        let Some(view) = renderer.texture_view(scene, tex) else { return };
        let (Some(gpu), device, queue) = (self.gpu.as_mut(), &renderer.device, &renderer.queue) else { return };
        let n_bg = bg.len();
        let mut all = bg.verts;
        let n_list = all.len() as u32;
        all.extend(list.verts);
        let n_ui = all.len() as u32;
        all.extend(ui.verts);
        gpu.upload(device, queue, 5, &all);
        gpu.upload_atlas(queue, &mut self.atlas);
        let layers = [Layer::flat([0.0, 0.0, w, h], 10.0 * s, 1.0), Layer::flat([body.x, body.y, body.right(), body.bottom()], 0.0, 1.0)];
        let draws = [
            Draw { buffer: 5, range: 0..n_bg, layer: 0, texture: 0 },
            Draw { buffer: 5, range: n_list..n_ui, layer: 1, texture: 0 },
            Draw { buffer: 5, range: n_ui..all.len() as u32, layer: 0, texture: 0 },
        ];
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("city map sign-on") });
        gpu.render(device, queue, &mut enc, &view, (tw, th), Some(wgpu::Color::TRANSPARENT), &layers, &draws);
        queue.submit([enc.finish()]);
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
        let back = omsi_launcher_lib::nav_rect_parts(&n.take_rect().unwrap()).0.unwrap();
        assert!((back[0] - 0.5).abs() < 1e-3 && (back[1] - 0.5).abs() < 1e-3);
        assert_eq!(n.take_rect(), None, "handed over once");
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

    /// Over the duty sheet beside the city map the wheel scrolls its list and a press does
    /// not drag the map; its button in the header puts it away.
    #[test]
    fn the_duty_sheet_takes_the_wheel_and_the_press() {
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        n.toggle_map();
        n.city.rect = [100.0, 50.0, 1100.0, 850.0];
        n.city.sheet = Rect::new(12.0, 56.0, 340.0, 700.0);
        n.city.sheet_scale = 1.0;
        n.city.sheet_scroll = 100.0;
        let mpp = n.city.mpp;
        n.map_wheel(1.0, 200.0, 300.0);
        assert_eq!((n.city.sheet_scroll, n.city.mpp), (52.0, mpp));
        n.map_wheel(5.0, 200.0, 300.0);
        assert_eq!(n.city.sheet_scroll, 0.0);
        n.map_press(200.0, 300.0);
        assert!(n.city.drag.is_none() && n.city.open);
        // (dragged with a finger, as a list on a phone)
        n.map_move(200.0, 260.0);
        assert_eq!((n.city.sheet_scroll, n.city.center), (40.0, DVec2::ZERO));
        n.map_release();
        n.map_press(800.0, 300.0);
        assert!(n.city.drag.is_some());
        n.map_release();
        n.city.buttons = vec![(Rect::new(900.0, 7.0, 30.0, 30.0), 4)];
        n.city.shown = Left::Duty;
        n.map_press(1015.0, 72.0);
        assert!(n.city.shown == Left::Nothing && n.city.drag.is_none());
        assert_eq!(left_page(n.city.left, n.city.stage, true), Left::Nothing, "put away");
    }

    /// The left column shows the sign-on page until the duty is signed, then the duty
    /// sheet; a button chooses for as long as the stage is the same, a second press on it
    /// puts the column away.
    #[test]
    fn the_left_column_follows_the_signing_on() {
        assert_eq!(left_page(None, Stage::SignOn, true), Left::SignOn);
        // (no duty to sign for, not signed on: the map - the page would cover all of it)
        assert_eq!(left_page(None, Stage::SignOn, false), Left::Nothing);
        assert_eq!(left_page(None, Stage::DutyMenu, false), Left::SignOn);
        assert_eq!(left_page(None, Stage::DutyOrder, true), Left::SignOn);
        assert_eq!(left_page(None, Stage::OnDuty, true), Left::Duty);
        assert_eq!(left_page(None, Stage::OnDuty, false), Left::Nothing, "a free drive: the whole map");
        let mut c = CityMap { stage: Stage::SignOn, shown: Left::SignOn, ..CityMap::default() };
        c.choose(Left::Duty);
        assert_eq!(left_page(c.left, Stage::SignOn, true), Left::Duty);
        // (no duty: no sheet)
        assert_eq!(left_page(c.left, Stage::SignOn, false), Left::Nothing);
        // signed on (here or on a phone): the choice was for the stage before
        assert_eq!(left_page(c.left, Stage::DutyOrder, true), Left::SignOn);
        c.shown = Left::SignOn;
        c.choose(Left::SignOn);
        assert_eq!((c.shown, left_page(c.left, Stage::SignOn, true)), (Left::Nothing, Left::Nothing));
        c.choose(Left::SignOn);
        assert_eq!(c.shown, Left::SignOn);
    }

    /// On the sign-on page a press works its key, as the keyboard's digits do while it asks
    /// for the number; a press beside the page's scrolling part does nothing.
    #[test]
    fn the_sign_on_page_takes_presses_and_keys() {
        use crate::nav_signon::Action;
        use winit::keyboard::KeyCode;
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        n.toggle_map();
        n.city.rect = [100.0, 50.0, 1100.0, 850.0];
        n.city.sheet = Rect::new(12.0, 56.0, 340.0, 700.0);
        n.city.shown = Left::SignOn;
        n.city.phone_view = Rect::new(12.0, 122.0, 340.0, 626.0);
        n.city.phone_hits = vec![(Rect::new(28.0, 200.0, 98.0, 48.0), Action::Digit(4)), (Rect::new(28.0, 90.0, 98.0, 48.0), Action::Digit(9))];
        n.map_press(100.0 + 60.0, 50.0 + 220.0);
        assert_eq!(n.city.phone.typed, "4");
        assert!(n.city.drag.is_none());
        // (above the scrolling part: the key scrolled out of view under the head)
        n.map_press(100.0 + 60.0, 50.0 + 100.0);
        assert_eq!(n.city.phone.typed, "4");
        assert!(n.page_key(KeyCode::Digit8));
        assert!(n.page_key(KeyCode::Numpad2));
        assert!(n.page_key(KeyCode::Backspace));
        assert_eq!(n.city.phone.typed, "48");
        assert!(!n.page_key(KeyCode::KeyW), "not the keypad's");
        n.city.stage = Stage::OnDuty;
        assert!(!n.page_key(KeyCode::Digit1), "nothing to type at work");
        n.city.open = false;
        n.city.stage = Stage::SignOn;
        assert!(!n.page_key(KeyCode::Digit1), "the map is shut");
    }

    /// The small navigator as the sign-on page (while a duty waits to be signed for): a press
    /// on its keypad's key works it - no click that opens the city map, no drag - and the
    /// digit keys work it while it asks for the number or the code; elsewhere on it a press is
    /// the navigator's as ever.
    #[test]
    fn the_small_navigator_signs_on_too() {
        use crate::nav_signon::{self as signon, Action, Room};
        use winit::keyboard::KeyCode;
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        let fonts = Fonts::hanken();
        let st = crate::companion::CompanionState { personnel_number: "482913".into(), personnel_code: "5821".into(), ..Default::default() };
        assert!(signon::waiting(&st));
        let page = signon::page_in(&st, &n.city.phone, None, 0.0, Room::Panel, None);
        let fit = signon::fit_panel(page, 360.0, 1.0, 900.0, &fonts);
        let height = fit.height;
        let hits = signon::panel_hits(&fit, 0.0);
        let four = hits.iter().find(|h| h.1 == Action::Digit(4)).unwrap().0.center();
        n.signing = Some(PanelPage { fit, hits, stage: Stage::SignOn });
        n.panel_rect = [10.0, 300.0, 370.0, 300.0 + height];
        n.panel_room = [1240.0, 600.0];
        // a press on the 4: typed, and nothing else
        n.panel_press(10.0 + four.x, 300.0 + four.y);
        assert_eq!(n.city.phone.typed, "4");
        assert_eq!(n.panel_release(), None, "no click, no drag");
        assert!(!n.city.open);
        // the keys, as on the city map
        assert!(n.keypad_shown());
        assert!(n.page_key(KeyCode::Digit8) && n.page_key(KeyCode::Numpad2));
        assert_eq!(n.city.phone.typed, "482");
        assert!(!n.page_key(KeyCode::KeyW));
        // the head is no key: a click there opens the city map, as on the map
        n.panel_press(30.0, 310.0);
        assert_eq!(n.panel_release(), Some(false));
        // the duty order on it: no keypad for the keys
        n.signing.as_mut().unwrap().stage = Stage::DutyOrder;
        assert!(!n.page_key(KeyCode::Digit1));
        // the navigator off: none
        n.signing.as_mut().unwrap().stage = Stage::SignOn;
        n.enabled = false;
        assert!(!n.keypad_shown());
    }

    /// Ctrl + the wheel over the small navigator or the city map sizes the navigator by the
    /// setting's steps (a touchpad's parts of a notch add up), within its range, as do the
    /// city map's - and +; the size is handed over once to be kept. Without Ctrl the wheel
    /// over the small navigator is not the navigator's, and on the city map it zooms.
    #[test]
    fn ctrl_and_the_wheel_size_the_navigator() {
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        n.panel_rect = [10.0, 600.0, 370.0, 890.0];
        assert!(!n.wheel(1.0, 100.0, 700.0, false, true), "the wheel alone is the cab's");
        assert!(!n.wheel(1.0, 900.0, 700.0, true, true), "not over the navigator");
        assert!(!n.wheel(1.0, 100.0, 700.0, true, false), "not in VR's cab");
        assert!(n.wheel(2.0, 100.0, 700.0, true, true));
        assert!((n.size - 1.1).abs() < 1e-4, "{}", n.size);
        assert_eq!(n.take_resized().map(|v| (v * 100.0).round()), Some(110.0));
        assert_eq!(n.take_resized(), None, "handed over once");
        // a touchpad: parts of a notch
        for _ in 0..3 {
            n.wheel(-0.4, 100.0, 700.0, true, true);
        }
        assert!((n.size - 1.05).abs() < 1e-4, "{}", n.size);
        for _ in 0..40 {
            n.wheel(-1.0, 100.0, 700.0, true, true);
        }
        assert!((n.size - 0.6).abs() < 1e-4, "{}", n.size);
        // the city map: the wheel zooms, Ctrl + the wheel sizes; its - and +
        n.toggle_map();
        n.city.rect = [100.0, 50.0, 1100.0, 850.0];
        let mpp = n.city.mpp;
        n.take_resized();
        assert!(n.wheel(1.0, 800.0, 400.0, false, true) && n.city.mpp < mpp && n.take_resized().is_none());
        assert!(n.wheel(1.0, 800.0, 400.0, true, true) && (n.size - 0.65).abs() < 1e-4);
        n.city.buttons = vec![(Rect::new(500.0, 7.0, 30.0, 30.0), 7), (Rect::new(600.0, 7.0, 30.0, 30.0), 8)];
        n.map_press(100.0 + 615.0, 50.0 + 20.0);
        n.map_press(100.0 + 615.0, 50.0 + 20.0);
        assert!((n.size - 0.75).abs() < 1e-4 && n.city.drag.is_none());
        n.map_press(100.0 + 515.0, 50.0 + 20.0);
        assert!((n.size - 0.7).abs() < 1e-4);
        assert_eq!(n.take_resized().map(|v| (v * 100.0).round()), Some(70.0));
        assert_eq!(size_text(1.25), "125 %");
    }

    /// A frame for painting: the duty's line, no traffic, no duty board.
    fn painting_frame() -> NavFrame<'static> {
        NavFrame {
            traffic: None,
            players: Vec::new(),
            bus: DVec3::ZERO,
            heading: 0.0,
            speed_kmh: 0.0,
            outside_temp: 15.0,
            inside_temp: 15.0,
            line: Some("307".into()),
            terminus: Some("Markgraf-Berthold-Platz".into()),
            stops: Vec::new(),
            delay: None,
            duty: None,
            passengers: None,
            stop_requested: false,
            time: 19.0 * 3600.0 + 5.0 * 60.0,
            weekday: 3,
            language: "NLD",
            screen: (1920.0, 1080.0),
            ui_scale: 1.0,
            follow_window: true,
            dt: 1.0 / 60.0,
            info_rect: None,
        }
    }

    /// The city map's whole view as the sign-on page, painted at the navigator's sizes, in a
    /// wide window and a narrow one: everything within the window, the cards across it; the
    /// header's buttons - closing, the map, the size's - and + - apart from each other and in
    /// it; the page's keys where a press finds them.
    #[test]
    fn the_city_map_paints_the_sign_on_page_within_its_window() {
        use crate::nav_signon::tests::{hans, listening, made_up_qr};
        let st = listening(hans());
        let f = painting_frame();
        for (w, h) in [(1536.0f32, 886.0f32), (760.0, 620.0)] {
            for size in [0.6, 1.0, 2.0] {
                let mut n = Navigator::new(true, 0.85, "bottom-left");
                n.size = size;
                n.qr = (qr_key(&st), 0.0, Some(made_up_qr(29)));
                let s = (h / 760.0).clamp(0.95, 2.0) * size;
                n.atlas.begin_frame();
                let (bg, list, ui, body) = n.paint_city_signon(&f, &st, None, w, h, s);
                assert!(body.y > 0.0 && body.bottom() <= h + 0.01);
                for v in bg.verts.iter().chain(&ui.verts) {
                    assert!(v.pos[0] >= -0.01 && v.pos[0] <= w + 0.01 && v.pos[1] >= -0.01 && v.pos[1] <= h + 0.01, "{v:?} at {size} in {w}x{h}");
                }
                assert!(list.verts.iter().all(|v| v.pos[0] >= 0.0 && v.pos[0] <= w), "the cards across the window");
                let mut ids: Vec<u8> = n.city.buttons.iter().map(|b| b.1).collect();
                ids.sort_unstable();
                assert_eq!(ids, [3, 6, 7, 8]);
                for (i, (a, _)) in n.city.buttons.iter().enumerate() {
                    assert!(a.x >= 0.0 && a.right() <= w && a.bottom() <= body.y + 0.01);
                    for (b, _) in &n.city.buttons[i + 1..] {
                        assert!(a.right() <= b.x + 0.01 || b.right() <= a.x + 0.01, "{a:?} on {b:?}");
                    }
                }
                let five = n.city.phone_hits.iter().find(|h| h.1 == crate::nav_signon::Action::Digit(5)).map(|h| h.0);
                assert!(five.is_some_and(|r| body.contains(r.center()) || size > 1.5), "{five:?} at {size}");
            }
        }
    }

    /// Pictures of the navigator as the sign-on page, as the game paints it (with its header
    /// and its size): the city map's whole view at 100 % and 130 %, and the small navigator,
    /// in Dutch: `OMSI_NAV_SIGNON_PREVIEW=<folder> cargo test -p omsi-app --lib navigator --
    /// --ignored`.
    #[test]
    #[ignore]
    fn preview_sign_on_pictures() {
        use crate::nav_signon::tests::{hans, listening, made_up_qr};
        let Ok(dir) = std::env::var("OMSI_NAV_SIGNON_PREVIEW") else { return };
        crate::ui_language("NLD");
        let st = listening(hans());
        let f = painting_frame();
        let raster = crate::nav_duty::tests::raster;
        let all = Rect::new(0.0, 0.0, 1e5, 1e5);
        for (name, size) in [("city-signon", 1.0f32), ("city-signon-130", 1.3)] {
            let mut n = Navigator::new(true, 0.85, "bottom-left");
            n.atlas = Atlas::new(2048);
            n.size = size;
            n.qr = (qr_key(&st), 0.0, Some(made_up_qr(29)));
            n.city.phone.typed = "48".into();
            // (just made larger: the size lit in the header)
            n.size_at = 0.0;
            let (w, h) = (1536.0, 886.0);
            let s = (h / 760.0f32).clamp(0.95, 2.0) * size;
            n.atlas.begin_frame();
            let (bg, list, ui, body) = n.paint_city_signon(&f, &st, None, w, h, s);
            // (the game behind it: a grey)
            let mut img = image::RgbaImage::from_pixel(w as u32, h as u32, image::Rgba([70, 84, 96, 255]));
            raster(&bg.verts, &n.atlas, &mut img, all);
            raster(&list.verts, &n.atlas, &mut img, body);
            raster(&ui.verts, &n.atlas, &mut img, all);
            img.save(format!("{dir}/{name}.png")).unwrap();
        }
        // the small navigator: as the frame lays it out at a 1080p window
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        n.atlas = Atlas::new(2048);
        n.qr = (qr_key(&st), 0.0, Some(made_up_qr(29)));
        let pw = (1080.0f32 * 0.33).round();
        // (and in a short window, its page scrolled a little: the bar says so)
        for (name, room, scroll) in [("panel-live", 1080.0 - 2.0 * 19.0, 0.0), ("panel-short", 330.0, 40.0)] {
            n.city.phone.scroll = scroll;
            let page = n.panel_page(&st, None, 0.0, pw, pw / 360.0, room, false);
            let ph = page.fit.height.round();
            n.signing = Some(page);
            n.size_at = if scroll > 0.0 { f32::MIN } else { 0.0 };
            n.atlas.begin_frame();
            let (bg, ui, list, view) = n.paint_signon(pw, ph).unwrap();
            let mut img = image::RgbaImage::from_pixel(pw as u32 + 40, ph as u32 + 40, image::Rgba([70, 84, 96, 255]));
            let shift = |p: Painter| p.verts.into_iter().map(|mut v| { v.pos[0] += 20.0; v.pos[1] += 20.0; v }).collect::<Vec<_>>();
            raster(&shift(bg), &n.atlas, &mut img, all);
            raster(&shift(list), &n.atlas, &mut img, Rect::new(view.x + 20.0, view.y + 20.0, view.w, view.h));
            raster(&shift(ui), &n.atlas, &mut img, all);
            img.save(format!("{dir}/{name}.png")).unwrap();
        }
    }

    /// While the duty waits to be signed for the city map is the sign-on page: no map to drag
    /// or to point at under it; its map button puts it away for the map (the badge brings it
    /// back), until the stage changes.
    #[test]
    fn the_city_map_is_the_sign_on_page_until_the_duty_is_signed() {
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        n.toggle_map();
        n.city.rect = [100.0, 50.0, 1100.0, 850.0];
        n.city.stage = Stage::SignOn;
        n.city.shown = left_page(n.city.left, Stage::SignOn, true);
        assert!(n.full_view());
        n.city.sheet = Rect::new(0.0, 44.0, 1000.0, 756.0);
        n.city.buttons = vec![(Rect::new(950.0, 7.0, 30.0, 30.0), 3), (Rect::new(912.0, 7.0, 30.0, 30.0), 6)];
        assert_eq!(n.map_point(600.0, 400.0), None, "no map to put the bus on");
        // (the header beside its buttons: no drag of a map that is not there)
        n.map_press(100.0 + 400.0, 50.0 + 20.0);
        assert!(n.city.drag.is_none() && n.city.open);
        // the map button: the map, for as long as the stage is this one
        n.map_press(100.0 + 925.0, 50.0 + 20.0);
        assert_eq!(left_page(n.city.left, Stage::SignOn, true), Left::Nothing);
        assert_eq!(left_page(n.city.left, Stage::DutyOrder, true), Left::SignOn, "signed on: the duty order on the page");
        n.city.shown = Left::Nothing;
        assert!(!n.full_view());
        assert!(n.map_point(600.0, 400.0).is_some());
        // at work the page is the column again
        n.city.shown = Left::SignOn;
        n.city.stage = Stage::OnDuty;
        assert!(!n.full_view());
    }

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

    // --- the player's own pins (`nav_pins`) --------------------------------------------------

    use crate::nav_pins::tests::{find, grid};
    use crate::nav_pins::{CardHit, Note, Op, Pin, Role};

    /// A frame with the bus at `bus` heading `heading`, no duty.
    fn at(bus: (f64, f64), heading: f64) -> NavFrame<'static> {
        NavFrame { bus: DVec3::new(bus.0, bus.1, 0.0), heading, line: None, terminus: None, ..painting_frame() }
    }

    fn on_grid(n: &mut Navigator, blocks: i32) -> std::sync::Arc<Network> {
        let net = std::sync::Arc::new(grid(blocks));
        n.global = Some(net.clone());
        n.global_version = 1;
        net
    }

    /// A free drive: the pins make the route (the destination set first, a via before it, a
    /// click far from any road no pin); the free drive's `clear_route` leaves it; the via is
    /// passed and dropped, the destination reached, and a moment later route and pins are gone.
    #[test]
    fn a_free_drive_goes_through_the_pins_to_the_destination() {
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        let net = on_grid(&mut n, 3);
        n.pins.ops = vec![Op::Add(DVec2::new(398.5, 300.0)), Op::Add(DVec2::new(100.0, 198.5)), Op::Add(DVec2::new(100.0, 100.0))];
        n.follow(&at((1.5, 20.0), 0.0));
        assert_eq!(n.pins.note.map(|x| x.0), Some(Note::NoRoad), "the middle of a block");
        assert_eq!(n.pins.list.len(), 2);
        assert_eq!((n.pins.role(0), n.pins.role(1)), (Role::Via(1), Role::Destination));
        assert_eq!(n.route.key, DEST_KEY);
        assert!(n.route.on_route && n.route.joined && n.route.lanes.len() > 3, "{:?}", n.route.lanes);
        assert!(n.route.end.is_some());
        n.pin_distances(&net);
        let d: Vec<f64> = n.pins.dist.iter().map(|d| d.unwrap()).collect();
        assert!(d[0] < d[1] && (d[0] - (180.0 + 100.0 + 1.5)).abs() < 5.0, "{d:?}");
        // the card and the header say so
        let card = n.pin_card(&at((1.5, 20.0), 0.0));
        assert!(!card.diversion && card.rows.len() == 2 && card.total.is_some_and(|t| t.0 == d[1]));
        assert!(n.pins_head().is_some_and(|h| h.0.starts_with(&n.pins.list[1].name)));
        // (a free drive's frame clears the route of no trip: not this one)
        n.clear_route();
        assert_eq!(n.route.key, DEST_KEY);
        // past the via: dropped, said
        n.follow(&at((100.0, 198.5), 90.0));
        assert_eq!(n.pins.note.map(|x| x.0), Some(Note::Reached(1)));
        assert_eq!(n.pins.list.len(), 1);
        assert!(n.route.on_route);
        // at the destination (on the other side of its street): arrived; a moment later all gone
        n.follow(&at((401.5, 290.0), 0.0));
        assert_eq!(n.pins.note.map(|x| x.0), Some(Note::Arrived));
        assert!(n.pins.arrived.is_some() && !n.route.lanes.is_empty());
        n.follow(&NavFrame { dt: crate::nav_pins::ARRIVED_FOR + 0.1, ..at((401.5, 295.0), 0.0) });
        assert!(n.pins.list.is_empty() && n.route.lanes.is_empty() && n.route.key.is_empty());
        // removing the last pin ends the drive too
        n.pins.ops = vec![Op::Add(DVec2::new(398.5, 300.0))];
        n.follow(&at((1.5, 20.0), 0.0));
        assert_eq!(n.route.key, DEST_KEY);
        n.pins.ops = vec![Op::Remove(0)];
        n.follow(&at((1.5, 20.0), 0.0));
        assert!(n.route.lanes.is_empty() && n.route.key.is_empty());
    }

    /// A destination no road leads to: said once, no route (and no "recalculated"), tried
    /// again now and then without saying it again; the pin stays for the player to move.
    #[test]
    fn a_destination_no_way_leads_to_is_said_once() {
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        let mut net = grid(2);
        net.lanes.push(crate::nav_pins::tests::lane((5000.0, 0.0), (5100.0, 0.0)));
        net.build_grid();
        n.global = Some(std::sync::Arc::new(net));
        n.global_version = 1;
        n.pins.ops = vec![Op::Add(DVec2::new(5050.0, 0.0))];
        let f = at((1.5, 20.0), 0.0);
        n.follow(&f);
        assert_eq!(n.pins.note.map(|x| x.0), Some(Note::NoWay));
        assert!(n.route.lanes.is_empty() && n.pins.list.len() == 1 && n.pins.marks == vec![None]);
        n.pins.note = None;
        for _ in 0..((REROUTE_EVERY * 2.0 / f.dt) as usize + 10) {
            n.follow(&f);
        }
        assert_eq!(n.pins.note, None, "not said again");
        assert!(n.route.lanes.is_empty() && n.route.note == 0.0);
        n.clear_route();
        assert_eq!(n.pins.list.len(), 1);
    }

    /// Off the pins' route the navigator plans again from the bus through the pins ahead.
    #[test]
    fn off_the_pins_route_it_is_planned_again_from_the_bus() {
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        on_grid(&mut n, 3);
        n.pins.ops = vec![Op::Add(DVec2::new(598.5, 500.0))];
        let f = at((1.5, 20.0), 0.0);
        n.follow(&f);
        let first = n.route.lanes.clone();
        // the bus went the wrong way: east along the map's southern edge
        let wrong = at((100.0, -1.5), 90.0);
        for _ in 0..(((OFF_ROUTE_AFTER + REROUTE_EVERY) / f.dt) as usize + 10) {
            n.follow(&wrong);
        }
        assert_ne!(n.route.lanes, first);
        assert!(n.route.on_route && n.route.note > 0.0, "recalculated");
        assert_eq!(n.pins.list.len(), 1, "the destination stays");
        assert_eq!(n.pins.marks.len(), 1);
    }

    /// On a duty the pins are a diversion: the trip's route key stays (the timetable is not
    /// asked again), the route goes through the via and back onto the trip's; the timetable's
    /// route given again leaves it; "Clear diversion" gives the trip's route back with the bus
    /// on it; and a diversion ends with its trip.
    #[test]
    fn a_diversion_is_the_navigations_alone_and_ends_with_its_trip() {
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        let net = on_grid(&mut n, 3);
        let route: Vec<usize> = (0..3).map(|j| find(&net, (1.5, j as f64 * 200.0), (1.5, (j + 1) as f64 * 200.0))).collect();
        let g = n.global_version + (1 << 40);
        n.set_route("0/trip", route.clone(), true, g);
        let f = at((1.5, 50.0), 0.0);
        n.follow(&f);
        assert!(n.route.on_route);
        n.pins.ops.push(Op::Add(DVec2::new(201.5, 300.0)));
        n.follow(&f);
        assert!(n.pins.diverts("0/trip"));
        assert_eq!(n.pins.role(0), Role::Via(1));
        assert_eq!(n.route.key, "0/trip");
        assert!(!n.wants_route("0/trip", 0), "the trip's route is not asked for again");
        let join = n.pins.join.expect("back onto the trip's route");
        assert_eq!(&n.route.lanes[join..], &route[2..]);
        assert_eq!(n.route.end, None);
        assert!(n.pin_card(&f).diversion);
        assert!(n.pins_head().is_some_and(|h| h.1 == crate::nav_duty::NOW));
        let diverted = n.route.lanes.clone();
        n.set_route("0/trip", route.clone(), true, g);
        assert_eq!(n.route.lanes, diverted, "the timetable's route again: the diversion stays");
        // cleared: the trip's route, the bus found on it
        n.pins.ops.push(Op::Clear);
        n.follow(&f);
        assert!(n.pins.list.is_empty() && !n.pins.diversion());
        assert_eq!((&n.route.lanes, n.route.progress, n.route.on_route), (&route, 0, true));
        // the via reached: on along the trip's route from where it rejoins
        n.pins.ops.push(Op::Add(DVec2::new(201.5, 300.0)));
        n.follow(&f);
        n.follow(&at((201.5, 295.0), 0.0));
        assert_eq!(n.pins.note.map(|x| x.0), Some(Note::Reached(1)));
        assert!(!n.pins.diversion() && n.route.lanes.ends_with(&route[2..]));
        // a diversion ends with its trip: the next trip, or none
        n.set_route("0/trip", route.clone(), true, g + 1);
        n.pins.ops.push(Op::Add(DVec2::new(201.5, 300.0)));
        n.follow(&f);
        assert!(n.pins.diverts("0/trip"));
        n.set_route("1/next", route.clone(), true, g);
        assert!(n.pins.list.is_empty() && n.route.lanes == route);
        n.pins.ops.push(Op::Add(DVec2::new(201.5, 300.0)));
        n.follow(&f);
        assert!(n.pins.diverts("1/next"));
        n.clear_route();
        assert!(n.pins.list.is_empty() && n.route.lanes.is_empty());
    }

    /// The city map's pin tool: a click sets a pin (a drag of the map, a click on the header
    /// none); a right-click sets one without the tool and takes the one under it away; a pin
    /// is dragged by as far as the cursor went; the card's buttons work and it takes no drag
    /// or wheel.
    #[test]
    fn pins_are_set_moved_and_taken_away_on_the_city_map() {
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        n.toggle_map();
        n.city.rect = [100.0, 50.0, 1100.0, 850.0];
        n.city.center = DVec2::ZERO;
        n.city.mpp = 1.0;
        n.city.sheet_scale = 1.0;
        n.city.buttons = vec![(Rect::new(900.0, 7.0, 30.0, 30.0), 9)];
        // the window's middle is the map's centre
        n.map_press(600.0, 450.0);
        n.map_release();
        assert!(n.pins.ops.is_empty() && !n.city.pin_tool, "no tool: a drag");
        n.map_press(100.0 + 915.0, 50.0 + 20.0);
        assert!(n.city.pin_tool);
        n.map_press(600.0, 450.0);
        n.map_release();
        assert_eq!(n.pins.ops, vec![Op::Add(DVec2::ZERO)]);
        n.pins.ops.clear();
        n.map_press(600.0, 450.0);
        n.map_move(640.0, 450.0);
        n.map_release();
        assert!(n.pins.ops.is_empty(), "a drag of the map");
        n.city.center = DVec2::ZERO;
        n.map_press(400.0, 60.0);
        n.map_release();
        assert!(n.pins.ops.is_empty(), "the header");
        // a right-click, without the tool
        n.city.pin_tool = false;
        n.map_right_press(700.0, 450.0);
        assert_eq!(n.pins.ops, vec![Op::Add(DVec2::new(100.0, 0.0))]);
        n.pins.ops.clear();
        n.map_right_press(400.0, 60.0);
        assert!(n.pins.ops.is_empty(), "the header");
        n.pins.list = vec![Pin { at: DVec2::new(100.0, 0.0), name: "Markt".into(), n: 1 }];
        n.map_right_press(702.0, 452.0);
        assert_eq!(n.pins.ops, vec![Op::Remove(0)]);
        n.pins.ops.clear();
        // dragged by its flag's head: moved as far as the cursor went
        let head = crate::nav_pins::destination_head(Vec2::new(600.0, 400.0), crate::nav_pins::DEST_R);
        n.map_press(100.0 + head.x, 50.0 + head.y);
        assert!(n.city.drag.is_none() && n.city.pin_drag.is_some());
        n.map_move(100.0 + head.x + 50.0, 50.0 + head.y - 20.0);
        n.map_release();
        assert_eq!(n.pins.ops, vec![Op::Move(0, DVec2::new(150.0, 20.0), DVec2::new(100.0, 0.0))]);
        assert_eq!(n.pins.list[0].at, DVec2::new(150.0, 20.0), "where it was let go, until snapped");
        n.pins.ops.clear();
        // a press on a pin without moving: nothing
        n.map_press(100.0 + 650.0, 50.0 + 380.0);
        n.map_release();
        assert!(n.pins.ops.is_empty());
        // the card
        n.city.card = Rect::new(12.0, 56.0, 340.0, 200.0);
        n.city.card_hits = vec![(Rect::new(300.0, 120.0, 26.0, 26.0), CardHit::Remove(0)), (Rect::new(270.0, 120.0, 26.0, 26.0), CardHit::Up(1)), (Rect::new(26.0, 210.0, 300.0, 32.0), CardHit::Clear)];
        n.map_press(100.0 + 310.0, 50.0 + 130.0);
        n.map_press(100.0 + 280.0, 50.0 + 130.0);
        n.map_press(100.0 + 50.0, 50.0 + 220.0);
        n.map_press(100.0 + 50.0, 50.0 + 70.0);
        assert_eq!(n.pins.ops, vec![Op::Remove(0), Op::Shift(1, true), Op::Clear]);
        assert!(n.city.drag.is_none());
        let mpp = n.city.mpp;
        n.map_wheel(2.0, 100.0 + 50.0, 50.0 + 70.0);
        assert_eq!(n.city.mpp, mpp);
        // the map shut: the tool too
        n.city.pin_tool = true;
        n.toggle_map();
        assert!(!n.city.pin_tool);
    }

    /// The small navigator's bottom bar says where the pins lead - the destination with how
    /// far, how long and when there; a diversion and its via beside the next stop - within it.
    #[test]
    fn the_bottom_bar_says_where_the_pins_lead() {
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        on_grid(&mut n, 3);
        n.pins.ops = vec![Op::Add(DVec2::new(398.5, 300.0)), Op::Add(DVec2::new(100.0, 198.5))];
        let f = at((1.5, 20.0), 0.0);
        n.follow(&f);
        n.pin_distances(n.global.clone().unwrap().as_ref());
        for s in [0.9f32, 1.0, 2.0] {
            let bottom = Rect::new(0.0, 200.0, 360.0 * s, 46.0 * s);
            let mut ui = Painter::new();
            n.paint_next(&mut ui, &f, bottom, s, &[]);
            // (the bar, the flag, the name, the way there and the via: a quad each)
            assert_eq!(ui.len(), 5 * 6);
            // (a letter's sprite has a little room round it)
            let e = 2.0 * s;
            for v in &ui.verts {
                assert!(v.pos[0] >= -e && v.pos[0] <= bottom.right() + e && v.pos[1] >= bottom.y - e && v.pos[1] <= bottom.bottom() + e, "{v:?} at {s}");
            }
        }
    }

    /// The map's world vertices (relative to `anchor`) as the city map's flat view shows them:
    /// the pixels of a window `w` x `h` centred on `center`, `mpp` metres a pixel.
    fn flatten(verts: &[omsi_ui::Vertex], anchor: DVec2, center: DVec2, mpp: f64, w: f32, h: f32) -> Vec<omsi_ui::Vertex> {
        verts
            .iter()
            .map(|v| {
                let p = DVec2::new(v.pos[0] as f64, v.pos[1] as f64) + anchor - center;
                let pos = [(w as f64 * 0.5 + p.x / mpp) as f32, (h as f64 * 0.5 - p.y / mpp) as f32, 0.0];
                omsi_ui::Vertex { pos, ext: [v.ext[0], -v.ext[1]], width: [(v.width[0] / mpp as f32).max(v.width[1]), 0.0], mode: [0.0, v.mode[1]], ..*v }
            })
            .collect()
    }

    /// Pictures of the pins as the game paints them, in Dutch:
    /// `OMSI_NAV_PINS_PREVIEW=<folder> cargo test -p omsi-app --lib navigator -- --ignored`.
    /// The city map on a free drive (the pin tool on, a via and a destination, the card in the
    /// left column, a pin being dragged), on a duty with a diversion (the duty sheet in the
    /// column, the card at the right edge, a note), and the small navigator's bottom bar.
    #[test]
    #[ignore]
    fn preview_pin_pictures() {
        let Ok(dir) = std::env::var("OMSI_NAV_PINS_PREVIEW") else { return };
        crate::ui_language("NLD");
        let raster = crate::nav_duty::tests::raster;
        let all = Rect::new(0.0, 0.0, 1e5, 1e5);
        let (w, h) = (1536.0f32, 886.0f32);
        let s = (h / 760.0f32).clamp(0.95, 2.0);
        let signs = vec![(DVec3::new(10.0, 300.0, 0.0), 90.0, "Hauptstraße".to_string()), (DVec3::new(300.0, 190.0, 0.0), 0.0, "Kirchweg".to_string()), (DVec3::new(410.0, 500.0, 0.0), 90.0, "Parkallee".to_string()), (DVec3::new(500.0, 410.0, 0.0), 0.0, "Schulstraße".to_string())];
        let picture = |n: &mut Navigator, f: &NavFrame, duty: Option<&crate::nav_duty::DutyState>, left: Left, name: &str| {
            // (the roads and the route made again: the game keeps them on the GPU)
            n.city.roads = None;
            n.city.route.0 = (u64::MAX, 0, 0, 0);
            n.atlas.begin_frame();
            let companion = crate::nav_signon::tests::listening(crate::nav_signon::tests::hans());
            let p = n.paint_city(f, &companion, duty, left, w, h, s);
            let mut img = image::RgbaImage::from_pixel(w as u32, h as u32, image::Rgba([70, 84, 96, 255]));
            let (c, mpp) = (n.city.center, n.city.mpp);
            raster(&p.bg.verts, &n.atlas, &mut img, all);
            raster(&flatten(&p.roads.unwrap_or_default(), p.anchor, c, mpp, w, h), &n.atlas, &mut img, all);
            raster(&flatten(&p.route.unwrap_or_default(), p.anchor, c, mpp, w, h), &n.atlas, &mut img, all);
            raster(&p.ui.verts, &n.atlas, &mut img, all);
            raster(&p.list.verts, &n.atlas, &mut img, Rect::new(p.list_clip[0], p.list_clip[1], p.list_clip[2] - p.list_clip[0], p.list_clip[3] - p.list_clip[1]));
            img.save(format!("{dir}/{name}.png")).unwrap();
        };
        // a free drive
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        n.atlas = Atlas::new(2048);
        let net = on_grid(&mut n, 3);
        n.streets = Some(std::sync::Arc::new(build_streets(&net, &signs)));
        n.stop_names = Some(vec![(7, "Rathaus".to_string())]);
        n.stop_pos = std::sync::Arc::new([(7, DVec3::new(412.0, 310.0, 0.0))].into_iter().collect());
        n.toggle_map();
        n.city.rect = [0.0, 0.0, w, h];
        n.city.mpp = 0.85;
        n.city.follow = false;
        n.city.center = DVec2::new(122.0, 290.0);
        n.city.pin_tool = true;
        n.speed_avg = 9.0;
        let f = NavFrame { speed_kmh: 32.0, ..at((1.5, 60.0), 0.0) };
        n.pins.ops = vec![Op::Add(DVec2::new(398.5, 300.0)), Op::Add(DVec2::new(100.0, 198.5)), Op::Add(DVec2::new(300.0, 401.5))];
        n.follow(&f);
        n.pin_distances(&net);
        picture(&mut n, &f, None, Left::Nothing, "pins-free");
        // the same, a via being dragged and a note
        n.city.pin_drag = Some(PinDrag { k: 1, from: Vec2::new(500.0, 400.0), at: Vec2::new(560.0, 360.0), moved: true });
        n.pins.say(Note::NoRoad);
        picture(&mut n, &f, None, Left::Nothing, "pins-free-drag");
        n.city.pin_drag = None;
        // the small navigator's bottom bar: on the way, a via reached, arrived; a diversion
        let bars = |n: &mut Navigator, f: &NavFrame, img: &mut image::RgbaImage, y: f32| {
            n.atlas.begin_frame();
            let sb = 1.6f32;
            let mut ui = Painter::new();
            n.paint_next(&mut ui, f, Rect::new(20.0, y, 360.0 * sb, 46.0 * sb), sb, &[]);
            raster(&ui.verts, &n.atlas, img, all);
        };
        let mut img = image::RgbaImage::from_pixel(620, 4 * 90 + 20, image::Rgba([70, 84, 96, 255]));
        n.pins.note = None;
        bars(&mut n, &f, &mut img, 20.0);
        n.pins.say(Note::Reached(1));
        bars(&mut n, &f, &mut img, 110.0);
        n.pins.note = None;
        n.pins.arrived = Some(3.0);
        n.pins.say(Note::Arrived);
        bars(&mut n, &f, &mut img, 200.0);
        // on a duty, diverted
        let mut d = Navigator::new(true, 0.85, "bottom-left");
        d.atlas = Atlas::new(2048);
        let net = on_grid(&mut d, 3);
        d.streets = Some(std::sync::Arc::new(build_streets(&net, &signs)));
        let route: Vec<usize> = (0..3).map(|j| find(&net, (1.5, j as f64 * 200.0), (1.5, (j + 1) as f64 * 200.0))).collect();
        d.set_route("0/trip", route, true, d.global_version + (1 << 40));
        let stop = |name: &str, y: f64, min: f64| NavStop { object_id: 0, position: DVec3::new(4.0, y, 0.0), name: name.into(), arrival: 11.0 * 3600.0 + min * 60.0 };
        let f = NavFrame { line: Some("35".into()), terminus: Some("Diakonissenkrankenhaus".into()), stops: vec![stop("Kirchweg", 300.0, 32.0), stop("Rathaus", 450.0, 35.0), stop("Schule", 590.0, 38.0)], delay: Some(40.0), time: 11.0 * 3600.0 + 30.0 * 60.0, ..at((1.5, 60.0), 0.0) };
        d.follow(&f);
        d.pins.ops = vec![Op::Add(DVec2::new(201.5, 300.0)), Op::Add(DVec2::new(398.5, 450.0))];
        d.follow(&f);
        d.pin_distances(&net);
        d.toggle_map();
        d.city.rect = [0.0, 0.0, w, h];
        d.city.mpp = 0.85;
        d.city.follow = false;
        d.city.center = DVec2::new(285.0, 290.0);
        let (trips, tours) = crate::nav_duty::tests::duty();
        let state = crate::nav_duty::tests::state(&trips, &tours, 0, 3, false, 40.0);
        picture(&mut d, &f, Some(&state), Left::Duty, "pins-diversion");
        bars(&mut d, &f, &mut img, 290.0);
        img.save(format!("{dir}/pins-bar.png")).unwrap();
    }

    /// The map's world vertices as the GPU's shader places them (`ui.wgsl`): through the
    /// layer's view, at least their pixel width at their depth - as pixels, for `raster`.
    fn on_screen(verts: &[omsi_ui::Vertex], l: &Layer) -> Vec<omsi_ui::Vertex> {
        let px = |p: Vec3| {
            let c = l.view_proj * p.extend(1.0);
            // (the GPU clips at the near plane, a metre from the eye: no triangle across it)
            (c.w > 0.999).then(|| Vec2::new(l.viewport[0] + (c.x / c.w * 0.5 + 0.5) * l.viewport[2], l.viewport[1] + (0.5 - c.y / c.w * 0.5) * l.viewport[3]))
        };
        let mut out = Vec::with_capacity(verts.len());
        for t in verts.chunks_exact(3) {
            let pts: Vec<Option<Vec2>> = t
                .iter()
                .map(|v| {
                    let p = Vec3::from_array(v.pos);
                    let c = l.view_proj * p.extend(1.0);
                    let mpp = c.w.max(0.01) * l.px_scale;
                    let w = v.width[0].max(v.width[1] * mpp);
                    px(p + Vec3::new(v.ext[0] * w, v.ext[1] * w, 0.0))
                })
                .collect();
            if pts.iter().all(|p| p.is_some()) {
                for (v, p) in t.iter().zip(pts) {
                    let p = p.unwrap();
                    out.push(omsi_ui::Vertex { pos: [p.x, p.y, 0.0], ext: [0.0, 0.0], width: [0.0, 0.0], mode: [0.0, v.mode[1]], ..*v });
                }
            }
        }
        out
    }

    /// Some traffic round the bus on the grid's street (x 1.5, heading north): buses of each
    /// kind with their lines (one of the duty's own), an articulated one, a tram and cars.
    fn traffic_round() -> Vec<MapVehicle> {
        let v = |x: f64, y: f64, heading: f64, len: f32, width: f32, color: Color, line: Option<&str>| MapVehicle { at: DVec3::new(x, y, 0.0), heading, len, width, color, bus: color != DOT, line: line.map(str::to_string) };
        vec![
            v(1.5, 112.0, 0.0, 12.0, 2.55, BUS, Some("314")),
            v(-1.5, 150.0, 180.0, 18.0, 2.55, BUS, Some("35")),
            v(60.0, 198.5, 90.0, 12.0, 2.55, TROLLEY, Some("1")),
            v(150.0, 201.5, 270.0, 30.0, 2.4, TRAM, Some("8")),
            v(1.5, 88.0, 0.0, 4.4, 1.8, DOT, None),
            v(-1.5, 128.0, 180.0, 4.6, 1.85, DOT, None),
            v(-1.5, 236.0, 180.0, 4.2, 1.75, DOT, None),
            v(28.0, 201.5, 270.0, 4.4, 1.8, DOT, None),
            v(105.0, 198.5, 90.0, 4.8, 1.9, DOT, None),
            v(1.5, 262.0, 30.0, 4.4, 1.8, DOT, None),
        ]
    }

    /// Pictures of the small navigator as the game paints it - in its own size, and dragged to
    /// a narrow tall shape, a wide short one, a square, the smallest and a huge one, each with
    /// the duty board and without - with the other vehicles as shapes, and far out as dots; and
    /// the vehicles' shapes at the city map's zooms. In Dutch:
    /// `OMSI_NAV_PANEL_PREVIEW=<folder> cargo test -p omsi-app --lib navigator -- --ignored`.
    #[test]
    #[ignore]
    fn preview_panel_shapes() {
        use crate::nav_panel as np;
        let Ok(dir) = std::env::var("OMSI_NAV_PANEL_PREVIEW") else { return };
        crate::ui_language("NLD");
        let raster = crate::nav_duty::tests::raster;
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        n.atlas = Atlas::new(2048);
        let net = on_grid(&mut n, 3);
        let signs = vec![(DVec3::new(10.0, 300.0, 0.0), 90.0, "Hauptstraße".to_string()), (DVec3::new(300.0, 190.0, 0.0), 0.0, "Kirchweg".to_string())];
        n.streets = Some(std::sync::Arc::new(build_streets(&net, &signs)));
        let route: Vec<usize> = (0..3).map(|j| find(&net, (1.5, j as f64 * 200.0), (1.5, (j + 1) as f64 * 200.0))).collect();
        n.set_route("0/trip", route, true, n.global_version + (1 << 40));
        let stop = |name: &str, y: f64, min: f64| NavStop { object_id: 0, position: DVec3::new(4.0, y, 0.0), name: name.into(), arrival: 7.0 * 3600.0 + min * 60.0 };
        let f = NavFrame {
            line: Some("314".into()),
            terminus: Some("Großweier".into()),
            stops: vec![stop("Stadttheater", 300.0, 52.0), stop("Altstadtring", 450.0, 53.0), stop("Alter Bahnhof - Altstadtforum", 590.0, 55.0)],
            delay: Some(20.0),
            speed_kmh: 34.0,
            time: 7.0 * 3600.0 + 51.0 * 60.0,
            ..at((1.5, 60.0), 0.0)
        };
        n.follow(&f);
        n.follow(&f);
        n.speed_avg = 9.0;
        n.next_dist = Some(160.0);
        // (as the camera stands at 34 km/h)
        n.zoom = 110.0 + 34.0 * 2.2;
        let (trips, tours) = crate::nav_duty::tests::duty();
        let duty = crate::nav_duty::tests::state(&trips, &tours, 0, 3, false, 20.0);
        let vehicles = traffic_round();
        let unit = 1.0;
        // (the panel painted as the game does, flattened on the CPU over a cab-grey ground)
        let paint = |n: &mut Navigator, img: &mut image::RgbaImage, x0: f32, y0: f32, w: f32, h: f32, board_on: bool, auto: bool| {
            n.roads = None;
            n.route_mesh.0 = u64::MAX;
            let lay = if auto {
                let s = w / np::WIDTH;
                let (rows, bh) = if board_on { crate::nav_duty::board_fitting(Some(&duty), s, 1e4) } else { (Vec::new(), 0.0) };
                let ph = ((w * 0.62).round() + (np::HEAD * s).round() + (np::NEXT * s).round() + bh).round();
                (np::layout(w, ph, s, board_on.then_some(bh), false, true), rows)
            } else {
                let (s, beside) = np::scale(w, h, unit, board_on);
                let (rows, bh) = if board_on { crate::nav_duty::board_within(Some(&duty), s, np::board_room(h, s, beside)) } else { (Vec::new(), 0.0) };
                (np::layout(w, h, s, (!rows.is_empty()).then_some(bh), beside, true), rows)
            };
            let (lay, rows) = lay;
            let h = if auto { lay.next.bottom() + lay.board.map(|b| b.h).unwrap_or(0.0) } else { h };
            n.schedule = board_on;
            let p = n.paint_panel((w, h), &lay, &f, &rows, &vehicles);
            let mut panel = image::RgbaImage::from_pixel(w as u32, h as u32, image::Rgba([70, 84, 96, 255]));
            let all = Rect::new(0.0, 0.0, w, h);
            raster(&p.bg.verts, &n.atlas, &mut panel, all);
            let map = lay.map;
            for world in [p.roads.unwrap_or_default(), p.traffic.verts, p.route.unwrap_or_default(), p.vehicles.verts] {
                raster(&on_screen(&world, &p.map_layer), &n.atlas, &mut panel, map);
            }
            raster(&p.ui.verts, &n.atlas, &mut panel, all);
            image::imageops::overlay(img, &panel, x0 as i64, y0 as i64);
            h
        };
        let shapes: [(&str, f32, f32); 6] = [("own-size", 356.0, 0.0), ("narrow-tall", 280.0, 900.0), ("wide-short", 1100.0, 330.0), ("square", 520.0, 520.0), ("smallest", 220.0, 150.0), ("huge", 1000.0, 1000.0)];
        for (name, w, h) in shapes {
            let auto = h == 0.0;
            let tall = if auto { 900.0 } else { h };
            let mut img = image::RgbaImage::from_pixel((2.0 * w + 60.0) as u32, (tall + 40.0) as u32, image::Rgba([40, 46, 54, 255]));
            for (k, board_on) in [true, false].into_iter().enumerate() {
                paint(&mut n, &mut img, 20.0 + k as f32 * (w + 20.0), 20.0, w, h, board_on, auto);
            }
            img.save(format!("{dir}/panel-{name}.png")).unwrap();
        }
        // far out: the vehicles as dots with their tips
        n.zoom = 900.0;
        let mut img = image::RgbaImage::from_pixel(560, 560, image::Rgba([40, 46, 54, 255]));
        paint(&mut n, &mut img, 20.0, 20.0, 520.0, 520.0, false, false);
        img.save(format!("{dir}/panel-far.png")).unwrap();
        // the shapes themselves at the city map's zooms (north up, metres a pixel), each in a
        // row of its own: 70 px apart, as they head
        let (cw, ch) = (760.0f32, 150.0f32);
        let mpps = [0.08f64, 0.2, 0.6, 1.6, 4.0];
        let mut img = image::RgbaImage::from_pixel(cw as u32, (ch * mpps.len() as f32) as u32, image::Rgba([12, 16, 26, 255]));
        for (k, mpp) in mpps.into_iter().enumerate() {
            let (hw, hh) = (cw as f64 * 0.5 * mpp, ch as f64 * 0.5 * mpp);
            let proj = Mat4::orthographic_rh(-hw as f32, hw as f32, -hh as f32, hh as f32, -1000.0, 1000.0);
            let row = Rect::new(0.0, k as f32 * ch, cw, ch);
            let layer = Layer { view_proj: proj, viewport: [row.x, row.y, row.w, row.h], clip: [0.0; 4], radius: 0.0, opacity: 1.0, px_scale: mpp as f32 };
            let row_of: Vec<MapVehicle> = vehicles.iter().enumerate().map(|(j, v)| MapVehicle { at: DVec3::new((-(cw as f64) * 0.5 + 50.0 + j as f64 * 72.0) * mpp, 0.0, 0.0), ..v.clone() }).collect();
            let mut p = Painter::new();
            paint_vehicles(&mut p, &row_of, &|q: DVec3| Vec3::new(q.x as f32, q.y as f32, 0.0), mpp as f32, 1.2);
            raster(&on_screen(&p.verts, &layer), &n.atlas, &mut img, row);
        }
        img.save(format!("{dir}/vehicles.png")).unwrap();
    }

    /// The board's handle shows and hides the board and is kept; an edge of the panel sizes
    /// it (the place following when the top or left edge goes), a corner both ways, held to
    /// the smallest size and the window; the place and size are kept as `nav_rect` and read
    /// back; Ctrl + the wheel leaves a sized panel's size; the reset forgets it all.
    #[test]
    fn the_navigator_is_sized_by_its_edges_and_its_board_switched() {
        let mut n = Navigator::new(true, 0.85, "bottom-left");
        n.window = [1920.0, 1080.0];
        n.unit = 1.0;
        n.panel_rect = [20.0, 500.0, 380.0, 1060.0];
        n.panel_room = [1560.0, 520.0];
        n.layout = Some(crate::nav_panel::layout(360.0, 560.0, 1.0, Some(240.0), false, true));
        // the handle: the board away and back, kept each time; no click, no drag
        let hd = n.layout.unwrap().handle.unwrap();
        let (hx, hy) = (20.0 + hd.center().x, 500.0 + hd.center().y);
        assert_eq!(n.hover(hx, hy), Some(1));
        n.schedule = true;
        n.panel_press(hx, hy);
        assert_eq!(n.panel_release(), None);
        assert!(!n.schedule && !n.board);
        assert_eq!(n.take_board(), Some(false));
        n.panel_press(hx, hy);
        assert!(n.schedule && n.board && n.take_board() == Some(true) && n.take_board().is_none());
        // the right edge: wider, the place where it was
        assert_eq!(n.hover(378.0, 800.0), Some(5));
        n.panel_press(378.0, 800.0);
        assert!(n.panel_move(478.0, 800.0));
        let c = n.custom.unwrap();
        assert!((c[0] * 1080.0 - 460.0).abs() < 0.5 && (c[1] * 1080.0 - 560.0).abs() < 0.5, "{c:?}");
        assert_eq!(n.panel_release(), Some(true));
        let kept = n.take_rect().unwrap();
        let (at, size) = omsi_launcher_lib::nav_rect_parts(&kept);
        assert!(size.is_some_and(|z| (z[0] * 1080.0 - 460.0).abs() < 0.5));
        let p = crate::nav_panel::place([at.unwrap()[0] as f32, at.unwrap()[1] as f32], 460.0, 560.0, [1920.0, 1080.0]);
        assert!((p.x - 20.0).abs() < 0.6 && (p.y - 500.0).abs() < 0.6, "{p}");
        // the top-left corner, far past the smallest size: held there, the bottom right staying
        n.panel_rect = [20.0, 500.0, 480.0, 1060.0];
        n.panel_press(22.0, 502.0);
        n.panel_move(2000.0, 2000.0);
        let c = n.custom.unwrap();
        assert!((c[0] * 1080.0 - 220.0).abs() < 0.5 && (c[1] * 1080.0 - 150.0).abs() < 0.5, "{c:?}");
        n.panel_release();
        // read back as the game starts
        let mut m = Navigator::new(true, 0.85, "bottom-left");
        m.set_rect(&n.take_rect().unwrap());
        assert!(m.custom.is_some() && m.at.is_some());
        // Ctrl + the wheel: a sized panel keeps its size (its texts follow the size)
        let before = n.custom.unwrap();
        n.resize_by(2);
        assert!(n.custom == Some(before) && n.take_rect().is_none() && n.take_resized().is_some());
        // the middle of it still moves it, the cockpit's display is only moved
        assert_eq!(n.hover(300.0, 700.0), Some(0));
        n.cockpit_display = true;
        assert_eq!(n.hover(378.0, 800.0), Some(0));
        n.cockpit_display = false;
        n.reset_panel();
        assert!(n.custom.is_none() && n.at.is_none() && n.board && n.size == 1.0);
        assert_eq!(n.rect_setting(), "");
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
}

// --- for the phone and tablet companion (`companion::nav`): what the navigator follows, read
// only, so that a phone or tablet shows the same route, turn, stop and note as the panel

impl Navigator {
    /// What the navigator follows now, as the companion hands it to a phone or tablet: the
    /// route's network (the whole map's, else the traffic's), the route's lanes with the one
    /// the bus is on and how far along it, the note the panel shows, the next turn, the way to
    /// the next stop, the street, the speed limit at the bus and what the jams ahead cost.
    pub(crate) fn companion_look<'a>(&'a self, traffic: Option<&'a Traffic>, bus: DVec3) -> crate::companion::NavLook<'a> {
        use crate::companion::RouteNote;
        let net = self.global.as_deref().or(traffic.map(|t| &t.net)).or(self.own_net.as_deref());
        let r = &self.route;
        // (as the panel's bottom bar has it)
        let note = if r.note > 0.0 {
            Some(RouteNote::Recalculated)
        } else if r.joined && !r.lanes.is_empty() && !r.on_route && r.off_for > OFF_ROUTE_AFTER {
            Some(if r.off_for < OFF_ROUTE_AFTER + 20.0 { RouteNote::Rerouting } else { RouteNote::OffRoute })
        } else {
            None
        };
        // (as the panel's speed limit sign has it)
        let limit = net.and_then(|n| {
            let lane = if r.on_route { r.lanes.get(r.progress).copied() } else { None };
            let lane = lane.or_else(|| n.nearest_lane_near(bus, LaneKind::Street).filter(|l| l.2 < 8.0).map(|l| l.0))?;
            let v = n.lanes.get(lane)?.speed_limit_kmh;
            (v > 1.0 && v < 200.0).then_some(v)
        });
        crate::companion::NavLook {
            net: self.global.as_deref().or(traffic.map(|t| &t.net)),
            map: self.global.clone().map(|g| (self.global_version, g)),
            lanes: &r.lanes,
            progress: r.progress,
            s: r.s,
            on_route: r.on_route,
            note,
            turn: self.next_turn.clone(),
            next_dist: self.next_dist,
            street: self.street_here.as_deref(),
            limit,
            speed_avg: self.speed_avg,
            jam_cost: self.jam_cost,
            places: &self.stop_pos,
            pins: self
                .pins
                .list
                .iter()
                .enumerate()
                .map(|(k, p)| crate::companion::NavPin {
                    at: p.at,
                    via: match self.pins.role(k) {
                        crate::nav_pins::Role::Via(n) => n,
                        crate::nav_pins::Role::Destination => 0,
                    },
                    name: p.name.clone(),
                    dist: self.pins.dist.get(k).copied().flatten(),
                })
                .collect(),
            diversion: self.pins.diversion() && self.pins.steers(),
            arrived: self.pins.arrived.is_some(),
            pin_note: self.pins.note.map(|(n, _)| (n.key(), match n {
                crate::nav_pins::Note::Reached(v) => v,
                _ => 0,
            })),
        }
    }

    /// The other vehicles within `reach` metres of `bus` as the maps draw them (none with the
    /// `nav_ai` setting off): place, heading, what it is (0 a car, 1 a trolleybus, 2 a bus,
    /// 3 a tram) and the line it shows.
    pub(crate) fn companion_traffic(&self, traffic: Option<&Traffic>, bus: DVec3, reach: f64) -> Vec<(DVec3, f64, u8, Option<String>)> {
        let Some(t) = traffic.filter(|_| self.show_ai) else { return Vec::new() };
        t.cars
            .iter()
            .filter(|c| !c.gone && (c.vehicle.position - bus).truncate().length() <= reach)
            .map(|c| {
                let (color, line) = traffic_kind(c);
                let kind = if color == TROLLEY { 1 } else if color == BUS { 2 } else if color == TRAM { 3 } else { 0 };
                (c.vehicle.position, c.vehicle.heading, kind, line)
            })
            .collect()
    }

    /// The roads of a map's network as the city map draws them: carriageways with their
    /// widths, the main roads marked (the phone's map draws the same roads).
    pub(crate) fn companion_roads(net: &Network) -> Vec<MapRoad> {
        road_geometry(net)
    }
}
