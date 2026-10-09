//! Start-up: finding the OMSI 2 installation, the graphics instance, the build, the allocator and the console.

use super::*;

/// CPU seconds this process has used so far (all threads), from `ps`.
pub(crate) fn process_cpu_seconds() -> Option<f64> {
    let out = std::process::Command::new("ps")
        .args(["-o", "cputime=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // [[dd-]hh:]mm:ss.ss
    let (days, rest) = match text.trim().split_once('-') {
        Some((d, r)) => (d.parse::<f64>().ok()?, r.to_string()),
        None => (0.0, text.trim().to_string()),
    };
    let secs = rest.split(':').try_fold(0.0, |acc, part| {
        part.parse::<f64>().ok().map(|v| acc * 60.0 + v)
    })?;
    Some(days * 86400.0 + secs)
}

/// The CPU time the calling thread has used (s): what the frame's own work costs, whatever
/// else the machine runs meanwhile - wall-clock stage times grow with every other busy
/// program, a thread's CPU time hardly.
pub(crate) fn thread_cpu_seconds() -> Option<f64> {
    #[cfg(unix)]
    {
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: `ts` is a valid timespec for the call to fill.
        if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) } == 0 {
            return Some(ts.tv_sec as f64 + ts.tv_nsec as f64 * 1e-9);
        }
        None
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// The instructions the process has retired (macOS): the work done, counted the same however
/// busy the machine is and whichever cores ran it - CPU times grow when other programs push
/// the game's threads onto the efficiency cores.
pub(crate) fn process_instructions() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a rusage_info_v4 for the call to fill, as RUSAGE_INFO_V4 says.
        let r = unsafe { libc::proc_pid_rusage(std::process::id() as i32, libc::RUSAGE_INFO_V4, &mut info as *mut _ as *mut libc::rusage_info_t) };
        (r == 0).then_some(info.ri_instructions)
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

pub(crate) fn shift_held_now(keys: &hashbrown::HashSet<KeyCode>) -> bool {
    keys.contains(&KeyCode::ShiftLeft) || keys.contains(&KeyCode::ShiftRight)
}

/// Does this folder look like an OMSI 2 installation?
pub(crate) fn is_omsi_root(p: &Path) -> bool {
    // a complete installation of the original game (openOMSI's own content folder has the
    // same layout, but it is a mod overlay, not the game)
    omsi_cfg::missing_original_essentials(p).is_empty()
}

/// Say that the game cannot start, where the player sees it: a dialog when there is no
/// terminal to print to (a double click, the launcher), and the log as always.
pub(crate) fn fatal_dialog(title: &str, text: &str) {
    log::error!("{title}: {text}");
    // started from a terminal (the message is right there) or by a test harness
    if std::io::IsTerminal::is_terminal(&std::io::stderr()) || omsi_cfg::flags::OMSI_BACKGROUND.is_set() {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "display alert \"{}\" message \"{}\" as critical",
            esc(title),
            esc(text)
        );
        let _ = std::process::Command::new("osascript").arg("-e").arg(script).status();
    }
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" {
            fn MessageBoxW(hwnd: *mut core::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
        }
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (t, c) = (wide(text), wide(title));
        // MB_OK | MB_ICONERROR
        unsafe {
            MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), 0x10);
        }
    }
}

/// openOMSI's own content folder: the folder of the game binary, laid out like an OMSI 2
/// installation (Vehicles, maps, Sceneryobjects ...). Mods live here; it is searched before
/// the original installation.
/// The `Inputs/keyboard.cfg` the game follows: the content folder's once the launcher has
/// saved key bindings there, else the original installation's (never written) - or, where
/// that has none, its `keyboard_reset.cfg`, the standard keys OMSI falls back to as well.
pub(crate) fn keyboard_cfg(root: &Path) -> PathBuf {
    if let Some(own) = content_dir().map(|c| c.join("Inputs/keyboard.cfg")).filter(|p| p.exists()) {
        return own;
    }
    omsi_cfg::original_keyboard_cfg(root)
}

/// The keys (scan codes without a modifier) the player's own `keyboard.cfg` (the content
/// folder's, written by the launcher) binds to something the original's does not bind to
/// them: see `App::own_keys`.
pub(crate) fn own_keys(root: &Path) -> std::collections::HashSet<i32> {
    own_bindings(root, 0)
}

/// The keys held with `modifier` (a chord: `KEY_SHIFT` …) that the file in use binds otherwise than OMSI 2's
/// own assignment ([`crate::stock_keys::STOCK_KEYS`]): the player's own. Told apart from the
/// built-in list, not from the installation's file - a player who edited that file had
/// every change overridden by the game's conveniences (Z / X / C, Shift+number).
pub(crate) fn own_bindings(root: &Path, modifier: i32) -> std::collections::HashSet<i32> {
    let Ok(m) = omsi_content::KeyboardCfg::load(&keyboard_cfg(root)) else { return Default::default() };
    let stock: std::collections::HashSet<(String, i32, i32)> = crate::stock_keys::STOCK_KEYS.iter().map(|(a, k, md)| (a.to_ascii_lowercase(), *k, *md)).collect();
    m.vehicles
        .iter()
        .chain(m.game.iter())
        .filter(|b| b.chord() == modifier && b.scan_code != 0 && !stock.contains(&(b.action.to_ascii_lowercase(), b.scan_code, b.modifier)))
        .map(|b| b.scan_code)
        .collect()
}

pub(crate) fn content_dir() -> Option<PathBuf> {
    if let Some(d) = omsi_cfg::flags::OMSI_CONTENT.os() {
        return Some(PathBuf::from(d));
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();
    // inside an .app bundle the binary sits in Contents/MacOS: use the folder beside the bundle
    let dir = if dir.ends_with("Contents/MacOS") {
        dir.parent()?.parent()?.parent()?.to_path_buf()
    } else {
        dir
    };
    let cand = omsi_cfg::content_folder_of(&dir);
    if !omsi_cfg::is_programs_folder(&cand) && (cand.exists() || std::fs::create_dir_all(&cand).is_ok()) && omsi_cfg::is_writable(&cand) {
        Some(cand)
    } else {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
        let fallback = PathBuf::from(home).join(".openomsi").join("content");
        let _ = omsi_cfg::ensure_content_layout(&fallback);
        Some(fallback)
    }
}

/// Where a save goes when the content folder takes none (`~/.openomsi/content`): a game
/// unpacked into Program Files, or a folder Windows' controlled folder access guards, said
/// "Could not save: access is denied (os error 5)" (#1673). The launcher lists the saves
/// there as well.
pub(crate) fn save_fallback_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".openomsi").join("content"))
}

/// Where the last working installation was remembered.
pub(crate) fn root_memo() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".openomsi-root"))
}

/// Find the OMSI 2 installation without being told where it is.
pub(crate) fn find_root() -> Option<PathBuf> {
    let mut first: Vec<PathBuf> = Vec::new();
    if let Some(p) = omsi_cfg::flags::OMSI_ROOT.os().map(PathBuf::from) {
        first.push(p);
    }
    if let Some(memo) = root_memo() {
        if let Ok(text) = std::fs::read_to_string(&memo) {
            first.push(PathBuf::from(text.trim()));
        }
    }
    // the folder the launcher was told about (Setup)
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        if let Ok(t) = std::fs::read_to_string(PathBuf::from(home).join(".openomsi/launcher.json")) {
            if let Some(r) = serde_json::from_str::<serde_json::Value>(&t).ok().and_then(|v| v.get("root").and_then(|r| r.as_str()).map(PathBuf::from)) {
                first.push(r);
            }
        }
    }
    omsi_cfg::find_original_install(&first)
}

/// Use the native supported API for both windowed and offscreen rendering.
pub(crate) fn graphics_instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    if crate::server::SERVER_MODE.load(std::sync::atomic::Ordering::Relaxed) {
        // the dedicated server draws nothing: wgpu's no-op device takes every call
        descriptor.backends = wgpu::Backends::NOOP;
        descriptor.backend_options.noop = wgpu::NoopBackendOptions { enable: true };
        return wgpu::Instance::new(descriptor);
    }
    descriptor.backend_options.gl.context_lock_timeout = Some(GL_CONTEXT_LOCK_TIMEOUT);
    // (an interface whose only adapter is a software renderer - DirectX 12's "Microsoft
    // Basic Render Driver" on a chip without a DirectX 12 driver - comes after the others)
    let (mut software, mut last) = (None, None);
    for b in backend_order() {
        let instance = backend_instance(b);
        let adapters = pollster::block_on(instance.enumerate_adapters(b.backends()));
        if adapters.iter().any(|a| !is_software(&a.get_info())) {
            log::info!("graphics: {:?} ({})", b, adapters.iter().map(|a| a.get_info().name).collect::<Vec<_>>().join(", "));
            return instance;
        }
        if adapters.is_empty() {
            log::info!("graphics: no {b:?} adapter here");
            last = Some(instance);
        } else {
            log::info!("graphics: {b:?} has only a software renderer ({})", adapters.iter().map(|a| a.get_info().name).collect::<Vec<_>>().join(", "));
            software.get_or_insert(instance);
        }
    }
    software.or(last).unwrap_or_else(|| wgpu::Instance::new(descriptor))
}

/// How long a thread waits for the shared OpenGL context before wgpu gives up on it (its own
/// default is a second with WGL, six with EGL): one waiting for the GPU no longer holds the
/// context, but a long upload or shader compile on another thread still can.
const GL_CONTEXT_LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A graphics interface to try: one of wgpu's backends, or its OpenGL ES backend on ANGLE.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum GraphicsApi {
    Native(wgpu::Backends),
    /// OpenGL ES on ANGLE's Direct3D 11 renderer (Windows only), for a chip whose OpenGL,
    /// Vulkan and DirectX 12 drivers are missing or broken (Intel HD Graphics 2000-4000, AMD
    /// Radeon HD 5000/6000): its DirectX 11 driver draws
    Angle,
}

impl GraphicsApi {
    /// wgpu's backend it draws on (ANGLE: the OpenGL one)
    fn backends(self) -> wgpu::Backends {
        match self {
            GraphicsApi::Native(b) => b,
            GraphicsApi::Angle => wgpu::Backends::GL,
        }
    }
}

// (the logs name a backend as before: `Backends(DX12)`)
impl std::fmt::Debug for GraphicsApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphicsApi::Native(b) => b.fmt(f),
            GraphicsApi::Angle => f.write_str("ANGLE (DirectX 11)"),
        }
    }
}

/// Is ANGLE there to be tried: on Windows, with Google's `libEGL.dll` and `libGLESv2.dll`
/// next to the game (the package brings them; without them it is left out, with a line in
/// the log once).
fn angle_available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        if !cfg!(windows) {
            return false;
        }
        let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) else { return false };
        let missing: Vec<&str> = ["libEGL.dll", "libGLESv2.dll"].into_iter().filter(|f| !dir.join(f).is_file()).collect();
        if !missing.is_empty() {
            log::info!("graphics: ANGLE (DirectX 11) is not tried: no {} next to the game", missing.join(" or "));
        }
        missing.is_empty()
    })
}

/// The graphics interfaces in the order they are tried: Metal on a Mac; on Windows DirectX
/// 12 first (the Windows drivers' best-kept path: on Vulkan they reset the device -
/// "the graphics device was lost" - far more often), then Vulkan, then OpenGL for a card
/// without either (a GeForce GT 530), then OpenGL ES on ANGLE over DirectX 11 when its DLLs
/// are there; elsewhere Vulkan, then OpenGL. Settings → Graphics API (`graphics_api`) or
/// OMSI_BACKEND=vulkan|dx12|gl|angle puts one first: a driver whose Vulkan misbehaves is
/// got round.
pub(crate) fn backend_order() -> Vec<GraphicsApi> {
    if cfg!(target_os = "macos") {
        return vec![GraphicsApi::Native(wgpu::Backends::METAL)];
    }
    let settings = crate::settings::Settings::load();
    let wanted = if settings.vr_requested() {
        "dx12".to_owned()
    } else {
        omsi_cfg::flags::OMSI_BACKEND.var().map(str::to_string).unwrap_or(settings.graphics_api)
    };
    let native = if cfg!(windows) {
        vec![wgpu::Backends::DX12, wgpu::Backends::VULKAN, wgpu::Backends::GL]
    } else {
        vec![wgpu::Backends::VULKAN, wgpu::Backends::GL]
    };
    let mut all: Vec<GraphicsApi> = native.into_iter().map(GraphicsApi::Native).collect();
    if cfg!(windows) && angle_available() {
        all.push(GraphicsApi::Angle);
    }
    let first = match wanted.trim().to_ascii_lowercase().as_str() {
        "vulkan" => Some(GraphicsApi::Native(wgpu::Backends::VULKAN)),
        "dx12" | "directx" | "d3d12" if cfg!(windows) => Some(GraphicsApi::Native(wgpu::Backends::DX12)),
        "gl" | "opengl" | "gles" => Some(GraphicsApi::Native(wgpu::Backends::GL)),
        "angle" | "dx11" | "d3d11" if all.contains(&GraphicsApi::Angle) => Some(GraphicsApi::Angle),
        _ => None,
    };
    // (the one asked for first, the others after it: a machine without it still starts)
    first.into_iter().chain(all.into_iter().filter(|b| Some(*b) != first)).collect()
}

/// A renderer in software on the processor (wgpu calls it a CPU adapter: WARP, llvmpipe,
/// SwiftShader).
fn is_software(info: &wgpu::AdapterInfo) -> bool {
    software_adapter(&info.name, info.device_type)
}

fn software_adapter(name: &str, device_type: wgpu::DeviceType) -> bool {
    device_type == wgpu::DeviceType::Cpu || name.contains("Microsoft Basic Render Driver") || name.contains("llvmpipe")
}

fn backend_instance(api: GraphicsApi) -> wgpu::Instance {
    let mut d = wgpu::InstanceDescriptor::new_without_display_handle();
    d.backend_options.gl.context_lock_timeout = Some(GL_CONTEXT_LOCK_TIMEOUT);
    d.backends = api.backends();
    if api == GraphicsApi::Angle {
        d.backend_options.gl.platform = wgpu::GlPlatform::Angle;
    }
    wgpu::Instance::new(d)
}

/// The renderer for a window: on `instance` if it can, else on the next graphics interface
/// and adapter that can (`instance` then becomes that one's). Laptops with a GeForce GT or
/// GTX beside the processor's graphics listed a Vulkan adapter whose device then could not
/// be opened (an old driver, the switchable graphics), or whose opening took wgpu down: the
/// game and the launcher ended before their window showed anything. Now each adapter that
/// can show the window is tried in turn - the card, then the processor's graphics - on
/// Vulkan, DirectX 12 and OpenGL, and only when none opens is the game given up, saying so.
pub(crate) fn window_renderer(
    instance: &mut wgpu::Instance,
    window: &std::sync::Arc<winit::window::Window>,
    options: omsi_render::RenderOptions,
) -> Result<Renderer> {
    let mut failures: Vec<String> = Vec::new();
    // The instance made for the settings' interface first, then every interface in turn -
    // each made only when the ones before it could not draw. Made all at once, a Windows
    // machine drawing on DirectX 12 loaded the Vulkan loader and its layers (Optimus,
    // overlays) and made an OpenGL context for nothing, and when those were dropped right
    // after the renderer was made, the game and the launcher went down ("[Vulkan Loader]
    // vkDestroyFramebuffer: Invalid device", #1058, #1044) or hung (#746).
    // A software renderer (DirectX 12's "Microsoft Basic Render Driver", llvmpipe) only
    // when no graphics chip opens on any interface: an Intel HD 2500 has no DirectX 12
    // driver, DirectX 12 listed that renderer alone, and the game "started" on it at a
    // frame every few seconds instead of on the chip's OpenGL (#770).
    let mut made: Vec<(Option<GraphicsApi>, wgpu::Instance)> = vec![(None, instance.clone())];
    let mut order = backend_order().into_iter();
    let mut no_surface: Vec<usize> = Vec::new();
    for software_round in [false, true] {
        let mut k = 0;
        loop {
            if k == made.len() {
                let Some(b) = order.next() else { break };
                log::info!("graphics: trying {b:?}");
                made.push((Some(b), backend_instance(b)));
            }
            let (b, inst) = made[k].clone();
            k += 1;
            if no_surface.contains(&(k - 1)) {
                continue;
            }
            let surface = match inst.create_surface(window.clone()) {
                Ok(s) => s,
                Err(e) => {
                    failures.push(format!("{}: no surface ({e})", b.map(|b| format!("{b:?}")).unwrap_or_else(|| "first choice".into())));
                    no_surface.push(k - 1);
                    continue;
                }
            };
            for adapter in Renderer::adapters_for(&inst, &surface) {
                let info = adapter.get_info();
                if is_software(&info) != software_round {
                    continue;
                }
                let what = format!("{} ({:?})", info.name, info.backend);
                if failures.iter().any(|f| f.starts_with(&what)) {
                    continue;
                }
                if software_round {
                    log::warn!("graphics: no graphics chip could be opened; trying the software renderer {what} (a few frames a second at best)");
                }
                match omsi_render::catch(|| pollster::block_on(Renderer::new_on(adapter, Some(&surface), None, options))) {
                    Some(Ok(r)) => {
                        if !failures.is_empty() {
                            log::warn!("graphics: drawing on {what}; before it {}", failures.join("; "));
                        }
                        drop(surface);
                        *instance = inst;
                        return Ok(r);
                    }
                    Some(Err(e)) => {
                        log::warn!("graphics: {what} could not be opened: {e:#}");
                        failures.push(format!("{what}: {e:#}"));
                    }
                    None => {
                        log::warn!("graphics: {what} failed while being opened");
                        failures.push(format!("{what}: failed while being opened"));
                    }
                }
            }
        }
    }
    Err(anyhow!("no graphics device could be opened ({}); updating the graphics driver usually helps", if failures.is_empty() { "no adapter can show a window".to_string() } else { failures.join("; ") }))
}

/// Tell the player why the game cannot go on, also where there is no window yet (a message
/// box on Windows; the log and the terminal elsewhere).
pub(crate) fn fatal_message(text: &str) {
    log::error!("{text}");
    eprintln!("openOMSI: {text}");
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (t, c) = (wide(text), wide("openOMSI"));
        unsafe {
            MessageBoxW(None, PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), MB_OK | MB_ICONERROR);
        }
    }
}

/// The commit this binary was built from (see `build.rs`), so a log or a screenshot says
/// which version is running.
pub const BUILD: &str = env!("OMSI_BUILD");

/// The release version, `MAJOR.MINOR.COMMIT` (see `build.rs` and docs/VERSIONING.md).
pub const VERSION: &str = env!("OPENOMSI_VERSION");

/// A window of `w` x `h` points made to fit the screen it opens on, and placed in its
/// middle: 1600 x 900 points at 125 % are 2000 x 1125 pixels, wider than a 1920 screen, and
/// the window opened partly off it (#771). Where the system tells no screen (Wayland), the
/// size as asked and no place.
/// The game runs inside gamescope (a Steam Deck's Gaming Mode, a Steam Machine): one
/// window, shown over the whole screen.
pub(crate) fn under_gamescope() -> bool {
    std::env::var_os("GAMESCOPE_WAYLAND_DISPLAY").is_some() || std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_ascii_lowercase().contains("gamescope"))
}

/// The screen the game opens on: the one the launcher stands on (`OMSI_SCREEN_AT`, the
/// launcher's middle), else the main one (#1959: it always opened on the main screen).
pub(crate) fn home_monitor(event_loop: &winit::event_loop::ActiveEventLoop) -> Option<winit::monitor::MonitorHandle> {
    let at = omsi_cfg::flags::OMSI_SCREEN_AT.var().and_then(|v| {
        let (x, y) = v.split_once(',')?;
        Some((x.trim().parse::<i32>().ok()?, y.trim().parse::<i32>().ok()?))
    });
    at.and_then(|(x, y)| {
        event_loop.available_monitors().find(|m| {
            let (p, s) = (m.position(), m.size());
            x >= p.x && y >= p.y && x < p.x + s.width as i32 && y < p.y + s.height as i32
        })
    })
    .or_else(|| event_loop.primary_monitor())
    .or_else(|| event_loop.available_monitors().next())
}

pub(crate) fn fit_window(event_loop: &winit::event_loop::ActiveEventLoop, w: f64, h: f64) -> (winit::dpi::LogicalSize<f64>, Option<winit::dpi::PhysicalPosition<i32>>) {
    let Some(m) = home_monitor(event_loop) else {
        return (winit::dpi::LogicalSize::new(w, h), None);
    };
    let (screen, scale) = (m.size(), m.scale_factor().max(0.5));
    let ((lw, lh), (x, y)) = fit_rect((w, h), (screen.width as f64, screen.height as f64), scale);
    let at = winit::dpi::PhysicalPosition::new(m.position().x + x, m.position().y + y);
    (winit::dpi::LogicalSize::new(lw, lh), Some(at))
}

/// `want` points fitted into a screen of `screen` pixels at `scale` (the size kept to 90 %
/// of its width and 85 % of its height - a title bar and a task bar take some - with the
/// shape kept), and where it starts for the screen's middle (pixels).
pub(crate) fn fit_rect(want: (f64, f64), screen: (f64, f64), scale: f64) -> ((f64, f64), (i32, i32)) {
    let (sw, sh) = (screen.0 / scale, screen.1 / scale);
    let k = (sw * 0.9 / want.0).min(sh * 0.85 / want.1).min(1.0);
    let (w, h) = ((want.0 * k).round(), (want.1 * k).round());
    let x = ((screen.0 - w * scale) * 0.5).max(0.0) as i32;
    // (a little above the middle: the title bar sits over the window's top)
    let y = ((screen.1 - h * scale) * 0.4).max(0.0) as i32;
    ((w, h), (x, y))
}

/// The application icon for the window (Windows and Linux; macOS takes the bundle's).
pub(crate) fn window_icon() -> Option<winit::window::Icon> {
    static PNG: &[u8] = include_bytes!("../../../assets/icons/app/openomsi-256.png");
    let img = image::load_from_memory(PNG).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}

/// Enhanced graphics wanted (from the settings, `--enhanced`, or OMSI_ENHANCED=1).
pub(crate) static ENHANCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Enhanced+ (ray tracing) asked for by `--enhanced-plus` or OMSI_ENHANCED_PLUS=1 whatever
/// the settings say (see `Settings::render_options`).
pub(crate) static ENHANCED_PLUS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Vanilla graphics: the picture as OMSI 2 draws it (no Vanilla+ extras, see
/// `omsi_render::Lighting::classic`).
pub(crate) static CLASSIC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Volume of the AI vehicles and of the scenery's own sounds (OMSI's `sound_ai` and
/// `sound_scenery`), as the bits of an f32.
pub(crate) static SOUND_AI: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);
pub(crate) static SOUND_SCENERY: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

/// Edge of a mirror's picture (the `mirror_size` setting, OMSI's reflTexSize).
pub(crate) static MIRROR_SIZE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(256);

pub(crate) fn sound_gain(which: &std::sync::atomic::AtomicU32) -> f32 {
    f32::from_bits(which.load(std::sync::atomic::Ordering::Relaxed))
}

/// The `clouds` setting: clouds in the sky (off: a clear sky whatever the weather says).
pub(crate) static CLOUDS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// macOS's allocator keeps freed large blocks for reuse, and they count as the game's
/// memory: after loading and unloading tiles it held more than half a gigabyte of them, and
/// `malloc_zone_pressure_relief` does not give them back. `MallocLargeCache=0` (read when
/// the process starts) returns them at once, so the game starts itself again with it, in
/// the same process (exec keeps the pid the launcher knows). `OMSI_KEEP_ALLOCATOR=1` skips it.
#[cfg(target_os = "macos")]
pub(crate) fn restart_with_allocator_settings() {
    use std::os::unix::process::CommandExt;
    if std::env::var_os("MallocLargeCache").is_some()
        || omsi_cfg::flags::OMSI_KEEP_ALLOCATOR.is_set()
    {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let err = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env("MallocLargeCache", "0")
        .exec();
    // only comes back when the exec failed: go on as we are
    eprintln!("could not restart with MallocLargeCache=0: {err}");
}

/// A Windows GUI program has no console; when it was started from one (cmd, PowerShell)
/// the log and --help still belong there.
#[cfg(windows)]
pub(crate) fn attach_parent_console() {
    extern "system" {
        fn AttachConsole(process: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // fails harmlessly when there is no parent console (a double click, the launcher, whose
    // redirected log file stays the output)
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(test)]
mod own_key_tests {
    /// OMSI's own file as it comes has no key of the player's own.
    #[test]
    fn the_stock_file_has_none_of_the_players() {
        let root = std::path::Path::new("../../../OMSI 2 Original");
        if !root.join("Inputs/keyboard.cfg").exists() {
            return;
        }
        assert!(super::own_bindings(root, 0).is_empty());
        assert!(super::own_bindings(root, omsi_content::input::KEY_SHIFT).is_empty());
    }
}

#[cfg(test)]
mod window_tests {
    use super::software_adapter;

    /// The software renderers come after every graphics chip (#770).
    #[test]
    fn software_renderers_are_told_from_chips() {
        use wgpu::DeviceType::*;
        assert!(software_adapter("Microsoft Basic Render Driver", Cpu));
        assert!(software_adapter("Microsoft Basic Render Driver", Other));
        assert!(software_adapter("llvmpipe (LLVM 15.0.7, 256 bits)", Cpu));
        assert!(!software_adapter("Intel(R) HD Graphics 2500", IntegratedGpu));
        assert!(!software_adapter("NVIDIA GeForce RTX 3050 Laptop GPU", DiscreteGpu));
    }

    #[test]
    fn the_window_fits_a_small_screen_and_sits_in_its_middle() {
        // 1920 x 1200 at 125 %: 1600 x 900 points would be 2000 px wide
        let ((w, h), (x, y)) = super::fit_rect((1600.0, 900.0), (1920.0, 1200.0), 1.25);
        assert!(w * 1.25 <= 1920.0 * 0.9 + 1.0 && h * 1.25 <= 1200.0 * 0.85 + 1.0, "{w} x {h}");
        assert!(((w / h) - 16.0 / 9.0).abs() < 0.01);
        assert!((x as f64 - (1920.0 - w * 1.25) / 2.0).abs() <= 1.0);
        assert!(y > 0);
        // a big screen keeps the size asked for
        let ((w, h), _) = super::fit_rect((1600.0, 900.0), (3840.0, 2160.0), 1.5);
        assert_eq!((w, h), (1600.0, 900.0));
    }
}
