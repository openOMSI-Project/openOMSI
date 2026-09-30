//! What the launcher knows and has chosen, and the work it has running in the background
//! (reading the content lists and timetables, polling the running games and installs).
//!
//! Everything slow runs on a thread of its own and comes back as a [`Msg`]; the window
//! drains them at the start of each frame, so it never waits.

use omsi_launcher_lib as core;
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;

/// Results of background work.
pub enum Msg {
    Content(Result<(Vec<core::MapInfo>, Vec<core::VehicleInfo>, Vec<core::WeatherInfo>), String>),
    Lines { map: String, date: String, lines: Result<Vec<core::LineInfo>, String> },
    Poll(Result<core::Poll, String>),
    Profile(Result<core::Profile, String>),
    Profiles(Vec<String>),
    Ibis { key: String, info: Result<core::IbisInfo, String> },
    Args(Result<Vec<String>, String>),
    Launched(Result<core::Launched, String>),
    Stopped { pid: u32, result: Result<bool, String> },
    LogTail { pid: u32, lines: Vec<String> },
    Mods(Result<core::ModsStatus, String>),
    ModInfo(Result<core::install::SourceInfo, String>),
    Installed(Result<core::install::Progress, String>),
    Join(serde_json::Value),
    Server { address: String, info: Result<omsi_net::ws::ServerInfo, String> },
    /// A background job stopped on an error of its own (a panic): whatever it was loading
    /// is not coming.
    Crashed(String),
}

/// A server in the Multiplayer page's list (`~/.openomsi/servers.json`), as the player
/// added it: its address (`https://….trycloudflare.com`, `http://host:port`) and a name of
/// their own (empty: the server's).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ServerEntry {
    pub name: String,
    pub address: String,
}

/// A code host's status page (its gateway is the session's port + 10).
fn host_status(code: &str) -> Result<omsi_net::ws::ServerInfo, String> {
    let c = omsi_net::SessionCode::decode(code)?;
    for a in c.addrs().into_iter().take(3) {
        if let Ok(i) = omsi_net::ws::query(&format!("http://{}:{}", a.ip(), a.port().saturating_add(10)), false) {
            return Ok(i);
        }
    }
    match omsi_net::bridge::lookup_tunnel(c.session) {
        Some(url) => omsi_net::ws::query(&url, false),
        None => Err("the host did not answer".into()),
    }
}

/// The list as saved, with the official server first when it is not in it.
fn with_official(mut list: Vec<ServerEntry>) -> Vec<ServerEntry> {
    if !list.iter().any(|s| omsi_net::official::is_alias(&s.address)) {
        list.insert(0, ServerEntry { name: omsi_net::official::NAME.into(), address: omsi_net::official::ALIAS.into() });
    }
    list
}

fn servers_path() -> std::path::PathBuf {
    core::data_dir().join("servers.json")
}

/// The duty as it is remembered between launches (`~/.openomsi/launcher-duty.json`).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Choice {
    pub bus: String,
    pub paint: String,
    /// The number plate the player typed for the bus (empty: as the content says).
    pub plate: String,
    pub hof: String,
    /// The depot file was chosen by hand (else it follows the map and the date).
    pub hof_manual: bool,
    pub map: String,
    /// The start: the entry point's place in the map's list, or -1 = automatic (nearest to
    /// the duty's first stop by road).
    pub entry: i32,
    pub line: Option<String>,
    pub tour: Option<String>,
    pub free: bool,
    /// Minutes of the day.
    pub time: i32,
    pub date: String,
    /// "auto", spring, summer, autumn, winter.
    pub season: String,
    pub weather: String,
    pub traffic: f32,
    pub passengers: bool,
    pub schedule: bool,
    pub autostart: bool,
    /// Start on foot (no bus of one's own until one is placed).
    pub on_foot: bool,
    /// off, host, join
    pub lan_mode: String,
    pub lan_addr: String,
    /// 2: `entry` may be -1 (automatic); older files had a fixed entry point there.
    pub version: u32,
}

impl Default for Choice {
    fn default() -> Self {
        Choice {
            bus: String::new(),
            paint: String::new(),
            plate: String::new(),
            hof: String::new(),
            hof_manual: false,
            map: String::new(),
            entry: -1,
            line: None,
            tour: None,
            free: false,
            time: 9 * 60,
            date: "1989-05-30".into(),
            season: "auto".into(),
            weather: String::new(),
            traffic: 30.0,
            passengers: true,
            schedule: true,
            autostart: false,
            on_foot: false,
            lan_mode: "off".into(),
            lan_addr: String::new(),
            version: 2,
        }
    }
}

fn choice_path() -> std::path::PathBuf {
    core::data_dir().join("launcher-duty.json")
}

impl Choice {
    pub fn load() -> Choice {
        let mut c: Choice = std::fs::read_to_string(choice_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        if c.version < 2 {
            // the start was always the map's first entry point: now it is automatic
            c.entry = -1;
            c.version = 2;
        }
        // (older launchers took a vehicle line of a broken ailists.cfg for the map's depot)
        if c.hof.to_ascii_lowercase().contains(".bus") || c.hof.to_ascii_lowercase().contains(".ovh") {
            c.hof.clear();
        }
        c
    }
    pub fn save(&self) {
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(choice_path(), t);
        }
    }
}

pub struct State {
    pub config: core::Config,
    pub maps: Vec<core::MapInfo>,
    pub vehicles: Vec<core::VehicleInfo>,
    pub weathers: Vec<core::WeatherInfo>,
    pub lines: Vec<core::LineInfo>,
    pub lines_for: (String, String),
    pub loading_content: bool,
    pub loading_lines: bool,
    pub choice: Choice,
    pub choice_dirty: f32,
    /// Map, whether it has a `laststn.osn`, when that was looked up.
    pub last_sit: Option<(String, bool, std::time::Instant)>,
    pub profiles: Vec<String>,
    pub profile: Option<core::Profile>,
    pub settings: serde_json::Value,
    pub settings_dirty: f32,
    /// `settings.cfg` as last read or written here: a game changes it too (its Options
    /// in the pause menu), and the launcher's copy from before must not be written back
    /// over that.
    settings_file: Option<String>,
    pub keybindings: serde_json::Value,
    pub keybindings_error: String,
    pub instances: Vec<core::Instance>,
    /// A game started from here ended on an error: what it said, and the end of its log
    /// (see `crash_of`), for the dialog that asks to report it.
    pub crash: Option<(String, String)>,
    pub jobs: Vec<core::install::Progress>,
    pub mods: Option<core::ModsStatus>,
    pub mods_asked: bool,
    pub mod_info: Option<Result<core::install::SourceInfo, String>>,
    pub mod_path: String,
    pub mod_mode: usize,
    pub ibis: Option<(String, Result<core::IbisInfo, String>)>,
    pub cmdline: String,
    pub join: (bool, String),
    pub join_checked: String,
    pub logs: std::collections::HashMap<u32, Vec<String>>,
    pub open_logs: std::collections::HashSet<u32>,
    pub stopping: std::collections::HashSet<u32>,
    /// Content that appeared while the launcher was open (marked NEW for a while).
    pub fresh: std::collections::HashMap<String, Instant>,
    pub status: (String, bool, Instant),
    stamp: Option<String>,
    poll_t: f32,
    polling: bool,
    pub second_armed: Option<Instant>,
    /// Multiplayer: the saved servers, what each said last (and when it was asked), and the
    /// one the Drive page is joined to now (its address).
    pub servers: Vec<ServerEntry>,
    pub server_info: std::collections::HashMap<String, (Instant, Result<omsi_net::ws::ServerInfo, String>)>,
    pub server_asked: std::collections::HashMap<String, Instant>,
    pub joined_server: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl State {
    pub fn new() -> State {
        let (tx, rx) = channel();
        let config = core::load_config();
        let settings = core::get_settings().unwrap_or_else(|_| core::settings_from_text(None));
        crate::ui_language(settings.get("language").and_then(|x| x.as_str()).unwrap_or("ENG"));
        crate::mt::enable(settings.get("machine_translation").and_then(|x| x.as_bool()).unwrap_or(false));
        let keybindings = core::get_keybindings().unwrap_or(serde_json::Value::Null);
        let choice = Choice::load();
        let mut s = State {
            config,
            maps: Vec::new(),
            vehicles: Vec::new(),
            weathers: Vec::new(),
            lines: Vec::new(),
            lines_for: (String::new(), String::new()),
            loading_content: false,
            loading_lines: false,
            choice,
            choice_dirty: 0.0,
            last_sit: None,
            profiles: Vec::new(),
            profile: None,
            settings,
            settings_dirty: 0.0,
            settings_file: read_settings_file(),
            keybindings,
            keybindings_error: String::new(),
            instances: Vec::new(),
            crash: None,
            jobs: Vec::new(),
            mods: None,
            mods_asked: false,
            mod_info: None,
            mod_path: String::new(),
            mod_mode: 0,
            ibis: None,
            cmdline: String::new(),
            join: (true, String::new()),
            join_checked: "\u{0}".into(),
            logs: Default::default(),
            open_logs: Default::default(),
            stopping: Default::default(),
            fresh: Default::default(),
            status: (String::new(), false, Instant::now()),
            stamp: None,
            poll_t: 0.0,
            polling: false,
            second_armed: None,
            servers: with_official(std::fs::read(servers_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()),
            server_info: Default::default(),
            server_asked: Default::default(),
            joined_server: None,
            tx,
            rx,
        };
        s.load_content();
        s.load_profiles();
        s.poll_now();
        // (a phone runs the game in the launcher's process: a crash took both, and the
        // launcher learns of it from the previous run's log)
        #[cfg(target_os = "android")]
        {
            s.crash = crate::android::previous_run_crash();
        }
        s
    }

    pub fn set_status(&mut self, text: impl Into<String>, err: bool) {
        let t = text.into();
        if err {
            core::log_to_file(&format!("ERROR {t}"));
        }
        self.status = (t, err, Instant::now());
    }

    fn spawn(&self, f: impl FnOnce() -> Msg + Send + 'static) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            // (a job that panics - an odd file of some mod - sent nothing, and the page
            // it was loading for said "loading" for ever: it says what went wrong instead)
            let m = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|e| {
                let why = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "an unknown error".into());
                Msg::Crashed(why)
            });
            let _ = tx.send(m);
        });
    }

    pub fn load_content(&mut self) {
        self.loading_content = true;
        self.set_status("Reading the OMSI folder…", false);
        self.spawn(|| {
            let r = (|| -> anyhow::Result<_> { Ok((core::list_maps()?, core::list_vehicles()?, core::list_weather()?)) })();
            Msg::Content(r.map_err(|e| format!("{e:#}")))
        });
    }

    pub fn load_lines(&mut self) {
        let (map, date) = (self.choice.map.clone(), self.choice.date.clone());
        if map.is_empty() {
            return;
        }
        // (every change of the date comes here: the depot file of that date, unless one was
        // chosen by hand)
        let hof = self.default_hof();
        if hof != self.choice.hof && !self.choice.hof_manual {
            log::info!("depot file for {date}: {hof}");
            self.choice.hof = hof;
        }
        self.loading_lines = true;
        self.lines_for = (map.clone(), date.clone());
        self.spawn(move || {
            let lines = core::list_lines(&map, &date).map_err(|e| format!("{e:#}"));
            Msg::Lines { map, date, lines }
        });
    }

    pub fn load_profiles(&mut self) {
        self.spawn(|| Msg::Profiles(core::list_profiles().unwrap_or_default()));
        let name = self.config.profile.clone();
        self.spawn(move || Msg::Profile(core::get_profile(&name).map_err(|e| format!("{e:#}"))));
    }

    pub fn load_profile(&mut self) {
        let name = self.config.profile.clone();
        self.spawn(move || Msg::Profile(core::get_profile(&name).map_err(|e| format!("{e:#}"))));
    }

    pub fn load_ibis(&mut self) {
        let (Some(line), false) = (self.choice.line.clone(), self.choice.free) else {
            self.ibis = None;
            return;
        };
        let (bus, hof) = (self.choice.bus.clone(), self.choice.hof.clone());
        let key = format!("{bus}|{hof}|{line}");
        if self.ibis.as_ref().map(|i| i.0 == key).unwrap_or(false) {
            return;
        }
        self.spawn(move || Msg::Ibis { key, info: core::ibis_info(&bus, &hof, &line).map_err(|e| format!("{e:#}")) });
    }

    pub fn load_args(&mut self) {
        let d = self.duty();
        self.spawn(move || Msg::Args(core::duty_args(&d).map_err(|e| format!("{e:#}"))));
    }

    pub fn load_mods(&mut self) {
        self.mods_asked = true;
        self.spawn(|| Msg::Mods(core::mods_status().map_err(|e| format!("{e:#}"))));
    }

    pub fn check_join(&mut self) {
        if self.join_checked == self.choice.lan_addr {
            return;
        }
        self.join_checked = self.choice.lan_addr.clone();
        let t = self.choice.lan_addr.clone();
        self.spawn(move || Msg::Join(core::check_join(&t)));
        // the host's status (its buses): at the code's addresses, else through its tunnel
        if omsi_net::looks_like_code(&self.choice.lan_addr) {
            let code = self.choice.lan_addr.clone();
            self.spawn(move || Msg::Server { info: host_status(&code), address: code });
        }
    }

    /// The buses the host or server offers (None: any, it has not said).
    pub fn host_vehicles(&self) -> Option<Vec<String>> {
        if self.choice.lan_mode != "join" {
            return None;
        }
        let key = self.joined_server.clone().unwrap_or_else(|| self.choice.lan_addr.clone());
        let list = self.server_info.get(&key)?.1.as_ref().ok()?.vehicles.clone();
        (!list.is_empty()).then_some(list)
    }

    /// Keep the server list on disk.
    pub fn save_servers(&self) {
        let _ = std::fs::create_dir_all(core::data_dir());
        let _ = std::fs::write(servers_path(), serde_json::to_vec_pretty(&self.servers).unwrap_or_default());
    }

    /// Ask a server about itself (its status and icon), at most every `every` seconds.
    pub fn ask_server(&mut self, address: &str, every: f32) {
        if self.server_asked.get(address).map(|t| t.elapsed().as_secs_f32() < every).unwrap_or(false) {
            return;
        }
        self.server_asked.insert(address.to_string(), Instant::now());
        let a = address.to_string();
        self.spawn(move || Msg::Server { info: omsi_net::ws::query(&a, true), address: a });
    }

    /// The Drive page joins `address`: the server's map is the map, the session is joined.
    pub fn join_server(&mut self, address: &str) {
        let Some((_, Ok(info))) = self.server_info.get(address).cloned() else {
            self.set_status("The server has not answered yet (is its address right? is it running?)", true);
            return;
        };
        if !self.maps.is_empty() && !self.maps.iter().any(|m| m.file.eq_ignore_ascii_case(&info.map)) {
            self.set_status(format!("The server plays {}, which is not installed here: install that map first.", info.map), true);
            return;
        }
        self.choice.map = info.map.clone();
        self.choice.lan_mode = "join".into();
        // (a server added by its bare address is joined where it answered: its web gateway)
        let bare = omsi_net::ws::ws_url(address).is_none() && !omsi_net::official::is_alias(address);
        self.choice.lan_addr = if bare && !info.reached_at.is_empty() { info.reached_at.clone() } else { address.to_string() };
        self.joined_server = Some(address.to_string());
        self.join = (true, format!("the server {}", info.name));
        self.join_checked = address.to_string();
        self.touched();
        self.set_status(format!("Joined {} - choose your bus and duty, then Start the duty", info.name), false);
    }

    /// Back to playing alone (the Drive page's "Leave Server").
    pub fn leave_server(&mut self) {
        self.joined_server = None;
        self.choice.lan_mode = "off".into();
        self.touched();
    }

    pub fn poll_now(&mut self) {
        if self.polling {
            return;
        }
        self.polling = true;
        self.spawn(|| Msg::Poll(core::poll().map_err(|e| format!("{e:#}"))));
    }

    pub fn log_tail(&mut self, pid: u32) {
        self.spawn(move || Msg::LogTail { pid, lines: core::log_tail(pid, 80).unwrap_or_default() });
    }

    pub fn stop(&mut self, pid: u32) {
        self.stopping.insert(pid);
        self.spawn(move || Msg::Stopped { pid, result: core::stop_instance(pid).map_err(|e| format!("{e:#}")) });
    }

    pub fn install(&mut self, path: String) {
        let mode = ["auto", "extract", "inplace"][self.mod_mode.min(2)].to_string();
        self.mod_path = path.clone();
        let p2 = path.clone();
        self.spawn(move || Msg::ModInfo(core::inspect_mod(std::path::Path::new(&p2)).map_err(|e| format!("{e:#}"))));
        self.spawn(move || Msg::Installed(core::start_install(std::path::Path::new(&path), &mode).map_err(|e| format!("{e:#}"))));
    }

    pub fn launch(&mut self) {
        if !self.save_pending_settings() {
            return;
        }
        if !omsi_cfg::missing_original_essentials(std::path::Path::new(&self.config.root)).is_empty() {
            self.set_status("A session needs the original OMSI 2: choose its folder under Setup first.", true);
            return;
        }
        let d = self.duty();
        self.set_status("Starting the game…", false);
        self.spawn(move || Msg::Launched(core::launch(&d).map_err(|e| format!("{e:#}"))));
    }

    /// The duty as the backend takes it.
    pub fn duty(&self) -> core::Duty {
        let c = &self.choice;
        let lan = match c.lan_mode.as_str() {
            "host" => "host".to_string(),
            "join" => format!("join:{}", c.lan_addr.trim()),
            _ => "off".to_string(),
        };
        // Joining: the host's map, as its status gives it (a code's host is asked when the
        // code is typed). The game takes it from the host's welcome too, but only when that
        // comes before the map is loaded: through a tunnel it came later, the game started
        // on the map chosen here, and the players never met ("the host drives on X10 Berlin,
        // you on Berlin-Spandau"). A map not installed here comes with the host's mods.
        let host_map = (c.lan_mode == "join")
            .then(|| self.joined_server.clone().unwrap_or_else(|| c.lan_addr.clone()))
            .and_then(|k| self.server_info.get(&k).and_then(|x| x.1.as_ref().ok()).map(|i| i.map.trim().replace('\\', "/")))
            .filter(|m| m.to_ascii_lowercase().contains("maps/"));
        core::Duty {
            map: host_map.unwrap_or_else(|| c.map.clone()),
            bus: c.bus.clone(),
            paint: Some(c.paint.clone()).filter(|p| !p.is_empty()),
            plate: Some(c.plate.clone()).filter(|p| !p.trim().is_empty()),
            hof: Some(c.hof.clone()).filter(|p| !p.is_empty()),
            entry: Some(c.entry),
            line: if c.free { None } else { c.line.clone() },
            tour: if c.free { None } else { c.tour.clone() },
            trip: None,
            time: format!("{:02}:{:02}", c.time / 60, c.time % 60),
            date: Some(c.date.clone()),
            weather: Some(c.weather.clone()).filter(|w| !w.is_empty()),
            traffic: Some(c.traffic.round() as u32),
            passengers: Some(c.passengers),
            schedule: Some(c.schedule),
            autostart: Some(c.autostart),
            on_foot: Some(c.on_foot),
            profile: Some(self.config.profile.clone()).filter(|p| !p.is_empty()),
            lan: Some(lan),
            lan_name: None,
            season: Some(c.season.clone()).filter(|s| s != "auto"),
            tutorial: None,
            situation: None,
        }
    }

    /// Whether a situation to continue lies on the chosen map (looked up at most every
    /// two seconds: the page asks every frame).
    pub fn has_last_situation(&mut self) -> bool {
        let fresh = self.last_sit.as_ref().is_some_and(|(m, _, t)| *m == self.choice.map && t.elapsed().as_secs_f32() < 2.0);
        if !fresh {
            let there = core::last_situation(&self.choice.map).is_some();
            self.last_sit = Some((self.choice.map.clone(), there, std::time::Instant::now()));
        }
        self.last_sit.as_ref().map(|x| x.1).unwrap_or(false)
    }

    /// Continue the situation the game left on the chosen map (`laststn.osn`).
    pub fn launch_last_situation(&mut self) {
        if !self.save_pending_settings() {
            return;
        }
        let Some(file) = core::last_situation(&self.choice.map) else {
            self.set_status("No situation left on this map yet", true);
            return;
        };
        let mut d = self.duty();
        d.situation = Some(file.to_string_lossy().to_string());
        d.lan = Some("off".into());
        self.set_status("Continuing where you left off…", false);
        self.spawn(move || Msg::Launched(core::launch(&d).map_err(|e| format!("{e:#}"))));
    }

    /// Start one of OMSI's tutorials (1..4).
    pub fn launch_tutorial(&mut self, n: usize) {
        if !self.save_pending_settings() {
            return;
        }
        let mut d = self.duty();
        d.tutorial = Some(n);
        d.lan = Some("off".into());
        self.set_status("Starting the tutorial…", false);
        self.spawn(move || Msg::Launched(core::launch(&d).map_err(|e| format!("{e:#}"))));
    }

    /// Settings a game changed while it ran: taken over, unless the launcher's own changes
    /// wait to be saved (those win, as the later ones).
    fn reload_changed_settings(&mut self) {
        if self.settings_dirty > 0.0 {
            return;
        }
        let now = read_settings_file();
        if now.is_some() && now != self.settings_file {
            if let Ok(v) = core::get_settings() {
                self.settings = v;
            }
            self.settings_file = now;
        }
    }

    fn save_pending_settings(&mut self) -> bool {
        if self.settings_dirty <= 0.0 {
            return true;
        }
        match core::save_settings(&self.settings) {
            Ok(()) => {
                self.settings_dirty = 0.0;
                self.settings_file = read_settings_file();
                true
            }
            Err(e) => {
                self.set_status(format!("Could not save settings: {e:#}"), true);
                false
            }
        }
    }

    /// Something of the duty changed: remember it (soon) and refresh what depends on it.
    pub fn touched(&mut self) {
        self.choice_dirty = 0.4;
    }

    /// Work done each frame: results of background work, the regular poll, saving.
    pub fn update(&mut self, dt: f32) {
        while let Ok(m) = self.rx.try_recv() {
            self.handle(m);
        }
        self.poll_t -= dt;
        if self.poll_t <= 0.0 {
            self.poll_t = 2.5;
            self.poll_now();
            self.reload_changed_settings();
        }
        if self.choice_dirty > 0.0 {
            self.choice_dirty -= dt;
            if self.choice_dirty <= 0.0 {
                self.choice.save();
                self.load_args();
                self.load_ibis();
            }
        }
        if self.settings_dirty > 0.0 {
            self.settings_dirty -= dt;
            if self.settings_dirty <= 0.0 {
                match core::save_settings(&self.settings) {
                    Ok(()) => {
                        self.settings_file = read_settings_file();
                        self.set_status("Settings saved.", false)
                    }
                    Err(e) => self.set_status(format!("{e:#}"), true),
                }
            }
        }
        if self.choice.lan_mode == "join" {
            self.check_join();
        }
        let now = Instant::now();
        self.fresh.retain(|_, t| now.duration_since(*t).as_secs() < 600);
    }

    fn handle(&mut self, m: Msg) {
        match m {
            Msg::Crashed(why) => {
                log::error!("launcher: a background job stopped: {why}");
                self.loading_content = false;
                self.loading_lines = false;
                self.set_status(format!("Reading the content stopped on an error: {why}"), true);
            }
            Msg::Server { address, info } => {
                // the host of the code typed in: its map is the one the duty is chosen on
                // (installed here: the line, tour and entry point of another map go)
                if self.choice.lan_mode == "join" && self.joined_server.is_none() && address == self.choice.lan_addr {
                    if let Ok(i) = &info {
                        let theirs = i.map.trim().replace('\\', "/");
                        if let Some((file, name)) = self.maps.iter().find(|m| m.file.eq_ignore_ascii_case(&theirs)).map(|m| (m.file.clone(), m.name.clone())) {
                            if !self.choice.map.eq_ignore_ascii_case(&file) {
                                self.choice.map = file;
                                self.choice.line = None;
                                self.choice.tour = None;
                                self.choice.entry = 0;
                                self.touched();
                                self.set_status(format!("The host drives on {name}: that map is chosen"), false);
                            }
                        }
                    }
                }
                self.server_info.insert(address, (Instant::now(), info));
            }
            Msg::Content(Ok((maps, vehicles, weathers))) => {
                // (names of things, not the interface: never machine-translated)
                crate::mt::protect(maps.iter().flat_map(|m| [m.name.as_str(), m.friendly.as_str()]));
                crate::mt::protect(vehicles.iter().flat_map(|v| [v.name.as_str(), v.manufacturer.as_str(), v.type_name.as_str()]).chain(vehicles.iter().flat_map(|v| v.paints.iter().map(|p| p.as_str()))));
                crate::mt::protect(weathers.iter().map(|w| w.name.as_str()));
                let known: std::collections::HashSet<String> = self.maps.iter().map(|m| m.file.clone()).chain(self.vehicles.iter().map(|v| v.file.clone())).chain(self.weathers.iter().map(|w| w.file.clone())).collect();
                if !known.is_empty() {
                    for f in maps.iter().map(|m| &m.file).chain(vehicles.iter().map(|v| &v.file)).chain(weathers.iter().map(|w| &w.file)) {
                        if !known.contains(f) {
                            self.fresh.insert(f.clone(), Instant::now());
                        }
                    }
                }
                self.set_status(format!("{} maps, {} buses, {} weathers", maps.len(), vehicles.len(), weathers.len()), false);
                self.maps = maps;
                self.vehicles = vehicles;
                self.weathers = weathers;
                self.loading_content = false;
                // what was chosen last time, where it still exists
                if !self.vehicles.iter().any(|v| v.file == self.choice.bus) {
                    if let Some(v) = self.vehicles.first() {
                        self.choice.bus = v.file.clone();
                        self.choice.paint.clear();
                        self.choice.hof = self.default_hof();
                    }
                }
                if !self.maps.iter().any(|m| m.file == self.choice.map) {
                    // the map OMSI 2 had last, else the first
                    let last = core::omsi_options(std::path::Path::new(&self.config.root)).and_then(|o| o.last_map);
                    if let Some(m) = last.and_then(|l| self.maps.iter().find(|m| m.file.eq_ignore_ascii_case(&l))).or(self.maps.first()) {
                        self.choice.map = m.file.clone();
                        self.choice.entry = -1;
                    }
                }
                self.load_lines();
                self.load_args();
                self.load_ibis();
            }
            Msg::Content(Err(e)) => {
                self.loading_content = false;
                if omsi_cfg::missing_original_essentials(std::path::Path::new(&self.config.root)).is_empty() {
                    self.set_status(format!("{e}\nSet the OMSI 2 folder under Setup."), true);
                } else {
                    self.set_status(root_problem(&self.config.root), true);
                }
            }
            Msg::Lines { map, date, lines } => {
                if let Ok(ls) = lines.as_ref() {
                    crate::mt::protect(ls.iter().flat_map(|l| l.termini.iter().map(|t| t.as_str()).chain([l.name.as_str()])).chain(ls.iter().flat_map(|l| l.tours.iter().map(|t| t.number.as_str()))));
                }
                if (map, date) != self.lines_for {
                    return;
                }
                self.loading_lines = false;
                match lines {
                    Ok(l) => {
                        self.lines = l;
                        let mut note = String::new();
                        if let Some(line) = self.choice.line.clone() {
                            match self.lines.iter().find(|x| x.name == line) {
                                None => {
                                    note = format!(" - line {line} does not run on {}", self.choice.date);
                                    self.choice.line = None;
                                    self.choice.tour = None;
                                }
                                Some(l) => {
                                    if let Some(t) = &self.choice.tour {
                                        match l.tours.iter().find(|x| &x.number == t) {
                                            None => self.choice.tour = None,
                                            Some(t) if !t.runs => note = format!(" - tour {} of line {line} does not run that day ({})", t.number, t.days),
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                        self.set_status(format!("{} lines on {}{note}", self.lines.len(), self.choice.date), !note.is_empty());
                    }
                    Err(e) => {
                        self.lines.clear();
                        self.set_status(e, true);
                    }
                }
                self.load_ibis();
            }
            Msg::Poll(r) => {
                self.polling = false;
                match r {
                    Ok(p) => {
                        for s in &p.started {
                            core::log_to_file(&format!("inbox: installing {s}"));
                        }
                        if !p.started.is_empty() {
                            self.set_status(format!("Installing from the Mods folder: {}", p.started.iter().map(|x| x.rsplit('/').next().unwrap_or(x)).collect::<Vec<_>>().join(", ")), false);
                        }
                        let mut installed = false;
                        for j in &p.jobs {
                            let was = self.jobs.iter().find(|x| x.id == j.id).map(|x| x.finished.is_some()).unwrap_or(false);
                            if j.finished.is_some() && !was && self.stamp.is_some() {
                                if j.state == "done" {
                                    installed = true;
                                    self.set_status(j.message.clone(), false);
                                } else if j.state == "failed" {
                                    self.set_status(format!("{}: {}", j.name, j.message), true);
                                }
                            }
                        }
                        self.jobs = p.jobs;
                        // a game that was running and is not any more: did it end on an error?
                        for old in self.instances.iter().filter(|i| i.running) {
                            let still = p.instances.iter().any(|n| n.pid == old.pid && n.running);
                            if !still && !self.stopping.contains(&old.pid) {
                                if let Some(c) = crash_of(std::path::Path::new(&old.log)) {
                                    core::log_to_file(&format!("game {} ended on an error: {}", old.pid, c.0));
                                    self.crash = Some(c);
                                }
                            }
                        }
                        self.instances = p.instances;
                        for i in &self.instances {
                            if !i.running {
                                self.stopping.remove(&i.pid);
                            }
                        }
                        let changed = self.stamp.as_ref().map(|s| *s != p.stamp).unwrap_or(false);
                        self.stamp = Some(p.stamp);
                        if changed || installed {
                            self.load_content();
                            self.load_mods();
                        }
                        for pid in self.open_logs.clone() {
                            self.log_tail(pid);
                        }
                    }
                    Err(e) => core::log_to_file(&format!("poll: {e}")),
                }
            }
            Msg::Profile(Ok(p)) => self.profile = Some(p),
            Msg::Profile(Err(e)) => {
                self.profile = None;
                core::log_to_file(&format!("profile: {e}"));
            }
            Msg::Profiles(p) => {
                self.profiles = p;
                if !self.profiles.contains(&self.config.profile) {
                    if let Some(f) = self.profiles.first() {
                        self.config.profile = f.clone();
                        let _ = core::save_config(&self.config);
                        self.load_profile();
                    }
                }
            }
            Msg::Ibis { key, info } => self.ibis = Some((key, info)),
            Msg::Args(r) => {
                self.cmdline = match r {
                    Ok(a) => format!("omsi {}", a.iter().map(|x| if x.contains(' ') { format!("\"{x}\"") } else { x.clone() }).collect::<Vec<_>>().join(" ")),
                    Err(e) => e,
                }
            }
            Msg::Launched(Ok(l)) => {
                core::log_to_file(&format!("launched pid {} ({} other game(s) running): {}", l.pid, l.others, l.command));
                self.set_status(format!("Game started (process {}), log {}{}", l.pid, l.log, if l.others > 0 { format!(" - {} other game(s) keep running", l.others) } else { String::new() }), false);
                self.poll_now();
            }
            Msg::Launched(Err(e)) => self.set_status(e, true),
            Msg::Stopped { pid, result } => {
                self.stopping.remove(&pid);
                match result {
                    Ok(true) => self.set_status(format!("Game {pid} ended by itself."), false),
                    Ok(false) => self.set_status(format!("Game {pid} did not end by itself and was killed - this run is not saved."), true),
                    Err(e) => self.set_status(e, true),
                }
                self.poll_now();
            }
            Msg::LogTail { pid, lines } => {
                self.logs.insert(pid, lines);
            }
            Msg::Mods(Ok(m)) => {
                for c in &m.cleaned {
                    core::log_to_file(&format!("cleanup: {c}"));
                }
                self.jobs = m.jobs.clone();
                self.mods = Some(m);
            }
            Msg::Mods(Err(e)) => self.set_status(e, true),
            Msg::ModInfo(r) => {
                if let Ok(i) = &r {
                    // a big archive that does not fit is used in place
                    if !i.fits && i.in_place_ok && self.mod_mode == 1 {
                        self.mod_mode = 2;
                    }
                }
                self.mod_info = Some(r);
            }
            Msg::Installed(Ok(j)) => {
                core::log_to_file(&format!("install started: {}", j.source));
                self.jobs.retain(|x| x.id != j.id);
                self.jobs.insert(0, j);
                self.poll_now();
            }
            Msg::Installed(Err(e)) => self.set_status(e, true),
            Msg::Join(v) => {
                let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
                let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
                self.join = (ok, text);
            }
        }
    }

    // --- helpers for the pages -------------------------------------------------------------

    pub fn bus(&self) -> Option<&core::VehicleInfo> {
        self.vehicles.iter().find(|v| v.file == self.choice.bus)
    }
    pub fn map(&self) -> Option<&core::MapInfo> {
        self.maps.iter().find(|m| m.file == self.choice.map)
    }
    pub fn line(&self) -> Option<&core::LineInfo> {
        let l = self.choice.line.as_ref()?;
        self.lines.iter().find(|x| &x.name == l)
    }
    pub fn tour(&self) -> Option<&core::TourInfo> {
        let t = self.choice.tour.as_ref()?;
        self.line()?.tours.iter().find(|x| &x.number == t)
    }

    /// The depot file a bus uses on the chosen map: the map's own when the bus has it (or
    /// has none: the game borrows it), else the bus's first.
    /// The depot file for the chosen bus: the one the map's own buses use on the chosen date
    /// (the chrono scenarios change it: Berlin's 1994 depot has line 137 where 1986's had
    /// 92), which the bus has, else its first.
    pub fn default_hof(&self) -> String {
        let on_date = self.map().and_then(|m| {
            let dir = omsi_cfg::resolve_path(std::path::Path::new(&self.config.root), &m.file);
            omsi_map::ailists::depot_hof_on(dir.parent()?, omsi_map::ailists::date_code(&self.choice.date)?)
        });
        let want = on_date.or_else(|| self.map().map(|m| m.hof.clone())).unwrap_or_default();
        let Some(v) = self.bus() else { return want };
        v.hofs.iter().find(|h| h.eq_ignore_ascii_case(&want)).cloned().or(Some(want).filter(|w| !w.is_empty())).or_else(|| v.hofs.first().cloned()).unwrap_or_default()
    }

    pub fn select_bus(&mut self, file: &str) {
        if self.choice.bus == file {
            return;
        }
        self.choice.bus = file.to_string();
        self.choice.paint.clear();
        // (a hand-picked depot file stays when the new bus has one of that name)
        let keep = self.choice.hof_manual && self.bus().is_some_and(|v| v.hofs.iter().any(|h| h.eq_ignore_ascii_case(&self.choice.hof)));
        if !keep {
            self.choice.hof_manual = false;
            self.choice.hof = self.default_hof();
        }
        self.touched();
    }

    pub fn select_map(&mut self, file: &str) {
        if self.choice.map == file {
            return;
        }
        self.choice.map = file.to_string();
        self.choice.entry = -1;
        self.choice.line = None;
        self.choice.tour = None;
        self.choice.hof = self.default_hof();
        self.lines.clear();
        self.load_lines();
        self.touched();
    }

    /// The season the chosen date (or the override) means.
    pub fn season(&self) -> &str {
        if self.choice.season != "auto" {
            return &self.choice.season;
        }
        let m: u32 = self.choice.date.get(5..7).and_then(|x| x.parse().ok()).unwrap_or(5);
        match m {
            12 | 1 | 2 => "winter",
            3..=5 => "spring",
            6..=8 => "summer",
            _ => "autumn",
        }
    }

    /// Weather presets that make sense in the season (no summer shower in the snow).
    pub fn weather_fits(&self, w: &core::WeatherInfo) -> bool {
        let rain = w.precip.starts_with("rain");
        match self.season() {
            "winter" => w.snow || if rain { w.temp <= 10.0 } else { w.temp <= 16.0 },
            s => {
                if w.snow || w.temp <= 0.0 {
                    return false;
                }
                s != "summer" || w.temp >= 8.0
            }
        }
    }

    /// The trip of the tour a start at the chosen time begins with: the next to leave (one
    /// that left a minute or two ago still counts), else the tour's last.
    pub fn first_trip(&self) -> Option<usize> {
        let t = self.tour()?;
        let now = self.choice.time as f64 * 60.0;
        trip_index_at(t, now)
    }
}

/// The trip a tour starts with at `now`, shared by the route preview and the launch choice.
pub(super) fn trip_index_at(tour: &core::TourInfo, now: f64) -> Option<usize> {
    tour.trips.iter().position(|x| x.departure >= now - 120.0).or(if tour.trips.is_empty() { None } else { Some(tour.trips.len() - 1) })
}

pub fn hhmm(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as i64;
    format!("{:02}:{:02}", (s / 3600) % 24, (s % 3600) / 60)
}

pub fn fmt_bytes(b: u64) -> String {
    let b = b as f64;
    if b >= (1u64 << 30) as f64 {
        format!("{:.1} GB", b / (1u64 << 30) as f64)
    } else if b >= (1u64 << 20) as f64 {
        format!("{:.0} MB", b / (1u64 << 20) as f64)
    } else {
        format!("{:.0} KB", (b / 1024.0).max(1.0))
    }
}

pub fn short_map(m: &str) -> String {
    m.trim_start_matches("maps/").trim_end_matches("/global.cfg").to_string()
}

/// Why `root` is not an OMSI 2 to play on, said so that the player knows what to choose.
pub fn root_problem(root: &str) -> String {
    let root = root.trim();
    let p = std::path::Path::new(root);
    let missing = omsi_cfg::missing_original_essentials(p);
    if root.is_empty() {
        "The original OMSI 2 was not found automatically: choose its folder (the one with Omsi.exe, maps and Vehicles in it) under Setup and press Save.".to_string()
    } else if !p.exists() {
        format!("{root} does not exist: choose the folder of the original OMSI 2 (with Omsi.exe, maps and Vehicles in it) under Setup.")
    } else if missing.iter().any(|m| m.contains("content folder")) || p.join("openomsi.exe").exists() || p.join("openomsi").is_file() {
        format!("{root} is openOMSI's own folder, not OMSI 2's: choose the folder of the original game (with Omsi.exe in it) under Setup.")
    } else if missing.is_empty() {
        String::new()
    } else {
        format!("{root} is not a complete OMSI 2 - it lacks {}. openOMSI plays on the original's stock content: choose the folder of a complete installation under Setup.", missing.iter().take(3).cloned().collect::<Vec<_>>().join(", "))
    }
}

/// What a game's log says when the game ended on an error: the error (a panic, "no graphics
/// adapter", a fatal message) and the last lines of the log. None for a game that ended as
/// it should.
pub fn crash_of(log: &std::path::Path) -> Option<(String, String)> {
    let text = std::fs::read(log).ok()?;
    let text = String::from_utf8_lossy(&text[text.len().saturating_sub(64 * 1024)..]).to_string();
    let lines: Vec<&str> = text.lines().collect();
    // (a lost graphics device ends the game in order - it saves the run - but it is a crash
    // for the player all the same: the driver gave up)
    // (one the game got over by starting again with safer graphics is no crash)
    if lines.iter().any(|l| l.contains("starting again with safer graphics")) {
        return None;
    }
    let lost = lines.iter().rposition(|l| l.contains("the graphics device was lost"));
    if lost.is_none() && lines.iter().any(|l| l.contains("game ends")) {
        return None;
    }
    let at = lost.or_else(|| lines.iter().rposition(|l| l.contains("the game stopped on an error") || l.contains(" ERROR ")))?;
    let first = lines[at].split_once("] ").map(|x| x.1).unwrap_or(lines[at]).trim();
    // (a panic's message is on the following lines)
    let mut what = first.to_string();
    for l in lines.iter().skip(at + 1).take(6) {
        if l.trim().is_empty() || l.trim_start().starts_with("0:") {
            break;
        }
        what.push(' ');
        what.push_str(l.trim());
    }
    let tail = lines[lines.len().saturating_sub(150)..].join("\n");
    Some((what.chars().take(600).collect(), tail))
}

#[cfg(test)]
mod choice_tests {
    /// `launcher-duty.json` from before the number plate field: the missing key falls back to
    /// the default (no plate), and a typed plate survives a round trip.
    #[test]
    fn an_old_duty_file_loads_and_a_typed_plate_is_kept() {
        let old: super::Choice = serde_json::from_str(r#"{"bus":"Vehicles/x.bus","map":"maps/x/global.cfg"}"#).unwrap();
        assert_eq!(old.plate, "");
        let mut c = super::Choice::default();
        c.plate = "B-AB 1234".into();
        let back: super::Choice = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.plate, "B-AB 1234");
    }
}

#[cfg(test)]
mod crash_tests {
    #[test]
    fn a_panic_is_found_and_a_clean_end_is_not() {
        let dir = std::env::temp_dir().join("openomsi-crash-test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("game.log");
        std::fs::write(&p, "[t INFO x] loading\n[t ERROR openomsi_game] the game stopped on an error (build x): panicked at a.rs:1:1:\n    index out of bounds\n\n   0: std::backtrace\n").unwrap();
        let (what, tail) = super::crash_of(&p).unwrap();
        assert!(what.contains("index out of bounds"), "{what}");
        assert!(tail.contains("loading"));
        std::fs::write(&p, "[t INFO x] loading\n[t INFO openomsi_game::app_events] game ends\n").unwrap();
        assert!(super::crash_of(&p).is_none());
        std::fs::write(&p, "[t ERROR omsi_render] the graphics device was lost (Unknown): Unexpected error variant\n[t INFO openomsi_game::app_events] game ends\n").unwrap();
        assert!(super::crash_of(&p).unwrap().0.contains("device was lost"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

fn read_settings_file() -> Option<String> {
    std::fs::read_to_string(core::data_dir().join("settings.cfg")).ok()
}
