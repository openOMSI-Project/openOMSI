//! Object, spline and tile staging types: what a tile is read into before it is placed.
use super::*;

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
    /// Whether any loaded mesh carries a `[mouseevent]`.
    pub has_mouse_events: bool,
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
    /// Per `[maplight]`: whether it sits inside its own pole fixture (worked out on first
    /// use, see `place::embedded_pole_light`): its lamp's shadow map leaves the fixture out.
    pub embedded_lights: std::sync::OnceLock<Vec<bool>>,
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
pub(super) fn larger_glass(old: Option<MirrorGlass>, new: MirrorGlass) -> Option<MirrorGlass> {
    Some(old.filter(|o| o.uv_area >= new.uv_area).unwrap_or(new))
}

/// `reflexionN.bmp`: the texture drawn by reflection camera N of the vehicle.
pub(super) fn mirror_index(name: &str) -> Option<usize> {
    let n = name.trim().to_ascii_lowercase();
    let rest = n.strip_prefix("reflexion")?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// A light map that is white all over (the LED panels' `vmatrix_leer_led_LM.png`, one white
/// pixel): the surface is all its own light. A flipdot panel carries the same `\S:n` mask,
/// but its light map is a picture of the lamps over it (`vmatrix_leer_LM.bmp`).
pub(super) fn is_white_lightmap(rgba: &[u8]) -> bool {
    !rgba.is_empty() && rgba.chunks_exact(4).all(|p| p[0] >= 242 && p[1] >= 242 && p[2] >= 242)
}

/// Whether the picture at `path` has no transparent texel at all (no alpha channel, or one
/// that is 255 throughout), read once per file. A `[matl_alpha] 2` slot that names one has
/// nothing to blend by, so its fragments are drawn opaque (`World::type_gpu`): the GG2 signs'
/// pictures (`directions\temp_1.png`, `terminal_east_2.tga`) are like that - an alpha channel
/// that never goes below 255 - and left a no-write blend, the surface phases drawn after them
/// ate the signs' blue and the gantries' bare panels.
pub(super) fn picture_is_opaque(path: &Path) -> bool {
    static MEMO: std::sync::OnceLock<Mutex<HashMap<PathBuf, bool>>> = std::sync::OnceLock::new();
    let memo = MEMO.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = memo.lock().get(path) {
        return *v;
    }
    let v = omsi_texture::decode_file(path)
        .map(|i| !i.has_alpha || i.rgba.chunks_exact(4).all(|p| p[3] == 255))
        .unwrap_or(false);
    memo.lock().insert(path.to_path_buf(), v);
    v
}

/// [`is_white_lightmap`] of the light map `name` (found in `dirs`, read once per file);
/// `None` when the file is not there.
pub(super) fn lightmap_is_white(name: &str, dirs: &[&Path]) -> Option<bool> {
    // (global, not a field of `World`: a memo of what the file holds, the same whatever map
    // is open, so a new `World` reads nothing twice)
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

/// Where a ray lands on a mesh of a scenery object with a `[mouseevent]`: see
/// [`World::scenery_object_hit`].
#[derive(Clone, Debug)]
pub struct SceneryHit {
    /// Distance (m) along the ray.
    pub t: f32,
    pub map_id: i64,
    #[allow(dead_code)]
    pub mesh_index: usize,
    /// The event name from the mesh's `[mouseevent]`.
    pub event: String,
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
    pub departures: std::collections::HashMap<String, Vec<omsi_sim::vehicle_api::Departure>>,
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
    /// The `[surface]` id of each of them (see [`surface_id`]).
    pub surface: Vec<u8>,
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
pub(super) enum Placement {
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
pub(super) struct StagedObject {
    pub(super) ot: Arc<ObjectType>,
    pub(super) id: i64,
    pub(super) place: Placement,
    pub(super) rules: Vec<omsi_map::MapRule>,
    /// The record's trailing lines: text strings, tree parameters, a lamp's light index,
    /// a bus stop's name.
    pub(super) extra: Vec<String>,
    /// The crossing a traffic light lamp belongs to (`[varparent]`, else what it hangs on).
    pub(super) lamp_parent: Option<i64>,
    /// A car put on a parking space (the traffic steers round it).
    pub(super) parked: bool,
    /// The record is an `[object]` (its position goes into `object_positions`).
    pub(super) map_object: bool,
    /// The instance of a spline-attachment row this object represents.
    pub(super) instance: usize,
    /// What the collisions call this object: its map id, or for an object of a spline
    /// attachment row (which all share the row's id) a key of its own, so that one post of
    /// a row of `[crashmode_pole]` bollards falls alone.
    pub(super) key: i64,
}

/// The collision key of object `index` of spline attachment row `row` in tile (tx, ty):
/// above every map id (2^53 and up), and the same on every load.
pub(super) fn row_object_key(tx: i32, ty: i32, row: i64, index: usize) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in [tx as i64 as u64, ty as i64 as u64, row as u64, index as u64] {
        h = (h ^ v).wrapping_mul(0x0000_0100_0000_01b3);
        h ^= h >> 29;
    }
    ((1u64 << 53) | (h & ((1u64 << 53) - 1))) as i64
}

/// A spline of a staged tile as the rasters see it.
pub(super) struct StagedSpline {
    /// Positions (relative to the tile origin) and triangles only.
    pub(super) shape: MeshData,
    pub(super) ty: Arc<SplineType>,
    /// World bounds (x0, y0, x1, y1).
    pub(super) bounds: [f64; 4],
    /// It carries a road or a footway (a railway embankment or a bridge deck does not).
    pub(super) drivable: bool,
    /// Every profile of it is blended (`[matl_alpha] 2`): a layer laid over the ground or a
    /// road, not a surface of its own (see `prepare_surfaces`).
    pub(super) overlay: bool,
    /// It is ground (see `SPLINE_OVERHEAD`): it goes into the surface raster, cutting the
    /// terrain where that comes up through it. Overhead wires do not.
    pub(super) cuts_terrain: bool,
    /// It stands clear of the ground all along (see `SPLINE_SHADOW_CLEARANCE`): it casts a
    /// sun shadow.
    pub(super) casts_shadow: bool,
    /// Start point used by OMSI's far-to-near blend sort for spline surfaces.
    pub(super) sort_origin: DVec3,
}

/// How far a spline has to stand clear of the ground under it, everywhere, before it casts
/// a sun shadow: a bridge deck, a viaduct or an elevated railway does, a road lying on the
/// terrain does not - a caster in one plane with what it falls on paints dark patches into
/// it (the sun shadow's bias is 6 cm). Splines are surfaces and cast nothing otherwise.
pub(super) const SPLINE_SHADOW_CLEARANCE: f32 = 0.75;
/// A spline whose profiles all hang this far (m) over its line - wires, catenaries, a
/// canopy - is no ground surface: it neither cuts the terrain nor carries anything.
pub(super) const SPLINE_OVERHEAD: f32 = 2.0;

/// Whether a plain object (no `[rendertype]`, no `[surface]`) is only paint at its foot:
/// every point of every mesh between 5 cm under and 25 cm over its origin, all of them
/// within 5 cm of height (a plate standing upright - a line plate on a bridge rail, a stop's
/// name plate - is a sign, not paint; the mesh pivots are for animations, not for the
/// points). Omsi.exe draws such a marking after the roads at its own height, a few
/// millimetres over the road it was made for - a bus bay's lines (`Korean Road Object\
/// Parkinglot\Parkbox(bus).sco`) lie 5 mm over a road at 10 cm. The roads here are pulled
/// towards the eye by their depth bias instead, and an object lying that close over one
/// went under it: a depot's parking bays were all gone (#1009).
pub(super) fn paint_at_foot(sco: &SceneryObject, meshes: &[(MeshData, Vec<omsi_o3d::Material>, Vec<MaterialDef>)]) -> bool {
    if sco.render_type.is_ground_layer() || sco.surface || meshes.is_empty() || meshes.iter().any(|(m, _, _)| m.positions.is_empty()) {
        return false;
    }
    let (lo, hi) = meshes.iter().flat_map(|(m, _, _)| m.positions.iter()).fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.z), hi.max(p.z)));
    lo >= -0.05 && hi <= 0.25 && hi - lo <= 0.05
}

pub(super) fn spline_paint_slots(def: &omsi_scenery::sli::Spline) -> Vec<usize> {
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

pub(super) fn spline_render_phase(def: &omsi_scenery::sli::Spline) -> RenderPhase {
    let slots = spline_paint_slots(def);
    if !def.profiles.is_empty() && def.profiles.iter().all(|p| slots.contains(&p.texture)) {
        RenderPhase::BeforeNormal
    } else {
        RenderPhase::Spline
    }
}

pub(super) fn split_spline_paint(src: &MeshData, def: &omsi_scenery::sli::Spline) -> (MeshData, MeshData) {
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

pub(super) fn scenery_render_phase(kind: omsi_scenery::sco::RenderType) -> RenderPhase {
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
pub(super) fn overhead_only(def: &omsi_scenery::sli::Spline) -> bool {
    !def.profiles.is_empty() && def.profiles.iter().all(|p| !p.points.is_empty() && p.points.iter().all(|q| q.z >= SPLINE_OVERHEAD))
}

/// Which `parklist_p` a car park draws from: its first map string, as a number (Omsi.exe
/// sub_79c8b8 - `StrToInt`, 0 when that fails or there is none). 0 is `parklist_p.txt`,
/// n is `parklist_p_n.txt`.
pub(super) fn parklist_index(strings: &[String]) -> usize {
    strings.first().and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(0)
}

/// A tile read and tessellated, its objects typed but not yet standing on the ground. Kept
/// (by [`World::prepare_tiles`]) while a loaded tile or one on its way depends on it.
pub struct StagedTile {
    pub(super) tx: i32,
    pub(super) ty: i32,
    pub(super) origin: DVec3,
    pub(super) path: PathBuf,
    /// The terrain as the tile file has it, before roads and crossings pulled it about.
    pub(super) base_terrain: Terrain,
    /// `[spline_terrain_align]` splines: (index into `splines`, reach in metres).
    pub(super) align: Vec<(usize, f32)>,
    /// The hole boundaries in world space, including the profile's authored height.
    pub(super) hole_rims: Vec<Vec<DVec3>>,
    pub(super) water: Option<[f32; 4]>,
    /// `[variable_terrainlightmap]`: the tile's light map is baked from the lamps around it
    /// (see [`bake_light_map`]), not read from its `.map.LM.bmp`.
    pub(super) bakes_light_map: bool,
    pub(super) splines: Vec<StagedSpline>,
    /// The whole spline meshes, in the order of `splines`, until the tile is placed.
    pub(super) meshes: Mutex<Option<Vec<Arc<MeshData>>>>,
    /// The `[heightprofile]` surfaces of the tile's splines (local to `origin`) with their
    /// world bounds and `.surf` maps: what the wheels roll on.
    pub(super) drive: Vec<(MeshData, [f64; 4], Option<omsi_geometry::SurfFaces>)>,
    /// Lanes, taken when the tile is loaded for the first time.
    pub(super) lanes: Mutex<Vec<Lane>>,
    /// The street lanes' points of the tile's splines, kept for good (what an object's box
    /// is checked against: a road through it makes it no wall).
    pub(super) street_points: Vec<DVec3>,
    pub(super) objects: Vec<StagedObject>,
    /// The spline attachment rows `[attachObj]` records can hang on: (row id, where its first
    /// object stands, the row's own type - a car park row's, not its car's).
    pub(super) anchors: Vec<(i64, Pose, Arc<ObjectType>)>,
    /// What reading the tile counted (missing types, empty car parks, rows, attachments).
    pub(super) counts: LoadStats,
    pub(super) resolved: std::sync::OnceLock<Arc<Resolved>>,
}

/// A staged tile's final ground and where its objects finally stand.
pub(super) struct Resolved {
    pub(super) terrain: Arc<Terrain>,
    /// Crossings warped onto the ground: object index → its own meshes.
    pub(super) warped: HashMap<usize, Arc<Vec<MeshData>>>,
    /// By object index; None for an attachment without its parent or attachment point.
    pub(super) poses: Vec<Option<Pose>>,
    pub(super) unattached: usize,
    pub(super) aligned_points: usize,
    pub(super) biggest: Option<(f32, f64, f64)>,
    pub(super) deformed: bool,
}

/// The tile and its eight neighbours.
pub(super) const NEIGHBOURHOOD: [(i32, i32); 9] = [
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
pub(super) const SOURCE_MARGIN: f64 = 150.0;

/// The tiles of a map and what each depends on.
pub struct TileLayout {
    pub paths: HashMap<(i32, i32), PathBuf>,
    /// Tile → the tiles whose splines, crossings or ground its own final ground, crossings
    /// and cut are made from: its neighbours, and tiles with splines reaching it from
    /// further away. The same for a whole-map load as for streaming.
    pub(super) sources: HashMap<(i32, i32), Vec<(i32, i32)>>,
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
    pub(super) ot: Arc<ObjectType>,
    pub(super) pos: DVec3,
    pub(super) xf: Mat4,
    /// Traffic light lamp: (crossing id, light index).
    pub(super) lamp: Option<(i64, usize, bool)>,
    pub(super) map_id: i64,
    /// Collision key (see [`StagedObject::key`]).
    pub(super) key: i64,
    pub(super) controller: Option<usize>,
    pub(super) strings: Vec<String>,
    pub(super) warped: Option<Arc<Vec<MeshData>>>,
    /// `[varparent]` of the record.
    pub(super) var_parent: Option<i64>,
    /// A car on a `[carpark_p]` space.
    pub(super) parked: bool,
    /// A tile's own `[object]` record standing on the ground: the object editor may move it.
    pub(super) editable: bool,
    pub(super) script: Option<omsi_sim::scenery::SceneryInstance>,
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
    pub(super) terrain: Option<MeshData>,
    /// The terrain's exposed sides, drawn separately so the hole mask cannot cut them.
    pub(super) hole_walls: MeshData,
    /// Ground painting: for every `[groundtex]` layer above the first that is painted on
    /// this tile, its index and the alpha mask the editor's brush left behind (as read;
    /// [`World::cut_terrain`] turns them into `paint`).
    pub(super) paint_masks: Vec<(usize, Image)>,
    /// The painted layers ready for the GPU: index, mask (with the roads' cut taken out)
    /// and the painted fraction of the tile.
    pub(super) paint: Vec<(usize, TextureData, f32)>,
    /// The same brush masks without the hole cut, for the exposed terrain sides.
    pub(super) wall_paint: Vec<(usize, TextureData)>,
    /// `tile.map.water`: the height of the tile's water surface at its four corners.
    pub(super) water: Option<[f32; 4]>,
    /// Spline meshes are local to the tile origin.
    /// Spline meshes (local to the tile origin), their type and whether they cast a shadow.
    pub(super) splines: Vec<(Arc<MeshData>, Arc<SplineType>, bool, DVec3)>,
    /// Terrain-mapped spline faces pooled across types within spatial cells.
    pub(super) ground_splines: Vec<Arc<MeshData>>,
    pub(super) objects: Vec<PlacedObject>,
    /// (type, texture, position, height, width, heading)
    pub(super) trees: Vec<(Arc<ObjectType>, String, DVec3, f64, f64, f64)>,
    pub(super) origin: DVec3,
    pub(super) light_map: Option<TextureData>,
    /// The cut the roads make into the ground (alpha 0 = cut), in tile space.
    pub(super) cut: Option<TextureData>,
    /// Textures prepared on the worker, by file (shared by the tiles of a batch).
    pub(super) images: Arc<HashMap<PathBuf, Arc<TextureData>>>,
}

/// A prepared tile on its way to the GPU (see [`World::begin_upload`]).
pub struct PendingUpload {
    pub prepared: Prepared,
    pub(super) textures: Vec<PathBuf>,
    pub(super) types: Vec<Arc<ObjectType>>,
    pub(super) tg: TileGpu,
    pub(super) placing: Placing,
}

/// How far [`World::place_step`] got with a tile, and what it made so far.
#[derive(Default)]
pub(super) struct Placing {
    /// 0: the ground, 1: splines, 2: trees, 3: objects, 4: done.
    pub(super) phase: u8,
    /// The next spline or tree of the phase.
    pub(super) next: usize,
    pub(super) ground_next: usize,
    pub(super) night_slots: Vec<(usize, usize, MaterialId, MaterialId)>,
    pub(super) night_modes: Vec<NightMode>,
    pub(super) light_objects: Vec<LightObject>,
    pub(super) poles: Vec<i64>,
    pub(super) splines: usize,
    pub(super) trees: usize,
    pub(super) objects: usize,
    /// Seconds per phase (OMSI_PROFILE).
    pub(super) secs: [f64; 4],
    /// `[terrainmapping]` uses the first [groundtex], without the roads' cut (which
    /// would punch holes into a traffic island). Painted terrain layers belong to the
    /// ground itself and must not be projected onto a spline verge or object.
    pub(super) terrain_mapping_mat: Option<MaterialId>,
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

pub(super) struct TexEntry {
    pub(super) id: TextureId,
    pub(super) alpha: bool,
    pub(super) users: usize,
    /// Texels of the uploaded image (mip levels not counted).
    pub(super) texels: u64,
    /// Bytes on the GPU (all levels, as they are now).
    pub(super) bytes: u64,
    pub(super) format: omsi_texture::PixelFormat,
    /// Finest mip levels let go while the textures are over their budget.
    pub(super) dropped: u32,
}

/// An object type on the GPU.
pub(super) struct TypeGpu {
    /// Keeps the type (and so the pointer that keys this entry) alive while cached.
    pub(super) ot: Arc<ObjectType>,
    pub(super) meshes: Vec<(MeshId, Vec<MaterialId>)>,
    /// `[matl_change]` variants: (mesh index, slot, base, item, variable).
    pub(super) variants: Vec<(usize, usize, MaterialId, MaterialId, String)>,
    /// Dynamic texture overrides, made only for combinations that placed scripts use.
    /// Rows align with LOD 0 meshes and material slots; each entry holds (base, item).
    pub(super) dynamic_texture_variants: HashMap<Vec<usize>, Vec<Vec<Option<(MaterialId, MaterialId)>>>>,
    /// Lower LODs: (min size, max size, meshes).
    pub(super) lods: Vec<(f32, f32, Vec<(MeshId, Vec<MaterialId>)>)>,
    pub(super) materials: Vec<MaterialId>,
    pub(super) textures: Vec<PathBuf>,
    pub(super) users: usize,
    /// Some texture has a night copy in the `night` folder beside it (lit windows).
    pub(super) auto_night: bool,
    /// The screen sizes from and up to which the first level (`meshes`) is drawn (see
    /// `lods`).
    pub(super) lod0_lo: f32,
    pub(super) lod0_max: f32,
    /// Material slots whose texture carries `[terrainmapping]`: (level: 0 the first,
    /// k the k-th of `lods`, mesh index in that level, slot).
    pub(super) terrain_slots: Vec<(usize, usize, usize)>,
    /// The meshes without those slots, made once for all placements: ((level, mesh), id).
    pub(super) terrain_rest: Vec<((usize, usize), MeshId)>,
}

pub(super) struct SplineGpu {
    pub(super) _st: Arc<SplineType>,
    pub(super) materials: Vec<MaterialId>,
    pub(super) textures: Vec<PathBuf>,
    pub(super) users: usize,
    /// Texture slots with `[terrainmapping]` (the grass verges of Berlin-Spandau's
    /// `Splines/Ruede`): drawn with the ground of the tile, like such an object's slots.
    pub(super) terrain: Vec<usize>,
}

pub(super) struct TreeGpu {
    pub(super) material: MaterialId,
    pub(super) texture: Option<PathBuf>,
    pub(super) users: usize,
}

/// Materials every tile shares (never freed; made again for another season's textures).
pub(super) struct GroundGpu {
    /// The season's texture folder they were made with (`omsi_texture::season_folder`).
    pub(super) season: Option<String>,
    pub(super) ground_id: Option<TextureId>,
    pub(super) ground_mat: MaterialId,
    pub(super) plain_terrain_mat: MaterialId,
    pub(super) ground_detail: Option<(TextureId, f32)>,
    pub(super) ground_repeats: f32,
    /// Whether the map's base ground layer (`ground_id`) carries `[moisture]`/`[puddles]`.
    pub(super) ground_wet: f32,
    pub(super) water_mat: MaterialId,
    pub(super) tree_mesh: MeshId,
}

/// Freed scene slots, the lowest handed out first: new resources fill the front of the
/// scene's arrays, so that after a big unload their tail can be cut off (`compact_slots`).
#[derive(Default)]
pub(super) struct FreeList(pub(super) std::collections::BinaryHeap<std::cmp::Reverse<usize>>);

impl FreeList {
    pub(super) fn push(&mut self, id: usize) {
        self.0.push(std::cmp::Reverse(id));
    }

    pub(super) fn pop(&mut self) -> Option<usize> {
        self.0.pop().map(|r| r.0)
    }

    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    /// Drop the ids from `len` on (the part of the array that is cut off).
    pub(super) fn keep_below(&mut self, len: usize) {
        self.0.retain(|r| r.0 < len);
    }

    /// The free ids at the end of an array of `len` slots: the new length.
    pub(super) fn free_tail(&self, len: usize) -> usize {
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
