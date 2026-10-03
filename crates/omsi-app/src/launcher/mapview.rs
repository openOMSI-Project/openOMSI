//! The Drive page's map: the chosen map as the game's own city map has it - every spline's
//! and every object's `[path]`, linked to its neighbours, grouped into carriageways with
//! their real widths, where its entry points stand and where its objects are - and the road
//! pieces the chosen trip drives over it.
//!
//! It reads the map the way the navigator does (`scene::navigation_map_of`, then
//! `navigator::city_roads`), so what the launcher shows cannot drift from what the game
//! draws on its own map. What it leaves out is what choosing a duty does not need: no
//! meshes, no textures, no traffic, no timetable beyond the chosen trip. That read is the
//! one thing here that costs anything (the same 0.1 - 1.5 s the game's own map costs) and it
//! runs once per map selection, off the interface thread, with the page saying so while it
//! does.
//!
//! It is not a picture but a map: dragged to move, the wheel to zoom where the cursor is, an
//! entry point clicked to choose where the bus is put down. The chosen line's route runs over
//! the roads in red and its stops stand on it, their names and times written beside them by
//! the interface (`Launcher::map_labels`). The lists next to it do what a map cannot say, and
//! they stay.
//!
//! The lines are built once per zoom band - a point every fraction of a pixel is all the eye
//! gets (`tolerance`) - so moving the map is a matrix and not a rebuild.

use glam::{DVec2, DVec3, Mat4, Vec2, Vec3};
use hashbrown::HashMap;
use omsi_render::Renderer;
use omsi_sim::traffic::Network;
use omsi_ui::{Color, Draw, Gpu, Layer, Painter, Rect, Vertex};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Instant;

// The map is drawn with the game's own city map palette and order (see `navigator`): every
// road's dark casing first, then every road's surface over every casing, then the chosen
// line's route in the map's own red - so the two maps are the same picture.
use crate::navigator::{ROAD, ROAD_CASING, ROAD_MAIN, ROUTE};
const STOP: Color = Color::rgba(240, 240, 240, 1.0);
/// Entry points wear the launcher's own amber; the one under the mouse a grey ring, the
/// chosen one a white (a click on the map takes the place of a name in a list of seventy).
const ENTRY: Color = Color::rgba(232, 160, 48, 1.0);
const ENTRY_HOVER: Color = Color::rgba(160, 160, 160, 1.0);
const ENTRY_HERE: Color = Color::rgba(255, 255, 255, 1.0);
/// A road is at least this wide on the screen when the map is far out, its own metres when
/// it is near (the toolkit takes both: `Painter::ribbon`); the route, the dots and the rings
/// have no metres of their own. The first two match the game's city map.
const CASING_PX: f32 = 2.4;
const ROAD_PX: f32 = 1.4;
const ROUTE_PX: f32 = 5.0;
const STOP_PX: f32 = 2.8;
const ENTRY_PX: f32 = 3.4;
/// The map's own background: the dark the window is cleared to, so the rail, the panels and
/// the map meet without a seam.
const BACKDROP: wgpu::Color = wgpu::Color { r: 0.0056, g: 0.0056, b: 0.0056, a: 1.0 };
/// The closest the map goes in, in metres a pixel.
const MPP_MIN: f64 = 0.15;
/// A press that travelled less than this many pixels is a click on what stands under it.
const CLICK_SLOP: f32 = 5.0;
/// How long the wheel has to be still before the plan is built again for the new zoom (see
/// `plan_is_stale`): a zoom that keeps moving, a pinch or a rolled wheel, would otherwise ask
/// for the whole map to be thinned again on every notch.
const ZOOM_SETTLE: std::time::Duration = std::time::Duration::from_millis(140);

/// What the map should show. Changing `map` reads the tiles again; changing `trip` only walks
/// the timetable; changing `entry` only moves the ring.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Look {
    /// The map as the launcher names it (`maps/<name>/global.cfg`), and where that is.
    pub map: String,
    pub global: PathBuf,
    /// The date the duty starts on (the chrono folders change the map and the timetable).
    pub date: String,
    /// The trip whose route is drawn (empty: the map alone).
    pub trip: String,
    /// The entry point the player picked, -1 = automatic (nothing is ringed).
    pub entry: i32,
}

/// One frame of the mouse over the map. `blocked` is what a panel or a field over it takes.
#[derive(Clone, Copy, Default)]
pub struct Pointer {
    pub at: Vec2,
    pub pressed: bool,
    pub released: bool,
    pub down: bool,
    pub wheel: f32,
    pub blocked: bool,
}

/// An entry point as the map draws it: where its marker stands, its name, and its own place
/// in `global.cfg`'s list. That place - not its place among the drawn markers - is what the
/// choice and the game count (`--entry` is the position in that list), so an entry point
/// whose position cannot be resolved leaves a gap that does not shift every marker after it
/// onto the wrong spawn.
struct Entry {
    /// Where the marker stands, in world metres.
    at: DVec2,
    /// Its name, as `global.cfg` writes it.
    name: String,
    /// Its position in `global.entry_points`.
    index: usize,
}

/// A map as the map needs it: the roads the game's own city map draws, the lanes they were
/// grouped from (the chosen trip's route runs on those), where its entry points stand and
/// where its objects do - all in world metres.
#[derive(Default)]
struct Roads {
    /// Every carriageway the map has, as the game groups them: points in world metres.
    roads: Vec<crate::navigator::MapRoad>,
    /// The linked network behind them (a trip's road pieces are lanes of it).
    net: Arc<Network>,
    /// (tile x, tile y, spline id, path) → the lane the timetable names.
    lanes: HashMap<(i32, i32, i64, u16), usize>,
    /// (tile x, tile y, spline id) → every lane of that spline (for a piece whose exact path
    /// is not there any more).
    splines: HashMap<(i32, i32, i64), Vec<usize>>,
    /// The world point the drawn lines are measured from: an f32 cannot hold a map's own
    /// millions of metres to the centimetre, and roads are drawn to the centimetre.
    origin: DVec2,
    /// The world rectangle the roads cover (the 1 - 99 % box: see `read_map`).
    lo: DVec2,
    hi: DVec2,
    /// The entry points that could be placed, in `global.cfg`'s order - each carrying its own
    /// place in that list (see `Entry`).
    entries: Vec<Entry>,
    /// Every placed object's world place: the trip's stops are found here.
    objects: HashMap<i64, DVec2>,
}

/// What a worker read.
struct Reply {
    look: Look,
    roads: Option<Arc<Roads>>,
    trip: omsi_launcher_lib::TripPath,
    /// What went wrong, when nothing could be read at all.
    error: Option<String>,
}

/// The plan (buffer 0): the roads and the route as one vertex list, and what it was built
/// for - another map, another trip, or another zoom band (whose own thinning it is).
struct Plan {
    key: Key,
    verts: Vec<Vertex>,
    count: u32,
    uploaded: bool,
}

#[derive(Clone, PartialEq)]
struct Key {
    global: PathBuf,
    trip: String,
    /// The simplification the plan was built with, as the bit pattern of its metres.
    tolerance: u32,
}

/// The map: what it shows, where it is looking, and the texture it is drawn into.
pub struct MapView {
    want: Option<Look>,
    shown: Option<Look>,
    roads: Option<Arc<Roads>>,
    trip: omsi_launcher_lib::TripPath,
    /// Where the trip calls: (its place on the map, the stop's own number in the timetable).
    stops: Vec<(DVec2, usize)>,
    /// The world rectangle the trip's route covers (the map's own when no trip is chosen).
    route_box: Option<(DVec2, DVec2)>,
    error: Option<String>,
    loading: Option<Receiver<Reply>>,

    /// The world point the middle of the window looks at, and what one pixel of it holds.
    center: DVec2,
    mpp: f64,
    /// When the wheel last moved the map: while it keeps moving, the plan is not built again
    /// for the new zoom (see `plan_is_stale`).
    wheeled: Option<Instant>,
    /// The plan must be fitted into the window again (another map, another trip, a window
    /// that changed while nothing was moved by hand).
    fit: bool,
    /// The player moved the map: a window that changes afterwards only shifts the middle.
    manual: bool,
    /// The mouse: how far a press has come, and whether the map has it.
    travelled: f32,
    panning: bool,
    last: Option<Vec2>,
    /// The drawn marker under the mouse, and the one a click took this frame - both a number
    /// among the drawn markers; `take_clicked` turns the second into the choice's own number.
    hover: Option<usize>,
    clicked: Option<usize>,

    /// The plan and the markers (buffers 0 and 1); the markers are built every frame, the
    /// plan only when its key changes.
    plan: Option<Plan>,
    gpu: Option<Gpu>,
    target: Option<(wgpu::Texture, wgpu::TextureView, u32, u32)>,
    /// Bumped whenever `target` was made anew (the interface binds it again).
    pub generation: u64,

    /// The rect the picture covers and the window it should be framed in (interface pixels),
    /// with the scale they were given at.
    rect: Rect,
    window: Rect,
    scale: f32,
}

impl MapView {
    pub fn new() -> MapView {
        MapView {
            want: None,
            shown: None,
            roads: None,
            trip: Default::default(),
            stops: Vec::new(),
            route_box: None,
            error: None,
            loading: None,
            center: DVec2::ZERO,
            mpp: 4.0,
            wheeled: None,
            fit: true,
            manual: false,
            travelled: 0.0,
            panning: false,
            last: None,
            hover: None,
            clicked: None,
            plan: None,
            gpu: None,
            target: None,
            generation: 0,
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            window: Rect::new(0.0, 0.0, 1.0, 1.0),
            scale: 1.0,
        }
    }

    /// What the map should show. Called every frame with the current choice; only a change
    /// starts any work.
    pub fn want(&mut self, look: Look) {
        // (before the map list has arrived: nothing to read yet)
        if look.map.trim().is_empty() {
            return;
        }
        if self.want.as_ref() != Some(&look) {
            self.want = Some(look);
        }
    }

    /// What to say while there is no map yet.
    pub fn status(&self) -> &'static str {
        if self.loading.is_some() || (self.want.is_some() && self.shown.is_none()) {
            "Reading the map…"
        } else if self.error.is_some() {
            "The map cannot be read"
        } else if self.roads.as_deref().map(|r| r.roads.is_empty()).unwrap_or(false) {
            "This map has no roads"
        } else {
            ""
        }
    }

    /// A worker is reading (the page shows its little spinner).
    pub fn busy(&self) -> bool {
        self.loading.is_some()
    }

    /// The counts a legend says: (roads, the trip's stops, the map's entry points).
    pub fn counts(&self) -> Option<(usize, usize, usize)> {
        let r = self.roads.as_deref()?;
        Some((r.roads.len(), self.stops.len(), r.entries.len()))
    }

    /// Its size in physical pixels.
    pub fn pixels(&self) -> (u32, u32) {
        let (w, h) = self.size();
        (w as u32, h as u32)
    }

    /// Where the trip calls, as the map knows it: (place, the stop's number in the
    /// timetable's own list of it).
    pub fn stops_placed(&self) -> &[(DVec2, usize)] {
        &self.stops
    }

    /// The chosen entry point, as the choice has it (-1: automatic).
    fn chosen(&self) -> i32 {
        self.shown.as_ref().map(|s| s.entry).unwrap_or(-1)
    }

    /// The drawn entry point under the mouse (a marker's own number among the drawn ones,
    /// which `entry_name` and `entry_at` take - not the choice's number).
    pub fn hovered(&self) -> Option<usize> {
        self.hover
    }

    /// The name of a drawn entry point, as `global.cfg` writes it.
    pub fn entry_name(&self, i: usize) -> Option<&str> {
        self.roads.as_deref()?.entries.get(i).map(|e| e.name.as_str())
    }

    /// Where a drawn entry point stands, in the picture's own pixels.
    pub fn entry_at(&self, i: usize) -> Option<Vec2> {
        Some(self.project(self.roads.as_deref()?.entries.get(i)?.at))
    }

    /// Which drawn entry point the choice means (`-1`: automatic, or one that could not be
    /// placed - there is no marker to ring).
    pub fn shown_of(&self, choice: i32) -> Option<usize> {
        let choice = usize::try_from(choice).ok()?;
        self.roads.as_deref()?.entries.iter().position(|e| e.index == choice)
    }

    /// The entry point a click took this frame, as its own place in `global.cfg`'s list (the
    /// page applies it to the choice, and the game counts that list, not the drawn markers).
    pub fn take_clicked(&mut self) -> Option<usize> {
        let drawn = self.clicked.take()?;
        Some(self.roads.as_deref()?.entries.get(drawn)?.index)
    }

    /// Where a world point lies in the picture (interface pixels, `rect`'s own coordinates).
    pub fn project(&self, p: DVec2) -> Vec2 {
        let a = self.anchor();
        let q = (p - self.center) / self.mpp;
        let at = Vec2::new(self.rect.x, self.rect.y);
        at + (a + Vec2::new(q.x as f32, -q.y as f32)) / self.scale
    }

    /// The world point under a point of the picture.
    fn world_at(&self, at: Vec2) -> DVec2 {
        let a = self.anchor();
        let q = (at - Vec2::new(self.rect.x, self.rect.y)) * self.scale - a;
        self.center + DVec2::new(q.x as f64, -q.y as f64) * self.mpp
    }

    /// Where the middle of the visible window is, in the target's own pixels.
    fn anchor(&self) -> Vec2 {
        (self.window.center() - Vec2::new(self.rect.x, self.rect.y)) * self.scale
    }

    /// The picture's size in physical pixels.
    fn size(&self) -> (f32, f32) {
        ((self.rect.w * self.scale).max(1.0), (self.rect.h * self.scale).max(1.0))
    }

    /// The part of the picture no panel lies over, in the target's own pixels.
    fn visible(&self) -> Rect {
        let (w, h) = self.size();
        let x0 = ((self.window.x - self.rect.x) * self.scale).clamp(0.0, w);
        let y0 = ((self.window.y - self.rect.y) * self.scale).clamp(0.0, h);
        let x1 = ((self.window.right() - self.rect.x) * self.scale).clamp(x0, w);
        let y1 = ((self.window.bottom() - self.rect.y) * self.scale).clamp(y0, h);
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// One frame of the mouse: what it hovers, what it drags, and where the map is looking.
    /// Called before the picture is drawn, so the labels the page writes over the map stand
    /// where the roads are this frame.
    pub fn think(&mut self, rect: Rect, window: Rect, scale: f32, p: Pointer) {
        let moved_window = (self.window.w - window.w).abs() > 8.0 || (self.window.h - window.h).abs() > 8.0;
        self.rect = rect;
        self.window = window;
        self.scale = scale;
        self.pump();
        if self.roads.is_some() && (self.fit || (moved_window && !self.manual)) {
            self.fit_to();
        }
        let over = rect.contains(p.at) && !p.blocked;
        self.hover = if over { self.hit(p.at) } else { None };
        // the wheel zooms where the cursor is: the world under it stays under it
        if over && p.wheel.abs() > 0.0 {
            let before = self.world_at(p.at);
            self.mpp = (self.mpp * (1.0 - p.wheel * 0.12) as f64).clamp(MPP_MIN, self.max_mpp());
            self.center += before - self.world_at(p.at);
            self.manual = true;
            self.wheeled = Some(Instant::now());
        }
        if p.pressed {
            self.travelled = 0.0;
            self.panning = over;
            // (the mouse may have arrived here in one jump - the first click after the
            // cursor comes back to the window: that is not a drag)
            self.last = Some(p.at);
        } else if p.down && self.panning {
            if let Some(last) = self.last {
                let d = p.at - last;
                self.travelled += d.length();
                self.center -= DVec2::new(d.x as f64, -d.y as f64) * self.mpp / self.scale as f64;
                self.manual = true;
            }
        }
        self.last = Some(p.at);
        if p.released {
            // a press that stayed where it was is a click on the entry point under it
            if self.panning && self.travelled < CLICK_SLOP {
                self.clicked = self.hit(p.at);
            }
            self.panning = false;
        }
    }

    /// The entry point under a point of the picture, if one is within reach.
    fn hit(&self, at: Vec2) -> Option<usize> {
        let roads = self.roads.as_deref()?;
        let mut best: Option<(f32, usize)> = None;
        for (i, e) in roads.entries.iter().enumerate() {
            let d = (self.project(e.at) - at).length();
            if d <= 14.0 && best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, i));
            }
        }
        best.map(|(_, i)| i)
    }

    /// Start whatever the current choice still needs, and take what a worker finished.
    fn pump(&mut self) {
        if let Some(rx) = self.loading.as_ref() {
            match rx.try_recv() {
                Ok(r) => {
                    self.loading = None;
                    // what was shown before, to see whether this is a whole new map (or a
                    // whole new trip): the map opens on the trip's own route, and the
                    // timetable arrives a moment after the page does, so the first read of a
                    // map often has no trip at all
                    let before = self.shown.as_ref().map(|s| (s.global.clone(), s.date.clone(), s.trip.clone()));
                    let another = before.as_ref().map(|(g, d, _)| g != &r.look.global || d != &r.look.date).unwrap_or(true);
                    let another_trip = before.as_ref().map(|(_, _, t)| t != &r.look.trip).unwrap_or(true);
                    if let Some(roads) = r.roads {
                        self.roads = Some(roads);
                    }
                    self.trip = r.trip;
                    self.error = r.error;
                    self.shown = Some(r.look);
                    self.plan = None;
                    self.place();
                    if another || another_trip {
                        self.fit = true;
                        self.manual = false;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.loading = None;
                    self.error = Some("the worker went".into());
                }
            }
        }
        let Some(want) = self.want.clone() else { return };
        if self.shown.as_ref() == Some(&want) {
            return;
        }
        // only the ring moved: nothing has to be read again
        if self.shown.as_ref().map(|s| s.map == want.map && s.date == want.date && s.trip == want.trip).unwrap_or(false) {
            self.shown = Some(want);
            return;
        }
        // the tiles of the map are read once; another trip on the same map only walks the
        // timetable again
        let have = self.roads.clone().filter(|_| self.shown.as_ref().map(|s| s.global == want.global && s.date == want.date).unwrap_or(false));
        let (tx, rx) = channel();
        let w = want.clone();
        let spawned = std::thread::Builder::new().name("launcher map".into()).spawn(move || {
            let _ = tx.send(read(w, have));
        });
        match spawned {
            Ok(_) => self.loading = Some(rx),
            Err(e) => {
                self.error = Some(e.to_string());
                self.shown = Some(want);
            }
        }
    }

    /// Where the trip's stops stand, and how much of the map its route covers - once per
    /// read, not once per frame.
    fn place(&mut self) {
        self.stops.clear();
        self.route_box = None;
        let Some(roads) = self.roads.clone() else { return };
        for (i, id) in self.trip.stops.iter().enumerate() {
            if let Some(q) = roads.objects.get(id) {
                self.stops.push((*q, i));
            }
        }
        let (mut lo, mut hi) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
        let mut found = 0usize;
        for piece in &self.trip.route {
            let lanes = roads.routes_of(piece);
            found += usize::from(!lanes.is_empty());
            for i in lanes {
                let Some(l) = roads.net.lanes.get(i) else { continue };
                for q in &l.points {
                    lo = lo.min(q.truncate());
                    hi = hi.max(q.truncate());
                }
            }
        }
        if found < self.trip.route.len() {
            // (a map whose timetable still names splines it no longer has: the game's own
            // routing skips the same pieces, so the picture is right even where it is short)
            log::info!("launcher map: the route - {found} of {} pieces are on the map", self.trip.route.len());
        }
        if lo.x != f64::MAX {
            self.route_box = Some((lo, hi));
        }
    }

    /// What the map is looking at: the chosen trip's own route when there is one (that is
    /// what the player is choosing), else the whole map.
    fn content(&self) -> Option<(DVec2, DVec2)> {
        if let Some((lo, hi)) = self.route_box {
            return Some((lo, hi));
        }
        let roads = self.roads.as_deref()?;
        Some((roads.lo, roads.hi))
    }

    /// The closest zoom that still holds the whole map in the window.
    fn max_mpp(&self) -> f64 {
        let Some((lo, hi)) = self.content() else { return 64.0 };
        let vis = self.visible();
        let span = (hi - lo).max(DVec2::splat(1.0));
        let fit = (span.x / vis.w.max(1.0) as f64).max(span.y / vis.h.max(1.0) as f64);
        (fit * 2.0).max(MPP_MIN * 4.0)
    }

    /// Fit what the map shows into the part of the window no panel covers.
    fn fit_to(&mut self) {
        let Some((lo, hi)) = self.content() else { return };
        let vis = self.visible();
        let span = (hi - lo).max(DVec2::splat(1.0));
        // (a little air around it: a route that touches the frame reads as if it runs on)
        let mpp = (span.x / vis.w.max(1.0) as f64).max(span.y / vis.h.max(1.0) as f64) * 1.06;
        self.mpp = mpp.max(MPP_MIN);
        self.center = (lo + hi) * 0.5;
        self.fit = false;
        log::info!(
            "map: looking at {} ({:.0}, {:.0}) - ({:.0}, {:.0}) through a {} x {} window, {:.1} m per pixel",
            if self.route_box.is_some() { "the trip's route" } else { "the whole map" },
            lo.x, lo.y, hi.x, hi.y, vis.w.round(), vis.h.round(), self.mpp
        );
    }

    /// How far the drawn lines may leave the map's own shape: half a pixel is all the eye
    /// gets, and it is rounded to a power of two because the plan is built again whenever this
    /// changes. A wheel notch moves the metres a pixel by 12 %, so unrounded there was a plan
    /// built per notch - and the plan is the whole map in one vertex list, thirty thousand
    /// roads' worth of it, over a hundred megabytes through the queue each time. Rounded, only
    /// a crossing between bands builds one, and a half pixel stands for at most 0.71 of one.
    fn tolerance(&self) -> f32 {
        let half_a_pixel = (self.mpp * 0.5) as f32;
        (2f32.powf(half_a_pixel.max(f32::MIN_POSITIVE).log2().round())).clamp(0.05, 24.0)
    }

    /// The device it was drawn on is gone: the picture, the plan's buffers and the drawing
    /// pipeline are made again on the next one (the interface binds the new picture anew).
    pub fn drop_gpu(&mut self) {
        self.gpu = None;
        self.target = None;
        self.plan = None;
        self.generation += 1;
    }

    /// The picture, drawn again when what it shows or how coarse it is changed.
    pub fn picture(&mut self, renderer: &Renderer) -> Option<wgpu::TextureView> {
        let (w, h) = self.size();
        let (w, h) = (w as u32, h as u32);
        if self.target.as_ref().map(|t| (t.2, t.3) != (w, h)).unwrap_or(true) {
            let tex = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("launcher map"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: renderer.format(),
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            self.target = Some((tex, view, w, h));
            self.generation += 1;
            self.plan = None;
        }
        let roads = self.roads.clone()?;
        let tolerance = self.tolerance();
        let key = Key { global: self.shown.as_ref().map(|s| s.global.clone()).unwrap_or_default(), trip: self.trip_name(), tolerance: tolerance.to_bits() };
        if self.plan_is_stale(&key) {
            let (verts, points) = self.build(&roads, tolerance);
            let count = verts.len() as u32;
            log::info!("map: the plan rebuilt - {points} points of {} roads in its {count} vertices, {:.1} m per pixel", roads.roads.len(), self.mpp);
            self.plan = Some(Plan { key, verts, count, uploaded: false });
        }
        // everything the drawing needs is read before the GPU is borrowed (the markers are
        // built every frame: the mouse moves what they look like)
        let marks = self.mark_vertices(&roads);
        let (count, marks_len) = (self.plan.as_ref().map(|p| p.count).unwrap_or(0), marks.len() as u32);
        let (proj, px_scale) = (self.projection(), self.mpp as f32);
        let target = self.target.as_ref()?.1.clone();
        let device = &renderer.device;
        let queue = &renderer.queue;
        let gpu = self.gpu.get_or_insert_with(|| Gpu::new(device, renderer.format(), 1, 64));
        if let Some(p) = self.plan.as_mut() {
            if !p.uploaded {
                gpu.upload(device, queue, 0, &p.verts);
                p.uploaded = true;
            }
        }
        gpu.upload(device, queue, 1, &marks);
        let layer = Layer { view_proj: proj, viewport: [0.0, 0.0, w as f32, h as f32], clip: [0.0, 0.0, w as f32, h as f32], radius: 0.0, opacity: 1.0, px_scale };
        let draws = [
            Draw { buffer: 0, range: 0..count, layer: 0, texture: 0 },
            Draw { buffer: 1, range: 0..marks_len, layer: 0, texture: 0 },
        ];
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("launcher map") });
        gpu.render(device, queue, &mut enc, &target, (w, h), Some(BACKDROP), &[layer], &draws);
        queue.submit([enc.finish()]);
        Some(target)
    }

    /// The name of the trip whose route is drawn (the plan is built again when it changes).
    fn trip_name(&self) -> String {
        self.shown.as_ref().map(|s| s.trip.clone()).unwrap_or_default()
    }

    /// Whether the plan has to be built again: another map, another trip, or another band of
    /// thinning than the one it was built for - and the wheel has stopped.
    ///
    /// A zoom in progress is not a reason to build one: the plan holds the whole map and the
    /// camera only moves a matrix over it, so what is on the screen is the same lines, thinned
    /// for the zoom the wheel was last at. Building waits for the wheel to stop (`ZOOM_SETTLE`),
    /// which is what makes a big map's zoom smooth - a plan of thirty thousand roads is over a
    /// hundred megabytes to build and to send, and a wheel that keeps moving would ask for one
    /// per notch.
    fn plan_is_stale(&self, key: &Key) -> bool {
        match self.plan.as_ref() {
            // nothing at all to draw: a map being read shows nothing until one is built
            None => true,
            Some(p) if &p.key == key => false,
            Some(_) => self.wheel_settled(),
        }
    }

    /// The wheel has been still for `ZOOM_SETTLE`: the plan may be built again.
    fn wheel_settled(&self) -> bool {
        self.wheeled.map(|t| t.elapsed() >= ZOOM_SETTLE).unwrap_or(true)
    }

    /// The camera as a matrix: `center` lands at the middle of the visible window. `roads`'
    /// origin is taken off both - the vertices are drawn relative to it.
    fn projection(&self) -> Mat4 {
        let (w, h) = self.size();
        let a = self.anchor();
        let o = self.roads.as_deref().map(|r| r.origin).unwrap_or_default();
        let c = self.center - o;
        let left = (c.x - a.x as f64 * self.mpp) as f32;
        let right = (c.x + (w as f64 - a.x as f64) * self.mpp) as f32;
        let bottom = (c.y - (h as f64 - a.y as f64) * self.mpp) as f32;
        let top = (c.y + a.y as f64 * self.mpp) as f32;
        Mat4::orthographic_rh(left, right, bottom, top, -100.0, 100.0)
    }

    /// The roads of the whole map and the chosen trip's route over them, as one vertex list
    /// (the camera only moves a matrix over it).
    fn build(&self, roads: &Roads, tolerance: f32) -> (Vec<Vertex>, usize) {
        let k = self.scale;
        let rel = |q: DVec3| Vec3::new((q.x - roads.origin.x) as f32, (q.y - roads.origin.y) as f32, 0.0);
        let mut p = Painter::new();
        let mut points = 0usize;
        // what a road is drawn from: its own metres, at least a hair of a pixel
        let mut band = |p: &mut Painter, pts: &[DVec3], w_m: f32, w_px: f32, c: Color| {
            let v: Vec<Vec3> = pts.iter().map(|q| rel(*q)).collect();
            let v = simplify(&v, tolerance);
            points += v.len();
            if v.len() >= 2 {
                p.ribbon(&v, w_m, w_px, c, true);
            }
        };
        // the game's own city map draws every road twice: a dark casing, then the surface of
        // every road over every casing (a crossing's surface is not cut by a neighbour's
        // casing)
        for pass in 0..2 {
            for r in &roads.roads {
                if pass == 0 {
                    band(&mut p, &r.points, r.width + 2.0, CASING_PX * k, ROAD_CASING);
                } else {
                    band(&mut p, &r.points, r.width, ROAD_PX * k, if r.main { ROAD_MAIN } else { ROAD });
                }
            }
        }
        // the route: the very lanes the game would drive, so it runs on its own side of the
        // road and not down the middle of it
        for piece in &self.trip.route {
            for i in roads.routes_of(piece) {
                let Some(l) = roads.net.lanes.get(i) else { continue };
                let v: Vec<Vec3> = l.points.iter().map(|q| rel(*q)).collect();
                let v = simplify(&v, tolerance);
                points += v.len();
                if v.len() >= 2 {
                    p.ribbon(&v, 0.0, ROUTE_PX * k, ROUTE, false);
                }
            }
        }
        (p.verts, points)
    }

    /// The markers (buffer 1, built every frame: the mouse moves what they look like).
    fn mark_vertices(&self, roads: &Roads) -> Vec<Vertex> {
        let k = self.scale;
        let mpp = self.mpp as f32;
        let at = |q: DVec2| Vec3::new((q.x - roads.origin.x) as f32, (q.y - roads.origin.y) as f32, 0.0);
        let mut p = Painter::new();
        for (q, _) in &self.stops {
            p.world_disc(at(*q), 0.0, STOP_PX * k, STOP);
        }
        let chosen = self.chosen();
        for (i, e) in roads.entries.iter().enumerate() {
            p.world_disc(at(e.at), 0.0, ENTRY_PX * k, ENTRY);
            if self.hover == Some(i) {
                ring(&mut p, at(e.at), (ENTRY_PX + 3.0) * k, 2.0 * k, ENTRY_HOVER, mpp);
            }
            // (the choice counts `global.cfg`'s list, which the marker's own place names)
            if chosen == e.index as i32 {
                ring(&mut p, at(e.at), (ENTRY_PX + 6.5) * k, 2.0 * k, ENTRY_HERE, mpp);
            }
        }
        p.verts
    }
}

impl Roads {
    /// The lanes a trip's road piece runs on: the very `[path]` the timetable names, else -
    /// a map whose timetable still names a spline it no longer has - every lane of that
    /// spline, so a hole in the map's own data is not a hole in the picture.
    fn routes_of(&self, piece: &omsi_launcher_lib::RoadPiece) -> Vec<usize> {
        if let Some(&i) = self.lanes.get(&(piece.tile_x, piece.tile_y, piece.spline, piece.path)) {
            return vec![i];
        }
        self.splines.get(&(piece.tile_x, piece.tile_y, piece.spline)).cloned().unwrap_or_default()
    }
}

/// A ring `r_px` out and `w_px` thick around a world point, as the camera's own metres drawn
/// as a band (the toolkit's widths are measured on the screen, its circles are not).
fn ring(p: &mut Painter, at: Vec3, r_px: f32, w_px: f32, c: Color, mpp: f32) {
    let r = (r_px * mpp) as f64;
    let n = 32usize;
    let v: Vec<Vec3> = (0..=n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            Vec3::new(at.x + (r * a.cos()) as f32, at.y + (r * a.sin()) as f32, 0.0)
        })
        .collect();
    p.ribbon(&v, 0.0, w_px, c, false);
}

/// A polyline with the points the eye cannot tell apart taken out (the game's own map
/// simplification, in metres: 12 cm there, half a pixel here).
fn simplify(pts: &[Vec3], tol: f32) -> Vec<Vec3> {
    crate::navigator::simplify(pts, tol)
}

/// Read a map the way the map needs it: the roads the game's city map draws, the entry
/// points, the objects - and the chosen trip's route with them.
fn read(look: Look, have: Option<Arc<Roads>>) -> Reply {
    let mut reply = Reply { look, roads: None, trip: Default::default(), error: None };
    if let Some(r) = have {
        reply.trip = trip_of(&reply.look);
        reply.roads = Some(r);
        return reply;
    }
    let roads = read_map(&reply.look);
    if roads.roads.is_empty() && roads.objects.is_empty() {
        reply.error = Some(format!("{}", reply.look.global.display()));
    }
    reply.trip = trip_of(&reply.look);
    reply.roads = Some(Arc::new(roads));
    reply
}

/// The route of the chosen trip (nothing when no trip is chosen or the map has a timetable
/// that does not know it).
fn trip_of(look: &Look) -> omsi_launcher_lib::TripPath {
    if look.trip.trim().is_empty() {
        return Default::default();
    }
    match omsi_launcher_lib::trip_path(&look.map, &look.date, &look.trip) {
        Ok(t) => t,
        Err(e) => {
            log::debug!("trip path for {}: {e}", look.trip);
            Default::default()
        }
    }
}

/// The map's own data, read the way the game reads it for its city map: every tile's paths
/// (splines and the crossings' objects), linked, grouped into carriageways with their real
/// widths. `look.global` is the map's `global.cfg`; the tiles stand beside it and the assets
/// the map names under the installation it belongs to.
fn read_map(look: &Look) -> Roads {
    let Ok(global) = omsi_map::GlobalCfg::load(&look.global) else { return Roads::default() };
    // everything that turns a tile index and a place in the tile into world metres needs
    // this first (`[worldcoordinates]` maps use another grid)
    omsi_map::configure_grid(&global);
    let Some(map_dir) = look.global.parent().map(Path::to_path_buf) else { return Roads::default() };
    let chrono = omsi_map::date_code(&look.date).map(|c| omsi_map::active_chrono_dirs(&map_dir, c)).unwrap_or_default();
    let size = omsi_map::tile_size();
    // the installation the map belongs to: what its `.sli` and `.sco` files are named from.
    // A map is a folder under a content root's `maps` (that is where the launcher's list came
    // from), so the root that holds it is the one its assets are named from; a map outside
    // every root falls back to the folder two steps up, as `<root>/maps/<name>`.
    let root = omsi_cfg::content_roots()
        .into_iter()
        .find(|r| map_dir.starts_with(r))
        .or_else(|| map_dir.parent().and_then(|p| p.parent()).map(Path::to_path_buf))
        .unwrap_or_else(|| map_dir.clone());
    let tiles: Vec<(i32, i32, PathBuf)> = global
        .tiles
        .iter()
        .map(|t| (t.x, t.y, omsi_cfg::resolve_path(&map_dir, &t.file)))
        .filter(|t| omsi_cfg::vfs::is_file(&t.2))
        .collect();
    let t0 = std::time::Instant::now();
    let crate::scene::NavigationMap { lanes, road_surfaces, places, signs: _ } = crate::scene::navigation_map_of(&root, &tiles, &chrono);
    let (roads, net) = crate::navigator::city_roads(lanes, &road_surfaces);
    // the lanes the timetable can name, and the lanes of every spline behind them
    let mut by_path = HashMap::new();
    let mut by_spline: HashMap<(i32, i32, i64), Vec<usize>> = HashMap::new();
    for (i, l) in net.lanes.iter().enumerate() {
        let Some(key) = l.key else { continue };
        by_path.insert((key.tile.0, key.tile.1, key.id, key.path), i);
        by_spline.entry((key.tile.0, key.tile.1, key.id)).or_default().push(i);
    }
    // the entry points: on the tile the record names, else at their object (`World::
    // entry_point_place` reads them the same way)
    let mut entries = Vec::new();
    for (index, ep) in global.entry_points.iter().enumerate() {
        let p = usize::try_from(ep.group)
            .ok()
            .and_then(|i| global.raw_tiles.get(i))
            .map(|t| DVec2::new(t.0 as f64 * size + ep.pos[0], t.1 as f64 * size + ep.pos[1]))
            .or_else(|| places.get(&ep.object_id).map(|q| q.truncate()));
        // the marker carries where it stands in `global.cfg`'s list, so one left out here
        // (no tile, no object) does not shift the ones after it
        if let Some(p) = p {
            entries.push(Entry { at: p, name: ep.name.clone(), index });
        }
    }
    let objects: HashMap<i64, DVec2> = places.iter().map(|(k, v)| (*k, v.truncate())).collect();
    // where the roads are, and where the picture is measured from
    let (mut min, mut max) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
    for r in &roads {
        for q in &r.points {
            min = min.min(q.truncate());
            max = max.max(q.truncate());
        }
    }
    if min.x == f64::MAX {
        min = DVec2::ZERO;
        max = DVec2::ZERO;
    }
    let origin = (min + max) * 0.5;
    // Where the roads really are: one spline left in a far corner (an editor's marker, a
    // piece of the next town) would shrink everything else to a dot, so the map opens on the
    // 1 - 99 % box of what the roads cover instead of the extremes (the far ones are still
    // there to be dragged to).
    let mut xs: Vec<f32> = roads.iter().flat_map(|r| r.points.iter()).map(|q| q.x as f32).collect();
    let mut ys: Vec<f32> = roads.iter().flat_map(|r| r.points.iter()).map(|q| q.y as f32).collect();
    xs.sort_by(f32::total_cmp);
    ys.sort_by(f32::total_cmp);
    let (lo, hi) = if xs.is_empty() {
        (min, max)
    } else {
        let at = |v: &Vec<f32>, t: f64| v[((v.len() - 1) as f64 * t) as usize] as f64;
        (DVec2::new(at(&xs, 0.01), at(&ys, 0.01)), DVec2::new(at(&xs, 0.99), at(&ys, 0.99)))
    };
    log::info!(
        "map: {} roads from {} lanes in {} tiles, {} objects, {} entry points, read in {:.2} s; roads cover ({:.0}, {:.0}) - ({:.0}, {:.0}), opened at the 1 - 99 % box ({:.0}, {:.0}) - ({:.0}, {:.0})",
        roads.len(),
        net.lanes.len(),
        tiles.len(),
        objects.len(),
        entries.len(),
        t0.elapsed().as_secs_f64(),
        min.x, min.y, max.x, max.y,
        lo.x, lo.y, hi.x, hi.y
    );
    Roads { roads, net: Arc::new(net), lanes: by_path, splines: by_spline, origin, lo, hi, entries, objects }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three entry points are in the file, the second of which has no place on this map: the
    /// drawn markers are the file's first and third, and must answer with those places.
    fn map() -> MapView {
        let mut m = MapView::new();
        m.roads = Some(Arc::new(Roads {
            entries: vec![
                Entry { at: DVec2::new(0.0, 0.0), name: "Depot".into(), index: 0 },
                Entry { at: DVec2::new(100.0, 0.0), name: "Station".into(), index: 2 },
            ],
            ..Default::default()
        }));
        m
    }

    #[test]
    fn a_marker_answers_with_its_own_place_in_the_file() {
        let mut m = map();
        // the second drawn marker is the file's third entry point, not its second
        m.clicked = Some(1);
        assert_eq!(m.take_clicked(), Some(2));
        m.clicked = Some(0);
        assert_eq!(m.take_clicked(), Some(0));
        // a click is taken once
        assert_eq!(m.take_clicked(), None);
    }

    #[test]
    fn the_choice_finds_its_marker_by_the_files_own_place() {
        let m = map();
        assert_eq!(m.shown_of(0), Some(0));
        assert_eq!(m.shown_of(2), Some(1));
        // the entry point that could not be placed has no marker to ring
        assert_eq!(m.shown_of(1), None);
        // automatic, and anything out of the list
        assert_eq!(m.shown_of(-1), None);
        assert_eq!(m.shown_of(9), None);
    }

    /// The thinning a plan is built at is rounded to a power of two, so a wheel that keeps
    /// turning asks for the whole map to be built again only when it crosses into another band.
    #[test]
    fn a_zoom_builds_the_plan_again_only_across_a_band() {
        let mut m = MapView::new();
        m.mpp = 4.0;
        let band = m.tolerance();
        assert_eq!(band, 2.0, "half of four metres a pixel, which is a power of two");
        // a band is a factor of two wide: a wheel notch moves the metres a pixel by 12 %, and
        // six or seven of them, a whole gesture, stay inside the one the plan was built for
        for factor in [1.12, 1.25, 1.12 * 1.12, 0.88, 0.8, 0.88 * 0.88] {
            m.mpp = 4.0 * factor;
            assert_eq!(m.tolerance(), band, "at {factor} of four metres a pixel");
        }
        // twice as close, and twice as far out, are other bands
        m.mpp = 2.0;
        assert_eq!(m.tolerance(), 1.0);
        m.mpp = 8.0;
        assert_eq!(m.tolerance(), 4.0);
        // and the far out view is held where the clamp always held it
        m.mpp = 800.0;
        assert_eq!(m.tolerance(), 24.0);
        m.mpp = 100_000.0;
        assert_eq!(m.tolerance(), 24.0);
    }

    /// A zoom that keeps moving does not build the plan again: what stands on the screen is the
    /// same map, thinned for the zoom before it, and the camera is a matrix over it either way.
    #[test]
    fn a_zoom_in_progress_keeps_the_plan_it_has() {
        let mut m = map();
        let built = Key { global: PathBuf::from("maps/Grundorf/global.cfg"), trip: String::new(), tolerance: 2.0f32.to_bits() };
        let finer = Key { tolerance: 1.0f32.to_bits(), ..built.clone() };
        let plan = || Some(Plan { key: built.clone(), verts: Vec::new(), count: 0, uploaded: true });
        m.plan = plan();
        // (whatever the zoom now asks for, while the wheel is still turning)
        m.wheeled = Some(Instant::now());
        assert!(!m.plan_is_stale(&finer));
        assert!(!m.plan_is_stale(&built));
        // (and a map that has no plan at all is always built one: it shows nothing until then)
        m.plan = None;
        assert!(m.plan_is_stale(&finer));
        // the wheel has stopped: the band it stopped in is built
        m.plan = plan();
        m.wheeled = Some(Instant::now() - std::time::Duration::from_secs(1));
        assert!(m.plan_is_stale(&finer));
        assert!(!m.plan_is_stale(&built));
    }
}
