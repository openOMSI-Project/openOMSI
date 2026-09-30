//! Game controllers - steering wheels, pedals, joysticks, gamepads - as OMSI drives them
//! from `Inputs/gamectrler.cfg`. Each `[ctrl]` block names a device
//! and says what its eight DirectInput axes (X, Y, Z, Rx, Ry, Rz and the two sliders) do:
//! a pair per axis of the function (-1 none, 0 steering, 1 throttle, 2 brake, 3 clutch,
//! 4 throttle and brake on one axis - the options dialog's "<none>@Steering@Throttle@
//! Brake@Clutch@Throttle/Brake" less its first entry) and flags (bit 0: the axis runs the
//! other way, as the G25's pedals do). `[buttons]` lists per button the key action it
//! presses. A device the file does not know is taken as a gamepad: the left stick steers,
//! the right trigger is the throttle, the left one the brake. K switches the controller on
//! and off (OMSI's `toggel_ctrler`).

use gilrs::{Axis, EventType, Gilrs};
use std::path::Path;

/// What one axis of a device does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Func {
    Steering,
    Throttle,
    Brake,
    Clutch,
    ThrottleBrake,
}

impl Func {
    /// The file's number of the function (-1 none).
    pub(crate) fn code(f: Option<Func>) -> i32 {
        match f {
            None => -1,
            Some(Func::Steering) => 0,
            Some(Func::Throttle) => 1,
            Some(Func::Brake) => 2,
            Some(Func::Clutch) => 3,
            Some(Func::ThrottleBrake) => 4,
        }
    }

    pub(crate) fn from_code(c: i32) -> Option<Func> {
        match c {
            0 => Some(Func::Steering),
            1 => Some(Func::Throttle),
            2 => Some(Func::Brake),
            3 => Some(Func::Clutch),
            4 => Some(Func::ThrottleBrake),
            _ => None,
        }
    }

    /// As the options dialog lists them.
    pub(crate) const LABELS: [&'static str; 6] = ["<none>", "Steering", "Throttle", "Brake", "Clutch", "Throttle/Brake"];
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct DeviceCfg {
    pub(crate) name: String,
    /// The line after the name (kept as the file has it).
    pub(crate) second: String,
    /// Per DirectInput axis: the function and whether it runs the other way.
    pub(crate) axes: [Option<(Func, bool)>; 8],
    /// Per axis the file's flags beyond bit 0 (kept as they are).
    pub(crate) axis_flags: [i32; 8],
    /// Per button: the key action (empty: none) and the number after it.
    pub(crate) buttons: Vec<(String, String)>,
    /// `[FFScale]`: steering forces (centering and drag), then vibration strength.
    pub(crate) ff_scale: Option<(f32, f32)>,
}

/// The `gamectrler.cfg` in use: the content folder's (written by the launcher) before
/// OMSI 2's own.
pub(crate) fn cfg_path(root: &Path) -> std::path::PathBuf {
    omsi_cfg::find_in_roots("Inputs/gamectrler.cfg").map(|(_, p)| p).unwrap_or_else(|| root.join("Inputs").join("gamectrler.cfg"))
}

/// `Inputs/gamectrler.cfg`: the configured devices.
pub(crate) fn read_cfg(root: &Path) -> Vec<DeviceCfg> {
    let path = cfg_path(root);
    let Ok(text) = std::fs::read(&path) else { return Vec::new() };
    let mut devices = parse_cfg(&omsi_cfg::codepage::decode(&text));
    // An inherited OMSI file can contain 0/0 FFScale on a wheel. Keep its axis and
    // button bindings, but use openOMSI's 100/100 default until our own file is saved.
    let original = root.join("Inputs").join("gamectrler.cfg");
    let from_original = path == original
        || std::fs::canonicalize(&path).ok().zip(std::fs::canonicalize(&original).ok()).is_some_and(|(a, b)| a == b);
    if from_original {
        for d in &mut devices {
            if d.ff_scale == Some((0.0, 0.0)) {
                d.ff_scale = None;
            }
        }
    }
    devices
}

pub(crate) fn parse_cfg(text: &str) -> Vec<DeviceCfg> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut out: Vec<DeviceCfg> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        match lines[i] {
            "[ctrl]" => {
                out.push(DeviceCfg { name: lines.get(i + 1).unwrap_or(&"").to_string(), second: lines.get(i + 2).unwrap_or(&"0").to_string(), ..Default::default() });
                i += 3;
            }
            "[axis]" => {
                if let Some(d) = out.last_mut() {
                    for a in 0..8 {
                        let f: i32 = lines.get(i + 1 + a * 2).and_then(|v| v.parse().ok()).unwrap_or(-1);
                        let flags: i32 = lines.get(i + 2 + a * 2).and_then(|v| v.parse().ok()).unwrap_or(0);
                        d.axes[a] = Func::from_code(f).map(|f| (f, flags & 1 != 0));
                        d.axis_flags[a] = flags & !1;
                    }
                }
                i += 17;
            }
            "[buttons]" => {
                let n: usize = lines.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0).min(512);
                if let Some(d) = out.last_mut() {
                    for b in 0..n {
                        d.buttons.push((lines.get(i + 2 + b * 2).unwrap_or(&"").to_string(), lines.get(i + 3 + b * 2).unwrap_or(&"0").to_string()));
                    }
                }
                i += 2 + n * 2;
            }
            "[ffscale]" | "[FFScale]" => {
                if let Some(d) = out.last_mut() {
                    let f = |k: usize| lines.get(i + k).map(|v| omsi_cfg::parse_f64(v)).unwrap_or(1.0) as f32;
                    d.ff_scale = Some((f(1), f(2)));
                }
                i += 3;
            }
            _ => i += 1,
        }
    }
    out
}

/// The file's text for `devices`, as OMSI writes it (CR LF).
pub(crate) fn cfg_text(devices: &[DeviceCfg]) -> String {
    let mut t = String::new();
    for d in devices {
        t.push_str(&format!("\r\n[ctrl]\r\n{}\r\n{}\r\n\r\n[axis]\r\n", d.name, if d.second.is_empty() { "0" } else { &d.second }));
        for a in 0..8 {
            let (f, inv) = match d.axes[a] {
                Some((f, inv)) => (Func::code(Some(f)), inv),
                None => (-1, false),
            };
            t.push_str(&format!("{f}\r\n{}\r\n", d.axis_flags[a] | inv as i32));
        }
        t.push_str(&format!("\r\n[buttons]\r\n{}\r\n", d.buttons.len()));
        for (action, n) in &d.buttons {
            t.push_str(&format!("{action}\r\n{}\r\n", if n.is_empty() { "0" } else { n }));
        }
        let (a, b) = d.ff_scale.unwrap_or((1.0, 1.0));
        t.push_str(&format!("\r\n[FFScale]\r\n{a:.3}\r\n{b:.3}\r\n\r\n"));
    }
    t
}

/// The analog controls a controller gives this frame (None: that one is not on it).
#[derive(Debug, Clone, Copy, Default)]
pub struct Analog {
    pub steering: Option<f32>,
    /// The steering is a gamepad's stick (not a wheel): see `gamepad_steering`.
    pub stick: bool,
    pub throttle: Option<f32>,
    pub brake: Option<f32>,
    pub clutch: Option<f32>,
}

/// Where a gamepad's stick turns the wheel to (#200): a stick is no steering wheel - taken
/// as the wheel's place, the smallest movement turned the wheel a long way and a push to
/// the side was the full lock at any speed. As the bus games take it: a gentler curve
/// (squared), and less of the lock the faster the bus goes (the whole of it standing, a
/// third of it at 50 km/h, a fifth at 90 km/h).
pub fn gamepad_steering(x: f32, kmh: f32) -> f32 {
    let x = x.clamp(-1.0, 1.0);
    let curve = x * x.abs();
    let reach = 1.0 / (1.0 + (kmh.abs() - 10.0).max(0.0) / 20.0);
    curve * reach
}

/// A device connected now: its name, its axes (DirectInput slot, -1..1), whether the system
/// knows it as a gamepad (a known layout of sticks and triggers), and whether it can push
/// back (force feedback).
#[derive(Debug, Clone)]
pub(crate) struct Connected {
    pub name: String,
    pub axes: Vec<(usize, f32)>,
    pub gamepad: bool,
    pub ff: bool,
    /// The hardware advertises FFB, even when this window has not created an effect.
    pub ff_capable: bool,
    /// How many buttons it has (0: the system does not say).
    pub buttons: usize,
}

/// The first button number of the hat switches' directions (4 hats x up, right, down, left).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const HAT_BUTTONS: usize = 128;

/// The devices of every kind: gilrs (gamepads everywhere; on macOS and Linux every device),
/// and on Windows DirectInput for everything a gamepad is not (`crate::dinput`) - many wheels
/// never show up in the system's newer interface that gilrs uses there.
pub(crate) struct Devices {
    gilrs: Option<Gilrs>,
    #[cfg(windows)]
    di: Option<crate::dinput::DirectInput>,
    /// macOS: every axis element of every wheel and joystick, as last read (see `mac_hid`)
    #[cfg(target_os = "macos")]
    hid: Option<crate::mac_hid::MacHid>,
    #[cfg(target_os = "macos")]
    hid_axes: Vec<(String, Vec<(u32, f32)>)>,
}

impl Devices {
    /// `hwnd`: the window (Windows: the devices belong to it); `ff`: take them for force
    /// feedback (the game, not the launcher).
    pub fn new(hwnd: Option<isize>, ff: bool) -> Devices {
        // without gilrs's default filters: its dead zone took 10 % of every axis - on a
        // wheel of 1800 degrees, 90 degrees either side of the middle did nothing - and its
        // jitter filter held back small movements; the settings' dead zone is the only one
        let gilrs = gilrs::GilrsBuilder::new().with_default_filters(false).build().map_err(|e| log::info!("game controllers: {e}")).ok();
        #[cfg(windows)]
        let di = hwnd.and_then(|h| crate::dinput::DirectInput::new(h, ff));
        #[cfg(not(windows))]
        let _ = (hwnd, ff);
        Devices {
            gilrs,
            #[cfg(windows)]
            di,
            #[cfg(target_os = "macos")]
            hid: crate::mac_hid::MacHid::new(),
            #[cfg(target_os = "macos")]
            hid_axes: Vec::new(),
        }
    }

    /// macOS: the HID device of this name has the axes of a wheel or pedals (a slider, a
    /// dial, or the simulation page's steering, accelerator, brake, clutch).
    #[cfg(target_os = "macos")]
    pub(crate) fn hid_wheel(&self, name: &str) -> bool {
        self.hid_axes.iter().any(|(n, axes)| names_match(n, name) && axes.iter().any(|(c, _)| matches!(*c, 0x10036 | 0x10037) || (*c >> 16) == 2))
    }

    fn direct_input(&self) -> bool {
        #[cfg(windows)]
        return self.di.is_some();
        #[cfg(not(windows))]
        false
    }

    /// A device was plugged in or removed; ask the worker to rescan without blocking a frame.
    pub(crate) fn refresh(&self) {
        #[cfg(windows)]
        if let Some(d) = self.di.as_ref() {
            d.refresh();
        }
    }

    /// Release foreground wheel effects when the game loses focus.
    pub(crate) fn set_focus(&mut self, focused: bool) {
        #[cfg(windows)]
        if let Some(d) = self.di.as_mut() {
            d.set_focus(focused);
        }
        #[cfg(not(windows))]
        let _ = focused;
    }

    /// Read the devices; the buttons pressed (true) and let go since the last call:
    /// (device, button number from 0, as DirectInput and `gamectrler.cfg` count them).
    pub fn poll(&mut self) -> Vec<(String, usize, bool)> {
        let mut out = Vec::new();
        let di = self.direct_input();
        if let Some(g) = self.gilrs.as_mut() {
            while let Some(ev) = g.next_event() {
                let pad = g.gamepad(ev.id);
                match ev.event {
                    EventType::Connected => log::info!("game controller connected: {}", pad.name()),
                    // DirectInput handles wheels on Windows; system-mapped gamepads
                    // such as Xbox controllers are listed through gilrs.
                    EventType::ButtonPressed(_, code) | EventType::ButtonReleased(_, code)
                        if use_gilrs_buttons(di, pad.mapping_source() == gilrs::MappingSource::Driver) => {
                        out.push((pad.name().to_string(), button_number(&pad, code), matches!(ev.event, EventType::ButtonPressed(..))));
                    }
                    _ => {}
                }
            }
        }
        #[cfg(windows)]
        if let Some(d) = self.di.as_mut() {
            d.poll();
            out.append(&mut d.events);
        }
        #[cfg(target_os = "macos")]
        if let Some(h) = self.hid.as_mut() {
            self.hid_axes = h.read();
        }
        out
    }

    /// The devices connected, with their axes as last read.
    pub fn connected(&self) -> Vec<Connected> {
        let mut v = Vec::new();
        // (Windows: an Xbox-type pad is gilrs's - the system's own layout -, everything else
        // DirectInput's; a wheel that a community mapping makes a "gamepad" in gilrs was
        // listed twice, "Logitech G29" beside "G29 Driving Force Racing Wheel")
        let xinput_pads = self.gilrs.as_ref().is_some_and(|g| g.gamepads().any(|(_, p)| p.mapping_source() == gilrs::MappingSource::Driver));
        #[cfg(windows)]
        if let Some(d) = self.di.as_ref() {
            v.extend(
                d.devices
                    .iter()
                    .filter(|_| d.is_focused())
                    // A G920's DirectInput name contains "Xbox One", but it is the
                    // force-feedback wheel. Keep it even when gilrs also lists a pad.
                    .filter(|d| include_direct_input_device(&d.name, d.ff_capable(), xinput_pads))
                    .map(|d| Connected { name: d.name.clone(), axes: d.axes(), gamepad: false, ff: d.has_ff(), ff_capable: d.ff_capable(), buttons: d.buttons.min(128) }),
            );
        }
        let _ = xinput_pads;
        if let Some(g) = self.gilrs.as_ref() {
            for (_, pad) in g.gamepads() {
                #[allow(unused_mut)]
                let mut gamepad = pad.mapping_source() != gilrs::MappingSource::None;
                // (macOS: a device with sliders or the simulation page's axes is a wheel or
                // pedals, whatever SDL's list calls it - the HORI Truck Control System was
                // taken as a gamepad: its left stick steered, with a gamepad's dead zone)
                #[cfg(target_os = "macos")]
                if self.hid_wheel(pad.name()) {
                    gamepad = false;
                }
                if self.direct_input() && pad.mapping_source() != gilrs::MappingSource::Driver {
                    continue;
                }
                if v.iter().any(|c: &Connected| names_match(&c.name, pad.name())) {
                    continue;
                }
                #[allow(unused_mut)]
                let mut axes: Vec<(u32, f32)> = pad.state().axes().map(|(c, d)| (c.into_u32(), d.value())).collect();
                // (macOS: the device's own axis elements where it is found among them - two
                // of one usage stay two)
                #[cfg(target_os = "macos")]
                if !gamepad {
                    if let Some((_, a)) = self.hid_axes.iter().find(|(n, _)| names_match(n, pad.name())) {
                        axes = a.clone();
                    }
                }
                #[cfg(target_os = "linux")]
                let buttons = declared_button_count(pad.name());
                #[cfg(not(target_os = "linux"))]
                let buttons = 0;
                v.push(Connected { name: pad.name().to_string(), axes: di_slots(&axes), gamepad, ff: pad.is_ff_supported(), ff_capable: pad.is_ff_supported(), buttons });
            }
        }
        // (and a wheel gilrs does not list at all: one whose only axes are the simulation
        // page's steering and pedals)
        #[cfg(target_os = "macos")]
        for (name, axes) in &self.hid_axes {
            if !v.iter().any(|c| names_match(&c.name, name)) {
                v.push(Connected { name: name.clone(), axes: di_slots(axes), gamepad: false, ff: false, ff_capable: false, buttons: 0 });
            }
        }
        v
    }
}

fn use_gilrs_buttons(direct_input: bool, system_gamepad: bool) -> bool {
    !direct_input || system_gamepad
}

/// A DirectInput name of an Xbox-type pad (which gilrs lists with the system's layout).
#[cfg_attr(not(windows), allow(dead_code))]
fn xinput_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("xbox") || n.contains("xinput") || n.starts_with("controller (")
}

#[cfg_attr(not(windows), allow(dead_code))]
fn include_direct_input_device(name: &str, ff_capable: bool, xinput_pads: bool) -> bool {
    !xinput_pads || !xinput_name(name) || ff_capable
}

/// The handle of `window` for DirectInput (Windows; elsewhere nothing is needed).
pub(crate) fn window_handle(window: &winit::window::Window) -> Option<isize> {
    #[cfg(windows)]
    {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(h) = window.window_handle() {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                return Some(w.hwnd.get());
            }
        }
    }
    let _ = window;
    None
}

/// What the force feedback is made of this frame (OMSI's `FF_*` variables of the bus and
/// its speed).
#[derive(Debug, Clone, Copy, Default)]
pub struct FfInput {
    /// Driving (from the driver's seat): the forces are on; else the wheel is let go.
    pub on: bool,
    pub kmh: f32,
    /// Sideways acceleration in the bus frame (m/s², right positive).
    pub lateral_accel: f32,
    /// Short jolt from wheel suspension travel or an impact, 0..1.
    pub wheel_bump: f32,
    /// Time since the latest wheel jolt, for a repeatable initial kick.
    pub(crate) wheel_bump_age: f32,
    /// `FF_Vib_Amp` 0..1 and `FF_Vib_Period` (hundredths of a second) of the scripts.
    pub vib_amp: f32,
    pub vib_period: f32,
    pub dt: f32,
}

pub struct Controllers {
    devices: Devices,
    focused: bool,
    cfg: Vec<DeviceCfg>,
    pub enabled: bool,
    /// The settings' dead zone round the centre of a set-up device's axes (0..0.3).
    pub deadzone: f32,
    /// The pedals' response curves (Settings → pedal strength; 1 = as the pedal reads).
    pub pedal_throttle: f32,
    pub pedal_brake: f32,
    /// Devices switched off (Settings: `ctrl_off`): not read at all.
    pub disabled: Vec<String>,
    /// Force feedback the other way round (Settings: `ff_invert`).
    pub ff_invert: bool,
    /// Force feedback and rumble switched on (Settings: `ff_enabled`).
    pub ff_enabled: bool,
    /// The wheel's rotation over the rotation that is the bus's full lock (Settings:
    /// `wheel_range` / `wheel_lock`; 1 = the whole wheel is the full lock, as OMSI).
    pub steer_gain: f32,
    /// Key actions of buttons pressed (true) and released (false) since the last poll.
    pub actions: Vec<(String, bool)>,
    /// Devices told about in the log (and on the screen) as not set up.
    announced: Vec<String>,
    /// A message for the screen: a wheel that is not set up.
    pub notice: Option<String>,
    /// The steering device: its name, where the wheel stands (-1..1) and stood before, and
    /// whether it pushes back.
    steer: Option<(String, f32, f32, bool)>,
    ff_t: f32,
    ff_lateral: f32,
    ff_bump: f32,
    ff_bump_age: f32,
    ff_source_logged: Option<String>,
    /// The rumble playing (`FF_Vib_Amp` and `FF_Vib_Period` of the bus), rebuilt when
    /// either changes.
    rumble: Option<(gilrs::ff::Effect, f32, f32)>,
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    wheel: Option<crate::evdev_ff::Wheel>,
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    wheel_tried: Option<(String, std::time::Instant)>,
}

impl Controllers {
    pub(crate) fn refresh_devices(&self) {
        self.devices.refresh();
    }

    pub(crate) fn set_focus(&mut self, focused: bool) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        self.devices.set_focus(focused);
        if !focused {
            self.steer = None;
            self.actions.clear();
            self.ff_source_logged = None;
        }
    }

    pub fn new(root: &Path, hwnd: Option<isize>) -> Controllers {
        let devices = Devices::new(hwnd, true);
        let cfg = read_cfg(root);
        for c in devices.connected() {
            log::info!("game controller: {} ({})", c.name, if cfg.iter().any(|d| names_match(&d.name, &c.name)) { "set up in gamectrler.cfg" } else if c.gamepad { "as a gamepad" } else { "not set up: its X axis steers" });
        }
        Controllers { devices, focused: true, cfg, deadzone: 0.0, pedal_throttle: 1.0, pedal_brake: 1.0, disabled: Vec::new(), ff_invert: false, ff_enabled: true, steer_gain: 1.0, enabled: true, actions: Vec::new(), announced: Vec::new(), notice: None, steer: None, ff_t: 0.0, ff_lateral: 0.0, ff_bump: 0.0, ff_bump_age: 0.0, ff_source_logged: None, rumble: None, #[cfg(all(target_os = "linux", target_pointer_width = "64"))] wheel: None, #[cfg(all(target_os = "linux", target_pointer_width = "64"))] wheel_tried: None }
    }

    /// A wheel or joystick steers the bus (then the arrow keys look around, as in OMSI:
    /// a G29's buttons set to the arrow keys turned the view there).
    pub fn wheel_steering(&self) -> bool {
        self.enabled && self.steer.is_some()
    }

    /// Read the devices: the analog controls, and the button actions into `actions`.
    pub fn poll(&mut self) -> Analog {
        let mut out = Analog::default();
        for (name, n, down) in self.devices.poll() {
            if self.off(&name) {
                continue;
            }
            if let Some(action) = find_device_cfg(&self.cfg, &name).and_then(|d| d.buttons.get(n)).filter(|a| !a.0.is_empty()) {
                self.actions.push((action.0.clone(), down));
            }
        }
        if !self.enabled {
            return out;
        }
        // the devices set up in gamectrler.cfg first; a device the file does not know only
        // gives what none of them does - a pad lying beside a set-up wheel held the steering
        // at its own centre, whichever the system listed first
        let off = self.disabled.clone();
        let mut pads: Vec<(Option<&DeviceCfg>, Connected)> = self.devices.connected().into_iter().filter(|c| !off.iter().any(|d| names_match(d, &c.name))).map(|c| (find_device_cfg(&self.cfg, &c.name), c)).collect();
        pads.sort_by_key(|(cfg, _)| cfg.is_none());
        let mut steer: Option<(String, f32, bool)> = None;
        let dz = self.deadzone.clamp(0.0, 0.3);
        for (cfg, c) in pads {
            match cfg {
                Some(d) => {
                    for (k, v) in c.axes.iter().copied() {
                        let Some((f, inverted)) = d.axes[k] else { continue };
                        let v = if inverted { -v } else { v };
                        // the dead zone: round the wheel's centre, or at a pedal's rest
                        let v = match f {
                            Func::Steering | Func::ThrottleBrake => v.signum() * ((v.abs() - dz).max(0.0) / (1.0 - dz)),
                            _ => ((v + 1.0 - 2.0 * dz).max(0.0) / (1.0 - dz)) - 1.0,
                        };
                        // a pedal travels the whole range, -1 up to 1 down
                        let pedal = crate::settings::pedal_ends(((v + 1.0) * 0.5).clamp(0.0, 1.0));
                        match f {
                            Func::Steering => {
                                let v = v * self.steer_gain;
                                set(&mut out.steering, v.clamp(-1.0, 1.0));
                                if steer.is_none() {
                                    steer = Some((c.name.clone(), v.clamp(-1.0, 1.0), c.ff));
                                }
                            }
                            Func::Throttle => set(&mut out.throttle, crate::settings::pedal_curve(pedal, self.pedal_throttle)),
                            Func::Brake => set(&mut out.brake, crate::settings::pedal_curve(pedal, self.pedal_brake)),
                            Func::Clutch => set(&mut out.clutch, pedal),
                            Func::ThrottleBrake => {
                                set(&mut out.throttle, crate::settings::pedal_curve((-v).max(0.0), self.pedal_throttle));
                                set(&mut out.brake, crate::settings::pedal_curve(v.max(0.0), self.pedal_brake));
                            }
                        }
                    }
                }
                None if c.gamepad => {}
                None => {
                    // a wheel or joystick nobody has set up yet: its X axis steers (as on
                    // nearly every wheel), the pedals wait for the set-up (Launcher →
                    // Controls → Game controllers), said once on the screen
                    if !self.announced.contains(&c.name) {
                        self.announced.push(c.name.clone());
                        log::info!("game controller {} is not set up: its X axis steers", c.name);
                        self.notice = Some(format!("{} is not set up: it steers; set up its pedals and buttons in the launcher (Controls → Game controllers)", c.name));
                    }
                    if let Some((_, v)) = c.axes.iter().find(|(k, _)| *k == 0) {
                        let v = v.signum() * ((v.abs() - dz.max(0.02)).max(0.0) / (1.0 - dz.max(0.02))) * self.steer_gain;
                        out.steering.get_or_insert(v.clamp(-1.0, 1.0));
                        if steer.is_none() {
                            steer = Some((c.name.clone(), v.clamp(-1.0, 1.0), c.ff));
                        }
                    }
                }
            }
        }
        // gamepads: the left stick steers, the triggers are the pedals
        let di = self.devices.direct_input();
        let off = self.disabled.clone();
        if let Some(g) = self.devices.gilrs.as_ref() {
            for (_, pad) in g.gamepads() {
                // (a pad OMSI's gamectrler.cfg names is driven by that file through DirectInput
                // - except an Xbox-type pad on Windows, whose DirectInput twin is left out
                // for the system's own layout: with the file naming it, nobody read it, and
                // its triggers were no pedals, #171)
                let xinput = cfg!(windows) && pad.mapping_source() == gilrs::MappingSource::Driver;
                if pad.mapping_source() == gilrs::MappingSource::None || (!xinput && self.cfg.iter().any(|d| names_match(&d.name, pad.name()))) {
                    continue;
                }
                #[cfg(target_os = "macos")]
                if self.devices.hid_wheel(pad.name()) {
                    continue;
                }
                if (di && pad.mapping_source() != gilrs::MappingSource::Driver) || off.iter().any(|d| names_match(d, pad.name())) {
                    continue;
                }
                let x = pad.value(Axis::LeftStickX);
                let dead = |v: f32| if v.abs() < 0.08 { 0.0 } else { v };
                let rt = pad.button_data(gilrs::Button::RightTrigger2).map(|d| d.value()).unwrap_or(0.0);
                let lt = pad.button_data(gilrs::Button::LeftTrigger2).map(|d| d.value()).unwrap_or(0.0);
                if out.steering.is_none() {
                    out.steering = Some(dead(x));
                    out.stick = true;
                }
                out.throttle.get_or_insert(crate::settings::pedal_curve(rt, self.pedal_throttle));
                out.brake.get_or_insert(crate::settings::pedal_curve(lt, self.pedal_brake));
            }
        }
        let before = self.steer.as_ref().filter(|s| steer.as_ref().is_some_and(|n| n.0 == s.0)).map(|s| s.1);
        self.steer = steer.map(|(name, v, ff)| (name, v, before.unwrap_or(v), ff));
        out
    }

    /// On a force-feedback wheel, combine parking resistance, centring, the bus's
    /// lateral motion, front-wheel bumps and script-driven vibration. Other devices
    /// get vibration as rumble.
    pub fn feedback(&mut self, f: FfInput) {
        let on = self.enabled && f.on && self.ff_enabled;
        let mut f = f;
        if on {
            // The bus body reacts to road and tyre forces every physics step. A short
            // filter keeps those impulses from becoming sharp forces at the wheel.
            let blend = (f.dt / 0.15).clamp(0.0, 1.0);
            self.ff_lateral += (f.lateral_accel.clamp(-6.0, 6.0) - self.ff_lateral) * blend;
        } else {
            self.ff_lateral = 0.0;
        }
        f.lateral_accel = self.ff_lateral;
        let incoming_bump = if on { f.wheel_bump.clamp(0.0, 1.0) } else { 0.0 };
        if incoming_bump > 0.05 && (self.ff_bump < 0.02 || incoming_bump > self.ff_bump + 0.12) {
            self.ff_bump_age = 0.0;
        } else {
            self.ff_bump_age += f.dt.max(0.0);
        }
        self.ff_bump = if on { (self.ff_bump - f.dt.max(0.0) * 6.0).max(incoming_bump) } else { 0.0 };
        f.wheel_bump = self.ff_bump;
        f.wheel_bump_age = self.ff_bump_age;
        if self.ff_source_logged.as_deref() != self.steer.as_ref().map(|s| s.0.as_str()) {
            self.ff_source_logged = self.steer.as_ref().map(|s| s.0.clone());
            if let Some((name, _, _, effect)) = self.steer.as_ref() {
                let cfg = find_device_cfg(&self.cfg, name);
                let (steering, vibration) = cfg.and_then(|d| d.ff_scale).unwrap_or((1.0, 1.0));
                #[cfg(windows)]
                let axis_reversed = self.devices.di.as_ref().is_some_and(|di| force_axis_reversed(cfg, di.force_axis(name)));
                #[cfg(not(windows))]
                let axis_reversed = false;
                log::info!("force feedback: steering source {name}, effect available: {effect}, config: {}, steering force: {steering:.2}, vibration: {vibration:.2}, invert: {}", cfg.map(|d| d.name.as_str()).unwrap_or("none"), self.ff_invert ^ axis_reversed);
            }
        }
        #[cfg(windows)]
        if let (Some((name, x, x0, true)), Some(di)) = (self.steer.clone(), self.devices.di.as_mut()) {
            // (the file's [FFScale] of the device: steering forces, then vibration)
            let cfg = find_device_cfg(&self.cfg, &name);
            let (k_s, k_e) = cfg.and_then(|d| d.ff_scale).unwrap_or((1.0, 1.0));
            let force = if on { wheel_force(&f, x, x0, &mut self.ff_t, k_s, k_e) } else { 0.0 };
            // The wheel force is calculated from the steering axis after its configured
            // reversal, while DirectInput sends forces in the physical axis direction.
            let axis_reversed = force_axis_reversed(cfg, di.force_axis(&name));
            let force = if self.ff_invert ^ axis_reversed { -force } else { force };
            if di.set_force(&name, force) {
                return;
            }
        }
        #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
        if let Some((name, x, x0, true)) = self.steer.clone() {
            let other = self.wheel.as_ref().is_some_and(|w| w.name != name);
            let retry = self.wheel.is_none() && self.wheel_tried.as_ref().is_none_or(|(n, t)| *n != name || t.elapsed() > std::time::Duration::from_secs(2));
            if other || retry {
                self.wheel_tried = Some((name.clone(), std::time::Instant::now()));
                self.wheel = crate::evdev_ff::Wheel::open(&name);
            }
            if let Some(w) = self.wheel.as_mut() {
                let (k_s, k_e) = find_device_cfg(&self.cfg, &name).and_then(|d| d.ff_scale).unwrap_or((1.0, 1.0));
                let force = if on { wheel_force(&f, x, x0, &mut self.ff_t, k_s, k_e) } else { 0.0 };
                if !w.set_force(if self.ff_invert { -force } else { force }) {
                    log::warn!("force feedback: {name} went away; looking for it again");
                    self.wheel = None;
                }
                return;
            }
        }
        let _ = (&self.steer, &self.ff_t, wheel_force);
        self.rumble_feedback(if on { f.vib_amp.max(f.wheel_bump * 0.75) } else { 0.0 }, f.vib_period);
    }

    /// The shaking as a rumble (`FF_Vib_Amp`, `FF_Vib_Period`: OMSI hands DirectInput
    /// Round(period × 10000) µs, so a hundredth of a second per unit - the shaking comes in
    /// pulses that long, on for half of it; 0 is a steady rumble).
    fn rumble_feedback(&mut self, amp: f32, period: f32) {
        let amp = amp.clamp(0.0, 1.0);
        let period = if period.is_finite() { period.clamp(0.0, 100.0) } else { 0.0 };
        let Some(g) = self.devices.gilrs.as_mut() else { return };
        if let Some((_, was, was_period)) = &self.rumble {
            if (was - amp).abs() < 0.02 && (was_period - period).abs() < 0.05 {
                return;
            }
        }
        self.rumble = None;
        if amp < 0.01 {
            return;
        }
        let pads: Vec<gilrs::GamepadId> = g.gamepads().filter(|(_, p)| p.is_ff_supported()).map(|(id, _)| id).collect();
        if pads.is_empty() {
            return;
        }
        // (the file's [FFScale] of a set-up device scales it)
        let scale = self.cfg.iter().find_map(|d| d.ff_scale).map(|s| s.1).unwrap_or(1.0).clamp(0.0, 2.0);
        let m = ((amp * scale).min(1.0) * u16::MAX as f32) as u16;
        let ms = (period * 10.0).round() as u32;
        let scheduling = if ms >= 20 {
            gilrs::ff::Replay { after: gilrs::ff::Ticks::from_ms(0), play_for: gilrs::ff::Ticks::from_ms(ms / 2), with_delay: gilrs::ff::Ticks::from_ms(ms - ms / 2) }
        } else {
            Default::default()
        };
        let effect = gilrs::ff::EffectBuilder::new()
            .add_effect(gilrs::ff::BaseEffect { kind: gilrs::ff::BaseEffectType::Strong { magnitude: m }, scheduling, ..Default::default() })
            .add_effect(gilrs::ff::BaseEffect { kind: gilrs::ff::BaseEffectType::Weak { magnitude: m / 2 }, scheduling, ..Default::default() })
            .repeat(gilrs::ff::Repeat::Infinitely)
            .gamepads(&pads)
            .finish(g);
        if let Ok(e) = effect {
            let _ = e.play();
            self.rumble = Some((e, amp, period));
        }
    }

    fn off(&self, name: &str) -> bool {
        self.disabled.iter().any(|d| names_match(d, name))
    }

    /// Any controller there at all.
    pub fn any(&self) -> bool {
        !self.devices.connected().is_empty()
    }
}

/// Front-wheel contact reaches the steering linkage directly; rear-wheel contact
/// reaches it through the bus body at a lower strength.
pub(crate) fn wheel_contact_bump(body: &omsi_sim::rigid::RigidBody, kmh: f32) -> f32 {
    body.wheels.iter().enumerate().map(|(i, w)| {
        let impact_speed = body.wheel_impacts.iter().filter(|hit| hit.obstacle == i).map(|hit| hit.speed).fold(0.0, f32::max);
        bump_strength(w.compression_rate, impact_speed, kmh) * if w.steered { 1.0 } else { 0.55 }
    }).fold(0.0, f32::max)
}

fn bump_strength(compression_rate: f32, impact_speed: f32, kmh: f32) -> f32 {
    let suspension = ((compression_rate.abs() - 0.12) / 0.9).clamp(0.0, 1.0);
    let impact = ((impact_speed - 0.12) / 1.1).clamp(0.0, 1.0);
    suspension.max(impact) * (kmh.abs() / 4.0).clamp(0.0, 1.0)
}

/// The force on a wheel standing at `x` (-1 full left .. 1), `x0` the frame before: -1..1.
/// Tyre scrub resists turning the wheel at a standstill and falls away once the bus rolls.
/// Self-aligning torque then returns the wheel to centre, with a softer response near full
/// lock and feedback from the bus's lateral acceleration. Front-wheel jolts and the
/// scripts' shaking come on top.
fn wheel_force(f: &FfInput, x: f32, x0: f32, t: &mut f32, k_springs: f32, k_effects: f32) -> f32 {
    let dt = f.dt.max(1e-3);
    let v = f.kmh.abs();
    let x = x.clamp(-1.0, 1.0);
    // At road speed, power steering gives the driver a firmer sense of direction.
    // Keep parking and town-speed forces familiar while separating 70 km/h from 10 km/h.
    let road_speed = ((v - 20.0) / 50.0).clamp(0.0, 1.0);
    let spring_strength = (0.22 + 0.28 * v / (v + 10.0)) * (1.0 + 0.5 * road_speed);
    let moving_steering_gain = 1.0 + 0.18 * v / (v + 8.0);
    let spring = -spring_strength * x / (1.0 + 0.65 * x.abs());
    let road_align = -(f.lateral_accel / 9.81).clamp(-0.45, 0.45) * 0.25 * (v / 5.0).clamp(0.0, 1.0);
    // Assisted steering should not demand ever more hand force near full lock.
    let lock_assist = 1.0 / (1.0 + 0.55 * x * x);
    let turning_speed = ((x - x0) / dt).clamp(-4.0, 4.0);
    // Without a steering-column torque sensor, motion away from the centre is our
    // indication that the driver is actively turning. Assist that motion, but keep
    // the full self-aligning torque when the wheel is held or let go.
    let turning_out = (x * turning_speed * 2.0).clamp(0.0, 1.0);
    let assist = 1.0 - (0.4 - 0.16 * (v / 80.0).min(1.0)) * turning_out;
    let parking_drag = 0.018 + 0.12 / (1.0 + (v / 6.0).powi(2));
    // The power steering helps the wheel return; do not let parking resistance
    // cancel the centring force while it is already moving towards the middle.
    let returning = x * turning_speed < 0.0;
    let drag = -turning_speed * parking_drag * if returning { 0.2 } else { 1.0 };
    *t += dt;
    let period = (f.vib_period * 0.01).max(0.02);
    let shake = f.vib_amp.clamp(0.0, 1.0) * 0.25 * (std::f32::consts::TAU * *t / period).sin();
    // Preserve small road details while softening kerb-sized peaks. One short
    // kick and rebound feels less like a continuously shaking wheel mount.
    let bump = f.wheel_bump.clamp(0.0, 1.0).sqrt() * 0.46 * (std::f32::consts::TAU * f.wheel_bump_age * 6.5).cos();
    (((spring + road_align) * lock_assist * assist + drag) * moving_steering_gain * k_springs.clamp(0.0, 2.0) + (shake + bump) * k_effects.clamp(0.0, 2.0)).clamp(-1.0, 1.0)
}

/// A control several set-up devices give: the first one set wins, unless a later one is
/// moved further (two wheels, or pedals on their own device).
fn set(slot: &mut Option<f32>, v: f32) {
    match slot {
        Some(old) if old.abs() >= v.abs() => {}
        _ => *slot = Some(v),
    }
}

/// The DirectInput axis (0-5 X, Y, Z, Rx, Ry, Rz; 6, 7 the sliders) each of a device's axes
/// is, from the system's code for it: OMSI's gamectrler.cfg numbers them so. The HID usages
/// on macOS (generic desktop page 1: 0x30-0x35, slider 0x36, dial 0x37; the simulation page
/// 2: steering, accelerator, brake, clutch as Windows' HID driver places them), the evdev
/// ABS codes on Linux (ABS_X..ABS_RZ, then THROTTLE and RUDDER as the sliders), the axis
/// index on Windows. Axes of no known slot take the free ones in their order.
pub(crate) fn di_slots(axes: &[(u32, f32)]) -> Vec<(usize, f32)> {
    let known = |code: u32| -> Option<usize> {
        let (hi, lo) = (code >> 16, code & 0xFFFF);
        if cfg!(target_os = "macos") {
            match (hi, lo) {
                (1, 0x30..=0x35) => Some((lo - 0x30) as usize),
                (1, 0x36) => Some(6),
                (1, 0x37) => Some(7),
                (2, 0xC8) => Some(0), // steering
                (2, 0xC4) => Some(1), // accelerator
                (2, 0xC5) => Some(5), // brake
                (2, 0xC6) => Some(6), // clutch
                (2, 0xBB) => Some(2), // throttle
                (2, 0xBA) => Some(5), // rudder
                _ => None,
            }
        } else if cfg!(target_os = "linux") {
            match lo {
                0..=5 => Some(lo as usize),
                6 | 9 => Some(6), // ABS_THROTTLE, ABS_GAS
                7 | 10 => Some(7), // ABS_RUDDER, ABS_BRAKE
                _ => None,
            }
        } else {
            (lo < 8).then_some(lo as usize)
        }
    };
    let mut sorted = axes.to_vec();
    sorted.sort_by_key(|(c, _)| *c);
    let mut used = [false; 8];
    let mut out: Vec<(usize, f32)> = Vec::new();
    let mut rest = Vec::new();
    for (c, v) in sorted {
        // (a second slider of the same usage is DirectInput's slider 1)
        let k = known(c).map(|k| if used[k] && k == 6 && !used[7] && cfg!(target_os = "macos") && c == 0x10036 { 7 } else { k });
        match k.filter(|k| !used[*k]) {
            Some(k) => {
                used[k] = true;
                out.push((k, v));
            }
            None => rest.push(v),
        }
    }
    for v in rest {
        if let Some(k) = used.iter().position(|u| !u) {
            used[k] = true;
            out.push((k, v));
        }
    }
    out
}

/// OMSI stores DirectInput's product name; the system's may differ in spacing and case.
fn normalized_device_name(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

pub(crate) fn names_match(a: &str, b: &str) -> bool {
    // (letters of any script: a name of Cyrillic or Chinese letters only was empty here and
    // matched nothing)
    let (a, b) = (normalized_device_name(a), normalized_device_name(b));
    !a.is_empty() && (a == b || a.contains(&b) || b.contains(&a))
}

/// An exact device name wins over a shorter alias elsewhere in the same OMSI file.
fn find_device_cfg<'a>(cfg: &'a [DeviceCfg], name: &str) -> Option<&'a DeviceCfg> {
    let exact = normalized_device_name(name);
    cfg.iter().find(|d| !exact.is_empty() && normalized_device_name(&d.name) == exact)
        .or_else(|| cfg.iter().find(|d| names_match(&d.name, name)))
}

#[cfg_attr(not(windows), allow(dead_code))]
fn force_axis_reversed(cfg: Option<&DeviceCfg>, axis: Option<usize>) -> bool {
    axis.and_then(|axis| cfg.and_then(|d| d.axes.get(axis).copied().flatten()))
        .is_some_and(|(function, reversed)| function == Func::Steering && reversed)
}

/// The button's number on its device as DirectInput counts them (and `gamectrler.cfg` with
/// it), from the system's code for it: the HID button usage on macOS (page 9, from 1), the
/// evdev key code on Linux (BTN_JOYSTICK.. and BTN_TRIGGER_HAPPY.. for a wheel's or
/// joystick's buttons, BTN_GAMEPAD.. for a pad's), the button index on Windows. It used to be
/// the place among the buttons pressed so far - the first button ever pressed was "button 1"
/// whichever it was.
pub(crate) fn button_number(pad: &gilrs::Gamepad, code: gilrs::ev::Code) -> usize {
    #[cfg(target_os = "linux")]
    if let Some(n) = declared_button_index(pad.name(), code.into_u32()) {
        return n;
    }
    code_button(code.into_u32()).unwrap_or_else(|| {
        let mut codes: Vec<u32> = pad.state().buttons().map(|(c, _)| c.into_u32()).collect();
        codes.sort_unstable();
        codes.iter().position(|c| *c == code.into_u32()).unwrap_or(usize::MAX)
    })
}

#[cfg(target_os = "linux")]
fn with_declared<R>(name: &str, f: impl FnOnce(&[u32]) -> R) -> Option<R> {
    static DECLARED: std::sync::Mutex<Vec<(String, Option<Vec<u32>>)>> = std::sync::Mutex::new(Vec::new());
    let mut cache = DECLARED.lock().unwrap_or_else(|e| e.into_inner());
    if !cache.iter().any(|(n, _)| n == name) {
        cache.push((name.to_string(), declared_buttons(name)));
    }
    cache.iter().find(|(n, _)| n == name)?.1.as_deref().map(f)
}

#[cfg(target_os = "linux")]
fn declared_button_index(name: &str, code: u32) -> Option<usize> {
    with_declared(name, |codes| button_index(codes, code & 0xFFFF)).flatten()
}

#[cfg(target_os = "linux")]
fn declared_button_count(name: &str) -> usize {
    with_declared(name, button_count).unwrap_or(0)
}

#[cfg(any(target_os = "linux", test))]
fn button_count(declared: &[u32]) -> usize {
    declared.iter().filter_map(|c| button_index(declared, *c).or_else(|| code_button(*c))).map(|n| n + 1).max().unwrap_or(0).min(128)
}

#[cfg(target_os = "linux")]
fn declared_buttons(name: &str) -> Option<Vec<u32>> {
    let mut nodes: Vec<std::path::PathBuf> = std::fs::read_dir("/sys/class/input").ok()?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("event"))).collect();
    nodes.sort();
    let named: Vec<(String, Vec<u32>)> = nodes
        .iter()
        .filter_map(|p| {
            let dev_name = std::fs::read_to_string(p.join("device/name")).ok()?.trim().to_string();
            let bitmap = std::fs::read_to_string(p.join("device/capabilities/key")).ok()?;
            let codes = key_bitmap_buttons(&bitmap);
            (names_match(&dev_name, name) && !codes.is_empty()).then_some((dev_name, codes))
        })
        .collect();
    named.iter().find(|(n, _)| n == name).or(named.first()).map(|(_, c)| c.clone())
}

#[cfg(any(target_os = "linux", test))]
fn key_bitmap_buttons(bitmap: &str) -> Vec<u32> {
    let words: Vec<u64> = bitmap.split_whitespace().rev().filter_map(|w| u64::from_str_radix(w, 16).ok()).collect();
    (0x100..words.len() as u32 * 64).filter(|b| words[*b as usize / 64] >> (b % 64) & 1 != 0).collect()
}

#[cfg(any(target_os = "linux", test))]
fn button_index(declared: &[u32], code: u32) -> Option<usize> {
    if declared.iter().all(|c| code_button(*c).is_some()) {
        return None;
    }
    declared.iter().position(|c| *c == code)
}

fn code_button(code: u32) -> Option<usize> {
    let (hi, lo) = (code >> 16, (code & 0xFFFF) as usize);
    if cfg!(target_os = "macos") {
        (hi == 9 && lo >= 1).then(|| lo - 1)
    } else if cfg!(target_os = "linux") || cfg!(target_os = "android") {
        match lo {
            0x120..=0x12f => Some(lo - 0x120),
            0x2c0..=0x2e7 => Some(16 + lo - 0x2c0),
            0x130..=0x13e => Some(lo - 0x130),
            0x100..=0x109 => Some(lo - 0x100),
            _ => None,
        }
    } else if cfg!(windows) {
        (hi == 0).then_some(lo)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn names() {
        assert!(super::names_match("Logitech G25 Racing Wheel USB", "Logitech G25 Racing Wheel"));
        assert!(!super::names_match("", "x"));
        assert!(super::names_match("Кнопочная панель", "кнопочная  панель"));
        assert!(!super::names_match("Кнопочная панель", "Руль"));
    }

    #[test]
    fn an_xbox_named_ff_wheel_stays_visible_in_the_launcher() {
        assert!(super::include_direct_input_device("G920 Driving Force Racing Wheel for Xbox One", true, true));
        assert!(!super::include_direct_input_device("Controller (Xbox One)", false, true));
    }

    #[test]
    fn system_gamepad_buttons_work_alongside_direct_input_wheels() {
        assert!(super::use_gilrs_buttons(true, true));
        assert!(!super::use_gilrs_buttons(true, false));
        assert!(super::use_gilrs_buttons(false, false));
    }
}

#[cfg(test)]
mod slot_tests {
    use super::*;

    #[test]
    fn a_missing_axis_does_not_shift_the_rest() {
        // X and Rz only (a wheel with one pedal axis): Rz stays slot 5
        let axes = if cfg!(target_os = "macos") {
            vec![(0x1_0030, 0.5), (0x1_0035, -1.0)]
        } else {
            vec![(0x3_0000, 0.5), (0x3_0005, -1.0)]
        };
        let got = di_slots(&axes);
        assert!(got.contains(&(0, 0.5)), "{got:?}");
        if !cfg!(windows) {
            assert!(got.contains(&(5, -1.0)), "{got:?}");
        }
    }

    #[test]
    fn two_devices_merge_by_the_larger_movement() {
        let mut s = None;
        set(&mut s, 0.0);
        set(&mut s, 0.4);
        set(&mut s, -0.1);
        assert_eq!(s, Some(0.4));
    }
}

#[cfg(test)]
mod cfg_tests {
    #[test]
    fn force_feedback_scales_are_saved_per_controller() {
        let devices = vec![
            super::DeviceCfg { name: "Wheel A".into(), ff_scale: Some((2.0, 0.5)), ..Default::default() },
            super::DeviceCfg { name: "Wheel B".into(), ff_scale: Some((0.75, 1.25)), ..Default::default() },
        ];
        let saved = super::parse_cfg(&super::cfg_text(&devices));
        assert_eq!(saved[0].ff_scale, Some((2.0, 0.5)));
        assert_eq!(saved[1].ff_scale, Some((0.75, 1.25)));
    }

    #[test]
    fn exact_wheel_configuration_beats_a_shorter_alias() {
        let devices = vec![
            super::DeviceCfg { name: "G920".into(), ff_scale: Some((0.0, 0.0)), ..Default::default() },
            super::DeviceCfg { name: "G920 Driving Force Racing Wheel for Xbox One".into(), ff_scale: Some((1.2, 0.6)), ..Default::default() },
        ];
        let matched = super::find_device_cfg(&devices, "G920 Driving Force Racing Wheel for Xbox One").unwrap();
        assert_eq!(matched.ff_scale, Some((1.2, 0.6)));
    }

    #[test]
    fn a_reversed_steering_axis_reverses_its_motor_force_too() {
        let mut wheel = super::DeviceCfg::default();
        wheel.axes[0] = Some((super::Func::Steering, true));
        wheel.axes[1] = Some((super::Func::Throttle, true));
        assert!(super::force_axis_reversed(Some(&wheel), Some(0)));
        assert!(!super::force_axis_reversed(Some(&wheel), Some(1)));
        assert!(!super::force_axis_reversed(Some(&wheel), None));
    }

    #[test]
    fn the_stock_file_round_trips() {
        let Ok(bytes) = std::fs::read("../../../OMSI 2 Original/Inputs/gamectrler.cfg") else { return };
        let text = omsi_cfg::codepage::decode(&bytes);
        let devs = super::parse_cfg(&text);
        assert!(devs.iter().any(|d| d.name.contains("G25")), "{:?}", devs.iter().map(|d| &d.name).collect::<Vec<_>>());
        let again = super::parse_cfg(&super::cfg_text(&devs));
        assert_eq!(again, devs);
    }
}

#[cfg(test)]
mod button_tests {
    #[test]
    fn a_wheel_with_buttons_past_the_table_counts_them_in_order() {
        let moza = super::key_bitmap_buttons("ffffffff ffffffffffffffff ffff000000000000 0 0 0 0 ffff00000000 0 0 0 0");
        assert_eq!(moza.len(), 128);
        if cfg!(target_os = "linux") {
            assert_eq!(super::button_index(&moza, 0x120), Some(0));
            assert_eq!(super::button_index(&moza, 0x12f), Some(15));
            assert_eq!(super::button_index(&moza, 0x270), Some(16));
            assert_eq!(super::button_index(&moza, 0x2c0), Some(96));
            let pad: Vec<u32> = vec![0x130, 0x131, 0x133, 0x134];
            assert_eq!(super::button_index(&pad, 0x133), None);
        }
    }

    #[test]
    fn a_device_lists_as_many_buttons_as_its_highest_number() {
        let moza = super::key_bitmap_buttons("ffffffff ffffffffffffffff ffff000000000000 0 0 0 0 ffff00000000 0 0 0 0");
        assert_eq!(super::button_count(&moza), 128);
        if cfg!(target_os = "linux") {
            assert_eq!(super::button_count(&[0x130, 0x131, 0x133, 0x134]), 5);
        }
        assert_eq!(super::button_count(&[]), 0);
    }

    #[test]
    fn buttons_count_as_directinput_does() {
        if cfg!(target_os = "macos") {
            assert_eq!(super::code_button(0x9_0001), Some(0));
            assert_eq!(super::code_button(0x9_0010), Some(15));
        }
        if cfg!(target_os = "linux") {
            assert_eq!(super::code_button(0x1_0120), Some(0));
            assert_eq!(super::code_button(0x1_02c0), Some(16));
        }
    }

    #[test]
    fn the_wheel_is_pulled_to_the_middle_harder_at_speed() {
        let mut t = 0.0;
        let f = |kmh| super::FfInput { on: true, kmh, dt: 0.016, ..Default::default() };
        let slow = super::wheel_force(&f(0.0), 0.5, 0.5, &mut t, 1.0, 1.0);
        let fast = super::wheel_force(&f(60.0), 0.5, 0.5, &mut t, 1.0, 1.0);
        assert!(slow < 0.0 && fast < slow, "{slow} {fast}");
        // turned to the right, it is pushed left; turning, it is held back
        let turning = super::wheel_force(&f(0.0), 0.0, -0.05, &mut t, 1.0, 1.0);
        assert!(turning < 0.0);
    }

    #[test]
    fn steering_resistance_drops_as_the_bus_starts_rolling() {
        let mut t = 0.0;
        let f = |kmh| super::FfInput { on: true, kmh, dt: 0.016, ..Default::default() };
        let parked = super::wheel_force(&f(0.0), 0.2, 0.15, &mut t, 1.0, 0.0);
        let moving = super::wheel_force(&f(30.0), 0.2, 0.15, &mut t, 1.0, 0.0);
        assert!(parked < moving && moving < 0.0, "{parked} {moving}");
    }

    #[test]
    fn a_returning_wheel_is_not_stopped_by_parking_drag() {
        let mut t = 0.0;
        let f = super::FfInput { on: true, kmh: 20.0, dt: 0.016, ..Default::default() };
        let right = super::wheel_force(&f, 0.5, 0.55, &mut t, 1.0, 0.0);
        let left = super::wheel_force(&f, -0.5, -0.55, &mut t, 1.0, 0.0);
        assert!(right < 0.0 && left > 0.0, "{right} {left}");
    }

    #[test]
    fn a_real_turn_adds_aligning_torque_but_a_parked_bus_does_not() {
        let mut t = 0.0;
        let mut f = super::FfInput { on: true, kmh: 30.0, dt: 0.016, ..Default::default() };
        let straight = super::wheel_force(&f, 0.2, 0.2, &mut t, 1.0, 0.0);
        f.lateral_accel = 3.0;
        let right_turn = super::wheel_force(&f, 0.2, 0.2, &mut t, 1.0, 0.0);
        assert!(right_turn < straight, "{straight} {right_turn}");
        f.kmh = 0.0;
        let parked = super::wheel_force(&f, 0.2, 0.2, &mut t, 1.0, 0.0);
        f.lateral_accel = 0.0;
        let parked_without_accel = super::wheel_force(&f, 0.2, 0.2, &mut t, 1.0, 0.0);
        assert_eq!(parked, parked_without_accel);
    }

    #[test]
    fn steering_assist_lightens_turning_out_without_weakening_return() {
        let mut t = 0.0;
        let f = super::FfInput { on: true, kmh: 25.0, lateral_accel: 2.0, dt: 0.016, ..Default::default() };
        let turning_out = super::wheel_force(&f, 0.5, 0.48, &mut t, 1.0, 0.0);
        let returning = super::wheel_force(&f, 0.5, 0.52, &mut t, 1.0, 0.0);
        assert!(returning < turning_out && turning_out < 0.0, "{turning_out} {returning}");
    }

    #[test]
    fn centering_does_not_grow_linearly_to_full_lock() {
        let mut t = 0.0;
        let f = super::FfInput { on: true, kmh: 30.0, dt: 0.016, ..Default::default() };
        let quarter = super::wheel_force(&f, 0.25, 0.25, &mut t, 1.0, 0.0);
        let full = super::wheel_force(&f, 1.0, 1.0, &mut t, 1.0, 0.0);
        assert!(full < quarter && full > 2.5 * quarter, "{quarter} {full}");
    }

    #[test]
    fn wheel_bumps_need_motion_and_a_suspension_or_impact_event() {
        assert_eq!(super::bump_strength(0.0, 0.0, 20.0), 0.0);
        assert_eq!(super::bump_strength(1.0, 0.0, 0.0), 0.0);
        assert_eq!(super::bump_strength(0.08, 0.08, 20.0), 0.0);
        assert!(super::bump_strength(0.5, 0.0, 20.0) > 0.3);
        assert!(super::bump_strength(1.0, 0.0, 20.0) > 0.5);
        assert!(super::bump_strength(0.0, 1.0, 20.0) > 0.5);
    }

    #[test]
    fn a_wheel_bump_uses_the_device_vibration_strength() {
        let mut t = 0.0;
        let f = super::FfInput { on: true, kmh: 20.0, wheel_bump: 1.0, dt: 0.016, ..Default::default() };
        let off = super::wheel_force(&f, 0.0, 0.0, &mut t, 0.0, 0.0);
        t = 0.0;
        let on = super::wheel_force(&f, 0.0, 0.0, &mut t, 0.0, 1.0);
        assert_eq!(off, 0.0);
        assert!(on.abs() > 0.15, "{on}");
        assert!(on.abs() < 0.5, "{on}");
        let lighter = super::wheel_force(&super::FfInput { wheel_bump: 0.25, ..f }, 0.0, 0.0, &mut t, 0.0, 1.0);
        assert!(lighter.abs() > on.abs() * 0.45, "{lighter} {on}");
        t = 0.37;
        let at_impact = super::wheel_force(&f, 0.0, 0.0, &mut t, 0.0, 1.0);
        assert!((on - at_impact).abs() < 0.001, "{on} {at_impact}");
    }

    #[test]
    fn active_turning_feels_firmer_at_road_speed_than_in_town() {
        let mut t = 0.0;
        let f = |kmh| super::FfInput { on: true, kmh, dt: 0.016, ..Default::default() };
        let town = super::wheel_force(&f(10.0), 0.5, 0.48, &mut t, 1.0, 0.0);
        let road = super::wheel_force(&f(70.0), 0.5, 0.48, &mut t, 1.0, 0.0);
        assert!(town < 0.0 && road < town * 1.4, "{town} {road}");
    }

    #[test]
    fn steering_force_scale_changes_the_constant_force() {
        let mut t = 0.0;
        let f = super::FfInput { on: true, kmh: 30.0, dt: 0.016, ..Default::default() };
        let zero = super::wheel_force(&f, 0.4, 0.4, &mut t, 0.0, 0.0);
        let normal = super::wheel_force(&f, 0.4, 0.4, &mut t, 1.0, 0.0);
        assert_eq!(zero, 0.0);
        assert!(normal.abs() > 0.05, "{normal}");
    }
}
