//! Traffic path network and AI road vehicles (original units `mc_path`, `mc_pathrule`,
//! the AI part of `mc_roadvehicle`).
//!
//! Lanes come from `[path]` entries of splines (running along the spline at a lateral
//! offset) and of scenery objects (arcs in the object's frame). Lane ends are linked by
//! proximity, which reproduces the original's spline `prev`/`next` chains and the object
//! `[splinehelper]` connectors without needing their ids.

use glam::{DVec2, DVec3};
use hashbrown::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneKind {
    Street,
    Sidewalk,
    Rail,
    /// `[path]` type 3: flight paths of AI aircraft.
    Air,
}

impl LaneKind {
    pub fn from_code(c: i32) -> LaneKind {
        match c {
            1 => LaneKind::Sidewalk,
            2 => LaneKind::Rail,
            3 => LaneKind::Air,
            _ => LaneKind::Street,
        }
    }
}

/// Identity of a lane in map terms: the tile, the spline/object id and the `[path]` index.
/// Timetable tracks reference lanes this way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LaneKey {
    pub tile: (i32, i32),
    pub id: i64,
    pub path: u16,
}

/// A sampled lane: points with headings, in world coordinates.
#[derive(Debug, Clone)]
pub struct Lane {
    pub key: Option<LaneKey>,
    /// True for the backwards lane of a two-way path.
    pub reversed: bool,
    pub kind: LaneKind,
    pub width: f32,
    /// World positions along the lane (in travel direction).
    pub points: Vec<DVec3>,
    /// Heading (degrees, clockwise from north) at each point: the tangent of the spline or
    /// arc the lane was sampled from, not the direction of the chord to the next point.
    pub headings: Vec<f32>,
    /// Signed curvature at each point (1/m, positive = turning right).
    pub curvature: Vec<f32>,
    /// Cumulative distance at each point.
    pub dist: Vec<f32>,
    pub speed_limit_kmh: f32,
    /// Lanes reachable from the end of this one.
    pub next: Vec<usize>,
    /// Traffic light controlling entry into this lane: (object instance, light index).
    pub traffic_light: Option<(usize, usize)>,
    /// Turn indicator for AI: 0 none, 1 left, 2 right.
    pub turn: i32,
    /// Source for debugging.
    pub source: u32,
    /// Lateral offset of the spline path (m, positive = right of the spline direction).
    pub offset: f32,
    /// Spline/object file the lane came from (debugging).
    pub name: String,
    /// `[rule] trafficdensity`: how much of the road traffic uses this lane (0 = none), of
    /// any random traffic group.
    pub density: f32,
    /// `[rule] trafficdensity <value> <group>` of the path, the last per group: the group
    /// is the random traffic group's place in the map's `unsched_vehgroups.txt`. A group
    /// without one takes its default there (Berlin-Spandau's GDR cars only drive where
    /// the Falkensee paths ask for them).
    pub group_density: Vec<(u16, f32)>,
    /// `[rule] no_cars`: cars keep off this lane.
    pub no_cars: bool,
    /// `[rule] bus` / `[rule] trucks` on the path: it is open to the AI vehicles of
    /// `[ai_veh_type]` 2 / 3 (see [`Lane::allows`]). The rules are switches: the value
    /// after them is not read (Grundorf's 110 `trucks` rules all say 0).
    pub rule_bus: bool,
    pub rule_trucks: bool,
    /// The spline this lane belongs to is editor-only: OMSI's invisible service roads at
    /// the edge of a map, where AI traffic drives with no road drawn under it.
    pub invisible: bool,
    /// Parallel lanes of the same road in the same direction (for lane changes).
    pub left: Option<usize>,
    pub right: Option<usize>,
    /// `[rule] priority` (`TPRIPriority`): who goes first where two paths of a junction
    /// meet. The stock maps put 192 on the straight paths of the main road and 64 on the
    /// paths coming out of a side road and leave the rest alone, so an unmarked path sits
    /// in between at `DEFAULT_PRIORITY`.
    pub priority: f32,
    /// Other `[path]`s of its object this lane blocks while taken (`[blockpath]`), beyond
    /// the places where they cross.
    pub blocks: Vec<u16>,
}

/// Priority of a path without a `[rule] priority`.
pub const DEFAULT_PRIORITY: f32 = 128.0;

/// How much of `unsched_vehgroups.txt` group `pool`'s traffic a path carries: its `[rule]
/// trafficdensity` for the group (`rules`, see `Lane::group_density`: the last rule of a
/// group counts), else the group's default there (`defaults`): 0 none, for the first group 1
/// its medium density, for any other k that of the k-th group on the same path.
pub fn pool_density(rules: &[(u16, f32)], defaults: &[i32], pool: usize) -> f32 {
    let mut u = pool;
    // (a default naming another group that names this one again would go round for ever)
    for _ in 0..=defaults.len() {
        if let Some(&(_, v)) = rules.iter().rev().find(|(k, _)| *k as usize == u) {
            return v;
        }
        match defaults.get(u).copied().unwrap_or(0) {
            d if d <= 0 => return 0.0,
            _ if u == 0 => return 1.0,
            d => u = d as usize - 1,
        }
    }
    0.0
}

impl Lane {
    /// May an AI vehicle of `[ai_veh_type]` `veh_type` drive here (Omsi.exe 0x71d714, by
    /// the path's rules): a car (0) where there is no `no_cars`, a taxi (1) also where
    /// `bus` or `trucks` opens a no_cars path, a bus (2) only where `bus` and a truck (3)
    /// only where `trucks` is set. Other values (and the timetable's buses, -1) anywhere.
    pub fn allows(&self, veh_type: i32) -> bool {
        match veh_type {
            0 => !self.no_cars,
            1 => !self.no_cars || self.rule_bus || self.rule_trucks,
            2 => self.rule_bus,
            3 => self.rule_trucks,
            _ => true,
        }
    }

    /// How much of `unsched_vehgroups.txt` group `pool`'s traffic the lane carries (see
    /// [`pool_density`]).
    pub fn pool_density(&self, defaults: &[i32], pool: usize) -> f32 {
        pool_density(&self.group_density, defaults, pool)
    }

    /// Closest point of the lane's polyline to `p`: (distance along the lane, distance to it).
    pub fn nearest_point(&self, p: DVec3) -> Option<(f32, f64)> {
        let mut best: Option<(f32, f64)> = None;
        for k in 0..self.points.len().saturating_sub(1) {
            let a = self.points[k];
            let b = self.points[k + 1];
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
            let d = (a + ab * t - p).length();
            if best.map(|b| d < b.1).unwrap_or(true) {
                best = Some((self.dist[k] + (self.dist[k + 1] - self.dist[k]) * t as f32, d));
            }
        }
        best
    }

    pub fn length(&self) -> f32 {
        *self.dist.last().unwrap_or(&0.0)
    }

    /// A random road driver's forward choice across two short, reciprocally linked
    /// paths. A return whose entry and exit have already been passed folds the planned
    /// way back on itself. This checks the driving choice, not the network topology.
    fn continues_from(&self, from: &Lane, tol: f64) -> bool {
        if self.kind != LaneKind::Street || self.length() as f64 > tol || from.length() as f64 > tol {
            return true;
        }
        let end = from.end();
        let backwards = (self.start() - end).truncate().dot(heading_dir(self.start_heading() as f64)) < 0.0
            && (self.end() - end).truncate().dot(heading_dir(self.end_heading() as f64)) <= 0.0;
        let forward_gap = (from.start() - self.end()).truncate().length_squared();
        let return_gap = (self.start() - from.end()).truncate().length_squared();
        !(backwards && forward_gap < return_gap)
    }

    /// Measure the lane again after its points were moved (an object's tilt).
    pub fn refresh(&mut self) {
        self.dist = cumulative(&self.points);
    }

    /// Segment and fraction along it at distance `s` (clamped to the lane).
    fn locate(&self, s: f32) -> (usize, f32) {
        // (a position gone NaN takes the lane's start rather than stopping the game)
        let s = if s.is_nan() { 0.0 } else { s.clamp(0.0, self.length()) };
        let i = match self.dist.binary_search_by(|d| d.total_cmp(&s)) {
            Ok(i) => i.min(self.points.len() - 2),
            Err(i) => i.saturating_sub(1).min(self.points.len() - 2),
        };
        let (d0, d1) = (self.dist[i], self.dist[i + 1]);
        (i, if d1 > d0 { (s - d0) / (d1 - d0) } else { 0.0 })
    }

    /// Position and heading at distance `s`. Between two samples the lane is a cubic
    /// Hermite curve whose tangents are the sampled headings, so a bend stays round
    /// instead of becoming a chain of chords: a car following the chords turned in small
    /// jerks at every sample and at every lane joint.
    pub fn at(&self, s: f32) -> (DVec3, f32) {
        if self.points.len() < 2 {
            return (self.points.first().copied().unwrap_or(DVec3::ZERO), self.headings.first().copied().unwrap_or(0.0));
        }
        let (i, t) = self.locate(s);
        let (h0, h1) = (self.headings[i], self.headings[i + 1]);
        (self.hermite(i, t as f64), h0 + wrap_deg(h1 - h0) * t)
    }

    fn hermite(&self, i: usize, t: f64) -> DVec3 {
        let (a, b) = (self.points[i], self.points[i + 1]);
        let lin = a.lerp(b, t);
        let chord = (b - a).truncate();
        let len = chord.length();
        if len < 1e-6 {
            return lin;
        }
        let d0 = heading_dir(self.headings[i] as f64);
        let d1 = heading_dir(self.headings[i + 1] as f64);
        // tangents far off the chord (a kink in hand-made samples) would make the curve loop
        let c = chord / len;
        if d0.dot(c) < 0.8 || d1.dot(c) < 0.8 {
            return lin;
        }
        let (t2, t3) = (t * t, t * t * t);
        let xy = a.truncate() * (2.0 * t3 - 3.0 * t2 + 1.0) + d0 * len * (t3 - 2.0 * t2 + t) + b.truncate() * (3.0 * t2 - 2.0 * t3) + d1 * len * (t3 - t2);
        DVec3::new(xy.x, xy.y, lin.z)
    }

    /// Position and heading at `s`, continued straight along the end tangents beyond either
    /// end of the lane.
    pub fn at_ext(&self, s: f32) -> (DVec3, f32) {
        if s < 0.0 {
            let h = self.start_heading();
            let d = heading_dir(h as f64) * s as f64;
            (self.start() + DVec3::new(d.x, d.y, 0.0), h)
        } else if s > self.length() {
            let h = self.end_heading();
            let d = heading_dir(h as f64) * (s - self.length()) as f64;
            (self.end() + DVec3::new(d.x, d.y, 0.0), h)
        } else {
            self.at(s)
        }
    }

    /// Curvature at `s` (1/m, positive = right).
    pub fn curvature_at(&self, s: f32) -> f32 {
        if self.points.len() < 2 || self.curvature.len() != self.points.len() {
            return 0.0;
        }
        let (i, t) = self.locate(s);
        self.curvature[i] + (self.curvature[i + 1] - self.curvature[i]) * t
    }

    pub fn start(&self) -> DVec3 {
        self.points[0]
    }
    pub fn end(&self) -> DVec3 {
        *self.points.last().unwrap()
    }
    pub fn start_heading(&self) -> f32 {
        self.headings[0]
    }
    pub fn end_heading(&self) -> f32 {
        *self.headings.last().unwrap()
    }
}

/// Speed limit of a flight path without a `[rule] speedlimit` (km/h): none, the aircraft
/// flies at its own speed.
pub const AIR_NO_LIMIT_KMH: f32 = 1000.0;

/// Builds lanes from analytic descriptions.
pub struct LaneBuilder;

impl LaneBuilder {
    /// Sample an arc/straight lane: start position, heading (deg), length, radius (0 = straight,
    /// > 0 right turn), height change over the length.
    pub fn arc(start: DVec3, heading_deg: f64, length: f64, radius: f64, dz: f64, kind: LaneKind, width: f32) -> Lane {
        Self::sample_arc(start, heading_deg, length, radius, kind, width, |s| dz * s / length.max(1e-6))
    }

    /// Object-path gradients are rise per metre, not a total height change. Integrate
    /// the gradient between the ends so the lane meets the next path on a slope.
    pub fn arc_with_gradients(start: DVec3, heading_deg: f64, length: f64, radius: f64, grad_start: f64, grad_end: f64, kind: LaneKind, width: f32) -> Lane {
        Self::sample_arc(start, heading_deg, length, radius, kind, width, |s| {
            grad_start * s + (grad_end - grad_start) * s * s / (2.0 * length.max(1e-6))
        })
    }

    fn sample_arc(start: DVec3, heading_deg: f64, length: f64, radius: f64, kind: LaneKind, width: f32, height: impl Fn(f64) -> f64) -> Lane {
        let n = ((length / 2.0).ceil() as usize).clamp(1, 400);
        let mut points = Vec::with_capacity(n + 1);
        let mut headings = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let s = length * i as f64 / n as f64;
            let (p, h) = arc_point(start, heading_deg, s, radius);
            points.push(DVec3::new(p.x, p.y, start.z + height(s)));
            headings.push(h as f32);
        }
        let k = if radius.abs() < 1e-6 { 0.0 } else { (1.0 / radius) as f32 };
        Self::curve(points, headings, vec![k; n + 1], kind, width)
    }

    /// Lane from sampled points with their tangent headings and curvatures.
    /// A flight path has no speed limit of its own unless the map gives it one with a
    /// `[rule] speedlimit`; the street default of 50 km/h had the Tegel approach flown at
    /// walking pace for an airliner.
    pub fn curve(points: Vec<DVec3>, headings: Vec<f32>, curvature: Vec<f32>, kind: LaneKind, width: f32) -> Lane {
        let dist = cumulative(&points);
        let speed_limit_kmh = if kind == LaneKind::Air { AIR_NO_LIMIT_KMH } else { 50.0 };
        Lane { key: None, reversed: false, kind, width, points, headings, curvature, dist, speed_limit_kmh, next: Vec::new(), traffic_light: None, turn: 0, source: 0, offset: 0.0, name: String::new(), invisible: false, density: 1.0, group_density: Vec::new(), no_cars: false, rule_bus: false, rule_trucks: false, left: None, right: None, priority: DEFAULT_PRIORITY, blocks: Vec::new() }
    }

    /// Lane from bare points: headings from the neighbouring points on both sides (the
    /// tangent there, not the chord ahead), curvature from how the heading changes.
    pub fn polyline(points: Vec<DVec3>, kind: LaneKind, width: f32) -> Lane {
        let n = points.len();
        let mut headings = Vec::with_capacity(n);
        for i in 0..n {
            let (a, b) = match (i.checked_sub(1), (i + 1 < n).then_some(i + 1)) {
                (Some(p), Some(q)) => (points[p], points[q]),
                (None, Some(q)) => (points[i], points[q]),
                (Some(p), None) => (points[p], points[i]),
                (None, None) => (points[i], points[i] + DVec3::Y),
            };
            let d = b - a;
            headings.push((d.x.atan2(d.y)).to_degrees() as f32);
        }
        let dist = cumulative(&points);
        let curvature = (0..n)
            .map(|i| {
                let (p, q) = (i.saturating_sub(1), (i + 1).min(n.saturating_sub(1)));
                let ds = dist.get(q).copied().unwrap_or(0.0) - dist.get(p).copied().unwrap_or(0.0);
                if ds > 1e-3 { wrap_deg(headings[q] - headings[p]).to_radians() / ds } else { 0.0 }
            })
            .collect();
        Self::curve(points, headings, curvature, kind, width)
    }
}

/// Cumulative distance along a sequence of points.
fn cumulative(points: &[DVec3]) -> Vec<f32> {
    let mut acc = 0.0f32;
    points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if i > 0 {
                acc += (*p - points[i - 1]).length() as f32;
            }
            acc
        })
        .collect()
}

/// An angle difference in degrees brought into -180..180.
pub fn wrap_deg(d: f32) -> f32 {
    (d + 180.0).rem_euclid(360.0) - 180.0
}

/// Unit vector (east, north) of a heading in degrees.
fn heading_dir(h: f64) -> DVec2 {
    let r = h.to_radians();
    DVec2::new(r.sin(), r.cos())
}

/// Point and heading after travelling `s` metres on an arc starting at `start`.
pub fn arc_point(start: DVec3, heading_deg: f64, s: f64, radius: f64) -> (DVec2, f64) {
    let h = heading_deg.to_radians();
    let d = DVec2::new(h.sin(), h.cos());
    if radius.abs() < 1e-6 {
        return (start.truncate() + d * s, heading_deg);
    }
    let r = radius.abs();
    let turn = radius.signum();
    let right = DVec2::new(d.y, -d.x);
    let centre = start.truncate() + right * r * turn;
    let ang = -turn * s / r;
    let p = start.truncate() - centre;
    let (sa, ca) = ang.sin_cos();
    (centre + DVec2::new(p.x * ca - p.y * sa, p.x * sa + p.y * ca), heading_deg + turn * (s / r).to_degrees())
}

/// The whole network.
#[derive(Default)]
pub struct Network {
    pub lanes: Vec<Lane>,
    /// Lanes by map identity (both directions of a two-way path share a key).
    pub by_key: HashMap<LaneKey, Vec<usize>>,
    /// Per lane: lanes of the same crossing object whose geometry crosses or merges into it.
    pub conflicts: Vec<Vec<usize>>,
    /// Per lane: the lanes that lead into it (the reverse of `next`), for walking a
    /// one-way path backwards.
    pub prev: Vec<Vec<usize>>,
    /// Lanes by 50 m grid cell (every cell a lane's points touch), so that a nearest-lane
    /// query looks at a handful of lanes instead of all 17 000 of Spandau.
    pub grid: HashMap<(i32, i32), Vec<usize>>,
    /// Lanes by the cell containing their start. Population only needs lane starts near
    /// a viewer; the geometry grid above can contain the same long lane in many cells.
    pub start_grid: HashMap<(i32, i32), Vec<usize>>,
    /// Per street lane: where the other street and rail lanes of its junction object cross
    /// it or run into its end (`conflicts` with the places).
    pub crossings: Vec<Vec<Crossing>>,
    /// Per street lane: the footpaths of its junction object that cross it (zebra and
    /// signalled crossings): (footpath lane, distance along the street lane, along the path).
    pub walks: Vec<Vec<(usize, f32, f32)>>,
    /// Per lane: how far a vehicle can drive from its start before the network ends (m, at
    /// most `REACH_MAX`; lanes closed to cars do not count as a way on).
    pub reach: Vec<f32>,
    /// The map drives on the left (`global.cfg` `[lht]`): priority to the left, the
    /// oncoming lane on the right, turning right across the oncoming traffic.
    pub left_hand: bool,
}

/// `Network::reach` is counted up to this far (m).
pub const REACH_MAX: f32 = 600.0;
/// A way on that ends within this distance is a dead end to a driver who has a choice (m).
pub const DEAD_END: f32 = 500.0;

/// Where two lanes of a junction meet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crossing {
    pub other: usize,
    /// Distance along this lane and along the other one to the meeting point.
    pub at: f32,
    pub other_at: f32,
    /// Both lanes end in the same place (one road runs into the other) rather than crossing.
    pub merge: bool,
    /// The meeting place: how far before and after `at` this lane's centre line stays
    /// closer than `MEET_DIST` to the other's (where two vehicles on them would touch),
    /// and the same along the other lane. Two paths that cross at a shallow angle, or two
    /// turns that bend towards each other, meet over metres, not at a point.
    pub before: f32,
    pub after: f32,
    pub other_before: f32,
    pub other_after: f32,
}

/// Two vehicles whose centre lines come closer than this (m) touch: the half widths of two
/// buses.
pub const MEET_DIST: f64 = 2.6;
/// A meeting place reaches at most this far either side of the crossing point (m).
const MEET_MAX: f32 = 14.0;

/// How far before and after distance `at` of lane `a` its centre line stays within
/// `MEET_DIST` of lane `b`'s (in half-metre steps, at least the half metre around the point).
fn meeting_extent(a: &Lane, at: f32, b: &Lane) -> (f32, f32) {
    let near = |s: f32| b.nearest_point(a.at(s).0).map(|(_, d)| d < MEET_DIST).unwrap_or(false);
    let mut before = 0.5f32;
    while before < MEET_MAX && at - before >= 0.0 && near(at - before) {
        before += 0.5;
    }
    let mut after = 0.5f32;
    while after < MEET_MAX && at + after <= a.length() && near(at + after) {
        after += 0.5;
    }
    (before, after)
}

/// Where two polylines cross: the distance along each (ends excluded).
fn polyline_crossing(a: &Lane, b: &Lane) -> Option<(f32, f32)> {
    for (i, pa) in a.points.windows(2).enumerate() {
        for (j, pb) in b.points.windows(2).enumerate() {
            let (p, r) = (pa[0].truncate(), (pa[1] - pa[0]).truncate());
            let (q, s) = (pb[0].truncate(), (pb[1] - pb[0]).truncate());
            let den = r.perp_dot(s);
            if den.abs() < 1e-9 {
                continue;
            }
            let t = (q - p).perp_dot(s) / den;
            let u = (q - p).perp_dot(r) / den;
            if !(0.0..=1.0).contains(&t) || !(0.0..=1.0).contains(&u) {
                continue;
            }
            let sa = a.dist[i] + (a.dist[i + 1] - a.dist[i]) * t as f32;
            let sb = b.dist[j] + (b.dist[j + 1] - b.dist[j]) * u as f32;
            // meeting at an end is a joint (or a fork), not a crossing
            let inner = |s: f32, l: &Lane| s > 0.3 && s < l.length() - 0.3;
            if inner(sa, a) && inner(sb, b) {
                return Some((sa, sb));
            }
        }
    }
    None
}

/// Grid cell size of `Network::grid` (m).
pub const GRID_CELL: f64 = 50.0;

impl Network {
    /// The sign of a lateral offset (positive to the right) towards the oncoming lane:
    /// -1 on the right-hand side of the road, +1 on a left-hand-traffic map.
    pub fn oncoming_sign(&self) -> f32 {
        if self.left_hand {
            1.0
        } else {
            -1.0
        }
    }

    /// Lane with the given key, preferring the direction flag.
    pub fn find(&self, key: LaneKey, reversed: Option<bool>) -> Option<usize> {
        let list = self.by_key.get(&key)?;
        match reversed {
            Some(r) => list.iter().copied().find(|&i| self.lanes[i].reversed == r).or_else(|| list.first().copied()),
            None => list.first().copied(),
        }
    }

    /// Does `b` run beside `a` from about where `a` starts - another lane of the same spline,
    /// or the other branch of a fork in the same junction - without `a` leading into it? A
    /// timetable track that lists two such paths one after the other changes lanes there
    /// (Spandau's line 92 moves from lane 5 to lane 6 of a six-lane Falkenseer Chaussee
    /// piece that way); driving `a` to its end first and then starting `b` sent the bus
    /// 30 m back and sideways.
    ///
    /// Or `b` runs beside a stretch of `a` without starting where it does: a bus lane or
    /// stop lane of a junction object that begins before or after the lane the route came
    /// in on (Spandau's Klosterstraße, Heerstraße and Altstädter Ring junctions: the station
    /// link reaches the stop on the through lane, the next one leaves from the stop lane
    /// beside it). Driven one after the other the bus jumped up to 58 m back.
    pub fn parallel(&self, a: usize, b: usize) -> bool {
        let (Some(la), Some(lb)) = (self.lanes.get(a), self.lanes.get(b)) else { return false };
        if a == b || la.next.contains(&b) {
            return false;
        }
        let same = match (la.key, lb.key) {
            (Some(x), Some(y)) => x.tile == y.tile && x.id == y.id && x.path != y.path,
            _ => false,
        };
        if same && (lb.start() - la.start()).truncate().length() < 8.0 && (lb.start().z - la.start().z).abs() < 1.0 && wrap_deg(lb.start_heading() - la.start_heading()).abs() < 45.0 {
            return true;
        }
        // (a lane of another spline beside it counts as well: mod maps lay a bus lane or a
        // second carriageway as splines of their own, and Novi Sad's tracks change onto
        // them 30 m before the lane they come in on ends)
        self.beside(la, lb)
    }

    /// Does lane `b` run beside `a` (a lane's width or so to the side, the same way) along
    /// at least 8 m of it?
    fn beside(&self, la: &Lane, lb: &Lane) -> bool {
        let n = 8;
        let (mut along, step) = (0.0f32, la.length() / n as f32);
        for i in 0..=n {
            let s = la.length() * i as f32 / n as f32;
            let (p, h) = la.at(s);
            let Some((sb, d)) = lb.nearest_point(p) else { continue };
            let inside = sb > 0.5 && sb < lb.length() - 0.5;
            // (on the same level: a track or road under a bridge lies a few metres off in 3D
            // too, and a train "changed lanes" down onto the line under its viaduct)
            let (q, hb) = lb.at(sb);
            let level = (q.z - p.z).abs() < 1.0;
            if inside && level && d > 0.8 && d < 5.5 && wrap_deg(hb - h).abs() < 30.0 {
                along += step;
            }
        }
        along >= 8.0 - 1e-3
    }

    /// Where on lane `b`, beside `a` (see `parallel`), the car at distance `s` along `a` is:
    /// the same distance on lanes that start together, else the point of `b` beside it.
    pub fn beside_s(&self, a: usize, b: usize, s: f32) -> f32 {
        let (la, lb) = (&self.lanes[a], &self.lanes[b]);
        if (lb.start() - la.start()).truncate().length() < 8.0 {
            let frac = if la.length() > 0.0 { s / la.length() } else { 0.0 };
            return frac * lb.length();
        }
        (s + self.beside_delta(a, b)).clamp(0.0, lb.length())
    }

    /// How much further along `b` than along `a` a point beside both lies (see `parallel`):
    /// 0 for lanes that start together, the distance `b` starts before `a` (positive) or
    /// after it (negative) otherwise.
    pub fn beside_delta(&self, a: usize, b: usize) -> f32 {
        let (la, lb) = (&self.lanes[a], &self.lanes[b]);
        if (lb.start() - la.start()).truncate().length() < 8.0 {
            return 0.0;
        }
        match la.nearest_point(lb.start()) {
            Some((sa, _)) if sa > 0.5 => -sa,
            _ => lb.nearest_point(la.start()).map(|(sb, _)| sb).unwrap_or(0.0),
        }
    }

    /// Point on a lane sequence closest to `p`: (index into `route`, distance along that lane).
    pub fn project_on_route(&self, route: &[usize], p: DVec3) -> Option<(usize, f32)> {
        self.project_on_route_lateral(route, p).map(|(ri, s, _)| (ri, s))
    }

    /// Like `project_on_route`, plus the lateral offset of `p` from the lane (m, right of
    /// the driving direction positive).
    pub fn project_on_route_lateral(&self, route: &[usize], p: DVec3) -> Option<(usize, f32, f32)> {
        let mut best: Option<(usize, f32, f64, f32)> = None;
        for (ri, &li) in route.iter().enumerate() {
            let l = &self.lanes[li];
            for k in 0..l.points.len().saturating_sub(1) {
                let a = l.points[k];
                let b = l.points[k + 1];
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
                let q = a + ab * t;
                let d = (q - p).truncate().length();
                if best.map(|b| d < b.2).unwrap_or(true) {
                    let dir = ab.truncate().normalize_or_zero();
                    let rel = (p - q).truncate();
                    // right vector of (dir.x, dir.y) is (dir.y, -dir.x)
                    let lateral = (rel.x * dir.y - rel.y * dir.x) as f32;
                    best = Some((ri, l.dist[k] + (l.dist[k + 1] - l.dist[k]) * t as f32, d, lateral));
                }
            }
        }
        best.map(|(ri, s, _, lat)| (ri, s, lat))
    }

    /// Where a bus stop at `p` lies on `route`: like [`Network::project_on_route_lateral`],
    /// but among the points within `reach` (when given) and not before route index
    /// `from`, a lane with the stop on its kerb side (right, or left with left-hand traffic)
    /// goes before a nearer one with the stop across it - a route back along the same
    /// street passes each stop twice, once from the other side, and the bus stopped at the
    /// stop across the road on its way out. Falls back to the nearest point.
    pub fn project_stop_on_route(&self, route: &[usize], p: DVec3, reach: Option<f64>, from: usize) -> Option<(usize, f32, f32)> {
        self.project_stop_on_route_side(route, p, reach, from, 0)
    }

    /// The same for a platform opposite the usual kerb (1), or either side (2).
    /// The side is the stop object's OMSI value, relative to the map's traffic hand.
    pub fn project_stop_on_route_side(&self, route: &[usize], p: DVec3, reach: Option<f64>, from: usize, side: u8) -> Option<(usize, f32, f32)> {
        // (index, s, distance, lateral) of the best on the platform side, and of any
        let mut kerb: Option<(usize, f32, f64, f32)> = None;
        let mut any: Option<(usize, f32, f64, f32)> = None;
        for (ri, &li) in route.iter().enumerate().skip(from.min(route.len())) {
            let l = &self.lanes[li];
            for k in 0..l.points.len().saturating_sub(1) {
                let a = l.points[k];
                let b = l.points[k + 1];
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
                let q = a + ab * t;
                let d = (q - p).truncate().length();
                if reach.is_some_and(|r| d > r) {
                    continue;
                }
                let dir = ab.truncate().normalize_or_zero();
                let rel = (p - q).truncate();
                let lateral = (rel.x * dir.y - rel.y * dir.x) as f32;
                let cand = (ri, l.dist[k] + (l.dist[k + 1] - l.dist[k]) * t as f32, d, lateral);
                let left = self.left_hand != (side == 1);
                let kerb_side = side == 2 || if left { lateral < -0.3 } else { lateral > 0.3 };
                if kerb_side && kerb.map(|b| d < b.2).unwrap_or(true) {
                    kerb = Some(cand);
                }
                if any.map(|b| d < b.2).unwrap_or(true) {
                    any = Some(cand);
                }
            }
        }
        match kerb.or(any) {
            Some((ri, s, _, lat)) => Some((ri, s, lat)),
            None if from > 0 => self.project_stop_on_route_side(route, p, reach, 0, side),
            None => None,
        }
    }

    /// Geometric conflicts between the lanes of each crossing object (paths that intersect
    /// or end at the same point, with where they meet), and the footpaths crossing its
    /// street lanes. AI cars sort out who goes first from these (`Network::must_yield`).
    pub fn compute_conflicts(&mut self) {
        self.conflicts = vec![Vec::new(); self.lanes.len()];
        self.crossings = vec![Vec::new(); self.lanes.len()];
        self.walks = vec![Vec::new(); self.lanes.len()];
        let (n, walks) = self.conflicts_from(0);
        log::info!("path network: {n} conflicting lane pairs at crossings, {walks} footpath crossings");
    }

    /// `conflicts`, `crossings` and `walks` of the junction objects whose lanes start at
    /// index `first` (all lanes of an object come with its tile, so a grown network only
    /// needs its new objects looked at). Returns the pairs and footpath crossings found.
    fn conflicts_from(&mut self, first: usize) -> (usize, usize) {
        let len = self.lanes.len();
        self.conflicts.resize(len, Vec::new());
        self.crossings.resize(len, Vec::new());
        self.walks.resize(len, Vec::new());
        let mut by_object: HashMap<((i32, i32), i64), Vec<usize>> = HashMap::new();
        let mut walks_by_object: HashMap<((i32, i32), i64), Vec<usize>> = HashMap::new();
        for (i, l) in self.lanes.iter().enumerate().skip(first) {
            if l.source != 2 {
                continue;
            }
            let Some(k) = l.key else { continue };
            match l.kind {
                // a level crossing is a junction of a road and a railway
                LaneKind::Street | LaneKind::Rail => by_object.entry((k.tile, k.id)).or_default().push(i),
                LaneKind::Sidewalk => walks_by_object.entry((k.tile, k.id)).or_default().push(i),
                LaneKind::Air => {}
            }
        }
        let mut n = 0;
        for (key, lanes) in &by_object {
            for (x, &i) in lanes.iter().enumerate() {
                for &j in &lanes[x + 1..] {
                    let (a, b) = (&self.lanes[i], &self.lanes[j]);
                    let same_path = a.key.map(|k| k.path) == b.key.map(|k| k.path);
                    if same_path || (a.kind == LaneKind::Rail && b.kind == LaneKind::Rail) {
                        continue;
                    }
                    // one runs into the other: the same end, or ends a car's width apart that
                    // lead into the same lane (two turns converging on one exit)
                    let ends = (a.end() - b.end()).truncate().length();
                    let joint = ends < 1.5 || (ends < MEET_DIST && a.next.iter().any(|n| b.next.contains(n)));
                    let merge = joint && (a.start() - b.start()).truncate().length() > 3.0;
                    let place = if merge { Some((a.length(), b.length())) } else { polyline_crossing(a, b) };
                    // `[blockpath]`: the object says the two are in each other's way even
                    // where their lines do not cross - the whole of both is the meeting place
                    let (pa, pb) = (a.key.map(|k| k.path).unwrap_or(u16::MAX), b.key.map(|k| k.path).unwrap_or(u16::MAX));
                    if place.is_none() && (a.blocks.contains(&pb) || b.blocks.contains(&pa)) {
                        let (la, lb) = (a.length(), b.length());
                        self.conflicts[i].push(j);
                        self.conflicts[j].push(i);
                        self.crossings[i].push(Crossing { other: j, at: la * 0.5, other_at: lb * 0.5, merge: false, before: la * 0.5, after: la * 0.5, other_before: lb * 0.5, other_after: lb * 0.5 });
                        self.crossings[j].push(Crossing { other: i, at: lb * 0.5, other_at: la * 0.5, merge: false, before: lb * 0.5, after: lb * 0.5, other_before: la * 0.5, other_after: la * 0.5 });
                        n += 1;
                        continue;
                    }
                    if let Some((sa, sb)) = place {
                        // (after a joint the two are in one lane, and follow each other)
                        let (ab, aa) = if merge { (meeting_extent(a, sa, b).0, 0.5) } else { meeting_extent(a, sa, b) };
                        let (bb, ba) = if merge { (meeting_extent(b, sb, a).0, 0.5) } else { meeting_extent(b, sb, a) };
                        self.conflicts[i].push(j);
                        self.conflicts[j].push(i);
                        self.crossings[i].push(Crossing { other: j, at: sa, other_at: sb, merge, before: ab, after: aa, other_before: bb, other_after: ba });
                        self.crossings[j].push(Crossing { other: i, at: sb, other_at: sa, merge, before: bb, after: ba, other_before: ab, other_after: aa });
                        n += 1;
                    }
                }
            }
            if let Some(walks) = walks_by_object.get(key) {
                for &i in lanes.iter().filter(|&&i| self.lanes[i].kind == LaneKind::Street) {
                    for &w in walks {
                        if let Some((sa, sw)) = polyline_crossing(&self.lanes[i], &self.lanes[w]) {
                            self.walks[i].push((w, sa, sw));
                        }
                    }
                }
            }
        }
        let walks: usize = self.walks[first..].iter().map(|w| w.len()).sum();
        (n, walks)
    }

    /// Must a vehicle on lane `a` let one on lane `b` go first where they meet? The path with
    /// the higher `[rule] priority` goes first; between equals, a left turn waits for the
    /// oncoming traffic, and otherwise the one coming from the right has the right of way
    /// (the German "rechts vor links" that the stock maps rely on wherever they set no
    /// priorities) - mirrored on a left-hand-traffic map. A train always goes first.
    pub fn must_yield(&self, a: usize, b: usize) -> bool {
        let (la, lb) = (&self.lanes[a], &self.lanes[b]);
        if la.kind == LaneKind::Rail {
            return false;
        }
        if lb.kind == LaneKind::Rail {
            return true;
        }
        if (la.priority - lb.priority).abs() > 0.5 {
            return la.priority < lb.priority;
        }
        let rel = wrap_deg(lb.start_heading() - la.start_heading());
        // (the turn across the oncoming traffic: left, or right when driving on the left)
        let across = if self.left_hand { 2 } else { 1 };
        if rel.abs() > 135.0 {
            // oncoming
            return la.turn == across && lb.turn != across;
        }
        // `b` comes from the right when it travels to the left of `a`'s way (from the left,
        // travelling to its right, on a left-hand-traffic map)
        let rel = if self.left_hand { -rel } else { rel };
        if rel < -30.0 {
            return true;
        }
        if rel > 30.0 {
            return false;
        }
        // side by side from the same direction: the one turning across the other waits
        la.turn != 0 && lb.turn == 0
    }

    /// The lane beside `lane` at `s` that carries the traffic the other way (the other half
    /// of a two-way street): (lane, distance along it at the same place, how far its middle
    /// lies over to the oncoming side - the left, or the right on a left-hand-traffic map;
    /// `oncoming_sign` turns it into a lateral offset).
    pub fn opposite(&self, lane: usize, s: f32) -> Option<(usize, f32, f32)> {
        let l = self.lanes.get(lane)?;
        let (p, h) = l.at(s);
        let hr = (h as f64).to_radians();
        let left = DVec3::new(-hr.cos(), hr.sin(), 0.0);
        let left = if self.left_hand { -left } else { left };
        let cx = (p.x / GRID_CELL).floor() as i32;
        let cy = (p.y / GRID_CELL).floor() as i32;
        let mut best: Option<(usize, f32, f32)> = None;
        let mut seen: Vec<usize> = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &i in self.grid.get(&(cx + dx, cy + dy)).map(|v| v.as_slice()).unwrap_or(&[]) {
                    if i == lane || seen.contains(&i) {
                        continue;
                    }
                    seen.push(i);
                    let o = &self.lanes[i];
                    if o.kind != LaneKind::Street || o.no_cars {
                        continue;
                    }
                    let Some((os, d)) = o.nearest_point(p) else { continue };
                    if d > 7.0 || d < 1.5 {
                        continue;
                    }
                    let (q, oh) = o.at(os);
                    let side = (q - p).dot(left) as f32;
                    if side < 1.5 || wrap_deg(oh - h).abs() < 150.0 {
                        continue;
                    }
                    if best.map(|b| side < b.2).unwrap_or(true) {
                        best = Some((i, os, side));
                    }
                }
            }
        }
        best
    }

    /// The lanes traffic comes from towards distance `s` of `lane`, walked backwards over
    /// joints and through junctions for up to `within` metres: (lane, offset, the lane it
    /// leads into on the way to `lane`). A vehicle at distance `x` along such a lane is at
    /// `x + offset` in `lane`'s own distances (negative before its start); the first entry
    /// is `lane` itself with offset 0. Where several ways lead to one lane, the shortest
    /// counts. At most `max` lanes.
    pub fn upstream(&self, lane: usize, s: f32, within: f32, max: usize) -> Vec<(usize, f32, Option<usize>)> {
        let mut out: Vec<(usize, f32, Option<usize>)> = vec![(lane, 0.0, None)];
        if lane >= self.lanes.len() {
            return out;
        }
        // nearest first (a junction's lanes can be reached two ways)
        let mut done = vec![false];
        while let Some(k) = (0..out.len()).filter(|&k| !done[k]).max_by(|&a, &b| out[a].1.total_cmp(&out[b].1)) {
            done[k] = true;
            let (l, off, _) = out[k];
            // this lane starts `s - off` metres before the place: nothing before it is in reach
            if s - off > within {
                continue;
            }
            for &p in self.prev.get(l).map(|v| v.as_slice()).unwrap_or(&[]) {
                if p == lane || self.lanes[p].kind != self.lanes[lane].kind {
                    continue;
                }
                let o = off - self.lanes[p].length();
                match out.iter().position(|e| e.0 == p) {
                    Some(e) => {
                        if o > out[e].1 && !done[e] {
                            out[e] = (p, o, Some(l));
                        }
                    }
                    None => {
                        if out.len() < max {
                            out.push((p, o, Some(l)));
                            done.push(false);
                        }
                    }
                }
            }
        }
        out
    }

    /// Connect lane ends to lane starts that lie within `tol` metres with a compatible heading.
    pub fn link(&mut self, tol: f64) {
        self.by_key.clear();
        for (i, l) in self.lanes.iter().enumerate() {
            if let Some(k) = l.key {
                self.by_key.entry(k).or_default().push(i);
            }
        }
        self.build_grid();
        // neighbouring lanes: paths of one spline running the same way, next to each other
        let mut by_spline: HashMap<((i32, i32), i64, bool), Vec<usize>> = HashMap::new();
        for (i, l) in self.lanes.iter().enumerate() {
            if let Some(k) = l.key {
                if l.source == 1 && matches!(l.kind, LaneKind::Street) {
                    by_spline.entry((k.tile, k.id as i64, l.reversed)).or_default().push(i);
                }
            }
        }
        let mut neighbours = 0usize;
        for (_, mut list) in by_spline {
            list.sort_by(|a, b| self.lanes[*a].offset.total_cmp(&self.lanes[*b].offset));
            for w in list.windows(2) {
                let (a, b) = (w[0], w[1]);
                let gap = self.lanes[b].offset - self.lanes[a].offset;
                if gap > 1.5 && gap < 6.0 {
                    // b lies at the larger offset: to the right when driving along the spline
                    if neighbours == 0 {
                        let p = self.lanes[a].start();
                        log::info!("first neighbouring lane pair at ({:.1}, {:.1}), heading {:.0}", p.x, p.y, self.lanes[a].start_heading());
                    }
                    neighbours += 1;
                    let reversed = self.lanes[a].reversed;
                    if reversed {
                        self.lanes[a].left = Some(b);
                        self.lanes[b].right = Some(a);
                    } else {
                        self.lanes[a].right = Some(b);
                        self.lanes[b].left = Some(a);
                    }
                }
            }
        }
        log::info!("lanes: {} neighbouring lane pairs (lane changes)", neighbours);
        let cell = 4.0;
        let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, l) in self.lanes.iter().enumerate() {
            let s = l.start();
            grid.entry(((s.x / cell).floor() as i64, (s.y / cell).floor() as i64)).or_default().push(i);
        }
        let mut links = 0;
        for i in 0..self.lanes.len() {
            let e = self.lanes[i].end();
            let eh = self.lanes[i].end_heading();
            let kind = self.lanes[i].kind;
            let cx = (e.x / cell).floor() as i64;
            let cy = (e.y / cell).floor() as i64;
            let mut found = Vec::new();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    if let Some(list) = grid.get(&(cx + dx, cy + dy)) {
                        for &j in list {
                            if j == i {
                                continue;
                            }
                            let l = &self.lanes[j];
                            if l.kind != kind {
                                continue;
                            }
                            let d = (l.start() - e).truncate().length();
                            let dh = wrap_deg(l.start_heading() - eh).abs();
                            if d <= tol && dh < 40.0 && (l.start().z - e.z).abs() < 3.0 {
                                found.push(j);
                            }
                        }
                    }
                }
            }
            if found.is_empty() && omsi_cfg::flags::OMSI_DEBUG_UNLINKED.is_set() {
                // a lane end with a start of its kind near it that was not taken
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for &j in grid.get(&(cx + dx, cy + dy)).map(|v| v.as_slice()).unwrap_or(&[]) {
                            let l = &self.lanes[j];
                            if j != i && l.kind == kind {
                                let d = (l.start() - e).truncate().length();
                                let dh = wrap_deg(l.start_heading() - eh).abs();
                                if d < 4.0 && dh < 40.0 {
                                    log::info!("unlinked: lane {i} {:?} end ({:.2}, {:.2}, {:.2}) -> lane {j} {:?} start {:.2} m away, dz {:.2}, dh {:.1}", self.lanes[i].key, e.x, e.y, e.z, l.key, d, l.start().z - e.z, dh);
                                }
                            }
                        }
                    }
                }
            }
            links += found.len();
            self.lanes[i].next = found;
        }
        log::info!("path network: {} lanes, {} links", self.lanes.len(), links);
        self.prev = vec![Vec::new(); self.lanes.len()];
        for i in 0..self.lanes.len() {
            for &n in &self.lanes[i].next {
                if n < self.lanes.len() {
                    self.prev[n].push(i);
                }
            }
        }
        self.compute_conflicts();
        self.compute_reach();
    }

    /// `reach` of every lane: its length plus the best reach of the lanes after it. The lanes
    /// are settled from the dead ends backwards (a lane once all the lanes after it are); a
    /// lane that never settles has a loop ahead of it and can be driven on for ever.
    pub fn compute_reach(&mut self) {
        let dead = self.update_reach();
        log::info!("path network: {dead} street lanes lead into a dead end within {DEAD_END} m");
    }

    /// `compute_reach` without the log line; returns the street lanes that lead into a dead
    /// end.
    fn update_reach(&mut self) -> usize {
        let n = self.lanes.len();
        // a lane closed to cars is no way on for one that is open
        let counts = |i: usize, j: usize| !self.lanes[j].no_cars || self.lanes[i].no_cars;
        let mut waiting: Vec<usize> = (0..n).map(|i| self.lanes[i].next.iter().filter(|&&j| counts(i, j)).count()).collect();
        let mut reach = vec![REACH_MAX; n];
        let mut settled = vec![false; n];
        let mut ready: Vec<usize> = (0..n).filter(|&i| waiting[i] == 0).collect();
        while let Some(j) = ready.pop() {
            let l = &self.lanes[j];
            let best = l.next.iter().filter(|&&k| counts(j, k)).map(|&k| reach[k]).fold(0.0f32, f32::max);
            reach[j] = (l.length() + best).min(REACH_MAX);
            settled[j] = true;
            for &p in self.prev.get(j).map(|v| v.as_slice()).unwrap_or(&[]) {
                if counts(p, j) && !settled[p] {
                    waiting[p] = waiting[p].saturating_sub(1);
                    if waiting[p] == 0 {
                        ready.push(p);
                    }
                }
            }
        }
        let dead = self.lanes.iter().zip(&reach).filter(|(l, r)| l.kind == LaneKind::Street && **r < DEAD_END).count();
        self.reach = reach;
        dead
    }

    /// Nearest street lane and distance along it to a world point.
    /// Sort the lanes into the grid (called by `link`; call again after adding lanes).
    pub fn build_grid(&mut self) {
        self.grid.clear();
        self.start_grid.clear();
        for (i, l) in self.lanes.iter().enumerate() {
            self.start_grid.entry(Self::grid_cell(l.start())).or_default().push(i);
            let mut cells: Vec<(i32, i32)> = Vec::new();
            for w in l.points.windows(2) {
                // every cell along the segment, sampled finer than a cell
                let n = ((w[1] - w[0]).truncate().length() / (GRID_CELL * 0.5)).ceil().max(1.0) as usize;
                for k in 0..=n {
                    let q = w[0].lerp(w[1], k as f64 / n as f64);
                    let c = ((q.x / GRID_CELL).floor() as i32, (q.y / GRID_CELL).floor() as i32);
                    if !cells.contains(&c) {
                        cells.push(c);
                    }
                }
            }
            if l.points.len() == 1 {
                cells.push(((l.points[0].x / GRID_CELL).floor() as i32, (l.points[0].y / GRID_CELL).floor() as i32));
            }
            for c in cells {
                self.grid.entry(c).or_default().push(i);
            }
        }
    }

    /// Lane indices whose starts lie in cells intersecting a circle around `p`.
    /// Callers still apply their own exact distance and lane-kind tests. Sorting keeps
    /// their traversal (and seeded traffic selection) in map order.
    pub fn lanes_starting_near(&self, p: DVec3, radius: f64) -> Vec<usize> {
        let min_x = ((p.x - radius) / GRID_CELL).floor() as i32;
        let max_x = ((p.x + radius) / GRID_CELL).floor() as i32;
        let min_y = ((p.y - radius) / GRID_CELL).floor() as i32;
        let max_y = ((p.y + radius) / GRID_CELL).floor() as i32;
        let cells = (max_x as i64 - min_x as i64 + 1) * (max_y as i64 - min_y as i64 + 1);
        if self.start_grid.is_empty() || cells > self.start_grid.len() as i64 * 2 {
            return (0..self.lanes.len()).collect();
        }
        let mut out = Vec::new();
        for x in min_x..=max_x {
            for y in min_y..=max_y {
                if let Some(lanes) = self.start_grid.get(&(x, y)) {
                    out.extend_from_slice(lanes);
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// The nearest lane of `kind` to `p`: (lane, distance along it, distance to it). With
    /// the grid built, only lanes within about a cell of `p` are considered - enough for
    /// everything that asks where a vehicle or a person stands.
    pub fn nearest_lane(&self, p: DVec3, kind: LaneKind) -> Option<(usize, f32, f64)> {
        if !self.grid.is_empty() {
            if let Some(best) = self.nearest_lane_near(p, kind) {
                return Some(best);
            }
        }
        let mut best: Option<(usize, f32, f64)> = None;
        for (i, l) in self.lanes.iter().enumerate() {
            if l.kind != kind {
                continue;
            }
            for k in 0..l.points.len().saturating_sub(1) {
                let a = l.points[k];
                let b = l.points[k + 1];
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
                let q = a + ab * t;
                let d = (q - p).length();
                if best.map(|b| d < b.2).unwrap_or(true) {
                    let s = l.dist[k] + (l.dist[k + 1] - l.dist[k]) * t as f32;
                    best = Some((i, s, d));
                }
            }
        }
        best
    }

    /// Find shortest lane path from `start` to `target` using Dijkstra's algorithm.
    pub fn shortest_path(&self, start: usize, target: usize) -> Option<Vec<usize>> {
        if start >= self.lanes.len() || target >= self.lanes.len() {
            return None;
        }
        if start == target {
            return Some(vec![start]);
        }
        use std::cmp::Ordering;
        use std::collections::BinaryHeap;

        #[derive(Copy, Clone, PartialEq)]
        struct State {
            cost: f32,
            position: usize,
        }
        impl Eq for State {}
        impl Ord for State {
            fn cmp(&self, other: &Self) -> Ordering {
                other.cost.total_cmp(&self.cost)
            }
        }
        impl PartialOrd for State {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        let mut dist = vec![f32::INFINITY; self.lanes.len()];
        let mut parent = vec![None; self.lanes.len()];
        let mut heap = BinaryHeap::new();

        dist[start] = 0.0;
        heap.push(State { cost: 0.0, position: start });

        while let Some(State { cost, position }) = heap.pop() {
            if position == target {
                let mut path = Vec::new();
                let mut curr = Some(target);
                while let Some(node) = curr {
                    path.push(node);
                    curr = parent[node];
                }
                path.reverse();
                return Some(path);
            }

            if cost > dist[position] {
                continue;
            }

            for &next_idx in &self.lanes[position].next {
                if next_idx >= self.lanes.len() {
                    continue;
                }
                let next_len = self.lanes[next_idx].length();
                let next_cost = cost + next_len;
                if next_cost < dist[next_idx] {
                    dist[next_idx] = next_cost;
                    parent[next_idx] = Some(position);
                    heap.push(State { cost: next_cost, position: next_idx });
                }
            }
        }
        None
    }
}

/// Growing the network while the map streams in: the lanes of newly loaded tiles are
/// appended (existing indices stay valid for the cars and routes that hold them) and linked
/// to what is there, with the same rules as [`Network::link`].
/// How near (m) a lane's end and the next one's start must lie to be linked: what the game
/// links its road network with, and what a driving choice measures short paths against.
pub const LINK_TOLERANCE: f64 = 1.5;

impl Network {
    /// Append `new` lanes and link them. Returns the range of their indices.
    pub fn extend(&mut self, new: Vec<Lane>, tol: f64) -> std::ops::Range<usize> {
        let first = self.lanes.len();
        if new.is_empty() {
            return first..first;
        }
        if first == 0 {
            self.lanes = new;
            self.link(tol);
            return 0..self.lanes.len();
        }
        self.lanes.extend(new);
        let end = self.lanes.len();
        for i in first..end {
            if let Some(k) = self.lanes[i].key {
                self.by_key.entry(k).or_default().push(i);
            }
            self.start_grid.entry(Self::grid_cell(self.lanes[i].start())).or_default().push(i);
            for c in Self::lane_cells(&self.lanes[i]) {
                self.grid.entry(c).or_default().push(i);
            }
        }
        // neighbouring lanes of the new splines
        let mut by_spline: HashMap<((i32, i32), i64, bool), Vec<usize>> = HashMap::new();
        for i in first..end {
            let l = &self.lanes[i];
            if let (Some(k), 1, LaneKind::Street) = (l.key, l.source, l.kind) {
                by_spline.entry((k.tile, k.id, l.reversed)).or_default().push(i);
            }
        }
        for (_, mut list) in by_spline {
            list.sort_by(|a, b| self.lanes[*a].offset.total_cmp(&self.lanes[*b].offset));
            for w in list.windows(2) {
                let (a, b) = (w[0], w[1]);
                let gap = self.lanes[b].offset - self.lanes[a].offset;
                if gap > 1.5 && gap < 6.0 {
                    if self.lanes[a].reversed {
                        self.lanes[a].left = Some(b);
                        self.lanes[b].right = Some(a);
                    } else {
                        self.lanes[a].right = Some(b);
                        self.lanes[b].left = Some(a);
                    }
                }
            }
        }
        // links: ends of new lanes to any start, ends of old lanes to new starts
        let joins = |net: &Network, from: usize, to: usize| -> bool {
            let (a, b) = (&net.lanes[from], &net.lanes[to]);
            if from == to || a.kind != b.kind {
                return false;
            }
            let (e, s) = (a.end(), b.start());
            let dh = ((b.start_heading() - a.end_heading() + 540.0) % 360.0 - 180.0).abs();
            (s - e).truncate().length() <= tol && dh < 40.0 && (s.z - e.z).abs() < 3.0
        };
        let near = |net: &Network, p: DVec3| -> Vec<usize> {
            let c = ((p.x / GRID_CELL).floor() as i32, (p.y / GRID_CELL).floor() as i32);
            let mut out: Vec<usize> = Vec::new();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    if let Some(list) = net.grid.get(&(c.0 + dx, c.1 + dy)) {
                        out.extend(list.iter().copied());
                    }
                }
            }
            out.sort_unstable();
            out.dedup();
            out
        };
        let mut added: Vec<(usize, usize)> = Vec::new();
        for i in first..end {
            let e = self.lanes[i].end();
            let found: Vec<usize> = near(self, e).into_iter().filter(|&j| joins(self, i, j)).collect();
            for &j in &found {
                added.push((i, j));
            }
            self.lanes[i].next = found;
            let s = self.lanes[i].start();
            for j in near(self, s) {
                if j < first && joins(self, j, i) && !self.lanes[j].next.contains(&i) {
                    self.lanes[j].next.push(i);
                    added.push((j, i));
                }
            }
        }
        self.prev.resize(end, Vec::new());
        for (from, to) in added {
            if !self.prev[to].contains(&from) {
                self.prev[to].push(from);
            }
        }
        // meeting places and footpath crossings inside the new junction objects, and how far
        // every lane now leads (a dead end may go on into the new tiles)
        self.conflicts_from(first);
        self.update_reach_from(first);
        first..end
    }

    /// Appending lanes can only change the reach of those lanes and their predecessors.
    /// An outgoing edge into the rest of the network uses its already settled reach. This
    /// avoids rewalking every loaded tile whenever the streamer adds a small batch.
    fn update_reach_from(&mut self, first: usize) {
        if self.reach.len() != first {
            self.update_reach();
            return;
        }
        let mut affected: hashbrown::HashSet<usize> = (first..self.lanes.len()).collect();
        let mut pending: Vec<usize> = affected.iter().copied().collect();
        while let Some(i) = pending.pop() {
            // The vector-based full pass is cheaper once much of the network leads
            // into this tile (for example a strongly connected city grid).
            if affected.len() > self.lanes.len() / 4 {
                self.update_reach();
                return;
            }
            for &p in &self.prev[i] {
                if affected.insert(p) {
                    pending.push(p);
                }
            }
        }
        self.reach.resize(self.lanes.len(), REACH_MAX);
        let counts = |i: usize, j: usize| !self.lanes[j].no_cars || self.lanes[i].no_cars;
        let mut waiting: HashMap<usize, usize> = affected.iter().map(|&i| {
            (i, self.lanes[i].next.iter().filter(|&&j| affected.contains(&j) && counts(i, j)).count())
        }).collect();
        let mut ready: Vec<usize> = waiting.iter().filter_map(|(&i, &n)| (n == 0).then_some(i)).collect();
        for &i in &affected {
            self.reach[i] = REACH_MAX;
        }
        while let Some(i) = ready.pop() {
            let l = &self.lanes[i];
            let best = l.next.iter().filter(|&&j| counts(i, j)).map(|&j| self.reach[j]).fold(0.0f32, f32::max);
            self.reach[i] = (l.length() + best).min(REACH_MAX);
            for &p in &self.prev[i] {
                if counts(p, i) {
                    if let Some(n) = waiting.get_mut(&p) {
                        *n = n.saturating_sub(1);
                        if *n == 0 {
                            ready.push(p);
                        }
                    }
                }
            }
        }
    }

    /// Like [`nearest_lane`](Self::nearest_lane), but only among the lanes in the grid cells
    /// around `p` (a cell is [`GRID_CELL`]): None when there are none there, never a search
    /// of the whole network.
    pub fn nearest_lane_near(&self, p: DVec3, kind: LaneKind) -> Option<(usize, f32, f64)> {
        let (cx, cy) = Self::grid_cell(p);
        let mut best: Option<(usize, f32, f64)> = None;
        let mut seen: Vec<usize> = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(list) = self.grid.get(&(cx + dx, cy + dy)) else { continue };
                for &i in list {
                    if seen.contains(&i) {
                        continue;
                    }
                    seen.push(i);
                    let l = &self.lanes[i];
                    if l.kind != kind {
                        continue;
                    }
                    if let Some((s, d)) = l.nearest_point(p) {
                        if best.map(|b| d < b.2).unwrap_or(true) {
                            best = Some((i, s, d));
                        }
                    }
                }
            }
        }
        best
    }

    /// The nearest lane of `kind` within `max_dist` of `p` that runs within `max_turn`
    /// degrees of `heading` there (a vehicle's own lane, not the one beside it going the
    /// other way): (lane, distance along it, distance to it).
    pub fn lane_along(&self, p: DVec3, heading: f64, kind: LaneKind, max_dist: f64, max_turn: f64) -> Option<(usize, f32, f64)> {
        let (cx, cy) = Self::grid_cell(p);
        let mut best: Option<(usize, f32, f64)> = None;
        let mut seen: Vec<usize> = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(list) = self.grid.get(&(cx + dx, cy + dy)) else { continue };
                for &i in list {
                    if seen.contains(&i) {
                        continue;
                    }
                    seen.push(i);
                    let l = &self.lanes[i];
                    if l.kind != kind {
                        continue;
                    }
                    let Some((s, d)) = l.nearest_point(p) else { continue };
                    if d > max_dist || best.map(|b| d >= b.2).unwrap_or(false) {
                        continue;
                    }
                    let turn = (l.at(s).1 as f64 - heading + 540.0).rem_euclid(360.0) - 180.0;
                    if turn.abs() <= max_turn {
                        best = Some((i, s, d));
                    }
                }
            }
        }
        best
    }

    /// The grid cell `p` lies in.
    pub fn grid_cell(p: DVec3) -> (i32, i32) {
        ((p.x / GRID_CELL).floor() as i32, (p.y / GRID_CELL).floor() as i32)
    }

    /// The grid cells a lane's points touch (see [`Network::build_grid`]).
    pub fn lane_cells(l: &Lane) -> Vec<(i32, i32)> {
        let mut cells: Vec<(i32, i32)> = Vec::new();
        for w in l.points.windows(2) {
            let n = ((w[1] - w[0]).truncate().length() / (GRID_CELL * 0.5)).ceil().max(1.0) as usize;
            for k in 0..=n {
                let q = w[0].lerp(w[1], k as f64 / n as f64);
                let c = ((q.x / GRID_CELL).floor() as i32, (q.y / GRID_CELL).floor() as i32);
                if !cells.contains(&c) {
                    cells.push(c);
                }
            }
        }
        if l.points.len() == 1 {
            cells.push(((l.points[0].x / GRID_CELL).floor() as i32, (l.points[0].y / GRID_CELL).floor() as i32));
        }
        cells
    }
}

#[cfg(test)]
mod extend_tests {
    use super::*;

    fn straight(x: f64, y0: f64, y1: f64, id: i64, tile: (i32, i32)) -> Lane {
        let mut l = LaneBuilder::polyline(vec![DVec3::new(x, y0, 0.0), DVec3::new(x, y1, 0.0)], LaneKind::Street, 3.0);
        l.key = Some(LaneKey { tile, id, path: 0 });
        l.source = 1;
        l
    }

    #[test]
    fn extend_links_like_link() {
        let all = vec![straight(0.0, 0.0, 100.0, 1, (0, 0)), straight(0.0, 100.0, 200.0, 2, (0, 0)), straight(0.0, 200.0, 300.0, 3, (0, 1)), straight(0.0, 300.0, 400.0, 4, (0, 1))];
        let mut whole = Network { lanes: all.clone(), ..Default::default() };
        whole.link(1.5);
        let mut grown = Network { lanes: all[..2].to_vec(), ..Default::default() };
        grown.link(1.5);
        let r = grown.extend(all[2..].to_vec(), 1.5);
        assert_eq!(r, 2..4);
        for i in 0..4 {
            assert_eq!(grown.lanes[i].next, whole.lanes[i].next, "lane {i}");
            assert_eq!(grown.prev[i], whole.prev[i], "lane {i}");
        }
        assert_eq!(grown.reach, whole.reach);
        assert_eq!(grown.crossings.len(), 4);
        assert_eq!(grown.find(LaneKey { tile: (0, 1), id: 3, path: 0 }, None), Some(2));
        assert_eq!(grown.nearest_lane(DVec3::new(0.5, 350.0, 0.0), LaneKind::Street).map(|n| n.0), Some(3));
    }

    #[test]
    fn short_road_choices_progress_without_changing_linking_or_reach() {
        for heading in [0.0, 129.0, 309.0] {
            let mut p = DVec3::new(-1200.0, -600.0, 20.0);
            let mut all = Vec::new();
            for length in [30.0, 0.47, 1.0, 25.0] {
                let lane = LaneBuilder::arc(p, heading, length, 0.0, length * 0.05, LaneKind::Street, 3.0);
                p = lane.end();
                all.push(lane);
            }
            let mut whole = Network { lanes: all.clone(), ..Default::default() };
            whole.link(1.5);
            for i in 0..3 {
                assert!(whole.lanes[i].next.contains(&(i + 1)), "missing forward joint {i}");
                let mut driver = AiState::new(i, 0.0, 7);
                assert!(driver.choose_after(&whole, i).is_some_and(|j| j > i), "backward choice {i}");
            }
            assert!(whole.lanes[3].next.is_empty());
            assert!(whole.lanes[2].next.contains(&1), "original graph must be preserved");
            assert_eq!(whole.reach[0], REACH_MAX);
            // Both new->old and old->new joins must use the same test as link().
            for split in 1..all.len() {
                let mut grown = Network { lanes: all[..split].to_vec(), ..Default::default() };
                grown.link(1.5);
                grown.extend(all[split..].to_vec(), 1.5);
                for i in 0..all.len() {
                    let mut got = grown.lanes[i].next.clone();
                    let mut want = whole.lanes[i].next.clone();
                    got.sort_unstable();
                    want.sort_unstable();
                    assert_eq!(got, want, "heading {heading}, split {split}, lane {i}");
                }
                assert_eq!(grown.reach, whole.reach);
                let mut driver = AiState::new(2, 0.0, 7);
                assert_eq!(driver.choose_after(&grown, 2), Some(3));
            }
        }
    }

    #[test]
    fn short_forward_paths_keep_gaps_and_overlapping_starts() {
        for heading in [0.0, 129.0, 309.0] {
            let a = LaneBuilder::arc(DVec3::ZERO, heading, 30.0, 0.0, 0.0, LaneKind::Street, 3.0);
            for gap in [-0.4, 0.0, 0.4] {
                let start = a.end() + heading_dir(heading).extend(0.0) * gap;
                let b = LaneBuilder::arc(start, heading, 1.0, 0.0, 0.0, LaneKind::Street, 3.0);
                let mut net = Network { lanes: vec![a.clone(), b], ..Default::default() };
                net.link(1.5);
                assert_eq!(net.lanes[0].next, vec![1], "heading {heading}, gap {gap}");
                let mut driver = AiState::new(0, 0.0, 7);
                assert_eq!(driver.choose_after(&net, 0), Some(1));
            }
            // A sharp, short turn can end behind its entry tangent. When its start
            // is the joint itself, it has not been passed and remains a valid path.
            let turn = LaneBuilder::arc(a.end(), heading, 1.0, 0.3, 0.0, LaneKind::Street, 3.0);
            let mut net = Network { lanes: vec![a, turn], ..Default::default() };
            net.link(1.5);
            assert_eq!(net.lanes[0].next, vec![1], "short turn at heading {heading}");
            let mut driver = AiState::new(0, 0.0, 7);
            assert_eq!(driver.choose_after(&net, 0), Some(1));
        }
    }

    #[test]
    fn short_merge_paths_and_nonroad_connections_keep_their_existing_links() {
        // Two approaches end together. The short approach starts behind the long
        // one's endpoint, but there is no return link and no two-path routing cycle.
        let a = straight(0.0, 0.0, 30.0, 1, (0, 0));
        let b = straight(0.0, 29.0, 29.999, 2, (0, 0));
        let mut net = Network { lanes: vec![a, b], ..Default::default() };
        net.link(1.5);
        assert_eq!(net.lanes[0].next, vec![1]);
        assert!(net.lanes[1].next.is_empty());
        let mut driver = AiState::new(0, 0.0, 7);
        assert_eq!(driver.choose_after(&net, 0), Some(1));

        // Short paths whose ends are near each other but do not have the inverse
        // join also retain their connection.
        let mut net = Network {
            lanes: vec![straight(0.0, 0.0, 1.4, 1, (0, 0)), straight(1.4, 0.9, 1.2, 2, (0, 0))],
            ..Default::default()
        };
        net.link(1.5);
        assert_eq!(net.lanes[0].next, vec![1]);
        assert!(net.lanes[1].next.is_empty());
        let mut driver = AiState::new(0, 0.0, 7);
        assert_eq!(driver.choose_after(&net, 0), Some(1));

        for kind in [LaneKind::Sidewalk, LaneKind::Rail, LaneKind::Air] {
            let mut all = vec![straight(0.0, 0.0, 0.47, 1, (0, 0)), straight(0.0, 0.47, 1.47, 2, (0, 1))];
            for l in &mut all { l.kind = kind; }
            let mut net = Network { lanes: all.clone(), ..Default::default() };
            net.link(1.5);
            assert_eq!(net.lanes[0].next, vec![1]);
            assert_eq!(net.lanes[1].next, vec![0]);
            let mut driver = AiState::new(1, 0.0, 7);
            assert_eq!(driver.choose_after(&net, 1), Some(0));
            let mut grown = Network::default();
            grown.extend(all[..1].to_vec(), 1.5);
            grown.extend(all[1..].to_vec(), 1.5);
            assert_eq!(grown.lanes[0].next, net.lanes[0].next);
            assert_eq!(grown.lanes[1].next, net.lanes[1].next);
        }
    }

    #[test]
    fn start_grid_tracks_added_lanes_in_map_order() {
        let first = vec![straight(0.0, 0.0, 100.0, 1, (0, 0)), straight(300.0, 0.0, 100.0, 2, (1, 0))];
        let mut net = Network { lanes: first, ..Default::default() };
        net.link(1.5);
        let near = |net: &Network| net.lanes_starting_near(DVec3::ZERO, 60.0).into_iter()
            .filter(|&i| net.lanes[i].start().truncate().length() < 60.0).collect::<Vec<_>>();
        assert_eq!(near(&net), vec![0]);
        net.extend(vec![straight(25.0, 0.0, 100.0, 3, (0, 1))], 1.5);
        assert_eq!(near(&net), vec![0, 2]);
    }

    #[test]
    fn added_lanes_only_change_reach_of_their_predecessors() {
        let old = vec![
            straight(0.0, 0.0, 100.0, 1, (0, 0)),
            straight(0.0, 100.0, 200.0, 2, (0, 0)),
            straight(500.0, 0.0, 100.0, 3, (2, 0)),
        ];
        let extra = vec![straight(0.0, 200.0, 300.0, 4, (0, 1))];
        let mut grown = Network { lanes: old.clone(), ..Default::default() };
        grown.link(1.5);
        let distant_reach = grown.reach[2];
        grown.extend(extra.clone(), 1.5);
        let mut whole = Network { lanes: old.into_iter().chain(extra).collect(), ..Default::default() };
        whole.link(1.5);
        assert_eq!(grown.reach, whole.reach);
        assert_eq!(grown.reach[2], distant_reach);
    }
}

/// A `[traffic_light_stop]` / `[traffic_light_jump]` of a light program (`TAmpelStop`:
/// ampel, time, jumptime, ifAnf). When the cycle clock reaches `time` and the condition
/// holds, the clock waits there (stop) or continues at `jump_to` (jump). With `if_request`
/// set the condition is "nobody is asking at `light`": the stock railway crossings keep the
/// road green at 1 s until a train approaches, the bus loop of Heerstraße skips its bus
/// phase when no bus is waiting, the depot gates jump over their phases when nothing comes.
/// Without it the condition is "somebody is asking": a gate stays green while buses keep
/// coming, a barrier stays closed while the train is still there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightStop {
    pub light: usize,
    pub time: f32,
    pub if_request: bool,
    pub jump_to: Option<f32>,
}

/// How far before its stop line a vehicle asks a light for green when the program gives no
/// `[approachdist]` (m): about four seconds at town speed.
pub const DEFAULT_APPROACH: f32 = 50.0;

/// The `TrafficLightPhase` of an object no crossing light drives: a signal without a
/// `[varparent]`, one whose crossing has no program, any other scripted object. Omsi.exe
/// binds every placed object's `TrafficLightPhase` and `TrafficLightApproach` to a light
/// of its parent crossing ("RefreshAmpelParenting", 0x77d460) and, failing that, to a
/// shared dummy variable in .bss (0x8619a4), which reads 0: red, not "no light".
pub const UNLINKED_PHASE: i32 = 0;

/// Traffic light program of a crossing object instance (`TAmpelGroup`): its lights'
/// phases and one cycle clock that runs on game time. The clock is state, not a function
/// of the time of day, because the program may wait at a stop point or jump.
#[derive(Debug, Clone)]
pub struct TrafficLightController {
    /// Per light: phases (state, duration).
    pub lights: Vec<Vec<(i32, f32)>>,
    /// Cycle length (`[traffic_lights_group]`); 0 = the longest light's phases.
    pub cycle: f32,
    pub offset: f32,
    /// `[approachdist]` per light.
    pub approach: Vec<Option<f32>>,
    pub stops: Vec<LightStop>,
    /// Position in the cycle (s).
    pub time: f64,
    /// Per light: a vehicle (or a pedestrian) is asking for it this frame.
    pub request: Vec<bool>,
    /// The clock waits at a stop point.
    pub held: bool,
    /// Points already let past at this instant. Remember all of them so coincident
    /// inactive points cannot keep making one another eligible again.
    passed: Vec<usize>,
    /// A short backwards jump has replayed its stretch once. Do not take it again before
    /// the clock has moved past its source time.
    rewound: Option<usize>,
    started: bool,
}

impl TrafficLightController {
    pub fn new(lights: Vec<Vec<(i32, f32)>>, cycle: f32) -> TrafficLightController {
        let n = lights.len();
        TrafficLightController { lights, cycle, offset: 0.0, approach: vec![None; n], stops: Vec::new(), time: 0.0, request: vec![false; n], held: false, passed: Vec::new(), rewound: None, started: false }
    }

    /// From the `[traffic_light]` program of a crossing object: (per light: name, phases
    /// as (state, seconds), `[approachdist]`), the cycle, the stop and jump points.
    pub fn from_program(lights: Vec<(Vec<(i32, f32)>, Option<f32>)>, cycle: Option<f32>, stops: &[[f32; 3]], jumps: &[[f32; 4]]) -> TrafficLightController {
        let approach = lights.iter().map(|l| l.1).collect();
        let mut c = TrafficLightController::new(lights.into_iter().map(|l| l.0).collect(), cycle.unwrap_or(0.0));
        c.approach = approach;
        for s in stops {
            c.stops.push(LightStop { light: s[0].max(0.0) as usize, time: s[1], if_request: s[2] > 0.5, jump_to: None });
        }
        for j in jumps {
            c.stops.push(LightStop { light: j[0].max(0.0) as usize, time: j[1], if_request: j[2] > 0.5, jump_to: Some(j[3]) });
        }
        c
    }

    /// Length of the cycle in seconds.
    pub fn cycle_len(&self) -> f64 {
        if self.cycle > 0.0 {
            return self.cycle as f64;
        }
        self.lights.iter().map(|p| p.iter().map(|x| x.1).sum::<f32>()).fold(0.0f32, f32::max).max(1.0) as f64
    }

    /// Set the clock from the time of day (s) the first time the program runs: crossings
    /// with the same cycle length then run in step, as a coordinated street would.
    pub fn start(&mut self, day_time: f64) {
        if !self.started {
            self.time = (day_time + self.offset as f64).rem_euclid(self.cycle_len());
            self.started = true;
        }
    }

    /// Request distance of light `i` (m).
    pub fn approach_dist(&self, i: usize) -> f32 {
        self.approach.get(i).copied().flatten().unwrap_or(DEFAULT_APPROACH)
    }

    /// Run the cycle clock on by `dt` seconds of game time, honouring the stop and jump
    /// points with this frame's requests (`request`, set by the caller before).
    pub fn advance(&mut self, dt: f32) {
        let cycle = self.cycle_len();
        let mut left = dt.max(0.0) as f64;
        self.held = false;
        let clear_rewind = |this: &mut Self, move_by: f64| {
            let Some(k) = this.rewound else { return };
            let source = this.stops[k].time as f64;
            if ((this.time - source).abs() < 1e-6 && move_by > 1e-6)
                || (this.time < source && this.time + move_by > source + 1e-6)
            {
                this.rewound = None;
            }
        };
        // Allow every coincident inactive point without losing this frame's time,
        // but still bound authored jump loops.
        for _ in 0..16 + self.stops.len() {
            let mut best: Option<(usize, f64)> = None;
            for (k, p) in self.stops.iter().enumerate() {
                if self.rewound == Some(k) {
                    continue;
                }
                let d = (p.time as f64 - self.time).rem_euclid(cycle);
                let d = if d > cycle - 1e-6 { 0.0 } else { d };
                if d < 1e-6 && self.passed.contains(&k) {
                    continue;
                }
                if d <= left && best.map(|b| d < b.1).unwrap_or(true) {
                    best = Some((k, d));
                }
            }
            let Some((k, d)) = best else {
                if left > 0.0 {
                    clear_rewind(self, left);
                    self.passed.clear();
                }
                self.time = (self.time + left).rem_euclid(cycle);
                return;
            };
            if d > 1e-6 {
                self.passed.clear();
            }
            clear_rewind(self, d);
            self.time = (self.time + d).rem_euclid(cycle);
            left -= d;
            let p = self.stops[k];
            let asked = self.request.get(p.light).copied().unwrap_or(false);
            let active = if p.if_request { !asked } else { asked };
            if !active {
                self.passed.push(k);
                continue;
            }
            match p.jump_to {
                Some(to) => {
                    // A jump a couple of seconds back while somebody asks extends the
                    // current phase. It is not a loop: after replaying that small stretch,
                    // continue through it. One taken while nobody asks is the program's
                    // rest: the main road stays green round it until a request comes
                    // (Winsenburg's win-4: replayed once, it ran through yellow and red
                    // with nobody waiting and flickered between green and yellow, #1722).
                    self.rewound = (!p.if_request && to > 1e-6 && to < p.time - 1e-6).then_some(k);
                    self.time = (to as f64).rem_euclid(cycle);
                    self.passed.clear();
                    if left <= 0.0 {
                        return;
                    }
                }
                None => {
                    self.time = p.time as f64;
                    self.held = true;
                    self.passed.clear();
                    return;
                }
            }
        }
    }

    /// State of light `i` at position `x` of the cycle (s).
    pub fn state_at(&self, i: usize, x: f64) -> i32 {
        let phases = match self.lights.get(i) {
            Some(p) if !p.is_empty() => p,
            _ => return 6,
        };
        let mut x = x.rem_euclid(self.cycle_len()) as f32;
        for (state, dur) in phases {
            if x < *dur {
                return *state;
            }
            x -= dur;
        }
        // after the last phase its state holds until the cycle starts again
        phases.last().map(|p| p.0).unwrap_or(0)
    }

    /// Current state of light `i` (the `TrafficLightPhase` value of its lamps).
    pub fn state(&self, i: usize) -> i32 {
        self.state_at(i, self.time)
    }

    /// Seconds until light `i` next shows another state, if the clock keeps running (None
    /// within a whole cycle).
    pub fn time_to_change(&self, i: usize) -> Option<f32> {
        let now = self.state(i);
        let cycle = self.cycle_len();
        let mut t = 0.25;
        while t <= cycle {
            if self.state_at(i, self.time + t) != now {
                return Some(t as f32);
            }
            t += 0.25;
        }
        None
    }

    /// Seconds until light `i` lets vehicles go (0 while it does), if the clock keeps
    /// running; None when it never does within a cycle.
    pub fn time_until_go(&self, i: usize) -> Option<f32> {
        if Self::allows_go(self.state(i)) {
            return Some(0.0);
        }
        let cycle = self.cycle_len();
        let mut t = 0.25;
        while t <= cycle {
            if Self::allows_go(self.state_at(i, self.time + t)) {
                return Some(t as f32);
            }
            t += 0.25;
        }
        None
    }

    /// Index of the phase light `i` is in (debugging).
    pub fn phase_index(&self, i: usize) -> i32 {
        let phases = match self.lights.get(i) {
            Some(p) if !p.is_empty() => p,
            _ => return -1,
        };
        let mut x = self.time.rem_euclid(self.cycle_len()) as f32;
        for (k, (_, dur)) in phases.iter().enumerate() {
            if x < *dur {
                return k as i32;
            }
            x -= dur;
        }
        phases.len() as i32 - 1
    }

    /// Seconds until light `i` leaves the state it shows now while the clock runs on
    /// (phases in a row with the same state count as one; a stop point may hold it
    /// longer). A pedestrian starts across only when the green lasts.
    pub fn remaining(&self, i: usize) -> f32 {
        let phases = match self.lights.get(i) {
            Some(p) if !p.is_empty() => p,
            _ => return f32::INFINITY,
        };
        let cycle = self.cycle_len() as f32;
        let x = self.time.rem_euclid(self.cycle_len()) as f32;
        // the schedule of one cycle: (state, start, end); a last phase of length 0 (or a
        // cycle longer than the phases) lasts until the cycle ends
        let mut spans: Vec<(i32, f32, f32)> = Vec::new();
        let mut start = 0.0;
        for (k, (state, dur)) in phases.iter().enumerate() {
            let end = if k + 1 == phases.len() { cycle.max(start + dur) } else { start + dur };
            spans.push((*state, start, end));
            start = end;
        }
        let Some(k) = spans.iter().position(|s| x < s.2).or(Some(spans.len() - 1)) else { return f32::INFINITY };
        let state = spans[k].0;
        if spans.iter().all(|s| s.0 == state) {
            return f32::INFINITY;
        }
        // run on through the following phases (wrapping round) while the state stays
        let mut left = spans[k].2 - x;
        let mut j = (k + 1) % spans.len();
        while spans[j].0 == state && j != k {
            left += spans[j].2 - spans[j].1;
            j = (j + 1) % spans.len();
        }
        left
    }

    /// OMSI light states as the stock lamp scripts read them: 0..2 red, 3..5 red and
    /// yellow, 6..8 green (8: the GDR's green with yellow), 9..11 yellow, 12 and above dark.
    /// (The code once read 8 as yellow and 9 as all-red: every stock program's yellow, 9,
    /// showed red, so the lights went from green straight to red.)
    pub fn aspect(state: i32) -> Aspect {
        match state {
            0..=2 => Aspect::Red,
            3..=5 => Aspect::RedYellow,
            6 | 7 => Aspect::Green,
            8 => Aspect::GreenYellow,
            9..=11 => Aspect::Yellow,
            _ => Aspect::Dark,
        }
    }

    /// May a vehicle drive over the stop line now (not counting yellow, which is the
    /// driver's decision)?
    pub fn allows_go(state: i32) -> bool {
        matches!(Self::aspect(state), Aspect::Green | Aspect::Dark)
    }

    /// Lamp variables (red, yellow, green) of a state, for lamp objects without a script
    /// (the rules of the stock `ampel1_ddr.osc`, which agree with `ampel1.osc` for the
    /// states the West Berlin programs use).
    pub fn lamps(state: i32) -> (bool, bool, bool) {
        (matches!(state, 0..=5), matches!(state, 3..=5 | 8..=11), matches!(state, 6..=8))
    }
}

/// What a traffic light shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aspect {
    Red,
    RedYellow,
    Green,
    GreenYellow,
    Yellow,
    Dark,
}

/// How far ahead (m) a car decides which way it goes: far enough to signal a turn in good
/// time and for the steering to look beyond the junction.
const PLAN_AHEAD: f32 = 90.0;
/// At most this many lanes are planned ahead (junction pieces can be a few metres long).
const PLAN_LANES: usize = 10;
/// Linked lanes may end up to 1.5 m apart; over this many metres on either side of a joint
/// the way is bent so that it has no step (half the gap on each side).
const JOINT_BLEND: f32 = 6.0;
/// Seconds of signalling before a lane change begins to move sideways.
const SIGNAL_BEFORE_CHANGE: f32 = 1.2;

/// One AI vehicle moving on the network: where it is along its lanes and which way it is
/// going. The body that follows this way is `ai_motion::AiBody`.
#[derive(Debug, Clone)]
pub struct AiState {
    pub traffic_pool: Option<(usize, std::sync::Arc<Vec<i32>>)>,
    /// The vehicle's `[ai_veh_type]` (0 car, 1 taxi, 2 bus, 3 truck; -1 a timetable bus):
    /// which lanes it may take, see [`Lane::allows`].
    pub veh_type: i32,
    pub lane: usize,
    pub s: f32,
    pub speed: f32,
    pub max_speed_kmh: f32,
    pub accel: f32,
    pub decel: f32,
    pub length: f32,
    pub rng: u64,
    pub blinker: i32,
    pub braking: bool,
    /// Distance travelled, for wheel animation.
    pub odometer: f32,
    /// Lane chosen for after the current one.
    pub planned_next: Option<usize>,
    /// The lanes after `planned_next`, decided `PLAN_AHEAD` metres in advance.
    pub ahead: Vec<usize>,
    /// During a lane change: the way on from the lane the car is moving over to, chosen
    /// when the change begins, so that the steering and the speed for the bends already
    /// know it.
    pub change_plan: Vec<usize>,
    /// The lane the car came from (the way behind it, for the rear axle).
    pub prev_lane: Option<usize>,
    /// Seconds spent waiting to enter a crossing (deadlock breaker).
    pub yield_time: f32,
    /// Fixed lane sequence (timetable track); empty = wander randomly.
    pub route: Vec<usize>,
    /// Position in `route` of the current lane.
    pub route_index: usize,
    /// Lane change in progress.
    pub change: Option<LaneChange>,
    /// Seconds until the next lane change may start.
    pub change_cooldown: f32,
    /// Lateral offset from the lane (m, positive = right): bus bays, swerving round a
    /// parked car. It moves towards `lateral_target` along an S-curve over the distance
    /// driven (`lateral_ramp`), so that the car is straight again when it gets there and
    /// nothing moves while it stands.
    pub lateral: f32,
    pub lateral_target: f32,
    /// The S-curve in progress: (from, to, odometer at its start, its length in m).
    pub lateral_ramp: (f32, f32, f32, f32),
    /// A turn the car has taken a turn lane for (1 left, 2 right): the way out of the
    /// junction is chosen to match.
    pub turn_wish: i32,
    /// Indicator the driver sets for a manoeuvre of its own (pulling away from a stop),
    /// held while `signal_time` runs.
    pub signal: i32,
    pub signal_time: f32,
    /// Sideways acceleration the driver accepts in a bend (m/s²): the speed through a
    /// curve of radius r is at most sqrt(this × r).
    pub lat_accel: f32,
    /// The driver (`TPathInfo`'s rowdy_factor & co): how fast they like to go relative to
    /// the limit, the time gap they keep to the car ahead (s), the distance they stop
    /// behind it (m), the gap in the cross traffic they accept at a junction (s) and how
    /// long they take to move off when the way clears (s). `accel` is how hard they pull
    /// away and `decel` how hard they like to brake.
    pub desire: f32,
    pub headway: f32,
    pub min_gap: f32,
    pub accept_gap: f32,
    pub reaction: f32,
    /// From the vehicle's origin (the lane position `s`) to its front and rear bumper (m).
    pub front: f32,
    pub rear: f32,
    /// Standing still and held there (by a car ahead, a light, a stop): moving off again
    /// starts after `reaction`; `start_timer` counts it down.
    pub held: bool,
    pub start_timer: f32,
    /// Acceleration of the last step (m/s²).
    pub acc: f32,
    /// At most this much acceleration for now (m/s²): edging out round something standing
    /// close ahead.
    pub accel_cap: Option<f32>,
}

/// What a car keeps its distance to: the gap from its front bumper to the thing (m), how
/// fast that is moving along the car's way (m/s) and how it is speeding up (m/s²).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Lead {
    pub gap: f32,
    pub speed: f32,
    pub acc: f32,
}

impl Lead {
    /// The nearer (more constraining) of two.
    pub fn min(a: Option<Lead>, b: Option<Lead>) -> Option<Lead> {
        match (a, b) {
            (Some(x), Some(y)) => Some(if y.gap < x.gap { y } else { x }),
            (x, None) => x,
            (None, y) => y,
        }
    }
}

/// Seconds a vehicle needs to cover `dist` metres from speed `v`, speeding up at `a` to at
/// most `v_max` (the earliest it can be there when nothing holds it back).
pub fn arrival_time(dist: f32, v: f32, a: f32, v_max: f32) -> f32 {
    if dist <= 0.0 {
        return 0.0;
    }
    let (v, a) = (v.max(0.0), a.max(0.05));
    let v_max = v_max.max(v).max(0.1);
    // speeding up to v_max takes (v_max - v) / a seconds and that much road
    let t_up = (v_max - v) / a;
    let d_up = (v + v_max) * 0.5 * t_up;
    if dist <= d_up {
        (-v + (v * v + 2.0 * a * dist).sqrt()) / a
    } else {
        t_up + (dist - d_up) / v_max
    }
}

/// How far along its S-curve (smoothstep, 0..1) a car moving back from the lane `side`
/// metres to the left has to be before its middle is `clear` metres from that lane's
/// middle (out of the way of the traffic there).
pub fn ramp_progress_for(side: f32, clear: f32) -> f32 {
    if clear <= 0.0 {
        return 0.0;
    }
    if side <= clear {
        return 1.0;
    }
    // the offset left is side × (1 − smoothstep(t)); it is clear once smoothstep(t) ≥ clear / side
    let want = clear / side;
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..20 {
        let mid = 0.5 * (lo + hi);
        if smooth01(mid) < want {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

/// Hardest braking of an AI driver (m/s²): an emergency stop.
pub const MAX_BRAKE: f32 = 8.0;
/// Gap a car leaves before a stop line or a stop point (m).
const STOP_LINE_GAP: f32 = 0.6;

/// A lane change: the car moves over from its lane to `to` along `length` metres of road
/// (by distance, not by time: a car that has to stop halfway stands still, and so does its
/// way).
#[derive(Debug, Clone, Copy)]
pub struct LaneChange {
    pub to: usize,
    /// Progress 0..1.
    pub t: f32,
    pub length: f32,
    /// Position along `to`.
    pub s_to: f32,
    /// 1 left, 2 right.
    pub dir: i32,
    /// Seconds of indicating left before the car starts to move over.
    pub wait: f32,
    /// Pulling out from behind something standing: the car moves off sideways from a
    /// standstill, and what it leaves behind in its lane no longer holds it.
    pub bypass: bool,
}

/// Smoothstep 0..1.
fn smooth01(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A short sequence of lanes the way runs along, with the lane the car is on at `cur`.
struct LaneSeq {
    lanes: [usize; PLAN_LANES + 2],
    n: usize,
    cur: usize,
}

impl LaneSeq {
    /// Gap from the end of `lanes[i]` to the start of `lanes[i + 1]`, if they are linked
    /// (a gap of a few metres means they are not one road, and nothing is bent).
    fn gap(&self, net: &Network, i: usize) -> Option<DVec3> {
        let (a, b) = (&net.lanes[self.lanes[i]], &net.lanes[self.lanes[i + 1]]);
        let g = b.start() - a.end();
        (g.length() < 3.0).then_some(g)
    }

    /// Point and heading `u` metres into `lanes[i]` (beyond the ends of the first and last
    /// lane: straight on), with the joints on both sides smoothed.
    fn at(&self, net: &Network, i: usize, u: f32) -> (DVec3, f32) {
        let l = &net.lanes[self.lanes[i]];
        let (mut p, h) = l.at_ext(u);
        let w = |x: f32| 1.0 - smooth01(x / JOINT_BLEND);
        if i > 0 {
            if let Some(g) = self.gap(net, i - 1) {
                p -= g * (0.5 * w(u.max(0.0))) as f64;
            }
        }
        if i + 1 < self.n {
            if let Some(g) = self.gap(net, i) {
                p += g * (0.5 * w((l.length() - u).max(0.0))) as f64;
            }
        }
        (p, h)
    }

    /// Lane index in `lanes` and distance along it, `d` metres from distance `s` of the
    /// current lane.
    fn locate(&self, net: &Network, s: f32, d: f32) -> (usize, f32) {
        let mut i = self.cur;
        let mut u = s + d;
        while u < 0.0 && i > 0 {
            i -= 1;
            u += net.lanes[self.lanes[i]].length();
        }
        while i + 1 < self.n && u > net.lanes[self.lanes[i]].length() {
            u -= net.lanes[self.lanes[i]].length();
            i += 1;
        }
        (i, u)
    }

    /// Point and heading `d` metres from distance `s` of the current lane.
    fn point(&self, net: &Network, s: f32, d: f32) -> (DVec3, f32) {
        let (i, u) = self.locate(net, s, d);
        self.at(net, i, u)
    }
}

impl AiState {
    pub fn new(lane: usize, s: f32, seed: u64) -> AiState {
        AiState { traffic_pool: None, veh_type: 0, lane, s, speed: 0.0, max_speed_kmh: 50.0, accel: 1.2, decel: 3.0, length: 5.0, rng: seed | 1, blinker: 0, braking: false, odometer: 0.0, planned_next: None, ahead: Vec::new(), change_plan: Vec::new(), prev_lane: None, yield_time: 0.0, route: Vec::new(), route_index: 0, change: None, change_cooldown: 5.0, lateral: 0.0, lateral_target: 0.0, lateral_ramp: (0.0, 0.0, 0.0, 1.0), turn_wish: 0, signal: 0, signal_time: 0.0, lat_accel: 2.8, desire: 1.0, headway: 1.4, min_gap: 2.0, accept_gap: 4.0, reaction: 0.7, front: 2.5, rear: 2.5, held: false, start_timer: 0.0, acc: 0.0, accel_cap: None }
    }

    fn rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// The lanes ahead, nearest first: `planned_next`, then the rest of the plan.
    pub fn upcoming(&self) -> impl Iterator<Item = usize> + '_ {
        self.planned_next.into_iter().chain(self.ahead.iter().copied())
    }

    /// A random way on from the end of `lane`. Lanes the map closes to this vehicle ([rule]
    /// no_cars, bus, trucks: `Lane::allows`) and lanes whose traffic density is zero are not driven
    /// into - filtering them only at spawn still let cars turn into a pedestrian street or a
    /// depot yard from next door. A car that has taken a turn lane takes the turn.
    fn choose_after(&mut self, net: &Network, lane: usize) -> Option<usize> {
        let l = &net.lanes[lane];
        // (a car of a traffic pool - the trucks of a map that keeps them to its port roads -
        // takes only the ways its pool may go, as it was put on one. A forbidden exit
        // remains forbidden even if every exit is closed; normal end-of-route handling
        // must deal with that, rather than driving into a pedestrian or restricted lane.)
        let open_to = |pooled: bool| -> Vec<usize> {
            l.next
                .iter()
                .copied()
                .filter(|&n| {
                    let nl = &net.lanes[n];
                    let d = match self.traffic_pool.as_ref().filter(|_| pooled) {
                        Some((p, defaults)) => nl.pool_density(defaults, *p),
                        None => nl.density,
                    };
                    nl.allows(self.veh_type) && d > 0.0
                        // Reject the short return only when the graph links both ways.
                        // Keep the graph itself intact: reach and upstream route choices
                        // retain their existing behavior.
                        && (!nl.next.contains(&lane) || nl.continues_from(l, LINK_TOLERANCE))
                })
                .collect()
        };
        let pooled = self.traffic_pool.is_some().then(|| open_to(true));
        let weighted = pooled.is_some();
        let open = pooled.unwrap_or_else(|| open_to(false));
        let mut choices = open;
        // and a way that goes on rather than into the end of the network, where there is
        // the choice (the map's edge is where OMSI takes its cars away; a village like
        // Grundorf had a queue of twenty growing at the end of its one outbound road)
        if l.kind == LaneKind::Street && net.reach.len() == net.lanes.len() {
            let through: Vec<usize> = choices.iter().copied().filter(|&n| net.reach[n] >= DEAD_END).collect();
            if !through.is_empty() {
                choices = through;
            }
        }
        if self.turn_wish != 0 {
            let wished: Vec<usize> = choices.iter().copied().filter(|&n| net.lanes[n].turn == self.turn_wish).collect();
            if !wished.is_empty() {
                choices = wished;
                self.turn_wish = 0;
            }
        }
        if choices.is_empty() {
            None
        } else {
            if let Some((pool, defaults)) = self.traffic_pool.clone().filter(|_| weighted) {
                let total: f32 = choices.iter().map(|&n| net.lanes[n].pool_density(&defaults, pool)).sum();
                let mut pick = (self.rand() >> 32) as f32 / (u32::MAX as f32 + 1.0) * total;
                for &n in &choices {
                    let weight = net.lanes[n].pool_density(&defaults, pool);
                    if pick < weight { return Some(n); }
                    pick -= weight;
                }
                choices.last().copied()
            } else { Some(choices[(self.rand() % choices.len() as u64) as usize]) }
        }
    }

    /// Decide the way on: the lane after the current one and enough lanes after it to
    /// cover `PLAN_AHEAD` metres (a timetable route simply is that plan).
    pub fn plan_next(&mut self, net: &Network) {
        if !self.route.is_empty() {
            // a route step onto a lane beside this one is a lane change, not the way on
            let mut k = self.route_index + 1;
            if self.route.get(k).map(|&b| net.parallel(self.lane, b)).unwrap_or(false) {
                k += 1;
            }
            self.planned_next = self.route.get(k).copied();
            self.ahead = self.route.iter().skip(k + 1).take(PLAN_LANES).copied().collect();
            return;
        }
        if self.planned_next.is_none() {
            self.ahead.clear();
        }
        let planned: f32 = self.upcoming().map(|l| net.lanes[l].length()).sum();
        let rest = net.lanes[self.lane].length() - self.s;
        if self.planned_next.is_some() && (rest + planned >= PLAN_AHEAD || self.ahead.len() >= PLAN_LANES) {
            return;
        }
        let mut plan: Vec<usize> = self.upcoming().collect();
        self.extend_plan(net, self.lane, rest, &mut plan);
        self.planned_next = plan.first().copied();
        self.ahead = plan.into_iter().skip(1).collect();
    }

    /// Choose lanes after `from` (which has `rest` metres left) until `plan` covers
    /// `PLAN_AHEAD` metres.
    fn extend_plan(&mut self, net: &Network, from: usize, rest: f32, plan: &mut Vec<usize>) {
        let mut dist = rest + plan.iter().map(|&l| net.lanes[l].length()).sum::<f32>();
        while dist < PLAN_AHEAD && plan.len() <= PLAN_LANES {
            let last = plan.last().copied().unwrap_or(from);
            match self.choose_after(net, last) {
                Some(n) => {
                    dist += net.lanes[n].length();
                    plan.push(n);
                }
                None => break,
            }
        }
    }

    /// Put the car on a fixed route starting at its first lane.
    pub fn set_route(&mut self, net: &Network, route: Vec<usize>, s: f32) {
        self.route = route;
        self.route_index = 0;
        if let Some(&l) = self.route.first() {
            self.lane = l;
            self.s = s;
        }
        self.prev_lane = None;
        self.plan_next(net);
    }

    /// Begin a lane change onto the neighbour `to` (`dir` 1 left, 2 right): the indicator
    /// goes on at once, the car moves over after a moment.
    pub fn start_change(&mut self, net: &Network, to: usize, dir: i32) {
        let Some(l) = net.lanes.get(self.lane) else { return };
        let Some(t) = net.lanes.get(to) else { return };
        // two to three and a half seconds at the speed the car has now, at least 12 m
        let duration = (3.5 - self.speed * 0.05).clamp(2.0, 3.5);
        let length = (self.speed * duration).max(12.0);
        // (the same share of a lane that starts beside this one, else the point beside the car)
        let s_to = net.beside_s(self.lane, to, self.s.min(l.length()));
        let rest = t.length() - s_to;
        self.change = Some(LaneChange { to, t: 0.0, length, s_to, dir, wait: SIGNAL_BEFORE_CHANGE, bypass: false });
        self.blinker = dir;
        let mut plan = Vec::new();
        if self.route.is_empty() {
            self.extend_plan(net, to, rest, &mut plan);
        } else {
            plan.extend(self.route.iter().skip(self.route_index + 2).take(PLAN_LANES).copied());
        }
        self.change_plan = plan;
    }

    /// How far the car still has to drive to distance `ss` of route lane `ri` (a lane it
    /// changes over from counts as the lane beside it).
    pub fn route_distance(&self, net: &Network, ri: usize, ss: f32) -> f32 {
        let (mut k, mut d) = (self.route_index, -self.s);
        if let Some(c) = self.change {
            if self.route.get(k + 1) == Some(&c.to) {
                k += 1;
                d = -c.s_to;
            }
        }
        while k < ri && k + 1 < self.route.len() {
            if !net.parallel(self.route[k], self.route[k + 1]) {
                d += net.lanes[self.route[k]].length();
            } else {
                // a lane beside that starts earlier is further along at the same place
                d -= net.beside_delta(self.route[k], self.route[k + 1]);
            }
            k += 1;
        }
        d + ss
    }

    /// A timetable route that moves over to the lane beside the current one next: that lane
    /// and the side it lies on (1 left, 2 right). The driver starts the move as soon as the
    /// lane is free (`start_route_change`).
    pub fn route_change_due(&self, net: &Network) -> Option<(usize, i32)> {
        if self.route.is_empty() || self.change.is_some() {
            return None;
        }
        let &b = self.route.get(self.route_index + 1)?;
        if !net.parallel(self.lane, b) {
            return None;
        }
        // a lane beside that begins further on: not before the car is level with its start
        let delta = net.beside_delta(self.lane, b);
        if delta < 0.0 && self.s < -delta + 1.0 {
            return None;
        }
        let (la, lb) = (&net.lanes[self.lane], &net.lanes[b]);
        // (compared where the car is: a lane beside may start well before or after this one)
        let (pa, ha) = la.at(self.s.min(la.length()));
        let pb = lb.at(net.beside_s(self.lane, b, self.s)).0;
        let h = (ha as f64).to_radians();
        let side = (pb - pa).truncate().dot(DVec2::new(h.cos(), -h.sin()));
        Some((b, if side > 0.0 || (side.abs() < 0.5 && lb.offset > la.offset) { 2 } else { 1 }))
    }

    /// Pull out into `to` from behind a standing obstacle: a short, steep move.
    pub fn start_bypass(&mut self, net: &Network, to: usize, dir: i32) {
        self.start_change(net, to, dir);
        if let Some(c) = self.change.as_mut() {
            c.length = 8.0;
            c.bypass = true;
        }
    }

    /// Start the lane change a route asks for: at once (the indicator has been on while the
    /// driver waited for a gap) and quick enough to be done well before a fork's branches part.
    pub fn start_route_change(&mut self, net: &Network, to: usize, dir: i32) {
        self.start_change(net, to, dir);
        let left = (net.lanes[self.lane].length() - self.s).max(0.0);
        if let Some(c) = self.change.as_mut() {
            c.wait = 0.0;
            c.length = c.length.min(left * 0.7).max(6.0);
        }
    }

    /// The lane sequence the car is driving along: where it came from, where it is, the plan.
    fn seq(&self) -> LaneSeq {
        let mut q = LaneSeq { lanes: [0; PLAN_LANES + 2], n: 0, cur: 0 };
        if let Some(p) = self.prev_lane {
            q.lanes[0] = p;
            q.n = 1;
        }
        q.cur = q.n;
        q.lanes[q.n] = self.lane;
        q.n += 1;
        for l in self.upcoming() {
            if q.n == q.lanes.len() {
                break;
            }
            q.lanes[q.n] = l;
            q.n += 1;
        }
        q
    }

    /// The lane sequence of the lane the car is changing to, with the way on it chose.
    fn change_seq(&self, to: usize) -> LaneSeq {
        let mut q = LaneSeq { lanes: [0; PLAN_LANES + 2], n: 1, cur: 0 };
        q.lanes[0] = to;
        for &l in &self.change_plan {
            if q.n == q.lanes.len() {
                break;
            }
            q.lanes[q.n] = l;
            q.n += 1;
        }
        q
    }

    /// The point of the car's way `d` metres ahead of it (behind for negative `d`): along
    /// its lanes with the joints smoothed, over to the new lane during a lane change, and
    /// out to the side where it pulls into a bay or round a parked car. The steering
    /// follows this curve; it has no steps, so neither does the car.
    pub fn way_point(&self, net: &Network, d: f32) -> DVec3 {
        let (mut p, mut h) = self.seq().point(net, self.s, d);
        if let Some(c) = self.change {
            if c.to < net.lanes.len() {
                let (pt, ht) = self.change_seq(c.to).point(net, c.s_to, d);
                // how far the change will have got by the time the car is `d` metres on
                let tau = (d - c.wait * self.speed.max(2.0)) / c.length;
                let k = smooth01(c.t + tau);
                p = p.lerp(pt, k as f64);
                h += wrap_deg(ht - h) * k;
            }
        }
        let lat = self.lateral_at(self.odometer + d);
        if lat.abs() > 1e-3 {
            let hr = (h as f64).to_radians();
            p += DVec3::new(hr.cos(), -hr.sin(), 0.0) * lat as f64;
        }
        p
    }

    /// How fast the car may be going now so that it can take every bend of the next stretch
    /// of its way at `lat_accel`, braking gently (2 m/s²) into the tight ones. Without this
    /// the cars swept round a junction at 40 km/h - well over a g. The bend is the lane's
    /// own curvature or the turn of the way over 6 m, whichever is sharper: lanes are linked
    /// with up to 40° between them, and such a kink is a bend too.
    pub fn curve_speed(&self, net: &Network) -> f32 {
        let mut best = self.curve_speed_on(net, &self.seq(), self.s);
        if let Some(c) = self.change {
            if c.to < net.lanes.len() {
                best = best.min(self.curve_speed_on(net, &self.change_seq(c.to), c.s_to));
            }
        }
        best
    }

    fn curve_speed_on(&self, net: &Network, q: &LaneSeq, s: f32) -> f32 {
        let reach = (self.speed * self.speed / 4.0 + 12.0).min(90.0);
        let mut best = f32::MAX;
        // The samples lie at fixed places of the road (every 2.5 m of the odometer), not at
        // fixed distances ahead of the car: moving with the car, the sample that caught a
        // bend jumped 2.5 m nearer every 2.5 m, the allowed speed fell in steps of 4 m²/s²
        // and the car braked hard, let go and braked hard again with nothing in front of it.
        let first = 2.5 - self.odometer.rem_euclid(2.5);
        let mut d = 0.0f32;
        let mut next = first;
        while d <= reach {
            let (i, u) = q.locate(net, s, d);
            let turn = wrap_deg(q.point(net, s, d + 3.0).1 - q.point(net, s, d - 3.0).1).abs().to_radians() / 6.0;
            let k = net.lanes[q.lanes[i]].curvature_at(u).abs().max(turn);
            if k > 1e-4 {
                let v = (self.lat_accel / k).sqrt().max(2.5);
                // (a bend may begin anywhere up to a sample's spacing before the sample
                // that finds it: the speed is taken from there, or the car met the start of
                // a tight turn half a metre after its profile had allowed 1 m/s more)
                best = best.min((v * v + 2.0 * 2.0 * (d - 2.5).max(0.0)).sqrt());
            }
            d = next;
            next += 2.5;
        }
        best
    }

    /// The sideways offset of the way `d` metres ahead of the car (m, + = right).
    pub fn lateral_ahead(&self, d: f32) -> f32 {
        self.lateral_at(self.odometer + d)
    }

    /// The sideways offset of the way when the odometer reads `x`.
    fn lateral_at(&self, x: f32) -> f32 {
        let (from, to, x0, len) = self.lateral_ramp;
        from + (to - from) * smooth01((x - x0) / len.max(0.1))
    }

    /// The first turn (1 left, 2 right) on the way within `within` metres, with the
    /// distance to the start of the turning lane (0 while on it).
    pub fn turn_ahead(&self, net: &Network, within: f32) -> Option<(i32, f32)> {
        let here = &net.lanes[self.lane];
        if here.turn != 0 {
            return Some((here.turn, 0.0));
        }
        let mut dist = here.length() - self.s;
        for l in self.upcoming() {
            if dist > within {
                break;
            }
            let lane = &net.lanes[l];
            if lane.turn != 0 {
                return Some((lane.turn, dist));
            }
            dist += lane.length();
        }
        None
    }

    /// Set the indicator for what the car is doing or about to do: a lane change, its own
    /// manoeuvre (pulling away from a stop), moving sideways into a bay or out round a
    /// parked car and back, and a turn at the junction ahead - from about four seconds
    /// before it (at least 25 m, at most 60 m) until the turn is done.
    pub fn update_blinker(&mut self, net: &Network) {
        self.blinker = if let Some(c) = self.change {
            c.dir
        } else if self.signal != 0 && self.signal_time > 0.0 {
            self.signal
        } else if (self.lateral_target - self.lateral).abs() > 0.3 {
            if self.lateral_target > self.lateral { 2 } else { 1 }
        } else {
            let within = (self.speed * 4.0).clamp(25.0, 60.0);
            self.turn_ahead(net, within).map(|t| t.0).unwrap_or(0)
        };
    }

    /// Advance along the network. `obstacle` = distance from the car's origin to the rear of
    /// a standing vehicle ahead (m), `stop_at` = distance from its origin to a stop line
    /// (m). False at a dead end (the car is taken off the road).
    pub fn advance(&mut self, net: &Network, dt: f32, obstacle: Option<f32>, stop_at: Option<f32>) -> bool {
        let lead = obstacle.map(|d| Lead { gap: d - self.front, speed: 0.0, acc: 0.0 });
        self.drive(net, dt, lead, stop_at)
    }

    /// The acceleration the driver wants (the Intelligent Driver Model): towards the
    /// desired speed on a free road, and a braking term for what is ahead that is gentle
    /// (`decel`) while there is room and only as hard as it must be when there is not.
    /// `lead` is the vehicle ahead, `stop` the distance from the car's origin to where it
    /// has to stop (a light, a junction it gives way at, a bus stop).
    pub fn desired_accel(&self, net: &Network, lead: Option<Lead>, stop: Option<f32>) -> f32 {
        let Some(lane) = net.lanes.get(self.lane) else { return 0.0 };
        let (a, b) = (self.accel.max(0.1), self.decel.max(0.5));
        let v = self.speed;
        let limit = |l: &Lane| (l.speed_limit_kmh * self.desire).min(self.max_speed_kmh).max(3.0) / 3.6;
        let mut v0 = limit(lane);
        // a lower limit on the lanes ahead (a junction's turning lanes, a 30 zone) is
        // reached at that speed, slowing gently (1.2 m/s²) from where it has to: taken only
        // on entering the lane, the new limit threw the model into a -3 m/s² stop at every
        // junction, with nothing in front of the car
        {
            let reach = (v * v / 2.4 + 10.0).min(120.0);
            let mut d = lane.length() - self.s;
            for l in self.upcoming() {
                if d > reach {
                    break;
                }
                if let Some(nl) = net.lanes.get(l) {
                    let vl = limit(nl);
                    if vl < v0 {
                        v0 = v0.min((vl * vl + 2.0 * 1.2 * d.max(0.0)).sqrt());
                    }
                    d += nl.length();
                }
            }
        }
        let mut acc = a * (1.0 - (v / v0).powi(4)).max(-1.5 * b / a);
        // bends: follow the speed profile `curve_speed` lays out (it assumes 2 m/s² of
        // braking), blending in over the last metre per second above it
        let bend = self.curve_speed(net);
        if bend < v0 && v > bend - 1.0 {
            let track = -2.0 + (bend - v) / 0.6;
            let k = ((v - (bend - 1.0)) / 1.0).clamp(0.0, 1.0);
            acc = acc.min(acc + (track - acc) * k);
        }
        let interaction = |gap: f32, lead_speed: f32, s0: f32, headway: f32| -> f32 {
            let s_star = s0 + (v * headway + v * (v - lead_speed) / (2.0 * (a * b).sqrt())).max(0.0);
            -a * (s_star / gap.max(0.05)).powi(2)
        };
        let mut out = acc;
        if let Some(l) = lead {
            let idm = acc + interaction(l.gap, l.speed.max(0.0), self.min_gap, self.headway);
            // The constant-acceleration heuristic (the "ACC" variant of the model): how hard
            // the driver has to brake if the car ahead keeps doing what it does. A car that
            // has just cut in close ahead but drives almost as fast calls for a firm brake
            // and a gap that opens again over the next seconds, not an emergency stop.
            let (vl, s, al) = (l.speed.max(0.0), l.gap.max(0.05), l.acc.min(a));
            let den = vl * vl - 2.0 * s * al;
            let cah = if vl * (v - vl) <= -2.0 * s * al && den > 1e-3 { v * v * al / den } else { al - (v - vl).max(0.0).powi(2) / (2.0 * s) };
            // (a car beside or just ahead that drives away faster gives the model a gap of
            // nothing: its braking term is bounded so that the heuristic decides)
            let idm = idm.max(-2.0 * MAX_BRAKE);
            let acc_lead = if idm >= cah { idm } else { 0.01 * idm + 0.99 * (cah + b * ((idm - cah) / b).tanh()) };
            out = out.min(acc_lead);
        }
        if let Some(d) = stop {
            // A stop line: far away the model's braking term (without a time gap, the line
            // does not move off), gently; once the constant deceleration that stops the car
            // at the line reaches half the driver's comfortable braking, that deceleration.
            // The model alone braked twice as hard as needed when a light turned yellow 25 m
            // ahead, and then eased off.
            let room = (d - self.front - STOP_LINE_GAP).max(0.02);
            let need = v * v / (2.0 * room);
            let idm = (acc + interaction(d - self.front, 0.0, STOP_LINE_GAP, 0.2)).max(-0.5 * b);
            let constant = -need * 1.05;
            let k = ((need - 0.35 * b) / (0.15 * b)).clamp(0.0, 1.0);
            let a_stop = idm + (constant - idm) * k;
            out = out.min(a_stop);
        }
        out.clamp(-MAX_BRAKE, a)
    }

    /// Advance along the network with the car ahead (`lead`) and a stop point (`stop`,
    /// distance from the car's origin). False at a dead end.
    pub fn drive(&mut self, net: &Network, dt: f32, lead: Option<Lead>, stop: Option<f32>) -> bool {
        self.change_cooldown = (self.change_cooldown - dt).max(0.0);
        self.signal_time = (self.signal_time - dt).max(0.0);
        if self.signal_time <= 0.0 {
            self.signal = 0;
        }
        if net.lanes.get(self.lane).is_none() {
            return false;
        }
        let mut acc = self.desired_accel(net, lead, stop);
        if let Some(cap) = self.accel_cap {
            acc = acc.min(cap);
        }
        // standing: a driver who is held there moves off only after a moment when the way
        // clears (the wave that runs down a queue at a green light)
        if self.speed < 0.05 {
            if acc <= 0.05 {
                // (a hold of a frame or two - a junction that is free and not free by turns
                // as the cars on the ring come and go - winds the reaction back only a
                // little: set back whole every frame, it never ran out, and the car stood at
                // an empty roundabout for minutes, "about to go")
                self.start_timer = if self.held { (self.start_timer + 3.0 * dt).min(self.reaction) } else { self.reaction };
                self.held = true;
                acc = acc.min(0.0);
            } else if self.held {
                self.start_timer -= dt;
                if self.start_timer > 0.0 {
                    acc = 0.0;
                } else {
                    self.held = false;
                }
            }
        } else if self.speed > 0.5 {
            self.held = false;
        }
        let v0 = self.speed;
        let v1 = (v0 + acc * dt).max(0.0);
        self.speed = v1;
        self.acc = if dt > 0.0 { (v1 - v0) / dt } else { 0.0 };
        // brake lights: braking, or holding the car on the brake
        self.braking = acc < -0.6 || (v1 < 0.3 && self.held);
        let ds = (v0 + v1) * 0.5 * dt;
        self.s += ds;
        self.odometer += ds;
        // lane change: signal, glide over to the neighbour, then continue there
        if let Some(mut c) = self.change {
            c.s_to += ds;
            if c.wait > 0.0 {
                c.wait = (c.wait - dt).max(0.0);
            } else {
                c.t += ds / c.length;
            }
            let from_len = net.lanes[self.lane].length();
            let mut to_len = net.lanes.get(c.to).map(|l| l.length()).unwrap_or(0.0);
            // A move that is not over where a lane ends carries on across the joint: the
            // lane beside goes on into the way chosen on it, and this lane into its plan
            // (below). Finished at the joint instead, the car jumped sideways by what was
            // left of the move - so a lane change was only started 20 m and more before a
            // lane's end, and on a road of short spline pieces a car stopped behind a parked
            // car just past a joint never got round it. (A timetable vehicle's route moves
            // over well before the end, as before.)
            let carry_on = self.route.is_empty();
            while carry_on && c.t < 1.0 && c.s_to >= to_len && !self.change_plan.is_empty() {
                c.s_to -= to_len;
                c.to = self.change_plan.remove(0);
                to_len = net.lanes.get(c.to).map(|l| l.length()).unwrap_or(0.0);
            }
            let from_ends = self.s >= from_len && (self.planned_next.is_none() || !carry_on);
            if c.t >= 1.0 || from_ends || c.s_to >= to_len {
                // arrived on the new lane: it has no joint behind the car, and the way on
                // is planned afresh from there
                if self.route.get(self.route_index + 1) == Some(&c.to) {
                    self.route_index += 1;
                }
                self.lane = c.to;
                self.s = c.s_to.min(to_len);
                self.change = None;
                self.change_cooldown = 6.0;
                self.prev_lane = None;
                let plan = std::mem::take(&mut self.change_plan);
                self.planned_next = plan.first().copied();
                self.ahead = plan.into_iter().skip(1).collect();
            } else {
                self.change = Some(c);
            }
        }
        // bay offset: a new target starts an S-curve from where the car is now, eight metres
        // long for every metre sideways (at least 8, at most 30)
        if (self.lateral_target - self.lateral_ramp.1).abs() > 1e-3 {
            let from = self.lateral_at(self.odometer - ds);
            let len = ((self.lateral_target - from).abs() * 8.0).clamp(8.0, 30.0);
            self.lateral_ramp = (from, self.lateral_target, self.odometer - ds, len);
        }
        self.lateral = self.lateral_at(self.odometer);
        if self.planned_next.is_none() {
            self.plan_next(net);
        }
        while (self.change.is_none() || self.route.is_empty()) && self.s >= net.lanes[self.lane].length() {
            let l = &net.lanes[self.lane];
            let Some(next) = self.planned_next else {
                // the end of a flight path: the aircraft flies on straight (the way runs on
                // along the end tangent) until the traffic takes it away out of sight
                if l.kind == LaneKind::Air {
                    break;
                }
                return false; // dead end: despawn
            };
            self.s -= l.length();
            self.prev_lane = Some(self.lane);
            self.lane = next;
            if self.route.is_empty() {
                self.planned_next = if self.ahead.is_empty() { None } else { Some(self.ahead.remove(0)) };
            } else {
                // the plan may have stepped over a lane-change entry of the route
                let from = self.route_index + 1;
                self.route_index = (from..(from + 3).min(self.route.len())).find(|&k| self.route[k] == next).unwrap_or(from);
            }
            self.plan_next(net);
        }
        // keep the plan PLAN_AHEAD metres long as the car eats into it (a route's plan
        // only moves when the car enters its next lane)
        if self.route.is_empty() {
            self.plan_next(net);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn platform_side_selects_the_correct_visit_on_a_two_way_route() {
        let out = LaneBuilder::polyline(
            vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(100.0, 0.0, 0.0)],
            LaneKind::Street,
            3.0,
        );
        let back = LaneBuilder::polyline(
            vec![DVec3::new(100.0, 6.0, 0.0), DVec3::new(0.0, 6.0, 0.0)],
            LaneKind::Street,
            3.0,
        );
        let mut net = Network {
            lanes: vec![out, back],
            ..Default::default()
        };
        // North of both lanes: right of the westbound lane, left of the eastbound.
        let stop = DVec3::new(50.0, 8.0, 0.0);
        let at = |net: &Network, side| {
            net.project_stop_on_route_side(&[0, 1], stop, Some(25.0), 0, side)
                .unwrap()
                .0
        };
        assert_eq!(at(&net, 0), 1);
        assert_eq!(at(&net, 1), 0);
        assert_eq!(at(&net, 2), 1);
        net.left_hand = true;
        assert_eq!(at(&net, 0), 0);
        assert_eq!(at(&net, 1), 1);
        assert_eq!(at(&net, 2), 1);
        assert_eq!(
            net.project_stop_on_route_side(&[0, 1], stop, Some(25.0), 2, 1)
                .unwrap()
                .0,
            1
        );
        // A central platform is left of both directions: choose the nearest lane.
        let central = DVec3::new(50.0, 2.0, 0.0);
        net.left_hand = false;
        for side in [1, 2] {
            assert_eq!(
                net.project_stop_on_route_side(&[0, 1], central, Some(25.0), 0, side)
                    .unwrap()
                    .0,
                0
            );
        }
    }

    #[test]
    fn platform_side_uses_the_local_tangent_on_a_curve() {
        let lane = LaneBuilder::arc(DVec3::ZERO, 0.0, 30.0, 40.0, 0.0, LaneKind::Street, 3.0);
        let (point, heading) = lane.at(15.0);
        let h = (heading as f64).to_radians();
        let right = DVec3::new(h.cos(), -h.sin(), 0.0);
        let net = Network {
            lanes: vec![lane],
            ..Default::default()
        };
        let (_, s, lat) = net
            .project_stop_on_route_side(&[0], point - right * 4.0, Some(25.0), 0, 1)
            .unwrap();
        assert!((s - 15.0).abs() < 1.0);
        assert!((lat + 4.0).abs() < 0.2);
    }

    #[test]
    fn a_stop_is_matched_to_the_lane_it_stands_beside() {
        // out along y = 0 (east), back along y = 6 (west); the stop stands north of the
        // way back: on its right, across the road from the way out
        let out = LaneBuilder::polyline(vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(100.0, 0.0, 0.0)], LaneKind::Street, 3.0);
        let back = LaneBuilder::polyline(vec![DVec3::new(100.0, 6.0, 0.0), DVec3::new(0.0, 6.0, 0.0)], LaneKind::Street, 3.0);
        let net = Network { lanes: vec![out, back], ..Default::default() };
        let stop = DVec3::new(50.0, 9.0, 0.0);
        // nearer to the way back anyway: matched there
        assert_eq!(net.project_stop_on_route(&[0, 1], stop, Some(25.0), 0).unwrap().0, 1);
        // a stop on the right of the way out, nearer the middle of the road
        let stop2 = DVec3::new(50.0, -2.0, 0.0);
        assert_eq!(net.project_stop_on_route(&[0, 1], stop2, Some(25.0), 0).unwrap().0, 0);
        // the route out, back and out again: a stop on the way out, once the trip is past
        // its first leg, is the one on the second way out
        assert_eq!(net.project_stop_on_route(&[0, 1, 0], stop2, Some(25.0), 1).unwrap().0, 2);
    }

    use super::*;

    #[test]
    fn arc_height_change_and_object_gradients_have_distinct_units() {
        let start = DVec3::new(3.0, 4.0, 5.0);
        let linear = LaneBuilder::arc(start, 20.0, 20.0, 30.0, 0.2, LaneKind::Street, 3.0);
        assert!((linear.points[5].z - 5.1).abs() < 1e-12);
        assert!((linear.end().z - 5.2).abs() < 1e-12);
        let graded = LaneBuilder::arc_with_gradients(start, 20.0, 20.0, 30.0, 0.06, -0.04, LaneKind::Street, 3.0);
        assert!((graded.points[5].z - 5.35).abs() < 1e-12);
        assert!((graded.end().z - 5.2).abs() < 1e-12);
        assert_eq!(linear.headings, graded.headings);
        assert_eq!(linear.curvature, graded.curvature);
        for (a, b) in linear.points.iter().zip(&graded.points) {
            assert_eq!(a.truncate(), b.truncate());
        }
    }

    /// A straight lane of 60 m running north, then a right-hand bend of radius 14 m over
    /// 60°, then straight on again.
    fn junction() -> Network {
        let a = LaneBuilder::arc(DVec3::ZERO, 0.0, 60.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let bend = LaneBuilder::arc(a.end(), 0.0, 14.0 * 60f64.to_radians(), 14.0, 0.0, LaneKind::Street, 3.0);
        let c = LaneBuilder::arc(bend.end(), 60.0, 80.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let mut net = Network { lanes: vec![a, bend, c], ..Default::default() };
        net.link(1.5);
        net
    }

    #[test]
    fn path_rules_open_lanes_by_vehicle_type() {
        let mut l = LaneBuilder::polyline(vec![DVec3::ZERO, DVec3::new(0.0, 50.0, 0.0)], LaneKind::Street, 3.0);
        // no rules: cars and taxis, no AI buses or trucks (Omsi.exe 0x71d714)
        assert!(l.allows(0) && l.allows(1) && !l.allows(2) && !l.allows(3) && l.allows(-1));
        l.rule_trucks = true;
        assert!(l.allows(0) && l.allows(3) && !l.allows(2));
        l.no_cars = true;
        assert!(!l.allows(0) && l.allows(1) && l.allows(3));
        l.rule_trucks = false;
        assert!(!l.allows(0) && !l.allows(1));
        l.rule_bus = true;
        assert!(!l.allows(0) && l.allows(1) && l.allows(2) && !l.allows(3));
    }

    #[test]
    fn no_legal_exit_does_not_fall_back_to_a_forbidden_lane() {
        let mut net = junction();
        let mut car = AiState::new(0, 0.0, 7);
        net.lanes[1].no_cars = true;
        assert_eq!(car.choose_after(&net, 0), None);
        net.lanes[1].no_cars = false;
        net.lanes[1].density = 0.0;
        assert_eq!(car.choose_after(&net, 0), None);
        net.lanes[1].density = 1.0;
        car.veh_type = 3;
        assert_eq!(car.choose_after(&net, 0), None);
        net.lanes[1].rule_trucks = true;
        assert_eq!(car.choose_after(&net, 0), Some(1));
        // Explicit timetable paths retain their original OMSI semantics.
        car.veh_type = -1;
        car.route = vec![0, 1];
        net.lanes[1].density = 0.0;
        car.plan_next(&net);
        assert_eq!(car.planned_next, Some(1));
    }

    #[test]
    fn pool_closed_exits_cannot_fall_back_to_general_traffic() {
        let mut net = junction();
        let mut car = AiState::new(0, 0.0, 7);
        car.traffic_pool = Some((2, vec![1, 1, 1].into()));
        net.lanes[1].group_density = vec![(2, 0.0)];
        assert_eq!(car.choose_after(&net, 0), None);
        net.lanes[1].group_density = vec![(2, 0.5)];
        assert_eq!(car.choose_after(&net, 0), Some(1));
    }

    #[test]
    fn slows_down_for_a_bend() {
        let net = junction();
        assert_eq!(net.lanes[0].next, vec![1]);
        let mut car = AiState::new(0, 0.0, 7);
        car.speed = 13.9;
        car.plan_next(&net);
        assert_eq!(car.upcoming().collect::<Vec<_>>(), vec![1, 2]);
        // the bend allows sqrt(2.8 × 14) ≈ 6.3 m/s; 60 m before it the car may still go fast
        let far = car.curve_speed(&net);
        assert!(far > 13.0, "60 m before the bend: {far}");
        let dt = 1.0 / 30.0;
        let mut entered = None;
        for _ in 0..600 {
            if !car.advance(&net, dt, None, None) {
                break; // the end of the test road
            }
            if car.lane == 1 && entered.is_none() {
                entered = Some(car.speed);
            }
        }
        let v = entered.expect("reached the bend");
        assert!(v < 7.5, "entered the bend at {v} m/s");
    }


    /// A pull-out started a few metres before the car's lane ends (a road of short spline
    /// pieces) carries on across the joint and ends on the lane beside's next piece, without
    /// a jump: it used to be finished at the joint, the car moved sideways in one frame.
    #[test]
    fn a_lane_change_carries_on_across_a_joint() {
        let lane = |x: f64, y0: f64, y1: f64| LaneBuilder::polyline(vec![DVec3::new(x, y0, 0.0), DVec3::new(x, y1, 0.0)], LaneKind::Street, 3.0);
        let mut net = Network { lanes: vec![lane(0.0, 0.0, 20.0), lane(-3.5, 0.0, 20.0), lane(0.0, 20.0, 70.0), lane(-3.5, 20.0, 70.0)], ..Default::default() };
        net.link(1.5);
        net.lanes[0].left = Some(1);
        net.lanes[2].left = Some(3);
        let mut car = AiState::new(0, 16.0, 7);
        car.speed = 3.0;
        car.plan_next(&net);
        assert_eq!(car.planned_next, Some(2));
        car.start_bypass(&net, 1, 1);
        let dt = 1.0 / 30.0;
        let mut at = car.way_point(&net, 0.0);
        for _ in 0..300 {
            assert!(car.drive(&net, dt, None, None));
            let p = car.way_point(&net, 0.0);
            let step = (p - at).length();
            assert!(step < car.speed as f64 * dt as f64 + 0.05, "a jump of {step:.2} m on lane {} s {:.1} change {:?}", car.lane, car.s, car.change);
            at = p;
            if car.change.is_none() {
                break;
            }
        }
        assert!(car.change.is_none(), "the move is over");
        assert_eq!(car.lane, 3, "on the lane beside's next piece");
        assert!((at.x + 3.5).abs() < 0.05, "over in the lane beside: {at:?}");
    }

    #[test]
    fn a_turning_lane_is_signalled_in_advance() {
        let mut net = junction();
        net.lanes[1].turn = 2;
        let mut car = AiState::new(0, 0.0, 7);
        car.speed = 10.0;
        car.plan_next(&net);
        let dt = 1.0 / 30.0;
        let mut first_on = None;
        for _ in 0..900 {
            car.advance(&net, dt, None, None);
            car.update_blinker(&net);
            if car.blinker == 2 && first_on.is_none() {
                first_on = Some((car.lane, net.lanes[0].length() - car.s));
            }
            if car.lane == 2 && car.s > 10.0 {
                assert_eq!(car.blinker, 0, "indicator off after the turn");
                break;
            }
        }
        let (lane, before) = first_on.expect("indicator on");
        assert_eq!(lane, 0);
        assert!(before >= 24.0, "indicator on only {before} m before the turn");
    }

    /// Einm_erzgebirgs.sco's "Main" light: red and yellow 2 s, green 11 s, yellow 3 s,
    /// red for the rest of the 38 s cycle.
    fn erzgebirgs() -> TrafficLightController {
        TrafficLightController::from_program(vec![(vec![(3, 2.0), (6, 11.0), (9, 3.0), (0, 0.0)], None)], Some(38.0), &[], &[])
    }

    #[test]
    fn a_light_shows_every_aspect_for_its_time_at_any_frame_rate() {
        for dt in [1.0 / 144.0, 1.0 / 30.0, 0.1, 0.25] {
            let mut c = erzgebirgs();
            c.start(0.0);
            // (aspect, seconds) as the lamps show it over two cycles
            let mut seen: Vec<(Aspect, f32)> = Vec::new();
            let mut t = 0.0f32;
            while t < 76.0 {
                let a = TrafficLightController::aspect(c.state(0));
                match seen.last_mut() {
                    Some(last) if last.0 == a => last.1 += dt,
                    _ => seen.push((a, dt)),
                }
                c.advance(dt);
                t += dt;
            }
            let order: Vec<Aspect> = seen.iter().map(|s| s.0).collect();
            assert_eq!(&order[..5], &[Aspect::RedYellow, Aspect::Green, Aspect::Yellow, Aspect::Red, Aspect::RedYellow], "dt {dt}");
            let tol = dt + 1e-3;
            assert!((seen[0].1 - 2.0).abs() <= tol, "red-yellow {} at dt {dt}", seen[0].1);
            assert!((seen[1].1 - 11.0).abs() <= tol, "green {} at dt {dt}", seen[1].1);
            assert!((seen[2].1 - 3.0).abs() <= tol, "yellow {} at dt {dt}", seen[2].1);
            assert!((seen[3].1 - 22.0).abs() <= tol, "red {} at dt {dt}", seen[3].1);
        }
    }

    #[test]
    fn stock_state_codes_mean_what_the_lamp_scripts_show() {
        assert_eq!(TrafficLightController::lamps(0), (true, false, false));
        assert_eq!(TrafficLightController::lamps(3), (true, true, false));
        assert_eq!(TrafficLightController::lamps(6), (false, false, true));
        assert_eq!(TrafficLightController::lamps(9), (false, true, false));
        assert_eq!(TrafficLightController::lamps(12), (false, false, false));
        assert!(TrafficLightController::allows_go(6) && !TrafficLightController::allows_go(9) && !TrafficLightController::allows_go(3));
        // the clock starts from the time of day: two crossings with one cycle run in step
        let (mut a, mut b) = (erzgebirgs(), erzgebirgs());
        a.start(8.0 * 3600.0 + 10.0);
        b.start(8.0 * 3600.0 + 10.0);
        assert_eq!(a.time, b.time);
        // 28 810 s = 758 cycles and 6 s: green
        assert!((a.time - 6.0).abs() < 1e-6);
        assert_eq!(a.state(0), 6);
    }

    #[test]
    fn a_level_crossing_waits_for_its_train() {
        // bue_falks_ohe.sco: road green until a train asks at light 0 (stop at 1 s), barrier
        // down while it is still there (stop at 16 s)
        let mut c = TrafficLightController::from_program(
            vec![(vec![(0, 15.0), (6, 4.0), (0, 1.0)], None), (vec![(6, 2.0), (9, 12.0), (0, 5.0), (3, 1.0)], None)],
            Some(22.0),
            &[[0.0, 1.0, 1.0], [0.0, 16.0, 0.0]],
            &[],
        );
        c.start(0.0);
        for _ in 0..600 {
            c.advance(0.1);
        }
        assert!((c.time - 1.0).abs() < 1e-6 && c.held, "holding at {}", c.time);
        assert_eq!(c.state(1), 6, "road green while no train comes");
        c.request[0] = true;
        let mut t = 0.0;
        while t < 20.0 {
            c.advance(0.1);
            t += 0.1;
        }
        assert!((c.time - 16.0).abs() < 1e-6 && c.held, "the train is still there: holding at {}", c.time);
        assert_eq!(c.state(0), 6);
        assert_eq!(c.state(1), 0, "road red while the train passes");
        c.request[0] = false;
        c.advance(1.0);
        assert!((c.time - 17.0).abs() < 1e-3, "{}", c.time);
    }

    #[test]
    fn a_bus_phase_is_skipped_when_no_bus_comes() {
        // Kreuz_Heerstr_Pillnitzer_Reimer.sco: the bus light 3 jumps from 51.5 to 61.5 s
        let mut c = TrafficLightController::from_program(vec![(vec![(0, 52.0), (3, 2.0), (6, 4.0), (9, 3.0), (0, 0.0)], Some(10.0))], Some(64.0), &[], &[[0.0, 51.5, 1.0, 61.5]]);
        c.start(50.0);
        c.advance(2.0);
        assert!((c.time - 62.0).abs() < 1e-3, "jumped: {}", c.time);
        c.start(0.0);
        c.time = 50.0;
        c.request[0] = true;
        c.advance(3.0);
        assert!((c.time - 53.0).abs() < 1e-3);
        assert_eq!(c.state(0), 3, "the bus gets its phase");
    }

    #[test]
    fn a_rest_loop_holds_while_nobody_asks() {
        // win-4.sco (#1722): main green 2-12 s, jumping back from 12 to 2 while no bus
        // asks at the bus light; a bus sends it on through yellow.
        let mut c = TrafficLightController::from_program(
            vec![
                (vec![(3, 2.0), (6, 10.0), (9, 3.0), (0, 7.0), (9, 3.0), (0, 1.0)], None),
                (vec![(0, 16.0), (6, 6.0), (0, 1.0)], None),
            ],
            Some(37.0),
            &[],
            &[[1.0, 12.0, 1.0, 2.0], [1.0, 22.0, 1.0, 2.0], [1.0, 22.0, 0.0, 18.0], [1.0, 2.0, 0.0, 12.0], [1.0, 7.0, 0.0, 12.0]],
        );
        c.time = 3.0;
        for _ in 0..1200 {
            c.advance(0.1);
            assert_eq!(c.state(0), 6, "the main road left its green at {:.1} s with nobody asking", c.time);
        }
        c.request[1] = true;
        let mut saw_bus_green = false;
        for _ in 0..300 {
            c.advance(0.1);
            saw_bus_green |= c.state(1) == 6;
        }
        assert!(saw_bus_green, "a bus asking gets its green");
    }

    #[test]
    fn a_backwards_jump_replays_its_phase_once() {
        // win-wit.sco and similar crossings use a small rewind to extend a green. Taking
        // that jump again on every pass locked the whole program in that stretch.
        let mut c = TrafficLightController::from_program(
            vec![(vec![(0, 2.0), (3, 2.0), (6, 4.0), (9, 2.0), (0, 0.0)], None)],
            Some(12.0),
            &[],
            &[[0.0, 8.0, 0.0, 4.0]],
        );
        c.time = 7.9;
        c.request[0] = true;
        for _ in 0..50 {
            c.advance(0.1);
        }
        assert_eq!(c.state(0), 9, "the clock left the replayed green");
    }

    #[test]
    fn simultaneous_inactive_events_do_not_stall_the_cycle() {
        let mut c = TrafficLightController::from_program(
            vec![(vec![(0, 4.0), (6, 16.0)], None); 2],
            Some(20.0),
            &[[0.0, 4.0, 0.0]],
            &[[1.0, 4.0, 0.0, 12.0]],
        );
        c.start(0.0);
        c.advance(5.0);
        assert_eq!(c.time, 5.0);
        assert_eq!(c.state(0), 6);
        for _ in 0..40 {
            c.advance(1.0);
        }
        assert_eq!(c.time, 5.0, "the clock completes later cycles too");
        assert!(!c.held);
    }

    #[test]
    fn simultaneous_inactive_events_do_not_hide_a_later_active_jump() {
        let mut c = TrafficLightController::from_program(
            vec![(vec![(0, 4.0), (6, 16.0)], None); 3],
            Some(20.0),
            &[[0.0, 4.0, 0.0]],
            &[[1.0, 4.0, 0.0, 9.0], [2.0, 4.0, 0.0, 12.0]],
        );
        c.request[2] = true;
        c.start(0.0);
        c.advance(5.0);
        assert_eq!(c.time, 13.0, "take the jump and consume the remaining second");
        assert!(!c.held);
    }

    #[test]
    fn simultaneous_stops_release_and_are_checked_on_the_next_cycle() {
        let mut c = TrafficLightController::from_program(
            vec![(vec![(0, 4.0), (6, 16.0)], None); 3],
            Some(20.0),
            &[[0.0, 4.0, 0.0], [1.0, 4.0, 0.0], [2.0, 4.0, 0.0]],
            &[],
        );
        c.request[2] = true;
        c.start(0.0);
        c.advance(5.0);
        assert_eq!(c.time, 4.0);
        assert!(c.held);
        c.advance(1.0);
        assert_eq!(c.time, 4.0, "an active stop remains held next frame");
        c.request[2] = false;
        c.advance(1.0);
        assert_eq!(c.time, 5.0);
        assert!(!c.held);
        c.advance(16.0);
        assert_eq!(c.time, 1.0);
        c.request[2] = true;
        c.advance(4.0);
        assert_eq!(c.time, 4.0, "the released stop is evaluated after wrapping");
        assert!(c.held);
    }

    #[test]
    fn many_simultaneous_inactive_events_consume_the_frame_time() {
        let mut c = TrafficLightController::new(vec![vec![(0, 4.0), (6, 16.0)]], 20.0);
        c.stops = vec![
            LightStop {
                light: 0,
                time: 4.0,
                if_request: false,
                jump_to: None,
            };
            33
        ];
        c.start(4.0);
        c.advance(0.25);
        assert_eq!(c.time, 4.25, "the inactive group must not exhaust the event budget");
        assert_eq!(c.state(0), 6);
        assert!(!c.held);
    }

    #[test]
    fn a_car_stops_at_the_line_without_braking_hard() {
        let net = junction();
        let mut car = AiState::new(0, 0.0, 3);
        car.speed = 13.9;
        car.accel = 1.5;
        car.decel = 2.2;
        car.plan_next(&net);
        let dt = 1.0 / 60.0;
        let line = 55.0;
        let mut hardest = 0.0f32;
        for _ in 0..1200 {
            let stop = line - car.s;
            car.drive(&net, dt, None, Some(stop));
            hardest = hardest.min(car.acc);
        }
        let front = car.s + car.front;
        assert!(car.speed < 0.01, "stopped: {}", car.speed);
        assert!(front <= line && front > line - 1.5, "front at {front}, line at {line}");
        assert!(hardest > -3.5, "braked at {hardest} m/s²");
    }

    #[test]
    fn a_follower_keeps_its_distance_when_the_leader_brakes() {
        let net = junction();
        let mut car = AiState::new(0, 0.0, 5);
        car.speed = 12.0;
        car.plan_next(&net);
        let (mut lead_s, mut lead_v) = (30.0f32, 12.0f32);
        let dt = 1.0 / 60.0;
        let mut closest = f32::MAX;
        for k in 0..900 {
            // the leader brakes hard after a second and stays standing
            if k > 60 {
                lead_v = (lead_v - 6.0 * dt).max(0.0);
            }
            lead_s += lead_v * dt;
            let gap = lead_s - 2.5 - (car.s + car.front);
            closest = closest.min(gap);
            car.drive(&net, dt, Some(Lead { gap, speed: lead_v, acc: if k > 60 && lead_v > 0.0 { -6.0 } else { 0.0 } }), None);
        }
        assert!(closest > 0.8, "came within {closest} m");
        assert!(car.speed < 0.05);
    }

    #[test]
    fn right_of_way_between_paths() {
        // a crossing: a from the south going north, b from the east going west, c from the
        // north going south and turning left (east)
        let a = LaneBuilder::arc(DVec3::new(0.0, -10.0, 0.0), 0.0, 20.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let b = LaneBuilder::arc(DVec3::new(10.0, 0.0, 0.0), 270.0, 20.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let mut c = LaneBuilder::arc(DVec3::new(-1.5, 10.0, 0.0), 180.0, 10.0 * std::f64::consts::FRAC_PI_2, -10.0, 0.0, LaneKind::Street, 3.0);
        c.turn = 1;
        let mut net = Network { lanes: vec![a, b, c], ..Default::default() };
        net.link(1.5);
        // equal priority: b comes from a's right
        assert!(net.must_yield(0, 1));
        assert!(!net.must_yield(1, 0));
        // the left turn waits for the oncoming car
        assert!(net.must_yield(2, 0));
        assert!(!net.must_yield(0, 2));
        // a [rule] priority beats the geometry
        net.lanes[0].priority = 192.0;
        net.lanes[1].priority = 64.0;
        assert!(!net.must_yield(0, 1));
        assert!(net.must_yield(1, 0));
    }

    #[test]
    fn right_of_way_on_the_left() {
        // the same crossing on a left-hand-traffic map: b (from a's right) now waits for a
        // (from b's left), and a right turn across the oncoming traffic waits, a left one not
        let a = LaneBuilder::arc(DVec3::new(0.0, -10.0, 0.0), 0.0, 20.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let b = LaneBuilder::arc(DVec3::new(10.0, 0.0, 0.0), 270.0, 20.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let mut c = LaneBuilder::arc(DVec3::new(-1.5, 10.0, 0.0), 180.0, 10.0 * std::f64::consts::FRAC_PI_2, -10.0, 0.0, LaneKind::Street, 3.0);
        c.turn = 2;
        let mut net = Network { lanes: vec![a, b, c], left_hand: true, ..Default::default() };
        net.link(1.5);
        assert!(!net.must_yield(0, 1));
        assert!(net.must_yield(1, 0));
        assert!(net.must_yield(2, 0));
        net.lanes[2].turn = 1;
        assert!(!net.must_yield(2, 0));
        assert_eq!(net.oncoming_sign(), 1.0);
    }

    #[test]
    fn a_shallow_crossing_is_a_long_meeting_place() {
        // one junction object (source 2, same key): a straight lane and two lanes crossing
        // it, one square, one at 20°
        let key = |path: u16| Some(LaneKey { tile: (0, 0), id: 1, path });
        let mut a = LaneBuilder::arc(DVec3::new(0.0, -20.0, 0.0), 0.0, 40.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let mut b = LaneBuilder::arc(DVec3::new(20.0, 0.0, 0.0), 270.0, 40.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let h = 20f64.to_radians();
        let mut c = LaneBuilder::arc(DVec3::new(-20.0 * h.sin(), -20.0 * h.cos(), 0.0), 20.0, 40.0, 0.0, 0.0, LaneKind::Street, 3.0);
        for (l, k) in [(&mut a, 0), (&mut b, 1), (&mut c, 2)] {
            l.source = 2;
            l.key = key(k);
        }
        let mut net = Network { lanes: vec![a, b, c], ..Default::default() };
        net.link(1.5);
        let square = net.crossings[0].iter().find(|x| x.other == 1).expect("square crossing");
        let shallow = net.crossings[0].iter().find(|x| x.other == 2).expect("shallow crossing");
        assert!((square.at - 20.0).abs() < 0.5 && (shallow.at - 20.0).abs() < 0.5);
        // square: the bodies touch within a car's width or so of the point
        assert!(square.before <= 3.0 && square.after <= 3.0, "{square:?}");
        // at 20° the centre lines stay within 2.6 m for 2.6 / sin 20° ≈ 7.6 m either side
        assert!(shallow.before >= 7.0 && shallow.after >= 7.0, "{shallow:?}");
    }

    #[test]
    fn a_driver_with_a_choice_keeps_out_of_a_dead_end() {
        // a lane that forks: one way ends after 50 m, the other runs round a long loop
        let a = LaneBuilder::arc(DVec3::ZERO, 0.0, 30.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let dead = LaneBuilder::arc(DVec3::new(0.0, 30.0, 0.0), 10.0, 50.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let on = LaneBuilder::arc(DVec3::new(0.0, 30.0, 0.0), 350.0, 700.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let mut net = Network { lanes: vec![a, dead, on], ..Default::default() };
        net.link(1.5);
        assert_eq!(net.lanes[0].next.len(), 2);
        assert!(net.reach[1] < DEAD_END && net.reach[0] >= DEAD_END && net.reach[2] >= DEAD_END);
        for seed in 1..40 {
            let mut car = AiState::new(0, 0.0, seed);
            car.plan_next(&net);
            assert_eq!(car.planned_next, Some(2), "seed {seed}");
        }
    }

    #[test]
    fn rejecting_a_short_return_preserves_reach_and_existing_exit_choices() {
        // Filter the erroneous driving choice without removing graph links.
        let a = LaneBuilder::arc(DVec3::ZERO, 0.0, 30.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let finite = LaneBuilder::arc(a.end(), 10.0, 20.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let short = LaneBuilder::arc(finite.end(), 10.0, 0.47, 0.0, 0.0, LaneKind::Street, 3.0);
        let next = LaneBuilder::arc(short.end(), 10.0, 1.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let exit = LaneBuilder::arc(next.end(), 10.0, 280.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let on = LaneBuilder::arc(DVec3::new(0.0, 30.0, 0.0), 350.0, 700.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let mut net = Network { lanes: vec![a, finite, short, next, exit, on], ..Default::default() };
        net.link(1.5);
        assert_eq!(net.lanes[0].next.len(), 2);
        assert!(net.lanes[3].next.contains(&2));
        assert_eq!(net.reach[1], REACH_MAX);
        assert!(net.reach[5] >= DEAD_END);
        let mut at_joint = AiState::new(3, 0.0, 7);
        assert_eq!(at_joint.choose_after(&net, 3), Some(4));
        let mut selected = [0; 2];
        for seed in 1..128 {
            let mut car = AiState::new(0, 0.0, seed);
            car.plan_next(&net);
            match car.planned_next {
                Some(1) => selected[0] += 1,
                Some(5) => selected[1] += 1,
                other => panic!("unexpected exit {other:?}"),
            }
        }
        assert!(selected.iter().all(|&n| n > 0), "an open exit disappeared: {selected:?}");
    }

    #[test]
    fn the_way_has_no_steps_at_a_lane_joint() {
        // two lanes that meet 1.2 m apart: the way bends over the joint instead of jumping
        let a = LaneBuilder::arc(DVec3::ZERO, 0.0, 30.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let b = LaneBuilder::arc(DVec3::new(1.2, 30.0, 0.0), 0.0, 30.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let mut net = Network { lanes: vec![a, b], ..Default::default() };
        net.link(1.5);
        let mut car = AiState::new(0, 20.0, 1);
        car.plan_next(&net);
        let mut last = car.way_point(&net, -10.0);
        let mut d = -10.0;
        while d < 25.0 {
            d += 0.25;
            let p = car.way_point(&net, d);
            assert!((p - last).length() < 0.3, "step of {} m at {d}", (p - last).length());
            last = p;
        }
    }

    /// The southbound half of a road through a junction: a lane B (60 m) into the junction's
    /// straight path J1 (10 m), a left turn J2 from the east into the same exit, and the lane
    /// A (50 m) after the junction.
    fn oncoming_road() -> Network {
        let a = LaneBuilder::arc(DVec3::new(-3.0, 150.0, 0.0), 180.0, 50.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let j1 = LaneBuilder::arc(DVec3::new(-3.0, 160.0, 0.0), 180.0, 10.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let j2 = LaneBuilder::arc(DVec3::new(5.0, 158.0, 0.0), 270.0, 8.0 * std::f64::consts::FRAC_PI_2, -8.0, 0.0, LaneKind::Street, 3.0);
        let b = LaneBuilder::arc(DVec3::new(-3.0, 220.0, 0.0), 180.0, 60.0, 0.0, 0.0, LaneKind::Street, 3.0);
        assert!((j2.end() - DVec3::new(-3.0, 150.0, 0.0)).length() < 0.05, "{:?}", j2.end());
        let mut net = Network { lanes: vec![a, j1, j2, b], ..Default::default() };
        net.link(1.5);
        net
    }

    #[test]
    fn upstream_walks_back_through_the_junction() {
        let net = oncoming_road();
        assert_eq!(net.prev[0].len(), 2, "{:?}", net.prev);
        // 20 m into A, looking 100 m back: A itself, both junction paths and the lane before
        let up = net.upstream(0, 20.0, 100.0, 16);
        let find = |l: usize| up.iter().find(|e| e.0 == l).copied();
        assert_eq!(up[0], (0, 0.0, None));
        let (_, off1, into1) = find(1).expect("J1");
        assert!((off1 + 10.0).abs() < 0.01 && into1 == Some(0), "{up:?}");
        let (_, off2, into2) = find(2).expect("J2");
        assert!((off2 + 8.0 * std::f32::consts::FRAC_PI_2).abs() < 0.05 && into2 == Some(0), "{up:?}");
        let (_, off3, into3) = find(3).expect("B");
        assert!((off3 + 70.0).abs() < 0.01 && into3 == Some(1), "{up:?}");
        // a car 15 m into B is at 15 - 70 = -55 in A's distances: 75 m before the place
        // looking only 25 m back, the lanes that end within reach are there, B is not
        let near = net.upstream(0, 20.0, 25.0, 16);
        assert!(near.iter().any(|e| e.0 == 1) && !near.iter().any(|e| e.0 == 3), "{near:?}");
        // and the list is capped
        assert_eq!(net.upstream(0, 20.0, 100.0, 2).len(), 2);
    }

    #[test]
    fn an_acceleration_cap_holds_a_car_back() {
        let net = junction();
        let mut car = AiState::new(0, 0.0, 7);
        car.accel = 2.5;
        car.plan_next(&net);
        car.accel_cap = Some(1.0);
        for _ in 0..30 {
            car.drive(&net, 1.0 / 30.0, None, None);
        }
        assert!(car.speed > 0.9 && car.speed < 1.0 + 1e-3, "{}", car.speed);
        car.accel_cap = None;
        for _ in 0..30 {
            car.drive(&net, 1.0 / 30.0, None, None);
        }
        assert!(car.speed > 3.0, "{}", car.speed);
    }

    #[test]
    fn arrival_times() {
        assert_eq!(arrival_time(-1.0, 5.0, 1.0, 10.0), 0.0);
        // at a steady 10 m/s
        assert!((arrival_time(100.0, 10.0, 1.0, 10.0) - 10.0).abs() < 1e-4);
        // from a standstill at 2 m/s² without reaching the limit: sqrt(2 d / a)
        assert!((arrival_time(50.0, 0.0, 2.0, 20.0) - 50f32.sqrt()).abs() < 1e-3);
        // 5 s up to 10 m/s over 25 m, then 125 m at 10 m/s
        assert!((arrival_time(150.0, 0.0, 2.0, 10.0) - 17.5).abs() < 1e-3);
        // a car faster than the limit keeps its speed
        assert!((arrival_time(60.0, 15.0, 2.0, 10.0) - 4.0).abs() < 1e-3);
    }

    #[test]
    fn moving_back_in_clears_the_oncoming_lane_part_way() {
        // 3.3 m over, 2.05 m needed to the oncoming lane's middle: 62 % of the offset
        let t = ramp_progress_for(3.3, 2.05);
        assert!((smooth01(t) - 2.05 / 3.3).abs() < 1e-4, "{t}");
        assert!(t > 0.5 && t < 0.7, "{t}");
        assert_eq!(ramp_progress_for(3.3, 0.0), 0.0);
        assert_eq!(ramp_progress_for(2.0, 2.5), 1.0);
        assert!(ramp_progress_for(3.3, 1.5) < t);
    }
}

#[cfg(test)]
mod light_tests {
    use super::TrafficLightController;

    #[test]
    fn remaining_green_of_a_pedestrian_light() {
        let at = |mut c: TrafficLightController, t: f64| {
            c.time = t;
            c
        };
        // Einm_Stresow_Obermeier "Main_Ped": green 8 s, then red to the end of a 38 s cycle
        let ped = TrafficLightController::new(vec![vec![(6, 8.0), (0, 0.0)]], 38.0);
        let c = at(ped.clone(), 3.0);
        assert_eq!(c.state(0), 6);
        assert!((c.remaining(0) - 5.0).abs() < 1e-4);
        let c = at(ped, 20.0);
        assert_eq!(c.state(0), 0);
        assert!((c.remaining(0) - 18.0).abs() < 1e-4);
        // red running over the end of the cycle into a red start
        let d = at(TrafficLightController::new(vec![vec![(0, 19.0), (3, 2.0), (6, 11.0), (9, 3.0), (0, 0.0)]], 38.0), 36.0);
        assert!((d.remaining(0) - 21.0).abs() < 1e-4, "{}", d.remaining(0));
        assert!(at(TrafficLightController::new(vec![vec![(6, 5.0)]], 0.0), 1.0).remaining(0).is_infinite());
    }
}

#[cfg(test)]
mod aurora_pool_tests {
    use super::*;
    #[test]
    fn independent_rules_and_default_references() {
        let mut lane = LaneBuilder::arc(DVec3::ZERO, 0.0, 100.0, 0.0, 0.0, LaneKind::Street, 3.0);
        lane.group_density = vec![(0, 1.0), (1, 0.1), (3, 0.001), (5, 0.0)];
        let defaults = [1, 0, 1, 0, 0, 0];
        assert_eq!(lane.pool_density(&defaults, 1), 0.1);
        assert_eq!(lane.pool_density(&defaults, 2), 1.0);
        assert_eq!(lane.pool_density(&defaults, 3), 0.001);
        assert_eq!(lane.pool_density(&defaults, 4), 0.0);
        assert_eq!(lane.pool_density(&defaults, 5), 0.0);
        lane.group_density.push((1, 0.5));
        assert_eq!(lane.pool_density(&defaults, 1), 0.5);
    }
}
