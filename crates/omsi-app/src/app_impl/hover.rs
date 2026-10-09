//! What the cursor points at: the rays under it, the switch named, the cursor's shape.

use super::*;

impl App {
    pub(crate) fn hud_size(&self) -> (f32, f32) {
        let v = self
            .gfx.surface
            .as_ref()
            .map(|s| {
                self.settings
                    .hud_viewport((s.config.width, s.config.height))
            })
            .unwrap_or([0.0, 0.0, 1.0, 1.0]);
        (v[2], v[3])
    }

    pub(crate) fn hud_cursor(&self) -> (f32, f32) {
        let x = self
            .gfx.surface
            .as_ref()
            .map(|s| {
                self.settings
                    .hud_viewport((s.config.width, s.config.height))[0]
            })
            .unwrap_or(0.0);
        (self.input.cursor.0 - x, self.input.cursor.1)
    }

    pub(crate) fn cockpit_cursor_ray(&self, cam: &Camera, size: (u32, u32)) -> (glam::DVec3, glam::Vec3, f32) {
        #[cfg(windows)]
        if let Some(ray) = self.xr.vr.as_ref().and_then(|vr| vr.cursor_ray(self.input.cursor.0, self.input.cursor.1, size)) {
            return (ray.0, ray.1, ray.2 * 6.0);
        }
        if let Some(rig) = self.triple_rig(size) {
            let (o, d, spread) = rig.cursor_ray(cam, self.input.cursor, size);
            return (o, d, spread * 6.0);
        }
        let (o, d) = cursor_ray(cam, self.input.cursor.0, self.input.cursor.1, size.0 as f32, size.1 as f32);
        (o, d, pixel_angle(cam, size.1 as f32) * 6.0)
    }

    /// The triple screen's rig as drawn this frame (the view's zoom applied), while it is on
    /// and no headset is asked for.
    pub(crate) fn triple_rig(&self, size: (u32, u32)) -> Option<omsi_render::TripleScreen> {
        (self.settings.triple.enabled && !self.settings.vr_requested()).then(|| {
            self.settings.triple.zoomed(size.0, size.1, self.cam.view_zoom.get(&self.view).copied().unwrap_or(1.0))
        })
    }

    /// The desktop ray under the cursor for placing and editing on the ground: the window's
    /// own projection, or the triple screen panel's under the cursor.
    pub(crate) fn world_cursor_ray(&self, cam: &Camera, size: (u32, u32)) -> (glam::DVec3, glam::Vec3) {
        match self.triple_rig(size) {
            Some(rig) => {
                let (o, d, _) = rig.cursor_ray(cam, self.input.cursor, size);
                (o, d)
            }
            None => cursor_ray(cam, self.input.cursor.0, self.input.cursor.1, size.0 as f32, size.1 as f32),
        }
    }

    /// With a triple screen, the frustum around all three panels for "nothing appears or
    /// vanishes in sight": tangents of its half-angles, horizontal and vertical (None: the
    /// window's own view is the whole picture).
    pub(crate) fn sight_extent(&self, cam: &Camera, size: (u32, u32)) -> Option<(f64, f64)> {
        let rig = self.triple_rig(size)?;
        let (tx, ty) = rig.view_extent(cam, size.0, size.1);
        Some((tx as f64, ty as f64))
    }

    pub(crate) fn update_hover(&mut self) {
        if self.xr.vr_nav_edit.is_some() || self.input.cursor_hidden.is_some() {
            self.menus.hover = None;
            self.menus.hover_part = None;
            self.menus.hover_hand = false;
            return;
        }
        #[cfg(windows)]
        if !self.input.mouse_drive && self.xr.vr.as_ref().is_some_and(|vr| vr.needs_cursor_surface(
            self.input.cursor, self.menus.game_menu.is_some() || self.menus.chooser.is_some())) {
            let surface = self.player.as_ref()
                .zip(self.camera.as_ref())
                .zip(self.gfx.surface.as_ref())
                .filter(|_| matches!(self.view.as_str(), "driver" | "pax"))
                .map(|((player, camera), window)| {
                    let (origin, direction, _) = self.cockpit_cursor_ray(camera,
                                                                         (window.config.width, window.config.height));
                    (player.surface_hit(origin, direction),
                     (player.vehicle.position, player.vehicle.body_rotation()))
                });
            if let Some(vr) = self.xr.vr.as_mut() {
                vr.set_cursor_surface(surface.as_ref().and_then(|s| s.0),
                                      surface.map(|s| s.1));
            }
        }
        let outside = self.view == "outside";
        if let Some(p) = self.player.as_mut() {
            p.occlude_controls = outside;
        }
        let found = match (
            self.player.as_ref(),
            self.camera.as_ref(),
            self.gfx.surface.as_ref(),
        ) {
            (Some(p), Some(cam), Some(s)) if self.view != "free"
                && (self.view != "foot" || self.foot_reaches_bus())
                && !(self.vr_active() && self.input.mouse_drive
                && matches!(self.view.as_str(), "driver" | "pax")) => {
                let (o, d, spread) = self.cockpit_cursor_ray(cam, (s.config.width, s.config.height));
                p.hovered_part(o, d, spread)
            }
            // (in another player's bus nothing is offered: its switches are the driver's)
            _ => (None, false),
        };
        let (found, hand) = found;
        let (found, hand) = if let (Some(found), true) = (found.as_ref(), hand) {
            (Some(found.clone()), true)
        } else if let (Some(w), Some((o, d, spread))) = (self.world.as_ref(), self.cursor_ray_now()) {
            let blocked = self.player.as_ref().and_then(|p| p.opaque_body_hit(o, d));
            if let Some(hit) = w.scenery_object_hit(o, d, crate::input_script::SCENERY_OBJECT_REACH, spread).filter(|h| blocked.map_or(true, |t| t >= h.t)) {
                (Some((hit.event, true)), true)
            } else {
                (found, hand)
            }
        } else {
            (found, hand)
        };
        self.menus.hover_hand = hand;
        match found {
            Some((name, true)) => {
                self.menus.hover = Some(name);
                self.menus.hover_part = None;
            }
            Some((name, false)) => {
                self.menus.hover = None;
                self.menus.hover_part = Some(name);
            }
            None => {
                self.menus.hover = None;
                self.menus.hover_part = None;
            }
        }
        // the cursor itself says when it is over something that can be operated
        // (steering with the mouse: a cross, as OMSI shows it; turning the view with the
        // right button held: the four arrows OMSI shows then, #185)
        // (zooming with the mouse: the up-down arrows, Omsi's crSizeNS)
        // SIZENS only while the right button really zooms (with `alt_view`
        // it turns the view instead, and keeps the four arrows).
        let rmb_zoom = self.input.buttons_held.1
            && !self.input.mmb_held
            && !self.settings.alt_view
            && self.player.is_some()
            && self.input.both_drag.is_none()
            && matches!(self.view.as_str(), "driver" | "outside" | "pax" | "free");
        let kind: u8 = if self.input.both_drag.is_some() && self.menus.game_menu.is_none() {
            4
        } else if rmb_zoom && self.menus.game_menu.is_none() {
            4
        } else if self.input.mouse_look && self.menus.game_menu.is_none() {
            3
        } else if self.input.mouse_drive && self.mouse_steers_in_view() && self.menus.game_menu.is_none() {
            2
        } else if self.menus.game_menu.is_some() {
            // (the game menu's own cursor: not overwritten here, or it flips back and forth)
            self.menu_cursor_kind()
        } else if self.menus.hover.is_some() || self.menus.hover_hand {
            1
        } else {
            0
        };
        self.set_cursor_kind(kind);
    }

    /// Show the mouse cursor `kind` (0 arrow, 1 pointing hand, 2 cross, 3 arrows, 4 closed hand).
    pub(crate) fn set_cursor_kind(&mut self, kind: u8) {
        if kind != self.input.cursor_kind {
            self.input.cursor_kind = kind;
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

    /// The cursor over the open game menu: a pointing hand over what can be clicked (a line,
    /// a page, a control, the scroll bar), the closed hand while a slider or the scroll bar
    /// is held.
    pub(crate) fn menu_cursor_kind(&self) -> u8 {
        if self.menus.menu_drag.is_some() || self.menus.menu_scroll_drag || self.menus.dd_scroll_drag.is_some() || self.menus.pane_scroll_drag.is_some() {
            return 4;
        }
        let Some(u) = self.ui.as_ref() else { return 0 };
        let (x, y) = self.input.cursor;
        let inside = |r: &[f32; 4]| x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3];
        let clickable = u.menu_scroll_thumb.is_some_and(|r| inside(&r))
            || u.menu_side.iter().any(|r| inside(r))
            || u.menu_pane.iter().any(|r| inside(r))
            || u.menu_pane_go.as_ref().is_some_and(|r| inside(r))
            || u.menu_time.iter().any(|r| inside(r))
            || u.menu_ctl.iter().flatten().any(|r| inside(r))
            || u.menu_rects.iter().enumerate().any(|(i, r)| inside(r) && !self.menu_item_off(i + u.menu_start));
        if clickable {
            1
        } else {
            0
        }
    }
}
