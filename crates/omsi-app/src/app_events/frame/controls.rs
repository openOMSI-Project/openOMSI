//! The game controllers, the mouse steering and the controllers' buttons in the window's
//! frame.

use super::*;

impl App {
    /// The tutorial's pages, then the game controllers: their axes this frame (returned with
    /// their buttons' key actions), the cursor hidden while they drive, their force feedback.
    pub(super) fn frame_controllers(&mut self, dt: f32) -> (crate::controllers::Analog, Vec<(String, bool)>) {
        // the tutorial's pages, once the world is there
        if self.world.is_some() {
            if let Some(n) = self.args.tutorial.take() {
                self.menus.tutorial = crate::tutorial::Tutorial::load(&self.args.root, n, &self.settings.language);
            }
        }
        // the game controllers: their axes this frame, their buttons' key actions
        let hwnd = self.window.as_deref().and_then(crate::controllers::window_handle);
        let ctl = self.input.controllers.get_or_insert_with(|| crate::controllers::Controllers::new(&self.args.root, hwnd));
        ctl.set_focus(self.input.window_focused);
        ctl.deadzone = self.settings.ctrl_deadzone;
        ctl.right_stick_look = self.settings.right_stick_look;
        ctl.pad_deadzone = self.settings.pad_deadzone;
        ctl.pad_presets = self.settings.pad_buttons;
        ctl.pedal_throttle = self.settings.pedal_throttle;
        ctl.pedal_brake = self.settings.pedal_brake;
        ctl.ff_invert = self.settings.ff_invert;
        ctl.ff_enabled = self.settings.ff_enabled;
        ctl.ff_road = self.settings.ff_road_vib;
        ctl.ff_engine = self.settings.ff_engine_vib;
        ctl.ff_fade = self.settings.ff_fade;
        ctl.steer_gain = if self.settings.wheel_lock >= 45.0 { (self.settings.wheel_range / self.settings.wheel_lock).clamp(0.1, 20.0) } else { 1.0 };
        ctl.disabled = self.settings.ctrl_off.split('|').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        ctl.set_editing(self.menus.game_menu.is_some() || self.menus.chooser.is_some());
        let analog = ctl.poll();
        let actions = std::mem::take(&mut ctl.actions);
        let moved = match (analog.steering, self.input.last_ctl_steer) {
            (Some(x), Some(x0)) => (x - x0).abs() > 0.02,
            _ => false,
        };
        if analog.steering.is_some() && (moved || self.input.last_ctl_steer.is_none()) {
            self.input.last_ctl_steer = analog.steering;
        }
        #[cfg(windows)]
        let vr_on = self.xr.vr.is_some();
        #[cfg(not(windows))]
        let vr_on = false;
        let needs_mouse = self.input.mouse_drive
            || self.menus.game_menu.is_some()
            || self.menus.chooser.is_some()
            || self.menus.list_kind.is_some()
            || self.menus.navigator.as_ref().is_some_and(|n| n.map_open())
            || crate::plugin_ui::focused(&self.integrations.plugins)
            || !matches!(self.view.as_str(), "driver" | "outside" | "pax");
        let hide = (moved || actions.iter().any(|a| a.1)) && !needs_mouse && !vr_on;
        if self.xr.vr_nav_edit.is_none() && hide != self.input.cursor_hidden.is_some() && (hide || needs_mouse) {
            if let Some(win) = self.window.as_ref() {
                win.set_cursor_visible(!hide);
                self.input.cursor_hidden = hide.then_some(self.input.cursor);
            }
        }
        if let Some(n) = ctl.notice.take() {
            self.service_msg = Some((n, 8.0));
        }
        // the bus's force feedback (OMSI's FF_Vib_Amp, and on a wheel its forces)
        // (in every view of the bus - the wheel went slack outside and in the
        // passenger view)
        let driving = self.player.as_ref().filter(|_| matches!(self.view.as_str(), "driver" | "outside" | "pax") && !self.paused);
        let kmh = driving.map(|p| p.vehicle.physics.velocity_kmh()).unwrap_or(0.0);
        let wheel_bump = ctl.wheel_bump(driving.and_then(|p| p.vehicle.rigid.as_ref()), kmh, dt);
        ctl.feedback(crate::controllers::FfInput {
            on: driving.is_some(),
            kmh,
            lateral_accel: driving.and_then(|p| p.vehicle.rigid.as_ref()).map(|r| r.accel_body.x).unwrap_or(0.0),
            wheel_bump,
            wheel_bump_age: 0.0,
            vib_amp: driving.and_then(|p| p.vehicle.var("FF_Vib_Amp")).unwrap_or(0.0),
            vib_period: driving.and_then(|p| p.vehicle.var("FF_Vib_Period")).unwrap_or(0.0),
            // what the bus is standing on and running on: the road's own grain
            // (`StreetCond`: 0 dry, 1 wet, 2 snow) and the engine
            street_cond: driving.map(|p| p.vehicle.host.street_cond).unwrap_or(0.0),
            engine_rpm: driving.and_then(|p| omsi_sim::startup::engine_rpm(&p.vehicle)).unwrap_or(0.0),
            engine_load: driving.map(|p| p.vehicle.physics.controls.throttle.clamp(0.0, 1.0)).unwrap_or(0.0),
            micro: 0.0,
            dt,
        });
        crate::game_controller_menu::frame(self);
        (analog, actions)
    }

    /// The head turned by a stick, a gamepad's steering, and OMSI's mouse steering: the
    /// controllers' axes as the bus is to take them.
    pub(super) fn frame_mouse_drive(&mut self, dt: f32, analog: crate::controllers::Analog) -> crate::controllers::Analog {
        // OMSI's mouse control: the cursor's place across steers, above the middle
        // of the window is the throttle, below it the brake.
        // Steering as Omsi.exe has it (0x6f4284..0x6f447b): the whole width of the
        // window is the full lock from left to right, divided by the speed in tens
        // of km/h above 10 km/h - at 50 km/h the same hand movement turns the wheel a
        // fifth as far, which is what makes the wheel feel heavier the faster the bus
        // goes. For a second after mouse steering is switched on the wheel eases
        // towards the cursor (a half-life of the time that is left), then follows it.
        let mut analog = analog;
        // the head turned by a stick or an axis set up for it (#454), at up to
        // 120 degrees a second, in the views of the bus, on foot and flying
        if analog.look != [0.0, 0.0] && self.menus.game_menu.is_none() && self.menus.chooser.is_none() && !self.paused {
            let k = LOOK_STICK_DEG_S * dt * self.settings.look_sens;
            self.look_by(analog.look[0] * k, analog.look[1] * k);
        }
        // a gamepad's stick: a target the wheel turns towards at a hand's pace (from the
        // middle to the full lock in `pad_steer_speed` seconds), not the wheel's place
        // itself (#200). The target is smoothed first (`pad_steer_smooth`)
        let stick = analog.stick.then_some(analog.steering).flatten().zip(self.player.as_ref());
        if let (Some((x, _)), true) = (stick, self.settings.pad_steer_linear) {
            // (a wheel the system takes for a gamepad: its axis as it reads, as a
            // wheel's - the stick's curve and its less lock at speed made its first
            // degrees do nothing and the rest too much, #1653)
            analog.steering = Some(x);
        } else if let Some((x, p)) = stick {
            let now = p.vehicle.physics.controls.steering;
            let kmh = p.vehicle.physics.velocity_kmh() as f32;
            self.input.pad_kmh = crate::controllers::smooth_toward(self.input.pad_kmh, kmh, dt, 0.4);
            let target = crate::controllers::gamepad_steering(x, self.input.pad_kmh);
            self.input.pad_steer_target = crate::controllers::smooth_toward(self.input.pad_steer_target, target, dt, self.settings.pad_steer_smooth / 1000.0);
<<<<<<< HEAD
            // the wheel turns lock to lock in `pad_steer_speed` seconds, and slower the faster
            // the bus goes - at 50 km/h by half again - so that a flick of the thumb never
            // throws a heavy bus sideways (it went from lock to lock in 1.2 s at any speed)
            let lock_time = (self.settings.pad_steer_speed * (1.0 + self.input.pad_kmh.abs().min(100.0) / 100.0)).max(0.3);
            let step = dt / lock_time;
=======
            // (the hand's pace is the same at any speed: the stick's reach already shrinks
            // with the speed, `gamepad_steering`)
            let step = dt / self.settings.pad_steer_speed.max(0.3);
>>>>>>> c4738ed6f43f11b4c06ead299af7ac74280d5c5b
            analog.steering = Some(now + (self.input.pad_steer_target - now).clamp(-step, step));
        } else if let Some(p) = self.player.as_ref() {
            // (the stick picks up from where the wheel is, at the bus's speed)
            self.input.pad_steer_target = p.vehicle.physics.controls.steering;
            self.input.pad_kmh = p.vehicle.physics.velocity_kmh() as f32;
        }
        // (in every view of the bus - driver, outside, passenger and the map camera -
        // as in OMSI, where switching the camera leaves the mouse steering on: its
        // mouse steering asks only for a player's vehicle, 0x6f4257; not on foot,
        // #516)
        let bus_view = self.mouse_steers_in_view();
        // (the plugins' panels having the mouse hold the wheel and the pedals as
        // looking round does: the cursor goes to their buttons)
        let panels_mouse = self.plugin_focus();
        // (the cursor held while the mouse steers, let go when it is wanted: mouse_grab.rs)
        self.sync_mouse_grab();
        if let (true, Some(s)) = (self.mouse_steering_now(), self.gfx.surface.as_ref()) {
            let (w, h) = (s.config.width as f32, s.config.height as f32);
            if std::mem::take(&mut self.input.center_cursor) {
                self.input.cursor = (w * 0.5, h * 0.5);
                self.input.mouse_grab.at = Some(self.input.cursor);
                // (a locked cursor stands in the middle already, and moving it unlocks it)
                if let (Some(win), false) = (self.window.as_ref(), self.input.mouse_grab.mode == Some(crate::app_impl::GrabMode::Locked)) {
                    let _ = win.set_cursor_position(winit::dpi::PhysicalPosition::new((w * 0.5) as f64, (h * 0.5) as f64));
                }
            }
            // (the speed the divisor takes, smoothed over 0.4 s: the bus's own speed
            // trembles by fractions of a km/h from frame to frame on its springs and
            // tyres, and at 30 km/h the wheel twitched with it by itself)
            let raw_kmh = self.player.as_ref().map(|p| p.vehicle.physics.velocity_kmh()).unwrap_or(0.0);
            let k_v = 1.0 - (-dt / 0.4).exp();
            self.input.mouse_kmh += (raw_kmh - self.input.mouse_kmh) * k_v;
            let kmh = self.input.mouse_kmh;
            // (the mouse's own point, which goes on past the window's edges as far as the
            // full lock at this speed: the cursor stopped at the edge of the screen, and at
            // speed the lock lies further out than that)
            self.input.mouse_grab.clamp((w, h), crate::app_impl::mouse_grab_reach(kmh, self.settings.mouse_sens));
            let (cx, cy) = self.input.mouse_grab.at.unwrap_or(self.input.cursor);
            let target = (crate::player::mouse_steering(cx, w, kmh) * self.settings.mouse_sens).clamp(-1.0, 1.0);
            // the pedals as Omsi.exe has them: from the middle of the window to its
            // top edge the throttle, to the bottom one the brake, straight on
            let y = (2.0 * cy / h.max(1.0) - 1.0).clamp(-1.0, 1.0);
            let (pedal_t, pedal_b) = (
                crate::player::mouse_pedal_target((-y).max(0.0), self.settings.mouse_pedal_strength),
                crate::player::mouse_pedal_target(y.max(0.0), self.settings.mouse_pedal_strength),
            );
            let (steer, fade) = &mut self.input.mouse_steer;
            // (after the first second the wheel follows the cursor within ~60 ms, or at
            // once with Smooth mouse steering off, #1092)
            let k = crate::player::mouse_follow(*fade, dt, self.settings.mouse_smooth);
            *steer = target + (*steer - target) * k;
            let (mt, mb) = &mut self.input.mouse_pedals;
            *mt = crate::player::mouse_pedal(*mt, pedal_t, k);
            *mb = crate::player::mouse_pedal(*mb, pedal_b, k);
            *fade = (*fade - dt).max(0.0);
            analog.steering = Some(*steer);
            // OMSI_TRACE_STEER=<csv>: the mouse steering frame by frame
            if let Some(path) = omsi_cfg::flags::OMSI_TRACE_STEER.os() {
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
                    let _ = writeln!(f, "{:.3},{:.4},{:.1},{:.2},{:.4},{:.4},{:.3}", self.clock.run_time, dt, cx, kmh, target, self.input.mouse_steer.0, deg);
                }
            }
            // the mouse owns the wheel (OMSI sets the curvature from it every frame):
            // a steering key's leftover turn must not take over whenever the cursor
            // passes the middle - the wheel jumped there; and the pedals, which Omsi.exe
            // writes from the cursor every frame: a brake the keys held stayed on (#395)
            if let Some(p) = self.player.as_mut() {
                p.axes.steering = 0.0;
                p.axes.brake = 0.0;
                p.axes.throttle = 0.0;
            }
            analog.throttle = Some(self.input.mouse_pedals.0);
            analog.brake = Some(self.input.mouse_pedals.1);
        } else if self.input.mouse_drive && bus_view && (self.input.mouse_look || self.input.input_away || panels_mouse)
            && self.menus.game_menu.is_none() {
            // looking round with the right button: the wheel and the pedals stay where
            // the mouse left them, as in OMSI (they went slack until the button was let
            // go - no quick look round while driving). With the window in the
            // background the same, its throttle let go (`App::input_lost`): the
            // cursor wandering over other windows steered and drove the bus.
            analog.steering = Some(self.input.mouse_steer.0);
            analog.throttle = Some(self.input.mouse_pedals.0);
            analog.brake = Some(self.input.mouse_pedals.1);
            if let Some(p) = self.player.as_mut() {
                p.axes.steering = 0.0;
            }
        }
        analog
    }

    /// The controllers' buttons: the game's own actions, then the bus's, with the axes.
    pub(super) fn frame_pad_actions(&mut self, analog: crate::controllers::Analog, actions: Vec<(String, bool)>) {
        // the controller's view buttons are the game's, not the bus's: looking around
        // while held (`view_look_*`), the bus radio while held (`voice_radio`), and
        // OMSI's view actions (other cameras, views)
        let mut actions = actions;
        {
            let menu_open = self.menus.game_menu.is_some() || self.menus.chooser.is_some();
            let mut game: Vec<String> = Vec::new();
            actions.retain(|(name, down)| {
                let n = name.to_ascii_lowercase();
                // (with a menu open the buttons do nothing - but the one that opened it
                // closes it again)
                if menu_open && *down && n != "open_menu" { return false; }
                if let Some(k) = ["view_look_left", "view_look_right", "view_look_up", "view_look_down"].iter().position(|x| *x == n) {
                    self.input.pad_look[k] = *down;
                    return false;
                }
                // hold-to-talk: track press/release like `view_look_*`, do not fire once
                if n == "voice_radio" {
                    self.input.pad_voice_radio = *down;
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
            p.axes.red_steer_spd = self.settings.red_steer_spd;
            p.axes.pedal_hold = self.settings.brake_hold;
            p.analog = analog;
            for (name, down) in actions {
                if !down || (self.menus.game_menu.is_none() && self.menus.chooser.is_none()) {
                    p.action(&name, down);
                }
            }
        }
    }
}
