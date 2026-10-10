//! Scheduled AI buses: the map's timetable lines (`TTData`) put buses on their tracks at the
//! tour departure times; they follow the track lanes and stop at the trip's stations.
//!
//! The timetable itself is omsi-sim's `timetable_run` (`ScheduleSim`: the departures and
//! which are due, the tours' vehicles, the routes on the lanes, the departure boards; the
//! trip times, the IBIS, the player's duty), which does not touch the GPU. `Schedule` holds
//! it (`Deref` to `ScheduleSim`) with what the GPU needs: `fleet` reads and uploads the
//! vehicles ahead, `dispatch` and `spawn` put the buses on the road and take them off.

mod dispatch;
mod fleet;
mod spawn;
#[cfg(test)]
pub(crate) mod tests;

use crate::scene::World;
use crate::traffic::Traffic;
use crate::view_sync::traffic::TrafficView;
use hashbrown::{HashMap, HashSet};
use omsi_sim::VehicleType;
use parking_lot::MutexGuard;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use omsi_sim::timetable_run::*;

pub use omsi_sim::timetable_run::{PlannedStop, PlannedTrip, PlayerDuty};
// (the tests of `schedule_paper` name it by this path)
#[cfg_attr(not(test), allow(unused_imports))]
pub use omsi_sim::timetable_run::StopDir;
pub(crate) use omsi_sim::timetable_run::{
    has_roller_blind, hhmm, ibis_stop_index, player_ibis, set_ai_destination,
    set_ai_destination_at, set_player_destination_at, set_player_destination_directly,
    shown_destination, turn_roller_blind, BlindPick,
};

/// The timetable (`ScheduleSim`, which everything of the timetable reads through `Deref`)
/// with the vehicle sets it reads and uploads ahead for the GPU.
pub struct Schedule {
    pub sim: ScheduleSim,
    next_number: usize,
    /// Vehicle sets being read ahead on the workers, with the type to upload them with, and
    /// those read and waiting for their upload.
    fleet_reading: HashMap<crate::scene::VehicleKey, Arc<VehicleType>>,
    fleet_ready: Arc<parking_lot::Mutex<Vec<crate::scene::VehicleKey>>>,
    /// Time of day of the last look at the next departures' vehicles.
    fleet_check: f64,
}

impl std::ops::Deref for Schedule {
    type Target = ScheduleSim;
    fn deref(&self) -> &ScheduleSim {
        &self.sim
    }
}

impl std::ops::DerefMut for Schedule {
    fn deref_mut(&mut self) -> &mut ScheduleSim {
        &mut self.sim
    }
}

/// What the timetable needs of the loaded world (see `omsi_sim::timetable_run::TimetableWorld`).
impl TimetableWorld for World {
    fn root(&self) -> &Path {
        &self.root
    }
    fn map_dir(&self) -> &Path {
        &self.map_dir
    }
    fn chrono_dirs(&self) -> Vec<PathBuf> {
        self.chrono_dirs.read().clone()
    }
    fn ailists(&self) -> &omsi_map::AiLists {
        &self.ailists
    }
    fn raw_tiles(&self) -> Vec<(i32, i32)> {
        self.global.raw_tiles.clone()
    }
    fn has_tile(&self, key: (i32, i32)) -> bool {
        World::has_tile(self, key)
    }
    fn date(&self) -> i32 {
        self.date
    }
    fn object_positions(&self) -> MutexGuard<'_, HashMap<i64, (glam::DVec3, [f64; 3])>> {
        self.object_positions.lock()
    }
    fn stop_side(&self, id: i64) -> f32 {
        World::stop_side(self, id)
    }
}

impl Schedule {
    /// `clock` gives the date: tours carry a validity mask (bits 0-6 Monday…Sunday, 7 public
    /// holiday, 8 school days, 9 school holidays) that selects which run today.
    pub fn new(root: &Path, world: &World, clock: &omsi_sim::SimClock) -> Schedule {
        Schedule {
            sim: ScheduleSim::new(root, world, clock),
            next_number: 0,
            fleet_reading: HashMap::new(),
            fleet_ready: Default::default(),
            fleet_check: f64::NEG_INFINITY,
        }
    }

    /// The lanes a trip runs on (for the navigator): its track, else the station links
    /// between its stops, as far as the tiles have brought them - and whether that is all.
    pub fn trip_route(&self, world: &World, traffic: &Traffic, trip_name: &str) -> (Vec<usize>, bool) {
        self.sim.trip_route(world, &traffic.sim, trip_name)
    }

    /// `OMSI_CHECK_TRIPS=1`: the routes of every trip on the loaded lanes checked (see
    /// `ScheduleSim::check_routes`).
    pub fn check_routes(&self, world: &World, traffic: &mut Traffic) {
        self.sim.check_routes(world, &mut traffic.sim)
    }

    /// Assign the player a tour of a line (see `ScheduleSim::player_duty`).
    pub fn player_duty(
        &mut self,
        world: &World,
        line: &str,
        tour: &str,
        now: f64,
        trip: Option<&str>,
        whole_tour: bool,
    ) -> Result<PlayerDuty, String> {
        self.sim.player_duty(world, line, tour, now, trip, whole_tour)
    }

    /// Make the departure boards of the stops whose displays are near
    /// (`World::timetable_boards`), and hand the scenery the time of day. The boards are
    /// made at most once a second: the buses due at each stop in the next two hours,
    /// soonest first - the timetable buses with the delay they run with, and the player's.
    pub fn update_boards(
        &mut self,
        world: &World,
        traffic: Option<&Traffic>,
        duty: Option<&PlayerDuty>,
        clock: &omsi_sim::SimClock,
    ) {
        let now = clock.time;
        let mut boards = world.timetable_boards.lock();
        boards.clock = Some(clock.clone());
        // (a stop a page asks for anew is made at once, not up to a second later: the boards of
        // the map's own displays may just have been made without it)
        let new_name = boards.wanted_names.iter().any(|k| !boards.departures.contains_key(k));
        if (self.sim.boards_fresh(now) && !new_name) || (boards.wanted.is_empty() && boards.wanted_names.is_empty()) {
            return;
        }
        let (wanted, wanted_names) = (boards.wanted.clone(), boards.wanted_names.clone());
        let (by_stop, departures) =
            self.sim.make_boards(traffic.map(|t| &t.sim), wanted, wanted_names, duty, clock);
        boards.by_stop = by_stop;
        boards.departures = departures;
        boards.departures_gen = boards.departures_gen.wrapping_add(1);
    }
}

impl Schedule {
    /// The timetable buses on the road, for outside tools (`telemetry`): line, tour, trip file,
    /// next stop (map object), standing at it, delay, position and fleet number of each.
    pub fn telemetry_ai(&self, traffic: Option<&crate::traffic::Traffic>) -> serde_json::Value {
        let Some(t) = traffic else { return serde_json::Value::Array(Vec::new()) };
        let list = self.sim.ai_bus_rows(&t.sim);
        serde_json::Value::Array(list.into_iter().map(|b| serde_json::json!({
            "id": b.id,
            "line": b.line,
            "tour": b.tour,
            "trip": b.trip,
            "terminus": b.terminus,
            "depart": b.depart,
            "next_stop_id": b.next_stop_id,
            "at_stop": b.at_stop,
            "trip_done": b.trip_done,
            "delay_s": b.delay_s,
            "x": b.x, "y": b.y,
            "number": b.number,
        })).collect())
    }
}
