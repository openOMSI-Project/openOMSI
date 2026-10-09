//! The AI traffic's step in the window's frame.

use super::*;

impl App {
    /// The AI traffic: what it needs to know, the timetable's departures, its step, its
    /// sound and its pictures.
    pub(super) fn frame_traffic(&mut self, dt: f32) {
        let __t = Instant::now();
        // (with a triple screen, all of its three panels are in sight)
        let sight = self.camera.as_ref().zip(self.gfx.surface.as_ref())
            .and_then(|(c, s)| self.sight_extent(c, (s.config.width, s.config.height)));
        if let (Some(t), Some(w), Some(r), Some(scene)) = (
            self.session.traffic.as_mut(),
            self.world.as_ref(),
            self.renderer.as_ref(),
            self.scene.as_mut(),
        ) {
            let center = self
                .player
                .as_ref()
                .map(|p| p.vehicle.position)
                .or(self.camera.as_ref().map(|c| c.position))
                .unwrap_or(DVec3::ZERO);
            let aspect = self
                .gfx.surface
                .as_ref()
                .map(|s| s.config.width as f64 / s.config.height.max(1) as f64)
                .unwrap_or(16.0 / 9.0);
            let fog = self
                .session.weather
                .as_ref()
                .map(|w| w.fog.0 as f64)
                .unwrap_or(50000.0);
            traffic_inputs(
                t,
                self.camera.as_ref(),
                aspect,
                sight,
                fog,
                &self.clock,
                self.session.humans.as_ref(),
                self.player.as_ref(),
                &r.options,
            );
            self.session.populate_t -= dt;
            if self.session.populate_t <= 0.0 && !self.paused {
                // come back quickly while there is a backlog of departures to put out
                self.session.populate_t = if self
                    .session.schedule
                    .as_ref()
                    .map(|s| s.pending() > 0)
                    .unwrap_or(false)
                {
                    0.1
                } else {
                    2.0
                };
                let __t5 = Instant::now();
                let view = self.camera.as_ref().map(|c| c.forward().as_dvec3());
                t.populate_seen(&mut self.gfx.sim_view.traffic, w, r, scene, center, view);
                *self.perf.profile.entry("traffic.populate").or_default() +=
                    __t5.elapsed().as_secs_f64();
                steps::set_keep_clear(t, self.player.as_ref(), &self.net.remotes);
                if let Some(s) = self.session.schedule.as_mut() {
                    let __t6 = Instant::now();
                    steps::schedule_tick(s, w, t, &mut self.gfx.sim_view.traffic, r, scene, self.session.first_populate);
                    *self.perf.profile.entry("traffic.schedule").or_default() +=
                        __t6.elapsed().as_secs_f64();
                }
                self.session.first_populate = false;
            }
            // the AI's lights (and a bus's saloon lamps, which its scripts switch with
            // them): by the time of day, and by day in fog, rain, snow or under a
            // closed cloud cover as drivers do
            let gloomy = steps::gloomy_weather(self.session.weather.as_ref());
            // Omsi switches the AI's lights on below a light value of 0.75, before
            // the street lamps (0.6), and off after them in the morning
            let daylight = omsi_sim::Daylight::compute(&self.clock, self.session.envir.as_ref());
            steps::set_ai_daylight(t, daylight, gloomy);
            let __t2 = Instant::now();
            let rail = self.player.as_ref().and_then(|p| p.rail.as_ref()).map(|r| (r.lane, r.along));
            steps::traffic_tick(t, &mut self.gfx.sim_view.traffic, w, dt, self.paused, self.player.as_ref(), &self.net.remotes, &self.session.placed, rail);
            *self.perf.profile.entry("traffic.tick").or_default() +=
                __t2.elapsed().as_secs_f64();
            for (k, v) in ["traffic.tick.lanes", "traffic.tick.plan", "traffic.tick.ai"]
                .into_iter()
                .zip(t.tick_split)
            {
                *self.perf.profile.entry(k).or_default() += v;
            }
            // the options' [no_collision_vehToVeh]: the bus drives through the traffic
            steps::traffic_boxes(t, self.player.as_mut(), self.settings.collision_vehicles);
            let __t3 = Instant::now();
            if let Some(a) = self.sound.audio.as_ref() {
                let street = self
                    .session.weather
                    .as_ref()
                    .map(|w| street_condition(w, self.session.wetness))
                    .unwrap_or(0.0);
                let muffled = self.cam.in_cab;
                // heard round the camera (the ear), not round the player's bus: a
                // free camera following an AI bus lost its sound 250 m from the bus
                let ear = self.camera.as_ref().map(|c| c.position).unwrap_or(center);
                // (riding in an AI bus on foot: that bus is heard from inside, #1286)
                let riding = self.session.on_foot.as_ref().and_then(|f| match f.inside {
                    Some((omsi_sim::people::BusId::Ai(id), _)) => Some(id),
                    _ => None,
                });
                t.update_audio(a, ear, street, muffled || riding.is_some(), riding);
            }
            *self.perf.profile.entry("traffic.audio").or_default() +=
                __t3.elapsed().as_secs_f64();
            let __t4 = Instant::now();
            t.camera = self.camera.as_ref().map(|c| c.position);
            // the view sync of the traffic: before the player and the people move, as the
            // cars that parked leave the traffic here (see `view_sync`)
            view_sync::sync(ViewSync::traffic(t), &mut self.gfx.sim_view, w, r, scene);
            *self.perf.profile.entry("traffic.sync").or_default() +=
                __t4.elapsed().as_secs_f64();
        }
        *self.perf.profile.entry("traffic").or_default() += __t.elapsed().as_secs_f64();
    }
}
