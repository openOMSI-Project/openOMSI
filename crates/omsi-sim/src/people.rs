//! People on foot: passengers and pedestrians as agents with a goal.
//!
//! A passenger comes along the pavement (or already stands at the stop when the map
//! starts), waits at a free waiting place of the stop - the `[passpos]` points of the
//! map's `people_standing_*` markers and shelters, else places spread along the back of
//! the platform - and when a bus opens its doors there, queues at the nearest open
//! `[entry]` (a passenger who still has to buy a ticket only at one with a cash desk),
//! steps in when the doorway is free, pays or shows a pass at the desk, walks the cabin's
//! `paths.cfg` network to a free `[passpos]` (a standing place once the seats are gone),
//! rides, presses the stop button before their stop, walks to the nearest `[exit]` when
//! the bus stands there, steps out and walks away along the pavement - or waits at the
//! stop for another bus. Timetable (AI) buses carry their passengers the same way.
//! Nobody is taken away while the player can see them.
//!
//! An articulated bus is one cabin: the sections' path networks, seats and exits are put
//! together in the front section's frame with the sections straight behind each other, and
//! the front section's `[linkToPrevVeh]` point is joined to the rear section's
//! `[linkToNextVeh]` point, so people walk through the bellows to the seats and exits at the
//! back. Entries and exits are numbered front section first, which is how the stock door
//! scripts count them (the GN92's rear door is `PAX_Exit2`/`PAX_Exit3`). A point behind a
//! joint is carried by its own section, whatever the angle of the bend.
//!
//! Movement is a crowd: everybody on the same floor (the ground, or one bus) avoids
//! everybody else with the anticipatory model of `omsi_sim::crowd`, does not push into
//! somebody standing in front, speeds up, slows down and turns at a human pace, and keeps
//! to the aisle inside a bus. Doorways and the cash desk are taken one at a time, people
//! getting off go first, and somebody pressed against another for seconds slips past.
//! Every waiting state has a way out, and `OMSI_DEBUG_PAX=1` logs every change of state
//! and why somebody stands still.
//!
//! Pedestrians walk the map's pavement paths as one network (path ends that meet are
//! joined whatever their heading), wait at the kerb for a pedestrian light's green - and
//! only start across when it lasts long enough - and for approaching cars where there is
//! no light; nobody stops in the middle of the road.
//!
//! The map streams: stops, waiting places and pavements come with their tiles. The
//! pavement network grows as the traffic network does, a stop is set up again when its
//! neighbourhood changed and nobody uses it, a stop whose tile went takes its people with
//! it, and nobody stands or walks where the ground is not loaded.

use crate::ai_traffic::TrafficSim;
use crate::crowd::{self, Block, CrowdParams, PathGraph, Walker};
use crate::human::{Activity, HumanType};
use crate::human_omsi::{AnimInput, OmsiAnim};
use crate::traffic::{LaneKind, Network};
use crate::VehicleInstance;
use glam::{DVec2, DVec3, Mat4, Vec3};
use hashbrown::{HashMap, HashSet};
use omsi_vehicle::PassengerCabin;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// The simulation: the people and what they do. It does not draw: what the renderer has to
// follow it writes down (`bodies`), and omsi-app's `humans` view replays that at the end
// of the public calls that make people (`tick`, `populate`, `avatar`, ...).
pub mod model;
pub mod cabin;
pub mod pednet;
pub mod spawn;
pub mod populate;
pub mod buses;
pub mod tick;
pub mod walk;
pub mod report;
pub mod avatar;
pub mod mirror;
// The passengers, as Omsi.exe runs them.
pub mod pax;
pub mod pax_stops;
pub mod pax_tick;
pub mod pax_task;
pub mod pax_driver;
pub mod runners;
// What the renderer has to follow, the money on the desk and what the people need of the map.
pub mod bodies;
pub mod money;
pub mod world;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod population_limit_tests;

use avatar::*;
use cabin::*;
use model::*;
use pax::*;
use pednet::*;
use runners::*;

// The module's API, at the paths it always had in omsi-app (some only returned, never named
// outside).
pub use avatar::{AvatarCmd, SeatSpot};
pub use bodies::{BodyOp, BodyOps};
pub use mirror::{placed_bus_id, remote_bus_id, remote_bus_player, LanPerson, MirrorPose};
pub use model::{BusId, DoorWants, Eye, Footfall, PeopleSim, Person, Place, State, VoiceLine};
pub use money::Money;
pub use pax::DutyTrip;
pub use world::World;

/// The map's traffic keeps left (its stops are on the left): see the doors of `Cabin`.
pub static LEFT_HAND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// How far outside the bus side somebody stands at a door (m).
const DOOR_OUT: f32 = 0.5;
/// Entries and exits with door variables of their own in Omsi.exe (`PAX_Entry0..7` ...).
const OMSI_PAX_DOORS: usize = 8;
/// Body radius for the crowd outside (m): shoulders and swinging arms. With the cabin's
/// radius people on the pavement came within 0.46 m, and two walking past each other or a
/// group crossing the road merged into one another in the picture.
const BODY_OUTSIDE: f64 = 0.28;
/// Stops within this distance of the player have their people (Omsi.exe: the stop's tile
/// and the eight round the camera's, sub_61bf94).
const STOP_RANGE: f64 = 450.0;
/// Pedestrians stroll within this distance of the player (m).
const STROLL_RADIUS: f64 = 200.0;
/// How far in front of a seat's hip point somebody stands to sit down - where the feet
/// stay while seated (m).
const SEAT_FRONT: f32 = 0.34;
/// Over this distance on either side of a joint (m) a point of an articulated bus's cabin
/// moves from the frame of the section in front to the one behind.
const JOINT_BLEND: f32 = 0.5;

fn debug_pax() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        omsi_cfg::flags::OMSI_DEBUG_PAX.is_set()
            || omsi_cfg::flags::OMSI_DEBUG_HUMANS.is_set()
    })
}
