//! The clock: its speed, the real-time sync, jumps, and the world following the date.

use super::*;

impl App {
    /// OMSI's weather dialog, the short way: the next weather of the Weather folder, in
    /// force at once (the roads keep their wetness until the rain changes it).
    /// How fast the clock runs: the session's in LAN play (the host's, which its time speed
    /// setting or its administration set), else the settings'.
    pub(crate) fn time_speed(&self) -> f64 {
        if self.real_time_locked() {
            return 1.0;
        }
        match self.net.lan.as_ref() {
            Some(l) => l.clock_speed,
            None => self.settings.time_speed.clamp(1.0, 30.0),
        }
    }

    /// The clock follows the real time and cannot be changed (the `time_sync` setting). In a
    /// LAN session as a client the host's clock counts: the host or the server syncs, not us.
    pub(crate) fn real_time_locked(&self) -> bool {
        self.settings.time_sync && !self.net.lan.as_ref().is_some_and(|l| l.role == omsi_net::Role::Client)
    }

    /// With the real-time sync on: hold the clock to this device's date and time (a second
    /// off at most; a bigger gap - the game was paused - is jumped, the traffic's clock with it).
    pub(crate) fn sync_real_time(&mut self) {
        if !self.real_time_locked() {
            return;
        }
        let Some(real) = crate::real_time::clock_now(&self.clock) else { return };
        let gap = crate::real_time::gap(&self.clock, &real);
        if gap.abs() < 0.25 {
            return;
        }
        self.clock.year = real.year;
        self.clock.day_of_year = real.day_of_year;
        self.clock.time = real.time;
        if let Some(tr) = self.session.traffic.as_mut() {
            tr.day_time += gap;
        }
        if let Some(p) = self.player.as_mut() {
            p.vehicle.host.clock = self.clock.clone();
        }
    }

    /// Move the clock by `secs` (the traffic's clock with it), as OMSI's time dialog does.
    pub(crate) fn shift_clock(&mut self, secs: f64) {
        if self.real_time_locked() {
            self.service_msg = Some(("The time cannot be changed while the real-time sync is on".into(), 3.0));
            return;
        }
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
        if let Some(tr) = self.session.traffic.as_mut() {
            tr.day_time += secs;
        }
        if let Some(p) = self.player.as_mut() {
            p.vehicle.host.clock = self.clock.clone();
        }
        let h = (t / 3600.0) as u32;
        self.service_msg = Some((format!("Clock: {h:02}:{:02}", ((t / 60.0) as u32) % 60), 3.0));
        self.session.clock_jump += secs;
        // (held Page Up/Down: once they are let go)
        if self.input.clock_hold == 0.0 {
            self.timetable_after_clock_jump();
        }
    }

    /// After the clock was set by more than two minutes: the timetable's buses put out again
    /// for the new time (`Schedule::restart`).
    pub(crate) fn timetable_after_clock_jump(&mut self) {
        let jump = std::mem::take(&mut self.session.clock_jump);
        if jump.abs() < crate::schedule::RESTART_JUMP {
            return;
        }
        if let (Some(s), Some(w), Some(t), Some(r), Some(scene)) = (self.session.schedule.as_mut(), self.world.as_ref(), self.session.traffic.as_mut(), self.renderer.as_ref(), self.scene.as_mut()) {
            let day_time = t.day_time;
            s.restart(w, t, &mut self.gfx.sim_view.traffic, r, scene, day_time);
        }
    }

    /// The world follows the sim date as OMSI's does at the day's change:
    /// the chrono scenarios in force (the tiles they change are read again) and the
    /// season's textures - also when the weather turns to snow or thaws (every loaded tile
    /// is read again with the other texture folder).
    pub(crate) fn follow_date(&mut self) {
        let Some(w) = self.world.clone() else { return };
        let date = self.clock.date_code();
        let snow = self.session.weather.as_ref().is_some_and(|x| x.snow);
        let on_road = self.session.weather.as_ref().is_some_and(|x| x.snow_on_road);
        let season = crate::world_load::season_folder_on(&self.args, &w.global, self.clock.day_of_year, snow, on_road).1;
        let Some((was_date, was_season)) = self.session.world_day.clone() else {
            self.session.world_day = Some((date, omsi_texture::season_folder()));
            return;
        };
        if was_date == date && was_season == season {
            return;
        }
        self.session.world_day = Some((date, season.clone()));
        let changed = if was_date != date { w.set_date(date) } else { Vec::new() };
        let (Some(st), Some(r), Some(scene)) = (self.gfx.streamer.as_mut(), self.renderer.as_ref(), self.scene.as_mut()) else { return };
        if was_season != season {
            log::info!("season: the textures of {:?} now (were {:?})", season, was_season);
            omsi_texture::set_season_folder(season);
            omsi_cfg::content_changed();
            st.reload(r, scene, None, self.sound.audio.as_ref());
        } else if !changed.is_empty() {
            st.reload(r, scene, Some(&changed), self.sound.audio.as_ref());
        }
    }
}
