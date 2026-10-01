//! Updates from the project's GitHub releases (github.com/openOMSI-Project/openOMSI).
//!
//! Every push to main publishes a release `v<MAJOR.MINOR.COMMIT>` with one archive per
//! platform (see .github/workflows/release.yml). The launcher asks the GitHub API for the
//! latest release when it starts (setting `update_check`), and when it is newer than this
//! build it offers it - or, with `update_auto`, installs it at once:
//!
//! * **Windows, macOS, Linux**: the archive is downloaded (and checked against the SHA-256
//!   GitHub lists for it), unpacked into `.openomsi-update` beside the program, and every
//!   program file it holds takes the place of the old one: the old one is renamed to
//!   `*.old-update` first (Windows lets a running .exe be renamed, not overwritten) and put
//!   back if anything fails. On macOS the running `.app` bundle is the one replaced,
//!   whatever the user named it. Files an earlier update installed that the new archive no
//!   longer has go as well (`.openomsi-files` lists them); nothing else in the folder - the
//!   mods, the content - is touched. Then the new launcher is started and this one ends; the
//!   next start deletes the `*.old-update` files.
//! * **Android**: the APK is downloaded and handed to the system's package installer
//!   (`OmsiActivity.installApk`, a PackageInstaller session). The system asks the player;
//!   Cancel comes back as an error here, Update replaces the app and starts it again.
//!
//! `OMSI_UPDATE_URL=<url or file:///…json>` points the check at another release description
//! (for testing: a file in the GitHub API's format whose asset URLs may be `file://` too),
//! `OMSI_NO_UPDATE=1` switches the check off. A development build (run from a cargo `target`
//! folder) checks, but never replaces itself.

// (a phone installs through the system: the unpacking and swapping below are the computers')
#![cfg_attr(target_os = "android", allow(dead_code))]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The project on GitHub.
pub const REPO: &str = "openOMSI-Project/openOMSI";
pub const REPO_URL: &str = "https://github.com/openOMSI-Project/openOMSI";
const LATEST_API: &str = "https://api.github.com/repos/openOMSI-Project/openOMSI/releases/latest";

/// A release newer than this build, with the file for this platform.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    /// The release's page on GitHub.
    pub page: String,
    pub notes: String,
    pub asset_name: String,
    pub asset_url: String,
    pub size: u64,
    /// `sha256:<hex>` as GitHub lists it for the asset (None for releases older than that).
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading { release: Release, done: u64, total: u64 },
    Installing(Release),
    /// Android: the system's installer has the APK and asks the player.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    WaitingForInstaller(Release),
    /// A computer: the new launcher is being started; this one ends.
    Restarting(Release),
    Failed(String),
}

/// The launcher's updater: its state (shared with the worker thread), and whether the
/// player put the offer aside for this start.
pub struct Updater {
    status: Arc<Mutex<Status>>,
    pub dismissed: bool,
    /// Checked (or asked not to) once since the start.
    pub checked_once: bool,
    /// `update_auto` started the installation (once a start), and the new launcher was
    /// started.
    pub auto_started: bool,
    pub relaunched: bool,
    /// This start follows an update to this version (shown for a few seconds).
    pub updated: Option<(String, std::time::Instant)>,
}

impl Default for Updater {
    fn default() -> Self {
        Updater { status: Arc::new(Mutex::new(Status::Idle)), dismissed: false, checked_once: false, auto_started: false, relaunched: false, updated: None }
    }
}

fn lock(s: &Mutex<Status>) -> std::sync::MutexGuard<'_, Status> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

impl Updater {
    pub fn status(&self) -> Status {
        lock(&self.status).clone()
    }

    fn set(&self, s: Status) {
        *lock(&self.status) = s;
    }

    /// Ask GitHub for the latest release (in the background).
    pub fn check(&mut self) {
        self.checked_once = true;
        if matches!(self.status(), Status::Checking | Status::Downloading { .. } | Status::Installing(_) | Status::WaitingForInstaller(_) | Status::Restarting(_)) {
            return;
        }
        self.dismissed = false;
        self.set(Status::Checking);
        let status = self.status.clone();
        std::thread::spawn(move || {
            let s = match latest() {
                Ok(Some(r)) => Status::Available(r),
                Ok(None) => Status::UpToDate,
                Err(e) => {
                    log::warn!("update check: {e:#}");
                    Status::Failed(format!("Could not check for updates: {e}"))
                }
            };
            log::info!("update check: {s:?}");
            *lock(&status) = s;
        });
    }

    /// Download and install `r` (in the background).
    pub fn install(&mut self, r: Release) {
        if matches!(self.status(), Status::Downloading { .. } | Status::Installing(_) | Status::WaitingForInstaller(_) | Status::Restarting(_)) {
            return;
        }
        self.dismissed = false;
        self.set(Status::Downloading { release: r.clone(), done: 0, total: r.size });
        let status = self.status.clone();
        std::thread::spawn(move || {
            let result = download_and_install(&r, &status);
            if let Err(e) = result {
                log::warn!("update to {}: {e:#}", r.version);
                *lock(&status) = Status::Failed(format!("openOMSI was not updated to {}: {e}", r.version));
            }
        });
    }

    /// Android: what the system's installer answered (polled every frame while it has the
    /// APK). A computer: nothing to poll.
    pub fn poll(&mut self) {
        #[cfg(target_os = "android")]
        if let Status::WaitingForInstaller(r) = self.status() {
            match crate::android::install_status() {
                Some((3, _)) => self.set(Status::Failed(format!("openOMSI was not updated to {}: the installation was cancelled.", r.version))),
                Some((4, msg)) => self.set(Status::Failed(format!("openOMSI was not updated to {}: {}", r.version, if msg.is_empty() { "the system's installer refused the package." } else { msg.as_str() }))),
                Some((6, _)) => self.set(Status::Failed(format!("openOMSI was not updated to {}: installing apps was not allowed for openOMSI (Settings → Apps → openOMSI → Install unknown apps).", r.version))),
                // (2, success: the system ends this process and starts the new app)
                _ => {}
            }
        }
    }

    /// Forget a failure or an offer (the dialog's "Close" / "Not now").
    pub fn dismiss(&mut self) {
        self.dismissed = true;
        if matches!(self.status(), Status::Failed(_) | Status::UpToDate) {
            self.set(Status::Idle);
        }
    }
}

// --- versions ------------------------------------------------------------------------------

/// This build's version.
pub fn current_version() -> &'static str {
    crate::startup::VERSION
}

/// `0.1.7` / `v0.1.7` as numbers (missing parts are 0).
fn version_parts(v: &str) -> Vec<u64> {
    v.trim().trim_start_matches(['v', 'V']).split(['.', '-', '+']).map_while(|p| p.parse::<u64>().ok()).collect()
}

/// Whether `candidate` is a newer version than `current`.
pub fn newer(candidate: &str, current: &str) -> bool {
    let (mut a, mut b) = (version_parts(candidate), version_parts(current));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    !a.is_empty() && a > b
}

/// The release file for this platform, as `release.yml` names it.
pub fn asset_name(version: &str) -> Option<String> {
    let suffix = if cfg!(target_os = "android") {
        "android-arm64.apk"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "windows-x64.zip"
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        "windows-arm64.zip"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos-arm64.zip"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "macos-x64.zip"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-x64.zip"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "linux-arm64.zip"
    } else {
        return None;
    };
    Some(format!("openOMSI-{version}-{suffix}"))
}

// --- the release ----------------------------------------------------------------------------

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(15))
        .timeout_read(std::time::Duration::from_secs(60))
        .user_agent(&format!("openOMSI/{} (updater)", current_version()))
        .build()
}

/// A URL's body: `file://` read from the disk (tests), anything else over HTTP(S).
fn fetch_text(url: &str) -> anyhow::Result<String> {
    if let Some(p) = url.strip_prefix("file://") {
        return Ok(std::fs::read_to_string(p)?);
    }
    let r = agent().get(url).set("Accept", "application/vnd.github+json").call().map_err(|e| anyhow::anyhow!("{}", short_error(&e)))?;
    Ok(r.into_string()?)
}

fn short_error(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("the server answered {code}"),
        ureq::Error::Transport(t) => format!("no connection ({})", t.kind()),
    }
}

/// The latest release when it is newer than this build and has a file for this platform.
pub fn latest() -> anyhow::Result<Option<Release>> {
    let url = omsi_cfg::env::var("OMSI_UPDATE_URL").unwrap_or_else(|_| LATEST_API.to_string());
    let v: serde_json::Value = serde_json::from_str(&fetch_text(&url)?)?;
    parse_release(&v, current_version())
}

/// A release described as the GitHub API does, when newer than `current`.
fn parse_release(v: &serde_json::Value, current: &str) -> anyhow::Result<Option<Release>> {
    let tag = v["tag_name"].as_str().ok_or_else(|| anyhow::anyhow!("the release has no tag"))?;
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) || !newer(&version, current) {
        return Ok(None);
    }
    let Some(want) = asset_name(&version) else { return Ok(None) };
    let Some(a) = v["assets"].as_array().and_then(|a| a.iter().find(|a| a["name"].as_str() == Some(want.as_str()))) else {
        // (the release is still being built: its files come a few minutes after the tag)
        log::info!("update check: {version} has no {want} (yet)");
        return Ok(None);
    };
    Ok(Some(Release {
        version,
        page: v["html_url"].as_str().map(str::to_string).unwrap_or_else(|| format!("{REPO_URL}/releases/tag/{tag}")),
        notes: v["body"].as_str().unwrap_or("").to_string(),
        asset_name: want,
        asset_url: a["browser_download_url"].as_str().ok_or_else(|| anyhow::anyhow!("the release file has no address"))?.to_string(),
        size: a["size"].as_u64().unwrap_or(0),
        sha256: a["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(|h| h.to_ascii_lowercase()),
    }))
}

/// Where downloads wait (the data folder: the program's own folder is only written when the
/// new files go in).
fn download_dir() -> PathBuf {
    let d = omsi_launcher_lib::data_dir().join("updates");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Download the release file to `to`, with the progress in `status`, and check it.
fn download(r: &Release, to: &Path, status: &Mutex<Status>) -> anyhow::Result<()> {
    use sha2::Digest;
    let part = to.with_extension("part");
    let mut hasher = sha2::Sha256::new();
    let mut out = std::fs::File::create(&part)?;
    let (mut reader, total): (Box<dyn Read>, u64) = if let Some(p) = r.asset_url.strip_prefix("file://") {
        let f = std::fs::File::open(p)?;
        let n = f.metadata()?.len();
        (Box::new(f), n)
    } else {
        let resp = agent().get(&r.asset_url).set("Accept", "application/octet-stream").call().map_err(|e| anyhow::anyhow!("{}", short_error(&e)))?;
        let n = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(r.size);
        (Box::new(resp.into_reader()), n)
    };
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        done += n as u64;
        *lock(status) = Status::Downloading { release: r.clone(), done, total: total.max(done) };
    }
    out.flush()?;
    drop(out);
    if r.size > 0 && done != r.size {
        anyhow::bail!("the download stopped at {} of {} bytes", done, r.size);
    }
    if let Some(want) = &r.sha256 {
        let got: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if &got != want {
            let _ = std::fs::remove_file(&part);
            anyhow::bail!("the downloaded file is damaged (SHA-256 {got}, GitHub lists {want})");
        }
    }
    std::fs::rename(&part, to)?;
    Ok(())
}

fn download_and_install(r: &Release, status: &Mutex<Status>) -> anyhow::Result<()> {
    // (on a computer: where it goes must be writable before 15 MB are fetched for nothing)
    #[cfg(not(target_os = "android"))]
    {
        let place = install_place()?;
        if !writable(&place.dir) {
            let admin = if cfg!(windows) { " (or start it once as administrator)" } else { "" };
            anyhow::bail!("the folder {} cannot be written. Put openOMSI in a folder of yours{admin} and update again", short_path(&place.dir));
        }
    }
    let file = download_dir().join(&r.asset_name);
    download(r, &file, status)?;
    log::info!("update {}: downloaded {}", r.version, file.display());
    *lock(status) = Status::Installing(r.clone());
    #[cfg(target_os = "android")]
    {
        crate::android::install_apk(&file)?;
        // (the new version is noted: the app that starts after the update says so)
        let _ = std::fs::write(download_dir().join("updating-to"), &r.version);
        *lock(status) = Status::WaitingForInstaller(r.clone());
        Ok(())
    }
    #[cfg(not(target_os = "android"))]
    {
        let place = install_place()?;
        install_archive(&file, &place)?;
        let _ = std::fs::write(download_dir().join("updating-to"), &r.version);
        let _ = std::fs::remove_file(&file);
        *lock(status) = Status::Restarting(r.clone());
        Ok(())
    }
}

// --- installing on a computer -----------------------------------------------------------------

/// Where this program is installed: its folder, and on macOS the `.app` bundle it runs in.
#[derive(Clone, Debug)]
pub struct Place {
    pub dir: PathBuf,
    pub bundle: Option<PathBuf>,
    pub exe: PathBuf,
}

/// The installation this process runs from (refused for a development build).
pub fn install_place() -> anyhow::Result<Place> {
    let exe = std::env::current_exe()?;
    let exe = exe.canonicalize().unwrap_or(exe);
    if is_dev_build(&exe) {
        anyhow::bail!("this is a development build ({}); update it with git and cargo", short_path(&exe));
    }
    // (macOS runs an app opened straight from Downloads from a read-only copy elsewhere)
    if exe.to_string_lossy().contains("/AppTranslocation/") {
        anyhow::bail!("macOS runs openOMSI from a temporary read-only copy. Move openOMSI.app into your Applications folder (or any folder), start it from there and update again");
    }
    let bundle = exe.ancestors().find(|p| p.extension().map(|e| e.eq_ignore_ascii_case("app")).unwrap_or(false)).map(Path::to_path_buf);
    let dir = bundle.as_deref().unwrap_or(&exe).parent().ok_or_else(|| anyhow::anyhow!("no folder around {}", exe.display()))?.to_path_buf();
    Ok(Place { dir, bundle, exe })
}

/// A path short enough for a dialog's line: its end, where the telling part is.
fn short_path(p: &Path) -> String {
    let s = p.display().to_string();
    let n = s.chars().count();
    if n <= 48 {
        s
    } else {
        format!("…{}", s.chars().skip(n - 46).collect::<String>())
    }
}

/// Whether files can be put into `dir` (the check before anything is downloaded).
fn writable(dir: &Path) -> bool {
    let probe = dir.join(".openomsi-write-test");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Run from a cargo `target/<profile>` folder.
pub fn is_dev_build(exe: &Path) -> bool {
    let text = exe.to_string_lossy().to_ascii_lowercase();
    let parts: Vec<&str> = text.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let profile = |s: &str| s == "release" || s == "debug";
    parts.windows(2).any(|w| w[0] == "target" && profile(w[1])) || parts.windows(3).any(|w| w[0] == "target" && profile(w[2]))
}

const STAGING: &str = ".openomsi-update";
const MANIFEST: &str = ".openomsi-files";
const OLD: &str = ".old-update";

fn old_of(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(OLD);
    PathBuf::from(s)
}

/// Unpack `zip` into `to` (a fresh folder), with the files' Unix modes and links.
fn unpack(zip: &Path, to: &Path) -> anyhow::Result<()> {
    let _ = std::fs::remove_dir_all(to);
    std::fs::create_dir_all(to)?;
    let mut a = zip::ZipArchive::new(std::fs::File::open(zip)?)?;
    for i in 0..a.len() {
        let mut e = a.by_index(i)?;
        let Some(rel) = e.enclosed_name() else { continue };
        // (macOS' resource forks, which ditto keeps beside the files)
        if rel.components().next().map(|c| c.as_os_str() == "__MACOSX").unwrap_or(false) || rel.file_name().map(|n| n.to_string_lossy().starts_with("._")).unwrap_or(false) {
            continue;
        }
        let out = to.join(&rel);
        if e.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mode = e.unix_mode();
        #[cfg(unix)]
        if let Some(m) = mode {
            if m & 0o170000 == 0o120000 {
                let mut target = String::new();
                e.read_to_string(&mut target)?;
                std::os::unix::fs::symlink(target, &out)?;
                continue;
            }
        }
        let mut f = std::fs::File::create(&out)?;
        std::io::copy(&mut e, &mut f)?;
        drop(f);
        #[cfg(unix)]
        if let Some(m) = mode {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(m & 0o7777))?;
        }
        #[cfg(not(unix))]
        let _ = mode;
    }
    Ok(())
}

/// What the archive's top-level item `name` replaces in the installation.
fn target_of(name: &str, place: &Place) -> PathBuf {
    // (the macOS app, whatever the player named the one that runs)
    if name.to_ascii_lowercase().ends_with(".app") {
        if let Some(b) = &place.bundle {
            return b.clone();
        }
    }
    place.dir.join(name)
}

/// Put the program files of the release archive `zip` into the installation at `place`.
pub fn install_archive(zip: &Path, place: &Place) -> anyhow::Result<()> {
    let staging = place.dir.join(STAGING);
    unpack(zip, &staging).map_err(|e| anyhow::anyhow!("could not unpack the update into {} ({e})", short_path(&place.dir)))?;
    let mut items: Vec<String> = std::fs::read_dir(&staging)?.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    items.sort();
    if items.is_empty() {
        anyhow::bail!("the update archive is empty");
    }
    // a program must be among them: a wrong archive must not replace anything
    let has_program = items.iter().any(|n| {
        let l = n.to_ascii_lowercase();
        l == "openomsi" || l == "openomsi.exe" || l.ends_with(".app")
    });
    if !has_program {
        anyhow::bail!("the update archive holds no openOMSI program");
    }
    // what an earlier update put here and this one no longer brings
    let before: Vec<String> = std::fs::read_to_string(place.dir.join(MANIFEST)).map(|t| t.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.contains(['/', '\\']) && l != ".." && l != ".").collect()).unwrap_or_default();
    let gone: Vec<String> = before.into_iter().filter(|n| !items.iter().any(|i| i.eq_ignore_ascii_case(n))).collect();
    // swap: the old file aside, the new one in; undone completely on the first failure
    let mut moved: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    let result = (|| -> anyhow::Result<()> {
        for name in &items {
            let target = target_of(name, place);
            let old = old_of(&target);
            remove_any(&old);
            let had = target.exists() || target.is_symlink();
            if had {
                std::fs::rename(&target, &old).map_err(|e| anyhow::anyhow!("{} cannot be replaced ({e}) - is the folder writable?", short_path(&target)))?;
            }
            moved.push((target.clone(), had.then_some(old)));
            std::fs::rename(staging.join(name), &target).map_err(|e| anyhow::anyhow!("{} cannot be written ({e})", short_path(&target)))?;
        }
        for name in &gone {
            let target = place.dir.join(name);
            if target.exists() {
                let old = old_of(&target);
                remove_any(&old);
                std::fs::rename(&target, &old)?;
                moved.push((target, None));
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        // back as it was
        for (target, old) in moved.into_iter().rev() {
            match old {
                Some(old) => {
                    remove_any(&target);
                    let _ = std::fs::rename(&old, &target);
                }
                None => {
                    let o = old_of(&target);
                    if o.exists() {
                        let _ = std::fs::rename(&o, &target);
                    } else {
                        remove_any(&target);
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    let _ = std::fs::write(place.dir.join(MANIFEST), items.join("\n") + "\n");
    let _ = std::fs::remove_dir_all(&staging);
    log::info!("update: {} replaced in {}", items.join(", "), place.dir.display());
    Ok(())
}

fn remove_any(p: &Path) {
    if p.is_dir() && !p.is_symlink() {
        let _ = std::fs::remove_dir_all(p);
    } else {
        let _ = std::fs::remove_file(p);
    }
}

/// Start the new launcher (after `install_archive`): the program file at the same place -
/// on macOS inside the bundle, which makes it the app with its icon - with this process's
/// environment. (A script driving this launcher - `OMSI_LAUNCHER_INPUT` - is not handed on:
/// it was meant for this one.)
pub fn relaunch(place: &Place) -> anyhow::Result<()> {
    let mut cmd = std::process::Command::new(&place.exe);
    cmd.current_dir(&place.dir).env_remove("OMSI_LAUNCHER_INPUT");
    cmd.spawn()?;
    Ok(())
}

/// At the start: the files an update set aside are no longer running - away with them
/// (tried for a while: on Windows the old program may take a moment to end).
pub fn cleanup_after_update() {
    let Ok(place) = install_place() else { return };
    std::thread::spawn(move || {
        for _ in 0..20 {
            let mut left = false;
            if let Ok(rd) = std::fs::read_dir(&place.dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.file_name().map(|n| n.to_string_lossy().ends_with(OLD)).unwrap_or(false) {
                        remove_any(&p);
                        left |= p.exists();
                    }
                }
            }
            let staging = place.dir.join(STAGING);
            if staging.exists() {
                remove_any(&staging);
            }
            if !left {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });
}

/// After an update: the version this start was updated to (once), for the launcher to say.
pub fn just_updated() -> Option<String> {
    let p = download_dir().join("updating-to");
    let v = std::fs::read_to_string(&p).ok()?.trim().to_string();
    if v.is_empty() {
        return None;
    }
    // (only when it is this build: a cancelled Android installation leaves the note)
    if !newer(&v, current_version()) {
        let _ = std::fs::remove_file(&p);
        // old APKs and archives go too
        if let Ok(rd) = std::fs::read_dir(download_dir()) {
            for e in rd.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        return (v == current_version()).then_some(v);
    }
    None
}

/// Open a web page in the system's browser.
pub fn open_url(url: &str) {
    #[cfg(target_os = "android")]
    crate::android::open_url(url);
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("rundll32").arg("url.dll,FileProtocolHandler").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.1.8", "0.1.7"));
        assert!(newer("v0.1.10", "0.1.9"));
        assert!(newer("0.2.0", "0.1.99"));
        assert!(newer("1.0", "0.9.9"));
        assert!(!newer("0.1.7", "0.1.7"));
        assert!(!newer("v0.1.6", "0.1.7"));
        assert!(!newer("garbage", "0.1.7"));
    }

    #[test]
    fn the_platform_file_of_a_github_release() {
        let name = asset_name("0.1.9").unwrap();
        let v = serde_json::json!({
            "tag_name": "v0.1.9", "html_url": "https://github.com/openOMSI-Project/openOMSI/releases/tag/v0.1.9", "body": "notes",
            "assets": [
                {"name": "openOMSI-0.1.9-server-linux-x64.zip", "browser_download_url": "https://x/server", "size": 5},
                {"name": name, "browser_download_url": "https://x/mine", "size": 42, "digest": "sha256:ABCDEF"}
            ]
        });
        let r = parse_release(&v, "0.1.7").unwrap().unwrap();
        assert_eq!((r.version.as_str(), r.asset_url.as_str(), r.size, r.sha256.as_deref()), ("0.1.9", "https://x/mine", 42, Some("abcdef")));
        // not newer, a draft, or without this platform's file: nothing to offer
        assert!(parse_release(&v, "0.1.9").unwrap().is_none());
        let mut d = v.clone();
        d["draft"] = serde_json::json!(true);
        assert!(parse_release(&d, "0.1.7").unwrap().is_none());
        let mut n = v.clone();
        n["assets"] = serde_json::json!([]);
        assert!(parse_release(&n, "0.1.7").unwrap().is_none());
    }

    #[test]
    fn development_builds_are_recognised() {
        assert!(is_dev_build(Path::new("/src/openOMSI/target/release/openomsi")));
        assert!(is_dev_build(Path::new("C:\\src\\target\\x86_64-pc-windows-msvc\\release\\openomsi.exe")));
        assert!(!is_dev_build(Path::new("/Applications/openOMSI.app/Contents/MacOS/openomsi")));
        assert!(!is_dev_build(Path::new("/home/me/Games/openOMSI/openomsi")));
    }

    /// The whole swap on a folder: the program files replaced, a file only the old version
    /// had removed, the player's things left alone, the old files set aside.
    #[test]
    fn an_archive_replaces_the_program_and_nothing_else() {
        let root = std::env::temp_dir().join(format!("omsi_update_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("install");
        std::fs::create_dir_all(dir.join("Vehicles/MyMod")).unwrap();
        std::fs::write(dir.join("openomsi"), "old game").unwrap();
        std::fs::write(dir.join("openomsi-launcher"), "old cli").unwrap();
        std::fs::write(dir.join("retired.dll"), "old").unwrap();
        std::fs::write(dir.join("Vehicles/MyMod/bus.bus"), "mod").unwrap();
        std::fs::write(dir.join(MANIFEST), "openomsi\nopenomsi-launcher\nretired.dll\n").unwrap();
        // the new release
        let zip_path = root.join("new.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let o = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
            for (n, body) in [("openomsi", "new game"), ("openomsi-launcher", "new cli"), ("README.md", "readme")] {
                z.start_file(n, o).unwrap();
                z.write_all(body.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        let place = Place { dir: dir.clone(), bundle: None, exe: dir.join("openomsi") };
        install_archive(&zip_path, &place).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("openomsi")).unwrap(), "new game");
        assert_eq!(std::fs::read_to_string(dir.join("openomsi-launcher")).unwrap(), "new cli");
        assert_eq!(std::fs::read_to_string(dir.join("README.md")).unwrap(), "readme");
        assert!(!dir.join("retired.dll").exists());
        assert_eq!(std::fs::read_to_string(dir.join("Vehicles/MyMod/bus.bus")).unwrap(), "mod");
        assert_eq!(std::fs::read_to_string(dir.join("openomsi.old-update")).unwrap(), "old game");
        assert!(!dir.join(STAGING).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join("openomsi")).unwrap().permissions().mode() & 0o777, 0o755);
        }
        // an archive without a program replaces nothing
        let bad = root.join("bad.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&bad).unwrap());
            z.start_file("notes.txt", zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b"x").unwrap();
            z.finish().unwrap();
        }
        assert!(install_archive(&bad, &place).is_err());
        assert_eq!(std::fs::read_to_string(dir.join("openomsi")).unwrap(), "new game");
        let _ = std::fs::remove_dir_all(&root);
    }
}
