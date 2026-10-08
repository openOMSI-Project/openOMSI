//! The LAN, the people and the player's duty in the window's frame.

use super::*;

impl App {
    /// The LAN session, the player on foot and the other players walking about, and the
    /// people at the stops and in the buses.
    pub(super) fn frame_people(&mut self, dt: f32) {
        let __t = Instant::now();
        self.tick_lan(dt);
        // (a stage of its own: a joining player's bus is loaded here, and that frame
        // was counted as the people's)
        *self.perf.profile.entry("lan").or_default() += __t.elapsed().as_secs_f64();
        // the player on foot, and the other players walking about
        self.tick_on_foot(if self.paused { 0.0 } else { dt });
        self.sync_remote_walkers();
        let __t = Instant::now();
        let sight = self.camera.as_ref().zip(self.gfx.surface.as_ref())
            .and_then(|(c, s)| self.sight_extent(c, (s.config.width, s.config.height)));
        if let (Some(h), Some(w), Some(r), Some(scene)) = (
            self.session.humans.as_mut(),
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
            steps::humans_due_trips(h, self.session.schedule.as_ref(), self.session.traffic.as_ref());
            if h.stop_targets.is_none() {
                h.stop_targets = self.session.schedule.as_ref().map(|s| s.stop_targets());
                h.stop_names = self.session.schedule.as_ref().map(|s| s.stop_names());
                if let Some(t) = &h.stop_targets {
                    log::info!("people: {} bus stops with timetable targets", t.len());
                }
            }
            // (whom the player's bus takes on: by the duty, or in free drive by its terminus)
            h.set_duty(self.session.duty.as_ref());
            // (the riders leave a bus the driver has walked away from)
            h.driver_away = self.session.on_foot.as_ref().is_some_and(|f| {
                let own = Some(crate::humans::BusId::Player);
                f.seat.map(|s| s.0) != own && f.inside.map(|i| i.0) != own
            });
            // (OMSI's `AIPassFactor`, the passengers setting in per cent)
            steps::humans_by_hour(h, w, self.clock.time, self.settings.pax_density, self.session.duty.as_ref());
            self.session.humans_populate_t -= dt;
            if self.session.humans_populate_t <= 0.0 && !self.paused {
                self.session.humans_populate_t = 2.0;
                h.populate(&mut self.gfx.sim_view.people, w, r, scene, center);
            }
            if let (Some(cam), Some(s)) = (self.camera.as_ref(), self.gfx.surface.as_ref()) {
                h.eye = Some(humans::Eye::of(
                    cam,
                    s.config.width as f32 / s.config.height.max(1) as f32,
                ).widened(sight));
            }
            // (the other LAN players' buses, for their riders to sit in)
            h.set_remote_buses(self.net.remotes.remotes.iter().map(|(id, r)| (*id, r.vehicle())));
            // (and the vehicles the player placed and left, with their riders)
            h.set_placed_buses(self.session.placed.iter().map(|q| (q.uid, &q.vehicle)));
            h.set_player_next_stop(
                self.session.duty
                    .as_ref()
                    .and_then(|d| d.trip().stops.get(d.next_stop)),
            );
            let took = steps::tick_humans(
                h,
                &mut self.gfx.sim_view.people,
                if self.paused { 0.0 } else { dt },
                w,
                self.player.as_mut(),
                self.session.traffic.as_mut(),
                r,
                scene,
            );
            if let Some(m) = h.take_message() {
                self.service_msg = Some((m, 6.0));
            }
            if let Some(p) = self.player.as_mut() {
                if took {
                    p.vehicle.set_var("GivenTicket", -1.0);
                }
                h.give_ticket = std::mem::take(&mut p.give_ticket);
                h.give_change_all = std::mem::take(&mut p.give_change);
                if std::mem::take(&mut p.take_change) {
                    h.take_change_tray();
                }
                if std::mem::take(&mut h.stop_request) {
                    // a passenger's request is the vehicle trigger Omsi.exe fires
                    // (0x62e42c), not the cab's stop button `door_haltewunsch`,
                    // whose switch and brake sounds some buses play
                    p.vehicle.trigger("int_haltewunsch");
                }
                h.write_pax_vars(&mut p.vehicle);
                p.vehicle.host.humans_on_path_link = h.path_link_counts();
                p.vehicle.host.humans_on_seat = h.seat_counts();
                let coins: Vec<usize> = std::mem::take(&mut p.vehicle.host.change_coins);
                h.give_change(w, r, scene, &coins);
            }
            // the view sync of the people: the coins and ticket blocks of the player's bus,
            // then everybody's pose (see `view_sync`)
            let bus = self.player.as_ref().map(|p| &p.vehicle);
            view_sync::sync(ViewSync::people(h, bus, center), &mut self.gfx.sim_view, w, r, scene);
        }
        *self.perf.profile.entry("humans").or_default() += __t.elapsed().as_secs_f64();
        self.foot_after_humans();
        if !self.paused {
            self.tick_service(dt);
        }
    }

    /// The player's duty, the personnel file's step, the placing of vehicles, the host's map
    /// edits and `--on-foot`.
    pub(super) fn frame_duty(&mut self, dt: f32) {
        if self.session.duty.is_none() {
            self.session.career.no_trip();
        }
        if let (Some(d), Some(p), Some(w), false) = (
            self.session.duty.as_mut(),
            self.player.as_mut(),
            self.world.as_ref(),
            self.paused,
        ) {
            steps::duty_step(
                d,
                p,
                w,
                &mut self.session.career,
                &mut self.session.journey,
                &self.args.root,
                self.clock.time,
                Some(&self.clock),
                true,
                Some(&mut self.integrations.plugin_events),
            );
        }
        if let Some(p) = self.player.as_mut() {
            let riders = self.session.humans.as_ref().map(|h| h.riding()).unwrap_or(0);
            // (a frame after the session was written must not start another one)
            let tick = !self.exiting && !self.paused;
            let crash = steps::career_step(&mut self.session.career, p, riders, self.session.duty.is_some(), dt, tick);
            if crash > 0.0 {
                self.service_msg = Some((format!("Crash: {:.0} kJ", crash / 1000.0), 6.0));
                use omsi_plugin::InfoValue::Num;
                let args = vec![Num(crash as f64 / 1000.0), Num(p.vehicle.physics.velocity_kmh().abs() as f64)];
                crate::plugins::queue_event(&mut self.integrations.plugin_events, "crash", args);
            }
        }
        for j in self.session.career.take_jolts() {
            use crate::plugins::num_f32;
            let args = vec![num_f32(j.along), num_f32(j.across), num_f32(j.speed_kmh), omsi_plugin::InfoValue::Num(j.riders as f64)];
            crate::plugins::queue_event(&mut self.integrations.plugin_events, "jolt", args);
        }
        if !self.paused {
            crate::admin::guard_fall(self, dt);
        }
        self.placing_frame();
        // the host sends every edit of the map again now and then (players join)
        if self.net.lan.as_ref().map(|l| l.role == omsi_net::Role::Host).unwrap_or(false) {
            self.menus.editor_sync_t -= dt;
            if self.menus.editor_sync_t <= 0.0 {
                self.menus.editor_sync_t = 10.0;
                self.editor_broadcast(true);
            }
        }
        // --on-foot: the bus the start put down goes, the player stands beside it
        if self.args.on_foot && self.world.is_some() {
            self.args.on_foot = false;
            if self.player.is_some() {
                self.remove_driven_vehicle();
            } else if let Some(c) = self.camera.as_ref() {
                // (no bus came: where the camera stands, on the ground)
                let p = c.position;
                let z = self.world.as_ref().and_then(|w| w.walk_height(p.x, p.y)).unwrap_or(p.z - 1.7);
                let yaw = c.yaw as f64;
                self.start_on_foot(glam::DVec3::new(p.x, p.y, z), yaw);
            }
            self.service_msg = Some(("On foot: Esc menu, Place a vehicle..., then G at its driver's door to drive it".into(), 8.0));
        }
    }
}
