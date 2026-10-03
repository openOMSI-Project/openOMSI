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
    #[cfg(windows)]
    di: Option<crate::dinput::DirectInput>,
    /// macOS: every axis element of every wheel and joystick, as last read (see `mac_hid`)
    #[cfg(target_os = "macos")]
    hid: Option<crate::mac_hid::MacHid>,
    #[cfg(target_os = "macos")]
    hid_axes: Vec<(String, Vec<(u32, f32)>)>,
    #[cfg(target_os = "linux")]
    hats: Vec<(String, [i8; 8])>,
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
            #[cfg(windows)]
            di,
            #[cfg(target_os = "macos")]
            hid: crate::mac_hid::MacHid::new(),
            #[cfg(target_os = "macos")]
            hid_axes: Vec::new(),
            #[cfg(target_os = "linux")]
            hats: Vec::new(),
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
            while let Some(ev) = g.next_event() {
                let pad = g.gamepad(ev.id);
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
                    #[cfg(target_os = "linux")]
                    EventType::AxisChanged(_, value, code) if code.into_u32() >> 16 == 3 && (0x10..0x18).contains(&(code.into_u32() & 0xFFFF)) => {
                        let axis = (code.into_u32() & 0xFFFF) as usize - 0x10;
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
                        gamepad: false,
                        ff: d.has_ff(),
                        ff_capable: d.ff_capable(),
                        buttons: d.buttons.min(128),
                    }),
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
                v.push(Connected { name: pad.name().to_string(), hardware_id: id, axes: di_slots(&axes), gamepad, ff: pad.is_ff_supported(), ff_capable: pad.is_ff_supported(), buttons });
            }
        }
        // (and a wheel gilrs does not list at all: one whose only axes are the simulation
        // page's steering and pedals)
        #[cfg(target_os = "macos")]
        for (name, axes) in &self.hid_axes {
            if !v.iter().any(|c| names_match(&c.name, name)) {
                v.push(Connected { name: name.clone(), hardware_id: None, axes: di_slots(axes), gamepad: false, ff: false, ff_capable: false, buttons: 0 });
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
    pub dt: f32,
}

pub struct Controllers {
    /// Each wheel's suspension travel, settled over a tenth of a second (see `wheel_bump`).
    settled: Vec<f32>,
    devices: Devices,
    focused: bool,
    cfg: Vec<DeviceCfg>,
    pub enabled: bool,
    /// The settings' dead zone round the centre of a set-up device's axes (0..0.3).
    pub deadzone: f32,
    /// Automatic right-stick camera movement (Settings: `right_stick_look`).
    pub right_stick_look: bool,
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
    /// The steering device: its name, physical position (-1..1, before dead zone
    /// and steering gain) now and previously, and whether it pushes back.
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
        Controllers { settled: Vec::new(), devices, focused: true, cfg, deadzone: 0.0, right_stick_look: true, pedal_throttle: 1.0, pedal_brake: 1.0, disabled: Vec::new(), ff_invert: false, ff_enabled: true, steer_gain: 1.0, enabled: true, actions: Vec::new(), announced: Vec::new(), notice: None, steer: None, ff_t: 0.0, ff_lateral: 0.0, ff_bump: 0.0, ff_bump_age: 0.0, ff_source_logged: None, rumble: None, #[cfg(all(target_os = "linux", target_pointer_width = "64"))] wheel: None, #[cfg(all(target_os = "linux", target_pointer_width = "64"))] wheel_tried: None }
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
            let Some(d) = find_device_cfg(&self.cfg, &name) else { continue };
            if let Some(action) = d.buttons.get(n).filter(|a| !a.0.is_empty()) {
                self.actions.push((action.0.clone(), down));
                // a latching switch coming out switches back: pressed in again it would
                // only have toggled the hazard lights on the next press, and the lever's
                // turn signal stayed on in the middle
                if !down && d.latching.contains(&n) {
                    let back = latch_release(&action.0);
                    self.actions.push((back.clone(), true));
                    self.actions.push((back, false));
                }
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
                            let (steering, position) = wheel_steering(v, inverted, d.axis_flags[k], dz, self.steer_gain);
                            set(&mut out.steering, steering);
                            if steer.is_none() {
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
                if pad.mapping_source() == gilrs::MappingSource::None || (!xinput && self.cfg.iter().any(|d| names_match(&d.name, pad.name()))) {
                    continue;
                }
                #[cfg(target_os = "macos")]
                if self.devices.hid_wheel(pad.name()) {
                    continue;
                }
                if (di && !xinput) || off.iter().any(|d| names_match(d, pad.name())) {
                    continue;
                }
                let x = pad.value(Axis::LeftStickX);
                let dead = |v: f32| if v.abs() < 0.08 { 0.0 } else { v };
                // (said once per pad: the stick moved, and whether it steers - a report of
                // "the sticks do nothing" then says which way the pad came in)
                if x.abs() > 0.5 && !self.announced.iter().any(|n| n == &format!("stick:{}", pad.name())) {
                    self.announced.push(format!("stick:{}", pad.name()));
                    log::info!("game controller {}: left stick {x:.2}, steers: {} (layout {:?})", pad.name(), out.steering.is_none(), pad.mapping_source());
                }
                let rt = pad.button_data(gilrs::Button::RightTrigger2).map(|d| d.value()).unwrap_or(0.0);
                let lt = pad.button_data(gilrs::Button::LeftTrigger2).map(|d| d.value()).unwrap_or(0.0);
                if out.steering.is_none() {
                    out.steering = Some(dead(x));
                    out.stick = true;
                }
                out.throttle.get_or_insert(crate::settings::pedal_curve(rt, self.pedal_throttle));
                out.brake.get_or_insert(crate::settings::pedal_curve(lt, self.pedal_brake));
                // the right stick looks round, as the truck games have it (#454)
                out.apply_default_gamepad_look(self.right_stick_look, pad.value(Axis::RightStickX), pad.value(Axis::RightStickY));
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
/// lock and feedback from the bus's lateral acceleration. Front-wheel jolts and the
/// scripts' shaking come on top.
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
    (steering * k_springs.clamp(0.0, 2.0) + (shake + bump) * k_effects.clamp(0.0, 2.0)).clamp(-1.0, 1.0)
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
/// triggers, axis codes, into button events too: they are pedals, not numbered buttons).
pub(crate) fn button_number(pad: &gilrs::Gamepad, code: gilrs::ev::Code) -> Option<usize> {
    #[cfg(target_os = "linux")]
    if let Some(n) = declared_button_index(pad.name(), code.into_u32()) {
        return Some(n);
    }
    if cfg!(windows) {
        return code_button(code.into_u32());
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
