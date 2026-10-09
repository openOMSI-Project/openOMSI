//! Placing a staged tile: crossings warped onto the ground, final ground, objects.
use super::*;

impl World {
    /// The crossings of `st` warped onto the ground: object index → its own meshes.
    ///
    /// A junction plate is one flat object covering a couple of hundred metres, and the
    /// roads running into it do not all lie at its height. `[crossing_heightdeformation]`
    /// names a coarse height field in the plate's own frame: every vertex of the plate is
    /// raised by it, so the plate keeps its kerbs and camber and its arms meet their roads.
    pub(super) fn warp_crossings(
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
            if biggest > 1.0 && omsi_cfg::flags::OMSI_DEBUG_WARP.is_set() {
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
    pub(super) fn final_ground(
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
        if omsi_cfg::flags::OMSI_TERRAIN_ALIGN.is_set() {
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
        if omsi_cfg::flags::OMSI_CROSSING_DEFORM.is_set() {
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
    pub(super) fn place_tile(
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
        let debug_objects = omsi_cfg::flags::OMSI_DEBUG_OBJECTS.is_set();
        let check_objects = omsi_cfg::flags::OMSI_CHECK_OBJECTS.is_set();
        let debug_float = omsi_cfg::flags::OMSI_DEBUG_FLOAT.is_set();
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
            if let Some(mut tree) = tree_of(&ot, o, pos, heading) {
                // (a season's phase: this tree's own look)
                tree.1 = self.tree_look_texture(&ot, &tree.1, o.key, pos);
                trees.push(tree);
                continue;
            }
            // Stock junctions carry a light program even where the map places no signals.
            // Use the map-wide index so lamps on an unloaded neighbouring tile still count.
            let controller = self.object_controller(&ot, o, pos, &index);
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
                lanes.extend(placed_object_lanes(&ot, o, pos, xf, heading, controller, key));
            }
            self.object_collision(&mut state, &st, &ot, o, (pos, xf, heading), is_surface);
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
            let lamp = object_lamp(&ot, o, pos, &index);
            // lights of the placed object
            object_lights(&mut state, &ot, pos, xf, lamp, o.key);
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
        note_place_stats(stats, &st, &res, objects.len());
        self.tile_state.lock().insert(key, state);
        let light_map = self.tile_light_map(key, &st, layout, staged);
        let (splines, ground_splines) = self.tile_splines(&st);
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

    /// The light program of a crossing object (its index in `traffic_lights`), made the
    /// first time the object comes up.
    fn object_controller(&self, ot: &Arc<ObjectType>, o: &StagedObject, pos: DVec3, index: &MapIndex) -> Option<usize> {
        if !traffic_light_program_enabled(&ot.sco, index.traffic_light_parents.contains(&o.id)) {
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
                if omsi_cfg::flags::OMSI_DEBUG_TRAFFIC.is_set() {
                    log::info!("traffic light program {idx}: object {} {} at ({:.1}, {:.1}) cycle {:?} lights {}", o.id, ot.sco.path.display(), pos.x, pos.y, ot.sco.traffic_lights_group, ot.sco.traffic_lights.len());
                }
                idx
            }))
        }
    }

    /// What vehicles hit of a placed object: its collision mesh or its bounding box.
    fn object_collision(
        &self,
        state: &mut TileState,
        st: &StagedTile,
        ot: &Arc<ObjectType>,
        o: &StagedObject,
        (pos, xf, heading): (DVec3, Mat4, f64),
        is_surface: bool,
    ) {
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
                if omsi_cfg::flags::OMSI_DEBUG_COLLISION.is_set() {
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
                if road_through && omsi_cfg::flags::OMSI_DEBUG_COLLISION.is_set() {
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
                    if omsi_cfg::flags::OMSI_DEBUG_COLLISION.is_set() {
                        log::info!("obstacle {} key {} at ({:.1}, {:.1}) z {:.1}..{:.1} size {:.1}x{:.1}x{:.1} centre offset ({:.1}, {:.1}) rot {:.0}{}{}", ot.sco.path.display(), o.key, pos.x, pos.y, obb.z0, obb.z1, bb[0], bb[1], bb[2], bb[3], bb[4], heading, if obb.pole.is_some() { " pole" } else { "" }, " [boundingbox]");
                    }
                }
            }
        }
    }

    /// The tile's night light map: baked from the lamps round it, or read from the map.
    fn tile_light_map(
        &self,
        key: (i32, i32),
        st: &StagedTile,
        layout: &TileLayout,
        staged: &HashMap<(i32, i32), Arc<StagedTile>>,
    ) -> Option<omsi_texture::Image> {
        let (tx, ty) = key;
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
        light_map
    }

    /// The tile's spline meshes for the GPU: their `[terrainmapping]` faces pooled apart,
    /// the rest batched.
    #[allow(clippy::type_complexity)]
    fn tile_splines(&self, st: &StagedTile) -> (Vec<(Arc<MeshData>, Arc<SplineType>, bool, DVec3)>, Vec<Arc<MeshData>>) {
        // the whole spline meshes go to the GPU from here (a later load reads the tile again)
        let meshes = st.meshes.lock().take().unwrap_or_default();
        let splines: Vec<_> = meshes
            .into_iter()
            .zip(st.splines.iter())
            .map(|(m, sp)| (m, sp.ty.clone(), sp.casts_shadow, sp.sort_origin))
            .collect();
        let (splines, ground_splines) = if omsi_cfg::flags::OMSI_NO_GROUND_SPLINE_BATCHING.is_set()
            || omsi_cfg::flags::OMSI_NO_SPLINE_BATCHING.is_set()
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
        (splines, ground_splines)
    }
}

/// A tree billboard of a placed object: (type, texture, position, height, width, heading).
fn tree_of(ot: &Arc<ObjectType>, o: &StagedObject, pos: DVec3, heading: f64) -> Option<(Arc<ObjectType>, String, DVec3, f64, f64, f64)> {
    let (tex, min_h, max_h, min_r, max_r) = ot.sco.tree.as_ref()?;
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
    Some((ot.clone(), texture, pos, height, height * ratio, heading))
}

/// The paths of a placed object, tilted and deformed with it.
fn placed_object_lanes(
    ot: &ObjectType,
    o: &StagedObject,
    pos: DVec3,
    xf: Mat4,
    heading: f64,
    controller: Option<usize>,
    key: (i32, i32),
) -> Vec<Lane> {
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
    own
}

/// Whether a placed object is a traffic lamp: (crossing, light index, lit by any light).
fn object_lamp(ot: &ObjectType, o: &StagedObject, pos: DVec3, index: &MapIndex) -> Option<(i64, usize, bool)> {
    // (OMSI hands `TrafficLightPhase` to any child of a crossing whose first string
    // names one of its lights, `[trafficlight]` or not - see `names_traffic_light`;
    // a mod lamp without the keyword sat at its "off" picture, blinking yellow.
    // Objects with textures of their own to choose stay ordinary objects.)
    let child_lamp = o.lamp_parent.is_some_and(|p| index.traffic_light_parents.contains(&p))
        && crate::tiles::names_traffic_light(&o.extra)
        && ot.dynamic_textures.is_empty()
        && !ot.meshes.iter().any(|(_, _, ov)| ov.iter().any(|m| !m.item && m.freetex.is_some()));
    if ot.sco.is_traffic_light || child_lamp {
        let named = o.extra.first().map(|s| s.trim()).filter(|s| !s.is_empty());
        let index = named.map(|s| omsi_cfg::parse_f64(s) as usize).unwrap_or(0);
        if omsi_cfg::flags::OMSI_DEBUG_LAMPS.is_set() {
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
    }
}

/// Legacy pole fixtures can put the virtual `[maplight]` inside the emitting mesh
/// (Spandau's Ufo Big puts it above the pole cap, inside the head). The source was
/// authored for an unoccluded light map, not as a bulb hidden behind opaque geometry.
/// Require a pole, an enclosing emitting mesh, and a directional emitter whose
/// output surface is in front of the source. A maplight in a pole shaft below the
/// lamp, or coincident with its output, is not this embedded-head case.
fn embedded_pole_light(ot: &ObjectType, source: glam::Vec3) -> bool {
    ot.sco.crash_mode_pole.is_some() && ot.meshes.iter().zip(&ot.mesh_def_index).any(|(mesh, &def)| {
        let md = &ot.model.meshes[def];
        md.light_enh_2.iter().any(|effect| {
            !effect.omni
                && (source - glam::Vec3::from(effect.pos)).dot(glam::Vec3::from(effect.dir)) < 0.0
        }) && mesh_encloses_light(&mesh.0, source)
    })
}

/// Generalized winding number: an enclosed point subtends a full sphere. This works
/// with either winding and duplicated seams, unlike a bounding-box containment test.
fn mesh_encloses_light(mesh: &MeshData, source: glam::Vec3) -> bool {
    let source = source.as_dvec3();
    let angle: f64 = mesh.indices.chunks_exact(3).map(|t| {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize].as_dvec3() - source);
        2.0 * a.dot(b.cross(c)).atan2(
            a.length() * b.length() * c.length()
                + a.dot(b) * c.length() + b.dot(c) * a.length() + c.dot(a) * b.length(),
        )
    }).sum();
    angle.abs() > 2.0 * std::f64::consts::PI
}

/// The sprites and `[maplight]`s of a placed object.
fn object_lights(state: &mut TileState, ot: &ObjectType, pos: DVec3, xf: Mat4, lamp: Option<(i64, usize, bool)>, key: i64) {
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
    // (the same for every placement of the type: its meshes are walked once)
    let embedded = ot.embedded_lights.get_or_init(|| ot.sco.map_lights.iter().map(|ml| embedded_pole_light(ot, glam::Vec3::from(ml.pos))).collect());
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
            shadow_owner: embedded.get(k).copied().unwrap_or(false).then_some(key),
            ..Default::default()
        });
    }
}

/// Add what placing a tile counted to the load statistics.
fn note_place_stats(stats: &Mutex<LoadStats>, st: &StagedTile, res: &Resolved, placed: usize) {
    let mut s = stats.lock();
    s.failed_objects += st.counts.failed_objects;
    s.empty_spaces += st.counts.empty_spaces;
    s.rows += st.counts.rows;
    s.attached += st.counts.attached;
    s.unattached += res.unattached;
    s.objects_placed += placed;
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


/// The height of a `[crossing_heightdeformation]` field at (x, y) of its object's frame.
pub(super) fn field_height(m: &MeshData, x: f32, y: f32) -> Option<f32> {
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

#[cfg(test)]
mod embedded_light_tests {
    use super::*;

    fn add_cube(mesh: &mut MeshData, size: f32, reverse: bool) {
        let base = mesh.positions.len() as u32;
        mesh.positions.extend([
            glam::Vec3::new(-size, -size, -size), glam::Vec3::new(size, -size, -size),
            glam::Vec3::new(size, size, -size), glam::Vec3::new(-size, size, -size),
            glam::Vec3::new(-size, -size, size), glam::Vec3::new(size, -size, size),
            glam::Vec3::new(size, size, size), glam::Vec3::new(-size, size, size),
        ]);
        for mut t in [[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4],
            [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]] {
            if reverse { t.swap(1, 2); }
            mesh.indices.extend(t.map(|i| base + i));
        }
    }

    fn pole_fixture() -> ObjectType {
        let mut mesh = MeshData::default();
        add_cube(&mut mesh, 1.0, false);
        let emitter = omsi_model::LightEnh2 {
            pos: [0.0, 0.0, -0.5], dir: [0.0, 0.0, -1.0],
            color: [255.0; 3], size: 0.5, factor: 1.0,
            ..Default::default()
        };
        ObjectType {
            sco: SceneryObject {
                crash_mode_pole: Some((0.02, 0.7)),
                map_lights: vec![omsi_scenery::sco::MapLight {
                    pos: [0.0; 3], color: [0.4, 0.7, 0.9], radius: 1.5,
                }],
                ..Default::default()
            },
            model: Model {
                // A skipped definition before the loaded mesh exercises def-index mapping.
                meshes: vec![MeshDef::default(), MeshDef {
                    light_enh_2: vec![emitter], ..Default::default()
                }],
                ..Default::default()
            },
            meshes: vec![(mesh, Vec::new(), Vec::new())],
            mesh_def_index: vec![1],
            sound_path: Default::default(), model_dir: Default::default(),
            mesh_visible: vec![None], mesh_pivots: vec![Mat4::IDENTITY],
            mesh_shadow: vec![false], mesh_casts: vec![true], has_mouse_events: false, program: None,
            lower_lods: Vec::new(), lod0_min: 0.0, paint_scheme_count: 0,
            dynamic_textures: Vec::new(), holes: Vec::new(), deform: None,
            collision: None, paint: false, camera: Default::default(),
            collision_shape: Default::default(),
            embedded_lights: Default::default(),
        }
    }

    #[test]
    fn enclosing_geometry_is_not_a_bounding_box_test() {
        let mut mesh = MeshData::default();
        add_cube(&mut mesh, 2.0, false);
        add_cube(&mut mesh, 1.0, true); // A hollow enclosure: its room is air.
        assert!(!mesh_encloses_light(&mesh, glam::Vec3::ZERO));
        assert!(mesh_encloses_light(&mesh, glam::Vec3::new(1.5, 0.0, 0.0)));
        assert!(!mesh_encloses_light(&mesh, glam::Vec3::new(3.0, 0.0, 0.0)));
        for t in mesh.indices.chunks_exact_mut(3) { t.swap(1, 2); }
        assert!(mesh_encloses_light(&mesh, glam::Vec3::new(1.5, 0.0, 0.0)));
        assert!(!mesh_encloses_light(&mesh, glam::Vec3::ZERO));
    }

    #[test]
    fn only_enclosed_sources_behind_a_directional_pole_emitter_are_selected() {
        let mut fixture = pole_fixture();
        assert!(embedded_pole_light(&fixture, glam::Vec3::ZERO));
        for no_map_lighting in [false, true] {
            fixture.sco.no_map_lighting = no_map_lighting;
            assert!(embedded_pole_light(&fixture, glam::Vec3::ZERO),
                "nomaplighting is not a shadow exemption");
        }
        for source in [glam::Vec3::new(0.0, 0.0, -0.75), // Inside, in front of emitter.
            glam::Vec3::new(0.0, 0.0, -0.5), // Coincident with emitter.
            glam::Vec3::new(0.0, 0.0, 2.0)] { // Behind emitter, outside geometry.
            assert!(!embedded_pole_light(&fixture, source), "source {source:?}");
        }
        fixture.sco.crash_mode_pole = None;
        assert!(!embedded_pole_light(&fixture, glam::Vec3::ZERO));
        fixture = pole_fixture();
        fixture.model.meshes[1].light_enh_2[0].omni = true;
        assert!(!embedded_pole_light(&fixture, glam::Vec3::ZERO));
        fixture = pole_fixture();
        fixture.model.meshes[1].light_enh_2[0].dir = [0.0; 3];
        assert!(!embedded_pole_light(&fixture, glam::Vec3::ZERO));
        fixture = pole_fixture();
        fixture.model.meshes[1].light_enh_2.clear();
        assert!(!embedded_pole_light(&fixture, glam::Vec3::ZERO));
        fixture = pole_fixture();
        fixture.mesh_def_index[0] = 0;
        assert!(!embedded_pole_light(&fixture, glam::Vec3::ZERO),
            "an emitter on another mesh does not qualify this mesh");
        fixture = pole_fixture();
        fixture.meshes[0].0.indices.clear();
        assert!(!embedded_pole_light(&fixture, glam::Vec3::ZERO));
        fixture = pole_fixture();
        add_cube(&mut fixture.meshes[0].0, 0.25, true);
        assert!(!embedded_pole_light(&fixture, glam::Vec3::ZERO),
            "a source in enclosed air does not qualify");
    }

    #[test]
    fn placing_embedded_and_exterior_sources_preserves_light_parameters() {
        let mut fixture = pole_fixture();
        let mut outside = fixture.sco.map_lights[0].clone();
        outside.pos = [0.0, 0.0, 2.0];
        fixture.sco.map_lights.push(outside);
        let mut state = TileState::default();
        let position = DVec3::new(10.0, 20.0, 30.0);
        let transform = Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
        object_lights(&mut state, &fixture, position, transform, None, 123);
        assert_eq!(state.lights.len(), 2);
        assert_eq!(state.lights[0].shadow_owner, Some(123));
        assert_eq!(state.lights[1].shadow_owner, None);
        for (placed, source) in state.lights.iter().zip(&fixture.sco.map_lights) {
            assert_eq!(placed.position,
                position + transform.transform_point3(glam::Vec3::from(source.pos)).as_dvec3());
            assert_eq!(placed.core, source.radius);
            assert_eq!(placed.radius, source.radius * 6.0);
            assert_eq!(placed.color, source.color);
            assert_eq!(placed.intensity, 1.0);
        }
    }
}
