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
//!
//! The force feedback is what a heavy vehicle needs (see `Micro` and `Controllers::feedback`):
//! the parking resistance and centring of a bus that weighs twelve tonnes, and over the top
//! of it what it drives over and runs on - the grain of the road, the engine's buzz, a kerb
//! and whatever a script shakes the wheel with - every vibration that comes to an end eased
//! away rather than cut off.

use crate::gamepad_profile::{self, PadKind};
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
    /// The driver's head turned left and right, up and down (#454; openOMSI's own: OMSI's
    /// file has the five above, numbered 0 to 4).
    LookX,
    LookY,
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
            Some(Func::LookX) => 5,
            Some(Func::LookY) => 6,
        }
    }

    pub(crate) fn from_code(c: i32) -> Option<Func> {
        match c {
            0 => Some(Func::Steering),
            1 => Some(Func::Throttle),
            2 => Some(Func::Brake),
            3 => Some(Func::Clutch),
            4 => Some(Func::ThrottleBrake),
            5 => Some(Func::LookX),
            6 => Some(Func::LookY),
            _ => None,
        }
    }

    /// As the options dialog lists them.
    pub(crate) const LABELS: [&'static str; 8] = ["<none>", "Steering", "Throttle", "Brake", "Clutch", "Throttle/Brake", "Look left / right", "Look up / down"];
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
    /// Motor polarity for this device; None uses the existing global setting.
    pub(crate) ff_invert: Option<bool>,
    /// `[openOMSI.Latching]`: the buttons (from 0) that are latching switches - a turn signal
    /// lever, a lit hazard button - and switch back when they come out.
    pub(crate) latching: Vec<usize>,
}

/// The `gamectrler.cfg` in use: the content folder's (written by the launcher) before
/// OMSI 2's own.
pub(crate) fn cfg_path(root: &Path) -> std::path::PathBuf {
    omsi_cfg::find_in_roots("Inputs/gamectrler.cfg").map(|(_, p)| p).unwrap_or_else(|| root.join("Inputs").join("gamectrler.cfg"))
}

/// `Inputs/gamectrler.cfg`: the configured devices.
pub(crate) fn read_cfg(root: &Path) -> Vec<DeviceCfg> {
    read_cfg_checked(root).unwrap_or_default()
}

pub(crate) fn read_cfg_checked(root: &Path) -> Result<Vec<DeviceCfg>, String> {
    let path = cfg_path(root);
    let text = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
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
    Ok(devices)
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
            "[openOMSI.Latching]" => {
                if let Some(d) = out.last_mut() {
                    // (button numbers as the launcher shows them, from 1)
                    d.latching = lines.get(i + 1).unwrap_or(&"").split_whitespace().filter_map(|v| v.parse::<usize>().ok()?.checked_sub(1)).collect();
                }
                i += 2;
            }
            "[openOMSI.FFInvert]" => {
                if let Some(d) = out.last_mut() {
                    d.ff_invert = lines.get(i + 1).and_then(|v| match *v { "0" => Some(false), "1" => Some(true), _ => None });
                }
                i += 2;
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
        if let Some(invert) = d.ff_invert {
            t.push_str(&format!("[openOMSI.FFInvert]\r\n{}\r\n\r\n", invert as u8));
        }
        if !d.latching.is_empty() {
            let numbers: Vec<String> = d.latching.iter().map(|b| (b + 1).to_string()).collect();
            t.push_str(&format!("[openOMSI.Latching]\r\n{}\r\n\r\n", numbers.join(" ")));
        }
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
    /// How far the head is to turn this moment, right and down (-1 .. 1 each): a set-up
    /// axis that looks round, or a gamepad's right stick (#454).
    pub look: [f32; 2],
}

impl Analog {
    /// Automatic gamepad camera input leaves explicitly assigned look axes and driving
    /// controls alone, including when the automatic right-stick input is switched off.
    fn apply_default_gamepad_look(&mut self, enabled: bool, x: f32, y: f32) {
        if enabled && self.look == [0.0, 0.0] {
            self.look = [look_axis(x), look_axis(-y)];
        }
    }
}

/// Below this a steering value counts as the wheel at its centre (see `stick_steers`).
const CENTRE_SNAP: f32 = 0.03;

/// How far the X axis of a device nobody set up must leave its centre before it steers.
const FREE_AXIS_MOVED: f32 = 0.15;

/// Whether the X axis (at `x`) of a device nobody set up steers: once it has left its
/// centre, and from then on (`moved` keeps the devices that have).
fn free_axis_steers(moved: &mut Vec<String>, name: &str, x: f32) -> bool {
    if moved.iter().any(|n| n == name) {
        return true;
    }
    if x.abs() < FREE_AXIS_MOVED {
        return false;
    }
    log::info!("game controller {name}: its X axis moved, it steers now");
    moved.push(name.to_string());
    true
}

/// Whether a pad's left stick at `x` steers, given what steers already (`current`) and whether
/// that is a device set up to steer. Only the first device used to: an idle joystick, wheel or
/// virtual pad nobody set up (its X axis lends the steering) held the wheel at its centre and
/// the stick did nothing (#1165). Now the stick steers unless a set-up device has the wheel,
/// whenever it is pushed further than that device or the device lies at its centre.
fn stick_steers(current: Option<f32>, set_up: bool, x: f32) -> bool {
    match current {
        None => true,
        Some(s) => !set_up && (x.abs() > s.abs() || s.abs() < CENTRE_SNAP),
    }
}

/// An axis that turns the head: nothing round its centre, then the rest of the way.
pub(crate) fn look_axis(v: f32) -> f32 {
    const DEAD: f32 = 0.12;
    if v.abs() <= DEAD { 0.0 } else { v.signum() * (v.abs() - DEAD) / (1.0 - DEAD) }
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

fn mapped_device_is_gamepad(mapped: bool, force_feedback_wheel: bool) -> bool {
    mapped && !force_feedback_wheel
}

/// A gamepad stick's dead zone: nothing round its centre, then the rest of the way from
/// nothing - it used to cut off below 0.08 and jump straight to 0.08 past it.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn stick_deadzone(x: f32) -> f32 {
    stick_deadzone_with(x, 0.08)
}

/// `stick_deadzone` with the dead zone of the settings (`pad_deadzone`).
pub(crate) fn stick_deadzone_with(x: f32, dead: f32) -> f32 {
    let dead = dead.clamp(0.0, 0.4);
    if x.abs() <= dead { 0.0 } else { x.signum() * ((x.abs() - dead) / (1.0 - dead)).min(1.0) }
}

/// `current` eased towards `target` over a time constant `tau` (s), the same whatever the
/// frame rate; `tau` 0 is the target at once. A stick's few hundredths of wobble round
/// where the thumb rests went into the bus's curvature frame by frame.
pub fn smooth_toward(current: f32, target: f32, dt: f32, tau: f32) -> f32 {
    if tau <= 0.0 || !current.is_finite() {
        return target;
    }
    current + (target - current) * (1.0 - (-dt.max(0.0) / tau).exp())
}

/// Bus steering follows its characteristic, dead zone and range; feedback follows the physical
/// wheel position, so it can keep returning even inside the input dead zone.
fn wheel_steering(axis: f32, reversed: bool, flags: i32, deadzone: f32, gain: f32) -> (f32, f32) {
    let position = if reversed { -axis } else { axis };
    let shaped = axis_shape((position + 1.0) * 0.5, flags) * 2.0 - 1.0;
    let deadzone = deadzone.clamp(0.0, 0.3);
    let steering = shaped.signum() * ((shaped.abs() - deadzone).max(0.0) / (1.0 - deadzone)) * gain;
    (steering.clamp(-1.0, 1.0), position.clamp(-1.0, 1.0))
}

/// A device connected now: its name, its axes (DirectInput slot, -1..1), whether the system
/// knows it as a gamepad (a known layout of sticks and triggers), and whether it can push
/// back (force feedback).
#[derive(Debug, Clone)]
pub(crate) struct Connected {
    pub name: String,
    pub hardware_id: Option<(u16, u16)>,
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
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    calibration_wheel: Option<crate::evdev_ff::Wheel>,
    /// Whether a gilrs device is also a native evdev constant-force wheel.
    /// `connected()` runs every frame, so cache the /sys lookup by device name.
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    ff_wheels: std::cell::RefCell<std::collections::HashMap<String, bool>>,
    /// Linux: devices with buttons only (a gear shifter), which gilrs does not list
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    button_devices: crate::evdev_buttons::ButtonDevices,
    #[cfg(windows)]
    di: Option<crate::dinput::DirectInput>,
    /// macOS: Xbox-type controllers, which gilrs and `hid` cannot read at all (see
    /// `mac_game_controller`).
    #[cfg(target_os = "macos")]
    gc: crate::mac_game_controller::GcPads,
    /// macOS: every axis element of every wheel and joystick, as last read (see `mac_hid`)
    #[cfg(target_os = "macos")]
    hid: Option<crate::mac_hid::MacHid>,
    #[cfg(target_os = "macos")]
    hid_axes: Vec<crate::mac_hid::HidAxes>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    hats: Vec<(String, [i8; 8])>,
    /// The buttons of the pads gilrs knows a layout of, by what they are (south, D-pad left)
    /// and not by number: what `gamepad_profile` binds, whatever the system numbers them.
    pad_buttons: Vec<(String, gilrs::Button, bool)>,
}

impl Devices {
    /// `hwnd`: the window (Windows: the devices belong to it); `ff`: take them for force
    /// feedback (the game, not the launcher).
    pub fn new(hwnd: Option<isize>, ff: bool) -> Devices {
        // without gilrs's default filters: its dead zone took 10 % of every axis - on a
        // wheel of 1800 degrees, 90 degrees either side of the middle did nothing - and its
        // jitter filter held back small movements; the settings' dead zone is the only one
        // Linux: gilrs takes a wheel with periodic effects for a rumbling gamepad and starts
        // a rumble effect on it every 50 ms, even at strength 0 - a HID PID wheel's motor
        // kicks on every start and the wheel buzzes. A wheel's forces go through evdev_ff,
        // so gilrs's force feedback stays off while one is connected.
        #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
        let gilrs_ff = !crate::evdev_ff::wheel_connected();
        #[cfg(not(all(target_os = "linux", target_pointer_width = "64")))]
        let gilrs_ff = true;
        let gilrs = gilrs::GilrsBuilder::new().with_default_filters(false).with_force_feedback(gilrs_ff).build().map_err(|e| log::info!("game controllers: {e}")).ok();
        #[cfg(windows)]
        let di = hwnd.and_then(|h| crate::dinput::DirectInput::new(h, ff));
        #[cfg(not(windows))]
        let _ = (hwnd, ff);
        Devices {
            gilrs,
            #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
            calibration_wheel: None,
            #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
            ff_wheels: Default::default(),
            #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
            button_devices: crate::evdev_buttons::ButtonDevices::new(),
            #[cfg(windows)]
            di,
            #[cfg(target_os = "macos")]
            gc: crate::mac_game_controller::GcPads::new(),
            #[cfg(target_os = "macos")]
            hid: crate::mac_hid::MacHid::new(),
            #[cfg(target_os = "macos")]
            hid_axes: Vec::new(),
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            hats: Vec::new(),
            pad_buttons: Vec::new(),
        }
    }

    /// macOS: the HID device that is this gilrs pad (see `same_hid_device`).
    #[cfg(target_os = "macos")]
    fn hid_of(&self, pad: &gilrs::Gamepad<'_>) -> Option<&crate::mac_hid::HidAxes> {
        let id = pad.vendor_id().zip(pad.product_id());
        self.hid_axes.iter().find(|h| same_hid_device(&h.name, h.id, [pad.name(), pad.os_name()], id))
    }

    /// macOS: the pad's HID device has the axes of a wheel or pedals (a slider, a dial, or
    /// the simulation page's steering, accelerator, brake, clutch).
    #[cfg(target_os = "macos")]
    pub(crate) fn hid_wheel(&self, pad: &gilrs::Gamepad<'_>) -> bool {
        self.hid_of(pad).is_some_and(|h| h.axes.iter().any(|(c, _)| matches!(*c, 0x10036 | 0x10037) || (*c >> 16) == 2))
    }

    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    fn linux_ff_wheel(&self, name: &str) -> bool {
        let mut wheels = self.ff_wheels.borrow_mut();
        *wheels.entry(name.to_string()).or_insert_with(|| crate::evdev_ff::wheel_named(name))
    }

    fn direct_input(&self) -> bool {
        #[cfg(windows)]
        return self.di.is_some();
        #[cfg(not(windows))]
        false
    }

    /// A device of buttons only (a gear shifter, a button box): nothing for the axis assistant.
    pub(crate) fn buttons_only(&self, name: &str) -> bool {
        #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
        return self.button_devices.connected().any(|(n, _)| names_match(n, name));
        #[cfg(not(all(target_os = "linux", target_pointer_width = "64")))]
        { let _ = name; false }
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
        #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
        if !focused {
            self.calibration_wheel = None;
        }
        #[cfg(windows)]
        if let Some(d) = self.di.as_mut() {
            d.set_focus(focused);
        }
        #[cfg(not(windows))]
        let _ = focused;
    }

    pub(crate) fn calibration_pulse(&mut self, name: &str, axis: usize, force: f32) -> bool {
        #[cfg(windows)]
        return self.di.as_mut().is_some_and(|di| di.force_axis(name) == Some(axis) && di.pulse_force(name, force));
        #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
        {
            let _ = axis;
            if self.calibration_wheel.is_none() {
                self.calibration_wheel = crate::evdev_ff::Wheel::open(name);
            }
            return self.calibration_wheel.as_mut().is_some_and(|wheel| wheel.pulse_force(force));
        }
        #[cfg(not(any(windows, all(target_os = "linux", target_pointer_width = "64"))))]
        { let _ = (name, axis, force); false }
    }

    /// Read the devices; the buttons pressed (true) and let go since the last call:
    /// (device, button number from 0, as DirectInput and `gamectrler.cfg` count them).
    pub fn poll(&mut self) -> Vec<(String, usize, bool)> {
        let mut out = Vec::new();
        self.pad_buttons.clear();
        let di = self.direct_input();
        #[cfg(windows)]
        let xinput_pads = self.gilrs.as_ref().is_some_and(|g| g.gamepads().any(|(_, p)| xinput_name(p.name())));
        #[cfg(windows)]
        let is_di = |pad: &gilrs::Gamepad<'_>| -> bool {
            let id = pad.vendor_id().zip(pad.product_id());
            self.di.as_ref().is_some_and(|d| {
                d.devices.iter().any(|dev| {
                    include_direct_input_device(&dev.name, dev.ff_capable(), xinput_pads)
                        && (names_match(&dev.name, pad.name()) || id.is_some_and(|id| dev.hardware_id == Some(id)))
                })
            })
        };
        #[cfg(not(windows))]
        let is_di = |_pad: &gilrs::Gamepad<'_>| -> bool { false };

        if let Some(g) = self.gilrs.as_mut() {
            while let Some(ev) = next_event_caught(|| g.next_event()) {
                // A focus/device change can leave a queued gilrs event pointing at a
                // device that has already been removed. `gamepad` panics in that case.
                let Some(pad) = g.connected_gamepad(ev.id) else {
                    continue;
                };
                if let EventType::ButtonPressed(b, _) | EventType::ButtonReleased(b, _) = ev.event {
                    if pad.mapping_source() != gilrs::MappingSource::None {
                        self.pad_buttons.push((pad.name().to_string(), b, matches!(ev.event, EventType::ButtonPressed(..))));
                    }
                }
                match ev.event {
                    EventType::Connected => log::info!("game controller connected: {} (layout {:?}, DirectInput {})", pad.name(), pad.mapping_source(), di),
                    // DirectInput handles wheels on Windows; system-mapped gamepads
                    // such as Xbox controllers are listed through gilrs.
                    EventType::ButtonPressed(_, code) | EventType::ButtonReleased(_, code)
                        if use_gilrs_buttons(di, is_system_gamepad(pad.name(), is_di(&pad))) => {
                        if let Some(n) = button_number(&pad, code) {
                            out.push((pad.name().to_string(), n, matches!(ev.event, EventType::ButtonPressed(..))));
                        }
                    }
                    // (the hat switches: a wheel rim's, a gamepad's D-pad - on macOS gilrs
                    // turns every hat into two axes and, its filters off, never into buttons)
                    #[cfg(any(target_os = "linux", target_os = "macos"))]
                    EventType::AxisChanged(axis_name, value, code) if hat_axis(code.into_u32(), value, cfg!(target_os = "macos") && axis_name == Axis::DPadY).is_some() => {
                        let Some((axis, value)) = hat_axis(code.into_u32(), value, cfg!(target_os = "macos") && axis_name == Axis::DPadY) else { continue };
                        let name = pad.name().to_string();
                        let k = match self.hats.iter().position(|(n, _)| *n == name) {
                            Some(k) => k,
                            None => {
                                self.hats.push((name.clone(), [0; 8]));
                                self.hats.len() - 1
                            }
                        };
                        let now = if value > 0.5 { 1 } else if value < -0.5 { -1 } else { 0 };
                        let was = std::mem::replace(&mut self.hats[k].1[axis], now);
                        let (hat, y) = (axis / 2, axis % 2 == 1);
                        let dir = |v: i8| match (y, v) {
                            (true, -1) => Some(0),
                            (false, 1) => Some(1),
                            (true, 1) => Some(2),
                            (false, -1) => Some(3),
                            _ => None,
                        };
                        if was != now {
                            if let Some(d) = dir(was) {
                                out.push((name.clone(), HAT_BUTTONS + hat * 4 + d, false));
                            }
                            if let Some(d) = dir(now) {
                                out.push((name, HAT_BUTTONS + hat * 4 + d, true));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        #[cfg(windows)]
        if let Some(d) = self.di.as_mut() {
            d.poll();
            out.extend(d.events.drain(..).filter(|(name, ..)| {
                d.devices.iter().find(|dev| &dev.name == name)
                    .map_or(true, |dev| include_direct_input_device(&dev.name, dev.ff_capable(), xinput_pads))
            }));
        }
        #[cfg(target_os = "macos")]
        if let Some(h) = self.hid.as_mut() {
            self.hid_axes = h.read();
        }
        #[cfg(target_os = "macos")]
        out.extend(self.gc.poll());
        #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
        self.button_devices.poll(&mut out);
        out
    }

    /// The devices connected, with their axes as last read.
    pub fn connected(&self) -> Vec<Connected> {
        let mut v = Vec::new();
        // (Windows: an Xbox-type pad is gilrs's - the system's own layout -, everything else
        // DirectInput's; a wheel that a community mapping makes a "gamepad" in gilrs was
        // listed twice, "Logitech G29" beside "G29 Driving Force Racing Wheel")
        let xinput_pads = self.gilrs.as_ref().is_some_and(|g| g.gamepads().any(|(_, p)| xinput_name(p.name())));
        #[cfg(windows)]
        if let Some(d) = self.di.as_ref() {
            v.extend(
                d.devices
                    .iter()
                    .filter(|_| d.is_focused())
                    // A G920's DirectInput name contains "Xbox One", but it is the
                    // force-feedback wheel. Keep it even when gilrs also lists a pad.
                    .filter(|d| include_direct_input_device(&d.name, d.ff_capable(), xinput_pads))
                    .map(|d| Connected {
                        name: d.name.clone(),
                        hardware_id: d.hardware_id,
                        axes: d.axes(),
                        // (a DualShock or DualSense: a gamepad, not a joystick nobody set up)
                        gamepad: sony_pad(&d.name, d.hardware_id),
                        ff: d.has_ff(),
                        ff_capable: d.ff_capable(),
                        buttons: d.buttons.min(128),
                    }),
            );
        }
        let _ = xinput_pads;
        if let Some(g) = self.gilrs.as_ref() {
            for (_, pad) in g.gamepads() {
                let mapped = pad.mapping_source() != gilrs::MappingSource::None;
                #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
                let force_feedback_wheel = mapped && self.linux_ff_wheel(pad.name());
                #[cfg(not(all(target_os = "linux", target_pointer_width = "64")))]
                let force_feedback_wheel = false;
                #[allow(unused_mut)]
                let mut gamepad = mapped_device_is_gamepad(mapped, force_feedback_wheel);
                // (macOS: a device with sliders or the simulation page's axes is a wheel or
                // pedals, whatever SDL's list calls it - the HORI Truck Control System was
                // taken as a gamepad: its left stick steered, with a gamepad's dead zone)
                #[cfg(target_os = "macos")]
                if self.hid_wheel(&pad) {
                    gamepad = false;
                }
                let id = pad.vendor_id().zip(pad.product_id());
                let is_di = v.iter().any(|c: &Connected| names_match(&c.name, pad.name()) || (id.is_some() && c.hardware_id == id));
                if self.direct_input() && (is_di || !xinput_name(pad.name())) {
                    continue;
                }
                if v.iter().any(|c: &Connected| names_match(&c.name, pad.name())) {
                    continue;
                }
                #[allow(unused_mut)]
                let mut axes: Vec<(u32, f32)> = pad.state().axes().map(|(c, d)| (c.into_u32(), d.value())).collect();
                // (macOS: the device's own axis elements where it is found among them - two
                // of one usage stay two, and a gamepad's analog triggers are axes: gilrs makes
                // them buttons from its SDL mapping, and the DualSense's L2/R2 - Rx, Ry - were
                // missing from "PS5 Controller" and listed on a second device of the same pad)
                #[cfg(target_os = "macos")]
                if let Some(h) = self.hid_of(&pad) {
                    axes = h.axes.clone();
                }
                #[cfg(target_os = "linux")]
                let buttons = declared_button_count(pad.name());
                #[cfg(not(target_os = "linux"))]
                let buttons = 0;
                v.push(Connected { name: pad.name().to_string(), hardware_id: id, axes: {
                    let mut slots = di_slots(&axes);
                    if cfg!(windows) && xinput_name(pad.name()) {
                        let trigger = |b| pad.button_data(b).map(|d| d.value()).unwrap_or(0.0);
                        gamepad_triggers(&mut slots, trigger(gilrs::Button::LeftTrigger2), trigger(gilrs::Button::RightTrigger2));
                    }
                    slots
                }, gamepad, ff: pad.is_ff_supported(), ff_capable: pad.is_ff_supported(), buttons });
            }
        }
        // (and a wheel gilrs does not list at all: one whose only axes are the simulation
        // page's steering and pedals - not a device gilrs lists under another name: a
        // DualSense was "PS5 Controller" with its buttons and "DualSense Wireless Controller"
        // with its axes and no buttons, and two devices were one pad)
        #[cfg(target_os = "macos")]
        for h in &self.hid_axes {
            let listed = self.gilrs.as_ref().is_some_and(|g| g.gamepads().any(|(_, pad)| same_hid_device(&h.name, h.id, [pad.name(), pad.os_name()], pad.vendor_id().zip(pad.product_id()))));
            if !listed && !v.iter().any(|c| names_match(&c.name, &h.name)) {
                v.push(Connected { name: h.name.clone(), hardware_id: h.id, axes: di_slots(&h.axes), gamepad: false, ff: false, ff_capable: false, buttons: 0 });
            }
        }
        // (an Xbox-type controller: gilrs lists it too, as a generic "Controller" with no
        // axis or button at all - Apple's own driver leaves nothing else for it to read.
        // That name is left in the list too: it fuzzy-matches this one's by `names_match`
        // - "Controller" is a substring of "Xbox Controller" - and would otherwise look
        // like this device already had a usable entry and lose it)
        #[cfg(target_os = "macos")]
        for (name, axes) in self.gc.connected() {
            if !v.iter().any(|c| c.name == name) {
                v.push(Connected { name, hardware_id: None, axes, gamepad: true, ff: false, ff_capable: false, buttons: 0 });
            }
        }
        #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
        for (name, buttons) in self.button_devices.connected() {
            if !v.iter().any(|c| names_match(&c.name, name)) {
                v.push(Connected { name: name.to_string(), hardware_id: None, axes: Vec::new(), gamepad: false, ff: false, ff_capable: false, buttons });
            }
        }
        v
    }
}

/// What a latching switch fires when it comes out: a turn signal set goes off, the parking
/// brake set is released, anything else (a toggle) fires once more and so switches back.
fn latch_release(action: &str) -> String {
    match action.to_ascii_lowercase().as_str() {
        "blinker_left_set" | "blinker_right_set" => "blinker_off".to_string(),
        "parking_brake_set" => "parking_brake_release".to_string(),
        _ => action.to_string(),
    }
}

/// The next event of gilrs (`next`), past any that panics inside it: gilrs 0.11 on Windows
/// can hand on a button or axis of a controller before it reports the controller connected
/// (one that appears while the game runs) and then indexes past its own list of them
/// (gilrs#206) - it ended the game, through the window procedure, at `gamepad.rs:474`.
/// Only that one event is lost: the controller works once its `Connected` comes.
fn next_event_caught<T>(mut next: impl FnMut() -> Option<T>) -> Option<T> {
    loop {
        match omsi_render::catch(&mut next) {
            Some(ev) => return ev,
            None => {
                static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    log::warn!("game controllers: gilrs stopped on an event of a controller it does not list yet; skipped");
                }
            }
        }
    }
}

/// macOS: a HID device and a gilrs pad are one device when they have the same (vendor,
/// product), or the same name - gilrs's (an SDL mapping's, "PS5 Controller") or the
/// system's (`pad_names`: both).
#[cfg(any(target_os = "macos", test))]
fn same_hid_device(hid_name: &str, hid_id: Option<(u16, u16)>, pad_names: [&str; 2], pad_id: Option<(u16, u16)>) -> bool {
    (hid_id.is_some() && hid_id == pad_id) || pad_names.iter().any(|n| names_match(hid_name, n))
}

/// A hat switch's axis as gilrs reports it: its number (hat * 2, + 1 for up/down) and its
/// value, right and down positive. Linux: evdev's ABS_HAT0X..ABS_HAT3Y. macOS: gilrs makes
/// a hat two axes, 0x39 and 0x3A of the generic desktop page, and turns the up/down one
/// round when an SDL mapping names it the D-pad's (`mapped_y`).
#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn hat_axis(code: u32, value: f32, mapped_y: bool) -> Option<(usize, f32)> {
    // (the two kinds of code cannot meet: gilrs's axes on Linux are all of evdev's EV_ABS, 3)
    match (code >> 16, code & 0xFFFF) {
        (1, 0x39) => Some((0, value)),
        (1, 0x3A) => Some((1, if mapped_y { -value } else { value })),
        (3, lo @ 0x10..=0x17) => Some(((lo - 0x10) as usize, value)),
        _ => None,
    }
}

/// macOS: the code of an axis (the axes `mac_hid` reads, and a hat), which gilrs hands on
/// as a button when an SDL mapping makes it one - a gamepad's analog trigger.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn mac_axis_code(code: u32) -> bool {
    matches!((code >> 16, code & 0xFFFF), (1, 0x30..=0x3A) | (2, 0xBA | 0xBB | 0xC4 | 0xC5 | 0xC6 | 0xC8))
}

/// A PlayStation pad (the Xbox ones are the system's own gamepads, see `xinput_name`).
#[cfg_attr(not(windows), allow(dead_code))]
fn sony_pad(name: &str, id: Option<(u16, u16)>) -> bool {
    matches!(PadKind::detect(name, id), Some(PadKind::Ps4 | PadKind::Ps5))
}

fn use_gilrs_buttons(direct_input: bool, system_gamepad: bool) -> bool {
    !direct_input || system_gamepad
}

pub(crate) fn is_system_gamepad(name: &str, is_di_device: bool) -> bool {
    !is_di_device && xinput_name(name)
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
    /// `StreetCond` of the road under the wheels: 0 dry, 1 wet, 2 covered in snow.
    pub street_cond: f32,
    /// The engine's speed (rpm; 0 while it is not running).
    pub engine_rpm: f32,
    /// How hard the engine is working (0..1).
    pub engine_load: f32,
    /// The trembling of the tarmac and the engine, worked out here from the four above
    /// (see `Micro`); the caller leaves it at 0.
    pub(crate) micro: f32,
    pub dt: f32,
}

/// How long (s) a jolt's or a script's vibration eases away once it stops, unless the
/// settings say otherwise (`ff_fade`).
const FF_FADE: f32 = 0.28;

/// What is left of a vibration `t` seconds after its source stopped: all of it while it is
/// still coming, then easing away over `fade` seconds. A force that falls to zero in a
/// single step is a jolt of its own - the rim feels it and the wheel's motor clunks - so
/// everything that shakes the wheel once eases out rather than letting go. `fade` 0 is the
/// old behaviour of stopping where it stands.
fn fade_gain(t: f32, fade: f32) -> f32 {
    if fade <= 0.0 {
        return 1.0;
    }
    if t >= fade {
        return 0.0;
    }
    // a smoothstep, so the fade neither starts nor ends with a step of its own
    let k = 1.0 - t / fade;
    k * k * (3.0 - 2.0 * k)
}

/// A script's shaking (`FF_Vib_Amp` and `FF_Vib_Period`) and the little state its fade
/// needs. The scripts write both afresh every frame, so once they stop there is nothing
/// left to ease away unless the last of what they asked for is kept here.
#[derive(Debug, Clone, Copy, Default)]
struct ScriptVib {
    /// The last amplitude the scripts reached the wheel with.
    amp: f32,
    /// The period that came with it: the scripts stop writing that too, and a rattle eased
    /// away at the wrong period would not be the one that stopped.
    period: f32,
    /// How long ago (s) they last reached the wheel.
    t: f32,
}

impl ScriptVib {
    /// What the wheel is told this frame: the retained amplitude eased on the settings'
    /// fade since the scripts went quiet, and the period to shake it at.
    ///
    /// The fade cannot be a gain applied to the incoming amplitude, because that amplitude
    /// is already zero on the frame the scripts stop and any gain leaves zero at zero - the
    /// wheel then drops the shake in a single step, which is the jolt the fade is here to
    /// avoid. It shows on the DirectInput path, where a zero amplitude stops the periodic
    /// effect outright, and on the fallback, whose shake is worked out of the same
    /// amplitude. The fade at zero stops the shake where it stands, as it always did.
    fn step(&mut self, on: bool, incoming: f32, period: f32, fade: f32, dt: f32) -> (f32, f32) {
        let incoming = if on { incoming.clamp(0.0, 1.0) } else { 0.0 };
        if incoming > 0.004 {
            self.amp = incoming;
            if period > 0.0 {
                self.period = period;
            }
            self.t = 0.0;
        } else if on && fade > 0.01 {
            self.t += dt.max(0.0);
        } else {
            self.amp = 0.0;
            self.t = 0.0;
        }
        (self.amp * fade_gain(self.t, fade), self.period)
    }
}

/// The trembling that never stops while the bus runs: the grain of the tarmac under the
/// wheels, and the engine's own buzz coming up through the frame.
///
/// Both are small - a tenth of the wheel's lock at most, and a wheel with a real motor
/// spends most of its travel on parking resistance anyway - and both are fast enough to be
/// felt, which is why a wheel feels them as a buzz and not as a push. A constant-force
/// wheel can only be told one force at a time; a force that changes every few milliseconds
/// comes out of the motor as a tremble rather than as a movement of the wheel, which is
/// what the road is. A slow one is felt as nothing at all: the rim goes where it is told and
/// the hands go with it, so the rate matters more here than the size. And a force that lines
/// up with itself is felt as something solid being carried while one that does not is felt
/// as a loose surface, so most of the road is sines and only a little of it is noise.
///
/// It keeps a little state of its own: the distance the bus has rolled (the grain is read
/// along the road, so a bus standing still feels none of it and the same stretch of tarmac
/// shakes the same way twice) and the engine's angle (so its buzz is at the engine's own
/// rate whatever gear the bus is in, and a script's rpm that twitches does not reach the
/// wheel as a rattle).
#[derive(Debug, Clone, Copy, Default)]
struct Micro {
    /// The distance the bus has rolled (m).
    roll: f32,
    /// The engine's speed, with the twitches of the script that writes it taken out (rpm).
    rpm: f32,
    /// Where each part of the engine's buzz stands (rad). Each is kept on its own because a
    /// phase reduced on its own rate is exact for that rate, and a phase shared between them
    /// is not: wrap the crank angle into a single turn, then take half or a quarter of it,
    /// and you land on a different half turn every time instead of carrying on.
    crank: f32,
    firing: f32,
    mass: f32,
}

impl Micro {
    /// How much the wheel trembles this frame, of its full lock (-1..1). `road` and
    /// `engine` are the settings' strengths (0 = neither); the rest is what the bus is
    /// doing - `kmh`, the road's `street_cond` (0 dry, 1 wet, 2 snow), the engine's rpm and
    /// how hard it is working.
    fn sample(&mut self, dt: f32, kmh: f32, street_cond: f32, rpm: f32, load: f32, road: f32, engine: f32) -> f32 {
        let dt = dt.clamp(0.0, 0.1);
        // The grain of the road: nothing at all at a standstill, all of it by 30 km/h, and
        // a wet or snowy road hums under the tyres more than dry asphalt does.
        let v = kmh.abs() / 3.6;
        let drive = (v / 8.5).min(1.0);
        let rough = 0.6 + 0.25 * street_cond.clamp(0.0, 2.0);
        self.roll += v * dt;
// The road, read along the distance: mostly its ride - the shapes the whole bus is
        // being carried over - and only a seventh its grain. A wheel is told one force at a
        // time, and the difference the driver feels between a bus that feels heavy and one
        // whose wheel rattles is whether that force lines up with itself. Noise never does:
        // read along the road it gives every part of the spectrum a share, and a signal with
        // no shape to it is felt as a loose surface however fast it is felt. A ride made of
        // sines whose lengths do not fit together lines up and never repeats over a stretch
        // of road, and that is what reads as one solid weight being carried. The grain is
        // left in under it, small, because it is what says the road is made of something and
        // not polished. The three lengths are six metres, two and a quarter and nine
        // tenths: the deep ones are the mass, the last is what the tyre feels, and none of
        // them is so short that it comes back as a beat of its own before a wheel cannot be
        // told about it (a hundred times a second, so the 0.1 m grain is let go above 40 Hz,
        // where the 0.9 m one carries the surface on its own).
        let fine = 1.0 - (v / 0.4 / 40.0).clamp(0.0, 1.0);
        let shape = ride(self.roll / 6.0) * 0.34 + ride(self.roll / 2.2 + 5.3) * 0.26 + ride(self.roll / 0.9 + 11.9) * 0.4;
        let surface = grain(self.roll / 0.25) * 0.55 + grain(self.roll / 0.1 + 31.7) * 0.45 * fine;
        let tarmac = (shape * 0.85 + surface * 0.15) * drive * rough * 0.25 * road;
        // The engine's buzz, each part on the rate it really runs at: the crankshaft
        // turning, the firing pulses above it, and a low rumble under both standing for the
        // mass the frame works against. A wheel is told its force once a frame, so nothing
        // above about 28 Hz can be carried at all and each part is let go as it reaches for
        // it; above the revs where that has happened the crank and the pulses are both gone
        // and what is left is the low rumble.
        let turn = std::f32::consts::TAU * dt;
        self.rpm += (rpm.clamp(0.0, 4500.0) - self.rpm) * (dt / 0.08).min(1.0);
        let spin = |p: &mut f32, per_turn: f32| *p = (*p + self.rpm / per_turn * turn).rem_euclid(std::f32::consts::TAU);
        // a wheel is told its force once a frame, so nothing above about 28 Hz can be
        // carried at all and each part is let go over the last stretch before that
        let room = |f: f32| 1.0 - ((f - 28.0) / 7.0).clamp(0.0, 1.0);
        spin(&mut self.crank, 60.0);
        spin(&mut self.firing, 30.0);
        spin(&mut self.mass, 240.0);
        let running = ((self.rpm - 150.0) / 400.0).clamp(0.0, 1.0);
        // an engine working against its mounts twists harder than one idling
        let mount = 0.7 + 0.6 * load.clamp(0.0, 1.0);
        let lump = 0.6 + 0.4 * (self.rpm / 2400.0).clamp(0.0, 1.0);
        let hum = running * mount * lump
            * (self.crank.sin() * room(self.rpm / 60.0) * 0.03
                + self.firing.sin() * room(self.rpm / 30.0) * 0.02
                + self.mass.sin() * (self.rpm / 2400.0).min(1.0) * 0.055)
            * engine;
        tarmac + hum
    }
}

/// The ride of the road under the wheels: smooth, always moving, and never the same twice.
///
/// Where `grain` reads the surface, this reads the shape. It is sines whose lengths do not
/// fit into one another, so it never repeats itself over a stretch of road and never jumps
/// either. Its sum is 1 at the very most, so it can be mixed by eye.
fn ride(x: f32) -> f32 {
    let t = std::f32::consts::TAU * x;
    t.sin() * 0.55 + (t * 1.7 + 1.3).sin() * 0.3 + (t * 2.9 + 4.2).sin() * 0.15
}

/// Value noise along one line, -1..1. The grain of a road surface is not a sine wave, and
/// a wheel handed the same sine wave every few metres reads it as a whine from the gearbox
/// rather than as tarmac.
fn grain(x: f32) -> f32 {
    let i = x.floor();
    let f = x - i;
    let s = f * f * (3.0 - 2.0 * f); // the lattice's points, eased into one another
    let a = lattice(i as i32);
    let b = lattice(i as i32 + 1);
    (a + (b - a) * s) * 2.0 - 1.0
}

/// One point of the grain's lattice, hashed from its place in whole metres (so a long
/// drive does not walk the pattern off).
fn lattice(i: i32) -> f32 {
    let mut h = (i as u32).wrapping_mul(0x27d4_eb2d) ^ 0x9e37_79b1;
    h ^= h >> 15;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    (h >> 8) as f32 / 16_777_216.0
}


/// Button-down actions remember their original mapping until release. Editing a binding
/// must release that action, rather than sending an unrelated new action's key-up.
#[derive(Default)]
struct HeldButtons(Vec<(String, usize, String, bool)>);

impl HeldButtons {
    fn event(&mut self, cfg: &[DeviceCfg], name: &str, button: usize, down: bool, actions: &mut Vec<(String, bool)>) {
        if down {
            if self.0.iter().any(|(n, b, ..)| names_match(n, name) && *b == button) {
                return;
            }
            let Some(d) = find_device_cfg(cfg, name) else { return };
            let Some((action, _)) = d.buttons.get(button).filter(|a| !a.0.is_empty()) else { return };
            self.0.push((name.to_string(), button, action.clone(), d.latching.contains(&button)));
            actions.push((action.clone(), true));
        } else if let Some(i) = self.0.iter().position(|(n, b, ..)| names_match(n, name) && *b == button) {
            let (_, _, action, latching) = self.0.remove(i);
            actions.push((action.clone(), false));
            if latching {
                let back = latch_release(&action);
                actions.push((back.clone(), true));
                actions.push((back, false));
            }
        }
    }

    fn release(&mut self, actions: &mut Vec<(String, bool)>) {
        for (_, _, action, _) in self.0.drain(..) {
            // Do not toggle a latching switch just because its configuration changed.
            actions.push((action, false));
        }
    }
}

/// Save only to the writable openOMSI overlay. The original OMSI installation is read-only.
pub(crate) fn save_cfg(devices: &[DeviceCfg]) -> Result<(), String> {
    let dir = crate::startup::content_dir().ok_or("No writable openOMSI content folder was found")?.join("Inputs");
    save_cfg_to(&dir.join("gamectrler.cfg"), devices)?;
    omsi_cfg::content_changed();
    Ok(())
}

fn save_cfg_to(path: &Path, devices: &[DeviceCfg]) -> Result<(), String> {
    let text = cfg_text(devices);
    if cfg_text(&parse_cfg(&text)) != text {
        return Err("Controller configuration did not pass its round-trip check".into());
    }
    let dir = path.parent().ok_or("Controller configuration has no parent folder")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("cfg.tmp");
    let result = (|| {
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    })();
    if result.is_err() { let _ = std::fs::remove_file(&tmp); }
    result
}

/// Scale only the configured gamepad, rather than borrowing a wheel's vibration setting.
fn rumble_scale(cfg: &[DeviceCfg], name: &str) -> f32 {
    find_device_cfg(cfg, name).and_then(|d| d.ff_scale).map(|s| s.1).unwrap_or(1.0).clamp(0.0, 2.0)
}

/// A button-only gamepad configuration keeps the automatic analog layout. Once axes
/// are assigned, the explicit layout takes complete ownership of them.
/// Keep the steering's device kind with the value that actually won arbitration.
/// A centred gamepad beside a wheel must not give the wheel gamepad smoothing.
fn set_steering(out: &mut Analog, value: f32, stick: bool) {
    if out.steering.is_none_or(|old| value.abs() > old.abs()) {
        out.steering = Some(value);
        out.stick = stick;
    }
}

fn custom_gamepad_axes(cfg: &[DeviceCfg], name: &str) -> bool {
    find_device_cfg(cfg, name).is_some_and(|d| d.axes.iter().any(Option::is_some))
}

/// XInput exposes triggers as buttons in gilrs. Supply their full -1..1 travel in
/// unused slider slots so they can also be assigned as independent pedals.
fn gamepad_triggers(axes: &mut Vec<(usize, f32)>, left: f32, right: f32) {
    for (slot, value) in [(6, left), (7, right)] {
        if !axes.iter().any(|(k, _)| *k == slot) {
            axes.push((slot, value.clamp(0.0, 1.0) * 2.0 - 1.0));
        }
    }
}

pub struct Controllers {
    /// Each wheel's suspension travel, settled over a tenth of a second (see `wheel_bump`).
    settled: Vec<f32>,
    devices: Devices,
    focused: bool,
    cfg: Vec<DeviceCfg>,
    held: HeldButtons,
    editing: bool,
    pub(crate) raw_buttons: Vec<(String, usize, bool)>,
    pub enabled: bool,
    /// The settings' dead zone round the centre of a set-up device's axes (0..0.3).
    pub deadzone: f32,
    /// Automatic right-stick camera movement (Settings: `right_stick_look`).
    pub right_stick_look: bool,
    /// The dead zone round a gamepad stick's centre (Settings: `pad_deadzone`).
    pub pad_deadzone: f32,
    /// A gamepad with no buttons set up drives with its default ones (Settings:
    /// `pad_buttons`, see `gamepad_profile`).
    pub pad_presets: bool,
    /// The pedals' response curves (Settings → pedal strength; 1 = as the pedal reads).
    pub pedal_throttle: f32,
    pub pedal_brake: f32,
    /// Devices switched off (Settings: `ctrl_off`): not read at all.
    pub disabled: Vec<String>,
    /// Force feedback the other way round (Settings: `ff_invert`).
    pub ff_invert: bool,
    /// Force feedback and rumble switched on (Settings: `ff_enabled`).
    pub ff_enabled: bool,
    /// How strongly the tarmac's grain is felt under the wheels (Settings: `ff_road_vib`,
    /// 0 = none).
    pub ff_road: f32,
    /// How strongly the engine's buzz is felt through the frame (Settings: `ff_engine_vib`,
    /// 0 = none).
    pub ff_engine: f32,
    /// How long (s) a vibration eases away once it stops (Settings: `ff_fade`; 0 = it
    /// stops where it stands).
    pub ff_fade: f32,
    /// The wheel's rotation over the rotation that is the bus's full lock (Settings:
    /// `wheel_range` / `wheel_lock`; 1 = the whole wheel is the full lock, as OMSI).
    pub steer_gain: f32,
    /// Key actions of buttons pressed (true) and released (false) since the last poll.
    pub actions: Vec<(String, bool)>,
    /// Devices told about in the log (and on the screen) as not set up.
    announced: Vec<String>,
    /// Devices nobody set up whose X axis has left its centre: only from then on does it
    /// steer (an idle joystick beside the keyboard took the arrow keys for looking, #1476).
    moved: Vec<String>,
    /// A message for the screen: a wheel that is not set up.
    pub notice: Option<String>,
    /// The steering device: its name, physical position (-1..1, before dead zone
    /// and steering gain) now and previously, and whether it pushes back.
    steer: Option<(String, f32, f32, bool)>,
    ff_t: f32,
    ff_lateral: f32,
    ff_bump: f32,
    ff_bump_age: f32,
    /// The trembling of the tarmac and the engine, and the little state it keeps.
    ff_micro: Micro,
    /// The scripts' shaking and the state its fade keeps.
    ff_vib: ScriptVib,
    /// A rumble motor's share of the trembling, eased over half a second (see
    /// `rumble_feedback`).
    ff_rumble: f32,
    ff_source_logged: Option<String>,
    /// The rumble playing (`FF_Vib_Amp` and `FF_Vib_Period` of the bus), rebuilt when
    /// either changes.
    rumble: Vec<(gilrs::GamepadId, gilrs::ff::Effect, f32, f32, f32)>,
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    wheel: Option<crate::evdev_ff::Wheel>,
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    wheel_tried: Option<(String, std::time::Instant)>,
}

impl Controllers {
    pub(crate) fn configuration(&self) -> Vec<DeviceCfg> { self.cfg.clone() }

    pub(crate) fn connected(&self) -> Vec<Connected> { self.devices.connected() }

    /// Replace the mappings on the same Devices object. DirectInput's worker, device
    /// handles, effects and FFB filters remain alive.
    pub(crate) fn install_cfg(&mut self, cfg: Vec<DeviceCfg>) {
        self.held.release(&mut self.actions);
        self.cfg = cfg;
        self.notice = None;
    }

    pub(crate) fn set_editing(&mut self, editing: bool) {
        if editing && !self.editing { self.held.release(&mut self.actions); }
        self.editing = editing;
    }

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
            self.held.release(&mut self.actions);
            self.raw_buttons.clear();
            self.ff_source_logged = None;
        }
    }

    pub fn new(root: &Path, hwnd: Option<isize>) -> Controllers {
        let devices = Devices::new(hwnd, true);
        let cfg = read_cfg(root);
        for c in devices.connected() {
            log::info!("game controller: {} ({})", c.name, if cfg.iter().any(|d| names_match(&d.name, &c.name)) { "set up in gamectrler.cfg" } else if c.gamepad { "as a gamepad" } else { "not set up: its X axis steers" });
        }
        Self::with_devices(devices, cfg)
    }

    fn with_devices(devices: Devices, cfg: Vec<DeviceCfg>) -> Controllers {
        Controllers { settled: Vec::new(), devices, focused: true, cfg, held: HeldButtons::default(), editing: false, raw_buttons: Vec::new(), deadzone: 0.0, right_stick_look: true, pad_deadzone: 0.08, pad_presets: true, pedal_throttle: 1.0, pedal_brake: 1.0, disabled: Vec::new(), ff_invert: false, ff_enabled: true, ff_road: 1.0, ff_engine: 1.0, ff_fade: FF_FADE, steer_gain: 1.0, enabled: true, actions: Vec::new(), announced: Vec::new(), moved: Vec::new(), notice: None, steer: None, ff_t: 0.0, ff_lateral: 0.0, ff_bump: 0.0, ff_bump_age: 0.0, ff_micro: Micro::default(), ff_vib: ScriptVib::default(), ff_rumble: 0.0, ff_source_logged: None, rumble: Vec::new(), #[cfg(all(target_os = "linux", target_pointer_width = "64"))] wheel: None, #[cfg(all(target_os = "linux", target_pointer_width = "64"))] wheel_tried: None }
    }

    /// A wheel or joystick steers the bus (then the arrow keys look around, as in OMSI:
    /// a G29's buttons set to the arrow keys turned the view there).
    pub fn wheel_steering(&self) -> bool {
        self.enabled && self.steer.is_some()
    }

    /// The default buttons of a gamepad nobody set up buttons for (`gamepad_profile`): the
    /// buttons of the pads gilrs knows a layout of, and on Windows those of a DualShock or a
    /// DualSense, which DirectInput numbers.
    fn preset_buttons(&mut self) {
        #[allow(unused_mut)]
        let mut events = std::mem::take(&mut self.devices.pad_buttons);
        if !self.pad_presets || self.editing || !self.enabled {
            return;
        }
        let connected = self.devices.connected();
        #[cfg(windows)]
        for (name, n, down) in &self.raw_buttons {
            let sony = find_connected(&connected, name).is_some_and(|c| sony_pad(&c.name, c.hardware_id));
            if let Some(b) = sony.then(|| gamepad_profile::sony_direct_input_button(*n, HAT_BUTTONS)).flatten() {
                events.push((name.clone(), b, *down));
            }
        }
        for (name, button, down) in events {
            let Some(action) = gamepad_profile::default_action(button) else { continue };
            if self.off(&name) || !find_connected(&connected, &name).is_some_and(|c| c.gamepad) {
                continue;
            }
            if find_device_cfg(&self.cfg, &name).is_some_and(|d| d.buttons.iter().any(|b| !b.0.is_empty())) {
                continue;
            }
            self.actions.push((action.to_string(), down));
        }
    }

    /// Read the devices: the analog controls, and the button actions into `actions`.
    pub fn poll(&mut self) -> Analog {
        let mut out = Analog::default();
        self.raw_buttons = self.devices.poll();
        for (name, n, down) in &self.raw_buttons {
            if self.editing || !self.enabled || self.off(name) {
                continue;
            }
            self.held.event(&self.cfg, name, *n, *down, &mut self.actions);
        }
        self.preset_buttons();
        if !self.enabled {
            return out;
        }
        // the devices set up in gamectrler.cfg first; a device the file does not know only
        // gives what none of them does - a pad lying beside a set-up wheel held the steering
        // at its own centre, whichever the system listed first
        let off = self.disabled.clone();
        let mut pads: Vec<(Option<&DeviceCfg>, Connected)> = self.devices.connected().into_iter().filter(|c| !off.iter().any(|d| names_match(d, &c.name))).map(|c| (find_device_cfg(&self.cfg, &c.name), c)).collect();
        pads.sort_by_key(|(cfg, _)| cfg.is_none());
        // (the PlayStation pads nobody set up: read below, with the gamepads)
        #[cfg(windows)]
        let sony: Vec<Connected> = pads.iter().filter(|(cfg, c)| cfg.is_none() && sony_pad(&c.name, c.hardware_id)).map(|(_, c)| c.clone()).collect();
        let mut steer: Option<(String, f32, bool)> = None;
        // (a device set up to steer has the wheel; one nobody set up only lends its X axis)
        let mut steering_set_up = false;
        let dz = self.deadzone.clamp(0.0, 0.3);
        for (cfg, c) in pads {
            if let Some((k, v)) = c.axes.iter().find(|(_, v)| v.abs() > 0.5) {
                let key = format!("axis:{}", c.name);
                if !self.announced.contains(&key) {
                    log::info!("game controller {}: axis {k} at {v:.2} (set up in gamectrler.cfg: {}, gamepad: {})", c.name, cfg.is_some(), c.gamepad);
                    self.announced.push(key);
                }
            }
            match cfg {
                Some(d) => {
                    for (k, v) in c.axes.iter().copied() {
                        let Some((f, inverted)) = d.axes[k] else { continue };
                        if matches!(f, Func::Steering) {
                            steering_set_up = true;
                            let (steering, position) = wheel_steering(v, inverted, d.axis_flags[k], dz, self.steer_gain);
                            set_steering(&mut out, steering, c.gamepad);
                            if steer.is_none() && !c.gamepad {
                                steer = Some((c.name.clone(), position, c.ff));
                            }
                            continue;
                        }
                        let v = if inverted { -v } else { v };
                        if let Func::LookX | Func::LookY = f {
                            let i = (f == Func::LookY) as usize;
                            if out.look[i] == 0.0 {
                                out.look[i] = look_axis(v);
                            }
                            continue;
                        }
                        // the characteristic set up for the axis (gamectrler.cfg flags)
                        let v = axis_shape((v + 1.0) * 0.5, d.axis_flags[k]) * 2.0 - 1.0;
                        // the dead zone: round the wheel's centre, or at a pedal's rest
                        let v = match f {
                            Func::ThrottleBrake => v.signum() * ((v.abs() - dz).max(0.0) / (1.0 - dz)),
                            _ => ((v + 1.0 - 2.0 * dz).max(0.0) / (1.0 - dz)) - 1.0,
                        };
                        // a pedal travels the whole range, -1 up to 1 down
                        let pedal = crate::settings::pedal_ends(((v + 1.0) * 0.5).clamp(0.0, 1.0));
                        match f {
                            Func::Steering | Func::LookX | Func::LookY => unreachable!("steering and looking handled before pedal mapping"),
                            Func::Throttle => set(&mut out.throttle, crate::settings::pedal_curve(pedal, self.pedal_throttle)),
                            Func::Brake => set(&mut out.brake, crate::settings::pedal_curve(pedal, self.pedal_brake)),
                            Func::Clutch => set(&mut out.clutch, pedal),
                            Func::ThrottleBrake => {
                                // Omsi.exe: throttle = 2v-1 and brake = 1-2v (v = 0..1), so the
                                // raw-maximum half is the throttle
                                set(&mut out.throttle, crate::settings::pedal_curve(v.max(0.0), self.pedal_throttle));
                                set(&mut out.brake, crate::settings::pedal_curve((-v).max(0.0), self.pedal_brake));
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
                        if !free_axis_steers(&mut self.moved, &c.name, *v) {
                            continue;
                        }
                        // (a joystick's centre is slack, so it gets a little dead zone; a
                        // force-feedback wheel's is not: 2 % of it held a 1080° wheel's
                        // picture 11° behind the rim, #866)
                        let dz_free = if c.ff_capable { dz } else { dz.max(0.02) };
                        let (steering, position) = wheel_steering(*v, false, 0, dz_free, self.steer_gain);
                        out.steering.get_or_insert(steering);
                        if steer.is_none() {
                            steer = Some((c.name.clone(), position, c.ff));
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
                let xinput = cfg!(windows) && xinput_name(pad.name());
                if pad.mapping_source() == gilrs::MappingSource::None || custom_gamepad_axes(&self.cfg, pad.name()) {
                    continue;
                }
                #[cfg(target_os = "macos")]
                if self.devices.hid_wheel(&pad) {
                    continue;
                }
                if (di && !xinput) || off.iter().any(|d| names_match(d, pad.name())) {
                    continue;
                }
                let x = pad.value(Axis::LeftStickX);
                let steers = stick_steers(out.steering, steering_set_up, stick_deadzone_with(x, self.pad_deadzone));
                // (said once per pad: the stick moved, and whether it steers - a report of
                // "the sticks do nothing" then says which way the pad came in)
                if x.abs() > 0.5 && !self.announced.iter().any(|n| n == &format!("stick:{}", pad.name())) {
                    self.announced.push(format!("stick:{}", pad.name()));
                    log::info!("game controller {}: left stick {x:.2}, steers: {steers} (layout {:?})", pad.name(), pad.mapping_source());
                }
                let rt = pad.button_data(gilrs::Button::RightTrigger2).map(|d| d.value()).unwrap_or(0.0);
                let lt = pad.button_data(gilrs::Button::LeftTrigger2).map(|d| d.value()).unwrap_or(0.0);
                if steers {
                    out.steering = Some(stick_deadzone_with(x, self.pad_deadzone));
                    out.stick = true;
                }
                out.throttle.get_or_insert(crate::settings::pedal_curve(rt, self.pedal_throttle));
                out.brake.get_or_insert(crate::settings::pedal_curve(lt, self.pedal_brake));
                // the right stick looks round, as the truck games have it (#454)
                out.apply_default_gamepad_look(self.right_stick_look, pad.value(Axis::RightStickX), pad.value(Axis::RightStickY));
            }
        }
        // Windows: a DualShock 4 or a DualSense is a DirectInput device (gilrs reads XInput
        // only there), laid out as the HID descriptor has it: X and Y the left stick, Z and
        // Rz the right one, Rx and Ry the triggers (at rest at -1) - the same default layout
        // as the Xbox pad's all the same
        #[cfg(windows)]
        for c in &sony {
            let axis = |k: usize| c.axes.iter().find(|(s, _)| *s == k).map_or(0.0, |(_, v)| *v);
            let x = axis(0);
            let steers = stick_steers(out.steering, steering_set_up, stick_deadzone_with(x, self.pad_deadzone));
            if steers {
                out.steering = Some(stick_deadzone_with(x, self.pad_deadzone));
                out.stick = true;
            }
            out.throttle.get_or_insert(crate::settings::pedal_curve((axis(4) + 1.0) / 2.0, self.pedal_throttle));
            out.brake.get_or_insert(crate::settings::pedal_curve((axis(3) + 1.0) / 2.0, self.pedal_brake));
            out.apply_default_gamepad_look(self.right_stick_look, axis(2), -axis(5));
        }
        // macOS: Xbox-type controllers, read through the GameController framework and so
        // never gilrs's pads above - the same default layout all the same
        #[cfg(target_os = "macos")]
        for (name, axes) in self.devices.gc.connected() {
            if custom_gamepad_axes(&self.cfg, &name) || off.iter().any(|d| names_match(d, &name)) {
                continue;
            }
            let axis = |k: usize| axes.iter().find(|(c, _)| *c == k).map_or(0.0, |(_, v)| *v);
            let x = axis(0);
            let steers = stick_steers(out.steering, steering_set_up, stick_deadzone_with(x, self.pad_deadzone));
            if x.abs() > 0.5 && !self.announced.iter().any(|n| n == &format!("stick:{name}")) {
                self.announced.push(format!("stick:{name}"));
                log::info!("game controller {name}: left stick {x:.2}, steers: {steers}");
            }
            if steers {
                out.steering = Some(stick_deadzone_with(x, self.pad_deadzone));
                out.stick = true;
            }
            out.throttle.get_or_insert(crate::settings::pedal_curve((axis(4) + 1.0) / 2.0, self.pedal_throttle));
            out.brake.get_or_insert(crate::settings::pedal_curve((axis(5) + 1.0) / 2.0, self.pedal_brake));
            out.apply_default_gamepad_look(self.right_stick_look, axis(2), axis(3));
        }
        let before = self.steer.as_ref().filter(|s| steer.as_ref().is_some_and(|n| n.0 == s.0)).map(|s| s.1);
        self.steer = steer.map(|(name, v, ff)| (name, v, before.unwrap_or(v), ff));
        out
    }

    /// On a force-feedback wheel, combine parking resistance, centring, the bus's
    /// lateral motion, front-wheel bumps, the trembling of the tarmac and the engine and
    /// script-driven vibration - every vibration that comes to an end eased away rather
    /// than cut. Other devices get the vibration as rumble.
    pub fn feedback(&mut self, f: FfInput) {
        let on = self.enabled && f.on && self.ff_enabled && self.focused;
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
        // A jolt is held and eased away over the settings' fade instead of being let go the
        // frame the physics stops reporting it: a force that falls to zero in a single step
        // is a jolt of its own, felt through the rim and heard in the wheel's motor. The
        // envelope decays on its own rate rather than through `fade_gain`, because a gain
        // only starts once the jolt it follows is already spent. With the fade at zero the
        // jolt stops where it stands, which is how the wheel behaved before.
        self.ff_bump = if !on {
            0.0
        } else if self.ff_fade > 0.01 {
            let eased = self.ff_bump * (-f.dt.max(0.0) / (self.ff_fade * 0.3)).exp();
            let held = incoming_bump.max(eased);
            if held < 0.002 { 0.0 } else { held }
        } else {
            incoming_bump
        };
        // the trembling of the tarmac and of the engine, which never stops while the bus
        // runs and needs no fade of its own
        let micro = if on {
            self.ff_micro.sample(f.dt, f.kmh, f.street_cond, f.engine_rpm, f.engine_load, self.ff_road, self.ff_engine)
        } else {
            0.0
        };
        // A script's shaking is not an envelope that can be held in place - the scripts write
        // it afresh every frame - so `ScriptVib` keeps the last amplitude and period they
        // asked for and eases those, which is what reaches the wheel. The road and the
        // engine are not something that begins and ends: they go quiet with the bus and
        // the engine, smoothly, where they stand.
        let (vib_amp, vib_period) = self.ff_vib.step(on, f.vib_amp, f.vib_period, self.ff_fade, f.dt);
        f.wheel_bump = self.ff_bump;
        f.wheel_bump_age = self.ff_bump_age;
        f.vib_amp = vib_amp;
        f.vib_period = vib_period;
        f.micro = micro;
        if self.ff_source_logged.as_deref() != self.steer.as_ref().map(|s| s.0.as_str()) {
            self.ff_source_logged = self.steer.as_ref().map(|s| s.0.clone());
            if let Some((name, _, _, effect)) = self.steer.as_ref() {
                let cfg = find_device_cfg(&self.cfg, name);
                let (steering, vibration) = cfg.and_then(|d| d.ff_scale).unwrap_or((1.0, 1.0));
                #[cfg(windows)]
                let axis_reversed = self.devices.di.as_ref().is_some_and(|di| force_axis_reversed(cfg, di.force_axis(name)));
                #[cfg(not(windows))]
                let axis_reversed = calibrated_steering_reversed(cfg);
                log::info!("force feedback: steering source {name}, effect available: {effect}, config: {}, steering force: {steering:.2}, vibration: {vibration:.2}, invert: {}", cfg.map(|d| d.name.as_str()).unwrap_or("none"), feedback_inverted(cfg, self.ff_invert, axis_reversed));
            }
        }
        #[cfg(windows)]
        if let (Some((name, x, x0, true)), Some(di)) = (self.steer.clone(), self.devices.di.as_mut()) {
            // (the file's [FFScale] of the device: steering forces, then vibration)
            let cfg = find_device_cfg(&self.cfg, &name);
            let (k_s, k_e) = cfg.and_then(|d| d.ff_scale).unwrap_or((1.0, 1.0));
            // The scripts' shaking is the wheel's own periodic effect where it has one
            // (a rattle of a few ms sampled once a frame comes out as a random wobble).
            let vib_amp = if on { f.vib_amp.clamp(0.0, 1.0) * VIB_SHARE * k_e.clamp(0.0, 2.0) } else { 0.0 };
            let f = if di.set_vibration(&name, vib_amp, f.vib_period) { FfInput { vib_amp: 0.0, ..f } } else { f };
            let force = if on { wheel_force(&f, x, x0, &mut self.ff_t, k_s, k_e) } else { 0.0 };
            // The wheel force is calculated from the steering axis after its configured
            // reversal, while DirectInput sends forces in the physical axis direction.
            let axis_reversed = force_axis_reversed(cfg, di.force_axis(&name));
            let force = if feedback_inverted(cfg, self.ff_invert, axis_reversed) { -force } else { force };
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
                let cfg = find_device_cfg(&self.cfg, &name);
                let (k_s, k_e) = cfg.and_then(|d| d.ff_scale).unwrap_or((1.0, 1.0));
                let force = if on { wheel_force(&f, x, x0, &mut self.ff_t, k_s, k_e) } else { 0.0 };
                let axis_reversed = calibrated_steering_reversed(cfg);
                let inverted = feedback_inverted(cfg, self.ff_invert, axis_reversed);
                if !w.set_force(if inverted { -force } else { force }) {
                    log::warn!("force feedback: {name} went away; looking for it again");
                    self.wheel = None;
                }
                return;
            }
        }
        let _ = (&self.steer, &self.ff_t, wheel_force);
        // A rumble motor cannot show a texture: it can only buzz harder or softer, and
        // every change of strength is another effect built on the device, which drops the
        // one it had where it stood. Its share of the trembling is therefore eased over
        // half a second and moved in steps, so the rumble swells and fades with the road
        // instead of building an effect on every frame.
        self.ff_rumble += (micro.abs().min(1.0) - self.ff_rumble) * (f.dt / 0.5).clamp(0.0, 1.0);
        let tarmac = (self.ff_rumble * 16.0).round() / 16.0;
        self.rumble_feedback(if on { f.vib_amp.max(f.wheel_bump * 0.75) + tarmac } else { 0.0 }, f.vib_period);
    }

    /// The shaking as a rumble (`FF_Vib_Amp`, `FF_Vib_Period`: OMSI hands DirectInput
    /// Round(period × 10000) µs, so a hundredth of a second per unit - the shaking comes in
    /// pulses that long, on for half of it; 0 is a steady rumble).
    fn rumble_feedback(&mut self, amp: f32, period: f32) {
        let amp = amp.clamp(0.0, 1.0);
        let period = if period.is_finite() { period.clamp(0.0, 100.0) } else { 0.0 };
        let Some(g) = self.devices.gilrs.as_mut() else { return };
        let pads: Vec<(gilrs::GamepadId, f32, f32)> = g.gamepads()
            .filter(|(_, p)| p.is_ff_supported() && !self.disabled.iter().any(|n| names_match(n, p.name())))
            .map(|(id, p)| {
                let scale = rumble_scale(&self.cfg, p.name());
                (id, (amp * scale).min(1.0), scale)
            }).collect();
        // Keep unchanged gamepad effects. A saved vibration-strength change must take
        // effect even when the bus's vibration amplitude and period have not changed.
        self.rumble.retain(|(id, _, was, was_period, was_scale)| pads.iter().any(|(pad, value, scale)|
            pad == id && *value >= 0.01 && (was - value).abs() < 0.02
                && (was_period - period).abs() < 0.05 && (was_scale - scale).abs() < 1e-6));
        for (id, value, scale) in pads {
            if value < 0.01 || self.rumble.iter().any(|(pad, ..)| *pad == id) { continue; }
            let m = (value * u16::MAX as f32) as u16;
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
                .gamepads(&[id])
                .finish(g);
            if let Ok(e) = effect {
                let _ = e.play();
                self.rumble.push((id, e, value, period, scale));
            }
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
impl Controllers {
    /// The wheels' jolt this frame (None: not driving, the travel is settled anew on return).
    pub(crate) fn wheel_bump(&mut self, body: Option<&omsi_sim::rigid::RigidBody>, kmh: f32, dt: f32) -> f32 {
        let Some(body) = body else {
            self.settled.clear();
            return 0.0;
        };
        let travel: Vec<f32> = body.wheels.iter().map(|w| w.compression).collect();
        let ripples = settle(&mut self.settled, &travel, dt);
        body.wheels.iter().zip(ripples).enumerate().map(|(i, (w, ripple))| {
            let impact_speed = body.wheel_impacts.iter().filter(|hit| hit.obstacle == i).map(|hit| hit.speed).fold(0.0, f32::max);
            bump_strength(ripple, impact_speed, kmh) * if w.steered { 1.0 } else { 0.55 }
        }).fold(0.0, f32::max)
    }
}

/// How far each wheel's travel leaves where it settled (a 1-2 cm road seam is no jolt).
fn settle(settled: &mut Vec<f32>, travel: &[f32], dt: f32) -> Vec<f32> {
    if settled.len() != travel.len() {
        *settled = travel.to_vec();
    }
    let k = 1.0 - (-dt.max(0.0) / 0.1).exp();
    travel.iter().zip(settled.iter_mut()).map(|(t, s)| {
        let ripple = t - *s;
        *s += ripple * k;
        ripple
    }).collect()
}

fn bump_strength(ripple: f32, impact_speed: f32, kmh: f32) -> f32 {
    let suspension = ((ripple.abs() - 0.012) / 0.09).clamp(0.0, 1.0);
    let impact = ((impact_speed - 0.12) / 1.1).clamp(0.0, 1.0);
    suspension.max(impact) * (kmh.abs() / 4.0).clamp(0.0, 1.0)
}

/// How much of the full force `FF_Vib_Amp` 1 shakes the wheel with.
const VIB_SHARE: f32 = 0.25;

/// The force on a wheel standing at `x` (-1 full left .. 1), `x0` the frame before: -1..1.
/// Tyre scrub resists turning the wheel at a standstill and falls away once the bus rolls.
/// Self-aligning torque then returns the wheel to centre, with a softer response near full
/// lock and feedback from the bus's lateral acceleration. On top of that come front-wheel
/// jolts, the scripts' shaking and the tremble of the tarmac and the engine.
fn wheel_force(f: &FfInput, x: f32, x0: f32, t: &mut f32, k_springs: f32, k_effects: f32) -> f32 {
    let dt = f.dt.max(1e-3);
    let v = f.kmh.abs();
    let x = x.clamp(-1.0, 1.0);
    // A parked bus has no rolling self-aligning torque. Ignore tiny physics
    // speed fluctuations, then smoothly restore the usual forces by 5 km/h.
    let rolling = ((v - 0.5) / 4.5).clamp(0.0, 1.0);
    let rolling = rolling * rolling * (3.0 - 2.0 * rolling);
    // At road speed, power steering gives the driver a firmer sense of direction.
    // Keep parking and town-speed forces familiar while separating 70 km/h from 10 km/h.
    let road_speed = ((v - 20.0) / 50.0).clamp(0.0, 1.0);
    let spring_strength = (0.22 + 0.28 * v / (v + 10.0)) * (1.0 + 0.5 * road_speed);
    let moving_steering_gain = 1.0 + 0.18 * v / (v + 8.0);
    // More return torque at small and medium angles, where wheel friction can
    // otherwise stop the return. Compensate wheel friction near the centre with
    // a smooth extra torque, tapered away at larger angles. It crosses zero
    // continuously so there is no fixed kick when the wheel passes the centre.
    let centre_return = 0.025 * x / (x * x + 0.004 * 0.004).sqrt() / (1.0 + (x / 0.12).powi(4));
    let spring = -(spring_strength * x / (0.5 + 1.15 * x.abs()) + centre_return) * rolling;
    let road_align = -(f.lateral_accel / 9.81).clamp(-0.45, 0.45) * 0.25 * (v / 5.0).clamp(0.0, 1.0) * rolling;
    // Assisted steering should not demand ever more hand force near full lock.
    let lock_assist = 1.0 / (1.0 + 0.55 * x * x);
    let turning_speed = ((x - x0) / dt).clamp(-4.0, 4.0);
    // Without a steering-column torque sensor, motion away from the centre is our
    // indication that the driver is actively turning. Assist that motion, but keep
    // the full self-aligning torque when the wheel is held or let go.
    let turning_out = (x * turning_speed * 2.0).clamp(0.0, 1.0);
    let assist = 1.0 - (0.4 - 0.16 * (v / 80.0).min(1.0)) * turning_out;
    let parking_drag = 0.018 + 0.12 / (1.0 + (v / 6.0).powi(2));
    // Once rolling, reduce drag on return so it does not cancel the centring
    // force. At a standstill, resist motion equally in either direction.
    let returning = x * turning_speed < 0.0;
    let drag = -turning_speed * parking_drag * if returning { 1.0 - 0.8 * rolling } else { 1.0 };
    // Keep some damping on return as well as when turning out. Reducing all
    // resistance on return lets a quick wheel overshoot and oscillate around
    // the centre, especially when force commands arrive at a low frame rate.
    // This only resists motion: it adds no holding force or parked centring.
    let damping = -0.03 * turning_speed * rolling;
    *t += dt;
    let period = (f.vib_period * 0.01).max(0.02);
    let shake = f.vib_amp.clamp(0.0, 1.0) * VIB_SHARE * (std::f32::consts::TAU * *t / period).sin();
    // Preserve small road details while softening kerb-sized peaks. One short
    // kick and rebound feels less like a continuously shaking wheel mount.
    let bump = f.wheel_bump.clamp(0.0, 1.0).sqrt() * 0.46 * (std::f32::consts::TAU * f.wheel_bump_age * 6.5).cos();
let steering = ((spring + road_align) * lock_assist * assist + drag) * moving_steering_gain + damping;
    let micro = f.micro.clamp(-1.0, 1.0);
    (steering * k_springs.clamp(0.0, 2.0) + (shake + bump + micro) * k_effects.clamp(0.0, 2.0)).clamp(-1.0, 1.0)
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

/// The connected device this name is set up as - its exact name first: an Xbox-type pad's
/// gilrs lists too, as a plain "Controller" with no axis or button at all (see
/// `mac_game_controller`), and that name is a substring of (so fuzzy-matches) this device's.
pub(crate) fn find_connected<'a>(connected: &'a [Connected], name: &str) -> Option<&'a Connected> {
    connected.iter().find(|c| c.name == name).or_else(|| connected.iter().find(|c| names_match(&c.name, name)))
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

fn feedback_inverted(cfg: Option<&DeviceCfg>, global: bool, axis_reversed: bool) -> bool {
    cfg.and_then(|d| d.ff_invert).unwrap_or(global) ^ axis_reversed
}

#[cfg_attr(windows, allow(dead_code))]
fn calibrated_steering_reversed(cfg: Option<&DeviceCfg>) -> bool {
    // Linux previously applied the global sign directly; preserve it for uncalibrated wheels.
    cfg.filter(|d| d.ff_invert.is_some())
        .and_then(|d| d.axes.iter().flatten().find(|(function, _)| *function == Func::Steering))
        .is_some_and(|(_, reversed)| *reversed)
}

/// The button's number on its device as DirectInput counts them (and `gamectrler.cfg` with
/// it), from the system's code for it: the HID button usage on macOS (page 9, from 1), the
/// evdev key code on Linux (BTN_JOYSTICK.. and BTN_TRIGGER_HAPPY.. for a wheel's or
/// joystick's buttons, BTN_GAMEPAD.. for a pad's), the button index on Windows. It used to be
/// the place among the buttons pressed so far - the first button ever pressed was "button 1"
/// whichever it was. None on Windows for a code that is no button (gilrs turns the analog
/// triggers, axis codes, into button events too: they are pedals, not numbered buttons), and
/// on macOS for an axis' code (a DualSense's L2 and R2 came in as buttons 1 and 2, the
/// numbers of its square and cross: their place among the codes; they are axes).
pub(crate) fn button_number(pad: &gilrs::Gamepad, code: gilrs::ev::Code) -> Option<usize> {
    #[cfg(target_os = "linux")]
    if let Some(n) = declared_button_index(pad.name(), code.into_u32()) {
        return Some(n);
    }
    if cfg!(windows) {
        return code_button(code.into_u32());
    }
    if cfg!(target_os = "macos") && mac_axis_code(code.into_u32()) {
        return None;
    }
    Some(code_button(code.into_u32()).unwrap_or_else(|| {
        let mut codes: Vec<u32> = pad.state().buttons().map(|(c, _)| c.into_u32()).collect();
        codes.sort_unstable();
        codes.iter().position(|c| *c == code.into_u32()).unwrap_or(usize::MAX)
    }))
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

/// The button number of an evdev key code of a device read from evdev (`evdev_buttons`).
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
pub(crate) fn evdev_button_number(name: &str, code: u32) -> Option<usize> {
    declared_button_index(name, code).or_else(|| code_button(code))
}

#[cfg(target_os = "linux")]
pub(crate) fn declared_button_count(name: &str) -> usize {
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
pub(crate) fn key_bitmap_buttons(bitmap: &str) -> Vec<u32> {
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

pub(crate) fn code_button(code: u32) -> Option<usize> {
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
        // gilrs' D-pad (Button codes u32::MAX-3.. up, right, down, left) is the pad's hat,
        // numbered as DirectInput's first hat
        if code >= u32::MAX - 3 {
            return Some(HAT_BUTTONS + (code - (u32::MAX - 3)) as usize);
        }
        (hi == 0).then_some(lo)
    } else {
        None
    }
}

/// An axis' characteristic from its gamectrler.cfg flags, on 0..1 (after the inversion),
/// as Omsi.exe applies it (poll sub_6466b8): 2 extends the range by a quarter, 4 is
/// degressive, 8 progressive, 0x10 makes either of them symmetric round the centre.
pub(crate) fn axis_shape(v: f32, flags: i32) -> f32 {
    use std::f32::consts::PI;
    let mut v = v;
    if flags & 2 != 0 {
        v = v * 1.25 - 0.125;
    }
    let bi = flags & 0x10 != 0;
    if flags & 4 != 0 {
        v = if bi { ((v - 0.5) * PI).sin() * 0.5 + 0.5 } else { (v * PI * 0.5).sin() };
    } else if flags & 8 != 0 {
        v = if bi {
            let d = (v - 0.5) * 2.0;
            0.5 + d.signum() * d * d * 0.5
        } else {
            v * v
        };
    }
    v.clamp(0.0, 1.0)
}

/// The characteristics the set-up offers, with their flag bits (Omsi.exe's combo, 0x652ac4).
pub(crate) const AXIS_SHAPES: [(&str, i32); 5] = [("Linear", 0), ("Progressive", 8), ("Degressive", 4), ("Bi-progressive", 8 | 0x10), ("Bi-degressive", 4 | 0x10)];

#[cfg(test)]
mod axis_shape_tests {
    use super::axis_shape;
    #[test]
    fn characteristics_match_omsi() {
        assert!((axis_shape(0.75, 24) - 0.625).abs() < 1e-6);
        assert!((axis_shape(0.5, 24) - 0.5).abs() < 1e-6);
        assert!((axis_shape(0.5, 8) - 0.25).abs() < 1e-6);
        assert!((axis_shape(0.5, 4) - (std::f32::consts::PI / 4.0).sin()).abs() < 1e-6);
        assert!((axis_shape(0.3, 0) - 0.3).abs() < 1e-6);
        assert_eq!(axis_shape(1.0, 2), 1.0);
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_gilrs_event_that_panics_is_skipped() {
        let mut queue = vec![Some(2), None, Some(1)];
        let mut next = || match queue.pop().unwrap() {
            None => panic!("index out of bounds: the len is 0 but the index is 0"),
            ev => ev,
        };
        assert_eq!(super::next_event_caught(&mut next), Some(1));
        assert_eq!(super::next_event_caught(&mut next), Some(2));
    }

    #[test]
    fn latching_switches_switch_back_and_stay_in_the_file() {
        assert_eq!(super::latch_release("blinker_left_set"), "blinker_off");
        assert_eq!(super::latch_release("Blinker_Right_Set"), "blinker_off");
        assert_eq!(super::latch_release("parking_brake_set"), "parking_brake_release");
        assert_eq!(super::latch_release("blinker_warn_toggle"), "blinker_warn_toggle");
        let text = "[ctrl]\r\nAER0 Truck Simulator Gear\r\n0\r\n\r\n[buttons]\r\n2\r\nblinker_warn_toggle\r\n0\r\n\r\n0\r\n\r\n[openOMSI.Latching]\r\n1\r\n";
        let devices = super::parse_cfg(text);
        assert_eq!(devices[0].latching, vec![0]);
        let again = super::parse_cfg(&super::cfg_text(&devices));
        assert_eq!(again[0].latching, vec![0]);
        assert_eq!(again[0].buttons, devices[0].buttons);
        assert!(super::cfg_text(&devices).contains("[openOMSI.Latching]\r\n1\r\n"));
    }

    #[test]
    fn look_axes_are_kept_in_the_file_and_rest_at_the_centre() {
        // (openOMSI's own numbers after OMSI's five, written back as read)
        for f in [super::Func::LookX, super::Func::LookY] {
            assert_eq!(super::Func::from_code(super::Func::code(Some(f))), Some(f));
        }
        assert_eq!(super::Func::LABELS.len() as i32, super::Func::code(Some(super::Func::LookY)) + 2);
        assert_eq!(super::look_axis(0.1), 0.0);
        assert_eq!(super::look_axis(1.0), 1.0);
        assert_eq!(super::look_axis(-1.0), -1.0);
        assert!(super::look_axis(0.5) > 0.4 && super::look_axis(0.5) < 0.5);
    }

    #[test]
    fn a_stick_leaves_its_dead_zone_from_nothing() {
        use super::stick_deadzone;
        assert_eq!(stick_deadzone(0.05), 0.0);
        assert_eq!(stick_deadzone(-0.08), 0.0);
        assert!(stick_deadzone(0.0801) < 0.001);
        assert!((stick_deadzone(1.0) - 1.0).abs() < 1e-6);
        assert!((stick_deadzone(-1.0) + 1.0).abs() < 1e-6);
        assert!(stick_deadzone(-0.5) < 0.0);
    }

    #[test]
    fn stick_steering_is_smoothed_at_any_frame_rate() {
        use super::smooth_toward;
        assert_eq!(smooth_toward(0.0, 0.7, 0.016, 0.0), 0.7);
        // never past the target
        assert!(smooth_toward(0.0, 0.5, 1.0, 0.12) <= 0.5);
        // two 8 ms frames land where one 16 ms frame does
        let twice = smooth_toward(smooth_toward(0.0, 1.0, 0.008, 0.12), 1.0, 0.008, 0.12);
        assert!((twice - smooth_toward(0.0, 1.0, 0.016, 0.12)).abs() < 1e-5);
        // a stick wobbling 0.03 either side of 0.2 every frame: the wheel holds near 0.2
        let mut v = 0.2;
        for i in 0..240 {
            v = smooth_toward(v, if i % 2 == 0 { 0.23 } else { 0.17 }, 1.0 / 60.0, 0.12);
        }
        assert!((v - 0.2).abs() < 0.01, "{v}");
    }

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
    fn xbox_gamepad_with_direct_input_twin_is_not_claimed_by_direct_input() {
        let xinput_pads = true;
        let is_claimed_by_di = |name: &str, ff_capable: bool| {
            super::include_direct_input_device(name, ff_capable, xinput_pads)
        };
        assert!(!is_claimed_by_di("Controller (XBOX 360 For Windows)", false));
        assert!(!is_claimed_by_di("Xbox Wireless Controller", false));
        assert!(is_claimed_by_di("Logitech G29 Driving Force Racing Wheel", true));
        assert!(is_claimed_by_di("HORI Racing Wheel APEX", false));
        assert!(is_claimed_by_di("G920 Driving Force Racing Wheel for Xbox One", true));
    }

    #[test]
    fn a_dualsense_on_macos_is_one_device() {
        let pad = ["PS5 Controller", "DualSense Wireless Controller"];
        let id = Some((0x054c, 0x0ce6));
        // the HID device under the system's name, or with the same ids, is gilrs's pad
        assert!(super::same_hid_device("DualSense Wireless Controller", None, pad, id));
        assert!(super::same_hid_device("Wireless Controller", id, ["PS5 Controller", "PS5 Controller"], id));
        assert!(!super::same_hid_device("HORI Truck Control System", Some((0x0f0d, 0x01b7)), pad, id));
        assert!(!super::same_hid_device("Thrustmaster T.16000M", None, pad, None));
    }

    #[test]
    fn analog_triggers_are_no_buttons_on_macos() {
        // L2 and R2 (Rx, Ry), sliders, the simulation page's pedals and the hat
        for code in [0x10033, 0x10034, 0x10036, 0x10039, 0x200C4, 0x200C5] {
            assert!(super::mac_axis_code(code), "{code:#x}");
        }
        // the button page, and a generic desktop D-pad button
        for code in [0x90001, 0x9000e, 0x10090] {
            assert!(!super::mac_axis_code(code), "{code:#x}");
        }
    }

    #[test]
    fn a_hat_switch_is_read_with_up_negative() {
        // macOS: the hat's two axes; an SDL mapping's D-pad Y comes turned round
        assert_eq!(super::hat_axis(0x10039, 1.0, false), Some((0, 1.0)));
        assert_eq!(super::hat_axis(0x1003A, 1.0, true), Some((1, -1.0)));
        assert_eq!(super::hat_axis(0x1003A, 1.0, false), Some((1, 1.0)));
        // Linux: ABS_HAT0X .. ABS_HAT3Y
        assert_eq!(super::hat_axis(0x30010, -1.0, false), Some((0, -1.0)));
        assert_eq!(super::hat_axis(0x30017, 1.0, false), Some((7, 1.0)));
        assert_eq!(super::hat_axis(0x30018, 1.0, false), None);
        assert_eq!(super::hat_axis(0x10030, 1.0, false), None);
    }

    #[test]
    fn system_gamepad_buttons_work_alongside_direct_input_wheels() {
        assert!(super::use_gilrs_buttons(true, true));
        assert!(!super::use_gilrs_buttons(true, false));
        assert!(super::use_gilrs_buttons(false, false));
    }

    #[test]
    fn wheels_are_not_treated_as_system_gamepads() {
        assert!(!super::is_system_gamepad("Logitech G29 Driving Force Racing Wheel", true));
        assert!(!super::is_system_gamepad("Logitech Driving Force GT", true));
        assert!(!super::is_system_gamepad("Logitech Driving Force GT", false));
        assert!(!super::is_system_gamepad("Thrustmaster T300RS", true));
        assert!(!super::is_system_gamepad("Thrustmaster T300RS", false));

        assert!(super::is_system_gamepad("Xbox 360 Controller", false));
        assert!(super::is_system_gamepad("Controller (Xbox One)", false));
        assert!(super::is_system_gamepad("Xbox Series X Controller", false));
        assert!(!super::is_system_gamepad("G920 Driving Force Racing Wheel for Xbox One", true));
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
    fn feedback_keeps_the_physical_position_inside_the_steering_deadzone() {
        for (axis, deadzone) in [(0.2, 0.3), (0.01, 0.02)] {
            let (steering, position) = super::wheel_steering(axis, false, 0, deadzone, 1.0);
            assert_eq!(steering, 0.0);
            assert_eq!(position, axis);
            let f = super::FfInput { on: true, kmh: 30.0, dt: 0.016, ..Default::default() };
            let mut t = 0.0;
            let force = super::wheel_force(&f, position, position, &mut t, 1.0, 0.0);
            assert!(force < 0.0, "axis={axis}: {force}");
        }
    }

    #[test]
    fn steering_range_and_reversal_preserve_the_feedback_position() {
        let (steering, position) = super::wheel_steering(0.5, false, 0, 0.1, 2.0);
        assert!((steering - 8.0 / 9.0).abs() < 1e-6);
        assert_eq!(position, 0.5);
        let (steering, position) = super::wheel_steering(0.5, true, 0, 0.1, 2.0);
        assert!((steering + 8.0 / 9.0).abs() < 1e-6);
        assert_eq!(position, -0.5);
        let (steering, position) = super::wheel_steering(0.5, false, 0, 0.0, 0.25);
        assert_eq!(steering, 0.125);
        assert_eq!(position, 0.5);
    }

    #[test]
    fn axis_characteristics_change_bus_steering_but_not_feedback_position() {
        let (steering, position) = super::wheel_steering(0.5, false, 24, 0.0, 1.0);
        assert_eq!(steering, 0.25);
        assert_eq!(position, 0.5);
        let (steering, position) = super::wheel_steering(0.5, true, 24, 0.0, 1.0);
        assert_eq!(steering, -0.25);
        assert_eq!(position, -0.5);
        let (steering, position) = super::wheel_steering(0.4, false, 2, 0.0, 1.0);
        assert!((steering - 0.5).abs() < 1e-6);
        assert_eq!(position, 0.4);
    }

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
    fn force_feedback_direction_is_saved_and_applied_per_wheel() {
        let devices = vec![
            super::DeviceCfg { name: "Wheel A".into(), ff_invert: Some(true), ..Default::default() },
            super::DeviceCfg { name: "Wheel B".into(), ff_invert: Some(false), ..Default::default() },
            super::DeviceCfg { name: "Uncalibrated".into(), ..Default::default() },
        ];
        let saved = super::parse_cfg(&super::cfg_text(&devices));
        assert_eq!(saved[0].ff_invert, Some(true));
        assert_eq!(saved[1].ff_invert, Some(false));
        assert_eq!(saved[2].ff_invert, None);
        assert!(super::feedback_inverted(Some(&saved[0]), false, false));
        assert!(!super::feedback_inverted(Some(&saved[1]), true, false));
        assert!(super::feedback_inverted(Some(&saved[2]), true, false));
        assert!(!super::feedback_inverted(Some(&saved[0]), false, true));
        assert!(super::feedback_inverted(Some(&saved[1]), true, true));
        let mut reversed = super::DeviceCfg::default();
        reversed.axes[0] = Some((super::Func::Steering, true));
        assert!(!super::calibrated_steering_reversed(Some(&reversed)));
        reversed.ff_invert = Some(false);
        assert!(super::calibrated_steering_reversed(Some(&reversed)));
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
        if cfg!(windows) {
            assert_eq!(super::code_button(3), Some(3));
            // an analog trigger (WGI axis code) is no numbered button
            assert_eq!(super::code_button(0x1_0004), None);
            // the D-pad is the first hat, not dropped
            assert_eq!(super::code_button(u32::MAX - 3), Some(super::HAT_BUTTONS));
            assert_eq!(super::code_button(u32::MAX), Some(super::HAT_BUTTONS + 3));
        }
    }

    #[test]
    fn the_wheel_is_pulled_to_the_middle_harder_at_speed() {
        let mut t = 0.0;
        let f = |kmh| super::FfInput { on: true, kmh, dt: 0.016, ..Default::default() };
        let slow = super::wheel_force(&f(10.0), 0.5, 0.5, &mut t, 1.0, 1.0);
        let fast = super::wheel_force(&f(60.0), 0.5, 0.5, &mut t, 1.0, 1.0);
        assert!(slow < 0.0 && fast < slow, "{slow} {fast}");
        // Moving to the right at a standstill, it is held back by tyre scrub.
        let turning = super::wheel_force(&f(0.0), 0.0, -0.05, &mut t, 1.0, 1.0);
        assert!(turning < 0.0);
    }

    #[test]
    fn a_parked_wheel_is_not_pulled_to_the_middle() {
        let mut t = 0.0;
        for kmh in [-0.5, -0.1, 0.0, 0.1, 0.5] {
            // Include residual lateral acceleration from the body's suspension.
            let f = super::FfInput { on: true, kmh, lateral_accel: 3.0, dt: 0.016, ..Default::default() };
            for x in [-1.0, -0.5, 0.0, 0.5, 1.0] {
                let force = super::wheel_force(&f, x, x, &mut t, 1.0, 0.0);
                assert_eq!(force, 0.0, "kmh={kmh}, steering={x}");
            }
        }
    }

    #[test]
    fn parking_drag_opposes_turning_in_both_directions() {
        let mut t = 0.0;
        let f = super::FfInput { on: true, dt: 0.016, ..Default::default() };
        for x in [-0.5, 0.5] {
            let right = super::wheel_force(&f, x, x - 0.01, &mut t, 1.0, 0.0);
            let left = super::wheel_force(&f, x, x + 0.01, &mut t, 1.0, 0.0);
            assert!(right < 0.0 && left > 0.0, "{right} {left}");
            assert!((right + left).abs() < 1e-6, "{right} {left}");
        }
    }

    #[test]
    fn self_aligning_torque_builds_up_as_the_bus_rolls_in_either_direction() {
        let mut t = 0.0;
        let force = |kmh, t: &mut f32| {
            let f = super::FfInput { on: true, kmh, dt: 0.016, ..Default::default() };
            super::wheel_force(&f, 0.5, 0.5, t, 1.0, 0.0)
        };
        let mut previous = 0.0;
        for kmh in [0.51, 1.0, 2.0, 3.0, 4.0, 5.0] {
            let forward = force(kmh, &mut t);
            let reverse = force(-kmh, &mut t);
            assert!(forward < previous, "kmh={kmh}: {forward} >= {previous}");
            assert_eq!(forward, reverse);
            previous = forward;
        }
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
    fn a_ninety_degree_turn_has_return_torque_across_wheel_ranges() {
        let mut t = 0.0;
        let f = super::FfInput { on: true, kmh: 30.0, dt: 0.016, ..Default::default() };
        for range in [900.0, 1080.0, 1800.0, 2880.0] {
            let x = 90.0 / (range * 0.5);
            let right = super::wheel_force(&f, x, x, &mut t, 1.0, 0.0);
            let left = super::wheel_force(&f, -x, -x, &mut t, 1.0, 0.0);
            assert!(right < -0.05, "range={range}: {right}");
            assert_eq!(right, -left);
        }
    }

    #[test]
    fn small_remaining_angles_keep_returning_while_the_bus_rolls() {
        let mut t = 0.0;
        for kmh in [5.0, 20.0, 70.0] {
            let f = super::FfInput { on: true, kmh, dt: 1.0 / 60.0, ..Default::default() };
            for x in [0.005, 0.01, 0.05] {
                // Already returning slowly: do not let steering drag cancel
                // the last few degrees of return on a wheel with friction.
                let right = super::wheel_force(&f, x, x + 0.001, &mut t, 1.0, 0.0);
                let left = super::wheel_force(&f, -x, -x - 0.001, &mut t, 1.0, 0.0);
                assert!(right < -0.02 && left > 0.02, "kmh={kmh}, x={x}: {right} {left}");
                assert!((right + left).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn return_torque_fades_smoothly_at_the_physical_centre() {
        let mut t = 0.0;
        let f = super::FfInput { on: true, kmh: 30.0, dt: 0.016, ..Default::default() };
        let mut previous: f32 = 1.0;
        for x in [0.2, 0.1, 0.01, 0.001, 0.0001, 0.00001, 0.0] {
            let force = super::wheel_force(&f, x, x, &mut t, 1.0, 0.0);
            assert!(force.abs() < previous, "x={x}: {force}");
            if x == 0.0 {
                assert_eq!(force, 0.0);
            } else {
                assert!(force < 0.0);
            }
            if x <= 0.0001 {
                assert!(force.abs() < 0.002, "x={x}: {force}");
            }
            previous = force.abs();
        }
    }

    #[test]
    fn a_moving_wheel_is_damped_when_it_crosses_the_centre() {
        let mut t = 0.0;
        for kmh in [5.0, 20.0, 70.0] {
            let f = super::FfInput { on: true, kmh, dt: 0.02, ..Default::default() };
            // At the centre there is no spring torque. The remaining force must
            // slow a crossing wheel in either direction rather than accelerate it.
            let right = super::wheel_force(&f, 0.0, -0.01, &mut t, 1.0, 0.0);
            let left = super::wheel_force(&f, 0.0, 0.01, &mut t, 1.0, 0.0);
            assert!(right < -0.015 && left > 0.015, "kmh={kmh}: {right} {left}");
            assert!((right + left).abs() < 1e-6);
            assert_eq!(super::wheel_force(&f, 0.0, 0.0, &mut t, 1.0, 0.0), 0.0);
            // Steering strength controls damping too, independently of vibration.
            assert_eq!(super::wheel_force(&f, 0.0, -0.01, &mut t, 0.0, 0.0), 0.0);
        }
    }

    #[test]
    fn the_same_wheel_velocity_gives_the_same_force_at_different_frame_rates() {
        let mut t = 0.0;
        for velocity in [-0.5, 0.5] {
            let mut reference: Option<f32> = None;
            for fps in [20.0, 30.0, 60.0, 144.0] {
                let dt = 1.0 / fps;
                let f = super::FfInput { on: true, kmh: 30.0, dt, ..Default::default() };
                let force = super::wheel_force(&f, 0.05, 0.05 - velocity * dt, &mut t, 1.0, 0.0);
                if let Some(previous) = reference {
                    assert!((force - previous).abs() < 1e-6, "fps={fps}: {force} {previous}");
                } else {
                    reference = Some(force);
                }
            }
        }
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
    fn steering_assist_lightens_turning_out() {
        let mut t = 0.0;
        let f = super::FfInput { on: true, kmh: 25.0, lateral_accel: 2.0, dt: 0.016, ..Default::default() };
        let held = super::wheel_force(&f, 0.5, 0.5, &mut t, 1.0, 0.0);
        let turning_out = super::wheel_force(&f, 0.5, 0.48, &mut t, 1.0, 0.0);
        let returning = super::wheel_force(&f, 0.5, 0.52, &mut t, 1.0, 0.0);
        // Assistance still lightens turning out. A moving return also includes
        // damping, so its net torque need not exceed the outward-turning torque.
        assert!(held < turning_out && turning_out < 0.0, "{held} {turning_out}");
        assert!(returning < 0.0, "{returning}");
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
        assert_eq!(super::bump_strength(0.008, 0.08, 20.0), 0.0);
        assert!(super::bump_strength(0.015, 0.0, 20.0) < 0.05);
        assert!(super::bump_strength(0.05, 0.0, 20.0) > 0.4);
        assert!(super::bump_strength(0.1, 0.0, 20.0) > 0.9);
        assert!(super::bump_strength(0.0, 1.0, 20.0) > 0.5);
    }

    #[test]
    fn settling_ignores_seams_and_keeps_slow_motion_as_it_was() {
        let dt = 1.0 / 60.0;
        let mut settled = Vec::new();
        // the first frame (or the first back in the bus) starts from the travel as it is
        assert_eq!(super::settle(&mut settled, &[0.09], dt), vec![0.0]);
        // a 1.5 cm seam for one step
        let seam = super::settle(&mut settled, &[0.105], dt)[0];
        assert!(super::bump_strength(seam, 0.0, 20.0) < 0.05);
        // a 6 cm step (a pothole's edge)
        let mut settled = vec![0.0];
        let step = super::settle(&mut settled, &[0.06], dt)[0];
        assert!(super::bump_strength(step, 0.0, 20.0) > 0.45);
        // brake dive at 0.3 m/s: about as strong as the old one-step rate made it
        let mut settled = vec![0.0];
        let mut travel = 0.0;
        let mut ripple = 0.0;
        for _ in 0..60 {
            travel += 0.3 * dt;
            ripple = super::settle(&mut settled, &[travel], dt)[0];
        }
        let old = (0.3 - 0.12) / 0.9;
        assert!((super::bump_strength(ripple, 0.0, 20.0) - old).abs() < 0.05, "{ripple}");
    }

    /// Where the wheel plays the shaking as its own periodic effect, the force set each
    /// frame carries none of it.
    #[test]
    fn the_shaking_leaves_the_force_when_the_wheel_plays_it() {
        let f = super::FfInput { on: true, kmh: 0.0, vib_amp: 1.0, vib_period: 2.5, dt: 0.016, ..Default::default() };
        let mut t = 0.0;
        let soft = super::wheel_force(&f, 0.0, 0.0, &mut t, 0.0, 1.0);
        assert!(soft.abs() > 0.01, "{soft}");
        t = 0.0;
        let periodic = super::wheel_force(&super::FfInput { vib_amp: 0.0, ..f }, 0.0, 0.0, &mut t, 0.0, 1.0);
        assert_eq!(periodic, 0.0);
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

    /// The largest the trembling of the tarmac and the engine may become (of the wheel's
    /// full lock) - a wheel's centring spring is several times this, and the scripts' own
    /// shaking is 0.25, so the tremble stays felt as a buzz and not as a push.
    const MICRO_MAX: f32 = 0.25;

    /// Ten seconds of the road at `kmh`, as sampled by the game.
    fn road(kmh: f32) -> Vec<f32> {
        let mut m = super::Micro::default();
        (0..600).map(|_| m.sample(1.0 / 60.0, kmh, 0.0, 0.0, 0.0, 1.0, 0.0)).collect()
    }

    /// The most the signal ever lines up with itself, at any lag up to two seconds. Sines
    /// line themselves up; noise cannot, so this is what tells a road carrying a weight
    /// from a road rattling.
    fn self_similarity(v: &[f32]) -> f32 {
        let rms = (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt();
        let mut best = 0.0f32;
        for lag in 4..120.min(v.len()) {
            let mut num = 0.0f32;
            for i in lag..v.len() {
                num += v[i] * v[i - lag];
            }
            best = best.max((num / ((v.len() - lag) as f32) / rms / rms).abs());
        }
        best
    }

    #[test]
    fn the_road_carries_a_weight_rather_than_a_rattle() {
        // A wheel is told one force at a time, and a force that repeats itself tells the
        // driver there is something solid underneath the bus, while one that never does
        // tells them the surface is broken up into loose pieces - which is what the
        // complaint was: a road made of nothing but noise reads as random however fast it
        // is felt. Noise scores about 0.2 here at any speed and the ride about 0.8, so
        // the grain left under it cannot pull this down where it matters.
        for kmh in [15.0, 30.0, 50.0, 80.0] {
            let alike = self_similarity(&road(kmh));
            assert!(alike > 0.5, "the road at {kmh} km/h carries nothing solid: {alike:.2}");
        }
    }
    #[test]
    fn the_road_is_under_the_tyres_and_not_merely_slow() {
        // How often the road changes is what the tyre feels, and it has to outrun the
        // hands: the rim goes wherever it is told and the hands go with it, so a change a
        // few times a second is a push to be dragged along by rather than a texture. The
        // short layer is what keeps this true as the bus speeds up.
        let rate = |v: &[f32]| {
            let mut crossings = 0;
            for i in 1..v.len() {
                if (v[i] > 0.0) != (v[i - 1] > 0.0) {
                    crossings += 1;
                }
            }
            crossings as f32 / (v.len() as f32 / 60.0) / 2.0
        };
        assert!(rate(&road(30.0)) > 6.0, "the road in town is too slow to feel: {}", rate(&road(30.0)));
        assert!(rate(&road(50.0)) > 10.0, "the road at 50 is too slow to feel: {}", rate(&road(50.0)));
        assert!(rate(&road(80.0)) > 13.0, "the road does not quicken with the speed: {}", rate(&road(80.0)));
    }


    #[test]
    fn the_tremble_stays_silent_at_a_standstill_and_appears_with_the_speed() {
        // how much is felt is the average of the tremble, not its loudest frame: the
        // grain is noise, and its peaks fall where the pattern happens to be
        let felt = |m: &mut super::Micro, kmh: f32| {
            (0..240).map(|_| m.sample(1.0 / 60.0, kmh, 0.0, 0.0, 0.0, 1.0, 0.0).abs()).sum::<f32>() / 240.0
        };
        let mut m = super::Micro::default();
        assert_eq!(felt(&mut m, 0.0), 0.0, "a bus standing still feels no grain at all");
        let crawling = felt(&mut m, 5.0);
        let town = felt(&mut m, 30.0);
        let road = felt(&mut m, 50.0);
        let fast = felt(&mut m, 80.0);
        assert!(crawling > 0.0 && town > crawling * 2.0, "{crawling} {town}");
        // the finer layer is let go above 40 Hz and the coarser one carries the road
        // there, so the strength holds from town speed up rather than falling away
        assert!(road > town * 0.9, "the grain should be at its strongest by 30 km/h: {town} {road}");
        assert!(fast > road * 0.9, "the grain thins out at speed: {fast} against {road}");
        assert!(fast < MICRO_MAX, "the grain grows too strong: {fast}");
    }

    #[test]
    fn the_tremble_is_read_along_the_road_not_along_the_clock() {
        let mut m = super::Micro::default();
        // two seconds at 50 km/h over one stretch of tarmac, then the same stretch again
        let over = |m: &mut super::Micro| (0..120).map(|_| m.sample(1.0 / 60.0, 50.0, 0.0, 0.0, 0.0, 1.0, 0.0)).collect::<Vec<_>>();
        let first = over(&mut m);
        assert!(first.iter().any(|v| v.abs() > 0.001));
        let start = m.roll;
        assert!(start > 20.0, "the distance the bus rolled was not kept: {start}");
        m.roll = 0.0;
        assert_eq!(over(&mut m), first, "the same stretch of road shook differently");
        // a long drive does not walk the pattern off into a constant
        m.roll = 2_000_000.0;
        assert!(over(&mut m).iter().any(|v| v.abs() > 0.001), "the grain died out after a long drive");
    }

    #[test]
    fn a_wet_or_snowy_road_hums_more_than_dry_asphalt() {
        let felt = |cond: f32| {
            let mut m = super::Micro::default();
            (0..120).map(|_| m.sample(1.0 / 60.0, 50.0, cond, 0.0, 0.0, 1.0, 0.0).abs()).sum::<f32>() / 120.0
        };
        let (dry, wet, snow) = (felt(0.0), felt(1.0), felt(2.0));
        assert!(wet > dry && snow > wet, "{dry} {wet} {snow}");
    }

    #[test]
    fn the_engine_is_felt_idling_and_grows_with_what_it_is_doing() {
        let mut m = super::Micro::default();
        let off = (0..60).map(|_| m.sample(1.0 / 60.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0)).fold(0.0f32, f32::max);
        assert_eq!(off, 0.0, "an engine that is not running is felt through nothing");
        let peak = |rpm: f32, load: f32| {
            let mut m = super::Micro::default();
            // two seconds, so the engine's speed is settled and its angle has turned
            (0..120).map(|_| m.sample(1.0 / 60.0, 0.0, 0.0, rpm, load, 0.0, 1.0)).fold(0.0f32, f32::max)
        };
        let idling = peak(700.0, 0.0);
        let working = peak(700.0, 1.0);
        let pulling = peak(2200.0, 1.0);
        assert!(idling > 0.001, "a running engine at idle is not felt at all: {idling}");
        assert!(working > idling, "a loaded engine is not felt harder than an idling one: {idling} {working}");
        assert!(pulling > working, "a fast engine is not felt harder than a slow one: {working} {pulling}");
        assert!(pulling < MICRO_MAX, "the engine shakes the wheel too hard: {pulling}");
    }

    #[test]
    fn the_engine_leads_on_the_crankshaft_and_stays_under_the_aliasing_limit() {
        // six seconds of an engine at 700 rpm, which is where the buzz is at its most
        // characteristic; the whole of it is a sum of sines, so projecting the signal on
        // each frequency that is in it says how the sound is shared out between them
        let dt = 1.0f32 / 60.0;
        let mut m = super::Micro::default();
        let sig: Vec<f32> = (0..360).map(|_| m.sample(dt, 0.0, 0.0, 700.0, 0.5, 0.0, 1.0)).collect();
        let partial = |f: f32| {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, &x) in sig.iter().enumerate() {
                let w = std::f32::consts::TAU * f * dt * i as f32;
                re += x * w.cos();
                im += x * w.sin();
            }
            (re * re + im * im).sqrt() / sig.len() as f32 * 2.0
        };
        // at 700 rpm the crankshaft turns at 11.7 Hz and a four-stroke four fires at
        // 23.3 Hz; the rumble under them is at 2.9 Hz. What leads is the crankshaft.
        let rumble = partial(700.0 / 240.0);
        let crank = partial(700.0 / 60.0);
        let firing = partial(700.0 / 30.0);
        assert!(crank > rumble, "the buzz leads on the low rumble instead of the engine: {rumble} {crank}");
        assert!(crank > firing, "the firing pulses lead instead of the engine turning: {crank} {firing}");
        // a wheel is told its force once a frame, so nothing the engine does may ask for
        // a frequency the frame cannot carry
        for f in [700.0 / 240.0, 700.0 / 60.0, 700.0 / 30.0] {
            assert!(f < 28.0, "the engine asks for {f} Hz, past what a wheel can be told");
        }
        // and at the top of the rev range, where the crank and the pulses have both been
        // let go, what is left is the rumble and not silence
        let mut m = super::Micro::default();
        let top: Vec<f32> = (0..360).map(|_| m.sample(dt, 0.0, 0.0, 4500.0, 1.0, 0.0, 1.0)).collect();
        let mut im = 0.0f32;
        for (i, &x) in top.iter().enumerate() {
            im += x * (std::f32::consts::TAU * (4500.0 / 240.0) * dt * i as f32).sin();
        }
        let mass = (im / top.len() as f32).abs();
        assert!(mass > 0.01, "a flat-out engine is felt as nothing: {mass}");
    }

    #[test]
    fn the_tremble_is_turned_off_by_its_settings() {
        let run = |road: f32, engine: f32| {
            let mut m = super::Micro::default();
            (0..120).map(|_| m.sample(1.0 / 60.0, 60.0, 0.0, 900.0, 0.5, road, engine)).fold(0.0f32, f32::max)
        };
        assert_eq!(run(0.0, 0.0), 0.0);
        assert!(run(1.0, 0.0) > 0.0 && run(0.0, 1.0) > 0.0);
        assert!(run(2.0, 2.0) > run(1.0, 1.0), "the settings do not make it stronger");
        // a bigger engine buzz is not a road that got rougher
        let mut road_only = super::Micro::default();
        let mut both = super::Micro::default();
        for _ in 0..60 {
            road_only.sample(1.0 / 60.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0);
            both.sample(1.0 / 60.0, 0.0, 0.0, 900.0, 0.0, 1.0, 1.0);
        }
        assert_eq!(road_only.roll, both.roll, "the engine changed how the road is read");
    }

    #[test]
    fn the_tremble_comes_out_of_the_wheel_through_the_vibration_strength() {
        let mut t = 0.0;
        let f = super::FfInput { on: true, kmh: 50.0, micro: 0.08, dt: 0.016, ..Default::default() };
        assert_eq!(super::wheel_force(&f, 0.0, 0.0, &mut t, 1.0, 0.0), 0.0, "it shook the wheel with the vibrations switched off");
        let felt = super::wheel_force(&f, 0.0, 0.0, &mut t, 1.0, 1.0);
        assert!((felt - 0.08).abs() < 0.0001, "{felt}");
        assert_eq!(super::wheel_force(&super::FfInput { micro: 0.5, ..f }, 0.0, 0.0, &mut t, 1.0, 1.0), 0.5);
    }

    #[test]
    fn a_vibration_eases_away_once_it_stops() {
        assert_eq!(super::fade_gain(0.0, super::FF_FADE), 1.0);
        assert_eq!(super::fade_gain(super::FF_FADE * 0.5, super::FF_FADE), 0.5);
        assert_eq!(super::fade_gain(super::FF_FADE, super::FF_FADE), 0.0);
        assert_eq!(super::fade_gain(super::FF_FADE * 3.0, super::FF_FADE), 0.0);
        // it never jumps: no frame hands the wheel a step of its own
        let mut last = 1.0;
        for i in 0..40 {
            let g = super::fade_gain(i as f32 / 40.0 * super::FF_FADE * 1.2, super::FF_FADE);
            assert!(g <= last && last - g < 0.15, "{g} after {last}");
            last = g;
        }
        // a fade of nothing is the old way: it stops where it stands
        assert_eq!(super::fade_gain(0.5, 0.0), 1.0);
    }

    #[test]
    fn the_settings_fade_is_how_long_a_jolt_takes_to_go() {
        // The jolt the physics hands over is not faded by a gain applied afterwards: by the
        // time such a gain would start, the jolt it was following is already spent. So the
        // jolt is held and eased on the settings' own time, and this is what says whether
        // the setting reaches the jolt at all.
        let dt = 1.0f32 / 60.0;
        let tail = |fade: f32| {
            let mut held = 0.6f32;
            let mut frames = 0;
            for _ in 0..600 {
                held = held * (-dt / (fade * 0.3)).exp();
                frames += 1;
                if held < 0.002 {
                    break;
                }
            }
            frames
        };
        let fast = tail(0.05);
        let slow = tail(0.6);
        assert!(fast < slow * 3, "the setting barely changes the jolt: {fast} {slow}");
        assert!(fast >= 2, "the jolt was cut off in a single frame: {fast}");
        assert!(slow <= 120, "a long fade never ends: {slow}");
    }

    #[test]
    fn a_scripts_shaking_eases_away_instead_of_stopping_dead() {
        // The scripts write the amplitude afresh every frame, so on the frame they stop it
        // is already zero and fading that fades nothing: the wheel drops the shake in a
        // single step, which is the jolt the fade is here to avoid. What the fade needs is
        // the last amplitude the scripts reached the wheel with, so this is what says
        // whether it is still there once they have gone quiet.
        let dt = 1.0f32 / 60.0;
        let mut vib = super::ScriptVib::default();
        let (amp, period) = vib.step(true, 0.8, 12.0, super::FF_FADE, dt);
        assert_eq!(amp, 0.8, "the shake the scripts asked for never reached the wheel");
        assert_eq!(period, 12.0, "the period the scripts asked for was not passed on");
        // the frame the scripts stop: kept, not cut off
        let (mut amp, period) = vib.step(true, 0.0, 0.0, super::FF_FADE, dt);
        assert!(amp > 0.5, "the shake stopped dead the frame the scripts did: {amp}");
        assert_eq!(period, 12.0, "the period went with the amplitude instead of being kept");
        // and it eases away to nothing rather than to a stop
        let mut frames = 0;
        while amp > 0.0 && frames < 600 {
            amp = vib.step(true, 0.0, 0.0, super::FF_FADE, dt).0;
            frames += 1;
        }
        assert_eq!(amp, 0.0, "the shake never finished fading: {amp}");
        // the setting is what decides how long that takes
        let mut fast = super::ScriptVib::default();
        fast.step(true, 0.8, 12.0, 0.05, dt);
        let mut quick = 0;
        while fast.step(true, 0.0, 0.0, 0.05, dt).0 > 0.0 && quick < 600 {
            quick += 1;
        }
        assert!(quick < frames, "the setting barely changes the shake: {quick} {frames}");
        // with the fade at zero it stops where it stands, as it always did
        let mut none = super::ScriptVib::default();
        none.step(true, 0.8, 12.0, 0.0, dt);
        assert_eq!(none.step(true, 0.0, 0.0, 0.0, dt).0, 0.0, "the fade at zero still hangs on");
        // and a bus whose force feedback is off is not shaking at all
        let mut off = super::ScriptVib::default();
        off.step(true, 0.8, 12.0, super::FF_FADE, dt);
        assert_eq!(off.step(false, 0.0, 0.0, super::FF_FADE, dt).0, 0.0);
    }
}

#[cfg(test)]
mod device_kind_tests {
    #[test]
    fn a_mapped_constant_force_wheel_is_not_a_gamepad() {
        assert!(super::mapped_device_is_gamepad(true, false));
        assert!(!super::mapped_device_is_gamepad(true, true));
        assert!(!super::mapped_device_is_gamepad(false, true));
    }
}

#[cfg(test)]
mod stick_deadzone_tests {
    use super::stick_deadzone_with;

    #[test]
    fn the_dead_zone_is_the_settings_and_the_rest_of_the_stick_still_reaches_one() {
        assert_eq!(stick_deadzone_with(0.14, 0.15), 0.0);
        assert!(stick_deadzone_with(0.3, 0.15) > 0.0);
        assert!((stick_deadzone_with(-1.0, 0.15) + 1.0).abs() < 1e-6);
        assert_eq!(stick_deadzone_with(0.05, 0.0), 0.05);
    }
}

#[cfg(test)]
mod stick_steering_tests {
    #[test]
    fn an_idle_device_nobody_set_up_does_not_hold_the_sticks_steering() {
        assert!(super::stick_steers(None, false, 0.0));
        assert!(super::stick_steers(Some(0.0), false, -0.6));
        assert!(super::stick_steers(Some(0.0), false, 0.0));
        assert!(super::stick_steers(Some(0.02), false, 0.0));
        assert!(!super::stick_steers(Some(0.8), false, 0.3));
        assert!(!super::stick_steers(Some(0.0), true, 1.0));
    }
}

#[cfg(test)]
mod right_stick_look_tests {
    #[test]
    fn disabling_automatic_look_keeps_driving_controls_and_assigned_look() {
        for assigned in [[0.0, 0.0], [0.5, -0.25]] {
            let mut analog = super::Analog { steering: Some(0.3), stick: true, throttle: Some(0.8), brake: Some(0.2), clutch: Some(0.4), look: assigned };
            analog.apply_default_gamepad_look(false, 1.0, -1.0);
            assert_eq!(analog.look, assigned);
            assert_eq!(analog.steering, Some(0.3));
            assert!(analog.stick);
            assert_eq!(analog.throttle, Some(0.8));
            assert_eq!(analog.brake, Some(0.2));
            assert_eq!(analog.clutch, Some(0.4));
        }
    }

    #[test]
    fn enabled_automatic_look_keeps_dead_zone_direction_and_assigned_axes() {
        let mut analog = super::Analog::default();
        analog.apply_default_gamepad_look(true, 0.1, -0.1);
        assert_eq!(analog.look, [0.0, 0.0]);
        analog.apply_default_gamepad_look(true, 1.0, -1.0);
        assert_eq!(analog.look, [1.0, 1.0]);
        analog.look = [0.25, -0.5];
        analog.apply_default_gamepad_look(true, -1.0, 1.0);
        assert_eq!(analog.look, [0.25, -0.5]);
    }
}

#[cfg(test)]
mod hot_reload_tests {
    use super::*;

    fn wheel() -> DeviceCfg {
        let mut d = DeviceCfg { name: "Test wheel".into(), second: "0".into(), ff_scale: Some((0.65, 1.25)), ff_invert: Some(true), ..Default::default() };
        d.axes[0] = Some((Func::Steering, true));
        d.axes[1] = Some((Func::Throttle, true));
        d.axes[5] = Some((Func::Brake, false));
        d.axis_flags[0] = 2 | 8 | 0x10;
        d.buttons = vec![("horn".into(), "7".into()), ("kw_s_1_fest".into(), "0".into()), ("blinker_warn_toggle".into(), "0".into())];
        d.latching = vec![2];
        d
    }

    fn no_hardware() -> Devices {
        Devices {
            gilrs: None,
            #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
            calibration_wheel: None,
            #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
            ff_wheels: Default::default(),
            #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
            button_devices: crate::evdev_buttons::ButtonDevices::new(),
            #[cfg(windows)]
            di: None,
            #[cfg(target_os = "macos")]
            gc: crate::mac_game_controller::GcPads::new(),
            #[cfg(target_os = "macos")]
            hid: None,
            #[cfg(target_os = "macos")]
            hid_axes: Vec::new(),
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            hats: Vec::new(),
            pad_buttons: Vec::new(),
        }
    }

    #[test]
    fn hot_reload_keeps_device_owner_and_ffb_runtime_state() {
        let mut c = Controllers::with_devices(no_hardware(), vec![wheel()]);
        c.ff_t = 12.0;
        c.ff_lateral = 0.25;
        c.ff_bump = 0.75;
        c.ff_bump_age = 0.125;
        c.ff_micro = Micro { roll: 10.0, rpm: 850.0, crank: 0.1, firing: 0.2, mass: 0.3 };
        c.ff_vib = ScriptVib { amp: 0.4, period: 2.0, t: 0.12 };
        c.ff_rumble = 0.6;
        c.steer = Some(("Test wheel".into(), 0.2, 0.1, true));
        c.settled = vec![0.3, 0.4];
        let devices = std::ptr::addr_of!(c.devices);
        c.held.event(&c.cfg, "Test wheel", 1, true, &mut c.actions);
        let mut cfg = c.configuration();
        cfg[0].ff_invert = Some(false);
        c.install_cfg(cfg);
        assert_eq!(std::ptr::addr_of!(c.devices), devices);
        assert_eq!((c.ff_t, c.ff_lateral, c.ff_bump, c.ff_bump_age), (12.0, 0.25, 0.75, 0.125));
        assert_eq!(c.steer, Some(("Test wheel".into(), 0.2, 0.1, true)));
        assert_eq!(c.settled, vec![0.3, 0.4]);
        assert_eq!((c.ff_micro.roll, c.ff_micro.rpm, c.ff_micro.crank, c.ff_micro.firing, c.ff_micro.mass), (10.0, 850.0, 0.1, 0.2, 0.3));
        assert_eq!((c.ff_vib.amp, c.ff_vib.period, c.ff_vib.t, c.ff_rumble), (0.4, 2.0, 0.12, 0.6));
        assert_eq!(c.actions.last(), Some(&("kw_s_1_fest".into(), false)));
        assert_eq!(c.configuration()[0].ff_invert, Some(false));
    }

    #[test]
    fn a_device_nobody_set_up_steers_only_once_its_axis_moved() {
        let mut moved = Vec::new();
        // idle at its centre (or a little off it): it does not steer, so the arrow keys
        // switch the interior camera as with no device at all
        assert!(!free_axis_steers(&mut moved, "4 axes, 25 buttons, joystick", 0.0));
        assert!(!free_axis_steers(&mut moved, "4 axes, 25 buttons, joystick", -0.1));
        // turned: it steers, also when back at the centre
        assert!(free_axis_steers(&mut moved, "4 axes, 25 buttons, joystick", 0.4));
        assert!(free_axis_steers(&mut moved, "4 axes, 25 buttons, joystick", 0.0));
        assert!(!free_axis_steers(&mut moved, "another joystick", 0.0));
    }

    #[test]
    fn hot_reload_editing_suppresses_input_and_releases_once() {
        let mut c = Controllers::with_devices(no_hardware(), vec![wheel()]);
        c.held.event(&c.cfg, "Test wheel", 0, true, &mut c.actions);
        c.set_editing(true);
        c.set_editing(true);
        c.poll();
        assert!(c.editing);
        assert_eq!(c.actions, vec![("horn".into(), true), ("horn".into(), false)]);
        c.set_editing(false);
        assert!(!c.editing);
    }

    #[test]
    fn hot_reload_releases_original_held_gear_and_does_not_press_new_binding() {
        let mut held = HeldButtons::default();
        let mut cfg = vec![wheel()];
        let mut actions = Vec::new();
        held.event(&cfg, "Test wheel", 1, true, &mut actions);
        cfg[0].buttons[1].0 = "horn".into();
        held.release(&mut actions);
        held.event(&cfg, "Test wheel", 1, false, &mut actions);
        assert_eq!(actions, vec![("kw_s_1_fest".into(), true), ("kw_s_1_fest".into(), false)]);
        held.event(&cfg, "Test wheel", 1, true, &mut actions);
        assert_eq!(actions.last(), Some(&("horn".into(), true)));
    }

    #[test]
    fn hot_reload_does_not_toggle_latching_switch_on_menu_entry() {
        let mut held = HeldButtons::default();
        let cfg = vec![wheel()];
        let mut actions = Vec::new();
        held.event(&cfg, "Test wheel", 2, true, &mut actions);
        held.event(&cfg, "Test wheel", 2, true, &mut actions); // duplicate down
        held.release(&mut actions);
        held.event(&cfg, "Test wheel", 2, false, &mut actions);
        assert_eq!(actions, vec![("blinker_warn_toggle".into(), true), ("blinker_warn_toggle".into(), false)]);
        // A physical release during driving still performs the existing latching behaviour.
        held.event(&cfg, "Test wheel", 2, true, &mut actions);
        held.event(&cfg, "Test wheel", 2, false, &mut actions);
        assert_eq!(actions.len(), 6);
    }

    #[test]
    fn hot_reload_release_uses_mapping_at_press_even_after_external_change() {
        let mut held = HeldButtons::default();
        let mut cfg = vec![wheel()];
        let mut actions = Vec::new();
        held.event(&cfg, "Test wheel", 0, true, &mut actions);
        cfg[0].buttons[0].0 = "door".into();
        held.event(&cfg, "Test wheel", 0, false, &mut actions);
        assert_eq!(actions, vec![("horn".into(), true), ("horn".into(), false)]);
    }

    #[test]
    fn hot_reload_preserves_axis_flags_button_metadata_and_per_device_ffb() {
        let cfg = vec![wheel(), DeviceCfg { name: "Separate pedals".into(), second: "12".into(), ff_scale: Some((1.0, 1.0)), ..Default::default() }];
        assert_eq!(parse_cfg(&cfg_text(&cfg)), cfg);
        assert_eq!(cfg[0].axis_flags[0], 26);
        let (steer, physical) = wheel_steering(0.6, true, 0, 0.1, 1.0);
        assert!((physical + 0.6).abs() < 1e-6);
        assert!((steer + 0.5 / 0.9).abs() < 1e-6);
    }

    #[test]
    fn hot_reload_mixed_devices_keep_the_selected_steering_kind() {
        let mut out = Analog::default();
        set_steering(&mut out, 0.75, false);
        set_steering(&mut out, 0.0, true);
        assert_eq!(out.steering, Some(0.75));
        assert!(!out.stick);
        set_steering(&mut out, -0.9, true);
        assert_eq!(out.steering, Some(-0.9));
        assert!(out.stick);
        set_steering(&mut out, -0.9, false);
        assert!(out.stick); // identical readings keep the original source
    }

    #[test]
    fn hot_reload_gamepad_vibration_uses_its_own_device_scale() {
        let mut cfg = vec![wheel(), DeviceCfg { name: "Xbox Controller".into(), ff_scale: Some((1.0, 0.5)), ..Default::default() }];
        assert_eq!(rumble_scale(&cfg, "Xbox Controller"), 0.5);
        cfg[1].ff_scale = Some((1.0, 1.5));
        assert_eq!(rumble_scale(&cfg, "Xbox Controller"), 1.5);
        assert_eq!(rumble_scale(&cfg, "Test wheel"), 1.25);
        assert_eq!(rumble_scale(&cfg, "Unconfigured gamepad"), 1.0);
    }

    #[test]
    fn hot_reload_custom_gamepad_layout_owns_analog_controls() {
        let mut cfg = vec![DeviceCfg { name: "Xbox Controller".into(), second: "0".into(), ..Default::default() }];
        cfg[0].buttons.push(("horn".into(), "0".into()));
        assert!(!custom_gamepad_axes(&cfg, "Xbox Controller"));
        cfg[0].axes[0] = Some((Func::Steering, false));
        assert!(custom_gamepad_axes(&cfg, "Xbox Controller"));
        let mut axes = vec![(0, 0.2), (1, -0.1)];
        gamepad_triggers(&mut axes, 0.0, 1.0);
        assert_eq!(axes[2..], [(6, -1.0), (7, 1.0)]);
        gamepad_triggers(&mut axes, 1.0, 0.0);
        assert_eq!(axes.len(), 4);
    }

    #[test]
    fn hot_reload_save_replaces_existing_file_and_rejects_invalid_input() {
        let dir = std::env::temp_dir().join(format!("openomsi-controllers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gamectrler.cfg");
        let mut cfg = vec![wheel()];
        save_cfg_to(&path, &cfg).unwrap();
        cfg[0].ff_invert = Some(false);
        save_cfg_to(&path, &cfg).unwrap();
        assert_eq!(parse_cfg(&std::fs::read_to_string(&path).unwrap()), cfg);
        let original = std::fs::read(&path).unwrap();
        cfg[0].buttons[0].0 = "horn\n[ctrl]".into();
        assert!(save_cfg_to(&path, &cfg).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(save_cfg_to(&path.join("not-a-directory.cfg"), &[]).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
