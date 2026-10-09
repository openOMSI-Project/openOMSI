//! The timetable at run time, as far as it needs no GPU: a trip's times at its stations
//! (`times`), the destinations and IBIS codes of the buses (`ibis`), the player's duty
//! (`duty`), and the timetable's state with its decisions (`sim`: `ScheduleSim`, which
//! departure leaves when, with which vehicle, where on its route). The game's `schedule`
//! puts the buses on the road with them.

mod duty;
mod ibis;
mod route;
mod sim;
mod times;
mod tours;
#[cfg(test)]
pub(crate) mod tests;

use crate::traffic::LaneKey;
use hashbrown::{HashMap, HashSet};

pub use duty::*;
pub use ibis::*;
pub use route::*;
pub use sim::*;
pub use times::*;
pub use tours::*;

/// One step of a trip's route: a lane in map terms (None when the tile index is not in the
/// map's list) and the leg between two stations it belongs to (0 for a track).
#[derive(Debug, Clone, Copy)]
pub struct Step {
    pub key: Option<LaneKey>,
    pub leg: usize,
    /// The path's length as the timetable file has it (m).
    pub length: f64,
}

/// A route step as the loaded network has it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Slot {
    /// The lane it runs on.
    Lane(usize),
    /// Its tile is part of the map, but has not brought its lanes yet.
    Waiting,
    /// Not in the map (a tile or path that does not exist): passed over, as a whole-map load
    /// passes it over.
    Absent,
}

/// When a trip's bus is at each of its stations, as OMSI's timetable has it: the profile
/// gives the trip's duration and, for some stations, the minute the bus arrives or leaves
/// (`[profile_man_arr_time]`, `[profile_man_dep_time]`, minutes after the trip's start);
/// the stations in between are timed by the lengths of the station links (Spandau's line 5
/// gives nearly every station its minute; these used to be ignored for the whole duration
/// split by the link lengths, and the tours ran their first profile whatever `[addtrip]`
/// said). A station marked `[profile_otherstopping] 2` is passed without a stop (every
/// station of a depot run).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TripTimes {
    /// Seconds after the trip's departure: (arrival, departure) per station.
    pub stations: Vec<(f64, f64)>,
    /// Whether the bus stops at the station.
    pub stops: Vec<bool>,
    /// `[profile_otherstopping]` per station (0 when not given): 1 and 4 stop whoever
    /// wants to get on or off, 2 is passed, 3 is served when the bus would be more than 20 s
    /// early (Omsi.exe 0x7da6f0 .. 0x7da8bf; see `bus_service::BusService::must_serve`).
    /// This is the editor's per-station stop setting; whether the bus *waits* there is a
    /// separate question (`bus_service::BusService::waits_here`).
    pub kinds: Vec<u8>,
    /// The stations whose time the map wrote itself (`[profile_man_arr_time]` /
    /// `[profile_man_dep_time]`): the bus waits there for its departure. A station whose
    /// time is only shared out of the trip's duration is no time point - a bus that beat
    /// its running time serves it and drives on, instead of standing there until its time
    /// (which held up every bus behind it; see `bus_service::BusService::waits_here`).
    pub holds: Vec<bool>,
    /// Seconds from the departure to the arrival at the last station.
    pub duration: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StopRoute {
    Nearest,
    Track(usize),
    Outside,
}

/// One stop of a planned trip with its scheduled times (seconds since midnight).
#[derive(Debug, Clone)]
pub struct PlannedStop {
    pub object_id: i64,
    pub name: String,
    pub arr: f64,
    pub dep: f64,
    pub position: Option<glam::DVec3>,
    /// Which way the trip runs through the stop ([`StopDir`]): a circular route, or one
    /// that turns back, calls at the same place twice and the two stops of it stand a few
    /// metres apart. Only the direction says which of them a bus has reached (#254).
    pub dir: StopDir,
    /// The bus stops here (a depot run passes its stations).
    pub stops: bool,
}

/// Which way a trip runs through one of its stops: the direction it arrives on and the one
/// it leaves on, as unit vectors of the ground plane (x east, y north). None where a
/// neighbour's place is unknown or too near to tell a direction - any heading will do then.
#[derive(Debug, Clone, Copy, Default)]
pub struct StopDir {
    pub inbound: Option<glam::DVec2>,
    pub outbound: Option<glam::DVec2>,
}

#[derive(Debug, Clone)]
pub struct PlannedTrip {
    pub name: String,
    pub line: String,
    pub terminus: String,
    pub departure: f64,
    /// Arrival at the last station.
    pub end: f64,
    pub stops: Vec<PlannedStop>,
}

/// The bus is at a stop within this distance (m), and has left it beyond the second.
pub const AT_STOP: f64 = 25.0;

/// How long before its departure the next trip of a duty may begin when the bus leaves the
/// terminus it has served (s).
pub const EARLY_START: f64 = 300.0;

pub const LEFT_STOP: f64 = 35.0;

/// How a trip of the player's duty ended (`Finished`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TripEnd {
    /// Its last stop reached, or the next trip's first stop from its last leg.
    Arrived,
    /// Its last stop skipped.
    Skipped,
    /// Begun, and over half an hour past its end: the duty went on.
    GivenUp,
}

impl TripEnd {
    /// As Lua plugins' `trip_done` names it.
    pub fn as_str(self) -> &'static str {
        match self {
            TripEnd::Arrived => "arrived",
            TripEnd::Skipped => "skipped",
            TripEnd::GivenUp => "given_up",
        }
    }
}

/// A trip of the player's duty that ended (`PlayerDuty::take_finished`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Finished {
    /// Its index in the duty's `trips`.
    pub index: usize,
    pub how: TripEnd,
    /// Its run (`PlayerDuty::trip_run`).
    pub run: u64,
}

/// The player's tour: its trips with planned stop times, and the progress along them.
pub struct PlayerDuty {
    pub line: String,
    pub tour: String,
    pub trips: Vec<PlannedTrip>,
    pub trip_index: usize,
    /// Where `trips` begins in the tour: a picked trip is a duty of its own, and a saved
    /// situation counts the trip under way from the tour's first.
    pub first_trip: usize,
    /// Next stop to serve on the current trip.
    pub next_stop: usize,
    /// True while the bus stands at the next stop.
    at_stop: bool,
    /// How late the bus arrived at the stop it stands at (s after its arrival time).
    arrived_late: Option<f64>,
    /// The bus has reached the last stop of the current trip.
    done: bool,
    /// The last stop where this trip actually stopped with a passenger door open.
    served_terminus: Option<glam::DVec3>,
    /// How late (s, negative = early) the bus left the last stop it served on this trip;
    /// None while it has not left one.
    left_late: Option<f64>,
    /// A page moved the duty back to an earlier stop: `catch_up` must not jump forward
    /// again to a later stop the bus still stands at, until the bus reaches a stop again.
    held_back: bool,
    /// The first update looks where the bus stands.
    placed: bool,
    /// The current trip changed since the last `take_trip_change`.
    trip_changed: bool,
    /// Stops the bus passed without stopping since the last `take_skipped` (see `catch_up`).
    skipped: Option<(usize, usize, usize)>,
    /// The current trip's run: a number no other trip of this game had (a trip taken again
    /// is another run), for rating a trip on its own (`trip_run`).
    run: u64,
    /// A trip ended since the last `take_finished` (see there).
    finished: Option<Finished>,
    /// A page reopened this run's trip after its end was taken (`take_reopened`).
    reopened: Option<u64>,
    /// The player picked the current trip: the duty does not move on past it before it is
    /// driven (or given up), however late the bus is for it.
    picked: bool,
    /// Time of day of the first update (placing waits a little for the places of stops
    /// beyond the loaded tiles, see `learn_places`).
    first_update: Option<f64>,
    /// The way the bus faces (degrees clockwise from north), from the last update: it says
    /// which of two stops a few metres apart the bus is at (see `StopDir`).
    heading: f64,
    /// Where the bus was at the last update: how far along the way to the next stop it
    /// is, for the delay on the way (`delay`).
    position: Option<glam::DVec3>,
}

pub const DAY: f64 = 86_400.0;

/// "HH:MM" of a time of day in seconds.
pub fn hhmm(t: f64) -> String {
    // (yesterday's trips of a night tour taken after midnight are before 0:00)
    let t = if t < 0.0 { t + DAY } else { t };
    format!("{:02}:{:02}", (t / 3600.0) as i32, ((t % 3600.0) / 60.0) as i32)
}
