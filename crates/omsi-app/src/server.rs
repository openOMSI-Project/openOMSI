//! The dedicated server (`omsi --server server.cfg`): a session host that is always on, like
//! a Minecraft server - no window, no sound, no graphics card (the renderer runs on wgpu's
//! no-op device: the world, the traffic, the timetable and the people are simulated as the
//! host's game simulates them, and nothing is drawn). Players reach it at `host:port` (UDP)
//! or at its web address over a WebSocket (`omsi_net::ws`), which is what a free Cloudflare
//! tunnel carries: `tunnel = 1` starts one and prints the `https://….trycloudflare.com`
//! address to add in the launcher's Multiplayer → Servers.
//!
//! The folder of `server.cfg` is the server's: `server-icon.png` beside it is its icon (a
//! 64x64 PNG, like Minecraft's), `server.log` its log. A missing `server.cfg` is written with
//! the defaults and a comment for every key.

use super::*;
use std::path::Path;

/// What `server.cfg` says (see `DEFAULT_CFG`).
#[derive(Debug, Clone)]
pub(crate) struct ServerCfg {
    pub name: String,
    pub motd: String,
    pub map: String,
    pub date: Option<String>,
    pub time: String,
    pub weather: Option<String>,
    pub traffic: usize,
    pub timetable: bool,
    pub passengers: bool,
    pub port: u16,
    pub web_port: u16,
    pub max_players: usize,
    pub tunnel: bool,
    pub radius: i32,
    pub icon: Vec<u8>,
    /// Players who say `/admin <password>` in the chat administer the server (empty: nobody).
    pub admin_password: String,
    /// How fast the server's clock runs (1 real time).
    pub time_speed: f64,
    /// Only these buses may be driven on the server (vehicle files, empty: every bus the
    /// server has installed).
    pub vehicles: Vec<String>,
    /// `GET /players` on the web port tells who drives what and where (for a web map).
    pub share_positions: bool,
}

pub(crate) const DEFAULT_CFG: &str = "\
# openOMSI dedicated server
# (key = value; lines starting with # are comments)

# shown in the players' server list and when they join
name = openOMSI server
motd = Welcome! Drive safely.

# the map (relative to the OMSI 2 folder), the start date (YYYY-MM-DD, empty: today),
# the time of day and the weather (a .owt of the OMSI 2 folder, empty: the map's default,
# cycle: one after another through the day, as the month allows)
map = maps/Berlin-Spandau/global.cfg
date =
time = 08:00
weather =

# the shared world: random traffic (cars), timetable buses, waiting passengers
traffic = 30
timetable = 1
passengers = 1

# UDP port of the session, TCP port of the web gateway (status, icon, WebSocket players)
port = 27015
web_port = 27025
max_players = 16

# start a free Cloudflare quick tunnel (needs cloudflared) and print its https address
tunnel = 1

# how many tiles round the map's first entry point are kept loaded (0: all of them)
radius = 0

# players who say \"/admin <password>\" in the chat get the Administration menu (Esc):
# send players away, bring them, the clock, its speed, the weather (empty: nobody)
admin_password =

# how fast the clock runs (1 = real time, 2 = twice as fast, up to 30)
time_speed = 1

# the buses players may drive, separated by ; (vehicle files such as
# Vehicles/MAN_SD200/MAN_SD77.bus; empty: every bus installed on the server)
vehicles =

# tell anyone who asks the web port (GET /players) the players' names, buses, lines and
# positions - for a live map of the server on a website; tell your players when it is on
share_positions = 0
";

impl ServerCfg {
    pub(crate) fn load(path: &Path) -> Result<ServerCfg> {
        if !path.is_file() {
            std::fs::write(path, DEFAULT_CFG).with_context(|| format!("writing {}", path.display()))?;
            log::info!("server: {} did not exist; written with the defaults", path.display());
        }
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut kv: std::collections::HashMap<String, String> = Default::default();
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = l.split_once('=') {
                kv.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
            }
        }
        let get = |k: &str, d: &str| kv.get(k).cloned().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string());
        let flag = |k: &str, d: bool| kv.get(k).map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")).unwrap_or(d);
        let num = |k: &str, d: i64| kv.get(k).and_then(|v| v.parse::<i64>().ok()).unwrap_or(d);
        let dir = path.parent().unwrap_or(Path::new("."));
        let icon = std::fs::read(dir.join("server-icon.png")).ok().filter(|b| b.starts_with(b"\x89PNG") && b.len() < 256 * 1024).unwrap_or_default();
        Ok(ServerCfg {
            name: get("name", "openOMSI server"),
            motd: get("motd", ""),
            map: get("map", "maps/Berlin-Spandau/global.cfg"),
            date: kv.get("date").cloned().filter(|v| !v.is_empty()),
            time: get("time", "08:00"),
            weather: kv.get("weather").cloned().filter(|v| !v.is_empty()),
            traffic: num("traffic", 30).clamp(0, 200) as usize,
            timetable: flag("timetable", true),
            passengers: flag("passengers", true),
            port: num("port", 27015).clamp(1, 65535) as u16,
            web_port: num("web_port", 27025).clamp(1, 65535) as u16,
            max_players: num("max_players", 16).clamp(1, 64) as usize,
            tunnel: flag("tunnel", true),
            radius: num("radius", 0) as i32,
            icon,
            admin_password: kv.get("admin_password").cloned().unwrap_or_default(),
            time_speed: kv.get("time_speed").and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(1.0).clamp(1.0, 30.0),
            vehicles: kv.get("vehicles").map(|v| v.split(';').map(|x| x.trim().replace('\\', "/")).filter(|x| !x.is_empty()).collect()).unwrap_or_default(),
            share_positions: flag("share_positions", false),
        })
    }
}

/// The server's status as the web gateway tells it (see `omsi_net::ws::ServerInfo`).
pub(crate) fn info_of(cfg: &ServerCfg) -> omsi_net::ws::ServerInfo {
    omsi_net::ws::ServerInfo {
        name: cfg.name.clone(),
        motd: cfg.motd.clone(),
        map: cfg.map.clone(),
        players: 0,
        max_players: cfg.max_players,
        version: format!("{} ({})", env!("CARGO_PKG_VERSION"), BUILD),
        icon: cfg.icon.clone(),
        time: cfg.time.clone(),
        weather: cfg.weather.clone().unwrap_or_default(),
        password: false,
        vehicles: cfg.vehicles.clone(),
        reached_at: String::new(),
        players_public: cfg.share_positions,
        player_list: Vec::new(),
    }
}

/// Set up the arguments for a server run from `server.cfg` (the rest is the offscreen
/// host loop, see `offscreen::run_offscreen`).
pub(crate) fn prepare(args: &mut Args, path: &Path) -> Result<ServerCfg> {
    let cfg = ServerCfg::load(path)?;
    SERVER_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = SERVER_ADMIN.set((cfg.admin_password.clone(), cfg.time_speed));
    let _ = SERVER_VEHICLES.set(cfg.vehicles.clone());
    args.map = cfg.map.clone();
    args.time = cfg.time.clone();
    if let Some(d) = &cfg.date {
        args.date = Some(d.clone());
    }
    args.weather = cfg.weather.clone();
    args.traffic = cfg.traffic;
    args.schedule = cfg.timetable;
    args.passengers = cfg.passengers;
    args.lan_host = Some(cfg.port);
    args.lan_join = None;
    args.lan_name = cfg.name.clone();
    args.bus = None;
    args.radius = Some(if cfg.radius <= 0 { 99 } else { cfg.radius });
    args.size = "64x36".into();
    if args.offscreen.is_none() {
        args.offscreen = Some(std::env::temp_dir().join("omsi-server-unused.png"));
    }
    log::info!("server '{}': map {}, {} at {}, traffic {}, timetable {}, passengers {}, UDP {} / web {}, at most {} players", cfg.name, cfg.map, cfg.date.as_deref().unwrap_or("today"), cfg.time, cfg.traffic, cfg.timetable, cfg.passengers, cfg.port, cfg.web_port, cfg.max_players);
    Ok(cfg)
}

/// The buses a dedicated server allows (`vehicles`; empty: every bus it has).
pub(crate) static SERVER_VEHICLES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// A dedicated server's admin password and clock speed (for the host loop).
pub(crate) static SERVER_ADMIN: std::sync::OnceLock<(String, f64)> = std::sync::OnceLock::new();

/// A dedicated server run: graphics without a device, the whole world by interest.
pub(crate) static SERVER_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Every second of a server run: what the status page says (players, time, weather) and
/// who is where (`GET /players`, when `share_positions` is on).
pub(crate) fn tick_status(lan: &omsi_net::LanSession, time: f64, weather: &str) {
    let players = lan.peers().filter(|p| p.has_info).count();
    // (the admin's clock shift may take the time below 0 or past midnight: 23:08 had come
    // out as "00:-52")
    crate::lan::update_server_info(players, &crate::schedule::hhmm(time.rem_euclid(86400.0)), weather);
    let list = lan
        .peers()
        .filter(|p| p.has_info && p.has_pose)
        .map(|p| {
            let q = &p.pose;
            omsi_net::ws::PlayerInfo {
                id: q.id,
                name: q.name.clone(),
                bus: q.bus.clone(),
                line: q.line.clone(),
                destination: q.destination.clone(),
                tour: q.tour.clone(),
                x: q.x,
                y: q.y,
                heading: q.heading,
                speed_kmh: q.speed_kmh,
                lat_lon: omsi_map::world_to_lat_lon(q.x, q.y),
            }
        })
        .collect();
    crate::lan::update_server_players(list);
}
