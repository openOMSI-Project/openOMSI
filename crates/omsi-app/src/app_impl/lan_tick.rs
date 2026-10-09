//! LAN play once a frame, and what the host's world and the other players' games ask of ours.

use super::*;

impl App {
    /// LAN play, once a frame: send our bus, take in the others', and keep a drawn
    /// vehicle for each of them.
    pub(crate) fn tick_lan(&mut self, dt: f32) {
        let walker = self.walker_pose();
        let radio_keyed = self.voice_radio_held();
        let Some(lan) = self.net.lan.as_mut() else {
            // (the session is over: the plugin is told so)
            self.sound.voice = None;
            return;
        };
        let duty = self
            .session.duty
            .as_ref()
            .map(|d| &d.trips[d.trip_index])
            .map(|t| (t.line.as_str(), t.terminus.as_str()));
        let frame = lan::Frame {
            audio: self.sound.audio.as_ref(),
            listener: self.camera.as_ref().map(|c| c.position),
            muffled: self.cam.in_cab || self.net.inside_remote.is_some(),
            riders: self.session.humans.as_ref().map(|h| h.riding()).unwrap_or(0),
            clock: Some(&self.clock),
            tour: self.session.duty.as_ref().map(|d| format!("{}/{}", d.line, d.tour)),
            walker,
            inside_of: self.net.inside_remote,
            radio_keyed,
        };
        let updates = lan::tick(
            lan,
            &mut self.net.remotes,
            dt,
            &self.args,
            self.player.as_mut(),
            self.world.as_deref(),
            self.renderer.as_ref(),
            self.scene.as_mut(),
            self.session.traffic.as_mut(),
            self.session.humans.as_mut(),
            &mut self.gfx.sim_view,
            duty,
            &frame,
        );
        for u in updates {
            self.apply_world_update(u);
        }
        let cmds = self.net.lan.as_mut().map(|l| l.take_commands()).unwrap_or_default();
        for (from, text) in cmds {
            self.lan_command(from, &text);
        }
        self.tick_voice(dt);
    }

    /// A command another player's game sent ours (`LanSession::command`).
    pub(crate) fn lan_command(&mut self, from: u32, text: &str) {
        let Some(lan) = self.net.lan.as_ref() else { return };
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
        // the voice server of the session (`voice`): asked of the host, told by it
        if text == "voice?" {
            if lan.role == omsi_net::Role::Host {
                let answer = crate::voice::VoiceServer::command(crate::voice::hosted().as_ref());
                if let Some(l) = self.net.lan.as_mut() {
                    l.command(from, &answer);
                }
            }
            return;
        }
        if text.starts_with("voice ") {
            if from == 1 {
                if let (Some(v), Some(server)) = (self.sound.voice.as_mut(), crate::voice::VoiceServer::parse_command(text)) {
                    v.set_server(server);
                }
            }
            return;
        }
        // a plugin's message to the same plugin here (`lan.send`)
        if self.plugin_lan_message(from, text) {
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
                if let Some(t) = self.session.traffic.as_mut() {
                    t.day_time = time;
                }
            }
            lan::WorldUpdate::Slew(s) => {
                self.clock.time = (self.clock.time + s).clamp(0.0, 86399.999);
                if let Some(t) = self.session.traffic.as_mut() {
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
                if let Some(s) = self.session.schedule.as_mut() {
                    s.set_lan_tours(tours);
                }
            }
        }
        if let Some(p) = self.player.as_mut() {
            p.vehicle.host.clock = self.clock.clone();
        }
    }
}
