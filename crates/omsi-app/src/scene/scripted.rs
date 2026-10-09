//! Signals, switches, particles, HTML objects and scripted scenery objects.
use super::*;

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
                if omsi_cfg::flags::OMSI_DEBUG_SIGNALS.is_set() {
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

    /// Finds the closest scenery object mesh carrying a `[mouseevent]` under a ray.
    pub fn scenery_object_hit(&self, origin: DVec3, dir: glam::Vec3, reach: f32, spread: f32) -> Option<SceneryHit> {
        let scripted = self.scripted.lock();
        let mut best: Option<SceneryHit> = None;
        let right = glam::Vec3::new(-dir.y, dir.x, 0.0).normalize_or_zero();
        let up = dir.cross(right).normalize_or_zero();
        let dirs = if spread > 0.0 {
            vec![
                dir,
                (dir + right * spread).normalize(),
                (dir - right * spread).normalize(),
                (dir + up * spread).normalize(),
                (dir - up * spread).normalize(),
            ]
        } else {
            vec![dir]
        };
        for o in scripted.iter().filter(|o| o.ty.has_mouse_events) {
            if (o.pos - origin).length() > reach as f64 + 60.0 {
                continue;
            }
            let local = (origin - o.pos).as_vec3();
            for mi in 0..o.ty.meshes.len() {
                let Some((data, _, _)) = o.ty.meshes.get(mi) else { continue };
                if !o.inst.mesh_visible.get(mi).copied().unwrap_or(true) {
                    continue;
                }
                let Some(&def_idx) = o.ty.mesh_def_index.get(mi) else { continue };
                let Some(event) = o.ty.model.meshes.get(def_idx).and_then(|m| m.mouse_event.as_ref()) else { continue };
                let xf = o.xf * o.inst.mesh_transforms.get(mi).copied().unwrap_or(Mat4::IDENTITY);
                for d in &dirs {
                    if let Some(t) = omsi_geometry::ray_mesh(local, *d, data, &xf) {
                        if t <= reach && best.as_ref().map_or(true, |b| t < b.t) {
                            best = Some(SceneryHit {
                                map_id: o.map_id,
                                mesh_index: mi,
                                event: event.clone(),
                                t,
                            });
                            break;
                        }
                    }
                }
            }
        }
        best
    }

    /// Click down on a scenery object's `[mouseevent]` switch or button.
    pub fn scenery_object_click(&self, map_id: i64, event: &str) -> bool {
        let mut scripted = self.scripted.lock();
        let Some(o) = scripted.iter_mut().find(|o| o.map_id == map_id) else {
            return false;
        };
        log::info!("scenery mouse event {event} on object {map_id}");
        let ok = o.inst.trigger(event);
        let drag = format!("{event}_drag");
        let low = event.to_ascii_lowercase();
        if (low.contains("taste") || low.contains("button") || low.contains("click"))
            && o.inst.program.trigger(&drag).is_some()
        {
            o.inst.host.mouse = (0.0, 0.0);
            o.inst.trigger(&drag);
            o.inst.host.mouse = (0.0, 0.0);
        }
        ok
    }

    /// Mouse dragged while holding down a scenery object switch.
    pub fn scenery_object_drag(&self, map_id: i64, event: &str, dx: f32, dy: f32) -> bool {
        let mut scripted = self.scripted.lock();
        let Some(o) = scripted.iter_mut().find(|o| o.map_id == map_id) else {
            return false;
        };
        let drag = format!("{event}_drag");
        o.inst.host.mouse = (dx, dy);
        let ok = o.inst.trigger(&drag);
        o.inst.host.mouse = (0.0, 0.0);
        ok
    }

    /// Mouse button released from a scenery object switch (`<event>_off`).
    pub fn scenery_object_release(&self, map_id: i64, event: &str) -> bool {
        let mut scripted = self.scripted.lock();
        let Some(o) = scripted.iter_mut().find(|o| o.map_id == map_id) else {
            return false;
        };
        let off = format!("{event}_off");
        o.inst.trigger(&off)
    }

    /// Mouse wheel notch over a scenery object switch.
    pub fn scenery_object_wheel(&self, map_id: i64, event: &str, amount: f32) -> bool {
        let mut scripted = self.scripted.lock();
        let Some(o) = scripted.iter_mut().find(|o| o.map_id == map_id) else {
            return false;
        };
        o.inst.host.mouse = (0.0, amount);
        let _ = o.inst.trigger(&format!("{event}_drag"));
        o.inst.host.mouse = (0.0, 0.0);
        o.inst.trigger(&format!("{event}_off"))
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
        if omsi_cfg::flags::OMSI_NO_LIGHT_MAP.is_set() {
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
                if o.inst.set_var("Switch", *d as f32) && was != Some(*d as f32) && omsi_cfg::flags::OMSI_DEBUG_SWITCHES.is_set() {
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
        // (an environment switch read once per process)
        static DEBUG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *DEBUG.get_or_init(|| omsi_cfg::flags::OMSI_DEBUG_PARTICLES.is_set()) {
            let mut near: Vec<(f64, &ParticleObject)> = objs.values().flatten().map(|po| ((po.pos - center).length(), po)).collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (d, po) in near.iter().take(3) {
                log::info!("particle object {} at ({:.1}, {:.1}, {:.1}), {d:.0} m: {} particles", po.map_id, po.pos.x, po.pos.y, po.pos.z, po.set.particles().count());
            }
        }
        // The scripted objects whose variables the emitters read, found in one pass over the
        // list - the first of each id, as a search would find it: a search of the whole list
        // for every emitter's object, every frame, was most of the scenery scripts' time on a
        // city map.
        let mut owners: HashMap<i64, Option<usize>> = HashMap::new();
        for po in objs.values().flatten() {
            if !((po.pos - center).length() > 1500.0) {
                owners.insert(po.map_id, None);
            }
        }
        if !owners.is_empty() {
            for (i, s) in scripted.iter().enumerate() {
                if let Some(slot) = owners.get_mut(&s.map_id) {
                    slot.get_or_insert(i);
                }
            }
        }
        for list in objs.values_mut() {
            for po in list.iter_mut() {
                if (po.pos - center).length() > 1500.0 {
                    continue;
                }
                let inst = owners.get(&po.map_id).copied().flatten().map(|i| &scripted[i].inst);
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
                    // (read in place: a copy each frame was only compared with the last)
                    let text = o.inst.str_var(st.def.variable.trim());
                    if st.update(text) {
                        let (w, h) = (st.def.width.max(1) as u32, st.def.height.max(1) as u32);
                        if let Some(rgba) = st.pending.take() {
                            // OMSI_DUMP_SCENERY_TEXT=<dir>: the pictures as drawn
                            if let Some(dir) = omsi_cfg::flags::OMSI_DUMP_SCENERY_TEXT.os() {
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
    pub(super) fn object_sounds(
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
