//! The functions every plugin had from the start: the player's bus by its script names,
//! `info`, events, timers and watches, messages on the screen and in the log, `send`, and
//! the panels (`ui.*`).

use super::{def, multi, o, p, rec};
use crate::api::runtime::{self, WatchKind};
use crate::api::{ApiError, ApiFn, Args, Ctx, Value};
use crate::ui;
use std::net::{Ipv4Addr, UdpSocket};
use std::time::{Duration, Instant};

const V5: &str = "0.1.5";
const V10: &str = "0.1.10";
const V21: &str = "0.2.21";
const NEW: &str = crate::api::VERSION;

/// Most `send` messages of a plugin in one second, so a plugin cannot flood a program on
/// this computer.
const SEND_PER_SECOND: u32 = 100;
/// Longest `send` message: the same on every system (macOS takes UDP datagrams of at most
/// 9 KB by default, Windows and Linux about 64 KB).
const SEND_MAX: usize = 8 * 1024;
/// The game's multiplayer ports (`omsi_net::DEFAULT_PORT` and the `PORT_RANGE` after it): a
/// plugin's messages must not reach a session hosted on this computer.
const MULTIPLAYER_PORTS: std::ops::Range<u16> = 27015..27025;

/// A plugin's `send`: its socket, opened on the first message, and the messages of the
/// current second.
#[derive(Default)]
pub struct Sender {
    socket: Option<UdpSocket>,
    second: Option<Instant>,
    sent: u32,
}

impl Sender {
    /// Send `data` to `127.0.0.1:port`, never waiting: with nobody listening it is lost, as
    /// UDP is. Err says why it was not sent.
    pub fn send(&mut self, port: i64, data: &[u8]) -> Result<(), String> {
        let port = u16::try_from(port).ok().filter(|p| *p >= 1024).ok_or("the port must be 1024-65535")?;
        if MULTIPLAYER_PORTS.contains(&port) {
            return Err(format!("ports {}-{} are the game's multiplayer", MULTIPLAYER_PORTS.start, MULTIPLAYER_PORTS.end - 1));
        }
        if data.len() > SEND_MAX {
            return Err(format!("a message is at most 8 KB ({} bytes given)", data.len()));
        }
        if self.second.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
            self.second = Some(Instant::now());
            self.sent = 0;
        }
        if self.sent >= SEND_PER_SECOND {
            return Err(format!("more than {SEND_PER_SECOND} messages in a second"));
        }
        if self.socket.is_none() {
            let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
            socket.set_nonblocking(true).map_err(|e| e.to_string())?;
            self.socket = Some(socket);
        }
        let socket = self.socket.as_ref().expect("opened above");
        socket.send_to(data, (Ipv4Addr::LOCALHOST, port)).map_err(|e| e.to_string())?;
        // (only what went out counts)
        self.sent += 1;
        Ok(())
    }
}

/// `true`, or `false` and the reason: what `send` and `ui.set` answer.
pub(crate) fn ok_or_reason(r: Result<(), String>) -> Value {
    match r {
        Ok(()) => Value::List(vec![Value::Bool(true), Value::Nil]),
        Err(why) => Value::List(vec![Value::Bool(false), Value::Str(why)]),
    }
}

/// The arguments written as `print` writes them, separated by tabs.
fn join(a: &Args) -> String {
    a.v.iter()
        .map(|v| match v {
            Value::Nil => "nil".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::List(_) | Value::Map(_) => "table".to_string(),
            Value::Callback(_) => "function".to_string(),
            Value::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
            v => v.to_text().unwrap_or_default(),
        })
        .collect::<Vec<_>>()
        .join("\t")
}

fn veh(ctx: &mut Ctx<'_>) -> bool {
    ctx.io().has_vehicle()
}

/// A panel id: a text, or a number written as one.
fn ui_id(v: &Value) -> Result<String, String> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        Value::Int(i) => Ok(i.to_string()),
        _ => Err("the panel id is a text".to_string()),
    }
}

fn info_value(ctx: &mut Ctx<'_>, key: &str) -> Value {
    ctx.io().info_value(key).map_or(Value::Nil, Value::from)
}

pub static FNS: &[ApiFn] = &[
    // --- the player's bus by its script names ---
    def!("has_vehicle", "vehicle", [], "boolean", "`true` while the player drives a vehicle.", V5, None, false, |c, a| c.io().has_vehicle()),
    def!("vehicle", "vehicle", [], "string or nil", "The vehicle's name (manufacturer and type), or `nil` on foot.", V5, None, false, |c, a| {
        let io = c.io();
        io.vehicle_name().filter(|_| io.has_vehicle())
    }),
    def!("vehicle_manufacturer", "vehicle", [], "string or nil", "The manufacturer part of the vehicle's name, as its `[friendlyname]` has it (`\"Solaris III Gen\"`).", V21, None, false, |c, a| {
        let io = c.io();
        io.vehicle_manufacturer_model().filter(|_| io.has_vehicle()).map(|(m, _)| m)
    }),
    def!("vehicle_model", "vehicle", [], "string or nil", "The model part of the vehicle's name (`\"Urbino 10 / 2D\"`).", V21, None, false, |c, a| {
        let io = c.io();
        io.vehicle_manufacturer_model().filter(|_| io.has_vehicle()).map(|(_, m)| m)
    }),
    def!("var", "vehicle", [p("name", "string")], "number or nil", "A script variable of the bus (`Velocity`, `elec_busbar_main`, the names the `.osc` files and `.opl` lists use); `nil` without a bus or for a name it does not have.", V5, None, false, |c, a| {
        let n = a.str(0)?;
        Ok::<_, ApiError>(if veh(c) { c.io().var(&n).map(|v| v as f64) } else { None })
    }),
    def!("set_var", "vehicle", [p("name", "string"), p("value", "number")], "boolean", "Sets a script variable; `true` when the bus has that variable.", V5, VehicleWrite, false, |c, a| {
        let (n, v) = (a.str(0)?, a.num(1)? as f32);
        let io = c.io();
        Ok::<_, ApiError>(io.has_vehicle() && io.var(&n).is_some() && {
            io.set_var(&n, v);
            true
        })
    }),
    def!("str", "vehicle", [p("name", "string")], "string or nil", "A string variable of the bus's scripts (`IBIS_terminus_name`).", V5, None, false, |c, a| {
        let n = a.str(0)?;
        Ok::<_, ApiError>(if veh(c) { c.io().string(&n) } else { None })
    }),
    def!("set_str", "vehicle", [p("name", "string"), p("text", "string")], "boolean", "Sets a string variable; `true` when the bus has it.", V5, VehicleWrite, false, |c, a| {
        let (n, s) = (a.str(0)?, a.str(1)?);
        let io = c.io();
        Ok::<_, ApiError>(io.has_vehicle() && io.string(&n).is_some() && {
            io.set_string(&n, &s);
            true
        })
    }),
    def!("sys", "vehicle", [p("name", "string")], "number or nil", "A system variable of the scripts: `Time`, `Day`, `Weather_Temperature`, `SunAlt`, ... (read only).", V5, None, false, |c, a| {
        let n = a.str(0)?;
        Ok::<_, ApiError>(c.io().system(&n).map(|v| v as f64))
    }),
    def!("press", "vehicle", [p("name", "string")], "nil", "Holds a key of the bus down: fires the trigger `name` (let it go with `release`).", V5, VehicleWrite, false, |c, a| {
        let n = a.str(0)?;
        c.io().fire(&n, true);
        Ok::<_, ApiError>(())
    }),
    def!("release", "vehicle", [p("name", "string")], "nil", "Lets a key go: fires `<name>_off`.", V5, VehicleWrite, false, |c, a| {
        let n = a.str(0)?;
        c.io().fire(&n, false);
        Ok::<_, ApiError>(())
    }),
    def!("trigger", "vehicle", [p("name", "string")], "nil", "A key press of the bus: fires the trigger, then `<name>_off`.", V5, VehicleWrite, false, |c, a| {
        let n = a.str(0)?;
        c.io().fire(&n, true);
        c.io().fire(&n, false);
        Ok::<_, ApiError>(())
    }),
    def!("position", "vehicle", [], "x, y, z, heading", "Where the bus is: map metres (x east, y north, z up) and its heading in degrees clockwise from north; nothing on foot.", V10, None, true, |c, a| multi(c.io().position().map(|p| p.map(Value::Num)))),
    def!("vars", "vehicle", [o("kind", "string")], "list of strings", "The names of every variable of the bus's scripts, or of every string variable with `\"str\"`.", V10, None, false, |c, a| {
        let kind = a.opt_str(0)?;
        let (vars, strs) = c.io().var_names();
        Ok::<_, ApiError>(if kind.as_deref() == Some("str") { strs } else { vars })
    }),
    def!("speed", "vehicle", [], "number", "The bus's speed in km/h, forwards or backwards (0 on foot).", V10, None, false, |c, a| {
        let v = if veh(c) { c.io().var("Velocity") } else { None };
        (v.unwrap_or(0.0).abs()) as f64
    }),
    def!("distance", "vehicle", [p("x", "number"), p("y", "number")], "number or nil", "Metres from the bus to a map point, or `nil` on foot.", V10, None, false, |c, a| {
        let (x, y) = (a.num(0)?, a.num(1)?);
        Ok::<_, ApiError>(c.io().position().map(|p| ((p[0] - x).powi(2) + (p[1] - y).powi(2)).sqrt()))
    }),
    def!("others", "traffic", [o("radius", "number")], "list of tables", "The other vehicles within `radius` m of the bus (default 300): each `{id, kind, name, x, y, z, heading}`, `kind` being `\"ai\"` (the traffic) or `\"player\"` (another player's bus in a LAN game); empty on foot.", V21, None, false, |c, a| {
        let r = a.opt_num(0)?.unwrap_or(300.0);
        let list = c.io().others(r);
        Ok::<_, ApiError>(Value::List(
            list.into_iter()
                .map(|o| rec(vec![("id", Value::Int(o.id as i64)), ("kind", o.kind.into()), ("name", o.name.into()), ("x", o.pos[0].into()), ("y", o.pos[1].into()), ("z", o.pos[2].into()), ("heading", o.pos[3].into())]))
                .collect(),
        ))
    }),
    def!("other_var", "traffic", [p("id", "integer"), p("name", "string")], "number or nil", "A script variable of one of `others`, or `nil`.", V21, None, false, |c, a| {
        let (id, n) = (a.int(0)? as u64, a.str(1)?);
        Ok::<_, ApiError>(c.io().other_var(id, &n).map(|v| v as f64))
    }),
    def!("set_other_var", "traffic", [p("id", "integer"), p("name", "string"), p("value", "number")], "boolean", "Sets a script variable of one of `others`; `true` when that vehicle has it. An AI vehicle keeps it until its scripts write it again; another player's bus takes its values from the network again.", V21, TrafficWrite, false, |c, a| {
        let (id, n, v) = (a.int(0)? as u64, a.str(1)?, a.num(2)? as f32);
        Ok::<_, ApiError>(c.io().set_other_var(id, &n, v))
    }),
    // --- the game ---
    def!("info", "game", [], "table", "What the game is doing: `map`, `clock` (seconds since midnight), `day`, `year`, `view`, `paused`, `on_foot`, `multiplayer`, `traffic`, `speed`, `delay`, `map_path`, `version`; with a bus also `tile_x`, `tile_y`, `tile_pos_x`, `tile_pos_y`, `heading`, `vehicle_manufacturer`, `vehicle_model`, `destination`, `passengers`; `crashes`, `heavy_crashes`, `pedestrians_hit`; `situation`; on a duty also `line`, `tour`, `trip`, `trips`, `trip_name`, `terminus`, `stops`, `trip_done`, `next_stop`, `next_stop_number`, `next_stop_arrival`, `next_stop_departure`, `next_stop_id`, `at_stop`, `previous_stop`, `previous_stop_id`, `next_stop_distance`, `previous_stop_distance` (see the plugin docs for each).", V10, None, false, |c, a| {
        Value::Map(c.io().info().into_iter().map(|(k, v)| (k.to_string(), v.into())).collect())
    }),
    def!("info_value", "game", [p("key", "string")], "any", "One value of `info()` without building the whole table: cheaper for a plugin that reads one or two every frame.", NEW, None, false, |c, a| {
        let k = a.str(0)?;
        Ok::<_, ApiError>(info_value(c, &k))
    }),
    def!("clock", "time", [], "string", "The game's time of day as `\"HH:MM:SS\"`.", V10, None, false, |c, a| {
        let t = info_value(c, "clock").as_f64().unwrap_or(0.0).floor() as i64;
        format!("{:02}:{:02}:{:02}", t / 3600 % 24, t / 60 % 60, t % 60)
    }),
    def!("command", "game", [p("name", "string")], "boolean", "Does what a line of the game menu does: `refuel`, `wash`, `repair`, `shot`, `save`, `load`, `weather`, `later`, `earlier`, `info`, `timetable`, `reset`, `couple`, `uncouple`; `true` when the game knows it (it runs after the frame).", V10, WorldWrite, false, |c, a| {
        let n = a.str(0)?;
        Ok::<_, ApiError>(c.io().command(&n))
    }),
    def!("message", "ui", [p("text", "string"), o("seconds", "number")], "nil", "A line of text on the screen (5 seconds when not given).", V5, Ui, false, |c, a| {
        let (t, s) = (a.str(0)?, a.opt_num(1)?.unwrap_or(5.0));
        c.io().message(&t, s as f32);
        Ok::<_, ApiError>(())
    }),
    def!("log", "game", [o("...", "any")], "nil", "A line in `game.log`, tagged `[lua <name>]` (`print` does the same).", V5, None, false, |c, a| {
        log::info!("{} {}", c.state().tag, join(&a));
    }),
    def!("warn", "game", [o("...", "any")], "nil", "The same as a warning.", V5, None, false, |c, a| {
        log::warn!("{} {}", c.state().tag, join(&a));
    }),
    def!("error", "game", [o("...", "any")], "nil", "The same as an error line (the plugin goes on).", NEW, None, false, |c, a| {
        log::error!("{} {}", c.state().tag, join(&a));
    }),
    def!("debug", "game", [o("...", "any")], "nil", "A line of the log's debug level (shown with `RUST_LOG=debug`).", NEW, None, false, |c, a| {
        log::debug!("{} {}", c.state().tag, join(&a));
    }),
    def!("send", "network", [p("port", "integer"), p("data", "string")], "true, or false and the reason", "Sends `data` as one UDP datagram to `127.0.0.1:port`: to another program on this computer, never over the network. Not sent when the port is below 1024 or one of the game's multiplayer ports (27015-27024), the message is longer than 8 KB, or the plugin sent 100 in the last second.", V21, NetworkLocal, true, |c, a| {
        let port = a.int(0)?;
        let data = match a.get(1) {
            Value::Bytes(b) => b.clone(),
            v => v.to_text().ok_or_else(|| ApiError("omsi.send: argument 2 (data) must be a string".into()))?.into_bytes(),
        };
        Ok::<_, ApiError>(ok_or_reason(c.state().sender.send(port, &data)))
    }),
    // --- events, timers, watches ---
    def!("on", "events", [p("event", "string"), p("fn", "function")], "the function", "Adds a handler of an event (see the events); several may hear one event, in the order they were added.", V5, None, false, |c, a| {
        let (e, cb) = (a.str(0)?, a.callback(1)?);
        runtime::on(c, &e, cb);
        Ok::<_, ApiError>(Value::Callback(cb))
    }),
    def!("off", "events", [p("event", "string"), p("fn", "function")], "nil", "Removes a handler added with `on`.", V5, None, false, |c, a| {
        let e = a.str(0)?;
        if let Value::Callback(cb) = a.get(1) {
            runtime::off(c, &e, *cb);
        }
        Ok::<_, ApiError>(())
    }),
    def!("emit", "events", [p("event", "string"), o("...", "any")], "nil", "Sends an event to this plugin's own handlers at once (handy between the modules of a bigger plugin); an error of a handler is this call's.", V5, None, false, |c, a| {
        let e = a.str(0)?;
        let rest = a.v.split_off(1.min(a.v.len()));
        runtime::dispatch(c, &e, rest).map_err(|e| ApiError(e.msg))
    }),
    def!("after", "events", [p("seconds", "number"), p("fn", "function")], "integer id", "Runs `fn` once, `seconds` of game time later.", V5, None, false, |c, a| {
        let (s, cb) = (a.num(0)?, a.callback(1)?);
        Ok::<_, ApiError>(runtime::after(c, s, false, cb) as i64)
    }),
    def!("every", "events", [p("seconds", "number"), p("fn", "function")], "integer id", "Runs `fn` every `seconds` of game time.", V5, None, false, |c, a| {
        let (s, cb) = (a.num(0)?, a.callback(1)?);
        if s <= 0.0 {
            return Err(ApiError("omsi.every: the interval must be above 0".into()));
        }
        Ok(runtime::after(c, s, true, cb) as i64)
    }),
    def!("watch", "events", [p("kind", "string"), p("name", "string"), o("fn", "function")], "integer id", "Runs `fn(new, old)` whenever a value changes: `watch(name, fn)` a variable of the bus, `watch(kind, name, fn)` of kind `\"var\"`, `\"str\"`, `\"sys\"` or `\"info\"` (a key of `info()`).", V5, None, false, |c, a| {
        let (kind, name, cb) = match a.get(2) {
            Value::Nil => ("var".to_string(), a.str(0)?, a.callback(1)?),
            _ => (a.str(0)?, a.str(1)?, a.callback(2)?),
        };
        let kind = WatchKind::parse(&kind).ok_or_else(|| ApiError("omsi.watch: kind is var, str, sys or info".into()))?;
        Ok::<_, ApiError>(runtime::watch(c, kind, name, cb) as i64)
    }),
    def!("cancel", "events", [p("id", "integer")], "boolean", "Stops a timer, a watch or a hotkey; `true` when there was one.", V5, None, false, |c, a| {
        let id = a.int(0)?;
        Ok::<_, ApiError>(runtime::cancel(c, id as u64))
    }),
    def!("time", "events", [], "number", "Seconds of game time since the plugin started (stands still while paused).", V5, None, false, |c, a| c.state().clock),
    // --- the panels ---
    def!("ui.set", "ui", [p("id", "string"), p("panel", "table")], "true, or false and the reason", "Creates the panel `id` or replaces it; a table that is not right gives `false` and where (`\"children[2].size: a number is expected\"`). The same table again changes nothing.", V21, Ui, true, |c, a| {
        let r = (|| {
            let id = ui_id(a.get(0))?;
            let t = a.get(1);
            if !t.is_table() {
                return Err("the panel is a table".to_string());
            }
            let s = c.state();
            let panel = ui::parse_panel(t, s.folder.as_deref())?;
            let owner = s.owner;
            s.ui().set(owner, &id, panel)
        })();
        ok_or_reason(r)
    }),
    def!("ui.remove", "ui", [p("id", "string")], "boolean", "Removes a panel; `true` when there was one.", V21, Ui, false, |c, a| {
        let s = c.state();
        let owner = s.owner;
        ui_id(a.get(0)).is_ok_and(|id| s.ui().remove(owner, &id))
    }),
    def!("ui.clear", "ui", [], "nil", "Removes every panel of the plugin.", V21, Ui, false, |c, a| {
        let s = c.state();
        let owner = s.owner;
        s.ui().clear(owner);
    }),
    def!("ui.toast", "ui", [p("text", "string"), o("opts", "table")], "true, or false and the reason", "A notification card at the top right, newest at the top; it goes after `opts.seconds` (1 to 60, default 5). `opts`: `title`, `icon`, `color`. A plugin shows 8 at most: a ninth makes its oldest go.", V21, Ui, true, |c, a| {
        let opts = a.get(1).is_table().then(|| a.get(1));
        let r = ui::parse_toast(a.get(0), opts).map(|spec| {
            let s = c.state();
            let owner = s.owner;
            s.ui().toast(owner, spec);
        });
        ok_or_reason(r)
    }),
    def!("ui.focus", "ui", [p("on", "bool")], "boolean", "`true`: the panels get the mouse (the cursor shows, a click goes to the panel under it and none to the bus); `false`, Esc or a menu of the game gives it back. Returns the new state.", V21, Ui, false, |c, a| {
        let on = match a.get(0) {
            Value::Bool(b) => *b,
            v => !v.is_nil(),
        };
        let s = c.state();
        let owner = s.owner;
        s.ui().set_focus(owner, on)
    }),
    def!("ui.focused", "ui", [], "boolean", "Whether the panels have the mouse.", V21, None, false, |c, a| c.state().ui().focused()),
    def!("ui.screen", "ui", [], "width, height, scale", "The screen in the panels' pixels, and how many of the screen's own pixels one of them is.", V21, None, true, |c, a| {
        let [w, h, s] = c.state().ui().screen();
        Value::List(vec![Value::Num(w as f64), Value::Num(h as f64), Value::Num(s as f64)])
    }),
];
