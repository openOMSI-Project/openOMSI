//! `duty.*` and `timetable.*`: the player's duty - its trips, stops, times and delay - and the
//! map's timetable: lines, tours and their stops. Times are seconds since midnight.

use super::{def, o, p, rec, xyz};
use crate::api::{ApiError, ApiFn, Value};
use crate::io::{Duty, Stop, Trip};

const NEW: &str = crate::api::VERSION;

fn stop(s: &Stop, n: usize, next: Option<usize>) -> Value {
    let mut r = vec![("number", Value::from(n + 1)), ("name", s.name.trim().into()), ("id", Value::Int(s.id)), ("arrival", s.arr.into()), ("departure", s.dep.into()), ("stops", s.stops.into())];
    if let Some(p) = s.pos {
        r.extend(xyz(p));
    }
    if let Some(next) = next {
        r.push(("passed", (n < next).into()));
    }
    rec(r)
}

fn stops(t: &Trip, next: Option<usize>) -> Value {
    Value::List(t.stops.iter().enumerate().map(|(i, s)| stop(s, i, next)).collect())
}

fn trip(t: &Trip, n: usize) -> Value {
    rec(vec![("number", Value::from(n + 1)), ("name", t.name.trim().into()), ("line", t.line.trim().into()), ("terminus", t.terminus.trim().into()), ("departure", t.departure.into()), ("arrival", t.end.into()), ("stops", Value::from(t.stops.len()))])
}

fn duty(d: &Duty) -> Value {
    let next = d.current.stops.get(d.next);
    let prev = d.current.stops.iter().take(d.next).rev().find(|s| s.stops);
    rec(vec![
        ("line", d.line.trim().into()),
        ("tour", d.tour.trim().into()),
        ("trip", Value::from(d.trip + 1)),
        ("trips", Value::from(d.trips)),
        ("trip_name", d.current.name.trim().into()),
        ("terminus", d.current.terminus.trim().into()),
        ("departure", d.current.departure.into()),
        ("arrival", d.current.end.into()),
        ("stops", Value::from(d.current.stops.len())),
        ("next_stop", next.map(|s| stop(s, d.next, None)).into()),
        ("previous_stop", prev.map(|s| stop(s, d.current.stops.iter().position(|x| std::ptr::eq(x, s)).unwrap_or(0), None)).into()),
        ("at_stop", d.at_stop.into()),
        ("trip_done", d.trip_done.into()),
        ("delay", d.delay.into()),
    ])
}

pub static FNS: &[ApiFn] = &[
    def!("duty.active", "duty", [], "boolean", "Whether the player drives a duty (a line and tour of the timetable).", NEW, None, false, |c, a| c.io().duty().is_some()),
    def!("duty.get", "duty", [], "table or nil", "The duty now: `line`, `tour`, `trip` (its number in the duty, from 1), `trips`, `trip_name`, `terminus`, `departure`, `arrival`, `stops`, `next_stop` and `previous_stop` (each a stop: `{number, name, id, arrival, departure, stops, x, y, z}`), `at_stop`, `trip_done`, `delay` (seconds, late positive).", NEW, None, false, |c, a| c.io().duty().map(|d| duty(&d))),
    def!("duty.delay", "duty", [], "number or nil", "Seconds the bus is late (early negative), worked out between the stops as the game's timetable does.", NEW, None, false, |c, a| c.io().duty().map(|d| d.delay)),
    def!("duty.stops", "duty", [], "list of tables", "The stops of the trip now: `{number, name, id, arrival, departure, stops, passed, x, y, z}` (`stops` false: the bus passes it; `x, y, z` where the stop's place is known; `id` the map's object id).", NEW, None, false, |c, a| c.io().duty().map(|d| stops(&d.current, Some(d.next)))),
    def!("duty.next_stop", "duty", [], "table or nil", "The next stop of the trip (as `duty.stops` gives them).", NEW, None, false, |c, a| c.io().duty().and_then(|d| d.current.stops.get(d.next).map(|s| stop(s, d.next, None)))),
    def!("duty.at_stop", "duty", [], "boolean", "Whether the bus stands at the next stop (within 25 m of it).", NEW, None, false, |c, a| c.io().duty().is_some_and(|d| d.at_stop)),
    def!("duty.trips", "duty", [], "list of tables", "The trips of the duty: `{number, name, line, terminus, departure, arrival, stops}`.", NEW, None, false, |c, a| Value::List(c.io().duty_trips().iter().enumerate().map(|(i, t)| trip(t, i)).collect())),
    def!("duty.trip_stops", "duty", [p("trip", "integer")], "list of tables or nil", "The stops of a trip of the duty (from 1), as `duty.stops` gives them.", NEW, None, false, |c, a| {
        let n = a.int(0)?;
        Ok::<_, ApiError>(c.io().duty_trips().get((n - 1).max(0) as usize).filter(|_| n >= 1).map(|t| stops(t, None)))
    }),
    def!("duty.skip_stop", "duty", [], "string or nil", "Skips the next stop (the duty goes on to the one after); the name of the stop skipped.", NEW, WorldWrite, false, |c, a| c.io().duty_skip_next()),
    def!("duty.skip_to", "duty", [p("stop", "integer")], "boolean", "Makes stop `stop` (from 1) of the trip the next one, forwards or back.", NEW, WorldWrite, false, |c, a| {
        let n = a.int(0)?;
        Ok::<_, ApiError>(n >= 1 && c.io().duty_skip_to(n as usize - 1))
    }),
    def!("duty.start", "duty", [p("line", "string"), p("tour", "string"), o("trip", "integer"), o("stop", "integer")], "true, or false and the reason", "Takes a duty, as the game menu's \"Line and tour\" does: a line and tour of `timetable.lines`, its trip (from 1 in the order they leave; default the first) and the stop to start at (from 1 among those the trip calls at; default the first). The bus stays where it is.", NEW, WorldWrite, true, |c, a| {
        let (line, tour) = (a.str(0)?, a.str(1)?);
        let (trip, stop) = (a.opt_int(2)?.unwrap_or(1).max(1) as usize - 1, a.opt_int(3)?.unwrap_or(1).max(1) as usize - 1);
        Ok::<_, ApiError>(super::core::ok_or_reason(c.io().duty_start(&line, &tour, trip, stop)))
    }),
    def!("duty.finish", "duty", [], "boolean", "Gives the duty up (the bus drives on without a timetable); `true` when there was one.", NEW, WorldWrite, false, |c, a| c.io().duty_end()),
    def!("timetable.lines", "duty", [], "list of tables", "The map's lines: `{name, user_allowed, tours}`, each tour `{number, today, trips}` (`today`: it runs on the game's date).", NEW, None, false, |c, a| {
        Value::List(c.io().timetable_lines().into_iter().map(|l| rec(vec![("name", l.name.into()), ("user_allowed", l.user_allowed.into()), ("tours", Value::List(l.tours.into_iter().map(|t| rec(vec![("number", t.number.into()), ("today", t.today.into()), ("trips", t.trips.into())])).collect()))])).collect())
    }),
    def!("timetable.stops", "duty", [p("line", "string"), p("tour", "string"), o("trip", "integer")], "list of tables", "The stops of a tour's trips (or of its trip `trip`, from 1 in the order they leave) that the bus calls at: `{trip, station, name, departure}`.", NEW, None, false, |c, a| {
        let (line, tour) = (a.str(0)?, a.str(1)?);
        let trip = a.opt_int(2)?.map(|t| t.max(1) as usize - 1);
        Ok::<_, ApiError>(Value::List(c.io().timetable_stops(&line, &tour, trip).into_iter().map(|(t, s, n, d)| rec(vec![("trip", Value::from(t + 1)), ("station", Value::from(s + 1)), ("name", n.trim().into()), ("departure", d.into())])).collect()))
    }),
    def!("timetable.stop_names", "duty", [], "list of tables", "The stops the timetable knows: `{id, name}` (`id` the map's object id).", NEW, None, false, |c, a| Value::List(c.io().timetable_stop_names().into_iter().map(|(id, n)| rec(vec![("id", Value::Int(id)), ("name", n.trim().into())])).collect())),
    def!("timetable.buses", "duty", [], "list of tables", "The timetable buses of the AI on the road: `{id, line, tour, trip, terminus, departure, next_stop_id, at_stop, trip_done, delay, x, y, number}`.", NEW, None, false, |c, a| {
        Value::List(c.io().traffic_buses().into_iter().map(|b| rec(vec![("id", Value::Int(b.id as i64)), ("line", b.line.into()), ("tour", b.tour.into()), ("trip", b.trip.into()), ("terminus", b.terminus.into()), ("departure", b.departure.into()), ("next_stop_id", b.next_stop_id.map(Value::Int).into()), ("at_stop", b.at_stop.into()), ("trip_done", b.trip_done.into()), ("delay", b.delay.into()), ("x", b.x.into()), ("y", b.y.into()), ("number", b.number.into())])).collect())
    }),
];
