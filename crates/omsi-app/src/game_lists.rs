//! The game menu's own windows besides the administration (see `admin`), as OMSI has them
//! in its menus: the options that can change while driving, the line and tour to drive,
//! the driver whose personnel file the run goes into, and the bus's fleet number. Each is a
//! list of (label, action) lines in the menu's chooser; choosing a line does it and shows
//! the list again (or the next one: a line's tours).

use crate::App;

/// Which list the chooser shows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ListKind {
    Admin,
    Options,
    Lines,
    Tours(String),
    Drivers,
    Numbers,
    /// The termini of the bus's depot file, for its destination display.
    Destinations,
    /// The route numbers (lines) for the destination display: the depot file's and the
    /// map timetable's.
    RouteNumbers,
    /// The depot files (.hof) of the bus driven.
    Hofs,
    /// The clock set by hand: steps, and on a duty the time the timetable wants.
    Clock,
    /// Placing a vehicle: its livery, then its depot file (bus file; bus file and livery).
    PlaceLivery(String),
    PlaceHof(String, String),
}

/// A vehicle file of the menu's list (`Vehicles/...`) as its definition.
fn bus_def(app: &App, bus: &str) -> Option<omsi_vehicle::Vehicle> {
    let path = crate::spawn::player_bus_path(&app.args.root, bus).ok()?;
    omsi_vehicle::Vehicle::load(&path).ok()
}

/// The route numbers to choose from: the lines of the depot file's routes (their own
/// line, or the route code without its last two digits) and of the map's timetable.
fn route_numbers(app: &App) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(hof) = app.player.as_ref().and_then(|p| p.vehicle.host.hof.clone()) {
        for t in &hof.info_trips {
            let code = t.code.trim();
            let l = if !t.line.trim().is_empty() { t.line.trim().to_string() } else if code.len() > 2 && code.chars().all(|c| c.is_ascii_digit()) { code[..code.len() - 2].trim_start_matches('0').to_string() } else { String::new() };
            let l = l.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
            if !l.is_empty() && !out.contains(&l) {
                out.push(l);
            }
        }
    }
    if let Some(sch) = app.schedule.as_ref() {
        for l in &sch.data.lines {
            let n = l.name.trim().to_string();
            if !n.is_empty() && !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out.sort_by(|a, b| natural(a, b));
    out
}

/// The paint schemes of a vehicle file, by name (without loading its meshes).
fn liveries(def: &omsi_vehicle::Vehicle) -> Vec<String> {
    let Some(m) = def.model.as_ref() else { return Vec::new() };
    let mp = omsi_cfg::resolve_path(def.dir(), m);
    let Ok(model) = omsi_model::Model::load(&mp) else { return Vec::new() };
    let mut names: Vec<String> = model.ctc.iter().flat_map(|c| omsi_sim::vehicle::load_paint_schemes(&omsi_cfg::resolve_path(def.dir(), &c.path))).map(|s| s.name).collect();
    names.dedup();
    names
}

/// A depot file's name for the lists: its `[name]`, and the file.
fn hof_label(p: &std::path::Path) -> String {
    let file = p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    match omsi_vehicle::Hof::load(p).ok().map(|h| h.name).filter(|n| !n.trim().is_empty()) {
        Some(n) if !file.to_ascii_lowercase().starts_with(&n.trim().to_ascii_lowercase()) => format!("{}  ({file})", n.trim()),
        _ => file,
    }
}

/// The time speeds, traffic amounts and passenger shares the options step through.
const SPEEDS: [f64; 5] = [1.0, 2.0, 4.0, 8.0, 15.0];
pub(crate) const TRAFFIC: [usize; 7] = [0, 10, 20, 30, 50, 80, 120];
const PAX: [f32; 6] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0];
const VOLUME: [f32; 6] = [0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
/// The pedal strengths the options step through (see `settings::pedal_curve`).
const PEDAL: [f32; 7] = [0.5, 0.7, 0.85, 1.0, 1.25, 1.5, 2.0];

fn on_off(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

/// The next of `steps` after `now` (round to the first).
pub(crate) fn next_step<T: PartialOrd + Copy>(steps: &[T], now: T) -> T {
    steps.iter().copied().find(|s| *s > now).unwrap_or(steps[0])
}

pub(crate) fn items(app: &App, kind: &ListKind) -> Vec<(String, String)> {
    let tr = |t: &str| omsi_ui::tr(t).into_owned();
    let mut out: Vec<(String, String)> = Vec::new();
    match kind {
        ListKind::Admin => return crate::admin::items(app),
        ListKind::Options => {
            let s = &app.settings;
            if app.lan.is_none() {
                out.push((format!("{}: x{}", tr("Time speed"), s.time_speed), "speed".into()));
            }
            if let Some(t) = app.traffic.as_ref() {
                out.push((format!("{}: {}", tr("Traffic"), t.target), "traffic".into()));
            }
            out.push((format!("{}: {:.0} %", tr("Passengers"), s.pax_density * 100.0), "pax".into()));
            out.push((format!("{}: {:.0} %", tr("Volume"), s.volume * 100.0), "volume".into()));
            out.push((format!("{}: {}", tr("Navigator"), tr(on_off(app.navigator.as_ref().is_some_and(|n| n.enabled)))), "navigator".into()));
            out.push((format!("{}: {}", tr("Sun shadows"), tr(on_off(s.shadows))), "shadows".into()));
            out.push((format!("{}: {}", tr("Head movement"), tr(on_off(s.head_movement))), "head".into()));
            out.push((format!("{}: {}", tr("Camera glides between viewpoints"), tr(on_off(s.driverview_smooth))), "cam_smooth".into()));
            out.push((format!("{}: {}", tr("Collisions with objects"), tr(on_off(s.collision_objects))), "coll_objects".into()));
            out.push((format!("{}: {}", tr("Collisions with vehicles"), tr(on_off(s.collision_vehicles))), "coll_vehicles".into()));
            out.push((format!("{}: {}", tr("Steering with the mouse"), tr(on_off(app.mouse_drive))), "mouse".into()));
            out.push((format!("{}: {}", tr("Frame rate"), tr(on_off(s.show_fps))), "fps".into()));
            out.push((format!("{}: {}", tr("Camera collisions"), tr(on_off(s.camera_collision))), "camcoll".into()));
            out.push((format!("{}: {}", tr("View turns with steering"), tr(on_off(s.steer_look))), "steer_look".into()));
            out.push((format!("{}: {}", tr("Driver's hands in the cab view"), tr(on_off(s.hands_in_cab))), "hands_in_cab".into()));
            out.push((format!("{}: {}", tr("Force feedback and vibration"), tr(on_off(s.ff_enabled))), "ff".into()));
            out.push((format!("{}: {}", tr("Keyboard brake stays on until the throttle"), tr(on_off(s.brake_hold))), "brake_hold".into()));
            out.push((format!("{}: {}", tr("Automatic clutch"), tr(on_off(s.auto_clutch))), "auto_clutch".into()));
            // (the LED panels' dots glow, and whether their mask keeps its mip chain)
            out.push((format!("{}: {}/15", tr("LED glow"), s.led_glow), "led_glow".into()));
            out.push((format!("{}: {}", tr("LED masks keep their mipmaps"), tr(on_off(s.led_mips))), "led_mips".into()));
            out.push((format!("{} (opentrack UDP {}): {}", tr("Head tracking"), s.head_tracking_port, tr(on_off(s.head_tracking))), "headtrack".into()));
            out.push((format!("{}: x{}", tr("Throttle pedal strength"), s.pedal_throttle), "pedal_t".into()));
            out.push((format!("{}: x{}", tr("Brake pedal strength"), s.pedal_brake), "pedal_b".into()));
            let seat = |v: f32| format!("{:+.0} cm", v * 100.0);
            out.push((format!("{} ({})", tr("Seat forward"), seat(s.seat[1])), "seat 1 0.05".into()));
            out.push((format!("{} ({})", tr("Seat back"), seat(s.seat[1])), "seat 1 -0.05".into()));
            out.push((format!("{} ({})", tr("Seat up"), seat(s.seat[2])), "seat 2 0.05".into()));
            out.push((format!("{} ({})", tr("Seat down"), seat(s.seat[2])), "seat 2 -0.05".into()));
            out.push((format!("{} ({})", tr("Seat right"), seat(s.seat[0])), "seat 0 0.05".into()));
            out.push((format!("{} ({})", tr("Seat left"), seat(s.seat[0])), "seat 0 -0.05".into()));
            out.push((tr("Reset the seat position"), "seat_reset".into()));
        }
        ListKind::Lines => {
            if let Some(sch) = app.schedule.as_ref() {
                let mut lines: Vec<&omsi_timetable::Line> = sch.data.lines.iter().filter(|l| l.user_allowed && !l.tours.is_empty()).collect();
                lines.sort_by(|a, b| natural(&a.name, &b.name));
                for l in lines {
                    out.push((format!("{} {}  ({} {})", tr("Line"), l.name, l.tours.len(), tr("tours")), format!("line {}", l.name)));
                }
            }
            if app.duty.is_some() {
                out.push((tr("Free drive (no duty)"), "free".into()));
            }
            if out.is_empty() {
                out.push((tr("No timetable on this map"), "back".into()));
            }
        }
        ListKind::Tours(line) => {
            if let Some(l) = app.schedule.as_ref().and_then(|s| s.data.lines.iter().find(|l| l.name == *line)) {
                for t in &l.tours {
                    let first = t.trips.first().map(|x| format!("  {:02}:{:02}", (x.departure / 60.0) as i32 % 24, (x.departure % 60.0) as i32)).unwrap_or_default();
                    out.push((format!("{} {}{first}", tr("Tour"), t.number.trim()), format!("tour {}\u{1}{}", line, t.number)));
                }
            }
        }
        ListKind::Drivers => {
            for name in driver_names(app) {
                let mark = if app.career.path.as_ref().and_then(|p| p.file_stem()).is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(&name)) { format!("  {}", tr("(now)")) } else { String::new() };
                out.push((format!("{name}{mark}"), format!("driver {name}")));
            }
        }
        ListKind::Destinations => {
            if let Some(p) = app.player.as_ref().filter(|p| p.vehicle.host.hof.is_some()) {
                let now = p.vehicle.var("IBIS_LinieKurs").filter(|l| *l > 0.0).map(|l| format!("{}", l as i64)).unwrap_or_else(|| "-".into());
                out.push((format!("{}: {now}...", tr("Route number")), "routes".into()));
            }
            if let Some(hof) = app.player.as_ref().and_then(|p| p.vehicle.host.hof.clone()) {
                for t in hof.termini.iter() {
                    let name = t.strings.iter().find(|s| !s.trim().is_empty()).cloned().unwrap_or_else(|| t.code.to_string());
                    out.push((format!("{:>3}  {}", t.code, name.trim()), format!("dest {}", t.code)));
                }
            }
            if out.is_empty() {
                out.push((tr("This bus has no depot file (.hof) with destinations"), "back".into()));
            }
        }
        ListKind::RouteNumbers => {
            for l in route_numbers(app) {
                out.push((format!("{} {l}", tr("Route")), format!("route {l}")));
            }
            if out.is_empty() {
                out.push((tr("No route numbers in the depot file or the timetable"), "back".into()));
            }
        }
        ListKind::Clock => {
            let t = app.clock.time;
            out.push((format!("{}: {:02}:{:02}:{:02}", tr("Now"), (t / 3600.0) as i64 % 24, (t / 60.0) as i64 % 60, t as i64 % 60), "back".into()));
            if let (Some(_), Some(p)) = (app.duty.as_ref(), app.player.as_ref()) {
                let d = p.vehicle.host.tt_delay as f64;
                if d.abs() >= 1.0 {
                    out.push((format!("{} ({}{}:{:02})", tr("On time with the timetable"), if d < 0.0 { "−" } else { "+" }, (d.abs() / 60.0) as i64, d.abs() as i64 % 60), format!("clock {}", -d)));
                }
            }
            for m in [1i64, 5, 15, 60] {
                out.push((format!("+{m} min"), format!("clock {}", m * 60)));
            }
            for m in [1i64, 5, 15, 60] {
                out.push((format!("−{m} min"), format!("clock {}", -m * 60)));
            }
        }
        ListKind::Hofs => {
            if let Some(p) = app.player.as_ref() {
                let now = p.vehicle.host.hof.as_ref().map(|h| h.path.clone());
                for f in omsi_vehicle::hof::depot_files(p.vehicle.ty.def.dir()) {
                    let mark = if now.as_ref() == Some(&f) { format!("  {}", tr("(now)")) } else { String::new() };
                    out.push((format!("{}{mark}", hof_label(&f)), format!("hof {}", f.to_string_lossy())));
                }
            }
            if out.is_empty() {
                out.push((tr("This bus has no depot files (.hof)"), "back".into()));
            }
        }
        ListKind::PlaceLivery(bus) => {
            out.push((tr("Random livery"), "livery ".into()));
            for n in bus_def(app, bus).map(|d| liveries(&d)).unwrap_or_default() {
                out.push((n.clone(), format!("livery {n}")));
            }
        }
        ListKind::PlaceHof(bus, _) => {
            out.push((tr("The map's depot file"), "placehof ".into()));
            if let Some(d) = bus_def(app, bus) {
                for f in omsi_vehicle::hof::depot_files(d.dir()) {
                    let name = f.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                    out.push((hof_label(&f), format!("placehof {name}")));
                }
            }
        }
        ListKind::Numbers => {
            if let Some(p) = app.player.as_ref() {
                for (n, reg) in fleet_numbers(&p.vehicle) {
                    out.push((if reg.is_empty() { n.clone() } else { format!("{n}  ({reg})") }, format!("number {n}\u{1}{reg}")));
                }
            }
            if out.is_empty() {
                out.push((tr("This bus has no list of fleet numbers"), "back".into()));
            }
        }
    }
    out.push((tr("Back"), "back".into()));
    out
}

/// Do a line of the list; returns the list to show next (None: back to the menu).
pub(crate) fn run(app: &mut App, kind: &ListKind, action: &str) -> Option<ListKind> {
    if action == "back" {
        return None;
    }
    let (verb, arg) = action.split_once(' ').unwrap_or((action, ""));
    match kind {
        ListKind::Admin => {
            crate::admin::run(app, action);
            Some(ListKind::Admin)
        }
        ListKind::Options => {
            let s = &mut app.settings;
            let key_value: Option<(&str, String)> = match verb {
                "speed" => {
                    s.time_speed = next_step(&SPEEDS, s.time_speed);
                    Some(("time_speed", s.time_speed.to_string()))
                }
                "traffic" => {
                    if let Some(t) = app.traffic.as_mut() {
                        t.target = next_step(&TRAFFIC, t.target);
                        app.args.traffic = t.target;
                    }
                    None
                }
                "pax" => {
                    s.pax_density = next_step(&PAX, s.pax_density);
                    Some(("pax_density", s.pax_density.to_string()))
                }
                "volume" => {
                    s.volume = next_step(&VOLUME, s.volume);
                    Some(("volume", s.volume.to_string()))
                }
                "navigator" => {
                    let on = app.navigator.as_ref().is_some_and(|n| n.enabled);
                    if let Some(n) = app.navigator.as_mut() {
                        n.enabled = !on;
                    }
                    app.settings.navigator = !on;
                    Some(("navigator", (!on as u8).to_string()))
                }
                "shadows" => {
                    s.shadows = !s.shadows;
                    Some(("shadows", (s.shadows as u8).to_string()))
                }
                "head" => {
                    s.head_movement = !s.head_movement;
                    Some(("head_movement", (s.head_movement as u8).to_string()))
                }
                "cam_smooth" => {
                    s.driverview_smooth = !s.driverview_smooth;
                    Some(("driverview_smooth", (s.driverview_smooth as u8).to_string()))
                }
                // (at once: stuck under a bridge a map made too low, the bus drives on)
                "coll_objects" => {
                    s.collision_objects = !s.collision_objects;
                    let on = s.collision_objects;
                    let cw = app.world.as_ref().map(|w| w.collision.lock().clone());
                    if let Some(p) = app.player.as_mut() {
                        p.vehicle.collision = cw.filter(|_| on);
                    }
                    Some(("collision_objects", (on as u8).to_string()))
                }
                "coll_vehicles" => {
                    s.collision_vehicles = !s.collision_vehicles;
                    Some(("collision_vehicles", (s.collision_vehicles as u8).to_string()))
                }
                "mouse" => {
                    app.mouse_drive = !app.mouse_drive;
                    if !app.mouse_drive {
                        crate::player::keep_wheel(app.player.as_mut());
                    }
                    #[cfg(windows)]
                    if !app.mouse_drive {
                        app.reset_vr_pointer();
                    }
                    app.mouse_steer = (app.player.as_ref().map(|p| p.vehicle.physics.controls.steering).unwrap_or(0.0), 1.0);
                    app.mouse_pedals = app.player.as_ref().map(|p| (p.vehicle.physics.controls.throttle, p.vehicle.physics.controls.brake)).unwrap_or((0.0, 0.0));
                    None
                }
                "fps" => {
                    s.show_fps = !s.show_fps;
                    Some(("show_fps", (s.show_fps as u8).to_string()))
                }
                "headtrack" => {
                    s.head_tracking = !s.head_tracking;
                    Some(("head_tracking", (s.head_tracking as u8).to_string()))
                }
                "camcoll" => {
                    s.camera_collision = !s.camera_collision;
                    Some(("camera_collision", (s.camera_collision as u8).to_string()))
                }
                "steer_look" => {
                    s.steer_look = !s.steer_look;
                    Some(("steer_look", (s.steer_look as u8).to_string()))
                }
                "hands_in_cab" => {
                    s.hands_in_cab = !s.hands_in_cab;
                    Some(("hands_in_cab", (s.hands_in_cab as u8).to_string()))
                }
                "pedal_t" => {
                    s.pedal_throttle = next_step(&PEDAL, s.pedal_throttle);
                    Some(("pedal_throttle", s.pedal_throttle.to_string()))
                }
                "pedal_b" => {
                    s.pedal_brake = next_step(&PEDAL, s.pedal_brake);
                    Some(("pedal_brake", s.pedal_brake.to_string()))
                }
                "seat" => {
                    let mut it = arg.split_whitespace();
                    let k: usize = it.next().and_then(|x| x.parse().ok()).unwrap_or(0).min(2);
                    let d: f32 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0.0);
                    s.seat[k] = ((s.seat[k] + d) * 100.0).round().clamp(-150.0, 150.0) / 100.0;
                    Some((["seat_x", "seat_y", "seat_z"][k], s.seat[k].to_string()))
                }
                "brake_hold" => {
                    s.brake_hold = !s.brake_hold;
                    Some(("brake_hold", (s.brake_hold as u8).to_string()))
                }
                "auto_clutch" => {
                    s.auto_clutch = !s.auto_clutch;
                    if let Some(p) = app.player.as_mut() {
                        p.vehicle.host.auto_clutch = if s.auto_clutch { 1.0 } else { 0.0 };
                    }
                    Some(("auto_clutch", (s.auto_clutch as u8).to_string()))
                }
                "ff" => {
                    s.ff_enabled = !s.ff_enabled;
                    Some(("ff_enabled", (s.ff_enabled as u8).to_string()))
                }
                // the 16 levels run on, off after 15
                "led_glow" => {
                    s.led_glow = (s.led_glow + 1) % 16;
                    Some(("led_glow", s.led_glow.to_string()))
                }
                "led_mips" => {
                    s.led_mips = !s.led_mips;
                    Some(("led_mips", (s.led_mips as u8).to_string()))
                }
                "seat_reset" => {
                    s.seat = [0.0; 3];
                    for k in ["seat_x", "seat_y", "seat_z"] {
                        remember_setting(k, "0");
                    }
                    None
                }
                _ => None,
            };
            // kept for the next game too, as OMSI keeps its options
            if let Some((k, v)) = key_value {
                remember_setting(k, &v);
            }
            Some(ListKind::Options)
        }
        ListKind::Lines => match verb {
            "line" => Some(ListKind::Tours(arg.to_string())),
            "free" => {
                app.duty = None;
                app.service_msg = Some(("Free drive: no duty".into(), 4.0));
                None
            }
            _ => None,
        },
        ListKind::Tours(_) => {
            if let Some((line, tour)) = arg.split_once('\u{1}') {
                start_duty(app, line, tour);
            }
            None
        }
        ListKind::Drivers => {
            switch_driver(app, arg);
            Some(ListKind::Drivers)
        }
        ListKind::Clock => {
            if let Ok(secs) = arg.trim().parse::<f64>() {
                if app.lan.as_ref().is_some_and(|l| l.role == omsi_net::Role::Client) {
                    app.service_msg = Some(("In a LAN session the host sets the clock".into(), 3.0));
                } else {
                    app.shift_clock(secs);
                }
            }
            Some(ListKind::Clock)
        }
        ListKind::Hofs => {
            if let Some(p) = app.player.as_mut() {
                match omsi_vehicle::Hof::load(std::path::Path::new(arg)) {
                    Ok(h) => {
                        let name = h.name.clone();
                        p.vehicle.host.hof = Some(std::sync::Arc::new(h));
                        app.service_msg = Some((format!("Depot file: {}", name.trim()), 3.0));
                    }
                    Err(e) => app.service_msg = Some((format!("Depot file: {e}"), 4.0)),
                }
            }
            None
        }
        ListKind::PlaceLivery(bus) => Some(ListKind::PlaceHof(bus.clone(), arg.to_string())),
        ListKind::PlaceHof(bus, paint) => {
            let (bus, paint, hof) = (bus.clone(), paint.clone(), arg.trim().to_string());
            app.close_game_menu();
            app.place_vehicle(&bus, Some(paint).filter(|p| !p.is_empty()), Some(hof).filter(|h| !h.is_empty()));
            None
        }
        ListKind::Destinations if verb == "routes" => Some(ListKind::RouteNumbers),
        ListKind::RouteNumbers => {
            if let Some(p) = app.player.as_mut() {
                let hof = p.vehicle.host.hof.clone();
                let line = arg.trim();
                // (the destination stays: the one on the display now, else the first)
                let code = p.vehicle.var("IBIS_TerminusCode").unwrap_or(-1.0) as i32;
                let named = |t: &&omsi_vehicle::hof::Terminus| t.strings.first().is_some_and(|s| !s.trim().is_empty());
                let term = hof.as_ref().and_then(|h| h.termini.iter().filter(named).find(|t| t.code == code).or_else(|| h.termini.iter().find(named)));
                let name = term.and_then(|t| t.strings.first().cloned()).unwrap_or_default();
                crate::schedule::set_player_destination_directly(&mut p.vehicle, hof.as_deref(), line, &name);
                log::info!("route number set by hand: {line} (IBIS_LinieKurs {:?})", p.vehicle.var("IBIS_LinieKurs"));
                app.service_msg = Some((format!("Route {line}"), 3.0));
            }
            None
        }
        ListKind::Destinations => {
            if let Some(p) = app.player.as_mut() {
                let hof = p.vehicle.host.hof.clone();
                let code: i32 = arg.trim().parse().unwrap_or(-1);
                if let Some(t) = hof.as_ref().and_then(|h| h.termini.iter().find(|t| t.code == code)) {
                    // (the line on the IBIS stays; only the destination changes)
                    let line = p.vehicle.var("IBIS_LinieKurs").filter(|l| *l > 0.0).map(|l| format!("{}", l as i64)).unwrap_or_default();
                    let name = t.strings.first().cloned().unwrap_or_default();
                    crate::schedule::set_player_destination_directly(&mut p.vehicle, hof.as_deref(), &line, &name);
                    log::info!("destination display set by hand: {code} {} (terminus code now {:?})", name.trim(), p.vehicle.var("IBIS_TerminusCode"));
                    app.service_msg = Some((format!("Destination: {}", name.trim()), 3.0));
                }
            }
            None
        }
        ListKind::Numbers => {
            if let (Some((n, reg)), Some(p)) = (arg.split_once('\u{1}'), app.player.as_mut()) {
                let v = &mut p.vehicle;
                if let Some(i) = v.ty.program.str_var("number") {
                    v.state.str_vars[i as usize] = n.to_string();
                }
                if !reg.is_empty() {
                    if let Some(i) = v.ty.program.str_var("ident") {
                        v.state.str_vars[i as usize] = reg.to_string();
                    }
                }
                app.service_msg = Some((format!("Fleet number {n}"), 3.0));
            }
            None
        }
    }
}

/// Numbers compared as numbers where they are ("5" before "13", "N30" after "M49").
fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |s: &str| {
        let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
        (digits.parse::<u64>().unwrap_or(u64::MAX), s.to_ascii_lowercase())
    };
    key(a).cmp(&key(b))
}

/// Write one key of `~/.openomsi/settings.cfg` (the launcher's file; the other lines
/// stay as they are).
fn remember_setting(key: &str, value: &str) {
    let Ok(mut v) = omsi_launcher_lib::get_settings() else { return };
    let parsed: serde_json::Value = value.parse::<f64>().map(serde_json::Value::from).unwrap_or_else(|_| serde_json::Value::from(value));
    // a switch goes in as true/false, as the launcher's own values are: written as 1 it
    // was read as not set and saved back as its default (the pause menu's options were
    // lost with the next game)
    let parsed = match (&v[key], &parsed) {
        (serde_json::Value::Bool(_), serde_json::Value::Number(n)) => serde_json::Value::Bool(n.as_f64().unwrap_or(0.0) > 0.5),
        _ if key == "time_speed" => serde_json::Value::from(value),
        _ => parsed,
    };
    v[key] = parsed;
    if let Err(e) = omsi_launcher_lib::save_settings(&v) {
        log::warn!("settings not saved: {e:#}");
    }
}

/// The personnel files there are (content folder and OMSI 2's `Drivers`), by name.
fn driver_names(app: &App) -> Vec<String> {
    let mut names: Vec<String> = omsi_cfg::read_dir_merged("Drivers")
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("odr")))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .collect();
    let _ = app;
    names.sort_by_key(|n| n.to_ascii_lowercase());
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    names
}

/// Go on with another driver: this run so far into the old personnel file, the rest into
/// the new one.
fn switch_driver(app: &mut App, name: &str) {
    if app.career.path.is_some() {
        if let Err(e) = app.career.save() {
            log::warn!("writing the personnel file: {e}");
        }
    }
    let rel = format!("Drivers/{name}.odr");
    let mut next = crate::career::Career::load(&app.args.root, &rel);
    // (the distance and the clock of the run go on; the counters start with the new file)
    next.seconds = app.career.seconds;
    app.career = next;
    app.args.driver = Some(rel);
    app.service_msg = Some((format!("Driver: {name}"), 3.0));
}

/// The fleet numbers of the bus's `[number]` list with their registrations.
fn fleet_numbers(v: &omsi_sim::VehicleInstance) -> Vec<(String, String)> {
    let def = &v.ty.def;
    def.numbers_with_plates()
        .into_iter()
        .map(|(n, _)| {
            let reg = if def.registration_mode == 1 { String::new() } else { def.plate_of_number(&n) };
            (n, reg)
        })
        .collect()
}

/// Take on line `line`, tour `tour` from now: the duty, and the IBIS typed for it.
fn start_duty(app: &mut App, line: &str, tour: &str) {
    let (Some(w), Some(sch)) = (app.world.clone(), app.schedule.as_mut()) else { return };
    let now = app.clock.time;
    match sch.player_duty(&w, line, tour, now, None, false) {
        Ok(mut d) => {
            if let Some(p) = app.player.as_mut() {
                d.update(&mut p.vehicle, now);
                let (trip, stop) = d.trip_for_ibis();
                p.set_duty_destination(trip, stop);
            }
            app.args.line = Some(line.to_string());
            app.args.tour = Some(tour.to_string());
            app.duty = Some(d);
            app.service_msg = Some((format!("Line {line}, tour {}", tour.trim()), 4.0));
        }
        Err(e) => app.service_msg = Some((format!("No duty: {e}"), 8.0)),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn steps_wrap_round() {
        assert_eq!(super::next_step(&super::SPEEDS, 1.0), 2.0);
        assert_eq!(super::next_step(&super::SPEEDS, 15.0), 1.0);
        assert_eq!(super::next_step(&super::TRAFFIC, 35), 50);
    }

    #[test]
    fn lines_sort_as_numbers() {
        let mut v = vec!["13N", "5", "137", "N30", "92"];
        v.sort_by(|a, b| super::natural(a, b));
        assert_eq!(v, vec!["5", "13N", "92", "137", "N30"]);
    }
}