//! The game menu (Esc): its lines, its keys, the mouse wheel on it and what its lines do.

use super::*;

impl App {
    /// What the cursor points at, for the HUD. Recomputed every frame: the head turns and
    /// the bus moves under a cursor that is standing still.
    /// Open the game menu: the simulation pauses (not in a LAN session, which runs on
    /// for the other players).
    pub(crate) fn open_game_menu(&mut self) {
        self.menus.menu_prev_pause = self.paused;
        // the menu takes the keys, their key-ups too: what is held now is let go here, or a
        // steering key let go in the menu went on turning the wheel to full lock once the
        // menu closed (a throttle key went on accelerating, a door button stayed pressed)
        self.release_vehicle_keys();
        if self.net.lan.is_none() {
            self.paused = true;
        }
        self.menus.game_menu = Some(0);
        self.menus.menu_top = None;
        self.menus.menu_kbd = true;
        self.menus.menu_drag = None;
    }

    pub(crate) fn close_game_menu(&mut self) {
        self.menus.key_capture = None;
        if self.menus.menu_edit_icao {
            if let Some(w)=self.window.as_ref(){w.set_ime_allowed(false);}
            self.menus.menu_edit_icao=false; self.menus.menu_edit=None;
        }
        self.menus.game_menu = None;
        self.menus.menu_top = None;
        self.paused = self.menus.menu_prev_pause;
    }

    pub(crate) fn menu_key(&mut self, event_loop: &ActiveEventLoop, code: KeyCode) {
        self.menus.menu_kbd = true;
        if self.menus.key_capture.is_some() {
            match code {
                KeyCode::ShiftLeft | KeyCode::ShiftRight | KeyCode::ControlLeft | KeyCode::ControlRight | KeyCode::AltLeft | KeyCode::AltRight => {}
                KeyCode::Escape => self.cancel_key_capture(),
                KeyCode::Delete | KeyCode::Backspace => self.apply_key_capture(None, 0),
                _ => match crate::keys::dik_code(code) {
                    Some(scan) => {
                        let chord = omsi_content::input::chord(
                            self.input.keys.contains(&KeyCode::ShiftLeft) || self.input.keys.contains(&KeyCode::ShiftRight),
                            self.input.keys.contains(&KeyCode::ControlLeft) || self.input.keys.contains(&KeyCode::ControlRight),
                            self.input.keys.contains(&KeyCode::AltLeft) || self.input.keys.contains(&KeyCode::AltRight),
                        );
                        self.apply_key_capture(Some(scan), chord);
                    }
                    None => self.service_msg = Some(("That key has no DirectInput scan code".into(), 3.0)),
                },
            }
            return;
        }
        if self.menus.chooser.is_some() {
            self.chooser_key(code);
            return;
        }
        let n = self.game_menu_items().len();
        let sel = self.menus.game_menu.unwrap_or(0);
        let modified = self.input.keys.iter().any(|key| {
            matches!(*key, KeyCode::ControlLeft | KeyCode::ControlRight | KeyCode::AltLeft | KeyCode::AltRight | KeyCode::ShiftLeft | KeyCode::ShiftRight)
        });
        self.menus.menu_top = None;
        match code {
            // P changes only the simulation state, even while a menu is open.
            KeyCode::KeyP if !modified => self.toggle_pause(),
            KeyCode::Escape => self.close_game_menu(),
            KeyCode::ArrowUp | KeyCode::KeyW => self.menus.game_menu = Some(self.menu_step(sel, n, false)),
            KeyCode::ArrowDown | KeyCode::KeyS => self.menus.game_menu = Some(self.menu_step(sel, n, true)),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.menu_choose(event_loop, sel),
            _ => {}
        }
    }

    /// The open drop-down's scroll bar held with the mouse at height `y`: the first entry
    /// shown follows the thumb.
    pub(crate) fn drag_dropdown(&mut self, y: f32) {
        let (Some(grab), Some(ui)) = (self.menus.dd_scroll_drag, self.ui.as_ref()) else { return };
        let Some((track, thumb)) = ui.dd_scroll else { return };
        let rows = ui.dd_rows.max(1);
        let Some(d) = self.menus.dropdown.as_mut() else { return };
        d.top = dropdown_top_at(y - grab, track, thumb[3] - thumb[1], d.items.len(), rows);
    }

    /// The scroll bar of the timetable beside the tours held with the mouse at height `y`:
    /// the first stop shown follows the thumb.
    pub(crate) fn drag_pane(&mut self, y: f32) {
        let (Some(grab), Some(k)) = (self.menus.pane_scroll_drag, self.menus.chooser) else { return };
        let Some((track, thumb, n, fit)) = self.ui.as_ref().and_then(|u| u.menu_pane_scroll) else { return };
        self.menus.pane_scroll = Some((k, dropdown_top_at(y - grab, track, thumb[3] - thumb[1], n, fit)));
    }

    /// The mouse wheel over the game menu: the chosen line moves (the menu scrolls with it),
    /// in a list the same; no wrapping round.
    pub(crate) fn menu_wheel(&mut self, amount: f32) {
        // (an open drop-down scrolls, not the window under it)
        if self.menus.dropdown.is_some() {
            self.menus.wheel_acc += amount;
            let steps = self.menus.wheel_acc.trunc() as i64;
            if steps == 0 {
                return;
            }
            self.menus.wheel_acc -= steps as f32;
            let rows = self.ui.as_ref().map(|u| u.dd_rows).unwrap_or(8);
            if let Some(d) = self.menus.dropdown.as_mut() {
                let max = d.items.len().saturating_sub(rows) as i64;
                d.top = (d.top as i64 - steps).clamp(0, max) as usize;
            }
            return;
        }
        self.menus.wheel_acc += amount;
        let steps = self.menus.wheel_acc.trunc() as i64;
        if steps == 0 {
            return;
        }
        self.menus.wheel_acc -= steps as f32;
        // the wheel over the timetable beside the tours scrolls its stops
        if let (Some(u), Some(k)) = (self.ui.as_ref(), self.menus.chooser) {
            let (x, y) = self.input.cursor;
            if u.menu_pane_box.is_some_and(|r| x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3]) {
                let first = (u.menu_pane_start as i64 - steps).max(0) as usize;
                self.menus.pane_scroll = Some((k, first));
                return;
            }
        }
        // the list scrolls under the mouse; what is chosen stays chosen (the wheel used to
        // walk the highlight up and down the lines)
        let n = self.menu_len() as f32;
        let (start, rows) = self.ui.as_ref().map(|u| (u.menu_start as f32, u.menu_rows as f32)).unwrap_or((0.0, n));
        let top = (self.menus.menu_top.unwrap_or(start) - steps as f32).clamp(0.0, (n - rows).max(0.0));
        self.menus.menu_top = Some(top);
    }

    /// How many lines the menu shows now (the chooser's list, else the game menu's).
    pub(crate) fn menu_len(&self) -> usize {
        match self.menus.chooser {
            Some(_) => self.menus.admin_list.as_ref().unwrap_or(&self.menus.vehicle_list).len(),
            None => self.game_menu_items().len(),
        }
    }

    /// Do what line `k` of the game menu says.
    pub(crate) fn menu_choose(&mut self, event_loop: &ActiveEventLoop, k: usize) {
        // (a click in an open list keeps the scroll where it is: no jump to the line)
        self.menus.menu_top = if self.menus.chooser.is_some() { self.ui.as_ref().map(|u| u.menu_start as f32) } else { None };
        if self.menus.chooser.is_some() {
            self.chooser_pick(k);
            return;
        }
        // (a greyed-out line does nothing)
        if self.menu_item_off(k) {
            return;
        }
        let Some(id) = self.game_menu_items().get(k).map(|m| m.0) else { return };
        match id {
            "resume" => self.close_game_menu(),
            "options" => self.open_list(crate::game_lists::ListKind::Options(0)),
            "controls" => self.open_list(crate::game_lists::ListKind::Controls),
            "camera" => {
                let tab = crate::game_lists::options_tab(self, "Camera");
                self.open_list(crate::game_lists::ListKind::Options(tab));
            }
            "vehicle" => self.open_list(crate::game_lists::ListKind::Vehicle(0)),
            "world" => self.open_list(crate::game_lists::ListKind::World(0)),
            "copycode" => {
                self.close_game_menu();
                self.copy_server_code();
            }
            "admin" => self.open_list(crate::game_lists::ListKind::Admin),
            "duty" => self.open_list(crate::game_lists::ListKind::Lines),
            "map" => {
                self.close_game_menu();
                if let Some(n) = self.menus.navigator.as_mut() {
                    if !n.map_open() {
                        n.toggle_map();
                    }
                }
            }
            "save" => {
                self.quick_save();
                self.close_game_menu();
            }
            "saveslot" => {
                self.save_slot();
                self.close_game_menu();
            }
            "shot" => {
                self.close_game_menu();
                self.take_screenshot();
            }
            "skipstop" => {
                self.close_game_menu();
                self.skip_next_stop();
            }
            // the route ends here: free drive, as the list of lines has it
            "endduty" => {
                crate::game_lists::end_duty(self);
                self.close_game_menu();
            }
            "tobus" => {
                self.close_game_menu();
                self.back_to_bus();
            }
            "load" => {
                self.menus.game_menu = None;
                if self.load_quicksave() {
                    self.finish_session();
                    crate::platform::exit(event_loop);
                }
            }
            "quit" => {
                self.menus.game_menu = None;
                self.finish_session();
                crate::platform::exit(event_loop);
            }
            // (the rest are the lines of the vehicle and world pages)
            other => {
                self.page_action(other);
            }
        }
    }

    /// The actions of the vehicle and world pages (and of what the plugins and the input
    /// script ask of the menu by name). False when `id` is none of them.
    pub(crate) fn page_action(&mut self, id: &str) -> bool {
        match id {
            "swap" | "place" => {
                // (a plain "Place a vehicle" puts one beside; "Swap" in the driven one's place)
                self.menus.swap_pending = id == "swap" && self.player.is_some();
                if self.menus.vehicle_list.is_empty() {
                    let menu = crate::menu::Menu::new(&self.args.root, &self.args.map);
                    self.menus.vehicle_meta = menu.vehicles.iter().zip(menu.vehicle_meta).map(|(v, meta)| (v.1.clone(), meta)).collect();
                    self.menus.vehicle_list = menu.vehicles;
                    // (alphabetical)
                    self.menus.vehicle_list.sort_by_key(|v| v.0.to_lowercase());
                    crate::mt::protect(self.menus.vehicle_list.iter().map(|v| v.0.as_str()));
                }
                if self.menus.vehicle_list.is_empty() {
                    self.service_msg = Some(("No vehicles found".into(), 3.0));
                } else if crate::lan::server_offers().is_some_and(|o| !self.menus.vehicle_list.iter().any(|v| crate::lan::offers(&o, &v.1))) {
                    self.service_msg = Some(("The server offers none of the vehicles installed here".into(), 4.0));
                } else {
                    // (as the launcher's bus step: the manufacturer, then the type)
                    self.open_list(crate::game_lists::ListKind::PlaceMaker);
                }
            }
            "couple" => {
                self.close_game_menu();
                self.couple();
            }
            "uncouple" => {
                self.close_game_menu();
                self.uncouple();
            }
            "tobus" => {
                self.close_game_menu();
                self.back_to_bus();
            }
            "remove" => {
                self.close_game_menu();
                self.remove_driven_vehicle();
            }
            "reload" => {
                self.close_game_menu();
                self.reload_driven_vehicle();
            }
            "clearplaced" => {
                self.close_game_menu();
                self.remove_placed_vehicles();
            }
            "getout" => {
                self.close_game_menu();
                self.get_up();
            }
            "reset" => {
                self.close_game_menu();
                if let Some(p) = self.player.as_ref() {
                    let (at, heading) = (p.vehicle.position, p.vehicle.heading);
                    crate::admin::teleport(self, at, heading);
                    self.service_msg = Some(("The vehicle stands on its wheels again".into(), 3.0));
                    self.service_event("reset", self.menu_by(), None);
                }
            }
            "teleport" => {
                self.close_game_menu();
                if let Some(n) = self.menus.navigator.as_mut() {
                    if !n.map_open() {
                        n.toggle_map();
                    }
                    self.menus.teleport_pick = true;
                    self.service_msg = Some(("Click a street on the map: the bus is put there".into(), 6.0));
                }
            }
            "driver" => self.open_list(crate::game_lists::ListKind::Drivers),
            "number" => self.open_list(crate::game_lists::ListKind::Numbers),
            "dest" => self.open_list(crate::game_lists::ListKind::Destinations),
            "hof" => self.open_list(crate::game_lists::ListKind::Hofs),
            "tplist" => self.open_list(crate::game_lists::ListKind::Spots),
            "editor" => {
                self.close_game_menu();
                self.toggle_editor();
            }
            "timetable" => {
                self.menus.timetable = !self.menus.timetable;
                self.close_game_menu();
            }
            "info" => {
                self.set_info_bar(!self.menus.info_bar);
                self.close_game_menu();
            }
            "refuel" | "wash" | "repair" => {
                self.close_game_menu();
                self.run_service(id, self.menu_by());
            }
            "weather" => {
                self.close_game_menu();
                self.next_weather();
            }
            "metar_once" => self.load_metar_once(),
            "metar_refresh" => self.refresh_metar_now(),
            "weather_custom" => self.current_weather_as_custom(),
            "switch" => {
                self.close_game_menu();
                self.switch_vehicle();
            }
            "later" | "earlier" | "later10" | "earlier10" => {
                self.close_game_menu();
                if self.net.lan.as_ref().map(|l| l.role == omsi_net::Role::Client).unwrap_or(false) {
                    self.service_msg = Some(("In a LAN session the host sets the clock".into(), 3.0));
                } else {
                    self.shift_clock(match id {
                        "later" => 3600.0,
                        "earlier" => -3600.0,
                        "later10" => 600.0,
                        _ => -600.0,
                    });
                }
            }
            _ => return false,
        }
        true
    }
}

/// The game menu on a server (`--lan-join https://…`): the world's clock and weather are the
/// server's, and the way out leaves the server.
pub(crate) const SERVER_GAME_MENU: [(&str, &str); 8] = [
    ("resume", "Resume"),
    ("options", "Options..."),
    ("camera", "Camera..."),
    ("vehicle", "Vehicle options..."),
    ("world", "World options..."),
    ("map", "City map"),
    ("shot", "Screenshot"),
    ("quit", "Leave the server"),
];

/// Whether the session was started on a server (`--lan-join https://…`).
pub(crate) fn on_server(args: &crate::Args) -> bool {
    args.lan_join.as_deref().map(|t| omsi_net::ws::ws_url(t).is_some()).unwrap_or(false)
}

/// The game menu's lines for a session with these arguments.
pub(crate) fn game_menu_for(args: &crate::Args) -> &'static [(&'static str, &'static str)] {
    if on_server(args) {
        &SERVER_GAME_MENU
    } else {
        &GAME_MENU
    }
}

impl crate::App {
    /// The game menu's lines for this session: back to the own bus while walking about,
    /// the administration for a host and a server's admin. What can be set is behind
    /// "Options", "Vehicle options" and "World options".
    pub(crate) fn game_menu_items(&self) -> Vec<(&'static str, &'static str)> {
        let mut v: Vec<(&'static str, &'static str)> = game_menu_for(&self.args).to_vec();
        let mut at = 1;
        if self.session.on_foot.is_some() && self.player.is_some() {
            v.insert(at, ("tobus", "Back to my bus"));
            at += 1;
        }
        // without a bus of one's own: no line to drive
        if self.player.is_none() {
            v.retain(|x| x.0 != "duty");
        }
        // ending the route is offered only while there is one, skipping a stop while its
        // trip still has one to come
        if self.session.duty.is_none() {
            v.retain(|x| x.0 != "endduty");
        }
        if !self.session.duty.as_ref().is_some_and(|d| d.stop_to_skip()) {
            v.retain(|x| x.0 != "skipstop");
        }
        if self.menus.navigator.is_none() {
            v.retain(|x| x.0 != "map");
        }
        let host = self.net.lan.as_ref().map(|l| l.role == omsi_net::Role::Host).unwrap_or(false);
        // the server code: a line to copy it, right under "World options" (only in a LAN session or on a server)
        if self.net.lan.is_some() || on_server(&self.args) {
            if let Some(w) = v.iter().position(|x| x.0 == "world") {
                v.insert(w + 1, ("copycode", "Copy server code"));
                at = at.max(w + 2);
            }
        }
        if host || self.net.is_admin {
            let before_quit = v.iter().position(|x| x.0 == "quit").unwrap_or(v.len()).max(at);
            v.insert(before_quit, ("admin", "Administration..."));
        }
        v
    }

    /// Put the session's server code on the clipboard.
    pub(crate) fn copy_server_code(&mut self) {
        // (the host's code; a player or a server's join code or address as it was entered)
        let Some(code) = self.net.lan.as_ref().and_then(|l| l.code()).map(|c| c.encode()).or_else(|| self.args.lan_join.clone()).filter(|c| !c.trim().is_empty()) else {
            self.service_msg = Some(("No server code: not in a LAN session or on a server".into(), 3.0));
            return;
        };
        #[cfg(not(target_os = "android"))]
        {
            thread_local! {
                // (kept alive: on X11 the text is gone when the clipboard is dropped)
                static CLIPBOARD: std::cell::RefCell<Option<arboard::Clipboard>> = const { std::cell::RefCell::new(None) };
            }
            let ok = CLIPBOARD.with(|c| {
                let mut c = c.borrow_mut();
                if c.is_none() {
                    *c = arboard::Clipboard::new().ok();
                }
                c.as_mut().is_some_and(|cb| cb.set_text(code.clone()).is_ok())
            });
            self.service_msg = Some(if ok { ("Server code copied".into(), 3.0) } else { (format!("{}: {code}", omsi_ui::tr("Server code")), 8.0) });
        }
        #[cfg(target_os = "android")]
        {
            self.service_msg = Some((format!("{}: {code}", omsi_ui::tr("Server code")), 8.0));
        }
    }

    /// The ids of the game menu's lines that are greyed out and cannot be chosen now.
    pub(crate) fn menu_disabled_ids(&self) -> &'static [&'static str] {
        &[]
    }

    /// Whether line `k` of the game menu is greyed out.
    pub(crate) fn menu_item_off(&self, k: usize) -> bool {
        self.menus.chooser.is_none() && self.game_menu_items().get(k).is_some_and(|m| self.menu_disabled_ids().contains(&m.0))
    }

    /// The next line up or down from `from` that can be chosen (round the ends; the greyed-out
    /// lines are skipped).
    fn menu_step(&self, from: usize, n: usize, down: bool) -> usize {
        let mut k = from;
        for _ in 0..n {
            k = if down { (k + 1) % n } else { (k + n - 1) % n };
            if !self.menu_item_off(k) {
                return k;
            }
        }
        from
    }
}

/// The lines of the game menu: (what, label). What can be set is on the pages behind
/// "Options", "Vehicle options" and "World options" (see `game_lists`).
pub(crate) const GAME_MENU: [(&str, &str); 15] = [
    ("resume", "Resume"),
    ("options", "Options..."),
    ("controls", "Controls..."),
    // (the driver's view - seat, field of view, head movement - straight from the pause
    // menu: it is what is changed most while driving, #908)
    ("camera", "Camera..."),
    ("vehicle", "Vehicle options..."),
    ("world", "World options..."),
    ("map", "City map"),
    ("duty", "Line and tour..."),
    ("skipstop", "Skip the next stop"),
    ("endduty", "End the tour"),
    ("save", "Save the situation"),
    ("saveslot", "Save to a new slot"),
    ("load", "Load the quicksave"),
    ("shot", "Screenshot"),
    ("quit", "End the session"),
];
