//! `camera.*`, `input.*` and `audio.*`: what the player sees, presses and hears.

use super::{def, multi, nums, o, p, rec};
use crate::api::runtime::{self, Hotkey};
use crate::api::{paths, ApiError, ApiFn, Ctx, Value};
use crate::io::Sound;

const NEW: &str = crate::api::VERSION;
/// Most sounds of one plugin playing at once.
const MAX_VOICES: usize = 32;

/// The point on the screen (the panels' pixels) a map point is seen at, from the camera;
/// None behind it.
fn project(c: &mut Ctx<'_>, at: [f64; 3]) -> Option<[f64; 2]> {
    let cam = c.io().camera()?;
    let [sw, sh, _] = c.state().ui().screen();
    let (yaw, pitch) = ((cam.yaw as f64).to_radians(), (cam.pitch as f64).to_radians());
    // the camera's axes: yaw 0 looks north (+y), clockwise; pitch up positive
    let fwd = [yaw.sin() * pitch.cos(), yaw.cos() * pitch.cos(), pitch.sin()];
    let right = [yaw.cos(), -yaw.sin(), 0.0];
    let up = [-yaw.sin() * pitch.sin(), -yaw.cos() * pitch.sin(), pitch.cos()];
    let d = [at[0] - cam.pos[0], at[1] - cam.pos[1], at[2] - cam.pos[2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let z = dot(d, fwd);
    if z <= 0.05 {
        return None;
    }
    let f = 1.0 / ((cam.fov as f64).to_radians() * 0.5).tan();
    let aspect = sw as f64 / (sh as f64).max(1.0);
    let x = dot(d, right) / z * f / aspect;
    let y = dot(d, up) / z * f;
    Some([(x + 1.0) * 0.5 * sw as f64, (1.0 - y) * 0.5 * sh as f64])
}

/// `audio.play`'s options.
fn sound_opts(c: &mut Ctx<'_>, t: Option<&Value>) -> Result<Sound, ApiError> {
    let mut s = Sound::default();
    let Some(t) = t else { return Ok(s) };
    let num = |k: &str| t.get(k).and_then(Value::as_f64);
    if let Some(v) = num("volume") {
        s.volume = v.clamp(0.0, 4.0) as f32;
    }
    if let Some(v) = num("pitch") {
        s.pitch = v.clamp(0.1, 4.0) as f32;
    }
    if let Some(v) = num("range") {
        s.range = v.clamp(0.1, 1000.0) as f32;
    }
    s.looping = t.get("loop").is_some_and(Value::truthy);
    match (num("x"), num("y")) {
        (Some(x), Some(y)) => s.at = Some([x, y, num("z").unwrap_or(0.0)]),
        _ if t.get("on_bus").is_some_and(Value::truthy) => {
            s.on_bus = true;
            s.at = c.io().position().map(|p| [p[0], p[1], p[2] + 1.5]);
        }
        _ => {}
    }
    Ok(s)
}

/// A key combination: `"F10"`, `"Ctrl+KeyH"`, `"Shift+Alt+Digit1"`.
fn hotkey(spec: &str, cb: u64) -> Result<Hotkey, ApiError> {
    let mut h = Hotkey { key: String::new(), ctrl: false, shift: false, alt: false, cb };
    for part in spec.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => h.ctrl = true,
            "shift" => h.shift = true,
            "alt" => h.alt = true,
            _ if h.key.is_empty() && !part.is_empty() => h.key = part.to_string(),
            _ => return Err(ApiError(format!("omsi.input.hotkey: \"{spec}\" is no key combination (\"Ctrl+KeyH\")"))),
        }
    }
    if h.key.is_empty() {
        return Err(ApiError(format!("omsi.input.hotkey: \"{spec}\" has no key")));
    }
    Ok(h)
}

pub static FNS: &[ApiFn] = &[
    def!("camera.get", "camera", [], "table or nil", "The camera: `{view, x, y, z, yaw, pitch, roll, fov, in_cab, zoom, look_yaw, look_pitch, width, height}` - map metres, degrees (yaw clockwise from north, pitch up positive), the vertical field of view, the picture's pixels.", NEW, None, false, |c, a| {
        c.io().camera().map(|k| rec(vec![
            ("view", k.view.into()),
            ("x", k.pos[0].into()),
            ("y", k.pos[1].into()),
            ("z", k.pos[2].into()),
            ("yaw", Value::from(k.yaw)),
            ("pitch", Value::from(k.pitch)),
            ("roll", Value::from(k.roll)),
            ("fov", Value::from(k.fov)),
            ("in_cab", k.in_cab.into()),
            ("zoom", Value::from(k.zoom)),
            ("look_yaw", Value::from(k.look.0)),
            ("look_pitch", Value::from(k.look.1)),
            ("width", k.width.into()),
            ("height", k.height.into()),
        ]))
    }),
    def!("camera.view", "camera", [], "string or nil", "The view: `\"driver\"`, `\"pax\"`, `\"outside\"`, `\"free\"` or `\"foot\"`.", NEW, None, false, |c, a| c.io().camera().map(|k| k.view)),
    def!("camera.position", "camera", [], "x, y, z", "Where the camera is (map metres).", NEW, None, true, |c, a| nums(c.io().camera().map(|k| k.pos))),
    def!("camera.orientation", "camera", [], "yaw, pitch, roll", "Where it looks, degrees.", NEW, None, true, |c, a| nums(c.io().camera().map(|k| [k.yaw as f64, k.pitch as f64, k.roll as f64]))),
    def!("camera.fov", "camera", [], "number or nil", "Its vertical field of view, degrees.", NEW, None, false, |c, a| c.io().camera().map(|k| k.fov as f64)),
    def!("camera.in_cab", "camera", [], "boolean", "Whether the camera is in the player's own bus (driver or passenger view).", NEW, None, false, |c, a| c.io().camera().is_some_and(|k| k.in_cab)),
    def!("camera.set_view", "camera", [p("view", "string")], "boolean", "Switches the view: `\"driver\"`, `\"pax\"`, `\"outside\"`, `\"map\"` (the free camera above the bus), `\"ego\"` (walking), or a `view_*` action of keyboard.cfg.", NEW, Camera, false, |c, a| {
        let v = a.str(0)?;
        Ok::<_, ApiError>(c.io().set_view(&v))
    }),
    def!("camera.set_free", "camera", [p("x", "number"), p("y", "number"), p("z", "number"), o("yaw", "number"), o("pitch", "number")], "boolean", "Puts the free camera at a map point looking along `yaw` and `pitch` (degrees); the view becomes `\"free\"` (the player moves it on from there).", NEW, Camera, false, |c, a| {
        let at = [a.num(0)?, a.num(1)?, a.num(2)?];
        let (yaw, pitch) = (a.opt_num(3)?.unwrap_or(0.0) as f32, a.opt_num(4)?.unwrap_or(0.0).clamp(-89.0, 89.0) as f32);
        Ok::<_, ApiError>(c.io().set_free_camera(at, yaw, pitch))
    }),
    def!("camera.set_zoom", "camera", [p("zoom", "number")], "boolean", "The zoom of the view now (its field of view times this, 0.2 to 3).", NEW, Camera, false, |c, a| {
        let k = a.num(0)?.clamp(0.2, 3.0) as f32;
        Ok::<_, ApiError>(c.io().set_zoom(k))
    }),
    def!("camera.look", "camera", [p("yaw", "number"), p("pitch", "number")], "boolean", "Turns the head (driver, passenger view) or swings the outside camera round the bus: degrees from straight ahead.", NEW, Camera, false, |c, a| {
        let (y, p) = (a.num(0)? as f32, a.num(1)?.clamp(-89.0, 89.0) as f32);
        Ok::<_, ApiError>(c.io().set_look(y, p))
    }),
    def!("camera.project", "camera", [p("x", "number"), p("y", "number"), p("z", "number")], "screen_x, screen_y", "Where a map point is seen on the screen, in the panels' pixels (as `ui.screen` measures them); nothing when it is behind the camera. For labels over buses, stops, people.", NEW, None, true, |c, a| {
        let at = [a.num(0)?, a.num(1)?, a.num(2)?];
        Ok::<_, ApiError>(nums(project(c, at)))
    }),
    def!("input.key_down", "input", [p("key", "string")], "boolean", "Whether a key is held now (winit's names, as the `key` event: `\"KeyW\"`, `\"ShiftLeft\"`, `\"F5\"`).", NEW, None, false, |c, a| {
        let k = a.str(0)?;
        Ok::<_, ApiError>(c.io().key_held(&k))
    }),
    def!("input.keys_down", "input", [], "list of strings", "Every key held now.", NEW, None, false, |c, a| c.io().keys_held()),
    def!("input.mouse", "input", [], "x, y, left, right, middle", "The mouse: where it is in the panels' pixels and its buttons held.", NEW, None, true, |c, a| {
        let m = c.io().mouse();
        let k = c.state().ui().screen()[2].max(0.01) as f64;
        multi(m.map(|m| [Value::Num(m.x as f64 / k), Value::Num(m.y as f64 / k), Value::Bool(m.left), Value::Bool(m.right), Value::Bool(m.middle)]))
    }),
    def!("input.controllers", "input", [], "list of tables", "The steering wheels, pedals, joysticks and gamepads: `{name, gamepad, axes, buttons}` (`axes` each axis's value; `buttons` how many it has - the `controller_button` event tells presses).", NEW, None, false, |c, a| {
        Value::List(c.io().controllers().into_iter().map(|k| rec(vec![("name", k.name.into()), ("gamepad", k.gamepad.into()), ("axes", Value::List(k.axes.into_iter().map(|x| Value::from(x)).collect())), ("buttons", k.buttons.into())])).collect())
    }),
    def!("input.bindings", "input", [o("vehicles", "bool")], "list of tables", "The key bindings: `{action, key}` of the game's keys, or the vehicles' with `true` (`key` as the game writes it: `\"Ctrl+D\"`).", NEW, None, false, |c, a| {
        let v = a.flag(0, false);
        Value::List(c.io().bindings(v).into_iter().map(|(act, k)| rec(vec![("action", act.into()), ("key", k.into())])).collect())
    }),
    def!("input.hotkey", "input", [p("keys", "string"), p("fn", "function")], "integer id", "Runs `fn(key)` when a key combination is pressed: `\"F10\"`, `\"Ctrl+KeyH\"`, `\"Shift+Alt+Digit1\"` (modifiers exact: `\"KeyH\"` is not `Ctrl+KeyH`). The keys still reach the bus. `cancel(id)` removes it.", NEW, None, false, |c, a| {
        let (spec, cb) = (a.str(0)?, a.callback(1)?);
        let h = hotkey(&spec, cb)?;
        Ok::<_, ApiError>(runtime::hotkey(c, h) as i64)
    }),
    def!("audio.play", "audio", [p("file", "string"), o("opts", "table")], "integer id or nil, reason", "Plays a WAV file of the plugin's folder. `opts`: `volume` (1), `pitch` (1), `loop`, `range` (metres heard at full volume, 5), and either `x, y, z` (a sound at a map point) or `on_bus = true` (it moves with the player's bus); none: heard alike everywhere. The game's volume setting applies. A plugin plays 32 at most.", NEW, Audio, true, |c, a| {
        let rel = a.str(0)?;
        let opts = sound_opts(c, a.opt_table(1)?)?;
        let s = c.state();
        let r = (|| {
            let folder = s.folder.clone().ok_or_else(|| "the plugin has no folder of files".to_string())?;
            let path = paths::inside(&folder, &rel)?;
            if !path.is_file() {
                return Err(format!("no file \"{rel}\""));
            }
            Ok(path)
        })();
        let path = match r {
            Ok(p) => p,
            Err(e) => return Ok(Value::List(vec![Value::Nil, Value::Str(e)])),
        };
        // (the oldest of its sounds that ended make room; a plugin holding 32 that play is refused)
        let voices = std::mem::take(&mut c.state().voices);
        let alive: Vec<u64> = voices.into_iter().filter(|v| c.io().sound_playing(*v)).collect();
        c.state().voices = alive;
        if c.state().voices.len() >= MAX_VOICES {
            return Ok(Value::List(vec![Value::Nil, Value::Str(format!("a plugin plays at most {MAX_VOICES} sounds"))]));
        }
        Ok::<_, ApiError>(match c.io().sound_play(&path, &opts) {
            Some(id) => {
                c.state().voices.push(id);
                Value::List(vec![Value::Int(id as i64)])
            }
            None => Value::List(vec![Value::Nil, Value::Str("the game cannot play it (no sound, or not a WAV file)".into())]),
        })
    }),
    def!("audio.set", "audio", [p("id", "integer"), p("opts", "table")], "boolean", "Changes a sound of the plugin's: `volume`, `pitch`, `range`, `x, y, z` / `on_bus`.", NEW, Audio, false, |c, a| {
        let id = a.int(0)? as u64;
        let opts = sound_opts(c, Some(a.table(1)?))?;
        if !c.state().voices.contains(&id) {
            return Ok(false.into());
        }
        Ok::<_, ApiError>(c.io().sound_set(id, &opts))
    }),
    def!("audio.stop", "audio", [p("id", "integer")], "boolean", "Stops a sound of the plugin's.", NEW, Audio, false, |c, a| {
        let id = a.int(0)? as u64;
        let s = c.state();
        let had = s.voices.contains(&id);
        s.voices.retain(|v| *v != id);
        if had {
            c.io().sound_stop(id);
        }
        Ok::<_, ApiError>(had)
    }),
    def!("audio.stop_all", "audio", [], "nil", "Stops every sound of the plugin's.", NEW, Audio, false, |c, a| {
        let v = std::mem::take(&mut c.state().voices);
        for id in v {
            c.io().sound_stop(id);
        }
    }),
    def!("audio.playing", "audio", [p("id", "integer")], "boolean", "Whether a sound of the plugin's still plays.", NEW, None, false, |c, a| {
        let id = a.int(0)? as u64;
        Ok::<_, ApiError>(c.state().voices.contains(&id) && c.io().sound_playing(id))
    }),
    def!("audio.volume", "audio", [], "number or nil", "The game's volume setting, 0 to 1.", NEW, None, false, |c, a| c.io().volume().map(Value::from)),
];
