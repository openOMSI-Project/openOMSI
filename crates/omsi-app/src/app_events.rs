//! The window's events: winit's `ApplicationHandler` for `App`.

/// Mirror pictures drawn per second at most, all mirrors together (see the redraw).
const MIRROR_RATE: f32 = 75.0;
/// The least a mirror is redrawn a second (see the mirrors in `window_event`).
const MIRROR_MIN_HZ: f32 = 8.0;
/// The most a mirror in the picture is redrawn a second, with the real-time reflections
/// economical (`mirror_refresh=eco`) and full (the default).
const MIRROR_MAX_HZ_ECO: f32 = 15.0;
const MIRROR_MAX_HZ_FULL: f32 = 30.0;
/// With no real-time reflections (`mirror_refresh=off`) a bus's mirrors are drawn once when
/// it is taken over and once more this many seconds later.
const MIRROR_FREEZE_REDRAW: f32 = 2.0;

/// Consume the VR redraw budget without updating a mirror twice in one frame.
/// Negative rates request every mirror each frame; zero freezes immediately.
fn vr_mirror_updates(budget: &mut f32, dt: f32, rate: f32, mirrors: usize) -> usize {
    if mirrors == 0 || rate == 0.0 {
        *budget = 0.0;
        return 0;
    }
    if rate < 0.0 {
        *budget = 0.0;
        return mirrors;
    }
    // Keep only a frame's worth of work after a stall, with the fractional
    // credit carried forward for rates below the game's frame rate.
    *budget = (*budget + dt.clamp(0.0, 0.1) * rate).min(mirrors as f32 + 0.5);
    let updates = (budget.floor() as usize).min(mirrors);
    *budget -= updates as f32;
    updates
}

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

mod frame;
pub(crate) use frame::steps;

/// How fast a stick turns the head, fully pushed (degrees a second, see `Analog::look`).
const LOOK_STICK_DEG_S: f32 = 120.0;

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.resumed_impl(event_loop);
    }

    /// A phone put the app into the background: its window's surface goes (made again on
    /// `resumed`), the fingers and the held keys are let go.
    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.gfx.surface = None;
        self.input.touch.drop_gpu();
        self.input_lost();
        self.save_last_situation();
        // (a phone's app in the background is often ended without `exiting`)
        omsi_render::pipeline_cache::save();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.finish_vr_nav_edit();
                self.finish_session();
                crate::platform::exit(event_loop);
            }
            WindowEvent::Resized(size) => {
                if let (Some(s), Some(r)) = (self.gfx.surface.as_mut(), self.renderer.as_ref()) {
                    s.resize(r, size.width, size.height);
                }
                // (minimised, Windows makes the window 0 x 0)
                let hidden = size.width == 0 || size.height == 0;
                if hidden && !self.gfx.window_hidden {
                    self.input_lost();
                }
                self.gfx.window_hidden = hidden;
            }
            // (minimised or covered entirely, macOS and Wayland: the focus goes with it, and a
            // window merely covered by another may still be the one the player drives with)
            WindowEvent::Occluded(hidden) => self.gfx.window_hidden = hidden,
            WindowEvent::Focused(true) => {
                self.input.window_focused = true;
                if let Some(ctl) = self.input.controllers.as_mut() {
                    ctl.set_focus(true);
                }
                self.input_back();
            }
            WindowEvent::Focused(false) => {
                self.finish_vr_nav_edit();
                self.input.window_focused = false;
                if let Some(ctl) = self.input.controllers.as_mut() {
                    ctl.set_focus(false);
                }
                #[cfg(windows)]
                {
                    self.xr.vr_cursor_physical = None;
                    self.xr.vr_cursor_warp_pending = None;
                }
                // No key-up reaches us for whatever was held when focus left (alt-tab, a
                // click outside the window, an OS dialog popping up): without this, a held
                // modifier got "stuck" and made the next plain key press look like it was
                // held with that modifier - Shift got stuck this way once, and a plain `W`
                // (throttle in the wasd preset) was then read as Shift+W, OMSI's own wiper
                // key, toggling the wipers on every press instead of driving. The vehicle's
                // own keys, a door button, a switch held with the mouse and the mouse
                // steering's throttle went on as well, with the window minimised.
                self.input_lost();
            }
            // nothing the keyboard or the mouse does reaches the game while the window is
            // in the background (a wheel turned over a window behind another one zoomed;
            // the keys a system sends again for what is still held when the focus comes
            // back count only when pressed anew)
            WindowEvent::KeyboardInput { is_synthetic, ref event, .. } if self.input.input_away || (is_synthetic && event.state == ElementState::Pressed) => {}
            WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. } if self.input.input_away => {}
            WindowEvent::ModifiersChanged(modifiers) => {
                for code in input_script::release_inactive_modifiers(&mut self.input.keys, modifiers.state()) {
                    self.on_key(event_loop, code, false, false);
                }
            }
            // (a modifier's key-up that `ModifiersChanged` let go of already: Windows tells
            // the new modifier state before the key-up itself, and the release ran twice)
            WindowEvent::KeyboardInput { ref event, .. }
                if event.state == ElementState::Released
                    && matches!(event.physical_key, PhysicalKey::Code(c) if input_script::is_modifier(c) && !self.input.keys.contains(&c)) => {}
            WindowEvent::KeyboardInput { event, .. } => {
                // a plugin's text field being typed into takes the keys pressed (their
                // releases go on, so that nothing held stays held)
                if event.state == ElementState::Pressed {
                    if let PhysicalKey::Code(code) = event.physical_key {
                        if self.plugin_typing_key(code, event.text.as_deref()) {
                            return;
                        }
                    }
                }
                if event.state == ElementState::Pressed && self.menus.menu_edit_icao {
                    if let Some(text)=event.text.as_deref(){ self.icao_edit_text(text); }
                }
                // Route numbers are free display text in OMSI. Take the text produced by
                // the keyboard layout (rather than only the physical key) so '-', shifted
                // symbols and non-US layouts reach the destination display unchanged.
                if event.state == ElementState::Pressed
                    && self.menus.menu_edit.is_some()
                    && !self.menus.menu_edit_icao
                    && (self.menus.menu_edit_search || matches!(self.menus.list_kind, Some(crate::game_lists::ListKind::RouteNumbers)))
                {
                    if let Some(text) = event.text.as_deref() {
                        if text.chars().any(|c| !c.is_control()) {
                            self.route_edit_text(text);
                            return;
                        }
                    }
                }
                // '/' opens the chat's input box wherever the keyboard has it (the key
                // itself is then swallowed by the chat) - but not Numpad ÷, OMSI's stock
                // front door key (keyboard.cfg `bus_doorfront0 181`)
                // (only while `chat_open` is on its own key: one the player moved it to is
                // the only one, #130)
                if event.state == ElementState::Pressed
                    && event.text.as_deref() == Some("/")
                    && event.physical_key != PhysicalKey::Code(KeyCode::NumpadDivide)
                    && self.input.game_keys.iter().any(|b| b.action.eq_ignore_ascii_case("chat_open") && b.scan_code == 53 && b.chord() == 0)
                    && self.net.lan.is_some()
                    && !lan::chat_open(&self.net.remotes)
                {
                    self.net.remotes.chat.open();
                    if let PhysicalKey::Code(code) = event.physical_key {
                        lan::chat_swallow(&mut self.net.remotes, code);
                    }
                    return;
                }
                // what is typed into an open LAN chat line (the key itself goes on to on_key)
                if let (Some(text), true, true) = (
                    event.text.as_deref(),
                    event.state == ElementState::Pressed,
                    lan::chat_open(&self.net.remotes),
                ) {
                    lan::chat_type(&mut self.net.remotes, text);
                }
                // (the Lua plugins' `key` event; a key held down repeats nothing)
                if let (PhysicalKey::Code(code), false) = (event.physical_key, event.repeat) {
                    if self.integrations.plugin_keys.len() < 64 {
                        self.integrations.plugin_keys.push((format!("{code:?}"), event.state == ElementState::Pressed));
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
            // On the desktop a right-drag zooms, as in OMSI (`on_right`).
            WindowEvent::MouseInput {
                state,
                button: winit::event::MouseButton::Right,
                ..
            } => {
                if let Some(edit) = self.xr.vr_nav_edit.as_mut() {
                    edit.rotating = state == ElementState::Pressed;
                    return;
                }
                if self.menus.navigator.as_ref().map(|n| n.map_open()).unwrap_or(false) {
                    return;
                }
                if self.vr_active() {
                    #[cfg(windows)]
                    if state == ElementState::Pressed && self.menus.game_menu.is_none()
                        && self.menus.chooser.is_none() {
                        if self.input.mouse_drive {
                            self.set_mouse_drive(false);
                            self.service_msg = Some(("Mouse steering off".into(), 3.0));
                        } else {
                            self.xr.vr_zoom_active = !self.xr.vr_zoom_active;
                        }
                    }
                } else {
                    self.on_right(state == ElementState::Pressed);
                }
            }
            // (the middle button - the wheel pressed - turns the view as well: OMSI's pan)
            WindowEvent::MouseInput {
                state,
                button: winit::event::MouseButton::Middle,
                ..
            } => {
                if self.xr.vr_nav_edit.is_some() { return; }
                if self.menus.navigator.as_ref().map(|n| n.map_open()).unwrap_or(false) {
                    return;
                }
                self.input.mouse_look = state == ElementState::Pressed;
                self.input.mmb_held = state == ElementState::Pressed;
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
                if self.xr.vr_nav_edit.is_some() { return; }
                // (both physical pixels)
                if let Some((x, y)) = self.input.cursor_hidden {
                    if (position.x as f32 - x).abs() + (position.y as f32 - y).abs() > 8.0 {
                        self.input.cursor_hidden = None;
                        if let Some(win) = self.window.as_ref() {
                            win.set_cursor_visible(true);
                        }
                    }
                }
                // (the on-screen controls on a computer, `OMSI_TOUCH=1`: the mouse is a
                // finger on them - from #202)
                if self.input.touch.enabled {
                    self.finger_move(0, glam::Vec2::new(position.x as f32, position.y as f32));
                }
                #[cfg(windows)]
                let vr_cockpit = self.xr.vr.is_some() && self.menus.game_menu.is_none()
                    && matches!(self.view.as_str(), "driver" | "pax");
                #[cfg(not(windows))]
                let vr_cockpit = false;
                if vr_cockpit && !self.input.mouse_look && !self.input.mouse_drive {
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
                // while the plugins' panels have the mouse its clicks are theirs, none the bus's
                if self.plugin_focus() {
                    self.plugin_click(state == ElementState::Pressed);
                    return;
                }
                if self.input.touch.enabled {
                    let p = glam::Vec2::new(self.input.cursor.0, self.input.cursor.1);
                    if state == ElementState::Pressed {
                        self.finger_down(event_loop, 0, p);
                    } else {
                        self.finger_up(event_loop, 0, p, false);
                    }
                } else {
                    let pressed = state == ElementState::Pressed;
                    self.input.buttons_held.0 = pressed;
                    // the right button already down (looking round): both held zoom, and
                    // the click works nothing in the cab
                    if pressed && self.input.buttons_held.1 && self.start_both_drag() {
                        return;
                    }
                    // (the right button still held goes on zooming by itself, unless with
                    // `[altView]` it turns the view)
                    if !pressed && self.input.both_drag.is_some() && !(self.input.buttons_held.1 && self.right_zooms()) {
                        self.input.both_drag = None;
                        self.input.mouse_look = self.input.buttons_held.1;
                        self.update_hover();
                    }
                    self.left_button(event_loop, pressed);
                }
            }
            // a finger (a phone; see touch.rs)
            WindowEvent::Touch(t) => self.on_touch(event_loop, t),
            WindowEvent::RedrawRequested => self.frame(event_loop),
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
            if let Some(controllers) = self.input.controllers.as_ref() {
                controllers.refresh_devices();
            }
        }
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.xr.vr_nav_edit.is_some() {
                if self.input.window_focused { self.vr_nav_drag(delta.0 as f32, delta.1 as f32); }
                return;
            }
            // (in a view of the bus the cursor's own way turns it: move_cursor)
            if self.input.mouse_look {
                if !self.cursor_looks() {
                    if self.view == "outside" {
                        // F3 chase orbits at its own gain, not the head's.
                        self.sync_view_look();
                        let (y, p) = crate::input_script::chase_orbit_step(
                            self.cam.look.0,
                            self.cam.look.1,
                            delta.0 as f32,
                            delta.1 as f32,
                        );
                        self.cam.look.0 = y;
                        self.cam.look.1 = p;
                    } else {
                        let k = 0.15 * self.settings.look_sens;
                        self.look_by(delta.0 as f32 * k, delta.1 as f32 * k);
                    }
                }
            } else if self.input.mouse_grab.mode == Some(crate::app_impl::GrabMode::Locked) {
                // (the locked cursor's raw movement, in points on macOS: window pixels)
                let s = self.window.as_ref().map(|w| w.scale_factor() as f32).unwrap_or(1.0);
                self.steer_by(delta.0 as f32 * s, delta.1 as f32 * s);
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // the server sent us away (kick, ban): the game ends, the launcher says why
        if self.net.lan.as_ref().and_then(crate::lan::turned_away).is_some() {
            self.finish_session();
            crate::platform::exit(event_loop);
            return;
        }
        crate::game_lists::flush_settings(false);
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
        crate::game_lists::flush_settings(true);
        self.finish_session();
        // (with the pipelines made since the start: puddles, Enhanced+)
        omsi_render::pipeline_cache::save();
        // ("playing now" ends with the game)
        self.integrations.presence = None;
        if let Some(lan) = self.net.lan.take() {
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
    /// The window's size in pixels, while the mirror panels can be worked (in the cab, no menu).
    pub(crate) fn mirror_hud_size(&self) -> Option<(f32, f32)> {
        if !self.cam.in_cab || self.menus.game_menu.is_some() || !self.gfx.mirror_hud.editing() {
            return None;
        }
        Some(self.hud_size())
    }

    /// The mouse wheel (or a pinch of two fingers): `amount` notches, up positive.
    pub(crate) fn wheel(&mut self, amount: f32) {
        if self.xr.vr_nav_edit.is_some() { self.vr_nav_scroll(amount); return; }
        // over a mirror panel the wheel resizes it (Shift: wider or narrower)
        if let Some(size) = self.mirror_hud_size() {
            let shift =
                self.input.keys.contains(&KeyCode::ShiftLeft) || self.input.keys.contains(&KeyCode::ShiftRight);
            if self
                .gfx.mirror_hud
                .wheel(amount, shift, self.hud_cursor(), size)
            {
                return;
            }
        }
        // the object editor: the wheel turns (Shift: lifts) the object
        if self.menus.game_menu.is_none() && self.editor_wheel(amount) {
            return;
        }
        // placing a vehicle: the wheel turns it
        if self.menus.placing.is_some() && self.menus.game_menu.is_none() {
            self.placing_wheel(amount);
            return;
        }
        // the game menu and its lists scroll with the wheel
        if self.menus.game_menu.is_some() {
            self.menu_wheel(amount);
            return;
        }
        // the city map takes the wheel while it is open
        if let Some(n) = self.menus.navigator.as_mut().filter(|n| n.map_open()) {
            n.map_wheel(amount, self.input.cursor.0, self.input.cursor.1);
            return;
        }
        // the wheel over the chat (or while typing) scrolls its history
        if let Some(ui) = self.ui.as_mut() {
            if self.net.lan.is_some() && (ui.chat.hovered || lan::chat_open(&self.net.remotes)) {
                // Ctrl + the wheel makes the chat larger or smaller (kept for the next game)
                if self.input.keys.contains(&KeyCode::ControlLeft) || self.input.keys.contains(&KeyCode::ControlRight) {
                    let to = ((self.settings.chat_size + amount.signum() * 0.1) * 10.0).round() / 10.0;
                    self.settings.chat_size = to.clamp(0.5, 3.0);
                    crate::game_lists::remember_setting("chat_size", &self.settings.chat_size.to_string());
                    return;
                }
                ui.chat.wheel(self.net.remotes.chat.lines.len(), amount);
                return;
            }
        }
        // The wheel over a cockpit switch turns it: the same <event>_drag the
        // original fires while the mouse is dragged, with the notch as the
        // movement. Knobs, the sun blind and the ignition key are far easier to
        // set that way than by holding the button down and moving the mouse.
        if self.menus.hover.is_some() && self.view != "free" {
            let ray = self.camera.as_ref().zip(self.gfx.surface.as_ref())
                .map(|(cam, s)| self.cockpit_cursor_ray(cam, (s.config.width, s.config.height)));
            if let (Some(p), Some((o, d, spread))) = (
                self.player.as_mut(),
                ray,
            ) {
                p.occlude_controls = self.view == "outside";
                if p.pick(o, d, spread).is_some() {
                    // a notch is worth a good push of the mouse: the scripts divide
                    // the movement by 10 (the ignition key), 200 (the parking brake)
                    // or 500 (the driver's window), so a few pixels would do nothing
                    p.wheel(o, d, spread, -amount * 40.0);
                    return;
                }
            }
            if let (Some(w), Some((o, d, spread))) = (self.world.as_ref(), self.cursor_ray_now()) {
                let blocked = self.player.as_ref().and_then(|p| p.opaque_body_hit(o, d));
                if let Some(hit) = w.scenery_object_hit(o, d, crate::input_script::SCENERY_OBJECT_REACH, spread).filter(|h| blocked.map_or(true, |t| t >= h.t)) {
                    w.scenery_object_wheel(hit.map_id, &hit.event, -amount * 40.0);
                    return;
                }
            }
        }
        let ctrl = self.input.keys.contains(&KeyCode::ControlLeft) || self.input.keys.contains(&KeyCode::ControlRight);
        if self.view == "outside" && self.player.is_some() && ctrl {
            // Ctrl+wheel: the outside camera stays where it is and narrows its field of view
            // (a telephoto; OMSI's own zoom there only moves the camera, as the wheel does)
            self.zoom_by(amount);
        } else if self.view == "outside" && self.player.is_some() {
            self.cam.orbit = (self.cam.orbit - amount * 1.5).clamp(ORBIT_MIN, ORBIT_MAX);
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
        if let Some(edit) = self.xr.vr_nav_edit.as_mut() { edit.moving = pressed; return; }
        // a mirror panel is dragged with the left button (a release always ends a drag)
        if let Some(size) = self
            .mirror_hud_size()
            .or_else(|| (!pressed).then(|| self.hud_size()))
        {
            if self.gfx.mirror_hud.press(pressed, self.hud_cursor(), size) {
                return;
            }
        }
        let state = if pressed { ElementState::Pressed } else { ElementState::Released };
        // placing a vehicle: a click sets it down
        if self.menus.placing.is_some() && self.menus.game_menu.is_none() {
            if state == ElementState::Pressed {
                self.placing_click();
            }
            return;
        }
        // the game menu takes the clicks while it is open
        if self.menus.game_menu.is_some() {
            if state == ElementState::Pressed {
                // (a tap or a click: only what is under the finger or the mouse is lit)
                self.menus.menu_kbd = false;
            }
            // Releasing the mouse button finishes scrollbar dragging.
            if state == ElementState::Released {
                self.menus.menu_drag = None;
                self.menus.dd_scroll_drag = None;
                self.menus.pane_scroll_drag = None;
                if self.menus.menu_scroll_drag {
                    self.menus.menu_scroll_drag = false;
                    self.menus.menu_top = self.menus.menu_top.map(f32::round);
                }
                return;
            }

            // an open drop-down takes the click: an entry is chosen, anywhere else closes it
            if self.menus.dropdown.is_some() {
                let inside = |r: &[f32; 4]| self.input.cursor.0 >= r[0] && self.input.cursor.0 <= r[2] && self.input.cursor.1 >= r[1] && self.input.cursor.1 <= r[3];
                let hit = self.ui.as_ref().and_then(|u| u.dd_rects.iter().position(|r| inside(r)).map(|i| i + u.dd_top));
                // its scroll bar is dragged (a press on the track beside the thumb takes the
                // thumb there by its middle); before, the press closed the list (#794)
                if let Some((track, thumb)) = self.ui.as_ref().and_then(|u| u.dd_scroll).filter(|_| hit.is_none()) {
                    let bar = [thumb[0], track[1], thumb[2], track[3]];
                    if inside(&bar) {
                        let grab = if inside(&thumb) { self.input.cursor.1 - thumb[1] } else { (thumb[3] - thumb[1]) * 0.5 };
                        self.menus.dd_scroll_drag = Some(grab);
                        self.drag_dropdown(self.input.cursor.1);
                        return;
                    }
                }
                match hit {
                    Some(i) => self.dropdown_pick(i),
                    None => self.menus.dropdown = None,
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
                    if self.input.cursor.0 >= thumb[0]
                        && self.input.cursor.0 <= thumb[2]
                        && self.input.cursor.1 >= thumb[1]
                        && self.input.cursor.1 <= thumb[3]
                    {
                        self.menus.menu_scroll_drag = true;
                        return;
                    }
                }

                // The sidebar of a settings window: a page, or the way back.
                if self.menus.chooser.is_some() {
                    let side = self.ui.as_ref().and_then(|u| {
                        u.menu_side.iter().position(|r| {
                            self.input.cursor.0 >= r[0]
                                && self.input.cursor.0 <= r[2]
                                && self.input.cursor.1 >= r[1]
                                && self.input.cursor.1 <= r[3]
                        })
                    });
                    if let Some(i) = side {
                        self.settings_side_click(i);
                        return;
                    }
                }

                // The timetable beside a line's tours: a stop to start from, or the button.
                if self.menus.chooser.is_some() {
                    // its scroll bar is dragged (a press on the track beside the thumb takes
                    // the thumb there by its middle)
                    if let Some((track, thumb, _, _)) = self.ui.as_ref().and_then(|u| u.menu_pane_scroll) {
                        let inside = |r: &[f32; 4]| self.input.cursor.0 >= r[0] && self.input.cursor.0 <= r[2] && self.input.cursor.1 >= r[1] && self.input.cursor.1 <= r[3];
                        if inside(&[thumb[0], track[1], thumb[2], track[3]]) {
                            let grab = if inside(&thumb) { self.input.cursor.1 - thumb[1] } else { (thumb[3] - thumb[1]) * 0.5 };
                            self.menus.pane_scroll_drag = Some(grab);
                            self.drag_pane(self.input.cursor.1);
                            return;
                        }
                    }
                    let pane = self.ui.as_ref().and_then(|u| {
                        let inside = |r: &[f32; 4]| self.input.cursor.0 >= r[0] && self.input.cursor.0 <= r[2] && self.input.cursor.1 >= r[1] && self.input.cursor.1 <= r[3];
                        if u.menu_pane_go.as_ref().is_some_and(inside) {
                            return Some(usize::MAX);
                        }
                        if let Some(j) = u.menu_time.iter().position(inside) {
                            return Some(usize::MAX - 1 - j);
                        }
                        u.menu_pane.iter().position(inside).map(|i| i + u.menu_pane_start)
                    });
                    if let Some(i) = pane {
                        self.tour_pane_click(i);
                        return;
                    }
                }

                // Otherwise check whether a menu row was clicked.
                let hit = self.ui.as_ref().and_then(|u| {
                    u.menu_rects.iter().position(|r| {
                        self.input.cursor.0 >= r[0]
                            && self.input.cursor.0 <= r[2]
                            && self.input.cursor.1 >= r[1]
                            && self.input.cursor.1 <= r[3]
                    })
                });

                if let Some(row) = hit {
                    // (a click on a slider or a stepper sets the value there)
                    let k = row
                        + self
                        .ui
                        .as_ref()
                        .map(|u| u.menu_start)
                        .unwrap_or(0);
                    let ctl = self.ui.as_ref().and_then(|u| u.menu_ctl.get(row).copied().flatten());

                    // (a greyed-out line cannot be clicked)
                    if self.menu_item_off(k) {
                        return;
                    }

                    if let Some(c) = ctl {
                        if self.menus.chooser.is_some() && self.input.cursor.0 >= c[0] && self.input.cursor.0 <= c[2] {
                            let fx = ((self.input.cursor.0 - c[0]) / (c[2] - c[0]).max(1.0)).clamp(0.0, 1.0);
                            self.menus.chooser = Some(k);
                            // (a slider is held: it follows the cursor till the button is let go)
                            if self.list_click(k, fx) {
                                self.menus.menu_drag = Some(k);
                            }
                            return;
                        }
                    }

                    if self.menus.chooser.is_none() {
                        self.menus.game_menu = Some(k);
                    }

                    // (a click on a tour shows its stops: the trip starts with the button)
                    if matches!(self.menus.list_kind, Some(crate::game_lists::ListKind::Tours(..))) && crate::game_lists::tour_at(self, k).is_some() {
                        self.menus.chooser = Some(k);
                        if let Some(crate::game_lists::ListKind::Tours(line, _)) = self.menus.list_kind.clone() {
                            self.menus.list_kind = Some(crate::game_lists::ListKind::Tours(line, None));
                        }
                        return;
                    }

                    // on the arrows round a line's value: one step down or up; elsewhere on
                    // the line as Enter
                    let arrows = self.ui.as_ref().and_then(|u| u.menu_arrows.get(row).copied().flatten());
                    match arrows {
                        Some([from, to, _]) if self.input.cursor.0 >= from && self.input.cursor.0 < to => self.chooser_adjust(k, "-"),
                        Some([_, _, plus]) if self.input.cursor.0 >= plus => self.chooser_adjust(k, "+"),
                        _ => self.menu_choose(event_loop, k),
                    }
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

/// The outside air from the weather and the cabin air the vehicle scripts/engine maintain.
/// OMSI exposes both to every bus as Weather_Temperature and Cabinair_Temp.
pub(crate) fn vehicle_temperatures(p: &Player) -> (f32, f32) {
    let outside = p.vehicle.host.temperature;
    let inside = p
        .vehicle
        .var("Cabinair_Temp")
        .filter(|v| v.is_finite())
        .unwrap_or_else(|| outside.clamp(18.0, 25.0));
    (outside, inside)
}

/// OMSI's information bar: the time, the speed, the kilometres driven this session and the
/// bus's odometer, temperatures, the passengers aboard, and the trip with its next stop and delay.
fn info_line(clock: &omsi_sim::SimClock, player: Option<&Player>, duty: Option<&crate::schedule::PlayerDuty>, passengers: Option<usize>, metres: f64) -> String {
    let t = clock.time;
    let mut parts = vec![format!("{:02}:{:02}:{:02}", ((t / 3600.0) as i64).rem_euclid(24), ((t % 3600.0) / 60.0) as i64, (t % 60.0) as i64)];
    if let Some(p) = player {
        parts.push(format!("{:.0} km/h", p.vehicle.physics.velocity_kmh().abs()));
        parts.push(distance_driven(metres));
        // the bus's whole mileage, as OMSI's Shift+Z overlay reads it ("Mileometer", the
        // `kmcounter_*` the cockpit shows, whole kilometres and the metres of the fraction)
        if let Some(km) = p.vehicle.var("kmcounter_km").filter(|v| v.is_finite()) {
            parts.push(odometer_reading(km as f64 + p.vehicle.var("kmcounter_m").unwrap_or(0.0) as f64 / 1000.0));
        }
        let (outside, inside) = vehicle_temperatures(p);
        parts.push(format!("EXT {:.0} °C / INT {:.0} °C", outside, inside));
        // the tank as the bus's script says it (OMSI's RL_TankContent: tank_percent)
        if let Some(tank) = p.vehicle.var("tank_percent").filter(|v| v.is_finite()) {
            parts.push(format!("tank {:.0} %", (tank * 100.0).round()));
        }
        // how many are aboard right now, out of the bus's places (None: the passengers are
        // switched off for this drive, so there is nothing to count)
        if let Some(n) = passengers {
            parts.push(passengers_aboard(n, passenger_capacity(&p.vehicle)));
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
    parts.join(ui::INFO_SEP)
}

/// The kilometres driven this session (`Career::metres`), to a hundred metres.
fn distance_driven(metres: f64) -> String {
    format!("{:.1} km", metres.max(0.0) / 1000.0)
}

/// The passenger places (`[passpos]`) in the player's bus and any coupled sections: nobody
/// boards once they are all taken (None: no passenger cabin to count).
fn passenger_capacity(vehicle: &omsi_sim::VehicleInstance) -> Option<usize> {
    let sections =
        std::iter::once(&vehicle.ty).chain(vehicle.trailers.iter().map(|trailer| &trailer.ty));
    let mut capacity = 0;
    let mut found_cabin = false;
    for ty in sections {
        if let Some(cabin) = crate::driver::cabin_of(&ty.def) {
            capacity += cabin.pass_positions.len();
            found_cabin = true;
        }
    }
    found_cabin.then_some(capacity)
}

/// The bus's odometer to a hundred metres, as the stock cockpits show it (km and tenths).
fn odometer_reading(km: f64) -> String {
    format!("{} {:.1} km", omsi_ui::tr("Odometer"), km.max(0.0))
}

/// `n` (out of `capacity` places, where known) with the word for a passenger in the
/// interface's language (singular for one; both words are keys of the tables - the whole
/// line is too much of a sentence to translate).
fn passengers_aboard(n: usize, capacity: Option<usize>) -> String {
    let count = capacity.map_or_else(|| n.to_string(), |capacity| format!("{n}/{capacity}"));
    format!(
        "{count} {}",
        omsi_ui::tr(if n == 1 { "Passenger" } else { "Passengers" })
    )
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

#[cfg(test)]
mod info_tests {
    use super::{distance_driven, odometer_reading, passengers_aboard};

    #[test]
    fn the_distance_driven_is_written_in_kilometres() {
        assert_eq!(distance_driven(0.0), "0.0 km");
        assert_eq!(distance_driven(12_345.0), "12.3 km");
    }

    #[test]
    fn the_odometer_reads_kilometres_and_tenths() {
        assert_eq!(odometer_reading(75_556.639), "Odometer 75556.6 km");
    }

    /// The count stands before the word, which is singular for one passenger (in the
    /// tables' language; without a lookup the English key is drawn as it is).
    #[test]
    fn one_passenger_is_written_in_the_singular() {
        assert_eq!(passengers_aboard(0, None), "0 Passengers");
        assert_eq!(passengers_aboard(1, None), "1 Passenger");
        assert_eq!(passengers_aboard(23, None), "23 Passengers");
    }

    #[test]
    fn passenger_count_includes_the_bus_capacity() {
        assert_eq!(passengers_aboard(4, Some(65)), "4/65 Passengers");
    }
}

#[cfg(test)]
mod vr_mirror_tests {
    use super::vr_mirror_updates;

    #[test]
    fn every_frame_updates_all_mirrors_even_at_low_game_fps() {
        let mut budget = 0.75;
        for dt in [1.0 / 90.0, 1.0 / 30.0, 0.5] {
            assert_eq!(vr_mirror_updates(&mut budget, dt, -1.0, 8), 8);
            assert_eq!(budget, 0.0);
        }
    }

    #[test]
    fn a_high_budget_is_not_limited_to_two_mirrors_per_frame() {
        let mut budget = 0.0;
        assert_eq!(vr_mirror_updates(&mut budget, 1.0 / 60.0, 240.0, 4), 4);
        assert_eq!(vr_mirror_updates(&mut budget, 0.5, 360.0, 4), 4);
        assert!(budget <= 0.5);
    }

    #[test]
    fn fractional_credit_preserves_the_selected_total_rate() {
        for fps in [30, 60, 90] {
            let mut budget = 0.0;
            let updates: usize = (0..fps * 10).map(|_| vr_mirror_updates(&mut budget, 1.0 / fps as f32, 16.0, 4)).sum();
            assert!((159..=160).contains(&updates), "fps={fps}: {updates}");
        }
    }

    #[test]
    fn off_and_no_mirrors_discard_old_credit() {
        let mut budget = 2.5;
        assert_eq!(vr_mirror_updates(&mut budget, 0.1, 0.0, 4), 0);
        assert_eq!(budget, 0.0);
        assert_eq!(vr_mirror_updates(&mut budget, 0.1, -1.0, 0), 0);
        assert_eq!(vr_mirror_updates(&mut budget, 0.1, 360.0, 0), 0);
        assert_eq!(budget, 0.0);
    }
}
