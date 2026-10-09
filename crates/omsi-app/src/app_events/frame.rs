//! The window's frame (`WindowEvent::RedrawRequested`): its steps in order in `App::frame`,
//! each a method of its own in the files beside this one. What the offscreen run takes as
//! well is in `steps`.

use super::*;
use crate::view_sync::{self, ViewSync};

mod controls;
mod driving;
mod integrations;
mod interface;
mod people;
mod present;
mod road;
pub(crate) mod steps;
mod surroundings;
mod timing;
mod view_camera;
mod view_keys;

/// The frame's time: when it started, the time since the last one, and that time as the
/// simulation steps it (at most a tenth of a second).
pub(super) struct FrameTime {
    pub(super) now: Instant,
    pub(super) raw_dt: f32,
    pub(super) dt: f32,
}

impl App {
    /// One frame of the window, step by step.
    pub(super) fn frame(&mut self, event_loop: &ActiveEventLoop) {
        // the session's housekeeping, the graphics device, the frame's time (none: the
        // session ends)
        let Some(time) = self.frame_timing(event_loop) else { return };
        let dt = time.dt;
        // a vehicle chosen in the menu, read on a worker meanwhile, put down once it is ready
        self.poll_vehicle_placement();
        // the start menu, and the map's first area still loading
        if !self.frame_menus(event_loop, dt) {
            return;
        }
        let __t = Instant::now();
        self.drive_streaming();
        *self.perf.profile.entry("streaming").or_default() += __t.elapsed().as_secs_f64();
        self.frame_traffic(dt);
        // The player's vehicle moves before the passengers are placed: they sit in
        // the bus frame, and placing them on the pose of the frame before made everyone
        // aboard tremble at speed (a quarter of a metre behind the seat, every frame).
        let __t = Instant::now();
        let (analog, actions) = self.frame_controllers(dt);
        let analog = self.frame_mouse_drive(dt, analog);
        self.frame_pad_actions(analog, actions);
        self.frame_player(dt);
        self.frame_audio(dt);
        *self.perf.profile.entry("player").or_default() += __t.elapsed().as_secs_f64();
        self.frame_people(dt);
        self.frame_duty(dt);
        self.frame_integrations(event_loop, dt);
        self.frame_view_keys(dt);
        let daylight = self.frame_weather(dt);
        self.frame_lights(dt, daylight);
        self.frame_scripted(dt, daylight);
        if let (Some(w), Some(p)) = (self.world.as_deref(), self.player.as_mut()) {
            crate::wheel_surface::tell_scripts(w, &mut p.vehicle);
        }
        let vr_nav_display = self.frame_ui(dt);
        let lighting = self.frame_lighting(dt, daylight);
        self.frame_render(event_loop, &time, &lighting, vr_nav_display);
    }
}
