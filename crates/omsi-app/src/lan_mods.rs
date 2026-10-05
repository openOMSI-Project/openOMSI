//! The host's mods for the players who join: whatever of the host's session is not stock
//! OMSI 2 content - its map, the bus it drives, the objects, splines, AI vehicles and people
//! the map uses, and the mod fonts - goes to every player who joins, for the session only.
//!
//! The host serves the list of those files and the files themselves over TCP, on the same
//! port number as its LAN session (UDP). It serves nothing but the files of that list, by
//! their number in it: no path a client names is ever opened. A joining game asks for the
//! list before its world is loaded, fetches what it does not have in the same version
//! (compared by SHA-256), checks every file against the list's size and hash, and keeps
//! them in a folder of its own for this session (`~/.openomsi/lan-mods/<pid>`), which
//! becomes the first content root. The folder goes when the session ends (and one left by a
//! game that did not end cleanly goes at the next start).
//!
//! What may come: files under the content folders a map or a vehicle lives in (maps,
//! Vehicles, Sceneryobjects, Splines, Humans, Fonts, TicketPacks, Money, Weather, Texture,
//! Sound, Sounds), with plain relative names, and nothing that runs code: executables,
//! libraries, scripts of the operating system and OMSI plugins are refused by name, and by
//! content (a Windows, Linux or macOS executable or a script with `#!`, whatever it is
//! called). OMSI's own vehicle scripts are data for the game's own interpreter, which
//! touches nothing outside the vehicle. No plugin is ever loaded from the session folder
//! (`omsi_cfg::mark_sandbox`).

use crate::Args;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAGIC: &str = "OMSIMODS/1";
/// The most one file may have (a map tile or a texture is a few MB; a sound a few tens).
const MAX_FILE: u64 = 1 << 30;
/// The most a whole session may bring.
const MAX_TOTAL: u64 = 40 << 30;
/// Room left free on the disk after the download.
const KEEP_FREE: u64 = 3 << 30;
/// The most files a list may name.
const MAX_FILES: usize = 400_000;

/// The top-level content folders a session may bring files into.
const FOLDERS: &[&str] = &["maps", "vehicles", "sceneryobjects", "splines", "humans", "fonts", "ticketpacks", "money", "weather", "texture", "sound", "sounds", "trains", "announcements"];

/// Names that run code somewhere (Windows, macOS, Linux, the JVM, Office macros, OMSI
/// plugins and their configuration).
const REFUSED: &[&str] = &[
    "exe", "dll", "com", "bat", "cmd", "scr", "ps1", "psm1", "psd1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "msi", "msp", "mst", "jar", "hta", "cpl", "sys", "drv", "ocx", "ax", "lnk", "url", "reg", "inf", "sh", "bash", "zsh", "csh", "ksh", "command", "tool", "app", "dylib", "so", "py", "pyc", "pyw", "pl", "rb", "php", "lua", "apk", "run", "pif", "gadget", "appx", "msix", "iso", "img", "dmg", "pkg", "vhd", "vhdx", "opl", "docm", "xlsm", "pptm", "dotm", "xlam", "scpt", "applescript", "workflow", "action", "zip", "rar", "7z", "cab", "tar", "gz",
];

/// One file of the list: path relative to a content root (`/`-separated, as the host
/// spells it), its size and SHA-256 (hex).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    /// Only in `LIST ALL`: the bus folder (`vehicles/<name>`, lower case) of a bus a
    /// dedicated server offers that this file is there for; none for the session's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vehicle: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    /// The host's map (`maps/<name>/global.cfg`).
    pub map: String,
    pub bus: String,
    pub entries: Vec<Entry>,
}

// -------------------------------------------------------------------------------------
// checks (both sides)

/// Is `path` something a session may bring? Returns the reason when not.
pub fn refuse_path(path: &str) -> Option<String> {
    if path.is_empty() || path.len() > 400 {
        return Some("empty or too long".into());
    }
    if path.starts_with('/') || path.starts_with('\\') || path.contains(':') || path.contains('\\') {
        return Some("not a plain relative name".into());
    }
    let comps: Vec<&str> = path.split('/').collect();
    if comps.len() < 2 {
        return Some("not inside a content folder".into());
    }
    for c in &comps {
        if c.is_empty() || *c == "." || *c == ".." || c.chars().any(|ch| ch.is_control() || matches!(ch, '<' | '>' | '"' | '|' | '?' | '*')) || c.trim() != *c && c.trim().is_empty() {
            return Some(format!("bad name component {c:?}"));
        }
    }
    let top = comps[0].to_ascii_lowercase();
    if !FOLDERS.contains(&top.as_str()) {
        return Some(format!("{} is no content folder a session brings", comps[0]));
    }
    if comps.iter().any(|c| c.eq_ignore_ascii_case("plugins")) {
        return Some("plugins are never taken from another machine".into());
    }
    let name = comps.last().unwrap().to_ascii_lowercase();
    if let Some((_, ext)) = name.rsplit_once('.') {
        if REFUSED.contains(&ext) {
            return Some(format!(".{ext} files are never taken from another machine"));
        }
    }
    None
}

/// What a file manager leaves in a folder (thumbnails, folder settings, a Mac's resource
/// forks): nothing the game reads, and nothing a mod install keeps.
fn is_litter(rel: &str) -> bool {
    rel.split('/').any(|p| p == "__MACOSX" || p == ".DS_Store" || p.eq_ignore_ascii_case("thumbs.db") || p.eq_ignore_ascii_case("desktop.ini") || p.starts_with("._"))
}

/// Does the start of a file show a program (whatever its name says)?
pub fn looks_executable(head: &[u8]) -> bool {
    head.starts_with(b"MZ")
        || head.starts_with(b"\x7fELF")
        || head.starts_with(b"#!")
        || head.starts_with(&[0xfe, 0xed, 0xfa, 0xce])
        || head.starts_with(&[0xfe, 0xed, 0xfa, 0xcf])
        || head.starts_with(&[0xce, 0xfa, 0xed, 0xfe])
        || head.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
        || head.starts_with(&[0xca, 0xfe, 0xba, 0xbe])
        || head.starts_with(b"PK\x03\x04")
}

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_of(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

// -------------------------------------------------------------------------------------
// the host

/// Is content root `r` the OMSI 2 installation itself (its files are taken to be there on
/// every machine)? A content root inside it is not: openOMSI unpacked
/// into the OMSI 2 folder keeps what it installs in `<OMSI 2>/openOMSI`, and those mods
/// were never passed on.
fn is_original(r: &Path, original: &Path) -> bool {
    r == original
}

/// A path relative to a content root, `/`-separated, with `.` and `..` worked out.
fn norm(p: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for c in p.split(['/', '\\']) {
        match c.trim() {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c.to_string()),
        }
    }
    parts.join("/")
}

/// The folder an object, spline, vehicle or person file lives in: a vehicle is its whole
/// folder (Vehicles/<name>); an object, spline or person the folder it is in.
fn owner_folder(rel: &str) -> Option<String> {
    let comps: Vec<&str> = rel.split('/').collect();
    if comps.len() < 3 {
        return None;
    }
    let top = comps[0].to_ascii_lowercase();
    if top == "vehicles" || top == "trains" {
        return Some(comps[..2].join("/"));
    }
    Some(comps[..comps.len() - 1].join("/"))
}

/// The vehicle folder (`vehicles/<name>`, lower case) of a bus file, whatever comes before
/// it (`D:/OMSI 2/Vehicles/…`).
fn vehicle_folder(bus: &str) -> Option<String> {
    let n = norm(bus);
    let comps: Vec<&str> = n.split('/').collect();
    let i = comps.iter().rposition(|c| c.eq_ignore_ascii_case("vehicles"))?;
    (i + 2 < comps.len()).then(|| format!("vehicles/{}", comps[i + 1].to_lowercase()))
}

/// (lower-case relative path) -> (spelling, source)
type Found = HashMap<String, (String, PathBuf)>;

/// Every file of `folders` and of what their text files (and `texts`) name in turn: the
/// objects, splines, vehicles and people, with the folders those name - from the content
/// roots that are not the original installation. Also the fonts their text textures write
/// with.
fn gather(folders: Vec<String>, texts: Vec<(String, PathBuf)>, original: &Path) -> (Found, HashSet<String>) {
    let mut files: Found = HashMap::new();
    let mut folders_done: HashSet<String> = HashSet::new();
    let mut text_todo: Vec<(String, PathBuf)> = Vec::new();
    // a folder (relative) and everything in it, from the content roots that are not the
    // original installation (a mod's copy of a stock folder brings only what it adds)
    fn add_folder(rel: &str, original: &Path, files: &mut Found, text_todo: &mut Vec<(String, PathBuf)>, depth: usize) {
        if depth > 12 {
            return;
        }
        let roots = omsi_cfg::content_roots();
        let comps = omsi_cfg::windows_components(rel);
        let mut seen: HashSet<String> = HashSet::new();
        for r in roots {
            if is_original(&r, original) {
                continue;
            }
            let dir = comps.iter().fold(r.clone(), |p, c| p.join(c));
            let Some(list) = omsi_cfg::vfs::list_dir(&dir).or_else(|| {
                // (case-insensitively, as Windows would find it)
                omsi_cfg::find_in_roots(rel).filter(|(root, _)| *root == r).and_then(|(_, p)| omsi_cfg::vfs::list_dir(&p))
            }) else {
                continue;
            };
            for (name, is_dir) in list {
                let name = name.to_string_lossy().to_string();
                if !seen.insert(name.to_lowercase()) {
                    continue;
                }
                let child = format!("{rel}/{name}");
                if is_dir {
                    add_folder(&child, original, files, text_todo, depth + 1);
                } else {
                    let key = child.to_lowercase();
                    if files.contains_key(&key) {
                        continue;
                    }
                    if let Some((root, path)) = omsi_cfg::find_in_roots(&child) {
                        if is_original(&root, original) {
                            continue;
                        }
                        let lower = name.to_lowercase();
                        if [".cfg", ".sco", ".sli", ".bus", ".ovh", ".zug", ".hum", ".txt", ".hof"].iter().any(|e| lower.ends_with(e)) {
                            text_todo.push((child.clone(), path.clone()));
                        }
                        files.insert(key, (child, path));
                    }
                }
            }
        }
    }
    let mut want_folder = |rel: String, files: &mut Found, text_todo: &mut Vec<(String, PathBuf)>| {
        if folders_done.insert(rel.to_lowercase()) {
            add_folder(&rel, original, files, text_todo, 0);
        }
    };
    for f in folders {
        want_folder(f, &mut files, &mut text_todo);
    }
    let mut scanned: HashSet<String> = HashSet::new();
    let mut fonts_used: HashSet<String> = HashSet::new();
    let mut queue: Vec<(String, PathBuf)> = texts;
    queue.append(&mut text_todo);
    let mut rounds = 0;
    while !queue.is_empty() && rounds < 6 {
        rounds += 1;
        let mut next: Vec<(String, PathBuf)> = Vec::new();
        for (rel, path) in std::mem::take(&mut queue) {
            if !scanned.insert(rel.to_lowercase()) {
                continue;
            }
            let Ok(text) = omsi_cfg::vfs::read_text(&path) else { continue };
            let here = rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
            // the fonts its text textures write with (`[texttexture]`: variable, font, …)
            let lines: Vec<&str> = text.lines().collect();
            for (i, l) in lines.iter().enumerate() {
                if l.trim().to_ascii_lowercase().starts_with("[texttexture") {
                    if let Some(f) = lines.get(i + 2) {
                        fonts_used.insert(f.trim().to_lowercase());
                    }
                }
            }
            for line in text.lines() {
                let t = line.trim();
                if t.len() < 5 || t.len() > 300 || t.starts_with('[') {
                    continue;
                }
                let lower = t.to_ascii_lowercase();
                let is_ref = [".sco", ".sli", ".bus", ".ovh", ".zug", ".hum", ".owt"].iter().any(|e| lower.ends_with(e));
                let relative = t.contains("..");
                // a picture named from the root: a map's ground textures (`[groundtex]` of its
                // global.cfg: `Texture\<map>\gras.jpg`) - without them its ground was white
                let picture = [".bmp", ".jpg", ".jpeg", ".dds", ".tga", ".png"].iter().any(|e| lower.ends_with(e));
                if !is_ref && !relative && !picture {
                    continue;
                }
                // a path from the root ("Sceneryobjects\\...") or from this file's folder
                let candidates = [norm(t), norm(&format!("{here}/{t}"))];
                for c in candidates {
                    if c.split('/').count() < 2 {
                        continue;
                    }
                    let top = c.split('/').next().unwrap_or("").to_ascii_lowercase();
                    if !FOLDERS.contains(&top.as_str()) {
                        continue;
                    }
                    let Some((root, _)) = omsi_cfg::find_in_roots(&c) else { continue };
                    if is_original(&root, original) {
                        break;
                    }
                    let folder = if is_ref { owner_folder(&c) } else { c.rsplit_once('/').map(|(d, _)| d.to_string()) };
                    match folder.filter(|f| f.split('/').count() >= 2) {
                        Some(f) => want_folder(f, &mut files, &mut next),
                        // (one right in a content folder, `Texture\x.bmp`: that file alone)
                        None if picture => {
                            if let Some((_, path)) = omsi_cfg::find_in_roots(&c) {
                                files.entry(c.to_lowercase()).or_insert((c.clone(), path));
                            }
                        }
                        None => {}
                    }
                    break;
                }
            }
        }
        queue = next;
    }
    (files, fonts_used)
}

/// The mod fonts `fonts_used` names (OMSI reads the `.oft` files of `Fonts` itself, not its
/// sub-folders): each `.oft` with a `[newfont]` of a name in use, and its bitmaps.
fn add_fonts(fonts_used: &HashSet<String>, files: &mut Found, original: &Path) {
    for r in omsi_cfg::content_roots() {
        if is_original(&r, original) {
            continue;
        }
        let Some(dir) = omsi_cfg::find_in_roots("Fonts").filter(|(root, _)| *root == r).map(|(_, p)| p).or_else(|| Some(r.join("Fonts"))) else { continue };
        for (name, is_dir) in omsi_cfg::vfs::list_dir(&dir).unwrap_or_default() {
            let n = name.to_string_lossy().to_string();
            if is_dir || !n.to_lowercase().ends_with(".oft") {
                continue;
            }
            let Ok(text) = omsi_cfg::vfs::read_text(&dir.join(&n)) else { continue };
            let lines: Vec<&str> = text.lines().map(|l| l.trim()).collect();
            let mut wanted = false;
            let mut bitmaps: Vec<String> = Vec::new();
            for (i, l) in lines.iter().enumerate() {
                if l.eq_ignore_ascii_case("[newfont]") {
                    let name = lines.get(i + 1).map(|x| x.to_lowercase()).unwrap_or_default();
                    if fonts_used.contains(&name) {
                        wanted = true;
                        bitmaps.extend(lines.iter().skip(i + 2).take(2).map(|b| b.to_string()));
                    }
                }
            }
            if !wanted {
                continue;
            }
            for f in std::iter::once(n.clone()).chain(bitmaps) {
                let rel = format!("Fonts/{f}");
                if let Some((root, path)) = omsi_cfg::find_in_roots(&rel) {
                    if !is_original(&root, original) {
                        files.insert(rel.to_lowercase(), (rel, path));
                    }
                }
            }
        }
    }
}

/// The entries of `files` (size and hash read), in name order, with where each is read from.
fn entries_of(files: Found, vehicle: Option<&str>) -> (Vec<Entry>, Vec<PathBuf>, u64) {
    let mut list: Vec<(String, PathBuf)> = files.into_values().filter(|(rel, _)| refuse_path(rel).is_none() && !is_litter(rel)).collect();
    list.sort_by(|a, b| a.0.cmp(&b.0));
    let mut entries = Vec::with_capacity(list.len());
    let mut sources = Vec::with_capacity(list.len());
    let mut total = 0u64;
    for (rel, path) in list {
        let Ok(data) = omsi_cfg::vfs::read(&path) else { continue };
        if data.len() as u64 > MAX_FILE || looks_executable(&data[..data.len().min(8)]) {
            continue;
        }
        total += data.len() as u64;
        entries.push(Entry { path: rel, size: data.len() as u64, sha256: sha256_of(&data), vehicle: vehicle.map(str::to_string) });
        sources.push(path);
    }
    (entries, sources, total)
}

/// What the host serves: the session's list (`LIST`), the longer one with the buses a
/// dedicated server offers (`LIST ALL`: the same entries first, so a file has one number in
/// both), and where each file is read from.
struct Served {
    list: Vec<u8>,
    list_all: Vec<u8>,
    sources: Vec<PathBuf>,
}

/// The files the host's session uses that are not stock content, with where each is read
/// from (a folder or a mounted archive); then those of the buses a dedicated server offers
/// (`vehicles` of `server.cfg`) that the session has not named already.
fn collect(args: &Args) -> Served {
    let t0 = Instant::now();
    let original = args.root.clone();
    let mut folders: Vec<String> = Vec::new();
    let map_dir = norm(Path::new(&args.map).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default().as_str());
    if !map_dir.is_empty() {
        folders.push(map_dir.clone());
    }
    if let Some(f) = args.bus.as_deref().and_then(|b| owner_folder(&norm(b))) {
        folders.push(f);
    }
    // the map's tiles and lists, and every text file of what they bring
    let mut map_texts: Vec<(String, PathBuf)> = Vec::new();
    for r in omsi_cfg::content_roots() {
        let comps = omsi_cfg::windows_components(&map_dir);
        let dir = comps.iter().fold(r.clone(), |p, c| p.join(c));
        for (name, is_dir) in omsi_cfg::vfs::list_dir(&dir).unwrap_or_default() {
            let n = name.to_string_lossy().to_string();
            let l = n.to_lowercase();
            if !is_dir && (l.ends_with(".map") || l.ends_with(".cfg") || l.ends_with(".txt")) {
                map_texts.push((format!("{map_dir}/{n}"), dir.join(&n)));
            }
        }
    }
    let (mut files, fonts_used) = gather(folders, map_texts, &original);
    if let Some(w) = args.weather.as_deref() {
        let w = norm(w);
        if let Some((root, path)) = omsi_cfg::find_in_roots(&w) {
            if root != original {
                files.insert(w.to_lowercase(), (w, path));
            }
        }
    }
    add_fonts(&fonts_used, &mut files, &original);
    let (mut entries, mut sources, total) = entries_of(files, None);
    log::info!(
        "LAN mods: {} files ({:.1} MB) of this session are not stock content and go to joining players (listed in {:.1} s)",
        entries.len(),
        total as f64 / 1e6,
        t0.elapsed().as_secs_f64()
    );
    let manifest = |entries: &[Entry]| Manifest { map: args.map.replace('\\', "/"), bus: args.bus.clone().unwrap_or_default().replace('\\', "/"), entries: entries.to_vec() };
    let list = serde_json::to_vec(&manifest(&entries)).unwrap_or_default();
    // a dedicated server's buses, when its owner shares its content (`share_content`): a player
    // fetches the one it drives when it joins, the launcher all of them (with the map) before,
    // when the player asks it to
    let mut offered: Vec<String> = Vec::new();
    let vehicles = crate::server::SERVER_VEHICLES.get().filter(|_| crate::server::shares_content());
    for bus in vehicles.map(|v| v.as_slice()).unwrap_or_default() {
        if let Some(f) = owner_folder(&norm(bus)) {
            if !offered.iter().any(|o| o.eq_ignore_ascii_case(&f)) {
                offered.push(f);
            }
        }
    }
    if !offered.is_empty() {
        let t1 = Instant::now();
        let mut taken: HashSet<String> = entries.iter().map(|e| e.path.to_lowercase()).collect();
        let (mut n, mut bytes) = (0usize, 0u64);
        for folder in &offered {
            let (mut found, fonts_used) = gather(vec![folder.clone()], Vec::new(), &original);
            add_fonts(&fonts_used, &mut found, &original);
            found.retain(|k, _| !taken.contains(k));
            let (more, more_sources, size) = entries_of(found, Some(&folder.to_lowercase()));
            taken.extend(more.iter().map(|e| e.path.to_lowercase()));
            n += more.len();
            bytes += size;
            entries.extend(more);
            sources.extend(more_sources);
        }
        log::info!("LAN mods: the {} buses offered bring {n} files more ({:.1} MB; listed in {:.1} s)", offered.len(), bytes as f64 / 1e6, t1.elapsed().as_secs_f64());
    }
    let list_all = serde_json::to_vec(&manifest(&entries)).unwrap_or_default();
    Served { list, list_all, sources }
}

/// Serve the session's mods on TCP `port` (a thread; the list is made in the background).
pub fn serve(port: u16, session: u64, args: &Args) {
    if omsi_cfg::env::var_os("OMSI_NO_LAN_MODS").is_some() {
        return;
    }
    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(l) => l,
        Err(e) => {
            log::warn!("LAN mods: cannot serve on TCP port {port}: {e} (joining players need the host's mods installed)");
            return;
        }
    };
    let ready: Arc<Mutex<Option<Arc<Served>>>> = Arc::new(Mutex::new(None));
    {
        let ready = ready.clone();
        let args = args.clone();
        std::thread::Builder::new()
            .name("lan-mods-list".into())
            .spawn(move || {
                let m = collect(&args);
                *ready.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(m));
            })
            .ok();
    }
    log::info!("LAN mods: serving this session's mods on TCP port {port}");
    std::thread::Builder::new()
        .name("lan-mods".into())
        .spawn(move || {
            // (one thread per connection, so only so many at once)
            let open = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            for conn in listener.incoming() {
                let Ok(stream) = conn else { continue };
                if open.load(std::sync::atomic::Ordering::Relaxed) >= MAX_CONNECTIONS {
                    log::info!("LAN mods: {MAX_CONNECTIONS} connections open already; one more closed");
                    continue;
                }
                open.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let (ready, held) = (ready.clone(), open.clone());
                let spawned = std::thread::Builder::new().name("lan-mods-conn".into()).spawn(move || {
                    let peer = stream.peer_addr().ok();
                    if let Err(e) = handle(stream, session, &ready) {
                        log::info!("LAN mods: connection from {peer:?} ended: {e}");
                    }
                    held.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                });
                if spawned.is_err() {
                    open.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        })
        .ok();
}

/// Connections the mods server serves at once.
const MAX_CONNECTIONS: usize = 16;
/// The longest request line (they are a few words).
const MAX_LINE: u64 = 256;

/// One line of at most `MAX_LINE` bytes (a longer one ends the connection).
fn read_line_limited(input: &mut BufReader<TcpStream>, line: &mut String) -> std::io::Result<usize> {
    let n = input.by_ref().take(MAX_LINE).read_line(line)?;
    if n as u64 >= MAX_LINE && !line.ends_with('\n') {
        return Err(std::io::Error::other("request line too long"));
    }
    Ok(n)
}

fn handle(stream: TcpStream, session: u64, ready: &Mutex<Option<Arc<Served>>>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(120)))?;
    stream.set_nodelay(true).ok();
    let mut out = stream.try_clone()?;
    let mut input = BufReader::new(stream);
    let mut line = String::new();
    read_line_limited(&mut input, &mut line)?;
    let hello: Vec<&str> = line.split_whitespace().collect();
    if hello.len() != 2 || hello[0] != MAGIC || u64::from_str_radix(hello[1], 16).ok() != Some(session) {
        out.write_all(b"ERR not this session\n")?;
        return Ok(());
    }
    // the list may still be being made (hashing a big map takes a while)
    let t0 = Instant::now();
    let data = loop {
        if let Some(d) = ready.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            break d;
        }
        if t0.elapsed() > Duration::from_secs(600) {
            out.write_all(b"ERR the host has no list\n")?;
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let sources = &data.sources;
    writeln!(out, "OK")?;
    loop {
        line.clear();
        if read_line_limited(&mut input, &mut line)? == 0 {
            return Ok(());
        }
        let req: Vec<&str> = line.split_whitespace().collect();
        match req.as_slice() {
            ["LIST"] | ["LIST", "ALL"] => {
                let body = if req.len() == 2 { &data.list_all } else { &data.list };
                writeln!(out, "OK {}", body.len())?;
                out.write_all(body)?;
            }
            ["GET", idx] => {
                let Some(k) = idx.parse::<usize>().ok().filter(|k| *k < sources.len()) else {
                    writeln!(out, "ERR no such file")?;
                    continue;
                };
                // a plain file goes as it is read (never all of it in memory); one in a
                // mounted archive is read whole (archives hold small files)
                let src = &sources[k];
                if omsi_cfg::vfs::archive_of(src).is_none() {
                    match std::fs::File::open(src).and_then(|f| f.metadata().map(|m| (f, m.len()))) {
                        Ok((f, len)) => {
                            writeln!(out, "OK {len}")?;
                            let sent = std::io::copy(&mut f.take(len), &mut out)?;
                            if sent != len {
                                // (the file shrank meanwhile: the stream is out of step)
                                return Err(std::io::Error::other("a file changed while it was sent"));
                            }
                        }
                        Err(e) => writeln!(out, "ERR {e}")?,
                    }
                } else {
                    match omsi_cfg::vfs::read(src) {
                        Ok(bytes) => {
                            writeln!(out, "OK {}", bytes.len())?;
                            out.write_all(&bytes)?;
                        }
                        Err(e) => writeln!(out, "ERR {e}")?,
                    }
                }
            }
            ["BYE"] | [] => return Ok(()),
            _ => writeln!(out, "ERR what")?,
        }
    }
}

// -------------------------------------------------------------------------------------
// the joining player

/// The session folder of this game (`~/.openomsi/lan-mods/<pid>`).
fn sandbox_base() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".openomsi").join("lan-mods"))
}

static SANDBOX: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Remove the session folders of games that are no longer running (one that ended without
/// cleaning up).
pub fn remove_stale() {
    let Some(base) = sandbox_base() else { return };
    let Ok(rd) = std::fs::read_dir(&base) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let alive = name.parse::<u32>().map(process_alive).unwrap_or(false);
        if !alive {
            let _ = std::fs::remove_dir_all(e.path());
            log::info!("LAN mods: removed the content of an old session ({name})");
        }
    }
}

fn process_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
}

/// The session is over: its content goes.
pub fn clean_up() {
    let taken = SANDBOX.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(dir) = taken {
        omsi_cfg::remove_content_root(&dir);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => log::info!("LAN mods: the host's mods of this session were removed"),
            Err(e) => log::warn!("LAN mods: could not remove {}: {e} (it goes at the next start)", dir.display()),
        }
    }
}

#[cfg(unix)]
fn free_space(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

#[cfg(not(unix))]
fn free_space(_dir: &Path) -> Option<u64> {
    None
}

/// What the joining game found out: to say in the HUD/launcher.
#[derive(Debug, Default)]
pub struct Report {
    pub fetched: usize,
    pub bytes: u64,
    pub had: usize,
    /// (the launcher) Files placed from the store, fetched at an earlier join.
    pub reused: usize,
    pub refused: Vec<String>,
}

fn read_reply(input: &mut BufReader<TcpStream>) -> Result<Option<u64>, String> {
    let mut line = String::new();
    input.read_line(&mut line).map_err(|e| e.to_string())?;
    let t = line.trim();
    if let Some(rest) = t.strip_prefix("OK") {
        let rest = rest.trim();
        if rest.is_empty() {
            return Ok(None);
        }
        return rest.parse::<u64>().map(Some).map_err(|_| format!("bad reply {t:?}"));
    }
    Err(format!("the host says: {}", t.strip_prefix("ERR").unwrap_or(t).trim()))
}

/// Local files' hashes by path: (size, modification time, SHA-256).
type HashCache = std::collections::HashMap<String, (u64, u64, String)>;

fn hash_cache_path() -> Option<std::path::PathBuf> {
    sandbox_base().map(|b| b.with_file_name("lan-hash-cache.json"))
}

fn hash_cache() -> HashCache {
    hash_cache_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_hash_cache(c: &HashCache) {
    if let Some(p) = hash_cache_path() {
        let _ = std::fs::write(p, serde_json::to_vec(c).unwrap_or_default());
    }
}

/// A connection to the host's mods, greeted.
fn open(host: SocketAddr, session: u64) -> Result<(TcpStream, BufReader<TcpStream>), String> {
    let stream = TcpStream::connect_timeout(&host, Duration::from_secs(6)).map_err(|e| format!("cannot reach the host's mods on TCP {host}: {e}"))?;
    // the host greets once its list is made, which takes a while on a big map (it waits
    // up to ten minutes for it): after 25 s a join gave up on a big add-on map, and the
    // player was left without the host's map
    stream.set_read_timeout(Some(Duration::from_secs(600))).ok();
    stream.set_nodelay(true).ok();
    let mut out = stream.try_clone().map_err(|e| e.to_string())?;
    let mut input = BufReader::with_capacity(1 << 20, stream);
    writeln!(out, "{MAGIC} {}", omsi_net::session_hex(session)).map_err(|e| e.to_string())?;
    read_reply(&mut input)?;
    // (then a stalled transfer ends the attempt instead of holding the game's start for ever)
    input.get_ref().set_read_timeout(Some(Duration::from_secs(25))).ok();
    Ok((out, input))
}

/// The downloads kept between sessions, by their SHA-256 (`~/.openomsi/lan-store`): a
/// map fetched once is not fetched again at the next join.
fn store_dir() -> Option<PathBuf> {
    sandbox_base().map(|b| b.with_file_name("lan-store"))
}

/// Put a stored file at `target` (a hard link, a copy where links are not possible).
fn place(stored: &Path, target: &Path) -> std::io::Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(target);
    std::fs::hard_link(stored, target).or_else(|_| std::fs::copy(stored, target).map(|_| ()))
}

/// The host's list: with the buses a dedicated server offers (`LIST ALL`) when `all` and the
/// host knows it, else the session's own.
fn read_list(out: &mut TcpStream, input: &mut BufReader<TcpStream>, all: bool) -> Result<Manifest, String> {
    let mut len = None;
    if all {
        writeln!(out, "LIST ALL").map_err(|e| e.to_string())?;
        // (a host older than `LIST ALL` says it does not know it, and is asked the old way)
        len = read_reply(input).ok().flatten();
    }
    let len = match len {
        Some(l) => l,
        None => {
            writeln!(out, "LIST").map_err(|e| e.to_string())?;
            read_reply(input)?.ok_or("no list")?
        }
    };
    if len > 256 << 20 {
        return Err("the list is too large".into());
    }
    let mut body = vec![0u8; len as usize];
    input.read_exact(&mut body).map_err(|e| e.to_string())?;
    let manifest: Manifest = serde_json::from_slice(&body).map_err(|e| format!("bad list: {e}"))?;
    if manifest.entries.len() > MAX_FILES {
        return Err(format!("{} files are too many", manifest.entries.len()));
    }
    Ok(manifest)
}

/// The entries of the list that may be taken at all and that `wanted` keeps (the others are
/// noted in the report as refused).
fn acceptable(manifest: &Manifest, report: &mut Report, wanted: impl Fn(&Entry) -> bool) -> Vec<usize> {
    let mut todo: Vec<usize> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (k, e) in manifest.entries.iter().enumerate() {
        if !wanted(e) {
            continue;
        }
        if let Some(why) = refuse_path(&e.path) {
            report.refused.push(format!("{}: {why}", e.path));
            continue;
        }
        // (the hash names the file in the store: nothing but 64 hex digits may go there)
        if e.size > MAX_FILE || e.sha256.len() != 64 || !e.sha256.bytes().all(|b| b.is_ascii_hexdigit()) || !seen.insert(e.path.to_lowercase()) {
            report.refused.push(format!("{}: refused", e.path));
            continue;
        }
        todo.push(k);
    }
    todo
}

/// Fetch the entries `todo` (`total` bytes) of `manifest` into `dir` (by their paths in it),
/// through the store when `via_store` (a game's join: the next join finds them there), else
/// straight into `dir` (the launcher installs them: a copy in the store too was the whole
/// download twice, on another disk as often as not). Every request at once (one after the other took a round trip per
/// file: through a tunnel a few hundred kB/s), the answers read as they come; a broken
/// connection is opened again and goes on with what is left (what came is in the store).
#[allow(clippy::too_many_arguments)]
fn transfer(
    conn: (TcpStream, BufReader<TcpStream>),
    host: SocketAddr,
    session: u64,
    manifest: &Manifest,
    todo: &[usize],
    total: u64,
    dir: &Path,
    report: &mut Report,
    progress: &mut dyn FnMut(u64, u64, &str),
    cancel: &std::sync::atomic::AtomicBool,
    via_store: bool,
) -> Result<(), String> {
    let store = store_dir().ok_or("no home folder")?;
    std::fs::create_dir_all(&store).map_err(|e| e.to_string())?;
    let target_of = |e: &Entry| e.path.split('/').fold(dir.to_path_buf(), |p, c| p.join(c));
    let (mut out, mut input) = conn;
    let mut done = 0u64;
    let mut left: Vec<usize> = todo.to_vec();
    let mut attempt = 0;
    while !left.is_empty() {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(500 * attempt as u64));
            match open(host, session) {
                Ok(c) => (out, input) = c,
                Err(e) if attempt < 8 => {
                    log::warn!("LAN mods: {e}; trying again");
                    attempt += 1;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        attempt += 1;
        let mut req = String::with_capacity(left.len() * 10);
        for k in &left {
            req.push_str(&format!("GET {k}\n"));
        }
        req.push_str("BYE\n");
        let mut w = out.try_clone().map_err(|e| e.to_string())?;
        let writer = std::thread::spawn(move || {
            let _ = w.write_all(req.as_bytes());
        });
        let mut got = 0usize;
        let mut failed: Option<String> = None;
        for &k in &left {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                failed = Some("cancelled".into());
                break;
            }
            let e = &manifest.entries[k];
            let r = (|| -> Result<Vec<u8>, String> {
                let len = read_reply(&mut input)?.ok_or("no size")?;
                if len != e.size {
                    return Err(format!("{}: {len} bytes, the list said {}", e.path, e.size));
                }
                let mut data = vec![0u8; len as usize];
                input.read_exact(&mut data).map_err(|x| format!("{}: {x}", e.path))?;
                if sha256_of(&data) != e.sha256 {
                    return Err(format!("{}: not the file the list names (hash)", e.path));
                }
                Ok(data)
            })();
            let data = match r {
                Ok(d) => d,
                Err(x) => {
                    failed = Some(x);
                    break;
                }
            };
            got += 1;
            let len = data.len() as u64;
            if looks_executable(&data[..data.len().min(8)]) {
                report.refused.push(format!("{}: a program", e.path));
                continue;
            }
            let target = target_of(e);
            // (inside the folder, whatever the name: `refuse_path` has seen to that)
            if !target.starts_with(dir) {
                report.refused.push(format!("{}: outside", e.path));
                continue;
            }
            if via_store {
                let stored = store.join(&e.sha256);
                let tmp = store.join(format!("{}.part", e.sha256));
                std::fs::write(&tmp, &data).and_then(|_| std::fs::rename(&tmp, &stored)).map_err(|x| format!("{}: {x}", e.path))?;
                place(&stored, &target).map_err(|x| format!("{}: {x}", e.path))?;
            } else {
                let tmp = target.with_file_name(format!(".{}.part", e.sha256));
                target.parent().map(std::fs::create_dir_all).transpose().and_then(|_| std::fs::write(&tmp, &data)).and_then(|_| std::fs::rename(&tmp, &target)).map_err(|x| format!("{}: {x}", e.path))?;
            }
            done += len;
            report.fetched += 1;
            report.bytes += len;
            progress(done, total, &e.path);
        }
        let _ = out.shutdown(std::net::Shutdown::Both);
        let _ = writer.join();
        left.drain(..got);
        if let Some(x) = failed {
            if attempt >= 8 || x.contains("the host says") || x == "cancelled" {
                return Err(x);
            }
            log::warn!("LAN mods: {x}; {} files left, connecting again", left.len());
        }
    }
    Ok(())
}

/// Fetch the host's mods (a joining player, before its world is loaded): what this machine
/// does not have in the host's version goes into the session folder, which becomes the
/// first content root; the host's map becomes ours. Of the buses a dedicated server offers,
/// only the one this game drives comes. `progress` is told (done, total bytes).
pub fn fetch(args: &mut Args, host: SocketAddr, session: u64, progress: &mut dyn FnMut(u64, u64, &str)) -> Result<Report, String> {
    if omsi_cfg::env::var_os("OMSI_NO_LAN_MODS").is_some() {
        return Err("switched off (OMSI_NO_LAN_MODS)".into());
    }
    let (mut out, mut input) = open(host, session)?;
    let manifest = read_list(&mut out, &mut input, true)?;
    let mut report = Report::default();
    let mine = args.bus.as_deref().and_then(vehicle_folder);
    let mut todo = acceptable(&manifest, &mut report, |e| e.vehicle.is_none() || e.vehicle == mine);
    let mut total = 0u64;
    // which of them this machine has already: the sizes first, the hashes remembered by
    // size and time (hashing every local file again took two minutes on each join, long
    // enough for the session to give the player up), the rest in parallel
    let cache = hash_cache();
    let checks: Vec<(usize, bool, Option<(String, (u64, u64, String))>)> = {
        use rayon::prelude::*;
        todo.par_iter()
            .map(|&k| {
                let e = &manifest.entries[k];
                let Some((_, p)) = omsi_cfg::find_in_roots(&e.path).filter(|(root, _)| !omsi_cfg::is_sandbox(root)) else { return (k, false, None) };
                let key = p.to_string_lossy().to_string();
                if let Ok(md) = std::fs::metadata(&p) {
                    if md.len() != e.size {
                        return (k, false, None);
                    }
                    let mtime = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
                    if let Some((sz, mt, h)) = cache.get(&key) {
                        if *sz == md.len() && *mt == mtime {
                            return (k, *h == e.sha256, None);
                        }
                    }
                    let Ok(d) = std::fs::read(&p) else { return (k, false, None) };
                    let h = sha256_of(&d);
                    return (k, h == e.sha256, Some((key, (md.len(), mtime, h))));
                }
                let same = omsi_cfg::vfs::read(&p).ok().map(|d| d.len() as u64 == e.size && sha256_of(&d) == e.sha256).unwrap_or(false);
                (k, same, None)
            })
            .collect()
    };
    let mut cache = cache;
    let base = sandbox_base().ok_or("no home folder")?;
    let dir = base.join(std::process::id().to_string());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let store = store_dir().ok_or("no home folder")?;
    std::fs::create_dir_all(&store).map_err(|e| e.to_string())?;
    let target_of = |e: &Entry| e.path.split('/').fold(dir.clone(), |p, c| p.join(c));
    todo.clear();
    for (k, same, fresh) in checks {
        if let Some((key, v)) = fresh {
            cache.insert(key, v);
        }
        let e = &manifest.entries[k];
        if same {
            report.had += 1;
            continue;
        }
        // fetched at an earlier join
        let stored = store.join(&e.sha256);
        if std::fs::metadata(&stored).map(|m| m.len() == e.size).unwrap_or(false) && place(&stored, &target_of(e)).is_ok() {
            report.had += 1;
            continue;
        }
        todo.push(k);
        total += e.size;
    }
    save_hash_cache(&cache);
    if total > MAX_TOTAL {
        return Err(format!("the host's mods are {:.1} GB, more than a session takes ({:.0} GB)", total as f64 / 1e9, MAX_TOTAL as f64 / 1e9));
    }
    *SANDBOX.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir.clone());
    log::info!("LAN mods: {} files of the host's are here already, {} to fetch ({:.1} MB)", report.had, todo.len(), total as f64 / 1e6);
    if !todo.is_empty() {
        let some: Vec<&str> = todo.iter().take(6).map(|k| manifest.entries[*k].path.as_str()).collect();
        log::info!("LAN mods: to fetch, e.g. {}", some.join(", "));
    }
    // (nothing to fetch needs no room: a nearly full disk turned away a join that had
    // everything already)
    if let Some(free) = free_space(&dir).filter(|_| total > 0) {
        if free < total + KEEP_FREE {
            return Err(format!("{:.1} GB are needed for the host's mods, {:.1} GB are free", (total + KEEP_FREE) as f64 / 1e9, free as f64 / 1e9));
        }
    }
    transfer((out, input), host, session, &manifest, &todo, total, &dir, &mut report, progress, &std::sync::atomic::AtomicBool::new(false), true)?;
    // the session folder is content like any other, searched first, and never a source of
    // plugins
    omsi_cfg::mark_sandbox(dir.clone());
    omsi_cfg::add_content_root_first(dir.clone());
    // the host's map (now that we have it)
    let map_ok = refuse_path(&manifest.map).is_none() && omsi_cfg::find_in_roots(&manifest.map).is_some();
    if map_ok && !manifest.map.is_empty() && !manifest.map.eq_ignore_ascii_case(&args.map.replace('\\', "/")) {
        log::info!("LAN mods: the session is on the host's map {}", manifest.map);
        args.map = manifest.map.clone();
    }
    if !report.refused.is_empty() {
        log::warn!("LAN mods: {} files refused: {}", report.refused.len(), report.refused.iter().take(8).cloned().collect::<Vec<_>>().join("; "));
    }
    log::info!("LAN mods: fetched {} files ({:.1} MB), {} were here already", report.fetched, report.bytes as f64 / 1e6, report.had);
    Ok(report)
}

/// What of a host's content this machine lacks (the launcher, before joining).
#[derive(Debug, Default, Clone)]
pub struct Lacking {
    /// Files it has nothing of.
    pub missing: usize,
    /// Files put here from that host before that the host has in another version now.
    pub outdated: usize,
    /// Their size.
    pub bytes: u64,
    /// The host's map is among the missing.
    pub map_missing: bool,
}

/// The entries of the list this machine lacks: those it has nothing of, and those `installed`
/// (lower-case path -> SHA-256 of the files put here from that host before) has in another
/// version (and not those of `skipped`). A file it has from elsewhere stays as it is (the game fetches the host's version
/// for the session when it joins), so the other maps keep theirs.
fn lacking(manifest: &Manifest, report: &mut Report, installed: &HashMap<String, String>, skipped: &HashSet<String>) -> (Vec<usize>, Lacking) {
    let mut todo = Vec::new();
    let mut l = Lacking::default();
    // (what a mod install leaves out never comes: those of an older host, and `skipped`, the
    // files an install of this host's content left out before)
    for k in acceptable(manifest, report, |e| !is_litter(&e.path) && !skipped.contains(&e.path.to_lowercase())) {
        let e = &manifest.entries[k];
        let here = omsi_cfg::find_in_roots(&e.path).is_some();
        match installed.get(&e.path.to_lowercase()) {
            Some(sha) if here && *sha != e.sha256 => l.outdated += 1,
            _ if here => {
                report.had += 1;
                continue;
            }
            _ => {
                l.missing += 1;
                if e.path.eq_ignore_ascii_case(&manifest.map) {
                    l.map_missing = true;
                }
            }
        }
        l.bytes += e.size;
        todo.push(k);
    }
    (todo, l)
}

/// Ask the host what of its content this machine lacks (see `lacking`), without fetching.
pub fn check_missing(host: SocketAddr, session: u64, installed: &HashMap<String, String>, skipped: &HashSet<String>) -> Result<Lacking, String> {
    let (mut out, mut input) = open(host, session)?;
    let manifest = read_list(&mut out, &mut input, true)?;
    let _ = writeln!(out, "BYE");
    Ok(lacking(&manifest, &mut Report::default(), installed, skipped).1)
}

/// The launcher, before joining (the player asked for it, or it updates what it fetched
/// before): what of the host's mods - its map, what the map uses, the buses a dedicated
/// server offers - this machine lacks (see `lacking`) goes into `dir`, laid out like a
/// content folder, to be installed as a mod. Files fetched at an earlier join come from the
/// store. `progress` is told (done, total bytes, file). Returns, besides the report, every
/// file put into `dir` (path, SHA-256).
pub fn fetch_missing(
    host: SocketAddr,
    session: u64,
    dir: &Path,
    installed: &HashMap<String, String>,
    skipped: &HashSet<String>,
    progress: &mut dyn FnMut(u64, u64, &str),
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(Report, Vec<(String, String)>), String> {
    if omsi_cfg::env::var_os("OMSI_NO_LAN_MODS").is_some() {
        return Err("switched off (OMSI_NO_LAN_MODS)".into());
    }
    let (mut out, mut input) = open(host, session)?;
    let manifest = read_list(&mut out, &mut input, true)?;
    let mut report = Report::default();
    let (wanted, _) = lacking(&manifest, &mut report, installed, skipped);
    let store = store_dir().ok_or("no home folder")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut todo: Vec<usize> = Vec::new();
    let mut total = 0u64;
    for k in wanted.iter().copied() {
        let e = &manifest.entries[k];
        let stored = store.join(&e.sha256);
        let target = e.path.split('/').fold(dir.to_path_buf(), |p, c| p.join(c));
        // (fetched by an attempt that did not get to the install: the folder is kept then)
        if std::fs::read(&target).map(|d| d.len() as u64 == e.size && sha256_of(&d) == e.sha256).unwrap_or(false) {
            report.reused += 1;
            continue;
        }
        if std::fs::metadata(&stored).map(|m| m.len() == e.size).unwrap_or(false) && place(&stored, &target).is_ok() {
            report.reused += 1;
            continue;
        }
        todo.push(k);
        total += e.size;
    }
    log::info!("LAN mods: {} of the host's files are installed here, {} came from earlier joins, {} to fetch ({:.1} MB)", report.had, report.reused, todo.len(), total as f64 / 1e6);
    if let Some(free) = free_space(dir).filter(|_| total > 0) {
        if free < total + KEEP_FREE {
            return Err(format!("{:.1} GB are needed for the host's mods, {:.1} GB are free", (total + KEEP_FREE) as f64 / 1e9, free as f64 / 1e9));
        }
    }
    transfer((out, input), host, session, &manifest, &todo, total, dir, &mut report, progress, cancel, false)?;
    if !report.refused.is_empty() {
        log::warn!("LAN mods: {} files refused: {}", report.refused.len(), report.refused.iter().take(8).cloned().collect::<Vec<_>>().join("; "));
    }
    // (what is in the folder now: the refused are not)
    let placed = wanted
        .iter()
        .map(|k| &manifest.entries[*k])
        .filter(|e| e.path.split('/').fold(dir.to_path_buf(), |p, c| p.join(c)).is_file())
        .map(|e| (e.path.clone(), e.sha256.clone()))
        .collect();
    Ok((report, placed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_that_are_refused() {
        assert!(refuse_path("maps/Ahlheim/global.cfg").is_none());
        assert!(refuse_path("Vehicles/LiAZ/Model/model.cfg").is_none());
        assert!(refuse_path("Sceneryobjects/X/texture/a.dds").is_none());
        for bad in [
            "../etc/passwd",
            "maps/../../x.cfg",
            "/abs/maps/x",
            "C:/Windows/x.cfg",
            "maps\\x\\y.cfg",
            "Plugins/evil.dll",
            "Vehicles/X/Plugins/a.cfg",
            "Vehicles/X/setup.exe",
            "maps/x/run.BAT",
            "maps/x/a.sh",
            "maps/x/lib.dylib",
            "maps/x/pack.zip",
            "Startup/x.cfg",
            "x.cfg",
            "maps//x.cfg",
            "maps/x/y?.cfg",
        ] {
            assert!(refuse_path(bad).is_some(), "{bad} should be refused");
        }
    }

    #[test]
    fn the_folder_of_a_bus() {
        assert_eq!(vehicle_folder("Vehicles/MAZ-103/MAZ-103_K.bus").as_deref(), Some("vehicles/maz-103"));
        assert_eq!(vehicle_folder("vehicles\\Ruta_25_ПЕ\\Ruta.bus").as_deref(), Some("vehicles/ruta_25_пе"));
        assert_eq!(vehicle_folder("D:/OMSI 2/Vehicles/LAZ_5207/LAZ_5207.bus").as_deref(), Some("vehicles/laz_5207"));
        assert_eq!(vehicle_folder("Vehicles/x.bus"), None);
        assert_eq!(vehicle_folder(""), None);
        assert_eq!(owner_folder("Vehicles/LAZ_5207/Model/model.cfg").as_deref(), Some("Vehicles/LAZ_5207"));
        assert_eq!(owner_folder("Sceneryobjects/X/a.sco").as_deref(), Some("Sceneryobjects/X"));
    }

    fn served_on_a_port(session: u64, served: Served) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let ready = Arc::new(Mutex::new(Some(Arc::new(served))));
        std::thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                let ready = ready.clone();
                std::thread::spawn(move || handle(conn, session, &ready));
            }
        });
        addr
    }

    fn manifest_json(paths: &[(&str, Option<&str>)]) -> Vec<u8> {
        let entries = paths.iter().map(|(p, v)| Entry { path: p.to_string(), size: 1, sha256: "0".repeat(64), vehicle: v.map(str::to_string) }).collect();
        serde_json::to_vec(&Manifest { map: "maps/Chuguev/global.cfg".into(), bus: String::new(), entries }).unwrap()
    }

    #[test]
    fn the_offered_buses_come_with_list_all_only() {
        let session = 0x1234_5678_9abc;
        let list = manifest_json(&[("maps/Chuguev/global.cfg", None)]);
        let list_all = manifest_json(&[("maps/Chuguev/global.cfg", None), ("Vehicles/LAZ_5207/LAZ_5207.bus", Some("vehicles/laz_5207"))]);
        let host = served_on_a_port(session, Served { list, list_all, sources: Vec::new() });
        let (mut out, mut input) = open(host, session).unwrap();
        let all = read_list(&mut out, &mut input, true).unwrap();
        assert_eq!(all.entries.len(), 2);
        assert_eq!(all.entries[1].vehicle.as_deref(), Some("vehicles/laz_5207"));
        // (the old request, as games before this one send it: the session's own files only,
        // without the new key)
        let own = read_list(&mut out, &mut input, false).unwrap();
        assert_eq!(own.entries.len(), 1);
        assert!(own.entries[0].vehicle.is_none());
        // another session's id is not served
        assert!(open(host, session + 1).is_err());
    }

    #[test]
    fn an_older_host_is_asked_for_its_list_the_old_way() {
        // a host before `LIST ALL`: it greets, says it does not know the request and lists
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = listener.local_addr().unwrap();
        let body = manifest_json(&[("maps/Grundorf2/global.cfg", None)]);
        std::thread::spawn(move || {
            let (conn, _) = listener.accept().unwrap();
            let mut out = conn.try_clone().unwrap();
            let mut input = BufReader::new(conn);
            let mut line = String::new();
            input.read_line(&mut line).unwrap();
            out.write_all(b"OK\n").unwrap();
            loop {
                line.clear();
                if input.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                if line.trim() == "LIST" {
                    out.write_all(format!("OK {}\n", body.len()).as_bytes()).unwrap();
                    out.write_all(&body).unwrap();
                } else {
                    out.write_all(b"ERR what\n").unwrap();
                }
            }
        });
        let (mut out, mut input) = open(host, 7).unwrap();
        let m = read_list(&mut out, &mut input, true).unwrap();
        assert_eq!(m.entries.len(), 1);
        assert_eq!(m.entries[0].path, "maps/Grundorf2/global.cfg");
    }

    #[test]
    fn programs_are_seen_by_their_content() {
        assert!(looks_executable(b"MZ\x90\x00"));
        assert!(looks_executable(b"\x7fELF\x02"));
        assert!(looks_executable(b"#!/bin/sh"));
        assert!(looks_executable(&[0xcf, 0xfa, 0xed, 0xfe]));
        assert!(!looks_executable(b"DDS "));
        assert!(!looks_executable(b"[mesh]"));
    }
}
