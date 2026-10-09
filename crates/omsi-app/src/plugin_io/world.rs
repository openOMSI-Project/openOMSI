//! Time, weather, camera, input, sound, the game and the LAN for the plugin API.

use crate::App;
use omsi_plugin as op;
use op::InfoValue::{Bool, Num, Text};

fn lan_client(app: &App) -> bool {
    app.net.lan.as_ref().is_some_and(|l| l.role == omsi_net::Role::Client)
}

pub(crate) fn clock(app: &App) -> op::Clock {
    let c = &app.clock;
    let (day, month) = c.day_month();
    op::Clock { time: c.time, year: c.year, month: month as u32, day: day as u32, day_of_year: c.day_of_year, weekday: c.weekday() as i32, run_time: c.run_time }
}

pub(crate) fn set_time(app: &mut App, t: f64) -> bool {
    if lan_client(app) || app.real_time_locked() {
        return false;
    }
    let now = app.clock.time;
    app.shift_clock(t - now);
    true
}

pub(crate) fn set_date(app: &mut App, y: i32, m: u32, d: u32) -> bool {
    if lan_client(app) || app.real_time_locked() {
        return false;
    }
    app.clock.set_date(y, m as i32, d as i32);
    if let Some(p) = app.player.as_mut() {
        p.vehicle.host.clock = app.clock.clone();
    }
    app.follow_date();
    true
}

pub(crate) fn weather(app: &App) -> Option<op::Weather> {
    let w = app.session.weather.as_ref()?;
    let (kind, rate) = crate::weather_setup::precip_of(w);
    Some(op::Weather {
        name: w.name.trim().to_string(),
        visibility_m: w.fog.0,
        wind_dir: w.wind.0,
        wind_ms: w.wind.1,
        temperature: w.temp.0,
        humidity_abs: w.temp.1,
        humidity_rel: crate::weather_setup::relative_humidity(w.temp.0, w.temp.1),
        pressure: w.pressure,
        clouds: w.clouds.0.trim().to_string(),
        cloud_base_m: w.clouds.1,
        precip_kind: kind,
        precip_rate: rate,
        snow_cover: w.snow,
        snow_on_road: w.snow_on_road,
        wetness: app.session.wetness,
        changing: app.session.weather_blend.is_some(),
        locked: app.metar_locked(),
    })
}

/// The weather changed as the game menu's sliders change it (`edit_weather`).
pub(crate) fn set_weather(app: &mut App, c: &op::WeatherChange) -> Result<(), String> {
    if lan_client(app) {
        return Err("in a LAN session the host sets the weather".into());
    }
    if app.metar_locked() {
        return Err("the weather follows a weather station (METAR sync)".into());
    }
    let c = c.clone();
    let any = c.visibility_m.is_some() || c.wind_dir.is_some() || c.wind_ms.is_some() || c.temperature.is_some() || c.pressure.is_some() || c.clouds.is_some() || c.cloud_base_m.is_some() || c.precip_kind.is_some() || c.precip_rate.is_some() || c.snow_cover.is_some() || c.snow_on_road.is_some();
    if any {
        app.edit_weather(|w| {
            if let Some(v) = c.visibility_m {
                w.fog.0 = v;
            }
            if let Some(v) = c.wind_dir {
                w.wind.0 = v.rem_euclid(360.0);
            }
            if let Some(v) = c.wind_ms {
                w.wind.1 = v;
            }
            if let Some(v) = c.temperature {
                w.temp.0 = v;
            }
            if let Some(v) = c.pressure {
                w.pressure = v;
            }
            if let Some(v) = c.clouds.clone() {
                w.clouds.0 = v;
            }
            if let Some(v) = c.cloud_base_m {
                w.clouds.1 = v;
            }
            if let Some(k) = c.precip_kind {
                w.precip[0] = k.clamp(0, 2) as f32;
                // (rain asked for without a rate: a steady one)
                if k > 0 && w.precip[1] <= 0.0 && c.precip_rate.is_none() {
                    w.precip[1] = 128.0;
                }
            }
            if let Some(r) = c.precip_rate {
                w.precip[1] = (r.clamp(0.0, 1.0) * 255.0).round();
            }
            if let Some(v) = c.snow_cover {
                w.snow = v;
            }
            if let Some(v) = c.snow_on_road {
                w.snow_on_road = v;
            }
        });
    }
    if let Some(v) = c.wetness {
        app.session.wetness = v.clamp(0.0, 1.0);
    }
    Ok(())
}

pub(crate) fn presets() -> Vec<(String, String)> {
    crate::weather_cycle::installed()
        .into_iter()
        .map(|(file, w)| {
            let name = if w.name.trim().is_empty() { file.rsplit('/').next().unwrap_or(&file).trim_end_matches(".owt").to_string() } else { w.name.trim().to_string() };
            (file, name)
        })
        .collect()
}

pub(crate) fn set_preset(app: &mut App, file: Option<&str>, secs: f32) -> Result<(), String> {
    if lan_client(app) {
        return Err("in a LAN session the host sets the weather".into());
    }
    if app.metar_locked() {
        return Err("the weather follows a weather station (METAR sync)".into());
    }
    if let Some(f) = file {
        if !presets().iter().any(|(p, _)| p.eq_ignore_ascii_case(f)) {
            return Err(format!("no weather file \"{f}\" (see weather.presets)"));
        }
    }
    app.change_weather(file.map(str::to_string), true, secs);
    Ok(())
}

// --- the camera --------------------------------------------------------------------------

pub(crate) fn camera(app: &App) -> Option<op::Camera> {
    let c = app.camera.as_ref()?;
    let (width, height) = app.gfx.surface.as_ref().map(|s| (s.config.width, s.config.height)).or_else(|| app.window.as_ref().map(|w| (w.inner_size().width, w.inner_size().height))).unwrap_or((0, 0));
    Some(op::Camera {
        view: app.view.clone(),
        pos: [c.position.x, c.position.y, c.position.z],
        yaw: c.yaw,
        pitch: c.pitch,
        roll: c.roll,
        fov: c.fov_deg,
        in_cab: app.cam.in_cab,
        zoom: app.cam.view_zoom.get(&app.view).copied().unwrap_or(1.0),
        look: app.cam.look,
        width,
        height,
    })
}

pub(crate) fn set_view(app: &mut App, name: &str) -> bool {
    let action = match name {
        "driver" => "view_set_driver",
        "pax" | "passenger" => "view_set_passenger",
        "outside" => "view_set_outside",
        "map" | "free" => "view_set_map",
        "ego" | "walk" => "view_set_ego",
        n if n.starts_with("view_") => n,
        _ => return false,
    };
    app.game_action(action)
}

pub(crate) fn set_free_camera(app: &mut App, at: [f64; 3], yaw: f32, pitch: f32) -> bool {
    if !at.iter().all(|v| v.is_finite()) || app.camera.is_none() {
        return false;
    }
    app.view = "free".into();
    app.cam.ego = false;
    if let Some(c) = app.camera.as_mut() {
        c.position = glam::DVec3::new(at[0], at[1], at[2]);
        c.yaw = yaw.rem_euclid(360.0);
        c.pitch = pitch;
    }
    true
}

// --- input -------------------------------------------------------------------------------

pub(crate) fn key_held(app: &App, name: &str) -> bool {
    app.input.keys.iter().any(|k| format!("{k:?}") == name)
}

pub(crate) fn keys_held(app: &App) -> Vec<String> {
    let mut v: Vec<String> = app.input.keys.iter().map(|k| format!("{k:?}")).collect();
    v.sort();
    v
}

pub(crate) fn controllers(app: &App) -> Vec<op::Controller> {
    let Some(c) = app.input.controllers.as_ref() else { return Vec::new() };
    c.connected()
        .into_iter()
        .map(|d| {
            let n = d.axes.iter().map(|a| a.0 + 1).max().unwrap_or(0);
            let mut axes = vec![0.0; n];
            for (i, v) in d.axes {
                axes[i] = v;
            }
            op::Controller { name: d.name, gamepad: d.gamepad, axes, buttons: d.buttons }
        })
        .collect()
}

pub(crate) fn bindings(app: &App, vehicle: bool) -> Vec<(String, String)> {
    let list: &[omsi_content::KeyBinding] = if vehicle { app.player.as_ref().map(|p| p.bindings.as_slice()).unwrap_or(&[]) } else { &app.input.game_keys };
    list.iter().map(|b| (b.action.clone(), crate::keys::key_name(b.scan_code as i64, b.modifier as i64))).collect()
}

// --- sound -------------------------------------------------------------------------------

fn params(app: &App, s: &op::Sound) -> omsi_audio::VoiceParams {
    let at = if s.on_bus { app.player.as_ref().map(|p| p.vehicle.position + glam::DVec3::Z * 1.5) } else { s.at.map(|a| glam::DVec3::new(a[0], a[1], a[2])) };
    omsi_audio::VoiceParams { gain: s.volume, pitch: s.pitch, looping: s.looping, position: at.map(|p| p.as_vec3()), range: s.range, ..Default::default() }
}

pub(crate) fn sound_play(app: &mut App, file: &std::path::Path, s: &op::Sound) -> Option<u64> {
    let a = app.sound.audio.as_ref()?;
    let clip = a.load_clip(file)?;
    let id = a.play(clip, params(app, s));
    app.integrations.plugin_voices.push((id, s.on_bus));
    Some(id)
}

pub(crate) fn sound_set(app: &mut App, id: u64, s: &op::Sound) -> bool {
    let Some(a) = app.sound.audio.as_ref() else { return false };
    if !a.is_playing(id) {
        return false;
    }
    a.set_params(id, params(app, s));
    if let Some(v) = app.integrations.plugin_voices.iter_mut().find(|v| v.0 == id) {
        v.1 = s.on_bus;
    }
    true
}

/// The plugins' sounds that move with the bus follow it; those over are forgotten.
pub(crate) fn follow_sounds(app: &mut App) {
    if app.integrations.plugin_voices.is_empty() {
        return;
    }
    let Some(a) = app.sound.audio.as_ref() else { return };
    let bus = app.player.as_ref().map(|p| (p.vehicle.position + glam::DVec3::Z * 1.5).as_vec3());
    app.integrations.plugin_voices.retain(|(id, on_bus)| {
        if !a.is_playing(*id) {
            return false;
        }
        if let (true, Some(at), Some((mut p, _))) = (*on_bus, bus, a.voice_state(*id)) {
            p.position = Some(at);
            a.set_params(*id, p);
        }
        true
    });
}

// --- the game ----------------------------------------------------------------------------

pub(crate) fn settings(app: &App) -> Vec<(&'static str, op::InfoValue)> {
    let s = &app.settings;
    vec![
        ("graphics", Text(s.graphics.clone())),
        ("enhanced", Bool(s.enhanced)),
        ("language", Text(s.language.clone())),
        ("ui_language", Text(omsi_ui::i18n::language())),
        ("ui_scale", Num(s.ui_scale as f64)),
        ("volume", Num(s.volume as f64)),
        ("fov", Num(s.fov as f64)),
        ("render_scale", Num(s.render_scale as f64)),
        ("max_fps", Num(s.max_fps as f64)),
        ("fullscreen", Bool(s.fullscreen)),
        ("vsync", Bool(s.vsync)),
        ("msaa", Num(s.msaa as f64)),
        ("shadows", Bool(s.shadows)),
        ("time_speed", Num(app.time_speed())),
        ("time_sync", Bool(s.time_sync)),
        ("people", Num(s.pax_density as f64)),
        ("units", Text("metric".into())),
        ("mobile", Bool(crate::platform::MOBILE)),
        ("vr", Bool(app.vr_active())),
    ]
}

pub(crate) fn stats(app: &App) -> Vec<(&'static str, op::InfoValue)> {
    let c = &app.session.career;
    let mut v = vec![
        ("km", Num(c.metres / 1000.0)),
        ("stops_served", Num(c.stops[0] as f64)),
        ("stops_late", Num(c.stops[1] as f64)),
        ("stops_skipped", Num(c.stops[2] as f64)),
        ("crashes", Num(c.crashes[0] as f64)),
        ("pedestrians", Num(c.crashes[1] as f64)),
        ("heavy_crashes", Num(c.crashes[3] as f64)),
        ("tickets", Num(c.tickets.0 as f64)),
        ("cash", Num(c.tickets.1)),
        ("play_time", Num(app.clock.run_time)),
    ];
    if let Some(h) = app.session.humans.as_ref() {
        v.push(("passengers", Num(h.stepped_in as f64)));
    }
    v
}

pub(crate) fn notify(app: &mut App, text: &str, kind: u8, secs: f32) -> bool {
    let kind = match kind {
        1 => crate::ui::NoticeKind::Warn,
        2 => crate::ui::NoticeKind::Alert,
        _ => crate::ui::NoticeKind::Info,
    };
    let text: String = text.chars().take(300).collect();
    crate::ui::push_notice(&mut app.menus.notices, crate::ui::Notice { kind, text, left: secs, total: secs });
    true
}

// --- the LAN -----------------------------------------------------------------------------

pub(crate) fn lan(app: &App) -> Option<(bool, u32, String)> {
    let l = app.net.lan.as_ref()?;
    Some((l.role == omsi_net::Role::Host, l.my_id, l.my_name.clone()))
}

pub(crate) fn lan_players(app: &App) -> Vec<op::LanPlayer> {
    let Some(l) = app.net.lan.as_ref() else { return Vec::new() };
    l.peers()
        .filter(|p| p.has_info)
        .map(|p| {
            let s = &p.pose;
            let (pos, on_foot) = match (&s.walker, s.has_vehicle()) {
                (Some(w), false) => ([w.x, w.y, w.z, w.heading as f64], true),
                _ => ([s.x, s.y, s.z, s.heading as f64], false),
            };
            op::LanPlayer { id: s.id, name: s.name.clone(), host: s.id == 1, bus: s.bus.clone(), line: s.line.clone(), tour: s.tour.clone(), pos, speed_kmh: s.speed_kmh, on_foot, passengers: s.passengers as u32 }
        })
        .collect()
}

/// The prefix of a plugin's message among the session's commands: `plugin <name> <text>`.
pub(crate) const LAN_PREFIX: &str = "plugin ";

pub(crate) fn lan_send(app: &mut App, plugin: &str, to: u32, text: &str) -> Result<(), String> {
    let l = app.net.lan.as_mut().ok_or("no LAN session")?;
    if to == l.my_id {
        return Err("that is this player".into());
    }
    if !l.peers().any(|p| p.pose.id == to) {
        return Err(format!("no player {to}"));
    }
    if plugin.contains(char::is_whitespace) {
        return Err("a plugin whose name has spaces cannot send".into());
    }
    l.command(to, &format!("{LAN_PREFIX}{plugin} {text}"));
    Ok(())
}

pub(crate) fn lan_chat(app: &mut App, text: &str) -> Result<(), String> {
    let l = app.net.lan.as_mut().ok_or("no LAN session")?;
    // (the chat's own filter, as for the player's lines; never a `/` command)
    let text = crate::ui::filter_chat(text.trim_start_matches('/'));
    if text.trim().is_empty() {
        return Err("nothing to say".into());
    }
    l.say(&text)
}
