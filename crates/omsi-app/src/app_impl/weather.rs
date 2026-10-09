//! The weather: the Weather folder's, the custom one, the slow change, and METAR.

use super::*;

impl App {
    /// Change the weather in force by hand: `f` changes a copy of it, which takes the place of
    /// the weather at once (a change on its way and the weather cycle stop: this is the
    /// weather now). The sky's clouds are made again when their type changed.
    pub(crate) fn edit_weather(&mut self,f:impl FnOnce(&mut omsi_content::weather::Weather)){
        if self.net.lan.as_ref().is_some_and(|l|l.role==omsi_net::Role::Client){
            self.service_msg=Some(("In a LAN session the host sets the weather".into(),3.0));return;
        }
        if self.metar_locked(){self.service_msg=Some(("The weather cannot be changed while the METAR sync is on".into(),3.0));return;}
        let mut w=self.session.weather.clone().unwrap_or_default();
        if w.precip.len()<5{w.precip.resize(5,0.0);}
        f(&mut w);
        let brightness=crate::weather_setup::custom_weather(self.args.weather.as_deref()).map(|c|c.brightness).unwrap_or(1.0);
        let custom=crate::weather_setup::CustomWeather::from_weather(&w,brightness,self.session.wetness);
        self.set_custom_weather(custom);
    }

    pub(crate) fn set_custom_weather(&mut self,mut custom:crate::weather_setup::CustomWeather){
        if self.net.lan.as_ref().is_some_and(|l|l.role==omsi_net::Role::Client){
            self.service_msg=Some(("In a LAN session the host sets the weather".into(),3.0));return;
        }
        if self.metar_locked(){self.service_msg=Some(("The weather cannot be changed while the METAR sync is on".into(),3.0));return;}
        custom.normalize();
        // (the weather set by hand is the weather now: the physical model of the "natural"
        // weather stops, or its next step put its own back at once)
        crate::weather_model::stop();
        self.session.metar_rx=None;
        self.session.metar_once=false;
        let spec=custom.encode();
        let to=custom.to_weather();
        let clouds_changed=self.session.weather.as_ref().is_none_or(|w|w.clouds.0.trim()!=to.clouds.0.trim());
        self.args.weather=Some(spec.clone()); self.session.weather_blend=None; self.session.weather_cycle=None; self.session.wetness=custom.road_wetness;
        crate::scene::SNOW_WEATHER.store(to.snow,std::sync::atomic::Ordering::Relaxed);
        omsi_sim::host::set_ambient_weather(to.temp.0,to.temp.1);
        self.session.weather=Some(to);
        if clouds_changed{
            if let (Some(r),Some(scene))=(self.renderer.as_ref(),self.scene.as_mut()){
                crate::weather_setup::setup_sky(&self.args,r,scene,self.session.envir.as_ref(),self.session.weather.as_ref());
            }
        }
        self.follow_date();
        if let Some(l)=self.net.lan.as_mut().filter(|l|l.role==omsi_net::Role::Host){l.set_weather(&spec);}
        self.service_msg=Some(("Weather: Custom weather".into(),2.0));
    }

    pub(crate) fn next_weather(&mut self) {
        self.step_weather();
    }

    /// The next (`dir` 1) or previous (-1) weather of the Weather folder, round the ends.
    pub(crate) fn step_weather(&mut self) {
        if self.net.lan.as_ref().map(|l| l.role == omsi_net::Role::Client).unwrap_or(false) {
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
        if self.metar_locked() {
            self.service_msg = Some(("The weather cannot be changed while the METAR sync is on".into(), 3.0));
            return;
        }
        self.session.metar_rx = None;
        self.session.metar_once = false;
        let from = self.session.weather.clone().unwrap_or_default();
        self.args.weather = file.clone();
        let to = load_weather(&self.args);
        let name = to.name.clone();
        self.session.weather_blend = Some(crate::weather_cycle::Blend::new(from, to, secs));
        if share {
            // (a host: the others take it up with its next clock message)
            if let (Some(l), Some(f)) = (self.net.lan.as_mut(), file.as_ref()) {
                l.set_weather(f);
            }
        }
        log::info!("weather: going over to {file:?} ({name})");
        self.service_msg = Some((format!("Weather: {name}"), 4.0));
    }

    /// The weather this frame: a change coming in, and the cycle's next one (`secs` of the
    /// day went by; in LAN play only the host's cycle runs, the others follow it).
    pub(crate) fn tick_weather(&mut self, secs: f32) {
        if let Some(b) = self.session.weather_blend.as_mut() {
            let (w, clouds_changed, done) = b.step(secs);
            self.session.weather = Some(w);
            if done {
                self.session.weather_blend = None;
            }
            if clouds_changed {
                if let (Some(r), Some(scene)) = (self.renderer.as_ref(), self.scene.as_mut()) {
                    crate::weather_setup::setup_sky(&self.args, r, scene, self.session.envir.as_ref(), self.session.weather.as_ref());
                }
            }
        }
        // the physical model goes on with the clock (unless a change is coming in)
        if self.session.weather_blend.is_none() {
            if let Some(w) = crate::weather_model::refresh(&self.clock) {
                let kind_changed = self.session.weather.as_ref().is_none_or(|old| old.clouds.0 != w.clouds.0);
                self.session.weather = Some(w);
                if kind_changed {
                    if let (Some(r), Some(scene)) = (self.renderer.as_ref(), self.scene.as_mut()) {
                        crate::weather_setup::setup_sky(&self.args, r, scene, self.session.envir.as_ref(), self.session.weather.as_ref());
                    }
                }
            }
        }
        if let Some(w) = self.session.weather.as_ref() {
            crate::weather_setup::cloud_drift_step(&mut self.session.cloud_drift, w, secs as f64);
        }
        let follows = self.net.lan.as_ref().is_some_and(|l| l.role == omsi_net::Role::Client);
        if follows || self.session.weather_blend.is_some() {
            return;
        }
        if self.metar_locked() {
            return;
        }
        let Some(c) = self.session.weather_cycle.as_mut() else { return };
        c.next_in -= secs as f64;
        if c.next_in > 0.0 {
            return;
        }
        c.next_in = c.interval();
        let r = c.rand();
        let all = crate::weather_cycle::installed();
        let now = self.session.weather.clone().unwrap_or_default();
        let now_file = self.args.weather.clone().unwrap_or_default();
        if let Some(next) = crate::weather_cycle::pick(&all, &now, &now_file, self.clock.day_month().1, r) {
            self.change_weather(Some(next), true, 240.0);
        }
    }

    /// The weather follows the METAR report and cannot be changed (the `metar_sync` setting).
    /// In a LAN session as a client the host's weather counts: the host syncs, not us.
    pub(crate) fn metar_locked(&self) -> bool {
        self.settings.metar_sync && !crate::weather_tomorrow::selected(self.args.weather.as_deref()) && !self.net.lan.as_ref().is_some_and(|l| l.role == omsi_net::Role::Client)
    }

    /// The airport whose report the sync follows: the one chosen, else the one of the weather
    /// in force, else the one nearest the map.
    pub(crate) fn metar_station(&self) -> String {
        if !self.settings.metar_station.is_empty() {
            return self.settings.metar_station.to_ascii_uppercase();
        }
        match self.args.weather.as_deref().and_then(|w| w.strip_prefix("metar:")) {
            Some(code) if !code.trim().is_empty() => code.trim().to_ascii_uppercase(),
            _ => crate::launcher::drive::nearest_airport(&self.args.root.to_string_lossy(), &self.args.map),
        }
    }

    /// Fetch the selected station once, without turning the ten-minute METAR sync on.
    pub(crate) fn load_metar_once(&mut self) {
        if self.net.lan.as_ref().is_some_and(|l| l.role == omsi_net::Role::Client) {
            self.service_msg=Some(("In a LAN session the host sets the weather".into(),3.0));
            return;
        }
        let icao=self.metar_station();
        self.session.metar_rx=None;
        self.session.metar_once=true;
        let (tx,rx)=std::sync::mpsc::channel();
        self.session.metar_rx=Some(rx);
        std::thread::spawn(move||{let _=tx.send(crate::weather_setup::try_metar(&icao));});
        self.service_msg=Some((format!("Weather: loading METAR for {}",self.metar_station()),4.0));
    }

    /// Ask the continuous METAR sync to fetch its selected station immediately.
    pub(crate) fn refresh_metar_now(&mut self) {
        if !self.metar_locked() {
            self.load_metar_once();
            return;
        }
        self.session.metar_rx=None;
        self.session.metar_once=false;
        self.session.metar_next=0.0;
        self.service_msg=Some((format!("Weather: refreshing METAR for {}",self.metar_station()),4.0));
    }

    /// Freeze the weather currently in force into an editable custom state.
    pub(crate) fn current_weather_as_custom(&mut self) {
        if self.metar_locked() {
            self.service_msg=Some(("Turn METAR sync off before editing its current weather".into(),3.0));
            return;
        }
        let Some(w)=self.session.weather.as_ref() else{return};
        let brightness=crate::weather_setup::custom_weather(self.args.weather.as_deref()).map(|c|c.brightness).unwrap_or(1.0);
        let c=crate::weather_setup::CustomWeather::from_weather(w,brightness,self.session.wetness);
        self.set_custom_weather(c);
    }

    /// The METAR sync: with it on, the report is downloaded in the background (at once, then
    /// every ten minutes) and the weather goes over to it; `dt` is real seconds.
    pub(crate) fn tick_metar(&mut self, dt: f32) {
        if crate::weather_tomorrow::selected(self.args.weather.as_deref()) { return; }
        self.share_start_metar();
        if let Some(rx)=self.session.metar_rx.as_ref(){
            match rx.try_recv(){
                Ok(report)=>{
                    let once=self.session.metar_once;
                    self.session.metar_rx=None;
                    self.session.metar_once=false;
                    match report{
                        Some(w)=>self.apply_metar(w),
                        None=>self.service_msg=Some(("Weather: no METAR report could be loaded".into(),4.0)),
                    }
                    if once{return;}
                }
                Err(std::sync::mpsc::TryRecvError::Empty)=>return,
                Err(std::sync::mpsc::TryRecvError::Disconnected)=>{
                    let once=self.session.metar_once;
                    self.session.metar_rx=None;
                    self.session.metar_once=false;
                    if once{self.service_msg=Some(("Weather: METAR request failed".into(),4.0));return;}
                }
            }
        }
        if !self.metar_locked() {
            self.session.metar_next = 0.0;
            return;
        }
        self.session.metar_next -= dt as f64;
        if self.session.metar_next > 0.0 {
            return;
        }
        // (a failed download is tried again in a minute)
        self.session.metar_next = 60.0;
        let icao = self.metar_station();
        let (tx, rx) = std::sync::mpsc::channel();
        self.session.metar_rx = Some(rx);
        self.session.metar_once = false;
        std::thread::spawn(move || {
            let _ = tx.send(crate::weather_setup::try_metar(&icao));
        });
    }

    /// A host that started on `metar:<ICAO>` tells the players the report's values as soon as
    /// they are there (they cannot download it by the station's name: only the host syncs).
    fn share_start_metar(&mut self) {
        let Some(l) = self.net.lan.as_mut().filter(|l| l.role == omsi_net::Role::Host) else { return };
        if !l.weather().to_ascii_lowercase().starts_with("metar:") {
            return;
        }
        if let Some(wire) = self.session.weather.as_ref().and_then(crate::weather_setup::report_wire) {
            l.set_weather(&wire);
        }
    }

    /// Go over to the weather of a METAR report that came in.
    fn apply_metar(&mut self, to: omsi_content::weather::Weather) {
        self.session.metar_next = 600.0;
        let file = to.path.to_string_lossy().to_string();
        // (what the players are told: the report's values, which they make the weather from)
        let wire = crate::weather_setup::report_wire(&to).unwrap_or_else(|| file.clone());
        self.apply_report_weather(to, Some(file), &wire);
    }

    /// The common apply step for measured providers. `selection` changes the
    /// weather choice for METAR; Tomorrow.io keeps its provider mode selected.
    pub(crate) fn apply_report_weather(&mut self, to: omsi_content::weather::Weather, selection: Option<String>, wire: &str) {
        let name = to.name.clone();
        let from = self.session.weather.clone().unwrap_or_default();
        if let Some(file) = selection { self.args.weather = Some(file); }
        self.session.weather_cycle = None;
        self.session.weather_blend = Some(report_transition(from, to));
        if let Some(l) = self.net.lan.as_mut().filter(|l| l.role == omsi_net::Role::Host) {
            l.set_weather(wire);
        }
        log::info!("weather: applying report ({name})");
        self.service_msg = Some((format!("Weather: {name}"), 4.0));
    }
}

fn report_transition(from: omsi_content::weather::Weather, to: omsi_content::weather::Weather) -> crate::weather_cycle::Blend {
    // A natural model must not overwrite the provider after its blend finishes.
    crate::weather_model::stop();
    crate::scene::SNOW_WEATHER.store(to.snow, std::sync::atomic::Ordering::Relaxed);
    omsi_sim::host::set_ambient_weather(to.temp.0, to.temp.1);
    crate::weather_cycle::Blend::new(from, to, 60.0)
}

#[cfg(test)]
mod report_tests {
    #[test]
    fn report_updates_vehicle_initial_temperature_snow_and_transition() {
        use omsi_sim::host::VehicleHost;
        let clock = omsi_sim::SimClock::default();
        let previous = VehicleHost::new(clock.clone());
        let previous_snow = crate::scene::SNOW_WEATHER.load(std::sync::atomic::Ordering::Relaxed);
        let to = omsi_content::weather::Weather { temp: (-7.0, 2.1), snow: true, ..Default::default() };
        let mut blend = super::report_transition(Default::default(), to.clone());
        let host = VehicleHost::new(clock);
        assert_eq!(host.temperature, -7.0);
        assert_eq!(host.abs_humidity, 2.1);
        assert!(crate::scene::SNOW_WEATHER.load(std::sync::atomic::Ordering::Relaxed));
        let (weather, _, done) = blend.step(60.0);
        assert!(done);
        assert_eq!(weather.temp, to.temp);
        assert!(weather.snow);
        omsi_sim::host::set_ambient_weather(previous.temperature, previous.abs_humidity);
        crate::scene::SNOW_WEATHER.store(previous_snow, std::sync::atomic::Ordering::Relaxed);
    }
}
