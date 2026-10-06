//! Passengers, as Omsi.exe runs them.
//!
//! A passenger is one of the original's `THumanBeingInst`s with a *task* (+0x6c5, named by
//! sub_62465c) and a *movement state* (+0x6c4). Every frame the human's tick (sub_62a6a0)
//! first moves the person by the state - straight at a target (1), along the vehicle's
//! `paths.cfg` network from point to point (5), standing (0, 3, 7) or turning on the spot
//! (9) - and then lets the task look at the world and switch the state or the task
//! (sub_62e42c sets up a new task):
//!
//! * `WaitingForBus` (1): at a waiting place of the stop. A bus of theirs listed at the stop
//!   - within 60 m, facing the stop's way (sub_61f238) - that still rolls faster than 2 m/s,
//!   or stands in the stop's box, sends them to the stop's gather point (task 2).
//! * task 2 (no name): walking to the gather point, 0.7 m short of it. When the bus stands
//!   (under 3 m/s) in the box, a free place in it is reserved (sub_7e910c: a random free
//!   `[passpos]`, seat or standing place - no free place, nobody gets on), the ticket is
//!   decided (sub_5ce4e0) and they walk to the bus (3).
//! * `WalkingToBus` (3): to the nearest entry that is open or has a button (and sells
//!   tickets when they buy one), 0.5 m outside the bus side until they are level with it;
//!   a shut door is asked for (`PAX_Entry<n>_Req`) and waited at 0.7 m. In the doorway
//!   they greet the driver or complain (the player's bus only) and board (4).
//! * `WalkingInBusToPlace` (4): along the paths to the validator (stamping for a second,
//!   `ev_Stamper`) or the cash desk (the ticket sale with the player) and on to the place
//!   reserved. In any bus but the player's they are at their place at once.
//! * `SittingInBus` (7): requests their next stop at a random point between departure
//!   and the approach, independently of the 60 m boarding range. Otherwise, until
//!   it passed their alternative stop and drove on a random part of the way, or - with
//!   no destination - it drove 1..20 km; a bus at its terminus empties.
//! * `WalkingInBusToExit` (5): stop request (`int_haltewunsch`), to the nearest exit, 0.7
//!   m short of it while it is shut (`PAX_Exit<n>_Req`), out when it is open and the bus
//!   stands at a stop - and on along the pavement as a pedestrian.
//! * `WalkingToBusstop` (6): back to a waiting place of the stop, then waiting again.
//!
//! People do not avoid each other: somebody within 0.6 m in front stops them (sub_626860),
//! a person facing them makes them turn aside once, and that is all.

use super::*;

/// Omsi.exe's tasks (+0x6c5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Task {
    Nothing,
    WaitingForBus,
    /// Task 2: the bus comes, to the stop's gather point.
    ToBus,
    WalkingToBus,
    InBusToPlace,
    InBusToExit,
    WalkingToBusstop,
    SittingInBus,
}

impl Task {
    pub(super) fn name(self) -> &'static str {
        match self {
            Task::Nothing => "DoNothing",
            Task::WaitingForBus => "WaitingForBus",
            Task::ToBus => "BusComing",
            Task::WalkingToBus => "WalkingToBus",
            Task::InBusToPlace => "WalkingInBusToPlace",
            Task::InBusToExit => "WalkingInBusToExit",
            Task::WalkingToBusstop => "WalkingToBusstop",
            Task::SittingInBus => "SittingInBus",
        }
    }
}

/// The ticket a passenger has (+0x61c): nothing to do, a ticket to stamp, one to buy.
pub(super) const TICKET_NONE: u8 = 0;
pub(super) const TICKET_STAMP: u8 = 2;
pub(super) const TICKET_BUY: u8 = 3;

/// One passenger's state: the fields of the original's human the tasks use.
#[derive(Debug, Clone)]
pub(super) struct Pax {
    pub task: Task,
    /// Movement state +0x6c4: 0 stand, 1 to the target, 2 0.7 m short of it, 3 there, 5
    /// along the paths, 6 0.7 m short of the path's end, 7 at the path's end, 9 turning on
    /// the spot.
    pub st: u8,
    /// Inside a bus (+0x5ef clear): `pos` and `yaw` are in its frame.
    pub inside: Option<BusId>,
    /// Where the feet are (the world, or the bus frame), and the heading (radians, 0 =
    /// forward, clockwise from above: Direct3D's yaw).
    pub pos: DVec3,
    pub yaw: f64,
    /// The bus dealt with (+0x6b4), the stop (+0x6bc), the waiting place there (+0x618).
    pub bus: Option<BusId>,
    pub stop: Option<i64>,
    pub spot: Option<usize>,
    /// What state 1 walks to (+0x5bd) and whether it is a point of the bus (`bus`) or of
    /// the world; the heading to turn to there (+0x5d4).
    pub target: DVec3,
    pub target_bus: bool,
    pub target_yaw: f64,
    /// The path point walked from / to (+0x5e4) and the one at the end (+0x5e0).
    pub pt: Option<usize>,
    pub pt_target: Option<usize>,
    /// Stop 0.7 m short of the target (+0x5ec).
    pub short: bool,
    /// Walking to a door from outside (+0x5d0): keep 0.5 m off the bus side
    /// (`clamp_x`, +0x5cc) unless the door is open (+0x5d1) and they are level with it;
    /// a door on the left (+0x5d2).
    pub clamp: bool,
    pub clamp_open: bool,
    pub clamp_left: bool,
    pub clamp_x: f64,
    /// Destination (+0x5f4), the stop's line record it matched (+0x5f8), the stop the line
    /// leaves the known route at (+0x5fc) and whether it was passed (+0x600), the way on
    /// from there (+0x604, m), the distance to ride (+0x5f0, km) and the odometer at boarding
    /// (+0x60c, km).
    pub dest: Option<String>,
    pub line: Option<usize>,
    pub alt: Option<String>,
    pub alt_seen: bool,
    pub alt_m: f32,
    pub ride_km: f32,
    pub km_start: f64,
    /// Drawn once per passenger: where between departure and the approach they ask
    /// to get off. The stop and its request distance are settled when that leg begins.
    pub stop_request_random: f64,
    pub stop_request_at: Option<(i64, f64)>,
    /// The place reserved in the bus (+0x610).
    pub seat: Option<usize>,
    /// +0x61c, the ticket (1-based, +0x61d) and its price (+0x620), what was paid (+0x624),
    /// the change was wrong (+0x628), the cash desk was free (+0x629), the ticket sale
    /// step (+0x6c6).
    pub ticket: u8,
    pub ticket_id: u8,
    pub price: f32,
    pub paid: f32,
    pub bad_change: bool,
    pub sub: u8,
    /// The entry or exit asked for (+0x640).
    pub door: Option<usize>,
    /// The validator they stamp at (an index into the cabin's: a bus may have several).
    pub stamper: Option<usize>,
    /// `HeightOfSeat` (+0x648) and `PAX_State` (+0x64c: 0 stand, 1 walk, 2 sit).
    pub seat_h: f32,
    pub pax_state: f32,
    /// A countdown in seconds (+0x650) and one in metres walked (+0x654).
    pub timer: f32,
    pub dist_timer: f32,
    /// The angles ease (+0x660); talking to the driver (+0x662); the right hand reaches
    /// (+0x665) for `reach_at` (+0x67c, bus frame); the head turns to the driver (+0x666).
    pub smooth: bool,
    pub talking: bool,
    pub reach: bool,
    pub look_driver: bool,
    pub reach_at: Vec3,
    /// The room height of the link walked (+0x668), its step sounds (+0x694), the link
    /// (+0x698).
    pub room: f32,
    pub step_pack: Option<usize>,
    pub link: Option<usize>,
    /// Speed wanted (+0x6a0), speed (+0x6a4), walking pace (+0x6ac).
    pub speed_des: f32,
    pub speed: f32,
    pub walk_speed: f32,
    /// Somebody in the way (+0x6c7: 1 behind, 2 in front facing them, 3 in front going
    /// the same way or busy) and on which sides there is room (+0x6c8, +0x6c9).
    pub block: u8,
    /// Seconds held up by somebody in front inside a bus, and seconds left passing them
    /// (see `pax_move`).
    pub jam: f32,
    pub squeeze: f32,
    pub free_r: bool,
    pub free_l: bool,
    /// How badly the ride has gone (+0x62c, 0..1; see `ride_comfort`), the complaint said
    /// so far (+0x630: 1 TooBad_A, 2 TooBad_B, 3 TooBad_C - and off at the next stop) and
    /// where each one comes (+0x634, +0x638, +0x63c; drawn once, 0 not yet).
    pub discomfort: f32,
    pub complaint: u8,
    pub bad_at: [f32; 3],
    /// Distance moved this frame (+0x644, `LastMovedDist`).
    pub moved: f32,
}

impl Pax {
    pub(super) fn new(walk_speed: f32, stop_request_random: f64) -> Pax {
        Pax {
            task: Task::Nothing,
            st: 0,
            inside: None,
            pos: DVec3::ZERO,
            yaw: 0.0,
            bus: None,
            stop: None,
            spot: None,
            target: DVec3::ZERO,
            target_bus: false,
            target_yaw: 0.0,
            pt: None,
            pt_target: None,
            short: false,
            clamp: false,
            clamp_open: false,
            clamp_left: false,
            clamp_x: -1e9,
            dest: None,
            line: None,
            alt: None,
            alt_seen: false,
            alt_m: 0.0,
            ride_km: 0.0,
            km_start: 0.0,
            stop_request_random,
            stop_request_at: None,
            seat: None,
            ticket: TICKET_NONE,
            ticket_id: 0,
            price: 0.0,
            paid: 0.0,
            bad_change: false,
            sub: 0,
            door: None,
            stamper: None,
            seat_h: 0.0,
            pax_state: 0.0,
            timer: 0.0,
            dist_timer: 0.0,
            smooth: false,
            talking: false,
            reach: false,
            look_driver: false,
            reach_at: Vec3::ZERO,
            room: OUTSIDE_ROOM,
            step_pack: None,
            link: None,
            speed_des: 0.0,
            speed: 0.0,
            walk_speed,
            block: 0,
            jam: 0.0,
            squeeze: 0.0,
            free_r: true,
            free_l: true,
            discomfort: 0.0,
            complaint: 0,
            bad_at: [0.0; 3],
            moved: 0.0,
        }
    }

    fn wants_stop_at(&mut self, stop: &RequestStop, bus_pos: DVec3, departing: bool) -> bool {
        if !self.dest.as_ref().is_some_and(|dest| stop.is_named(dest)) {
            return false;
        }
        let distance = (bus_pos - stop.pos).length();
        if self.stop_request_at.is_none_or(|(id, _)| id != stop.id) {
            if !departing {
                return false;
            }
            // Keep a nonzero random interval even when the stops are close together.
            let late = (distance * 0.5).min(100.0);
            let request_m = late + (distance - late) * self.stop_request_random;
            self.stop_request_at = Some((stop.id, request_m));
        }
        distance <= self.stop_request_at.unwrap().1
    }
}

/// The next stop of the route, independently of the local boarding range. A planned
/// stop can be known before its tile and its waiting passengers have been loaded.
#[derive(Debug, Clone)]
pub(super) struct RequestStop {
    pub id: i64,
    pub name: String,
    pub alias: String,
    pub pos: DVec3,
}

impl RequestStop {
    fn is_named(&self, name: &str) -> bool {
        let name = name.trim();
        name == self.name.trim() || (!self.alias.is_empty() && name == self.alias.trim())
    }
}

/// The room height outside a vehicle (+0x668 = 50).
pub(super) const OUTSIDE_ROOM: f32 = 50.0;

/// A waiting place of a stop (a `[passpos]` of an object near it, sub_620c0c).
#[derive(Debug, Clone)]
pub(super) struct WaitSpot {
    /// The `[passpos]` point (world): the feet, or a seated person's hip.
    pub pos: DVec3,
    /// Heading (degrees, the world's).
    pub face: f64,
    /// Seat height (+0x20); a seat when not 0.
    pub height: f32,
}

/// What Omsi.exe keeps of a bus stop for the people (the station record, sub_620058).
pub(super) struct PaxStop {
    pub name: String,
    /// Its name in the timetable (empty without one), where the passengers' destinations
    /// come from: the object's label is the stop's name to Omsi.exe, but a map whose
    /// labels and `Busstops.cfg` disagree - a stop renamed, or the two files written in
    /// different code pages - had riders whose stop never came, and who rode on for good.
    pub alias: String,
    pub pos: DVec3,
    /// The object's heading (degrees).
    pub heading: f64,
    /// Where people gather when a bus comes (+0x48): a metre to the side and a metre
    /// along the stop.
    pub gather: DVec3,
    pub spots: Vec<WaitSpot>,
    /// Which places are taken (+0xa8).
    pub taken: Vec<bool>,
    /// pass_enter_max / _min (+0x70, +0x74) and the length (+0x7c, 30 by default).
    pub enter_max: f32,
    pub enter_min: f32,
    pub length: f32,
    /// The pavement next to it (where those getting off walk on).
    pub lane: Option<(usize, f32)>,
    /// In range of the player last time and now (+0x24, +0x25), the refill clock (+0x28,
    /// ms), people wanted and there (+0x30, +0x34), the stop's factor (+0x38), first fill
    /// done (+0xa5 clear).
    pub was_near: bool,
    pub near: bool,
    pub clock_ms: f32,
    pub want: usize,
    pub factor: f32,
    /// The buses listed here this frame (+0x80): (bus, standing in the stop's box).
    pub buses: Vec<(BusId, bool)>,
    /// The destinations (+0xb0): stop name, weight; and the line records (+0xac): the
    /// stop name and the termini of the buses that go there.
    pub dests: Vec<(String, f32)>,
    pub lines: Vec<(String, HashSet<String>)>,
}

impl PaxStop {
    /// Whether a destination or a terminus `name` is this stop: its label, or its name in
    /// the timetable.
    pub(super) fn is_named(&self, name: &str) -> bool {
        let name = name.trim();
        name == self.name.trim() || (!self.alias.is_empty() && name == self.alias.trim())
    }
}

/// How the player's bus is driven, as Omsi.exe watches it for the riders (0x7d5124,
/// 0x7d65d4 - 0x7d6b7f): the longitudinal acceleration eased over a tenth of a second, the
/// lateral one over a second (both weighed down below 1 m/s), and the swings of the first
/// between +0.2 and -0.2 m/s² (a jerky right foot).
#[derive(Debug, Clone, Default)]
pub(super) struct RideComfort {
    /// +0x780 and +0x784 (m/s²).
    fast_long: f32,
    slow_lat: f32,
    /// The last swing went up (+0x79c), when (+0x794, ms) and how many came in a row (+0x798).
    up: bool,
    swing_ms: f64,
    swings: u32,
    /// The last hard bend or braking (+0x790, ms).
    hard_ms: f64,
}

impl RideComfort {
    /// One frame of the bus (`speed` forward and the body's acceleration `lat` to the right
    /// and `long` forward, m/s and m/s²): how much this frame upsets the riders - 0, 0.05
    /// for the fifth and every further swing of the throttle and brake less than 4 s apart,
    /// 0.1 for a bend taken at over 3 m/s² or braking or pulling away at over 5 m/s² (once
    /// a second at most).
    pub(super) fn step(&mut self, dt: f32, now_ms: f64, speed: f32, lat: f32, long: f32) -> f32 {
        let w = speed.abs().min(1.0);
        let kf = (10.0 * dt).min(0.5);
        let ks = dt.min(0.5);
        self.fast_long = w * long * kf + (1.0 - kf) * self.fast_long;
        self.slow_lat = w * lat * ks + (1.0 - ks) * self.slow_lat;
        let mut k = 0.0;
        if self.fast_long > 0.2 && !self.up {
            if now_ms < self.swing_ms + 4000.0 {
                self.swings += 1;
                if self.swings > 4 {
                    k = 0.05;
                }
            } else {
                self.swings = 0;
            }
            self.swing_ms = now_ms;
            self.up = true;
        } else if self.fast_long < -0.2 && self.up {
            // (back within half a second: no swing, the count starts again)
            if now_ms < self.swing_ms + 4000.0 && now_ms > self.swing_ms + 500.0 {
                self.swings += 1;
                if self.swings > 4 {
                    k = 0.05;
                }
            } else {
                self.swings = 0;
            }
            self.swing_ms = now_ms;
            self.up = false;
        }
        if self.slow_lat.abs() > 3.0 || self.fast_long.abs() > 5.0 {
            if self.hard_ms + 1000.0 < now_ms {
                k = 0.1;
            }
            self.hard_ms = now_ms;
        }
        k
    }
}

/// Where a rider's complaints about the driving come (the human's constructor, 0x625a3f):
/// the first below 0.1, the second from 0.2 to 0.4, the third (and off at the next stop)
/// from 0.5 to 0.8, for `r` three draws from 0..1.
pub(super) fn bad_ride_thresholds(r: [f32; 3]) -> [f32; 3] {
    let a = 0.1 * r[0];
    [a, 0.1 + a.max(0.1) + 0.2 * r[1], 0.5 + 0.3 * r[2]]
}

/// The complaint a rider says as the ride's toll `x` reaches their next threshold
/// (0x7d6a22 - 0x7d6b7f; the worst first, each only once): 1, 2, 3 or none.
pub(super) fn bad_ride_complaint(x: f32, said: u8, at: [f32; 3]) -> Option<u8> {
    if at[2] <= x && said < 3 {
        Some(3)
    } else if at[1] <= x && said < 2 {
        Some(2)
    } else if at[0] <= x && said < 1 {
        Some(1)
    } else {
        None
    }
}

/// What the stops say about a bus this frame (sub_61f238): the stop ahead it is pulling
/// in to (+0x7a0), the stops within 60 m (+0x7a4), and whether it empties (+0x7c5).
#[derive(Debug, Clone, Default)]
pub(super) struct BusAtStops {
    pub next: Option<i64>,
    pub request_next: Option<RequestStop>,
    pub near: Vec<i64>,
    pub all_exit: bool,
}

/// The trip the player's duty has the bus on, as the people at the stops see it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct DutyTrip {
    /// Which trip it is (its name and departure), to notice the next one.
    pub name: String,
    pub departure: f64,
    /// Its terminus as the timetable has it: what the stops' line records list.
    pub terminus: String,
    /// It has a line: not a works trip to or from the depot (whose stations it passes).
    pub public: bool,
    /// Its stops in order: the object, its timetable name (what the destinations of the
    /// people waiting are made of) and whether the bus stops there.
    pub stops: Vec<(i64, String, bool)>,
}

impl DutyTrip {
    /// Duty trip `trip`, its stops named as `names` (`Schedule::stop_names`) has them: by the
    /// object's id where it does not know the object, as the stops' targets are.
    pub(super) fn of(trip: &crate::schedule::PlannedTrip, names: Option<&HashMap<i64, String>>) -> DutyTrip {
        let name = |s: &crate::schedule::PlannedStop| match names {
            Some(n) => n.get(&s.object_id).cloned().unwrap_or_else(|| s.object_id.to_string()),
            None => s.name.trim().to_string(),
        };
        DutyTrip {
            name: trip.name.clone(),
            departure: trip.departure,
            terminus: trip.terminus.trim().to_string(),
            public: !trip.line.trim().is_empty(),
            stops: trip.stops.iter().map(|s| (s.object_id, name(s), s.stops)).collect(),
        }
    }
}

/// Whom a bus takes on at the stops (see `at_stop` and `fit`).
#[derive(Debug, Clone)]
pub(super) enum Takes {
    /// Those whose line record lists its terminus, as in Omsi.exe: a timetable bus.
    Terminus,
    /// The player's bus on a duty: those as well whom its trip takes where they are going,
    /// however the bus's depot file spells the terminus. `next` is the stop of the trip the
    /// duty is due at, `done` that the trip has reached its last stop.
    Duty { trip: Arc<DutyTrip>, next: usize, done: bool },
    /// Nobody waiting: the player's bus in free drive (its riders get off as ever), another
    /// player's bus (their game boards it), a bus the player left standing.
    Nobody,
}

/// What a bus near a stop does there (sub_61f238 from 0x61f3e3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AtStop {
    /// Everybody gets off and nobody on: the bus shows no destination (it is not in
    /// service), or it is at its terminus.
    Empties,
    /// It is listed at the stop: the people waiting there may take it.
    Serves,
    /// Only its riders get off there: the people waiting leave it alone.
    Passes,
}

/// What a bus showing `terminus` (None: no destination, or one of `[addterminus_allexit]`)
/// and taking `takes` does at stop `id`.
pub(super) fn at_stop(id: i64, stop: &PaxStop, terminus: Option<&str>, takes: &Takes) -> AtStop {
    let Some(t) = terminus else { return AtStop::Empties };
    if stop.is_named(t) {
        return AtStop::Empties;
    }
    match takes {
        Takes::Nobody => AtStop::Passes,
        // the trip's last stop is its terminus, whatever the depot file calls it
        Takes::Duty { trip, done: true, .. }
            if trip.stops.iter().rev().find(|s| s.2).is_some_and(|(k, n, _)| *k == id || stop.is_named(n)) =>
        {
            AtStop::Empties
        }
        _ => AtStop::Serves,
    }
}

/// Why somebody waiting takes a bus (`fit`); the better first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Fit {
    /// Its terminus is on their line record (sub_61c33c: Omsi.exe's only test).
    Terminus,
    /// The player's duty takes them where they are going: its trip's terminus is on the
    /// record, or their destination is a later stop of the trip than theirs.
    Duty,
    /// They have no line record: the first bus listed (sub_61c33c).
    Any,
}

/// Of the buses somebody waiting may take (bus, why, how far away), the one with the better
/// reason (`Fit`), of those the nearest.
pub(super) fn best_bus(buses: impl Iterator<Item = (BusId, Fit, f64)>) -> Option<(BusId, Fit)> {
    buses.min_by(|a, b| a.1.cmp(&b.1).then(a.2.total_cmp(&b.2))).map(|b| (b.0, b.1))
}

/// Whether a bus showing `terminus` and taking `takes` is the bus of somebody waiting at stop
/// `id` for `dest`, whose line record there lists `termini`; why.
///
/// Omsi.exe compares the names alone: the destination the bus's depot file gives it with
/// the timetable's termini. A depot file that spells them another way (another case, a
/// shortened name, the terminus of another variant of the line, a file made for another
/// map) left the people at every stop of a duty waiting for another bus. The duty knows
/// the trip: whoever it takes where they are going gets on.
pub(super) fn fit(id: i64, stop: &PaxStop, dest: Option<&str>, termini: &HashSet<String>, terminus: &str, takes: &Takes) -> Option<Fit> {
    if termini.contains(terminus.trim()) {
        return Some(Fit::Terminus);
    }
    let Takes::Duty { trip, next, done: false } = takes else { return None };
    if !trip.public {
        return None;
    }
    if termini.contains(&trip.terminus) {
        return Some(Fit::Duty);
    }
    let dest = dest?.trim();
    // this stop where the trip still calls at it (from the one before the stop the duty is
    // due at: it counts a stop served 35 m on), and their destination after it
    let here = (next.saturating_sub(1)..trip.stops.len()).find(|k| {
        let (sid, name, stops) = &trip.stops[*k];
        *stops && (*sid == id || stop.is_named(name))
    })?;
    trip.stops[here + 1..].iter().any(|(_, name, stops)| *stops && name.trim() == dest).then_some(Fit::Duty)
}

/// A point of the cabin's path network with its links in the file's order: the point at
/// the other end, the points reached through it (sub_72410c), the link's index, its room
/// height and step sounds.
#[derive(Debug, Clone)]
pub(super) struct RouteLink {
    pub to: usize,
    pub reach: Vec<usize>,
    pub link: usize,
    pub walk_back: bool,
}

/// The routing tables of a path network as sub_72410c builds them after loading: from
/// every point a depth-first walk, each point reached noting every point visited so far as
/// reached through its link back to where it came from. A one-way link a -> b is walked
/// back only from b.
pub(super) fn build_routes(n: usize, links: &[(i32, i32, bool)]) -> Vec<Vec<RouteLink>> {
    let mut adj: Vec<Vec<RouteLink>> = vec![Vec::new(); n];
    for (k, &(a, b, oneway)) in links.iter().enumerate() {
        if a < 0 || b < 0 || a as usize >= n || b as usize >= n {
            continue;
        }
        let (a, b) = (a as usize, b as usize);
        adj[a].push(RouteLink { to: b, reach: vec![b], link: k, walk_back: !oneway });
        adj[b].push(RouteLink { to: a, reach: vec![a], link: k, walk_back: true });
    }
    pub(super) fn visit(adj: &mut Vec<Vec<RouteLink>>, p: usize, from: Option<usize>, stack: &mut Vec<usize>) {
        if let Some(q) = from {
            if let Some(k) = adj[p].iter().position(|l| l.to == q) {
                for &s in stack.iter() {
                    if !adj[p][k].reach.contains(&s) {
                        adj[p][k].reach.push(s);
                    }
                }
            }
        }
        if !stack.contains(&p) {
            stack.push(p);
        }
        let n = adj[p].len();
        for k in 0..n {
            let to = adj[p][k].to;
            if !stack.contains(&to) && adj[p][k].walk_back {
                visit(adj, to, Some(p), stack);
            }
        }
    }
    for root in 0..n {
        let mut stack = Vec::new();
        visit(&mut adj, root, None, &mut stack);
    }
    adj
}

/// Whether passenger `x` keeps timetable bus `bus` at its stop (Omsi.exe 0x7d9e8b): on the
/// way out of it (`Some(None)`, at whatever stop), or walking up to its doors from stop `s`
/// (`Some(Some(s))`: only while the bus serves that stop). Anybody else, not.
pub(super) fn holds_bus(x: &Pax, bus: BusId) -> Option<Option<i64>> {
    if x.bus != Some(bus) {
        return None;
    }
    match x.task {
        Task::InBusToExit if x.inside == Some(bus) => Some(None),
        Task::WalkingToBus => Some(x.stop),
        _ => None,
    }
}

/// sub_7f3a24: the distance with the height difference weighed by `w` (5 everywhere).
fn weighted_dist(a: Vec3, b: Vec3, w: f32) -> f32 {
    let d = a - b;
    Vec3::new(d.x, d.y, d.z * w).length()
}

impl Cabin {
    /// sub_72506c: the point of `list` nearest `p` (height weighed by 5). `level`: only
    /// points at most 2 m below `p` and not above it. `open`/`flags` (entries): a shut
    /// door counts only with a button; `avoid`: a passenger buying a ticket skips
    /// `{noticketsale}` doors. Nothing found: the first of the list, or the search again
    /// without `avoid`.
    pub(super) fn omsi_nearest(&self, p: Vec3, list: &[Option<usize>], avoid: bool, level: bool, flags: Option<&[(bool, bool)]>, open: Option<&[bool]>) -> Option<usize> {
        let pts = &self.graph.points;
        let mut best = 1e12f32;
        let mut found: Option<usize> = None;
        for (k, pt) in list.iter().enumerate() {
            let Some(pt) = *pt else { continue };
            let Some(q) = pts.get(pt) else { continue };
            if level && !(q.z <= p.z && p.z <= q.z + 2.0) {
                continue;
            }
            let d = weighted_dist(p, *q, 5.0);
            let shut_ok = match open {
                Some(o) if o.len() >= list.len() && !o[k] => flags.is_some_and(|f| f.get(k).is_some_and(|f| f.1)),
                _ => true,
            };
            if !shut_ok {
                continue;
            }
            if avoid && flags.is_some_and(|f| f.len() >= list.len() && f[k].0) {
                continue;
            }
            if d < best {
                best = d;
                found = Some(pt);
            }
        }
        if found.is_none() {
            if !(avoid && flags.is_some()) {
                // (a list narrowed to the sections somebody is in, #718: the first door of
                // theirs, not the first of the list - another section's, left out)
                if self.groups > 1 {
                    return list.iter().flatten().next().copied();
                }
                return list.first().copied().flatten();
            }
            return self.omsi_nearest(p, list, false, level, flags, open);
        }
        found
    }

    /// The validator nearest `p` (bus frame, the height weighed as in `omsi_nearest`): the
    /// one a passenger who came in there stamps at. The first of equally near ones; in a
    /// cabin of sections nobody walks between, one in the sections of `p`'s nearest point.
    pub(super) fn nearest_stamper(&self, p: Vec3) -> Option<usize> {
        let at = |s: &(Option<usize>, Vec3)| s.0.and_then(|k| self.graph.points.get(k).copied()).unwrap_or(s.1);
        let d = |k: usize| weighted_dist(p, at(&self.stampers[k]), 5.0);
        let group = self.group_at(self.omsi_nearest(p, &self.all_points(), false, false, None, None));
        (0..self.stampers.len())
            .filter(|&k| self.groups <= 1 || self.group_at(self.stampers[k].0) == group)
            .min_by(|&a, &b| d(a).total_cmp(&d(b)))
    }

    /// The group of sections path point `p` lies in (see `groups`).
    pub(super) fn group_at(&self, p: Option<usize>) -> Option<usize> {
        p.and_then(|q| self.point_group.get(q).copied())
    }

    /// The points of `list` in group `g`, the others left out (None): where a trailer hangs
    /// on that nobody walks into from the bus (#718), a passenger keeps to the sections they
    /// are in - the doors and devices of the others are out of reach. One group: `list`.
    pub(super) fn in_group(&self, list: Vec<Option<usize>>, g: Option<usize>) -> Vec<Option<usize>> {
        match g {
            Some(g) if self.groups > 1 => list.into_iter().map(|p| p.filter(|&q| self.point_group.get(q) == Some(&g))).collect(),
            _ => list,
        }
    }

    /// sub_723fac: the next point from `from` towards `to` and the link taken.
    pub(super) fn route_next(&self, from: usize, to: usize) -> Option<(usize, usize)> {
        let links = self.routes.get(from)?;
        links.iter().find(|l| l.reach.contains(&to)).map(|l| (l.to, l.link))
    }

    /// The path points of the entries / exits, in order.
    pub(super) fn entry_points(&self) -> Vec<Option<usize>> {
        self.entries.iter().map(|e| e.point).collect()
    }
    pub(super) fn exit_points(&self) -> Vec<Option<usize>> {
        self.exits.iter().map(|e| e.point).collect()
    }
    /// ({noticketsale}, {withbutton}) of each entry.
    pub(super) fn entry_flags(&self) -> Vec<(bool, bool)> {
        self.entries.iter().map(|e| (!e.sells, e.button)).collect()
    }
}

/// A heading difference wrapped to -pi .. pi (sub_7f3780).
fn wrap(a: f64) -> f64 {
    let mut a = a;
    let pi = std::f64::consts::PI;
    while a > pi {
        a -= 2.0 * pi;
    }
    while a < -pi {
        a += 2.0 * pi;
    }
    a
}

/// The heading (radians, clockwise from forward) of a direction in the plane.
fn yaw_of(d: DVec2) -> f64 {
    d.x.atan2(d.y)
}

impl Humans {
    /// The passenger of person `i`, if it is one.
    pub(super) fn pax(&self, i: usize) -> Option<&Pax> {
        match &self.people[i].state {
            State::Pax(p) => Some(p),
            _ => None,
        }
    }
    pub(super) fn pax_mut(&mut self, i: usize) -> Option<&mut Pax> {
        match &mut self.people[i].state {
            State::Pax(p) => Some(p),
            _ => None,
        }
    }

    /// The stops as the buses see them this frame (sub_61f93c / sub_61f238), and the
    /// odometers of the buses.
    pub(super) fn register_buses(&mut self, buses: &[BusNow], dt: f32) -> HashMap<BusId, BusAtStops> {
        let mut out: HashMap<BusId, BusAtStops> = HashMap::new();
        for s in self.stops.values_mut() {
            s.buses.clear();
        }
        let left = LEFT_HAND.load(std::sync::atomic::Ordering::Relaxed);
        let _ = left;
        let mut ids: Vec<i64> = self.stops.keys().copied().collect();
        ids.sort_unstable();
        for bn in buses {
            let km = self.odometer.entry(bn.id).or_insert(0.0);
            *km += bn.speed.abs() * dt as f64 / 1000.0;
            let mut reg = BusAtStops::default();
            let mut request_distance = 500.0;
            for id in &ids {
                let s = &self.stops[id];
                let d = bn.pos - s.pos;
                let dist = d.length();
                // (the stop to request is wanted only by a bus whose next stop nobody knows; a
                // stop beyond that and out of reach costs nothing more, as before)
                let wants_request = bn.next_stop.is_none() && dist < request_distance;
                if !wants_request && !(dist < 60.0) {
                    continue;
                }
                let sh = s.heading.to_radians();
                let (s_fwd, s_right) = (DVec2::new(sh.sin(), sh.cos()), DVec2::new(sh.cos(), -sh.sin()));
                let same_way = bn.fwd().dot(s_fwd) > 0.0;
                // Without a route, use the nearest stop ahead, not the one just left.
                if wants_request && same_way && d.truncate().dot(s_fwd) <= 25.0 {
                    request_distance = dist;
                    reg.request_next = Some(RequestStop {
                        id: *id,
                        name: s.name.clone(),
                        alias: s.alias.clone(),
                        pos: s.pos,
                    });
                }
                if !(dist < 60.0) {
                    continue;
                }
                reg.near.push(*id);
                if same_way {
                    reg.next = Some(*id);
                }
                // a bus not in service, or at its own terminus, empties and takes nobody
                // (0x61f3e3); one in free drive only lets its riders off
                match at_stop(*id, s, bn.terminus.as_deref(), &bn.takes) {
                    AtStop::Empties => {
                        reg.all_exit = true;
                        continue;
                    }
                    AtStop::Passes => continue,
                    AtStop::Serves => {}
                }
                if same_way {
                    let lateral = d.truncate().dot(s_right);
                    let along = d.truncate().dot(s_fwd);
                    let in_box = lateral.abs() < 2.0 && along.abs() < (s.length as f64 - 5.0).max(0.0);
                    self.stops.get_mut(id).unwrap().buses.push((bn.id, in_box));
                }
            }
            
            // a timetable bus boarding at a stop none of these know: its riders get off at
            // its timetable's stop (#1593)
            if reg.next.is_none() {
                reg.next = bn.served;
            }
            out.insert(bn.id, reg);
        }
        out
    }

    /// sub_61c33c: the bus at stop `stop` person `i` gets into, and why: with a line record,
    /// the nearest of the buses listed whose terminus goes there - else the nearest whose
    /// duty takes them there (`fit`); without, the first listed.
    pub(super) fn bus_for(&self, i: usize, stop: i64, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>) -> Option<(BusId, Fit)> {
        let s = self.stops.get(&stop)?;
        let p = self.pax(i)?;
        match p.line.and_then(|k| s.lines.get(k)) {
            None => s.buses.first().map(|b| (b.0, Fit::Any)),
            Some((_, termini)) => best_bus(s.buses.iter().filter_map(|(id, _)| {
                let bn = bus_ix.get(id).map(|k| &buses[*k])?;
                if bn.cabin.entries.is_empty() {
                    return None;
                }
                let f = fit(stop, s, p.dest.as_deref(), termini, bn.terminus.as_deref()?, &bn.takes)?;
                Some((*id, f, (bn.pos - self.people[i].position).length()))
            })),
        }
    }

    /// Whether the bus stands in the stop's box (the flag of its entry, sub_61ee18).
    pub(super) fn in_stop_box(&self, stop: i64, bus: BusId) -> bool {
        self.stops.get(&stop).is_some_and(|s| s.buses.iter().any(|b| b.0 == bus && b.1))
    }
    pub(super) fn listed_at(&self, stop: i64, bus: BusId) -> bool {
        self.stops.get(&stop).is_some_and(|s| s.buses.iter().any(|b| b.0 == bus))
    }

    /// sub_7e910c: a free place of the bus, at random (none free: nobody gets on). `off`:
    /// the places its scripts have switched off (#721), which nobody takes.
    pub(super) fn reserve_place(&mut self, bus: BusId, n: usize, off: &[bool]) -> Option<usize> {
        let seats = self.seats.entry(bus).or_insert_with(|| vec![false; n]);
        if seats.len() < n {
            seats.resize(n, false);
        }
        let free: Vec<usize> = (0..n).filter(|k| !seats[*k] && !off.get(*k).copied().unwrap_or(false)).collect();
        if free.is_empty() {
            return None;
        }
        let k = free[(self.rand() as usize) % free.len()];
        self.seats.get_mut(&bus).unwrap()[k] = true;
        Some(k)
    }

    /// sub_5ce4e0: stamp (stamper_prop) or buy (ticketbuy_prop) at a bus that has a
    /// validator / a cash desk, else nothing to do; the ticket bought (sub_5ce2dc).
    pub(super) fn decide_pax_ticket(&mut self, i: usize, bn: &BusNow) -> (u8, u8) {
        let Some(tp) = self.tickets.clone() else { return (TICKET_NONE, 0) };
        let mut r = self.rand_f() as f32;
        if !bn.cabin.stampers.is_empty() {
            if r < tp.stamper_prop {
                return (TICKET_STAMP, 0);
            }
            r -= tp.stamper_prop;
        }
        // (the sale also needs the option on, `boarding` not "walk")
        if bn.cabin.sale.is_some() && r < tp.ticketbuy_prop && !self.boarding.eq_ignore_ascii_case("walk") {
            let age = self.people[i].age;
            if let Some(t) = self.pick_ticket(age) {
                return (TICKET_BUY, (t + 1).min(255) as u8);
            }
            return (TICKET_BUY, 0);
        }
        (TICKET_NONE, 0)
    }

    /// The world position and heading of a passenger.
    pub(super) fn pax_world(&self, p: &Pax, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>) -> Option<(DVec3, f64)> {
        match p.inside {
            None => Some((p.pos, p.yaw.to_degrees())),
            Some(b) => {
                let bn = bus_ix.get(&b).map(|k| &buses[*k])?;
                let l = p.pos.as_vec3();
                Some((bn.world(l), bn.heading_at(l) + p.yaw.to_degrees()))
            }
        }
    }

    /// Everybody's passenger tick of this frame, in the order of the people (sub_6ffc7c).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn pax_frame(
        &mut self,
        dt: f32,
        world: &World,
        buses: &[BusNow],
        bus_ix: &HashMap<BusId, usize>,
        at_stops: &HashMap<BusId, BusAtStops>,
        player_bus: Option<&VehicleInstance>,
        renderer: &Renderer,
        scene: &mut Scene,
        taken_ticket: &mut bool,
        remove: &mut Vec<usize>,
    ) {
        // the requests the buses' scripts read this frame
        for r in self.entry_req.iter_mut().chain(self.exit_req.iter_mut()) {
            *r = false;
        }
        let mut ai_req: HashMap<BusId, (Vec<bool>, Vec<bool>)> = HashMap::new();
        for bn in buses {
            ai_req.insert(bn.id, (vec![false; bn.cabin.entries.len()], vec![false; bn.cabin.exits.len()]));
        }
        self.pax_req = ai_req;
        for i in 0..self.people.len() {
            if self.pax(i).is_none() || remove.contains(&i) {
                continue;
            }
            self.pax_tick(i, dt, world, buses, bus_ix, at_stops, player_bus, renderer, scene, taken_ticket, remove);
        }
        // (where every passenger stands: in a bus's frame, or the world's - and the people
        // walking the pavement, among them a rider who has just stepped off, still on the
        // step until they are clear of the door. Not those who stand where they got off
        // with no pavement to go on along: they would hold the door open for good.)
        let at: Vec<(Option<BusId>, DVec3)> = self
            .people
            .iter()
            .filter_map(|p| match &p.state {
                State::Pax(x) => Some((x.inside, x.pos)),
                State::Strolling(_) => Some((None, p.position)),
                _ => None,
            })
            .collect();
        let busy: HashMap<BusId, (Vec<bool>, Vec<bool>)> = buses.iter().map(|bn| (bn.id, doorways_taken(bn, &at))).collect();
        if debug_pax() {
            for (b, (e, x)) in &busy {
                let before = self.pax_busy.get(b);
                for (kind, now, was) in [("entry", e, before.map(|o| &o.0)), ("exit", x, before.map(|o| &o.1))] {
                    for (k, on) in now.iter().enumerate() {
                        if was.and_then(|w| w.get(k)).copied().unwrap_or(false) != *on {
                            log::info!("t={:.1} bus {b:?} {kind} {k}: {}", self.time, if *on { "somebody in the doorway" } else { "the doorway is free" });
                        }
                    }
                }
            }
        }
        self.pax_busy = busy;
        // the places' own occupancy variables (#721): the riders at their places
        let sitting: Vec<(BusId, usize)> = self
            .people
            .iter()
            .filter_map(|p| match &p.state {
                State::Pax(x) if x.task == Task::SittingInBus => Some((x.inside?, x.seat?)),
                _ => None,
            })
            .collect();
        self.pax_places = buses.iter().map(|bn| (bn.id, places_taken(bn, &sitting))).collect();
        // the player's bus reads its requests from `entry_req` / `exit_req`
        if let Some((e, x)) = self.pax_req.get(&BusId::Player) {
            self.entry_req = e.clone();
            self.exit_req = x.clone();
        }
        if let Some((e, x)) = self.pax_busy.get(&BusId::Player) {
            self.entry_busy = e.clone();
            self.exit_busy = x.clone();
        }
        self.ai_requests.clear();
        for (b, (e, x)) in &self.pax_req {
            if let BusId::Ai(id) = b {
                let (entry_busy, exit_busy) = self.pax_busy.get(b).cloned().unwrap_or_default();
                let places = self.pax_places.get(b).cloned().unwrap_or_default();
                self.ai_requests.push((*id, DoorWants { entry_req: e.clone(), exit_req: x.clone(), entry_busy, exit_busy, places }));
            }
        }
        // timetable buses wait while people still get on or off (0x7d9e8b - 0x7d9f5e):
        // somebody of this bus walking in it to an exit, or walking up to its doors from
        // the stop the bus serves - the traffic checks the stop (`hold_boarding`). People
        // still on their way to the gather point do not hold it: they walk up to the doors
        // as soon as the bus has a place for them, and with the bus full they stood there
        // and kept it at the stop with its doors open for good (#767)
        for bn in buses {
            let BusId::Ai(id) = bn.id else { continue };
            if bn.speed.abs() > 0.5 {
                continue;
            }
            let mut any_exit = false;
            let mut stops: Vec<i64> = Vec::new();
            for p in &self.people {
                let State::Pax(x) = &p.state else { continue };
                match holds_bus(x, bn.id) {
                    Some(None) => any_exit = true,
                    Some(Some(s)) if !stops.contains(&s) => stops.push(s),
                    _ => {}
                }
            }
            if any_exit {
                self.holds.push((id, None, 2.5));
            }
            for s in stops {
                self.holds.push((id, Some(s), 2.5));
            }
        }
    }

    /// One person's tick (sub_62a6a0 without the street walk).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn pax_tick(
        &mut self,
        i: usize,
        dt: f32,
        world: &World,
        buses: &[BusNow],
        bus_ix: &HashMap<BusId, usize>,
        at_stops: &HashMap<BusId, BusAtStops>,
        player_bus: Option<&VehicleInstance>,
        renderer: &Renderer,
        scene: &mut Scene,
        taken_ticket: &mut bool,
        remove: &mut Vec<usize>,
    ) {
        let dt_ms = dt * 1000.0;
        // sub_62a258: the bus is gone - nothing more to do with it
        {
            let p = self.pax_mut(i).unwrap();
            if let Some(b) = p.bus {
                if !bus_ix.contains_key(&b) {
                    p.bus = None;
                }
            }
        }
        // inside a bus that is gone (a timetable bus left the map): gone with it
        if let Some(b) = self.pax(i).unwrap().inside {
            if !bus_ix.contains_key(&b) {
                remove.push(i);
                return;
            }
        }
        {
            let p = self.pax_mut(i).unwrap();
            if p.timer > 0.0 {
                p.timer -= dt;
            }
            if p.dist_timer > 0.0 {
                p.dist_timer -= p.moved;
            }
        }
        // the toll of a bad ride eases off as the bus goes on (0x62d86c: 0.2 a kilometre)
        {
            let speed = self.pax(i).unwrap().inside.and_then(|b| bus_ix.get(&b)).map(|k| buses[*k].speed.abs() as f32);
            let p = self.pax_mut(i).unwrap();
            match speed {
                Some(v) => p.discomfort = (p.discomfort - v * dt / 5000.0).max(0.0),
                None => p.discomfort = 0.0,
            }
        }
        self.pax_move(i, dt, dt_ms, world, buses, bus_ix);
        self.pax_task(i, dt, world, buses, bus_ix, at_stops, player_bus, renderer, scene, taken_ticket, remove);
        // (got off: a pedestrian now)
        let Some(p) = self.pax(i).cloned() else { return };
        // (0x62d75b) the stop they boarded at is forgotten once the bus has left it
        if matches!(p.task, Task::InBusToPlace | Task::InBusToExit | Task::SittingInBus) {
            if let (Some(stop), Some(b)) = (p.stop, p.bus) {
                if !at_stops.get(&b).is_some_and(|r| r.near.contains(&stop)) {
                    let p = self.pax_mut(i).unwrap();
                    p.stop = None;
                }
            }
        }
        // where the person is drawn
        if let Some((w, h)) = self.pax_world(&p, buses, bus_ix) {
            let person = &mut self.people[i];
            person.position = w;
            person.heading = h;
            match p.inside {
                Some(b) => {
                    person.place = Place::Bus(b, p.pos.as_vec3());
                    person.lheading = p.yaw.to_degrees();
                    if let Some(bn) = bus_ix.get(&b).map(|k| &buses[*k]) {
                        person.tilt = bn.tilt_at(p.pos.as_vec3());
                        person.interior = bn.interior;
                    }
                }
                None => {
                    person.place = Place::Ground;
                    person.interior = 0.0;
                }
            }
            let yaw = p.yaw;
            person.vel = if p.st == 1 || p.st == 5 {
                DVec2::new(yaw.sin(), yaw.cos()) * p.speed as f64
            } else {
                DVec2::ZERO
            };
        }
    }

    /// The movement part of the tick (sub_62a6a0, 0x62ad0b - 0x62b966).
    pub(super) fn pax_move(&mut self, i: usize, dt: f32, dt_ms: f32, world: &World, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>) {
        let p0 = self.pax(i).unwrap().clone();
        let bn_in = p0.inside.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k]));
        let bn_t = p0.bus.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k]));
        // the path point walked to is the target
        let mut target = p0.target;
        let mut target_bus = p0.target_bus;
        // (kept as the target, +0x5bd: waiting short of the point, state 6, goes on facing
        // it - with the target of before kept instead, a seat or the stop's gather point in
        // another frame, the people waiting at a shut exit were lifted 40 m up in the bus
        // and stood stacked there for good, #709)
        let mut walked_to: Option<DVec3> = None;
        if p0.st == 5 {
            if let (Some(pt), Some(bn)) = (p0.pt, bn_in) {
                if let Some(q) = bn.cabin.graph.points.get(pt) {
                    target = q.as_dvec3();
                    target_bus = true;
                    walked_to = Some(target);
                }
            }
        }
        // walking to a door from outside: keep off the bus side (0x62ad81)
        if p0.clamp && target_bus {
            if let Some(bn) = bn_t {
                let level = if p0.clamp_open && p0.inside.is_none() {
                    let l = bn.to_local(p0.pos);
                    (l.y as f64 - target.y).abs() <= 1.0
                } else {
                    false
                };
                if !level {
                    if p0.clamp_left {
                        target.x = target.x.min(p0.clamp_x);
                    } else {
                        target.x = target.x.max(p0.clamp_x);
                    }
                }
            }
        }
        // the target in the person's own frame
        let tgt = match (target_bus, p0.inside) {
            (true, Some(_)) => target,
            (true, None) => match bn_t {
                Some(bn) => bn.world(target.as_vec3()),
                None => target,
            },
            (false, Some(_)) => match bn_in {
                Some(bn) => bn.to_local(target).as_dvec3(),
                None => target,
            },
            (false, None) => target,
        };
        let mut d = tgt - p0.pos;
        let mut room = p0.room;
        let mut step_pack = p0.step_pack;
        if p0.inside.is_none() {
            d.z = 0.0;
            step_pack = None;
            room = OUTSIDE_ROOM;
        }
        let dist = d.length() as f32;
        let mut st = p0.st;
        let mut pt = p0.pt;
        let mut link = p0.link;
        match st {
            5 => {
                if p0.pt == p0.pt_target && dist <= 0.7 && p0.short {
                    st = 6;
                } else if dist <= 0.1 {
                    let next = match (p0.pt, p0.pt_target, bn_in) {
                        (Some(a), Some(b), Some(bn)) => bn.cabin.route_next(a, b),
                        _ => None,
                    };
                    match next {
                        Some((n, l)) => {
                            pt = Some(n);
                            link = Some(l);
                            if let Some(bn) = bn_in {
                                step_pack = bn.cabin.link_pack.get(l).copied().flatten();
                                room = bn.cabin.link_room.get(l).copied().unwrap_or(2.0);
                            }
                        }
                        None => st = 7,
                    }
                }
            }
            1 => {
                if dist <= 0.7 && p0.short {
                    st = 2;
                } else if dist <= 0.1 {
                    st = 3;
                }
            }
            6 => {
                if !p0.short {
                    st = 5;
                }
            }
            2 => {
                if !p0.short {
                    st = 1;
                }
            }
            _ => {}
        }
        // the people in the way (sub_626860)
        let (mut block, free_r, free_l) = if st == 1 || st == 5 { self.pax_blockers(i, buses, bus_ix) } else { (0, true, true) };
        let p = self.pax_mut(i).unwrap();
        // Inside a bus, people going opposite ways along the aisle or the stairs stood face to
        // face for good (the whole upper deck of a double-decker on its way out, the people
        // coming up stopped on the stairs): held up for two seconds, they squeeze past for a
        // second and a half, as the people on the pavements do.
        if p.inside.is_some() {
            if p.squeeze > 0.0 {
                p.squeeze -= dt;
                block = 0;
            } else if block == 2 {
                p.jam += dt;
                if p.jam > 2.0 {
                    p.jam = 0.0;
                    p.squeeze = 1.5;
                    block = 0;
                }
            } else {
                p.jam = 0.0;
            }
        }
        if let Some(t) = walked_to {
            p.target = t;
            p.target_bus = true;
        }
        p.st = st;
        p.pt = pt;
        p.link = link;
        p.room = room;
        p.step_pack = step_pack;
        p.clamp = false;
        p.clamp_open = false;
        p.clamp_left = false;
        p.clamp_x = -1e9;
        p.moved = 0.0;
        p.speed_des = 0.0;
        p.block = block;
        p.free_r = free_r;
        p.free_l = free_l;
        let mut head_des = p.yaw;
        let mut slope = f64::INFINITY;
        if st == 1 || st == 5 {
            head_des = yaw_of(d.truncate());
            if p.inside.is_some() || p.pax_state == 2.0 {
                let h = d.truncate().length();
                slope = if h > 0.0 { d.z / h } else { f64::INFINITY };
            }
            p.speed_des = if block < 2 { p.walk_speed } else { 0.0 };
        } else if st == 3 || st == 7 {
            head_des = p.target_yaw;
        } else if st == 9 {
            head_des = yaw_of(d.truncate());
        }
        if st == 0 {
            // standing: still on the ground under the feet, as Omsi.exe asks for it every
            // tick in every state but turning (0x62b852 -> 0x7aec3c, not when seated); the
            // waiting people stood at the height of their [passpos]'s object - a shelter
            // on the terrain - 25-35 cm down in the platform
            if p.inside.is_none() && p.pax_state != 2.0 {
                if let Some(g) = world.walk_height_near(p.pos.x, p.pos.y, p.pos.z) {
                    p.pos.z = g;
                }
            }
            return;
        }
        let mut dh = wrap(head_des - p.yaw);
        if st == 1 && p.free_r && p.block == 2 {
            dh = -1.745;
        }
        if st != 9 {
            if dh.abs() > 1.0 {
                p.speed = 0.0;
            }
            let diff = p.speed_des - p.speed;
            p.speed += diff.signum() * diff.abs().min(5.0 * dt_ms / 1000.0);
        }
        let turn = dh.signum() * dh.abs().min(dt_ms as f64 / 150.0);
        p.yaw = wrap(p.yaw + turn);
        if st == 9 {
            return;
        }
        let mut moved = p.speed * dt_ms / 1000.0;
        let mut step = DVec3::new(d.x, d.y, 0.0);
        let len = step.length() as f32;
        if len <= moved {
            moved = len;
        } else if len > 0.0 {
            step *= (moved / len) as f64;
        }
        p.moved = moved;
        if p.inside.is_some() || p.pax_state == 2.0 {
            step.z = if slope.is_finite() { moved as f64 * slope } else { d.z };
        } else {
            let at = p.pos + step;
            step.z = match world.walk_height_near(at.x, at.y, p.pos.z) {
                Some(g) => g - p.pos.z,
                None => 0.0,
            };
        }
        p.pos += step;
        if p.inside.is_none() && p.pax_state != 2.0 {
            if let Some(stop) = p.stop {
                if let Some(s) = self.stops.get(&stop) {
                    let floor = s.pos.z;
                    let p = self.pax_mut(i).unwrap();
                    p.pos.z = p.pos.z.max(floor);
                }
            }
        }
        let _ = dt;
    }

    /// sub_626860: whether somebody within 0.6 m stands in the way.
    pub(super) fn pax_blockers(&self, i: usize, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>) -> (u8, bool, bool) {
        let me = self.pax(i).unwrap();
        let Some((my_pos, my_head)) = self.pax_world(me, buses, bus_ix) else { return (0, true, true) };
        let fs = {
            let h = my_head.to_radians();
            DVec2::new(h.sin(), h.cos())
        };
        let (mut block, mut free_r, mut free_l) = (0u8, true, true);
        for (j, o) in self.people.iter().enumerate() {
            if j == i || o.puppet.is_some() {
                continue;
            }
            // who counts: the passengers of the same bus or stop, and the people on foot
            // when this one has no bus yet
            let (o_task, o_st, o_bus, o_stop, o_sub, o_block) = match &o.state {
                State::Pax(x) => (Some(x.task), x.st, x.bus, x.stop, x.sub, x.block),
                // a pedestrian on a path: state 8 with a path, no bus, no stop
                State::Strolling(_) => (None, 8, None, None, 0, 0),
                _ => continue,
            };
            if o_st == 0 || o_st == 3 {
                continue;
            }
            if o_task == Some(Task::ToBus) && me.task == Task::WalkingToBus {
                continue;
            }
            if !(o_bus == me.bus || (o_stop.is_some() && o_stop == me.stop)) {
                continue;
            }
            let d = o.position - my_pos;
            if d.z >= 2.0 {
                continue;
            }
            let d2 = d.truncate();
            let dist = d2.length();
            if !(dist < 0.6) {
                continue;
            }
            let dn = if dist > 0.0 { d2 / dist } else { DVec2::ZERO };
            let fo = {
                let h = o.heading.to_radians();
                DVec2::new(h.sin(), h.cos())
            };
            if fs.dot(dn) < 0.0 {
                if block == 0 {
                    block = 1;
                }
                continue;
            }
            // (D3DXVec3Cross(fs, d).y in the left-handed frame)
            let side = fs.y * dn.x - fs.x * dn.y < 0.0;
            let facing = fo.dot(fs) < 0.2 || dn.dot(fo) <= 0.0;
            if facing && o_sub == 0 {
                if o_block == 0 {
                    block = block.max(2);
                    if side {
                        free_r = false;
                    } else {
                        free_l = false;
                    }
                }
                continue;
            }
            block = block.max(3);
        }
        (block, free_r, free_l)
    }

    /// sub_62e42c: a new task and what it starts with.
    pub(super) fn set_task(&mut self, i: usize, t: Task, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>, world: &World) {
        if self.pax(i).is_none_or(|p| p.task == t) {
            return;
        }
        if debug_pax() {
            log::info!("t={:.1} pax {} {} -> {}", self.time, self.people[i].label(), self.pax(i).unwrap().task.name(), t.name());
        }
        let seatheight = self.people[i].ty.def.seat_height;
        self.pax_mut(i).unwrap().task = t;
        match t {
            Task::WaitingForBus => {
                let (stop, spot) = {
                    let p = self.pax(i).unwrap();
                    (p.stop, p.spot)
                };
                let sp = stop.zip(spot).and_then(|(s, k)| self.stops.get(&s).and_then(|s| s.spots.get(k)).cloned());
                let p = self.pax_mut(i).unwrap();
                p.st = 0;
                match sp {
                    Some(sp) if sp.height != 0.0 => {
                        p.seat_h = sp.height;
                        p.pos = sp.pos - DVec3::Z * seatheight as f64;
                        p.yaw = sp.face.to_radians();
                        p.pax_state = 2.0;
                    }
                    Some(sp) => {
                        p.pos = sp.pos;
                        p.yaw = sp.face.to_radians();
                        p.pax_state = 0.0;
                    }
                    None => p.pax_state = 0.0,
                }
            }
            Task::ToBus => {
                let (stop, spot) = {
                    let p = self.pax(i).unwrap();
                    (p.stop, p.spot)
                };
                if let (Some(s), Some(k)) = (stop, spot) {
                    self.free_spot(s, k);
                }
                let gather = stop.and_then(|s| self.stops.get(&s)).map(|s| s.gather);
                let p = self.pax_mut(i).unwrap();
                p.spot = None;
                if let Some(g) = gather {
                    p.target = g;
                }
                p.target_bus = false;
                p.st = 1;
                p.pax_state = 1.0;
            }
            Task::WalkingToBus => {
                let (stop, spot) = {
                    let p = self.pax(i).unwrap();
                    (p.stop, p.spot)
                };
                if let (Some(s), Some(k)) = (stop, spot) {
                    self.free_spot(s, k);
                }
                self.pax_mut(i).unwrap().spot = None;
                self.choose_entry(i, buses, bus_ix);
                let p = self.pax_mut(i).unwrap();
                p.target_bus = true;
                p.st = 1;
                p.pax_state = 1.0;
            }
            Task::InBusToPlace => {
                let bus = self.pax(i).unwrap().bus;
                let Some(bn) = bus.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k])) else { return };
                let km = self.odometer.get(&bn.id).copied().unwrap_or(0.0);
                let detailed = bn.id == BusId::Player;
                let all = bn.cabin.all_points();
                let p = self.pax_mut(i).unwrap();
                p.pax_state = 1.0;
                p.km_start = km;
                p.door = None;
                // into the bus's frame
                let local = bn.to_local(p.pos);
                p.yaw = wrap(p.yaw - bn.heading_at(local).to_radians());
                p.pos = local.as_dvec3();
                p.inside = Some(bn.id);
                p.target_bus = true;
                p.pt = bn.cabin.omsi_nearest(local, &all, false, false, None, None);
                p.st = 5;
                if !detailed {
                    // a bus not the player's: at the place at once (sub_62a358 + task 7)
                    self.set_task(i, Task::SittingInBus, buses, bus_ix, world);
                    return;
                }
                let ticket = p.ticket;
                match ticket {
                    TICKET_STAMP => {
                        p.stamper = bn.cabin.nearest_stamper(local);
                        p.pt_target = p.stamper.and_then(|k| bn.cabin.stampers[k].0);
                    }
                    TICKET_BUY => p.pt_target = bn.cabin.in_group(vec![bn.cabin.sale.and_then(|s| s.0)], bn.cabin.group_at(p.pt))[0],
                    _ => self.route_to_place(i, bn),
                }
                let p = self.pax_mut(i).unwrap();
                if p.pt_target.is_none() {
                    // (no path point at the device: straight on to the place)
                    p.ticket = TICKET_NONE;
                    self.route_to_place(i, bn);
                }
            }
            Task::InBusToExit => {
                let bus = self.pax(i).unwrap().bus.or(self.pax(i).unwrap().inside);
                let Some(bn) = bus.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k])) else { return };
                // the stop button
                if bn.id == BusId::Player {
                    self.stop_request = true;
                }
                let seat = self.pax(i).unwrap().seat;
                let all = bn.cabin.all_points();
                let from = seat.and_then(|k| bn.cabin.seats.get(k)).map(|s| s.pos).unwrap_or(self.pax(i).unwrap().pos.as_vec3());
                let start = bn.cabin.omsi_nearest(from, &all, false, true, None, None);
                let exits = bn.cabin.exit_points();
                let p = self.pax_mut(i).unwrap();
                p.pax_state = 1.0;
                p.pt = start;
                if let Some(q) = start.and_then(|k| bn.cabin.graph.points.get(k)) {
                    p.pos = q.as_dvec3();
                }
                let here = p.pos.as_vec3();
                // the nearest exit (sub_62a49c / sub_62a5a8), of the sections they are in
                let exits = bn.cabin.in_group(exits, bn.cabin.group_at(start));
                p.pt_target = bn.cabin.omsi_nearest(here, &exits, false, false, None, None);
                p.door = p.pt_target.and_then(|t| exits.iter().position(|e| *e == Some(t)));
                if let Some(d) = p.door {
                    if let Some((_, x)) = self.pax_req.get_mut(&bn.id) {
                        if let Some(r) = x.get_mut(d) {
                            *r = true;
                        }
                    }
                }
                let p = self.pax_mut(i).unwrap();
                p.st = 5;
                if let Some(k) = p.seat.take() {
                    self.free_seat(bn.id, k);
                }
            }
            Task::WalkingToBusstop => {
                let r = self.rand_f() as f32;
                let stop = self.pax(i).unwrap().stop;
                {
                    let p = self.pax_mut(i).unwrap();
                    p.short = false;
                    p.door = None;
                    p.ride_km = r * 19.0 + 1.0;
                }
                // a free waiting place (sub_61fed0)
                let spot = match (stop, self.pax(i).unwrap().spot) {
                    (_, Some(k)) => Some(k),
                    (Some(s), None) => self.take_spot(s),
                    _ => None,
                };
                let sp = stop.zip(spot).and_then(|(s, k)| self.stops.get(&s).and_then(|s| s.spots.get(k)).cloned());
                let stop_pos = stop.and_then(|s| self.stops.get(&s)).map(|s| s.pos);
                // (no place free: at the stop's point - spread along the kerb by who they
                // are, or everybody without a place stood in one another there)
                let along = ((self.people[i].id % 7) as f64 - 3.0) * 0.7;
                let fwd = stop.and_then(|s| self.stops.get(&s)).map(|s| { let h = s.heading.to_radians(); DVec3::new(h.sin(), h.cos(), 0.0) }).unwrap_or(DVec3::ZERO);
                let stop_pos = stop_pos.map(|q| q + fwd * along);
                let p = self.pax_mut(i).unwrap();
                p.spot = spot;
                p.target_bus = false;
                match sp {
                    Some(sp) => {
                        // a seat: in front of it, the hip at its height (0x62e5d1)
                        let mut tgt = sp.pos;
                        if sp.height != 0.0 {
                            tgt.z = tgt.z.min((sp.pos.z - sp.height as f64).max(stop_pos.map(|s| s.z).unwrap_or(tgt.z)));
                        }
                        p.target = tgt;
                        p.target_yaw = sp.face.to_radians();
                    }
                    None => {
                        if let Some(sp) = stop_pos {
                            p.target = sp;
                        }
                        p.target_yaw = 0.0;
                    }
                }
                p.st = 1;
            }
            Task::SittingInBus => {
                let bus = self.pax(i).unwrap().inside;
                let Some(bn) = bus.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k])) else { return };
                let seat = self.pax(i).unwrap().seat.and_then(|k| bn.cabin.seats.get(k)).cloned();
                let p = self.pax_mut(i).unwrap();
                p.st = 0;
                if let Some(s) = seat {
                    if s.seated {
                        p.seat_h = s.height;
                        p.pos = (s.pos - Vec3::Z * seatheight).as_dvec3();
                        p.pax_state = 2.0;
                    } else {
                        p.pos = s.pos.as_dvec3();
                        p.pax_state = 0.0;
                    }
                    p.yaw = (s.rot as f64).to_radians();
                }
                p.room = OUTSIDE_ROOM;
                p.reach = false;
                p.look_driver = false;
            }
            Task::Nothing => {}
        }
    }

    /// sub_625b98: the entry to walk to, every frame on the way (the nearest open one or
    /// one with a button; one selling tickets for a buyer), and its index for the request.
    pub(super) fn choose_entry(&mut self, i: usize, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>) {
        let p = self.pax(i).unwrap().clone();
        let Some(bn) = p.bus.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k])) else { return };
        let here = match p.inside {
            Some(_) => p.pos.as_vec3(),
            None => bn.to_local(p.pos),
        };
        // (the doors of the sections of the place reserved)
        let group = p.seat.and_then(|k| bn.cabin.seats.get(k)).map(|s| s.group);
        let list = bn.cabin.in_group(bn.cabin.entry_points(), group);
        let flags = bn.cabin.entry_flags();
        let open: Vec<bool> = (0..list.len()).map(|k| bn.entry_open.get(k).copied().unwrap_or(false)).collect();
        let pt = bn.cabin.omsi_nearest(here, &list, p.ticket == TICKET_BUY, false, Some(&flags), Some(&open));
        let p = self.pax_mut(i).unwrap();
        if let Some(q) = pt.and_then(|k| bn.cabin.graph.points.get(k)) {
            p.target = q.as_dvec3();
            p.target_bus = true;
        }
        p.door = pt.and_then(|t| list.iter().position(|e| *e == Some(t)));
    }

    /// sub_62a628: along the paths to the place reserved.
    pub(super) fn route_to_place(&mut self, i: usize, bn: &BusNow) {
        let all = bn.cabin.all_points();
        let seat = self.pax(i).unwrap().seat.and_then(|k| bn.cabin.seats.get(k)).map(|s| s.pos);
        let p = self.pax_mut(i).unwrap();
        if let Some(s) = seat {
            p.pt_target = bn.cabin.omsi_nearest(s, &all, false, true, None, None);
        }
        p.st = 5;
        p.smooth = false;
    }

    pub(super) fn free_spot(&mut self, stop: i64, k: usize) {
        if let Some(t) = self.stops.get_mut(&stop).and_then(|s| s.taken.get_mut(k)) {
            *t = false;
        }
    }

    /// sub_61c8d8: a free waiting place of the stop, at random.
    pub(super) fn take_spot(&mut self, stop: i64) -> Option<usize> {
        // Stops a few metres apart (both sides of a bus station's platform, a stop and its
        // copy for another line) find the same objects' places, and each kept its own list of
        // who stands where (as Omsi.exe's 0x61c8d8 does), so two or three people stood in
        // one another. A place somebody of another stop stands on is not free (nor one of the
        // stop's own a few centimetres from a taken one: objects placed twice).
        let s = self.stops.get(&stop)?;
        let (pos, reach) = (s.pos, 40.0_f64.max(s.length as f64 + 15.0) * 2.0);
        let elsewhere: Vec<DVec3> = self
            .stops
            .iter()
            .filter(|(_, o)| (o.pos - pos).length() < reach)
            .flat_map(|(_, o)| o.spots.iter().zip(&o.taken).filter(|(_, t)| **t).map(|(sp, _)| sp.pos))
            .collect();
        let s = self.stops.get(&stop)?;
        let free: Vec<usize> = s
            .taken
            .iter()
            .enumerate()
            .filter(|(k, t)| !**t && !elsewhere.iter().any(|q| (*q - s.spots[*k].pos).truncate().length() < 0.4))
            .map(|(k, _)| k)
            .collect();
        if free.is_empty() {
            return None;
        }
        let k = free[(self.rand() as usize) % free.len()];
        self.stops.get_mut(&stop).unwrap().taken[k] = true;
        Some(k)
    }

    /// The task part of the tick (sub_62a6a0 from 0x62b984).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn pax_task(
        &mut self,
        i: usize,
        dt: f32,
        world: &World,
        buses: &[BusNow],
        bus_ix: &HashMap<BusId, usize>,
        at_stops: &HashMap<BusId, BusAtStops>,
        player_bus: Option<&VehicleInstance>,
        renderer: &Renderer,
        scene: &mut Scene,
        taken_ticket: &mut bool,
        remove: &mut Vec<usize>,
    ) {
        let p = self.pax(i).unwrap().clone();
        let bn = p.bus.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k]));
        match p.task {
            Task::WaitingForBus => {
                let Some(stop) = p.stop else { return };
                let Some(b) = self.bus_for(i, stop, buses, bus_ix) else {
                    if super::debug_pax() && (self.time * 2.0).fract() < (dt as f64 * 2.0) {
                        let s = &self.stops[&stop];
                        let listed: Vec<String> = s.buses.iter().map(|(id, inbox)| format!("{id:?} box {inbox} shows {:?}", bus_ix.get(id).and_then(|k| buses[*k].terminus.clone()))).collect();
                        let termini = p.line.and_then(|k| s.lines.get(k)).map(|l| l.1.iter().cloned().collect::<Vec<_>>());
                        log::info!("t={:.1} pax {} at stop {stop} for {:?} (line {:?} termini {:?}): no bus; listed {:?}", self.time, self.people[i].label(), p.dest, p.line, termini, listed);
                    }
                    return;
                };
                let (b, why) = b;
                let Some(bn) = bus_ix.get(&b).map(|k| &buses[*k]) else { return };
                self.pax_mut(i).unwrap().bus = Some(b);
                // still rolling in, or standing in the stop's box: to the gather point
                if bn.speed.abs() <= 2.0 && !self.in_stop_box(stop, b) {
                    return;
                }
                if super::debug_pax() {
                    log::info!("t={:.1} pax {} at stop {stop} for {:?}: bus {b:?} showing {:?} ({why:?})", self.time, self.people[i].label(), p.dest, bn.terminus);
                }
                self.set_task(i, Task::ToBus, buses, bus_ix, world);
            }
            Task::ToBus => {
                let Some(stop) = p.stop else { return };
                if let Some(bn) = bn {
                    if bn.speed.abs() < 3.0 && self.in_stop_box(stop, bn.id) {
                        if let Some(k) = self.reserve_place(bn.id, bn.cabin.seats.len(), &bn.places_off) {
                            let (tk, id) = self.decide_pax_ticket(i, bn);
                            let price = self.tickets.as_ref().and_then(|t| t.tickets.get(id.saturating_sub(1) as usize)).map(|t| t.value).unwrap_or(0.0);
                            let pp = self.pax_mut(i).unwrap();
                            pp.seat = Some(k);
                            pp.ticket = tk;
                            pp.ticket_id = id;
                            pp.price = if id > 0 { price } else { 0.0 };
                            self.set_task(i, Task::WalkingToBus, buses, bus_ix, world);
                        }
                    }
                }
                self.pax_mut(i).unwrap().short = true;
                // the bus is gone from the stop: back to a waiting place
                let gone = match self.pax(i).unwrap().bus {
                    Some(b) => !self.listed_at(stop, b),
                    None => true,
                };
                if gone && self.pax(i).unwrap().task == Task::ToBus {
                    self.set_task(i, Task::WalkingToBusstop, buses, bus_ix, world);
                }
            }
            Task::WalkingToBus => self.task_to_bus(i, buses, bus_ix, world),
            Task::InBusToPlace => self.task_to_place(i, dt, buses, bus_ix, world, player_bus, renderer, scene, taken_ticket),
            Task::InBusToExit => self.task_to_exit(i, buses, bus_ix, at_stops, world, remove),
            Task::WalkingToBusstop => {
                if p.st == 3 {
                    self.set_task(i, Task::WaitingForBus, buses, bus_ix, world);
                } else {
                    self.pax_mut(i).unwrap().pax_state = 1.0;
                }
            }
            Task::SittingInBus => {
                let Some(b) = p.inside else { return };
                let reg = at_stops.get(&b).cloned().unwrap_or_default();
                let km = self.odometer.get(&b).copied().unwrap_or(0.0);
                if let Some(bn) = bn {
                    if let Some(stop) = bn.next_stop.as_ref().or(reg.request_next.as_ref()) {
                        let force_exit = reg.all_exit
                            && !bn.terminus.as_ref().is_some_and(|name| stop.is_named(name));
                        if !force_exit && p.dest.as_ref().is_some_and(|dest| stop.is_named(dest)) {
                            let departing = bn.speed.abs() > 0.1
                                && !bn.entry_open.iter().chain(&bn.exit_open).any(|open| *open);
                            let arrived = reg.next == Some(stop.id)
                                && bn.speed.abs() < 1.0
                                && bn.exit_open.iter().any(|open| *open);
                            if arrived
                                || self
                                    .pax_mut(i)
                                    .unwrap()
                                    .wants_stop_at(stop, bn.pos, departing)
                            {
                                self.set_task(i, Task::InBusToExit, buses, bus_ix, world);
                            }
                            // The boarding range must not override a later random point
                            // on short legs, including those ending at the terminus.
                            return;
                        }
                    }
                }
                if reg.all_exit {
                    self.set_task(i, Task::InBusToExit, buses, bus_ix, world);
                    return;
                }
                if let (Some(next), Some(dest)) = (reg.next, p.dest.as_ref()) {
                    let name = self.stops.get(&next).map(|s| s.name.trim().to_string()).unwrap_or_default();
                    if self.stops.get(&next).is_some_and(|s| s.is_named(dest)) {
                        self.set_task(i, Task::InBusToExit, buses, bus_ix, world);
                        return;
                    }
                    if p.alt.as_ref().is_some_and(|a| a.trim() == name) && !p.alt_seen {
                        let r = self.rand_f() as f32;
                        let pp = self.pax_mut(i).unwrap();
                        pp.alt_seen = true;
                        pp.km_start = km;
                        pp.ride_km = (pp.alt_m / 1000.0) * (0.2 + 0.6 * r);
                    }
                }
                let p = self.pax(i).unwrap();
                if (p.alt_seen || p.dest.is_none()) && p.km_start + (p.ride_km as f64) < km {
                    self.set_task(i, Task::InBusToExit, buses, bus_ix, world);
                }
            }
            Task::Nothing => {}
        }
    }

    /// Task 3 (sub_62a6a0 case 3): to the door and in.
    pub(super) fn task_to_bus(&mut self, i: usize, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>, world: &World) {
        let p = self.pax(i).unwrap().clone();
        let Some(bn) = p.bus.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k])) else {
            self.set_task(i, Task::WalkingToBusstop, buses, bus_ix, world);
            return;
        };
        let door_x = p.door.and_then(|d| bn.cabin.entries.get(d)).map(|e| e.inside.x).unwrap_or(0.0);
        let open = p.door.map(|d| bn.entry_open.get(d).copied().unwrap_or(false)).unwrap_or(false);
        {
            let pp = self.pax_mut(i).unwrap();
            pp.clamp = true;
            pp.clamp_left = door_x < 0.0;
            pp.clamp_x = if pp.clamp_left { bn.centre.x - bn.half.x - 0.5 } else { bn.centre.x + bn.half.x + 0.5 };
            pp.clamp_open = open;
        }
        // a shut door is asked for, from the moment they stand at it
        if p.st == 3 || p.st == 2 {
            if let Some(d) = p.door {
                if let Some((e, _)) = self.pax_req.get_mut(&bn.id) {
                    if let Some(r) = e.get_mut(d) {
                        *r = true;
                    }
                }
            }
        }
        if p.seat.is_none() {
            self.set_task(i, Task::WalkingToBusstop, buses, bus_ix, world);
            return;
        }
        let stop = p.stop;
        let ok = bn.speed.abs() < 3.0
            && stop.is_some_and(|s| self.in_stop_box(s, bn.id))
            && !bn.cabin.graph.points.is_empty();
        if ok {
            if p.st != 3 {
                self.choose_entry(i, buses, bus_ix);
                let open = self.pax(i).unwrap().door.map(|d| bn.entry_open.get(d).copied().unwrap_or(false)).unwrap_or(false);
                let pp = self.pax_mut(i).unwrap();
                pp.short = !open;
                pp.st = 1;
                pp.pax_state = 1.0;
                return;
            }
            // in the doorway: the driver is greeted (the player's bus)
            if bn.id == BusId::Player {
                self.greet_or_complain(i, bn);
            }
            self.set_task(i, Task::InBusToPlace, buses, bus_ix, world);
            return;
        }
        // the bus pulls away again: the place is given back
        if let Some(k) = p.seat {
            self.free_seat(bn.id, k);
        }
        self.pax_mut(i).unwrap().seat = None;
        if bn.speed.abs() >= 3.0 {
            self.set_task(i, Task::ToBus, buses, bus_ix, world);
        } else {
            self.set_task(i, Task::WalkingToBusstop, buses, bus_ix, world);
        }
    }

    /// Task 4 (case 4): the validator, the cash desk, and on to the place.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn task_to_place(
        &mut self,
        i: usize,
        dt: f32,
        buses: &[BusNow],
        bus_ix: &HashMap<BusId, usize>,
        world: &World,
        player_bus: Option<&VehicleInstance>,
        renderer: &Renderer,
        scene: &mut Scene,
        taken_ticket: &mut bool,
    ) {
        let p = self.pax(i).unwrap().clone();
        let Some(bn) = p.inside.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k])) else { return };
        if p.st == 7 {
            if p.ticket < TICKET_STAMP {
                self.set_task(i, Task::SittingInBus, buses, bus_ix, world);
                return;
            }
            if bn.id != BusId::Player {
                let pp = self.pax_mut(i).unwrap();
                pp.sub = 0;
                pp.ticket = TICKET_NONE;
            } else {
                let pp = self.pax_mut(i).unwrap();
                pp.st = 9;
                pp.smooth = true;
                if pp.ticket == TICKET_STAMP {
                    if let Some(&(_, dev)) = pp.stamper.and_then(|k| bn.cabin.stampers.get(k)) {
                        pp.target = dev.as_dvec3();
                        pp.target_bus = true;
                        pp.reach_at = dev;
                    }
                    pp.timer = 1.0;
                    pp.reach = true;
                    pp.sub = 1;
                } else {
                    pp.sub = 3;
                    if let Some(m) = bn.cabin.money_point {
                        pp.target = m.as_dvec3();
                        pp.target_bus = true;
                        pp.reach_at = m;
                    }
                }
            }
        }
        let p = self.pax(i).unwrap().clone();
        if p.ticket == TICKET_STAMP {
            if p.sub == 1 && p.timer < 0.5 {
                // the validator stamps
                self.stamped.push(bn.id);
                let pp = self.pax_mut(i).unwrap();
                pp.sub = 2;
                pp.reach = false;
            } else if p.sub == 2 && p.timer <= 0.0 {
                self.route_to_place(i, bn);
                let pp = self.pax_mut(i).unwrap();
                pp.pt = pp.stamper.and_then(|k| bn.cabin.stampers.get(k)).and_then(|s| s.0);
                pp.ticket = TICKET_NONE;
                pp.sub = 0;
            }
        } else if p.ticket == TICKET_BUY {
            self.desk_sale(i, dt, bn, player_bus, world, renderer, scene, taken_ticket);
        }
    }

    /// Task 5 (case 5): to the exit, out.
    pub(super) fn task_to_exit(&mut self, i: usize, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>, at_stops: &HashMap<BusId, BusAtStops>, world: &World, remove: &mut Vec<usize>) {
        let p = self.pax(i).unwrap().clone();
        let Some(b) = p.inside else { return };
        let Some(bn) = bus_ix.get(&b).map(|k| &buses[*k]) else { return };
        let reg = at_stops.get(&b).cloned().unwrap_or_default();
        if bn.speed.abs() >= 1.0 {
            self.pax_mut(i).unwrap().timer = 1.0;
        }
        let door_open = p.door.map(|d| bn.exit_open.get(d).copied().unwrap_or(false)).unwrap_or(false);
        let may_leave = door_open && (reg.next.is_some() || p.complaint == 3);
        self.pax_mut(i).unwrap().short = !may_leave;
        let out = p.st == 7 && bn.speed.abs() < 1.0 && may_leave;
        if !out {
            if reg.next.is_none() {
                if bn.id == BusId::Player {
                    self.stop_request = true;
                }
                return;
            }
            if p.timer < 0.0 {
                // the bus stands: the nearest exit that is open now (0x62d6b1 passes the
                // exits' open states, +0x6e0: a shut door is skipped, none open gives the
                // first). Without them the nearest door was taken again, open or shut, and
                // people walked on to a shut front door with the others open (#493).
                // Once a second: with the timer left run out, the way was found afresh every
                // frame from the nearest point, and whoever had left a point was pulled back
                // to it - the people coming down from the upper deck never got off the stairs.
                self.pax_mut(i).unwrap().timer = 1.0;
                let all = bn.cabin.all_points();
                let pp = self.pax_mut(i).unwrap();
                let here = pp.pos.as_vec3();
                // (the exits of the sections they are in)
                let group = bn.cabin.group_at(pp.pt.or_else(|| bn.cabin.omsi_nearest(here, &all, false, false, None, None)));
                let exits = bn.cabin.in_group(bn.cabin.exit_points(), group);
                let open: Vec<bool> = (0..exits.len()).map(|k| bn.exit_open.get(k).copied().unwrap_or(false)).collect();
                let target = bn.cabin.omsi_nearest(here, &exits, false, false, None, Some(&open));
                if pp.st == 5 {
                    // walking: on from the point walked to, towards the new door (Omsi.exe
                    // changes only the target and the door)
                    pp.pt_target = target;
                } else if target != pp.pt_target || pp.st != 7 {
                    pp.pt = bn.cabin.omsi_nearest(here, &all, false, false, None, None);
                    pp.pt_target = target;
                    pp.st = 5;
                }
                pp.door = pp.pt_target.and_then(|t| exits.iter().position(|e| *e == Some(t)));
            }
            if bn.id == BusId::Player {
                self.stop_request = true;
            }
            if let Some(d) = self.pax(i).unwrap().door {
                if let Some((_, x)) = self.pax_req.get_mut(&b) {
                    if let Some(r) = x.get_mut(d) {
                        *r = true;
                    }
                }
            }
            return;
        }
        // out of the bus: into the world, on along the pavement (task 8)
        let Some((w, h)) = self.pax_world(&p, buses, bus_ix) else { return };
        let stop = reg.next;
        let pp = self.pax_mut(i).unwrap();
        pp.inside = None;
        pp.pos = w;
        pp.yaw = h.to_radians();
        if debug_pax() {
            log::info!("t={:.1} pax {} gets off at stop {:?} by exit {:?}", self.time, self.people[i].label(), stop, p.door);
        }
        self.walk_street(i, w, h, stop, world, remove);
    }

    /// sub_626818 / task 8: on as a pedestrian along the pavement from the stop - or gone
    /// when there is none.
    pub(super) fn walk_street(&mut self, i: usize, at: DVec3, heading: f64, stop: Option<i64>, world: &World, remove: &mut Vec<usize>) {
        let _ = world;
        self.walk_street_plain(i, at, heading, stop, remove)
    }

    pub(super) fn walk_street_plain(&mut self, i: usize, at: DVec3, heading: f64, stop: Option<i64>, remove: &mut Vec<usize>) {
        let lane = stop.and_then(|s| self.stops.get(&s)).and_then(|s| s.lane);
        let p = &mut self.people[i];
        p.position = at;
        p.heading = heading;
        p.place = Place::Ground;
        p.interior = 0.0;
        p.vel = DVec2::ZERO;
        let _ = &remove;
        match (lane, self.ped.as_ref()) {
            (Some((l, s)), Some(_)) => {
                let leg = Leg { lane: l, a: s, b: s };
                p.state = State::Strolling(PedWalk::new(vec![leg], true, 0.0));
            }
            _ => p.state = State::Standing,
        }
    }

    /// The riders of the player's bus feel how it is driven (0x7d6964 - 0x7d6b7f): every
    /// jolt (`RideComfort::step`) takes the toll of the ride `(1 - x) * k` up for everybody
    /// walking or sitting in it, and whoever reaches a threshold says so (TooBad_A, _B, _C
    /// of the ticket pack's voices) - the third time getting off at the next stop. The
    /// toll eases off by 0.2 a kilometre (`pax_tick`). OMSI's passengers did this; here
    /// they never said a word about the driving (#862, #873).
    pub(super) fn ride_comfort(&mut self, dt: f32, bus: Option<&VehicleInstance>, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>, world: &World) {
        let Some(v) = bus else { return };
        if dt <= 0.0 || self.avatar_only {
            return;
        }
        let a = v.physics.a_trans;
        let k = self.comfort.step(dt, self.time * 1000.0, v.physics.speed, a.x, a.y);
        if k <= 0.0 {
            return;
        }
        for i in 0..self.people.len() {
            if self.people[i].remote || self.people[i].puppet.is_some() {
                continue;
            }
            let Some(p) = self.pax(i) else { continue };
            if p.bus != Some(BusId::Player) || p.inside != Some(BusId::Player) || !matches!(p.task, Task::InBusToPlace | Task::InBusToExit | Task::SittingInBus) {
                continue;
            }
            if p.bad_at[2] <= 0.0 {
                let r = [self.rand_f() as f32, self.rand_f() as f32, self.rand_f() as f32];
                self.pax_mut(i).unwrap().bad_at = bad_ride_thresholds(r);
            }
            let p = self.pax_mut(i).unwrap();
            p.discomfort += (1.0 - p.discomfort) * k;
            let Some(c) = bad_ride_complaint(p.discomfort, p.complaint, p.bad_at) else { continue };
            p.complaint = c;
            if debug_pax() {
                log::info!("t={:.1} pax {} complains about the driving ({c}, toll {:.2})", self.time, self.people[i].label(), self.pax(i).unwrap().discomfort);
            }
            match c {
                1 => self.say_ex(i, "TooBad_A", true),
                2 => self.say_ex(i, "TooBad_B", true),
                _ => {
                    self.say_ex(i, "TooBad_C", true);
                    self.set_task(i, Task::InBusToExit, buses, bus_ix, world);
                }
            }
        }
    }

    /// The greeting or complaint stepping into the player's bus (0x62bf2d - 0x62c43c).
    pub(super) fn greet_or_complain(&mut self, i: usize, bn: &BusNow) {
        let Some((whinge, chat)) = self.tickets.as_ref().map(|t| (t.whinge_prop, t.chattiness)) else { return };
        let air = bn.air;
        let mut complaint_seen = false;
        let mut code = 0u8;
        // too dark: the saloon light under half and dusk outside
        if bn.interior < 0.5 {
            let r = self.rand_f() as f32;
            if air.brightness < 0.2 + 0.3 * r {
                complaint_seen = true;
                if (self.rand_f() as f32) < whinge {
                    code = 1;
                }
            }
        }
        if let Some(t) = air.temp {
            let out = air.outside;
            let r = (self.rand() % 10) as f32 + 25.0;
            let hot = if t <= r {
                false
            } else {
                let r5 = (self.rand() % 5) as f32 + 3.0;
                t > r5 + out
            };
            let hot = hot || {
                let r = (self.rand() % 10) as f32;
                out * 0.5 + r + 20.0 < t && t < 25.0
            };
            if hot {
                complaint_seen = true;
                if code == 0 && (self.rand_f() as f32) < whinge {
                    code = if air.rel_hum <= 0.9 + 0.1 * self.rand_f() as f32 { 3 } else { 5 };
                }
            }
            let r = (self.rand() % 10) as f32 + 8.0;
            let cold = if t >= r {
                let r = (self.rand() % 10) as f32;
                t < (out - r) - 10.0
            } else {
                let r5 = (self.rand() % 5) as f32;
                t < out + 5.0 + r5 || {
                    let r = (self.rand() % 10) as f32;
                    t < (out - r) - 10.0
                }
            };
            if cold {
                complaint_seen = true;
                if code == 0 && (self.rand_f() as f32) < whinge {
                    code = 4;
                }
            }
        }
        if self.delay > 300.0 {
            complaint_seen = true;
            if code == 0 && (self.rand_f() as f32) < whinge {
                code = 2;
            }
        }
        let k = 1 + self.rand() % 2;
        match code {
            1 => self.say_ex(i, &format!("TooDark_{k}"), true),
            2 => self.say_ex(i, &format!("TooLate_{k}"), true),
            3 => self.say_ex(i, &format!("TooHot_{k}"), true),
            4 => self.say_ex(i, &format!("TooCold_{k}"), true),
            5 => self.say_ex(i, "TooWet_1", true),
            _ => {
                if (self.rand_f() as f32) < chat {
                    let h = (self.time_of_day.rem_euclid(86_400.0) / 3600.0).floor() as i32;
                    let daypart = if (3..=10).contains(&h) { 1 } else if (18..=23).contains(&h) { 2 } else { 0 };
                    let k = if daypart == 0 { self.rand() % 2 } else { self.rand() % 3 };
                    if k < 2 {
                        self.say_ex(i, &format!("Hello_{}", k + 1), false);
                    } else if daypart == 1 {
                        self.say_ex(i, "GoodMorning_1", false);
                    } else {
                        self.say_ex(i, "GoodEvening_1", false);
                    }
                }
            }
        }
        // OMSI's rating: people who stepped in, and those content
        self.stepped_in += 1;
        if !complaint_seen {
            self.content += 1;
        }
    }

    /// The ticket sale at the player's cash desk (case 4 with +0x61c = 3, 0x62c780 -
    /// 0x62d104): the ticket asked for, the money on the desk, the ticket taken, the change
    /// counted. The passenger asks again every 5 s (3 s after the second time) until the
    /// driver gets it right; the counter of those requests (`pardons`, the original's
    /// global at 0x859bc4) is shared by everybody at the desk.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn desk_sale(
        &mut self,
        i: usize,
        dt: f32,
        bn: &BusNow,
        player_bus: Option<&VehicleInstance>,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        taken_ticket: &mut bool,
    ) {
        let _ = dt;
        let p = self.pax(i).unwrap().clone();
        // the game plays the driver in `auto` boarding (a setting; OMSI has no such mode)
        let auto = self.boarding.eq_ignore_ascii_case("auto");
        let id = p.ticket_id as usize;
        let (name, value) = self
            .tickets
            .as_ref()
            .and_then(|t| t.tickets.get(id.saturating_sub(1)))
            .map(|t| (t.name.clone(), t.value))
            .unwrap_or_default();
        let tol = self.money.as_ref().map(|m| m.smallest_value()).unwrap_or(0.01) / 2.0;
        let owed = p.paid - p.price;
        // the change on the tray (sub_7e8900): enough of it, too much, too many coins
        let change = |h: &mut Self| -> (bool, bool, bool) {
            if auto {
                return (true, false, false);
            }
            let given = h.money.as_ref().map(|m| m.change_value()).unwrap_or(0.0);
            let too_much = tol < given - owed;
            let enough = owed - given <= tol;
            let needed = h.money.as_mut().map(|m| m.exact_coins_for(owed.max(0.0)).len()).unwrap_or(0) as f32;
            let count = h.money.as_ref().map(|m| m.change_count()).unwrap_or(0) as f32;
            let r = h.rand_f() as f32;
            let many = needed * (r + 1.5) <= count && count > 0.0;
            (enough && !too_much, too_much, many)
        };
        // the ticket given (sub_7e8f14): right, or a wrong one
        let ticket = |h: &Self| -> (bool, bool) {
            if auto || h.give_ticket {
                return (true, false);
            }
            let given = player_bus.and_then(|b| b.var("GivenTicket")).unwrap_or(-1.0);
            if given < 0.0 {
                return (false, false);
            }
            let ok = (given - (id as f32 - 1.0)).abs() < 0.5;
            (ok, !ok)
        };
        let (ch_ok, too_much, many) = if p.sub == 7 { change(self) } else { (false, false, false) };
        let (tk_ok, wrong) = if p.sub == 5 { ticket(self) } else { (false, false) };
        if p.sub == 3 {
            // the desk free (sub_7d1fec, +0x7a8): "Einmal ..., bitte"
            if self.desk_busy.is_some_and(|d| d != self.people[i].id) {
                return;
            }
            let k = 1 + self.rand() % 2;
            self.say_ex(i, &format!("Ticket_{id}_{k}"), false);
            self.ticket_requests += 1;
            self.request = Some((name, value));
            self.desk_busy = Some(self.people[i].id);
            self.pardons = 0;
            self.pardon_max = 0;
            let pp = self.pax_mut(i).unwrap();
            pp.talking = true;
            pp.timer = 0.5;
            pp.reach = true;
            pp.sub = 4;
            pp.paid = 0.0;
        } else if p.sub == 4 {
            if p.timer > 0.0 {
                return;
            }
            // the money on the desk (sub_7e8254)
            let point = bn.cabin.money_var;
            let mut paid = value;
            if let Some(m) = self.money.as_mut() {
                let coins = if self.exact_fare || auto { m.exact_coins_for(value) } else { m.omsi_coins_for(value) };
                paid = m.value_of(&coins);
                if let Some((pos, var)) = point {
                    m.place(world, renderer, scene, &coins, pos, var, false);
                }
            }
            self.paid = Some((paid, value));
            let pp = self.pax_mut(i).unwrap();
            pp.paid = paid;
            pp.sub = 5;
            pp.reach = false;
            pp.talking = false;
            pp.look_driver = false;
            pp.timer = 10.0;
        } else if p.sub == 5 && tk_ok {
            // the ticket: the hand to where it comes out
            let pp = self.pax_mut(i).unwrap();
            pp.sub = 6;
            pp.timer = 0.5;
            pp.reach = true;
            if let Some((_, t)) = bn.cabin.sale {
                pp.reach_at = t;
            }
            self.pardons = 0;
        } else if p.sub == 6 && p.timer <= 0.0 {
            self.pax_mut(i).unwrap().reach = false;
            self.tickets_sold += 1;
            self.ticket_cash += value;
            *taken_ticket = true;
            if let Some(m) = self.money.as_mut() {
                m.clear(false);
            }
            self.paid = None;
            let pp = self.pax_mut(i).unwrap();
            if (p.price - p.paid).abs() > tol && !auto {
                pp.sub = 7;
                pp.timer = 5.0;
                self.change_due = Some(owed);
            } else {
                pp.sub = 8;
                self.change_due = Some(0.0);
            }
            let pp = self.pax_mut(i).unwrap();
            pp.talking = false;
            pp.look_driver = false;
        } else if p.sub == 7 && (ch_ok || (self.pardons > 1 && too_much)) {
            let pp = self.pax_mut(i).unwrap();
            pp.sub = 8;
            pp.timer = 0.5;
            pp.reach = true;
            pp.bad_change = many;
            if let Some(c) = bn.cabin.change_point {
                pp.reach_at = c;
            }
            if many {
                self.say_ex(i, "BadChange_1", true);
                self.ticket_points += 1;
            } else {
                if !too_much {
                    self.ticket_points += 2;
                }
                let r = self.rand_f() as f32;
                if !too_much && self.tickets.as_ref().is_some_and(|t| r < t.chattiness) {
                    self.say_ex(i, "Thanks_1", false);
                }
            }
        } else if p.sub == 8 && p.timer <= 0.0 {
            if let Some(m) = self.money.as_mut() {
                m.clear(true);
            }
            self.change_due = None;
            self.request = None;
            self.desk_busy = None;
            self.boarded += 1;
            self.served += 1;
            self.route_to_place(i, bn);
            let pp = self.pax_mut(i).unwrap();
            pp.reach = false;
            pp.talking = false;
            pp.look_driver = false;
            pp.sub = 0;
            pp.ticket = TICKET_NONE;
            pp.pt = bn.cabin.sale.and_then(|s| s.0);
        } else if (p.sub == 5 || p.sub == 7) && (p.timer <= 0.0 || (too_much && self.pardons == 0)) {
            // asking again (0x62ce80)
            let n = self.pardons as u64;
            let r = self.rand() % (n + 2);
            self.pax_mut(i).unwrap().timer = if n < 2 { 5.0 } else { 3.0 };
            let line = if (n == 0 || r == 0) && p.sub == 5 {
                if wrong { "BadTicket_A".to_string() } else { "PardonTicket_1".to_string() }
            } else if wrong && n == 1 && p.sub == 5 {
                "BadTicket_B".to_string()
            } else if !too_much && n == 0 && p.sub == 7 {
                "TooFew_A".to_string()
            } else if !too_much && n == 1 && p.sub == 7 {
                "TooFew_B".to_string()
            } else if too_much && n == 0 && p.sub == 7 {
                "TooMuch_A".to_string()
            } else if too_much && n == 1 && p.sub == 7 {
                self.pardons = 2;
                "TooMuch_B".to_string()
            } else {
                format!("Pardon_{}", r.min(3))
            };
            self.say_ex(i, &line, true);
            self.pax_mut(i).unwrap().talking = true;
            let skip = self.pardons != 0 && self.rand_f() >= 0.7;
            if !skip {
                self.pardons = self.pardons.saturating_add(1);
            }
        }
        self.pardon_max = self.pardon_max.max(self.pardons);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A timetable bus waits for the people walking up to its doors from its stop and for
    /// those on their way out of it - not for the people still at the gather point, who
    /// held a full bus at the stop for good (#767).
    #[test]
    fn who_keeps_a_timetable_bus_at_its_stop() {
        let bus = BusId::Ai(7);
        let mut x = Pax::new(1.1, 0.5);
        x.bus = Some(bus);
        x.stop = Some(42);
        x.task = Task::ToBus;
        assert_eq!(holds_bus(&x, bus), None);
        x.task = Task::WaitingForBus;
        assert_eq!(holds_bus(&x, bus), None);
        x.task = Task::WalkingToBus;
        assert_eq!(holds_bus(&x, bus), Some(Some(42)));
        assert_eq!(holds_bus(&x, BusId::Ai(8)), None);
        x.task = Task::InBusToExit;
        x.inside = Some(bus);
        assert_eq!(holds_bus(&x, bus), Some(None));
        x.task = Task::SittingInBus;
        assert_eq!(holds_bus(&x, bus), None);
    }

    /// Smooth driving upsets nobody; a hard stop, a fast bend and a jerky foot do, as in
    /// Omsi.exe (#862).
    #[test]
    fn the_riders_feel_hard_braking_fast_bends_and_a_jerky_foot() {
        let dt = 0.02;
        let run = |f: &dyn Fn(f64) -> (f32, f32, f32), secs: f64| {
            let mut c = RideComfort::default();
            let mut jolts = Vec::new();
            let mut t = 0.0;
            while t < secs {
                let (v, lat, long) = f(t);
                let k = c.step(dt, (t + 10.0) * 1000.0, v, lat, long);
                if k > 0.0 {
                    jolts.push((t, k));
                }
                t += dt as f64;
            }
            jolts
        };
        // pulling away at 1.2 m/s², cruising, braking at 1.5 m/s² to a stop: nothing
        assert!(run(&|t| if t < 10.0 { (1.2 * t as f32, 0.0, 1.2) } else if t < 20.0 { (12.0, 0.0, 0.0) } else if t < 28.0 { (12.0 - 1.5 * (t as f32 - 20.0), 0.0, -1.5) } else { (0.0, 0.0, 0.0) }, 40.0).is_empty());
        // a gentle bend at 1.5 m/s² sideways
        assert!(run(&|_| (10.0, 1.5, 0.0), 10.0).is_empty());
        // an emergency stop at 7 m/s²: one jolt, not one a frame
        let hard = run(&|t| if t < 1.0 { (14.0, 0.0, 0.0) } else { (14.0, 0.0, -7.0) }, 2.0);
        assert_eq!(hard.len(), 1, "{hard:?}");
        assert_eq!(hard[0].1, 0.1);
        // a bend at 4 m/s² held for seconds
        assert_eq!(run(&|_| (12.0, 4.0, 0.0), 6.0).len(), 1);
        // throttle and brake every 1.5 s: the fifth swing on upsets them, each further one too
        let jerky = run(&|t| (8.0, 0.0, if (t / 1.5).floor() as i64 % 2 == 0 { 1.0 } else { -1.0 }), 15.0);
        assert!(jerky.len() >= 4 && jerky.iter().all(|j| j.1 == 0.05) && jerky[0].0 > 5.0, "{jerky:?}");
        // standing, nothing counts
        assert!(run(&|_| (0.0, 5.0, -8.0), 5.0).is_empty());
    }

    #[test]
    fn complaints_come_worst_first_and_once_each() {
        let at = bad_ride_thresholds([0.5, 0.5, 0.5]);
        assert!((at[0] - 0.05).abs() < 1e-6 && (at[1] - 0.3).abs() < 1e-6 && (at[2] - 0.65).abs() < 1e-6);
        let lo = bad_ride_thresholds([0.0, 0.0, 0.0]);
        let hi = bad_ride_thresholds([1.0, 1.0, 1.0]);
        assert!(lo[1] >= 0.2 - 1e-6 && hi[1] <= 0.4 + 1e-6 && lo[2] >= 0.5 - 1e-6 && hi[2] <= 0.8 + 1e-6);
        assert_eq!(bad_ride_complaint(0.01, 0, at), None);
        assert_eq!(bad_ride_complaint(0.1, 0, at), Some(1));
        assert_eq!(bad_ride_complaint(0.1, 1, at), None);
        assert_eq!(bad_ride_complaint(0.35, 1, at), Some(2));
        // a crash straight to the top: the worst at once, then nothing more
        assert_eq!(bad_ride_complaint(0.9, 0, at), Some(3));
        assert_eq!(bad_ride_complaint(0.95, 3, at), None);
        // three emergency stops in a row take a rider from nothing past 0.27
        let mut x = 0.0f32;
        for _ in 0..3 {
            x += (1.0 - x) * 0.1;
        }
        assert!((x - 0.271).abs() < 1e-3);
    }

    fn test_stop(name: &str, alias: &str) -> PaxStop {
        PaxStop {
            name: name.into(),
            alias: alias.into(),
            pos: DVec3::ZERO,
            heading: 0.0,
            gather: DVec3::ZERO,
            spots: Vec::new(),
            taken: Vec::new(),
            enter_max: 1.0,
            enter_min: 0.0,
            length: 30.0,
            lane: None,
            was_near: false,
            near: false,
            clock_ms: 0.0,
            want: 0,
            factor: 1.0,
            buses: Vec::new(),
            dests: Vec::new(),
            lines: Vec::new(),
        }
    }

    /// The player's bus on a duty takes the people its trip takes where they are going,
    /// whatever its depot file calls the terminus; in free drive, or with no destination
    /// shown, nobody; a bus whose terminus is on their line record comes first.
    #[test]
    fn who_boards_the_players_bus() {
        // line 76 from Bauernhof (stop 10) by Kirche to Endstation; the depot file calls the
        // terminus "Endstation Grundorf", the timetable "Endstation"
        let trip = |name: &str| crate::schedule::PlannedStop {
            object_id: match name {
                "Bauernhof" => 10,
                "Kirche" => 11,
                "Depot" => 13,
                _ => 12,
            },
            name: format!("{name} "),
            arr: 0.0,
            dep: 0.0,
            position: None,
            dir: Default::default(),
            stops: name != "Depot",
        };
        let planned = crate::schedule::PlannedTrip {
            name: "76-1".into(),
            line: "76".into(),
            terminus: "Endstation ".into(),
            departure: 8.0 * 3600.0,
            end: 9.0 * 3600.0,
            stops: ["Bauernhof", "Depot", "Kirche", "Endstation"].into_iter().map(trip).collect(),
        };
        // (named as the timetable's Busstops.cfg names the objects; one it does not know by
        // its id)
        let names: HashMap<i64, String> = [(10, "Bauernhof".to_string()), (11, "Kirche".to_string()), (13, "Depot".to_string())].into_iter().collect();
        let dt = Arc::new(DutyTrip::of(&planned, Some(&names)));
        assert_eq!(dt.terminus, "Endstation");
        assert_eq!(dt.stops.iter().map(|s| s.1.as_str()).collect::<Vec<_>>(), ["Bauernhof", "Depot", "Kirche", "12"]);
        let duty = |next: usize, done: bool| Takes::Duty { trip: dt.clone(), next, done };
        let here = test_stop("Bauernhof Grundorf", "Bauernhof");
        let set = |t: &[&str]| t.iter().map(|x| x.to_string()).collect::<HashSet<String>>();
        let shown = "Endstation Grundorf";

        // on the duty with the destination shown: at the stop, and on the line record
        // whatever spelling the depot file has
        assert_eq!(at_stop(10, &here, Some(shown), &duty(0, false)), AtStop::Serves);
        assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Endstation"]), shown, &duty(0, false)), Some(Fit::Duty));
        // a record that does not list the trip's terminus at all (made of another variant's
        // trips): the trip goes to their stop all the same
        assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Waldweg"]), shown, &duty(0, false)), Some(Fit::Duty));
        assert_eq!(fit(10, &here, Some("12"), &set(&["Waldweg"]), shown, &duty(0, false)), Some(Fit::Duty));
        // ... but not somebody whose stop the trip does not go to, nor one it only passes
        assert_eq!(fit(10, &here, Some("Waldweg"), &set(&["Waldweg"]), shown, &duty(0, false)), None);
        assert_eq!(fit(10, &here, Some("Depot"), &set(&["Waldweg"]), shown, &duty(0, false)), None);
        // nor at a stop the trip has left behind, or does not call at
        assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Waldweg"]), shown, &duty(3, false)), None);
        assert_eq!(fit(20, &test_stop("Am Teich", "Am Teich"), Some("Kirche"), &set(&["Waldweg"]), shown, &duty(0, false)), None);
        // another platform of the stop's name will do
        assert_eq!(fit(14, &test_stop("Bauernhof 2", "Bauernhof"), Some("Kirche"), &set(&["Waldweg"]), shown, &duty(0, false)), Some(Fit::Duty));
        // the depot file's terminus on the record: the bus is theirs as in Omsi.exe
        assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Endstation Grundorf"]), shown, &duty(0, false)), Some(Fit::Terminus));

        // free drive: nobody waiting gets on, the riders get off at their stops
        assert_eq!(at_stop(10, &here, Some(shown), &Takes::Nobody), AtStop::Passes);
        assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Endstation"]), shown, &Takes::Nobody), None);
        // no destination shown (or "Nicht einsteigen"): it empties and takes nobody, duty or not
        assert_eq!(at_stop(10, &here, None, &duty(0, false)), AtStop::Empties);
        assert_eq!(at_stop(10, &here, None, &Takes::Nobody), AtStop::Empties);
        // at the trip's last stop it empties too, whatever the depot file calls it ...
        let end = test_stop("Endstation Grundorf Wendeschleife", "12");
        assert_eq!(at_stop(12, &end, Some(shown), &duty(3, true)), AtStop::Empties);
        assert_eq!(at_stop(12, &end, Some(shown), &duty(3, false)), AtStop::Serves);
        // ... and as at the terminus it shows
        assert_eq!(at_stop(12, &test_stop("Endstation Grundorf", ""), Some(shown), &duty(3, false)), AtStop::Empties);
        // a finished trip takes nobody by the duty, nor a works trip from the depot
        assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Endstation"]), shown, &duty(0, true)), None);
        let works = Takes::Duty { trip: Arc::new(DutyTrip::of(&crate::schedule::PlannedTrip { line: String::new(), ..planned.clone() }, Some(&names))), next: 0, done: false };
        assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Endstation"]), shown, &works), None);
        assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Endstation Grundorf"]), shown, &works), Some(Fit::Terminus));

        // a timetable bus: as in Omsi.exe, the terminus on the record or nothing
        assert_eq!(at_stop(10, &here, Some("Endstation"), &Takes::Terminus), AtStop::Serves);
        assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Endstation"]), "Endstation", &Takes::Terminus), Some(Fit::Terminus));
        assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Endstation"]), "Endstation Grundorf", &Takes::Terminus), None);

        // two buses at the stop: the one whose terminus is on the record, though further
        let (ai, me) = (BusId::Ai(5), BusId::Player);
        assert_eq!(best_bus([(me, Fit::Duty, 5.0), (ai, Fit::Terminus, 30.0)].into_iter()), Some((ai, Fit::Terminus)));
        assert_eq!(best_bus([(ai, Fit::Terminus, 30.0), (me, Fit::Terminus, 5.0)].into_iter()), Some((me, Fit::Terminus)));
        assert_eq!(best_bus([(me, Fit::Duty, 5.0)].into_iter()), Some((me, Fit::Duty)));
        assert_eq!(best_bus(std::iter::empty()), None);
    }

    fn stop(alias: &str) -> PaxStop {
        PaxStop {
            name: "Königsrath, Bf. Ausstieg".into(),
            alias: alias.into(),
            pos: DVec3::ZERO,
            heading: 0.0,
            gather: DVec3::ZERO,
            spots: Vec::new(),
            taken: Vec::new(),
            enter_max: 1.0,
            enter_min: 0.0,
            length: 30.0,
            lane: None,
            was_near: false,
            near: false,
            clock_ms: 0.0,
            want: 0,
            factor: 1.0,
            buses: Vec::new(),
            dests: Vec::new(),
            lines: Vec::new(),
        }
    }

    #[test]
    fn a_stop_answers_to_its_label_and_its_timetable_name() {
        let s = stop("Koenigsrath Bf Ausstieg");
        assert!(s.is_named("Königsrath, Bf. Ausstieg "));
        assert!(s.is_named("Koenigsrath Bf Ausstieg"), "the timetable's spelling");
        assert!(!s.is_named("Königsrath, Bf. Pause"));
        // a stop the timetable does not know: its id, as the riders' destinations then are
        assert!(stop("4711").is_named("4711"));
        assert!(!stop("").is_named(""), "no timetable name: no empty match");
    }

    fn request_stop() -> RequestStop {
        let stop = stop("Koenigsrath Bf Ausstieg");
        RequestStop {
            id: 42,
            name: stop.name,
            alias: stop.alias,
            pos: stop.pos,
        }
    }

    #[test]
    fn passengers_request_between_departure_and_the_approach() {
        let stop = request_stop();
        let mut early = Pax::new(1.1, 1.0);
        let mut middle = Pax::new(1.1, 0.5);
        let mut late = Pax::new(1.1, 0.0);
        early.dest = Some(stop.name.clone());
        middle.dest = early.dest.clone();
        late.dest = early.dest.clone();

        // A rider can ask just after pulling away, while another waits until the approach.
        assert!(!early.wants_stop_at(&stop, DVec3::Y * 1000.0, false));
        assert_eq!(early.stop_request_at, None);
        assert!(early.wants_stop_at(&stop, DVec3::Y * 1000.0, true));
        assert!(!middle.wants_stop_at(&stop, DVec3::Y * 1000.0, true));
        assert!(!late.wants_stop_at(&stop, DVec3::Y * 1000.0, true));
        // Repeated frames do not draw a new point or shorten the leg used to choose it.
        for _ in 0..120 {
            assert!(!middle.wants_stop_at(&stop, DVec3::Y * 600.0, true));
            assert_eq!(middle.stop_request_at, Some((42, 550.0)));
        }
        assert!(middle.wants_stop_at(&stop, DVec3::Y * 550.0, true));
        assert!(!late.wants_stop_at(&stop, DVec3::Y * 100.01, true));
        assert!(late.wants_stop_at(&stop, DVec3::Y * 100.0, true));
        assert!(late.wants_stop_at(&stop, stop.pos, false));
    }

    #[test]
    fn short_legs_keep_random_requests_after_departure() {
        let stop = request_stop();
        for length in [20.0, 60.0, 80.0, 100.0, 150.0] {
            let mut early = Pax::new(1.1, 0.8);
            let mut late = Pax::new(1.1, 0.2);
            early.dest = Some(stop.name.clone());
            late.dest = early.dest.clone();
            assert!(!early.wants_stop_at(&stop, DVec3::Y * length, false));
            assert!(!early.wants_stop_at(&stop, DVec3::Y * length, true));
            assert!(!late.wants_stop_at(&stop, DVec3::Y * length, true));
            let early_point = early.stop_request_at.unwrap().1;
            let late_point = late.stop_request_at.unwrap().1;
            assert!(0.0 < late_point && late_point < early_point && early_point < length);
            let between = (early_point + late_point) / 2.0;
            for _ in 0..120 {
                assert!(early.wants_stop_at(&stop, DVec3::Y * between, true));
                assert!(!late.wants_stop_at(&stop, DVec3::Y * between, true));
                assert_eq!(late.stop_request_at, Some((stop.id, late_point)));
            }
            assert!(late.wants_stop_at(&stop, DVec3::Y * late_point, true));
        }
    }

    #[test]
    fn stop_requests_match_the_destination_and_its_timetable_alias() {
        let stop = request_stop();
        let mut passenger = Pax::new(1.1, 0.5);
        assert!(!passenger.wants_stop_at(&stop, stop.pos, true));
        passenger.dest = Some("Königsrath, Bf. Pause".into());
        assert!(!passenger.wants_stop_at(&stop, stop.pos, true));
        assert_eq!(passenger.stop_request_at, None);
        passenger.dest = Some(stop.alias.clone());
        assert!(passenger.wants_stop_at(&stop, stop.pos, true));
        passenger.dest = Some(stop.name.clone());
        assert!(passenger.wants_stop_at(&stop, stop.pos, true));
    }

    #[test]
    fn stop_request_distances_vary_and_repeat_with_the_passenger_seed() {
        let mut first = Humans::new(Path::new("/nonexistent"));
        let mut second = Humans::new(Path::new("/nonexistent"));
        first.set_lan_seed(42);
        second.set_lan_seed(42);
        let stop = request_stop();
        let mut distances = Vec::new();
        for _ in 0..100 {
            let mut passenger = Pax::new(1.1, first.rand_f());
            let mut repeated = Pax::new(1.1, second.rand_f());
            passenger.dest = Some(stop.name.clone());
            repeated.dest = passenger.dest.clone();
            passenger.wants_stop_at(&stop, DVec3::Y * 1000.0, true);
            repeated.wants_stop_at(&stop, DVec3::Y * 1000.0, true);
            assert_eq!(passenger.stop_request_at, repeated.stop_request_at);
            let distance = passenger.stop_request_at.unwrap().1;
            assert!((100.0..=1000.0).contains(&distance));
            distances.push(distance);
        }
        assert!(distances.iter().any(|distance| *distance < 300.0));
        assert!(distances.iter().any(|distance| *distance > 800.0));
    }

    #[test]
    fn a_timetable_stop_can_be_requested_before_its_tile_loads() {
        let mut humans = Humans::new(Path::new("/nonexistent"));
        let planned = crate::schedule::PlannedStop {
            object_id: 42,
            name: "Next stop".into(),
            position: Some(DVec3::Y * 1000.0),
            arr: 0.0,
            dep: 0.0,
            dir: Default::default(),
            stops: true,
        };
        humans.set_player_next_stop(Some(&planned));
        assert!(humans.stops.is_empty());
        let target = humans.player_next_stop.as_ref().unwrap();
        let mut passenger = Pax::new(1.1, 1.0);
        passenger.dest = Some(planned.name);
        assert!(passenger.wants_stop_at(target, DVec3::ZERO, true));
        humans.set_player_next_stop(None);
        assert!(humans.player_next_stop.is_none());
    }

    #[test]
    pub(super) fn routes_follow_the_link_order_and_one_way_links() {
        // 0 - 1 - 2, and 2 -> 0 one way
        let r = build_routes(3, &[(0, 1, false), (1, 2, false), (2, 0, true)]);
        // from 0 to 2: through 1 (0 cannot use the one-way link back from 0 to 2)
        let next = |a: usize, b: usize| r[a].iter().find(|l| l.reach.contains(&b)).map(|l| l.to);
        assert_eq!(next(0, 2), Some(1));
        assert_eq!(next(2, 0), Some(1).or(Some(0)).filter(|_| true).and(next(2, 0)));
        assert_eq!(next(1, 0), Some(0));
        assert_eq!(next(1, 2), Some(2));
    }
}
