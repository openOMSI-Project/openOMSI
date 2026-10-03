//! Putting a vehicle into service by itself (Shift+U, `--autostart`), the way a driver does
//! it: the electrics on, then the starter held until the engine runs.
//!
//! A fixed list of trigger names only fits the stock buses. Mods keep the variables of the
//! stock scripts (`elec_busbar_main_sw`, `engine_starter`, `engine_on`) but wire them to
//! their own keys: the O530 Citaro turns its ignition key one notch per press of E
//! (`cp_batterietrennschalter_toggle`: key in, electrics, ignition, starter while held) and
//! M turns it back, so the stock sequence (E, then M held) pulled the key out again. The
//! start-up therefore asks the compiled scripts which triggers can write those variables,
//! presses them one at a time and watches what happens - a toggle is pressed until the
//! electrics are on, a starter is held once it cranks and let go when the engine runs.

use crate::VehicleInstance;

#[cfg(test)]
#[path = "startup/tests.rs"]
mod regressions;

/// Names tried first, as the stock scripts and `Inputs/keyboard.cfg` spell them.
const POWER_NAMES: &[&str] = &[
    "cp_batterietrennschalter_toggle",
    "kw_batterietrennschalter",
    "batterie_toggle",
];
const STARTER_NAMES: &[&str] = &[
    "kw_m_enginestart",
    "kw_m_engine_startbutton",
    "engine_start",
];

/// The stock state names, followed by harmless common alternatives found in vehicle mods.
/// They describe state, not a particular cockpit mesh: `cp_batterietrennschalter`, for
/// example, is merely a key position and must not by itself make Shift+U think the electrical
/// system is alive.
const POWER_VARIABLES: &[&str] = &[
    "elec_busbar_main_sw",
    "elec_busbar_main",
    // Hong Kong Volvo B9TL variants expose the key position as `elec_real_main`
    // and only raise `elec_busbar_main` after the first starter/ignition press.
    "elec_real_main",
    "elec_battery_on",
    "battery_on",
    "batterie_on",
    "electric_on",
    "electrics_on",
    "main_power",
];
const STARTER_VARIABLES: &[&str] = &[
    "engine_starter",
    "engine_starter_on",
    "motor_starter",
    "motor_anlasser",
];

/// Seconds a key is held for an ordinary press, and between two presses.
const PRESS: f32 = 0.2;
const GAP: f32 = 0.5;
/// Presses of one trigger before the next candidate is tried.
const PRESSES_PER_TRIGGER: u32 = 4;
/// How long the starter is held at most (the NL202's cold engine needs a good 3.5 s).
const MAX_CRANK: f32 = 8.0;
/// LECIP/DDU scripts normally accept login after their power-up timer expires.
const DDU_LOGIN_DELAY: f32 = 7.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Power,
    /// Vehicle systems such as the Hong Kong LECIP DDU must be logged in before cranking.
    Login,
    /// Return selector-style gear controls to neutral before the starter is pressed.
    Neutral,
    Ignition,
    Crank,
    /// The starter was let go with the engine running: watch it for a moment, a cold
    /// engine that only fired a few times dies again (and is cranked once more).
    Verify,
    /// The displays that have a power switch of their own (the Procity's destination
    /// display, `sw_afisaj`): each such switch is pressed once.
    Displays,
    /// Displays still blank after that: the power buttons named for a display (the LiAZ's
    /// IBIS wants `IBIS_POWER` held while its script's frame reads the button), each held
    /// like a finger would, until the display shows something.
    DisplayPower,
    Shutdown,
    Done,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SimClock, VehicleHost, VehicleType};
    use std::sync::Arc;

    /// A mod need not name its electrical controls like the stock buses.  This is the
    /// failure mode behind the one-second starter sound: the custom starter is held until
    /// its script turns `motor_on` on, rather than being released because there is no
    /// `elec_busbar_main_sw` variable.
    #[test]
    fn starts_a_bus_with_custom_battery_and_starter_names() {
        let dir = std::env::temp_dir().join(format!(
            "omsi-startup-custom-controls-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("custom.bus"),
            "[model]\nmodel.cfg\n[varnamelist]\n1\nvars.txt\n[script]\n1\nmain.osc\n",
        )
        .unwrap();
        std::fs::write(dir.join("model.cfg"), "").unwrap();
        std::fs::write(dir.join("vars.txt"), "battery_on\nmotor_anlasser\nmotor_on\n")
            .unwrap();
        std::fs::write(
            dir.join("main.osc"),
            "{init}\n0 (S.L.battery_on) 0 (S.L.motor_anlasser) 0 (S.L.motor_on)\n{end}\n\
             {trigger:master_battery}\n1 (S.L.battery_on)\n{end}\n\
             {trigger:motor_anlasser}\n1 (S.L.motor_anlasser)\n{end}\n\
             {frame}\n(L.L.battery_on) (L.L.motor_anlasser) && (S.L.motor_on)\n{end}\n",
        )
        .unwrap();

        let ty = Arc::new(VehicleType::load(&dir, &dir.join("custom.bus")).unwrap());
        assert_eq!(
            ty.program.trigger_names(),
            vec!["master_battery".to_string(), "motor_anlasser".to_string()],
            "{:?}",
            ty.program.errors
        );
        let mut v = VehicleInstance::new(ty, VehicleHost::new(SimClock::default()));
        let mut start = StartUp::new(&v, &[]);
        for _ in 0..300 {
            let running = start.tick(&mut v, &[], 1.0 / 30.0);
            v.update(1.0 / 30.0);
            if !running {
                break;
            }
        }
        assert!(power_on(&v), "{:?}", start.report);
        assert!(engine_running(&v), "{:?}", start.report);
        assert!(start.report.iter().any(|s| s.contains("engine started")), "{:?}", start.report);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn logs_in_and_selects_neutral_before_starting() {
        let dir = std::env::temp_dir().join(format!(
            "omsi-startup-login-neutral-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("login.bus"),
            "[model]\nmodel.cfg\n[varnamelist]\n1\nvars.txt\n[script]\n1\nmain.osc\n",
        )
        .unwrap();
        std::fs::write(dir.join("model.cfg"), "").unwrap();
        std::fs::write(
            dir.join("vars.txt"),
            "battery_on\nddu_login\nddu_power_timer\ncockpit_gangN\nengine_on\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("main.osc"),
            "{init}\n0 (S.L.battery_on) 0 (S.L.ddu_login) 0 (S.L.ddu_power_timer) 0.7 (S.L.cockpit_gangN) 0 (S.L.engine_on)\n{end}\n\\
             {trigger:master_battery}\n1 (S.L.battery_on)\n{end}\n\\
             {trigger:ddu_login}\n(L.L.ddu_power_timer) 1 > {if} 1 (S.L.ddu_login) {endif}\n{end}\n\\
             {trigger:automatic_N}\n0 (S.L.cockpit_gangN)\n{end}\n\\
             {trigger:kw_m_enginestart}\n(L.L.ddu_login) (L.L.cockpit_gangN) 0 = && {if} 1 (S.L.engine_on) {endif}\n{end}\n\\
             {frame}\n(L.L.battery_on) 1 = {if} (L.L.ddu_power_timer) (L.S.Timegap) + (S.L.ddu_power_timer) {endif}\n{end}\n",
        )
        .unwrap();

        let ty = Arc::new(VehicleType::load(&dir, &dir.join("login.bus")).unwrap());
        let mut v = VehicleInstance::new(ty, VehicleHost::new(SimClock::default()));
        let mut start = StartUp::new(&v, &[]);
        for _ in 0..900 {
            let running = start.tick(&mut v, &[], 1.0 / 30.0);
            v.update(1.0 / 30.0);
            if !running {
                break;
            }
        }
        assert!(engine_running(&v), "{:?}", start.report);
        assert!(start.report.iter().any(|s| s.contains("DDU login")), "{:?}", start.report);
        assert!(start.report.iter().any(|s| s.contains("neutral")), "{:?}", start.report);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn treats_real_main_as_power_before_two_stage_starter() {
        let dir = std::env::temp_dir().join(format!("omsi-startup-real-main-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("real.bus"), "[model]\nmodel.cfg\n[varnamelist]\n1\nvars.txt\n[script]\n1\nmain.osc\n").unwrap();
        std::fs::write(dir.join("model.cfg"), "").unwrap();
        std::fs::write(dir.join("vars.txt"), "elec_real_main\nelec_busbar_main\nengine_on\n").unwrap();
        std::fs::write(dir.join("main.osc"), "{init}\n0 (S.L.elec_real_main) 0 (S.L.elec_busbar_main) 0 (S.L.engine_on)\n{end}\n\\
             {trigger:master_battery}\n1 (S.L.elec_real_main)\n{end}\n\\
             {trigger:kw_m_enginestart}\n(L.L.elec_busbar_main) 0 = {if} 1 (S.L.elec_busbar_main) {else} 1 (S.L.engine_on) {endif}\n{end}\n\\
             {frame}\n{end}\n").unwrap();
        let ty = Arc::new(VehicleType::load(&dir, &dir.join("real.bus")).unwrap());
        let mut v = VehicleInstance::new(ty, VehicleHost::new(SimClock::default()));
        let mut start = StartUp::new(&v, &[]);
        for _ in 0..300 {
            if !start.tick(&mut v, &[], 1.0 / 30.0) { break; }
            v.update(1.0 / 30.0);
        }
        assert!(engine_running(&v), "{:?}", start.report);
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// A running start-up; `tick` it every frame until it returns false.
#[derive(Debug, Clone)]
pub struct StartUp {
    step: Step,
    /// Seconds since the current press began.
    t: f32,
    /// The trigger being pressed and whether its `_off` was fired already.
    pressed: Option<(String, bool)>,
    /// Holding the starter (crank time so far).
    cranking: Option<f32>,
    candidates: Vec<String>,
    candidate: usize,
    presses: u32,
    cranks: u32,
    /// `DisplayPower`: the text displays still blank.
    blank: Vec<String>,
    /// `Shutdown`: the first this many candidates stop the engine (held until it dies).
    stop_n: usize,
    /// What was done, for the log and the HUD.
    pub report: Vec<String>,
}

/// A loose match for a deliberately named input, not a variable.  This is only the final
/// fallback after the exact variables and stock trigger names; it lets Shift+U find controls
/// such as `master_battery`, `key_ignition` and `motor_anlasser` in otherwise conventional
/// bus scripts without accidentally operating lamp or door controls.
fn semantic_triggers(
    p: &omsi_script::Program,
    bound: &[String],
    words: &[&str],
) -> Vec<String> {
    let matches = |n: &str| {
        let n = n.to_ascii_lowercase();
        !n.ends_with("_off")
            && !n.ends_with("_drag")
            && !n.starts_with("ai_")
            && words.iter().any(|word| n.contains(word))
    };
    let mut out = Vec::new();
    for n in bound.iter().chain(p.trigger_names().iter()) {
        let n = n.to_ascii_lowercase();
        if matches(&n) && p.trigger(&n).is_some() && !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

/// The press triggers (no `_off`/`_drag` halves) that can write one of `vars`, the preferred
/// names first: bound keys, then the stock names, then whatever else the scripts offer.  The
/// semantic names are a last-resort bridge for a mod that does not keep OMSI's stock variable
/// spelling.
fn candidates(
    v: &VehicleInstance,
    vars: &[&str],
    stock: &[&str],
    bound: &[String],
    semantic_words: &[&str],
) -> Vec<String> {
    let p = &v.ty.program;
    let storing: Vec<String> = vars
        .iter()
        .flat_map(|var| p.triggers_setting(var))
        .fold(Vec::new(), |mut all, n| {
            if !all.iter().any(|x| x.eq_ignore_ascii_case(&n)) {
                all.push(n);
            }
            all
        });
    let has_state = vars.iter().any(|var| p.var(var).is_some());
    let usable = |n: &str| !n.ends_with("_off") && !n.ends_with("_drag") && p.trigger(n).is_some();
    let mut out: Vec<String> = Vec::new();
    let add = |n: &str, out: &mut Vec<String>| {
        let n = n.to_ascii_lowercase();
        if usable(&n) && !out.contains(&n) {
            out.push(n);
        }
    };
    for b in bound {
        if storing.iter().any(|s| s.eq_ignore_ascii_case(b)) {
            add(b, &mut out);
        }
    }
    for s in stock {
        // a stock name counts when it writes the variable, or when the scripts have no
        // such variable at all (then nothing can be checked and the name is all there is)
        if !has_state || storing.iter().any(|x| x.eq_ignore_ascii_case(s)) {
            add(s, &mut out);
        }
    }
    for s in &storing {
        add(s, &mut out);
    }
    for s in semantic_triggers(p, bound, semantic_words) {
        add(&s, &mut out);
    }
    out
}

fn flag(v: &VehicleInstance, name: &str) -> Option<bool> {
    v.var(name).map(|x| x > 0.5)
}

/// `Some` when this vehicle exposes any of `names`; a system is on if *any* state rail says
/// so.  Several buses leave the main battery bus live after the key-position switch itself
/// falls back to zero, so looking only at the first present variable made an already powered
/// bus look dead.
fn any_flag(v: &VehicleInstance, names: impl IntoIterator<Item = impl AsRef<str>>) -> Option<bool> {
    let mut known = false;
    for name in names {
        if let Some(value) = flag(v, name.as_ref()) {
            known = true;
            if value {
                return Some(true);
            }
        }
    }
    known.then_some(false)
}

fn custom_state_names<'a>(v: &'a VehicleInstance, kind: &str) -> Vec<&'a str> {
    v.ty.program.var_names.iter().map(String::as_str).filter(|name| {
        let n = name.to_ascii_lowercase();
        if n.starts_with("cp_") {
            return false;
        }
        match kind {
            "power" => {
                (n.contains("busbar") && n.contains("main"))
                    || ((n.contains("batter") || n.contains("battery"))
                        && (n.contains("_on") || n.contains("main") || n.contains("trenner")))
            }
            "starter" => n.contains("starter") || n.contains("anlasser"),
            "engine" => {
                (n.contains("engine") || n.contains("motor"))
                    && (n.ends_with("_on") || n.contains("running"))
            }
            _ => false,
        }
    }).collect()
}

fn power_state(v: &VehicleInstance) -> Option<bool> {
    any_flag(v, POWER_VARIABLES.iter().copied())
        .or_else(|| any_flag(v, custom_state_names(v, "power")))
}

fn starter_engaged(v: &VehicleInstance) -> bool {
    any_flag(v, STARTER_VARIABLES.iter().copied())
        .or_else(|| any_flag(v, custom_state_names(v, "starter")))
        .unwrap_or(false)
        // Some buses request a delayed start without exposing a starter flag. This is
        // an enum (1=start, 2=stop, 3=off), not an electrical on/off flag.
        || v.var("engine_ignition") == Some(1.0)
}

/// The electrics are on: the main switch, or the busbar where a bus has no switch variable.
pub fn power_on(v: &VehicleInstance) -> bool {
    power_state(v).unwrap_or(false)
}

/// The engine speed the scripts keep, when they keep one.
fn engine_rpm(v: &VehicleInstance) -> Option<f32> {
    ["engine_n", "engine_rpm", "motor_n", "motor_rpm"].into_iter().find_map(|n| v.var(n))
}

/// The engine has caught: it runs and turns faster than a starter turns it. The PAZ's
/// carburettor engine sets `engine_on` on its first ignitions at 150 rpm, and let go there it
/// died again.
fn engine_caught(v: &VehicleInstance) -> bool {
    engine_running(v) && engine_rpm(v).map(|n| n > 300.0).unwrap_or(true)
}

/// The engine runs. Script flags are authoritative; RPM is a fallback for scripts without
/// one. A starter can turn the engine above 350 rpm before combustion begins.
pub fn engine_running(v: &VehicleInstance) -> bool {
    any_flag(v, ["engine_on", "engine_running", "motor_on", "motor_running"])
        .or_else(|| any_flag(v, custom_state_names(v, "engine")))
        .unwrap_or_else(|| ["engine_n", "engine_rpm", "motor_n", "motor_rpm"]
            .into_iter()
            .filter_map(|n| v.var(n))
            .any(|n| n > 350.0))
}

/// The engine has come to rest: by its speed where the bus has one (the flags of some
/// scripts drop for a frame the moment the fuel is cut, while the engine still turns - the
/// D86's stop button was let go then and the engine ran on; let go at 150 rpm, its script
/// fired it up again: it is 0 only under 100 with the fuel still cut), else by the flags.
fn engine_dead(v: &VehicleInstance) -> bool {
    let rpm: Vec<f32> = ["engine_n", "engine_rpm", "motor_n", "motor_rpm"].into_iter().filter_map(|n| v.var(n)).collect();
    if rpm.is_empty() {
        !engine_running(v)
    } else {
        rpm.iter().all(|n| *n < 30.0)
    }
}

/// Switches that do nothing but turn a display on: a clickable trigger whose whole job is
/// toggling the variable a display mesh (one showing a script or text texture) is
/// `[visible]` by, while that display is dark. The Procity's destination display has its
/// own button on the dashboard (`sw_afisaj`), which a driver presses when taking the bus
/// over; Shift+U does it too. Menus, modding switches and anything that is not a plain
/// on/off toggle of that one variable are left alone.
fn display_switches(v: &VehicleInstance) -> Vec<(String, String, f32)> {
    let p = &v.ty.program;
    let clickable: Vec<String> = v
        .ty
        .model
        .meshes
        .iter()
        .filter_map(|m| m.mouse_event.as_ref().map(|e| e.trim().to_ascii_lowercase()))
        .collect();
    let mut out: Vec<(String, String, f32)> = Vec::new();
    // `(L.L.x) ! (S.L.x)`: a plain toggle of `x`
    let toggles_var = |t: &str, id: omsi_script::VarId| {
        p.trigger(t).map(|b| {
            p.blocks[b as usize].ops.windows(3).any(|w| {
                matches!(w, [omsi_script::Op::Load(a), omsi_script::Op::Not, omsi_script::Op::Store(c)] if *a == id && *c == id)
            })
        }).unwrap_or(false)
    };
    // Displays whose script only writes them while a switch is on: the LiAZ's Annax matrix
    // wants `matrix_power` (its "Matrix" switch) and the 12 V circuit (`elec_12v`). The
    // switches are the clickable plain toggles among the variables read by the blocks that
    // write a display's string (a `[texttexture]`) or draw on a script texture.
    let display_strings: Vec<u32> = v
        .ty
        .model
        .text_textures
        .iter()
        .filter_map(|t| p.str_var(&t.variable))
        .map(|id| id as u32)
        .collect();
    let mut read_by_displays: Vec<omsi_script::VarId> = Vec::new();
    for b in &p.blocks {
        let writes_display = b.ops.iter().any(|op| match op {
            omsi_script::Op::StoreStr(id) => display_strings.contains(&(*id as u32)),
            omsi_script::Op::Callback(n) => p.name(*n).to_ascii_lowercase().starts_with("st"),
            _ => false,
        });
        if writes_display {
            for op in &b.ops {
                if let omsi_script::Op::Load(id) = op {
                    if !read_by_displays.contains(id) {
                        read_by_displays.push(*id);
                    }
                }
            }
        }
    }
    // the dashboard displays read every switch they show the state of; only a switch named
    // as the display's power (or the 12 V circuit displays hang on) is one to turn on
    let display_power = ["matrix", "anzeige", "display", "afisaj", "tablo", "12v"];
    for id in read_by_displays {
        let var = p.var_names[id as usize].clone();
        let lower = var.to_ascii_lowercase();
        if !display_power.iter().any(|w| lower.contains(w)) || v.var(&var).map(|x| x.abs() > 1e-3).unwrap_or(true) {
            continue;
        }
        for t in p.triggers_setting(&var) {
            if clickable.contains(&t) && toggles_var(&t, id) && !out.iter().any(|(n, _, _)| *n == t) {
                out.push((t, var.clone(), 1.0));
            }
        }
    }
    for m in &v.ty.model.meshes {
        let Some((var, want)) = m.visible.as_ref() else { continue };
        if (*want - 1.0).abs() > 1e-3 || m.mouse_event.is_some() {
            continue;
        }
        let shows = m.materials.iter().any(|mat| {
            mat.use_script_texture.is_some()
                || mat.use_text_texture.is_some()
                || mat.transmap.as_deref().map(|t| t.trim().starts_with("\\S:")).unwrap_or(false)
                || mat.texture.trim().starts_with("\\S:")
        });
        if !shows || v.var(var).map(|x| (x - want).abs() < 1e-3).unwrap_or(true) {
            continue;
        }
        // a variable that also shows something else at another value chooses between
        // alternatives (the C2's small or big ATRON, an odometer's modes): not a power switch
        let selects = v.ty.model.meshes.iter().any(|o| {
            o.visible.as_ref().map(|(n, x)| n.eq_ignore_ascii_case(var) && (*x - want).abs() > 1e-3).unwrap_or(false)
        });
        if selects {
            continue;
        }
        let Some(id) = p.var(var) else { continue };
        for t in p.triggers_setting(var) {
            if !clickable.contains(&t) || out.iter().any(|(n, _, _)| *n == t) {
                continue;
            }
            // `(L.L.x) ! (S.L.x)`: a plain toggle of the display's variable
            let Some(b) = p.trigger(&t) else { continue };
            let ops = &p.blocks[b as usize].ops;
            let toggles = ops.windows(3).any(|w| {
                matches!(w, [omsi_script::Op::Load(a), omsi_script::Op::Not, omsi_script::Op::Store(c)] if *a == id && *c == id)
            });
            if toggles {
                out.push((t, var.clone(), *want));
            }
        }
    }
    out
}

/// The `[texttexture]` strings of the model that show nothing.
fn blank_displays(v: &VehicleInstance) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in &v.ty.model.text_textures {
        if v.ty.program.str_var(&t.variable).is_some()
            && v.str_var(&t.variable).trim().is_empty()
            && !out.contains(&t.variable)
        {
            out.push(t.variable.clone());
        }
    }
    out
}

/// Clickable buttons named as a display's power: `IBIS_POWER`, `matrix_on`, `anzeige_ein` ...
fn display_power_buttons(v: &VehicleInstance) -> Vec<String> {
    let p = &v.ty.program;
    let displays = ["ibis", "matrix", "anzeige", "display", "afisaj", "tablo", "informator", "annax", "zza", "lcd"];
    let power = ["power", "pwr", "strom", "_on", "_ein", "onoff", "on_off", "an_aus", "toggle", "_sw"];
    let mut out: Vec<String> = Vec::new();
    for m in &v.ty.model.meshes {
        let Some(e) = m.mouse_event.as_ref().map(|e| e.trim().to_string()) else { continue };
        let lower = e.to_ascii_lowercase();
        if lower.ends_with("_off")
            || !displays.iter().any(|w| lower.contains(w))
            || !power.iter().any(|w| lower.contains(w))
            || p.trigger(&e).is_none()
            || out.iter().any(|o| o.eq_ignore_ascii_case(&e))
        {
            continue;
        }
        out.push(e);
    }
    out.sort();
    out
}

impl StartUp {
    /// `bound` are the vehicle actions of `Inputs/keyboard.cfg` (the keys a driver presses).
    pub fn new(v: &VehicleInstance, bound: &[String]) -> StartUp {
        let mut s = StartUp {
            step: Step::Power,
            t: 0.0,
            pressed: None,
            cranking: None,
            candidates: Vec::new(),
            candidate: 0,
            presses: 0,
            cranks: 0,
            blank: Vec::new(),
            stop_n: 0,
            report: Vec::new(),
        };
        // (a bus under power whose engine has died - a crash stalled it - is started again:
        // taken for a running bus, it was "shut down" by pressing its switches round and
        // round, and every Shift+U after said "shutting down" for good)
        if engine_running(v) {
            s.enter(v, bound, Step::Shutdown);
        } else {
            s.enter(v, bound, Step::Power);
        }
        s
    }

    /// Everything was already done when the start-up began.
    pub fn nothing_to_do(v: &VehicleInstance) -> bool {
        power_on(v) && engine_running(v)
    }

    fn enter(&mut self, v: &VehicleInstance, bound: &[String], step: Step) {
        self.step = step;
        self.t = 0.0;
        self.pressed = None;
        self.candidate = 0;
        self.presses = 0;
        self.candidates = match step {
            Step::Power if !power_on(v) => candidates(
                v,
                POWER_VARIABLES,
                POWER_NAMES,
                bound,
                &["batter", "battery", "hauptschalter", "main_switch", "mainswitch", "mainpower", "master_switch", "masterpower"],
            ),
            Step::Ignition => semantic_triggers(
                &v.ty.program,
                bound,
                &["zuendung", "zündung", "zundung", "ignition", "schluessel", "schlüssel"],
            ),
            Step::Crank if !engine_running(v) => candidates(
                v,
                STARTER_VARIABLES,
                STARTER_NAMES,
                bound,
                &["engine_start", "enginestart", "engine_starter", "starter", "anlass", "anlasser", "motor_start", "motorstart", "start_stop"],
            ),
            // Shutting down: what stops the engine (a stop button, where a bus has one),
            // then what switches the electrics off - the very toggles that switched them on
            // (the stock buses and most mods stop the engine with the battery switch or the
            // key) - then an ignition switch. Pressing the starter's `_off` half, as before,
            // only let go of a starter nobody held: the engine ran on, and the next Shift+U
            // said "shutting down" again.
            Step::Shutdown => {
                let mut list = semantic_triggers(
                    &v.ty.program,
                    bound,
                    // (the stock `kw_m_engineshutdown`, "Motorabstellung", held down)
                    &["enginestop", "engine_stop", "motorstop", "motor_stop", "stop_engine", "motor_aus", "motoraus", "engine_kill", "engineshutdown", "engine_shutdown", "motorabstell", "engine_off", "motor_off"],
                );
                self.stop_n = list.len();
                for n in candidates(v, POWER_VARIABLES, POWER_NAMES, bound, &["batter", "battery", "hauptschalter", "main_switch", "mainswitch", "mainpower", "master_switch", "masterpower"])
                    .into_iter()
                    .chain(semantic_triggers(&v.ty.program, bound, &["zuendung", "zündung", "zundung", "ignition", "schluessel", "schlüssel"]))
                {
                    if !list.contains(&n) {
                        list.push(n);
                    }
                }
                list
            }
            _ => Vec::new(),
        };
        if omsi_cfg::env::var_os("OMSI_DEBUG_STARTUP").is_some() {
            log::info!("start-up {step:?}: candidates {:?}", self.candidates);
        }
    }

    pub fn running(&self) -> bool {
        self.step != Step::Done
    }

    /// Switching the vehicle off (rather than putting it into service).
    pub fn shutting_down(&self) -> bool {
        self.step == Step::Shutdown
    }

    fn release(&mut self, v: &mut VehicleInstance) {
        if let Some((name, released)) = self.pressed.as_mut() {
            if !*released {
                if !name.ends_with("_off") {
                    v.trigger(&format!("{name}_off"));
                }
                *released = true;
            }
        }
    }

    /// Advance by `dt`; false once the start-up is over.
    pub fn tick(&mut self, v: &mut VehicleInstance, bound: &[String], dt: f32) -> bool {
        self.t += dt;
        match self.step {
            Step::Done => return false,
            Step::Power => {
                if power_on(v) && self.pressed.as_ref().map(|p| p.1).unwrap_or(true) {
                    if self.presses > 0 {
                        self.report.push(format!(
                            "electrics on ({} x {})",
                            self.presses, self.candidates[self.candidate]
                        ));
                    }
                    self.enter(v, bound, Step::Login);
                } else {
                    self.press_loop(v, bound, Step::Ignition, "electrics");
                }
            }
            Step::Login => {
                let needs_login = v.var("ddu_login").is_some_and(|n| n < 0.5)
                    && v.ty.program.trigger("ddu_login").is_some();
                if !needs_login {
                    self.enter(v, bound, Step::Neutral);
                } else {
                    let timer_ready = v
                        .var("ddu_power_timer")
                        .is_none_or(|n| n >= DDU_LOGIN_DELAY)
                        || self.t >= DDU_LOGIN_DELAY + 1.0;
                    if timer_ready {
                        v.trigger("ddu_login");
                        v.trigger("ddu_login_off");
                        self.report.push("DDU login".to_string());
                        self.enter(v, bound, Step::Neutral);
                    }
                }
            }
            Step::Neutral => {
                if let Some(n) = ["automatic_N"]
                    .into_iter()
                    .find(|n| v.ty.program.trigger(n).is_some())
                {
                    v.trigger(n);
                    v.trigger(&format!("{n}_off"));
                    self.report.push(format!("neutral ({n})"));
                }
                self.enter(v, bound, Step::Ignition);
            }
            Step::Ignition => {
                // the optional ignition switches of some mods: one press each
                if self.t >= GAP || self.candidates.is_empty() {
                    if let Some(n) = self.candidates.get(self.candidate).cloned() {
                        v.trigger(&n);
                        v.trigger(&format!("{n}_off"));
                        self.report.push(format!("ignition ({n})"));
                        self.candidate += 1;
                        self.t = 0.0;
                    } else {
                        self.enter(v, bound, Step::Crank);
                    }
                }
            }
            Step::Crank => {
                // A script may engage the starter several frames after the press. Check
                // before press_loop can release it, and include the pending start delay.
                if self.cranking.is_none()
                    && self.pressed.as_ref().is_some_and(|(_, released)| !released)
                    && starter_engaged(v)
                {
                    self.cranking = Some(self.t);
                }
                if let Some(held) = self.cranking.as_mut() {
                    *held = self.t;
                    let held = *held;
                    // An unknown electrical-state variable is not evidence that the power
                    // went away.  Releasing immediately here was the reason a custom bus
                    // whose only electrical state was `battery_on` produced one starter
                    // sound and then nothing.  Stop early only when a known power state
                    // explicitly dropped out.
                    if engine_caught(v)
                        || held > MAX_CRANK
                        || matches!(power_state(v), Some(false))
                    {
                        self.release(v);
                        self.cranking = None;
                        self.cranks += 1;
                        let name = self
                            .candidates
                            .get(self.candidate)
                            .cloned()
                            .unwrap_or_default();
                        if engine_running(v) {
                            self.report
                                .push(format!("engine started ({name} held {held:.1} s)"));
                            self.step = Step::Verify;
                            self.t = 0.0;
                        } else if self.cranks >= 3 {
                            self.report.push(format!(
                                "the engine did not start ({name} held {held:.1} s, {} tries)",
                                self.cranks
                            ));
                            self.step = Step::Done;
                        } else {
                            // crank again after a pause, as a driver would
                            self.t = -1.0;
                            self.pressed = None;
                        }
                    }
                } else if engine_running(v) && self.pressed.as_ref().map(|p| p.1).unwrap_or(true) {
                    self.step = Step::Displays;
                } else {
                    self.press_loop(v, bound, Step::Done, "starter");
                    // Also observe an immediate start request from a newly pressed key.
                    if self.pressed.as_ref().is_some_and(|(_, released)| !released)
                        && starter_engaged(v)
                    {
                        self.cranking = Some(self.t);
                    }
                }
            }
            Step::Verify => {
                if !engine_running(v) {
                    if self.cranks >= 3 {
                        self.report.push("the engine died again after starting".to_string());
                        self.step = Step::Done;
                    } else {
                        self.report.push("the engine died again: cranking once more".to_string());
                        self.step = Step::Crank;
                        self.t = -1.0;
                        self.pressed = None;
                        self.cranking = None;
                    }
                } else if self.t > 2.0 {
                    self.step = Step::Displays;
                    self.t = 0.0;
                }
            }
            Step::Displays => {
                // an automatic gearbox into D, the foot on the brake as the ZF scripts want it
                // (`(L.L.Brake) 0 >` for D: the GX7767 E500 MMC stayed in N)
                if v.ty.program.trigger("automatic_D").is_some() && v.var("antrieb_getr_gangvorwahl").is_none_or(|g| g < 3.5) {
                    let brake = v.var("Brake");
                    v.set_var("Brake", 1.0);
                    v.trigger("automatic_D");
                    v.trigger("automatic_D_off");
                    if let Some(b) = brake {
                        v.set_var("Brake", b);
                    }
                    self.report.push("gear D".to_string());
                }
                for (name, var, want) in display_switches(v) {
                    for _ in 0..3 {
                        v.trigger(&name);
                        v.trigger(&format!("{name}_off"));
                        if v.var(&var).map(|x| (x - want).abs() < 1e-3).unwrap_or(true) {
                            self.report.push(format!("display switched on ({name})"));
                            break;
                        }
                    }
                }
                self.blank = blank_displays(v);
                self.candidates = if self.blank.is_empty() { Vec::new() } else { display_power_buttons(v) };
                self.candidate = 0;
                self.t = 0.0;
                // (the starter's press is over: a stale one was taken for this step's own)
                self.pressed = None;
                self.step = Step::DisplayPower;
            }
            Step::DisplayPower => {
                match (self.candidates.get(self.candidate).cloned(), &self.pressed) {
                    (None, _) => self.step = Step::Done,
                    // pressed and held: the script reads the button in its frame
                    (Some(name), None) => {
                        v.trigger(&name);
                        self.pressed = Some((name, false));
                        self.t = 0.0;
                    }
                    (Some(_), Some((_, false))) if self.t >= 0.4 => self.release(v),
                    (Some(name), Some((_, true))) if self.t >= 1.5 => {
                        self.pressed = None;
                        let still = blank_displays(v);
                        if still.len() < self.blank.len() {
                            self.report.push(format!("display switched on ({name})"));
                        }
                        self.blank = still;
                        self.candidate += 1;
                        if self.blank.is_empty() {
                            self.step = Step::Done;
                        }
                    }
                    _ => {}
                }
            }
            Step::Shutdown => {
                let off = !power_on(v) && engine_dead(v);
                let n = self.candidates.len().max(1);
                let stopping = self.candidate % n < self.stop_n;

                match &self.pressed {
                    _ if off => {
                        self.release(v);
                        self.report.push("switched off".to_string());
                        self.step = Step::Done;
                    }
                    // an engine stop button is held until the engine has died (a few seconds)
                    Some((_, false)) if stopping && (self.t >= 8.0 || engine_dead(v)) => self.release(v),
                    Some((_, false)) if !stopping && self.t >= PRESS => self.release(v),
                    // (the engine stopped already: no stop button pressed for nothing)
                    None if stopping && engine_dead(v) => self.candidate += 1,
                    // (a second to see what the press did: a dying engine runs down)
                    Some((_, true)) if self.t >= 1.2 => {
                        self.pressed = None;
                        self.candidate += 1;
                    }
                    None => {
                        // (the list twice over: the key of some mods turns one notch a press)
                        let n = self.candidates.len();
                        if n == 0 || self.candidate >= n * 2 {
                            self.report.push(format!("could not switch off ({} tried)", self.candidates.join(", ")));
                            self.step = Step::Done;
                        } else {
                            let name = self.candidates[self.candidate % n].clone();
                            v.trigger(&name);
                            self.presses += 1;
                            self.report.push(format!("pressed {name}"));
                            self.pressed = Some((name, false));
                            self.t = 0.0;
                        }
                    }
                    _ => {}
                }
            }
        }
        if self.step == Step::Done {
            self.release(v);
            return false;
        }
        true
    }

    /// Press the current candidate, release it after `PRESS`, press again after `GAP`;
    /// move on to the next candidate after a few presses and to `next` when all failed.
    fn press_loop(&mut self, v: &mut VehicleInstance, bound: &[String], next: Step, what: &str) {
        if self.cranking.is_some() {
            return;
        }
        match &self.pressed {
            Some((_, false)) if self.t >= PRESS => self.release(v),
            Some((_, true)) | None if self.t >= GAP || self.pressed.is_none() && self.t >= 0.0 => {
                // (the starter longer: a bus's self-test after the key may hold it back for a
                // few seconds - the GX7767 E500 MMC's lamp test takes four - and a driver
                // presses it again once the lamps are out)
                let per_trigger = if what == "starter" { PRESSES_PER_TRIGGER * 4 } else { PRESSES_PER_TRIGGER };
                if self.presses >= per_trigger {
                    self.candidate += 1;
                    self.presses = 0;
                }
                match self.candidates.get(self.candidate).cloned() {
                    Some(name) => {
                        v.trigger(&name);
                        self.presses += 1;
                        self.pressed = Some((name, false));
                        self.t = 0.0;
                    }
                    None => {
                        if !self.candidates.is_empty() {
                            self.report.push(format!(
                                "could not work the {what} ({} tried)",
                                self.candidates.join(", ")
                            ));
                        }
                        self.enter(v, bound, next);
                    }
                }
            }
            _ => {}
        }
    }
}
