//! The clock, the weather, the lights, the rain and the scenery's scripts in the window's
//! frame.

use super::*;

impl App {
    /// The METAR sync, the clock and the weather; the street lamps and the lit windows by the
    /// daylight, which the bus's scripts are told. The frame's daylight.
    pub(super) fn frame_weather(&mut self, dt: f32) -> omsi_sim::Daylight {
        // (the METAR sync: the report's weather, in real time)
        self.tick_metar(dt);
        if !self.paused {
            // (the time speed: the settings', or the session's in LAN play)
            let speed = self.time_speed();
            self.clock.advance(dt * speed as f32);
            // (the real-time sync: the device's date and time, whatever the speed was)
            self.sync_real_time();
            if let Some(t) = self.session.traffic.as_mut() {
                t.time_scale = speed;
            }
            self.tick_weather(dt * speed as f32);
        } else if self.session.weather_blend.is_some() {
            // (a preset picked in the paused menu: the change goes over in real time)
            self.tick_weather(dt);
        }
        let daylight = omsi_sim::Daylight::compute(&self.clock, self.session.envir.as_ref());
        let lamps = self.session.lamps_on != Some(daylight.lamps_on);
        self.session.lamps_on = Some(daylight.lamps_on);
        // the lit windows of the houses by their [NightMapMode] timetable (once a
        // second: tiles come and go, and the hours pass)
        let night_modes = self.perf.total_frames % 60 == 0;
        if let (Some(w), Some(r), Some(scene)) = (
            self.world.as_ref(),
            self.renderer.as_ref(),
            self.scene.as_mut(),
        ) {
            steps::world_lamps(w, r, scene, &self.clock, &daylight, lamps, night_modes);
        }
        if night_modes {
            self.follow_date();
        }
        if let Some(p) = self.player.as_mut() {
            steps::tell_surroundings(p, self.world.as_deref(), &daylight, self.session.weather.as_ref(), self.session.wetness);
        }
        daylight
    }

    /// The lights (the lamps' cones, the light maps, every light that shines), the rain and
    /// the snow, the cabin air, the tyres' spray and the sounds of the street.
    pub(super) fn frame_lights(&mut self, dt: f32, daylight: omsi_sim::Daylight) {
        let __t = Instant::now();
        // the lamps' cones in fog and falling rain or snow, and new light pictures
        if let Some(wt) = &self.session.weather {
            lights::set_cone_strength(wt.fog.0, precip_of(wt).1, daylight.night);
        }
        if let Some(r) = self.renderer.as_mut() {
            lights::upload_corona_textures(r);
        }
        // the tile light maps around the camera, for the roads' night light
        let __ta = Instant::now();
        if let (Some(w), Some(r), Some(cam)) = (self.world.as_ref(), self.renderer.as_ref(), self.camera.as_ref()) {
            w.update_light_map_atlas(r, cam.position);
        }
        *self.perf.profile.entry("lights.atlas").or_default() += __ta.elapsed().as_secs_f64();
        // the snow on the roads: how far it has built up, and the ruts and the tyres'
        // tracks around the camera (road_snow.wgsl)
        if crate::road_snow::enabled() {
            let __tr = Instant::now();
            if let Some(wt) = &self.session.weather {
                self.session.road_snow.step(if self.paused { 0.0 } else { dt * self.time_speed() as f32 }, wt);
            }
            if let (Some(r), Some(cam)) = (self.renderer.as_ref(), self.camera.as_ref()) {
                let tyres = crate::road_snow::tyres(self.player.as_ref(), self.session.traffic.as_ref(), &self.net.remotes);
                let net = self.session.traffic.as_ref().map(|t| &t.net);
                let (cover, fallen) = (self.session.road_snow.cover, self.session.road_snow.fallen);
                self.session.snow_tracks.update(r, cam.position, net, &tyres, cover, fallen);
            }
            // the snow on the roofs of the player's bus and of the traffic
            {
                let vehicles = crate::road_snow::roof_vehicles(self.player.as_ref(), self.session.traffic.as_ref());
                let (cover, fallen) = (self.session.road_snow.cover, self.session.road_snow.fallen);
                self.session.road_snow.roofs.step(&vehicles, cover, fallen);
            }
            if let (Some(r), Some(scene)) = (self.renderer.as_ref(), self.scene.as_mut()) {
                crate::road_snow::show_roofs(r, scene, &self.session.road_snow.roofs, self.player.as_ref(), self.session.traffic.as_ref(), &self.gfx.sim_view.traffic);
            }
            *self.perf.profile.entry("lights.road_snow").or_default() += __tr.elapsed().as_secs_f64();
        }
        if let (Some(w), Some(scene), Some(cam)) = (
            self.world.as_ref(),
            self.scene.as_mut(),
            self.camera.as_ref(),
        ) {
            let vehicles = steps::light_vehicles(self.player.as_ref(), self.session.traffic.as_ref(), &self.net.remotes);
            let __tc = Instant::now();
            lights::collect(w, scene, &daylight, cam.position, &vehicles);
            *self.perf.profile.entry("lights.collect").or_default() += __tc.elapsed().as_secs_f64();
            // the object editor's pick: a magenta glow over it
            if let Some(id) = self.menus.editor.as_ref().and_then(|e| e.selected) {
                let at = w.edit_objects.lock().get(&id).map(|o| o.pos);
                let moved = w.object_edits.lock().get(&id).map(|e| e.moved).unwrap_or_default();
                if let Some(p) = at {
                    scene.coronas.push(omsi_render::Corona {
                        position: p + moved + glam::DVec3::Z * 3.0,
                        size: 0.6,
                        color: [1.0, 0.1, 0.9],
                        brightness: 2.0,
                        ..Default::default()
                    });
                }
            }
            if let Some(wt) = &self.session.weather {
                let (kind, rate) = precip_of(wt);
                self.session.rain.set(kind, rate);
                // [wind] direction (deg) speed (m/s)
                let wind = crate::rain::weather_wind(wt);
                let spray_wind = steps::spray_wind(wt);
                // every bus one may ride in keeps the weather out: the own, another
                // player's, a timetable bus - each part of it: an articulated bus's
                // rear section is a coupled part with its own [boundingbox] (#777)
                let boxed = crate::rain::vehicle_boxes;
                let mut buses: Vec<(glam::DVec3, f64, [f32; 6])> = self.player.as_ref().map(|p| boxed(&p.vehicle)).unwrap_or_default();
                buses.extend(self.net.remotes.remotes.values().flat_map(|rv| boxed(rv.vehicle())));
                if let Some(t) = self.session.traffic.as_ref() {
                    buses.extend(t.cars.iter().filter(|c| c.is_bus() && (c.vehicle.position - cam.position).length() < 40.0).flat_map(|c| boxed(&c.vehicle)));
                }
                let __tr = Instant::now();
                self.session.rain.tick(if self.paused { 0.0 } else { dt }, cam.position, wind, scene, &buses);
                // the player's bus's cabin air and the condensation on its glass
                if let Some(p) = self.player.as_ref() {
                    steps::cabin_air_step(&mut self.session.cabin_air, if self.paused { 0.0 } else { dt }, p, wt, self.session.humans.as_ref());
                }
                *self.perf.profile.entry("lights.rain").or_default() += __tr.elapsed().as_secs_f64();
                // what every vehicle's tyres throw up from the water on the road: the
                // puddles and the wet asphalt the renderer draws (the same wetness:
                // none under snow, OMSI_WETNESS as the picture takes it)
                let wetness = puddles::road_wetness(self.session.wetness, wt.snow);
                if (wetness > 0.0 || !self.session.spray.is_empty()) && !omsi_cfg::flags::OMSI_NO_SPRAY.is_set() {
                    let __ts = Instant::now();
                    steps::throw_spray(
                        &mut self.session.spray,
                        if self.paused { 0.0 } else { dt },
                        self.player.as_ref(),
                        self.session.traffic.as_ref(),
                        &self.net.remotes,
                        cam.position,
                        spray_wind,
                        w,
                        wetness,
                    );
                    self.session.spray.sprites(cam.position, &mut scene.smoke);
                    *self.perf.profile.entry("lights.spray").or_default() += __ts.elapsed().as_secs_f64();
                }
                // the rain heard in the street and the footsteps on the pavement
                if let (Some(amb), Some(a)) = (self.sound.ambience.as_mut(), self.sound.audio.as_ref())
                {
                    let steps = self
                        .session.humans
                        .as_mut()
                        .map(|h| h.take_footfalls())
                        .unwrap_or_default();
                    // what the passengers say, where they stand
                    for line in self.session.humans.as_mut().map(|h| h.take_voice_lines()).unwrap_or_default() {
                        if let Some(clip) = a.load_clip(&line.path) {
                            a.play(
                                clip,
                                omsi_audio::mixer::VoiceParams {
                                    gain: 1.0,
                                    pitch: 1.0,
                                    looping: false,
                                    position: Some(line.position.as_vec3()),
                                    doppler: true,
                                    range: 3.0,
                                    lowpass_hz: 0.0,
                                    important: false,
                                },
                            );
                        }
                    }
                    let inside = self.cam.in_cab;
                    let __tm = Instant::now();
                    amb.update(
                        a,
                        dt,
                        (kind, rate),
                        inside,
                        street_condition(wt, self.session.wetness),
                        cam.position,
                        &steps,
                    );
                    *self.perf.profile.entry("lights.ambience").or_default() += __tm.elapsed().as_secs_f64();
                    if let Some(every) = debug_sound_every() {
                        static LAST: std::sync::atomic::AtomicU32 =
                            std::sync::atomic::AtomicU32::new(u32::MAX);
                        let bucket = (self.clock.time / every as f64) as u32;
                        if LAST.swap(bucket, std::sync::atomic::Ordering::Relaxed) != bucket
                        {
                            log::info!("sound: environment - {} (precip {kind} {rate:.2}, StreetCond {:.2}, {} voices)", amb.last, street_condition(wt, self.session.wetness), a.voice_count());
                        }
                    }
                }
            }
        }
        *self.perf.profile.entry("lights+rain").or_default() += __t.elapsed().as_secs_f64();
    }

    /// The departure boards, the map's route arrows and the scenery's scripts.
    pub(super) fn frame_scripted(&mut self, dt: f32, daylight: omsi_sim::Daylight) {
        let __t = Instant::now();
        if let (Some(w), Some(r), Some(scene), Some(cam)) = (
            self.world.as_ref(),
            self.renderer.as_ref(),
            self.scene.as_mut(),
            self.camera.as_ref(),
        ) {
            let traffic = self.session.traffic.as_ref();
            let phase = |c: usize, li: usize| {
                traffic.map(|t| t.light_vars(c, li)).unwrap_or((omsi_sim::traffic::UNLINKED_PHASE as f32, 0.0))
            };
            let __tb = Instant::now();
            if let Some(p) = self.player.as_mut() {
                w.sync_html_departures(&mut p.vehicle.host);
            }
            steps::departure_boards(
                self.session.schedule.as_mut(),
                w,
                traffic,
                self.session.duty.as_ref(),
                self.player
                    .as_ref()
                    .and_then(|p| p.vehicle.host.hof.as_deref()),
                &self.clock,
            );
            *self.perf.profile.entry("scripted.boards").or_default() += __tb.elapsed().as_secs_f64();
            // the map's own route arrows, with OMSI 2's route arrows
            w.show_help_arrows(r, scene, self.settings.nav_arrows);
            w.update_scripted(
                r,
                scene,
                dt,
                cam.position,
                daylight.brightness,
                &phase,
                self.sound.audio.as_ref(),
                self.cam.in_cab,
            );
        }
        *self.perf.profile.entry("scripted").or_default() += __t.elapsed().as_secs_f64();
    }
}
