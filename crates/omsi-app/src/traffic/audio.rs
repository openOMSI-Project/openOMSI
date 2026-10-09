//! The AI vehicles' sounds.

use super::*;

impl Traffic {
    /// Play the `[sound_ai]` sets of the cars near `listener` (others are silenced).
    /// `street_cond` is the state of the road (see `VehicleHost::street_cond`): the stock
    /// AI sound configuration fades `WetLane_1`/`WetLane_2` in with it, which is what a car
    /// driving past through the wet sounds like. `muffled`: the listener (the player) sits in
    /// a cabin right now, so every AI car's sound is heard through that bodywork and glass -
    /// a passing car's horn does not simply sound like the street outside once the windows
    /// are shut. `riding`: the AI bus the player rides in on foot - heard from inside it.
    pub fn update_audio(
        &mut self,
        audio: &omsi_audio::AudioEngine,
        listener: DVec3,
        street_cond: f32,
        muffled: bool,
        riding: Option<u64>,
    ) {
        self.update_riding_audio(audio, riding);
        let freed = audio.trim_clips(std::time::Duration::from_secs(60));
        if freed > 0 && omsi_cfg::flags::OMSI_PROFILE.is_set() {
            log::info!(
                "sound clips: {:.1} MB nobody used for a minute let go",
                freed as f64 / 1e6
            );
        }
        for mut s in self.orphan_sounds.drain(..) {
            s.stop_all(audio);
        }
        let near = 250.0;
        for c in &mut self.sim.cars {
            let d = (c.vehicle.position - listener).length();
            // (the bus ridden in is heard from inside, by `update_riding_audio`)
            if d > near * 1.2 || riding == Some(c.id) {
                if let Some(mut s) = self.sounds.remove(&c.id) {
                    s.stop_all(audio);
                }
                continue;
            }
            if !self.sounds.contains_key(&c.id) && d < near {
                let def = &c.vehicle.ty.def;
                let Some(rel) = def.sound_ai.clone().or_else(|| def.sound.clone()) else {
                    continue;
                };
                let path = omsi_cfg::resolve_path(def.dir(), &rel);
                let cfg = self
                    .sound_cfgs
                    .entry(path.clone())
                    .or_insert_with(|| {
                        omsi_vehicle::SoundCfg::load(&path)
                            .map_err(|e| log::warn!("{e}"))
                            .ok()
                            .map(Arc::new)
                    })
                    .clone();
                if let Some(cfg) = cfg {
                    let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                    // an articulated bus's rear section sounds too (its engine, on a pusher)
                    let mut parts = Vec::new();
                    for (i, t) in c.vehicle.trailers.iter().enumerate() {
                        let def = &t.ty.def;
                        let Some(rel) = def.sound_ai.clone().or_else(|| def.sound.clone()) else {
                            continue;
                        };
                        let path = omsi_cfg::resolve_path(def.dir(), &rel);
                        let part = self
                            .sound_cfgs
                            .entry(path.clone())
                            .or_insert_with(|| {
                                omsi_vehicle::SoundCfg::load(&path)
                                    .map_err(|e| log::warn!("{e}"))
                                    .ok()
                                    .map(Arc::new)
                            })
                            .clone();
                        if let Some(part) = part {
                            let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                            parts.push((i, part, dir));
                        }
                    }
                    // the clips are read in the background the first time; silent till then
                    let ready = audio.clips_ready(&omsi_audio::SoundSet::clip_paths(&cfg, &dir))
                        && parts.iter().all(|(_, part, dir)| {
                            audio.clips_ready(&omsi_audio::SoundSet::clip_paths(part, dir))
                        });
                    if ready {
                        let number = c.vehicle.number();
                        let mut ss = omsi_audio::SoundSet::new_exterior(audio, &cfg.chosen_for(&number), &dir);
                        for (i, part, dir) in &parts {
                            ss.add_part(*i, omsi_audio::SoundSet::new_exterior(audio, &part.chosen_for(&number), dir));
                        }
                        ss.master = crate::sound_gain(&crate::SOUND_AI);
                        c.vehicle.host.snapshot_triggers = ss.curve_triggers().into_iter().collect();
                        self.sounds.insert(c.id, ss);
                    }
                }
            }
            let fired: Vec<String> = std::mem::take(&mut c.vehicle.host.fired_triggers);
            let fired_vars: Vec<(String, Vec<f32>)> = std::mem::take(&mut c.vehicle.host.fired_trigger_vars);
            let fired_files: Vec<(String, String)> =
                std::mem::take(&mut c.vehicle.host.fired_file_triggers);
            c.vehicle.host.street_cond = street_cond;
            c.vehicle.set_engine_var("StreetCond", street_cond);
            if let Some(ss) = self.sounds.get_mut(&c.id) {
                ss.set_muffled(muffled);
                let xf = c.vehicle.world_transform();
                let v = &c.vehicle;
                let at_fire = |t: &str, n: &str| -> Option<f32> {
                    let vals = &fired_vars.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(t))?.1;
                    v.var_slot(n).and_then(|i| vals.get(i).copied())
                };
                ss.update_fired(audio, &|n| v.var(n), &xf, &fired, &at_fire);
                ss.update_parts(
                    audio,
                    &|n| v.var(n),
                    &|i| v.trailers.get(i).map(|t| t.world_transform()),
                    &fired,
                );
                for (t, f) in &fired_files {
                    ss.play_file_trigger(audio, t, f, &|n| v.var(n), &xf);
                }
            }
        }
    }

    /// The AI bus ridden in on foot: its full `[sound]` set from inside ([viewpoint] 2, the
    /// outside sounds through what it has open - its own `Snd_OutsideVol`), instead of its
    /// `[sound_ai]` heard as from the street (#1286: a passenger heard next to nothing).
    fn update_riding_audio(&mut self, audio: &omsi_audio::AudioEngine, riding: Option<u64>) {
        if self.riding_sounds.as_ref().map(|r| r.0) != riding {
            if let Some((_, mut s)) = self.riding_sounds.take() {
                s.stop_all(audio);
            }
        }
        let Some(id) = riding else { return };
        let Some(c) = self.sim.cars.iter_mut().find(|c| c.id == id) else {
            if let Some((_, mut s)) = self.riding_sounds.take() {
                s.stop_all(audio);
            }
            return;
        };
        if let Some(mut s) = self.sounds.remove(&id) {
            s.stop_all(audio);
        }
        if self.riding_sounds.is_none() {
            let def = &c.vehicle.ty.def;
            let Some(rel) = def.sound.clone().or_else(|| def.sound_ai.clone()) else { return };
            let path = omsi_cfg::resolve_path(def.dir(), &rel);
            let Some(cfg) = self
                .sound_cfgs
                .entry(path.clone())
                .or_insert_with(|| omsi_vehicle::SoundCfg::load(&path).map_err(|e| log::warn!("{e}")).ok().map(Arc::new))
                .clone()
            else {
                return;
            };
            let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
            if !audio.clips_ready(&omsi_audio::SoundSet::clip_paths(&cfg, &dir)) {
                return;
            }
            let mut ss = omsi_audio::SoundSet::new(audio, &cfg.chosen_for(&c.vehicle.number()), &dir);
            ss.master = crate::sound_gain(&crate::SOUND_AI);
            self.riding_sounds = Some((id, ss));
        }
        if let Some((_, ss)) = self.riding_sounds.as_mut() {
            let fired = std::mem::take(&mut c.vehicle.host.fired_triggers);
            c.vehicle.host.fired_trigger_vars.clear();
            let fired_files = std::mem::take(&mut c.vehicle.host.fired_file_triggers);
            let v = &c.vehicle;
            ss.set_inside(true);
            ss.set_muffled(true);
            ss.set_listener_vehicle(true);
            // (how open this bus is to the street, for its own outside sounds and the traffic's)
            omsi_audio::soundset::set_outside_open(Some(v.var("Snd_OutsideVol").unwrap_or(0.0)));
            let xf = v.world_transform();
            ss.update(audio, &|n| v.var(n), &xf, &fired);
            for (t, f) in &fired_files {
                ss.play_file_trigger(audio, t, f, &|n| v.var(n), &xf);
            }
        }
    }
}
