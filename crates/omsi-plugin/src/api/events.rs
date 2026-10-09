//! The events a plugin hears (`omsi.on(name, fn)`, Lua's `on_<name>`): their documentation,
//! and those the runtime works out itself by comparing what the game shows from one frame
//! to the next ([`probe`]). The others the game sends (`PluginIo::events`).
//!
//! A comparison runs only while the plugin listens to one of its events, so a plugin that
//! does not care costs nothing; the first frame it listens only learns what is there.

use super::{runtime, Ctx, Value};
use crate::api::CallError;

/// One event, for the documentation and the manifest.
pub struct EventDef {
    pub name: &'static str,
    pub args: &'static [&'static str],
    pub doc: &'static str,
    pub since: &'static str,
}

const fn ev(name: &'static str, args: &'static [&'static str], since: &'static str, doc: &'static str) -> EventDef {
    EventDef { name, args, doc, since }
}

const OLD: &str = "0.1.5";
const OLD10: &str = "0.1.10";
const LATER: &str = "0.2.21";
const NEW: &str = super::VERSION;

/// Every event, in the order of the documentation.
pub static EVENTS: &[EventDef] = &[
    ev("start", &[], OLD, "Right after the plugin was loaded (also after a reload)."),
    ev("vehicle", &["name"], OLD, "The player got into a vehicle, changed it, or left it (`nil`)."),
    ev("frame", &["dt"], OLD, "Every frame of the game, after the bus's own scripts; not while paused. `dt` is the frame's seconds of game time."),
    ev("stop", &[], OLD, "The game ends, or the plugin is about to be loaded again."),
    ev("key", &["key", "down"], OLD10, "A key went down (`true`) or came up: winit's name of it (`\"KeyH\"`, `\"F5\"`, `\"Numpad8\"`)."),
    ev("next_stop", &["new", "old"], OLD10, "The duty's next stop changed, also to one of the same name (`omsi.info().next_stop_number` tells them apart)."),
    ev("view", &["new", "old"], OLD10, "The view changed (`\"driver\"`, `\"pax\"`, `\"outside\"`, `\"free\"`, `\"foot\"`)."),
    ev("duty", &["line", "tour"], OLD10, "A line and tour were taken (or given up: `nil`)."),
    ev("crash", &["energy_kj", "speed_kmh"], LATER, "The player's bus crashed: every crash, also one the same as the last (the screen's \"Crash: 136 kJ\"); above 50 kJ it is a heavy one."),
    ev("pedestrian", &["count"], LATER, "The bus knocked people down."),
    ev("stops_skipped", &["count", "due_at", "now_at"], LATER, "The duty jumped ahead: the bus passed stops of its trip without stopping (or was moved) and is now at a later one; the stops are numbered in the trip from 1."),
    ev("service", &["kind", "by", "amount"], LATER, "The player's bus was serviced or moved. `kind`: `\"refuel\"` (amount: litres put in), `\"wash\"` (amount: the dirt left), `\"repair\"` (amount: the game minutes it took), `\"reset\"` or `\"teleport\"`; `by`: `\"player\"`, `\"plugin\"`, `\"host\"` or `\"game\"`."),
    ev("trip_done", &["trip", "how", "driving", "comfort", "tickets"], LATER, "A trip of the duty ended, once: its number in the duty, `\"arrived\"`, `\"skipped\"` or `\"given_up\"`, then its ratings in per cent (driving, comfort, ticket selling)."),
    ev("jolt", &["along", "across", "speed_kmh", "passengers"], LATER, "The bus braked, sped up or cornered hard enough to cost driving rating (m/s², signed); at most one a second."),
    ev("ticket_sold", &["name", "price"], LATER, "A ticket was sold at the cash desk: its name and price as the bus's ticket list has them."),
    ev("ui_click", &["panel", "element"], LATER, "A button (or another clickable part) of one of the plugin's panels was clicked; `element` is `nil` for the panel itself. Enter in a text field is a click on it."),
    ev("ui_focus", &["focused"], LATER, "The panels got the mouse or gave it back (also by Esc, or a menu of the game opening)."),
    ev("ui_change", &["panel", "element", "value"], NEW, "A checkbox (`true`/`false`), slider (its number), text field (its text, at every key) or tabs element (the tab's number from 1) of the plugin's panels was changed by the player."),
    ev("message", &["from", "topic", "data"], NEW, "Another plugin sent this one a message (`plugin.send`, `plugin.broadcast`): its name, the topic and the data (numbers, texts, tables)."),
    ev("setting", &["key", "value"], NEW, "The player changed one of the plugin's settings in its settings panel (`plugin.settings`)."),
    ev("door", &["door", "open"], NEW, "A door leaf of the player's bus opened (`true`) or closed: its number from 1, front to back."),
    ev("doors", &["open"], NEW, "The bus's doors opened (`true`: one or more open) or were all closed."),
    ev("engine_start", &[], NEW, "The player's bus's engine started running."),
    ev("engine_stop", &[], NEW, "Its engine stopped."),
    ev("gear", &["new", "old"], NEW, "The gear engaged changed (-1 reverse, 0 neutral), where the bus has a gearbox it shows."),
    ev("indicator", &["new", "old"], NEW, "The indicators changed: `\"off\"`, `\"left\"`, `\"right\"` or `\"hazard\"`."),
    ev("headlights", &["new", "old"], NEW, "The headlights changed: 0 off, 1 side lights, 2 dipped, 3 high beam."),
    ev("horn", &["down"], NEW, "The horn started (`true`) or stopped."),
    ev("handbrake", &["on"], NEW, "The parking brake was put on (`true`) or released."),
    ev("stop_request", &["on"], NEW, "A passenger asked to stop (`true`), or the request went out."),
    ev("passengers", &["count", "old"], NEW, "The number of passengers aboard the player's bus changed."),
    ev("passenger_board", &["count", "stop_id"], NEW, "People got into the player's bus: how many, and the stop they waited at (its map object id, `nil` when none)."),
    ev("passenger_alight", &["count"], NEW, "People got out of the player's bus."),
    ev("stop_arrive", &["name", "id", "number", "delay"], NEW, "The bus came to the duty's next stop (within 25 m of it): its name, map object id, number in the trip (from 1) and the delay in seconds (late positive)."),
    ev("stop_depart", &["name", "id", "number", "delay"], NEW, "The bus left the stop it stood at (more than 35 m from it, or the duty moved on): the same values, the delay as it left."),
    ev("trip_start", &["trip", "name", "terminus"], NEW, "The duty moved on to another trip: its number in the duty (from 1), the timetable's name of the trip and its terminus."),
    ev("duty_start", &["line", "tour"], NEW, "A duty was taken."),
    ev("duty_end", &["line", "tour"], NEW, "The duty was given up (or another taken: `duty_end` of the old one comes first)."),
    ev("destination", &["new", "old"], NEW, "The destination the bus shows changed."),
    ev("coupled", &["parts", "old"], NEW, "Something was coupled to or uncoupled from the bus: the parts behind it now."),
    ev("minute", &["hour", "minute"], NEW, "The game's clock reached a new minute (also when it was set)."),
    ev("hour", &["hour"], NEW, "The game's clock reached a new hour."),
    ev("day", &["year", "month", "day"], NEW, "The game's date changed."),
    ev("tile", &["x", "y", "old_x", "old_y"], NEW, "The player's bus drove onto another tile of the map (numbered as global.cfg's `[map]` list)."),
    ev("weather", &["name"], NEW, "The weather changed: another weather file, rain or snow beginning or ending, the snow cover, the clouds."),
    ev("light_ahead", &["aspect", "distance"], NEW, "The traffic light ahead of the bus (within 80 m) changed or a new one came: its aspect (`\"red\"`, `\"red_yellow\"`, `\"green\"`, `\"green_yellow\"`, `\"yellow\"`, `\"dark\"`, `nil` when none is ahead any more) and its distance in metres."),
    ev("red_light", &["speed_kmh"], NEW, "The bus drove past a traffic light showing red (or red and yellow) at more than 5 km/h."),
    ev("speed_limit", &["kmh", "old"], NEW, "The speed limit of the lane the bus drives on changed."),
    ev("ai_collision", &["id", "energy_kj"], NEW, "The player's bus hit an AI vehicle (its id, as `traffic.list` gives it) with this energy."),
    ev("pause", &[], NEW, "The game was paused (the plugins stand still until `resume`; nothing else comes in between)."),
    ev("resume", &[], NEW, "The game goes on after a pause."),
    ev("menu_open", &[], NEW, "The game menu opened."),
    ev("menu_close", &[], NEW, "The game menu closed."),
    ev("screenshot", &["file"], NEW, "A screenshot was taken: its file."),
    ev("controller_button", &["device", "button", "down"], NEW, "A button of a steering wheel, joystick or gamepad went down (`true`) or up: the device's name and the button's number."),
    ev("lan_join", &["id", "name"], NEW, "A player joined the LAN session (or came back)."),
    ev("lan_leave", &["id", "name"], NEW, "A player left the LAN session."),
    ev("lan_message", &["from", "data"], NEW, "The same plugin on another player's game sent this one a message (`lan.send`): the player's id and the text."),
    ev("lan_chat", &["name", "text"], NEW, "A line was said in the LAN session's chat (by another player or this one)."),
];

/// The event of this name.
pub fn find(name: &str) -> Option<&'static EventDef> {
    EVENTS.iter().find(|e| e.name == name)
}

/// What the comparisons saw at the last frame they ran.
#[derive(Default)]
pub struct ProbeState {
    doors: Option<(Vec<bool>, bool)>,
    engine: Option<bool>,
    gear: Option<Option<i64>>,
    indicator: Option<u8>,
    headlights: Option<u8>,
    horn: Option<bool>,
    handbrake: Option<bool>,
    request: Option<bool>,
    passengers: Option<usize>,
    stop: Option<StopSeen>,
    trip: Option<Option<(String, String, usize)>>,
    duty: Option<Option<(String, String)>>,
    destination: Option<Value>,
    coupled: Option<usize>,
    minute: Option<(i64, i64, (i32, u32, u32))>,
    tile: Option<(Value, Value)>,
    weather: Option<Option<(String, i32, bool, String)>>,
    light: Option<Option<(&'static str, f64)>>,
    limit: Option<Option<i64>>,
}

/// The stop the bus stood at (`stop_arrive` / `stop_depart`).
#[derive(Clone, PartialEq)]
struct StopSeen {
    at: Option<(String, i64, usize)>,
}

/// Fire `event` when `now` differs from what was seen; the first look only learns.
macro_rules! changed {
    ($ctx:ident, $field:ident, [$($ev:literal),+], $now:expr, |$old:ident, $new:ident| $fire:block) => {{
        if $(runtime::hears($ctx, $ev))||+ {
            let $new = $now;
            let prev = $ctx.state().probes.$field.replace($new.clone());
            if let Some($old) = prev {
                if $old != $new $fire
            }
        } else {
            $ctx.state().probes.$field = None;
        }
    }};
}

fn indicator_name(i: u8) -> &'static str {
    match i {
        1 => "left",
        2 => "right",
        3 => "hazard",
        _ => "off",
    }
}

/// The first of these bus variables that the bus has.
fn var_of(ctx: &mut Ctx<'_>, names: &[&str]) -> Option<f32> {
    let io = ctx.io();
    if !io.has_vehicle() {
        return None;
    }
    names.iter().find_map(|n| io.var(n))
}

fn info_of(ctx: &mut Ctx<'_>, key: &str) -> Value {
    ctx.io().info_value(key).map_or(Value::Nil, Value::from)
}

/// The comparisons of this frame.
pub fn probe(ctx: &mut Ctx<'_>) -> Result<(), CallError> {
    bus_probes(ctx)?;
    duty_probes(ctx)?;
    world_probes(ctx)
}

fn bus_probes(ctx: &mut Ctx<'_>) -> Result<(), CallError> {
    changed!(ctx, doors, ["door", "doors"], {
        let io = ctx.io();
        (io.bus_doors().iter().map(|d| *d > 0.05).collect::<Vec<bool>>(), io.bus_doors_open().unwrap_or(false))
    }, |old, new| {
        for (i, (a, b)) in old.0.iter().zip(&new.0).enumerate() {
            if a != b {
                runtime::dispatch(ctx, "door", vec![Value::Int(i as i64 + 1), Value::Bool(*b)])?;
            }
        }
        if old.1 != new.1 {
            runtime::dispatch(ctx, "doors", vec![Value::Bool(new.1)])?;
        }
    });
    changed!(ctx, engine, ["engine_start", "engine_stop"], ctx.io().bus_engine().is_some_and(|e| e.0), |_old, new| {
        runtime::dispatch(ctx, if new { "engine_start" } else { "engine_stop" }, Vec::new())?;
    });
    changed!(ctx, gear, ["gear"], ctx.io().bus_gear().map(|g| g.round() as i64), |old, new| {
        runtime::dispatch(ctx, "gear", vec![Value::opt(new), Value::opt(old)])?;
    });
    changed!(ctx, indicator, ["indicator"], ctx.io().bus_indicator().unwrap_or(0), |old, new| {
        runtime::dispatch(ctx, "indicator", vec![indicator_name(new).into(), indicator_name(old).into()])?;
    });
    changed!(ctx, headlights, ["headlights"], ctx.io().bus_headlights().unwrap_or(0), |old, new| {
        runtime::dispatch(ctx, "headlights", vec![Value::Int(new as i64), Value::Int(old as i64)])?;
    });
    changed!(ctx, horn, ["horn"], var_of(ctx, &["cockpit_hupe", "cockpit_hupe_swheel", "horn"]).is_some_and(|v| v > 0.5), |_old, new| {
        runtime::dispatch(ctx, "horn", vec![Value::Bool(new)])?;
    });
    changed!(ctx, handbrake, ["handbrake"], var_of(ctx, &["bremse_feststell", "parking_brake"]).is_some_and(|v| v > 0.5), |_old, new| {
        runtime::dispatch(ctx, "handbrake", vec![Value::Bool(new)])?;
    });
    changed!(ctx, request, ["stop_request"], var_of(ctx, &["haltewunsch", "stop_request"]).is_some_and(|v| v > 0.5), |_old, new| {
        runtime::dispatch(ctx, "stop_request", vec![Value::Bool(new)])?;
    });
    changed!(ctx, passengers, ["passengers"], ctx.io().bus_passengers().map_or(0, |p| p.0), |old, new| {
        runtime::dispatch(ctx, "passengers", vec![Value::Int(new as i64), Value::Int(old as i64)])?;
    });
    changed!(ctx, destination, ["destination"], info_of(ctx, "destination"), |old, new| {
        runtime::dispatch(ctx, "destination", vec![new, old])?;
    });
    changed!(ctx, coupled, ["coupled"], ctx.io().bus_trailers().unwrap_or(0), |old, new| {
        runtime::dispatch(ctx, "coupled", vec![Value::Int(new as i64), Value::Int(old as i64)])?;
    });
    Ok(())
}

fn duty_probes(ctx: &mut Ctx<'_>) -> Result<(), CallError> {
    let wants = ["stop_arrive", "stop_depart", "trip_start", "duty_start", "duty_end"].iter().any(|e| runtime::hears(ctx, e));
    let duty = if wants { ctx.io().duty() } else { None };
    // the stop the bus stands at
    changed!(ctx, stop, ["stop_arrive", "stop_depart"], StopSeen {
        at: duty.as_ref().filter(|d| d.at_stop).and_then(|d| d.current.stops.get(d.next).map(|s| (s.name.trim().to_string(), s.id, d.next + 1)))
    }, |old, new| {
        let delay = duty.as_ref().map_or(0.0, |d| d.delay);
        if let Some((name, id, n)) = old.at.clone() {
            runtime::dispatch(ctx, "stop_depart", vec![name.into(), Value::Int(id), Value::Int(n as i64), Value::Num(delay)])?;
        }
        if let Some((name, id, n)) = new.at.clone() {
            runtime::dispatch(ctx, "stop_arrive", vec![name.into(), Value::Int(id), Value::Int(n as i64), Value::Num(delay)])?;
        }
    });
    changed!(ctx, trip, ["trip_start"], duty.as_ref().map(|d| (d.current.name.trim().to_string(), d.current.terminus.trim().to_string(), d.trip)), |_old, new| {
        if let Some((name, terminus, n)) = new {
            runtime::dispatch(ctx, "trip_start", vec![Value::Int(n as i64 + 1), name.into(), terminus.into()])?;
        }
    });
    changed!(ctx, duty, ["duty_start", "duty_end"], duty.as_ref().map(|d| (d.line.trim().to_string(), d.tour.trim().to_string())), |old, new| {
        if let Some((l, t)) = old {
            runtime::dispatch(ctx, "duty_end", vec![l.into(), t.into()])?;
        }
        if let Some((l, t)) = new {
            runtime::dispatch(ctx, "duty_start", vec![l.into(), t.into()])?;
        }
    });
    Ok(())
}

fn world_probes(ctx: &mut Ctx<'_>) -> Result<(), CallError> {
    let clock = if ["minute", "hour", "day"].iter().any(|e| runtime::hears(ctx, e)) { ctx.io().clock() } else { None };
    changed!(ctx, minute, ["minute", "hour", "day"], clock.as_ref().map_or((0, 0, (0, 0, 0)), |c| {
        let t = c.time.rem_euclid(86400.0) as i64;
        (t / 3600, t / 60 % 60, (c.year, c.month, c.day))
    }), |old, new| {
        if clock.is_some() {
            if new.2 != old.2 {
                runtime::dispatch(ctx, "day", vec![Value::Int(new.2 .0 as i64), Value::Int(new.2 .1 as i64), Value::Int(new.2 .2 as i64)])?;
            }
            if new.0 != old.0 {
                runtime::dispatch(ctx, "hour", vec![Value::Int(new.0)])?;
            }
            if (new.0, new.1) != (old.0, old.1) {
                runtime::dispatch(ctx, "minute", vec![Value::Int(new.0), Value::Int(new.1)])?;
            }
        }
    });
    changed!(ctx, tile, ["tile"], (info_of(ctx, "tile_x"), info_of(ctx, "tile_y")), |old, new| {
        if !new.0.is_nil() {
            runtime::dispatch(ctx, "tile", vec![new.0, new.1, old.0, old.1])?;
        }
    });
    changed!(ctx, weather, ["weather"], ctx.io().weather().map(|w| (w.name, w.precip_kind, w.snow_cover, w.clouds)), |_old, new| {
        if let Some((name, ..)) = new {
            runtime::dispatch(ctx, "weather", vec![name.into()])?;
        }
    });
    let light = if runtime::hears(ctx, "light_ahead") || runtime::hears(ctx, "red_light") { ctx.io().traffic_light_ahead(80.0) } else { None };
    changed!(ctx, light, ["light_ahead", "red_light"], light.as_ref().map(|l| (l.aspect, l.distance)), |old, new| {
        // (the distance moves every frame: only a new aspect, or a light that came or went, is news)
        let aspect = |x: &Option<(&'static str, f64)>| x.map(|l| l.0);
        let speed = { let io = ctx.io(); io.var("Velocity").map(|v| v.abs() as f64).unwrap_or(0.0) };
        if let (Some((a, d)), None) = (old, new) {
            if matches!(a, "red" | "red_yellow") && d < 15.0 && speed > 5.0 {
                runtime::dispatch(ctx, "red_light", vec![Value::Num(speed)])?;
            }
        }
        if aspect(&old) != aspect(&new) || (new.is_some() && old.is_some_and(|o| o.1 + 5.0 < new.unwrap().1)) {
            runtime::dispatch(ctx, "light_ahead", vec![Value::opt(new.map(|l| l.0)), Value::opt(new.map(|l| (l.1 * 10.0).round() / 10.0))])?;
        }
    });
    // (the frame's own distance kept, without firing: see above)
    let limit = if runtime::hears(ctx, "speed_limit") {
        let io = ctx.io();
        io.position().and_then(|p| io.map_lane(p[0], p[1])).filter(|l| l.distance < 6.0).map(|l| l.speed_limit_kmh.round() as i64)
    } else {
        None
    };
    changed!(ctx, limit, ["speed_limit"], limit, |old, new| {
        if new.is_some() {
            runtime::dispatch(ctx, "speed_limit", vec![Value::opt(new), Value::opt(old)])?;
        }
    });
    Ok(())
}
