//! `OMSI_INPUT`: scripted keyboard, mouse and camera input for window runs, and its key names.

use super::*;

// (moved to `app_impl` with the methods they serve; kept reachable at their old path)
pub(crate) use crate::app_impl::{cab_look_yaw, chase_orbit_step, ease_look, gate_gear_var, look_key_of, on_server, reset_blend, swap_view_look};


/// Global actions a controller button should send to the game instead of to the bus script.
pub(crate) fn is_game_action(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("view_")
        || matches!(
            name.as_str(),
            "sim_pause"
                | "open_menu"
                | "screenshot"
                | "quicksave"
                | "toggel_mouse_ctrl"
                | "toggel_ctrler"
                | "voice_radio"
        )
}

/// How far (m) a click reaches a scenery object with a `[mouseevent]`.
pub(crate) const SCENERY_OBJECT_REACH: f32 = 50.0;

impl App {
    /// A key of the window, or of an `OMSI_INPUT` script.
    pub(crate) fn on_key(&mut self, event_loop: &ActiveEventLoop, code: KeyCode, pressed: bool, repeat: bool) {
        if self.xr.vr_nav_edit.is_some() {
            self.vr_nav_edit_key(code, pressed, repeat);
            return;
        }
        // The mirror panels (see mirror_hud.rs): Ctrl+M shows or hides them, Ctrl+Shift+M
        // starts and ends their editor; in the editor Insert, Delete, C and Esc are its keys.
        if self.mirror_hud_key(code, pressed, repeat) {
            return;
        }
        // Escape closes the city map first (it would end the session)
        if pressed && code == KeyCode::Escape {
            if let Some(n) = self.menus.navigator.as_mut().filter(|n| n.map_open()) {
                n.toggle_map();
                return;
            }
            // and gives the mouse back to the bus when the plugins' panels have it
            if self.release_plugin_focus() {
                return;
            }
        }
        let event_key = PhysicalKey::Code(code);
        if self.start_menu_key(event_loop, event_key, pressed) {
            return;
        }
        if let PhysicalKey::Code(code) = event_key {
            let pressed = pressed;
            // LAN chat: its keys (`chat_open`, '/' or '`', and `chat_toggle`, V, in keyboard.cfg's
            // [game]: the player can move them, #130) open the line and show or hide the
            // chat, and while the line is open the keys are its own
            if self.lan_chat_key(code, pressed, repeat) {
                return;
            }
            if pressed && !repeat {
                self.input.keys.insert(code);
            } else if !pressed {
                self.input.keys.remove(&code);
            }
            // Alt+Enter: full screen on and off
            if pressed && !repeat && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter) && (self.input.keys.contains(&KeyCode::AltLeft) || self.input.keys.contains(&KeyCode::AltRight)) {
                if self.gfx.spanned {
                    log::info!("triple screen: the window spans three monitors, Alt+Enter is left alone");
                } else if let Some(win) = self.window.as_ref() {
                    win.set_fullscreen(if win.fullscreen().is_some() { None } else { Some(winit::window::Fullscreen::Borderless(None)) });
                }
                return;
            }
            #[cfg(windows)]
            if pressed && !repeat && (self.xr.vr.is_some() || self.settings.vr_requested()) {
                let modifier = omsi_content::input::chord(
                    self.input.keys.contains(&KeyCode::ShiftLeft) || self.input.keys.contains(&KeyCode::ShiftRight),
                    self.input.keys.contains(&KeyCode::ControlLeft) || self.input.keys.contains(&KeyCode::ControlRight),
                    self.input.keys.contains(&KeyCode::AltLeft) || self.input.keys.contains(&KeyCode::AltRight),
                );
                let action = keys::dik_code(code).and_then(|scan| self.input.game_keys.iter()
                    .find(|b| b.scan_code == scan && b.matches(modifier) && b.action.starts_with("vr_"))
                    .map(|b| b.action.clone()));
                if let Some(action) = action {
                    if self.game_action(&action) { return; }
                }
            }
            // Shift+number works a door the way a key bound to its trigger in
            // `Inputs/keyboard.cfg` does in OMSI: `<trigger>` when it goes down and
            // `<trigger>_off` when it comes up. The door buttons of automatic-door buses
            // are push buttons held between the two (the MAN Lion's City A21's
            // `cockpit_tuertaster1`); never let go, the button stayed pressed and, once
            // the door release was on, the front door opened and closed for ever.
            if !pressed {
                let released: Vec<Vec<String>> = if digit_of(code).is_some() {
                    self.input.door_key_triggers.remove(&code).into_iter().collect()
                } else if matches!(code, KeyCode::ShiftLeft | KeyCode::ShiftRight) {
                    self.input.door_key_triggers.drain().map(|(_, g)| g).collect()
                } else {
                    Vec::new()
                };
                if let Some(p) = self.player.as_mut() {
                    for fired in &released {
                        p.door_key_off(fired);
                    }
                }
            }
            if self.overlay_key(event_loop, code, pressed, repeat) {
                return;
            }
            let ctrl = self.input.keys.contains(&KeyCode::ControlLeft) || self.input.keys.contains(&KeyCode::ControlRight);
            let alt = self.input.keys.contains(&KeyCode::AltLeft) || self.input.keys.contains(&KeyCode::AltRight);
            let shift_now = self.input.keys.contains(&KeyCode::ShiftLeft) || self.input.keys.contains(&KeyCode::ShiftRight);
            // getting up (Ctrl+Shift+G) and, on foot, the walker's keys
            if self.foot_key(code, pressed, repeat, ctrl, shift_now) {
                return;
            }
            // OMSI's global actions as `Inputs/keyboard.cfg` binds them ([game]); a key our
            // driving layout uses keeps that meaning (with the OMSI layout, every binding
            // counts)
            if self.game_binding_key(event_loop, code, pressed, repeat, ctrl, alt, shift_now) {
                return;
            }
            if self.shortcut_key(code, pressed, repeat, ctrl, alt, shift_now) {
                return;
            }
            if self.layout_extra_key(code, pressed, repeat) {
                return;
            }
            self.drive_key(code, pressed, repeat);
        }
    }

    /// `on_key` while the VR navigator is being placed: its keys are the only ones.
    fn vr_nav_edit_key(&mut self, code: KeyCode, pressed: bool, repeat: bool) {
        if !pressed { self.input.keys.remove(&code); }
        if matches!(code, KeyCode::ControlLeft | KeyCode::ControlRight | KeyCode::ShiftLeft | KeyCode::ShiftRight) && pressed {
            self.input.keys.insert(code);
        }
        if pressed && !repeat {
            match code {
                KeyCode::Escape | KeyCode::Enter => self.finish_vr_nav_edit(),
                KeyCode::KeyR => self.vr_nav_adjust("reset", 1.0),
                _ => {}
            }
        }
    }

    /// `on_key` for the mirror panels in the cab; true when the key was theirs.
    fn mirror_hud_key(&mut self, code: KeyCode, pressed: bool, repeat: bool) -> bool {
        if self.cam.in_cab && self.menus.game_menu.is_none() && self.player.is_some() {
            let ctrl = self.input.keys.contains(&KeyCode::ControlLeft) || self.input.keys.contains(&KeyCode::ControlRight);
            let shift = self.input.keys.contains(&KeyCode::ShiftLeft) || self.input.keys.contains(&KeyCode::ShiftRight);
            if pressed && !repeat && code == KeyCode::KeyM && ctrl {
                if let Some(p) = self.player.as_ref() {
                    let msg = if shift { self.gfx.mirror_hud.toggle_edit(p) } else { self.gfx.mirror_hud.toggle(p) };
                    self.service_msg = Some((msg, if shift { 6.0 } else { 3.0 }));
                }
                return true;
            }
            // (in the editor the arrows aim the mirror under the cursor; see the frame)
            if matches!(code, KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Minus | KeyCode::Equal | KeyCode::NumpadAdd | KeyCode::NumpadSubtract) && self.gfx.mirror_hud.arrow(code, pressed) {
                return true;
            }
            // R puts the mirror under the cursor back as the bus has it, Shift+R every mirror
            if self.gfx.mirror_hud.editing() && code == KeyCode::KeyR {
                if pressed && !repeat {
                    let size = self.hud_size();
                    let which = self.gfx.mirror_hud.cam_under(self.hud_cursor(), size);
                    let msg = match self.player.as_mut() {
                        Some(p) if shift => {
                            let n = p.vehicle.ty.def.cameras_reflexion.len();
                            p.mirror_offsets = vec![[0.0; 2]; n];
                            p.mirror_shifts = vec![[0.0; 3]; n];
                            p.mirror_fovs = vec![0.0; n];
                            p.mirrors_dirty = true;
                            "Every mirror is back as the bus has it".to_string()
                        }
                        Some(p) if which.is_some() => {
                            let i = which.unwrap_or(0);
                            if let Some(o) = p.mirror_offsets.get_mut(i) {
                                *o = [0.0; 2];
                            }
                            if let Some(s) = p.mirror_shifts.get_mut(i) {
                                *s = [0.0; 3];
                            }
                            if let Some(f) = p.mirror_fovs.get_mut(i) {
                                *f = 0.0;
                            }
                            p.mirrors_dirty = true;
                            format!("Mirror {} is back as the bus has it (Shift+R: every mirror)", i + 1)
                        }
                        _ => "R: put the cursor on a mirror panel (Shift+R: every mirror)".to_string(),
                    };
                    self.service_msg = Some((msg, 3.0));
                }
                return true;
            }
            if self.gfx.mirror_hud.editing() && matches!(code, KeyCode::BracketLeft | KeyCode::BracketRight | KeyCode::Semicolon | KeyCode::Quote) {
                if pressed {
                    let size = self.hud_size();
                    self.gfx.mirror_hud.size_key(code, self.hud_cursor(), size);
                }
                return true;
            }
            if self.gfx.mirror_hud.editing() && matches!(code, KeyCode::Insert | KeyCode::Delete | KeyCode::Backspace | KeyCode::KeyC | KeyCode::Escape) {
                if pressed && !repeat {
                    let size = self.hud_size();
                    if let Some(p) = self.player.as_ref() {
                        if let Some(msg) = self.gfx.mirror_hud.key(code, p, self.hud_cursor(), size) {
                            self.service_msg = Some((msg, 4.0));
                        }
                    }
                }
                return true;
            }
        }
        false
    }

    /// `on_key` while the start menu is shown (it takes every key); true when it was.
    fn start_menu_key(&mut self, event_loop: &ActiveEventLoop, event_key: PhysicalKey, pressed: bool) -> bool {
        if let (Some(m), PhysicalKey::Code(code)) = (self.menus.menu.as_mut(), event_key) {
            if pressed {
                if code == KeyCode::Escape {
                    crate::platform::exit(event_loop);
                }
                m.key(code);
                if m.start {
                    self.args.map = m
                        .maps
                        .get(m.map)
                        .map(|x| x.1.clone())
                        .unwrap_or(self.args.map.clone());
                    self.args.bus = m.vehicles.get(m.vehicle).map(|x| x.1.clone());
                    self.args.time = format!("{:02}:00", m.hour);
                    self.args.traffic = m.traffic;
                    self.args.passengers = m.passengers;
                    self.args.schedule = m.schedule;
                    self.args.day_of_year = Some(m.day);
                    if m.weather > 0 {
                        self.args.weather = m.weathers.get(m.weather).map(|w| w.1.clone());
                    }
                    if m.situation > 0 {
                        self.args.situation = m.situations.get(m.situation).map(|s| s.1.clone());
                        if let Err(e) = apply_situation(&mut self.args) {
                            log::error!("{e:#}");
                        }
                    }
                    self.menus.menu = None;
                    self.menus.hud = None;
                    self.load_world_now(event_loop);
                }
            }
            return true;
        }
        false
    }

    /// `on_key` for the LAN chat; true when the key was the chat's.
    fn lan_chat_key(&mut self, code: KeyCode, pressed: bool, repeat: bool) -> bool {
        if let Some(l) = self.net.lan.as_mut() {
            let held = |a: KeyCode, b: KeyCode| self.input.keys.contains(&a) || self.input.keys.contains(&b);
            let chord = omsi_content::input::chord(
                held(KeyCode::ShiftLeft, KeyCode::ShiftRight),
                held(KeyCode::ControlLeft, KeyCode::ControlRight),
                held(KeyCode::AltLeft, KeyCode::AltRight),
            );
            let bound = if held(KeyCode::SuperLeft, KeyCode::SuperRight) {
                None
            } else {
                keys::dik_code(code).and_then(|scan| self.input.game_keys.iter()
                    .find(|b| b.scan_code == scan && b.matches(chord) && b.action.to_ascii_lowercase().starts_with("chat_"))
                    .map(|b| b.action.clone()))
            };
            if lan::chat_key(l, &mut self.net.remotes, code, pressed, repeat, bound.as_deref()) {
                return true;
            }
        }
        false
    }

    /// `on_key` for what lies over the driving: placing a vehicle, the game menu, the object
    /// editor, Escape, a tutorial's pages. True when the key was theirs.
    fn overlay_key(&mut self, event_loop: &ActiveEventLoop, code: KeyCode, pressed: bool, repeat: bool) -> bool {
        // placing a vehicle with the mouse: its keys first (Escape takes it away)
        if self.menus.game_menu.is_none() && self.placing_key(code, pressed) {
            return true;
        }
        // the game menu: Escape opens it (and pauses, except in a LAN session, which
        // goes on for the others), and while it is open the keys are its own
        if self.menus.game_menu.is_some() {
            if pressed && !repeat {
                self.menu_key(event_loop, code);
            }
            return true;
        }
        // the object editor takes its keys first (Escape leaves it)
        if pressed && self.menus.editor.is_some() && self.editor_key(code) {
            return true;
        }
        if pressed && !repeat && code == KeyCode::Escape {
            self.open_game_menu();
            return true;
        }
        // a tutorial's pages: Enter / Page Down on, Page Up back, Ctrl+T hides them
        if let (true, Some(t)) = (pressed, self.menus.tutorial.as_mut()) {
            let ctrl = self.input.keys.contains(&KeyCode::ControlLeft) || self.input.keys.contains(&KeyCode::ControlRight);
            match code {
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::PageDown if !t.hidden && self.net.lan.is_none() => {
                    t.next();
                    return true;
                }
                KeyCode::PageUp if !t.hidden => {
                    t.back();
                    return true;
                }
                KeyCode::KeyT if ctrl => {
                    t.hidden = !t.hidden;
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    /// `on_key` for the [game] keys of keyboard.cfg; true when the key was used up.
    #[allow(clippy::too_many_arguments)]
    fn game_binding_key(&mut self, event_loop: &ActiveEventLoop, code: KeyCode, pressed: bool, repeat: bool, ctrl: bool, alt: bool, shift_now: bool) -> bool {
        if pressed && !repeat {
            // (a modifier key pressed is a key of its own, not its own modifier: Shift
            // bound to gear_up in keyboard.cfg came as Shift+Shift and matched nothing,
            // #1477; OMSI fires it)
            let m = omsi_content::input::chord(
                shift_now && !matches!(code, KeyCode::ShiftLeft | KeyCode::ShiftRight),
                ctrl && !matches!(code, KeyCode::ControlLeft | KeyCode::ControlRight),
                alt && !matches!(code, KeyCode::AltLeft | KeyCode::AltRight),
            );
            let own = keys::dik_code(code).is_some_and(|s| self.input.own_keys.contains(&s));
            let ours = self.args.drive_keys != "omsi"
                && m == 0
                && !own
                && (fallback_action(code, &self.args.drive_keys).is_some()
                || matches!(code, KeyCode::KeyZ | KeyCode::KeyX | KeyCode::KeyC | KeyCode::KeyI | KeyCode::KeyL));
            // plain Left/Right are OMSI's view_interiorcam_minus/plus, except when a wheel
            // steers: then the arrows glance (held, the head turns) and only Ctrl+Left/Right
            // switch the interior camera, below. (Where the arrows drive, `ours` skips this.)
            // (unless the settings ask for the cameras on them all the same, #1345)
            let plain_arrow = matches!(code, KeyCode::ArrowLeft | KeyCode::ArrowRight) && !ctrl
                && !self.settings.arrows_switch_cams
                && self.input.controllers.as_ref().is_some_and(|c| c.wheel_steering());
            // (the keys that fly the camera are the camera's, unmodified: S, OMSI's
            // view_toggle_viewpoint, threw the free camera back to the driver's view,
            // and with no bus of one's own every view flies - #868; a chord such as
            // Ctrl+S, OMSI's quicksave, stays a [game] key)
            let flying = m == 0
                && flies_free_camera(code)
                && (self.view == "free" || (self.player.is_none() && self.session.on_foot.is_none()));
            // (Ctrl+Alt+arrows turn the mirror looked at: not Ctrl+arrow's gear or camera)
            let mirror_aim = ctrl && alt && matches!(code, KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::ArrowUp | KeyCode::ArrowDown);
            if let Some(scan) = keys::dik_code(code).filter(|_| !ours && !flying && !mirror_aim) {
                let action = self.input.game_keys.iter().find(|b| b.scan_code == scan && b.matches(m)
                    && !b.action.starts_with("vr_")
                    && !(plain_arrow && b.action.starts_with("view_interiorcam_"))).map(|b| b.action.clone());
                // (a key bound in [game] and in [vehicles] does both, as in Omsi.exe: the
                // parking brake put on Space, the stock view_reset_all_directions key,
                // reset the view and never reached the bus - #745)
                let vehicle_too = self.player.as_ref().is_some_and(|p| p.bindings.iter().any(|b| b.scan_code == scan && b.matches(m)));
                log::debug!("key {code:?} (DIK {scan}, chord {m}): [game] {action:?}, a key of the bus too: {vehicle_too}; [game] keys on it: {:?}", self.input.game_keys.iter().filter(|b| b.scan_code == scan).collect::<Vec<_>>());
                // OMSI's `exit` (Ctrl+Q, or what the player put it on): the game ends as
                // the menu's Quit ends it. It was no action here at all, so the key did
                // nothing (#817)
                if action.as_deref() == Some("exit") {
                    self.finish_vr_nav_edit();
                    self.menus.game_menu = None;
                    self.finish_session();
                    crate::platform::exit(event_loop);
                    return true;
                }
                if let Some(a) = action {
                    if self.game_action(&a) && !vehicle_too {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// `on_key` for the built-in shortcuts (mouse steering, screenshot, pause, quicksave...);
    /// true when the key was one.
    fn shortcut_key(&mut self, code: KeyCode, pressed: bool, repeat: bool, ctrl: bool, alt: bool, shift_now: bool) -> bool {
        if pressed && !repeat {
            match code {
                // OMSI's `toggel_mouse_ctrl` (O): steering and pedals with the mouse
                KeyCode::KeyO if !ctrl && !alt && !shift_now && self.key_left_free(code, "toggel_mouse_ctrl") => {
                    self.game_action("toggel_mouse_ctrl");
                    return true;
                }
                // (a manual gearbox's Ctrl+Up / Ctrl+Down are the [game] keys `gear_up` and
                // `gear_down` now, which can be moved: see `with_game_defaults`, #907)
                // (Ctrl+Alt+arrows turn the mirror looked at, see the frame)
                KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::ArrowUp | KeyCode::ArrowDown if ctrl && alt => return true,
                // the interior cameras: Ctrl+Left/Right as well (the arrows drive) - unless
                // the player gave that combination to something else (#907)
                KeyCode::ArrowLeft | KeyCode::ArrowRight if ctrl && !self.chord_bound(code, shift_now, ctrl, alt) => {
                    self.game_action(if code == KeyCode::ArrowLeft { "view_interiorcam_minus" } else { "view_interiorcam_plus" });
                    return true;
                }
                // OMSI's `screenshot` (Ctrl+Shift+P: 25 / 6), and F12 as most games have it
                KeyCode::KeyP if ctrl && shift_now => {
                    self.take_screenshot();
                    return true;
                }
                // (F12 alone only where the bus has no key of its own on it: in OMSI's
                // keyboard.cfg it is the pram/wheelchair button, which it took away)
                KeyCode::F12 if !self.player.as_ref().is_some_and(|p| p.bindings.iter().any(|b| b.scan_code == 88 && b.chord() == 0 && p.vehicle.ty.program.trigger(&b.action).is_some())) => {
                    self.take_screenshot();
                    return true;
                }

                // the duty's next stop given up (#1015), as the game menu's line ("H" for
                // Haltestelle: Ctrl+Shift+N is the VR navigator's)
                KeyCode::KeyH if ctrl && shift_now && !alt && self.session.duty.is_some() && !self.chord_bound(code, shift_now, ctrl, alt) => {
                    self.skip_next_stop();
                    return true;
                }
                // the object editor (`crate::editor`)
                KeyCode::KeyE if ctrl && shift_now => {
                    self.toggle_editor();
                    return true;
                }
                // OMSI's `sim_pause`
                KeyCode::KeyP if !ctrl && !alt && !shift_now => {
                    self.toggle_pause();
                    return true;
                }
                // OMSI's `quicksave` (Alt+S)
                KeyCode::KeyS if alt && !ctrl => {
                    self.quick_save();
                    return true;
                }
                // OMSI's `view_toggle_informationdisplay` (Ctrl+Y)
                // OMSI's `view_toggle_informationdisplay` (Shift+Y: 21 / 2)
                KeyCode::KeyY if shift_now && !ctrl => {
                    self.set_info_bar(!self.menus.info_bar);
                    return true;
                }
                // OMSI's `view_set_schedule` (Insert: 210 / 1, the key's state every frame),
                // only where keyboard.cfg has no entry for it: an entry is the player's
                // binding, handled above, and one with scan code 0 is unbound - Insert opened
                // the timetable all the same (#1245)
                KeyCode::Insert if !shift_now && !ctrl && !self.input.game_keys.iter().any(|b| b.action.eq_ignore_ascii_case("view_set_schedule")) => {
                    self.menus.timetable = !self.menus.timetable;
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    /// `on_key` for the extra keys of the ready-made layouts and the camera / debug F-keys;
    /// true when the key was used up.
    fn layout_extra_key(&mut self, code: KeyCode, pressed: bool, repeat: bool) -> bool {
        // the extra keys of the ready-made layouts (below): not with Custom controls, not on
        // a key the player bound, not with a modifier held
        let extras = self.args.drive_keys != "omsi"
            && !keys::dik_code(code).is_some_and(|s| self.input.own_keys.contains(&s))
            && !self.input.keys.iter().any(|k| matches!(k, KeyCode::ControlLeft | KeyCode::ControlRight | KeyCode::AltLeft | KeyCode::AltRight | KeyCode::ShiftLeft | KeyCode::ShiftRight));
        if pressed && !repeat {
            // Z / X / C: indicator left / hazard / right, where the hand rests
            // (OMSI's own layout wants Shift and the numpad for them). Each is a
            // toggle: pressing the same key again turns it back off, tracked in
            // `blinker_key_state` since the scripts expose separate "set"/"off"
            // triggers for left/right rather than a toggle (hazard already has a
            // dedicated toggle trigger, `blinker_warn_toggle`).
            if self.view != "free"
                && extras
                && matches!(code, KeyCode::KeyZ | KeyCode::KeyX | KeyCode::KeyC)
            {
                let want: u8 = match code {
                    KeyCode::KeyZ => 1,
                    KeyCode::KeyC => 2,
                    _ => 3,
                };
                self.blinker(want);
            }
            // Shift + 1..9: open or close that physical door, front to back (see
            // `door_trigger_groups`); plain digits are left alone (some buses put
            // gears or numbered presets on them, `kw_s_1`/`automatic_1`).
            if self.view != "free" && shift_held_now(&self.input.keys) && !keys::dik_code(code).is_some_and(|s| self.input.own_shift.contains(&s)) {
                if let Some(n) = digit_of(code) {
                    if let Some(p) = self.player.as_mut() {
                        let fire = p.door_key(n);
                        if !fire.is_empty() {
                            self.input.door_key_triggers.insert(code, fire);
                        }
                    }
                }
            }
            // I: every saloon light circuit of the bus at once (OMSI has a key for
            // each: 7, 8, 9 - see Player::toggle_saloon_lights).
            if self.view != "free"
                && !repeat
                && extras
                && code == KeyCode::KeyI
            {
                if let Some(p) = self.player.as_mut() {
                    let msg = p.toggle_saloon_lights();
                    self.service_msg = Some((msg, 3.0));
                }
            }
            // (F1-F4 where keyboard.cfg has no view keys; a key the player gave to
            // something else, or a view key moved elsewhere, leaves them alone, #701)
            match code {
                KeyCode::F1 if self.key_left_free(code, "view_set_driver") => self.view = "driver".into(),
                KeyCode::F2 if self.key_left_free(code, "view_set_passenger") => self.view = "pax".into(),
                KeyCode::F3 if self.key_left_free(code, "view_set_outside") => self.view = "outside".into(),
                KeyCode::F4 if self.key_left_free(code, "view_set_map") => {
                    // the free camera starts where the current view is looking
                    self.view = "free".into();
                    self.cam.ego = false;
                }
                KeyCode::KeyU
                if self.input.keys.contains(&KeyCode::ShiftLeft)
                    || self.input.keys.contains(&KeyCode::ShiftRight) =>
                    {
                        // Shift+U: toggle a bus's service state by itself (start a shut
                        // bus, shut down a running one), and set its IBIS to the current
                        // duty as the driver would do while putting it into service.
                        if let Some(p) = self.player.as_mut() {
                            let msg = p.start_up();
                            self.service_msg = Some((msg, 6.0));
                            if let Some(d) = self.session.duty.as_ref() {
                                let (trip, stop) = d.trip_for_ibis();
                                p.set_duty_destination(trip, stop);
                            }
                        }
                    }

                KeyCode::KeyR
                if self.input.keys.contains(&KeyCode::ShiftLeft)
                    || self.input.keys.contains(&KeyCode::ShiftRight) =>
                    {
                        // Shift+R: the next internet radio station (see radio.rs)
                        let msg = self.sound.radio.next_station();
                        self.service_msg = Some((msg, 4.0));
                    }
                KeyCode::KeyM
                if self.input.keys.contains(&KeyCode::ShiftLeft)
                    || self.input.keys.contains(&KeyCode::ShiftRight) =>
                    {
                        // Shift+M: the city map (M alone is the starter)
                        if let Some(n) = self.menus.navigator.as_mut() {
                            n.toggle_map();
                        }
                    }
                KeyCode::KeyN
                if self.input.keys.contains(&KeyCode::ShiftLeft)
                    || self.input.keys.contains(&KeyCode::ShiftRight) =>
                    {
                        // Shift+N: navigator → navigator with the schedule → off (N alone is
                        // the gearbox's neutral)
                        if self.cycle_navigator() {
                            return true;
                        }
                    }
                KeyCode::F11 => {
                    // (Ctrl+F11; F11 alone is OMSI's pedestrian view)
                    // where am I: so a place that looks wrong can be named
                    if let Some(cam) = self.camera.as_ref() {
                        let ts = omsi_map::tile_size();
                        let (tx, ty) = (
                            (cam.position.x / ts).floor() as i32,
                            (cam.position.y / ts).floor() as i32,
                        );
                        let line = format!(
                            "Position {:.0}, {:.0}, {:.1}   tile {tx}_{ty}   heading {:.0} deg",
                            cam.position.x, cam.position.y, cam.position.z, cam.yaw
                        );
                        log::info!(
                            "{line}  (--cam {:.0},{:.0},{:.0},{:.0},{:.0})",
                            cam.position.x,
                            cam.position.y,
                            cam.position.z,
                            cam.yaw,
                            cam.pitch
                        );
                        self.service_msg = Some((line, 12.0));
                    }
                }
                KeyCode::F9 => {
                    // write the run into the driver's personnel file
                    let line = self.session.career.summary();
                    if self.session.career.path.is_some() {
                        if let Err(e) = self.session.career.save() {
                            log::warn!("writing the personnel file: {e}");
                        }
                    } else {
                        log::info!("this run: {line}");
                    }
                    self.service_msg = Some((line, 8.0));
                }
                // F5-F8 are the destination sign and roller blind keys of OMSI's
                // keyboard.cfg (bus_linie_minus/plus, bus_ziel_minus/plus,
                // bus_rollband_setL1..T): they go to the bus below and nowhere else. The
                // depot services are in the game menu (Esc), as in OMSI's menu.
                _ => {}
            }
        }
        false
    }

    /// `on_key` for the driving keys and the vehicle's own keys of keyboard.cfg.
    fn drive_key(&mut self, code: KeyCode, pressed: bool, repeat: bool) {
        // the arrow keys drive when a bus is being driven (the free camera keeps them)
        // (a key the player bound to something else is theirs, not the preset's)
        let own = keys::dik_code(code).is_some_and(|s| self.input.own_keys.contains(&s));
        let wheel = self.input.controllers.as_ref().is_some_and(|c| c.wheel_steering());
        let wasd = if own {
            "omsi"
        } else if wheel {
            // (with a wheel steering, the arrow keys are OMSI's: they look around)
            match self.args.drive_keys.as_str() {
                "arrows" | "omsi" => "omsi",
                _ => "wasd",
            }
        } else {
            self.args.drive_keys.as_str()
        };
        let shift_held =
            self.input.keys.contains(&KeyCode::ShiftLeft) || self.input.keys.contains(&KeyCode::ShiftRight);
        if let Some(p) = self.player.as_mut() {
            let ctrl_alt_held = (self.input.keys.contains(&KeyCode::ControlLeft) || self.input.keys.contains(&KeyCode::ControlRight)) && (self.input.keys.contains(&KeyCode::AltLeft) || self.input.keys.contains(&KeyCode::AltRight));
            if self.view != "free" && !repeat && !shift_held && !(ctrl_alt_held && pressed) {
                if let Some(a) = fallback_action(code, wasd) {
                    p.axes.set(a, pressed);
                }
            } else if !pressed {
                // a driving key let go always lets go: released while Shift was held (or
                // in the free view) it stayed "pressed", and the wheel went on turning to
                // full lock until that key was pressed again (#1040, #1050, #1053)
                if let Some(a) = fallback_action(code, wasd) {
                    p.axes.set(a, false);
                }
            }
        }
        // A driving key held with shift is the vehicle key it covers: Shift+W is
        // OMSI's wiper key, Shift+D selects the automatic's D, Shift+S the
        // viewpoint - otherwise a bus driven with WASD could never be put in gear.
        let shift =
            self.input.keys.contains(&KeyCode::ShiftLeft) || self.input.keys.contains(&KeyCode::ShiftRight);
        let covers_vehicle_key = fallback_action(code, wasd).is_some() && self.view != "free";
        let driving_key = covers_vehicle_key && !shift;
        // (the keys that fly the free camera are the camera's: W switched the wipers on
        // while flying)
        let fly_key = self.view == "free" && flies_free_camera(code);
        if let (Some(p), Some(scan)) = (
            self.player.as_mut(),
            keys::dik_code(code).filter(|_| !driving_key && !fly_key),
        ) {
            if !repeat {
                let m = if covers_vehicle_key {
                    0
                } else {
                    // (a modifier key is a key of its own here, not its own modifier, #1477)
                    omsi_content::input::chord(
                        shift && !matches!(code, KeyCode::ShiftLeft | KeyCode::ShiftRight),
                        (self.input.keys.contains(&KeyCode::ControlLeft) || self.input.keys.contains(&KeyCode::ControlRight))
                            && !matches!(code, KeyCode::ControlLeft | KeyCode::ControlRight),
                        (self.input.keys.contains(&KeyCode::AltLeft) || self.input.keys.contains(&KeyCode::AltRight))
                            && !matches!(code, KeyCode::AltLeft | KeyCode::AltRight),
                    )
                };
                p.key(scan, m, pressed);
            }
        }
    }

    /// Whether a key the game gives `action` by itself (F1 the driver's view, O the mouse
    /// steering) is still free for it: neither bound by the player to something of their
    /// own nor `action` bound to another key in keyboard.cfg.
    pub(crate) fn key_left_free(&self, code: KeyCode, action: &str) -> bool {
        key_left_free(keys::dik_code(code), action, &self.input.own_keys, &self.input.game_keys)
    }

    /// A key of the input script by name: a letter, a digit, F1..F12, or one of the named keys.
    pub(crate) fn script_key(name: &str) -> Option<KeyCode> {
        use KeyCode::*;
        pub(crate) const LETTERS: [KeyCode; 26] = [KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ];
        pub(crate) const DIGITS: [KeyCode; 10] = [Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9];
        pub(crate) const FKEYS: [KeyCode; 12] = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];
        let b = name.as_bytes();
        if b.len() == 1 && b[0].is_ascii_alphabetic() {
            return Some(LETTERS[(b[0].to_ascii_uppercase() - b'A') as usize]);
        }
        if b.len() == 1 && b[0].is_ascii_digit() {
            return Some(DIGITS[(b[0] - b'0') as usize]);
        }
        if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<usize>().ok()) {
            return FKEYS.get(n.wrapping_sub(1)).copied();
        }
        Some(match name {
            "Shift" => ShiftLeft,
            "Ctrl" => ControlLeft,
            "Alt" => AltLeft,
            "." => Period,
            "," => Comma,
            "Up" => ArrowUp,
            "Down" => ArrowDown,
            "Left" => ArrowLeft,
            "Right" => ArrowRight,
            "Enter" => Enter,
            "Escape" => Escape,
            "Backspace" => Backspace,
            "Space" => Space,
            "PageUp" => PageUp,
            "PageDown" => PageDown,
            "Insert" => Insert,
            "Home" => Home,
            "End" => End,
            "Delete" => Delete,
            "[" => BracketLeft,
            "]" => BracketRight,
            _ => return None,
        })
    }

    /// `OMSI_INPUT`: scripted window input, so the very same handlers the mouse and keyboard
    /// reach can be driven from the command line and checked without a hand on the mouse:
    /// `t=3 move 1045,826; t=3.2 press; t=3.5 drag 0,-80; t=4 release; t=4.5 log bremse_feststell;
    /// t=5 key F3` - coordinates in logical pixels, `drag` relative, `key` a winit key name;
    /// also `look yaw,pitch` (turn the head to), `turn dx,dy` (turn the view by degrees, as a
    /// right-button drag does, the free and outside cameras too), `set name=value` (a script
    /// variable), `trigger name`, `log name` (with what the HUD says about the cursor),
    /// `dumptex <folder>` (the display pictures), `shot <file>` (the window's picture as a
    /// PNG) and `type <text>` (into the LAN chat line that `key V` opened; `key Enter`
    /// sends it).
    pub(crate) fn run_input_script(&mut self, event_loop: &ActiveEventLoop) {
        if self.perf.input_script.is_empty() {
            return;
        }
        let t = self.started.elapsed().as_secs_f32();
        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0);
        while let Some((at, cmd)) = self.perf.input_script.first().cloned() {
            if t < at {
                break;
            }
            self.perf.input_script.remove(0);
            let mut parts = cmd.split_whitespace();
            let verb = parts.next().unwrap_or("");
            let arg = parts.next().unwrap_or("");
            let xy = || -> (f32, f32) {
                let mut it = arg.split(',').filter_map(|v| v.trim().parse::<f32>().ok());
                (it.next().unwrap_or(0.0), it.next().unwrap_or(0.0))
            };
            log::info!("input script t={t:.1}: {cmd}");
            match verb {
                "move" => {
                    let (x, y) = xy();
                    self.on_cursor(x * scale, y * scale);
                }
                // `weather`: the next weather, as the admin menu's "Next weather"
                "weather" => self.next_weather(),
                // `rawmouse dx[,dy]`: the mouse moved by (dx, dy) logical pixels as a locked
                // cursor's raw movement (past the window's edges too, as mouse steering
                // takes it)
                "rawmouse" => {
                    let (dx, dy) = xy();
                    self.steer_by(dx * scale, dy * scale);
                }
                "drag" => {
                    let (dx, dy) = xy();
                    let (x, y) = self.input.cursor;
                    self.on_cursor(x + dx * scale, y + dy * scale);
                }
                // `look yaw,pitch`: turn the head, as a right-drag does
                // `touch down|move|up x,y[,id]`: a finger (logical pixels), as the phone's
                // screen gives it to the on-screen controls (OMSI_TOUCH=1 shows them)
                "touch" => {
                    let rest = parts.next().unwrap_or("");
                    let mut it = rest.split(',').filter_map(|v| v.trim().parse::<f32>().ok());
                    let (x, y) = (it.next().unwrap_or(0.0), it.next().unwrap_or(0.0));
                    let id = it.next().unwrap_or(0.0) as u64;
                    self.script_touch(event_loop, arg, x * scale, y * scale, id);
                }
                "look" => self.cam.look = xy(),
                // `orbit <m>`: how far the outside camera stands off, as the mouse wheel sets it
                "orbit" => self.cam.orbit = xy().0.clamp(ORBIT_MIN, ORBIT_MAX),
                // `set name=value`: put a script variable somewhere (a switch half way)
                "set" => {
                    if let (Some((k, v)), Some(p)) = (arg.split_once('='), self.player.as_mut()) {
                        let ok = p.vehicle.set_var(k.trim(), v.trim().parse().unwrap_or(0.0));
                        log::info!("input script: set {k} -> {ok}");
                    }
                }
                "trigger" => {
                    if let Some(p) = self.player.as_mut() {
                        let ok = p.vehicle.trigger(arg);
                        log::info!("input script: trigger {arg} -> {ok}");
                    }
                }
                // `wheel <notches>`: the mouse wheel, where the placing and the menu take it
                "wheel" => {
                    let n = xy().0;
                    if self.menus.editor.is_some() && self.menus.game_menu.is_none() {
                        self.editor_wheel(n);
                    } else if self.menus.placing.is_some() && self.menus.game_menu.is_none() {
                        self.placing_wheel(n);
                    } else if self.menus.game_menu.is_some() {
                        self.menu_wheel(n);
                    } else {
                        self.wheel(n);
                    }
                    log::info!("input script: wheel {n}: menu line {:?}, chooser {:?}, placing heading {:?}", self.menus.game_menu, self.menus.chooser, self.menus.placing.as_ref().map(|p| p.heading));
                }
                // `click`: a left click where the cursor is, through the window's own path;
                // `click down` / `click up` only press or let go (a drag in between)
                "click" => {
                    let (press, release) = (arg != "up", arg != "down");
                    if self.menus.placing.is_some() && self.menus.game_menu.is_none() {
                        self.placing_click();
                    } else if self.menus.game_menu.is_some() {
                        // (on the menu as the window's button: its lines, its arrows)
                        if press {
                            self.left_button(event_loop, true);
                        }
                        if release {
                            self.left_button(event_loop, false);
                        }
                    } else {
                        if press {
                            self.on_left(true);
                        }
                        if release {
                            self.on_left(false);
                        }
                    }
                    log::info!("input script: click: placing {:?}, placed at {:?}", self.menus.placing.as_ref().map(|p| (p.at, p.blocked)), self.session.placed.last().map(|q| (q.vehicle.position, q.vehicle.heading)));
                }
                // `both down|up`: both mouse buttons held (OMSI's mouse zoom) or let go
                "both" => {
                    if arg == "down" {
                        self.input.buttons_held = (true, true);
                        let started = self.start_both_drag();
                        log::info!("input script: both buttons: zoom drag {started}");
                    } else {
                        self.input.buttons_held = (false, false);
                        self.input.both_drag = None;
                        log::info!("input script: both buttons up: zoom {:?}, orbit {:.1}", self.cam.view_zoom.get(&self.view), self.cam.orbit);
                    }
                }
                // `right down|up`: the right mouse button, through the window's own path
                "right" => {
                    self.on_right(arg == "down");
                    log::info!("input script: right button {arg}: zoom drag {}, look {}, zoom {:?}, orbit {:.1}", self.input.both_drag.is_some(), self.input.mouse_look, self.cam.view_zoom.get(&self.view), self.cam.orbit);
                }
                "press" => self.on_left(true),
                "release" => self.on_left(false),
                // `type <text>`: characters into the open LAN chat line (after `key V`)
                "type" => {
                    let text = cmd.split_once(' ').map(|x| x.1).unwrap_or("");
                    if lan::chat_open(&self.net.remotes) {
                        lan::chat_type(&mut self.net.remotes, text);
                    } else {
                        log::warn!("input script: the chat line is not open");
                    }
                }
                // `turn dx,dy`: turn the view by degrees, as a right-button drag does
                "turn" => {
                    let (dx, dy) = xy();
                    self.look_by(dx, dy);
                }
                // `focus 0|1`, `minimize`, `restore`: the window losing and getting back the
                // keyboard and the mouse, as the window's own events do it
                "focus" if arg == "0" => self.input_lost(),
                "focus" => self.input_back(),
                "minimize" => {
                    self.gfx.window_hidden = true;
                    self.input_lost();
                }
                "restore" => {
                    self.gfx.window_hidden = false;
                    self.input_back();
                }
                "key" | "keydown" | "keyup" => {
                    let Some(code) = Self::script_key(arg) else {
                        log::warn!("input script: unknown key {arg}");
                        continue;
                    };
                    if verb != "keyup" {
                        self.on_key(event_loop, code, true, false);
                    }
                    if verb != "keydown" {
                        self.on_key(event_loop, code, false, false);
                    }
                }
                // `log pose`: where the bus and the camera are, and whether the bus has ground
                "log" if arg == "pose" => {
                    let bus = self.player.as_ref().map(|p| {
                        (
                            p.vehicle.position,
                            p.vehicle
                                .ground
                                .as_ref()
                                .and_then(|g| g(p.vehicle.position.x, p.vehicle.position.y)),
                        )
                    });
                    let tiles = self
                        .world
                        .as_ref()
                        .map(|w| w.loaded_tiles().len())
                        .unwrap_or(0);
                    log::info!("input script: bus at {:?} (ground {:?}), camera at {:?}, {tiles} tiles loaded", bus.map(|b| b.0), bus.and_then(|b| b.1), self.camera.as_ref().map(|c| c.position));
                    // the camera in the bus's own frame (x right, y forward, z up)
                    if let (Some(p), Some(c)) = (self.player.as_ref(), self.camera.as_ref()) {
                        let d = c.position - p.vehicle.position;
                        let h = p.vehicle.heading.to_radians();
                        let (fwd, right) = (glam::DVec2::new(h.sin(), h.cos()), glam::DVec2::new(h.cos(), -h.sin()));
                        log::info!("input script: view {} camera in the bus ({:.2}, {:.2}, {:.2}), on foot {:?}", self.view, d.truncate().dot(right), d.truncate().dot(fwd), d.z, self.session.on_foot.as_ref().map(|f| f.pos));
                    }
                }
                // `log mouse`: the mouse steering's state
                "log" if arg == "mouse" => {
                    log::info!(
                        "input script: mouse steering {} look {} menu {:?} paused {} focused {} steer {:.3} at {:?} pedals {:.3},{:.3} held {:?}",
                        self.input.mouse_drive,
                        self.input.mouse_look,
                        self.menus.game_menu,
                        self.paused,
                        self.input.window_focused,
                        self.input.mouse_steer.0,
                        self.input.mouse_grab.at,
                        self.input.mouse_pedals.0,
                        self.input.mouse_pedals.1,
                        self.input.mouse_grab.mode
                    );
                }
                "log" => {
                    let v = self
                        .player
                        .as_ref()
                        .map(|p| (p.vehicle.var(arg), p.vehicle.str_var(arg)));
                    let names = describe::names(&self.args.root, &self.settings.language);
                    let shown = self
                        .menus.hover
                        .as_deref()
                        .map(|h| names.control(h))
                        .or_else(|| self.menus.hover_part.as_deref().map(|p| names.part(p)));
                    log::info!(
                        "input script: {arg} = {:?}  hover {:?} / {:?} shown as {:?}",
                        v,
                        self.menus.hover,
                        self.menus.hover_part,
                        shown
                    );
                }
                // `menu <what>`: a line of the game menu by its id (`menu remove`, `menu switch`),
                // `menu pick:<n>`: line n of the list open
                "menu" => {
                    if let Some(n) = arg.strip_prefix("pick:").and_then(|n| n.parse::<usize>().ok()) {
                        self.chooser_pick(n);
                    } else {
                        if self.menus.game_menu.is_none() {
                            self.open_game_menu();
                        }
                        // (a line of the vehicle or world pages is done directly)
                        match self.game_menu_items().iter().position(|m| m.0 == arg) {
                            Some(k) => self.menu_choose(event_loop, k),
                            None => {
                                if !self.page_action(arg) {
                                    log::warn!("input script: no menu line {arg}");
                                }
                            }
                        }
                    }
                    let riders = self.session.humans.as_ref().map(|h| (h.people_in(crate::humans::BusId::Player), self.session.placed.iter().map(|q| h.people_in(crate::humans::BusId::Ai(crate::humans::placed_bus_id(q.uid)))).collect::<Vec<_>>()));
                    log::info!("input script: menu {arg}: player {:?}, on foot {:?}, placed {}, people in the bus / the placed ones {:?}", self.player.as_ref().map(|p| p.vehicle.position), self.session.on_foot.as_ref().map(|f| f.pos), self.session.placed.len(), riders);
                }
                // `shot <file>`: the window's own view into a PNG, drawn from the scene the
                // window is showing (the only way to see what the window path renders)
                "shot" => self.perf.shot = Some((PathBuf::from(arg), true)),
                // `dumptex <folder>`: the player's display pictures as the window has them
                "dumptex" => {
                    if let Some(p) = self.player.as_ref() {
                        dump_display_textures(&p.vehicle, Path::new(arg));
                    }
                }
                _ => log::warn!("input script: unknown command {cmd}"),
            }
        }
    }

    /// Let go of every key the vehicle holds: the keyboard's driving keys and pedals, the
    /// vehicle keys of `Inputs/keyboard.cfg` (their `<trigger>_off` fires) and the
    /// Shift+number door buttons.
    pub(crate) fn release_vehicle_keys(&mut self) {
        if let Some(p) = self.player.as_mut() {
            let held: Vec<_> = p.held_keys.keys().copied().collect();
            for scan in held {
                p.key(scan, 0, false);
            }
            p.axes.release_all();
            for fired in self.input.door_key_triggers.drain().map(|(_, g)| g).collect::<Vec<_>>() {
                p.door_key_off(&fired);
            }
        }
        self.input.door_key_triggers.clear();
    }

    /// The window lost the keyboard and the mouse (focus gone to another window, minimised,
    /// hidden, a phone sending the app to the background): no key-up or button-up comes for
    /// what is held now, so all of it is let go here - the vehicle's keys, a switch held
    /// with the mouse, looking round, the mouse's zoom - and the mouse steering holds the
    /// wheel and the brake where they are with its throttle off. Until the window has the
    /// focus again the mouse and the keyboard work nothing (see `input_away`). A game
    /// controller's axes stay theirs.
    pub(crate) fn input_lost(&mut self) {
        self.input.input_away = true;
        self.release_vehicle_keys();
        self.input.keys.clear();
        if let Some(p) = self.player.as_mut() {
            p.release();
        }
        self.input.dragging = false;
        self.input.buttons_held = (false, false);
        self.input.both_drag = None;
        self.input.mouse_look = false;
        self.input.steer_cursor = None;
        self.input.mouse_pedals.0 = 0.0;
        // (the cursor held for the mouse steering goes back to the system)
        self.sync_mouse_grab();
    }

    /// The window has the focus again: the mouse steering eases from where the wheel stands
    /// to the cursor (as when it is switched on) instead of jumping there. A key still held
    /// from before counts only once it is pressed again.
    pub(crate) fn input_back(&mut self) {
        if !self.input.input_away {
            return;
        }
        self.input.input_away = false;
        self.input.mouse_steer = (self.player.as_ref().map(|p| p.vehicle.physics.controls.steering).unwrap_or(0.0), 1.0);
    }

    /// OMSI's `sim_pause`: the simulation stands still, the camera and the picture go on.
    /// Shift a manual gearbox up or down: the first of the usual trigger names the bus's
    /// scripts have (pressed and let go). False when it has none.
    /// Whether `code` with these modifiers is one of the keyboard file's keys, of the game's
    /// or of the bus's: then a built-in shortcut on it stands back.
    pub(crate) fn chord_bound(&self, code: KeyCode, shift: bool, ctrl: bool, alt: bool) -> bool {
        let m = omsi_content::input::chord(shift, ctrl, alt);
        let Some(scan) = keys::dik_code(code) else { return false };
        self.input.game_keys.iter().any(|b| b.scan_code == scan && b.matches(m))
            || self.player.as_ref().is_some_and(|p| p.bindings.iter().any(|b| b.scan_code == scan && b.matches(m)))
    }
}

/// Write a vehicle's `[scripttexture]` images and `[texttexture]` pictures (with the text
/// they show in the log) into `dir`, to check destination displays and the IBIS.
pub(crate) fn dump_display_textures(vehicle: &omsi_sim::VehicleInstance, dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    for (i, st) in vehicle.host.script_textures.iter().enumerate() {
        let _ = image::save_buffer(
            dir.join(format!("scripttex_{i}.png")),
            &st.rgba,
            st.width,
            st.height,
            image::ColorType::Rgba8,
        );
    }
    for (i, tt) in vehicle.text_textures.iter().enumerate() {
        let text = tt.last_text.clone().unwrap_or_default();
        log::info!(
            "text texture {i} ({} in \"{}\"): {text:?}",
            tt.def.variable,
            tt.def.font
        );
        let (w, h) = (tt.def.width.max(1) as u32, tt.def.height.max(1) as u32);
        if tt.atlas.is_some() {
            let _ = image::save_buffer(
                dir.join(format!("texttex_{i}.png")),
                &tt.image(&text),
                w,
                h,
                image::ColorType::Rgba8,
            );
        }
    }
}

/// `OMSI_INPUT="t=3 move 1045,826; t=3.2 press; ..."` → (time, command) pairs.
pub(crate) fn parse_input_script() -> Vec<(f32, String)> {
    let Some(v) = omsi_cfg::flags::OMSI_INPUT.var() else {
        return Vec::new();
    };
    let mut out: Vec<(f32, String)> = v
        .split(';')
        .filter_map(|item| {
            let item = item.trim();
            let rest = item.strip_prefix("t=")?;
            let (t, cmd) = rest.split_once(' ')?;
            Some((t.trim().parse::<f32>().ok()?, cmd.trim().to_string()))
        })
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// The keys that fly the free camera (and, with no bus of one's own, the view).
pub(crate) fn flies_free_camera(code: KeyCode) -> bool {
    matches!(code, KeyCode::KeyW | KeyCode::KeyA | KeyCode::KeyS | KeyCode::KeyD | KeyCode::KeyQ | KeyCode::KeyE | KeyCode::Space | KeyCode::ShiftLeft | KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::ArrowUp | KeyCode::ArrowDown)
}

/// `App::key_left_free` for a key's scan code.
pub(crate) fn key_left_free(scan: Option<i32>, action: &str, own: &std::collections::HashSet<i32>, game: &[omsi_content::KeyBinding]) -> bool {
    !scan.is_some_and(|s| own.contains(&s)) && !game.iter().any(|b| b.scan_code != 0 && b.action.eq_ignore_ascii_case(action))
}

/// Shift, Ctrl, Alt and the system key, either side: the keys `ModifiersChanged` keeps.
pub(crate) fn is_modifier(code: KeyCode) -> bool {
    matches!(code, KeyCode::ShiftLeft | KeyCode::ShiftRight | KeyCode::ControlLeft | KeyCode::ControlRight
        | KeyCode::AltLeft | KeyCode::AltRight | KeyCode::SuperLeft | KeyCode::SuperRight)
}

/// OS shortcuts can consume modifier key-ups without taking window focus. Winit then
/// reports the released modifiers through `ModifiersChanged` (on macOS, also before
/// the next ordinary key event). Remove those stale keys and return them so their
/// normal release handlers run, including vehicle keys and Shift+number door buttons.
pub(crate) fn release_inactive_modifiers(
    keys: &mut hashbrown::HashSet<KeyCode>,
    modifiers: winit::keyboard::ModifiersState,
) -> Vec<KeyCode> {
    use winit::keyboard::ModifiersState as M;
    let mut released = Vec::new();
    for (flag, left, right) in [
        (M::SHIFT, KeyCode::ShiftLeft, KeyCode::ShiftRight),
        (M::CONTROL, KeyCode::ControlLeft, KeyCode::ControlRight),
        (M::ALT, KeyCode::AltLeft, KeyCode::AltRight),
        (M::SUPER, KeyCode::SuperLeft, KeyCode::SuperRight),
    ] {
        if !modifiers.contains(flag) {
            for code in [left, right] {
                if keys.remove(&code) { released.push(code); }
            }
        }
    }
    released
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use winit::keyboard::ModifiersState as M;

    #[test]
    fn screenshot_modifier_release_without_focus_loss_restores_wasd() {
        // Captured on macOS: Shift down, Cmd down, screenshot, ModifiersChanged(empty),
        // W down. There was no Focused(false) or modifier KeyboardInput release.
        let mut keys: hashbrown::HashSet<KeyCode> = [KeyCode::ShiftLeft, KeyCode::SuperLeft].into();
        assert!(keys.contains(&KeyCode::ShiftLeft)); // W would reach the wipers.
        let released = release_inactive_modifiers(&mut keys, M::empty());
        assert_eq!(released, [KeyCode::ShiftLeft, KeyCode::SuperLeft]);
        keys.insert(KeyCode::KeyW);
        assert!(!shift_held_now(&keys));
        assert_eq!(fallback_action(KeyCode::KeyW, "wasd"), Some(omsi_sim::input::EngineAction::Throttle));
        assert!(release_inactive_modifiers(&mut keys, M::empty()).is_empty());
        assert!(keys.contains(&KeyCode::KeyW));
    }

    #[test]
    fn modifier_state_preserves_held_chords_and_releases_both_sides() {
        let mut keys = [KeyCode::ShiftLeft, KeyCode::ShiftRight, KeyCode::ControlRight,
            KeyCode::AltRight, KeyCode::SuperRight, KeyCode::KeyW].into();
        assert_eq!(release_inactive_modifiers(&mut keys, M::SHIFT | M::CONTROL),
            [KeyCode::AltRight, KeyCode::SuperRight]);
        assert!(shift_held_now(&keys));
        assert!(keys.contains(&KeyCode::ControlRight));
        assert_eq!(release_inactive_modifiers(&mut keys, M::empty()),
            [KeyCode::ShiftLeft, KeyCode::ShiftRight, KeyCode::ControlRight]);
        assert_eq!(keys, [KeyCode::KeyW].into());
        // A modifier held while focus returns must not fabricate a new key press.
        keys.clear();
        assert!(release_inactive_modifiers(&mut keys, M::SHIFT).is_empty());
        assert!(keys.is_empty());
    }

    /// The modifier key-ups the window drops once `ModifiersChanged` has let them go (Windows
    /// reports the state first): the eight modifier keys, nothing else.
    #[test]
    fn only_the_modifier_keys_count_as_modifiers() {
        for code in [KeyCode::ShiftLeft, KeyCode::ShiftRight, KeyCode::ControlLeft, KeyCode::ControlRight,
            KeyCode::AltLeft, KeyCode::AltRight, KeyCode::SuperLeft, KeyCode::SuperRight] {
            assert!(is_modifier(code), "{code:?}");
        }
        for code in [KeyCode::KeyW, KeyCode::CapsLock, KeyCode::Escape, KeyCode::Digit1] {
            assert!(!is_modifier(code), "{code:?}");
        }
    }

    /// F1 given to a door and the driver's view moved to 1 (#701): F1 is not the view any more.
    #[test]
    fn a_built_in_view_key_steps_aside_for_the_players_own() {
        use omsi_content::KeyBinding;
        let kb = |a: &str, k: i32| KeyBinding { action: a.into(), scan_code: k, modifier: 0 };
        let none = std::collections::HashSet::new();
        assert!(super::key_left_free(Some(59), "view_set_driver", &none, &[]));
        let own: std::collections::HashSet<i32> = [59].into();
        assert!(!super::key_left_free(Some(59), "view_set_driver", &own, &[]));
        assert!(!super::key_left_free(Some(59), "view_set_driver", &none, &[kb("view_set_driver", 2)]));
        assert!(super::key_left_free(Some(59), "view_set_driver", &none, &[kb("view_set_passenger", 60)]));
    }
}
