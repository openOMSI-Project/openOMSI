//! The people's data: who they are (`Person`), where they are, what they want this frame,
//! and everything the simulation keeps about them (`PeopleSim`).

use super::*;

/// Where the player looks from, for "nobody appears or vanishes in sight".
#[derive(Debug, Clone, Copy)]
pub struct Eye {
    pub pos: DVec3,
    pub fwd: DVec3,
    /// Cosine of half the diagonal field of view, with a margin.
    pub cos_half: f64,
}

impl Eye {
    /// The eye of a camera at `pos` looking along `fwd` with a vertical field of view of
    /// `fov_deg` degrees and the picture's `aspect` (omsi-app's `humans::Eye::of`).
    pub fn looking(pos: DVec3, fwd: DVec3, fov_deg: f64, aspect: f32) -> Eye {
        let half_v = (fov_deg * 0.5).to_radians();
        let half_diag = (half_v.tan() * (1.0 + (aspect as f64).powi(2)).sqrt()).atan();
        Eye {
            pos,
            fwd: fwd.normalize_or_zero(),
            cos_half: (half_diag + 10f64.to_radians())
                .min(89f64.to_radians())
                .cos(),
        }
    }

    /// A wider picture than the camera's own (a triple screen's side panels): the tangents
    /// of its half-angles, horizontal and vertical.
    pub fn widened(mut self, extent: Option<(f64, f64)>) -> Eye {
        if let Some((tan_x, tan_y)) = extent {
            let half_diag = tan_x.hypot(tan_y).atan();
            self.cos_half = self.cos_half.min((half_diag + 10f64.to_radians()).min(89f64.to_radians()).cos());
        }
        self
    }
}

/// What the passengers tell a bus's scripts in a frame: about its doors, entry by entry and
/// exit by exit, and its places (see [`PeopleSim::write_door_requests`]).
#[derive(Debug, Clone, Default)]
pub struct DoorWants {
    /// `PAX_Entry<i>_Req` / `PAX_Exit<i>_Req`: somebody wants in / out there.
    pub entry_req: Vec<bool>,
    pub exit_req: Vec<bool>,
    /// `PAX_Entry<i>_Busy` / `PAX_Exit<i>_Busy`: somebody stands in that doorway.
    pub entry_busy: Vec<bool>,
    pub exit_busy: Vec<bool>,
    /// The occupancy variables the `[passpos]` places name (#721): whether somebody is on
    /// each.
    pub places: Vec<(String, bool)>,
}

/// A bus as the passengers know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BusId {
    Player,
    Ai(u64),
}

impl BusId {
}

#[derive(Debug, Clone)]
pub enum State {
    /// A pedestrian strolling the pavements (Omsi.exe's task 8, `WalkStreet`).
    Strolling(PedWalk),
    /// Moved by somebody else: an avatar, or one of a LAN host's people.
    Idle,
    /// Task 8 without a path (+0x2f0 = -1): somebody who got off where no pavement is
    /// stands where they are until the player is gone.
    Standing,
    /// A passenger (see `humans_pax`).
    Pax(Box<Pax>),
}

impl State {
    pub fn name(&self) -> &'static str {
        match self {
            State::Strolling(_) => "WalkStreet",
            State::Idle => "Idle",
            State::Standing => "WalkStreet",
            State::Pax(p) => p.task.name(),
        }
    }
    pub fn bus(&self) -> Option<BusId> {
        match self {
            State::Pax(p) => p.inside.or(p.bus),
            _ => None,
        }
    }
}

/// Where a person is: on the ground, or inside a bus at a point of its frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    Ground,
    Bus(BusId, Vec3),
}

pub struct Person {
    pub id: u32,
    pub ty: Arc<HumanType>,
    /// Clothing variant (`HumanType::variant_texture`).
    pub variant: usize,
    /// The renderer's meshes and instances of the person (the view gives them).
    pub meshes: Vec<(usize, usize)>,
    pub position: DVec3,
    pub heading: f64,
    /// Heading in the bus frame while inside one.
    pub lheading: f64,
    pub place: Place,
    /// Velocity in the plane the person walks in (ground or bus floor).
    pub vel: DVec2,
    pub pace: f64,
    pub activity: Activity,
    /// The animation: Omsi.exe's walk phase and joint angles (sub_626ae8).
    pub anim: OmsiAnim,
    pub state: State,
    /// Seconds in the current state.
    pub t_state: f32,
    /// Skinned positions and normals, per mesh.
    pub skins: Vec<(Vec<Vec3>, Vec<Vec3>)>,
    /// The bones the skins were made with, and whether this frame's pose changed them
    /// (somebody standing still keeps the mesh of the frame before: skinning and uploading
    /// thirty waiting people every frame took 2 ms of the frame at a bus station).
    pub skin_bones: Option<[glam::Affine3A; crate::human::SLOTS]>,
    pub pose_changed: bool,
    /// Interior light of the bus the person is in (0 outside).
    pub interior: f32,
    /// The interior light as drawn: it follows `interior` over a moment (stepping through
    /// the door, people lit up and went dark again from one frame to the next).
    pub lit: f32,
    /// The tilt of the floor the person stands on (a bus pitching under the brakes and
    /// leaning in a bend), without its heading: riders are drawn with it. Upright on the
    /// ground; drawn upright in a tilted bus, their feet sank through the floor on one side.
    pub tilt: Mat4,
    /// Age in years: the `.hum`'s `[age]`, else 40 as in OMSI. The
    /// ticket pack's tickets have age ranges (the reduced fare is for 6..13).
    pub age: f32,
    /// Seconds without getting nearer the goal while wanting to move; seconds left
    /// passing through others.
    pub stuck: f32,
    pub ghost: f32,
    /// Seconds a standing vehicle has stood in the way (see the crowd step).
    pub car_wait: f32,
    /// Seconds left going round something in the way off the pavement's line (a lamp post
    /// on the path): the corridor does not pull them back into it meanwhile.
    pub detour: f32,
    /// Which way round (+1 anticlockwise, -1 clockwise) while `detour` lasts: round a corner
    /// the sides' own choices flipped each other and people shuffled at a post.
    pub detour_side: f64,
    /// Why the person is standing, for `OMSI_DEBUG_PAX`.
    pub why: &'static str,
    /// Whether this person has ever been posed (an unposed model is the file's T-pose).
    pub skinned: bool,
    /// Frames since the last pose and where the person stood then (the
    /// feet of a mesh posed a frame ago stay on the floor when it is drawn there).
    pub since_posed: u32,
    pub posed_at: (DVec3, f64),
    /// Ankles of the last pose (model frame), for `OMSI_TRACE_PAX`.
    pub ankles: [Vec3; 2],
    /// A scripted test person (`OMSI_PAX_GALLERY`).
    pub puppet: Option<Puppet>,
    /// LAN play: one of the host's people, drawn where the host says (`mirror_set`).
    pub remote: bool,
}

impl Person {
    pub fn state_name(&self) -> String {
        format!("#{} {} ({})", self.id, self.state.name(), self.why)
    }
    pub fn position(&self) -> DVec3 {
        self.position
    }
    pub fn inside(&self, bus: BusId) -> bool {
        matches!(self.place, Place::Bus(b, _) if b == bus)
    }
    pub fn label(&self) -> String {
        format!(
            "#{} {}",
            self.id,
            self.ty
                .def
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        )
    }
}

/// What a person wants this frame.
pub struct Want {
    pub vel: DVec2,
    /// Heading to turn to when standing (world on the ground, bus frame inside).
    pub face: Option<f64>,
    pub give: f64,
    pub corridor: Option<(DVec2, DVec2, f64)>,
    /// What they do when not walking.
    pub idle: Activity,
}

impl Want {
    pub fn stand(face: Option<f64>, idle: Activity) -> Want {
        Want {
            vel: DVec2::ZERO,
            face,
            give: 0.35,
            corridor: None,
            idle,
        }
    }
}

/// Velocity towards `to`, easing into the stop over the last metre.
/// Somebody off the pavement's path by more than this beyond its corridor (just off a bus,
/// at its door) walks onto it at their own pace before the corridor holds them; within it
/// (a nudge of the crowd) the corridor takes them back.
pub const OFF_PATH: f64 = 0.1;

/// The corridor a walker at `pos` keeps to: none while they are still well off it. Eased
/// into it at up to 0.6 m/s (twice a step) on top of walking there, the people getting off
/// a bus slid sideways from its door to the pavement at twice their pace (#1033, #1079).
pub fn path_corridor(pos: DVec2, corridor: Option<(DVec2, DVec2, f64)>) -> Option<(DVec2, DVec2, f64)> {
    corridor.filter(|&(a, b, dev)| (crowd::clamp_to_corridor(pos, a, b, dev) - pos).length() <= OFF_PATH)
}

pub fn arrive(from: DVec2, to: DVec2, pace: f64) -> DVec2 {
    let d = to - from;
    let dist = d.length();
    if dist < 0.1 {
        return DVec2::ZERO;
    }
    let speed = (pace * dist.min(1.0)).max(if dist > 0.3 { 0.25 } else { 0.0 });
    d / dist * speed
}

/// Seconds after one passenger's greeting or complaint before anybody says another.
pub const CHAT_PAUSE: f64 = 12.0;

pub struct PeopleSim {
    pub types: Vec<Arc<HumanType>>,
    pub people: Vec<Person>,
    pub rng: u64,
    pub next_id: u32,
    /// Seconds since the start.
    pub time: f64,
    pub wall_cells: HashMap<(i32, i32, i32), Vec<(Block, f64, f64)>>,
    pub wall_key: (usize, usize, usize, f64),
    /// Passenger cabins by vehicle files (the front vehicle and its coupled parts).
    pub cabins: HashMap<Vec<PathBuf>, Option<Arc<Cabin>>>,
    pub player_cabin: Option<Arc<Cabin>>,
    pub player_next_stop: Option<RequestStop>,
    /// Which places of each bus are taken.
    pub seats: HashMap<BusId, Vec<bool>>,
    /// The bus stops as Omsi.exe keeps them for the people (see `humans_pax`).
    pub stops: HashMap<i64, PaxStop>,
    /// Kilometres each bus has driven (the odometer the riders read, +0x430).
    pub odometer: HashMap<BusId, f64>,
    /// The `PAX_Entry<n>_Req` / `PAX_Exit<n>_Req` of each bus this frame.
    pub pax_req: HashMap<BusId, (Vec<bool>, Vec<bool>)>,
    /// Its `PAX_Entry<n>_Busy` / `PAX_Exit<n>_Busy`: somebody in that doorway.
    pub pax_busy: HashMap<BusId, (Vec<bool>, Vec<bool>)>,
    /// The occupancy variables its places name, and whether somebody is on each (#721).
    pub pax_places: HashMap<BusId, Vec<(String, bool)>>,
    /// Who is at the player's cash desk (+0x7a8), how often the driver has been asked
    /// again (0x859bc4) and the most of that in this sale (0x859df4).
    pub desk_busy: Option<u32>,
    pub pardons: u8,
    pub pardon_max: u8,
    /// The first populate put people at the stops; later stops fill on foot when in sight.
    pub started: bool,
    pub ped: Option<PedNet>,
    /// What the renderer has to follow (see `bodies`).
    pub bodies: BodyOps,
    /// Stop the player's bus is serving (standing at it).
    pub served_stop: Option<i64>,
    /// Timetable buses at a stop: id → (stop, time the visit began).
    pub ai_visits: HashMap<u64, (i64, f64)>,
    /// When each bus last had a door open (the passengers' clock).
    pub last_door_open: HashMap<BusId, f64>,
    /// The buses coming to or listed at a stop near the player, for somebody running up late
    /// (`runners_tick`): (stop, bus) → where its roll stands.
    pub runner_rolls: HashMap<(i64, BusId), RollEntry>,
    /// Timetable buses to keep at their stop for a few seconds more (for the traffic): the
    /// bus, the stop it must be serving for it (none: any), the seconds.
    pub holds: Vec<(u64, Option<i64>, f32)>,
    /// Door requests for the timetable buses' scripts.
    pub ai_requests: Vec<(u64, DoorWants)>,
    pub tickets: Option<Arc<omsi_content::tickets::TicketPack>>,
    /// Current ticket request at the player's cash desk: (ticket name, value).
    pub request: Option<(String, f32)>,
    /// Payment on the desk: (paid, ticket value), and the change still owed after the ticket.
    pub paid: Option<(f32, f32)>,
    pub change_due: Option<f32>,
    pub money: Option<Money>,
    /// The people inside the player's bus's box last frame (`run_over`): knocked down once
    /// when they come into it, not again every frame they are in it.
    pub under_bus: hashbrown::HashSet<u32>,
    /// A rider pressed the stop button for the next stop (the app fires the vehicle trigger `int_haltewunsch`).
    pub stop_request: bool,
    /// Tickets sold at the cash desk this session and what they were worth.
    pub tickets_sold: u32,
    pub ticket_cash: f32,
    /// The tickets sold since the last `take_sales`: name and value.
    pub sales: Vec<(String, f32)>,
    /// Passengers that reached the cash desk, and those the driver served there.
    pub boarded: u32,
    pub served: u32,
    /// OMSI's rating counters: people who stepped into the player's bus
    /// and of those who had nothing to complain about (comfort = content / stepped in);
    /// tickets asked for and the points for selling them, two for the right change, one
    /// for the wrong (ticket selling = points / 2 × asked).
    pub stepped_in: u32,
    pub content: u32,
    pub ticket_requests: u32,
    pub ticket_points: u32,
    /// `PAX_Entry<i>_Req`: somebody at the kerb wants in through entry `i`.
    pub entry_req: Vec<bool>,
    /// `PAX_Exit<i>_Req`: somebody inside wants out through exit `i`.
    pub exit_req: Vec<bool>,
    /// `PAX_Entry<i>_Busy` / `PAX_Exit<i>_Busy`: somebody stands in that doorway.
    pub entry_busy: Vec<bool>,
    pub exit_busy: Vec<bool>,
    /// Feet put down since the app last collected them (see [`PeopleSim::take_footfalls`]).
    pub footfalls: Vec<Footfall>,
    /// `[trafficdensity_passenger]` factor for the current hour (set by the app).
    pub density: f32,
    /// The clock's time of day in seconds (set by the app): day tickets sell by it.
    pub time_of_day: f64,
    /// How late the player's bus is on its duty (s; set by the app): over five minutes,
    /// boarding passengers may say so.
    pub delay: f64,
    /// The game's folder (the ticket pack's voices are found from it).
    pub root: std::path::PathBuf,
    /// What passengers said since the app last collected it (see `take_voice_lines`).
    pub voice_lines: Vec<VoiceLine>,
    /// When each voice file was last said (seconds of `time`): OMSI keeps such a list
    /// and says a greeting or a complaint only when that
    /// very file has not been heard for 10 s - without it every boarding passenger said
    /// "Hallo" one after the other.
    pub voice_said: HashMap<std::path::PathBuf, f64>,
    /// What passengers may say (the `pax_voices` setting): 0 everything, 1 only the
    /// ticket they ask for, 2 nothing.
    pub voices: u8,
    /// When anybody last greeted or complained (seconds of `time`).
    pub last_chat: f64,
    /// Avatars (the player on foot, other players' walkers): key → person id, and what
    /// the game wants of each this frame.
    pub avatars: HashMap<u32, u32>,
    pub avatar_cmds: HashMap<u32, AvatarCmd>,
    /// Avatars not drawn (the first-person view), by person id.
    pub avatar_hidden: HashMap<u32, bool>,
    /// The buses of the last tick (for the avatars' seats and doors).
    pub last_buses: Vec<BusNow>,
    /// Only avatars: nobody else is put on the map (the passengers are off).
    pub avatar_only: bool,
    /// The player has got up and left the wheel: a standing bus with a door open is left
    /// by its riders as at a terminus (see `ALL_OUT_STOP`).
    pub driver_away: bool,
    /// Per bus stop, Omsi.exe's station targets (0x61cb18, `Schedule::stop_targets`): the
    /// stops the trips go on to, each with the termini of those trips. A person waiting there
    /// wants one of them and boards only a bus showing one of its termini; at a stop no trip
    /// goes on from, anybody takes the first bus (0x61c33c).
    pub stop_targets: Option<HashMap<i64, Vec<(String, HashSet<String>)>>>,
    /// Per bus stop, the destinations of the trips due there soon (`Schedule::
    /// due_destinations`, made anew every game minute): the people turning up draw theirs
    /// from these alone. None: from all of the stop's (no timetable).
    pub due_dests: Option<HashMap<i64, HashSet<String>>>,
    /// Game time `due_dests` was made at.
    pub due_at: f64,
    /// The timetable's name of each stop object (`Schedule::stop_names`), the names the
    /// targets above are made of.
    pub stop_names: Option<HashMap<i64, String>>,
    /// The player's duty (`set_duty`): its trip, the stop of it the duty is due at, and
    /// whether the trip has reached its last stop. None: free drive, and nobody waiting
    /// boards the player's bus.
    pub duty: Option<(Arc<DutyTrip>, usize, bool)>,
    /// Buses whose validator somebody used since the app last looked (`take_stamped`).
    pub stamped: Vec<BusId>,
    /// Pedestrians to keep strolling near the player (scaled by `density`).
    pub pedestrians: usize,
    /// Omsi.exe's people (`[AIMaxCountRandom]`'s second line, the `ai_max_humans` setting):
    /// it makes that many at the start (0x709274) and draws everybody waiting at a stop,
    /// walking the pavements or riding from them - never more; at most half of them walk
    /// the pavements (0x62463c). Here people are made as they are wanted, so they are
    /// counted against it instead.
    pub max_people: usize,
    pub stroll_timer: f32,
    /// Passengers pay the exact fare: no change is ever due.
    pub exact_fare: bool,
    /// How passengers board (`boarding` in the settings): `auto` - pay and take the
    /// ticket by themselves after a moment; `pay` - wait at the desk for the driver to
    /// sell it (and show a pass after `PAY_PATIENCE`); `walk` - no cash desk at all.
    pub boarding: String,
    /// The driver pressed the ticket key (`ticket_give`): sell the requested ticket.
    pub give_ticket: bool,
    /// The driver pressed `change_give`: all the change owed goes on the tray at once.
    pub give_change_all: bool,
    /// The key that sells a ticket, as the HUD names it.
    pub ticket_key: String,
    /// Where the player looks from (set by the app every frame).
    pub eye: Option<Eye>,
    /// Around where people are kept (the player's bus, else the camera).
    pub center: DVec3,
    /// A line for the HUD about something that just happened.
    pub message: Option<String>,
    /// Frames ticked, total and longest tick (ms).
    pub tick_stats: (u32, f64, f64),
    /// Where the time of this tick went (stage, ms since the one before), for the slow
    /// ticks OMSI_PROFILE reports.
    pub tick_stages: Vec<(&'static str, f64)>,
    /// Speed, heading and floor acceleration of each bus last frame (for the riders' balance).
    pub bus_motion: HashMap<BusId, (f64, f64, DVec2)>,
    /// `types` has been cut down to the map's `humans.txt`.
    pub map_humans_done: bool,
    /// `World::tiles_generation` the stops were last checked against.
    pub tiles_seen: u64,
    /// LAN play: this game draws the host's people instead of its own (`lan_world`).
    pub mirror: bool,
    /// LAN play: where the other players are (host): people are kept around them too.
    pub lan_centers: Vec<DVec3>,
    /// A dedicated server: nobody plays at its own place (`center`, the map's camera), so
    /// people are kept around the LAN players alone (`anchors`). Kept around the camera
    /// too, the stops there took the whole pool (`max_people`) for people nobody saw, and
    /// none were left for the stops and pavements around the players.
    pub players_only: bool,
    /// LAN play: the other players' buses this frame (`set_remote_buses`), for their riders
    /// to sit in. Nobody of ours boards them: their doors count as shut.
    pub remote_now: Vec<BusNow>,
    /// The vehicles the player placed and is not driving now (`placed_bus_id`): their
    /// riders stay in them when the player drives another.
    pub placed_now: Vec<BusNow>,
    /// LAN play (client): waiting people our bus could take, to ask the host for, and when
    /// each was last asked for.
    pub claims_out: Vec<u32>,
    pub claimed: HashMap<u32, f64>,
    /// The host's people waiting at a stop (client): (stop, waiting place).
    pub mirror_wait: HashMap<u32, (i64, usize)>,
    /// How the player's bus is driven, for its riders' complaints.
    pub comfort: RideComfort,
    /// LAN play (host): the waiting people handed over to another player's bus, by stop and
    /// that bus. They count among the people of the stop while the bus stands there, as
    /// the people who board a bus of ours keep their stop until it has left: without them
    /// the stop filled up again at once - one more person a frame once its 10..15 s were
    /// up - and the client's bus took them all, one stream of passengers that never ended
    /// (#842, #840, #830).
    pub handed: Vec<(i64, u64)>,
}

/// Resolve each map entry directly, including human packs with nested folders.
/// Keep duplicate entries as spawn weights, but load each definition only once.
pub fn map_human_types(root: &Path, list: &[String]) -> Vec<Arc<HumanType>> {
    // (keyed case-blind: OMSI paths are, and the lists spell one file several ways)
    let mut loaded: HashMap<String, Option<Arc<HumanType>>> = HashMap::new();
    let mut picked = Vec::new();
    for line in list {
        let rel = line.trim().replace('\\', "/");
        // Lists normally include Humans/, but also accept paths relative to that folder.
        let rel = if rel.to_ascii_lowercase().starts_with("humans/") {
            rel
        } else {
            format!("Humans/{rel}")
        };
        let path = omsi_cfg::resolve_path(root, &rel);
        let ty = loaded.entry(path.to_string_lossy().to_lowercase()).or_insert_with(|| {
            match HumanType::load(&path) {
                Ok(t) => Some(Arc::new(t)),
                Err(e) => {
                    log::warn!("map human {}: {e:#}", path.display());
                    None
                }
            }
        });
        if let Some(t) = ty {
            picked.push(t.clone());
        }
    }
    picked
}

// A malformed/imported population setting must not request millions of rendered agents.
// Ordinary OMSI budgets (including the default 200) remain unchanged below this ceiling.
pub const MAX_LOCAL_PEOPLE: usize = 4096;

pub fn bounded_people_limit(configured: usize) -> usize {
    configured.clamp(1, MAX_LOCAL_PEOPLE)
}

/// A foot put down inside a vehicle, for the environment sounds (omsi-app's
/// `ambience::Footfall`).
pub struct Footfall {
    pub position: DVec3,
    pub inside: bool,
    /// On the floor of the player's own bus.
    pub own_bus: bool,
    /// The files (in `Sounds\Passengers\`) of the `[stepsoundpack]` of the path link the
    /// step is on, one picked at random as Omsi.exe does (0x6274c9); none, no sound - a
    /// link without a pack, a bus whose paths.cfg has none, the street.
    pub pack: Option<Arc<[String]>>,
}

/// A line a passenger says: the sample and where they stand.
pub struct VoiceLine {
    pub position: DVec3,
    pub path: std::path::PathBuf,
}

/// How much of its probability a day ticket keeps at a time of day (seconds): rising from
/// nothing at midnight to all of it at 9:00, as the ticket packs describe it, then falling
/// on OMSI's line.
pub fn day_ticket_factor(t: f64) -> f32 {
    let t = t.rem_euclid(86_400.0);
    let rise = t / 32_400.0;
    let fall = 1.0 - (t - 32_400.0) / (88_776.0 - 32_400.0);
    rise.min(fall).clamp(0.0, 1.0) as f32
}

/// A heading in 0..360 degrees.
pub fn wrap_heading(h: f64) -> f64 {
    h.rem_euclid(360.0)
}

/// The angle between two headings (degrees, 0..180).
pub fn angle_between(a: f64, b: f64) -> f64 {
    ((b - a + 540.0).rem_euclid(360.0) - 180.0).abs()
}
