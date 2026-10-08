//! What the traffic shows: the GPU side of the AI vehicles, their drivers and the
//! traffic lamps.
//!
//! The renders live in `TrafficView` (`SimView::traffic`, apart from the `Traffic`): the
//! traffic's own steps that put a car on the road or take one off (population, timetable
//! departures, trains, the LAN mirror) take it as a parameter and make or let go the car's
//! renders at once, through `TrafficView` and `Traffic::new_car_render`; everything
//! else - parked cars, lamps, drivers, the cars' transforms, materials and script textures
//! - is brought up to date by the view sync (`view_sync::sync`, `Traffic::sync`).

use crate::scene::{VehicleRender, World};
use crate::traffic::Traffic;
use hashbrown::HashMap;
use omsi_sim::traffic::TrafficLightController;
use omsi_sim::{VehicleInstance, VehicleType};
use std::sync::Arc;
use omsi_render::{Renderer, Scene};

pub(super) const SCRIPT_UPLOAD_BUDGET: usize = 4 << 20;

/// Within this distance (m) of the camera a timetable bus has its driver at the wheel.
pub(super) const DRIVER_NEAR: f64 = 70.0;

/// How near an articulated AI bus has to be for its bellows to be reshaped as its joint
/// turns (m): the fold of the bend is a few centimetres, which is a screen pixel or more
/// within this range - farther out it is not worth a mesh update every frame it steers.
pub(super) const SKIN_DISTANCE: f64 = 200.0;

/// What an AI car looks like on the screen: the render of its body and those of its
/// coupled parts (trailers, rear sections, the cars of a train), in their order.
pub(crate) struct CarRender {
    pub(crate) body: VehicleRender,
    pub(crate) trailers: Vec<VehicleRender>,
}

/// The GPU side of the traffic, kept apart from the simulation: the cars' renders by car
/// id, and the drivers of the timetable buses.
#[derive(Default)]
pub(crate) struct TrafficView {
    cars: HashMap<u64, CarRender>,
    /// Renders of cars that have gone, given back at the next `sync`.
    released: Vec<VehicleRender>,
    /// The drivers at the wheel of the timetable buses near the camera, by car id (see
    /// `driver.rs`; made within `DRIVER_NEAR` m of the camera, let go beyond twice that).
    drivers: HashMap<u64, crate::driver::DriverFigure>,
    /// Figures let go by their bus, hidden, for the next one (their GPU meshes stay).
    driver_pool: Vec<crate::driver::DriverFigure>,
}

impl TrafficView {
    /// The renders of the car `id` that has just been put on the road.
    pub(crate) fn insert(&mut self, id: u64, render: CarRender) {
        if let Some(old) = self.cars.insert(id, render) {
            log::warn!("traffic: two cars with id {id}; the renders of the first are let go");
            self.released.push(old.body);
            self.released.extend(old.trailers);
        }
    }

    /// The renders of car `id`.
    pub(crate) fn car(&self, id: u64) -> Option<&CarRender> {
        self.cars.get(&id)
    }

    /// Take the renders of car `id` out (it is made anew under the same id).
    pub(crate) fn take(&mut self, id: u64) -> Option<CarRender> {
        self.cars.remove(&id)
    }

    /// Car `id` has gone: its renders go back to the world at the next sync.
    pub(crate) fn retire(&mut self, id: u64) {
        if let Some(r) = self.cars.remove(&id) {
            self.released.push(r.body);
            self.released.extend(r.trailers);
        }
    }

    /// Car `id` has gone: its renders go back to the world now.
    pub(crate) fn release_car(&mut self, world: &World, renderer: &Renderer, scene: &mut Scene, id: u64) {
        if let Some(r) = self.cars.remove(&id) {
            release_car_render(world, renderer, scene, r);
        }
    }

    /// Couple another part of type `t` to car `id`'s picture.
    pub(crate) fn add_trailer(&mut self, world: &World, renderer: &Renderer, scene: &mut Scene, id: u64, t: &Arc<VehicleType>) {
        if let Some(r) = self.cars.get_mut(&id) {
            r.trailers
                .push(world.add_vehicle_shared(renderer, scene, t, None, Some(&r.body)));
        }
    }

    /// Let the renders of car `id`'s coupled parts go now (it gets others).
    pub(crate) fn release_trailers(&mut self, world: &World, renderer: &Renderer, scene: &mut Scene, id: u64) {
        if let Some(r) = self.cars.get_mut(&id) {
            for t in r.trailers.drain(..) {
                world.release_vehicle(renderer, scene, t);
            }
        }
    }
}

/// Give the renders of a car back to the world, its body first.
pub(crate) fn release_car_render(world: &World, renderer: &Renderer, scene: &mut Scene, r: CarRender) {
    for r in std::iter::once(r.body).chain(r.trailers) {
        world.release_vehicle(renderer, scene, r);
    }
}

/// A parked car drives off: its object goes from the scene (None when it is not there).
pub(crate) fn depart_parked(world: &World, renderer: &Renderer, scene: &mut Scene, key: i64) -> Option<()> {
    world.depart_parked(renderer, scene, key).map(|_| ())
}

impl Traffic {
    /// Load and attach the `[couple_back]` chain of `vehicle`; returns the renders.
    pub(crate) fn attach_trailers(
        &mut self,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        vehicle: &mut VehicleInstance,
        scheme: Option<usize>,
        lead: &VehicleRender,
    ) -> Vec<VehicleRender> {
        let mut renders = Vec::new();
        let ty = vehicle.ty.clone();
        for (t, rev) in self.trailer_chain(&ty) {
            renders.push(world.add_vehicle_shared(
                renderer,
                scene,
                &t,
                scheme.filter(|i| *i < t.paint_schemes.len()),
                Some(lead),
            ));
            vehicle.attach_trailer_ex(t, rev);
        }
        renders
    }

    /// The renders of a new car of type `ty` (paint `scheme`) and of the parts its
    /// `[couple_back]` chain couples to `vehicle`.
    pub(crate) fn new_car_render(
        &mut self,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        vehicle: &mut VehicleInstance,
        ty: &Arc<VehicleType>,
        scheme: Option<usize>,
    ) -> CarRender {
        let body = world.add_vehicle_shared(renderer, scene, ty, scheme, None);
        let trailers = self.attach_trailers(world, renderer, scene, vehicle, scheme, &body);
        CarRender { body, trailers }
    }

    pub fn precache_random(&mut self, world: &World, renderer: &Renderer, scene: &mut Scene) {
        let t0 = std::time::Instant::now();
        let sets = self.random_sets();
        for chunk in sets.chunks(3) {
            world.prefetch_vehicle_sets(renderer, chunk);
            for (ty, scheme) in chunk {
                world.precache_vehicle(renderer, scene, ty, *scheme);
            }
        }
        world.forget_prefetched();
        for (ty, _) in &sets {
            self.prime_pull_out_room(ty, false);
        }
        log::info!("traffic: {} vehicle/paint sets of the random traffic read and uploaded in {:.1} s", sets.len(), t0.elapsed().as_secs_f32());
    }

    /// The drivers of the timetable buses near the camera: made when a bus comes within
    /// `DRIVER_NEAR`, posed every sync, let go when it is twice that far or gone.
    pub(super) fn sync_drivers(&mut self, view: &mut TrafficView, world: &World, renderer: &Renderer, scene: &mut Scene) {
        let Some(eye) = self.sim.viewer.map(|v| v.pos) else { return };
        let dt = self.sim.last_dt.max(1.0 / 120.0);
        let mut keep: Vec<u64> = Vec::new();
        for c in &self.sim.cars {
            if !c.is_bus() || c.gone {
                continue;
            }
            let d = (c.vehicle.position - eye).length();
            if d > DRIVER_NEAR * 2.0 {
                continue;
            }
            keep.push(c.id);
            if !view.drivers.contains_key(&c.id) {
                if d > DRIVER_NEAR {
                    continue;
                }
                let figure = match view.driver_pool.pop() {
                    Some(mut f) => {
                        if f.attach(&c.vehicle) {
                            Some(f)
                        } else {
                            view.driver_pool.push(f);
                            None
                        }
                    }
                    None => crate::driver::DriverFigure::new(world, renderer, scene, &c.vehicle, c.id),
                };
                match figure {
                    Some(f) => {
                        view.drivers.insert(c.id, f);
                    }
                    None => continue,
                }
            }
            if let (Some(f), Some(r)) = (view.drivers.get_mut(&c.id), view.cars.get(&c.id)) {
                f.update(renderer, scene, &c.vehicle, &r.body, dt, true, false);
            }
        }
        let gone: Vec<u64> = view.drivers.keys().copied().filter(|id| !keep.contains(id)).collect();
        for id in gone {
            if let Some(mut f) = view.drivers.remove(&id) {
                f.hide(renderer, scene);
                view.driver_pool.push(f);
            }
        }
    }

    pub(super) fn sync(&mut self, view: &mut TrafficView, world: &World, renderer: &Renderer, scene: &mut Scene) {
        // cars that have parked: the parked object stands in their place from now on
        let mut i = 0;
        while i < self.sim.cars.len() {
            match self.sim.cars[i].park {
                Some(p) if p.done => {
                    let c = self.sim.cars.swap_remove(i);
                    if world.return_parked(renderer, scene, p.key) {
                        if let Some(list) = self.sim.parked.get_mut(&p.lane) {
                            list.push((p.s, p.lat));
                        } else {
                            self.sim.parked.insert(p.lane, vec![(p.s, p.lat)]);
                        }
                    }
                    if omsi_cfg::flags::OMSI_DEBUG_TRAFFIC.is_set() {
                        log::info!("car {} has parked (space {})", c.id, p.key);
                    }
                    self.drop_sounds(c.id);
                    view.retire(c.id);
                }
                _ => i += 1,
            }
        }
        for r in std::mem::take(&mut view.released) {
            world.release_vehicle(renderer, scene, r);
        }
        self.sync_drivers(view, world, renderer, scene);
        self.sync_lamps(world, renderer, scene);
        self.sync_cars(view, world, renderer, scene);
    }

    /// The traffic light lamps (see `sync`).
    fn sync_lamps(&mut self, world: &World, renderer: &Renderer, scene: &mut Scene) {
        // traffic light lamps: the lamp's script (or the stock rules) turns the state of its
        // light into the `[visible] red|yellow|green 1` meshes and the coronas, and moves
        // what it animates (a barrier arm); it runs on the time since the last sync (an
        // offscreen run syncs only for its pictures)
        let dt = std::mem::take(&mut self.sim.lamp_dt);
        let near = self.sim.viewer.map(|v| v.pos);
        let debug_lamps = omsi_cfg::flags::OMSI_DEBUG_LAMPS.is_set();
        for lamp in world.light_objects.lock().iter_mut() {
            if debug_lamps && lamp.animated {
                log::info!("moving lamp at ({:.1}, {:.1}), {:.0} m from the viewer", lamp.pos.x, lamp.pos.y, near.map(|p| (lamp.pos - p).length()).unwrap_or(0.0));
            }
            if let Some(p) = near {
                if (lamp.pos - p).length() > 1200.0 {
                    continue;
                }
            }
            let (state, request) = match self
                .controller_of_object
                .get(&lamp.parent)
                .and_then(|&c| self.sim.lights.get(c))
            {
                Some(ctl) if lamp.any_light && ctl.lights.len() > 1 => {
                    // the most open of the crossing's lights (see `LightObject::any_light`)
                    let open = |s: i32| match s {
                        6..=8 => 3,
                        3..=5 => 2,
                        9..=11 => 1,
                        0..=2 => 0,
                        _ => -1,
                    };
                    let li = (0..ctl.lights.len())
                        .max_by_key(|&i| open(ctl.state(i)))
                        .unwrap_or(0);
                    (ctl.state(li), ctl.request.iter().any(|r| *r))
                }
                Some(ctl) => (
                    ctl.state(lamp.index),
                    ctl.request.get(lamp.index).copied().unwrap_or(false),
                ),
                // a lamp that names no crossing, or one whose crossing has no program,
                // reads the engine's dummy (see `UNLINKED_PHASE`): red, as in OMSI
                None => (omsi_sim::traffic::UNLINKED_PHASE, false),
            };
            let (r, y, g) = TrafficLightController::lamps(state);
            let value = |lamp: &crate::scene::LightObject, var: &str| -> f32 {
                // Custom signals can shift phases or blink the standard channels (Numazu
                // pedestrian lamps). Use their script outputs whenever they are available.
                let scripted = lamp.script.as_ref().and_then(|script| {
                    let s = script.lock();
                    // A failed/missing script can still have a varlist of zeroes. Keep
                    // stock fallback behaviour if it has no runnable frame block.
                    if s.program.frame.is_empty() { None } else { s.var(var) }
                });
                crate::scene::traffic_lamp_value(
                    var,
                    scripted,
                    crate::scene::standard_traffic_lamp(var, r, y, g, request),
                )
            };
            if let Some(script) = lamp.script.as_ref() {
                let vars = omsi_sim::scenery::SceneryVars {
                    nightlight: self.sim.night as i32 as f32,
                    in_use: 1.0,
                    traffic_light_phase: state as f32,
                    traffic_light_approach: request as i32 as f32,
                    switch: None,
                };
                let mut s = script.lock();
                s.update(dt, &vars);
                // `OMSI_DEBUG_LAMPS`: where the moving lamps are (barriers) and how far their
                // meshes are turned, each time the lamps are updated
                if lamp.animated && debug_lamps {
                    let turn = s
                        .mesh_transforms
                        .iter()
                        .map(|m| {
                            let (_, r, _) = m.to_scale_rotation_translation();
                            r.to_axis_angle().1.to_degrees()
                        })
                        .fold(0.0f32, f32::max);
                    log::info!("lamp at ({:.1}, {:.1}): light state {:?} (crossing {:?}, light {}{}), meshes turned up to {turn:.0} deg", lamp.pos.x, lamp.pos.y, vars.traffic_light_phase, self.sim.controller_of_object.get(&lamp.parent), lamp.index, if lamp.any_light { ", any" } else { "" });
                }
                if lamp.animated {
                    for (i, (inst, _)) in lamp.instances.iter().enumerate() {
                        if let Some(m) = s.mesh_transforms.get(i) {
                            renderer.set_transform(scene, *inst, lamp.pos, lamp.xf * *m);
                        }
                    }
                    // the lights go with their meshes (a barrier's lamps rise with its arm)
                    for (c, (mi, local, dir)) in lamp.coronas.iter_mut().zip(&lamp.corona_mesh) {
                        if let Some(m) = s.mesh_transforms.get(*mi) {
                            let xf = lamp.xf * *m;
                            c.0.position = lamp.pos + xf.transform_point3(*local).as_dvec3();
                            if *dir != glam::Vec3::ZERO {
                                c.0.direction = xf.transform_vector3(*dir).normalize_or_zero();
                            }
                        }
                    }
                }
            }
            if !lamp.texts.is_empty() {
                if let Some(script) = lamp.script.as_ref() {
                    let mut s = script.lock();
                    let _ = s.take_refresh_strings();
                    for (tex, st) in lamp.texts.iter_mut() {
                        let text = s.str_var(st.def.variable.trim()).to_string();
                        if st.update(&text) {
                            if let Some(rgba) = st.pending.take() {
                                let (w, h) = (st.def.width.max(1) as u32, st.def.height.max(1) as u32);
                                renderer.update_texture_mips(
                                    scene,
                                    *tex,
                                    &omsi_texture::Image { width: w, height: h, rgba, has_alpha: true },
                                );
                            }
                        }
                    }
                }
            }
            if !lamp.animated {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (state, request).hash(&mut h);
                if let Some(script) = lamp.script.as_ref() {
                    for v in &script.lock().state.vars {
                        v.to_bits().hash(&mut h);
                    }
                }
                let sig = h.finish();
                if lamp.shown == Some(sig) {
                    continue;
                }
                lamp.shown = Some(sig);
            }
            // Traffic lamps do not enter World's ordinary scripted-object update path.
            // Switch their materials here too, so [matl_item] nightmaps light the LEDs.
            for (inst, slot, base, item, var) in &lamp.variants {
                renderer.set_material(
                    scene,
                    *inst,
                    *slot,
                    if crate::scene::change_picks_item(value(lamp, var)) { *item } else { *base },
                );
            }
            for k in 0..lamp.coronas.len() {
                let v = value(lamp, &lamp.coronas[k].1);
                lamp.lit[k] = v;
            }
            for (k, (inst, cond)) in lamp.instances.iter().enumerate() {
                let visible = match cond {
                    Some((var, want)) => (value(lamp, var) - want).abs() < 0.5,
                    None => true,
                };
                // lenses switched by their material instead (`[alphascale]` and
                // `[matl_lightmap]` on the lamp's variables, #826)
                match lamp.slots.get(k).filter(|s| !s.is_empty()) {
                    Some(slots) => {
                        let known = |v: &str| -> Option<f32> {
                            let scripted = lamp.script.as_ref().and_then(|script| {
                                let s = script.lock();
                                if s.program.frame.is_empty() { None } else { s.var(v) }
                            });
                            scripted
                                .or_else(|| v.trim().parse::<f32>().ok())
                                .or_else(|| crate::scene::standard_traffic_lamp(v, r, y, g, request))
                        };
                        let (alpha, light) = slots.values(&known);
                        renderer.set_params(scene, *inst, &alpha, visible, &[]);
                        renderer.set_slot_light(scene, *inst, &light);
                    }
                    None => renderer.set_params(scene, *inst, &[], visible, &[]),
                }
            }
        }
    }

    /// The AI vehicles' pictures: where they stand, their materials, their script
    /// textures (see `sync`).
    fn sync_cars(&mut self, view: &mut TrafficView, world: &World, renderer: &Renderer, scene: &mut Scene) {
        // A far car's script textures (its destination sign) stay as they are drawn: OMSI
        // shows them at any distance its model level has them. (They were stood in for by
        // their mean colour beyond 50 m, and every timetable bus coming up the street had
        // a blank sign until it was almost there.) What a far car's scripts redraw goes to
        // the GPU at most every half second, a slice of the cars per frame.
        let tick = (self.sim.time as f64 * 2.0) as u64;
        let mut budget = SCRIPT_UPLOAD_BUDGET;
        // `Envir_Brightness`, which Omsi.exe sets for every road vehicle as for the
        // player's: the stock buses fade their windows by it at night (left at the engine's
        // default of 1, an AI bus under the street lamps kept its daytime brown glass)
        if let Some(d) = self.sim.daylight {
            for c in self.sim.cars.iter_mut().filter(|c| c.vehicle.ai_visuals) {
                let b = d.envir_brightness(world.light_map_light_at(c.vehicle.position));
                c.vehicle.set_var("Envir_Brightness", b);
            }
        }
        for c in &mut self.sim.cars {
            let Some(r) = view.cars.get_mut(&c.id) else { continue };
            let (render, trailer_renders) = (&mut r.body, &mut r.trailers);
            // out of sight (`tick` decided): hidden once, then left alone until it comes
            // into view again - its many per-mesh updates were a third of this stage
            if !c.vehicle.ai_visuals {
                if !render.hidden {
                    render.hidden = true;
                    for inst in render
                        .instances
                        .iter()
                        .chain(trailer_renders.iter().flat_map(|r| r.instances.iter()))
                    {
                        renderer.set_params(scene, *inst, &[], false, &[]);
                    }
                }
                continue;
            }
            render.hidden = false;
            if let Some(cam) = self.sim.camera {
                let far = (c.vehicle.position - cam).length() > crate::scene::DISPLAYS_FAR;
                let due = render.display_tick != tick;
                render.displays_far = far && !due;
                if far && due {
                    render.display_tick = tick;
                }
            }
            crate::scene::sync_vehicle_textures(renderer, scene, &mut c.vehicle, &*render, &mut budget);
            crate::scene::sync_vehicle_materials(renderer, scene, &c.vehicle, render);
            crate::scene::sync_vehicle_damage(renderer, scene, &mut c.vehicle, render);
            // a coupled part runs no scripts of its own: its plates, its displays and its
            // switched materials follow the leading vehicle's, as the player's own rear
            // sections do (without this an AI bus's rear section kept the blank textures and
            // the unswitched materials it was built with)
            {
                let mut trailers = std::mem::take(&mut c.vehicle.trailers);
                for (t, r) in trailers.iter_mut().zip(trailer_renders.iter_mut()) {
                    crate::scene::sync_vehicle_part(renderer, scene, &c.vehicle, t, r);
                }
                c.vehicle.trailers = trailers;
            }
            // an articulated AI bus (timetable or random traffic) bends its bellows like the
            // player's while it is near enough for the fold to show; farther out its shape
            // just stays as it was, which nobody can tell from still following the road
            if !render.skinned.is_empty()
                || trailer_renders.iter().any(|r| !r.skinned.is_empty())
            {
                let near = self
                    .sim
                    .camera
                    .map(|cam| (c.vehicle.position - cam).length() < SKIN_DISTANCE)
                    .unwrap_or(true);
                if near {
                    crate::scene::sync_skinned(
                        renderer,
                        scene,
                        &mut c.vehicle,
                        render,
                        trailer_renders,
                    );
                }
            }
            for (i, inst) in render.instances.iter().enumerate() {
                renderer.set_transform(
                    scene,
                    *inst,
                    c.vehicle.position,
                    c.vehicle.mesh_local_transform(i),
                );
                let p = &c.vehicle.mesh_props[i];
                let def = &c.vehicle.ty.model.meshes[c.vehicle.ty.meshes[i].def_index];
                let vp = def.viewpoint;
                let vp_ok = vp == 0 || vp & 4 != 0;
                // AI vehicles do not run every cockpit/material script that the player
                // vehicle runs.  Some models consequently leave an `[alphascale]`
                // variable at zero; applying it to an opaque body makes the traffic bus
                // translucent and reveals the interior through its panels.  Opaque slots
                // are never allowed to be faded by a dynamic alpha value; windows and
                // explicitly alpha-tested/blended slots retain their authored behavior.
                let mut alpha = p.slot_alpha.clone();
                for (slot, mat) in scene.instances[*inst].materials.iter().enumerate() {
                    if scene
                        .materials
                        .get(*mat)
                        .is_some_and(|m| m.alpha == omsi_render::AlphaMode::Opaque)
                    {
                        if let Some(a) = alpha.get_mut(slot) {
                            *a = 1.0;
                        }
                    }
                }
                renderer.set_params(scene, *inst, &alpha, p.visible && vp_ok, &p.slot_uv);
                renderer.set_slot_light(scene, *inst, &p.slot_light);
                renderer.set_slot_night(scene, *inst, &p.slot_night);
                renderer.set_interior(scene, *inst, p.interior);
            }
            for (t, r) in c.vehicle.trailers.iter().zip(trailer_renders.iter()) {
                for (i, inst) in r.instances.iter().enumerate() {
                    renderer.set_transform(scene, *inst, t.position, t.mesh_local_transform(i));
                    let p = &t.mesh_props[i];
                    let def = &t.ty.model.meshes[t.ty.meshes[i].def_index];
                    let vp = def.viewpoint;
                    let vp_ok = vp == 0 || vp & 4 != 0;
                    let mut alpha = p.slot_alpha.clone();
                    for (slot, mat) in scene.instances[*inst].materials.iter().enumerate() {
                        if scene
                            .materials
                            .get(*mat)
                            .is_some_and(|m| m.alpha == omsi_render::AlphaMode::Opaque)
                        {
                            if let Some(a) = alpha.get_mut(slot) {
                                *a = 1.0;
                            }
                        }
                    }
                    renderer.set_params(scene, *inst, &alpha, p.visible && vp_ok, &p.slot_uv);
                    renderer.set_slot_light(scene, *inst, &p.slot_light);
                    renderer.set_slot_night(scene, *inst, &p.slot_night);
                    renderer.set_interior(scene, *inst, p.interior);
                }
            }
        }
    }
}
