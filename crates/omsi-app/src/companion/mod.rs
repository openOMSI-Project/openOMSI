//! The phone and tablet companion: Omsi-Hub's phone, in the game.
//!
//! Omsi-Hub (the Electron app beside OMSI 2) had a phone in its overlay and the same phone on
//! a real phone or tablet: the driver signs on with a personnel number and a code, signs the
//! duty order, and then has the bus's screens - the IBIS, the ticket machine - as a
//! touchscreen in the hand. This module does the same in openOMSI, with the game itself as
//! the source: it knows the duty (`App::duty`), the duty menu (`game_lists`' "Line and
//! tour...") and draws every screen of the bus itself.
//!
//! * Signing on ([`signon`]): number, then code, ten tries a minute; then the duty order to
//!   sign, or - without a duty - the duty menu, where picking a tour takes it on (the game
//!   types the IBIS for it, `game_lists::start_duty_at`). Signing a duty the game already had
//!   (the launcher's) types its IBIS when the game has not done so yet: the codes come after
//!   the signature, as at a depot. The state is the game's, not a device's: the navigator
//!   and every device see the same driver signed on (see [`state`] and the functions after
//!   it, for the navigator).
//! * The server ([`server`], [`http`], [`pairing`]): on the home network only when the
//!   setting `companion=1` is in `settings.cfg` (off by default; `companion_port`, default
//!   47811), behind a pairing code, serving the page in `assets/companion` (built into the
//!   game) and a small JSON interface.
//! * The screens ([`screens`]): the textures the inside of the bus shows, sent when watched,
//!   and taps and keys worked through the cab's own click handling; and each screen's device
//!   (the screen with its keys) as a live picture the game takes of it ([`device`],
//!   [`draw_devices`]), a tap on which clicks into the cab where the picture shows it. A
//!   device made of pages (an ALMEX, [`panels`]) is its display photographed straight on, at a
//!   tablet's size, shown on the whole screen.
//! * The devices drawn by the page ([`form`]): a device is sent once as its form - the
//!   triangles of its face laid flat, with their textures - and the page draws it itself,
//!   sharp and straight on the whole screen, from what lives on it (which parts show, the
//!   moving ones, the strings of its text textures, written in the bus's own fonts); a tap
//!   clicks the switch under the finger (`Player::click_mesh`). The live picture is left for
//!   a device of which no form could be made.
//! * Streaming: the address, the pairing code and the QR code are covered wherever the game
//!   shows them (`companion_hide=1`, the default) until "Show" uncovers them for a while
//!   ([`reveal`]).
//! * The Cloudflare tunnel (`companion_tunnel=1`, off by default): a quick tunnel to the
//!   server's port makes the page reachable from anywhere at an `https://….trycloudflare.com`
//!   address, shown instead of the home network's; the pairing is stricter then.
//! * The navigator ([`nav`]): a phone or tablet is the game's navigator as well - the map
//!   with the route, the bus, its stops and the traffic, the next turn and stop, the duty
//!   board and sheet, the IBIS codes of the duty, and a report at the end of each trip.
//! * Pairing by QR code ([`qr`], [`pair_qr`]): the page's address with the pairing code in
//!   it, scanned with a camera, pairs a device without typing.
//!
//! * The company ([`company`]): the bus company the launcher has open, on its own tab - money,
//!   today's dispositions, the depot and its workshop - and the few orders a paired device may
//!   send for it (queued for the launcher, which owns the company).
//!
//! The game calls [`frame`] once a frame; everything a device asks waits for that.

mod company;
mod device;
mod form;
mod http;
mod nav;
mod pairing;
mod panels;
mod qr;
mod screens;
mod server;
mod signon;

pub(crate) use nav::{NavLook, NavPin, RouteNote};
pub(crate) use signon::{Attempt, Stage};

use crate::App;
use glam::Vec3;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// The keys in `settings.cfg`: the companion on (`1`) or off (the default), and its port.
pub(crate) const SETTING: &str = "companion";
pub(crate) const PORT_SETTING: &str = "companion_port";
/// The address, pairing code and QR code covered on the screen (`1`, the default: a stream
/// shows them to everyone), and the Cloudflare tunnel (`0`, the default).
pub(crate) const HIDE_SETTING: &str = "companion_hide";
pub(crate) const TUNNEL_SETTING: &str = "companion_tunnel";
/// "Show" uncovers the address and the code this long.
const REVEAL_FOR: Duration = Duration::from_secs(30);
/// The tunnel's process id file (`~/.openomsi/…`), apart from the multiplayer gateway's.
const TUNNEL_PID: &str = "companion-cloudflared.pid";
/// Not Omsi-Hub's 47810: both may run on one computer.
pub(crate) const DEFAULT_PORT: u16 = 47811;

/// How often `settings.cfg` is looked at for a change of the setting.
const CONFIG_EVERY: Duration = Duration::from_secs(2);
/// A server that could not start is tried again after this long.
const RETRY_AFTER: Duration = Duration::from_secs(30);
/// The state for the devices is made at most this often (the clock in it moves on).
const PUBLISH_EVERY: Duration = Duration::from_millis(500);
/// A tap or key goes into the cab from this far before the screen (m), along its normal...
const REACH: f32 = 0.08;
/// ...within this cone (radians) round the ray.
const SPREAD: f32 = 0.004;
/// A switch held down from a device longer than this is let go (its release got lost).
const HELD_FOR: Duration = Duration::from_secs(10);
/// The bus's screens are looked for again this long after it came.
const LOOK_AGAIN: Duration = Duration::from_secs(3);
/// The navigator's live picture is made this often while a device shows the map.
const NAV_EVERY: Duration = Duration::from_millis(200);
/// Other vehicles on a device's map: within this far of the bus (m), this many at most.
const TRAFFIC_REACH: f64 = 700.0;
const TRAFFIC_MAX: usize = 80;

/// The companion's part of `settings.cfg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Config {
    pub enabled: bool,
    pub port: u16,
    /// The address and the codes covered on the screen.
    pub hide: bool,
    /// A Cloudflare quick tunnel to the server.
    pub tunnel: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { enabled: false, port: DEFAULT_PORT, hide: true, tunnel: false }
    }
}

impl Config {
    /// From the text of `settings.cfg` (`key=value` lines, as `Settings::from_text` reads
    /// them; anything missing keeps its default).
    pub(crate) fn from_text(text: &str) -> Config {
        let mut c = Config::default();
        for line in text.lines() {
            let Some((k, v)) = line.trim().split_once('=') else { continue };
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_ascii_lowercase());
            let on = matches!(v.as_str(), "1" | "true" | "on" | "yes");
            if k == SETTING {
                c.enabled = on;
            } else if k == HIDE_SETTING {
                // (covered unless asked not to be)
                c.hide = !matches!(v.as_str(), "0" | "false" | "off" | "no");
            } else if k == TUNNEL_SETTING {
                c.tunnel = on;
            } else if k == PORT_SETTING {
                c.port = v.parse::<u16>().ok().filter(|p| *p >= 1024).unwrap_or(DEFAULT_PORT);
            }
        }
        c
    }
}

/// The companion as the navigator (and anything else in the game) sees it.
// (for the navigator, which is to show it: the game reads only a part of it so far)
#[allow(dead_code)]
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CompanionState {
    /// `companion=1` in the settings.
    pub enabled: bool,
    /// The port the server listens on, while it does.
    pub listening: Option<u16>,
    /// Where a phone opens the page (`http://192.168.1.20:47811/`), the home network's first;
    /// the Cloudflare tunnel's alone (`https://….trycloudflare.com`) while it runs.
    pub addresses: Vec<String>,
    /// The code a device pairs with (empty while the server is off).
    pub pairing_code: String,
    /// The address, the code and the QR code are to be covered now (the setting, and "Show"
    /// not pressed lately), and the setting itself.
    pub hidden: bool,
    pub hide: bool,
    /// The Cloudflare tunnel, when it is asked for.
    pub tunnel: Option<TunnelState>,
    /// Devices that asked something lately.
    pub devices: Vec<DeviceInfo>,
    /// Devices paired.
    pub paired: usize,
    /// Why the server is not running although it should.
    pub error: Option<String>,
    /// The driver, and their personnel number and code (the code is for the game's own
    /// screens, the duty menu: a device never gets it).
    pub driver: String,
    pub personnel_number: String,
    pub personnel_code: String,
    pub stage: Stage,
    pub signed_on: bool,
    pub accepted: bool,
    pub free: bool,
    /// Signing on with the number and the code is not asked for (the setting `nav_signon`
    /// off): signed on and signed for by itself, nothing to sign off from.
    pub auto_sign_on: bool,
    /// When the break began (seconds of the day, the game's clock).
    pub break_since: Option<f64>,
    /// The duty the game has (the duty order the driver signs), if any.
    pub duty: Option<DutyOrder>,
    /// The bus's screens: id and name.
    pub screens: Vec<(String, String)>,
    /// The duty menu as the navigator asked for it ([`request`]).
    pub menu: DutyMenu,
}

/// How far the Cloudflare tunnel is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TunnelState {
    /// cloudflared runs and has not said its address yet.
    Starting,
    /// Reachable at this address.
    Ready(String),
    /// There is no cloudflared (the launcher's settings fetch it).
    Missing,
    /// It could not start, or it ended (tried again in a while).
    Failed,
}

/// What the navigator's sign-on page asks of the game that needs the game itself (`App`,
/// which the navigator is drawn without): done at the start of the next frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Request {
    /// Sign the duty order ([`accept_duty`]).
    Accept,
    /// Start or end a break ([`set_break`]).
    Break(bool),
    /// The duty menu's lines, afresh.
    Lines,
    /// The tours of the menu's line with this number.
    Tours(usize),
    /// Take on this tour (its number) of this line (its number): picking it is signing it.
    Pick(usize, usize),
}

/// The duty menu ("Line and tour...") as the navigator's page has it: its own copy, so that a
/// phone choosing at the same time does not change what the numbers on the page mean.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct DutyMenu {
    /// The lines (name, label); None until they are there.
    pub lines: Option<Vec<(String, String)>>,
    /// The tours of the line with this number: what ("Tour 3 › Hafen") and when ("05:42").
    pub tours: Option<(usize, Vec<(String, String)>)>,
    /// The last tour picked did not become the duty.
    pub failed: bool,
}

/// What the duty order says (Omsi-Hub's `DienstOpdracht`).
#[allow(dead_code)]
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct DutyOrder {
    pub line: String,
    pub tour: String,
    /// The line numbers its trips carry ("5 / 5E").
    pub lines: String,
    /// When it starts and ends (seconds of the day).
    pub start: f64,
    pub end: f64,
    pub trips: usize,
}

impl DutyOrder {
    fn of(d: &crate::schedule::PlayerDuty) -> DutyOrder {
        let mut lines: Vec<&str> = Vec::new();
        for l in d.trips.iter().map(|t| t.line.trim()).filter(|l| !l.is_empty()) {
            if !lines.contains(&l) {
                lines.push(l);
            }
        }
        DutyOrder {
            line: d.line.trim().to_string(),
            tour: d.tour.trim().to_string(),
            lines: lines.join(" / "),
            start: d.trips.first().map_or(0.0, |t| t.departure),
            end: d.trips.last().map_or(0.0, |t| t.end),
            trips: d.trips.len(),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DeviceInfo {
    /// What it is ("Safari on iPad").
    pub name: String,
    /// Seconds since it last asked something.
    pub seconds_ago: f32,
    /// The screen it watches, if any.
    pub screen: Option<String>,
}

struct Companion {
    config: Config,
    config_seen: Option<(Instant, Option<SystemTime>)>,
    shared: Arc<server::Shared>,
    server: Option<server::Server>,
    /// The settings the running server was started with.
    running: Config,
    /// The last start that failed: with which settings, when, and why.
    failed: Option<(Config, Instant, String)>,
    /// The way to the devices was told on the screen.
    told: bool,
    personnel: signon::PersonnelFile,
    driver: Option<(String, String, signon::Personnel)>,
    phone: signon::SignOn,
    /// The duty the game has, as its order says it.
    duty: Option<DutyOrder>,
    /// The bus whose screens these are (its type), and its screens.
    bus: usize,
    screens: Vec<screens::Screen>,
    /// When the screens are looked for once more.
    look_again: Option<Instant>,
    /// The last list of lines offered, and of tours (of which line).
    lines: Vec<String>,
    tours: (Option<usize>, Vec<(String, String)>),
    published: Option<Instant>,
    /// The navigator's requests, its duty menu, and the line and tour behind each tour of it.
    requests: Vec<Request>,
    menu: DutyMenu,
    menu_tours: Vec<(String, String)>,
    /// The navigator as the devices see it (see [`NavCache`]).
    nav: NavCache,
    /// The trip followed for its report, and the IBIS codes of the duty (what they were
    /// worked out for, and the codes).
    watch: nav::TripWatch,
    ibis: Option<(String, Value)>,
    /// A duty came that signed for itself (signing on not asked for, `nav_signon` off): its
    /// codes go to the IBIS as an accepted duty order's do, once there is a bus for them.
    ibis_due: bool,
    /// The Cloudflare tunnel's cloudflared while it runs; why there is none (and since when,
    /// for trying again).
    tunnel: Option<omsi_net::tunnel::Tunnel>,
    tunnel_failed: Option<(Instant, TunnelState)>,
    /// The devices' live pictures: per screen, the texture they are drawn into.
    views: std::collections::HashMap<String, ViewTarget>,
    /// Per screen with a form, a sum of it (the page loads it again when it changed).
    form_sums: std::collections::HashMap<String, u64>,
    /// The script textures the forms show, as they were last sent.
    feeds: std::collections::HashMap<usize, Feed>,
}

/// A script texture a form shows, as it was last sent (see `Companion::capture`).
#[derive(Default)]
struct Feed {
    pacer: screens::Pacer,
    last: Option<Arc<Vec<u8>>>,
    still: Option<Arc<Vec<u8>>>,
}

/// The texture a device's live picture is drawn into, and the picture on its way back.
struct ViewTarget {
    texture: omsi_render::TextureId,
    /// Which texture that is in the scene (a new scene reuses the slot for another one).
    generation: u64,
    size: (u32, u32),
    /// The picture on its way back, and since when.
    pending: Option<(omsi_render::Readback, Instant)>,
    pacer: screens::Pacer,
    /// Last watched (the texture goes a while after nobody watches).
    used: Instant,
    /// A sum of the picture last sent ([`picture_sum`]).
    sent: Option<u64>,
}

/// A quick sum of a picture's pixels, to tell whether it changed (FNV-1a over 8 bytes at a
/// time: a tablet's picture of 8 MB in a millisecond or two).
fn picture_sum(rgba: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut chunks = rgba.chunks_exact(8);
    for c in &mut chunks {
        h = (h ^ u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])).wrapping_mul(0x0000_0100_0000_01b3);
    }
    for &b in chunks.remainder() {
        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^ rgba.len() as u64
}

/// What the devices' navigator was last made of: the route's lanes and the network version
/// its line was drawn from, the line, the trip's stops, the version of both as `/api/trip`
/// sends them, the roads being made on a worker (for which map version), the map version
/// the server has roads of, and when the live picture was made last.
#[derive(Default)]
struct NavCache {
    lanes: Vec<usize>,
    net_version: u64,
    line: nav::RouteLine,
    stops: Vec<nav::MapStop>,
    trip: Option<(String, String)>,
    version: u64,
    roads: Option<(u64, std::sync::mpsc::Receiver<nav::RoadIndex>)>,
    roads_version: u64,
    published: Option<Instant>,
}

static COMPANION: parking_lot::Mutex<Option<Companion>> = parking_lot::Mutex::new(None);

impl Companion {
    fn new() -> Companion {
        let dir = crate::lan::data_dir();
        let devices_path = dir.as_ref().map(|d| d.join("companion-devices.json"));
        let devices = devices_path.as_deref().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| pairing::parse_devices(&t)).unwrap_or_default();
        let shared = Arc::new(server::Shared::default());
        {
            let mut i = shared.lock();
            i.pairing = pairing::Pairing::new(pairing_code(pairing::CODE_LEN), devices);
            i.devices_path = devices_path;
        }
        Companion {
            config: Config::default(),
            config_seen: None,
            shared,
            server: None,
            running: Config::default(),
            failed: None,
            told: false,
            personnel: signon::PersonnelFile::load(dir.map(|d| d.join("personnel.json"))),
            driver: None,
            phone: signon::SignOn::default(),
            duty: None,
            bus: 0,
            screens: Vec::new(),
            look_again: None,
            lines: Vec::new(),
            tours: (None, Vec::new()),
            published: None,
            requests: Vec::new(),
            menu: DutyMenu::default(),
            menu_tours: Vec::new(),
            nav: NavCache::default(),
            watch: nav::TripWatch::default(),
            ibis: None,
            ibis_due: false,
            tunnel: None,
            tunnel_failed: None,
            views: Default::default(),
            form_sums: Default::default(),
            feeds: Default::default(),
        }
    }

    /// Read the setting again when `settings.cfg` changed (the launcher writes it).
    fn check_config(&mut self, now: Instant) {
        if self.config_seen.is_some_and(|(t, _)| now - t < CONFIG_EVERY) {
            return;
        }
        let path = crate::settings::Settings::path();
        let stamp = path.as_deref().and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok());
        let changed = self.config_seen.is_none_or(|(_, s)| s != stamp);
        self.config_seen = Some((now, stamp));
        if changed {
            let text = path.and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
            let c = Config::from_text(&text);
            if c != self.config {
                log::info!("companion: {} (port {})", if c.enabled { "on" } else { "off" }, c.port);
            }
            self.config = c;
        }
    }

    /// Start the server when the setting asks for it, stop it when not (and start it again on
    /// another port when the setting's port changed).
    fn run_server(&mut self, app: &mut App, now: Instant) {
        if self.server.is_some() && (!self.config.enabled || self.running.port != self.config.port) {
            self.server = None;
            let mut i = self.shared.lock();
            i.frames.clear();
            i.watch.clear();
            i.addresses.clear();
            drop(i);
            self.told = false;
            self.bus = 0;
        }
        if !self.config.enabled {
            self.failed = None;
            return;
        }
        if self.server.is_some() || self.failed.as_ref().is_some_and(|(c, t, _)| *c == self.config && now - *t < RETRY_AFTER) {
            return;
        }
        // OMSI_COMPANION_HOST=127.0.0.1: only this machine (a test, without asking the firewall)
        let host: std::net::IpAddr = omsi_cfg::env::var("OMSI_COMPANION_HOST").ok().and_then(|h| h.trim().parse().ok()).unwrap_or(std::net::Ipv4Addr::UNSPECIFIED.into());
        let start = |port: u16| server::Server::start(SocketAddr::new(host, port), self.shared.clone(), lookup, true);
        // (the port taken by something else: one the system picks, said on the screen)
        let started = start(self.config.port).or_else(|e| if e.kind() == std::io::ErrorKind::AddrInUse { start(0) } else { Err(e) });
        match started {
            Ok(s) => {
                self.server = Some(s);
                self.running = self.config;
                self.failed = None;
                self.told = false;
            }
            Err(e) => {
                log::warn!("companion: cannot listen on port {}: {e}", self.config.port);
                app.service_msg = Some((omsi_ui::tr("The phone companion could not start: %{error}").replace("%{error}", &e.to_string()), 8.0));
                self.failed = Some((self.config, now, e.to_string()));
            }
        }
    }

    /// The Cloudflare tunnel: started while the server runs and the setting asks for it (with
    /// a cloudflared that is there: none is fetched here), stopped when not, started again a
    /// while after it ended; its address handed to the server once cloudflared says it. The
    /// pairing is the stricter one while the tunnel is asked for.
    fn run_tunnel(&mut self, now: Instant) {
        let wanted = self.config.tunnel && self.server.is_some();
        if !wanted {
            if self.tunnel.take().is_some() {
                log::info!("companion: Cloudflare tunnel stopped");
            }
            self.tunnel_failed = None;
        } else if self.tunnel.as_mut().is_some_and(|t| !t.alive()) {
            log::warn!("companion: cloudflared ended; trying again in {} s", RETRY_AFTER.as_secs());
            self.tunnel = None;
            self.tunnel_failed = Some((now, TunnelState::Failed));
        } else if self.tunnel.is_none() && self.tunnel_failed.as_ref().is_none_or(|(t, _)| now - *t >= RETRY_AFTER) {
            let port = self.server.as_ref().map_or(self.config.port, |s| s.addr.port());
            match omsi_net::tunnel::find_cloudflared() {
                None => {
                    log::info!("companion: the Cloudflare tunnel is on, but there is no cloudflared (the launcher's settings fetch it)");
                    self.tunnel_failed = Some((now, TunnelState::Missing));
                }
                Some(bin) => match omsi_net::tunnel::Tunnel::start_with(&bin, port, TUNNEL_PID) {
                    Some(t) => {
                        self.tunnel = Some(t);
                        self.tunnel_failed = None;
                    }
                    None => self.tunnel_failed = Some((now, TunnelState::Failed)),
                },
            }
        }
        let public = self.tunnel.as_ref().and_then(|t| t.url.lock().unwrap_or_else(|e| e.into_inner()).clone());
        let mut i = self.shared.lock();
        let mut news = false;
        if i.public != public {
            log::info!("companion: {}", public.as_deref().map_or("no public address".to_string(), |u| format!("reachable through Cloudflare at {u}")));
            i.public = public;
            news = true;
            self.told = false;
        }
        if i.pairing.strict() != wanted {
            i.pairing.set_strict(wanted, &mut pairing_code);
            news = true;
        }
        drop(i);
        if news {
            self.shared.news.notify_all();
        }
    }

    /// What the navigator shows of the tunnel.
    fn tunnel_state(&self) -> Option<TunnelState> {
        if !self.config.tunnel || !self.config.enabled {
            return None;
        }
        if let Some(t) = self.tunnel.as_ref() {
            return Some(match t.url.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                Some(u) => TunnelState::Ready(u),
                None => TunnelState::Starting,
            });
        }
        Some(self.tunnel_failed.as_ref().map_or(TunnelState::Starting, |f| f.1.clone()))
    }

    /// The driver, the duty and the bus as they are now.
    fn follow(&mut self, app: &App, now: Instant) {
        let stem = app.career.path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().to_string());
        let key = signon::driver_key(stem.as_deref());
        if self.driver.as_ref().is_none_or(|d| d.0 != key) {
            let name = app.career.driver.as_ref().map(|d| d.name.trim().to_string()).filter(|n| !n.is_empty()).or(stem).unwrap_or_else(|| "Driver".into());
            let p = self.personnel.of(&key, &mut random_u64);
            self.driver = Some((key.clone(), name, p));
            self.published = None;
        }
        self.phone.follow_driver(&key);
        // (signing on with the number and the code only when the settings ask for it)
        self.phone.ask_sign_on(app.settings.nav_signon);
        let before = self.phone.stage();
        let duty = duty_key_of(app);
        if duty != self.phone.duty() || self.duty.is_some() != app.duty.is_some() {
            self.duty = app.duty.as_ref().map(DutyOrder::of);
        }
        if self.phone.follow_duty(&duty) && !duty.is_empty() && self.phone.auto {
            log::info!("companion: duty signed for by itself (signing on is not asked for)");
            self.ibis_due = true;
        }
        if self.phone.stage() != before {
            self.published = None;
        }
        let bus = app.player.as_ref().map_or(0, |p| Arc::as_ptr(&p.vehicle.ty) as usize);
        // (a second look a moment after the bus came: its scripts have set which of its
        // meshes show by then)
        let again = self.look_again.is_some_and(|t| now >= t);
        if (bus != self.bus || again) && self.server.is_some() {
            self.look_again = (bus != self.bus).then(|| now + LOOK_AGAIN);
            self.bus = bus;
            let looked = Instant::now();
            self.screens = app.player.as_ref().map(|p| screens::discover(&p.vehicle, &app.args.root)).unwrap_or_default();
            log::info!(
                "companion: {} screen(s) in the bus ({} ms): {}",
                self.screens.len(),
                looked.elapsed().as_millis(),
                self.screens.iter().map(|s| format!("{} ({}, {} keys, {})", s.name, s.id, s.keys.len(), s.form.as_ref().map_or("no form".to_string(), |f| format!("{} parts, {} touch areas", f.parts.len(), f.touches.len())))).collect::<Vec<_>>().join(", ")
            );
            let mut i = self.shared.lock();
            i.frames.clear();
            i.watch.clear();
            drop(i);
            self.feeds.clear();
            self.publish_forms(app);
            self.published = None;
        }
    }

    /// The forms of the bus's screens for the devices, with the texture files and fonts they
    /// show under ids of their own.
    fn publish_forms(&mut self, app: &App) {
        let mut files: Vec<server::FormFile> = Vec::new();
        let mut fonts: Vec<server::FormFont> = Vec::new();
        let mut forms = std::collections::HashMap::new();
        self.form_sums.clear();
        if let Some(p) = app.player.as_ref() {
            let v = &p.vehicle;
            for s in &self.screens {
                let Some(f) = s.form.as_ref() else { continue };
                let ids: Vec<usize> = f
                    .files
                    .iter()
                    .map(|u| match files.iter().position(|g| g.path == u.path) {
                        Some(k) => {
                            let g = &mut files[k].uv;
                            *g = [g[0].min(u.uv[0]), g[1].min(u.uv[1]), g[2].max(u.uv[2]), g[3].max(u.uv[3])];
                            k
                        }
                        None => {
                            files.push(server::FormFile { path: u.path.clone(), uv: u.uv, encoded: None });
                            files.len() - 1
                        }
                    })
                    .collect();
                let texts: Vec<Value> = f
                    .texts
                    .iter()
                    .filter_map(|&n| {
                        let t = v.text_textures.get(n)?;
                        let font = t.atlas.as_ref().map(|a| match fonts.iter().position(|g| Arc::ptr_eq(&g.atlas, a)) {
                            Some(k) => k,
                            None => {
                                fonts.push(server::FormFont { atlas: a.clone(), json: None, png: None });
                                fonts.len() - 1
                            }
                        });
                        Some(form::text_json(t, n, font))
                    })
                    .collect();
                let pages: Vec<usize> = v.html_textures.iter().map(|h| h.script_index).collect();
                let json = f.json(&ids, &texts, &pages).to_string();
                self.form_sums.insert(s.id.clone(), picture_sum(json.as_bytes()));
                log::info!("companion: {} drawn by the page: {} parts, {} touch areas, {} texture files, {} text textures, {} script textures ({} kB)", s.name, f.parts.len(), f.touches.len(), f.files.len(), f.texts.len(), f.scripts.len(), json.len() / 1024);
                forms.insert(s.id.clone(), Arc::new(json));
            }
        }
        let mut i = self.shared.lock();
        i.files = files;
        i.fonts = fonts;
        i.forms = forms;
        i.lives.clear();
        drop(i);
        self.shared.news.notify_all();
    }

    /// What lives on the forms devices watch, at most [`form::LIVE_FPS`] times a second.
    fn live_forms(&mut self, app: &App, now: Instant) {
        let Some(p) = app.player.as_ref() else { return };
        if Arc::as_ptr(&p.vehicle.ty) as usize != self.bus {
            return;
        }
        let t = steady();
        for s in &mut self.screens {
            let Some(f) = s.form.as_ref() else { continue };
            if !self.shared.watched(&server::live_key(&s.id), now) || !s.live.due(t, form::LIVE_FPS) {
                continue;
            }
            self.shared.set_live(&s.id, form::live(&p.vehicle, f).to_string());
        }
    }

    /// The state for the devices, when it is due.
    fn publish(&mut self, app: &App, now: Instant) {
        if self.follow_trip(app, now) {
            self.published = None;
        }
        if self.published.is_some_and(|t| now - t < PUBLISH_EVERY) {
            return;
        }
        self.published = Some(now);
        self.update_ibis(app);
        let lang = omsi_ui::i18n::language();
        let state = self.device_state(app).to_string();
        {
            let mut i = self.shared.lock();
            if i.language != lang {
                i.language = lang;
            }
        }
        self.shared.set_state(state);
    }

    /// What a device sees: never the code, the number only once it was typed.
    fn device_state(&self, app: &App) -> Value {
        let stage = self.phone.stage();
        let driver = self.driver.as_ref().map(|(_, name, p)| {
            json!({
                "name": name,
                "number_len": p.number.len(),
                "code_len": p.code.len(),
                "number": if self.phone.signed_on { Some(&p.number) } else { None },
            })
        });
        let bus = app.player.as_ref().map(|p| {
            let d = &p.vehicle.ty.def;
            let short = omsi_launcher_lib::vehicle_type_label(&d.type_name, &d.path);
            omsi_launcher_lib::display_bus_name(&format!("{} {short}", d.manufacturer))
        });
        let duty = app.duty.as_ref().map(|d| {
            let order = self.duty.clone().unwrap_or_else(|| DutyOrder::of(d));
            let trips: Vec<Value> = d
                .trips
                .iter()
                .map(|t| {
                    json!({
                        "line": t.line.trim(),
                        "terminus": t.terminus.trim(),
                        "dep": t.departure.round(),
                        "arr": t.end.round(),
                        "stops": t.stops.iter().filter(|s| s.stops).count(),
                        "from": t.stops.first().map(|s| s.name.trim()).unwrap_or(""),
                    })
                })
                .collect();
            let next = d.trips.get(d.trip_index).and_then(|t| t.stops.get(d.next_stop)).map(|s| s.name.trim().to_string());
            // the navigator's board and sheet, row for row (`nav_duty`)
            let ds = crate::nav_duty::DutyState::of(d, app.clock.time);
            json!({
                "line": order.line,
                "tour": order.tour,
                "lines": order.lines,
                "start": order.start.round(),
                "end": order.end.round(),
                "trips": trips,
                "trip": d.trip_index,
                "next_stop": next,
                "delay": d.delay(app.clock.time).round(),
                "free_line": d.free,
                "focus": ds.as_ref().map(|s| s.focus()),
                "board": ds.as_ref().map(|s| nav::board_json(&crate::nav_duty::board(Some(s), 4))),
                "sheet": ds.as_ref().map(|s| nav::sheet_json(&crate::nav_duty::sheet(s))),
                "break_planned": nav::planned_break(&d.trips, d.trip_index),
                "ibis": self.ibis.as_ref().map(|i| i.1.clone()),
            })
        });
        let screens: Vec<Value> = self
            .screens
            .iter()
            .map(|s| {
                let round = |x: f32| (x * 10_000.0).round() / 10_000.0;
                let keys: Vec<Value> = s.keys.iter().map(|k| json!({ "label": k.label, "title": k.event, "r": k.rect.map(round) })).collect();
                // the device's live picture: its size, and where its keys are in it (for the
                // page to light up the key a finger is on)
                let view = s.view.map(|v| json!({ "w": v.size.0, "h": v.size.1, "keys": s.key_spots.iter().filter(|r| r[2] > r[0]).map(|r| r.map(round)).collect::<Vec<_>>() }));
                // (a device of pages: its picture is all there is of it, full screen)
                let panel = s.source == screens::Source::Panel;
                // (drawn by the page: its form's sum, which changes when the form does)
                let form = self.form_sums.get(&s.id).map(|n| format!("{n:016x}"));
                json!({ "id": s.id, "name": s.name, "html": s.html, "panel": panel, "fields": s.fields.len(), "size": s.size.map(round), "keys": keys, "view": view, "form": form })
            })
            .collect();
        json!({
            "stage": stage.key(),
            "lang": omsi_ui::i18n::language(),
            "driver": driver,
            "clock": app.clock.time.floor(),
            "paused": app.paused,
            "bus": bus,
            "duty": duty,
            "free": self.phone.is_free(),
            // (signing on not asked for: the page has no signing off either)
            "auto_sign_on": self.phone.auto,
            "break_since": self.phone.break_since.map(f64::floor),
            "screens": screens,
            "stop_style": omsi_launcher_lib::stop_style(&app.settings.stop_style),
            "accent": crate::accent::css(),
            "navigator": app.navigator.is_some(),
            "report": self.watch.report,
            // (the page covers its QR code and pairing code as well)
            "hide": self.config.hide,
        })
    }

    /// The trip followed for its report (see [`nav::TripWatch`]); true when a report was
    /// made just now.
    fn follow_trip(&mut self, app: &App, now: Instant) -> bool {
        let d = app.duty.as_ref();
        let info = d.and_then(|d| {
            let t = d.trips.get(d.trip_index)?;
            let line = if t.line.trim().is_empty() { d.line.clone() } else { t.line.clone() };
            Some(nav::TripInfo { index: d.trip_index + 1, count: d.trips.len(), line, terminus: t.terminus.clone(), departure: t.departure, end: t.end })
        });
        let made = self.watch.follow(self.phone.duty(), d.is_some_and(|d| d.free), info, d.is_some_and(|d| d.trip_done()), app.career.stops, app.clock.time, now);
        if made {
            log::info!("companion: trip report for the devices: {}", self.watch.report.as_ref().map(|r| r.to_string()).unwrap_or_default());
        }
        made
    }

    /// The IBIS codes of the duty's trip as the game types them, worked out again when the
    /// trip, the stop, the bus or how far the typing is changed.
    fn update_ibis(&mut self, app: &App) {
        let (Some(d), Some(p)) = (app.duty.as_ref(), app.player.as_ref()) else {
            self.ibis = None;
            return;
        };
        let (trip, stop) = d.trip_for_ibis();
        let state = if p.ibis_typist.is_some() || p.ibis_duty.is_some() {
            nav::IbisState::Typing
        } else if p.duty_typed {
            nav::IbisState::Typed
        } else {
            nav::IbisState::Waiting
        };
        let hof = p.vehicle.host.hof.as_deref();
        let tour = d.tours.get(d.trip_index).map_or(d.tour.as_str(), |t| t.1.as_str());
        let key = format!("{}|{}|{stop}|{state:?}|{}|{tour}|{}", trip.name, trip.line, hof.map_or(0, |h| h as *const omsi_vehicle::Hof as usize), trip.terminus);
        if self.ibis.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        self.ibis = Some((key, nav::ibis_json(hof, trip, stop, tour, state)));
    }

    /// The navigator's live picture for the devices that show the map, five times a second:
    /// the bus, what the navigator says, the trip's route and stops (when they changed) and
    /// the map's roads (made once on a worker, when the map's network is there).
    fn nav(&mut self, app: &App, now: Instant) {
        if !self.shared.nav_watched(now) || self.nav.published.is_some_and(|t| now - t < NAV_EVERY) {
            return;
        }
        self.nav.published = Some(now);
        let Some(p) = app.player.as_ref() else { return };
        // (on foot the map follows the walker, as the navigator does)
        let (bus, heading) = match app.on_foot.as_ref() {
            Some(f) => (f.pos, f.heading),
            None => (p.vehicle.position, p.vehicle.heading),
        };
        let look = app.navigator.as_ref().map(|n| n.companion_look(app.traffic.as_ref(), bus));
        // the roads
        if let Some((v, net)) = look.as_ref().and_then(|l| l.map.clone()) {
            if self.nav.roads_version != v && self.nav.roads.as_ref().is_none_or(|r| r.0 != v) {
                let (tx, rx) = std::sync::mpsc::channel();
                let spawned = std::thread::Builder::new().name("companion roads".into()).spawn(move || {
                    let t0 = Instant::now();
                    let idx = nav::RoadIndex::build(v, crate::navigator::Navigator::companion_roads(&net));
                    log::info!("companion: {} roads for the devices' map in {:.0} ms", idx.len(), t0.elapsed().as_secs_f64() * 1000.0);
                    let _ = tx.send(idx);
                });
                if spawned.is_ok() {
                    self.nav.roads = Some((v, rx));
                }
            }
        }
        if let Some((v, rx)) = self.nav.roads.as_ref() {
            match rx.try_recv() {
                Ok(idx) => {
                    self.nav.roads_version = *v;
                    self.shared.set_roads(idx);
                    self.nav.roads = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.nav.roads = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        // the trip under way (the one the navigator follows): its stops and the route
        let duty = app.duty.as_ref();
        let trip = duty.and_then(|d| d.trips.get(d.trip_index));
        let empty = hashbrown::HashMap::new();
        let stops = trip.map(|t| nav::trip_stops(t, look.as_ref().map_or(&empty, |l| l.places))).unwrap_or_default();
        let lanes: &[usize] = look.as_ref().map_or(&[], |l| l.lanes);
        let net_version = look.as_ref().and_then(|l| l.map.as_ref()).map_or(0, |m| m.0);
        let line_changed = lanes != self.nav.lanes.as_slice() || net_version != self.nav.net_version;
        if line_changed {
            self.nav.line = look.as_ref().and_then(|l| l.net).map(|n| nav::route_line(n, lanes)).unwrap_or_default();
            self.nav.lanes = lanes.to_vec();
            self.nav.net_version = net_version;
        }
        let named = trip.map(|t| (if t.line.trim().is_empty() { duty.map_or("", |d| d.line.as_str()) } else { t.line.as_str() }.to_string(), t.terminus.clone()));
        if line_changed || stops != self.nav.stops || named != self.nav.trip {
            self.nav.stops = stops;
            self.nav.trip = named;
            self.nav.version += 1;
            let trip = self.nav.trip.as_ref().map(|(l, t)| (l.as_str(), t.as_str()));
            self.shared.set_trip(nav::trip_map_json(self.nav.version, &self.nav.line, &self.nav.stops, trip));
        }
        // the next stop: the first the trip serves from the one it is due at
        let next = duty.zip(trip).filter(|(d, _)| !d.trip_done()).and_then(|(d, t)| {
            let (k, s) = t.stops.iter().enumerate().skip(d.next_stop).find(|(_, s)| s.stops)?;
            let last = !t.stops.iter().skip(k + 1).any(|s| s.stops);
            Some((k, s.name.as_str(), s.arr, last))
        });
        let mut traffic = app.navigator.as_ref().map(|n| n.companion_traffic(app.traffic.as_ref(), bus, TRAFFIC_REACH)).unwrap_or_default();
        if traffic.len() > TRAFFIC_MAX {
            traffic.sort_by(|a, b| (a.0 - bus).length_squared().total_cmp(&(b.0 - bus).length_squared()));
            traffic.truncate(TRAFFIC_MAX);
        }
        let along = look.as_ref().filter(|l| !l.lanes.is_empty()).and_then(|l| self.nav.line.along_at(l.progress, l.s));
        let live = nav::Live {
            time: app.clock.time,
            weekday: app.clock.weekday(),
            bus,
            heading,
            speed_kmh: p.vehicle.physics.velocity_kmh(),
            look: look.as_ref(),
            along,
            trip_version: self.nav.version,
            roads_version: self.nav.roads_version,
            next,
            delay: duty.filter(|d| !d.free).map(|_| p.vehicle.host.tt_delay as f64),
            line: self.nav.trip.as_ref().map(|t| t.0.as_str()),
            terminus: self.nav.trip.as_ref().map(|t| t.1.as_str()),
            stop_requested: crate::navigator::stop_requested(&p.vehicle),
            temps: crate::app_events::vehicle_temperatures(p),
            passengers: app.humans.as_ref().map(|h| h.riding()),
            traffic: &traffic,
        };
        self.shared.set_nav(nav::live_json(&live).to_string());
    }

    /// Read the watched screens that are due, and hand over those that changed.
    fn capture(&mut self, app: &App, now: Instant) {
        let Some(p) = app.player.as_ref() else { return };
        let t = steady();
        for s in &mut self.screens {
            if !self.shared.watched(&s.id, now) || !s.pacer.due(t, screens::FPS) {
                continue;
            }
            match s.source {
                screens::Source::Script(n) => {
                    let Some(st) = p.vehicle.host.script_textures.get(n) else { continue };
                    if st.rgba.len() != (st.width * st.height * 4) as usize || s.last.as_ref().is_some_and(|l| l.as_slice() == st.rgba.as_slice()) {
                        continue;
                    }
                    // a texture the script holds locked may be half drawn (the Atron AFR4 keeps
                    // its buffer locked for good, see `ScriptTexture`): sent once it held still
                    // from one look to the next
                    let rgba = if st.locked {
                        match s.still.take() {
                            Some(still) if still.as_slice() == st.rgba.as_slice() => still,
                            _ => {
                                s.still = Some(Arc::new(st.rgba.clone()));
                                continue;
                            }
                        }
                    } else {
                        Arc::new(st.rgba.clone())
                    };
                    s.still = None;
                    s.last = Some(rgba.clone());
                    let crop = screens::crop_pixels(s.crop, st.width, st.height);
                    let small = (crop[2] - crop[0]) * (crop[3] - crop[1]) <= 160 * 160;
                    self.shared.put_frame(&s.id, st.width, st.height, rgba, crop, small);
                }
                screens::Source::Text(n) => {
                    let Some(tt) = p.vehicle.text_textures.get(n) else { continue };
                    let text = tt.last_text.clone().unwrap_or_default();
                    if s.last_text.as_ref() == Some(&text) {
                        continue;
                    }
                    let (w, h) = (tt.def.width.max(1) as u32, tt.def.height.max(1) as u32);
                    let rgba = Arc::new(tt.image(&text));
                    s.last_text = Some(text);
                    self.shared.put_frame(&s.id, w, h, rgba, screens::crop_pixels(s.crop, w, h), true);
                }
                // (only the game's picture of it, `draw_devices`)
                screens::Source::Panel => {}
            }
        }
        // the script textures the forms show (what a script paints, an `[htmltexture]` page):
        // their whole pictures, while a device watches one
        let scripts: std::collections::BTreeSet<usize> = self.screens.iter().filter_map(|s| s.form.as_ref()).flat_map(|f| f.scripts.iter().copied()).collect();
        for n in scripts {
            let key = server::script_key(n);
            if !self.shared.watched(&key, now) {
                continue;
            }
            let feed = self.feeds.entry(n).or_default();
            if !feed.pacer.due(t, screens::FPS) {
                continue;
            }
            let Some(st) = p.vehicle.host.script_textures.get(n) else { continue };
            if st.rgba.len() != (st.width * st.height * 4) as usize || feed.last.as_ref().is_some_and(|l| l.as_slice() == st.rgba.as_slice()) {
                continue;
            }
            // (a texture held locked goes once it held still, as above)
            let rgba = if st.locked {
                match feed.still.take() {
                    Some(still) if still.as_slice() == st.rgba.as_slice() => still,
                    _ => {
                        feed.still = Some(Arc::new(st.rgba.clone()));
                        continue;
                    }
                }
            } else {
                Arc::new(st.rgba.clone())
            };
            feed.still = None;
            feed.last = Some(rgba.clone());
            self.shared.put_frame(&key, st.width, st.height, rgba, [0, 0, st.width, st.height], true);
        }
    }

    fn state(&self) -> CompanionState {
        let i = self.shared.lock();
        let now = Instant::now();
        let running = self.server.is_some();
        CompanionState {
            enabled: self.config.enabled,
            listening: self.server.as_ref().map(|s| s.addr.port()),
            addresses: if running { i.shown_addresses() } else { Vec::new() },
            pairing_code: if running { i.pairing.code().to_string() } else { String::new() },
            hidden: self.config.hide && !revealed(),
            hide: self.config.hide,
            tunnel: self.tunnel_state(),
            devices: i
                .seen
                .iter()
                .filter(|s| now - s.last < server::SEEN_FOR)
                .map(|s| DeviceInfo { name: s.name.clone(), seconds_ago: (now - s.last).as_secs_f32(), screen: s.screen.clone() })
                .collect(),
            paired: i.pairing.devices.len(),
            error: self.failed.as_ref().filter(|_| self.config.enabled && !running).map(|f| f.2.clone()),
            driver: self.driver.as_ref().map(|d| d.1.clone()).unwrap_or_default(),
            personnel_number: self.driver.as_ref().map(|d| d.2.number.clone()).unwrap_or_default(),
            personnel_code: self.driver.as_ref().map(|d| d.2.code.clone()).unwrap_or_default(),
            stage: self.phone.stage(),
            signed_on: self.phone.is_signed_on(),
            accepted: self.phone.is_accepted(),
            free: self.phone.is_free(),
            auto_sign_on: self.phone.auto,
            break_since: self.phone.break_since,
            duty: self.duty.clone(),
            screens: self.screens.iter().map(|s| (s.id.clone(), s.name.clone())).collect(),
            menu: self.menu.clone(),
        }
    }
}

/// The page's texts, from the game's tables (`locales/telefoon.yml` among them).
fn lookup(lang: &str, key: &str) -> Option<String> {
    crate::_rust_i18n_try_translate(lang, key).map(|t| t.into_owned())
}

/// A driver's personnel number and code (`~/.openomsi/personnel.json`), made and written
/// down now when the driver has none yet: the launcher's welcome shows them on a new driver's
/// pass, as Omsi-Hub does, so they are known before the first sign-on. `name` is the driver's
/// file name, as the game keys them (`signon::driver_key`).
pub(crate) fn personnel_of_driver(name: &str) -> Option<(String, String)> {
    let dir = crate::lan::data_dir()?;
    let p = signon::PersonnelFile::load(Some(dir.join("personnel.json"))).of(&signon::driver_key(Some(name)), &mut random_u64);
    Some((p.number, p.code))
}

/// Random numbers: std's `RandomState` keys come from the system's random source (as
/// `voice`'s plugin key).
pub(crate) fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(N.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    h.write_u128(SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.finish()
}

/// A new pairing code of `len` digits.
pub(crate) fn pairing_code(len: usize) -> String {
    (0..len.clamp(1, 12)).map(|_| char::from(b'0' + (random_u64() % 10) as u8)).collect()
}

/// Until when "Show" uncovered the address and the codes (for the navigator, the game menu
/// and the messages alike).
static REVEALED_UNTIL: parking_lot::Mutex<Option<Instant>> = parking_lot::Mutex::new(None);

/// Uncover the address, the pairing code and the QR code for [`REVEAL_FOR`] (`true`), or
/// cover them again at once.
pub(crate) fn reveal(on: bool) {
    *REVEALED_UNTIL.lock() = on.then(|| Instant::now() + REVEAL_FOR);
}

/// "Show" was pressed lately: the address and the codes are not covered now.
fn revealed() -> bool {
    REVEALED_UNTIL.lock().is_some_and(|t| Instant::now() < t)
}

/// cloudflared for the launcher's settings: there or not, being fetched, or the fetch failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cloudflared {
    Installed(std::path::PathBuf),
    Missing,
    Downloading,
    Failed,
}

/// The last look for cloudflared (at most every two seconds: the settings are drawn every
/// frame), and a download under way or failed.
static CLOUDFLARED: parking_lot::Mutex<Option<(Instant, Cloudflared)>> = parking_lot::Mutex::new(None);

/// Whether cloudflared is there for the Cloudflare tunnel (see [`download_cloudflared`]).
pub(crate) fn cloudflared() -> Cloudflared {
    let mut g = CLOUDFLARED.lock();
    match g.as_ref() {
        Some((_, Cloudflared::Downloading)) => return Cloudflared::Downloading,
        Some((t, c)) if t.elapsed() < Duration::from_secs(2) => return c.clone(),
        _ => {}
    }
    let failed = matches!(g.as_ref(), Some((_, Cloudflared::Failed)));
    let found = match omsi_net::tunnel::find_cloudflared() {
        Some(p) => Cloudflared::Installed(p),
        None if failed => Cloudflared::Failed,
        None => Cloudflared::Missing,
    };
    *g = Some((Instant::now(), found.clone()));
    found
}

/// Fetch cloudflared - only when the player asks for it in the launcher's settings: Cloudflare's
/// own release build from its GitHub releases, checked against its published SHA-256, into
/// `~/.openomsi/bin` (`omsi_net::tunnel::ensure_cloudflared`). On a thread of its own.
pub(crate) fn download_cloudflared() {
    {
        let mut g = CLOUDFLARED.lock();
        if matches!(g.as_ref(), Some((_, Cloudflared::Downloading))) {
            return;
        }
        *g = Some((Instant::now(), Cloudflared::Downloading));
    }
    let spawned = std::thread::Builder::new().name("cloudflared download".into()).spawn(|| {
        let got = omsi_net::tunnel::ensure_cloudflared();
        *CLOUDFLARED.lock() = Some((Instant::now(), got.map_or(Cloudflared::Failed, Cloudflared::Installed)));
    });
    if spawned.is_err() {
        *CLOUDFLARED.lock() = Some((Instant::now(), Cloudflared::Failed));
    }
}

/// The game ends: the tunnel's cloudflared goes with it (kept in a static, which Rust never
/// drops), and the server stops.
pub(crate) fn shutdown() {
    if let Some(c) = COMPANION.lock().as_mut() {
        if c.tunnel.take().is_some() {
            log::info!("companion: Cloudflare tunnel stopped (the game ends)");
        }
        c.server = None;
    }
}

/// A new device key: 128 random bits as hex.
pub(crate) fn device_key() -> String {
    format!("{:016x}{:016x}", random_u64(), random_u64())
}

/// Seconds of a steady clock.
fn steady() -> f64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// The duty the game has now, as [`signon::duty_key`] knows it (empty: none).
fn duty_key_of(app: &App) -> String {
    let Some(d) = app.duty.as_ref() else { return String::new() };
    let map = app.world.as_ref().map(|w| w.global.name.clone()).unwrap_or_default();
    let trips: Vec<(&str, f64)> = d.trips.iter().map(|t| (t.name.as_str(), t.departure)).collect();
    signon::duty_key(&map, &d.line, &d.tour, &trips)
}

/// The duty's IBIS typed as the driver would, unless the game did that already.
fn type_ibis(app: &mut App) {
    let (Some(d), Some(p)) = (app.duty.as_ref(), app.player.as_mut()) else { return };
    if p.duty_typed {
        return;
    }
    let (trip, stop) = d.trip_for_ibis();
    log::info!("companion: duty signed for: the IBIS gets line {} to {}", trip.line.trim(), trip.terminus.trim());
    p.set_duty_destination(trip, stop);
}

/// A click into the cab at `at` (bus frame) along the screen's normal, from before the
/// screen (or behind it, should the normal face the other way): the switch found there is
/// worked as the mouse works it. The switch's event, if one was found.
fn press_at(p: &mut crate::player::Player, at: Vec3, normal: Vec3) -> Option<String> {
    let normal = normal.normalize_or_zero();
    if normal == Vec3::ZERO {
        return None;
    }
    for side in [1.0f32, -1.0] {
        let origin = p.vehicle.position + (at + normal * (REACH * side)).as_dvec3();
        let dir = -normal * side;
        p.occlude_controls = false;
        let Some(i) = p.pick(origin, dir, SPREAD) else { continue };
        if p.vehicle.ty.mesh_bounds.get(i).is_some_and(|b| b.1 > screens::BIG) {
            continue;
        }
        let i = p.click(origin, dir, SPREAD)?;
        return p.vehicle.ty.meshes.get(i).and_then(|m| p.vehicle.ty.model.meshes.get(m.def_index)).and_then(|d| d.mouse_event.clone());
    }
    None
}

/// The game's lines in the duty menu: (name, label).
fn duty_lines(app: &App) -> Vec<(String, String)> {
    crate::game_lists::items(app, &crate::game_lists::ListKind::Lines).into_iter().filter_map(|(label, action)| action.strip_prefix("line ").map(|n| (n.to_string(), label))).collect()
}

/// The tours of `line` in the duty menu: (line, tour, what, when) - "Tour 3 › Hafen", "05:42".
fn duty_tours(app: &App, line: &str) -> Vec<(String, String, String, String)> {
    use crate::game_lists::{items, menu_extras, ListKind};
    let list = items(app, &ListKind::Lines);
    let sel = list.iter().position(|(_, a)| a.strip_prefix("line ") == Some(line));
    let rows = sel.and_then(|s| menu_extras(Some(&ListKind::Lines), Some(&list), Some(s), app.schedule.as_ref(), app.clock.time).2).map(|p| p.rows).unwrap_or_default();
    let tours: Vec<(String, String, String)> = items(app, &ListKind::Tours(line.to_string(), None))
        .into_iter()
        .filter_map(|(label, action)| {
            let (l, t) = action.strip_prefix("tour ")?.split_once('\u{1}')?;
            Some((l.to_string(), t.to_string(), label))
        })
        .collect();
    let same = rows.len() == tours.len();
    tours.into_iter().enumerate().map(|(k, (l, t, label))| if same { (l, t, rows[k].0.clone(), rows[k].1.clone()) } else { (l, t, label, String::new()) }).collect()
}

/// Take on tour `t` of line `l` as the duty menu does; picking it is signing for it (the game
/// typed its IBIS already). False when it did not become the duty.
fn take_on(app: &mut App, l: &str, t: &str) -> bool {
    crate::game_lists::run(app, &crate::game_lists::ListKind::Tours(l.to_string(), None), &format!("tour {l}\u{1}{t}"));
    let key = duty_key_of(app);
    COMPANION.lock().as_mut().is_some_and(|c| {
        c.phone.follow_duty(&key);
        c.published = None;
        c.phone.accept()
    })
}

/// Do what the navigator asked.
fn answer(app: &mut App, r: Request) {
    match r {
        Request::Accept => {
            accept_duty(app);
        }
        Request::Break(on) => {
            set_break(app, on);
        }
        Request::Lines => {
            let lines = duty_lines(app);
            if let Some(c) = COMPANION.lock().as_mut() {
                c.menu.lines = Some(lines);
            }
        }
        Request::Tours(k) => {
            let Some(name) = COMPANION.lock().as_ref().and_then(|c| c.menu.lines.as_ref()?.get(k).map(|l| l.0.clone())) else { return };
            let tours = duty_tours(app, &name);
            if let Some(c) = COMPANION.lock().as_mut() {
                c.menu_tours = tours.iter().map(|t| (t.0.clone(), t.1.clone())).collect();
                c.menu.tours = Some((k, tours.into_iter().map(|t| (t.2, t.3)).collect()));
            }
        }
        Request::Pick(k, n) => {
            let chosen = COMPANION.lock().as_ref().and_then(|c| if c.menu.tours.as_ref()?.0 == k { c.menu_tours.get(n).cloned() } else { None });
            let ok = chosen.is_some_and(|(l, t)| {
                log::info!("companion: line {l} tour {} taken on in the navigator", t.trim());
                take_on(app, &l, &t)
            });
            if let Some(c) = COMPANION.lock().as_mut() {
                c.menu.failed = !ok;
            }
        }
    }
}

/// Do what a device asked; its answer.
fn run(app: &mut App, command: http::Command) -> Value {
    use http::Command as C;
    let signed_on = COMPANION.lock().as_ref().is_some_and(|c| c.phone.is_signed_on());
    let needs_sign_on = !matches!(command, C::SignOn { .. } | C::SignOff);
    if needs_sign_on && !signed_on {
        return json!({ "ok": false, "error": "sign_on" });
    }
    match command {
        C::SignOn { number, code } => json!({ "result": sign_on(&number, code.as_deref()).key() }),
        C::SignOff => {
            sign_off();
            json!({ "ok": true })
        }
        C::Accept => json!({ "ok": accept_duty(app) }),
        C::Free => json!({ "ok": drive_free() }),
        C::Break(on) => json!({ "ok": set_break(app, on) }),
        C::Lines => {
            let lines = duty_lines(app);
            if let Some(c) = COMPANION.lock().as_mut() {
                c.lines = lines.iter().map(|l| l.0.clone()).collect();
                c.tours = (None, Vec::new());
            }
            let list: Vec<Value> = lines.iter().enumerate().map(|(nr, (name, label))| json!({ "nr": nr, "name": name, "label": label })).collect();
            json!({ "lines": list })
        }
        C::Tours { line } => {
            let Some(name) = COMPANION.lock().as_ref().and_then(|c| c.lines.get(line).cloned()) else { return json!({ "ok": false }) };
            let tours = duty_tours(app, &name);
            if let Some(c) = COMPANION.lock().as_mut() {
                c.tours = (Some(line), tours.iter().map(|t| (t.0.clone(), t.1.clone())).collect());
            }
            let list: Vec<Value> = tours.iter().enumerate().map(|(nr, (_, tour, what, when))| json!({ "nr": nr, "tour": tour.trim(), "what": what, "when": when })).collect();
            json!({ "line": name, "tours": list })
        }
        C::Pick { line, tour } => {
            let chosen = COMPANION.lock().as_ref().and_then(|c| (c.tours.0 == Some(line)).then(|| c.tours.1.get(tour).cloned()).flatten());
            let Some((l, t)) = chosen else { return json!({ "ok": false }) };
            log::info!("companion: line {l} tour {} taken on from the phone", t.trim());
            json!({ "ok": take_on(app, &l, &t) })
        }
        C::Pointer { screen, kind, u, v } => {
            let Some(s) = COMPANION.lock().as_ref().and_then(|c| c.screens.iter().find(|s| s.id == screen).cloned()) else { return json!({ "ok": false }) };
            let Some(p) = app.player.as_mut() else { return json!({ "ok": false }) };
            let (uf, vf) = (s.crop[0] + u * (s.crop[2] - s.crop[0]), s.crop[1] + v * (s.crop[3] - s.crop[1]));
            if s.html {
                let screens::Source::Script(n) = s.source else { return json!({ "ok": false }) };
                let (uf, vf) = screens::in_texture(s.crop, uf, vf);
                let kind = match kind {
                    http::Pointer::Down => omsi_sim::htmltex::PointerKind::Down,
                    http::Pointer::Move => omsi_sim::htmltex::PointerKind::Move,
                    http::Pointer::Up => omsi_sim::htmltex::PointerKind::Up,
                };
                return json!({ "ok": p.html_pointer(n, uf, vf, kind) });
            }
            match kind {
                http::Pointer::Down => {
                    let tris = screens::tris_now(&p.vehicle, &s.meshes);
                    let event = screens::surface_at(&tris, uf, vf).and_then(|(at, n)| press_at(p, at, n));
                    set_held(&screen, event.is_some());
                    json!({ "ok": event.is_some(), "event": event })
                }
                http::Pointer::Up => {
                    if s.held.is_some() {
                        p.release();
                        set_held(&screen, false);
                    }
                    json!({ "ok": true })
                }
                http::Pointer::Move => json!({ "ok": true }),
            }
        }
        C::Key { screen, key, down } => {
            let Some(s) = COMPANION.lock().as_ref().and_then(|c| c.screens.iter().find(|s| s.id == screen).cloned()) else { return json!({ "ok": false }) };
            let (Some(k), Some(p)) = (s.keys.get(key), app.player.as_mut()) else { return json!({ "ok": false }) };
            if !down {
                if s.held.is_some() {
                    p.release();
                    set_held(&screen, false);
                }
                return json!({ "ok": true });
            }
            let normal = screens::plane_of(&screens::tris_now(&p.vehicle, &s.meshes)).map(|pl| pl.normal);
            let event = normal.zip(screens::mesh_centre(&p.vehicle, k.mesh)).and_then(|(n, (c, _))| press_at(p, c, n));
            set_held(&screen, event.is_some());
            json!({ "ok": event.is_some(), "event": event })
        }
        C::Touch { screen, touch, down } => {
            let Some(s) = COMPANION.lock().as_ref().and_then(|c| c.screens.iter().find(|s| s.id == screen).cloned()) else { return json!({ "ok": false }) };
            let Some(p) = app.player.as_mut() else { return json!({ "ok": false }) };
            if !down {
                if s.held.is_some() {
                    p.release();
                    set_held(&screen, false);
                }
                return json!({ "ok": true });
            }
            // (the switch of that touch area, clicked as the mouse clicks it)
            let Some(mesh) = s.form.as_ref().and_then(|f| f.meshes.get(f.touches.get(touch)?.mesh).copied()) else { return json!({ "ok": false }) };
            let event = p.click_mesh(mesh);
            set_held(&screen, event.is_some());
            json!({ "ok": event.is_some(), "event": event })
        }
        C::Page { screen, page, kind, u, v } => {
            let shows = COMPANION.lock().as_ref().is_some_and(|c| c.screens.iter().any(|s| s.id == screen && s.form.as_ref().is_some_and(|f| f.scripts.contains(&page))));
            let Some(p) = app.player.as_mut() else { return json!({ "ok": false }) };
            if !shows || !p.vehicle.html_textures.iter().any(|h| h.script_index == page) {
                return json!({ "ok": false });
            }
            let kind = match kind {
                http::Pointer::Down => omsi_sim::htmltex::PointerKind::Down,
                http::Pointer::Move => omsi_sim::htmltex::PointerKind::Move,
                http::Pointer::Up => omsi_sim::htmltex::PointerKind::Up,
            };
            json!({ "ok": p.html_pointer(page, u, v, kind) })
        }
        C::Tap { screen, kind, x, y } => {
            let Some(s) = COMPANION.lock().as_ref().and_then(|c| c.screens.iter().find(|s| s.id == screen).cloned()) else { return json!({ "ok": false }) };
            let Some(p) = app.player.as_mut() else { return json!({ "ok": false }) };
            let Some(view) = s.view_now(&p.vehicle) else { return json!({ "ok": false }) };
            // the ray from the picture's camera through the tap, as the bus stands now
            let rot = p.vehicle.body_rotation();
            let (o, d) = view.ray(x, y);
            let origin = p.vehicle.position + rot.transform_point3(o).as_dvec3();
            let dir = rot.transform_vector3(d).normalize_or_zero();
            p.occlude_controls = false;
            match kind {
                http::Pointer::Down => {
                    // a page under it takes the press, as under the mouse (`input_script`)
                    if let Some((page, u, v)) = p.html_hit(origin, dir) {
                        p.release();
                        p.html_pointer(page, u, v, omsi_sim::htmltex::PointerKind::Down);
                        set_page_held(&screen, Some((page, u, v)));
                        return json!({ "ok": true, "page": true });
                    }
                    // (a whole panel that is dragged into place is no switch to tap: a plain
                    // click on it would swing it to its other end)
                    if p.pick(origin, dir, view.spread()).is_some_and(|i| p.vehicle.ty.mesh_bounds.get(i).is_some_and(|b| b.1 > screens::BIG)) {
                        return json!({ "ok": false });
                    }
                    let event = p.click(origin, dir, view.spread()).and_then(|i| p.vehicle.ty.meshes.get(i).and_then(|m| p.vehicle.ty.model.meshes.get(m.def_index)).and_then(|d| d.mouse_event.clone()));
                    set_held(&screen, event.is_some());
                    json!({ "ok": event.is_some(), "event": event })
                }
                http::Pointer::Move => {
                    if let Some((page, u, v)) = s.page_held {
                        let (u, v) = p.html_hit(origin, dir).filter(|h| h.0 == page).map_or((u, v), |h| (h.1, h.2));
                        p.html_pointer(page, u, v, omsi_sim::htmltex::PointerKind::Move);
                    }
                    json!({ "ok": true })
                }
                http::Pointer::Up => {
                    if let Some((page, u, v)) = s.page_held {
                        let (u, v) = p.html_hit(origin, dir).filter(|h| h.0 == page).map_or((u, v), |h| (h.1, h.2));
                        p.html_pointer(page, u, v, omsi_sim::htmltex::PointerKind::Up);
                        set_page_held(&screen, None);
                    } else if s.held.is_some() {
                        p.release();
                        set_held(&screen, false);
                    }
                    json!({ "ok": true })
                }
            }
        }
    }
}

/// A device's picture texture nobody watched for this long goes.
const VIEW_KEEP: Duration = Duration::from_secs(20);
/// A picture the GPU has not handed back within this long is given up.
const READBACK_WAIT: Duration = Duration::from_secs(2);

/// The devices' live pictures, in the redraw after the mirrors: for each device a phone or
/// tablet watches, a picture at most [`device::FPS`] times a second, drawn into a texture of
/// its own as a mirror is and read back without waiting for the GPU; a picture that came back
/// goes to the server (encoded as JPEG on the connection's thread).
pub(crate) fn draw_devices(r: &mut omsi_render::Renderer, scene: &mut omsi_render::Scene, p: &crate::player::Player, lighting: &omsi_render::Lighting) {
    let mut g = COMPANION.lock();
    let Some(c) = g.as_mut() else { return };
    let now = Instant::now();
    let Companion { screens, views, shared, server, bus, .. } = c;
    let free = |r: &omsi_render::Renderer, scene: &mut omsi_render::Scene, v: ViewTarget| {
        if r.texture_generation(scene, v.texture) == Some(v.generation) {
            r.free_texture(scene, v.texture);
        }
    };
    if server.is_none() || *bus != Arc::as_ptr(&p.vehicle.ty) as usize {
        if server.is_some() && screens.iter().any(|s| shared.watched(&server::view_key(&s.id), now)) {
            once("other bus", || log::warn!("companion: a device's picture is watched, but the bus driven is not the one whose screens were found"));
        }
        for (_, v) in views.drain() {
            free(r, scene, v);
        }
        return;
    }
    let t = steady();
    for s in screens.iter() {
        let key = server::view_key(&s.id);
        let Some(view) = s.view_now(&p.vehicle) else {
            if shared.watched(&key, now) {
                once(&format!("noview {}", s.id), || log::warn!("companion: {} ({}) is watched but has no picture to take (no view)", s.name, s.id));
            }
            continue;
        };
        // a picture that came back (sent when it is another than the last: a display that
        // stands still costs the network nothing)
        if let Some(target) = views.get_mut(&s.id) {
            if let Some((rb, since)) = target.pending.as_ref() {
                match rb.take() {
                    Some(Some((w, h, rgba))) => {
                        once(&format!("back {}", s.id), || log::info!("companion: {} ({}): first picture back from the GPU, {w}x{h}", s.name, s.id));
                        let sum = picture_sum(&rgba);
                        if target.sent != Some(sum) {
                            target.sent = Some(sum);
                            shared.put_frame(&key, w, h, Arc::new(rgba), [0, 0, w, h], false);
                        }
                        target.pending = None;
                    }
                    Some(None) => {
                        once(&format!("failed {}", s.id), || log::warn!("companion: {} ({}): the GPU could not hand the picture back", s.name, s.id));
                        target.pending = None
                    }
                    None if now - *since > READBACK_WAIT => {
                        once(&format!("late {}", s.id), || log::warn!("companion: {} ({}): the picture did not come back from the GPU in time", s.name, s.id));
                        target.pending = None
                    }
                    None => {}
                }
            }
        }
        if !shared.watched(&key, now) {
            continue;
        }
        // the texture, at the picture's size (made again when the scene has another texture
        // in its place: a new map was loaded)
        let fits = views.get(&s.id).is_some_and(|v| v.size == view.size && r.texture_generation(scene, v.texture) == Some(v.generation));
        if !fits {
            if let Some(old) = views.remove(&s.id) {
                free(r, scene, old);
            }
            let texture = r.add_readable_render_texture(scene, view.size.0, view.size.1);
            let Some(generation) = r.texture_generation(scene, texture) else { continue };
            views.insert(s.id.clone(), ViewTarget { texture, generation, size: view.size, pending: None, pacer: screens::Pacer::default(), used: now, sent: None });
        }
        let Some(target) = views.get_mut(&s.id) else { continue };
        target.used = now;
        if target.pending.is_some() || !target.pacer.due(t, device::FPS) {
            continue;
        }
        let cam = view.camera(p.vehicle.body_rotation(), p.vehicle.position);
        let mut light = lighting.clone();
        light.shadows = false;
        // (the small switches are large in this picture; and a device is to be readable at
        // night too: the cab at least as light as on a dull day)
        light.min_obj_size = 0.0;
        light.ambient = light.ambient.max(Vec3::splat(0.35));
        r.render_to_texture(scene, target.texture, &cam, &light, view.aspect);
        target.pending = r.start_readback(scene, target.texture).map(|rb| (rb, now));
        once(&format!("drawn {}", s.id), || log::info!("companion: {} ({}): first picture drawn, {}x{}, read back: {}", s.name, s.id, view.size.0, view.size.1, target.pending.is_some()));
    }
    // the textures nobody watches any more
    let stale: Vec<String> = views.iter().filter(|(_, v)| now - v.used > VIEW_KEEP && v.pending.is_none()).map(|(k, _)| k.clone()).collect();
    for k in stale {
        if let Some(v) = views.remove(&k) {
            free(r, scene, v);
        }
    }
}

/// Say `what` in the log once per run for `key` (the devices' pictures: where they stop, if
/// they do, shows in the log without a line every frame).
fn once(key: &str, what: impl FnOnce()) {
    static SAID: parking_lot::Mutex<Option<std::collections::HashSet<String>>> = parking_lot::Mutex::new(None);
    let mut g = SAID.lock();
    if g.get_or_insert_with(Default::default).insert(key.to_string()) {
        what();
    }
}

/// The page under a tap on a device's picture that holds the press (it gets the release).
fn set_page_held(screen: &str, held: Option<(usize, f32, f32)>) {
    if let Some(s) = COMPANION.lock().as_mut().and_then(|c| c.screens.iter_mut().find(|s| s.id == screen)) {
        s.page_held = held;
    }
}

fn set_held(screen: &str, held: bool) {
    if let Some(s) = COMPANION.lock().as_mut().and_then(|c| c.screens.iter_mut().find(|s| s.id == screen)) {
        s.held = held.then(Instant::now);
    }
}

/// Once a frame: the setting, the server, the driver and the duty, the devices' commands,
/// the state and the pictures for them.
pub(crate) fn frame(app: &mut App, _dt: f32) {
    // (a dedicated server has no driver at its place)
    if app.args.server.is_some() {
        return;
    }
    let now = Instant::now();
    // the navigator's requests first (without the lock: they ask for the companion's state),
    // so that the duty taken on is followed below before anything shows it
    let requests = COMPANION.lock().as_mut().map(|c| std::mem::take(&mut c.requests)).unwrap_or_default();
    for r in requests {
        answer(app, r);
    }
    let (jobs, ibis) = {
        let mut g = COMPANION.lock();
        let c = g.get_or_insert_with(Companion::new);
        c.check_config(now);
        c.run_server(app, now);
        c.run_tunnel(now);
        c.follow(app, now);
        // (a duty signed for by itself: its codes to the IBIS once the bus is there)
        let ibis = c.ibis_due && app.player.is_some() && app.duty.is_some();
        if ibis {
            c.ibis_due = false;
        }
        (if c.server.is_some() { c.shared.take_jobs() } else { Vec::new() }, ibis)
    };
    if ibis {
        type_ibis(app);
    }
    // (without the lock: what they do may ask for the companion's state)
    let had_jobs = !jobs.is_empty();
    for job in jobs {
        let answer = run(app, job.command);
        let _ = job.reply.send(answer);
    }
    let mut g = COMPANION.lock();
    let Some(c) = g.as_mut() else { return };
    if c.server.is_none() {
        return;
    }
    if had_jobs {
        c.published = None;
        // (a duty taken on: the driver, the duty and the bus again before the state goes out)
        c.follow(app, now);
    }
    // a switch a device pressed and whose release never came (the Wi-Fi went): let go
    if let Some(s) = c.screens.iter_mut().find(|s| s.held.is_some_and(|t| now - t > HELD_FOR)) {
        s.held = None;
        if let Some(p) = app.player.as_mut() {
            log::info!("companion: a switch of {} held for {} s without being let go: released", s.name, HELD_FOR.as_secs());
            p.release();
        }
    }
    let (notes, addresses, code) = {
        let mut i = c.shared.lock();
        (std::mem::take(&mut i.notes), i.shown_addresses(), i.pairing.code().to_string())
    };
    if !c.told && !addresses.is_empty() {
        c.told = true;
        // (covered for streaming: where to find them, not what they are)
        app.service_msg = Some(if c.config.hide && !revealed() {
            (omsi_ui::tr("Phone & tablet ready: the address and pairing code are in the navigator (hidden while streaming)").into_owned(), 8.0)
        } else {
            let at = shown_address(&addresses[0]);
            (omsi_ui::tr("Phone: open %{address} and enter the pairing code %{code}").replace("%{address}", &at).replace("%{code}", &code), 20.0)
        });
    }
    if let Some(name) = notes.last() {
        app.service_msg = Some((omsi_ui::tr("%{device} is connected").replace("%{device}", name), 5.0));
    }
    c.publish(app, now);
    c.nav(app, now);
    c.live_forms(app, now);
    c.capture(app, now);
}

// ---------------------------------------------------------------------------------------
// for the navigator: the same signing on as on a device, in the game's own interface

/// The companion now: the server, the devices, the driver and how far the signing on is.
pub(crate) fn state() -> CompanionState {
    COMPANION.lock().as_ref().map(Companion::state).unwrap_or_default()
}

/// A try at signing on: the number alone (the keypad's first step), then with the code.
pub(crate) fn sign_on(number: &str, code: Option<&str>) -> Attempt {
    let mut g = COMPANION.lock();
    let Some(c) = g.as_mut() else { return Attempt::Wrong };
    let Some(p) = c.driver.as_ref().map(|d| d.2.clone()) else { return Attempt::Wrong };
    let a = c.phone.attempt(&p, number, code, steady());
    if a == Attempt::SignedOn {
        c.published = None;
    }
    a
}

pub(crate) fn sign_off() {
    if let Some(c) = COMPANION.lock().as_mut() {
        c.phone.sign_off();
        c.published = None;
    }
}

/// Sign the duty order of the duty the game has: the IBIS gets the duty's line, route and
/// destination (typed as the driver would, unless the game has typed them already). False
/// when not signed on or without a duty.
pub(crate) fn accept_duty(app: &mut App) -> bool {
    let key = duty_key_of(app);
    let ok = COMPANION.lock().as_mut().is_some_and(|c| {
        c.phone.follow_duty(&key);
        c.published = None;
        c.phone.accept()
    });
    if ok {
        type_ibis(app);
    }
    ok
}

/// Drive without a duty (signed on, the game has none).
pub(crate) fn drive_free() -> bool {
    COMPANION.lock().as_mut().is_some_and(|c| {
        c.published = None;
        c.phone.drive_free()
    })
}

/// Start (from the game's clock now) or end a break.
pub(crate) fn set_break(app: &App, on: bool) -> bool {
    let at = on.then_some(app.clock.time);
    COMPANION.lock().as_mut().is_some_and(|c| {
        c.published = None;
        c.phone.set_break(at)
    })
}

/// Ask for something that needs the game (see [`Request`]); the same request twice before
/// the next frame is one. Asking for the lines (or a line's tours) again forgets the ones
/// there were, so that nothing stale shows meanwhile.
pub(crate) fn request(r: Request) {
    let mut g = COMPANION.lock();
    let Some(c) = g.as_mut() else { return };
    match r {
        Request::Lines => {
            c.menu = DutyMenu::default();
            c.menu_tours.clear();
        }
        Request::Tours(_) => {
            c.menu.tours = None;
            c.menu.failed = false;
            c.menu_tours.clear();
        }
        _ => {}
    }
    if !c.requests.contains(&r) {
        c.requests.push(r);
    }
}

/// Unpair every device (they have to pair again with the code).
pub(crate) fn forget_devices() {
    if let Some(c) = COMPANION.lock().as_mut() {
        let mut i = c.shared.lock();
        i.pairing.forget_all();
        i.seen.clear();
        if let Some(path) = i.devices_path.clone() {
            let _ = signon::write_private(&path, &pairing::devices_json(&[]));
        }
    }
}

/// Another pairing code (the one shown was seen by someone it was not meant for).
pub(crate) fn new_pairing_code() {
    if let Some(c) = COMPANION.lock().as_ref() {
        let mut i = c.shared.lock();
        let n = i.pairing.code_len();
        i.pairing.set_code(pairing_code(n));
    }
}

/// The QR code a phone or tablet scans to open the page and pair at once (the address with
/// the pairing code in it, `http://192.168.1.20:47811/?pair=463055`): its side in modules and
/// the dark ones, row by row. None while the companion is off or has no address. A code
/// that paired a device is replaced by a new one, and so is this QR code.
// (for the navigator's sign-on page, which draws it)
pub(crate) fn pair_qr() -> Option<(usize, Vec<bool>)> {
    let s = state();
    let url = server::pair_url(s.addresses.first()?, &s.pairing_code)?;
    let q = qr::encode_address(&url)?;
    Some((q.size, q.modules))
}

/// The line above the game menu's duty menu ("Line and tour..."): the driver's personnel
/// number and code, which the driver signs on with, and with the companion on where a phone
/// connects. Empty before the game's first frame.
pub(crate) fn duty_menu_note() -> String {
    let s = state();
    if s.personnel_number.is_empty() {
        return String::new();
    }
    let mut note = omsi_ui::tr("Personnel no. %{number} · code %{code}").replace("%{number}", &s.personnel_number).replace("%{code}", &s.personnel_code);
    if let (Some(at), false) = (s.addresses.first(), s.pairing_code.is_empty()) {
        note.push_str("   ·   ");
        if s.hidden {
            note.push_str(&omsi_ui::tr("Phone: address and pairing code hidden (Show them in the navigator)"));
        } else {
            note.push_str(&omsi_ui::tr("Phone %{address}, pairing code %{code}").replace("%{address}", &shown_address(at)).replace("%{code}", &s.pairing_code));
        }
    }
    note
}

/// An address as the driver types it: without `http://` and the slash at the end
/// (`192.168.1.20:47811`); a tunnel's keeps its `https://`.
pub(crate) fn shown_address(a: &str) -> String {
    a.trim().trim_start_matches("http://").trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_is_off_by_default_and_read_like_the_other_settings() {
        let off = Config::default();
        assert_eq!(Config::from_text(""), Config { enabled: false, port: DEFAULT_PORT, ..off });
        assert_eq!(Config::from_text("vsync=1\ncompanion=1\n"), Config { enabled: true, port: DEFAULT_PORT, ..off });
        assert_eq!(Config::from_text(" Companion = on \ncompanion_port=50000"), Config { enabled: true, port: 50000, ..off });
        assert_eq!(Config::from_text("companion=0\ncompanion_port=80"), Config { enabled: false, port: DEFAULT_PORT, ..off }, "no port below 1024");
        assert_eq!(Config::from_text("companion=yes please").enabled, false);
    }

    #[test]
    fn the_address_is_covered_and_the_tunnel_off_unless_asked_otherwise() {
        let c = Config::from_text("companion=1\n");
        assert!(c.hide && !c.tunnel, "streaming-safe and on the home network only by default");
        let c = Config::from_text("companion=1\ncompanion_hide=0\ncompanion_tunnel=1\n");
        assert!(!c.hide && c.tunnel);
        assert!(Config::from_text("companion_hide=maybe").hide, "anything but a clear no keeps it covered");
        reveal(true);
        assert!(revealed() && REVEALED_UNTIL.lock().is_some_and(|t| t > Instant::now() + REVEAL_FOR - Duration::from_secs(2)));
        reveal(false);
        assert!(!revealed());
    }

    #[test]
    fn the_pairing_link_goes_through_the_tunnel_when_there_is_one() {
        assert_eq!(server::pair_url("http://192.168.1.20:47811/", "463055").as_deref(), Some("http://192.168.1.20:47811/?pair=463055"));
        assert_eq!(server::pair_url("https://quiet-river.trycloudflare.com", "123456789").as_deref(), Some("https://quiet-river.trycloudflare.com/?pair=123456789"));
        assert_eq!(server::pair_url("10.0.0.5:47811", "1").as_deref(), Some("http://10.0.0.5:47811/?pair=1"));
        let mut i = server::Inner { addresses: vec!["http://192.168.1.20:47811/".into()], ..Default::default() };
        assert_eq!(i.shown_addresses(), vec!["http://192.168.1.20:47811/".to_string()]);
        i.public = Some("https://quiet-river.trycloudflare.com".into());
        assert_eq!(i.shown_addresses(), vec!["https://quiet-river.trycloudflare.com".to_string()], "the tunnel's instead of the home network's");
    }

    #[test]
    fn the_companions_texts_are_in_the_games_tables() {
        assert_eq!(lookup("nl", "Sign on").as_deref(), Some("Aanmelden"));
        assert_eq!(lookup("de", "Personnel no. %{number} · code %{code}").as_deref(), Some("Personalnummer %{number} · PIN %{code}"));
        assert_eq!(lookup("uk", "Accept duty").as_deref(), Some("Прийняти зміну"));
        // (the game's own table is still there beside it)
        assert_eq!(lookup("nl", "Line").as_deref(), Some("Lijn"));
    }

    #[test]
    fn codes_and_keys_look_as_they_should() {
        let code = pairing_code(pairing::CODE_LEN);
        assert_eq!(code.len(), 6);
        assert!(code.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(pairing_code(pairing::STRICT_CODE_LEN).len(), 9);
        assert_ne!(pairing_code(9), pairing_code(9));
        let (a, b) = (device_key(), device_key());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
