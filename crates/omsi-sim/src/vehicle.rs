//! A road vehicle: its type (definition + compiled scripts + model) and instances.

use crate::particles::ParticleSet;
use crate::anim::{pivot_from_mesh, MeshAnimator};
use crate::host::VehicleHost;
use crate::physics::{Controls, VehiclePhysics};
use anyhow::{Context, Result};
use glam::{DVec3, Mat4, Quat, Vec3, Vec4};
use hashbrown::HashMap;
use omsi_geometry::{mesh_from_o3d, MeshData};
use omsi_model::{MaterialDef, Model};
use omsi_script::{compile, CompileInput, Program, State, Vm};
use omsi_vehicle::Vehicle;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Built-in variables every road vehicle has (`program/varlist_roadvehicle.txt` + generated).
pub fn builtin_vars(root: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        match omsi_cfg::CfgFile::read(root.join("program/varlist_roadvehicle.txt")) {
            Ok(f) => f
                .lines
                .iter()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect(),
            Err(_) => Vec::new(),
        };
    for a in 0..8 {
        for side in ["L", "R"] {
            for pre in [
                "Wheel_Rotation_",
                "Wheel_RotationSpeed_",
                "Axle_Steering_",
                "Axle_Suspension_",
                "Axle_Springfactor_",
                "Axle_Brakeforce_",
                "Axle_SurfaceID_",
            ] {
                v.push(format!("{pre}{a}_{side}"));
            }
        }
        v.push(format!("PAX_Entry{a}_Open"));
        v.push(format!("PAX_Entry{a}_Req"));
        v.push(format!("PAX_Exit{a}_Open"));
        v.push(format!("PAX_Exit{a}_Req"));
    }
    for i in 0..6 {
        v.push(format!("Debug_{i}"));
    }
    for i in 0..4 {
        for n in ["alpha", "beta", "gamma"] {
            v.push(format!("articulation_{i}_{n}"));
        }
    }
    v
}

pub fn builtin_str_vars(root: &Path) -> Vec<String> {
    match omsi_cfg::CfgFile::read(root.join("program/stringvarlist_roadvehicle.txt")) {
        Ok(f) => f
            .lines
            .iter()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        Err(_) => vec![
            "ident".into(),
            "number".into(),
            "act_route".into(),
            "act_busstop".into(),
            "SetLineTo".into(),
            "yard".into(),
            "file_schedule".into(),
        ],
    }
}

/// A bone of a `[smoothskin]` mesh: the model mesh whose animation moves it (`[setbone]
/// name id`, the id counting the meshes of the model's first level of detail - the GN92's and
/// the O530G's joint dummies `Gelenk_A`-`D` are its first four) and the weights of the o3d
/// bone of that name. `None`: an o3d bone no `[setbone]` names, which keeps its weights and
/// stays with the mesh itself (the root bone of an armature: the Agora's bellows hang half of
/// many a vertex on `Armature1_Bone`, its retarder lever on `Bone`).
#[derive(Debug, Clone, Default)]
pub struct SkinBone {
    pub def_index: Option<usize>,
    pub weights: Vec<(u32, f32)>,
}

/// One mesh of the vehicle model ready for rendering.
pub struct VehicleMesh {
    pub def_index: usize,
    /// The mesh, or (for a type loaded with [`VehicleType::load_ai`]) only its material
    /// ranges: the vertices are read again from `file` when the GPU needs them.
    pub data: MeshData,
    /// The `.o3d` file the mesh came from.
    pub file: PathBuf,
    pub materials: Vec<omsi_o3d::Material>,
    pub overrides: Vec<MaterialDef>,
    pub pivot: Mat4,
    pub viewpoint: i32,
    /// `[smoothskin]` bones (empty for a rigid mesh, and for an AI type).
    pub skin: Vec<SkinBone>,
}

pub struct VehicleType {
    pub def: Vehicle,
    pub model: Model,
    pub model_dir: PathBuf,
    pub program: Arc<Program>,
    pub meshes: Vec<VehicleMesh>,
    pub keep_winding: bool,
    /// Paint schemes / adverts from the `[CTC]` folders' `.cti` files.
    pub paint_schemes: Vec<PaintScheme>,
    /// `[texchanges]`: material textures a script variable swaps (roller blinds, trim).
    pub texchanges: Vec<omsi_model::TexChangeMaster>,
    /// The model's tyres, by the axle number of the `Wheel_Rotation_<n>_*` that turns them
    /// (an articulated bus's rear section counts on from the front): (n, centre height,
    /// centre position along the bus, radius).
    pub wheel_meshes: Vec<(usize, f32, f32, f32)>,
    /// Axle numbers whose wheels the model moves with `Axle_Suspension_<n>_*`.
    pub suspension_axles: Vec<usize>,
    /// Other vehicle packs this one takes meshes from that are not installed (see
    /// `omsi_cfg::missing_vehicle_pack`), with how many meshes are missing for it.
    pub missing_packs: Vec<(String, usize)>,
    /// Per mesh: a sphere (centre, radius) around its vertices in the mesh's own frame, so
    /// that a ray (the mouse over the cockpit) can pass most meshes by without looking at a
    /// triangle. Zero for the meshes an AI type keeps no vertices of.
    pub mesh_bounds: Vec<(Vec3, f32)>,
    /// Per mesh: the box (least, greatest corner) its vertices take in the mesh's own frame,
    /// kept for AI types too (zero for a mesh without vertices).
    pub mesh_boxes: Vec<(Vec3, Vec3)>,
}

/// One `.cti` item group: replaces the textures of `[CTCTexture]` slots and sets variables.
#[derive(Debug, Clone, Default)]
pub struct PaintScheme {
    pub name: String,
    /// Folder the item came from (textures resolve there first).
    pub dir: PathBuf,
    /// (`[CTCTexture]` name, texture file)
    pub textures: Vec<(String, String)>,
    pub set_vars: Vec<(String, f32)>,
}

/// Collect the `[item]` groups of every `.cti` file in `dir`. Consecutive items with the same
/// name form one scheme; `[setvar]` lines belong to the item before them. The folder is read
/// in every content root (a repaint installed as a mod lies in the content folder's copy of
/// the stock folder); a file of the same name in a higher-priority root replaces the other.
pub fn load_paint_schemes(dir: &Path) -> Vec<PaintScheme> {
    let mut schemes: Vec<PaintScheme> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    // the folder in every content root (and mounted archive) that has it
    for d in omsi_cfg::mirrored_dirs(dir) {
        let Some(list) = omsi_cfg::vfs::list_dir(&d) else {
            continue;
        };
        let mut here: Vec<PathBuf> = list
            .into_iter()
            .map(|(n, _)| d.join(n))
            .filter(|p| {
                p.extension()
                    .map(|e| e.eq_ignore_ascii_case("cti"))
                    .unwrap_or(false)
            })
            .collect();
        here.sort();
        for f in here {
            if seen.insert(
                f.file_name()
                    .map(|n| n.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default(),
            ) {
                files.push(f);
            }
        }
    }
    for f in files {
        let dir = f
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| dir.to_path_buf());
        let Ok(cfg) = omsi_cfg::CfgFile::read(&f) else {
            continue;
        };
        let mut r = cfg.reader();
        while let Some(k) = r.next_keyword() {
            match k.as_str() {
                "item" => {
                    let name = r.str().to_string();
                    let ctc = r.str().to_string();
                    let tex = r.str().to_string();
                    match schemes
                        .iter_mut()
                        .find(|s| s.name.eq_ignore_ascii_case(&name))
                    {
                        Some(s) => s.textures.push((ctc, tex)),
                        None => schemes.push(PaintScheme {
                            name,
                            dir: dir.clone(),
                            textures: vec![(ctc, tex)],
                            set_vars: Vec::new(),
                        }),
                    }
                }
                "setvar" => {
                    let var = r.str().to_string();
                    let v = r.f32();
                    if let Some(s) = schemes.last_mut() {
                        s.set_vars.push((var, v));
                    }
                }
                _ => {}
            }
        }
    }
    schemes
}

/// Where a `[mesh]` file of a model lives: next to the model file as OMSI reads it, else -
/// for add-ons laid out for another folder (Studio Polygon's `Configuration Files` sit
/// beside `model`, and packs that borrow parts name them from the vehicle folder or the
/// game folder) - the first of the vehicle folder, its `model` folder and the game folder
/// that has it. The model folder's spelling when none does, for the warning.
fn mesh_path(root: &Path, dir: &Path, model_dir: &Path, file: &str) -> PathBuf {
    let first = omsi_cfg::resolve_path(model_dir, file);
    if omsi_cfg::vfs::exists(&first) {
        return first;
    }
    let model = omsi_cfg::resolve_path(dir, "model");
    let parent = model_dir.parent().map(Path::to_path_buf);
    for base in [Some(dir.to_path_buf()), Some(model), parent, Some(root.to_path_buf())].into_iter().flatten() {
        let p = omsi_cfg::resolve_path(&base, file);
        if omsi_cfg::vfs::exists(&p) {
            return p;
        }
        // (a path that names its own folder again: "model\Configuration Files\..." given
        // from inside `model`)
        let trimmed = file.trim_start_matches(['\\', '/']);
        if let Some(rest) = trimmed.split_once(['\\', '/']).map(|(_, r)| r) {
            let p = omsi_cfg::resolve_path(&base, rest);
            if omsi_cfg::vfs::exists(&p) {
                return p;
            }
        }
    }
    first
}

impl VehicleType {
    pub fn load(root: &Path, bus_file: &Path) -> Result<VehicleType> {
        Self::load_with(root, bus_file, true)
    }

    /// The box the whole model takes (least, greatest corner; y forward), None without
    /// vertices.
    pub fn model_box(&self) -> Option<(Vec3, Vec3)> {
        let (lo, hi) = self
            .mesh_boxes
            .iter()
            .filter(|(lo, hi)| hi.x > lo.x || hi.y > lo.y)
            .fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), (lo, hi)| (a.min(*lo), b.max(*hi)));
        (hi.y > lo.y).then_some((lo, hi))
    }

    /// Half its length as Omsi.exe keeps it (type +0xd4): half the `[boundingbox]`'s
    /// length, else half the model's.
    pub fn half_length(&self) -> Option<f32> {
        match self.def.bounding_box {
            Some(bb) if bb[1] > 0.0 => Some(bb[1] * 0.5),
            _ => self.model_box().map(|(lo, hi)| (hi.y - lo.y) * 0.5),
        }
    }

    /// A type for AI copies: its meshes are measured (the tyres) and let go - nothing but
    /// the upload to the GPU needs them, and a timetable fleet kept half a gigabyte of
    /// vertices on the CPU. [`VehicleType::mesh_data`] reads a mesh again.
    pub fn load_ai(root: &Path, bus_file: &Path) -> Result<VehicleType> {
        Self::load_with(root, bus_file, false)
    }

    fn load_with(root: &Path, bus_file: &Path, keep_meshes: bool) -> Result<VehicleType> {
        let def =
            Vehicle::load(bus_file).with_context(|| format!("loading {}", bus_file.display()))?;
        let dir = def.dir().to_path_buf();
        let model_rel = def.model.clone().context("vehicle has no [model]")?;
        let model_path = omsi_cfg::resolve_path(&dir, &model_rel);
        let model = Model::load(&model_path)
            .with_context(|| format!("loading {}", model_path.display()))?;
        let model_dir = model_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let mut input = CompileInput {
            builtin_vars: builtin_vars(root),
            builtin_str_vars: builtin_str_vars(root),
            ..Default::default()
        };
        input.varlists = def.scripts.varlists.clone();
        input.stringvarlists = def.scripts.stringvarlists.clone();
        input.constfiles = def.scripts.constfiles.clone();
        input.scripts = def.scripts.scripts.clone();
        let program = compile(&input);
        for e in &program.errors {
            log::warn!("{e}");
        }
        let mut meshes = Vec::new();
        let mut missing_packs: Vec<(String, usize)> = Vec::new();
        let (mut turned, mut positive_forward, mut positive_backward) = (Vec::new(), 0usize, 0usize);
        if !model.lods.is_empty() {
            let start = model.lods[0].first_mesh;
            let end = model
                .lods
                .get(1)
                .map(|l| l.first_mesh)
                .unwrap_or(model.meshes.len());
            for (i, md) in model.meshes[start..end].iter().enumerate() {
                let p = mesh_path(root, &dir, &model_dir, &md.file);
                match omsi_o3d::load_mesh(&p) {
                    Ok(m) => {
                        let skin: Vec<SkinBone> = if md.smooth_skin {
                            m.bones
                                .iter()
                                .map(|b| {
                                    let id = md
                                        .bones
                                        .iter()
                                        .find(|(n, _)| {
                                            n.trim().eq_ignore_ascii_case(b.name.trim())
                                        })
                                        .map(|(_, id)| *id)
                                        .filter(|id| *id >= 0);
                                    SkinBone {
                                        def_index: id.map(|id| start + id as usize),
                                        weights: b
                                            .weights
                                            .iter()
                                            .map(|w| (w.vertex, w.weight))
                                            .collect(),
                                    }
                                })
                                .collect()
                        } else {
                            Vec::new()
                        };
                        // (a mesh none of whose bones is bound moves as a rigid one)
                        let skin = if skin.iter().any(|b| b.def_index.is_some()) { skin } else { Vec::new() };
                        match omsi_geometry::positive_det_faces_forward(&m) {
                            Some(true) => positive_forward += 1,
                            Some(false) => positive_backward += 1,
                            None => {}
                        }
                        if omsi_geometry::turns_round(&m) {
                            turned.push(meshes.len());
                        }
                        meshes.push(VehicleMesh {
                            def_index: start + i,
                            data: mesh_from_o3d(&m),
                            file: p,
                            materials: m.materials.clone(),
                            overrides: md.materials.clone(),
                            pivot: pivot_from_mesh(&m),
                            viewpoint: md.viewpoint,
                            skin,
                        })
                    }
                    Err(e) => {
                        match omsi_cfg::missing_vehicle_pack(&p) {
                            Some(pack) => match missing_packs.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(&pack)) {
                                Some(m) => m.1 += 1,
                                None => missing_packs.push((pack, 1)),
                            },
                            None => log::warn!("{}: {e}", p.display()),
                        }
                    }
                }
            }
        }
        let keep_winding = positive_forward > positive_backward;
        if keep_winding && !turned.is_empty() {
            for &i in &turned {
                omsi_geometry::reverse_winding(&mut meshes[i].data);
            }
            log::info!("{}: {} meshes keep their winding ({positive_forward} of the meshes with a positive determinant face along their normals, {positive_backward} against them)", bus_file.display(), turned.len());
        }
        for (pack, n) in &missing_packs {
            log::warn!(
                "{}: {n} meshes come from the vehicle pack 'Vehicles/{pack}', which is not installed - install it for the parts this bus borrows from it (displays, ticket machine, dashboard)",
                bus_file.display()
            );
        }
        let mut paint_schemes = Vec::new();
        for c in &model.ctc {
            let d = omsi_cfg::resolve_path(&dir, &c.path);
            paint_schemes.extend(load_paint_schemes(&d));
        }
        // the model's own items, after the `.cti` files' (their textures in the first
        // `[CTC]` folder, as a `.cti` of that folder has them)
        let item_dir = model.ctc.first().map(|c| omsi_cfg::resolve_path(&dir, &c.path)).unwrap_or_else(|| dir.clone());
        for it in &model.items {
            match paint_schemes.last_mut().filter(|s: &&mut PaintScheme| s.name.eq_ignore_ascii_case(&it.name) && s.dir == item_dir) {
                Some(s) => {
                    s.textures.push((it.ctc.clone(), it.texture.clone()));
                    s.set_vars.extend(it.set_vars.iter().cloned());
                }
                None => paint_schemes.push(PaintScheme { name: it.name.clone(), dir: item_dir.clone(), textures: vec![(it.ctc.clone(), it.texture.clone())], set_vars: it.set_vars.clone() }),
            }
        }
        let texchanges = omsi_model::load_texchanges(&dir, &model.texchanges);
        let (wheel_meshes, suspension_axles) = wheel_meshes(&model, &meshes);
        let mesh_boxes: Vec<(Vec3, Vec3)> = meshes
            .iter()
            .map(|m| {
                let lo = m.data.positions.iter().fold(Vec3::splat(f32::MAX), |a, p| a.min(*p));
                let hi = m.data.positions.iter().fold(Vec3::splat(f32::MIN), |a, p| a.max(*p));
                if lo.x <= hi.x { (lo, hi) } else { (Vec3::ZERO, Vec3::ZERO) }
            })
            .collect();
        let mesh_bounds = if keep_meshes {
            meshes.iter().map(|m| omsi_geometry::bounding_sphere(&m.data.positions)).collect()
        } else {
            vec![(Vec3::ZERO, 0.0); meshes.len()]
        };
        // A `[boundingbox]` much longer or wider than the model itself: the W906 Sprinter mod
        // declares 12 m for a 7 m van (copied from a bus), and the walkers ran into an
        // invisible wall metres ahead of its nose and behind its tail, the traffic kept its
        // distance from nothing. The box is cut down to the model (never made bigger).
        let mut def = def;
        if let Some(bb) = def.bounding_box.as_mut() {
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for m in &meshes {
                for p in &m.data.positions {
                    lo = lo.min(*p);
                    hi = hi.max(*p);
                }
            }
            if hi.x > lo.x && hi.y > lo.y {
                let size = hi - lo;
                let mid = (hi + lo) * 0.5;
                if bb[1] > size.y + 1.5 {
                    log::info!("{}: [boundingbox] {:.1} m long, the model {:.1} m: the model's length taken", bus_file.display(), bb[1], size.y);
                    bb[1] = size.y + 0.1;
                    bb[4] = mid.y;
                }
                if bb[0] > size.x + 0.8 {
                    bb[0] = size.x + 0.05;
                    bb[3] = mid.x;
                }
            }
        }
        if !keep_meshes {
            // a `[smoothskin]` mesh (the bellows of an articulated bus, `[setbone]`-bound to
            // the joint's dummies) keeps its vertices and its bones even for an AI copy: it
            // is reshaped as the joint turns (`scene::own_skinned_meshes`), which needs the
            // rest pose and the bone weights this would otherwise throw away - an AI-loaded
            // GN92 or O530G kept its bellows perfectly straight through every bend because
            // there was nothing left to skin it from. There are only ever one or two such
            // meshes in a model, so keeping them costs nothing like the half gigabyte this
            // was written to save.
            for m in meshes.iter_mut().filter(|m| m.skin.is_empty()) {
                m.data = MeshData {
                    ranges: std::mem::take(&mut m.data.ranges),
                    ..Default::default()
                };
            }
        }
        Ok(VehicleType {
            def,
            model,
            model_dir,
            program: Arc::new(program),
            meshes,
            paint_schemes,
            texchanges,
            wheel_meshes,
            suspension_axles,
            missing_packs,
            mesh_bounds,
            mesh_boxes,
            keep_winding,
        })
    }

    /// Per axle of the `.bus` (whose first axle is number `first_axle` of the whole train):
    /// the model's wheel centre height and tyre radius. The largest mesh turning with the
    /// axle is its tyre (a rim or a hub cap turns with it); one standing elsewhere along the
    /// bus belongs to another axle.
    pub fn wheel_geometry(&self, first_axle: usize) -> Vec<Option<(f32, f32)>> {
        self.def
            .axles
            .iter()
            .enumerate()
            .map(|(a, axle)| {
                self.wheel_meshes
                    .iter()
                    .filter(|(n, _, long, r)| {
                        *n == first_axle + a
                            && (long - axle.long).abs() <= 0.5
                            && *r >= axle.wheel_diameter * 0.3
                            && *r <= axle.wheel_diameter * 0.75
                    })
                    .fold(None, |best: Option<(f32, f32)>, (_, z, _, r)| {
                        if best.map(|b| *r > b.1 + 1e-3).unwrap_or(true) {
                            Some((*z, *r))
                        } else {
                            best
                        }
                    })
            })
            .collect()
    }

    /// Unloaded hub height per axle for the physics: the model's wheel centre, raised by what
    /// the `.bus` tyre is larger than the modelled one. The tyre then touches the road exactly
    /// where the physics puts the contact - the stock SD202's wheels are modelled 1.6 cm
    /// below the model's ground plane and stood that much in the asphalt. A model that says
    /// something implausible (more than 8 cm off the tyre radius) keeps the radius.
    ///
    /// A model that does not move an axle's wheels with `Axle_Suspension` (a railway car, a
    /// plane) cannot show them sitting up in their arches: that axle hangs its wheels as much
    /// lower as the load compresses the spring, and the body keeps its unloaded height.
    pub fn hub_heights(&self, first_axle: usize) -> Vec<Option<f32>> {
        let loads = crate::rigid::wheel_rest_loads(&self.def);
        self.def
            .axles
            .iter()
            .zip(self.wheel_geometry(first_axle))
            .enumerate()
            .map(|(a, (axle, g))| {
                let r = (axle.wheel_diameter / 2.0).max(0.15);
                let model = g
                    .map(|(z, r_mesh)| z + (r - r_mesh))
                    .filter(|z| (z - r).abs() <= 0.08);
                if self.suspension_axles.contains(&(first_axle + a)) {
                    return model;
                }
                let k = if axle.spring > 0.0 {
                    axle.spring * 1000.0
                } else {
                    150_000.0
                };
                let sag = (loads.get(a).copied().unwrap_or(0.0) / k).min(crate::rigid::BUMP);
                Some(model.unwrap_or(r) - sag)
            })
            .collect()
    }

    /// Texture substitutions of a paint scheme: default `[CTCTexture]` file (lower case) →
    /// replacement file, plus the folder to search first.
    pub fn scheme_substitutions(
        &self,
        scheme: usize,
    ) -> (HashMap<String, String>, Option<PathBuf>) {
        let mut map = HashMap::new();
        let Some(s) = self.paint_schemes.get(scheme) else {
            return (map, None);
        };
        for (ctc_name, file) in &s.textures {
            for (name, default) in &self.model.ctc_textures {
                if name.eq_ignore_ascii_case(ctc_name) {
                    map.insert(default.to_ascii_lowercase(), file.clone());
                }
            }
        }
        (map, Some(s.dir.clone()))
    }

    /// The texture swaps of the model's own look (no paint scheme chosen): none for the
    /// livery, but a `[CTCTexture]` whose own texture is a flat, colourless placeholder
    /// takes the first scheme's. The BMC Procity's destination display is lit through its
    /// CTC texture `afisaj`: the model's own `vmatrix_voll_LCD.dds` is plain grey and every
    /// repaint brings the orange of the LEDs - with the model's own textures the display
    /// shone white. A body texture is never a flat colour, so the livery stays the model's.
    /// Values are full paths.
    pub fn default_substitutions(&self, root: &Path) -> HashMap<String, String> {
        static CACHE: std::sync::OnceLock<parking_lot::Mutex<HashMap<PathBuf, HashMap<String, String>>>> = std::sync::OnceLock::new();
        let cache = CACHE.get_or_init(|| parking_lot::Mutex::new(HashMap::new()));
        if let Some(m) = cache.lock().get(&self.def.path) {
            return m.clone();
        }
        let mut map = HashMap::new();
        if let Some(first) = self.paint_schemes.first() {
            let dirs = self.texture_dirs(root);
            for (name, default) in &self.model.ctc_textures {
                let Some((_, file)) = first.textures.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) else {
                    continue;
                };
                // only a display's colour: a material that shows a script or text texture
                // through this one (`[matl_transmap] \S:n`, [useScriptTexture],
                // [useTextTexture]). Window paint and a body's transparency map are CTC
                // textures of one flat tone as well (the LiAZ's `Kuzov_trans.dds`), and the
                // first repaint's put its livery over the windows as large splashes.
                let display = self.model.meshes.iter().flat_map(|m| m.materials.iter()).any(|o| {
                    o.texture.trim().eq_ignore_ascii_case(default.trim())
                        && (o.transmap.as_deref().is_some_and(|t| t.trim().starts_with("\\S:"))
                            || o.use_script_texture.is_some()
                            || o.use_text_texture.is_some())
                });
                if !display {
                    continue;
                }
                let own = dirs.iter().map(|d| omsi_cfg::resolve_path(d, default)).find(|p| omsi_cfg::vfs::is_file(p));
                let Some(own) = own else { continue };
                if !flat_placeholder(&own) {
                    continue;
                }
                let theirs = omsi_cfg::resolve_path(&first.dir, file);
                if omsi_cfg::vfs::is_file(&theirs) {
                    log::info!("{}: CTC texture '{name}' is a flat placeholder ({default}); the first paint scheme's {file} lights it", self.def.path.display());
                    map.insert(default.to_ascii_lowercase(), theirs.to_string_lossy().into_owned());
                }
            }
        }
        cache.lock().insert(self.def.path.clone(), map.clone());
        map
    }

    /// The `[texchanges]` master for a mesh material's texture name, if there is one.
    pub fn texchange(&self, texture: &str) -> Option<&omsi_model::TexChangeMaster> {
        self.texchanges
            .iter()
            .find(|m| m.texture.eq_ignore_ascii_case(texture.trim()))
    }

    /// Bytes of the meshes the type keeps on the CPU.
    pub fn mesh_bytes(&self) -> usize {
        self.meshes.iter().map(|m| m.data.heap_bytes()).sum()
    }

    /// Mesh `i` with its vertices: as kept, or read again from its file (a type loaded with
    /// [`VehicleType::load_ai`]). None when the file cannot be read any more.
    pub fn mesh_data(&self, i: usize) -> Option<std::borrow::Cow<'_, MeshData>> {
        let m = self.meshes.get(i)?;
        if !m.data.indices.is_empty() || m.data.ranges.is_empty() {
            return Some(std::borrow::Cow::Borrowed(&m.data));
        }
        match omsi_o3d::load_mesh(&m.file) {
            Ok(o) => Some(std::borrow::Cow::Owned(omsi_geometry::mesh_from_o3d_turning(&o, !self.keep_winding))),
            Err(e) => {
                log::warn!("{}: {e}", m.file.display());
                None
            }
        }
    }

    /// Whether mesh `i` has to be read again before it can go to the GPU.
    pub fn mesh_dropped(&self, i: usize) -> bool {
        self.meshes
            .get(i)
            .map(|m| m.data.indices.is_empty() && !m.data.ranges.is_empty())
            .unwrap_or(false)
    }

    pub fn texture_dirs(&self, root: &Path) -> Vec<PathBuf> {
        vec![
            // (the folder as it is spelled on the disk: many add-ons ship `texture`, which a
            // case-sensitive file system - Linux, a phone - does not find as `Texture`)
            omsi_cfg::resolve_path(self.def.dir(), "Texture"),
            omsi_cfg::resolve_path(&self.model_dir, "Texture"),
            self.model_dir.clone(),
            omsi_cfg::resolve_path(root, "Texture"),
        ]
    }
}

/// A texture of one flat, colourless tone (a stand-in the paint schemes replace).
fn flat_placeholder(p: &Path) -> bool {
    let Ok(img) = omsi_texture::decode_file(p) else { return false };
    let n = (img.width as usize * img.height as usize).max(1);
    let step = (n / 4096).max(1);
    let (mut sum, mut sq, mut k) = ([0f64; 3], [0f64; 3], 0f64);
    for px in img.rgba.chunks(4).step_by(step) {
        for c in 0..3 {
            let v = px[c] as f64;
            sum[c] += v;
            sq[c] += v * v;
        }
        k += 1.0;
    }
    if k < 1.0 {
        return false;
    }
    let mean: Vec<f64> = sum.iter().map(|s| s / k).collect();
    let spread = (0..3).map(|c| (sq[c] / k - mean[c] * mean[c]).max(0.0).sqrt()).fold(0.0, f64::max);
    let chroma = mean.iter().cloned().fold(0.0, f64::max) - mean.iter().cloned().fold(255.0, f64::min);
    // (a few black cells for the unlit part of a matrix are allowed: the median tone matters)
    spread < 40.0 && chroma < 20.0
}

/// The tyres of a model - every mesh turned by `Wheel_Rotation_<n>_*`, with its centre and
/// its radius about the axle - and the axle numbers it moves with `Axle_Suspension_<n>_*`.
fn wheel_meshes(
    model: &Model,
    meshes: &[VehicleMesh],
) -> (Vec<(usize, f32, f32, f32)>, Vec<usize>) {
    let number = |var: &str, prefix: &str| {
        var.to_ascii_lowercase()
            .strip_prefix(prefix)
            .and_then(|r| r.split('_').next().and_then(|a| a.parse::<usize>().ok()))
    };
    let mut wheels = Vec::new();
    let mut suspension = Vec::new();
    for m in meshes {
        for an in &model.meshes[m.def_index].animations {
            if let Some(n) = number(&an.variable, "axle_suspension_") {
                if !suspension.contains(&n) {
                    suspension.push(n);
                }
            }
            if an.kind != Some(omsi_model::AnimKind::Rot) {
                continue;
            }
            let Some(n) = number(&an.variable, "wheel_rotation_") else {
                continue;
            };
            let mut pivot = None;
            for o in &an.origins {
                match o {
                    omsi_model::AnimOrigin::Trans(t) => {
                        pivot = Some(pivot.unwrap_or(Vec3::ZERO) + Vec3::from(*t))
                    }
                    omsi_model::AnimOrigin::FromMesh => pivot = Some(m.pivot.w_axis.truncate()),
                    _ => {}
                }
            }
            let Some(p) = pivot else { continue };
            let radius = m
                .data
                .positions
                .iter()
                .map(|v| ((v.y - p.y).powi(2) + (v.z - p.z).powi(2)).sqrt())
                .fold(0.0f32, f32::max);
            wheels.push((n, p.z, p.y, radius));
        }
    }
    (wheels, suspension)
}

/// Per-mesh dynamic display properties evaluated from script variables every frame.
#[derive(Debug, Clone)]
pub struct MeshProps {
    pub visible: bool,
    /// Per material slot `[matl_lightmap]` strength.
    pub slot_light: Vec<f32>,
    /// Per material slot: `[matl_change]` item active (its night map shows).
    pub slot_night: Vec<f32>,
    /// Alpha multiplier per material slot of the mesh.
    pub slot_alpha: Vec<f32>,
    /// `[texcoordtransX/Y]` texture offset per material slot.
    pub slot_uv: Vec<[f32; 2]>,
    /// Brightness of the `[interiorlight]`s listed in the mesh's `[illumination_interior]`.
    pub interior: f32,
}

impl Default for MeshProps {
    fn default() -> Self {
        Self {
            visible: true,
            slot_alpha: Vec::new(),
            slot_light: Vec::new(),
            slot_night: Vec::new(),
            slot_uv: Vec::new(),
            interior: 0.0,
        }
    }
}

/// Index of the material slot a `[matl]` override refers to (texture name + nth occurrence).
pub fn override_slot(materials: &[omsi_o3d::Material], o: &MaterialDef) -> Option<usize> {
    let mut nth = 0;
    for (slot, m) in materials.iter().enumerate() {
        if m.texture.eq_ignore_ascii_case(&o.texture) {
            if nth == o.index.max(0) as usize {
                return Some(slot);
            }
            nth += 1;
        }
    }
    None
}

/// Motion state handed to `VehicleInstance::update_ai`.
#[derive(Debug, Clone, Copy, Default)]
pub struct AiFrame {
    /// m/s
    pub speed: f32,
    /// Distance travelled so far (m), drives the wheel rotation.
    pub odometer: f32,
    /// Front wheel angle of the bicycle model through the `[rot_pnt_long]` line (deg,
    /// positive = right); each wheel's own angle follows from it.
    pub steer_deg: f32,
    /// 0 none, 1 left, 2 right, 3 both (hazard warning).
    pub blinker: i32,
    pub brake: bool,
    pub lights: bool,
    /// `AI_Scheduled_AtStation` as OMSI hands it to the script: 1 = standing at a stop, open
    /// the doors; -1 = time to go, close them (the script answers with 0 once they are
    /// shut); 0 = not at a stop (a script still closing its doors is told -1 until it
    /// answers, see `VehicleInstance::station_released`).
    pub at_station: i32,
    /// `TrafficPriorityWarningNeeded`: a vehicle with right of way (`TrafficPriority`) has
    /// something in its way that is to be warned - the stock ambulance's script sounds its
    /// siren for the next 30 m on it.
    pub priority_warning: bool,
}

pub struct VehicleInstance {
    pub ty: Arc<VehicleType>,
    pub state: State,
    pub vm: Vm,
    pub host: VehicleHost,
    /// World position of the vehicle origin and heading (degrees, clockwise from north).
    pub position: DVec3,
    pub heading: f64,
    pub pitch: f32,
    /// The driver's seat's travel and speed, and the body's turning last frame (see
    /// `update_driver_seat`).
    seat: (f32, f32, Vec3),
    pub bank: f32,
    pub animators: Vec<MeshAnimator>,
    /// Current mesh-space transforms, parallel to `ty.meshes`.
    pub mesh_transforms: Vec<Mat4>,
    pub mesh_props: Vec<MeshProps>,
    pub physics: VehiclePhysics,
    /// `[texttexture]` states, parallel to `ty.model.text_textures`.
    pub text_textures: Vec<crate::texttex::TextTextureState>,
    pub html_textures: Vec<crate::htmltex::HtmlTexture>,
    /// Ground height query (world x, y → z), set by the world.
    /// Ground height sampler (shared: the script host probes the same one).
    pub ground: Option<std::sync::Arc<dyn Fn(f64, f64) -> Option<f64> + Send + Sync>>,
    /// What the wheels stand on: the highest face at or below a height and the lowest one
    /// above it. Without it the wheels stand on `ground`.
    pub contact: Option<std::sync::Arc<dyn crate::rigid::Ground>>,
    /// Vehicles coupled behind this one.
    pub trailers: Vec<TrailerPart>,
    /// The vehicle's `[smoke]` particle systems (see `crate::particles`).
    pub particles: ParticleSet,
    /// How far each `[light_enh_2]` of the model (in order) has come on: its fading variable
    /// followed with the light's `timeconst` (63 % of the way in that time when switched on,
    /// down to 27 % when switched off, as the stock files document it).
    pub light_fade: Vec<f32>,
    /// The meshes' transforms in the modelled pose, for `[smoothskin]` (made when needed).
    skin_rest: Vec<Mat4>,
    /// Static obstacles; the vehicle's `[boundingbox]` is kept out of them.
    pub collision: Option<Arc<crate::collision::CollisionWorld>>,
    /// Moving obstacles (AI vehicles) for this frame, set by the app.
    pub dynamic_boxes: Vec<crate::collision::Obb>,
    /// A collision happened this frame: the `{trigger:collision}` block runs after physics.
    collided: bool,
    /// Energy of the last crash (J), for whoever wants to report it; cleared by the reader.
    pub last_crash: f32,
    /// Faces the wheels cannot climb stop the vehicle (see `RigidBody::wheel_walls`); the
    /// player's bus follows the setting for collisions with objects.
    pub wheel_walls: bool,
    /// Crashes so far and the energy of the latest (J), kept for logs and the HUD.
    pub crashes: u32,
    pub last_impact: f32,
    /// Map ids of the `[crashmode_pole]` posts this vehicle has knocked over, and those
    /// knocked this frame with the direction they fall in (the scene lays them down).
    pub knocked: Vec<i64>,
    pub knocked_now: Vec<(i64, DVec3)>,
    /// Moving obstacles (AI cars, by their collision id) in contact last frame: one crash per
    /// contact, not one per frame of it.
    touching: Vec<i64>,
    /// Per axle, the static spring compression and how far the tyre radius exceeds the
    /// unloaded hub height (m): the simple model has no springs, but sits as low.
    rest_sag: Vec<(f32, f32)>,
    /// How far an AI copy has to stand higher for its modelled tyres to touch the road.
    ai_lift: f32,
    /// How dirty the body is (0..1). OMSI reports it to the model as `Dirt_Norm`, where
    /// it scales the alpha of the dirt overlay; the bus wash clears it.
    pub dirt: f32,
    /// Rigid-body dynamics (player vehicles); None = simple kinematic model.
    pub rigid: Option<crate::rigid::RigidBody>,
    /// The tyre meshes that are seated on their physical hub (see `seat_wheels`): mesh,
    /// wheel of the rigid body, the mesh's turning centre at rest (model frame). Built on
    /// first use.
    wheel_seats: Option<Vec<(usize, usize, Vec3)>>,
    /// The AI odometer at the last `update_ai` (the wheels roll the difference).
    ai_odometer: f32,
    /// Kilometres driven this session (the odometer's `kmcounter_*`).
    driven_km: f64,
    /// The odometer's start was set from `[kmcounter_init]` (see `update_engine_vars`).
    km_started: bool,
    /// The cabin air as the engine keeps it (°C, g/m³), and the value last written, so that
    /// a bus whose own scripts heat or cool the cabin keeps what they write.
    cabin_air: Option<(f32, f32, f32)>,
    /// An AI vehicle nobody can see (out of the view and away from the mirrors) leaves its
    /// mesh animations and material values as they are: the traffic clears this while it is
    /// out of sight, and the time it missed is made up at once when it comes back. The body,
    /// the coupled parts and the scripts still run every frame.
    pub ai_visuals: bool,
    ai_visuals_missed: f32,
    var_index: HashMap<String, omsi_script::VarId>,
    /// Where each mesh's material properties come from, resolved against `var_index`
    /// (rebuilt when the vehicle gains engine variables).
    props_plan: PropsPlan,
    // cached variable ids for the physics interface
    v_velocity: Option<omsi_script::VarId>,
    v_velocity_ground: Option<omsi_script::VarId>,
    v_n_wheel: Option<omsi_script::VarId>,
    v_m_wheel: Option<omsi_script::VarId>,
    v_throttle: Option<omsi_script::VarId>,
    v_brake: Option<omsi_script::VarId>,
    /// `Brakeforce`: the scripts' brake force for the whole vehicle (N), shared out over its
    /// wheels besides each wheel's own `Axle_Brakeforce_*`.
    v_brakeforce: Option<omsi_script::VarId>,
    v_clutch: Option<omsi_script::VarId>,
    /// `PAX_Entry0..7_Req` and `PAX_Exit0..7_Req`: set by the passengers every frame and
    /// cleared after the scripts' frame (see `clear_pax_requests`).
    v_pax_req: Vec<omsi_script::VarId>,
    v_accel: [Option<omsi_script::VarId>; 3],
    v_wheels: Vec<[[Option<omsi_script::VarId>; 5]; 2]>,
    v_springfactor: Vec<[Option<omsi_script::VarId>; 2]>,
}

impl VehicleInstance {
    /// The fleet number the scripts know as `number` (empty for a vehicle without one).
    pub fn number(&self) -> String {
        self.ty
            .program
            .str_var("number")
            .and_then(|i| self.state.str_vars.get(i as usize))
            .cloned()
            .unwrap_or_default()
    }

    pub fn new(ty: Arc<VehicleType>, host: VehicleHost) -> VehicleInstance {
        let program = ty.program.clone();
        let mut state = State::new(&program);
        let mut vm = Vm::new();
        let mut host = host;
        host.script_textures = ty
            .model
            .script_textures
            .iter()
            .map(|(w, h)| crate::scripttex::ScriptTexture::new(*w, *h))
            .collect();
        host.content_dir = ty.def.dir().to_path_buf();
        let html_textures: Vec<crate::htmltex::HtmlTexture> = ty
            .model
            .html_textures
            .iter()
            .map(|d| {
                let dirs = [ty.model_dir.as_path(), ty.def.dir()];
                let html = crate::htmltex::load_page(&dirs, &d.path);
                crate::htmltex::HtmlTexture::new(d.script_index, d.width, d.height, &html)
                    .with_asset_dirs(crate::htmltex::asset_dirs(&dirs, &d.path))
            })
            .collect();
        host.number_var = program.str_var("number");
        // defaults every bus expects before {init}
        let mut var_index = HashMap::new();
        for (i, n) in program.var_names.iter().enumerate() {
            var_index.insert(n.to_ascii_lowercase(), i as omsi_script::VarId);
        }
        // engine-provided defaults
        if let Some(i) = var_index.get("envir_brightness") {
            state.vars[*i as usize] = 1.0;
        }
        // no ticket printed yet
        if let Some(i) = var_index.get("giventicket") {
            state.vars[*i as usize] = -1.0;
        }
        // `wearlifespan` (varlist_roadvehicle.txt, written by the engine from the
        // difficulty settings): every random part lifetime the scripts draw at {init} is
        // multiplied by it - the rear-door automatic, bulbs, the matrix display. Left at 0
        // the SD200's rear door was worn out on its first closing and reopened by itself,
        // with the opening sound, from then on; the matrix "spinnt" for the same reason.
        if let Some(i) = var_index.get("wearlifespan") {
            state.vars[*i as usize] = host.wear_lifespan.max(0.01);
        }
        // These are engine-owned visual inputs, not script variables.  They still need
        // to exist before {init}: material/prop setup can read them while the scene is
        // created, which otherwise makes stock buses (notably GN92) start with the dirt
        // layer at full opacity until the first simulation tick repairs it.
        for (name, value) in [
            ("Dirt_Norm", 0.0),
            ("DirtRate", 0.0),
            ("PrecipRate", 0.0),
            ("StreetCond", 0.0),
        ] {
            let key = name.to_ascii_lowercase();
            let id = match var_index.get(&key).copied() {
                Some(id) => id,
                None => {
                    let id = state.vars.len() as omsi_script::VarId;
                    state.vars.push(value);
                    var_index.insert(key, id);
                    id
                }
            };
            state.vars[id as usize] = value;
        }
        // `Axle_Springfactor_*` (varlist_roadvehicle.txt): the engine's spring scale, which
        // air-suspension scripts overwrite with their bellows pressure; a steel-sprung bus
        // never writes it and must not ride on springs of zero
        for n in program
            .var_names
            .iter()
            .filter(|n| n.to_ascii_lowercase().starts_with("axle_springfactor_"))
        {
            if let Some(i) = var_index.get(&n.to_ascii_lowercase()) {
                state.vars[*i as usize] = 1.0;
            }
        }
        // `$.yard` is the depot the bus runs from, the [name] of its .hof, known before
        // {init}: the matrix displays build their line list and palette paths from it
        // (`Linienlisten\<yard>_ANX.jpg`, `palettes\<yard>.bmp`), the roller blind its
        // line list, and some IBIS scripts branch on it.
        if let (Some(i), Some(h)) = (program.str_var("yard"), host.hof.as_ref()) {
            state.str_vars[i as usize] = h.name.clone();
        }
        if let Some(scheme) = host.paint_scheme {
            let scheme = scheme.filter(|i| *i < ty.paint_schemes.len());
            let mut put = |name: &str, v: f32| {
                if let Some(id) = var_index.get(&name.to_ascii_lowercase()) {
                    state.vars[*id as usize] = v;
                }
            };
            put("Colorscheme", scheme.map(|i| i as f32).unwrap_or(-1.0));
            if let Some(i) = scheme {
                for (var, v) in &ty.paint_schemes[i].set_vars {
                    put(var, *v);
                }
            }
        }
        vm.run_init(&program, &mut state, &mut host);
        let mut animators: Vec<MeshAnimator> = ty
            .meshes
            .iter()
            .map(|m| MeshAnimator::new(&ty.model.meshes[m.def_index], m.pivot, |n| program.var(n)))
            .collect();
        crate::anim::link_parents(
            &mut animators,
            &ty.meshes
                .iter()
                .map(|m| &ty.model.meshes[m.def_index])
                .collect::<Vec<_>>(),
        );
        let n = ty.meshes.len();
        let physics = VehiclePhysics::from_definition(&ty.def);
        let v = |name: &str| program.var(name);
        let v_wheels = (0..physics.wheels.len())
            .map(|a| {
                let side = |s: &str| {
                    [
                        v(&format!("Wheel_Rotation_{a}_{s}")),
                        v(&format!("Wheel_RotationSpeed_{a}_{s}")),
                        v(&format!("Axle_Steering_{a}_{s}")),
                        v(&format!("Axle_Suspension_{a}_{s}")),
                        v(&format!("Axle_Brakeforce_{a}_{s}")),
                    ]
                };
                [side("L"), side("R")]
            })
            .collect();
        let v_springfactor = (0..physics.wheels.len())
            .map(|a| {
                [
                    v(&format!("Axle_Springfactor_{a}_L")),
                    v(&format!("Axle_Springfactor_{a}_R")),
                ]
            })
            .collect();
        let rest_sag: Vec<(f32, f32)> = {
            let rb = crate::rigid::RigidBody::from_definition(&ty.def, &ty.hub_heights(0));
            (0..physics.wheels.len())
                .map(|a| {
                    let w = &rb.wheels[a * 2];
                    (
                        w.rest_compression().min(crate::rigid::BUMP),
                        w.radius - w.attach.z,
                    )
                })
                .collect()
        };
        // An AI copy has no rigid body: it stands on the contact plane of its wheels
        // (`ai_motion::AiBody`), and what the modelled tyre reaches below its hub less than
        // the model thinks lifts it. An axle the model does not move with `Axle_Suspension`
        // got the spring sag taken off its hub height (`VehicleType::hub_heights`), which an
        // AI copy without springs must not stand on.
        let ai_lift = {
            let offs: Vec<f32> = rest_sag
                .iter()
                .enumerate()
                .map(|(a, (comp, off))| {
                    if ty.suspension_axles.contains(&a) {
                        *off
                    } else {
                        off - comp
                    }
                })
                .collect();
            offs.iter().sum::<f32>() / offs.len().max(1) as f32
        };
        VehicleInstance {
            particles: ParticleSet::new(ty.model.particle_systems(), std::ptr::addr_of!(host) as u64 ^ 0x9e37_79b9),
            light_fade: Vec::new(),
            v_springfactor,
            rest_sag,
            ai_lift,
            contact: None,
            knocked: Vec::new(),
            knocked_now: Vec::new(),
            touching: Vec::new(),
            v_velocity: v("Velocity"),
            v_velocity_ground: v("Velocity_Ground"),
            v_n_wheel: v("n_Wheel"),
            v_m_wheel: v("M_Wheel"),
            v_throttle: v("Throttle").or_else(|| v("throttle_pedal")),
            v_brake: v("Brake").or_else(|| v("brake_pedal")),
            v_brakeforce: v("Brakeforce"),
            v_clutch: v("Clutch").or_else(|| v("clutch_pedal")),
            v_pax_req: (0..8).flat_map(|i| [format!("PAX_Entry{i}_Req"), format!("PAX_Exit{i}_Req")]).filter_map(|n| v(&n)).collect(),
            v_accel: [v("A_Trans_X"), v("A_Trans_Y"), v("A_Trans_Z")],
            v_wheels,
            ty,
            state,
            vm,
            host,
            position: DVec3::ZERO,
            heading: 0.0,
            pitch: 0.0,
            bank: 0.0,
            animators,
            mesh_transforms: vec![Mat4::IDENTITY; n],
            mesh_props: vec![MeshProps::default(); n],
            physics,
            text_textures: Vec::new(),
            html_textures,
            ground: None,
            trailers: Vec::new(),
            skin_rest: Vec::new(),
            collision: None,
            dynamic_boxes: Vec::new(),
            collided: false,
            last_crash: 0.0,
            wheel_walls: true,
            crashes: 0,
            last_impact: 0.0,
            dirt: 0.0,
            rigid: None,
            wheel_seats: None,
            ai_odometer: 0.0,
            seat: (0.0, 0.0, Vec3::ZERO),
            driven_km: 0.0,
            km_started: false,
            cabin_air: None,
            ai_visuals: true,
            ai_visuals_missed: 0.0,
            var_index,
            props_plan: PropsPlan::default(),
        }
    }

    /// Prepare the text textures with fonts from `lib`.
    pub fn init_text_textures(
        &mut self,
        lib: &mut crate::texttex::FontLibrary,
        decode: &dyn Fn(&Path) -> Option<(u32, u32, Vec<u8>)>,
    ) {
        self.text_textures = self
            .ty
            .model
            .text_textures
            .iter()
            .map(|t| crate::texttex::TextTextureState::new(t.clone(), lib.get(&t.font, decode)))
            .collect();
    }

    /// Current text of a string variable (empty when it does not exist).
    pub fn str_var(&self, name: &str) -> String {
        self.ty
            .program
            .str_var(name)
            .map(|i| self.state.str_vars[i as usize].clone())
            .unwrap_or_default()
    }

    /// Re-render changed text textures; returns the indices with a pending image.
    pub fn update_text_textures(&mut self) -> Vec<usize> {
        let mut changed = Vec::new();
        // `Refresh_Strings` is reset; an unchanged string would draw the same picture again
        if let Some(id) = self.ty.program.var("Refresh_Strings") {
            self.state.vars[id as usize] = 0.0;
        }
        for i in 0..self.text_textures.len() {
            let var = self.text_textures[i].def.variable.clone();
            let text = self.str_var(&var);
            if self.text_textures[i].update(&text) {
                changed.push(i);
            }
        }
        changed
    }

    /// `Driver_Seat_VertTransl`: the sprung driver's seat, as OMSI moves it:
    /// the body's vertical acceleration and its roll and pitch at the seat's place (the
    /// driver's camera) push a spring (3000) and damper (2000) on 150 kg; ±10 cm at most.
    fn update_driver_seat(&mut self, dt: f32) {
        let Some(id) = self.ty.program.var("Driver_Seat_VertTransl") else { return };
        if dt <= 0.0 {
            return;
        }
        let seat = self.ty.def.cameras_driver.first().map(|c| c.pos).unwrap_or([0.0; 3]);
        let (acc_z, omega) = match &self.rigid {
            Some(rb) => (rb.accel_body.z - 9.81, rb.omega),
            None => (0.0, Vec3::ZERO),
        };
        let d_omega = omega - self.seat.2;
        self.seat.2 = omega;
        // (the rolls' and pitches' change moves the seat by its lever: y forward rolls
        // about x, x across pitches about y)
        let push = -acc_z * dt + d_omega.y * seat[0] - d_omega.x * seat[1];
        let k = (1000.0 / (15.0 * dt * 1000.0)).min(1.0);
        let (mut x, mut v) = (self.seat.0, self.seat.1);
        v += push + ((-x * 3000.0 - v * 2000.0) / 150.0) * k * dt;
        x += v * dt;
        if x.abs() > 0.1 {
            x = x.clamp(-0.1, 0.1);
            v = 0.0;
        }
        self.seat = (x, v, omega);
        self.state.vars[id as usize] = x;
    }

    /// The variables a paint scheme sets: its `[setvar]` lines, and `Colorscheme` - the
    /// scheme's index in the `.cti` list, −1 for the model's own textures (OMSI
    /// the original at the spawn; the original reads it back as the index).
    pub fn apply_paint_vars(&mut self, scheme: Option<usize>) {
        let scheme = scheme.filter(|i| *i < self.ty.paint_schemes.len());
        self.set_var("Colorscheme", scheme.map(|i| i as f32).unwrap_or(-1.0));
        if let Some(i) = scheme {
            for (var, v) in self.ty.paint_schemes[i].set_vars.clone() {
                self.set_var(&var, v);
            }
        }
    }

    pub fn set_controls(&mut self, c: Controls) {
        self.physics.controls = c;
    }

    fn get(&self, id: Option<omsi_script::VarId>) -> f32 {
        id.map(|i| self.state.vars[i as usize]).unwrap_or(0.0)
    }

    fn put(&mut self, id: Option<omsi_script::VarId>, v: f32) {
        if let Some(i) = id {
            self.state.vars[i as usize] = v;
        }
    }

    /// Physics step: read the script's torque/brake outputs, integrate, write the motion
    /// variables back for the next script frame.
    /// Switch the player vehicle to rigid-body dynamics at its current pose.
    pub fn enable_rigid_body(&mut self) {
        let mut rb =
            crate::rigid::RigidBody::from_definition(&self.ty.def, &self.ty.hub_heights(0));
        for (i, w) in rb.wheels.iter_mut().enumerate() {
            w.spring_factor = self.spring_factor(i / 2, i % 2);
        }
        rb.place(self.position, self.heading);
        self.rigid = Some(rb);
    }

    /// `Axle_Springfactor_<axle>_<side>` as the scripts left it.
    /// The axle that steers: the frontmost, as the rigid body takes it (the simple model
    /// took the first listed, which is not always the front one).
    fn steered_axle(&self) -> usize {
        self.ty.def.axles.iter().enumerate().max_by(|a, b| a.1.long.total_cmp(&b.1.long)).map(|(i, _)| i).unwrap_or(0)
    }

    fn spring_factor(&self, axle: usize, side: usize) -> f32 {
        self.v_springfactor
            .get(axle)
            .and_then(|a| a[side])
            .map(|i| self.state.vars[i as usize])
            .unwrap_or(1.0)
    }

    /// Set the forward speed (m/s), for tests that need a run-up without the road.
    pub fn set_speed(&mut self, v: f32) {
        self.physics.speed = v;
        if let Some(rb) = self.rigid.as_mut() {
            rb.velocity = rb.orientation.mul_vec3(Vec3::Y) * v;
        }
    }

    fn step_physics(&mut self, dt: f32) {
        let c = self.physics.controls;
        self.put(self.v_throttle, c.throttle);
        self.put(self.v_brake, c.brake);
        self.put(self.v_clutch, c.clutch);
        let m_wheel = self.get(self.v_m_wheel);
        // each wheel: its own brake force, and its share of the vehicle's (the rolling
        // resistance's share is the physics' own)
        let n = (self.v_wheels.len() * 2).max(1) as f32;
        let shared = self.get(self.v_brakeforce).max(0.0) / n;
        let brakes: Vec<f32> = self
            .v_wheels
            .iter()
            .flat_map(|a| a.iter().map(|w| self.get(w[4]).max(0.0) + shared))
            .collect();
        // Read, the brake forces go back to 0, as Omsi.exe clears them every frame before
        // the scripts run (0x7e58d9..0x7e5930): a script sets them each frame it brakes. Kept,
        // a mod that brakes only inside an {if} (a retarder, a stop brake, its own physics)
        // stayed braked for good, and one that adds to its own value kept on growing.
        self.put(self.v_brakeforce, 0.0);
        for a in self.v_wheels.clone() {
            for w in a {
                self.put(w[4], 0.0);
            }
        }
        if self.rigid.is_some() {
            self.step_rigid(dt, m_wheel, &brakes, c.steering);
            return;
        }
        let (ds, dheading) = self.physics.step(dt, m_wheel, &brakes);
        // move along the heading
        let prev = (self.position, self.heading);
        let h = self.heading.to_radians();
        self.position.x += h.sin() * ds as f64;
        self.position.y += h.cos() * ds as f64;
        self.heading = (self.heading + dheading as f64).rem_euclid(360.0);
        // collisions: back out of obstacles and stop
        if let (Some(cw), Some(bb)) = (&self.collision, self.ty.def.bounding_box) {
            let obb = crate::collision::Obb::from_box(bb, self.position, self.heading);
            // an obstacle we were already inside before this step (spawned on it, pushed into
            // it) never blocks: only entering an obstacle does
            let prev_obb = crate::collision::Obb::from_box(bb, prev.0, prev.1);
            let hit = cw
                .obstacles_near(&obb)
                .into_iter()
                .chain(self.dynamic_boxes.iter().copied())
                .find(|b| {
                    b.overlaps(&obb)
                        && !b.overlaps(&prev_obb)
                        && !(b.id >= 0 && self.knocked.contains(&b.id))
                });
            if let Some(hit) = hit {
                let v = self.physics.speed;
                if v.abs() > crate::rigid::CRASH_SPEED {
                    let e = 0.5 * self.physics.mass_kg * v * v;
                    let rel = hit.center - self.position.truncate();
                    let (sh, ch) = (h.sin(), h.cos());
                    self.host.coll_pos = [
                        (rel.x * ch - rel.y * sh) as f32,
                        (rel.x * sh + rel.y * ch) as f32,
                        (crate::collision::impact_height(hit.z0.max(obb.z0), hit.z1.min(obb.z1))
                            - self.position.z) as f32,
                    ];
                    // kJ, like the rigid model reports it
                    self.host.coll_energy += e / 1000.0;
                    self.last_crash += e;
                    self.crashes += 1;
                    self.last_impact = e;
                    self.collided = true;
                }
                self.position = prev.0;
                self.heading = prev.1;
                self.physics.speed = 0.0;
            }
        }
        // ground under each wheel: height, terrain pitch and bank of the body
        let (mut z_sum, mut n_z) = (0.0, 0);
        let (mut zf, mut zr, mut zl, mut zrr, mut nf, mut nr, mut nl, mut nrr) =
            (0.0, 0.0, 0.0, 0.0, 0, 0, 0, 0);
        let (sh, ch) = (h.sin(), h.cos());
        let wheelbase = self.physics.wheelbase as f64;
        let mut track = 2.0f64;
        if let Some(g) = &self.ground {
            for axle in &self.physics.wheels {
                for w in axle {
                    let (lat, long) = (w.lat as f64, w.long as f64);
                    track = track.max(w.lat.abs() as f64 * 2.0);
                    let wx = self.position.x + lat * ch + long * sh;
                    let wy = self.position.y - lat * sh + long * ch;
                    if let Some(z) = g(wx, wy) {
                        z_sum += z;
                        n_z += 1;
                        if long > 0.0 {
                            zf += z;
                            nf += 1;
                        } else {
                            zr += z;
                            nr += 1;
                        }
                        if lat < 0.0 {
                            zl += z;
                            nl += 1;
                        } else {
                            zrr += z;
                            nrr += 1;
                        }
                    }
                }
            }
            // the body sits down on its springs like the rigid one does
            let n_axles = self.rest_sag.len().max(1) as f32;
            let lift = self.rest_sag.iter().map(|(c, off)| off - c).sum::<f32>() / n_axles;
            if n_z > 0 {
                self.position.z = z_sum / n_z as f64 + lift as f64;
            } else if let Some(z) = g(self.position.x, self.position.y) {
                self.position.z = z + lift as f64;
            }
        }
        let pitch_terrain = if nf > 0 && nr > 0 {
            ((zf / nf as f64 - zr / nr as f64) / wheelbase)
                .atan()
                .to_degrees()
        } else {
            0.0
        };
        let bank_terrain = if nl > 0 && nrr > 0 {
            ((zl / nl as f64 - zrr / nrr as f64) / track)
                .atan()
                .to_degrees()
        } else {
            0.0
        };
        // dynamic body motion: nose dives when braking, leans out of curves
        let a_long = self.physics.accel.y;
        let curvature = self.physics.steer_deg.to_radians().tan() / self.physics.wheelbase;
        let a_lat = self.physics.speed * self.physics.speed * curvature;
        let pitch_target = pitch_terrain as f32 - a_long * 0.45;
        let bank_target = bank_terrain as f32 - a_lat * 0.6;
        let k = (dt / 0.35).min(1.0);
        self.pitch += (pitch_target - self.pitch) * k;
        self.bank += (bank_target - self.bank) * k;
        // suspension travel per wheel (m): weight transfer from the same accelerations
        for (ai, axle) in self.physics.wheels.iter_mut().enumerate() {
            let rest = self.rest_sag.get(ai).map(|r| r.0).unwrap_or(0.0);
            for w in axle.iter_mut() {
                let front = if w.long > 0.0 { 1.0 } else { -1.0 };
                let right = if w.lat > 0.0 { 1.0 } else { -1.0 };
                // positive = the wheel drops below the body (see step_rigid): braking
                // compresses the front, so its wheels go up (negative)
                let target = -rest + a_long * 0.008 * front - a_lat * 0.01 * right;
                w.suspension += (target - w.suspension) * k;
            }
        }
        let v = self.physics.velocity_kmh();
        self.put(self.v_velocity, v);
        self.put(self.v_velocity_ground, v);
        let n_wheel = self
            .physics
            .wheels
            .iter()
            .find(|w| w[0].driven)
            .or(self.physics.wheels.first())
            .map(|w| w[0].rpm)
            .unwrap_or(0.0);
        self.put(self.v_n_wheel, n_wheel);
        let a = self.physics.accel;
        self.put(self.v_accel[0], a.x);
        self.put(self.v_accel[1], a.y);
        self.put(self.v_accel[2], a.z);
        // Wheel_Rotation_* and Axle_Steering_* are radians in the original: the stock
        // model.cfg turns a wheel with anim_rot ... 57.29577951308232 (180/pi) and the
        // steering wheel with 1450 (SD200) or 1680 (SD202), roughly two and a half turns
        // from lock to lock. Positive is to the right, as here - checked by looking
        // straight down on the front wheel alone (OMSI_ONLY_MESH=SD_Rad_VL) and at the
        // steering wheel in the cab, which turn together with this sign.
        let steer = self.physics.steer_deg.to_radians();
        for (ai, axle) in self.v_wheels.clone().iter().enumerate() {
            for (si, w) in axle.iter().enumerate() {
                let ws = self.physics.wheels[ai][si].clone();
                self.put(w[0], ws.rotation_deg.to_radians());
                self.put(w[1], ws.rpm);
                self.put(w[2], if ai == self.steered_axle() { steer } else { 0.0 });
                self.put(w[3], ws.suspension);
            }
        }
    }

    /// The coupled parts as the leading body feels them at its rear coupling (each along
    /// the direction of the part hung on that coupling; the parts follow kinematically and
    /// pass on what acts on them): drive, brakes, rolling resistance and mass.
    fn coupled_parts(&self) -> Vec<crate::rigid::CoupledPart> {
        let Some(first) = self.trailers.first() else {
            return Vec::new();
        };
        // everything behind comes through the first coupling (the chain passes it on), but
        // each part pulls or holds back along its own heading: the third section of a
        // double articulated bus in a bend does not point where the second does
        let dir_of = |t: &TrailerPart| {
            let h = if t.pivot.is_some() { t.heading } else { self.heading }.to_radians();
            Vec3::new(h.sin() as f32, h.cos() as f32, 0.0)
        };
        self.trailers
            .iter()
            .map(|t| {
                let dir = dir_of(t);
                let def = &t.ty.def;
                let driven: Vec<usize> = def
                    .axles
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| a.driven)
                    .map(|(i, _)| i)
                    .collect();
                let wheel_load = |a: usize| t.rest.get(a).map(|r| r.1).unwrap_or(0.0) * 2.0;
                crate::rigid::CoupledPart {
                    point: first.coupling_back,
                    dir,
                    mass: if def.mass < 100.0 {
                        def.mass * 1000.0
                    } else {
                        def.mass
                    }
                    .max(0.0),
                    driven_wheels: driven.len() * 2,
                    radius: driven
                        .first()
                        .map(|&a| (def.axles[a].wheel_diameter / 2.0).max(0.15))
                        .unwrap_or(0.5),
                    driven_load: driven.iter().map(|&a| wheel_load(a)).sum(),
                    brake: t
                        .v_brakes
                        .iter()
                        .map(|&id| self.get(id).max(0.0))
                        .sum::<f32>()
                        + def.rolling_resistance.max(0.0),
                    load: (0..def.axles.len()).map(wheel_load).sum(),
                }
            })
            .collect()
    }

    fn step_rigid(&mut self, dt: f32, m_wheel: f32, brakes: &[f32], steer: f32) {
        let mut rb = self.rigid.take().unwrap();
        for (i, w) in rb.wheels.iter_mut().enumerate() {
            w.spring_factor = self.spring_factor(i / 2, i % 2);
        }
        rb.coupled = self.coupled_parts();
        {
            let ground = self.ground.clone();
            let flat = |_: f64, _: f64, _: f64| crate::rigid::GroundProbe {
                below: Some(0.0),
                above: None,
            };
            let from_ground = |x: f64, y: f64, top: f64| {
                let z = ground.as_ref().and_then(|g| g(x, y));
                match z {
                    Some(z) if z > top => crate::rigid::GroundProbe {
                        below: None,
                        above: Some(z),
                    },
                    z => crate::rigid::GroundProbe {
                        below: z,
                        above: None,
                    },
                }
            };
            // one session for the whole step: the tyres ask for a few hundred points close
            // together, and it looks the tiles under them up once
            let session = self.contact.as_ref().map(|c| c.session());
            let probe: &dyn Fn(f64, f64, f64) -> crate::rigid::GroundProbe =
                match (&session, &self.ground) {
                    (Some(s), _) => s.as_ref(),
                    (None, Some(_)) => &from_ground,
                    (None, None) => &flat,
                };
            rb.wheel_walls = self.wheel_walls;
            rb.step(dt, m_wheel, brakes, steer, probe);
        }
        // what the crashes of this step destroyed, and the worst of them
        let mut energy = 0.0f32;
        let mut worst: Option<crate::rigid::Impact> = None;
        // a wheel stopped by a face it cannot climb (a platform, a step in the road)
        for hit in &rb.wheel_impacts {
            if hit.speed < crate::rigid::CRASH_SPEED || hit.energy < 1000.0 {
                continue;
            }
            energy += hit.energy;
            if worst.map(|w| hit.energy > w.energy).unwrap_or(true) {
                worst = Some(*hit);
            }
        }
        // Obstacles: the static boxes around the vehicle and the moving ones, answered with
        // impulses. The old answer - put the bus back where it was and take its speed - held
        // it against whatever it touched: it could not slide along a wall or turn away, and
        // every frame of pushing counted as another crash.
        if let Some(bb) = self.ty.def.bounding_box {
            let me = rb.body_box(bb);
            let mut obstacles: Vec<crate::collision::Obb> = Vec::new();
            if let Some(cw) = &self.collision {
                let mut probe_box = me;
                probe_box.half += glam::DVec2::splat(1.0);
                // the faces of collision meshes only where they are in the body as it
                // stands: pitched on a ramp, the plan-view box reaches down to its lowest
                // corner all along its length and ran into the ramp it drove on
                let solid = rb.body_solid(bb, 0.05);
                obstacles.extend(cw.obstacles_near_solid(&probe_box, Some(&solid)));
            }
            obstacles.extend(self.dynamic_boxes.iter().copied());
            let knocked = self.knocked.clone();
            let impacts = rb.collide(
                bb,
                &obstacles,
                &|i| obstacles[i].id >= 0 && knocked.contains(&obstacles[i].id),
                dt,
            );
            let touching: Vec<i64> = impacts
                .iter()
                .map(|h| &obstacles[h.obstacle])
                .filter(|o| o.mass > 0.0)
                .map(|o| o.id)
                .collect();
            for hit in &impacts {
                if hit.broke {
                    let o = &obstacles[hit.obstacle];
                    if o.id >= 0 && !self.knocked.contains(&o.id) {
                        self.knocked.push(o.id);
                        self.knocked_now.push((o.id, hit.push.as_dvec3()));
                    }
                }
                // a brush that takes less than a kilojoule is no accident (a broken post is), and
                // a car still inside the bus from the frame before is the same accident
                let o = &obstacles[hit.obstacle];
                if hit.speed >= crate::rigid::CRASH_SPEED && omsi_cfg::env::var_os("OMSI_DEBUG_PHYSICS").is_some() {
                    log::info!("  hit obstacle {} at ({:.1}, {:.1}) z {:.2}..{:.2} half {:.2}x{:.2} heading {:.0}, {:.1} km/h", o.id, o.center.x, o.center.y, o.z0, o.z1, o.half.x, o.half.y, o.heading.to_degrees(), hit.speed * 3.6);
                }
                if hit.speed < crate::rigid::CRASH_SPEED
                    || (hit.energy < 1000.0 && !hit.broke)
                    || (o.mass > 0.0 && self.touching.contains(&o.id))
                {
                    continue;
                }
                energy += hit.energy;
                if worst.map(|w| hit.energy > w.energy).unwrap_or(true) {
                    worst = Some(*hit);
                }
            }
            self.touching = touching;
        }
        if let Some(hit) = worst {
            if omsi_cfg::env::var_os("OMSI_DEBUG_PHYSICS").is_some() {
                log::info!(
                    "impact at {:.1} km/h, {:.1} kJ, body point ({:.2}, {:.2}, {:.2}){}",
                    hit.speed * 3.6,
                    energy / 1000.0,
                    hit.point.x,
                    hit.point.y,
                    hit.point.z,
                    if hit.broke { ", a post broke off" } else { "" }
                );
            }
            // `coll_energy` is in kJ: the stock and mod collision scripts compare it with
            // 100-1000 for the electrics and 1-10 for a lamp - in joules a bump at
            // walking pace took the electrics out
            self.host.coll_pos = hit.point.to_array();
            self.host.coll_energy += energy / 1000.0;
            self.last_crash += energy;
            self.last_impact = energy;
            self.crashes += 1;
            self.collided = true;
        }
        self.position = rb.origin();
        let (heading, pitch, bank) = rb.heading_pitch_bank();
        self.heading = heading;
        self.pitch = pitch;
        self.bank = bank;
        // script interface
        let speed = rb.forward_speed();
        self.physics.speed = speed;
        self.physics.steer_deg = rb.steer_deg;
        self.physics.accel = rb.accel_body;
        let v = speed * 3.6;
        self.put(self.v_velocity, v);
        self.put(self.v_velocity_ground, v);
        let n_wheel = rb
            .wheels
            .iter()
            .find(|w| w.driven)
            .or(rb.wheels.first())
            .map(|w| w.rpm)
            .unwrap_or(0.0);
        self.put(self.v_n_wheel, n_wheel);
        self.put(self.v_accel[0], rb.accel_body.x);
        self.put(self.v_accel[1], rb.accel_body.y);
        self.put(self.v_accel[2], rb.accel_body.z);
        for (ai, axle) in self.v_wheels.clone().iter().enumerate() {
            for (si, w) in axle.iter().enumerate() {
                if let Some(rw) = rb.wheels.get(ai * 2 + si) {
                    self.put(w[0], rw.rotation_deg.to_radians());
                    self.put(w[1], rw.rpm);
                    // (each axle's own angle: OMSI turns every axle towards the centre of
                    // the bend on the `[rot_pnt_long]` line)
                    self.put(w[2], rw.steer);
                    // `Axle_Suspension_*` is the wheel's travel *relative to the body*, and
                    // the stock model.cfg moves the wheel down for a positive value
                    // (`origin_rot_y -90` + `anim_trans`, checked with OMSI_DEBUG_ANIM on
                    // the SD200's front wheel). A compressed spring means the body has
                    // come down towards the wheel, i.e. the wheel sits *up* in its arch:
                    // the value is minus the compression. Handing over the compression
                    // itself pushed every wheel down by twice its travel - into the road
                    // under braking, with the body riding high above the arches.
                    // (never below where the spring is unloaded: Omsi.exe hands over
                    // -clamp(travel, 0, maxforce / k), 0x7e4afa - a wheel in the air stays
                    // where it hangs at rest)
                    let shown = -rw.compression.max(0.0);
                    self.put(w[3], shown);
                    if let Some(ws) = self.physics.wheels.get_mut(ai).and_then(|a| a.get_mut(si)) {
                        ws.rotation_deg = rw.rotation_deg;
                        ws.rpm = rw.rpm;
                        ws.suspension = shown;
                    }
                }
            }
        }
        self.rigid = Some(rb);
    }

    pub fn var(&self, name: &str) -> Option<f32> {
        self.var_index
            .get(&name.to_ascii_lowercase())
            .map(|&i| self.state.vars[i as usize])
    }

    /// The name of script variable `index` (lower case), for diagnostics and key helpers.
    pub fn var_name(&self, index: usize) -> Option<&str> {
        self.var_index
            .iter()
            .find(|(_, &i)| i as usize == index)
            .map(|(n, _)| n.as_str())
    }

    pub fn set_var(&mut self, name: &str, v: f32) -> bool {
        match self.var_index.get(&name.to_ascii_lowercase()) {
            Some(&i) => {
                self.state.vars[i as usize] = v;
                true
            }
            None => false,
        }
    }

    /// Variables the engine feeds the model directly (`Dirt_Norm`, `AI`, …): OMSI's
    /// varlists rarely declare them, and an undeclared `[alphascale]` variable used to
    /// read as 1.0 - every bus wore its dirt film at full strength from the first frame.
    pub fn set_engine_var(&mut self, name: &str, v: f32) {
        if !self.set_var(name, v) {
            let id = self.state.vars.len() as omsi_script::VarId;
            self.state.vars.push(v);
            self.var_index.insert(name.to_ascii_lowercase(), id);
        }
    }

    pub fn trigger(&mut self, name: &str) -> bool {
        let p = self.ty.program.clone();
        self.vm
            .run_trigger(&p, name, &mut self.state, &mut self.host)
    }

    /// Dirt and spray. OMSI writes three engine variables every frame and the bus scripts
    /// turn them into what you see: `Dirt_Norm` is how dirty the body is (the `[alphascale]`
    /// of the dirt overlay), `DirtRate` how fast the windscreen is soiling right now
    /// (`dirt.osc` adds it up into `Dirt_Wiped`, which the wipers clear), and `PrecipRate`
    /// is signed, so rain wets the glass in `rain.osc` and dry weather dries it again.
    fn update_dirt(&mut self, dt: f32) {
        let rain = self.host.precip_rate.clamp(0.0, 1.0);
        let speed = (self.physics.velocity_kmh().abs() / 50.0).min(2.0);
        // a dry duty adds a few percent, a wet one soils the bus in a couple of hours
        // a dry duty of a couple of hours shows; a wet one soils the bus in half an hour
        self.dirt = (self.dirt + dt * speed * (8.0e-5 + 1.2e-3 * rain)).clamp(0.0, 1.0);
        self.set_engine_var("Dirt_Norm", self.dirt);
        // spray off a wet road, dust off a dry one
        self.set_engine_var("DirtRate", speed * (1.5e-4 + 6.0e-3 * rain));
        // rain soaks the glass in a few seconds; without it the film dries in about a minute.
        // Snow does not run down a pane and the cab is warm: the flakes that land on the
        // glass melt and leave a light haze, nothing like the film a shower leaves, so the
        // rate is steered towards that haze instead of driving the layer to full strength
        // (`rain.osc` only ever adds `PrecipRate * Timegap` and clamps at 1 - it never asks
        // what is falling, which is why the original shows raindrops in a snowstorm).
        let rate = if self.host.precip_type as i32 == 2 {
            let film = self.var("Rain_Window_Norm_Wetness").unwrap_or(0.0);
            (SNOW_ON_GLASS * rain - film) * 0.5
        } else if rain > 0.0 {
            rain * 0.25
        } else {
            -0.02
        };
        self.set_engine_var("PrecipRate", rate);
        // the state of the road, for the tyre sounds and the wheel spray
        self.set_engine_var("StreetCond", self.host.street_cond);
        // what is coupled to this vehicle (the train scripts light their ends by it: a head
        // lamp only where nothing is coupled, the tail lamps at the last car)
        self.set_engine_var("train_frontcoupling", 0.0);
        self.set_engine_var("train_backcoupling", if self.trailers.is_empty() { 0.0 } else { 1.0 });
        self.set_engine_var("train_me_reverse", 0.0);
        // and for the tyres' grip
        let mu = road_grip(self.host.street_cond, self.host.temperature);
        if let Some(rb) = self.rigid.as_mut() {
            rb.friction = mu;
        }
    }

    /// The engine's own vehicle variables that OMSI binds to fields of the vehicle
    /// (`TRoadVehicle.virtual_00` registers them, the original points each at its field, and
    /// the original keeps the fields up to date): the odometer (`kmcounter_km` whole
    /// kilometres, `kmcounter_m` the metres of the fractions), the people aboard, whether a
    /// timetable is driven, the precipitation type, and the cabin air - kept within reach of
    /// the weather (never more than 10 °C from the air outside and pulled towards 18..25 °C
    /// as a heated/ventilated bus is; the absolute humidity follows the outside air).
    fn update_engine_vars(&mut self, dt: f32) {
        self.update_driver_seat(dt);
        // `[kmcounter_init] year km`: in service since that year, so many kilometres a year -
        // the odometer starts at what that comes to on the day driven (it stood at 0 on
        // every bus that has one, #305), a little different from bus to bus of the kind
        if !self.km_started {
            self.km_started = true;
            if let (Some((year, per_year)), true) = (self.ty.def.km_counter_init, self.host.km_base == 0.0) {
                // (a map whose day lies before the bus was built: the part of this year)
                let part = self.host.clock.day_of_year as f64 / 365.0;
                let years = ((self.host.clock.year - year) as f64 + part).max(part);
                if years > 0.0 && per_year > 0.0 {
                    let seed = (std::ptr::addr_of!(self.host) as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 40;
                    let spread = 0.85 + 0.3 * (seed % 1000) as f64 / 1000.0;
                    self.host.km_base = years * per_year as f64 * spread;
                }
            }
        }
        // (the sum is split, not the parts: 0.7 km + 0.5 km is 1 km 200 m, not 0 km 1200 m)
        let total = self.host.km_base + self.driven_km;
        self.set_engine_var("kmcounter_km", total.trunc() as f32);
        self.set_engine_var("kmcounter_m", (total.fract() * 1000.0) as f32);
        self.set_engine_var("humans_count", self.host.humans_count);
        self.set_engine_var("schedule_active", self.host.schedule_active);
        self.set_engine_var("precipType", self.host.precip_type);
        // A bus whose scripts keep their own cabin air (`heizung.osc` of the stock buses and
        // their descendants: they integrate the temperature from heat flows) is left to
        // them; only the relative humidity comes from their temperature and humidity. An
        // engine model on top of theirs broke that balance - the Sprinter 312D's cabin
        // went to infinity within a second, and its engine with it.
        let owns = |name: &str| self.ty.program.var(name).is_some_and(|v| self.ty.program.stores(v));
        if owns("Cabinair_Temp") {
            if let (Some(t), Some(hum)) = (self.var("Cabinair_Temp"), self.var("Cabinair_absHum")) {
                if !owns("Cabinair_relHum") && t.is_finite() && t > -237.0 {
                    self.set_engine_var("Cabinair_relHum", relative_humidity(t, hum));
                }
            }
            return;
        }
        let out = self.host.temperature;
        let (mut t, mut hum, last) = self.cabin_air.unwrap_or((out.clamp(18.0, 25.0), 9.0, f32::NAN));
        // a script of the bus's own wrote the cabin temperature: that is what it is now
        if let Some(cur) = self.var("Cabinair_Temp") {
            if !last.is_nan() && (cur - last).abs() > 1e-4 {
                t = cur;
            }
        }
        let lower = (out + 10.0).min(18.0).max(out - 10.0);
        let upper = (out + 20.0).min(out.max(25.0)).max(lower);
        let target = t.clamp(lower, upper);
        t += (target - t) * (dt / 120.0).min(1.0);
        let out_hum = if self.host.abs_humidity > 0.0 { self.host.abs_humidity } else { 9.0 };
        hum += (out_hum - hum) * (dt / 300.0).min(1.0);
        // relative humidity: against the saturation (Magnus) at the cabin temperature

        self.set_engine_var("Cabinair_Temp", t);
        self.set_engine_var("Cabinair_absHum", hum);
        self.set_engine_var("Cabinair_relHum", relative_humidity(t, hum));
        self.cabin_air = Some((t, hum, t));
    }

    /// Fire a trigger and read the number the script leaves on the stack. OMSI asks the
    /// bus how long its repairs take this way (`malfunction_gettime`): every damaged system
    /// adds its minutes to the value passing through the chain of macros.
    pub fn trigger_value(&mut self, name: &str) -> Option<f32> {
        let p = self.ty.program.clone();
        self.vm.stacks = Default::default();
        if !self
            .vm
            .run_trigger(&p, name, &mut self.state, &mut self.host)
        {
            return None;
        }
        Some(self.vm.stacks.st[0])
    }

    /// Run one of the engine's service triggers with `secs` on the clock: OMSI holds
    /// `veh_tank` / `veh_wash` down while the pump or the wash runs and the bus script
    /// decides what a second of it is worth (the SD202 takes 3 litres and caps at 250).
    fn service(&mut self, name: &str, secs: f32) -> bool {
        let keep = self.host.clock.timegap;
        self.host.clock.timegap = secs;
        let ok = self.trigger(name);
        self.host.clock.timegap = keep;
        ok
    }

    /// Refuel until the tank stops filling. Returns the tank content the script reports.
    pub fn refuel(&mut self) -> Option<f32> {
        let mut last = f32::NEG_INFINITY;
        for _ in 0..600 {
            if !self.service("veh_tank", 1.0) {
                return None;
            }
            let now = self.var("engine_tank_content")?;
            if now <= last + 1e-3 {
                break;
            }
            last = now;
        }
        self.var("engine_tank_content")
    }

    /// Run the bus through the wash until it stops getting cleaner.
    pub fn wash(&mut self) -> Option<f32> {
        self.dirt = 0.0;
        self.set_engine_var("Dirt_Norm", 0.0);
        let mut last = f32::INFINITY;
        for _ in 0..600 {
            if !self.service("veh_wash", 1.0) {
                return None;
            }
            let now = self.var("Dirt_Wiped")?;
            if now >= last - 1e-4 {
                break;
            }
            last = now;
        }
        self.var("Dirt_Wiped")
    }

    /// Minutes the workshop needs for the damage the bus has right now.
    pub fn repair_minutes(&mut self) -> Option<f32> {
        self.trigger_value("malfunction_gettime")
    }

    /// Repair everything (the engine fires this once the driver accepts the waiting time).
    pub fn repair(&mut self) -> bool {
        self.trigger("malfunction_reset")
    }

    /// Brightest `[interiorlight]` of the vehicle right now (0..1), for the passengers.
    pub fn interior_light(&self) -> f32 {
        self.ty
            .model
            .interior_lights
            .iter()
            .map(|il| {
                il.variable
                    .trim()
                    .parse::<f32>()
                    .ok()
                    .or_else(|| self.var(&il.variable))
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0)
                    * il.range.clamp(0.0, 1.0)
            })
            .fold(0.0, f32::max)
    }

    /// The saloon lamps' light at a point of the model frame (0..1): each lit
    /// `[interiorlight]` as the Direct3D point light OMSI makes of it (full within its
    /// range, falling with the square of the distance beyond), summed and saturated. Unlike
    /// [`Self::interior_light`] a lamp at the far end of the saloon hardly reaches the point,
    /// and a lamp that is off gives nothing.
    pub fn interior_light_at(&self, p: Vec3) -> f32 {
        self.ty
            .model
            .interior_lights
            .iter()
            .map(|il| {
                let on = il.variable.trim().parse::<f32>().ok().or_else(|| self.var(&il.variable)).unwrap_or(0.0).clamp(0.0, 1.0);
                if on <= 0.0 {
                    return 0.0;
                }
                let core = il.range.max(0.05);
                let d2 = (Vec3::from(il.pos) - p).length_squared().max(1e-4);
                on * (core * core / d2).min(1.0)
            })
            .sum::<f32>()
            .min(1.0)
    }

    /// Run one simulation frame: physics, scripts, then animations.
    pub fn update(&mut self, dt: f32) {
        self.host.clock.advance(dt);
        self.step_physics(dt);
        self.update_ground_probe();
        if std::mem::take(&mut self.collided) {
            // OMSI runs the vehicle's `collision` block on a crash (it damages the bus); the
            // energy is that crash's, for as many reads as the block makes
            self.trigger("collision");
            self.host.coll_energy = 0.0;
        }
        self.update_dirt(dt);
        self.driven_km += (self.physics.velocity_kmh().abs() as f64 / 3600.0) * dt as f64;
        self.update_engine_vars(dt);
        let p = self.ty.program.clone();
        self.vm.run_frame(&p, &mut self.state, &mut self.host);
        self.clear_pax_requests();
        self.update_visuals(dt);
    }

    /// The passengers' door requests are pulses: Omsi.exe clears all eight of each kind
    /// after the vehicle's scripts ran (0x7d6214) and the passengers set them again every
    /// frame. Kept, a timetable bus that drove out of the passengers' reach kept its last
    /// request, and its automatic door never shut.
    fn clear_pax_requests(&mut self) {
        for &id in &self.v_pax_req {
            self.state.vars[id as usize] = 0.0;
        }
    }

    /// Run the scripts' frame once with no time passing (no physics, no clock): what a
    /// switch just pressed makes of the variables the scripts derive from it.
    pub fn update_scripts_only(&mut self, _dt: f32) {
        let p = self.ty.program.clone();
        let gap = self.host.clock.timegap;
        self.host.clock.timegap = 0.0;
        self.vm.run_frame(&p, &mut self.state, &mut self.host);
        self.host.clock.timegap = gap;
    }

    /// Let the scripts measure the ground under a point of the vehicle
    /// (`GetHeightAbovePoint`): the probe captures this frame's pose and ground sampler.
    /// The answer is how high the point stands *over* the ground, positive with room below:
    /// the Solaris' kneeling sensor stops lowering the body "as soon as it touches the
    /// kerb" when it reads under 5 cm and lets the level control vent the right-hand
    /// bellows only above that, and the NL/NG lift may drop by what it reads (up to
    /// 0.3 m). Read the other way round (ground minus point) the sensor sat at 0, the
    /// right-hand bellows of an Urbino never let their air out and the bus leant 1.7° to
    /// the left all the way.
    fn update_ground_probe(&mut self) {
        if self.ground.is_none() {
            return;
        }
        let rot = self.body_rotation();
        let pos = self.position;
        let g = self.ground.clone().unwrap();
        self.host.ground_probe = Some(std::sync::Arc::new(move |x, y, z| {
            let w = pos + rot.transform_point3(glam::Vec3::new(x, y, z)).as_dvec3();
            match g(w.x, w.y) {
                Some(h) => (w.z - h) as f32,
                None => 0.0,
            }
        }));
    }

    /// One frame of an AI-controlled vehicle: the pose is set by the traffic system
    /// (`ai_motion::AiBody`, which also leaves the accelerations and each wheel's travel in
    /// `physics`), the scripts see the motion through `Velocity` and the `AI_*` variables and
    /// run their `{frame_ai}` blocks (falling back to `{frame}` when a script has none).
    pub fn update_ai(&mut self, dt: f32, ai: &AiFrame) {
        self.update_ai_with(dt, ai, &[], &[]);
    }

    /// `update_ai` for the copy of a vehicle another game drives (LAN play): `inputs` are
    /// written after the `AI_*` variables (so they may override `AI_Engine` or `AI_Light`)
    /// and before the scripts run; `pinned` are written before and once more after the
    /// scripts, so the animations, lamps and sounds show the other game's values whatever
    /// this copy's AI scripts made of them. The odometer may run backwards (reversing).
    pub fn update_ai_with(
        &mut self,
        dt: f32,
        ai: &AiFrame,
        inputs: &[(omsi_script::VarId, f32)],
        pinned: &[(omsi_script::VarId, f32)],
    ) {
        self.host.clock.advance(dt);
        self.physics.speed = ai.speed;
        self.physics.steer_deg = ai.steer_deg;
        let v_kmh = ai.speed * 3.6;
        self.put(self.v_velocity, v_kmh);
        self.put(self.v_velocity_ground, v_kmh);
        // how far the vehicle rolled since the last frame (a reset or a jump counts as none;
        // backwards only for a vehicle that reverses)
        let ds = ai.odometer - self.ai_odometer;
        self.ai_odometer = ai.odometer;
        let ds = if (0.0..50.0).contains(&ds) || (ai.speed < 0.0 && ds > -50.0 && ds < 0.0) {
            ds
        } else {
            0.0
        };
        // `steer_deg` is the front wheel angle of a bicycle model turning about the
        // `[rot_pnt_long]` line: every wheel ahead of that line points at the common turning
        // centre (the inner one further than the outer, as with Ackermann steering), and
        // every wheel rolls as far as its own track through the bend is long
        let (rot, wheelbase) = crate::ai_motion::rotation_point(&self.ty.def);
        let k = ai.steer_deg.to_radians().tan() / wheelbase;
        for (ai_idx, axle) in self.v_wheels.clone().iter().enumerate() {
            for (si, w) in axle.iter().enumerate() {
                let Some(ws) = self.physics.wheels.get_mut(ai_idx).map(|a| &mut a[si]) else {
                    continue;
                };
                let arm = ws.long - rot;
                let across = 1.0 - ws.lat * k;
                let steer = if arm > 0.5 {
                    (arm * k).atan2(across).clamp(-1.2, 1.2)
                } else {
                    0.0
                };
                let travel = (across * across + (arm * k) * (arm * k)).sqrt();
                let r = ws.radius.max(0.05);
                ws.rotation_deg =
                    (ws.rotation_deg + (ds * travel / r).to_degrees()).rem_euclid(360.0);
                ws.rpm = ai.speed * travel / (std::f32::consts::TAU * r) * 60.0;
                let (rotation, rpm, suspension) =
                    (ws.rotation_deg.to_radians(), ws.rpm, ws.suspension);
                self.put(w[0], rotation);
                self.put(w[1], rpm);
                self.put(w[2], steer);
                // the travel `AiBody` works out on its springs, about the static sag of an AI
                // copy (`ai_rest_offset`, already in it)
                self.put(w[3], suspension);
            }
        }
        let n_wheel = self
            .physics
            .wheels
            .iter()
            .find(|w| w[0].driven)
            .or(self.physics.wheels.first())
            .map(|w| w[0].rpm)
            .unwrap_or(0.0);
        self.put(self.v_n_wheel, n_wheel);
        let a = self.physics.accel;
        self.put(self.v_accel[0], a.x);
        self.put(self.v_accel[1], a.y);
        self.put(self.v_accel[2], a.z);
        // `AI` is the engine's "this vehicle is driven by the AI" flag; the police and
        // ambulance script sounds the siren whenever `AI` is 0 and the car rolls, which is
        // why every spawn came with an ambulance nobody could see
        // `AI_Scheduled_AtStation`: 1 while the bus serves a stop (the door scripts open the
        // doors), then -1 when it wants to leave - the stock `door_X10_AI.osc` closes the
        // doors on -1, releases the stop brake and only then writes 0, which is the signal
        // that the bus may move (`VehicleInstance::station_released`). Writing 0 straight
        // away left every timetable bus driving off with its doors open. A frame that says
        // nothing (0) keeps telling a script that has not answered yet to close.
        let station = match ai.at_station {
            1 => 1.0,
            -1 => -1.0,
            _ => match self.var("AI_Scheduled_AtStation") {
                Some(v) if v.abs() > 0.5 => -1.0,
                _ => 0.0,
            },
        };
        for (name, v) in [
            ("AI", 1.0),
            ("AI_Blinker_L", matches!(ai.blinker, 1 | 3) as i32 as f32),
            ("AI_Blinker_R", matches!(ai.blinker, 2 | 3) as i32 as f32),
            ("AI_Brakelight", ai.brake as i32 as f32),
            ("AI_Light", ai.lights as i32 as f32),
            // (the engine's field +0x638: an AI bus lights its saloon when it drives with
            // its lights on - the LiAZ's `lights_AI` switches both saloon circuits on it)
            ("AI_Interiorlight", ai.lights as i32 as f32),
            ("AI_Engine", 1.0),
            ("AI_Scheduled_AtStation", station),
            ("TrafficPriorityWarningNeeded", ai.priority_warning as i32 as f32),
        ] {
            self.set_var(name, v);
        }
        for &(id, v) in inputs.iter().chain(pinned) {
            self.put(Some(id), v);
        }
        let p = self.ty.program.clone();
        if p.frame_ai.is_empty() {
            self.vm.run_frame(&p, &mut self.state, &mut self.host);
        } else {
            self.vm.run_frame_ai(&p, &mut self.state, &mut self.host);
        }
        self.clear_pax_requests();
        for &(id, v) in pinned {
            self.put(Some(id), v);
        }
        if self.ai_visuals {
            let dt = dt + std::mem::take(&mut self.ai_visuals_missed);
            self.update_visuals(dt);
        } else {
            // the coupled parts follow all the same (other cars keep clear of them)
            self.update_trailers(dt);
            self.ai_visuals_missed = (self.ai_visuals_missed + dt).min(10.0);
        }
    }

    /// The `[smoothskin]` meshes (the bellows of an articulated bus).
    pub fn skinned_meshes(&self) -> Vec<usize> {
        (0..self.ty.meshes.len())
            .filter(|&i| !self.ty.meshes[i].skin.is_empty())
            .collect()
    }

    /// The transforms a skinned mesh's shape depends on (its own and its bones'): when they
    /// are unchanged, so is the shape.
    pub fn skin_key(&self, i: usize) -> Vec<Mat4> {
        skin_key(&self.ty, i, &self.mesh_transforms)
    }

    /// Skinned mesh `i` as its bones hold it now (see [`skin_vertices`]).
    pub fn skinned(&mut self, i: usize) -> Option<(Vec<Vec3>, Vec<Vec3>)> {
        if self.skin_rest.len() != self.animators.len() {
            self.skin_rest = rest_transforms(&self.animators, self.state.vars.len());
        }
        skin_vertices(&self.ty, i, &self.mesh_transforms, &self.skin_rest)
    }

    /// Has the bus's script finished leaving its stop (doors shut, stop brake off)? True
    /// for vehicles whose scripts do not take part in that handshake.
    pub fn station_released(&self) -> bool {
        self.var("AI_Scheduled_AtStation")
            .map(|v| v > -0.5)
            .unwrap_or(true)
    }

    /// Seat the drawn tyres on the hubs the physics has: a model whose suspension
    /// animation lifts its wheels by less (or more) than the spring is compressed - the
    /// LiAZ 5292 turns them about pivots across the bus, and at rest they stood 4 cm in the
    /// asphalt, like flat tyres - has each tyre mesh moved up or down to where its hub is.
    /// Only small corrections (under 8 cm) and only axles the model moves with
    /// `Axle_Suspension`: a larger mismatch is the model's own business.
    fn seat_wheels(&mut self) {
        let Some(rb) = self.rigid.as_ref() else { return };
        if self.wheel_seats.is_none() {
            let mut seats = Vec::new();
            for (i, vm) in self.ty.meshes.iter().enumerate() {
                let def = &self.ty.model.meshes[vm.def_index];
                let Some(an) = def.animations.iter().find(|a| a.variable.to_ascii_lowercase().starts_with("wheel_rotation_")) else { continue };
                let v = an.variable.to_ascii_lowercase();
                let rest = v.trim_start_matches("wheel_rotation_");
                let mut parts = rest.split('_');
                let (Some(Ok(axle)), Some(side)) = (parts.next().map(|a| a.parse::<usize>()), parts.next()) else { continue };
                if !self.ty.suspension_axles.contains(&axle) {
                    continue;
                }
                let k = axle * 2 + if side.starts_with('r') { 1 } else { 0 };
                if k >= rb.wheels.len() {
                    continue;
                }
                seats.push((i, k, vm.pivot.w_axis.truncate()));
            }
            self.wheel_seats = Some(seats);
        }
        let seats = self.wheel_seats.as_ref().unwrap();
        for &(i, k, pivot) in seats {
            let comp = rb.wheels[k].compression.clamp(-crate::rigid::DROOP, crate::rigid::BUMP);
            let drawn = self.mesh_transforms[i].transform_point3(pivot).z;
            let dz = pivot.z + comp - drawn;
            if k == 0 && omsi_cfg::env::var_os("OMSI_DEBUG_SEAT").is_some() {
                log::info!("seat mesh {i} wheel {k}: comp {:.4} drawn {:.4} pivot {:.4} dz {:.4}", comp, drawn, pivot.z, dz);
            }
            // (every frame, however small: a dead band of 3 mm had the correction switch on
            // and off as the travel crossed it, and the tyre ticked up and down by that much)
            if dz.abs() < 0.08 {
                self.mesh_transforms[i] = Mat4::from_translation(Vec3::new(0.0, 0.0, dz)) * self.mesh_transforms[i];
            }
        }
    }

    /// Coupled vehicles follow and animate from this vehicle's variables.
    fn update_trailers(&mut self, dt: f32) {
        let mut trailers = std::mem::take(&mut self.trailers);
        let mut lead: Option<(DVec3, Mat4, f64)> = None;
        for t in &mut trailers {
            t.update(self, dt, lead);
            lead = Some((t.position, t.body_rotation(), t.heading));
        }
        self.trailers = trailers;
    }

    /// Put the coupled parts on a track: `at_behind(d)` is the point of the track `d` metres
    /// behind the vehicle's origin (None: not known, the part follows as it does on a road).
    /// Each part's turning axle is placed there, and the part follows its coupling.
    pub fn retrail(&mut self, dt: f32, at_behind: &dyn Fn(f64) -> Option<DVec3>) {
        let mut trailers = std::mem::take(&mut self.trailers);
        let mut lead: Option<(DVec3, Mat4, f64)> = None;
        let (mut prev_c, mut b_prev) = (self.position, 0.0f64);
        for t in &mut trailers {
            let (lp, lr) = lead.map(|(p, r, _)| (p, r)).unwrap_or((self.position, self.body_rotation()));
            let c = t.coupling_point(lp, lr);
            let b_c = b_prev + (c - prev_c).truncate().length();
            if let Some(p) = at_behind(b_c + t.pivot_length() as f64) {
                t.place_on_track(p);
            }
            t.update(self, dt, lead);
            lead = Some((t.position, t.body_rotation(), t.heading));
            prev_c = c;
            b_prev = b_c;
        }
        self.trailers = trailers;
    }

    /// Animations and per-mesh material properties after the scripts ran.
    fn update_visuals(&mut self, dt: f32) {
        // coupled vehicles come first, because they write the joint's angles this vehicle's
        // plates and bellows turn with (a frame late, the joint lagged behind the rear section)
        self.update_trailers(dt);
        for (i, a) in self.animators.iter_mut().enumerate() {
            self.mesh_transforms[i] = a.update(dt, &self.state.vars);
        }
        crate::anim::apply_parents(&self.animators, &mut self.mesh_transforms);
        self.seat_wheels();
        static DEBUG_ANIM: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        if let Some(want) = DEBUG_ANIM.get_or_init(|| omsi_cfg::env::var("OMSI_DEBUG_ANIM").ok()) {
            for (i, m) in self.ty.meshes.iter().enumerate() {
                let file = &self.ty.model.meshes[m.def_index].file;
                if file
                    .to_ascii_lowercase()
                    .contains(&want.to_ascii_lowercase())
                {
                    let a = &self.animators[i];
                    let vals: Vec<(String, f32, f32, [f32; 3], [f32; 3])> = a
                        .anims
                        .iter()
                        .map(|(d, o, st)| {
                            (
                                d.variable.clone(),
                                st.var
                                    .map(|v| self.state.vars[v as usize])
                                    .unwrap_or(f32::NAN),
                                st.value,
                                o.x_axis.truncate().to_array(),
                                o.w_axis.truncate().to_array(),
                            )
                        })
                        .collect();
                    log::info!(
                        "anim {file}: {vals:?} transform {:?}",
                        self.mesh_transforms[i]
                            .to_cols_array()
                            .map(|v| (v * 100.0).round() / 100.0)
                    );
                }
            }
        }
        self.props_plan.refresh(&self.ty, &self.var_index);
        self.props_plan
            .apply(&self.state.vars, &mut self.mesh_props);
        // Dirt overlays are engine-owned layers. A model may declare Dirt_Norm or
        // Dirt_Wiped in its cfg without putting the name in the script varlist; the
        // generic alphascale fallback is 1.0 in that case, which makes Dreck.tga an
        // opaque full-body decal. Resolve these values explicitly for every vehicle.
        for (i, vm) in self.ty.meshes.iter().enumerate() {
            let def = &self.ty.model.meshes[vm.def_index];
            for m in &def.materials {
                let Some(name) = m.alphascale.as_deref().map(str::trim) else {
                    continue;
                };
                // (compared in place: a lower-case copy per material and frame of every AI
                // vehicle was a steady stream of allocations)
                let value = if name.eq_ignore_ascii_case("dirt_norm") {
                    Some(self.dirt)
                } else if name.eq_ignore_ascii_case("dirt_wiped") {
                    Some(self.var("Dirt_Wiped").unwrap_or(0.0))
                } else {
                    None
                };
                let Some(value) = value else { continue };
                if let Some(slot) = override_slot(&vm.materials, m) {
                    if let Some(alpha) = self.mesh_props[i].slot_alpha.get_mut(slot) {
                        *alpha = value.clamp(0.0, 1.0);
                    }
                }
            }
        }
        // the lamps come on and go out with their `timeconst`
        let mut lf = std::mem::take(&mut self.light_fade);
        let mut part_fades: Vec<Vec<f32>> = self.trailers.iter_mut().map(|t| std::mem::take(&mut t.light_fade)).collect();
        let value = |n: &str| -> f32 { n.trim().parse::<f32>().ok().or_else(|| self.var(n.trim())).unwrap_or(0.0) };
        let fade = |model: &omsi_model::Model, fades: &mut Vec<f32>| {
            let mut k = 0;
            for md in &model.meshes {
                for l in &md.light_enh_2 {
                    let target = (value(&l.variable) * if l.factor > 0.0 { l.factor } else { 1.0 }).clamp(0.0, 2.0);
                    // (a NaN stays NaN through clamp: the lamp's glow was drawn black on Windows)
                    let target = if target.is_finite() { target } else { 0.0 };
                    if fades.len() <= k {
                        fades.push(target);
                    }
                    let b = &mut fades[k];
                    if !b.is_finite() {
                        *b = target;
                    }
                    *b = if l.time_const <= 0.001 {
                        target
                    } else {
                        let rate = if target > *b { 1.0 } else { 1.31 } / l.time_const;
                        *b + (target - *b) * (1.0 - (-dt * rate).exp())
                    };
                    k += 1;
                }
            }
        };
        fade(&self.ty.model, &mut lf);
        for (t, f) in self.trailers.iter().zip(part_fades.iter_mut()) {
            fade(&t.ty.model, f);
        }
        self.light_fade = lf;
        for (t, f) in self.trailers.iter_mut().zip(part_fades) {
            t.light_fade = f;
        }
        // the particle systems ([smoke]: exhaust, boiling coolant, wheel spray) of the
        // vehicle and its coupled parts, which read the same scripts' variables
        if !self.particles.is_empty() || self.trailers.iter().any(|t| !t.particles.is_empty()) {
            let mut ps = std::mem::take(&mut self.particles);
            let mut parts: Vec<ParticleSet> = self.trailers.iter_mut().map(|t| std::mem::take(&mut t.particles)).collect();
            {
                let value = |n: &str| self.var(n).unwrap_or(0.0);
                ps.update(dt, self.position, self.body_rotation(), &value);
                for (t, set) in self.trailers.iter().zip(parts.iter_mut()) {
                    set.update(dt, t.position, t.body_rotation(), &value);
                }
            }
            self.particles = ps;
            for (t, set) in self.trailers.iter_mut().zip(parts) {
                t.particles = set;
            }
        }
    }

    /// How far each tyre of the model (and of the coupled parts) stands above (+) or sinks
    /// into (-) what is under it, in metres, from the meshes as they are animated right now:
    /// the check that the wheels touch the road. (mesh file, gap). An AI type keeps no
    /// vertices on the CPU, so its wheel meshes are read again ([`VehicleType::mesh_data`]).
    pub fn wheel_ground_gaps(&self) -> Vec<(String, f32)> {
        let under = |x: f64, y: f64, top: f64| -> Option<f64> {
            match (&self.contact, &self.ground) {
                (Some(c), _) => c.probe(x, y, top).below,
                (None, Some(g)) => g(x, y),
                _ => None,
            }
        };
        let is_wheel = |ty: &VehicleType, def_index: usize| {
            ty.model.meshes[def_index].animations.iter().any(|a| {
                a.variable
                    .to_ascii_lowercase()
                    .starts_with("wheel_rotation_")
            })
        };
        let mut out = Vec::new();
        let mut measure = |ty: &VehicleType, origin: DVec3, xf: &dyn Fn(usize) -> Mat4| {
            for (i, vm) in ty.meshes.iter().enumerate() {
                if !is_wheel(ty, vm.def_index) {
                    continue;
                }
                let m = xf(i);
                let Some(data) = ty.mesh_data(i) else {
                    continue;
                };
                let Some(low) = data
                    .positions
                    .iter()
                    .map(|p| m.transform_point3(*p))
                    .min_by(|a, b| a.z.total_cmp(&b.z))
                else {
                    continue;
                };
                let w = origin + low.as_dvec3();
                if let Some(g) = under(w.x, w.y, w.z + 0.3) {
                    out.push((ty.model.meshes[vm.def_index].file.clone(), (w.z - g) as f32));
                }
            }
        };
        measure(&self.ty, self.position, &|i| self.mesh_local_transform(i));
        for t in &self.trailers {
            measure(&t.ty, t.position, &|i| t.mesh_local_transform(i));
        }
        out
    }

    /// The static pose of an AI copy on the road: how far its origin stands above the plane
    /// its wheels touch (m) and the `Axle_Suspension` that goes with it. OMSI sets an AI copy
    /// `[ai_deltaheight]` lower than its model origin and lifts the wheels by as much through
    /// `Axle_Suspension` (every stock AI road model animates it), so it stands exactly like a
    /// driven one that has settled on its springs - the delta is that sag; left at zero the
    /// AI buses rode 10 cm high. On top of it, the model's wheel geometry (see `ai_lift`).
    /// `AiBody` adds its own travel on its springs, which is zero at rest.
    pub fn ai_rest_offset(&self) -> (f32, f32) {
        let delta = if self.ty.suspension_axles.is_empty() {
            0.0
        } else {
            self.ty.def.ai_delta_height
        };
        (delta + self.ai_lift, delta)
    }

    /// Where the wheel meshes turn (`OMSI_DEBUG_WHEELS`): for every mesh animated by a
    /// `Wheel_Rotation_*` or `Axle_Steering_*` variable, how far the centre of its vertices
    /// moves when that variable alone is set - a wheel spinning or steering about its own
    /// centre does not move at all, one with an offset pivot swings around. An AI type keeps
    /// no vertices on the CPU (`VehicleType::load_ai`), so the mesh is read again here.
    pub fn wheel_pivot_report(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (i, m) in self.ty.meshes.iter().enumerate() {
            let a = &self.animators[i];
            let names: Vec<&str> = a
                .anims
                .iter()
                .map(|(d, _, _)| d.variable.as_str())
                .filter(|v| v.starts_with("Wheel_Rotation") || v.starts_with("Axle_Steering"))
                .collect();
            if names.is_empty() {
                continue;
            }
            let Some(data) = self.ty.mesh_data(i) else {
                continue;
            };
            if data.positions.is_empty() {
                continue;
            }
            let (lo, hi) = data.positions.iter().fold(
                (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
                |(lo, hi), p| (lo.min(*p), hi.max(*p)),
            );
            let c = (lo + hi) * 0.5;
            let pv = m.pivot.col(3);
            let px = m.pivot.col(0);
            let mut line = format!("{} {}: centre ({:.3}, {:.3}, {:.3}) pivot ({:.3}, {:.3}, {:.3}) x ({:.2}, {:.2}, {:.2}) det {:.2}", self.ty.def.path.file_name().unwrap_or_default().to_string_lossy(), self.ty.model.meshes[m.def_index].file, c.x, c.y, c.z, pv.x, pv.y, pv.z, px.x, px.y, px.z, m.pivot.determinant());
            for name in names {
                let Some(id) = self.ty.program.var(name) else {
                    continue;
                };
                let mut vars = vec![0.0; self.state.vars.len()];
                vars[id as usize] = 0.35;
                let mut probe = a.clone();
                for (_, _, st) in probe.anims.iter_mut() {
                    st.initialized = false;
                }
                let t = probe.update(0.0, &vars);
                let moved = t.transform_point3(c) - c;
                let (axis, angle) = Quat::from_mat4(&t).to_axis_angle();
                line.push_str(&format!("; {name} 0.35 rad moves it ({:.3}, {:.3}, {:.3}), axis ({:.2}, {:.2}, {:.2}) {:.1} deg", moved.x, moved.y, moved.z, axis.x, axis.y, axis.z, angle.to_degrees()));
            }
            out.push(line);
        }
        out
    }

    /// Couple a vehicle behind this one (`[couple_back]`).
    pub fn attach_trailer(&mut self, ty: Arc<VehicleType>) {
        self.attach_trailer_ex(ty, false);
    }

    /// Uncouple the last part of the train (it is gone from this vehicle).
    pub fn detach_last_trailer(&mut self) -> Option<TrailerPart> {
        self.trailers.pop()
    }

    /// Couple a vehicle behind the last part of the train; `reversed` cars run backwards
    /// (their `[coupling_back]` becomes the front coupling).
    pub fn attach_trailer_ex(&mut self, ty: Arc<VehicleType>, reversed: bool) {
        let first_axle =
            self.physics.wheels.len() + self.trailers.iter().map(|t| t.axle_count).sum::<usize>();
        let (lead_ty, lead_reversed) = match self.trailers.last() {
            Some(t) => (t.ty.clone(), t.reversed),
            None => (self.ty.clone(), false),
        };
        let t = TrailerPart::new_ex(
            ty,
            &lead_ty,
            lead_reversed,
            reversed,
            self.ty.program.as_ref(),
            first_axle,
            self.trailers.len(),
        );
        self.trailers.push(t);
    }
}

/// Per-mesh `[visible]`, `[matl_alphascale]` and texture offsets from variables. The
/// vehicles use the same rules through `PropsPlan`, resolved once; this is the plain
/// statement of them (and what the plan is tested against).
pub fn compute_mesh_props(ty: &VehicleType, var: &dyn Fn(&str) -> Option<f32>) -> Vec<MeshProps> {
    ty.meshes
        .iter()
        .map(|vm| {
            let def = &ty.model.meshes[vm.def_index];
            let mut props = MeshProps {
                slot_alpha: vec![1.0; vm.materials.len().max(1)],
                slot_light: vec![1.0; vm.materials.len().max(1)],
                slot_night: vec![1.0; vm.materials.len().max(1)],
                slot_uv: vec![[0.0; 2]; vm.materials.len().max(1)],
                ..Default::default()
            };
            for m in &def.materials {
                // [matl_change] tex idx var + [matl_item]: the variant (night map) is active
                // when the variable is set
                if let Some((_, _, v)) = &m.change {
                    if let Some(slot) = override_slot(&vm.materials, m) {
                        // (the item as Omsi.exe picks it: the variable rounded is 1; an
                        // undeclared one is 0 - see scene.rs `change_picks_item`)
                        let x = v
                            .trim()
                            .parse::<f32>()
                            .ok()
                            .or_else(|| var(v))
                            .unwrap_or(0.0);
                        props.slot_night[slot] = if x.is_finite() && x.round_ties_even() == 1.0 { 1.0 } else { 0.0 };
                    }
                }
                // several light maps: the slot is as bright as the brightest (the texture
                // is made of the ones switched on, see scene.rs `MultiLight`)
                if !m.lightmaps.is_empty() {
                    if let Some(slot) = override_slot(&vm.materials, m) {
                        props.slot_light[slot] = 0.0;
                    }
                }
                for (_, v) in &m.lightmaps {
                    if let Some(slot) = override_slot(&vm.materials, m) {
                        let x = v
                            .trim()
                            .parse::<f32>()
                            .ok()
                            .or_else(|| var(v))
                            .unwrap_or(0.0);
                        props.slot_light[slot] = props.slot_light[slot].max(if x >= 0.5 { 1.0 } else { 0.0 });
                    }
                }
            }
            if let Some((v, value)) = &def.visible {
                if let Some(x) = var(v) {
                    props.visible = (x - value).abs() < 0.5;
                }
            }
            // [illumination_interior] a b c d: the interior lights (by index) lighting this mesh
            let mut interior = 0.0f32;
            for idx in &def.illumination_interior {
                if let Some(il) = usize::try_from(*idx)
                    .ok()
                    .and_then(|i| ty.model.interior_lights.get(i))
                {
                    let b = il
                        .variable
                        .trim()
                        .parse::<f32>()
                        .ok()
                        .or_else(|| var(&il.variable))
                        .unwrap_or(0.0)
                        .clamp(0.0, 1.0);
                    interior = interior.max(b * il.range.clamp(0.0, 1.0));
                }
            }
            props.interior = interior * 0.5;
            for m in &def.materials {
                if let Some(v) = &m.alphascale {
                    if let (Some(x), Some(slot)) = (var(v), override_slot(&vm.materials, m)) {
                        let boost = if v.trim().to_ascii_lowercase().starts_with("rain_window") {
                            1.8
                        } else {
                            1.0
                        };
                        props.slot_alpha[slot] = (x * boost).clamp(0.0, 1.0);
                    }
                }
                // [texcoordtransX/Y] scroll one material slot's texture (each band of a
                // roller blind has its own variable)
                if let Some(v) = &m.texcoord_trans_x {
                    if let (Some(x), Some(slot)) = (var(v), override_slot(&vm.materials, m)) {
                        props.slot_uv[slot][0] = x;
                    }
                }
                if let Some(v) = &m.texcoord_trans_y {
                    if let (Some(x), Some(slot)) = (var(v), override_slot(&vm.materials, m)) {
                        props.slot_uv[slot][1] = x;
                    }
                }
            }
            props
        })
        .collect()
}

/// Where a mesh property takes its value from: a number written in model.cfg, a
/// variable, or nothing (the property keeps its default).
#[derive(Debug, Clone, Copy)]
enum PropSource {
    Const(f32),
    Var(usize),
    Missing,
}

impl PropSource {
    fn value(self, vars: &[f32], default: f32) -> f32 {
        match self {
            PropSource::Const(c) => c,
            PropSource::Var(i) => vars.get(i).copied().unwrap_or(default),
            PropSource::Missing => default,
        }
    }
}

/// The property sources of one mesh, in the terms of `compute_mesh_props`.
#[derive(Debug, Clone, Default)]
struct MeshPlan {
    slots: usize,
    /// `[matl_change]` (default 1) and `[matl_lightmap]` (default 0) per slot.
    night: Vec<(usize, PropSource)>,
    light: Vec<(usize, PropSource)>,
    /// `[visible]` variable and value.
    visible: Option<(usize, f32)>,
    /// `[illumination_interior]`: (brightness, range).
    interior: Vec<(PropSource, f32)>,
    /// `[alphascale]`, `[texcoordtransX/Y]` variables per slot; the third field of `alpha`
    /// boosts a raindrop-film layer (`Rain_Window_*_Wetness`) so it stays visible instead of
    /// reading as the texture's own faint alpha (its drops are only a few percent opaque,
    /// so a middling wetness value was nearly invisible against the glass behind it).
    alpha: Vec<(usize, usize, f32)>,
    uv_x: Vec<(usize, usize)>,
    uv_y: Vec<(usize, usize)>,
}

/// `compute_mesh_props` resolved once per vehicle: every frame it looked each variable up
/// by name (a lower-cased copy of the name each time), matched material slots by texture
/// name and allocated four vectors per mesh - for every mesh of every AI car, which made
/// the mesh properties the costliest part of the traffic's frame.
#[derive(Debug, Clone, Default)]
struct PropsPlan {
    meshes: Vec<MeshPlan>,
    /// Size of the variable table the plan was resolved against (engine variables such
    /// as `Dirt_Norm` join it later, and an `[alphascale]` on one must then find it).
    vars_seen: usize,
    built: bool,
}

impl PropsPlan {
    fn refresh(&mut self, ty: &VehicleType, var_index: &HashMap<String, omsi_script::VarId>) {
        if self.built && self.vars_seen == var_index.len() {
            return;
        }
        let var = |v: &str| var_index.get(&v.to_ascii_lowercase()).map(|&i| i as usize);
        let source = |v: &str| match v.trim().parse::<f32>() {
            Ok(c) => PropSource::Const(c),
            Err(_) => var(v).map(PropSource::Var).unwrap_or(PropSource::Missing),
        };
        self.meshes = ty
            .meshes
            .iter()
            .map(|vm| {
                let def = &ty.model.meshes[vm.def_index];
                let mut plan = MeshPlan {
                    slots: vm.materials.len().max(1),
                    ..Default::default()
                };
                for m in &def.materials {
                    let Some(slot) = override_slot(&vm.materials, m) else {
                        continue;
                    };
                    if let Some((_, _, v)) = &m.change {
                        plan.night.push((slot, source(v)));
                    }
                    for (_, v) in &m.lightmaps {
                        plan.light.push((slot, source(v)));
                    }
                    if let Some(name) = m.alphascale.as_deref() {
                        if let Some(i) = var(name) {
                            let boost =
                                if name.trim().to_ascii_lowercase().starts_with("rain_window") {
                                    1.8
                                } else {
                                    1.0
                                };
                            plan.alpha.push((slot, i, boost));
                        }
                    }
                    if let Some(i) = m.texcoord_trans_x.as_deref().and_then(var) {
                        plan.uv_x.push((slot, i));
                    }
                    if let Some(i) = m.texcoord_trans_y.as_deref().and_then(var) {
                        plan.uv_y.push((slot, i));
                    }
                }
                plan.visible = def
                    .visible
                    .as_ref()
                    .and_then(|(v, value)| var(v).map(|i| (i, *value)));
                for idx in &def.illumination_interior {
                    if let Some(il) = usize::try_from(*idx)
                        .ok()
                        .and_then(|i| ty.model.interior_lights.get(i))
                    {
                        plan.interior
                            .push((source(&il.variable), il.range.clamp(0.0, 1.0)));
                    }
                }
                plan
            })
            .collect();
        self.vars_seen = var_index.len();
        self.built = true;
    }

    /// The mesh properties for the current variables, written into `out` in place.
    fn apply(&self, vars: &[f32], out: &mut Vec<MeshProps>) {
        out.resize_with(self.meshes.len(), MeshProps::default);
        for (plan, props) in self.meshes.iter().zip(out.iter_mut()) {
            let n = plan.slots;
            for (v, fill) in [
                (&mut props.slot_alpha, 1.0),
                (&mut props.slot_light, 1.0),
                (&mut props.slot_night, 1.0),
            ] {
                v.clear();
                v.resize(n, fill);
            }
            props.slot_uv.clear();
            props.slot_uv.resize(n, [0.0; 2]);
            props.visible = true;
            for &(slot, src) in &plan.night {
                let x = src.value(vars, 0.0);
                props.slot_night[slot] = if x.is_finite() && x.round_ties_even() == 1.0 { 1.0 } else { 0.0 };
            }
            for &(slot, _) in &plan.light {
                props.slot_light[slot] = 0.0;
            }
            // (a light map is on at its variable's 0.5 and off below - Omsi.exe skips the
            // texture stage of one whose variable reads under 0.5, 0x7fe51f - never half lit)
            for &(slot, src) in &plan.light {
                props.slot_light[slot] = props.slot_light[slot].max(if src.value(vars, 0.0) >= 0.5 { 1.0 } else { 0.0 });
            }
            if let Some((i, value)) = plan.visible {
                if let Some(x) = vars.get(i) {
                    props.visible = (x - value).abs() < 0.5;
                }
            }
            let mut interior = 0.0f32;
            for &(src, range) in &plan.interior {
                interior = interior.max(src.value(vars, 0.0).clamp(0.0, 1.0) * range);
            }
            props.interior = interior * 0.5;
            for &(slot, i, boost) in &plan.alpha {
                props.slot_alpha[slot] =
                    (vars.get(i).copied().unwrap_or(1.0) * boost).clamp(0.0, 1.0);
            }
            for &(slot, i) in &plan.uv_x {
                props.slot_uv[slot][0] = vars.get(i).copied().unwrap_or(0.0);
            }
            for &(slot, i) in &plan.uv_y {
                props.slot_uv[slot][1] = vars.get(i).copied().unwrap_or(0.0);
            }
        }
    }
}

/// How far above the plane its wheels stand on a shadow blob is drawn (m): over the road's
/// crown between the wheels, and clear of it for the surfaces' depth bias.
pub const SHADOW_LIFT: f32 = 0.02;

/// How strong the film on the glass gets in the thickest snowfall (`Rain_Window_*_Wetness`,
/// 0 … 1): a haze of crystals, not a windscreen running with water.
const SNOW_ON_GLASS: f32 = 0.22;

fn is_shadow_mesh(ty: &VehicleType, i: usize) -> bool {
    ty.meshes
        .get(i)
        .is_some_and(|m| ty.model.meshes[m.def_index].is_shadow)
}

/// Body frame → body frame with the plane z = 0 laid onto z = p[0] + p[1]·x + p[2]·y
/// (lifted by `SHADOW_LIFT`).
pub fn onto_plane(p: [f32; 3]) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(1.0, 0.0, p[1], 0.0),
        Vec4::new(0.0, 1.0, p[2], 0.0),
        Vec4::Z,
        Vec4::new(0.0, 0.0, p[0] + SHADOW_LIFT, 1.0),
    )
}

/// The least-squares plane z = p[0] + p[1]·x + p[2]·y through `points`; level through their
/// mean height when they do not span a plane (one axle).
pub fn fit_plane(points: &[Vec3]) -> [f32; 3] {
    let n = points.len().max(1) as f32;
    let mean = points.iter().copied().sum::<Vec3>() / n;
    let (mut sxx, mut sxy, mut syy, mut sxz, mut syz) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for p in points {
        let d = *p - mean;
        sxx += d.x * d.x;
        sxy += d.x * d.y;
        syy += d.y * d.y;
        sxz += d.x * d.z;
        syz += d.y * d.z;
    }
    let det = sxx * syy - sxy * sxy;
    let (b, c) = if det > 1e-3 * (sxx + syy).max(1e-6) {
        ((sxz * syy - syz * sxy) / det, (syz * sxx - sxz * sxy) / det)
    } else {
        (0.0, 0.0)
    };
    [mean.z - b * mean.x - c * mean.y, b, c]
}

/// A vehicle coupled behind another (articulated bus rear section, trailer, train car).
/// It has no script state of its own: meshes animate from the leading vehicle's
/// variables, and it is towed at the coupling point with trailer kinematics.
/// Sign of `articulation_<n>_alpha` relative to (heading in front − own heading), clockwise
/// degrees: settled with the GN92's joint dummies (see the vehicle tests).
const ARTICULATION_SIGN: f64 = 1.0;

pub struct TrailerPart {
    pub ty: Arc<VehicleType>,
    /// The part's own `[smoke]` particle systems (the pusher's rear engine).
    pub particles: ParticleSet,
    /// Its lights' brightness as they come on and go out (see `VehicleInstance::light_fade`).
    pub light_fade: Vec<f32>,
    animators: Vec<MeshAnimator>,
    pub mesh_transforms: Vec<Mat4>,
    pub mesh_props: Vec<MeshProps>,
    pub position: DVec3,
    pub heading: f64,
    /// World position of the rear pivot (the trailer axle) from the last frame.
    pivot: Option<DVec3>,
    /// Coupling → rear axle distance.
    length: f32,
    coupling_front: Vec3,
    coupling_back: Vec3,
    axle_long: f32,
    axle_count: usize,
    /// First axle index of this part in the train's `Wheel_*` variables.
    first_axle: usize,
    wheel_radius: f32,
    /// `articulation_<n>_alpha` (the angle about the vertical axis, which the jackknife
    /// protection watches and the joint's plates and bellows turn with) and
    /// `articulation_<n>_beta` (about the transverse axis) of this part's joint.
    v_alpha: Option<omsi_script::VarId>,
    v_beta: Option<omsi_script::VarId>,
    odometer: f32,
    pub reversed: bool,
    /// Per axle of this part: tyre radius less the unloaded hub height, the static load per
    /// wheel and the spring rate - it sits down on its springs like the part it follows.
    rest: Vec<(f32, f32, f32)>,
    /// `Axle_Brakeforce_<axle>_L/R` of this part's axles, in the leading vehicle's scripts.
    v_brakes: Vec<Option<omsi_script::VarId>>,
    /// How far the part's origin stands above the ground under its axles (m).
    ground_lift: f32,
    /// The part's pitch (degrees, nose up) and lean (degrees, as the vehicle's `bank`) as
    /// it travels (not turned round for a reversed part), and the eased height of its axle.
    pitch: f32,
    bank: f32,
    axle_z: Option<f64>,
    /// How fast the axle's height moves (m/s), for the road part's springs (see `follow`).
    axle_vz: f64,
    /// The track under its turning axle, when it runs on rails (`VehicleInstance::retrail`):
    /// it stands at the track's height, not on whatever the ground probe finds there.
    track: Option<DVec3>,
    /// Mesh property sources, resolved against the leading vehicle's variables.
    props_plan: PropsPlan,
    /// The meshes' transforms in the modelled pose, for `[smoothskin]` (made when needed).
    skin_rest: Vec<Mat4>,
    /// The part's own `[texttexture]`s, written from the leading vehicle's string
    /// variables (`[scriptshare]`: the rear section of an articulated bus shows the
    /// number and the destination the front's scripts set).
    pub text_textures: Vec<crate::texttex::TextTextureState>,
}

impl TrailerPart {
    /// Pitch (degrees, nose up), eased axle height and the track point it stands on (for
    /// the `OMSI_DEBUG_TRAILERS` trace).
    pub fn debug_pose(&self) -> (f32, Option<f64>, Option<DVec3>) {
        (self.pitch, self.axle_z, self.track)
    }

    pub fn new(
        ty: Arc<VehicleType>,
        main: &VehicleType,
        program: &Program,
        first_axle: usize,
    ) -> TrailerPart {
        Self::new_ex(ty, main, false, false, program, first_axle, 0)
    }

    /// `joint`: which joint of the train this part hangs on (0 behind the leading vehicle).
    pub fn new_ex(
        ty: Arc<VehicleType>,
        main: &VehicleType,
        main_reversed: bool,
        reversed: bool,
        program: &Program,
        first_axle: usize,
        joint: usize,
    ) -> TrailerPart {
        let mut animators: Vec<MeshAnimator> = ty
            .meshes
            .iter()
            .map(|m| MeshAnimator::new(&ty.model.meshes[m.def_index], m.pivot, |n| program.var(n)))
            .collect();
        crate::anim::link_parents(
            &mut animators,
            &ty.meshes
                .iter()
                .map(|m| &ty.model.meshes[m.def_index])
                .collect::<Vec<_>>(),
        );
        // Couplings stay in the body frame: `body_rotation` already turns a reversed part
        // around, so its rear coupling faces the leading vehicle and its front axle trails.
        let own_front = if reversed {
            ty.def.coupling_back.as_ref().map(|c| Vec3::from(c.pos))
        } else {
            ty.def.coupling_front.as_ref().map(|c| Vec3::from(c.pos))
        };
        let coupling_front = own_front.unwrap_or(if reversed {
            Vec3::new(0.0, -4.0, 0.3)
        } else {
            Vec3::new(0.0, 4.0, 0.3)
        });
        let lead_back = if main_reversed {
            main.def.coupling_front.as_ref().map(|c| Vec3::from(c.pos))
        } else {
            main.def.coupling_back.as_ref().map(|c| Vec3::from(c.pos))
        };
        let coupling_back = lead_back.unwrap_or(if main_reversed {
            Vec3::new(0.0, 4.0, 0.3)
        } else {
            Vec3::new(0.0, -4.0, 0.3)
        });
        // the pivot axle: the one farthest from the coupled end
        let axle_long = if reversed {
            let a = ty.def.axles.iter().map(|a| a.long).fold(f32::MIN, f32::max);
            if a == f32::MIN {
                0.5
            } else {
                a
            }
        } else {
            let a = ty.def.axles.iter().map(|a| a.long).fold(f32::MAX, f32::min);
            if a == f32::MAX {
                -0.5
            } else {
                a
            }
        };
        let wheel_radius = ty
            .def
            .axles
            .first()
            .map(|a| (a.wheel_diameter / 2.0).max(0.1))
            .unwrap_or(0.5);
        let n = ty.meshes.len();
        let loads = crate::rigid::wheel_rest_loads(&ty.def);
        let rest = ty
            .def
            .axles
            .iter()
            .zip(ty.hub_heights(first_axle))
            .enumerate()
            .map(|(a, (axle, z))| {
                let r = (axle.wheel_diameter / 2.0).max(0.15);
                let k = if axle.spring > 0.0 {
                    axle.spring * 1000.0
                } else {
                    150_000.0
                };
                (r - z.unwrap_or(r), loads.get(a).copied().unwrap_or(0.0), k)
            })
            .collect();
        let v_brakes = (0..ty.def.axles.len())
            .flat_map(|a| {
                ["L", "R"]
                    .map(|side| program.var(&format!("Axle_Brakeforce_{}_{side}", first_axle + a)))
            })
            .collect();
        TrailerPart {
            particles: ParticleSet::new(ty.model.particle_systems(), first_axle as u64 * 7919 + 17),
            light_fade: Vec::new(),
            rest,
            v_brakes,
            ground_lift: 0.0,
            pitch: 0.0,
            bank: 0.0,
            axle_z: None,
            axle_vz: 0.0,
            track: None,
            v_alpha: program.var(&format!("articulation_{joint}_alpha")),
            v_beta: program.var(&format!("articulation_{joint}_beta")),
            animators,
            mesh_transforms: vec![Mat4::IDENTITY; n],
            mesh_props: vec![MeshProps::default(); n],
            props_plan: PropsPlan::default(),
            position: DVec3::ZERO,
            heading: 0.0,
            pivot: None,
            length: (coupling_front.y - axle_long).abs(),
            coupling_front,
            coupling_back,
            axle_long,
            axle_count: ty.def.axles.len().max(1),
            first_axle,
            wheel_radius,
            odometer: 0.0,
            reversed,
            text_textures: Vec::new(),
            skin_rest: Vec::new(),
            ty,
        }
    }

    /// The `[smoothskin]` meshes of this part.
    pub fn skinned_meshes(&self) -> Vec<usize> {
        (0..self.ty.meshes.len())
            .filter(|&i| !self.ty.meshes[i].skin.is_empty())
            .collect()
    }

    /// See [`VehicleInstance::skin_key`].
    pub fn skin_key(&self, i: usize) -> Vec<Mat4> {
        skin_key(&self.ty, i, &self.mesh_transforms)
    }

    /// Skinned mesh `i` as its bones hold it now; `n_vars`: the leading vehicle's variable
    /// count (this part animates from its variables).
    pub fn skinned(&mut self, i: usize, n_vars: usize) -> Option<(Vec<Vec3>, Vec<Vec3>)> {
        if self.skin_rest.len() != self.animators.len() {
            self.skin_rest = rest_transforms(&self.animators, n_vars);
        }
        skin_vertices(&self.ty, i, &self.mesh_transforms, &self.skin_rest)
    }

    /// Prepare the part's text textures with fonts from `lib`.
    pub fn init_text_textures(
        &mut self,
        lib: &mut crate::texttex::FontLibrary,
        decode: &dyn Fn(&Path) -> Option<(u32, u32, Vec<u8>)>,
    ) {
        self.text_textures = self
            .ty
            .model
            .text_textures
            .iter()
            .map(|t| crate::texttex::TextTextureState::new(t.clone(), lib.get(&t.font, decode)))
            .collect();
    }

    /// Re-render the text textures whose variable (of the leading vehicle) changed;
    /// returns the indices with a pending image.
    pub fn update_text_textures(&mut self, main: &VehicleInstance) -> Vec<usize> {
        let mut changed = Vec::new();
        for (i, t) in self.text_textures.iter_mut().enumerate() {
            let text = main.str_var(&t.def.variable);
            if t.update(&text) {
                changed.push(i);
            }
        }
        changed
    }

    /// A trailer follows its tractor's heading; it pitches between the coupling and the
    /// ground under its axle, and leans as the vehicle does.
    pub fn body_rotation(&self) -> Mat4 {
        let (h, p, b) = if self.reversed {
            (self.heading + 180.0, -self.pitch, -self.bank)
        } else {
            (self.heading, self.pitch, self.bank)
        };
        Mat4::from_quat(Quat::from_rotation_z((-h).to_radians() as f32) * Quat::from_rotation_x(p.to_radians()) * Quat::from_rotation_y(b.to_radians()))
    }

    /// World transform of the part's body (f32, for sound positions).
    pub fn world_transform(&self) -> Mat4 {
        Mat4::from_translation(self.position.as_vec3()) * self.body_rotation()
    }

    /// Position/direction of one of the part's own `.bus` cameras in world space: (eye,
    /// yaw, pitch) - as `VehicleInstance::camera_world` for the front part.
    pub fn camera_world(&self, cam: &omsi_vehicle::Camera) -> (DVec3, f32, f32) {
        let local = Vec3::new(cam.pos[0], cam.pos[1], cam.pos[2]);
        let eye = self.position + self.body_rotation().transform_point3(local).as_dvec3();
        let heading = if self.reversed { self.heading + 180.0 } else { self.heading };
        (eye, heading as f32 + cam.yaw, cam.pitch)
    }

    /// Transform for mesh `i` relative to the part's position; a shadow blob lies on the
    /// ground under its axles (see `VehicleInstance::mesh_local_transform`).
    pub fn mesh_local_transform(&self, i: usize) -> Mat4 {
        if is_shadow_mesh(&self.ty, i) {
            return self.body_rotation()
                * onto_plane([-self.ground_lift, 0.0, 0.0])
                * self.mesh_transforms[i];
        }
        self.body_rotation() * self.mesh_transforms[i]
    }

    /// Where this part hangs on the one in front: the coupling point in the frame of the
    /// part in front, and its own coupling point in its own frame (the two meet in the
    /// world). With the parts straight behind each other, a point `p` of this part lies at
    /// `p + back - front` in the frame of the part in front.
    pub fn couplings(&self) -> (Vec3, Vec3) {
        (self.coupling_back, self.coupling_front)
    }

    /// Put this part where another source says it is (a LAN player's rear section): the
    /// pivot is set so that the next follow step keeps this heading.
    pub fn set_pose(&mut self, position: DVec3, heading: f64) {
        self.heading = heading;
        let rot = self.body_rotation();
        let c = position + rot.transform_point3(self.coupling_front).as_dvec3();
        let h = heading.to_radians();
        self.pivot = Some(c - DVec3::new(h.sin(), h.cos(), 0.0) * self.length as f64);
        self.position = position;
    }

    /// Forget where this part was: the next step puts it straight behind the leading part
    /// (after the vehicle was moved somewhere else).
    pub fn realign(&mut self) {
        self.pivot = None;
        self.axle_z = None;
        self.track = None;
    }

    /// Where this part couples to the part in front of it, in the world, with the leading
    /// part as it stands now (`lead`: position, rotation of the part in front; the vehicle
    /// itself for the first trailer).
    pub fn coupling_point(&self, lead_pos: DVec3, lead_rot: Mat4) -> DVec3 {
        lead_pos + lead_rot.transform_point3(self.coupling_back).as_dvec3()
    }

    /// The distance from the coupling to the axle the part turns about.
    pub fn pivot_length(&self) -> f32 {
        self.length
    }

    /// Put the part's turning axle at `pivot` (a point on the road behind the coupling):
    /// a bus placed on a bend stands with its rear section following the bend rather than
    /// straight behind it.
    pub fn place_pivot(&mut self, pivot: DVec3) {
        self.pivot = Some(pivot);
    }

    /// Put the part's turning axle on a track at `p` (a rail vehicle's coupled car or
    /// section): it also takes the track's height there.
    pub fn place_on_track(&mut self, p: DVec3) {
        self.pivot = Some(p);
        self.track = Some(p);
    }

    fn update(&mut self, main: &mut VehicleInstance, dt: f32, lead: Option<(DVec3, Mat4, f64)>) {
        // coupling point in the world: on the leading part (the vehicle or the previous trailer)
        let (lead_pos, lead_rot, lead_heading) =
            lead.unwrap_or((main.position, main.body_rotation(), main.heading));
        let c = lead_pos + lead_rot.transform_point3(self.coupling_back).as_dvec3();
        // the height its axle had (the new one eases from it; nothing after a move)
        let prev_z = self.pivot.and(self.axle_z);
        let pivot = match self.pivot {
            Some(p) => p,
            None => {
                // start straight behind the leading vehicle
                let h = main.heading.to_radians();
                c - DVec3::new(h.sin(), h.cos(), 0.0) * self.length as f64
            }
        };
        let mut dir = (c - pivot).truncate();
        if dir.length() < 1e-3 {
            let h = main.heading.to_radians();
            dir = glam::DVec2::new(h.sin(), h.cos());
        }
        let dir = dir.normalize();
        self.heading = dir.x.atan2(dir.y).to_degrees();
        let new_pivot = c - DVec3::new(dir.x, dir.y, 0.0) * self.length as f64;
        let ds = (new_pivot - pivot).truncate().length() as f32;
        self.odometer += ds * (main.physics.velocity_kmh().signum().max(0.0) * 2.0 - 1.0).max(-1.0);
        self.pivot = Some(new_pivot);
        // Its springs: an AI copy sits `[ai_deltaheight]` low like its tractor; a driven one
        // as far down as the scripts' `Axle_Springfactor` lets the load press it.
        let ai = main.var("AI").map(|v| v > 0.5).unwrap_or(false);
        let shows = !self.ty.suspension_axles.is_empty();
        let mut sag = Vec::with_capacity(self.rest.len());
        for (a, (offset, load, k)) in self.rest.iter().enumerate() {
            let axle = self.first_axle + a;
            let compression = if ai {
                if shows {
                    -self.ty.def.ai_delta_height
                } else {
                    0.0
                }
            } else if self.ty.suspension_axles.contains(&axle) {
                let factor = main
                    .var(&format!("Axle_Springfactor_{axle}_L"))
                    .unwrap_or(1.0)
                    .max(0.05);
                (load / (k * factor)).min(crate::rigid::BUMP)
            } else {
                0.0
            };
            for side in ["L", "R"] {
                if let Some(id) = main
                    .ty
                    .program
                    .var(&format!("Axle_Suspension_{axle}_{side}"))
                {
                    main.state.vars[id as usize] = -compression;
                }
            }
            sag.push(offset - compression);
        }
        let lift = sag.iter().sum::<f32>() as f64 / sag.len().max(1) as f64;
        self.ground_lift = lift as f32;
        // On rails: the track's height where it was put on it (the ground probe found the
        // platform edge or the embankment beside a bend, and the car jumped up and down).
        let on_track = self
            .track
            .filter(|t| (t.truncate() - new_pivot.truncate()).length() < 1.0)
            .map(|t| t.z);
        // the ground under its axle: what the wheels stand on where the world says, else the
        // plain height sampler
        let ground_z = match (on_track, &main.contact, &main.ground) {
            (Some(_), _, _) => None,
            (None, Some(c), _) => {
                c.probe(new_pivot.x, new_pivot.y, self.position.z + 1.5)
                    .below
            }
            (None, None, Some(g)) => g(new_pivot.x, new_pivot.y),
            _ => None,
        };
        // the height of the part's origin over its axle (where the ground has none: level
        // with the coupling, as before)
        // A height far from where the coupling holds the part is another level's: the AI's
        // ground lookup knows only x and y and gives the highest road there, which under a
        // bridge is the deck (or, on the deck, a road that runs on beneath it) - the trailer
        // of a lorry and the rear of an articulated bus stood up on the bridge or down under
        // it (#140). Level with the coupling instead.
        let level = c.z - self.coupling_front.z as f64;
        let ground_z = ground_z.filter(|z| main.contact.is_some() || (z + lift - level).abs() < 1.5);
        let axle_z = match on_track.or(ground_z.map(|z| z + lift)) {
            Some(z) if on_track.is_some() => z,
            Some(z) if main.contact.is_some() && dt > 0.0 => {
                // On the road the part stands on its springs as the part in front does in
                // Omsi.exe (each section is a body of its own on the same wheel springs): a
                // bump under its axle is a jolt that swings out, not a height eased into over
                // a sixth of a second, which smoothed every bump away under the rear of an
                // articulated bus. (Sprung at about 1.6 Hz, a little damped, as a bus body.)
                let from = prev_z.unwrap_or(z);
                if (z - from).abs() > 0.5 {
                    self.axle_vz = 0.0;
                    z
                } else {
                    let (w, zeta) = (2.0 * std::f64::consts::PI * 1.6, 0.35);
                    let h = (dt as f64).min(0.05);
                    let acc = w * w * (z - from) - 2.0 * zeta * w * self.axle_vz;
                    self.axle_vz = (self.axle_vz + acc * h).clamp(-3.0, 3.0);
                    from + self.axle_vz * h
                }
            }
            Some(z) => {
                // The sampled surface is not perfectly smooth (a centimetre of wobble along
                // the railway ballast every metre or two), and a car that follows every
                // sample shivers up and down. Ease towards it instead: a slope still comes
                // through within a fraction of a second, the wobble does not.
                let from = prev_z.unwrap_or(z);
                let dz = z - from;
                if dz.abs() > 0.5 {
                    z
                } else {
                    from + dz * (dt as f64 * 6.0).min(1.0)
                }
            }
            None => level,
        };
        self.axle_z = Some(axle_z);
        // The part hangs at the coupling in front and stands on its axle behind: its pitch is
        // the slope between them (it was drawn level at the axle's height, and on a grade or
        // a crest the joint came apart by a hand's breadth or more). It leans as the vehicle
        // it is coupled to does.
        let rise = c.z - (axle_z + self.coupling_front.z as f64);
        self.pitch = (rise.atan2(self.length.max(0.5) as f64).to_degrees() as f32).clamp(-15.0, 15.0);
        self.bank = main.bank;
        // the trailer origin: coupling_front sits at c
        let rot = self.body_rotation();
        self.position = c - rot.transform_point3(self.coupling_front).as_dvec3();
        // The joint's angles (degrees) for its plates and bellows and for the scripts: alpha
        // about the vertical axis - the stock articulation.osc's jackknife protection brakes
        // at |alpha| > 47° - and beta about the transverse axis. (The horizontal angle went
        // to beta: the protection never engaged, the bellows turned in the wrong plane.)
        // The part in front is drawn pitched, this one level: beta is that difference, the
        // part in front's pitch less this one's. (Taken the other way round, the Agora L's
        // joint arch and bellows - `anim_rot articulation_0_beta` - tilted away from the rear
        // section instead of towards it, twice the angle apart at the far ring.)
        // (the pitch of the part in front as it travels, read off its rotation: forward along
        // its heading, whichever way its model is turned)
        let lead_pitch = {
            let f = lead_rot.transform_vector3(Vec3::Y);
            let h = lead_heading.to_radians();
            let along = f.x as f64 * h.sin() + f.y as f64 * h.cos();
            (f.z as f64).atan2(along.abs().max(1e-6) * along.signum()).to_degrees()
        };
        let lead_pitch = if lead_pitch.abs() > 90.0 { lead_pitch - 180.0 * lead_pitch.signum() } else { lead_pitch };
        let alpha = ((lead_heading - self.heading + 540.0) % 360.0) - 180.0;
        // (the part in front's pitch less this one's, as Omsi.exe's beta runs (0x7de798: it
        // grows as the rear axle sinks): taken the other way round the bellows bent away
        // from the rear section on any grade, their folds sheared and a gap opened at one end)
        let beta = lead_pitch - self.pitch as f64;
        if let Some(id) = self.v_alpha {
            main.state.vars[id as usize] = (alpha * ARTICULATION_SIGN) as f32;
        }
        if let Some(id) = self.v_beta {
            main.state.vars[id as usize] = beta as f32;
        }
        // wheels of this part
        let rpm =
            main.physics.velocity_kmh() / 3.6 / (2.0 * std::f32::consts::PI * self.wheel_radius)
                * 60.0;
        // radians, like every `Wheel_Rotation_*` (the degrees written here before spun the
        // rear section's wheels 57 times too fast: a flicker instead of a rolling wheel)
        let rot = (self.odometer / self.wheel_radius).rem_euclid(std::f32::consts::TAU);
        for a in 0..self.axle_count {
            for side in ["L", "R"] {
                let k = self.first_axle + a;
                if let Some(id) = main.ty.program.var(&format!("Wheel_Rotation_{k}_{side}")) {
                    main.state.vars[id as usize] = rot;
                }
                if let Some(id) = main
                    .ty
                    .program
                    .var(&format!("Wheel_RotationSpeed_{k}_{side}"))
                {
                    main.state.vars[id as usize] = rpm;
                }
            }
        }
        for (i, a) in self.animators.iter_mut().enumerate() {
            self.mesh_transforms[i] = a.update(dt, &main.state.vars);
        }
        crate::anim::apply_parents(&self.animators, &mut self.mesh_transforms);
        if omsi_cfg::env::var_os("OMSI_DEBUG_TRAILER").is_some() {
            log::info!(
                "trailer: rest {:?} sag {:?} lift {lift:.3} ground {ground_z:?} z {:.3}",
                self.rest,
                sag,
                self.position.z
            );
            for (i, m) in self.ty.meshes.iter().enumerate() {
                let file = &self.ty.model.meshes[m.def_index].file;
                if file.to_ascii_lowercase().contains("rad") {
                    log::info!(
                        "  {file}: pivot w {:?} transform w {:?}",
                        m.pivot.w_axis.truncate(),
                        self.mesh_transforms[i].w_axis.truncate()
                    );
                }
            }
        }
        self.props_plan.refresh(&self.ty, &main.var_index);
        self.props_plan
            .apply(&main.state.vars, &mut self.mesh_props);
        let _ = self.axle_long;
    }
}

impl VehicleInstance {
    /// Rotation of the vehicle body (model frame → world, without translation).
    pub fn body_rotation(&self) -> Mat4 {
        let q = Quat::from_rotation_z((-self.heading).to_radians() as f32)
            * Quat::from_rotation_x(self.pitch.to_radians())
            * Quat::from_rotation_y(self.bank.to_radians());
        Mat4::from_quat(q)
    }

    /// World transform of the vehicle body (f32; for local computations such as sound
    /// positions - use `body_rotation` + `position` for rendering).
    pub fn world_transform(&self) -> Mat4 {
        Mat4::from_translation(self.position.as_vec3()) * self.body_rotation()
    }

    /// Transform for mesh `i` relative to the vehicle position (rotation + animation). A
    /// flat shadow blob (`[isshadow]`, drawn `[matl_noZcheck]` in the original, i.e. over the
    /// road whatever the depth) is put onto the plane the wheels stand on: in the model it
    /// lies at z = 0, which the springs' sag takes 10-16 cm under the road, where no depth
    /// bias brings it through.
    pub fn mesh_local_transform(&self, i: usize) -> Mat4 {
        if is_shadow_mesh(&self.ty, i) {
            return self.body_rotation()
                * onto_plane(self.contact_plane())
                * self.mesh_transforms[i];
        }
        self.body_rotation() * self.mesh_transforms[i]
    }

    /// The plane the wheels stand on, in the body frame: z = p[0] + p[1]·x + p[2]·y (m).
    /// The rigid body's tyres touch it a radius under their hubs (which the springs have
    /// pushed up into the body), the simple physics asks the ground under each wheel; an
    /// AI copy stands `ai_rest_offset` above its plane.
    pub fn contact_plane(&self) -> [f32; 3] {
        let mut points: Vec<Vec3> = Vec::new();
        if let Some(rb) = &self.rigid {
            let standing = rb.wheels.iter().any(|w| w.on_ground);
            for w in rb.wheels.iter().filter(|w| w.on_ground || !standing) {
                points.push(
                    w.attach + Vec3::Z * (w.compression.max(-crate::rigid::DROOP) - w.radius),
                );
            }
        } else if let Some(g) = &self.ground {
            let rot = self.body_rotation();
            let inv = rot.inverse();
            for w in self.physics.wheels.iter().flatten() {
                let p = self.position
                    + rot
                        .transform_vector3(Vec3::new(w.lat, w.long, 0.0))
                        .as_dvec3();
                if let Some(z) = g(p.x, p.y) {
                    points.push(
                        inv.transform_vector3((DVec3::new(p.x, p.y, z) - self.position).as_vec3()),
                    );
                }
            }
        }
        if points.is_empty() {
            return [-self.ai_rest_offset().0, 0.0, 0.0];
        }
        fit_plane(&points)
    }

    /// Position/direction of a `.bus` camera in world space: (eye, yaw, pitch).
    pub fn camera_world(&self, cam: &omsi_vehicle::Camera) -> (DVec3, f32, f32) {
        let local = Vec3::new(cam.pos[0], cam.pos[1], cam.pos[2]);
        let eye = self.position + self.body_rotation().transform_point3(local).as_dvec3();
        (eye, self.heading as f32 + cam.yaw, cam.pitch)
    }

    /// A camera fixed to the body as OMSI keeps one (`[add_camera_reflexion]`: Omsi.exe
    /// 0x7edfd0 puts it in the vehicle's own matrix): its eye, and yaw, pitch and roll (deg)
    /// of its view with the body's pitch and bank in them - a mirror leans with the bus. A
    /// `dist` above zero puts the eye that far behind the point along the view.
    pub fn camera_world_full(&self, cam: &omsi_vehicle::Camera) -> (DVec3, f32, f32, f32) {
        let rot = self.body_rotation();
        let (sy, cy) = cam.yaw.to_radians().sin_cos();
        let (sp, cp) = cam.pitch.to_radians().sin_cos();
        let f_local = Vec3::new(sy * cp, cy * cp, sp);
        let r_local = Vec3::new(cy, -sy, 0.0);
        let f = rot.transform_vector3(f_local).normalize_or(Vec3::Y);
        let up = rot.transform_vector3(r_local.cross(f_local)).normalize_or(Vec3::Z);
        let local = Vec3::new(cam.pos[0], cam.pos[1], cam.pos[2]);
        let eye = self.position + rot.transform_point3(local).as_dvec3() - (f * cam.dist.max(0.0)).as_dvec3();
        let yaw = f.x.atan2(f.y).to_degrees();
        let pitch = f.z.clamp(-1.0, 1.0).asin().to_degrees();
        // (the roll the renderer's `Camera::up` turns back into this up)
        let r0 = Vec3::new(f.y, -f.x, 0.0).normalize_or(Vec3::X);
        let u0 = r0.cross(f);
        let roll = up.dot(r0).atan2(up.dot(u0)).to_degrees();
        (eye, yaw, pitch, roll)
    }
}

fn skin_key(ty: &VehicleType, i: usize, transforms: &[Mat4]) -> Vec<Mat4> {
    let Some(vm) = ty.meshes.get(i) else {
        return Vec::new();
    };
    let mut key = vec![transforms.get(i).copied().unwrap_or(Mat4::IDENTITY)];
    for b in &vm.skin {
        if let Some(k) = ty.meshes.iter().position(|m| Some(m.def_index) == b.def_index) {
            key.push(transforms.get(k).copied().unwrap_or(Mat4::IDENTITY));
        }
    }
    key
}

/// The transforms of every mesh with all variables at 0: the pose the model was built in
/// (what a skinned mesh's vertices are given in).
fn rest_transforms(animators: &[MeshAnimator], n_vars: usize) -> Vec<Mat4> {
    let zeros = vec![0.0; n_vars];
    let mut probes = animators.to_vec();
    let mut out: Vec<Mat4> = probes
        .iter_mut()
        .map(|a| {
            for (_, _, st) in a.anims.iter_mut() {
                st.initialized = false;
            }
            a.update(0.0, &zeros)
        })
        .collect();
    crate::anim::apply_parents(&probes, &mut out);
    out
}

/// The vertices (positions, normals) of `[smoothskin]` mesh `i` with its bones where
/// `transforms` has them, in the mesh's own frame (the renderer puts `transforms[i]` on
/// top); `rest` are the transforms of the modelled pose. None for a mesh without bones or
/// vertices.
///
/// As in Omsi.exe, the bones move the vertices first and the mesh's own motion (its
/// animations and its `[animparent]`) comes on top: the Agora L's rear half of the bellows
/// (`gelenk_B`) hangs on the arch that turns by half the joint's angle and takes the same
/// bones as the front half (a quarter and a half of it), so that its far ring ends up at
/// the whole angle, with the rear section. Put in place of the mesh's motion, the bones
/// held that ring at half the angle and the bellows fanned out across the bend.
///
/// A vertex no bone holds stays with the mesh, and so does the share of a vertex that an
/// unbound bone holds: dropping that share and making up the rest to 1, a vertex hung half
/// on the armature's fixed root moved all the way with the other bone - the Agora's retarder
/// lever bent out of shape.
pub fn skin_vertices(
    ty: &VehicleType,
    i: usize,
    transforms: &[Mat4],
    rest: &[Mat4],
) -> Option<(Vec<Vec3>, Vec<Vec3>)> {
    let vm = ty.meshes.get(i)?;
    let n = vm.data.positions.len();
    if vm.skin.is_empty()
        || n == 0
        || transforms.len() < ty.meshes.len()
        || rest.len() < ty.meshes.len()
    {
        return None;
    }
    let mut sum = vec![Mat4::ZERO; n];
    let mut total = vec![0.0f32; n];
    for b in &vm.skin {
        let bone = match b.def_index {
            None => Mat4::IDENTITY,
            Some(d) => {
                let Some(k) = ty.meshes.iter().position(|m| m.def_index == d) else {
                    continue;
                };
                transforms[k] * rest[k].inverse()
            }
        };
        for &(v, w) in &b.weights {
            let v = v as usize;
            if v < n && w.is_finite() && w > 0.0 {
                sum[v] += bone * w;
                total[v] += w;
            }
        }
    }
    // (the renderer puts the mesh's transform on top: in the modelled pose, the vertices
    // are where the file has them)
    let own_rest_inv = rest[i].inverse();
    let mut pos = Vec::with_capacity(n);
    let mut nrm = Vec::with_capacity(n);
    for v in 0..n {
        let p = vm.data.positions[v];
        let q = vm.data.normals.get(v).copied().unwrap_or(Vec3::Z);
        if total[v] < 1e-4 {
            pos.push(p);
            nrm.push(q);
            continue;
        }
        let m = own_rest_inv * (sum[v] * (1.0 / total[v]));
        pos.push(m.transform_point3(p));
        nrm.push(m.transform_vector3(q).normalize_or_zero());
    }
    Some((pos, nrm))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shadow blob at the model's z = 0 is laid onto the plane through the wheels: 15 cm
    /// up with the body sagging, and following a pitch; one axle gives a level plane.
    #[test]
    fn shadow_blob_lies_on_the_wheels_plane() {
        let wheels = [
            Vec3::new(-1.0, 3.2, 0.13),
            Vec3::new(1.0, 3.2, 0.13),
            Vec3::new(-1.0, -2.6, 0.17),
            Vec3::new(1.0, -2.6, 0.17),
        ];
        let p = fit_plane(&wheels);
        for w in wheels {
            assert!(
                (p[0] + p[1] * w.x + p[2] * w.y - w.z).abs() < 1e-4,
                "{p:?} at {w:?}"
            );
        }
        let m = onto_plane(p);
        for corner in [Vec3::new(1.4, 5.7, 0.0), Vec3::new(-1.4, -5.7, 0.0)] {
            let q = m.transform_point3(corner);
            assert!((q.truncate() - corner.truncate()).length() < 1e-5);
            assert!(
                (q.z - (p[0] + p[2] * corner.y) - SHADOW_LIFT).abs() < 1e-4,
                "{q:?}"
            );
        }
        let axle = fit_plane(&[Vec3::new(-1.0, -0.4, 0.2), Vec3::new(1.0, -0.4, 0.1)]);
        assert!(
            (axle[0] - 0.15).abs() < 1e-5 && axle[1] == 0.0 && axle[2] == 0.0,
            "{axle:?}"
        );
    }

    /// The resolved property plan gives what `compute_mesh_props` gives, for a stock bus
    /// with its variables set to changing values, and after an engine variable joins.
    #[test]
    fn props_plan_matches_compute_mesh_props() {
        let root = omsi_cfg::env::var_os("OMSI_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("../../../OMSI 2 Original"));
        let bus = root.join("Vehicles/MAN_NL_NG/MAN_EN92_main.bus");
        if !bus.exists() {
            eprintln!("skipped: no {}", bus.display());
            return;
        }
        let ty = Arc::new(VehicleType::load(&root, &bus).expect("EN92"));
        let mut v = VehicleInstance::new(ty, VehicleHost::new(crate::SimClock::default()));
        let compare = |v: &VehicleInstance| {
            let mut plan = PropsPlan::default();
            plan.refresh(&v.ty, &v.var_index);
            let mut got = vec![MeshProps::default(); 3];
            plan.apply(&v.state.vars, &mut got);
            let want = compute_mesh_props(&v.ty, &|n| v.var(n));
            assert_eq!(got.len(), want.len());
            for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                assert_eq!(
                    (
                        g.visible,
                        &g.slot_alpha,
                        &g.slot_light,
                        &g.slot_night,
                        &g.slot_uv,
                        g.interior
                    ),
                    (
                        w.visible,
                        &w.slot_alpha,
                        &w.slot_light,
                        &w.slot_night,
                        &w.slot_uv,
                        w.interior
                    ),
                    "mesh {i} {}",
                    v.ty.model.meshes[v.ty.meshes[i].def_index].file
                );
            }
        };
        for round in 0..4 {
            for (k, x) in v.state.vars.iter_mut().enumerate() {
                *x = ((k * 7 + round * 13) % 5) as f32 * 0.37 - 0.2;
            }
            compare(&v);
        }
        // Dirt_Norm is not declared by the scripts; the engine adds it later
        v.set_engine_var("Dirt_Norm", 0.63);
        compare(&v);
    }

    /// The articulated GN92 turning right: the angle goes to `articulation_0_alpha` (the
    /// jackknife protection's), and the joint's last dummy - the bone the bellows' rear end
    /// hangs on - turns with it onto the rear section's line.
    #[test]
    fn articulation_alpha_turns_the_joint_onto_the_rear_section() {
        let root = omsi_cfg::env::var_os("OMSI_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("../../../OMSI 2 Original"));
        let bus = root.join("Vehicles/MAN_NL_NG/MAN_GN92_main.bus");
        let trail = root.join("Vehicles/MAN_NL_NG/MAN_GN92_trail.bus");
        if !bus.exists() || !trail.exists() {
            eprintln!("skipped: no {}", bus.display());
            return;
        }
        let ty = Arc::new(VehicleType::load(&root, &bus).expect("GN92"));
        let mut v = VehicleInstance::new(ty, VehicleHost::new(crate::SimClock::default()));
        v.attach_trailer_ex(
            Arc::new(VehicleType::load(&root, &trail).expect("GN92 trail")),
            false,
        );
        v.heading = 10.0;
        v.update_visuals(0.02);
        assert!(v.var("articulation_0_alpha").unwrap().abs() < 1e-3);
        // the front section turns right (clockwise) by 20 degrees about its rear coupling's
        // neighbourhood: the rear section still points the old way
        v.heading = 30.0;
        v.update_visuals(0.02);
        let t = &v.trailers[0];
        let alpha = v.var("articulation_0_alpha").unwrap();
        let beta = v.var("articulation_0_beta").unwrap();
        let geometric = ((v.heading - t.heading + 540.0) % 360.0) - 180.0;
        assert!(geometric > 5.0, "{geometric}");
        assert!(
            (alpha.abs() - geometric as f32).abs() < 1e-3,
            "alpha {alpha} for {geometric}"
        );
        assert!(beta.abs() < 1.0, "beta {beta}");
        // where the rear dummy points, in the front section's frame, against where the rear
        // section lies: (−sin a, cos a) forwards for the angle a
        let d =
            v.ty.meshes
                .iter()
                .position(|m| {
                    v.ty.model.meshes[m.def_index]
                        .file
                        .to_ascii_lowercase()
                        .ends_with("gelenk_d.o3d")
                })
                .expect("GN_Dum_Gelenk_D");
        let back = v.mesh_transforms[d].transform_vector3(-Vec3::Y);
        let a = (geometric as f32).to_radians();
        let want = Vec3::new(a.sin(), -a.cos(), 0.0);
        assert!(
            (back - want).length() < 0.02,
            "dummy D points {back:?}, the rear section {want:?} (alpha {alpha})"
        );
        // the bellows ([smoothskin] on the dummies) bend with it: the rear ring swings
        // towards the rear section's side, the front ring stays
        let skinned = v.skinned_meshes();
        assert!(!skinned.is_empty(), "no skinned mesh on the GN92");
        for i in skinned {
            let rest = v.ty.meshes[i].data.positions.clone();
            let (pos, nrm) = v.skinned(i).expect("skinned vertices");
            assert_eq!((pos.len(), nrm.len()), (rest.len(), rest.len()));
            let mean_dx = |f: &dyn Fn(f32) -> bool| {
                let d: Vec<f32> = pos
                    .iter()
                    .zip(&rest)
                    .filter(|(_, r)| f(r.y))
                    .map(|(p, r)| p.x - r.x)
                    .collect();
                d.iter().sum::<f32>() / d.len().max(1) as f32
            };
            let (front, rear) = (mean_dx(&|y| y > -3.8), mean_dx(&|y| y < -4.5));
            assert!(
                front.abs() < 0.05,
                "{}: front ring moved {front}",
                v.ty.model.meshes[v.ty.meshes[i].def_index].file
            );
            assert!(
                rear > 0.1,
                "{}: rear ring moved {rear}",
                v.ty.model.meshes[v.ty.meshes[i].def_index].file
            );
        }
        // straight: the modelled shape
        let mut s =
            VehicleInstance::new(v.ty.clone(), VehicleHost::new(crate::SimClock::default()));
        s.attach_trailer_ex(v.trailers[0].ty.clone(), false);
        s.update_visuals(0.02);
        let i = s.skinned_meshes()[0];
        let rest = s.ty.meshes[i].data.positions.clone();
        let (pos, _) = s.skinned(i).unwrap();
        let worst = pos
            .iter()
            .zip(&rest)
            .map(|(p, r)| (*p - *r).length())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-3, "straight bellows off by {worst}");
    }

    /// A timetable duty and a random traffic car load their bus with `VehicleType::load_ai`,
    /// which lets the vertices of every mesh go to save memory - except a `[smoothskin]`
    /// mesh (the bellows) has to keep its own, or there is nothing left to bend it from and
    /// every AI articulated bus (and the rear-section trailer of a `.zug` train) drove with
    /// its bellows frozen in the rest pose through every corner.
    #[test]
    fn ai_loaded_bellows_keep_their_skin_and_still_bend() {
        let root = omsi_cfg::env::var_os("OMSI_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("../../../OMSI 2 Original"));
        let bus = root.join("Vehicles/MAN_NL_NG/MAN_GN92_main.bus");
        let trail = root.join("Vehicles/MAN_NL_NG/MAN_GN92_trail.bus");
        if !bus.exists() || !trail.exists() {
            eprintln!("skipped: no {}", bus.display());
            return;
        }
        let ai = VehicleType::load_ai(&root, &bus).expect("GN92 (AI)");
        let skinned: Vec<usize> = (0..ai.meshes.len())
            .filter(|&i| !ai.meshes[i].skin.is_empty())
            .collect();
        assert!(
            !skinned.is_empty(),
            "the GN92 has no [smoothskin] mesh to check"
        );
        for &i in &skinned {
            assert!(
                !ai.meshes[i].data.positions.is_empty(),
                "{}: an AI copy dropped the bellows' vertices",
                ai.model.meshes[ai.meshes[i].def_index].file
            );
            assert!(
                !ai.mesh_dropped(i),
                "{}: an AI copy still marks the bellows as needing a re-read",
                ai.model.meshes[ai.meshes[i].def_index].file
            );
        }
        // every other mesh still lets its vertices go, exactly as before this fix
        assert!(
            (0..ai.meshes.len()).any(|i| ai.mesh_dropped(i)),
            "the AI type keeps meshes it should have let go"
        );

        let ty = Arc::new(ai);
        let mut v = VehicleInstance::new(ty, VehicleHost::new(crate::SimClock::default()));
        v.attach_trailer_ex(
            Arc::new(VehicleType::load_ai(&root, &trail).expect("GN92 trail (AI)")),
            false,
        );
        v.heading = 10.0;
        v.update_visuals(0.02);
        v.heading = 30.0;
        v.update_visuals(0.02);
        let i = skinned[0];
        let rest = v.ty.meshes[i].data.positions.clone();
        let (pos, nrm) = v.skinned(i).expect("skinned vertices of an AI copy");
        assert_eq!((pos.len(), nrm.len()), (rest.len(), rest.len()));
        let moved = pos
            .iter()
            .zip(&rest)
            .map(|(p, r)| (*p - *r).length())
            .fold(0.0f32, f32::max);
        assert!(
            moved > 0.1,
            "the AI copy's bellows did not bend with the joint (moved {moved})"
        );
    }

    /// The same check on the O530G Facelift / Citaro LE mod pack (`OMSI_CONTENT`, or the
    /// user's overlay by default): a third-party model with its own bone names
    /// (`Armature_Gelenk_A-D` on `18m_main\bellows_out.o3d` / `bellows_in.o3d`, against the
    /// stock GN92's `Gelenk_A-D` on its own files) still bends, both loaded normally and
    /// loaded as an AI copy - the fix is generic over `[setbone]` names, not tied to MAN's.
    #[test]
    fn o530g_mod_bellows_bend_and_survive_an_ai_load() {
        let content = omsi_cfg::env::var_os("OMSI_CONTENT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(
                    "/Users/savva/OMSI 2 Source Code/openOMSI/target/release",
                )
            });
        let bus = content.join("Vehicles/MB_O530_Facelift/MB_O530GFL EL 3D Main.bus");
        let trail = content.join("Vehicles/MB_O530_Facelift/MB_O530GFL EL 3D Trail.bus");
        if !bus.exists() || !trail.exists() {
            eprintln!("skipped: no {}", bus.display());
            return;
        }
        // loaded fully (the player's own bus): bends like the GN92
        let ty = Arc::new(VehicleType::load(&content, &bus).expect("O530G"));
        let skinned = ty
            .meshes
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.skin.is_empty())
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        assert!(
            !skinned.is_empty(),
            "the O530G has no [smoothskin] mesh to check (bellows_out/bellows_in)"
        );
        let mut v = VehicleInstance::new(ty, VehicleHost::new(crate::SimClock::default()));
        v.attach_trailer_ex(
            Arc::new(VehicleType::load(&content, &trail).expect("O530G trail")),
            false,
        );
        v.heading = 10.0;
        v.update_visuals(0.02);
        v.heading = 30.0;
        v.update_visuals(0.02);
        for &i in &skinned {
            let rest = v.ty.meshes[i].data.positions.clone();
            let (pos, nrm) = v.skinned(i).expect("skinned vertices");
            assert_eq!((pos.len(), nrm.len()), (rest.len(), rest.len()));
            let moved = pos
                .iter()
                .zip(&rest)
                .map(|(p, r)| (*p - *r).length())
                .fold(0.0f32, f32::max);
            assert!(
                moved > 0.1,
                "{}: did not bend with the joint (moved {moved})",
                v.ty.model.meshes[v.ty.meshes[i].def_index].file
            );
        }
        // loaded as an AI copy (the timetable's or a random traffic bus): same bones kept
        let ai = VehicleType::load_ai(&content, &bus).expect("O530G (AI)");
        for &i in &skinned {
            assert!(
                !ai.meshes[i].data.positions.is_empty(),
                "an AI copy dropped the O530G's bellows vertices"
            );
        }
    }

    /// `OMSI_DEBUG_WHEELS` reports the wheels of an AI type, whose vertices were let go after
    /// loading, exactly as it reports those of a fully loaded one.
    #[test]
    fn wheel_pivot_report_reads_dropped_meshes() {
        let root = omsi_cfg::env::var_os("OMSI_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("../../../OMSI 2 Original"));
        let bus = root.join("Vehicles/MAN_F90/AI_MAN_F90_Wechselbruecke.bus");
        if !bus.exists() {
            eprintln!("skipped: no {}", bus.display());
            return;
        }
        let report = |ty: VehicleType| {
            VehicleInstance::new(Arc::new(ty), VehicleHost::new(crate::SimClock::default()))
                .wheel_pivot_report()
        };
        let ai = VehicleType::load_ai(&root, &bus).expect("F90");
        assert!(
            (0..ai.meshes.len()).any(|i| ai.mesh_dropped(i)),
            "the AI type keeps its vertices"
        );
        let full = report(VehicleType::load(&root, &bus).expect("F90"));
        assert!(!full.is_empty(), "the F90 has no animated wheels");
        assert_eq!(report(ai), full);
    }
}

/// The relative humidity (a fraction, 1 = saturated) of air at `t` °C holding `abs_hum` g/m³,
/// as OMSI computes `Cabinair_relHum` (the original, Magnus over water with 7.5/237.3,
/// over ice with 7.6/240.7 below 0 °C). It was written as a percentage: 100 times what the
/// scripts and the passengers (a humid saloon at over 0.9) expect.
pub fn relative_humidity(t: f32, abs_hum: f32) -> f32 {
    let (a, b) = if t >= 0.0 { (7.5, 237.3) } else { (7.6, 240.7) };
    let sat = 1323.480_3 * 10f32.powf(a * t / (b + t)) / (273.15 + t);
    if sat > 0.0 && sat.is_finite() {
        (abs_hum / sat).max(0.0)
    } else {
        0.0
    }
}

/// The tyres' friction coefficient on the road under them: `street_cond` as the weather sets
/// it (0 dry … 1 wet, 1 … 2 snow from packed to fresh) and the air temperature (°C), below
/// which a wet road freezes (the stock "Ueberfrierende Naesse" weather). Dry asphalt 0.85,
/// wet 0.6, snow 0.3, black ice 0.12 - textbook values for truck tyres; OMSI keeps its
/// friction inside ode.dll and only hands `StreetCond` to the scripts, so these are not
/// read off it.
pub fn road_grip(street_cond: f32, temperature: f32) -> f32 {
    const DRY: f32 = 0.85;
    const WET: f32 = 0.6;
    const SNOW: f32 = 0.3;
    const ICE: f32 = 0.12;
    let c = street_cond.clamp(0.0, 2.0);
    if c <= 1.0 {
        let wet = DRY + (WET - DRY) * c;
        // a wet road at or below freezing turns to ice as the film freezes; a damp one is
        // black ice all over (the stock "Ueberfrierende Naesse": -6 degC, groundwet 0.31)
        if temperature <= 0.0 && c > 0.05 {
            let frozen = ((0.5 - temperature) / 3.0).clamp(0.0, 1.0) * (c / 0.25).min(1.0);
            wet + (ICE - wet) * frozen
        } else {
            wet
        }
    } else {
        WET + (SNOW - WET) * (c - 1.0)
    }
}

#[cfg(test)]
mod grip_tests {
    use super::road_grip;

    #[test]
    fn wet_snowy_and_frozen_roads_hold_less() {
        assert_eq!(road_grip(0.0, 15.0), 0.85);
        assert!((road_grip(1.0, 15.0) - 0.6).abs() < 1e-6);
        assert!((road_grip(2.0, -5.0) - 0.3).abs() < 1e-6);
        assert!(road_grip(1.0, -3.0) < 0.2, "black ice");
        assert_eq!(road_grip(0.0, -10.0), 0.85, "a dry road does not freeze");
    }

    /// A `[smoothskin]` vertex hung partly on an o3d bone no `[setbone]` names keeps that
    /// share where it was modelled: the AA-FR Agora's retarder lever (`retarderhebel_2.o3d`)
    /// splits its vertices between `Bone` (unbound) and `Bone.001` (the animated dummy), and
    /// with the unbound share dropped the lever bent out of shape as it moved.
    #[test]
    fn unbound_bones_keep_their_share_of_a_vertex() {
        use super::*;
        use std::sync::Arc;
        let root = omsi_cfg::env::var_os("OMSI_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("../../../OMSI 2 Original"));
        let bus = root.join("Vehicles/AA-FR_BusBundle/2002_Agora_S_2d.bus");
        if !bus.exists() {
            eprintln!("skipped: no {}", bus.display());
            return;
        }
        let ty = Arc::new(VehicleType::load(&root, &bus).expect("Agora S"));
        let i = (0..ty.meshes.len())
            .find(|&i| ty.model.meshes[ty.meshes[i].def_index].file.to_ascii_lowercase().contains("retarderhebel_2"))
            .expect("the Agora's retarder lever");
        let vm = &ty.meshes[i];
        let bound = vm.skin.iter().find(|b| b.def_index.is_some()).expect("Bone.001");
        assert!(vm.skin.iter().any(|b| b.def_index.is_none()), "the unbound Bone is kept");
        let k = ty.meshes.iter().position(|m| Some(m.def_index) == bound.def_index).unwrap();
        let mut v = VehicleInstance::new(ty.clone(), VehicleHost::new(crate::SimClock::default()));
        v.update_visuals(0.02);
        let rest = v.mesh_transforms.clone();
        assert!(v.set_var("cp_retarder_hebel", 4.0));
        for _ in 0..200 {
            v.update_visuals(0.05);
        }
        let bone = v.mesh_transforms[k] * rest[k].inverse();
        assert!(bone.abs_diff_eq(Mat4::IDENTITY, 1e-3) == false, "the lever's dummy did not move");
        let (pos, _) = v.skinned(i).expect("skinned lever");
        let mut checked = 0;
        for &(vi, w) in &bound.weights {
            let vi = vi as usize;
            if !(0.2..0.8).contains(&w) {
                continue;
            }
            let p = vm.data.positions[vi];
            // (1 - w) where it was modelled, w with the bone, in the mesh's own frame
            let want = rest[i].inverse() * (Mat4::IDENTITY * (1.0 - w) + bone * w);
            let want = want.transform_point3(p);
            assert!((pos[vi] - want).length() < 1e-4, "vertex {vi} (weight {w}): {:?} for {:?}", pos[vi], want);
            checked += 1;
        }
        assert!(checked > 0, "no vertex shared between the two bones");
    }

    /// The joint's vertical angle turns the Agora L's arch (`anim_rot articulation_0_beta
    /// 0.5`) and the bone its bellows' far ring hangs on half-way towards the rear section,
    /// whichever way the front section pitches.
    #[test]
    fn articulation_beta_tilts_the_joint_towards_the_rear_section() {
        use super::*;
        use std::sync::Arc;
        let root = omsi_cfg::env::var_os("OMSI_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("../../../OMSI 2 Original"));
        let bus = root.join("Vehicles/AA-FR_BusBundle/2002_Agora_L_3d_main.bus");
        let trail = root.join("Vehicles/AA-FR_BusBundle/2002_Agora_L_3d_trail.bus");
        if !bus.exists() || !trail.exists() {
            eprintln!("skipped: no {}", bus.display());
            return;
        }
        let ty = Arc::new(VehicleType::load(&root, &bus).expect("Agora L"));
        let mut v = VehicleInstance::new(ty, VehicleHost::new(crate::SimClock::default()));
        v.attach_trailer_ex(Arc::new(VehicleType::load(&root, &trail).expect("Agora L trail")), false);
        v.update_visuals(0.02);
        let find = |v: &VehicleInstance, n: &str| {
            v.ty.meshes
                .iter()
                .position(|m| v.ty.model.meshes[m.def_index].file.to_ascii_lowercase().ends_with(n))
                .expect(n)
        };
        let (arch, bone_b) = (find(&v, "gelenk_arch.o3d"), find(&v, "bone_b.o3d"));
        for pitch in [4.0f32, -4.0] {
            v.pitch = pitch;
            for _ in 0..50 {
                v.update_visuals(0.05);
            }
            // where the rear section lies, in the front section's frame
            let rel = v.body_rotation().inverse() * v.trailers[0].body_rotation();
            let back = rel.transform_vector3(-Vec3::Y);
            assert!(back.z.abs() > 0.03, "the sections are not pitched apart ({back:?})");
            for k in [arch, bone_b] {
                let d = v.mesh_transforms[k].transform_vector3(-Vec3::Y);
                assert!(
                    d.z * back.z > 0.0 && d.z.abs() < back.z.abs(),
                    "pitch {pitch}: {} points {d:?}, the rear section {back:?}",
                    v.ty.model.meshes[v.ty.meshes[k].def_index].file
                );
            }
        }
    }
}
