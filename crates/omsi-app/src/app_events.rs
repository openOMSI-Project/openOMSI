//! The window's events: winit's `ApplicationHandler` for `App`.

/// Mirror pictures drawn per second at most, all mirrors together (see the redraw).
const MIRROR_RATE: f32 = 75.0;
/// The least a mirror is redrawn a second (see the mirrors in `window_event`).
const MIRROR_MIN_HZ: f32 = 8.0;
const MIRROR_MAX_HZ: f32 = 30.0;

fn render_scale_step(fps: f32, slow_frame_wait_share: f32) -> f32 {
    // (three levels, far apart, and a wide band between going down and up again: every
    // step makes the picture's targets anew - hundreds of MB with MSAA and HDR - and a
    // scale that went up and down by 5 % every two seconds stuttered at each change and
    // filled the card's memory with the old ones until the driver gave up)
    if fps < 40.0 && slow_frame_wait_share >= 0.4 {
        -0.15
    } else if fps > 58.0 || slow_frame_wait_share < 0.2 {
        0.15
    } else {
        0.0
    }
}

use super::*;

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.resumed_impl(event_loop);
    }

    /// A phone put the app into the background: its window's surface goes (made again on
    /// `resumed`), the fingers and the held keys are let go.
    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.surface = None;
        self.touch.drop_gpu();
        self.keys.clear();
        if let Some(p) = self.player.as_mut() {
            p.axes.release_all();
        }
        self.save_last_situation();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.finish_session();
                crate::platform::exit(event_loop);
            }
            WindowEvent::Resized(size) => {
                if let (Some(s), Some(r)) = (self.surface.as_mut(), self.renderer.as_ref()) {
                    s.resize(r, size.width, size.height);
                }
            }
            WindowEvent::Focused(true) => {
                self.window_focused = true;
                if let Some(ctl) = self.controllers.as_mut() {
                    ctl.set_focus(true);
                }
            }
            WindowEvent::Focused(false) => {
                self.window_focused = false;
                if let Some(ctl) = self.controllers.as_mut() {
                    ctl.set_focus(false);
                }
                #[cfg(windows)]
                {
                    self.vr_cursor_physical = None;
                    self.vr_cursor_warp_pending = None;
                }
                // No key-up reaches us for whatever was held when focus left (alt-tab, a
                // click outside the window, an OS dialog popping up): without this, a held
                // modifier got "stuck" and made the next plain key press look like it was
                // held with that modifier - Shift got stuck this way once, and a plain `W`
                // (throttle in the wasd preset) was then read as Shift+W, OMSI's own wiper
                // key, toggling the wipers on every press instead of driving.
                self.keys.clear();
                if let Some(p) = self.player.as_mut() {
                    p.axes.release_all();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // '/' opens the chat's input box wherever the keyboard has it (the key
                // itself is then swallowed by the chat)
                if event.state == ElementState::Pressed
                    && event.text.as_deref() == Some("/")
                    && self.lan.is_some()
                    && !lan::chat_open(&self.remotes)
                {
                    self.remotes.chat.open();
                    if let PhysicalKey::Code(code) = event.physical_key {
                        lan::chat_swallow(&mut self.remotes, code);
                    }
                    return;
                }
                // what is typed into an open LAN chat line (the key itself goes on to on_key)
                if let (Some(text), true, true) = (
                    event.text.as_deref(),
                    event.state == ElementState::Pressed,
                    lan::chat_open(&self.remotes),
                ) {
                    lan::chat_type(&mut self.remotes, text);
                }
                // (the Lua plugins' `key` event; a key held down repeats nothing)
                if let (PhysicalKey::Code(code), false) = (event.physical_key, event.repeat) {
                    if self.plugin_keys.len() < 64 {
                        self.plugin_keys.push((format!("{code:?}"), event.state == ElementState::Pressed));
                    }
                }
                // a phone's back key is Escape (the game menu, out of the city map ...)
                let physical = match event.physical_key {
                    PhysicalKey::Code(KeyCode::BrowserBack) => PhysicalKey::Code(KeyCode::Escape),
                    k => k,
                };
                if let PhysicalKey::Code(code) = physical {
                    self.on_key(
                        event_loop,
                        code,
                        event.state == ElementState::Pressed,
                        event.repeat,
                    );
                }
            }
            // In VR right-click zooms; with mouse steering it first releases the steering.
            // On the desktop it retains OMSI's mouse-look and mouse-steering behaviour.
            WindowEvent::MouseInput {
                state,
                button: winit::event::MouseButton::Right,
                ..
            } => {
                if self.navigator.as_ref().map(|n| n.map_open()).unwrap_or(false) {
                    return;
                }
                if self.vr_active() {
                    #[cfg(windows)]
                    if state == ElementState::Pressed && self.game_menu.is_none()
                        && self.chooser.is_none() {
                        if self.mouse_drive {
                            self.mouse_drive = false;
                            crate::player::keep_wheel(self.player.as_mut());
                            self.reset_vr_pointer();
                            self.service_msg = Some(("Mouse steering off".into(), 3.0));
                        } else {
                            self.vr_zoom_active = !self.vr_zoom_active;
                        }
                    }
                } else {
                    // a right click lets go of the mouse steering, as in OMSI (#162)
                    if state == ElementState::Pressed && self.mouse_drive && self.game_menu.is_none() {
                        self.mouse_drive = false;
                        crate::player::keep_wheel(self.player.as_mut());
                        self.service_msg = Some(("Mouse steering off".into(), 3.0));
                    }
                    self.mouse_look = state == ElementState::Pressed;
                    // (the cursor shows it at once, not with the next look at what is under it)
                    self.update_hover();
                }
            }
            // (the middle button - the wheel pressed - turns the view as well: OMSI's pan)
            WindowEvent::MouseInput {
                state,
                button: winit::event::MouseButton::Middle,
                ..
            } => {
                if self.navigator.as_ref().map(|n| n.map_open()).unwrap_or(false) {
                    return;
                }
                self.mouse_look = state == ElementState::Pressed;
                self.update_hover();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let amount = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                self.wheel(amount);
            }
            WindowEvent::CursorMoved { position, .. } => {
                // (the on-screen controls on a computer, `OMSI_TOUCH=1`: the mouse is a
                // finger on them - from #202)
                if self.touch.enabled {
                    self.finger_move(0, glam::Vec2::new(position.x as f32, position.y as f32));
                }
                #[cfg(windows)]
                let vr_cockpit = self.vr.is_some() && self.game_menu.is_none()
                    && matches!(self.view.as_str(), "driver" | "pax");
                #[cfg(not(windows))]
                let vr_cockpit = false;
                if vr_cockpit && !self.mouse_look && !self.mouse_drive {
                    #[cfg(windows)]
                    self.on_vr_cursor_moved(position.x as f32, position.y as f32);
                } else {
                    self.on_mouse_moved(position.x as f32, position.y as f32);
                }
            }
            WindowEvent::MouseInput {
                state,
                button: winit::event::MouseButton::Left,
                ..
            } => {
                if self.touch.enabled {
                    let p = glam::Vec2::new(self.cursor.0, self.cursor.1);
                    if state == ElementState::Pressed {
                        self.finger_down(event_loop, 0, p);
                    } else {
                        self.finger_up(event_loop, 0, p, false);
                    }
                } else {
                    self.left_button(event_loop, state == ElementState::Pressed);
                }
            }
            // a finger (a phone; see touch.rs)
            WindowEvent::Touch(t) => self.on_touch(event_loop, t),
            WindowEvent::RedrawRequested => {
                #[cfg(windows)]
                self.poll_vr_cursor_position();
                // OMSI's autosave of the last situation: every five minutes of play
                if !self.paused && self.player.is_some() && self.clock.run_time - self.autosave_t >= 300.0 {
                    self.autosave_t = self.clock.run_time;
                    self.save_last_situation();
                }
                // the time of day a script set last frame (the nearer way round the clock)
                if let Some(t) = self.pending_time.take() {
                    let d = (t - self.clock.time + 43_200.0).rem_euclid(86_400.0) - 43_200.0;
                    self.shift_clock(d);
                }
                // the graphics device is gone (a driver reset, an external card unplugged):
                // nothing can be drawn again - end the session the ordinary way, so that the
                // summary, the personnel file and the LAN goodbye are not lost
                // the card ran out of memory: fewer textures (the finest levels of the far
                // ones go), before the driver gives the device up
                if self.renderer.as_ref().is_some_and(|r| r.take_out_of_memory()) {
                    if let Some(w) = self.world.as_ref() {
                        let now = w.texture_budget_bytes();
                        let less = if now == 0 { 600_000_000 } else { (now * 3 / 5).max(300_000_000) };
                        w.set_texture_budget(less);
                        log::warn!("the graphics card ran out of memory: textures kept to {:.0} MB from now on", less as f64 / 1e6);
                    }
                }
                if let Some(why) = self.renderer.as_ref().and_then(|r| r.device_lost()) {
                    if self.restart_after_device_loss() {
                        log::warn!("device lost ({why}): the game goes on in a new start");
                    } else {
                        log::error!("ending the session: the graphics device was lost ({why})");
                    }
                    crate::platform::exit(event_loop);
                    return;
                }
                let desktop_vsync = self.settings.vsync && !self.vr_active();
                if let (Some(surface), Some(renderer)) = (self.surface.as_mut(), self.renderer.as_ref()) {
                    surface.set_vsync(renderer, desktop_vsync);
                }
                // in the own bus's cab: at the wheel, a passenger's view, or sitting in a
                // seat of it after getting up (its inside is drawn and heard from inside)
                self.in_cab = matches!(self.view.as_str(), "driver" | "pax")
                    || (self.view == "foot" && self.foot_bus() == Some(crate::humans::BusId::Player));
                // standing or sitting in another player's bus: that bus is drawn and heard
                // from inside (its interior meshes, not the outside ones over them)
                self.inside_remote = match self.foot_bus() {
                    Some(crate::humans::BusId::Ai(x)) if self.view == "foot" => crate::humans::remote_bus_player(x),
                    _ => None,
                };
                let now = Instant::now();
                let raw_dt = (now - self.last).as_secs_f32();
                self.log_frame(raw_dt);
                let profiling = omsi_cfg::env::var_os("OMSI_PROFILE").is_some();
                let waited: f64 = ["acquire", "present", "gpu"].iter()
                    .map(|&k| self.profile.get(k).copied().unwrap_or(0.0))
                    .sum();
                let wait_this_frame = (waited - self.governor_wait_prev).max(0.0) as f32;
                self.governor_wait_prev = waited;
                if self.total_frames > 60 {
                    if raw_dt > 0.05 {
                        self.spikes += 1;
                        if profiling {
                            // where the slow frame went: the stages that took more than 2 ms,
                            // and what no stage accounts for (waiting for the window, the
                            // system, other processes)
                            let mut parts: Vec<(&'static str, f64)> = self
                                .profile
                                .iter()
                                .map(|(k, v)| {
                                    (*k, v - self.profile_prev.get(k).copied().unwrap_or(0.0))
                                })
                                .collect();
                            let staged: f64 = parts
                                .iter()
                                .filter(|(k, _)| !k.contains('.'))
                                .map(|p| p.1)
                                .sum();
                            parts.retain(|p| p.1 > 0.002);
                            parts.sort_by(|a, b| b.1.total_cmp(&a.1));
                            let list: Vec<String> = parts
                                .iter()
                                .map(|(k, v)| format!("{k} {:.0}", v * 1000.0))
                                .collect();
                            log::info!(
                                "stutter: frame {} took {:.0} ms ({}; outside the stages {:.0} ms)",
                                self.total_frames,
                                raw_dt * 1000.0,
                                list.join(", "),
                                (raw_dt as f64 - staged).max(0.0) * 1000.0
                            );
                        }
                    }
                    self.worst_ms = self.worst_ms.max(raw_dt * 1000.0);
                    // Reduce resolution only when slow frames spend substantial time waiting
                    // for presentation or the GPU. Traffic, scripts and tile work can drop
                    // the frame rate too, but fewer pixels cannot make those stages faster.
                    // Keep the player's chosen scale and explicit fixed-scale override.
                    // A fast V-synced frame can wait for the next refresh without being
                    // GPU-bound. Count presentation wait only on slow frames.
                    if raw_dt > 0.02 {
                        self.governor.2 += wait_this_frame;
                    }
                    self.governor.0 += raw_dt;
                    self.governor.1 += 1;
                    if self.governor.0 >= 5.0 {
                        let fps = self.governor.1 as f32 / self.governor.0;
                        let wait_share = self.governor.2 / self.governor.0;
                        self.governor = (0.0, 0, 0.0);
                        let free = self.settings.render_scale <= 0.0
                            && (self.settings.max_fps == 0 || self.settings.max_fps >= 50)
                            && omsi_cfg::env::var_os("OMSI_FIXED_SCALE").is_none();
                        if let (Some(r), true) = (self.renderer.as_mut(), free) {
                            let s = r.dynamic_scale();
                            let step = render_scale_step(fps, wait_share);
                            r.set_dynamic_scale(s + step);
                            if (r.dynamic_scale() - s).abs() > 1e-3 {
                                log::info!("frame rate {fps:.0} fps (presentation wait {:.0}%): the 3D picture is drawn at {:.0} % of the window now", wait_share * 100.0, r.dynamic_scale() * 100.0);
                                self.governor_low = 0;
                            } else if step < 0.0 {
                                // at the smallest scale and still waiting for the card: after
                                // two such readings the picture gets lighter itself (a weak or
                                // old graphics chip keeps a playable frame rate)
                                self.governor_low += 1;
                                if self.governor_low >= 2 && fps < 30.0 {
                                    self.governor_low = 0;
                                    // (SSAO first, then the shadows; for this drive only)
                                    let what = r.lighten().or_else(|| {
                                        std::mem::replace(&mut self.settings.shadows, false).then_some("shadows off")
                                    });
                                    if let Some(what) = what {
                                        log::warn!("frame rate {fps:.0} fps at the smallest render scale: {what} to keep up");
                                        self.service_msg = Some((format!("The graphics card cannot keep up: {what}"), 4.0));
                                    }
                                }
                            }
                        }
                    }
                }
                if profiling {
                    self.profile_prev.clone_from(&self.profile);
                }
                let dt = raw_dt.min(0.1);
                self.last = now;
                self.run_input_script(event_loop);
                if let Some(m) = self.menu.as_ref() {
                    if let Some(limit) = self.args.exit_after {
                        if self.started.elapsed().as_secs_f32() > limit {
                            log::info!(
                                "menu: {} maps, {} vehicles",
                                m.maps.len(),
                                m.vehicles.len()
                            );
                            crate::platform::exit(event_loop);
                        }
                    }
                    let lines = m.lines();
                    if let (Some(hud), Some(s), Some(r), Some(scene), Some(win)) = (
                        self.hud.as_mut(),
                        self.surface.as_ref(),
                        self.renderer.as_mut(),
                        self.scene.as_mut(),
                        self.window.as_ref(),
                    ) {
                        hud.update(r, scene, &lines);
                        if let wgpu::CurrentSurfaceTexture::Success(frame)
                        | wgpu::CurrentSurfaceTexture::Suboptimal(frame) =
                            s.surface.get_current_texture()
                        {
                            let view = frame.texture.create_view(&Default::default());
                            let cam = Camera {
                                position: DVec3::ZERO,
                                yaw: 0.0,
                                pitch: 0.0,
                                roll: 0.0,
                                fov_deg: 60.0,
                                near: 0.5,
                                far: 100.0,
                            };
                            let lighting = omsi_render::Lighting {
                                sky_color: glam::Vec3::new(0.08, 0.10, 0.14),
                                ..Default::default()
                            };
                            r.render(
                                scene,
                                &view,
                                s.config.width,
                                s.config.height,
                                &cam,
                                &lighting,
                            );
                            win.pre_present_notify();
                            frame.present();
                        }
                    }
                    return;
                }
                if !self.drive_start(event_loop) {
                    // the session goes on while the map loads: a big map's first area took
                    // longer than the host waits for a silent player
                    if let Some(l) = self.lan.as_mut() {
                        let planned = omsi_net::Pose {
                            bus: self.args.bus.clone().unwrap_or_default().replace('\\', "/"),
                            ..Default::default()
                        };
                        l.keepalive(dt, &planned);
                    }
                    return;
                }
                let __t = Instant::now();
                self.drive_streaming();
                *self.profile.entry("streaming").or_default() += __t.elapsed().as_secs_f64();
                let __t = Instant::now();
                if let (Some(t), Some(w), Some(r), Some(scene)) = (
                    self.traffic.as_mut(),
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
                        .surface
                        .as_ref()
                        .map(|s| s.config.width as f64 / s.config.height.max(1) as f64)
                        .unwrap_or(16.0 / 9.0);
                    let fog = self
                        .weather
                        .as_ref()
                        .map(|w| w.fog.0 as f64)
                        .unwrap_or(50000.0);
                    traffic_inputs(
                        t,
                        self.camera.as_ref(),
                        aspect,
                        fog,
                        &self.clock,
                        self.humans.as_ref(),
                        self.player.as_ref(),
                        &r.options,
                    );
                    self.populate_t -= dt;
                    if self.populate_t <= 0.0 && !self.paused {
                        // come back quickly while there is a backlog of departures to put out
                        self.populate_t = if self
                            .schedule
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
                        t.populate_seen(w, r, scene, center, view);
                        *self.profile.entry("traffic.populate").or_default() +=
                            __t5.elapsed().as_secs_f64();
                        t.keep_clear = self
                            .player
                            .as_ref()
                            .map(|p| traffic::vehicle_bodies(&p.vehicle))
                            .unwrap_or_default();
                        t.keep_clear.extend(
                            self.remotes
                                .remotes
                                .values()
                                .flat_map(|r| traffic::vehicle_bodies(r.vehicle())),
                        );
                        if let Some(s) = self.schedule.as_mut() {
                            let window = if self.first_populate {
                                20.0 * 60.0
                            } else {
                                2.5
                            };
                            let __t6 = Instant::now();
                            s.tick(w, t, r, scene, t.day_time, window);
                            *self.profile.entry("traffic.schedule").or_default() +=
                                __t6.elapsed().as_secs_f64();
                        }
                        self.first_populate = false;
                    }
                    // the AI's lights (and a bus's saloon lamps, which its scripts switch with
                    // them): by the time of day, and by day in fog, rain, snow or under a
                    // closed cloud cover as drivers do
                    let gloomy = self.weather.as_ref().map(|w| {
                        let (kind, rate) = precip_of(w);
                        w.fog.0 < 600.0 || (kind != 0 && rate > 0.05) || w.clouds.0.trim().to_ascii_lowercase().starts_with("overcast")
                    }).unwrap_or(false);
                    t.night = omsi_sim::Daylight::compute(&self.clock, self.envir.as_ref())
                        .lamps_on
                        || gloomy;
                    let __t2 = Instant::now();
                    t.others = lan_outlines(&self.remotes);
                    t.others.extend(own_outlines(self.player.as_ref(), &self.placed));
                    if !self.paused {
                        t.player_priority = self.player.as_ref().and_then(|p| p.vehicle.var("TrafficPriority")).is_some_and(|v| v > 0.5);
                        t.tick(dt, self.player.as_ref().map(|p| player_outline(p)));
                        if let Some(w) = self.world.as_ref() {
                            w.set_switches(&t.switch_requests());
                            let rail = self.player.as_ref().and_then(|p| p.rail.as_ref()).map(|r| (r.lane, r.along));
                            w.set_signals(&t.signal_aspects(&w.signal_routes, rail));
                        }
                    }
                    *self.profile.entry("traffic.tick").or_default() +=
                        __t2.elapsed().as_secs_f64();
                    for (k, v) in ["traffic.tick.lanes", "traffic.tick.plan", "traffic.tick.ai"]
                        .into_iter()
                        .zip(t.tick_split)
                    {
                        *self.profile.entry(k).or_default() += v;
                    }
                    if let Some(p) = self.player.as_mut() {
                        // the options' [no_collision_vehToVeh]: the bus drives through the traffic
                        p.vehicle.dynamic_boxes = if self.settings.collision_vehicles { t.boxes(p.vehicle.position, 80.0) } else { Vec::new() };
                    }
                    let __t3 = Instant::now();
                    if let Some(a) = self.audio.as_ref() {
                        let street = self
                            .weather
                            .as_ref()
                            .map(|w| street_condition(w, self.wetness))
                            .unwrap_or(0.0);
                        let muffled = self.in_cab;
                        // heard round the camera (the ear), not round the player's bus: a
                        // free camera following an AI bus lost its sound 250 m from the bus
                        let ear = self.camera.as_ref().map(|c| c.position).unwrap_or(center);
                        t.update_audio(a, ear, street, muffled);
                    }
                    *self.profile.entry("traffic.audio").or_default() +=
                        __t3.elapsed().as_secs_f64();
                    let __t4 = Instant::now();
                    t.camera = self.camera.as_ref().map(|c| c.position);
                    t.sync(w, r, scene);
                    *self.profile.entry("traffic.sync").or_default() +=
                        __t4.elapsed().as_secs_f64();
                }
                *self.profile.entry("traffic").or_default() += __t.elapsed().as_secs_f64();
                // The player's vehicle moves before the passengers are placed: they sit in
                // the bus frame, and placing them on the pose of the frame before made everyone
                // aboard tremble at speed (a quarter of a metre behind the seat, every frame).
                let __t = Instant::now();
                self.drag_frame();
                // the tutorial's pages, once the world is there
                if self.world.is_some() {
                    if let Some(n) = self.args.tutorial.take() {
                        self.tutorial = crate::tutorial::Tutorial::load(&self.args.root, n, &self.settings.language);
                    }
                }
                // the game controllers: their axes this frame, their buttons' key actions
                let hwnd = self.window.as_deref().and_then(crate::controllers::window_handle);
                let ctl = self.controllers.get_or_insert_with(|| crate::controllers::Controllers::new(&self.args.root, hwnd));
                ctl.set_focus(self.window_focused);
                ctl.deadzone = self.settings.ctrl_deadzone;
                ctl.pedal_throttle = self.settings.pedal_throttle;
                ctl.pedal_brake = self.settings.pedal_brake;
                ctl.ff_invert = self.settings.ff_invert;
                ctl.ff_enabled = self.settings.ff_enabled;
                ctl.steer_gain = if self.settings.wheel_lock >= 45.0 { (self.settings.wheel_range / self.settings.wheel_lock).clamp(0.1, 20.0) } else { 1.0 };
                if ctl.disabled.is_empty() && !self.settings.ctrl_off.is_empty() {
                    ctl.disabled = self.settings.ctrl_off.split('|').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                }
                let analog = ctl.poll();
                let actions = std::mem::take(&mut ctl.actions);
                if let Some(n) = ctl.notice.take() {
                    self.service_msg = Some((n, 8.0));
                }
                // the bus's force feedback (OMSI's FF_Vib_Amp, and on a wheel its forces)
                // (in every view of the bus - the wheel went slack outside and in the
                // passenger view)
                let driving = self.player.as_ref().filter(|_| matches!(self.view.as_str(), "driver" | "outside" | "pax") && !self.paused);
                let kmh = driving.map(|p| p.vehicle.physics.velocity_kmh()).unwrap_or(0.0);
                ctl.feedback(crate::controllers::FfInput {
                    on: driving.is_some(),
                    kmh,
                    lateral_accel: driving.and_then(|p| p.vehicle.rigid.as_ref()).map(|r| r.accel_body.x).unwrap_or(0.0),
                    wheel_bump: driving.and_then(|p| p.vehicle.rigid.as_ref()).map(|r| crate::controllers::wheel_contact_bump(r, kmh)).unwrap_or(0.0),
                    wheel_bump_age: 0.0,
                    vib_amp: driving.and_then(|p| p.vehicle.var("FF_Vib_Amp")).unwrap_or(0.0),
                    vib_period: driving.and_then(|p| p.vehicle.var("FF_Vib_Period")).unwrap_or(0.0),
                    dt,
                });
                // OMSI's mouse control: the cursor's place across steers, above the middle
                // of the window is the throttle, below it the brake.
                // Steering as Omsi.exe has it (0x6f4284..0x6f447b): the whole width of the
                // window is the full lock from left to right, divided by the speed in tens
                // of km/h above 10 km/h - at 50 km/h the same hand movement turns the wheel a
                // fifth as far, which is what makes the wheel feel heavier the faster the bus
                // goes. For a second after mouse steering is switched on the wheel eases
                // towards the cursor (a half-life of the time that is left), then follows it.
                let mut analog = analog;
                // a gamepad's stick: a target the wheel turns towards at a hand's pace (the
                // whole lock in 1.2 s), not the wheel's place itself (#200)
                if analog.stick {
                    if let (Some(x), Some(p)) = (analog.steering, self.player.as_ref()) {
                        let target = crate::controllers::gamepad_steering(x, p.vehicle.physics.velocity_kmh() as f32);
                        let now = p.vehicle.physics.controls.steering;
                        let step = dt / 1.2;
                        analog.steering = Some(now + (target - now).clamp(-step, step));
                    }
                }
                // (in every view of the bus - driver, outside, passenger - as in OMSI, where
                // switching the camera leaves the mouse steering on; not on foot or flying)
                let bus_view = matches!(self.view.as_str(), "driver" | "outside" | "pax");
                if let (true, Some(s)) = (self.mouse_drive && bus_view && !self.mouse_look
                                              && self.game_menu.is_none(), self.surface.as_ref()) {
                    let (w, h) = (s.config.width as f32, s.config.height as f32);
                    // (the speed the divisor takes, smoothed over 0.4 s: the bus's own speed
                    // trembles by fractions of a km/h from frame to frame on its springs and
                    // tyres, and at 30 km/h the wheel twitched with it by itself)
                    let raw_kmh = self.player.as_ref().map(|p| p.vehicle.physics.velocity_kmh()).unwrap_or(0.0);
                    let k_v = 1.0 - (-dt / 0.4).exp();
                    self.mouse_kmh += (raw_kmh - self.mouse_kmh) * k_v;
                    let kmh = self.mouse_kmh;
                    let base = (crate::player::mouse_steering(self.cursor.0, w, kmh) * self.settings.mouse_sens).clamp(-1.0, 1.0);
                    // (what the mouse added at the edge, up to the full lock)
                    self.mouse_edge = self.mouse_edge.clamp((-1.0 - base).min(0.0), (1.0 - base).max(0.0));
                    let target = (base + self.mouse_edge).clamp(-1.0, 1.0);
                    // the pedals as Omsi.exe has them: from the middle of the window to its
                    // top edge the throttle, to the bottom one the brake, straight on
                    let y = (2.0 * self.cursor.1 / h.max(1.0) - 1.0).clamp(-1.0, 1.0);
                    let (pedal_t, pedal_b) = ((-y).max(0.0), y.max(0.0));
                    let (steer, fade) = &mut self.mouse_steer;
                    // (after the first second the wheel follows the cursor within ~60 ms: the
                    // cursor comes in bursts, and taken as it came the wheel moved in steps)
                    let k = if *fade > 0.0 { (-std::f32::consts::LN_2 / *fade * dt).exp() } else { (-dt / 0.06).exp() };
                    *steer = target + (*steer - target) * k;
                    let (mt, mb) = &mut self.mouse_pedals;
                    *mt = crate::player::mouse_pedal(*mt, pedal_t, k);
                    *mb = crate::player::mouse_pedal(*mb, pedal_b, k);
                    *fade = (*fade - dt).max(0.0);
                    analog.steering = Some(*steer);
                    // OMSI_TRACE_STEER=<csv>: the mouse steering frame by frame
                    if let Some(path) = omsi_cfg::env::var_os("OMSI_TRACE_STEER") {
                        use std::io::Write;
                        static TRACE: std::sync::Mutex<Option<std::fs::File>> = std::sync::Mutex::new(None);
                        let mut g = TRACE.lock().unwrap_or_else(|e| e.into_inner());
                        if g.is_none() {
                            *g = std::fs::File::create(&path).ok();
                            if let Some(f) = g.as_mut() {
                                let _ = writeln!(f, "t,dt,cursor_x,kmh,target,steer,steer_deg");
                            }
                        }
                        let deg = self.player.as_ref().map(|p| p.vehicle.physics.steer_deg).unwrap_or(0.0);
                        if let Some(f) = g.as_mut() {
                            let _ = writeln!(f, "{:.3},{:.4},{:.1},{:.2},{:.4},{:.4},{:.3}", self.clock.run_time, dt, self.cursor.0, kmh, target, self.mouse_steer.0, deg);
                        }
                    }
                    // the mouse owns the wheel (OMSI sets the curvature from it every frame):
                    // a steering key's leftover turn must not take over whenever the cursor
                    // passes the middle - the wheel jumped there
                    if let Some(p) = self.player.as_mut() {
                        p.axes.steering = 0.0;
                    }
                    analog.throttle = Some(self.mouse_pedals.0);
                    analog.brake = Some(self.mouse_pedals.1);
                } else if self.mouse_drive && bus_view && self.mouse_look
                    && self.game_menu.is_none() {
                    // looking round with the right button: the wheel and the pedals stay where
                    // the mouse left them, as in OMSI (they went slack until the button was let
                    // go - no quick look round while driving)
                    analog.steering = Some(self.mouse_steer.0);
                    analog.throttle = Some(self.mouse_pedals.0);
                    analog.brake = Some(self.mouse_pedals.1);
                    if let Some(p) = self.player.as_mut() {
                        p.axes.steering = 0.0;
                    }
                }
                // the controller's view buttons are the game's, not the bus's: looking around
                // while held (`view_look_*`), and OMSI's view actions (other cameras, views)
                let mut actions = actions;
                if self.game_menu.is_none() {
                    let mut game: Vec<String> = Vec::new();
                    actions.retain(|(name, down)| {
                        let n = name.to_ascii_lowercase();
                        if let Some(k) = ["view_look_left", "view_look_right", "view_look_up", "view_look_down"].iter().position(|x| *x == n) {
                            self.pad_look[k] = *down;
                            return false;
                        }
                        if n == "gear_up" || n == "gear_down" {
                            if *down {
                                game.push(n);
                            }
                            return false;
                        }
                        if crate::input_script::is_game_action(&n) {
                            if *down {
                                game.push(n);
                            }
                            return false;
                        }
                        true
                    });
                    for n in game {
                        match n.as_str() {
                            "gear_up" => { self.shift_gear(true); }
                            "gear_down" => { self.shift_gear(false); }
                            _ => { self.game_action(&n); }
                        }
                    }
                }
                if let Some(p) = self.player.as_mut() {
                    p.axes.linear = self.settings.steering_linear;
                    p.axes.old_steering = self.settings.old_steering;
                    p.axes.pedal_hold = self.settings.brake_hold;
                    p.analog = analog;
                    if self.game_menu.is_none() {
                        for (name, down) in actions {
                            p.action(&name, down);
                        }
                    }
                }
                // the on-screen wheel and pedals (a phone)
                self.touch_frame(dt);
                if let (Some(p), Some(r), Some(scene)) = (
                    self.player.as_mut(),
                    self.renderer.as_ref(),
                    self.scene.as_mut(),
                ) {
                    // (while the tile under the bus is being read again - the weather turned
                    // to snow and every tile came back with the winter textures - there is
                    // no ground under it: it is held where it is rather than falling through
                    // the world and being put back somewhere in the sky)
                    let ground_here = self.world.as_ref().is_none_or(|w| {
                        let at = p.vehicle.position;
                        let k = ((at.x / omsi_map::tile_size()).floor() as i32, (at.y / omsi_map::tile_size()).floor() as i32);
                        w.terrains.read().contains_key(&k) || w.surfaces.read().contains_key(&k)
                    });
                    if !self.paused && ground_here {
                        p.tick(
                            dt,
                            self.audio.as_ref(),
                            self.in_cab,
                            !matches!(self.view.as_str(), "free" | "foot"),
                        );
                        // (not in the headset: the player's own head moves there, and a head
                        // thrown about by the bus on top of it made the whole cab sway and
                        // shift before the eyes)
                        #[cfg(windows)]
                        let vr_on = self.vr.is_some();
                        #[cfg(not(windows))]
                        let vr_on = false;
                        p.move_head(dt, self.settings.head_movement && !vr_on, self.settings.steer_look && !vr_on);
                        if let Some(w) = self.world.as_ref() {
                            crate::rail_drive::frame(p, self.traffic.as_ref().map(|t| &t.net), w, dt);
                        }
                    }
                    // a script that set the time of day (`(S.S.Time)`) moves the game's clock
                    if let Some(t) = p.vehicle.host.time_written.take() {
                        self.pending_time = Some(t);
                    }
                    // the situation's further vehicles stand and run their scripts, and the
                    // player's bus meets them
                    let mut placed_boxes = Vec::new();
                    for q in self.placed.iter_mut() {
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
                    if !self.placed.is_empty() {
                        // (the traffic writes the list afresh every frame; without it, this does)
                        if self.traffic.is_none() {
                            p.vehicle.dynamic_boxes.clear();
                        }
                        p.vehicle.dynamic_boxes.extend(placed_boxes);
                    }
                    if let Some(w) = self.world.as_ref() {
                        lay_down_poles(w, r, scene, &mut p.vehicle);
                    }
                    static EVERY: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
                    if let Some(every) = *EVERY.get_or_init(|| {
                        omsi_cfg::env::var("OMSI_DEBUG_PHYSICS")
                            .ok()
                            .and_then(|v| v.parse::<f32>().ok())
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
                    let inside = self.in_cab;
                    p.sync_transforms(r, scene, inside);
                    // from the driver's seat the figure stays in the mirrors
                    // (from the driver's seat only the mirrors show him)
                    // (out of the seat: nobody at the wheel)
                    p.sync_driver(r, scene, dt, self.settings.driver && self.on_foot.is_none(), self.view == "driver");
                    if self.view != "free" && self.view != "foot" {
                        let key = crate::input_script::look_key_of(&self.view, Some(p.cam_choice));
                        crate::input_script::swap_view_look(&mut self.look, &mut self.view_looks, &mut self.look_view, &key);
                        if let Some(cam) = self.camera.as_ref() {
                            p.seat = glam::Vec3::from_array(self.settings.seat);
                            // head tracking: the head's turn on top of the look, its movement
                            // on top of the seat (opentrack: x right, y up, z back, in cm)
                            if self.settings.head_tracking && self.headtrack.is_none() {
                                self.headtrack = crate::headtrack::HeadTracker::start(self.settings.head_tracking_port);
                                if self.headtrack.is_none() {
                                    self.settings.head_tracking = false;
                                }
                            }
                            let tracked = self.headtrack.as_ref().and_then(|h| h.pose()).filter(|_| self.settings.head_tracking && matches!(self.view.as_str(), "driver" | "pax"));
                            if let Some(t) = tracked {
                                p.seat += glam::Vec3::new(t.pos[0], -t.pos[2], t.pos[1]).clamp(glam::Vec3::splat(-60.0), glam::Vec3::splat(60.0)) / 100.0;
                            }
                            // (the outside view's field of view starts from the plain 60
                            // degrees every frame: taken from the last frame's camera, the
                            // zoom was applied on top of itself and ran off to its narrowest
                            // or widest at once)
                            let base = omsi_render::Camera { fov_deg: 60.0, ..*cam };
                            let mut cam = p.camera_look(&self.view, &base, self.look, self.orbit);
                            if let Some(mut t) = tracked {
                                for (k, axis) in ["yaw", "pitch", "roll"].iter().enumerate() {
                                    if self.settings.head_tracking_invert.contains(axis) {
                                        t.rot[k] = -t.rot[k];
                                    }
                                }
                                cam.yaw += t.rot[0].clamp(-170.0, 170.0);
                                cam.pitch = (cam.pitch + t.rot[1].clamp(-80.0, 80.0)).clamp(-89.0, 89.0);
                                cam.roll += t.rot[2].clamp(-60.0, 60.0);
                            }
                            // Settings → Field of view (0: the bus's own cameras)
                            if self.settings.fov >= 20.0 {
                                cam.fov_deg = self.settings.fov.min(120.0);
                            }
                            if let Some(z) = self.view_zoom.get(&self.view) {
                                cam.fov_deg = (cam.fov_deg * z).clamp(8.0, 120.0);
                            }
                            if self.view == "outside" && self.settings.camera_collision {
                                if let Some(w) = self.world.as_ref() {
                                    cam = p.camera_clipped(cam, w, self.orbit, dt);
                                }
                            } else {
                                p.arm.reset();
                            }
                            self.camera = Some(cam);
                        }
                    } else if let Some(cam) = self.camera.as_mut() {
                        // the free camera and the view on foot follow the setting too (they
                        // stayed at 60 degrees whatever it said)
                        let base = if self.settings.fov >= 20.0 { self.settings.fov.min(120.0) } else { 60.0 };
                        cam.fov_deg = (base * self.view_zoom.get(&self.view).copied().unwrap_or(1.0)).clamp(8.0, 120.0);
                    }
                    let __th = Instant::now();
                    // (the cursor's aim into the cab: again when the cursor or the view
                    // turned, else every few frames for switches that moved under it - a ray
                    // through every cockpit mesh every frame was a tenth of the frame)
                    let key = self.camera.as_ref().map(|c| (self.cursor.0.round() as i32, self.cursor.1.round() as i32, (c.yaw * 4.0).round() as i32, (c.pitch * 4.0).round() as i32));
                    // (the cab sways with the suspension: a view that only turned waits a few frames)
                    let cursor_moved = key.map(|k| (k.0, k.1)) != self.hover_key.map(|k| (k.0, k.1));
                    if cursor_moved || (key != self.hover_key && self.total_frames % 6 == 0) || self.total_frames % 12 == 0 {
                        self.hover_key = key;
                        self.update_hover();
                    }
                    *self.profile.entry("player.hover").or_default() +=
                        __th.elapsed().as_secs_f64();
                    if let Some(a) = self.audio.as_ref() {
                        a.follow_device();
                    }
                    if let (Some(a), Some(cam)) = (self.audio.as_ref(), self.camera.as_ref()) {
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
                }
                // on foot without a bus of one's own: the vehicles one placed still stand, run
                // their scripts and are drawn where they are (the player's frame did it)
                if let (None, Some(r), Some(scene)) = (self.player.as_ref(), self.renderer.as_ref(), self.scene.as_mut()) {
                    for q in self.placed.iter_mut() {
                        if !self.paused {
                            q.vehicle.update(dt);
                        }
                        q.sync_transforms(r, scene, false);
                    }
                }
                if let Some(a) = self.audio.as_ref() {
                    match self.player.as_ref() {
                        Some(p) => {
                            let inside = self.in_cab;
                            if let Some(m) = self.radio.update(a, &p.vehicle, inside) {
                                self.service_msg = Some((m, 6.0));
                            }
                        }
                        None => self.radio.stop(a),
                    }
                }
                *self.profile.entry("player").or_default() += __t.elapsed().as_secs_f64();
                let __t = Instant::now();
                self.tick_lan(dt);
                // (a stage of its own: a joining player's bus is loaded here, and that frame
                // was counted as the people's)
                *self.profile.entry("lan").or_default() += __t.elapsed().as_secs_f64();
                // the player on foot, and the other players walking about
                self.tick_on_foot(if self.paused { 0.0 } else { dt });
                self.sync_remote_walkers();
                let __t = Instant::now();
                if let (Some(h), Some(w), Some(r), Some(scene)) = (
                    self.humans.as_mut(),
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
                    // (the riders leave a bus the driver has walked away from)
                    if h.stop_targets.is_none() {
                        h.stop_targets = self.schedule.as_ref().map(|s| s.stop_targets());
                        if let Some(t) = &h.stop_targets {
                            log::info!("people: {} bus stops with timetable targets", t.len());
                        }
                    }
                    h.driver_away = self.on_foot.as_ref().is_some_and(|f| {
                        let own = Some(crate::humans::BusId::Player);
                        f.seat.map(|s| s.0) != own && f.inside.map(|i| i.0) != own
                    });
                    h.density = w
                        .global
                        .passenger_density((self.clock.time / 3600.0) as f32)
                        // (OMSI's `AIPassFactor`, the passengers setting in per cent)
                        * self.settings.pax_density;
                    h.time_of_day = self.clock.time;
                    h.delay = self.duty.as_ref().map(|d| d.delay(self.clock.time)).unwrap_or(0.0);
                    self.humans_populate_t -= dt;
                    if self.humans_populate_t <= 0.0 && !self.paused {
                        self.humans_populate_t = 2.0;
                        h.populate(w, r, scene, center);
                    }
                    if let (Some(cam), Some(s)) = (self.camera.as_ref(), self.surface.as_ref()) {
                        h.eye = Some(humans::Eye::of(
                            cam,
                            s.config.width as f32 / s.config.height.max(1) as f32,
                        ));
                    }
                    // (the other LAN players' buses, for their riders to sit in)
                    h.set_remote_buses(self.remotes.remotes.iter().map(|(id, r)| (*id, r.vehicle())));
                    // (and the vehicles the player placed and left, with their riders)
                    h.set_placed_buses(self.placed.iter().map(|q| (q.uid, &q.vehicle)));
                    let took = h.tick(
                        if self.paused { 0.0 } else { dt },
                        w,
                        self.player.as_ref().map(|p| &p.vehicle),
                        self.traffic.as_ref(),
                        r,
                        scene,
                    );
                    // validators used: the bus's `ev_Stamper` sound
                    for bus in h.take_stamped() {
                        match bus {
                            None => {
                                if let Some(p) = self.player.as_mut() {
                                    p.vehicle.host.fired_triggers.push("ev_Stamper".into());
                                }
                            }
                            Some(id) => {
                                if let Some(c) = self.traffic.as_mut().and_then(|t| t.cars.iter_mut().find(|c| c.id == id)) {
                                    c.vehicle.host.fired_triggers.push("ev_Stamper".into());
                                }
                            }
                        }
                    }
                    if let Some(t) = self.traffic.as_mut() {
                        for (id, secs) in h.take_holds() {
                            t.hold_boarding(id, secs);
                        }
                        for (id, entry, exit) in h.take_ai_requests() {
                            t.set_pax_requests(id, &entry, &exit);
                        }
                    }
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
                            p.vehicle.trigger("door_haltewunsch");
                            // (a press is let go again: the script keeps its button pressed
                            // until `_off`, and the stop request never ended - the automatic
                            // rear door opened again whenever it was shut)
                            p.vehicle.trigger("door_haltewunsch_off");
                        }
                        if std::mem::take(&mut h.door_request) {
                            p.vehicle.trigger("door_aussenoeffner");
                            p.vehicle.trigger("door_aussenoeffner_off");
                        }
                        h.write_pax_vars(&mut p.vehicle);
                        p.vehicle.host.humans_on_path_link = h.path_link_counts();
                        p.vehicle.host.humans_on_seat = h.seat_counts();
                        let coins: Vec<usize> = std::mem::take(&mut p.vehicle.host.change_coins);
                        h.give_change(w, r, scene, &coins);
                        h.sync_money(r, scene, &p.vehicle);
                    }
                    h.sync(r, scene, center);
                }
                *self.profile.entry("humans").or_default() += __t.elapsed().as_secs_f64();
                self.foot_after_humans();
                if let (Some(d), Some(p), false) = (self.duty.as_mut(), self.player.as_mut(), self.paused) {
                    if let Some(stop) = p.html_next_stop.take() {
                        d.skip_to(stop);
                    }
                    if let Some((arrival, departure)) = d.update(&mut p.vehicle, self.clock.time) {
                        self.career.stop_served(arrival, departure);
                    }
                    if d.take_trip_change() && p.duty_typed {
                        let (trip, stop) = d.trip_for_ibis();
                        p.set_duty_destination(trip, stop);
                    }
                }
                if let Some(p) = self.player.as_mut() {
                    let riders = self.humans.as_ref().map(|h| h.riding()).unwrap_or(0);
                    // the engine's own variables of the bus (see `update_engine_vars`)
                    p.vehicle.host.humans_count = riders as f32;
                    p.vehicle.host.schedule_active = if self.duty.is_some() { 1.0 } else { 0.0 };
                    let crash = std::mem::take(&mut p.vehicle.last_crash);
                    // (a frame after the session was written must not start another one)
                    if !self.exiting && !self.paused {
                        self.career.tick(dt, &p.vehicle, riders);
                    }
                    if crash > 0.0 {
                        self.career.crashed(crash, p.vehicle.physics.velocity_kmh() / 3.6);
                        self.service_msg = Some((format!("Crash: {:.0} kJ", crash / 1000.0), 6.0));
                    }
                }
                if !self.paused {
                    crate::admin::guard_fall(self, dt);
                }
                self.placing_frame();
                // the host sends every edit of the map again now and then (players join)
                if self.lan.as_ref().map(|l| l.role == omsi_net::Role::Host).unwrap_or(false) {
                    self.editor_sync_t -= dt;
                    if self.editor_sync_t <= 0.0 {
                        self.editor_sync_t = 10.0;
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
                // Discord's status: the map, the bus, the line (every few seconds)
                self.discord_t -= dt;
                if self.discord_t <= 0.0 {
                    self.discord_t = 5.0;
                    if self.discord.is_none() && self.settings.discord_status && !self.settings.discord_app_id.is_empty() {
                        self.discord = crate::discord::Discord::start(&self.settings.discord_app_id);
                        if self.discord.is_none() {
                            self.settings.discord_status = false;
                        }
                    }
                    if let Some(d) = self.discord.as_ref() {
                        let map = self.world.as_ref().map(|w| w.global.name.clone()).unwrap_or_default();
                        let bus = self.player.as_ref().map(|p| format!("{} {}", p.vehicle.ty.def.manufacturer, p.vehicle.ty.def.type_name).trim().to_string());
                        let state = match (self.duty.as_ref(), bus.as_ref()) {
                            (Some(duty), _) => format!("Line {} · tour {}{}", duty.line.trim(), duty.tour.trim(), if self.lan.is_some() { " · multiplayer" } else { "" }),
                            (None, Some(_)) => if self.lan.is_some() { "Free drive · multiplayer".to_string() } else { "Free drive".to_string() },
                            (None, None) => "On foot".to_string(),
                        };
                        let details = match bus { Some(b) if !b.is_empty() => format!("{b} · {map}"), _ => map };
                        d.set(crate::discord::Presence { details, state });
                    }
                }
                // the plugins' frame, with the bus's scripts done
                let plugins = self.plugins.get_or_insert_with(crate::plugins::load);
                if !plugins.is_empty() && !self.paused {
                    let info = crate::plugins::game_info(self);
                    let keys = std::mem::take(&mut self.plugin_keys);
                    let plugins = self.plugins.as_mut().unwrap();
                    let mut io = crate::plugins::Io { vehicle: self.player.as_mut().map(|p| &mut p.vehicle), dt, message: None, info, commands: Vec::new(), keys };
                    plugins.frame(&mut io);
                    let commands = std::mem::take(&mut io.commands);
                    if let Some(m) = io.message {
                        self.service_msg = Some(m);
                    }
                    // what the plugins asked the game to do: lines of the game menu
                    for c in commands {
                        if let Some(k) = self.game_menu_items().iter().position(|m| m.0 == c) {
                            let was = self.game_menu;
                            self.menu_prev_pause = self.paused;
                            self.menu_choose(event_loop, k);
                            // (an action leaves the menu as it found it)
                            if self.chooser.is_none() && was.is_none() {
                                self.game_menu = None;
                            }
                        }
                    }
                } else {
                    self.plugin_keys.clear();
                }
                // OMSI_WATCH_VARS=a,b: every change of those variables of the player's bus
                if let (Some(p), Ok(list)) = (self.player.as_ref(), omsi_cfg::env::var("OMSI_WATCH_VARS")) {
                    thread_local!(static LAST: std::cell::RefCell<std::collections::HashMap<String, f32>> = Default::default());
                    LAST.with(|last| {
                        let mut last = last.borrow_mut();
                        for n in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                            let v = p.vehicle.var(n).unwrap_or(f32::NAN);
                            if last.get(n).is_none_or(|&o| o.to_bits() != v.to_bits()) {
                                log::info!("watch: {n} = {v} at {:.2} s", self.clock.time);
                                last.insert(n.to_string(), v);
                            }
                        }
                    });
                }
                if let (Some(h), Some(p)) = (self.humans.as_mut(), self.player.as_ref()) {
                    self.career.tickets = (h.tickets_sold as i32, h.ticket_cash as f64);
                    self.career.boarded = h.boarded as i32;
                    self.career.served = h.served as i32;
                    self.career.stepped_in = h.stepped_in as i32;
                    self.career.content = h.content as i32;
                    self.career.ticket_requests = h.ticket_requests as i32;
                    self.career.ticket_points = h.ticket_points as i32;
                    // the options' [no_collision_pedastrians]: nobody is knocked down
                    let hurt = if self.settings.collision_pedestrians { h.run_over(&p.vehicle) } else { 0 };
                    if hurt > 0 {
                        self.career.crashes[1] += hurt as i32;
                        self.service_msg = Some(("Pedestrian knocked down!".into(), 6.0));
                    }
                }
                // looking around and zooming work in every view, not only the free camera
                self.sync_view_look();
                if self.player.is_some() && self.view != "free" {
                    // looking around with the keyboard: Alt + I/J/K/L (the plain letters
                    // belong to the bus - L is the headlights in Inputs/keyboard.cfg)
                    let step = 60.0 * dt;
                    // Ctrl+Alt+arrows in the cab: the mirror nearest to where the driver looks
                    // turns (kept per bus in mirrors.cfg when the keys are let go)
                    let ctrl_alt = (self.keys.contains(&KeyCode::ControlLeft) || self.keys.contains(&KeyCode::ControlRight)) && (self.keys.contains(&KeyCode::AltLeft) || self.keys.contains(&KeyCode::AltRight));
                    let arrows = [KeyCode::ArrowLeft, KeyCode::ArrowRight, KeyCode::ArrowUp, KeyCode::ArrowDown].map(|k| self.keys.contains(&k));
                    if let (true, Some(p), Some(cam)) = (ctrl_alt && self.view == "driver" && arrows.iter().any(|a| *a), self.player.as_mut(), self.camera.as_ref()) {
                        let cams = &p.vehicle.ty.def.cameras_reflexion;
                        let f = cam.forward();
                        let best = (0..cams.len())
                            .map(|i| (i, (p.vehicle.camera_world_full(&cams[i]).0 - cam.position).as_vec3().normalize_or_zero().dot(f)))
                            .max_by(|a, b| a.1.total_cmp(&b.1))
                            .map(|(i, _)| i);
                        if let Some(i) = best {
                            if p.mirror_offsets.len() <= i {
                                p.mirror_offsets.resize(cams.len(), [0.0; 2]);
                            }
                            let o = &mut p.mirror_offsets[i];
                            let rate = 12.0 * dt;
                            o[0] = (o[0] + rate * (arrows[1] as i32 - arrows[0] as i32) as f32).clamp(-45.0, 45.0);
                            o[1] = (o[1] + rate * (arrows[2] as i32 - arrows[3] as i32) as f32).clamp(-30.0, 30.0);
                            p.mirrors_dirty = true;
                            self.service_msg = Some((format!("Mirror {}: {:+.1}° across, {:+.1}° up (Ctrl+Alt+arrows)", i + 1, o[0], o[1]), 2.0));
                        }
                    } else if let Some(p) = self.player.as_mut().filter(|p| p.mirrors_dirty) {
                        p.mirrors_dirty = false;
                        crate::settings::save_mirror_offsets(&p.vehicle.ty.def.path, &p.mirror_offsets);
                    }
                    // a controller's look buttons (Settings → Controllers: view_look_*)
                    self.look.0 += step * 1.5 * (self.pad_look[1] as i32 - self.pad_look[0] as i32) as f32;
                    self.look.1 = (self.look.1 + step * 0.7 * (self.pad_look[2] as i32 - self.pad_look[3] as i32) as f32).clamp(-85.0, 85.0);
                    // with a wheel steering, the arrow keys look around as in OMSI
                    if !ctrl_alt && self.controllers.as_ref().is_some_and(|c| c.wheel_steering()) && !self.keys.contains(&KeyCode::ControlLeft) && !self.keys.contains(&KeyCode::ControlRight) {
                        // a glance: held, the head turns (to 140 degrees at most); let go, it
                        // comes back to the road - held, it went round and round, and the
                        // other key never brought it back straight
                        let (l, r) = (self.keys.contains(&KeyCode::ArrowLeft), self.keys.contains(&KeyCode::ArrowRight));
                        if l || r {
                            self.look.0 = (self.look.0 + step * 1.5 * (r as i32 - l as i32) as f32).clamp(-140.0, 140.0);
                            self.arrow_glance = true;
                        } else if self.arrow_glance {
                            self.look.0 *= (-6.0 * dt).exp();
                            if self.look.0.abs() < 0.5 {
                                self.look.0 = 0.0;
                                self.arrow_glance = false;
                            }
                        }
                        if self.keys.contains(&KeyCode::ArrowUp) {
                            self.look.1 = (self.look.1 + step * 0.7).min(85.0);
                        }
                        if self.keys.contains(&KeyCode::ArrowDown) {
                            self.look.1 = (self.look.1 - step * 0.7).max(-85.0);
                        }
                    }
                    let alt = self.keys.contains(&KeyCode::AltLeft)
                        || self.keys.contains(&KeyCode::AltRight);
                    if alt && self.keys.contains(&KeyCode::KeyJ) {
                        self.look.0 -= step;
                    }
                    if alt && self.keys.contains(&KeyCode::KeyL) {
                        self.look.0 += step;
                    }
                    if alt && self.keys.contains(&KeyCode::KeyI) {
                        self.look.1 = (self.look.1 + step * 0.7).min(85.0);
                    }
                    if alt && self.keys.contains(&KeyCode::KeyK) {
                        self.look.1 = (self.look.1 - step * 0.7).max(-85.0);
                    }
                    if self.view != "outside" {
                        self.look.0 = self.look.0.clamp(-140.0, 140.0);
                    }
                    // Ctrl+Shift+Page Up / Page Down held: the clock runs forwards / backwards,
                    // a quarter of an hour per second at first, faster the longer it is held
                    // (the menu's whole hours were the only way)
                    {
                        let ctrl = self.keys.contains(&KeyCode::ControlLeft) || self.keys.contains(&KeyCode::ControlRight);
                        let shift = self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight);
                        let dir = match (self.keys.contains(&KeyCode::PageUp), self.keys.contains(&KeyCode::PageDown)) {
                            (true, false) => 1.0,
                            (false, true) => -1.0,
                            _ => 0.0,
                        };
                        let client = self.lan.as_ref().is_some_and(|l| l.role == omsi_net::Role::Client);
                        if ctrl && shift && dir != 0.0 && !client {
                            self.clock_hold += dt;
                            let rate = 900.0 * (1.0 + self.clock_hold * 1.5).min(8.0);
                            self.shift_clock(dir * rate as f64 * dt as f64);
                        } else {
                            self.clock_hold = 0.0;
                        }
                    }
                    // = and - zoom inside the bus (the numpad's are door keys there)
                    if matches!(self.view.as_str(), "driver" | "pax") {
                        if self.keys.contains(&KeyCode::Equal) {
                            self.zoom_by(3.0 * dt);
                        }
                        if self.keys.contains(&KeyCode::Minus) {
                            self.zoom_by(-3.0 * dt);
                        }
                    }
                    // W/S and the wheel pull the outside camera in and out
                    if self.view == "outside" {
                        if self.keys.contains(&KeyCode::Equal)
                            || self.keys.contains(&KeyCode::NumpadAdd)
                        {
                            self.orbit = (self.orbit - 12.0 * dt).max(ORBIT_MIN);
                        }
                        if self.keys.contains(&KeyCode::Minus)
                            || self.keys.contains(&KeyCode::NumpadSubtract)
                        {
                            self.orbit = (self.orbit + 12.0 * dt).min(ORBIT_MAX);
                        }
                    }
                    if self.keys.contains(&KeyCode::Home) {
                        self.look = (0.0, 0.0);
                        self.orbit = ORBIT_DEFAULT;
                        self.view_zoom.remove(&self.view);
                    }
                }
                if self.view != "free" {
                    self.ego = false;
                }
                if let (Some(cam), true) = (
                    self.camera.as_mut(),
                    self.view == "free" || self.player.is_none(),
                ) {
                    let mut v = Vec3::ZERO;
                    let f = cam.forward();
                    let r = cam.right();
                    if self.keys.contains(&KeyCode::KeyW) {
                        v += f;
                    }
                    if self.keys.contains(&KeyCode::KeyS) {
                        v -= f;
                    }
                    if self.keys.contains(&KeyCode::KeyD) {
                        v += r;
                    }
                    if self.keys.contains(&KeyCode::KeyA) {
                        v -= r;
                    }
                    if self.keys.contains(&KeyCode::KeyE) || self.keys.contains(&KeyCode::Space) {
                        v += Vec3::Z;
                    }
                    if self.keys.contains(&KeyCode::KeyQ) {
                        v -= Vec3::Z;
                    }
                    let boost = if self.keys.contains(&KeyCode::ShiftLeft) {
                        5.0
                    } else {
                        1.0
                    };
                    if self.ego {
                        // walking: along the ground at eye height, 1.4 m/s (running 4.5)
                        let flat = Vec3::new(v.x, v.y, 0.0).normalize_or_zero();
                        let pace = if boost > 1.0 { 4.5 } else { 1.4 };
                        cam.position += (flat * pace * dt).as_dvec3();
                        if let Some(g) = self.world.as_ref().and_then(|w| w.walk_height(cam.position.x, cam.position.y)) {
                            cam.position.z = g + 1.7;
                        }
                    } else {
                        cam.position += (v.normalize_or_zero() * self.speed * boost * dt).as_dvec3();
                    }
                    if self.keys.contains(&KeyCode::ArrowLeft) {
                        cam.yaw -= 60.0 * dt;
                    }
                    if self.keys.contains(&KeyCode::ArrowRight) {
                        cam.yaw += 60.0 * dt;
                    }
                    if self.keys.contains(&KeyCode::ArrowUp) {
                        cam.pitch = (cam.pitch + 40.0 * dt).min(89.0);
                    }
                    if self.keys.contains(&KeyCode::ArrowDown) {
                        cam.pitch = (cam.pitch - 40.0 * dt).max(-89.0);
                    }
                }
                if !self.paused {
                    // (the time speed: the settings', or the session's in LAN play)
                    let speed = self.time_speed();
                    self.clock.advance(dt * speed as f32);
                    if let Some(t) = self.traffic.as_mut() {
                        t.time_scale = speed;
                    }
                    self.tick_weather(dt * speed as f32);
                }
                let daylight = omsi_sim::Daylight::compute(&self.clock, self.envir.as_ref());
                if self.lamps_on != Some(daylight.lamps_on) {
                    self.lamps_on = Some(daylight.lamps_on);
                    if let (Some(w), Some(r), Some(scene)) = (
                        self.world.as_ref(),
                        self.renderer.as_ref(),
                        self.scene.as_mut(),
                    ) {
                        w.set_lamps(r, scene, daylight.lamps_on);
                    }
                }
                // the lit windows of the houses by their [NightMapMode] timetable (once a
                // second: tiles come and go, and the hours pass)
                if self.total_frames % 60 == 0 {
                    if let (Some(w), Some(r), Some(scene)) = (
                        self.world.as_ref(),
                        self.renderer.as_ref(),
                        self.scene.as_mut(),
                    ) {
                        w.update_night_modes(r, scene, &self.clock, daylight.brightness);
                    }
                    self.follow_date();
                }
                if let Some(p) = self.player.as_mut() {
                    p.vehicle.set_var("Envir_Brightness", daylight.brightness);
                    p.vehicle.host.sun_alt = daylight.altitude_deg;
                    if let Some(w) = &self.weather {
                        apply_weather(&mut p.vehicle, w, self.wetness);
                    }
                }
                let __t = Instant::now();
                // the lamps' cones in fog and falling rain or snow, and new light pictures
                if let Some(wt) = &self.weather {
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
                *self.profile.entry("lights.atlas").or_default() += __ta.elapsed().as_secs_f64();
                if let (Some(w), Some(scene), Some(cam)) = (
                    self.world.as_ref(),
                    self.scene.as_mut(),
                    self.camera.as_ref(),
                ) {
                    let mut vehicles: Vec<&omsi_sim::VehicleInstance> = Vec::new();
                    if let Some(p) = self.player.as_ref() {
                        vehicles.push(&p.vehicle);
                    }
                    if let Some(t) = self.traffic.as_ref() {
                        vehicles.extend(t.cars.iter().map(|c| &c.vehicle));
                    }
                    vehicles.extend(self.remotes.remotes.values().map(|r| r.vehicle()));
                    let __tc = Instant::now();
                    lights::collect(w, scene, &daylight, cam.position, &vehicles);
                    *self.profile.entry("lights.collect").or_default() += __tc.elapsed().as_secs_f64();
                    // the object editor's pick: a magenta glow over it
                    if let Some(id) = self.editor.as_ref().and_then(|e| e.selected) {
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
                    if let Some(wt) = &self.weather {
                        let (kind, rate) = precip_of(wt);
                        self.rain.set(kind, rate);
                        // [wind] direction (deg) speed (m/s)
                        let wind = Vec3::new(
                            wt.wind.0.to_radians().sin() * wt.wind.1,
                            wt.wind.0.to_radians().cos() * wt.wind.1,
                            0.0,
                        );
                        // every bus one may ride in keeps the weather out: the own, another
                        // player's, a timetable bus
                        let boxed = |v: &omsi_sim::VehicleInstance| v.ty.def.bounding_box.map(|bb| (v.position, v.heading, bb));
                        let mut buses: Vec<(glam::DVec3, f64, [f32; 6])> = self.player.as_ref().and_then(|p| boxed(&p.vehicle)).into_iter().collect();
                        buses.extend(self.remotes.remotes.values().filter_map(|rv| boxed(rv.vehicle())));
                        if let Some(t) = self.traffic.as_ref() {
                            buses.extend(t.cars.iter().filter(|c| c.is_bus() && (c.vehicle.position - cam.position).length() < 40.0).filter_map(|c| boxed(&c.vehicle)));
                        }
                        let __tr = Instant::now();
                        self.rain.tick(if self.paused { 0.0 } else { dt }, cam.position, wind, scene, &buses);
                        *self.profile.entry("lights.rain").or_default() += __tr.elapsed().as_secs_f64();
                        // wheel splashes through the puddles enhanced.wgsl paints on wet roads
                        if kind == 1 {
                            if let Some(p) = self.player.as_ref() {
                                let wheels = puddles::wheel_contacts(&p.vehicle);
                                let speed = p.vehicle.physics.velocity_kmh().abs() / 3.6;
                                let wetness = self.wetness;
                                scene.smoke.extend(self.splashes.update(
                                    dt,
                                    &wheels,
                                    speed,
                                    &|x, y| {
                                        puddles::puddle_coverage(x, y, w.wet_road_at(x, y, wetness))
                                    },
                                ));
                            }
                        }
                        // the rain heard in the street and the footsteps on the pavement
                        if let (Some(amb), Some(a)) = (self.ambience.as_mut(), self.audio.as_ref())
                        {
                            let steps = self
                                .humans
                                .as_mut()
                                .map(|h| h.take_footfalls())
                                .unwrap_or_default();
                            // what the passengers say, where they stand
                            for line in self.humans.as_mut().map(|h| h.take_voice_lines()).unwrap_or_default() {
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
                                        },
                                    );
                                }
                            }
                            let inside = self.in_cab;
                            let engine_running = self
                                .player
                                .as_ref()
                                .map(|p| omsi_sim::startup::engine_running(&p.vehicle))
                                .unwrap_or(false);
                            let __tm = Instant::now();
                            amb.update(
                                a,
                                dt,
                                (kind, rate),
                                inside,
                                engine_running,
                                street_condition(wt, self.wetness),
                                cam.position,
                                &steps,
                            );
                            *self.profile.entry("lights.ambience").or_default() += __tm.elapsed().as_secs_f64();
                            if let Some(every) = debug_sound_every() {
                                static LAST: std::sync::atomic::AtomicU32 =
                                    std::sync::atomic::AtomicU32::new(u32::MAX);
                                let bucket = (self.clock.time / every as f64) as u32;
                                if LAST.swap(bucket, std::sync::atomic::Ordering::Relaxed) != bucket
                                {
                                    log::info!("sound: environment - {} (precip {kind} {rate:.2}, StreetCond {:.2}, {} voices)", amb.last, street_condition(wt, self.wetness), a.voice_count());
                                }
                            }
                        }
                    }
                }
                *self.profile.entry("lights+rain").or_default() += __t.elapsed().as_secs_f64();
                let __t = Instant::now();
                if let (Some(w), Some(r), Some(scene), Some(cam)) = (
                    self.world.as_ref(),
                    self.renderer.as_ref(),
                    self.scene.as_mut(),
                    self.camera.as_ref(),
                ) {
                    let traffic = self.traffic.as_ref();
                    let phase = |c: usize, li: usize| {
                        traffic.map(|t| t.light_vars(c, li)).unwrap_or((-1.0, 0.0))
                    };
                    let __tb = Instant::now();
                    match self.schedule.as_mut() {
                        Some(s) => s.update_boards(
                            w,
                            traffic,
                            self.duty.as_ref(),
                            self.player
                                .as_ref()
                                .and_then(|p| p.vehicle.host.hof.as_deref()),
                            &self.clock,
                        ),
                        None => w.timetable_boards.lock().clock = Some(self.clock.clone()),
                    }
                    *self.profile.entry("scripted.boards").or_default() += __tb.elapsed().as_secs_f64();
                    w.update_scripted(
                        r,
                        scene,
                        dt,
                        cam.position,
                        daylight.lamps_on,
                        &phase,
                        self.audio.as_ref(),
                        self.in_cab,
                    );
                }
                *self.profile.entry("scripted").or_default() += __t.elapsed().as_secs_f64();
                // (the game menu's lines, for the interface below)
                let menu_lines = if self.game_menu.is_some() { self.game_menu_items() } else { Vec::new() };
                // HUD
                if let (Some(hud), Some(r), Some(scene)) = (
                    self.hud.as_mut(),
                    self.renderer.as_ref(),
                    self.scene.as_mut(),
                ) {
                    // No block of text over the picture (the clock, the bus, the trip and the
                    // key reminder were the old HUD in OMSI's bitmap font; the navigator shows
                    // the trip, the launcher the keys): only what the driver has to act on,
                    // in the interface font, top left.
                    let mut lines: Vec<String> = Vec::new();
                    // why the bus is not moving, whenever the throttle is pressed and nothing
                    // happens: the things a driver checks first
                    if let Some(p) = self.player.as_ref() {
                        lines.extend(standing_reasons(&p.vehicle));
                    }
                    // what is under the cursor, in the player's language (the scripts only
                    // know internal, mostly German names)
                    let names = describe::names(&self.args.root, &self.settings.language);
                    // next to the cursor (`ui`), when the setting asks for it
                    let tooltip = self.hover.as_ref().map(|h| names.control(h));
                    // the object editor's keys, while it is on (one quiet line)
                    if self.editor.is_some() {
                        lines.push("Object editor: click picks · drag moves · wheel turns (Shift lifts) · Del · C copy · V variant · Backspace undo · Ctrl+S save · Esc".into());
                    }
                    if let Some((msg, left)) = self.service_msg.as_mut() {
                        *left -= dt;
                        if *left > 0.0 {
                            lines.push(msg.clone());
                        }
                    }
                    self.service_msg = self.service_msg.take().filter(|(_, l)| *l > 0.0);
                    if let Some(lan) = self.lan.as_ref() {
                        lines.extend(lan::hud_lines(lan, &self.remotes, self.player.as_ref()));
                    }
                    if let Some(h) = self.humans.as_ref() {
                        if let Some(hint) = h.hint() {
                            lines.push(hint);
                        } else if let Some((name, value)) = &h.request {
                            lines.push(format!("Passenger wants: {name}  {value:.2}"));
                        }
                        if let Some((paid, value)) = h.paid {
                            lines.push(format!(
                                "paid: {paid:.2}  (change {:.2})",
                                (paid - value).max(0.0)
                            ));
                        }
                        if let Some(owed) = h.change_due {
                            lines.push(format!("Change due: {owed:.2}"));
                        }
                    }
                    let __t = Instant::now();
                    hud.update(r, scene, &[]);
                    let notes = lines;
                    if let (Some(nav), Some(p), Some(s)) = (self.navigator.as_mut(), self.player.as_ref(), self.surface.as_ref()) {
                        if let Some(w) = self.world.as_ref() {
                            nav.start_map(w.clone());
                        }
                        // stops beyond the loaded tiles: their places from the navigator's map
                        if let (Some(places), Some(d)) = (nav.places(), self.duty.as_mut()) {
                            if !self.duty_places {
                                self.duty_places = true;
                                d.learn_places(places);
                            }
                        }
                        let (line, terminus, stops, trip) = navigator::duty_parts(self.duty.as_ref());
                        match (trip, self.schedule.as_ref(), self.traffic.as_ref(), self.world.as_ref()) {
                            (Some((key, name)), Some(sch), _, _) if nav.map_net().is_some() => {
                                if nav.wants_route(&key, 0) {
                                    let lanes = sch.trip_route_in(nav.map_net().unwrap(), &name);
                                    let g = nav.global_version + (1 << 40);
                                    nav.set_route(&key, lanes, true, g);
                                }
                            }
                            (Some((key, name)), Some(sch), Some(t), Some(w)) => {
                                if nav.wants_route(&key, t.lanes_generation) {
                                    let (lanes, complete) = sch.trip_route(w, t, &name);
                                    nav.set_route(&key, lanes, complete, t.lanes_generation);
                                }
                            }
                            _ => nav.clear_route(),
                        }
                        let frame = navigator::NavFrame {
                            traffic: self.traffic.as_ref(),
                            bus: p.vehicle.position,
                            heading: p.vehicle.heading,
                            speed_kmh: p.vehicle.physics.velocity_kmh(),
                            line,
                            terminus,
                            stops,
                            delay: self.duty.as_ref().map(|_| p.vehicle.host.tt_delay as f64),
                            passengers: self.humans.as_ref().map(|h| h.riding()),
                            time: self.clock.time,
                            weekday: self.clock.weekday(),
                            language: &self.settings.language,
                            screen: (s.config.width as f32, s.config.height as f32),
                            dt,
                        };
                        let __tn = Instant::now();
                        nav.frame(r, scene, &frame);
                        *self.profile.entry("hud.navigator").or_default() += __tn.elapsed().as_secs_f64();
                        // OMSI 2's dynamic route arrows over the junctions ahead
                        if nav.arrows {
                            if let Some(w) = self.world.as_ref() {
                                let spots = nav.arrow_spots(self.traffic.as_ref().map(|t| &t.net), 350.0, &|id| w.object_positions.lock().get(&id).map(|p| (p.0, p.1[0])));
                                self.route_arrows.tick(dt, w, r, scene, &spots);
                            }
                        }
                    }
                    if let (Some(ui), Some(s)) = (self.ui.as_mut(), self.surface.as_ref()) {
                        let scale = self.window.as_ref().map(|w| w.scale_factor() as f32).unwrap_or(1.0);
                        let (w, h) = (s.config.width as f32, s.config.height as f32);
                        self.remotes.chat.disabled = !self.settings.chat;
                        let chat = (self.lan.is_some() && self.settings.chat).then(|| ui::ChatView {
                            lines: &self.remotes.chat.lines,
                            typing: self.remotes.chat.typing.as_deref(),
                            error: self.remotes.chat.error(),
                        });
                        ui.chat.hidden = self.remotes.chat.hidden;
                        let tags = if self.settings.name_tags {
                            self.camera.as_ref().map(|c| lan::name_tags(&self.remotes, c, w, h)).unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        // the vehicle chooser shows its vehicles in the menu's place (the menu
                        // scrolls a long list)
                        let chooser_list = self.admin_list.as_ref().unwrap_or(&self.vehicle_list);
                        let (chooser_items, chooser_sel): (Vec<(&str, &str)>, Option<usize>) = match self.chooser {
                            Some(sel) => {
                                let items = chooser_list.iter().map(|(name, path)| (path.as_str(), name.as_str())).collect();
                                (items, Some(sel))
                            }
                            None => (Vec::new(), None),
                        };
                        let frame = ui::Frame {
                            scale,
                            width: w,
                            height: h,
                            cursor: self.cursor,
                            vr: {
                                #[cfg(windows)] { self.vr.is_some() }
                                #[cfg(not(windows))] { false }
                            },
                            tooltip: tooltip.filter(|_| self.settings.tooltips && !self.dragging),
                            notes: &notes,
                            fps: self.settings.show_fps.then_some(self.fps),
                            paused: self.paused,
                            menu: match chooser_sel {
                                Some(k) => Some((k, &chooser_items[..])),
                                None => self.game_menu.map(|k| (k, &menu_lines[..])),
                            },
                            menu_top: self.menu_top,
                            timetable: self.timetable.then(|| timetable_rows(self.duty.as_ref(), self.player.as_ref().map(|p| p.vehicle.host.tt_delay as f64))).flatten(),
                            info: self.info_bar.then(|| info_line(&self.clock, self.player.as_ref(), self.duty.as_ref())),
                            tutorial: self.tutorial.as_ref().filter(|t| !t.hidden).and_then(|t| t.page().map(|p| (p.title.as_str(), p.text.as_str(), p.image.as_deref(), t.at, t.pages.len()))),
                            chat,
                            tags,
                        };
                        ui.draw(r, scene, &frame, dt);
                    }
                    *self.profile.entry("hud").or_default() += __t.elapsed().as_secs_f64();
                }

                let mut lighting = match self.weather.as_ref() {
                    Some(w) => {
                        self.wetness = road_wetness(precip_of(w).1, dt as f64, self.wetness);
                        weather_lighting(
                            &daylight,
                            w,
                            // (on over midnight, for the clouds' drift)
                            self.clock.time + self.clock.day_of_year as f64 * 86400.0,
                            self.wetness,
                            self.settings.shadows,
                        )
                    }
                    None => lights::lighting_from(&daylight, 50000.0),
                };
                lighting.wetness = omsi_cfg::env::var("OMSI_WETNESS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(self.wetness);
                lighting.inside = match self.inside_remote.and_then(|id| self.remotes.remotes.get(&id)) {
                    // (in another player's bus: its box is the one the camera is in)
                    Some(rv) => rv.vehicle().ty.def.bounding_box.map(|bb| (rv.vehicle().position, rv.vehicle().heading, bb)),
                    None => self.player.as_ref().and_then(|p| {
                        p.vehicle
                            .ty
                            .def
                            .bounding_box
                            .map(|bb| (p.vehicle.position, p.vehicle.heading, bb))
                    }),
                };
                lighting.detail = self.settings.detail_textures;
                lighting.glass_wind = self.player.as_ref().map(|p| crate::lights::vehicle_velocity(&p.vehicle)).unwrap_or_default();
                // an LED panel's dots burn this much above their own colour (16 levels,
                // see `Settings::led_glow`); the masks keep their mip chain unless the
                // player asks for the sharper look (`Settings::led_mips`)
                lighting.led_glow = self.settings.led_glow as f32 * 0.25;
                lighting.led_mips = self.settings.led_mips;
                let mut finish = false;
                let mut reconfigure = false;
                let shot = self.shot.take();
                if let Some(s) = self.surface.as_ref() {
                    let (w, h) = (s.config.width, s.config.height);
                    self.touch_prepare(w, h);
                }
                if let (Some(s), Some(r), Some(scene), Some(cam), Some(win)) = (
                    self.surface.as_ref(),
                    self.renderer.as_mut(),
                    self.scene.as_mut(),
                    self.camera.as_ref(),
                    self.window.as_ref(),
                ) {
                    // `shot <file>` from the input script: the scene the window is showing,
                    // from its camera and lighting, into a PNG - the only way to look at
                    // what an automated window run draws (also when the window is hidden,
                    // so it does not depend on a frame being acquired)
                    if let Some(path) = shot {
                        match r.render_to_image(
                            scene,
                            s.config.width,
                            s.config.height,
                            cam,
                            &lighting,
                        ) {
                            Ok(mut px) => match {
                                // (with the on-screen controls, when there are)
                                if let Some(over) = self.touch.picture(r, s.config.width, s.config.height) {
                                    crate::touch::composite(&mut px, &over);
                                }
                                image::save_buffer(
                                &path,
                                &px,
                                s.config.width,
                                s.config.height,
                                image::ColorType::Rgba8,
                            ) } {
                                Ok(()) => log::info!(
                                    "input script: window picture written to {}",
                                    path.display()
                                ),
                                Err(e) => log::warn!(
                                    "input script: {} could not be written: {e}",
                                    path.display()
                                ),
                            },
                            Err(e) => log::warn!(
                                "input script: the window picture could not be rendered: {e}"
                            ),
                        }
                    }
                    // A window that is hidden (another app covers it, another Space) gets
                    // no frames on macOS. OMSI_RENDER_OCCLUDED=1 draws them into a texture
                    // of the window's size anyway and waits for the GPU as a present would,
                    // so frame times can be measured with the window out of sight.
                    let __t = Instant::now();
                    // OMSI_HIDE_WINDOW=from,to: treat the window as hidden between these
                    // seconds of the session (the frame is acquired and dropped unshown), to
                    // check the hidden-window path without covering the window by hand
                    let hide_test = omsi_cfg::env::var("OMSI_HIDE_WINDOW").ok().and_then(|v| {
                        let mut it = v.split(',').filter_map(|x| x.trim().parse::<f32>().ok());
                        Some((it.next()?, it.next()?))
                    });
                    let hidden_now = hide_test
                        .map(|(a, b)| (a..b).contains(&self.started.elapsed().as_secs_f32()))
                        .unwrap_or(false);
                    let acquired = match s.surface.get_current_texture() {
                        wgpu::CurrentSurfaceTexture::Success(_)
                        | wgpu::CurrentSurfaceTexture::Suboptimal(_)
                            if hidden_now =>
                        {
                            wgpu::CurrentSurfaceTexture::Occluded
                        }
                        other => other,
                    };
                    let (frame, stand_in) = match acquired {
                        wgpu::CurrentSurfaceTexture::Success(frame)
                        | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (Some(frame), None),
                        wgpu::CurrentSurfaceTexture::Occluded
                            if omsi_cfg::env::var_os("OMSI_RENDER_OCCLUDED").is_some() =>
                        {
                            let (w, h) = (s.config.width, s.config.height);
                            if self
                                .stand_in
                                .as_ref()
                                .map(|t| (t.width(), t.height()) != (w, h))
                                .unwrap_or(true)
                            {
                                self.stand_in =
                                    Some(r.device.create_texture(&wgpu::TextureDescriptor {
                                        label: Some("hidden window"),
                                        size: wgpu::Extent3d {
                                            width: w,
                                            height: h,
                                            depth_or_array_layers: 1,
                                        },
                                        mip_level_count: 1,
                                        sample_count: 1,
                                        dimension: wgpu::TextureDimension::D2,
                                        format: r.format(),
                                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                                        view_formats: &[],
                                    }));
                            }
                            (
                                None,
                                self.stand_in
                                    .as_ref()
                                    .map(|t| t.create_view(&Default::default())),
                            )
                        }
                        wgpu::CurrentSurfaceTexture::Outdated
                        | wgpu::CurrentSurfaceTexture::Lost => {
                            reconfigure = true;
                            (None, None)
                        }
                        _ => (None, None),
                    };
                    *self.profile.entry("acquire").or_default() += __t.elapsed().as_secs_f64();
                    if frame.is_none() {
                        self.hidden_frames += 1;
                    }
                    let view = frame
                        .as_ref()
                        .map(|f| f.texture.create_view(&Default::default()))
                        .or(stand_in);
                    if let Some(view) = view {
                        let __t = Instant::now();
                        // One mirror a turn, in turn, at most MIRROR_RATE pictures a second in
                        // all: a mirror costs half the main picture's CPU time, and at 140 fps
                        // five mirrors were each redrawn 28 times a second, a small picture
                        // that nobody can tell from 15.
                        // Every mirror at least MIRROR_MIN_HZ, though: with eight of them (the
                        // Procity) at 25 fps each was redrawn three times a second, and the
                        // street jerked past in them - up to two a frame then (each costs a
                        // few milliseconds of the frame).
                        let mirrors = self.player.as_ref().map(|p| p.vehicle.ty.def.cameras_reflexion.len()).unwrap_or(0) as f32;
                        let rate = {
                            #[cfg(windows)]
                            let vr_active = self.vr.is_some();
                            #[cfg(not(windows))]
                            let vr_active = false;
                            if vr_active {
                                // Each VR frame already draws two full-size eyes. Keep bus
                                // mirrors useful without spending two more scene renders
                                // on nearly every frame when the headset is below refresh.
                                omsi_cfg::env::var("OMSI_OPENXR_MIRROR_RATE")
                                    .ok()
                                    .and_then(|s| s.parse::<f32>().ok())
                                    .filter(|rate| rate.is_finite() && *rate >= 0.0)
                                    .unwrap_or(self.settings.vr_mirror_rate)
                            } else {
                                MIRROR_RATE.max(mirrors * MIRROR_MIN_HZ).min(MIRROR_MAX_HZ * self.mirrors_seen.max(1) as f32)
                            }
                        };
                        self.mirror_budget = (self.mirror_budget + raw_dt.min(0.1) * rate).min(2.5);
                        let mut drawn = 0;
                        // (in the cab, and from outside too while the bus is near: its
                        // mirrors are seen from the pavement and stood frozen)
                        let near = self.player.as_ref().zip(self.camera.as_ref()).is_some_and(|(p, c)| (p.vehicle.position - c.position).length() < 12.0);
                        while (self.in_cab || near) && self.mirror_budget >= 1.0 && drawn < self.mirrors_seen.clamp(1, 2) {
                            let (Some(w), Some(p)) = (self.world.as_ref(), self.player.as_ref()) else { break };
                            self.mirror_budget -= 1.0;
                            drawn += 1;
                            self.mirror_turn = self.mirror_turn.wrapping_add(1);
                            self.mirrors_seen = render_mirrors(
                                r,
                                scene,
                                w,
                                p,
                                &lighting,
                                Some(self.mirror_turn),
                                Some((*cam, s.config.width as f32 / s.config.height.max(1) as f32)),
                            );
                        }
                        *self.profile.entry("mirrors").or_default() += __t.elapsed().as_secs_f64();
                        let __t = Instant::now();
                        #[cfg(windows)]
                        let mut mirrored = false;
                        #[cfg(not(windows))]
                        let mirrored = false;
                        #[cfg(windows)]
                        if let Some(vr) = self.vr.as_mut() {
                            let menu_range = self.ui.as_ref().map(|u| u.menu_overlay_range.clone()).unwrap_or(0..0);
                            let cursor_overlay = self.ui.as_ref().and_then(|u| u.vr_cursor_overlay);
                            let tooltip_overlay = self.ui.as_ref().and_then(|u| u.vr_tooltip_overlay);
                            match vr.render(
                                r,
                                scene,
                                cam,
                                &lighting,
                                &view,
                                (s.config.width, s.config.height),
                                menu_range,
                                cursor_overlay,
                                tooltip_overlay,
                                self.cursor,
                                self.player.as_ref().map(|p| (p.vehicle.position, p.vehicle.body_rotation())),
                                self.settings.vr_head_smoothing_ms,
                                !self.mouse_drive,
                                self.vr_zoom_active,
                            ) {
                                Ok(visible) => mirrored = visible,
                                Err(e) => {
                                    log::error!("OpenXR rendering stopped: {e:#}");
                                    self.vr = None;
                                }
                            }
                        }
                        if !mirrored {
                            r.render(
                                scene,
                                &view,
                                s.config.width,
                                s.config.height,
                                cam,
                                &lighting,
                            );
                        }
                        // the on-screen controls over the picture (a phone)
                        self.touch.render(r, &view, s.config.width, s.config.height);
                        *self.profile.entry("render").or_default() += __t.elapsed().as_secs_f64();
                        if omsi_cfg::env::var_os("OMSI_PROFILE_GPU").is_some() {
                            // wait for the GPU here, so that its time shows as a stage of its own
                            let __t = Instant::now();
                            let _ = r.device.poll(wgpu::PollType::Wait {
                                submission_index: None,
                                timeout: None,
                            });
                            *self.profile.entry("gpu").or_default() += __t.elapsed().as_secs_f64();
                        }
                        let __t = Instant::now();
                        match frame {
                            Some(frame) => {
                                // (without V-sync max_fps paces the frames: waiting for the compositor's frame callback cost a missed refresh each slow frame)
                                if self.settings.vsync {
                                    win.pre_present_notify();
                                }
                                frame.present();
                            }
                            None => {
                                let _ = r.device.poll(wgpu::PollType::Wait {
                                    submission_index: None,
                                    timeout: None,
                                });
                            }
                        }
                        *self.profile.entry("present").or_default() += __t.elapsed().as_secs_f64();
                    } else {
                        // Nothing to draw into (a hidden window): the simulation goes on at
                        // a display's pace instead of spinning a core a thousand times a
                        // second. What it uploaded (traffic instances, streamed tiles,
                        // people, the navigator) waits in wgpu's staging buffers until the
                        // next submit, so submit nothing to let them go: without it a hidden
                        // window on Ahlheim grew by 100 MB a second (5.6 GB after 55 s).
                        let __t = Instant::now();
                        r.queue.submit(std::iter::empty::<wgpu::CommandBuffer>());
                        let _ = r.device.poll(wgpu::PollType::Poll);
                        *self.profile.entry("present").or_default() += __t.elapsed().as_secs_f64();
                        if let Some(rest) =
                            std::time::Duration::from_millis(16).checked_sub(now.elapsed())
                        {
                            std::thread::sleep(rest);
                        }
                    }
                    // max_fps (the original's [maxFPS]; OMSI_MAX_FPS for a test): the rest of
                    // the frame's time is slept, not spun, so a limit gives the CPU back
                    // (and keeps a laptop cool enough not to slow itself down)
                    let max_fps = omsi_cfg::env::var("OMSI_MAX_FPS")
                        .ok()
                        .and_then(|v| v.parse::<u32>().ok())
                        .unwrap_or(self.settings.max_fps);
                    // 0 = the screen's refresh rate: frames the screen never shows only heat the
                    // machine (with V-sync off and no limit an M4 drew 300 frames a second in the
                    // depot and ran hot); 1000 and more = no limit at all
                    let max_fps = if max_fps == 0 {
                        self.window.as_ref().and_then(|w| w.current_monitor()).and_then(|m| m.refresh_rate_millihertz()).map(|mhz| (mhz as f64 / 1000.0).round() as u32).filter(|r| *r >= 30).unwrap_or(120)
                    } else if max_fps >= 1000 {
                        0
                    } else {
                        max_fps
                    };
                    #[cfg(windows)]
                    let vr_active = self.vr.is_some();
                    #[cfg(not(windows))]
                    let vr_active = false;
                    if max_fps > 0 && !vr_active {
                        let __t = Instant::now();
                        if let Some(rest) = std::time::Duration::from_secs_f64(1.0 / max_fps as f64)
                            .checked_sub(now.elapsed())
                        {
                            std::thread::sleep(rest);
                        }
                        *self.profile.entry("limiter").or_default() += __t.elapsed().as_secs_f64();
                    }
                    self.frames += 1;
                    let profiling = omsi_cfg::env::var_os("OMSI_PROFILE").is_some();
                    if profiling
                        && self.cpu_mark.is_none()
                        && self.started.elapsed().as_secs_f32() > 15.0
                    {
                        self.cpu_mark =
                            process_cpu_seconds().map(|c| (c, Instant::now(), self.total_frames));
                    }
                    if let (Some(limit), false) = (self.args.exit_after, self.exiting) {
                        if self.started.elapsed().as_secs_f32() > limit {
                            self.exiting = true;
                            log::info!("exit after {limit} s: {} frames total ({} with the window hidden{}), {:.1} fps average, {} frames over 50 ms, worst {:.0} ms", self.total_frames, self.hidden_frames, if omsi_cfg::env::var_os("OMSI_RENDER_OCCLUDED").is_some() { ", drawn off-screen" } else { ", not drawn" }, self.total_frames as f32 / self.started.elapsed().as_secs_f32(), self.spikes, self.worst_ms);
                            if let (Some(st), Some(w)) =
                                (self.streamer.as_ref(), self.world.as_ref())
                            {
                                log::info!("tile streaming: {} tiles loaded now, {} loaded and {} unloaded in all, {:.1} s preparing on the worker, slowest upload {:.0} ms, streaming over 16 ms in {} frames (worst {:.0} ms); {} objects + {} trees, {} rows, {} attached ({} without parent), {} unresolved", w.loaded_tiles().len(), st.loaded_total, st.unloaded_total, st.prepare_secs, st.worst_upload_ms, st.slow_frames, st.worst_frame_ms, st.stats.objects, st.stats.trees, st.stats.rows, st.stats.attached, st.stats.unattached, st.stats.failed_objects);
                                st.stats.log_ground();
                            }
                            if omsi_cfg::env::var_os("OMSI_PROFILE").is_some() {
                                let n = self.total_frames.max(1) as f64;
                                for (k, v) in &self.profile {
                                    log::info!("profile {k:10}: {:.1} ms/frame", v / n * 1000.0);
                                }
                                if let Some(h) = self.humans.as_ref() {
                                    log::info!(
                                        "profile people: {} ({})",
                                        h.people.len(),
                                        h.summary()
                                    );
                                }
                                for (k, v) in r.stats.borrow().iter() {
                                    log::info!(
                                        "profile render.{k:10}: {:.2} ms/frame",
                                        v / n * 1000.0
                                    );
                                }
                                for (k, v) in r.counts.borrow().iter() {
                                    log::info!("profile count {k}: {:.0} a frame", v / n);
                                }
                                for (pass, ms, frames) in r.gpu_pass_times() {
                                    log::info!("profile gpu pass {pass:12}: {ms:.2} ms ({frames} frames measured)");
                                }
                                if let (Some((c0, t0, f0)), Some(c1)) =
                                    (self.cpu_mark, process_cpu_seconds())
                                {
                                    let frames = self.total_frames.saturating_sub(f0).max(1) as f64;
                                    log::info!("profile: since 15 s {:.1} ms wall and {:.1} ms CPU (all threads) per frame, {:.1} cores busy", t0.elapsed().as_secs_f64() / frames * 1000.0, (c1 - c0) / frames * 1000.0, (c1 - c0) / t0.elapsed().as_secs_f64().max(1e-3));
                                }
                                let (sw, sh) = r.scene_size(s.config.width, s.config.height);
                                log::info!(
                                    "profile: window {}x{}, scene drawn at {sw}x{sh}, {}x MSAA",
                                    s.config.width,
                                    s.config.height,
                                    r.options.msaa
                                );
                            }
                            finish = true;
                            crate::platform::exit(event_loop);
                        }
                    }
                    self.total_frames += 1;
                    if self.fps_t.elapsed().as_secs_f32() >= 1.0 {
                        if omsi_cfg::env::var_os("OMSI_PROFILE").is_some() {
                            let secs = self.fps_t.elapsed().as_secs_f32();
                            log::info!("profile interval: {:.1} fps over {secs:.2} s", self.frames as f32 / secs);
                        }
                        let speed = self
                            .player
                            .as_ref()
                            .map(|p| format!(" - {:.0} km/h", p.vehicle.physics.velocity_kmh()))
                            .unwrap_or_default();
                        self.fps = self.frames as f32;
                        win.set_title(&format!(
                            "openOMSI - {} fps{speed} - {:.0},{:.0},{:.0} yaw {:.0}",
                            self.frames,
                            cam.position.x,
                            cam.position.y,
                            cam.position.z,
                            cam.yaw.rem_euclid(360.0)
                        ));
                        self.frames = 0;
                        self.fps_t = Instant::now();
                    }
                    win.request_redraw();
                }
                if reconfigure {
                    // the drawable went away under us (display change, lost surface)
                    if let (Some(s), Some(r), Some(win)) = (
                        self.surface.as_mut(),
                        self.renderer.as_ref(),
                        self.window.as_ref(),
                    ) {
                        let size = win.inner_size();
                        s.resize(r, size.width, size.height);
                    }
                }
                if finish {
                    self.finish_session();
                }
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if matches!(&event, DeviceEvent::Added | DeviceEvent::Removed) {
            if let Some(controllers) = self.controllers.as_ref() {
                controllers.refresh_devices();
            }
        }
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.mouse_look {
                self.look_by(delta.0 as f32 * 0.15, delta.1 as f32 * 0.15);
            } else if self.mouse_drive && self.game_menu.is_none() {
                self.mouse_past_edge(delta.0 as f32);
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.mouse_edge != 0.0 && !self.mouse_drive {
            self.mouse_edge = 0.0;
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// The only user event: a quit signal arrived (see quit.rs).
    fn user_event(&mut self, event_loop: &ActiveEventLoop, _event: ()) {
        if let Some(sig) = quit::requested() {
            log::info!("{} received: ending the session", quit::signal_name(sig));
            self.finish_session();
            crate::platform::exit(event_loop);
        }
    }

    /// Every way out ends here (Escape, the window's close button, Cmd+Q, --exit-after, a
    /// quit signal): the session is written and the LAN peers hear that we left, before
    /// anything else is torn down (Cmd+Q ends the process without returning from the loop).
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.finish_session();
        if let Some(lan) = self.lan.take() {
            // dropping the session says goodbye (BYE) to the host or the players
            drop(lan);
            log::info!("LAN: left the session");
        }
        // the tunnel's cloudflared and the WebSocket gateway go with the game (kept in a
        // static, which Rust never drops: cloudflared outlived every session, holding the
        // port and a public tunnel open)
        crate::lan::close_public_gateway();
        // and the launcher's LAN status file goes (Cmd+Q never returns to main's guard)
        drop(lan::StatusFileGuard);
        log::info!("game ends");
    }
}

impl App {
    /// The mouse wheel (or a pinch of two fingers): `amount` notches, up positive.
    pub(crate) fn wheel(&mut self, amount: f32) {
        // the object editor: the wheel turns (Shift: lifts) the object
        if self.game_menu.is_none() && self.editor_wheel(amount) {
            return;
        }
        // placing a vehicle: the wheel turns it
        if self.placing.is_some() && self.game_menu.is_none() {
            self.placing_wheel(amount);
            return;
        }
        // the game menu and its lists scroll with the wheel
        if self.game_menu.is_some() {
            self.menu_wheel(amount);
            return;
        }
        // the city map takes the wheel while it is open
        if let Some(n) = self.navigator.as_mut().filter(|n| n.map_open()) {
            n.map_wheel(amount, self.cursor.0, self.cursor.1);
            return;
        }
        // the wheel over the chat (or while typing) scrolls its history
        if let Some(ui) = self.ui.as_mut() {
            if self.lan.is_some() && (ui.chat.hovered || lan::chat_open(&self.remotes)) {
                ui.chat.wheel(self.remotes.chat.lines.len(), amount);
                return;
            }
        }
        // The wheel over a cockpit switch turns it: the same <event>_drag the
        // original fires while the mouse is dragged, with the notch as the
        // movement. Knobs, the sun blind and the ignition key are far easier to
        // set that way than by holding the button down and moving the mouse.
        if self.hover.is_some() && self.view != "free" {
            let ray = self.camera.as_ref().zip(self.surface.as_ref())
                .map(|(cam, s)| self.cockpit_cursor_ray(cam, (s.config.width, s.config.height)));
            if let (Some(p), Some((o, d, spread))) = (
                self.player.as_mut(),
                ray,
            ) {
                if p.pick(o, d, spread).is_some() {
                    // a notch is worth a good push of the mouse: the scripts divide
                    // the movement by 10 (the ignition key), 200 (the parking brake)
                    // or 500 (the driver's window), so a few pixels would do nothing
                    p.wheel(o, d, spread, -amount * 40.0);
                    return;
                }
            }
        }
        let ctrl = self.keys.contains(&KeyCode::ControlLeft) || self.keys.contains(&KeyCode::ControlRight);
        if self.view == "outside" && self.player.is_some() && ctrl {
            // Ctrl+wheel: the outside camera stays where it is and narrows its field of view
            // (a telephoto; OMSI's own zoom there only moves the camera, as the wheel does)
            self.zoom_by(amount);
        } else if self.view == "outside" && self.player.is_some() {
            self.orbit = (self.orbit - amount * 1.5).clamp(ORBIT_MIN, ORBIT_MAX);
        } else if matches!(self.view.as_str(), "driver" | "pax") && self.player.is_some() {
            // inside the bus the wheel zooms, as in OMSI (the camera itself stays in the seat)
            self.zoom_by(amount);
        } else if matches!(self.view.as_str(), "free" | "foot") && !ctrl {
            // the free camera and on foot: the wheel zooms too (Ctrl+wheel moves the free
            // camera on, as the wheel alone did)
            self.zoom_by(amount);
        } else if let Some(cam) = self.camera.as_mut() {
            let f = cam.forward();
            cam.position += (f * amount * 4.0).as_dvec3();
        }
    }

    /// The left mouse button (or a finger's tap) where the cursor is.
    pub(crate) fn left_button(&mut self, event_loop: &ActiveEventLoop, pressed: bool) {
        let state = if pressed { ElementState::Pressed } else { ElementState::Released };
        // placing a vehicle: a click sets it down
        if self.placing.is_some() && self.game_menu.is_none() {
            if state == ElementState::Pressed {
                self.placing_click();
            }
            return;
        }
        // the game menu takes the clicks while it is open
        if self.game_menu.is_some() {
            // Releasing the mouse button finishes scrollbar dragging.
            if state == ElementState::Released {
                if self.menu_scroll_drag {
                    self.menu_scroll_drag = false;
                    self.menu_top = self.menu_top.map(f32::round);
                }
                return;
            }

            // Pressing the mouse button on the scrollbar thumb starts dragging.
            if state == ElementState::Pressed {
                if let Some(thumb) = self
                    .ui
                    .as_ref()
                    .and_then(|u| u.menu_scroll_thumb)
                {
                    if self.cursor.0 >= thumb[0]
                        && self.cursor.0 <= thumb[2]
                        && self.cursor.1 >= thumb[1]
                        && self.cursor.1 <= thumb[3]
                    {
                        self.menu_scroll_drag = true;
                        return;
                    }
                }

                // Otherwise check whether a menu row was clicked.
                let hit = self.ui.as_ref().and_then(|u| {
                    u.menu_rects.iter().position(|r| {
                        self.cursor.0 >= r[0]
                            && self.cursor.0 <= r[2]
                            && self.cursor.1 >= r[1]
                            && self.cursor.1 <= r[3]
                    })
                });

                if let Some(k) = hit {
                    let k = k
                        + self
                            .ui
                            .as_ref()
                            .map(|u| u.menu_start)
                            .unwrap_or(0);

                    if self.chooser.is_none() {
                        self.game_menu = Some(k);
                    }

                    self.menu_choose(event_loop, k);
                }
            }

            return;
        }
        self.on_left(state == ElementState::Pressed)
    }
}

/// OMSI's timetable window: the current trip's stops with their times, the ones served
/// greyed, the next one marked.
fn timetable_rows(duty: Option<&crate::schedule::PlayerDuty>, delay: Option<f64>) -> Option<(String, Vec<(String, String, u8)>)> {
    let d = duty?;
    let trip = d.trips.get(d.trip_index)?;
    let hm = |t: f64| format!("{:02}:{:02}", ((t / 3600.0) as i64).rem_euclid(24), ((t % 3600.0) / 60.0) as i64);
    let delay = delay.unwrap_or(0.0);
    let title = format!(
        "{} › {}   {}{}:{:02}   ({}/{})",
        if trip.line.trim().is_empty() { d.line.trim() } else { trip.line.trim() },
        trip.terminus.trim(),
        if delay < 0.0 { "−" } else { "+" },
        (delay.abs() / 60.0) as i64,
        (delay.abs() % 60.0) as i64,
        d.trip_index + 1,
        d.trips.len()
    );
    // (as a driver's paper timetable: the departure, the arrival at the last stop; a stop
    // with a wait shows both)
    let last = trip.stops.iter().rposition(|s| s.stops);
    let mut rows: Vec<(String, String, u8)> = trip
        .stops
        .iter()
        .enumerate()
        .filter(|(_, s)| s.stops)
        .map(|(k, s)| {
            let time = if Some(k) == last { hm(s.arr) } else if s.dep - s.arr >= 60.0 { format!("{}-{}", hm(s.arr), &hm(s.dep)[3..]) } else { hm(s.dep) };
            (s.name.trim().to_string(), time, if k < d.next_stop { 0 } else if k == d.next_stop { 1 } else { 2 })
        })
        .collect();
    // the trip after this one
    if let Some(next) = d.trips.get(d.trip_index + 1) {
        let name = format!("› {} {}", if next.line.trim().is_empty() { d.line.trim() } else { next.line.trim() }, next.terminus.trim());
        rows.push((name, hm(next.departure), 0));
    }
    Some((title, rows))
}

/// OMSI's information bar: the time, the speed, and the trip with its next stop and delay.
fn info_line(clock: &omsi_sim::SimClock, player: Option<&Player>, duty: Option<&crate::schedule::PlayerDuty>) -> String {
    let t = clock.time;
    let mut parts = vec![format!("{:02}:{:02}:{:02}", ((t / 3600.0) as i64).rem_euclid(24), ((t % 3600.0) / 60.0) as i64, (t % 60.0) as i64)];
    if let Some(p) = player {
        parts.push(format!("{:.0} km/h", p.vehicle.physics.velocity_kmh().abs()));
        // the tank as the bus's script says it (OMSI's RL_TankContent: tank_percent)
        if let Some(tank) = p.vehicle.var("tank_percent").filter(|v| v.is_finite()) {
            parts.push(format!("tank {:.0} %", (tank * 100.0).round()));
        }
        if let Some(d) = duty {
            if let Some(trip) = d.trips.get(d.trip_index) {
                let line = if trip.line.trim().is_empty() { d.line.trim() } else { trip.line.trim() };
                parts.push(format!("{line} › {}", trip.terminus.trim()));
                if let Some(s) = trip.stops.get(d.next_stop) {
                    parts.push(format!("next: {}", s.name.trim()));
                }
                let delay = p.vehicle.host.tt_delay;
                parts.push(format!("{}{}:{:02}", if delay < 0.0 { "−" } else { "+" }, (delay.abs() / 60.0) as i64, (delay.abs() % 60.0) as i64));
            }
        }
    }
    parts.join("   ·   ")
}

#[cfg(test)]
mod governor_tests {
    use super::render_scale_step;

    #[test]
    fn cpu_stutters_do_not_reduce_picture_quality() {
        assert!(render_scale_step(35.0, 0.1) > 0.0);
        assert!(render_scale_step(35.0, 0.6) < 0.0);
        assert!(render_scale_step(60.0, 0.6) > 0.0);
    }
}
