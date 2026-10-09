//! What the offscreen run says after its steps: the traffic's health, the drive, the
//! clicks into the cab, the people.

use super::*;
use crate::view_sync::{self, ViewSync};

impl Offscreen<'_> {
    /// The traffic's health and its vehicles.
    pub(super) fn traffic_report(&mut self) {
        let Self {
            ref mut traffic,
            ref world,
            ref renderer,
            ref mut scene,
            ref mut sim_view,
            ref player_ref,
            ..
        } = *self;
        if let Some(t) = traffic.as_mut() {
            view_sync::sync(ViewSync::traffic(t), sim_view, world, renderer, scene);
            let buses = t
                .cars
                .iter()
                .filter(|c| c.vehicle.ty.def.passenger_cabin.is_some())
                .count();
            // a car that is stopped where nothing is holding it, or one sitting inside another,
            // is a traffic bug: report both so they can be counted rather than guessed at
            let stuck = t.cars.iter().filter(|c| c.stopped > 60.0).count();
            let mut overlapping = 0;
            for (i, a) in t.cars.iter().enumerate() {
                for b in t.cars.iter().skip(i + 1) {
                    if (a.vehicle.position - b.vehicle.position).length() < 2.5 {
                        overlapping += 1;
                    }
                }
            }
            if let Some(p) = player_ref.as_ref() {
                let near = t
                    .cars
                    .iter()
                    .map(|c| (c.vehicle.position - p.vehicle.position).length())
                    .fold(f64::MAX, f64::min);
                if near < 40.0 {
                    log::info!("nearest AI vehicle to the player: {near:.1} m");
                }
            }
            if omsi_cfg::flags::OMSI_DEBUG_STUCK.is_set() {
                for l in t.stuck_report() {
                    log::info!("stuck: {l}");
                }
            }
            if stuck > 0 || overlapping > 0 {
                log::info!(
                    "traffic health: {stuck} stuck for over a minute, {overlapping} pairs overlapping"
                );
                if omsi_cfg::flags::OMSI_DEBUG_STUCK.is_set() {
                    for c in t.cars.iter().filter(|c| c.stopped > 30.0) {
                        log::info!("  waiting {:.0} s: car {} ({}) lane {} at ({:.1}, {:.1}) lead {:?} why {:?} {:.1} junction {}", c.stopped, c.id, c.vehicle.ty.def.type_name, c.state.lane, c.vehicle.position.x, c.vehicle.position.y, c.lead_car, c.why.0, c.why.1, c.junction_why);
                    }
                }
                for c in t.cars.iter().filter(|c| c.stopped > 60.0).take(4) {
                    log::info!(
                        "  stuck {:.0} s at ({:.0}, {:.0}) on lane {} of {} ({}): {}",
                        c.stopped,
                        c.vehicle.position.x,
                        c.vehicle.position.y,
                        c.state.lane,
                        t.net.lanes.len(),
                        c.vehicle.ty.def.type_name,
                        c.holding.as_deref().unwrap_or("-")
                    );
                }
            }
            if let Some(stats) = t.stats.as_ref() {
                log::info!("{}", stats.summary());
            }
            log::info!("traffic: {} vehicles ({buses} of them buses), {} waiting at red lights, mean speed {:.1} km/h", t.cars.len(), t.held_at_red, t.cars.iter().map(|c| c.state.speed).sum::<f32>() / t.cars.len().max(1) as f32 * 3.6);
            for c in t.cars.iter().filter(|c| c.is_bus()) {
                log::info!("scheduled {} at ({:.1}, {:.1}, {:.1}) heading {:.0} speed {:.1} km/h, {} stops left, at_station={} dwell={:.1} delay={:+.0} s", c.vehicle.ty.def.type_name, c.vehicle.position.x, c.vehicle.position.y, c.vehicle.position.z, c.vehicle.heading, c.state.speed * 3.6, c.bus.as_ref().map(|b| b.stops.len()).unwrap_or(0), c.at_station(), c.standing_for(t.day_time), c.bus.as_ref().map(|b| b.delay).unwrap_or(0.0));
                if omsi_cfg::flags::OMSI_DEBUG_PROPS.is_set() {
                    for v in [
                        "Matrix_Nr",
                        "Matrix_TerminusL1",
                        "Matrix_TerminusL2",
                        "SetLineTo",
                    ] {
                        log::info!("  ${v} = {:?}", c.vehicle.str_var(v));
                    }
                    for v in [
                        "AI_target_index",
                        "IBIS_TerminusIndex",
                        "Matrix_RefreshCursor",
                        "elec_busbar_main",
                        "Font_7x6",
                    ] {
                        log::info!("  {v} = {:?}", c.vehicle.var(v));
                    }
                    log::info!(
                        "  hof: {:?}, fonts: {}",
                        c.vehicle.host.hof.as_ref().map(|h| h.name.clone()),
                        c.vehicle.host.fonts.entries.len()
                    );
                }
                let st = &c.state;
                let lane = &t.net.lanes[st.lane];
                log::info!("  lane {} (key {:?} len {:.1}) s={:.1} route_index {} of {} planned_next {:?} next stop {:?} light {:?}", st.lane, lane.key, lane.length(), st.s, st.route_index, st.route.len(), st.planned_next, c.next_stop(), st.planned_next.and_then(|n| t.net.lanes[n].traffic_light));
            }
        }
    }

    /// The player's bus after the steps: what the drive did (`--drive`), the clicks into
    /// the cab (`--click`), the final camera; the bus becomes `player_ref`.
    pub(super) fn player_report(&mut self) {
        let args = self.args;
        if let Some(mut player) = self.player.take() {
            if let Some(secs) = args.drive {
                self.drive_report(&mut player, secs);
            }
            if let Some(spec) = args.click.clone() {
                self.click_test(&mut player, spec);
            }
            if args.cam.is_none() && args.view != "free" && args.follow.is_none() {
                self.camera = player_view(args, &self.settings, &mut player, &self.camera, &self.world);
            }
            vehicle_camera(&player, &mut self.camera);
            // the driver at the wheel, as the window has him every frame (not posed, he was
            // not drawn - or stood in the aisle in the file's T-pose)
            player.sync_driver(&self.renderer, &mut self.scene, 1.0 / 30.0, self.settings.driver, args.view == "driver");
            self.player_ref = Some(player);
        }
    }

    /// What the drive did: the announcements, where the bus came to rest, the variables, the
    /// collisions, the tyres, the distance driven.
    fn drive_report(&mut self, player: &mut Player, secs: f32) {
        let args = self.args;
        let Self {
            ref drive_start,
            ref spawn_z,
            ref renderer,
            ref mut scene,
            ref settings,
            ref world,
            ref traffic,
            ..
        } = *self;
        let drive_spawn_z = *spawn_z;
        let start = *drive_start;
        pose_player(player, renderer, scene, args, settings);
        for (t, f) in std::mem::take(&mut player.vehicle.host.fired_file_triggers) {
            log::info!("announcement: {t} -> {f}");
        }
        // OMSI_DEBUG_REST: where the bus came to rest against the ground under it (a
        // bus sunk into the road, or hanging over it, after spawning)
        if omsi_cfg::flags::OMSI_DEBUG_REST.is_set() {
            let p = player.vehicle.position;
            log::info!(
                "rest: entry {} bus at ({:.1}, {:.1}, {:.2}) heading {:.0}; road/ground there {:?}, walk {:?}, spawned at z {:.2}",
                args.entry,
                p.x,
                p.y,
                p.z,
                player.vehicle.heading,
                world.ground_height(p.x, p.y),
                world.walk_height(p.x, p.y),
                drive_spawn_z
            );
            let v = &player.vehicle;
            let wheels: Vec<String> = tyre_lows(v, world).iter().map(|(_, d)| format!("{d:+.3}")).collect();
            log::info!("rest wheels: {} lowest tyre points against the road: [{}]", v.ty.def.type_name, wheels.join(", "));
        }
        if omsi_cfg::flags::OMSI_DEBUG_HUMANS.is_set() {
            log::info!(
                "people per cabin path link: {:?}",
                player.vehicle.host.humans_on_path_link
            );
        }
        if omsi_cfg::flags::OMSI_DEBUG_PROPS.is_set() {
            if let Some(probe) = player.vehicle.host.ground_probe.clone() {
                log::info!(
                    "ground probe: at the origin {:+.2} m, 1 m up {:+.2}, 1 m down {:+.2}",
                    probe(0.0, 0.0, 0.0),
                    probe(0.0, 0.0, 1.0),
                    probe(0.0, 0.0, -1.0)
                );
            }
        }
        {
            let e = DRIVE_EXTREMES.lock();
            log::info!("drive extremes: pitch {:.1} (at ({:.1}, {:.1}, {:.1}), {:.1} s) bank {:.1}, origin {:+.2}..{:+.2} m over the ground", e.0, e.4 .0.x, e.4 .0.y, e.4 .0.z, e.4 .1, e.1, e.3, e.2);
        }
        if let Some(list) = omsi_cfg::flags::OMSI_DEBUG_VARS.var() {
            for v in list.split(',').map(str::trim).filter(|v| !v.is_empty()) {
                log::info!(
                    "after drive: {v} = {:?} / {:?}",
                    player.vehicle.var(v),
                    player.vehicle.str_var(v)
                );
            }
        }
        if omsi_cfg::flags::OMSI_DEBUG_PROPS.is_set() {
            for v in [
                "IBIS_mode",
                "IBIS_RouteIndex",
                "IBIS_TerminusIndex",
                "IBIS_TerminusCode",
                "IBIS_LinieKurs",
                "elec_busbar_main",
                "Rain_Window_Front_Wetness",
                "PrecipRate",
                "GivenTicket",
                "ticketprinter_ticket_selection",
                "ticketprinter_ticket_preselection",
                "ticketprinter_druckt",
                "ticketprinter_ticket_pos",
            ] {
                log::info!("after drive: {v} = {:?}", player.vehicle.var(v));
            }
            for v in [
                "IBIS_terminus_name",
                "IBIS_Complex_Line",
                "IBIS_busstop_name",
                "act_busstop",
                "Haltestelle",
                "Matrix_Nr",
                "Matrix_TerminusL1",
                "Matrix_Terminus",
                "Matrix_Bitmapfilename",
            ] {
                log::info!("after drive: ${v} = {:?}", player.vehicle.str_var(v));
            }
            for v in [
                "Matrix_RefreshCursor",
                "matrix_steckschild_Termindex",
                "Font_16x9",
                "elec_busbar_main_sw",
                "AI_target_index",
            ] {
                log::info!("after drive: {v} = {:?}", player.vehicle.var(v));
            }
            for (i, st) in player.vehicle.host.script_textures.iter().enumerate() {
                let lit = st.rgba.chunks_exact(4).filter(|p| p[3] > 0).count();
                log::info!(
                    "script texture {i}: {}x{} {lit} pixels with alpha, dirty={} locked={} mipmaps={}",
                    st.width,
                    st.height,
                    st.dirty,
                    st.locked,
                    st.mipmaps
                );
            }
            if let Some(dir) = omsi_cfg::flags::OMSI_DUMP_SCRIPTTEX.var() {
                dump_display_textures(&player.vehicle, Path::new(&dir));
            }
            log::info!(
                "fonts registered: {:?}",
                player
                    .vehicle
                    .host
                    .fonts
                    .entries
                    .iter()
                    .map(|f| (f.0.clone(), f.1.is_some()))
                    .collect::<Vec<_>>()
            );
        }
        log::info!(
            "collision: {} crashes, the last {:.1} kJ; unread energy {:.1} kJ, last point {:?}",
            player.vehicle.crashes,
            player.vehicle.last_impact / 1000.0,
            player.vehicle.host.coll_energy,
            player.vehicle.host.coll_pos
        );
        if omsi_cfg::flags::OMSI_DEBUG_COLLISION.is_set() {
            if let Some(cw) = player.vehicle.collision.as_ref() {
                let p = player.vehicle.position;
                let mut near: Vec<(f64, &omsi_sim::collision::Obb)> = cw
                    .boxes
                    .iter()
                    .map(|b| ((b.center - p.truncate()).length(), b))
                    .collect();
                near.sort_by(|a, b| a.0.total_cmp(&b.0));
                for (d, b) in near.iter().take(4) {
                    log::info!("  nearest obstacle {:.1} m away at ({:.1}, {:.1}) half {:.1}x{:.1} z {:.1}..{:.1}", d, b.center.x, b.center.y, b.half.x, b.half.y, b.z0, b.z1);
                }
                log::info!(
                    "  bus at ({:.1}, {:.1}, {:.1}) box {:?}",
                    p.x,
                    p.y,
                    p.z,
                    player.vehicle.ty.def.bounding_box
                );
            }
        }
        if omsi_cfg::flags::OMSI_DEBUG_PHYSICS.is_set() {
            let gaps = |v: &omsi_sim::VehicleInstance| {
                v.wheel_ground_gaps()
                    .iter()
                    .map(|(f, g)| {
                        format!("{} {:+.3}", f.rsplit(['\\', '/']).next().unwrap_or(f), g)
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            log::info!("tyres over the ground: {}", gaps(&player.vehicle));
            if let Some(t) = traffic.as_ref() {
                for c in t.cars.iter().filter(|c| {
                    (c.vehicle.position - player.vehicle.position).length() < 1500.0
                }) {
                    let def = &c.vehicle.ty.def;
                    let name = if def.type_name.is_empty() {
                        def.path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        def.type_name.clone()
                    };
                    log::info!("  AI {} {}: {}", c.id, name, gaps(&c.vehicle));
                }
            }
        }
        if let Some(rb) = &player.vehicle.rigid {
            log::info!("rigid: pos ({:.1}, {:.1}, {:.2}) heading {:.1} pitch {:.2} bank {:.2} v {:?} compressions {:?}", player.vehicle.position.x, player.vehicle.position.y, player.vehicle.position.z, player.vehicle.heading, player.vehicle.pitch, player.vehicle.bank, rb.velocity, rb.wheels.iter().map(|w| (w.compression * 1000.0).round() / 1000.0).collect::<Vec<_>>());
        }
        log::info!(
            "drove {:.1} m in {secs} s, now {:.1} km/h, engine_n={:?} M_Wheel={:?} gear={:?}",
            (player.vehicle.position - start).length(),
            player.vehicle.physics.velocity_kmh(),
            player.vehicle.var("engine_n"),
            player.vehicle.var("M_Wheel"),
            player.vehicle.var("antrieb_getr_aktugang")
        );
        if let Some(mins) = player.vehicle.repair_minutes() {
            log::info!("damage: the workshop would need {mins:.0} min (elec {:?} engine {:?} drive {:?})", player.vehicle.var("elec_failure_general"), player.vehicle.var("engine_failure_general"), player.vehicle.var("antrieb_failure_general"));
        }
    }

    /// `--click x,y[,dx,dy]`: a click into the cab where the picture is taken from, the
    /// switches on the screen, and the switch tests asked for.
    fn click_test(&mut self, player: &mut Player, spec: String) {
        let args = self.args;
        let Self {
            ref settings,
            ref camera,
            ref renderer,
            ref mut scene,
            ..
        } = *self;
        let v: Vec<f32> = spec
            .split(',')
            .filter_map(|x| x.trim().parse().ok())
            .collect();
        if v.len() >= 2 {
            let (w, h) = args
                .size
                .split_once('x')
                .map(|(a, b)| {
                    (
                        a.parse::<u32>().unwrap_or(1600),
                        b.parse::<u32>().unwrap_or(900),
                    )
                })
                .unwrap_or((1600, 900));
            // the same camera the picture is taken with, head turn and all
            let look = crate::player::driver_head_look(
                look_of(args),
                &args.view,
                settings.seat_pitch_deg,
                false,
            );
            let cam =
                player.camera_look(&args.view, camera, look, offscreen_orbit());
            let (o, d) = cursor_ray(&cam, v[0], v[1], w as f32, h as f32);
            match player.click(o, d, pixel_angle(&cam, h as f32) * 6.0) {
                Some(i) => log::info!(
                    "click at ({}, {}) hit mesh {i} '{}'",
                    v[0],
                    v[1],
                    player.vehicle.ty.model.meshes[player.vehicle.ty.meshes[i].def_index].file
                ),
                None => log::info!(
                    "click at ({}, {}) hit nothing with a [mouseevent]",
                    v[0],
                    v[1]
                ),
            }
            // where the clickable switches actually are on this screen
            let vp = cam.view_proj(w as f32 / h as f32, player.vehicle.position);
            let mut switches: Vec<(String, f32, f32)> = Vec::new();
            {
                let (mut total, mut invisible, mut offscreen) = (0, 0, 0);
                for (i, vm) in player.vehicle.ty.meshes.iter().enumerate() {
                    let def = &player.vehicle.ty.model.meshes[vm.def_index];
                    if def.mouse_event.is_none() {
                        continue;
                    }
                    total += 1;
                    if !player.vehicle.mesh_props[i].visible {
                        invisible += 1;
                        log::info!(
                            "  switch '{}' is invisible ([visible] {:?})",
                            def.mouse_event.as_deref().unwrap_or(""),
                            def.visible
                        );
                    } else {
                        offscreen += 1;
                    }
                }
                log::info!("switches on this vehicle: {total}, of them {invisible} invisible, {offscreen} visible (some off screen)");
            }
            for (i, vm) in player.vehicle.ty.meshes.iter().enumerate() {
                let def = &player.vehicle.ty.model.meshes[vm.def_index];
                let Some(ev) = def.mouse_event.as_ref() else {
                    continue;
                };
                if !player.vehicle.mesh_props[i].visible || vm.data.positions.is_empty() {
                    continue;
                }
                let mut c = Vec3::ZERO;
                for q in &vm.data.positions {
                    c += *q;
                }
                c /= vm.data.positions.len() as f32;
                let world = player.vehicle.mesh_local_transform(i).transform_point3(c);
                let ndc = vp.project_point3(world);
                if ndc.z < 0.0 || ndc.z > 1.0 || ndc.x.abs() > 1.0 || ndc.y.abs() > 1.0 {
                    continue;
                }
                let (sx, sy) = (
                    (ndc.x * 0.5 + 0.5) * w as f32,
                    (0.5 - ndc.y * 0.5) * h as f32,
                );
                log::info!(
                    "  switch '{ev}' at screen ({sx:.0}, {sy:.0}): {}",
                    describe::names(&args.root, &settings.language).control(ev)
                );
                switches.push((ev.clone(), sx, sy));
            }
            // Aim at each of them in turn and say which one would actually be operated:
            // a switch that cannot be hit where it is drawn, or that hands the click to
            // its neighbour, is unusable with the mouse however good the rest is.
            if omsi_cfg::flags::OMSI_CLICK_ALL.is_set() {
                let spread = pixel_angle(&cam, h as f32) * 6.0;
                let (mut hit, mut wrong, mut missed) = (0, 0, 0);
                for (ev, sx, sy) in &switches {
                    let (o, d) = cursor_ray(&cam, *sx, *sy, w as f32, h as f32);
                    match player.pick(o, d, spread).map(|i| {
                        player.vehicle.ty.model.meshes[player.vehicle.ty.meshes[i].def_index]
                            .mouse_event
                            .clone()
                            .unwrap_or_default()
                    }) {
                        Some(got) if &got == ev => hit += 1,
                        Some(got) => {
                            wrong += 1;
                            log::info!(
                                "  aiming at '{ev}' ({sx:.0}, {sy:.0}) operates '{got}'"
                            );
                        }
                        None => {
                            missed += 1;
                            log::info!("  aiming at '{ev}' ({sx:.0}, {sy:.0}) hits nothing");
                        }
                    }
                }
                log::info!("switch test: {hit} of {} operable, {wrong} hand the click to a neighbour, {missed} unreachable", switches.len());
            }
            // Does the script answer at all? Every [mouseevent] of the model is fired
            // here (whether it is on screen or not) and the vehicle's variables are
            // compared before and after: a switch whose trigger the script does not
            // define, or that changes nothing, is a switch that does nothing when
            // clicked.
            if omsi_cfg::flags::OMSI_TRIGGER_ALL.is_set() {
                trigger_test(player);
            }
            // and a drag, so the offscreen test can turn a knob too
            if let Some(v3) = v.get(2) {
                player.drag(*v3, v.get(3).copied().unwrap_or(0.0));
            }
            player.release();
            // Let the switch move: the picture is taken after this, and a lever that
            // has been flipped only travels when the vehicle's scripts and animations
            // are run once more (in the window that happens on the next frame anyway).
            for _ in 0..12 {
                player.vehicle.update(0.05);
            }
            pose_player(player, renderer, scene, args, settings);
            if let Some(names) = omsi_cfg::flags::OMSI_DEBUG_VARS.var() {
                for n in names.split(',') {
                    log::info!("after click: {n} = {:?}", player.vehicle.var(n.trim()));
                }
            }
        }
    }

    /// The people after the steps: their pictures, their counts, the player's doors and the
    /// stops.
    pub(super) fn humans_report(&mut self) {
        let Self {
            ref mut humans_off,
            ref player_ref,
            ref camera,
            ref world,
            ref renderer,
            ref mut scene,
            ref mut sim_view,
            ..
        } = *self;
        if let Some(mut h) = humans_off.take() {
            let center = player_ref
                .as_ref()
                .map(|p| p.vehicle.position)
                .unwrap_or(camera.position);
            let bus = player_ref.as_ref().map(|p| &p.vehicle);
            view_sync::sync(ViewSync::people(&mut h, bus, center), sim_view, world, renderer, scene);
            log::info!(
                "passengers: {} people ({}), request {:?}, paid {:?}, change due {:?}",
                h.people.len(),
                h.summary(&sim_view.people),
                h.request,
                h.paid,
                h.change_due
            );
            if omsi_cfg::flags::OMSI_DEBUG_HUMANS.is_set() {
                let centre = player_ref
                    .as_ref()
                    .map(|p| p.vehicle.position)
                    .unwrap_or(camera.position);
                for (k, p) in h
                    .positions()
                    .into_iter()
                    .filter(|(_, p)| (*p - centre).length() < 40.0)
                {
                    log::info!("  {k} at ({:.1}, {:.1}, {:.1})", p.x, p.y, p.z);
                }
            }
            if omsi_cfg::flags::OMSI_DEBUG_HUMANS.is_set() {
                for p in &h.people {
                    log::info!(
                        "  {:?} at ({:.1}, {:.1}, {:.1})",
                        p.state_name(),
                        p.position().x,
                        p.position().y,
                        p.position().z
                    );
                }
            }
            if let Some(p) = player_ref.as_ref() {
                log::info!(
                    "player doors {:?} PAX_Entry_Open {:?} PAX_Entry_Req {:?} PAX_Exit_Open {:?} PAX_Exit_Req {:?} haltewunsch {:?} speed {:.1}",
                    (0..4).map(|i| p.vehicle.var(&format!("door_{i}")).unwrap_or(0.0)).collect::<Vec<_>>(),
                    (0..2).map(|i| p.vehicle.var(&format!("PAX_Entry{i}_Open")).unwrap_or(0.0)).collect::<Vec<_>>(),
                    (0..2).map(|i| p.vehicle.var(&format!("PAX_Entry{i}_Req")).unwrap_or(0.0)).collect::<Vec<_>>(),
                    (0..2).map(|i| p.vehicle.var(&format!("PAX_Exit{i}_Open")).unwrap_or(0.0)).collect::<Vec<_>>(),
                    (0..2).map(|i| p.vehicle.var(&format!("PAX_Exit{i}_Req")).unwrap_or(0.0)).collect::<Vec<_>>(),
                    p.vehicle.var("haltewunsch"),
                    p.vehicle.physics.velocity_kmh()
                );
                if omsi_cfg::flags::OMSI_DEBUG_HUMANS.is_set() {
                    for (id, pos, rot, name) in world.bus_stops.lock().iter() {
                        log::info!(
                            "  bus stop {id} '{name}' at ({:.1}, {:.1}) heading {rot:.0}",
                            pos.x,
                            pos.y
                        );
                    }
                }
                if let Some((id, pos, _, name)) = world.bus_stops.lock().iter().min_by(|a, b| (a.1 - p.vehicle.position).length().total_cmp(&(b.1 - p.vehicle.position).length())) {
                    log::info!(
                        "nearest bus stop {id} '{name}' at ({:.1}, {:.1}) is {:.1} m away",
                        pos.x,
                        pos.y,
                        (*pos - p.vehicle.position).length()
                    );
                }
            }
            // their view goes with the people (an `OMSI_TRACE_PAX` trace written out now)
            sim_view.people = Default::default();
        }
    }
}

/// `OMSI_TRIGGER_ALL`: every switch's trigger fired, and what it changed (see `click_test`).
fn trigger_test(player: &mut Player) {
    let names: Vec<String> = {
        let mut v: Vec<String> = player
            .vehicle
            .ty
            .meshes
            .iter()
            .filter_map(|m| {
                player.vehicle.ty.model.meshes[m.def_index]
                    .mouse_event
                    .clone()
            })
            .collect();
        v.sort();
        v.dedup();
        v
    };
    let (mut dead, mut silent, mut ok) = (Vec::new(), Vec::new(), 0);
    // Each switch is tried from the same variables, and what it changed is
    // measured against the same time passing without it: with the engine
    // running half the variables move by themselves, and counting those made
    // every switch look alive. Variables that differ between two identical
    // runs (random numbers) are left out.
    let v0 = &player.vehicle;
    let start = (
        v0.state.clone(),
        v0.host.clock.clone(),
        v0.position,
        v0.heading,
        v0.physics.clone(),
        v0.rigid.clone(),
    );
    let restore = |v: &mut omsi_sim::VehicleInstance| {
        v.state = start.0.clone();
        v.host.clock = start.1.clone();
        v.position = start.2;
        v.heading = start.3;
        v.physics = start.4.clone();
        v.rigid = start.5.clone();
    };
    // One try: press, drag by `d`, hold, let go. Returns whether the script
    // has the trigger, the variables while held and after letting go, and the
    // sound events. The idle run (no switch) is the same with nothing pressed.
    let run = |v: &mut omsi_sim::VehicleInstance,
               name: Option<&str>,
               d: (f32, f32)|
               -> (bool, Vec<f32>, Vec<f32>, Vec<String>) {
        restore(v);
        v.host.fired_triggers.clear();
        v.host.fired_file_triggers.clear();
        let mut exists = false;
        if let Some(name) = name {
            exists = v.trigger(name);
            v.host.mouse = d;
            exists |= v.trigger(&format!("{name}_drag"));
            v.host.mouse = (0.0, 0.0);
        }
        for _ in 0..6 {
            v.update(0.05);
        }
        let held = v.state.vars.clone();
        if let Some(name) = name {
            v.trigger(&format!("{name}_off"));
        }
        for _ in 0..6 {
            v.update(0.05);
        }
        let mut sounds: Vec<String> = std::mem::take(&mut v.host.fired_triggers);
        sounds.extend(
            std::mem::take(&mut v.host.fired_file_triggers)
                .into_iter()
                .map(|(t, _)| t),
        );
        (exists, held, v.state.vars.clone(), sounds)
    };
    let (_, idle_held, idle, idle_sounds) =
        run(&mut player.vehicle, None, (0.0, 0.0));
    let (_, idle_held2, idle2, _) = run(&mut player.vehicle, None, (0.0, 0.0));
    let differs = |a: f32, b: f32| (a - b).abs() > 1e-4 * (1.0 + a.abs());
    // what moves by itself between two identical runs (random numbers)
    let noisy: Vec<bool> = (0..idle.len())
        .map(|k| differs(idle[k], idle2[k]) || differs(idle_held[k], idle_held2[k]))
        .collect();
    for name in &names {
        let mut exists = false;
        let mut changed: Vec<usize> = Vec::new();
        let mut played: Vec<String> = Vec::new();
        // knobs and levers answer to one drag direction only (the driver's
        // door and window sideways, the parking brake and the sign clamp
        // up or down)
        for d in [(0.0, 6.0), (0.0, -6.0), (6.0, 0.0), (-6.0, 0.0)] {
            let (e, held, after, sounds) =
                run(&mut player.vehicle, Some(name.as_str()), d);
            exists |= e;
            changed = (0..idle.len().min(after.len()))
                .filter(|&k| {
                    !noisy[k]
                        && (differs(after[k], idle[k])
                        || differs(held[k], idle_held[k]))
                })
                .collect();
            played = sounds
                .into_iter()
                .filter(|t| !idle_sounds.contains(t))
                .collect();
            if !e || !changed.is_empty() || !played.is_empty() {
                break;
            }
        }
        if !exists {
            dead.push(name.clone());
        } else if changed.is_empty() && played.is_empty() {
            silent.push(name.clone());
        } else {
            ok += 1;
            if omsi_cfg::flags::OMSI_DEBUG_TRIGGERS.is_set() {
                let names: Vec<&str> = changed
                    .iter()
                    .take(6)
                    .filter_map(|k| {
                        player
                            .vehicle
                            .ty
                            .program
                            .var_names
                            .get(*k)
                            .map(|s| s.as_str())
                    })
                    .collect();
                log::info!(
                    "  {name}: {} variables, e.g. {names:?}; sounds {played:?}",
                    changed.len()
                );
            }
        }
    }
    restore(&mut player.vehicle);
    log::info!("trigger test: {ok} of {} switches do something; {} have no trigger in the script: {:?}", names.len(), dead.len(), dead);
    log::info!("  {} fire a trigger but change nothing (may need power or another switch first): {:?}", silent.len(), silent);
}
