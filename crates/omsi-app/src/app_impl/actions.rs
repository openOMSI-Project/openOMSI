//! OMSI's global actions (`[game]` of keyboard.cfg) and the toggles they work.

use super::*;

impl App {
    /// Shift+N: the navigator, the navigator with the schedule, off. True in VR (where the
    /// key is used up).
    pub(crate) fn cycle_navigator(&mut self) -> bool {
        if self.vr_active() {
            if !self.vr_nav_profile().enabled {
                self.vr_nav_adjust("enabled", 1.0);
            } else if self.menus.navigator.as_ref().is_some_and(|n| n.schedule) {
                if let Some(n) = self.menus.navigator.as_mut() { n.schedule = false; }
                self.vr_nav_adjust("enabled", 1.0);
            } else if let Some(n) = self.menus.navigator.as_mut() {
                n.schedule = true;
            }
            return true;
        }
        if let Some(n) = self.menus.navigator.as_mut() {
            match (n.enabled, n.schedule) {
                (true, false) => n.schedule = true,
                (true, true) => {
                    n.enabled = false;
                    n.schedule = false;
                }
                _ => n.enabled = true,
            }
        }
        false
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

    /// The duty gives up the stop it is due at and goes on with the one after it (the game
    /// menu's "Skip the next stop", Ctrl+Shift+H): the IBIS moves on with it, as it does
    /// when a bus page sets the next stop.
    pub(crate) fn skip_next_stop(&mut self) {
        let Some(d) = self.session.duty.as_mut() else { return };
        let Some(name) = d.skip_next() else {
            self.service_msg = Some(("The trip is over: no stop to skip".into(), 3.0));
            return;
        };
        log::info!("duty: stop '{name}' skipped, next stop {}", d.next_stop);
        if let Some(p) = self.player.as_mut() {
            let (trip, k) = d.trip_for_ibis();
            p.ibis_to_stop(trip, k);
        }
        self.service_msg = Some((format!("Stop skipped: {name}"), 3.0));
    }

    #[cfg(windows)]
    fn vr_action(&mut self, name: &str) -> bool {
        if name == "vr_toggle_mode" {
            self.xr.vr_zoom_active = false;
            if !self.settings.vr_requested() { return false; }
            if self.xr.vr.is_some() {
                self.xr.vr = None;
                self.service_msg = Some(("Desktop mode".into(), 2.0));
            } else if let Some(renderer) = self.renderer.as_ref() {
                match crate::openxr::Vr::new(renderer, self.settings.vr_scale,
                                             self.settings.vr_desktop_mirror) {
                    Ok(vr) => {
                        self.xr.vr = Some(vr);
                        self.service_msg = Some(("VR mode".into(), 2.0));
                    }
                    Err(e) => {
                        log::error!("OpenXR could not restart: {e:#}");
                        self.service_msg = Some((format!("{}: {e}", omsi_ui::tr("Could not start VR")), 5.0));
                    }
                }
            }
            self.menus.hover_key = None;
            if matches!(self.menus.list_kind, Some(crate::game_lists::ListKind::Options(_))) {
                self.refresh_list();
            }
            return true;
        }
        if self.xr.vr.is_none() { return false; }
        match name {
            "vr_recenter" => {
                self.xr.vr.as_mut().unwrap().recenter();
                self.cam.look = (0.0, 0.0);
                self.service_msg = Some(("VR view recentered".into(), 2.0));
            }
            "vr_toggle_desktop_mirror" => {
                let visible = self.xr.vr.as_mut().unwrap().toggle_desktop_mirror();
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
            // the menu Esc opens (a controller has no Esc): pressed again, it closes the menu
            // and whatever list it had open, as leaving them with Esc does
            "open_menu" => {
                if self.menus.game_menu.is_some() || self.menus.chooser.is_some() {
                    if self.menus.key_capture.is_some() {
                        self.cancel_key_capture();
                    }
                    self.menus.chooser = None;
                    self.menus.admin_list = None;
                    self.menus.list_kind = None;
                    self.menus.dropdown = None;
                    if self.menus.game_menu.is_some() {
                        self.close_game_menu();
                    }
                } else {
                    self.open_game_menu();
                }
            }
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
                self.cam.ego = true;
                self.service_msg = Some(("On foot: W A S D walk, Shift runs, right mouse button looks (F1 back to the bus)".into(), 5.0));
            }
            "view_set_driver" => self.view = "driver".into(),
            "view_set_passenger" => self.view = "pax".into(),
            "view_set_outside" => self.view = "outside".into(),
            // the cabin (the driver's or the passenger's) and the outside, one press apart:
            // what a single button on a controller wants. `view_toggle_viewpoint` is the
            // four-mode cycle, with the map in it, and stays where it is.
            "view_toggle_interior" => {
                if !self.cam.ego {
                    self.view = if self.view == "outside" { "driver".into() } else { "outside".into() };
                }
            }
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
                self.cam.ego = false;
            }
            // the timetable and the ticket desk each have a camera of their own in the bus
            // (`[view_schedule]`, `[view_ticketselling]`): the key switches the driver's view
            // to it and back
            "view_set_schedule" | "view_set_ticketselling" => {
                let schedule = name == "view_set_schedule";
                if schedule {
                    self.menus.timetable = !self.menus.timetable;
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
            "view_toggle_informationdisplay" => self.set_info_bar(!self.menus.info_bar),
            // (Omsi.exe's camera reset, 0x7edde4, puts back the field of view with the
            // direction: the zoom goes as well, #244)
            "view_reset_direction" => {
                // F1 eases home (look + zoom glide) from the values in place:
                // zeroing them first would flash a frame of the destination.
                if self.view == "driver"
                    && self.settings.driverview_smooth
                    && (self.cam.look != (0.0, 0.0) || self.cam.view_zoom.contains_key(&self.view))
                {
                    let zoom = self.cam.view_zoom.get(&self.view).copied().unwrap_or(1.0);
                    let key = self.look_key();
                    self.cam.f1_reset = Some((self.cam.look, zoom, 0.0, key));
                } else {
                    self.cam.f1_reset = None;
                    self.cam.look = (0.0, 0.0);
                    self.cam.view_zoom.remove(&self.view);
                }
                #[cfg(windows)]
                if let Some(vr) = self.xr.vr.as_mut() { vr.recenter(); }
            }
            // (Space in Inputs/keyboard.cfg: every view looks ahead again, and back to the
            // standard camera - "center")
            "view_reset_all_directions" => {
                // F1 eases home (look + zoom glide) from the values in place:
                // zeroing them first would flash a frame of the destination.
                // Everything else snaps. The glide belongs to the standard
                // camera (cam reset first), so a mid-glide switch finalizes it.
                let zoom = self.cam.view_zoom.get(&self.view).copied().unwrap_or(1.0);
                let eyed = self.view == "driver"
                    && self.settings.driverview_smooth
                    && (self.cam.look != (0.0, 0.0) || self.cam.view_zoom.contains_key(&self.view));
                if let Some(p) = self.player.as_mut() {
                    p.cam_choice = (0, 0);
                }
                self.cam.orbit = ORBIT_DEFAULT;
                if eyed {
                    self.cam.view_looks.clear();
                    self.cam.view_zoom.retain(|k, _| k == "driver");
                    let key = self.look_key();
                    self.cam.f1_reset = Some((self.cam.look, zoom, 0.0, key));
                    // the bookkeeping follows the camera change at once: left
                    // stale, the next swap would write the old look straight
                    // back into the previous camera's slot.
                    self.cam.look_view = self.look_key();
                } else {
                    self.cam.f1_reset = None;
                    self.cam.look = (0.0, 0.0);
                    self.cam.view_looks.clear();
                    self.cam.view_zoom.clear();
                }
            }
            // the next (or the previous) view mode, driver - passenger - outside - map and
            // round again; nothing on foot (Omsi.exe 0x706278 @0x70634a: (mode + 1) and 3,
            // @0x706392 the inverse)
            "view_toggle_viewpoint" | "view_toggle_viewpoint_inverse" => {
                if self.cam.ego {
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
            // a manual gearbox (Ctrl+Up / Ctrl+Down unless moved; a controller's button)
            "gear_up" | "gear_down" => {
                self.shift_gear(name == "gear_up");
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
                self.set_mouse_drive(!self.input.mouse_drive);
                let msg = if self.input.mouse_drive { "Mouse steering on: across steers, up is the throttle, down the brake (O turns it off)" } else { "Mouse steering off" };
                self.service_msg = Some((msg.into(), 4.0));
            }
            "toggel_ctrler" => {
                if let Some(c) = self.input.controllers.as_mut() {
                    c.enabled = !c.enabled;
                    let msg = if !c.any() { "No game controller found" } else if c.enabled { "Game controller on" } else { "Game controller off" };
                    self.service_msg = Some((msg.into(), 3.0));
                }
            }
            _ => return false,
        }
        true
    }

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
            let Some(cur) = p.gate_gear() else { return false };
            let to = if up { cur + 1 } else { cur - 1 };
            if !p.shift_gate_to(to) {
                return false;
            }
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
        self.input.mouse_drive = on;
        if on {
            // O can be pressed while the pointer is anywhere in the window. Start mouse
            // steering from the neutral cursor position instead of applying that offset
            // to the wheel on the first frame.
            self.input.center_cursor = true;
        }
        // (the mouse steers from the middle afresh; switched off, the cursor is let go)
        self.input.mouse_grab.at = None;
        self.sync_mouse_grab();
        if !on {
            crate::player::keep_wheel(self.player.as_mut());
            // the brake the mouse held stays on, as the brake key leaves it (OMSI has one
            // brake for both): the bus rolled off when the mouse let go of it (#517, #760)
            if let Some(p) = self.player.as_mut() {
                p.axes.brake = p.axes.brake.max(self.input.mouse_pedals.1);
            }
            #[cfg(windows)]
            self.reset_vr_pointer();
        }
        self.input.mouse_steer = (self.player.as_ref().map(|p| p.vehicle.physics.controls.steering).unwrap_or(0.0), 1.0);
        self.input.mouse_pedals = self.player.as_ref().map(|p| (p.vehicle.physics.controls.throttle, p.vehicle.physics.controls.brake)).unwrap_or((0.0, 0.0));
        if self.settings.mouse_steering != on {
            self.settings.mouse_steering = on;
            crate::game_lists::remember_setting("mouse_steering", if on { "1" } else { "0" });
        }
    }

    /// The information bar on or off, and kept so for the next session (#1164).
    pub(crate) fn set_info_bar(&mut self, on: bool) {
        self.menus.info_bar = on;
        if self.settings.info_bar != on {
            self.settings.info_bar = on;
            crate::game_lists::remember_setting("info_bar", if on { "1" } else { "0" });
        }
    }

    pub(crate) fn toggle_pause(&mut self) {
        // (a LAN session goes on for the others: it cannot be paused)
        if self.net.lan.is_some() {
            self.service_msg = Some(("A LAN session cannot be paused".into(), 3.0));
            return;
        }
        self.paused = !self.paused;
        if self.menus.game_menu.is_some() {
            // Keep the state a menu close should restore in step with P.
            self.menus.menu_prev_pause = self.paused;
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
        self.perf.shot = Some((path, false));
    }
}

/// The variable a gear lever's gate triggers keep the gear in: `antrieb_getr_aktugang` (the
/// LiAZ, whose gates only move the lever and leave the gear to its frame), else
/// `antrieb_getr_gang` (the stock cars' antrieb.osc), else one named for the gear that both
/// `kw_s_1` and `kw_s_2` set.
pub(crate) fn gate_gear_var(program: &omsi_script::Program) -> Option<String> {
    for known in ["antrieb_getr_aktugang", "antrieb_getr_gang"] {
        if program.var(known).is_some() {
            return Some(known.to_string());
        }
    }
    let mut names: Vec<String> = program
        .var_names()
        .into_iter()
        .filter(|n| n.contains("gang") || n.contains("gear"))
        .filter(|n| {
            let by = program.triggers_setting(n);
            by.iter().any(|t| t == "kw_s_1") && by.iter().any(|t| t == "kw_s_2")
        })
        .collect();
    names.sort_by_key(|n| (n.len(), n.clone()));
    names.into_iter().next()
}

#[cfg(test)]
mod gear_lever_tests {
    /// The stock cars' gates keep the gear in `antrieb_getr_gang` (#866).
    #[test]
    fn the_gear_is_read_where_the_gates_store_it() {
        let program = |vars: &str, osc: &str| {
            let dir = std::env::temp_dir().join(format!("omsi_gates_{}_{}", std::process::id(), vars.len()));
            std::fs::create_dir_all(&dir).unwrap();
            let (vl, sc) = (dir.join("varlist.txt"), dir.join("antrieb.osc"));
            std::fs::write(&vl, vars).unwrap();
            std::fs::write(&sc, osc).unwrap();
            let p = omsi_script::compile(&omsi_script::CompileInput { varlists: vec![vl], scripts: vec![sc], ..Default::default() });
            let _ = std::fs::remove_dir_all(&dir);
            p
        };
        let stock = program("antrieb_getr_gang\n", "{trigger:kw_s_1_fest}\n{trigger:kw_s_1}\n1 (S.L.antrieb_getr_gang)\n{end}\n{end}\n{trigger:kw_s_2}\n2 (S.L.antrieb_getr_gang)\n{end}\n");
        assert_eq!(super::gate_gear_var(&stock).as_deref(), Some("antrieb_getr_gang"));
        // a lever moved by every gate is not the gear
        let own = program("lever_moved\nmy_gear\n", "{trigger:kw_s_1}\n1 (S.L.lever_moved)\n1 (S.L.my_gear)\n{end}\n{trigger:kw_s_2}\n1 (S.L.lever_moved)\n2 (S.L.my_gear)\n{end}\n");
        assert_eq!(super::gate_gear_var(&own).as_deref(), Some("my_gear"));
    }
}
