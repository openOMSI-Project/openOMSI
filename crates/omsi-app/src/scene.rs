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

/// A loaded scenery object type: model meshes + material descriptions.
pub struct ObjectType {
    pub sco: SceneryObject,
    /// The `[sound]` config's file, found the first time it is needed.
    pub sound_path: std::sync::OnceLock<Option<PathBuf>>,
    pub model: Model,
    pub model_dir: PathBuf,
    /// LOD 0 meshes (mesh data, o3d materials, model material overrides).
    pub meshes: Vec<(MeshData, Vec<omsi_o3d::Material>, Vec<MaterialDef>)>,
    /// `[visible] var value` per mesh, parallel to `meshes`.
    pub mesh_visible: Vec<Option<(String, f32)>>,
    /// Model mesh definition index and pivot per loaded mesh (parallel to `meshes`).
    pub mesh_def_index: Vec<usize>,
    pub mesh_pivots: Vec<Mat4>,
    /// `[isshadow]` per loaded mesh: a flat shadow blob drawn on the ground.
    pub mesh_shadow: Vec<bool>,
    /// `[shadow]` per loaded mesh: the meshes OMSI casts (stencil) shadows from.
    pub mesh_casts: Vec<bool>,
    /// Compiled scripts when the object is scripted or animated.
    pub program: Option<Arc<omsi_script::Program>>,
    /// Further `[LOD]` levels: (min screen size, meshes), in model order after LOD 0.
    pub lower_lods: Vec<(
        f32,
        Vec<(MeshData, Vec<omsi_o3d::Material>, Vec<MaterialDef>)>,
    )>,
    /// Min screen size of LOD 0 (0 = always).
    pub lod0_min: f32,
    /// Number of `[CTC]` paint schemes the object offers.
    pub paint_scheme_count: usize,
    /// Runtime scenery texture groups (`[CTC]` and `[texchanges]`), each selected by its own
    /// script variable and supplying one or more material texture replacements per choice.
    pub dynamic_textures: Vec<DynamicTextureGroup>,
    /// `[terrainhole]` meshes of the model: where they lie the ground is taken away.
    pub holes: Vec<MeshData>,
    /// `[crossing_heightdeformation]`: the mesh a crossing presses the terrain into, so
    /// the junction plate and the roads that meet it sit on one surface.
    pub deform: Option<MeshData>,
    /// `[collision_mesh]`: what vehicles actually hit (often much plainer than the model).
    pub collision: Option<MeshData>,
    /// A plain object that is only paint at its foot ([`paint_at_foot`]).
    pub paint: bool,
    /// What of the type stops the outside camera (decided on first use).
    pub camera: std::sync::OnceLock<crate::camera_arm::BlockerShape>,
    /// The collision mesh as the vehicles meet it (built on first use).
    pub collision_shape: std::sync::OnceLock<Arc<omsi_sim::collision::MeshShape>>,
}

/// One scenery texture selector and its indexed replacement sets.
#[derive(Clone)]
pub struct DynamicTextureGroup {
    pub variable: String,
    /// Each replacement is (the material's default texture key, replacement file, folder).
    pub choices: Vec<Vec<(String, String, PathBuf)>>,
}

impl World {
    /// The map's indexed parking lists: index 0 is `parklist_p.txt`, and an
    /// editor caption of 1 selects `parklist_p_1.txt` for that parking space.
    pub fn parked_car_types(&self, index: usize) -> Vec<String> {
        let mut g = self.parklist.lock();
        if !g.contains_key(&index) {
            let filename = if index == 0 { "parklist_p.txt".to_string() } else { format!("parklist_p_{index}.txt") };
            let text =
                omsi_cfg::vfs::read(&omsi_cfg::resolve_path(&self.map_dir, &filename))
                    .ok()
                    .map(|b| omsi_cfg::decode_text(&b))
                    .unwrap_or_default();
            let list: Vec<String> = text
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && l.to_ascii_lowercase().ends_with(".sco"))
                .collect();
            log::info!("{filename}: {} parked car types", list.len());
            g.insert(index, list);
        }
        g.get(&index).cloned().unwrap_or_default()
    }

    /// The shape of the glass that shows mirror `i`'s picture: material `slot` of `data`. How far
    /// the position moves for a step of the texture coordinates across and down, over a
    /// triangle at a time (area weighted), times the part of the picture the mesh uses.
    pub fn note_mirror_aspect(&self, i: usize, data: &MeshData, slot: usize) {
        let (mut tu, mut tv, mut area) = (0.0f64, 0.0f64, 0.0f64);
        let (mut umin, mut umax, mut vmin, mut vmax) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        // the glass's middle and the way the texture's axes lie on it, in the bus's frame
        let (mut centre, mut vertices) = (glam::Vec3::ZERO, 0.0f32);
        let (mut du, mut dv) = (glam::Vec3::ZERO, glam::Vec3::ZERO);
        for &(first, count, mat) in &data.ranges {
            if mat as usize != slot {
                continue;
            }
            let end = ((first + count) as usize).min(data.indices.len());
            for tri in data.indices[(first as usize).min(end)..end].chunks_exact(3) {
                let (Some(&a), Some(&b), Some(&c)) = (data.positions.get(tri[0] as usize), data.positions.get(tri[1] as usize), data.positions.get(tri[2] as usize)) else { continue };
                let (Some(&ua), Some(&ub), Some(&uc)) = (data.uvs.get(tri[0] as usize), data.uvs.get(tri[1] as usize), data.uvs.get(tri[2] as usize)) else { continue };
                for uv in [ua, ub, uc] {
                    umin = umin.min(uv.x);
                    umax = umax.max(uv.x);
                    vmin = vmin.min(uv.y);
                    vmax = vmax.max(uv.y);
                }
                let (e1, e2) = (b - a, c - a);
                let (d1, d2) = (ub - ua, uc - ua);
                let det = d1.x * d2.y - d1.y * d2.x;
                if det.abs() < 1e-9 {
                    continue;
                }
                let t = (e1 * d2.y - e2 * d1.y) / det;
                let v = (e2 * d1.x - e1 * d2.x) / det;
                let w = e1.cross(e2).length() as f64;
                centre += a + b + c;
                vertices += 3.0;
                du += t * w as f32;
                dv += v * w as f32;
                tu += t.length() as f64 * w;
                tv += v.length() as f64 * w;
                area += w;
            }
        }
        if area <= 0.0 || umax <= umin || vmax <= vmin || tv <= 0.0 {
            return;
        }
        let aspect = ((tu / area) * (umax - umin) as f64) / ((tv / area) * (vmax - vmin) as f64);
        if !aspect.is_finite() || !(0.15..=6.0).contains(&aspect) {
            return;
        }
        let mut g = self.mirror_aspect.lock();
        if g.len() <= i {
            g.resize(i + 1, 0.0);
        }
        g[i] = aspect as f32;
        drop(g);
        // (a mirror's material may also cover a bit of its housing, with the texture
        // coordinates all in one place: the mesh that uses most of the picture is the glass)
        let uv_area = (umax - umin) * (vmax - vmin);
        let mut g = self.mirror_glass.lock();
        if g.len() <= i {
            g.resize(i + 1, None);
        }
        g[i] = larger_glass(g[i], MirrorGlass { centre: centre / vertices.max(1.0), du, dv, uv_area });
    }

    /// Render texture of mirror `i` (created on first use, as large as the `mirror_size`
    /// setting says).
    pub fn mirror_texture(&self, renderer: &Renderer, scene: &mut Scene, i: usize) -> TextureId {
        let mut g = self.mirror_textures.lock();
        if g.len() <= i {
            g.resize(i + 1, None);
        }
        if let Some(t) = g[i] {
            return t;
        }
        let n = crate::MIRROR_SIZE.load(std::sync::atomic::Ordering::Relaxed).clamp(64, 2048);
        let t = renderer.add_render_texture(scene, n, n);
        g[i] = Some(t);
        t
    }
}

/// Where the glass that shows a mirror's picture is and how the picture lies on it, in the
/// bus's frame (x right, y forward, z up).
#[derive(Clone, Copy, Debug)]
pub struct MirrorGlass {
    pub centre: glam::Vec3,
    /// How the position moves with the texture's u and v.
    pub du: glam::Vec3,
    pub dv: glam::Vec3,
    /// How much of the picture the mesh uses.
    pub uv_area: f32,
}

/// Of two meshes that show one mirror's picture, the glass: the one that uses more of it
/// (the other is a bit of housing sharing the material).
fn larger_glass(old: Option<MirrorGlass>, new: MirrorGlass) -> Option<MirrorGlass> {
    Some(old.filter(|o| o.uv_area >= new.uv_area).unwrap_or(new))
}

/// `reflexionN.bmp`: the texture drawn by reflection camera N of the vehicle.
fn mirror_index(name: &str) -> Option<usize> {
    let n = name.trim().to_ascii_lowercase();
    let rest = n.strip_prefix("reflexion")?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// A light map that is white all over (the LED panels' `vmatrix_leer_led_LM.png`, one white
/// pixel): the surface is all its own light. A flipdot panel carries the same `\S:n` mask,
/// but its light map is a picture of the lamps over it (`vmatrix_leer_LM.bmp`).
fn is_white_lightmap(rgba: &[u8]) -> bool {
    !rgba.is_empty() && rgba.chunks_exact(4).all(|p| p[0] >= 242 && p[1] >= 242 && p[2] >= 242)
}

/// [`is_white_lightmap`] of the light map `name` (found in `dirs`, read once per file);
/// `None` when the file is not there.
fn lightmap_is_white(name: &str, dirs: &[&Path]) -> Option<bool> {
    static WHITE: std::sync::OnceLock<Mutex<HashMap<PathBuf, bool>>> = std::sync::OnceLock::new();
    let path = omsi_texture::find_texture(name, dirs)?;
    let cache = WHITE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(w) = cache.lock().get(&path) {
        return Some(*w);
    }
    let w = omsi_texture::decode_file(&path).ok().map(|i| is_white_lightmap(&i.rgba))?;
    cache.lock().insert(path, w);
    Some(w)
}

impl ObjectType {
    /// The folders the object's textures are looked for in. Omsi.exe loads the model of a
    /// `.sco` with the `.sco`'s own folder as the base of `texture\`, also when `[model]`
    /// takes the model file from another folder: a retexture (a copy of the `.sco` with its
    /// own `texture` folder that points at the original's model) shows its own pictures, not
    /// the original's (#978).
    pub fn texture_dirs(&self, root: &Path) -> Vec<PathBuf> {
        let mut dirs = texture_dirs(root, &self.model_dir);
        if let (Some(_), Some(sco_dir)) = (&self.sco.model_file, self.sco.path.parent()) {
            let own = omsi_cfg::resolve_path(sco_dir, "texture");
            dirs.retain(|d| *d != own);
            dirs.insert(0, own);
        }
        dirs
    }

    /// The model's own extents as a `[boundingbox]` would give them (width, length, height,
    /// centre x, y, z), for an object that has none.
    pub fn local_box(&self) -> Option<[f32; 6]> {
        let mut lo = glam::Vec3::splat(f32::MAX);
        let mut hi = glam::Vec3::splat(f32::MIN);
        for p in self.meshes.iter().flat_map(|m| m.0.positions.iter()) {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        if lo.x > hi.x {
            return None;
        }
        let (s, c) = (hi - lo, (hi + lo) * 0.5);
        Some([s.x, s.y, s.z, c.x, c.y, c.z])
    }

    /// Bytes of the meshes the type keeps on the CPU.
    pub fn mesh_bytes(&self) -> usize {
        self.meshes
            .iter()
            .chain(self.lower_lods.iter().flat_map(|l| l.1.iter()))
            .map(|m| m.0.heap_bytes())
            .sum::<usize>()
            + self.holes.iter().map(|m| m.heap_bytes()).sum::<usize>()
            + self.deform.as_ref().map(|m| m.heap_bytes()).unwrap_or(0)
            + self.collision.as_ref().map(|m| m.heap_bytes()).unwrap_or(0)
    }

    /// The type's solid shape when it stops the outside camera.
    pub fn camera_shape(&self) -> Option<&crate::camera_arm::BlockerShape> {
        Some(
            self.camera
                .get_or_init(|| crate::camera_arm::classify(self)),
        )
        .filter(|s| s.blocks)
    }

    /// (definition, pivot) per loaded mesh, for the scenery script runtime.
    pub fn mesh_defs(&self) -> Vec<(&MeshDef, Mat4)> {
        self.mesh_def_index
            .iter()
            .zip(&self.mesh_pivots)
            .map(|(d, p)| (&self.model.meshes[*d], *p))
            .collect()
    }
}

/// A placed scenery object with a running script / animations.
pub struct ScriptedObject {
    pub ty: Arc<ObjectType>,
    pub pos: DVec3,
    pub xf: Mat4,
    /// Render instance per loaded mesh.
    pub instances: Vec<usize>,
    pub inst: omsi_sim::scenery::SceneryInstance,
    /// Traffic light program of this object (crossings) or of its parent (lamps).
    pub controller: Option<usize>,
    pub light_index: usize,
    /// A child of a crossing that names one of its lights without being one of our lamps
    /// (its own textures to choose, see `child_lamp`): (the crossing, the light). Its
    /// script reads that light's `TrafficLightPhase` and `TrafficLightApproach`, as
    /// Omsi.exe's RefreshAmpelParenting (0x77d460) hands them to any such child. The
    /// crossing's program is looked up when the script runs: it may stand on a tile
    /// loaded later.
    pub light_parent: Option<(i64, usize)>,
    pub map_id: i64,
    /// `[matl_change]` variants: (instance, slot, base, item, variable).
    pub variants: Vec<(usize, usize, MaterialId, MaterialId, String)>,
    /// `[sound]` config of the object, loaded when the listener comes near.
    pub sounds: Option<omsi_audio::SoundSet>,
    /// The tile it belongs to (it goes when the tile is unloaded).
    pub tile: (i32, i32),
    /// `[varparent]`: the object whose data this one shows (a departure display's stop).
    pub var_parent: Option<i64>,
    /// `[texttexture]`s drawn from the script's string variables: (texture, state).
    pub texts: Vec<(TextureId, omsi_sim::texttex::TextTextureState)>,
    /// The script asks for the buses due at its stop (`GetArrBus*`).
    pub arrivals: bool,
    /// `[htmltexture]` pages shown on the object: (script texture index, texture). The
    /// pages themselves are `inst.html_textures`.
    pub htmls: Vec<(usize, TextureId)>,
    /// Per instance, its slots faded by `[alphascale]` variables (`LampSlots::alpha`; empty
    /// when none is), and the alphas last given.
    pub alpha_slots: Vec<LampSlots>,
    pub alpha_last: Vec<Vec<f32>>,
}

/// Where a ray lands on a page (`[htmltexture]`) of a scenery object: see
/// [`World::html_object_hit`].
#[derive(Clone, Copy, Debug)]
pub struct PageHit {
    /// Distance (m) along the ray.
    pub t: f32,
    pub map_id: i64,
    /// The page's script texture index.
    pub page: usize,
    /// 0..1 across the page, `v` down from the top.
    pub u: f32,
    pub v: f32,
}

/// What the timetable tells the scenery: the time of day, and the buses due at the stops
/// whose departure displays are near (see [`World::timetable_boards`]).
#[derive(Default)]
pub struct StopBoards {
    /// The simulation clock the boards were made at (None before a timetable or clock ran:
    /// the scenery scripts then keep their own).
    pub clock: Option<omsi_sim::SimClock>,
    /// Per bus stop (map object id): the buses due, soonest first, as (line, terminus,
    /// expected arrival in seconds of the day).
    pub by_stop: HashMap<i64, Vec<(String, String, f64)>>,
    /// The stops whose displays asked in the last scenery update.
    pub wanted: Vec<i64>,
    /// The stop names the HTML pages asked departures for (`omsi.getDepartures`): trimmed,
    /// lower case.
    pub wanted_names: Vec<String>,
    /// Per stop name of `wanted_names`: the departures of the next two hours, soonest first,
    /// at most 20, as (line, destination, timestamp).
    pub departures: std::collections::HashMap<String, Vec<(String, String, f64)>>,
    /// Counts up whenever `departures` was made anew.
    pub departures_gen: u64,
}

/// The material slots of a lamp's mesh switched by its variables: `[alphascale] var`
/// fades a slot (a lens shown by its alpha rather than by a `[visible]` mesh, as the
/// Korean maps' signals do, #826) and `[matl_lightmap] tex var` lights it.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct LampSlots {
    pub count: usize,
    pub alpha: Vec<(usize, String)>,
    pub light: Vec<(usize, String)>,
}

impl LampSlots {
    /// The `[alphascale]` and `[matl_lightmap]` variables of a mesh's material slots.
    pub fn of_mesh(o3d_mats: &[omsi_o3d::Material], overrides: &[MaterialDef], count: usize) -> LampSlots {
        let mut l = LampSlots { count, ..Default::default() };
        for o in overrides.iter().filter(|o| !o.item) {
            let Some(slot) = omsi_sim::vehicle::override_slot(o3d_mats, o) else { continue };
            if let Some(v) = o.alphascale.as_ref().filter(|v| !v.trim().is_empty()) {
                l.alpha.push((slot, v.trim().to_string()));
            }
            if let Some((_, v)) = &o.lightmap {
                l.light.push((slot, v.trim().to_string()));
            }
        }
        l
    }

    pub fn is_empty(&self) -> bool {
        self.alpha.is_empty() && self.light.is_empty()
    }

    /// The slots' alpha and light-map switch from the lamp's variables (`value`: `None`
    /// for a variable the lamp does not have). Alpha: the variable's value, a slot without
    /// one opaque as authored. Light map: on from 0.5; a variable the lamp does not have
    /// (or none at all) leaves it on, as Omsi.exe's stage does with an unregistered one.
    pub fn values(&self, value: &dyn Fn(&str) -> Option<f32>) -> (Vec<f32>, Vec<f32>) {
        let mut alpha = vec![1.0; self.count.max(1)];
        let mut light = vec![1.0; self.count.max(1)];
        for (slot, v) in &self.alpha {
            if let (Some(a), Some(x)) = (alpha.get_mut(*slot), value(v)) {
                *a = x.clamp(0.0, 1.0);
            }
        }
        for (slot, v) in &self.light {
            if let (Some(l), Some(x)) = (light.get_mut(*slot), (!v.is_empty()).then(|| value(v)).flatten()) {
                *l = if x >= 0.5 { 1.0 } else { 0.0 };
            }
        }
        (alpha, light)
    }
}

/// `LightObject::parent` of a signal that names no crossing: no crossing has this id.
pub const NO_CROSSING: i64 = i64::MIN;

/// A placed `[trafficlight]` object: its render instances follow the light state of
/// light `index` of the crossing `parent`.
#[derive(Clone)]
pub struct LightObject {
    pub parent: i64,
    pub index: usize,
    /// The map names no light for it (no string, or an empty one): a gate that is its own
    /// crossing, such as the Spandau depot's barrier (`Omnibushof_S_1`/`_S_2`, one arm for
    /// the way in and one for the way out, both moved by one script from one
    /// `TrafficLightPhase`). It is shown the most open state of its crossing's lights, so
    /// that the arms rise for whoever is let through, coming in or going out; on light 0
    /// alone the exit arm stayed down while the buses drove out through it.
    pub any_light: bool,
    /// (render instance, `[visible]` condition of that mesh)
    pub instances: Vec<(usize, Option<(String, f32)>)>,
    /// Per instance (parallel to `instances`) the material slots its lamp variables switch
    /// besides `[visible]`: see [`LampSlots`].
    pub slots: Vec<LampSlots>,
    /// Material switches kept with the lamp, updated alongside its visibility.
    pub variants: Vec<(usize, usize, MaterialId, MaterialId, String)>,
    pub pos: DVec3,
    /// The lamp's own script (`ampel1.osc` & co): it turns `TrafficLightPhase` into the
    /// `Red`/`Yellow`/`Green`/`Left`/`Light` variables its meshes and coronas show.
    /// Shared by the copies of the lamp list (the loaded tiles' lists are copied into
    /// `World::light_objects` whenever a tile comes or goes), so that it keeps its state.
    pub script: Option<Arc<Mutex<omsi_sim::scenery::SceneryInstance>>>,
    /// `[light_enh_2]` coronas switched by a lamp variable, and that variable.
    pub coronas: Vec<(omsi_render::Corona, String)>,
    /// Per corona the mesh its light belongs to and the light's place and direction in the
    /// model: an animated lamp's lights move with their mesh (see `model_light_sources`).
    pub corona_mesh: Vec<(usize, glam::Vec3, glam::Vec3)>,
    /// Current brightness of each corona (set with the lamp state every frame).
    pub lit: Vec<f32>,
    /// The object's rotation, and whether its script moves meshes of it: a level
    /// crossing's barrier is a `[trafficlight]` object whose arm turns with its light
    /// (`bue_schranke.osc`), and stood as a static model across the road.
    pub xf: Mat4,
    pub animated: bool,
    /// `[sound]` of the lamp (a crossing's bell, `bue_anlage1`), resolved, and its sounds
    /// once the listener is near (shared by the copies of the list, like `script`).
    pub sound: Option<PathBuf>,
    pub sounds: Arc<Mutex<Option<omsi_audio::SoundSet>>>,
    pub shown: Option<u64>,
    pub texts: Vec<(TextureId, omsi_sim::texttex::TextTextureState)>,
}

pub struct SplineType {
    pub def: Spline,
    pub dir: PathBuf,
    /// The `.surf` map of each of `def.textures` (see [`surf_map`]).
    pub surf: Vec<Option<Arc<omsi_geometry::HeightMap>>>,
}

#[derive(Debug, Default)]
pub struct LoadStats {
    pub tiles: usize,
    pub objects: usize,
    pub trees: usize,
    pub splines: usize,
    pub object_types: usize,
    pub spline_types: usize,
    pub textures: usize,
    /// Object records whose type is not in this installation (logged once per file).
    pub failed_objects: usize,
    /// Parking spaces the map leaves empty on purpose (a quarter of them).
    pub empty_spaces: usize,
    /// `[splineAttachement]` rows (and repeaters) that put objects on a spline.
    pub rows: usize,
    /// `[attachObj]` records, and those whose parent or attachment point is missing.
    pub attached: usize,
    pub unattached: usize,
    /// Objects stood on the ground (trees and invisible helpers included).
    pub objects_placed: usize,
    /// Ground points pulled onto `[spline_terrain_align]` roads, on how many tiles, and the
    /// biggest move (metres, where).
    pub ground_aligned: usize,
    pub ground_aligned_tiles: usize,
    pub ground_moved_most: Option<(f32, f64, f64)>,
    /// Tiles a crossing's `[crossing_heightdeformation]` changed, and crossings warped.
    pub ground_deformed_tiles: usize,
    pub crossings_warped: usize,
}

impl LoadStats {
    /// What the roads and crossings did to the ground.
    pub fn log_ground(&self) {
        if self.ground_aligned > 0 {
            log::info!(
                "terrain aligned to the roads: {} ground points on {} tiles",
                self.ground_aligned,
                self.ground_aligned_tiles
            );
            if let Some((d, x, y)) = self.ground_moved_most {
                log::info!("  the ground moved most at ({x:.0}, {y:.0}): {d:.2} m");
            }
        }
        if self.ground_deformed_tiles > 0 || self.crossings_warped > 0 {
            log::info!(
                "crossings deform the terrain on {} tiles; {} crossings warped onto the ground",
                self.ground_deformed_tiles,
                self.crossings_warped
            );
        }
    }

    /// Add what a batch of prepared tiles counted.
    pub fn add_prepared(&mut self, s: &LoadStats) {
        self.failed_objects += s.failed_objects;
        self.empty_spaces += s.empty_spaces;
        self.rows += s.rows;
        self.attached += s.attached;
        self.unattached += s.unattached;
        self.objects_placed += s.objects_placed;
        self.ground_aligned += s.ground_aligned;
        self.ground_aligned_tiles += s.ground_aligned_tiles;
        self.ground_deformed_tiles += s.ground_deformed_tiles;
        self.crossings_warped += s.crossings_warped;
        if let Some(b) = s.ground_moved_most {
            if self.ground_moved_most.map(|m| b.0 > m.0).unwrap_or(true) {
                self.ground_moved_most = Some(b);
            }
        }
    }
}

/// Where a staged object will stand once the ground under it is final.
#[derive(Clone)]
enum Placement {
    /// An `[object]` record: world x, y, the height above the terrain and its rotation.
    Ground {
        x: f64,
        y: f64,
        z: f64,
        rot: [f64; 3],
    },
    /// A pose known from the start: `[absheight]` objects, objects joined to splines by
    /// `[splinehelper]` (crossings, switches) and spline attachment rows.
    Pose(Pose),
    /// `[attachObj]`: attachment point `index` of object `parent`, turned by `rot`.
    Attached {
        parent: i64,
        index: usize,
        rot: [f64; 3],
    },
}

/// An object of a tile with its type, before it stands on the final ground.
struct StagedObject {
    ot: Arc<ObjectType>,
    id: i64,
    place: Placement,
    rules: Vec<omsi_map::MapRule>,
    /// The record's trailing lines: text strings, tree parameters, a lamp's light index,
    /// a bus stop's name.
    extra: Vec<String>,
    /// The crossing a traffic light lamp belongs to (`[varparent]`, else what it hangs on).
    lamp_parent: Option<i64>,
    /// A car put on a parking space (the traffic steers round it).
    parked: bool,
    /// The record is an `[object]` (its position goes into `object_positions`).
    map_object: bool,
    /// The instance of a spline-attachment row this object represents.
    instance: usize,
    /// What the collisions call this object: its map id, or for an object of a spline
    /// attachment row (which all share the row's id) a key of its own, so that one post of
    /// a row of `[crashmode_pole]` bollards falls alone.
    key: i64,
}

/// The collision key of object `index` of spline attachment row `row` in tile (tx, ty):
/// above every map id (2^53 and up), and the same on every load.
fn row_object_key(tx: i32, ty: i32, row: i64, index: usize) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in [tx as i64 as u64, ty as i64 as u64, row as u64, index as u64] {
        h = (h ^ v).wrapping_mul(0x0000_0100_0000_01b3);
        h ^= h >> 29;
    }
    ((1u64 << 53) | (h & ((1u64 << 53) - 1))) as i64
}

/// A spline of a staged tile as the rasters see it.
struct StagedSpline {
    /// Positions (relative to the tile origin) and triangles only.
    shape: MeshData,
    ty: Arc<SplineType>,
    /// World bounds (x0, y0, x1, y1).
    bounds: [f64; 4],
    /// It carries a road or a footway (a railway embankment or a bridge deck does not).
    drivable: bool,
    /// Every profile of it is blended (`[matl_alpha] 2`): a layer laid over the ground or a
    /// road, not a surface of its own (see `prepare_surfaces`).
    overlay: bool,
    /// It is ground (see `SPLINE_OVERHEAD`): it goes into the surface raster, cutting the
    /// terrain where that comes up through it. Overhead wires do not.
    cuts_terrain: bool,
    /// It stands clear of the ground all along (see `SPLINE_SHADOW_CLEARANCE`): it casts a
    /// sun shadow.
    casts_shadow: bool,
    /// Start point used by OMSI's far-to-near blend sort for spline surfaces.
    sort_origin: DVec3,
}

/// How far a spline has to stand clear of the ground under it, everywhere, before it casts
/// a sun shadow: a bridge deck, a viaduct or an elevated railway does, a road lying on the
/// terrain does not - a caster in one plane with what it falls on paints dark patches into
/// it (the sun shadow's bias is 6 cm). Splines are surfaces and cast nothing otherwise.
const SPLINE_SHADOW_CLEARANCE: f32 = 0.75;
/// A spline whose profiles all hang this far (m) over its line - wires, catenaries, a
/// canopy - is no ground surface: it neither cuts the terrain nor carries anything.
const SPLINE_OVERHEAD: f32 = 2.0;

/// Whether a plain object (no `[rendertype]`, no `[surface]`) is only paint at its foot:
/// every point of every mesh between 5 cm under and 25 cm over its origin, all of them
/// within 5 cm of height (a plate standing upright - a line plate on a bridge rail, a stop's
/// name plate - is a sign, not paint; the mesh pivots are for animations, not for the
/// points). Omsi.exe draws such a marking after the roads at its own height, a few
/// millimetres over the road it was made for - a bus bay's lines (`Korean Road Object\
/// Parkinglot\Parkbox(bus).sco`) lie 5 mm over a road at 10 cm. The roads here are pulled
/// towards the eye by their depth bias instead, and an object lying that close over one
/// went under it: a depot's parking bays were all gone (#1009).
fn paint_at_foot(sco: &SceneryObject, meshes: &[(MeshData, Vec<omsi_o3d::Material>, Vec<MaterialDef>)]) -> bool {
    if sco.render_type.is_ground_layer() || sco.surface || meshes.is_empty() || meshes.iter().any(|(m, _, _)| m.positions.is_empty()) {
        return false;
    }
    let (lo, hi) = meshes.iter().flat_map(|(m, _, _)| m.positions.iter()).fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.z), hi.max(p.z)));
    lo >= -0.05 && hi <= 0.25 && hi - lo <= 0.05
}

fn spline_paint_slots(def: &omsi_scenery::sli::Spline) -> Vec<usize> {
    // Painted profiles compose after the roads commit depth, like plain-object
    // paint. A stripe can share a spline with asphalt: classifying the whole
    // cross-section left those edge lines in the camera-dependent blend sort.
    if !def.paths.is_empty() || !def.rail_enh.is_empty() || !def.third_rail.is_empty()
    {
        return Vec::new();
    }
    let painted = |p: &omsi_scenery::sli::SplineProfile| {
        if p.points.len() < 2 || def.textures.get(p.texture).is_none_or(|t| t.alpha != 2)
            || p.points.iter().any(|q| !q.x.is_finite() || !q.z.is_finite())
        {
            return false;
        }
        let (x0, x1, z0, z1) = p.points.iter().fold(
            (f32::MAX, f32::MIN, f32::MAX, f32::MIN),
            |(x0, x1, z0, z1), p| (x0.min(p.x), x1.max(p.x), z0.min(p.z), z1.max(p.z)),
        );
        x1 > x0 && x1 - x0 <= 0.5 && z0 >= -0.05 && z1 <= 0.25 && z1 - z0 <= 0.05
    };
    let mut slots = Vec::new();
    for p in &def.profiles {
        // Never move asphalt that happens to use the stripe's material too.
        if !slots.contains(&p.texture)
            && def.profiles.iter().filter(|q| q.texture == p.texture).all(&painted)
        {
            slots.push(p.texture);
        }
    }
    slots
}

fn spline_render_phase(def: &omsi_scenery::sli::Spline) -> RenderPhase {
    let slots = spline_paint_slots(def);
    if !def.profiles.is_empty() && def.profiles.iter().all(|p| slots.contains(&p.texture)) {
        RenderPhase::BeforeNormal
    } else {
        RenderPhase::Spline
    }
}

fn split_spline_paint(src: &MeshData, def: &omsi_scenery::sli::Spline) -> (MeshData, MeshData) {
    let slots = spline_paint_slots(def);
    if slots.is_empty() { return (src.clone(), MeshData::default()); }
    let part = |paint| {
        let mut out = MeshData { one_sided: src.one_sided, ..Default::default() };
        let mut vertices = HashMap::new();
        for &(start, count, slot) in &src.ranges {
            if slots.contains(&(slot as usize)) != paint { continue; }
            let first = out.indices.len() as u32;
            for &k in &src.indices[start as usize..(start + count) as usize] {
                let v = *vertices.entry(k).or_insert_with(|| {
                    out.positions.push(src.positions[k as usize]);
                    out.normals.push(src.normals.get(k as usize).copied().unwrap_or(glam::Vec3::Z));
                    out.uvs.push(src.uvs.get(k as usize).copied().unwrap_or(glam::Vec2::ZERO));
                    out.positions.len() as u32 - 1
                });
                out.indices.push(v);
            }
            if count > 0 { out.ranges.push((first, count, slot)); }
        }
        out
    };
    (part(false), part(true))
}

fn scenery_render_phase(kind: omsi_scenery::sco::RenderType) -> RenderPhase {
    use omsi_scenery::sco::RenderType as ScoPhase;
    match kind {
        ScoPhase::PreSurface => RenderPhase::PreSurface,
        ScoPhase::Surface => RenderPhase::Surface,
        ScoPhase::OnSurface => RenderPhase::OnSurface,
        ScoPhase::BeforeNormal => RenderPhase::BeforeNormal,
        ScoPhase::AfterNormal => RenderPhase::AfterNormal,
        ScoPhase::AfterVehicles => RenderPhase::AfterVehicles,
        ScoPhase::Normal => RenderPhase::Normal,
    }
}

/// Does every profile of the spline hang `SPLINE_OVERHEAD` or more over its line?
fn overhead_only(def: &omsi_scenery::sli::Spline) -> bool {
    !def.profiles.is_empty() && def.profiles.iter().all(|p| !p.points.is_empty() && p.points.iter().all(|q| q.z >= SPLINE_OVERHEAD))
}

/// Which `parklist_p` a car park draws from: its first map string, as a number (Omsi.exe
/// sub_79c8b8 - `StrToInt`, 0 when that fails or there is none). 0 is `parklist_p.txt`,
/// n is `parklist_p_n.txt`.
fn parklist_index(strings: &[String]) -> usize {
    strings.first().and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(0)
}

/// A tile read and tessellated, its objects typed but not yet standing on the ground. Kept
/// (by [`World::prepare_tiles`]) while a loaded tile or one on its way depends on it.
pub struct StagedTile {
    tx: i32,
    ty: i32,
    origin: DVec3,
    path: PathBuf,
    /// The terrain as the tile file has it, before roads and crossings pulled it about.
    base_terrain: Terrain,
    /// `[spline_terrain_align]` splines: (index into `splines`, reach in metres).
    align: Vec<(usize, f32)>,
    /// The hole boundaries in world space, including the profile's authored height.
    hole_rims: Vec<Vec<DVec3>>,
    water: Option<[f32; 4]>,
    /// `[variable_terrainlightmap]`: the tile's light map is baked from the lamps around it
    /// (see [`bake_light_map`]), not read from its `.map.LM.bmp`.
    bakes_light_map: bool,
    splines: Vec<StagedSpline>,
    /// The whole spline meshes, in the order of `splines`, until the tile is placed.
    meshes: Mutex<Option<Vec<Arc<MeshData>>>>,
    /// The `[heightprofile]` surfaces of the tile's splines (local to `origin`) with their
    /// world bounds and `.surf` maps: what the wheels roll on.
    drive: Vec<(MeshData, [f64; 4], Option<omsi_geometry::SurfFaces>)>,
    /// Lanes, taken when the tile is loaded for the first time.
    lanes: Mutex<Vec<Lane>>,
    /// The street lanes' points of the tile's splines, kept for good (what an object's box
    /// is checked against: a road through it makes it no wall).
    street_points: Vec<DVec3>,
    objects: Vec<StagedObject>,
    /// The spline attachment rows `[attachObj]` records can hang on: (row id, where its first
    /// object stands, the row's own type - a car park row's, not its car's).
    anchors: Vec<(i64, Pose, Arc<ObjectType>)>,
    /// What reading the tile counted (missing types, empty car parks, rows, attachments).
    counts: LoadStats,
    resolved: std::sync::OnceLock<Arc<Resolved>>,
}

/// A staged tile's final ground and where its objects finally stand.
struct Resolved {
    terrain: Arc<Terrain>,
    /// Crossings warped onto the ground: object index → its own meshes.
    warped: HashMap<usize, Arc<Vec<MeshData>>>,
    /// By object index; None for an attachment without its parent or attachment point.
    poses: Vec<Option<Pose>>,
    unattached: usize,
    aligned_points: usize,
    biggest: Option<(f32, f64, f64)>,
    deformed: bool,
}

/// The tile and its eight neighbours.
const NEIGHBOURHOOD: [(i32, i32); 9] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (0, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// How far outside a tile a spline still counts for it: the terrain alignment reaches 20 m
/// beyond, and a crossing plate standing on the tile's edge looks for the roads that run
/// into it well past that.
const SOURCE_MARGIN: f64 = 150.0;

/// The tiles of a map and what each depends on.
pub struct TileLayout {
    pub paths: HashMap<(i32, i32), PathBuf>,
    /// Tile → the tiles whose splines, crossings or ground its own final ground, crossings
    /// and cut are made from: its neighbours, and tiles with splines reaching it from
    /// further away. The same for a whole-map load as for streaming.
    sources: HashMap<(i32, i32), Vec<(i32, i32)>>,
}

impl TileLayout {
    pub fn sources_of(&self, key: (i32, i32)) -> &[(i32, i32)] {
        self.sources.get(&key).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// The tile and its neighbours that exist.
    pub fn ring(&self, key: (i32, i32)) -> impl Iterator<Item = (i32, i32)> + '_ {
        NEIGHBOURHOOD
            .iter()
            .map(move |(dx, dy)| (key.0 + dx, key.1 + dy))
            .filter(|k| self.paths.contains_key(k))
    }
}

/// A placed scenery object, ready for the GPU.
pub struct PlacedObject {
    ot: Arc<ObjectType>,
    pos: DVec3,
    xf: Mat4,
    /// Traffic light lamp: (crossing id, light index).
    lamp: Option<(i64, usize, bool)>,
    map_id: i64,
    /// Collision key (see [`StagedObject::key`]).
    key: i64,
    controller: Option<usize>,
    strings: Vec<String>,
    warped: Option<Arc<Vec<MeshData>>>,
    /// `[varparent]` of the record.
    var_parent: Option<i64>,
    /// A car on a `[carpark_p]` space.
    parked: bool,
    /// A tile's own `[object]` record standing on the ground: the object editor may move it.
    editable: bool,
    script: Option<omsi_sim::scenery::SceneryInstance>,
}

/// A scenery object the object editor can take hold of (see [`World::edit_objects`]).
#[derive(Clone)]
pub struct EditObject {
    pub tile: (i32, i32),
    pub pos: DVec3,
    pub xf: Mat4,
    /// Collision key (its boxes carry it).
    pub key: i64,
    pub instances: Vec<usize>,
    /// Its `.sco`, for the editor's display.
    pub sco: std::path::PathBuf,
}

/// What the object editor did to one object: moved by `moved` (m), turned by `turned`
/// (degrees clockwise, as a map object's heading), or taken away.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ObjectEdit {
    pub moved: DVec3,
    pub turned: f64,
    pub deleted: bool,
}

/// A parked car of a loaded tile (see [`World::parked_objects`]).
#[derive(Clone)]
pub struct ParkedObject {
    pub tile: (i32, i32),
    pub pos: DVec3,
    pub heading: f64,
    /// Its `.sco` (the AI car of the same folder is what drives off in its place).
    pub sco: std::path::PathBuf,
    /// Its instances (every LOD).
    pub instances: Vec<usize>,
}

/// A tile ready for upload: everything computed, nothing on the GPU yet.
pub struct Prepared {
    pub tx: i32,
    pub ty: i32,
    terrain: Option<MeshData>,
    /// The terrain's exposed sides, drawn separately so the hole mask cannot cut them.
    hole_walls: MeshData,
    /// Ground painting: for every `[groundtex]` layer above the first that is painted on
    /// this tile, its index and the alpha mask the editor's brush left behind (as read;
    /// [`World::cut_terrain`] turns them into `paint`).
    paint_masks: Vec<(usize, Image)>,
    /// The painted layers ready for the GPU: index, mask (with the roads' cut taken out)
    /// and the painted fraction of the tile.
    paint: Vec<(usize, TextureData, f32)>,
    /// The same brush masks without the hole cut, for the exposed terrain sides.
    wall_paint: Vec<(usize, TextureData)>,
    /// `tile.map.water`: the height of the tile's water surface at its four corners.
    water: Option<[f32; 4]>,
    /// Spline meshes are local to the tile origin.
    /// Spline meshes (local to the tile origin), their type and whether they cast a shadow.
    splines: Vec<(Arc<MeshData>, Arc<SplineType>, bool, DVec3)>,
    /// Terrain-mapped spline faces pooled across types within spatial cells.
    ground_splines: Vec<Arc<MeshData>>,
    objects: Vec<PlacedObject>,
    /// (type, texture, position, height, width, heading)
    trees: Vec<(Arc<ObjectType>, String, DVec3, f64, f64, f64)>,
    origin: DVec3,
    light_map: Option<TextureData>,
    /// The cut the roads make into the ground (alpha 0 = cut), in tile space.
    cut: Option<TextureData>,
    /// Textures prepared on the worker, by file (shared by the tiles of a batch).
    images: Arc<HashMap<PathBuf, Arc<TextureData>>>,
}

/// A prepared tile on its way to the GPU (see [`World::begin_upload`]).
pub struct PendingUpload {
    pub prepared: Prepared,
    textures: Vec<PathBuf>,
    types: Vec<Arc<ObjectType>>,
    tg: TileGpu,
    placing: Placing,
}

/// How far [`World::place_step`] got with a tile, and what it made so far.
#[derive(Default)]
struct Placing {
    /// 0: the ground, 1: splines, 2: trees, 3: objects, 4: done.
    phase: u8,
    /// The next spline or tree of the phase.
    next: usize,
    ground_next: usize,
    night_slots: Vec<(usize, usize, MaterialId, MaterialId)>,
    night_modes: Vec<NightMode>,
    light_objects: Vec<LightObject>,
    poles: Vec<i64>,
    splines: usize,
    trees: usize,
    objects: usize,
    /// Seconds per phase (OMSI_PROFILE).
    secs: [f64; 4],
    /// `[terrainmapping]` uses the first [groundtex], without the roads' cut (which
    /// would punch holes into a traffic island). Painted terrain layers belong to the
    /// ground itself and must not be projected onto a spline verge or object.
    terrain_mapping_mat: Option<MaterialId>,
}

impl PendingUpload {
    pub fn key(&self) -> (i32, i32) {
        (self.prepared.tx, self.prepared.ty)
    }
}

/// What a loaded tile added to the world, so that unloading it can take it away again.
#[derive(Default)]
pub struct TileState {
    pub bus_stops: Vec<(i64, DVec3, f64, String)>,
    /// The tile's waiting places (see `World::waiting_places`).
    pub waiting_places: Vec<(i64, DVec3, f64, f32)>,
    pub obstacles: Vec<omsi_sim::collision::Obb>,
    /// The boxes of the tile's parked cars (also among `obstacles`), for the pedestrians.
    pub parked_boxes: Vec<omsi_sim::collision::Obb>,
    /// Objects that collide with their `[collision_mesh]`.
    pub mesh_obstacles: Vec<omsi_sim::collision::MeshObstacle>,
    pub coronas: Vec<StaticCorona>,
    pub lights: Vec<omsi_render::PointLight>,
    pub light_objects: Vec<LightObject>,
    pub night_slots: Vec<(usize, usize, MaterialId, MaterialId)>,
    /// The tile's objects whose night textures follow a `[NightMapMode]` timetable.
    pub night_modes: Vec<NightMode>,
    /// Collision keys of the tile's `[crashmode_pole]` posts (in `World::poles`).
    pub poles: Vec<i64>,
    /// The tile's objects that stop the outside camera.
    pub blockers: Vec<crate::camera_arm::Blocker>,
    /// The boxes of the tile's `[petrolstation]` objects (see `World::petrol_stations`).
    pub petrol_stations: Vec<omsi_sim::collision::Obb>,
    /// Parked cars the tile placed (counted in `World::parked_live`).
    pub parked_count: usize,
    /// The tile's echoing places (see `World::reverb_zones`).
    pub reverb_zones: Vec<(omsi_sim::collision::Obb, f32, f32)>,
    pub gpu: TileGpu,
}

/// The GPU resources a tile holds: its own (terrain, splines, masks, text) and the shared
/// ones it uses (object and spline types, textures through them).
#[derive(Default)]
pub struct TileGpu {
    pub instances: Vec<usize>,
    pub meshes: Vec<MeshId>,
    pub textures: Vec<TextureId>,
    pub materials: Vec<MaterialId>,
    /// Shared textures the tile uses directly (ground layers).
    pub shared_textures: Vec<PathBuf>,
    pub types: Vec<usize>,
    pub spline_types: Vec<usize>,
    pub trees: Vec<String>,
    /// Sign texts the tile's objects show (shared, see `GpuCache::text_textures`).
    pub texts: Vec<String>,
}

struct TexEntry {
    id: TextureId,
    alpha: bool,
    users: usize,
    /// Texels of the uploaded image (mip levels not counted).
    texels: u64,
    /// Bytes on the GPU (all levels, as they are now).
    bytes: u64,
    format: omsi_texture::PixelFormat,
    /// Finest mip levels let go while the textures are over their budget.
    dropped: u32,
}

/// An object type on the GPU.
struct TypeGpu {
    /// Keeps the type (and so the pointer that keys this entry) alive while cached.
    ot: Arc<ObjectType>,
    meshes: Vec<(MeshId, Vec<MaterialId>)>,
    /// `[matl_change]` variants: (mesh index, slot, base, item, variable).
    variants: Vec<(usize, usize, MaterialId, MaterialId, String)>,
    /// Dynamic texture overrides, made only for combinations that placed scripts use.
    /// Rows align with LOD 0 meshes and material slots; each entry holds (base, item).
    dynamic_texture_variants: HashMap<Vec<usize>, Vec<Vec<Option<(MaterialId, MaterialId)>>>>,
    /// Lower LODs: (min size, max size, meshes).
    lods: Vec<(f32, f32, Vec<(MeshId, Vec<MaterialId>)>)>,
    materials: Vec<MaterialId>,
    textures: Vec<PathBuf>,
    users: usize,
    /// Some texture has a night copy in the `night` folder beside it (lit windows).
    auto_night: bool,
    /// The screen sizes from and up to which the first level (`meshes`) is drawn (see
    /// `lods`).
    lod0_lo: f32,
    lod0_max: f32,
    /// Material slots whose texture carries `[terrainmapping]`: (level: 0 the first,
    /// k the k-th of `lods`, mesh index in that level, slot).
    terrain_slots: Vec<(usize, usize, usize)>,
    /// The meshes without those slots, made once for all placements: ((level, mesh), id).
    terrain_rest: Vec<((usize, usize), MeshId)>,
}

struct SplineGpu {
    _st: Arc<SplineType>,
    materials: Vec<MaterialId>,
    textures: Vec<PathBuf>,
    users: usize,
    /// Texture slots with `[terrainmapping]` (the grass verges of Berlin-Spandau's
    /// `Splines/Ruede`): drawn with the ground of the tile, like such an object's slots.
    terrain: Vec<usize>,
}

struct TreeGpu {
    material: MaterialId,
    texture: Option<PathBuf>,
    users: usize,
}

/// Materials every tile shares (never freed).
struct GroundGpu {
    ground_id: Option<TextureId>,
    ground_mat: MaterialId,
    plain_terrain_mat: MaterialId,
    ground_detail: Option<(TextureId, f32)>,
    ground_repeats: f32,
    /// Whether the map's base ground layer (`ground_id`) carries `[moisture]`/`[puddles]`.
    ground_wet: f32,
    water_mat: MaterialId,
    tree_mesh: MeshId,
}

/// Freed scene slots, the lowest handed out first: new resources fill the front of the
/// scene's arrays, so that after a big unload their tail can be cut off (`compact_slots`).
#[derive(Default)]
struct FreeList(std::collections::BinaryHeap<std::cmp::Reverse<usize>>);

impl FreeList {
    fn push(&mut self, id: usize) {
        self.0.push(std::cmp::Reverse(id));
    }

    fn pop(&mut self) -> Option<usize> {
        self.0.pop().map(|r| r.0)
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    /// Drop the ids from `len` on (the part of the array that is cut off).
    fn keep_below(&mut self, len: usize) {
        self.0.retain(|r| r.0 < len);
    }

    /// The free ids at the end of an array of `len` slots: the new length.
    fn free_tail(&self, len: usize) -> usize {
        let mut ids: Vec<usize> = self.0.iter().map(|r| r.0).filter(|i| *i < len).collect();
        ids.sort_unstable();
        let mut n = len;
        while ids.last() == Some(&(n.wrapping_sub(1))) && n > 0 {
            ids.pop();
            n -= 1;
        }
        n
    }
}

/// Short static spline segments with matching materials share a mesh within a 48 m cell.
/// Their coordinates, material order, terrain mapping and shadow flag stay intact; long segments retain
/// their own culling bounds. The original meshes remain in the staging/collision data.
fn batch_static_splines(
    splines: Vec<(Arc<MeshData>, Arc<SplineType>, bool, DVec3)>,
) -> Vec<(Arc<MeshData>, Arc<SplineType>, bool, DVec3)> {
    if omsi_cfg::env::var_os("OMSI_NO_SPLINE_BATCHING").is_some() {
        return splines;
    }
    let mut groups: Vec<(Vec<Arc<MeshData>>, Arc<SplineType>, bool, DVec3)> = Vec::new();
    let mut cells = HashMap::new();
    let mut signatures = HashMap::new();
    let mut type_materials = HashMap::new();
    let material_batching = omsi_cfg::env::var_os("OMSI_NO_MATERIAL_SPLINE_BATCHING").is_none();
    for (mesh, ty, casts, sort_origin) in splines {
        let (lo, hi) = mesh.positions.iter().fold(
            (glam::Vec3::splat(f32::INFINITY), glam::Vec3::splat(f32::NEG_INFINITY)),
            |(lo, hi), &p| (lo.min(p), hi.max(p)),
        );
        let centre = (lo + hi) * 0.5;
        // Blended segments retain their individual placement origins and draw order.
        let blended = mesh.ranges.iter().any(|r| ty.def.textures.get(r.2 as usize).is_some_and(|t| t.alpha >= 2));
        let short = !blended && centre.is_finite() && (hi - lo).length() <= 48.0;
        let group = if short {
            let slots: Vec<_> = mesh.ranges.iter().map(|r| r.2).collect();
            // UV generation is already complete. Only the textures of the remaining
            // ranges matter now; an unused terrain slot must not split identical curbs.
            // Keep the lookup directory and alpha mode exact so similarly named files
            // in different content packs cannot be combined.
            let material_type = if material_batching && slots.iter().all(|&s| (s as usize) < ty.def.textures.len()) {
                (true, *type_materials.entry((Arc::as_ptr(&ty) as usize, slots.clone())).or_insert_with(|| {
                    let signature = (ty.dir.clone(), slots.iter().map(|&s| {
                        let texture = &ty.def.textures[s as usize];
                        (s, texture.file.clone(), texture.alpha)
                    }).collect::<Vec<_>>());
                    let next = signatures.len();
                    *signatures.entry(signature).or_insert(next)
                }))
            } else {
                (false, Arc::as_ptr(&ty) as usize)
            };
            let key = (
                material_type,
                (centre.x / 48.0).floor() as i32,
                (centre.y / 48.0).floor() as i32,
                casts,
                mesh.one_sided,
                slots,
            );
            *cells.entry(key).or_insert_with(|| {
                let i = groups.len();
                groups.push((Vec::new(), ty.clone(), casts, sort_origin));
                i
            })
        } else {
            let i = groups.len();
            groups.push((Vec::new(), ty, casts, sort_origin));
            i
        };
        groups[group].0.push(mesh);
    }
    groups.into_iter().map(|(mut meshes, ty, casts, sort_origin)| {
        let mesh = if meshes.len() == 1 {
            meshes.pop().unwrap()
        } else {
            Arc::new(MeshData::merge_static(&meshes.iter().map(AsRef::as_ref).collect::<Vec<_>>()))
        };
        (mesh, ty, casts, sort_origin)
    }).collect()
}

/// These faces all use the tile's ground materials, irrespective of the source .sli.
/// Pool them before upload so grass widths and curb types can share a ground draw.
fn batch_ground_splines(meshes: Vec<Arc<MeshData>>) -> Vec<Arc<MeshData>> {
    let mut groups: Vec<Vec<Arc<MeshData>>> = Vec::new();
    let mut cells = HashMap::new();
    for mesh in meshes {
        let (lo, hi) = mesh.positions.iter().fold(
            (glam::Vec3::splat(f32::INFINITY), glam::Vec3::splat(f32::NEG_INFINITY)),
            |(lo, hi), &p| (lo.min(p), hi.max(p)),
        );
        let centre = (lo + hi) * 0.5;
        let group = if centre.is_finite() && (hi - lo).length() <= 48.0 {
            let key = (
                (centre.x / 48.0).floor() as i32,
                (centre.y / 48.0).floor() as i32,
                (centre.z / 48.0).floor() as i32,
                mesh.one_sided,
            );
            *cells.entry(key).or_insert_with(|| {
                let i = groups.len();
                groups.push(Vec::new());
                i
            })
        } else {
            let i = groups.len();
            groups.push(Vec::new());
            i
        };
        groups[group].push(mesh);
    }
    groups.into_iter().map(|mut meshes| {
        if meshes.len() == 1 {
            meshes.pop().unwrap()
        } else {
            Arc::new(MeshData::merge_static(&meshes.iter().map(AsRef::as_ref).collect::<Vec<_>>()))
        }
    }).collect()
}

/// The GPU side of the loaded tiles: shared resources with their users, and the freed ids
/// that new resources take over.
#[derive(Default)]
pub struct GpuCache {
    textures: hashbrown::HashMap<PathBuf, TexEntry>,
    misses: hashbrown::HashSet<String>,
    types: HashMap<usize, TypeGpu>,
    splines: HashMap<usize, SplineGpu>,
    trees: HashMap<String, TreeGpu>,
    ground: Option<GroundGpu>,
    free_meshes: FreeList,
    free_textures: FreeList,
    free_materials: FreeList,
    /// Removed instances by material slot count.
    free_instances: HashMap<usize, FreeList>,
    /// Textures decoded on the thread that draws (not decoded ahead on the loader): how
    /// many, and the seconds they took (`OMSI_PROFILE`).
    sync_decodes: usize,
    sync_decode_secs: f64,
    /// Textures read here as RGBA to spare the frame, to be compressed on a worker.
    wants_upgrade: Vec<PathBuf>,
    /// Pictures read on the thread that draws are read the quick way (a window's frames;
    /// an offscreen load compresses them at once).
    fast_loads: bool,
    /// Textures that lost mip levels to the budget and are near again: read again whole.
    wants_restore: Vec<PathBuf>,
    /// Scenery sign texts (`[texttexture]`) by what they show: (texture, material, users).
    /// A street's name on twenty signs is one texture - every sign had its own, 270 MB
    /// around the Ahlheim main station.
    text_textures: HashMap<String, (TextureId, MaterialId, usize)>,
}

impl GpuCache {
    fn add_mesh(&mut self, renderer: &Renderer, scene: &mut Scene, data: &MeshData) -> MeshId {
        let id = renderer.add_mesh(scene, data);
        self.take_mesh_slot(renderer, scene, id)
    }

    fn take_mesh_slot(&mut self, renderer: &Renderer, scene: &mut Scene, id: MeshId) -> MeshId {
        match self.free_meshes.pop() {
            Some(slot) => {
                let r = renderer.recycle_mesh(scene, id, slot);
                if r != slot {
                    self.free_meshes.push(slot);
                }
                r
            }
            None => id,
        }
    }

    fn add_data(
        &mut self,
        renderer: &Renderer,
        scene: &mut Scene,
        data: &TextureData,
    ) -> TextureId {
        let id = renderer.add_texture_data(scene, data);
        self.take_texture_slot(renderer, scene, id)
    }

    /// An RGBA picture of the scene's own (a sign's text), uploaded as it is.
    fn add_image(
        &mut self,
        renderer: &Renderer,
        scene: &mut Scene,
        img: &Image,
        mipmaps: bool,
    ) -> TextureId {
        let id = renderer.add_texture(scene, img, mipmaps);
        self.take_texture_slot(renderer, scene, id)
    }

    fn add_blank(&mut self, renderer: &Renderer, scene: &mut Scene, width: u32, height: u32) -> TextureId {
        let id = renderer.add_blank_texture(scene, width, height);
        self.take_texture_slot(renderer, scene, id)
    }

    /// A transparent dynamic text texture with a full mip chain. Text textures are often
    /// viewed much smaller than their authored pixel size; without lower levels the sampler
    /// minifies level zero directly and thin glyph strokes break into unstable pixels.
    fn add_blank_mips(&mut self, renderer: &Renderer, scene: &mut Scene, width: u32, height: u32) -> TextureId {
        let (width, height) = (width.max(1), height.max(1));
        self.add_image(
            renderer,
            scene,
            &Image {
                width,
                height,
                rgba: vec![0; (width * height * 4) as usize],
                has_alpha: true,
            },
            true,
        )
    }

    fn take_texture_slot(
        &mut self,
        renderer: &Renderer,
        scene: &mut Scene,
        id: TextureId,
    ) -> TextureId {
        match self.free_textures.pop() {
            Some(slot) => {
                let r = renderer.recycle_texture(scene, id, slot);
                if r != slot {
                    self.free_textures.push(slot);
                }
                r
            }
            None => id,
        }
    }

    fn material(&mut self, renderer: &Renderer, scene: &mut Scene, id: MaterialId) -> MaterialId {
        match self.free_materials.pop() {
            Some(slot) => {
                let r = renderer.recycle_material(scene, id, slot);
                if r != slot {
                    self.free_materials.push(slot);
                }
                r
            }
            None => id,
        }
    }

    fn instance(&mut self, renderer: &Renderer, scene: &mut Scene, id: usize) -> usize {
        let slots = renderer.instance_slots(scene, id);
        match self.free_instances.get_mut(&slots).and_then(|l| l.pop()) {
            Some(slot) => {
                let r = renderer.recycle_instance(scene, id, slot);
                if r != slot {
                    self.free_instances.entry(slots).or_default().push(slot);
                }
                r
            }
            None => id,
        }
    }

    /// A texture found by OMSI's rules, uploaded on first use; the caller becomes one of its
    /// users (and must `release_texture` the returned path).
    fn texture(
        &mut self,
        renderer: &Renderer,
        scene: &mut Scene,
        name: &str,
        dirs: &[PathBuf],
        images: &HashMap<PathBuf, Arc<TextureData>>,
    ) -> Option<(TextureId, PathBuf)> {
        let dirs_ref: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
        let Some(path) = omsi_texture::find_texture(name, &dirs_ref) else {
            if !name.trim().is_empty() && self.misses.insert(name.to_string()) {
                log::warn!("Did not find texture file \"{name}\"!");
                if omsi_cfg::env::var_os("OMSI_DEBUG_MISSING").is_some() {
                    log::info!("  texture {name} looked for in {:?}", dirs);
                }
            }
            return None;
        };
        if let Some(e) = self.textures.get_mut(&path) {
            e.users += 1;
            return Some((e.id, path));
        }
        let img = match images.get(&path) {
            Some(i) => i.clone(),
            None => {
                let t = std::time::Instant::now();
                let decoded = if self.fast_loads {
                    omsi_texture::gpu::load_gpu_fast(&path)
                } else {
                    omsi_texture::gpu::load_gpu(&path).map(|(t, _)| (t, false))
                };
                self.sync_decodes += 1;
                self.sync_decode_secs += t.elapsed().as_secs_f64();
                match decoded {
                    Ok((i, worth)) => {
                        if worth {
                            self.wants_upgrade.push(path.clone());
                            // (to be swapped for the compressed whole soon: meanwhile at half
                            // the size, a quarter of the memory - uploaded whole as RGBA, a
                            // map's first tiles took a gigabyte more than compressed, and
                            // cards with little memory ran out while the game was loading)
                            Arc::new(omsi_texture::gpu::halved_for_now(i))
                        } else {
                            Arc::new(i)
                        }
                    }
                    Err(e) => {
                        if self.misses.insert(path.to_string_lossy().into_owned()) {
                            log::warn!("{e}");
                        }
                        return None;
                    }
                }
            }
        };
        let id = self.add_data(renderer, scene, &img);
        attach_pbr(renderer, scene, &path, id);
        self.textures.insert(
            path.clone(),
            TexEntry {
                id,
                alpha: img.has_alpha,
                users: 1,
                texels: img.width as u64 * img.height as u64,
                bytes: renderer.texture_size_bytes(scene, id),
                format: img.format,
                dropped: 0,
            },
        );
        Some((id, path))
    }

    fn has_alpha(&self, path: &Path) -> bool {
        self.textures.get(path).map(|e| e.alpha).unwrap_or(false)
    }

    /// Lazily make material overrides for one placed scenery object's active `[CTC]` and
    /// `[texchanges]` choices. The choices are cached by their complete per-group index
    /// vector so placements sharing a type and choices also share textures and materials.
    fn dynamic_texture_variant(
        &mut self,
        renderer: &Renderer,
        scene: &mut Scene,
        type_key: usize,
        selection: &[usize],
        root: &Path,
        images: &HashMap<PathBuf, Arc<TextureData>>,
    ) -> Option<Vec<Vec<Option<(MaterialId, MaterialId)>>>> {
        if let Some(found) = self
            .types
            .get(&type_key)?
            .dynamic_texture_variants
            .get(selection)
        {
            return Some(found.clone());
        }
        let ot = self.types.get(&type_key)?.ot.clone();
        let mut replacements: HashMap<String, (String, PathBuf)> = HashMap::new();
        let mut affected_keys: HashMap<String, ()> = HashMap::new();
        for (group, &index) in ot.dynamic_textures.iter().zip(selection) {
            for choice in &group.choices {
                for (default, _, _) in choice {
                    affected_keys.insert(scenery_texture_key(default), ());
                }
            }
            if let Some(choice) = group.choices.get(index) {
                for (default, file, dir) in choice {
                    replacements.insert(scenery_texture_key(default), (file.clone(), dir.clone()));
                }
            }
        }

        let (base_meshes, base_variants) = {
            let ty = self.types.get(&type_key)?;
            (ty.meshes.clone(), ty.variants.clone())
        };
        let mut rows: Vec<Vec<Option<(MaterialId, MaterialId)>>> = base_meshes
            .iter()
            .map(|(_, materials)| vec![None; materials.len()])
            .collect();
        for (mesh_index, (_, o3d_materials, _)) in ot.meshes.iter().enumerate() {
            let Some((_, base_materials)) = base_meshes.get(mesh_index) else {
                continue;
            };
            for (slot, source) in o3d_materials.iter().enumerate() {
                let key = scenery_texture_key(&source.texture);
                if !affected_keys.contains_key(&key) {
                    continue;
                }
                let Some(&base) = base_materials.get(slot) else {
                    continue;
                };
                let item = base_variants
                    .iter()
                    .find(|v| v.0 == mesh_index && v.1 == slot)
                    .map(|v| v.3)
                    .unwrap_or(base);
                // Always retain a reset pair. An invalid index or a missing replacement
                // texture must restore the model material after a previously valid choice.
                rows[mesh_index][slot] = Some((base, item));
                let Some((file, scheme_dir)) = replacements.get(&key) else {
                    continue;
                };
                let mut dirs = ot.texture_dirs(root);
                dirs.insert(0, scheme_dir.clone());
                let Some((texture, path)) = self.texture(renderer, scene, file, &dirs, images)
                else {
                    continue;
                };
                let Some(base_ctc) = renderer.add_material_retextured(scene, base, Some(texture))
                else {
                    self.release_texture(renderer, scene, &path);
                    continue;
                };
                let item_ctc = if item == base {
                    base_ctc
                } else if let Some(mat) =
                    renderer.add_material_retextured(scene, item, Some(texture))
                {
                    mat
                } else {
                    base_ctc
                };
                let ty = self.types.get_mut(&type_key)?;
                ty.materials.push(base_ctc);
                if item_ctc != base_ctc {
                    ty.materials.push(item_ctc);
                }
                ty.textures.push(path);
                rows[mesh_index][slot] = Some((base_ctc, item_ctc));
            }
        }
        self.types
            .get_mut(&type_key)?
            .dynamic_texture_variants
            .insert(selection.to_vec(), rows.clone());
        Some(rows)
    }

    /// A `[matl_bumpmap]` height map (`omsi_texture::gpu::prepare_bump`), shared like
    /// [`GpuCache::texture`] under its own key (`bump_key`); read here, which the two
    /// stock objects with one can afford.
    fn bump_texture(
        &mut self,
        renderer: &Renderer,
        scene: &mut Scene,
        name: &str,
        dirs: &[PathBuf],
    ) -> Option<(TextureId, PathBuf)> {
        let dirs_ref: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
        let path = omsi_texture::find_texture(name, &dirs_ref)?;
        let key = bump_key(&path);
        if let Some(e) = self.textures.get_mut(&key) {
            e.users += 1;
            return Some((e.id, key));
        }
        let t = std::time::Instant::now();
        let data = load_texture_key(&key, !self.fast_loads)?;
        self.sync_decodes += 1;
        self.sync_decode_secs += t.elapsed().as_secs_f64();
        let id = self.add_data(renderer, scene, &data);
        self.textures.insert(
            key.clone(),
            TexEntry {
                id,
                alpha: true,
                users: 1,
                texels: data.width as u64 * data.height as u64,
                bytes: renderer.texture_size_bytes(scene, id),
                format: data.format,
                dropped: 0,
            },
        );
        Some((id, key))
    }

    fn release_texture(&mut self, renderer: &Renderer, scene: &mut Scene, path: &Path) {
        let Some(e) = self.textures.get_mut(path) else {
            return;
        };
        e.users = e.users.saturating_sub(1);
        if e.users == 0 {
            let id = e.id;
            self.textures.remove(path);
            renderer.free_texture(scene, id);
            self.free_textures.push(id);
        }
    }

    fn free_material(&mut self, renderer: &Renderer, scene: &mut Scene, id: MaterialId) {
        renderer.free_material(scene, id);
        self.free_materials.push(id);
    }

    /// Give back everything a tile held.
    fn release_tile(&mut self, renderer: &Renderer, scene: &mut Scene, tg: TileGpu) -> usize {
        let mut freed_types = 0;
        for i in tg.instances {
            renderer.remove_instance(scene, i);
            let slots = renderer.instance_slots(scene, i);
            self.free_instances.entry(slots).or_default().push(i);
        }
        for m in tg.materials {
            self.free_material(renderer, scene, m);
        }
        for m in tg.meshes {
            renderer.free_mesh(scene, m);
            self.free_meshes.push(m);
        }
        for t in tg.textures {
            renderer.free_texture(scene, t);
            self.free_textures.push(t);
        }
        for p in tg.shared_textures {
            self.release_texture(renderer, scene, &p);
        }
        for key in tg.texts {
            let gone = match self.text_textures.get_mut(&key) {
                Some(e) => {
                    e.2 = e.2.saturating_sub(1);
                    e.2 == 0
                }
                None => false,
            };
            if gone {
                let (tex, mat, _) = self.text_textures.remove(&key).unwrap();
                self.free_material(renderer, scene, mat);
                renderer.free_texture(scene, tex);
                self.free_textures.push(tex);
            }
        }
        for key in tg.types {
            let gone = match self.types.get_mut(&key) {
                Some(t) => {
                    t.users = t.users.saturating_sub(1);
                    t.users == 0
                }
                None => false,
            };
            if gone {
                let t = self.types.remove(&key).unwrap();
                let mut meshes: Vec<MeshId> = t.meshes.iter().map(|m| m.0).collect();
                meshes.extend(t.lods.iter().flat_map(|l| l.2.iter().map(|m| m.0)));
                meshes.extend(t.terrain_rest.iter().map(|r| r.1));
                for m in meshes {
                    renderer.free_mesh(scene, m);
                    self.free_meshes.push(m);
                }
                for m in t.materials {
                    self.free_material(renderer, scene, m);
                }
                for p in t.textures {
                    self.release_texture(renderer, scene, &p);
                }
                drop(t.ot);
                freed_types += 1;
            }
        }
        for key in tg.spline_types {
            let gone = match self.splines.get_mut(&key) {
                Some(s) => {
                    s.users = s.users.saturating_sub(1);
                    s.users == 0
                }
                None => false,
            };
            if gone {
                let s = self.splines.remove(&key).unwrap();
                for m in s.materials {
                    self.free_material(renderer, scene, m);
                }
                for p in s.textures {
                    self.release_texture(renderer, scene, &p);
                }
            }
        }
        for key in tg.trees {
            let gone = match self.trees.get_mut(&key) {
                Some(t) => {
                    t.users = t.users.saturating_sub(1);
                    t.users == 0
                }
                None => false,
            };
            if gone {
                let t = self.trees.remove(&key).unwrap();
                self.free_material(renderer, scene, t.material);
                if let Some(p) = t.texture {
                    self.release_texture(renderer, scene, &p);
                }
            }
        }
        freed_types
    }
}

/// A texture of a tile's own (its night light map, the roads' cut, a ground paint mask) for
/// the GPU: one level as always, compressed where the device takes blocks and the picture
/// stays close (`mask`: only the alpha channel is read).
fn tile_texture(img: Image, mask: bool) -> TextureData {
    omsi_texture::gpu::prepare_single_level(img, mask).0
}

/// Edge (texels) a ground paint mask is brought up to before it is smoothed.
const PAINT_MASK_MIN: usize = 512;

/// A ground paint mask made ready to be drawn: the editor's brush writes nothing but 0 and
/// 255, one texel every 0.6-3 m (2^params[0] texels a tile). Sampled as it is, the edge of
/// a car park or a field follows the texel grid, and the shader's sharpening (see
/// `fs_main`) turned that into hard steps - a staircase along every painted edge, metres
/// long where the edge runs nearly along the grid. Brought to at least
/// `PAINT_MASK_MIN` texels (bilinearly) and blurred by a little under one of its own
/// texels, the mask becomes a soft ramp whose half-way line is a smooth curve through the
/// steps' middles; the shader's sharpening then gives a crisp edge along that curve.
/// Returns the alpha as RGBA (white) and the new edge lengths.
fn smooth_paint_mask(rgba: &[u8], w: usize, h: usize) -> (Vec<u8>, usize, usize) {
    let s = (PAINT_MASK_MIN / w.max(1)).max(1).min(PAINT_MASK_MIN / h.max(1)).max(1);
    let (dw, dh) = (w * s, h * s);
    let src = |i: isize, j: isize| -> f32 {
        let i = i.clamp(0, w as isize - 1) as usize;
        let j = j.clamp(0, h as isize - 1) as usize;
        rgba[(j * w + i) * 4 + 3] as f32
    };
    // bilinear magnification, texel centres aligned as the GPU samples them
    let mut a = vec![0f32; dw * dh];
    for y in 0..dh {
        let fy = (y as f32 + 0.5) / s as f32 - 0.5;
        let (j0, ty) = (fy.floor() as isize, fy - fy.floor());
        for x in 0..dw {
            let fx = (x as f32 + 0.5) / s as f32 - 0.5;
            let (i0, tx) = (fx.floor() as isize, fx - fx.floor());
            let top = src(i0, j0) * (1.0 - tx) + src(i0 + 1, j0) * tx;
            let bottom = src(i0, j0 + 1) * (1.0 - tx) + src(i0 + 1, j0 + 1) * tx;
            a[y * dw + x] = top * (1.0 - ty) + bottom * ty;
        }
    }
    // separable Gaussian, sigma 0.85 of a source texel (edges clamped)
    let sigma = 0.85 * s as f32;
    let r = (sigma * 3.0).ceil() as isize;
    let kernel: Vec<f32> = (-r..=r)
        .map(|k| (-(k * k) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let norm: f32 = kernel.iter().sum();
    let mut tmp = vec![0f32; dw * dh];
    for y in 0..dh {
        let row = &a[y * dw..][..dw];
        for x in 0..dw {
            let mut acc = 0.0;
            for (k, wk) in kernel.iter().enumerate() {
                let xi = (x as isize + k as isize - r).clamp(0, dw as isize - 1) as usize;
                acc += row[xi] * wk;
            }
            tmp[y * dw + x] = acc / norm;
        }
    }
    let mut out = vec![255u8; dw * dh * 4];
    for y in 0..dh {
        for x in 0..dw {
            let mut acc = 0.0;
            for (k, wk) in kernel.iter().enumerate() {
                let yi = (y as isize + k as isize - r).clamp(0, dh as isize - 1) as usize;
                acc += tmp[yi * dw + x] * wk;
            }
            out[(y * dw + x) * 4 + 3] = (acc / norm).round().clamp(0.0, 255.0) as u8;
        }
    }
    (out, dw, dh)
}

/// The alpha (0..1) of an image at (u, v) in 0..1, sampled bilinearly with the texel
/// centres where the GPU has them (edges clamped).
fn bilinear_alpha(img: &Image, u: f32, v: f32) -> f32 {
    let (w, h) = (img.width as isize, img.height as isize);
    let fx = u * w as f32 - 0.5;
    let fy = v * h as f32 - 0.5;
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let a = |x: isize, y: isize| -> f32 {
        let x = x.clamp(0, w - 1) as usize;
        let y = y.clamp(0, h - 1) as usize;
        img.rgba[(y * w as usize + x) * 4 + 3] as f32 / 255.0
    };
    let (x0, y0) = (x0 as isize, y0 as isize);
    let top = a(x0, y0) * (1.0 - tx) + a(x0 + 1, y0) * tx;
    let bottom = a(x0, y0 + 1) * (1.0 - tx) + a(x0 + 1, y0 + 1) * tx;
    top * (1.0 - ty) + bottom * ty
}

/// Split the material slots of an object mesh whose texture carries `[terrainmapping]`
/// off into a mesh of their own. OMSI does not draw such a slot with its texture (the
/// stock ones are a 1x1 placeholder, TH_Wald's Gras01.dds a single green pixel): the slot
/// takes on the map's first ground texture, so that the grass on top of a rock, a
/// traffic island or a roundabout runs on seamlessly from the meadow around it. The split
/// mesh therefore gets the terrain's own uv (tile space, see `build_terrain_mesh`) for
/// the object placed at `pos`/`xf` on the tile at `origin`, and is drawn with the tile's
/// uncut base material. Returns the mesh without those slots and the split-off one.
#[cfg(test)]
fn split_terrain_mapped(
    src: &MeshData,
    slots: &[usize],
    pos: DVec3,
    xf: Mat4,
    origin: DVec3,
) -> (MeshData, MeshData) {
    (terrain_rest(src, slots), terrain_ground(src, slots, pos, xf, origin))
}

/// The mesh without its `[terrainmapping]` slots (see `split_terrain_mapped`): the same for
/// every placement of a type, so it is made once per type.
fn terrain_rest(src: &MeshData, slots: &[usize]) -> MeshData {
    let mut rest = src.clone();
    rest.ranges.retain(|r| !slots.contains(&(r.2 as usize)));
    rest
}

/// The `[terrainmapping]` slots of a mesh in tile space (see `split_terrain_mapped`).
fn terrain_ground(src: &MeshData, slots: &[usize], pos: DVec3, xf: Mat4, origin: DVec3) -> MeshData {
    let mut ground = MeshData {
        one_sided: src.one_sided,
        ..MeshData::default()
    };
    let mut map: HashMap<u32, u32> = HashMap::new();
    let to_tile = (pos - origin) / tile_size();
    for &(start, count, slot) in &src.ranges {
        if !slots.contains(&(slot as usize)) {
            continue;
        }
        for &k in &src.indices[start as usize..(start + count) as usize] {
            let v = *map.entry(k).or_insert_with(|| {
                let p = src.positions[k as usize];
                let local = xf.transform_point3(p).as_dvec3() / tile_size() + to_tile;
                ground.positions.push(p);
                ground.normals.push(src.normals.get(k as usize).copied().unwrap_or(glam::Vec3::Z));
                ground.uvs.push(glam::Vec2::new(local.x as f32, local.y as f32));
                ground.positions.len() as u32 - 1
            });
            ground.indices.push(v);
        }
    }
    let n = ground.indices.len() as u32;
    if n > 0 {
        ground.ranges.push((0, n, 0));
    }
    ground
}

/// Two crossed unit quads (1 m wide, 1 m tall, centred at x=0, standing on z=0).
/// Windy trees: whether a scenery object is a plant whose leaves the wind moves - by its
/// `[groups]` (the stock "Trees LQ", "Deciduous", "Shrubbery", "Arbors", "Plants" and their
/// kin in other content), or, for an object filed in no group, by the words of its file's
/// name - and if so how much it gives to the wind. Not a `[tree]` (its cards have their
/// own) and not a backdrop (a forest painted on one wide card).
fn vegetation_give_of(sco: &omsi_scenery::sco::SceneryObject) -> Option<f32> {
    const GROUP_WORDS: [&str; 18] = ["tree", "baum", "bäume", "baeume", "deciduous", "conifer", "shrub", "bush", "busch", "strauch", "hecke", "hedge", "plant", "pflanz", "arbor", "vegetation", "forest", "wald"];
    const NAME_WORDS: [&str; 15] = ["tree", "trees", "baum", "shrub", "shrubbery", "bush", "busch", "strauch", "hecke", "hedge", "arbor", "chestnut", "kastanie", "palm", "plant"];
    if sco.tree.is_some() {
        return None;
    }
    let groups: Vec<String> = sco.groups.iter().map(|g| g.trim().to_lowercase()).filter(|g| !g.is_empty()).collect();
    if groups.iter().any(|g| g.contains("backdrop")) {
        return None;
    }
    let stem = sco.path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let plant = if groups.is_empty() {
        stem.split(|c: char| !c.is_alphabetic()).any(|w| NAME_WORDS.contains(&w))
    } else {
        groups.iter().any(|g| GROUP_WORDS.iter().any(|w| g.contains(w)))
    };
    plant.then(|| vegetation_give(&[&stem, &groups.join(" ")]))
}

/// The windy trees' pivot of a plant model's leaf slot (`MaterialExtra::sway`, mesh units):
/// the crown leaves the trunk where the slot's lowest leaves hang, and at a quarter of the
/// plant's height at least (a picture of the whole tree on crossed cards, trunk and all,
/// is one slot); a plant up to 3 m tall - a shrub, a hedge - bends from the ground.
fn foliage_sway(meshes: &[(MeshData, Vec<omsi_o3d::Material>, Vec<MaterialDef>)], mesh: &MeshData, slot: u32, give: f32) -> Option<[f32; 3]> {
    let (lo, hi) = meshes
        .iter()
        .flat_map(|(m, _, _)| m.positions.iter())
        .fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.z), hi.max(p.z)));
    let leaf_lo = mesh
        .ranges
        .iter()
        .filter(|r| r.2 == slot)
        .flat_map(|r| mesh.indices.get(r.0 as usize..(r.0 + r.1) as usize).unwrap_or(&[]))
        .filter_map(|&i| mesh.positions.get(i as usize))
        .fold(f32::MAX, |lo, p| lo.min(p.z));
    let height = hi - lo;
    if !(height > 0.2) || leaf_lo == f32::MAX {
        return None;
    }
    let pivot = if height <= 3.0 { lo } else { leaf_lo.max(lo + 0.25 * height) };
    (hi - pivot > 0.1).then_some([pivot, hi, if height <= 3.0 { 0.5 * give } else { give }])
}

/// How much a tree gives to the wind by its kind (windy trees): a conifer's needles and
/// stiff whorls of branches move less than a broadleaf's crown.
fn vegetation_give(names: &[&str]) -> f32 {
    const CONIFER: [&str; 12] = ["fir", "tanne", "fichte", "kiefer", "pine", "spruce", "conifer", "nadel", "cypress", "zypresse", "thuja", "larch"];
    let conifer = names.iter().any(|n| {
        let n = n.to_ascii_lowercase();
        CONIFER.iter().any(|c| n.contains(c))
    });
    if conifer { 0.6 } else { 1.0 }
}

/// The windy trees' pivot of a `[tree]`'s cards (`MaterialExtra::sway`, in the card's units
/// of its height): the crown leaves the trunk at about a quarter of the picture's height; a
/// shrub (a type no taller than 3 m) bends from the ground.
fn tree_card_sway(ot: &ObjectType, texture: &str) -> [f32; 3] {
    let shrub = ot.sco.tree.as_ref().is_some_and(|t| t.2 > 0.0 && t.2 <= 3.0);
    let path = ot.sco.path.to_string_lossy();
    [if shrub { SHRUB_CARD_PIVOT } else { TREE_CARD_PIVOT }, 1.0, vegetation_give(&[texture, &path])]
}

/// Where a `[tree]` card's crown leaves its trunk, and a shrub's (of the card's height).
const TREE_CARD_PIVOT: f32 = 0.28;
const SHRUB_CARD_PIVOT: f32 = 0.03;

fn tree_quad_mesh() -> MeshData {
    // Each card is a small grid of quads rather than one: a flat grid is the same picture,
    // and it gives the windy trees (shader.wgsl `tree_sway`) vertices to bend the crown by
    // while the trunk below the pivot stays where it stands. The rows lie on the pivots
    // (the bend starts exactly there) and evenly over the crown, enough for its curve; the
    // bend is the same across the card, so two columns do (the boughs' lobes are a crown
    // wide). 12 quads a side: every tree of a forest pays for it.
    const TREE_COLS: u32 = 2;
    const ROWS: [f32; 7] = [0.0, SHRUB_CARD_PIVOT, TREE_CARD_PIVOT, 0.46, 0.64, 0.82, 1.0];
    const TREE_ROWS: u32 = ROWS.len() as u32 - 1;
    let mut m = MeshData::default();
    for (dx, dy) in [(0.5f32, 0.0f32), (0.0, 0.5)] {
        let base = m.positions.len() as u32;
        for r in 0..=TREE_ROWS {
            for c in 0..=TREE_COLS {
                let u = c as f32 / TREE_COLS as f32;
                let z = ROWS[r as usize];
                let sx = u * 2.0 - 1.0;
                m.positions.push(glam::Vec3::new(dx * sx, dy * sx, z));
                m.normals.push(glam::Vec3::Z);
                m.uvs.push(glam::Vec2::new(u, 1.0 - z));
            }
        }
        let at = |c: u32, r: u32| base + r * (TREE_COLS + 1) + c;
        for r in 0..TREE_ROWS {
            for c in 0..TREE_COLS {
                let (a, b, cc, d) = (at(c, r), at(c + 1, r), at(c + 1, r + 1), at(c, r + 1));
                // both sides
                m.indices.extend_from_slice(&[a, b, cc, a, cc, d, a, cc, b, a, d, cc]);
            }
        }
    }
    m.ranges.push((0, m.indices.len() as u32, 0));
    m
}

/// Where textures are looked up for a given content directory.
/// How far a road surface may ride above the ground and still have the ground cut away
/// under it. Anything higher is a bridge or an embankment, where cutting would open a hole.
/// `OMSI_HEIGHTPROFILE_GROUND=1`: the wheels stand on the splines' `[heightprofile]`s as
/// they did before, instead of on the drawn splines as Omsi.exe stands them (A/B runs).
/// `OMSI_CHECK_ROADS`: road points under the ground, and where.
static OVER_ROAD: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static OVER_ROAD_AT: std::sync::Mutex<Vec<(f64, f64, f32, f32)>> = std::sync::Mutex::new(Vec::new());

fn heightprofile_ground() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| omsi_cfg::env::var_os("OMSI_HEIGHTPROFILE_GROUND").is_some())
}

fn surface_flush() -> f32 {
    static V: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        omsi_cfg::env::var("OMSI_SURFACE_FLUSH")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.12)
    })
}

/// Whether this session's weather lies as snow (`[snow]` in the `.owt`), set by the app
/// when it reads the weather and asked while the vehicles go onto the GPU.
pub static SNOW_WEATHER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn snowing() -> bool {
    SNOW_WEATHER.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether a texture name resolves to the season's own copy of it (`texture\WinterSnow\…`,
/// or a folder the season falls back to):
/// a vehicle that brings its own winter picture keeps it.
fn seasonal_texture(name: &str, dirs: &[&Path]) -> bool {
    let seasons = omsi_texture::season_folders();
    match omsi_texture::find_texture(name, dirs) {
        Some(p) if !seasons.is_empty() => p.components().any(|c| {
            let c = c.as_os_str().to_string_lossy();
            seasons.iter().any(|s| c.eq_ignore_ascii_case(s))
        }),
        _ => false,
    }
}

pub fn texture_dirs(root: &Path, content_dir: &Path) -> Vec<PathBuf> {
    // (found whatever its case: `Texture` of a parked car's folder on Linux, whose file
    // system tells `texture` from `Texture`, left the car white)
    let mut dirs = vec![omsi_cfg::resolve_path(content_dir, "texture"), content_dir.to_path_buf()];
    // vehicle folders keep the model in `model\` and the textures in `texture\` next to it
    if let Some(parent) = content_dir.parent() {
        dirs.push(omsi_cfg::resolve_path(parent, "texture"));
    }
    dirs.push(omsi_cfg::resolve_path(root, "Texture"));
    dirs
}

/// The `.surf` map of a texture: a picture named after the texture as the content asks for
/// it, plus `.surf` (`str_kopfgr01.bmp.surf`, also beside a `.dds`), in the texture's folder.
/// Its red channel is the bumpiness of a road drawn with the texture ([`HeightMap`]), which
/// OMSI 2 lays under the wheels (#886). Loaded once per file.
///
/// [`HeightMap`]: omsi_geometry::HeightMap
pub fn surf_map(texture: &str, dirs: &[&Path]) -> Option<Arc<omsi_geometry::HeightMap>> {
    static MEMO: std::sync::OnceLock<Mutex<HashMap<PathBuf, Option<Arc<omsi_geometry::HeightMap>>>>> = std::sync::OnceLock::new();
    // OMSI_NO_SURF: every road as smooth as before (A/B)
    if omsi_cfg::env::var_os("OMSI_NO_SURF").is_some() {
        return None;
    }
    let found = omsi_texture::find_texture(texture, dirs)?;
    let dir = found.parent()?;
    let req = texture.trim().replace('\\', "/");
    let base = req.rsplit('/').next().unwrap_or(&req).to_string();
    let found_name = found.file_name()?.to_string_lossy().into_owned();
    let path = [base, found_name]
        .iter()
        .map(|n| omsi_cfg::resolve_path(dir, &format!("{n}.surf")))
        // through the VFS: a texture found in a mounted archive has its map in there too
        .find(|p| omsi_cfg::vfs::is_file(p))?;
    let memo = MEMO.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(m) = memo.lock().get(&path) {
        return m.clone();
    }
    let map = match omsi_texture::decode_file(&path) {
        Ok(img) => omsi_geometry::HeightMap::from_rgba(img.width as usize, img.height as usize, &img.rgba).map(Arc::new),
        Err(e) => {
            log::warn!("{e}");
            None
        }
    };
    memo.lock().insert(path, map.clone());
    map
}

#[cfg(test)]
mod surf_map_tests {
    use super::*;
    use std::io::Write;

    fn bmp(red: u8) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(2, 2, image::Rgb([red, 0, 0])).write_to(&mut out, image::ImageFormat::Bmp).unwrap();
        out.into_inner()
    }

    #[test]
    fn surf_map_beside_a_texture_in_a_mounted_archive() {
        let dir = std::env::temp_dir().join(format!("openomsi-surf-zip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("roads.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        for (name, data) in [("Splines/Roads/texture/cobbles.bmp", bmp(128)), ("Splines/Roads/texture/cobbles.bmp.surf", bmp(255))] {
            z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(&data).unwrap();
        }
        z.finish().unwrap();
        let mount = omsi_cfg::vfs::mount_zip(&zip_path).unwrap();
        let tex = mount.join("Splines").join("Roads").join("texture");
        // only the archive holds it: nothing on the disk beside the zip
        assert!(!tex.join("cobbles.bmp.surf").is_file());
        let map = surf_map("cobbles.bmp", &[&tex]).expect("the .surf in the archive");
        assert!((map.lift(glam::Vec2::new(0.5, 0.5)) - 0.02).abs() < 1e-3);
        std::fs::remove_dir_all(&dir).ok();
    }
}

pub struct World {
    pub root: PathBuf,
    pub global: GlobalCfg,
    pub map_dir: PathBuf,
    /// Indexed parked car lists of the map, loaded when a parking space uses one.
    parklist: Mutex<HashMap<usize, Vec<String>>>,
    /// Render textures of the player's mirrors (`reflexionN.bmp`), by camera index.
    pub mirror_textures: Mutex<Vec<Option<TextureId>>>,
    /// Width / height of the glass of the player bus mirror N (from the mesh that shows its
    /// picture), 0 when not known: the shape of the panels that copy the mirrors to the screen.
    pub mirror_aspect: Mutex<Vec<f32>>,
    /// Where the glass of the player bus mirror N is and which way the picture's u and v run
    /// over it (middle, position per u, position per v; the bus's frame), from the mesh that
    /// shows it: how the panels that copy the mirrors turn the picture.
    pub mirror_glass: Mutex<Vec<Option<MirrorGlass>>>,
    object_types: Mutex<HashMap<String, Option<Arc<ObjectType>>>>,
    spline_types: Mutex<HashMap<String, Option<Arc<SplineType>>>>,
    pub textures: Arc<TextureCache>,
    /// World position (with terrain height) and rotation of every loaded map object by id.
    pub object_positions: Mutex<HashMap<i64, (DVec3, [f64; 3])>>,
    /// The objects whose id is used on more than one tile, by (tile, id) (see
    /// `MapIndex::duplicates`, `World::entry_point_place`).
    pub object_dups: Mutex<HashMap<((i32, i32), i64), (DVec3, [f64; 3])>>,
    /// Loaded terrains by tile coordinate.
    pub terrains: Arc<RwLock<HashMap<(i32, i32), Arc<Terrain>>>>,
    /// Surface rasters (roads) by tile coordinate.
    pub surfaces: Arc<RwLock<HashMap<(i32, i32), Arc<TileSurface>>>>,
    /// GPU resources per vehicle type (by .bus path) and paint scheme, shared by AI vehicles.
    vehicle_gpu: Mutex<HashMap<VehicleKey, VehicleSet>>,
    /// GPU textures of vehicles by file: one bus spawned in twenty adverts used to upload
    /// its whole texture set twenty times (200 ms a spawn, 1.5 fps on Spandau).
    /// (With the number of sets holding each.)
    vehicle_textures: Arc<Mutex<HashMap<PathBuf, (TextureId, usize)>>>,
    /// GPU meshes of vehicles by (bus file, mesh index), shared across paint schemes.
    vehicle_meshes: Arc<Mutex<HashMap<(PathBuf, usize), (MeshId, usize)>>>,
    /// Vehicle meshes and textures made on a worker, until their set is uploaded.
    vehicle_ready: Arc<Mutex<PreparedVehicles>>,
    /// Textures uploaded as RGBA to spare a frame, being compressed on the workers, and the
    /// compressed ones waiting to be swapped in.
    upgrades_pending: Mutex<hashbrown::HashSet<PathBuf>>,
    upgrades_done: Arc<Mutex<Vec<(PathBuf, Arc<TextureData>)>>>,
    /// Roller-blind pictures (`[matl_freetex]`) uploaded as RGBA, to be compressed.
    freetex_upgrades: Arc<Mutex<Vec<PathBuf>>>,
    /// OMSI's `[texmemlimit]`: bytes the scenery and vehicle textures may take on the GPU
    /// (0 = no limit), and when the budget was last looked at.
    texture_limit: std::sync::atomic::AtomicU64,
    budget_checked: Mutex<Option<std::time::Instant>>,
    /// Traffic-path lanes collected while building tiles.
    pub lanes: Mutex<Vec<omsi_sim::traffic::Lane>>,
    /// The tiles whose lanes and parked cars have been put into `lanes` and `parked_cars`.
    /// These two and `lanes` are only filled, and should only be taken, while the `lanes`
    /// lock is held: whoever takes them then has every parked car together with the lanes
    /// it stands beside, and knows which tiles those came from.
    pub lane_tiles: Mutex<Vec<(i32, i32)>>,
    /// Traffic light programs of placed crossings, and which map object owns each.
    pub traffic_lights: Mutex<Vec<TrafficLightController>>,
    pub controller_of_object: Mutex<HashMap<i64, usize>>,
    /// Placed traffic light lamps.
    pub light_objects: Mutex<Vec<LightObject>>,
    /// Placed objects with scripts / animations.
    pub scripted: Mutex<Vec<ScriptedObject>>,
    /// The clock and the departure boards the scenery scripts read.
    pub timetable_boards: Mutex<StopBoards>,
    /// The map's `Holidays.txt`, read when first asked.
    pub calendar: std::sync::OnceLock<omsi_map::Calendar>,
    /// The number plates of `registrations.txt` (the active chrono scenarios' first, the
    /// latest before, then the map's own: the original), read when first asked.
    pub registrations: std::sync::OnceLock<Vec<String>>,
    /// The clock the run starts at: a scenery object placed before the simulation's clock
    /// reaches the boards runs its `{init}` on it (it ran on 09:00 of 1989).
    pub start_clock: Mutex<omsi_sim::SimClock>,
    /// Obstacles for vehicle collisions (of the loaded tiles; replaced when they change).
    pub collision: Mutex<Arc<omsi_sim::collision::CollisionWorld>>,
    /// `[crashmode_pole]` objects of the loaded tiles by collision key: where they stand and
    /// their instances, so that a post a vehicle knocked over can be laid on the ground
    /// (see [`World::lay_down_pole`]). A tile's posts leave with it: its instances are
    /// handed to other objects.
    pub poles: Mutex<HashMap<i64, (DVec3, Mat4, Vec<usize>)>>,
    /// Posts knocked over in this run and the way they fell: a tile loaded again lays them
    /// down again.
    fallen_poles: Mutex<HashMap<i64, DVec3>>,
    /// The parked cars of the loaded tiles by collision key, so that one can pull out into
    /// the traffic (see [`World::depart_parked`]). They leave with their tile.
    pub parked_objects: Mutex<HashMap<i64, ParkedObject>>,
    /// The instances of the route arrows the map's author put up (`[helparrow]` objects) by
    /// tile: drawn only while OMSI 2's route arrows are on (see [`World::show_help_arrows`]).
    help_arrows: Mutex<HashMap<(i32, i32), Vec<usize>>>,
    /// Whether they are drawn now.
    help_arrows_shown: std::sync::atomic::AtomicBool,
    /// Parked cars that drove off in this run: their space stays empty when the tile comes
    /// back.
    departed: Mutex<std::collections::HashSet<i64>>,
    /// What a parked car that drove off left behind: its object and its boxes, for an AI
    /// car of the same kind that parks in the space again (`return_parked`).
    departed_objects: Mutex<std::collections::HashMap<i64, (ParkedObject, Vec<omsi_sim::collision::Obb>, Vec<omsi_sim::collision::Obb>)>>,
    /// The loaded tiles' own objects by map id, for the object editor (`crate::editor`).
    pub edit_objects: Mutex<HashMap<i64, EditObject>>,
    /// What the object editor did this run, by map id: kept over tile reloads until saved.
    pub object_edits: Mutex<HashMap<i64, ObjectEdit>>,
    /// The ground as the editor's brush has left it, by tile (read instead of the file).
    pub terrain_edits: Mutex<HashMap<(i32, i32), Terrain>>,
    /// Placed `[busstop]` objects: (map id, world position, heading, name).
    pub bus_stops: Mutex<Vec<(i64, DVec3, f64, String)>>,
    /// Where people wait at the stops: the `[passpos]` points of placed objects with a
    /// `[passengercabin]` (the maps' `people_standing_*` markers and bus shelters) as
    /// (object id, world position, heading in degrees, seat height - 0 for a standing place).
    pub waiting_places: Mutex<Vec<(i64, DVec3, f64, f32)>>,
    /// Passenger cabins of waiting objects by file, read once.
    waiting_cabins: Mutex<HashMap<PathBuf, Option<Arc<omsi_vehicle::PassengerCabin>>>>,
    /// Counts the changes of the loaded tiles (see [`World::refresh_tile_lists`]): whoever
    /// keeps what it derived from the stops, the waiting places or the ground looks again.
    pub tiles_generation: std::sync::atomic::AtomicU64,
    /// Parked cars placed on `[carpark_p]` spaces: world position and heading (deg), for
    /// the traffic to steer round (see `lane_tiles`).
    pub parked_cars: Mutex<Vec<(DVec3, f64)>>,
    /// The boxes of the parked cars of the loaded tiles, which people walk round.
    pub parked_boxes: Mutex<Arc<Vec<omsi_sim::collision::Obb>>>,
    /// Placed objects with particle systems (chimney smoke, the fireworks, a memorial's
    /// flame) by tile.
    pub particle_objects: Mutex<HashMap<(i32, i32), Vec<ParticleObject>>>,
    /// The boxes of the loaded `[petrolstation]` objects (the depots' fuel and wash yards):
    /// OMSI lets the pump and the wash run only while the bus's box
    /// overlaps one, and sends the workshop's team out when the bus stands in none.
    pub petrol_stations: Mutex<Vec<omsi_sim::collision::Obb>>,
    /// Parked cars standing in the loaded tiles, and the options' `[AIMaxCountParked]`
    /// (0 = every space the map fills, -1 = none): past it the spaces stay empty.
    pub parked_live: std::sync::atomic::AtomicUsize,
    pub parked_max: i64,
    /// Places that echo (`[triggerbox_new]` + `[triggerbox_setreverb]`: the railway bridges'
    /// underpasses): the box, the reverberation time (s) and the distance (m) over which it
    /// fades in at the box's sides.
    pub reverb_zones: Mutex<Vec<(omsi_sim::collision::Obb, f32, f32)>>,
    /// The loaded tiles' night light maps (`.map.LM.bmp`), for the light map atlas.
    pub light_maps: Mutex<HashMap<(i32, i32), Arc<omsi_texture::Image>>>,
    light_maps_generation: std::sync::atomic::AtomicU64,
    /// The atlas as last filled: centre tile and the generation of `light_maps`.
    light_map_atlas: Mutex<Option<((i32, i32), u64)>>,
    /// The map's `signalroutes.cfg` (unit `mc_fahrstrasse`): which track pieces each railway
    /// signal protects, its distant signal, the next signal and a speed limit.
    pub signal_routes: Vec<omsi_map::ailists::SignalRoute>,
    /// Chrono folders active on the sim date, in order, and the merged AI lists / date.
    /// The chrono scenarios in force on the sim date (changed at midnight: `set_date`).
    pub chrono_dirs: parking_lot::RwLock<Vec<PathBuf>>,
    pub ailists: omsi_map::AiLists,
    pub date: i32,
    /// Ticket pack (chrono folders may override the map's).
    pub ticket_pack: String,
    /// Fonts for text and script textures, shared by all vehicles.
    pub fonts: Arc<Mutex<omsi_sim::texttex::FontLibrary>>,
    /// Scenery material variants switched by `NightlightA`: (instance, slot, material on, off).
    pub night_slots: Mutex<Vec<(usize, usize, MaterialId, MaterialId)>>,
    /// Objects whose night textures follow a `[NightMapMode]` timetable (see `update_night_modes`).
    pub night_modes: Mutex<Vec<NightMode>>,
    /// Light coronas and point lights of placed scenery objects.
    pub static_coronas: Mutex<Vec<StaticCorona>>,
    pub static_lights: Mutex<Vec<omsi_render::PointLight>>,
    /// Every tile file of the map read once: splines for the spline attachment rows,
    /// objects for entry points and stops of tiles that are not loaded.
    index: Mutex<Option<Arc<MapIndex>>>,
    /// What each loaded tile added (see [`TileState`]).
    pub tile_state: Mutex<HashMap<(i32, i32), TileState>>,
    /// Tiles that have been loaded at least once: their lanes, light programs and parked
    /// cars are in the lists for good.
    seeded: Mutex<hashbrown::HashSet<(i32, i32)>>,
    /// The GPU side of the loaded tiles.
    gpu: Mutex<GpuCache>,
    /// Types this installation lacks, logged once each (file, what it is).
    missing: Mutex<hashbrown::HashMap<String, &'static str>>,
    /// Tiles read and typed that loaded tiles (or tiles on their way) depend on.
    staged: Mutex<HashMap<(i32, i32), Arc<StagedTile>>>,
    layout: Mutex<Option<Arc<TileLayout>>>,
    /// Scenery objects' sound configurations, read once per file.
    sound_cfgs: Mutex<HashMap<PathBuf, Option<Arc<omsi_vehicle::SoundCfg>>>>,
}

/// A placed object's particle systems (`[smoke]`, `[particle_emitter]`).
pub struct ParticleObject {
    pub map_id: i64,
    pub pos: DVec3,
    pub rot: Mat4,
    pub set: omsi_sim::particles::ParticleSet,
}

/// A `[light_enh]`/`[light_enh_2]` of a placed scenery object.
#[derive(Debug, Clone)]
pub struct StaticCorona {
    pub corona: omsi_render::Corona,
    /// What switches it: a constant, the night flag, or an object variable (treated as on).
    pub switch: LightSwitch,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LightSwitch {
    Constant(f32),
    Night,
    Variable(String),
}

impl LightSwitch {
    pub fn parse(var: &str) -> LightSwitch {
        let v = var.trim();
        if let Ok(x) = v.parse::<f32>() {
            LightSwitch::Constant(x)
        } else if v.eq_ignore_ascii_case("NightlightA") || v.is_empty() {
            LightSwitch::Night
        } else {
            LightSwitch::Variable(v.to_string())
        }
    }
}

/// Resolve the three standard traffic-lamp channels without going through the scenery VM.
///
/// Stock `.sco` files use `red`, `yellow` and `green` in both `[visible]` and
/// `[light_enh_2]`.  Keeping this mapping at the renderer boundary is important: a missing
/// or platform-specific script load must not turn an unknown variable into an always-visible
/// mesh (which makes all three bulbs appear lit).  Non-standard channels, such as `Left`, are
/// still resolved by the object's script.
pub fn standard_traffic_lamp(
    var: &str,
    red: bool,
    yellow: bool,
    green: bool,
    approach: bool,
) -> Option<f32> {
    match var.trim().to_ascii_lowercase().as_str() {
        "red" | "rot" => Some(red as i32 as f32),
        "yellow" | "gelb" | "amber" => Some(yellow as i32 as f32),
        "green" | "gruen" | "grün" => Some(green as i32 as f32),
        "trafficlightapproach" => Some(approach as i32 as f32),
        _ => None,
    }
}

/// Resolve a lamp channel after its script has run. A zero script output is meaningful
/// (not a reason to use the stock phase), notably while a pedestrian lamp is blinking.
pub(crate) fn traffic_lamp_value(var: &str, scripted: Option<f32>, standard: Option<f32>) -> f32 {
    scripted
        .or_else(|| var.trim().parse::<f32>().ok())
        .or(standard)
        .unwrap_or(0.0)
}

/// Coronas and point lights of a model in the frame of `xf` (rotation) at `pos`.
/// `value_of` resolves the light variables (vehicle state or scenery switches), with the
/// lights' brightness as their `timeconst` has let it follow the switch (`fades`: one value
/// per `[light_enh_2]` of the model in order; missing = at once).
///
/// The lights of every detail level count, not only LOD 0's: a `[light_enh]` belongs to the
/// mesh before it, and 40 stock models (the Spandau neon, sodium and gas street lamps, the
/// Sv signals, the ICE and RE160 coaches) declare theirs after the far `[LOD] 0` mesh - their
/// glow still shows up close in OMSI. No stock model repeats a light in two levels.
pub fn model_lights_faded(
    model: &Model,
    mesh_transforms: &dyn Fn(usize) -> Mat4,
    pos: DVec3,
    value_of: &dyn Fn(&str) -> f32,
    fades: &[f32],
) -> Vec<omsi_render::Corona> {
    model_lights_owned(model, mesh_transforms, pos, value_of, fades).into_iter().map(|c| c.0).collect()
}

/// Every light of a model in the order [`model_lights_owned`] numbers them: the mesh it
/// belongs to, its place and its direction (zero for a `[light_enh]` and an omni light).
/// Omsi.exe files each `[light_enh]`/`[light_enh_2]` with the `[mesh]` before it (the
/// model loader, 0x5f3140: the light goes into the current mesh's list, mesh +0x1b0) and
/// draws it where that mesh's animation takes it - the lamps along a level crossing's arm
/// rise with the arm.
pub fn model_light_sources(model: &Model) -> Vec<(usize, glam::Vec3, glam::Vec3)> {
    let mut out = Vec::new();
    for (i, md) in model.meshes.iter().enumerate() {
        for l in &md.light_enh {
            out.push((i, glam::Vec3::from(l.pos), glam::Vec3::ZERO));
        }
        for l in &md.light_enh_2 {
            out.push((i, glam::Vec3::from(l.pos), if l.omni { glam::Vec3::ZERO } else { glam::Vec3::from(l.dir) }));
        }
    }
    out
}

/// The sprites of one lamp, a `[light_enh]` or a `[light_enh_2]` alike (Omsi.exe 0x5a0068):
/// its `glow`, left out with effect bit 4; with effect bit 1 a star (light_effect1.bmp)
/// turned to the viewer, 2.5 times the size and growing with the glow's strength
/// (corona.wgsl, flag bit 8); without bit 2 a halo round it in fog, seen from in front
/// (sizes and strengths in `lights::collect` and corona.wgsl, like the cone's) - `size` is
/// the light's size, `halo_cone` its outer and inner half cone angles (radians).
fn push_lamp_sprites(out: &mut Vec<omsi_render::Corona>, glow: omsi_render::Corona, effect: u8, size: f32, halo_cone: (f32, f32)) {
    if effect & 4 == 0 {
        out.push(glow);
    }
    if effect & 1 != 0 {
        out.push(omsi_render::Corona {
            size: size * 1.25,
            rotating: 2,
            flags: 8,
            texture: crate::lights::star_texture_id(),
            ..glow
        });
    }
    if effect & 2 == 0 {
        out.push(omsi_render::Corona {
            position: glow.position,
            size,
            color: glow.color,
            brightness: glow.brightness,
            direction: glow.direction,
            cone_cos: halo_cone.0,
            inner_cos: halo_cone.1,
            texture: crate::lights::glow_texture_id(),
            halo: true,
            ..Default::default()
        });
    }
}

/// [`model_lights_faded`], each sprite with the light it belongs to (the n-th light of the
/// model, `[light_enh]` and `[light_enh_2]` in file order - the order `value_of` is asked
/// in): one light gives several sprites (its glow, star, fog halo and cone).
pub fn model_lights_owned(
    model: &Model,
    mesh_transforms: &dyn Fn(usize) -> Mat4,
    pos: DVec3,
    value_of: &dyn Fn(&str) -> f32,
    fades: &[f32],
) -> Vec<(omsi_render::Corona, usize)> {
    let mut out = Vec::new();
    let mut owners: Vec<usize> = Vec::new();
    let mut seq = 0usize;
    let mut li = 0usize;
    let model_dir = model.path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    for (i, md) in model.meshes.iter().enumerate() {
        // most meshes carry no light, and their transform is not free (every mesh of every
        // AI car, every frame)
        if md.light_enh.is_empty() && md.light_enh_2.is_empty() {
            continue;
        }
        let first_li = li;
        li += md.light_enh_2.len();
        let xf = mesh_transforms(i);
        for l in &md.light_enh {
            owners.resize(out.len(), seq.wrapping_sub(1));
            seq += 1;
            // Omsi.exe reads a `[light_enh]` into the same lamp as a `[light_enh_2]` (0x5f2bb6,
            // a TLampensetting) and draws it alike (0x5a0068): omnidirectional, turned to the
            // viewer, its four numbers the brightness factor, the z offset, the effect bits
            // and the fade time, then its own bitmap. Drawn as a bare licht.bmp glow on the
            // lamp, the stop request lamp of the MAN NL and SD202 (`D92_Haltewunsch.bmp`,
            // 5 cm to the front) showed as a ring round its dome (#1159). (Its fade time is
            // not followed: such a lamp is on or off at once.)
            let factor = l.values.first().copied().filter(|f| *f > 0.0).unwrap_or(1.0);
            let b = (value_of(&l.variable) * factor).clamp(0.0, 2.0);
            if !(b > 0.0) || !b.is_finite() {
                continue;
            }
            let p = xf.transform_point3(glam::Vec3::from(l.pos)).as_dvec3() + pos;
            let effect = l.values.get(2).map(|v| *v as i32).unwrap_or(1).clamp(0, 7) as u8;
            // (the glow is as wide as the light's size: OMSI draws its sprite half that
            // either side of the lamp)
            let glow = omsi_render::Corona {
                position: p,
                size: (l.size * 0.5).max(0.0),
                color: [l.color[0] / 255.0, l.color[1] / 255.0, l.color[2] / 255.0],
                brightness: b,
                direction: glam::Vec3::ZERO,
                cone_cos: -1.0,
                rotating: 2,
                z_offset: l.values.get(1).copied().unwrap_or(0.1).max(0.0),
                flags: effect & !1,
                texture: l.texture.as_deref().map(|b| crate::lights::corona_texture_id(&model_dir, b)).filter(|t| *t != 0).unwrap_or_else(crate::lights::glow_texture_id),
                ..Default::default()
            };
            push_lamp_sprites(&mut out, glow, effect, l.size, (0.0, 0.0));
        }
        for (k, l) in md.light_enh_2.iter().enumerate() {
            owners.resize(out.len(), seq.wrapping_sub(1));
            seq += 1;
            // the fading variable: 0 dark, 1 normal, 2 double (times the factor), as far as
            // the lamp has come on or gone out (`timeconst`)
            let b = match fades.get(first_li + k) {
                Some(f) => *f,
                None => (value_of(&l.variable) * if l.factor > 0.0 { l.factor } else { 1.0 }).clamp(0.0, 2.0),
            };
            if !(b > 0.0) || !b.is_finite() {
                continue;
            }
            let p = xf.transform_point3(glam::Vec3::from(l.pos)).as_dvec3() + pos;
            let dir = if l.omni {
                glam::Vec3::ZERO
            } else {
                xf.transform_vector3(glam::Vec3::from(l.dir))
                    .normalize_or_zero()
            };
            let up = xf.transform_vector3(glam::Vec3::from(l.up)).normalize_or(glam::Vec3::Z);
            let half_cos = |deg: f32| (deg.max(1.0) * 0.5).min(180.0).to_radians().cos();
            let (outer, inner) = (l.cone_outer.max(l.cone_inner), l.cone_inner.min(l.cone_outer));
            let flags = l.values.first().map(|v| omsi_cfg::parse_f32(v) as i32).unwrap_or(0).clamp(0, 7) as u8;
            let color = [l.color[0] / 255.0, l.color[1] / 255.0, l.color[2] / 255.0];
            // the original: the glow, the light's own bitmap (else licht.bmp) as wide as its
            // size, with its star and its halo in fog (`push_lamp_sprites`)
            let glow = omsi_render::Corona {
                position: p,
                size: (l.size * 0.5).max(0.0),
                color,
                brightness: b,
                direction: dir,
                cone_cos: half_cos(outer),
                inner_cos: if inner > 0.0 { half_cos(inner) } else { -2.0 },
                // an omnidirectional light has no face to show: it turns to the viewer
                rotating: if l.omni { 2 } else { l.rotating.clamp(0, 2) as u8 },
                up,
                z_offset: l.z_offset.max(0.0),
                flags: flags & !1,
                texture: l.bitmap.as_deref().map(|b| crate::lights::corona_texture_id(&model_dir, b)).filter(|t| *t != 0).unwrap_or_else(crate::lights::glow_texture_id),
                ..Default::default()
            };
            push_lamp_sprites(&mut out, glow, flags, l.size, ((outer * 0.5).to_radians(), (inner.max(0.0) * 0.5).to_radians()));
            // the light's cone in fog (the original: built for a directional light
            // with the cone flag whose cone angles make sense; effect bit 2 leaves it out).
            // Its size and strength follow the weather and the viewer (`lights::collect`,
            // corona.wgsl), so the raw values go along: the light's size and brightness and
            // the half cone angles in radians.
            if l.cone && !l.omni && dir.length_squared() > 0.5 && l.cone_inner >= 0.0 && l.cone_outer >= l.cone_inner && flags & 2 == 0 {
                out.push(omsi_render::Corona {
                    position: p,
                    size: l.size,
                    color: [l.color[0] / 255.0, l.color[1] / 255.0, l.color[2] / 255.0],
                    brightness: b,
                    direction: dir,
                    cone_cos: (l.cone_outer * 0.5).to_radians(),
                    inner_cos: (l.cone_inner * 0.5).to_radians(),
                    texture: crate::lights::cone_texture_id(),
                    beam: true,
                    ..Default::default()
                });
            }
        }
    }
    owners.resize(out.len(), seq.wrapping_sub(1));
    out.into_iter().zip(owners).collect()
}

/// An object whose top stays this low (m over its foot) is no wall for a vehicle body; with
/// a `[collision_mesh]` it is a step the wheels climb (a traffic island).
pub const LOW_OBJECT: f32 = 0.3;

/// Faces this close over another road face are paint on it, not a step (m). Omsi.exe's
/// ground query (0x7a0814) takes the highest face whatever lies under it; this keeps only
/// the thinnest layers flat (a marking a centimetre or two over the asphalt). At 4.5 cm it
/// also took the speed cushions, manhole and plate objects, lowered kerbs and slab edges
/// away, and the bottom 4.5 cm of every speed bump's ramp - "no road bumps", and wheels
/// drawn sunk into what they drove on.
const PAINT_LAYER: f32 = 0.02;

/// What a wheel stands on at world (x, y): the faces of the roads, crossings and surface
/// objects there, and the terrain wherever it is not cut away under them - the highest at
/// or below `top`, and the lowest above it (a kerb the tyre is up against).
pub fn drive_probe(
    terrains: &RwLock<HashMap<(i32, i32), Arc<Terrain>>>,
    surfaces: &RwLock<HashMap<(i32, i32), Arc<TileSurface>>>,
    x: f64,
    y: f64,
    top: f64,
) -> omsi_sim::rigid::GroundProbe {
    let key = tile_key(x, y);
    let surface = surfaces.read().get(&key).cloned();
    let terrain = terrains.read().get(&key).cloned();
    probe_tile(surface.as_deref(), terrain.as_deref(), key, x, y, top)
}

fn tile_key(x: f64, y: f64) -> (i32, i32) {
    (
        (x / tile_size()).floor() as i32,
        (y / tile_size()).floor() as i32,
    )
}

/// [`drive_probe`] on the tile `key` that holds (x, y).
fn probe_tile(
    surface: Option<&TileSurface>,
    terrain: Option<&Terrain>,
    key: (i32, i32),
    x: f64,
    y: f64,
    top: f64,
) -> omsi_sim::rigid::GroundProbe {
    let lx = (x - key.0 as f64 * tile_size()) as f32;
    let ly = (y - key.1 as f64 * tile_size()) as f32;
    let mut probe = omsi_geometry::Probe::default();
    let mut zs = [0f32; 64];
    let mut walls = None;
    let n = surface.map_or(0, |s| s.drive.heights(lx, ly, &mut zs, &mut walls));
    if let Some(s) = surface {
        let below = |lim: f32| {
            if n > zs.len() {
                s.drive.probe(lx, ly, lim)
            } else {
                zs[..n].iter().fold(omsi_geometry::Probe::default(), |p, &z| p.merge(omsi_geometry::Probe::of(z, lim)))
            }
        };
        probe = below(top as f32);
        // a painted layer is no step: road markings made as `[surface]` objects or as
        // splines with a height profile lie a centimetre or three over the asphalt, and the
        // wheels climbed every line - the bus hopped at stops and over dotted lines (Horizon).
        // Where another road face lies that little below, the wheel stands on that one.
        // (only road faces: the terrain under a road is often that close too)
        // (layer under layer: a marking over a marking over the asphalt, as the map editor
        // stacks them where lines cross or a box junction lies over a lane's arrows, was
        // still a step when only the first was looked through)
        let mut layers = 0;
        while let Some(z1) = probe.below.filter(|_| layers < 4) {
            layers += 1;
            match below(z1 - 0.0005).below {
                Some(z2) if z1 - z2 < PAINT_LAYER => probe.below = Some(z2),
                _ => break,
            }
        }
    }
    // On a road the wheel stands on the road, as in OMSI: the terrain under it or over it
    // (an embankment the road runs under, ground poking through the asphalt) is no
    // ground and no wall there. Taken with the road, a terrain face over the carriageway was
    // an invisible wall under bridges, and one through it a bump that threw the bus.
    let ground = terrain.map(|t| {
        let h = omsi_geometry::terrain_height(t, lx, ly);
        let cut = surface
            .map(|s| s.cut_at(lx, ly, h, surface_flush()))
            .unwrap_or(false);
        (h, cut)
    });
    // ... unless that face lies buried well under ground that is drawn here and is under the
    // wheel, not over it: the lower slope of an embankment spline (Marcel's `Damm1` falls
    // 20 m over 30 m on each side) reaching under a junction the terrain carries. Omsi.exe
    // takes the highest face there, the ground; taken as the road, it dropped the bus 8 m
    // through the asphalt into the slope (Cotterell, the junction by the park at 250, 427).
    let buried = matches!((probe.below, ground), (Some(z), Some((h, false))) if h <= top as f32 && h - z > BURIED_FACE);
    let on_road = probe.below.is_some() && !buried;
    if let (Some((h, cut)), false) = (ground, on_road) {
        // the ground counts where it is drawn; where it is cut away and nothing else is
        // there (a surface without a collision), it still carries rather than let the
        // vehicle drop out of the world
        if !cut || (probe.below.is_none() && h <= top as f32) {
            probe = probe.merge(omsi_geometry::Probe::of(h, top as f32));
        }
    }
    // A wall's top (a narrow height profile high on a wall spline) is never stood on: where
    // it stands a step over the ground here it is a wall the tyre meets, whatever the height
    // it is probed from; nearer the ground than that the wheel rolls on the road beside it
    // (where the wall's top met the road the wheels went up onto it and rode along it)
    if let Some(s) = surface {
        let walls = if n > zs.len() { s.drive.probe_walls(lx, ly, f32::MAX).below } else { walls };
        if let (Some(zw), Some(g)) = (walls, probe.below) {
            if zw > g + WALL_TOP_STEP {
                probe.above = Some(probe.above.map_or(zw, |a| a.min(zw)));
            }
        }
    }
    omsi_sim::rigid::GroundProbe {
        below: probe.below.map(|z| z as f64),
        above: probe.above.map(|z| z as f64),
    }
}

/// What is drawn at world (x, y) under `top`: the highest face of the splines and surface
/// objects (as lifted for drawing) and the terrain where it is not cut away - the picture's
/// ground, without any of the wheel rules of [`drive_probe`] (`OMSI_GROUND_GAP` measures the
/// tyres against it).
pub fn drawn_ground(
    terrains: &RwLock<HashMap<(i32, i32), Arc<Terrain>>>,
    surfaces: &RwLock<HashMap<(i32, i32), Arc<TileSurface>>>,
    x: f64,
    y: f64,
    top: f64,
) -> Option<f64> {
    let key = tile_key(x, y);
    let lx = (x - key.0 as f64 * tile_size()) as f32;
    let ly = (y - key.1 as f64 * tile_size()) as f32;
    let surface = surfaces.read().get(&key).cloned();
    let terrain = terrains.read().get(&key).cloned();
    let mut best: Option<f32> = None;
    if let Some(s) = surface.as_deref() {
        let a = s.drive.probe(lx, ly, top as f32).below;
        let b = s.drive.probe_walls(lx, ly, top as f32).below;
        best = a.into_iter().chain(b).reduce(f32::max);
    }
    if let Some(t) = terrain.as_deref() {
        let h = omsi_geometry::terrain_height(t, lx, ly);
        let cut = surface.as_deref().is_some_and(|s| s.cut_at(lx, ly, h, surface_flush()));
        if !cut && h <= top as f32 {
            best = Some(best.map_or(h, |b| b.max(h)));
        }
    }
    best.map(|z| z as f64)
}

/// How far a road face may lie under drawn ground before it counts as buried (m): far more
/// than the ground poking through the asphalt that the road is there to keep out.
const BURIED_FACE: f32 = 1.0;

/// How far over the ground a wall's top must stand to be a wall to the wheels (a kerb is
/// less, and the tyre climbs it).
const WALL_TOP_STEP: f32 = 0.3;

/// The ground the player's wheels stand on: [`drive_probe`] over the loaded tiles.
pub struct DriveGround {
    pub terrains: Arc<RwLock<HashMap<(i32, i32), Arc<Terrain>>>>,
    pub surfaces: Arc<RwLock<HashMap<(i32, i32), Arc<TileSurface>>>>,
}

type TileRefs = ((i32, i32), Option<Arc<TileSurface>>, Option<Arc<Terrain>>);

impl omsi_sim::rigid::Ground for DriveGround {
    fn probe(&self, x: f64, y: f64, top: f64) -> omsi_sim::rigid::GroundProbe {
        drive_probe(&self.terrains, &self.surfaces, x, y, top)
    }

    /// The tiles under the vehicle are looked up once per step, not twice for every one of
    /// the few hundred points its tyres ask for (each a lock of both maps the loader works
    /// on, two lookups and two reference counts).
    fn session(&self) -> Box<dyn Fn(f64, f64, f64) -> omsi_sim::rigid::GroundProbe + '_> {
        let tiles: std::cell::RefCell<Vec<TileRefs>> =
            std::cell::RefCell::new(Vec::with_capacity(4));
        Box::new(move |x, y, top| {
            let key = tile_key(x, y);
            let mut tiles = tiles.borrow_mut();
            let i = match tiles.iter().position(|t| t.0 == key) {
                Some(i) => i,
                None => {
                    tiles.push((
                        key,
                        self.surfaces.read().get(&key).cloned(),
                        self.terrains.read().get(&key).cloned(),
                    ));
                    tiles.len() - 1
                }
            };
            let (_, surface, terrain) = &tiles[i];
            probe_tile(surface.as_deref(), terrain.as_deref(), key, x, y, top)
        })
    }
}

/// Raster resolution of the per-tile surface mask (texels per tile edge).
pub const SURFACE_RASTER: usize = 512;

impl World {
    /// Ground height (road surface where present, else terrain) at world x, y.
    /// Terrain height alone at world x, y (no road surfaces).
    pub fn ground_terrain(&self, x: f64, y: f64) -> Option<f64> {
        let tx = (x / tile_size()).floor() as i32;
        let ty = (y / tile_size()).floor() as i32;
        let lx = (x - tx as f64 * tile_size()) as f32;
        let ly = (y - ty as f64 * tile_size()) as f32;
        let t = self.terrains.read();
        Some(t.get(&(tx, ty))?.sample(lx, ly) as f64)
    }

    /// The height a vehicle put down at (x, y) stands at: the face its wheels would stand on
    /// (a road, a deck, a floor, the ground; [`drive_probe`]) under `near` + 1.5 m and at most
    /// 3 m below it. The raster's [`World::ground_height`] takes the surface of its texel,
    /// and a bus put down beside a wall (an entry point on a pavement, London) stood on the
    /// wall's top and floated there.
    pub fn stand_height(&self, x: f64, y: f64, near: f64) -> Option<f64> {
        drive_probe(&self.terrains, &self.surfaces, x, y, near + 1.5).below.filter(|g| near - g < 3.0)
    }

    /// Where entry point `ep` stands (position, heading): its object, found on the tile
    /// the entry point names (global.cfg's `[entrypoints]` record holds the index of its
    /// tile in the `[map]` list, and the place within that tile). An object of that id on
    /// another tile (a map joined from two, whose ids repeat) is not it: the record's own
    /// place is taken then.
    pub fn entry_point_place(&self, ep: &omsi_map::global::EntryPoint) -> Option<(DVec3, [f64; 3])> {
        let s = tile_size();
        let tile = usize::try_from(ep.group).ok().and_then(|i| self.global.raw_tiles.get(i)).copied();
        if let Some(t) = tile {
            if let Some(p) = self.object_dups.lock().get(&(t, ep.object_id)) {
                return Some(*p);
            }
        }
        let found = self.object_positions.lock().get(&ep.object_id).copied();
        let recorded = tile.filter(|_| ep.pos.iter().chain(ep.quat.iter()).all(|v| v.is_finite())).map(|(tx, ty)| {
            let heading = (2.0 * ep.quat[1].atan2(ep.quat[3])).to_degrees().rem_euclid(360.0);
            (DVec3::new(tx as f64 * s + ep.pos[0], ty as f64 * s + ep.pos[1], ep.pos[2]), [heading, 0.0, 0.0])
        });
        match (found, recorded) {
            // (an object may stand a little outside its tile's square: far off only is another)
            (Some(f), Some(r)) if (f.0.truncate() - r.0.truncate()).length() > 50.0 => {
                log::info!("entry point {} \"{}\": object {} stands at ({:.0}, {:.0}), on another tile than the entry point's ({:.0}, {:.0}): the entry point's own place", ep.index, ep.name, ep.object_id, f.0.x, f.0.y, r.0.x, r.0.y);
                Some(r)
            }
            (Some(f), _) => Some(f),
            (None, r) => r,
        }
    }

    pub fn ground_height(&self, x: f64, y: f64) -> Option<f64> {
        let tx = (x / tile_size()).floor() as i32;
        let ty = (y / tile_size()).floor() as i32;
        let lx = (x - tx as f64 * tile_size()) as f32;
        let ly = (y - ty as f64 * tile_size()) as f32;
        if let Some(s) = self.surfaces.read().get(&(tx, ty)) {
            // the road the wheels stand on, not a bridge deck or an embankment over it
            if let Some(h) = s.sample_road(lx, ly).or_else(|| s.sample(lx, ly)) {
                return Some(h as f64);
            }
        }
        let t = self.terrains.read();
        let terrain = t.get(&(tx, ty))?;
        Some(terrain.sample(lx, ly) as f64)
    }

    /// The wetness a puddle would use at world (x, y): `wetness` where a road surface is
    /// under the point (the same `[moisture]` ground `enhanced.wgsl`'s reflective puddle
    /// patches sit on), 0 on bare terrain or where no surface is loaded there yet. Approximate
    /// on purpose - `puddles::water_at` only needs to agree with the shader's own mask
    /// closely enough that a tyre's spray starts where the reflection does, not to the texel.
    pub fn wet_road_at(&self, x: f64, y: f64, wetness: f32) -> f32 {
        let tx = (x / tile_size()).floor() as i32;
        let ty = (y / tile_size()).floor() as i32;
        let lx = (x - tx as f64 * tile_size()) as f32;
        let ly = (y - ty as f64 * tile_size()) as f32;
        let on_road = self
            .surfaces
            .read()
            .get(&(tx, ty))
            .is_some_and(|s| s.sample_road(lx, ly).is_some());
        if on_road {
            wetness.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// The ground under a point of the outside camera's arm: the highest face of the roads,
    /// crossings, surface objects and terrain at or below `top`. A face higher up - the
    /// roof over a petrol station's forecourt, a bridge deck - is not the ground there (the
    /// top surface of the raster is, and it put the camera on the canopy); a roof's mesh
    /// stops the camera instead.
    pub fn camera_ground(&self, x: f64, y: f64, top: f64) -> Option<f64> {
        drive_probe(&self.terrains, &self.surfaces, x, y, top).below
    }

    /// Local visible road plane under a vehicle: the exact faces where there are any (raster
    /// heights are coarser). Choose the nearby deck, never a roof above it.
    pub fn puddle_surface(&self, position: DVec3) -> Option<(f64, glam::Vec3)> {
        let height = self.camera_ground(position.x, position.y, position.z + 0.35)?;
        let key = tile_key(position.x, position.y);
        let x = (position.x - key.0 as f64 * tile_size()) as f32;
        let y = (position.y - key.1 as f64 * tile_size()) as f32;
        let normal = self.surfaces.read().get(&key)
            .and_then(|s| s.drive.surface_below(x, y, height as f32 + 0.002))
            .filter(|(z, _)| (*z as f64 - height).abs() < 0.005)
            .map(|(_, n)| n).unwrap_or(glam::Vec3::Z);
        Some((height, normal))
    }

    /// The ground painting of one tile: `texture/map/<tile>.map.<layer>.dds`, one 8-bit
    /// alpha mask per `[groundtex]` above the first that the editor's brush has touched on
    /// this tile. That is how OMSI puts asphalt under a car park, cobbles on a side street
    /// or a field into the meadow without placing a single object.
    ///
    /// The mask is stored like a picture (first row = north), the terrain mesh's v runs
    /// north with y, so the rows are turned over here.
    fn load_ground_paint(&self, tile_path: &Path) -> Vec<(usize, Image)> {
        let Some(name) = tile_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for layer in 1..self.global.ground_textures.len() {
            let path =
                omsi_cfg::resolve_path(&self.map_dir, &format!("texture/map/{name}.{layer}.dds"));
            if !omsi_cfg::vfs::is_file(&path) {
                continue;
            }
            match omsi_texture::decode_file(&path) {
                Ok(img) => {
                    let (w, h) = (img.width as usize, img.height as usize);
                    let mut rgba = vec![0u8; w * h * 4];
                    for j in 0..h {
                        let src = &img.rgba[(h - 1 - j) * w * 4..][..w * 4];
                        rgba[j * w * 4..][..w * 4].copy_from_slice(src);
                    }
                    out.push((
                        layer,
                        Image {
                            width: img.width,
                            height: img.height,
                            rgba,
                            has_alpha: true,
                        },
                    ));
                }
                Err(e) => log::warn!("ground paint {}: {e}", path.display()),
            }
        }
        out
    }

    /// Height for somebody on foot: the top of whatever is here - a pavement, a platform,
    /// a painted yard - and the bare ground where there is nothing. [`ground_height`] is
    /// the wheels' answer instead: it picks the drivable surface, which is the road *under*
    /// the kerb, and standing people on that buried them to the ankles in the pavement.
    pub fn walk_height(&self, x: f64, y: f64) -> Option<f64> {
        let tx = (x / tile_size()).floor() as i32;
        let ty = (y / tile_size()).floor() as i32;
        let lx = (x - tx as f64 * tile_size()) as f32;
        let ly = (y - ty as f64 * tile_size()) as f32;
        let surface = self
            .surfaces
            .read()
            .get(&(tx, ty))
            .and_then(|s| s.sample(lx, ly))
            .map(|h| h as f64);
        let terrain = self
            .terrains
            .read()
            .get(&(tx, ty))
            .map(|t| t.sample(lx, ly) as f64);
        let rough = match (surface, terrain) {
            (Some(s), Some(t)) => Some(s.max(t)),
            (s, t) => s.or(t),
        }?;
        // The raster says roughly where the floor is (a texel is 0.7 m on a Berlin tile, and
        // it holds the highest surface in it): the faces themselves say exactly. Read from the
        // raster, people stood 15 cm up in the air beside a kerb or sank into it, and climbed
        // every slope in steps. The highest face a little over the raster's height is taken:
        // the kerb's top on the pavement, the carriageway beside it.
        let probe = drive_probe(&self.terrains, &self.surfaces, x, y, rough + 0.3);
        match probe.below {
            Some(b) if rough - (b as f64) < 1.0 => Some(b as f64),
            _ => Some(rough),
        }
    }

    /// The floor under somebody at height `near` at (x, y): the highest face no more than a
    /// step (0.5 m, Omsi.exe 0x630498) over them - a station's floor under its roof, a car park's level under the
    /// deck above - else [`World::walk_height`]'s highest one. (Asked for the highest, the
    /// people of an indoor station stood on its roof.)
    ///
    /// Nothing under them within 3 m: the highest face, but only up to 0.5 m over them - a
    /// pavement whose tile came after them. Omsi.exe keeps its people at the heights of
    /// their paths and waiting places; the highest face, a bus shelter's roof 2.5 m up, put
    /// the people waiting under it on top of it.
    pub fn walk_height_near(&self, x: f64, y: f64, near: f64) -> Option<f64> {
        self.walk_height_reach(x, y, near, 0.5)
    }

    /// [`World::walk_height_near`] that also sees faces up to `reach` over `near`: the walker
    /// on foot looks a metre up to stop at a face too high to step onto (a platform's edge)
    /// instead of walking under it.
    pub fn walk_height_reach(&self, x: f64, y: f64, near: f64, reach: f64) -> Option<f64> {
        let probe = drive_probe(&self.terrains, &self.surfaces, x, y, near + reach);
        match probe.below {
            Some(b) if near - b < 3.0 => Some(b),
            _ => self.walk_height(x, y).filter(|z| *z < near + reach.max(0.5)),
        }
    }

    /// The clock a scenery script starts on: the simulation's, else the run's start.
    pub fn script_clock(&self) -> omsi_sim::SimClock {
        self.timetable_boards.lock().clock.clone().unwrap_or_else(|| self.start_clock.lock().clone())
    }

    pub fn open(root: &Path, global_cfg: &Path, date: i32) -> Result<World> {
        let global = GlobalCfg::load(global_cfg)
            .with_context(|| format!("loading {}", global_cfg.display()))?;
        let map_dir = global.dir().to_path_buf();
        omsi_map::configure_grid(&global);
        crate::humans::LEFT_HAND.store(global.left_hand_traffic, std::sync::atomic::Ordering::Relaxed);
        log::info!(
            "tile size {:.1} m ({})",
            omsi_map::tile_size(),
            if global.world_coordinates {
                "[worldcoordinates]"
            } else {
                "plain map"
            }
        );
        // where the sun is: the map's time zone, place and summer time
        let tz_path = omsi_cfg::resolve_path(&map_dir, "timezone.txt");
        let mut place = omsi_sim::daylight::SunPlace::default();
        if let Ok(tz) = omsi_map::TimeZone::load(&tz_path) {
            place.timezone = tz.offset_hours as f64;
            if let Some((lat, lon)) = tz.lat_lon() {
                place.latitude = lat;
                place.longitude = lon;
            }
            place.dst = tz.dst.iter().map(|d| (d.start, d.end, d.params[0], d.params[1], d.params[2])).collect();
            log::info!("sun: {:.3} N {:.3} E, UTC{:+}, {} summer time periods", place.latitude, place.longitude, place.timezone, place.dst.len());
        }
        omsi_sim::daylight::set_place(place);
        let signal_routes = omsi_cfg::CfgFile::read(&omsi_cfg::resolve_path(&map_dir, "signalroutes.cfg"))
            .map(|f| omsi_map::ailists::parse_signalroutes(&f))
            .unwrap_or_default();
        if !signal_routes.is_empty() {
            log::info!("signal routes: {} for {} signals", signal_routes.len(), signal_routes.iter().map(|r| r.signal.0).collect::<hashbrown::HashSet<_>>().len());
        }
        let chrono_dirs = omsi_map::active_chrono_dirs(&map_dir, date);
        // AI lists: the map's plus the chrono updates; depot entries filtered by validity date
        let mut ailists = omsi_map::ailists::ailists_with_chrono(&map_dir, &chrono_dirs);
        let mut ticket_pack = global.ticket_pack.clone();
        for c in &chrono_dirs {
            if let Some(cfg) = Some(omsi_cfg::resolve_path(c, "Chrono.cfg"))
                .filter(|p| omsi_cfg::vfs::is_file(p))
                .and_then(|p| omsi_cfg::CfgFile::read(&p).ok())
            {
                let cc = omsi_map::ailists::parse_chrono_cfg(&cfg);
                if let Some(t) = cc.ticket_pack {
                    ticket_pack = t;
                }
            }
        }
        for g in ailists.groups.iter_mut() {
            for tg in g.typgroups.iter_mut() {
                tg.entries
                    .retain(|e| omsi_map::typgroup_entry_valid(e, date));
            }
        }
        if !chrono_dirs.is_empty() {
            log::info!(
                "chrono: {} folders active on {date}: {:?}",
                chrono_dirs.len(),
                chrono_dirs
                    .iter()
                    .map(|d| d.file_name().unwrap().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            );
        }
        Ok(World {
            root: root.to_path_buf(),
            global,
            map_dir,
            parklist: Mutex::new(HashMap::new()),
            mirror_textures: Mutex::new(Vec::new()),
            mirror_aspect: Mutex::new(Vec::new()),
            mirror_glass: Mutex::new(Vec::new()),
            chrono_dirs: parking_lot::RwLock::new(chrono_dirs),
            ailists,
            date,
            ticket_pack,
            object_types: Mutex::new(HashMap::new()),
            spline_types: Mutex::new(HashMap::new()),
            textures: Arc::new(TextureCache::new()),
            object_positions: Mutex::new(HashMap::new()),
            object_dups: Mutex::new(HashMap::new()),
            terrains: Arc::new(RwLock::new(HashMap::new())),
            surfaces: Arc::new(RwLock::new(HashMap::new())),
            vehicle_gpu: Mutex::new(HashMap::new()),
            vehicle_textures: Default::default(),
            vehicle_meshes: Default::default(),
            vehicle_ready: Default::default(),
            upgrades_pending: Default::default(),
            upgrades_done: Default::default(),
            freetex_upgrades: Default::default(),
            texture_limit: Default::default(),
            budget_checked: Default::default(),
            lanes: Mutex::new(Vec::new()),
            lane_tiles: Mutex::new(Vec::new()),
            traffic_lights: Mutex::new(Vec::new()),
            controller_of_object: Mutex::new(HashMap::new()),
            light_objects: Mutex::new(Vec::new()),
            scripted: Mutex::new(Vec::new()),
            collision: Mutex::new(Default::default()),
            poles: Mutex::new(HashMap::new()),
            fallen_poles: Mutex::new(HashMap::new()),
            parked_objects: Mutex::new(HashMap::new()),
            help_arrows: Mutex::new(HashMap::new()),
            help_arrows_shown: std::sync::atomic::AtomicBool::new(false),
            departed: Mutex::new(std::collections::HashSet::new()),
            departed_objects: Mutex::new(std::collections::HashMap::new()),
            edit_objects: Mutex::new(HashMap::new()),
            object_edits: Mutex::new(HashMap::new()),
            terrain_edits: Mutex::new(HashMap::new()),
            bus_stops: Mutex::new(Vec::new()),
            waiting_places: Mutex::new(Vec::new()),
            waiting_cabins: Mutex::new(HashMap::new()),
            tiles_generation: std::sync::atomic::AtomicU64::new(0),
            parked_cars: Mutex::new(Vec::new()),
            parked_boxes: Mutex::new(Arc::new(Vec::new())),
            petrol_stations: Mutex::new(Vec::new()),
            reverb_zones: Mutex::new(Vec::new()),
            light_maps: Mutex::new(HashMap::new()),
            light_maps_generation: std::sync::atomic::AtomicU64::new(0),
            light_map_atlas: Mutex::new(None),
            parked_live: std::sync::atomic::AtomicUsize::new(0),
            parked_max: crate::settings::Settings::load().ai_max_parked as i64,
            signal_routes,
            particle_objects: Mutex::new(HashMap::new()),
            fonts: Arc::new(Mutex::new(omsi_sim::texttex::FontLibrary::new(root))),
            night_slots: Mutex::new(Vec::new()),
            night_modes: Mutex::new(Vec::new()),
            static_coronas: Mutex::new(Vec::new()),
            static_lights: Mutex::new(Vec::new()),
            index: Mutex::new(None),
            tile_state: Mutex::new(HashMap::new()),
            seeded: Mutex::new(Default::default()),
            gpu: Mutex::new(GpuCache::default()),
            missing: Mutex::new(Default::default()),
            staged: Mutex::new(HashMap::new()),
            layout: Mutex::new(None),
            sound_cfgs: Mutex::new(HashMap::new()),
            timetable_boards: Mutex::new(StopBoards::default()),
            calendar: std::sync::OnceLock::new(),
            registrations: std::sync::OnceLock::new(),
            start_clock: Mutex::new(omsi_sim::SimClock::default()),
        })
    }

    pub fn object_type(&self, rel: &str) -> Option<Arc<ObjectType>> {
        self.object_type_scheme(rel, None)
    }

    /// An object type with one of its `[CTC]` paint schemes applied (parked cars).
    pub fn object_type_scheme(&self, rel: &str, scheme: Option<usize>) -> Option<Arc<ObjectType>> {
        let mut key = rel.to_ascii_lowercase().replace('\\', "/");
        if let Some(i) = scheme {
            key = format!("{key}#{i}");
        }
        if let Some(t) = self.object_types.lock().get(&key) {
            return t.clone();
        }
        let path = omsi_cfg::resolve_path(&self.root, rel);
        let loaded = (|| -> Option<Arc<ObjectType>> {
            let mut sco = SceneryObject::load(&path)
                .map_err(|e| log::warn!("{e}"))
                .ok()?;
            let sco_dir = path.parent()?.to_path_buf();
            let (model, model_dir) = match &sco.model_file {
                Some(m) => {
                    let mp = omsi_cfg::resolve_path(&sco_dir, m);
                    let model = Model::load(&mp).map_err(|e| log::warn!("{e}")).ok()?;
                    (model, mp.parent()?.to_path_buf())
                }
                None => (sco.model.clone(), sco_dir.clone()),
            };
            // OMSI reads these world-pass tags from a referenced model.cfg as well as from
            // the .sco wrapper. Preserve explicit wrapper values, including an explicit
            // Normal/false override; otherwise inherit the model definition as the C++ path
            // does. Missing render phases leave junction geometry in Normal, after splines.
            sco.inherit_model_tags(&model);
            let mut meshes = Vec::new();
            let mut mesh_visible = Vec::new();
            let mut mesh_def_index = Vec::new();
            let mut mesh_pivots = Vec::new();
            if !model.lods.is_empty() {
                let start = model.lods[0].first_mesh;
                for (i, md) in model.lod_meshes(0).iter().enumerate() {
                    let mesh_path = omsi_cfg::resolve_path(&omsi_cfg::resolve_path(&model_dir, "model"), &md.file);
                    let mesh_path = if omsi_cfg::vfs::is_file(&mesh_path) {
                        mesh_path
                    } else {
                        omsi_cfg::resolve_path(&model_dir, &md.file)
                    };
                    match omsi_o3d::load_mesh(&mesh_path) {
                        Ok(m) => {
                            meshes.push((
                                mesh_from_o3d(&m),
                                m.materials.clone(),
                                md.materials.clone(),
                            ));
                            mesh_visible.push(md.visible.clone());
                            mesh_def_index.push(start + i);
                            mesh_pivots.push(omsi_sim::anim::pivot_from_mesh(&m));
                        }
                        Err(e) => log::debug!("{}: {e}", mesh_path.display()),
                    }
                }
            }
            // lower detail levels
            let mut lower_lods = Vec::new();
            for l in 1..model.lods.len() {
                let mut list = Vec::new();
                for md in model.lod_meshes(l) {
                    let mesh_path = omsi_cfg::resolve_path(&omsi_cfg::resolve_path(&model_dir, "model"), &md.file);
                    let mesh_path = if omsi_cfg::vfs::is_file(&mesh_path) {
                        mesh_path
                    } else {
                        omsi_cfg::resolve_path(&model_dir, &md.file)
                    };
                    if let Ok(m) = omsi_o3d::load_mesh(&mesh_path) {
                        list.push((mesh_from_o3d(&m), m.materials.clone(), md.materials.clone()));
                    }
                }
                lower_lods.push((model.lods[l].min_size, list));
            }
            let lod0_min = model.lods.first().map(|l| l.min_size).unwrap_or(0.0);
            // [CTC] paint schemes (.cti items): retain their texture keys and folders so
            // the selected advertisements can be resolved when a placement chooses them.
            let ctc_schemes: Vec<(String, Vec<omsi_sim::vehicle::PaintScheme>)> = model
                .ctc
                .iter()
                .map(|c| {
                    (
                        c.variable.clone(),
                        omsi_sim::vehicle::load_paint_schemes(&omsi_cfg::resolve_path(
                            &sco_dir, &c.path,
                        )),
                    )
                })
                .collect();
            let paint_schemes: Vec<omsi_sim::vehicle::PaintScheme> = ctc_schemes
                .iter()
                .flat_map(|(_, schemes)| schemes.iter().cloned())
                .collect();
            let mut dynamic_textures: Vec<DynamicTextureGroup> = ctc_schemes
                .iter()
                .map(|(variable, schemes)| DynamicTextureGroup {
                    variable: variable.clone(),
                    choices: schemes
                        .iter()
                        .map(|scheme| {
                            scheme
                                .textures
                                .iter()
                                .filter_map(|(name, file)| {
                                    model
                                        .ctc_textures
                                        .iter()
                                        .find(|(ctc_name, _)| ctc_name.eq_ignore_ascii_case(name))
                                        .map(|(_, default)| {
                                            (default.clone(), file.clone(), scheme.dir.clone())
                                        })
                                })
                                .collect()
                        })
                        .collect(),
                })
                .collect();
            // Scenery models can also use the same script-variable texture selectors as
            // vehicles. Each [newtexchangemaster] is an independent dynamic texture group.
            dynamic_textures.extend(
                omsi_model::load_texchanges(&model_dir, &model.texchanges)
                    .into_iter()
                    .map(|master| {
                        let omsi_model::TexChangeMaster {
                            texture,
                            variable,
                            entries,
                            dir,
                        } = master;
                        DynamicTextureGroup {
                            variable,
                            choices: entries
                                .into_iter()
                                .map(|file| vec![(texture.clone(), file, dir.clone())])
                                .collect(),
                        }
                    }),
            );
            if let Some(ps) = scheme.and_then(|i| paint_schemes.get(i)) {
                let mut map: HashMap<String, String> = HashMap::new();
                for (ctc_name, file) in &ps.textures {
                    // (the scheme's picture lies in the scheme's folder, as for the buses;
                    // taken as a bare name it was looked for among the model's textures, not
                    // found, and the parked car stood there white)
                    let in_scheme = omsi_cfg::resolve_path(&ps.dir, file);
                    let file = if omsi_cfg::vfs::is_file(&in_scheme) { in_scheme.to_string_lossy().into_owned() } else { file.clone() };
                    for (name, default) in &model.ctc_textures {
                        if name.eq_ignore_ascii_case(ctc_name) {
                            map.insert(default.to_ascii_lowercase(), file.clone());
                        }
                    }
                }
                let subst = |t: &mut String| {
                    if let Some(n) = map.get(&t.to_ascii_lowercase()) {
                        *t = n.clone();
                    }
                };
                for (_, mats, overrides) in meshes
                    .iter_mut()
                    .chain(lower_lods.iter_mut().flat_map(|l| l.1.iter_mut()))
                {
                    mats.iter_mut().for_each(|m| subst(&mut m.texture));
                    overrides.iter_mut().for_each(|o| subst(&mut o.texture));
                }
            }
            let paint_scheme_count = paint_schemes.len();
            // scripts (or an empty program) for objects that are scripted or animated
            let animated = mesh_def_index.iter().any(|d| {
                !model.meshes[*d].animations.is_empty() || model.meshes[*d].visible.is_some()
            });
            let has_freetex = mesh_def_index.iter().any(|d| {
                model.meshes[*d].materials.iter().any(|o| !o.item && o.freetex.is_some())
            });
            let program = if !sco.scripts.scripts.is_empty()
                || !sco.scripts.stringvarlists.is_empty()
                || !sco.scripts.varlists.is_empty()
                || has_freetex
                || animated
                || sco.sound.is_some()
            {
                Some(Arc::new(omsi_sim::scenery::compile_scenery(
                    &self.root,
                    &sco.scripts,
                )))
            } else {
                None
            };
            let mesh_shadow = mesh_def_index
                .iter()
                .map(|d| model.meshes[*d].is_shadow)
                .collect();
            let mesh_casts = mesh_def_index.iter().map(|d| model.meshes[*d].shadow).collect();
            let deform = sco.crossing_height_deformation.as_ref().and_then(|f| {
                let mp = omsi_cfg::resolve_path(&omsi_cfg::resolve_path(&model_dir, "model"), f);
                let mp = if omsi_cfg::vfs::is_file(&mp) {
                    mp
                } else {
                    omsi_cfg::resolve_path(&model_dir, f)
                };
                match omsi_o3d::load_mesh(&mp) {
                    Ok(m) => Some(mesh_from_o3d(&m)),
                    Err(e) => {
                        log::warn!("crossing height deformation {}: {e}", mp.display());
                        None
                    }
                }
            });
            // Omsi.exe hands the deformation mesh to the model loader, which drapes every
            // [mesh] of the object onto it and rebuilds the normals from the faces
            // (D3DXComputeNormals): the file's normals of a crossing are never used.
            if deform.is_some() {
                for (mesh, _, _) in meshes
                    .iter_mut()
                    .chain(lower_lods.iter_mut().flat_map(|l| l.1.iter_mut()))
                {
                    omsi_geometry::compute_normals_d3d(mesh);
                }
            }
            // [terrainhole] <mesh>: the cutter that takes the ground away under a junction
            // or an underpass, so the carriageway is not buried under a mound of terrain
            let holes: Vec<MeshData> = sco
                .terrain_hole_sources(&model)
                .filter_map(|(hole_dir, f)| {
                    // the cutter sits next to the model, which is either the object's own
                    // folder or a `model` folder inside it
                    let mp = omsi_cfg::resolve_path(hole_dir, f);
                    let mp = if omsi_cfg::vfs::is_file(&mp) {
                        mp
                    } else {
                        omsi_cfg::resolve_path(&omsi_cfg::resolve_path(hole_dir, "model"), f)
                    };
                    match omsi_o3d::load_mesh(&mp) {
                        Ok(m) => Some(mesh_from_o3d(&m)),
                        Err(e) => {
                            log::warn!("terrain hole {}: {e}", mp.display());
                            None
                        }
                    }
                })
                .collect();
            let collision = sco
                .collision_mesh
                .as_ref()
                .filter(|_| !sco.no_collision)
                .and_then(|f| {
                    let mp = omsi_cfg::resolve_path(&omsi_cfg::resolve_path(&model_dir, "model"), f);
                    let mp = if omsi_cfg::vfs::is_file(&mp) {
                        mp
                    } else {
                        omsi_cfg::resolve_path(&model_dir, f)
                    };
                    let mp = if omsi_cfg::vfs::is_file(&mp) {
                        mp
                    } else {
                        omsi_cfg::resolve_path(&sco_dir, f)
                    };
                    omsi_o3d::load_mesh(&mp)
                        .map(|m| mesh_from_o3d(&m))
                        .map_err(|e| log::debug!("collision mesh {}: {e}", mp.display()))
                        .ok()
                });
            let paint = paint_at_foot(&sco, &meshes);
            Some(Arc::new(ObjectType {
                sco,
                sound_path: Default::default(),
                model,
                model_dir,
                meshes,
                mesh_visible,
                mesh_def_index,
                mesh_pivots,
                mesh_shadow,
                mesh_casts,
                program,
                lower_lods,
                lod0_min,
                paint_scheme_count,
                dynamic_textures,
                holes,
                deform,
                collision,
                paint,
                camera: Default::default(),
                collision_shape: Default::default(),
            }))
        })();
        // two loaders may have read the same type at once: all of them get the first copy,
        // so that it is uploaded (and evicted) once
        self.object_types
            .lock()
            .entry(key)
            .or_insert(loaded)
            .clone()
    }

    pub fn spline_type(&self, rel: &str) -> Option<Arc<SplineType>> {
        let key = rel.to_ascii_lowercase().replace('\\', "/");
        if let Some(t) = self.spline_types.lock().get(&key) {
            return t.clone();
        }
        let path = omsi_cfg::resolve_path(&self.root, rel);
        let loaded = Spline::load(&path)
            .map_err(|e| log::warn!("{e}"))
            .ok()
            .map(|def| {
                omsi_geometry::register_half_cant_width(rel, &def);
                let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                let dirs = texture_dirs(&self.root, &dir);
                let dirs: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
                let surf = def.textures.iter().map(|t| surf_map(&t.file, &dirs)).collect();
                Arc::new(SplineType { dir, def, surf })
            });
        self.spline_types
            .lock()
            .entry(key)
            .or_insert(loaded)
            .clone()
    }

    /// The whole map's road network and where its objects stand, read from the tile files
    /// alone - the splines' and objects' paths, no mesh, no texture - for the navigator,
    /// which must route beyond the tiles loaded around the bus. Objects placed on the ground
    /// take the tile's terrain height; editor-only splines and objects count (some maps put
    /// all their traffic paths on invisible splines).
    pub fn navigation_map(&self) -> NavigationMap {
        let tiles: Vec<(i32, i32, PathBuf)> = self.map_tiles().into_iter().map(|(_, x, y, p)| (x, y, p)).collect();
        navigation_map_of(&self.root, &tiles, &self.chrono_dirs.read())
    }
}

/// The same read with no `World` holding the caches, so that anything else that wants to
/// know what a map has (the launcher's map picture) reads it the way the navigator does.
/// `root` is the installation the map belongs to: what its tiles name - a `.sli`, a `.sco`
/// and the model beside it - is resolved against it, so a mod's own copy wins over the
/// original's and a map that lacks one borrows the other installation's (`omsi_cfg::
/// resolve_path`).
pub fn navigation_map_of(root: &Path, tiles: &[(i32, i32, PathBuf)], chrono_dirs: &[PathBuf]) -> NavigationMap {
    use rayon::prelude::*;
    let t0 = std::time::Instant::now();
    let scos: Mutex<HashMap<String, Option<Arc<SceneryObject>>>> = Mutex::new(HashMap::new());
    let sco_of = |file: &str| -> Option<Arc<SceneryObject>> {
        let key = file.trim().to_ascii_lowercase().replace('\\', "/");
        if let Some(v) = scos.lock().get(&key) {
            return v.clone();
        }
        let v = SceneryObject::load(&omsi_cfg::resolve_path(root, file)).ok().map(Arc::new);
        scos.lock().insert(key, v.clone());
        v
    };
    let slis: Mutex<HashMap<String, Option<Arc<Spline>>>> = Mutex::new(HashMap::new());
    let sli_of = |file: &str| -> Option<Arc<Spline>> {
        // (the navigator loads `.sli` through `World::spline_type` with the same two calls)
        let key = file.trim().to_ascii_lowercase().replace('\\', "/");
        if let Some(v) = slis.lock().get(&key) {
            return v.clone();
        }
        let v = Spline::load(&omsi_cfg::resolve_path(root, file)).ok().map(|def| {
            omsi_geometry::register_half_cant_width(file, &def);
            Arc::new(def)
        });
        slis.lock().insert(key, v.clone());
        v
    };
    #[allow(clippy::type_complexity)]
    let parts: Vec<(Vec<Lane>, Vec<(i64, DVec3)>, Vec<(DVec3, f64, String)>, Vec<(Vec<DVec3>, f32)>)> = tiles
        .par_iter()
        .map(|(tx, ty, path)| {
            let (tx, ty) = (*tx, *ty);
            let mut lanes = Vec::new();
            let mut positions = Vec::new();
            let mut signs = Vec::new();
            let mut roads = Vec::new();
            let Some(tile) = crate::tiles::read_tile(path, chrono_dirs) else {
                return (lanes, positions, signs, roads);
            };
            let origin2 = DVec2::new(tx as f64 * tile_size(), ty as f64 * tile_size());
            let terrain = Terrain::load(&tile_companion(&path, ".terrain")).unwrap_or_else(|_| Terrain::flat());
            for sp in tile.splines.iter().filter(|s| !s.deleted && !s.file.trim().is_empty()) {
                let Some(def) = sli_of(&sp.file) else { continue };
                if !def.paths.iter().any(|p| p.kind == 0) {
                    // Short surfaces also corroborate editor-only traffic paths at
                    // junctions; the navigator filters decorative patches for display.
                    if sp.length >= 2.0 {
                        let curve = SplineCurve::from_map(sp, origin2).with_sli(&def);
                        let n = ((curve.length / 4.0).ceil() as usize).clamp(1, 400);
                        let side = if sp.mirror { -1.0 } else { 1.0 };
                        for (lo, hi, z) in road_sections(&sp.file, &def) {
                            let offset = side * ((lo + hi) * 0.5) as f64;
                            let pts: Vec<DVec3> = (0..=n).map(|k| curve.offset_point(curve.length * k as f64 / n as f64, offset, z as f64)).collect();
                            roads.push((pts, hi - lo));
                        }
                    }
                }
                if def.paths.is_empty() {
                    continue;
                }
                let curve = SplineCurve::from_map(sp, origin2);
                let mut new_lanes = spline_lanes(&def, sp, &curve, (tx, ty));
                for l in new_lanes.iter_mut() {
                    l.invisible = def.only_editor;
                }
                lanes.extend(new_lanes);
            }
            for o in &tile.objects {
                if o.file.trim().is_empty() {
                    continue;
                }
                let (x, y) = (origin2.x + o.pos[0], origin2.y + o.pos[1]);
                let ground = || {
                    let (lx, ly) = ((x - origin2.x).clamp(0.0, tile_size()) as f32, (y - origin2.y).clamp(0.0, tile_size()) as f32);
                    terrain.sample(lx, ly) as f64
                };
                // street name signs carry the street's name as their text
                if is_street_sign(&o.file) {
                    if let Some(name) = o.extra.first().map(|t| t.trim()).filter(|t| t.chars().filter(|c| c.is_alphabetic()).count() >= 3) {
                        signs.push((DVec3::new(x, y, o.pos[2] + ground()), o.rot[0], name.to_string()));
                    }
                }
                let Some(sco) = sco_of(&o.file) else {
                    positions.push((o.id, DVec3::new(x, y, o.pos[2] + ground())));
                    continue;
                };
                let absolute = sco.absolute_height();
                let pos = DVec3::new(x, y, if absolute { o.pos[2] } else { o.pos[2] + ground() });
                positions.push((o.id, pos));
                if !sco.paths.is_empty() {
                    lanes.extend(object_lanes(&sco, pos, [o.rot[0], 0.0, 0.0], None, (tx, ty), o.id, &o.rules));
                }
            }
            (lanes, positions, signs, roads)
        })
        .collect();
    let mut lanes = Vec::new();
    let mut positions = HashMap::new();
    let mut signs = Vec::new();
    let mut roads = Vec::new();
    for (l, p, s, r) in parts {
        lanes.extend(l);
        positions.extend(p);
        signs.extend(s);
        roads.extend(r);
    }
    log::info!("navigation map: {} roads without a path for cars", roads.len());
    log::info!(
        "navigation map: {} tiles, {} lanes, {} objects placed, {} street name signs, {} object types, {} spline types, {:.1} s",
        tiles.len(),
        lanes.len(),
        positions.len(),
        signs.len(),
        scos.lock().len(),
        slis.lock().len(),
        t0.elapsed().as_secs_f64()
    );
    NavigationMap { lanes, road_surfaces: roads, places: positions, signs }
}

impl World {
    /// Every tile of global.cfg's `[map]` list whose file exists, with its index in that list
    /// (repeaters and timetable tracks name a tile by that index).
    pub fn map_tiles(&self) -> Vec<(usize, i32, i32, PathBuf)> {
        self.global
            .tiles
            .iter()
            .map(|t| (t.index, t.x, t.y, omsi_cfg::resolve_path(&self.map_dir, &t.file)))
            .filter(|t| omsi_cfg::vfs::is_file(&t.3))
            .collect()
    }

    /// The map has tile `key` (listed in global.cfg, and its file exists).
    pub fn has_tile(&self, key: (i32, i32)) -> bool {
        self.layout().paths.contains_key(&key)
    }

    /// Tiles within `radius` (in tiles, Chebyshev) of `center`; `None` = all.
    pub fn select_tiles(
        &self,
        center: Option<(i32, i32)>,
        radius: Option<i32>,
    ) -> Vec<(i32, i32, PathBuf)> {
        self.global
            .tiles
            .iter()
            .filter(|t| match (center, radius) {
                (Some((cx, cy)), Some(r)) => (t.x - cx).abs() <= r && (t.y - cy).abs() <= r,
                _ => true,
            })
            .map(|t| (t.x, t.y, omsi_cfg::resolve_path(&self.map_dir, &t.file)))
            .filter(|(_, _, p)| omsi_cfg::vfs::is_file(p))
            .collect()
    }

    /// Load, tessellate and upload `tiles` all at once (offscreen runs; the window streams).
    pub fn build_scene(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        tiles: &[(i32, i32, PathBuf)],
    ) -> Result<LoadStats> {
        let t0 = std::time::Instant::now();
        // OMSI_BATCH=n: prepare the tiles a few at a time, as the window's streaming does
        let batch = omsi_cfg::env::var("OMSI_BATCH")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(tiles.len().max(1));
        let mut stats = LoadStats {
            tiles: tiles.len(),
            ..Default::default()
        };
        let mut t_prepare = std::time::Duration::ZERO;
        for chunk in tiles.chunks(batch) {
            let t = std::time::Instant::now();
            let (prepared, s) = self.prepare_tiles(chunk);
            t_prepare += t.elapsed();
            stats.add_prepared(&s);
            for p in prepared {
                self.upload_tile(renderer, scene, p, &mut stats);
            }
        }
        let t1 = t0 + t_prepare;
        // nothing more is loaded after this
        self.staged.lock().clear();
        self.refresh_tile_lists();
        stats.log_ground();
        if omsi_cfg::env::var_os("OMSI_PROFILE").is_some() {
            log::info!(
                "build_scene: prepared in {:.2} s, uploaded in {:.2} s",
                (t1 - t0).as_secs_f64(),
                t1.elapsed().as_secs_f64()
            );
        }
        // OMSI_DUMP_GROUND=file: every loaded tile's final terrain and how much of it the
        // roads cut away, as text (to compare a whole-map load with a streamed one)
        if let Some(path) = omsi_cfg::env::var_os("OMSI_DUMP_GROUND") {
            let mut out = String::new();
            let terrains = self.terrains.read();
            let surfaces = self.surfaces.read();
            let mut keys: Vec<&(i32, i32)> = terrains.keys().collect();
            keys.sort();
            for k in keys {
                let t = &terrains[k];
                let cut = surfaces
                    .get(k)
                    .map(|sf| {
                        let n = sf.size;
                        let cell = tile_size() as f32 / n as f32;
                        (0..n * n)
                            .filter(|&i| {
                                sf.cut_at(
                                    (i % n) as f32 * cell + cell * 0.5,
                                    (i / n) as f32 * cell + cell * 0.5,
                                    t.sample(
                                        (i % n) as f32 * cell + cell * 0.5,
                                        (i / n) as f32 * cell + cell * 0.5,
                                    ),
                                    surface_flush(),
                                )
                            })
                            .count()
                    })
                    .unwrap_or(0);
                out.push_str(&format!(
                    "{} {} {} {}
",
                    k.0,
                    k.1,
                    cut,
                    t.heights
                        .iter()
                        .map(|h| format!("{h:.3}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            }
            let _ = std::fs::write(path, out);
        }
        stats.textures = self.gpu.lock().textures.len();
        Ok(stats)
    }

    /// The sim date moved on (midnight, a clock set by hand): the chrono scenarios in force
    /// then (OMSI re-evaluates them at the day's change: the original → the original).
    /// Returns the tiles a scenario that came or went changes (to be read again), and
    /// forgets the map index built with the old ones.
    pub fn set_date(&self, date: i32) -> Vec<(i32, i32)> {
        let new = omsi_map::active_chrono_dirs(&self.map_dir, date);
        let old = self.chrono_dirs.read().clone();
        if new == old {
            return Vec::new();
        }
        let mut tiles: hashbrown::HashSet<(i32, i32)> = hashbrown::HashSet::new();
        for d in old.iter().filter(|d| !new.contains(d)).chain(new.iter().filter(|d| !old.contains(d))) {
            log::info!("chrono: {} {} on {date}", d.display(), if new.contains(d) { "comes into force" } else { "ends" });
            for (name, is_dir) in omsi_cfg::vfs::list_dir(d).unwrap_or_default() {
                if !is_dir {
                    if let Some(k) = omsi_map::tile_index_of(&name.to_string_lossy()) {
                        tiles.insert(k);
                    }
                }
            }
        }
        *self.chrono_dirs.write() = new;
        *self.index.lock() = None;
        let keys: Vec<(i32, i32)> = tiles.into_iter().collect();
        self.forget_staged(&keys);
        keys
    }

    /// Drop the read tiles `keys` from the staging cache (they are read again when asked).
    pub fn forget_staged(&self, keys: &[(i32, i32)]) {
        let mut st = self.staged.lock();
        for k in keys {
            st.remove(k);
        }
    }

    /// Drop every read tile from the staging cache.
    pub fn forget_all_staged(&self) {
        self.staged.lock().clear();
    }

    /// The map index, built on first use (every tile file read once, in parallel).
    /// How many passengers get off at stop object `id` (see `tiles::stop_exit_weight`; a
    /// stop without strings: the defaults' mean, 0.5).
    pub fn stop_exit_weight(&self, id: i64) -> f32 {
        self.index().stop_weights.get(&id).copied().unwrap_or(0.5)
    }

    /// Stop object `id`'s (pass_enter_max, pass_enter_min) (see `tiles::stop_enter`; a
    /// stop without strings: the defaults, 1 and 0).
    pub fn stop_enter(&self, id: i64) -> (f32, f32) {
        self.index().stop_enter.get(&id).copied().unwrap_or((1.0, 0.0))
    }

    /// The side stop object `id`'s platform lies on (see `tiles::stop_side`): 0 = right,
    /// 1 = the other, 2 = both; a stop the map says nothing about: 0.
    pub fn stop_side(&self, id: i64) -> f32 {
        self.index().stop_side.get(&id).copied().unwrap_or(0.0)
    }

    /// Stop object `id`'s length (see `tiles::stop_length`; 30 m when the map says nothing).
    pub fn stop_length(&self, id: i64) -> f32 {
        self.index().stop_length.get(&id).copied().unwrap_or(30.0)
    }

    pub fn index(&self) -> Arc<MapIndex> {
        let mut g = self.index.lock();
        if let Some(ix) = g.as_ref() {
            return ix.clone();
        }
        let mut built = MapIndex::build(&self.map_tiles(), &self.chrono_dirs.read(), &self.root);
        // the index's object positions go to `object_positions` (kept once, not twice: 345 000
        // objects on Ahlheim took 30 MB in each)
        let objects = std::mem::take(&mut built.objects);
        {
            let dups = std::mem::take(&mut built.duplicates);
            if !dups.is_empty() {
                log::info!("map index: {} objects share their id with an object of another tile", dups.len());
            }
            let mut d = self.object_dups.lock();
            for (k, v) in dups {
                d.entry(k).or_insert(v);
            }
        }
        let ix = Arc::new(built);
        // what the map names that is not installed: counted always, listed on request
        // (OMSI_CHECK_TYPES=1); loading leaves those objects out either way
        let groups = ix.missing_files(&self.root);
        let files: usize = groups.iter().map(|g| g.1.len()).sum();
        if files > 0 {
            let records: usize = groups.iter().flat_map(|g| g.1.iter().map(|m| m.1)).sum();
            log::warn!("{} object and spline files named by the map are not installed ({} records, in {} add-on folders; OMSI_CHECK_TYPES=1 lists them)", files, records, groups.len());
            if omsi_cfg::env::var_os("OMSI_CHECK_TYPES").is_some() {
                for (folder, list) in &groups {
                    log::info!(
                        "  missing from {folder}: {} files, {} records",
                        list.len(),
                        list.iter().map(|m| m.1).sum::<usize>()
                    );
                    for (f, n, t) in list {
                        log::info!("    {f}  ({n} records, e.g. tile {},{})", t.0, t.1);
                    }
                }
            }
        }
        {
            let mut positions = self.object_positions.lock();
            positions.reserve(objects.len());
            for (id, (_, pos, rot)) in objects {
                positions.entry(id).or_insert((pos, rot));
            }
        }
        *g = Some(ix.clone());
        ix
    }

    /// Everything a batch of tiles needs before the GPU. Nothing here needs the renderer,
    /// so the window runs it on a worker thread.
    ///
    /// A tile's final ground depends on the roads and crossings around it, and its road cut
    /// on the surfaces of its neighbours as they finally stand, so a tile cannot be finished
    /// from its own file. Every tile is therefore staged first (read and typed once, see
    /// [`StagedTile`], kept in a cache), and a tile is only placed once everything its
    /// neighbourhood depends on is staged. What a tile works with is fixed by the map
    /// ([`TileLayout::sources_of`]), not by what else happens to be loaded, so a tile
    /// streamed in gets exactly the ground and the cut a whole-map load gives it.
    pub fn prepare_tiles(&self, tiles: &[(i32, i32, PathBuf)]) -> (Vec<Prepared>, LoadStats) {
        self.prepare_tiles_impl(tiles, false)
    }

    /// Prepare the streamed map's first visible area with bounded diagnostics. This is kept
    /// separate from normal streaming so an ordinary drive does not produce per-tile log I/O.
    pub fn prepare_tiles_initial(&self, tiles: &[(i32, i32, PathBuf)]) -> (Vec<Prepared>, LoadStats) {
        self.prepare_tiles_impl(tiles, true)
    }

    fn prepare_tiles_impl(&self, tiles: &[(i32, i32, PathBuf)], initial_diag: bool) -> (Vec<Prepared>, LoadStats) {
        let profile = omsi_cfg::env::var_os("OMSI_PROFILE").is_some();
        let t0 = std::time::Instant::now();
        let index = self.index();
        let layout = self.layout();
        let t1 = std::time::Instant::now();
        let keys: Vec<(i32, i32)> = tiles.iter().map(|t| (t.0, t.1)).collect();
        if initial_diag {
            log::info!(
                "tile loading: first-area batch {} tile(s) {:?}: index/layout {:.2} s",
                keys.len(),
                keys,
                (t1 - t0).as_secs_f64()
            );
        }
        // the batch needs the sources of every tile next to it (their placed surfaces go
        // into its cut)
        let mut wanted: Vec<(i32, i32)> = Vec::new();
        for k in &keys {
            for q in layout.ring(*k) {
                wanted.extend(layout.sources_of(q).iter().copied());
            }
        }
        wanted.sort();
        wanted.dedup();
        let missing: Vec<(i32, i32)> = {
            let cache = self.staged.lock();
            wanted
                .iter()
                .copied()
                // a tile placed before gave its meshes to the GPU and is read again
                .filter(|k| {
                    cache
                        .get(k)
                        .map(|s| keys.contains(k) && s.meshes.lock().is_none())
                        .unwrap_or(true)
                })
                .collect()
        };
        if initial_diag && !missing.is_empty() {
            log::info!(
                "tile loading: first-area batch needs {} staged dependency tile(s): {:?}",
                missing.len(),
                missing
            );
        }
        let fresh: Vec<((i32, i32), Arc<StagedTile>)> = missing
            .par_iter()
            .filter_map(|k| {
                let path = layout.paths.get(k)?;
                let started = std::time::Instant::now();
                if initial_diag {
                    log::info!(
                        "tile loading: staging tile {},{} ({})",
                        k.0,
                        k.1,
                        path.file_name().unwrap_or_default().to_string_lossy()
                    );
                }
                let staged = Arc::new(self.stage_tile(k.0, k.1, path, &index));
                let secs = started.elapsed().as_secs_f64();
                if initial_diag {
                    if secs >= 2.0 {
                        log::warn!(
                            "tile loading: slow stage tile {},{} ({}) took {:.2} s",
                            k.0,
                            k.1,
                            path.file_name().unwrap_or_default().to_string_lossy(),
                            secs
                        );
                    } else {
                        log::info!(
                            "tile loading: staged tile {},{} ({}) in {:.2} s",
                            k.0,
                            k.1,
                            path.file_name().unwrap_or_default().to_string_lossy(),
                            secs
                        );
                    }
                } else if secs >= 5.0 {
                    log::warn!(
                        "tile streaming: staging tile {},{} ({}) took {:.2} s",
                        k.0,
                        k.1,
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        secs
                    );
                }
                Some((*k, staged))
            })
            .collect();
        // what this batch works with, held here: the cache may drop entries meanwhile
        let staged: HashMap<(i32, i32), Arc<StagedTile>> = {
            let mut cache = self.staged.lock();
            for (k, st) in fresh {
                cache.insert(k, st);
            }
            wanted
                .iter()
                .filter_map(|k| cache.get(k).map(|s| (*k, s.clone())))
                .collect()
        };
        let t2 = std::time::Instant::now();
        if initial_diag {
            log::info!(
                "tile loading: first-area staging finished in {:.2} s ({} dependency tile(s) read now)",
                (t2 - t1).as_secs_f64(),
                missing.len()
            );
        }
        let stats = Mutex::new(LoadStats {
            tiles: tiles.len(),
            ..Default::default()
        });
        let mut prepared: Vec<Prepared> = keys
            .par_iter()
            .filter_map(|k| {
                let started = std::time::Instant::now();
                if initial_diag {
                    log::info!("tile loading: placing tile {},{}", k.0, k.1);
                }
                let out = self.place_tile(*k, &staged, &layout, &stats);
                let secs = started.elapsed().as_secs_f64();
                if initial_diag {
                    if secs >= 2.0 {
                        log::warn!("tile loading: slow place tile {},{} took {:.2} s", k.0, k.1, secs);
                    } else {
                        log::info!("tile loading: placed tile {},{} in {:.2} s", k.0, k.1, secs);
                    }
                } else if secs >= 5.0 {
                    log::warn!("tile streaming: placing tile {},{} took {:.2} s", k.0, k.1, secs);
                }
                out
            })
            .collect();
        let t3 = std::time::Instant::now();
        if initial_diag {
            log::info!(
                "tile loading: first-area placement finished in {:.2} s; cutting terrain/textures",
                (t3 - t2).as_secs_f64()
            );
        }
        self.cut_terrain(&mut prepared, &staged, &layout);
        let cut_secs = t3.elapsed().as_secs_f64();
        if initial_diag {
            let total = t0.elapsed().as_secs_f64();
            if total >= 2.0 {
                log::warn!(
                    "tile loading: first-area batch prepared in {:.2} s: index/layout {:.2}, stage {:.2}, place {:.2}, cut/textures {:.2}",
                    total,
                    (t1 - t0).as_secs_f64(),
                    (t2 - t1).as_secs_f64(),
                    (t3 - t2).as_secs_f64(),
                    cut_secs
                );
            } else {
                log::info!(
                    "tile loading: first-area batch prepared in {:.2} s: index/layout {:.2}, stage {:.2}, place {:.2}, cut/textures {:.2}",
                    total,
                    (t1 - t0).as_secs_f64(),
                    (t2 - t1).as_secs_f64(),
                    (t3 - t2).as_secs_f64(),
                    cut_secs
                );
            }
        } else if profile {
            log::info!("prepare {} tiles ({} staged, {} read now): index {:.2} s, read {:.2} s, place {:.2} s, cut + textures {:.2} s", tiles.len(), staged.len(), missing.len(), (t1 - t0).as_secs_f64(), (t2 - t1).as_secs_f64(), (t3 - t2).as_secs_f64(), cut_secs);
        }
        let stats = stats.into_inner();
        (prepared, stats)
    }

    /// Which tiles the map has and which tiles each one depends on, built on first use.
    pub fn layout(&self) -> Arc<TileLayout> {
        if let Some(l) = self.layout.lock().as_ref() {
            return l.clone();
        }
        let index = self.index();
        let paths: HashMap<(i32, i32), PathBuf> = self
            .select_tiles(None, None)
            .into_iter()
            .map(|(x, y, p)| ((x, y), p))
            .collect();
        let mut sources: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
        for k in paths.keys() {
            let list = sources.entry(*k).or_default();
            for (dx, dy) in NEIGHBOURHOOD {
                if paths.contains_key(&(k.0 + dx, k.1 + dy)) {
                    list.push((k.0 + dx, k.1 + dy));
                }
            }
        }
        // a spline reaching further than its neighbours shapes the tiles it reaches
        let (lo, hi) = paths.keys().fold(
            ((i32::MAX, i32::MAX), (i32::MIN, i32::MIN)),
            |(lo, hi), k| {
                (
                    (lo.0.min(k.0), lo.1.min(k.1)),
                    (hi.0.max(k.0), hi.1.max(k.1)),
                )
            },
        );
        let ts = tile_size();
        let mut extra = 0usize;
        for (s, c) in &index.covers {
            if !paths.contains_key(s) {
                continue;
            }
            let kx0 = (((c[0] - SOURCE_MARGIN) / ts).floor() as i32 - 1).max(lo.0);
            let kx1 = (((c[2] + SOURCE_MARGIN) / ts).floor() as i32).min(hi.0);
            let ky0 = (((c[1] - SOURCE_MARGIN) / ts).floor() as i32 - 1).max(lo.1);
            let ky1 = (((c[3] + SOURCE_MARGIN) / ts).floor() as i32).min(hi.1);
            for kx in kx0..=kx1 {
                for ky in ky0..=ky1 {
                    if (kx - s.0).abs() <= 1 && (ky - s.1).abs() <= 1 {
                        continue;
                    }
                    let (x0, y0) = (
                        kx as f64 * ts - SOURCE_MARGIN,
                        ky as f64 * ts - SOURCE_MARGIN,
                    );
                    let (x1, y1) = (
                        (kx + 1) as f64 * ts + SOURCE_MARGIN,
                        (ky + 1) as f64 * ts + SOURCE_MARGIN,
                    );
                    if c[2] < x0 || c[0] > x1 || c[3] < y0 || c[1] > y1 {
                        continue;
                    }
                    if let Some(list) = sources.get_mut(&(kx, ky)) {
                        list.push(*s);
                        extra += 1;
                    }
                }
            }
        }
        for l in sources.values_mut() {
            l.sort();
            l.dedup();
        }
        if extra > 0 {
            log::info!(
                "tile layout: {extra} times a tile takes a long spline from beyond its neighbours"
            );
        }
        let layout = Arc::new(TileLayout { paths, sources });
        *self.layout.lock() = Some(layout.clone());
        layout
    }

    /// Forget staged tiles no loaded (or requested) tile depends on any more.
    pub fn trim_staged(&self, requested: &hashbrown::HashSet<(i32, i32)>) {
        let layout = self.layout();
        let mut keep: hashbrown::HashSet<(i32, i32)> = hashbrown::HashSet::new();
        let loaded: Vec<(i32, i32)> = self.tile_state.lock().keys().copied().collect();
        for k in loaded.iter().chain(requested.iter()) {
            for q in layout.ring(*k) {
                keep.extend(layout.sources_of(q).iter().copied());
            }
        }
        self.staged.lock().retain(|k, _| keep.contains(k));
    }

    /// The staged tiles `key` depends on, from `staged`.
    fn sources(
        layout: &TileLayout,
        staged: &HashMap<(i32, i32), Arc<StagedTile>>,
        key: (i32, i32),
    ) -> HashMap<(i32, i32), Arc<StagedTile>> {
        layout
            .sources_of(key)
            .iter()
            .filter_map(|k| staged.get(k).map(|s| (*k, s.clone())))
            .collect()
    }

    /// Read one tile: its records with the chrono patches applied, the terrain as the file has
    /// it, the water, the spline meshes and lanes, and every object with its type.
    fn stage_tile(&self, tx: i32, ty: i32, path: &Path, index: &MapIndex) -> StagedTile {
        let origin2 = DVec2::new(tx as f64 * tile_size(), ty as f64 * tile_size());
        let origin = DVec3::new(origin2.x, origin2.y, 0.0);
        let tile = crate::tiles::read_tile(path, &self.chrono_dirs.read());
        // (an active chrono patch may bring the tile's terrain: see `Tile::terrain_from`)
        let terrain_path = match &tile {
            Some(t) => crate::tiles::terrain_file(t, path),
            None => tile_companion(path, ".terrain"),
        };
        let edited = self.terrain_edits.lock().get(&(tx, ty)).cloned();
        let base_terrain = edited.unwrap_or_else(|| Terrain::load(&terrain_path).unwrap_or_else(|_| Terrain::flat()));
        let counts = Mutex::new(LoadStats::default());
        let mut out = StagedTile {
            tx,
            ty,
            origin,
            path: path.to_path_buf(),
            base_terrain,
            align: Vec::new(),
            hole_rims: Vec::new(),
            water: None,
            bakes_light_map: false,
            splines: Vec::new(),
            meshes: Mutex::new(Some(Vec::new())),
            drive: Vec::new(),
            lanes: Mutex::new(Vec::new()),
            street_points: Vec::new(),
            objects: Vec::new(),
            anchors: Vec::new(),
            counts: LoadStats::default(),
            resolved: std::sync::OnceLock::new(),
        };
        let Some(tile) = tile else {
            return out;
        };
        out.bakes_light_map = tile.variable_terrain_lightmap;
        // a tile with water carries one surface with a height at each corner. As in Omsi.exe
        // (TMapKachel.loadMapFile 0x792188) only a `[water]` tile has it: the editor never
        // deletes the `.water` file of a tile whose water was removed. The corners start at
        // -5 m (TFileWater 0x7ab6f0) and take the file's heights only when its count is 1
        // (0x7ab760).
        out.water = tile.has_water.then(|| {
            match omsi_cfg::vfs::read(&crate::tiles::water_file(&tile, path))
                .ok()
                .map(|b| omsi_map::terrain::Water::parse(&b))
            {
                Some(w) if w.count == 1 && w.values.len() >= 4 => [w.values[0], w.values[1], w.values[2], w.values[3]],
                _ => [-5.0; 4],
            }
        });
        let debug_splines = omsi_cfg::env::var_os("OMSI_DEBUG_SPLINES").is_some();
        let mut lanes: Vec<Lane> = Vec::new();
        let mut meshes: Vec<Arc<MeshData>> = Vec::new();
        for s in tile
            .splines
            .iter()
            .filter(|s| !s.deleted && !s.file.trim().is_empty())
        {
            let Some(st) = self.spline_type(&s.file) else {
                self.note_missing(&s.file, "spline", tx, ty, s.id);
                continue;
            };
            let curve = SplineCurve::from_map(s, origin2);
            if omsi_cfg::env::var_os("OMSI_CHECK_SPLINES").is_some() {
                SPLINE_ENDS.lock().insert(s.id, (curve.point_at(0.0), curve.end_point(), s.prev_id, s.next_id, s.file.clone()));
            }
            // editor-only splines (invisible streets, flight paths) still carry lanes
            let mut new_lanes = spline_lanes(&st.def, s, &curve, (tx, ty));
            for l in new_lanes.iter_mut() {
                l.invisible = st.def.only_editor;
            }
            lanes.extend(new_lanes);
            if st.def.only_editor {
                if debug_splines {
                    log::info!("tile {tx},{ty} spline {} {} is editor-only (lanes only, no mesh) at ({:.1},{:.1},{:.2})", s.id, s.file, s.pos[0], s.pos[1], s.pos[2]);
                }
                continue;
            }
            // What the wheels stand on is the spline's drawn mesh, not its `[heightprofile]`:
            // Omsi.exe's ground query (0x7a0814) casts a ray from 3 m over the point down
            // into each spline segment of the tile (0x5b2d94 -> 0x7c40c8, D3DXIntersect on
            // the segment's mesh +0xa0), and that mesh is the one TSplineSegment.Generate
            // (0x5b1e14) builds from the `[profile]`/`[profilepnt]` lists (+0xc) for drawing.
            // The height profile (+0x20) is read by the editor's "is the point on this
            // spline" test alone (0x5b2b1c). Taken as the ground, a height profile wider than
            // the drawn road reached under the bus from the road beside it, one lower than
            // the asphalt sank the wheels into it, and a road without one had no ground at
            // all. `OMSI_HEIGHTPROFILE_GROUND=1` goes back to the height profiles (A/B).
            if heightprofile_ground() {
                let hp = omsi_geometry::build_height_profile_mesh(&st.def, &curve, s.mirror, origin);
                if !hp.is_empty() {
                    let b = mesh_bounds(&hp, &Mat4::IDENTITY, origin);
                    out.drive.push((hp, b, None));
                }
            }
            let mesh = build_spline_mesh(&st.def, &curve, s.mirror, origin);
            // OMSI_CHECK_SPIKES: a face standing taller than the profile, the gradient and
            // the cant allow (a spike out of the road)
            if omsi_cfg::env::var_os("OMSI_CHECK_SPIKES").is_some() && !mesh.is_empty() {
                let (zlo, zhi) = st.def.profiles.iter().flat_map(|p| p.points.iter().map(|q| q.z)).fold((f32::MAX, f32::MIN), |(a, b), z| (a.min(z), b.max(z)));
                let n = omsi_geometry::spline_station_count(&st.def, &curve).max(1);
                let step = curve.length / n as f64;
                let slope = curve.grad_start.abs().max(curve.grad_end.abs()) / 100.0;
                let cant = curve.cant_start.abs().max(curve.cant_end.abs()) / 100.0 * 2.0 * omsi_geometry::half_cant_width(&st.def).min(20.0);
                let allow = (zhi - zlo) as f64 + slope * step * 2.0 + cant + 0.5;
                let worst = mesh.indices.chunks_exact(3).map(|t| {
                    let z = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize].z);
                    (z.iter().cloned().fold(f32::MIN, f32::max) - z.iter().cloned().fold(f32::MAX, f32::min)) as f64
                }).fold(0.0f64, f64::max);
                if worst > allow {
                    log::info!("spike: tile {tx},{ty} spline {} {} face {worst:.1} m tall (allowed {allow:.1}) len {:.1} r {:.1} grad {:.2}/{:.2} h {:?} cant {:.1}/{:.1} skew {:.2}/{:.2} at ({:.1}, {:.1}, {:.1})", s.id, s.file, s.length, s.radius, s.grad_start, s.grad_end, s.delta_h, s.cant_start, s.cant_end, s.skew_start, s.skew_end, origin2.x + s.pos[0], origin2.y + s.pos[1], s.pos[2]);
                }
            }
            if debug_splines {
                let (lo, hi) = mesh.positions.iter().fold(
                    (glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN)),
                    |(lo, hi), v| (lo.min(*v), hi.max(*v)),
                );
                let tz = out.base_terrain.sample(
                    s.pos[0].clamp(0.0, tile_size()) as f32,
                    s.pos[1].clamp(0.0, tile_size()) as f32,
                );
                log::info!("tile {tx},{ty} spline {} {} h={} len={:.1} r={:.1} start=({:.1},{:.1},{:.2}) terrain_z={:.2} verts={} tris={} profiles={} tex={} local z {:.2}..{:.2}", s.id, s.file, s.is_h, s.length, s.radius, s.pos[0], s.pos[1], s.pos[2], tz, mesh.positions.len(), mesh.indices.len() / 3, st.def.profiles.len(), st.def.textures.len(), lo.z, hi.z);
            }
            if !mesh.is_empty() {
                // `[spline_terrain_align]` / `_2 <m>`: the editor pulled the ground onto
                // this road, and OMSI does it again on every load.
                let aligned = s.terrain_align_flag || s.terrain_align.is_some();
                // (a road the ground was pulled onto lies on it whatever the heights say; a
                // wire - under a metre across - would only flicker in the shadow map)
                let (lo_x, hi_x) = st.def.profiles.iter().flat_map(|p| p.points.iter().map(|q| q.x)).fold((f32::MAX, f32::MIN), |(a, b), x| (a.min(x), b.max(x)));
                let casts_shadow = !aligned && hi_x - lo_x >= 1.0 && {
                    let step = mesh.positions.len().div_ceil(64).max(1);
                    mesh.positions.iter().step_by(step).all(|p| p.z - out.base_terrain.sample(p.x.clamp(0.0, tile_size() as f32), p.y.clamp(0.0, tile_size() as f32)) > SPLINE_SHADOW_CLEARANCE)
                };
                if casts_shadow && debug_splines {
                    log::info!("tile {tx},{ty} spline {} {} stands clear of the ground: it casts a sun shadow", s.id, s.file);
                }
                if aligned {
                    out.align
                        .push((out.splines.len(), s.terrain_align.unwrap_or(1.0) as f32));
                    // Omsi.exe cuts the ground out under such a spline (the flag, or the
                    // `_2` number, goes to the segment, 0x79b95e -> +0x205, and Generate
                    // makes the outline from it; the terrain takes it with the `[terrainhole]`
                    // meshes, "Terrain hole cutting: Spline")
                    let mode = s.terrain_align.map(|v| v.clamp(0.0, 255.0) as u8).unwrap_or(1);
                    if omsi_cfg::env::var_os("OMSI_LIST_ALIGNED").is_some() {
                        let p = curve.point_at(curve.length * 0.5);
                        log::info!("aligned spline {} {} mode {mode} mid ({:.1}, {:.1}, {:.1}) heading {:.0}", s.id, s.file, p.x, p.y, p.z, curve.heading_at(curve.length * 0.5));
                    }
                    for rim in omsi_geometry::spline_hole_rims(&st.def, &curve, s.mirror, mode) {
                        let ring: Vec<_> = rim.iter().map(|v| v.truncate()).collect();
                        if omsi_geometry::outline_crosses_itself(&ring) {
                            if debug_splines {
                                log::info!("tile {tx},{ty} spline {} {}: its hole outline crosses itself, no hole (as in Omsi.exe)", s.id, s.file);
                            }
                            continue;
                        }
                        out.hole_rims.push(rim);
                    }
                }
                let bounds = mesh_bounds(&mesh, &Mat4::IDENTITY, origin);
                // the rasters only need the shape; the whole mesh waits for the upload
                let shape = MeshData {
                    positions: mesh.positions.clone(),
                    indices: mesh.indices.clone(),
                    ..Default::default()
                };
                // (every spline the game draws: Omsi.exe asks them all, roads or not)
                if !heightprofile_ground() {
                    out.drive.push((shape.clone(), bounds, omsi_geometry::SurfFaces::of(&mesh, &st.surf)));
                }
                let drivable = st.def.paths.iter().any(|pd| pd.kind == 0 || pd.kind == 1);
                let overlay = !st.def.profiles.is_empty()
                    && st.def.profiles.iter().all(|p| st.def.textures.get(p.texture).is_some_and(|t| t.alpha == 2));
                // A spline's visible profile is not necessarily a ground surface: power
                // cables and overhead trim have horizontal quads, and in the raster they cut
                // the ground up to their own height and stood in for the surface there. Only
                // one whose every profile hangs at least `SPLINE_OVERHEAD` over the spline's
                // line stays out; a wall, an embankment or a waterside without paths or
                // height profiles (Moges' `embankment.sli`) is ground all the same.
                let cuts_terrain = !overhead_only(&st.def) || !st.def.height_profiles.is_empty() || drivable;
                out.splines.push(StagedSpline {
                    shape,
                    ty: st,
                    bounds,
                    drivable,
                    overlay,
                    cuts_terrain,
                    casts_shadow,
                    sort_origin: curve.point_at(0.0),
                });
                meshes.push(Arc::new(mesh));
            }
        }
        // (every 2 m along the streets)
        out.street_points = lanes
            .iter()
            .filter(|l| l.kind == LaneKind::Street)
            .flat_map(|l| {
                let n = (l.length() / 2.0).ceil().max(1.0) as usize;
                (0..=n).map(move |k| l.at(l.length() * k as f32 / n as f32).0)
            })
            .collect();
        *out.lanes.lock() = lanes;
        *out.meshes.lock() = Some(meshes);
        let only_object = omsi_cfg::env::var("OMSI_ONLY_OBJECT")
            .ok()
            .map(|f| f.to_ascii_lowercase());
        let skip_object = omsi_cfg::env::var("OMSI_SKIP_OBJECT")
            .ok()
            .map(|f| f.to_ascii_lowercase());
        let wanted = |file: &str| {
            let f = file.to_ascii_lowercase();
            only_object
                .as_ref()
                .map(|o| f.contains(o.as_str()))
                .unwrap_or(true)
                && !skip_object
                    .as_ref()
                    .map(|s| f.contains(s.as_str()))
                    .unwrap_or(false)
        };
        // [object]
        let debug_outside = omsi_cfg::env::var_os("OMSI_DEBUG_OBJECTS").is_some();
        let mut outside = 0usize;
        for o in &tile.objects {
            if !wanted(&o.file) {
                continue;
            }
            let Some((ot, parked)) = self.placed_type(&o.file, &o.extra, o.id, tx, ty, &counts) else {
                continue;
            };
            // Objects with traffic paths (crossings, switches, road pieces) are stored with
            // absolute heights like the splines themselves; so are [absheight] ones.
            let absolute = ot.sco.absolute_height();
            // An object that stands on the terrain but lies outside its own tile is never
            // seen in OMSI: Omsi.exe finds its height with a ray from 1000 m down onto that
            // tile's terrain mesh only (0x79e43d -> 0x7ab594), and where the ray misses the
            // mesh it answers 10000 m more, which puts the object 11 km under the ground.
            // Maps copied from a `[worldcoordinates]` map keep such leftovers (Ahlheim's
            // Bostoner Weg: 588 objects past the edge, redrawn by the author where they
            // belong), and drawn they stood as houses and bushes in the road (#787).
            let edge = tile_size() + 1e-3;
            if !absolute && !((-1e-3..=edge).contains(&o.pos[0]) && (-1e-3..=edge).contains(&o.pos[1])) {
                if outside == 0 || debug_outside {
                    log::info!("object {} id {} at ({:.1}, {:.1}) lies outside tile ({tx}, {ty}): not shown, as in OMSI", o.file, o.id, o.pos[0], o.pos[1]);
                }
                outside += 1;
                continue;
            }
            let (x, y) = (origin2.x + o.pos[0], origin2.y + o.pos[1]);
            let place = if absolute {
                // On a `[worldcoordinates]` map the tile's splines are stretched onto the
                // grid with it, lengths included (`fit_to_world_grid`); a crossing has to
                // stretch as well, or the roads ending at its edges stop short of it - 1.6 cm
                // at a 27 m arm in Spandau, a line of sky across the road where the ground
                // is cut out underneath.
                let (kx, ky) = omsi_map::world_tile_scale(ty);
                Placement::Pose(Pose {
                    pos: DVec3::new(x, y, o.pos[2]),
                    rot: Mat4::from_scale(glam::Vec3::new(kx as f32, ky as f32, 1.0))
                        * object_rotation(omsi_geometry::map_rotation(o.rot)),
                })
            } else {
                Placement::Ground {
                    x,
                    y,
                    z: o.pos[2],
                    rot: omsi_geometry::map_rotation(o.rot),
                }
            };
            out.objects.push(StagedObject {
                ot,
                id: o.id,
                place,
                rules: o.rules.clone(),
                extra: o.extra.clone(),
                lamp_parent: o.var_parent,
                parked,
                map_object: true,
                instance: 0,
                key: o.id,
            });
        }
        if outside > 1 {
            log::info!("tile ({tx}, {ty}): {outside} objects lie outside the tile and are not shown, as in OMSI");
        }
        // [attachObj]
        for o in &tile.attach_objects {
            if !wanted(&o.file) {
                continue;
            }
            let Some((ot, parked)) = self.placed_type(&o.file, &o.extra, o.id, tx, ty, &counts) else {
                continue;
            };
            let Some(parent) = o.parent_id else { continue };
            out.objects.push(StagedObject {
                ot,
                id: o.id,
                place: Placement::Attached {
                    parent,
                    index: o.attach_index,
                    rot: omsi_geometry::map_rotation(o.rot),
                },
                rules: o.rules.clone(),
                extra: o.extra.clone(),
                lamp_parent: o.var_parent.or(Some(parent)),
                parked,
                map_object: false,
                instance: 0,
                key: o.id,
            });
        }
        // [splineAttachement] rows and their repeaters
        let mut rows = 0usize;
        for a in &tile.spline_attachments {
            if !wanted(&a.file) {
                continue;
            }
            let objs = crate::tiles::tile_row_objects(a, &tile.splines, origin2, Some(index));
            if objs.is_empty() || a.file.trim().is_empty() {
                continue;
            }
            // without its own type the whole row is missing; otherwise each object is on
            // its own (a car park row leaves some spaces empty)
            let Some(row_type) = self.object_type(&a.file) else {
                self.note_missing(&a.file, "scenery object", tx, ty, a.id);
                counts.lock().failed_objects += 1;
                continue;
            };
            rows += 1;
            let first = objs.iter().map(|o| o.1.index).min().unwrap_or(0);
            for (_, ro) in objs {
                if a.repeater.is_none() && ro.index == first {
                    out.anchors.push((a.id, ro.pose, row_type.clone()));
                }
                // every car park of a row draws its own car
                let key = a.id.wrapping_mul(1_000_003).wrapping_add(ro.index as i64);
                let Some((ot, parked)) = self.placed_type(&a.file, &a.strings, key, tx, ty, &counts) else {
                    continue;
                };
                let key = row_object_key(tx, ty, a.id, ro.index);
                out.objects.push(StagedObject {
                    ot,
                    id: a.id,
                    place: Placement::Pose(ro.pose),
                    rules: a.rules.clone(),
                    extra: a.strings.clone(),
                    lamp_parent: a.var_parent,
                    parked,
                    map_object: false,
                    instance: ro.index,
                    key,
                });
            }
        }
        let mut counts = counts.into_inner();
        counts.rows = rows;
        counts.attached = tile.attach_objects.len();
        out.counts = counts;
        out
    }

    /// Note a type that is not in this installation, once per file.
    fn note_missing(&self, file: &str, what: &'static str, tx: i32, ty: i32, id: i64) {
        let key = file.trim().to_ascii_lowercase().replace('\\', "/");
        if self.missing.lock().insert(key.clone(), what).is_none() {
            // the folder under Sceneryobjects/Splines names the add-on it comes with
            let addon = key.split('/').nth(1).unwrap_or("");
            log::warn!("{what} not found: {file} (add-on folder \"{addon}\"; first used by id {id} in tile {tx},{ty}) - left out");
        }
    }

    /// What the map uses and this installation lacks: the objects, splines and parked
    /// cars (file, what) and the textures of the tiles loaded so far.
    pub fn missing_content(&self) -> (Vec<(String, &'static str)>, Vec<String>) {
        let mut files: Vec<(String, &'static str)> = self.missing.lock().iter().map(|(f, w)| (f.clone(), *w)).collect();
        files.sort();
        let mut tex: Vec<String> = self.gpu.lock().misses.iter().cloned().collect();
        tex.sort();
        (files, tex)
    }

    /// The type an object record puts on the map: a parking space gets a random car of the
    /// map's parklist (a quarter of them stay empty, as in the original). None when nothing
    /// is to be placed; a missing type is counted and logged once.
    fn placed_type(
        &self,
        file: &str,
        captions: &[String],
        key: i64,
        tx: i32,
        ty: i32,
        stats: &Mutex<LoadStats>,
    ) -> Option<(Arc<ObjectType>, bool)> {
        if file.trim().is_empty() {
            // a damaged record names nothing
            return None;
        }
        let Some(ot) = self.object_type(file) else {
            self.note_missing(file, "scenery object", tx, ty, key);
            stats.lock().failed_objects += 1;
            if omsi_cfg::env::var_os("OMSI_DEBUG_MISSING").is_some() {
                log::info!("object not placed: {file} (tile {tx},{ty}, id {key})");
            }
            return None;
        };
        if !ot.sco.is_car_park {
            return Some((ot, false));
        }
        let list = self.parked_car_types(parklist_index(captions));
        if list.is_empty() {
            return Some((ot, false));
        }
        let h = (key as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 33;
        // leave some spaces empty like the original, and all once the options' count of
        // parked cars stands
        let full = self.parked_max < 0 || (self.parked_max > 0 && self.parked_live.load(std::sync::atomic::Ordering::Relaxed) as i64 >= self.parked_max);
        if h % 4 == 0 || full {
            stats.lock().empty_spaces += 1;
            return None;
        }
        let car = &list[(h as usize / 4) % list.len()];
        let Some(base) = self.object_type(car) else {
            self.note_missing(car, "parked car", tx, ty, key);
            return None;
        };
        if base.paint_scheme_count > 0 {
            return self
                .object_type_scheme(car, Some((h as usize / 64) % base.paint_scheme_count))
                .map(|t| (t, true));
        }
        Some((base, true))
    }

    /// The ground as the tile files have it at world x, y (None outside `src`).
    fn base_ground(src: &HashMap<(i32, i32), Arc<StagedTile>>, x: f64, y: f64) -> Option<f64> {
        let k = (
            (x / tile_size()).floor() as i32,
            (y / tile_size()).floor() as i32,
        );
        let st = src.get(&k)?;
        Some(
            st.base_terrain
                .sample((x - st.origin.x) as f32, (y - st.origin.y) as f32) as f64,
        )
    }

    /// Where a staged object stands before the ground is edited: crossings are warped and
    /// the ground deformed from there.
    fn provisional_pose(
        st: &StagedTile,
        o: &StagedObject,
        src: &HashMap<(i32, i32), Arc<StagedTile>>,
    ) -> Option<Pose> {
        match &o.place {
            Placement::Pose(p) => Some(*p),
            Placement::Ground { x, y, z, rot } => {
                let (lx, ly) = (
                    (x - st.origin.x).clamp(0.0, tile_size()) as f32,
                    (y - st.origin.y).clamp(0.0, tile_size()) as f32,
                );
                let base_height = Self::base_ground(src, *x, *y)
                    .unwrap_or_else(|| st.base_terrain.sample(lx, ly) as f64);
                // Omsi.exe sets every object without an absolute height (those are `Pose`s,
                // `SceneryObject::absolute_height`) on the terrain, `[surface]` ones as well
                // (TMap.RefreshObjectsKacheln 0x79e3c8 reads sco+0x194).
                Some(Pose {
                    pos: DVec3::new(*x, *y, z + base_height),
                    rot: object_rotation(*rot),
                })
            }
            Placement::Attached { .. } => None,
        }
    }

    /// The `[maplight]`s of the objects of tile `key` and its eight neighbours, as a light map
    /// bakes them (Omsi.exe 0x7903e0 goes through the tile's near objects, those of the nine
    /// tiles, 0x77fe5c).
    fn light_map_lamps(
        &self,
        key: (i32, i32),
        layout: &TileLayout,
        staged: &HashMap<(i32, i32), Arc<StagedTile>>,
    ) -> Vec<BakeLamp> {
        let mut lamps = Vec::new();
        for q in layout.ring(key) {
            let Some(qs) = staged.get(&q) else { continue };
            if qs.objects.iter().all(|o| o.ot.sco.map_lights.is_empty()) {
                continue;
            }
            let Some(res) = self.resolve(q, &Self::sources(layout, staged, q)) else { continue };
            for (o, pose) in qs.objects.iter().zip(res.poses.iter()) {
                let Some(pose) = pose else { continue };
                for ml in &o.ot.sco.map_lights {
                    let p = pose.rot.transform_point3(glam::Vec3::from(ml.pos)).as_dvec3() + pose.pos;
                    lamps.push(BakeLamp { x: p.x, y: p.y, height: ml.pos[2], color: ml.color, radius: ml.radius });
                }
            }
        }
        lamps
    }

    /// A tile's final ground and final object poses (computed once per staging; `src` are
    /// the staged tiles it depends on).
    fn resolve(
        &self,
        key: (i32, i32),
        src: &HashMap<(i32, i32), Arc<StagedTile>>,
    ) -> Option<Arc<Resolved>> {
        let st = src.get(&key)?;
        Some(
            st.resolved
                .get_or_init(|| Arc::new(self.compute_resolved(st, src)))
                .clone(),
        )
    }

    fn compute_resolved(
        &self,
        st: &StagedTile,
        src: &HashMap<(i32, i32), Arc<StagedTile>>,
    ) -> Resolved {
        let key = (st.tx, st.ty);
        let (terrain, aligned_points, biggest, deformed) = self.final_ground(key, src);
        let warped = self.warp_crossings(st, src);
        let ground_at = |x: f64, y: f64| -> f64 {
            let actual_key = (
                (x / tile_size()).floor() as i32,
                (y / tile_size()).floor() as i32,
            );
            if actual_key == key {
                let lx = (x - st.origin.x).clamp(0.0, tile_size()) as f32;
                let ly = (y - st.origin.y).clamp(0.0, tile_size()) as f32;
                terrain.sample(lx, ly) as f64
            } else {
                // Old maps and converted maps can keep an object in the neighbouring tile's
                // file with local coordinates past the edge. Sample the terrain actually
                // under the object instead of pinning it to this tile's border height.
                Self::base_ground(src, x, y).unwrap_or_else(|| {
                    let lx = (x - st.origin.x).clamp(0.0, tile_size()) as f32;
                    let ly = (y - st.origin.y).clamp(0.0, tile_size()) as f32;
                    terrain.sample(lx, ly) as f64
                })
            }
        };
        // poses of everything that can carry an attachment: objects by id, spline rows by
        // their first object
        let mut poses: HashMap<(i64, usize), (Pose, Arc<ObjectType>)> = HashMap::new();
        let mut final_poses: Vec<Option<Pose>> = st
            .objects
            .iter()
            .map(|o| match &o.place {
                // Omsi.exe places every object, a parking space's car as well, with the pitch
                // and bank of the map file on the terrain height at its position (0x79e3c8
                // .. 0x79e5fb: RotationX(pitch), RotationZ(bank), RotationY(heading), the
                // translation) - it is never leaned to the slope. Leaned by the terrain under
                // it, a car at the kerb of a hill street stood crooked on a road that runs
                // on a different grade from the ground beneath.
                Placement::Ground { x, y, z, rot } => {
                    Some(Pose {
                        pos: DVec3::new(*x, *y, z + ground_at(*x, *y)),
                        rot: object_rotation(*rot),
                    })
                }
                Placement::Pose(p) => Some(*p),
                Placement::Attached { .. } => None,
            })
            .collect();
        for (o, p) in st.objects.iter().zip(&final_poses) {
            if let Some(p) = p {
                if !matches!(o.place, Placement::Pose(_)) || o.map_object {
                    poses
                        .entry((o.id, o.instance))
                        .or_insert((*p, o.ot.clone()));
                }
            }
        }
        for (id, pose, ot) in &st.anchors {
            poses.entry((*id, 0)).or_insert((*pose, ot.clone()));
        }
        // attachments hang on attachments (a whip beam on a lamp post, a traffic light on
        // the beam): resolve them round by round
        loop {
            let mut progress = false;
            for (o, fp) in st.objects.iter().zip(final_poses.iter_mut()) {
                let Placement::Attached { parent, index, rot } = &o.place else {
                    continue;
                };
                if fp.is_some() {
                    continue;
                }
                // (a spline attachment row by its first object: Omsi.exe refuses objects
                // on its later ones)
                let Some((pp, pt)) = poses.get(&(*parent, 0)) else {
                    continue;
                };
                // a point the parent does not have (its object was changed after the map
                // was made: the Ahlheim signal heads on `Arm_3f` point 10 of 1) is the
                // parent's own origin, as in the original - the object is not dropped
                let m = pt
                    .sco
                    .attachments
                    .get(*index)
                    .map(crate::tiles::attachment_matrix)
                    .unwrap_or(glam::Mat4::IDENTITY);
                if *index >= pt.sco.attachments.len() {
                    // `OMSI_NO_ATTACH_FALLBACK=1` drops them instead, for an A/B
                    if omsi_cfg::env::var_os("OMSI_NO_ATTACH_FALLBACK").is_some() {
                        continue;
                    }
                    if omsi_cfg::env::var_os("OMSI_DEBUG_MISSING").is_some() {
                        log::info!("attached object {} (id {}) on point index {index} of {parent} ({} has {}): at the parent's origin ({:.1}, {:.1}, {:.1})", o.ot.sco.path.display(), o.id, pt.sco.path.display(), pt.sco.attachments.len(), pp.pos.x, pp.pos.y, pp.pos.z);
                    }
                }
                let pose = pp.attached(&m, *rot);
                *fp = Some(pose);
                poses.entry((o.id, 0)).or_insert((pose, o.ot.clone()));
                progress = true;
            }
            if !progress {
                break;
            }
        }
        let mut unattached = 0usize;
        for (o, fp) in st.objects.iter().zip(&final_poses) {
            if let (Placement::Attached { parent, .. }, None) = (&o.place, fp) {
                unattached += 1;
                if omsi_cfg::env::var_os("OMSI_DEBUG_MISSING").is_some() {
                    log::info!("attached object {} (id {}) on {parent} not placed: the parent is not in the tile", o.ot.sco.path.display(), o.id);
                }
            }
        }
        Resolved {
            terrain: Arc::new(terrain),
            warped,
            poses: final_poses,
            unattached,
            aligned_points,
            biggest,
            deformed,
        }
    }

    /// The crossings of `st` warped onto the ground: object index → its own meshes.
    ///
    /// A junction plate is one flat object covering a couple of hundred metres, and the
    /// roads running into it do not all lie at its height. `[crossing_heightdeformation]`
    /// names a coarse height field in the plate's own frame: every vertex of the plate is
    /// raised by it, so the plate keeps its kerbs and camber and its arms meet their roads.
    fn warp_crossings(
        &self,
        st: &StagedTile,
        src: &HashMap<(i32, i32), Arc<StagedTile>>,
    ) -> HashMap<usize, Arc<Vec<MeshData>>> {
        let mut out: HashMap<usize, Arc<Vec<MeshData>>> = HashMap::new();
        let plates: Vec<(usize, Pose, &MeshData)> = st
            .objects
            .iter()
            .enumerate()
            .filter_map(|(i, o)| Some((i, Self::provisional_pose(st, o, src)?, o.ot.deform.as_ref()?)))
            .collect();
        if plates.is_empty() {
            return out;
        }
        for (oi, Pose { pos, rot: _ }, base) in plates {
            let ot = &st.objects[oi].ot;
            // Wherever the map puts the plate - a bridge deck twenty metres over the river
            // under it, a dead junction record buried in a field - it is draped by its own
            // field: Omsi.exe lays the meshes onto it in the object's frame as it loads the
            // type (0x7c5934), and the ground under a placement plays no part (nor is it
            // pressed onto the field, see `final_ground`). Left flat more than 12 m from the
            // ground, London Bridge's deck met neither of its roads (#961).
            // The Juliusturm junction's field is a 3.5 % plane, -0.62 m under its west
            // arm and +0.76 m under its east one: adding these offsets to the plate's
            // 35.53 m base puts its arms at the arriving roads' 34.91 and 36.29 m.
            // Deforming the plate to the terrain and nearby roads instead of its own
            // field previously made it sag into a trough with a 0.8 m wall at one end.
            // Only vertices covered by the object's height field are displaced. A
            // vertical ray missing the field leaves the authored height unchanged;
            // extending corner heights beyond it lifts otherwise level road entrances.
            let mut meshes = Vec::with_capacity(ot.meshes.len());
            let mut moved = 0usize;
            let mut biggest = 0f32;
            for (mesh, _, _) in &ot.meshes {
                let mut m = mesh.clone();
                for v in m.positions.iter_mut() {
                    let Some(d) = field_height(base, v.x, v.y) else { continue };
                    if d.abs() > 0.001 {
                        v.z += d;
                        moved += 1;
                        biggest = biggest.max(d.abs());
                    }
                }
                // (the normals of the draped mesh, as Omsi.exe makes them after draping)
                omsi_geometry::compute_normals_d3d(&mut m);
                meshes.push(m);
            }
            if biggest > 1.0 && omsi_cfg::env::var_os("OMSI_DEBUG_WARP").is_some() {
                let (lo, hi) = base.positions.iter().fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p.z), a.1.max(p.z)));
                log::info!("crossing {} at ({:.1}, {:.1}, {:.1}) moved up to {biggest:.2} m (field {lo:.2}..{hi:.2}, {} points)", ot.sco.path.display(), pos.x, pos.y, pos.z, base.positions.len());
            }
            if moved > 0 {
                log::debug!(
                    "crossing {} at ({:.0}, {:.0}) raised by its height field",
                    ot.sco.path.file_name().unwrap_or_default().to_string_lossy(),
                    pos.x,
                    pos.y
                );
                out.insert(oi, Arc::new(meshes));
            }
        }
        out
    }

    /// The ground of tile `key`: the tile's `.terrain` as Omsi.exe loads it, which the objects
    /// stand on. The editor's "align the terrain to this spline" (`[spline_terrain_align]`)
    /// and a crossing's `[crossing_heightdeformation]` were applied when the map was made;
    /// `OMSI_TERRAIN_ALIGN=1` and `OMSI_CROSSING_DEFORM=1` apply them again (A/B runs).
    /// Returns the ground, the ground points aligned, the biggest move (with where it was)
    /// and whether a crossing deformed it.
    fn final_ground(
        &self,
        key: (i32, i32),
        src: &HashMap<(i32, i32), Arc<StagedTile>>,
    ) -> (Terrain, usize, Option<(f32, f64, f64)>, bool) {
        let mut t = src[&key].base_terrain.clone();
        let (x0, y0) = (key.0 as f64 * tile_size(), key.1 as f64 * tile_size());
        let (x1, y1) = (x0 + tile_size(), y0 + tile_size());
        let n = t.samples();
        let cell = tile_size() as f32 / t.cells.max(1) as f32;
        let (mut aligned_points, mut biggest) = (0usize, None);
        // one order whatever the hash map's (the rasters keep the highest surface, but a
        // tie should not depend on it either)
        let mut order: Vec<&Arc<StagedTile>> = src.values().collect();
        order.sort_by_key(|q| (q.tx, q.ty));
        // (Omsi.exe does not move the ground at all when it loads a map: the editor's "align
        // the terrain to the spline" wrote the heights into the tile's `.terrain`, and the
        // flag left in the map only makes the spline cut its outline out of the ground -
        // see `hole_rims`. Pulled onto the road again here, every vertex under it took
        // the height of whatever lay over it, and between those five-metre points the
        // ground's triangles cut through the camber and past the kerbs: a piece of road
        // gone under the grass, while beside it the ground stood lifted over the verge.
        // `OMSI_TERRAIN_ALIGN=1` still does it.)
        if omsi_cfg::env::var_os("OMSI_TERRAIN_ALIGN").is_some() {
            let mut ts = TileSurface::new(SURFACE_RASTER);
            let mut reach = 0.0f32;
            let mut any = false;
            for q in &order {
                for (i, r) in &q.align {
                    let Some(sp) = q.splines.get(*i) else {
                        continue;
                    };
                    let b = &sp.bounds;
                    if b[2] < x0 - 20.0 || b[0] > x1 + 20.0 || b[3] < y0 - 20.0 || b[1] > y1 + 20.0
                    {
                        continue;
                    }
                    ts.rasterize(&sp.shape, &Mat4::IDENTITY, q.origin, key.0, key.1);
                    reach = reach.max(*r);
                    any = true;
                }
            }
            if any {
                // a terrain vertex takes the road's height where the road is under it, and
                // half of the difference one cell further out, so the ground runs into the
                // verge instead of stepping
                let ring = (reach / cell).ceil().clamp(1.0, 3.0) as i32;
                let mut heights: Vec<Option<f32>> = vec![None; n * n];
                for iy in 0..n {
                    for ix in 0..n {
                        if let Some(h) = ts.sample(ix as f32 * cell, iy as f32 * cell) {
                            heights[iy * n + ix] = Some(h);
                        }
                    }
                }
                let mut out = t.heights.clone();
                for iy in 0..n {
                    for ix in 0..n {
                        let k = iy * n + ix;
                        if let Some(h) = heights[k] {
                            let d = (h - t.heights[k]).abs();
                            if biggest.map(|(b, _, _)| d > b).unwrap_or(true) {
                                biggest = Some((
                                    d,
                                    x0 + (ix as f32 * cell) as f64,
                                    y0 + (iy as f32 * cell) as f64,
                                ));
                            }
                            out[k] = h;
                            aligned_points += 1;
                            continue;
                        }
                        // the skirt: blend towards the nearest aligned vertex
                        let mut best: Option<(i32, f32)> = None;
                        for dy in -ring..=ring {
                            for dx in -ring..=ring {
                                let (jx, jy) = (ix as i32 + dx, iy as i32 + dy);
                                if jx < 0 || jy < 0 || jx >= n as i32 || jy >= n as i32 {
                                    continue;
                                }
                                if let Some(h) = heights[jy as usize * n + jx as usize] {
                                    let d = dx.abs().max(dy.abs());
                                    if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                                        best = Some((d, h));
                                    }
                                }
                            }
                        }
                        if let Some((d, h)) = best {
                            let w = 1.0 - d as f32 / (ring as f32 + 1.0);
                            let v = t.heights[k] + (h - t.heights[k]) * w;
                            if (v - t.heights[k]).abs() > 0.01 {
                                aligned_points += 1;
                            }
                            out[k] = v;
                        }
                    }
                }
                t.heights = out;
            }
        }
        // (Nor does it press the ground into a crossing's `[crossing_heightdeformation]` mesh:
        // Omsi.exe reads that mesh only to warp the plate and to give its paths their heights
        // (0x7ba818, "Path deform"); the editor's terrain tools left the ground as the
        // `.terrain` has it. Pressed in here, the ground stood up to 2.3 m over a Spandau
        // pavement in front of the houses beside a junction, and everything standing on the
        // ground - every pole, sign and tree there - floated over the pavement with it (#860).
        // `OMSI_CROSSING_DEFORM=1` still does it.)
        let mut deformed = false;
        if omsi_cfg::env::var_os("OMSI_CROSSING_DEFORM").is_some() {
            let mut ds = TileSurface::new(SURFACE_RASTER);
            let mut any = false;
            for q in &order {
                for o in &q.objects {
                    let Some(d) = &o.ot.deform else { continue };
                    let Some(pose) = Self::provisional_pose(q, o, src) else {
                        continue;
                    };
                    let b = mesh_bounds(d, &pose.rot, pose.pos);
                    if b[2] < x0 || b[0] > x1 || b[3] < y0 || b[1] > y1 {
                        continue;
                    }
                    ds.rasterize(d, &pose.rot, pose.pos, key.0, key.1);
                    any = true;
                }
            }
            if any {
                for iy in 0..n {
                    for ix in 0..n {
                        if let Some(h) = ds.sample(ix as f32 * cell, iy as f32 * cell) {
                            let k = iy * n + ix;
                            deformed |= (t.heights[k] - h).abs() > 0.01;
                            t.heights[k] = h;
                        }
                    }
                }
            }
        }
        (t, aligned_points, biggest, deformed)
    }

    /// Stand the objects of a staged tile on its final ground, hang the attached ones on
    /// their parents, and register what the traffic, the passengers, the collisions and the
    /// lights need from them.
    fn place_tile(
        &self,
        key: (i32, i32),
        staged: &HashMap<(i32, i32), Arc<StagedTile>>,
        layout: &TileLayout,
        stats: &Mutex<LoadStats>,
    ) -> Option<Prepared> {
        let src = Self::sources(layout, staged, key);
        let st = src.get(&key)?.clone();
        let res = self.resolve(key, &src)?;
        let (tx, ty) = key;
        let first_load = self.seeded.lock().insert(key);
        let terrain: &Terrain = &res.terrain;
        self.terrains.write().insert(key, res.terrain.clone());
        let ground_at = |x: f64, y: f64| -> f64 {
            let lx = (x - st.origin.x).clamp(0.0, tile_size()) as f32;
            let ly = (y - st.origin.y).clamp(0.0, tile_size()) as f32;
            terrain.sample(lx, ly) as f64
        };
        let mut state = TileState::default();
        let mut lanes: Vec<Lane> = if first_load {
            std::mem::take(&mut *st.lanes.lock())
        } else {
            Vec::new()
        };
        let mut parked_cars: Vec<(DVec3, f64)> = Vec::new();
        let mut objects: Vec<PlacedObject> = Vec::new();
        let mut trees = Vec::new();
        let debug_objects = omsi_cfg::env::var_os("OMSI_DEBUG_OBJECTS").is_some();
        let check_objects = omsi_cfg::env::var_os("OMSI_CHECK_OBJECTS").is_some();
        let debug_float = omsi_cfg::env::var_os("OMSI_DEBUG_FLOAT").is_some();
        let index = self.index();
        for (oi, (o, fp)) in st.objects.iter().zip(res.poses.iter()).enumerate() {
            let Some(Pose { pos, rot: xf }) = *fp else {
                continue;
            };
            if let (true, Placement::Ground { x, y, .. }) = (debug_float, &o.place) {
                // how far the object stood off the ground when it was placed before the
                // roads and crossings had pulled the ground about
                let (lx, ly) = (
                    (x - st.origin.x).clamp(0.0, tile_size()) as f32,
                    (y - st.origin.y).clamp(0.0, tile_size()) as f32,
                );
                let moved = terrain.sample(lx, ly) - st.base_terrain.sample(lx, ly);
                if moved.abs() > 0.3 {
                    log::info!("float: {} id {} at ({:.1}, {:.1}): the ground under it moved {:+.2} m (it stood {:.2} m {} before)", o.ot.sco.path.display(), o.id, x, y, moved, moved.abs(), if moved < 0.0 { "in the air" } else { "in the ground" });
                }
            }
            let ot = o.ot.clone();
            let heading = Pose { pos, rot: xf }.heading();
            // (a spline attachment row's first object stands for the row: an entry point or a
            // stop put on a road is found by its id)
            if o.map_object || o.instance == 0 {
                self.object_positions
                    .lock()
                    .insert(o.id, (pos, [heading, 0.0, 0.0]));
                let mut dups = self.object_dups.lock();
                if let Some(d) = dups.get_mut(&(key, o.id)) {
                    *d = (pos, [heading, 0.0, 0.0]);
                }
            }
            if o.parked {
                state.parked_count += 1;
                self.parked_live.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            if o.parked && first_load {
                // on the ground: the lane match of the traffic is by distance in 3D
                parked_cars.push((pos, heading));
            }
            if ot.sco.is_bus_stop {
                state.bus_stops.push((
                    o.id,
                    pos,
                    heading,
                    o.extra.first().cloned().unwrap_or_default(),
                ));
            }
            if let Some(rel) = &ot.sco.passenger_cabin {
                // waiting places: an object's `[passpos]` in its own frame (the editor-only
                // markers have no mesh, so this comes before that test)
                let dir = ot
                    .sco
                    .path
                    .parent()
                    .map(|d| d.to_path_buf())
                    .unwrap_or_default();
                if let Some(cabin) = self.waiting_cabin(&omsi_cfg::resolve_path(&dir, rel)) {
                    for pp in &cabin.pass_positions {
                        let p = pos + xf.transform_vector3(glam::Vec3::from(pp.pos)).as_dvec3();
                        state
                            .waiting_places
                            .push((o.id, p, heading + pp.rot as f64, pp.height));
                    }
                }
            }
            if let Some((tex, min_h, max_h, min_r, max_r)) = &ot.sco.tree {
                // Trees are billboards: the map stores texture, height and the ratio of the
                // width to the height chosen by the editor; a row of trees along a spline
                // takes the middle of the type's ranges. OMSI scales its tree by (height x
                // ratio, height, height x ratio) (Omsi.exe 0x77e6b0 fills the record,
                // 0x774444 builds the matrix): the ratio multiplies. Divided by it, as here
                // before, a slim tree (0.4 on Spandau) came out six times too wide and its
                // crown stood metres away from its trunk's place.
                let texture = o
                    .extra
                    .first()
                    .cloned()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| tex.clone());
                let mid_h = ((min_h + max_h) * 0.5) as f64;
                let mid_r = ((min_r + max_r) * 0.5) as f64;
                let height = o
                    .extra
                    .get(1)
                    .map(|s| omsi_cfg::parse_f64(s))
                    .filter(|h| *h > 0.0)
                    .unwrap_or(if mid_h > 0.0 { mid_h } else { 10.0 });
                let ratio = o
                    .extra
                    .get(2)
                    .map(|s| omsi_cfg::parse_f64(s))
                    .filter(|r| *r > 0.0)
                    .unwrap_or(if mid_r > 0.0 { mid_r } else { 1.0 });
                trees.push((ot.clone(), texture, pos, height, height * ratio, heading));
                continue;
            }
            // Stock junctions carry a light program even where the map places no signals.
            // Use the map-wide index so lamps on an unloaded neighbouring tile still count.
            let controller = if !traffic_light_program_enabled(&ot.sco, index.traffic_light_parents.contains(&o.id)) {
                None
            } else {
                let known = self.controller_of_object.lock().get(&o.id).copied();
                Some(known.unwrap_or_else(|| {
                    let program = ot.sco.traffic_lights.iter().map(|l| (l.phases.iter().map(|p| (p.state, p.duration)).collect(), l.approach_dist)).collect();
                    let c = TrafficLightController::from_program(program, ot.sco.traffic_lights_group, &ot.sco.traffic_light_stop, &ot.sco.traffic_light_jump);
                    let mut list = self.traffic_lights.lock();
                    list.push(c);
                    let idx = list.len() - 1;
                    self.controller_of_object.lock().insert(o.id, idx);
                    if omsi_cfg::env::var_os("OMSI_DEBUG_TRAFFIC").is_some() {
                        log::info!("traffic light program {idx}: object {} {} at ({:.1}, {:.1}) cycle {:?} lights {}", o.id, ot.sco.path.display(), pos.x, pos.y, ot.sco.traffic_lights_group, ot.sco.traffic_lights.len());
                    }
                    idx
                }))
            };
            // (a `[helparrow]` object goes on: it is put up hidden, and drawn while the route
            // arrows are on - see `World::show_help_arrows`)
            if ot.sco.only_editor || ot.meshes.is_empty() {
                // invisible sound sources (ambient sound objects) still run their script
                if let (Some(program), true) = (&ot.program, ot.sco.sound.is_some()) {
                    let inst = omsi_sim::scenery::SceneryInstance::new(
                        program.clone(),
                        &ot.mesh_defs(),
                        self.script_clock(),
                        &o.extra,
                    );
                    self.scripted.lock().push(ScriptedObject {
                        ty: ot.clone(),
                        pos,
                        xf,
                        instances: Vec::new(),
                        inst,
                        controller: None,
                        light_index: 0,
                        light_parent: None,
                        map_id: o.id,
                        variants: Vec::new(),
                        sounds: None,
                        tile: key,
                        var_parent: o.lamp_parent,
                        texts: Vec::new(),
                        arrivals: false,
                        htmls: Vec::new(),
                        alpha_slots: Vec::new(),
                        alpha_last: Vec::new(),
                    });
                }
                // An editor-only object still lays its paths out: OMSI's invisible
                // crossings (Novi Sad's junctions are 52-path `[onlyeditor]` objects, their
                // light program driving the lamps placed round them) are where its traffic
                // and its timetable buses turn - skipped, the junctions were holes in the
                // road network and every bus route through one jumped across it.
                if first_load && !ot.sco.paths.is_empty() {
                    lanes.extend(object_lanes(
                        &ot.sco,
                        pos,
                        [heading, 0.0, 0.0],
                        controller,
                        key,
                        o.id,
                        &o.rules,
                    ));
                }
                continue;
            }
            let is_surface = ot.sco.render_type.is_ground_layer()
                || ot.sco.surface;
            if check_objects && is_surface {
                let over = pos.z - ground_at(pos.x, pos.y);
                if !(-1.0..=3.0).contains(&over) {
                    log::info!("surface object {:+.1} m from the ground at ({:.0}, {:.0}): {} (stored as {})", over, pos.x, pos.y, ot.sco.path.display(), match o.place { Placement::Ground { .. } => "height above the terrain", Placement::Pose(_) => "absolute height", Placement::Attached { .. } => "attachment" });
                }
            }
            if first_load {
                let mut own = object_lanes(
                    &ot.sco,
                    pos,
                    [heading, 0.0, 0.0],
                    controller,
                    key,
                    o.id,
                    &o.rules,
                );
                // An object tilted on a slope (the map's pitch and bank) tilts its paths with
                // it, as the whole object matrix places them in Omsi.exe: laid out by the
                // heading alone, a junction on a hill had flat lanes through a sloping plate
                // and its traffic drove into the road on one side and over it on the other.
                let yaw = omsi_geometry::object_rotation([heading, 0.0, 0.0]);
                let tilt = xf * yaw.inverse();
                if !tilt.abs_diff_eq(Mat4::IDENTITY, 1e-5) {
                    for l in own.iter_mut() {
                        for q in l.points.iter_mut() {
                            *q = pos + tilt.transform_point3((*q - pos).as_vec3()).as_dvec3();
                        }
                        l.refresh();
                    }
                }
                // Paths sample the field independently of the visual mesh: a coarse
                // mesh can have no covered vertices while lane points lie inside it.
                if let Some(field) = ot.deform.as_ref() {
                    let inv = xf.inverse();
                    for l in own.iter_mut() {
                        for q in l.points.iter_mut() {
                            let local = inv.transform_point3((*q - pos).as_vec3());
                            if let Some(d) = field_height(field, local.x, local.y) {
                                q.z += d as f64;
                            }
                        }
                    }
                }
                lanes.extend(own);
            }
            // What vehicles hit, as OMSI gives it to ODE: the `[collision_mesh]` as a
            // triangle mesh when there is one (it wins over a `[boundingbox]`), else the
            // `[boundingbox]` as a box. A visual mesh without either declaration is not a
            // collision shape, and neither are the extents of a collision mesh: one box
            // around a housing estate's mesh or the Heerstraße bridge stood as an invisible
            // wall across the roads through and under it.
            // Only a `[fixed]` object (or a `[crashmode_pole]`) is solid for the vehicles, as
            // Omsi.exe sets it up (0x7af0a4: the shape is made for those only; any other is a
            // loose body the bus is not stopped by). We made every object with a shape solid:
            // the line plates and name signs hanging off bus stop poles, and any bridge or
            // gantry of a mod map not marked `[fixed]` - an invisible wall under it.
            // (a parked car is a vehicle: it is hit as the traffic is)
            let solid = ot.sco.fixed || ot.sco.crash_mode_pole.is_some() || o.parked;
            // (Not a `[surface]` object, although Omsi.exe makes it `[fixed]` and puts its
            // collision mesh into the tile's static ODE space like any other (0x7af0a4, the
            // vehicle collides with that space in 0x6ff5b8): the Spandau depot's
            // `Betr_S_Bauten` has fence rails 1.9 m up across its yard's drive paths, which
            // the original's buses pass through - something drops those contacts that is not
            // found yet, and made solid here they walled in the whole yard.)
            let mesh_shape = ot
                .collision
                .as_ref()
                .filter(|_| solid && !ot.sco.no_collision && !is_surface && !ot.meshes.is_empty());
            if let Some(c) = mesh_shape {
                let tris = |m: &dyn Fn(glam::Vec3) -> glam::DVec3| -> Vec<[glam::DVec3; 3]> {
                    c.indices
                        .chunks_exact(3)
                        .map(|t| [m(c.positions[t[0] as usize]), m(c.positions[t[1] as usize]), m(c.positions[t[2] as usize])])
                        .collect()
                };
                // the shape is the type's, in its own frame; an object tilted on a slope
                // (rare) gets one of its own, turned by all but its heading
                let yaw = omsi_geometry::object_rotation([heading, 0.0, 0.0]);
                let tilt = yaw.inverse() * xf;
                let upright = (tilt.x_axis.truncate() - glam::Vec3::X).length() < 1e-3
                    && (tilt.y_axis.truncate() - glam::Vec3::Y).length() < 1e-3;
                let shape = if upright {
                    ot.collision_shape
                        .get_or_init(|| {
                            Arc::new(omsi_sim::collision::MeshShape::from_triangles(
                                tris(&|p| p.as_dvec3()).into_iter(),
                                LOW_OBJECT as f64,
                            ))
                        })
                        .clone()
                } else {
                    Arc::new(omsi_sim::collision::MeshShape::from_triangles(
                        tris(&|p| tilt.transform_vector3(p).as_dvec3()).into_iter(),
                        LOW_OBJECT as f64,
                    ))
                };
                if !shape.parts.is_empty() {
                    if omsi_cfg::env::var_os("OMSI_DEBUG_COLLISION").is_some() {
                        log::info!("obstacle {} key {} at ({:.1}, {:.1}) z {:.1} rot {:.0}: collision mesh of {} triangles as {} parts{}", ot.sco.path.display(), o.key, pos.x, pos.y, pos.z, heading, c.indices.len() / 3, shape.parts.len(), if upright { "" } else { " (tilted)" });
                    }
                    state
                        .mesh_obstacles
                        .push(omsi_sim::collision::MeshObstacle::new(shape, pos, heading, o.key));
                }
            } else if solid && !ot.sco.no_collision && !is_surface && !ot.meshes.is_empty() {
                if let Some(bb) = ot.sco.bounding_box {
                    // Ignore flat decals and oversized helpers - and anything whose top stays
                    // under a bus floor: a manhole cover's half-metre box centred on the road
                    // (ViewApp's Kanaldeckel) stands 25 cm proud of the asphalt, and a
                    // pitching bus ran into it as into a wall.
                    let top = bb[5] + bb[2] * 0.5;
                    // a road that runs through the box (under a bridge, a gantry, an arch,
                    // a station hall) says it is no wall there: a mod map's big objects give
                    // their whole extent as the `[boundingbox]`, and the bus met an invisible
                    // wall across the street (Grand Paris Moulon, Saint Servant)
                    let probe = omsi_sim::collision::Obb::from_box(bb, pos, heading);
                    let road_through = !o.parked && (bb[0] > 3.0 || bb[1] > 3.0) && {
                        let [r, f] = probe.axes();
                        // a street lane through its footprint, at a height a vehicle on it
                        // would be inside the box (not a road on its roof or far below)
                        st.street_points.iter().any(|w| {
                            let d = w.truncate() - probe.center;
                            d.dot(r).abs() <= probe.half.x && d.dot(f).abs() <= probe.half.y && w.z >= probe.z0 - 1.0 && w.z <= probe.z1 - 0.5
                        })
                    };
                    if road_through && omsi_cfg::env::var_os("OMSI_DEBUG_COLLISION").is_some() {
                        log::info!("no wall: {} key {} - a road runs through its [boundingbox]", ot.sco.path.display(), o.key);
                    }
                    if bb[2] > 0.4
                        && top > LOW_OBJECT
                        && bb[0] < 400.0
                        && bb[1] < 400.0
                        && bb[0] > 0.05
                        && bb[1] > 0.05
                        && !road_through
                    {
                        let mut obb = omsi_sim::collision::Obb::from_box(bb, pos, heading);
                        obb.pole = ot.sco.crash_mode_pole;
                        obb.id = o.key;
                        // (a car that has driven off leaves its space free)
                        let gone = o.parked && self.departed.lock().contains(&o.key);
                        if !gone {
                            state.obstacles.push(obb);
                        }
                        if o.parked && !gone {
                            state.parked_boxes.push(obb);
                        }
                        if omsi_cfg::env::var_os("OMSI_DEBUG_COLLISION").is_some() {
                            log::info!("obstacle {} key {} at ({:.1}, {:.1}) z {:.1}..{:.1} size {:.1}x{:.1}x{:.1} centre offset ({:.1}, {:.1}) rot {:.0}{}{}", ot.sco.path.display(), o.key, pos.x, pos.y, obb.z0, obb.z1, bb[0], bb[1], bb[2], bb[3], bb[4], heading, if obb.pole.is_some() { " pole" } else { "" }, " [boundingbox]");
                        }
                    }
                }
            }
            if !ot.model.smokes.is_empty() || !ot.model.particle_emitters.is_empty() {
                let set = omsi_sim::particles::ParticleSet::new(ot.model.particle_systems(), (o.id as u64) ^ 0x51ed_2701);
                self.particle_objects.lock().entry(key).or_default().push(ParticleObject { map_id: o.id, pos, rot: xf, set });
            }
            for tb in &ot.sco.trigger_boxes {
                if let Some((time, fade)) = tb.reverb {
                    let bb = [tb.size[0], tb.size[1], tb.size[2], tb.center[0], tb.center[1], tb.center[2]];
                    state.reverb_zones.push((omsi_sim::collision::Obb::from_box(bb, pos, heading), time, fade));
                }
            }
            if ot.sco.is_petrol_station {
                if let Some(bb) = ot.sco.bounding_box.or_else(|| ot.local_box()) {
                    state.petrol_stations.push(omsi_sim::collision::Obb::from_box(bb, pos, heading));
                }
            }
            // what the outside camera cannot pass through: houses, walls, shelters, canopies
            // (surface objects too - a petrol station is a drivable [surface] with a roof)
            if let Some(shape) = ot.camera_shape() {
                state.blockers.push(crate::camera_arm::Blocker {
                    ty: Arc::downgrade(&ot),
                    pos,
                    xf,
                    radius: shape.radius(),
                });
            }
            // (OMSI hands `TrafficLightPhase` to any child of a crossing whose first string
            // names one of its lights, `[trafficlight]` or not - see `names_traffic_light`;
            // a mod lamp without the keyword sat at its "off" picture, blinking yellow.
            // Objects with textures of their own to choose stay ordinary objects.)
            let child_lamp = o.lamp_parent.is_some_and(|p| index.traffic_light_parents.contains(&p))
                && crate::tiles::names_traffic_light(&o.extra)
                && ot.dynamic_textures.is_empty()
                && !ot.meshes.iter().any(|(_, _, ov)| ov.iter().any(|m| !m.item && m.freetex.is_some()));
            let lamp = if ot.sco.is_traffic_light || child_lamp {
                let named = o.extra.first().map(|s| s.trim()).filter(|s| !s.is_empty());
                let index = named.map(|s| omsi_cfg::parse_f64(s) as usize).unwrap_or(0);
                if omsi_cfg::env::var_os("OMSI_DEBUG_LAMPS").is_some() {
                    match o.lamp_parent {
                        None => log::info!("traffic light {} (id {}) names no crossing ([varparent]); extra {:?}", ot.sco.path.display(), o.id, o.extra),
                        Some(p) => log::info!("traffic light {} (id {}) at ({:.0}, {:.0}): crossing {p}, light {:?}", ot.sco.path.display(), o.id, pos.x, pos.y, o.extra),
                    }
                }
                // (a signal that names no crossing - Korean maps fix pedestrian heads to a
                // road spline without a [varparent] - is a lamp all the same: its lenses
                // follow [visible]/[alphascale] on the dummy phase every unlinked object
                // reads, see `UNLINKED_PHASE`; drawn as plain scenery, the red and the green
                // man were both lit all the time, #988)
                Some((o.lamp_parent.unwrap_or(NO_CROSSING), index, named.is_none()))
            } else {
                None
            };
            // lights of the placed object
            {
                let switches: Mutex<Vec<LightSwitch>> = Mutex::new(Vec::new());
                // (a light gives several sprites: each takes its own light's switch)
                let coronas = model_lights_owned(&ot.model, &|_| xf, pos, &|var| {
                    switches.lock().push(LightSwitch::parse(var));
                    1.0
                }, &[]);
                let switches = switches.into_inner();
                for (c, sw) in coronas.into_iter().filter_map(|(c, k)| switches.get(k).cloned().map(|sw| (c, sw))) {
                    // a traffic lamp's red, yellow and green glow with its state
                    // (`LightObject::coronas`), not all at once by night
                    if lamp.is_some() && matches!(sw, LightSwitch::Variable(_)) {
                        continue;
                    }
                    state.coronas.push(StaticCorona {
                        corona: c,
                        switch: sw,
                    });
                }
                for (k, ml) in ot.sco.map_lights.iter().enumerate() {
                    if ot.sco.map_lights[..k].iter().any(|o| o.pos == ml.pos && o.color == ml.color && o.radius == ml.radius) {
                        continue;
                    }
                    let p = xf.transform_point3(glam::Vec3::from(ml.pos)).as_dvec3() + pos;
                    // `[maplight] … radius` is the core the light fills at full colour; it
                    // fades inverse-square beyond and is cut off at six times that. The
                    // colour is the brightness, so the intensity stays at one: an Esso sign
                    // declared as 0.1 red is a glow by its pumps, not a red wash over the
                    // whole street.
                    state.lights.push(omsi_render::PointLight {
                        position: p,
                        radius: ml.radius.max(0.5) * 6.0,
                        color: ml.color,
                        intensity: 1.0,
                        core: ml.radius.max(0.5),
                        housed: true,
                        ..Default::default()
                    });
                }
            }
            if debug_objects {
                let kind = match (&o.place, o.map_object) {
                    (Placement::Attached { .. }, _) => "attachObj",
                    (_, true) => "object",
                    (Placement::Pose(_), false) => "spline row",
                    (Placement::Ground { .. }, false) => "object",
                };
                log::info!(
                    "object {} id {} at ({:.1}, {:.1}, {:.1}) rot {:.1} tile ({tx}, {ty}) {kind}",
                    ot.sco.path.display(),
                    o.id,
                    pos.x,
                    pos.y,
                    pos.z,
                    heading
                );
            }
            objects.push(PlacedObject {
                ot,
                pos,
                xf,
                lamp,
                map_id: o.id,
                key: o.key,
                controller,
                strings: o.extra.clone(),
                warped: res.warped.get(&oi).cloned(),
                var_parent: o.lamp_parent,
                parked: o.parked,
                editable: o.map_object && matches!(o.place, Placement::Ground { .. }),
                script: None,
            });
        }
        if first_load {
            // together, under the lanes lock (see `World::lane_tiles`)
            let mut all = self.lanes.lock();
            all.extend(lanes);
            self.parked_cars.lock().extend(parked_cars);
            self.lane_tiles.lock().push(key);
        }
        {
            let mut s = stats.lock();
            s.failed_objects += st.counts.failed_objects;
            s.empty_spaces += st.counts.empty_spaces;
            s.rows += st.counts.rows;
            s.attached += st.counts.attached;
            s.unattached += res.unattached;
            s.objects_placed += objects.len();
            s.ground_aligned += res.aligned_points;
            s.ground_aligned_tiles += (res.aligned_points > 0) as usize;
            s.ground_deformed_tiles += res.deformed as usize;
            s.crossings_warped += res.warped.len();
            if let Some(b) = res.biggest {
                if s.ground_moved_most.map(|m| b.0 > m.0).unwrap_or(true) {
                    s.ground_moved_most = Some(b);
                }
            }
        }
        self.tile_state.lock().insert(key, state);
        // the tile's night light map (lamp light pools on the ground). Omsi.exe bakes a
        // `[variable_terrainlightmap]` tile's own from the lamps of the tiles round it once
        // those are loaded (0x780694, unless `[no_generateTerrLightMaps]`) and writes it over
        // the `.map.LM.bmp`: the file is only what the map's last OMSI run left there, if
        // anything. (Novi Sad's light maps are older than its tiles: read from them, a road
        // lay dark under the lamps put up along it since, #951.)
        let file_light_map = || {
            self.global.tiles.iter().find(|t| t.x == tx && t.y == ty).and_then(|t| {
                let p = omsi_cfg::resolve_path(&self.map_dir, &format!("{}.LM.bmp", t.file));
                if omsi_cfg::vfs::is_file(&p) {
                    omsi_texture::decode_file(&p).ok()
                } else {
                    None
                }
            })
        };
        let light_map = if st.bakes_light_map {
            Some(bake_light_map(&self.light_map_lamps(key, layout, staged), st.origin))
        } else {
            file_light_map()
        };
        let light_map = light_map.map(|img| own_tile_of_light_map(&img));
        // kept for the light map atlas of the splines and [LightMapMapping] objects
        match &light_map {
            Some(img) => {
                self.light_maps.lock().insert(key, Arc::new(img.clone()));
            }
            None => {
                self.light_maps.lock().remove(&key);
            }
        }
        self.light_maps_generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // The lamps' light takes the colour the map's light map gives the ground there: OMSI
        // lights the ground from that map only, and a map's author paints the sodium lamps'
        // orange into it, while the lamp objects' own [maplight] is often a generic white.
        // Lit by that white, the roads and houses under an orange pool of light turned white.
        if let Some(img) = light_map.as_ref() {
            if let Some(state) = self.tile_state.lock().get_mut(&key) {
                tint_lights_from_light_map(&mut state.lights, img, st.origin);
            }
        }
        // the whole spline meshes go to the GPU from here (a later load reads the tile again)
        let meshes = st.meshes.lock().take().unwrap_or_default();
        let splines: Vec<_> = meshes
            .into_iter()
            .zip(st.splines.iter())
            .map(|(m, sp)| (m, sp.ty.clone(), sp.casts_shadow, sp.sort_origin))
            .collect();
        let (splines, ground_splines) = if omsi_cfg::env::var_os("OMSI_NO_GROUND_SPLINE_BATCHING").is_some()
            || omsi_cfg::env::var_os("OMSI_NO_SPLINE_BATCHING").is_some()
        {
            (splines, Vec::new())
        } else {
            let mut slots_by_type = HashMap::new();
            let mut rest = Vec::new();
            let mut ground = Vec::new();
            for (mesh, ty, casts, sort_origin) in splines {
                let slots: &Vec<usize> = slots_by_type.entry(Arc::as_ptr(&ty) as usize).or_insert_with(|| {
                    let dirs = texture_dirs(&self.root, &ty.dir);
                    let dirs: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
                    ty.def.textures.iter().enumerate()
                        .filter(|(_, t)| self.textures.cfg(&t.file, &dirs).terrain_mapping)
                        .map(|(i, _)| i).collect()
                });
                if slots.is_empty() {
                    rest.push((mesh, ty, casts, sort_origin));
                    continue;
                }
                let faces = terrain_ground(&mesh, slots, st.origin, Mat4::IDENTITY, st.origin);
                if !faces.is_empty() { ground.push(Arc::new(faces)); }
                let mesh = terrain_rest(&mesh, slots);
                if !mesh.ranges.is_empty() { rest.push((Arc::new(mesh), ty, casts, sort_origin)); }
            }
            (rest, batch_ground_splines(ground))
        };
        let splines = batch_static_splines(splines);
        Some(Prepared {
            tx,
            ty,
            terrain: Some(build_terrain_mesh(terrain)),
            hole_walls: MeshData::default(),
            paint_masks: self.load_ground_paint(&st.path),
            paint: Vec::new(),
            wall_paint: Vec::new(),
            water: st.water,
            splines,
            ground_splines,
            objects,
            trees,
            origin: st.origin,
            light_map: light_map.map(|i| tile_texture(i, false)),
            cut: None,
            images: Arc::new(HashMap::new()),
        })
    }

    /// Surface rasters: cut the terrain under roads and crossings, keep their heights (for
    /// the wheels and the feet), and decode the textures the upload will want. The roads that
    /// reach a tile and the surfaces and cutters of the tiles next to it count as much as its
    /// own, each as it finally stands.
    fn cut_terrain(
        &self,
        prepared: &mut [Prepared],
        staged: &HashMap<(i32, i32), Arc<StagedTile>>,
        layout: &TileLayout,
    ) {
        let debug_raster = omsi_cfg::env::var("OMSI_DEBUG_RASTER").ok().and_then(|q| {
            q.split_once(',')
                .and_then(|(a, b)| Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?)))
        });
        let check_roads = omsi_cfg::env::var_os("OMSI_CHECK_ROADS").is_some();
        let debug = omsi_cfg::env::var_os("OMSI_DEBUG_SPLINES").is_some();
        type Check = (usize, usize, Vec<(f64, f64, f32)>);
        let debug_physics = omsi_cfg::env::var_os("OMSI_DEBUG_PHYSICS").is_some();
        let results: Vec<(Arc<TileSurface>, Option<Image>, Check, usize)> = prepared
            .par_iter_mut()
            .map(|p| {
                let key = (p.tx, p.ty);
                let (tx, ty) = key;
                let (x0, y0) = (tx as f64 * tile_size(), ty as f64 * tile_size());
                let (x1, y1) = (x0 + tile_size(), y0 + tile_size());
                let outside = |b: &[f64; 4]| b[2] < x0 || b[0] > x1 || b[3] < y0 || b[1] > y1;
                let src = Self::sources(layout, staged, key);
                let mut order: Vec<&Arc<StagedTile>> = src.values().collect();
                order.sort_by_key(|q| (q.tx, q.ty));
                let mut ts = TileSurface::new(SURFACE_RASTER);
                let mut hole_rims = Vec::new();
                // meshes the wheels stand on, and of them low objects they climb
                let mut wheel_meshes = 0usize;
                let report = |mesh: &MeshData,
                              xf: &Mat4,
                              o: DVec3,
                              b: &[f64; 4],
                              label: &dyn Fn() -> String| {
                    let Some((qx, qy)) = debug_raster else { return };
                    if qx < b[0] || qx > b[2] || qy < b[1] || qy > b[3] {
                        return;
                    }
                    let inside = mesh.indices.chunks_exact(3).any(|t| {
                        let w = |i: u32| {
                            let v = xf.transform_point3(mesh.positions[i as usize]).as_dvec3() + o;
                            (v.x, v.y)
                        };
                        let (a, bb, c) = (w(t[0]), w(t[1]), w(t[2]));
                        let s1 = (bb.0 - a.0) * (qy - a.1) - (bb.1 - a.1) * (qx - a.0);
                        let s2 = (c.0 - bb.0) * (qy - bb.1) - (c.1 - bb.1) * (qx - bb.0);
                        let s3 = (a.0 - c.0) * (qy - c.1) - (a.1 - c.1) * (qx - c.0);
                        (s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0)
                            || (s1 <= 0.0 && s2 <= 0.0 && s3 <= 0.0)
                    });
                    if inside {
                        log::info!("point ({qx},{qy}) covered by {} (bounds {:?})", label(), b);
                    }
                };
                // every road that reaches the tile; a railway embankment or a bridge deck is
                // a surface (the ground is cut under it) but not something the wheels stand
                // on: only splines that carry a road or footway path count as drivable
                for q in &order {
                    for sp in &q.splines {
                        // (a blended layer - Westcountry's lane darkeners over the painted
                        // ground of its junctions - cuts no ground away: under it the ground
                        // is what shows through, and cut away it was the sky)
                        // (nor do wires overhead: see `SPLINE_OVERHEAD`)
                        if !sp.cuts_terrain || sp.overlay || outside(&sp.bounds) {
                            continue;
                        }
                        report(&sp.shape, &Mat4::IDENTITY, q.origin, &sp.bounds, &|| {
                            format!("spline {}", sp.ty.def.path.display())
                        });
                        ts.rasterize_kind(
                            &sp.shape,
                            &Mat4::IDENTITY,
                            q.origin,
                            tx,
                            ty,
                            sp.drivable,
                        );
                    }
                    // what the wheels roll on: the splines' height profiles
                    for (hp, b, surf) in &q.drive {
                        if !outside(b) {
                            ts.add_spline_drive(
                                hp,
                                surf.as_ref(),
                                q.origin,
                                tx,
                                ty,
                            );
                            wheel_meshes += 1;
                        }
                    }
                }
                // the surfaces and [terrainhole] cutters of this tile and the ones around it
                for q in &order {
                    if (q.tx - tx).abs() > 1 || (q.ty - ty).abs() > 1 {
                        continue;
                    }
                    let Some(res) =
                        self.resolve((q.tx, q.ty), &Self::sources(layout, staged, (q.tx, q.ty)))
                    else {
                        continue;
                    };
                    if omsi_cfg::env::var_os("OMSI_NO_SPLINE_HOLES").is_none() {
                        for rim in &q.hole_rims {
                            let ring: Vec<_> = rim.iter().map(|v| v.truncate()).collect();
                            ts.add_outline(&ring, tx, ty);
                            hole_rims.push(rim.iter().map(|v| *v - p.origin).collect());
                        }
                    }
                    for (oi, (o, pose)) in q.objects.iter().zip(res.poses.iter()).enumerate() {
                        let Some(pose) = pose else { continue };
                        let ot = &o.ot;
                        // Editor-only helpers and trees do not cut terrain.
                        if ot.sco.tree.is_some()
                            || ot.sco.only_editor
                            || ot.sco.is_help_arrow
                        {
                            continue;
                        }
                        for h in &ot.holes {
                            if !outside(&mesh_bounds(h, &pose.rot, pose.pos)) {
                                ts.rasterize_hole(h, &pose.rot, pose.pos, tx, ty);
                                // and cut exactly along its rim, as along a spline's outline:
                                // by texel alone the ground stood a metre into the road at
                                // the edges of a junction (Spandau, Bahnstr./Hansastr.)
                                for rim in omsi_geometry::hole_mesh_rims(h, &pose.rot, pose.pos) {
                                    let ring: Vec<_> = rim.iter().map(|v| v.truncate()).collect();
                                    if !omsi_geometry::outline_crosses_itself(&ring) {
                                        ts.add_outline(&ring, tx, ty);
                                        hole_rims.push(rim.iter().map(|v| *v - p.origin).collect());
                                    }
                                }
                            }
                        }
                        // An explicit cutter is independent of the object's render meshes.
                        if ot.meshes.is_empty() {
                            continue;
                        }
                        // Laid on the ground (the terrain is cut under it): a `[surface]` object
                        // and one drawn as a ground layer (`[rendertype]`).
                        let surface =
                            ot.sco.render_type.is_ground_layer()
                                || ot.sco.surface;
                        if !surface {
                            continue;
                        }
                        let meshes: Vec<&MeshData> = match res.warped.get(&oi) {
                            Some(w) => w.iter().collect(),
                            None => ot.meshes.iter().map(|(m, _, _)| m).collect(),
                        };
                        // What the wheels stand on is Omsi.exe's ground query (0x7a0814): the
                        // terrain, the splines, and of the objects only the `[surface]` ones
                        // (the tile's list of them, 0x79eb63) - and of those only the first
                        // `[mesh]` of the model, a ray cast down into it (0x5f9218 with only
                        // mesh 0). A collision mesh is never ground (it only shapes the crash
                        // body), nor is an object drawn as a ground layer without `[surface]`
                        // (the road markings), nor are the other meshes of a surface object
                        // (the Spandau depot's buildings stand on its yard, `Betr_S_Boden`,
                        // its first mesh). Every one of those lifted the wheels here: the bus
                        // hopped over markings, low collision meshes and whatever a surface
                        // object carried - bumps nobody could see.
                        let ground_mesh = ot.sco.surface.then(|| ot.mesh_def_index.iter().position(|&d| d == 0)).flatten();
                        for (k, mesh) in meshes.into_iter().enumerate() {
                            let b = mesh_bounds(mesh, &pose.rot, pose.pos);
                            if outside(&b) {
                                continue;
                            }
                            report(mesh, &pose.rot, pose.pos, &b, &|| {
                                format!(
                                    "object {} rendertype={:?} surface={}",
                                    ot.sco.path.display(),
                                    ot.sco.render_type,
                                    ot.sco.surface
                                )
                            });
                            ts.rasterize_kind(mesh, &pose.rot, pose.pos, tx, ty, true);
                            if Some(k) == ground_mesh {
                                // (its textures' `.surf` maps: cobbled junctions shake the bus too)
                                let dirs = ot.texture_dirs(&self.root);
                                let dirs: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
                                let maps: Vec<_> = ot.meshes.get(k).map(|m| m.1.iter().map(|m| surf_map(&m.texture, &dirs)).collect()).unwrap_or_default();
                                ts.add_drive_mesh_surf(
                                    mesh,
                                    &pose.rot,
                                    pose.pos,
                                    tx,
                                    ty,
                                    omsi_geometry::SurfFaces::of(mesh, &maps).as_ref(),
                                );
                                wheel_meshes += 1;
                            }
                        }
                    }
                }
                ts.finish();
                let tile_terrain = self.terrains.read().get(&key).cloned();
                p.hole_walls = tile_terrain
                    .as_ref()
                    .map(|terrain| omsi_geometry::terrain_hole_walls(&hole_rims, terrain))
                    .unwrap_or_default();
                // How much of the ground the old cut rule ("anything below the terrain takes
                // it away") would have removed with nothing to put in its place: a hole in
                // the world you can see the sky through.
                let mut check: Check = (0, 0, Vec::new());
                if let (true, Some(t)) = (check_roads, tile_terrain.as_ref()) {
                    let n = ts.size;
                    let cell = tile_size() as f32 / n as f32;
                    for j in 0..n {
                        for i in 0..n {
                            let k = j * n + i;
                            if !ts.covered(k) {
                                continue;
                            }
                            check.1 += 1;
                            let th = t.sample((i as f32 + 0.5) * cell, (j as f32 + 0.5) * cell);
                            // the ground over a road: shows through it (Omsi.exe cuts nothing)
                            if ts.road_covered(k) && th > ts.road_height(k) + 0.03 && th < ts.road_height(k) + 1.5 && !ts.cut_at((i as f32 + 0.5) * cell, (j as f32 + 0.5) * cell, th, surface_flush()) {
                                OVER_ROAD.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                if let Ok(mut w) = OVER_ROAD_AT.lock() {
                                    w.push((x0 + ((i as f32 + 0.5) * cell) as f64, y0 + ((j as f32 + 0.5) * cell) as f64, th - ts.road_height(k), th));
                                }
                            }
                            let old_rule = th >= ts.low_height(k) - surface_flush();
                            let new_rule = ts.low_height(k) - surface_flush() <= th
                                && th <= ts.height(k) + surface_flush();
                            if old_rule && !new_rule {
                                check.0 += 1;
                                if check.2.len() < 100 {
                                    check.2.push((
                                        x0 + (i as f32 * cell) as f64,
                                        y0 + (j as f32 * cell) as f64,
                                        th - ts.height(k),
                                    ));
                                }
                            }
                        }
                    }
                }
                let terrain_at = move |x: f32, y: f32| {
                    tile_terrain.as_ref().map(|t| t.sample(x, y)).unwrap_or(0.0)
                };
                // the hole the roads cut into the ground, as an alpha image in tile space
                let cut = if ts.cuts_anything(&terrain_at, surface_flush()) {
                    if debug {
                        log::info!("tile ({tx}, {ty}): terrain cut under flush surfaces");
                    }
                    let rgba = ts.mask_image(&terrain_at, surface_flush());
                    if let Some(dir) = omsi_cfg::env::var("OMSI_DUMP_CUT").ok() {
                        let a: Vec<u8> = rgba.chunks_exact(4).map(|p| p[3]).collect();
                        if let Some(img) = image::GrayImage::from_raw(ts.size as u32, ts.size as u32, a) {
                            let _ = image::imageops::flip_vertical(&img).save(format!("{dir}/cut_{tx}_{ty}.png"));
                        }
                    }
                    Some(Image {
                        width: ts.size as u32,
                        height: ts.size as u32,
                        rgba,
                        has_alpha: true,
                    })
                } else {
                    None
                };
                // the painted ground layers: where the roads cut the ground away the paint
                // goes too, and a layer with nothing left on the tile is not drawn
                let masks = std::mem::take(&mut p.paint_masks);
                p.wall_paint.clear();
                p.paint = masks
                    .into_iter()
                    .filter_map(|(layer, img)| {
                        let (mut rgba, w, h) = smooth_paint_mask(
                            &img.rgba,
                            img.width as usize,
                            img.height as usize,
                        );
                        if !p.hole_walls.indices.is_empty()
                            && rgba.chunks_exact(4).any(|v| v[3] > 8)
                        {
                            p.wall_paint.push((
                                layer,
                                tile_texture(
                                    Image {
                                        width: w as u32,
                                        height: h as u32,
                                        rgba: rgba.clone(),
                                        has_alpha: true,
                                    },
                                    true,
                                ),
                            ));
                        }
                        let img = Image {
                            width: w as u32,
                            height: h as u32,
                            rgba: Vec::new(),
                            has_alpha: true,
                        };
                        let mut painted = 0usize;
                        for j in 0..h {
                            for i in 0..w {
                                let a = &mut rgba[(j * w + i) * 4 + 3];
                                if let Some(c) = &cut {
                                    // The cut as the terrain's alpha test sees it: sampled
                                    // bilinearly at this texel's centre, cut below one half.
                                    // Taken from the nearest cut texel (1.5-3 m each), the
                                    // paint kept teeth over the hole the road left in the
                                    // ground - drawn on top of the carriageway, a staircase
                                    // of asphalt or cobbles reaching into the road.
                                    if bilinear_alpha(c, (i as f32 + 0.5) / w as f32, (j as f32 + 0.5) / h as f32) < 0.5 {
                                        *a = 0;
                                    }
                                }
                                if *a > 8 {
                                    painted += 1;
                                }
                            }
                        }
                        (painted > 0).then(|| {
                            (
                                layer,
                                tile_texture(
                                    Image {
                                        width: img.width,
                                        height: img.height,
                                        rgba,
                                        has_alpha: true,
                                    },
                                    true,
                                ),
                                painted as f32 / (w * h).max(1) as f32,
                            )
                        })
                    })
                    .collect();
                (Arc::new(ts), cut, check, wheel_meshes)
            })
            .collect();
        let (mut holes, mut cells) = (0usize, 0usize);
        let mut where_: Vec<(f64, f64, f32)> = Vec::new();
        let (mut tris, mut wheel_meshes) = (0usize, 0usize);
        for (p, (ts, cut, check, wheels)) in prepared.iter_mut().zip(results) {
            p.cut = cut.map(|c| if omsi_cfg::env::var_os("OMSI_CUT_PLAIN").is_some() { omsi_texture::gpu::TextureData { gpu_mips: false, ..omsi_texture::gpu::TextureData::from_image(c) } } else { tile_texture(c, true) });
            tris += ts.drive.tris.len();
            wheel_meshes += wheels;
            // the wheel surfaces come and go with the tile (World::unload_tile)
            self.surfaces.write().insert((p.tx, p.ty), ts);
            holes += check.0;
            cells += check.1;
            where_.extend(check.2);
        }
        if debug_physics {
            log::info!("wheel surfaces: {tris} faces on {} tiles from {wheel_meshes} meshes", prepared.len());
        }
        if check_roads {
            where_.sort_by(|a, b| b.2.total_cmp(&a.2));
            log::info!("ground-cut check: {holes} of {cells} covered ground points would be cut away with nothing under them ({:.2} %)", holes as f32 / cells.max(1) as f32 * 100.0);
            let over = OVER_ROAD.load(std::sync::atomic::Ordering::Relaxed);
            log::info!("ground-over-road check: {over} of {cells} road points lie under the ground (3 cm to 1.5 m)");
            if let Ok(mut w) = OVER_ROAD_AT.lock() {
                w.sort_by(|a, b| b.2.total_cmp(&a.2));
                let by = |lo: f32, hi: f32| w.iter().filter(|p| p.2 >= lo && p.2 < hi).count();
                log::info!("   by depth: 3-10 cm {}, 10-30 cm {}, 30-60 cm {}, 60 cm-1.5 m {}", by(0.0, 0.1), by(0.1, 0.3), by(0.3, 0.6), by(0.6, 9.0));
                for (x, y, d, z) in w.iter().filter(|p| p.2 > 0.08 && p.2 < 0.3).step_by(97).take(6) {
                    log::info!("   (shallow) ground {d:.2} m over the road at ({x:.1}, {y:.1}, {z:.1})");
                }
                for (x, y, d, _) in w.iter().take(8) {
                    log::info!("   ground {d:.2} m over the road at ({x:.1}, {y:.1})");
                }
            }
            for (x, y, d) in where_.iter().take(5) {
                log::info!("   {d:.1} m of nothing under the ground at ({x:.0}, {y:.0})");
            }
        }
        // The textures of object types, splines and trees that are not on the GPU yet are
        // decoded here instead of on the thread that draws. A big batch (a whole map at
        // once) decodes them as it uploads instead: all at once they would not fit.
        if prepared.len() <= 16 {
            for p in prepared.iter_mut() {
                for o in p.objects.iter_mut() {
                    let freetex = o.ot.meshes.iter().any(|(_, _, ov)| ov.iter().any(|m| !m.item && m.freetex.is_some()));
                    if o.lamp.is_some() || (o.ot.dynamic_textures.is_empty() && !freetex) {
                        continue;
                    }
                    if let Some(program) = o.ot.program.clone() {
                        o.script = Some(omsi_sim::scenery::SceneryInstance::new(program, &o.ot.mesh_defs(), self.script_clock(), &o.strings));
                    }
                }
            }
            let mut wanted: Vec<(String, Vec<PathBuf>)> = prepared
                .iter()
                .flat_map(|p| self.wanted_textures(p))
                .collect();
            wanted.sort();
            wanted.dedup();
            let decoded: Vec<(PathBuf, Arc<TextureData>)> = wanted
                .par_iter()
                .filter_map(|(name, dirs)| {
                    let dirs_ref: Vec<&Path> = dirs.iter().map(|d| d.as_path()).collect();
                    let path = omsi_texture::find_texture(name, &dirs_ref)?;
                    let (img, _) = omsi_texture::gpu::load_gpu(&path).ok()?;
                    Some((path, Arc::new(img)))
                })
                .collect();
            let decoded: Arc<HashMap<PathBuf, Arc<TextureData>>> =
                Arc::new(decoded.into_iter().collect());
            for p in prepared.iter_mut() {
                p.images = decoded.clone();
            }
        }
    }

    /// Texture names (with their search folders) a prepared tile will ask for that are not
    /// on the GPU yet.
    ///
    /// Runs on the loader thread. The GPU cache is only locked for two quick looks (which
    /// types are there, then which of the found files are): the lookups themselves go to
    /// the disk or into an archive the first time a name comes up, and the thread that
    /// draws waited for them - a tile upload of the Ahlheim main station took 372 ms.
    fn wanted_textures(&self, p: &Prepared) -> Vec<(String, Vec<PathBuf>)> {
        let (have_types, have_splines, have_trees): (
            hashbrown::HashSet<usize>,
            hashbrown::HashSet<usize>,
            hashbrown::HashSet<String>,
        ) = {
            let gpu = self.gpu.lock();
            (
                p.objects
                    .iter()
                    .map(|o| Arc::as_ptr(&o.ot) as usize)
                    .filter(|k| gpu.types.contains_key(k))
                    .collect(),
                p.splines
                    .iter()
                    .map(|(_, st, _, _)| Arc::as_ptr(st) as usize)
                    .filter(|k| gpu.splines.contains_key(k))
                    .collect(),
                p.trees
                    .iter()
                    .map(|t| t.1.to_ascii_lowercase())
                    .filter(|k| gpu.trees.contains_key(k))
                    .collect(),
            )
        };
        let mut names: Vec<(String, Vec<PathBuf>)> = Vec::new();
        let push = |name: &str, dirs: &[PathBuf], out: &mut Vec<(String, Vec<PathBuf>)>| {
            if name.trim().is_empty() || out.len() > 4096 {
                return;
            }
            out.push((name.to_string(), dirs.to_vec()));
        };
        let mut seen: hashbrown::HashSet<*const ObjectType> = hashbrown::HashSet::new();
        for o in &p.objects {
            if let Some(inst) = &o.script {
                let selection = scenery_texture_selection(&o.ot, inst);
                for (group, &index) in o.ot.dynamic_textures.iter().zip(&selection) {
                    for (_, file, dir) in group.choices.get(index).into_iter().flatten() {
                        let mut dirs = o.ot.texture_dirs(&self.root);
                        dirs.insert(0, dir.clone());
                        push(file, &dirs, &mut names);
                    }
                }
            }
            if o.lamp.is_none() {
                for (_, _, overrides) in &o.ot.meshes {
                    for ov in overrides.iter().filter(|m| !m.item && m.freetex.is_some()) {
                        let Some((_, var)) = &ov.freetex else { continue };
                        if let Some(name) = resolve_scenery_freetex_name(var, ov, overrides, o.script.as_ref(), None, &o.strings) {
                            push(name, &o.ot.texture_dirs(&self.root), &mut names);
                        }
                    }
                }
            }
            let key = Arc::as_ptr(&o.ot);
            if have_types.contains(&(key as usize)) || !seen.insert(key) {
                continue;
            }
            let dirs = o.ot.texture_dirs(&self.root);
            for (_, mats, overrides) in
                o.ot.meshes
                    .iter()
                    .chain(o.ot.lower_lods.iter().flat_map(|l| l.1.iter()))
            {
                for m in mats {
                    push(&m.texture, &dirs, &mut names);
                    // and its copy in the `night` folder, which `type_gpu` looks for: left to
                    // the main thread, big night JPEGs of a mod map were decoded there, up to
                    // 1.7 s for one object type (the freezes on Grande Porto, Novi Sad)
                    if !is_null_texture(&m.texture) {
                        push(&night_texture_name(&m.texture), &dirs, &mut names);
                    }
                }
                for ov in overrides {
                    push(&ov.texture, &dirs, &mut names);
                    if let Some(n) = &ov.nightmap {
                        push(n, &dirs, &mut names);
                    }
                    if let Some(t) = &ov.transmap {
                        push(t, &dirs, &mut names);
                    }
                    if let Some((e, _)) = &ov.envmap {
                        push(e, &dirs, &mut names);
                    }
                    if let Some(m) = &ov.envmap_mask {
                        push(m, &dirs, &mut names);
                    }
                }
            }
        }
        for (_, st, _, _) in &p.splines {
            if have_splines.contains(&(Arc::as_ptr(st) as usize)) {
                continue;
            }
            let dirs = texture_dirs(&self.root, &st.dir);
            for t in &st.def.textures {
                push(&t.file, &dirs, &mut names);
            }
        }
        for (ot, tex, ..) in &p.trees {
            if !have_trees.contains(&tex.to_ascii_lowercase()) {
                push(tex, &ot.texture_dirs(&self.root), &mut names);
            }
        }
        // the painted ground layers (and their detail textures) of the tile
        let ground_dirs = vec![self.root.clone()];
        for (layer, _) in &p.paint_masks {
            if let Some(gt) = self.global.ground_textures.get(*layer) {
                push(&gt.texture, &ground_dirs, &mut names);
                push(&gt.detail_texture, &ground_dirs, &mut names);
            }
        }
        names.sort();
        names.dedup();
        // find the files without the lock, then keep what the GPU does not have yet
        let found: Vec<(PathBuf, (String, Vec<PathBuf>))> = names
            .into_iter()
            .filter_map(|(name, dirs)| {
                let dirs_ref: Vec<&Path> = dirs.iter().map(|d| d.as_path()).collect();
                omsi_texture::find_texture(&name, &dirs_ref).map(|path| (path, (name, dirs)))
            })
            .collect();
        let gpu = self.gpu.lock();
        found
            .into_iter()
            .filter(|(path, _)| !gpu.textures.contains_key(path))
            .map(|(_, n)| n)
            .collect()
    }

    /// The materials every tile shares: the plain ground, the water, the tree quad.
    fn ensure_ground(&self, renderer: &Renderer, scene: &mut Scene, gpu: &mut GpuCache) {
        if gpu.ground.is_some() {
            return;
        }
        let none: HashMap<PathBuf, Arc<TextureData>> = HashMap::new();
        let ground_dirs = vec![self.root.clone()];
        let ground0 = self
            .global
            .ground_textures
            .first()
            .cloned()
            .unwrap_or_default();
        let ground_tex = if ground0.texture.trim().is_empty() {
            "Texture/gras.bmp".to_string()
        } else {
            ground0.texture.clone()
        };
        let ground_id = gpu
            .texture(renderer, scene, &ground_tex, &ground_dirs, &none)
            .map(|t| t.0);
        let ground_mat = renderer.add_material_night(
            scene,
            ground_id,
            AlphaMode::Opaque,
            [1.0; 4],
            false,
            None,
            None,
        );
        // The first [groundtex] is what the whole map starts as; its two numbers say how
        // often the texture and its detail texture repeat across one tile.
        let ground_repeats = if ground0.params[1] > 0.0 {
            ground0.repeats()
        } else {
            (tile_size() / 12.0) as f32
        };
        let ground_detail = gpu
            .texture(
                renderer,
                scene,
                &ground0.detail_texture,
                &ground_dirs,
                &none,
            )
            .map(|t| (t.0, ground0.detail_repeats()));
        // The base layer wets in the rain exactly like a painted one when its own
        // <texture>.cfg carries [moisture]/[puddles] - this used to be dropped on the
        // floor (add_terrain_material had no moisture parameter at all), so a map whose
        // default ground is a wet-tagged surface (rather than the untagged stock grass)
        // never showed it, while the very same texture painted as a later [groundtex]
        // layer (add_terrain_layer_material) got it right: a patchwork of wet and dry
        // that had nothing to do with the weather.
        let ground_dirs_ref: Vec<&Path> = ground_dirs.iter().map(|p| p.as_path()).collect();
        let ground_cfg = self.textures.cfg(&ground_tex, &ground_dirs_ref);
        let ground_wet = if ground_cfg.moisture || ground_cfg.puddles {
            1.0
        } else {
            0.0
        };
        let plain_terrain_mat = renderer.add_terrain_material(
            scene,
            ground_id,
            None,
            ground_detail,
            ground_repeats,
            None,
            ground_wet,
        );
        // Water: the map carries its own colour in `texture/water.tga` (the stock maps use
        // an 8x8 swatch of 47, 74, 83 at three quarters opacity, so the riverbed shows
        // through) and its own sphere map in `texture/water_envmap.bmp`. A map or mod that
        // ships different ones gets its own water.
        let water_mat = {
            let wdir = omsi_cfg::resolve_path(&self.map_dir, "texture");
            let dirs: Vec<&Path> = vec![wdir.as_path(), self.root.as_path()];
            let tex = match self.textures.get("water.tga", &dirs) {
                Some(img) => renderer.add_texture(scene, &img, true),
                None => {
                    let img = Image {
                        width: 1,
                        height: 1,
                        rgba: vec![47, 74, 83, 192],
                        has_alpha: true,
                    };
                    renderer.add_texture(scene, &img, false)
                }
            };
            let env = self
                .textures
                .get("water_envmap.bmp", &dirs)
                .or_else(|| {
                    self.textures.get(
                        "envmap_unscharf.bmp",
                        &[
                            &self.root.join("Vehicles/MAN_SD202/Texture"),
                            &omsi_cfg::resolve_path(&self.root, "Texture"),
                        ],
                    )
                })
                .map(|i| renderer.add_texture(scene, &i, true));
            renderer.add_material_extra(
                scene,
                Some(tex),
                AlphaMode::Blend,
                [1.0; 4],
                false,
                None,
                None,
                None,
                env.map(|e| (e, 0.45)),
                [0.0; 3],
                omsi_render::MaterialExtra { water: true, ..Default::default() },
            )
        };
        let tree_mesh = renderer.add_mesh(scene, &tree_quad_mesh());
        gpu.ground = Some(GroundGpu {
            ground_id,
            ground_mat,
            plain_terrain_mat,
            ground_detail,
            ground_repeats,
            ground_wet,
            water_mat,
            tree_mesh,
        });
    }

    /// The GPU side of an object type (meshes, materials with their variants and night maps,
    /// lower LODs), uploaded once while any loaded tile uses it.
    fn type_gpu(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        gpu: &mut GpuCache,
        ot: &Arc<ObjectType>,
        images: &HashMap<PathBuf, Arc<TextureData>>,
        ground_mat: MaterialId,
    ) -> usize {
        let key = Arc::as_ptr(ot) as usize;
        if gpu.types.contains_key(&key) {
            return key;
        }
        let dirs = ot.texture_dirs(&self.root);
        let mut t = TypeGpu {
            ot: ot.clone(),
            meshes: Vec::new(),
            variants: Vec::new(),
            dynamic_texture_variants: HashMap::new(),
            lods: Vec::new(),
            materials: Vec::new(),
            textures: Vec::new(),
            users: 0,
            auto_night: false,
            lod0_lo: 0.0,
            lod0_max: f32::MAX,
            terrain_slots: Vec::new(),
            terrain_rest: Vec::new(),
        };
        for (mesh, o3d_mats, overrides) in &ot.meshes {
            let mut mats: Vec<MaterialId> = Vec::new();
            for (slot, m) in o3d_mats.iter().enumerate() {
                let tex_of = |gpu: &mut GpuCache,
                              scene: &mut Scene,
                              name: &str,
                              t: &mut TypeGpu|
                 -> Option<TextureId> {
                    let (id, path) = gpu.texture(renderer, scene, name, &dirs, images)?;
                    t.textures.push(path);
                    Some(id)
                };
                let for_slot =
                    |o: &&MaterialDef| omsi_sim::vehicle::override_slot(o3d_mats, o) == Some(slot);
                // a slot fed by [useTextTexture] / [useScriptTexture] gets a generated picture:
                // the name in the mesh is a placeholder (the stop poles' Textfeld_1.bmp, the
                // street signs' StrSchild_Text1.bmp), and looking for it on disk only wrote
                // "Did not find texture file" into the log for every map
                let generated = overrides
                    .iter()
                    .filter(|o| !o.item)
                    .filter(for_slot)
                    .any(|o| o.use_text_texture.is_some() || o.use_script_texture.is_some());
                let tex = if is_null_texture(&m.texture) || generated {
                    None
                } else {
                    tex_of(gpu, scene, &m.texture, &mut t)
                };
                let base_ov: Vec<MaterialDef> =
                    overrides.iter().filter(|o| !o.item).cloned().collect();
                let alpha = material_alpha(o3d_mats, slot, &base_ov);
                let night = match overrides
                    .iter()
                    .filter(|o| !o.item && o.nightmap.is_some())
                    .filter(for_slot)
                    .find_map(|o| o.nightmap.clone())
                {
                    Some(n) => tex_of(gpu, scene, &n, &mut t),
                    // OMSI's own night textures: a copy of the texture in the `night` folder
                    // beside it, black but for the lit windows and signs, added at night as
                    // the object's [NightMapMode] says. Every stock building has them (the
                    // Buildings_RW1HH folder alone 60), and without them the city stood dark.
                    None if !is_null_texture(&m.texture) => {
                        let rel = night_texture_name(&m.texture);
                        let dirs_ref: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
                        if night_texture_exists(&rel, &dirs_ref) {
                            t.auto_night = true;
                            tex_of(gpu, scene, &rel, &mut t)
                        } else {
                            None
                        }
                    }
                    None => None,
                };
                let slot_ov: Vec<&MaterialDef> = overrides
                    .iter()
                    .filter(|o| !o.item)
                    .filter(for_slot)
                    .collect();
                let (color, emissive, specular, ambient) =
                    d3d_material(m, slot_ov.iter().find_map(|o| o.allcolor), tex.is_some());
                // [matl_transmap]: transparency from a separate map (parked cars: body opaque, windows clear)
                let transmap = match overrides
                    .iter()
                    .filter(|o| !o.item)
                    .filter(for_slot)
                    .find_map(|o| o.transmap.clone())
                    .filter(|t| !t.trim().is_empty() && !t.trim().starts_with("\\S:"))
                {
                    Some(name) => {
                        let dirs_ref: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
                        match tex_of(gpu, scene, &name, &mut t) {
                            Some(id) => {
                                let has_alpha = omsi_texture::find_texture(&name, &dirs_ref)
                                    .map(|p| gpu.has_alpha(&p))
                                    .unwrap_or(false);
                                Some((id, has_alpha))
                            }
                            None => None,
                        }
                    }
                    None => None,
                };
                // A separate transmap is a mask, not an automatic instruction to make the
                // whole material transparent. Opaque body panels must stay opaque unless the
                // model's `[matl_alpha]` or a material override explicitly says otherwise.
                // A declared blend on a texture without alpha is opaque, as the splines take
                // it, for a ground-layer object (`[rendertype] surface` / `on_surface`): the
                // surface phases draw a blend without writing depth, so every spline after
                // it showed through - the far roads and a bridge over NCCR's apartment
                // blocks (`[matl_alpha] 2` on a 24-bit BMP), Westcountry's road signs. In
                // Omsi.exe such a blend writes depth and its alpha is 1 throughout, the same
                // picture. (Not with a transmap, an `[alphascale]` or a texture with alpha.)
                let surface_phase = matches!(
                    ot.sco.render_type,
                    omsi_scenery::sco::RenderType::Surface | omsi_scenery::sco::RenderType::OnSurface
                );
                let faded = slot_ov.iter().any(|o| o.alphascale.as_ref().is_some_and(|v| !v.trim().is_empty()));
                let alpha = if alpha == AlphaMode::Blend && surface_phase && tex.is_some() && transmap.is_none() && !faded && {
                    let dirs_ref: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
                    omsi_texture::find_texture(&m.texture, &dirs_ref).is_some_and(|p| gpu.textures.get(&p).is_some_and(|e| !e.alpha))
                } {
                    AlphaMode::Opaque
                } else {
                    alpha
                };
                // [matl_envmap]: the same reflection rule as on vehicles (factor x mask; a
                // texture without an alpha channel reads as a full mask)
                let envmap = match slot_ov.iter().find_map(|o| o.envmap.clone()) {
                    Some((name, f)) => tex_of(gpu, scene, &name, &mut t).map(|id| (id, f)),
                    None => None,
                };
                let env_mask = match slot_ov
                    .iter()
                    .find_map(|o| o.envmap_mask.clone())
                    .filter(|_| envmap.is_some())
                {
                    Some(name) => tex_of(gpu, scene, &name, &mut t),
                    None => None,
                };
                let bump = match slot_ov
                    .iter()
                    .find_map(|o| o.bumpmap.clone())
                    .filter(|_| envmap.is_some() && omsi_cfg::env::var_os("OMSI_NO_BUMP").is_none())
                {
                    Some((name, f)) => {
                        gpu.bump_texture(renderer, scene, &name, &dirs)
                            .map(|(id, key)| {
                                t.textures.push(key);
                                (id, f)
                            })
                    }
                    None => None,
                };
                let mut extra = material_extra(&slot_ov, env_mask, bump, specular);
                extra.ambient = Some(ambient);
                extra.no_map_lights = ot.sco.no_map_lighting;
                // windy trees: a plant's leaves (its cut-out or blended slots) bend in the wind
                if alpha != AlphaMode::Opaque {
                    if let Some(give) = vegetation_give_of(&ot.sco) {
                        extra.sway = foliage_sway(&ot.meshes, mesh, slot as u32, give);
                    }
                }
                if tex.is_some() {
                    let dirs_ref: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
                    let c = self.textures.cfg(&m.texture, &dirs_ref);
                    if c.moisture || c.puddles {
                        extra.moisture = 1.0;
                    }
                    if c.terrain_mapping {
                        t.terrain_slots.push((0, t.meshes.len(), slot));
                    }
                }
                let address = tex_addressing(overrides.iter().filter(for_slot));
                renderer.address_next.set(address);
                renderer.light_map_next.set(ot.sco.light_map_mapping);
                // OMSI_DEBUG_OBJMAT=<part of the object's file name>: how its slots are made
                if let Ok(f) = omsi_cfg::env::var("OMSI_DEBUG_OBJMAT") {
                    if ot.sco.path.to_string_lossy().to_ascii_lowercase().contains(&f.to_ascii_lowercase()) {
                        log::info!("{} slot {slot} '{}': tex {} alpha {:?} color {:?} emissive {:?} night {} transmap {:?} envmap {:?} auto_night {}", ot.sco.path.display(), m.texture, tex.is_some(), alpha, color, emissive, night.is_some(), transmap.map(|t| t.1), envmap.map(|e| e.1), t.auto_night);
                    }
                }
                // [matl_lightmap]: laid on the light as on a vehicle (a lamp's lens, a lit
                // shelter or advertising pillar); switched by its variable per placement
                // (`LampSlots`), on where nothing switches it. (Left out, a signal whose
                // lenses are lit by their light maps stayed dark, #826.)
                let light = match slot_ov.iter().find_map(|o| o.lightmap.clone()) {
                    Some((name, _)) => tex_of(gpu, scene, &name, &mut t),
                    None => None,
                };
                let base = renderer.add_material_extra(
                    scene, tex, alpha, color, false, transmap, night, light, envmap, emissive, extra,
                );
                let base = gpu.material(renderer, scene, base);
                t.materials.push(base);
                // variant of a [matl_change]
                let change_var = overrides
                    .iter()
                    .filter(|o| !o.item)
                    .filter(for_slot)
                    .find_map(|o| o.change.as_ref().map(|c| c.2.clone()));
                let items: Vec<&MaterialDef> = overrides
                    .iter()
                    .filter(|o| o.item)
                    .filter(for_slot)
                    .collect();
                if let (Some(var), false) = (change_var, items.is_empty()) {
                    let it_night = match items.iter().find_map(|o| o.nightmap.clone()) {
                        Some(n) => tex_of(gpu, scene, &n, &mut t).or(night),
                        None => night,
                    };
                    let (ic, ie, is, ia) = d3d_material(
                        m,
                        items
                            .iter()
                            .find_map(|o| o.allcolor)
                            .or(slot_ov.iter().find_map(|o| o.allcolor)),
                        tex.is_some(),
                    );
                    let it_alpha = items.first().map(|o| alpha_mode(o.alpha)).unwrap_or(alpha);
                    let mut it_extra = material_extra(&items, env_mask, bump, is);
                    it_extra.ambient = Some(ia);
                    it_extra.night_switched = items.iter().any(|o| o.nightmap.is_some());
                    it_extra.no_z_write |= extra.no_z_write;
                    it_extra.no_z_check |= extra.no_z_check;
                    it_extra.glass |= extra.glass;
                    renderer.address_next.set(address);
                    renderer.light_map_next.set(ot.sco.light_map_mapping);
                    let it_light = match items.iter().find_map(|o| o.lightmap.clone()) {
                        Some((name, _)) => tex_of(gpu, scene, &name, &mut t).or(light),
                        None => light,
                    };
                    let item = renderer.add_material_extra(
                        scene, tex, it_alpha, ic, false, transmap, it_night, it_light, envmap, ie,
                        it_extra,
                    );
                    let item = gpu.material(renderer, scene, item);
                    t.materials.push(item);
                    t.variants.push((t.meshes.len(), slot, base, item, var));
                }
                mats.push(base);
            }
            let id = gpu.add_mesh(renderer, scene, mesh);
            scene.meshes[id].source = Some(ot.sco.path.display().to_string());
            t.meshes.push((
                id,
                if mats.is_empty() {
                    vec![ground_mat]
                } else {
                    mats
                },
            ));
        }
        // lower LODs (plain materials). OMSI picks a level the way the model lists them
        // (Omsi.exe 0x5ef860): the first whose least size the object's screen size reaches,
        // else the last one whatever its own. A level is so drawn from its least size (the
        // last from 0) up to the least of the sizes listed before it. The stock Sv signals
        // say [LOD] 0.1 (the signal) before [LOD] 1 (its low version): taken as size bands
        // the low version stood in close up and the signal vanished in the distance; and a
        // model with a single [LOD] 0.5 is drawn at any size.
        let mins: Vec<f32> = std::iter::once(ot.lod0_min).chain(ot.lower_lods.iter().map(|l| l.0)).collect();
        let band = |i: usize| -> (f32, f32) {
            let lo = if i + 1 == mins.len() { 0.0 } else { mins[i] };
            (lo, mins[..i].iter().copied().fold(f32::MAX, f32::min))
        };
        (t.lod0_lo, t.lod0_max) = band(0);
        for (k, (_, meshes)) in ot.lower_lods.iter().enumerate() {
            let (lo, upper) = band(k + 1);
            let mut l = Vec::new();
            for (mesh, o3d_mats, overrides) in meshes {
                let mut mats = Vec::new();
                for (slot, m) in o3d_mats.iter().enumerate() {
                    // (a text or script texture slot's name is a placeholder, see above)
                    let generated = overrides.iter().any(|o| {
                        !o.item
                            && omsi_sim::vehicle::override_slot(o3d_mats, o) == Some(slot)
                            && (o.use_text_texture.is_some() || o.use_script_texture.is_some())
                    });
                    let tex = if is_null_texture(&m.texture) || generated {
                        None
                    } else {
                        match gpu.texture(renderer, scene, &m.texture, &dirs, images) {
                            Some((id, path)) => {
                                t.textures.push(path);
                                Some(id)
                            }
                            None => None,
                        }
                    };
                    if tex.is_some() {
                        let dirs_ref: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
                        if self.textures.cfg(&m.texture, &dirs_ref).terrain_mapping {
                            t.terrain_slots.push((k + 1, l.len(), slot));
                        }
                    }
                    let mat = renderer.add_material_night(
                        scene,
                        tex,
                        material_alpha(o3d_mats, slot, overrides),
                        [1.0; 4],
                        false,
                        None,
                        None,
                    );
                    let mat = gpu.material(renderer, scene, mat);
                    t.materials.push(mat);
                    mats.push(mat);
                }
                let id = gpu.add_mesh(renderer, scene, mesh);
                scene.meshes[id].source = Some(ot.sco.path.display().to_string());
                l.push((
                    id,
                    if mats.is_empty() {
                        vec![ground_mat]
                    } else {
                        mats
                    },
                ));
            }
            t.lods.push((lo, upper, l));
        }
        gpu.types.insert(key, t);
        key
    }

    /// Put a prepared tile on the GPU in one go.
    pub fn upload_tile(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        p: Prepared,
        stats: &mut LoadStats,
    ) {
        let mut u = self.begin_upload(p);
        while !self.upload_step(renderer, scene, &mut u, None) {}
        self.finish_upload(renderer, scene, u, stats);
    }

    /// Give up on a tile on its way in (it went out of range): what it already has on the
    /// GPU - textures and object types it holds, instances placed so far - goes back.
    pub fn abandon_upload(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        u: PendingUpload,
        audio: Option<&omsi_audio::AudioEngine>,
    ) {
        let key = u.key();
        let PendingUpload { tg, placing, .. } = u;
        let orphan = {
            let mut states = self.tile_state.lock();
            match states.get_mut(&key) {
                Some(state) => {
                    state.gpu = tg;
                    state.poles = placing.poles;
                    None
                }
                None => Some((tg, placing.poles)),
            }
        };
        let mut freed = false;
        if let Some((tg, poles)) = orphan {
            self.poles.lock().retain(|k, _| !poles.contains(k));
            self.help_arrows.lock().remove(&key);
            self.parked_objects.lock().retain(|_, p| p.tile != key);
            self.departed_objects.lock().retain(|_, (p, _, _)| p.tile != key);
            self.edit_objects.lock().retain(|_, o| o.tile != key);
            freed = self.gpu.lock().release_tile(renderer, scene, tg) > 0;
        }
        freed |= self.unload_tile(renderer, scene, key, audio);
        if freed {
            self.trim_object_types();
        }
    }

    /// Start putting a prepared tile on the GPU: what it needs that is not there yet (the
    /// textures decoded for it, its new object types) goes up in steps first.
    pub fn begin_upload(&self, p: Prepared) -> PendingUpload {
        let gpu = self.gpu.lock();
        let mut textures: Vec<PathBuf> = p
            .images
            .keys()
            .filter(|k| !gpu.textures.contains_key(*k))
            .cloned()
            .collect();
        textures.sort();
        let mut types: Vec<Arc<ObjectType>> = Vec::new();
        let mut seen: hashbrown::HashSet<usize> = hashbrown::HashSet::new();
        for o in &p.objects {
            let key = Arc::as_ptr(&o.ot) as usize;
            if !gpu.types.contains_key(&key) && seen.insert(key) {
                types.push(o.ot.clone());
            }
        }
        // the objects are placed from the back of the list: in the order of the tile
        let mut p = p;
        p.objects.reverse();
        PendingUpload {
            prepared: p,
            textures,
            types,
            tg: TileGpu::default(),
            placing: Placing::default(),
        }
    }

    /// Upload one texture or one object type after the other until `deadline` (just one
    /// with no deadline). True when only the tile itself is left to place.
    pub fn upload_step(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        u: &mut PendingUpload,
        deadline: Option<std::time::Instant>,
    ) -> bool {
        let mut gpu_guard = self.gpu.lock();
        let gpu = &mut *gpu_guard;
        self.ensure_ground(renderer, scene, gpu);
        let ground_mat = gpu.ground.as_ref().unwrap().ground_mat;
        let slow = omsi_cfg::env::var_os("OMSI_DEBUG_UPLOAD").is_some();
        loop {
            let t_item = std::time::Instant::now();
            if let Some(path) = u.textures.pop() {
                if !gpu.textures.contains_key(&path) {
                    if let Some(img) = u.prepared.images.get(&path) {
                        let id = gpu.add_data(renderer, scene, img);
                        // (the PBR maps beside it: only the textures decoded on the spot had
                        // them, the ones the tile's preparation brought - nearly all of a
                        // map's - were drawn flat)
                        if !path.to_string_lossy().ends_with("#bump") {
                            attach_pbr(renderer, scene, &path, id);
                        }
                        gpu.textures.insert(
                            path.clone(),
                            TexEntry {
                                id,
                                alpha: img.has_alpha,
                                users: 1,
                                texels: img.width as u64 * img.height as u64,
                                bytes: renderer.texture_size_bytes(scene, id),
                                format: img.format,
                                dropped: 0,
                            },
                        );
                        // the tile holds the texture until its object types take it over
                        if slow && t_item.elapsed().as_millis() > 8 { log::info!("upload: texture {} ({}x{} {:?}) took {} ms", path.display(), img.width, img.height, img.format, t_item.elapsed().as_millis()); }
                        u.tg.shared_textures.push(path);
                    }
                }
            } else if let Some(ot) = u.types.pop() {
                let before = (gpu.sync_decodes, gpu.sync_decode_secs);
                let key = self.type_gpu(renderer, scene, gpu, &ot, &u.prepared.images, ground_mat);
                if slow && t_item.elapsed().as_millis() > 8 { log::info!("upload: object type {} took {} ms ({} meshes, {} textures decoded here in {:.0} ms)", ot.model_dir.display(), t_item.elapsed().as_millis(), ot.meshes.len(), gpu.sync_decodes - before.0, (gpu.sync_decode_secs - before.1) * 1000.0); }
                if !u.tg.types.contains(&key) {
                    gpu.types.get_mut(&key).unwrap().users += 1;
                    u.tg.types.push(key);
                }
            } else {
                return true;
            }
            match deadline {
                Some(d) if std::time::Instant::now() < d => {}
                _ => return u.textures.is_empty() && u.types.is_empty(),
            }
        }
    }

    /// Place a tile whose textures and object types are on the GPU, in one go (see
    /// [`World::place_step`]).
    pub fn finish_upload(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        mut u: PendingUpload,
        stats: &mut LoadStats,
    ) {
        while !self.place_step(renderer, scene, &mut u, None) {}
        self.commit_upload(u, stats);
    }

    /// Hand a placed tile's records to its [`TileState`], so that [`World::unload_tile`] can
    /// give everything back.
    pub fn commit_upload(&self, u: PendingUpload, stats: &mut LoadStats) {
        let key = u.key();
        let PendingUpload { tg, placing, .. } = u;
        stats.splines += placing.splines;
        stats.trees += placing.trees;
        stats.objects += placing.objects;
        {
            let gpu = self.gpu.lock();
            stats.object_types = gpu.types.len();
            stats.spline_types = gpu.splines.len();
        }
        let mut states = self.tile_state.lock();
        let state = states.entry(key).or_default();
        state.gpu = tg;
        state.night_slots = placing.night_slots;
        state.night_modes = placing.night_modes;
        state.light_objects = placing.light_objects;
        state.poles = placing.poles;
    }

    /// Place a tile whose textures and object types are on the GPU - the ground, then the
    /// splines, the trees and the objects - until `deadline` (always a little: at least the
    /// ground or one spline, a few trees or one object). True when all of it is placed.
    /// A big tile (a main station with a few thousand objects and signs) took a third of a
    /// second in one piece.
    pub fn place_step(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        u: &mut PendingUpload,
        deadline: Option<std::time::Instant>,
    ) -> bool {
        let t_lock = std::time::Instant::now();
        let mut gpu_guard = self.gpu.lock();
        let lock_wait = t_lock.elapsed().as_secs_f64();
        let gpu = &mut *gpu_guard;
        self.ensure_ground(renderer, scene, gpu);
        let (
            ground_id,
            ground_mat,
            plain_terrain_mat,
            ground_detail,
            ground_repeats,
            ground_wet,
            water_mat,
            tree_mesh,
        ) = {
            let g = gpu.ground.as_ref().unwrap();
            (
                g.ground_id,
                g.ground_mat,
                g.plain_terrain_mat,
                g.ground_detail,
                g.ground_repeats,
                g.ground_wet,
                g.water_mat,
                g.tree_mesh,
            )
        };
        let PendingUpload {
            prepared: p,
            tg,
            placing: pl,
            ..
        } = u;
        let key = (p.tx, p.ty);
        let images: &HashMap<PathBuf, Arc<TextureData>> = &p.images;
        let ground_dirs = vec![self.root.clone()];
        let only_object = omsi_cfg::env::var("OMSI_ONLY_OBJECT").is_ok();
        let decodes_before = (gpu.sync_decodes, gpu.sync_decode_secs);
        let t_start = std::time::Instant::now();
        let out_of_time = |done_some: bool| {
            done_some
                && deadline
                    .map(|d| std::time::Instant::now() >= d)
                    .unwrap_or(false)
        };
        macro_rules! instance {
            ($id:expr) => {{
                let new = $id;
                let i = gpu.instance(renderer, scene, new);
                tg.instances.push(i);
                i
            }};
        }
        let mut done_some = false;
        while pl.phase < 4 && !out_of_time(done_some) {
            let t_phase = std::time::Instant::now();
            let phase = pl.phase;
            match phase {
                0 => {
                    if let (Some(mesh), false) = (&p.terrain, only_object) {
                        let id = gpu.add_mesh(renderer, scene, mesh);
                        tg.meshes.push(id);
                        // the tile's night light map (lamp light pools on the ground)
                        let lm = p.light_map.as_ref().map(|img| {
                            let t = gpu.add_data(renderer, scene, img);
                            tg.textures.push(t);
                            t
                        });
                        let mat = match &p.cut {
                            Some(img) => {
                                let tex = gpu.add_data(renderer, scene, img);
                                tg.textures.push(tex);
                                let m = renderer.add_terrain_material(
                                    scene,
                                    ground_id,
                                    Some(tex),
                                    ground_detail,
                                    ground_repeats,
                                    lm,
                                    ground_wet,
                                );
                                let m = gpu.material(renderer, scene, m);
                                tg.materials.push(m);
                                m
                            }
                            None if lm.is_some() => {
                                let m = renderer.add_terrain_material(
                                    scene,
                                    ground_id,
                                    None,
                                    ground_detail,
                                    ground_repeats,
                                    lm,
                                    ground_wet,
                                );
                                let m = gpu.material(renderer, scene, m);
                                tg.materials.push(m);
                                m
                            }
                            None => plain_terrain_mat,
                        };
                        let ground_instance = instance!(renderer.add_instance(
                            scene,
                            id,
                            p.origin,
                            Mat4::IDENTITY,
                            vec![mat]
                        ));
                        if let Some(inst) = scene.instances.get_mut(ground_instance) {
                            inst.render_phase = RenderPhase::Terrain;
                        }
                        // (the base layer once more without the cut, when the tile has one)
                        let uncut = match (&p.cut, lm) {
                            (None, _) => mat,
                            (Some(_), None) => plain_terrain_mat,
                            (Some(_), Some(_)) => {
                                let m = renderer.add_terrain_material(
                                    scene,
                                    ground_id,
                                    None,
                                    ground_detail,
                                    ground_repeats,
                                    lm,
                                    ground_wet,
                                );
                                let m = gpu.material(renderer, scene, m);
                                tg.materials.push(m);
                                m
                            }
                        };
                        pl.terrain_mapping_mat = Some(uncut);
                        let wall_id = if p.hole_walls.indices.is_empty() {
                            None
                        } else {
                            let wall = gpu.add_mesh(renderer, scene, &p.hole_walls);
                            tg.meshes.push(wall);
                            let wi = instance!(renderer.add_instance(
                                scene,
                                wall,
                                p.origin,
                                Mat4::IDENTITY,
                                vec![uncut]
                            ));
                            if let Some(inst) = scene.instances.get_mut(wi) {
                                inst.render_phase = RenderPhase::Terrain;
                            }
                            Some(wall)
                        };
                        // The painted ground: every further [groundtex] the editor's brush put on this
                        // tile is the same tile mesh once more, blended in through its own mask - which
                        // is how OMSI's car parks get their asphalt, its side streets their cobbles and
                        // its meadows their fields.
                        let no_paint = omsi_cfg::env::var_os("OMSI_NO_GROUND_PAINT").is_some();
                        for (layer, mask, painted) in p.paint.iter().filter(|_| !no_paint) {
                            let Some(gt) = self.global.ground_textures.get(*layer) else {
                                continue;
                            };
                            let tex = gpu.add_data(renderer, scene, mask);
                            tg.textures.push(tex);
                            let layer_tex = gpu
                                .texture(renderer, scene, &gt.texture, &ground_dirs, images)
                                .map(|(id, path)| {
                                    tg.shared_textures.push(path);
                                    id
                                });
                            let detail = gpu
                                .texture(renderer, scene, &gt.detail_texture, &ground_dirs, images)
                                .map(|(id, path)| {
                                    tg.shared_textures.push(path);
                                    (id, gt.detail_repeats())
                                });
                            let gdirs: Vec<&Path> =
                                ground_dirs.iter().map(|p| p.as_path()).collect();
                            let gcfg = self.textures.cfg(&gt.texture, &gdirs);
                            let wet = gcfg.moisture || gcfg.puddles;
                            let m = renderer.add_terrain_layer_material(
                                scene,
                                layer_tex,
                                tex,
                                detail,
                                gt.repeats(),
                                lm,
                                if wet { 1.0 } else { 0.0 },
                            );
                            let m = gpu.material(renderer, scene, m);
                            tg.materials.push(m);
                            let li = instance!(renderer.add_surface_instance(
                                scene,
                                id,
                                p.origin,
                                Mat4::IDENTITY,
                                vec![m]
                            ));
                            if let Some(inst) = scene.instances.get_mut(li) {
                                inst.ground_layer = true;
                                inst.render_phase = RenderPhase::Terrain;
                            }
                            if omsi_cfg::env::var_os("OMSI_DEBUG_SURFACES").is_some() {
                                log::info!("tile ({}, {}): ground layer {layer} '{}' painted on {:.1} % of the tile, mask {:?}", p.tx, p.ty, gt.texture, painted * 100.0, mask.format);
                            }
                        }
                        // Exposed sides keep the original brush layers. The horizontal ground's
                        // masks include the hole cut and would erase these vertical faces again.
                        if let Some(wall) = wall_id {
                            for (layer, mask) in p.wall_paint.iter().filter(|_| !no_paint) {
                                let Some(gt) = self.global.ground_textures.get(*layer) else {
                                    continue;
                                };
                                let tex = gpu.add_data(renderer, scene, mask);
                                tg.textures.push(tex);
                                let layer_tex = gpu
                                    .texture(renderer, scene, &gt.texture, &ground_dirs, images)
                                    .map(|(id, path)| {
                                        tg.shared_textures.push(path);
                                        id
                                    });
                                let detail = gpu
                                    .texture(renderer, scene, &gt.detail_texture, &ground_dirs, images)
                                    .map(|(id, path)| {
                                        tg.shared_textures.push(path);
                                        (id, gt.detail_repeats())
                                    });
                                let gdirs: Vec<&Path> =
                                    ground_dirs.iter().map(|p| p.as_path()).collect();
                                let cfg = self.textures.cfg(&gt.texture, &gdirs);
                                let m = renderer.add_terrain_layer_material(
                                    scene,
                                    layer_tex,
                                    tex,
                                    detail,
                                    gt.repeats(),
                                    lm,
                                    if cfg.moisture || cfg.puddles { 1.0 } else { 0.0 },
                                );
                                let m = gpu.material(renderer, scene, m);
                                tg.materials.push(m);
                                let wi = instance!(renderer.add_surface_instance(
                                    scene,
                                    wall,
                                    p.origin,
                                    Mat4::IDENTITY,
                                    vec![m]
                                ));
                                if let Some(inst) = scene.instances.get_mut(wi) {
                                    inst.ground_layer = true;
                                    inst.render_phase = RenderPhase::Terrain;
                                }
                            }
                        }
                        // the tile's water surface: one quad at the four corner heights, drawn over the
                        // riverbed. OMSI keeps it in `tile.map.water`, one height per corner.
                        if let Some(h) = p.water {
                            let t = tile_size() as f32;
                            let mut wm = MeshData::default();
                            for (i, (x, y)) in [(0.0, 0.0), (t, 0.0), (0.0, t), (t, t)]
                                .into_iter()
                                .enumerate()
                            {
                                wm.positions.push(glam::Vec3::new(x, y, h[i]));
                                wm.normals.push(glam::Vec3::Z);
                                wm.uvs.push(glam::Vec2::new(x / 40.0, y / 40.0));
                            }
                            wm.indices.extend_from_slice(&[0, 1, 2, 2, 1, 3]);
                            wm.ranges.push((0, 6, 0));
                            let wid = gpu.add_mesh(renderer, scene, &wm);
                            tg.meshes.push(wid);
                            if omsi_cfg::env::var_os("OMSI_DEBUG_SURFACES").is_some() {
                                log::info!("tile ({}, {}): water at {:.1}..{:.1} m, centred ({:.0}, {:.0})", p.tx, p.ty, h.iter().cloned().fold(f32::MAX, f32::min), h.iter().cloned().fold(f32::MIN, f32::max), p.origin.x + tile_size() / 2.0, p.origin.y + tile_size() / 2.0);
                            }
                            // an ordinary instance, not a surface: the surface depth bias would let a
                            // tile-wide water quad win the depth test against the banks and flood the
                            // whole tile when seen at a shallow angle
                            let _ = instance!(renderer.add_instance(
                                scene,
                                wid,
                                p.origin,
                                Mat4::IDENTITY,
                                vec![water_mat]
                            ));
                        }
                    }
                    pl.phase = 1;
                    pl.next = 0;
                    done_some = true;
                }
                1 => {
                    if !only_object && pl.ground_next < p.ground_splines.len() {
                        let mesh = &p.ground_splines[pl.ground_next];
                        pl.ground_next += 1;
                        let id = gpu.add_mesh(renderer, scene, mesh);
                        scene.meshes[id].source = Some("terrain-mapped spline cells".to_string());
                        tg.meshes.push(id);
                        if let Some(mat) = pl.terrain_mapping_mat {
                            let si = instance!(renderer.add_surface_instance(scene, id, p.origin, Mat4::IDENTITY, vec![mat]));
                            if let Some(inst) = scene.instances.get_mut(si) {
                                inst.render_phase = RenderPhase::Spline;
                            }
                        }
                        done_some = true;
                        pl.secs[1] += t_phase.elapsed().as_secs_f64();
                        continue;
                    }
                    if only_object || pl.next >= p.splines.len() {
                        pl.phase = 2;
                        pl.next = 0;
                        continue;
                    }
                    let (mesh, st, casts_shadow, sort_origin) = &p.splines[pl.next];
                    pl.next += 1;
                    let skey = Arc::as_ptr(st) as usize;
                    if !gpu.splines.contains_key(&skey) {
                        let dirs = texture_dirs(&self.root, &st.dir);
                        let mut sg = SplineGpu {
                            _st: st.clone(),
                            materials: Vec::new(),
                            textures: Vec::new(),
                            users: 0,
                            terrain: Vec::new(),
                        };
                        for t in &st.def.textures {
                            let (tex, texture_has_alpha) =
                                match gpu.texture(renderer, scene, &t.file, &dirs, images) {
                                    Some((id, path)) => {
                                        let has_alpha = gpu.has_alpha(&path);
                                        sg.textures.push(path);
                                        (Some(id), has_alpha)
                                    }
                                    None => (None, false),
                                };
                            // OMSI spline [matl_alpha] uses 0 = opaque, 1 = alpha test,
                            // and 2 = blend.  Like the C++ handler, a declared blend on
                            // a texture without alpha is opaque; otherwise the surface
                            // belongs in the blended pass, not the depth-writing cutout
                            // pass.  Blended spline overlaps also need depth writes off,
                            // matching the reference handler's far-to-near spline pass.
                            let alpha = match (t.alpha, texture_has_alpha) {
                                (1, _) => AlphaMode::Test,
                                (mode, true) if mode >= 2 => AlphaMode::Blend,
                                _ => AlphaMode::Opaque,
                            };
                            let dirs_ref: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
                            let cfg = self.textures.cfg(&t.file, &dirs_ref);
                            let wet = cfg.moisture || cfg.puddles;
                            if cfg.terrain_mapping {
                                sg.terrain.push(sg.materials.len());
                            }
                            // (lit at night by the tile's light map, as OMSI lights the roads)
                            renderer.light_map_next.set(true);
                            let m = renderer.add_material_extra(
                                scene,
                                tex,
                                alpha,
                                [1.0; 4],
                                false,
                                None,
                                None,
                                None,
                                None,
                                [0.0; 3],
                                MaterialExtra {
                                    no_z_write: alpha == AlphaMode::Blend,
                                    moisture: if wet { 1.0 } else { 0.0 },
                                    ..MaterialExtra::default()
                                },
                            );
                            let m = gpu.material(renderer, scene, m);
                            sg.materials.push(m);
                        }
                        gpu.splines.insert(skey, sg);
                    }
                    let sg = gpu.splines.get_mut(&skey).unwrap();
                    if !tg.spline_types.contains(&skey) {
                        sg.users += 1;
                        tg.spline_types.push(skey);
                    }
                    let mats = if sg.materials.is_empty() {
                        vec![ground_mat]
                    } else {
                        sg.materials.clone()
                    };
                    if omsi_cfg::env::var_os("OMSI_DEBUG_SPLINES").is_some() {
                        let mean_nz = mesh.normals.iter().map(|n| n.z).sum::<f32>()
                            / mesh.normals.len().max(1) as f32;
                        log::info!("upload spline {} origin={:?} ranges={:?} mats={:?} mean normal z={mean_nz:+.2} verts={} first positions {:?}", st.def.path.display(), p.origin, &mesh.ranges[..mesh.ranges.len().min(3)], mats, mesh.positions.len(), &mesh.positions[..mesh.positions.len().min(3)]);
                    }
                    // [terrainmapping] slots take only the first ground texture. The
                    // spline mesh is already in tile space, which supplies the ground UVs.
                    let terrain: Vec<usize> = if pl.terrain_mapping_mat.is_none() {
                        Vec::new()
                    } else {
                        sg.terrain.iter().copied().filter(|t| mesh.ranges.iter().any(|r| r.2 as usize == *t)).collect()
                    };
                    if !terrain.is_empty() {
                        let ground = terrain_ground(mesh, &terrain, p.origin, Mat4::IDENTITY, p.origin);
                        let gid = gpu.add_mesh(renderer, scene, &ground);
                        scene.meshes[gid].source = Some(st.def.path.display().to_string());
                        tg.meshes.push(gid);
                        if let Some(mat) = pl.terrain_mapping_mat {
                            let terrain_instance = instance!(renderer.add_surface_instance(
                                scene,
                                gid,
                                p.origin,
                                Mat4::IDENTITY,
                                vec![mat],
                            ));
                            if let Some(inst) = scene.instances.get_mut(terrain_instance) {
                                inst.render_phase = spline_render_phase(&st.def);
                                inst.blend_sort_origin = Some(*sort_origin);
                            }
                        }
                    }
                    let rest = (!terrain.is_empty()).then(|| terrain_rest(mesh, &terrain));
                    let src = rest.as_ref().unwrap_or(mesh);
                    let (road, paint) = split_spline_paint(src, &st.def);
                    // Drawn where the map puts it and drawn over the ground by the surfaces'
                    // depth bias, as a road wins over flush ground in Omsi.exe. Lifted 8 cm
                    // instead, it stood over the ground the editor had aligned to it (the
                    // footways of Spandau's Hansastr. lie at the ground's height), and under
                    // every kerb and footway edge one saw into the hole cut beneath it (#823).
                    for (data, phase) in [(road, RenderPhase::Spline), (paint, RenderPhase::BeforeNormal)] {
                        if data.is_empty() { continue; }
                        let id = gpu.add_mesh(renderer, scene, &data);
                        tg.meshes.push(id);
                        scene.meshes[id].source = Some(st.def.path.display().to_string());
                        let si = instance!(renderer.add_surface_instance(
                            scene, id, p.origin, Mat4::IDENTITY, mats.clone()
                        ));
                        if let Some(inst) = scene.instances.get_mut(si) {
                            inst.render_phase = phase;
                            inst.blend_sort_origin = Some(*sort_origin);
                        }
                        // Preserve the deck's shadow geometry after splitting its slots.
                        if *casts_shadow { renderer.set_casts_shadow(scene, si, true); }
                    }
                    pl.splines += 1;
                    done_some = true;
                }
                2 => {
                    if only_object || pl.next >= p.trees.len() {
                        pl.phase = 3;
                        pl.next = 0;
                        continue;
                    }
                    // trees are cheap: a few dozen at a time
                    let end = (pl.next + 64).min(p.trees.len());
                    for (ot, texture, pos, height, width, heading) in &p.trees[pl.next..end] {
                        let tkey = texture.to_ascii_lowercase();
                        if !gpu.trees.contains_key(&tkey) {
                            let dirs = ot.texture_dirs(&self.root);
                            let found = gpu.texture(renderer, scene, texture, &dirs, images);
                            // (not repeated: the picture's bottom row, a wide trunk or grass, drew a line along the top of the card)
                            renderer.address_next.set(omsi_render::TexAddressing::Clamp);
                            let m = renderer.add_material_extra(
                                scene,
                                found.as_ref().map(|f| f.0),
                                AlphaMode::Test,
                                [1.0; 4],
                                false,
                                None,
                                None,
                                None,
                                None,
                                [0.0; 3],
                                omsi_render::MaterialExtra {
                                    tree: true,
                                    sway: Some(tree_card_sway(ot, texture)),
                                    ..Default::default()
                                },
                            );
                            let m = gpu.material(renderer, scene, m);
                            gpu.trees.insert(
                                tkey.clone(),
                                TreeGpu {
                                    material: m,
                                    texture: found.map(|f| f.1),
                                    users: 0,
                                },
                            );
                        }
                        let tr = gpu.trees.get_mut(&tkey).unwrap();
                        if !tg.trees.contains(&tkey) {
                            tr.users += 1;
                            tg.trees.push(tkey.clone());
                        }
                        let mat = tr.material;
                        let xf = Mat4::from_rotation_z((-heading).to_radians() as f32)
                            * Mat4::from_scale(glam::Vec3::new(
                                *width as f32,
                                *width as f32,
                                *height as f32,
                            ));
                        let _ =
                            instance!(renderer.add_instance(scene, tree_mesh, *pos, xf, vec![mat]));
                        pl.trees += 1;
                    }
                    pl.next = end;
                    done_some = true;
                }
                _ => {
                    let Some(o) = p.objects.pop() else {
                        pl.phase = 4;
                        continue;
                    };
                    let PlacedObject {
                        ot,
                        pos,
                        xf,
                        lamp,
                        map_id,
                        key: collision_key,
                        controller,
                        strings,
                        warped,
                        var_parent,
                        parked,
                        editable,
                        script: mut early_script,
                    } = o;
                    let script_strings: &[String] = match (&lamp, strings.as_slice()) {
                        (&Some((_, _, false)), [_, rest @ ..]) => rest,
                        _ => strings.as_slice(),
                    };
                    let tkey = self.type_gpu(renderer, scene, gpu, &ot, images, ground_mat);
                    if !tg.types.contains(&tkey) {
                        gpu.types.get_mut(&tkey).unwrap().users += 1;
                        tg.types.push(tkey);
                    }
                    let (type_meshes, type_variants, mut type_lods, type_auto_night, lod0_lo, lod0_max, terrain_slots) = {
                        let t = &gpu.types[&tkey];
                        (t.meshes.clone(), t.variants.clone(), t.lods.clone(), t.auto_night, t.lod0_lo, t.lod0_max, t.terrain_slots.clone())
                    };
                    let surface =
                        ot.sco.render_type.is_ground_layer()
                            || ot.sco.surface;
                    let render_phase = scenery_render_phase(ot.sco.render_type);
                    let has_lower = !type_lods.is_empty();
                    let mut lamp_instances = Vec::new();
                    let mut lamp_slots = Vec::new();
                    let mut all_instances = Vec::new();
                    let mut object_variants: Vec<(usize, usize, MaterialId, MaterialId, String)> =
                        Vec::new();
                    let mut script_texts: Vec<(TextureId, omsi_sim::texttex::TextTextureState)> =
                        Vec::new();
                    let mut lamp_texts: Vec<(TextureId, omsi_sim::texttex::TextTextureState)> =
                        Vec::new();
                    // The materials this placement's own `[texttexture]`s made: (slot, is item, material),
                    // to keep them on a `[matl_change]` slot (see below).
                    let mut text_slot_mats: Vec<(usize, bool, MaterialId)> = Vec::new();
                    // `[htmltexture]` pages shown on this object: (script texture index, texture)
                    let mut html_pages: Vec<(usize, TextureId)> = Vec::new();
                    let mut html_mats: HashMap<usize, MaterialId> = HashMap::new();
                    // Run {init} once for this placement: its variable values choose CTC
                    // schemes and its strings can name [matl_freetex] pictures.
                    let needs_own_script = lamp.is_none()
                        || ot.meshes.iter().any(|(_, _, overrides)| {
                            overrides.iter().any(|o| !o.item && o.freetex.is_some())
                        });
                    // (a model with `[htmltexture]` pages or `[matl_freetex]` needs a script instance to feed them,
                    // also when the object has no script of its own)
                    let has_pages = lamp.is_none() && !ot.model.html_textures.is_empty();
                    let has_freetex = ot.meshes.iter().any(|(_, _, overrides)| {
                        overrides.iter().any(|o| !o.item && o.freetex.is_some())
                    });
                    let mut object_script = if needs_own_script {
                        let program = ot.program.clone().or_else(|| {
                            (has_pages || (has_freetex && !script_strings.is_empty())).then(|| Arc::new(omsi_script::Program::default()))
                        });
                        program.map(|program| {
                            let mut inst = early_script.take().unwrap_or_else(|| omsi_sim::scenery::SceneryInstance::new(
                                program,
                                &ot.mesh_defs(),
                                self.script_clock(),
                                script_strings,
                            ));
                            if has_pages {
                                let object_dir = ot.sco.path.parent().unwrap_or(std::path::Path::new(""));
                                inst.init_html_textures(&ot.model.html_textures, &ot.model_dir, object_dir);
                            }
                            inst
                        })
                    } else {
                        None
                    };
                    // Some signs derive filenames in {frame}. Probe on a separate
                    // instance: its placeholder inputs must not mutate the live script
                    // state or retain queued sounds/animations.
                    let freetex_probe = if ot.meshes.iter().any(|(_, _, overrides)| {
                        overrides.iter().any(|o| !o.item && o.freetex.is_some())
                    }) {
                        ot.program.as_ref().map(|program| {
                            let mut probe = omsi_sim::scenery::SceneryInstance::new(
                                program.clone(), &ot.mesh_defs(), self.script_clock(), script_strings,
                            );
                            probe.update(0.0, &omsi_sim::scenery::SceneryVars {
                                in_use: 1.0, ..Default::default()
                            });
                            probe
                        })
                    } else { None };
                    // a crossing warped onto the ground has meshes of its own
                    let own_meshes: Option<Vec<(MeshId, Vec<MaterialId>)>> = warped.as_ref().map(|ms| {
                        ms.iter()
                            .zip(type_meshes.iter())
                            .map(|(m, (_, mats))| {
                                let id = gpu.add_mesh(renderer, scene, m);
                                scene.meshes[id].source = Some(ot.sco.path.display().to_string());
                                tg.meshes.push(id);
                                (id, mats.clone())
                            })
                            .collect()
                    });
                    let mut mesh_list: Vec<(MeshId, Vec<MaterialId>)> =
                        own_meshes.unwrap_or_else(|| type_meshes.clone());
                    // [terrainmapping] slots: drawn with the uncut base ground, from a mesh
                    // of this placement's own (see split_terrain_mapped); (level, mesh, id)
                    let mut ground_meshes: Vec<(usize, usize, MeshId)> = Vec::new();
                    if pl.terrain_mapping_mat.is_some() {
                        let mut parts: Vec<(usize, usize)> =
                            terrain_slots.iter().map(|t| (t.0, t.1)).collect();
                        parts.dedup();
                        for (level, mi) in parts {
                            let slots: Vec<usize> = terrain_slots
                                .iter()
                                .filter(|t| (t.0, t.1) == (level, mi))
                                .map(|t| t.2)
                                .collect();
                            let src = if level == 0 {
                                warped.as_ref().and_then(|w| w.get(mi)).or(ot.meshes.get(mi).map(|m| &m.0))
                            } else {
                                ot.lower_lods.get(level - 1).and_then(|l| l.1.get(mi)).map(|m| &m.0)
                            };
                            let Some(src) = src else { continue };
                            let ground = terrain_ground(src, &slots, pos, xf, p.origin);
                            if ground.is_empty() {
                                continue;
                            }
                            // (a crossing warped onto the ground has a mesh of its own; the
                            // rest of every other object is the same for all its placements)
                            let rest_id = if level == 0 && warped.is_some() {
                                let id = gpu.add_mesh(renderer, scene, &terrain_rest(src, &slots));
                                scene.meshes[id].source = Some(ot.sco.path.display().to_string());
                                tg.meshes.push(id);
                                id
                            } else if let Some(&(_, id)) = gpu.types[&tkey].terrain_rest.iter().find(|r| r.0 == (level, mi)) {
                                id
                            } else {
                                let id = gpu.add_mesh(renderer, scene, &terrain_rest(src, &slots));
                                scene.meshes[id].source = Some(ot.sco.path.display().to_string());
                                if let Some(t) = gpu.types.get_mut(&tkey) {
                                    t.terrain_rest.push(((level, mi), id));
                                }
                                id
                            };
                            let slot = if level == 0 {
                                mesh_list.get_mut(mi)
                            } else {
                                type_lods.get_mut(level - 1).and_then(|l| l.2.get_mut(mi))
                            };
                            if let Some(slot) = slot {
                                slot.0 = rest_id;
                            }
                            let ground_id = gpu.add_mesh(renderer, scene, &ground);
                            scene.meshes[ground_id].source = Some(ot.sco.path.display().to_string());
                            tg.meshes.push(ground_id);
                            ground_meshes.push((level, mi, ground_id));
                        }
                    }
                    for (mi, (mesh_id, mats)) in mesh_list.iter().enumerate() {
                        let inst = if surface || ot.mesh_shadow.get(mi).copied().unwrap_or(false) {
                            let i = instance!(renderer.add_surface_instance(
                                scene,
                                *mesh_id,
                                pos,
                                xf,
                                mats.clone()
                            ));
                            i
                        } else {
                            let i = instance!(renderer.add_instance(scene, *mesh_id, pos, xf, mats.clone()));
                            renderer.set_omsi_caster(scene, i, ot.mesh_casts.get(mi).copied().unwrap_or(false));
                            i
                        };
                        if let Some(inst) = scene.instances.get_mut(inst) {
                            inst.render_phase = render_phase;
                            if surface {
                                // an object lying on the road (a crossing, markings, a zebra)
                                // goes over the splines it overlaps
                                inst.decal = true;
                            } else if ot.paint {
                                // and so does paint made as a plain object, drawn as the
                                // markings are: with the roads' depth bias and a little more
                                inst.decal = true;
                                inst.surface_bias = true;
                            }
                        }
                        // Scenery signs use [matl_freetex] with a string from the map
                        // object's [object] / [splineAttachement] record. The type's
                        // material is shared, so make a material for this placement only.
                        // (the map's strings are the object's string variables, and its
                        // {init} may make the file name of them: read after it has run)
                        if let Some((_, o3d_mats, overrides)) = ot.meshes.get(mi) {
                            for override_ in overrides.iter().filter(|o| !o.item && o.freetex.is_some()) {
                                let Some(slot) = omsi_sim::vehicle::override_slot(o3d_mats, override_) else { continue };
                                let Some((_, var)) = &override_.freetex else { continue };
                                let Some(name) = resolve_scenery_freetex_name(
                                    var,
                                    override_,
                                    overrides,
                                    object_script.as_ref(),
                                    freetex_probe.as_ref(),
                                    script_strings,
                                ) else {
                                    continue;
                                };
                                let dirs = ot.texture_dirs(&self.root);
                                let Some((tex, path)) = gpu.texture(renderer, scene, name, &dirs, images) else { continue };
                                let Some(base) = mats.get(slot).and_then(|id| scene.materials.get(*id)) else {
                                    gpu.release_texture(renderer, scene, &path);
                                    continue;
                                };
                                let (alpha, color, unlit, transmap, night, light, env, emissive) =
                                    (base.alpha, base.color, base.unlit, base.transmap, base.nightmap, base.lightmap, base.envmap, base.emissive);
                                let slot_ov: Vec<&MaterialDef> = overrides.iter().filter(|o| !o.item && omsi_sim::vehicle::override_slot(o3d_mats, o) == Some(slot)).collect();
                                let mut extra = material_extra(&slot_ov, base.env_mask, base.bump, [0.0; 4]);
                                extra.ambient = o3d_mats.get(slot).map(|m| d3d_material(m, slot_ov.iter().find_map(|o| o.allcolor), true).3);
                                renderer.address_next.set(tex_addressing(slot_ov.iter().copied()));
                                let mat = renderer.add_material_extra(scene, Some(tex), alpha, color, unlit, transmap, night, light, env, emissive, extra);
                                let mat = gpu.material(renderer, scene, mat);
                                tg.materials.push(mat);
                                tg.shared_textures.push(path);
                                renderer.set_material(scene, inst, slot, mat);
                            }
                        }
                        // (only where the lower levels are drawn instead: a scripted object
                        // or a lamp keeps its first level, which alone the script poses -
                        // limited as well, it vanished when small, with nothing in its place)
                        if has_lower && !surface && lamp.is_none() && ot.program.is_none() {
                            renderer.set_lod_range(scene, inst, lod0_lo, lod0_max);
                        }
                        // [matl_change] variants of this mesh
                        for (_, slot, base, item, var) in type_variants.iter().filter(|v| v.0 == mi)
                        {
                            // Traffic lamps are updated by Traffic::sync; keep their
                            // switches even when no custom script was loaded.
                            if lamp.is_some() || ot.program.is_some() {
                                object_variants.push((inst, *slot, *base, *item, var.clone()));
                            } else if var.trim().eq_ignore_ascii_case("NightlightA") {
                                pl.night_slots.push((inst, *slot, *item, *base));
                            } else if var.trim().parse::<f32>().map(|x| x > 0.5).unwrap_or(false) {
                                renderer.set_material(scene, inst, *slot, *item);
                            }
                        }
                        if lamp.is_some() {
                            lamp_instances.push((inst, ot.mesh_visible.get(mi).cloned().flatten()));
                            lamp_slots.push(
                                ot.meshes
                                    .get(mi)
                                    .map(|(_, o3d_mats, overrides)| LampSlots::of_mesh(o3d_mats, overrides, mats.len()))
                                    .unwrap_or_default(),
                            );
                        }
                        if type_auto_night && (2..=4).contains(&ot.sco.night_map_mode) {
                            // each house its own hours (OMSI draws them once per object)
                            pl.night_modes.push(NightMode { inst, use_: InUse::new(ot.sco.night_map_mode, map_id as u64), slots: mats.len().max(1) });
                        }
                        // [texttexture] + [useTextTexture]: street names etc. from the map strings
                        if !ot.model.text_textures.is_empty() {
                            if let Some((_, o3d_mats, overrides)) = ot.meshes.get(mi) {
                                for o in overrides.iter().filter(|o| o.use_text_texture.is_some()) {
                                    let (Some(slot), Some(tt)) = (
                                        omsi_sim::vehicle::override_slot(o3d_mats, o),
                                        ot.model
                                            .text_textures
                                            .get(o.use_text_texture.unwrap().max(0) as usize),
                                    ) else {
                                        continue;
                                    };
                                    // a script's string variable (the stock bus stop display's
                                    // departures): a texture of the object's own, drawn by
                                    // `update_scripted` whenever the script refreshes it
                                    let scripted_text = tt.variable.trim().parse::<usize>().is_err()
                                        && ot
                                            .program
                                            .as_ref()
                                            .map(|p| p.str_var(tt.variable.trim()).is_some())
                                            .unwrap_or(false);
                                    if scripted_text {
                                        let atlas = self.fonts.lock().get(&tt.font, &|p| {
                                            omsi_texture::decode_file(p)
                                                .ok()
                                                .map(|i| (i.width, i.height, i.rgba))
                                        });
                                        let state = omsi_sim::texttex::TextTextureState::new(
                                            tt.clone(),
                                            atlas,
                                        );
                                        let (w, h) =
                                            (tt.width.max(1) as u32, tt.height.max(1) as u32);
                                        let tex = gpu.add_blank_mips(renderer, scene, w, h);
                                        let mat = renderer.add_material(
                                            scene,
                                            Some(tex),
                                            AlphaMode::Blend,
                                            [1.0; 4],
                                            true,
                                        );
                                        let mat = gpu.material(renderer, scene, mat);
                                        tg.textures.push(tex);
                                        tg.materials.push(mat);
                                        text_slot_mats.push((slot, o.item, mat));
                                        renderer.set_material(scene, inst, slot, mat);
                                        if lamp.is_none() {
                                            script_texts.push((tex, state));
                                        } else {
                                            lamp_texts.push((tex, state));
                                        }
                                        continue;
                                    }
                                    let text = tt
                                        .variable
                                        .trim()
                                        .parse::<usize>()
                                        .ok()
                                        .and_then(|k| script_strings.get(k))
                                        .cloned()
                                        .unwrap_or_default();
                                    let alpha = text_alpha(o3d_mats, slot, overrides);
                                    let slot_ov: Vec<&MaterialDef> = overrides.iter().filter(|o| !o.item && omsi_sim::vehicle::override_slot(o3d_mats, o) == Some(slot)).collect();
                                    let key = text_material_key(scenery_text_key(tt, &text, alpha), &slot_ov);
                                    if let Some(e) = gpu.text_textures.get_mut(&key) {
                                        e.2 += 1;
                                        let mat = e.1;
                                        tg.texts.push(key);
                                        text_slot_mats.push((slot, o.item, mat));
                                        renderer.set_material(scene, inst, slot, mat);
                                        continue;
                                    }
                                    let atlas = self.fonts.lock().get(&tt.font, &|p| {
                                        omsi_texture::decode_file(p)
                                            .ok()
                                            .map(|i| (i.width, i.height, i.rgba))
                                    });
                                    // drawn as they are: the street name signs that seemed to want
                                    // their text turned by 180° were `.x` meshes whose frames were
                                    // read transposed (upside down), the stop name plates are not
                                    // (a route arrow's name in letters its font lacks: as on the
                                    // game's own arrows)
                                    let helper = if ot.sco.is_help_arrow { helper_text_image(tt, atlas.as_deref(), &text) } else { None };
                                    let image = helper.unwrap_or_else(|| scenery_text_image(tt, atlas, &text));
                                    let tex = gpu.add_image(renderer, scene, &image, true);
                                    let mat = text_material(renderer, scene, tex, alpha, &slot_ov);
                                    let mat = gpu.material(renderer, scene, mat);
                                    gpu.text_textures.insert(key.clone(), (tex, mat, 1));
                                    tg.texts.push(key);
                                    text_slot_mats.push((slot, o.item, mat));
                                    renderer.set_material(scene, inst, slot, mat);
                                }
                            }
                        }
                        // A slot a `[matl_change]` switches keeps the material this
                        // placement's own `[texttexture]` drew it with: the switch puts the
                        // type's plain materials back every frame (`update_scripted`), and a
                        // bus stop sign's route numbers went blank with them (#1756).
                        for v in object_variants.iter_mut().filter(|v| v.0 == inst) {
                            if let Some(&(_, _, m)) = text_slot_mats.iter().rev().find(|(s, it, _)| *s == v.1 && *it) {
                                v.3 = m;
                            }
                            if let Some(&(_, _, m)) = text_slot_mats.iter().rev().find(|(s, it, _)| *s == v.1 && !*it) {
                                v.2 = m;
                            }
                        }
                        // [htmltexture] + [useHtmlTexture]: a page drawn onto the slot; the
                        // pictures come from `update_scripted`
                        if has_pages && object_script.is_some() {
                            if let Some((_, o3d_mats, overrides)) = ot.meshes.get(mi) {
                                for o in overrides.iter().filter(|o| !o.item) {
                                    let Some(page) = o.use_script_texture.map(|n| n.max(0) as usize) else { continue };
                                    let Some(def) = ot.model.html_textures.iter().find(|d| d.script_index == page) else { continue };
                                    let Some(slot) = omsi_sim::vehicle::override_slot(o3d_mats, o) else { continue };
                                    let mat = match html_mats.get(&page) {
                                        Some(m) => *m,
                                        None => {
                                            let (w, h) = (def.width.max(1) as u32, def.height.max(1) as u32);
                                            let tex = gpu.add_image(
                                                renderer,
                                                scene,
                                                // (black until the page first draws: a page far away starts later)
                                                &Image { width: w, height: h, rgba: [0, 0, 0, 255].repeat((w * h) as usize), has_alpha: true },
                                                false,
                                            );
                                            let mat = renderer.add_material(scene, Some(tex), text_alpha(o3d_mats, slot, overrides), [1.0; 4], true);
                                            let mat = gpu.material(renderer, scene, mat);
                                            tg.textures.push(tex);
                                            tg.materials.push(mat);
                                            html_pages.push((page, tex));
                                            html_mats.insert(page, mat);
                                            mat
                                        }
                                    };
                                    renderer.set_material(scene, inst, slot, mat);
                                }
                            }
                        }
                        all_instances.push(inst);
                    }
                    let mut lod_instances = Vec::new();
                    // (the meshes' own instances: a script poses them one by one, the ground
                    // drawn in the [terrainmapping] slots after them keeps the object's place)
                    let mesh_instances = all_instances.len();
                    let lod_drawn = has_lower && !surface && lamp.is_none() && ot.program.is_none();
                    for &(level, _, ground_id) in &ground_meshes {
                        // the first level without lower ones is drawn at any size
                        let range = if level == 0 {
                            lod_drawn.then_some((lod0_lo, lod0_max))
                        } else if lod_drawn {
                            type_lods.get(level - 1).map(|l| (l.0, l.1))
                        } else {
                            continue;
                        };
                        // Keep the first ground texture on the object even where the map
                        // author painted asphalt or another layer on the terrain below it.
                        if let Some(mat) = pl.terrain_mapping_mat {
                            let inst = if surface {
                                instance!(renderer.add_surface_instance(scene, ground_id, pos, xf, vec![mat]))
                            } else {
                                instance!(renderer.add_instance(scene, ground_id, pos, xf, vec![mat]))
                            };
                            if let Some(x) = scene.instances.get_mut(inst) {
                                x.decal = surface;
                                x.render_phase = render_phase;
                            }
                            if let Some((lo, hi)) = range {
                                renderer.set_lod_range(scene, inst, lo, hi);
                            }
                            if level == 0 {
                                all_instances.push(inst);
                            } else {
                                lod_instances.push(inst);
                            }
                        }
                    }
                    if lod_drawn {
                        for (min_size, max_size, meshes) in &type_lods {
                            for (mesh_id, mats) in meshes {
                                let inst = instance!(renderer.add_instance(
                                    scene,
                                    *mesh_id,
                                    pos,
                                    xf,
                                    mats.clone()
                                ));
                                renderer.set_lod_range(scene, inst, *min_size, *max_size);
                                if let Some(x) = scene.instances.get_mut(inst).filter(|_| ot.paint) {
                                    x.decal = true;
                                    x.surface_bias = true;
                                }
                                lod_instances.push(inst);
                            }
                        }
                    }
                    // the object is drawn, left out and switched to another LOD as one
                    // (performance_minObjSize, performance_maxObjDist, [detail_factor],
                    // [noDistanceCheck]); its sphere about its origin holds every level
                    {
                        let radius = mesh_list
                            .iter()
                            .map(|m| m.0)
                            .chain(type_lods.iter().flat_map(|l| l.2.iter().map(|m| m.0)))
                            .filter_map(|id| scene.meshes.get(id))
                            .filter(|m| m.bounds_radius > 0.0)
                            .map(|m| m.bounds_center.length() + m.bounds_radius)
                            .fold(0.0f32, f32::max);
                        let detail = if ot.sco.model.detail_factor != 1.0 {
                            ot.sco.model.detail_factor
                        } else {
                            ot.model.detail_factor
                        };
                        let any_distance = ot.sco.model.no_distance_check
                            || ot.model.no_distance_check
                            || ot.model.meshes.iter().any(|m| m.no_distance_check);
                        let near_only = stand_in_area(&ot, &xf, pos, (p.tx, p.ty));
                        for inst in all_instances.iter().chain(&lod_instances) {
                            scene.instances[*inst].presurface =
                                ot.sco.render_type == omsi_scenery::sco::RenderType::PreSurface;
                            renderer.set_object_culling(scene, *inst, radius, detail, any_distance);
                            renderer.set_near_only(scene, *inst, near_only);
                        }
                    }
                    if ot.sco.crash_mode_pole.is_some() && !ot.sco.no_collision {
                        let instances: Vec<usize> = all_instances
                            .iter()
                            .chain(&lod_instances)
                            .copied()
                            .collect();
                        // knocked over before the tile went away: it lies where it fell
                        if let Some(push) = self.fallen_poles.lock().get(&collision_key) {
                            let fallen = fallen_pole(xf, *push);
                            for inst in &instances {
                                renderer.set_transform(scene, *inst, pos, fallen);
                            }
                        }
                        self.poles
                            .lock()
                            .insert(collision_key, (pos, xf, instances));
                        pl.poles.push(collision_key);
                    }
                    if ot.sco.is_help_arrow {
                        // A route arrow the map's author put up: Omsi.exe draws its `[helparrow]`
                        // objects (type 8) only while its route arrows are on (0x78e4b8; the
                        // game menu's button switches them, 0x686e3c). Left out for good, the
                        // stock maps' arrows to Grundorf's hospital and round Spandau's
                        // junctions never showed (#954).
                        let instances: Vec<usize> = all_instances.iter().chain(&lod_instances).copied().collect();
                        let shown = self.help_arrows_shown.load(std::sync::atomic::Ordering::Relaxed);
                        for inst in &instances {
                            // (no shadow, as the game's own arrows)
                            renderer.set_casts_shadow(scene, *inst, false);
                            if !shown {
                                hide_instance(renderer, scene, *inst);
                            }
                        }
                        self.help_arrows.lock().entry(key).or_default().extend(instances);
                    }
                    if parked {
                        let instances: Vec<usize> = all_instances.iter().chain(&lod_instances).copied().collect();
                        if self.departed.lock().contains(&collision_key) {
                            for inst in &instances {
                                hide_instance(renderer, scene, *inst);
                            }
                        } else {
                            self.parked_objects.lock().insert(
                                collision_key,
                                ParkedObject { tile: key, pos, heading: Pose { pos, rot: xf }.heading(), sco: ot.sco.path.clone(), instances },
                            );
                        }
                    }
                    if editable {
                        let instances: Vec<usize> = all_instances.iter().chain(&lod_instances).copied().collect();
                        let eo = EditObject { tile: key, pos, xf, key: collision_key, instances, sco: ot.sco.path.clone() };
                        // an object edited before its tile went shows the edit again
                        if let Some(e) = self.object_edits.lock().get(&map_id).copied() {
                            show_edit(renderer, scene, &eo, e);
                        }
                        self.edit_objects.lock().insert(map_id, eo);
                    }
                    if let Some((parent, index, any_light)) = lamp {
                        let names: Mutex<Vec<LightSwitch>> = Mutex::new(Vec::new());
                        let lights = model_lights_owned(&ot.model, &|_| xf, pos, &|var| {
                            names.lock().push(LightSwitch::parse(var));
                            1.0
                        }, &[]);
                        let names = names.into_inner();
                        let sources = model_light_sources(&ot.model);
                        let (coronas, corona_mesh): (Vec<(omsi_render::Corona, String)>, Vec<(usize, glam::Vec3, glam::Vec3)>) = lights
                            .into_iter()
                            .filter_map(|(c, k)| match names.get(k) {
                                Some(LightSwitch::Variable(v)) => Some(((c, v.clone()), sources.get(k).copied().unwrap_or((0, glam::Vec3::ZERO, glam::Vec3::ZERO)))),
                                _ => None,
                            })
                            .unzip();
                        let script = ot.program.as_ref().map(|p| {
                            Arc::new(Mutex::new(omsi_sim::scenery::SceneryInstance::new(
                                p.clone(),
                                &ot.mesh_defs(),
                                self.script_clock(),
                                script_strings,
                            )))
                        });
                        let lit = vec![0.0; coronas.len()];
                        let animated = script.as_ref().map(|s| s.lock().animated()).unwrap_or(false);
                        let sound = ot.sco.sound.as_ref().map(|rel| {
                            let dir = ot.sco.path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                            omsi_cfg::resolve_path(&dir, rel)
                        });
                        pl.light_objects.push(LightObject {
                            parent,
                            index,
                            any_light,
                            instances: lamp_instances,
                            slots: lamp_slots,
                            variants: object_variants,
                            pos,
                            script,
                            coronas,
                            corona_mesh,
                            lit,
                            xf,
                            animated,
                            sound,
                            sounds: Default::default(),
                            shown: None,
                            texts: lamp_texts,
                        });
                    } else if let Some(inst) = object_script.take() {
                        let texture_selection = scenery_texture_selection(&ot, &inst);
                        if !ot.dynamic_textures.is_empty() {
                            if let Some(rows) = gpu.dynamic_texture_variant(
                                renderer,
                                scene,
                                tkey,
                                &texture_selection,
                                &self.root,
                                images,
                            ) {
                                for (mi, row) in rows.iter().enumerate() {
                                    let Some(&mesh_inst) = all_instances.get(mi) else {
                                        continue;
                                    };
                                    for (slot, pair) in row.iter().enumerate() {
                                        let Some((base, item)) = pair else { continue };
                                        let item_on = object_variants
                                            .iter()
                                            .find(|v| v.0 == mesh_inst && v.1 == slot)
                                            .map(|v| {
                                                v.4.trim()
                                                    .parse::<f32>()
                                                    .ok()
                                                    .or_else(|| inst.var(&v.4))
                                                    .is_some_and(change_picks_item)
                                            })
                                            .unwrap_or(false);
                                        renderer.set_material(
                                            scene,
                                            mesh_inst,
                                            slot,
                                            if item_on { *item } else { *base },
                                        );
                                    }
                                }
                            }
                        }
                        // `[alphascale]` on the object's own slots (#1299: only the traffic
                        // lights' were read, a bus stop sign faded by its script stood there
                        // whole): as its script's {init} leaves the variables, and then
                        // every frame for a script that changes them
                        let alpha_slots: Vec<LampSlots> = if lamp.is_none() {
                            all_instances
                                .iter()
                                .take(mesh_instances)
                                .enumerate()
                                .map(|(mi, &id)| {
                                    let count = scene.instances.get(id).map(|i| i.materials.len()).unwrap_or(0);
                                    let mut l = ot.meshes.get(mi).map(|(_, o3d_mats, overrides)| LampSlots::of_mesh(o3d_mats, overrides, count)).unwrap_or_default();
                                    l.light.clear();
                                    l
                                })
                                .collect()
                        } else {
                            Vec::new()
                        };
                        let faded = alpha_slots.iter().any(|l| !l.alpha.is_empty());
                        let mut alpha_last = Vec::new();
                        if faded {
                            for (l, &id) in alpha_slots.iter().zip(&all_instances) {
                                let (a, _) = l.values(&|v| v.trim().parse::<f32>().ok().or_else(|| inst.var(v)));
                                let visible = scene.instances[id].visible;
                                renderer.set_params(scene, id, &a, visible, &[]);
                                alpha_last.push(a);
                            }
                        }
                        if inst.is_dynamic()
                            || !object_variants.is_empty()
                            || ot.sco.sound.is_some()
                            || !script_texts.is_empty()
                            || !html_pages.is_empty()
                            || !ot.dynamic_textures.is_empty()
                        {
                            let arrivals = inst.wants_arrivals();
                            // (a scripted object with [terrainmapping] slots had more instances
                            // than its script has meshes: "index out of bounds", #111)
                            all_instances.truncate(mesh_instances);
                            self.scripted.lock().push(ScriptedObject {
                                ty: ot.clone(),
                                pos,
                                xf,
                                instances: all_instances,
                                inst,
                                controller,
                                light_index: 0,
                                light_parent: if controller.is_none() && lamp.is_none() {
                                    light_child_of(&self.index().traffic_light_parents, var_parent, &strings)
                                } else {
                                    None
                                },
                                map_id,
                                variants: object_variants,
                                sounds: None,
                                tile: key,
                                var_parent,
                                texts: script_texts,
                                arrivals,
                                htmls: html_pages,
                                alpha_slots: if faded { alpha_slots } else { Vec::new() },
                                alpha_last,
                            });
                        }
                    }
                    pl.objects += 1;
                    done_some = true;
                }
            }
            pl.secs[phase.min(3) as usize] += t_phase.elapsed().as_secs_f64();
        }
        let done = pl.phase >= 4;
        if omsi_cfg::env::var_os("OMSI_PROFILE").is_some() {
            let took = t_start.elapsed().as_secs_f64();
            let decodes = (
                gpu.sync_decodes - decodes_before.0,
                gpu.sync_decode_secs - decodes_before.1,
            );
            if took + lock_wait > 0.02 || decodes.0 > 0 {
                log::info!(
                    "place tile ({}, {}): {:.1} ms this step (+{:.1} ms waiting for the GPU cache), {} textures decoded here in {:.1} ms; so far ground {:.1} ms, {} splines {:.1} ms, {} trees {:.1} ms, {} objects {:.1} ms{}",
                    key.0,
                    key.1,
                    took * 1000.0,
                    lock_wait * 1000.0,
                    decodes.0,
                    decodes.1 * 1000.0,
                    pl.secs[0] * 1000.0,
                    pl.splines,
                    pl.secs[1] * 1000.0,
                    pl.trees,
                    pl.secs[2] * 1000.0,
                    pl.objects,
                    pl.secs[3] * 1000.0,
                    if done { ", done" } else { "" }
                );
            }
        }
        done
    }

    /// Lay the `[crashmode_pole]` post `key` on the ground from its foot, fallen the way it
    /// was pushed (`push`), and remember that for when its tile comes back.
    pub fn lay_down_pole(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        key: i64,
        push: DVec3,
    ) -> Option<DVec3> {
        self.fallen_poles.lock().insert(key, push);
        let (pos, xf, instances) = self.poles.lock().get(&key).cloned()?;
        let fallen = fallen_pole(xf, push);
        for inst in instances {
            renderer.set_transform(scene, inst, pos, fallen);
        }
        Some(pos)
    }

    /// Draw the route arrows the map's author put up (`[helparrow]` objects) or hide them:
    /// `on` is whether OMSI 2's route arrows are on (the `nav_arrows` setting). Nothing to
    /// do when that has not changed; the tiles placed later follow it.
    pub fn show_help_arrows(&self, renderer: &Renderer, scene: &mut Scene, on: bool) {
        if self.help_arrows_shown.swap(on, std::sync::atomic::Ordering::Relaxed) == on {
            return;
        }
        for inst in self.help_arrows.lock().values().flatten() {
            let Some(i) = scene.instances.get(*inst) else { continue };
            let (alpha, uv) = (i.slot_alpha.clone(), i.slot_uv.clone());
            renderer.set_params(scene, *inst, &alpha, on, &uv);
        }
    }

    /// Parked car `key` drives off: it is hidden, its box leaves the obstacles, and its space
    /// stays empty for the rest of the run. What it was, for the car that takes its place.
    pub fn depart_parked(&self, renderer: &Renderer, scene: &mut Scene, key: i64) -> Option<ParkedObject> {
        let p = self.parked_objects.lock().remove(&key)?;
        self.departed.lock().insert(key);
        for inst in &p.instances {
            hide_instance(renderer, scene, *inst);
        }
        let (mut obst, mut boxes) = (Vec::new(), Vec::new());
        if let Some(st) = self.tile_state.lock().get_mut(&p.tile) {
            obst = st.obstacles.iter().filter(|b| b.id == key).cloned().collect();
            boxes = st.parked_boxes.iter().filter(|b| b.id == key).cloned().collect();
            st.obstacles.retain(|b| b.id != key);
            st.parked_boxes.retain(|b| b.id != key);
        }
        self.departed_objects.lock().insert(key, (p.clone(), obst, boxes));
        self.refresh_tile_lists();
        Some(p)
    }

    /// The parking spaces whose cars have driven off (LAN host: the clients take the same
    /// cars away).
    pub fn departed_keys(&self) -> Vec<i64> {
        let mut k: Vec<i64> = self.departed.lock().iter().copied().collect();
        k.sort_unstable();
        k
    }

    /// LAN client: the parked cars as the host has them - the spaces it lists empty, and
    /// (when the list is `complete`) every other one taken again. A space on a tile not
    /// loaded here yet is remembered: the tile comes up with it empty.
    pub fn mirror_departed(&self, renderer: &Renderer, scene: &mut Scene, keys: &[i64], complete: bool) {
        let before = self.departed.lock().len();
        for &k in keys {
            if !self.departed.lock().contains(&k) && self.depart_parked(renderer, scene, k).is_none() {
                self.departed.lock().insert(k);
            }
        }
        if complete {
            let back: Vec<i64> = self.departed.lock().iter().copied().filter(|k| !keys.contains(k)).collect();
            for k in back {
                if !self.return_parked(renderer, scene, k) {
                    self.departed.lock().remove(&k);
                }
            }
        }
        let after = self.departed.lock().len();
        if after != before {
            log::info!("LAN: parked cars as the host has them: {after} spaces empty (were {before})");
        }
    }

    /// Parked cars standing where `b` is (the player's bus just put down at a depot's entry
    /// point, over a parked bus): they go, as parked cars that drive off do.
    pub fn clear_parked_under(&self, renderer: &Renderer, scene: &mut Scene, b: &omsi_sim::collision::Obb) -> usize {
        let keys: Vec<i64> = self
            .tile_state
            .lock()
            .values()
            .flat_map(|st| st.parked_boxes.iter().filter(|p| p.overlaps_plan(b)).map(|p| p.id).collect::<Vec<_>>())
            .collect();
        let mut n = 0;
        for k in keys {
            if self.depart_parked(renderer, scene, k).is_some() {
                n += 1;
            }
        }
        n
    }

    /// Scenery the bus is put down inside (a mod map's static buses standing in its depot
    /// where the entry point is): objects no taller than a vehicle whose collision boxes lie
    /// for a third or more inside the bus's footprint are taken away, as the object editor
    /// takes one away. A shelter or a sign at the kerb that the bus only touches stays.
    pub fn clear_props_under(&self, renderer: &Renderer, scene: &mut Scene, b: &omsi_sim::collision::Obb) -> usize {
        let [ax, ay] = b.axes();
        let inside = |p: glam::DVec2| {
            let d = p - b.center;
            d.dot(ax).abs() <= b.half.x && d.dot(ay).abs() <= b.half.y
        };
        let mut keys: Vec<i64> = Vec::new();
        for st in self.tile_state.lock().values() {
            for o in st.obstacles.iter() {
                if o.id < 0 || o.mass > 0.0 || o.z1 - o.z0 > 5.0 || o.z1 < b.z0 || o.z0 > b.z1 || !o.overlaps_plan(b) || keys.contains(&o.id) {
                    continue;
                }
                let [ox, oy] = o.axes();
                let mut n = 0;
                for i in 0..5 {
                    for j in 0..5 {
                        let p = o.center + ox * o.half.x * (i as f64 / 2.0 - 1.0) + oy * o.half.y * (j as f64 / 2.0 - 1.0);
                        if inside(p) {
                            n += 1;
                        }
                    }
                }
                if n >= 9 {
                    keys.push(o.id);
                }
            }
        }
        // (a prop without a collision box - most static vehicles of mod maps have none: its
        // drawn meshes, a third of their points inside the bus's footprint)
        let types: Vec<Arc<ObjectType>> = self.object_types.lock().values().flatten().cloned().collect();
        let mut by_mesh: Vec<i64> = Vec::new();
        for (id, eo) in self.edit_objects.lock().iter() {
            if keys.contains(&eo.key) || (eo.pos.truncate() - b.center).length() > 25.0 {
                continue;
            }
            let Some(ot) = types.iter().find(|t| t.sco.path == eo.sco) else { continue };
            if ot.sco.surface || ot.sco.render_type.is_ground_layer() {
                continue;
            }
            let (mut n, mut inn, mut z0, mut z1) = (0usize, 0usize, f32::MAX, f32::MIN);
            for (m, _, _) in ot.meshes.iter() {
                for p in m.positions.iter().step_by(4) {
                    let w = eo.xf.transform_point3(*p);
                    n += 1;
                    z0 = z0.min(w.z);
                    z1 = z1.max(w.z);
                    if inside(eo.pos.truncate() + glam::DVec2::new(w.x as f64, w.y as f64)) {
                        inn += 1;
                    }
                }
            }
            if n >= 8 && z1 - z0 <= 5.0 && inn * 3 >= n {
                by_mesh.push(*id);
            }
        }
        if keys.is_empty() && by_mesh.is_empty() {
            return 0;
        }
        let ids: Vec<(i64, std::path::PathBuf)> = self.edit_objects.lock().iter().filter(|(id, eo)| keys.contains(&eo.key) || by_mesh.contains(id)).map(|(id, eo)| (*id, eo.sco.clone())).collect();
        for (id, sco) in &ids {
            log::info!("spawn: scenery object {id} ({}) stood where the bus is put: taken away", sco.display());
            self.apply_object_edit(renderer, scene, *id, ObjectEdit { deleted: true, ..Default::default() });
        }
        ids.len()
    }

    /// The spaces parked cars left (key, the object that stood there), whose tile is still
    /// loaded.
    pub fn free_parking(&self) -> Vec<(i64, ParkedObject)> {
        let states = self.tile_state.lock();
        self.departed_objects
            .lock()
            .iter()
            .filter(|(_, (p, _, _))| states.contains_key(&p.tile))
            .map(|(k, (p, _, _))| (*k, p.clone()))
            .collect()
    }

    /// An AI car has parked in the space of departed parked car `key`: the parked object is
    /// there again (shown, an obstacle, a parked car the traffic keeps clear of).
    pub fn return_parked(&self, renderer: &Renderer, scene: &mut Scene, key: i64) -> bool {
        let Some((p, obst, boxes)) = self.departed_objects.lock().remove(&key) else { return false };
        let mut states = self.tile_state.lock();
        let Some(st) = states.get_mut(&p.tile) else { return false };
        st.obstacles.extend(obst);
        st.parked_boxes.extend(boxes);
        drop(states);
        for &inst in &p.instances {
            if let Some(i) = scene.instances.get(inst) {
                let (alpha, uv) = (i.slot_alpha.clone(), i.slot_uv.clone());
                renderer.set_params(scene, inst, &alpha, true, &uv);
            }
        }
        self.departed.lock().remove(&key);
        self.parked_objects.lock().insert(key, p);
        self.refresh_tile_lists();
        true
    }

    /// The object editor changed map object `id` (the whole edit so far, from where the
    /// tile put it): its instances and its collision boxes follow.
    pub fn apply_object_edit(&self, renderer: &Renderer, scene: &mut Scene, id: i64, edit: ObjectEdit) {
        let before = self.object_edits.lock().insert(id, edit).unwrap_or_default();
        let Some(eo) = self.edit_objects.lock().get(&id).cloned() else { return };
        show_edit(renderer, scene, &eo, edit);
        // the boxes: from the previous edit to this one, turned about the object's place
        if let Some(st) = self.tile_state.lock().get_mut(&eo.tile) {
            let from = eo.pos + before.moved;
            let to = eo.pos + edit.moved;
            let turn = (edit.turned - before.turned).to_radians();
            let (sin, cos) = turn.sin_cos();
            let spin = |c: glam::DVec2| {
                let d = c - from.truncate();
                // clockwise, as headings go
                to.truncate() + glam::DVec2::new(d.x * cos + d.y * sin, -d.x * sin + d.y * cos)
            };
            // (a deleted object's boxes go under the ground with it)
            let sunk = |e: &ObjectEdit| e.moved.z - if e.deleted { 10_000.0 } else { 0.0 };
            let dz = sunk(&edit) - sunk(&before);
            for b in st.obstacles.iter_mut().filter(|b| b.id == eo.key) {
                b.center = spin(b.center);
                b.heading += turn;
                b.z0 += dz;
                b.z1 += dz;
            }
            for m in st.mesh_obstacles.iter_mut().filter(|m| m.id == eo.key) {
                let c = spin(m.pos.truncate());
                m.pos = DVec3::new(c.x, c.y, m.pos.z + dz);
                m.heading += turn;
                m.bounds.center = spin(m.bounds.center);
                m.bounds.heading += turn;
                m.bounds.z0 += dz;
                m.bounds.z1 += dz;
            }
        }
        self.refresh_tile_lists();
    }

    /// The file tile (tx, ty) is read from, and the map folder's place relative to `root`.
    pub fn tile_source(&self, tx: i32, ty: i32) -> Option<PathBuf> {
        let t = self.global.tiles.iter().find(|t| t.x == tx && t.y == ty)?;
        Some(omsi_cfg::resolve_path(&self.map_dir, &t.file))
    }

    /// Take a tile off the GPU and out of the world's lists (its lanes, traffic light
    /// programs and parked cars stay: the traffic holds on to them by index).
    ///
    /// True when object types went with it: [`World::trim_object_types`] then lets their
    /// meshes go on the CPU side too (once after a batch of tiles, not per tile).
    pub fn unload_tile(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        key: (i32, i32),
        audio: Option<&omsi_audio::AudioEngine>,
    ) -> bool {
        let Some(state) = self.tile_state.lock().remove(&key) else {
            self.drop_scripted(key, audio);
            return false;
        };
        let _ = self.parked_live.fetch_update(std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed, |n| Some(n.saturating_sub(state.parked_count)));
        self.help_arrows.lock().remove(&key);
        self.parked_objects.lock().retain(|_, p| p.tile != key);
            self.departed_objects.lock().retain(|_, (p, _, _)| p.tile != key);
        self.edit_objects.lock().retain(|_, o| o.tile != key);
        {
            // the posts' instances go back to the pool with the tile
            let mut poles = self.poles.lock();
            for k in &state.poles {
                poles.remove(k);
            }
        }
        let freed = self.gpu.lock().release_tile(renderer, scene, state.gpu);
        self.drop_scripted(key, audio);
        self.terrains.write().remove(&key);
        self.surfaces.write().remove(&key);
        freed > 0
    }

    /// Put scenery object `rel` (a `.sco`) at `pos` turned to `heading` (degrees), outside
    /// any tile, its `[texttexture]` strings taken from `strings` - the game's own helpers,
    /// as OMSI puts the dynamic route arrows. Taken away again with
    /// `remove_helper_object`.
    pub fn add_helper_object(&self, renderer: &Renderer, scene: &mut Scene, rel: &str, pos: DVec3, heading: f64, strings: &[String]) -> Option<TileGpu> {
        let ot = self.object_type(rel)?;
        let mut guard = self.gpu.lock();
        let gpu = &mut *guard;
        self.ensure_ground(renderer, scene, gpu);
        let ground_mat = gpu.ground.as_ref()?.ground_mat;
        let tkey = self.type_gpu(renderer, scene, gpu, &ot, &HashMap::new(), ground_mat);
        let mut tg = TileGpu::default();
        gpu.types.get_mut(&tkey)?.users += 1;
        tg.types.push(tkey);
        let meshes = gpu.types[&tkey].meshes.clone();
        let xf = Mat4::from_rotation_z((-heading).to_radians() as f32);
        for (mi, (mesh_id, mats)) in meshes.iter().enumerate() {
            let new = renderer.add_instance(scene, *mesh_id, pos, xf, mats.clone());
            renderer.set_omsi_caster(scene, new, ot.mesh_casts.get(mi).copied().unwrap_or(false));
            // a route arrow casts no shadow (only [shadow] meshes do in Omsi.exe)
            if ot.sco.is_help_arrow {
                renderer.set_casts_shadow(scene, new, false);
            }
            let inst = gpu.instance(renderer, scene, new);
            tg.instances.push(inst);
            let Some((_, o3d_mats, overrides)) = ot.meshes.get(mi) else { continue };
            for o in overrides.iter().filter(|o| o.use_text_texture.is_some()) {
                let (Some(slot), Some(tt)) = (
                    omsi_sim::vehicle::override_slot(o3d_mats, o),
                    ot.model.text_textures.get(o.use_text_texture.unwrap().max(0) as usize),
                ) else {
                    continue;
                };
                let text = tt.variable.trim().parse::<usize>().ok().and_then(|k| strings.get(k)).cloned().unwrap_or_default();
                let alpha = text_alpha(o3d_mats, slot, overrides);
                let slot_ov: Vec<&MaterialDef> = overrides.iter().filter(|o| !o.item && omsi_sim::vehicle::override_slot(o3d_mats, o) == Some(slot)).collect();
                let key = text_material_key(scenery_text_key(tt, &text, alpha), &slot_ov);
                if let Some(e) = gpu.text_textures.get_mut(&key) {
                    e.2 += 1;
                    let mat = e.1;
                    tg.texts.push(key);
                    renderer.set_material(scene, inst, slot, mat);
                    continue;
                }
                let atlas = self.fonts.lock().get(&tt.font, &|p| omsi_texture::decode_file(p).ok().map(|i| (i.width, i.height, i.rgba)));
                let image = helper_text_image(tt, atlas.as_deref(), &text).unwrap_or_else(|| scenery_text_image(tt, atlas, &text));
                let tex = gpu.add_image(renderer, scene, &image, true);
                let mat = text_material(renderer, scene, tex, alpha, &slot_ov);
                let mat = gpu.material(renderer, scene, mat);
                gpu.text_textures.insert(key.clone(), (tex, mat, 1));
                tg.texts.push(key);
                renderer.set_material(scene, inst, slot, mat);
            }
        }
        Some(tg)
    }

    /// Take away an object `add_helper_object` put down.
    pub fn remove_helper_object(&self, renderer: &Renderer, scene: &mut Scene, tg: TileGpu) {
        self.gpu.lock().release_tile(renderer, scene, tg);
    }

    /// The scripted objects of tile `key` go (their sounds stop).
    fn drop_scripted(&self, key: (i32, i32), audio: Option<&omsi_audio::AudioEngine>) {
        self.particle_objects.lock().remove(&key);
        if self.light_maps.lock().remove(&key).is_some() {
            self.light_maps_generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let mut scripted = self.scripted.lock();
        if !scripted.iter().any(|o| o.tile == key) {
            return;
        }
        let mut kept = Vec::with_capacity(scripted.len());
        for mut o in scripted.drain(..) {
            if o.tile == key {
                if let (Some(a), Some(mut ss)) = (audio, o.sounds.take()) {
                    ss.stop_all(a);
                }
            } else {
                kept.push(o);
            }
        }
        *scripted = kept;
    }

    /// Object types no loaded tile uses any more leave the type cache (with their meshes on
    /// the CPU side).
    pub fn trim_object_types(&self) {
        self.object_types
            .lock()
            .retain(|_, t| t.as_ref().map(|t| Arc::strong_count(t) > 1).unwrap_or(true));
    }

    /// The placed objects that stop the outside camera and may reach into the rectangle
    /// `lo`..`hi` of the ground plane (with their types, which are alive while a tile uses
    /// them).
    pub fn camera_blockers(
        &self,
        lo: DVec2,
        hi: DVec2,
    ) -> Vec<(Arc<ObjectType>, crate::camera_arm::Blocker)> {
        let ts = tile_size();
        // an object is kept with the tile its origin stands in, but a big one (a school, a
        // supermarket) reaches well into the next: the tiles around are looked at as well
        let (x0, x1) = (
            ((lo.x - ts) / ts).floor() as i32,
            ((hi.x + ts) / ts).floor() as i32,
        );
        let (y0, y1) = (
            ((lo.y - ts) / ts).floor() as i32,
            ((hi.y + ts) / ts).floor() as i32,
        );
        let states = self.tile_state.lock();
        let mut out = Vec::new();
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                let Some(s) = states.get(&(tx, ty)) else {
                    continue;
                };
                for b in &s.blockers {
                    let r = b.radius;
                    if b.pos.x + r < lo.x
                        || b.pos.x - r > hi.x
                        || b.pos.y + r < lo.y
                        || b.pos.y - r > hi.y
                    {
                        continue;
                    }
                    if let Some(t) = b.ty.upgrade() {
                        out.push((t, b.clone()));
                    }
                }
            }
        }
        out
    }

    /// Rebuild the world's lists (stops, obstacles, lights, lamps) from the loaded tiles.
    pub fn refresh_tile_lists(&self) {
        let states = self.tile_state.lock();
        let mut keys: Vec<&(i32, i32)> = states.keys().collect();
        keys.sort();
        let mut stops = Vec::new();
        let mut waiting = Vec::new();
        let mut collision = omsi_sim::collision::CollisionWorld::default();
        let mut parked = Vec::new();
        let mut coronas = Vec::new();
        let mut lights = Vec::new();
        let mut lamps = Vec::new();
        let mut night = Vec::new();
        let mut modes = Vec::new();
        let mut petrol = Vec::new();
        let mut reverb = Vec::new();
        for k in keys {
            let s = &states[k];
            petrol.extend(s.petrol_stations.iter().copied());
            reverb.extend(s.reverb_zones.iter().copied());
            stops.extend(s.bus_stops.iter().cloned());
            waiting.extend(s.waiting_places.iter().cloned());
            for b in &s.obstacles {
                collision.add(*b);
            }
            parked.extend(s.parked_boxes.iter().copied());
            for m in &s.mesh_obstacles {
                collision.add_mesh(m.clone());
            }
            coronas.extend(s.coronas.iter().cloned());
            lights.extend(s.lights.iter().cloned());
            lamps.extend(s.light_objects.iter().cloned());
            night.extend(s.night_slots.iter().cloned());
            modes.extend(s.night_modes.iter().cloned());
        }
        *self.bus_stops.lock() = stops;
        *self.waiting_places.lock() = waiting;
        *self.collision.lock() = Arc::new(collision);
        *self.parked_boxes.lock() = Arc::new(parked);
        *self.static_coronas.lock() = coronas;
        *self.static_lights.lock() = lights;
        *self.light_objects.lock() = lamps;
        *self.night_slots.lock() = night;
        *self.night_modes.lock() = modes;
        *self.petrol_stations.lock() = petrol;
        *self.reverb_zones.lock() = reverb;
        self.tiles_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// The passenger cabin of a waiting object (a `people_standing_*` marker, a shelter),
    /// read once per file.
    fn waiting_cabin(&self, path: &Path) -> Option<Arc<omsi_vehicle::PassengerCabin>> {
        if let Some(c) = self.waiting_cabins.lock().get(path) {
            return c.clone();
        }
        let c = omsi_vehicle::PassengerCabin::load(path)
            .map_err(|e| log::debug!("waiting places {}: {e}", path.display()))
            .ok()
            .map(Arc::new);
        self.waiting_cabins
            .lock()
            .insert(path.to_path_buf(), c.clone());
        c
    }

    /// Whether the ground at world (x, y) is loaded.
    pub fn has_ground(&self, x: f64, y: f64) -> bool {
        let key = (
            (x / tile_size()).floor() as i32,
            (y / tile_size()).floor() as i32,
        );
        self.terrains.read().contains_key(&key)
    }

    /// What the loaded tiles hold, for the streaming statistics.
    pub fn gpu_summary(&self, scene: &Scene) -> String {
        let (vehicle_tex, vehicle_formats) = {
            let v = self.vehicle_textures.lock();
            let mut f: std::collections::BTreeMap<String, (usize, u64)> =
                std::collections::BTreeMap::new();
            let mut total = 0u64;
            for (t, _) in v.values() {
                let b = scene.texture_bytes_of(*t);
                total += b;
                let e = f.entry(scene.texture_format_of(*t)).or_default();
                e.0 += 1;
                e.1 += b;
            }
            (
                total,
                f.iter()
                    .map(|(k, (n, b))| format!("{n} {k} {:.0} MB", *b as f64 / 1e6))
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        };
        let (tile_tex, tile_formats) = {
            let mut f: std::collections::BTreeMap<String, (usize, u64)> =
                std::collections::BTreeMap::new();
            let mut total = 0u64;
            for t in self
                .tile_state
                .lock()
                .values()
                .flat_map(|s| s.gpu.textures.iter())
            {
                let b = scene.texture_bytes_of(*t);
                total += b;
                let e = f
                    .entry(format!(
                        "{} {}",
                        scene.texture_format_of(*t),
                        scene
                            .texture_size_of(*t)
                            .map(|s| format!("{}x{}", s.0, s.1))
                            .unwrap_or_default()
                    ))
                    .or_default();
                e.0 += 1;
                e.1 += b;
            }
            let mut v: Vec<(String, (usize, u64))> = f.into_iter().collect();
            v.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
            (
                total,
                v.iter()
                    .take(6)
                    .map(|(k, (n, b))| format!("{n} {k} {:.0} MB", *b as f64 / 1e6))
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        };
        let all_formats = {
            let mut f: std::collections::BTreeMap<String, (usize, u64)> =
                std::collections::BTreeMap::new();
            for t in 0..scene.textures.len() {
                let b = scene.texture_bytes_of(t);
                let e = f
                    .entry(format!(
                        "{} {}",
                        scene.texture_format_of(t),
                        scene
                            .texture_size_of(t)
                            .map(|s| format!("{}x{}", s.0, s.1))
                            .unwrap_or_default()
                    ))
                    .or_default();
                e.0 += 1;
                e.1 += b;
            }
            let mut v: Vec<(String, (usize, u64))> = f.into_iter().collect();
            v.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
            let total: u64 = v.iter().map(|x| x.1 .1).sum();
            format!(
                "{:.0} MB in all: {}",
                total as f64 / 1e6,
                v.iter()
                    .take(30)
                    .map(|(k, (n, b))| format!("{n} {k} {:.1} MB", *b as f64 / 1e6))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        if omsi_cfg::env::var_os("OMSI_DEBUG_TEXTURES").is_some() {
            log::info!("all textures by size: {all_formats}");
        }
        let gpu = self.gpu.lock();
        let texels: u64 = gpu.textures.values().map(|t| t.texels).sum();
        let bytes: u64 = gpu.textures.values().map(|t| t.bytes).sum();
        let mut by_format: std::collections::BTreeMap<String, (usize, u64)> =
            std::collections::BTreeMap::new();
        for t in gpu.textures.values() {
            let e = by_format.entry(format!("{:?}", t.format)).or_default();
            e.0 += 1;
            e.1 += t.bytes;
        }
        let formats: Vec<String> = by_format
            .iter()
            .map(|(k, (n, b))| format!("{n} {k} {:.0} MB", *b as f64 / 1e6))
            .collect();
        let (tex_all, mesh_all, other_all) = scene.gpu_bytes();

        let free_instances: usize = gpu.free_instances.values().map(|v| v.len()).sum();
        format!(
            "{} tiles; {} scenery textures ({:.0} MB of texels, {:.0} MB on the GPU: {}), {} object types on the GPU, {} in the type cache; scene {} meshes / {} textures / {} materials / {} instances, free {} / {} / {} / {}; GPU {:.0} MB textures ({:.0} MB vehicles': {}; {:.0} MB tiles' own: {}), {:.0} MB meshes, {:.0} MB draw data",
            self.tile_state.lock().len(),
            gpu.textures.len(),
            texels as f64 * 4.0 / 1e6,
            bytes as f64 / 1e6,
            formats.join(", "),
            gpu.types.len(),
            self.object_types.lock().len(),
            scene.meshes.len(),
            scene.textures.len(),
            scene.materials.len(),
            scene.instances.len(),
            gpu.free_meshes.len(),
            gpu.free_textures.len(),
            gpu.free_materials.len(),
            free_instances,
            tex_all as f64 / 1e6,
            vehicle_tex as f64 / 1e6,
            vehicle_formats,
            tile_tex as f64 / 1e6,
            tile_formats,
            mesh_all as f64 / 1e6,
            other_all as f64 / 1e6
        )
    }

    /// What the loaded map holds in memory on the CPU side (MB, estimated from the sizes of
    /// the big buffers), for OMSI_PROFILE.
    pub fn cpu_summary(&self) -> String {
        let mb = |b: usize| b as f64 / 1e6;
        let types: Vec<Arc<ObjectType>> = self
            .object_types
            .lock()
            .values()
            .flatten()
            .cloned()
            .collect();
        let type_bytes: usize = types.iter().map(|t| t.mesh_bytes()).sum();
        let (mut staged_n, mut staged_bytes) = (0usize, 0usize);
        for st in self.staged.lock().values() {
            staged_n += 1;
            staged_bytes += st
                .splines
                .iter()
                .map(|s| s.shape.heap_bytes())
                .sum::<usize>()
                + st.drive.iter().map(|d| d.0.heap_bytes() + d.2.as_ref().map_or(0, |s| s.heap_bytes())).sum::<usize>()
                + st.base_terrain.heights.capacity() * 4;
            staged_bytes += st
                .meshes
                .lock()
                .as_ref()
                .map(|m| m.iter().map(|x| x.heap_bytes()).sum::<usize>())
                .unwrap_or(0);
            if let Some(r) = st.resolved.get() {
                staged_bytes += r
                    .warped
                    .values()
                    .flat_map(|v| v.iter())
                    .map(|m| m.heap_bytes())
                    .sum::<usize>()
                    + r.terrain.heights.capacity() * 4;
            }
        }
        let (mut rasters, mut drive, mut tris) = (0usize, 0usize, 0usize);
        let surfaces = self.surfaces.read();
        for sf in surfaces.values() {
            let (r, d) = sf.heap_bytes();
            rasters += r;
            drive += d;
            tris += sf.drive.tris.len();
        }
        let terrains: usize = self
            .terrains
            .read()
            .values()
            .map(|t| t.heights.capacity() * 4)
            .sum();
        let vehicle_textures = self.vehicle_textures.lock().len();
        format!(
            "CPU: {} object types {:.0} MB of meshes, {} staged tiles {:.0} MB, {} surfaces {:.0} MB rasters + {:.0} MB wheel grids ({} faces), terrains {:.0} MB, decoded textures held {:.0} MB; {} vehicle textures, {} vehicle sets on the GPU",
            types.len(),
            mb(type_bytes),
            staged_n,
            mb(staged_bytes),
            surfaces.len(),
            mb(rasters),
            mb(drive),
            tris,
            mb(terrains),
            mb(self.textures.held_bytes()),
            vehicle_textures,
            self.vehicle_gpu.lock().len()
        )
    }

    /// Swap in the textures compressed on the workers since the last call (until
    /// `deadline`), and start compressing the ones uploaded as RGBA since. Returns how many
    /// were swapped.
    pub fn apply_texture_upgrades(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        deadline: Option<std::time::Instant>,
    ) -> usize {
        let (wanted, restores) = {
            let mut gpu = self.gpu.lock();
            let mut w = std::mem::take(&mut gpu.wants_upgrade);
            w.append(&mut self.freetex_upgrades.lock());
            let r = std::mem::take(&mut gpu.wants_restore);
            w.extend(r.iter().cloned());
            (w, r)
        };
        if !wanted.is_empty() {
            let mut pending = self.upgrades_pending.lock();
            for path in wanted {
                if !pending.insert(path.clone()) {
                    continue;
                }
                let done = self.upgrades_done.clone();
                let restore = restores.contains(&path);
                // (off the frame's pool: see `threads`)
                crate::threads::background_pool().spawn(move || {
                    if let Some(t) = load_texture_key(&path, true) {
                        // (a texture over the budget comes back whatever it is, and so
                        // does one put up at half its size meanwhile: see
                        // `omsi_texture::gpu::halved_for_now`)
                        if t.format.is_compressed() || restore || (t.width >= 256 && t.height >= 256) {
                            done.lock().push((path, Arc::new(t)));
                            return;
                        }
                    }
                    // nothing better to be had: it stays as it is
                    done.lock().push((
                        path,
                        Arc::new(TextureData {
                            width: 0,
                            height: 0,
                            format: omsi_texture::PixelFormat::Rgba8,
                            levels: Vec::new(),
                            has_alpha: false,
                            gpu_mips: false,
                        }),
                    ));
                });
            }
        }
        let mut swapped: Vec<TextureId> = Vec::new();
        loop {
            if deadline
                .map(|d| std::time::Instant::now() >= d)
                .unwrap_or(false)
                && !swapped.is_empty()
            {
                break;
            }
            let Some((path, data)) = self.upgrades_done.lock().pop() else {
                break;
            };
            self.upgrades_pending.lock().remove(&path);
            // the texture is still up under that name (it may have gone meanwhile)
            let vid = self.vehicle_textures.lock().get(&path).map(|e| e.0);
            let id = match vid {
                Some(id) => Some(id),
                None => self.gpu.lock().textures.get(&path).map(|e| e.id),
            };
            let Some(id) = id else { continue };
            if data.levels.is_empty() {
                // nothing better to be had (an upgrade that stays RGBA)
                continue;
            }
            renderer.replace_texture(scene, id, &data);
            if let Some(e) = self.gpu.lock().textures.get_mut(&path) {
                e.bytes = scene.texture_bytes_of(id);
                e.format = data.format;
                e.dropped = 0;
            }
            swapped.push(id);
        }
        let n = swapped.len();
        if n > 0 {
            let t = std::time::Instant::now();
            let rebound = renderer.rebind_textures(scene, &swapped);
            if omsi_cfg::env::var_os("OMSI_PROFILE").is_some() {
                log::info!("textures: {n} compressed ones swapped in, {rebound} materials rebound in {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
            }
        }
        n
    }

    /// OMSI's `[texmemlimit]`: the scenery and vehicle textures may take `bytes` on the GPU
    /// (0 = no limit), see [`World::update_texture_budget`].
    pub fn set_texture_budget(&self, bytes: u64) {
        self.texture_limit
            .store(bytes, std::sync::atomic::Ordering::Relaxed);
    }

    /// The textures' budget now (bytes, 0 = none).
    pub fn texture_budget_bytes(&self) -> u64 {
        self.texture_limit.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Keep the textures within their budget, once a second (`force`: now): while they take
    /// more, the scenery textures only far tiles use lose their finest mip level, the
    /// farthest first, down to 64 texels a side and never within 150 m of `centers` (the
    /// camera, the player's bus); when there is room again, those that came within 400 m
    /// are read again whole on a worker and swapped back. Vehicle textures count but keep
    /// their levels (a fleet set nobody draws leaves the GPU anyway). Returns the textures
    /// shrunk or sent to be read again.
    pub fn update_texture_budget(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        centers: &[DVec3],
        force: bool,
    ) -> usize {
        const NEAR: f64 = 150.0;
        const RESTORE: f64 = 400.0;
        const MIN_SIDE: u32 = 64;
        let limit = self
            .texture_limit
            .load(std::sync::atomic::Ordering::Relaxed);
        if limit == 0 || centers.is_empty() {
            return 0;
        }
        {
            let mut last = self.budget_checked.lock();
            if !force
                && last
                    .map(|t| t.elapsed().as_secs_f32() < 1.0)
                    .unwrap_or(false)
            {
                return 0;
            }
            *last = Some(std::time::Instant::now());
        }
        let t0 = std::time::Instant::now();
        let vehicle_bytes: u64 = self
            .vehicle_textures
            .lock()
            .values()
            .map(|(t, _)| scene.texture_bytes_of(*t))
            .sum();
        let ts = tile_size();
        let tile_distance = |k: &(i32, i32)| -> f64 {
            centers
                .iter()
                .map(|c| {
                    let (x0, y0) = (k.0 as f64 * ts, k.1 as f64 * ts);
                    let dx = (x0 - c.x).max(c.x - (x0 + ts)).max(0.0);
                    let dy = (y0 - c.y).max(c.y - (y0 + ts)).max(0.0);
                    (dx * dx + dy * dy).sqrt()
                })
                .fold(f64::MAX, f64::min)
        };
        let states = self.tile_state.lock();
        let mut gpu = self.gpu.lock();
        if gpu.textures.values().map(|e| e.bytes).sum::<u64>() + vehicle_bytes <= limit && gpu.textures.values().all(|e| e.dropped == 0) {
            return 0;
        }
        // how near each scenery texture is: the nearest tile that uses it
        let mut near: hashbrown::HashMap<TextureId, f64> = hashbrown::HashMap::new();
        let mut spline_textures = hashbrown::HashSet::new();
        let (mut types, mut splines): (hashbrown::HashMap<usize, f64>, hashbrown::HashMap<usize, f64>) = Default::default();
        let (mut trees, mut shared): (hashbrown::HashMap<&str, f64>, hashbrown::HashMap<&PathBuf, f64>) = Default::default();
        for (key, st) in states.iter() {
            let d = tile_distance(key);
            let nearer = |e: &mut f64| *e = e.min(d);
            st.gpu.types.iter().for_each(|t| nearer(types.entry(*t).or_insert(f64::MAX)));
            st.gpu.spline_types.iter().for_each(|t| nearer(splines.entry(*t).or_insert(f64::MAX)));
            st.gpu.trees.iter().for_each(|t| nearer(trees.entry(t.as_str()).or_insert(f64::MAX)));
            st.gpu.shared_textures.iter().for_each(|p| nearer(shared.entry(p).or_insert(f64::MAX)));
        }
        {
            let g = &*gpu;
            let mut see = |p: &PathBuf, d: f64| {
                if let Some(e) = g.textures.get(p) {
                    let n = near.entry(e.id).or_insert(f64::MAX);
                    *n = n.min(d);
                }
            };
            for (t, d) in &types {
                g.types.get(t).into_iter().flat_map(|t| t.textures.iter()).for_each(|p| see(p, *d));
            }
            for (t, d) in &splines {
                for p in g.splines.get(t).into_iter().flat_map(|s| s.textures.iter()) {
                    see(p, *d);
                    spline_textures.insert(p.clone());
                }
            }
            for (t, d) in &trees {
                g.trees.get(*t).and_then(|t| t.texture.as_ref()).into_iter().for_each(|p| see(p, *d));
            }
            for (p, d) in &shared {
                see(p, *d);
            }
        }
        drop((trees, shared));
        drop(states);
        let usage: u64 = gpu.textures.values().map(|e| e.bytes).sum::<u64>() + vehicle_bytes;
        let mut entries: Vec<(f64, PathBuf)> = gpu
            .textures
            .iter()
            .map(|(p, e)| (near.get(&e.id).copied().unwrap_or(f64::MAX), p.clone()))
            .collect();
        let mut shrunk: Vec<TextureId> = Vec::new();
        let mut restoring = 0usize;
        // A texture shrunk while its tiles were far that is near now comes back whole at
        // once, room or not: the far ones give way for it in the seconds after. (Waiting for
        // room left the buildings right in front of the bus blurred for good on a map that
        // filled the budget - they had lost their levels on the way in.)
        {
            let pending = self.upgrades_pending.lock();
            for (d, p) in &entries {
                if *d >= NEAR || restoring >= 24 {
                    continue;
                }
                if gpu.textures.get(p).is_some_and(|e| e.dropped > 0) && !pending.contains(p) && !gpu.wants_restore.contains(p) {
                    gpu.wants_restore.push(p.clone());
                    restoring += 1;
                }
            }
        }
        if usage > limit {
            entries.sort_by(|a, b| b.0.total_cmp(&a.0));
            let mut over = usage - limit;
            for (d, p) in &entries {
                // (96 a second: at 16 a map's first tiles stayed over a small card's budget
                // for a minute and a half)
                if over == 0 || shrunk.len() >= 96 || *d < NEAR {
                    break;
                }
                if spline_textures.contains(p) {
                    continue;
                }
                let Some(e) = gpu.textures.get_mut(p) else {
                    continue;
                };
                let Some((w, h, levels)) = renderer.texture_levels(scene, e.id) else {
                    continue;
                };
                if w.min(h) / 2 < MIN_SIDE || levels < 2 {
                    continue;
                }
                let before = e.bytes;
                // far away and big: two levels at once
                let n = if *d > 700.0 && w.min(h) / 4 >= MIN_SIDE.max(256) && levels > 2 { 2 } else { 1 };
                if renderer.drop_top_levels(scene, e.id, n) {
                    e.bytes = scene.texture_bytes_of(e.id);
                    e.dropped += n;
                    over = over.saturating_sub(before - e.bytes);
                    shrunk.push(e.id);
                }
            }
            // Still over: the vehicles' pictures give way as well, the biggest first, down
            // to 1024 pixels a side. Counted but never made smaller, the timetable's fleet
            // filled 5 GB of textures on a graphics chip with 0.5 GB of its own until the
            // device was lost (#1463).
            if over > 0 && shrunk.len() < 96 {
                let mut own: Vec<(TextureId, u64)> = self
                    .vehicle_textures
                    .lock()
                    .values()
                    .map(|(t, _)| (*t, scene.texture_bytes_of(*t)))
                    .collect();
                own.sort_by(|a, b| b.1.cmp(&a.1));
                for (id, before) in own {
                    if over == 0 || shrunk.len() >= 96 {
                        break;
                    }
                    let Some((w, h, levels)) = renderer.texture_levels(scene, id) else { continue };
                    if w.min(h) / 2 < 1024 || levels < 2 {
                        continue;
                    }
                    if renderer.drop_top_levels(scene, id, 1) {
                        over = over.saturating_sub(before.saturating_sub(scene.texture_bytes_of(id)));
                        shrunk.push(id);
                    }
                }
            }
        } else {
            // room for the near ones to come back (with a tenth kept free)
            entries.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut room = (limit - limit / 10).saturating_sub(usage);
            let pending = self.upgrades_pending.lock();
            for (d, p) in &entries {
                if *d > RESTORE || restoring >= 16 {
                    break;
                }
                let Some(e) = gpu.textures.get(p) else {
                    continue;
                };
                if e.dropped == 0 || pending.contains(p) {
                    continue;
                }
                let whole = e.bytes << (2 * e.dropped.min(8));
                if whole - e.bytes > room {
                    break;
                }
                room -= whole - e.bytes;
                gpu.wants_restore.push(p.clone());
                restoring += 1;
            }
        }
        drop(gpu);
        let rebound = renderer.rebind_textures(scene, &shrunk);
        if (!shrunk.is_empty() || restoring > 0) && omsi_cfg::env::var_os("OMSI_PROFILE").is_some() {
            log::info!("texture budget: {:.0} of {:.0} MB in use, {} textures lost a level ({} materials rebound), {} coming back, in {:.1} ms", usage as f64 / 1e6, limit as f64 / 1e6, shrunk.len(), rebound, restoring, t0.elapsed().as_secs_f64() * 1000.0);
        }
        shrunk.len() + restoring
    }

    /// After big unloads, cut the free tail off the scene's arrays (meshes, textures,
    /// materials, instances), so that a long drive across a big map does not keep the
    /// arrays at their largest. Only when a quarter of an array or more would go (cutting
    /// instances makes the renderer rebuild its per-draw buffers once).
    pub fn compact_slots(&self, renderer: &Renderer, scene: &mut Scene) -> [usize; 4] {
        let mut gpu = self.gpu.lock();
        let worth = |len: usize, keep: usize| len - keep >= 1024 && (len - keep) * 4 >= len;
        let meshes = gpu.free_meshes.free_tail(scene.meshes.len());
        let textures = gpu.free_textures.free_tail(scene.textures.len());
        let materials = gpu.free_materials.free_tail(scene.materials.len());
        // instances are free by slot count: their tail across all counts
        let mut instances = scene.instances.len();
        // Most checks have no free slot at the end. Avoid sorting every free instance
        // after each streaming burst just to discover that the tail cannot shrink.
        if instances > 0 && gpu.free_instances.values().any(|l| l.0.iter().any(|r| r.0 == instances - 1)) {
            let free: hashbrown::HashSet<usize> = gpu.free_instances.values()
                .flat_map(|l| l.0.iter().map(|r| r.0))
                .collect();
            while instances > 0 && free.contains(&(instances - 1)) {
                instances -= 1;
            }
        }
        let lens = [
            scene.meshes.len(),
            scene.textures.len(),
            scene.materials.len(),
            scene.instances.len(),
        ];
        let keep = [meshes, textures, materials, instances];
        let keep: Vec<usize> = lens
            .iter()
            .zip(keep)
            .map(|(l, k)| if worth(*l, k) { k } else { *l })
            .collect();
        if keep.iter().zip(lens).all(|(k, l)| *k == l) {
            return [0; 4];
        }
        let t = std::time::Instant::now();
        renderer.truncate(scene, keep[0], keep[1], keep[2], keep[3]);
        gpu.free_meshes.keep_below(keep[0]);
        gpu.free_textures.keep_below(keep[1]);
        gpu.free_materials.keep_below(keep[2]);
        for l in gpu.free_instances.values_mut() {
            l.keep_below(keep[3]);
        }
        let cut = [
            lens[0] - keep[0],
            lens[1] - keep[1],
            lens[2] - keep[2],
            lens[3] - keep[3],
        ];
        if omsi_cfg::env::var_os("OMSI_PROFILE").is_some() {
            log::info!("scene slots: cut {} meshes, {} textures, {} materials, {} instances off the end in {:.1} ms", cut[0], cut[1], cut[2], cut[3], t.elapsed().as_secs_f64() * 1000.0);
        }
        cut
    }

    /// Wait for the textures being compressed and swap them all in (offscreen pictures).
    pub fn finish_texture_upgrades(&self, renderer: &Renderer, scene: &mut Scene) {
        loop {
            self.apply_texture_upgrades(renderer, scene, None);
            if self.upgrades_pending.lock().is_empty() && self.gpu.lock().wants_upgrade.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// The tiles whose data is loaded.
    pub fn loaded_tiles(&self) -> Vec<(i32, i32)> {
        self.tile_state.lock().keys().copied().collect()
    }
}

/// The rotation of a knocked-over post: turned about its foot by 86° (it rests on its own
/// thickness) in the direction of `push`.
/// A file that belongs to a tile (`.terrain`, …): beside the tile file, or - for a tile the
/// object editor saved as a copy in the content folder, which has only the tile itself -
/// in the same map folder under the other content roots.
pub fn tile_companion(path: &Path, ext: &str) -> PathBuf {
    // a copy in a content root before the installation's (the editor's ground, a mod's)
    if let (Some(dir), Some(name)) = (path.parent(), path.file_name()) {
        let p = omsi_cfg::resolve_path(dir, &format!("{}{}", name.to_string_lossy(), ext));
        if omsi_cfg::vfs::exists(&p) {
            return p;
        }
    }
    let direct = PathBuf::from(format!("{}{}", path.display(), ext));
    if omsi_cfg::vfs::exists(&direct) {
        return direct;
    }
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else { return direct };
    let name = format!("{}{}", name.to_string_lossy(), ext);
    omsi_cfg::mirrored_dirs(dir)
        .into_iter()
        .map(|d| omsi_cfg::resolve_path(&d, &name))
        .find(|p| omsi_cfg::vfs::exists(p))
        .unwrap_or(direct)
}

/// An object as the object editor has left it: moved, turned about its place, or gone.
fn show_edit(renderer: &Renderer, scene: &mut Scene, eo: &EditObject, e: ObjectEdit) {
    // (a deleted one goes deep under the ground rather than being hidden: its instances'
    // visibility is the level-of-detail switch's, and an undone delete brings it back)
    let rot = Mat4::from_rotation_z(-(e.turned.to_radians() as f32)) * eo.xf;
    let at = eo.pos + e.moved - if e.deleted { DVec3::Z * 10_000.0 } else { DVec3::ZERO };
    for inst in &eo.instances {
        renderer.set_transform(scene, *inst, at, rot);
    }
}

/// Stop drawing an instance, keeping its other parameters.
fn hide_instance(renderer: &Renderer, scene: &mut Scene, inst: usize) {
    let Some(i) = scene.instances.get(inst) else { return };
    let (alpha, uv) = (i.slot_alpha.clone(), i.slot_uv.clone());
    renderer.set_params(scene, inst, &alpha, false, &uv);
}

fn fallen_pole(xf: Mat4, push: DVec3) -> Mat4 {
    let dir = glam::Vec3::new(push.x as f32, push.y as f32, 0.0).normalize_or(glam::Vec3::Y);
    let axis = glam::Vec3::Z.cross(dir).normalize_or(glam::Vec3::X);
    Mat4::from_axis_angle(axis, 86f32.to_radians()) * xf
}

/// How many tiles OMSI keeps loaded around the camera's own: its `[performance_tiledistmax]`,
/// 1 in the shipped options.cfg and in the presets maps ask for (Chicago Downtown's manual:
/// "Set neighbor tiles count to 1 or max. 2").
const OMSI_TILE_DIST: i32 = 1;

/// Where the camera has to stand for a stand-in for far tiles to be drawn: the ground of
/// the tiles OMSI loads with the one it is on. None for every other object.
///
/// OMSI has a tile's objects only while the camera is at most `OMSI_TILE_DIST` tiles away
/// from it, and maps build on that: Chicago Downtown puts a model of the whole city at Navy
/// Pier (`LOD_247.sco`, 5.5 km across, its parks and the lake as flat faces 2 m above the
/// streets) to fill the view beyond the tiles loaded there, with a hole where they are.
/// openOMSI keeps the tiles of its whole view distance, so the model was there from
/// Columbus Drive on as well, its grass over the streets, the lower level and the vehicles
/// on it (#650). A stand-in is told apart by its size: more than twice as wide as all the
/// tiles OMSI has loaded with it (Chicago's are 3.6 to 8 km, its largest real objects -
/// Navy Pier, the Merchandise Mart, the road grids of whole tiles - at most 1.25 km), so
/// the far view keeps every ordinary object. A large model whose entire geometry is far
/// from its origin is also a stand-in: TH_Wald's forest cards sit over a kilometre from
/// their placement, crossing local roads when their distant owner tile is loaded here.
fn stand_in_area(ot: &ObjectType, xf: &Mat4, pos: DVec3, tile: (i32, i32)) -> Option<[f64; 4]> {
    let ts = tile_size();
    let loaded = (2 * OMSI_TILE_DIST + 1) as f64 * ts;
    // The footprint belongs to the whole object, regardless of how the exporter
    // splits it into meshes. Separate parts can each fit below the cutoff while
    // their combined extent is a distant scenery backdrop.
    let bounds = ot.meshes.iter()
        .filter(|(m, _, _)| !m.positions.is_empty())
        .map(|(m, _, _)| mesh_bounds(m, xf, pos))
        .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
    let wide = bounds.is_some_and(|b| (b[2] - b[0]).max(b[3] - b[1]) > 2.0 * loaded)
        || ot.meshes.iter().any(|(m, _, _)| stand_in_mesh(m, xf, pos, loaded));
    if !wide {
        return None;
    }
    log::debug!("{} on tile {tile:?} stands in for far tiles: drawn only from the tiles around it", ot.sco.path.display());
    Some([
        (tile.0 - OMSI_TILE_DIST) as f64 * ts,
        (tile.1 - OMSI_TILE_DIST) as f64 * ts,
        (tile.0 + OMSI_TILE_DIST + 1) as f64 * ts,
        (tile.1 + OMSI_TILE_DIST + 1) as f64 * ts,
    ])
}

fn stand_in_mesh(m: &MeshData, xf: &Mat4, pos: DVec3, loaded: f64) -> bool {
    let b = mesh_bounds(m, xf, pos);
    let width = (b[2] - b[0]).max(b[3] - b[1]);
    if width > 2.0 * loaded {
        return true;
    }
    if width <= loaded || m.positions.is_empty() {
        return false;
    }
    // Measure the offset in the model's own frame, before heading rotates its bounds:
    // the world AABB of a diagonal card can include its origin although the card is
    // wholly far away. Small offset parts and ordinary long models keep the far view.
    let local = mesh_bounds(m, &Mat4::IDENTITY, DVec3::ZERO);
    let nearest = glam::Vec3::new(
        0.0f64.clamp(local[0], local[2]) as f32,
        0.0f64.clamp(local[1], local[3]) as f32,
        0.0,
    );
    xf.transform_vector3(nearest).length() as f64 > loaded
}

/// World bounds (min x, min y, max x, max y) of a mesh placed with `xf` at `origin`.
fn mesh_bounds(m: &MeshData, xf: &Mat4, origin: DVec3) -> [f64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for p in &m.positions {
        let w = xf.transform_point3(*p).as_dvec3() + origin;
        b[0] = b[0].min(w.x);
        b[1] = b[1].min(w.y);
        b[2] = b[2].max(w.x);
        b[3] = b[3].max(w.y);
    }
    b
}

/// A `[maplight]` as a light map bakes it: where it stands (world x, y), its height over its
/// object, colour and core radius.
#[derive(Debug, Clone, Copy)]
struct BakeLamp {
    x: f64,
    y: f64,
    height: f32,
    color: [f32; 3],
    radius: f32,
}

/// A light map as Omsi.exe bakes one (0x7903e0): 256 x 256 texels over the tile at `origin`
/// and its eight neighbours, north at the top row. A texel, on the ground at its south-west
/// corner, takes each lamp's colour x min(1, (core / distance)^2), added up and held at 1,
/// and its bytes truncated (0x404c88). The distance is in three dimensions from the lamp's
/// height over its own object (the `[maplight]`'s z: where the object stands does not count),
/// and a lamp further than 15.96 cores along either axis adds nothing (less than a step).
fn bake_light_map(lamps: &[BakeLamp], origin: DVec3) -> omsi_texture::Image {
    const SIDE: usize = 256;
    let ts = tile_size() as f32;
    let reach = 15.96f32;
    let mut sum = vec![[0f32; 3]; SIDE * SIDE];
    // texel column c lies at x = (3c / 256 - 1) ts, row r at y = (2 - 3r / 256) ts
    let texel = 3.0 * ts / SIDE as f32;
    for l in lamps {
        let (lx, ly) = ((l.x - origin.x) as f32, (l.y - origin.y) as f32);
        let box_ = reach * l.radius;
        let c0 = (((lx - box_) / texel + SIDE as f32 / 3.0).floor().max(0.0)) as usize;
        let c1 = (((lx + box_) / texel + SIDE as f32 / 3.0).ceil().max(0.0) as usize).min(SIDE - 1);
        let r0 = (((2.0 * ts - (ly + box_)) / texel).floor().max(0.0)) as usize;
        let r1 = (((2.0 * ts - (ly - box_)) / texel).ceil().max(0.0) as usize).min(SIDE - 1);
        for r in r0..=r1 {
            let py = (2.0 - r as f32 / SIDE as f32 * 3.0) * ts;
            let dy = ly - py;
            if dy.abs() > box_ {
                continue;
            }
            for c in c0..=c1 {
                let px = (c as f32 / SIDE as f32 * 3.0 - 1.0) * ts;
                let dx = lx - px;
                if dx.abs() > box_ {
                    continue;
                }
                let d = (dx * dx + l.height * l.height + dy * dy).sqrt();
                let f = (l.radius / d).powi(2).min(1.0);
                let t = &mut sum[r * SIDE + c];
                for k in 0..3 {
                    t[k] = (t[k] + l.color[k] * f).min(1.0);
                }
            }
        }
    }
    let mut rgba = vec![255u8; SIDE * SIDE * 4];
    for (t, px) in sum.iter().zip(rgba.chunks_exact_mut(4)) {
        for k in 0..3 {
            px[k] = (t[k] * 255.0) as u8;
        }
    }
    omsi_texture::Image { width: SIDE as u32, height: SIDE as u32, rgba, has_alpha: false }
}

/// The part of a `.map.LM.bmp` that covers its own tile, resampled to the picture's full
/// size. The editor bakes each light map over the tile and its eight neighbours, north at the
/// top row: the tile is the middle third. Neighbouring light maps are the same picture shifted
/// by a third (85 texels between two tiles, 171 between every other one, on all stock maps).
/// Laid over the tile whole, every pool of light came out three times as large and away from
/// its lamp - a filling station's blue light lay on a garden 390 m off.
fn own_tile_of_light_map(img: &omsi_texture::Image) -> omsi_texture::Image {
    let (w, h) = (img.width as usize, img.height as usize);
    if w < 3 || h < 3 {
        return img.clone();
    }
    let texel = |x: usize, y: usize, c: usize| img.rgba[(y * w + x) * 4 + c] as f32;
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        // (bilinear, the texel centres of the output spread evenly over the middle third)
        let sy = (h as f32 / 3.0 + (y as f32 + 0.5) / 3.0 - 0.5).clamp(0.0, (h - 1) as f32);
        let (y0, fy) = (sy.floor() as usize, sy.fract());
        let y1 = (y0 + 1).min(h - 1);
        for x in 0..w {
            let sx = (w as f32 / 3.0 + (x as f32 + 0.5) / 3.0 - 0.5).clamp(0.0, (w - 1) as f32);
            let (x0, fx) = (sx.floor() as usize, sx.fract());
            let x1 = (x0 + 1).min(w - 1);
            for c in 0..4 {
                let top = texel(x0, y0, c) * (1.0 - fx) + texel(x1, y0, c) * fx;
                let bottom = texel(x0, y1, c) * (1.0 - fx) + texel(x1, y1, c) * fx;
                rgba[(y * w + x) * 4 + c] = (top * (1.0 - fy) + bottom * fy).round() as u8;
            }
        }
    }
    omsi_texture::Image { width: img.width, height: img.height, rgba, has_alpha: img.has_alpha }
}

/// Give the lamps of a tile the hue its light map (the tile's own part, see
/// [`own_tile_of_light_map`]; north at the top row) shows under them, where the map is lit
/// there at all; their brightness stays.
fn tint_lights_from_light_map(lights: &mut [omsi_render::PointLight], img: &omsi_texture::Image, origin: DVec3) {
    let ts = omsi_map::tile_size();
    let (w, h) = (img.width as i64, img.height as i64);
    if w == 0 || h == 0 {
        return;
    }
    for l in lights.iter_mut() {
        let u = (l.position.x - origin.x) / ts;
        let v = (l.position.y - origin.y) / ts;
        if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
            continue;
        }
        let (cx, cy) = ((u * w as f64) as i64, ((1.0 - v) * h as f64) as i64);
        // the brightest texel within a few metres (the lamp stands over its pool's edge)
        let reach = ((6.0 / ts) * w as f64).ceil().max(1.0) as i64;
        let mut best = [0u8; 3];
        for y in (cy - reach).max(0)..=(cy + reach).min(h - 1) {
            for x in (cx - reach).max(0)..=(cx + reach).min(w - 1) {
                let i = ((y * w + x) * 4) as usize;
                let c = [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2]];
                if c.iter().map(|&v| v as u32).sum::<u32>() > best.iter().map(|&v| v as u32).sum::<u32>() {
                    best = c;
                }
            }
        }
        let peak = *best.iter().max().unwrap() as f32;
        if peak < 40.0 {
            continue;
        }
        let hue = [best[0] as f32 / peak, best[1] as f32 / peak, best[2] as f32 / peak];
        let bright = l.color.iter().cloned().fold(0.0f32, f32::max);
        l.color = [hue[0] * bright, hue[1] * bright, hue[2] * bright];
    }
}

/// An object whose night textures are lit by a `[NightMapMode]` timetable.
#[derive(Debug, Clone, Copy)]
pub struct NightMode {
    pub inst: usize,
    /// This object's in-use window and darkness threshold (see [`InUse`]).
    pub use_: InUse,
    pub slots: usize,
}

/// When a building with a `[NightMapMode]` is in use and when its windows are lit, as
/// OMSI decides once per object and every frame: in use
/// between `on` and `off` (seconds of the day; mode 2 homes 5.5-9.5 h until 22-24 h, mode 3
/// offices 6-8 h until 17-19 h on working days that are no holiday, mode 4 schools 6-8 h until
/// 14-16 h on school days, any other mode all day); lit while in use and the daylight under
/// `threshold` (0.6 for mode 0, else 0.3-0.75).
#[derive(Clone, Copy, Debug)]
pub struct InUse {
    pub mode: i32,
    pub on: f64,
    pub off: f64,
    pub threshold: f32,
}

/// The day as the in-use rules ask about it.
#[derive(Clone, Copy, Debug, Default)]
pub struct DayKind {
    pub workday: bool,
    pub holiday: bool,
    pub school_holiday: bool,
}

impl InUse {
    /// The window of object `seed` (its map id: the same building keeps its hours).
    pub fn new(mode: i32, seed: u64) -> InUse {
        let r = |k: u64| {
            let h = (seed ^ k.wrapping_mul(0x9E37_79B9_7F4A_7C15)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            ((h >> 11) % 1_000_000) as f64 / 1_000_000.0
        };
        let (on, off) = match mode {
            2 => (5.5 + 4.0 * r(1), 22.0 + 2.0 * r(2)),
            3 => (6.0 + 2.0 * r(1), 17.0 + 2.0 * r(2)),
            4 => (6.0 + 2.0 * r(1), 14.0 + 2.0 * r(2)),
            _ => (0.0, 24.0),
        };
        let threshold = if mode == 0 { 0.6 } else { (0.3 + 0.45 * r(3)) as f32 };
        InUse { mode, on: on * 3600.0, off: off * 3600.0, threshold }
    }

    pub fn in_use(&self, time: f64, day: DayKind) -> bool {
        let t = time.rem_euclid(86_400.0);
        let hours = t >= self.on && t <= self.off;
        match self.mode {
            3 => hours && day.workday && !day.holiday,
            4 => hours && day.workday && !day.holiday && !day.school_holiday,
            _ => hours,
        }
    }

    pub fn lit(&self, time: f64, day: DayKind, brightness: f32) -> bool {
        self.in_use(time, day) && brightness < self.threshold
    }
}

impl World {
    /// Light or darken the night textures of the objects with a `[NightMapMode]` timetable
    /// for this hour of the day (0..24).
    pub fn update_night_modes(&self, renderer: &Renderer, scene: &mut Scene, clock: &omsi_sim::SimClock, brightness: f32) {
        let day = self.day_kind(clock);
        for m in self.night_modes.lock().iter() {
            let v = if m.use_.lit(clock.time, day, brightness) { 1.0 } else { 0.0 };
            renderer.set_slot_night(scene, m.inst, &vec![v; m.slots]);
        }
    }

    /// A number plate from `registrations.txt` for a vehicle with `[registration_free]`
    /// (OMSI gives such a vehicle's `ident` a random line of it at the spawn).
    pub fn free_registration(&self, seed: u64) -> Option<String> {
        let list = self.registrations.get_or_init(|| {
            self.chrono_dirs
                .read()
                .iter()
                .rev()
                .chain(std::iter::once(&self.map_dir))
                .map(|d| omsi_cfg::resolve_path(d, "registrations.txt"))
                .find(|p| omsi_cfg::vfs::is_file(p))
                .map(|p| omsi_map::ailists::load_list(&p))
                .unwrap_or_default()
        });
        (!list.is_empty()).then(|| list[(seed % list.len() as u64) as usize].clone())
    }

    /// Working day, holiday and school holidays at `clock`'s date, from the map's
    /// `Holidays.txt`.
    pub fn day_kind(&self, clock: &omsi_sim::SimClock) -> DayKind {
        let cal = self.calendar.get_or_init(|| omsi_map::Calendar::load(&self.map_dir.join("Holidays.txt")).unwrap_or_default());
        let date = clock.date_code();
        DayKind { workday: clock.weekday() < 5, holiday: cal.is_holiday(date), school_holiday: cal.in_holiday_range(date) }
    }

    /// Switch the `NightlightA` material variants of static objects.
    pub fn set_lamps(&self, renderer: &Renderer, scene: &mut Scene, on: bool) {
        for (inst, slot, m_on, m_off) in self.night_slots.lock().iter() {
            renderer.set_material(scene, *inst, *slot, if on { *m_on } else { *m_off });
        }
    }

    /// Whether switch object `id` is set to its path `path` (None: no such switch, or the
    /// path has no `[switchdir]`).
    pub fn switch_set_to(&self, id: i64, path: u16) -> Option<bool> {
        let scripted = self.scripted.lock();
        let o = scripted.iter().find(|o| o.map_id == id)?;
        let d = (*o.ty.sco.path_switch_dir.get(path as usize)?)?;
        Some(o.inst.var("Switch").map(|v| (v - d as f32).abs() < 0.5).unwrap_or(false))
    }

    /// Set the railway signals: `aspects` gives each signal object's `Signal` (0 stop, 1 go,
    /// 2 go at the route's speed limit) as the traffic worked it out, and every signal
    /// learns what the next one shows (`NextSignal`) - a distant signal what its main
    /// signal shows.
    pub fn set_signals(&self, aspects: &HashMap<i64, f32>) {
        if self.signal_routes.is_empty() {
            return;
        }
        let mut next: HashMap<i64, f32> = HashMap::new();
        for r in &self.signal_routes {
            let own = aspects.get(&r.signal.0).copied().unwrap_or(0.0);
            if let Some(d) = r.dist_signal {
                let e = next.entry(d).or_insert(0.0);
                *e = e.max(own);
            }
            if let Some(n) = r.next_signal {
                let e = next.entry(r.signal.0).or_insert(0.0);
                *e = e.max(aspects.get(&n).copied().unwrap_or(0.0));
            }
        }
        let ids: hashbrown::HashSet<i64> = self.signal_routes.iter().flat_map(|r| std::iter::once(r.signal.0).chain(r.dist_signal)).collect();
        let mut scripted = self.scripted.lock();
        for o in scripted.iter_mut() {
            if !ids.contains(&o.map_id) {
                continue;
            }
            let a = aspects.get(&o.map_id).copied().unwrap_or(0.0);
            if o.inst.var("Signal") != Some(a) {
                o.inst.set_var("Signal", a);
                if omsi_cfg::env::var_os("OMSI_DEBUG_SIGNALS").is_some() {
                    log::info!("signal {} at ({:.0}, {:.0}) shows {a}", o.map_id, o.pos.x, o.pos.y);
                }
            }
            o.inst.set_var("NextSignal", next.get(&o.map_id).copied().unwrap_or(0.0));
        }
    }

    /// The echo at `p`: (reverberation time, how much of it is heard) - full inside an
    /// underpass's box, fading over its edge distance at the sides.
    pub fn reverb_at(&self, p: DVec3) -> (f32, f32) {
        let me = omsi_sim::collision::Obb::point(p, 0.01);
        let mut best = (0.0f32, 0.0f32);
        for (b, time, fade) in self.reverb_zones.lock().iter() {
            if p.z < b.z0 - 1.0 || p.z > b.z1 + 1.0 {
                continue;
            }
            let inside = (-me.separation(b)) as f32;
            let mix = (inside / fade.max(0.1)).clamp(0.0, 1.0);
            if mix > best.1 {
                best = (*time, mix);
            }
        }
        best
    }

    /// The `[htmltexture]` page of a scenery object a ray lands on (within `reach` metres).
    /// The nearest triangle of those objects decides, as for the bus's pages: a part of
    /// the object in front of its page takes the click away from it.
    pub fn html_object_hit(&self, origin: DVec3, dir: glam::Vec3, reach: f32) -> Option<PageHit> {
        let scripted = self.scripted.lock();
        let mut best: Option<(f32, Option<PageHit>)> = None;
        for o in scripted.iter().filter(|o| !o.htmls.is_empty()) {
            if (o.pos - origin).length() > reach as f64 + 60.0 {
                continue;
            }
            let local = (origin - o.pos).as_vec3();
            for mi in 0..o.instances.len() {
                let Some((data, o3d_mats, overrides)) = o.ty.meshes.get(mi) else { continue };
                if !o.inst.mesh_visible.get(mi).copied().unwrap_or(true) {
                    continue;
                }
                let xf = o.xf * o.inst.mesh_transforms.get(mi).copied().unwrap_or(Mat4::IDENTITY);
                let Some(hit) = omsi_geometry::ray_mesh_hit(local, dir, data, &xf) else { continue };
                if hit.t > reach || best.as_ref().is_some_and(|b| b.0 <= hit.t) {
                    continue;
                }
                // the page the hit material slot shows (a slot that shows none is in the way)
                let slot = data.slot_of(hit.index) as usize;
                let page = overrides
                    .iter()
                    .filter(|m| !m.item && omsi_sim::vehicle::override_slot(o3d_mats, m) == Some(slot))
                    .find_map(|m| m.use_script_texture)
                    .map(|n| n.max(0) as usize)
                    .filter(|n| o.htmls.iter().any(|(i, _)| i == n));
                let page = page.map(|page| PageHit {
                    t: hit.t,
                    map_id: o.map_id,
                    page,
                    u: hit.uv.x.clamp(0.0, 1.0),
                    v: hit.uv.y.clamp(0.0, 1.0),
                });
                best = Some((hit.t, page));
            }
        }
        best.and_then(|b| b.1)
    }

    /// A press, release or move on a page of a scenery object (see [`Self::html_object_hit`]).
    /// What the page does (`omsi.setVar`, `omsi.trigger`) reaches the object's script.
    pub fn html_object_pointer(&self, map_id: i64, page: usize, u: f32, v: f32, kind: omsi_sim::htmltex::PointerKind) -> bool {
        let mut scripted = self.scripted.lock();
        match scripted.iter_mut().find(|o| o.map_id == map_id) {
            Some(o) => o.inst.html_pointer(page, u, v, kind),
            None => false,
        }
    }

    /// The colour the tile's night light map (its own part, see [`own_tile_of_light_map`])
    /// has at `pos` (0..1, bilinear), or `None` where no light map is loaded: the light it
    /// throws on a vehicle standing there (Omsi.exe samples it at the vehicle's place,
    /// 0x61378c, for its ambient light and `Envir_Brightness`).
    pub fn light_map_light_at(&self, pos: DVec3) -> Option<glam::Vec3> {
        let ts = tile_size();
        let key = ((pos.x / ts).floor() as i32, (pos.y / ts).floor() as i32);
        let img = self.light_maps.lock().get(&key).cloned()?;
        let (w, h) = (img.width as usize, img.height as usize);
        if w == 0 || h == 0 || img.rgba.len() < w * h * 4 {
            return None;
        }
        let u = ((pos.x / ts - key.0 as f64) * w as f64 - 0.5).clamp(0.0, (w - 1) as f64);
        let v = ((1.0 - (pos.y / ts - key.1 as f64)) * h as f64 - 0.5).clamp(0.0, (h - 1) as f64);
        let (x0, y0) = (u.floor() as usize, v.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (fx, fy) = ((u - x0 as f64) as f32, (v - y0 as f64) as f32);
        let px = |x: usize, y: usize| {
            let i = (y * w + x) * 4;
            glam::Vec3::new(img.rgba[i] as f32, img.rgba[i + 1] as f32, img.rgba[i + 2] as f32) / 255.0
        };
        let top = px(x0, y0).lerp(px(x1, y0), fx);
        let bottom = px(x0, y1).lerp(px(x1, y1), fx);
        Some(top.lerp(bottom, fy))
    }

    /// Fill the light map atlas with the 5x5 tiles around `eye` (when it moved to another
    /// tile or tiles came or went): the splines and `[LightMapMapping]` objects are lit by it
    /// at night as the terrain is.
    pub fn update_light_map_atlas(&self, renderer: &Renderer, eye: DVec3) {
        // (`OMSI_NO_LIGHT_MAP=1`: the tiles' night light maps left out, for an A/B)
        if omsi_cfg::env::var_os("OMSI_NO_LIGHT_MAP").is_some() {
            return;
        }
        let ts = tile_size();
        let centre = ((eye.x / ts).floor() as i32, (eye.y / ts).floor() as i32);
        let generation = self.light_maps_generation.load(std::sync::atomic::Ordering::Relaxed);
        let mut last = self.light_map_atlas.lock();
        if *last == Some((centre, generation)) {
            return;
        }
        *last = Some((centre, generation));
        let n = omsi_render::LM_ATLAS_TILES as i32;
        let maps = self.light_maps.lock();
        for row in 0..n {
            for col in 0..n {
                // column from the west, row from the north
                let key = (centre.0 - n / 2 + col, centre.1 + n / 2 - row);
                renderer.set_light_map_tile((col as u32, row as u32), maps.get(&key).map(|a| a.as_ref()));
            }
        }
        let sw = ((centre.0 - n / 2) as f64 * ts, (centre.1 - n / 2) as f64 * ts);
        renderer.set_light_map_place(sw.0, sw.1, n as f64 * ts);
    }

    /// Throw the points the trains need (`Traffic::switch_requests`): a switch object whose
    /// `[path]` carries a `[switchdir]` gets that value in its script's `Switch` variable,
    /// and its blades turn with it.
    pub fn set_switches(&self, requests: &[(i64, u16)]) {
        if requests.is_empty() {
            return;
        }
        let mut scripted = self.scripted.lock();
        for &(id, path) in requests {
            let Some(o) = scripted.iter_mut().find(|o| o.map_id == id) else { continue };
            if let Some(Some(d)) = o.ty.sco.path_switch_dir.get(path as usize) {
                let was = o.inst.var("Switch");
                if o.inst.set_var("Switch", *d as f32) && was != Some(*d as f32) && omsi_cfg::env::var_os("OMSI_DEBUG_SWITCHES").is_some() {
                    log::info!("switch {} ({}) at ({:.0}, {:.0}) thrown to {d} for a train", id, o.ty.sco.path.display(), o.pos.x, o.pos.y);
                }
            }
        }
    }

    /// Move the particles of the placed objects within 1.5 km of `center`, their variables
    /// read from the object's script (the fireworks' frequency).
    pub fn update_particles(&self, dt: f32, center: DVec3) {
        let mut objs = self.particle_objects.lock();
        if objs.is_empty() {
            return;
        }
        let scripted = self.scripted.lock();
        static DEBUG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *DEBUG.get_or_init(|| omsi_cfg::env::var_os("OMSI_DEBUG_PARTICLES").is_some()) {
            let mut near: Vec<(f64, &ParticleObject)> = objs.values().flatten().map(|po| ((po.pos - center).length(), po)).collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (d, po) in near.iter().take(3) {
                log::info!("particle object {} at ({:.1}, {:.1}, {:.1}), {d:.0} m: {} particles", po.map_id, po.pos.x, po.pos.y, po.pos.z, po.set.particles().count());
            }
        }
        for list in objs.values_mut() {
            for po in list.iter_mut() {
                if (po.pos - center).length() > 1500.0 {
                    continue;
                }
                let inst = scripted.iter().find(|s| s.map_id == po.map_id).map(|s| &s.inst);
                let value = |n: &str| inst.and_then(|i| i.var(n)).unwrap_or(0.0);
                po.set.update(dt, po.pos, po.rot, &value);
            }
        }
    }

    /// The departures for the HTML pages of the player's vehicle: the stop names its pages asked
    /// for go to the boards, and the departures made for them come back into its host.
    pub fn sync_html_departures(&self, host: &mut omsi_sim::host::VehicleHost) {
        if host.html_departure_wants.is_empty() {
            return;
        }
        let mut boards = self.timetable_boards.lock();
        for k in &host.html_departure_wants {
            if !boards.wanted_names.contains(k) {
                boards.wanted_names.push(k.clone());
            }
        }
        if host.html_departures_gen != boards.departures_gen {
            host.html_departures = host
                .html_departure_wants
                .iter()
                .filter_map(|k| boards.departures.get(k).map(|l| (k.clone(), l.clone())))
                .collect();
            host.html_departures_gen = boards.departures_gen;
        }
    }

    /// Run the scripts and animations of the placed objects near `center` and push their
    /// mesh transforms / visibility to the renderer. `phase_of(controller, light)` gives the
    /// light's current state (the `TrafficLightPhase` value) and whether a vehicle is
    /// asking for it (`TrafficLightApproach`).
    pub fn update_scripted(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        dt: f32,
        center: DVec3,
        brightness: f32,
        phase_of: &dyn Fn(usize, usize) -> (f32, f32),
        audio: Option<&omsi_audio::AudioEngine>,
        muffled: bool,
    ) -> usize {
        self.update_particles(dt, center);
        let mut updated = 0;
        let now = self.script_clock();
        let day = self.day_kind(&now);
        let mut scripted = self.scripted.lock();
        let mut boards = self.timetable_boards.lock();
        let mut wanted: Vec<i64> = Vec::new();
        let mut wanted_names: Vec<String> = Vec::new();
        let mut texture_updates: Vec<(
            Arc<ObjectType>,
            Vec<usize>,
            Vec<usize>,
            HashMap<(usize, usize), bool>,
        )> = Vec::new();
        // First what every object's script is given (in order: the light programs and the
        // boards are read here), then the scripts themselves, side by side on the worker
        // threads (a city's hundreds of scripted objects took a core's worth of a frame),
        // then what they did, in order again.
        let mut inputs: Vec<Option<omsi_sim::scenery::SceneryVars>> = Vec::with_capacity(scripted.len());
        let controllers = self.controller_of_object.lock();
        for o in scripted.iter_mut() {
            let dist = (o.pos - center).length();
            if dist > 800.0 {
                inputs.push(None);
                if let (Some(a), Some(mut ss)) = (audio, o.sounds.take()) {
                    ss.stop_all(a);
                }
                continue;
            }
            // the object's own hours ([NightMapMode]): in use, and lit while in use and the
            // daylight under its own threshold (0.6, or 0.3-0.75 with a [NightMapMode])
            let use_ = InUse::new(o.ty.sco.night_map_mode, o.map_id as u64);
            let in_use = use_.in_use(now.time, day);
            let light = match (o.controller, o.light_parent) {
                (Some(c), _) => Some((c, o.light_index)),
                (None, Some((parent, li))) => controllers.get(&parent).map(|&c| (c, li)),
                _ => None,
            };
            let vars = omsi_sim::scenery::SceneryVars {
                nightlight: use_.lit(now.time, day, brightness) as i32 as f32,
                in_use: in_use as i32 as f32,
                traffic_light_phase: light.map(|(c, li)| phase_of(c, li).0).unwrap_or(omsi_sim::traffic::UNLINKED_PHASE as f32),
                traffic_light_approach: light.map(|(c, li)| phase_of(c, li).1).unwrap_or(0.0),
                switch: None,
            };
            // the scripts read the simulation's time of day (clocks, the display's blinking)
            if let Some(c) = &boards.clock {
                let own = &mut o.inst.host.clock;
                *own = c.clone();
                // (the update moves it on by `dt` again)
                if !own.paused {
                    own.time -= dt as f64;
                    own.run_time -= dt as f64;
                }
            }
            // a departure display: the buses due at its stop
            if let (true, Some(stop)) = (o.arrivals, o.var_parent) {
                wanted.push(stop);
                let now = boards.clock.as_ref().map(|c| c.time).unwrap_or(0.0);
                o.inst.host.arrivals = boards
                    .by_stop
                    .get(&stop)
                    .map(|l| {
                        l.iter()
                            .map(|(line, terminus, t)| omsi_sim::host::Arrival {
                                line: line.clone(),
                                terminus: terminus.clone(),
                                due: (t - now) as f32,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
            }
            // an HTML page that asks for departures by stop name
            if !o.htmls.is_empty() && dist < HTML_OBJECT_NEAR && !o.inst.host.html_departure_wants.is_empty() {
                for k in &o.inst.host.html_departure_wants {
                    if !wanted_names.contains(k) {
                        wanted_names.push(k.clone());
                    }
                }
                if o.inst.host.html_departures_gen != boards.departures_gen {
                    o.inst.host.html_departures = o
                        .inst
                        .host
                        .html_departure_wants
                        .iter()
                        .filter_map(|k| boards.departures.get(k).map(|l| (k.clone(), l.clone())))
                        .collect();
                    o.inst.host.html_departures_gen = boards.departures_gen;
                }
            }
            inputs.push(Some(vars));
        }
        drop(controllers);
        {
            use rayon::prelude::*;
            scripted.par_iter_mut().zip(inputs.par_iter()).for_each(|(o, vars)| {
                if let Some(vars) = vars {
                    o.inst.update(dt, vars);
                }
            });
        }
        for (o, vars) in scripted.iter_mut().zip(inputs.iter()) {
            let Some(nightlight) = vars.as_ref().map(|v| v.nightlight) else {
                continue;
            };
            let dist = (o.pos - center).length();
            // text textures from the script's strings whenever they change (`update` leaves
            // an unchanged one alone): read only on `Refresh_Strings`, a board whose string
            // was still empty at its first frame stayed blank for good (#367)
            if !o.texts.is_empty() {
                let _ = o.inst.take_refresh_strings();
                for (tex, st) in o.texts.iter_mut() {
                    let text = o.inst.str_var(st.def.variable.trim()).to_string();
                    if st.update(&text) {
                        let (w, h) = (st.def.width.max(1) as u32, st.def.height.max(1) as u32);
                        if let Some(rgba) = st.pending.take() {
                            // OMSI_DUMP_SCENERY_TEXT=<dir>: the pictures as drawn
                            if let Some(dir) = omsi_cfg::env::var_os("OMSI_DUMP_SCENERY_TEXT") {
                                let path = std::path::Path::new(&dir)
                                    .join(format!("text_{}.png", o.map_id));
                                log::info!(
                                    "scenery text of object {}: {text:?} -> {}",
                                    o.map_id,
                                    path.display()
                                );
                                if let Some(img) = image::RgbaImage::from_raw(w, h, rgba.clone()) {
                                    let _ = img.save(&path);
                                }
                            }
                            renderer.update_texture_mips(
                                scene,
                                *tex,
                                &Image {
                                    width: w,
                                    height: h,
                                    rgba,
                                    has_alpha: true,
                                },
                            );
                        }
                    }
                }
            }
            // [htmltexture] pages: only near the listener (a page is a whole browser frame)
            if !o.htmls.is_empty() && dist < HTML_OBJECT_NEAR {
                for (index, w, h, rgba) in o.inst.update_html_textures() {
                    if let Some((_, tex)) = o.htmls.iter().find(|(i, _)| *i == index) {
                        renderer.update_texture(scene, *tex, &Image { width: w, height: h, rgba, has_alpha: true });
                    }
                }
            }
            // [sound] of scenery objects: crossing bells, ambient loops
            let fired: Vec<String> = std::mem::take(&mut o.inst.host.fired_triggers);
            // (out of earshot with nothing playing: nothing to do - finding the sound file
            // for each of a city's scripted objects every frame took 1.8 ms)
            let near = dist < 300.0 || o.sounds.is_some();
            if let (Some(a), true) = (audio, near) {
                let ty = o.ty.clone();
                let path = ty.sound_path.get_or_init(|| {
                    let rel = ty.sco.sound.as_ref()?;
                    let dir = ty.sco.path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                    Some(omsi_cfg::resolve_path(&dir, rel))
                });
                if let Some(path) = path {
                    let inst = &o.inst;
                    self.object_sounds(a, &mut o.sounds, path, dist, muffled, o.pos, o.xf, &|n| inst.var(n), &fired);
                }
            }
            for (inst, slot, base, item, var) in &o.variants {
                let x = if var.trim().eq_ignore_ascii_case("NightlightA") {
                    nightlight
                } else {
                    var.trim()
                        .parse()
                        .ok()
                        .or_else(|| o.inst.var(var))
                        .unwrap_or(0.0)
                };
                renderer.set_material(scene, *inst, *slot, if change_picks_item(x) { *item } else { *base });
            }
            if !o.ty.dynamic_textures.is_empty() {
                let selection = scenery_texture_selection(&o.ty, &o.inst);
                let switches = o
                    .variants
                    .iter()
                    .map(|(inst, slot, _, _, var)| {
                        let value = if var.trim().eq_ignore_ascii_case("NightlightA") {
                            nightlight
                        } else {
                            var.trim()
                                .parse::<f32>()
                                .ok()
                                .or_else(|| o.inst.var(var))
                                .unwrap_or(0.0)
                        };
                        ((*inst, *slot), change_picks_item(value))
                    })
                    .collect();
                texture_updates.push((o.ty.clone(), selection, o.instances.clone(), switches));
            }
            for (k, ((inst, xf), &visible)) in o.instances.iter().zip(&o.inst.mesh_transforms).zip(&o.inst.mesh_visible).enumerate() {
                renderer.set_transform(scene, *inst, o.pos, o.xf * *xf);
                // (the slots its `[alphascale]` variables fade, see `ScriptedObject::alpha_slots`)
                let alpha = o.alpha_slots.get(k).filter(|l| !l.alpha.is_empty()).map(|l| l.values(&|v| v.trim().parse::<f32>().ok().or_else(|| o.inst.var(v))).0);
                let p = &mut scene.instances[*inst];
                match alpha {
                    Some(a) => {
                        if p.visible != visible || o.alpha_last.get(k) != Some(&a) {
                            renderer.set_params(scene, *inst, &a, visible, &[]);
                            if let Some(last) = o.alpha_last.get_mut(k) {
                                *last = a;
                            }
                        }
                    }
                    None => {
                        if p.visible != visible {
                            renderer.set_params(scene, *inst, &[], visible, &[]);
                        }
                    }
                }
            }
            updated += 1;
        }
        wanted.sort_unstable();
        wanted.dedup();
        boards.wanted = wanted;
        boards.wanted_names = wanted_names;
        drop(scripted);
        drop(boards);
        // Scenery placement takes the GPU-cache lock before the script list. Apply dynamic
        // texture changes after releasing the script-list lock to keep that lock order
        // consistent.
        for (ty, selection, instances, switches) in texture_updates {
            let variant = {
                let mut gpu = self.gpu.lock();
                gpu.dynamic_texture_variant(
                    renderer,
                    scene,
                    Arc::as_ptr(&ty) as usize,
                    &selection,
                    &self.root,
                    &HashMap::new(),
                )
            };
            if let Some(rows) = variant {
                for (mi, row) in rows.iter().enumerate() {
                    let Some(&mesh_inst) = instances.get(mi) else {
                        continue;
                    };
                    for (slot, pair) in row.iter().enumerate() {
                        let Some((base, item)) = pair else { continue };
                        let item_on = switches.get(&(mesh_inst, slot)).copied().unwrap_or(false);
                        renderer.set_material(
                            scene,
                            mesh_inst,
                            slot,
                            if item_on { *item } else { *base },
                        );
                    }
                }
            }
        }
        // the lamps' own sounds (a level crossing's bell): their scripts run with the light
        // programs (`Traffic::sync`), what they fired is heard here
        for lamp in self.light_objects.lock().iter() {
            // (only the lamps that have a sound, and near enough to hear: a city map has
            // nearly a thousand lamps, and going through all of them took 1.8 ms a frame)
            let Some(path) = lamp.sound.as_ref() else { continue };
            let Some(script) = lamp.script.as_ref() else { continue };
            let dist = (lamp.pos - center).length();
            if dist >= 300.0 {
                if let (Some(a), Some(mut ss)) = (audio, lamp.sounds.lock().take()) {
                    ss.stop_all(a);
                }
                // (what it fired out of earshot is not heard later)
                script.lock().host.fired_triggers.clear();
                continue;
            }
            let mut inst = script.lock();
            let fired = std::mem::take(&mut inst.host.fired_triggers);
            if let Some(a) = audio {
                let mut sounds = lamp.sounds.lock();
                self.object_sounds(a, &mut sounds, path, dist, muffled, lamp.pos, lamp.xf, &|n| inst.var(n), &fired);
            }
        }
        updated
    }

    /// Play a scenery object's `[sound]` (the config at `path`) while the listener is within
    /// 300 m: loaded when it comes near, stopped when it goes.
    #[allow(clippy::too_many_arguments)]
    fn object_sounds(
        &self,
        a: &omsi_audio::AudioEngine,
        sounds: &mut Option<omsi_audio::SoundSet>,
        path: &Path,
        dist: f64,
        muffled: bool,
        pos: DVec3,
        xf: Mat4,
        var: &dyn Fn(&str) -> Option<f32>,
        fired: &[String],
    ) {
        if dist >= 300.0 {
            if let Some(mut ss) = sounds.take() {
                ss.stop_all(a);
            }
            return;
        }
        if sounds.is_none() {
            // read once per file, the clips in the background (see AudioEngine::clips_ready)
            let cfg = self
                .sound_cfgs
                .lock()
                .entry(path.to_path_buf())
                .or_insert_with(|| {
                    omsi_vehicle::SoundCfg::load(path)
                        .map_err(|e| log::warn!("{e}"))
                        .ok()
                        .map(Arc::new)
                })
                .clone();
            if let Some(cfg) = cfg {
                let sdir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                if a.clips_ready(&omsi_audio::SoundSet::clip_paths(&cfg, &sdir)) {
                    log::info!("scenery sound {}: {} sounds ({:.0} m away)", path.display(), cfg.sounds.len(), dist);
                    let mut ss = omsi_audio::SoundSet::new(a, &cfg, &sdir);
                    ss.master = crate::sound_gain(&crate::SOUND_SCENERY);
                    *sounds = Some(ss);
                }
            }
        }
        if let Some(ss) = sounds.as_mut() {
            // scenery (a fountain, machinery, a level crossing bell): heard through the
            // player's own bodywork and glass just like any other sound from outside the cabin
            ss.set_muffled(muffled);
            let xf = Mat4::from_translation(pos.as_vec3()) * xf;
            ss.update(a, var, &xf, fired);
        }
    }
}

/// Give the `[smoothskin]` meshes of a vehicle instance copies of their own: their vertices
/// are rewritten as the joint turns, which a mesh shared between every instance of the type
/// (the AI pool, and the player's own set before this ran) cannot be. Called for the
/// player's vehicle and for every AI copy alike (`render.set` says which).
fn own_skinned_meshes(
    renderer: &Renderer,
    scene: &mut Scene,
    vt: &omsi_sim::VehicleType,
    render: &mut VehicleRender,
) {
    for (i, vm) in vt.meshes.iter().enumerate() {
        if vm.skin.is_empty() || vm.data.positions.is_empty() {
            continue;
        }
        let Some(&inst) = render.instances.get(i) else {
            continue;
        };
        // (a mesh OMSI_ONLY_MESH / OMSI_HIDE_MESH left out stays out)
        if scene
            .meshes
            .get(scene.instances[inst].mesh)
            .map(|m| m.ranges.is_empty())
            .unwrap_or(true)
        {
            continue;
        }
        let id = renderer.add_mesh(scene, &vm.data);
        renderer.set_instance_mesh(scene, inst, id);
        render.skinned.push((i, id, Vec::new()));
    }
    if !render.skinned.is_empty() {
        let names = render
            .skinned
            .iter()
            .map(|(i, _, _)| vt.model.meshes[vt.meshes[*i].def_index].file.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let name = vt
            .def
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        // the player's own vehicle says so once; an AI copy (the timetable's articulated
        // buses spawn dozens of these) only on demand
        match render.set {
            None => log::info!("{name}: {} skinned meshes ({names})", render.skinned.len()),
            Some(_) => log::debug!(
                "{name} (AI): {} skinned meshes ({names})",
                render.skinned.len()
            ),
        }
    }
}

/// Reshape the skinned meshes of a vehicle and its coupled parts whose bones moved (the
/// player's own, or an AI copy's - see `own_skinned_meshes`).
pub fn sync_skinned(
    renderer: &Renderer,
    scene: &mut Scene,
    vehicle: &mut omsi_sim::VehicleInstance,
    render: &mut VehicleRender,
    parts: &mut [VehicleRender],
) {
    for (i, id, last) in render.skinned.iter_mut() {
        let key = vehicle.skin_key(*i);
        if key == *last {
            continue;
        }
        if let Some((pos, nrm)) = vehicle.skinned(*i) {
            renderer.update_mesh(scene, *id, &pos, &nrm, &vehicle.ty.meshes[*i].data.uvs);
        }
        *last = key;
    }
    let n_vars = vehicle.state.vars.len();
    for (t, r) in vehicle.trailers.iter_mut().zip(parts.iter_mut()) {
        for (i, id, last) in r.skinned.iter_mut() {
            let key = t.skin_key(*i);
            if key == *last {
                continue;
            }
            if let Some((pos, nrm)) = t.skinned(*i, n_vars) {
                renderer.update_mesh(scene, *id, &pos, &nrm, &t.ty.meshes[*i].data.uvs);
            }
            *last = key;
        }
    }
}

/// Upload the text and script textures of a vehicle that changed since the last frame.
/// Switch the material of every slot a variable controls: `[matl_freetex]` loads the file a
/// string variable names, `[texchanges]` picks an entry of its master, `[matl_change]` picks
/// between the plain material and the `[matl_item]` variant.
pub fn sync_vehicle_materials(
    renderer: &Renderer,
    scene: &mut Scene,
    vehicle: &omsi_sim::VehicleInstance,
    render: &mut VehicleRender,
) {
    sync_materials(renderer, scene, vehicle, render);
}

/// A coupled part's switched materials and text textures, driven by the variables of the
/// vehicle it is coupled to (its scripts are shared with it).
pub fn sync_vehicle_part(
    renderer: &Renderer,
    scene: &mut Scene,
    main: &omsi_sim::VehicleInstance,
    part: &mut omsi_sim::vehicle::TrailerPart,
    render: &mut VehicleRender,
) {
    sync_interior_lamps(
        renderer,
        scene,
        &part.ty,
        part.position,
        part.body_rotation(),
        |n| main.var(n),
        render,
    );
    sync_materials(renderer, scene, main, render);
    for i in part.update_text_textures(main) {
        if let (Some(Some(tex)), Some(img)) = (
            render.text_textures.get(i),
            part.text_textures[i].pending.take(),
        ) {
            let d = &part.text_textures[i].def;
            renderer.update_texture_mips(
                scene,
                *tex,
                &Image {
                    width: d.width.max(1) as u32,
                    height: d.height.max(1) as u32,
                    rgba: img,
                    has_alpha: true,
                },
            );
        }
    }
}

/// Resolve a vehicle `[matl_freetex]` name. OMSI add-ons often write paths such as
/// `..\\Texture\\mb_pmon\\warning.bmp`: if the normal lookup misses, retry the part
/// below the `Texture` component against the vehicle's texture search directories.
fn find_vehicle_freetex(name: &str, dirs: &[&Path]) -> Option<PathBuf> {
    if let Some(path) = omsi_texture::find_texture(name, dirs) {
        return Some(path);
    }
    let normalized = name.trim().replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|p| !p.is_empty()).collect();
    let texture = parts.iter().position(|p| p.eq_ignore_ascii_case("Texture"))?;
    let rel = parts.get(texture + 1..)?.join("/");
    if rel.is_empty() {
        return None;
    }
    omsi_texture::find_texture(&rel, dirs)
}

fn sync_materials(
    renderer: &Renderer,
    scene: &mut Scene,
    vehicle: &omsi_sim::VehicleInstance,
    render: &mut VehicleRender,
) {
    for v in &mut render.variants {
        let item_has_freetex = v.free.iter().any(|f| f.item_only);
        // the light maps switched on now (several `[matl_lightmap]`s, see `MultiLight`)
        let mask = v.lights.as_ref().map(|l| {
            let mut mask = 0u32;
            for (k, (_, var)) in l.maps.iter().enumerate() {
                let x = var.trim().parse::<f32>().ok().or_else(|| vehicle.var(var)).unwrap_or(0.0);
                // (on at 0.5, as each map's texture stage is, 0x7fe51f: a variable a script
                // dims through 0.1 lit the map at full)
                if x >= 0.5 {
                    mask |= 1 << k;
                }
            }
            mask
        });
        // A free texture under several light maps (an on-board unit's display dimmed by
        // two of them, #1650) is made with the maps switched on: the light maps' own
        // materials knew nothing of the free texture and put the slot's default texture
        // over the display, and a new picture came with the last map whatever its variable.
        let lights = &mut v.lights;
        for f in &mut v.free {
            let name = vehicle.str_var(&f.var);
            let name = name.trim().to_string();
            let key = match mask {
                Some(m) => format!("{}#{m}", name.to_ascii_lowercase()),
                None => name.to_ascii_lowercase(),
            };
            if f.current.as_deref() != Some(key.as_str()) {
                f.current = Some(key.clone());
                let pair = match f.cache.get(&key) {
                    Some(p) => *p,
                    None => {
                        let dirs: Vec<&Path> = f.dirs.iter().map(|p| p.as_path()).collect();
                        let found = if name.is_empty() {
                            None
                        } else {
                            let resolved = find_vehicle_freetex(&name, &dirs);
                            if resolved.is_none() {
                                log::warn!(
                                    "vehicle [matl_freetex] '{}' = {:?}: texture not found",
                                    f.var,
                                    name
                                );
                            }
                            resolved.and_then(|path| {
                                let mut shared = f.shared.lock();
                                if let Some(e) = shared.get_mut(&path) {
                                    e.1 += 1;
                                    f.held.push(path);
                                    return Some(e.0);
                                }
                                let (img, worth) = f.textures.get_gpu_fast(&path)?;
                                let id = renderer.add_texture_data(scene, &img);
                                if worth {
                                    f.wants_upgrade.lock().push(path.clone());
                                }
                                attach_pbr(renderer, scene, &path, id);
                                shared.insert(path.clone(), (id, 1));
                                f.held.push(path);
                                Some(id)
                            })
                        };
                        // An empty string or a file not found leaves the slot its own
                        // texture from the mesh (with its addressing): a roller blind's idle
                        // "next" band then stays out of sight in its transparent border
                        // instead of covering the display as an untextured white plane.
                        let mut spec = match found {
                            Some(tex) => v.spec.with_freetex(f.key, tex, f.diffuse, f.item_only),
                            None => v.spec.clone(),
                        };
                        if let (Some(m), Some(l)) = (mask, lights.as_mut()) {
                            let tex = if m == 0 { None } else { l.composite(renderer, scene, m) };
                            spec.set_lightmap(tex);
                        }
                        let p = spec.build(renderer, scene, v.base_tex);
                        f.cache.insert(key, p);
                        p
                    }
                };
                if !f.item_only {
                    v.base = pair.0;
                }
                if f.item_only || !item_has_freetex {
                    v.item = pair.1;
                }
            }
        }
        if let (Some(l), Some(mask)) = (&mut v.lights, mask) {
            // (with a free texture, the free texture's materials above carry the maps)
            if mask != l.current && v.free.is_empty() {
                l.current = mask;
                let pair = if let Some(p) = l.cache.get(&mask) {
                    *p
                } else {
                    // none on: no light map at all (the set's own materials have the last
                    // map, lit whatever its variable said)
                    let tex = if mask == 0 { None } else { l.composite(renderer, scene, mask) };
                    let mut spec = v.spec.clone();
                    spec.set_lightmap(tex);
                    let p = spec.build(renderer, scene, v.base_tex);
                    l.cache.insert(mask, p);
                    p
                };
                v.base = pair.0;
                v.item = pair.1;
            }
        }
        if let Some(inst) = render.instances.get(v.mesh) {
            let m = v.material(|n| vehicle.var(n));
            if omsi_cfg::env::var("OMSI_DEBUG_VARIANTS").ok().is_some_and(|f| !f.is_empty() && v.var.to_ascii_lowercase().contains(&f.to_ascii_lowercase())) {
                log::info!("variant mesh {} slot {} var {} = {:?}: material {m} (base {}, item {})", v.mesh, v.slot, v.var, vehicle.var(&v.var), v.base, v.item);
            }
            renderer.set_material(scene, *inst, v.slot, m);
        }
    }
}

/// A vehicle's `[interiorlight]`s (`variable range r g b x y z`) as lamps for this frame:
/// points of light at their place in the vehicle, as strong as their variable (0..1) times
/// `range` (1 for a saloon lamp, 0.4 for a door lamp, 2 for the LiAZ's saloon rows - a door
/// lamp 0.4 m across could not reach the step 2 m below it, so it is no distance), each
/// lighting only the meshes that list it in
/// their `[illumination_interior]` - OMSI's four per mesh, or as many as a model lists
/// (up to `omsi_render::MAX_LAMPS_PER_MESH`), as OMSI switches those lights on
/// for just that mesh. Every set of lamps some mesh names gets a run of slots of its own
/// (the LiAZ 5292 has 32 lamps; only the first eight were drawn, and its saloon stayed dark).
/// The seats' sets (`PassPos::illumination`, the lamps that light a person sitting there)
/// get theirs too: see [`VehicleRender::seat_lamps`].
fn sync_interior_lamps(
    renderer: &Renderer,
    scene: &mut Scene,
    ty: &omsi_sim::VehicleType,
    position: DVec3,
    rotation: Mat4,
    var: impl Fn(&str) -> Option<f32>,
    render: &VehicleRender,
) {
    let n = ty.model.interior_lights.len();
    if n == 0 || render.instances.is_empty() {
        return;
    }
    let blocks = render.interior_blocks.get_or_init(|| {
        let mut blocks: Vec<(u32, Vec<usize>)> = Vec::new();
        let mut sets: Vec<(usize, Vec<usize>)> = Vec::new();
        for (i, _) in render.instances.iter().enumerate() {
            let Some(vm) = ty.meshes.get(i) else { continue };
            let set = lamp_set(&ty.model.meshes[vm.def_index].illumination_interior, n);
            if !set.is_empty() {
                sets.push((i, set));
            }
        }
        let mut distinct: Vec<Vec<usize>> = Vec::new();
        for (_, set) in &sets {
            if !distinct.contains(set) {
                distinct.push(set.clone());
            }
        }
        if let Some(cabin) = crate::driver::cabin_of(&ty.def) {
            for seat in cabin.driver_positions.iter().chain(&cabin.pass_positions) {
                let set = lamp_set(&seat.illumination, n);
                if !set.is_empty() && !distinct.contains(&set) {
                    distinct.push(set);
                }
            }
        }
        let total: u32 = distinct.iter().map(|d| d.len() as u32).sum();
        if omsi_cfg::env::var_os("OMSI_DEBUG_INTERIOR").is_some() {
            log::info!("interior lamps of {}: {} lamps, {} instances, {} meshes lit by sets {:?}", ty.def.path.display(), n, render.instances.len(), sets.len(), distinct);
        }
        if total == 0 {
            return blocks;
        }
        let first = renderer.alloc_interior_lights(scene, total);
        render.interior_lamps.set(Some((first, total)));
        let mut at = first;
        for d in distinct {
            blocks.push((at, d.clone()));
            at += d.len() as u32;
        }
        for (i, set) in &sets {
            if let Some(b) = blocks.iter().find(|b| &b.1 == set) {
                renderer.set_interior_lamps(scene, render.instances[*i], b.0, set.len() as u32);
            }
        }
        blocks
    });
    for (first, set) in blocks {
        for (k, &li) in set.iter().enumerate() {
            let il = &ty.model.interior_lights[li];
            // on or off: OMSI enables the lamp when its variable is 0.5 or more
            //, there is no dimming
            let on = il
                .variable
                .trim()
                .parse::<f32>()
                .ok()
                .or_else(|| var(&il.variable))
                .unwrap_or(0.0)
                >= 0.5;
            let at = rotation.transform_vector3(glam::Vec3::from(il.pos));
            renderer.set_interior_light(
                scene,
                first + k as u32,
                omsi_render::PointLight {
                    position: position + at.as_dvec3(),
                    // OMSI's Direct3D light: a point light of the
                    // colour / 255, Range 100 m, attenuation 1 / (d² / range²) - full
                    // light at `range` metres, stronger closer in, a quarter at twice
                    radius: 100.0,
                    core: il.range.max(0.01),
                    color: [il.color[0] / 255.0, il.color[1] / 255.0, il.color[2] / 255.0],
                    intensity: if on { 1.0 } else { 0.0 },
                    ..Default::default()
                },
            );
        }
    }
}

/// The lamps (indices into the model's `[interiorlight]`s) a mesh's or a seat's
/// `[illumination_interior]` names: those that exist, each once, as many as a mesh may have.
fn lamp_set(indices: &[i32], n: usize) -> Vec<usize> {
    let mut set: Vec<usize> = Vec::new();
    for &k in indices {
        if k >= 0 && (k as usize) < n && !set.contains(&(k as usize)) && set.len() < omsi_render::MAX_LAMPS_PER_MESH as usize {
            set.push(k as usize);
        }
    }
    set
}

impl VehicleRender {
    /// The lamp slots (first, count) for `set_interior_lamps` that light a person on a seat
    /// with these four lamps (`PassPos::illumination`) in a vehicle of `n` lamps; None
    /// before the vehicle's lamps are first synced, or when none of them exists.
    pub fn seat_lamps(&self, n: usize, lamps: &[i32; 4]) -> Option<(u32, u32)> {
        let set = lamp_set(lamps, n);
        let blocks = self.interior_blocks.get()?;
        blocks.iter().find(|b| b.1 == set).map(|b| (b.0, set.len() as u32))
    }
}

pub fn sync_vehicle_textures(
    renderer: &Renderer,
    scene: &mut Scene,
    vehicle: &mut omsi_sim::VehicleInstance,
    render: &VehicleRender,
    budget: &mut usize,
) {
    vehicle.update_html_textures();
    sync_interior_lamps(
        renderer,
        scene,
        &vehicle.ty,
        vehicle.position,
        vehicle.body_rotation(),
        |n| vehicle.var(n),
        render,
    );
    for i in vehicle.update_text_textures() {
        if let (Some(Some(tex)), Some(img)) = (
            render.text_textures.get(i),
            vehicle.text_textures[i].pending.take(),
        ) {
            let d = &vehicle.text_textures[i].def;
            renderer.update_texture_mips(
                scene,
                *tex,
                &Image {
                    width: d.width.max(1) as u32,
                    height: d.height.max(1) as u32,
                    rgba: img,
                    has_alpha: true,
                },
            );
        }
    }
    let mut rebound = Vec::new();
    for (i, st) in vehicle.host.script_textures.iter_mut().enumerate() {
        // (far away what the scripts redraw goes up every half second: `displays_far`)
        if !render.displays_far {
            if let Some(Some(tex)) = render.script_textures.get(i) {
                if *budget == 0 {
                    continue;
                }
                let Some(rgba) = st.take_upload() else { continue };
                *budget = budget.saturating_sub(rgba.len());
                let img = Image {
                    width: st.width,
                    height: st.height,
                    rgba,
                    has_alpha: true,
                };
                if st.mipmaps {
                    if renderer.update_texture_mips(scene, *tex, &img) {
                        rebound.push(*tex);
                    }
                } else {
                    renderer.update_texture(scene, *tex, &img);
                }
            }
        }
    }
    renderer.rebind_textures(scene, &rebound);
}

/// Distance (m) up to which the `[htmltexture]` pages of scenery objects are kept running.
pub const HTML_OBJECT_NEAR: f64 = 60.0;

/// Distance (m) beyond which what a vehicle's scripts redraw is uploaded only every half
/// second (the picture itself stays: see `Traffic::sync`).
pub const DISPLAYS_FAR: f64 = 50.0;

/// A texture name that stands for "no texture": exporters write `null.bmp` into slots
/// that have none (the SD202's IBIS key click spots). The slot shows its material colour;
/// nothing is looked up, and nothing is reported missing.
pub(crate) fn is_null_texture(name: &str) -> bool {
    let n = name.trim();
    n.is_empty()
        || Path::new(&n.replace('\\', "/"))
            .file_stem()
            .is_some_and(|s| s.eq_ignore_ascii_case("null"))
}

fn scenery_texture_key(name: &str) -> String {
    name.trim().replace('\\', "/").to_ascii_lowercase()
}

fn scenery_texture_selection(
    ot: &ObjectType,
    inst: &omsi_sim::scenery::SceneryInstance,
) -> Vec<usize> {
    ot.dynamic_textures
        .iter()
        .map(|group| {
            let Some(value) = inst.var(&group.variable) else {
                return usize::MAX;
            };
            if !value.is_finite() || value < 0.0 {
                return usize::MAX;
            }
            let index = value.trunc() as usize;
            if index < group.choices.len() {
                index
            } else {
                usize::MAX
            }
        })
        .collect()
}

/// The Direct3D material of a slot as OMSI sets it: a `[matl_allcolor]` (diffuse rgba,
/// ambient rgb, specular rgb, emissive rgb, power) replaces the o3d file's material. Returns (diffuse colour, emissive colour, specular colour and power).
/// The diffuse colour modulates the texture; its alpha only counts where there is no
/// texture (with one, the texture's alpha is used alone, as D3D's default stage does).
/// The emissive colour lights the texture by itself (the NL202's interior display, the
/// lamps of a traffic light); the specular term is the sun's highlight.
fn d3d_material(
    m: &omsi_o3d::Material,
    allcolor: Option<[f32; 14]>,
    textured: bool,
) -> ([f32; 4], [f32; 3], [f32; 4], [f32; 3]) {
    // (the ambient colour: Omsi.exe gives every o3d slot a white one, 0x7c62f8, and a
    // textured .x slot too, 0x7c6d2d; a [matl_allcolor] sets its own)
    let (diffuse, emissive, specular, power, ambient) = match allcolor {
        Some(v) => (
            [v[0], v[1], v[2], v[3]],
            [v[10], v[11], v[12]],
            [v[7], v[8], v[9]],
            v[13],
            [v[4], v[5], v[6]],
        ),
        None => (m.diffuse, m.emissive, m.specular, m.specular_power, [1.0; 3]),
    };
    let clamp01 = |x: f32| {
        if x.is_finite() {
            x.clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let color = [
        clamp01(diffuse[0]),
        clamp01(diffuse[1]),
        clamp01(diffuse[2]),
        if textured { 1.0 } else { clamp01(diffuse[3]) },
    ];
    let emissive = emissive.map(clamp01);
    let specular = specular.map(clamp01);
    // D3D ignores the specular colour without a power to raise the highlight to
    let power = if power.is_finite() && power >= 1.0 && specular.iter().any(|c| *c > 0.004) {
        power.min(256.0)
    } else {
        0.0
    };
    (
        color,
        emissive,
        [specular[0], specular[1], specular[2], power],
        ambient.map(clamp01),
    )
}

/// The material manager's depth and reflection settings of a slot's `[matl]` commands
/// (`bump`: the loaded `[matl_bumpmap]` height map and its factor).
fn material_extra(
    ov: &[&MaterialDef],
    env_mask: Option<TextureId>,
    bump: Option<(TextureId, f32)>,
    specular: [f32; 4],
) -> MaterialExtra {
    MaterialExtra {
        env_mask,
        no_z_write: ov.iter().any(|o| o.no_z_write),
        writes_depth: false,
        // `[matl_noZcheck]` leaves Omsi.exe's depth test on: its draw of the slot (0x7fd6c4)
        // never reads the flag, which only adds a colourless stencil pass marking the panes
        // for the raindrops (0x7c32c4 -> 0x7fc58c, ZENABLE 1, blend ZERO/ONE). Taken as "no
        // depth test", the Sprinter's inner window glass (flagged so) was drawn over the
        // body skin round every opening. OMSI_NOZCHECK_BIAS=1: the old reading.
        no_z_check: ov.iter().any(|o| o.no_z_check) && omsi_cfg::env::var_os("OMSI_NOZCHECK_BIAS").is_some(),
        z_bias: ov.iter().map(|o| o.z_bias).find(|b| *b != 0).unwrap_or(0),
        ambient: None,
        specular,
        bump: bump.filter(|b| b.1.is_finite() && b.1 != 0.0),
        glass: false,
        night_switched: false,
        rain_film: false,
        water: false,
        display: false,
        screen: false,
        led: false,
        no_map_lights: false,
        tree: false,
        sway: None,
        moisture: 0.0,
        transmap_declared: ov.iter().any(|o| o.transmap.is_some()),
        // (the last addressing command of the slot decides; the colour is given in bytes)
        border: ov
            .iter()
            .rev()
            .find(|o| o.tex_address != omsi_model::TexAddress::Wrap)
            .filter(|o| o.tex_address == omsi_model::TexAddress::Border)
            .map(|o| o.border_color.map(|c| (c / 255.0).clamp(0.0, 1.0))),
        metal_ok: false,
    }
}

/// The addressing of a slot's textures: its last `[matl_texadress_*]` command decides (the
/// border mode is clamped, its colour comes with `material_extra`).
fn tex_addressing<'a>(ov: impl DoubleEndedIterator<Item = &'a MaterialDef>) -> omsi_render::TexAddressing {
    use omsi_model::TexAddress as A;
    use omsi_render::TexAddressing as R;
    match ov.rev().map(|o| o.tex_address).find(|a| *a != A::Wrap) {
        None | Some(A::Wrap) => R::Wrap,
        Some(A::Mirror) => R::Mirror,
        Some(A::Clamp | A::Border) => R::Clamp,
        Some(A::MirrorOnce) => R::MirrorOnce,
    }
}

/// The key a `[matl_bumpmap]` height map of `path` is kept under (the same file may be a
/// colour texture as well).
fn bump_key(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}#bump", path.display()))
}

/// Whether a texture file is a season's snow picture: it lies in a `WinterSnow` folder
/// (`Texture\WinterSnow\gras.bmp`, any case), where the snow weather finds the map's
/// snowy textures, or in a `WinterSnowfall` one, its snowy roads.
fn is_snow_picture(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str().to_str().is_some_and(|s| s.eq_ignore_ascii_case("WinterSnow") || s.eq_ignore_ascii_case("WinterSnowfall")))
}

/// A PBR set beside the diffuse texture `path` (`foo_n.png` and the rest, see
/// `omsi_texture::pbr`), put up and tied to texture `id` for the materials made with it.
pub(crate) fn attach_pbr(renderer: &Renderer, scene: &mut Scene, path: &Path, id: TextureId) {
    // (and a season's snow picture is known as one: it gets no snow laid over it, #879)
    if is_snow_picture(path) {
        scene.snow_textures.insert(id);
    }
    if omsi_cfg::env::var_os("OMSI_NO_PBR").is_some() {
        return;
    }
    let files = omsi_texture::pbr::find(path);
    if files.is_empty() {
        return;
    }
    if let Some(set) = omsi_texture::pbr::load_set(&files) {
        log::info!("PBR maps for {}: normal {:?}, occlusion/roughness/metal {:?}", path.display(), set.normal.as_ref().map(|i| (i.width, i.height)), set.flags);
        renderer.add_pbr_maps(scene, id, &set);
    }
}

/// The texture data for a key of the texture maps: a file, or a file's bump height map.
fn load_texture_key(key: &Path, compress: bool) -> Option<TextureData> {
    let k = key.to_string_lossy();
    match k.strip_suffix("#bump") {
        Some(file) => omsi_texture::decode_file(Path::new(file))
            .map_err(|e| log::warn!("{e}"))
            .ok()
            .map(|img| omsi_texture::gpu::prepare_bump(&img, compress)),
        None => {
            if compress {
                omsi_texture::gpu::load_gpu(key).ok().map(|t| t.0)
            } else {
                omsi_texture::gpu::load_gpu_fast(key).ok().map(|t| t.0)
            }
        }
    }
}

/// Decide the alpha mode of an o3d material from the model.cfg `[matl]` overrides.
pub(crate) fn material_alpha(
    materials: &[omsi_o3d::Material],
    slot: usize,
    overrides: &[MaterialDef],
) -> AlphaMode {
    // the plain [matl] overrides of this slot decide; without one: opaque. A
    // `[matl_change]` record only opens the variants (`[matl_item]`) and says nothing of the
    // slot's own look: a `[matl]` of the same slot after it does. (The LED matrices of
    // churaPixel/Krüger++ open a change first and give the slot `[matl_alpha] 2` and the
    // script texture as its mask in a `[matl]` after it: taken as opaque from the change,
    // the mask cut nothing and the whole panel was lit.)
    // Several plain [matl] of one slot are one material in OMSI: each selects it again and
    // the commands after it modify it, so the last `[matl_alpha]` among them counts. (Taken
    // from the first block alone, an alpha-tested texture whose `[matl_alpha]` sits in a
    // second [matl] was drawn opaque, its transparent parts as solid areas.) omsi-model
    // already joins blocks spelt the same; this covers those that reach the slot otherwise
    // (an index of -1 selects the first one, as 0 does).
    let mine: Vec<&MaterialDef> = overrides.iter().filter(|o| !o.item && omsi_sim::vehicle::override_slot(materials, o) == Some(slot)).collect();
    let plain = || mine.iter().filter(|o| o.change.is_none());
    plain()
        .rev()
        .find(|o| o.alpha_set)
        .or_else(|| plain().next())
        .or(mine.first())
        .map(|o| alpha_mode(o.alpha))
        .unwrap_or(AlphaMode::Opaque)
}

fn alpha_mode(a: i32) -> AlphaMode {
    match a {
        0 => AlphaMode::Opaque,
        1 => AlphaMode::Test,
        _ => AlphaMode::Blend,
    }
}

/// Whether a vehicle's `[useTextTexture]` slot is a display (`MaterialExtra::display`): one
/// that has a light of its own, a light map or a night map (a destination matrix, a
/// counter lit with the dashboard), not lettering on the body.
fn text_is_display(lightmap: bool, night: bool) -> bool {
    lightmap || night
}

/// How a `[texttexture]` shows on its slot: alpha tested where the slot's `[matl_alpha]` is 1
/// (the stock route helpers, `routearrows_busstop.sco`: blended, the empty part of the text
/// wrote depth and cut away whatever was drawn behind it later - a bus beside the stop lost
/// half its roof), blended otherwise.
fn text_alpha(materials: &[omsi_o3d::Material], slot: usize, overrides: &[MaterialDef]) -> AlphaMode {
    match material_alpha(materials, slot, overrides) {
        AlphaMode::Test => AlphaMode::Test,
        _ => AlphaMode::Blend,
    }
}

/// Placement is part of the picture: otherwise a centred sign can lend its cached
/// texture to a left-aligned one showing the same words.
/// A text texture's material, addressed as its slot is: `[matl_texadress_border]` (and
/// clamp, mirror) on the slot carry over to the text drawn into it. Made with the plain
/// wrap, Road-Hog123's bus stop flags - a second mesh whose UVs run past the text texture
/// into its border - showed the stop name repeated across the whole flag (#1645).
fn text_material(renderer: &Renderer, scene: &mut Scene, tex: TextureId, alpha: AlphaMode, slot_ov: &[&MaterialDef]) -> MaterialId {
    let extra = material_extra(slot_ov, None, None, [0.0; 4]);
    let plain = MaterialExtra::default();
    let border_only = MaterialExtra { border: extra.border, ..plain.clone() };
    renderer.address_next.set(tex_addressing(slot_ov.iter().copied()));
    // (lit like the rest of the object: Omsi.exe only swaps the slot's texture, a sign
    // does not shine at night)
    renderer.add_material_extra(scene, Some(tex), alpha, [1.0; 4], false, None, None, None, None, [0.0; 3], border_only)
}

/// The key of a text texture material: the text's own key and how its slot addresses it.
fn text_material_key(base: String, slot_ov: &[&MaterialDef]) -> String {
    let border = material_extra(slot_ov, None, None, [0.0; 4]).border;
    format!("{base}|{:?}|{border:?}", tex_addressing(slot_ov.iter().copied()))
}

fn scenery_text_key(tt: &omsi_model::TextTexture, text: &str, alpha: AlphaMode) -> String {
    format!(
        "{}|{}|{}x{}|{}|{:?}|{:?}|{}|{}",
        tt.font.to_ascii_lowercase(),
        text,
        tt.width.max(1),
        tt.height.max(1),
        tt.full_color,
        tt.color,
        alpha,
        tt.orientation,
        tt.grid,
    )
}

fn scenery_text_image(
    tt: &omsi_model::TextTexture,
    atlas: Option<Arc<omsi_content::font::FontAtlas>>,
    text: &str,
) -> Image {
    // Static and scripted text textures use the placement from their definition.
    let state = omsi_sim::texttex::TextTextureState::new(tt.clone(), atlas);
    Image {
        width: tt.width.max(1) as u32,
        height: tt.height.max(1) as u32,
        rgba: state.image(text),
        has_alpha: true,
    }
}

/// The text of one of the game's own helper objects (the route arrows' street and stop
/// names) that its `.oft` font cannot draw: the stock arrows ask for the font "test"
/// (`Fonts/test1.oft`), which has the Latin letters and German umlauts only, so a street
/// or a stop named in Cyrillic (or Greek, Chinese ...) came out as an empty arrow - at
/// most a stray `Ä` where a code page variant of `Д` happened to be in the font. Such a
/// text is drawn with the interface font (Roboto, then the system's fonts for the
/// scripts it lacks) in the texture's colour, the height of the `.oft` font's letters,
/// centred, and narrowed to the texture's width. None when the font draws every letter:
/// that text keeps OMSI's own look.
fn helper_text_image(tt: &omsi_model::TextTexture, atlas: Option<&omsi_content::font::FontAtlas>, text: &str) -> Option<Image> {
    let drawable = |c: char| c == '@' || c.is_whitespace() || atlas.is_some_and(|a| a.font.glyph(c).is_some());
    if text.trim().is_empty() || text.chars().all(drawable) {
        return None;
    }
    static FONTS: std::sync::OnceLock<omsi_ui::Fonts> = std::sync::OnceLock::new();
    let fonts = FONTS.get_or_init(omsi_ui::Fonts::new);
    let (w, h) = (tt.width.max(1) as u32, tt.height.max(1) as u32);
    // `@` starts a new line, as in OMSI's own text textures (#1553, #1482): drawn as a
    // letter, a Polish stop's "622@Sosnowiec@Urząd@Miasta" stood in one line with its
    // at signs, and a Chinese one was squeezed into a sliver. (A text starting with `@`
    // keeps its empty first line.)
    let text = text.trim_end();
    let lines: Vec<&str> = text.split('@').map(str::trim).collect();
    let n = lines.len().max(1) as f32;
    // (the .oft's line height holds its capitals and the gap below them; Roboto's capitals
    // are 0.7 of its size, so nearly the line height gives letters of the same height)
    let line = atlas.map(|a| a.font.height.max(8) as f32).unwrap_or(h as f32 * 0.2);
    // the lines share the texture's height when they would not fit at the font's own
    let mut pitch = line.min(h as f32 / n);
    // A line too long for the texture is made smaller first, letters and all, down to two
    // thirds of the height, and only what is still too wide is narrowed: narrowed alone, a
    // long Polish street name came out in thin sticks nobody could read (#1696, #1365).
    let widest = lines.iter().filter(|l| !l.is_empty()).map(|l| fonts.render(l, pitch * 0.95, omsi_ui::Weight::Medium).w).max().unwrap_or(0);
    if widest > w {
        pitch *= (w as f32 / widest as f32).max(2.0 / 3.0);
    }
    let px = pitch * 0.95;
    let top = (h as f32 - pitch * n) * 0.5;
    let rgb = if tt.full_color { [255u8; 3] } else { [tt.color[0] as u8, tt.color[1] as u8, tt.color[2] as u8] };
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for (k, l) in lines.iter().enumerate() {
        if l.is_empty() {
            continue;
        }
        let bmp = fonts.render(l, px, omsi_ui::Weight::Medium);
        if bmp.w == 0 || bmp.h == 0 {
            continue;
        }
        // too long for the texture: narrowed to fit (columns sampled), the height kept
        let scale = (w as f32 / bmp.w as f32).min(1.0);
        let out_w = ((bmp.w as f32 * scale).floor() as u32).max(1);
        let x0 = (w - out_w.min(w)) / 2;
        let y0 = (top + pitch * k as f32 + (pitch - bmp.h as f32) * 0.5).round() as i32;
        for y in 0..bmp.h as i32 {
            let dy = y0 + y;
            if dy < 0 || dy >= h as i32 {
                continue;
            }
            for x in 0..out_w.min(w) {
                let sx = ((x as f32 + 0.5) / scale) as u32;
                let a = bmp.alpha[(y as u32 * bmp.w + sx.min(bmp.w - 1)) as usize];
                if a == 0 {
                    continue;
                }
                let i = ((dy as u32 * w + x0 + x) * 4) as usize;
                rgba[i..i + 3].copy_from_slice(&rgb);
                rgba[i + 3] = rgba[i + 3].max(a);
            }
        }
    }
    Some(Image { width: w, height: h, rgba, has_alpha: true })
}

/// Identify a solid vehicle body material that should participate in the depth buffer.
/// Some bus packs put either `[matl_alpha] 2` or `[matl_noZcheck]` on a complete body mesh.
/// The decision must not depend on one creator's language or on a particular bus name:
/// use the model metadata and the material's actual mesh volume, while keeping thin glass
/// and explicit overlay/transparency materials on their authored paths.
/// Words that name a pane of glass in a mesh or texture file, in the languages OMSI's
/// add-ons are made in. The body-depth repair must not turn one of these opaque when the
/// model.cfg declares it blended: a Czech bus's `okna.o3d` (windows) on the shared
/// `body.png` was drawn as a black wall, where OMSI shows the tinted glass.
const GLASS_WORDS: [&str; 20] = [
    "window", "fenster", "glas", "scheibe", "windshield", "windscreen", // en, de ("glas" is also German: `Leuchtmelderglas.tga`)
    "okn", "sklo", // cs, sk (okna, okno, sklo)
    "szyb", "okien", // pl
    "ablak", // hu
    "steklo", // ru (transliterated)
    "vitre", "fenetre", // fr
    "vetro", "finestr", // it
    "raam", "ruit", // nl
    "ventan", "cristal", // es
];

fn is_vehicle_body_material(
    mesh_file: &str,
    texture: &str,
    has_texture: bool,
    has_transmap: bool,
    no_z_write: bool,
    body_hint: bool,
) -> bool {
    if !has_texture || has_transmap || no_z_write || !body_hint {
        return false;
    }
    let name = format!("{} {}", mesh_file, texture).to_ascii_lowercase();
    let overlay = ["regen", "dirt", "dreck", "wiper", "matrix", "display", "shadow"];
    if GLASS_WORDS.iter().chain(overlay.iter()).any(|part| name.contains(part)) {
        return false;
    }
    true
}

/// The texture whose alpha is a blended slot's coverage: its `[matl_transmap]` file where
/// it has one (the diffuse alpha is then the reflection mask), else the diffuse texture. A
/// script texture as the map (`\S:n`) is not known at load time: the diffuse texture then.
fn coverage_texture<'a>(transmap: Option<&'a str>, diffuse: &'a str) -> &'a str {
    match transmap.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) if !t.starts_with("\\S:") => t,
        _ => diffuse,
    }
}

/// Side of the square a texture's alpha is kept at for [`slot_is_see_through`].
const ALPHA_MASK: usize = 256;

/// A texture's alpha channel, thinned out to [`ALPHA_MASK`] squared (a body texture is
/// 4096 squared, and every blended slot of a bus asks). None for a file that cannot be
/// read or has no alpha.
fn alpha_mask(path: &Path) -> Option<Arc<Vec<u8>>> {
    static MASKS: std::sync::OnceLock<Mutex<HashMap<PathBuf, Option<Arc<Vec<u8>>>>>> = std::sync::OnceLock::new();
    let masks = MASKS.get_or_init(Default::default);
    if let Some(m) = masks.lock().get(path) {
        return m.clone();
    }
    let mask = omsi_texture::decode_file(path).ok().filter(|img| img.has_alpha && img.width > 0 && img.height > 0).map(|img| {
        let (w, h) = (img.width as usize, img.height as usize);
        let mut out = vec![255u8; ALPHA_MASK * ALPHA_MASK];
        for y in 0..ALPHA_MASK {
            for x in 0..ALPHA_MASK {
                let (sx, sy) = ((x * w / ALPHA_MASK).min(w - 1), (y * h / ALPHA_MASK).min(h - 1));
                out[y * ALPHA_MASK + x] = img.rgba[(sy * w + sx) * 4 + 3];
            }
        }
        Arc::new(out)
    });
    masks.lock().insert(path.to_path_buf(), mask.clone());
    mask
}

/// Whether the triangles of material `slot` lie on a see-through part of their texture
/// (`mask`, see [`alpha_mask`]): two in three of them with an alpha under 0.9 at their
/// middle. A pane does - the SOR NB12's glass is 62 of 255 on its body texture; a door or
/// a body panel blended by `[matl_alpha] 2` has its paint at 255 and does not.
fn slot_is_see_through(mesh: &MeshData, slot: usize, mask: &[u8]) -> bool {
    let (mut clear, mut all) = (0usize, 0usize);
    for &(first, count, material) in &mesh.ranges {
        if material as usize != slot {
            continue;
        }
        let start = first as usize;
        let end = start.saturating_add(count as usize).min(mesh.indices.len());
        for tri in mesh.indices.get(start..end).unwrap_or_default().chunks_exact(3) {
            let Some(uv) = tri.iter().map(|&i| mesh.uvs.get(i as usize).copied()).sum::<Option<glam::Vec2>>() else { continue };
            let uv = uv / 3.0;
            let at = |t: f32| ((t.rem_euclid(1.0) * ALPHA_MASK as f32) as usize).min(ALPHA_MASK - 1);
            all += 1;
            if mask[at(uv.y) * ALPHA_MASK + at(uv.x)] < 230 {
                clear += 1;
            }
        }
    }
    // (two thirds: a layer over the windows that painters draw on - the glass's own unwrap
    // in the paint scheme, its adverts and tint - is covered where the picture is, and is
    // a pane all the same)
    all > 0 && clear * 3 >= all * 2
}

/// Whether a texture (`mask`, see [`alpha_mask`]) is clear all over: no texel above 8 of
/// 255. A slot blended by `[matl_alpha] 2` with such a texture is drawn for its depth alone
/// - an "invisible cover" bus makers put in front of a roller blind or a window to hide it
/// (a 16x16 clear `.tga`, #1113, #576): Omsi.exe writes its depth in model order, and what
/// the model lists after it behind it is gone.
fn texture_is_clear(mask: &[u8]) -> bool {
    !mask.is_empty() && mask.iter().all(|&a| a <= 8)
}

/// Return whether the triangles of one material occupy a volumetric part of the vehicle.
/// Windows and rain films are normally very thin sheets; this lets unnamed/modded body
/// meshes be repaired without maintaining a language-specific list of mesh names.
fn material_has_vehicle_volume(mesh: &MeshData, slot: usize) -> bool {
    let mut lo = glam::Vec3::splat(f32::MAX);
    let mut hi = glam::Vec3::splat(f32::MIN);
    let mut points = 0usize;
    for &(first, count, material) in &mesh.ranges {
        if material as usize != slot {
            continue;
        }
        let start = first as usize;
        let end = start.saturating_add(count as usize).min(mesh.indices.len());
        for &index in mesh.indices.get(start..end).unwrap_or_default() {
            if let Some(p) = mesh.positions.get(index as usize) {
                lo = lo.min(*p);
                hi = hi.max(*p);
                points += 1;
            }
        }
    }
    if points < 8 || !lo.x.is_finite() || !hi.x.is_finite() {
        return false;
    }
    let extent = hi - lo;
    let mut sides = [extent.x.abs(), extent.y.abs(), extent.z.abs()];
    sides.sort_by(f32::total_cmp);
    // A solid shell has meaningful thickness compared with both of its other dimensions.
    // A windshield or side pane may be wide and tall, but remains a sheet in its thin axis.
    sides[2] > 3.0
        && sides[1] > 1.0
        && sides[0] > 0.5
        && sides[0] / sides[1] > 0.25
        && sides[0] / sides[2] > 0.02
}

/// Whether the triangles of material `slot` lie on the faces of another slot of the same
/// mesh: a layer modelled as a copy of the surface under it with a material of its own (a
/// baked ambient-occlusion or shading film over the floor, `[matl_alpha] 2`), which OMSI
/// blends over the surface as declared.
fn slot_overlays_another(mesh: &MeshData, slot: usize) -> bool {
    let key = |p: &glam::Vec3| ((p.x * 1000.0).round() as i32, (p.y * 1000.0).round() as i32, (p.z * 1000.0).round() as i32);
    let mut own = std::collections::HashSet::new();
    let mut others = std::collections::HashSet::new();
    for &(first, count, material) in &mesh.ranges {
        let start = first as usize;
        let end = start.saturating_add(count as usize).min(mesh.indices.len());
        let set = if material as usize == slot { &mut own } else { &mut others };
        for &index in mesh.indices.get(start..end).unwrap_or_default() {
            if let Some(p) = mesh.positions.get(index as usize) {
                set.insert(key(p));
            }
        }
    }
    own.len() >= 3 && own.iter().filter(|k| others.contains(*k)).count() * 10 >= own.len() * 9
}

/// GPU-side representation of a vehicle instance: one render instance per mesh.
pub struct VehicleRender {
    pub instances: Vec<usize>,
    /// Materials made for this vehicle alone (its text and script texture slots).
    pub own_materials: Vec<MaterialId>,
    /// The shared set it is drawn with (None for the player's own).
    pub set: Option<VehicleKey>,
    /// `[matl_change]` / `[texchanges]` slots whose material a variable switches.
    pub variants: Vec<VariantSlot>,
    /// GPU texture per `[texttexture]` index.
    pub text_textures: Vec<Option<TextureId>>,
    /// GPU texture per `[scripttexture]` index.
    pub script_textures: Vec<Option<TextureId>>,
    /// `script_textures` belong to the vehicle this part is coupled to (`[scriptshare]`):
    /// they are not this render's to give back.
    pub shared_script: bool,
    /// The script textures (cockpit and passenger displays, 1024×512 pictures for a C2) wait
    /// with their upload this frame: the vehicle is far and it is not its half second.
    pub displays_far: bool,
    /// The half second a far vehicle's displays were last uploaded in.
    pub display_tick: u64,
    /// `[smoothskin]` meshes drawn from a copy of their own (the player's articulated bus's
    /// bellows): (mesh index, the copy, the bone transforms it was last shaped for).
    pub skinned: Vec<(usize, MeshId, Vec<Mat4>)>,
    /// An AI vehicle out of sight: its instances are hidden and not updated (see
    /// `Traffic::sync`).
    pub hidden: bool,
    /// The renderer's slots for the vehicle's `[interiorlight]` lamps (first, count), taken
    /// the first time the vehicle is synced (see `sync_interior_lamps`).
    pub interior_lamps: std::cell::Cell<Option<(u32, u32)>>,
    /// The runs of lamp slots and which of the vehicle's lamps each holds.
    pub interior_blocks: std::cell::OnceCell<Vec<(u32, Vec<usize>)>>,
}

/// A material slot whose texture is generated per vehicle (text or script texture).
#[derive(Debug, Clone)]
pub struct DynSlot {
    pub mesh: usize,
    pub slot: usize,
    pub text: Option<usize>,
    pub script: Option<usize>,
    pub script_trans: Option<usize>,
    pub tex: Option<TextureId>,
    pub alpha: AlphaMode,
    pub transmap: Option<(TextureId, bool)>,
    pub night: Option<TextureId>,
    pub lightmap: Option<TextureId>,
    pub envmap: Option<(TextureId, f32)>,
    /// `[matl_texadress_*]`: how the slot's textures read outside [0, 1].
    pub address: omsi_render::TexAddressing,
    /// Depth handling, reflection mask and specular term of the slot.
    pub extra: MaterialExtra,
    /// Diffuse and emissive colour of the slot's D3D material (see `d3d_material`); a
    /// script texture is drawn unlit and keeps its own colours.
    pub color: [f32; 4],
    pub emissive: [f32; 3],
}

/// Whether a `[matl_change]` variable at `x` shows the slot's `[matl_item]`: Omsi.exe
/// (0x5fd6xx) rounds the variable (to the nearest, ties to even) and shows item `n` for
/// 1 <= n <= the items there are, the plain material otherwise - a lamp's variable at 2
/// with one item is dark. A variable no script declares is registered by the model loader
/// at 0 (the stock MANs' spare buttons, `*Noch nicht belegt*`, and a mod's door lamps
/// were lit for good when it was taken as on, #231).
pub(crate) fn change_picks_item(x: f32) -> bool {
    x.is_finite() && x.round_ties_even() == 1.0
}

/// A material variant switched by a variable.
#[derive(Clone)]
pub struct VariantSlot {
    pub mesh: usize,
    pub slot: usize,
    pub base: MaterialId,
    pub item: MaterialId,
    /// Items 2, 3, ... of the first `[matl_change]`.
    pub more: Vec<MaterialId>,
    /// `[matl_change]` variable: at 1 (rounded) the item variant shows.
    pub var: String,
    /// The variables of the slot's further `[matl_change]`s: the item shows while any of
    /// them is on as well (Omsi.exe sub_7c2d80: each record picks its item by its own
    /// variable, and one at 0 leaves the material to the others).
    pub more_vars: Vec<String>,
    /// `[texchanges]`: the (base, item) pair of every entry of the master, in order.
    pub entries: Vec<(MaterialId, MaterialId)>,
    /// `[texchanges]` variable: its integer value picks the entry.
    pub tex_var: String,
    /// Free textures for the plain material and, independently, its switched item.
    pub free: Vec<FreeTex>,
    /// How to build a material of this slot for a texture loaded later.
    pub spec: SlotSpec,
    /// The textures `base`/`item` and each entry were made with (made again per vehicle
    /// when the slot shows the vehicle's own pictures, see `SlotSpec::per_vehicle`).
    pub base_tex: Option<TextureId>,
    pub entry_tex: Vec<Option<TextureId>>,
    /// Several `[matl_lightmap]`s on the slot (see `MultiLight`).
    pub lights: Option<MultiLight>,
}

/// A material slot with several `[matl_lightmap]`s, each a texture and a variable (the
/// LiAZ 5292's saloon: the cab lamp, saloon circuit 1 and circuit 2). OMSI keeps them
/// all; drawn with only the last one, the
/// saloon stayed unlit whenever circuit 2 was off. The slot's light map is the sum of the
/// maps switched on, made the first time that combination shows and shared by every
/// vehicle of the kind; the slot is as bright as its brightest variable.
#[derive(Clone)]
pub struct MultiLight {
    /// The maps' files and variables, in the order the model lists them.
    pub maps: Vec<(PathBuf, String)>,
    /// The set's own materials (the last map), shown while none is on.
    pub plain: (MaterialId, MaterialId),
    /// Materials made per switched-on combination (bit k: map k).
    pub cache: HashMap<u32, (MaterialId, MaterialId)>,
    pub current: u32,
    /// Composite maps are shared like the vehicles' pictures (see `FreeTex::shared`).
    pub shared: Arc<Mutex<HashMap<PathBuf, (TextureId, usize)>>>,
    pub held: Vec<PathBuf>,
}

impl MultiLight {
    /// The light map made of the maps in `mask`, from the shared store or read now.
    fn composite(&mut self, renderer: &Renderer, scene: &mut Scene, mask: u32) -> Option<TextureId> {
        let mut key = String::from("lightmap-sum:");
        for (k, (path, _)) in self.maps.iter().enumerate() {
            if mask & (1 << k) != 0 {
                key.push_str(&path.to_string_lossy());
                key.push('|');
            }
        }
        let key = PathBuf::from(key);
        let mut shared = self.shared.lock();
        if let Some(e) = shared.get_mut(&key) {
            e.1 += 1;
            self.held.push(key);
            return Some(e.0);
        }
        let mut sum: Option<omsi_texture::Image> = None;
        for (k, (path, _)) in self.maps.iter().enumerate() {
            if mask & (1 << k) == 0 {
                continue;
            }
            let Ok(img) = omsi_texture::decode_file(path) else { continue };
            match &mut sum {
                None => sum = Some(img),
                Some(acc) => {
                    // (maps of another size are sampled at the nearest texel)
                    let (w, h) = (acc.width as usize, acc.height as usize);
                    let (iw, ih) = (img.width as usize, img.height as usize);
                    for y in 0..h {
                        let sy = y * ih / h.max(1);
                        for x in 0..w {
                            let sx = x * iw / w.max(1);
                            let (d, s) = ((y * w + x) * 4, (sy * iw + sx) * 4);
                            for c in 0..3 {
                                // (ADDSMOOTH, as Omsi.exe chains a slot's maps in its
                                // texture stages, 0x7fe5ff: a + b - a b)
                                let (a, b) = (acc.rgba[d + c] as u32, img.rgba[s + c] as u32);
                                acc.rgba[d + c] = (a + b - a * b / 255).min(255) as u8;
                            }
                        }
                    }
                }
            }
        }
        let img = sum?;
        let id = renderer.add_texture(scene, &img, true);
        shared.insert(key.clone(), (id, 1));
        self.held.push(key);
        Some(id)
    }
}

/// `[matl_freetex]`: the slot shows the texture file named by a string variable - the
/// SD200's destination roller reads the terminus pictures of the map's `.hof` this way.
fn free_texture_defs(overrides: &[&MaterialDef]) -> Vec<(bool, String, String)> {
    [false, true]
        .into_iter()
        .filter_map(|item| {
            overrides.iter().filter(|o| o.item == item).find_map(|o| {
                o.freetex
                    .as_ref()
                    .map(|(key, var)| (item, key.clone(), var.clone()))
            })
        })
        .collect()
}

#[derive(Clone)]
pub struct FreeTex {
    pub var: String,
    /// The original named texture can be used by several stages (a display commonly
    /// names the same black texture as its diffuse and its switched night map).
    pub key: Option<TextureId>,
    pub diffuse: bool,
    /// A declaration inside `[matl_item]` must not change the unpowered material.
    pub item_only: bool,
    /// Where the file name is looked up (the vehicle's texture folders).
    pub dirs: Vec<PathBuf>,
    pub textures: Arc<omsi_texture::TextureCache>,
    /// File name (lower case) → the materials already built for it.
    pub cache: HashMap<String, (MaterialId, MaterialId)>,
    /// The name currently applied.
    pub current: Option<String>,
    /// The world's shared vehicle textures (a picture is one texture for every vehicle
    /// showing it; every bus had its own copy, half a gigabyte of destination pictures on
    /// Ahlheim), the pictures this vehicle holds, and where pictures uploaded as RGBA are
    /// sent to be compressed.
    pub shared: Arc<Mutex<HashMap<PathBuf, (TextureId, usize)>>>,
    pub held: Vec<PathBuf>,
    pub wants_upgrade: Arc<Mutex<Vec<PathBuf>>>,
}

impl VariantSlot {
    /// The material the slot shows now: `[texchanges]` picks the texture, `[matl_change]`
    /// then picks between the plain material and the `[matl_item]` variant.
    pub fn material(&self, var: impl Fn(&str) -> Option<f32>) -> MaterialId {
        let (base, item) = if self.entries.is_empty() {
            (self.base, self.item)
        } else {
            let v = var(&self.tex_var).unwrap_or(0.0);
            let i = if v.is_finite() { v.trunc() as i64 } else { 0 };
            self.entries[i.clamp(0, self.entries.len() as i64 - 1) as usize]
        };
        let x = self.var.trim().parse().ok().or_else(|| var(&self.var)).unwrap_or(0.0);
        if self.entries.is_empty() && x.is_finite() {
            let n = x.round_ties_even();
            if n >= 2.0 && ((n - 2.0) as usize) < self.more.len() {
                return self.more[(n - 2.0) as usize];
            }
        }
        if change_picks_item(x) || self.more_vars.iter().any(|v| var(v).is_some_and(change_picks_item)) {
            item
        } else {
            base
        }
    }
}

/// How one material slot is built, so that the same description can be applied to every
/// texture a `[texchanges]` master or a `[matl_freetex]` string switches between.
#[derive(Clone)]
pub struct SlotSpec {
    base: Look,
    /// The `[matl_item]` half.
    item: Option<Look>,
    /// The first `[matl_change]`'s items after its first (shown at 2, 3, ...).
    more: Vec<Look>,
}

/// How one half of a material slot (the plain material or its `[matl_item]`) is drawn.
#[derive(Clone)]
pub struct Look {
    alpha: AlphaMode,
    color: [f32; 4],
    emissive: [f32; 3],
    unlit: bool,
    /// A picture of the vehicle's own in place of the diffuse texture (see `DynTex`).
    diffuse: Option<TextureId>,
    transmap: Option<(TextureId, bool)>,
    night: Option<TextureId>,
    lightmap: Option<TextureId>,
    envmap: Option<(TextureId, f32)>,
    extra: MaterialExtra,
    dyn_tex: DynTex,
}

/// The pictures of its own vehicle a half of a slot shows: `[useTextTexture]`,
/// `[useScriptTexture]` and a script texture as the transparency map
/// (`[matl_transmap] \S:n`), and whether it is addressed without repeating.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DynTex {
    text: Option<usize>,
    script: Option<usize>,
    script_trans: Option<usize>,
    address: omsi_render::TexAddressing,
}

impl DynTex {
    fn any(&self) -> bool {
        self.text.is_some() || self.script.is_some() || self.script_trans.is_some()
    }
}

impl Look {
    fn add(&self, renderer: &Renderer, scene: &mut Scene, tex: Option<TextureId>) -> MaterialId {
        renderer.address_next.set(self.dyn_tex.address);
        renderer.add_material_extra(
            scene,
            self.diffuse.or(tex),
            self.alpha,
            self.color,
            self.unlit,
            self.transmap,
            self.night,
            self.lightmap,
            self.envmap,
            self.emissive,
            self.extra,
        )
    }

    /// This half with one vehicle's text and script textures, set up as
    /// `instantiate_vehicle` sets up a `DynSlot`.
    fn for_vehicle(&self, text: &[Option<TextureId>], script: &[Option<TextureId>]) -> Look {
        let d = self.dyn_tex;
        let mut l = self.clone();
        l.dyn_tex = DynTex {
            address: d.address,
            ..DynTex::default()
        };
        if let Some(t) = d.text.and_then(|i| text.get(i).copied().flatten()) {
            // lit as the slot's own material is, as `instantiate_vehicle` makes a text slot
            // that is not switched: drawn unlit, a switched slot's fleet number or plate
            // shone at full brightness at night (#698)
            let mut extra = l.extra;
            extra.display = text_is_display(l.lightmap.is_some(), l.night.is_some());
            extra.screen = true;
            return Look {
                diffuse: Some(t),
                alpha: AlphaMode::Blend,
                color: [1.0; 4],
                emissive: [0.0; 3],
                unlit: false,
                transmap: None,
                envmap: None,
                extra,
                ..l
            };
        }
        if let Some(t) = d.script.and_then(|i| script.get(i).copied().flatten()) {
            l.diffuse = Some(t);
        }
        if let Some(t) = d
            .script_trans
            .and_then(|i| script.get(i).copied().flatten())
        {
            l.transmap = Some((t, true));
        }
        // A transmap is not a reason by itself to force a material into blend mode; it only
        // carries the alpha for the chosen material mode. Keep any explicit alpha setting,
        // otherwise body slots stay solid.
        if d.script.is_some() {
            l.color = [1.0; 4];
            l.emissive = [0.0; 3];
            l.unlit = true;
        }
        l
    }
}

impl SlotSpec {
    fn with_freetex(
        &self,
        key: Option<TextureId>,
        tex: TextureId,
        diffuse: bool,
        item_only: bool,
    ) -> Self {
        let mut spec = self.clone();
        let replace = |look: &mut Look| {
            // A per-vehicle text/script texture has already replaced the original
            // diffuse and is not the file named by this free-texture declaration.
            if (diffuse && look.diffuse.is_none()) || (key.is_some() && look.diffuse == key) {
                look.diffuse = Some(tex);
            }
            if let Some(key) = key {
                for stage in [&mut look.night, &mut look.lightmap] {
                    if *stage == Some(key) {
                        *stage = Some(tex);
                    }
                }
                if let Some((id, _)) = &mut look.transmap {
                    if *id == key {
                        *id = tex;
                    }
                }
                if let Some((id, _)) = &mut look.envmap {
                    if *id == key {
                        *id = tex;
                    }
                }
            }
        };
        if !item_only {
            replace(&mut spec.base);
        }
        if let Some(item) = &mut spec.item {
            replace(item);
        }
        spec
    }

    /// (plain material, `[matl_item]` material) for one diffuse texture; without a
    /// `[matl_item]` both are the same material.
    pub fn build(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        tex: Option<TextureId>,
    ) -> (MaterialId, MaterialId) {
        let base = self.base.add(renderer, scene, tex);
        let item = match &self.item {
            Some(it) => it.add(renderer, scene, tex),
            None => base,
        };
        (base, item)
    }

    /// The slot shows pictures of its own vehicle: every vehicle makes its materials with
    /// `for_vehicle`.
    /// The same slot with another light map.
    pub fn set_lightmap(&mut self, tex: Option<TextureId>) {
        self.base.lightmap = tex;
        if let Some(it) = &mut self.item {
            it.lightmap = tex;
        }
        for it in &mut self.more {
            it.lightmap = tex;
        }
    }

    /// The further items' materials (each made last, so that `recycle` can move it).
    pub fn build_more(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        tex: Option<TextureId>,
        mut recycle: impl FnMut(&mut Scene, MaterialId) -> MaterialId,
    ) -> Vec<MaterialId> {
        self.more
            .iter()
            .map(|l| {
                let m = l.add(renderer, scene, tex);
                recycle(scene, m)
            })
            .collect()
    }

    pub fn per_vehicle(&self) -> bool {
        self.base.dyn_tex.any() || self.item.as_ref().is_some_and(|i| i.dyn_tex.any()) || self.more.iter().any(|i| i.dyn_tex.any())
    }

    pub fn for_vehicle(
        &self,
        text: &[Option<TextureId>],
        script: &[Option<TextureId>],
    ) -> SlotSpec {
        SlotSpec {
            base: self.base.for_vehicle(text, script),
            item: self.item.as_ref().map(|i| i.for_vehicle(text, script)),
            more: self.more.iter().map(|i| i.for_vehicle(text, script)).collect(),
        }
    }
}

/// A vehicle type with a paint scheme, as its GPU set is known by.
pub type VehicleKey = (PathBuf, Option<usize>);

/// The GPU side of a vehicle type in one paint scheme, shared by its AI copies: meshes and
/// materials, the slots every copy fills itself, and what the set holds of the shared
/// textures and meshes (given back when the set is trimmed).
#[derive(Clone)]
pub struct VehicleSet {
    meshes: Vec<(MeshId, Vec<MaterialId>)>,
    dyn_slots: Vec<DynSlot>,
    variants: Vec<VariantSlot>,
    textures: Vec<PathBuf>,
    mesh_keys: Vec<(PathBuf, usize)>,
    materials: Vec<MaterialId>,
    /// Vehicles drawn with it now, and since when nobody is.
    users: usize,
    idle_since: Option<std::time::Instant>,
}

/// What reads vehicle sets ahead on a worker thread: their textures and meshes, made on the
/// GPU right there (the device takes calls from any thread) and waiting in `ready` until the
/// set is uploaded - which then only puts them into the scene and makes the materials. A
/// C2's set took 60 ms on the thread that draws when it had to read and upload it all.
#[derive(Clone)]
pub struct VehiclePrefetch {
    root: PathBuf,
    textures: Arc<TextureCache>,
    on_gpu: Arc<Mutex<HashMap<PathBuf, (TextureId, usize)>>>,
    meshes_on_gpu: Arc<Mutex<HashMap<(PathBuf, usize), (MeshId, usize)>>>,
    ready: Arc<Mutex<PreparedVehicles>>,
    gpu: (wgpu::Device, wgpu::Queue),
    mesh_pages: bool,
}

/// Vehicle meshes and textures made on a worker, by (bus file, mesh) and by file.
#[derive(Default)]
struct PreparedVehicles {
    meshes: HashMap<(PathBuf, usize), omsi_render::PreparedMesh>,
    textures: HashMap<PathBuf, (omsi_render::PreparedTexture, omsi_texture::PixelFormat)>,
}

impl VehiclePrefetch {
    /// Read what uploading `vt` in `scheme` will ask for and the GPU does not have.
    pub fn prefetch(&self, vt: &omsi_sim::VehicleType, scheme: Option<usize>) {
        // OpenGL has one adapter context; GPU uploads from this worker can time out while
        // the render thread holds it, so let the normal vehicle upload handle them.
        if omsi_render::gl_backend() {
            return;
        }
        for (name, dirs) in vehicle_texture_names(&self.root, vt, scheme) {
            let refs: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
            let Some(path) = omsi_texture::find_texture(&name, &refs) else {
                continue;
            };
            if self.on_gpu.lock().contains_key(&path)
                || self.ready.lock().textures.contains_key(&path)
            {
                continue;
            }
            let Some(data) = self.textures.get_gpu_path(&path) else {
                continue;
            };
            // on the GPU now, unless the GPU has to make its chain (the upload does that)
            if let Some(t) = omsi_render::prepare_texture(&self.gpu.0, &self.gpu.1, &data) {
                self.textures.release(&path);
                self.ready
                    .lock()
                    .textures
                    .entry(path)
                    .or_insert((t, data.format));
            }
        }
        // [matl_bumpmap] height maps, kept under their own key
        for (name, dirs) in vehicle_bump_names(&self.root, vt, scheme) {
            let refs: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
            let Some(path) = omsi_texture::find_texture(&name, &refs) else {
                continue;
            };
            let key = bump_key(&path);
            if self.on_gpu.lock().contains_key(&key)
                || self.ready.lock().textures.contains_key(&key)
            {
                continue;
            }
            let Some(data) = load_texture_key(&key, true) else {
                continue;
            };
            if let Some(t) = omsi_render::prepare_texture(&self.gpu.0, &self.gpu.1, &data) {
                self.ready
                    .lock()
                    .textures
                    .entry(key)
                    .or_insert((t, data.format));
            }
        }
        let mut todo = Vec::new();
        for i in 0..vt.meshes.len() {
            let key = (vt.def.path.clone(), i);
            if self.meshes_on_gpu.lock().contains_key(&key)
                || self.ready.lock().meshes.contains_key(&key)
            {
                continue;
            }
            if let Some(d) = vt.mesh_data(i) {
                todo.push((key, d));
            }
        }
        let data: Vec<&omsi_geometry::MeshData> = todo.iter().map(|(_, d)| d.as_ref()).collect();
        let meshes = omsi_render::prepare_meshes(&self.gpu.0, &self.gpu.1, &data, self.mesh_pages);
        let mut ready = self.ready.lock();
        for ((key, _), m) in todo.into_iter().zip(meshes) {
            ready.meshes.entry(key).or_insert(m);
        }
    }
}

/// The texture names (with their folders) uploading a vehicle in a paint scheme looks up,
/// as `World::upload_vehicle` does.
fn vehicle_texture_names(
    root: &Path,
    vt: &omsi_sim::VehicleType,
    scheme: Option<usize>,
) -> Vec<(String, Vec<PathBuf>)> {
    let mut dirs = vt.texture_dirs(root);
    let (subst, scheme_dir) = match scheme {
        Some(i) => vt.scheme_substitutions(i),
        None => (vt.default_substitutions(root), None),
    };
    if let Some(d) = scheme_dir {
        dirs.insert(0, d);
    }
    let subst = |name: &str| -> String {
        subst
            .get(&name.to_ascii_lowercase())
            .cloned()
            .unwrap_or_else(|| name.to_string())
    };
    let mut out: Vec<(String, Vec<PathBuf>)> = Vec::new();
    let mut push = |name: String, d: &Vec<PathBuf>| {
        let name = name.trim().to_string();
        if !name.is_empty() && !name.starts_with("\\S:") && mirror_index(&name).is_none() {
            out.push((name, d.clone()));
        }
    };
    for vm in &vt.meshes {
        for m in &vm.materials {
            match vt.texchange(&m.texture) {
                Some(master) => {
                    let mut edirs = vec![master.dir.clone()];
                    edirs.extend(dirs.iter().cloned());
                    for e in &master.entries {
                        push(subst(e), &edirs);
                    }
                }
                None => push(subst(&m.texture), &dirs),
            }
        }
        for o in &vm.overrides {
            for name in [
                o.nightmap.clone().map(|t| subst(&t)),
                o.transmap.clone().map(|t| subst(&t)),
                o.lightmap.clone().map(|l| subst(&l.0)),
                o.envmap.clone().map(|e| e.0),
                o.envmap_mask
                    .clone()
                    .filter(|_| o.envmap.is_some())
                    .map(|t| subst(&t)),
            ]
            .into_iter()
            .flatten()
            {
                push(name, &dirs);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The `[matl_bumpmap]` files (with their folders) uploading a vehicle in a paint scheme
/// asks for (only where the slot reflects, as `World::upload_vehicle` does).
fn vehicle_bump_names(
    root: &Path,
    vt: &omsi_sim::VehicleType,
    scheme: Option<usize>,
) -> Vec<(String, Vec<PathBuf>)> {
    if omsi_cfg::env::var_os("OMSI_NO_BUMP").is_some() || omsi_cfg::env::var_os("OMSI_NO_ENVMAP").is_some() {
        return Vec::new();
    }
    let mut dirs = vt.texture_dirs(root);
    let (subst, scheme_dir) = match scheme {
        Some(i) => vt.scheme_substitutions(i),
        None => (vt.default_substitutions(root), None),
    };
    if let Some(d) = scheme_dir {
        dirs.insert(0, d);
    }
    let mut out: Vec<(String, Vec<PathBuf>)> = Vec::new();
    for vm in &vt.meshes {
        for o in vm.overrides.iter().filter(|o| o.envmap.is_some()) {
            if let Some((t, _)) = &o.bumpmap {
                let name = subst
                    .get(&t.to_ascii_lowercase())
                    .cloned()
                    .unwrap_or_else(|| t.clone());
                if !name.trim().is_empty() {
                    out.push((name.trim().to_string(), dirs.clone()));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

impl World {
    /// Upload a vehicle type's meshes and create render instances for one vehicle (the
    /// player's: its set is not shared and stays).
    pub fn add_vehicle(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        vt: &omsi_sim::VehicleType,
        scheme: Option<usize>,
    ) -> VehicleRender {
        // (the mirrors' glass is the player's bus's: what the last one left is forgotten)
        self.mirror_aspect.lock().clear();
        self.mirror_glass.lock().clear();
        let set = self.upload_vehicle(renderer, scene, vt, scheme, true);
        let mut render = self.instantiate_vehicle(renderer, scene, vt, &set, None, None);
        own_skinned_meshes(renderer, scene, vt, &mut render);
        render
    }

    /// A part coupled behind the player's vehicle (the rear section of an articulated bus).
    /// With `[scriptshare]` it has no scripts of its own: its matrix displays (`\S:n`,
    /// `[useScriptTexture] n`) are the leading vehicle's script textures, which is why the
    /// O530G's rear section declares no `[scripttexture]` at all.
    pub fn add_vehicle_part(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        vt: &omsi_sim::VehicleType,
        scheme: Option<usize>,
        lead: &VehicleRender,
    ) -> VehicleRender {
        let set = self.upload_vehicle(renderer, scene, vt, scheme, false);
        let shared = if vt.def.script_share || vt.model.script_textures.is_empty() {
            Some(lead.script_textures.as_slice())
        } else {
            None
        };
        let mut render = self.instantiate_vehicle(renderer, scene, vt, &set, None, shared);
        own_skinned_meshes(renderer, scene, vt, &mut render);
        render
    }

    /// The worker-side reader of vehicle sets (see [`VehiclePrefetch`]).
    pub fn vehicle_prefetch(&self, renderer: &Renderer) -> VehiclePrefetch {
        VehiclePrefetch {
            root: self.root.clone(),
            textures: self.textures.clone(),
            on_gpu: self.vehicle_textures.clone(),
            meshes_on_gpu: self.vehicle_meshes.clone(),
            ready: self.vehicle_ready.clone(),
            gpu: (renderer.device.clone(), renderer.queue.clone()),
            mesh_pages: renderer.mesh_pages(),
        }
    }

    /// Read these vehicle sets on the worker pool and wait for them (a timetable's first
    /// buses at load time). Returns the bytes held until they are uploaded.
    pub fn prefetch_vehicle_sets(
        &self,
        renderer: &Renderer,
        sets: &[(Arc<omsi_sim::VehicleType>, Option<usize>)],
    ) -> usize {
        use rayon::prelude::*;
        let p = self.vehicle_prefetch(renderer);
        sets.par_iter()
            .for_each(|(vt, scheme)| p.prefetch(vt, *scheme));
        self.textures.held_bytes()
            + self
                .vehicle_ready
                .lock()
                .textures
                .values()
                .map(|t| t.0.bytes() as usize)
                .sum::<usize>()
    }

    /// Textures the thread that draws has to read itself are read the quick way and
    /// compressed afterwards on the workers (a window: no frame waits for a compression).
    pub fn set_fast_texture_loads(&self, on: bool) {
        self.gpu.lock().fast_loads = on;
    }

    /// Drop what was read ahead for vehicle sets and not uploaded.
    pub fn forget_prefetched(&self) {
        self.textures.release_all();
        let mut r = self.vehicle_ready.lock();
        r.meshes.clear();
        r.textures.clear();
    }

    /// Whether the set of `vt` in `scheme` is on the GPU.
    pub fn has_vehicle_set(&self, key: &VehicleKey) -> bool {
        self.vehicle_gpu.lock().contains_key(key)
    }

    /// Upload a vehicle type's meshes, textures and materials ahead of time, so that the
    /// first bus of that type and paint scheme does not cost a frame when it spawns.
    pub fn precache_vehicle(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        vt: &omsi_sim::VehicleType,
        scheme: Option<usize>,
    ) {
        let key = (vt.def.path.clone(), scheme);
        if self.vehicle_gpu.lock().contains_key(&key) {
            // uploaded meanwhile (a bus came first): what was read for it can go
            let dirs_of = vehicle_texture_names(&self.root, vt, scheme);
            for (name, dirs) in dirs_of {
                let refs: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
                if let Some(p) = omsi_texture::find_texture(&name, &refs) {
                    if self.vehicle_textures.lock().contains_key(&p) {
                        self.textures.release(&p);
                        self.vehicle_ready.lock().textures.remove(&p);
                    }
                }
            }
            let on_gpu: Vec<(PathBuf, usize)> = (0..vt.meshes.len())
                .map(|i| (vt.def.path.clone(), i))
                .filter(|k| self.vehicle_meshes.lock().contains_key(k))
                .collect();
            let mut r = self.vehicle_ready.lock();
            for k in on_gpu {
                r.meshes.remove(&k);
            }
            return;
        }
        let c = self.upload_vehicle(renderer, scene, vt, scheme, false);
        self.vehicle_gpu.lock().insert(key, c);
    }

    /// Like `add_vehicle`, but meshes and static materials uploaded for the same vehicle
    /// type and scheme are shared between instances (AI traffic). Give the render back with
    /// [`World::release_vehicle`].
    /// `lead` is the vehicle this one is coupled behind, if any: a rear section takes the
    /// leading vehicle's script textures (`[scriptshare]`, its `[matl_transmap] \S:n`
    /// displays), which its own model declares none of. Built without them its matrix slot
    /// had no mask and drew the lit panel's own picture instead of the dots the leading
    /// vehicle's scripts put there.
    pub fn add_vehicle_shared(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        vt: &omsi_sim::VehicleType,
        scheme: Option<usize>,
        lead: Option<&VehicleRender>,
    ) -> VehicleRender {
        let shared = lead.and_then(|l| {
            (vt.def.script_share || vt.model.script_textures.is_empty()).then(|| l.script_textures.as_slice())
        });
        let key = (vt.def.path.clone(), scheme);
        let cached = self.vehicle_gpu.lock().get(&key).cloned();
        let set = match cached {
            Some(c) => c,
            None => {
                let c = self.upload_vehicle(renderer, scene, vt, scheme, false);
                self.vehicle_gpu.lock().insert(key.clone(), c.clone());
                c
            }
        };
        if let Some(s) = self.vehicle_gpu.lock().get_mut(&key) {
            s.users += 1;
            s.idle_since = None;
        }
        let mut render = self.instantiate_vehicle(renderer, scene, vt, &set, Some(key), shared);
        // an articulated AI bus (timetable or random traffic, and its coupled rear section)
        // bends its own bellows too, from a mesh copy of its own (freed again in
        // `release_vehicle`) - the shared set's copy has to stay in the rest pose, since
        // every other instance of the type still draws it
        own_skinned_meshes(renderer, scene, vt, &mut render);
        render
    }

    /// An AI vehicle has gone: its own instances, textures and materials go back to the free
    /// lists, and its set loses a user.
    pub fn release_vehicle(&self, renderer: &Renderer, scene: &mut Scene, render: VehicleRender) {
        {
            let mut gpu = self.gpu.lock();
            // the mesh copies its `[smoothskin]` meshes were reshaped in belong to this
            // vehicle alone (see `own_skinned_meshes`) and do not go back to any free list
            for (_, mesh, _) in &render.skinned {
                renderer.free_mesh(scene, *mesh);
            }
            for i in render.instances {
                renderer.remove_instance(scene, i);
                let slots = renderer.instance_slots(scene, i);
                gpu.free_instances.entry(slots).or_default().push(i);
            }
            let own_script: &[Option<TextureId>] = if render.shared_script {
                &[]
            } else {
                &render.script_textures
            };
            let mut own_textures: Vec<TextureId> = render
                .text_textures
                .iter()
                .chain(own_script)
                .flatten()
                .copied()
                .collect();
            let mut own_materials = render.own_materials;
            for v in &render.variants {
                if let Some(l) = &v.lights {
                    for (b, it) in l.cache.values() {
                        own_materials.push(*b);
                        own_materials.push(*it);
                    }
                    let mut shared = l.shared.lock();
                    for p in &l.held {
                        let Some(e) = shared.get_mut(p) else { continue };
                        e.1 = e.1.saturating_sub(1);
                        if e.1 == 0 {
                            own_textures.push(e.0);
                            shared.remove(p);
                        }
                    }
                }
            }
            for v in render.variants {
                for f in v.free {
                    for (b, it) in f.cache.into_values() {
                        own_materials.push(b);
                        own_materials.push(it);
                    }
                    // the pictures it showed: shared, gone with their last holder
                    let mut shared = f.shared.lock();
                    for p in f.held {
                        let Some(e) = shared.get_mut(&p) else {
                            continue;
                        };
                        e.1 = e.1.saturating_sub(1);
                        if e.1 == 0 {
                            own_textures.push(e.0);
                            shared.remove(&p);
                        }
                    }
                }
            }
            own_materials.sort_unstable();
            own_materials.dedup();
            for m in own_materials {
                gpu.free_material(renderer, scene, m);
            }
            own_textures.sort_unstable();
            own_textures.dedup();
            for t in own_textures {
                renderer.free_texture(scene, t);
                gpu.free_textures.push(t);
            }
        }
        if let Some((first, n)) = render.interior_lamps.get() {
            renderer.free_interior_lights(scene, first, n);
        }
        if let Some(key) = render.set {
            if let Some(s) = self.vehicle_gpu.lock().get_mut(&key) {
                s.users = s.users.saturating_sub(1);
                if s.users == 0 {
                    s.idle_since = Some(std::time::Instant::now());
                }
            }
        }
    }

    /// Let go of the vehicle sets nobody has drawn for `idle` and that are not in `keep`
    /// (the timetable's next buses): their materials, and the textures and meshes no other
    /// set holds. Returns how many sets went.
    pub fn trim_vehicle_sets(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        keep: &hashbrown::HashSet<VehicleKey>,
        idle: std::time::Duration,
    ) -> usize {
        let now = std::time::Instant::now();
        let mut sets = self.vehicle_gpu.lock();
        let gone: Vec<VehicleKey> = sets
            .iter()
            .filter(|(k, s)| {
                s.users == 0
                    && !keep.contains(*k)
                    && s.idle_since
                        .map(|t| now.duration_since(t) >= idle)
                        .unwrap_or(false)
            })
            .map(|(k, _)| k.clone())
            .collect();
        if gone.is_empty() {
            return 0;
        }
        let mut tex_ids = self.vehicle_textures.lock();
        let mut mesh_ids = self.vehicle_meshes.lock();
        let mut gpu = self.gpu.lock();
        let (mut textures, mut meshes) = (0usize, 0usize);
        for k in &gone {
            let s = sets.remove(k).unwrap();
            for m in s.materials {
                gpu.free_material(renderer, scene, m);
            }
            for p in s.textures {
                let Some(e) = tex_ids.get_mut(&p) else {
                    continue;
                };
                e.1 = e.1.saturating_sub(1);
                if e.1 == 0 {
                    let id = e.0;
                    tex_ids.remove(&p);
                    renderer.free_texture(scene, id);
                    gpu.free_textures.push(id);
                    textures += 1;
                }
            }
            for mk in s.mesh_keys {
                let Some(e) = mesh_ids.get_mut(&mk) else {
                    continue;
                };
                e.1 = e.1.saturating_sub(1);
                if e.1 == 0 {
                    let id = e.0;
                    mesh_ids.remove(&mk);
                    renderer.free_mesh(scene, id);
                    gpu.free_meshes.push(id);
                    meshes += 1;
                }
            }
        }
        if omsi_cfg::env::var_os("OMSI_PROFILE").is_some() {
            log::info!(
                "vehicle sets: {} let go ({textures} textures, {meshes} meshes), {} kept",
                gone.len(),
                sets.len()
            );
        }
        gone.len()
    }

    /// A shared vehicle texture: found by OMSI's rules, uploaded on first use; `held` (the
    /// set being built) becomes one of its holders.
    fn vehicle_texture(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        tex_ids: &mut HashMap<PathBuf, (TextureId, usize)>,
        held: &mut Vec<PathBuf>,
        name: &str,
        dirs: &[&Path],
    ) -> Option<TextureId> {
        let path = omsi_texture::find_texture(name, dirs)?;
        if let Some(e) = tex_ids.get_mut(&path) {
            // (a copy read ahead for another set is not needed)
            self.textures.release(&path);
            self.vehicle_ready.lock().textures.remove(&path);
            if !held.contains(&path) {
                e.1 += 1;
                held.push(path);
            }
            return Some(e.0);
        }
        // made on a worker ahead of the set
        let prepared = self.vehicle_ready.lock().textures.remove(&path);
        if let Some((t, format)) = prepared {
            let id = {
                let mut gpu = self.gpu.lock();
                let id = renderer.add_prepared_texture(scene, t);
                gpu.take_texture_slot(renderer, scene, id)
            };
            if omsi_cfg::env::var_os("OMSI_DEBUG_TEXTURES").is_some() {
                log::info!(
                    "vehicle texture {} (made ahead) {:?}, {:.2} MB",
                    path.display(),
                    format,
                    scene.texture_bytes_of(id) as f64 / 1e6
                );
            }
            attach_pbr(renderer, scene, &path, id);
            tex_ids.insert(path.clone(), (id, 1));
            held.push(path);
            return Some(id);
        }
        // read ahead (compressed) on a worker, or now as quickly as it goes and compressed
        // afterwards (offscreen: compressed at once)
        let fast = self.gpu.lock().fast_loads;
        let (img, worth) = if fast {
            self.textures.get_gpu_fast(&path)?
        } else {
            (self.textures.get_gpu_path(&path)?, false)
        };
        // (one to be swapped for its compressed whole goes up at half its size meanwhile, as
        // the scenery's do: an articulated bus of big PNG and TGA textures put up whole as
        // RGBA took gigabytes for a moment, and a card with 3 GB lost its device while the
        // bus was loading, every start again, #921)
        let img = if worth {
            Arc::new(omsi_texture::gpu::halved_for_now(Arc::try_unwrap(img).unwrap_or_else(|a| (*a).clone())))
        } else {
            img
        };
        let id = {
            let mut gpu = self.gpu.lock();
            let id = gpu.add_data(renderer, scene, &img);
            if worth {
                gpu.wants_upgrade.push(path.clone());
            }
            id
        };
        if omsi_cfg::env::var_os("OMSI_DEBUG_TEXTURES").is_some() {
            log::info!(
                "vehicle texture {} {}x{} {:?} {} levels, {:.2} MB",
                path.display(),
                img.width,
                img.height,
                img.format,
                img.levels.len(),
                scene.texture_bytes_of(id) as f64 / 1e6
            );
        }
        // on the GPU now: the decoded copy can go
        self.textures.release(&path);
        attach_pbr(renderer, scene, &path, id);
        tex_ids.insert(path.clone(), (id, 1));
        held.push(path);
        Some(id)
    }

    /// The snow the panes wear in a snow weather (`rain::snow_on_glass`), made once and
    /// shared by every vehicle like any other vehicle texture. `name` and `dirs` are not
    /// used - it has the signature the `tex!` macro calls with.
    fn snow_glass_texture(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        tex_ids: &mut HashMap<PathBuf, (TextureId, usize)>,
        held: &mut Vec<PathBuf>,
        _name: &str,
        _dirs: &[&Path],
    ) -> Option<TextureId> {
        let key = PathBuf::from("<snow on glass>");
        if let Some(e) = tex_ids.get_mut(&key) {
            if !held.contains(&key) {
                e.1 += 1;
                held.push(key);
            }
            return Some(e.0);
        }
        let id = crate::rain::add_snow_on_glass(renderer, scene, &self.root);
        tex_ids.insert(key.clone(), (id, 1));
        held.push(key);
        Some(id)
    }

    /// A shared `[matl_bumpmap]` height map of a vehicle (`bump_key`): what a worker made
    /// ahead, else made now (uncompressed in a window, where no frame waits for a
    /// compression); `held` becomes one of its holders.
    fn vehicle_bump_texture(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        tex_ids: &mut HashMap<PathBuf, (TextureId, usize)>,
        held: &mut Vec<PathBuf>,
        name: &str,
        dirs: &[&Path],
    ) -> Option<TextureId> {
        let key = bump_key(&omsi_texture::find_texture(name, dirs)?);
        if let Some(e) = tex_ids.get_mut(&key) {
            self.vehicle_ready.lock().textures.remove(&key);
            if !held.contains(&key) {
                e.1 += 1;
                held.push(key);
            }
            return Some(e.0);
        }
        let prepared = self.vehicle_ready.lock().textures.remove(&key);
        let id = match prepared {
            Some((t, _)) => {
                let mut gpu = self.gpu.lock();
                let id = renderer.add_prepared_texture(scene, t);
                gpu.take_texture_slot(renderer, scene, id)
            }
            None => {
                let fast = self.gpu.lock().fast_loads;
                let data = load_texture_key(&key, !fast)?;
                self.gpu.lock().add_data(renderer, scene, &data)
            }
        };
        if omsi_cfg::env::var_os("OMSI_DEBUG_TEXTURES").is_some() {
            log::info!(
                "vehicle bump map {}, {:.2} MB",
                key.display(),
                scene.texture_bytes_of(id) as f64 / 1e6
            );
        }
        tex_ids.insert(key.clone(), (id, 1));
        held.push(key);
        Some(id)
    }

    /// A shared vehicle mesh (by bus file and mesh index), uploaded on first use from what
    /// the prefetch read, else from the type (read again for an AI type).
    fn vehicle_mesh(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        mesh_ids: &mut HashMap<(PathBuf, usize), (MeshId, usize)>,
        keys: &mut Vec<(PathBuf, usize)>,
        vt: &omsi_sim::VehicleType,
        i: usize,
    ) -> MeshId {
        let key = (vt.def.path.clone(), i);
        if let Some(e) = mesh_ids.get_mut(&key) {
            self.vehicle_ready.lock().meshes.remove(&key);
            if !keys.contains(&key) {
                e.1 += 1;
                keys.push(key);
            }
            return e.0;
        }
        let pre = self.vehicle_ready.lock().meshes.remove(&key);
        let id = match pre {
            Some(m) => {
                let mut gpu = self.gpu.lock();
                let id = renderer.add_prepared_mesh(scene, m);
                gpu.take_mesh_slot(renderer, scene, id)
            }
            None => {
                let data = vt.mesh_data(i);
                let empty = MeshData {
                    ranges: vt.meshes[i].data.ranges.clone(),
                    ..Default::default()
                };
                let mut gpu = self.gpu.lock();
                gpu.add_mesh(renderer, scene, data.as_deref().unwrap_or(&empty))
            }
        };
        mesh_ids.insert(key.clone(), (id, 1));
        scene.meshes[id].source = Some(vt.def.path.display().to_string());
        keys.push(key);
        id
    }

    /// A (plain, `[matl_item]`) material pair just built, moved into freed material slots.
    fn recycle_pair(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        pair: (MaterialId, MaterialId),
    ) -> (MaterialId, MaterialId) {
        let mut gpu = self.gpu.lock();
        if pair.0 == pair.1 {
            let m = gpu.material(renderer, scene, pair.0);
            (m, m)
        } else {
            // the item was built last: it moves first, then the plain one is the last
            let item = gpu.material(renderer, scene, pair.1);
            let base = gpu.material(renderer, scene, pair.0);
            (base, item)
        }
    }

    /// Render instances for one vehicle: text and script textures are per vehicle, so the
    /// slots using them get their own textures and materials. Freed slots are taken over.
    /// `shared_script` are the script textures of the vehicle this one shares its scripts
    /// with (they stay that vehicle's).
    #[allow(clippy::too_many_arguments)]
    fn instantiate_vehicle(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        vt: &omsi_sim::VehicleType,
        set: &VehicleSet,
        key: Option<VehicleKey>,
        shared_script: Option<&[Option<TextureId>]>,
    ) -> VehicleRender {
        let mut gpu = self.gpu.lock();
        let blank = |gpu: &mut GpuCache, scene: &mut Scene, w: i32, h: i32| {
            Some(gpu.add_blank(renderer, scene, w.max(1) as u32, h.max(1) as u32))
        };
        let blank_text = |gpu: &mut GpuCache, scene: &mut Scene, w: i32, h: i32| {
            Some(gpu.add_blank_mips(renderer, scene, w.max(1) as u32, h.max(1) as u32))
        };
        let sizes: Vec<(i32, i32)> = vt
            .model
            .text_textures
            .iter()
            .map(|t| (t.width, t.height))
            .collect();
        let text_textures: Vec<Option<TextureId>> = sizes
            .iter()
            .map(|(w, h)| blank_text(&mut gpu, scene, *w, *h))
            .collect();
        let script_textures: Vec<Option<TextureId>> = match shared_script {
            Some(s) => s.to_vec(),
            None => vt
                .model
                .script_textures
                .iter()
                .map(|(w, h)| blank(&mut gpu, scene, *w, *h))
                .collect(),
        };
        let mut instances = Vec::new();
        let mut own_materials = Vec::new();
        let mut variants = set.variants.to_vec();
        for (mi, (id, mats)) in set.meshes.iter().enumerate() {
            let mut mats = mats.clone();
            // A switched slot ([matl_change], [texchanges], [matl_freetex]) that shows the
            // vehicle's own pictures gets all its materials made again with them: the switch
            // sets the slot's material every frame, and the shared pair it took had lost the
            // script texture mask (the Citaro LE's destination displays were solid blocks of
            // the LCD text colour) and the text texture (the O530's ticket printer).
            for v in variants
                .iter_mut()
                .filter(|v| v.mesh == mi && v.spec.per_vehicle())
            {
                let spec = v.spec.for_vehicle(&text_textures, &script_textures);
                let make = |gpu: &mut GpuCache, scene: &mut Scene, tex: Option<TextureId>| {
                    let (base, item) = spec.build(renderer, scene, tex);
                    if base == item {
                        let m = gpu.material(renderer, scene, base);
                        (m, m)
                    } else {
                        // the item was built last: it moves first
                        let item = gpu.material(renderer, scene, item);
                        (gpu.material(renderer, scene, base), item)
                    }
                };
                let (base, item) = make(&mut gpu, scene, v.base_tex);
                let entries: Vec<(MaterialId, MaterialId)> = v
                    .entry_tex
                    .iter()
                    .map(|t| make(&mut gpu, scene, *t))
                    .collect();
                own_materials.extend([base, item]);
                own_materials.extend(entries.iter().flat_map(|e| [e.0, e.1]));
                let more = spec.build_more(renderer, scene, v.base_tex, |scene, m| gpu.material(renderer, scene, m));
                own_materials.extend(more.iter().copied());
                (v.base, v.item, v.more, v.entries, v.spec) = (base, item, more, entries, spec);
                if let Some(l) = &mut v.lights {
                    l.plain = (base, item);
                }
                if let Some(x) = mats.get_mut(v.slot) {
                    *x = v.entries.first().map(|e| e.0).unwrap_or(v.base);
                }
            }
            for d in set.dyn_slots.iter().filter(|d| d.mesh == mi) {
                let Some(x) = mats.get_mut(d.slot) else {
                    continue;
                };
                if let Some(Some(tex)) = d.text.and_then(|i| text_textures.get(i)) {
                    renderer.address_next.set(d.address);
                    // Number/route text is a normal bus material, not an emissive HUD.
                    // Marking it unlit made the glyph RGB stay at full intensity at night,
                    // which turned dark registration characters into glowing white ones.
                    // It keeps the slot's light and night maps: a destination matrix or a
                    // dashboard counter is lit by them ([matl_lightmap] lights_stand,
                    // elec_busbar_main), and without them it stayed dark at night.
                    // Only a slot with a light of its own is a display that glows a little in
                    // the enhanced picture: a fleet number or a number plate on the body
                    // (the EN92's `D_wagennummer.tga`, blended, neither light nor night map)
                    // glowed in the dark with it, where OMSI lights it as the paint (#698).
                    let mut extra = d.extra;
                    extra.display = text_is_display(d.lightmap.is_some(), d.night.is_some());
                    // (the bus's own screen: no glow halo, no FXAA over its letters)
                    extra.screen = true;
                    let m = renderer.add_material_extra(
                        scene,
                        Some(*tex),
                        if d.alpha == AlphaMode::Test { AlphaMode::Test } else { AlphaMode::Blend },
                        [1.0; 4],
                        false,
                        None,
                        d.night,
                        d.lightmap,
                        None,
                        [0.0; 3],
                        extra,
                    );
                    *x = gpu.material(renderer, scene, m);
                    own_materials.push(*x);
                    continue;
                }
                let tex = d
                    .script
                    .and_then(|i| script_textures.get(i).copied().flatten())
                    .or(d.tex);
                let transmap = d
                    .script_trans
                    .and_then(|i| script_textures.get(i).copied().flatten())
                    .map(|t| (t, true))
                    .or(d.transmap);
                let alpha = d.alpha;
                let (color, emissive) = if d.script.is_some() {
                    ([1.0; 4], [0.0; 3])
                } else {
                    (d.color, d.emissive)
                };
                renderer.address_next.set(d.address);
                // a script's screen (matrix displays, the IBIS's picture, LCDs) likewise
                let mut extra = d.extra;
                extra.screen = d.script.is_some() || d.script_trans.is_some();
                // ... and a `\S:n` mask makes it an LED panel: its lit dots are its own
                // light, which the enhanced picture blooms (see `MaterialExtra::led`)
                extra.led = d.script_trans.is_some() && d.extra.led;
                let m = renderer.add_material_extra(
                    scene,
                    tex,
                    alpha,
                    color,
                    d.script.is_some(),
                    transmap,
                    d.night,
                    d.lightmap,
                    d.envmap,
                    emissive,
                    extra,
                );
                *x = gpu.material(renderer, scene, m);
                own_materials.push(*x);
            }
            let shadow = vt
                .meshes
                .get(mi)
                .map(|m| vt.model.meshes[m.def_index].is_shadow)
                .unwrap_or(false);
            let inst = if shadow {
                renderer.add_shadow_blob_instance(scene, *id, DVec3::ZERO, Mat4::IDENTITY, mats)
            } else {
                let i = renderer.add_instance(scene, *id, DVec3::ZERO, Mat4::IDENTITY, mats);
                let casts = vt.meshes.get(mi).map(|m| vt.model.meshes[m.def_index].shadow).unwrap_or(false);
                renderer.set_omsi_caster(scene, i, casts);
                // (the floor and seats under the roof stay out of the snow and the rain)
                renderer.set_roof(scene, i, vt.def.bounding_box.map(|b| b[5] + b[2] * 0.5));
                i
            };
            instances.push(if key.is_some() {
                gpu.instance(renderer, scene, inst)
            } else {
                inst
            });
        }
        // Omsi.exe draws a model mesh after mesh and each material subset in its turn, with
        // the subset's own blend and depth-write states (0x7c32c4 -> 0x7fd6c4), so a slot
        // blended by `[matl_alpha] 2` that writes depth hides what the model lists after it.
        // Where that happens - a blended slot writing depth before an opaque or cut-out one -
        // the whole vehicle is drawn in that order (see `Instance::ordered`); drawn with its
        // opaque parts first, a body blended by its alpha showed the interior through it.
        let slots_in_order = |i: usize| -> Vec<(omsi_render::AlphaMode, bool)> {
            let Some(inst) = scene.instances.get(i) else { return Vec::new() };
            let Some(mesh) = scene.meshes.get(inst.mesh) else { return Vec::new() };
            mesh.ranges
                .iter()
                .filter_map(|(_, _, slot)| inst.materials.get(*slot as usize))
                .filter_map(|&m| scene.materials.get(m))
                // (a pane - a window, or a layer over the glass painted on its own unwrap -
                // writes its depth as Omsi.exe writes it, but it is no blended bodywork:
                // taken as one, a bus whose glass the model lists before its saloon was
                // drawn in model order and its saloon was gone behind every window, the
                // panes on both sides over the street behind them coming out black)
                .map(|m| (m.alpha, !m.no_z_write && !m.no_z_check))
                .collect()
        };
        let mut blended_first = false;
        let mut ordered = false;
        for &i in &instances {
            if scene.instances.get(i).is_none_or(|x| x.blob) {
                continue;
            }
            for (alpha, writes) in slots_in_order(i) {
                match alpha {
                    omsi_render::AlphaMode::Blend if writes => blended_first = true,
                    omsi_render::AlphaMode::Blend => {}
                    _ if blended_first => ordered = true,
                    _ => {}
                }
            }
        }
        // (the Sprinter's, the Mercus's, the Urbino 15's saloon showed through half their
        // panels drawn so while `[matl_noZcheck]` still took their inner glass out of the
        // depth test; OMSI_NO_MODEL_ORDER=1 draws opaque parts first again)
        if ordered && omsi_cfg::env::var_os("OMSI_NO_MODEL_ORDER").is_none() {
            log::debug!("{}: drawn in model order (a blended slot writes depth before an opaque one)", vt.def.path.display());
            for &i in &instances {
                if scene.instances.get(i).is_some_and(|x| !x.blob) {
                    renderer.set_ordered(scene, i, true);
                }
            }
        }
        // the vehicle is drawn or left out as one object (see `set_object_culling`): its
        // sphere about the vehicle's origin, which every mesh instance shares
        let radius = set
            .meshes
            .iter()
            .filter_map(|(id, _)| scene.meshes.get(*id))
            .filter(|m| m.bounds_radius > 0.0)
            .map(|m| m.bounds_center.length() + m.bounds_radius)
            .fold(0.0f32, f32::max);
        let any_distance =
            vt.model.no_distance_check || vt.model.meshes.iter().any(|m| m.no_distance_check);
        for inst in &instances {
            renderer.set_object_culling(scene, *inst, radius, vt.model.detail_factor, any_distance);
        }
        VehicleRender {
            instances,
            text_textures,
            script_textures,
            shared_script: shared_script.is_some(),
            variants,
            own_materials,
            set: key,
            displays_far: false,
            display_tick: 0,
            skinned: Vec::new(),
            hidden: false,
            interior_lamps: std::cell::Cell::new(None),
            interior_blocks: std::cell::OnceCell::new(),
        }
    }

    /// Upload the meshes and materials of a vehicle type: (mesh, materials) per model mesh
    /// and one texture per `[texttexture]`.
    /// Also returns the slots whose textures are generated per vehicle. `player`: the
    /// player's own vehicle, whose mirrors' glass is noted (for the panels).
    fn upload_vehicle(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        vt: &omsi_sim::VehicleType,
        scheme: Option<usize>,
        player: bool,
    ) -> VehicleSet {
        let mut dyn_slots: Vec<DynSlot> = Vec::new();
        let mut variants: Vec<VariantSlot> = Vec::new();
        let mut dirs = vt.texture_dirs(&self.root);
        let (subst, scheme_dir) = match scheme {
            Some(i) => vt.scheme_substitutions(i),
            None => (vt.default_substitutions(&self.root), None),
        };
        if let Some(d) = scheme_dir {
            dirs.insert(0, d);
        }
        // [CTCTexture] slots swapped by the paint scheme
        let subst = |name: &str| -> String {
            subst
                .get(&name.to_ascii_lowercase())
                .cloned()
                .unwrap_or_else(|| name.to_string())
        };
        let dirs_ref: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
        let mut tex_ids = self.vehicle_textures.lock();
        let mut mesh_ids = self.vehicle_meshes.lock();
        // what the set holds, given back when it is trimmed
        let mut held: Vec<PathBuf> = Vec::new();
        let mut mesh_keys: Vec<(PathBuf, usize)> = Vec::new();
        let mut materials: Vec<MaterialId> = Vec::new();
        let t_all = std::time::Instant::now();
        let mut mesh_secs = 0.0f64;
        let tex_time = std::cell::RefCell::new((0usize, 0.0f64));
        macro_rules! tex {
            // (`reflexionN.bmp` wherever a material names it - its light map, its night map,
            // a `[matl_item]`'s - is camera N's picture: a monitor that shows the camera once
            // switched on names it so, and was white)
            ($name:expr, $dirs:expr) => {{
                let nm: &str = &$name;
                match mirror_index(nm) {
                    Some(mi) => Some(self.mirror_texture(renderer, scene, mi)),
                    None => tex!(nm, $dirs, vehicle_texture),
                }
            }};
            ($name:expr, $dirs:expr, $how:ident) => {{
                let t = std::time::Instant::now();
                let n = tex_ids.len();
                let r = self.$how(renderer, scene, &mut tex_ids, &mut held, $name, $dirs);
                if tex_ids.len() > n {
                    let mut tt = tex_time.borrow_mut();
                    tt.0 += 1;
                    tt.1 += t.elapsed().as_secs_f64();
                }
                r
            }};
        }
        let mut instances = Vec::new();
        let mut missing_tex: Vec<String> = Vec::new();
        // OMSI_ONLY_MESH=a|b draws only the meshes whose file names contain one of the
        // parts (and logs their materials); OMSI_HIDE_MESH=a|b leaves those out
        let only = omsi_cfg::env::var("OMSI_ONLY_MESH").ok();
        let hide = omsi_cfg::env::var("OMSI_HIDE_MESH").ok();
        let matches = |list: &str, file: &str| {
            list.split('|').any(|f| {
                !f.is_empty() && file.to_ascii_lowercase().contains(&f.to_ascii_lowercase())
            })
        };
        for (mesh_index, vm) in vt.meshes.iter().enumerate() {
            let def = &vt.model.meshes[vm.def_index];
            if only.as_deref().is_some_and(|f| !matches(f, &def.file))
                || hide.as_deref().is_some_and(|f| matches(f, &def.file))
            {
                // keep instance numbering stable: an empty placeholder mesh
                let id = renderer.add_mesh(scene, &MeshData::default());
                instances.push((id, vec![]));
                continue;
            }
            let mats: Vec<MaterialId> = vm
                .materials
                .iter()
                .enumerate()
                .map(|(slot, m)| {
                    // [useTextTexture] replaces the material's texture by a generated one;
                    // after a [matl_change]'s [matl_item] it is the item's picture alone (the
                    // O530's ticket printer shows its text while the electrics are on, the
                    // plain field otherwise)
                    let of_slot = |o: &&MaterialDef| omsi_sim::vehicle::override_slot(&vm.materials, o) == Some(slot);
                    let itemised = def.materials.iter().filter(of_slot).any(|o| o.item) && def.materials.iter().filter(of_slot).any(|o| !o.item && o.change.is_some());
                    let text_of = |item: Option<bool>| def.materials.iter().filter(of_slot).find(|o| o.use_text_texture.is_some() && item.is_none_or(|i| o.item == i)).map(|o| o.use_text_texture.unwrap().max(0) as usize);
                    let script_of = |item: Option<bool>| def.materials.iter().filter(of_slot).find(|o| o.use_script_texture.is_some() && item.is_none_or(|i| o.item == i)).map(|o| o.use_script_texture.unwrap().max(0) as usize);
                    let (text_slot, script_slot) = (text_of(None), script_of(None));
                    let (text_base, script_base) = if itemised { (text_of(Some(false)), script_of(Some(false))) } else { (text_slot, script_slot) };
                    let (text_item, script_item) = (text_of(Some(true)).or(text_base), script_of(Some(true)).or(script_base));
                    let tex_name = subst(&m.texture);
                    // The film of water on the glass (`[alphascale] Rain_Window_…`) wears
                    // snow crystals while it snows, unless the vehicle brings a seasonal
                    // texture of its own: see `rain::snow_on_glass`.
                    let rain_layer = vm.overrides.iter().filter(of_slot).any(|o| o.alphascale.as_deref().is_some_and(|v| v.trim().to_ascii_lowercase().starts_with("rain_window")));
                    let tex = if is_null_texture(&m.texture) || text_base.is_some() || script_base.is_some() || vt.texchange(&m.texture).is_some() {
                        // a slot fed by [useTextTexture] / [useScriptTexture] gets a
                        // generated picture; the name in the mesh is a placeholder and
                        // looking for it on disk only produced a false "texture not found"
                        None
                    } else if let Some(mi) = mirror_index(&tex_name) {
                        if player {
                            self.note_mirror_aspect(mi, &vm.data, slot);
                        }
                        Some(self.mirror_texture(renderer, scene, mi))
                    } else if rain_layer && snowing() && !seasonal_texture(&tex_name, &dirs_ref) {
                        tex!("", &dirs_ref, snow_glass_texture)
                    } else {
                        tex!(&tex_name, &dirs_ref)
                    };
                    let ov_all: Vec<&MaterialDef> = vm.overrides.iter().filter(|o| omsi_sim::vehicle::override_slot(&vm.materials, o) == Some(slot)).collect();
                    let ov_item: Vec<&MaterialDef> = ov_all.iter().copied().filter(|o| o.item).collect();
                    let ov: Vec<&MaterialDef> = ov_all.iter().copied().filter(|o| !o.item).collect();
                    // (every [matl_change] of the slot: Omsi.exe keeps one switch per record,
                    // each showing its item while its variable is on - the Procity's door
                    // buttons light with door_light_n as well as with haltewunschlampe)
                    let change_vars: Vec<String> = ov.iter().filter_map(|o| o.change.as_ref().map(|c| c.2.clone())).collect();
                    let change_var = change_vars.first().cloned();
                    let base_overrides: Vec<MaterialDef> = ov.iter().map(|o| (*o).clone()).collect();
                    let mut alpha = material_alpha(&vm.materials, slot, &base_overrides);
                    // what the model.cfg says: without [matl_alpha] OMSI draws a slot opaque
                    // and its texture's alpha is only the reflection mask
                    let declared_alpha = alpha;
                    // Dirt.tga/Dreck.tga is an overlay controlled by Dirt_Norm or
                    // Dirt_Wiped. Keep it in the blended no-depth-write path globally,
                    // even when an add-on has a missing or misordered [matl_alpha].
                    let dirt_overlay = ov.iter().any(|o| o.alphascale.as_deref().is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "dirt_norm" | "dirt_wiped")));
                    if dirt_overlay {
                        alpha = AlphaMode::Blend;
                    }
                    // `[alphascale]` is also used by some buses for dirt/paint variables.
                    // Treating every such slot as blended makes an otherwise solid body
                    // translucent on AI vehicles. Only the authored rain-window film is
                    // intrinsically transparent; ordinary body alphascales must retain the
                    // material's declared alpha mode. Stock rain-film materials declare
                    // `[matl_alpha] 2` explicitly, so the variable itself need not promote
                    // a slot into transparency.
                    // a `[isshadow]` mesh is a soft ground decal by convention, its texture's
                    // own alpha fading it out at the edges - without a `[matl_alpha]`
                    // override of its own (most shadow blobs have none) it defaulted to
                    // opaque, so the decal's square base texture painted a solid (often
                    // white or grey) tile under the bus instead of a soft shadow.
                    if def.is_shadow {
                        alpha = AlphaMode::Blend;
                    }
                    // `\S:n` = script texture n as transparency map
                    let script_trans = ov.iter().find_map(|o| o.transmap.clone()).and_then(|t| t.trim().strip_prefix("\\S:").and_then(|n| n.trim().parse::<usize>().ok()));
                    let transmap = ov.iter().find_map(|o| o.transmap.clone()).filter(|t| !t.trim().is_empty() && !t.trim().starts_with("\\S:")).map(|t| subst(&t)).and_then(|t| {
                        let id = tex!(&t, &dirs_ref)?;
                        let has_alpha = self.textures.has_alpha(&t, &dirs_ref).unwrap_or(false);
                        Some((id, has_alpha))
                    });
                    // A few bus packs mark a solid body mesh as `[matl_alpha] 2` and leave
                    // a non-opaque diffuse material alpha on it (the O530 Facelift's
                    // `wagenkasten_embl_eev.o3d` is a concrete example). That alpha belongs
                    // to the paint/reflection data, not to a window, so treating the whole
                    // panel as a blended surface makes the cabin and traffic show through.
                    // Keep real glass/dirt/display layers blended, and keep explicit
                    // transmaps on the mask path; repair only the unambiguous body case.
                    let mesh_name = def.file.to_ascii_lowercase();
                    let transparent_layer_name = ["regen", "dreck", "dirt", "folie"];
                    let material_name = format!("{} {}", mesh_name, m.texture).to_ascii_lowercase();
                    let named_pane = GLASS_WORDS
                        .iter()
                        .chain(transparent_layer_name.iter())
                        .any(|part| material_name.contains(part));
                    // A pane whose name says nothing: its faces lie on a see-through part of
                    // its texture. No list of words finds the SOR NB12's `celokint.o3d` (its
                    // windscreen), `okridic.o3d` (the driver's window) or `vyklopnel1.o3d`
                    // (a tilting window): taken for bodywork they wrote their depth, and the
                    // glow of every lamp and the lit lenses of the traffic lights behind them
                    // were gone - seen only through an opened window.
                    // The alpha that says so is the [matl_transmap]'s where the slot has one:
                    // the diffuse alpha is then only the reflection mask (the stock Golf 2's
                    // body texture is 0 almost everywhere, its transmap opaque). Read from
                    // the diffuse texture, every transmapped car body wrote no depth, and its
                    // wheel arches, far wheels and interior drawn after it showed through the
                    // paint (#928, #932).
                    let coverage_tex = subst(coverage_texture(ov.iter().find_map(|o| o.transmap.as_deref()), &m.texture));
                    let coverage = omsi_texture::find_texture(&coverage_tex, &dirs_ref).and_then(|p| alpha_mask(&p));
                    // (an invisible cover: clear all over and writing its depth - no pane, it is
                    // there to hide what comes after it, see `texture_is_clear`)
                    let cover = declared_alpha == AlphaMode::Blend
                        && !ov.iter().any(|o| o.no_z_write || o.no_z_check)
                        && coverage.as_ref().is_some_and(|mask| texture_is_clear(mask));
                    if cover {
                        log::debug!("  {} slot {slot} '{}': an invisible cover (clear texture), writes depth in model order", def.file, m.texture);
                    }
                    let see_through = !named_pane
                        && !cover
                        && declared_alpha == AlphaMode::Blend
                        && coverage.as_ref().is_some_and(|mask| slot_is_see_through(&vm.data, slot, mask));
                    if see_through {
                        log::debug!("  {} slot {slot} '{}': see-through by its texture's alpha, writes no depth", def.file, m.texture);
                    }
                    // (the name alone still says what is drawn as glass: the same test finds
                    // a gauge's needle film, a blind's net and the shadow under the bus)
                    let transparent_layer_hint = named_pane;
                    let named_body = ["body", "wagenkasten", "karos", "chassis", "kuzov"].iter().any(|part| mesh_name.contains(part));
                    let mesh_has_overlay = def.materials.iter().any(|o| o.no_z_write);
                    // (a body-sized part in any case: a name or a bump map alone also took a
                    // dashboard's display or a sticker on a mesh called "body" for bodywork)
                    let body_hint = (named_body || ov.iter().any(|o| o.bumpmap.is_some()) || !mesh_has_overlay)
                        && material_has_vehicle_volume(&vm.data, slot);
                    // a layer over another mesh of the same shape drawn before it (the WH UK
                    // AI cars' baked shading over their paint, `[matl_alpha] 2`): blended as
                    // the model says - made opaque, the dark bake covered the paint and the
                    // cars drove about black, or with black roofs
                    let layer = vt.mesh_boxes.get(mesh_index).is_some_and(|&(lo, hi)| {
                        (hi - lo).max_element() > 0.5
                            && vt.mesh_boxes[..mesh_index].iter().any(|&(l2, h2)| (l2 - lo).abs().max_element() < 0.03 && (h2 - hi).abs().max_element() < 0.03)
                    });
                    // (Retired: a body blended by `[matl_alpha] 2` is drawn as Omsi.exe draws
                    // it, in model order with its depth written - see `Instance::ordered` -
                    // instead of being guessed opaque, which drew overlay layers black, #127.
                    // `OMSI_REPAIR_BODY_DEPTH=1` brings the old guess back for comparison.)
                    let repair_body_depth = omsi_cfg::env::var_os("OMSI_REPAIR_BODY_DEPTH").is_some() && !layer && is_vehicle_body_material(&def.file, &m.texture, tex.is_some(), transmap.is_some(), ov.iter().any(|o| o.no_z_write), body_hint);
                    // (only a blended slot: an alpha-tested one - `[matl_alpha] 1`, the EN92's
                    // pictograms, a Sprinter's seat covers - is cut out as the model says, and
                    // made opaque its cut-out parts were grey boxes; and not a layer made of
                    // the same faces as another slot of its mesh, an ambient-occlusion or
                    // shading film over the floor, which drawn opaque was black)
                    if repair_body_depth && alpha == AlphaMode::Blend && !dirt_overlay && !transparent_layer_hint && !slot_overlays_another(&vm.data, slot) {
                        alpha = AlphaMode::Opaque;
                    }
                    // Body-volume heuristics must never turn a named pane back into an
                    // opaque draw (the windscreen became a pale grey wall from inside after
                    // the body-depth repair) - but only a pane the model.cfg declares
                    // blended: a "glass" slot without [matl_alpha] is opaque in OMSI (the
                    // LiAZ's dark glass_gr.dds around its displays and over its windows,
                    // which drawn blended let the sky show through the body).
                    if transparent_layer_hint && !dirt_overlay && declared_alpha == AlphaMode::Blend {
                        alpha = AlphaMode::Blend;
                    }
                    // Keep the material's declared alpha mode: a transmap mask alone must not
                    // make a solid body panel translucent.
                    if omsi_cfg::env::var_os("OMSI_FORCE_OPAQUE").is_some() && !dirt_overlay {
                        alpha = AlphaMode::Opaque;
                    }
                    // (a night or light map named as a [CTCTexture] is the paint scheme's
                    // picture as well, like the diffuse texture and the transparency map:
                    // looked up by the model's own name, a destination display lit by its own
                    // texture glowed with the model's default text over the repaint's, #895)
                    let night = ov.iter().find_map(|o| o.nightmap.clone()).and_then(|t| {
                        tex!(&subst(&t), &dirs_ref)
                    });
                    let lightmap = ov.iter().find_map(|o| o.lightmap.clone()).and_then(|(t, _)| {
                        tex!(&subst(&t), &dirs_ref)
                    });
                    // (a `\S:n` panel lit all over by its light map is an LED panel; one
                    // whose light map is a picture is a flipdot: see `is_white_lightmap`)
                    let lm_white = |ov: &[&MaterialDef]| -> bool {
                        ov.iter().find_map(|o| o.lightmap.as_ref()).and_then(|(t, _)| lightmap_is_white(&subst(t), &dirs_ref)).unwrap_or(true)
                    };
                    // [matl_envmap] tex factor: reflectivity = factor (saturating at 1) x the
                    // reflection mask, which is the [matl_envmap_mask]'s alpha or else the
                    // diffuse alpha - 1 for a texture without an alpha channel, as D3D samples
                    // it (a BC1 texture samples as 1 too): the SD200's dashboard (24-bit
                    // bitmap, factor 0.1) keeps a faint gloss. The mask matters: the Citaro's
                    // doors and the O530 Facelift's bodies carry a paint whose alpha is 255 and
                    // a separate mask of about 6-10 %; read as the mask, the alpha made them
                    // mirrors. The mask and the bump map are shared vehicle textures like the
                    // rest (the bump map as a height map under a key of its own).
                    let envmap = ov.iter().find_map(|o| o.envmap.clone()).filter(|_| omsi_cfg::env::var_os("OMSI_NO_ENVMAP").is_none()).and_then(|(t, f)| {
                        let id = tex!(&t, &dirs_ref)?;
                        Some((id, f))
                    });
                    let env_mask = ov.iter().find_map(|o| o.envmap_mask.clone()).filter(|t| envmap.is_some() && !t.trim().is_empty()).and_then(|t| tex!(&subst(&t), &dirs_ref));
                    let bump = ov.iter().find_map(|o| o.bumpmap.clone()).filter(|_| envmap.is_some() && omsi_cfg::env::var_os("OMSI_NO_BUMP").is_none()).and_then(|(t, f)| tex!(&subst(&t), &dirs_ref, vehicle_bump_texture).map(|id| (id, f)));
                    // a [matl_freetex] slot gets its texture from a string variable at run
                    // time, so an empty slot here is not a missing file
                    let freetex = ov_all.iter().any(|o| o.freetex.is_some());
                    if tex.is_none() && !is_null_texture(&m.texture) && text_slot.is_none() && script_slot.is_none() && !freetex && vt.texchange(&m.texture).is_none() {
                        missing_tex.push(format!("{} ({})", tex_name, def.file));
                    }
                    if only.is_some() {
                        log::info!("  {} slot {slot} '{}' diffuse={:?} emissive={:?} specular={:?}/{} tex={:?} alpha={:?} transmap={:?} night={:?} light={:?} env={:?} mask={:?} bump={:?} text={:?} script={:?} script_trans={:?} noZwrite={} noZcheck={} zbias={}", def.file, m.texture, m.diffuse, m.emissive, m.specular, m.specular_power, tex, alpha, transmap, night, lightmap, envmap, env_mask, bump, text_slot, script_slot, script_trans, ov.iter().any(|o| o.no_z_write), ov.iter().any(|o| o.no_z_check), ov.iter().map(|o| o.z_bias).find(|b| *b != 0).unwrap_or(0));
                    }
                    let textured = tex.is_some() || text_slot.is_some() || script_slot.is_some() || freetex || vt.texchange(&m.texture).is_some();
                    let (color, emissive, specular, ambient) = d3d_material(m, ov.iter().find_map(|o| o.allcolor), textured);
                    let mut extra = material_extra(&ov, env_mask, bump, specular);
                    extra.ambient = Some(ambient);
                    // A vehicle's [matl_nightmap] is added whenever the mesh is drawn, by day
                    // as well, as OMSI 2 does - with or without a [matl_change] around it.
                    // Its lamps and displays are switched by the mesh's [visible] variable or
                    // by what the script draws, not by the time of day: faded in with the
                    // night, a dashboard's warning lamps stayed dark in the daylight (#497).
                    extra.night_switched = night.is_some();
                    // a script's screen (matrix displays, the IBIS's picture, LCDs) is the
                    // glow's and FXAA's business (see `MaterialExtra::screen`), and a `\S:n`
                    // mask makes it an LED panel whose lit dots are its own light
                    // (`MaterialExtra::led`, the enhanced picture's bloom). A slot that is a
                    // `[matl_item]` variant keeps its materials here, not in `dyn_slots`:
                    // without the flags on this `extra` the K++ and Krueger panels showed
                    // their dots but never glowed.
                    extra.screen = script_slot.is_some() || script_trans.is_some();
                    extra.led = script_trans.is_some() && lm_white(&ov);
                    if dirt_overlay {
                        extra.no_z_write = true;
                    }
                    // (chrome: a small opaque part with a sphere map, not the body - see
                    // `MaterialExtra::metal_ok`)
                    extra.metal_ok = envmap.is_some() && alpha == AlphaMode::Opaque && !named_body && !material_has_vehicle_volume(&vm.data, slot);
                    // A few stock vehicles leave noZwrite off on window/dirt materials even
                    // though their alpha mode is Blend. They are transparent colour layers,
                    // not solid shadow casters; letting them into the shadow map paints the
                    // bus shadow with the pane/film texture (the striped triangular artifact).
                    // (Its depth is still written as Omsi.exe writes it, whenever the model
                    // blends the slot by [matl_alpha] 2 without [matl_noZwrite] - a dirt
                    // film's as well: see `MaterialExtra::writes_depth`. Left out of the
                    // depth buffer, the stacked panes of a door blended over each other
                    // whichever lay in front, #211.)
                    if (transparent_layer_hint || see_through) && !cover && alpha == AlphaMode::Blend {
                        extra.writes_depth = declared_alpha == AlphaMode::Blend && !ov.iter().any(|o| o.no_z_write) && !def.is_shadow;
                        extra.no_z_write = true;
                    }
                    // Name the pane explicitly for the shader. A plain blended window has
                    // neither an envmap nor a transmap to identify it, while dirt/rain films
                    // must remain overlays and must not reveal the cabin behind themselves.
                    extra.glass = transparent_layer_hint
                        && alpha == AlphaMode::Blend
                        && !dirt_overlay
                        && !rain_layer;
                    // (while it snows the film is the snow-crystal texture, drawn as it is)
                    // (all three graphics: OMSI 2's own rain, its texture sliding down the
                    // pane, looked like wet paper next to drops that bend the street)
                    extra.rain_film = rain_layer && !snowing() && omsi_cfg::env::var_os("OMSI_TEXTURE_RAIN").is_none();
                    // Some mod buses put [matl_noZcheck] on the complete body mesh.
                    // That flag is for decals; on a body it disables depth writing and
                    // lets the cabin bleed through the outside shell. Keep it on genuine
                    // overlays, but make a repaired body a normal depth-writing surface.
                    if repair_body_depth {
                        extra.no_z_check = false;
                    }
                    // Text textures repeat like any other (Direct3D's default): the D-series
                    // Annax meshes address their lines at v = -0.85..-0.39, and clamped they
                    // showed nothing but the empty top row. Number plates, whose UVs run far
                    // past the edges, ask for [matl_texadress_clamp] themselves.
                    let address = tex_addressing(ov.iter().copied());
                    let base_dyn = DynTex { text: text_base, script: script_base, script_trans, address };
                    // a mirror already holds a rendered picture of the lit world, so it is
                    // drawn as it is; shading it again by the glass's own normal (which
                    // faces backwards, away from the sun) is what made mirrors look black
                    let unlit = mirror_index(&tex_name).is_some();
                    // [matl_item] variant: same slot with the item's own maps / colours
                    // (Omsi.exe keeps every [matl_item] of a [matl_change] as a material of its
                    // own and shows item round(x): a door button at 2 - lit while its door is
                    // open - showed the plain dark material, and item 2's maps leaked into item
                    // 1, #352. Each item of the first [matl_change] is made of its own block:
                    // item 1 read item 2's `\S:n` mask, and an LED matrix showed the script
                    // texture at 1 instead of its boot picture, #210. Items of a later
                    // [matl_change] still merge into item 1.)
                    let later_items: Vec<&MaterialDef> = {
                        let mut changes = 0;
                        let mut first = Vec::new();
                        for o in &ov_all {
                            if !o.item && o.change.is_some() {
                                changes += 1;
                            } else if o.item && changes == 1 {
                                first.push(*o);
                            }
                        }
                        first.into_iter().skip(1).collect()
                    };
                    let mut item_look = |ov_item: &Vec<&MaterialDef>| -> Look {
                        let mut find_tex = |t: &str| -> Option<TextureId> { tex!(t, &dirs_ref) };
                        let it_night = ov_item.iter().find_map(|o| o.nightmap.clone()).and_then(|t| find_tex(&subst(&t))).or(night);
                        let it_light = ov_item.iter().find_map(|o| o.lightmap.clone()).and_then(|(t, _)| find_tex(&subst(&t))).or(lightmap);
                        // the item's own transparency map, else the plain material's
                        let it_script_trans = match ov_item.iter().find_map(|o| o.transmap.clone()) {
                            Some(t) => t.trim().strip_prefix("\\S:").and_then(|n| n.trim().parse::<usize>().ok()),
                            None => script_trans,
                        };
                        let it_trans = ov_item.iter().find_map(|o| o.transmap.clone()).filter(|t| !t.trim().is_empty() && !t.trim().starts_with("\\S:")).and_then(|t| {
                            let id = find_tex(&subst(&t))?;
                            let has_alpha = self.textures.has_alpha(&t, &dirs_ref).unwrap_or(false);
                            Some((id, has_alpha))
                        }).or(transmap);
                        // `[matl_item]` inherits the base alpha mode. A transmap only supplies
                        // the mask; it must not turn an otherwise opaque body variant into a
                        // blended mesh (which makes the whole shared slot look like glass).
                        // (An item block that never set `[matl_alpha]` carries OMSI's 0, not an
                        // alpha of its own: read as one, a K++ panel's item - the half the
                        // busbar switches to - was opaque, its `\S:n` mask cut nothing, and the
                        // whole matrix was lit.)
                        let it_alpha = if repair_body_depth { AlphaMode::Opaque } else { ov_item.iter().find(|o| o.alpha_set).map(|o| alpha_mode(o.alpha)).unwrap_or(alpha) };
                        let (it_color, it_emissive, it_specular, it_ambient) = d3d_material(m, ov_item.iter().find_map(|o| o.allcolor).or(ov.iter().find_map(|o| o.allcolor)), textured);
                        let mut it_extra = material_extra(&ov_item, env_mask, bump, it_specular);
                        it_extra.ambient = Some(it_ambient);
                        // (an item without a night map of its own keeps the plain one, lit
                        // the same way)
                        it_extra.night_switched = it_night.is_some();
                        it_extra.screen = script_item.is_some() || it_script_trans.is_some();
                        // (the item's `\S:n`, or the one it inherits from its base, keeps it
                        // an LED panel: see `MaterialExtra::led`)
                        it_extra.led = it_script_trans.is_some() && if ov_item.iter().any(|o| o.lightmap.is_some()) { lm_white(ov_item) } else { lm_white(&ov) };
                        it_extra.no_z_write |= extra.no_z_write;
                        it_extra.no_z_check |= extra.no_z_check;
                        it_extra.glass |= extra.glass;
                        if repair_body_depth {
                            it_extra.no_z_check = false;
                        }
                        let it_dyn = DynTex { text: text_item, script: script_item, script_trans: it_script_trans, address };
                        Look { alpha: it_alpha, color: it_color, emissive: it_emissive, unlit: false, diffuse: None, transmap: it_trans, night: it_night, lightmap: it_light, envmap, extra: it_extra, dyn_tex: it_dyn }
                    };
                    let first_item: Vec<&MaterialDef> = ov_item.iter().copied().filter(|o| !later_items.iter().any(|l| std::ptr::eq(*l, *o))).collect();
                    let item_spec = (change_var.is_some() && !ov_item.is_empty()).then(|| item_look(&first_item));
                    let more_items: Vec<Look> = if item_spec.is_some() { later_items.iter().map(|o| item_look(&vec![*o])).collect() } else { Vec::new() };
                    if only.is_some() {
                        if let Some(it) = &item_spec {
                            log::info!("  {} slot {slot} item (switched by {:?}): alpha={:?} night={:?} light={:?} switched={}", def.file, change_var, it.alpha, it.night, it.lightmap, it.extra.night_switched);
                        }
                    }
                    // [matl_noZwrite]: glass, the rain film and the dirt layer are blended
                    // and must not write depth, or everything blended behind them is thrown
                    // away and the window turns into a pale hole in the world
                    let spec = SlotSpec { base: Look { alpha, color, emissive, unlit, diffuse: None, transmap, night, lightmap, envmap, extra, dyn_tex: base_dyn }, item: item_spec, more: more_items };
                    // [texchanges]: the texture named in the mesh is only a key - the master
                    // of that name holds the textures a script variable switches between
                    // (the SD200's roller blinds, the seat covers of the AI interior).
                    let master = vt.texchange(&m.texture);
                    let entry_tex: Vec<Option<TextureId>> = match master {
                        Some(master) => {
                            let mut edirs: Vec<&Path> = vec![master.dir.as_path()];
                            edirs.extend(dirs_ref.iter().copied());
                            let mut find_tex = |t: &str| -> Option<TextureId> { tex!(t, &edirs) };
                            master.entries.iter().map(|e| find_tex(&subst(e))).collect()
                        }
                        None => Vec::new(),
                    };
                    if let (Some(master), true) = (master, only.is_some()) {
                        log::info!("    [texchanges] {} -> {} entries by '{}', loaded {:?}", master.texture, master.entries.len(), master.variable, entry_tex);
                    }
                    let base_tex = if master.is_some() { entry_tex.first().copied().flatten() } else { tex };
                    let built = spec.build(renderer, scene, base_tex);
                    let (base, item) = self.recycle_pair(renderer, scene, built);
                    materials.extend([base, item]);
                    let entries: Vec<(MaterialId, MaterialId)> = entry_tex
                        .iter()
                        .map(|t| {
                            let built = spec.build(renderer, scene, *t);
                            self.recycle_pair(renderer, scene, built)
                        })
                        .collect();
                    materials.extend(entries.iter().flat_map(|e| [e.0, e.1]));
                    let more = spec.build_more(renderer, scene, base_tex, |scene, m| self.gpu.lock().material(renderer, scene, m));
                    materials.extend(more.iter().copied());
                    // [matl_freetex]: the file is only known at run time (the destination
                    // roller builds its path from the map's depot and terminus strings)
                    let free: Vec<FreeTex> = free_texture_defs(&ov_all).into_iter().map(|(item_only, key, var)| FreeTex {
                        var,
                        diffuse: key.eq_ignore_ascii_case(&m.texture),
                        key: tex!(&subst(&key), &dirs_ref),
                        item_only,
                        dirs: dirs.clone(),
                        textures: self.textures.clone(),
                        cache: HashMap::new(),
                        current: None,
                        shared: self.vehicle_textures.clone(),
                        held: Vec::new(),
                        wants_upgrade: self.freetex_upgrades.clone(),
                    }).collect();
                    let multi_light = |base: MaterialId, item: MaterialId| -> Option<MultiLight> {
                        let list = ov.iter().map(|o| &o.lightmaps).find(|l| !l.is_empty())?;
                        let maps: Vec<(PathBuf, String)> = list
                            .iter()
                            .filter_map(|(t, v)| omsi_texture::find_texture(&subst(t), &dirs_ref).map(|p| (p, v.clone())))
                            .collect();
                        (maps.len() >= 2 && maps.len() <= 8).then(|| MultiLight {
                            maps,
                            plain: (base, item),
                            cache: HashMap::new(),
                            // (none yet: the first frame makes the materials of what is on)
                            current: u32::MAX,
                            shared: self.vehicle_textures.clone(),
                            held: Vec::new(),
                        })
                    };
                    if spec.item.is_some() || !entries.is_empty() || !free.is_empty() {
                        let tex_var = master.map(|m| m.variable.clone()).unwrap_or_default();
                        variants.push(VariantSlot { mesh: instances.len(), slot, base, item, more, var: change_var.unwrap_or_default(), more_vars: change_vars.iter().skip(1).cloned().collect(), entries, tex_var, free, spec, base_tex, entry_tex, lights: multi_light(base, item) });
                    } else if let Some(lights) = multi_light(base, item) {
                        variants.push(VariantSlot { mesh: instances.len(), slot, base, item, more: Vec::new(), var: String::new(), more_vars: Vec::new(), entries, tex_var: String::new(), free: Vec::new(), spec, base_tex, entry_tex, lights: Some(lights) });
                    } else if base_dyn.any() {
                        dyn_slots.push(DynSlot { mesh: instances.len(), slot, text: text_slot, script: script_slot, script_trans, tex, alpha, transmap, night, lightmap, envmap, address, extra, color, emissive });
                    }
                    base
                })
                .collect();
            let t_mesh = std::time::Instant::now();
            let id = self.vehicle_mesh(
                renderer,
                scene,
                &mut mesh_ids,
                &mut mesh_keys,
                vt,
                mesh_index,
            );
            mesh_secs += t_mesh.elapsed().as_secs_f64();
            instances.push((id, mats));
        }
        if !missing_tex.is_empty() {
            missing_tex.sort();
            missing_tex.dedup();
            log::warn!(
                "{}: {} material slots have no texture: {:?}",
                vt.def
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
                missing_tex.len(),
                missing_tex
            );
        }
        materials.sort_unstable();
        materials.dedup();
        if omsi_cfg::env::var_os("OMSI_PROFILE").is_some() {
            let (tn, ts) = *tex_time.borrow();
            log::info!("  vehicle set {}: {} meshes ({:.1} ms), {} textures uploaded ({:.1} ms), {} materials, {:.1} ms in all", vt.def.path.file_name().unwrap_or_default().to_string_lossy(), mesh_keys.len(), mesh_secs * 1000.0, tn, ts * 1000.0, materials.len(), t_all.elapsed().as_secs_f64() * 1000.0);
        }
        VehicleSet {
            meshes: instances,
            dyn_slots,
            variants,
            textures: held,
            mesh_keys,
            materials,
            users: 0,
            idle_since: Some(std::time::Instant::now()),
        }
    }
}

/// A path's `[rule] trafficdensity`s: how much random traffic of any group it carries,
/// and the last value per group (the rule's fourth line: the group's place in the map's
/// `unsched_vehgroups.txt`). Without a rule for the first group the path has its medium
/// density (1); the lane carries traffic as long as any group drives on it - on
/// Berlin-Spandau 462 Falkensee paths set only the GDR cars' density.
fn path_densities(rules: &[omsi_map::MapRule], path: usize) -> (f32, Vec<(u16, f32)>) {
    let mut per: Vec<(u16, f32)> = Vec::new();
    for r in rules.iter().filter(|r| {
        r.path_index == path as i32 && r.kind.eq_ignore_ascii_case("trafficdensity") && !r.kill
    }) {
        let g = r.extra.max(0.0) as u16;
        let v = (r.value as f32).max(0.0);
        match per.iter_mut().find(|(k, _)| *k == g) {
            Some(e) => e.1 = v,
            None => per.push((g, v)),
        }
    }
    let first = per.iter().find(|(g, _)| *g == 0).map(|e| e.1).unwrap_or(1.0);
    let density = per.iter().map(|e| e.1).fold(first, f32::max);
    (density, per)
}

/// Lanes of one map spline: every `[path]` of the spline type runs along the curve at its
/// lateral offset; `direction` 1 runs backwards, 2 both ways (two lanes).
///
/// A spline placed with `mirror` has its cross-section turned over: each path lies on the
/// other side of the centre line and runs the other way, as its carriageway does (a right
/// lane that ran forward is a left lane running backward - traffic still keeps right).
/// Ignoring the flag put every lane of Spandau's 40-odd mirrored road pieces 5 to 20 m
/// beside its road and the wrong way round; a timetable route through one had its bus turn
/// into the oncoming lanes and jump back where the next piece began.
fn spline_lanes(
    def: &Spline,
    s: &omsi_map::MapSpline,
    curve: &SplineCurve,
    tile: (i32, i32),
) -> Vec<Lane> {
    let mut out = Vec::new();
    let side = if s.mirror { -1.0 } else { 1.0 };
    let curve = &curve.with_sli(def);
    for (pi, p) in def.paths.iter().enumerate() {
        let n = ((curve.length / 3.0).ceil() as usize).clamp(1, 300);
        let pts: Vec<DVec3> = (0..=n)
            .map(|i| {
                curve.offset_point(
                    curve.length * i as f64 / n as f64,
                    side * p.start[0] as f64,
                    p.start[2] as f64,
                )
            })
            .collect();
        let kind = LaneKind::from_code(p.kind);
        let limit = s
            .rules
            .iter()
            .filter(|r| {
                r.path_index == pi as i32
                    && r.kind.eq_ignore_ascii_case("speedlimit")
                    && r.value > 0.0
            })
            .map(|r| r.value as f32)
            .last();
        // The other rules of this path: how much traffic the mapper wants here at all.
        // Berlin-Spandau alone carries 8516 [rule] trafficdensity and 68 no_cars, and with
        // them ignored cars appeared in pedestrian streets, depot yards and back lanes the
        // original keeps empty.
        let rule_of = |name: &str| {
            s.rules
                .iter()
                .filter(|r| {
                    r.path_index == pi as i32 && r.kind.eq_ignore_ascii_case(name) && !r.kill
                })
                .map(|r| r.value as f32)
                .last()
        };
        let (density, group_density) = path_densities(&s.rules, pi);
        let no_cars = s.rules.iter().any(|r| {
            r.path_index == pi as i32 && r.kind.eq_ignore_ascii_case("no_cars") && !r.kill
        });
        // (`bus` and `trucks` are switches that open the path to those AI vehicles, see
        // `Lane::allows`; `bus` does not close it to cars)
        let rule_bus = rule_of("bus").is_some();
        let rule_trucks = rule_of("trucks").is_some();
        let priority = rule_of("priority");
        let mut push = |pts: Vec<DVec3>, reversed: bool| {
            let mut l = LaneBuilder::polyline(pts, kind, p.width);
            if let Some(v) = priority {
                l.priority = v;
            }
            if let Some(v) = limit {
                l.speed_limit_kmh = v;
            }
            l.density = density;
            l.group_density = group_density.clone();
            l.no_cars = no_cars;
            l.rule_bus = rule_bus;
            l.rule_trucks = rule_trucks;
            l.source = 1;
            l.key = Some(LaneKey {
                tile,
                id: s.id,
                path: pi as u16,
            });
            l.reversed = reversed;
            l.offset = side as f32 * p.start[0];
            l.name = def
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            out.push(l);
        };
        // (a mirrored spline's forward path runs backwards and the other way round)
        match (p.direction, s.mirror) {
            (2, _) => {
                push(pts.clone(), false);
                push(pts.into_iter().rev().collect(), true);
            }
            (1, false) | (0, true) => push(pts.iter().rev().copied().collect(), true),
            _ => push(pts, false),
        }
    }
    out
}

/// The crossing light a placed child names (see `ScriptedObject::light_parent`): its
/// `[varparent]` is a crossing whose program runs and its first string a light index.
fn light_child_of(crossings: &hashbrown::HashSet<i64>, var_parent: Option<i64>, strings: &[String]) -> Option<(i64, usize)> {
    let parent = var_parent.filter(|p| crossings.contains(p))?;
    if !crate::tiles::names_traffic_light(strings) {
        return None;
    }
    let index = strings.first()?.trim().parse::<usize>().ok()?;
    Some((parent, index))
}

fn traffic_light_program_enabled(sco: &SceneryObject, has_signals: bool) -> bool {
    !sco.traffic_lights.is_empty() && (has_signals || sco.is_traffic_light)
}

/// Lanes of one placed scenery object: `[path]` arcs in the object frame (x right,
/// y forward, z up; heading clockwise, radius > 0 right turn) rotated by the object heading.
fn object_lanes(
    sco: &SceneryObject,
    pos: DVec3,
    rot: [f64; 3],
    controller: Option<usize>,
    tile: (i32, i32),
    id: i64,
    rules: &[omsi_map::MapRule],
) -> Vec<Lane> {
    let mut out = Vec::new();
    let heading = rot[0];
    let h = heading.to_radians();
    let (sh, ch) = (h.sin(), h.cos());
    // local (x east, y north) → world for an object turned clockwise by `heading`
    let to_world = |x: f64, y: f64| DVec2::new(x * ch + y * sh, -x * sh + y * ch);
    for (pi, p) in sco.paths.iter().enumerate() {
        let v = &p.params;
        if v.len() < 11 {
            continue;
        }
        let start_local = to_world(v[0] as f64, v[1] as f64);
        let start = DVec3::new(
            pos.x + start_local.x,
            pos.y + start_local.y,
            pos.z + v[2] as f64,
        );
        let (path_heading, radius, length) = (v[3] as f64 + heading, v[4] as f64, v[5] as f64);
        if length <= 0.01 {
            continue;
        }
        let (grad_start, grad_end) = (v[6] as f64, v[7] as f64);
        let kind = LaneKind::from_code(p.kind);
        let turn = match v.get(11).map(|t| *t as i32) {
            Some(2) => 1,
            Some(3) => 2,
            _ => 0,
        };
        // The `[rule]`s the map put on this object's path. Most of a map's rules sit on the
        // junctions, not on the splines - Berlin-Spandau has 5402 trafficdensity, 693
        // speedlimit, 576 trucks and 64 no_cars on objects against 3114/810/284/4 on splines
        // - so ignoring them left cars turning into every yard and pedestrian street.
        let rule_of = |name: &str| {
            rules
                .iter()
                .filter(|r| {
                    r.path_index == pi as i32 && r.kind.eq_ignore_ascii_case(name) && !r.kill
                })
                .map(|r| r.value as f32)
                .last()
        };
        let limit = rules
            .iter()
            .filter(|r| {
                r.path_index == pi as i32
                    && r.kind.eq_ignore_ascii_case("speedlimit")
                    && r.value > 0.0
                    && !r.kill
            })
            .map(|r| r.value as f32)
            .last();
        let (density, group_density) = path_densities(rules, pi);
        let no_cars = rules.iter().any(|r| {
            r.path_index == pi as i32
                && r.kind.eq_ignore_ascii_case("no_cars")
                && !r.kill
        });
        let rule_bus = rule_of("bus").is_some();
        let rule_trucks = rule_of("trucks").is_some();
        // who goes first where this path meets another (`Network::must_yield`)
        let priority = rule_of("priority");
        let mut push = |reverse: bool| {
            let mut l = LaneBuilder::arc_with_gradients(start, path_heading, length, radius, grad_start, grad_end, kind, p.width);
            if reverse {
                let pts: Vec<DVec3> = l.points.iter().rev().copied().collect();
                l = LaneBuilder::polyline(pts, kind, p.width);
            }
            if let Some(v) = limit {
                l.speed_limit_kmh = v;
            }
            l.density = density;
            l.group_density = group_density.clone();
            l.no_cars = no_cars;
            l.rule_bus = rule_bus;
            l.rule_trucks = rule_trucks;
            l.turn = turn;
            if let Some(v) = priority {
                l.priority = v;
            }
            l.source = 2;
            l.key = Some(LaneKey {
                tile,
                id,
                path: pi as u16,
            });
            l.blocks = sco.path_blocks.get(pi).map(|b| b.iter().filter(|(n, _)| *n >= 0).map(|(n, _)| *n as u16).collect()).unwrap_or_default();
            l.reversed = reverse;
            l.traffic_light = controller.and_then(|c| {
                sco.path_traffic_light
                    .get(pi)
                    .copied()
                    .filter(|t| *t >= 0)
                    .map(|t| (c, t as usize))
            });
            out.push(l);
        };
        match p.direction {
            1 => push(true),
            2 => {
                push(false);
                push(true);
            }
            _ => push(false),
        }
    }
    out
}

/// Resolve the texture name for a scenery object's `[matl_freetex]` slot.
/// Tries the object's script variable first, then freetex probe, and falls back to
/// tile placement strings (by explicit numeric index or by freetex declaration order).
///
/// The object's own string variable owns the slot: when its `[stringvarnamelist]` (or a
/// script) declares `var`, its value is the file name and an empty value means the slot
/// has none - the object's own texture stays, as in Omsi.exe. Reading the placement
/// strings of another variable instead put a bus stop sign's route number (`72`, `A47X`)
/// or a stop name in a `[matl_freetex]` slot, which Omsi.exe never does; those are the
/// `[texttexture]` sizes of the sign, not files (#1756).
pub(crate) fn resolve_scenery_freetex_name<'a>(
    var: &str,
    override_: &MaterialDef,
    overrides: &[MaterialDef],
    object_script: Option<&'a omsi_sim::scenery::SceneryInstance>,
    freetex_probe: Option<&'a omsi_sim::scenery::SceneryInstance>,
    strings: &'a [String],
) -> Option<&'a str> {
    let script_name = object_script.map(|s| s.str_var(var).trim()).unwrap_or("");
    let probe_name = freetex_probe.map(|p| p.str_var(var).trim()).unwrap_or("");
    let declared = object_script.map(|s| s.program.str_var(var).is_some()).unwrap_or(false)
        || freetex_probe.map(|p| p.program.str_var(var).is_some()).unwrap_or(false);
    if declared {
        // a name a script sets in {frame} is only in the probe's state at first (see the
        // caller), so both are read; neither having one leaves the slot's own texture
        let name = if !script_name.is_empty() { script_name } else { probe_name };
        let name = name.trim_matches('"');
        return (!name.is_empty()).then_some(name);
    }
    let string_by_idx = var.parse::<usize>().ok().and_then(|idx| strings.get(idx)).map(|s| s.trim()).unwrap_or("");
    let freetex_idx = overrides.iter().filter(|o| !o.item && o.freetex.is_some()).position(|o| std::ptr::eq(o, override_)).unwrap_or(0);
    let string_by_order = strings.get(freetex_idx).map(|s| s.trim()).unwrap_or("");
    let name = if !string_by_idx.is_empty() {
        string_by_idx
    } else if !string_by_order.is_empty() {
        string_by_order
    } else {
        return None;
    };
    let name = name.trim_matches('"');
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

#[cfg(test)]
mod tests {
    /// A route helper's text in letters its font lacks breaks its lines at `@` as OMSI's
    /// own text textures do (#1553): two bands of letters, one above the other, no `@`.
    #[test]
    fn a_long_helper_text_is_made_smaller_before_it_is_narrowed() {
        let tt = omsi_model::TextTexture { width: 64, height: 128, color: [255.0; 3], ..Default::default() };
        let ink_rows = |text: &str| -> usize {
            let img = helper_text_image(&tt, None, text).expect("drawn with the interface font");
            (0..128).filter(|&y| (0..64).any(|x| img.rgba[(y * 64 + x) * 4 + 3] > 0)).count()
        };
        // (#1696: the long name keeps letters of a readable shape: smaller, not only thinner)
        assert!(ink_rows("Łąka Łąka Łąka Łąka Łąka") < ink_rows("Łąka"));
    }

    #[test]
    fn helper_text_breaks_lines_at_the_at_sign() {
        let tt = omsi_model::TextTexture { width: 256, height: 64, color: [255.0; 3], ..Default::default() };
        let rows = |text: &str| -> Vec<bool> {
            let img = helper_text_image(&tt, None, text).expect("drawn with the interface font");
            (0..64).map(|y| (0..256).any(|x| img.rgba[(y * 256 + x) * 4 + 3] > 0)).collect()
        };
        let bands = |r: &[bool]| r.windows(2).filter(|w| !w[0] && w[1]).count() + usize::from(r[0]);
        assert_eq!(bands(&rows("Urząd")), 1);
        assert_eq!(bands(&rows("622@Urząd")), 2);
        assert_eq!(bands(&rows("622@Sosnowiec@Urząd")), 3);
    }

    use super::*;

    #[test]
    fn far_offset_forest_backdrops_keep_their_owner_tiles_visibility() {
        let rectangle = |x: f32, half_width: f32| MeshData {
            positions: vec![
                glam::Vec3::new(x, -half_width, -80.0),
                glam::Vec3::new(x, half_width, -80.0),
                glam::Vec3::new(x, half_width, 127.0),
                glam::Vec3::new(x, -half_width, 127.0),
            ],
            ..Default::default()
        };
        let loaded = 900.0;
        let forest = rectangle(-1109.0, 693.0);
        for heading in [0.0, 45.0, 225.0] {
            let xf = object_rotation([heading, 0.0, 0.0]);
            assert!(stand_in_mesh(&forest, &xf, DVec3::new(6242.0, 3348.0, 70.0), loaded),
                "the distant forest at heading {heading} must not cover a road outside its owner tiles");
        }
        // Ordinary large geometry beside its origin still uses the full view distance.
        assert!(!stand_in_mesh(&rectangle(0.0, 693.0), &Mat4::IDENTITY, DVec3::ZERO, loaded));
        // A small offset part is not enough to classify an object as a far backdrop.
        assert!(!stand_in_mesh(&rectangle(-1109.0, 5.0), &Mat4::IDENTITY, DVec3::ZERO, loaded));
        assert!(!stand_in_mesh(&forest, &Mat4::from_scale(glam::Vec3::splat(0.2)), DVec3::ZERO, loaded));
        // Keep the existing whole-city stand-in rule, even for a centred model.
        assert!(stand_in_mesh(&rectangle(0.0, 2000.0), &Mat4::IDENTITY, DVec3::ZERO, loaded));
    }

    #[test]
    fn a_mirror_keeps_the_glass_that_uses_most_of_the_picture() {
        let glass = |uv_area: f32, x: f32| MirrorGlass { centre: glam::Vec3::new(x, 0.0, 0.0), du: glam::Vec3::X, dv: glam::Vec3::Z, uv_area };
        let big = larger_glass(None, glass(0.4, 1.0)).unwrap();
        // a smaller mesh for the same mirror, loaded later, does not take its place
        assert_eq!(larger_glass(Some(big), glass(0.001, 2.0)).unwrap().centre.x, 1.0);
        // a larger one does
        assert_eq!(larger_glass(Some(big), glass(0.9, 3.0)).unwrap().centre.x, 3.0);
    }

    /// A route arrow's Cyrillic street name with the stock Latin-only "test" font: drawn
    /// with the interface font (it was an empty texture); a Latin one keeps the .oft.
    #[test]
    fn a_helper_text_the_font_cannot_draw_comes_from_the_interface_font() {
        use omsi_content::font::{Font, FontAtlas, FontChar};
        // a Latin font with its umlauts (`Ä` is `Д` in code page 1251, so one Cyrillic
        // letter alone is "in" the font)
        let chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyzÄÖÜäöüß"
            .chars()
            .enumerate()
            .map(|(k, ch)| FontChar { ch, x0: k as i32 * 4, x1: k as i32 * 4 + 3, y: 0 })
            .collect();
        let font = Font { path: PathBuf::new(), name: "test".into(), bitmap: String::new(), alpha: String::new(), height: 27, gap: 1, chars };
        let (aw, ah) = (256u32, 32u32);
        let atlas = FontAtlas::new(font, aw, ah, vec![255; (aw * ah * 4) as usize], vec![255; (aw * ah * 4) as usize]);
        let tt = omsi_model::TextTexture { variable: "0".into(), font: "test".into(), width: 128, height: 128, full_color: false, color: [255.0, 0.0, 0.0], orientation: 0, grid: 1 };
        assert!(helper_text_image(&tt, Some(&atlas), "Bauernhof").is_none());
        assert!(helper_text_image(&tt, Some(&atlas), "  ").is_none());
        for text in ["Улица Ленина", "Булевар ослобођења", "Δ"] {
            let img = helper_text_image(&tt, Some(&atlas), text).unwrap_or_else(|| panic!("{text}: drawn with the .oft"));
            assert_eq!((img.width, img.height), (128, 128));
            let ink: Vec<usize> = (0..128 * 128).filter(|&p| img.rgba[p * 4 + 3] > 128).collect();
            assert!(ink.len() > 40, "{text}: {} pixels", ink.len());
            assert!(ink.iter().all(|&p| img.rgba[p * 4..p * 4 + 3] == [255, 0, 0]), "{text}: in the texture's colour");
            // centred, and a long name narrowed into the texture
            let rows: Vec<usize> = ink.iter().map(|p| p / 128).collect();
            let (top, bottom) = (*rows.iter().min().unwrap(), *rows.iter().max().unwrap());
            assert!(top > 40 && bottom < 88, "{text}: rows {top}..{bottom}");
        }
        // no font at all (missing from the installation): still readable
        assert!(helper_text_image(&tt, None, "Bauernhof").is_some());
    }

    #[test]
    fn nightlight_follows_the_objects_darkness_threshold() {
        let day = DayKind { workday: true, ..Default::default() };
        let plain = InUse::new(0, 7);
        assert!(plain.lit(12.0 * 3600.0, day, 0.5));
        assert!(!plain.lit(12.0 * 3600.0, day, 0.65));
        let home = InUse::new(2, 7);
        assert!((0.3..=0.75).contains(&home.threshold));
        assert!(!home.lit(3.0 * 3600.0, day, 0.0));
    }

    #[test]
    fn vehicle_freetex_retries_paths_below_texture_component() {
        let root = std::env::temp_dir().join("openomsi-freetex-path-test");
        let vehicle_texture = root.join("Vehicles/TestBus/Texture");
        let wanted = vehicle_texture.join("mb_pmon/alerta_FalhaCambio.bmp");
        std::fs::create_dir_all(wanted.parent().unwrap()).unwrap();
        std::fs::write(&wanted, b"x").unwrap();
        let dirs = [vehicle_texture.as_path()];
        let found = find_vehicle_freetex(
            r"..\Texture\mb_pmon\alerta_FalhaCambio.bmp",
            &dirs,
        );
        assert_eq!(found.as_deref(), Some(wanted.as_path()));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_child_naming_a_crossing_light_reads_that_light() {
        // (RefreshAmpelParenting hands any child of a running crossing whose first string
        // is a light index that light's phase, lamp or not, #922)
        let crossings: hashbrown::HashSet<i64> = [7].into_iter().collect();
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(light_child_of(&crossings, Some(7), &s(&["2", "x"])), Some((7, 2)));
        assert_eq!(light_child_of(&crossings, Some(7), &s(&[" 0 "])), Some((7, 0)));
        assert_eq!(light_child_of(&crossings, Some(8), &s(&["2"])), None, "not a crossing with lights");
        assert_eq!(light_child_of(&crossings, None, &s(&["2"])), None);
        assert_eq!(light_child_of(&crossings, Some(7), &s(&["Hbf"])), None);
        assert_eq!(light_child_of(&crossings, Some(7), &s(&["-1"])), None);
    }

    #[test]
    fn traffic_light_program_requires_a_placed_signal() {
        let sco = SceneryObject::parse(&omsi_cfg::CfgFile::from_str("junction.sco", "[traffic_lights_group]\n72\n[traffic_light]\nMain\n[phase]\n0\n10\n[phase]\n6\n62\n"));
        assert!(!traffic_light_program_enabled(&sco, false));
        assert!(traffic_light_program_enabled(&sco, true));
        let gate = SceneryObject { is_traffic_light: true, ..sco };
        assert!(traffic_light_program_enabled(&gate, false));
        assert!(!traffic_light_program_enabled(&SceneryObject::default(), true));
    }

    fn freetex_test_look() -> Look {
        Look {
            alpha: AlphaMode::Opaque,
            color: [1.0; 4],
            emissive: [0.0; 3],
            unlit: false,
            diffuse: None,
            transmap: None,
            night: None,
            lightmap: None,
            envmap: None,
            extra: MaterialExtra::default(),
            dyn_tex: DynTex::default(),
        }
    }

    /// An LED panel's light map is one white pixel; a flipdot's is a picture with dark
    /// parts (the Krueger's `vmatrix_leer_LM.bmp`), and does not make an LED panel (#413).
    #[test]
    fn a_clear_texture_is_an_invisible_cover() {
        assert!(texture_is_clear(&vec![0u8; 64]));
        assert!(texture_is_clear(&vec![5u8; 64]));
        let mut pane = vec![0u8; 64];
        pane[10] = 62;
        assert!(!texture_is_clear(&pane));
        assert!(!texture_is_clear(&[]));
    }

    #[test]
    fn a_transmapped_slot_is_see_through_by_its_transmap_not_its_reflection_mask() {
        // the stock Golf 2: diffuse alpha 0 (reflection mask), transmap opaque (#928, #932)
        assert_eq!(coverage_texture(Some("Golf2_main1_T.tga"), "Golf2_main1.tga"), "Golf2_main1_T.tga");
        assert_eq!(coverage_texture(None, "glass.tga"), "glass.tga");
        assert_eq!(coverage_texture(Some("  "), "glass.tga"), "glass.tga");
        assert_eq!(coverage_texture(Some("\\S:2"), "led.tga"), "led.tga");
    }

    #[test]
    fn a_lamps_lenses_follow_its_alphascale_and_light_map_variables() {
        // three lenses on one mesh, each faded by its colour's variable and lit by its
        // light map (Cheongsan's signals, #826); a fourth slot switched by nothing
        let slots = LampSlots {
            count: 4,
            alpha: vec![(0, "Red".into()), (1, "Yellow".into()), (2, "Green".into())],
            light: vec![(0, "Red".into()), (1, "Yellow".into()), (2, "Green".into()), (3, "NoSuchVar".into())],
        };
        let state = |v: &str| standard_traffic_lamp(v, true, false, false, false);
        let (alpha, light) = slots.values(&state);
        assert_eq!(alpha, vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(light, vec![1.0, 0.0, 0.0, 1.0]);
        let state = |v: &str| standard_traffic_lamp(v, false, false, true, false);
        assert_eq!(slots.values(&state), (vec![0.0, 0.0, 1.0, 1.0], vec![0.0, 0.0, 1.0, 1.0]));
        // a light map without a variable is always on
        let plain = LampSlots { count: 1, alpha: vec![], light: vec![(0, String::new())] };
        assert_eq!(plain.values(&|_| Some(0.0)).1, vec![1.0]);
    }

    /// A season's snow textures are told by their folder, whatever its case (#879).
    #[test]
    fn snow_pictures_are_the_winter_snow_folders() {
        assert!(is_snow_picture(Path::new("/omsi/Texture/WinterSnow/gras.bmp")));
        assert!(is_snow_picture(Path::new("/omsi/Sceneryobjects/Buildings_RW1HH/texture/Wintersnow/wall.jpg")));
        assert!(!is_snow_picture(Path::new("/omsi/Texture/Winter/gras.bmp")));
        assert!(!is_snow_picture(Path::new("/omsi/Texture/WinterSnow_gras.bmp")));
    }

    #[test]
    fn only_a_white_light_map_makes_an_led_panel() {
        assert!(is_white_lightmap(&[255, 255, 255, 255]));
        assert!(is_white_lightmap(&[250, 248, 255, 0, 255, 255, 255, 255]));
        assert!(!is_white_lightmap(&[255, 255, 255, 255, 127, 127, 127, 255]));
        assert!(!is_white_lightmap(&[0, 0, 0, 255]));
        assert!(!is_white_lightmap(&[]));
    }

    #[test]
    #[ignore = "requires the installed SOR NB content in OMSI_TEST_CONTENT"]
    fn installed_sor_ois_retains_powered_freetex() {
        let root = PathBuf::from(
            std::env::var_os("OMSI_TEST_CONTENT")
                .expect("set OMSI_TEST_CONTENT to the OMSI content root"),
        );
        let model =
            omsi_model::Model::load(&root.join("Vehicles/SOR NB/model/1_2011.cfg")).unwrap();
        let definitions: Vec<_> = model
            .meshes
            .iter()
            .flat_map(|mesh| {
                let refs: Vec<_> = mesh.materials.iter().collect();
                free_texture_defs(&refs)
            })
            .filter(|(_, _, var)| var == "mypoldisplej")
            .collect();
        assert!(!definitions.is_empty());
        assert!(
            definitions
                .iter()
                .any(|(item, key, _)| *item && key.eq_ignore_ascii_case("cerna.bmp")),
            "{definitions:?}"
        );
        println!("SOR NB OIS: {definitions:?}");
    }

    #[test]
    fn powered_terminal_freetex_is_kept_and_replaces_its_black_nightmap() {
        // The vehicle's OIS declares the free texture in the powered item, not in
        // the base [matl]. The black key is also used as its self-lit night map.
        let model = omsi_model::Model::parse(&omsi_cfg::CfgFile::from_str(
            "model.cfg",
            concat!(
                "[mesh]\nterminal.o3d\n[matl]\nblack.bmp\n0\n",
                "[matl_change]\nblack.bmp\n0\npower\n[matl_item]\n",
                "[matl_nightmap]\nblack.bmp\n[matl_freetex]\nblack.bmp\nscreen\n",
            ),
        ));
        let defs: Vec<&MaterialDef> = model.meshes[0].materials.iter().collect();
        assert_eq!(
            free_texture_defs(&defs),
            vec![(true, "black.bmp".into(), "screen".into())]
        );
        let base = freetex_test_look();
        let mut powered = base.clone();
        powered.night = Some(10);
        let spec = SlotSpec {
            base,
            item: Some(powered),
            more: Vec::new(),
        };
        let changed = spec.with_freetex(Some(10), 20, true, true);
        assert_eq!(changed.base.diffuse, None); // unpowered remains black
        assert_eq!(changed.item.as_ref().unwrap().diffuse, Some(20));
        assert_eq!(changed.item.as_ref().unwrap().night, Some(20));
        assert_eq!(spec.item.as_ref().unwrap().night, Some(10)); // reusable template
    }

    #[test]
    fn freetex_preserves_other_stages_and_per_vehicle_script_textures() {
        let mut base = freetex_test_look();
        base.night = Some(10);
        base.lightmap = Some(11);
        base.transmap = Some((10, true));
        base.envmap = Some((12, 0.5));
        let mut item = base.clone();
        item.diffuse = Some(99); // a script texture is not the file being replaced
        let spec = SlotSpec {
            base,
            item: Some(item),
            more: Vec::new(),
        };
        let changed = spec.with_freetex(Some(10), 20, true, false);
        assert_eq!(changed.base.diffuse, Some(20));
        assert_eq!(changed.base.night, Some(20));
        assert_eq!(changed.base.lightmap, Some(11));
        assert_eq!(changed.base.transmap, Some((20, true)));
        assert_eq!(changed.base.envmap, Some((12, 0.5)));
        assert_eq!(changed.item.as_ref().unwrap().diffuse, Some(99));
        let missing_key = spec.with_freetex(None, 21, true, true);
        assert_eq!(missing_key.item.as_ref().unwrap().night, Some(10));
        assert_eq!(missing_key.item.as_ref().unwrap().diffuse, Some(99));
    }

    #[test]
    fn spline_batches_keep_materials_cells_shadows_and_long_segments_separate() {
        use omsi_scenery::sli::SplineTexture;
        let def = |file: &str| Spline {
            textures: vec![SplineTexture { file: file.into(), ..Default::default() }],
            ..Default::default()
        };
        let ty = Arc::new(SplineType { def: def("curb.dds"), dir: PathBuf::new(), surf: Vec::new() });
        let other = Arc::new(SplineType { def: def("other.dds"), dir: PathBuf::new(), surf: Vec::new() });
        let other_dir = Arc::new(SplineType { def: def("curb.dds"), dir: PathBuf::from("another_pack"), surf: Vec::new() });
        let mut tested = def("curb.dds");
        tested.textures[0].alpha = 1;
        let tested = Arc::new(SplineType { def: tested, dir: PathBuf::new(), surf: Vec::new() });
        let mut blended = def("curb.dds");
        blended.textures[0].alpha = 2;
        let blended = Arc::new(SplineType { def: blended, dir: PathBuf::new(), surf: Vec::new() });
        let mut compatible = def("curb.dds");
        compatible.path = PathBuf::from("another_profile.sli");
        compatible.textures.push(SplineTexture { file: "unused-grass.dds".into(), ..Default::default() });
        let compatible = Arc::new(SplineType { def: compatible, dir: PathBuf::new(), surf: Vec::new() });
        let mesh = |x: f32, length: f32| Arc::new(MeshData {
            positions: vec![glam::Vec3::new(x, 0.0, 0.0), glam::Vec3::new(x + length, 0.0, 0.0), glam::Vec3::new(x, 1.0, 0.0)],
            normals: vec![glam::Vec3::Z; 3],
            uvs: vec![glam::Vec2::ZERO; 3],
            indices: vec![0, 1, 2],
            ranges: vec![(0, 3, 0)],
            one_sided: true,
        });
        let batched = batch_static_splines(vec![
            (mesh(1.0, 2.0), ty.clone(), false, DVec3::ZERO),
            (mesh(5.0, 2.0), ty.clone(), false, DVec3::ZERO),
            (mesh(9.0, 2.0), compatible, false, DVec3::ZERO),
            (mesh(49.0, 2.0), ty.clone(), false, DVec3::ZERO),
            (mesh(1.0, 2.0), other, false, DVec3::ZERO),
            (mesh(1.0, 2.0), other_dir, false, DVec3::ZERO),
            (mesh(1.0, 2.0), tested, false, DVec3::ZERO),
            (mesh(1.0, 2.0), ty.clone(), true, DVec3::ZERO),
            (mesh(1.0, 100.0), ty.clone(), false, DVec3::ZERO),
            (mesh(1.0, 100.0), ty, false, DVec3::ZERO),
            (mesh(1.0, 2.0), blended.clone(), false, DVec3::new(1.0, 2.0, 3.0)),
            (mesh(5.0, 2.0), blended, false, DVec3::new(4.0, 5.0, 6.0)),
        ]);
        assert_eq!(batched.len(), 10);
        assert_eq!(batched[8].0.indices.len(), 3);
        assert_eq!(batched[9].0.indices.len(), 3);
        assert_eq!(batched[8].3, DVec3::new(1.0, 2.0, 3.0));
        assert_eq!(batched[9].3, DVec3::new(4.0, 5.0, 6.0));
        assert_eq!(batched[0].0.indices.len(), 9);
        assert_eq!(batched[0].0.ranges, vec![(0, 9, 0)]);
        assert_eq!(batched.iter().filter(|b| b.2).count(), 1);
        assert_eq!(batched.iter().map(|b| b.0.indices.len()).sum::<usize>(), 36);
    }

    #[test]
    fn ground_spline_batches_preserve_faces_and_uvs_with_local_bounds() {
        let mesh = |x: f32, z: f32, length: f32, one_sided: bool| Arc::new(MeshData {
            positions: vec![glam::Vec3::new(x, 0.0, z), glam::Vec3::new(x + length, 0.0, z), glam::Vec3::new(x, 1.0, z)],
            normals: vec![glam::Vec3::Z; 3],
            uvs: vec![glam::Vec2::new(x / 300.0, z / 300.0); 3],
            indices: vec![0, 1, 2],
            ranges: vec![(0, 3, 0)],
            one_sided,
        });
        let a = mesh(1.0, 0.0, 2.0, true);
        let b = mesh(5.0, 0.0, 2.0, true);
        let batched = batch_ground_splines(vec![
            a.clone(), b.clone(),
            mesh(49.0, 0.0, 2.0, true),
            mesh(1.0, 49.0, 2.0, true),
            mesh(1.0, 0.0, 2.0, false),
            mesh(1.0, 0.0, 100.0, true),
            mesh(1.0, 0.0, 100.0, true),
        ]);
        assert_eq!(batched.len(), 6);
        let combined = &batched[0];
        assert_eq!(combined.positions, [a.positions.clone(), b.positions.clone()].concat());
        assert_eq!(combined.normals, [a.normals.clone(), b.normals.clone()].concat());
        assert_eq!(combined.uvs, [a.uvs.clone(), b.uvs.clone()].concat());
        assert_eq!(combined.indices, vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(combined.ranges, vec![(0, 6, 0)]);
        assert_eq!(batched.iter().map(|m| m.indices.len()).sum::<usize>(), 21);
    }

    /// A light map covers the 3x3 tiles around its own: a lamp's pool in the middle of the
    /// picture is in the middle of the tile, one in a neighbour's third is left out, and the
    /// tile's edges take the texels a third of the way in.
    #[test]
    fn a_light_map_is_laid_on_its_middle_third() {
        let n = 12usize;
        let mut rgba = vec![0u8; n * n * 4];
        for y in 0..n {
            for x in 0..n {
                let i = (y * n + x) * 4;
                rgba[i] = (x * 20) as u8;
                rgba[i + 1] = (y * 20) as u8;
                rgba[i + 3] = 255;
            }
        }
        // a pool in the western neighbour, left out
        rgba[(6 * n + 1) * 4 + 2] = 255;
        let img = omsi_texture::Image { width: n as u32, height: n as u32, rgba, has_alpha: false };
        let own = own_tile_of_light_map(&img);
        assert_eq!((own.width, own.height), (n as u32, n as u32));
        let at = |x: usize, y: usize, c: usize| own.rgba[(y * n + x) * 4 + c] as f32;
        // output texel x samples x = 4 + (x + 0.5) / 3 - 0.5 of the source
        let expect = |x: usize| 20.0 * (4.0 + (x as f32 + 0.5) / 3.0 - 0.5);
        for x in [0, 5, 11] {
            assert!((at(x, 0, 0) - expect(x)).abs() <= 1.0, "column {x}: {} against {}", at(x, 0, 0), expect(x));
            assert!((at(0, x, 1) - expect(x)).abs() <= 1.0, "row {x}");
        }
        assert!((0..n * n).all(|i| own.rgba[i * 4 + 2] == 0));
    }

    /// A bus bay's lines made as a plain object (NCCR's `Parkbox(bus).sco`: a flat mesh 5 mm
    /// over a road at 10 cm) are paint and drawn over the road as the markings are (#1009);
    /// a kerb, a sign, a pole, a box under the road, a typed marking and an empty object
    /// are not.
    #[test]
    fn a_flat_plain_object_is_paint_on_the_road() {
        let sco = |text: &str| SceneryObject::parse(&omsi_cfg::CfgFile::from_str("x.sco", text));
        let mesh = |zs: &[f32]| {
            let positions: Vec<glam::Vec3> = zs.iter().enumerate().map(|(i, z)| glam::Vec3::new(i as f32, (i % 2) as f32 * 5.0, *z)).collect();
            let n = positions.len();
            (
                MeshData { positions, normals: vec![glam::Vec3::Z; n], uvs: vec![glam::Vec2::ZERO; n], ranges: vec![(0, 3, 0)], indices: vec![0, 1, 2], one_sided: true },
                Vec::new(),
                Vec::new(),
            )
        };
        let plain = sco("[mesh]\nParkbox.o3d\n");
        assert!(paint_at_foot(&plain, &[mesh(&[0.105, 0.105, 0.105, 0.105])]));
        // a slightly sunk plate, and lines raised a little over it
        assert!(paint_at_foot(&plain, &[mesh(&[-0.04, -0.03, -0.04]), mesh(&[0.0, 0.01, 0.0])]));
        assert!(paint_at_foot(&plain, &[mesh(&[0.2, 0.25, 0.22])]));
        // a kerb stone 30 cm high, a line plate standing upright (Busstop_Lineplate_Bridge,
        // -1 to 15 cm), a pole, a box under the road
        assert!(!paint_at_foot(&plain, &[mesh(&[0.0, 0.3, 0.1])]));
        assert!(!paint_at_foot(&plain, &[mesh(&[-0.01, 0.15, 0.07])]));
        assert!(!paint_at_foot(&plain, &[mesh(&[0.1, 0.1, 0.1]), mesh(&[0.0, 2.5, 0.0])]));
        assert!(!paint_at_foot(&plain, &[mesh(&[-0.3, -0.1, -0.2])]));
        assert!(!paint_at_foot(&plain, &[]));
        // the typed ones are drawn over the roads as surfaces already
        assert!(!paint_at_foot(&sco("[rendertype]\non_surface\n[mesh]\narrow.o3d\n"), &[mesh(&[0.11, 0.11, 0.11])]));
        assert!(!paint_at_foot(&sco("[surface]\n[mesh]\nplate.o3d\n"), &[mesh(&[0.0, 0.0, 0.0])]));
    }

    #[test]
    #[ignore = "requires a graphics adapter; checks road marking composition"]
    fn painted_spline_stays_above_roads_and_below_raised_surfaces() {
        let def = omsi_scenery::sli::Spline::parse(&omsi_cfg::CfgFile::from_str(
            "synthetic.sli",
            "[texture]\nasphalt.dds\n[matl_alpha]\n2\n[texture]\nline.dds\n[matl_alpha]\n2\n\
             [profile]\n0\n[profilepnt]\n-4.3\n0.1\n0\n1\n[profilepnt]\n4.3\n0.1\n1\n1\n\
             [profile]\n1\n[profilepnt]\n-0.11\n0.105\n0\n1\n[profilepnt]\n0.11\n0.105\n1\n1\n",
        ));
        for msaa in [1, 4] {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let mut renderer = pollster::block_on(Renderer::new_with(&instance, None,
                Some(wgpu::TextureFormat::Rgba8UnormSrgb),
                omsi_render::RenderOptions { msaa, ssao: false, shadow_size: 1024,
                    fxaa: false, render_scale: 1.0, ..Default::default() },
            )).expect("test renderer");
            let mut scene = renderer.new_scene();
            let plane = |half_width: f32| MeshData {
                positions: vec![glam::Vec3::new(-half_width, -1000.0, 0.0), glam::Vec3::new(half_width, -1000.0, 0.0),
                    glam::Vec3::new(half_width, 1000.0, 0.0), glam::Vec3::new(-half_width, 1000.0, 0.0)],
                normals: vec![glam::Vec3::Z; 4], uvs: vec![glam::Vec2::ZERO; 4],
                ranges: vec![(0, 6, 0)], indices: vec![0, 1, 2, 0, 2, 3], one_sided: false,
            };
            let mut combined = plane(1000.0);
            let stripe = plane(0.11);
            combined.positions.extend(stripe.positions.iter().map(|p| *p + glam::Vec3::Z * 0.005));
            combined.normals.extend(stripe.normals);
            combined.uvs.extend(stripe.uvs);
            combined.indices.extend(stripe.indices.iter().map(|i| i + 4));
            combined.ranges.push((6, 6, 1));
            let (road_data, line_data) = split_spline_paint(&combined, &def);
            let road_mesh = renderer.add_mesh(&mut scene, &road_data);
            let line_mesh = renderer.add_mesh(&mut scene, &line_data);
            let blue = renderer.add_material_extra(&mut scene, None, AlphaMode::Blend,
                [0.0, 0.0, 1.0, 1.0], true, None, None, None, None, [0.0; 3],
                MaterialExtra { no_z_write: true, ..Default::default() });
            let red = renderer.add_material_extra(&mut scene, None, AlphaMode::Blend,
                [1.0, 0.0, 0.0, 1.0], true, None, None, None, None, [0.0; 3],
                MaterialExtra { no_z_write: true, ..Default::default() });
            let green = renderer.add_material(&mut scene, None, AlphaMode::Opaque, [0.0, 1.0, 0.0, 1.0], true);
            let road = renderer.add_surface_instance(&mut scene, road_mesh, DVec3::ZERO, Mat4::IDENTITY, vec![blue]);
            scene.instances[road].render_phase = RenderPhase::Spline;
            scene.instances[road].blend_sort_origin = Some(DVec3::ZERO);
            let line = renderer.add_surface_instance(&mut scene, line_mesh, DVec3::ZERO, Mat4::IDENTITY, vec![blue, red]);
            scene.instances[line].render_phase = RenderPhase::BeforeNormal;
            scene.instances[line].blend_sort_origin = Some(DVec3::new(0.0, 0.01, 0.0));
            let bridge = renderer.add_surface_instance(&mut scene, road_mesh, DVec3::new(0.0, 0.0, 0.03), Mat4::IDENTITY, vec![green]);
            scene.instances[bridge].render_phase = RenderPhase::Surface;
            let lighting = omsi_render::Lighting { shadows: false, fog_density: 0.0, ..Default::default() };
            for y in [-20.0, 20.0] {
                let camera = omsi_render::Camera { position: DVec3::new(0.0, y, 2.0),
                    yaw: if y < 0.0 { 0.0 } else { 180.0 }, pitch: -(2.0f32 / 20.0).atan().to_degrees(),
                    roll: 0.0, fov_deg: 60.0, near: 0.1, far: 2000.0 };
                for raised in [false, true] {
                    renderer.set_params(&mut scene, bridge, &[], raised, &[]);
                    let rgba = renderer.render_to_image(&mut scene, 129, 129, &camera, &lighting).unwrap();
                    let pixel = &rgba[(64 * 129 + 64) * 4..][..3];
                    let wanted = if raised { 1 } else { 0 };
                    assert!(pixel[wanted] > 200 && pixel[2] < 20 && pixel[1 - wanted] < 20,
                        "msaa {msaa}, camera y {y}, raised {raised}: {pixel:?}");
                }
            }
        }
    }

    #[test]
    fn only_flat_blended_ground_strips_draw_after_the_roads() {
        use omsi_scenery::sli::{Spline, SplineProfile, SplineProfilePoint, SplineTexture};
        let line = Spline {
            textures: vec![SplineTexture { alpha: 2, ..Default::default() }],
            profiles: vec![SplineProfile { texture: 0, points: vec![
                SplineProfilePoint { x: -0.11, z: 0.1, ..Default::default() },
                SplineProfilePoint { x: 0.11, z: 0.1, ..Default::default() },
            ] }], ..Default::default()
        };
        assert_eq!(spline_render_phase(&line), RenderPhase::BeforeNormal);
        let mut road = line.clone();
        road.profiles[0].points[0].x = -4.3;
        road.profiles[0].points[1].x = 4.3;
        assert_eq!(spline_render_phase(&road), RenderPhase::Spline);
        let mut solid = line.clone();
        solid.textures[0].alpha = 0;
        assert_eq!(spline_render_phase(&solid), RenderPhase::Spline);
        let mut fence = line.clone();
        fence.profiles[0].points[1].z = 1.0;
        assert_eq!(spline_render_phase(&fence), RenderPhase::Spline);
        let mut wire = line.clone();
        for p in &mut wire.profiles[0].points { p.z = 5.0; }
        assert_eq!(spline_render_phase(&wire), RenderPhase::Spline);
        let mut mixed = line.clone();
        mixed.profiles.push(road.profiles[0].clone());
        assert_eq!(spline_render_phase(&mixed), RenderPhase::Spline);
        let mut traffic = line.clone();
        traffic.paths.push(Default::default());
        assert_eq!(spline_render_phase(&traffic), RenderPhase::Spline);
        let mut rail = line.clone();
        rail.rail_enh.push(Default::default());
        assert_eq!(spline_render_phase(&rail), RenderPhase::Spline);
        let mut unused = line;
        unused.textures.push(SplineTexture::default());
        assert_eq!(spline_render_phase(&unused), RenderPhase::BeforeNormal,
            "an unused opaque texture does not change a painted strip");
        assert_eq!(spline_render_phase(&Spline::default()), RenderPhase::Spline);
    }

    #[test]
    fn mixed_spline_keeps_asphalt_and_painted_ranges_apart() {
        let def = omsi_scenery::sli::Spline::parse(&omsi_cfg::CfgFile::from_str("mixed.sli",
            "[texture]\nasphalt.dds\n[matl_alpha]\n2\n[texture]\nline.dds\n[matl_alpha]\n2\n\
             [profile]\n0\n[profilepnt]\n-2.15\n0.1\n0\n1\n[profilepnt]\n2.15\n0.1\n1\n1\n\
             [profile]\n1\n[profilepnt]\n1.23\n0.105\n0\n1\n[profilepnt]\n1.45\n0.105\n1\n1\n"));
        let src = MeshData {
            positions: vec![glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Y,
                glam::Vec3::Z, glam::Vec3::ONE, glam::Vec3::new(1.0, 1.0, 0.0)],
            normals: vec![glam::Vec3::Z; 6], uvs: vec![glam::Vec2::ZERO; 6],
            indices: vec![0, 1, 2, 3, 4, 5], ranges: vec![(0, 3, 0), (3, 3, 1)], one_sided: true,
        };
        let (road, paint) = split_spline_paint(&src, &def);
        assert_eq!(road.ranges.iter().map(|r| r.2).collect::<Vec<_>>(), vec![0]);
        assert_eq!(paint.ranges.iter().map(|r| r.2).collect::<Vec<_>>(), vec![1]);
        assert_eq!(road.indices.len(), 3);
        assert_eq!(paint.indices.len(), 3);
        assert_eq!(paint.positions, src.positions[3..]);
        assert_eq!(paint.normals, src.normals[3..]);
        assert_eq!(paint.uvs, src.uvs[3..]);
        assert!(road.one_sided && paint.one_sided);
    }

    /// A `[variable_terrainlightmap]` tile's light map is baked as Omsi.exe bakes it: over the
    /// tile and its neighbours, north up, colour x min(1, (core / distance)^2) from the lamp's
    /// own height, added up, held at 1 and truncated, nothing beyond 15.96 cores.
    #[test]
    fn a_light_map_is_baked_from_the_lamps_round_the_tile() {
        let ts = tile_size();
        let origin = DVec3::new(10.0 * ts, -4.0 * ts, 0.0);
        let texel = 3.0 * ts / 256.0;
        let at = |img: &omsi_texture::Image, c: usize, r: usize| {
            let i = (r * 256 + c) * 4;
            [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2]]
        };
        // texel (c, r) lies at x = (3c / 256 - 1) ts, y = (2 - 3r / 256) ts
        let world = |c: usize, r: usize| (origin.x + c as f64 * texel - ts, origin.y + 2.0 * ts - r as f64 * texel);
        let lamp = |c: usize, r: usize, height: f32, color: [f32; 3], radius: f32| {
            let (x, y) = world(c, r);
            BakeLamp { x, y, height, color, radius }
        };
        // a lamp at the ground over a texel of the tile's own third: its full colour there
        let img = bake_light_map(&[lamp(100, 120, 0.0, [0.5, 0.25, 1.0], 3.0)], origin);
        assert_eq!(at(&img, 100, 120), [127, 63, 255]);
        // six metres up with a three metre core: a quarter of it under it (63.75 → 63)
        let img = bake_light_map(&[lamp(100, 120, 6.0, [1.0, 1.0, 1.0], 3.0)], origin);
        assert_eq!(at(&img, 100, 120), [63, 63, 63]);
        // north is the top row: a lamp in the northern neighbour lights a row above the
        // tile's third, and two of them add up to white, not beyond
        let img = bake_light_map(
            &[lamp(128, 40, 0.0, [0.75, 0.0, 0.0], 3.0), lamp(128, 40, 0.0, [0.75, 0.0, 0.0], 3.0)],
            origin,
        );
        assert_eq!(at(&img, 128, 40), [255, 0, 0]);
        assert_eq!(at(&img, 128, 128), [0, 0, 0]);
        // a bright lamp lights up to 15.96 cores along an axis and no further
        let core = 2.0;
        let near = (15.9 * core / texel as f32).floor() as usize;
        let img = bake_light_map(&[lamp(128, 128, 0.0, [100.0, 0.0, 0.0], core)], origin);
        assert!(at(&img, 128 + near, 128)[0] > 0);
        assert!(at(&img, 128, 128 - near)[0] > 0);
        assert_eq!(at(&img, 128 + near + 1, 128)[0], 0);
        assert_eq!(at(&img, 128, 128 - near - 1)[0], 0);
    }

    #[test]
    fn scenery_render_types_map_to_the_cpp_pass_order() {
        use omsi_scenery::sco::RenderType as ScoPhase;
        for (source, expected) in [
            (ScoPhase::PreSurface, RenderPhase::PreSurface),
            (ScoPhase::Surface, RenderPhase::Surface),
            (ScoPhase::OnSurface, RenderPhase::OnSurface),
            (ScoPhase::BeforeNormal, RenderPhase::BeforeNormal),
            (ScoPhase::Normal, RenderPhase::Normal),
            (ScoPhase::AfterNormal, RenderPhase::AfterNormal),
            (ScoPhase::AfterVehicles, RenderPhase::AfterVehicles),
        ] {
            assert_eq!(scenery_render_phase(source), expected);
        }
    }

    /// A film modelled as a copy of the floor's faces with a slot of its own is an overlay;
    /// a panel beside the floor, sharing one edge with it, is not.
    #[test]
    fn a_copy_of_another_slots_faces_is_an_overlay() {
        let v = glam::Vec3::new;
        let mesh = MeshData {
            positions: vec![v(0.0, 0.0, 0.0), v(4.0, 0.0, 0.0), v(4.0, 2.0, 0.0), v(0.0, 2.0, 0.0), v(4.0, 0.0, 1.0), v(0.0, 0.0, 1.0)],
            // slot 0 the floor, slot 1 the film over it (the same corners), slot 2 a wall
            // standing on the floor's front edge
            indices: vec![0, 1, 2, 0, 2, 3, 0, 1, 2, 0, 2, 3, 0, 1, 4, 0, 4, 5],
            ranges: vec![(0, 6, 0), (6, 6, 1), (12, 6, 2)],
            ..Default::default()
        };
        assert!(slot_overlays_another(&mesh, 1));
        assert!(!slot_overlays_another(&mesh, 2));
    }

    /// A path's trafficdensity rules per group: the last of each, and traffic on the lane
    /// while any group drives there (a path without a rule for the first group has its
    /// medium density).
    #[test]
    fn path_densities_per_group() {
        let rule = |path: i32, value: f64, extra: f64| omsi_map::MapRule {
            path_index: path,
            kind: "trafficdensity".into(),
            value,
            extra,
            ..Default::default()
        };
        let rules = [rule(0, 0.0, 0.0), rule(0, 1.0, 4.0), rule(0, 0.5, 4.0), rule(1, 2.0, 0.0)];
        assert_eq!(path_densities(&rules, 0), (0.5, vec![(0, 0.0), (4, 0.5)]));
        assert_eq!(path_densities(&rules, 1), (2.0, vec![(0, 2.0)]));
        assert_eq!(path_densities(&[rule(2, 0.3, 4.0)], 2), (1.0, vec![(4, 0.3)]));
        assert_eq!(path_densities(&[], 0), (1.0, vec![]));
    }

    /// A wire strung 5.5 m over its spline is no ground; a wall standing on it, or a
    /// catenary spline that has a track bed at the bottom, is.
    #[test]
    fn only_splines_all_overhead_leave_the_ground() {
        use omsi_scenery::sli::{Spline, SplineProfile, SplineProfilePoint};
        let prof = |zs: &[f32]| SplineProfile { texture: 0, points: zs.iter().map(|&z| SplineProfilePoint { x: z, z, ..Default::default() }).collect() };
        let def = |ps: Vec<SplineProfile>| Spline { profiles: ps, ..Default::default() };
        assert!(overhead_only(&def(vec![prof(&[5.5, 5.6]), prof(&[2.0, 2.0])])));
        assert!(!overhead_only(&def(vec![prof(&[0.0, 2.4])])));
        assert!(!overhead_only(&def(vec![prof(&[5.5, 5.6]), prof(&[-0.2, 0.0])])));
        assert!(!overhead_only(&def(vec![])));
    }

    /// The car park's first string picks the list; anything that is no number is list 0.
    #[test]
    fn a_car_park_picks_its_parklist_by_its_first_string() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(parklist_index(&s(&[])), 0);
        assert_eq!(parklist_index(&s(&["2", "x"])), 2);
        assert_eq!(parklist_index(&s(&[" 1 "])), 1);
        assert_eq!(parklist_index(&s(&["Taxi"])), 0);
    }

    /// A `[terrainmapping]` slot (TH_Wald's Fels01: rock in slot 0, grass top in slot 1)
    /// leaves the object's own mesh and comes back in tile space, where the ground under the
    /// placed object is: turned a quarter, 10 m into a tile whose corner is at 300/600.
    #[test]
    fn terrain_mapped_slots_split_off_in_tile_space() {
        let mut src = MeshData::default();
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 2.0], [3.0, 0.0, 2.0], [0.0, 3.0, 2.0]] {
            src.positions.push(glam::Vec3::from_array(p));
            src.normals.push(glam::Vec3::Z);
            src.uvs.push(glam::Vec2::ZERO);
        }
        src.indices = vec![0, 1, 2, 3, 4, 5];
        src.ranges = vec![(0, 3, 0), (3, 3, 1)];
        src.one_sided = true;
        let origin = DVec3::new(300.0, 600.0, 0.0);
        let pos = DVec3::new(310.0, 620.0, 5.0);
        let xf = Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let (rest, ground) = split_terrain_mapped(&src, &[1], pos, xf, origin);
        assert_eq!(rest.ranges, vec![(0, 3, 0)]);
        assert_eq!(ground.ranges, vec![(0, 3, 0)]);
        assert!(ground.one_sided);
        assert_eq!(ground.positions, src.positions[3..6].to_vec());
        // (3, 0) turned a quarter is (0, 3): 10/23 m into the tile
        let uv = ground.uvs[1] * tile_size() as f32;
        assert!((uv - glam::Vec2::new(10.0, 23.0)).length() < 1e-3, "{uv:?}");
        let (_, none) = split_terrain_mapped(&src, &[2], pos, xf, origin);
        assert!(none.is_empty() && none.ranges.is_empty());
    }

    /// The Spandau neon lamp (Streetobjects_RUE/neonlight_M_whip_S.sco) declares its glow
    /// after the far mesh of `[LOD] 0`: it still glows, near or far.
    #[test]
    fn lights_of_lower_lods_count() {
        let text = "[LOD]\n0.15\n[mesh]\nnear.o3d\n[LOD]\n0\n[mesh]\nfar.o3d\n[light_enh_2]\n-2.965\n0\n7.170\n-0.3817\n0\n-1.5344\n-1.5344\n0\n0.3817\n0\n1\n230\n230\n255\n2.0\n120\n200\nNightlightA\n0.8\n0.5\n1\n1\n0.2\nlichteffekt1.bmp\n";
        let model = Model::parse(&omsi_cfg::CfgFile::from_str("lamp.sco", text));
        assert_eq!(model.lods.len(), 2);
        assert!(model.lod_meshes(0)[0].light_enh_2.is_empty());
        let asked = Mutex::new(Vec::new());
        let coronas = model_lights_faded(
            &model,
            &|_| Mat4::IDENTITY,
            DVec3::new(100.0, 200.0, 30.0),
            &|v| {
                asked.lock().push(v.to_string());
                1.0
            },
            &[],
        );
        // the glow, its star (effect bit 1), the halo round it in fog and (its cone flag set)
        // the light cone it throws there
        assert_eq!(coronas.len(), 4);
        assert!(!coronas[0].beam && !coronas[0].halo && coronas[0].flags & 8 == 0);
        assert!(coronas[1].flags & 8 != 0 && coronas[1].rotating == 2);
        assert!(coronas[2].halo && coronas[3].beam);
        assert_eq!(asked.into_inner(), vec!["NightlightA".to_string()]);
        assert!((coronas[0].position - DVec3::new(97.035, 200.0, 37.17)).length() < 1e-3);
        assert!(
            coronas[0].direction.z < -0.9,
            "points down: {:?}",
            coronas[0].direction
        );
    }

    #[test]
    fn small_instrument_lights_keep_their_small_size() {
        let text = "[mesh]\ndash.o3d\n[light_enh]\n0\n0\n0\n255\n0\n0\n0.01\nspeedo_warn\n0\n";
        let model = Model::parse(&omsi_cfg::CfgFile::from_str("bus.cfg", text));
        let coronas = model_lights_faded(&model, &|_| Mat4::IDENTITY, DVec3::ZERO, &|_| 1.0, &[]);
        // (effect 0: the glow, and the halo it has in fog)
        let glows: Vec<_> = coronas.iter().filter(|c| !c.halo).collect();
        assert_eq!(glows.len(), 1);
        assert!((glows[0].size - 0.005).abs() < 1e-6);
    }

    /// The MAN NL's stop request lamp (`model_EN92.cfg`): a `[light_enh]` is drawn as
    /// Omsi.exe draws a `[light_enh_2]` - its own bitmap, 5 cm towards the viewer, its
    /// brightness factor and its effect bits (3: a star, no halo in fog) - not as a bare
    /// glow in its dome (#1159).
    #[test]
    fn a_light_enh_has_its_bitmap_z_offset_factor_and_effects() {
        let dir = std::env::temp_dir().join("openomsi-light-enh-test");
        std::fs::create_dir_all(dir.join("MAN_NL_NG/model")).unwrap();
        std::fs::create_dir_all(dir.join("MAN_NL_NG/Texture")).unwrap();
        std::fs::write(dir.join("MAN_NL_NG/Texture/D92_Haltewunsch.bmp"), b"BM").unwrap();
        let lamp = |factor: &str, effect: &str| {
            format!("[mesh]\npanel.o3d\n[light_enh]\n-0.635\n5.435\n1.288\n255\n150\n0\n0.05\nhaltewunschlampe_all\n{factor}\n0.05\n{effect}\n0.05\nD92_Haltewunsch.bmp\n")
        };
        let path = dir.join("MAN_NL_NG/model/model_EN92.cfg");
        let model = Model::parse(&omsi_cfg::CfgFile::from_str(&path, &lamp("1", "3")));
        let coronas = model_lights_faded(&model, &|_| Mat4::IDENTITY, DVec3::ZERO, &|_| 1.0, &[]);
        assert_eq!(coronas.len(), 2, "the glow and its star, no fog halo: {coronas:?}");
        let (glow, star) = (&coronas[0], &coronas[1]);
        assert_ne!(glow.texture, crate::lights::glow_texture_id(), "the lamp's own picture");
        assert_eq!(glow.texture, crate::lights::corona_texture_id(&dir.join("MAN_NL_NG/model"), "D92_Haltewunsch.bmp"));
        assert!((glow.z_offset - 0.05).abs() < 1e-6 && glow.rotating == 2);
        assert!((glow.size - 0.025).abs() < 1e-6 && (glow.brightness - 1.0).abs() < 1e-6);
        assert!(star.flags & 8 != 0 && (star.size - 0.0625).abs() < 1e-6);
        // the factor scales the lamp (OMSI: variable times factor), effect 0 has the halo
        let model = Model::parse(&omsi_cfg::CfgFile::from_str(&path, &lamp("0.5", "0")));
        let coronas = model_lights_faded(&model, &|_| Mat4::IDENTITY, DVec3::ZERO, &|_| 1.0, &[]);
        assert_eq!(coronas.len(), 2);
        assert!((coronas[0].brightness - 0.5).abs() < 1e-6 && !coronas[0].halo);
        assert!(coronas[1].halo);
        // switched off: nothing
        assert!(model_lights_faded(&model, &|_| Mat4::IDENTITY, DVec3::ZERO, &|_| 0.0, &[]).is_empty());
    }

    /// A bridge deck high over the ground under it (London Bridge over the Thames) is
    /// draped by its `[crossing_heightdeformation]` as one lying on the ground is: Omsi.exe
    /// does it in the object's own frame, wherever the map puts the object (#961).
    #[test]
    fn a_crossing_far_over_the_ground_is_draped_by_its_height_field() {
        let dir = std::env::temp_dir().join(format!("openomsi-deck-warp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("global.cfg"), "[name]\nDeck\n").unwrap();
        std::fs::write(dir.join("deck.sco"), "[surface]\n[mesh]\ndeck.x\n[crossing_heightdeformation]\ndeck_def.x\n").unwrap();
        // a 10 m square deck at 0, and its field 1 m higher at the far end (y up in a .x)
        let quad = |name: &str, far: f32| {
            format!("xof 0303txt 0032\nMesh {name} {{\n 4;\n 0;0;0;,\n 10;0;0;,\n 10;{far};10;,\n 0;{far};10;;\n 2;\n 3;0,2,1;,\n 3;0,3,2;;\n}}\n")
        };
        std::fs::write(dir.join("deck.x"), quad("deck", 0.0)).unwrap();
        std::fs::write(dir.join("deck_def.x"), quad("field", 1.0)).unwrap();
        let world = World::open(&dir, &dir.join("global.cfg"), 20261001).unwrap();
        let ot = world.object_type("deck.sco").expect("the deck loads");
        assert!(ot.deform.is_some());
        let staged = |z: f64| StagedTile {
            tx: 0,
            ty: 0,
            origin: DVec3::ZERO,
            path: dir.join("tile_0_0.map"),
            base_terrain: Terrain::flat(),
            align: Vec::new(),
            hole_rims: Vec::new(),
            water: None,
            bakes_light_map: false,
            splines: Vec::new(),
            meshes: Mutex::new(Some(Vec::new())),
            drive: Vec::new(),
            lanes: Mutex::new(Vec::new()),
            street_points: Vec::new(),
            objects: vec![StagedObject {
                ot: ot.clone(),
                id: 1,
                place: Placement::Ground { x: 100.0, y: 100.0, z, rot: [0.0; 3] },
                rules: Vec::new(),
                extra: Vec::new(),
                lamp_parent: None,
                parked: false,
                map_object: true,
                instance: 0,
                key: 1,
            }],
            anchors: Vec::new(),
            counts: LoadStats::default(),
            resolved: std::sync::OnceLock::new(),
        };
        // on the ground and 20 m over it, alike
        for z in [0.0, 20.0] {
            let src: HashMap<(i32, i32), Arc<StagedTile>> = [((0, 0), Arc::new(staged(z)))].into_iter().collect();
            let warped = world.warp_crossings(&src[&(0, 0)], &src);
            let deck = warped.get(&0).unwrap_or_else(|| panic!("the deck {z} m over the ground is draped"));
            let top = deck[0].positions.iter().map(|p| p.z).fold(f32::MIN, f32::max);
            assert!((top - 1.0).abs() < 1e-3, "its far end is raised by the field: {top}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_retexture_using_the_originals_model_shows_its_own_textures() {
        let dir = std::env::temp_dir().join(format!("openomsi-retexture-{}", std::process::id()));
        for d in ["orig/model", "orig/texture", "retex/model", "retex/texture"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("global.cfg"), "[name]\nRetexture\n").unwrap();
        let plate = "xof 0303txt 0032\nMesh plate {\n 3;\n 0;0;0;,\n 1;0;0;,\n 0;1;0;;\n 1;\n 3;0,1,2;;\n MeshMaterialList {\n  1;\n  1;\n  0;;\n  Material { 1;1;1;1;; 0; 0;0;0;; 0;0;0;; TextureFilename { \"plate.bmp\"; } }\n }\n}\n";
        std::fs::write(dir.join("orig/model/plate.x"), plate).unwrap();
        std::fs::write(dir.join("retex/model/plate.x"), plate).unwrap();
        std::fs::write(dir.join("orig/model/plate.cfg"), "[mesh]\nplate.x\n").unwrap();
        std::fs::write(dir.join("orig/texture/plate.bmp"), b"original").unwrap();
        std::fs::write(dir.join("retex/texture/plate.bmp"), b"retexture").unwrap();
        std::fs::write(dir.join("orig/orig.sco"), "[model]\nmodel\\plate.cfg\n").unwrap();
        std::fs::write(dir.join("retex/retex.sco"), "[model]\n..\\orig\\model\\plate.cfg\n").unwrap();
        let world = World::open(&dir, &dir.join("global.cfg"), 20261003).unwrap();
        let found = |sco: &str| {
            let ot = world.object_type(sco).expect("the object loads");
            let dirs = ot.texture_dirs(&dir);
            let dirs: Vec<&Path> = dirs.iter().map(|d| d.as_path()).collect();
            omsi_texture::find_texture(&ot.meshes[0].1[0].texture, &dirs).and_then(|p| std::fs::read(p).ok())
        };
        assert_eq!(found("orig/orig.sco").as_deref(), Some(&b"original"[..]));
        // the copy's own `texture` folder, as Omsi.exe takes it, not the model file's
        assert_eq!(found("retex/retex.sco").as_deref(), Some(&b"retexture"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scripted_lamp_channels_override_stock_phases_and_switch_led_materials() {
        // Numazu's pedestrian script shows green at phase 5, where stock car lamps
        // are red/yellow; it also switches the green mesh via its Yellow blink output.
        let stock_green = standard_traffic_lamp("green", true, true, false, false);
        let green = traffic_lamp_value("green", Some(1.0), stock_green);
        assert!(change_picks_item(green));
        let stock_yellow = standard_traffic_lamp("yellow", false, true, false, false);
        let blink_off = traffic_lamp_value("yellow", Some(0.0), stock_yellow);
        assert!(!change_picks_item(blink_off));
        assert!(change_picks_item(traffic_lamp_value("yellow", Some(1.0), stock_yellow)));
        // A missing script keeps the safe stock fallback and unknown channels stay off.
        assert_eq!(traffic_lamp_value("green", None, stock_green), 0.0);
        assert_eq!(traffic_lamp_value("custom_channel", None, None), 0.0);
        assert_eq!(traffic_lamp_value("1", None, None), 1.0);
    }

    #[test]
    fn standard_traffic_lamps_are_state_driven() {
        assert_eq!(standard_traffic_lamp("red", true, false, false, false), Some(1.0));
        assert_eq!(standard_traffic_lamp("YELLOW", false, true, false, false), Some(1.0));
        assert_eq!(standard_traffic_lamp("green", false, false, true, false), Some(1.0));
        assert_eq!(standard_traffic_lamp("red", false, true, false, false), Some(0.0));
        assert_eq!(standard_traffic_lamp("custom_channel", true, true, true, false), None);
    }

}

#[cfg(test)]
#[path = "scene/object_path_tests.rs"]
mod object_path_tests;

#[cfg(test)]
#[path = "scene/terrain_mapping_tests.rs"]
mod terrain_mapping_tests;

#[cfg(test)]
#[path = "scene/crossing_deformation_tests.rs"]
mod crossing_deformation_tests;

#[cfg(test)]
mod material_tests {
    use super::*;

    #[test]
    fn null_texture_names() {
        assert!(is_null_texture("null.bmp"));
        assert!(is_null_texture(" NULL.BMP "));
        assert!(is_null_texture("texture\\null.tga"));
        assert!(is_null_texture(""));
        assert!(!is_null_texture("D_Matrix.bmp"));
        assert!(!is_null_texture("nullschild.bmp"));
    }

    #[test]
    fn d3d_material_colours() {
        // a Blender export: grey diffuse with alpha 0, a bogus specular, no emissive
        let m = omsi_o3d::Material {
            diffuse: [0.64, 0.64, 0.64, 0.0],
            specular: [255.0, 255.0, 355.0],
            emissive: [0.0; 3],
            specular_power: 96.0,
            texture: "int_glass.tga".into(),
        };
        let (color, emissive, specular, ambient) = d3d_material(&m, None, true);
        // textured: the texture's alpha alone counts
        assert_eq!(color, [0.64, 0.64, 0.64, 1.0]);
        assert_eq!(emissive, [0.0; 3]);
        assert_eq!(specular, [1.0, 1.0, 1.0, 96.0]);
        // Omsi.exe's o3d slot: a white ambient, whatever the diffuse colour (0x7c62f8)
        assert_eq!(ambient, [1.0; 3]);
        // untextured: the material's alpha
        assert_eq!(d3d_material(&m, None, false).0[3], 0.0);
        // no specular colour, no highlight whatever the power
        let plain = omsi_o3d::Material {
            specular_power: 25.0,
            ..Default::default()
        };
        assert_eq!(d3d_material(&plain, None, true).2[3], 0.0);
        // a lit display: emissive white
        let lcd = omsi_o3d::Material {
            emissive: [1.0; 3],
            ..Default::default()
        };
        assert_eq!(d3d_material(&lcd, None, true).1, [1.0; 3]);
        // [matl_allcolor] replaces the o3d material (the stock lower-deck lighting item)
        let all = [
            1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.24, 0.23, 0.2, 0.0,
        ];
        let (c, e, s, _) = d3d_material(&m, Some(all), true);
        assert_eq!(c, [1.0; 4]);
        // [matl_allcolor]'s own ambient
        let mut dim = all;
        dim[4..7].copy_from_slice(&[0.2, 0.3, 0.4]);
        assert_eq!(d3d_material(&m, Some(dim), true).3, [0.2, 0.3, 0.4]);
        assert_eq!(e, [0.24, 0.23, 0.2]);
        assert_eq!(s[3], 0.0);
    }

    /// A second `[matl]` of the same slot with `[matl_alpha] 1` makes the slot
    /// alpha-tested; a later `[matl_alpha] 0` makes it opaque again,
    /// and a later `[matl]` without one keeps the mode.
    #[test]
    fn later_matl_of_the_same_slot_sets_its_alpha() {
        let mats = [omsi_o3d::Material { texture: "Chain.dds".into(), ..Default::default() }];
        let def = |alpha: Option<i32>| MaterialDef {
            texture: "chain.dds".into(),
            alpha: alpha.unwrap_or(0),
            alpha_set: alpha.is_some(),
            ..Default::default()
        };
        assert_eq!(material_alpha(&mats, 0, &[def(None), def(Some(1))]), AlphaMode::Test);
        assert_eq!(material_alpha(&mats, 0, &[def(Some(1)), def(None)]), AlphaMode::Test);
        assert_eq!(material_alpha(&mats, 0, &[def(Some(2)), def(Some(0))]), AlphaMode::Opaque);
        assert_eq!(material_alpha(&mats, 0, &[def(None), def(None)]), AlphaMode::Opaque);
    }

    #[test]
    fn transmap_mask_does_not_make_opaque_body_blend() {
        let mats = [omsi_o3d::Material {
            texture: "body.tga".into(),
            ..Default::default()
        }];
        let defs = [
            MaterialDef {
                texture: "body.tga".into(),
                index: 0,
                alpha: 0,
                transmap: Some("body_mask.tga".into()),
                ..Default::default()
            },
            MaterialDef {
                texture: "body.tga".into(),
                index: 0,
                alpha: 0,
                alphascale: Some("Rain_Window_Front_Wetness".into()),
                ..Default::default()
            },
        ];
        let alpha = material_alpha(&mats, 0, &defs);
        assert_eq!(alpha, AlphaMode::Opaque);
        assert_eq!(Renderer::clamp_slot_alpha(0.35, alpha, false), 1.0);
    }

    #[test]
    fn blended_body_alpha_repair_does_not_touch_glass() {
        assert!(is_vehicle_body_material(
            "12m/wagenkasten_embl_eev.o3d",
            "01white_FL.tga",
            true,
            false,
            false,
            true
        ));
        assert!(is_vehicle_body_material(
            "A21_EEV/body.o3d",
            "a21_body.png",
            true,
            false,
            false,
            true
        ));
        assert!(is_vehicle_body_material(
            "Exterior/unnamed_shell.o3d",
            "paint.tga",
            true,
            false,
            false,
            true
        ));
        assert!(!is_vehicle_body_material(
            "A21/windows.o3d",
            "a21_body_windows.png",
            true,
            false,
            false,
            false
        ));
        assert!(!is_vehicle_body_material(
            "Exterior/unnamed_window.o3d",
            "glass.tga",
            true,
            false,
            false,
            true
        ));
        assert!(!is_vehicle_body_material(
            "12m/wagenkasten.o3d",
            "01white_FL.tga",
            true,
            true,
            false,
            true
        ));
        assert!(!is_vehicle_body_material(
            "12m/wagenkasten.o3d",
            "01white_FL.tga",
            true,
            false,
            true,
            true
        ));
    }

    /// The ICU400 controller's screen layer: a script texture as its transmap declares one.
    #[test]
    fn script_transmap_is_declared() {
        let text = "[mesh]\nscreen.o3d\n\n[matl]\nScreen.dds\n0\n[matl_transmap]\n\\S:1\n[alphascale]\nsignController_alphaScale\n[matl_alpha]\n2\n\n[matl]\nPlain.dds\n0\n";
        let m = omsi_model::Model::parse(&omsi_cfg::CfgFile::from_str("model.cfg", text));
        let mats = &m.meshes[0].materials;
        let screen = mats.iter().find(|d| d.texture == "Screen.dds").unwrap();
        let plain = mats.iter().find(|d| d.texture == "Plain.dds").unwrap();
        assert_eq!(screen.transmap.as_deref(), Some("\\S:1"));
        assert!(material_extra(&[screen], None, None, [0.0; 4]).transmap_declared);
        assert!(!material_extra(&[plain], None, None, [0.0; 4]).transmap_declared);
    }

    #[test]
    fn material_extra_from_commands() {
        let glass = MaterialDef {
            texture: "wischwasser.tga".into(),
            alpha: 2,
            no_z_write: true,
            no_z_check: true,
            ..Default::default()
        };
        let decal = MaterialDef {
            texture: "bw_jul_mod5.bmp".into(),
            z_bias: 16,
            ..Default::default()
        };
        let e = material_extra(
            &[&glass, &decal],
            Some(7),
            Some((3, 0.1)),
            [0.2, 0.2, 0.2, 10.0],
        );
        assert!(e.no_z_write && !e.no_z_check);
        assert_eq!(e.z_bias, 16);
        assert_eq!(e.env_mask, Some(7));
        assert_eq!(e.specular, [0.2, 0.2, 0.2, 10.0]);
        assert_eq!(e.bump, Some((3, 0.1)));
        assert_eq!(
            material_extra(&[], None, None, [0.0; 4]),
            MaterialExtra::default()
        );
        // a factor of 0 moves nothing: no bump map to sample
        assert_eq!(
            material_extra(&[], None, Some((3, 0.0)), [0.0; 4]).bump,
            None
        );
        // [matl_texadress_border]: the colour in bytes, as 0..1
        let roller = MaterialDef {
            texture: "rlb_512.tga".into(),
            tex_address: omsi_model::TexAddress::Border,
            border_color: [255.0, 255.0, 255.0, 0.0],
            ..Default::default()
        };
        assert_eq!(
            material_extra(&[&roller], None, None, [0.0; 4]).border,
            Some([1.0, 1.0, 1.0, 0.0])
        );
        let clamped = MaterialDef {
            tex_address: omsi_model::TexAddress::Clamp,
            ..roller.clone()
        };
        assert_eq!(material_extra(&[&roller, &clamped], None, None, [0.0; 4]).border, None);
        // the slot's last addressing command decides how its textures repeat
        use omsi_render::TexAddressing as R;
        let mirror = MaterialDef { tex_address: omsi_model::TexAddress::Mirror, ..roller.clone() };
        let once = MaterialDef { tex_address: omsi_model::TexAddress::MirrorOnce, ..roller.clone() };
        let plain = MaterialDef::default();
        assert_eq!(tex_addressing([&plain].into_iter()), R::Wrap);
        assert_eq!(tex_addressing([&clamped, &mirror, &plain].into_iter()), R::Mirror);
        assert_eq!(tex_addressing([&mirror, &once].into_iter()), R::MirrorOnce);
        assert_eq!(tex_addressing([&once, &roller].into_iter()), R::Clamp);
    }

    #[test]
    fn scenery_freetex_name_resolution() {
        let ov1 = MaterialDef {
            freetex: Some(("placeholder.bmp".into(), "Textur".into())),
            ..Default::default()
        };
        let ov2 = MaterialDef {
            freetex: Some(("placeholder2.bmp".into(), "Textur2".into())),
            ..Default::default()
        };
        let overrides = vec![ov1.clone(), ov2.clone()];

        // 1. Script variable takes precedence when available
        let mut prog = omsi_script::Program::default();
        prog.declare_str_var("Textur");
        let script = omsi_sim::scenery::SceneryInstance::new(
            Arc::new(prog),
            &[],
            omsi_sim::SimClock::default(),
            &["from_script.bmp".into()],
        );
        let from_strings = vec!["from_strings.bmp".to_string()];
        let name = resolve_scenery_freetex_name("Textur", &ov1, &overrides, Some(&script), None, &from_strings);
        assert_eq!(name, Some("from_script.bmp"));

        // 2. Fallback to strings by explicit numeric index (e.g. var = "1")
        let strings = vec!["zero.bmp".to_string(), "\"quoted_one.bmp\"".to_string()];
        let name = resolve_scenery_freetex_name("1", &ov1, &overrides, None, None, &strings);
        assert_eq!(name, Some("quoted_one.bmp"));

        // 3. Fallback to strings by freetex declaration order
        let name_first = resolve_scenery_freetex_name("Textur", &overrides[0], &overrides, None, None, &strings);
        assert_eq!(name_first, Some("zero.bmp"));
        let name_second = resolve_scenery_freetex_name("Textur2", &overrides[1], &overrides, None, None, &strings);
        assert_eq!(name_second, Some("quoted_one.bmp"));

        // 4. Returns None when no matching string exists
        let name_empty = resolve_scenery_freetex_name("Missing", &ov1, &overrides, None, None, &[]);
        assert_eq!(name_empty, None);

        // 5. A string variable the object declares owns the slot: an empty one means the
        //    slot has no picture of its own, never another variable's placement string.
        //    (A bus stop sign's route number stood in a `[matl_freetex]` slot this way,
        //    #1756.)
        let mut prog2 = omsi_script::Program::default();
        prog2.declare_str_var("Textur");
        let empty_script = omsi_sim::scenery::SceneryInstance::new(
            Arc::new(prog2),
            &[],
            omsi_sim::SimClock::default(),
            &[String::new()],
        );
        assert_eq!(
            resolve_scenery_freetex_name("Textur", &ov1, &overrides, Some(&empty_script), None, &strings),
            None
        );
        // a variable the object does not declare keeps the positional fallback
        assert_eq!(
            resolve_scenery_freetex_name("Textur2", &ov1, &overrides, Some(&empty_script), None, &strings),
            Some("zero.bmp")
        );
    }
}

/// The whole map for the navigator (see [`World::navigation_map`]).
pub struct NavigationMap {
    pub lanes: Vec<Lane>,
    /// Asphalt footprints used to corroborate editor-only driving paths. They are evidence
    /// for roads, not streets to draw: a paved yard or median has no road centre line.
    pub road_surfaces: Vec<(Vec<DVec3>, f32)>,
    /// Every placed object's position by id (bus stops beyond the loaded tiles).
    pub places: HashMap<i64, DVec3>,
    /// Street name signs: where, the object's heading and the name on it.
    pub signs: Vec<(DVec3, f64, String)>,
}

/// Whether an asset name describes a road surface. Match whole filename tokens so objects
/// such as `StreetLight.sli` do not become roads just because their name contains "street".
fn road_surface_name(file: &str) -> bool {
    let base = file.replace('\\', "/").rsplit('/').next().unwrap_or("").to_ascii_lowercase();
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(&base);
    let tokens: Vec<&str> = stem.split(|c: char| !c.is_ascii_alphanumeric() && c != 'ß').filter(|t| !t.is_empty()).collect();
    let excludes = ["light", "lamp", "sign", "schild", "rail", "track", "gleis", "tram", "strab", "wire", "mast", "wall", "fence", "leitplanke", "gehweg", "side", "bord", "pavement", "fahrrad", "radweg", "cycle", "parking", "parkplatz", "gruen", "gras"];
    if tokens.iter().any(|t| excludes.iter().any(|x| t.contains(x))) {
        return false;
    }
    stem.starts_with("str_")
        || tokens.iter().any(|t| {
            ["str", "strasse", "straße", "road", "roads", "street", "streets", "fahrbahn", "pflaster", "kopfstein", "cobble"].contains(t)
                || t.starts_with("asph")
        })
}

/// The horizontal road surfaces actually drawn by a pathless spline: lateral bounds and
/// height. A texture merely listed in the file is not evidence of a road, and the origin
/// need not be in the middle of the surface. Keep medians and pavements out of its width.
fn road_sections(file: &str, def: &omsi_scenery::sli::Spline) -> Vec<(f32, f32, f32)> {
    let name = file.to_ascii_lowercase();
    if def.only_editor || ["gehweg", "radweg", "fahrrad", "tram", "strab", "gleis", "rail", "parking"].iter().any(|s| name.contains(s)) || def.paths.iter().any(|p| p.kind == 2) {
        return Vec::new();
    }
    let mut sections = Vec::new();
    for profile in &def.profiles {
        let road = def.textures.get(profile.texture).map(|t| road_surface_name(&t.file)).unwrap_or_else(|| def.textures.is_empty() && road_surface_name(file));
        if !road { continue; }
        for pair in profile.points.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let (lo, hi) = (a.x.min(b.x), a.x.max(b.x));
            if lo.is_finite() && hi.is_finite() && a.z.is_finite() && b.z.is_finite() && hi - lo > 0.1 && (a.z - b.z).abs() <= (hi - lo) * 0.15 {
                sections.push((lo, hi, (a.z + b.z) * 0.5));
            }
        }
    }
    sections.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f32, f32, f32)> = Vec::new();
    for (lo, hi, z) in sections {
        if let Some(last) = merged.last_mut().filter(|s| lo <= s.1 + 0.2 && (z - s.2).abs() < 0.2) {
            last.1 = last.1.max(hi);
        } else {
            merged.push((lo, hi, z));
        }
    }
    merged.retain(|(lo, hi, _)| hi - lo >= 4.0);
    merged
}

#[cfg(test)]
mod navigation_road_tests {
    use super::*;

    fn road_spline() -> omsi_scenery::sli::Spline {
        use omsi_scenery::sli::{SplineProfile, SplineProfilePoint};
        omsi_scenery::sli::Spline {
            profiles: vec![SplineProfile { points: vec![SplineProfilePoint { x: -4.0, ..Default::default() }, SplineProfilePoint { x: 4.0, ..Default::default() }], ..Default::default() }],
            // This editor selection width is deliberately much wider than the mesh.
            height_profiles: vec![omsi_scenery::sli::HeightProfile { x0: -40.0, x1: 40.0, ..Default::default() }],
            ..Default::default()
        }
    }

    #[test]
    fn road_candidates_use_drawn_width_and_ignore_nonroad_assets() {
        let def = road_spline();
        assert!(road_sections("Splines/StreetLight.sli", &def).is_empty());
        assert!(road_surface_name("Splines/Fahrbahn.sli"));
        assert!(!road_surface_name("Splines/Straßenbahn.sli"));
        assert_eq!(road_sections("Splines/Str_2spur_6m.sli", &def), vec![(-4.0, 4.0, 0.0)]);
        let mut only_editor = def.clone();
        only_editor.only_editor = true;
        assert!(road_sections("Splines/Str_2spur_6m.sli", &only_editor).is_empty());
        let mut asphalt = def;
        asphalt.textures.push(omsi_scenery::sli::SplineTexture { file: "Texture/asphalt_rough.bmp".into(), ..Default::default() });
        assert_eq!(road_sections("Splines/CustomCurve.sli", &asphalt), vec![(-4.0, 4.0, 0.0)]);
        assert!(road_sections("Splines/BS_Gehweg_Asphalt01_MD_04m.sli", &asphalt).is_empty());
    }

    #[test]
    fn road_surface_bounds_keep_offsets_medians_and_unused_textures() {
        use omsi_scenery::sli::{Spline, SplineProfile, SplineProfilePoint, SplineTexture};
        let profile = |texture, lo, hi| SplineProfile { texture, points: vec![SplineProfilePoint { x: lo, ..Default::default() }, SplineProfilePoint { x: hi, ..Default::default() }] };
        let mut def = Spline {
            textures: vec![SplineTexture { file: "str_asphdrk.bmp".into(), ..Default::default() }, SplineTexture { file: "str_side1.bmp".into(), ..Default::default() }, SplineTexture { file: "gras1.bmp".into(), ..Default::default() }],
            profiles: vec![profile(1, -25.0, -18.0), profile(0, -18.0, -15.0), profile(0, -15.0, -12.0), profile(2, -12.0, 12.0), profile(0, 12.0, 18.0), profile(1, 18.0, 25.0)],
            ..Default::default()
        };
        assert_eq!(road_sections("Custom/divided.sli", &def), vec![(-18.0, -12.0, 0.0), (12.0, 18.0, 0.0)]);
        def.profiles = vec![profile(1, -25.0, 25.0)];
        assert!(road_sections("Custom/surface.sli", &def).is_empty(), "unused asphalt texture must not turn pavement into a road");
        def.profiles = vec![profile(0, -4.0, 4.0)];
        def.paths.push(omsi_scenery::PathDef { kind: 2, ..Default::default() });
        assert!(road_sections("Custom/track.sli", &def).is_empty());
    }
}

/// A street name sign object (the stock Verkehrszeichen_MC `StreetSign_*`, and the
/// German `Strassenschild`/`StrSchild` names add-on maps use).
fn is_street_sign(file: &str) -> bool {
    let f = file.to_ascii_lowercase().replace(['\\', '_', ' ', '-'], "");
    let name = f.rsplit('/').next().unwrap_or(&f);
    ["streetsign", "streetname", "strschild", "strassenschild", "straßenschild", "strassenname", "roadsignname", "roadname"].iter().any(|k| name.contains(k))
}

/// The height of a `[crossing_heightdeformation]` field at (x, y) of its object's frame.
fn field_height(m: &MeshData, x: f32, y: f32) -> Option<f32> {
    let mut best: Option<f32> = None;
    for t in m.indices.chunks_exact(3) {
        let (a, b, c) = (
            m.positions[t[0] as usize],
            m.positions[t[1] as usize],
            m.positions[t[2] as usize],
        );
        let det = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
        if det.abs() < 1e-9 {
            continue;
        }
        let l1 = ((b.x - a.x) * (y - a.y) - (x - a.x) * (b.y - a.y)) / det;
        let l2 = ((x - a.x) * (c.y - a.y) - (c.x - a.x) * (y - a.y)) / det;
        let l0 = 1.0 - l1 - l2;
        if l0 >= -1e-4 && l1 >= -1e-4 && l2 >= -1e-4 {
            let h = l0 * a.z + l2 * b.z + l1 * c.z;
            best = Some(best.map_or(h, |o: f32| o.max(h)));
        }
    }
    best
}


/// The name of a texture's night copy: the same file in a `night` folder beside it.
/// Whether an object's texture has its night copy (see [`night_texture_name`]). A texture
/// named by its full path - a parked car's paint, resolved in its scheme's folder - has it
/// there or not at all: the texture lookup takes such a path for one of its author's
/// machine and falls back to the bare file name, which found the day picture itself, and
/// every parked car of a paint scheme was lit by its own paint at night, glowing in the
/// dark street.
fn night_texture_exists(rel: &str, dirs: &[&Path]) -> bool {
    // (`night_texture_name` writes backslashes: "\\Users\\...", "C:\\...")
    let norm = rel.trim().replace('\\', "/");
    if norm.starts_with('/') || norm.as_bytes().get(1) == Some(&b':') {
        return omsi_cfg::vfs::is_file(Path::new(&norm));
    }
    omsi_texture::find_texture(rel, dirs).is_some()
}

fn night_texture_name(texture: &str) -> String {
    let name = texture.trim().replace('/', "\\");
    match name.rsplit_once('\\') {
        Some((dir, file)) => format!("{dir}\\night\\{file}"),
        None => format!("night\\{name}"),
    }
}

/// OMSI_CHECK_SPLINES: every spline's two ends, its neighbours in the chain and its file.
pub(crate) static SPLINE_ENDS: std::sync::LazyLock<Mutex<HashMap<i64, (DVec3, DVec3, i64, i64, String)>>> = std::sync::LazyLock::new(Default::default);
