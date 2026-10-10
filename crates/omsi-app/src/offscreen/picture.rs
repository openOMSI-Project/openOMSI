//! The offscreen run's picture: the personnel file and the situation saved, then the
//! final moment's lighting, scenery, lights, HUD and mirrors, and the picture written.

use super::*;

impl Offscreen<'_> {
    /// The personnel file and `--save-situation`; the clock of the run's last moment.
    pub(super) fn save_run(&mut self) -> omsi_sim::SimClock {
        let (args, service_seconds) = (self.args, self.service_seconds);
        let Self {
            ref mut career,
            ref world,
            ref player_ref,
            ref camera,
            ref duty,
            ..
        } = *self;
        if career.path.is_some() {
            if let Err(e) = career.save() {
                log::warn!("writing the personnel file: {e}");
            }
        } else if career.metres > 1.0 {
            log::info!("this run: {}", career.summary());
        }
        let clock = {
            let mut c = start_clock(args);
            c.time += args.drive.unwrap_or(0.0) as f64 + service_seconds;
            c
        };
        if let Some(out) = &args.save_situation {
            let sit = build_situation(
                args,
                world,
                &clock,
                args.weather.as_deref(),
                player_ref.as_ref(),
                &[],
                camera,
                duty.as_ref(),
                "openOMSI save",
            );
            match sit.save(out) {
                Ok(()) => log::info!(
                    "saved situation {} ({} vehicles)",
                    out.display(),
                    sit.vehicles.len()
                ),
                Err(e) => log::warn!("saving {}: {e}", out.display()),
            }
        }
        clock
    }

    /// The final picture at `clock`.
    pub(super) fn picture(&mut self, clock: omsi_sim::SimClock) -> Result<()> {
        let (daylight, lighting) = self.final_lighting(&clock);
        self.settle_scenery(&clock, &daylight);
        self.final_lights(&daylight, &lighting);
        self.final_hud(&clock);
        checks::road_photo(&mut self.renderer, &mut self.scene, self.traffic.as_ref(), &lighting);
        self.bench(&lighting);
        self.final_textures(&lighting)?;
        self.final_render(&lighting)
    }

    /// The street lamps, the lit windows and the lighting at `clock` (the weather the
    /// physical model has then).
    fn final_lighting(&mut self, clock: &omsi_sim::SimClock) -> (omsi_sim::Daylight, omsi_render::Lighting) {
        let (args, service_seconds) = (self.args, self.service_seconds);
        let Self {
            ref world,
            ref mut renderer,
            ref mut scene,
            ref envir,
            ref mut weather,
            ref settings,
            ref player_ref,
            ref player,
            ref cabin_air,
            wetness,
            ..
        } = *self;
        let daylight = omsi_sim::Daylight::compute(clock, envir.as_ref());
        steps::world_lamps(world, renderer, scene, clock, &daylight, true, true);
        // (the physical model at the map's own place and the picture's moment)
        *weather = crate::weather_model::refresh(clock).unwrap_or(std::mem::take(weather));
        // (the roads as wet as the run left them: a run starts with them in the state this
        // weather would leave them; OMSI_WETNESS in their place)
        // the player's vehicle has moved into `player_ref` by now (after --drive): without
        // this the offscreen picture had no cab box, unlike the window
        let driven = player_ref.as_ref().or(player.as_ref()).map(|p| &p.vehicle);
        let mut lighting = steps::picture_lighting(
            &daylight,
            Some(weather),
            cloud_drift_at(weather, clock.time),
            wetness,
            Some(world),
            driven,
            driven,
            cabin_air.appearance(),
            settings,
            args.drive.unwrap_or(0.0) + service_seconds as f32,
        );
        // OMSI_CONDENSATION=<minutes>,<people>[,engine 0/1]: the cabin air and the condensation
        // on the player's glass after that long with that many aboard
        if let (Some(spec), Some(p)) = (omsi_cfg::flags::OMSI_CONDENSATION.var(), player_ref.as_ref().or(player.as_ref())) {
            let mut it = spec.split(',').map(|x| x.trim().parse::<f32>().unwrap_or(0.0));
            let (minutes, people, engine) = (it.next().unwrap_or(15.0), it.next().unwrap_or(30.0) as usize, it.next().unwrap_or(1.0) > 0.5);
            let mut ci = crate::condensation::inputs_for(&p.vehicle, weather, people, 0);
            ci.engine = engine;
            let mut cabin = crate::condensation::CabinAir::new();
            for _ in 0..(minutes * 60.0) as usize {
                cabin.step(1.0, &ci);
            }
            log::info!("condensation after {minutes} min, {people} aboard: {cabin:?} -> {:?}", cabin.appearance());
            lighting.condensation = cabin.appearance();
        }
        // OMSI_GLASS_WIND=<m/s>: the rain on the glass as the bus would meet it at that speed
        if let (Some(v), Some(p)) = (omsi_cfg::flags::OMSI_GLASS_WIND.parse::<f32>(), player_ref.as_ref().or(player.as_ref())) {
            let h = p.vehicle.heading.to_radians();
            lighting.glass_wind = glam::Vec3::new(h.sin() as f32, h.cos() as f32, 0.0) * v;
        }
        (daylight, lighting)
    }

    /// The scenery's scripts run a few frames, so that their animations settle.
    fn settle_scenery(&mut self, clock: &omsi_sim::SimClock, daylight: &omsi_sim::Daylight) {
        let args = self.args;
        let Self {
            ref traffic,
            ref mut schedule,
            ref world,
            ref duty,
            ref renderer,
            ref mut scene,
            ref camera,
            ..
        } = *self;
        {
            // scenery scripts: a few frames so animations settle
            let phase = |c: usize, li: usize| {
                traffic
                    .as_ref()
                    .map(|t| t.light_vars(c, li))
                    .unwrap_or((omsi_sim::traffic::UNLINKED_PHASE as f32, 0.0))
            };
            let dt = 1.0 / 30.0;
            let mut n = 0;
            for _ in 0..(args.drive.unwrap_or(1.0) / dt).max(3.0) as usize {
                // the time of day and the departure displays' boards (the first pass says which
                // stops have displays)
                steps::departure_boards(
                    schedule.as_mut(),
                    world,
                    traffic.as_ref(),
                    duty.as_ref(),
                    clock,
                );
                n = world.update_scripted(
                    renderer,
                    scene,
                    dt,
                    camera.position,
                    daylight.brightness,
                    &phase,
                    None,
                    false,
                );
            }
            log::info!(
                "scenery scripts: {} objects updated of {}",
                n,
                world.scripted.lock().len()
            );
        }
    }

    /// The lights, the rain or snow and the tyres' spray of the final moment, and what the
    /// run says of them.
    fn final_lights(&mut self, daylight: &omsi_sim::Daylight, lighting: &omsi_render::Lighting) {
        let Self {
            ref player_ref,
            ref player,
            ref traffic,
            ref remotes_off,
            ref weather,
            ref mut renderer,
            ref world,
            ref camera,
            ref mut scene,
            ref spray,
            ..
        } = *self;
        {
            let vehicles = steps::light_vehicles(player_ref.as_ref(), traffic.as_ref(), remotes_off);
            world_lights(renderer, world, scene, weather, daylight, camera.position, &vehicles);
            let (kind, rate) = precip_of(weather);
            let mut rn = rain::Rain::new();
            rn.set(kind, rate);
            for _ in 0..30 {
                scene
                    .coronas
                    .retain(|c| c.cone_cos > -1.5 && !(c.size < 0.07 && c.brightness < 0.95));
                rn.tick(
                    1.0 / 30.0,
                    camera.position,
                    // ([wind] direction (deg) and speed (m/s), as the window's frame takes it)
                    rain::weather_wind(weather),
                    scene,
                    &player_ref.as_ref().or(player.as_ref()).map(|p| rain::vehicle_boxes(&p.vehicle)).unwrap_or_default(),
                );
            }
            // the tyres' spray as the drive left it (OMSI_DRIVE_V0=S moves the bus without a
            // full engine-start sequence)
            spray.sprites(camera.position, &mut scene.smoke);
            log::info!(
                "spray: {} puffs; at the end {} tyres threw water, {} of them in a puddle",
                spray.len(),
                spray.tyres_wet,
                spray.tyres_in_puddle
            );
            log::info!(
                "daylight: sun altitude {:.1}°, night {:.2}, lamps {}, {} lights, {} coronas",
                daylight.altitude_deg,
                daylight.night,
                daylight.lamps_on,
                scene.lights.len(),
                scene.coronas.len()
            );
            log::info!(
                "  light A (sun) {:?} B (sky) {:?} C (ambient) {:?} sky {:?} fog {:?} density {:.5}",
                daylight.sun_color,
                daylight.secondary,
                daylight.ambient,
                daylight.sky,
                lighting.fog_color,
                lighting.fog_density
            );
            {
                let mut near: Vec<&omsi_render::PointLight> = scene.lights.iter().collect();
                near.sort_by(|a, b| (a.position - camera.position).length().total_cmp(&(b.position - camera.position).length()));
                for l in near.iter().take(4) {
                    log::info!(
                        "  light {:.0} m away: colour {:?} radius {:.1} intensity {:.2}",
                        (l.position - camera.position).length(),
                        l.color,
                        l.radius,
                        l.intensity
                    );
                }
            }
        }
    }

    /// The HUD and the navigator over the picture, as the window shows them.
    fn final_hud(&mut self, clock: &omsi_sim::SimClock) {
        let (w, h) = (self.w, self.h);
        let Self {
            ref player_ref,
            ref world,
            ref duty_error,
            ref lan_off,
            ref remotes_off,
            ref settings,
            ref renderer,
            ref mut scene,
            ref traffic,
            ref duty,
            ref schedule,
            ref humans_off,
            ..
        } = *self;
        if let Some(p) = player_ref.as_ref() {
            let mut hud = hud::Hud::new(&mut world.fonts.lock());
            let t = clock.time;
            let mut lines = vec![
                format!(
                    "{:02}:{:02}:{:02}",
                    (t / 3600.0) as i32,
                    ((t % 3600.0) / 60.0) as i32,
                    (t % 60.0) as i32
                ),
                format!(
                    "{:.0} km/h   {}",
                    p.vehicle.physics.velocity_kmh().abs(),
                    p.vehicle.ty.def.type_name
                ),
            ];
            if !p.vehicle.host.tt_line.is_empty() {
                let next = p
                    .vehicle
                    .host
                    .tt_stops
                    .get(p.vehicle.host.tt_busstop_index.max(0) as usize)
                    .map(|s| s.0.clone())
                    .unwrap_or_default();
                lines.push(format!(
                    "Line {}   next: {}   {:+.0} s",
                    p.vehicle.host.tt_line, next, p.vehicle.host.tt_delay
                ));
            }
            if let Some(e) = duty_error.as_ref() {
                lines.push(format!("No duty: {e}"));
            }
            if let Some(l) = lan_off.as_ref() {
                lines.extend(lan::hud_lines(l, remotes_off, Some(p)));
            }
            let viewport = settings.hud_viewport((w, h));
            let overlay_start = scene.overlays.len();
            hud.update(renderer, scene, &lines);
            crate::ui::shift_overlays(scene, overlay_start, viewport[0]);
            // the navigator, as the window shows it (its camera settled first)
            if settings.navigator {
                let mut nav = navigator::Navigator::new(true, settings.ui_opacity, &settings.navigator_corner);
                nav.schedule = omsi_cfg::flags::OMSI_NAV_SCHEDULE.is_set();
                nav.show_ai = settings.nav_ai;
                if omsi_cfg::flags::OMSI_NAV_MAP.is_set() {
                    nav.toggle_map();
                }
                if traffic.is_none() {
                    nav.add_lanes(world.lanes.lock().clone());
                }
                nav.set_map(world.navigation_map());
                let (line, terminus, stops, trip) = navigator::duty_parts(duty.as_ref());
                if let (Some((key, name)), Some(sch)) = (trip, schedule.as_ref()) {
                    let lanes = sch.trip_route_in(nav.map_net().unwrap(), &name);
                    let g = nav.global_version + (1 << 40);
                    nav.set_route(&key, lanes, true, g);
                }
                let (outside_temp, inside_temp) = crate::app_events::vehicle_temperatures(p);
                let frame = navigator::NavFrame {
                    traffic: traffic.as_ref(),
                    players: lan_off.as_ref().map(|l| lan::nav_players(remotes_off, l.my_id)).unwrap_or_default(),
                    bus: p.vehicle.position,
                    heading: p.vehicle.heading,
                    speed_kmh: p.vehicle.physics.velocity_kmh(),
                    outside_temp,
                    inside_temp,
                    line,
                    terminus,
                    stops,
                    delay: duty.as_ref().map(|_| p.vehicle.host.tt_delay as f64),
                    passengers: humans_off.as_ref().map(|h| h.riding()),
                    stop_requested: navigator::stop_requested(&p.vehicle),
                    time: clock.time,
                    weekday: clock.weekday(),
                    language: &settings.language,
                    screen: (viewport[2], viewport[3]),
                    ui_scale: settings.ui_scale,
                    follow_window: settings.ui_scale_window,
                    dt: 0.1,
                    info_rect: None,
                };
                for _ in 0..30 {
                    nav.frame_at(renderer, scene, &frame, viewport[0]);
                    scene.overlays.pop();
                }
                nav.frame_at(renderer, scene, &frame, viewport[0]);
            }
        }
    }

    /// `OMSI_BENCH=n`.
    fn bench(&mut self, lighting: &omsi_render::Lighting) {
        let (w, h) = (self.w, self.h);
        let Self {
            ref mut renderer,
            ref mut scene,
            ref world,
            ref player_ref,
            ref camera,
            ..
        } = *self;
        // OMSI_BENCH=n: the final picture drawn n more times as a window frame would be (one
        // mirror, then the view), with the median CPU time of the drawing calls and of the wait
        // for the GPU - medians shrug off what else the machine is doing
        if let Some(n) = omsi_cfg::flags::OMSI_BENCH.parse::<usize>()
        {
            let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("bench"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: renderer.format(),
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = target.create_view(&Default::default());
            let (mut cpu, mut gpu) = (Vec::with_capacity(n), Vec::with_capacity(n));
            for k in 0..n {
                let t = Instant::now();
                if let Some(p) = player_ref.as_ref() {
                    render_mirrors(renderer, scene, world, p, lighting, Some(k), None);
                }
                renderer.render(scene, &view, w, h, camera, lighting);
                let drawn = t.elapsed().as_secs_f64();
                let t = Instant::now();
                let _ = omsi_render::wait_gpu(&renderer.device, None);
                cpu.push(drawn * 1000.0);
                gpu.push(t.elapsed().as_secs_f64() * 1000.0);
                if omsi_cfg::flags::OMSI_BENCH_FRAMES.is_set() {
                    log::info!("bench frame {k}: drawing {:.2} ms, GPU wait {:.2} ms", drawn * 1000.0, gpu[k]);
                }
            }
            let median = |v: &mut Vec<f64>| {
                v.sort_by(|a, b| a.total_cmp(b));
                v.get(v.len() / 2).copied().unwrap_or(0.0)
            };
            let total: Vec<f64> = cpu.iter().zip(&gpu).map(|(a, b)| a + b).collect();
            let worst = total.iter().copied().fold(0.0f64, f64::max);
            let mean = total.iter().sum::<f64>() / total.len().max(1) as f64;
            log::info!(
                "bench: {n} frames of {w}x{h}: drawing {:.2} ms, GPU wait {:.2} ms (medians), frame mean {mean:.2} ms, worst {worst:.2} ms",
                median(&mut cpu),
                median(&mut gpu)
            );
            for (k, v) in renderer.stats.borrow().iter() {
                log::info!("bench stage {k:18}: {:.2} ms/frame", v / n as f64 * 1000.0);
            }
            for (k, v) in renderer.counts.borrow().iter() {
                log::info!("bench count {k}: {:.0} a frame", v / n as f64);
            }
            // (each pass from the end of the one before it; the mirrors' frames on their own)
            for (pass, ms, frames) in renderer.gpu_pass_times() {
                log::info!("bench gpu pass {pass:12}: {ms:.2} ms ({frames} frames measured)");
            }
        }
    }

    /// The textures the picture is drawn with (`OMSI_TEXTURE_MEMORY`), and
    /// `OMSI_WARM_FRAMES`.
    fn final_textures(&mut self, lighting: &omsi_render::Lighting) -> Result<()> {
        let (w, h) = (self.w, self.h);
        let Self {
            ref world,
            ref settings,
            ref mut renderer,
            ref mut scene,
            ref camera,
            ..
        } = *self;
        // the textures compressed on the workers are in the picture, as in a window after its
        // first seconds; OMSI_TEXTURE_MEMORY=<MB> applies a texture budget first
        // (OMSI_BUDGET_FROM=x,y[,MB]: the budget is met from there first, as if the camera had
        // been there, then - with the budget raised to MB - the textures that come near again
        // with the camera are read back)
        if omsi_cfg::flags::OMSI_TEXTURE_MEMORY.is_set() {
            world.set_texture_budget(texture_budget(settings));
            let from: Vec<f64> = omsi_cfg::flags::OMSI_BUDGET_FROM.var()
                .unwrap_or_default()
                .split(',')
                .filter_map(|v| v.trim().parse::<f64>().ok())
                .collect();
            if from.len() >= 2 {
                let at = [DVec3::new(from[0], from[1], 0.0)];
                while world.update_texture_budget(renderer, scene, &at, true) > 0 {}
                if let Some(mb) = from.get(2) {
                    world.set_texture_budget((*mb * 1e6) as u64);
                }
            }
            let centers = [camera.position];
            while world.update_texture_budget(renderer, scene, &centers, true) > 0 {
                world.finish_texture_upgrades(renderer, scene);
            }
        }
        world.finish_texture_upgrades(renderer, scene);
        // OMSI_WARM_FRAMES=n: n frames drawn before the picture, for what reads the frame
        // before it (the rain on the glass looks through the last picture)
        for _ in 0..omsi_cfg::flags::OMSI_WARM_FRAMES.parse::<usize>().unwrap_or(0) {
            let _ = renderer.render_to_image(scene, w, h, camera, lighting)?;
        }
        Ok(())
    }

    /// The mirrors, the mirror panels and the picture, written to `--out`.
    fn final_render(&mut self, lighting: &omsi_render::Lighting) -> Result<()> {
        let (out, w, h) = (self.out, self.w, self.h);
        let Self {
            ref mut renderer,
            ref mut scene,
            ref world,
            ref player_ref,
            ref settings,
            ref camera,
            ..
        } = *self;
        let t0 = Instant::now();
        if let Some(p) = player_ref.as_ref() {
            render_mirrors(renderer, scene, world, p, lighting, None, None);
        }
        if let Some(p) = player_ref.as_ref() {
            let mode = omsi_cfg::flags::OMSI_MIRROR_HUD.parse::<u8>().unwrap_or(settings.mirror_hud);
            let mut panels = crate::mirror_hud::MirrorHud::default();
            steps::mirror_hud_sync(&mut panels, world, p, mode);
            if mode != 0 {
                panels.enabled = true;
                if panels.panels.is_empty() {
                    panels.toggle_edit(p);
                    panels.toggle_edit(p);
                }
            }
            let viewport = settings.hud_viewport((w, h));
            steps::push_mirror_hud(&panels, scene, world, viewport, (0.0, 0.0));
        }
        let pixels = if settings.triple.enabled && !settings.vr_requested() {
            renderer.render_triple_to_image(scene, w, h, camera, lighting, &settings.triple)?
        } else {
            renderer.render_to_image(scene, w, h, camera, lighting)?
        };
        log::info!(
            "rendered {} instances in {:.1} ms; GPU memory: textures {:.0} MB, meshes {:.0} MB ({} meshes, {} textures)",
            scene.instances.len(),
            t0.elapsed().as_secs_f32() * 1000.0,
            renderer.texture_bytes(scene) as f64 / 1e6,
            renderer.mesh_bytes(scene) as f64 / 1e6,
            scene.meshes.len(),
            scene.textures.len()
        );
        image::save_buffer(out, &pixels, w, h, image::ColorType::Rgba8)?;
        println!("wrote {}", out.display());
        Ok(())
    }
}
