//! Placing an uploaded tile in the scene a slice at a time.
use super::*;

/// Add a render instance (in a free slot of the GPU cache) and keep it with the tile's
/// resources, so that it goes when the tile does.
macro_rules! instance {
    ($gpu:ident, $renderer:ident, $scene:ident, $tg:ident; $id:expr) => {{
        let new = $id;
        let i = $gpu.instance($renderer, $scene, new);
        $tg.instances.push(i);
        i
    }};
}

/// The materials and meshes every tile shares (see [`GroundGpu`]), copied out of the GPU
/// cache for one [`World::place_step`].
#[derive(Clone, Copy)]
struct GroundIds {
    ground_id: Option<TextureId>,
    ground_mat: MaterialId,
    plain_terrain_mat: MaterialId,
    ground_detail: Option<(TextureId, f32)>,
    ground_repeats: f32,
    ground_wet: f32,
    water_mat: MaterialId,
    tree_mesh: MeshId,
}

/// One object of a tile being placed: where it stands, what it is and what its type has
/// on the GPU.
#[derive(Clone, Copy)]
struct ObjectCx<'o> {
    ot: &'o Arc<ObjectType>,
    pos: DVec3,
    xf: Mat4,
    lamp: Option<(i64, usize, bool)>,
    map_id: i64,
    controller: Option<usize>,
    strings: &'o [String],
    var_parent: Option<i64>,
    script_strings: &'o [String],
    /// The tile's key.
    key: (i32, i32),
    tkey: usize,
    type_variants: &'o [(usize, usize, MaterialId, MaterialId, String)],
    type_auto_night: bool,
    lod0_lo: f32,
    lod0_max: f32,
    surface: bool,
    render_phase: RenderPhase,
    has_lower: bool,
    has_pages: bool,
    images: &'o HashMap<PathBuf, Arc<TextureData>>,
}

/// What placing one object made: its instances, and what its lamp or its script drives.
#[derive(Default)]
struct ObjectMade {
    lamp_instances: Vec<(usize, Option<(String, f32)>)>,
    lamp_slots: Vec<LampSlots>,
    all_instances: Vec<usize>,
    object_variants: Vec<(usize, usize, MaterialId, MaterialId, String)>,
    script_texts: Vec<(TextureId, omsi_sim::texttex::TextTextureState)>,
    lamp_texts: Vec<(TextureId, omsi_sim::texttex::TextTextureState)>,
    /// The materials this placement's own `[texttexture]`s made: (slot, is item, material),
    /// to keep them on a `[matl_change]` slot (see below).
    text_slot_mats: Vec<(usize, bool, MaterialId)>,
    /// `[htmltexture]` pages shown on this object: (script texture index, texture)
    html_pages: Vec<(usize, TextureId)>,
    html_mats: HashMap<usize, MaterialId>,
}

impl World {
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
        let ground = {
            let g = gpu.ground.as_ref().unwrap();
            GroundIds {
                ground_id: g.ground_id,
                ground_mat: g.ground_mat,
                plain_terrain_mat: g.plain_terrain_mat,
                ground_detail: g.ground_detail,
                ground_repeats: g.ground_repeats,
                ground_wet: g.ground_wet,
                water_mat: g.water_mat,
                tree_mesh: g.tree_mesh,
            }
        };
        let PendingUpload {
            prepared: p,
            tg,
            placing: pl,
            ..
        } = u;
        let key = (p.tx, p.ty);
        let only_object = omsi_cfg::flags::OMSI_ONLY_OBJECT.var().is_some();
        let decodes_before = (gpu.sync_decodes, gpu.sync_decode_secs);
        let t_start = std::time::Instant::now();
        let out_of_time = |done_some: bool| {
            done_some
                && deadline
                    .map(|d| std::time::Instant::now() >= d)
                    .unwrap_or(false)
        };
        let mut done_some = false;
        while pl.phase < 4 && !out_of_time(done_some) {
            let t_phase = std::time::Instant::now();
            let phase = pl.phase;
            match phase {
                0 => {
                    if let (Some(mesh), false) = (&p.terrain, only_object) {
                        self.place_ground(renderer, scene, gpu, p, tg, pl, mesh, &ground);
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
                            let si = instance!(gpu, renderer, scene, tg; renderer.add_surface_instance(scene, id, p.origin, Mat4::IDENTITY, vec![mat]));
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
                    self.place_spline(renderer, scene, gpu, p, tg, pl, ground.ground_mat);
                    pl.splines += 1;
                    done_some = true;
                }
                2 => {
                    if only_object || pl.next >= p.trees.len() {
                        pl.phase = 3;
                        pl.next = 0;
                        continue;
                    }
                    self.place_trees(renderer, scene, gpu, p, tg, pl, ground.tree_mesh);
                    done_some = true;
                }
                _ => {
                    let Some(o) = p.objects.pop() else {
                        pl.phase = 4;
                        continue;
                    };
                    self.place_object(renderer, scene, gpu, p, tg, pl, o, key, ground.ground_mat);
                    pl.objects += 1;
                    done_some = true;
                }
            }
            pl.secs[phase.min(3) as usize] += t_phase.elapsed().as_secs_f64();
        }
        let done = pl.phase >= 4;
        log_place_step(gpu, pl, key, decodes_before, t_start, lock_wait, done);
        done
    }

    /// The ground of a tile: its terrain (cut under the roads and lit by its light map),
    /// the walls of the holes, the painted ground layers and the water.
    #[allow(clippy::too_many_arguments)]
    fn place_ground(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        gpu: &mut GpuCache,
        p: &Prepared,
        tg: &mut TileGpu,
        pl: &mut Placing,
        mesh: &MeshData,
        g: &GroundIds,
    ) {
        let GroundIds { ground_id, plain_terrain_mat, ground_detail, ground_repeats, ground_wet, water_mat, .. } = *g;
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
        let ground_instance = instance!(gpu, renderer, scene, tg; renderer.add_instance(
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
            let wi = instance!(gpu, renderer, scene, tg; renderer.add_instance(
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
        self.place_ground_paint(renderer, scene, gpu, p, tg, id, wall_id, lm);
        place_water(renderer, scene, gpu, p, tg, water_mat);
    }

    #[allow(clippy::too_many_arguments)]
    fn place_ground_paint(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        gpu: &mut GpuCache,
        p: &Prepared,
        tg: &mut TileGpu,
        id: MeshId,
        wall_id: Option<MeshId>,
        lm: Option<TextureId>,
    ) {
        let images: &HashMap<PathBuf, Arc<TextureData>> = &p.images;
        let ground_dirs = vec![self.root.clone()];
        // The painted ground: every further [groundtex] the editor's brush put on this
        // tile is the same tile mesh once more, blended in through its own mask - which
        // is how OMSI's car parks get their asphalt, its side streets their cobbles and
        // its meadows their fields.
        let no_paint = omsi_cfg::flags::OMSI_NO_GROUND_PAINT.is_set();
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
            let li = instance!(gpu, renderer, scene, tg; renderer.add_surface_instance(
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
            if omsi_cfg::flags::OMSI_DEBUG_SURFACES.is_set() {
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
                let wi = instance!(gpu, renderer, scene, tg; renderer.add_surface_instance(
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
    }

    /// The next spline of the tile (`pl.next`), with its type's materials made the first
    /// time the type comes up.
    #[allow(clippy::too_many_arguments)]
    fn place_spline(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        gpu: &mut GpuCache,
        p: &Prepared,
        tg: &mut TileGpu,
        pl: &mut Placing,
        ground_mat: MaterialId,
    ) {
        let images: &HashMap<PathBuf, Arc<TextureData>> = &p.images;
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
        if omsi_cfg::flags::OMSI_DEBUG_SPLINES.is_set() {
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
                let terrain_instance = instance!(gpu, renderer, scene, tg; renderer.add_surface_instance(
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
            let si = instance!(gpu, renderer, scene, tg; renderer.add_surface_instance(
                scene, id, p.origin, Mat4::IDENTITY, mats.clone()
            ));
            if let Some(inst) = scene.instances.get_mut(si) {
                inst.render_phase = phase;
                inst.blend_sort_origin = Some(*sort_origin);
            }
            // Preserve the deck's shadow geometry after splitting its slots.
            if *casts_shadow { renderer.set_casts_shadow(scene, si, true); }
        }
    }

    /// The next few dozen trees of the tile (from `pl.next`).
    #[allow(clippy::too_many_arguments)]
    fn place_trees(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        gpu: &mut GpuCache,
        p: &Prepared,
        tg: &mut TileGpu,
        pl: &mut Placing,
        tree_mesh: MeshId,
    ) {
        let images: &HashMap<PathBuf, Arc<TextureData>> = &p.images;
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
                instance!(gpu, renderer, scene, tg; renderer.add_instance(scene, tree_mesh, *pos, xf, vec![mat]));
            pl.trees += 1;
        }
        pl.next = end;
    }

    /// One object of the tile: its meshes and lower levels, its lamp or its script, and
    /// what the poles, the route arrows, the parked cars and the object editor keep of it.
    #[allow(clippy::too_many_arguments)]
    fn place_object(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        gpu: &mut GpuCache,
        p: &Prepared,
        tg: &mut TileGpu,
        pl: &mut Placing,
        o: PlacedObject,
        key: (i32, i32),
        ground_mat: MaterialId,
    ) {
        let images: &HashMap<PathBuf, Arc<TextureData>> = &p.images;
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
        let mut made = ObjectMade::default();
        let (mut object_script, freetex_probe, has_pages) =
            self.object_scripts(&ot, lamp, early_script.take(), script_strings);
        let obj = ObjectCx {
            ot: &ot,
            pos,
            xf,
            lamp,
            map_id,
            controller,
            strings: &strings,
            var_parent,
            script_strings,
            key,
            tkey,
            type_variants: &type_variants,
            type_auto_night,
            lod0_lo,
            lod0_max,
            surface,
            render_phase,
            has_lower,
            has_pages,
            images,
        };
        let (mesh_list, ground_meshes) = object_meshes(
            renderer,
            scene,
            gpu,
            p,
            tg,
            pl.terrain_mapping_mat.is_some(),
            &obj,
            (warped.as_deref(), &type_meshes, &terrain_slots),
            &mut type_lods,
        );
        for (mi, (mesh_id, mats)) in mesh_list.iter().enumerate() {
            self.place_object_mesh(
                renderer,
                scene,
                gpu,
                tg,
                pl,
                &obj,
                (mi, mesh_id, mats),
                &mut made,
                (object_script.as_ref(), freetex_probe.as_ref()),
            );
        }
        // (the meshes' own instances: a script poses them one by one, the ground
        // drawn in the [terrainmapping] slots after them keeps the object's place)
        let mesh_instances = made.all_instances.len();
        let lod_instances =
            place_object_lods(renderer, scene, gpu, tg, pl, &obj, &ground_meshes, &type_lods, &mut made.all_instances);
        let all_instances = &made.all_instances;
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
                scene.instances[*inst].shadow_owner =
                    ot.sco.crash_mode_pole.is_some().then_some(collision_key);
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
        if let Some(lamp) = lamp {
            self.keep_light_object(pl, &obj, lamp, made);
        } else if let Some(inst) = object_script.take() {
            self.keep_scripted_object(renderer, scene, gpu, &obj, inst, made, mesh_instances);
        }
    }

    /// The script instances an object needs before its meshes are placed: its own (its
    /// {init} run once), a probe for `[matl_freetex]` names, and whether it shows
    /// `[htmltexture]` pages.
    fn object_scripts(
        &self,
        ot: &Arc<ObjectType>,
        lamp: Option<(i64, usize, bool)>,
        mut early_script: Option<omsi_sim::scenery::SceneryInstance>,
        script_strings: &[String],
    ) -> (Option<omsi_sim::scenery::SceneryInstance>, Option<omsi_sim::scenery::SceneryInstance>, bool) {
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
        let object_script = if needs_own_script {
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
        (object_script, freetex_probe, has_pages)
    }

    /// One mesh of an object: its instance, its own `[matl_freetex]`, `[texttexture]` and
    /// `[htmltexture]` materials, and its `[matl_change]` variants and lamp slots.
    #[allow(clippy::too_many_arguments)]
    fn place_object_mesh(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        gpu: &mut GpuCache,
        tg: &mut TileGpu,
        pl: &mut Placing,
        obj: &ObjectCx,
        (mi, mesh_id, mats): (usize, &MeshId, &Vec<MaterialId>),
        made: &mut ObjectMade,
        (object_script, freetex_probe): (Option<&omsi_sim::scenery::SceneryInstance>, Option<&omsi_sim::scenery::SceneryInstance>),
    ) {
        let ObjectCx { ot, pos, xf, lamp, map_id, tkey: _, type_variants, type_auto_night, lod0_lo, lod0_max, surface, render_phase, has_lower, has_pages, images, script_strings, .. } = *obj;
        let ObjectMade { lamp_instances, lamp_slots, all_instances, object_variants, script_texts, lamp_texts, text_slot_mats, html_pages, html_mats } = made;
        let inst = if surface || ot.mesh_shadow.get(mi).copied().unwrap_or(false) {
            let i = instance!(gpu, renderer, scene, tg; renderer.add_surface_instance(
                scene,
                *mesh_id,
                pos,
                xf,
                mats.clone()
            ));
            // What stands on a surface object casts its shadow: a `[shadow]`
            // mesh, or (casters "all") one rising more than 1.5 m over the
            // object's foot. Drawn as a ground layer it cast none - the
            // Spandau depot's buildings, made one object with its yard,
            // threw no shadow at all (#1503).
            if surface && !ot.mesh_shadow.get(mi).copied().unwrap_or(false) {
                let tagged = ot.mesh_casts.get(mi).copied().unwrap_or(false);
                let tall = ot.meshes.get(mi).is_some_and(|(m, _, _)| m.positions.iter().any(|p| p.z > 1.5));
                if tagged || tall {
                    renderer.set_casts_shadow(scene, i, true);
                    renderer.set_omsi_caster(scene, i, tagged);
                }
            }
            i
        } else {
            let i = instance!(gpu, renderer, scene, tg; renderer.add_instance(scene, *mesh_id, pos, xf, mats.clone()));
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
                    object_script,
                    freetex_probe,
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
        self.object_text_textures(renderer, scene, gpu, tg, obj, (mi, inst), (&mut *text_slot_mats, &mut *script_texts, &mut *lamp_texts));
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
            object_html_pages(renderer, scene, gpu, tg, ot, (mi, inst), html_pages, html_mats);
        }
        all_instances.push(inst);
    }

    /// `[texttexture]` + `[useTextTexture]` on one mesh of an object.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn object_text_textures(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        gpu: &mut GpuCache,
        tg: &mut TileGpu,
        obj: &ObjectCx,
        (mi, inst): (usize, usize),
        (text_slot_mats, script_texts, lamp_texts): (
            &mut Vec<(usize, bool, MaterialId)>,
            &mut Vec<(TextureId, omsi_sim::texttex::TextTextureState)>,
            &mut Vec<(TextureId, omsi_sim::texttex::TextTextureState)>,
        ),
    ) {
        let ObjectCx { ot, lamp, script_strings, .. } = *obj;
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
    }

    /// Keep a traffic lamp (or another object a crossing's light program drives) for the
    /// traffic to switch.
    fn keep_light_object(&self, pl: &mut Placing, obj: &ObjectCx, lamp: (i64, usize, bool), made: ObjectMade) {
        let ObjectCx { ot, pos, xf, script_strings, .. } = *obj;
        let ObjectMade { lamp_instances, lamp_slots, object_variants, lamp_texts, .. } = made;
        let (parent, index, any_light) = lamp;
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
    }

    /// Keep an object whose script changes it (its meshes, materials, texts, pages or sound)
    /// for `update_scripted`.
    #[allow(clippy::too_many_arguments)]
    fn keep_scripted_object(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        gpu: &mut GpuCache,
        obj: &ObjectCx,
        inst: omsi_sim::scenery::SceneryInstance,
        made: ObjectMade,
        mesh_instances: usize,
    ) {
        let ObjectCx { ot, pos, xf, lamp, map_id, controller, strings, var_parent, key, tkey, images, .. } = *obj;
        let ObjectMade { mut all_instances, object_variants, script_texts, html_pages, .. } = made;
        let texture_selection = scenery_texture_selection(ot, &inst);
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
            || ot.has_mouse_events
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
                    light_child_of(&self.index().traffic_light_parents, var_parent, strings)
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
}

mod parts;
use parts::*;
