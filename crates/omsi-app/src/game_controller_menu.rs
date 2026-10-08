//! In-game controller configuration. Edits are saved before installing mappings on the
//! existing controller; opening this UI never creates another hardware connection.
use crate::controllers::{DeviceCfg, Func};
use crate::gamepad_profile::{self, PadKind};
use crate::game_lists::{ListKind, Move, HEADING};
use crate::App;

pub(crate) fn is_controller_list(kind: Option<&ListKind>) -> bool {
    matches!(kind, Some(ListKind::ControllerDevices(_) | ListKind::Controller(..) | ListKind::ControllerAxis(..) | ListKind::ControllerButtons(_) | ListKind::ControllerButtonSettings(..) | ListKind::ControllerButton(..) | ListKind::ControllerCapture(_)))
}

fn configurations(app: &App) -> Vec<DeviceCfg> {
    app.input.controllers.as_ref().map(|c| c.configuration()).unwrap_or_else(|| crate::controllers::read_cfg(&app.args.root))
}

fn index(devices: &[DeviceCfg], name: &str) -> Option<usize> {
    devices.iter().position(|d| d.name == name)
        .or_else(|| devices.iter().position(|d| crate::controllers::names_match(&d.name, name)))
}

fn event_names(app: &App, device: &DeviceCfg) -> Vec<(String, String)> {
    let names = crate::describe::names(&app.args.root, &app.settings.language);
    let mut events = names.events();
    for action in configurations(app).iter().flat_map(|d| d.buttons.iter().map(|b| b.0.clone()))
        .chain(device.buttons.iter().map(|b| b.0.clone()))
        .chain(crate::game_lists::keyboard_actions(app))
        .chain(app.player.as_ref().into_iter().flat_map(|p| p.vehicle.ty.program.trigger_names()))
        .chain(["kw_s_R_fest", "kw_s_1_fest", "kw_s_2_fest", "kw_s_3_fest", "kw_s_4_fest", "kw_s_5_fest", "kw_s_6_fest", "kw_s_7_fest", "kw_s_8_fest", "kw_s_9_fest", "kw_s_10_fest",
                "gear_up", "gear_down", "view_look_left", "view_look_right", "view_look_up", "view_look_down", "view_toggle_viewpoint", "view_driver", "view_outside", "view_passenger", "voice_radio"].into_iter().map(str::to_string)) {
        if !action.is_empty() && !events.iter().any(|(a, _)| a.eq_ignore_ascii_case(&action)) {
            let label = if action.eq_ignore_ascii_case("voice_radio") {
                "Multiplayer: bus radio (hold)".into()
            } else {
                names.control(&action)
            };
            events.push((action.clone(), label));
        }
    }
    events.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()).then_with(|| a.0.cmp(&b.0)));
    events
}

fn row(name: &str, value: &str, desc: &str, action: String) -> (String, String) {
    (crate::game_lists::row(name, 'a', value, desc, None), action)
}

const DEVICE_TABS: [&str; 4] = ["Device", "Axes and pedals", "Buttons", "Force feedback"];
const COMMON_TABS: [&str; 4] = ["Devices", "Driving", "Gamepad", "Force feedback"];
const AXES: [&str; 8] = ["X axis", "Y axis", "Z axis", "X rotation", "Y rotation", "Z rotation", "Slider 1", "Slider 2"];
const PAD_TYPES: [&str; 4] = ["auto", "xbox", "ps4", "ps5"];
type Rows = Vec<(String, String)>;

/// The family the gamepad's buttons are named for: the setting, else the first pad found,
/// else Xbox (what most players hold).
fn pad_kind(app: &App, connected: &[crate::controllers::Connected]) -> PadKind {
    PadKind::from_setting(&app.settings.pad_type)
        .or_else(|| connected.iter().find_map(|c| PadKind::detect(&c.name, c.hardware_id)))
        .unwrap_or(PadKind::Xbox)
}

/// The Gamepad page: which pad it is, how its stick steers and what its buttons do.
fn gamepad_rows(app: &App, connected: &[crate::controllers::Connected]) -> Rows {
    let mut out: Rows = Vec::new();
    out.push(("Controller".into(), HEADING.into()));
    let found: Vec<_> = connected.iter().filter_map(|c| PadKind::detect(&c.name, c.hardware_id).map(|k| (c, k))).collect();
    for (c, k) in &found {
        out.push((crate::game_lists::row(&c.name, 'i', k.title(), "Found by its name and USB ids", None), "noop".into()));
    }
    if found.is_empty() {
        out.push((crate::game_lists::row("No Xbox or PlayStation pad found", 'i', "", "Connect a pad, or choose its type below to see the button names", None), "noop".into()));
    }
    let shown = match PadKind::from_setting(&app.settings.pad_type) {
        Some(k) => k.title().to_string(),
        None => format!("Automatic ({})", pad_kind(app, connected).title()),
    };
    out.push((crate::game_lists::row("Controller type", 'o', &shown, "Names the buttons below for the pad you hold: Automatic, Xbox, PlayStation 4 or PlayStation 5", None), "pad_type".into()));
    out.push(("Steering and view".into(), HEADING.into()));
    out.extend([
        crate::game_lists::slider_row(app, "pad_steer_speed", "Steering speed", "Seconds the stick takes to turn the wheel from the middle to the full lock - more is smoother; the wheel is slower still at speed", &|v| format!("{v:.1} s")),
        crate::game_lists::slider_row(app, "pad_steer_smooth", "Stick smoothing", "Evens out the small shakes of the stick (off: the stick as it reads)", &|v| if v <= 0.0 { "Off".to_string() } else { format!("{v:.0} ms") }),
        crate::game_lists::slider_row(app, "pad_deadzone", "Stick dead zone", "Ignore movement round the stick's centre (raise it if the bus steers by itself)", &|v| format!("{:.0} %", v * 100.0)),
        crate::game_lists::switch_row(app, "pad_steer_linear", "Stick steers like a wheel", "The wheel where the stick points, as far at any speed: for a wheel the system calls an Xbox controller"),
        crate::game_lists::switch_row(app, "right_stick_look", "Right stick turns the view", "Switch off to keep the camera still while you steer"),
    ].into_iter().flatten());
    out.push(("Buttons".into(), HEADING.into()));
    out.extend(crate::game_lists::switch_row(app, "pad_buttons", "Default buttons", "A gamepad with no buttons set up drives the cabin with these; set up its buttons under Devices to use your own"));
    out.push((crate::game_lists::row("Left stick / triggers", 'i', "Steer / brake, throttle", "Left stick steers, left trigger brakes, right trigger is the throttle", None), "noop".into()));
    let kind = pad_kind(app, connected);
    for b in gamepad_profile::PRESET_BUTTONS {
        let Some(action) = gamepad_profile::default_action(b) else { continue };
        out.push((crate::game_lists::row(kind.label(b), 'i', gamepad_profile::action_text(action), "", None), "noop".into()));
    }
    out
}

/// The sidebar uses the same pages and indices as keyboard/mouse tab navigation.
pub(crate) fn pages(app: &App, kind: &ListKind) -> Option<(Vec<(&'static str, Rows)>, usize)> {
    match kind {
        ListKind::ControllerDevices(tab) => Some((COMMON_TABS.iter().enumerate()
            .map(|(i, title)| (*title, items(app, &ListKind::ControllerDevices(i)))).collect(), *tab)),
        ListKind::Controller(name, tab) => Some((DEVICE_TABS.iter().enumerate()
            .map(|(i, title)| (*title, items(app, &ListKind::Controller(name.clone(), i)))).collect(), *tab)),
        ListKind::ControllerAxis(..) => Some((vec![("Axis", items(app, kind))], 0)),
        ListKind::ControllerButtonSettings(..) => Some((vec![("Button", items(app, kind))], 0)),
        _ => None,
    }
}

/// Every editor returns to its own page, including physical capture and action selection.
pub(crate) fn parent(kind: &ListKind) -> Option<ListKind> {
    Some(match kind {
        ListKind::ControllerDevices(_) => ListKind::Controls,
        ListKind::Controller(..) => ListKind::ControllerDevices(0),
        ListKind::ControllerAxis(name, _) => ListKind::Controller(name.clone(), 1),
        ListKind::ControllerButtons(name) | ListKind::ControllerButtonSettings(name, _)
            | ListKind::ControllerCapture(name) => ListKind::Controller(name.clone(), 2),
        ListKind::ControllerButton(name, b) => ListKind::ControllerButtonSettings(name.clone(), *b),
        _ => return None,
    })
}

fn axis_rows(d: &DeviceCfg, live: Option<&crate::controllers::Connected>) -> Rows {
    AXES.iter().enumerate().map(|(a, label)| {
        let function = Func::LABELS[(Func::code(d.axes[a].map(|x| x.0)) + 1) as usize];
        let value = live.and_then(|c| c.axes.iter().find(|(k, _)| *k == a))
            .map(|(_, v)| format!(" · {v:+.2}")).unwrap_or_default();
        crate::game_lists::opens(&format!("{label}: {function}{value}"),
            "Choose the function, direction and response curve of this axis", &format!("edit_axis {a}"))
    }).collect()
}

/// Only assigned buttons and latching switches appear here; empty hardware slots stay
/// available through physical capture or the separate button-number list.
fn button_rows(d: &DeviceCfg, names: &crate::describe::ControlNames) -> Rows {
    let mut out = vec![
        crate::game_lists::opens("Press a button to assign it…", "Capture a button on this device", "capture_button"),
        crate::game_lists::opens("Choose a button by number…", "Includes unassigned buttons and hat directions", "button_numbers"),
        ("Assigned buttons".into(), HEADING.into()),
    ];
    for b in 0..d.buttons.len().max(d.latching.iter().max().map(|b| b + 1).unwrap_or(0)).min(crate::controllers::HAT_BUTTONS + 16) {
        let action = d.buttons.get(b).map(|x| x.0.as_str()).unwrap_or("");
        if action.is_empty() && !d.latching.contains(&b) { continue; }
        let label = if action.is_empty() { "<none>".into() } else { names.control(action) };
        let desc = if d.latching.contains(&b) { "Latching switch · choose its action or behaviour" } else { "Choose its action or switch behaviour" };
        out.push(crate::game_lists::opens(&format!("{}: {label}", button_label(b)), desc, &format!("button {b}")));
    }
    if out.len() == 3 {
        out.push((crate::game_lists::row("No buttons assigned", 'i', "", "Press a physical button or choose its number above", None), "noop".into()));
    }
    out
}

/// Readable names stay short; identically named mod actions retain their identifier.
fn action_rows(events: Vec<(String, String)>) -> Rows {
    let mut counts = std::collections::HashMap::new();
    for (_, label) in &events { *counts.entry(label.clone()).or_insert(0usize) += 1; }
    events.into_iter().map(|(action, label)| {
        let label = if counts[&label] > 1 { format!("{label} · {action}") } else { label };
        (label, format!("bind {action}"))
    }).collect()
}

pub(crate) fn items(app: &App, kind: &ListKind) -> Rows {
    let devices = configurations(app);
    let connected = app.input.controllers.as_ref().map(|c| c.connected()).unwrap_or_default();
    let mut out = Vec::new();
    match kind {
        ListKind::ControllerDevices(tab) => match (*tab).min(COMMON_TABS.len() - 1) {
            0 => {
                out.push(crate::game_lists::button("Reload saved controllers", "Reload", "Apply saved mappings to the connected devices", "reload_controllers"));
                out.push(("Connected devices".into(), HEADING.into()));
                for c in &connected {
                    let name = index(&devices, &c.name).map(|i| &devices[i].name).unwrap_or(&c.name);
                    out.push(crate::game_lists::opens(name, "Configure this wheel, pedals or gamepad", &format!("controller {name}")));
                }
                if connected.is_empty() {
                    out.push((crate::game_lists::row("No controller connected", 'i', "", "Saved devices can still be edited below", None), "noop".into()));
                }
                let offline: Vec<_> = devices.iter().filter(|d| !connected.iter().any(|c| crate::controllers::names_match(&d.name, &c.name))).collect();
                if !offline.is_empty() {
                    out.push(("Saved devices (disconnected)".into(), HEADING.into()));
                    for d in offline {
                        out.push(crate::game_lists::opens(&d.name, "Edit the saved configuration", &format!("controller {}", d.name)));
                    }
                }
            }
            1 => {
                out.push(("Steering".into(), HEADING.into()));
                out.extend([
                    crate::game_lists::slider_row(app, "ctrl_deadzone", "Dead zone", "Ignore movement around the centre or at pedal rest", &|v| format!("{:.0} %", v * 100.0)),
                    crate::game_lists::switch_row(app, "arrows_switch_cams", "Arrows switch the cameras", "With a wheel, Left/Right change the interior camera as without one, instead of turning the head"),
                    crate::game_lists::switch_row(app, "pad_steer_linear", "Stick steers like a wheel", "The wheel where the stick points, as far at any speed: for a wheel the system calls an Xbox controller"),
                    crate::game_lists::slider_row(app, "pad_steer_smooth", "Stick steering smoothing", "Evens out a gamepad stick's small shakes (off: the stick as it reads)", &|v| if v <= 0.0 { "Off".to_string() } else { format!("{v:.0} ms") }),
                    crate::game_lists::slider_row(app, "wheel_range", "Wheel rotation", "Your wheel's rotation from lock to lock", &|v| format!("{v:.0}°")),
                    crate::game_lists::slider_row(app, "wheel_lock", "Full lock at", "Rotation for the bus's full lock", &|v| if v < 45.0 { "OMSI".into() } else { format!("{v:.0}°") }),
                ].into_iter().flatten());
                out.push(("Pedals".into(), HEADING.into()));
                out.extend([
                    crate::game_lists::slider_row(app, "pedal_t", "Throttle pedal strength", "Pedal response", &|v| format!("x{v}")),
                    crate::game_lists::slider_row(app, "pedal_b", "Brake pedal strength", "Pedal response", &|v| format!("x{v}")),
                ].into_iter().flatten());
            }
            2 => out = gamepad_rows(app, &connected),
            _ => {
                out.extend([
                    crate::game_lists::switch_row(app, "ff", "Force feedback and vibration", "Enable steering forces and gamepad rumble"),
                    crate::game_lists::switch_row(app, "ff_invert", "Invert force feedback by default", "For wheels without a saved direction"),
                ].into_iter().flatten());
                out.push((crate::game_lists::row("Device strengths", 'i', "", "Select a device and open its Force feedback tab to adjust force, vibration and direction", None), "noop".into()));
            }
        },
        ListKind::Controller(name, tab) => {
            let d = index(&devices, name).map(|i| devices[i].clone()).unwrap_or_else(|| DeviceCfg { name: name.clone(), second: "0".into(), ..Default::default() });
            let live = crate::controllers::find_connected(&connected, name);
            match (*tab).min(DEVICE_TABS.len() - 1) {
                0 => {
                    out.push((crate::game_lists::row("Connection", 'i', if live.is_some() { "Connected" } else { "Disconnected" }, "Successful changes are saved and applied immediately", None), "noop".into()));
                    let disabled = app.settings.ctrl_off.split('|').any(|n| crate::controllers::names_match(n, name));
                    out.push((crate::game_lists::row("Use this device", 's', if disabled { "off" } else { "on" }, "Enable axes and buttons", None), "device_on".into()));
                    out.push(crate::game_lists::opens("Axes and pedals", "Map steering, throttle, brake and clutch", "device_tab 1"));
                    out.push(crate::game_lists::opens("Buttons", "Assign controls and configure latching switches", "device_tab 2"));
                    out.push(crate::game_lists::opens("Force feedback", "Adjust this device's force, vibration and direction", "device_tab 3"));
                }
                1 => out = axis_rows(&d, live),
                2 => out = button_rows(&d, &crate::describe::names(&app.args.root, &app.settings.language)),
                _ => {
                    let ff = d.ff_scale.unwrap_or((1.0, 1.0));
                    out.push(row("Steering force", &format!("{:.0} %", ff.0 * 100.0), "Left/right adjusts in steps of 5 % (0–200 %)", "force".into()));
                    out.push(row("Vibration", &format!("{:.0} %", ff.1 * 100.0), "Left/right adjusts in steps of 5 % (0–200 %)", "vibration".into()));
                    let invert = d.ff_invert.unwrap_or(app.settings.ff_invert);
                    out.push((crate::game_lists::row("Invert force feedback", 's', if invert { "on" } else { "off" }, "Motor direction for this device", None), "force_invert".into()));
                }
            }
        }
        ListKind::ControllerAxis(name, a) if *a < AXES.len() => {
            let d = index(&devices, name).map(|i| devices[i].clone()).unwrap_or_default();
            let live = crate::controllers::find_connected(&connected, name)
                .and_then(|c| c.axes.iter().find(|(k, _)| k == a)).map(|(_, v)| format!("{v:+.2}")).unwrap_or_else(|| "Disconnected".into());
            out.push((crate::game_lists::row("Live reading", 'i', &live, "Move the wheel, pedal or stick to identify this axis", None), "noop".into()));
            let function = (Func::code(d.axes[*a].map(|x| x.0)) + 1) as usize;
            out.push(row("Function", Func::LABELS[function], "Choose what this axis controls", format!("axis {a}")));
            if let Some((_, inv)) = d.axes[*a] {
                out.push((crate::game_lists::row("Reversed", 's', if inv { "on" } else { "off" }, "Reverse this axis", None), format!("reverse {a}")));
                let curve = crate::controllers::AXIS_SHAPES.iter().find(|s| s.1 == d.axis_flags[*a] & (4 | 8 | 0x10)).map(|s| s.0).unwrap_or("Linear");
                out.push(row("Response curve", curve, "Choose the characteristic of this axis", format!("curve {a}")));
            }
        }
        ListKind::ControllerButtons(name) => {
            let d = index(&devices, name).map(|i| devices[i].clone()).unwrap_or_default();
            let live = crate::controllers::find_connected(&connected, name);
            let count = d.buttons.len().max(live.map(|c| c.buttons).unwrap_or(0))
                .max(d.latching.iter().max().map(|b| b + 1).unwrap_or(0)).max(32).min(crate::controllers::HAT_BUTTONS + 16);
            let names = crate::describe::names(&app.args.root, &app.settings.language);
            for b in (0..count.min(crate::controllers::HAT_BUTTONS))
                .chain(crate::controllers::HAT_BUTTONS..crate::controllers::HAT_BUTTONS + 16) {
                let action = d.buttons.get(b).map(|x| x.0.as_str()).unwrap_or("");
                let value = if action.is_empty() { "Unassigned".into() } else { names.control(action) };
                out.push((format!("{}: {value}", button_label(b)), format!("button {b}")));
            }
        }
        ListKind::ControllerButtonSettings(name, b) => {
            let d = index(&devices, name).map(|i| devices[i].clone()).unwrap_or_default();
            let action = d.buttons.get(*b).map(|x| x.0.as_str()).unwrap_or("");
            let names = crate::describe::names(&app.args.root, &app.settings.language);
            let value = if action.is_empty() { "Unassigned".into() } else { names.control(action) };
            out.push(crate::game_lists::opens(&format!("Action: {value}"), "Choose an OMSI event or game action", "choose_action"));
            out.push((crate::game_lists::row("Latching switch", 's', if d.latching.contains(b) { "on" } else { "off" }, "Switch back when a physical switch is released", None), format!("latching {b}")));
            out.push(crate::game_lists::button("Clear assignment", "Clear", "Leave this button without an action", "bind "));
        }
        ListKind::ControllerCapture(name) => {
            out.push((format!("Press a button on {name} (Esc cancels)"), "noop".into()));
            out.push(("Cancel".into(), "back".into()));
        }
        ListKind::ControllerButton(name, b) => {
            let d = index(&devices, name).map(|i| devices[i].clone()).unwrap_or_default();
            out.extend(action_rows(event_names(app, &d)));
            out.push((format!("Clear {}", button_label(*b)), "bind ".into()));
            out.push(("Back".into(), "back".into()));
        }
        _ => {}
    }
    out
}

pub(crate) fn button_label(b: usize) -> String {
    if b >= crate::controllers::HAT_BUTTONS {
        let h = b - crate::controllers::HAT_BUTTONS;
        format!("Hat {} {}", h / 4 + 1, ["up", "right", "down", "left"][h % 4])
    } else {
        format!("Button {}", b + 1)
    }
}

fn step(now: usize, count: usize, mv: Move) -> usize {
    match mv {
        Move::Next => (now + 1) % count,
        Move::Inc => (now + 1).min(count - 1),
        Move::Dec => now.saturating_sub(1),
        Move::To(f) => (f.clamp(0.0, 1.0) * (count - 1) as f32).round() as usize,
    }
}

fn switched(now: bool, mv: Move) -> bool {
    match mv { Move::Next => !now, Move::Inc => true, Move::Dec => false, Move::To(f) => f >= 0.5 }
}

fn save(app: &mut App, devices: Vec<DeviceCfg>) {
    match crate::controllers::save_cfg(&devices) {
        Ok(()) => {
            if let Some(c) = app.input.controllers.as_mut() { c.install_cfg(devices); }
            app.input.last_ctl_steer = None;
            app.service_msg = Some(("Controller configuration saved and applied".into(), 3.0));
        }
        Err(e) => app.service_msg = Some((format!("Controller configuration was not saved: {e}"), 6.0)),
    }
}

pub(crate) fn run(app: &mut App, kind: &ListKind, action: &str, mv: Move) -> Option<ListKind> {
    let (verb, arg) = action.split_once(' ').unwrap_or((action, ""));
    if action == "back" { return parent(kind); }
    if let ListKind::ControllerDevices(_) = kind {
        if verb == "pad_type" {
            let to = step(PAD_TYPES.iter().position(|t| *t == app.settings.pad_type).unwrap_or(0), PAD_TYPES.len(), mv);
            app.settings.pad_type = PAD_TYPES[to].into();
            crate::game_lists::remember_setting("pad_type", PAD_TYPES[to]);
            return Some(kind.clone());
        }
        if crate::game_lists::option_do(app, verb, arg, mv) { return Some(kind.clone()); }
        if !matches!(mv, Move::Next) { return Some(kind.clone()); }
        return Some(match verb {
            "controller" => ListKind::Controller(arg.to_string(), 0),
            "reload_controllers" => {
                match crate::controllers::read_cfg_checked(&app.args.root) {
                    Ok(devices) => {
                        if let Some(c) = app.input.controllers.as_mut() { c.install_cfg(devices); }
                        app.input.last_ctl_steer = None;
                        app.service_msg = Some(("Saved controller mappings reloaded".into(), 3.0));
                    }
                    Err(e) => app.service_msg = Some((format!("Controller mappings were not reloaded: {e}"), 6.0)),
                }
                kind.clone()
            }
            _ => kind.clone(),
        });
    }
    let (name, button) = match kind {
        ListKind::Controller(name, _) | ListKind::ControllerAxis(name, _)
            | ListKind::ControllerButtons(name) => (name, None),
        ListKind::ControllerButtonSettings(name, b) | ListKind::ControllerButton(name, b) => (name, Some(*b)),
        _ => return Some(kind.clone()),
    };
    if matches!(mv, Move::Next) {
        match verb {
            "device_tab" => if let Ok(tab) = arg.parse::<usize>() {
                return Some(ListKind::Controller(name.clone(), tab.min(DEVICE_TABS.len() - 1)));
            },
            "edit_axis" => if let Ok(a) = arg.parse::<usize>() {
                if a < AXES.len() { return Some(ListKind::ControllerAxis(name.clone(), a)); }
            },
            "button_numbers" => return Some(ListKind::ControllerButtons(name.clone())),
            "choose_action" => if let Some(b) = button {
                return Some(ListKind::ControllerButton(name.clone(), b));
            },
            "capture_button" => return Some(ListKind::ControllerCapture(name.clone())),
            "button" => if let Ok(b) = arg.parse::<usize>() {
                if b < crate::controllers::HAT_BUTTONS + 16 { return Some(ListKind::ControllerButtonSettings(name.clone(), b)); }
            },
            _ => {}
        }
    }
    if verb == "device_on" {
        let mut off: Vec<String> = app.settings.ctrl_off.split('|').filter(|s| !s.is_empty()).map(str::to_string).collect();
        let on = !off.iter().any(|n| crate::controllers::names_match(n, name));
        off.retain(|n| !crate::controllers::names_match(n, name));
        if !switched(on, mv) { off.push(name.clone()); }
        app.settings.ctrl_off = off.join("|");
        crate::game_lists::remember_setting("ctrl_off", &app.settings.ctrl_off);
        return Some(kind.clone());
    }
    let mut devices = configurations(app);
    let i = index(&devices, name).unwrap_or_else(|| {
        devices.push(DeviceCfg { name: name.clone(), second: "0".into(), ..Default::default() });
        devices.len() - 1
    });
    let d = &mut devices[i];
    let axis = arg.parse::<usize>().ok().filter(|a| *a < 8);
    match verb {
        "axis" => if let Some(a) = axis {
            let current = (Func::code(d.axes[a].map(|x| x.0)) + 1) as usize;
            let to = step(current, Func::LABELS.len(), mv);
            let inv = d.axes[a].map(|x| x.1).unwrap_or(false);
            d.axes[a] = Func::from_code(to as i32 - 1).map(|f| (f, inv));
        } else { return Some(kind.clone()); },
        "reverse" => if let Some(a) = axis {
            if let Some((_, inv)) = &mut d.axes[a] { *inv = switched(*inv, mv); }
        } else { return Some(kind.clone()); },
        "curve" => if let Some(a) = axis {
            let curve = d.axis_flags[a] & (4 | 8 | 0x10);
            let current = crate::controllers::AXIS_SHAPES.iter().position(|s| s.1 == curve).unwrap_or(0);
            let to = step(current, crate::controllers::AXIS_SHAPES.len(), mv);
            d.axis_flags[a] = (d.axis_flags[a] & !(4 | 8 | 0x10)) | crate::controllers::AXIS_SHAPES[to].1;
        } else { return Some(kind.clone()); },
        "force" | "vibration" => {
            let mut ff = d.ff_scale.unwrap_or((1.0, 1.0));
            let v = if verb == "force" { &mut ff.0 } else { &mut ff.1 };
            *v = step((*v * 20.0).round().clamp(0.0, 40.0) as usize, 41, mv) as f32 / 20.0;
            d.ff_scale = Some(ff);
        }
        "force_invert" => d.ff_invert = Some(switched(d.ff_invert.unwrap_or(app.settings.ff_invert), mv)),
        "latching" => if let Some(b) = arg.parse::<usize>().ok().filter(|b| *b < crate::controllers::HAT_BUTTONS + 16) {
            let on = switched(d.latching.contains(&b), mv);
            d.latching.retain(|x| *x != b);
            if on { d.latching.push(b); d.latching.sort_unstable(); }
        } else { return Some(kind.clone()); },
        "bind" if matches!(mv, Move::Next) => {
            if let Some(b) = button {
                if b >= crate::controllers::HAT_BUTTONS + 16 { return Some(kind.clone()); }
                d.buttons.resize(d.buttons.len().max(b + 1), (String::new(), "0".into()));
                d.buttons[b].0 = arg.to_string();
                save(app, devices);
                return Some(ListKind::ControllerButtonSettings(name.clone(), b));
            }
            return Some(kind.clone());
        }
        _ => return Some(kind.clone()),
    }
    save(app, devices);
    Some(kind.clone())
}

/// Called after the existing controller's single poll for this frame.
pub(crate) fn frame(app: &mut App) {
    if matches!(app.menus.list_kind, Some(ListKind::ControllerDevices(_) | ListKind::Controller(..) | ListKind::ControllerAxis(..))) {
        thread_local! {
            static LAST_REFRESH: std::cell::RefCell<std::time::Instant> = std::cell::RefCell::new(std::time::Instant::now());
        }
        let refresh = LAST_REFRESH.with(|last| {
            let mut last = last.borrow_mut();
            if last.elapsed().as_millis() < 250 { return false; }
            *last = std::time::Instant::now();
            true
        });
        if refresh { app.refresh_list(); }
        return;
    }
    let Some(ListKind::ControllerCapture(name)) = app.menus.list_kind.clone() else { return };
    let pressed = app.input.controllers.as_ref().and_then(|c| c.raw_buttons.iter().find(|(n, b, down)|
        *down && *b < crate::controllers::HAT_BUTTONS + 16 && crate::controllers::names_match(n, &name)).map(|(_, b, _)| *b));
    if let Some(button) = pressed {
        app.open_list(ListKind::ControllerButtonSettings(name, button));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controller_menu_steps_clamp_and_wrap() {
        assert_eq!(step(0, 8, Move::Dec), 0);
        assert_eq!(step(7, 8, Move::Inc), 7);
        assert_eq!(step(7, 8, Move::Next), 0);
        assert_eq!(step(0, 41, Move::To(1.0)), 40);
        assert_eq!(button_label(0), "Button 1");
        assert_eq!(button_label(128), "Hat 1 up");
        assert!(is_controller_list(Some(&ListKind::ControllerCapture("Wheel".into()))));
        assert!(!is_controller_list(Some(&ListKind::Events)));
    }

    #[test]
    fn axes_overview_keeps_eight_distinct_editors_and_live_readings() {
        let mut d = DeviceCfg::default();
        d.axes[0] = Some((Func::Steering, true));
        d.axes[5] = Some((Func::Brake, false));
        let live = crate::controllers::Connected {
            name: "Wheel".into(), hardware_id: None, axes: vec![(0, 0.25)],
            gamepad: false, ff: false, ff_capable: false, buttons: 0,
        };
        let rows = axis_rows(&d, Some(&live));
        assert_eq!(rows.len(), 8);
        assert_eq!(rows.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(),
            ["edit_axis 0", "edit_axis 1", "edit_axis 2", "edit_axis 3", "edit_axis 4", "edit_axis 5", "edit_axis 6", "edit_axis 7"]);
        assert!(rows[0].0.starts_with("X axis: Steering · +0.25"));
        assert!(rows[5].0.starts_with("Z rotation: Brake"));
        assert!(axis_rows(&d, None)[0].0.starts_with("X axis: Steering"));
    }

    #[test]
    fn buttons_overview_hides_empty_slots_without_renumbering_assignments() {
        let mut d = DeviceCfg::default();
        d.buttons.resize(144, (String::new(), "0".into()));
        d.buttons[7].0 = "horn".into();
        d.buttons[131].0 = "view_look_left".into();
        d.latching = vec![129];
        let names = crate::describe::ControlNames::load(std::path::Path::new("."), "ENG");
        let rows = button_rows(&d, &names);
        assert_eq!(rows.iter().filter_map(|r| r.1.strip_prefix("button ")).collect::<Vec<_>>(), ["7", "129", "131"]);
        assert_eq!(rows.len(), 6);
        assert!(rows[3].0.starts_with("Button 8:"));
        assert!(rows[4].0.starts_with("Hat 1 right:"));
        assert!(rows[5].0.starts_with("Hat 1 left:"));
        assert_eq!(button_rows(&DeviceCfg::default(), &names).len(), 4);
    }

    #[test]
    fn editor_back_navigation_preserves_the_device_and_parent_tab() {
        let wheel = "Wheel".to_string();
        assert_eq!(parent(&ListKind::ControllerAxis(wheel.clone(), 5)), Some(ListKind::Controller(wheel.clone(), 1)));
        for kind in [ListKind::ControllerCapture(wheel.clone()), ListKind::ControllerButtons(wheel.clone()), ListKind::ControllerButtonSettings(wheel.clone(), 131)] {
            assert_eq!(parent(&kind), Some(ListKind::Controller(wheel.clone(), 2)));
        }
        assert_eq!(parent(&ListKind::ControllerButton(wheel.clone(), 131)), Some(ListKind::ControllerButtonSettings(wheel.clone(), 131)));
        assert_eq!(parent(&ListKind::Controller(wheel, 3)), Some(ListKind::ControllerDevices(0)));
        assert_eq!(parent(&ListKind::ControllerDevices(2)), Some(ListKind::Controls));
        assert_eq!(parent(&ListKind::Events), None);
    }

    #[test]
    fn identically_named_actions_remain_distinguishable_in_the_picker() {
        let rows = action_rows(vec![("horn".into(), "Horn".into()), ("door_front".into(), "Door".into()), ("door_rear".into(), "Door".into())]);
        assert_eq!(rows, vec![("Horn".into(), "bind horn".into()),
            ("Door · door_front".into(), "bind door_front".into()), ("Door · door_rear".into(), "bind door_rear".into())]);
    }
}
