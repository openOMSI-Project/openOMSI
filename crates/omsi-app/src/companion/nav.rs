//! The navigator on a phone or tablet: what the game's navigator shows, as data for the
//! companion's page, so that a tablet beside the wheel (or a phone in the holder) is the
//! navigator - the map with the route, the bus and its stops, the next turn, the next stop
//! with its time and the delay, the duty board under it and the duty sheet beside it.
//!
//! Nothing is worked out twice: the route, the turn, the way to the next stop, the street,
//! the speed limit and the "recalculated" note are the navigator's own (read through
//! `Navigator::companion_look`, at the end of `navigator.rs`), the board and the sheet are
//! `nav_duty`'s rows, the IBIS codes are the ones the game types (`schedule::ibis_target`).
//! What is here turns them into what the page draws:
//!
//! * the route as a line ([`route_line`]): the lanes' points, each with how far along the
//!   route it is, thinned to its bends; the page cuts it where the bus is (`along`);
//! * the roads around a place ([`RoadIndex`]): the city map's carriageways, kept in cells
//!   and handed out by the server for the part of the map a device shows;
//! * the live picture ([`live_json`]): where the bus is and what the navigator says, five
//!   times a second while a device shows the map;
//! * the duty board and sheet ([`board_json`], [`sheet_json`]), the IBIS codes
//!   ([`ibis_json`]) and the report at the end of a trip ([`TripWatch`]).
//!
//! Coordinates are the map's (metres, x east, y north); headings are compass degrees.

use crate::nav_duty::{Punctuality, Row, SheetRow, Status, StopState, TripState};
use crate::schedule::PlannedTrip;
use glam::{DVec2, DVec3};
use hashbrown::HashMap;
use omsi_sim::traffic::Network;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// What the in-game navigator follows, as `Navigator::companion_look` hands it over.
pub(crate) struct NavLook<'a> {
    /// The network the route's lanes belong to (the whole map's, else the traffic's).
    pub net: Option<&'a Network>,
    /// The whole map's network and its version, once it was read.
    pub map: Option<(u64, Arc<Network>)>,
    /// The route: its lanes, the one the bus is on and how far along it (m), and whether
    /// the bus is on it.
    pub lanes: &'a [usize],
    pub progress: usize,
    pub s: f32,
    pub on_route: bool,
    pub note: Option<RouteNote>,
    /// The next turn: which way (-1 left, 1 right, 2 back), how sharp (degrees), how far
    /// (m) and the street it turns into.
    pub turn: Option<(i32, f32, f64, Option<String>)>,
    /// The way to the next stop along the route (m).
    pub next_dist: Option<f64>,
    pub street: Option<&'a str>,
    pub limit: Option<f32>,
    /// The bus's speed over the last while (m/s), for the time to the next stop.
    pub speed_avg: f32,
    /// What the jams on the route ahead cost (s).
    pub jam_cost: f32,
    /// The places of the map's objects (stops beyond the loaded tiles).
    pub places: &'a HashMap<i64, DVec3>,
    /// The player's own pins set on the city map (`nav_pins`), in the order the route takes
    /// them; whether they are a diversion of the trip, whether the bus is at their destination,
    /// and what they have to say for a moment (a key, and the via's number).
    pub pins: Vec<NavPin>,
    pub diversion: bool,
    pub arrived: bool,
    pub pin_note: Option<(&'static str, usize)>,
}

/// A pin of the player's own: where, which (a via's number, 0 the destination), its name and
/// how far along the route it is (m).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NavPin {
    pub at: DVec2,
    pub via: usize,
    pub name: String,
    pub dist: Option<f64>,
}

/// What the navigator's bottom bar says about the route instead of the way to the stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteNote {
    Recalculated,
    Rerouting,
    OffRoute,
}

impl RouteNote {
    pub(crate) fn key(self) -> &'static str {
        match self {
            RouteNote::Recalculated => "recalculated",
            RouteNote::Rerouting => "rerouting",
            RouteNote::OffRoute => "off_route",
        }
    }
}

/// Tenths of a metre (or second) are as fine as a device needs.
fn r1(x: f64) -> f64 {
    if x.is_finite() { (x * 10.0).round() / 10.0 } else { 0.0 }
}

// --- the route as a line ------------------------------------------------------------------

/// The route as one line: its points, how far along the route each is (m), and where each of
/// the route's lanes begins along it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RouteLine {
    pub pts: Vec<DVec2>,
    pub along: Vec<f64>,
    pub lane_start: Vec<f64>,
}

impl RouteLine {
    /// How far along the route the bus is: on lane `progress` of it, `s` metres along that.
    pub(crate) fn along_at(&self, progress: usize, s: f32) -> Option<f64> {
        self.lane_start.get(progress).map(|a| a + s.max(0.0) as f64)
    }
}

/// The route's lanes of `net` as a line, thinned to its bends (within `TOLERANCE`).
pub(crate) fn route_line(net: &Network, lanes: &[usize]) -> RouteLine {
    const TOLERANCE: f64 = 0.4;
    let mut pts: Vec<DVec2> = Vec::new();
    let mut along: Vec<f64> = Vec::new();
    let mut lane_start = Vec::with_capacity(lanes.len());
    let mut start = 0.0f64;
    for &l in lanes {
        lane_start.push(start);
        let Some(lane) = net.lanes.get(l) else { continue };
        for (q, d) in lane.points.iter().zip(&lane.dist) {
            let p = q.truncate();
            // (one lane ends where the next begins)
            if pts.last().is_some_and(|last| (*last - p).length() < 0.05) {
                continue;
            }
            pts.push(p);
            along.push(start + *d as f64);
        }
        start += lane.length() as f64;
    }
    let keep = bends(&pts, TOLERANCE);
    let (pts, along) = pts.into_iter().zip(along).zip(keep).filter(|(_, k)| *k).map(|(p, _)| p).unzip();
    RouteLine { pts, along, lane_start }
}

/// Which points of a line to keep so that none left out lies further than `tol` from the
/// line through the ones kept (Douglas-Peucker).
pub(crate) fn bends(pts: &[DVec2], tol: f64) -> Vec<bool> {
    let n = pts.len();
    let mut keep = vec![n < 3; n];
    if n < 3 {
        return keep;
    }
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((a, b)) = stack.pop() {
        let (pa, pb) = (pts[a], pts[b]);
        let ab = pb - pa;
        let len = ab.length();
        let mut worst = (0.0f64, 0usize);
        for (k, p) in pts.iter().enumerate().take(b).skip(a + 1) {
            let d = if len < 1e-9 { (*p - pa).length() } else { (*p - pa).perp_dot(ab).abs() / len };
            if d > worst.0 {
                worst = (d, k);
            }
        }
        if worst.0 > tol {
            keep[worst.1] = true;
            stack.push((a, worst.1));
            stack.push((worst.1, b));
        }
    }
    keep
}

/// A stop of the trip on the map: its number in the trip, its name, its times (s of the
/// day) and its place when it is known.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MapStop {
    pub k: usize,
    pub name: String,
    pub arr: f64,
    pub dep: f64,
    pub at: Option<DVec2>,
}

/// The stops `trip` serves (a depot run passes stations it does not stop at), placed where
/// the timetable or else the map (`places`) has them.
pub(crate) fn trip_stops(trip: &PlannedTrip, places: &HashMap<i64, DVec3>) -> Vec<MapStop> {
    trip.stops
        .iter()
        .enumerate()
        .filter(|(_, s)| s.stops)
        .map(|(k, s)| MapStop {
            k,
            name: s.name.trim().to_string(),
            arr: s.arr,
            dep: s.dep,
            at: s.position.filter(|p| *p != DVec3::ZERO).or_else(|| places.get(&s.object_id).copied()).map(|p| p.truncate()),
        })
        .collect()
}

/// What `/api/trip` answers: the route as a line (its points and how far along each is,
/// flat), the trip's stops, its line and terminus.
pub(crate) fn trip_map_json(version: u64, line: &RouteLine, stops: &[MapStop], trip: Option<(&str, &str)>) -> String {
    let pts: Vec<f64> = line.pts.iter().flat_map(|p| [r1(p.x), r1(p.y)]).collect();
    let along: Vec<f64> = line.along.iter().map(|a| r1(*a)).collect();
    let stops: Vec<Value> = stops
        .iter()
        .map(|s| {
            let at = s.at.map(|p| [r1(p.x), r1(p.y)]);
            json!({ "k": s.k, "name": s.name, "arr": s.arr.round(), "dep": s.dep.round(), "at": at })
        })
        .collect();
    json!({
        "v": version,
        "pts": pts,
        "along": along,
        "stops": stops,
        "line": trip.map(|t| t.0.trim()),
        "terminus": trip.map(|t| t.1.trim()),
    })
    .to_string()
}

// --- the roads around a place ---------------------------------------------------------------

/// Side of the cells the roads are kept in (m).
const CELL: f64 = 200.0;

/// A road of the map for the phone: its centre line (thinned to its bends), its width and
/// whether it is a main road.
struct Road {
    pts: Vec<DVec2>,
    width: f32,
    main: bool,
}

/// The map's roads (the city map's, `Navigator::companion_roads`), kept in cells so that the
/// part a device shows is found at once.
pub(crate) struct RoadIndex {
    /// The navigator's map version they were made from.
    pub version: u64,
    roads: Vec<Road>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

fn cell_of(p: DVec2) -> (i32, i32) {
    ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32)
}

impl RoadIndex {
    pub(crate) fn build(version: u64, roads: Vec<crate::navigator::MapRoad>) -> RoadIndex {
        let mut out = RoadIndex { version, roads: Vec::with_capacity(roads.len()), cells: HashMap::new() };
        for r in roads {
            let mut pts: Vec<DVec2> = r.points.iter().map(|p| p.truncate()).collect();
            pts.dedup_by(|b, a| (*a - *b).length() < 0.05);
            let keep = bends(&pts, 0.35);
            let pts: Vec<DVec2> = pts.into_iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| p).collect();
            if pts.len() < 2 {
                continue;
            }
            let i = out.roads.len() as u32;
            let mut seen: Vec<(i32, i32)> = Vec::new();
            for ab in pts.windows(2) {
                // every cell a stretch passes (stretches are short beside a cell)
                let steps = ((ab[1] - ab[0]).length() / (CELL * 0.5)).ceil().max(1.0) as usize;
                for k in 0..=steps {
                    let c = cell_of(ab[0].lerp(ab[1], k as f64 / steps as f64));
                    if !seen.contains(&c) {
                        seen.push(c);
                        out.cells.entry(c).or_default().push(i);
                    }
                }
            }
            out.roads.push(Road { pts, width: r.width, main: r.main });
        }
        out
    }

    pub(crate) fn len(&self) -> usize {
        self.roads.len()
    }

    /// The roads with a part within the square of half side `r` round `c`.
    fn near(&self, c: DVec2, r: f64) -> Vec<u32> {
        let (lo, hi) = (cell_of(c - DVec2::splat(r)), cell_of(c + DVec2::splat(r)));
        let mut out: Vec<u32> = Vec::new();
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                if let Some(v) = self.cells.get(&(x, y)) {
                    out.extend_from_slice(v);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// What `/api/roads` answers: the roads round `c` within `r` metres, thinned to `tol`
    /// metres (a device zoomed far out needs fewer points), each as its width and whether it
    /// is a main road (decimetres, 0 or 1) and its points in decimetres from the centre
    /// (rounded to whole metres), flat.
    pub(crate) fn json_near(&self, c: DVec2, r: f64, tol: f64) -> String {
        let c = c.round();
        let mut out = String::with_capacity(64 * 1024);
        out.push_str(&format!("{{\"v\":{},\"x\":{},\"y\":{},\"r\":{},\"roads\":[", self.version, c.x, c.y, r.round()));
        let mut first = true;
        for i in self.near(c, r) {
            let road = &self.roads[i as usize];
            let keep = if tol > 0.4 { bends(&road.pts, tol) } else { vec![true; road.pts.len()] };
            if !first {
                out.push(',');
            }
            first = false;
            out.push_str(&format!("[{},{}", (road.width * 10.0).round() as i32, road.main as u8));
            for (p, k) in road.pts.iter().zip(keep) {
                if k {
                    let d = (*p - c) * 10.0;
                    out.push_str(&format!(",{},{}", d.x.round() as i64, d.y.round() as i64));
                }
            }
            out.push(']');
        }
        out.push_str("]}");
        out
    }
}

// --- the live picture -------------------------------------------------------------------

/// What the live picture is made of (the game's side gathers it, see `Companion::nav`).
pub(crate) struct Live<'a> {
    pub time: f64,
    pub weekday: i32,
    pub bus: DVec3,
    pub heading: f64,
    pub speed_kmh: f32,
    pub look: Option<&'a NavLook<'a>>,
    /// How far along the route line the bus is (m), and the line's version.
    pub along: Option<f64>,
    pub trip_version: u64,
    pub roads_version: u64,
    /// The next stop: its number in the trip, name, planned arrival and whether it is the
    /// trip's last.
    pub next: Option<(usize, &'a str, f64, bool)>,
    /// How late the bus is (s), on a duty that keeps time.
    pub delay: Option<f64>,
    pub line: Option<&'a str>,
    pub terminus: Option<&'a str>,
    pub stop_requested: bool,
    pub temps: (f32, f32),
    pub passengers: Option<usize>,
    pub traffic: &'a [(DVec3, f64, u8, Option<String>)],
}

/// The live picture as `/api/nav` sends it.
pub(crate) fn live_json(l: &Live) -> Value {
    let look = l.look;
    let turn = look.and_then(|k| k.turn.as_ref()).map(|(dir, deg, dist, street)| json!({ "dir": dir, "deg": deg.round(), "dist": dist.round(), "street": street }));
    let next = l.next.map(|(k, name, arr, last)| {
        let dist = look.and_then(|x| x.next_dist);
        let eta = dist.map(|d| (d / look.map_or(5.0, |x| x.speed_avg.max(5.0)) as f64).round());
        json!({ "k": k, "name": name.trim(), "arr": arr.round(), "dist": dist.map(f64::round), "eta": eta, "last": last })
    });
    // the player's own pins: where, which (0 the destination), the name, how far and how long
    let pins: Vec<Value> = look
        .map(|k| {
            k.pins
                .iter()
                .map(|p| {
                    let eta = p.dist.map(|d| crate::nav_pins::eta(d, k.speed_avg).round());
                    json!({ "x": r1(p.at.x), "y": r1(p.at.y), "via": p.via, "name": p.name.trim(), "dist": p.dist.map(f64::round), "eta": eta })
                })
                .collect()
        })
        .unwrap_or_default();
    let ai: Vec<Value> = l
        .traffic
        .iter()
        .map(|(p, h, kind, line)| {
            let mut v = vec![json!(r1(p.x)), json!(r1(p.y)), json!(h.round()), json!(kind)];
            if let Some(line) = line {
                v.push(json!(line));
            }
            Value::Array(v)
        })
        .collect();
    json!({
        "t": l.time.floor(),
        "wd": l.weekday,
        "bus": [r1(l.bus.x), r1(l.bus.y)],
        "h": r1(l.heading),
        "v": r1(l.speed_kmh as f64),
        "limit": look.and_then(|k| k.limit).map(|v| ((v / 5.0).round() * 5.0) as i32),
        "street": look.and_then(|k| k.street),
        "turn": turn,
        "next": next,
        "delay": l.delay.map(f64::round),
        "punctuality": l.delay.map(|d| punctuality_key(crate::nav_duty::punctuality(d))),
        "note": look.and_then(|k| k.note).map(RouteNote::key),
        "jam": look.map(|k| k.jam_cost.round()).filter(|c| *c >= 30.0),
        "on_route": look.is_some_and(|k| k.on_route),
        "along": l.along.map(r1),
        "trip": l.trip_version,
        "roads": l.roads_version,
        "line": l.line.map(str::trim).filter(|s| !s.is_empty()),
        "terminus": l.terminus.map(str::trim).filter(|s| !s.is_empty()),
        "req": l.stop_requested,
        "temps": [l.temps.0.round(), l.temps.1.round()],
        "pax": l.passengers,
        "ai": ai,
        "pins": pins,
        "diversion": look.is_some_and(|k| k.diversion),
        "arrived": look.is_some_and(|k| k.arrived),
        "pin_note": look.and_then(|k| k.pin_note).map(|(key, n)| json!({ "k": key, "n": n })),
    })
}

fn punctuality_key(p: Punctuality) -> &'static str {
    match p {
        Punctuality::Early => "early",
        Punctuality::OnTime => "on_time",
        Punctuality::Late => "late",
    }
}

// --- the duty board and the duty sheet ----------------------------------------------------

fn status_json(s: Status) -> Value {
    match s {
        Status::DepartsIn(t) => json!({ "k": "departs", "s": t.round() }),
        Status::Running(d) => json!({ "k": "running", "s": d.round(), "p": punctuality_key(crate::nav_duty::punctuality(d)) }),
        Status::Break(left) => json!({ "k": "break", "s": left.round() }),
        Status::Finished => json!({ "k": "finished" }),
    }
}

fn stop_state_key(s: StopState) -> &'static str {
    match s {
        StopState::Done => "done",
        StopState::Now => "now",
        StopState::Ahead => "ahead",
    }
}

/// The board under the small map (`nav_duty::board`) as the page draws it.
pub(crate) fn board_json(rows: &[Row]) -> Value {
    Value::Array(
        rows.iter()
            .map(|r| match r {
                Row::Head { line, terminus, index, count, time, status } => json!({ "row": "head", "line": line, "terminus": terminus, "index": index, "count": count, "time": time.round(), "status": status_json(*status) }),
                Row::Stop { planned, expected, name, state, last } => json!({ "row": "stop", "planned": planned.round(), "expected": expected.map(f64::round), "name": name, "state": stop_state_key(*state), "last": last }),
                Row::More { count, terminus } => json!({ "row": "more", "count": count, "terminus": terminus }),
                Row::Next { departure, line, terminus, pause, change } => json!({ "row": "next", "departure": departure.round(), "line": line, "terminus": terminus, "pause": pause, "change": change.as_ref().map(|c| [c.0.clone(), c.1.clone()]) }),
                Row::Change { line, tour } => json!({ "row": "change", "line": line, "tour": tour }),
                Row::Finished => json!({ "row": "finished" }),
                Row::NoDuty => json!({ "row": "no_duty" }),
            })
            .collect(),
    )
}

/// The sheet beside the city map (`nav_duty::sheet`) as the page draws it.
pub(crate) fn sheet_json(rows: &[SheetRow]) -> Value {
    Value::Array(
        rows.iter()
            .map(|r| match r {
                SheetRow::Trip { index, departure, arrival, line, terminus, stops, state } => json!({
                    "row": "trip", "index": index, "departure": departure.round(), "arrival": arrival.round(), "line": line, "terminus": terminus, "stops": stops,
                    "state": match state { TripState::Done => "done", TripState::Now => "now", TripState::Ahead => "ahead" },
                }),
                SheetRow::Stop { planned, name, state, last } => json!({ "row": "stop", "planned": planned.round(), "name": name, "state": stop_state_key(*state), "last": last }),
                SheetRow::Pause { minutes } => json!({ "row": "pause", "minutes": minutes }),
                SheetRow::Change { line, tour } => json!({ "row": "change", "line": line, "tour": tour }),
            })
            .collect(),
    )
}

/// The break at the terminus before the trip after `trip` (whole minutes), as the
/// navigator's break timer has it.
pub(crate) fn planned_break(trips: &[PlannedTrip], trip: usize) -> i64 {
    match (trips.get(trip), trips.get(trip + 1)) {
        (Some(a), Some(b)) => ((b.departure - a.end) / 60.0).round().max(0.0) as i64,
        _ => 0,
    }
}

// --- the IBIS ----------------------------------------------------------------------------

/// How far the game is with the IBIS of the duty: typing it now, typed, or not yet (it
/// types the codes once the duty order is signed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IbisState {
    Waiting,
    Typing,
    Typed,
}

impl IbisState {
    fn key(self) -> &'static str {
        match self {
            IbisState::Waiting => "waiting",
            IbisState::Typing => "typing",
            IbisState::Typed => "typed",
        }
    }
}

/// What goes into the bus's IBIS for `trip` from its stop `stop`, by the bus's depot file:
/// the line (and its letter's code), the route (as the full code a driver types, the line
/// with the route's two digits after it), the destination code and text, the stop the IBIS
/// should stand at; and the tour, which a driver gives as the course. The codes are the
/// ones the game types itself (`schedule::ibis_target`), so what the phone tells the driver
/// is what the IBIS gets. Without a depot file, or one that does not know the trip's
/// terminus, only the line and where it goes.
pub(crate) fn ibis_json(hof: Option<&omsi_vehicle::Hof>, trip: &PlannedTrip, stop: usize, tour: &str, state: IbisState) -> Value {
    let line = trip.line.trim();
    let terminus = trip.terminus.trim();
    let names: Vec<&str> = trip.stops.iter().map(|s| s.name.as_str()).collect();
    let stop_name = trip.stops.get(stop).map(|s| s.name.trim().to_string()).unwrap_or_default();
    let target = hof.and_then(|h| crate::schedule::ibis_target(h, line, terminus, &names, Some((stop, stop_name.as_str()))).map(|t| (h, t)));
    let mut out = json!({
        "line": line,
        "terminus": terminus,
        "tour": tour.trim(),
        "depot": hof.map(|h| h.name.trim()).filter(|n| !n.is_empty()),
        "state": state.key(),
        "known": target.is_some(),
    });
    if let Some((h, t)) = target {
        let dest = h.termini.get(t.terminus_index.max(0) as usize);
        let code = dest.map(|d| d.code).filter(|c| (0..1000).contains(c));
        out["code_line"] = json!(format!("{:03}", t.line));
        out["suffix"] = json!((t.suffix > 0).then(|| format!("{:02}", t.suffix)));
        out["route"] = json!(t.route.map(|r| format!("{r:02}")));
        out["route_full"] = json!(t.route.map(|r| format!("{}{r:02}", t.line)));
        out["dest"] = json!(code.map(|c| format!("{c:03}")));
        // (the route takes the destination with it; the code is typed only without one)
        out["dest_typed"] = json!(t.terminus_code.is_some());
        out["dest_text"] = json!(dest.and_then(|d| d.strings.iter().map(|s| s.trim()).find(|s| !s.is_empty())).unwrap_or(terminus));
        out["stop"] = json!(t.stop + 1);
        out["stop_name"] = json!(stop_name);
    }
    out
}

// --- the report at the end of a trip -------------------------------------------------------

/// How long after the bus reached a trip's last stop its report is made: the last stop
/// counts once the bus has stood there a moment.
const REPORT_AFTER: Duration = Duration::from_millis(2500);

/// A trip as the report names it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TripInfo {
    pub index: usize,
    pub count: usize,
    pub line: String,
    pub terminus: String,
    pub departure: f64,
    pub end: f64,
}

/// Follows the duty from trip to trip and makes the report at the end of each: the stops
/// served on it, of those too early and too late (OMSI's counting, the personnel file's:
/// `Career::stops`), and how late the bus reached its last stop. The page shows it as a
/// card once; the session keeps the counts in the personnel file as it always did.
#[derive(Debug, Default)]
pub(crate) struct TripWatch {
    duty: String,
    trip: Option<TripInfo>,
    /// The career's counters when the trip began.
    from: [i32; 3],
    done_at: Option<(Instant, f64)>,
    reported: bool,
    seq: u64,
    pub report: Option<Value>,
}

impl TripWatch {
    /// The duty now (`duty`: its key, empty for none; `free`: a free drive, which is not
    /// judged), the trip under way, whether the bus is at its last stop, the career's stop
    /// counters and the game's time of day. True when a new report was made.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn follow(&mut self, duty: &str, free: bool, trip: Option<TripInfo>, done: bool, counters: [i32; 3], clock: f64, now: Instant) -> bool {
        let Some(trip) = trip.filter(|_| !duty.is_empty() && !free) else {
            self.duty.clear();
            self.trip = None;
            return false;
        };
        if self.duty != duty || self.trip.is_none() {
            self.duty = duty.to_string();
            self.start(trip, counters);
            return false;
        }
        let mut made = false;
        if self.trip.as_ref().is_some_and(|t| t.index != trip.index) {
            // (the trip ended before its last stop: what it served counts all the same)
            if !self.reported && counters[0] > self.from[0] {
                made = self.make(counters, None, clock);
            }
            self.start(trip, counters);
            return made;
        }
        if done && !self.reported {
            let (at, _) = *self.done_at.get_or_insert((now, clock - trip.end));
            if now.duration_since(at) >= REPORT_AFTER {
                let arrival = self.done_at.map(|d| d.1);
                made = self.make(counters, arrival, clock);
            }
        }
        made
    }

    fn start(&mut self, trip: TripInfo, counters: [i32; 3]) {
        self.trip = Some(trip);
        self.from = counters;
        self.done_at = None;
        self.reported = false;
    }

    /// The report of the trip followed, made at `clock` (s of the day).
    fn make(&mut self, counters: [i32; 3], arrival: Option<f64>, clock: f64) -> bool {
        let Some(t) = self.trip.as_ref() else { return false };
        self.reported = true;
        self.seq += 1;
        let served = (counters[0] - self.from[0]).max(0);
        let early = (counters[1] - self.from[1]).clamp(0, served);
        let late = (counters[2] - self.from[2]).clamp(0, served);
        self.report = Some(json!({
            "seq": self.seq,
            "index": t.index,
            "count": t.count,
            "line": t.line.trim(),
            "terminus": t.terminus.trim(),
            "departure": t.departure.round(),
            "end": t.end.round(),
            "served": served,
            "early": early,
            "late": late,
            "on_time": (served - early - late).max(0),
            "arrival": arrival.map(f64::round),
            "arrival_p": arrival.map(|a| punctuality_key(crate::nav_duty::punctuality(a))),
            "at": clock.round(),
        }));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omsi_sim::traffic::{Lane, LaneKind};

    fn lane(a: (f64, f64), b: (f64, f64), n: usize) -> Lane {
        let pts: Vec<DVec3> = (0..=n).map(|i| DVec3::new(a.0, a.1, 0.0).lerp(DVec3::new(b.0, b.1, 0.0), i as f64 / n as f64)).collect();
        omsi_sim::traffic::LaneBuilder::polyline(pts, LaneKind::Street, 3.0)
    }

    #[test]
    fn the_route_becomes_one_line_that_knows_how_far_along_each_point_is() {
        let mut net = Network::default();
        net.lanes.push(lane((0.0, 0.0), (100.0, 0.0), 50));
        net.lanes.push(lane((100.0, 0.0), (100.0, 50.0), 25));
        net.lanes.push(lane((500.0, 500.0), (600.0, 500.0), 10));
        let line = route_line(&net, &[0, 1]);
        // a straight lane is its two ends; the corner is kept, the joint is not doubled
        assert_eq!(line.pts, vec![DVec2::new(0.0, 0.0), DVec2::new(100.0, 0.0), DVec2::new(100.0, 50.0)]);
        assert_eq!(line.along.len(), 3);
        assert!((line.along[1] - 100.0).abs() < 0.01 && (line.along[2] - 150.0).abs() < 0.01, "{:?}", line.along);
        assert_eq!(line.along_at(1, 20.0), Some(line.lane_start[1] + 20.0));
        assert!((line.lane_start[1] - 100.0).abs() < 0.01);
        assert_eq!(line.along_at(5, 0.0), None);
        // a lane the network does not have is passed over
        let gap = route_line(&net, &[0, 99, 1]);
        assert_eq!(gap.pts.len(), 3);
        assert_eq!(gap.lane_start.len(), 3);
    }

    #[test]
    fn a_line_keeps_its_bends_and_drops_what_lies_on_it() {
        let pts: Vec<DVec2> = (0..=20).map(|i| DVec2::new(i as f64, if i == 10 { 3.0 } else { 0.0 })).collect();
        let keep = bends(&pts, 0.5);
        let kept: Vec<usize> = keep.iter().enumerate().filter(|(_, k)| **k).map(|(i, _)| i).collect();
        assert_eq!(kept, vec![0, 9, 10, 11, 20]);
        assert_eq!(bends(&pts[..2], 0.5), vec![true, true]);
    }

    #[test]
    fn roads_are_found_by_where_a_device_looks() {
        let road = |a: (f64, f64), b: (f64, f64), main: bool| crate::navigator::MapRoad { points: vec![DVec3::new(a.0, a.1, 0.0), DVec3::new(b.0, b.1, 0.0)], width: 7.0, main };
        let idx = RoadIndex::build(3, vec![road((0.0, 0.0), (1000.0, 0.0), true), road((5000.0, 5000.0), (5100.0, 5000.0), false), road((1.0, 1.0), (1.0, 1.0), false)]);
        assert_eq!(idx.len(), 2, "a road of one point is none");
        // (the long road is in the cell at its middle too, not only at its ends)
        let j: Value = serde_json::from_str(&idx.json_near(DVec2::new(500.0, 30.0), 100.0, 0.0)).unwrap();
        assert_eq!(j["v"], 3);
        assert_eq!(j["roads"].as_array().unwrap().len(), 1);
        assert_eq!(j["roads"][0], json!([70, 1, -5000, -300, 5000, -300]));
        let far: Value = serde_json::from_str(&idx.json_near(DVec2::new(5050.0, 5000.0), 300.0, 2.0)).unwrap();
        assert_eq!(far["roads"].as_array().unwrap().len(), 1);
        assert_eq!(far["roads"][0][1], 0);
        let none: Value = serde_json::from_str(&idx.json_near(DVec2::new(-9000.0, 0.0), 300.0, 0.0)).unwrap();
        assert!(none["roads"].as_array().unwrap().is_empty());
    }

    fn info(index: usize) -> TripInfo {
        TripInfo { index, count: 3, line: "307".into(), terminus: "Bismarckplatz".into(), departure: 1000.0, end: 2000.0 }
    }

    #[test]
    fn a_trip_is_reported_once_a_moment_after_its_last_stop() {
        let t0 = Instant::now();
        let mut w = TripWatch::default();
        assert!(!w.follow("d1", false, Some(info(1)), false, [10, 1, 2], 900.0, t0));
        // under way: four stops served, one of them early, one late
        assert!(!w.follow("d1", false, Some(info(1)), false, [14, 2, 3], 1500.0, t0));
        // at the last stop, 70 s late: not at once ...
        assert!(!w.follow("d1", false, Some(info(1)), true, [14, 2, 3], 2070.0, t0));
        assert!(!w.follow("d1", false, Some(info(1)), true, [15, 2, 3], 2071.0, t0 + Duration::from_secs(1)));
        // ... but a moment later, with the last stop counted
        assert!(w.follow("d1", false, Some(info(1)), true, [15, 2, 3], 2072.0, t0 + Duration::from_secs(3)));
        let r = w.report.clone().unwrap();
        assert_eq!((r["seq"].as_u64(), r["served"].as_i64(), r["early"].as_i64(), r["late"].as_i64(), r["on_time"].as_i64()), (Some(1), Some(5), Some(1), Some(1), Some(3)));
        assert_eq!((r["arrival"].as_f64(), r["arrival_p"].as_str(), r["at"].as_f64()), (Some(70.0), Some("on_time"), Some(2072.0)));
        // once
        assert!(!w.follow("d1", false, Some(info(1)), true, [15, 2, 3], 2100.0, t0 + Duration::from_secs(9)));
        // the next trip starts from the counters then
        assert!(!w.follow("d1", false, Some(info(2)), false, [15, 2, 3], 2400.0, t0 + Duration::from_secs(10)));
        assert!(!w.follow("d1", false, Some(info(2)), false, [17, 2, 4], 2600.0, t0 + Duration::from_secs(11)));
        // given up before its end: reported with what it served when the next one starts
        assert!(w.follow("d1", false, Some(info(3)), false, [17, 2, 4], 2700.0, t0 + Duration::from_secs(12)));
        let r = w.report.clone().unwrap();
        assert_eq!((r["seq"].as_u64(), r["index"].as_u64(), r["served"].as_i64(), r["late"].as_i64(), r["arrival"].clone()), (Some(2), Some(2), Some(2), Some(1), Value::Null));
    }

    #[test]
    fn another_duty_or_a_free_drive_makes_no_report() {
        let t0 = Instant::now();
        let mut w = TripWatch::default();
        w.follow("d1", false, Some(info(1)), false, [0, 0, 0], 900.0, t0);
        // another duty: begun afresh, nothing reported for the old one
        assert!(!w.follow("d2", false, Some(info(1)), false, [3, 0, 0], 1000.0, t0));
        assert!(w.report.is_none());
        // a free drive is not judged
        assert!(!w.follow("d3", true, Some(info(1)), true, [9, 0, 0], 2100.0, t0 + Duration::from_secs(60)));
        assert!(!w.follow("", false, None, false, [9, 0, 0], 2100.0, t0 + Duration::from_secs(60)));
        assert!(w.report.is_none());
    }

    #[test]
    fn the_board_and_the_sheet_go_to_the_page_as_the_game_has_them() {
        let rows = vec![
            Row::Head { line: Some("307".into()), terminus: "Markgraf".into(), index: 1, count: 2, time: 69_000.4, status: Status::Running(250.0) },
            Row::Stop { planned: 68_700.0, expected: Some(68_950.0), name: "Barbarossastrasse".into(), state: StopState::Now, last: false },
            Row::More { count: 14, terminus: "Markgraf".into() },
            Row::Next { departure: 72_420.0, line: Some("307".into()), terminus: "Bismarckplatz".into(), pause: 39, change: Some(("307".into(), "16".into())) },
        ];
        let j = board_json(&rows);
        assert_eq!(j.as_array().unwrap().len(), 4);
        assert_eq!(j[0]["status"], json!({ "k": "running", "s": 250.0, "p": "late" }));
        assert_eq!(j[1]["state"], "now");
        assert_eq!(j[3]["change"], json!(["307", "16"]));
        let s = sheet_json(&[SheetRow::Trip { index: 1, departure: 1.0, arrival: 2.0, line: None, terminus: "Hof".into(), stops: 0, state: TripState::Done }, SheetRow::Pause { minutes: 7 }]);
        assert_eq!((s[0]["state"].as_str(), s[0]["line"].clone(), s[1]["minutes"].as_i64()), (Some("done"), Value::Null, Some(7)));
    }

    #[test]
    fn the_live_picture_says_what_the_panel_says() {
        let places = HashMap::new();
        let pins = vec![NavPin { at: DVec2::new(10.04, 20.0), via: 1, name: " Markt ".into(), dist: Some(420.4) }, NavPin { at: DVec2::new(-5.0, 7.0), via: 0, name: "Zoo".into(), dist: None }];
        let look = NavLook { net: None, map: None, lanes: &[], progress: 0, s: 0.0, on_route: true, note: Some(RouteNote::Recalculated), turn: Some((-1, 80.4, 213.0, Some("Kunstweg".into()))), next_dist: Some(300.0), street: Some("Barbarossastrasse"), limit: Some(48.0), speed_avg: 10.0, jam_cost: 75.0, places: &places, pins, diversion: false, arrived: false, pin_note: Some(("reached", 2)) };
        let ai = [(DVec3::new(10.0, 20.0, 0.0), 90.0, 2u8, Some("301".to_string())), (DVec3::new(5.0, 5.0, 0.0), 180.0, 0u8, None)];
        let l = Live { time: 68_700.6, weekday: 3, bus: DVec3::new(1.234, 5.678, 0.0), heading: 12.34, speed_kmh: 36.0, look: Some(&look), along: Some(123.45), trip_version: 4, roads_version: 2, next: Some((5, " Koenigsring ", 68_760.0, false)), delay: Some(-150.0), line: Some("307"), terminus: Some("Markgraf"), stop_requested: true, temps: (15.2, 19.7), passengers: Some(12), traffic: &ai };
        let j = live_json(&l);
        assert_eq!((j["t"].as_f64(), j["bus"].clone(), j["limit"].as_i64()), (Some(68_700.0), json!([1.2, 5.7]), Some(50)));
        assert_eq!(j["turn"], json!({ "dir": -1, "deg": 80.0, "dist": 213.0, "street": "Kunstweg" }));
        assert_eq!(j["next"], json!({ "k": 5, "name": "Koenigsring", "arr": 68_760.0, "dist": 300.0, "eta": 30.0, "last": false }));
        assert_eq!((j["note"].as_str(), j["jam"].as_f64(), j["punctuality"].as_str()), (Some("recalculated"), Some(75.0), Some("early")));
        assert_eq!(j["ai"], json!([[10.0, 20.0, 90.0, 2, "301"], [5.0, 5.0, 180.0, 0]]));
        // the player's own pins, as the city map has them
        assert_eq!(j["pins"][0], json!({ "x": 10.0, "y": 20.0, "via": 1, "name": "Markt", "dist": 420.0, "eta": 42.0 }));
        assert_eq!((j["pins"][1]["via"].as_u64(), j["pins"][1]["dist"].clone()), (Some(0), Value::Null));
        assert_eq!((j["diversion"].as_bool(), j["pin_note"].clone()), (Some(false), json!({ "k": "reached", "n": 2 })));
        // without the navigator: the bus, the time and the duty's stop, no route
        let bare = live_json(&Live { look: None, along: None, traffic: &[], ..l });
        assert_eq!((bare["turn"].clone(), bare["note"].clone(), bare["next"]["dist"].clone(), bare["next"]["eta"].clone()), (Value::Null, Value::Null, Value::Null, Value::Null));
    }
}
