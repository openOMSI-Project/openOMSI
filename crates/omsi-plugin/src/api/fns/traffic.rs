//! `traffic.*` and `people.*`: the AI vehicles and the people of the map.

use super::{def, multi, o, p, rec};
use crate::api::{ApiError, ApiFn, Ctx, Value};
use crate::io::AiCar;

const NEW: &str = crate::api::VERSION;

fn car(c: &AiCar, from: Option<[f64; 4]>) -> Value {
    let mut r = vec![
        ("id", Value::Int(c.id as i64)),
        ("kind", c.kind.into()),
        ("name", c.name.clone().into()),
        ("x", c.pos[0].into()),
        ("y", c.pos[1].into()),
        ("z", c.pos[2].into()),
        ("heading", c.pos[3].into()),
        ("speed", c.speed_kmh.into()),
        ("max_speed", c.max_speed_kmh.into()),
        ("waiting_for", if c.why.is_empty() { Value::Nil } else { c.why.into() }),
        ("standing", Value::from(c.standing)),
        ("braking", c.braking.into()),
        ("blinker", ["off", "left", "right"].get(c.blinker as usize).copied().unwrap_or("off").into()),
        ("line", c.line.clone().into()),
    ];
    if let Some(p) = from {
        r.push(("distance", (c.pos[0] - p[0]).hypot(c.pos[1] - p[1]).into()));
    }
    rec(r)
}

/// The cars within `r` m of the player's bus (all, without a bus or without `r`).
fn cars(c: &mut Ctx<'_>, r: Option<f64>) -> Vec<(f64, AiCar)> {
    let io = c.io();
    let me = io.position();
    let mut list: Vec<(f64, AiCar)> = io.traffic_list().into_iter().map(|x| (me.map_or(0.0, |p| (x.pos[0] - p[0]).hypot(x.pos[1] - p[1])), x)).collect();
    if let (Some(r), Some(_)) = (r, me) {
        list.retain(|(d, _)| *d <= r);
    }
    list
}

pub static FNS: &[ApiFn] = &[
    def!("traffic.list", "traffic", [o("radius", "number")], "list of tables", "The AI vehicles (within `radius` m of the player's bus, when given): `{id, kind, name, x, y, z, heading, speed, max_speed, waiting_for, standing, braking, blinker, line, distance}`. `kind`: `\"car\"`, `\"taxi\"`, `\"bus\"`, `\"truck\"`, `\"timetable_bus\"`, `\"tram\"`, `\"bicycle\"`; `waiting_for` why it waits or slows (`\"lead\"`, `\"light\"`, `\"yield\"`, `\"people\"`, ...); `standing` the seconds it has stood.", NEW, None, false, |c, a| {
        let r = a.opt_num(0)?;
        let me = c.io().position();
        Ok::<_, ApiError>(Value::List(cars(c, r).iter().map(|(_, x)| car(x, me)).collect()))
    }),
    def!("traffic.get", "traffic", [p("id", "integer")], "table or nil", "One AI vehicle by its id, as `traffic.list` gives them.", NEW, None, false, |c, a| {
        let id = a.int(0)? as u64;
        let me = c.io().position();
        Ok::<_, ApiError>(c.io().traffic_list().into_iter().find(|x| x.id == id).map(|x| car(&x, me)))
    }),
    def!("traffic.nearest", "traffic", [o("kind", "string")], "table or nil", "The AI vehicle nearest to the player's bus (of that `kind`, when given), with its `distance`.", NEW, None, false, |c, a| {
        let kind = a.opt_str(0)?;
        let me = c.io().position();
        let best = cars(c, None).into_iter().filter(|(_, x)| kind.as_deref().is_none_or(|k| k == x.kind)).min_by(|a, b| a.0.total_cmp(&b.0));
        Ok::<_, ApiError>(best.filter(|_| me.is_some()).map(|(_, x)| car(&x, me)))
    }),
    def!("traffic.ahead", "traffic", [o("reach", "number")], "table or nil", "The AI vehicle ahead of the player's bus within `reach` m (default 100) and 20° of its heading, with its `distance`: for a distance warning or a cruise control.", NEW, None, false, |c, a| {
        let reach = a.opt_num(0)?.unwrap_or(100.0);
        let Some(me) = c.io().position() else { return Ok(Value::Nil) };
        let h = me[3].to_radians();
        let (fx, fy) = (h.sin(), h.cos());
        let best = cars(c, Some(reach)).into_iter().filter(|(d, x)| {
            let (dx, dy) = (x.pos[0] - me[0], x.pos[1] - me[1]);
            let along = dx * fx + dy * fy;
            along > 0.0 && (dx * fy - dy * fx).abs() < along * 20f64.to_radians().tan() + 1.5 && *d > 0.5
        }).min_by(|a, b| a.0.total_cmp(&b.0));
        Ok::<_, ApiError>(best.map(|(_, x)| car(&x, Some(me))))
    }),
    def!("traffic.counts", "traffic", [], "driving, buses, asleep, parked", "The AI vehicles: driving, timetable buses among them, asleep out of range, parked.", NEW, None, true, |c, a| multi(c.io().traffic_counts().map(|n| n.map(Value::from)))),
    def!("traffic.density", "traffic", [], "cars, share", "How many cars the traffic keeps around the camera, and the share of them that do not run to a timetable (0 to 1).", NEW, None, true, |c, a| multi(c.io().traffic_density().map(|(n, s)| [Value::from(n), Value::from(s)]))),
    def!("traffic.set_density", "traffic", [p("cars", "integer"), o("share", "number")], "boolean", "Changes the traffic's amount (as the game menu's slider: 0 to 500 cars) and, when given, the share not running to a timetable; it fills up or thins out over the next seconds.", NEW, TrafficWrite, false, |c, a| {
        let n = a.int(0)?.clamp(0, 500) as usize;
        let share = a.opt_num(1)?.map(|s| s.clamp(0.0, 1.0) as f32);
        Ok::<_, ApiError>(c.io().traffic_set_density(Some(n), share))
    }),
    def!("traffic.remove", "traffic", [p("id", "integer")], "boolean", "Takes an AI vehicle off the road (a timetable bus too: its timetable forgets it); its passengers get out.", NEW, TrafficWrite, false, |c, a| {
        let id = a.int(0)? as u64;
        Ok::<_, ApiError>(c.io().traffic_remove(id))
    }),
    def!("traffic.clear", "traffic", [], "integer or nil", "Takes every car not running to a timetable off the road; how many went.", NEW, TrafficWrite, false, |c, a| c.io().traffic_clear()),
    def!("traffic.light_ahead", "traffic", [o("reach", "number")], "table or nil", "The traffic light the player's bus comes to within `reach` m (default 80): `{aspect, change_in, distance}` - `aspect` `\"red\"`, `\"red_yellow\"`, `\"green\"`, `\"green_yellow\"`, `\"yellow\"` or `\"dark\"`, `change_in` the seconds to its next change.", NEW, None, false, |c, a| {
        let r = a.opt_num(0)?.unwrap_or(80.0).clamp(1.0, 500.0);
        Ok::<_, ApiError>(c.io().traffic_light_ahead(r).map(|l| rec(vec![("aspect", l.aspect.into()), ("change_in", Value::from(l.change_in)), ("distance", l.distance.into())])))
    }),
    def!("people.counts", "people", [], "walking, waiting, riding", "The people of the map near the camera: walking, waiting at stops, riding a bus.", NEW, None, true, |c, a| multi(c.io().people_counts().map(|n| n.map(Value::from)))),
    def!("people.list", "people", [o("radius", "number")], "list of tables", "The people (within `radius` m of the player's bus, when given): `{id, x, y, z, state, aboard, ai_bus, stop, destination, ticket, complaint}`. `state`: `\"strolling\"`, `\"idle\"`, `\"standing\"`, `\"waiting\"`, `\"to_bus\"`, `\"boarding\"`, `\"riding\"`, `\"seated\"`, `\"leaving\"`, `\"to_stop\"`; `aboard` in the player's bus; `complaint` 0 to 3 (3: they leave).", NEW, None, false, |c, a| {
        let r = a.opt_num(0)?;
        let io = c.io();
        let me = io.position();
        Ok::<_, ApiError>(Value::List(io.people_list().into_iter().filter(|p| match (r, me) {
            (Some(r), Some(m)) => (p.pos[0] - m[0]).hypot(p.pos[1] - m[1]) <= r,
            _ => true,
        }).map(|p| rec(vec![
            ("id", Value::from(p.id)),
            ("x", p.pos[0].into()),
            ("y", p.pos[1].into()),
            ("z", p.pos[2].into()),
            ("state", p.state.into()),
            ("aboard", p.aboard.into()),
            ("ai_bus", p.in_ai_bus.map(|b| Value::Int(b as i64)).into()),
            ("stop", p.stop.map(Value::Int).into()),
            ("destination", p.destination.into()),
            ("ticket", p.ticket.into()),
            ("complaint", Value::Int(p.complaint as i64)),
        ])).collect()))
    }),
    def!("people.stops", "people", [], "list of tables", "The stops near the camera where people wait: `{id, name, x, y, z, waiting}`.", NEW, None, false, |c, a| {
        Value::List(c.io().people_stops().into_iter().map(|s| rec(vec![("id", Value::Int(s.id)), ("name", s.name.trim().into()), ("x", s.pos[0].into()), ("y", s.pos[1].into()), ("z", s.pos[2].into()), ("waiting", s.waiting.into())])).collect())
    }),
    def!("people.waiting", "people", [p("stop", "integer")], "integer", "How many people wait at a stop (its map object id).", NEW, None, false, |c, a| {
        let id = a.int(0)?;
        Ok::<_, ApiError>(c.io().people_stops().into_iter().find(|s| s.id == id).map_or(0, |s| s.waiting))
    }),
    def!("people.density", "people", [], "number or nil", "The people setting: 0 to 3, 1 the map's own amount.", NEW, None, false, |c, a| c.io().people_density().map(Value::from)),
    def!("people.set_density", "people", [p("value", "number")], "boolean", "Changes the people setting (0 to 3).", NEW, TrafficWrite, false, |c, a| {
        let v = a.num(0)?.clamp(0.0, 3.0) as f32;
        Ok::<_, ApiError>(c.io().people_set_density(v))
    }),
];
