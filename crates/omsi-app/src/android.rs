//! openOMSI on Android: the NativeActivity's `android_main`.
//!
//! A phone runs one program in one window, so the launcher and the game share both: the
//! launcher hands the game's command line over (`omsi_launcher_lib::launch` keeps it
//! instead of starting a process), the window goes to the game, and when the session ends
//! (Escape, the menu's Quit) the window comes back to the launcher, which is kept as it
//! was. The app's own data (settings, profiles, sessions) lives in its private folder
//! (`HOME`); the original game, the mods and the screenshots are on the shared storage in
//! `openOMSI/`, where a cable or a file manager reaches them.
//!
//! The Java side (`android/java/.../OmsiActivity.java`) only adds what NativeActivity
//! lacks: the full screen without the system bars, the screen kept on, asking for access
//! to the shared storage, and the vibration.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use winit::platform::android::activity::AndroidApp;
use winit::platform::android::EventLoopBuilderExtAndroid;

/// Whether the first drive of this run was already started (see `Shell::switch`).
static SAFER_TRIED: AtomicBool = AtomicBool::new(false);

/// Where the app keeps what a person puts on the phone for it.
pub const SHARED: &str = "/storage/emulated/0/openOMSI";

#[no_mangle]
fn android_main(app: AndroidApp) {
    // an error is also written where a person finds it without a computer:
    // openOMSI/crash.log (or Android/data/org.openomsi.game/files/crash.log)
    let crash_files: Vec<PathBuf> = [Some(PathBuf::from(SHARED)), app.external_data_path()].into_iter().flatten().map(|d| d.join("crash.log")).collect();
    std::panic::set_hook(Box::new(move |info| {
        // (one the renderer catches - a graphics interface it cannot open, the next one is
        // tried - is no end of the game, as on a computer: logged and written as one, it
        // was reported as the crash of a game that went on, #1133, #1154, #1162, #1176)
        if omsi_render::catching() {
            log::warn!("caught by the renderer: {info}");
            return;
        }
        let text = format!("the game stopped on an error (build {BUILD}): {info}\n{}", std::backtrace::Backtrace::force_capture());
        log::error!("{text}");
        for f in &crash_files {
            let _ = std::fs::write(f, &text);
        }
    }));
    log::info!("openOMSI {VERSION} for Android, build {BUILD}");
    log::info!("device: {} {} (Android {}, API {})", prop("ro.product.manufacturer"), prop("ro.product.model"), prop("ro.build.version.release"), prop("ro.build.version.sdk"));
    // the Java activity (OmsiActivity) for the calls into it: ndk_context's context is the
    // Application, which has none of the activity's methods
    ACTIVITY.store(app.activity_as_ptr(), Ordering::Relaxed);
    // the app's own folder is the home of settings.cfg, launcher.json, the profiles
    if let Some(home) = app.internal_data_path() {
        std::env::set_var("HOME", &home);
        // Adreno 6xx-8xx Vulkan drivers corrupt the shader cache Android keeps in the app's
        // `code_cache`, and then hand back broken pipelines without an error: a black game
        // and grey previews (#1310). After a run on such a chip the cache is thrown away and
        // the shaders are built again; every other chip keeps its cache.
        if let Some(parent) = home.parent() {
            let prev = std::fs::read_to_string(home.join("game-prev.log")).unwrap_or_default();
            if prev.contains("Adreno (TM) 7") || prev.contains("Adreno (TM) 8") || prev.contains("Adreno (TM) 6") {
                let _ = std::fs::remove_dir_all(parent.join("code_cache"));
            }
        }
    }
    init_log();
    rescue_update();
    // the content folder (mods, archives, screenshots): on the shared storage when the
    // app may write there, else in the app's own folder on it
    let shared = PathBuf::from(SHARED);
    let content = if std::fs::create_dir_all(&shared).is_ok() && is_writable(&shared) {
        shared
    } else {
        log::warn!("{SHARED} cannot be written (no access to the storage yet): the content folder is the app's own");
        app.external_data_path().unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()))
    };
    std::env::set_var("OMSI_CONTENT", &content);
    let _ = std::fs::write(content.join("README.txt"), README);
    hide_from_gallery(&content);
    // `openOMSI/env.txt`: the OMSI_* switches a computer takes from its environment, one
    // `NAME=value` a line (a phone has no environment to set; for looking into problems)
    if let Ok(t) = std::fs::read_to_string(content.join("env.txt")) {
        for line in t.lines() {
            if let Some((k, v)) = line.trim().split_once('=') {
                let k = k.trim();
                if k.starts_with("OMSI_") && !k.contains(char::is_whitespace) {
                    log::info!("env.txt: {k}");
                    std::env::set_var(k, v.trim());
                }
            }
        }
    }
    log::info!("home {:?}, content {}", std::env::var_os("HOME"), content.display());
    omsi_cfg::migrate_legacy_data_dir();
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) % 1_000_000_000;
    omsi_script::set_session_seed(seed);

    let event_loop = match EventLoop::builder().with_android_app(app).build() {
        Ok(e) => e,
        Err(e) => {
            log::error!("no event loop: {e}");
            return;
        }
    };
    let mut shell = Shell { launcher: None, game: None, instance: None };
    if let Err(e) = event_loop.run_app(&mut shell) {
        log::error!("{e}");
    }
    lan_mods::clean_up();
    // (the activity ends with the program)
    std::process::exit(0);
}

/// The file a start leaves in the app's folder until the launcher has drawn its first
/// picture (`started_ok`).
fn start_mark() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("start-pending"))
}

/// A start after one that never got as far as the launcher's first picture (the graphics
/// driver hung or took the process down while the shaders were made - 0.2.4 to 0.2.10 on
/// the Galaxy S24/S25, #1708): the update is looked for at once, in the background, beside
/// whatever the start is doing, and a newer release is downloaded and handed to the
/// system's installer, which asks the player. The launcher's own check comes after its
/// first picture, and a build that cannot draw one could never be updated from within.
fn rescue_update() {
    let Some(mark) = start_mark() else { return };
    let stuck = mark.exists();
    let _ = std::fs::write(&mark, VERSION);
    if !stuck || omsi_cfg::env::var_os("OMSI_NO_UPDATE").is_some() {
        return;
    }
    log::warn!("the last start did not reach the launcher: looking for an update now");
    std::thread::spawn(|| {
        let r = match crate::updater::latest() {
            Ok(Some(r)) => r,
            Ok(None) => return log::info!("rescue update: this is the latest version"),
            Err(e) => return log::warn!("rescue update: {e:#}"),
        };
        log::warn!("rescue update: installing {}", r.version);
        let mut u = crate::updater::Updater::default();
        u.install(r);
        // (the installer's answer; on success the system ends this process)
        for _ in 0..1800 {
            std::thread::sleep(std::time::Duration::from_secs(1));
            u.poll();
            if let crate::updater::Status::Failed(e) = u.status() {
                return log::warn!("rescue update: {e}");
            }
        }
    });
}

/// The launcher (or a game) has drawn a picture: this start went well.
fn started_ok() {
    static DONE: AtomicBool = AtomicBool::new(false);
    if !DONE.swap(true, Ordering::Relaxed) {
        if let Some(m) = start_mark() {
            let _ = std::fs::remove_file(m);
        }
    }
}

/// The log to the system's (logcat) and to `game.log` in the app's data folder - a phone
/// has no terminal and the launcher and the game share one process, so without the file a
/// crash left the launcher's report empty (#229). The previous run's log is kept as
/// `game-prev.log`: when that run closed in the middle of a drive (a crash in the graphics
/// driver takes the process without a word), the launcher shows its end
/// ([`previous_run_crash`]).
fn init_log() {
    let dir = omsi_launcher_lib::data_dir();
    let (now, prev) = (dir.join("game.log"), dir.join("game-prev.log"));
    let _ = std::fs::rename(&now, &prev);
    let file = std::fs::File::create(&now).ok();
    let logger = TeeLogger {
        system: android_logger::AndroidLogger::new(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("openOMSI")),
        file: std::sync::Mutex::new(file),
    };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

struct TeeLogger {
    system: android_logger::AndroidLogger,
    file: std::sync::Mutex<Option<std::fs::File>>,
}

impl log::Log for TeeLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        self.system.log(r);
        use std::io::Write;
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
        if let Some(f) = self.file.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            // (the desktop's env_logger layout: the launcher's `crash_of` reads it)
            let _ = writeln!(f, "[{secs:.3} {:<5} {}] {}", r.level(), r.target(), r.args());
        }
    }

    fn flush(&self) {
        if let Some(f) = self.file.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            use std::io::Write;
            let _ = f.flush();
        }
    }
}

/// A system property (`ro.product.model` ...), for the log's first lines.
fn prop(name: &str) -> String {
    extern "C" {
        fn __system_property_get(name: *const std::ffi::c_char, value: *mut std::ffi::c_char) -> i32;
    }
    let (Ok(n), mut buf) = (std::ffi::CString::new(name), [0 as std::ffi::c_char; 92]) else { return String::new() };
    let len = unsafe { __system_property_get(n.as_ptr(), buf.as_mut_ptr()) };
    if len <= 0 {
        return String::new();
    }
    unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()
}

/// When the app's previous run closed while a drive was going (a game started from the
/// launcher, and neither "game ends" nor "session ended" after it): what it said last and
/// the end of its log, for the launcher's crash dialog.
pub(crate) fn previous_run_crash() -> Option<(String, String)> {
    let p = omsi_launcher_lib::data_dir().join("game-prev.log");
    let text = std::fs::read_to_string(&p).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().rposition(|l| l.contains("starting the game:"))?;
    if lines[start..].iter().any(|l| l.contains("game ends") || l.contains("session ended")) {
        return None;
    }
    // sent to the background and not brought back: the system ended the app there (or the
    // player swiped it away) - no crash, whatever the game was doing
    let back = lines[start..].iter().rposition(|l| l.contains("app in the background"));
    let front = lines[start..].iter().rposition(|l| l.contains("app in front"));
    if back.is_some() && back > front {
        return None;
    }
    if let Some(c) = launcher::crash_of(&p) {
        return Some(c);
    }
    let last = lines.last().map(|l| l.split_once("] ").map(|x| x.1).unwrap_or(l)).unwrap_or("");
    let tail = lines[lines.len().saturating_sub(150)..].join("\n");
    Some((format!("the game closed without a word while it was running (the last it said: {last})"), tail))
}

/// Android's media scanner hands every picture it finds to the gallery apps: the thousands
/// of textures of an OMSI installation and of the mods turned up there as photos, the scan
/// kept the phone busy, and people deleted them as clutter - white buses, empty maps
/// (#443). A `.nomedia` file keeps a folder and everything under it out of the gallery:
/// every folder of the content folder gets one except `Screenshots` (those are pictures to
/// find), and so does the OMSI installation when it lies somewhere else. Nothing of the
/// content is moved or changed.
fn hide_from_gallery(content: &Path) {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(content)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| !p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("Screenshots")))
        .collect();
    if let Some(root) = crate::startup::root_memo().and_then(|f| std::fs::read_to_string(f).ok()) {
        let root = PathBuf::from(root.trim());
        if root.is_dir() && !root.starts_with(content) && !content.starts_with(&root) {
            dirs.push(root);
        }
    }
    for dir in dirs {
        let marker = dir.join(".nomedia");
        if marker.exists() {
            continue;
        }
        match std::fs::write(&marker, b"") {
            Ok(()) => log::info!("{}: kept out of the gallery (.nomedia)", dir.display()),
            Err(e) => log::warn!("{}: no .nomedia ({e})", dir.display()),
        }
    }
}

/// `hide_from_gallery` for the content folder and the OMSI installation known now: right
/// after the launcher's Setup saved a new one (it waited for the next start of the app, and
/// the media scanner had put the installation's textures into the gallery by then, #1632).
pub(crate) fn hide_content_from_gallery() {
    if let Some(content) = crate::startup::content_dir() {
        hide_from_gallery(&content);
    }
}

fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".openomsi-write-test");
    let ok = std::fs::write(&probe, b"x").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

const README: &str = "openOMSI\n\
\n\
Put a complete copy of OMSI 2 (the folder with Omsi.exe, maps and Vehicles in it) here as\n\
\"OMSI 2\", e.g. openOMSI/OMSI 2, and choose it in the launcher under Setup.\n\
Mods: copy them into openOMSI/Mods (they are installed when the launcher opens), or install\n\
a folder or a .zip, .7z or .rar from the launcher's Mods page. Screenshots are written to openOMSI/Screenshots.\n\
The folders here hold a .nomedia file so that the gallery leaves the game's textures alone:\n\
they are not photos - deleting them breaks buses and maps.\n";

/// The launcher, or the game in the launcher's window.
struct Shell {
    launcher: Option<Box<launcher::Launcher>>,
    game: Option<Box<App>>,
    instance: Option<()>,
}

impl Shell {
    fn launcher(&mut self) -> &mut launcher::Launcher {
        if self.launcher.is_none() {
            // the original installation and the content roots, as a bare start finds them
            let args = Args::parse_from(["openomsi"]);
            if let Err(e) = prepare(args, true) {
                log::error!("{e:#}");
            }
            // (an installation chosen under Setup is known from here on)
            if let Some(content) = crate::startup::content_dir() {
                hide_from_gallery(&content);
            }
            launcher_statics();
            self.instance = Some(());
            self.launcher = Some(Box::new(launcher::Launcher::new(graphics_instance())));
        }
        self.launcher.as_mut().unwrap()
    }

    /// After every event: a game the launcher asked for starts, a game that ended gives
    /// the window back.
    fn switch(&mut self, event_loop: &ActiveEventLoop) {
        if self.game.is_some() {
            if !crate::platform::take_leave() {
                return;
            }
            let mut game = self.game.take().unwrap();
            game.exiting(event_loop);
            let window = game.window.take();
            drop(game);
            lan_mods::clean_up();
            log::info!("session ended: back to the launcher");
            let l = self.launcher();
            if let Some(w) = window {
                l.adopt_window(w);
            }
            l.resumed(event_loop);
            return;
        }
        let Some(line) = omsi_launcher_lib::take_in_process_launch() else { return };
        // the first drive after a run that closed in the middle of one (a graphics driver
        // that took the process down) starts with safer graphics, and on OpenGL when that
        // run drew with Vulkan: a phone whose Vulkan driver fails on the game still plays
        if !SAFER_TRIED.swap(true, Ordering::Relaxed) && previous_run_crash().is_some() {
            let prev = std::fs::read_to_string(omsi_launcher_lib::data_dir().join("game-prev.log")).unwrap_or_default();
            let vulkan = prev.lines().any(|l| l.contains("renderer: ") && l.contains("(Vulkan)")) || prev.lines().any(|l| l.contains("graphics: ") && l.to_ascii_uppercase().contains("VULKAN"));
            std::env::set_var("OMSI_SAFE_GPU", "1");
            // It went down while the graphics driver compiled the shaders (the last it said
            // was a stage of that): the phone's Vulkan driver cannot take them, and will not
            // next time either (the Maleoon and several Mali drivers after the cloud noise,
            // #229, #278). OpenGL from now on, in the settings - Settings → Graphics API
            // takes it back.
            let last = prev.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
            let compiling = last.contains("renderer: compiling") || last.contains("cloud noise made") || last.contains("opening graphics device") || last.contains("compiling renderer pipelines");
            if vulkan && compiling {
                if let Ok(mut v) = omsi_launcher_lib::get_settings() {
                    v["graphics_api"] = serde_json::json!("gl");
                    match omsi_launcher_lib::save_settings(&v) {
                        Ok(()) => log::warn!("the graphics driver went down compiling the shaders on Vulkan: OpenGL from now on (Settings → Graphics API)"),
                        Err(e) => log::warn!("settings not saved: {e:#}"),
                    }
                }
            }
            if vulkan && std::env::var_os("OMSI_BACKEND").is_none() {
                std::env::set_var("OMSI_BACKEND", "gl");
            }
            log::warn!("the last run closed in the middle of a drive: this one starts with safer graphics{}", if vulkan { " on OpenGL" } else { "" });
        }
        log::info!("starting the game: {}", line.join(" "));
        let argv: Vec<String> = std::iter::once("openomsi".to_string()).chain(line).collect();
        let args = match Args::try_parse_from(&argv) {
            Ok(a) => a,
            Err(e) => {
                log::error!("the launcher's command line: {e}");
                return;
            }
        };
        let game = prepare(args, false).and_then(|p| match p {
            Some((args, server)) => make_app(args, server),
            None => Ok(None),
        });
        let mut app = match game {
            Ok(Some(app)) => app,
            Ok(None) => return,
            Err(e) => {
                log::error!("the game could not start: {e:#}");
                return;
            }
        };
        let window = self.launcher().release_window();
        app.create_window(event_loop, window);
        self.game = Some(Box::new(app));
    }
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        log::info!("app in front");
        match self.game.as_mut() {
            Some(g) => g.resumed(event_loop),
            None => self.launcher().resumed(event_loop),
        }
        self.switch(event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        // (the system may end an app in the background without a word: see
        // `previous_run_crash`)
        log::info!("app in the background");
        match self.game.as_mut() {
            Some(g) => g.suspended(event_loop),
            None => self.launcher().suspended(event_loop),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let redraw = matches!(event, WindowEvent::RedrawRequested);
        match self.game.as_mut() {
            Some(g) => g.window_event(event_loop, id, event),
            None => self.launcher().window_event(event_loop, id, event),
        }
        if redraw {
            started_ok();
        }
        self.switch(event_loop);
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: winit::event::DeviceId, event: DeviceEvent) {
        if let Some(g) = self.game.as_mut() {
            g.device_event(event_loop, id, event);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.game.as_mut() {
            Some(g) => {
                event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
                g.about_to_wait(event_loop)
            }
            None => self.launcher().about_to_wait(event_loop),
        }
        self.switch(event_loop);
    }

    fn memory_warning(&mut self, _event_loop: &ActiveEventLoop) {
        log::warn!("the system is short of memory");
        crate::memory::release_free_memory();
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(g) = self.game.as_mut() {
            g.exiting(event_loop);
        }
    }
}

// --- the phone's tilt as a steering wheel ----------------------------------------------

static TILT_ON: AtomicBool = AtomicBool::new(false);
/// The latest turn (-1 .. 1) as f32 bits, and whether the sensor gave one yet.
static TILT: AtomicU32 = AtomicU32::new(0);
static TILT_SEEN: AtomicBool = AtomicBool::new(false);
static TILT_THREAD: std::sync::Once = std::sync::Once::new();

pub(crate) fn tilt() -> Option<f32> {
    (TILT_ON.load(Ordering::Relaxed) && TILT_SEEN.load(Ordering::Relaxed)).then(|| f32::from_bits(TILT.load(Ordering::Relaxed)))
}

pub(crate) fn set_tilt(on: bool) {
    TILT_ON.store(on, Ordering::Relaxed);
    if on {
        TILT_THREAD.call_once(|| {
            let _ = std::thread::Builder::new().name("tilt".into()).spawn(tilt_thread);
        });
    }
}

#[repr(C)]
struct SensorEvent {
    version: i32,
    sensor: i32,
    kind: i32,
    reserved0: i32,
    timestamp: i64,
    data: [f32; 16],
    flags: u32,
    reserved1: [i32; 3],
}

#[link(name = "android")]
extern "C" {
    fn ASensorManager_getInstanceForPackage(package: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    fn ASensorManager_getDefaultSensor(manager: *mut std::ffi::c_void, kind: i32) -> *const std::ffi::c_void;
    fn ASensorManager_createEventQueue(manager: *mut std::ffi::c_void, looper: *mut std::ffi::c_void, ident: i32, callback: *const std::ffi::c_void, data: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn ASensorEventQueue_enableSensor(queue: *mut std::ffi::c_void, sensor: *const std::ffi::c_void) -> i32;
    fn ASensorEventQueue_disableSensor(queue: *mut std::ffi::c_void, sensor: *const std::ffi::c_void) -> i32;
    fn ASensorEventQueue_setEventRate(queue: *mut std::ffi::c_void, sensor: *const std::ffi::c_void, usec: i32) -> i32;
    fn ASensorEventQueue_getEvents(queue: *mut std::ffi::c_void, events: *mut SensorEvent, count: usize) -> isize;
    fn ALooper_prepare(opts: i32) -> *mut std::ffi::c_void;
    fn ALooper_pollOnce(timeout_ms: i32, fd: *mut i32, events: *mut i32, data: *mut *mut std::ffi::c_void) -> i32;
}

/// Reads the accelerometer while tilt steering is on (a looper of its own, so the
/// activity's does not see the sensor's events).
fn tilt_thread() {
    const ACCELEROMETER: i32 = 1;
    // SAFETY: the NDK's sensor API used as documented, all on this one thread; the queue and
    // the sensor live as long as the thread (the program)
    unsafe {
        let looper = ALooper_prepare(0);
        let manager = ASensorManager_getInstanceForPackage(c"org.openomsi.game".as_ptr());
        if manager.is_null() {
            log::warn!("tilt steering: no sensor manager");
            return;
        }
        let sensor = ASensorManager_getDefaultSensor(manager, ACCELEROMETER);
        if sensor.is_null() {
            log::warn!("tilt steering: this device has no accelerometer");
            return;
        }
        let queue = ASensorManager_createEventQueue(manager, looper, 3, std::ptr::null(), std::ptr::null_mut());
        if queue.is_null() {
            log::warn!("tilt steering: no sensor queue");
            return;
        }
        let mut enabled = false;
        let mut smooth = 0.0f32;
        let mut events: Vec<SensorEvent> = (0..16).map(|_| std::mem::zeroed()).collect();
        loop {
            let on = TILT_ON.load(Ordering::Relaxed);
            if on != enabled {
                if on {
                    ASensorEventQueue_enableSensor(queue, sensor);
                    ASensorEventQueue_setEventRate(queue, sensor, 16_000);
                } else {
                    ASensorEventQueue_disableSensor(queue, sensor);
                    TILT_SEEN.store(false, Ordering::Relaxed);
                }
                enabled = on;
            }
            if !on {
                std::thread::sleep(std::time::Duration::from_millis(200));
                continue;
            }
            ALooper_pollOnce(100, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
            loop {
                let n = ASensorEventQueue_getEvents(queue, events.as_mut_ptr(), events.len());
                if n <= 0 {
                    break;
                }
                for e in &events[..n as usize] {
                    // held across (landscape either way round): gravity lies along the
                    // device's x axis, and turning it like a wheel moves it into y
                    let (ax, ay) = (e.data[0], e.data[1]);
                    let angle = (ay * ax.signum()).atan2(ax.abs()).to_degrees();
                    let dead = 2.5;
                    let a = if angle.abs() < dead { 0.0 } else { angle - dead * angle.signum() };
                    let turn = (a / 45.0).clamp(-1.0, 1.0);
                    smooth += (turn - smooth) * 0.35;
                    TILT.store(smooth.to_bits(), Ordering::Relaxed);
                    TILT_SEEN.store(true, Ordering::Relaxed);
                }
            }
        }
    }
}

// --- the Java side ---------------------------------------------------------------------

/// Call a `void name(int)` method of the activity (OmsiActivity.java).
/// The NativeActivity (`OmsiActivity`) instance, set in `android_main`.
static ACTIVITY: std::sync::atomic::AtomicPtr<std::ffi::c_void> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// The activity as a JNI object (the context ndk_context gives when it was not set).
fn activity_ptr() -> *mut std::ffi::c_void {
    let a = ACTIVITY.load(Ordering::Relaxed);
    if a.is_null() {
        ndk_context::android_context().context()
    } else {
        a
    }
}

fn call_activity_int(name: &str, arg: i32) -> Option<()> {
    let ctx = ndk_context::android_context();
    // SAFETY: the VM ndk_context was given and the activity android-activity holds live as
    // long as the program
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let activity = unsafe { jni::objects::JObject::from_raw(activity_ptr().cast()) };
    let r = env.call_method(&activity, name, "(I)V", &[jni::objects::JValue::Int(arg)]);
    if r.is_err() {
        let _ = env.exception_clear();
    }
    // (the activity is not ours to delete: it was lent)
    std::mem::forget(activity);
    Some(())
}

pub(crate) fn vibrate(ms: u32) {
    static OFF: AtomicBool = AtomicBool::new(false);
    if OFF.load(Ordering::Relaxed) {
        return;
    }
    if call_activity_int("vibrate", ms as i32).is_none() {
        OFF.store(true, Ordering::Relaxed);
    }
}

/// Run `f` with the Java environment and the activity (None when Java is out of reach or
/// the call threw).
fn with_activity<R>(f: impl FnOnce(&mut jni::JNIEnv, &jni::objects::JObject) -> jni::errors::Result<R>) -> Option<R> {
    let ctx = ndk_context::android_context();
    // SAFETY: as in `call_activity_int`
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let activity = unsafe { jni::objects::JObject::from_raw(activity_ptr().cast()) };
    let r = f(&mut env, &activity);
    if let Err(e) = &r {
        log::warn!("Java call failed: {e}");
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    std::mem::forget(activity);
    r.ok()
}

/// Hand the downloaded APK to the system's package installer (`OmsiActivity.installApk`):
/// the system asks the player, `install_status` tells what they answered.
pub(crate) fn install_apk(path: &std::path::Path) -> anyhow::Result<()> {
    let p = path.to_string_lossy().to_string();
    with_activity(|env, activity| {
        let s = env.new_string(&p)?;
        env.call_method(activity, "installApk", "(Ljava/lang/String;)V", &[(&s).into()])?;
        Ok(())
    })
    .ok_or_else(|| anyhow::anyhow!("the system's package installer could not be reached"))
}

/// The package installer's answer so far: 0 nothing yet, 1 asking the player, 2 installed,
/// 3 cancelled, 4 failed (with the system's message), 5 waiting for "Install unknown apps",
/// 6 that permission refused.
pub(crate) fn install_status() -> Option<(i32, String)> {
    with_activity(|env, activity| {
        let code = env.call_method(activity, "getInstallStatus", "()I", &[])?.i()?;
        let msg = env.call_method(activity, "getInstallMessage", "()Ljava/lang/String;", &[])?.l()?;
        let msg: String = if msg.is_null() { String::new() } else { env.get_string(&jni::objects::JString::from(msg))?.into() };
        Ok((code, msg))
    })
}

/// A web page in the phone's browser.
pub(crate) fn open_url(url: &str) {
    let u = url.to_string();
    let _ = with_activity(|env, activity| {
        let s = env.new_string(&u)?;
        env.call_method(activity, "openUrl", "(Ljava/lang/String;)V", &[(&s).into()])?;
        Ok(())
    });
}
