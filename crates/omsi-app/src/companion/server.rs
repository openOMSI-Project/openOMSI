//! The companion's server: a thread that listens on the home network and a thread for each
//! connection (sixteen at most), in the style of the WebSocket gateway (`omsi_net::ws`).
//!
//! The game and the server meet in [`Shared`]: the game writes the state a device sees and
//! the pictures of the screens into it, and takes out the commands the devices sent; the
//! server waits on its condition variable for news. The server never touches the game
//! itself - a command waits in the queue until the game's next frame has done it and answered
//! - and the game never waits for the server: encoding a picture happens on the connection's
//! thread, without the lock.
//!
//! A device asks for news by long polling: `GET /api/state?after=<version>` is answered as
//! soon as there is a newer state (or after 20 s with the same one), `GET /api/frame` as soon
//! as there is a newer picture of the screen. So the pictures go at the pace the device takes
//! them in, and a screen nobody looks at is never read or encoded (see `watch`).

use super::http::{self, Command, Request, Response, Route};
use super::pairing::{Pairing, Refused};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Connections served at once; more are closed at once.
const MAX_CONNECTIONS: usize = 16;
/// How long a state request waits for news, and a picture request for a new picture.
const STATE_WAIT: Duration = Duration::from_secs(20);
const FRAME_WAIT: Duration = Duration::from_secs(4);
/// A screen counts as watched this long after a request for its picture began (longer than
/// `FRAME_WAIT`: the request after it follows at once).
pub(crate) const WATCH_FOR: Duration = Duration::from_secs(6);
/// How long a command waits for the game to have done it.
const COMMAND_WAIT: Duration = Duration::from_secs(4);
/// A device counts as connected this long after its last request.
pub(crate) const SEEN_FOR: Duration = Duration::from_secs(45);

/// Translations of the page's texts: (language, English text) → the text in that language.
pub(crate) type Lookup = fn(&str, &str) -> Option<String>;

/// The translation table of the page and the game's companion texts; its keys are the texts
/// the page asks for (`/api/texts`).
pub(crate) const LOCALE: &str = include_str!("../../locales/telefoon.yml");

/// A screen's picture as the game took it, and its encoding once a device asked for it.
#[derive(Clone)]
pub(crate) struct Frame {
    pub seq: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
    /// The part of the texture the screen shows (pixels: x0, y0, x1, y1).
    pub crop: [u32; 4],
    /// PNG (small and sharp: a text display) rather than JPEG.
    pub png: bool,
    pub encoded: Option<(u64, Arc<Vec<u8>>, &'static str)>,
}

/// A command and where its answer goes.
pub(crate) struct Job {
    pub command: Command,
    pub reply: mpsc::Sender<Value>,
}

/// A device that asked something lately.
#[derive(Clone, Debug)]
pub(crate) struct Seen {
    pub hash: String,
    pub name: String,
    pub last: Instant,
    /// The screen it watches, if any.
    pub screen: Option<String>,
}

#[derive(Default)]
pub(crate) struct Inner {
    /// The state a device sees (JSON), and its version: one up with every change.
    pub state: String,
    pub version: u64,
    /// The game's interface language (ISO 639-1, empty for English): the page's default.
    pub language: String,
    pub frames: HashMap<String, Frame>,
    /// The number the next picture gets: one count for all of them, so that a device still
    /// waiting for one after a number it knows is not left waiting when the pictures were
    /// cleared (another bus, the screens looked for again).
    pub next_seq: u64,
    /// Screen → watched until.
    pub watch: HashMap<String, Instant>,
    /// The pictures asked for this run (said once in the log).
    pub asked: std::collections::HashSet<String>,
    pub queue: Vec<Job>,
    pub pairing: Pairing,
    pub devices_path: Option<PathBuf>,
    pub seen: Vec<Seen>,
    /// Where a device reaches the server (`http://192.168.1.20:47811/`).
    pub addresses: Vec<String>,
    /// The Cloudflare tunnel's address (`https://….trycloudflare.com`), while one runs and
    /// has said it: then the address shown, also outside the home network.
    pub public: Option<String>,
    pub stopped: bool,
    /// Things the game may tell the driver about (a device paired).
    pub notes: Vec<String>,
    /// The navigator's live picture (JSON) and its version, and until when a device shows
    /// the map: the picture is made only while one does.
    pub nav: String,
    pub nav_version: u64,
    pub nav_watch: Option<Instant>,
    /// The trip's route and stops as `/api/trip` sends them, and the map's roads.
    pub trip: Arc<String>,
    pub roads: Option<Arc<super::nav::RoadIndex>>,
    /// The devices as the page draws them (`companion::form`): per screen its form (JSON),
    /// and what lives on it (JSON) with its number (one count for all, as `next_seq`).
    pub forms: HashMap<String, Arc<String>>,
    pub lives: HashMap<String, (u64, Arc<String>)>,
    pub next_live: u64,
    /// The texture files and the fonts the forms show, by the ids the forms give them.
    pub files: Vec<FormFile>,
    pub fonts: Vec<FormFont>,
}

/// A texture file a form shows: the file, the part of it the form uses (u0, v0, u1, v1), and
/// once a device asked for it the picture of that part and the part it is.
#[derive(Clone)]
pub(crate) struct FormFile {
    pub path: PathBuf,
    pub uv: [f32; 4],
    pub encoded: Option<(Arc<Vec<u8>>, &'static str, [f32; 4])>,
}

/// A font the text textures of a form are written in, and once asked for its glyphs (JSON)
/// and its bitmap (PNG).
#[derive(Clone)]
pub(crate) struct FormFont {
    pub atlas: Arc<omsi_content::font::FontAtlas>,
    pub json: Option<Arc<String>>,
    pub png: Option<Arc<Vec<u8>>>,
}

impl Inner {
    /// The addresses a device is told to open: the tunnel's alone while there is one, else
    /// the home network's.
    pub(crate) fn shown_addresses(&self) -> Vec<String> {
        match &self.public {
            Some(p) => vec![p.clone()],
            None => self.addresses.clone(),
        }
    }
}

#[derive(Default)]
pub(crate) struct Shared {
    pub inner: Mutex<Inner>,
    pub news: Condvar,
}

impl Shared {
    pub(crate) fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A new state for the devices (nothing happens when it is the same as before).
    pub(crate) fn set_state(&self, state: String) {
        let mut i = self.lock();
        if i.state != state {
            i.state = state;
            i.version += 1;
            drop(i);
            self.news.notify_all();
        }
    }

    /// A new picture of `screen`.
    pub(crate) fn put_frame(&self, screen: &str, width: u32, height: u32, rgba: Arc<Vec<u8>>, crop: [u32; 4], png: bool) {
        let mut i = self.lock();
        i.next_seq += 1;
        let seq = i.next_seq;
        i.frames.insert(screen.to_string(), Frame { seq, width, height, rgba, crop, png, encoded: None });
        drop(i);
        self.news.notify_all();
    }

    /// Whether a device watches `screen` now.
    pub(crate) fn watched(&self, screen: &str, now: Instant) -> bool {
        self.lock().watch.get(screen).is_some_and(|t| *t > now)
    }

    /// The commands the devices sent since the last call.
    pub(crate) fn take_jobs(&self) -> Vec<Job> {
        std::mem::take(&mut self.lock().queue)
    }

    /// A new live picture of the navigator (nothing happens when it is the same).
    pub(crate) fn set_nav(&self, nav: String) {
        let mut i = self.lock();
        if i.nav != nav {
            i.nav = nav;
            i.nav_version += 1;
            drop(i);
            self.news.notify_all();
        }
    }

    /// Whether a device shows the navigator's map now.
    pub(crate) fn nav_watched(&self, now: Instant) -> bool {
        self.lock().nav_watch.is_some_and(|t| t > now)
    }

    pub(crate) fn set_trip(&self, trip: String) {
        self.lock().trip = Arc::new(trip);
    }

    pub(crate) fn set_roads(&self, roads: super::nav::RoadIndex) {
        self.lock().roads = Some(Arc::new(roads));
    }

    /// What lives on `screen`'s form now (nothing happens when it is the same).
    pub(crate) fn set_live(&self, screen: &str, live: String) {
        let mut i = self.lock();
        if i.lives.get(screen).is_some_and(|l| *l.1 == live) {
            return;
        }
        i.next_live += 1;
        let n = i.next_live;
        i.lives.insert(screen.to_string(), (n, Arc::new(live)));
        drop(i);
        self.news.notify_all();
    }
}

/// The name under which a device watching `screen`'s form is kept (`Inner::watch`).
pub(crate) fn live_key(screen: &str) -> String {
    format!("l{screen}")
}

/// The name under which a script texture's whole picture is kept for the forms
/// (`Inner::frames`, `Inner::watch`).
pub(crate) fn script_key(n: usize) -> String {
    format!("x{n}")
}

/// The longest side of a texture a form sends (pixels): bigger parts go at half size.
const TEX_MAX: u32 = 2048;

/// How long a request for the live picture waits for a newer one.
const NAV_WAIT: Duration = Duration::from_secs(3);

/// The name a screen's device picture is kept under beside the screen's own (`Inner::frames`,
/// `Inner::watch`).
pub(crate) fn view_key(screen: &str) -> String {
    format!("v{screen}")
}

/// The address a device opens to pair with `code` at once (`http://192.168.1.20:47811/?pair=
/// 463055`, or the Cloudflare tunnel's `https://….trycloudflare.com/?pair=…`): the page reads
/// the code from it and pairs without typing.
pub(crate) fn pair_url(address: &str, code: &str) -> Option<String> {
    let address = address.trim().trim_end_matches('/');
    if address.is_empty() || code.is_empty() {
        return None;
    }
    let address = if address.starts_with("http://") || address.starts_with("https://") { address.to_string() } else { format!("http://{address}") };
    Some(format!("{address}/?pair={code}"))
}

/// Where another device reaches the server, for the QR code a paired device shows: the
/// address this device used, when it is one on the network (the other device is beside it),
/// else the first one the game knows (the tunnel's, while there is one).
fn qr_address(host: Option<&str>, known: &[String]) -> Option<String> {
    if let Some(public) = known.first().filter(|a| a.starts_with("https://")) {
        return Some(public.clone());
    }
    let host = host.map(str::trim).filter(|h| !h.is_empty());
    let on_network = host.filter(|h| {
        let ip = h.rsplit_once(':').map_or(*h, |(ip, _)| ip).trim_start_matches('[').trim_end_matches(']');
        ip.parse::<std::net::IpAddr>().is_ok_and(|ip| !ip.is_loopback() && lan_peer(ip))
    });
    on_network.map(str::to_string).or_else(|| known.first().cloned()).or_else(|| host.map(str::to_string))
}

/// The running server; dropping it stops it.
pub(crate) struct Server {
    pub addr: SocketAddr,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.shared.lock().stopped = true;
        self.shared.news.notify_all();
        log::info!("companion: server on {} stopped", self.addr);
    }
}

impl Server {
    /// Listen on `listen`. `find_addresses`: look up this machine's addresses on the network
    /// for the devices (on a thread: on Windows that runs `ipconfig`).
    pub(crate) fn start(listen: SocketAddr, shared: Arc<Shared>, lookup: Lookup, find_addresses: bool) -> std::io::Result<Server> {
        let listener = TcpListener::bind(listen)?;
        let addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        {
            let mut i = shared.lock();
            i.stopped = false;
            i.addresses = if addr.ip().is_unspecified() { Vec::new() } else { vec![format!("http://{addr}/")] };
        }
        let (st, sh) = (stop.clone(), shared.clone());
        std::thread::Builder::new().name("companion".into()).spawn(move || {
            let open = Arc::new(AtomicUsize::new(0));
            while !st.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, peer)) => {
                        if !lan_peer(peer.ip()) {
                            log::info!("companion: {peer} is not on the home network: closed");
                            continue;
                        }
                        if open.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                            log::debug!("companion: {peer}: {MAX_CONNECTIONS} connections open already; closed");
                            continue;
                        }
                        open.fetch_add(1, Ordering::Relaxed);
                        let (st, sh, held) = (st.clone(), sh.clone(), open.clone());
                        let spawned = std::thread::Builder::new().name("companion device".into()).spawn(move || {
                            if let Err(e) = serve(stream, &sh, lookup, &st) {
                                log::debug!("companion: {peer}: {e}");
                            }
                            held.fetch_sub(1, Ordering::Relaxed);
                        });
                        if spawned.is_err() {
                            open.fetch_sub(1, Ordering::Relaxed);
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(40)),
                    Err(e) => {
                        log::warn!("companion: accept: {e}");
                        std::thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        })?;
        if find_addresses && addr.ip().is_unspecified() {
            let sh = shared.clone();
            let port = addr.port();
            let _ = std::thread::Builder::new().name("companion addresses".into()).spawn(move || {
                let list = lan_urls(port);
                log::info!("companion: a phone or tablet on this network opens {}", list.first().map(String::as_str).unwrap_or("(no network address found)"));
                sh.lock().addresses = list;
                sh.news.notify_all();
            });
        }
        log::info!("companion: listening on {addr}");
        Ok(Server { addr, shared, stop })
    }
}

/// Whether a device at `ip` is on the home network: this machine, a private network (a
/// router's 192.168.x.x, 10.x, 172.16-31.x) or a link without a router. A computer with an
/// address straight on the internet listens there too, and nobody from there gets in.
pub(crate) fn lan_peer(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        std::net::IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => lan_peer(v4.into()),
            // (unique local fc00::/7 and link-local fe80::/10)
            None => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80,
        },
    }
}

/// The addresses of this machine a phone on the same network reaches, the home network's
/// first (no loopback, no virtual machine bridges, no link-local addresses).
fn lan_urls(port: u16) -> Vec<String> {
    let mut list = omsi_net::addrs::joinable_addresses();
    list.retain(|a| lan_peer(a.ip.into()));
    list.sort_by_key(|a| a.kind != omsi_net::addrs::AddrKind::Lan);
    list.iter().map(|a| format!("http://{}:{port}/", a.ip)).collect()
}

/// Read one request from `s` and answer it.
fn serve(mut s: TcpStream, shared: &Shared, lookup: Lookup, stop: &AtomicBool) -> std::io::Result<()> {
    s.set_nonblocking(false)?;
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    s.set_write_timeout(Some(Duration::from_secs(10)))?;
    let _ = s.set_nodelay(true);
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    let (mut req, at) = loop {
        match http::parse_head(&buf) {
            Ok(r) => break r,
            Err(http::HttpError::Incomplete) => {}
            Err(e) => {
                let status = if e == http::HttpError::TooLarge { 413 } else { 400 };
                return s.write_all(&Response::status(status).to_bytes(false));
            }
        }
        let n = s.read(&mut chunk)?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let want = match http::body_length(&req) {
        Ok(n) => n,
        Err(e) => {
            let status = if e == http::HttpError::TooLarge { 413 } else { 400 };
            return s.write_all(&Response::status(status).to_bytes(false));
        }
    };
    while buf.len() < at + want {
        let n = s.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    req.body = buf.get(at..(at + want).min(buf.len())).unwrap_or_default().to_vec();
    let answer = handle(&req, shared, lookup, stop);
    s.write_all(&answer.to_bytes(req.method == "HEAD"))?;
    s.flush()
}

/// What a browser says it is, short: "Safari on iPhone".
pub(crate) fn device_name(user_agent: &str) -> String {
    let ua = user_agent;
    let browser = if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("Firefox/") || ua.contains("FxiOS") {
        "Firefox"
    } else if ua.contains("Chrome/") || ua.contains("CriOS") {
        "Chrome"
    } else if ua.contains("Safari/") {
        "Safari"
    } else {
        "Browser"
    };
    let system = if ua.contains("iPad") {
        "iPad"
    } else if ua.contains("iPhone") {
        "iPhone"
    } else if ua.contains("Android") {
        "Android"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Mac OS") {
        "Mac"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        ""
    };
    if system.is_empty() { browser.to_string() } else { format!("{browser} on {system}") }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Seconds of a steady clock (for the pairing's count of wrong codes).
fn steady() -> f64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// The keys of the translation table (the texts the page may ask for).
pub(crate) fn locale_keys(table: &str) -> Vec<String> {
    table
        .lines()
        .filter(|l| l.starts_with('"'))
        .filter_map(|l| l.trim_end().strip_suffix(':'))
        .filter_map(|k| k.strip_prefix('"')?.strip_suffix('"').map(|k| k.replace("\\\"", "\"").replace("\\\\", "\\")))
        .collect()
}

/// Texts of the game's own table (`app.yml`) the page shows as well: the navigator's duty
/// board and sheet (`nav_duty`) and its route notes say the same on a phone.
const ALSO: &[&str] = &[
    "Duty",
    "Line",
    "Tour",
    "Back",
    "No timetable on this map",
    "Loading…",
    "%{m} min break",
    "%{m} min over",
    "%{n} stops",
    "%{n} trip",
    "%{n} trips",
    "Bus stop signs",
    "Continue on line %{line}, tour %{tour}",
    "Depot file",
    "Driving without a duty",
    "Duty finished",
    "Empty run",
    "Final stop",
    "In the game",
    "Map",
    "More",
    "Next %{time}",
    "No duty running",
    "Off route",
    "On break",
    "Recalculating route",
    "Route",
    "Route recalculated",
    "Slow traffic",
    "Traffic jam",
    "Trip",
    "and %{n} more to %{terminus}",
    "arrives %{time}",
    "break, %{m} min left",
    "leaves %{time}",
    "leaves in %{m} min",
    "on time",
    "terminus",
    "trip %{k} of %{n}",
];

/// The page's texts in `lang`: every key of the table with its translation (the English
/// text itself where there is none).
pub(crate) fn texts(lang: &str, lookup: Lookup) -> Value {
    let map: serde_json::Map<String, Value> = locale_keys(LOCALE)
        .into_iter()
        .chain(ALSO.iter().map(|k| k.to_string()))
        .map(|k| {
            let t = if lang.is_empty() || lang == "en" { None } else { lookup(lang, &k) };
            let v = Value::String(t.unwrap_or_else(|| k.clone()));
            (k, v)
        })
        .collect();
    json!({ "lang": lang, "texts": map })
}

/// The answer to one request.
pub(crate) fn handle(req: &Request, shared: &Shared, lookup: Lookup, stop: &AtomicBool) -> Response {
    let route = http::route(req);
    if route.needs_key() {
        let key = req.header("x-companion-key").unwrap_or("");
        let mut i = shared.lock();
        let Some(d) = i.pairing.device_of(key) else {
            return Response::json(&json!({ "error": "not_paired" })).with_status(403);
        };
        let hash = i.pairing.devices[d].hash.clone();
        let name = i.pairing.devices[d].name.clone();
        let screen = match &route {
            Route::Frame(s, _) | Route::View(s, _) | Route::Live(s, _) if !s.starts_with('x') => Some(s.clone()),
            _ => None,
        };
        let now = Instant::now();
        match i.seen.iter_mut().find(|s| s.hash == hash) {
            Some(s) => {
                s.last = now;
                if screen.is_some() {
                    s.screen = screen;
                }
            }
            None => i.seen.push(Seen { hash, name, last: now, screen }),
        }
    }
    match route {
        Route::Asset(k) => {
            let (_, ctype, body) = http::ASSETS[k];
            let r = Response::new(200, ctype, body);
            if k == 0 { r.with("Content-Security-Policy", http::CSP) } else { r }
        }
        Route::Manifest => Response::new(
            200,
            "application/manifest+json",
            json!({
                "name": "openOMSI",
                "short_name": "openOMSI",
                "start_url": "./",
                "scope": "./",
                "display": "standalone",
                "orientation": "any",
                "background_color": "#0e1522",
                "theme_color": "#0e1522",
                "icons": [{ "src": "icon.svg", "sizes": "any", "type": "image/svg+xml" }]
            })
            .to_string(),
        ),
        Route::Texts(lang) => {
            let (lang, pair_len) = {
                let i = shared.lock();
                (if lang.is_empty() { i.language.clone() } else { lang }, i.pairing.code_len())
            };
            let mut v = texts(&lang, lookup);
            // (the pairing page's boxes: nine digits through the Cloudflare tunnel)
            v["pair_len"] = json!(pair_len);
            Response::json(&v)
        }
        Route::Pair => {
            let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let code = body.get("code").and_then(|c| c.as_str()).unwrap_or("");
            let name = device_name(req.header("user-agent").unwrap_or(""));
            let mut i = shared.lock();
            let outcome = i.pairing.pair(code, &name, steady(), unix_now(), &mut super::pairing_code, &mut super::device_key);
            match outcome {
                Ok(key) => {
                    if let Some(path) = i.devices_path.clone() {
                        if let Err(e) = super::signon::write_private(&path, &super::pairing::devices_json(&i.pairing.devices)) {
                            log::warn!("companion: cannot write {}: {e}", path.display());
                        }
                    }
                    log::info!("companion: {name} paired");
                    i.notes.push(name);
                    Response::json(&json!({ "key": key }))
                }
                Err(Refused::Wrong) => Response::json(&json!({ "error": "wrong" })).with_status(403),
                Err(Refused::Locked) => Response::json(&json!({ "error": "locked" })).with_status(429),
            }
        }
        Route::State(after) => {
            let deadline = Instant::now() + STATE_WAIT;
            let mut i = shared.lock();
            // (a version from before the game started again: the state now, at once)
            let after = if after > i.version { 0 } else { after };
            while i.version <= after && !i.stopped && !stop.load(Ordering::Relaxed) {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                i = shared.news.wait_timeout(i, left).map(|r| r.0).unwrap_or_else(|e| e.into_inner().0);
            }
            let body = format!("{{\"v\":{},\"state\":{}}}", i.version, if i.state.is_empty() { "null" } else { i.state.as_str() });
            Response::new(200, "application/json; charset=utf-8", body)
        }
        Route::Frame(screen, after) => frame(shared, &screen, after, stop),
        // (the device's picture is kept beside the screen's, under its own name)
        Route::View(screen, after) => frame(shared, &view_key(&screen), after, stop),
        Route::Form(screen) => match shared.lock().forms.get(&screen).cloned() {
            Some(f) => Response::new(200, "application/json; charset=utf-8", f.as_str()).with("Cache-Control", "no-store"),
            None => Response::status(404),
        },
        Route::Live(screen, after) => {
            let deadline = Instant::now() + FRAME_WAIT;
            let mut i = shared.lock();
            let after = if after > i.next_live { 0 } else { after };
            i.watch.insert(live_key(&screen), Instant::now() + WATCH_FOR);
            loop {
                if let Some((n, live)) = i.lives.get(&screen).filter(|l| l.0 > after) {
                    let body = format!("{{\"v\":{n},\"live\":{live}}}");
                    return Response::new(200, "application/json; charset=utf-8", body);
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() || i.stopped || stop.load(Ordering::Relaxed) || !i.forms.contains_key(&screen) {
                    return Response::status(204);
                }
                i = shared.news.wait_timeout(i, left).map(|r| r.0).unwrap_or_else(|e| e.into_inner().0);
            }
        }
        Route::Tex(n) => form_texture(shared, n),
        Route::Font(n) => {
            let Some(f) = shared.lock().fonts.get(n).cloned() else { return Response::status(404) };
            let json = f.json.clone().unwrap_or_else(|| Arc::new(super::form::font_json(&f.atlas).to_string()));
            if let Some(g) = shared.lock().fonts.get_mut(n).filter(|g| Arc::ptr_eq(&g.atlas, &f.atlas)) {
                g.json = Some(json.clone());
            }
            Response::new(200, "application/json; charset=utf-8", json.as_str()).with("Cache-Control", "no-store")
        }
        Route::FontImg(n) => {
            let Some(f) = shared.lock().fonts.get(n).cloned() else { return Response::status(404) };
            let png = match f.png.clone() {
                Some(p) => p,
                None => {
                    let rgba = super::form::font_rgba(&f.atlas);
                    let Some((bytes, _)) = encode(&rgba, f.atlas.width, f.atlas.height, [0, 0, f.atlas.width, f.atlas.height], true) else { return Response::status(404) };
                    let bytes = Arc::new(bytes);
                    if let Some(g) = shared.lock().fonts.get_mut(n).filter(|g| Arc::ptr_eq(&g.atlas, &f.atlas)) {
                        g.png = Some(bytes.clone());
                    }
                    bytes
                }
            };
            Response::new(200, "image/png", png.as_slice()).with("Cache-Control", "no-store")
        }
        Route::Do => {
            let Some(command) = serde_json::from_slice::<Value>(&req.body).ok().as_ref().and_then(Command::from_json) else {
                return Response::status(400);
            };
            let (tx, rx) = mpsc::channel();
            shared.lock().queue.push(Job { command, reply: tx });
            match rx.recv_timeout(COMMAND_WAIT) {
                Ok(v) => Response::json(&v),
                // (the game is busy loading, or gone)
                Err(_) => Response::json(&json!({ "error": "busy" })).with_status(503),
            }
        }
        Route::Nav(after) => {
            let deadline = Instant::now() + NAV_WAIT;
            let mut i = shared.lock();
            i.nav_watch = Some(Instant::now() + WATCH_FOR);
            let after = if after > i.nav_version { 0 } else { after };
            while i.nav_version <= after && !i.stopped && !stop.load(Ordering::Relaxed) {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                i = shared.news.wait_timeout(i, left).map(|r| r.0).unwrap_or_else(|e| e.into_inner().0);
            }
            let body = format!("{{\"v\":{},\"nav\":{}}}", i.nav_version, if i.nav.is_empty() { "null" } else { i.nav.as_str() });
            Response::new(200, "application/json; charset=utf-8", body)
        }
        Route::Trip => {
            let mut i = shared.lock();
            i.nav_watch = Some(Instant::now() + WATCH_FOR);
            let trip = i.trip.clone();
            drop(i);
            Response::new(200, "application/json; charset=utf-8", if trip.is_empty() { "{\"v\":0}".to_string() } else { trip.as_str().to_string() })
        }
        Route::Roads { x, y, r, tol } => {
            let roads = shared.lock().roads.clone();
            // (made without the lock: the game goes on meanwhile)
            match roads {
                Some(idx) => Response::new(200, "application/json; charset=utf-8", idx.json_near(glam::DVec2::new(x, y), r, tol)),
                None => Response::json(&json!({ "v": 0, "x": x, "y": y, "r": r, "roads": [] })),
            }
        }
        Route::PairQr => {
            let (url, code) = {
                let i = shared.lock();
                let code = i.pairing.code().to_string();
                (qr_address(req.header("host"), &i.shown_addresses()).and_then(|a| pair_url(&a, &code)), code)
            };
            match url.as_deref().and_then(|u| super::qr::encode_address(u).map(|q| (u, q))) {
                Some((u, q)) => Response::json(&json!({ "url": u, "code": code, "size": q.size, "bits": q.bits() })),
                None => Response::json(&json!({ "url": Value::Null, "code": code })),
            }
        }
        Route::Company => Response::json(&super::company::state()).with("Cache-Control", "no-store"),
        Route::CompanyOrder => {
            let (status, v) = super::company::order(&req.body);
            Response::json(&v).with_status(status)
        }
        Route::NotFound => Response::status(404),
        Route::Method => Response::status(405),
    }
}

impl Response {
    fn with_status(mut self, status: u16) -> Response {
        self.status = status;
        self
    }
}

/// A picture of `screen` newer than `after`, encoded (204 when none came in time).
fn frame(shared: &Shared, screen: &str, after: u64, stop: &AtomicBool) -> Response {
    let deadline = Instant::now() + FRAME_WAIT;
    let mut i = shared.lock();
    // (a picture from before the game started again: the next one)
    let after = if after > i.next_seq { 0 } else { after };
    // (once a run per picture asked for: whether a device's picture is asked for at all shows
    // in the log)
    if !i.watch.contains_key(screen) && i.asked.insert(screen.to_string()) {
        log::info!("companion: a device asks for the picture '{screen}'");
    }
    i.watch.insert(screen.to_string(), Instant::now() + WATCH_FOR);
    loop {
        if i.frames.get(screen).is_some_and(|f| f.seq > after) {
            break;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || i.stopped || stop.load(Ordering::Relaxed) {
            return Response::status(204);
        }
        i = shared.news.wait_timeout(i, left).map(|r| r.0).unwrap_or_else(|e| e.into_inner().0);
    }
    let f = i.frames[screen].clone();
    drop(i);
    let (bytes, ctype) = match f.encoded.filter(|e| e.0 == f.seq) {
        Some((_, bytes, ctype)) => (bytes, ctype),
        None => {
            // (without the lock: the game goes on meanwhile)
            let Some((bytes, ctype)) = encode(&f.rgba, f.width, f.height, f.crop, f.png) else {
                return Response::status(204);
            };
            let bytes = Arc::new(bytes);
            if let Some(g) = shared.lock().frames.get_mut(screen).filter(|g| g.seq == f.seq) {
                g.encoded = Some((f.seq, bytes.clone(), ctype));
            }
            (bytes, ctype)
        }
    };
    Response::new(200, ctype, bytes.as_slice()).with("X-Seq", f.seq.to_string())
}

/// Texture file `n` of the forms, the part of it they use: decoded and encoded on the
/// connection's thread the first time a device asks (`X-Uv` says which part of the texture it
/// is: u0, v0, u1, v1).
fn form_texture(shared: &Shared, n: usize) -> Response {
    let Some(f) = shared.lock().files.get(n).cloned() else { return Response::status(404) };
    let (bytes, ctype, uv) = match f.encoded.clone() {
        Some(e) => e,
        None => {
            let Some(e) = encode_part(&f.path, f.uv) else { return Response::status(404) };
            if let Some(g) = shared.lock().files.get_mut(n).filter(|g| g.path == f.path) {
                g.encoded = Some(e.clone());
            }
            e
        }
    };
    Response::new(200, ctype, bytes.as_slice()).with("X-Uv", format!("{},{},{},{}", uv[0], uv[1], uv[2], uv[3])).with("Cache-Control", "no-store")
}

/// The part `uv` of a texture file as a picture (PNG, or JPEG for a big one without
/// transparency) and the part of the texture it is (a pixel more round it: the page samples
/// between pixels).
pub(crate) fn encode_part(path: &std::path::Path, uv: [f32; 4]) -> Option<(Arc<Vec<u8>>, &'static str, [f32; 4])> {
    let (w, h, part, cw, ch, uv) = texture_part(path, uv)?;
    let _ = (w, h);
    let opaque = part.chunks_exact(4).all(|p| p[3] == 255);
    let (bytes, ctype) = encode(&part, cw, ch, [0, 0, cw, ch], !(opaque && cw * ch > 512 * 512))?;
    Some((Arc::new(bytes), ctype, uv))
}

/// The part `uv` of a texture file as RGBA: the file's size, the part, its size and the part
/// of the texture it is.
pub(crate) fn texture_part(path: &std::path::Path, uv: [f32; 4]) -> Option<(u32, u32, Vec<u8>, u32, u32, [f32; 4])> {
    let img = match omsi_texture::decode_file(path) {
        Ok(i) => i,
        Err(e) => {
            log::warn!("companion: cannot read the texture {}: {e}", path.display());
            return None;
        }
    };
    let (w, h) = (img.width, img.height);
    if w == 0 || h == 0 || img.rgba.len() < (w * h * 4) as usize {
        return None;
    }
    let px = |t: f32, n: u32, up: bool| -> u32 { ((if up { (t * n as f32).ceil() + 1.0 } else { (t * n as f32).floor() - 1.0 }).max(0.0) as u32).min(n) };
    let crop = [px(uv[0], w, false), px(uv[1], h, false), px(uv[2], w, true), px(uv[3], h, true)];
    let crop = [crop[0].min(w - 1), crop[1].min(h - 1), crop[2].max(crop[0] + 1).min(w), crop[3].max(crop[1] + 1).min(h)];
    let crop = [crop[0], crop[1], crop[2].max(crop[0] + 1), crop[3].max(crop[1] + 1)];
    let out_uv = [crop[0] as f32 / w as f32, crop[1] as f32 / h as f32, crop[2] as f32 / w as f32, crop[3] as f32 / h as f32];
    let (cw, ch) = (crop[2] - crop[0], crop[3] - crop[1]);
    let mut part = Vec::with_capacity((cw * ch * 4) as usize);
    for y in crop[1]..crop[3] {
        let row = ((y * w + crop[0]) * 4) as usize;
        part.extend_from_slice(&img.rgba[row..row + (cw * 4) as usize]);
    }
    // (a very big part at half its size)
    if cw.max(ch) > TEX_MAX {
        let small = image::imageops::resize(&image::RgbaImage::from_raw(cw, ch, part)?, cw.div_ceil(2), ch.div_ceil(2), image::imageops::FilterType::Triangle);
        let (sw, sh) = small.dimensions();
        return Some((w, h, small.into_raw(), sw, sh, out_uv));
    }
    Some((w, h, part, cw, ch, out_uv))
}

/// The part `crop` (pixels: x0, y0, x1, y1) of an RGBA picture as PNG (`png`, keeping the
/// transparency: a text display's letters on the page's own background) or as JPEG (laid on
/// black).
pub(crate) fn encode(rgba: &[u8], width: u32, height: u32, crop: [u32; 4], png: bool) -> Option<(Vec<u8>, &'static str)> {
    use image::ImageEncoder;
    let [x0, y0, x1, y1] = [crop[0].min(width), crop[1].min(height), crop[2].min(width), crop[3].min(height)];
    let (w, h) = (x1.checked_sub(x0)?, y1.checked_sub(y0)?);
    if w == 0 || h == 0 || rgba.len() < (width * height * 4) as usize {
        return None;
    }
    let mut out = Vec::new();
    if png {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in y0..y1 {
            let row = ((y * width + x0) * 4) as usize;
            px.extend_from_slice(&rgba[row..row + (w * 4) as usize]);
        }
        let enc = image::codecs::png::PngEncoder::new_with_quality(&mut out, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Sub);
        enc.write_image(&px, w, h, image::ExtendedColorType::Rgba8).ok()?;
        Some((out, "image/png"))
    } else {
        let mut px = Vec::with_capacity((w * h * 3) as usize);
        for y in y0..y1 {
            let row = ((y * width + x0) * 4) as usize;
            for p in rgba[row..row + (w * 4) as usize].chunks_exact(4) {
                let a = p[3] as u32;
                px.extend_from_slice(&[(p[0] as u32 * a / 255) as u8, (p[1] as u32 * a / 255) as u8, (p[2] as u32 * a / 255) as u8]);
            }
        }
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80).encode(&px, w, h, image::ExtendedColorType::Rgb8).ok()?;
        Some((out, "image/jpeg"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(lang: &str, key: &str) -> Option<String> {
        (lang == "nl" && key == "Sign on").then(|| "Aanmelden".to_string())
    }

    /// One request over a real socket; (status, headers, body).
    fn call(addr: SocketAddr, method: &str, path: &str, key: Option<&str>, body: Option<&str>) -> (u16, String, Vec<u8>) {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        let mut req = format!("{method} {path} HTTP/1.1\r\nHost: test\r\nUser-Agent: Mozilla/5.0 (iPhone) Safari/604.1\r\n");
        if let Some(k) = key {
            req.push_str(&format!("X-Companion-Key: {k}\r\n"));
        }
        let body = body.unwrap_or("");
        req.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
        s.write_all(req.as_bytes()).unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).unwrap();
        let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&out[..split]).to_string();
        let status = head[9..12].parse().unwrap();
        (status, head, out[split + 4..].to_vec())
    }

    fn json_of(b: &[u8]) -> Value {
        serde_json::from_slice(b).unwrap()
    }

    #[test]
    fn a_device_pairs_follows_the_state_sends_commands_and_gets_pictures() {
        let shared = Arc::new(Shared::default());
        shared.lock().pairing = Pairing::new("123456".into(), Vec::new());
        shared.set_state(json!({ "stage": "sign_on" }).to_string());
        let server = Server::start("127.0.0.1:0".parse().unwrap(), shared.clone(), lookup, false).unwrap();
        let addr = server.addr;

        // the page and its files are there without a key, nothing else is
        let (status, head, body) = call(addr, "GET", "/", None, None);
        assert_eq!(status, 200);
        assert!(head.contains("Content-Security-Policy"));
        assert!(String::from_utf8_lossy(&body).contains("app.js"));
        assert_eq!(call(addr, "GET", "/api/state", None, None).0, 403);
        assert_eq!(call(addr, "GET", "/api/state", Some("00000000000000000000000000000000"), None).0, 403);
        assert_eq!(call(addr, "GET", "/etc/passwd", None, None).0, 404);

        // the texts, in the game's language unless asked otherwise
        shared.lock().language = "nl".into();
        let t = json_of(&call(addr, "GET", "/api/texts", None, None).2);
        assert_eq!(t["texts"]["Sign on"], "Aanmelden");
        let t = json_of(&call(addr, "GET", "/api/texts?lang=en", None, None).2);
        assert_eq!(t["texts"]["Sign on"], "Sign on");

        // pairing: a wrong code, then the right one (which is replaced afterwards)
        assert_eq!(call(addr, "POST", "/api/pair", None, Some(r#"{"code":"999999"}"#)).0, 403);
        let (status, _, body) = call(addr, "POST", "/api/pair", None, Some(r#"{"code":"123456"}"#));
        assert_eq!(status, 200);
        let key = json_of(&body)["key"].as_str().unwrap().to_string();
        assert_ne!(shared.lock().pairing.code(), "123456");
        assert_eq!(shared.lock().notes, vec!["Safari on iPhone".to_string()]);

        // the state at once, then a long poll answered by the next change
        let s = json_of(&call(addr, "GET", "/api/state?after=0", Some(&key), None).2);
        assert_eq!(s["state"]["stage"], "sign_on");
        let v = s["v"].as_u64().unwrap();
        let waiter = {
            let key = key.clone();
            std::thread::spawn(move || json_of(&call(addr, "GET", &format!("/api/state?after={v}"), Some(&key), None).2))
        };
        std::thread::sleep(Duration::from_millis(150));
        shared.set_state(json!({ "stage": "duty_menu" }).to_string());
        assert_eq!(waiter.join().unwrap()["state"]["stage"], "duty_menu");
        assert_eq!(shared.lock().seen.len(), 1);
        // a version from before the game started again is answered at once
        let t0 = Instant::now();
        let s = json_of(&call(addr, "GET", "/api/state?after=999", Some(&key), None).2);
        assert!(t0.elapsed() < Duration::from_secs(5));
        assert_eq!(s["state"]["stage"], "duty_menu");

        // a command reaches the game's queue and the game's answer comes back
        let game = {
            let shared = shared.clone();
            std::thread::spawn(move || {
                for _ in 0..200 {
                    if let Some(job) = shared.take_jobs().pop() {
                        assert_eq!(job.command, Command::SignOn { number: "482913".into(), code: None });
                        job.reply.send(json!({ "result": "number" })).unwrap();
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                panic!("no command came");
            })
        };
        let (status, _, body) = call(addr, "POST", "/api/do", Some(&key), Some(r#"{"do":"sign_on","number":"482913"}"#));
        game.join().unwrap();
        assert_eq!((status, json_of(&body)["result"].as_str()), (200, Some("number")));
        assert_eq!(call(addr, "POST", "/api/do", Some(&key), Some(r#"{"do":"format_disk"}"#)).0, 400);

        // a picture: asked for, the screen counts as watched; a new picture answers it
        assert!(!shared.watched("s3", Instant::now()));
        let getter = {
            let key = key.clone();
            std::thread::spawn(move || call(addr, "GET", "/api/frame?screen=s3&after=0", Some(&key), None))
        };
        let t0 = Instant::now();
        while !shared.watched("s3", Instant::now()) && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(shared.watched("s3", Instant::now()));
        let mut px = vec![0u8; 8 * 4 * 4];
        px.iter_mut().enumerate().for_each(|(i, b)| *b = if i % 4 == 3 { 255 } else { (i * 7) as u8 });
        shared.put_frame("s3", 8, 4, Arc::new(px), [0, 0, 8, 4], true);
        let (status, head, body) = getter.join().unwrap();
        assert_eq!(status, 200);
        assert!(head.contains("Content-Type: image/png"));
        assert!(head.contains("X-Seq: 1"));
        assert_eq!(&body[1..4], b"PNG");
        // nothing newer within the wait: 204
        assert_eq!(call(addr, "GET", "/api/frame?screen=s3&after=1", Some(&key), None).0, 204);

        drop(server);
        assert!(shared.lock().stopped);
    }

    #[test]
    fn a_device_follows_the_navigator_and_shows_the_pairing_qr_code() {
        let shared = Arc::new(Shared::default());
        shared.lock().pairing = Pairing::new("463055".into(), Vec::new());
        let server = Server::start("127.0.0.1:0".parse().unwrap(), shared.clone(), lookup, false).unwrap();
        let addr = server.addr;
        let key = json_of(&call(addr, "POST", "/api/pair", None, Some(r#"{"code":"463055"}"#)).2)["key"].as_str().unwrap().to_string();
        for path in ["/api/nav", "/api/trip", "/api/roads?x=0&y=0", "/api/qr"] {
            assert_eq!(call(addr, "GET", path, None, None).0, 403, "{path} without a key");
        }

        // nobody watches the map until a device asks for the picture; then it is watched
        // and the next picture answers the waiting request
        assert!(!shared.nav_watched(Instant::now()));
        let j = json_of(&call(addr, "GET", "/api/nav?after=0", Some(&key), None).2);
        assert_eq!((j["v"].as_u64(), j["nav"].clone()), (Some(0), Value::Null));
        assert!(shared.nav_watched(Instant::now()));
        let waiter = {
            let key = key.clone();
            std::thread::spawn(move || json_of(&call(addr, "GET", "/api/nav?after=0", Some(&key), None).2))
        };
        std::thread::sleep(Duration::from_millis(150));
        shared.set_nav(json!({ "bus": [1.0, 2.0] }).to_string());
        let j = waiter.join().unwrap();
        assert_eq!((j["v"].as_u64(), j["nav"]["bus"].clone()), (Some(1), json!([1.0, 2.0])));
        // the same picture again is no news
        shared.set_nav(json!({ "bus": [1.0, 2.0] }).to_string());
        assert_eq!(shared.lock().nav_version, 1);

        // the trip, and the roads once they are made
        assert_eq!(json_of(&call(addr, "GET", "/api/trip", Some(&key), None).2)["v"], 0);
        shared.set_trip(json!({ "v": 4, "pts": [0.0, 0.0, 10.0, 0.0] }).to_string());
        assert_eq!(json_of(&call(addr, "GET", "/api/trip", Some(&key), None).2)["v"], 4);
        assert_eq!(json_of(&call(addr, "GET", "/api/roads?x=0&y=0", Some(&key), None).2)["roads"], json!([]));
        let road = crate::navigator::MapRoad { points: vec![glam::DVec3::new(-50.0, 0.0, 0.0), glam::DVec3::new(50.0, 0.0, 0.0)], width: 6.0, main: false };
        shared.set_roads(super::super::nav::RoadIndex::build(2, vec![road]));
        let j = json_of(&call(addr, "GET", "/api/roads?x=0&y=0&r=300", Some(&key), None).2);
        assert_eq!((j["v"].as_u64(), j["roads"].clone()), (Some(2), json!([[60, 0, -500, 0, 500, 0]])));

        // the QR code: the address the device used is no network address (the test's host
        // is "test"), so the game's own goes into it, with the code as it is now
        shared.lock().addresses = vec!["http://192.168.1.20:47811/".into()];
        let j = json_of(&call(addr, "GET", "/api/qr", Some(&key), None).2);
        let code = shared.lock().pairing.code().to_string();
        assert_eq!(j["url"].as_str(), Some(format!("http://192.168.1.20:47811/?pair={code}").as_str()));
        let size = j["size"].as_u64().unwrap() as usize;
        assert_eq!(j["bits"].as_str().unwrap().len(), size * size);
        assert_eq!(size, 29, "an address with a code fits version 3");
        drop(server);
    }

    #[test]
    fn the_pairing_address_is_the_one_the_other_device_reaches() {
        assert_eq!(pair_url("http://192.168.1.20:47811/", "123456").as_deref(), Some("http://192.168.1.20:47811/?pair=123456"));
        assert_eq!(pair_url("10.0.0.5:5000", "1").as_deref(), Some("http://10.0.0.5:5000/?pair=1"));
        assert_eq!(pair_url("http://x/", ""), None);
        let known = vec!["http://192.168.1.20:47811/".to_string()];
        assert_eq!(qr_address(Some("192.168.178.30:47811"), &known).as_deref(), Some("192.168.178.30:47811"));
        assert_eq!(qr_address(Some("[fd00::7]:47811"), &known).as_deref(), Some("[fd00::7]:47811"));
        assert_eq!(qr_address(Some("127.0.0.1:47811"), &known).as_deref(), Some("http://192.168.1.20:47811/"));
        assert_eq!(qr_address(Some("localhost:47811"), &[]).as_deref(), Some("localhost:47811"));
        assert_eq!(qr_address(None, &[]), None);
    }

    #[test]
    fn only_the_home_network_gets_in() {
        let ok = |s: &str| lan_peer(s.parse().unwrap());
        for home in ["127.0.0.1", "192.168.1.23", "10.0.0.7", "172.20.1.1", "169.254.3.4", "::1", "fd12::5", "fe80::1", "::ffff:192.168.0.9"] {
            assert!(ok(home), "{home}");
        }
        for away in ["8.8.8.8", "172.32.0.1", "100.70.1.2", "2a00:1450::1", "::ffff:1.2.3.4"] {
            assert!(!ok(away), "{away}");
        }
    }

    #[test]
    fn a_picture_is_cut_to_the_part_the_screen_shows() {
        let (w, h) = (16u32, 8u32);
        let px = vec![255u8; (w * h * 4) as usize];
        let (png, ctype) = encode(&px, w, h, [4, 2, 12, 6], true).unwrap();
        assert_eq!(ctype, "image/png");
        let back = image::load_from_memory(&png).unwrap();
        assert_eq!((back.width(), back.height()), (8, 4));
        let (jpg, ctype) = encode(&px, w, h, [0, 0, 16, 8], false).unwrap();
        assert_eq!((ctype, &jpg[..2]), ("image/jpeg", &[0xff, 0xd8][..]));
        assert!(encode(&px, w, h, [12, 2, 4, 6], true).is_none(), "an empty part");
        assert!(encode(&px[..10], w, h, [0, 0, 16, 8], true).is_none(), "too few pixels");
    }

    #[test]
    fn the_page_asks_only_for_texts_the_table_has_in_six_languages() {
        let keys = locale_keys(LOCALE);
        // every literal text of the page (`t('...')`, `t("...")`) is a key
        let js = http::ASSETS[1].2;
        for quote in ['\'', '"'] {
            let open = format!("t({quote}");
            let mut rest = js;
            while let Some(at) = rest.find(&open) {
                let after = &rest[at + open.len()..];
                let end = after.find(quote).unwrap();
                let text = after[..end].replace("\\'", "'");
                // (not `t(` at the end of another name: `split(`, `setTimeout(`, `.at(`)
                let before = rest[..at].chars().last().unwrap_or(' ');
                if !(before.is_alphanumeric() || matches!(before, '_' | '.' | '$')) {
                    assert!(keys.contains(&text) || ALSO.contains(&text.as_str()), "the page asks for '{text}', which the table does not have");
                    // (and what it borrows from the game's table is there, in six languages)
                    if !keys.contains(&text) {
                        for lang in ["nl", "de", "fr", "ru", "uk", "pl"] {
                            assert!(crate::_rust_i18n_try_translate(lang, &text).is_some(), "'{text}' has no {lang} in the game's table");
                        }
                    }
                }
                rest = &after[end..];
            }
        }
        // every key has the six languages
        let mut current: Option<(String, Vec<String>)> = None;
        let mut blocks = Vec::new();
        for line in LOCALE.lines() {
            if line.starts_with('"') {
                blocks.extend(current.take());
                current = Some((line.to_string(), Vec::new()));
            } else if let (Some(c), Some(lang)) = (current.as_mut(), line.strip_prefix("  ").and_then(|l| l.split_once(':')).map(|l| l.0)) {
                c.1.push(lang.to_string());
            }
        }
        blocks.extend(current);
        assert!(blocks.len() > 40);
        for (key, langs) in blocks {
            for lang in ["nl", "de", "fr", "ru", "uk", "pl"] {
                assert!(langs.iter().any(|l| l == lang), "{key} has no {lang}");
            }
        }
    }

    #[test]
    fn every_text_of_the_table_is_a_key_and_devices_get_short_names() {
        let keys = locale_keys(LOCALE);
        assert!(keys.iter().any(|k| k == "Sign on"));
        assert!(!keys.iter().any(|k| k.starts_with('_')));
        assert_eq!(locale_keys("_version: 2\n\"A \\\"b\\\"\":\n  nl: \"x\"\n"), vec!["A \"b\"".to_string()]);
        assert_eq!(device_name("Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) AppleWebKit/605.1.15 Version/17.0 Mobile/15E148 Safari/604.1"), "Safari on iPad");
        assert_eq!(device_name("Mozilla/5.0 (Linux; Android 14) Chrome/126.0 Mobile Safari/537.36"), "Chrome on Android");
        assert_eq!(device_name(""), "Browser");
    }
}
