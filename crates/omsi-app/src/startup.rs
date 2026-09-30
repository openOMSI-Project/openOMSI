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
    if std::io::IsTerminal::is_terminal(&std::io::stderr()) || omsi_cfg::env::var_os("OMSI_BACKGROUND").is_some() {
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
    if let Some(d) = omsi_cfg::env::var_os("OMSI_CONTENT") {
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
    if (cand.exists() || std::fs::create_dir_all(&cand).is_ok()) && omsi_cfg::is_writable(&cand) {
        Some(cand)
    } else {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
        let fallback = PathBuf::from(home).join(".openomsi").join("content");
        let _ = omsi_cfg::ensure_content_layout(&fallback);
        Some(fallback)
    }
}

/// Where the last working installation was remembered.
pub(crate) fn root_memo() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".openomsi-root"))
}

/// Find the OMSI 2 installation without being told where it is.
pub(crate) fn find_root() -> Option<PathBuf> {
    let mut first: Vec<PathBuf> = Vec::new();
    if let Some(p) = omsi_cfg::env::var_os("OMSI_ROOT").map(PathBuf::from) {
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
    let mut last = None;
    for b in backend_order() {
        let instance = backend_instance(b);
        let adapters = pollster::block_on(instance.enumerate_adapters(b));
        if !adapters.is_empty() {
            log::info!("graphics: {:?} ({})", b, adapters.iter().map(|a| a.get_info().name).collect::<Vec<_>>().join(", "));
            return instance;
        }
        log::info!("graphics: no {b:?} adapter here");
        last = Some(instance);
    }
    last.unwrap_or_else(|| wgpu::Instance::new(descriptor))
}

/// The graphics interfaces in the order they are tried: Metal on a Mac; on Windows DirectX
/// 12 first (the Windows drivers' best-kept path: on Vulkan they reset the device -
/// "the graphics device was lost" - far more often), then Vulkan, then OpenGL for a card
/// without either (a GeForce GT 530); elsewhere Vulkan, then OpenGL. Settings → Graphics API
/// (`graphics_api`) or OMSI_BACKEND=vulkan|dx12|gl puts one first: a driver whose Vulkan
/// misbehaves is got round.
pub(crate) fn backend_order() -> Vec<wgpu::Backends> {
    if cfg!(target_os = "macos") {
        return vec![wgpu::Backends::METAL];
    }
    let settings = crate::settings::Settings::load();
    let wanted = if settings.vr_requested() {
        "dx12".to_owned()
    } else {
        omsi_cfg::env::var("OMSI_BACKEND").ok().unwrap_or(settings.graphics_api)
    };
    let all: Vec<wgpu::Backends> = if cfg!(windows) {
        vec![wgpu::Backends::DX12, wgpu::Backends::VULKAN, wgpu::Backends::GL]
    } else {
        vec![wgpu::Backends::VULKAN, wgpu::Backends::GL]
    };
    let first = match wanted.trim().to_ascii_lowercase().as_str() {
        "vulkan" => Some(wgpu::Backends::VULKAN),
        "dx12" | "directx" | "d3d12" if cfg!(windows) => Some(wgpu::Backends::DX12),
        "gl" | "opengl" | "gles" => Some(wgpu::Backends::GL),
        _ => None,
    };
    // (the one asked for first, the others after it: a machine without it still starts)
    first.into_iter().chain(all.into_iter().filter(|b| Some(*b) != first)).collect()
}

fn backend_instance(b: wgpu::Backends) -> wgpu::Instance {
    let mut d = wgpu::InstanceDescriptor::new_without_display_handle();
    d.backends = b;
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
    // the instance made for the settings' interface first, then every interface in turn
    let mut candidates: Vec<(Option<wgpu::Backends>, wgpu::Instance)> = vec![(None, instance.clone())];
    for b in backend_order() {
        candidates.push((Some(b), backend_instance(b)));
    }
    for (b, inst) in candidates {
        let surface = match inst.create_surface(window.clone()) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{}: no surface ({e})", b.map(|b| format!("{b:?}")).unwrap_or_else(|| "first choice".into())));
                continue;
            }
        };
        for adapter in Renderer::adapters_for(&inst, &surface) {
            let info = adapter.get_info();
            let what = format!("{} ({:?})", info.name, info.backend);
            if failures.iter().any(|f| f.starts_with(&what)) {
                continue;
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

/// The application icon for the window (Windows and Linux; macOS takes the bundle's).
pub(crate) fn window_icon() -> Option<winit::window::Icon> {
    static PNG: &[u8] = include_bytes!("../../../assets/icons/app/openomsi-256.png");
    let img = image::load_from_memory(PNG).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}

/// Enhanced graphics wanted (from the settings, `--enhanced`, or OMSI_ENHANCED=1).
pub(crate) static ENHANCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
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
        || omsi_cfg::env::var_os("OMSI_KEEP_ALLOCATOR").is_some()
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
