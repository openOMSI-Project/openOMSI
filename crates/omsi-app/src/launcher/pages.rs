//! The launcher's other pages: the driver's profile, the settings, the key bindings, the
//! running games, the mods and where things are.

use super::state::{fmt_bytes, hhmm, short_map};
use super::theme::*;
use super::ui::{id_of, ButtonKind, Ui};
use super::Launcher;
use glam::Vec2;
use omsi_launcher_lib as core;
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};
use serde_json::{json, Value};

#[derive(Default)]
pub struct PagesView {
    pub new_driver: String,
    pub confirm_delete: Option<std::time::Instant>,
    /// The "reset every setting" dialog is open.
    pub confirm_reset: bool,
    pub kb_filter: [String; 2],
    /// (section, index) of the binding waiting for a key.
    pub capturing: Option<(usize, usize)>,
    pub drop_hover: bool,
    pub setup_root: Option<String>,
    pub setup_game: Option<String>,
    /// The Controls page's tab: 0 the keyboard, 1 the game controllers.
    pub controls_tab: usize,
    pub pads: PadsView,
    pub tt: super::timetable::TimetableView,
}

/// The game controllers tab: the devices `gamectrler.cfg` sets up, the ones connected now,
/// the one shown, a button being waited for.
#[derive(Default)]
pub struct PadsView {
    pub io: Option<crate::controllers::Devices>,
    /// The set-up assistant, while it runs.
    pub wizard: Option<Wizard>,
    pub tried: bool,
    pub devices: Option<Vec<crate::controllers::DeviceCfg>>,
    pub selected: usize,
    /// Waiting for a button of the shown device to be pressed (to add its binding).
    pub capturing: bool,
    /// A button found through "Add a button", kept visible even past the highlight.
    pub revealed_button: Option<usize>,
    pub dirty: bool,
    /// The button last pressed on the shown device and when: its line is lit, so that one
    /// sees which it is and what it does, and can give it an action there.
    pub last_pressed: Option<(usize, std::time::Instant)>,
}

/// The set-up assistant of a device: the player lets go of everything, then turns the wheel
/// to the left and presses each pedal in turn; what moved most each time is that control
/// (and which way it runs), as OMSI's options dialog has the player choose by hand.
pub struct Wizard {
    pub step: usize,
    /// Where each axis rests, and where it stood at each step (left, throttle, brake,
    /// clutch).
    pub rest: [Option<f32>; 8],
    pub at: Vec<[Option<f32>; 8]>,
    pub error: Option<String>,
}

// --- profile --------------------------------------------------------------------------------

pub fn profile(l: &mut Launcher, area: Rect) {
    let body = l.page_title(area, "Profile", "The driver whose personnel file the game writes: hours, kilometres, punctuality, tickets.");
    let left_w = (body.w * 0.56).min(700.0);
    let left = Rect::new(body.x, body.y, left_w, body.h);
    let right = Rect::new(body.x + left_w + GAP * 2.0, body.y, body.w - left_w - GAP * 2.0, body.h);
    // driver chooser
    let top = Rect::new(left.x, left.y, left.w, 132.0);
    l.ui.panel(top);
    let inner = l.ui.heading(Rect::new(top.x + 18.0, top.y + 14.0, top.w - 36.0, top.h - 28.0), "Driver", Some("person"));
    let names = l.state.profiles.clone();
    let mut sel = names.iter().position(|n| *n == l.state.config.profile).unwrap_or(0);
    let half = (inner.w - GAP) * 0.5;
    if !names.is_empty() && l.ui.select("profile", Rect::new(inner.x, inner.y, half, ROW), &mut sel, &names) {
        l.state.config.profile = names[sel].clone();
        let _ = core::save_config(&l.state.config);
        l.state.load_profile();
        l.state.touched();
    }
    let danger_armed = l.pages.confirm_delete.map(|t| t.elapsed().as_secs() < 4).unwrap_or(false);
    if l.ui.button("profile-delete", Rect::new(inner.x + half + GAP, inner.y, half, ROW), if danger_armed { "Click again to delete" } else { "Delete this driver" }, Some("delete"), ButtonKind::Danger) {
        if danger_armed {
            let name = l.state.config.profile.clone();
            match core::delete_profile(&name) {
                Ok(()) => {
                    l.state.set_status(format!("Personnel file of {name} deleted."), false);
                    l.state.load_profiles();
                }
                Err(e) => l.state.set_status(format!("{e:#}"), true),
            }
            l.pages.confirm_delete = None;
        } else {
            l.pages.confirm_delete = Some(std::time::Instant::now());
        }
    }
    let y = inner.y + ROW + 10.0;
    l.ui.text_input("new-driver", Rect::new(inner.x, y, half, ROW), &mut l.pages.new_driver, "New driver's name", Some("person"));
    if l.ui.button("profile-create", Rect::new(inner.x + half + GAP, y, half, ROW), "Create", Some("add"), ButtonKind::Normal) {
        let name = l.pages.new_driver.trim().to_string();
        if !name.is_empty() {
            match core::create_profile(&name, "M") {
                Ok(_) => {
                    l.state.config.profile = name.clone();
                    let _ = core::save_config(&l.state.config);
                    l.pages.new_driver.clear();
                    l.state.load_profiles();
                    l.state.set_status(format!("Driver {name} created."), false);
                }
                Err(e) => l.state.set_status(format!("{e:#}"), true),
            }
        }
    }
    // level and stats
    let card = Rect::new(left.x, top.bottom() + GAP * 1.5, left.w, left.h - top.h - GAP * 1.5);
    l.ui.panel(card);
    let Some(p) = l.state.profile.clone() else {
        l.ui.text_in("Create a driver to start a personnel file.", card.pad(20.0, 20.0), 14.0, Weight::Medium, TEXT_DIM, Align::Left);
        return;
    };
    let c = Vec2::new(card.x + 70.0, card.y + 76.0);
    let prev = ((p.level - 1) * (p.level - 1) * 250) as f64;
    let frac = ((p.xp as f64 - prev) / (p.next_level_xp as f64 - prev).max(1.0)).clamp(0.0, 1.0) as f32;
    let shown = l.ui.anim(id_of("xp-ring"), frac, 0.6);
    l.ui.p().circle(c, 50.0, Color::rgba(28, 31, 37, 1.0));
    l.ui.p().arc(c, 44.0, 52.0, 0.0, std::f32::consts::TAU, Color::WHITE.alpha(0.08));
    let a0 = -std::f32::consts::FRAC_PI_2;
    l.ui.p().arc(c, 44.0, 52.0, a0, a0 + std::f32::consts::TAU * shown.max(0.01), ACCENT);
    l.ui.text_in(&p.level.to_string(), Rect::new(c.x - 40.0, c.y - 26.0, 80.0, 40.0), 34.0, Weight::Black, TEXT, Align::Center);
    l.ui.text_in("LEVEL", Rect::new(c.x - 40.0, c.y + 12.0, 80.0, 16.0), 10.0, Weight::Black, TEXT_DIM, Align::Center);
    let tx = card.x + 142.0;
    l.ui.text_in(&format!("{}{}", p.name, if p.exists { "" } else { " (no personnel file yet)" }), Rect::new(tx, card.y + 34.0, card.w - 160.0, 30.0), 24.0, Weight::Black, TEXT, Align::Left);
    l.ui.progress(Rect::new(tx, card.y + 76.0, card.w - 170.0, 10.0), shown, false);
    l.ui.text_in(&format!("{} XP · {} to level {}", p.xp, (p.next_level_xp - p.xp).max(0), p.level + 1), Rect::new(tx, card.y + 94.0, card.w - 160.0, 18.0), 12.5, Weight::Medium, TEXT_DIM, Align::Left);
    let hours = |h: f64| format!("{} h {:02} min", h.floor() as i64, ((h - h.floor()) * 60.0).round() as i64);
    let stats = [
        ("schedule", hours(p.hours), "hours driven"),
        ("route", format!("{:.1} km", p.km), "distance"),
        ("location_on", p.stops.to_string(), "stops served"),
        ("timer", format!("{} / {}", p.early, p.late), "early / late"),
        ("confirmation_number", format!("{:.0}", p.tickets), "tickets sold"),
        ("payments", format!("{:.2}", p.cash), "takings"),
        ("warning", p.crashes.to_string(), "crashes"),
        ("person", p.hurt.to_string(), "pedestrians hurt"),
        ("speed", format!("{:.0} %", p.rating_driving), "driving"),
        ("airport_shuttle", format!("{:.0} %", p.rating_comfort), "comfort"),
        ("receipt_long", format!("{:.0} %", p.rating_tickets), "ticket selling"),
        ("history", p.sessions.len().to_string(), "runs"),
    ];
    let grid = Rect::new(card.x + 18.0, card.y + 150.0, card.w - 36.0, card.h - 170.0);
    let cols = 3;
    let cw = (grid.w - GAP * (cols as f32 - 1.0)) / cols as f32;
    let ch = 68.0;
    for (k, (icon, v, label)) in stats.iter().enumerate() {
        let (cx, cy) = ((k % cols) as f32, (k / cols) as f32);
        let r = Rect::new(grid.x + cx * (cw + GAP), grid.y + cy * (ch + 10.0), cw, ch);
        if r.bottom() > card.bottom() - 6.0 {
            break;
        }
        l.ui.p().rounded(r, 10.0, Color::WHITE.alpha(0.04));
        l.ui.icon(icon, Vec2::new(r.x + 22.0, r.y + 22.0), 18.0, ACCENT);
        l.ui.text_in(v, Rect::new(r.x + 40.0, r.y + 8.0, r.w - 48.0, 28.0), 18.0, Weight::Black, TEXT, Align::Left);
        l.ui.text_in(label, Rect::new(r.x + 14.0, r.y + 42.0, r.w - 20.0, 18.0), 11.5, Weight::Medium, TEXT_DIM, Align::Left);
    }
    // recent runs
    l.ui.panel(right);
    let inner = l.ui.heading(Rect::new(right.x + 18.0, right.y + 14.0, right.w - 36.0, right.h - 28.0), "Recent runs", Some("history"));
    let sessions = p.sessions.clone();
    l.ui.scroll_area("runs", Rect::new(inner.x - 6.0, inner.y, inner.w + 12.0, inner.h), &mut |ui, v| {
        if sessions.is_empty() {
            ui.text_in("No runs yet. Drive a duty and it shows up here.", Rect::new(v.x + 8.0, v.y, v.w, 30.0), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
            return 30.0;
        }
        let rh = 62.0;
        for (k, s) in sessions.iter().enumerate() {
            let r = Rect::new(v.x + 6.0, v.y + k as f32 * rh, v.w - 16.0, rh - 6.0);
            ui.p().rounded(r, 9.0, Color::WHITE.alpha(0.04));
            let title = match &s.line {
                Some(line) => format!("Line {line}{} · {}", s.tour.as_ref().map(|t| format!(" / {t}")).unwrap_or_default(), short_map(&s.map)),
                None => format!("Free drive · {}", short_map(&s.map)),
            };
            ui.text_in(&title, Rect::new(r.x + 12.0, r.y + 6.0, r.w - 130.0, 20.0), 13.0, Weight::Bold, TEXT, Align::Left);
            ui.text_in(&format!("{} · {:.1} km · {} stops · {} tickets · {} crashes", s.bus.rsplit('/').next().unwrap_or(""), s.metres / 1000.0, s.stops, s.tickets, s.crashes), Rect::new(r.x + 12.0, r.y + 28.0, r.w - 130.0, 18.0), 11.0, Weight::Regular, TEXT_DIM, Align::Left);
            let when = chrono_like(s.time);
            ui.text_in(&when, Rect::new(r.right() - 120.0, r.y + 6.0, 110.0, 20.0), 11.5, Weight::Medium, TEXT_SOFT, Align::Right);
            ui.text_in(&hours_short(s.seconds / 3600.0), Rect::new(r.right() - 120.0, r.y + 28.0, 110.0, 18.0), 11.5, Weight::Medium, ACCENT, Align::Right);
        }
        sessions.len() as f32 * rh
    });
}

fn hours_short(h: f64) -> String {
    format!("{}:{:02} h", h.floor() as i64, ((h - h.floor()) * 60.0).round() as i64)
}

/// A Unix time as "YYYY-MM-DD HH:MM" in the machine's time zone.
fn chrono_like(t: u64) -> String {
    #[cfg(unix)]
    {
        let tt = t as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if !unsafe { libc::localtime_r(&tt, &mut tm) }.is_null() {
            return format!("{:04}-{:02}-{:02} {:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min);
        }
    }
    let days = (t / 86400) as i64;
    let secs = t % 86400;
    // civil from days (Howard Hinnant), UTC
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", secs / 3600, (secs % 3600) / 60)
}

// --- settings ---------------------------------------------------------------------------------

/// A value of the settings as the pages show it.
fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&Value::Null)
}

fn sel_setting(ui: &mut Ui, s: &mut Value, dirty: &mut f32, name: &str, r: Rect, label: &str, key: &str, options: &[(&str, &str)]) {
    ui.label(Rect::new(r.x, r.y, r.w * 0.45, r.h), label);
    let cur = match get(s, key) {
        Value::String(x) => x.clone(),
        Value::Bool(b) => (*b as u8).to_string(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    };
    let mut labels: Vec<String> = options.iter().map(|o| o.1.to_string()).collect();
    let mut values: Vec<String> = options.iter().map(|o| o.0.to_string()).collect();
    let mut sel = values.iter().position(|v| *v == cur || v.parse::<f64>().ok().zip(cur.parse::<f64>().ok()).map(|(a, b)| (a - b).abs() < 1e-6).unwrap_or(false));
    if sel.is_none() && !cur.is_empty() {
        // a value written by hand gets an entry of its own
        labels.push(cur.clone());
        values.push(cur.clone());
        sel = Some(values.len() - 1);
    }
    let mut sel = sel.unwrap_or(0);
    if ui.select(name, Rect::new(r.x + r.w * 0.45, r.y, r.w * 0.55, r.h), &mut sel, &labels) {
        let v = &values[sel];
        s[key] = match get(s, key) {
            Value::Bool(_) => json!(v == "1"),
            Value::Number(_) => v.parse::<f64>().map(|f| if f.fract() == 0.0 { json!(f as i64) } else { json!(f) }).unwrap_or(json!(v)),
            _ => json!(v),
        };
        *dirty = 0.3;
    }
}

fn toggle_setting(ui: &mut Ui, s: &mut Value, dirty: &mut f32, r: Rect, label: &str, key: &str) {
    let mut v = get(s, key).as_bool().unwrap_or(false);
    if ui.toggle(&format!("set-{key}"), r, &mut v, label) {
        s[key] = json!(v);
        *dirty = 0.3;
    }
}

/// The settings' columns are this tall at least (the page scrolls when the window is lower).
const SETTINGS_H: f32 = 1080.0;

/// The settings page's "Updates" row: what the updater is doing, and whether "Check now"
/// was pressed.
struct UpdateRow {
    status: crate::updater::Status,
    check: bool,
}

pub fn settings(l: &mut Launcher, area: Rect) {
    let body = l.page_title(area, "Settings", "Every change is saved at once; the game reads them when it starts.");
    let s = &mut l.state.settings;
    let dirty = &mut l.state.settings_dirty;
    let mut upd = UpdateRow { status: l.update.status(), check: false };
    // (a window lower than the columns scrolls them; they were cut off at the bottom)
    // (three columns side by side; where they would be too narrow to read - a phone - one
    // under the other, each as high as it was the frame before)
    let stacked = body.w < 900.0;
    l.ui.scroll_area("settings-page", body, &mut |ui, v| {
        let hs = SETTINGS_COL_H.with(|c| c.get());
        let h = if stacked { hs.iter().sum::<f32>() + GAP * 4.0 } else { hs.iter().fold(v.h, |a, b| a.max(*b)) };
        let used = settings_columns(ui, s, dirty, Rect::new(v.x, v.y, v.w - 8.0, h), &mut upd, stacked.then_some(hs));
        SETTINGS_COL_H.with(|c| c.set(used));
        h
    });
    if upd.check {
        l.update.check();
    }
    if RESET_ASKED.with(|c| c.replace(false)) {
        l.pages.confirm_reset = true;
    }
}

thread_local! {
    /// "Reset all settings" was pressed (in the columns, which do not see the launcher).
    static RESET_ASKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The dialog that asks before every setting goes back to how it came.
pub fn reset_dialog(l: &mut Launcher) {
    let size = l.ui.size;
    let full = Rect::new(0.0, 0.0, size.x, size.y);
    l.ui.solid(full);
    l.ui.p().rect(full, omsi_ui::Color::rgba(0, 0, 0, 0.62));
    let w = (size.x - 48.0).min(520.0);
    let h = 190.0;
    let r = Rect::new((size.x - w) * 0.5, (size.y - h) * 0.5, w, h);
    l.ui.panel(r);
    let inner = Rect::new(r.x + 24.0, r.y + 20.0, r.w - 48.0, r.h - 40.0);
    l.ui.icon("restart_alt", Vec2::new(inner.x + 14.0, inner.y + 14.0), 26.0, DANGER);
    l.ui.text_in("Reset every setting?", Rect::new(inner.x + 38.0, inner.y, inner.w - 38.0, 28.0), 18.0, Weight::Bold, TEXT, Align::Left);
    l.ui.paragraph("Graphics, sound, controllers and game settings go back to how they came. The language, the drivers, the key bindings and the game folder stay.", Vec2::new(inner.x, inner.y + 40.0), inner.w, 13.0, Weight::Regular, TEXT_DIM);
    let by = inner.bottom() - 38.0;
    if l.ui.button("reset-no", Rect::new(inner.right() - 250.0, by, 110.0, 38.0), "Cancel", None, ButtonKind::Normal) {
        l.pages.confirm_reset = false;
    }
    if l.ui.button("reset-yes", Rect::new(inner.right() - 130.0, by, 130.0, 38.0), "Reset", Some("restart_alt"), ButtonKind::Danger) {
        let language = l.state.settings.get("language").cloned();
        l.state.settings = core::settings_from_text(None);
        if let Some(lang) = language {
            l.state.settings["language"] = lang;
        }
        l.state.settings_dirty = 0.3;
        l.pages.confirm_reset = false;
        l.state.set_status("Every setting is back to how it came.", false);
    }
}

thread_local! {
    /// How high each settings column came out (the stacked layout's heights).
    static SETTINGS_COL_H: std::cell::Cell<[f32; 3]> = const { std::cell::Cell::new([SETTINGS_H; 3]) };
}

/// The settings' three columns in `body`: side by side, or with `stacked` (each one's height)
/// one under the other. Returns the height each needed.
fn settings_columns(ui: &mut Ui, s: &mut Value, dirty: &mut f32, body: Rect, upd: &mut UpdateRow, stacked: Option<[f32; 3]>) -> [f32; 3] {
    let cols = 3;
    let cw = (body.w - GAP * 2.0 * (cols as f32 - 1.0)) / cols as f32;
    let colr = |k: usize| match stacked {
        Some(hs) => Rect::new(body.x, body.y + hs[..k].iter().sum::<f32>() + k as f32 * GAP * 2.0, body.w, hs[k]),
        None => Rect::new(body.x + k as f32 * (cw + GAP * 2.0), body.y, cw, body.h),
    };
    let mut used = [0.0f32; 3];
    // graphics
    let c0 = colr(0);
    ui.panel(c0);
    let inner = ui.heading(Rect::new(c0.x + 18.0, c0.y + 14.0, c0.w - 36.0, c0.h - 28.0), "Graphics", Some("display_settings"));
    let mut y = inner.y;
    let row = |y: &mut f32| {
        let r = Rect::new(inner.x, *y, inner.w, ROW - 2.0);
        *y += ROW + 4.0;
        r
    };
    if ui.button("s-reset", row(&mut y), "Reset all settings...", Some("restart_alt"), ButtonKind::Danger) {
        RESET_ASKED.with(|c| c.set(true));
    }
    sel_setting(ui, s, dirty, "s-graphics", row(&mut y), "Graphics", "graphics", &[("vanilla", "Vanilla (as OMSI 2)"), ("vanilla_plus", "Vanilla+"), ("enhanced", "Enhanced")]);
    // Vanilla draws what OMSI 2 draws: no sun shadows, ambient occlusion or detail grain
    let classic = get(s, "graphics").as_str() == Some("vanilla");
    sel_setting(ui, s, dirty, "s-msaa", row(&mut y), "Anti-aliasing", "msaa", &[("1", "Off"), ("2", "2x MSAA"), ("4", "4x MSAA")]);
    sel_setting(ui, s, dirty, "s-scale", row(&mut y), "Render scale", "render_scale", &[("auto", "Auto"), ("1", "100%"), ("0.85", "85%"), ("0.75", "75%"), ("0.67", "67%"), ("0.5", "50%")]);
    sel_setting(ui, s, dirty, "s-af", row(&mut y), "Anisotropic", "anisotropy", &[("1", "Off"), ("2", "2x"), ("4", "4x"), ("8", "8x")]);
    if !classic {
        sel_setting(ui, s, dirty, "s-shadow", row(&mut y), "Shadow map", "shadow_size", &[("1024", "1024"), ("2048", "2048"), ("4096", "4096")]);
        toggle_setting(ui, s, dirty, row(&mut y), "Ambient occlusion", "ssao");
        toggle_setting(ui, s, dirty, row(&mut y), "Sun shadows", "shadows");
        sel_setting(ui, s, dirty, "s-casters", row(&mut y), "Shadows cast by", "shadow_casters", &[("all", "Every solid mesh"), ("omsi", "[shadow] meshes, as OMSI")]);
        toggle_setting(ui, s, dirty, row(&mut y), "Detail texturing up close", "detail_textures");
        // (an LED panel's dots are its own light: how bright they burn, and whether their
        // mask keeps the mip chain `STFilter` asks for - off keeps them dots when the panel
        // is small, at the cost of the shimmer the chain exists to prevent)
        let mut led = get(s, "led_glow").as_i64().unwrap_or(6) as f32;
        if ui.slider("s-led", row(&mut y), &mut led, 0.0, 15.0, 1.0, "LED glow", &|v| if v < 0.5 { "Off".to_string() } else { format!("{}", v as i64) }) {
            s["led_glow"] = json!(led.round() as i64);
            *dirty = 0.3;
        }
        toggle_setting(ui, s, dirty, row(&mut y), "LED masks keep their mipmaps", "led_mips");
    }
    toggle_setting(ui, s, dirty, row(&mut y), "Reflection maps (paint, chrome, glass)", "reflections");
    // (a Mac has Metal only; elsewhere a driver's Vulkan that misbehaves, or a card without
    // it, is got round here)
    if cfg!(windows) {
        sel_setting(ui, s, dirty, "s-api", row(&mut y), "Graphics API", "graphics_api", &[("auto", "Automatic"), ("vulkan", "Vulkan"), ("dx12", "DirectX 12"), ("gl", "OpenGL")]);
    } else if !cfg!(target_os = "macos") {
        sel_setting(ui, s, dirty, "s-api", row(&mut y), "Graphics API", "graphics_api", &[("auto", "Automatic"), ("vulkan", "Vulkan"), ("gl", "OpenGL")]);
    }
    toggle_setting(ui, s, dirty, row(&mut y), "Clouds", "clouds");
    toggle_setting(ui, s, dirty, row(&mut y), "Fullscreen", "fullscreen");
    toggle_setting(ui, s, dirty, row(&mut y), "V-sync", "vsync");
    y += 6.0;
    ui.heading(Rect::new(inner.x, y, inner.w, 28.0), "World & memory", Some("public"));
    y += 32.0;
    sel_setting(ui, s, dirty, "s-view", row(&mut y), "View distance", "view_distance", &[("auto", "Default (1200 m)"), ("600", "600 m - fastest"), ("900", "900 m"), ("1200", "1200 m"), ("1500", "1500 m"), ("2000", "2000 m"), ("2500", "2500 m")]);
    // (the game takes the smaller of an eighth of the memory and what the graphics
    // adapter is taken to hold, see `memory::texture_budget`)
    let adapter_mb = omsi_render::ADAPTER_TEXTURE_MB.load(std::sync::atomic::Ordering::Relaxed) as i64;
    let auto_mb = match (get(s, "texture_memory_auto").as_i64().unwrap_or(0), adapter_mb) {
        (m, 0) => m,
        (0, a) => a,
        (m, a) => m.min(a),
    };
    let auto_label = if auto_mb > 0 { format!("Automatic ({} here)", mb(auto_mb)) } else { "Automatic".to_string() };
    let opts: Vec<(&str, &str)> = vec![("0", auto_label.as_str()), ("500", "500 MB"), ("1000", "1 GB"), ("1500", "1.5 GB"), ("2000", "2 GB"), ("3000", "3 GB"), ("4000", "4 GB"), ("6000", "6 GB")];
    sel_setting(ui, s, dirty, "s-texmem", row(&mut y), "Texture memory", "texture_memory", &opts);
    toggle_setting(ui, s, dirty, row(&mut y), "Compress textures on loading", "texture_compression");
    // OMSI's own options (options.cfg)
    y += 6.0;
    ui.heading(Rect::new(inner.x, y, inner.w, 28.0), "Simulation", Some("tune"));
    y += 32.0;
    sel_setting(ui, s, dirty, "s-maint", row(&mut y), "Maintenance", "maintenance", &[("0", "Infinite (no wear)"), ("1", "Very bad"), ("2", "Bad"), ("3", "Normal"), ("4", "Good")]);
    sel_setting(ui, s, dirty, "s-unsched", row(&mut y), "Random traffic", "ai_unsched_factor", &[("25", "25%"), ("50", "50%"), ("75", "75%"), ("100", "100%"), ("150", "150%"), ("200", "200%")]);
    used[0] = y - c0.y + 14.0;
    // navigator & interface
    let c1 = colr(1);
    ui.panel(c1);
    let inner = ui.heading(Rect::new(c1.x + 18.0, c1.y + 14.0, c1.w - 36.0, c1.h - 28.0), "Navigator", Some("near_me"));
    let mut y = inner.y;
    let row = |y: &mut f32| {
        let r = Rect::new(inner.x, *y, inner.w, ROW - 2.0);
        *y += ROW + 4.0;
        r
    };
    toggle_setting(ui, s, dirty, row(&mut y), "Navigator (Shift+N: map, schedule, off)", "navigator");
    toggle_setting(ui, s, dirty, row(&mut y), "Route arrows (as in OMSI 2)", "nav_arrows");
    let mut op = get(s, "navigator_opacity").as_f64().unwrap_or(0.85) as f32;
    if ui.slider("s-navop", row(&mut y), &mut op, 0.2, 1.0, 0.05, "Opacity", &|v| format!("{:.0}%", v * 100.0)) {
        s["navigator_opacity"] = json!((op * 100.0).round() / 100.0);
        *dirty = 0.3;
    }
    // the corner: a little screen with four corners to click
    let r = Rect::new(inner.x, y, inner.w, 70.0);
    ui.label(Rect::new(r.x, r.y, r.w * 0.45, 24.0), "Corner");
    let screen = Rect::new(r.x + r.w * 0.45, r.y, 110.0, 64.0);
    ui.p().rounded(screen, 6.0, Color::WHITE.alpha(0.05));
    ui.p().rounded_border(screen, 6.0, 1.0, Color::WHITE.alpha(0.12));
    let cur = get(s, "navigator_corner").as_str().unwrap_or("bottom-left").to_string();
    for (name, x, yy) in [("top-left", 0.0, 0.0), ("top-right", 1.0, 0.0), ("bottom-left", 0.0, 1.0), ("bottom-right", 1.0, 1.0)] {
        let cell = Rect::new(screen.x + 5.0 + x * (screen.w * 0.5), screen.y + 5.0 + yy * (screen.h * 0.5), screen.w * 0.5 - 10.0, screen.h * 0.5 - 10.0);
        let (h, _, clicked) = ui.interact(id_of(&format!("corner-{name}")), cell);
        if clicked {
            s["navigator_corner"] = json!(name);
            *dirty = 0.3;
        }
        let on = cur == name;
        ui.p().rounded(cell, 3.0, if on { ACCENT } else { Color::WHITE.alpha(if h { 0.2 } else { 0.08 }) });
    }
    y += 74.0;
    ui.heading(Rect::new(inner.x, y, inner.w, 28.0), "Interface & online", Some("forum"));
    y += 32.0;
    {
        let langs: Vec<(&str, &str)> = core::LANGUAGES.iter().map(|l| (l.0, l.1)).collect();
        sel_setting(ui, s, dirty, "s-lang", row(&mut y), "Language", "language", &langs);
    }
    // (the launcher speaks the chosen language at once)
    crate::ui_language(get(s, "language").as_str().unwrap_or("ENG"));
    // (texts nobody has translated: translated on this machine, see `mt`)
    let was = get(s, "machine_translation").as_bool().unwrap_or(false);
    toggle_setting(ui, s, dirty, row(&mut y), "Translate the remaining texts automatically (offline, downloads 620 MB once)", "machine_translation");
    let now = get(s, "machine_translation").as_bool().unwrap_or(false);
    if now != was {
        crate::mt::enable(now);
    }
    let st = crate::mt::status();
    if now && !st.is_empty() && st != "Ready" {
        ui.text_in(&st, Rect::new(inner.x + 12.0, y - 6.0, inner.w - 24.0, 16.0), 11.5, omsi_ui::Weight::Regular, TEXT_FAINT, omsi_ui::paint::Align::Left);
        y += 14.0;
    }
    toggle_setting(ui, s, dirty, row(&mut y), "Chat in online games", "chat");
    toggle_setting(ui, s, dirty, row(&mut y), "Name of the button under the mouse", "tooltips");
    toggle_setting(ui, s, dirty, row(&mut y), "Other players' names above their buses", "name_tags");
    toggle_setting(ui, s, dirty, row(&mut y), "Driver at the wheel (outside views)", "driver");
    toggle_setting(ui, s, dirty, row(&mut y), "Frame rate in the corner", "show_fps");
    y += 6.0;
    ui.heading(Rect::new(inner.x, y, inner.w, 28.0), "Performance", Some("speed"));
    y += 32.0;
    // Quality presets. (OMSI's own option_presets/*.oop are named after the PCs of their
    // day - "PC 2006", "X10 high", "Chicago Recommended" - which read as random words here.)
    let presets: [(&str, serde_json::Value); 4] = [
        ("Low", json!({"msaa": 1, "anisotropy": 2, "shadow_size": 1024, "ssao": false, "shadows": false, "detail_textures": false, "clouds": false, "view_distance": "600", "min_obj_size": 0.03, "max_obj_dist": "500", "mirror_size": 128, "render_scale": "0.75", "texture_memory": 800})),
        ("Medium", json!({"msaa": 2, "anisotropy": 4, "shadow_size": 2048, "ssao": false, "shadows": true, "detail_textures": true, "clouds": true, "view_distance": "900", "min_obj_size": 0.02, "max_obj_dist": "750", "mirror_size": 256, "render_scale": "auto", "texture_memory": 1200})),
        ("High", json!({"msaa": 4, "anisotropy": 8, "shadow_size": 2048, "ssao": true, "shadows": true, "detail_textures": true, "clouds": true, "view_distance": "auto", "min_obj_size": 0.013, "max_obj_dist": "auto", "mirror_size": 256, "render_scale": "auto", "texture_memory": 0})),
        ("Ultra", json!({"msaa": 4, "anisotropy": 8, "shadow_size": 4096, "ssao": true, "shadows": true, "detail_textures": true, "clouds": true, "view_distance": "2000", "min_obj_size": 0.005, "max_obj_dist": "1500", "mirror_size": 512, "render_scale": "auto", "texture_memory": 0})),
    ];
    {
        // the preset the settings match now, else "Custom"
        let matches = |p: &serde_json::Value| p.as_object().map(|o| o.iter().all(|(k, v)| {
            let cur = get(s, k);
            cur == v || cur.as_f64().zip(v.as_f64()).map(|(a, b)| (a - b).abs() < 1e-6).unwrap_or(false) || cur.as_str().zip(v.as_f64()).map(|(a, b)| a.parse::<f64>().map(|a| (a - b).abs() < 1e-6).unwrap_or(false)).unwrap_or(false) || cur.as_f64().zip(v.as_str()).map(|(a, b)| b.parse::<f64>().map(|b| (a - b).abs() < 1e-6).unwrap_or(false)).unwrap_or(false)
        })).unwrap_or(false);
        let mut labels: Vec<String> = presets.iter().map(|p| p.0.to_string()).collect();
        labels.push("Custom".to_string());
        let mut sel = presets.iter().position(|p| matches(&p.1)).unwrap_or(presets.len());
        ui.label(Rect::new(inner.x, y, inner.w * 0.45, ROW - 2.0), "Quality preset");
        if ui.select("s-preset", Rect::new(inner.x + inner.w * 0.45, y, inner.w * 0.55, ROW - 2.0), &mut sel, &labels) && sel < presets.len() {
            if let Some(obj) = presets[sel].1.as_object() {
                for (k, v) in obj {
                    s[k.as_str()] = v.clone();
                }
                *dirty = 0.3;
            }
        }
        y += ROW + 4.0;
    }
    sel_setting(ui, s, dirty, "s-fps", row(&mut y), "Frame limit", "max_fps", &[("0", "Screen refresh rate"), ("30", "30 fps"), ("45", "45 fps"), ("60", "60 fps"), ("120", "120 fps"), ("144", "144 fps"), ("1000", "Unlimited")]);
    sel_setting(ui, s, dirty, "s-minobj", row(&mut y), "Small objects", "min_obj_size", &[("0.005", "All"), ("0.013", "Normal"), ("0.02", "Fewer (faster)"), ("0.03", "Few (fastest)")]);
    sel_setting(ui, s, dirty, "s-maxobj", row(&mut y), "Object distance", "max_obj_dist", &[("auto", "Automatic"), ("500", "500 m"), ("750", "750 m"), ("900", "900 m"), ("1500", "1500 m"), ("3000", "3000 m")]);
    sel_setting(ui, s, dirty, "s-mirror", row(&mut y), "Mirrors", "mirror_size", &[("128", "Low (128)"), ("256", "Normal (256)"), ("512", "High (512)"), ("1024", "Very high (1024)")]);
    if cfg!(windows) {
        y += 6.0;
        ui.heading(Rect::new(inner.x, y, inner.w, 28.0), "Virtual reality", Some("view_in_ar"));
        y += 32.0;
        toggle_setting(ui, s, dirty, row(&mut y), "Use OpenXR headset", "vr");
        if get(s, "vr").as_bool().unwrap_or(false) {
            sel_setting(ui, s, dirty, "s-vr-scale", row(&mut y), "Eye resolution", "vr_scale", &[("0.5", "50%"), ("0.65", "65%"), ("0.8", "80%"), ("1", "100%")]);
            sel_setting(ui, s, dirty, "s-vr-head-smoothing", row(&mut y), "Head tracking smoothing", "vr_head_smoothing_ms", &[("0", "Off"), ("5", "5 ms"), ("10", "10 ms"), ("20", "20 ms"), ("30", "30 ms")]);
            sel_setting(ui, s, dirty, "s-vr-mirror-rate", row(&mut y), "Bus mirror refresh", "vr_mirror_rate", &[("0", "Off"), ("8", "8/s"), ("16", "16/s"), ("24", "24/s"), ("32", "32/s")]);
            toggle_setting(ui, s, dirty, row(&mut y), "Show headset picture on monitor", "vr_desktop_mirror");
            ui.text_in("VR keys: Controls → Keyboard (search VR)", row(&mut y),
                11.0, omsi_ui::Weight::Regular, TEXT_DIM, omsi_ui::paint::Align::Left);
        }
    }
    // updates from the GitHub releases (see `crate::updater`)
    y += 6.0;
    ui.heading(Rect::new(inner.x, y, inner.w, 28.0), "Updates", Some("system_update"));
    y += 32.0;
    toggle_setting(ui, s, dirty, row(&mut y), "Look for updates when the launcher starts", "update_check");
    toggle_setting(ui, s, dirty, row(&mut y), "Install updates without asking", "update_auto");
    {
        use crate::updater::Status;
        let r = row(&mut y);
        let busy = matches!(upd.status, Status::Checking | Status::Downloading { .. } | Status::Installing(_) | Status::WaitingForInstaller(_) | Status::Restarting(_));
        if ui.button("s-upd-check", Rect::new(r.x, r.y, 150.0, r.h), if busy { "Checking…" } else { "Check now" }, Some("refresh"), ButtonKind::Normal) && !busy {
            upd.check = true;
        }
        let text = match &upd.status {
            Status::UpToDate => format!("{} is the latest version", crate::updater::current_version()),
            Status::Available(rel) => format!("{} is available", rel.version),
            Status::Failed(_) => "The last check failed".to_string(),
            _ => format!("This is openOMSI {}", crate::updater::current_version()),
        };
        ui.text_in(&text, Rect::new(r.x + 162.0, r.y, r.w - 162.0, r.h), 12.5, omsi_ui::Weight::Regular, TEXT_DIM, omsi_ui::paint::Align::Left);
    }
    {
        let r = row(&mut y);
        if ui.button("s-upd-github", Rect::new(r.x, r.y, r.w, r.h), "github.com/turbo-devv/openOMSI", Some("open_in_new"), ButtonKind::Ghost) {
            crate::updater::open_url(crate::updater::REPO_URL);
        }
    }
    used[1] = y - c1.y + 14.0;
    // passengers, controls, sound
    let c2 = colr(2);
    ui.panel(c2);
    let inner = ui.heading(Rect::new(c2.x + 18.0, c2.y + 14.0, c2.w - 36.0, c2.h - 28.0), "Passengers", Some("groups"));
    let mut y = inner.y;
    let row = |y: &mut f32| {
        let r = Rect::new(inner.x, *y, inner.w, ROW - 2.0);
        // (a little tighter than the other columns: this one holds the most)
        *y += ROW + 1.0;
        r
    };
    sel_setting(ui, s, dirty, "s-board", row(&mut y), "Boarding", "boarding", &[("auto", "Pay and take the ticket"), ("pay", "The driver sells the ticket"), ("walk", "Just walk in")]);
    toggle_setting(ui, s, dirty, row(&mut y), "Passengers pay the exact fare", "exact_fare");
    sel_setting(ui, s, dirty, "s-voices", row(&mut y), "Passenger voices", "pax_voices", &[("all", "Greetings and tickets"), ("tickets", "Only the ticket asked for"), ("off", "Silent")]);
    toggle_setting(ui, s, dirty, row(&mut y), "Ability to get up (Ctrl+Shift+G)", "get_up");
    let mut pd = get(s, "pax_density").as_f64().unwrap_or(1.0) as f32;
    if ui.slider("s-pax", row(&mut y), &mut pd, 0.0, 2.0, 0.1, "How many passengers", &|v| format!("{:.0}%", v * 100.0)) {
        s["pax_density"] = json!((pd * 100.0).round() / 100.0);
        *dirty = 0.3;
    }
    y += 6.0;
    ui.heading(Rect::new(inner.x, y, inner.w, 28.0), "Controls & sound", Some("volume_up"));
    y += 32.0;
    sel_setting(ui, s, dirty, "s-keys", row(&mut y), "Driving keys", "drive_keys", &[("omsi", "Custom controls (Controls page)"), ("simple", "W A S D + arrows"), ("wasd", "W A S D only"), ("arrows", "Arrow keys only")]);
    let mut vol = get(s, "volume").as_f64().unwrap_or(0.6) as f32;
    if ui.slider("s-vol", row(&mut y), &mut vol, 0.0, 1.0, 0.05, "Volume", &|v| format!("{:.0}%", v * 100.0)) {
        s["volume"] = json!((vol * 100.0).round() / 100.0);
        *dirty = 0.3;
    }
    toggle_setting(ui, s, dirty, row(&mut y), "Steering linearity (keys at OMSI's steady pace)", "steering_linear");
    toggle_setting(ui, s, dirty, row(&mut y), "Old Steering (the wheel stays, turn it back yourself)", "old_steering");
    let mut ms = get(s, "mouse_sens").as_f64().unwrap_or(1.0) as f32;
    if ui.slider("s-mouse", row(&mut y), &mut ms, 0.25, 2.0, 0.05, "Mouse steering (O)", &|v| if (v - 1.0).abs() < 0.01 { "OMSI".to_string() } else { format!("{:.0}%", v * 100.0) }) {
        s["mouse_sens"] = json!((ms * 100.0).round() / 100.0);
        *dirty = 0.3;
    }
    // steering wheels: the wheel's own rotation and how much of it is the bus's full lock
    // (a real bus: about two and a half turns), force feedback the other way round
    let mut range = get(s, "wheel_range").as_f64().unwrap_or(900.0) as f32;
    if ui.slider("s-wrange", row(&mut y), &mut range, 180.0, 1800.0, 30.0, "Wheel rotation", &|v| format!("{v:.0}°")) {
        s["wheel_range"] = json!(range.round());
        *dirty = 0.3;
    }
    let mut lock = get(s, "wheel_lock").as_f64().unwrap_or(0.0) as f32;
    if ui.slider("s-wlock", row(&mut y), &mut lock, 0.0, 1800.0, 30.0, "Full lock at", &|v| if v < 45.0 { "OMSI".to_string() } else { format!("{v:.0}°") }) {
        s["wheel_lock"] = json!(if lock < 45.0 { 0.0 } else { lock.round() });
        *dirty = 0.3;
    }
    toggle_setting(ui, s, dirty, row(&mut y), "Force feedback and vibration", "ff_enabled");
    toggle_setting(ui, s, dirty, row(&mut y), "The keyboard brake stays on until the throttle (as in OMSI)", "brake_hold");
    toggle_setting(ui, s, dirty, row(&mut y), "Automatic clutch (manual gearboxes)", "auto_clutch");
    toggle_setting(ui, s, dirty, row(&mut y), "Invert force feedback", "ff_invert");
    if ui.button("s-wreset", row(&mut y), "Reset wheel settings", Some("restart_alt"), ButtonKind::Normal) {
        s["wheel_range"] = json!(900.0);
        s["wheel_lock"] = json!(0.0);
        s["ff_invert"] = json!(false);
        s["ff_enabled"] = json!(true);
        *dirty = 0.3;
    }
    // the pedals' response: softer (below 1) or stronger (above 1) than the pedal reads
    for (key, label, id) in [("pedal_throttle", "Throttle pedal strength", "s-pedt"), ("pedal_brake", "Brake pedal strength", "s-pedb")] {
        let mut v = get(s, key).as_f64().unwrap_or(1.0) as f32;
        if ui.slider(id, row(&mut y), &mut v, 0.5, 2.0, 0.05, label, &|v| if (v - 1.0).abs() < 0.01 { "Normal".to_string() } else if v < 1.0 { format!("Softer x{v:.2}") } else { format!("Stronger x{v:.2}") }) {
            s[key] = json!((v * 100.0).round() / 100.0);
            *dirty = 0.3;
        }
    }
    // the driver's eye, moved from the bus's own camera
    for (key, label, id) in [("seat_y", "Seat forward / back", "s-seaty"), ("seat_z", "Seat up / down", "s-seatz"), ("seat_x", "Seat right / left", "s-seatx")] {
        let mut v = get(s, key).as_f64().unwrap_or(0.0) as f32;
        if ui.slider(id, row(&mut y), &mut v, -0.6, 0.6, 0.01, label, &|v| format!("{:+.0} cm", v * 100.0)) {
            s[key] = json!((v * 100.0).round() / 100.0);
            *dirty = 0.3;
        }
    }
    if ui.button("s-seatreset", row(&mut y), "Reset the seat position", Some("restart_alt"), ButtonKind::Normal) {
        for k in ["seat_x", "seat_y", "seat_z"] {
            s[k] = json!(0.0);
        }
        *dirty = 0.3;
    }
    toggle_setting(ui, s, dirty, row(&mut y), "Camera collisions (outside view)", "camera_collision");
    toggle_setting(ui, s, dirty, row(&mut y), "Driver's view turns with the steering", "steer_look");
    toggle_setting(ui, s, dirty, row(&mut y), "Head tracking (TrackIR and others through opentrack, UDP 4242)", "head_tracking");
    let mut fov = get(s, "fov").as_f64().unwrap_or(0.0) as f32;
    if ui.slider("s-fov", row(&mut y), &mut fov, 0.0, 120.0, 1.0, "Field of view", &|v| if v < 20.0 { "Default".to_string() } else { format!("{v:.0}°") }) {
        s["fov"] = json!(if fov < 20.0 { 0.0 } else { fov.round() });
        *dirty = 0.3;
    }
    toggle_setting(ui, s, dirty, row(&mut y), "Doppler effect", "doppler");
    for (key, label, id) in [("vol_ai", "Traffic", "s-volai"), ("vol_scenery", "Surroundings", "s-volsc")] {
        let mut v = get(s, key).as_f64().unwrap_or(1.0) as f32;
        if ui.slider(id, row(&mut y), &mut v, 0.0, 1.0, 0.05, label, &|v| format!("{:.0}%", v * 100.0)) {
            s[key] = json!((v * 100.0).round() / 100.0);
            *dirty = 0.3;
        }
    }
    y += 8.0;
    ui.heading(Rect::new(inner.x, y, inner.w, 28.0), "Simulation", Some("tune"));
    y += 32.0;
    sel_setting(ui, s, dirty, "s-maxsched", row(&mut y), "Timetable vehicles", "ai_max_scheduled", &[("0", "All"), ("10", "At most 10"), ("25", "At most 25"), ("50", "At most 50")]);
    sel_setting(ui, s, dirty, "s-maxpark", row(&mut y), "Parked cars", "ai_max_parked", &[("0", "Every space"), ("35", "At most 35"), ("100", "At most 100"), ("250", "At most 250")]);
    toggle_setting(ui, s, dirty, row(&mut y), "Start at the real time", "use_real_time");
    toggle_setting(ui, s, dirty, row(&mut y), "Start on today's date", "use_real_date");
    toggle_setting(ui, s, dirty, row(&mut y), "Collisions with vehicles", "collision_vehicles");
    toggle_setting(ui, s, dirty, row(&mut y), "Collisions with objects (walls, poles)", "collision_objects");
    toggle_setting(ui, s, dirty, row(&mut y), "Collisions with people", "collision_pedestrians");
    toggle_setting(ui, s, dirty, row(&mut y), "Head moves with the bus", "head_movement");
    toggle_setting(ui, s, dirty, row(&mut y), "Camera glides between viewpoints", "driverview_smooth");
    toggle_setting(ui, s, dirty, row(&mut y), "Driver's hands in the cab view", "hands_in_cab");
    // (in multiplayer the host's or the server's speed counts)
    sel_setting(ui, s, dirty, "s-timespeed", row(&mut y), "Time speed (not in multiplayer)", "time_speed", &[("1", "Real time"), ("2", "x2"), ("4", "x4"), ("8", "x8"), ("15", "x15"), ("30", "x30")]);
    // (the keys are on the Controls page)
    used[2] = y - c2.y + 14.0;
    used
}

fn mb(v: i64) -> String {
    if v >= 1000 {
        format!("{:.1} GB", v as f64 / 1000.0)
    } else {
        format!("{v} MB")
    }
}

// --- controls ---------------------------------------------------------------------------------

fn action_label(a: &str) -> String {
    if let Some(gear) = a.strip_prefix("kw_s_").and_then(|s| s.strip_suffix("_fest")) {
        return format!("Gear {gear} (H-pattern)");
    }
    let known: &[(&str, &str)] = &[
        ("throttle", "Throttle"),
        ("brake", "Brake"),
        ("throttle_amplify", "Throttle (full, kickdown)"),
        ("clutch", "Clutch"),
        ("steering_left", "Steer left"),
        ("steering_right", "Steer right"),
        ("steering_neutral", "Steering to centre"),
        ("parking_brake_toggle", "Parking brake"),
        ("blinker_left_set", "Indicator left"),
        ("blinker_right_set", "Indicator right"),
        ("blinker_off", "Indicators off"),
        ("blinker_warn_toggle", "Hazard lights"),
        ("horn", "Horn"),
        ("kw_scheinwerfer_toggle", "Headlights"),
        ("kw_standlicht_toggle", "Sidelights"),
        ("kw_fernlicht_toggle", "High beam"),
        ("kw_m_enginestart", "Starter"),
        ("kw_wipermode_up", "Wipers (next mode)"),
        ("cp_batterietrennschalter_toggle", "Battery / ignition"),
        ("automatic_D", "Gear D"),
        ("automatic_N", "Gear N"),
        ("automatic_R", "Gear R"),
        ("bus_doorfront0", "Front door (leaf 1)"),
        ("bus_doorfront1", "Front door (leaf 2)"),
        ("bus_dooraft", "Release rear doors"),
        ("ticket_give", "Sell the requested ticket"),
        ("view_set_driver", "Driver's view"),
        ("view_set_passenger", "Passenger view"),
        ("view_set_outside", "Outside view"),
        ("view_toggle_viewpoint", "Next view"),
        ("vr_recenter", "VR: Reset view"),
        ("vr_toggle_desktop_mirror", "VR: Monitor preview"),
        ("vr_toggle_mode", "VR: Switch VR / desktop"),
        ("exit", "Quit"),
        ("sim_pause", "Pause"),
        ("screenshot", "Screenshot"),
        ("quicksave", "Quicksave"),
        ("toggel_mouse_ctrl", "Toggle mouse steering"),
        ("toggel_ctrler", "Toggle game controllers"),
    ];
    known.iter().find(|k| k.0 == a).map(|k| k.1.to_string()).unwrap_or_else(|| a.trim_start_matches("kw_").trim_start_matches("cp_").trim_start_matches("bus_").replace('_', " "))
}

fn key_name(scan: i64, modifier: i64) -> String {
    if scan == 0 {
        return "(unbound)".into();
    }
    let k = crate::keys::scan_name(scan as i32).unwrap_or_else(|| format!("scan {scan}"));
    let mut mods = Vec::new();
    if modifier & omsi_content::input::KEY_SHIFT as i64 != 0 {
        mods.push("Shift");
    }
    if modifier & omsi_content::input::KEY_CTRL as i64 != 0 {
        mods.push("Ctrl");
    }
    if modifier & omsi_content::input::KEY_ALT as i64 != 0 {
        mods.push("Alt");
    }
    if mods.is_empty() {
        k
    } else {
        format!("{}+{k}", mods.join("+"))
    }
}

pub fn controls(l: &mut Launcher, area: Rect) {
    let body = l.page_title(area, "Controls", if l.pages.controls_tab == 0 { "Click a key and press the new one (hold Shift, Ctrl or Alt for a combination); Escape leaves it as it is." } else { "What each axis and button of a wheel, pedals or joystick does - OMSI 2's gamectrler.cfg, kept in the content folder." });
    let mut tab = l.pages.controls_tab;
    if l.ui.segmented("controls-tab", Rect::new(body.right() - 320.0, body.y - 46.0, 320.0, 34.0), &mut tab, &["Keyboard", "Game controllers"]) {
        l.pages.controls_tab = tab;
    }
    if l.pages.controls_tab == 1 {
        game_controllers(l, body);
        return;
    }
    // a key pressed while one binding waits for it
    if let (Some((sec, idx)), Some(code)) = (l.pages.capturing, l.ui.input.raw_key) {
        use winit::keyboard::KeyCode as K;
        if code == K::Escape {
            l.pages.capturing = None;
        } else if !matches!(code, K::ShiftLeft | K::ShiftRight | K::ControlLeft | K::ControlRight | K::AltLeft | K::AltRight | K::SuperLeft | K::SuperRight) {
            match crate::keys::dik_code(code) {
                Some(scan) => {
                    let m = omsi_content::input::chord(l.ui.input.shift, l.ui.input.ctrl, l.ui.input.alt) as i64;
                    let section = ["vehicles", "game"][sec];
                    let vr_binding = l.state.keybindings.get(section).and_then(|a| a.as_array())
                        .and_then(|a| a.get(idx)).and_then(|b| b.get("action"))
                        .and_then(|a| a.as_str()).is_some_and(|a| a.starts_with("vr_"));
                    if let Some(b) = l.state.keybindings.get_mut(section).and_then(|a| a.as_array_mut()).and_then(|a| a.get_mut(idx)) {
                        // (the entry's "held" bit is the action's, not the key's: it stays)
                        let hold = b.get("modifier").and_then(|x| x.as_i64()).unwrap_or(0) & omsi_content::input::KEY_HOLD as i64;
                        b["scan_code"] = json!(scan);
                        b["modifier"] = json!(m | hold);
                    }
                    save_keys(l, vr_binding);
                }
                None => l.state.set_status(format!("{code:?} has no DirectInput scan code the game understands."), true),
            }
            l.pages.capturing = None;
        }
        l.ui.input.raw_key = None;
    }
    // The keys below are the ones the game uses only with "Custom controls" (Settings →
    // Driving keys); the ready-made layouts keep W A S D / the arrows for driving. Say so,
    // with the switch right here - and changing a key switches by itself (see `save_keys`).
    let preset = l.state.settings.get("drive_keys").and_then(|v| v.as_str()).unwrap_or("simple").to_string();
    let body = if preset != "omsi" {
        let name = match preset.as_str() {
            "wasd" => "W A S D only",
            "arrows" => "Arrow keys only",
            _ => "W A S D + arrows",
        };
        let bw = if body.w < 700.0 { 150.0 } else { 200.0 };
        let text = format!("Driving keys: {name} (Settings). Those keys drive the bus and win over the list below. Change any key here and your own layout (Custom controls) is used from then on.");
        let tw = body.w - bw - 70.0;
        let th = l.ui.paragraph_height(&text, tw, 12.5, Weight::Medium);
        let bar = Rect::new(body.x, body.y, body.w, (th + 22.0).max(54.0));
        l.ui.p().rounded(bar, 8.0, ACCENT.alpha(0.1));
        l.ui.p().rounded_border(bar, 8.0, 1.0, ACCENT.alpha(0.45));
        l.ui.icon("info", Vec2::new(bar.x + 22.0, bar.center().y), 20.0, ACCENT);
        l.ui.paragraph(&text, Vec2::new(bar.x + 42.0, bar.center().y - th * 0.5), tw, 12.5, Weight::Medium, TEXT_SOFT);
        if l.ui.button("kb-use-custom", Rect::new(bar.right() - bw - 10.0, bar.center().y - 18.0, bw, 36.0), "Use these keys", Some("keyboard"), ButtonKind::Primary) {
            use_custom_keys(l);
        }
        let used = bar.h + 12.0;
        Rect::new(body.x, body.y + used, body.w, body.h - used)
    } else {
        body
    };
    let half = (body.w - GAP * 2.0) * 0.5;
    for (sec, (title, sub, key)) in [("Driving & the bus", "The bus's own keys", "vehicles"), ("The game", "Menus, views, pausing", "game")].iter().enumerate() {
        let r = Rect::new(body.x + sec as f32 * (half + GAP * 2.0), body.y, half, body.h);
        l.ui.panel(r);
        let inner = l.ui.heading(Rect::new(r.x + 18.0, r.y + 14.0, r.w - 36.0, r.h - 28.0), title, Some(if sec == 0 { "directions_bus" } else { "sports_esports" }));
        l.ui.text_in(sub, Rect::new(inner.x, inner.y - 6.0, inner.w, 18.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
        let mut filter = std::mem::take(&mut l.pages.kb_filter[sec]);
        l.ui.text_input(&format!("kb-filter-{sec}"), Rect::new(inner.x, inner.y + 18.0, inner.w, 34.0), &mut filter, "Filter…", Some("search"));
        l.pages.kb_filter[sec] = filter.clone();
        let q = filter.to_lowercase();
        let list: Vec<(usize, String, i64, i64)> = l.state.keybindings.get(*key).and_then(|a| a.as_array()).map(|a| a.iter().enumerate().map(|(i, b)| (i, b.get("action").and_then(|x| x.as_str()).unwrap_or("").to_string(), b.get("scan_code").and_then(|x| x.as_i64()).unwrap_or(0), b.get("modifier").and_then(|x| x.as_i64()).unwrap_or(0))).collect()).unwrap_or_default();
        let mut shown: Vec<(usize, String, String, bool)> = list
            .iter()
            .filter(|(_, a, s, m)| q.is_empty() || action_label(a).to_lowercase().contains(&q) || key_name(*s, *m).to_lowercase().contains(&q))
            .map(|(i, a, s, m)| {
                let clash = *s != 0 && list.iter().any(|(j, _, s2, m2)| j != i && s2 == s && m2 == m);
                (*i, action_label(a), key_name(*s, *m), clash)
            })
            .collect();
        if sec == 1 {
            shown.sort_by_key(|(_, label, _, _)| !label.starts_with("VR:"));
        }
        let capturing = l.pages.capturing;
        let mut clicked: Option<(usize, bool)> = None;
        let time = l.ui.time;
        l.ui.scroll_area(&format!("kb-{sec}"), Rect::new(inner.x - 6.0, inner.y + 62.0, inner.w + 12.0, inner.h - 62.0), &mut |ui, v| {
            let rh = 40.0;
            for (row, (i, label, keyn, clash)) in shown.iter().enumerate() {
                let rr = Rect::new(v.x + 6.0, v.y + row as f32 * rh, v.w - 16.0, rh - 4.0);
                if rr.bottom() < v.y - 50.0 {
                    continue;
                }
                ui.p().rounded(rr, 8.0, Color::WHITE.alpha(0.03));
                ui.text_in(label, Rect::new(rr.x + 12.0, rr.y, rr.w - 210.0, rr.h), 13.0, Weight::Medium, TEXT_SOFT, Align::Left);
                let kr = Rect::new(rr.right() - 190.0, rr.y + 5.0, 150.0, rr.h - 10.0);
                let waiting = capturing == Some((sec, *i));
                let id = id_of(&format!("kb-{sec}-{i}"));
                let (h, _, c) = ui.interact(id, kr);
                if c {
                    clicked = Some((*i, false));
                }
                let base = if waiting { ACCENT.alpha(0.25 + 0.15 * (time * 6.0).sin().abs()) } else if *clash { DANGER.alpha(0.22) } else { Color::WHITE.alpha(if h { 0.12 } else { 0.07 }) };
                ui.p().rounded(kr, 6.0, base);
                ui.p().rounded_border(kr, 6.0, 1.0, if waiting { ACCENT } else if *clash { DANGER } else { Color::WHITE.alpha(0.1) });
                ui.text_in(if waiting { "press a key…" } else { keyn }, kr, 12.0, Weight::Bold, if *clash { DANGER.lighten(0.3) } else { TEXT }, Align::Center);
                let xr = Rect::new(rr.right() - 32.0, rr.y + 5.0, 26.0, rr.h - 10.0);
                let (hx, _, cx) = ui.interact(id ^ 1, xr);
                ui.icon("close", xr.center(), 16.0, if hx { DANGER } else { TEXT_FAINT });
                if cx {
                    clicked = Some((*i, true));
                }
            }
            shown.len() as f32 * rh
        });
        match clicked {
            Some((i, true)) => {
                let vr_binding = l.state.keybindings.get(*key).and_then(|a| a.as_array())
                    .and_then(|a| a.get(i)).and_then(|b| b.get("action"))
                    .and_then(|a| a.as_str()).is_some_and(|a| a.starts_with("vr_"));
                if let Some(b) = l.state.keybindings.get_mut(*key).and_then(|a| a.as_array_mut()).and_then(|a| a.get_mut(i)) {
                    b["scan_code"] = json!(0);
                    b["modifier"] = json!(0);
                }
                save_keys(l, vr_binding);
            }
            Some((i, false)) => l.pages.capturing = Some((sec, i)),
            None => {}
        }
    }
    if !l.state.keybindings_error.is_empty() {
        let e = l.state.keybindings_error.clone();
        l.ui.text_in(&e, Rect::new(body.x, body.bottom() + 4.0, body.w, 18.0), 12.0, Weight::Medium, DANGER, Align::Left);
    }
}

/// Hide empty slots beyond the physical buttons without changing the saved controller file.
fn shown_button_count(buttons: &[(String, String)], physical: usize, revealed: Option<usize>) -> usize {
    physical
        .max(buttons.iter().rposition(|(action, _)| !action.trim().is_empty()).map(|i| i + 1).unwrap_or(0))
        .max(revealed.map(|i| i + 1).unwrap_or(0))
}

/// The game controllers tab (see `PadsView`).
fn game_controllers(l: &mut Launcher, body: Rect) {
    use crate::controllers::{DeviceCfg, Func};
    let hwnd = l.window.as_deref().and_then(crate::controllers::window_handle);
    let pv = &mut l.pages.pads;
    if !pv.tried {
        pv.tried = true;
        pv.io = Some(crate::controllers::Devices::new(hwnd, false));
    }
    if pv.devices.is_none() {
        let root = std::path::PathBuf::from(&l.state.config.root);
        pv.devices = Some(crate::controllers::read_cfg(&root));
    }
    // what the devices do now (and a button pressed while one is awaited)
    let mut pressed: Vec<(String, usize)> = Vec::new();
    let mut connected: Vec<crate::controllers::Connected> = Vec::new();
    if let Some(io) = pv.io.as_mut() {
        for (name, n, down) in io.poll() {
            if down {
                pressed.push((name, n));
            }
        }
        connected = io.connected();
    }
    let devices = pv.devices.get_or_insert_with(Vec::new);
    // devices connected but not set up yet can be added
    let list_w = (body.w * 0.32).min(360.0);
    let left = Rect::new(body.x, body.y, list_w, body.h);
    let right = Rect::new(body.x + list_w + GAP * 2.0, body.y, body.w - list_w - GAP * 2.0, body.h);
    l.ui.panel(left);
    let inner = l.ui.heading(Rect::new(left.x + 18.0, left.y + 14.0, left.w - 36.0, left.h - 28.0), "Devices", Some("sports_esports"));
    let mut add: Option<String> = None;
    let mut sel = pv.selected;
    let list_r = Rect::new(inner.x - 6.0, inner.y, inner.w + 12.0, inner.h - 108.0);
    let offs: Vec<String> = l.state.settings.get("ctrl_off").and_then(|v| v.as_str()).unwrap_or("").split('|').map(str::to_string).filter(|s| !s.is_empty()).collect();
    {
        let ui = &mut l.ui;
        let devices = &*devices;
        let connected = &connected;
        ui.scroll_area("pad-list", list_r, &mut |ui, v| {
            let mut y = v.y;
            for (i, d) in devices.iter().enumerate() {
                let on = connected.iter().any(|c| crate::controllers::names_match(&d.name, &c.name));
                let switched_off = offs.iter().any(|o| o.eq_ignore_ascii_case(&d.name));
                let r = Rect::new(v.x + 6.0, y, v.w - 12.0, 44.0);
                if ui.row(&format!("pad-{i}"), r, sel == i) {
                    sel = i;
                }
                ui.text_in(&d.name, Rect::new(r.x + 12.0, r.y, r.w - 40.0, r.h), 13.0, Weight::Medium, if on { TEXT } else { TEXT_DIM }, Align::Left);
                if switched_off {
                    ui.text_in("off", Rect::new(r.right() - 40.0, r.y, 30.0, r.h), 11.5, Weight::Bold, TEXT_FAINT, Align::Right);
                } else {
                    ui.icon(if on { "check_circle" } else { "remove" }, Vec2::new(r.right() - 18.0, r.center().y), 16.0, if on { OK } else { TEXT_FAINT });
                }
                y += 48.0;
            }
            for c in connected.iter().filter(|c| !devices.iter().any(|d| crate::controllers::names_match(&d.name, &c.name))) {
                let r = Rect::new(v.x + 6.0, y, v.w - 12.0, 38.0);
                if ui.button(&format!("pad-add-{}", c.name), r, &format!("Set up {}", c.name), Some("add"), ButtonKind::Primary) {
                    add = Some(c.name.clone());
                }
                y += 44.0;
            }
            if devices.is_empty() && connected.is_empty() {
                y += 4.0 + ui.paragraph("No game controller is connected, and none is set up. Connect a wheel, pedals or a joystick; a gamepad works without setting up (left stick steers, the triggers are the pedals).", Vec2::new(v.x + 6.0, y + 4.0), v.w - 12.0, 13.0, Weight::Regular, TEXT_DIM);
            }
            y - v.y
        });
    }
    if sel != pv.selected {
        pv.selected = sel;
        pv.capturing = false;
        pv.revealed_button = None;
        pv.wizard = None;
    }
    if let Some(name) = add {
        devices.push(DeviceCfg { name, second: "0".into(), ..Default::default() });
        pv.selected = devices.len() - 1;
        pv.revealed_button = None;
        pv.dirty = true;
        // a new device starts with the assistant
        pv.wizard = Some(Wizard { step: 0, rest: [None; 8], at: Vec::new(), error: None });
    }
    // the dead zone (a setting of the game's)
    let dz_r = Rect::new(inner.x, inner.bottom() - 98.0, inner.w, 34.0);
    let mut dz = l.state.settings.get("ctrl_deadzone").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
    if l.ui.slider("pad-dz", dz_r, &mut dz, 0.0, 0.3, 0.01, "Dead zone", &|v| format!("{:.0} %", v * 100.0)) {
        l.state.settings["ctrl_deadzone"] = json!(dz);
        l.state.settings_dirty = 0.3;
    }
    let save_r = Rect::new(inner.x, inner.bottom() - 48.0, inner.w, 40.0);
    if l.ui.button("pad-save", save_r, if pv.dirty { "Save" } else { "Saved" }, Some("save"), if pv.dirty { ButtonKind::Primary } else { ButtonKind::Normal }) && pv.dirty {
        match save_gamectrler(devices) {
            Ok(p) => {
                pv.dirty = false;
                l.state.set_status(format!("Game controllers saved to {}", p.display()), false);
            }
            Err(e) => l.state.set_status(format!("Not saved: {e}"), true),
        }
    }
    // the device shown
    l.ui.panel(right);
    let Some(d) = devices.get_mut(pv.selected) else { return };
    let inner = l.ui.heading(Rect::new(right.x + 18.0, right.y + 14.0, right.w - 36.0, right.h - 28.0), &d.name.clone(), Some("tune"));
    let live_dev = connected.iter().find(|c| crate::controllers::names_match(&d.name, &c.name));
    let live: Vec<(usize, f32)> = live_dev.map(|c| c.axes.clone()).unwrap_or_default();
    // every button the device has gets its line (DirectInput says how many)
    if let Some(n) = live_dev.map(|c| c.buttons).filter(|n| *n > d.buttons.len()) {
        d.buttons.resize(n, (String::new(), "0".into()));
    }
    // the assistant, over the device's page
    if let Some(w) = pv.wizard.as_mut() {
        let done = wizard(&mut l.ui, inner, w, d, &live, live_dev.is_some());
        match done {
            Some(true) => {
                pv.wizard = None;
                pv.dirty = true;
                l.state.set_status("Set up: press Save to keep it (the buttons can be given their keys below).", false);
            }
            Some(false) => pv.wizard = None,
            None => {}
        }
        return;
    }
    // this device on or off (a second listing of the same wheel, a device not to be used)
    {
        let offs: Vec<String> = l.state.settings.get("ctrl_off").and_then(|v| v.as_str()).unwrap_or("").split('|').map(str::to_string).filter(|s| !s.is_empty()).collect();
        let mut on = !offs.iter().any(|o| o.eq_ignore_ascii_case(&d.name));
        if l.ui.toggle("pad-on", Rect::new(inner.right() - 400.0, inner.y - 36.0, 170.0, 30.0), &mut on, "Use this device") {
            let mut offs: Vec<String> = offs.into_iter().filter(|o| !o.eq_ignore_ascii_case(&d.name)).collect();
            if !on {
                offs.push(d.name.clone());
            }
            l.state.settings["ctrl_off"] = json!(offs.join("|"));
            l.state.settings_dirty = 0.3;
        }
    }
    if l.ui.button("pad-wizard", Rect::new(inner.right() - 220.0, inner.y - 36.0, 220.0, 30.0), "Set up step by step", Some("touch_app"), ButtonKind::Normal) {
        pv.wizard = Some(Wizard { step: 0, rest: [None; 8], at: Vec::new(), error: None });
    }
    const AXES: [&str; 8] = ["X axis", "Y axis", "Z axis", "X rotation", "Y rotation", "Z rotation", "Slider 1", "Slider 2"];
    let funcs: Vec<String> = Func::LABELS.iter().map(|s| s.to_string()).collect();
    let mut actions: Vec<String> = vec!["<none>".into()];
    actions.extend(l.state.keybindings.get("vehicles").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|b| b.get("action").and_then(|x| x.as_str()).map(String::from)).collect::<Vec<_>>()).unwrap_or_default());
    // H-pattern shifters use OMSI's "_fest" actions: pressing the gate selects the gear,
    // releasing it fires "_fest_off", which lets the bus script return to neutral.
    for a in ["kw_s_R_fest", "kw_s_1_fest", "kw_s_2_fest", "kw_s_3_fest", "kw_s_4_fest", "kw_s_5_fest", "kw_s_6_fest", "kw_s_7_fest", "kw_s_8_fest", "kw_s_9_fest", "kw_s_10_fest"] {
        if !actions.iter().any(|x| x.eq_ignore_ascii_case(a)) {
            actions.push(a.to_string());
        }
    }
    // the game's own view actions (looking around while held, the cameras, the views)
    for a in ["gear_up", "gear_down", "view_look_left", "view_look_right", "view_look_up", "view_look_down", "view_reset_direction", "view_interiorcam_plus", "view_interiorcam_minus", "view_toggle_viewpoint", "view_set_driver", "view_set_passenger", "view_set_outside", "sim_pause", "screenshot", "quicksave", "toggel_mouse_ctrl", "toggel_ctrler"] {
        if !actions.iter().any(|x| x == a) {
            actions.insert(1, a.to_string());
        }
    }
    actions.dedup();
    let mut dirty = false;
    let lit = pv.last_pressed.filter(|(_, t)| t.elapsed().as_secs_f32() < 4.0).map(|(b, _)| b);
    // Some OMSI configs contain hundreds of empty trailing slots (the G920 report had
    // 131 entries for 18 physical buttons). Keep them on disk, but do not fill the UI with them.
    let shown_buttons = shown_button_count(&d.buttons, live_dev.map(|c| c.buttons).unwrap_or(0), pv.revealed_button);
    // (the axes, then every button of the device: the list scrolls - it stopped at the ten
    // buttons that fitted)
    let list = Rect::new(inner.x - 6.0, inner.y, inner.w + 12.0, inner.h - 50.0);
    let mut buttons_start_y = 0.0;
    l.ui.scroll_area("pad-detail", list, &mut |ui, v| {
        let x0 = v.x + 6.0;
        let w = v.w - 16.0;
        let mut y = v.y;
        if live_dev.is_some_and(|c| c.ff_capable) {
            let (mut steering_force, mut vibration) = d.ff_scale.unwrap_or((1.0, 1.0));
            if ui.slider("pad-ff-steering", Rect::new(x0, y, w, ROW), &mut steering_force, 0.0, 2.0, 0.05, "Steering force", &|v| format!("{:.0}%", v * 100.0)) {
                d.ff_scale = Some((steering_force, vibration));
                dirty = true;
            }
            y += ROW + 6.0;
            if ui.slider("pad-ff-vibration", Rect::new(x0, y, w, ROW), &mut vibration, 0.0, 2.0, 0.05, "Vibration", &|v| format!("{:.0}%", v * 100.0)) {
                d.ff_scale = Some((steering_force, vibration));
                dirty = true;
            }
            y += ROW + 20.0;
        }
        let lab_w = if w < 520.0 { 84.0 } else { 110.0 };
        let inv_w = 110.0;
        let sel_w = (w - lab_w - inv_w - 60.0 - 3.0 * GAP).clamp(120.0, 200.0);
        let bar_w = (w - lab_w - sel_w - inv_w - 3.0 * GAP).max(30.0);
        for a in 0..8 {
            let r = Rect::new(x0, y, w, ROW);
            ui.label(Rect::new(r.x, r.y, lab_w, r.h), AXES[a]);
            let bar = Rect::new(r.x + lab_w + GAP, r.y + 12.0, bar_w, r.h - 24.0);
            ui.p().rounded(bar, 4.0, Color::WHITE.alpha(0.06));
            if let Some((_, v)) = live.iter().find(|(k, _)| *k == a) {
                let x = bar.x + (v.clamp(-1.0, 1.0) + 1.0) * 0.5 * bar.w;
                ui.p().rounded(Rect::new(x - 2.0, bar.y - 3.0, 4.0, bar.h + 6.0), 2.0, ACCENT);
            }
            let mut sel = (Func::code(d.axes[a].map(|x| x.0)) + 1) as usize;
            if ui.select(&format!("pad-axis-{a}"), Rect::new(bar.right() + GAP, r.y, sel_w, r.h), &mut sel, &funcs) {
                let inv = d.axes[a].map(|x| x.1).unwrap_or(false);
                d.axes[a] = Func::from_code(sel as i32 - 1).map(|f| (f, inv));
                dirty = true;
            }
            let mut inv = d.axes[a].map(|x| x.1).unwrap_or(false);
            if d.axes[a].is_some() && ui.toggle(&format!("pad-inv-{a}"), Rect::new(bar.right() + GAP + sel_w + GAP, r.y, inv_w, r.h), &mut inv, "Reversed") {
                if let Some(x) = d.axes[a].as_mut() {
                    x.1 = inv;
                }
                dirty = true;
            }
            y += ROW + 6.0;
        }
        // buttons: their key actions
        y += 10.0;
        ui.text_in("Buttons", Rect::new(x0, y, w, 20.0), 14.0, Weight::Bold, TEXT, Align::Left);
        y += 26.0;
        buttons_start_y = y - v.y;
        let cols = if w < 560.0 { 1usize } else { 2 };
        let cw = (w - GAP * (cols - 1) as f32) / cols as f32;
        let per_row = ROW + 4.0;
        let rows = shown_buttons.div_ceil(cols);
        for (b, (act, _)) in d.buttons.iter_mut().take(shown_buttons).enumerate() {
            let (col, row) = (b / rows.max(1), b % rows.max(1));
            let r = Rect::new(x0 + col as f32 * (cw + GAP), y + row as f32 * per_row, cw, ROW);
            let label = match b.checked_sub(crate::controllers::HAT_BUTTONS) {
                Some(h) => format!("Hat {} {}", h / 4 + 1, ["up", "right", "down", "left"][h % 4]),
                None => format!("Button {}", b + 1),
            };
            if lit == Some(b) {
                ui.p().rounded(Rect::new(r.x - 4.0, r.y - 2.0, r.w + 8.0, r.h + 4.0), 6.0, ACCENT.alpha(0.28));
            }
            ui.label(Rect::new(r.x, r.y, 90.0, r.h), &label);
            let mut sel = actions.iter().position(|a| a.eq_ignore_ascii_case(act)).unwrap_or(0);
            if ui.select(&format!("pad-btn-{b}"), Rect::new(r.x + 90.0, r.y, r.w - 90.0, r.h), &mut sel, &actions) {
                *act = if sel == 0 { String::new() } else { actions[sel].clone() };
                dirty = true;
            }
        }
        y += rows as f32 * per_row;
        y - v.y + 8.0
    });
    if dirty {
        pv.dirty = true;
    }
    // a button pressed on the device: its line (added up to it)
    if let Some((name, n)) = pressed.into_iter().find(|(name, _)| crate::controllers::names_match(&d.name, name)) {
        if crate::controllers::names_match(&d.name, &name) && n < crate::controllers::HAT_BUTTONS + 16 {
            while d.buttons.len() <= n {
                d.buttons.push((String::new(), "0".into()));
                pv.dirty = true;
            }
            let was_capturing = pv.capturing;
            if was_capturing {
                pv.capturing = false;
                pv.revealed_button = Some(n);
                let cols = if list.w - 16.0 < 560.0 { 1usize } else { 2 };
                let rows = shown_buttons.max(n + 1).div_ceil(cols).max(1);
                let row = n % rows;
                l.ui.scroll_to("pad-detail", buttons_start_y + row as f32 * (ROW + 4.0), ROW, list.h);
            }
            pv.last_pressed = Some((n, std::time::Instant::now()));
            let now = d.buttons.get(n).map(|b| b.0.clone()).filter(|a| !a.is_empty());
            let label = match n.checked_sub(crate::controllers::HAT_BUTTONS) {
                Some(h) => format!("hat {} {}", h / 4 + 1, ["up", "right", "down", "left"][h % 4]),
                None => format!("button {}", n + 1),
            };
            l.state.set_status(match now {
                Some(a) => format!("{name}: {label} - {} (lit in the list: choose another there)", action_label(&a)),
                None => format!("{name}: {label} - nothing yet (lit in the list: choose what it does)"),
            }, false);
        }
    }
    let add_r = Rect::new(inner.x, inner.bottom() - 40.0, 260.0, 36.0);
    if l.ui.button("pad-add-button", add_r, if pv.capturing { "Press a button on the device…" } else { "Add a button" }, Some("add"), ButtonKind::Normal) {
        pv.capturing = !pv.capturing;
    }
}

/// The steps of the set-up assistant (see `Wizard`): what the player is asked each time.
const WIZARD_STEPS: [(&str, &str); 5] = [
    ("Let go of everything", "Take your hands off the wheel and your feet off the pedals (the wheel in the middle), then press Next."),
    ("Steering", "Turn the wheel (or move the stick) all the way to the LEFT and hold it there, then press Next."),
    ("Throttle", "Press the throttle pedal all the way down and hold it, then press Next. No pedals: Skip."),
    ("Brake", "Press the brake pedal all the way down and hold it, then press Next. No brake pedal: Skip."),
    ("Clutch", "Press the clutch pedal all the way down and hold it, then press Next. No clutch: Skip."),
];

/// One frame of the assistant in `r`; Some(true) when it has set the device up, Some(false)
/// when the player gave up.
fn wizard(ui: &mut Ui, r: Rect, w: &mut Wizard, d: &mut crate::controllers::DeviceCfg, live: &[(usize, f32)], connected: bool) -> Option<bool> {
    let (title, text) = WIZARD_STEPS[w.step];
    ui.text_in(&format!("Step {} of {}: {title}", w.step + 1, WIZARD_STEPS.len()), Rect::new(r.x, r.y, r.w, 26.0), 17.0, Weight::Bold, TEXT, Align::Left);
    let mut y = r.y + 34.0;
    y += ui.paragraph(text, Vec2::new(r.x, y), r.w, 13.5, Weight::Regular, TEXT_SOFT) + 10.0;
    if !connected {
        y += ui.paragraph("The device is not connected: plug it in (the list on the left marks it green).", Vec2::new(r.x, y), r.w, 13.0, Weight::Medium, DANGER);
    }
    if let Some(e) = &w.error {
        y += ui.paragraph(e, Vec2::new(r.x, y), r.w, 13.0, Weight::Medium, DANGER) + 6.0;
    }
    // the axes as they stand, so that the player sees the device answer
    for (k, v) in live {
        let bar = Rect::new(r.x + 90.0, y + 8.0, (r.w - 100.0).max(40.0), 8.0);
        ui.text_in(["X", "Y", "Z", "Rx", "Ry", "Rz", "Slider 1", "Slider 2"][*k], Rect::new(r.x, y, 84.0, 24.0), 12.0, Weight::Medium, TEXT_DIM, Align::Left);
        ui.p().rounded(bar, 4.0, Color::WHITE.alpha(0.06));
        let x = bar.x + (v.clamp(-1.0, 1.0) + 1.0) * 0.5 * bar.w;
        ui.p().rounded(Rect::new(x - 2.0, bar.y - 4.0, 4.0, bar.h + 8.0), 2.0, ACCENT);
        y += 26.0;
    }
    let now = |live: &[(usize, f32)]| {
        let mut a = [None; 8];
        for (k, v) in live {
            a[*k] = Some(*v);
        }
        a
    };
    let by = r.bottom() - 40.0;
    if ui.button("wiz-cancel", Rect::new(r.x, by, 120.0, 36.0), "Cancel", None, ButtonKind::Ghost) {
        return Some(false);
    }
    let skip = w.step >= 2 && ui.button("wiz-skip", Rect::new(r.right() - 260.0, by, 110.0, 36.0), "Skip", None, ButtonKind::Normal);
    let next = ui.button("wiz-next", Rect::new(r.right() - 140.0, by, 140.0, 36.0), if w.step + 1 == WIZARD_STEPS.len() { "Finish" } else { "Next" }, Some("chevron_right"), ButtonKind::Primary);
    if !(next || skip) {
        return None;
    }
    let cur = now(live);
    w.error = None;
    if w.step == 0 {
        if live.is_empty() {
            w.error = Some("The device shows no axis yet: move the wheel and the pedals a little, let go, and press Next again.".into());
            return None;
        }
        w.rest = cur;
    } else if skip {
        w.at.push([None; 8]);
    } else {
        // the axis that moved most since everything was let go (one taken before is not
        // taken again - but the throttle's axis may turn out to be the brake's too)
        let used: Vec<usize> = w.at.iter().filter_map(|a| moved_most(&w.rest, a, &[]).map(|m| m.0)).collect();
        let exclude: Vec<usize> = if w.step == 3 { used.iter().copied().take(1).collect() } else { used.clone() };
        match moved_most(&w.rest, &cur, &exclude) {
            Some(_) => w.at.push(cur),
            None => {
                w.error = Some("Nothing moved far enough. Hold it all the way, then press Next (or Skip).".into());
                return None;
            }
        }
    }
    w.step += 1;
    if w.step < WIZARD_STEPS.len() {
        return None;
    }
    d.axes = wizard_result(&w.rest, &w.at);
    Some(true)
}

/// The axes the assistant found: `rest` where everything rested, `at` where the axes stood
/// with the wheel turned left, the throttle, the brake and the clutch pressed (all None: that
/// step skipped).
fn wizard_result(rest: &[Option<f32>; 8], at: &[[Option<f32>; 8]]) -> [Option<(crate::controllers::Func, bool)>; 8] {
    use crate::controllers::Func;
    let mut axes: [Option<(Func, bool)>; 8] = [None; 8];
    let steer = at.first().and_then(|a| moved_most(rest, a, &[]));
    if let Some((k, delta)) = steer {
        // turned left the value falls: else the axis runs the other way
        axes[k] = Some((Func::Steering, delta > 0.0));
    }
    let taken: Vec<usize> = steer.map(|s| vec![s.0]).unwrap_or_default();
    let pedal = |i: usize, ex: &[usize]| at.get(i).and_then(|a| moved_most(rest, a, ex));
    let throttle = pedal(1, &taken);
    let brake = pedal(2, &taken);
    match (throttle, brake) {
        // one axis for both (pedals on a single axis): the throttle one way, the brake the other
        (Some((kt, dt)), Some((kb, db))) if kt == kb && dt * db < 0.0 => axes[kt] = Some((Func::ThrottleBrake, dt > 0.0)),
        _ => {
            // a pedal pressed goes towards 1
            if let Some((k, dl)) = throttle {
                axes[k] = Some((Func::Throttle, dl < 0.0));
            }
            if let Some((k, dl)) = brake.filter(|b| Some(b.0) != throttle.map(|t| t.0)) {
                axes[k] = Some((Func::Brake, dl < 0.0));
            }
        }
    }
    let mut ex = taken.clone();
    ex.extend(throttle.map(|t| t.0));
    ex.extend(brake.map(|t| t.0));
    if let Some((k, dl)) = pedal(3, &ex) {
        axes[k] = Some((Func::Clutch, dl < 0.0));
    }
    axes
}

/// The axis that moved most from `rest` to `now` (at least a sixth of its travel), not one of
/// `exclude`: (slot, how far, signed).
fn moved_most(rest: &[Option<f32>; 8], now: &[Option<f32>; 8], exclude: &[usize]) -> Option<(usize, f32)> {
    (0..8)
        .filter(|k| !exclude.contains(k))
        .filter_map(|k| Some((k, now[k]? - rest[k].unwrap_or(0.0))))
        .filter(|(_, d)| d.abs() > 0.33)
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
}

/// Write the devices to the content folder's `Inputs/gamectrler.cfg` (OMSI 2's own is only
/// read; the game takes the content folder's first).
fn save_gamectrler(devices: &[crate::controllers::DeviceCfg]) -> Result<std::path::PathBuf, String> {
    let candidate = core::content_dir().unwrap_or_else(core::data_dir).join("Inputs");
    let dir = if (candidate.exists() || std::fs::create_dir_all(&candidate).is_ok()) && omsi_cfg::is_writable(&candidate) {
        candidate
    } else {
        core::data_dir().join("Inputs")
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let p = dir.join("gamectrler.cfg");
    std::fs::write(&p, crate::controllers::cfg_text(devices)).map_err(|e| e.to_string())?;
    omsi_cfg::content_changed();
    Ok(p)
}

/// Settings → Driving keys: "Custom controls", the keys of the Controls page.
fn use_custom_keys(l: &mut Launcher) -> bool {
    if l.state.settings.get("drive_keys").and_then(|v| v.as_str()) == Some("omsi") {
        return false;
    }
    l.state.settings["drive_keys"] = json!("omsi");
    l.state.settings_dirty = 0.3;
    l.state.set_status("Driving keys: Custom controls - the game uses the keys of this page.", false);
    true
}

fn save_keys(l: &mut Launcher, vr_binding: bool) {
    match core::save_keybindings(&l.state.keybindings) {
        Ok(()) => {
            l.state.keybindings_error.clear();
            if let Ok(k) = core::get_keybindings() {
                l.state.keybindings = k;
            }
            // a key changed is a key the player wants to use: with a ready-made layout it
            // would be ignored wherever that layout has a key of its own
            if !vr_binding && use_custom_keys(l) {
                l.state.set_status("Key bindings saved; Driving keys switched to Custom controls so the game uses them.", false);
            } else {
                l.state.set_status("Key bindings saved.", false);
            }
        }
        Err(e) => {
            l.state.keybindings_error = format!("{e:#}");
            l.state.set_status(format!("{e:#}"), true);
        }
    }
}

// --- sessions ---------------------------------------------------------------------------------

fn ago(t: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(t);
    let s = now.saturating_sub(t);
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else {
        format!("{} h {} min", s / 3600, (s / 60) % 60)
    }
}

pub fn sessions(l: &mut Launcher, area: Rect) {
    let body = l.page_title(area, "Sessions", "The games you started, and who drives with you.");
    let list = l.state.instances.clone();
    if list.is_empty() {
        let r = Rect::new(body.x, body.y, body.w.min(720.0), 120.0);
        l.ui.panel(r);
        l.ui.icon("sports_esports", Vec2::new(r.x + 40.0, r.center().y), 36.0, TEXT_FAINT);
        l.ui.paragraph("No game is running. Start a duty on the Drive page; to drive with friends, turn on hosting on the Multiplayer page and give them the code shown here.", Vec2::new(r.x + 76.0, r.y + 30.0), r.w - 100.0, 13.5, Weight::Regular, TEXT_DIM);
        return;
    }
    let names: std::collections::HashMap<String, String> = l.state.vehicles.iter().map(|v| (v.file.clone(), v.name.clone())).collect();
    let short_bus = |b: &str| names.get(b).cloned().unwrap_or_else(|| b.rsplit('/').next().unwrap_or("").trim_end_matches(".bus").to_string());
    let mut y = body.y - l.ui.scroll.get(&id_of("sessions")).copied().unwrap_or(0.0);
    // (the page's whole width, as the other pages: at 900 a wide window had the cards in its
    // left half and the buttons in the middle of nowhere)
    let view = Rect::new(body.x, body.y, body.w, body.h);
    let mut actions: Vec<(u32, &str)> = Vec::new();
    let mut copy: Option<String> = None;
    l.ui.push_clip(view, 0.0);
    for i in &list {
        let lan = i.lan_status.clone().unwrap_or(Value::Null);
        let role = lan.get("role").and_then(|x| x.as_str()).unwrap_or("");
        let players: Vec<Value> = lan.get("players").and_then(|x| x.as_array()).cloned().unwrap_or_default();
        let chat: Vec<String> = lan.get("chat").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect()).unwrap_or_default();
        let warnings: Vec<String> = lan.get("warnings").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect()).unwrap_or_default();
        let log_open = l.state.open_logs.contains(&i.pid);
        let log_lines = l.state.logs.get(&i.pid).cloned().unwrap_or_default();
        let mut h = 96.0;
        if role == "host" {
            h += 96.0;
        } else if !role.is_empty() {
            h += 24.0;
        }
        h += if players.is_empty() { 0.0 } else { 26.0 + players.len() as f32 * 20.0 };
        h += if chat.is_empty() { 0.0 } else { 26.0 + chat.len().min(6) as f32 * 18.0 };
        h += warnings.len() as f32 * 20.0;
        if log_open {
            h += 220.0;
        }
        let r = Rect::new(view.x, y, view.w, h);
        l.ui.panel(r);
        let running = i.running;
        let c = Vec2::new(r.x + 24.0, r.y + 28.0);
        if running {
            let pulse = (l.ui.time * 3.0).sin() * 0.5 + 0.5;
            l.ui.p().circle(c, 6.0 + 3.0 * pulse, OK.alpha(0.25));
        }
        l.ui.p().circle(c, 6.0, if running { OK } else { TEXT_FAINT });
        let duty = i.line.as_ref().map(|ln| format!(" · line {ln}{}", i.tour.as_ref().map(|t| format!(" / {t}")).unwrap_or_default())).unwrap_or_default();
        l.ui.text_in(&format!("{} · {}{duty}", short_map(&i.map), short_bus(&i.bus)), Rect::new(r.x + 42.0, r.y + 16.0, r.w - 260.0, 24.0), 16.0, Weight::Black, TEXT, Align::Left);
        let status = if running {
            if l.state.stopping.contains(&i.pid) || i.stopping.is_some() { "stopping - saving the run…".to_string() } else { format!("running for {}", ago(i.started)) }
        } else {
            let how = if i.exit_code == Some(0) { String::new() } else if i.killed { " (killed - it did not end by itself, the run is not saved)".into() } else { i.exit_code.map(|c| format!(" (exit code {c})")).unwrap_or_default() };
            format!("ended{how}")
        };
        l.ui.text_in(&format!("{status} · driver {}", i.profile), Rect::new(r.x + 42.0, r.y + 42.0, r.w - 60.0, 18.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
        l.ui.text_in(&i.last_line, Rect::new(r.x + 42.0, r.y + 62.0, r.w - 60.0, 18.0), 11.5, Weight::Regular, TEXT_FAINT, Align::Left);
        // buttons
        let bw = 110.0;
        if running {
            let stopping = l.state.stopping.contains(&i.pid);
            if l.ui.button(&format!("stop-{}", i.pid), Rect::new(r.right() - 18.0 - bw, r.y + 14.0, bw, 34.0), if stopping { "Stopping…" } else { "Stop" }, Some("close"), ButtonKind::Danger) && !stopping {
                actions.push((i.pid, "stop"));
            }
        }
        if l.ui.button(&format!("log-{}", i.pid), Rect::new(r.right() - 18.0 - bw - if running { bw + 8.0 } else { 0.0 }, r.y + 14.0, bw, 34.0), if log_open { "Hide log" } else { "Show log" }, Some("receipt_long"), ButtonKind::Normal) {
            actions.push((i.pid, "log"));
        }
        let mut yy = r.y + 86.0;
        if role == "host" {
            let code = lan.get("code").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let box_r = Rect::new(r.x + 20.0, yy, r.w - 40.0, 84.0);
            l.ui.p().rounded(box_r, 10.0, ACCENT.alpha(0.10));
            l.ui.p().rounded_border(box_r, 10.0, 1.0, ACCENT.alpha(0.5));
            l.ui.text_in("SESSION CODE", Rect::new(box_r.x + 16.0, box_r.y + 8.0, 200.0, 16.0), 10.5, Weight::Black, ACCENT, Align::Left);
            l.ui.text_in(&code, Rect::new(box_r.x + 16.0, box_r.y + 26.0, box_r.w - 170.0, 30.0), 20.0, Weight::Condensed, TEXT, Align::Left);
            l.ui.text_in("Your friends paste it into Multiplayer → Connect by Code.", Rect::new(box_r.x + 16.0, box_r.y + 58.0, box_r.w - 170.0, 18.0), 11.5, Weight::Regular, TEXT_DIM, Align::Left);
            if l.ui.button(&format!("copy-{}", i.pid), Rect::new(box_r.right() - 146.0, box_r.y + 24.0, 130.0, 36.0), "Copy code", Some("content_copy"), ButtonKind::Primary) {
                copy = Some(code.clone());
            }
            yy += 96.0;
        } else if role == "client" {
            let connected = lan.get("connected").and_then(|x| x.as_bool()).unwrap_or(false);
            let text = if let Some(rej) = lan.get("rejected").and_then(|x| x.as_str()) {
                format!("not connected: {rej}")
            } else if connected {
                format!("connected to {}", lan.get("host_name").and_then(|x| x.as_str()).unwrap_or(""))
            } else {
                "connecting…".to_string()
            };
            l.ui.text_in(&format!("Multiplayer: {text}"), Rect::new(r.x + 42.0, yy, r.w - 60.0, 20.0), 12.5, Weight::Medium, if connected { OK } else { WARN }, Align::Left);
            yy += 24.0;
        }
        if !players.is_empty() {
            l.ui.text_in("PLAYERS", Rect::new(r.x + 42.0, yy, 200.0, 18.0), 10.5, Weight::Black, TEXT_FAINT, Align::Left);
            yy += 22.0;
            for p in &players {
                let s = |k: &str| p.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                let pax = p.get("passengers").and_then(|x| x.as_i64()).unwrap_or(0);
                let dest = if s("destination").is_empty() { String::new() } else { format!(" · {} → {}", s("line"), s("destination")) };
                l.ui.text_in(&format!("{} · {}{dest}{} · {}", s("name"), short_bus(&s("bus")), if pax > 0 { format!(" · {pax} passengers") } else { String::new() }, s("where")), Rect::new(r.x + 42.0, yy, r.w - 60.0, 18.0), 12.5, Weight::Medium, TEXT_SOFT, Align::Left);
                yy += 20.0;
            }
        }
        if !chat.is_empty() {
            l.ui.text_in("CHAT  (V in the game to write)", Rect::new(r.x + 42.0, yy + 4.0, 300.0, 18.0), 10.5, Weight::Black, TEXT_FAINT, Align::Left);
            yy += 26.0;
            for c in chat.iter().rev().take(6).rev() {
                l.ui.text_in(c, Rect::new(r.x + 42.0, yy, r.w - 60.0, 18.0), 12.0, Weight::Regular, TEXT_SOFT, Align::Left);
                yy += 18.0;
            }
        }
        for w in &warnings {
            l.ui.icon("warning", Vec2::new(r.x + 50.0, yy + 9.0), 15.0, WARN);
            l.ui.text_in(w, Rect::new(r.x + 64.0, yy, r.w - 80.0, 18.0), 12.0, Weight::Medium, WARN, Align::Left);
            yy += 20.0;
        }
        if log_open {
            let lr = Rect::new(r.x + 20.0, yy + 6.0, r.w - 40.0, 204.0);
            l.ui.p().rounded(lr, 8.0, Color::rgba(6, 8, 10, 0.9));
            let text: Vec<String> = log_lines.iter().rev().take(11).rev().cloned().collect();
            for (k, line) in text.iter().enumerate() {
                l.ui.text_in(line, Rect::new(lr.x + 10.0, lr.y + 6.0 + k as f32 * 18.0, lr.w - 20.0, 18.0), 11.0, Weight::Regular, if line.contains("ERROR") { DANGER } else if line.contains("WARN") { WARN } else { TEXT_DIM }, Align::Left);
            }
        }
        y += h + GAP;
    }
    l.ui.pop_clip();
    // scrolling the list
    let content = y + l.ui.scroll.get(&id_of("sessions")).copied().unwrap_or(0.0) - body.y;
    if l.ui.hover(view) && l.ui.input.wheel.y.abs() > 0.0 {
        let s = l.ui.scroll.entry(id_of("sessions")).or_insert(0.0);
        *s = (*s - l.ui.input.wheel.y * 40.0).clamp(0.0, (content - view.h).max(0.0));
    }
    for (pid, what) in actions {
        match what {
            "stop" => l.state.stop(pid),
            _ => {
                if !l.state.open_logs.remove(&pid) {
                    l.state.open_logs.insert(pid);
                    l.state.log_tail(pid);
                }
            }
        }
    }
    if let Some(c) = copy {
        l.ui.clipboard_out = Some(c);
        l.state.set_status("Session code copied.", false);
    }
    let _ = hhmm;
}

// --- mods ----------------------------------------------------------------------------------------

pub fn mods(l: &mut Launcher, area: Rect) {
    if !l.state.mods_asked {
        l.state.load_mods();
    }
    let body = l.page_title(area, "Mods", "A bus, a map, scenery, a whole OMSI folder - as a folder or a .zip. The original OMSI 2 folder is never written to.");
    let cols = 3;
    let cw = (body.w - GAP * 2.0 * (cols as f32 - 1.0)) / cols as f32;
    let colr = |k: usize| Rect::new(body.x + k as f32 * (cw + GAP * 2.0), body.y, cw, body.h);
    // install
    let c0 = colr(0);
    l.ui.panel(c0);
    let inner = l.ui.heading(Rect::new(c0.x + 18.0, c0.y + 14.0, c0.w - 36.0, c0.h - 28.0), "Install a mod", Some("download"));
    let mut y = inner.y;
    let half = (inner.w - GAP) * 0.5;
    if l.ui.button("mod-folder", Rect::new(inner.x, y, half, 40.0), "Choose a folder", Some("folder_open"), ButtonKind::Primary) {
        if super::mobile::mobile() {
            l.browse(super::mobile::Purpose::ModFolder, "");
        } else if let Some(p) = core::pick_mod(false) {
            l.state.install(p.to_string_lossy().to_string());
        }
    }
    if l.ui.button("mod-zip", Rect::new(inner.x + half + GAP, y, half, 40.0), "Choose a .zip", Some("inventory_2"), ButtonKind::Normal) {
        if super::mobile::mobile() {
            l.browse(super::mobile::Purpose::ModZip, "");
        } else if let Some(p) = core::pick_mod(true) {
            l.state.install(p.to_string_lossy().to_string());
        }
    }
    y += 52.0;
    l.ui.label(Rect::new(inner.x, y, inner.w, 20.0), "A .zip archive is");
    y += 22.0;
    let mut m = l.state.mod_mode;
    if l.ui.segmented("mod-mode", Rect::new(inner.x, y, inner.w, 34.0), &mut m, &["Auto", "Unpacked", "Used in place"]) {
        l.state.mod_mode = m;
    }
    y += 44.0;
    let drop = Rect::new(inner.x, y, inner.w, 110.0);
    let hot = l.pages.drop_hover;
    let t = l.ui.anim(id_of("drop"), if hot { 1.0 } else { 0.0 }, 0.1);
    l.ui.p().rounded(drop, 12.0, ACCENT.alpha(0.05 + 0.12 * t));
    // a dashed edge
    let per = 2.0 * (drop.w + drop.h);
    let n = (per / 14.0) as usize;
    for k in 0..n {
        let s = k as f32 * per / n as f32;
        let p = if s < drop.w {
            Vec2::new(drop.x + s, drop.y)
        } else if s < drop.w + drop.h {
            Vec2::new(drop.right(), drop.y + s - drop.w)
        } else if s < 2.0 * drop.w + drop.h {
            Vec2::new(drop.right() - (s - drop.w - drop.h), drop.bottom())
        } else {
            Vec2::new(drop.x, drop.bottom() - (s - 2.0 * drop.w - drop.h))
        };
        l.ui.p().circle(p, 1.3, ACCENT.alpha(0.35 + 0.5 * t));
    }
    l.ui.icon("upload", Vec2::new(drop.center().x, drop.y + 38.0), 30.0, ACCENT.alpha(0.6 + 0.4 * t));
    l.ui.text_in("…or drop a mod folder or .zip onto this window", Rect::new(drop.x, drop.y + 62.0, drop.w, 30.0), 12.5, Weight::Medium, TEXT_SOFT, Align::Center);
    y += 122.0;
    if !l.state.mod_path.is_empty() {
        let p = l.state.mod_path.clone();
        y += l.ui.paragraph(&p, Vec2::new(inner.x, y), inner.w, 11.5, Weight::Regular, TEXT_FAINT);
        match l.state.mod_info.clone() {
            Some(Ok(i)) if i.is_zip => {
                let fit = if i.fits { format!("fits ({} free)", fmt_bytes(i.free_bytes)) } else { format!("does not fit: needs {}, {} free", fmt_bytes(i.needed_bytes), fmt_bytes(i.free_bytes)) };
                let place = if i.in_place_ok { "can be used in place".to_string() } else { format!("cannot be used in place - {}", i.in_place) };
                y += l.ui.paragraph(&format!("{} archive, {} files, {} unpacked - {fit}; {place}", fmt_bytes(i.archive_bytes), i.files, fmt_bytes(i.unpacked_bytes)), Vec2::new(inner.x, y), inner.w, 12.0, Weight::Regular, if i.fits { TEXT_DIM } else { WARN });
            }
            Some(Err(e)) => {
                y += l.ui.paragraph(&e, Vec2::new(inner.x, y), inner.w, 12.0, Weight::Regular, DANGER);
            }
            _ => {}
        }
    }
    y += 10.0;
    if let Some(m) = l.state.mods.clone() {
        l.ui.heading(Rect::new(inner.x, y, inner.w, 28.0), "The Mods folder", None);
        y += 30.0;
        y += l.ui.paragraph(&format!("Anything put into {} is installed by itself once it has finished copying.", m.inbox), Vec2::new(inner.x, y), inner.w, 12.0, Weight::Regular, TEXT_DIM);
        if !m.inbox_items.is_empty() {
            l.ui.paragraph(&format!("In it now: {}", m.inbox_items.join(", ")), Vec2::new(inner.x, y + 4.0), inner.w, 12.0, Weight::Regular, TEXT_SOFT);
        }
    }
    // installs
    let c1 = colr(1);
    l.ui.panel(c1);
    let inner = l.ui.heading(Rect::new(c1.x + 18.0, c1.y + 14.0, c1.w - 36.0, c1.h - 28.0), "Installs", Some("inventory_2"));
    if l.ui.button("jobs-clear", Rect::new(c1.right() - 18.0 - 130.0, c1.y + 12.0, 130.0, 30.0), "Clear finished", None, ButtonKind::Ghost) {
        core::install::clear_finished();
        l.state.poll_now();
    }
    let jobs = l.state.jobs.clone();
    let mut cancel = None;
    l.ui.scroll_area("jobs", Rect::new(inner.x - 6.0, inner.y, inner.w + 12.0, inner.h), &mut |ui, v| {
        if jobs.is_empty() {
            ui.paragraph("Nothing installed since the launcher started. Big archives are checked against the free disk space before anything is unpacked; a cancelled or failed install leaves nothing behind.", Vec2::new(v.x + 6.0, v.y), v.w - 12.0, 12.5, Weight::Regular, TEXT_DIM);
            return 60.0;
        }
        let mut y = v.y;
        for j in &jobs {
            let running = j.finished.is_none();
            let msg_h = ui.paragraph_height(&j.message, v.w - 40.0, 12.0, Weight::Regular);
            let h = 50.0 + msg_h + if running { 44.0 } else { 0.0 } + j.warnings.len() as f32 * 18.0;
            let r = Rect::new(v.x + 6.0, y, v.w - 16.0, h);
            ui.p().rounded(r, 10.0, Color::WHITE.alpha(0.04));
            ui.text_in(&j.name, Rect::new(r.x + 12.0, r.y + 8.0, r.w - 120.0, 20.0), 13.5, Weight::Bold, TEXT, Align::Left);
            let sc = match j.state.as_str() {
                "done" => OK,
                "failed" => DANGER,
                "cancelled" => TEXT_DIM,
                _ => ACCENT,
            };
            ui.badge(Vec2::new(r.right() - 90.0, r.y + 10.0), &j.state.to_uppercase(), sc);
            let mut yy = r.y + 34.0;
            if running {
                let frac = if j.bytes_total > 0 { j.bytes_done as f32 / j.bytes_total as f32 } else if j.files_total > 0 { j.files_done as f32 / j.files_total as f32 } else { 0.0 };
                ui.progress(Rect::new(r.x + 12.0, yy, r.w - 24.0, 8.0), frac, true);
                ui.text_in(&format!("{} / {} files · {} / {}", j.files_done, j.files_total, fmt_bytes(j.bytes_done), fmt_bytes(j.bytes_total)), Rect::new(r.x + 12.0, yy + 10.0, r.w - 24.0, 16.0), 11.0, Weight::Regular, TEXT_DIM, Align::Left);
                yy += 30.0;
            }
            yy += ui.paragraph(&j.message, Vec2::new(r.x + 12.0, yy), r.w - 24.0, 12.0, Weight::Regular, if j.state == "failed" { DANGER } else { TEXT_SOFT });
            for w in &j.warnings {
                ui.text_in(&format!("⚠ {w}"), Rect::new(r.x + 12.0, yy, r.w - 24.0, 16.0), 11.0, Weight::Regular, WARN, Align::Left);
                yy += 18.0;
            }
            if running && ui.button(&format!("cancel-{}", j.id), Rect::new(r.x + 12.0, r.bottom() - 34.0, 100.0, 28.0), "Cancel", None, ButtonKind::Danger) {
                cancel = Some(j.id);
            }
            y += h + 10.0;
        }
        y - v.y
    });
    if let Some(id) = cancel {
        core::install::cancel(id);
        l.state.poll_now();
    }
    // content folder
    let c2 = colr(2);
    l.ui.panel(c2);
    let inner = l.ui.heading(Rect::new(c2.x + 18.0, c2.y + 14.0, c2.w - 36.0, c2.h - 28.0), "Content folder", Some("folder_open"));
    let Some(m) = l.state.mods.clone() else {
        l.ui.text_in("Reading…", Rect::new(inner.x, inner.y, inner.w, 20.0), 12.5, Weight::Regular, TEXT_DIM, Align::Left);
        return;
    };
    let mut y = inner.y;
    y += l.ui.paragraph(&m.content_dir, Vec2::new(inner.x, y), inner.w, 12.0, Weight::Medium, TEXT_SOFT);
    y += 6.0;
    l.ui.text_in(&format!("{} free on this disk", fmt_bytes(m.free_bytes)), Rect::new(inner.x, y, inner.w, 20.0), 13.0, Weight::Bold, ACCENT, Align::Left);
    y += 30.0;
    for (f, n) in &m.folders {
        l.ui.icon("folder_open", Vec2::new(inner.x + 9.0, y + 10.0), 16.0, TEXT_DIM);
        l.ui.text_in(f, Rect::new(inner.x + 26.0, y, inner.w * 0.6, 20.0), 12.5, Weight::Medium, TEXT, Align::Left);
        l.ui.text_in(&format!("{n} {}", if *n == 1 { "entry" } else { "entries" }), Rect::new(inner.x, y, inner.w, 20.0), 12.0, Weight::Regular, TEXT_DIM, Align::Right);
        y += 24.0;
    }
    if !m.archives.is_empty() {
        y += 8.0;
        l.ui.heading(Rect::new(inner.x, y, inner.w, 28.0), "Archives used in place", None);
        y += 30.0;
        for (n, b) in &m.archives {
            l.ui.text_in(&format!("{n}  ({})", fmt_bytes(*b)), Rect::new(inner.x, y, inner.w, 20.0), 12.0, Weight::Regular, TEXT_SOFT, Align::Left);
            y += 22.0;
        }
    }
    if !m.waiting.is_empty() {
        y += 8.0;
        l.ui.heading(Rect::new(inner.x, y, inner.w, 28.0), "Waiting for their bus", None);
        y += 30.0;
        for w in &m.waiting {
            l.ui.text_in(w, Rect::new(inner.x, y, inner.w, 20.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
            y += 22.0;
        }
    }
}

// --- setup -----------------------------------------------------------------------------------------

pub fn setup(l: &mut Launcher, area: Rect) {
    let body = l.page_title(area, "Setup", "Where the original game and this one are.");
    let r = Rect::new(body.x, body.y, body.w.min(820.0), 300.0);
    l.ui.panel(r);
    let inner = l.ui.heading(Rect::new(r.x + 20.0, r.y + 16.0, r.w - 40.0, r.h - 32.0), "Folders", Some("folder_open"));
    let mut root = l.pages.setup_root.clone().unwrap_or_else(|| l.state.config.root.clone());
    let mut game = l.pages.setup_game.clone().unwrap_or_else(|| l.state.config.game.clone());
    let mut y = inner.y;
    l.ui.label(Rect::new(inner.x, y, 150.0, ROW), "OMSI 2 folder");
    if l.ui.text_input("cfg-root", Rect::new(inner.x + 150.0, y, inner.w - 150.0 - 110.0, ROW), &mut root, "/path/to/OMSI 2", Some("folder_open")) {
        l.pages.setup_root = Some(root.clone());
    }
    if l.ui.button("browse-root", Rect::new(inner.right() - 100.0, y, 100.0, ROW), "Browse", None, ButtonKind::Normal) {
        if super::mobile::mobile() {
            let start = root.clone();
            l.browse(super::mobile::Purpose::Root, &start);
        } else if let Some(p) = core::pick_folder("The OMSI 2 folder (with maps and Vehicles in it)") {
            l.pages.setup_root = Some(p.to_string_lossy().to_string());
        }
    }
    y += ROW + 10.0;
    if core::IN_PROCESS_GAMES {
        // (a phone: the game is this app itself)
        y += l.ui.paragraph("Copy the whole OMSI 2 folder (with maps and Vehicles in it) onto the phone - by cable, from a PC or a USB stick - for example as openOMSI/OMSI 2 in the internal storage, then choose it here with Browse. Mods go into openOMSI/Mods or are installed from the Mods page.", Vec2::new(inner.x, y), inner.w, 12.5, Weight::Regular, TEXT_DIM);
    } else {
        l.ui.label(Rect::new(inner.x, y, 150.0, ROW), "Game binary");
        if l.ui.text_input("cfg-game", Rect::new(inner.x + 150.0, y, inner.w - 150.0 - 110.0, ROW), &mut game, "openomsi", Some("terminal")) {
            l.pages.setup_game = Some(game.clone());
        }
        if l.ui.button("browse-game", Rect::new(inner.right() - 100.0, y, 100.0, ROW), "Browse", None, ButtonKind::Normal) {
            if let Some(p) = core::pick_file("The openomsi program") {
                l.pages.setup_game = Some(p.to_string_lossy().to_string());
            }
        }
        y += ROW + 16.0;
        y += l.ui.paragraph("The OMSI 2 folder is the one with maps and Vehicles in it (any complete installation). The game binary is the openomsi program; it is found by itself when it sits next to the launcher.", Vec2::new(inner.x, y), inner.w, 12.5, Weight::Regular, TEXT_DIM);
    }
    y += 12.0;
    if l.ui.button("cfg-save", Rect::new(inner.x, y, 180.0, 42.0), "Save", Some("save"), ButtonKind::Primary) {
        l.state.config.root = root.trim().to_string();
        l.state.config.game = game.trim().to_string();
        match core::save_config(&l.state.config) {
            Ok(()) => {
                // (what was read of the folders before is forgotten: a folder copied or
                // changed while the launcher ran is read as it is now)
                omsi_cfg::content_changed();
                l.state.config = core::load_config();
                l.pages.setup_root = None;
                l.pages.setup_game = None;
                l.state.set_status("Saved. Reading the content again…", false);
                l.state.load_content();
            }
            Err(e) => l.state.set_status(format!("{e:#}"), true),
        }
    }
}


// --- tutorials --------------------------------------------------------------------------------

/// OMSI's four tutorials: what each teaches, and a button to start it.
pub fn tutorials(l: &mut Launcher, area: Rect) {
    let body = l.page_title(area, "Tutorials", "OMSI 2's own lessons: each opens its situation, with its pages beside the picture (Enter turns the page).");
    static LIST: std::sync::OnceLock<Vec<(usize, String, String)>> = std::sync::OnceLock::new();
    let list = LIST.get_or_init(omsi_launcher_lib::tutorials);
    if list.is_empty() {
        l.ui.paragraph("No tutorials were found in the OMSI 2 folder (Tutorials).", Vec2::new(body.x, body.y), body.w, 13.0, Weight::Regular, TEXT_DIM);
        return;
    }
    let cols = 2;
    let cw = (body.w - GAP * 2.0) / cols as f32;
    let ch = ((body.h - GAP * 2.0) / 2.0).min(330.0);
    let mut start = None;
    for (k, (n, title, text)) in list.iter().enumerate() {
        let r = Rect::new(body.x + (k % cols) as f32 * (cw + GAP * 2.0), body.y + (k / cols) as f32 * (ch + GAP * 2.0), cw, ch);
        l.ui.panel(r);
        l.ui.text_in(title, Rect::new(r.x + 18.0, r.y + 14.0, r.w - 36.0, 26.0), 17.0, Weight::Bold, TEXT, Align::Left);
        l.ui.push_clip(Rect::new(r.x + 18.0, r.y + 46.0, r.w - 36.0, r.h - 110.0), 0.0);
        l.ui.paragraph(text, Vec2::new(r.x + 18.0, r.y + 46.0), r.w - 36.0, 12.5, Weight::Regular, TEXT_DIM);
        l.ui.pop_clip();
        if l.ui.button(&format!("tut-{n}"), Rect::new(r.x + 18.0, r.bottom() - 58.0, 200.0, 40.0), "Start the lesson", Some("play_arrow"), ButtonKind::Primary) {
            start = Some(*n);
        }
    }
    if let Some(n) = start {
        l.state.launch_tutorial(n);
    }
}

#[cfg(test)]
mod wizard_tests {
    use crate::controllers::Func;

    #[test]
    fn h_pattern_gears_have_a_clear_name() {
        assert_eq!(super::action_label("kw_s_1_fest"), "Gear 1 (H-pattern)");
        assert_eq!(super::action_label("kw_s_R_fest"), "Gear R (H-pattern)");
    }

    #[test]
    fn empty_saved_button_slots_do_not_fill_the_controller_list() {
        let mut buttons = vec![(String::new(), "0".to_string()); 131];
        buttons[10].0 = "horn".to_string();
        assert_eq!(super::shown_button_count(&buttons, 18, None), 18);
        assert_eq!(super::shown_button_count(&buttons, 18, Some(128)), 129);
    }

    #[test]
    fn a_wheel_with_three_pedals() {
        // X the wheel; Y throttle, Z brake, Rz clutch - pedals reading 1 up, -1 down (as the
        // G25's run, "reversed")
        let rest = [Some(0.0), Some(1.0), Some(1.0), None, None, Some(1.0), None, None];
        let mut left = rest;
        left[0] = Some(-1.0);
        let mut thr = rest;
        thr[1] = Some(-1.0);
        let mut brk = rest;
        brk[2] = Some(-1.0);
        let mut clu = rest;
        clu[5] = Some(-1.0);
        let a = super::wizard_result(&rest, &[left, thr, brk, clu]);
        assert_eq!(a[0], Some((Func::Steering, false)));
        assert_eq!(a[1], Some((Func::Throttle, true)));
        assert_eq!(a[2], Some((Func::Brake, true)));
        assert_eq!(a[5], Some((Func::Clutch, true)));
    }

    #[test]
    fn pedals_on_one_axis_and_a_wheel_the_other_way() {
        let rest = [Some(0.0), Some(0.0), None, None, None, None, None, None];
        let a = super::wizard_result(&rest, &[[Some(0.9), Some(0.0), None, None, None, None, None, None], [Some(0.0), Some(-1.0), None, None, None, None, None, None], [Some(0.0), Some(1.0), None, None, None, None, None, None], [None; 8]]);
        assert_eq!(a[0], Some((Func::Steering, true)));
        assert_eq!(a[1], Some((Func::ThrottleBrake, false)));
    }
}
