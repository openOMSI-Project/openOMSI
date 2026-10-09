//! The driver's personnel file: what OMSI keeps about a career across duties.
//!
//! OMSI's `.odr` holds the counters its personnel dialog shows: bus stops served (and how
//! many of those too early or too late), hectometres driven, crashes, hurt pedestrians and
//! abscondings, tickets sold with the takings, and three ratings (driving, passenger
//! comfort, ticket selling). This module collects the same things over a session and
//! merges them into the file, rated as OMSI rates them (see `omsi_content::Driver`'s
//! `[rating]`): a stop is late when the bus arrived more than 180 s after its time, early
//! when it left more than 120 s before; driving is 100 (1 − P), where every jolt, pedal
//! see-saw or crash moves P a part of the way to 1 and the kilometres driven wear it down.

use glam::DVec3;
use omsi_content::Driver;
use omsi_sim::VehicleInstance;
use std::path::{Path, PathBuf};

/// OMSI's jolts: the smoothed acceleration across over 3 m/s² or along
/// over 5 m/s², weighing 0.1, at most one a second.
const HARSH_ACROSS: f32 = 3.0;
const HARSH_ALONG: f32 = 5.0;
const JOLT_WEIGHT: f64 = 0.1;
/// More than four changes between throttle and brake (either over 0.2) within four seconds.
const SEESAW_WEIGHT: f64 = 0.05;
/// Kilometres that wear the penalty down by one (P −= km / 30).
const PENALTY_KM: f64 = 30.0;
/// A stop's arrival later than this (s) is late, its departure earlier than minus that early.
pub(crate) const LATE_ARRIVAL: f64 = 180.0;
pub(crate) const EARLY_DEPARTURE: f64 = -120.0;

pub struct Career {
    pub driver: Option<Driver>,
    pub path: Option<PathBuf>,
    /// Metres driven this session.
    pub metres: f64,
    /// Bus stops served, of those too early, of those too late.
    pub stops: [i32; 3],
    /// Crashes, hurt pedestrians, abscondings, of those heavy.
    pub crashes: [i32; 4],
    /// Tickets sold and the takings.
    pub tickets: (i32, f64),
    /// Harsh accelerations, and those with passengers aboard.
    pub harsh: i32,
    pub harsh_pax: i32,
    /// Seconds driven with a door open.
    pub door_seconds: f32,
    /// Passengers that reached the cash desk, and those the driver served.
    pub boarded: i32,
    pub served: i32,
    /// OMSI's rating counters this session (see `Humans`): stepped in, of those without
    /// a complaint; tickets asked for and the points for them.
    pub stepped_in: i32,
    pub content: i32,
    pub ticket_requests: i32,
    pub ticket_points: i32,
    /// The driving penalty P (0..1), carried on from the personnel file.
    pub penalty: f64,
    /// When the pedals changed between throttle and brake lately (seconds of the session),
    /// and which was last (1 throttle, −1 brake).
    seesaw: std::collections::VecDeque<f64>,
    pedal: i8,
    last_pos: Option<DVec3>,
    last_v: f32,
    last_heading: f64,
    /// The accelerations along and across, smoothed over a fixed time (not per frame).
    smooth: (f32, f32),
    /// Cool-down so one bad brake counts once, not once per frame.
    harsh_cool: f32,
    /// Hardest acceleration seen (m/s²), for tuning and for the run report.
    pub worst_accel: f32,
    /// Seconds of simulation this session.
    pub seconds: f64,
    /// What `save` has already put into the personnel file.
    written: Written,
    /// The jolts since the last `take_jolts`.
    jolts: Vec<Jolt>,
    /// The duty's trip being driven, rated on its own.
    trip: Option<TripRun>,
}

/// A jolt as it happened (Lua plugins' `jolt` event): the smoothed accelerations along and
/// across the bus (m/s², signed), its speed (km/h, forwards or backwards) and the people
/// riding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Jolt {
    pub along: f32,
    pub across: f32,
    pub speed_kmh: f32,
    pub riders: usize,
}

/// The trip of the duty being rated (Lua plugins' `trip_done` event): from where its
/// ratings count, and once its end was told, from where the next trip's do.
struct TripRun {
    /// The duty's run of the trip (`PlayerDuty::trip_run`).
    run: u64,
    from: RatingStart,
    /// Its end was told (`trip_ended`): the next trip counts from then - the people boarding
    /// at its first stop before the duty goes on to it are its own. None again when a page
    /// reopens the trip.
    next: Option<RatingStart>,
}

/// Where ratings count from: a driving penalty of their own (from 0, moved as the
/// session's is) and the people's counters then.
#[derive(Clone, Copy)]
struct RatingStart {
    penalty: f64,
    stepped_in: i32,
    content: i32,
    ticket_requests: i32,
    ticket_points: i32,
}

/// The counters of this session already written into the personnel file: a second save
/// (F9, then the end of the run) adds only what happened since, and it starts from the file
/// as it is then, so that another game driving under the same name keeps its part.
#[derive(Default, Clone, Copy)]
struct Written {
    metres: f64,
    stops: [i32; 3],
    crashes: [i32; 4],
    tickets: (i32, f64),
    counters: [i32; 4],
}

impl Default for Career {
    fn default() -> Self {
        Career {
            driver: None,
            path: None,
            metres: 0.0,
            stops: [0; 3],
            crashes: [0; 4],
            tickets: (0, 0.0),
            harsh: 0,
            harsh_pax: 0,
            door_seconds: 0.0,
            boarded: 0,
            served: 0,
            stepped_in: 0,
            content: 0,
            ticket_requests: 0,
            ticket_points: 0,
            penalty: 0.0,
            seesaw: Default::default(),
            pedal: 0,
            last_pos: None,
            last_v: 0.0,
            last_heading: 0.0,
            smooth: (0.0, 0.0),
            harsh_cool: 0.0,
            worst_accel: 0.0,
            seconds: 0.0,
            written: Written::default(),
            jolts: Vec::new(),
            trip: None,
        }
    }
}

impl Career {
    /// Load the driver whose file this is; a missing file starts a new career.
    pub fn load(root: &Path, rel: &str) -> Career {
        // an absolute path is used as it is; a relative one is written into the content
        // folder (the original installation is only read: its copy is where a driver that
        // has not played openOMSI yet starts from)
        let given = Path::new(rel);
        let (path, read) = if given.is_absolute() {
            (given.to_path_buf(), given.to_path_buf())
        } else {
            let own = crate::startup::content_dir().map(|c| c.join(rel)).unwrap_or_else(|| omsi_cfg::resolve_path(root, rel));
            let read = if own.exists() { own.clone() } else { omsi_cfg::resolve_path(root, rel) };
            (own, read)
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let driver = match Driver::load(&read) {
            Ok(d) => {
                log::info!(
                    "driver {}: {} bus stops ({} early, {} late), {:.0} km, {} crashes, {:.0} tickets",
                    d.name,
                    d.bus_stops[0],
                    d.bus_stops[1],
                    d.bus_stops[2],
                    d.hektom / 10.0,
                    d.crashes[0],
                    d.tickets[0]
                );
                Some(d)
            }
            Err(e) => {
                log::info!("new personnel file {} ({e})", path.display());
                None
            }
        };
        let penalty = driver.as_ref().map(|d| d.rating[0].clamp(0.0, 1.0)).unwrap_or(0.0);
        Career { driver, path: Some(path), penalty, ..Default::default() }
    }

    /// One simulation frame of the player's bus.
    pub fn tick(&mut self, dt: f32, bus: &VehicleInstance, riders: usize) {
        self.seconds += dt as f64;
        let v = bus.physics.velocity_kmh() / 3.6;
        // (a jump no driving could make - the admin's teleport, the workshop, a host moving
        // the bus off an occupied spawn - is not distance driven)
        let mut step_km = 0.0;
        if let Some(p) = self.last_pos {
            let step = (bus.position - p).length();
            if step <= (v.abs() * dt) as f64 * 2.0 + 2.0 {
                self.metres += step;
                step_km = step / 1000.0;
            }
        }
        self.last_pos = Some(bus.position);
        let raw_along = (v - self.last_v) / dt.max(1e-3);
        self.last_v = v;
        // lateral acceleration from how fast the bus is turning
        let mut dh = bus.heading - self.last_heading;
        self.last_heading = bus.heading;
        while dh > 180.0 {
            dh -= 360.0;
        }
        while dh < -180.0 {
            dh += 360.0;
        }
        let raw_across = v * (dh.to_radians() / dt.max(1e-3) as f64) as f32;
        // OMSI weighs the accelerations by the speed up to 1 m/s (standing, the body's
        // rocking is nothing) and smooths them (α = min(10 dt, 0.5))
        let weight = v.abs().min(1.0);
        let k = (10.0 * dt).min(0.5);
        self.smooth.0 += (raw_along * weight - self.smooth.0) * k;
        self.smooth.1 += (raw_across * weight - self.smooth.1) * k;
        let (along, across) = self.smooth;
        if v.abs() > 1.0 {
            self.worst_accel = self.worst_accel.max(along.abs()).max(across.abs());
        }
        self.harsh_cool = (self.harsh_cool - dt).max(0.0);
        if (along.abs() > HARSH_ALONG || across.abs() > HARSH_ACROSS) && self.harsh_cool <= 0.0 {
            self.harsh_cool = 1.0;
            self.harsh += 1;
            self.penalise(JOLT_WEIGHT);
            self.jolts.push(Jolt { along, across, speed_kmh: v.abs() * 3.6, riders });
            if omsi_cfg::flags::OMSI_DEBUG_CAREER.is_set() {
                log::info!("jolt after {:.0} m: along {along:+.1} across {across:+.1} m/s2 at {:.0} km/h", self.metres, v * 3.6);
            }
            if riders > 0 {
                self.harsh_pax += 1;
            }
        }
        // see-sawing between throttle and brake
        let c = bus.physics.controls;
        let pedal = if c.throttle > 0.2 { 1 } else if c.brake > 0.2 { -1 } else { self.pedal };
        if pedal != self.pedal && self.pedal != 0 {
            self.seesaw.push_back(self.seconds);
        }
        self.pedal = pedal;
        while self.seesaw.front().is_some_and(|t| self.seconds - t > 4.0) {
            self.seesaw.pop_front();
        }
        if self.seesaw.len() > 4 {
            self.seesaw.clear();
            self.penalise(SEESAW_WEIGHT);
        }
        // the kilometres wear the penalty down
        self.penalty = worn(self.penalty, step_km);
        self.trip_penalties(|p| worn(p, step_km));
        // driving with a door open is the classic way to upset your passengers
        if v.abs() > 1.0 && (0..8).any(|i| bus.var(&format!("door_{i}")).unwrap_or(0.0) > 0.05) {
            self.door_seconds += dt;
        }
    }

    /// An event of weight `w` moves the driving penalty that part of the way to 1.
    fn penalise(&mut self, w: f64) {
        self.penalty = penalised(self.penalty, w);
        self.trip_penalties(|p| penalised(p, w));
    }

    /// The trip's driving penalties (its own, and the next one's once its end was told)
    /// moved as `f` moves the session's.
    fn trip_penalties(&mut self, f: impl Fn(f64) -> f64) {
        if let Some(t) = self.trip.as_mut() {
            t.from.penalty = f(t.from.penalty);
            if let Some(n) = t.next.as_mut() {
                n.penalty = f(n.penalty);
            }
        }
    }

    /// Ratings counting from now.
    fn rating_start(&self) -> RatingStart {
        RatingStart { penalty: 0.0, stepped_in: self.stepped_in, content: self.content, ticket_requests: self.ticket_requests, ticket_points: self.ticket_points }
    }

    /// The jolts since the last call.
    pub fn take_jolts(&mut self) -> Vec<Jolt> {
        std::mem::take(&mut self.jolts)
    }

    /// The duty's trip run `run` (`PlayerDuty::trip_run`) is driven: another run than the one
    /// rated so far is rated from the end of that one when it was told (see `TripRun::next`),
    /// else from now.
    pub fn trip_driven(&mut self, run: u64) {
        if self.trip.as_ref().is_some_and(|t| t.run == run) {
            return;
        }
        let from = self.trip.as_ref().and_then(|t| t.next).unwrap_or_else(|| self.rating_start());
        self.trip = Some(TripRun { run, from, next: None });
    }

    /// A page reopened the trip of run `run` after its end was told: it goes on being rated
    /// from its start, and its end is told again.
    pub fn trip_reopened(&mut self, run: u64) {
        if let Some(t) = self.trip.as_mut().filter(|t| t.run == run) {
            t.next = None;
        }
    }

    /// No duty is driven: no trip is rated.
    pub fn no_trip(&mut self) {
        self.trip = None;
    }

    /// This career goes on from `other`'s, under another driver: the bus's motion as it
    /// was (else the first frame takes the speed it has for a jolt), the people's counters
    /// as they are (the people's own, of the session) and the trip being rated.
    pub fn go_on_from(&mut self, other: &mut Career) {
        self.last_pos = other.last_pos;
        self.last_v = other.last_v;
        self.last_heading = other.last_heading;
        self.smooth = other.smooth;
        self.harsh_cool = other.harsh_cool;
        self.pedal = other.pedal;
        self.seesaw = std::mem::take(&mut other.seesaw);
        self.tickets = other.tickets;
        self.boarded = other.boarded;
        self.served = other.served;
        self.stepped_in = other.stepped_in;
        self.content = other.content;
        self.ticket_requests = other.ticket_requests;
        self.ticket_points = other.ticket_points;
        // (in the other driver's file already: not again in this one's)
        self.written.tickets = other.tickets;
        self.written.counters = [other.content, other.ticket_requests, other.ticket_points, other.stepped_in];
        self.trip = other.trip.take();
    }

    /// The trip run `run` ended: its driving, comfort and ticket selling (per cent, as the
    /// session's are rated), once - None for a run that was not being rated (one the bus
    /// was never driven on: a duty taken at a trip's last stop) or was told already (and not
    /// reopened since, `trip_reopened`).
    pub fn trip_ended(&mut self, run: u64) -> Option<[f64; 3]> {
        let now = self.rating_start();
        let t = self.trip.as_mut().filter(|t| t.run == run && t.next.is_none())?;
        t.next = Some(now);
        let f = t.from;
        Some([
            100.0 * (1.0 - f.penalty.clamp(0.0, 1.0)),
            comfort_percent(now.content - f.content, now.stepped_in - f.stepped_in),
            ticket_percent(now.ticket_points - f.ticket_points, now.ticket_requests - f.ticket_requests),
        ])
    }

    /// A timetable stop the bus arrived at `arrival` seconds after its time and left
    /// `departure` seconds after its departure (the original, both
    /// rounded to whole seconds).
    pub fn stop_served(&mut self, arrival: f64, departure: f64) {
        self.stops[0] += 1;
        if departure.round() < EARLY_DEPARTURE {
            self.stops[1] += 1;
        }
        if arrival.round() > LATE_ARRIVAL {
            self.stops[2] += 1;
        }
    }

    /// A crash of `energy` joules at `speed` m/s: it weighs min(|v| / 5, 1) on the driving.
    pub fn crashed(&mut self, energy: f32, speed: f32) {
        self.crashes[0] += 1;
        if energy > 50_000.0 {
            self.crashes[3] += 1;
        }
        self.penalise((speed.abs() / 5.0).min(1.0) as f64);
    }

    /// Per cent, as the personnel dialog shows it: 100 (1 − P).
    pub fn driving_rating(&self) -> f64 {
        100.0 * (1.0 - self.penalty.clamp(0.0, 1.0))
    }

    /// Per cent of the people who stepped in without a complaint (100 before anybody).
    pub fn comfort_rating(&self) -> f64 {
        comfort_percent(self.content, self.stepped_in)
    }

    /// Ticket selling: the points over twice the tickets asked for (100 before any).
    pub fn ticket_rating(&self) -> f64 {
        ticket_percent(self.ticket_points, self.ticket_requests)
    }

    pub fn summary(&self) -> String {
        format!(
            "{:.2} km, {} stops ({} early, {} late), {} tickets for {:.2}, {} crashes, {} hurt, {} jolts (worst {:.1} m/s2); driving {:.0}%, comfort {:.0}%, ticket selling {:.0}%",
            self.metres / 1000.0,
            self.stops[0],
            self.stops[1],
            self.stops[2],
            self.tickets.0,
            self.tickets.1,
            self.crashes[0],
            self.crashes[1],
            self.harsh,
            self.worst_accel,
            self.driving_rating(),
            self.comfort_rating(),
            self.ticket_rating()
        )
    }

    /// Add this session to the personnel file and write it back: the counters are added,
    /// the driving penalty is this session's (it started from the file's).
    pub fn save(&mut self) -> std::io::Result<()> {
        let Some(path) = self.path.clone() else { return Ok(()) };
        let mut d = Driver::load(&path).ok().or_else(|| self.driver.clone()).unwrap_or_else(|| Driver {
            name: path.file_stem().and_then(|s| s.to_str()).unwrap_or("Driver").to_string(),
            sex: "M".into(),
            ..Default::default()
        });
        let w = self.written;
        for i in 0..3 {
            d.bus_stops[i] += self.stops[i] - w.stops[i];
        }
        // whole hectometres only (the file keeps them as a whole number); the rest is carried
        // to the next save - rounding each save gained or lost up to 50 m every time
        let hm = ((self.metres - w.metres) / 100.0).max(0.0).floor();
        d.hektom = d.hektom.round() + hm;
        for i in 0..4 {
            d.crashes[i] += self.crashes[i] - w.crashes[i];
        }
        d.tickets[0] += (self.tickets.0 - w.tickets.0) as f64;
        d.tickets[1] += self.tickets.1 - w.tickets.1;
        let now = [self.content, self.ticket_requests, self.ticket_points, self.stepped_in];
        d.rating[0] = self.penalty.clamp(0.0, 1.0);
        for (k, slot) in [1usize, 2, 3, 4].into_iter().enumerate() {
            d.rating[slot] += (now[k] - w.counters[k]) as f64;
        }
        d.save(&path)?;
        self.written = Written { metres: w.metres + hm * 100.0, stops: self.stops, crashes: self.crashes, tickets: self.tickets, counters: now };
        log::info!("personnel file {} updated: {}", path.display(), self.summary());
        self.driver = Some(d);
        Ok(())
    }
}

/// The driving penalty `p` after an event of weight `w`: that part of the way to 1.
fn penalised(p: f64, w: f64) -> f64 {
    p + (1.0 - p) * w.clamp(0.0, 1.0)
}

/// The driving penalty `p` after `km` kilometres driven: worn down by km / 30.
fn worn(p: f64, km: f64) -> f64 {
    (p - km / PENALTY_KM).max(0.0)
}

/// Per cent of the `stepped_in` people who stepped in without a complaint (`content`; 100
/// before anybody).
fn comfort_percent(content: i32, stepped_in: i32) -> f64 {
    if stepped_in <= 0 {
        return 100.0;
    }
    100.0 * content as f64 / stepped_in as f64
}

/// Ticket selling: the `points` over twice the tickets asked for (`requests`; 100 before any).
fn ticket_percent(points: i32, requests: i32) -> f64 {
    if requests <= 0 {
        return 100.0;
    }
    100.0 * points as f64 / (2.0 * requests as f64)
}

impl Career {
    /// Write this session into `~/.openomsi/sessions/<time>-<process>.json`, where the
    /// launcher adds it up into the driver's hours, experience and level (the process id
    /// keeps two games that end in the same second apart).
    pub fn write_session(&self, map: &str, bus: &str, line: Option<&str>, tour: Option<&str>) -> std::io::Result<()> {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
        let dir = home.join(".openomsi").join("sessions");
        std::fs::create_dir_all(&dir)?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let driver = self.driver.as_ref().map(|d| d.name.clone()).or_else(|| self.path.as_ref().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))).unwrap_or_else(|| "Driver".into());
        let v = serde_json::json!({
            "time": now,
            "driver": driver,
            "driver_file": self.path.as_ref().map(|p| p.to_string_lossy().to_string()),
            "map": map,
            "bus": bus,
            "line": line,
            "tour": tour,
            "seconds": self.seconds,
            "metres": self.metres,
            "stops": self.stops[0],
            "early": self.stops[1],
            "late": self.stops[2],
            "tickets": self.tickets.0,
            "cash": self.tickets.1,
            "crashes": self.crashes[0],
            "hurt": self.crashes[1],
            "jolts": self.harsh,
            "boarded": self.boarded,
            "served": self.served,
            "driving": self.driving_rating(),
            "comfort": self.comfort_rating(),
            "ticketing": self.ticket_rating(),
        });
        let path = dir.join(format!("{now}-{}.json", std::process::id()));
        std::fs::write(&path, serde_json::to_vec_pretty(&v)?)?;
        log::info!("session written to {}", path.display());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trip is rated on its own: driving from 100 at its start, comfort and ticket selling
    /// from the people's counters then; the session's driving goes on from the file's. Its
    /// end is told once, and only for the run being rated.
    #[test]
    fn a_trip_is_rated_from_its_start_and_its_end_told_once() {
        let mut c = Career { penalty: 0.3, stepped_in: 10, content: 8, ticket_requests: 2, ticket_points: 4, ..Default::default() };
        assert_eq!(c.trip_ended(1), None, "no trip rated yet");
        c.trip_driven(1);
        c.crashed(1000.0, 2.5);
        (c.stepped_in, c.content, c.ticket_requests, c.ticket_points) = (14, 11, 4, 6);
        assert!((c.driving_rating() - 35.0).abs() < 1e-9, "{}", c.driving_rating());
        // the same run goes on
        c.trip_driven(1);
        assert_eq!(c.trip_ended(2), None, "another run's end");
        assert_eq!(c.trip_ended(1), Some([50.0, 75.0, 50.0]));
        assert_eq!(c.trip_ended(1), None, "told once");
        c.trip_driven(3);
        c.no_trip();
        assert_eq!(c.trip_ended(3), None);
    }

    /// The next trip counts from the end of the one before: people boarding at its first
    /// stop before the duty goes on to it are its own; a jolt there is its own too.
    #[test]
    fn the_next_trip_counts_from_the_end_of_the_one_before() {
        let mut c = Career::default();
        c.trip_driven(1);
        assert_eq!(c.trip_ended(1), Some([100.0; 3]));
        // the layover: four people board, one complains, two tickets, one with the wrong change
        (c.stepped_in, c.content, c.ticket_requests, c.ticket_points) = (4, 3, 2, 3);
        c.crashed(1000.0, 0.5);
        c.trip_driven(2);
        let r = c.trip_ended(2).unwrap();
        // (the crash's weight comes from an f32 speed)
        assert!((r[0] - 90.0).abs() < 1e-6, "{r:?}");
        assert_eq!([r[1], r[2]], [75.0, 75.0]);
    }

    /// A trip reopened after its end was told goes on from its start, and its end is told
    /// again.
    #[test]
    fn a_reopened_trip_is_told_again_from_its_start() {
        let mut c = Career::default();
        c.trip_driven(1);
        c.crashed(1000.0, 2.5);
        assert_eq!(c.trip_ended(1).map(|r| r[0]), Some(50.0));
        c.trip_reopened(1);
        c.crashed(1000.0, 2.5);
        assert_eq!(c.trip_ended(1).map(|r| r[0]), Some(25.0));
    }

    /// Another driver takes over while the bus moves: no jolt from the speed it has, the
    /// trip goes on being rated, and what the first driver's file has is not counted again.
    #[test]
    fn a_driver_taking_over_a_moving_bus_is_no_jolt() {
        let mut c = Career { last_v: 10.0, last_pos: Some(DVec3::ZERO), tickets: (10, 21.0), ..Default::default() };
        c.trip_driven(5);
        c.crashed(1000.0, 2.5);
        let mut next = Career::default();
        next.go_on_from(&mut c);
        assert_eq!(next.last_v, 10.0);
        assert_eq!(next.trip_ended(5).map(|r| r[0]), Some(50.0));
        assert_eq!(next.written.tickets, (10, 21.0));
    }
}
