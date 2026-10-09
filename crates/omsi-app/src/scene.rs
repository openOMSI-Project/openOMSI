//! Builds a renderable scene from a map: terrain, splines and scenery objects.
//!
//! Loading is parallel: every tile is parsed and tessellated on the rayon pool, scenery
//! object types are loaded once and shared, then everything is uploaded to the GPU on the
//! calling thread.

use crate::tiles::{MapIndex, Pose};
use anyhow::{Context, Result};
use glam::{DVec2, DVec3, Mat4};
use hashbrown::HashMap;
use omsi_geometry::{
    build_spline_mesh, build_terrain_mesh, mesh_from_o3d, object_rotation, MeshData, SplineCurve,
    TileSurface,
};
use omsi_map::{tile_size, GlobalCfg, Terrain};
use omsi_model::{MaterialDef, MeshDef, Model};
use omsi_render::{
    AlphaMode, MaterialExtra, MaterialId, MeshId, RenderPhase, Renderer, Scene, TextureId,
};
use omsi_scenery::{SceneryObject, Spline};
use omsi_sim::traffic::{Lane, LaneBuilder, LaneKey, LaneKind, TrafficLightController};
use omsi_texture::{Image, TextureCache, TextureData};
use parking_lot::{Mutex, RwLock};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod staging;
mod batching;
mod terrain_paint;
mod sound_probe;
mod world;
mod open;
mod stage;
mod place;
mod cut_terrain;
mod upload;
mod place_step;
mod props;
mod lightmaps;
mod scripted;
mod vehicle_materials;
mod vehicle_types;
mod vehicles;
mod vehicle_damage;
mod lanes;
mod season_looks;

pub(crate) use staging::*;
pub(crate) use batching::*;
pub(crate) use terrain_paint::*;
pub(crate) use world::*;
pub(crate) use open::*;
pub(crate) use props::*;
pub(crate) use scripted::*;
pub(crate) use vehicle_materials::*;
pub(crate) use vehicle_types::*;
pub(crate) use vehicle_damage::*;
use lanes::*;
use lightmaps::*;
pub(crate) use stage::*;
#[cfg(test)]
use place::field_height;

#[cfg(test)]
mod surf_map_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod material_tests;

#[cfg(test)]
mod navigation_road_tests;

#[cfg(test)]
mod object_path_tests;

#[cfg(test)]
mod terrain_mapping_tests;

#[cfg(test)]
mod crossing_deformation_tests;
