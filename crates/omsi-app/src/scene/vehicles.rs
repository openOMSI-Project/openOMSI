//! The vehicle upload: a vehicle type's meshes and materials made into render instances.
use super::*;

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
            for (_, mesh, _) in &render.damaged {
                renderer.free_mesh(scene, *mesh);
            }
            for (_, instance, mesh) in &render.glass_cracks {
                renderer.remove_instance(scene, *instance);
                let slots = renderer.instance_slots(scene, *instance);
                gpu.free_instances.entry(slots).or_default().push(*instance);
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
            if let Some(wipers) = &render.window_wipers {
                own_textures.extend(wipers.textures());
                own_materials.extend(wipers.materials());
            }
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
        if omsi_cfg::flags::OMSI_PROFILE.is_set() {
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
    pub(super) fn vehicle_texture(
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
            if omsi_cfg::flags::OMSI_DEBUG_TEXTURES.is_set() {
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
        if omsi_cfg::flags::OMSI_DEBUG_TEXTURES.is_set() {
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
    pub(super) fn snow_glass_texture(
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
    pub(super) fn vehicle_bump_texture(
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
        if omsi_cfg::flags::OMSI_DEBUG_TEXTURES.is_set() {
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
    pub(super) fn vehicle_mesh(
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
    pub(super) fn recycle_pair(
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
    pub(super) fn instantiate_vehicle(
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
                // (a page drawn as an LED panel carries its dot mask as a plain transmap)
                extra.led = (d.script_trans.is_some() || (d.script.is_some() && d.transmap.is_some())) && d.extra.led;
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
        if ordered && !omsi_cfg::flags::OMSI_NO_MODEL_ORDER.is_set() {
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
            window_wipers: None,
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
            damaged: Vec::new(),
            glass_cracks: Vec::new(),
            hidden: false,
            interior_lamps: std::cell::Cell::new(None),
            interior_blocks: std::cell::OnceCell::new(),
        }
    }

    /// Upload the meshes and materials of a vehicle type: (mesh, materials) per model mesh
    /// and one texture per `[texttexture]`.
    /// Also returns the slots whose textures are generated per vehicle. `player`: the
    /// player's own vehicle, whose mirrors' glass is noted (for the panels).
    pub(super) fn upload_vehicle(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        vt: &omsi_sim::VehicleType,
        scheme: Option<usize>,
        player: bool,
    ) -> VehicleSet {
        let mut dirs = vt.texture_dirs(&self.root);
        let (subst, scheme_dir) = match scheme {
            Some(i) => vt.scheme_substitutions(i),
            None => (vt.default_substitutions(&self.root), None),
        };
        if let Some(d) = scheme_dir {
            dirs.insert(0, d);
        }
        let dirs_ref: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
        let tex_ids = self.vehicle_textures.lock();
        let mut mesh_ids = self.vehicle_meshes.lock();
        let mut mesh_keys: Vec<(PathBuf, usize)> = Vec::new();
        let t_all = std::time::Instant::now();
        let mut mesh_secs = 0.0f64;
        // OMSI_ONLY_MESH=a|b draws only the meshes whose file names contain one of the
        // parts (and logs their materials); OMSI_HIDE_MESH=a|b leaves those out
        let only = omsi_cfg::flags::OMSI_ONLY_MESH.var().map(str::to_string);
        let hide = omsi_cfg::flags::OMSI_HIDE_MESH.var().map(str::to_string);
        let matches = |list: &str, file: &str| {
            list.split('|').any(|f| {
                !f.is_empty() && file.to_ascii_lowercase().contains(&f.to_ascii_lowercase())
            })
        };
        let mut up = VehicleUpload {
            vt,
            player,
            subst: &subst,
            dirs: &dirs,
            dirs_ref: &dirs_ref,
            tex_ids,
            // what the set holds, given back when it is trimmed
            held: Vec::new(),
            materials: Vec::new(),
            tex_time: std::cell::RefCell::new((0usize, 0.0f64)),
            instances: Vec::new(),
            missing_tex: Vec::new(),
            only: only.clone(),
            variants: Vec::new(),
            dyn_slots: Vec::new(),
        };
        for (mesh_index, vm) in vt.meshes.iter().enumerate() {
            let def = &vt.model.meshes[vm.def_index];
            if only.as_deref().is_some_and(|f| !matches(f, &def.file))
                || hide.as_deref().is_some_and(|f| matches(f, &def.file))
            {
                // keep instance numbering stable: an empty placeholder mesh
                let id = renderer.add_mesh(scene, &MeshData::default());
                up.instances.push((id, vec![]));
                continue;
            }
            let mats: Vec<MaterialId> = vm
                .materials
                .iter()
                .enumerate()
                .map(|(slot, m)| self.vehicle_slot_material(renderer, scene, &mut up, (mesh_index, vm), slot, m))
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
            up.instances.push((id, mats));
        }
        let VehicleUpload { held, mut materials, tex_time, instances, mut missing_tex, variants, dyn_slots, .. } = up;
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
        if omsi_cfg::flags::OMSI_PROFILE.is_set() {
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

    /// The material of one slot of a vehicle mesh being uploaded (see
    /// [`World::upload_vehicle`]); its `[matl_item]`s, `[texchanges]`, `[matl_freetex]` and
    /// generated textures are noted in `up`.
    fn vehicle_slot_material(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        up: &mut VehicleUpload,
        (mesh_index, vm): (usize, &omsi_sim::vehicle::VehicleMesh),
        slot: usize,
        m: &omsi_o3d::Material,
    ) -> MaterialId {
        let VehicleUpload { vt, player, subst, dirs, dirs_ref, .. } = *up;
        let VehicleUpload { tex_ids, held, materials, tex_time, instances, missing_tex, only, variants, dyn_slots, .. } = up;
        let def = &vt.model.meshes[vm.def_index];
        // [CTCTexture] slots swapped by the paint scheme
        let subst = |name: &str| -> String {
            subst
                .get(&name.to_ascii_lowercase())
                .cloned()
                .unwrap_or_else(|| name.to_string())
        };
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
                let r = self.$how(renderer, scene, tex_ids, held, $name, $dirs);
                if tex_ids.len() > n {
                    let mut tt = tex_time.borrow_mut();
                    tt.0 += 1;
                    tt.1 += t.elapsed().as_secs_f64();
                }
                r
            }};
        }
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
        } else if rain_layer && snowing() && !seasonal_texture(&tex_name, dirs_ref) {
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
        // `\S:n` = script texture n as transparency map
        let script_trans = ov.iter().find_map(|o| o.transmap.clone()).and_then(|t| t.trim().strip_prefix("\\S:").and_then(|n| n.trim().parse::<usize>().ok()));
        let transmap = ov.iter().find_map(|o| o.transmap.clone()).filter(|t| !t.trim().is_empty() && !t.trim().starts_with("\\S:")).map(|t| subst(&t)).and_then(|t| {
            let id = tex!(&t, &dirs_ref)?;
            let has_alpha = self.textures.has_alpha(&t, dirs_ref).unwrap_or(false);
            Some((id, has_alpha))
        });
        let cx = SlotCx { vt, mesh_index, vm, def, slot, m, dirs_ref, subst: &subst };
        let SlotAlpha { alpha, declared_alpha, dirt_overlay, cover, see_through, transparent_layer_hint, named_body, repair_body_depth } =
            slot_alpha(&cx, &ov, tex, transmap);
        // (a night or light map named as a [CTCTexture] is the paint scheme's
        // picture as well, like the diffuse texture and the transparency map:
        // looked up by the model's own name, a destination display lit by its own
        // texture glowed with the model's default text over the repaint's, #895)
        let night = ov.iter().find_map(|o| o.nightmap.clone()).and_then(|t| {
            tex!(&subst(&t), &dirs_ref)
        });
        // ([matl_glow]: the material is its own light - its mask rides in the light map's
        // slot, see `MaterialExtra::glow`)
        let lightmap = ov.iter().find_map(|o| o.lightmap.clone()).and_then(|(t, _)| {
            tex!(&subst(&t), &dirs_ref)
        }).or_else(|| glow_mask(&ov).and_then(|t| tex!(&subst(t), &dirs_ref)));
        // (a `\S:n` panel lit all over by its light map is an LED panel; one
        // whose light map is a picture is a flipdot: see `is_white_lightmap`)
        let lm_white = |ov: &[&MaterialDef]| -> bool {
            ov.iter().find_map(|o| o.lightmap.as_ref()).and_then(|(t, _)| lightmap_is_white(&subst(t), dirs_ref)).unwrap_or(true)
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
        let envmap = ov.iter().find_map(|o| o.envmap.clone()).filter(|_| !omsi_cfg::flags::OMSI_NO_ENVMAP.is_set()).and_then(|(t, f)| {
            let id = tex!(&t, &dirs_ref)?;
            Some((id, f))
        });
        let env_mask = ov.iter().find_map(|o| o.envmap_mask.clone()).filter(|t| envmap.is_some() && !t.trim().is_empty()).and_then(|t| tex!(&subst(&t), &dirs_ref));
        let bump = ov.iter().find_map(|o| o.bumpmap.clone()).filter(|_| envmap.is_some() && !omsi_cfg::flags::OMSI_NO_BUMP.is_set()).and_then(|(t, f)| tex!(&subst(&t), &dirs_ref, vehicle_bump_texture).map(|id| (id, f)));
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
        let (color, emissive, extra) = slot_extra(
            &cx,
            &ov,
            &SlotAlpha { alpha, declared_alpha, dirt_overlay, cover, see_through, transparent_layer_hint, named_body, repair_body_depth },
            textured,
            (night, envmap, env_mask, bump),
            (script_slot, script_trans, rain_layer),
            transmap.is_some(),
            &lm_white,
        );
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
            let it_light = ov_item.iter().find_map(|o| o.lightmap.clone()).and_then(|(t, _)| find_tex(&subst(&t))).or_else(|| glow_mask(ov_item).and_then(|t| find_tex(&subst(t)))).or(lightmap);
            // the item's own transparency map, else the plain material's
            let it_script_trans = match ov_item.iter().find_map(|o| o.transmap.clone()) {
                Some(t) => t.trim().strip_prefix("\\S:").and_then(|n| n.trim().parse::<usize>().ok()),
                None => script_trans,
            };
            let it_trans = ov_item.iter().find_map(|o| o.transmap.clone()).filter(|t| !t.trim().is_empty() && !t.trim().starts_with("\\S:")).and_then(|t| {
                let id = find_tex(&subst(&t))?;
                let has_alpha = self.textures.has_alpha(&t, dirs_ref).unwrap_or(false);
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
            // (an item with no light map or glow of its own keeps its base's)
            it_extra.glow = if own_light_slot(ov_item) { it_extra.glow } else { extra.glow };
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
            dirs: dirs.to_vec(),
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
                .filter_map(|(t, v)| omsi_texture::find_texture(&subst(t), dirs_ref).map(|p| (p, v.clone())))
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
    }
}


/// One material slot of a vehicle mesh being uploaded (see [`World::vehicle_slot_material`]).
#[derive(Clone, Copy)]
struct SlotCx<'s> {
    vt: &'s omsi_sim::VehicleType,
    mesh_index: usize,
    vm: &'s omsi_sim::vehicle::VehicleMesh,
    def: &'s MeshDef,
    slot: usize,
    m: &'s omsi_o3d::Material,
    dirs_ref: &'s [&'s Path],
    /// The paint scheme's `[CTCTexture]` substitution.
    subst: &'s dyn Fn(&str) -> String,
}

/// How a vehicle's material slot is blended and whether it writes depth.
#[derive(Clone, Copy)]
struct SlotAlpha {
    alpha: AlphaMode,
    declared_alpha: AlphaMode,
    dirt_overlay: bool,
    cover: bool,
    see_through: bool,
    transparent_layer_hint: bool,
    named_body: bool,
    repair_body_depth: bool,
}

/// The alpha mode of a vehicle's material slot: what the model.cfg declares, mended for
/// dirt films, shadow decals, panes and (on request) bodies.
fn slot_alpha(cx: &SlotCx, ov: &[&MaterialDef], tex: Option<TextureId>, transmap: Option<(TextureId, bool)>) -> SlotAlpha {
    let SlotCx { vt, mesh_index, vm, def, slot, m, dirs_ref, subst } = *cx;
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
    let coverage = omsi_texture::find_texture(&coverage_tex, dirs_ref).and_then(|p| alpha_mask(&p));
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
    let repair_body_depth = omsi_cfg::flags::OMSI_REPAIR_BODY_DEPTH.is_set() && !layer && is_vehicle_body_material(&def.file, &m.texture, tex.is_some(), transmap.is_some(), ov.iter().any(|o| o.no_z_write), body_hint);
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
    if omsi_cfg::flags::OMSI_FORCE_OPAQUE.is_set() && !dirt_overlay {
        alpha = AlphaMode::Opaque;
    }
    SlotAlpha { alpha, declared_alpha, dirt_overlay, cover, see_through, transparent_layer_hint, named_body, repair_body_depth }
}

/// The colours and the shader flags (`MaterialExtra`) of a vehicle's material slot.
#[allow(clippy::type_complexity)]
fn slot_extra(
    cx: &SlotCx,
    ov: &[&MaterialDef],
    a: &SlotAlpha,
    textured: bool,
    (night, envmap, env_mask, bump): (Option<TextureId>, Option<(TextureId, f32)>, Option<TextureId>, Option<(TextureId, f32)>),
    (script_slot, script_trans, rain_layer): (Option<usize>, Option<usize>, bool),
    has_transmap: bool,
    lm_white: &dyn Fn(&[&MaterialDef]) -> bool,
) -> ([f32; 4], [f32; 3], MaterialExtra) {
    let SlotCx { vm, def, slot, m, .. } = *cx;
    let SlotAlpha { alpha, declared_alpha, dirt_overlay, cover, see_through, transparent_layer_hint, named_body, repair_body_depth } = *a;
    let (color, emissive, specular, ambient) = d3d_material(m, ov.iter().find_map(|o| o.allcolor), textured);
    let mut extra = material_extra(ov, env_mask, bump, specular);
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
    // ... and so is a page (`[useHtmlTexture]`) drawn as an LED panel: its picture
    // is the dots' colour and a dot mask its `[matl_transmap]` (a page is a
    // script texture of its own, so the Krueger's `\S:n` mask cannot be reused)
    let html_page = script_slot.is_some_and(|s| cx.vt.model.html_textures.iter().any(|h| h.script_index == s));
    extra.led = (script_trans.is_some() || (html_page && has_transmap)) && lm_white(ov);
    extra.led_sign = html_page;
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
    extra.rain_film = rain_layer && !snowing() && !omsi_cfg::flags::OMSI_TEXTURE_RAIN.is_set();
    // Some mod buses put [matl_noZcheck] on the complete body mesh.
    // That flag is for decals; on a body it disables depth writing and
    // lets the cabin bleed through the outside shell. Keep it on genuine
    // overlays, but make a repaired body a normal depth-writing surface.
    if repair_body_depth {
        extra.no_z_check = false;
    }
    (color, emissive, extra)
}
/// What uploading a vehicle set gathers, slot by slot (see [`World::upload_vehicle`]).
struct VehicleUpload<'a> {
    vt: &'a omsi_sim::VehicleType,
    player: bool,
    /// The paint scheme's `[CTCTexture]` substitutions.
    subst: &'a HashMap<String, String>,
    dirs: &'a [PathBuf],
    dirs_ref: &'a [&'a Path],
    tex_ids: parking_lot::MutexGuard<'a, HashMap<PathBuf, (TextureId, usize)>>,
    held: Vec<PathBuf>,
    materials: Vec<MaterialId>,
    tex_time: std::cell::RefCell<(usize, f64)>,
    instances: Vec<(MeshId, Vec<MaterialId>)>,
    missing_tex: Vec<String>,
    only: Option<String>,
    variants: Vec<VariantSlot>,
    dyn_slots: Vec<DynSlot>,
}
