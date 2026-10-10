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
pub enum Task {
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
    pub fn name(self) -> &'static str {
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

/// Seconds a person stands at a shut door of the bus they want before going back to wait
/// at the stop (`Pax::door_since`).
pub const DOOR_GIVE_UP: f64 = 25.0;

/// One continuous wait at a shut entry: when it began and whether it is past `DOOR_GIVE_UP`.
/// An open door ends it, so a later closure starts a fresh wait.
pub fn shut_door_wait(previous: Option<f64>, now: f64, at_shut_door: bool) -> (Option<f64>, bool) {
    let since = if at_shut_door { Some(previous.unwrap_or(now)) } else { None };
    let expired = since.is_some_and(|start| now - start > DOOR_GIVE_UP);
    (since, expired)
}

/// The ticket a passenger has (+0x61c): nothing to do, a ticket to stamp, one to buy.
pub const TICKET_NONE: u8 = 0;
pub const TICKET_STAMP: u8 = 2;
pub const TICKET_BUY: u8 = 3;

/// One passenger's state: the fields of the original's human the tasks use.
#[derive(Debug, Clone)]
pub struct Pax {
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
    /// Since when (`time`) the person has stood at the shut door of the bus they want
    /// (`DOOR_GIVE_UP`), and the bus they then left standing: not walked to again before it
    /// opens a door.
    pub door_since: Option<f64>,
    pub shunned: Option<BusId>,
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
    /// Late for a bus pulling in to the stop: walking up to it, then hurrying for it (see
    /// `runners`).
    pub late: Option<Late>,
}

impl Pax {
    pub fn new(walk_speed: f32, stop_request_random: f64) -> Pax {
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
            door_since: None,
            shunned: None,
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
            late: None,
        }
    }

    pub fn wants_stop_at(&mut self, stop: &RequestStop, bus_pos: DVec3, departing: bool) -> bool {
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
pub struct RequestStop {
    pub id: i64,
    pub name: String,
    pub alias: String,
    pub pos: DVec3,
}

impl RequestStop {
    pub fn is_named(&self, name: &str) -> bool {
        let name = name.trim();
        name == self.name.trim() || (!self.alias.is_empty() && name == self.alias.trim())
    }
}

/// The room height outside a vehicle (+0x668 = 50).
pub const OUTSIDE_ROOM: f32 = 50.0;

/// A waiting place of a stop (a `[passpos]` of an object near it, sub_620c0c).
#[derive(Debug, Clone)]
pub struct WaitSpot {
    /// The `[passpos]` point (world): the feet, or a seated person's hip.
    pub pos: DVec3,
    /// Heading (degrees, the world's).
    pub face: f64,
    /// Seat height (+0x20); a seat when not 0.
    pub height: f32,
}

/// What Omsi.exe keeps of a bus stop for the people (the station record, sub_620058).
pub struct PaxStop {
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
    pub fn is_named(&self, name: &str) -> bool {
        let name = name.trim();
        name == self.name.trim() || (!self.alias.is_empty() && name == self.alias.trim())
    }
}

/// How the player's bus is driven, as Omsi.exe watches it for the riders (0x7d5124,
/// 0x7d65d4 - 0x7d6b7f): the longitudinal acceleration eased over a tenth of a second, the
/// lateral one over a second (both weighed down below 1 m/s), and the swings of the first
/// between +0.2 and -0.2 m/s² (a jerky right foot).
#[derive(Debug, Clone, Default)]
pub struct RideComfort {
    /// +0x780 and +0x784 (m/s²).
    pub fast_long: f32,
    pub slow_lat: f32,
    /// The last swing went up (+0x79c), when (+0x794, ms) and how many came in a row (+0x798).
    pub up: bool,
    pub swing_ms: f64,
    pub swings: u32,
    /// The last hard bend or braking (+0x790, ms).
    pub hard_ms: f64,
}

impl RideComfort {
    /// One frame of the bus (`speed` forward and the body's acceleration `lat` to the right
    /// and `long` forward, m/s and m/s²): how much this frame upsets the riders - 0, 0.05
    /// for the fifth and every further swing of the throttle and brake less than 4 s apart,
    /// 0.1 for a bend taken at over 3 m/s² or braking or pulling away at over 5 m/s² (once
    /// a second at most).
    pub fn step(&mut self, dt: f32, now_ms: f64, speed: f32, lat: f32, long: f32) -> f32 {
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
pub fn bad_ride_thresholds(r: [f32; 3]) -> [f32; 3] {
    let a = 0.1 * r[0];
    [a, 0.1 + a.max(0.1) + 0.2 * r[1], 0.5 + 0.3 * r[2]]
}

/// The complaint a rider says as the ride's toll `x` reaches their next threshold
/// (0x7d6a22 - 0x7d6b7f; the worst first, each only once): 1, 2, 3 or none.
pub fn bad_ride_complaint(x: f32, said: u8, at: [f32; 3]) -> Option<u8> {
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
pub struct BusAtStops {
    pub next: Option<i64>,
    pub request_next: Option<RequestStop>,
    pub near: Vec<i64>,
    pub all_exit: bool,
}

/// The trip the player's duty has the bus on, as the people at the stops see it.
#[derive(Debug, Clone, PartialEq)]
pub struct DutyTrip {
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
    /// Duty trip `trip` (omsi-app's `schedule::PlannedTrip`), its stops named as `names`
    /// (`Schedule::stop_names`) has them: by the object's id where it does not know the
    /// object, as the stops' targets are.
    pub fn of(trip: &TripPlan, names: Option<&HashMap<i64, String>>) -> DutyTrip {
        let name = |s: &StopPlan| match names {
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

/// A trip of the timetable as the passengers need it (omsi-app's `schedule::PlannedTrip`).
#[derive(Debug, Clone)]
pub struct TripPlan {
    pub name: String,
    pub line: String,
    pub terminus: String,
    pub departure: f64,
    pub stops: Vec<StopPlan>,
}

/// A stop of a trip as the passengers need it (omsi-app's `schedule::PlannedStop`).
#[derive(Debug, Clone)]
pub struct StopPlan {
    /// The stop object.
    pub object_id: i64,
    /// Its name in the timetable.
    pub name: String,
    /// The trip stops there (not a station it passes).
    pub stops: bool,
}

/// Whom a bus takes on at the stops (see `at_stop` and `fit`).
#[derive(Debug, Clone)]
pub enum Takes {
    /// Those whose line record lists its terminus, as in Omsi.exe: a timetable bus.
    Terminus,
    /// The player's bus on a duty: those as well whom its trip takes where they are going,
    /// however the bus's depot file spells the terminus. `next` is the stop of the trip the
    /// duty is due at, `done` that the trip has reached its last stop.
    Duty { trip: Arc<DutyTrip>, next: usize, done: bool },
    /// Nobody waiting: another player's bus (their game boards it), a bus the player left
    /// standing (its riders get off as ever).
    Nobody,
}

/// What a bus near a stop does there (sub_61f238 from 0x61f3e3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtStop {
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
pub fn at_stop(id: i64, stop: &PaxStop, terminus: Option<&str>, takes: &Takes) -> AtStop {
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
pub enum Fit {
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
pub fn best_bus(buses: impl Iterator<Item = (BusId, Fit, f64)>) -> Option<(BusId, Fit)> {
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
pub fn fit(id: i64, stop: &PaxStop, dest: Option<&str>, termini: &HashSet<String>, terminus: &str, takes: &Takes) -> Option<Fit> {
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
pub struct RouteLink {
    pub to: usize,
    pub reach: Vec<usize>,
    pub link: usize,
    pub walk_back: bool,
}

/// The routing tables of a path network as sub_72410c builds them after loading: from
/// every point a depth-first walk, each point reached noting every point visited so far as
/// reached through its link back to where it came from. A one-way link a -> b is walked
/// back only from b.
pub fn build_routes(n: usize, links: &[(i32, i32, bool)]) -> Vec<Vec<RouteLink>> {
    let mut adj: Vec<Vec<RouteLink>> = vec![Vec::new(); n];
    for (k, &(a, b, oneway)) in links.iter().enumerate() {
        if a < 0 || b < 0 || a as usize >= n || b as usize >= n {
            continue;
        }
        let (a, b) = (a as usize, b as usize);
        adj[a].push(RouteLink { to: b, reach: vec![b], link: k, walk_back: !oneway });
        adj[b].push(RouteLink { to: a, reach: vec![a], link: k, walk_back: true });
    }
    pub fn visit(adj: &mut Vec<Vec<RouteLink>>, p: usize, from: Option<usize>, stack: &mut Vec<usize>) {
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
/// (`Some(Some(s))`: only while the bus serves that stop). Anybody else, not - but for
/// somebody hurrying up to it from along the street (`Late::holds`), for a few seconds.
pub fn holds_bus(x: &Pax, bus: BusId) -> Option<Option<i64>> {
    if x.bus != Some(bus) {
        return None;
    }
    match x.task {
        Task::InBusToExit if x.inside == Some(bus) => Some(None),
        // (somebody late who has hurried for longer than a bus waits: not any more)
        Task::WalkingToBus => x.late.is_none_or(|l| l.holds()).then_some(x.stop),
        Task::ToBus | Task::WalkingToBusstop if x.inside.is_none() && x.late.is_some_and(|l| l.holds()) => Some(x.stop),
        _ => None,
    }
}

/// sub_7f3a24: the distance with the height difference weighed by `w` (5 everywhere).
pub fn weighted_dist(a: Vec3, b: Vec3, w: f32) -> f32 {
    let d = a - b;
    Vec3::new(d.x, d.y, d.z * w).length()
}

impl Cabin {
    /// sub_72506c: the point of `list` nearest `p` (height weighed by 5). `level`: only
    /// points at most 2 m below `p` and not above it. `open`/`flags` (entries): a shut
    /// door counts only with a button; `avoid`: a passenger buying a ticket skips
    /// `{noticketsale}` doors. Nothing found: the first of the list, or the search again
    /// without `avoid`.
    pub fn omsi_nearest(&self, p: Vec3, list: &[Option<usize>], avoid: bool, level: bool, flags: Option<&[(bool, bool)]>, open: Option<&[bool]>) -> Option<usize> {
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
    pub fn nearest_stamper(&self, p: Vec3) -> Option<usize> {
        let at = |s: &(Option<usize>, Vec3)| s.0.and_then(|k| self.graph.points.get(k).copied()).unwrap_or(s.1);
        let d = |k: usize| weighted_dist(p, at(&self.stampers[k]), 5.0);
        let group = self.group_at(self.omsi_nearest(p, &self.all_points(), false, false, None, None));
        (0..self.stampers.len())
            .filter(|&k| self.groups <= 1 || self.group_at(self.stampers[k].0) == group)
            .min_by(|&a, &b| d(a).total_cmp(&d(b)))
    }

    /// The group of sections path point `p` lies in (see `groups`).
    pub fn group_at(&self, p: Option<usize>) -> Option<usize> {
        p.and_then(|q| self.point_group.get(q).copied())
    }

    /// The points of `list` in group `g`, the others left out (None): where a trailer hangs
    /// on that nobody walks into from the bus (#718), a passenger keeps to the sections they
    /// are in - the doors and devices of the others are out of reach. One group: `list`.
    pub fn in_group(&self, list: Vec<Option<usize>>, g: Option<usize>) -> Vec<Option<usize>> {
        match g {
            Some(g) if self.groups > 1 => list.into_iter().map(|p| p.filter(|&q| self.point_group.get(q) == Some(&g))).collect(),
            _ => list,
        }
    }

    /// sub_723fac: the next point from `from` towards `to` and the link taken.
    pub fn route_next(&self, from: usize, to: usize) -> Option<(usize, usize)> {
        let links = self.routes.get(from)?;
        links.iter().find(|l| l.reach.contains(&to)).map(|l| (l.to, l.link))
    }

    /// The path points of the entries / exits, in order.
    pub fn entry_points(&self) -> Vec<Option<usize>> {
        self.entries.iter().map(|e| e.point).collect()
    }
    pub fn exit_points(&self) -> Vec<Option<usize>> {
        self.exits.iter().map(|e| e.point).collect()
    }
    /// ({noticketsale}, {withbutton}) of each entry.
    pub fn entry_flags(&self) -> Vec<(bool, bool)> {
        self.entries.iter().map(|e| (!e.sells, e.button)).collect()
    }
}

/// A heading difference wrapped to -pi .. pi (sub_7f3780).
pub fn wrap(a: f64) -> f64 {
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
pub fn yaw_of(d: DVec2) -> f64 {
    d.x.atan2(d.y)
}

#[cfg(test)]
mod tests;
