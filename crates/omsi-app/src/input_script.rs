//! `OMSI_INPUT`: scripted keyboard, mouse and camera input for window runs, and its key names.

use super::*;


/// Global actions a controller button should send to the game instead of to the bus script.
pub(crate) fn is_game_action(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("view_")
        || matches!(
            name.as_str(),
            "sim_pause" | "screenshot" | "quicksave" | "toggel_mouse_ctrl" | "toggel_ctrler"
        )
}


/// How far (m) a click reaches a page (`[htmltexture]`) on a scenery object.
const HTML_OBJECT_REACH: f32 = 4.0;

impl App {
    /// Save the personnel file and the session summary (once: every caller ends the game,
    /// and the frames the loop still runs before it stops count no more time).
    pub(crate) fn finish_session(&mut self) {
        self.exiting = true;
        // (the tiles loaded on the way added to what the map lacks)
        if let Some(w) = self.world.clone() {
            let mut none = None;
            crate::app::report_missing_content(&w, &mut none);
        }
        // PluginFinalize, as OMSI calls it on the way out
        if let Some(mut p) = self.plugins.take() {
            p.finalize();
        }
        if self.player.is_none() || self.career.seconds <= 0.0 {
            return;
        }
        // the situation to continue next time (OMSI writes it when a map is left; once, as
        // the rest: `career.seconds` is zero after the first time)
        self.save_last_situation();
        if self.career.path.is_some() {
            if let Err(e) = self.career.save() {
                log::warn!("writing the personnel file: {e}");
            }
        }
        let bus = self.args.bus.clone().unwrap_or_default();
        if let Err(e) = self.career.write_session(
            &self.args.map,
            &bus,
            self.args.line.as_deref(),
            self.args.tour.as_deref(),
        ) {
            log::warn!("writing the session summary: {e}");
        }
        self.career.seconds = 0.0;
    }

    /// A key of the window, or of an `OMSI_INPUT` script.
    pub(crate) fn on_key(&mut self, event_loop: &ActiveEventLoop, code: KeyCode, pressed: bool, repeat: bool) {
        if self.vr_nav_edit.is_some() {
            if !pressed { self.keys.remove(&code); }
            if matches!(code, KeyCode::ControlLeft | KeyCode::ControlRight | KeyCode::ShiftLeft | KeyCode::ShiftRight) && pressed {
                self.keys.insert(code);
            }
            if pressed && !repeat {
                match code {
                    KeyCode::Escape | KeyCode::Enter => self.finish_vr_nav_edit(),
                    KeyCode::KeyR => self.vr_nav_adjust("reset", 1.0),
                    _ => {}
                }
            }
            return;
        }
        // Escape closes the city map first (it would end the session)
        if pressed && code == KeyCode::Escape {
            if let Some(n) = self.navigator.as_mut().filter(|n| n.map_open()) {
                n.toggle_map();
                return;
            }
        }
        let event_key = PhysicalKey::Code(code);
        if let (Some(m), PhysicalKey::Code(code)) = (self.menu.as_mut(), event_key) {
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
                    self.menu = None;
                    self.hud = None;
                    self.load_world_now(event_loop);
                }
            }
            return;
        }
        if let PhysicalKey::Code(code) = event_key {
            let pressed = pressed;
            // LAN chat: V opens the line, and while it is open the keys are its own
            if let Some(l) = self.lan.as_mut() {
                let modifiers = [
                    KeyCode::ShiftLeft,
                    KeyCode::ShiftRight,
                    KeyCode::ControlLeft,
                    KeyCode::ControlRight,
                    KeyCode::AltLeft,
                    KeyCode::AltRight,
                    KeyCode::SuperLeft,
                    KeyCode::SuperRight,
                ]
                    .iter()
                    .any(|k| self.keys.contains(k));
                if lan::chat_key(l, &mut self.remotes, code, pressed, repeat, modifiers) {
                    return;
                }
            }
            if pressed && !repeat {
                self.keys.insert(code);
            } else if !pressed {
                self.keys.remove(&code);
            }
            #[cfg(windows)]
            if pressed && !repeat && (self.vr.is_some() || self.settings.vr_requested()) {
                let modifier = omsi_content::input::chord(
                    self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight),
                    self.keys.contains(&KeyCode::ControlLeft) || self.keys.contains(&KeyCode::ControlRight),
                    self.keys.contains(&KeyCode::AltLeft) || self.keys.contains(&KeyCode::AltRight),
                );
                let action = keys::dik_code(code).and_then(|scan| self.game_keys.iter()
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
                    self.door_key_triggers.remove(&code).into_iter().collect()
                } else if matches!(code, KeyCode::ShiftLeft | KeyCode::ShiftRight) {
                    self.door_key_triggers.drain().map(|(_, g)| g).collect()
                } else {
                    Vec::new()
                };
                if let Some(p) = self.player.as_mut() {
                    for name in released.iter().flatten() {
                        let off = format!("{name}_off");
                        if p.vehicle.ty.program.trigger(&off).is_some() {
                            p.vehicle.trigger(&off);
                        }
                    }
                }
            }
            // placing a vehicle with the mouse: its keys first (Escape takes it away)
            if self.game_menu.is_none() && self.placing_key(code, pressed) {
                return;
            }
            // the game menu: Escape opens it (and pauses, except in a LAN session, which
            // goes on for the others), and while it is open the keys are its own
            if self.game_menu.is_some() {
                if pressed && !repeat {
                    self.menu_key(event_loop, code);
                }
                return;
            }
            // the object editor takes its keys first (Escape leaves it)
            if pressed && self.editor.is_some() && self.editor_key(code) {
                return;
            }
            if pressed && !repeat && code == KeyCode::Escape {
                self.open_game_menu();
                return;
            }
            // a tutorial's pages: Enter / Page Down on, Page Up back, Ctrl+T hides them
            if let (true, Some(t)) = (pressed, self.tutorial.as_mut()) {
                let ctrl = self.keys.contains(&KeyCode::ControlLeft) || self.keys.contains(&KeyCode::ControlRight);
                match code {
                    KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::PageDown if !t.hidden && self.lan.is_none() => {
                        t.next();
                        return;
                    }
                    KeyCode::PageUp if !t.hidden => {
                        t.back();
                        return;
                    }
                    KeyCode::KeyT if ctrl => {
                        t.hidden = !t.hidden;
                        return;
                    }
                    _ => {}
                }
            }
            let ctrl = self.keys.contains(&KeyCode::ControlLeft) || self.keys.contains(&KeyCode::ControlRight);
            let alt = self.keys.contains(&KeyCode::AltLeft) || self.keys.contains(&KeyCode::AltRight);
            let shift_now = self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight);
            // getting up (Ctrl+Shift+G) and, on foot, the walker's keys
            if self.foot_key(code, pressed, repeat, ctrl, shift_now) {
                return;
            }
            // OMSI's global actions as `Inputs/keyboard.cfg` binds them ([game]); a key our
            // driving layout uses keeps that meaning (with the OMSI layout, every binding
            // counts)
            if pressed && !repeat {
                let m = omsi_content::input::chord(shift_now, ctrl, alt);
                let own = keys::dik_code(code).is_some_and(|s| self.own_keys.contains(&s));
                let ours = self.args.drive_keys != "omsi"
                    && m == 0
                    && !own
                    && (fallback_action(code, &self.args.drive_keys).is_some()
                    || matches!(code, KeyCode::KeyZ | KeyCode::KeyX | KeyCode::KeyC | KeyCode::KeyI | KeyCode::KeyL));
                // plain Left/Right are OMSI's view_interiorcam_minus/plus, except when a wheel
                // steers: then the arrows glance (held, the head turns) and only Ctrl+Left/Right
                // switch the interior camera, below. (Where the arrows drive, `ours` skips this.)
                let plain_arrow = matches!(code, KeyCode::ArrowLeft | KeyCode::ArrowRight) && !ctrl
                    && self.controllers.as_ref().is_some_and(|c| c.wheel_steering());
                if let Some(scan) = keys::dik_code(code).filter(|_| !ours) {
                    let action = self.game_keys.iter().find(|b| b.scan_code == scan && b.matches(m)
                        && !b.action.starts_with("vr_")
                        && !(plain_arrow && b.action.starts_with("view_interiorcam_"))).map(|b| b.action.clone());
                    if let Some(a) = action {
                        if self.game_action(&a) {
                            return;
                        }
                    }
                }
            }
            if pressed && !repeat {
                match code {
                    // OMSI's `toggel_mouse_ctrl` (O): steering and pedals with the mouse
                    KeyCode::KeyO if !ctrl && !alt && !shift_now => {
                        self.game_action("toggel_mouse_ctrl");
                        return;
                    }
                    // the interior cameras: Ctrl+Left/Right (the arrows drive)
                    // a manual gearbox: Ctrl+Up / Ctrl+Down shift up and down - the stock key file
                    // has no keys for it, and a bus like the LiAZ MKPP stayed in its gear
                    KeyCode::ArrowUp | KeyCode::ArrowDown if ctrl && !alt => {
                        let up = code == KeyCode::ArrowUp;
                        self.shift_gear(up);
                        return;
                    }
                    // (Ctrl+Alt+arrows turn the mirror looked at, see the frame)
                    KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::ArrowUp | KeyCode::ArrowDown if ctrl && alt => return,
                    KeyCode::ArrowLeft if ctrl => {
                        self.game_action("view_interiorcam_minus");
                        return;
                    }
                    KeyCode::ArrowRight if ctrl => {
                        self.game_action("view_interiorcam_plus");
                        return;
                    }
                    // OMSI's `screenshot` (Ctrl+Shift+P: 25 / 6), and F12 as most games have it
                    KeyCode::KeyP if ctrl && shift_now => {
                        self.take_screenshot();
                        return;
                    }
                    // (F12 alone only where the bus has no key of its own on it: in OMSI's
                    // keyboard.cfg it is the pram/wheelchair button, which it took away)
                    KeyCode::F12 if !self.player.as_ref().is_some_and(|p| p.bindings.iter().any(|b| b.scan_code == 88 && b.chord() == 0 && p.vehicle.ty.program.trigger(&b.action).is_some())) => {
                        self.take_screenshot();
                        return;
                    }

                    // the object editor (`crate::editor`)
                    KeyCode::KeyE if ctrl && shift_now => {
                        self.toggle_editor();
                        return;
                    }
                    // OMSI's `sim_pause`
                    KeyCode::KeyP if !ctrl && !alt && !shift_now => {
                        self.toggle_pause();
                        return;
                    }
                    // OMSI's `quicksave` (Alt+S)
                    KeyCode::KeyS if alt && !ctrl => {
                        self.quick_save();
                        return;
                    }
                    // OMSI's `view_toggle_informationdisplay` (Ctrl+Y)
                    // OMSI's `view_toggle_informationdisplay` (Shift+Y: 21 / 2)
                    KeyCode::KeyY if shift_now && !ctrl => {
                        self.info_bar = !self.info_bar;
                        return;
                    }
                    // OMSI's `view_set_schedule` (Insert: 210 / 1, the key's state every frame)
                    KeyCode::Insert if !shift_now && !ctrl => {
                        self.timetable = !self.timetable;
                        return;
                    }
                    _ => {}
                }
            }
            // the extra keys of the ready-made layouts (below): not with Custom controls, not on
            // a key the player bound, not with a modifier held
            let extras = self.args.drive_keys != "omsi"
                && !keys::dik_code(code).is_some_and(|s| self.own_keys.contains(&s))
                && !self.keys.iter().any(|k| matches!(k, KeyCode::ControlLeft | KeyCode::ControlRight | KeyCode::AltLeft | KeyCode::AltRight | KeyCode::ShiftLeft | KeyCode::ShiftRight));
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
                if self.view != "free" && shift_held_now(&self.keys) && !keys::dik_code(code).is_some_and(|s| self.own_shift.contains(&s)) {
                    if let Some(n) = digit_of(code) {
                        if let Some(p) = self.player.as_mut() {
                            let groups = crate::player::door_keys(&p.vehicle.ty);
                            if let Some(group) = groups.get(n - 1) {
                                let fire = crate::player::door_group_to_fire(&mut p.vehicle, group);
                                log::info!("door key Shift+{n}: {}", fire.join(" + "));
                                // the automatic rear doors of the stock Berlin buses (SD, NL): the
                                // key is their release, and switched off with the doors open it
                                // shuts them now rather than when the last request has lapsed
                                // ("why can I not close the rear doors at all?")
                                if group.len() == 1 && group[0] == "bus_dooraft" {
                                    let v = &mut p.vehicle;
                                    let release_on = v.var("bremse_halte_sw").is_some_and(|x| x > 0.5);
                                    let open = v.var("doorTarget_23").is_some_and(|x| x > 0.5);
                                    if release_on && open && v.var("doorAftLastOpen").is_some() {
                                        v.set_var("haltewunsch", 0.0);
                                        v.set_var("doorAftLastOpen", 1000.0);
                                    }
                                }
                                for name in &fire {
                                    p.vehicle.trigger(name);
                                }
                                self.door_key_triggers.insert(code, fire);
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
                match code {
                    KeyCode::F1 => self.view = "driver".into(),
                    KeyCode::F2 => self.view = "pax".into(),
                    KeyCode::F3 => self.view = "outside".into(),
                    KeyCode::F4 => {
                        // the free camera starts where the current view is looking
                        self.view = "free".into();
                        self.ego = false;
                    }
                    KeyCode::KeyU
                    if self.keys.contains(&KeyCode::ShiftLeft)
                        || self.keys.contains(&KeyCode::ShiftRight) =>
                        {
                            // Shift+U: toggle a bus's service state by itself (start a shut
                            // bus, shut down a running one), and set its IBIS to the current
                            // duty as the driver would do while putting it into service.
                            if let Some(p) = self.player.as_mut() {
                                let msg = p.start_up();
                                self.service_msg = Some((msg, 6.0));
                                if let Some(d) = self.duty.as_ref() {
                                    let (trip, stop) = d.trip_for_ibis();
                                    p.set_duty_destination(trip, stop);
                                }
                            }
                        }

                    KeyCode::KeyR
                    if self.keys.contains(&KeyCode::ShiftLeft)
                        || self.keys.contains(&KeyCode::ShiftRight) =>
                        {
                            // Shift+R: the next internet radio station (see radio.rs)
                            let msg = self.radio.next_station();
                            self.service_msg = Some((msg, 4.0));
                        }
                    KeyCode::KeyM
                    if self.keys.contains(&KeyCode::ShiftLeft)
                        || self.keys.contains(&KeyCode::ShiftRight) =>
                        {
                            // Shift+M: the city map (M alone is the starter)
                            if let Some(n) = self.navigator.as_mut() {
                                n.toggle_map();
                            }
                        }
                    KeyCode::KeyN
                    if self.keys.contains(&KeyCode::ShiftLeft)
                        || self.keys.contains(&KeyCode::ShiftRight) =>
                        {
                            // Shift+N: navigator → navigator with the schedule → off (N alone is
                            // the gearbox's neutral)
                            if self.vr_active() {
                                if !self.vr_nav_profile().enabled {
                                    self.vr_nav_adjust("enabled", 1.0);
                                } else if self.navigator.as_ref().is_some_and(|n| n.schedule) {
                                    if let Some(n) = self.navigator.as_mut() { n.schedule = false; }
                                    self.vr_nav_adjust("enabled", 1.0);
                                } else if let Some(n) = self.navigator.as_mut() {
                                    n.schedule = true;
                                }
                                return;
                            }
                            if let Some(n) = self.navigator.as_mut() {
                                match (n.enabled, n.schedule) {
                                    (true, false) => n.schedule = true,
                                    (true, true) => {
                                        n.enabled = false;
                                        n.schedule = false;
                                    }
                                    _ => n.enabled = true,
                                }
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
                        let line = self.career.summary();
                        if self.career.path.is_some() {
                            if let Err(e) = self.career.save() {
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
            // the arrow keys drive when a bus is being driven (the free camera keeps them)
            // (a key the player bound to something else is theirs, not the preset's)
            let own = keys::dik_code(code).is_some_and(|s| self.own_keys.contains(&s));
            let wheel = self.controllers.as_ref().is_some_and(|c| c.wheel_steering());
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
                self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight);
            if let Some(p) = self.player.as_mut() {
                let ctrl_alt_held = (self.keys.contains(&KeyCode::ControlLeft) || self.keys.contains(&KeyCode::ControlRight)) && (self.keys.contains(&KeyCode::AltLeft) || self.keys.contains(&KeyCode::AltRight));
                if self.view != "free" && !repeat && !shift_held && !(ctrl_alt_held && pressed) {
                    if let Some(a) = fallback_action(code, wasd) {
                        p.axes.set(a, pressed);
                    }
                }
            }
            // A driving key held with shift is the vehicle key it covers: Shift+W is
            // OMSI's wiper key, Shift+D selects the automatic's D, Shift+S the
            // viewpoint - otherwise a bus driven with WASD could never be put in gear.
            let shift =
                self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight);
            let covers_vehicle_key = fallback_action(code, wasd).is_some() && self.view != "free";
            let driving_key = covers_vehicle_key && !shift;
            // (the keys that fly the free camera are the camera's: W switched the wipers on
            // while flying)
            let fly_key = self.view == "free"
                && matches!(code, KeyCode::KeyW | KeyCode::KeyA | KeyCode::KeyS | KeyCode::KeyD | KeyCode::KeyQ | KeyCode::KeyE | KeyCode::Space | KeyCode::ShiftLeft | KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::ArrowUp | KeyCode::ArrowDown);
            if let (Some(p), Some(scan)) = (
                self.player.as_mut(),
                keys::dik_code(code).filter(|_| !driving_key && !fly_key),
            ) {
                if !repeat {
                    let m = if covers_vehicle_key {
                        0
                    } else {
                        omsi_content::input::chord(
                            shift,
                            self.keys.contains(&KeyCode::ControlLeft) || self.keys.contains(&KeyCode::ControlRight),
                            self.keys.contains(&KeyCode::AltLeft) || self.keys.contains(&KeyCode::AltRight),
                        )
                    };
                    p.key(scan, m, pressed);
                }
            }
        }
    }

    /// LAN play, once a frame: send our bus, take in the others', and keep a drawn
    /// vehicle for each of them.
    pub(crate) fn tick_lan(&mut self, dt: f32) {
        let walker = self.walker_pose();
        let Some(lan) = self.lan.as_mut() else { return };
        let duty = self
            .duty
            .as_ref()
            .map(|d| &d.trips[d.trip_index])
            .map(|t| (t.line.as_str(), t.terminus.as_str()));
        let frame = lan::Frame {
            audio: self.audio.as_ref(),
            listener: self.camera.as_ref().map(|c| c.position),
            muffled: self.in_cab || self.inside_remote.is_some(),
            riders: self.humans.as_ref().map(|h| h.riding()).unwrap_or(0),
            clock: Some(&self.clock),
            tour: self.duty.as_ref().map(|d| format!("{}/{}", d.line, d.tour)),
            walker,
            inside_of: self.inside_remote,
        };
        let updates = lan::tick(
            lan,
            &mut self.remotes,
            dt,
            &self.args,
            self.player.as_mut(),
            self.world.as_deref(),
            self.renderer.as_ref(),
            self.scene.as_mut(),
            self.traffic.as_mut(),
            self.humans.as_mut(),
            duty,
            &frame,
        );
        for u in updates {
            self.apply_world_update(u);
        }
        let cmds = self.lan.as_mut().map(|l| l.take_commands()).unwrap_or_default();
        for (from, text) in cmds {
            self.lan_command(from, &text);
        }
    }

    /// A command another player's game sent ours (`LanSession::command`).
    pub(crate) fn lan_command(&mut self, from: u32, text: &str) {
        let Some(lan) = self.lan.as_ref() else { return };
        let my_id = lan.my_id;
        if let Some(ev) = text.strip_prefix("trigger ") {
            // a switch worked by a passenger of ours: only by one who is in our bus
            let aboard = lan.peers().find(|p| p.pose.id == from).and_then(|p| p.pose.walker).and_then(|w| w.aboard).map(|a| a.owner == my_id).unwrap_or(false);
            if !aboard {
                log::info!("LAN: player {from} asked for switch {ev} of our bus from outside it: ignored");
                return;
            }
            if let Some(p) = self.player.as_mut() {
                log::info!("LAN: player {from} works {ev} in our bus");
                p.vehicle.trigger(ev.trim());
            }
            return;
        }
        crate::admin::command(self, from, text);
    }

    /// The host's world as LAN play asks for it: its clock (set or caught up with) and its
    /// weather, for everything that keeps a clock of its own.
    pub(crate) fn apply_world_update(&mut self, u: lan::WorldUpdate) {
        match u {
            lan::WorldUpdate::Clock {
                year,
                day_of_year,
                time,
            } => {
                self.clock.year = year;
                self.clock.day_of_year = day_of_year;
                self.clock.time = time;
                if let Some(t) = self.traffic.as_mut() {
                    t.day_time = time;
                }
            }
            lan::WorldUpdate::Slew(s) => {
                self.clock.time = (self.clock.time + s).clamp(0.0, 86399.999);
                if let Some(t) = self.traffic.as_mut() {
                    t.day_time += s;
                }
            }
            lan::WorldUpdate::Weather(w) => {
                log::info!(
                    "LAN: the host's weather: {}",
                    w.as_deref().unwrap_or("the map's default")
                );
                // (coming over to it as the host does, not at a stroke - the streets stay
                // as wet as they are and dry or wet with it)
                self.change_weather(w, false, 240.0);
            }
            lan::WorldUpdate::Tours(tours) => {
                if let Some(s) = self.schedule.as_mut() {
                    s.set_lan_tours(tours);
                }
            }
        }
        if let Some(p) = self.player.as_mut() {
            p.vehicle.host.clock = self.clock.clone();
        }
    }

    /// Turn the view by (dx, dy) degrees, as dragging with the right button does: the free
    /// camera turns, inside the bus the head turns, outside the camera swings around it.
    /// Keeps `look` with the view it belongs to: on a change of view the direction of the
    /// view left is put away and the one of the view entered comes back (straight ahead
    /// the first time).
    pub(crate) fn sync_view_look(&mut self) {
        let key = self.look_key();
        swap_view_look(&mut self.look, &mut self.view_looks, &mut self.look_view, &key);
    }

    /// Which camera the look belongs to: the view, and for the driver's and the passengers'
    /// view the camera chosen in it. Each of Omsi.exe's cameras keeps where it was turned
    /// (a `TCamera` has its own yaw and pitch besides the file's, 0x7edde4 resets them): the
    /// look went back to straight ahead whenever the viewpoint changed.
    pub(crate) fn look_key(&self) -> String {
        look_key_of(&self.view, self.player.as_ref().map(|p| p.cam_choice))
    }

    /// Zoom the view inside the bus by `notches` of the mouse wheel (in: positive).
    pub(crate) fn zoom_by(&mut self, notches: f32) {
        let z = self.view_zoom.entry(self.view.clone()).or_insert(1.0);
        *z = (*z * (1.0 - 0.08 * notches.clamp(-5.0, 5.0))).clamp(0.2, 1.6);
    }

    pub(crate) fn look_by(&mut self, dx: f32, dy: f32) {
        self.sync_view_look();
        // a hand on the view cancels an eased Space return.
        self.f1_reset = None;
        if self.view == "foot" {
            self.foot_look(dx, dy);
            return;
        }
        if self.view == "free" || self.player.is_none() {
            if let Some(cam) = self.camera.as_mut() {
                cam.yaw = (cam.yaw + dx).rem_euclid(360.0);
                cam.pitch = (cam.pitch - dy).clamp(-89.0, 89.0);
            }
        } else if self.view == "outside" {
            // F3 chase orbit: full turn in yaw; pitch stops between near
            // top-down and just below eye level so the camera never swings
            // under the bus (see `chase_orbit_step` for the mouse gain).
            self.look.0 = (self.look.0 + dx).rem_euclid(360.0);
            self.look.1 = (self.look.1 - dy).clamp(-60.0, 25.0);
        } else {
            self.look.0 = (self.look.0 + dx).clamp(-140.0, 140.0);
            self.look.1 = (self.look.1 - dy).clamp(-85.0, 85.0);
        }
    }

    /// The cursor moved, and the switch under it is named at once (touch input and
    /// `OMSI_INPUT` scripts read `hover` right after).
    pub(crate) fn on_cursor(&mut self, x: f32, y: f32) {
        if self.move_cursor(x, y) {
            self.update_hover();
        }
    }

    /// The window's `CursorMoved`: only the cursor is taken; the switch under it is named
    /// once a frame (`RedrawRequested`). A gaming mouse sends 500-8000 moves a second, and a
    /// ray through every cockpit mesh for each of them kept the event queue from ever
    /// draining - no frame was drawn while the mouse moved.
    /// The indicator lever: 1 left, 2 right, 3 the hazard lights - each a toggle, as the
    /// Z / C / X keys and the phone's buttons work it.
    pub(crate) fn blinker(&mut self, want: u8) {
        if let Some(player) = self.player.as_mut() {
            player.toggle_indicator(want);
        }
    }

    /// The right mouse button (or both) held in a view of the bus: start OMSI's mouse zoom
    /// (false when there is nothing to zoom - a menu, the city map, on foot).
    pub(crate) fn start_both_drag(&mut self) -> bool {
        if self.game_menu.is_some() || self.player.is_none() || self.navigator.as_ref().is_some_and(|n| n.map_open()) {
            return false;
        }
        let value = match self.view.as_str() {
            "outside" => self.orbit,
            "driver" | "pax" => *self.view_zoom.get(&self.view).unwrap_or(&1.0),
            _ => return false,
        };
        self.both_drag = Some((self.cursor.1, value));
        self.mouse_look = false;
        self.update_hover();
        true
    }

    /// Looking round with the mouse goes by the cursor's way in the window (a view of the
    /// bus); on foot and with the free camera it keeps the raw mouse movement.
    pub(crate) fn cursor_looks(&self) -> bool {
        self.mouse_look && self.player.is_some() && !matches!(self.view.as_str(), "foot" | "free")
    }

    /// The right button alone zooms, as in Omsi.exe (TForm_main.Panel1MouseMove 0x82c5f8:
    /// ssRight without `[altView]`, or Shift+right with it); otherwise it turns the view.
    pub(crate) fn right_zooms(&self) -> bool {
        !self.settings.alt_view || self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight)
    }

    /// The right mouse button on the desktop: held, it zooms (the outside camera's distance,
    /// the view in the bus), or with OMSI's `[altView]` turns the view; the middle button
    /// turns it in any case. Where there is nothing to zoom it turns the view.
    pub(crate) fn on_right(&mut self, pressed: bool) {
        self.buttons_held.1 = pressed;
        // the left button already down on nothing it works: both held zoom
        if pressed && self.buttons_held.0 && !self.dragging && self.start_both_drag() {
            return;
        }
        // (a switch held with the left button keeps the mouse: looking round
        // took the cursor's movement away from it, and the drag stopped)
        if pressed && self.dragging {
            return;
        }
        if !pressed {
            self.both_drag = None;
        }
        // a right click lets go of the mouse steering as in OMSI (#162) when the player
        // wants it so; otherwise the right button looks round and the wheel and pedals stay
        // where the mouse left them (it went off with every look round, and with every
        // look round in the pause)
        if pressed && self.mouse_drive && self.game_menu.is_none() && self.settings.mouse_right_off && !self.paused {
            self.set_mouse_drive(false);
            self.service_msg = Some(("Mouse steering off".into(), 3.0));
        }
        if pressed && self.right_zooms() && self.start_both_drag() {
            return;
        }
        if self.mouse_drive && self.game_menu.is_none() {
            if pressed {
                self.steer_cursor = Some(self.cursor);
            } else if let Some((x, y)) = self.steer_cursor.take() {
                self.cursor = (x, y);
                if let Some(win) = self.window.as_ref() {
                    let _ = win.set_cursor_position(winit::dpi::PhysicalPosition::new(x as f64, y as f64));
                }
            }
        }
        self.mouse_look = pressed;
        // (the cursor shows it at once, not with the next look at what is under it)
        self.update_hover();
    }

    pub(crate) fn on_mouse_moved(&mut self, x: f32, y: f32) {
        if self.move_cursor(x, y) {
            self.html_move();
        }
    }

    fn html_move(&mut self) {
        if let Some((id, page, ..)) = self.html_object_pressed {
            let Some((o, d, _)) = self.cursor_ray_now() else { return };
            let Some(w) = self.world.clone() else { return };
            if let Some(h) = w.html_object_hit(o, d, HTML_OBJECT_REACH).filter(|h| h.map_id == id && h.page == page) {
                w.html_object_pointer(id, page, h.u, h.v, omsi_sim::htmltex::PointerKind::Move);
                self.html_object_pressed = Some((id, page, h.u, h.v));
            }
            return;
        }
        let Some((page, ..)) = self.html_pressed else { return };
        let Some((o, d, _)) = self.cursor_ray_now() else { return };
        let Some(p) = self.player.as_mut() else { return };
        if let Some((pg, u, v)) = p.html_hit(o, d).filter(|h| h.0 == page) {
            p.html_pointer(pg, u, v, omsi_sim::htmltex::PointerKind::Move);
            self.html_pressed = Some((pg, u, v));
        }
    }

    #[cfg(windows)]
    pub(crate) fn reset_vr_pointer(&mut self) {
        if let Some(vr) = self.vr.as_mut() {
            vr.recenter_pointer();
        }
        self.vr_cursor_physical = None;
        self.vr_cursor_warp_pending = None;
    }

    #[cfg(windows)]
    pub(crate) fn on_vr_cursor_moved(&mut self, x: f32, y: f32) {
        if let Some(target) = self.vr_cursor_warp_pending.take() {
            // CursorMoved from set_cursor_position is not hand movement.
            self.vr_cursor_physical = Some((x, y));
            if (x - target.0).abs() < 3.0 && (y - target.1).abs() < 3.0 {
                return;
            }
            // A real move arrived first; use the next event as the new baseline.
            return;
        }
        if let Some(previous) = self.vr_cursor_physical {
            self.cursor.0 += x - previous.0;
            self.cursor.1 += y - previous.1;
        }
        self.vr_cursor_physical = Some((x, y));
        self.html_move();
        let Some((width, height)) = self.surface.as_ref().map(|s|
            (s.config.width as f32, s.config.height as f32)) else { return };
        if self.window_focused && !self.mouse_look
            && (x < 12.0 || x > width - 12.0 || y < 12.0 || y > height - 12.0) {
            let center = (width * 0.5, height * 0.5);
            if self.window.as_ref().is_some_and(|window| window.set_cursor_position(
                winit::dpi::PhysicalPosition::new(center.0 as f64, center.1 as f64)).is_ok()) {
                self.vr_cursor_physical = Some(center);
                self.vr_cursor_warp_pending = Some(center);
            }
        }
    }

    #[cfg(windows)]
    pub(crate) fn poll_vr_cursor_position(&mut self) {
        if self.vr_nav_edit.is_some() { return; }
        let cockpit = self.vr.is_some() && self.game_menu.is_none()
            && self.chooser.is_none() && !self.mouse_drive
            && matches!(self.view.as_str(), "driver" | "pax");
        if !cockpit {
            self.vr_cursor_physical = None;
            self.vr_cursor_warp_pending = None;
            return;
        }
        if !self.window_focused || self.mouse_look { return; }
        let Some(window) = self.window.as_ref() else { return };
        let Ok(client_origin) = window.inner_position() else { return };
        let mut point = windows::Win32::Foundation::POINT::default();
        if unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point) }.is_ok() {
            self.on_vr_cursor_moved((point.x - client_origin.x) as f32,
                                    (point.y - client_origin.y) as f32);
        }
    }

    /// Mouse steering beyond the window's edge: with the cursor pinned at the left or right
    /// edge, the mouse moving on outwards turns the wheel further (the whole width per full
    /// lock, as standing); moving back gives that back first, the cursor held at the edge
    /// until it is used up, so the wheel never jumps.
    pub(crate) fn mouse_past_edge(&mut self, dx: f32) {
        let Some(w) = self.surface.as_ref().map(|s| s.config.width as f32) else { return };
        let per_px = 2.0 / w.max(1.0);
        let (at_left, at_right) = (self.cursor.0 <= 2.0, self.cursor.0 >= w - 3.0);
        let before = self.mouse_edge;
        if (at_right && dx > 0.0) || (at_left && dx < 0.0) {
            self.mouse_edge = (self.mouse_edge + dx * per_px).clamp(-2.0, 2.0);
        } else if (self.mouse_edge > 0.0 && dx < 0.0) || (self.mouse_edge < 0.0 && dx > 0.0) {
            let m = self.mouse_edge + dx * per_px;
            self.mouse_edge = if m.signum() != before.signum() { 0.0 } else { m };
            // (the cursor stays where it was: the move went into the wheel)
            if let Some(win) = self.window.as_ref() {
                let x = if before > 0.0 { w - 2.0 } else { 1.0 };
                let _ = win.set_cursor_position(winit::dpi::PhysicalPosition::new(x as f64, self.cursor.1 as f64));
                self.cursor.0 = x;
            }
        }
    }

    /// Take the cursor's new place; false when the move was someone else's (the object
    /// editor's drag, the city map) and no switch is to be named.
    fn move_cursor(&mut self, x: f32, y: f32) -> bool {
        let last = self.cursor;
        self.cursor = (x, y);
        if let Some((y0, v0)) = self.both_drag {
            // (0x82c5f8: outside, the distance at the press times 1 + the way up over 500
            // pixels; in the bus the field of view at the press plus the way up over 500
            // pixels times the camera's own, which is also its widest (+0x31c, 0x7edde4):
            // moving up widens the view as it backs the outside camera away)
            if self.view == "outside" {
                let k = (1.0 + (y0 - y) / 500.0).max(0.05);
                self.orbit = (v0 * k).clamp(ORBIT_MIN, ORBIT_MAX);
            } else {
                self.view_zoom.insert(self.view.clone(), (v0 + (y0 - y) / 500.0).clamp(0.2, 1.0_f32.max(v0)));
            }
            return false;
        }
        if self.menu_scroll_drag {
            let Some(ui) = self.ui.as_ref() else {
                self.menu_scroll_drag = false;
                return true;
            };

            if let (Some(track), Some(thumb)) =
                (ui.menu_scroll_track, ui.menu_scroll_thumb)
            {
                let track_h = (track[3] - track[1]).max(1.0);
                let thumb_h = (thumb[3] - thumb[1]).max(1.0);
                let travel = (track_h - thumb_h).max(1.0);

                let max_top =
                    (self.menu_len() as f32 - ui.menu_rows as f32).max(0.0);

                if max_top > 0.0 {
                    let delta = (y - last.1) / travel * max_top;

                    self.menu_top = Some(
                        (self.menu_top.unwrap_or(ui.menu_start as f32) + delta)
                            .clamp(0.0, max_top),
                    );
                }
            }

            return false;
        }
        // an object dragged in the object editor follows
        if self.editor_drag {
            self.editor_drag_frame();
            return false;
        }
        // while the city map is open the mouse is the map's
        if let Some(n) = self.navigator.as_mut().filter(|n| n.map_open()) {
            n.map_move(x, y);
            return false;
        }
        // looking round in a view of the bus follows the cursor, as Omsi.exe turns it
        // (0x82c5f8: yaw and pitch at the press plus the cursor's way times fov / 78.75):
        // raw device deltas are no window pixels (a tablet, a remote desktop or a VM
        // reports positions there and spun the view) and did not follow the zoom
        // The right button held is precision zoom (vertical cursor travel),
        // never a look — except while a both-drag owns the gesture above.
        // The middle button looks round.
        let rmb_zoom = self.buttons_held.1
            && !self.mmb_held
            && self.both_drag.is_none()
            && matches!(self.view.as_str(), "driver" | "outside" | "pax");
        if rmb_zoom {
            let scale = self.window.as_ref().map(|w| w.scale_factor() as f32).unwrap_or(1.0).max(0.1);
            // zooming takes over from an eased Space return.
            self.f1_reset = None;
            let intent = if self.view == "driver" { ZOOM_INTENT_F1 } else { ZOOM_INTENT };
            let m = self.view_zoom.get(&self.view).copied().unwrap_or(1.0);
            self.view_zoom.insert(
                self.view.clone(),
                precision_zoom_step(m, (y - last.1) / scale, intent),
            );
        } else if self.cursor_looks() {
            let scale = self.window.as_ref().map(|w| w.scale_factor() as f32).unwrap_or(1.0).max(0.1);
            let fov = self.camera.as_ref().map(|c| c.fov_deg).unwrap_or(60.0);
            let k = look_deg_per_px(fov);
            self.look_by((x - last.0) / scale * k, (y - last.1) / scale * k);
        }
        // Dragging a switch reads the movement in screen pixels - take it from the
        // cursor itself rather than from the raw device delta, which is not in the
        // window's pixels (and on this Mac is not always delivered at all): that is
        // why the parking brake could not be pulled with the mouse. The movement is
        // collected here and handed to the script once a frame (`App::drag_frame`).
        if self.dragging {
            let scale = self
                .window
                .as_ref()
                .map(|w| w.scale_factor() as f32)
                .unwrap_or(1.0)
                .max(0.1);
            self.drag_delta.0 += (self.cursor.0 - last.0) / scale;
            self.drag_delta.1 += (self.cursor.1 - last.1) / scale;
        }
        true
    }

    pub(crate) fn on_left(&mut self, pressed: bool) {
        if self.vr_nav_edit.is_some() { return; }
        // the object editor: the mouse picks and drags
        if self.game_menu.is_none() && self.editor_mouse(pressed) {
            return;
        }
        // the city map: a click on the navigator opens it; while it is open the mouse is
        // the map's (a click outside closes it)
        let (x, y) = self.cursor;
        let vr_active = self.vr_active();
        if let Some(n) = self.navigator.as_mut() {
            if n.map_open() {
                let ctrl = self.keys.contains(&KeyCode::ControlLeft) || self.keys.contains(&KeyCode::ControlRight);
                if pressed && (ctrl || self.teleport_pick) {
                    // Ctrl+click (or a click after Esc → Move the bus): the bus to the street
                    // nearest that point, as OMSI's map window places vehicles
                    if let Some(at) = n.map_point(x, y) {
                        if std::mem::take(&mut self.teleport_pick) {
                            n.toggle_map();
                        }
                        self.place_bus_at(at);
                    }
                } else if pressed {
                    n.map_press(x, y);
                } else {
                    n.map_release();
                }
                return;
            }
            if pressed && !vr_active && n.over_panel(x, y) {
                n.toggle_map();
                return;
            }
        }
        // a click on the chat opens its input box (and is the chat's, not the cockpit's)
        if pressed && self.lan.is_some() && self.settings.chat {
            if self.ui.as_ref().map(|u| u.chat.hovered).unwrap_or(false) {
                self.remotes.chat.open();
                return;
            }
            // a click anywhere else leaves the line and goes on to the game
            self.remotes.chat.blur();
        }
        // in another player's bus: a passenger, whose clicks work nothing of it (they
        // went to the driver's game, which worked its switches for them)
        if self.view == "foot" && self.inside_remote.is_some() {
            return;
        }
        // a page (`[htmltexture]`) on a scenery object: pressed and released like the bus's own
        if self.html_object_click(pressed) {
            return;
        }
        // on foot: the own bus's switches, doors and flaps from inside it or standing by it
        if self.view == "foot" && !self.foot_reaches_bus() {
            return;
        }
        #[cfg(windows)]
        if self.vr.is_some() && self.mouse_drive && self.game_menu.is_none()
            && matches!(self.view.as_str(), "driver" | "pax") {
            if !pressed {
                if let Some(player) = self.player.as_mut() { player.release(); }
                self.dragging = false;
            }
            return;
        }
        let ray = self.camera.as_ref().zip(self.surface.as_ref())
            .map(|(cam, s)| self.cockpit_cursor_ray(cam, (s.config.width, s.config.height)));
        if let (Some(p), Some((o, d, spread))) = (
            self.player.as_mut(),
            ray,
        ) {
            self.drag_delta = (0.0, 0.0);
            if pressed {
                if let Some((page, u, v)) = p.html_hit(o, d) {
                    p.release();
                    p.html_pointer(page, u, v, omsi_sim::htmltex::PointerKind::Down);
                    self.html_pressed = Some((page, u, v));
                    self.dragging = false;
                    return;
                }
                self.dragging = p
                    .click(o, d, spread)
                    .is_some();
            } else {
                if let Some((page, u, v)) = self.html_pressed.take() {
                    let (u, v) = p.html_hit(o, d).filter(|h| h.0 == page).map_or((u, v), |h| (h.1, h.2));
                    p.html_pointer(page, u, v, omsi_sim::htmltex::PointerKind::Up);
                    self.dragging = false;
                    return;
                }
                p.release();
                self.dragging = false;
            }
        }
    }

    /// A click on a page of a scenery object (`[htmltexture]` in its model). True when the
    /// click was the page's: the press lands on it, the release goes to it wherever the
    /// pointer is by then.
    fn html_object_click(&mut self, pressed: bool) -> bool {
        let Some(w) = self.world.clone() else { return false };
        if !pressed {
            let Some((id, page, u, v)) = self.html_object_pressed.take() else { return false };
            let (u, v) = self
                .cursor_ray_now()
                .and_then(|(o, d, _)| w.html_object_hit(o, d, HTML_OBJECT_REACH))
                .filter(|h| h.map_id == id && h.page == page)
                .map_or((u, v), |h| (h.u, h.v));
            w.html_object_pointer(id, page, u, v, omsi_sim::htmltex::PointerKind::Up);
            self.dragging = false;
            return true;
        }
        // (driving with the VR pointer: the clicks are the bus's)
        #[cfg(windows)]
        if self.vr.is_some() && self.mouse_drive && self.game_menu.is_none() && matches!(self.view.as_str(), "driver" | "pax") {
            return false;
        }
        let Some((o, d, _)) = self.cursor_ray_now() else { return false };
        let Some(h) = w.html_object_hit(o, d, HTML_OBJECT_REACH) else { return false };
        // the bus in front of the page (a switch, a window, its own page) takes the click
        if self.player.as_ref().and_then(|p| p.body_hit(o, d)).is_some_and(|t| t < h.t) {
            return false;
        }
        if let Some(p) = self.player.as_mut() {
            p.release();
        }
        w.html_object_pointer(h.map_id, h.page, h.u, h.v, omsi_sim::htmltex::PointerKind::Down);
        self.html_object_pressed = Some((h.map_id, h.page, h.u, h.v));
        self.dragging = false;
        true
    }

    /// The ray under the cursor now (see [`Self::cockpit_cursor_ray`]).
    fn cursor_ray_now(&self) -> Option<(glam::DVec3, glam::Vec3, f32)> {
        let (cam, s) = self.camera.as_ref().zip(self.surface.as_ref())?;
        Some(self.cockpit_cursor_ray(cam, (s.config.width, s.config.height)))
    }

    /// A switch held with the mouse: OMSI runs its `<event>_drag` trigger every frame the
    /// button is down, with this frame's movement in `mouse_x` / `mouse_y` - 0 while the
    /// hand keeps still. The scripts rely on that: the EN92 cash desk takes its swing speed
    /// from the last two positions, and fired only on movement it kept the speed of the last
    /// small move through a pause and swung shut when let go; the door scripts set their
    /// push once per trigger.
    pub(crate) fn drag_frame(&mut self) {
        if !self.dragging {
            return;
        }
        let (dx, dy) = std::mem::take(&mut self.drag_delta);
        if let Some(p) = self.player.as_mut() {
            p.drag(dx, dy);
        }
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
        if self.input_script.is_empty() {
            return;
        }
        let t = self.started.elapsed().as_secs_f32();
        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0);
        while let Some((at, cmd)) = self.input_script.first().cloned() {
            if t < at {
                break;
            }
            self.input_script.remove(0);
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
                // `rawmouse dx`: the mouse moved by dx device units (past the window's edge
                // too, as mouse steering takes it)
                "rawmouse" => {
                    let (dx, _) = xy();
                    if self.mouse_drive && self.game_menu.is_none() {
                        self.mouse_past_edge(dx);
                    }
                }
                "drag" => {
                    let (dx, dy) = xy();
                    let (x, y) = self.cursor;
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
                "look" => self.look = xy(),
                // `orbit <m>`: how far the outside camera stands off, as the mouse wheel sets it
                "orbit" => self.orbit = xy().0.clamp(ORBIT_MIN, ORBIT_MAX),
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
                    if self.editor.is_some() && self.game_menu.is_none() {
                        self.editor_wheel(n);
                    } else if self.placing.is_some() && self.game_menu.is_none() {
                        self.placing_wheel(n);
                    } else if self.game_menu.is_some() {
                        self.menu_wheel(n);
                    } else {
                        self.wheel(n);
                    }
                    log::info!("input script: wheel {n}: menu line {:?}, chooser {:?}, placing heading {:?}", self.game_menu, self.chooser, self.placing.as_ref().map(|p| p.heading));
                }
                // `click`: a left click where the cursor is, through the window's own path
                "click" => {
                    if self.placing.is_some() && self.game_menu.is_none() {
                        self.placing_click();
                    } else if self.game_menu.is_some() {
                        // (on the menu as the window's button: its lines, its arrows)
                        self.left_button(event_loop, true);
                        self.left_button(event_loop, false);
                    } else {
                        self.on_left(true);
                        self.on_left(false);
                    }
                    log::info!("input script: click: placing {:?}, placed at {:?}", self.placing.as_ref().map(|p| (p.at, p.blocked)), self.placed.last().map(|q| (q.vehicle.position, q.vehicle.heading)));
                }
                // `both down|up`: both mouse buttons held (OMSI's mouse zoom) or let go
                "both" => {
                    if arg == "down" {
                        self.buttons_held = (true, true);
                        let started = self.start_both_drag();
                        log::info!("input script: both buttons: zoom drag {started}");
                    } else {
                        self.buttons_held = (false, false);
                        self.both_drag = None;
                        log::info!("input script: both buttons up: zoom {:?}, orbit {:.1}", self.view_zoom.get(&self.view), self.orbit);
                    }
                }
                // `right down|up`: the right mouse button, through the window's own path
                "right" => {
                    self.on_right(arg == "down");
                    log::info!("input script: right button {arg}: zoom drag {}, look {}, zoom {:?}, orbit {:.1}", self.both_drag.is_some(), self.mouse_look, self.view_zoom.get(&self.view), self.orbit);
                }
                "press" => self.on_left(true),
                "release" => self.on_left(false),
                // `type <text>`: characters into the open LAN chat line (after `key V`)
                "type" => {
                    let text = cmd.split_once(' ').map(|x| x.1).unwrap_or("");
                    if lan::chat_open(&self.remotes) {
                        lan::chat_type(&mut self.remotes, text);
                    } else {
                        log::warn!("input script: the chat line is not open");
                    }
                }
                // `turn dx,dy`: turn the view by degrees, as a right-button drag does
                "turn" => {
                    let (dx, dy) = xy();
                    self.look_by(dx, dy);
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
                        log::info!("input script: view {} camera in the bus ({:.2}, {:.2}, {:.2}), on foot {:?}", self.view, d.truncate().dot(right), d.truncate().dot(fwd), d.z, self.on_foot.as_ref().map(|f| f.pos));
                    }
                }
                // `log mouse`: the mouse steering's state
                "log" if arg == "mouse" => {
                    log::info!(
                        "input script: mouse steering {} look {} menu {:?} paused {} focused {} steer {:.3}",
                        self.mouse_drive,
                        self.mouse_look,
                        self.game_menu,
                        self.paused,
                        self.window_focused,
                        self.mouse_steer.0
                    );
                }
                "log" => {
                    let v = self
                        .player
                        .as_ref()
                        .map(|p| (p.vehicle.var(arg), p.vehicle.str_var(arg)));
                    let names = describe::names(&self.args.root, &self.settings.language);
                    let shown = self
                        .hover
                        .as_deref()
                        .map(|h| names.control(h))
                        .or_else(|| self.hover_part.as_deref().map(|p| names.part(p)));
                    log::info!(
                        "input script: {arg} = {:?}  hover {:?} / {:?} shown as {:?}",
                        v,
                        self.hover,
                        self.hover_part,
                        shown
                    );
                }
                // `menu <what>`: a line of the game menu by its id (`menu remove`, `menu switch`),
                // `menu pick:<n>`: line n of the list open
                "menu" => {
                    if let Some(n) = arg.strip_prefix("pick:").and_then(|n| n.parse::<usize>().ok()) {
                        self.chooser_pick(n);
                    } else {
                        if self.game_menu.is_none() {
                            self.open_game_menu();
                        }
                        // (a line under "More..." is found there)
                        if !self.game_menu_items().iter().any(|m| m.0 == arg) {
                            self.menu_more = !self.menu_more;
                        }
                        match self.game_menu_items().iter().position(|m| m.0 == arg) {
                            Some(k) => self.menu_choose(event_loop, k),
                            None => log::warn!("input script: no menu line {arg}"),
                        }
                    }
                    let riders = self.humans.as_ref().map(|h| (h.people_in(crate::humans::BusId::Player), self.placed.iter().map(|q| h.people_in(crate::humans::BusId::Ai(crate::humans::placed_bus_id(q.uid)))).collect::<Vec<_>>()));
                    log::info!("input script: menu {arg}: player {:?}, on foot {:?}, placed {}, people in the bus / the placed ones {:?}", self.player.as_ref().map(|p| p.vehicle.position), self.on_foot.as_ref().map(|f| f.pos), self.placed.len(), riders);
                }
                // `shot <file>`: the window's own view into a PNG, drawn from the scene the
                // window is showing (the only way to see what the window path renders)
                "shot" => self.shot = Some(PathBuf::from(arg)),
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

    /// What the cursor points at, for the HUD. Recomputed every frame: the head turns and
    /// the bus moves under a cursor that is standing still.
    /// Open the game menu: the simulation pauses (not in a LAN session, which runs on
    /// for the other players).
    pub(crate) fn open_game_menu(&mut self) {
        self.menu_prev_pause = self.paused;
        if self.lan.is_none() {
            self.paused = true;
        }
        self.game_menu = Some(0);
        self.menu_top = None;
        self.menu_more = false;
    }

    pub(crate) fn close_game_menu(&mut self) {
        self.game_menu = None;
        self.menu_top = None;
        self.paused = self.menu_prev_pause;
    }

    /// Show one of the menu's lists in the chooser (see `game_lists`).
    pub(crate) fn open_list(&mut self, kind: crate::game_lists::ListKind) {
        self.admin_list = Some(crate::game_lists::items(self, &kind));
        self.list_kind = Some(kind);
        // (on its first line, not on a heading)
        self.chooser = Some(if self.is_heading(0) { self.chooser_next(0, 1) } else { 0 });
    }

    /// Line `k` of the list shown heads the lines under it (`game_lists::HEADING`).
    fn is_heading(&self, k: usize) -> bool {
        self.admin_list.as_ref().and_then(|l| l.get(k)).is_some_and(|l| l.1 == crate::game_lists::HEADING)
    }

    /// The line `step` lines on from `sel` (round the list; `n - 1` is one back), over the
    /// headings.
    fn chooser_next(&self, sel: usize, step: usize) -> usize {
        let n = self.admin_list.as_ref().unwrap_or(&self.vehicle_list).len().max(1);
        let mut k = sel;
        for _ in 0..n {
            k = (k + step) % n;
            if !self.is_heading(k) {
                break;
            }
        }
        k
    }

    /// Left or Right on line `k` of a list, or a click on the arrows round its value: its
    /// setting one step down (`-`) or up (`+`), see `game_lists::ADJUST`; other lines stay.
    pub(crate) fn chooser_adjust(&mut self, k: usize, dir: &str) {
        let Some(action) = self.admin_list.as_ref().and_then(|l| l.get(k)).and_then(|l| l.1.strip_suffix(crate::game_lists::ADJUST)).map(|a| format!("{a} {dir}")) else { return };
        // (run as a pick of the line, with the step in place of the mark)
        if let Some(l) = self.admin_list.as_mut().and_then(|l| l.get_mut(k)) {
            l.1 = action;
        }
        self.chooser_pick(k);
    }

    /// A key while the vehicle chooser is open.
    fn chooser_key(&mut self, code: KeyCode) {
        let n = self.admin_list.as_ref().unwrap_or(&self.vehicle_list).len().max(1);
        let sel = self.chooser.unwrap_or(0);
        self.menu_top = None;
        match code {
            KeyCode::Escape => {
                self.chooser = None;
                self.admin_list = None;
                self.list_kind = None;
            }
            KeyCode::ArrowUp | KeyCode::KeyW => self.chooser = Some(self.chooser_next(sel, n - 1)),
            KeyCode::ArrowDown | KeyCode::KeyS => self.chooser = Some(self.chooser_next(sel, 1)),
            KeyCode::ArrowLeft | KeyCode::KeyA => self.chooser_adjust(sel, "-"),
            KeyCode::ArrowRight | KeyCode::KeyD => self.chooser_adjust(sel, "+"),
            // (off a heading onto the line under it)
            KeyCode::PageUp => self.chooser = Some(sel.saturating_sub(15)).map(|k| if self.is_heading(k) { self.chooser_next(k, 1) } else { k }),
            KeyCode::PageDown => self.chooser = Some((sel + 15).min(n - 1)).map(|k| if self.is_heading(k) { self.chooser_next(k, 1) } else { k }),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.chooser_pick(sel),
            _ => {}
        }
    }

    /// Place the chosen vehicle: in front of the camera in a free or map view, else beside
    /// the vehicle driven (OMSI puts a new vehicle where the map view points).
    pub(crate) fn chooser_pick(&mut self, k: usize) {
        // (a heading is no choice)
        if self.is_heading(k) {
            return;
        }
        self.chooser = None;
        // a list of the menu's (the administration, the options …): done, and the list
        // shown again - or the next one (a line's tours), or back to the menu
        if let Some(list) = self.admin_list.take() {
            let kind = self.list_kind.take().unwrap_or(crate::game_lists::ListKind::Admin);
            let Some((_, action)) = list.get(k).cloned() else { return };
            match crate::game_lists::run(self, &kind, &action) {
                Some(next) => {
                    let keep = next == kind;
                    self.open_list(next);
                    if keep {
                        self.chooser = Some(k.min(self.admin_list.as_ref().map(|l| l.len().saturating_sub(1)).unwrap_or(0)));
                    }
                }
                None if action != "back" && matches!(kind, crate::game_lists::ListKind::Tours(_) | crate::game_lists::ListKind::Numbers | crate::game_lists::ListKind::Destinations | crate::game_lists::ListKind::RouteNumbers | crate::game_lists::ListKind::Hofs) => self.close_game_menu(),
                None => {}
            }
            return;
        }
        // a vehicle of the list: its livery and depot file are asked for first
        let Some((_, bus)) = self.vehicle_list.get(k).cloned() else { return };
        self.open_list(crate::game_lists::ListKind::PlaceLivery(bus));
    }

    /// Put the vehicle file `bus` down beside the camera or the bus driven, in `paint` (a
    /// scheme's name; None: at random) with the depot file `hof` (None: the map's).
    pub(crate) fn place_vehicle(&mut self, bus: &str, paint: Option<String>, hof: Option<String>) {
        let name = self.vehicle_list.iter().find(|v| v.1 == bus).map(|v| v.0.clone()).unwrap_or_else(|| bus.to_string());
        let bus = bus.to_string();
        let (Some(w), Some(r), Some(scene), Some(cam)) = (self.world.clone(), self.renderer.as_ref(), self.scene.as_mut(), self.camera.as_ref()) else { return };
        let (x, y, heading) = match (self.view.as_str(), self.player.as_ref()) {
            ("free", _) | (_, None) => {
                let f = cam.forward();
                let flat = glam::DVec2::new(f.x as f64, f.y as f64).normalize_or_zero();
                let at = cam.position.truncate() + flat * 15.0;
                (at.x, at.y, cam.yaw as f64)
            }
            (_, Some(p)) => {
                let h = p.vehicle.heading.to_radians();
                let right = glam::DVec2::new(h.cos(), -h.sin());
                let at = p.vehicle.position.truncate() + right * 5.0;
                (at.x, at.y, p.vehicle.heading)
            }
        };
        let one = Args {
            bus: Some(bus.clone()),
            spawn: Some(format!("{x},{y},{heading}")),
            situation_vars: Vec::new(),
            situation_strvars: Vec::new(),
            situation_others: Vec::new(),
            line: None,
            tour: None,
            trip: None,
            autostart: false,
            paint,
            hof: hof.or(self.args.hof.clone()),
            ..self.args.clone()
        };
        match spawn_player(&one, &w, r, scene) {
            Ok(Some(q)) => {
                log::info!("placed {bus} at ({x:.1}, {y:.1})");
                let uid = q.uid;
                self.placed.push(q);
                // (then put down with the mouse, where the player wants it)
                self.begin_placing(uid, heading);
                let _ = name;
            }
            Ok(None) => {}
            Err(e) => self.service_msg = Some((format!("Could not place {name}: {e:#}"), 5.0)),
        }
    }

    /// Couple a standing vehicle to the back of the one driven: its front coupling within
    /// 2.5 m of the train's rear coupling, facing the same way.
    pub(crate) fn couple(&mut self) {
        let (Some(w), Some(r), Some(scene), Some(p)) = (self.world.clone(), self.renderer.as_ref(), self.scene.as_mut(), self.player.as_mut()) else { return };
        // the train's rear coupling in the world
        let rear = match p.vehicle.trailers.last() {
            Some(t) => {
                let c = if t.reversed { t.ty.def.coupling_front.as_ref() } else { t.ty.def.coupling_back.as_ref() };
                c.map(|c| (t.world_transform().transform_point3(glam::Vec3::from(c.pos)), t.heading))
            }
            None => p.vehicle.ty.def.coupling_back.as_ref().map(|c| (p.vehicle.world_transform().transform_point3(glam::Vec3::from(c.pos)), p.vehicle.heading)),
        };
        let Some((rear, rear_heading)) = rear else {
            self.service_msg = Some(("This vehicle has no coupling at its back".into(), 4.0));
            return;
        };
        let found = self.placed.iter().position(|q| {
            let Some(c) = q.vehicle.ty.def.coupling_front.as_ref() else { return false };
            let front = q.vehicle.world_transform().transform_point3(glam::Vec3::from(c.pos));
            let dh = ((q.vehicle.heading - rear_heading + 540.0).rem_euclid(360.0) - 180.0).abs();
            (front - rear).truncate().length() < 2.5 && dh < 35.0
        });
        let Some(k) = found else {
            self.service_msg = Some(("Nothing to couple: back up to a trailer's coupling (within 2.5 m, in line)".into(), 4.0));
            return;
        };
        let q = self.placed.remove(k);
        let ty = q.vehicle.ty.clone();
        if let (Some(a), Some(mut ss)) = (self.audio.as_ref(), q.sounds) {
            ss.stop_all(a);
        }
        w.release_vehicle(r, scene, q.render);
        for tr in q.trailer_renders {
            w.release_vehicle(r, scene, tr);
        }
        p.vehicle.attach_trailer_ex(ty.clone(), false);
        p.trailer_renders.push(w.add_vehicle_part(r, scene, &ty, None, &p.render));
        p.hand_coupled += 1;
        self.service_msg = Some((format!("Coupled: {} {}", ty.def.manufacturer, ty.def.type_name), 3.0));
    }

    /// Uncouple the last part coupled by hand: it stays where it is, a vehicle of its own.
    pub(crate) fn uncouple(&mut self) {
        let (Some(w), Some(r), Some(scene)) = (self.world.clone(), self.renderer.as_ref(), self.scene.as_mut()) else { return };
        let Some(p) = self.player.as_mut() else { return };
        if p.hand_coupled == 0 {
            self.service_msg = Some(("Nothing coupled by hand (an articulated bus's rear section stays)".into(), 4.0));
            return;
        }
        let Some(t) = p.vehicle.detach_last_trailer() else { return };
        if let Some(tr) = p.trailer_renders.pop() {
            w.release_vehicle(r, scene, tr);
        }
        p.hand_coupled -= 1;
        let one = Args {
            bus: Some(t.ty.def.path.to_string_lossy().to_string()),
            spawn: Some(format!("{},{},{},{}", t.position.x, t.position.y, t.heading, t.position.z)),
            situation_vars: Vec::new(),
            situation_strvars: Vec::new(),
            situation_others: Vec::new(),
            line: None,
            tour: None,
            trip: None,
            autostart: false,
            paint: None,
            ..self.args.clone()
        };
        match spawn_player(&one, &w, r, scene) {
            Ok(Some(q)) => {
                self.placed.push(q);
                self.service_msg = Some(("Uncoupled".into(), 3.0));
            }
            Ok(None) => {}
            Err(e) => log::warn!("uncoupled part: {e:#}"),
        }
    }

    /// A key while the game menu is open.
    /// The object editor on or off; on, it starts with the free camera where the view is.
    pub(crate) fn toggle_editor(&mut self) {
        // (in a LAN session the host edits the map for everybody: its edits go to the
        // others' games, a client's would stay its own)
        if self.editor.is_none() && self.lan.as_ref().map(|l| l.role == omsi_net::Role::Client).unwrap_or(false) {
            self.service_msg = Some(("In a LAN session only the host edits the map".into(), 3.0));
            return;
        }
        if self.editor.take().is_some() {
            self.editor_drag = false;
            self.service_msg = Some(("Object editor off (unsaved changes stay until the end of the session)".into(), 3.0));
            return;
        }
        let ed = crate::editor::Editor::default();
        let msg = self.world.as_ref().map(|w| ed.describe(w)).unwrap_or_default();
        self.editor = Some(ed);
        self.service_msg = Some((format!("{msg} - click picks, drag moves, wheel turns (Shift: height), Delete, C copy, V variant, Backspace undo, PgUp/PgDn/F ground, [ ] brush, Ctrl+S save, Esc leave"), 10.0));
    }

    /// A key while the object editor is on; true when it was the editor's.
    pub(crate) fn editor_key(&mut self, code: KeyCode) -> bool {
        let shift = self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight);
        let ctrl = self.keys.contains(&KeyCode::ControlLeft) || self.keys.contains(&KeyCode::ControlRight);
        let Some(cam) = self.camera.as_ref() else { return false };
        let (eye, fwd, yaw) = (cam.position, cam.forward(), cam.yaw as f64);
        let Some(action) = crate::editor::action_for(code, shift, ctrl, yaw) else { return false };
        let Some(world) = self.world.clone() else { return false };
        let msg = match action {
            crate::editor::Action::Leave => {
                self.toggle_editor();
                return true;
            }
            crate::editor::Action::Pick => {
                let ed = self.editor.as_mut().unwrap();
                ed.pick(&world, eye, fwd);
                ed.describe(&world)
            }
            crate::editor::Action::NextPick => {
                let ed = self.editor.as_mut().unwrap();
                ed.next_pick();
                ed.describe(&world)
            }
            crate::editor::Action::Save => {
                let content = crate::startup::content_dir();
                let ed = self.editor.as_ref().unwrap();
                match content.map(|c| ed.save(&world, &self.args.map, &c, &self.args.root)) {
                    Some(Ok(files)) if files.is_empty() => "Nothing to save".to_string(),
                    Some(Ok(files)) => format!("Saved {} tile(s) to the content folder ({})", files.len(), files.iter().filter_map(|f| f.file_name()).map(|n| n.to_string_lossy()).collect::<Vec<_>>().join(", ")),
                    Some(Err(e)) => format!("Not saved: {e}"),
                    None => "Not saved: no content folder".to_string(),
                }
            }
            a @ (crate::editor::Action::Ground(_) | crate::editor::Action::Flatten | crate::editor::Action::Brush(_)) => {
                let at = crate::editor::Editor::aim(&world, eye, fwd);
                let (msg, tiles) = self.editor.as_mut().unwrap().ground(&world, at, &a);
                if !tiles.is_empty() {
                    log::info!("map editor: ground of tiles {tiles:?} at {at:?}");
                    world.forget_staged(&tiles);
                    if let (Some(st), Some(r), Some(scene)) = (self.streamer.as_mut(), self.renderer.as_ref(), self.scene.as_mut()) {
                        st.reload(r, scene, Some(&tiles), self.audio.as_ref());
                    }
                }
                msg
            }
            a => {
                let (Some(r), Some(scene)) = (self.renderer.as_ref(), self.scene.as_mut()) else { return true };
                match self.editor.as_mut().unwrap().apply(&world, r, scene, &a) {
                    Some(m) => m,
                    None => "Pick an object first (Enter)".to_string(),
                }
            }
        };
        log::info!("object editor: {msg}");
        self.service_msg = Some((msg, 5.0));
        self.editor_broadcast(false);
        true
    }

    /// The host's edits to the other players' games (`all`: every edit of the session,
    /// sent again every ten seconds for the ones who joined since).
    pub(crate) fn editor_broadcast(&mut self, all: bool) {
        let (Some(ed), Some(w)) = (self.editor.as_ref(), self.world.as_ref()) else {
            if all {
                // (edits stay after the editor is left: sent from the world's list)
                if let (Some(w), Some(l)) = (self.world.as_ref(), self.lan.as_mut()) {
                    if l.role == omsi_net::Role::Host {
                        let lines = crate::editor::Editor::default().sync_lines(w, &self.args.root, true);
                        let ids: Vec<u32> = l.peers().map(|p| p.pose.id).filter(|id| *id != l.my_id).collect();
                        for line in &lines {
                            for id in &ids {
                                l.command(*id, line);
                            }
                        }
                    }
                }
            }
            return;
        };
        let Some(l) = self.lan.as_mut() else { return };
        if l.role != omsi_net::Role::Host {
            return;
        }
        let lines = ed.sync_lines(w, &self.args.root, all);
        let ids: Vec<u32> = l.peers().map(|p| p.pose.id).filter(|id| *id != l.my_id).collect();
        for line in &lines {
            if line.len() > omsi_net::MAX_CHAT {
                log::warn!("object editor: '{line}' is too long to send");
                continue;
            }
            for id in &ids {
                l.command(*id, line);
            }
        }
    }

    /// The mouse in the object editor: a click picks what is under the cursor (and starts
    /// dragging it), a drag moves it over the ground; true when the editor took it.
    pub(crate) fn editor_mouse(&mut self, pressed: bool) -> bool {
        if self.editor.is_none() {
            return false;
        }
        if !pressed {
            if self.editor_drag {
                self.editor_drag = false;
                self.editor_broadcast(false);
            }
            return true;
        }
        let (Some(cam), Some(s), Some(world)) = (self.camera.as_ref(), self.surface.as_ref(), self.world.clone()) else { return true };
        let (o, d) = cursor_ray(cam, self.cursor.0, self.cursor.1, s.config.width as f32, s.config.height as f32);
        let ed = self.editor.as_mut().unwrap();
        // (the copy being edited stays the one dragged while it is under the cursor)
        let on_added = ed.editing_added.and_then(|k| ed.added.get(k)).map(|a| {
            let p = a.base + a.moved - o;
            let along = p.dot(d.as_dvec3());
            along > 0.0 && (p - d.as_dvec3() * along).length() < 2.5
        }).unwrap_or(false);
        if !on_added {
            ed.pick(&world, o, d);
        }
        self.editor_drag = on_added || ed.selected.is_some();
        let msg = ed.describe(&world);
        self.service_msg = Some((msg, 5.0));
        true
    }

    /// The cursor moved while an object is dragged.
    pub(crate) fn editor_drag_frame(&mut self) {
        if !self.editor_drag {
            return;
        }
        let (Some(cam), Some(s), Some(world)) = (self.camera.as_ref(), self.surface.as_ref(), self.world.clone()) else { return };
        let (o, d) = cursor_ray(cam, self.cursor.0, self.cursor.1, s.config.width as f32, s.config.height as f32);
        let Some(hit) = crate::placing::ground_hit(&world, o, d.as_dvec3(), 400.0) else { return };
        let (Some(r), Some(scene), Some(ed)) = (self.renderer.as_ref(), self.scene.as_mut(), self.editor.as_mut()) else { return };
        if let Some(m) = ed.drag_to(&world, r, scene, hit) {
            self.service_msg = Some((m, 3.0));
        }
    }

    /// The wheel in the object editor: the object turns (5° a notch), with Shift it rises.
    pub(crate) fn editor_wheel(&mut self, amount: f32) -> bool {
        if self.editor.is_none() {
            return false;
        }
        let shift = self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight);
        let action = if shift { crate::editor::Action::Move(glam::DVec3::Z * 0.1 * amount as f64) } else { crate::editor::Action::Turn(5.0 * amount as f64) };
        let (Some(world), Some(r), Some(scene)) = (self.world.clone(), self.renderer.as_ref(), self.scene.as_mut()) else { return true };
        if let Some(m) = self.editor.as_mut().unwrap().apply(&world, r, scene, &action) {
            self.service_msg = Some((m, 3.0));
            self.editor_broadcast(false);
        }
        true
    }

    pub(crate) fn menu_key(&mut self, event_loop: &ActiveEventLoop, code: KeyCode) {
        if self.chooser.is_some() {
            self.chooser_key(code);
            return;
        }
        let n = self.game_menu_items().len();
        let sel = self.game_menu.unwrap_or(0);
        let modified = self.keys.iter().any(|key| {
            matches!(*key, KeyCode::ControlLeft | KeyCode::ControlRight | KeyCode::AltLeft | KeyCode::AltRight | KeyCode::ShiftLeft | KeyCode::ShiftRight)
        });
        self.menu_top = None;
        match code {
            // P changes only the simulation state, even while a menu is open.
            KeyCode::KeyP if !modified => self.toggle_pause(),
            // (from the full list back to the short one first)
            KeyCode::Escape if self.menu_more => {
                self.menu_more = false;
                self.game_menu = Some(0);
            }
            KeyCode::Escape => self.close_game_menu(),
            KeyCode::ArrowUp | KeyCode::KeyW => self.game_menu = Some((sel + n - 1) % n),
            KeyCode::ArrowDown | KeyCode::KeyS => self.game_menu = Some((sel + 1) % n),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.menu_choose(event_loop, sel),
            _ => {}
        }
    }

    /// The mouse wheel over the game menu: the chosen line moves (the menu scrolls with it),
    /// in a list the same; no wrapping round.
    pub(crate) fn menu_wheel(&mut self, amount: f32) {
        self.wheel_acc += amount;
        let steps = self.wheel_acc.trunc() as i64;
        if steps == 0 {
            return;
        }
        self.wheel_acc -= steps as f32;
        // the list scrolls under the mouse; what is chosen stays chosen (the wheel used to
        // walk the highlight up and down the lines)
        let n = self.menu_len() as f32;
        let (start, rows) = self.ui.as_ref().map(|u| (u.menu_start as f32, u.menu_rows as f32)).unwrap_or((0.0, n));
        let top = (self.menu_top.unwrap_or(start) - steps as f32).clamp(0.0, (n - rows).max(0.0));
        self.menu_top = Some(top);
    }

    /// How many lines the menu shows now (the chooser's list, else the game menu's).
    pub(crate) fn menu_len(&self) -> usize {
        match self.chooser {
            Some(_) => self.admin_list.as_ref().unwrap_or(&self.vehicle_list).len(),
            None => self.game_menu_items().len(),
        }
    }

    /// Do what line `k` of the game menu says.
    pub(crate) fn menu_choose(&mut self, event_loop: &ActiveEventLoop, k: usize) {
        self.menu_top = None;
        if self.chooser.is_some() {
            self.chooser_pick(k);
            return;
        }
        match self.game_menu_items().get(k).map(|m| m.0) {
            Some("more") => {
                self.menu_more = true;
                self.game_menu = Some(0);
            }
            Some("less") => {
                self.menu_more = false;
                self.game_menu = Some(0);
            }
            Some("place") => {
                if self.vehicle_list.is_empty() {
                    self.vehicle_list = crate::menu::Menu::new(&self.args.root, &self.args.map).vehicles;
                    crate::mt::protect(self.vehicle_list.iter().map(|v| v.0.as_str()));
                }
                if self.vehicle_list.is_empty() {
                    self.service_msg = Some(("No vehicles found".into(), 3.0));
                } else {
                    self.chooser = Some(0);
                }
            }
            Some("couple") => {
                self.close_game_menu();
                self.couple();
            }
            Some("tobus") => {
                self.close_game_menu();
                self.back_to_bus();
            }
            Some("remove") => {
                self.close_game_menu();
                self.remove_driven_vehicle();
            }
            Some("clearplaced") => {
                self.close_game_menu();
                self.remove_placed_vehicles();
            }
            Some("getout") => {
                self.close_game_menu();
                self.get_up();
            }
            Some("reset") => {
                self.close_game_menu();
                if let Some(p) = self.player.as_ref() {
                    let (at, heading) = (p.vehicle.position, p.vehicle.heading);
                    crate::admin::teleport(self, at, heading);
                    self.service_msg = Some(("The vehicle stands on its wheels again".into(), 3.0));
                }
            }
            Some("map") => {
                self.close_game_menu();
                if let Some(n) = self.navigator.as_mut() {
                    if !n.map_open() {
                        n.toggle_map();
                    }
                }
            }
            Some("admin") => self.open_list(crate::game_lists::ListKind::Admin),
            Some("options") => self.open_list(crate::game_lists::ListKind::Options),
            Some("duty") => self.open_list(crate::game_lists::ListKind::Lines),
            Some("driver") => self.open_list(crate::game_lists::ListKind::Drivers),
            Some("number") => self.open_list(crate::game_lists::ListKind::Numbers),
            Some("dest") => self.open_list(crate::game_lists::ListKind::Destinations),
            Some("hof") => self.open_list(crate::game_lists::ListKind::Hofs),
            Some("clock") => self.open_list(crate::game_lists::ListKind::Clock),
            Some("teleport") => {
                self.close_game_menu();
                if let Some(n) = self.navigator.as_mut() {
                    if !n.map_open() {
                        n.toggle_map();
                    }
                    self.teleport_pick = true;
                    self.service_msg = Some(("Click a street on the map: the bus is put there".into(), 6.0));
                }
            }
            Some("uncouple") => {
                self.close_game_menu();
                self.uncouple();
            }
            Some("resume") => self.close_game_menu(),
            Some("editor") => {
                self.close_game_menu();
                self.toggle_editor();
            }
            Some("save") => {
                self.quick_save();
                self.close_game_menu();
            }
            Some("shot") => {
                self.close_game_menu();
                self.take_screenshot();
            }
            Some("timetable") => {
                self.timetable = !self.timetable;
                self.close_game_menu();
            }
            Some("info") => {
                self.info_bar = !self.info_bar;
                self.close_game_menu();
            }
            Some(k @ ("refuel" | "wash" | "repair")) => {
                self.close_game_menu();
                self.run_service(k);
            }
            Some("weather") => {
                self.close_game_menu();
                self.next_weather();
            }
            Some("switch") => {
                self.close_game_menu();
                self.switch_vehicle();
            }
            Some(k @ ("later" | "earlier" | "later10" | "earlier10")) => {
                self.close_game_menu();
                if self.lan.as_ref().map(|l| l.role == omsi_net::Role::Client).unwrap_or(false) {
                    self.service_msg = Some(("In a LAN session the host sets the clock".into(), 3.0));
                } else {
                    self.shift_clock(match k {
                        "later" => 3600.0,
                        "earlier" => -3600.0,
                        "later10" => 600.0,
                        _ => -600.0,
                    });
                }
            }
            Some("load") => {
                self.game_menu = None;
                if self.load_quicksave() {
                    self.finish_session();
                    crate::platform::exit(event_loop);
                }
            }
            Some("quit") => {
                self.game_menu = None;
                self.finish_session();
                crate::platform::exit(event_loop);
            }
            _ => {}
        }
    }

    /// OMSI's weather dialog, the short way: the next weather of the Weather folder, in
    /// force at once (the roads keep their wetness until the rain changes it).
    /// How fast the clock runs: the session's in LAN play (the host's, which its time speed
    /// setting or its administration set), else the settings'.
    pub(crate) fn time_speed(&self) -> f64 {
        match self.lan.as_ref() {
            Some(l) => l.clock_speed,
            None => self.settings.time_speed.clamp(1.0, 30.0),
        }
    }

    pub(crate) fn next_weather(&mut self) {
        if self.lan.as_ref().map(|l| l.role == omsi_net::Role::Client).unwrap_or(false) {
            self.service_msg = Some(("In a LAN session the host sets the weather".into(), 3.0));
            return;
        }
        let mut files: Vec<String> = omsi_cfg::read_dir_merged("Weather")
            .into_iter()
            .filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("owt")).unwrap_or(false))
            .filter_map(|p| p.file_name().map(|n| format!("Weather/{}", n.to_string_lossy())))
            .collect();
        files.sort();
        files.dedup();
        if files.is_empty() {
            return;
        }
        let cur = self.args.weather.clone().unwrap_or_default().replace('\\', "/").to_ascii_lowercase();
        let i = files.iter().position(|f| f.to_ascii_lowercase() == cur).map(|i| (i + 1) % files.len()).unwrap_or(0);
        self.change_weather(Some(files[i].clone()), true, 1.0);
    }

    /// Go over to weather `file` (None: the map's default) in `secs` of the day (see
    /// `weather_cycle`); a host tells the others (`share`), who come over to it the same way.
    /// The player's own choice comes at once, as in Omsi.exe (the weather dialog loads the
    /// .owt and applies it straight away, 0x6828e0 -> 0x754c80); the cycle blends it in.
    pub(crate) fn change_weather(&mut self, file: Option<String>, share: bool, secs: f32) {
        let from = self.weather.clone().unwrap_or_default();
        self.args.weather = file.clone();
        let to = load_weather(&self.args);
        let name = to.name.clone();
        self.weather_blend = Some(crate::weather_cycle::Blend::new(from, to, secs));
        if share {
            // (a host: the others take it up with its next clock message)
            if let (Some(l), Some(f)) = (self.lan.as_mut(), file.as_ref()) {
                l.set_weather(f);
            }
        }
        log::info!("weather: going over to {file:?} ({name})");
        self.service_msg = Some((format!("Weather: {name}"), 4.0));
    }

    /// The weather this frame: a change coming in, and the cycle's next one (`secs` of the
    /// day went by; in LAN play only the host's cycle runs, the others follow it).
    pub(crate) fn tick_weather(&mut self, secs: f32) {
        if let Some(b) = self.weather_blend.as_mut() {
            let (w, clouds_changed, done) = b.step(secs);
            self.weather = Some(w);
            if done {
                self.weather_blend = None;
            }
            if clouds_changed {
                if let (Some(r), Some(scene)) = (self.renderer.as_ref(), self.scene.as_mut()) {
                    crate::weather_setup::setup_sky(&self.args, r, scene, self.envir.as_ref(), self.weather.as_ref());
                }
            }
        }
        if let Some(w) = self.weather.as_ref() {
            crate::weather_setup::cloud_drift_step(&mut self.cloud_drift, w, secs as f64);
        }
        let follows = self.lan.as_ref().is_some_and(|l| l.role == omsi_net::Role::Client);
        if follows || self.weather_blend.is_some() {
            return;
        }
        let Some(c) = self.weather_cycle.as_mut() else { return };
        c.next_in -= secs as f64;
        if c.next_in > 0.0 {
            return;
        }
        c.next_in = c.interval();
        let r = c.rand();
        let all = crate::weather_cycle::installed();
        let now = self.weather.clone().unwrap_or_default();
        let now_file = self.args.weather.clone().unwrap_or_default();
        if let Some(next) = crate::weather_cycle::pick(&all, &now, &now_file, self.clock.day_month().1, r) {
            self.change_weather(Some(next), true, 240.0);
        }
    }

    /// Drive another of the vehicles standing in the world (a situation's): the one driven
    /// now stays where it is with everything as it was, and its sounds go to the next one.
    pub(crate) fn switch_vehicle(&mut self) {
        if self.placed.is_empty() {
            self.service_msg = Some(("There is no other vehicle to drive".into(), 3.0));
            return;
        }
        let Some(mut now) = self.player.take() else {
            // on foot without a bus: the first placed one's wheel
            self.take_placed(0);
            return;
        };
        if let (Some(a), Some(mut ss)) = (self.audio.as_ref(), now.sounds.take()) {
            ss.stop_all(a);
        }
        let mut next = self.placed.remove(0);
        if let Some(a) = self.audio.as_ref() {
            next.load_sounds(a);
        }
        // the riders stay in the bus left; the people know the new one's cabin
        if let Some(h) = self.humans.as_mut() {
            h.player_bus_swapped(now.uid, next.uid, &mut next.vehicle);
        }
        next.vehicle.host.auto_clutch = if self.settings.auto_clutch { 1.0 } else { 0.0 };
        self.placed.push(now);
        let name = format!("{} {}", next.vehicle.ty.def.manufacturer, next.vehicle.ty.def.type_name);
        if let Some(cam) = self.camera.as_ref() {
            self.camera = Some(next.camera(&self.view, cam));
        }
        self.player = Some(next);
        self.look = (0.0, 0.0);
        self.service_msg = Some((format!("Now driving: {}", name.trim()), 4.0));
    }

    /// Move the clock by `secs` (the traffic's clock with it), as OMSI's time dialog does.
    pub(crate) fn shift_clock(&mut self, secs: f64) {
        let mut t = self.clock.time + secs;
        while t < 0.0 {
            t += 86400.0;
            self.clock.day_of_year = if self.clock.day_of_year > 1 { self.clock.day_of_year - 1 } else { omsi_sim::clock::days_in_year(self.clock.year - 1) };
        }
        while t >= 86400.0 {
            t -= 86400.0;
            self.clock.day_of_year = self.clock.day_of_year % omsi_sim::clock::days_in_year(self.clock.year) + 1;
        }
        self.clock.time = t;
        if let Some(tr) = self.traffic.as_mut() {
            tr.day_time += secs;
        }
        if let Some(p) = self.player.as_mut() {
            p.vehicle.host.clock = self.clock.clone();
        }
        let h = (t / 3600.0) as u32;
        self.service_msg = Some((format!("Clock: {h:02}:{:02}", ((t / 60.0) as u32) % 60), 3.0));
    }

    /// Start the game again on the quicksave (`Situations/quicksave.osn` of the content
    /// folder): a fresh process, as OMSI loads a situation into a fresh world. False when
    /// there is none.
    pub(crate) fn load_quicksave(&mut self) -> bool {
        let dir = crate::startup::content_dir().unwrap_or_else(|| self.args.root.clone()).join("Situations");
        let file = dir.join("quicksave.osn");
        if !file.exists() {
            self.service_msg = Some(("No quicksave yet (Ctrl+S saves one)".into(), 4.0));
            return false;
        }
        let Ok(exe) = std::env::current_exe() else { return false };
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--root").arg(&self.args.root).arg("--no-menu").arg("--situation").arg(&file);
        match cmd.spawn() {
            Ok(_) => {
                log::info!("loading {} in a new game", file.display());
                true
            }
            Err(e) => {
                self.service_msg = Some((format!("Could not start the game again: {e}"), 5.0));
                false
            }
        }
    }

    /// One of the depot services of the game menu: "refuel", "wash" or "repair".
    pub(crate) fn run_service(&mut self, kind: &str) {
        let Some(w) = self.world.clone() else { return };
        let Some(p) = self.player.as_mut() else { return };
        let one = Args {
            refuel: kind == "refuel",
            wash: kind == "wash",
            repair: kind == "repair",
            ..self.args.clone()
        };
        let at_station = at_petrol_station(&w, &p.vehicle);
        let mut clock = self.clock.clone();
        let msg = run_services(&one, &mut p.vehicle, &mut clock, w.global.repair_time_min, at_station);
        while clock.time >= 86400.0 {
            clock.time -= 86400.0;
            clock.day_of_year = clock.day_of_year % omsi_sim::clock::days_in_year(clock.year) + 1;
        }
        self.clock = clock;
        p.vehicle.host.clock = self.clock.clone();
        for line in &msg {
            log::info!("{line}");
        }
        if let Some(line) = msg.into_iter().next() {
            self.service_msg = Some((line, 6.0));
        }
    }

    #[cfg(windows)]
    fn vr_action(&mut self, name: &str) -> bool {
        if name == "vr_toggle_mode" {
            self.vr_zoom_active = false;
            if !self.settings.vr_requested() { return false; }
            if self.vr.is_some() {
                self.vr = None;
                self.service_msg = Some(("Desktop mode".into(), 2.0));
            } else if let Some(renderer) = self.renderer.as_ref() {
                match crate::openxr::Vr::new(renderer, self.settings.vr_scale,
                                             self.settings.vr_desktop_mirror) {
                    Ok(vr) => {
                        self.vr = Some(vr);
                        self.service_msg = Some(("VR mode".into(), 2.0));
                    }
                    Err(e) => {
                        log::error!("OpenXR could not restart: {e:#}");
                        self.service_msg = Some((format!("{}: {e}", omsi_ui::tr("Could not start VR")), 5.0));
                    }
                }
            }
            self.hover_key = None;
            return true;
        }
        if self.vr.is_none() { return false; }
        match name {
            "vr_recenter" => {
                self.vr.as_mut().unwrap().recenter();
                self.look = (0.0, 0.0);
                self.service_msg = Some(("VR view recentered".into(), 2.0));
            }
            "vr_toggle_desktop_mirror" => {
                let visible = self.vr.as_mut().unwrap().toggle_desktop_mirror();
                self.settings.vr_desktop_mirror = visible;
                self.service_msg = Some((if visible { "Desktop VR mirror on" }
                                         else { "Desktop VR mirror off" }.into(), 2.0));
            }
            "vr_toggle_navigator" => self.vr_nav_adjust("enabled", 1.0),
            "vr_position_navigator" => self.start_vr_nav_edit(),
            _ => return false,
        }
        true
    }

    /// One of OMSI's global key actions; false when it is not one this game does.
    pub(crate) fn game_action(&mut self, name: &str) -> bool {
        #[cfg(windows)]
        if self.vr_action(name) { return true; }
        match name {
            "sim_pause" => self.toggle_pause(),
            "screenshot" => self.take_screenshot(),
            "quicksave" => self.quick_save(),
            "view_set_ego" => {
                // on foot from where the camera is (beside the bus in the driver's view)
                if let (Some(cam), Some(p)) = (self.camera.as_mut(), self.player.as_ref()) {
                    if self.view != "free" {
                        let h = (p.vehicle.heading as f32 - 90.0).to_radians();
                        cam.position = p.vehicle.position + glam::DVec3::new(h.sin() as f64, h.cos() as f64, 0.0) * 2.5;
                        cam.yaw = p.vehicle.heading as f32;
                        cam.pitch = 0.0;
                    }
                }
                self.view = "free".into();
                self.ego = true;
                self.service_msg = Some(("On foot: W A S D walk, Shift runs, right mouse button looks (F1 back to the bus)".into(), 5.0));
            }
            "view_set_driver" => self.view = "driver".into(),
            "view_set_passenger" => self.view = "pax".into(),
            "view_set_outside" => self.view = "outside".into(),
            "view_set_map" => {
                // OMSI's map view (F4) is a camera flown over the map; the city map of the
                // navigator stays on Shift+M
                if self.view != "free" {
                    if let (Some(cam), Some(p)) = (self.camera.as_mut(), self.player.as_ref()) {
                        let h = (p.vehicle.heading as f32).to_radians();
                        cam.position = p.vehicle.position
                            + glam::DVec3::new(-(h.sin() as f64) * 25.0, -(h.cos() as f64) * 25.0, 30.0);
                        cam.yaw = p.vehicle.heading as f32;
                        cam.pitch = -45.0;
                    }
                }
                self.view = "free".into();
                self.ego = false;
            }
            // the timetable and the ticket desk each have a camera of their own in the bus
            // (`[view_schedule]`, `[view_ticketselling]`): the key switches the driver's view
            // to it and back
            "view_set_schedule" | "view_set_ticketselling" => {
                let schedule = name == "view_set_schedule";
                if schedule {
                    self.timetable = !self.timetable;
                }
                if let Some(p) = self.player.as_mut() {
                    let def = &p.vehicle.ty.def;
                    let cam = if schedule { def.view_schedule } else { def.view_ticketselling };
                    let n = def.cameras_driver.len().max(1);
                    // (the choice counts from the standard camera)
                    if let Some(c) = cam.filter(|c| *c < def.cameras_driver.len()).map(|c| (c + n - def.camera_std % n) % n) {
                        let back = p.cam_before_special.take();
                        if self.view == "driver" && p.cam_choice.0 == c {
                            p.cam_choice.0 = back.unwrap_or(0);
                        } else {
                            p.cam_before_special = Some(p.cam_choice.0);
                            p.cam_choice.0 = c;
                            self.view = "driver".into();
                        }
                        self.sync_view_look();
                    } else if !schedule {
                        self.service_msg = Some(("This bus has no ticket desk camera".into(), 3.0));
                    }
                }
            }
            "view_toggle_informationdisplay" => self.info_bar = !self.info_bar,
            // (Omsi.exe's camera reset, 0x7edde4, puts back the field of view with the
            // direction: the zoom goes as well, #244)
            "view_reset_direction" => {
                // F1 eases home (look + zoom glide); anywhere else, and with
                // the glide switched off, it snaps like before.
                if self.view == "driver"
                    && self.settings.driverview_smooth
                    && (self.look != (0.0, 0.0) || self.view_zoom.contains_key(&self.view))
                {
                    let zoom = self.view_zoom.get(&self.view).copied().unwrap_or(1.0);
                    self.f1_reset = Some((self.look, zoom, 0.0));
                } else {
                    self.f1_reset = None;
                    self.look = (0.0, 0.0);
                    self.view_zoom.remove(&self.view);
                }
                #[cfg(windows)]
                if let Some(vr) = self.vr.as_mut() { vr.recenter(); }
            }
            // (Space in Inputs/keyboard.cfg: every view looks ahead again, and back to the
            // standard camera - "center")
            "view_reset_all_directions" => {
                // F1 eases home (look + zoom glide) from the values in place:
                // zeroing them first would flash a frame of the destination.
                // Everything else snaps.
                if self.view == "driver"
                    && self.settings.driverview_smooth
                    && (self.look != (0.0, 0.0) || self.view_zoom.contains_key(&self.view))
                {
                    let zoom = self.view_zoom.get(&self.view).copied().unwrap_or(1.0);
                    self.f1_reset = Some((self.look, zoom, 0.0));
                    self.view_looks.clear();
                    self.view_zoom.retain(|k, _| k == "driver");
                } else {
                    self.f1_reset = None;
                    self.look = (0.0, 0.0);
                    self.view_looks.clear();
                    self.view_zoom.clear();
                }
                self.orbit = ORBIT_DEFAULT;
                if let Some(p) = self.player.as_mut() {
                    p.cam_choice = (0, 0);
                }
            }
            // the next (or the previous) view mode, driver - passenger - outside - map and
            // round again; nothing on foot (Omsi.exe 0x706278 @0x70634a: (mode + 1) and 3,
            // @0x706392 the inverse)
            "view_toggle_viewpoint" | "view_toggle_viewpoint_inverse" => {
                if self.ego {
                    return true;
                }
                let mode = match self.view.as_str() {
                    "driver" => 0,
                    "pax" => 1,
                    "outside" => 2,
                    _ => 3,
                };
                let next = if name == "view_toggle_viewpoint" { (mode + 1) % 4 } else { (mode + 3) % 4 };
                return self.game_action(["view_set_driver", "view_set_passenger", "view_set_outside", "view_set_map"][next]);
            }
            "view_interiorcam_plus" | "view_interiorcam_minus" => {
                let Some(p) = self.player.as_mut() else { return true };
                // (the interior cameras only cycle in the interior: from outside the keys
                // would change an invisible camera)
                if !matches!(self.view.as_str(), "driver" | "pax") {
                    return true;
                }
                let (count, pax) = if self.view == "pax" { (p.pax_camera_count(), true) } else { (p.driver_camera_count(), false) };
                if count > 1 {
                    let c = if pax { &mut p.cam_choice.1 } else { &mut p.cam_choice.0 };
                    *c = if name == "view_interiorcam_minus" { (*c + count - 1) % count } else { (*c + 1) % count };
                    let n = *c + 1;
                    // (the camera left keeps its look, the one taken finds its own again)
                    self.sync_view_look();
                    self.service_msg = Some((format!("{} camera {n} of {count}", if pax { "Passenger" } else { "Driver" }), 2.0));
                }
            }
            "toggel_mouse_ctrl" => {
                self.set_mouse_drive(!self.mouse_drive);
                let msg = if self.mouse_drive { "Mouse steering on: across steers, up is the throttle, down the brake (O turns it off)" } else { "Mouse steering off" };
                self.service_msg = Some((msg.into(), 4.0));
            }
            "toggel_ctrler" => {
                if let Some(c) = self.controllers.as_mut() {
                    c.enabled = !c.enabled;
                    let msg = if !c.any() { "No game controller found" } else if c.enabled { "Game controller on" } else { "Game controller off" };
                    self.service_msg = Some((msg.into(), 3.0));
                }
            }
            _ => return false,
        }
        true
    }

    /// OMSI's `sim_pause`: the simulation stands still, the camera and the picture go on.
    /// Shift a manual gearbox up or down: the first of the usual trigger names the bus's
    /// scripts have (pressed and let go). False when it has none.
    pub(crate) fn shift_gear(&mut self, up: bool) -> bool {
        let names: &[&str] = if up {
            &["kw_s_plus", "upshift", "gear_up", "gearup", "shift_up", "gang_hoch", "schalten_hoch", "manual_up"]
        } else {
            &["kw_s_minus", "downshift", "gear_down", "geardown", "shift_down", "gang_runter", "schalten_runter", "manual_down"]
        };
        let Some(p) = self.player.as_mut() else { return false };
        // a gear lever with a trigger per gate (`kw_s_1`..`kw_s_10`, `kw_s_N`, `kw_s_R`: the
        // LiAZ MKPP - its `kw_s_plus` never fires, the script's condition is broken): the
        // next gate from the gear engaged, with the clutch down as the gates want it
        if p.vehicle.ty.program.trigger("kw_s_1").is_some() {
            let cur = p.vehicle.var("antrieb_getr_aktugang").unwrap_or(0.0).round() as i32;
            let to = if up { cur + 1 } else { cur - 1 };
            let name = match to {
                0 => "kw_s_N".to_string(),
                -1 => "kw_s_R".to_string(),
                n => format!("kw_s_{n}"),
            };
            if to < -1 || p.vehicle.ty.program.trigger(&name).is_none() {
                return false;
            }
            // (as a driver does it: the clutch down, the gear in, the clutch let up over a
            // second and a half as OMSI's clutch key lets it - let go at once, a bus pulling
            // away stalled its engine)
            p.vehicle.set_var("Clutch", 1.0);
            p.axes.clutch = 1.0;
            p.vehicle.trigger(&name);
            p.vehicle.trigger(&format!("{name}_off"));
            self.service_msg = Some((format!("Gear {}", match to { 0 => "N".to_string(), -1 => "R".to_string(), n => n.to_string() }), 1.5));
            return true;
        }
        let Some(n) = names.iter().find(|n| p.vehicle.ty.program.trigger(n).is_some()) else {
            self.service_msg = Some(("This vehicle has no manual gearbox to shift".into(), 2.0));
            return false;
        };
        p.vehicle.trigger(n);
        p.vehicle.trigger(&format!("{n}_off"));
        true
    }

    /// Switch the mouse steering on or off, and remember it for the next game. Switched off,
    /// the wheel stays where the mouse left it; switched on, it eases from where it is to
    /// the cursor for the first second.
    pub(crate) fn set_mouse_drive(&mut self, on: bool) {
        self.mouse_drive = on;
        if !on {
            crate::player::keep_wheel(self.player.as_mut());
            #[cfg(windows)]
            self.reset_vr_pointer();
        }
        self.mouse_steer = (self.player.as_ref().map(|p| p.vehicle.physics.controls.steering).unwrap_or(0.0), 1.0);
        self.mouse_pedals = self.player.as_ref().map(|p| (p.vehicle.physics.controls.throttle, p.vehicle.physics.controls.brake)).unwrap_or((0.0, 0.0));
        if self.settings.mouse_steering != on {
            self.settings.mouse_steering = on;
            crate::game_lists::remember_setting("mouse_steering", if on { "1" } else { "0" });
        }
    }

    pub(crate) fn toggle_pause(&mut self) {
        // (a LAN session goes on for the others: it cannot be paused)
        if self.lan.is_some() {
            self.service_msg = Some(("A LAN session cannot be paused".into(), 3.0));
            return;
        }
        self.paused = !self.paused;
        if self.game_menu.is_some() {
            // Keep the state a menu close should restore in step with P.
            self.menu_prev_pause = self.paused;
        }
    }

    /// Put the bus on the street nearest the world point `at` (the city map's Ctrl+click),
    /// facing along it.
    pub(crate) fn place_bus_at(&mut self, at: glam::DVec2) {
        if self.lan.as_ref().is_some_and(|l| l.role == omsi_net::Role::Client) {
            self.service_msg = Some(("In a LAN session only the host moves vehicles on the map".into(), 4.0));
            return;
        }
        let p = glam::DVec3::new(at.x, at.y, 0.0);
        // the traffic's lanes (the tiles loaded around the bus), else the navigator's of the
        // whole map: a street far off on a big map was "no street" until the bus had been
        // flown there (#235). (the height of the point does not matter: the nearest by the
        // ground plan)
        let nets = [self.traffic.as_ref().map(|t| &t.net), self.navigator.as_ref().and_then(|n| n.map_net())];
        let Some((net, (lane, s, _))) = nets
            .into_iter()
            .flatten()
            .find_map(|net| net.nearest_lane(p, omsi_sim::traffic::LaneKind::Street).filter(|(_, _, d)| *d <= 300.0).map(|f| (net, f)))
        else {
            self.service_msg = Some(("No street near that point".into(), 3.0));
            return;
        };
        let l = &net.lanes[lane];
        let (pos, heading) = l.at(s);
        let heading = heading as f64;
        crate::admin::teleport(self, pos, heading);
        self.service_msg = Some(("The bus stands where the map was clicked".into(), 3.0));
    }

    /// The world follows the sim date as OMSI's does at the day's change:
    /// the chrono scenarios in force (the tiles they change are read again) and the
    /// season's textures - also when the weather turns to snow or thaws (every loaded tile
    /// is read again with the other texture folder).
    pub(crate) fn follow_date(&mut self) {
        let Some(w) = self.world.clone() else { return };
        let date = self.clock.date_code();
        let snow = self.weather.as_ref().is_some_and(|x| x.snow);
        let season = crate::world_load::season_folder_on(&self.args, &w.global, self.clock.day_of_year, snow).1;
        let Some((was_date, was_season)) = self.world_day.clone() else {
            self.world_day = Some((date, omsi_texture::season_folder()));
            return;
        };
        if was_date == date && was_season == season {
            return;
        }
        self.world_day = Some((date, season.clone()));
        let changed = if was_date != date { w.set_date(date) } else { Vec::new() };
        let (Some(st), Some(r), Some(scene)) = (self.streamer.as_mut(), self.renderer.as_ref(), self.scene.as_mut()) else { return };
        if was_season != season {
            log::info!("season: the textures of {:?} now (were {:?})", season, was_season);
            omsi_texture::set_season_folder(season);
            omsi_cfg::content_changed();
            st.reload(r, scene, None, self.audio.as_ref());
        } else if !changed.is_empty() {
            st.reload(r, scene, Some(&changed), self.audio.as_ref());
        }
    }

    /// `laststn.osn` in the map's folder (the content folder's copy: the original is never
    /// written), as OMSI keeps it: the launcher offers to continue it. Not
    /// in a tutorial, a LAN session or without a bus of one's own.
    pub(crate) fn save_last_situation(&mut self) -> Option<std::path::PathBuf> {
        if self.tutorial.is_some() || self.lan.is_some() || self.player.is_none() {
            return None;
        }
        let (Some(w), Some(cam)) = (self.world.as_ref(), self.camera.as_ref()) else { return None };
        let dir = std::path::Path::new(&self.args.map.replace('\\', "/")).parent().map(|d| d.to_path_buf())?;
        let base = crate::startup::content_dir()?;
        let dir = base.join(dir);
        let _ = std::fs::create_dir_all(&dir);
        let out = dir.join("laststn.osn");
        let sit = build_situation(&self.args, w, &self.clock, self.args.weather.as_deref(), self.player.as_ref(), &self.placed, cam, self.duty.as_ref(), "Last situation");
        match sit.save(&out) {
            Ok(()) => {
                log::info!("saved the last situation {}", out.display());
                Some(out)
            }
            Err(e) => {
                log::warn!("saving {}: {e}", out.display());
                None
            }
        }
    }

    /// The graphics device was lost (the driver reset the card: it ran out of memory, or a
    /// frame took it too long): the game starts again by itself on the situation just
    /// saved, with lighter graphics (`Settings::apply_safe_gpu`), and on Windows with the
    /// other graphics interface when the lost one was Vulkan. Twice at most in a row. False
    /// when it cannot (a LAN session, the tutorial, nothing to save): the session ends.
    pub(crate) fn restart_after_device_loss(&mut self) -> bool {
        let n = omsi_cfg::env::var("OMSI_SAFE_GPU").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        if n >= 2 {
            return false;
        }
        let Some(file) = self.save_last_situation() else { return false };
        let Ok(exe) = std::env::current_exe() else { return false };
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--root").arg(&self.args.root).arg("--no-menu").arg("--situation").arg(&file);
        cmd.env("OMSI_SAFE_GPU", (n + 1).to_string());
        // (on Windows the other interface: DirectX 12 after Vulkan, Vulkan after DirectX 12 -
        // an AMD Radeon's DX12 driver lost the device where its Vulkan one did not, #274)
        let name = self.renderer.as_ref().map(|r| r.adapter_name.clone()).unwrap_or_default();
        if cfg!(windows) {
            if name.contains("(Vulkan)") {
                cmd.env("OMSI_BACKEND", "dx12");
            } else if name.contains("(Dx12)") {
                cmd.env("OMSI_BACKEND", "vulkan");
            }
        }
        match cmd.spawn() {
            Ok(_) => {
                log::warn!("starting again with safer graphics on {} (the graphics device was lost)", file.display());
                true
            }
            Err(e) => {
                log::warn!("could not start the game again: {e}");
                false
            }
        }
    }

    /// Quick save: `Situations/quicksave.osn` next to the game, as OMSI's `quicksave`.
    pub(crate) fn quick_save(&mut self) {
        let (Some(w), Some(cam)) = (self.world.as_ref(), self.camera.as_ref()) else { return };
        // into openOMSI's content folder, never the original installation (the menu and
        // --situation find it there as they find a mod's files)
        let dir = crate::startup::content_dir().unwrap_or_else(|| self.args.root.clone()).join("Situations");
        let _ = std::fs::create_dir_all(&dir);
        let out = dir.join("quicksave.osn");
        let sit = build_situation(&self.args, w, &self.clock, self.args.weather.as_deref(), self.player.as_ref(), &self.placed, cam, self.duty.as_ref(), "Quicksave");
        match sit.save(&out) {
            Ok(()) => {
                log::info!("saved situation {} ({} vehicles)", out.display(), sit.vehicles.len());
                self.service_msg = Some(("Situation saved (quicksave)".into(), 3.0));
            }
            Err(e) => {
                log::warn!("saving {}: {e}", out.display());
                self.service_msg = Some((format!("Could not save: {e}"), 5.0));
            }
        }
    }

    /// OMSI's `screenshot`: the picture into the content folder's `Screenshots`, named by the
    /// date and time.
    pub(crate) fn take_screenshot(&mut self) {
        let dir = crate::startup::content_dir().unwrap_or_else(|| self.args.root.clone()).join("Screenshots");
        let _ = std::fs::create_dir_all(&dir);
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let path = dir.join(format!("omsi_{secs}.png"));
        self.service_msg = Some((format!("Screenshot: {}", path.display()), 4.0));
        self.shot = Some(path);
    }

    /// On foot, the own bus is within reach: inside it, or standing by it (a hand's reach
    /// round its body; a click still has to hit one of its meshes).
    pub(crate) fn foot_reaches_bus(&self) -> bool {
        if self.foot_bus() == Some(crate::humans::BusId::Player) {
            return true;
        }
        match (self.player.as_ref(), self.camera.as_ref()) {
            (Some(p), Some(c)) => {
                let bb = p.vehicle.ty.def.bounding_box.unwrap_or([2.5, 12.0, 3.0, 0.0, 0.0, 1.5]);
                let reach = (bb[0].max(bb[1]) as f64) * 0.5 + 3.0;
                (c.position - p.vehicle.position).length() < reach
            }
            _ => false,
        }
    }

    pub(crate) fn cockpit_cursor_ray(&self, cam: &Camera, size: (u32, u32)) -> (glam::DVec3, glam::Vec3, f32) {
        #[cfg(windows)]
        if let Some(ray) = self.vr.as_ref().and_then(|vr| vr.cursor_ray(self.cursor.0, self.cursor.1, size)) {
            return (ray.0, ray.1, ray.2 * 6.0);
        }
        let (o, d) = cursor_ray(cam, self.cursor.0, self.cursor.1, size.0 as f32, size.1 as f32);
        (o, d, pixel_angle(cam, size.1 as f32) * 6.0)
    }

    pub(crate) fn update_hover(&mut self) {
        if self.vr_nav_edit.is_some() {
            self.hover = None;
            self.hover_part = None;
            self.hover_hand = false;
            return;
        }
        #[cfg(windows)]
        if !self.mouse_drive && self.vr.as_ref().is_some_and(|vr| vr.needs_cursor_surface(
            self.cursor, self.game_menu.is_some() || self.chooser.is_some())) {
            let surface = self.player.as_ref()
                .zip(self.camera.as_ref())
                .zip(self.surface.as_ref())
                .filter(|_| matches!(self.view.as_str(), "driver" | "pax"))
                .map(|((player, camera), window)| {
                    let (origin, direction, _) = self.cockpit_cursor_ray(camera,
                                                                         (window.config.width, window.config.height));
                    (player.surface_hit(origin, direction),
                     (player.vehicle.position, player.vehicle.body_rotation()))
                });
            if let Some(vr) = self.vr.as_mut() {
                vr.set_cursor_surface(surface.as_ref().and_then(|s| s.0),
                                      surface.map(|s| s.1));
            }
        }
        let found = match (
            self.player.as_ref(),
            self.camera.as_ref(),
            self.surface.as_ref(),
        ) {
            (Some(p), Some(cam), Some(s)) if self.view != "free"
                && (self.view != "foot" || self.foot_reaches_bus())
                && !(self.vr_active() && self.mouse_drive
                && matches!(self.view.as_str(), "driver" | "pax")) => {
                let (o, d, spread) = self.cockpit_cursor_ray(cam, (s.config.width, s.config.height));
                p.hovered_part(o, d, spread)
            }
            // (in another player's bus nothing is offered: its switches are the driver's)
            _ => (None, false),
        };
        let (found, hand) = found;
        self.hover_hand = hand;
        match found {
            Some((name, true)) => {
                self.hover = Some(name);
                self.hover_part = None;
            }
            Some((name, false)) => {
                self.hover = None;
                self.hover_part = Some(name);
            }
            None => {
                self.hover = None;
                self.hover_part = None;
            }
        }
        // the cursor itself says when it is over something that can be operated
        // (steering with the mouse: a cross, as OMSI shows it; turning the view with the
        // right button held: the four arrows OMSI shows then, #185)
        // (zooming with the mouse: the up-down arrows, Omsi's crSizeNS)
        let rmb_zoom = self.buttons_held.1
            && !self.mmb_held
            && self.both_drag.is_none()
            && matches!(self.view.as_str(), "driver" | "outside" | "pax" | "free");
        let kind: u8 = if self.both_drag.is_some() && self.game_menu.is_none() {
            4
        } else if rmb_zoom && self.game_menu.is_none() {
            4
        } else if self.mouse_look && self.game_menu.is_none() {
            3
        } else if self.mouse_drive && matches!(self.view.as_str(), "driver" | "outside" | "pax") && self.game_menu.is_none() {
            2
        } else if self.hover.is_some() || self.hover_hand {
            1
        } else {
            0
        };
        if kind != self.cursor_kind {
            self.cursor_kind = kind;
            if let Some(w) = self.window.as_ref() {
                w.set_cursor(match kind {
                    4 => winit::window::CursorIcon::NsResize,
                    3 => winit::window::CursorIcon::Move,
                    2 => winit::window::CursorIcon::Crosshair,
                    1 => winit::window::CursorIcon::Pointer,
                    _ => winit::window::CursorIcon::Default,
                });
            }
        }
    }
}

/// Degrees the view turns per (logical) pixel of the cursor's way while looking round:
/// Omsi.exe's fov / 78.75 (TForm_main.Panel1MouseMove 0x82c5f8).
fn look_deg_per_px(fov_deg: f32) -> f32 {
    fov_deg / 78.75
}

/// F3 chase orbit step from raw drag pixels: full turn in yaw at 0.35
/// deg/px (faster than the head's 0.15), pitch between -60 (near top-down)
/// and +25 (just below eye level) around the -15 rest pose, so the camera
/// never swings under the bus. Pure (tested below).
pub(crate) fn chase_orbit_step(yaw: f32, pitch: f32, dx_px: f32, dy_px: f32) -> (f32, f32) {
    const GAIN: f32 = 0.35;
    (
        (yaw + dx_px * GAIN).rem_euclid(360.0),
        (pitch - dy_px * GAIN).clamp(-60.0, 25.0),
    )
}

/// Precision zoom step from a vertical drag: the zoom state `z` (0 wide ..
/// 1 full zoom) travels at `intent` per 364 px, and the FOV multiplier is
/// `1/(1+5.5*z)` — full zoom ~6.5x in. Drag down (`dy > 0`) zooms in.
/// Never past 1.0 (never wider than the bus's own field of view); the floor
/// is the caller's clamp. Pure (tested below).
pub(crate) fn precision_zoom_step(mult: f32, dy_px: f32, intent: f32) -> f32 {
    const RANGE: f32 = 5.5;
    const FULL_DRAG_PX: f32 = 364.0;
    let z = ((1.0 / mult.max(0.154) - 1.0) / RANGE).clamp(0.0, 1.0);
    let z2 = (z + dy_px * intent / FULL_DRAG_PX).clamp(0.0, 1.0);
    1.0 / (1.0 + RANGE * z2)
}

/// F1 zoom intent: the head zoom runs 20% slower than outside/free.
pub(crate) const ZOOM_INTENT_F1: f32 = 0.56;
/// Outside/free zoom intent: a full 364 px drag takes `z` 0 to 0.70.
pub(crate) const ZOOM_INTENT: f32 = 0.70;

/// Eased Space return for the F1 head: look and zoom glide home with the same
/// smootherstep the viewpoint glide uses (`CAM_BLEND_SECS`), instead of
/// teleporting. `t` seconds in; returns the current look, zoom and done.
/// Pure (tested below).
pub(crate) fn reset_blend(look_from: (f32, f32), zoom_from: f32, t: f32) -> ((f32, f32), f32, bool) {
    let x = (t / crate::app::CAM_BLEND_SECS).clamp(0.0, 1.0);
    let s = x * x * x * (x * (x * 6.0 - 15.0) + 10.0);
    (
        (look_from.0 * (1.0 - s), look_from.1 * (1.0 - s)),
        zoom_from + (1.0 - zoom_from) * s,
        x >= 1.0,
    )
}

#[cfg(test)]
mod look_tests {
    #[test]
    fn a_cursor_way_of_78_75_px_turns_by_the_field_of_view() {
        assert!((78.75 * super::look_deg_per_px(60.0) - 60.0).abs() < 1e-4);
    }

    #[test]
    fn chase_orbits_at_035_deg_px_with_stops_above_and_below() {
        // 100 px drag down-right: +35 yaw, -35 pitch.
        let (y, p) = super::chase_orbit_step(0.0, 0.0, 100.0, 100.0);
        assert!((y - 35.0).abs() < 1e-4 && (p + 35.0).abs() < 1e-4, "{y} {p}");
        // yaw wraps the full circle.
        assert!((super::chase_orbit_step(350.0, 0.0, 100.0, 0.0).0 - 25.0).abs() < 1e-3);
        // pitch never leaves the stops, whichever way it is dragged.
        assert_eq!(super::chase_orbit_step(0.0, 0.0, 0.0, -1000.0).1, 25.0);
        assert_eq!(super::chase_orbit_step(0.0, 0.0, 0.0, 1000.0).1, -60.0);
    }

    #[test]
    fn precision_zoom_follows_the_fov_curve_and_never_widens() {
        // a full 364 px drag down takes z 0 to 0.70: m = 1/(1+5.5*0.70).
        let m = super::precision_zoom_step(1.0, 364.0, super::ZOOM_INTENT);
        assert!((m - 1.0 / (1.0 + 5.5 * 0.70)).abs() < 1e-4, "{m}");
        // drag down zooms in, drag up undoes it, never past 1.0.
        let mid = super::precision_zoom_step(1.0, 100.0, super::ZOOM_INTENT);
        assert!(mid < 1.0 && mid > 0.45, "{mid}");
        assert!((super::precision_zoom_step(mid, -100.0, super::ZOOM_INTENT) - 1.0).abs() < 1e-4);
        assert_eq!(super::precision_zoom_step(1.0, -50.0, super::ZOOM_INTENT), 1.0);
        // F1 runs the same curve 20% slower.
        let slow = super::precision_zoom_step(1.0, 100.0, super::ZOOM_INTENT_F1);
        assert!(slow > mid && slow < 1.0, "{slow} vs {mid}");
    }

    #[test]
    fn space_return_eases_home_like_the_viewpoint_glide() {
        // start: untouched; halfway: ~halfway home; end: exact and done.
        let (look, zoom, done) = super::reset_blend((30.0, -10.0), 0.5, 0.0);
        assert_eq!((look, zoom, done), ((30.0, -10.0), 0.5, false));
        let (look, zoom, done) = super::reset_blend((30.0, -10.0), 0.5, 0.3);
        assert!(look.0 > 3.0 && look.0 < 27.0 && zoom > 0.5 && zoom < 1.0 && !done);
        let (look, zoom, done) = super::reset_blend((30.0, -10.0), 0.5, 0.6);
        assert_eq!((look, zoom, done), ((0.0, 0.0), 1.0, true));
        assert!(super::reset_blend((30.0, -10.0), 0.5, 5.0).2);
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
    let Ok(v) = omsi_cfg::env::var("OMSI_INPUT") else {
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

/// The game menu on a server (`--lan-join https://…`): the world's clock and weather are the
/// server's, and the way out leaves the server.
pub(crate) const SERVER_GAME_MENU: [(&str, &str); 21] = [
    ("resume", "Resume"),
    ("options", "Options..."),
    ("dest", "Destination display..."),
    ("hof", "Depot file (HOF)..."),
    ("switch", "Drive the next vehicle"),
    ("place", "Place a vehicle..."),
    ("couple", "Couple"),
    ("uncouple", "Uncouple"),
    ("remove", "Remove this vehicle (on foot)"),
    ("clearplaced", "Remove the placed vehicles"),
    ("getout", "Get up and out (on foot)"),
    ("reset", "Put the vehicle back on its wheels"),
    ("map", "City map"),
    ("shot", "Screenshot"),
    ("timetable", "Timetable"),
    ("info", "Information bar"),
    ("refuel", "Refuel"),
    ("wash", "Wash"),
    ("repair", "Repair"),
    ("editor", "Object editor"),
    ("quit", "Leave the server"),
];

/// The game menu's lines for a session with these arguments.
pub(crate) fn game_menu_for(args: &crate::Args) -> &'static [(&'static str, &'static str)] {
    if args.lan_join.as_deref().map(|t| omsi_net::ws::ws_url(t).is_some()).unwrap_or(false) {
        &SERVER_GAME_MENU
    } else {
        &GAME_MENU
    }
}

impl crate::App {
    /// The game menu's lines for this session: back to the own bus while walking about,
    /// the administration for a host and a server's admin.
    pub(crate) fn game_menu_items(&self) -> Vec<(&'static str, &'static str)> {
        let mut v: Vec<(&'static str, &'static str)> = game_menu_for(&self.args).to_vec();
        let mut at = 1;
        if self.on_foot.is_some() && self.player.is_some() {
            v.insert(at, ("tobus", "Back to my bus"));
            at += 1;
        }
        // without a bus of one's own: nothing of a bus's to offer
        if self.player.is_none() {
            v.retain(|x| !matches!(x.0, "remove" | "couple" | "uncouple" | "refuel" | "wash" | "repair" | "duty" | "number" | "dest" | "hof" | "teleport" | "getout" | "reset"));
            if self.placed.is_empty() {
                v.retain(|x| x.0 != "switch");
            }
        }
        if self.placed.is_empty() {
            v.retain(|x| x.0 != "clearplaced");
        }
        if self.on_foot.is_some() {
            v.retain(|x| x.0 != "getout");
        }
        if self.navigator.is_none() {
            v.retain(|x| x.0 != "map");
        }
        let host = self.lan.as_ref().map(|l| l.role == omsi_net::Role::Host).unwrap_or(false);
        if host || self.is_admin {
            v.insert(at, ("admin", "Administration..."));
        }
        // (a client's clock and weather are the host's)
        if self.lan.as_ref().map(|l| l.role == omsi_net::Role::Client).unwrap_or(false) {
            v.retain(|x| !matches!(x.0, "weather" | "clock" | "later" | "earlier" | "later10" | "earlier10" | "editor"));
        }
        // the everyday lines first; the rest behind "More..." (27 lines to scroll through
        // was the pause menu players found confusing)
        if self.menu_more {
            v.retain(|x| !MENU_BASIC.contains(&x.0) || x.0 == "quit");
            v.insert(0, ("less", "< Back"));
        } else {
            v.retain(|x| MENU_BASIC.contains(&x.0));
            let at = v.iter().position(|x| x.0 == "quit").unwrap_or(v.len());
            v.insert(at, ("more", "More..."));
        }
        v
    }
}

/// The lines the game menu shows before "More...".
const MENU_BASIC: [&str; 12] = ["resume", "tobus", "options", "duty", "dest", "map", "timetable", "getout", "reset", "save", "admin", "quit"];

/// The lines of the game menu: (what, label).
pub(crate) const GAME_MENU: [(&str, &str); 33] = [
    ("resume", "Resume"),
    ("options", "Options..."),
    ("duty", "Line and tour..."),
    ("driver", "Driver..."),
    ("number", "Fleet number..."),
    ("dest", "Destination display..."),
    ("hof", "Depot file (HOF)..."),
    ("switch", "Drive the next vehicle"),
    ("place", "Place a vehicle..."),
    ("couple", "Couple"),
    ("uncouple", "Uncouple"),
    ("remove", "Remove this vehicle (on foot)"),
    ("clearplaced", "Remove the placed vehicles"),
    ("getout", "Get up and out (on foot)"),
    ("reset", "Put the vehicle back on its wheels"),
    ("map", "City map"),
    ("teleport", "Move the bus on the map..."),
    ("save", "Save the situation"),
    ("load", "Load the quicksave"),
    ("weather", "Next weather"),
    ("clock", "Set the clock..."),
    ("later", "Clock +1 hour"),
    ("later10", "Clock +10 minutes"),
    ("earlier10", "Clock -10 minutes"),
    ("earlier", "Clock -1 hour"),
    ("shot", "Screenshot"),
    ("timetable", "Timetable"),
    ("info", "Information bar"),
    ("refuel", "Refuel"),
    ("wash", "Wash"),
    ("repair", "Repair"),
    ("editor", "Object editor"),
    ("quit", "End the session"),
];

/// `App::sync_view_look` for where `self` is borrowed in parts.
/// See `App::look_key`.
pub(crate) fn look_key_of(view: &str, cam: Option<(usize, usize)>) -> String {
    match (view, cam) {
        ("driver", Some((d, _))) => format!("driver#{d}"),
        ("pax", Some((_, x))) => format!("pax#{x}"),
        _ => view.to_string(),
    }
}

pub(crate) fn swap_view_look(look: &mut (f32, f32), looks: &mut std::collections::HashMap<String, (f32, f32)>, look_view: &mut String, view: &str) {
    if look_view != view {
        let old = std::mem::replace(look_view, view.to_string());
        if !old.is_empty() {
            looks.insert(old, *look);
        }
        *look = looks.get(view).copied().unwrap_or((0.0, 0.0));
    }
}
