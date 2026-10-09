//! Mouse steering with the cursor caught: the wheel and the pedals go by the mouse's own
//! movement, which goes on where the cursor would stop at the edge of the window or of the
//! screen - the wheel reaches its full lock at any speed. While the mouse steers the cursor
//! is hidden and held: locked where it stands (macOS, Wayland: the raw movement steers), or
//! kept in the window and put back in its middle before it reaches an edge (Windows, X11:
//! the cursor's own movement steers, with the system's pointer speed). It is let go, where
//! the mouse steers, whenever the cursor is wanted: looking round, a menu, the plugins'
//! panels, another window.

use super::*;

/// How the window holds the cursor while the mouse steers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GrabMode {
    /// The cursor stands still and hidden; the mouse's raw movement steers.
    Locked,
    /// The cursor is hidden, kept in the window and put back in its middle before an edge.
    Warp,
    /// The system lets the window neither hold nor move the cursor: it stays visible and
    /// free, and its movement steers.
    Plain,
}

/// The mouse steering's own point and the cursor's hold.
#[derive(Debug, Default)]
pub(crate) struct MouseGrab {
    /// Where the mouse steers, in the window's pixels: across the steering, up and down the
    /// pedals, as the cursor's place in the window - but it may lie past the window's left
    /// and right edges, as far as the full lock at the speed the bus goes.
    pub(crate) at: Option<(f32, f32)>,
    /// How the cursor is held (None: it is not).
    pub(crate) mode: Option<GrabMode>,
    /// The cursor's last place the system reported (its movement counts from there).
    last: Option<(f32, f32)>,
    /// The cursor was put back in the middle and the system's echo of that is awaited
    /// (an event from before the move still lies outside the middle and is not movement).
    warp_pending: bool,
    /// Reports outside the middle since the cursor was put back: a pointer that is not
    /// moved by putting it back (a graphics tablet's pen tells where it stands) never
    /// echoes, and every one of its reports was ignored - the wheel stood at full lock (#1945).
    stray: u8,
}

/// A cursor event while the mouse steers.
#[derive(Debug, PartialEq)]
pub(crate) enum CursorStep {
    /// Not the hand's movement (a locked cursor, a report from before it was put back).
    Ignore,
    /// Taken; `warp`: put the cursor back in the middle of the window now.
    Moved { warp: bool },
    /// The pointer stays where the hand holds it whatever the window does (a tablet): hold
    /// it no longer and steer by where it is.
    Absolute,
}

impl MouseGrab {
    /// The mouse moved by (dx, dy) window pixels; `from` is where it steers when it has no
    /// point yet (the cursor's place).
    pub(crate) fn add(&mut self, from: (f32, f32), dx: f32, dy: f32) {
        let (x, y) = self.at.unwrap_or(from);
        self.at = Some((x + dx, y + dy));
    }

    /// The system reports the cursor at (x, y) in a window of `size`.
    pub(crate) fn cursor_at(&mut self, x: f32, y: f32, from: (f32, f32), size: (f32, f32)) -> CursorStep {
        if self.mode == Some(GrabMode::Locked) {
            return CursorStep::Ignore;
        }
        let centre = (size.0 * 0.5, size.1 * 0.5);
        let outer = (x - centre.0).abs() > size.0 * 0.25 || (y - centre.1).abs() > size.1 * 0.25;
        let warping = self.mode == Some(GrabMode::Warp);
        if warping && self.warp_pending && outer {
            self.stray = self.stray.saturating_add(1);
            if self.stray > 4 {
                self.warp_pending = false;
                self.stray = 0;
                self.at = Some((x, y));
                self.last = Some((x, y));
                return CursorStep::Absolute;
            }
            return CursorStep::Ignore;
        }
        self.warp_pending = false;
        self.stray = 0;
        let last = self.last.unwrap_or(from);
        self.add(from, x - last.0, y - last.1);
        if warping && outer {
            self.last = Some(centre);
            self.warp_pending = true;
            CursorStep::Moved { warp: true }
        } else {
            self.last = Some((x, y));
            CursorStep::Moved { warp: false }
        }
    }

    /// The mouse stopped steering: the cursor's next report starts afresh.
    pub(crate) fn pause(&mut self) {
        self.last = None;
        self.warp_pending = false;
    }

    /// Held (`mode`) with the cursor put at `centre` (None: it could not be moved).
    pub(crate) fn caught(&mut self, mode: GrabMode, centre: Option<(f32, f32)>) {
        self.mode = Some(mode);
        if centre.is_some() {
            self.last = centre;
            self.warp_pending = mode == GrabMode::Warp;
        }
    }

    /// The steering point within the full lock - `reach` half widths of the window either
    /// side of its middle (`steer_reach`) - and the pedals' range, the window's height: what
    /// the mouse moves on past that turns nothing and is not to be given back first.
    pub(crate) fn clamp(&mut self, size: (f32, f32), reach: f32) {
        if let Some((x, y)) = self.at.as_mut() {
            let half = size.0 * 0.5;
            *x = x.clamp(half - reach * half, half + reach * half);
            *y = y.clamp(0.0, size.1);
        }
    }

    /// The place in the window that stands for the steering point (what is under the
    /// cursor, a switch dragged): the point, held at the window's edges.
    pub(crate) fn window_point(&self, size: (f32, f32)) -> Option<(f32, f32)> {
        self.at.map(|(x, y)| (x.clamp(0.0, (size.0 - 1.0).max(0.0)), y.clamp(0.0, (size.1 - 1.0).max(0.0))))
    }
}

/// How far from the middle of the window the mouse steers to the full lock, in half widths
/// of the window: one at walking pace, as Omsi.exe has the whole width for the lock (with
/// `mouse_sens` 1), more the faster the bus goes - OMSI divides the steering by the speed
/// in tens of km/h - and less never: the pedals and the cursor's place keep the window.
pub(crate) fn steer_reach(kmh: f32, sens: f32) -> f32 {
    ((kmh / 10.0).max(1.0) / sens.max(0.01)).max(1.0)
}

impl App {
    /// Whether the mouse steers now: mouse steering on, a view of the bus, and the cursor not
    /// wanted for anything else (looking round, a menu, the plugins' panels, the window in
    /// the background).
    pub(crate) fn mouse_steering_now(&self) -> bool {
        self.input.mouse_drive && self.mouse_steers_in_view() && !self.input.mouse_look && !self.input.input_away
            && !self.plugin_focus() && self.menus.game_menu.is_none()
    }

    /// Where the interface draws the steering cross while the cursor is hidden and held
    /// (the system's own crosshair cursor showed it before the cursor was held): the
    /// steering point, kept in the window. None while the cursor shows itself.
    pub(crate) fn steer_cross_point(&self) -> Option<(f32, f32)> {
        let hidden = matches!(self.input.mouse_grab.mode, Some(GrabMode::Locked | GrabMode::Warp));
        if !hidden || !self.mouse_steering_now() {
            return None;
        }
        self.input.mouse_grab.window_point(self.window_size()?)
    }

    fn window_size(&self) -> Option<(f32, f32)> {
        self.gfx.surface.as_ref().map(|s| (s.config.width as f32, s.config.height as f32))
    }

    /// Catch the cursor while the mouse steers in the window that has the focus, and let it
    /// go when it does not.
    pub(crate) fn sync_mouse_grab(&mut self) {
        // (the VR navigator's editor holds the cursor itself)
        if self.xr.vr_nav_edit.is_some() {
            self.input.mouse_grab.mode = None;
            self.input.mouse_grab.pause();
            return;
        }
        let steering = self.mouse_steering_now();
        if !steering {
            self.input.mouse_grab.pause();
        }
        #[cfg(windows)]
        let vr_on = self.xr.vr.is_some();
        #[cfg(not(windows))]
        let vr_on = false;
        // (not a test window in the background, OMSI_BACKGROUND: the cursor is whoever's
        // works at the screen)
        let want = steering && self.input.window_focused && !vr_on && !self.input.touch.enabled
            && !omsi_cfg::flags::OMSI_BACKGROUND.is_set()
            && !omsi_cfg::flags::OMSI_HIDDEN_WINDOW.is_set();
        // (the setting off: the cursor stays the system's crosshair, free and shown - it
        // shows the point without the frame's delay, #1948 - and its place steers)
        if want && !self.settings.mouse_hold {
            match self.input.mouse_grab.mode {
                Some(GrabMode::Plain) => {}
                Some(mode) => {
                    self.free_cursor(mode);
                    self.input.mouse_grab.mode = Some(GrabMode::Plain);
                }
                None => self.input.mouse_grab.mode = Some(GrabMode::Plain),
            }
            return;
        }
        match (self.input.mouse_grab.mode, want) {
            (None, true) => self.catch_cursor(),
            (Some(mode), false) => self.free_cursor(mode),
            _ => {}
        }
    }

    fn catch_cursor(&mut self) {
        let (Some(win), Some((w, h))) = (self.window.as_ref(), self.window_size()) else { return };
        let g = &mut self.input.mouse_grab;
        if g.at.is_none() {
            g.at = Some(self.input.cursor);
        }
        // (the cursor into the middle first: a locked cursor stands where it is, and a click
        // must not land outside the window; on macOS moving it unlocks it, so it is moved
        // before it is locked)
        let centre = (w * 0.5, h * 0.5);
        let warped = win.set_cursor_position(winit::dpi::PhysicalPosition::new(centre.0 as f64, centre.1 as f64)).is_ok();
        let mode = if win.set_cursor_grab(winit::window::CursorGrabMode::Locked).is_ok() {
            GrabMode::Locked
        } else if warped {
            let _ = win.set_cursor_grab(winit::window::CursorGrabMode::Confined);
            GrabMode::Warp
        } else {
            GrabMode::Plain
        };
        if mode != GrabMode::Plain {
            win.set_cursor_visible(false);
        }
        g.caught(mode, warped.then_some(centre));
        log::info!("mouse steering: cursor held ({mode:?})");
    }

    fn free_cursor(&mut self, mode: GrabMode) {
        self.input.mouse_grab.mode = None;
        self.input.mouse_grab.pause();
        let Some(win) = self.window.as_ref() else { return };
        let _ = win.set_cursor_grab(winit::window::CursorGrabMode::None);
        if mode == GrabMode::Plain {
            return;
        }
        win.set_cursor_visible(true);
        // the cursor shows where the mouse steered (held at the window's edges) - not when
        // the focus went to another window: the cursor is that one's
        if self.input.window_focused {
            let (x, y) = self.input.cursor;
            let _ = win.set_cursor_position(winit::dpi::PhysicalPosition::new(x as f64, y as f64));
        }
        log::info!("mouse steering: cursor let go");
    }

    /// The system reports the cursor at (x, y): while the mouse steers, its movement goes
    /// to the steering point. The place the rest of the game is to take as the cursor's, or
    /// None when the report is no movement of the hand.
    pub(crate) fn steer_cursor_event(&mut self, x: f32, y: f32) -> Option<(f32, f32)> {
        if !self.mouse_steering_now() {
            // (still held: let go at once - the report is the held cursor's)
            if let Some(mode) = self.input.mouse_grab.mode.filter(|_| self.xr.vr_nav_edit.is_none()) {
                self.free_cursor(mode);
                return None;
            }
            self.input.mouse_grab.pause();
            return Some((x, y));
        }
        // (a finger on the screen steers where it touches, as the cursor did)
        if self.input.touch.enabled {
            self.input.mouse_grab.at = None;
            return Some((x, y));
        }
        let Some(size) = self.window_size() else { return Some((x, y)) };
        let from = self.input.cursor;
        match self.input.mouse_grab.cursor_at(x, y, from, size) {
            CursorStep::Ignore => None,
            CursorStep::Absolute => {
                // (shown and free, and its place steers: the `Plain` hold)
                if let Some(win) = self.window.as_ref() {
                    let _ = win.set_cursor_grab(winit::window::CursorGrabMode::None);
                    win.set_cursor_visible(true);
                }
                self.input.mouse_grab.mode = Some(GrabMode::Plain);
                log::info!("mouse steering: the pointer does not follow the window (a tablet?); steering by where it is");
                self.steer_point_moved(size)
            }
            CursorStep::Moved { warp } => {
                if warp {
                    let moved = self.window.as_ref().is_some_and(|win| win.set_cursor_position(
                        winit::dpi::PhysicalPosition::new((size.0 * 0.5) as f64, (size.1 * 0.5) as f64)).is_ok());
                    if !moved {
                        self.input.mouse_grab.pause();
                    }
                }
                self.steer_point_moved(size)
            }
        }
    }

    /// The mouse moved by (dx, dy) window pixels with the cursor locked (or an `OMSI_INPUT`
    /// script's `rawmouse`).
    pub(crate) fn steer_by(&mut self, dx: f32, dy: f32) {
        if !self.mouse_steering_now() {
            return;
        }
        let Some(size) = self.window_size() else { return };
        let from = self.input.cursor;
        self.input.mouse_grab.add(from, dx, dy);
        if let Some((x, y)) = self.steer_point_moved(size) {
            self.cursor_moved_to(x, y);
        }
    }

    /// The steering point kept within the full lock; its place in the window.
    fn steer_point_moved(&mut self, size: (f32, f32)) -> Option<(f32, f32)> {
        let reach = steer_reach(self.input.mouse_kmh, self.settings.mouse_sens);
        self.input.mouse_grab.clamp(size, reach);
        self.input.mouse_grab.window_point(size)
    }
}

#[cfg(test)]
mod tests {
    use super::{steer_reach, CursorStep, GrabMode, MouseGrab};

    const SIZE: (f32, f32) = (1600.0, 900.0);

    #[test]
    fn locked_movement_goes_on_past_the_window_edge_to_the_full_lock() {
        let mut g = MouseGrab { mode: Some(GrabMode::Locked), ..Default::default() };
        // the locked cursor's own reports are no movement
        assert_eq!(g.cursor_at(10.0, 10.0, (800.0, 450.0), SIZE), CursorStep::Ignore);
        assert_eq!(g.at, None);
        // standing: the whole width is the lock, as with the cursor (same pixels per degree)
        g.add((800.0, 450.0), 400.0, 0.0);
        assert_eq!(g.at, Some((1200.0, 450.0)));
        // far past the right edge: held at the full lock, nothing to give back first
        g.add((0.0, 0.0), 5000.0, 0.0);
        g.clamp(SIZE, steer_reach(0.0, 1.0));
        assert_eq!(g.at, Some((1600.0, 450.0)));
        assert_eq!(g.window_point(SIZE), Some((1599.0, 450.0)));
        g.add((0.0, 0.0), -100.0, 0.0);
        assert_eq!(g.at, Some((1500.0, 450.0)));
        // and to the left lock the same
        g.add((0.0, 0.0), -9000.0, 0.0);
        g.clamp(SIZE, steer_reach(0.0, 1.0));
        assert_eq!(g.at.unwrap().0, 0.0);
    }

    /// A tablet's pen reports where it stands, wherever the window puts the cursor: after a
    /// few reports that never come back to the middle the steering follows the pen (#1945).
    #[test]
    fn a_pointer_that_is_not_put_back_steers_by_where_it_is() {
        let mut g = MouseGrab::default();
        g.caught(GrabMode::Warp, Some((800.0, 450.0)));
        // the pen far right: the cursor put back in the middle, and the pen is there again
        let reports: Vec<CursorStep> = (0..6).map(|_| g.cursor_at(1500.0, 450.0, (800.0, 450.0), SIZE)).collect();
        assert_eq!(reports.iter().filter(|r| **r == CursorStep::Ignore).count(), 4);
        assert_eq!(reports[4], CursorStep::Absolute);
        assert_eq!(g.at, Some((1500.0, 450.0)));
    }

    #[test]
    fn at_speed_the_lock_lies_past_the_window_and_is_reached() {
        let mut g = MouseGrab { mode: Some(GrabMode::Locked), at: Some((800.0, 450.0)), ..Default::default() };
        // 50 km/h: OMSI divides by 5, so the full lock is five half widths out
        let reach = steer_reach(50.0, 1.0);
        assert!((reach - 5.0).abs() < 1e-6);
        g.add((0.0, 0.0), 2.0 * 800.0 * 5.0, 0.0);
        g.clamp(SIZE, reach);
        let x = g.at.unwrap().0;
        assert_eq!(x, 800.0 + 5.0 * 800.0);
        assert!((crate::player::mouse_steering(x, SIZE.0, 50.0) - 1.0).abs() < 1e-6);
        // a lower sensitivity reaches further, a higher one keeps the window
        assert!((steer_reach(0.0, 0.5) - 2.0).abs() < 1e-6);
        assert_eq!(steer_reach(0.0, 3.0), 1.0);
    }

    #[test]
    fn pedals_reach_the_full_throttle_and_brake_past_the_top_and_bottom() {
        let mut g = MouseGrab { at: Some((800.0, 450.0)), ..Default::default() };
        g.add((0.0, 0.0), 0.0, -3000.0);
        g.clamp(SIZE, 1.0);
        assert_eq!(g.at, Some((800.0, 0.0)));
        g.add((0.0, 0.0), 0.0, 450.0);
        assert_eq!(g.at, Some((800.0, 450.0)));
        g.add((0.0, 0.0), 0.0, 3000.0);
        g.clamp(SIZE, 1.0);
        assert_eq!(g.at, Some((800.0, 900.0)));
    }

    #[test]
    fn a_kept_cursor_is_put_back_in_the_middle_and_its_way_counted() {
        let mut g = MouseGrab::default();
        g.caught(GrabMode::Warp, Some((800.0, 450.0)));
        g.at = Some((800.0, 450.0));
        // the system's echo of putting the cursor in the middle: no movement
        assert_eq!(g.cursor_at(800.0, 450.0, (0.0, 0.0), SIZE), CursorStep::Moved { warp: false });
        assert_eq!(g.at, Some((800.0, 450.0)));
        // the hand moves 300 px right, then on into the outer band: counted, put back
        assert_eq!(g.cursor_at(1100.0, 450.0, (0.0, 0.0), SIZE), CursorStep::Moved { warp: false });
        assert_eq!(g.cursor_at(1250.0, 450.0, (0.0, 0.0), SIZE), CursorStep::Moved { warp: true });
        assert_eq!(g.at, Some((1250.0, 450.0)));
        // a report from before the move back (still out there) is no movement
        assert_eq!(g.cursor_at(1260.0, 450.0, (0.0, 0.0), SIZE), CursorStep::Ignore);
        // the next from the middle counts from the middle: the way goes on past the edge
        assert_eq!(g.cursor_at(900.0, 450.0, (0.0, 0.0), SIZE), CursorStep::Moved { warp: false });
        assert_eq!(g.at, Some((1350.0, 450.0)));
        for _ in 0..10 {
            g.cursor_at(1250.0, 450.0, (0.0, 0.0), SIZE);
            g.cursor_at(800.0, 450.0, (0.0, 0.0), SIZE);
        }
        g.clamp(SIZE, 1.0);
        assert_eq!(g.at, Some((1600.0, 450.0)));
    }

    #[test]
    fn a_free_cursor_steers_by_its_way_from_where_it_was() {
        let mut g = MouseGrab::default();
        // the first report after the steering (re)starts counts from the cursor's place
        assert_eq!(g.cursor_at(820.0, 450.0, (800.0, 450.0), SIZE), CursorStep::Moved { warp: false });
        assert_eq!(g.at, Some((820.0, 450.0)));
        // no putting back without a hold, even at the edge
        assert_eq!(g.cursor_at(1599.0, 450.0, (0.0, 0.0), SIZE), CursorStep::Moved { warp: false });
        // (a script's raw movement on past the edge)
        g.add((0.0, 0.0), 300.0, 0.0);
        assert_eq!(g.at, Some((1899.0, 450.0)));
        // paused (looking round): the cursor goes elsewhere and is put back; the point stays
        // and the way counts from the cursor's place again
        g.pause();
        assert_eq!(g.cursor_at(1590.0, 450.0, (1599.0, 450.0), SIZE), CursorStep::Moved { warp: false });
        assert_eq!(g.at, Some((1890.0, 450.0)));
    }
}
