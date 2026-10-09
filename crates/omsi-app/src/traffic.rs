//! AI road traffic: vehicles from `ailists.cfg` moving on the map's path network, the
//! traffic light programs of the junctions, and the population of cars around the player.
//!
//! The simulation is omsi-sim's `ai_traffic` (`TrafficSim`, which knows nothing of the
//! GPU); here it gets the loaded world's lanes and vehicle types, the population around
//! the player, the cars' pictures (`crate::view_sync::traffic`) and their sounds (`audio`).

mod audio;
mod control;
mod mirror;
mod parked;
mod population;
mod setup;
mod trains;

use anyhow::Result;
use glam::DVec3;
use hashbrown::HashMap;
use omsi_sim::ai_motion::{AiBody, MotionKind};
use omsi_sim::ai_traffic::bus_service::BusService;
use omsi_sim::ai_traffic::setup::RandomTypes;
use omsi_sim::ai_traffic::TrafficSim;
use omsi_sim::traffic::{AiState, LaneKind, Network};
use omsi_sim::{VehicleInstance, VehicleType};
use std::path::Path;
use std::sync::Arc;

use omsi_sim::ai_traffic::density::*;
use omsi_sim::ai_traffic::dormant::*;
use omsi_sim::ai_traffic::lights::*;
use omsi_sim::ai_traffic::model::*;
use omsi_sim::ai_traffic::viewer::*;
use crate::view_sync::traffic::{depart_parked, release_car_render, TrafficView};

pub use omsi_sim::ai_traffic::AI_SCHEMES;
pub(crate) use omsi_sim::ai_traffic::model::{vehicle_bodies, AiCar, BusSetup, DormantCar, ParkPlan, PlayerBox};
pub(crate) use setup::warm_up;
pub(crate) use omsi_sim::ai_traffic::viewer::Viewer;

/// The traffic simulation with what the game makes of it: the cars' sounds. Everything of
/// the simulation reads through it (`Deref` to `TrafficSim`). The cars' pictures are the
/// view sync's (`TrafficView`, in `view_sync::SimView`): the traffic's own steps that put a
/// car on the road or take it off take it, and make or let go the car's renders at once.
pub struct Traffic {
    pub sim: TrafficSim,
    /// `[sound_ai]` set of each car near the listener, by car id (see `audio`).
    sounds: HashMap<u64, omsi_audio::SoundSet>,
    /// Sound sets of despawned cars, stopped at the next audio update.
    orphan_sounds: Vec<omsi_audio::SoundSet>,
    /// The bus the player rides in on foot, heard from inside: its whole `[sound]` set as
    /// the player's own bus is heard from its cab (#1286).
    riding_sounds: Option<(u64, omsi_audio::SoundSet)>,
    /// `[sound_ai]` configurations by file.
    sound_cfgs: HashMap<std::path::PathBuf, Option<Arc<omsi_vehicle::SoundCfg>>>,
}

impl std::ops::Deref for Traffic {
    type Target = TrafficSim;
    fn deref(&self) -> &TrafficSim {
        &self.sim
    }
}

impl std::ops::DerefMut for Traffic {
    fn deref_mut(&mut self) -> &mut TrafficSim {
        &mut self.sim
    }
}

impl Traffic {
    /// The simulation, with no car heard yet (and none drawn: its `TrafficView` starts
    /// afresh where it is put in place).
    fn with_sim(sim: TrafficSim) -> Traffic {
        Traffic {
            sim,
            sounds: HashMap::new(),
            orphan_sounds: Vec::new(),
            riding_sounds: None,
            sound_cfgs: HashMap::new(),
        }
    }

    /// Advance all cars (see `TrafficSim::tick`); the cars it took off the road go silent
    /// and their pictures go back to the world at the next sync.
    /// `player`: (centre, heading in degrees, half length, half width, speed) of the
    /// player's vehicle.
    pub fn tick(&mut self, view: &mut TrafficView, dt: f32, player: Option<PlayerBox>) {
        self.sim.tick(dt, player);
        for id in std::mem::take(&mut self.sim.retired) {
            self.orphan_sounds.extend(self.sounds.remove(&id));
            view.retire(id);
        }
    }

    /// Let go a car's sound set (it is stopped at the next audio update).
    pub(crate) fn drop_sounds(&mut self, id: u64) {
        self.orphan_sounds.extend(self.sounds.remove(&id));
    }
}
