//! The vehicle as a page sees it: `window.omsi.vehicle`.
//!
//! OMSI buses name their script variables as they please (`door_0`, `Fahrertuer_Rechts`,
//! `elec_busbar_main` ...), so a page that wants "is the engine running?" would have to
//! know every bus. This module turns the variables into one **normalised snapshot** with
//! fixed names, using the same conventions the game itself uses (the engine check of the
//! start-up helper, the LAN pose, the door logic of the passengers). Anything a bus does not
//! have shows up as `null` (numbers) or `false` (switches), never as a missing property, so
//! a page can read `omsi.vehicle.engine.rpm` without guarding.
//!
//! Variables the snapshot does not cover are still reachable: `omsi.vars.num`,
//! `omsi.vars.str` and `omsi.getVar(name)` give every variable of the bus's own variable list.
//!
//! The snapshot is a tree of [`ApiValue`]s that any [`crate::htmltex::HtmlRenderer`] backend
//! can turn into its own objects. Numbers are rounded to a sensible step so that a value that
//! merely jitters in the last digit does not redraw the page every frame.
//!
//! # Layout (API version 1)
//!
//! | path | meaning |
//! | --- | --- |
//! | `api` | version of this layout (1) |
//! | `info.number`, `.ident`, `.yard`, `.route`, `.nextStop` | text variables of the bus |
//! | `motion.speedKmh`, `.heading`, `.pitch`, `.bank`, `.steeringDeg`, `.x`, `.y`, `.z`, `.odometerKm` | movement and place |
//! | `engine.running`, `.rpm`, `.throttle`, `.brake`, `.clutch`, `.gear`, `.tankContent` | drive train and pedals |
//! | `electrics.on`, `.busbarMain`, `.busbarAvailable`, `.failure` | on-board network |
//! | `battery.on` | battery switch (`null` when the bus has none) |
//! | `doors.count`, `.anyOpen`, `.list[i].number`, `.open`, `.isOpen` | door leaves, `list[0]` is door 1 |
//! | `passengers.onboard`, `.entries[i]`, `.exits[i]` (`number`, `open`, `requested`) | boarding state |
//! | `lights.headlights` (0-3), `.brake`, `.reverse`, `.fog`, `.indicator` (0-3), `.indicatorLeft`, `.indicatorRight`, `.hazard`, `.interior` | lamps |
//! | `brakes.parking`, `.stop`, `.kneeling` | brake and kneeling switches |
//! | `wipers.running` | windscreen wipers |
//! | `cabin.temperature` | cabin air, °C |
//! | `condition.dirt`, `.crashes`, `.lastImpactKJ`, `.streetCondition` | wear and road |
//! | `train.trailers` | coupled vehicles behind this one |
//! | `route.active` | the bus has a timetable (a duty) |
//! | `route.line`, `.destination` | line text and the text of the destination sign |
//! | `route.current` | the stop the bus is at or heading for: `index`, `name`, `arrival`, `departure` (`HH:MM`), `arrivalSec`, `departureSec` (s after midnight) |
//! | `route.terminus` | the last stop, same fields |
//! | `route.stops[i]` | `index`, `name`, `arrival`, `departure`, `arrivalSec`, `departureSec`, `served`, `current` |
//! | `route.nextIndex`, `.delaySec`, `.source` | next stop, delay (s), `timetable` / `ibis` / `none` |
//! | `route.ibis` | the IBIS's own numbers: `line`, `suffix`, `routeIndex`, `terminusIndex`, `terminusCode` |
//!
//! `window.omsi.time` (`hour`, `minute`, `second`, `asString`), `window.omsi.date` (`day`,
//! `month`, `year`, `asString`) and `window.omsi.locale` come from [`environment`].
//!
//! `window.omsi.depot` (see [`depot`]) lists what the depot file offers: `lines[]` (each with
//! its `routes[]`), `routes[]`, `destinations[]`. The page acts on it with
//! `omsi.setRoute(index)`, `omsi.setLine(text)` and `omsi.setDestination(index)`; with a
//! timetable `omsi.setNextStop(index)` skips to that stop.

use crate::vehicle::VehicleInstance;
use crate::SimClock;
use omsi_vehicle::hof::Hof;
use std::sync::RwLock;

static LOCALE: RwLock<String> = RwLock::new(String::new());

/// Set the interface language pages see as `omsi.locale` (ISO 639-1, e.g. `de`; empty = `en`).
pub fn set_locale(iso: &str) {
    *LOCALE.write().unwrap() = iso.trim().to_ascii_lowercase();
}

/// The current `omsi.locale`.
pub fn locale() -> String {
    let l = LOCALE.read().unwrap();
    if l.is_empty() { "en".to_string() } else { l.clone() }
}

/// A time of day (seconds, `tod`) on the clock's date as a timestamp: seconds since 1970-01-01
/// 00:00:00 of the simulation's calendar, without a time zone.
pub fn timestamp(clock: &SimClock, tod: f64) -> f64 {
    let leaps_before = |y: i64| (y - 1) / 4 - (y - 1) / 100 + (y - 1) / 400;
    let y = clock.year as i64;
    let days = (y - 1970) * 365 + leaps_before(y) - leaps_before(1970) + (clock.day_of_year.max(1) - 1) as i64;
    (days * 86_400) as f64 + tod
}

/// `window.omsi.time`, `.date` and `.locale`: the simulation clock and the interface language.
/// `asString` of the time is `HH:MM:SS`; of the date `DD.MM.YYYY` (`MM/DD/YYYY` for `en`).
pub fn environment(clock: &SimClock, locale: &str) -> ApiValue {
    let secs: i64 = if clock.time.is_finite() { clock.time.floor() as i64 } else { 0 };
    let t = secs.rem_euclid(86_400);
    let (h, m, s) = (t / 3600, (t % 3600) / 60, t % 60);
    let (d, mo) = clock.day_month();
    let y = clock.year;
    let date = if locale == "en" { format!("{mo:02}/{d:02}/{y:04}") } else { format!("{d:02}.{mo:02}.{y:04}") };
    map(vec![
        (
            "time",
            map(vec![
                ("hour", ApiValue::Num(h as f64)),
                ("minute", ApiValue::Num(m as f64)),
                ("second", ApiValue::Num(s as f64)),
                ("asString", ApiValue::Str(format!("{h:02}:{m:02}:{s:02}"))),
            ]),
        ),
        (
            "date",
            map(vec![
                ("day", ApiValue::Num(d as f64)),
                ("month", ApiValue::Num(mo as f64)),
                ("year", ApiValue::Num(y as f64)),
                ("asString", ApiValue::Str(date)),
            ]),
        ),
        ("locale", ApiValue::Str(locale.to_string())),
        ("timestamp", ApiValue::Num(timestamp(clock, secs.rem_euclid(86_400) as f64))),
        ("weather", page_weather().map(|w| w.api()).unwrap_or(ApiValue::Null)),
    ])
}

/// The weather as pages see it (`omsi.weather`): set by the game whenever the weather is
/// loaded or changes, `null` until then.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageWeather {
    /// Air temperature (°C).
    pub temperature: f32,
    /// Absolute humidity (g/m³), as `Weather_AbsHum`.
    pub abs_humidity: f32,
    /// How far one sees (m): the weather's `[fog]`.
    pub visibility: f32,
    /// The weather's `[clouds]` type (`-1` clear, `Cumulus 2`, `AddOn - Overcast 3` ...).
    pub clouds: String,
    /// `[precip]` type (0 none, 1 rain, 2 snow) and rate (0..255).
    pub precip: (f32, f32),
}

impl PageWeather {
    /// Relative humidity 0..1 (Magnus formula for the saturation).
    pub fn relative_humidity(&self) -> f32 {
        let t = self.temperature;
        let es = 6.112 * ((17.62 * t) / (243.12 + t)).exp();
        let sat = 216.7 * es / (273.15 + t);
        if sat > 0.0 { (self.abs_humidity / sat).clamp(0.0, 1.0) } else { 0.0 }
    }

    fn api(&self) -> ApiValue {
        let kind = if self.precip.0 >= 0.5 && self.precip.1 > 0.0 { self.precip.0.round() } else { 0.0 };
        map(vec![
            ("temperature", ApiValue::Num((self.temperature as f64 * 10.0).round() / 10.0)),
            ("humidity", ApiValue::Num((self.relative_humidity() as f64 * 100.0).round() / 100.0)),
            ("visibility", ApiValue::Num(self.visibility.round() as f64)),
            ("clouds", ApiValue::Str(self.clouds.trim().to_string())),
            ("precip", ApiValue::Num(kind as f64)),
            ("precipRate", ApiValue::Num(if kind > 0.0 { ((self.precip.1 / 255.0).clamp(0.0, 1.0) as f64 * 100.0).round() / 100.0 } else { 0.0 })),
        ])
    }
}

static PAGE_WEATHER: RwLock<Option<PageWeather>> = RwLock::new(None);

/// Tell the pages what the weather is (see [`PageWeather`]).
pub fn set_page_weather(w: PageWeather) {
    if let Ok(mut g) = PAGE_WEATHER.write() {
        *g = Some(w);
    }
}

fn page_weather() -> Option<PageWeather> {
    PAGE_WEATHER.read().ok().and_then(|g| g.clone())
}

/// `window.omsi.departures`: per stop key the departures as `{ line, destination, time,
/// stopsAway, load, delaySec, lastTrip }` (see [`Departure`]; unknown ones `null`), `time`
/// being a timestamp on the scale of `omsi.timestamp` (see [`timestamp`]).
pub fn departures(by_key: &std::collections::HashMap<String, Vec<Departure>>) -> ApiValue {
    let mut keys: Vec<&String> = by_key.keys().collect();
    keys.sort();
    let opt = |v: Option<f64>| v.map(ApiValue::Num).unwrap_or(ApiValue::Null);
    ApiValue::Map(
        keys.into_iter()
            .map(|k| {
                let list = by_key[k]
                    .iter()
                    .map(|d| {
                        map(vec![
                            ("line", ApiValue::Str(d.line.clone())),
                            ("destination", ApiValue::Str(d.destination.clone())),
                            ("time", ApiValue::Num(d.time.round())),
                            ("stopsAway", opt(d.stops_away.map(|n| n as f64))),
                            ("load", opt(d.load.map(|l| (l as f64 * 100.0).round() / 100.0))),
                            ("delaySec", opt(d.delay.map(f64::round))),
                            ("lastTrip", ApiValue::Bool(d.last_trip)),
                        ])
                    })
                    .collect();
                (k.clone(), ApiValue::List(list))
            })
            .collect(),
    )
}

/// One departure of a stop for the pages (`omsi.getDepartures`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Departure {
    pub line: String,
    pub destination: String,
    /// Departure as a timestamp ([`timestamp`]), delay included.
    pub time: f64,
    /// Stops the bus still has to call at up to this one (this one counted; 0 = standing
    /// here). `None` while the bus is not on the road.
    pub stops_away: Option<u32>,
    /// How full the bus is (0..1 of its places) - only a bus whose passengers are simulated.
    pub load: Option<f32>,
    /// How late the bus runs (s), when it is on the road.
    pub delay: Option<f64>,
    /// The last departure of its line at this stop today.
    pub last_trip: bool,
}

/// A value of the snapshot tree.
#[derive(Clone, Debug, PartialEq)]
pub enum ApiValue {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<ApiValue>),
    /// Properties in the order they are listed in.
    Map(Vec<(String, ApiValue)>),
}

impl ApiValue {
    /// A property of a [`ApiValue::Map`].
    pub fn get(&self, key: &str) -> Option<&ApiValue> {
        match self {
            ApiValue::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The same map with property `key` set to `value`.
    fn with(mut self, key: &str, value: ApiValue) -> ApiValue {
        if let ApiValue::Map(m) = &mut self {
            match m.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = value,
                None => m.push((key.to_string(), value)),
            }
        }
        self
    }
}

/// Everything the snapshot is made from. [`VehicleInstance::html_api_snapshot`] fills it from
/// a running vehicle; tests fill it by hand.
pub struct Inputs<'a> {
    /// A script variable by name (any letter case), `None` when the bus has none.
    pub var: &'a dyn Fn(&str) -> Option<f32>,
    /// A text variable by name (empty when the bus has none).
    pub text: &'a dyn Fn(&str) -> String,
    pub speed_kmh: f32,
    pub steer_deg: f32,
    pub heading: f64,
    pub pitch: f32,
    pub bank: f32,
    pub position: (f64, f64, f64),
    pub engine_running: bool,
    pub interior_light: f32,
    pub crashes: u32,
    pub last_impact_j: f32,
    pub dirt: f32,
    pub trailers: usize,
}

/// `x` rounded to `dp` decimals, as the number a page prints without float noise
/// (`12.3`, not `12.300000190734863`).
fn round_dp(x: f64, dp: i32) -> f64 {
    if !x.is_finite() {
        return 0.0;
    }
    let p = 10f64.powi(dp);
    (x * p).round() / p
}

fn num(x: f32, dp: i32) -> ApiValue {
    ApiValue::Num(round_dp(x as f64, dp))
}

fn opt(x: Option<f32>, dp: i32) -> ApiValue {
    x.map_or(ApiValue::Null, |v| num(v, dp))
}

fn map(items: Vec<(&str, ApiValue)>) -> ApiValue {
    ApiValue::Map(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// Most doors of a bus the snapshot lists, and the most boarding doors (`PAX_Entry0..7`).
const MAX_DOORS: usize = 16;
const MAX_PAX_DOORS: usize = 8;

/// `HH:MM` of a time in seconds since midnight (wraps at 24 h).
fn hhmm(sec: f64) -> String {
    let s = if sec.is_finite() { sec.round() as i64 } else { 0 };
    let s = s.rem_euclid(86_400);
    format!("{:02}:{:02}", s / 3600, (s % 3600) / 60)
}

/// The name a depot file gives a stop of a route list (its first display string), else the
/// ident without its `#` suffix.
fn stop_name(h: &Hof, ident: &str) -> String {
    let id = ident.split('#').next().unwrap_or("").trim();
    h.bus_stops
        .iter()
        .find(|b| b.ident.trim().eq_ignore_ascii_case(id))
        .and_then(|b| b.strings.first())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| id.to_string())
}

/// Everything [`route`] is made from.
pub struct RouteInputs<'a> {
    pub var: &'a dyn Fn(&str) -> Option<f32>,
    pub text: &'a dyn Fn(&str) -> String,
    pub hof: Option<&'a Hof>,
    /// The timetable's line, its delay (s) and stops (name, arrival, departure in s after
    /// midnight), and the index of the stop the bus is at or heading for.
    pub line: &'a str,
    pub delay_s: f32,
    pub stops: &'a [(String, f32, f32)],
    pub next: i32,
}

fn stop_value(k: usize, name: &str, times: Option<(f64, f64)>, served: bool, current: bool) -> ApiValue {
    let (arr, dep) = match times {
        Some((a, d)) => (Some(a), Some(d)),
        None => (None, None),
    };
    let clock = |x: Option<f64>| x.map_or(ApiValue::Null, |x| ApiValue::Str(hhmm(x)));
    let secs = |x: Option<f64>| x.map_or(ApiValue::Null, |x| ApiValue::Num(x.round()));
    map(vec![
        ("index", ApiValue::Num(k as f64)),
        ("name", ApiValue::Str(name.trim().to_string())),
        ("arrival", clock(arr)),
        ("departure", clock(dep)),
        ("arrivalSec", secs(arr)),
        ("departureSec", secs(dep)),
        ("served", ApiValue::Bool(served)),
        ("current", ApiValue::Bool(current)),
    ])
}

/// `omsi.vehicle.route`: line, destination, the stops with their planned times, the stop
/// the bus is at and the last one. Without a timetable the stops are those of the IBIS's
/// route (without times), when the bus has one.
pub fn route(i: &RouteInputs) -> ApiValue {
    let var = i.var;
    let n = i.stops.len();
    let active = n > 0;
    let next = if active { i.next.clamp(0, n as i32 - 1) as usize } else { 0 };
    let ibis_line = var("IBIS_LinieKurs");
    let route_index = var("IBIS_RouteIndex").filter(|r| *r >= 0.0).map(|r| r.round() as usize);

    let (stops, source): (Vec<ApiValue>, &str) = if active {
        let list = i
            .stops
            .iter()
            .enumerate()
            .map(|(k, (name, arr, dep))| stop_value(k, name, Some((*arr as f64, *dep as f64)), k < next, k == next))
            .collect();
        (list, "timetable")
    } else if let (Some(h), Some(r)) = (i.hof, route_index) {
        let idents = h.info_busstop_lists.get(r).map(|l| l.as_slice()).unwrap_or(&[]);
        let list = idents.iter().enumerate().map(|(k, id)| stop_value(k, &stop_name(h, id), None, false, false)).collect();
        (list, "ibis")
    } else {
        (Vec::new(), "none")
    };

    let current = if active {
        stops.get(next).cloned().unwrap_or(ApiValue::Null)
    } else {
        let name = (i.text)("act_busstop");
        if name.trim().is_empty() {
            ApiValue::Null
        } else {
            stop_value(0, &name, None, false, true).with("index", ApiValue::Null)
        }
    };
    let terminus = stops.last().cloned().unwrap_or(ApiValue::Null);

    let line = if !i.line.trim().is_empty() { i.line.trim().to_string() } else { (i.text)("IBIS_Complex_Line").trim().to_string() };
    let line = if line.is_empty() {
        ibis_line.filter(|n| *n > 0.5).map(|n| format!("{}", n.round() as i64)).unwrap_or_default()
    } else {
        line
    };

    map(vec![
        ("active", ApiValue::Bool(active)),
        ("source", ApiValue::Str(source.to_string())),
        ("line", ApiValue::Str(line)),
        ("destination", ApiValue::Str((i.text)("IBIS_terminus_name").trim().to_string())),
        ("delaySec", num(i.delay_s, 0)),
        ("nextIndex", if active { ApiValue::Num(next as f64) } else { ApiValue::Null }),
        ("current", current),
        ("terminus", terminus),
        ("stops", ApiValue::List(stops)),
        (
            "ibis",
            map(vec![
                ("line", opt(ibis_line, 0)),
                ("suffix", opt(var("IBIS_Linie_Suffix"), 0)),
                ("routeIndex", opt(var("IBIS_RouteIndex"), 0)),
                ("terminusIndex", opt(var("IBIS_TerminusIndex"), 0)),
                ("terminusCode", opt(var("IBIS_TerminusCode"), 0)),
            ]),
        ),
    ])
}

/// `omsi.depot`: what the bus's depot file offers a driver, for `omsi.setRoute(index)`
/// and `omsi.setDestination(index)`.
///
/// * `routes[i]`: `index`, `code` (the IBIS route code), `name`, `line`, `terminusCode`,
///   `destinationIndex` (-1: none), `destination` (the sign's text), `first`, `last`,
///   `stops[]` (names).
/// * `lines[]`: `line` and its `routes[]` (the same objects), in the depot file's order.
/// * `destinations[i]`: `index`, `code`, `id`, `name`.
pub fn depot(h: &Hof) -> ApiValue {
    let first_string = |ti: Option<usize>| ti.and_then(|t| h.termini.get(t)).map(|t| t.display_name()).unwrap_or_default();
    let mut routes: Vec<ApiValue> = Vec::new();
    let mut lines: Vec<(String, Vec<ApiValue>)> = Vec::new();
    for (i, t) in h.info_trips.iter().enumerate() {
        let terminus_code = omsi_cfg::parse_i32(&t.route);
        let ti = h.termini.iter().position(|x| x.code == terminus_code);
        let names: Vec<String> = h.info_busstop_lists.get(i).map(|l| l.iter().map(|s| stop_name(h, s)).collect()).unwrap_or_default();
        let code = omsi_cfg::parse_i32(&t.code);
        // `code` = line x 100 + route (the depot file's own rule); many depot files leave the
        // line column as a placeholder ("XXX"), so the line then comes from the code
        let raw_line = t.line.trim();
        let placeholder = raw_line.is_empty() || raw_line.chars().all(|c| c == 'x' || c == 'X');
        let line = if placeholder && code >= 100 { (code / 100).to_string() } else { raw_line.to_string() };
        let r = map(vec![
            ("index", ApiValue::Num(i as f64)),
            ("code", ApiValue::Num(code as f64)),
            ("name", ApiValue::Str(t.name.trim().to_string())),
            ("line", ApiValue::Str(line.clone())),
            ("terminusCode", ApiValue::Num(terminus_code as f64)),
            ("destinationIndex", ApiValue::Num(ti.map_or(-1.0, |x| x as f64))),
            ("destination", ApiValue::Str(first_string(ti))),
            ("first", ApiValue::Str(names.first().cloned().unwrap_or_default())),
            ("last", ApiValue::Str(names.last().cloned().unwrap_or_default())),
            ("stops", ApiValue::List(names.into_iter().map(ApiValue::Str).collect())),
        ]);
        match lines.iter_mut().find(|(l, _)| l.eq_ignore_ascii_case(&line)) {
            Some((_, v)) => v.push(r.clone()),
            None => lines.push((line, vec![r.clone()])),
        }
        routes.push(r);
    }
    let destinations: Vec<ApiValue> = h
        .termini
        .iter()
        .enumerate()
        .map(|(i, t)| {
            map(vec![
                ("index", ApiValue::Num(i as f64)),
                ("code", ApiValue::Num(t.code as f64)),
                ("id", ApiValue::Str(t.texture_id.clone())),
                ("name", ApiValue::Str(t.menu_name())),
                // the terminus strings as the depot file has them (`GetTerminusString`'s
                // indices): a page that draws the destination sign itself finds the
                // Krueger/K++ bitmap names (`{{folder\file.png*POLY}}`) here
                ("strings", ApiValue::List(t.strings.iter().map(|s| ApiValue::Str(s.clone())).collect())),
            ])
        })
        .collect();
    log::info!(
        "omsi.depot '{}': {} trip(s), {} line(s), {} destination(s), {} busstop(s), {} busstop list(s)",
        h.name.trim(),
        h.info_trips.len(),
        lines.len(),
        h.termini.len(),
        h.bus_stops.len(),
        h.info_busstop_lists.len()
    );
    if h.info_trips.is_empty() {
        log::warn!("omsi.depot '{}': no [infosystem_trip] entries parsed, omsi.depot.routes stays empty", h.name.trim());
    }
    for (i, t) in h.info_trips.iter().enumerate().take(40) {
        log::info!("omsi.depot route #{i}: code={:?} name={:?} route={:?} line(file)={:?}", t.code, t.name, t.route, t.line);
    }
    map(vec![
        ("name", ApiValue::Str(h.name.clone())),
        (
            "lines",
            ApiValue::List(
                lines
                    .into_iter()
                    .map(|(line, rs)| map(vec![("line", ApiValue::Str(line)), ("routes", ApiValue::List(rs))]))
                    .collect(),
            ),
        ),
        ("routes", ApiValue::List(routes)),
        ("destinations", ApiValue::List(destinations)),
    ])
}

/// Build the snapshot.
pub fn snapshot(i: &Inputs) -> ApiValue {
    let var = i.var;
    // the first of some alternative names that the bus has
    let first = |names: &[&str]| names.iter().find_map(|n| var(n));
    let flag = |names: &[&str]| names.iter().any(|n| var(n).unwrap_or(0.0) > 0.5);
    // a switch that reads as `null` when the bus has none of the variables
    let opt_flag = |names: &[&str]| first(names).map(|v| v > 0.5);
    let on = |name: &str| var(name).unwrap_or(0.0) > 0.5;
    let flag_val = |b: bool| ApiValue::Bool(b);

    // ---- info
    let info = map(vec![
        ("number", ApiValue::Str((i.text)("number"))),
        ("ident", ApiValue::Str((i.text)("ident"))),
        ("yard", ApiValue::Str((i.text)("yard"))),
        ("route", ApiValue::Str((i.text)("act_route"))),
        ("nextStop", ApiValue::Str((i.text)("act_busstop"))),
    ]);

    // ---- motion
    let odometer = var("kmcounter_km").map(|km| km as f64 + var("kmcounter_m").unwrap_or(0.0) as f64 / 1000.0);
    let motion = map(vec![
        ("speedKmh", num(i.speed_kmh, 1)),
        ("heading", ApiValue::Num(round_dp(i.heading, 1))),
        ("pitch", num(i.pitch, 2)),
        ("bank", num(i.bank, 2)),
        ("steeringDeg", num(i.steer_deg, 1)),
        ("x", ApiValue::Num(round_dp(i.position.0, 2))),
        ("y", ApiValue::Num(round_dp(i.position.1, 2))),
        ("z", ApiValue::Num(round_dp(i.position.2, 2))),
        ("odometerKm", odometer.map_or(ApiValue::Null, |v| ApiValue::Num(round_dp(v, 3)))),
    ]);

    // ---- engine (`engine_n` is in rpm)
    let engine = map(vec![
        ("running", flag_val(i.engine_running)),
        ("rpm", opt(first(&["engine_n", "engine_rpm", "motor_n", "motor_rpm"]), 0)),
        ("throttle", opt(var("throttle"), 2)),
        ("brake", opt(var("brake"), 2)),
        ("clutch", opt(var("clutch"), 2)),
        ("gear", opt(first(&["antrieb_getr_aktugang", "gear"]), 0)),
        ("tankContent", opt(var("engine_tank_content"), 1)),
    ]);

    // ---- electrics and battery
    let busbar_main = on("elec_busbar_main");
    let busbar_avail = on("elec_busbar_avail");
    let electrics = map(vec![
        ("on", flag_val(busbar_main || busbar_avail)),
        ("busbarMain", flag_val(busbar_main)),
        ("busbarAvailable", flag_val(busbar_avail)),
        ("failure", flag_val(on("elec_failure_general"))),
    ]);
    let battery = map(vec![(
        "on",
        opt_flag(&["elec_battery_on", "battery_on", "batterie_on"]).map_or(ApiValue::Null, ApiValue::Bool),
    )]);

    // ---- doors: `door_0`, `door_1` ... as far as the bus has them (0 shut, 1 open)
    let door_list: Vec<ApiValue> = (0..MAX_DOORS)
        .map_while(|n| var(&format!("door_{n}")).map(|open| (n, open)))
        .map(|(n, open)| {
            map(vec![
                ("number", ApiValue::Num((n + 1) as f64)),
                ("open", num(open, 2)),
                ("isOpen", ApiValue::Bool(open > 0.05)),
            ])
        })
        .collect();
    let door_open = |n: usize| var(&format!("door_{n}")).unwrap_or(0.0) > 0.05;

    // ---- boarding: `PAX_Entry<n>_Open/_Req`, `PAX_Exit<n>_Open/_Req`
    let pax = |kind: &str| -> Vec<ApiValue> {
        (0..MAX_PAX_DOORS)
            .filter_map(|n| {
                let open = var(&format!("PAX_{kind}{n}_Open"));
                let req = var(&format!("PAX_{kind}{n}_Req"));
                if open.is_none() && req.is_none() {
                    return None;
                }
                Some(map(vec![
                    ("number", ApiValue::Num((n + 1) as f64)),
                    ("open", ApiValue::Bool(open.unwrap_or(0.0) > 0.5)),
                    ("requested", ApiValue::Bool(req.unwrap_or(0.0) > 0.5)),
                ]))
            })
            .collect()
    };
    let entries = pax("Entry");
    let exits = pax("Exit");
    let pax_open = entries.iter().chain(exits.iter()).any(|d| d.get("open") == Some(&ApiValue::Bool(true)));
    // what the passengers are told is open, where the bus has that; else the door leaves
    // (the same rule the game's own hints use)
    let any_open = if entries.is_empty() && exits.is_empty() {
        (0..door_list.len()).any(door_open)
    } else {
        pax_open
    };
    let doors = map(vec![
        ("count", ApiValue::Num(door_list.len() as f64)),
        ("anyOpen", ApiValue::Bool(any_open)),
        ("list", ApiValue::List(door_list)),
    ]);
    let passengers = map(vec![
        ("onboard", opt(var("humans_count"), 0)),
        ("entries", ApiValue::List(entries)),
        ("exits", ApiValue::List(exits)),
    ]);

    // ---- lights
    let headlights = if on("lights_fern") {
        3
    } else if on("lights_abbl") || on("lights_main") || var("Spot_Select").is_some_and(|s| s >= 0.0) {
        2
    } else if on("lights_stand") {
        1
    } else {
        0
    };
    let (lamp_l, lamp_r) = (on("lights_blinker_l"), on("lights_blinker_r"));
    // the indicator switch where the script has one (0 off, 1 left, 2 right, 3 hazard),
    // else the lamps
    let indicator = if on("lights_sw_warnblinker") {
        3
    } else {
        match var("lights_sw_blinker") {
            Some(s) if (0.5..2.5).contains(&s) => s.round() as i32,
            _ => match (lamp_l, lamp_r) {
                (true, true) => 3,
                (true, false) => 1,
                (false, true) => 2,
                _ => 0,
            },
        }
    };
    let lights = map(vec![
        ("headlights", ApiValue::Num(headlights as f64)),
        ("brake", flag_val(on("lights_brems"))),
        ("reverse", flag_val(on("lights_rueckfahr"))),
        ("fog", flag_val(on("lights_nebelschluss"))),
        ("indicator", ApiValue::Num(indicator as f64)),
        ("indicatorLeft", flag_val(lamp_l)),
        ("indicatorRight", flag_val(lamp_r)),
        ("hazard", flag_val(indicator == 3)),
        ("interior", num(i.interior_light, 2)),
    ]);

    // ---- brakes, wipers, cabin, condition, train
    let brakes = map(vec![
        ("parking", flag_val(flag(&["bremse_feststell", "parking_brake"]))),
        ("stop", flag_val(flag(&["bremse_halte", "bremse_halte_sw", "bus_stop_brake"]))),
        ("kneeling", flag_val(flag(&["bremse_kneeling", "vdv_kneel", "ecas_kneel", "kneeling"]))),
    ]);
    let wipers = map(vec![("running", flag_val(flag(&["wiperrunning", "wiper_running"])))]);
    let cabin = map(vec![("temperature", opt(var("Cabinair_Temp"), 1))]);
    let condition = map(vec![
        ("dirt", num(i.dirt, 2)),
        ("crashes", ApiValue::Num(i.crashes as f64)),
        ("lastImpactKJ", num(i.last_impact_j / 1000.0, 1)),
        ("streetCondition", opt(var("StreetCond"), 2)),
    ]);
    let train = map(vec![("trailers", ApiValue::Num(i.trailers as f64))]);

    map(vec![
        ("api", ApiValue::Num(1.0)),
        ("info", info),
        ("motion", motion),
        ("engine", engine),
        ("electrics", electrics),
        ("battery", battery),
        ("doors", doors),
        ("passengers", passengers),
        ("lights", lights),
        ("brakes", brakes),
        ("wipers", wipers),
        ("cabin", cabin),
        ("condition", condition),
        ("train", train),
    ])
}

impl VehicleInstance {
    /// The normalised state of this vehicle for its HTML textures (see the module docs).
    pub fn html_env_snapshot(&self) -> ApiValue {
        environment(&self.host.clock, &locale())
    }

    /// The normalised state of this vehicle for its HTML textures (see the module docs).
    pub fn html_api_snapshot(&self) -> ApiValue {
        let var = |n: &str| self.var(n);
        let text = |n: &str| self.str_var(n);
        let mut api = snapshot(&Inputs {
            var: &var,
            text: &text,
            speed_kmh: self.physics.velocity_kmh(),
            steer_deg: self.physics.steer_deg,
            heading: self.heading,
            pitch: self.pitch,
            bank: self.bank,
            position: (self.position.x, self.position.y, self.position.z),
            engine_running: crate::startup::engine_running(self),
            interior_light: self.interior_light(),
            crashes: self.crashes,
            last_impact_j: self.last_impact,
            dirt: self.dirt,
            trailers: self.trailers.len(),
        });
        let route = route(&RouteInputs {
            var: &var,
            text: &text,
            hof: self.host.hof.as_deref(),
            line: &self.host.tt_line,
            delay_s: self.host.tt_delay,
            stops: &self.host.tt_stops,
            next: self.host.tt_busstop_index,
        });
        if let ApiValue::Map(m) = &mut api {
            m.push(("route".to_string(), route));
        }
        api
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn snap(vars: &[(&str, f32)], texts: &[(&str, &str)]) -> ApiValue {
        let v: HashMap<String, f32> = vars.iter().map(|(k, x)| (k.to_ascii_lowercase(), *x)).collect();
        let t: HashMap<String, String> = texts.iter().map(|(k, x)| (k.to_string(), x.to_string())).collect();
        let var = move |n: &str| v.get(&n.to_ascii_lowercase()).copied();
        let text = move |n: &str| t.get(n).cloned().unwrap_or_default();
        snapshot(&Inputs {
            var: &var,
            text: &text,
            speed_kmh: 42.349_998,
            steer_deg: 0.0,
            heading: 90.0,
            pitch: 0.0,
            bank: 0.0,
            position: (1.0, 2.0, 3.0),
            engine_running: true,
            interior_light: 0.5,
            crashes: 0,
            last_impact_j: 0.0,
            dirt: 0.0,
            trailers: 0,
        })
    }

    fn at<'a>(v: &'a ApiValue, path: &str) -> &'a ApiValue {
        path.split('.').fold(v, |cur, key| match (cur, key.parse::<usize>()) {
            (ApiValue::List(l), Ok(n)) => &l[n],
            _ => cur.get(key).unwrap_or_else(|| panic!("no {key} in {path}")),
        })
    }

    #[test]
    fn numbers_are_rounded_without_float_noise() {
        let s = snap(&[], &[]);
        assert_eq!(at(&s, "motion.speedKmh"), &ApiValue::Num(42.3));
    }

    #[test]
    fn a_bus_without_a_signal_reports_null_or_false() {
        let s = snap(&[], &[]);
        assert_eq!(at(&s, "engine.rpm"), &ApiValue::Null);
        assert_eq!(at(&s, "battery.on"), &ApiValue::Null);
        assert_eq!(at(&s, "doors.count"), &ApiValue::Num(0.0));
        assert_eq!(at(&s, "brakes.parking"), &ApiValue::Bool(false));
        assert_eq!(at(&s, "engine.running"), &ApiValue::Bool(true));
    }

    #[test]
    fn doors_are_listed_as_far_as_the_bus_has_them() {
        let s = snap(&[("door_0", 1.0), ("door_1", 0.0), ("door_2", 0.4)], &[]);
        assert_eq!(at(&s, "doors.count"), &ApiValue::Num(3.0));
        assert_eq!(at(&s, "doors.list.0.number"), &ApiValue::Num(1.0));
        assert_eq!(at(&s, "doors.list.0.isOpen"), &ApiValue::Bool(true));
        assert_eq!(at(&s, "doors.list.1.isOpen"), &ApiValue::Bool(false));
        assert_eq!(at(&s, "doors.list.2.open"), &ApiValue::Num(0.4));
        assert_eq!(at(&s, "doors.anyOpen"), &ApiValue::Bool(true));
        // a gap ends the list (door_4 without door_3 is not a door of this bus)
        let g = snap(&[("door_0", 0.0), ("door_2", 1.0)], &[]);
        assert_eq!(at(&g, "doors.count"), &ApiValue::Num(1.0));
    }

    #[test]
    fn what_the_passengers_are_told_decides_whether_a_door_is_open() {
        // a mod bus whose `door_0` is something else, but whose boarding doors are shut
        let s = snap(&[("door_0", 1.0), ("PAX_Entry0_Open", 0.0), ("PAX_Exit0_Open", 0.0)], &[]);
        assert_eq!(at(&s, "doors.anyOpen"), &ApiValue::Bool(false));
        let o = snap(&[("PAX_Entry0_Open", 1.0), ("PAX_Entry0_Req", 1.0)], &[]);
        assert_eq!(at(&o, "doors.anyOpen"), &ApiValue::Bool(true));
        assert_eq!(at(&o, "passengers.entries.0.requested"), &ApiValue::Bool(true));
    }

    #[test]
    fn engine_battery_lights_and_texts() {
        let s = snap(
            &[
                ("engine_n", 812.4),
                ("elec_busbar_main", 1.0),
                ("batterie_on", 1.0),
                ("lights_abbl", 1.0),
                ("lights_blinker_l", 1.0),
                ("bremse_feststell", 1.0),
            ],
            &[("number", "4711"), ("act_busstop", "Hauptbahnhof")],
        );
        assert_eq!(at(&s, "engine.rpm"), &ApiValue::Num(812.0));
        assert_eq!(at(&s, "electrics.on"), &ApiValue::Bool(true));
        assert_eq!(at(&s, "battery.on"), &ApiValue::Bool(true));
        assert_eq!(at(&s, "lights.headlights"), &ApiValue::Num(2.0));
        assert_eq!(at(&s, "lights.indicator"), &ApiValue::Num(1.0));
        assert_eq!(at(&s, "brakes.parking"), &ApiValue::Bool(true));
        assert_eq!(at(&s, "info.number"), &ApiValue::Str("4711".into()));
        assert_eq!(at(&s, "info.nextStop"), &ApiValue::Str("Hauptbahnhof".into()));
    }

    #[test]
    fn a_timetable_gives_the_route_with_its_stops() {
        let var = |_: &str| -> Option<f32> { None };
        let text = |n: &str| if n == "IBIS_terminus_name" { " Hauptbahnhof ".to_string() } else { String::new() };
        let stops = vec![("Depot".to_string(), 3600.0, 3660.0), ("Markt".to_string(), 3900.0, 3930.0), ("Bahnhof".to_string(), 4500.0, 4500.0)];
        let r = route(&RouteInputs { var: &var, text: &text, hof: None, line: " 5E ", delay_s: 61.4, stops: &stops, next: 1 });
        assert_eq!(at(&r, "active"), &ApiValue::Bool(true));
        assert_eq!(at(&r, "line"), &ApiValue::Str("5E".into()));
        assert_eq!(at(&r, "destination"), &ApiValue::Str("Hauptbahnhof".into()));
        assert_eq!(at(&r, "delaySec"), &ApiValue::Num(61.0));
        assert_eq!(at(&r, "current.name"), &ApiValue::Str("Markt".into()));
        assert_eq!(at(&r, "current.arrival"), &ApiValue::Str("01:05".into()));
        assert_eq!(at(&r, "current.departureSec"), &ApiValue::Num(3930.0));
        assert_eq!(at(&r, "terminus.name"), &ApiValue::Str("Bahnhof".into()));
        assert_eq!(at(&r, "stops.0.served"), &ApiValue::Bool(true));
        assert_eq!(at(&r, "stops.1.current"), &ApiValue::Bool(true));
        assert_eq!(at(&r, "stops.2.served"), &ApiValue::Bool(false));
    }

    #[test]
    fn without_a_timetable_the_route_is_quiet() {
        let var = |_: &str| -> Option<f32> { None };
        let text = |_: &str| String::new();
        let r = route(&RouteInputs { var: &var, text: &text, hof: None, line: "", delay_s: 0.0, stops: &[], next: 0 });
        assert_eq!(at(&r, "active"), &ApiValue::Bool(false));
        assert_eq!(at(&r, "current"), &ApiValue::Null);
        assert_eq!(at(&r, "terminus"), &ApiValue::Null);
        assert_eq!(at(&r, "source"), &ApiValue::Str("none".into()));
    }

    #[test]
    fn the_indicator_switch_wins_over_the_lamps() {
        let s = snap(&[("lights_sw_blinker", 2.0), ("lights_blinker_l", 1.0)], &[]);
        assert_eq!(at(&s, "lights.indicator"), &ApiValue::Num(2.0));
        let h = snap(&[("lights_sw_warnblinker", 1.0)], &[]);
        assert_eq!(at(&h, "lights.hazard"), &ApiValue::Bool(true));
    }

    #[test]
    fn departures_carry_stops_load_delay_and_last_trip() {
        let mut by = std::collections::HashMap::new();
        by.insert(
            "harbor a".to_string(),
            vec![
                Departure { line: "221".into(), destination: "X".into(), time: 100.4, stops_away: Some(2), load: Some(0.456), delay: Some(95.6), last_trip: true },
                Departure { line: "1".into(), destination: "Y".into(), time: 200.0, ..Default::default() },
            ],
        );
        let v = departures(&by);
        let l = match v.get("harbor a") { Some(ApiValue::List(l)) => l.clone(), _ => panic!() };
        assert_eq!(l[0].get("stopsAway"), Some(&ApiValue::Num(2.0)));
        assert_eq!(l[0].get("load"), Some(&ApiValue::Num(0.46)));
        assert_eq!(l[0].get("delaySec"), Some(&ApiValue::Num(96.0)));
        assert_eq!(l[0].get("lastTrip"), Some(&ApiValue::Bool(true)));
        assert_eq!(l[1].get("stopsAway"), Some(&ApiValue::Null));
        assert_eq!(l[1].get("load"), Some(&ApiValue::Null));
        assert_eq!(l[1].get("lastTrip"), Some(&ApiValue::Bool(false)));
    }

    #[test]
    fn the_weather_reaches_the_environment() {
        let w = PageWeather { temperature: 14.0, abs_humidity: 6.0, visibility: 49999.9, clouds: "AddOn - Cumulus 7".into(), precip: (1.0, 127.5) };
        // 14 °C holds about 12 g/m³: 6 g/m³ is half
        assert!((w.relative_humidity() - 0.5).abs() < 0.03);
        set_page_weather(w);
        let env = environment(&SimClock::default(), "ko");
        let x = env.get("weather").unwrap();
        assert_eq!(x.get("temperature"), Some(&ApiValue::Num(14.0)));
        assert_eq!(x.get("visibility"), Some(&ApiValue::Num(50000.0)));
        assert_eq!(x.get("clouds"), Some(&ApiValue::Str("AddOn - Cumulus 7".into())));
        assert_eq!(x.get("precip"), Some(&ApiValue::Num(1.0)));
        assert_eq!(x.get("precipRate"), Some(&ApiValue::Num(0.5)));
    }
}
