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
/// A stop's arrival later than this (s) is late, its departure earlier than minus that early
/// (the trip's report judges its stops the same way).
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
            if omsi_cfg::env::var_os("OMSI_DEBUG_CAREER").is_some() {
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
        self.penalty = (self.penalty - step_km / PENALTY_KM).max(0.0);
        // driving with a door open is the classic way to upset your passengers
        if v.abs() > 1.0 && (0..8).any(|i| bus.var(&format!("door_{i}")).unwrap_or(0.0) > 0.05) {
            self.door_seconds += dt;
        }
    }

    /// An event of weight `w` moves the driving penalty that part of the way to 1.
    fn penalise(&mut self, w: f64) {
        self.penalty += (1.0 - self.penalty) * w.clamp(0.0, 1.0);
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
        if self.stepped_in == 0 {
            return 100.0;
        }
        100.0 * self.content as f64 / self.stepped_in as f64
    }

    /// Ticket selling: the points over twice the tickets asked for (100 before any).
    pub fn ticket_rating(&self) -> f64 {
        if self.ticket_requests == 0 {
            return 100.0;
        }
        100.0 * self.ticket_points as f64 / (2.0 * self.ticket_requests as f64)
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

impl Career {
    /// Write this session into `~/.openomsi/sessions/<time>-<process>.json`, where the
    /// launcher adds it up into the driver's hours, experience and level (the process id
    /// keeps two games that end in the same second apart).
    pub fn write_session(&self, map: &str, bus: &str, line: Option<&str>, tour: Option<&str>) -> std::io::Result<()> {
        let dir = data_dir().join("sessions");
        std::fs::create_dir_all(&dir)?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let driver = self.driver_name();
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

    /// The name the sessions and the trips are written under: the personnel file's, else
    /// its file's.
    pub fn driver_name(&self) -> String {
        self.driver.as_ref().map(|d| d.name.trim().to_string()).filter(|n| !n.is_empty()).or_else(|| self.path.as_ref().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))).unwrap_or_else(|| "Driver".into())
    }

    /// Add a trip the player just finished (`trip_report`) to the driver's trips in
    /// `~/.openomsi/trips`, where the launcher's service record reads them: written as the
    /// trip ends, so that a game that does not close cleanly keeps it.
    pub fn write_trip(&self, run: &omsi_launcher_lib::TripRun, map: &str, bus: &str) -> std::io::Result<PathBuf> {
        self.write_trip_in(&data_dir(), run, map, bus)
    }

    /// `write_trip` into the data folder `data`.
    pub fn write_trip_in(&self, data: &Path, run: &omsi_launcher_lib::TripRun, map: &str, bus: &str) -> std::io::Result<PathBuf> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let run = omsi_launcher_lib::TripRun { time: now, driver: self.driver_name(), map: map.to_string(), bus: bus.to_string(), ..run.clone() };
        let path = omsi_launcher_lib::append_trip(data, &run).map_err(std::io::Error::other)?;
        log::info!("trip written to {}: line {} to {}, {} stops ({} early, {} late)", path.display(), run.line, run.terminus, run.stops, run.early, run.late);
        Ok(path)
    }
}

/// openOMSI's own data folder, `~/.openomsi` (the launcher's `data_dir`).
fn data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    home.join(".openomsi")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trip is written under the driver's name with the map, the bus and the time, into the
    /// folder given (never the real one in a test).
    #[test]
    fn a_trip_is_written_for_the_driver() {
        let data = std::env::temp_dir().join(format!("omsi-career-trip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let career = Career { driver: Some(Driver { name: "Luc".into(), ..Default::default() }), ..Default::default() };
        let run = omsi_launcher_lib::TripRun { line: "5".into(), terminus: "Rathaus".into(), stops: 12, late: 1, ..Default::default() };
        career.write_trip_in(&data, &run, "maps/Spandau/global.cfg", "Vehicles/SD200/SD200.bus").unwrap();
        let read = omsi_launcher_lib::trips_of(&data, "luc");
        assert_eq!(read.len(), 1);
        assert_eq!((read[0].driver.as_str(), read[0].map.as_str(), read[0].line.as_str(), read[0].stops, read[0].late), ("Luc", "maps/Spandau/global.cfg", "5", 12, 1));
        assert!(read[0].time > 1_600_000_000);
        // (without a personnel file: the file's name, else "Driver")
        let stem = Career { path: Some(PathBuf::from("Drivers/Anna.odr")), ..Default::default() };
        assert_eq!(stem.driver_name(), "Anna");
        assert_eq!(Career::default().driver_name(), "Driver");
        let _ = std::fs::remove_dir_all(&data);
    }

    /// OMSI's limits: a stop is late more than three minutes after its arrival, early more
    /// than two before its departure, both rounded to the second.
    #[test]
    fn stops_are_counted_as_omsi_counts_them() {
        let mut c = Career::default();
        c.stop_served(180.4, 0.0);
        c.stop_served(181.0, 0.0);
        c.stop_served(0.0, -120.4);
        c.stop_served(0.0, -121.0);
        assert_eq!(c.stops, [4, 1, 1]);
    }
}
