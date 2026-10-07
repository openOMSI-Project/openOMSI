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
    /// The first reading of the content: the maps and weathers (quick to read), before the
    /// buses.
    ContentEarly { maps: Vec<core::MapInfo>, weathers: Vec<core::WeatherInfo> },
    /// The first reading of the content: the buses of the next few folders, how many
    /// folders are read and how many there are.
    VehiclesRead { batch: Vec<core::VehicleInfo>, done: usize, total: usize },
    Lines { map: String, date: String, lines: Result<Vec<core::LineInfo>, String>, own: Vec<core::lines::OwnLine> },
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

/// How the Multiplayer page joins a server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JoinProto {
    /// The web gateway where the server answered, else UDP.
    #[default]
    Auto,
    /// Straight to the game port over UDP (no status needed).
    Udp,
    /// Through the server's web gateway (a WebSocket).
    WebSocket,
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
    /// The fleet number picked from the bus's `[number]` list (empty: its first).
    pub number: String,
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
    pub start_trip: Option<(String, String, usize, i32)>,
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
    /// The duty is put together of trips of several tours (see `composed_duties`), not one
    /// tour of one line.
    pub composed: bool,
    /// The length of such a duty (minutes) and the part of the day it begins in (an index
    /// into `DAYPARTS`).
    pub duty_minutes: i32,
    pub daypart: usize,
    /// The composed duty chosen: its parts as `--duty-leg` takes them. Its first part is
    /// also `line`, `tour` and `start_trip`.
    pub legs: Vec<String>,
    /// Free drive with a line the player chose ("Choose a line yourself", see `freedrive`):
    /// the switch, the timetable's line (`LineInfo::name`) and the route driven (its trip
    /// file, `TripInfo::name`). The bus starts at the route's first stop; nothing is booked.
    pub own_line: bool,
    pub free_line: String,
    pub free_route: String,
    /// Free drive without a line: the stop chosen to start at (its name; the bus stands at
    /// the entry point nearest to it), and the entry point the bus stands at - the free
    /// drive's own, so that a stop chosen there does not move a duty's start (`entry`).
    /// Empty and -1: the map's first entry point.
    pub free_stop: String,
    pub free_entry: i32,
    /// The line lists show the player's own lines (the line editor's) instead of the map's.
    pub my_lines: bool,
}

/// The parts of the day a composed duty may begin in: name, and from-to in minutes of the
/// day (the night runs past midnight).
pub const DAYPARTS: [(&str, Option<(i32, i32)>); 5] = [
    ("All day", None),
    ("Morning", Some((4 * 60, 11 * 60))),
    ("Midday", Some((11 * 60, 16 * 60))),
    ("Evening", Some((16 * 60, 22 * 60))),
    ("Night", Some((22 * 60, 30 * 60))),
];

impl Default for Choice {
    fn default() -> Self {
        Choice {
            bus: String::new(),
            paint: String::new(),
            plate: String::new(),
            number: String::new(),
            hof: String::new(),
            hof_manual: false,
            map: String::new(),
            entry: -1,
            line: None,
            tour: None,
            free: false,
            time: 9 * 60,
            start_trip: None,
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
            composed: false,
            duty_minutes: 120,
            daypart: 0,
            legs: Vec::new(),
            own_line: false,
            free_line: String::new(),
            free_route: String::new(),
            free_stop: String::new(),
            free_entry: -1,
            my_lines: false,
        }
    }
}

fn choice_path() -> std::path::PathBuf {
    core::data_dir().join("launcher-duty.json")
}

impl Choice {
    pub fn load() -> Choice {
        let c: Choice = std::fs::read_to_string(choice_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self::fresh(c)
    }

    /// A duty read back from the file, as a new launcher starts with it.
    fn fresh(mut c: Choice) -> Choice {
        // A server is joined on purpose, in the launcher's own session (Multiplayer page): a
        // join left in the file made every later start connect to it, with no sign of it on
        // the Drive page ("Leave Server" is only there for a server joined since the launch).
        if c.lan_mode == "join" {
            c.lan_mode = "off".into();
        }
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
    /// The player's own lines of the map (see `ownlines`), read with the lines.
    pub own_lines: Vec<core::lines::OwnLine>,
    pub lines_for: (String, String),
    pub loading_content: bool,
    /// The lists are filled while they are read (the first reading: nothing to show yet).
    pub content_first: bool,
    /// The content changed while it was being read: read it again when that is done.
    pub reload_content: bool,
    pub loading_lines: bool,
    pub choice: Choice,
    pub choice_dirty: f32,
    /// The bus options chosen per bus, and what each bus offers (see `busoptions`).
    pub bus_options: super::busoptions::Options,
    /// The display font chosen per bus, the fonts installed and their previews (see
    /// `displayfont`).
    pub display_fonts: super::displayfont::DisplayFonts,
    /// Map, whether it has a `laststn.osn`, when that was looked up.
    pub last_sit: Option<(String, Vec<core::SavedSituation>, std::time::Instant)>,
    /// Which of them "Continue" starts (0: the newest, the last situation when there is one).
    pub save_pick: usize,
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
    pub queued_launch: Option<core::Duty>,
    /// Start was pressed: the graphics device stays given up until the list of games has the
    /// game started (its process, once it is known), 15 s at most.
    pub launch_hold: Option<std::time::Instant>,
    launched_pid: Option<u32>,
    /// A game started from here ended on an error: what it said, and the end of its log
    /// (see `crash_of`), for the dialog that asks to report it.
    pub crash: Option<(String, String)>,
    /// A game started from here was sent away by its server (kicked, banned) or turned away
    /// at the door: the server's message, for the "Disconnected from the server" dialog.
    pub disconnected: Option<String>,
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
    /// The composed duties offered (see `composed_duties`) and what they were made for: the
    /// lines' map and date, the length, the part of the day, the seed, the number of lines.
    composed: Vec<core::compose::ComposedDuty>,
    composed_for: Option<(String, String, i32, usize, u64, usize)>,
    /// What the composed duties are drawn with: "Other shifts" takes another.
    pub compose_seed: u64,
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
        let mut choice = Choice::load();
        // (at the real time, a trip picked in an earlier session has most likely left)
        if settings.get("use_real_time").and_then(|v| v.as_bool()).unwrap_or(false) {
            choice.start_trip = None;
        }
        let mut s = State {
            config,
            maps: Vec::new(),
            vehicles: Vec::new(),
            weathers: Vec::new(),
            lines: Vec::new(),
            own_lines: Vec::new(),
            lines_for: (String::new(), String::new()),
            loading_content: false,
            content_first: false,
            reload_content: false,
            loading_lines: false,
            choice,
            choice_dirty: 0.0,
            bus_options: super::busoptions::Options::load(),
            display_fonts: super::displayfont::DisplayFonts::load(),
            last_sit: None,
            save_pick: 0,
            profiles: Vec::new(),
            profile: None,
            settings,
            settings_dirty: 0.0,
            settings_file: read_settings_file(),
            keybindings,
            keybindings_error: String::new(),
            instances: Vec::new(),
            queued_launch: None,
            launch_hold: None,
            launched_pid: None,
            crash: None,
            disconnected: None,
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
            composed: Vec::new(),
            composed_for: None,
            // (another set of duties at each start of the launcher)
            compose_seed: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1),
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

    /// A game is about to start, starting (not in the list of games yet) or running.
    pub fn in_game(&self) -> bool {
        self.queued_launch.is_some() || self.launch_hold.is_some_and(|t| t.elapsed().as_secs_f32() < 15.0) || self.instances.iter().any(|i| i.running)
    }

    pub fn instances_ready(&self) -> bool {
        self.stamp.is_some()
    }

    pub fn spawn_launch(&mut self, d: core::Duty) {
        self.launch_hold = Some(std::time::Instant::now());
        self.launched_pid = None;
        self.spawn(move || Msg::Launched(core::launch(&d).map_err(|e| format!("{e:#}"))));
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
        // the first reading shows what it has read as it goes: a big installation's buses
        // (thousands of folders) took minutes, with nothing on the page all that time
        self.content_first = self.maps.is_empty() && self.vehicles.is_empty();
        self.set_status("Reading the OMSI folder…", false);
        let tx = self.tx.clone();
        self.spawn(move || {
            let r = (|| -> anyhow::Result<_> {
                let maps = core::list_maps()?;
                let weathers = core::list_weather()?;
                let _ = tx.send(Msg::ContentEarly { maps: maps.clone(), weathers: weathers.clone() });
                let vehicles = core::list_vehicles_progress(|batch, done, total| {
                    let _ = tx.send(Msg::VehiclesRead { batch: batch.to_vec(), done, total });
                })?;
                Ok((maps, vehicles, weathers))
            })();
            Msg::Content(r.map_err(|e| format!("{e:#}")))
        });
    }

    /// A reading of the lists ended: the next poll's stamp is the new reference, and a change
    /// that came while reading is read now.
    fn content_done(&mut self) {
        self.stamp = None;
        if std::mem::take(&mut self.reload_content) {
            self.load_content();
        }
    }

    /// The map chosen last time where it still exists, else the one OMSI 2 had last, else
    /// the first.
    fn pick_map(&mut self) {
        if !self.maps.iter().any(|m| m.file == self.choice.map) {
            let last = core::omsi_options(std::path::Path::new(&self.config.root)).and_then(|o| o.last_map);
            if let Some(m) = last.and_then(|l| self.maps.iter().find(|m| m.file.eq_ignore_ascii_case(&l))).or(self.maps.first()) {
                self.choice.map = m.file.clone();
                self.choice.entry = -1;
            }
        }
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
            // (the player's own lines of the map, from the line editor's registry)
            let own = core::lines::own_lines_of_map(&map);
            Msg::Lines { map, date, lines, own }
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

    /// What the bus options section asked for `bus` (see `busoptions`): done and kept, and
    /// the command line made again.
    pub fn edit_bus_options(&mut self, bus: &str, e: super::busoptions::Edit) {
        let fold = e == super::busoptions::Edit::Technical;
        self.bus_options.edit(bus, e);
        if !fold {
            self.load_args();
        }
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
    /// `proto` says how: over UDP straight to the game port, over the server's web gateway
    /// (a WebSocket), or `Auto` (the web gateway where the server answered, else UDP).
    pub fn join_server(&mut self, address: &str, proto: JoinProto) {
        let info = self.server_info.get(address).cloned().and_then(|(_, r)| r.ok());
        let is_web = omsi_net::ws::ws_url(address).is_some() || omsi_net::official::is_alias(address);
        let proto = match (proto, &info) {
            (JoinProto::Auto, Some(_)) => JoinProto::WebSocket,
            (JoinProto::Auto, None) if !is_web => JoinProto::Udp,
            (p, _) => p,
        };
        let lan_addr = match proto {
            JoinProto::Udp => {
                if is_web {
                    self.set_status("UDP needs the game's address (host or host:port), not a web link", true);
                    return;
                }
                address.to_string()
            }
            _ => {
                let Some(info) = info.as_ref() else {
                    self.set_status("The server has not answered yet (is its address right? is it running?)", true);
                    return;
                };
                // (a server added by its bare address is joined where it answered: its web gateway)
                if !is_web && !info.reached_at.is_empty() { info.reached_at.clone() } else { address.to_string() }
            }
        };
        if let Some(info) = info.as_ref() {
            // A map not installed here comes with the server's mods when the game joins
            // (`lan_mods`), so this is a notice, not a refusal.
            // The Drive page needs a map of this installation chosen, so the choice stays
            // as it is then: `duty()` starts the game on the server's map anyway.
            if !self.maps.is_empty() && !self.maps.iter().any(|m| m.file.eq_ignore_ascii_case(&info.map)) {
                self.set_status(omsi_ui::tr("The server plays %{map}, which is not installed here: it is fetched from the server on joining.").replace("%{map}", &info.map), false);
            } else {
                self.choice.map = info.map.clone();
            }
        }
        self.choice.lan_mode = "join".into();
        self.choice.lan_addr = lan_addr;
        self.joined_server = Some(address.to_string());
        let name = info.as_ref().map(|i| i.name.clone()).unwrap_or_else(|| address.to_string());
        self.join = (true, omsi_ui::tr("the server %{name}").replace("%{name}", &name));
        self.join_checked = address.to_string();
        self.touched();
        let how = if proto == JoinProto::Udp { " over UDP" } else { "" };
        self.set_status(omsi_ui::tr("Joined %{name} - choose your bus and duty, then Start the duty").replace("%{name}", &format!("{name}{how}")), false);
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
        self.queued_launch = Some(d);
    }

    /// The game's object editor on `map`: a free drive there with the chosen bus, nothing of
    /// the timetable, the editor on once the world is loaded (`--editor`).
    pub fn launch_editor(&mut self, map: &str) {
        if !self.save_pending_settings() {
            return;
        }
        if !omsi_cfg::missing_original_essentials(std::path::Path::new(&self.config.root)).is_empty() {
            self.set_status("A session needs the original OMSI 2: choose its folder under Setup first.", true);
            return;
        }
        if self.choice.bus.trim().is_empty() {
            self.set_status("Choose a bus on the Drive page first: the editor starts with it on the map.", true);
            return;
        }
        let mut d = self.duty();
        if d.map != map {
            // (another map than the duty's: its first entry point, its own depot)
            d.entry = Some(0);
            d.hof = None;
        }
        d.map = map.to_string();
        (d.line, d.tour, d.trip, d.free_line, d.whole_tour) = (None, None, None, None, false);
        d.legs.clear();
        d.schedule = Some(false);
        d.traffic = Some(0);
        d.lan = Some("off".into());
        d.editor = true;
        self.set_status("Starting the object editor…", false);
        self.queued_launch = Some(d);
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
        // a composed duty: its parts; none chosen (or one that no longer holds) is a free drive
        let legs = self.composed_legs().map(|l| l.to_vec());
        let free = c.free || (c.composed && legs.is_none());
        // a free drive along a line the player chose: its line and route, the bus at the
        // entry point nearest to the route's first stop
        let free_route = super::freedrive::free_route(c);
        // (a duty of the timetable starts the lead before its first departure: the bus stands
        // ready, the player is not late at once; a free drive starts when it was asked to)
        let lead = if free { 0 } else { core::bus_lead(self.settings.get("bus_lead").and_then(|x| x.as_i64())) };
        let (date, time) = core::lead_start(&c.date, c.time, lead);
        core::Duty {
            map: host_map.unwrap_or_else(|| c.map.clone()),
            bus: c.bus.clone(),
            paint: Some(c.paint.clone()).filter(|p| !p.is_empty()),
            set_vars: self.bus_options.for_game(&self.config.root, &c.bus, &c.paint),
            display_font: self.display_fonts.font_for(&c.bus),
            plate: Some(c.plate.clone()).filter(|p| !p.trim().is_empty()),
            number: Some(c.number.clone()).filter(|n| !n.trim().is_empty()),
            hof: Some(if c.hof_manual { c.hof.clone() } else { self.default_hof() }).filter(|p| !p.is_empty()),
            entry: Some(if free_route.is_some() { -1 } else if c.free { c.free_entry } else { c.entry }),
            line: if let Some((line, _)) = &free_route { Some(line.clone()) } else if free { None } else { c.line.clone() },
            tour: if free { None } else { c.tour.clone() },
            trip: if free { None } else { self.picked_trip().map(|i| i.to_string()) },
            whole_tour: !free && self.picked_trip().is_some(),
            legs: legs.unwrap_or_default(),
            free_line: free_route.map(|r| r.1),
            time: format!("{:02}:{:02}", time / 60, time % 60),
            date: Some(date),
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
            editor: false,
        }
    }

    /// The situations to continue on the chosen map: the last one and the save slots
    /// (looked up at most every two seconds: the page asks every frame).
    pub fn saved_situations(&mut self) -> &[core::SavedSituation] {
        let fresh = self.last_sit.as_ref().is_some_and(|(m, _, t)| *m == self.choice.map && t.elapsed().as_secs_f32() < 2.0);
        if !fresh {
            if self.last_sit.as_ref().is_some_and(|(m, _, _)| *m != self.choice.map) {
                self.save_pick = 0;
            }
            let list = core::saved_situations(&self.choice.map);
            self.save_pick = self.save_pick.min(list.len().saturating_sub(1));
            self.last_sit = Some((self.choice.map.clone(), list, std::time::Instant::now()));
        }
        self.last_sit.as_ref().map(|x| x.1.as_slice()).unwrap_or(&[])
    }

    /// Whether a situation to continue lies on the chosen map.
    pub fn has_last_situation(&mut self) -> bool {
        !self.saved_situations().is_empty()
    }

    /// Continue the situation chosen of the map's (`laststn.osn`, or a save slot, #341).
    pub fn launch_last_situation(&mut self) {
        if !self.save_pending_settings() {
            return;
        }
        let pick = self.save_pick;
        let list = self.saved_situations();
        let Some(file) = list.get(pick).or_else(|| list.first()).map(|s| s.file.clone()) else {
            self.set_status("No situation left on this map yet", true);
            return;
        };
        let mut d = self.duty();
        d.situation = Some(file.to_string_lossy().to_string());
        d.lan = Some("off".into());
        self.set_status("Continuing where you left off…", false);
        self.queued_launch = Some(d);
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
        self.queued_launch = Some(d);
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
                self.set_status(omsi_ui::tr("Could not save settings: %{error}").replace("%{error}", &format!("{e:#}")), true);
                false
            }
        }
    }

    /// Something of the duty changed: remember it (soon) and refresh what depends on it.
    pub fn touched(&mut self) {
        self.choice_dirty = 0.4;
    }

    /// Work done each frame: results of background work, the regular poll, saving.
    fn follow_clock(&mut self) {
        let on = |k: &str| self.settings.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
        let (time, date, year) = (on("use_real_time"), on("use_real_date"), on("use_real_year"));
        if !time && !date {
            return;
        }
        let Some((y, mo, d, h, m)) = core::local_now() else { return };
        if time {
            self.choice.time = h * 60 + m;
        }
        if date {
            let y = if year { y } else { self.choice.date.get(..4).and_then(|x| x.parse().ok()).unwrap_or(y) };
            let today = format!("{y:04}-{mo:02}-{d:02}");
            if self.choice.date != today {
                self.choice.date = today;
                self.load_lines();
            }
        }
    }

    pub fn update(&mut self, dt: f32) {
        while let Ok(m) = self.rx.try_recv() {
            self.handle(m);
        }
        self.follow_clock();
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
                self.set_status(omsi_ui::tr("Reading the content stopped on an error: %{error}").replace("%{error}", &why), true);
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
                                self.set_status(omsi_ui::tr("The host drives on %{map}: that map is chosen").replace("%{map}", &name), false);
                            }
                        }
                    }
                }
                self.server_info.insert(address, (Instant::now(), info));
            }
            Msg::ContentEarly { maps, weathers } => {
                if !self.content_first {
                    return;
                }
                crate::mt::protect(maps.iter().flat_map(|m| [m.name.as_str(), m.friendly.as_str()]));
                crate::mt::protect(weathers.iter().map(|w| w.name.as_str()));
                self.maps = maps;
                self.weathers = weathers;
                self.pick_map();
                self.load_lines();
                self.set_status(omsi_ui::tr("%{maps} maps - reading the buses…").replace("%{maps}", &self.maps.len().to_string()), false);
            }
            Msg::VehiclesRead { batch, done, total } => {
                if !self.content_first {
                    return;
                }
                crate::mt::protect(batch.iter().flat_map(|v| [v.name.as_str(), v.manufacturer.as_str(), v.type_name.as_str()]).chain(batch.iter().flat_map(|v| v.paints.iter().map(|p| p.as_str()))));
                self.vehicles.extend(batch);
                self.set_status(omsi_ui::tr("%{maps} maps, %{buses} buses - reading the vehicle folders: %{done} of %{total}").replace("%{maps}", &self.maps.len().to_string()).replace("%{buses}", &self.vehicles.len().to_string()).replace("%{done}", &done.to_string()).replace("%{total}", &total.to_string()), false);
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
                self.set_status(omsi_ui::tr("%{maps} maps, %{buses} buses, %{weathers} weathers").replace("%{maps}", &maps.len().to_string()).replace("%{buses}", &vehicles.len().to_string()).replace("%{weathers}", &weathers.len().to_string()), false);
                self.maps = maps;
                self.vehicles = vehicles;
                self.weathers = weathers;
                self.loading_content = false;
                // (a bus may have had liveries added or taken away)
                self.bus_options.forget();
                // what was chosen last time, where it still exists
                if !self.vehicles.iter().any(|v| v.file == self.choice.bus) {
                    if let Some(v) = self.vehicles.first() {
                        self.choice.bus = v.file.clone();
                        self.choice.paint.clear();
                        self.choice.hof = self.default_hof();
                    }
                }
                self.pick_map();
                // (the first reading asked for the lines with the maps already)
                if !std::mem::take(&mut self.content_first) || self.lines_for != (self.choice.map.clone(), self.choice.date.clone()) {
                    self.load_lines();
                }
                self.load_args();
                self.load_ibis();
                self.content_done();
            }
            Msg::Content(Err(e)) => {
                self.loading_content = false;
                self.content_first = false;
                self.content_done();
                if omsi_cfg::missing_original_essentials(std::path::Path::new(&self.config.root)).is_empty() {
                    self.set_status(format!("{e}\n{}", omsi_ui::tr("Set the OMSI 2 folder under Setup.")), true);
                } else {
                    self.set_status(root_problem(&self.config.root), true);
                }
            }
            Msg::Lines { map, date, lines, own } => {
                if let Ok(ls) = lines.as_ref() {
                    crate::mt::protect(ls.iter().flat_map(|l| l.termini.iter().map(|t| t.as_str()).chain([l.name.as_str()])).chain(ls.iter().flat_map(|l| l.tours.iter().map(|t| t.number.as_str()))));
                }
                let captions: Vec<String> = own.iter().map(|o| o.caption()).collect();
                crate::mt::protect(own.iter().flat_map(|o| o.destinations.iter().map(|d| d.as_str()).chain([o.name.as_str()])).chain(captions.iter().map(|c| c.as_str())));
                if (map, date) != self.lines_for {
                    return;
                }
                self.loading_lines = false;
                self.own_lines = own;
                match lines {
                    Ok(l) => {
                        self.lines = l;
                        let mut note = String::new();
                        if let Some(line) = self.choice.line.clone() {
                            match self.lines.iter().find(|x| x.name == line) {
                                None => {
                                    note = format!(" - {}", omsi_ui::tr("line %{line} does not run on %{date}").replace("%{line}", &line).replace("%{date}", &self.choice.date));
                                    self.choice.line = None;
                                    self.choice.tour = None;
                                }
                                Some(l) => {
                                    if let Some(t) = &self.choice.tour {
                                        match l.tours.iter().find(|x| &x.number == t) {
                                            None => self.choice.tour = None,
                                            Some(t) if !t.runs => note = format!(" - {}", omsi_ui::tr("tour %{tour} of line %{line} does not run that day (%{days})").replace("%{tour}", &t.number).replace("%{line}", &line).replace("%{days}", &super::drive::days_text(&t.days))),
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                        self.set_status(format!("{}{note}", omsi_ui::tr("%{n} lines on %{date}").replace("%{n}", &self.lines.len().to_string()).replace("%{date}", &self.choice.date)), !note.is_empty());
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
                            self.set_status(omsi_ui::tr("Installing from the Mods folder: %{mods}").replace("%{mods}", &p.started.iter().map(|x| x.rsplit('/').next().unwrap_or(x)).collect::<Vec<_>>().join(", ")), false);
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
                                if let Some(why) = disconnect_of(std::path::Path::new(&old.log)) {
                                    core::log_to_file(&format!("game {} was sent away by its server: {why}", old.pid));
                                    self.disconnected = Some(why);
                                } else if let Some(c) = crash_of(std::path::Path::new(&old.log)) {
                                    core::log_to_file(&format!("game {} ended on an error: {}", old.pid, c.0));
                                    self.crash = Some(c);
                                }
                            }
                        }
                        self.instances = p.instances;
                        // the game started from here is in the list: whether it runs is known
                        // (one that ended at once left the launcher blank until the 15 s were out)
                        if game_listed(self.launched_pid, &self.instances) {
                            self.launch_hold = None;
                            self.launched_pid = None;
                        }
                        for i in &self.instances {
                            if !i.running {
                                self.stopping.remove(&i.pid);
                            }
                        }
                        // (while the lists are read the stamp moves by itself - the cache gains
                        // the folders each bus depends on - and every poll started the whole
                        // reading over on top of the one going: a big installation never
                        // finished. A change then is taken up when the reading is done.)
                        let changed = self.stamp.as_ref().map(|s| *s != p.stamp).unwrap_or(false);
                        self.stamp = if self.loading_content { None } else { Some(p.stamp) };
                        if changed || installed {
                            if self.loading_content {
                                self.reload_content = true;
                            } else {
                                self.load_content();
                            }
                            self.load_mods();
                        }
                        for pid in self.open_logs.clone() {
                            self.log_tail(pid);
                        }
                    }
                    Err(e) => core::log_to_file(&format!("poll: {e}")),
                }
            }
            Msg::Profile(Ok(p)) => {
                // A read started before another driver was selected must not put the
                // old profile back on screen after deletion.
                if p.name.eq_ignore_ascii_case(&self.config.profile) {
                    self.profile = Some(p);
                }
            }
            Msg::Profile(Err(e)) => {
                self.profile = None;
                core::log_to_file(&format!("profile: {e}"));
            }
            Msg::Profiles(p) => {
                self.profiles = p;
                if !self.profiles.contains(&self.config.profile) {
                    self.config.profile = self.profiles.first().cloned().unwrap_or_default();
                    self.profile = None;
                    let _ = core::save_config(&self.config);
                }
                if !self.config.profile.is_empty() {
                    self.load_profile();
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
                self.launched_pid = Some(l.pid);
                self.set_status(format!("{}{}", omsi_ui::tr("Game started (process %{pid}), log %{log}").replace("%{pid}", &l.pid.to_string()).replace("%{log}", &l.log), if l.others > 0 { format!(" - {}", omsi_ui::tr("%{n} other game(s) keep running").replace("%{n}", &l.others.to_string())) } else { String::new() }), false);
                self.poll_now();
            }
            Msg::Launched(Err(e)) => {
                self.launch_hold = None;
                self.set_status(e, true)
            }
            Msg::Stopped { pid, result } => {
                self.stopping.remove(&pid);
                match result {
                    Ok(true) => self.set_status(omsi_ui::tr("Game %{pid} ended by itself.").replace("%{pid}", &pid.to_string()), false),
                    Ok(false) => self.set_status(omsi_ui::tr("Game %{pid} did not end by itself and was killed - this run is not saved.").replace("%{pid}", &pid.to_string()), true),
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

    /// The depot file for the chosen bus: the one the map's own buses use on the chosen date
    /// (the chrono scenarios change it: Berlin's 1994 depot has line 137 where 1986's had
    /// 92), which the bus has, else its first.
    pub fn default_hof(&self) -> String {
        let want = self.map_hof();
        let Some(v) = self.bus() else { return want };
        // (the bus's own depot of the same place before the map's borrowed from another
        // bus, and one named like the map before its first, #896)
        let names: Vec<&str> = v.hofs.iter().map(|h| h.as_str()).collect();
        let like = |hints: &[&str]| omsi_vehicle::hof::closest_name(&names, hints).map(|i| v.hofs[i].clone());
        let map_hints: Vec<String> = self.map().map(|m| vec![m.name.clone(), m.friendly.clone(), m.file.trim_end_matches("/global.cfg").rsplit('/').next().unwrap_or("").to_string()]).unwrap_or_default();
        let map_hints: Vec<&str> = map_hints.iter().map(|h| h.as_str()).collect();
        // a depot file carrying the line driven: the day's, one named like the map, else the fullest
        let with_line = match (&self.choice.line, self.choice.free) {
            (Some(line), false) => core::depot_names_with_line(std::path::Path::new(&self.config.root), &v.file, line),
            _ => Vec::new(),
        };
        if !with_line.is_empty() && !with_line.iter().any(|h| h.eq_ignore_ascii_case(&want)) {
            let names: Vec<&str> = with_line.iter().map(|h| h.as_str()).collect();
            let hints: Vec<&str> = std::iter::once(want.as_str()).chain(map_hints.iter().copied()).collect();
            let pick = omsi_vehicle::hof::closest_name(&names, &hints).unwrap_or(0);
            return with_line[pick].clone();
        }
        v.hofs
            .iter()
            .find(|h| h.eq_ignore_ascii_case(&want))
            .cloned()
            .or_else(|| like(&[want.as_str()]))
            .or(Some(want.clone()).filter(|w| !w.is_empty()))
            .or_else(|| like(&map_hints))
            .or_else(|| v.hofs.first().cloned())
            .unwrap_or_default()
    }

    /// The depot file of the chosen map: the one its own buses use on the chosen date, else the
    /// one its ailists name ("" for a map without one) - what a bus needs beside it to know the
    /// map's destinations.
    pub fn map_hof(&self) -> String {
        let on_date = self.map().and_then(|m| {
            let dir = omsi_cfg::resolve_path(std::path::Path::new(&self.config.root), &m.file);
            omsi_map::ailists::depot_hof_on(dir.parent()?, omsi_map::ailists::date_code(&self.choice.date)?)
        });
        on_date.or_else(|| self.map().map(|m| m.hof.clone())).unwrap_or_default()
    }

    /// The timetable line the duty drives (as `duty` gives it the game): a free drive's line
    /// followed, else the duty's; None for a free drive without a line.
    pub fn driven_line(&self) -> Option<String> {
        let c = &self.choice;
        if let Some((line, _)) = super::freedrive::free_route(c) {
            return Some(line);
        }
        if c.free {
            None
        } else {
            c.line.clone()
        }
    }

    /// The player's own depot file (its key, of `keys`) the drive gives its bus
    /// (`owndepot::for_duty`): the one chosen on the bus step, else the one the driven line of
    /// his chose (`true`: the line's).
    pub fn own_depot_in_use(&self, keys: &[String]) -> Option<(String, bool)> {
        if let Some(k) = keys.iter().find(|k| k.eq_ignore_ascii_case(self.choice.hof.trim())) {
            return Some((k.clone(), false));
        }
        let line = self.driven_line()?;
        let o = self.own_lines.iter().find(|o| o.file.eq_ignore_ascii_case(line.trim()))?;
        keys.iter().find(|k| !o.depot_file.trim().is_empty() && k.eq_ignore_ascii_case(o.depot_file.trim())).map(|k| (k.clone(), true))
    }

    pub fn select_bus(&mut self, file: &str) {
        if self.choice.bus == file {
            return;
        }
        self.choice.bus = file.to_string();
        self.choice.paint.clear();
        self.choice.number.clear();
        // (a hand-picked depot file stays when the new bus has one of that name - and one of the
        // player's own, which any bus is given)
        let own = core::owndepot::path_of(&core::owndepot::dir(), &self.choice.hof).is_some();
        let keep = self.choice.hof_manual && (own || self.bus().is_some_and(|v| v.hofs.iter().any(|h| h.eq_ignore_ascii_case(&self.choice.hof))));
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
        // (the free drive's line and stop are the old map's)
        self.choice.free_line.clear();
        self.choice.free_route.clear();
        self.choice.free_stop.clear();
        self.choice.free_entry = -1;
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
        if let Some(i) = self.picked_trip() {
            if let Some(k) = t.trips.iter().position(|x| x.index == i) {
                return Some(k);
            }
        }
        let now = self.choice.time as f64 * 60.0;
        trip_index_at(t, now)
    }

    pub fn picked_trip(&self) -> Option<usize> {
        let (line, tour, index, time) = self.choice.start_trip.as_ref()?;
        // (at the real time the clock moves on, and the trip picked stays: the bus waits for it)
        (self.choice.line.as_ref() == Some(line) && self.choice.tour.as_ref() == Some(tour) && (*time == self.choice.time || self.real_time())).then_some(*index)
    }

    /// The start time follows the computer's clock (`use_real_time`).
    pub fn real_time(&self) -> bool {
        self.settings.get("use_real_time").and_then(|v| v.as_bool()).unwrap_or(false)
    }

    /// Start the tour at trip `index` (leaving at `departure`): at its departure, or at the
    /// real time, which then stays and the bus waits for the trip.
    pub fn pick_trip(&mut self, index: usize, departure: f64) {
        let (Some(line), Some(tour)) = (self.choice.line.clone(), self.choice.tour.clone()) else { return };
        if !self.real_time() {
            self.choice.time = (departure / 60.0).floor() as i32;
        }
        self.choice.start_trip = Some((line, tour, index, self.choice.time));
        self.touched();
    }

    /// The duties put together of the day's tours for the chosen length and part of the day
    /// (made again when one of them changes, when the day's lines are read anew, or for
    /// "Other shifts"). Making them takes a millisecond: no thread for it.
    pub fn composed_duties(&mut self) -> &[core::compose::ComposedDuty] {
        let c = &self.choice;
        let key = (self.lines_for.0.clone(), self.lines_for.1.clone(), c.duty_minutes, c.daypart, self.compose_seed, self.lines.len());
        if self.composed_for.as_ref() != Some(&key) {
            let window = DAYPARTS.get(c.daypart).and_then(|d| d.1);
            let wish = core::compose::Wish {
                seconds: c.duty_minutes as f64 * 60.0,
                from: window.map(|w| w.0 as f64 * 60.0),
                to: window.map(|w| w.1 as f64 * 60.0),
                lines: Vec::new(),
                seed: self.compose_seed,
            };
            self.composed = core::compose::compose(&self.lines, &wish, 8);
            self.composed_for = Some(key);
        }
        &self.composed
    }

    /// Takes a composed duty: its first part is the line, the tour and the trip the game
    /// starts with, at that trip's departure.
    pub fn pick_composed(&mut self, d: &core::compose::ComposedDuty) {
        let blocks = d.blocks();
        let Some(first) = blocks.first() else { return };
        let time = (first.departure / 60.0).floor() as i32;
        self.choice.line = Some(first.line.clone());
        self.choice.tour = Some(first.tour.clone());
        self.choice.time = time;
        self.choice.start_trip = Some((first.line.clone(), first.tour.clone(), first.first, time));
        self.choice.legs = blocks.iter().map(|b| b.arg()).collect();
        self.touched();
    }

    /// The parts of the composed duty chosen while it still holds: its start as picked (not
    /// moved by another time, line or tour, nor taken by a trip picked in the roadbook of a
    /// tour since) and each part's tour running on the day.
    pub fn composed_legs(&self) -> Option<&[String]> {
        let c = &self.choice;
        if c.free || !c.composed || c.legs.is_empty() {
            return None;
        }
        let first = core::compose::Block::parse(&c.legs[0]);
        let picked = self.picked_trip().zip(c.start_trip.as_ref());
        if !matches!((first, picked), (Some((line, tour, k, _)), Some((i, (l, t, ..)))) if line == *l && tour == *t && k == i) {
            return None;
        }
        let runs =|leg: &String| core::compose::Block::parse(leg).is_some_and(|(line, tour, ..)| self.lines.iter().any(|l| l.name == line && l.tours.iter().any(|t| t.number == tour && t.runs)));
        c.legs.iter().all(runs).then_some(c.legs.as_slice())
    }
}

/// Whether the game started from here (its process `pid`) is in a list of games.
fn game_listed(pid: Option<u32>, instances: &[core::Instance]) -> bool {
    pid.is_some_and(|p| instances.iter().any(|i| i.pid == p))
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
        omsi_ui::tr("%{folder} does not exist: choose the folder of the original OMSI 2 (with Omsi.exe, maps and Vehicles in it) under Setup.").replace("%{folder}", root)
    } else if missing.iter().any(|m| m.contains("content folder")) || p.join("openomsi.exe").exists() || p.join("openomsi").is_file() {
        omsi_ui::tr("%{folder} is openOMSI's own folder, not OMSI 2's: choose the folder of the original game (with Omsi.exe in it) under Setup.").replace("%{folder}", root)
    } else if missing.is_empty() {
        String::new()
    } else {
        omsi_ui::tr("%{folder} is not a complete OMSI 2 - it lacks %{missing}. openOMSI plays on the original's stock content: choose the folder of a complete installation under Setup.").replace("%{folder}", root).replace("%{missing}", &missing.iter().take(3).cloned().collect::<Vec<_>>().join(", "))
    }
}

/// The server's message when a game ended because its server sent it away or turned it away
/// (`lan::LEFT_SERVER` in the log of this run); None for any other end.
pub fn disconnect_of(log: &std::path::Path) -> Option<String> {
    let text = std::fs::read(log).ok()?;
    let text = String::from_utf8_lossy(&text[text.len().saturating_sub(64 * 1024)..]).to_string();
    let all: Vec<&str> = text.lines().collect();
    let run = &all[all.iter().rposition(|l| l.contains("starting the game:")).unwrap_or(0)..];
    let line = run.iter().rev().find(|l| l.contains(crate::lan::LEFT_SERVER))?;
    let why = line.split_once(crate::lan::LEFT_SERVER)?.1.trim();
    Some(if why.is_empty() { "sent away by the host".to_string() } else { why.chars().take(300).collect() })
}

/// What a game's log says when the game ended on an error: the error (a panic, "no graphics
/// adapter", a fatal message) and the last lines of the log. None for a game that ended as
/// it should.
pub fn crash_of(log: &std::path::Path) -> Option<(String, String)> {
    let bytes = std::fs::read(log).ok()?;
    let text = String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(64 * 1024)..]).to_string();
    let all: Vec<&str> = text.lines().collect();
    // (the run itself only: an error the launcher logged before the game started - a
    // preview's picture left out - titled the report of a game that died much later, and
    // the phone's "died compiling a shader" hint never showed, #381, #331)
    let lines: Vec<&str> = match all.iter().rposition(|l| l.contains("starting the game:")) {
        Some(k) => all[k..].to_vec(),
        None => all.clone(),
    };
    // (an error the game got over - "the game goes on", a part of the picture left out -
    // is no crash)
    let recovered = |l: &str| l.contains("the game goes on") || l.contains("left out") || l.contains("could not be recorded");
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
    let at = lost.or_else(|| lines.iter().rposition(|l| l.contains("the game stopped on an error") || (l.contains(" ERROR ") && !recovered(l))))?;
    let first = lines[at].split_once("] ").map(|x| x.1).unwrap_or(lines[at]).trim();
    // (a panic's message is on the following lines)
    let mut what = first.to_string();
    // (and stops at the next record: a lost device's line had "game ends" and the tile
    // loading after it in the report's title, #1187)
    for l in lines.iter().skip(at + 1).take(6) {
        if l.trim().is_empty() || l.trim_start().starts_with("0:") || l.starts_with('[') {
            break;
        }
        what.push(' ');
        what.push_str(l.trim());
    }
    // the computer, its graphics card and the command line (the map) from the start of the
    // run, before the end of the log and a line `…`: a report of the end alone never said
    // what it happened on (#1187). Its GitHub link keeps them and shortens only the end.
    let whole = String::from_utf8_lossy(&bytes);
    let machine: Vec<&str> = MACHINE_LINES.iter().filter_map(|k| whole.lines().rfind(|l| l.contains(k))).collect();
    let end = all[all.len().saturating_sub(150)..].join("\n");
    let tail = if machine.is_empty() { end } else { format!("{}\n{CRASH_TAIL_GAP}\n{end}", machine.join("\n")) };
    Some((what.chars().take(600).collect(), tail))
}

/// The log lines that tell what a game ran on (see `crash_of`).
const MACHINE_LINES: [&str; 4] = ["] system: ", "] graphics adapter: ", "] opening graphics device: ", "] command line: "];

/// The line between the machine and the end of the log in a crash's tail.
pub const CRASH_TAIL_GAP: &str = "…";

#[cfg(test)]
mod disconnect_tests {
    #[test]
    fn the_servers_word_is_read_back_from_the_log_of_this_run() {
        let dir = std::env::temp_dir().join(format!("openomsi-disc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("game.log");
        let left = crate::lan::LEFT_SERVER;
        std::fs::write(&p, format!("[x INFO a] starting the game: one\n[x WARN openomsi_game::lan] {left}old\n[x INFO a] starting the game: two\n[x WARN openomsi_game::lan] {left}Expulsé : conduite dangereuse\n[x INFO a] game ends\n")).unwrap();
        assert_eq!(super::disconnect_of(&p).as_deref(), Some("Expulsé : conduite dangereuse"));
        // a run that ended in any other way: nothing
        std::fs::write(&p, format!("[x INFO a] starting the game: one\n[x WARN openomsi_game::lan] {left}old\n[x INFO a] starting the game: two\n[x INFO a] game ends\n")).unwrap();
        assert!(super::disconnect_of(&p).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod choice_tests {
    /// `launcher-duty.json` from before the number plate field: the missing key falls back to
    /// the default (no plate), and a typed plate survives a round trip.
    #[test]
    fn a_remembered_join_is_not_resumed_at_launch() {
        let saved: super::Choice = serde_json::from_str(r#"{"lan_mode":"join","lan_addr":"main.example.org"}"#).unwrap();
        assert_eq!(saved.lan_mode, "join");
        assert_eq!(super::Choice::fresh(saved).lan_mode, "off");
        let host: super::Choice = serde_json::from_str(r#"{"lan_mode":"host"}"#).unwrap();
        assert_eq!(super::Choice::fresh(host).lan_mode, "host");
    }

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
mod launch_tests {
    use omsi_launcher_lib::Instance;

    fn game(pid: u32, running: bool) -> Instance {
        Instance { pid, running, ..Default::default() }
    }

    /// The wait after Start ends with the first list of games that has the one started,
    /// running or ended already - a game that died at once left the launcher blank for the
    /// whole 15 seconds.
    #[test]
    fn the_wait_after_start_ends_once_the_game_is_listed() {
        // (a list asked for before the game was started)
        assert!(!super::game_listed(Some(7), &[game(3, true)]));
        assert!(super::game_listed(Some(7), &[game(3, true), game(7, true)]));
        assert!(super::game_listed(Some(7), &[game(7, false)]));
        // (Start pressed, the game not started yet)
        assert!(!super::game_listed(None, &[game(7, true)]));
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
        // an error before the game started, or one it got over, is not the crash
        std::fs::write(&p, "[t ERROR omsi_render] a part of the picture could not be recorded (left out)\n[t INFO x] starting the game: omsi\n[t INFO omsi_render] renderer: compiling the sky and clouds shaders\n").unwrap();
        assert!(super::crash_of(&p).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The report says what the game ran on (the start of the log), and its title is the
    /// error alone, not the records after it (#1187).
    #[test]
    fn the_report_has_the_computer_and_a_clean_title() {
        let dir = std::env::temp_dir().join(format!("openomsi-crash-machine-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("game.log");
        let mut log = String::from("[t INFO openomsi_game::applog] system: windows x86_64 (10.0), 8 threads, 8192 MB memory\n[t INFO openomsi_game::applog] command line: openomsi --map maps/X/global.cfg\n[t INFO omsi_render] graphics adapter: GTX 750 (DiscreteGpu, Dx12, 2048 MB of its own), texture memory taken for it: 716 MB\n");
        // (more than the end that goes with the report)
        for k in 0..400 {
            log.push_str(&format!("[t INFO openomsi_game::scene] tile loading: placed tile {k},0\n"));
        }
        log.push_str("[t ERROR openomsi_game::app_events] ending the session: the graphics device was lost (Unknown: Out of memory)\n[t INFO openomsi_game::app_events] game ends\n[t WARN openomsi_game::scene] tile loading: first-area batch prepared in 352.71 s\n");
        std::fs::write(&p, log).unwrap();
        let (what, tail) = super::crash_of(&p).unwrap();
        assert_eq!(what, "ending the session: the graphics device was lost (Unknown: Out of memory)");
        let (machine, end) = tail.split_once(&format!("\n{}\n", super::CRASH_TAIL_GAP)).unwrap();
        assert!(machine.contains("8192 MB memory") && machine.contains("GTX 750") && machine.contains("maps/X/global.cfg"), "{machine}");
        assert!(end.contains("352.71 s") && !end.contains("placed tile 0,0"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

fn read_settings_file() -> Option<String> {
    std::fs::read_to_string(core::data_dir().join("settings.cfg")).ok()
}
