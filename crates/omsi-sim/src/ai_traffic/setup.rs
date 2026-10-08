//! Making the traffic of its parts: the network, the random traffic's types, the light
//! programs (see omsi-app's `traffic::setup` for taking them from the loaded world).

use super::*;

/// The `OMSI_TRACE_AI` file, with its header written.
pub fn open_trace() -> Option<std::io::BufWriter<std::fs::File>> {
    use std::io::Write;
    let path = omsi_cfg::flags::OMSI_TRACE_AI.os()?;
    let mut f = std::io::BufWriter::new(
        std::fs::File::create(path)
            .map_err(|e| log::warn!("OMSI_TRACE_AI: {e}"))
            .ok()?,
    );
    writeln!(f, "t,id,type,x,y,z,heading,pitch,bank,steer,speed,lane,s,blinker,turn,lane_heading,lateral,at_station,acc,yielding,light_hold,passing,front,rear,half_width,scheduled,why,why_gap,phase,lane_z").ok()?;
    Some(f)
}

/// Tiles by their grid position.
pub type Tiles = Vec<(i32, i32)>;

/// The random traffic's vehicle types (with weight, the lanes they run on and their group),
/// its groups with their density curves and the paths' density rules for them.
pub struct RandomTypes {
    pub types: Vec<(Arc<VehicleType>, f32, LaneKind, usize)>,
    pub groups: Vec<omsi_map::ailists::UnschedGroup>,
    pub group_curves: bool,
    pub group_uvg: Vec<Option<usize>>,
    pub uvg_defaults: Vec<i32>,
}

impl TrafficSim {
    /// Make the population deterministic for a LAN room.  The room's session id is
    /// shared by the host and every client, so the same map/time produces the same
    /// initial cars instead of each process inventing a different world.
    pub fn set_lan_seed(&mut self, seed: u64) {
        self.rng = seed | 1;
    }

    /// The traffic made of its parts: the linked network, the random traffic's types,
    /// the light programs (and which crossing object has which), the parked cars and
    /// tiles the network came from, the map's density curve and the options' share of
    /// random traffic and number of timetable vehicles. Nothing in it needs a world or a
    /// GPU (see `Traffic::new`).
    #[allow(clippy::too_many_arguments)]
    pub fn assemble(
        root: &Path,
        net: Network,
        random: RandomTypes,
        lights: Vec<TrafficLightController>,
        controller_of_object: HashMap<i64, usize>,
        (parked_cars, lane_tiles): (Vec<(DVec3, f64)>, Tiles),
        density_curve: Vec<(f32, f32)>,
        (unsched_factor, max_scheduled): (f32, u32),
        target: usize,
    ) -> TrafficSim {
        let RandomTypes { types, groups, group_curves, group_uvg, uvg_defaults } = random;
        let parked: HashMap<usize, Vec<(f32, f32)>> = HashMap::new();
        let light_log = omsi_cfg::flags::OMSI_DEBUG_LIGHTS.var().map(String::from);
        let light_prev = lights.iter().map(|c| vec![-100; c.lights.len()]).collect();
        let lanes = 0..net.lanes.len();
        let street_weight = net.lanes.iter().filter_map(street_lane_weight).sum();
        let mut t = TrafficSim {
            net,
            street_weight,
            parked,
            parked_waiting: Vec::new(),
            lane_tiles: lane_tiles.into_iter().collect(),
            lanes_generation: 0,
            types,
            groups,
            group_curves,
            group_uvg,
            uvg_defaults: Arc::new(uvg_defaults),
            cars: Vec::new(),
            dormant: Vec::new(),
            dormant_time: 0.0,
            rng: 0x9E37_79B9_7F4A_7C15,
            target,
            lights_only: false,
            spawn_radius: 400.0,
            time: 0.0,
            camera: None,
            lights,
            controller_of_object,
            trailer_types: HashMap::new(),
            root: root.to_path_buf(),
            held_at_red: 0,
            stop_wishes: None,
            player_still: 0.0,
            day_time: 0.0,
            time_scale: 1.0,
            weekday: 0,
            night: false,
            daylight: None,
            next_id: 1,
            last_overtaker: None,
            first_turner: None,
            first_red: None,
            first_yield: None,
            first_passer: None,
            density_curve,
            unsched_factor,
            max_scheduled,
            timed_waits_only: false,
            no_timetable_buses: false,
            viewer: None,
            occluders: None,
            walkers: Vec::new(),
            people: Vec::new(),
            initial: true,
            last_dt: 0.0,
            lamp_dt: 0.0,
            trace: open_trace(),
            logged_hard: Default::default(),
            light_log,
            light_prev,
            debug_population: omsi_cfg::flags::OMSI_DEBUG_POPULATION.is_set(),
            framed_spawns: Vec::new(),
            player: None,
            player_priority: false,
            player_blinker: 0,
            player_signal_age: f32::MAX,
            player_signalling: 0.0,
            way_users: Vec::new(),
            others: Vec::new(),
            other_blinkers: HashMap::new(),
            tick_split: [0.0; 3],
            others_still: HashMap::new(),
            others_signal: HashMap::new(),
            geo_prev: Vec::new(),
            index_of: HashMap::new(),
            pull_out_rooms: HashMap::new(),
            removed_scheduled: Vec::new(),
            twinned: Default::default(),
            keep_clear: Vec::new(),
            mirror: false,
            count_near: None,
            lan_centers: Vec::new(),
            lan_eyes: Vec::new(),
            retired: Vec::new(),
        };
        t.sort_parked(parked_cars, lanes);
        t
    }
}
