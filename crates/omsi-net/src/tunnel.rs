//! A free Cloudflare quick tunnel in front of the WebSocket gateway (`ws`): `cloudflared
//! tunnel --url http://127.0.0.1:<port>` gives an `https://<words>.trycloudflare.com` address
//! that reaches this machine from anywhere, through any router - no account, no port
//! forwarding, no VPN. The session code still says where the host is on the LAN and the
//! internet; the tunnel is the way in when neither answers.
//!
//! `cloudflared` is looked for next to the game, in `OMSI_CLOUDFLARED`, on the `PATH`, in
//! Homebrew's folders and in the game's own data folder; when it is nowhere, the game fetches
//! Cloudflare's own release build into its data folder the first time it hosts
//! (`ensure_cloudflared`) - the player installs nothing.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Where `cloudflared` is, if anywhere.
pub fn find_cloudflared() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "cloudflared.exe" } else { "cloudflared" };
    if let Some(p) = std::env::var_os("OMSI_CLOUDFLARED").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    // (`OMSI_CLOUDFLARED_OWN=1`: only the game's own copy, for testing the download)
    let own = own_dir().filter(|d| d.join(VERIFIED).is_file()).map(|d| d.join(exe)).filter(|p| p.is_file());
    if std::env::var_os("OMSI_CLOUDFLARED_OWN").is_some() {
        return own;
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        dirs.push(d.clone());
        dirs.push(d.join("tools"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].iter().map(PathBuf::from));
    // (one the player put in the game's tools folder, `~/.openomsi/tools`)
    dirs.extend(own_dir().and_then(|d| d.parent().map(|p| p.join("tools"))));
    dirs.into_iter().map(|d| d.join(exe)).find(|p| p.is_file()).or(own)
}

/// The arguments that start a quick tunnel to `http://127.0.0.1:<port>` (no updating
/// itself while the game runs).
pub fn quick_tunnel_args(port: u16) -> Vec<String> {
    vec!["tunnel".into(), "--no-autoupdate".into(), "--url".into(), format!("http://127.0.0.1:{port}")]
}

/// The game's own folder for the tools it fetches (`~/.openomsi/bin`).
fn own_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".openomsi").join("bin"))
}

/// The cloudflared release the game fetches: a fixed version whose files' SHA-256 (as
/// GitHub lists them for the release) are checked before anything is run.
const RELEASE: &str = "2026.9.3";

/// Cloudflare's release file of cloudflared for this machine (a `.tgz` on macOS) and its
/// SHA-256.
fn release_asset() -> Option<(&'static str, &'static str)> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => ("cloudflared-darwin-arm64.tgz", "587c2cfb1c230fe36c7fa7727da78be459dae028cabe8c001291999350f07095"),
        ("macos", "x86_64") => ("cloudflared-darwin-amd64.tgz", "d1155d0837487f261183b15c1eab6c4ebcad9dc49b94675f1524c3564cea3977"),
        ("linux", "x86_64") => ("cloudflared-linux-amd64", "77e26d8d900e0b8469f416239d14b5f296525fdf79fee6f511ef55609e3fbac2"),
        ("linux", "aarch64") => ("cloudflared-linux-arm64", "aaeb2d7d0da3614634c7e03ab13487a1522c2e79165ed2929cfe23d5e95b326d"),
        ("linux", "arm") => ("cloudflared-linux-arm", "967dc371a3fedbf09e881c13ee7ba317155ebc336cbd4afb756b46fc6785e5af"),
        // (Windows on ARM runs the x64 build)
        ("windows", "x86_64") | ("windows", "aarch64") => ("cloudflared-windows-amd64.exe", "f096265ec2fcbe9bb6e2d64268db167ced3fcbb83d894bdb9e2fcdb26f2ea7e2"),
        ("windows", "x86") => ("cloudflared-windows-386.exe", "9b95ddc2eba67b86ed3dc4cc2a15881960563031b52ce564376af41fb91ad402"),
        _ => return None,
    })
}

/// The marker written beside the game's own copy once its download was checked (a copy
/// without it - fetched by an older game unchecked - is fetched again).
const VERIFIED: &str = "cloudflared.verified";

/// cloudflared, fetched from Cloudflare's releases into the game's data folder when it is
/// not installed (once; later runs find it there). Blocks while it downloads (~20-40 MB):
/// call it off the game's main thread.
pub fn ensure_cloudflared() -> Option<PathBuf> {
    if let Some(p) = find_cloudflared() {
        return Some(p);
    }
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().ok()?;
    if let Some(p) = find_cloudflared() {
        return Some(p);
    }
    let (asset, sha) = release_asset()?;
    let dir = own_dir()?;
    let exe = dir.join(if cfg!(windows) { "cloudflared.exe" } else { "cloudflared" });
    let url = format!("https://github.com/cloudflare/cloudflared/releases/download/{RELEASE}/{asset}");
    log::info!("tunnel: cloudflared is not installed; fetching {url}");
    let t0 = Instant::now();
    let resp = ureq::get(&url).timeout(Duration::from_secs(300)).call().map_err(|e| log::warn!("tunnel: cloudflared could not be fetched: {e}")).ok()?;
    let mut data = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(resp.into_reader(), 200 << 20), &mut data).map_err(|e| log::warn!("tunnel: cloudflared download broke off: {e}")).ok()?;
    {
        use sha2::{Digest, Sha256};
        let got: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
        if got != sha {
            log::warn!("tunnel: {asset} does not match its published SHA-256 ({got}); not installed");
            return None;
        }
    }
    let bin = if asset.ends_with(".tgz") {
        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&data[..]), &mut raw).map_err(|e| log::warn!("tunnel: {asset}: {e}")).ok()?;
        tar_entry(&raw, "cloudflared")?
    } else {
        data
    };
    if bin.len() < 1 << 20 {
        log::warn!("tunnel: {asset} is too small ({} bytes) to be cloudflared", bin.len());
        return None;
    }
    std::fs::create_dir_all(&dir).ok()?;
    let tmp = exe.with_extension("part");
    std::fs::write(&tmp, &bin).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    std::fs::rename(&tmp, &exe).ok()?;
    std::fs::write(dir.join(VERIFIED), format!("{RELEASE} {asset} {sha}\n")).ok()?;
    log::info!("tunnel: cloudflared installed at {} ({:.1} MB in {:.1} s)", exe.display(), bin.len() as f64 / 1e6, t0.elapsed().as_secs_f32());
    Some(exe)
}

/// The file `name` (by its last path component) out of a tar archive.
fn tar_entry(tar: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut at = 0;
    while at + 512 <= tar.len() {
        let h = &tar[at..at + 512];
        if h.iter().all(|b| *b == 0) {
            return None;
        }
        let path = String::from_utf8_lossy(&h[..100]).trim_end_matches('\0').to_string();
        let size = usize::from_str_radix(String::from_utf8_lossy(&h[124..136]).trim_matches(|c: char| c == '\0' || c == ' '), 8).ok()?;
        let body = at + 512;
        if h[156] == b'0' || h[156] == 0 {
            if path.rsplit('/').next() == Some(name) {
                return tar.get(body..body + size).map(|b| b.to_vec());
            }
        }
        at = body + size.div_ceil(512) * 512;
    }
    None
}

/// A running quick tunnel; the process goes with it.
pub struct Tunnel {
    child: Child,
    /// The public address, once cloudflared has said it.
    pub url: Arc<Mutex<Option<String>>>,
    /// Where its process id is kept.
    pid_name: &'static str,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(p) = pid_file(self.pid_name) {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// The multiplayer gateway's tunnel's process id file.
const GATEWAY_PID: &str = "cloudflared.pid";

/// Where a running tunnel's process id is kept (`~/.openomsi/<name>`).
fn pid_file(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(|h| std::path::PathBuf::from(h).join(".openomsi").join(name))
}

/// A cloudflared an earlier game left running (it was killed, or crashed before it could
/// stop its tunnel): stopped before a new one starts.
fn kill_stale(pid_name: &str) {
    let Some(p) = pid_file(pid_name) else { return };
    let Some(pid) = std::fs::read_to_string(&p).ok().and_then(|s| s.trim().parse::<u32>().ok()) else { return };
    let _ = std::fs::remove_file(&p);
    #[cfg(unix)]
    {
        // only if that process is still a cloudflared (the id may have been reused)
        let name = Command::new("ps").args(["-p", &pid.to_string(), "-o", "comm="]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        if name.contains("cloudflared") {
            log::info!("tunnel: stopping the cloudflared an earlier game left running (pid {pid})");
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .status();
    }
}

impl Tunnel {
    /// Start a quick tunnel to `http://127.0.0.1:<port>` (cloudflared fetched first when it
    /// is not installed: this may take a while). None when there is no cloudflared.
    pub fn start(port: u16) -> Option<Tunnel> {
        let bin = ensure_cloudflared()?;
        Tunnel::start_with(&bin, port, GATEWAY_PID)
    }

    /// Start a quick tunnel to `http://127.0.0.1:<port>` with the cloudflared at `bin` (none
    /// is fetched), its process id kept in `~/.openomsi/<pid_name>` (a tunnel of its own for
    /// each user of it: one's leftover process is stopped, never another's running one).
    pub fn start_with(bin: &std::path::Path, port: u16, pid_name: &'static str) -> Option<Tunnel> {
        kill_stale(pid_name);
        let mut command = Command::new(bin);
        command
            .args(quick_tunnel_args(port))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = command
            .spawn()
            .map_err(|e| log::warn!("tunnel: {} could not be started: {e}", bin.display()))
            .ok()?;
        let url = Arc::new(Mutex::new(None));
        let stderr = child.stderr.take()?;
        let u = url.clone();
        std::thread::Builder::new()
            .name("tunnel log".into())
            .spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if let Some(found) = trycloudflare_url(&line) {
                        log::info!("tunnel: the session is reachable at {found}");
                        *u.lock().unwrap_or_else(|e| e.into_inner()) = Some(found);
                    } else if line.contains("ERR") {
                        log::debug!("tunnel: {line}");
                    }
                }
            })
            .ok()?;
        log::info!("tunnel: {} started for port {port}", bin.display());
        if let Some(p) = pid_file(pid_name) {
            let _ = std::fs::write(p, child.id().to_string());
        }
        Some(Tunnel { child, url, pid_name })
    }

    /// Whether cloudflared still runs (it ends when Cloudflare drops a quick tunnel, or the
    /// network went away for long).
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Wait up to `wait` for the address.
    pub fn wait_url(&self, wait: Duration) -> Option<String> {
        let t0 = Instant::now();
        while t0.elapsed() < wait {
            if let Some(u) = self.url.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                return Some(u);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }
}

/// The `https://….trycloudflare.com` address in a line of cloudflared's log.
pub fn trycloudflare_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let rest = &line[start..];
    let end = rest.find(|c: char| c.is_whitespace() || c == '|' || c == '"').unwrap_or(rest.len());
    let u = &rest[..end];
    (u.ends_with(".trycloudflare.com") && !u.contains("api.trycloudflare.com")).then(|| u.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn url_in_log() {
        let l = "2026-09-26T10:00:00Z INF |  https://quiet-river-sample-words.trycloudflare.com                                   |";
        assert_eq!(super::trycloudflare_url(l).as_deref(), Some("https://quiet-river-sample-words.trycloudflare.com"));
        assert_eq!(super::trycloudflare_url("https://api.trycloudflare.com/tunnel"), None);
        assert_eq!(super::trycloudflare_url("2026-10-04T10:00:00Z INF Requesting new quick Tunnel on trycloudflare.com..."), None);
        assert_eq!(super::trycloudflare_url("INF +----+ https://example.com/x |"), None);
    }

    #[test]
    fn a_quick_tunnel_goes_to_the_local_port_and_does_not_update_itself() {
        assert_eq!(super::quick_tunnel_args(47811), vec!["tunnel", "--no-autoupdate", "--url", "http://127.0.0.1:47811"]);
    }
}
