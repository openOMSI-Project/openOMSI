//! The interface over the picture in the window's frame: the notes, the navigator, the
//! plugins' panels, the menus and the rest of the HUD.

use super::*;

impl App {
    /// The interface over the picture. The VR navigator's display (for the headset's
    /// picture).
    pub(super) fn frame_ui(&mut self, dt: f32) -> Option<crate::vr_navigator::Display> {
        // (the game menu's lines, for the interface below)
        let menu_lines = if self.menus.game_menu.is_some() { self.game_menu_items() } else { Vec::new() };
        // (the mirror editor's keys and the panel under the cursor, while it is on)
        let mirror_help = match (self.player.as_ref(), self.mirror_hud_size()) {
            (Some(p), Some(size)) => self.gfx.mirror_hud.help_lines(p, self.hud_cursor(), size),
            _ => Vec::new(),
        };
        let vr_nav_display = self.vr_nav_display();
        let vr_active = self.vr_active();
        // the interface over the picture
        // (the pages of an open settings window)
        let menu_tabs = match self.menus.list_kind.as_ref() {
            Some(k) if self.menus.chooser.is_some() => crate::game_lists::page_titles(self, k),
            _ => None,
        };
        if !(self.world.is_some() && self.renderer.is_some() && self.scene.is_some()) {
            return vr_nav_display;
        }
        let (notes, tooltip) = self.frame_notes(dt, mirror_help);
        let __t = Instant::now();
        // (the frame's overlays start empty; the notes are the interface's, in
        // Roboto - OMSI's bitmap font HUD is the start menu's and the offscreen
        // pictures' only)
        if let Some(scene) = self.scene.as_mut() {
            scene.overlays.clear();
        }
        let hud = self
            .gfx.surface
            .as_ref()
            .map(|s| {
                self.settings
                    .hud_viewport((s.config.width, s.config.height))
            })
            .unwrap_or([0.0, 0.0, 1.0, 1.0]);
        // (not under the open game menu: the pause menu's rail covers the left
        // edge, and the navigator's panel stood out from under it)
        let nav_hidden = !vr_active && self.menus.game_menu.is_some();
        self.frame_navigator(dt, hud, vr_active, nav_hidden, vr_nav_display);
        // the Lua plugins' panels (`omsi.ui`): over the picture and the navigator,
        // under the game's own interface; not under its menus, nor in VR
        let plugin_focus = crate::plugin_ui::focused(&self.integrations.plugins);
        self.frame_plugin_panels(dt, hud, vr_active);
        self.frame_ui_draw(dt, hud, vr_active, plugin_focus, &notes, tooltip, menu_lines, menu_tabs);
        *self.perf.profile.entry("hud").or_default() += __t.elapsed().as_secs_f64();
        vr_nav_display
    }

    /// The notes over the picture (why the bus is not moving, the editors' keys, the trip's
    /// end, the messages, the LAN and the passengers) and the tooltip of the switch under
    /// the cursor.
    fn frame_notes(&mut self, dt: f32, mirror_help: Vec<String>) -> (Vec<String>, Option<String>) {
        // No block of text over the picture (the clock, the bus, the trip and the
        // key reminder were the old HUD in OMSI's bitmap font; the navigator shows
        // the trip, the launcher the keys): only what the driver has to act on,
        // in the interface font, top left.
        let mut lines: Vec<String> = Vec::new();
        // why the bus is not moving, whenever the throttle is pressed and nothing
        // happens: the things a driver checks first
        if let Some(p) = self.player.as_ref() {
            lines.extend(standing_reasons(&p.vehicle, &|a| crate::diagnostics::rebound_key(&p.bindings, a)));
        }
        // what is under the cursor, in the player's language (the scripts only
        // know internal, mostly German names)
        let names = describe::names(&self.args.root, &self.settings.language);
        // next to the cursor (`ui`), when the setting asks for it
        let tooltip = self.menus.hover.as_ref().map(|h| names.control(h));
        // the object editor's keys, while it is on (one quiet line)
        // the mirror editor's keys and the panel under the cursor, while it is on
        lines.extend(mirror_help);
        if self.menus.editor.is_some() {
            lines.push("Object editor: click picks · drag moves · wheel turns (Shift lifts) · Del · C copy · V variant · Backspace undo · Ctrl+S save · Esc".into());
        }
        if let Some(d) = self.session.duty.as_ref().filter(|d| d.trip_done()) {
            lines.push(match d.trips.get(d.trip_index + 1) {
                Some(next) => format!(
                    "End of the trip. Next: {} to {}, from {} at {} (it starts by itself a minute before)",
                    if next.line.trim().is_empty() { "service trip".to_string() } else { format!("line {}", next.line) },
                    next.terminus.strip_prefix(&format!("{} ", next.line)).unwrap_or(&next.terminus),
                    next.stops.first().map(|s| s.name.trim()).unwrap_or("?"),
                    crate::schedule::hhmm(next.departure)
                ),
                None => "End of the duty: the tour's last trip is done".into(),
            });
        }
        if let Some((msg, left)) = self.service_msg.as_mut() {
            *left -= dt;
            if *left > 0.0 {
                lines.push(msg.clone());
            }
        }
        self.service_msg = self.service_msg.take().filter(|(_, l)| *l > 0.0);
        self.integrations.update_watch.tick(&mut self.menus.notices);
        for n in self.menus.notices.iter_mut() {
            n.left -= dt;
        }
        self.menus.notices.retain(|n| n.left > 0.0);
        if let Some(lan) = self.net.lan.as_ref() {
            lines.extend(lan::hud_lines(lan, &self.net.remotes, self.player.as_ref()));
            lines.extend(self.sound.voice.as_ref().and_then(|v| v.hud_line()));
        }
        if let Some(h) = self.session.humans.as_ref() {
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
        // (the cursor no longer works the cab: how to have it back)
        if crate::plugin_ui::focused(&self.integrations.plugins) {
            lines.push(crate::plugin_ui::FOCUS_NOTE.into());
        }
        (lines, tooltip)
    }

    /// The navigator and OMSI 2's dynamic route arrows.
    fn frame_navigator(
        &mut self,
        dt: f32,
        hud: [f32; 4],
        vr_active: bool,
        nav_hidden: bool,
        vr_nav_display: Option<crate::vr_navigator::Display>,
    ) {
        let (Some(r), Some(scene)) = (self.renderer.as_ref(), self.scene.as_mut()) else { return };
        if let (Some(nav), Some(p), Some(_)) = (
            self.menus.navigator.as_mut(),
            self.player.as_ref(),
            self.gfx.surface.as_ref(),
        ) {
            let old_enabled = nav.enabled;
            let old_opacity = nav.opacity;
            nav.cockpit_display = vr_active;
            if vr_active {
                nav.enabled = vr_nav_display.is_some_and(|d| d.placement.enabled);
                nav.opacity = vr_nav_display.map(|d| d.placement.opacity).unwrap_or(0.95);
            }
            if nav_hidden {
                nav.enabled = false;
            }
            if let Some(w) = self.world.as_ref() {
                nav.start_map(w.clone());
            }
            // stops beyond the loaded tiles: their places from the navigator's map
            if let (Some(places), Some(d)) = (nav.places(), self.session.duty.as_mut()) {
                if !self.session.duty_places {
                    self.session.duty_places = true;
                    d.learn_places(places);
                }
            }
            let (line, terminus, stops, trip) = navigator::duty_parts(self.session.duty.as_ref());
            match (trip, self.session.schedule.as_ref(), self.session.traffic.as_ref(), self.world.as_ref()) {
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
            let (outside_temp, inside_temp) = vehicle_temperatures(p);
            // (on foot the map follows the walker, not the bus left standing)
            let (at, heading) = match self.session.on_foot.as_ref() {
                Some(f) => (f.pos, f.heading),
                None => (p.vehicle.position, p.vehicle.heading),
            };
            let frame = navigator::NavFrame {
                traffic: self.session.traffic.as_ref(),
                players: self.net.lan.as_ref().map(|l| crate::lan::nav_players(&self.net.remotes, l.my_id)).unwrap_or_default(),
                bus: at,
                heading,
                speed_kmh: p.vehicle.physics.velocity_kmh(),
                outside_temp,
                inside_temp,
                line,
                terminus,
                stops,
                delay: self.session.duty.as_ref().map(|_| p.vehicle.host.tt_delay as f64),
                passengers: self.session.humans.as_ref().map(|h| h.riding()),
                stop_requested: navigator::stop_requested(&p.vehicle),
                time: self.clock.time,
                weekday: self.clock.weekday(),
                language: &self.settings.language,
                screen: if vr_active {
                    (1440.0, 1440.0)
                } else {
                    (hud[2], hud[3])
                },
                ui_scale: if vr_active {
                    1.0
                } else {
                    self.settings.ui_scale
                },
                follow_window: if vr_active {
                    true
                } else {
                    self.settings.ui_scale_window
                },
                dt,
                info_rect: self.ui.as_ref().and_then(|u| u.info_rect).filter(|_| !vr_active),
            };
            let __tn = Instant::now();
            nav.frame_at(r, scene, &frame, hud[0]);
            nav.enabled = old_enabled;
            nav.opacity = old_opacity;
            *self.perf.profile.entry("hud.navigator").or_default() += __tn.elapsed().as_secs_f64();
            // OMSI 2's dynamic route arrows over the junctions ahead
            if nav.arrows {
                if let Some(w) = self.world.as_ref() {
                    let spots = nav.arrow_spots(self.session.traffic.as_ref().map(|t| &t.net), 350.0, &|id| w.object_positions.lock().get(&id).map(|p| (p.0, p.1[0])));
                    self.gfx.route_arrows.tick(dt, w, r, scene, &spots);
                }
            } else if self.gfx.route_arrows.any() {
                // (switched off in the menu: the ones standing go too)
                if let Some(w) = self.world.as_ref() {
                    self.gfx.route_arrows.clear(w, r, scene);
                }
            }
        }
    }

    /// The Lua plugins' panels (`omsi.ui`).
    fn frame_plugin_panels(&mut self, dt: f32, hud: [f32; 4], vr_active: bool) {
        let (Some(r), Some(scene)) = (self.renderer.as_ref(), self.scene.as_mut()) else { return };
        if let Some(plugin_ui) = self.integrations.plugins.as_ref().map(|p| p.ui.clone()) {
            let dpi = self
                .window
                .as_ref()
                .map_or(1.0, |w| w.scale_factor() as f32);
            let settings = &self.settings;
            let size = ui::size_factor(
                hud[3],
                dpi,
                settings.ui_scale,
                settings.ui_scale_window,
            );
            let map_open = self.menus.navigator.as_ref().is_some_and(|n| n.map_open());
            let frame = crate::plugin_ui::PanelsFrame {
                hud,
                scale: dpi * size,
                hidden: vr_active
                    || self.menus.game_menu.is_some()
                    || self.menus.chooser.is_some()
                    || map_open,
                cursor: self.input.cursor,
                dt,
                backdrop: ui::backdrop(settings.ui_opacity),
                navigator: self.menus.navigator.as_ref().and_then(|n| n.screen_rect()),
            };
            let mut state = plugin_ui.borrow_mut();
            self.integrations.plugin_panels.frame(r, scene, &mut state, &frame);
        }
    }

    /// The game's own interface: the menus, the chat, the name tags, the notes, the
    /// timetable, the info bar, the tutorial.
    #[allow(clippy::too_many_arguments)]
    fn frame_ui_draw(
        &mut self,
        dt: f32,
        hud: [f32; 4],
        vr_active: bool,
        plugin_focus: bool,
        notes: &[String],
        tooltip: Option<String>,
        menu_lines: Vec<(&'static str, &'static str)>,
        menu_tabs: Option<(Vec<String>, usize)>,
    ) {
        // (the steering cross: before the renderer and the scene are borrowed)
        let steer_cross = self.steer_cross_point();
        let (Some(r), Some(scene)) = (self.renderer.as_ref(), self.scene.as_mut()) else { return };
        if let (Some(ui), Some(s)) = (self.ui.as_mut(), self.gfx.surface.as_ref()) {
            let scale = self.window.as_ref().map(|w| w.scale_factor() as f32).unwrap_or(1.0);
            let (w, h) = (hud[2], hud[3]);
            self.net.remotes.chat.disabled = !self.settings.chat;
            let chat = (self.net.lan.is_some() && self.settings.chat).then(|| ui::ChatView {
                lines: &self.net.remotes.chat.lines,
                typing: self.net.remotes.chat.typing.as_deref(),
                error: self.net.remotes.chat.error(),
            });
            ui.chat.hidden = self.net.remotes.chat.hidden;
            let mut tags = if self.settings.name_tags {
                let voice = self.sound.voice.as_ref();
                let speaks = |name: &str, id: u32| voice.is_some_and(|v| v.speaks(name, id));
                let on_radio = |name: &str, id: u32| voice.is_some_and(|v| v.on_radio(name, id));
                let rig = (self.settings.triple.enabled
                    && !self.settings.vr_requested())
                .then(|| {
                    self.settings.triple.zoomed(
                        s.config.width,
                        s.config.height,
                        self.cam.view_zoom.get(&self.view).copied().unwrap_or(1.0),
                    )
                });
                self.camera
                    .as_ref()
                    .map(|c| {
                        lan::name_tags(
                            &self.net.remotes,
                            c,
                            s.config.width as f32,
                            h,
                            rig.as_ref(),
                            &speaks,
                            &on_radio,
                        )
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            for (pos, _, _, _) in &mut tags {
                pos.0 -= hud[0];
            }
            // the vehicle chooser shows its vehicles in the menu's place (the menu
            // scrolls a long list)
            // the name of the cab's switch under the cursor, unless the interface
            // covers the cab there (it read like a line of the menu over it)
            let (cx, cy) = self.input.cursor;
            let map_open = self.menus.navigator.as_ref().is_some_and(|n| n.map_open());
            let covered = self.menus.game_menu.is_some()
                || plugin_focus
                || self.xr.vr_nav_edit.is_some()
                || self.menus.chooser.is_some()
                || ui.chat.hovered
                || map_open
                || (!vr_active && self.menus.navigator.as_ref().is_some_and(|n| n.over_panel(cx, cy)));
            let dropdown = self.menus.dropdown.as_ref().filter(|_| self.menus.chooser.is_some()).map(|d| ui::DropdownView {
                row: d.row,
                items: d.items.iter().map(|x| x.0.as_str()).collect(),
                sel: d.sel,
                top: d.top,
                current: d.current,
            });
            let chooser_list = self.menus.admin_list.as_ref().unwrap_or(&self.menus.vehicle_list);
            let (chooser_items, chooser_sel): (Vec<(&str, &str)>, Option<usize>) = match self.menus.chooser {
                Some(sel) => {
                    let items = chooser_list.iter().map(|(name, path)| (path.as_str(), name.as_str())).collect();
                    (items, Some(sel))
                }
                None => (Vec::new(), None),
            };
            // (the game menu's greyed-out lines: the timetable needs an active route)
            let menu_disabled: &[&str] = &[];
            let (menu_kind, menu_head, menu_preview) = crate::game_lists::menu_extras(self.menus.list_kind.as_ref(), self.menus.admin_list.as_deref(), chooser_sel, self.session.schedule.as_ref(), self.clock.time);
            let frame = ui::Frame {
                scale,
                ui_scale: ui::size_factor(h, scale, self.settings.ui_scale, self.settings.ui_scale_window),
                opacity: ui::backdrop(self.settings.ui_opacity),
                width: w,
                height: h,
                cursor: (self.input.cursor.0 - hud[0], self.input.cursor.1),
                steer_cross: steer_cross.map(|(x, y)| (x - hud[0], y)),
                vr: {
                    #[cfg(windows)] { self.xr.vr.is_some() }
                    #[cfg(not(windows))] { false }
                },
                tooltip: tooltip.filter(|_| self.settings.tooltips && !self.input.dragging && !covered && self.menus.game_menu.is_none()),
                // (switched off: none, `Settings::notes`; nor over the city map,
                // whose header they covered once they stood on the timetable's line)
                notes: if self.settings.notes && !map_open && self.menus.game_menu.is_none() { notes } else { &[] },
                fps: self.settings.show_fps.then_some(self.perf.fps),
                paused: self.paused,
                menu: match chooser_sel {
                    Some(k) => Some((k, &chooser_items[..])),
                    None => self.menus.game_menu.map(|k| (k, &menu_lines[..])),
                },
                menu_disabled,
                menu_kind,
                menu_head,
                menu_preview,
                pane_first: self.menus.pane_scroll.filter(|p| Some(p.0) == chooser_sel).map(|p| p.1),
                menu_tabs,
                dropdown,
                menu_kbd: self.menus.menu_kbd,
                menu_top: self.menus.menu_top,
                // (not over the city map, which has the stops and their times: it
                // covered the map's zoom and close buttons)
                timetable: (self.menus.timetable && !map_open).then(|| timetable_rows(self.session.duty.as_ref(), self.player.as_ref().map(|p| p.vehicle.host.tt_delay as f64))).flatten(),
                info: self.menus.info_bar.then(|| info_line(&self.clock, self.player.as_ref(), self.session.duty.as_ref(), self.session.humans.as_ref().map(|h| h.riding()), self.session.career.metres)),
                info_room: self.input.touch.info_room.filter(|_| self.input.touch.enabled),
                tutorial: self.menus.tutorial.as_ref().filter(|t| !t.hidden && self.menus.game_menu.is_none()).and_then(|t| t.page().map(|p| (p.title.as_str(), p.text.as_str(), p.image.as_deref(), t.at, t.pages.len()))),
                chat,
                chat_size: self.settings.chat_size,
                tags,
                notices: &self.menus.notices,
                notice_anchor: self.menus.navigator.as_ref().and_then(|n| n.screen_rect()),
            };
            ui.draw_at(r, scene, &frame, dt, hud[0]);
        }
    }
}
