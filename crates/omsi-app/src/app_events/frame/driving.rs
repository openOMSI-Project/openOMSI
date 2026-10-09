//! The player's bus in the window's frame: its step, the view from it, the sound round the
//! camera.

use super::*;

impl App {
    /// The player's bus: its step and its pictures, the view from it, the cursor's aim into
    /// the cab.
    pub(super) fn frame_player(&mut self, dt: f32) {
        // the on-screen wheel and pedals (a phone)
        self.touch_frame(dt);
        if self.player.is_none() || self.renderer.is_none() || self.scene.is_none() {
            return;
        }
        self.frame_player_physics(dt);
        self.frame_view_camera(dt);
        let __th = Instant::now();
        // (the cursor's aim into the cab: again when the cursor or the view
        // turned, else every few frames for switches that moved under it - a ray
        // through every cockpit mesh every frame was a tenth of the frame)
        let key = self.camera.as_ref().map(|c| (self.input.cursor.0.round() as i32, self.input.cursor.1.round() as i32, (c.yaw * 4.0).round() as i32, (c.pitch * 4.0).round() as i32));
        // (the cab sways with the suspension: a view that only turned waits a few frames)
        let cursor_moved = key.map(|k| (k.0, k.1)) != self.menus.hover_key.map(|k| (k.0, k.1));
        if cursor_moved || (key != self.menus.hover_key && self.perf.total_frames % 6 == 0) || self.perf.total_frames % 12 == 0 {
            self.menus.hover_key = key;
            self.update_hover();
        }
        *self.perf.profile.entry("player.hover").or_default() +=
            __th.elapsed().as_secs_f64();
    }

    /// The bus's step (scripts, physics, the head), the situation's other vehicles, and the
    /// bus and its driver into the scene.
    fn frame_player_physics(&mut self, dt: f32) {
        let (Some(p), Some(r), Some(scene)) = (
            self.player.as_mut(),
            self.renderer.as_ref(),
            self.scene.as_mut(),
        ) else {
            return;
        };
        // (while the tile under the bus is being read again - the weather turned
        // to snow and every tile came back with the winter textures - there is
        // no ground under it: it is held where it is rather than falling through
        // the world and being put back somewhere in the sky)
        // (the tile's wheel surfaces - its roads, bridges and yards - not its
        // terrain alone: the terrain comes first while the tile is placed, and a
        // parked bus taken over after a restart fell through its yard onto the
        // ground below before the surfaces came, as Omsi.exe holds it, #1279)
        let ground_here = self.world.as_ref().is_none_or(|w| {
            let at = p.vehicle.position;
            let k = ((at.x / omsi_map::tile_size()).floor() as i32, (at.y / omsi_map::tile_size()).floor() as i32);
            w.surfaces.read().contains_key(&k)
        });
        if !self.paused && ground_here {
            p.tick(
                dt,
                self.sound.audio.as_ref(),
                self.cam.in_cab,
                !matches!(self.view.as_str(), "free" | "foot"),
            );
            crate::plugins::plugin_impacts(&p.vehicle, &mut self.integrations);
            steps::deliver_player_impacts(p, self.session.traffic.as_mut());
            // After scripts: zero-movement `_drag` for a held switch. Running this
            // *before* `tick` cleared Aachen ibox momentary flags (incl. digit 0 /
            // `ibox_taste_D11`) before the frame could act when the click path had
            // not already consumed them (#744).
            if self.input.dragging {
                let (dx, dy) = std::mem::take(&mut self.input.drag_delta);
                if let Some((map_id, ref ev)) = self.input.pressed_scenery_object {
                    if let Some(w) = self.world.as_ref() {
                        w.scenery_object_drag(map_id, ev, dx, dy);
                    }
                } else {
                    p.drag(dx, dy);
                }
            }
            // (not in the headset: the player's own head moves there, and a head
            // thrown about by the bus on top of it made the whole cab sway and
            // shift before the eyes)
            #[cfg(windows)]
            let vr_on = self.xr.vr.is_some();
            #[cfg(not(windows))]
            let vr_on = false;
            p.move_head(dt, self.settings.head_movement && !vr_on);
            // (a head that is doing nothing still breathes and shifts its weight:
            // the sway goes on the head and the view while the bus waits, never
            // into the springs above. Nothing of it while a headset or a real
            // head tracker moves the head - that head is not a still one)
            let idle = if vr_on || (self.settings.head_tracking && self.input.headtrack.is_some()) { 0.0 } else { self.settings.head_idle };
            // (at a standstill, as the setting says: it fades out over the first
            // few km/h as the bus pulls away and comes back when it stands - it
            // swayed on the road as well, #1325)
            let idle = idle * crate::head_idle::standstill(p.vehicle.physics.velocity_kmh().abs());
            // (a switch under the cursor is a hand reaching for it, and a view that
            // goes on sliding under the pointer is a view that misses what it was
            // reaching for. Held, not reset: the camera stays where it is, which is
            // where any camera is while the player is busy with something)
            let reaching = idle > 0.0 && (self.menus.hover.is_some() || self.menus.hover_hand);
            if !self.cam.head_idle_hold.step(dt, reaching) {
                p.move_head_idle(dt, idle, self.settings.head_idle_pace);
            }
            if let Some(w) = self.world.as_ref() {
                crate::rail_drive::frame(p, self.session.traffic.as_ref().map(|t| &t.net), w, dt);
            }
        } else if self.input.dragging {
            // paused / no ground: still deliver held-switch `_drag` (was unconditional before)
            let (dx, dy) = std::mem::take(&mut self.input.drag_delta);
            if let Some((map_id, ref ev)) = self.input.pressed_scenery_object {
                if let Some(w) = self.world.as_ref() {
                    w.scenery_object_drag(map_id, ev, dx, dy);
                }
            } else {
                p.drag(dx, dy);
            }
        }
        // a script that set the time of day (`(S.S.Time)`) moves the game's clock
        if let Some(t) = p.vehicle.host.time_written.take() {
            self.session.pending_time = Some(t);
        }
        // the situation's further vehicles stand and run their scripts, and the
        // player's bus meets them
        let mut placed_boxes = Vec::new();
        for q in self.session.placed.iter_mut() {
            if !self.paused {
                q.vehicle.update(dt);
            }
            q.sync_transforms(r, scene, false);
            let f = crate::lan::footprint_of(&q.vehicle, [2.5, 11.5, 3.0, 0.0, 0.0, 1.5]);
            placed_boxes.push(omsi_sim::collision::Obb {
                center: glam::DVec2::new(f.x, f.y),
                half: glam::DVec2::new(f.width as f64 * 0.5, f.length as f64 * 0.5),
                heading: (f.heading as f64).to_radians(),
                z0: f.z,
                z1: f.z + 3.0,
                velocity: glam::DVec2::ZERO,
                mass: 12_000.0,
                pole: None,
                id: -1,
            });
        }
        if !self.session.placed.is_empty() {
            // (the traffic writes the list afresh every frame; without it, this does)
            if self.session.traffic.is_none() {
                p.vehicle.dynamic_boxes.clear();
            }
            p.vehicle.dynamic_boxes.extend(placed_boxes);
        }
        if let Some(w) = self.world.as_ref() {
            lay_down_poles(w, r, scene, &mut p.vehicle);
        }
        static EVERY: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
        if let Some(every) = *EVERY.get_or_init(|| {
            omsi_cfg::flags::OMSI_DEBUG_PHYSICS.parse::<f32>()
                .filter(|v| *v > 0.0)
        }) {
            static LAST: std::sync::atomic::AtomicU32 =
                std::sync::atomic::AtomicU32::new(u32::MAX);
            let t = self.started.elapsed().as_secs_f32();
            let bucket = (t / every) as u32;
            if LAST.swap(bucket, std::sync::atomic::Ordering::Relaxed) != bucket {
                log_physics(&p.vehicle, t);
            }
        }
        let inside = self.cam.in_cab;
        p.sync_transforms(r, scene, inside);
        crate::scene::sync_vehicle_damage(r, scene, &mut p.vehicle, &mut p.render);
        // from the driver's seat the figure stays in the mirrors
        // (from the driver's seat only the mirrors show him)
        // (out of the seat: nobody at the wheel)
        p.sync_driver_hands(r, scene, dt, self.settings.driver && self.session.on_foot.is_none(), self.view == "driver", self.settings.hands_in_cab);
    }

    /// The field of view without a bus, the sound's listener, the placed vehicles without a
    /// bus of one's own, and the bus radio.
    pub(super) fn frame_audio(&mut self, dt: f32) {
        // on foot (or the free camera) without a bus of one's own: the field of view
        // setting and the wheel's zoom, as with one - only the player's frame applied
        // them, so after removing the bus the wheel zoomed nothing (#837)
        if self.player.is_none() && matches!(self.view.as_str(), "free" | "foot") {
            if let Some(cam) = self.camera.as_mut() {
                let base = if self.settings.fov >= 20.0 { self.settings.fov.min(120.0) } else { 60.0 };
                cam.fov_deg = (base * self.cam.view_zoom.get(&self.view).copied().unwrap_or(1.0)).clamp(8.0, 120.0);
            }
        }
        if let Some(a) = self.sound.audio.as_ref() {
            a.follow_device();
        }
        if let (Some(a), Some(cam)) = (self.sound.audio.as_ref(), self.camera.as_ref()) {
            let (reverb_time, reverb_mix) = self.world.as_ref().map(|w| w.reverb_at(cam.position)).unwrap_or((0.0, 0.0));
            a.set_listener(omsi_audio::Listener {
                position: cam.position.as_vec3(),
                forward: cam.forward(),
                right: cam.right(),
                // (silent while paused: the engine's loops would go on)
                // (the settings' volume: it had been 0.6 whatever the slider said)
                master: if self.paused { 0.0 } else { self.settings.volume.clamp(0.0, 1.0) },
                reverb_time,
                reverb_mix,
            });
        }
        // on foot without a bus of one's own: the vehicles one placed still stand, run
        // their scripts and are drawn where they are (the player's frame did it)
        if let (None, Some(r), Some(scene)) = (self.player.as_ref(), self.renderer.as_ref(), self.scene.as_mut()) {
            for q in self.session.placed.iter_mut() {
                if !self.paused {
                    q.vehicle.update(dt);
                }
                q.sync_transforms(r, scene, false);
            }
        }
        if let Some(a) = self.sound.audio.as_ref() {
            match self.player.as_mut() {
                Some(p) => {
                    let inside = self.cam.in_cab;
                    self.sound.radio.set_map(&self.args.root, &self.args.map);
                    if let Some(m) = self.sound.radio.update(a, &p.vehicle, inside) {
                        self.service_msg = Some((m, 6.0));
                    }
                    // (a radio whose display is a text of its script shows the station)
                    p.vehicle.radio_text = self.sound.radio.display_text();
                    p.vehicle.radio_frequency = self.sound.radio.frequency(p.vehicle.position.x, p.vehicle.position.y);
                }
                None => self.sound.radio.stop(a),
            }
        }
    }
}
