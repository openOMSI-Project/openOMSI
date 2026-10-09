//! The AI cars and what the simulation keeps about them, and the helpers that put a
//! vehicle on its way.

use super::*;

/// Pulling out onto the other half of the road round something standing in the lane.
#[derive(Debug, Clone, Copy)]
pub struct Passing {
    /// The lane of the oncoming traffic the car moves over onto.
    pub lane: usize,
    /// How far to the left that lane lies (m).
    pub side: f32,
    /// Odometer reading at which the car is past the obstacle and moves back.
    pub until: f32,
    /// Odometer reading at which the car's front would reach the obstacle (had it stayed in
    /// its lane): until then it can still give up and stop behind it.
    pub block: f32,
    /// Length of the S-curve back into the lane (m).
    pub back: f32,
    /// Given up because somebody came the other way: back in and stopping at the odometer
    /// reading `hold`.
    pub aborted: bool,
    pub hold: f32,
    /// Started from a standstill close behind the obstacle: the car edges out
    /// (`PULL_OUT_ACCEL`) until its front is past the obstacle's corner.
    pub creep: bool,
}

impl Passing {
    /// Odometer reading at which the car has moved back far enough to be out of the way of
    /// the oncoming traffic (`half_width` its own half width).
    pub fn clear_at(&self, half_width: f32) -> f32 {
        self.until
            + self.back
                * crate::traffic::ramp_progress_for(self.side, half_width + ONCOMING_ROOM)
    }
}

/// Room an oncoming vehicle needs beside a car (m from the car's side to the middle of the
/// oncoming lane): its half width and a margin.
pub const ONCOMING_ROOM: f32 = 1.15;

/// What makes a new car a timetable bus (`Traffic::create_car`).
pub struct BusSetup {
    /// The trip's lanes, as far as the loaded tiles have them.
    pub route: Vec<usize>,
    pub stops: Vec<super::bus_service::Stop>,
    /// Fleet number and registration (`number`, `ident` string variables).
    pub number: Option<(String, String)>,
    pub hof: Option<Arc<omsi_vehicle::Hof>>,
    pub timetable: super::bus_service::AiTimetable,
}

pub struct AiCar {
    /// Stable id for references from other systems (passengers).
    pub id: u64,
    /// The random seed it was made with and its paint scheme: a car that goes out of range
    /// and comes back is the same car (`DormantCar`).
    pub seed: u64,
    pub scheme: Option<usize>,
    pub state: AiState,
    pub vehicle: VehicleInstance,
    /// The body following the way `state` lays out.
    pub body: AiBody,
    /// Seconds this car has been standing still without a stop of its own: a red light or
    /// a queue is seconds, a jam that never clears grows without bound.
    pub stopped: f32,
    /// The car it follows now (its id), when one is close ahead.
    pub lead_car: Option<u64>,
    /// A car it does not take for its lead until the time given: two that had each other
    /// for their lead (see `Traffic::break_lead_pairs`).
    pub ignore_lead: Option<(u64, f64)>,
    /// Seconds it has crept along below 1 m/s (a claim of one that crawls in a jam of its
    /// own is no car about to come either).
    pub crawl: f32,
    /// The odometer when it last got two metres further, and the seconds since: a car
    /// that creeps against something it never gets past stands as much as one that stops
    /// (`stopped` starts afresh with every centimetre it creeps).
    pub progress: (f32, f32),
    /// A timetable bus: its trip's stops, the doors, the layover, the people aboard (see
    /// `bus_service`). Everything else about it is this car's.
    pub bus: Option<Box<BusService>>,
    /// Half the vehicle's width (m).
    pub half_width: f32,
    /// Waiting at a junction for someone with the right of way this frame.
    pub yielding: bool,
    /// It waited at its junction's line last frame for room on the exit.
    pub exit_wait: bool,
    /// Stopped by a red light this frame.
    pub light_hold: bool,
    /// Junction lanes this car has claimed to drive through (`TPathInfo::reservePaths`).
    pub reserved: Vec<usize>,
    /// The light (controller, lamp) the driver decided to pass on yellow.
    pub amber: Option<(usize, usize)>,
    pub passing: Option<Passing>,
    /// Finished (a dead end, the end of a timetable trip, given up): taken off the road as
    /// soon as nobody can see it.
    pub gone: bool,
    /// Seconds since it was put on the road are fewer than this: its speed was a guess.
    pub fresh: f32,
    /// The car it lets go first at the next merge (by id).
    pub merge_after: Option<u64>,
    /// What holds it (`OMSI_DEBUG_TRAFFIC`, for cars standing for long).
    pub holding: Option<String>,
    /// What held the car back this frame (for OMSI_TRACE_AI): the constraint nearest ahead
    /// ("lead", "light", "yield", "merge", "keep_back", "people", "pull_out", "service",
    /// "end", "" for none) and its distance ahead of the front (m).
    pub why: (&'static str, f32),
    /// Something made it wait this frame: a stop point, or a car or an obstacle close ahead.
    pub held: bool,
    /// The vehicle (by id) whose body stands in this car's way off its lanes this frame
    /// (`Traffic::body_in_way`).
    pub geo_block: Option<u64>,
    /// What it keeps behind (by id) and the gap to it, as of its last step.
    pub lead_info: Option<(u64, f32)>,
    /// What it waited for at its last junction (`OMSI_DEBUG_STUCK` only).
    pub junction_why: String,
    /// Giving way: where it waits (distance from its origin to the line).
    pub wait_at: Option<f32>,
    /// The vehicle (by id) standing half out of the lane that this car is squeezing past.
    pub squeeze: Option<u64>,
    /// How far behind something standing (a bus at its stop, the player's bus) this car
    /// stops, so that it can steer out round it later (m, front bumper to the other's body;
    /// from its own steering, `pull_out_room`).
    pub pass_room: f32,
    /// No new look at passing before this time (a pull-out that did not clear the corner is
    /// not tried again every frame).
    pub pass_retry: f32,
    /// The traffic light it waited for in the last frame: distance from its origin.
    pub light_at: Option<f32>,
    /// A car that was parked at the kerb: seconds it still stands there, indicating, before
    /// it pulls out (see `Traffic::pull_out_parked`).
    pub pull_out: f32,
    /// Parking: the free space it drives into (see `Traffic::park_in`).
    pub park: Option<ParkPlan>,
    /// A rail vehicle: the track it has come along, (odometer, point), oldest first -
    /// where its rear bogie and its coupled cars and sections run (see `rail_behind`).
    pub rail_trail: std::collections::VecDeque<(f64, DVec3)>,
    /// Seconds its body and script took last frame (heavy ones get an AI job of their own).
    pub ai_secs: f32,
    /// A train turned round as a whole (its last car leads now): what a trip's
    /// `[trainreverse]` is compared with (Omsi.exe's vehicle +0x4e1).
    pub consist_reversed: bool,
    /// The vehicle (by id) it stands for this frame: the car ahead, the one it gives way
    /// to at a junction (`stats::WAITS_ON_PLAYER` the player's or a LAN player's). The
    /// waits-for graph of the traffic statistics.
    pub waits_on: Option<u64>,
    /// The vehicle (by id) it gave way to at its junction this frame.
    pub yield_to: Option<u64>,
    /// Chosen to break a ring of cars waiting on each other (`deadlock`): until this time
    /// it goes through its junction whatever the rules say.
    pub deadlock_pass: f32,
    /// When it was last chosen so.
    pub deadlock_tried: f32,
}

/// A free parking space beside a lane that a car means to park in: the space of parked car
/// `key` that drove off (its object comes back when the car is in).
#[derive(Debug, Clone, Copy)]
pub struct ParkPlan {
    pub key: i64,
    pub lane: usize,
    /// The space's middle along the lane and its offset to the right of it (m).
    pub s: f32,
    pub lat: f32,
    /// Moving over into the space.
    pub ramped: bool,
    /// In the space and standing: the parked object takes its place at the next sync.
    pub done: bool,
}

impl AiCar {
    /// A timetable bus (in service or on its way off after its trip).
    pub fn is_bus(&self) -> bool {
        self.bus.is_some()
    }

    /// Bound to rails (a train, a tram).
    pub fn is_rail(&self) -> bool {
        self.body.kind == MotionKind::Rail
    }

    /// Boarding at a stop: the script is told to open the doors (`AI_Scheduled_AtStation`).
    pub fn at_station(&self) -> bool {
        self.bus.as_ref().map(|b| b.at_station()).unwrap_or(false)
    }

    /// The side's doors to open at the stop it is boarding at (`AI_Scheduled_AtStation_Side`).
    pub fn at_station_side(&self) -> f32 {
        self.bus.as_ref().map(|b| b.at_station_side()).unwrap_or(0.0)
    }

    /// Standing at one of its stops (doors open, waiting for the departure, pulling out).
    pub fn at_stop(&self) -> bool {
        self.bus.as_ref().map(|b| b.at_stop()).unwrap_or(false)
    }

    pub fn trip_done(&self) -> bool {
        self.bus.as_ref().map(|b| b.trip_done()).unwrap_or(false)
    }

    pub fn route_open(&self) -> bool {
        self.bus.as_ref().map(|b| b.route_open).unwrap_or(false)
    }

    /// The next stop: (route index, distance along that lane).
    pub fn next_stop(&self) -> Option<(usize, f32)> {
        self.bus.as_ref()?.stops.front().map(|s| (s.ri, s.s))
    }

    /// Seconds it will still stand at its stop.
    pub fn standing_for(&self, day_time: f64) -> f32 {
        self.bus.as_ref().map(|b| b.standing_for(day_time)).unwrap_or(0.0)
    }
}

/// Where a vehicle's body stands, for the checks that go by geometry rather than by lanes:
/// the car it belongs to (a trailer or rear section counts as its own footprint), centre,
/// forward and right unit vectors, half length, half width and speed along its heading.
#[derive(Debug, Clone, Copy)]
pub struct Footprint {
    pub car: usize,
    pub center: DVec2,
    pub fwd: DVec2,
    pub right: DVec2,
    pub half_len: f64,
    pub half_w: f64,
    pub speed: f32,
    /// Height of the vehicle's origin (an aircraft overhead is not in a car's way).
    pub z: f64,
}

/// A car's own footprint, grown forward by `ahead` metres.
pub fn car_foot(c: &AiCar, ahead: f32) -> Footprint {
    let st = &c.state;
    let h = c.vehicle.heading.to_radians();
    let (fwd, right) = (DVec2::new(h.sin(), h.cos()), DVec2::new(h.cos(), -h.sin()));
    let center = c.vehicle.position.truncate() + fwd * ((st.front + ahead - st.rear) * 0.5) as f64;
    Footprint { car: usize::MAX, center, fwd, right, half_len: ((st.front + ahead + st.rear) * 0.5) as f64, half_w: c.half_width as f64, speed: st.speed, z: c.vehicle.position.z }
}

impl Footprint {
    pub fn from_obb(car: usize, b: &crate::collision::Obb, speed: f32) -> Footprint {
        let (sh, ch) = (b.heading.sin(), b.heading.cos());
        Footprint {
            car,
            center: b.center,
            fwd: DVec2::new(sh, ch),
            right: DVec2::new(ch, -sh),
            half_len: b.half.y,
            half_w: b.half.x,
            speed,
            z: b.z0,
        }
    }

    /// The footprint as a collision box (no height range).
    pub fn obb(&self) -> Obb {
        Obb {
            center: self.center,
            half: DVec2::new(self.half_w, self.half_len),
            heading: self.fwd.x.atan2(self.fwd.y),
            z0: f64::MIN,
            z1: f64::MAX,
            velocity: DVec2::ZERO,
            mass: 0.0,
            pole: None,
            id: -1,
        }
    }

    /// Does this footprint overlap `o`, both grown by `margin` (separating axes)?
    pub fn overlaps(&self, o: &Footprint, margin: f64) -> bool {
        let d = o.center - self.center;
        for axis in [self.fwd, self.right, o.fwd, o.right] {
            let extent = |f: &Footprint| {
                (f.fwd.dot(axis)).abs() * (f.half_len + margin)
                    + (f.right.dot(axis)).abs() * (f.half_w + margin)
            };
            if d.dot(axis).abs() > extent(self) + extent(o) {
                return false;
            }
        }
        true
    }
}

/// The player's vehicle as the traffic sees it: centre, heading (deg), half length, half
/// width, speed along the heading (m/s, negative when reversing).
pub type PlayerBox = (DVec3, f64, f32, f32, f32);

/// A random car out of the player's range: still on the map and still driving, but without
/// a body, a script or a picture - a few numbers. It comes back as the same car (type,
/// paint, id) where it has got to when the player comes near, and it only ever leaves the
/// map at the end of the road network. Before, every car out of range was simply taken
/// away and new ones made up around the player: the traffic followed the player about, and
/// a car driven past was never seen again.
pub struct DormantCar {
    pub id: u64,
    pub ty: Arc<VehicleType>,
    pub kind: LaneKind,
    pub lane: usize,
    pub s: f32,
    pub speed: f32,
    pub seed: u64,
    pub scheme: Option<usize>,
    /// Its own dice for the turns it takes.
    pub walk: u64,
}

/// `[boundingbox]` of a vehicle that gives none.
pub const DEFAULT_BOX: [f32; 6] = [2.5, 12.0, 3.0, 0.0, 0.0, 1.5];

/// The bodies of a vehicle and of the parts coupled to it, where they stand.
pub fn vehicle_bodies(v: &VehicleInstance) -> Vec<crate::collision::Obb> {
    let mut out = vec![crate::collision::Obb::from_box(
        v.ty.def.bounding_box.unwrap_or(DEFAULT_BOX),
        v.position,
        v.body_heading(),
    )];
    for t in &v.trailers {
        out.push(crate::collision::Obb::from_box(
            t.ty.def.bounding_box.unwrap_or(DEFAULT_BOX),
            t.position,
            t.body_heading(),
        ));
    }
    out
}

/// A timetable bus's IBIS moves on to its next stop as the driver would press it on: the
/// stock scripts' interior displays, announcements and side displays read `IBIS_busstop`
/// (an index into the depot file's stop list of the route), which nothing moved on an AI
/// bus - its saloon display stood on the first stop for the whole trip. `remaining` is the
/// number of stops still to come.
pub fn ibis_to_next_stop(v: &mut VehicleInstance, remaining: usize) {
    let Some(ri) = v.var("IBIS_RouteIndex").filter(|r| *r >= 0.0) else { return };
    let Some(n) = v.host.hof.as_ref().and_then(|h| h.info_busstop_lists.get(ri as usize)).map(|l| l.len()) else { return };
    if n == 0 || v.var("IBIS_busstop").is_none() {
        return;
    }
    let idx = n.saturating_sub(remaining.max(1)).min(n - 1);
    v.set_var("IBIS_busstop", idx as f32);
}

/// How a vehicle on lanes of `kind` moves.
pub fn motion_kind(kind: LaneKind) -> MotionKind {
    match kind {
        LaneKind::Air => MotionKind::Air,
        LaneKind::Rail => MotionKind::Rail,
        _ => MotionKind::Road,
    }
}

/// How much track an AI rail vehicle keeps behind it (m): a long train's length.
pub const RAIL_TRAIL: f64 = 400.0;

/// Note where an AI rail vehicle is: `odometer` (m) and the point of its way there. A jump
/// (put somewhere else, turned round at a terminus) starts the trail afresh.
pub fn record_rail_trail(trail: &mut std::collections::VecDeque<(f64, DVec3)>, odometer: f64, here: DVec3) {
    if let Some(&(u, p)) = trail.back() {
        if (here - p).truncate().length() > (odometer - u).abs() + 2.0 {
            trail.clear();
        } else if (odometer - u).abs() <= 0.5 {
            return;
        }
    }
    // (backing up takes the trail back with it)
    while trail.back().is_some_and(|b| b.0 > odometer) {
        trail.pop_back();
    }
    trail.push_back((odometer, here));
    while trail.front().is_some_and(|f| odometer - f.0 > RAIL_TRAIL) {
        trail.pop_front();
    }
}

/// The point of an AI rail vehicle's track `d` metres behind its origin: on the trail it
/// came along. (Its way knows only the lane it came off; farther back it runs straight on,
/// and a train's last cars stood beside the track after a pair of points.) Where the trail
/// does not reach - the last half metre, a vehicle just put there - the way.
pub fn rail_behind(trail: &std::collections::VecDeque<(f64, DVec3)>, state: &AiState, net: &Network, d: f64) -> DVec3 {
    let u = state.odometer as f64 - d;
    let newest = trail.back().map_or(f64::MIN, |b| b.0);
    if u >= newest {
        return state.way_point(net, -d as f32);
    }
    point_at(trail, u).unwrap_or_else(|| state.way_point(net, -d as f32))
}

/// The trail's point at travelled distance `u` (between its samples; None beyond its ends).
/// (The same as the player's train's `rail_drive::point_at`.)
pub fn point_at(trail: &std::collections::VecDeque<(f64, DVec3)>, u: f64) -> Option<DVec3> {
    let i = trail.iter().position(|(v, _)| *v >= u)?;
    if i == 0 {
        return (trail[0].0 - u < 0.01).then_some(trail[0].1);
    }
    let (a, b) = (trail[i - 1], trail[i]);
    let t = ((u - a.0) / (b.0 - a.0).max(1e-6)).clamp(0.0, 1.0);
    Some(a.1 + (b.1 - a.1) * t)
}

/// A body for a vehicle that has just been put on the way `state` describes, with the
/// vehicle posed on it.
pub fn place_body(
    net: &Network,
    state: &AiState,
    vehicle: &mut VehicleInstance,
    kind: MotionKind,
) -> AiBody {
    let mut body = AiBody::new(&vehicle.ty.def, kind);
    let ground = vehicle.ground.clone();
    let contact = vehicle.contact.clone();
    body.place(
        &|d| state.way_point(net, d),
        ground
            .as_ref()
            .map(|g| g.as_ref() as &dyn Fn(f64, f64) -> Option<f64>),
        contact.as_deref(),
        state.speed,
    );
    body.apply(vehicle);
    body
}

/// The vehicle's extent from its origin: (to the front bumper, to the rear bumper, half
/// the width) from its `[boundingbox]`.
pub fn extents(ty: &VehicleType, length: f32) -> (f32, f32, f32) {
    let (front, rear, width) = match ty.def.bounding_box {
        Some(bb) if bb[1] > 1.0 => (
            bb[1] * 0.5 + bb[4],
            bb[1] * 0.5 - bb[4],
            (bb[0] * 0.5).max(0.5),
        ),
        // without a `[boundingbox]` the model's own box, as Omsi.exe takes it (0x7b5da4):
        // the Berlin S-Bahn's cars, 18 m long, counted as 12 m ones
        _ => match ty.model_box() {
            Some((lo, hi)) if hi.y - lo.y > 1.0 => (hi.y.max(0.5), (-lo.y).max(0.5), (hi.x.max(-lo.x)).max(0.5)),
            _ => (length * 0.5, length * 0.5, 0.9),
        },
    };
    if crate::vehicle::body_reversed(&ty.def, false) {
        (rear, front, width)
    } else {
        (front, rear, width)
    }
}

/// The driver of a random car: how fast, how close, how patient (see `AiState`).
pub fn personality(state: &mut AiState, seed: u64, heavy: bool) {
    let r = |k: u32| ((seed >> k) & 0xff) as f32 / 255.0;
    state.desire = if heavy {
        0.88 + 0.1 * r(3)
    } else {
        0.9 + 0.22 * r(3)
    };
    state.headway = 1.0 + 0.8 * r(11);
    state.min_gap = 1.6 + 1.4 * r(19);
    state.accel = if heavy {
        0.8 + 0.4 * r(27)
    } else {
        1.3 + 1.0 * r(27)
    };
    state.decel = if heavy { 1.6 } else { 2.0 + 0.8 * r(35) };
    state.accept_gap = 3.0 + 2.5 * r(43);
    state.reaction = 0.4 + 0.8 * r(51);
}

/// Cruising speed of an AI aircraft where its flight path sets no limit (km/h): an
/// airliner on its final approach.
pub const AIRCRAFT_KMH: f32 = 280.0;

impl TrafficSim {
    /// Put `vehicle` (a random car of type `ty`, or a timetable bus with `bus`) on the road
    /// on `lane` at `s` metres into it and return its id (see `Traffic::create_car`, which
    /// makes the vehicle and its picture): its driver, its way, its body. `id` Some = that
    /// id, `speed` Some = at about that speed.
    #[allow(clippy::too_many_arguments)]
    pub fn place_car(
        &mut self,
        mut vehicle: VehicleInstance,
        kind: LaneKind,
        lane: usize,
        s: f32,
        ty: Arc<VehicleType>,
        seed: u64,
        scheme: Option<usize>,
        id: Option<u64>,
        speed: Option<f32>,
        bus: Option<BusSetup>,
    ) -> u64 {
        let mut state = AiState::new(lane, s, seed);
        state.veh_type = if bus.is_some() { -1 } else { ty.def.ai_veh_type };
        if bus.is_none() {
            state.traffic_pool = self.types.iter().find(|t| Arc::ptr_eq(&t.0, &ty))
                .and_then(|t| self.group_uvg[t.3])
                .map(|pool| (pool, self.uvg_defaults.clone()));
        }
        state.plan_next(&self.net);
        // heavy vehicles (trucks, vans) cruise slower, which is what gets them overtaken
        let heavy = ty.def.mass > 6.0 || bus.is_some();
        personality(&mut state, seed, heavy);
        state.max_speed_kmh = if kind == LaneKind::Air {
            AIRCRAFT_KMH
        } else if bus.is_some() {
            // a bus driver keeps to the limit (the town's 50) like the cars round him,
            // with a little more on the arterial roads
            56.0 + (seed % 7) as f32
        } else if heavy {
            // (a truck keeps to the limit like the cars, up to a truck's own 80-90 km/h: it
            // took 38-47 on every road, crawling along 80 km/h roads, #327)
            80.0 + (seed % 10) as f32
        } else if kind == LaneKind::Street && ty.def.mass > 0.0 && ty.def.mass <= 0.3 {
            // a bicycle (stock ones weigh exactly 0.3 t): 15-21 km/h, the `vmax` range their
            // script cuts the drive at (#327)
            15.0 + (seed % 7) as f32
        } else {
            100.0
        };
        // lorries and vans take bends more gently than cars
        state.lat_accel = if kind == LaneKind::Air {
            50.0
        } else if heavy {
            1.6
        } else {
            2.4 + (seed % 7) as f32 * 0.1
        };
        if bus.is_some() {
            // it brakes for its stops the way the town's drivers brake for a light: with
            // 1.5 m/s² of "comfortable" braking the planner braked at half that and crept
            // up to every stop for a hundred metres
            state.decel = 2.1;
            state.accel = state.accel.max(1.0);
            state.min_gap = state.min_gap.max(2.2);
        }
        let (front, rear, half_width) = extents(&ty, if bus.is_some() { 12.0 } else { 4.5 });
        state.front = front;
        state.rear = rear;
        state.length = front + rear;
        if let Some(b) = &bus {
            state.set_route(&self.net, b.route.clone(), s);
        }
        state.speed = (self.net.lanes[lane]
            .speed_limit_kmh
            .min(state.max_speed_kmh)
            / 3.6
            * 0.7)
            .min(state.curve_speed(&self.net));
        if kind == LaneKind::Air {
            state.speed = self.net.lanes[lane]
                .speed_limit_kmh
                .min(state.max_speed_kmh)
                / 3.6;
            state.accel = 0.5;
            state.decel = 0.5;
        }
        // a bus put out at a stop stands there (in the bay, if it has one)
        let at_stop = bus
            .as_ref()
            .and_then(|b| b.stops.first())
            .filter(|st| st.ri == 0 && (st.s - s).abs() < 1.5);
        if let Some(st) = at_stop {
            state.speed = 0.0;
            if st.bay.abs() > 0.01 {
                state.lateral = st.bay;
                state.lateral_target = st.bay;
                state.lateral_ramp = (st.bay, st.bay, 0.0, 1.0);
            }
        }
        let body = place_body(&self.net, &state, &mut vehicle, motion_kind(kind));
        if omsi_cfg::flags::OMSI_DEBUG_TRAFFIC.is_set() {
            let pos = vehicle.position;
            log::info!(
                "spawn {} on lane {lane} s={s:.1} at ({:.1}, {:.1}, {:.1}) heading {:.0}",
                ty.def.path.display(),
                pos.x,
                pos.y,
                pos.z,
                vehicle.heading
            );
        }
        let pass_room = if kind == LaneKind::Street {
            self.pull_out_room(&ty, front, rear, half_width)
        } else {
            0.0
        };
        let id = id.unwrap_or_else(|| {
            let id = self.next_id;
            self.next_id += 1;
            id
        });
        if let Some(v) = speed {
            state.speed = v.min(state.speed.max(v * 0.5));
        }
        self.cars.push(AiCar {
            id,
            state,
            vehicle,
            body,
            stopped: 0.0,
            lead_car: None,
            ignore_lead: None,
            crawl: 0.0,
            progress: (0.0, 0.0),
            bus: bus.map(|b| Box::new(BusService::new(b.stops))),
            half_width,
            yielding: false,
            exit_wait: false,
            light_hold: false,
            reserved: Vec::new(),
            amber: None,
            passing: None,
            gone: false,
            fresh: 1.5,
            merge_after: None,
            holding: None,
            why: ("", 0.0),
            held: false,
            geo_block: None,
            lead_info: None,
            junction_why: String::new(),
            wait_at: None,
            squeeze: None,
            pass_room,
            pass_retry: 0.0,
            light_at: None,
            pull_out: 0.0,
            rail_trail: Default::default(),
            ai_secs: 0.0,
            consist_reversed: false,
            waits_on: None,
            yield_to: None,
            deadlock_pass: f32::MIN,
            deadlock_tried: f32::MIN,
            park: None,
            seed,
            scheme,
        });
        if kind != LaneKind::Air {
            let i = self.cars.len() - 1;
            if let Some(gap) = self.red_ahead(i) {
                let st = &mut self.cars[i].state;
                st.speed = st.speed.min((2.0 * st.decel * (gap - 1.0).max(0.0)).sqrt());
            }
        }
        id
    }
}
