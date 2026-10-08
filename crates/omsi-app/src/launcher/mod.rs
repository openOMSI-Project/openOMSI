//! The launcher: where a duty is put together (bus, map, line and tour, time, weather),
//! the driver's profile, the settings and key bindings, the running games and the mods.
//!
//! It is a window of the game binary itself, drawn with wgpu: the chosen bus stands in a
//! picture drawn by the game's renderer (see `showroom`) whenever it changes, and the
//! interface - flat and dark, every control custom - is drawn with `omsi-ui` straight
//! onto the window. The data side (content
//! lists, timetables, profiles, installs, running games) is `omsi-launcher-core`, the same
//! functions `omsi-launcher --cli` offers a terminal.

pub(crate) mod drive;
pub(crate) mod mapview;
pub mod mobile;
pub mod phone;
mod multiplayer;
pub(crate) mod pages;
mod showroom;
mod season;
mod state;
#[cfg_attr(not(target_os = "android"), allow(unused_imports))]
pub(crate) use state::crash_of;
mod theme;
mod timetable;
mod ui;
mod update;

use glam::Vec2;
use omsi_launcher_lib as core;
use omsi_render::{Renderer, SurfaceState};
use omsi_ui::paint::Align;
use omsi_ui::{Draw, Rect, Weight};
use std::sync::Arc;
use std::time::Instant;
use theme::*;
use ui::{Key, Ui};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Drive,
    Multiplayer,
    Profile,
    Settings,
    Controls,
    Sessions,
    Mods,
    Tutorials,
    Timetable,
    Setup,
}

const PAGES: [(Page, &str, &str); 10] = [
    (Page::Drive, "Drive", "directions_bus"),
    (Page::Multiplayer, "Multiplayer", "groups"),
    (Page::Profile, "Profile", "badge"),
    (Page::Settings, "Settings", "tune"),
    (Page::Controls, "Controls", "keyboard"),
    (Page::Sessions, "Sessions", "sports_esports"),
    (Page::Mods, "Mods", "extension"),
    (Page::Tutorials, "Tutorials", "help"),
    (Page::Timetable, "Timetable", "schedule"),
    (Page::Setup, "Setup", "folder_open"),
];

#[cfg(not(target_os = "android"))]
type Clipboard = arboard::Clipboard;

/// A phone: text copied in the launcher can be pasted in it (the system's clipboard is
/// Java's).
#[cfg(target_os = "android")]
struct Clipboard(String);

#[cfg(target_os = "android")]
impl Clipboard {
    fn new() -> Result<Clipboard, ()> {
        Ok(Clipboard(String::new()))
    }
    fn get_text(&mut self) -> Result<String, ()> {
        Ok(self.0.clone())
    }
    fn set_text(&mut self, t: String) -> Result<(), ()> {
        self.0 = t;
        Ok(())
    }
}

/// Width of the left rail (points).
pub const RAIL_W: f32 = 236.0;

/// How often the launcher made its device again after losing it (see `recover_device`).
static LAUNCHER_RECOVERIES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub struct Launcher {
    instance: wgpu::Instance,
    window: Option<Arc<Window>>,
    surface: Option<SurfaceState<'static>>,
    renderer: Option<Renderer>,
    gpu: Option<omsi_ui::Gpu>,
    ui: Ui,
    state: state::State,
    showroom: showroom::Showroom,
    page: Page,
    page_anim: f32,
    pub drive: drive::DriveView,
    /// The launcher made for a phone (see `phone`).
    pub phone: phone::PhoneView,
    pub pages: pages::PagesView,
    pub mp: multiplayer::MultiplayerView,
    /// Server icons in the interface pipeline (by server address), and those decoded but
    /// not yet uploaded.
    pub icons: std::collections::HashMap<String, usize>,
    pub icons_pending: Vec<(String, image::RgbaImage)>,
    last: Instant,
    modifiers: ui::Modifiers,
    /// Right or left drag over the showroom.
    dragging: Option<Vec2>,
    clipboard: Option<Clipboard>,
    exit_after: Option<f32>,
    shot: Option<(f32, std::path::PathBuf)>,
    started: Instant,
    /// Until the first frame is on the screen: the preview loads no bus before it.
    first_frame: bool,
    /// `OMSI_LAUNCHER_INPUT="t=2 click 400,300; t=3 type Bauern; t=4 key Enter; t=5 shot a.png;
    /// t=6 wheel -3; t=7 move 900,400"`: the window worked by a script (logical pixels).
    script: Vec<(f32, String)>,
    release_next: bool,
    /// Where the bus preview is this frame, its texture in the interface pipeline, and the
    /// showroom picture it was bound to.
    preview_rect: Option<Rect>,
    preview_tex: Option<usize>,
    preview_gen: u64,
    /// The chosen map's picture (see `mapview`): where it is this frame and its texture.
    pub mapview: mapview::MapView,
    map_rect: Option<Rect>,
    map_tex: Option<usize>,
    map_gen: u64,
    /// The window has the keyboard / is hidden: without focus it is drawn ten times a
    /// second, hidden not at all (a game started from it is being played).
    focused: bool,
    occluded: bool,
    /// The player came back to the launcher's window while a game runs (clicked it, Alt+Tab):
    /// it is drawn and answers again until the game has the focus back. Before, it stood
    /// still the whole game long, and the session code could not be copied (#825).
    awake_in_game: bool,
    /// The last mouse or key event (an idle launcher draws less often: it kept the GPU busy
    /// at the screen's rate doing nothing).
    last_input: Instant,
    /// A phone's fingers, its storage browser, how far the page is scrolled (and can be),
    /// and whether the on-screen keyboard is up (see `mobile`).
    fingers: mobile::Fingers,
    pub browser: Option<mobile::Browser>,
    page_scroll: f32,
    page_max: f32,
    ime: bool,
    /// Updates from the GitHub releases (see `crate::updater`, `update.rs`).
    pub update: crate::updater::Updater,
    #[cfg(not(target_os = "android"))]
    discord: Option<crate::discord::Discord>,
    #[cfg(not(target_os = "android"))]
    discord_next_try: Instant,
    /// Background drop of the graphics device given up while a game runs (see `frame`).
    /// Joined before a new device is opened so the two do not meet on the card.
    gpu_rest_drop: Option<std::thread::JoinHandle<()>>,
}

/// Run the launcher window until it is closed.
pub fn run(instance: wgpu::Instance) -> anyhow::Result<()> {
    let event_loop = EventLoop::new()?;
    let mut app = Launcher::new(instance);
    event_loop.run_app(&mut app)?;
    Ok(())
}

impl Launcher {
    /// The launcher, not yet in a window (that comes with `resumed`).
    pub fn new(instance: wgpu::Instance) -> Launcher {
    let started = Instant::now();
    core::cleanup();
    let mut app = Launcher {
        instance,
        window: None,
        surface: None,
        renderer: None,
        gpu: None,
        ui: Ui::new(),
        state: state::State::new(),
        showroom: showroom::Showroom::new(),
        page: Page::Drive,
        page_anim: 1.0,
        drive: drive::DriveView::default(),
        phone: phone::PhoneView::default(),
        pages: pages::PagesView::default(),
        mp: multiplayer::MultiplayerView::default(),
        icons: Default::default(),
        icons_pending: Vec::new(),
        last: Instant::now(),
        modifiers: ui::Modifiers::default(),
        dragging: None,
        clipboard: Clipboard::new().ok(),
        // OMSI_LAUNCHER_EXIT=secs, OMSI_LAUNCHER_SHOT=secs:file.png, OMSI_LAUNCHER_PAGE=mods:
        // looking at the window without a person at it
        exit_after: omsi_cfg::flags::OMSI_LAUNCHER_EXIT.parse(),
        shot: omsi_cfg::flags::OMSI_LAUNCHER_SHOT.var().and_then(|v| v.split_once(':').map(|(t, f)| (t.parse().unwrap_or(5.0), std::path::PathBuf::from(f)))),
        started,
        first_frame: true,
        script: omsi_cfg::flags::OMSI_LAUNCHER_INPUT.var()
            .map(|v| {
                v.split(';')
                    .filter_map(|c| {
                        let c = c.trim();
                        let (t, rest) = c.strip_prefix("t=")?.split_once(' ')?;
                        Some((t.parse().ok()?, rest.trim().to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        release_next: false,
        preview_rect: None,
        preview_tex: None,
        preview_gen: 0,
        mapview: mapview::MapView::new(),
        map_rect: None,
        map_tex: None,
        map_gen: 0,
        focused: true,
        occluded: false,
        awake_in_game: false,
        last_input: Instant::now(),
        fingers: Default::default(),
        browser: None,
        page_scroll: 0.0,
        page_max: 0.0,
        ime: false,
        update: Default::default(),
        #[cfg(not(target_os = "android"))]
        discord: None,
        #[cfg(not(target_os = "android"))]
        discord_next_try: Instant::now(),
        gpu_rest_drop: None,
    };
    // after an update: the files it set aside go, and the launcher says what happened
    #[cfg(not(target_os = "android"))]
    crate::updater::cleanup_after_update();
    if let Some(v) = crate::updater::just_updated() {
        log::info!("update: this start follows the update to {v}");
        app.update.updated = Some((v, Instant::now()));
    }
    // no original installation found anywhere: the launcher still opens, on Setup, and says
    // what it needs (only starting a session needs the game)
    if omsi_cfg::missing_original_essentials(std::path::Path::new(&app.state.config.root)).len() > 0 {
        app.page = Page::Setup;
        let why = state::root_problem(&app.state.config.root);
        app.state.set_status(why, true);
    }
    if let Some(p) = omsi_cfg::flags::OMSI_LAUNCHER_PAGE.var().map(str::to_string) {
        if let Some((pg, _, _)) = PAGES.iter().find(|(_, n, _)| n.eq_ignore_ascii_case(p.split(':').next().unwrap_or(""))) {
            app.page = *pg;
            // (the phone's tab for it)
            app.phone.tab = match pg {
                Page::Drive => phone::Tab::Play,
                Page::Multiplayer => phone::Tab::Online,
                Page::Mods => phone::Tab::Mods,
                other => {
                    app.phone.page = Some(*other);
                    phone::Tab::More
                }
            };
        }
        // (`OMSI_LAUNCHER_PAGE=more`, `=sheet-bus` …: the phone's More, or one of its sheets)
        match p.as_str() {
            "more" => app.phone.tab = phone::Tab::More,
            "sheet-map" => app.phone.sheet = Some(phone::Sheet::Map),
            "sheet-bus" => app.phone.sheet = Some(phone::Sheet::Bus),
            "sheet-duty" => app.phone.sheet = Some(phone::Sheet::Duty),
            "sheet-time" => app.phone.sheet = Some(phone::Sheet::Time),
            "sheet-livery" => app.phone.sheet = Some(phone::Sheet::Livery),
            _ => {}
        }
        if let Some(step) = p.split(':').nth(1).and_then(|s| s.parse::<usize>().ok()) {
            // (the Drive page's second part is which of its three steps: drive:2 the map)
            app.drive.tab = step.min(2);
            // (the Controls and Settings pages' second part is their tab: controls:1 the game
            // controllers, settings:3 Sound)
            app.pages.controls_tab = step;
            app.pages.settings_tab = step.min(pages::SETTINGS_TABS.len() - 1);
        }
    }
    app
    }

    /// Everything made on the graphics device goes with it: the interface's textures and every
    /// number kept for one of them - the bus preview, the map picture (and the map's own
    /// drawing), the servers' icons. A number kept over a device made anew pointed past the
    /// new device's textures, and the map was drawn with the font atlas instead: the Drive
    /// page's map full of the interface's words after a game (the launcher gives its device
    /// up while one runs) or a lost device.
    fn drop_gpu(&mut self) {
        self.gpu = None;
        self.preview_tex = None;
        self.showroom = showroom::Showroom::new();
        self.preview_gen = 0;
        self.map_tex = None;
        self.map_gen = 0;
        self.mapview.drop_gpu();
        self.icons.clear();
    }

    /// The window, its surface and the renderer, given up for the game (a phone plays in the
    /// launcher's window).
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub fn release_window(&mut self) -> Option<Arc<Window>> {
        self.pages.pads.release_io();
        self.surface = None;
        self.drop_gpu();
        if let Some(h) = self.gpu_rest_drop.take() {
            let _ = h.join();
        }
        self.renderer = None;
        self.ime = false;
        self.window.take()
    }

    /// Back from a game: the window again (the launcher draws into it from the next resume).
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub fn adopt_window(&mut self, window: Arc<Window>) {
        self.window = Some(window);
        self.surface = None;
        self.renderer = None;
        self.state.poll_now();
        self.state.load_profile();
    }

    /// The surface for the window the launcher has (created again after the app was in the
    /// background: a phone takes the window's surface away meanwhile).
    fn make_surface(&mut self) {
        let Some(window) = self.window.clone() else { return };
        if self.renderer.is_none() {
            // Finish dropping the device given up for a game before opening another one.
            if let Some(h) = self.gpu_rest_drop.take() {
                let _ = h.join();
            }
            let settings = crate::settings::Settings::load();
            let renderer = match crate::startup::window_renderer(&mut self.instance, &window, showroom_options(&settings)) {
                Ok(r) => r,
                Err(e) => {
                    crate::startup::fatal_message(&format!("openOMSI cannot draw on this computer: {e:#}"));
                    std::process::exit(1);
                }
            };
            self.gpu = Some(omsi_ui::Gpu::new(&renderer.device, renderer.format(), 4, self.ui.atlas.size));
            self.ui.atlas = omsi_ui::Atlas::new(self.ui.atlas.size);
            self.renderer = Some(renderer);
        }
        let Some(renderer) = self.renderer.as_ref() else { return };
        let size = window.inner_size();
        self.surface = SurfaceState::new_with(&self.instance, window.clone(), renderer, size.width.max(1), size.height.max(1), true).ok();
        self.last = Instant::now();
    }
}

impl ApplicationHandler for Launcher {
    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: winit::event::DeviceId, event: DeviceEvent) {
        if matches!(event, DeviceEvent::Added | DeviceEvent::Removed) {
            if let Some(io) = self.pages.pads.io.as_ref() {
                io.refresh();
            }
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        // (a phone: the app went to the background and its window's surface goes with it)
        self.surface = None;
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            if self.surface.is_none() {
                self.make_surface();
            }
            return;
        }
        // (`OMSI_LAUNCHER_SIZE=WxH`: another window size, for looking at the layout)
        let asked = omsi_cfg::flags::OMSI_LAUNCHER_SIZE.var().and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?))));
        let (fit, at) = match asked {
            Some((iw, ih)) => (winit::dpi::LogicalSize::new(iw, ih), None),
            None => crate::startup::fit_window(event_loop, 1440.0, 880.0),
        };
        let mut attrs = Window::default_attributes().with_title("openOMSI").with_window_icon(crate::startup::window_icon()).with_inner_size(fit);
        if !mobile::mobile() {
            // (no bigger than the window fitted to the screen: a small one at 150 % has less)
            attrs = attrs.with_min_inner_size(winit::dpi::LogicalSize::new(1080.0f64.min(fit.width), 680.0f64.min(fit.height)));
            if let Some(at) = at {
                attrs = attrs.with_position(at);
            }
        }
        if omsi_cfg::flags::OMSI_BACKGROUND.is_set() {
            attrs = attrs.with_active(false);
        }
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                crate::startup::fatal_message(&format!("openOMSI cannot open its window: {e}"));
                event_loop.exit();
                return;
            }
        };
        let settings = crate::settings::Settings::load();
        let renderer = match crate::startup::window_renderer(&mut self.instance, &window, showroom_options(&settings)) {
            Ok(r) => r,
            Err(e) => {
                crate::startup::fatal_message(&format!("openOMSI cannot draw on this computer: {e:#}"));
                event_loop.exit();
                return;
            }
        };
        let size = window.inner_size();
        let surface = match SurfaceState::new_with(&self.instance, window.clone(), &renderer, size.width, size.height, true) {
            Ok(s) => s,
            Err(e) => {
                crate::startup::fatal_message(&format!("openOMSI cannot draw into its window: {e:#}"));
                event_loop.exit();
                return;
            }
        };
        log::info!("launcher window {}x{} (scale {:.2}), adapter {}", size.width, size.height, window.scale_factor(), renderer.adapter_name);
        self.gpu = Some(omsi_ui::Gpu::new(&renderer.device, renderer.format(), 4, self.ui.atlas.size));
        self.window = Some(window);
        self.surface = Some(surface);
        self.renderer = Some(renderer);
        self.last = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let scale = self.ui_scale();
        if !matches!(event, WindowEvent::RedrawRequested) {
            self.last_input = Instant::now();
        }
        match event {
            WindowEvent::CloseRequested => {
                self.pages.pads.cancel_feedback_test();
                event_loop.exit();
            }
            WindowEvent::Touch(t) => self.touch(t, scale),
            WindowEvent::Focused(f) => self.set_focus(f),
            WindowEvent::Occluded(o) => {
                self.occluded = o;
                if o {
                    self.pages.pads.cancel_feedback_test();
                } else if self.renderer.is_none() {
                    // Back in view without a graphics device (a game just ended, or the
                    // window was covered while resting): draw again so the device is opened
                    // without waiting for the next slow occluded tick.
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::Resized(s) => {
                if let (Some(sf), Some(r)) = (self.surface.as_mut(), self.renderer.as_ref()) {
                    sf.resize(r, s.width, s.height);
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers.told(m.state());
                self.modifiers.apply(&mut self.ui.input);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = Vec2::new(position.x as f32, position.y as f32) / scale;
                if let Some(last) = self.dragging {
                    let d = p - last;
                    self.showroom.orbit(d.x, d.y);
                    self.dragging = Some(p);
                }
                self.ui.input.mouse = p;
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => {
                        if down {
                            self.ui.input.pressed = true;
                            // a drag on the preview turns the bus (the panels lie over it:
                            // what the mouse is over is theirs, not the bus's)
                            if !self.ui.over_ui && self.preview_rect.map(|r| r.contains(self.ui.input.mouse)).unwrap_or(false) {
                                self.dragging = Some(self.ui.input.mouse);
                            }
                        } else {
                            self.ui.input.released = true;
                            self.dragging = None;
                        }
                        self.ui.input.down = down;
                    }
                    MouseButton::Right => {
                        self.ui.input.right_down = down;
                        if down {
                            self.ui.input.right_pressed = true;
                            if self.preview_rect.map(|r| r.contains(self.ui.input.mouse)).unwrap_or(false) {
                                self.dragging = Some(self.ui.input.mouse);
                            }
                        } else {
                            self.dragging = None;
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let d = match delta {
                    MouseScrollDelta::LineDelta(x, y) => Vec2::new(x, y),
                    MouseScrollDelta::PixelDelta(p) => Vec2::new(p.x as f32, p.y as f32) / 40.0,
                };
                self.ui.input.wheel += d;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // (Shift, Ctrl, Alt from their keys where the window never says: Android)
                if let PhysicalKey::Code(code) = event.physical_key {
                    if self.modifiers.key(code, event.state == ElementState::Pressed) {
                        self.modifiers.apply(&mut self.ui.input);
                    }
                }
                if event.state != ElementState::Pressed {
                    return;
                }
                let cmd = self.modifiers.command();
                // a phone's back key: out of the storage browser, else like Escape
                if event.physical_key == PhysicalKey::Code(KeyCode::BrowserBack) {
                    if self.browser.is_some() {
                        self.browser = None;
                    } else {
                        self.ui.input.keys.push(Key::Escape);
                    }
                    return;
                }
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.ui.input.capture_key(code);
                    let k = match code {
                        KeyCode::ArrowLeft => Some(Key::Left),
                        KeyCode::ArrowRight => Some(Key::Right),
                        KeyCode::ArrowUp => Some(Key::Up),
                        KeyCode::ArrowDown => Some(Key::Down),
                        KeyCode::Home => Some(Key::Home),
                        KeyCode::End => Some(Key::End),
                        KeyCode::Backspace => Some(Key::Backspace),
                        KeyCode::Delete => Some(Key::Delete),
                        KeyCode::Enter | KeyCode::NumpadEnter => Some(Key::Enter),
                        KeyCode::Escape => Some(Key::Escape),
                        KeyCode::Tab => Some(Key::Tab),
                        KeyCode::KeyA if cmd => Some(Key::SelectAll),
                        KeyCode::KeyC if cmd => Some(Key::Copy),
                        KeyCode::KeyV if cmd => Some(Key::Paste),
                        KeyCode::KeyX if cmd => Some(Key::Cut),
                        _ => None,
                    };
                    if let Some(k) = k {
                        if k == Key::Paste {
                            self.ui.clipboard_in = self.clipboard.as_mut().and_then(|c| c.get_text().ok());
                        }
                        self.ui.input.keys.push(k);
                    }
                }
                if !cmd {
                    if let Some(t) = event.text.as_ref() {
                        self.ui.input.text.push_str(t);
                    }
                }
            }
            WindowEvent::DroppedFile(path) => {
                // a mod folder or supported archive dropped on the window is installed
                self.page = Page::Mods;
                self.state.install(path.to_string_lossy().to_string());
            }
            WindowEvent::HoveredFile(_) => self.pages.drop_hover = true,
            WindowEvent::HoveredFileCancelled => self.pages.drop_hover = false,
            WindowEvent::RedrawRequested => {
                self.frame(event_loop);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.check_exit(event_loop);
        // (a screenshot asked for by a script is drawn even when hidden, and so is the frame
        // that gives the graphics device up again when the game is back in front: the
        // game's window hides the launcher's then, and drawn nothing, it kept the device)
        // (likewise the frame that opens it again for a window brought forward)
        let resting = !mobile::mobile() && self.renderer.is_some() && self.state.in_game() && !self.awake();
        let waking = !mobile::mobile() && self.renderer.is_none() && self.state.in_game() && self.awake();
        // Device given up while a game runs: still need a redraw when the game ends so the
        // device is opened again (Occluded alone used to leave the resting picture forever).
        let resume_needed = !mobile::mobile() && self.renderer.is_none() && !self.state.in_game();
        let occluded = self.occluded && self.shot.is_none() && !resting && !waking && !resume_needed
            && !self.script.iter().any(|(_, c)| c.starts_with("shot"));
        let interval = if occluded {
            0.5
        } else if !self.focused && !omsi_cfg::flags::OMSI_BACKGROUND.is_set() {
            0.1
        } else if self.last_input.elapsed().as_secs_f32() > 3.0 && self.dragging.is_none() && self.script.is_empty() {
            // idle: 20 frames a second keep the preview and the progress bars moving
            0.05
        } else {
            0.0
        };
        let since = self.last.elapsed().as_secs_f32();
        if interval > 0.0 && since < interval {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(self.last + std::time::Duration::from_secs_f32(interval)));
            return;
        }
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
        if occluded {
            // nothing to draw: keep the data side going (polls, installs, the script)
            let dt = since.min(1.0);
            self.last = Instant::now();
            self.run_script();
            self.state.update(dt);
            #[cfg(not(target_os = "android"))]
            self.update_discord();
            self.update_tick(event_loop);
            self.check_exit(event_loop);
            // A game may have ended on this tick: wake so `frame` opens the device again.
            if !mobile::mobile() && self.renderer.is_none() && !self.state.in_game() {
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
        } else if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }
}

impl Launcher {
    /// Physical pixels per interface pixel: the screen's scale, times a zoom that makes the
    /// interface (laid out for a 1440 x 880 window) grow with a bigger window and shrink a
    /// little with a smaller one, so that it fills the window the same way at any size.
    fn ui_scale(&self) -> f32 {
        let Some(w) = self.window.as_ref() else { return 1.0 };
        let dpi = w.scale_factor() as f32;
        let s = w.inner_size();
        let (lw, lh) = (s.width as f32 / dpi, s.height as f32 / dpi);
        if mobile::mobile() {
            // a phone held across: the text at least at the system's own size - smaller, it
            // was hard to read and the buttons hard to hit (the pages scroll where the screen
            // is lower than they are, and lay themselves out for its width), a tablet larger
            return dpi * (lh / 400.0).clamp(1.0, 1.35);
        }
        // (the height counts a little less: on a wide, low screen - 2560 x 1080 - the text
        // stayed the size of a 1440 x 880 window's, tiny across the width; the pages scroll or
        // keep their width, see `draw_ui`)
        dpi * (lw / 1440.0).min(lh / 820.0).clamp(0.8, 2.2)
    }

    /// The graphics device was lost (#274: an AMD Radeon's DX12 driver gave up while the
    /// preview's textures went up, and the launcher then drew on the dead device, with
    /// thousands of errors a second): everything made on it goes, the other interface is
    /// taken - DirectX 12 and Vulkan for each other, remembered in the settings for the game
    /// as well - and the window is drawn again on a new device. Twice at most.
    fn recover_device(&mut self) -> bool {
        let Some(why) = self.renderer.as_ref().and_then(|r| r.device_lost()) else { return false };
        let tries = LAUNCHER_RECOVERIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let name = self.renderer.as_ref().map(|r| r.adapter_name.clone()).unwrap_or_default();
        let other = if name.contains("(Dx12)") { Some("vulkan") } else if name.contains("(Vulkan)") && cfg!(windows) { Some("dx12") } else if name.contains("(Vulkan)") { Some("gl") } else { None };
        log::error!("launcher: the graphics device was lost on {name} ({why}); {}", match (tries < 2, other) {
            (true, Some(o)) => format!("drawing on {o} from now on"),
            (true, None) => "drawing on a new device".to_string(),
            _ => "giving up".to_string(),
        });
        if tries >= 2 {
            return false;
        }
        if let Some(o) = other {
            std::env::set_var("OMSI_BACKEND", o);
            self.state.settings["graphics_api"] = serde_json::json!(o);
            self.state.settings_dirty = 0.3;
        }
        self.surface = None;
        self.drop_gpu();
        if let Some(h) = self.gpu_rest_drop.take() {
            let _ = h.join();
        }
        self.renderer = None;
        self.make_surface();
        true
    }

    #[cfg(not(target_os = "android"))]
    fn update_discord(&mut self) {
        let enabled = self.state.settings.get("discord_status").and_then(|v| v.as_bool()).unwrap_or(true);
        let launching = self.state.queued_launch.is_some()
            || self.state.launch_hold.is_some_and(|at| at.elapsed().as_secs_f32() < 15.0);
        let game_running = self.state.instances.iter().any(|i| i.running);
        let presence = crate::discord::Presence::for_launcher(enabled, launching, game_running);
        if presence.is_none() {
            if let Some(discord) = self.discord.as_ref() {
                discord.stop();
                if discord.is_finished() {
                    drop(self.discord.take());
                }
            }
            return;
        }
        if let Some(discord) = self.discord.as_ref() {
            if discord.is_stopping() {
                if !discord.is_finished() {
                    return;
                }
                drop(self.discord.take());
            }
        }
        if self.discord.is_none() {
            if !self.state.instances_ready() {
                return;
            }
            if Instant::now() < self.discord_next_try {
                return;
            }
            self.discord_next_try = Instant::now() + std::time::Duration::from_secs(5);
            let app_id = self.state.settings.get("discord_app_id").and_then(|v| v.as_str()).unwrap_or("");
            self.discord = crate::discord::Discord::start(app_id);
        }
        if let Some(discord) = self.discord.as_ref() {
            discord.set(presence);
        }
    }

    /// The window got or lost the keyboard (`WindowEvent::Focused`, or `focus 0/1` of a
    /// launcher script).
    fn set_focus(&mut self, f: bool) {
        self.focused = f;
        if !f {
            self.pages.pads.cancel_feedback_test();
            self.modifiers.release_keys();
            self.modifiers.apply(&mut self.ui.input);
        }
        // (only once the game is on its way: the launcher has the focus while Start is
        // pressed, and gives the device up then as before)
        self.awake_in_game = f && self.renderer.is_none() && self.state.in_game() && self.state.queued_launch.is_none();
    }

    /// Looked at while a game runs (see `awake_in_game`): drawn and answering as usual.
    fn awake(&self) -> bool {
        // (the player asked the launcher not to rest while a game runs: it is always awake)
        !self.rests() || (self.awake_in_game && self.focused && self.state.queued_launch.is_none())
    }

    /// Whether the launcher gives the graphics device up while a game runs (#834: the setting
    /// "The launcher rests while a game runs"; on by default).
    fn rests(&self) -> bool {
        self.state.settings.get("launcher_rest").and_then(|v| v.as_bool()).unwrap_or(true)
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        if self.recover_device() {
            return;
        }
        let desktop = !mobile::mobile() && self.window.is_some();
        let presence_released = {
            #[cfg(not(target_os = "android"))]
            {
                if self.state.queued_launch.is_some() {
                    if let Some(discord) = self.discord.as_ref() {
                        discord.stop();
                    }
                }
                self.discord.as_ref().is_none_or(|discord| discord.is_finished())
            }
            #[cfg(target_os = "android")]
            { true }
        };
        // (looked at while a game runs: drawn as usual, see `awake_in_game`)
        let awake = self.awake();
        if desktop && self.renderer.is_none() {
            if self.state.in_game() && !awake {
                // nothing is drawn while a game runs; what is clicked or typed meanwhile is not
                // done once the launcher is back (the first frame pressed Start again)
                let now = Instant::now();
                let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
                self.last = now;
                self.run_script();
                self.state.update(dt);
                self.ui.discard_input();
                #[cfg(not(target_os = "android"))]
                self.update_discord();
                // `update` may have seen the game end: open the device this frame instead of
                // returning with the resting picture until another redraw happens to notice.
                if self.state.in_game() && !self.awake() {
                    return;
                }
            }
            if self.state.in_game() {
                log::info!("launcher: its window is looked at while a game runs, the graphics device is opened again");
                // (what was clicked while it stood still is not done: the click that brought
                // it forward pressed whatever lay under it)
                self.ui.discard_input();
            } else {
                self.awake_in_game = false;
                log::info!("launcher: no game runs any more, the graphics device is opened again");
            }
            self.make_surface();
        }
        self.draw_frame(event_loop);
        // a game starts or runs: the frame just drawn says so and stays in the window, and the
        // graphics device is given up until the game ends (with it open, a game on an NVIDIA
        // card without Resizable BAR uploaded at 20 MB/s)
        // (asked again: a script's `focus 0` comes in the frame just drawn)
        if desktop && self.renderer.is_some() && self.state.in_game() && !self.awake()
            && (self.state.queued_launch.is_none() || presence_released)
        {
            log::info!("launcher: a game starts or runs, the graphics device is given up until it ends");
            self.surface = None;
            self.drop_gpu();
            // Dropping a wgpu device can wait on the GPU for seconds (especially while the
            // game is opening the same card). Do it off the UI thread so Windows does not
            // mark the launcher "Not Responding" over the resting picture.
            if let Some(r) = self.renderer.take() {
                if let Some(prev) = self.gpu_rest_drop.take() {
                    let _ = prev.join();
                }
                self.gpu_rest_drop = std::thread::Builder::new()
                    .name("launcher-gpu-rest".into())
                    .spawn(move || drop(r))
                    .ok();
            }
        }
        if let Some(d) = presence_released.then(|| self.state.queued_launch.take()).flatten() {
            // Finish the Discord handoff in the background before starting the child.
            #[cfg(not(target_os = "android"))]
            drop(self.discord.take());
            // The Controls page may still own the same DirectInput wheel non-exclusively.
            // Drop it before the child asks for exclusive foreground access for force feedback.
            self.pages.pads.release_io();
            self.state.spawn_launch(d);
        }
    }

    /// The launcher's picture, put on the window.
    fn draw_frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;
        let (Some(window), Some(_)) = (self.window.clone(), self.surface.as_ref()) else { return };
        let scale = self.ui_scale();
        let phys = window.inner_size();
        let (pw, ph) = (phys.width.max(1), phys.height.max(1));
        let size = Vec2::new(pw as f32, ph as f32) / scale;

        self.run_script();
        self.state.update(dt);
        #[cfg(not(target_os = "android"))]
        self.update_discord();
        self.update_tick(event_loop);
        // the preview shows the chosen bus in the chosen light
        let c = &self.state.choice;
        let look = showroom::Look { root: std::path::PathBuf::from(&self.state.config.root), map: c.map.clone(), bus: c.bus.clone(), paint: c.paint.clone(), weather: c.weather.clone(), time: c.time, date: c.date.clone() };
        // (not while a game runs: the launcher looked at meanwhile loads no bus onto the card;
        // nor before the first frame is shown, which the bus's loading would hold up)
        if !self.first_frame && !look.bus.is_empty() && !look.map.is_empty() && !self.state.in_game() {
            self.showroom.want(look);
        }
        if let Some(r) = self.renderer.as_ref() {
            self.showroom.update(r, dt);
        }

        // --- the interface
        self.preview_rect = None;
        self.map_rect = None;
        self.ui.begin(size, scale, dt);
        self.draw_ui();
        if mobile::mobile() {
            // what no list took of a finger's drag scrolls the page
            if !self.ui.wheel_taken() && self.browser.is_none() {
                self.page_scroll = (self.page_scroll - self.ui.input.wheel.y * 42.0).clamp(0.0, self.page_max);
            }
            // the on-screen keyboard while a text field has the focus
            let want = self.ui.focus.is_some();
            if want != self.ime {
                self.ime = want;
                window.set_ime_allowed(want);
            }
        }
        window.set_cursor(if self.dragging.is_some() { winit::window::CursorIcon::Grabbing } else { self.ui.cursor });
        if let Some(t) = self.ui.clipboard_out.take() {
            if let Some(c) = self.clipboard.as_mut() {
                let _ = c.set_text(t);
            }
        }
        let (layers, verts, ranges) = self.ui.finish();
        self.touch_frame();

        // --- to the GPU: the preview when it changed, then the interface onto the window
        let Some(renderer) = self.renderer.as_mut() else { return };
        if let Some(r) = self.preview_rect {
            let (w, h) = ((r.w * scale) as u32, (r.h * scale) as u32);
            if let (Some(view), Some(gpu)) = (self.showroom.preview(renderer, w, h), self.gpu.as_mut()) {
                if self.preview_gen != self.showroom.generation {
                    self.preview_gen = self.showroom.generation;
                    match self.preview_tex {
                        Some(id) => gpu.set_view(&renderer.device, id, &view, (w, h)),
                        None => self.preview_tex = Some(gpu.add_view(&renderer.device, &view, (w, h))),
                    }
                }
            }
        }
        // the map picture, drawn again when what it shows, where it looks or the zoom's own
        // thinness changed
        if self.map_rect.is_some() {
            if let Some(view) = self.mapview.picture(renderer) {
                if let Some(gpu) = self.gpu.as_mut() {
                    if self.map_gen != self.mapview.generation {
                        self.map_gen = self.mapview.generation;
                        let size = self.mapview.pixels();
                        match self.map_tex {
                            Some(id) => gpu.set_view(&renderer.device, id, &view, size),
                            None => self.map_tex = Some(gpu.add_view(&renderer.device, &view, size)),
                        }
                    }
                }
            }
        }
        // server icons decoded this frame go to the GPU (drawn from the next)
        for (addr, img) in std::mem::take(&mut self.icons_pending) {
            if let Some(gpu) = self.gpu.as_mut() {
                let (w, h) = img.dimensions();
                let tex = renderer.device.create_texture(&wgpu::TextureDescriptor { label: Some("server icon"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8UnormSrgb, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
                renderer.queue.write_texture(tex.as_image_copy(), &img, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) }, wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
                let view = tex.create_view(&Default::default());
                let id = gpu.add_view(&renderer.device, &view, (w, h));
                self.icons.insert(addr, id);
            }
        }
        let draws: Vec<Draw> = ranges.iter().enumerate().map(|(k, (r, tex))| Draw { buffer: 0, range: r.clone(), layer: k, texture: *tex }).collect();
        let bg = theme::backdrop();
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.upload(&renderer.device, &renderer.queue, 0, &verts);
            gpu.upload_atlas(&renderer.queue, &mut self.ui.atlas);
        }
        // OMSI_LAUNCHER_SHOT: the window's picture into a file (drawn into a texture of its
        // own, so a hidden window gives one too)
        let t = self.started.elapsed().as_secs_f32();
        if let Some((at, file)) = self.shot.clone() {
            if t >= at {
                self.shot = None;
                if let Some(img) = self.shot_image(pw, ph, &layers, &draws, bg) {
                    let _ = img.save(&file);
                    log::info!("launcher: picture written to {}", file.display());
                }
            }
        }
        let Some(renderer) = self.renderer.as_mut() else { return };
        let surface = self.surface.as_mut().unwrap();
        let frame = match surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                surface.resize(renderer, pw, ph);
                return;
            }
            _ => return,
        };
        let view = frame.texture.create_view(&Default::default());
        if let Some(gpu) = self.gpu.as_mut() {
            let mut enc = renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("launcher") });
            gpu.render(&renderer.device, &renderer.queue, &mut enc, &view, (pw, ph), Some(bg), &layers, &draws);
            renderer.queue.submit([enc.finish()]);
        }
        window.pre_present_notify();
        frame.present();
        if std::mem::take(&mut self.first_frame) {
            log::info!("launcher: first frame presented in {:.2} s", self.started.elapsed().as_secs_f64());
        }
        self.check_exit(event_loop);
    }

    fn check_exit(&mut self, event_loop: &ActiveEventLoop) {
        if self.exit_after.map(|e| self.started.elapsed().as_secs_f32() >= e).unwrap_or(false) {
            event_loop.exit();
        }
    }

    fn run_script(&mut self) {
        if self.release_next {
            self.ui.input.released = true;
            self.ui.input.down = false;
            self.release_next = false;
        }
        let t = self.started.elapsed().as_secs_f32();
        while let Some((at, cmd)) = self.script.first().cloned() {
            if at > t {
                break;
            }
            self.script.remove(0);
            log::info!("launcher input t={at}: {cmd}");
            let (verb, arg) = cmd.split_once(' ').unwrap_or((cmd.as_str(), ""));
            let xy = || {
                let mut it = arg.split(',').map(|v| v.trim().parse::<f32>().unwrap_or(0.0));
                Vec2::new(it.next().unwrap_or(0.0), it.next().unwrap_or(0.0))
            };
            match verb {
                "move" => self.ui.input.mouse = xy(),
                "click" => {
                    self.ui.input.mouse = xy();
                    self.ui.input.pressed = true;
                    self.ui.input.down = true;
                    self.release_next = true;
                }
                "wheel" => self.ui.input.wheel.y += arg.trim().parse::<f32>().unwrap_or(0.0),
                // a drag: down at a place, `move` with the button still held, `up` at the end
                "down" => {
                    self.ui.input.mouse = xy();
                    self.ui.input.pressed = true;
                    self.ui.input.down = true;
                    self.release_next = false;
                }
                "up" => {
                    self.ui.input.released = true;
                    self.ui.input.down = false;
                }
                "type" => self.ui.input.text.push_str(arg),
                "key" => {
                    let k = match arg.trim() {
                        "Enter" => Some(Key::Enter),
                        "Escape" => Some(Key::Escape),
                        "Backspace" => Some(Key::Backspace),
                        _ => None,
                    };
                    if let Some(k) = k {
                        self.ui.input.keys.push(k);
                    }
                }
                "shot" => self.shot = Some((0.0, std::path::PathBuf::from(arg.trim()))),
                // `focus 0` / `focus 1`: the window loses or gets the keyboard
                "focus" => self.set_focus(arg.trim() != "0"),
                "page" => {
                    if let Some((pg, _, _)) = PAGES.iter().find(|(_, n, _)| n.eq_ignore_ascii_case(arg.trim())) {
                        self.go(*pg);
                    }
                }
                _ => log::warn!("launcher input: what is '{cmd}'?"),
            }
        }
    }

    /// The window's picture drawn again into a texture and read back (for OMSI_LAUNCHER_SHOT).
    fn shot_image(&mut self, w: u32, h: u32, layers: &[omsi_ui::Layer], draws: &[Draw], bg: wgpu::Color) -> Option<image::RgbaImage> {
        let r = self.renderer.as_mut()?;
        let gpu = self.gpu.as_mut()?;
        let tex = r.device.create_texture(&wgpu::TextureDescriptor { label: Some("launcher shot"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: r.format(), usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
        let view = tex.create_view(&Default::default());
        let mut enc = r.device.create_command_encoder(&Default::default());
        gpu.render(&r.device, &r.queue, &mut enc, &view, (w, h), Some(bg), layers, draws);
        let stride = (w * 4).div_ceil(256) * 256;
        let buf = r.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (stride * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        enc.copy_texture_to_buffer(tex.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: None } }, wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
        r.queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        r.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let data = buf.slice(..).get_mapped_range();
        let bgra = matches!(r.format(), wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm);
        let mut img = image::RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let i = (y * stride + x * 4) as usize;
                let (r_, g, b) = if bgra { (data[i + 2], data[i + 1], data[i]) } else { (data[i], data[i + 1], data[i + 2]) };
                img.put_pixel(x, y, image::Rgba([r_, g, b, 255]));
            }
        }
        Some(img)
    }

    fn draw_ui(&mut self) {
        if self.page != Page::Controls || self.pages.controls_tab != 1 {
            self.pages.pads.cancel_feedback_test();
        }
        let size = self.ui.size;
        let mobile = mobile::mobile();
        // the storage browser (or the update dialog) lies over the page: the page sees no
        // finger meanwhile
        let dialog = self.update_dialog_open();
        let disconnected = !dialog && self.state.disconnected.is_some();
        let crash = !dialog && !disconnected && self.state.crash.is_some();
        let reset = !dialog && !crash && !disconnected && self.pages.confirm_reset;
        if self.browser.is_some() || dialog || crash || reset || disconnected {
            self.pages.pads.cancel_feedback_test();
        }
        let saved = (self.browser.is_some() || dialog || crash || reset || disconnected).then(|| {
            let i = self.ui.input.clone();
            self.ui.input.mouse = Vec2::new(-1e4, -1e4);
            self.ui.input.pressed = false;
            self.ui.input.released = false;
            self.ui.input.wheel = Vec2::ZERO;
            self.ui.input.keys.clear();
            self.ui.input.text.clear();
            i
        });
        // a phone: the launcher made for it, not the desktop's pages
        if mobile {
            if self.page == Page::Setup && !omsi_cfg::missing_original_essentials(std::path::Path::new(&self.state.config.root)).is_empty() && self.phone.page.is_none() {
                self.phone.tab = phone::Tab::More;
                self.phone.page = Some(Page::Setup);
            }
            phone::draw(self);
        } else {
        let rail_w = RAIL_W;
        self.page_anim = (self.page_anim + self.ui.dt / 0.15).min(1.0);
        // (no wider than a page reads well: on a wide screen the rest is margin, the page
        // in the middle - the panels stretched across 2000 px with their text at one end)
        let (margin, top) = if mobile { (36.0, 14.0) } else { (64.0, 28.0) };
        let avail = size.x - rail_w - margin;
        let w = avail.min(1760.0);
        let seen = size.y - top - 40.0;
        // (a phone: laid out for a taller screen, scrolled)
        let h = if mobile { seen.max(mobile::PAGE_H) } else { seen };
        self.page_max = (h - seen).max(0.0);
        self.page_scroll = self.page_scroll.clamp(0.0, self.page_max);
        let content = Rect::new(rail_w + margin * 0.5 + (avail - w) * 0.5, top - self.page_scroll, w, h);
        let e = 1.0 - (1.0 - self.page_anim).powi(3);
        let content = Rect::new(content.x + 8.0 * (1.0 - e), content.y, content.w, content.h);
        match self.page {
            Page::Drive => drive::draw(self, content),
            Page::Multiplayer => multiplayer::draw(self, content),
            Page::Profile => pages::profile(self, content),
            Page::Settings => pages::settings(self, content),
            Page::Controls => pages::controls(self, content),
            Page::Sessions => pages::sessions(self, content),
            Page::Mods => pages::mods(self, content),
            Page::Tutorials => pages::tutorials(self, content),
            Page::Timetable => timetable::draw(self, content),
            Page::Setup => pages::setup(self, content),
        }
        // the rail over the page (a scrolled page passes under it)
        self.rail();
        self.status_bar();
        }
        self.draw_updated_notice();
        if let Some(i) = saved {
            self.ui.input = i;
            if dialog {
                self.draw_update_dialog();
            } else if disconnected {
                self.draw_disconnect_dialog();
            } else if crash {
                self.draw_crash_dialog();
            } else if reset {
                pages::reset_dialog(self);
            } else {
                self.draw_browser();
            }
        }
        // a game starts: the last picture before the launcher gives its graphics device up
        // (see `frame`), which stays in the window until the game ends
        if !mobile && self.state.in_game() && !self.awake() {
            self.draw_game_banner();
        }
    }

    /// Over the launcher's last picture while a game runs: why the launcher does not move.
    fn draw_game_banner(&mut self) {
        let size = self.ui.size;
        let full = Rect::new(0.0, 0.0, size.x, size.y);
        self.ui.solid(full);
        self.ui.p().rect(full, SHADE);
        let text = "The launcher rests while you drive, so that the game has the graphics card to itself. It is back as soon as the game ends.";
        let w = (size.x - 48.0).min(520.0);
        let th = self.ui.paragraph_height(text, w - 48.0, 13.0, Weight::Regular);
        let h = 80.0 + th;
        let r = Rect::new((size.x - w) * 0.5, (size.y - h) * 0.5, w, h);
        self.ui.panel(r);
        let inner = Rect::new(r.x + 24.0, r.y + 20.0, r.w - 48.0, r.h - 40.0);
        self.ui.icon("directions_bus", Vec2::new(inner.x + 14.0, inner.y + 14.0), 26.0, ACCENT);
        self.ui.text_in("The game is running", Rect::new(inner.x + 38.0, inner.y, inner.w - 38.0, 28.0), 18.0, Weight::Bold, TEXT, Align::Left);
        self.ui.paragraph(text, Vec2::new(inner.x, inner.y + 40.0), inner.w, 13.0, Weight::Regular, TEXT_DIM);
    }

    /// The bus preview in `r`: the game's picture of it, or a word while it loads. The mouse
    /// dragged on it turns the bus, the wheel zooms.
    pub fn preview(&mut self, r: Rect) {
        self.preview_rect = Some(r);
        self.ui.solid(r);
        self.ui.p().rounded(r, RADIUS, FIELD);
        match (self.preview_tex, self.showroom.has_picture()) {
            (Some(tex), true) => self.ui.image(r, tex, RADIUS),
            _ => {
                let t = if self.showroom.error.is_some() { "No preview" } else { "Loading…" };
                self.ui.text_in(t, r, 13.0, Weight::Regular, TEXT_FAINT, Align::Center);
            }
        }
        if self.showroom.busy && self.showroom.has_picture() {
            let c = Vec2::new(r.right() - 16.0, r.y + 16.0);
            let a = self.ui.time * 5.0;
            self.ui.p().arc(c, 6.0, 8.0, a, a + 4.2, TEXT_SOFT);
        }
        if self.ui.hover(r) && self.ui.input.wheel.y.abs() > 0.0 {
            self.showroom.zoom_by((1.0 - self.ui.input.wheel.y * 0.08).clamp(0.8, 1.25));
        }
        if self.ui.hover(r) {
            self.ui.cursor = winit::window::CursorIcon::Grab;
        }
    }

    /// The chosen map across `r`, the whole page behind the panels: every road the map has,
    /// its entry points and the chosen trip's route, read from the tile files (see
    /// `mapview`), or a word while it is read. `map_interact` gives it the mouse afterwards.
    pub fn map_background(&mut self, r: Rect) {
        self.map_rect = Some(r);
        let status = self.mapview.status();
        match (self.map_tex, status.is_empty()) {
            (Some(tex), true) => self.ui.image(r, tex, RADIUS),
            _ => {
                self.ui.solid(r);
                self.ui.p().rounded(r, RADIUS, BACKDROP);
                let t = if status.is_empty() { "Loading…" } else { status };
                self.ui.text_in(t, Rect::new(r.x, r.y + r.h * 0.5 - 12.0, r.w, 24.0), 13.5, Weight::Regular, TEXT_FAINT, Align::Center);
            }
        }
        if self.mapview.busy() {
            // (out of the way of the panels: the map is being read, the page is not)
            let c = Vec2::new(r.right() - 26.0, r.y + r.h - 26.0);
            let a = self.ui.time * 5.0;
            self.ui.p().arc(c, 6.0, 8.0, a, a + 4.2, TEXT_SOFT);
        }
    }

    /// The mouse over the map: what it drags, where it zooms, and the entry point a click
    /// takes. Called once the page's panels are drawn - they have the first claim on it.
    pub fn map_interact(&mut self, r: Rect, window: Rect) {
        let p = mapview::Pointer {
            at: self.ui.input.mouse,
            pressed: self.ui.input.pressed,
            released: self.ui.input.released,
            down: self.ui.input.down,
            wheel: self.ui.input.wheel.y,
            blocked: self.ui.over_ui || !r.contains(self.ui.input.mouse),
        };
        self.mapview.think(r, window, self.ui.scale, p);
        if let Some(i) = self.mapview.take_clicked() {
            if self.state.choice.entry != i as i32 {
                log::info!("launcher map: entry point {} of the map's list taken from the map", i + 1);
                self.state.choice.entry = i as i32;
                self.state.touched();
            }
        }
    }

    /// The bus across `r`, the whole page behind the panels. `focus` is where the panels end,
    /// as a share of the window: the showroom frames the bus in what is left of it.
    pub fn preview_full(&mut self, r: Rect, focus: f32) {
        self.preview_rect = Some(r);
        self.showroom.focus_x = focus.clamp(0.2, 0.95);
        match (self.preview_tex, self.showroom.has_picture()) {
            (Some(tex), true) => self.ui.image(r, tex, RADIUS),
            _ => {
                self.ui.solid(r);
                self.ui.p().rounded(r, RADIUS, FIELD);
                let t = if self.showroom.error.is_some() { "No preview" } else { "Loading…" };
                self.ui.text_in(t, Rect::new(r.x, r.y + r.h * 0.5 - 12.0, r.w, 24.0), 13.0, Weight::Regular, TEXT_FAINT, Align::Center);
            }
        }
    }

    /// The wheel and the cursor over the showroom, once the panels have had the mouse.
    pub fn showroom_pointer(&mut self, r: Rect) {
        if self.ui.over_ui || !r.contains(self.ui.input.mouse) {
            return;
        }
        if self.ui.input.wheel.y.abs() > 0.0 {
            self.showroom.zoom_by((1.0 - self.ui.input.wheel.y * 0.08).clamp(0.8, 1.25));
        }
        self.ui.cursor = winit::window::CursorIcon::Grab;
    }

    pub fn go(&mut self, p: Page) {
        if self.page != p {
            self.page = p;
            self.page_anim = 0.0;
            self.page_scroll = 0.0;
            self.phone.page = match p {
                Page::Drive => { self.phone.tab = phone::Tab::Play; None }
                Page::Multiplayer => { self.phone.tab = phone::Tab::Online; None }
                Page::Mods => { self.phone.tab = phone::Tab::Mods; None }
                other => { self.phone.tab = phone::Tab::More; Some(other) }
            };
            match p {
                Page::Profile => self.state.load_profile(),
                Page::Mods => self.state.load_mods(),
                Page::Sessions => self.state.poll_now(),
                _ => {}
            }
        }
    }

    fn rail(&mut self) {
        let size = self.ui.size;
        let rail = Rect::new(0.0, 0.0, RAIL_W, size.y);
        self.ui.solid(rail);
        self.ui.p().rect(rail, RAIL);
        self.ui.p().rect(Rect::new(RAIL_W - 1.0, 0.0, 1.0, size.y), EDGE);
        self.ui.text("openOMSI", Vec2::new(24.0, 46.0), 20.0, Weight::Bold, TEXT, Align::Left);
        self.ui.text(crate::startup::VERSION, Vec2::new(24.0, 64.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
        let mut y = 96.0;
        let running = self.state.instances.iter().filter(|i| i.running).count();
        let jobs = self.state.jobs.iter().filter(|j| j.finished.is_none()).count();
        // the pages are the stops of a line: the one shown is where the bus stands
        let stop_x = 32.0;
        let last = y + 42.0 * (PAGES.len() - 1) as f32;
        self.ui.p().rect(Rect::new(stop_x - 1.0, y + 19.0, 2.0, last - y), TRACK);
        for (p, name, icon) in PAGES {
            let r = Rect::new(12.0, y, RAIL_W - 24.0, 38.0);
            let id = ui::id_of(&format!("nav-{name}"));
            let (h, _, clicked) = self.ui.interact(id, r);
            if clicked {
                self.go(p);
            }
            let sel = self.page == p;
            let on = self.ui.anim(id ^ 5, if sel { 1.0 } else { 0.0 }, 0.09);
            let over = self.ui.anim(id ^ 6, if h { 1.0 } else { 0.0 }, 0.06);
            if over > 0.01 && !sel {
                self.ui.p().rounded(r, 19.0, HOVER.alpha(0.55 * over));
            }
            let stop = Vec2::new(stop_x, r.center().y);
            self.ui.p().circle(stop, 14.0 + on, TRACK.mix(TEXT_DIM, over).mix(ACCENT, on));
            self.ui.p().circle(stop, 12.5 - 12.5 * on, RAIL);
            let c = if sel { TEXT } else if h { TEXT_SOFT } else { TEXT_DIM };
            self.ui.icon(icon, stop, 16.0, c.mix(ON_ACCENT, on));
            self.ui.text_in(name, Rect::new(r.x + 46.0, r.y, r.w - 76.0, r.h), 13.5, if sel { Weight::Bold } else { Weight::Regular }, c, Align::Left);
            let count = match p {
                Page::Sessions => running,
                Page::Mods => jobs,
                _ => 0,
            };
            if count > 0 {
                self.ui.text_in(&count.to_string(), Rect::new(r.right() - 30.0, r.y, 20.0, r.h), 12.0, Weight::Bold, if p == Page::Sessions { OK } else { ACCENT }, Align::Right);
            }
            y += 42.0;
        }
        // the driver, quietly at the bottom
        let card = Rect::new(12.0, size.y - 64.0, RAIL_W - 24.0, 48.0);
        let id = ui::id_of("rail-profile");
        let (h, _, clicked) = self.ui.interact(id, card);
        if clicked {
            self.go(Page::Profile);
        }
        self.ui.p().rect(Rect::new(12.0, card.y - 9.0, RAIL_W - 24.0, 1.0), EDGE);
        if h {
            self.ui.p().rounded(card, RADIUS, HOVER);
        }
        let (level, name) = match &self.state.profile {
            Some(p) => (p.level, p.name.clone()),
            None => (1, self.state.config.profile.clone()),
        };
        self.ui.icon("account_circle", Vec2::new(card.x + 22.0, card.center().y), 24.0, TEXT_DIM);
        self.ui.text_in(if name.is_empty() { "No driver" } else { &name }, Rect::new(card.x + 42.0, card.y + 6.0, card.w - 48.0, 18.0), 13.0, Weight::Medium, TEXT, Align::Left);
        self.ui.text_in(&format!("Level {level}"), Rect::new(card.x + 42.0, card.y + 24.0, card.w - 48.0, 16.0), 11.5, Weight::Regular, TEXT_DIM, Align::Left);
    }

    fn status_bar(&mut self) {
        let (text, err, at) = self.state.status.clone();
        if text.is_empty() {
            return;
        }
        let fade = if err { 1.0 } else { (1.0 - (at.elapsed().as_secs_f32() - 6.0) / 1.5).clamp(0.0, 1.0) };
        if fade <= 0.0 {
            return;
        }
        let size = self.ui.size;
        let first = text.lines().next().unwrap_or("").to_string();
        let rail_w = if mobile::mobile() { mobile::RAIL_W_MOBILE } else { RAIL_W };
        let r = Rect::new(rail_w + 20.0, size.y - 30.0, size.x - rail_w - 40.0, 24.0);
        if mobile::mobile() {
            // (readable over a page scrolled under it)
            self.ui.p().rect(Rect::new(rail_w, size.y - 34.0, size.x - rail_w, 34.0), RAIL.alpha(0.92));
        }
        let c = if err { DANGER } else { TEXT_DIM };
        self.ui.text_in(&first, r, 12.0, Weight::Regular, c.alpha(fade), Align::Left);
        self.ui.tooltip(r, &text);
    }

    /// A page's title and what it is for.
    pub fn page_title(&mut self, r: Rect, title: &str, sub: &str) -> Rect {
        self.ui.text(title, Vec2::new(r.x, r.y + 23.0), 25.0, Weight::Bold, TEXT, Align::Left);
        if !sub.is_empty() {
            // (a narrow window: the line stops short of the tabs some pages put top right,
            // it ran under them on a phone)
            let w = if r.w < 1100.0 { r.w - 340.0 } else { r.w };
            self.ui.text_in(sub, Rect::new(r.x, r.y + 34.0, w, 20.0), 12.5, Weight::Regular, TEXT_DIM, Align::Left);
        }
        Rect::new(r.x, r.y + 64.0, r.w, (r.h - 64.0).max(0.0))
    }
}


/// The showroom's renderer: a bus on a floor needs none of the game's costly passes - no
/// ambient occlusion, a small shadow map, 4x MSAA for the edges whatever the game uses -
/// and it draws Vanilla+ whatever the game does (`showroom::lighting_for`): no enhanced
/// pipelines, reflection probe or cloud noise either (`RenderOptions::preview_only`).
fn showroom_options(settings: &crate::settings::Settings) -> omsi_render::RenderOptions {
    omsi_render::RenderOptions { msaa: 4, ssao: false, shadow_size: 1024, render_scale: 1.0, preview_only: true, no_enhanced: true, ray_tracing: false, ..settings.render_options() }
}
