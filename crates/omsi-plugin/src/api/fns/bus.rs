//! `bus.*`: the player's bus in the game's own terms - its motion, doors, lights, engine,
//! gearbox, the destination, the passengers and tickets - besides its script variables
//! (`var`). Without a bus every reading is `nil` (or empty) and every change `false`.
//!
//! Where the game has no state of its own the readings come from the script variables the
//! OMSI buses use (the same names the game's own HTML displays read: `door_<n>`,
//! `cockpit_hupe`, `bremse_feststell`, ...): a bus without them gives `nil`.

use super::{def, multi, nums, o, p, rec};
use crate::api::{ApiError, ApiFn, Ctx, Value};

const NEW: &str = crate::api::VERSION;

fn has(c: &mut Ctx<'_>) -> bool {
    c.io().has_vehicle()
}

/// The first of these script variables the bus has.
fn first_var(c: &mut Ctx<'_>, names: &[&str]) -> Option<f32> {
    let io = c.io();
    if !io.has_vehicle() {
        return None;
    }
    names.iter().find_map(|n| io.var(n))
}

fn flag(c: &mut Ctx<'_>, names: &[&str]) -> Value {
    first_var(c, names).map(|v| v > 0.5).into()
}

fn indicator(i: u8) -> &'static str {
    match i {
        1 => "left",
        2 => "right",
        3 => "hazard",
        _ => "off",
    }
}

const HORN: &[&str] = &["cockpit_hupe", "cockpit_hupe_swheel", "horn"];
const HANDBRAKE: &[&str] = &["bremse_feststell", "parking_brake"];
const STOP_BRAKE: &[&str] = &["bremse_halte", "bremse_halte_sw", "stop_brake"];
const KNEELING: &[&str] = &["bremse_kneeling", "vdv_kneel", "ecas_kneel", "kneeling"];
const REQUEST: &[&str] = &["haltewunsch", "stop_request"];
const RETARDER: &[&str] = &["retarder", "retarder_level"];
const FUEL: &[&str] = &["engine_tank_content", "fuel"];
const ELECTRICS: &[&str] = &["elec_busbar_main", "elec_busbar_main_sw"];

/// The whole state at once, for a dashboard.
fn state(c: &mut Ctx<'_>) -> Value {
    if !has(c) {
        return Value::Nil;
    }
    let io = c.io();
    let speed = io.var("Velocity").unwrap_or(0.0) as f64;
    let engine = io.bus_engine();
    let mut r = vec![
        ("speed", Value::Num(speed)),
        ("gear", io.bus_gear().map(|g| g.round() as i64).into()),
        ("engine", engine.map(|e| e.0).into()),
        ("rpm", engine.and_then(|e| e.1).map(|r| r as f64).into()),
        ("electrics", engine.map(|e| e.2).into()),
        ("doors_open", io.bus_doors_open().into()),
        ("indicator", io.bus_indicator().map(indicator).into()),
        ("headlights", io.bus_headlights().map(|h| h as i64).into()),
        ("dirt", io.bus_dirt().map(|d| d as f64).into()),
        ("passengers", io.bus_passengers().map(|p| p.0).into()),
    ];
    if let Some((odo, session)) = io.bus_km() {
        r.push(("odometer", odo.into()));
        r.push(("km_today", session.into()));
    }
    if let Some([t, b, cl, s]) = io.bus_controls() {
        r.extend([("throttle", Value::from(t as f64)), ("brake", Value::from(b)), ("clutch", Value::from(cl)), ("steering", Value::from(s))]);
    }
    r.push(("fuel", first_var(c, FUEL).map(Value::from).into()));
    r.push(("handbrake", flag(c, HANDBRAKE)));
    r.push(("horn", flag(c, HORN)));
    r.push(("stop_request", flag(c, REQUEST)));
    rec(r)
}

pub static FNS: &[ApiFn] = &[
    def!("bus.state", "vehicle", [], "table or nil", "Everything a dashboard shows, in one table: `speed` (km/h, signed), `gear`, `engine` (running), `rpm`, `electrics`, `doors_open`, `indicator`, `headlights`, `dirt`, `passengers`, `odometer`, `km_today`, `throttle`, `brake`, `clutch`, `steering`, `fuel`, `handbrake`, `horn`, `stop_request` (a key is `nil` where the bus does not have it).", NEW, None, false, |c, a| state(c)),
    def!("bus.velocity", "vehicle", [], "number or nil", "The bus's speed in km/h, negative backwards.", NEW, None, false, |c, a| first_var(c, &["Velocity"]).map(Value::from)),
    def!("bus.velocity_vector", "vehicle", [], "x, y, z", "Its velocity in the world, m/s (x east, y north, z up).", NEW, None, true, |c, a| nums(c.io().bus_velocity())),
    def!("bus.acceleration", "vehicle", [], "across, along, up", "Its acceleration in its own frame, m/s² without gravity: to the right, forwards, up.", NEW, None, true, |c, a| nums(c.io().bus_acceleration())),
    def!("bus.orientation", "vehicle", [], "heading, pitch, bank", "Heading (degrees clockwise from north), pitch (nose up positive) and bank (right side down positive).", NEW, None, true, |c, a| nums(c.io().bus_orientation())),
    def!("bus.mass", "vehicle", [], "number or nil", "The bus's mass in kg.", NEW, None, false, |c, a| c.io().bus_mass()),
    def!("bus.odometer", "vehicle", [], "number or nil", "The bus's odometer in km.", NEW, None, false, |c, a| c.io().bus_km().map(|k| k.0)),
    def!("bus.km_today", "vehicle", [], "number or nil", "Kilometres driven this session (as the personnel file counts them).", NEW, None, false, |c, a| c.io().bus_km().map(|k| k.1)),
    def!("bus.doors", "vehicle", [], "list of numbers", "Each door leaf's position, front to back: 0 shut, 1 open (the scripts' `door_0`, `door_1`, ...).", NEW, None, false, |c, a| c.io().bus_doors().into_iter().map(|d| d as f64).collect::<Vec<_>>()),
    def!("bus.door", "vehicle", [p("n", "integer")], "boolean or nil", "Whether door leaf `n` (from 1) is open.", NEW, None, false, |c, a| {
        let n = a.int(0)?;
        Ok::<_, ApiError>(c.io().bus_doors().get((n - 1).max(0) as usize).filter(|_| n >= 1).map(|d| *d > 0.05))
    }),
    def!("bus.doors_open", "vehicle", [], "boolean or nil", "Whether any door is open (by the passengers' door flags where the bus has them).", NEW, None, false, |c, a| c.io().bus_doors_open()),
    def!("bus.door_count", "vehicle", [], "integer", "The doorways the door keys work, front to back.", NEW, None, false, |c, a| c.io().bus_door_count()),
    def!("bus.toggle_door", "vehicle", [o("n", "integer")], "boolean", "Presses the key of doorway `n` (from 1; 0 or none: all doors), as the player would.", NEW, VehicleWrite, false, |c, a| {
        let n = a.opt_int(0)?.unwrap_or(0).max(0) as usize;
        Ok::<_, ApiError>(c.io().bus_door_key(n))
    }),
    def!("bus.indicator", "vehicle", [], "string or nil", "The indicators: `\"off\"`, `\"left\"`, `\"right\"` or `\"hazard\"`.", NEW, None, false, |c, a| c.io().bus_indicator().map(indicator)),
    def!("bus.set_indicator", "vehicle", [p("state", "string")], "boolean", "Sets the indicators: `\"off\"`, `\"left\"`, `\"right\"` or `\"hazard\"`.", NEW, VehicleWrite, false, |c, a| {
        let want = match a.str(0)?.as_str() {
            "off" => 0,
            "left" => 1,
            "right" => 2,
            "hazard" => 3,
            s => return Err(ApiError(format!("omsi.bus.set_indicator: \"{s}\" is none of off, left, right, hazard"))),
        };
        Ok(c.io().bus_set_indicator(want))
    }),
    def!("bus.headlights", "vehicle", [], "integer or nil", "The headlights: 0 off, 1 side lights, 2 dipped, 3 high beam.", NEW, None, false, |c, a| c.io().bus_headlights().map(|h| h as i64)),
    def!("bus.interior_light", "vehicle", [], "number or nil", "The passenger room's light, 0 (off) to 1.", NEW, None, false, |c, a| c.io().bus_interior_light().map(Value::from)),
    def!("bus.set_interior_light", "vehicle", [p("on", "bool")], "boolean", "Switches the passenger room's light on or off.", NEW, VehicleWrite, false, |c, a| {
        let on = a.flag(0, true);
        c.io().bus_set_interior_light(on)
    }),
    def!("bus.engine", "vehicle", [], "running, rpm, electrics", "The engine: whether it runs, its rpm (where the bus shows one) and whether the electrics are on.", NEW, None, true, |c, a| multi(c.io().bus_engine().map(|(r, rpm, e)| [Value::Bool(r), rpm.map(|x| x as f64).into(), Value::Bool(e)]))),
    def!("bus.engine_running", "vehicle", [], "boolean or nil", "Whether the engine runs.", NEW, None, false, |c, a| c.io().bus_engine().map(|e| e.0)),
    def!("bus.rpm", "vehicle", [], "number or nil", "The engine's revolutions per minute.", NEW, None, false, |c, a| c.io().bus_engine().and_then(|e| e.1).map(|r| r as f64)),
    def!("bus.electrics", "vehicle", [], "boolean or nil", "Whether the bus's electrics are on (the main switch).", NEW, None, false, |c, a| {
        let e = c.io().bus_engine().map(|e| e.2);
        e.or_else(|| first_var(c, ELECTRICS).map(|v| v > 0.5))
    }),
    def!("bus.start_up", "vehicle", [], "string or nil", "Starts the bus up the way Shift+U does (the battery, the electrics, the engine) - or shuts a running bus down; what the game says it does.", NEW, VehicleWrite, false, |c, a| c.io().bus_start_up()),
    def!("bus.gear", "vehicle", [], "integer or nil", "The gear engaged (-1 reverse, 0 neutral), where the bus shows one.", NEW, None, false, |c, a| c.io().bus_gear().map(|g| g.round() as i64)),
    def!("bus.shift", "vehicle", [p("gear", "integer")], "boolean", "Puts a manual gearbox's lever in a gear (-1 reverse, 0 neutral).", NEW, VehicleWrite, false, |c, a| {
        let g = a.int(0)? as i32;
        Ok::<_, ApiError>(c.io().bus_shift(g))
    }),
    def!("bus.fuel", "vehicle", [], "number or nil", "The fuel in the tank, litres (the scripts' `engine_tank_content`).", NEW, None, false, |c, a| first_var(c, FUEL).map(Value::from)),
    def!("bus.dirt", "vehicle", [], "number or nil", "How dirty the bus is, 0 (clean) to 1.", NEW, None, false, |c, a| c.io().bus_dirt().map(Value::from)),
    def!("bus.damage", "vehicle", [], "table or nil", "`{crashes, last_impact_kj, repair_minutes}`: the bus's crashes, the energy of the last one and how long a repair would take (`nil`: nothing to repair).", NEW, None, false, |c, a| c.io().bus_damage().map(|(n, j, m)| rec(vec![("crashes", n.into()), ("last_impact_kj", ((j / 1000.0) as f64).into()), ("repair_minutes", m.map(|m| m as f64).into())]))),
    def!("bus.controls", "vehicle", [], "throttle, brake, clutch, steering", "What the bus drives with now: the pedals (0 to 1) and the steering (-1 left to 1 right), from the keys, the mouse or a controller.", NEW, None, true, |c, a| nums(c.io().bus_controls().map(|v| v.map(|x| x as f64)))),
    def!("bus.steering_angle", "vehicle", [], "angle, max", "The front wheels' angle and the most they turn, degrees (right positive).", NEW, None, true, |c, a| nums(c.io().bus_steer().map(|(s, m)| [s as f64, m as f64]))),
    def!("bus.wheels", "vehicle", [], "list of tables", "Each wheel: `{axle, side, rpm, radius, suspension, driven}` (`side` 0 left, 1 right; `suspension` the spring's travel).", NEW, None, false, |c, a| {
        Value::List(c.io().bus_wheels().into_iter().map(|w| rec(vec![("axle", w.axle.into()), ("side", w.side.into()), ("rpm", Value::from(w.rpm)), ("radius", Value::from(w.radius)), ("suspension", Value::from(w.suspension)), ("driven", w.driven.into())])).collect())
    }),
    def!("bus.horn", "vehicle", [], "boolean or nil", "Whether the horn sounds.", NEW, None, false, |c, a| flag(c, HORN)),
    def!("bus.sound_horn", "vehicle", [p("down", "bool")], "boolean", "Holds the horn (`true`) or lets it go.", NEW, VehicleWrite, false, |c, a| {
        let d = a.flag(0, true);
        c.io().bus_action("horn", d)
    }),
    def!("bus.handbrake", "vehicle", [], "boolean or nil", "Whether the parking brake is on.", NEW, None, false, |c, a| flag(c, HANDBRAKE)),
    def!("bus.toggle_handbrake", "vehicle", [], "boolean", "Puts the parking brake on or off, as its key does.", NEW, VehicleWrite, false, |c, a| {
        let io = c.io();
        io.bus_action("parking_brake_toggle", true) && io.bus_action("parking_brake_toggle", false)
    }),
    def!("bus.stop_brake", "vehicle", [], "boolean or nil", "Whether the stop brake (the door brake) holds the bus.", NEW, None, false, |c, a| flag(c, STOP_BRAKE)),
    def!("bus.kneeling", "vehicle", [], "boolean or nil", "Whether the bus kneels.", NEW, None, false, |c, a| flag(c, KNEELING)),
    def!("bus.stop_requested", "vehicle", [], "boolean or nil", "Whether a passenger has asked to stop.", NEW, None, false, |c, a| flag(c, REQUEST)),
    def!("bus.retarder", "vehicle", [], "number or nil", "The retarder's step, where the bus has one.", NEW, None, false, |c, a| first_var(c, RETARDER).map(Value::from)),
    def!("bus.action", "vehicle", [p("name", "string"), o("down", "bool")], "boolean", "A key action of the vehicles (`[vehicles]` of keyboard.cfg: `horn`, `parking_brake_toggle`, `kw_scheinwerfer_toggle`, `blinker_left_set`, `ticket_give`, ...): pressed and let go, or held (`down` true) and let go (`false`). `true` when the bus knows it.", NEW, VehicleWrite, false, |c, a| {
        let n = a.str(0)?;
        let io = c.io();
        Ok::<_, ApiError>(match a.get(1) {
            Value::Nil => io.bus_action(&n, true) && { io.bus_action(&n, false); true },
            v => io.bus_action(&n, v.truthy()),
        })
    }),
    def!("bus.trailers", "vehicle", [], "integer or nil", "Parts coupled behind the bus (an articulated bus's rear counts).", NEW, None, false, |c, a| c.io().bus_trailers()),
    def!("bus.destinations", "vehicle", [], "list of tables", "The destinations of the bus's depot file: `{index, code, name, all_exit}` (`index` from 1, for `set_destination`).", NEW, None, false, |c, a| {
        let (list, _) = c.io().bus_destinations();
        Value::List(list.into_iter().enumerate().map(|(i, t)| rec(vec![("index", Value::Int(i as i64 + 1)), ("code", Value::Int(t.code)), ("name", t.name.into()), ("all_exit", t.all_exit.into())])).collect())
    }),
    def!("bus.destination", "vehicle", [], "name, index", "The destination the bus shows and its index in `destinations` (nothing when none).", NEW, None, true, |c, a| {
        let (list, i) = c.io().bus_destinations();
        multi(i.filter(|i| *i >= 0).and_then(|i| list.get(i as usize).map(|t| [Value::Str(t.name.clone()), Value::Int(i + 1)])))
    }),
    def!("bus.set_destination", "vehicle", [p("index", "integer")], "boolean", "Shows destination `index` (from 1, as `destinations` lists them).", NEW, VehicleWrite, false, |c, a| {
        let i = a.int(0)?;
        Ok::<_, ApiError>(i >= 1 && c.io().bus_set_destination(i as usize - 1))
    }),
    def!("bus.set_line", "vehicle", [p("line", "string")], "boolean", "Types a line (route) into the bus's IBIS, as the player would.", NEW, VehicleWrite, false, |c, a| {
        let l = a.str(0)?;
        Ok::<_, ApiError>(c.io().bus_set_line(&l))
    }),
    def!("bus.number", "vehicle", [], "string or nil", "The bus's fleet number.", NEW, None, false, |c, a| c.io().bus_ident().map(|i| i.0)),
    def!("bus.file", "vehicle", [], "string or nil", "The bus's `.bus` file, relative to the game folder.", NEW, None, false, |c, a| c.io().bus_ident().map(|i| i.1)),
    def!("bus.passengers", "vehicle", [], "total, seated, standing", "The people aboard the bus: all, sitting, standing.", NEW, None, true, |c, a| multi(c.io().bus_passengers().map(|(t, s, st)| [Value::from(t), s.into(), st.into()]))),
    def!("bus.tickets", "vehicle", [], "list of tables", "The tickets the map sells: `{name, price, day_ticket}`.", NEW, None, false, |c, a| {
        let (list, _) = c.io().bus_tickets();
        Value::List(list.into_iter().map(|t| rec(vec![("name", t.name.into()), ("price", Value::from(t.price)), ("day_ticket", t.day_ticket.into())])).collect())
    }),
    def!("bus.ticket_request", "vehicle", [], "name, price", "The ticket the passenger at the cash desk asks for (nothing when nobody asks).", NEW, None, true, |c, a| {
        let (_, r) = c.io().bus_tickets();
        multi(r.map(|(n, p)| [Value::Str(n), Value::from(p)]))
    }),
    def!("bus.sales", "vehicle", [], "tickets, money", "Tickets sold this session and the money taken (the game knows no currency).", NEW, None, true, |c, a| multi(c.io().bus_sales().map(|(n, m)| [Value::from(n), Value::Num(m)]))),
    def!("bus.triggers", "vehicle", [], "list of strings", "The names of the bus's script triggers (for `trigger`, `press`).", NEW, None, false, |c, a| c.io().bus_triggers()),
    def!("bus.play_sound", "vehicle", [p("event", "string")], "boolean", "Plays the bus's own sound of this event (a trigger name of its sound files, `ev_...`).", NEW, VehicleWrite, false, |c, a| {
        let e = a.str(0)?;
        Ok::<_, ApiError>(c.io().bus_sound(&e))
    }),
    def!("bus.get_vars", "vehicle", [o("names", "table")], "table", "Many script variables at once: a table name -> value of the names given, or of every variable of the bus without a list.", NEW, None, false, |c, a| {
        let names: Vec<String> = match a.opt_table(0)? {
            Some(t) => t.items().iter().filter_map(Value::to_text).collect(),
            None => c.io().var_names().0,
        };
        let io = c.io();
        if !io.has_vehicle() {
            return Ok(Value::Map(Vec::new()));
        }
        Ok::<_, ApiError>(Value::Map(names.into_iter().filter_map(|n| io.var(&n).map(|v| (n, Value::from(v)))).collect()))
    }),
    def!("bus.get_strings", "vehicle", [o("names", "table")], "table", "The same for string variables.", NEW, None, false, |c, a| {
        let names: Vec<String> = match a.opt_table(0)? {
            Some(t) => t.items().iter().filter_map(Value::to_text).collect(),
            None => c.io().var_names().1,
        };
        let io = c.io();
        if !io.has_vehicle() {
            return Ok(Value::Map(Vec::new()));
        }
        Ok::<_, ApiError>(Value::Map(names.into_iter().filter_map(|n| io.string(&n).map(|v| (n, Value::Str(v)))).collect()))
    }),
    def!("bus.set_vars", "vehicle", [p("values", "table")], "integer", "Sets many script variables at once (a table name -> number); how many the bus has.", NEW, VehicleWrite, false, |c, a| {
        let t = a.table(0)?.clone();
        let io = c.io();
        if !io.has_vehicle() {
            return Ok(Value::Int(0));
        }
        let mut n = 0;
        if let Value::Map(m) = t {
            for (k, v) in m {
                if let (Some(x), Some(_)) = (v.as_f64(), io.var(&k)) {
                    io.set_var(&k, x as f32);
                    n += 1;
                }
            }
        }
        Ok::<_, ApiError>(n)
    }),
];
