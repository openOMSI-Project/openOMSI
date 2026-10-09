//! Parked cars and props, the object editor, the texture budget and summaries.
use super::*;

impl World {
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
        let _ = self.parked_live.try_update(std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed, |n| Some(n.saturating_sub(state.parked_count)));
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
    pub(super) fn drop_scripted(&self, key: (i32, i32), audio: Option<&omsi_audio::AudioEngine>) {
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
    pub(super) fn waiting_cabin(&self, path: &Path) -> Option<Arc<omsi_vehicle::PassengerCabin>> {
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
        if omsi_cfg::flags::OMSI_DEBUG_TEXTURES.is_set() {
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
            if omsi_cfg::flags::OMSI_PROFILE.is_set() {
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
        if (!shrunk.is_empty() || restoring > 0) && omsi_cfg::flags::OMSI_PROFILE.is_set() {
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
        if omsi_cfg::flags::OMSI_PROFILE.is_set() {
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
pub(super) fn show_edit(renderer: &Renderer, scene: &mut Scene, eo: &EditObject, e: ObjectEdit) {
    // (a deleted one goes deep under the ground rather than being hidden: its instances'
    // visibility is the level-of-detail switch's, and an undone delete brings it back)
    let rot = Mat4::from_rotation_z(-(e.turned.to_radians() as f32)) * eo.xf;
    let at = eo.pos + e.moved - if e.deleted { DVec3::Z * 10_000.0 } else { DVec3::ZERO };
    for inst in &eo.instances {
        renderer.set_transform(scene, *inst, at, rot);
    }
}

/// Stop drawing an instance, keeping its other parameters.
pub(super) fn hide_instance(renderer: &Renderer, scene: &mut Scene, inst: usize) {
    let Some(i) = scene.instances.get(inst) else { return };
    let (alpha, uv) = (i.slot_alpha.clone(), i.slot_uv.clone());
    renderer.set_params(scene, inst, &alpha, false, &uv);
}

pub(super) fn fallen_pole(xf: Mat4, push: DVec3) -> Mat4 {
    let dir = glam::Vec3::new(push.x as f32, push.y as f32, 0.0).normalize_or(glam::Vec3::Y);
    let axis = glam::Vec3::Z.cross(dir).normalize_or(glam::Vec3::X);
    Mat4::from_axis_angle(axis, 86f32.to_radians()) * xf
}

/// How many tiles OMSI keeps loaded around the camera's own: its `[performance_tiledistmax]`,
/// 1 in the shipped options.cfg and in the presets maps ask for (Chicago Downtown's manual:
/// "Set neighbor tiles count to 1 or max. 2").
pub(super) const OMSI_TILE_DIST: i32 = 1;

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
/// So is a large mesh drawn only as a backdrop, every material `[matl_noZwrite]` or
/// `[matl_noZcheck]`: HafenCity's `3_BG_niederbaum` is a 1.6 km strip of the far bank of
/// the Elbe that starts at its placement by the Niederbaumbruecke, and it stood across
/// the road at the Landungsbruecken.
pub(super) fn stand_in_area(ot: &ObjectType, xf: &Mat4, pos: DVec3, tile: (i32, i32)) -> Option<[f64; 4]> {
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
        || ot.meshes.iter().any(|(m, _, defs)| stand_in_mesh(m, defs, xf, pos, loaded));
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

pub(super) fn stand_in_mesh(m: &MeshData, defs: &[MaterialDef], xf: &Mat4, pos: DVec3, loaded: f64) -> bool {
    let b = mesh_bounds(m, xf, pos);
    let width = (b[2] - b[0]).max(b[3] - b[1]);
    if width > 2.0 * loaded {
        return true;
    }
    if width <= loaded || m.positions.is_empty() {
        return false;
    }
    // a picture that never hides what is drawn after it, wherever it starts: a backdrop
    if !defs.is_empty() && defs.iter().all(|d| d.no_z_write || d.no_z_check) {
        return true;
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
pub(super) fn mesh_bounds(m: &MeshData, xf: &Mat4, origin: DVec3) -> [f64; 4] {
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
