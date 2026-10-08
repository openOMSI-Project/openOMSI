//! Timetable buses as traffic. A timetable bus is one of the town's AI cars (`AiCar`,
//! created by the same `Traffic::create_car` as every other car, driven by the same
//! following, lights, junctions, lane changes and passing) that carries a `BusService`:
//! the stops of its trip, the pull into the bay, the doors at each stop, the wait for the
//! departure time, the blinker and the pull back out, the end of the trip, and the people
//! aboard. Nothing of the player's bus is involved: no driver inputs, no throttle and brake
//! for the script to turn into motion - the car moves, the script only animates it.

use omsi_sim::traffic::{AiState, LaneKind, Network};
use omsi_sim::VehicleInstance;
use std::collections::VecDeque;

/// Script callbacks describe the whole trip, including stations on unloaded tiles.
/// The service's geometric queue only describes the part the bus can drive now.
pub struct AiTimetable {
    pub line: String,
    pub terminus: String,
    pub stops: Vec<(i64, String, f32, f32)>,
}

impl AiTimetable {
    pub fn install(&self, host: &mut omsi_sim::VehicleHost, next: Option<&Stop>) {
        host.schedule_active = 1.0;
        host.tt_line = self.line.clone();
        host.tt_stops = self.stops.iter().map(|s| (s.1.clone(), s.2, s.3)).collect();
        host.tt_stop_ids = self.stops.iter().map(|s| s.0).collect();
        host.tt_terminus_index = host
            .hof
            .as_ref()
            .and_then(|hof| {
                hof.termini
                    .iter()
                    .position(|t| t.texture_id == self.terminus)
            })
            .map_or(-1, |i| i as i32);
        host.tt_busstop_index = timetable_stop_index(host, next).unwrap_or(0) as i32;
        host.tt_delay = 0.0;
    }
}

fn timetable_stop_index(host: &omsi_sim::VehicleHost, next: Option<&Stop>) -> Option<usize> {
    let stop = next?;
    // A circular trip may visit the same object twice. Its departure time identifies
    // the visit, rather than a lookup by object id alone. f32 times lose subsecond
    // precision on later days, so compare after casting to the callback's type.
    host.tt_stop_ids
        .iter()
        .zip(&host.tt_stops)
        .position(|(id, data)| *id == stop.id && data.2 == stop.depart as f32)
}

/// One stop of the trip, on the car's route.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stop {
    /// Index into the car's route of the lane the stop is on.
    pub ri: usize,
    /// Where the front of the bus comes to rest, along that lane (m).
    pub s: f32,
    /// How far right of the lane's middle the stop's bay lies (m).
    pub bay: f32,
    /// Timetable departure (seconds of the day).
    pub depart: f64,
    /// The stop's map object (its `[busstop]` strings weigh who gets off there).
    pub id: i64,
    /// The side the platform lies on (see `tiles::stop_side`): 0 = right, 1 = the other,
    /// 2 = both. A bus whose doors are on both sides opens only these (it reads the value
    /// as `AI_Scheduled_AtStation_Side`).
    pub side: f32,
}

impl Stop {
    #[allow(clippy::type_complexity)]
    pub fn from_tuple(t: (usize, f32, f32, f64, i64, f32)) -> Stop {
        Stop { ri: t.0, s: t.1, bay: t.2, depart: t.3, id: t.4, side: t.5 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// On the way to the next stop (or with none left, to the end of the route).
    Running,
    /// At the stop with the doors open: people get off and on.
    Boarding,
    /// At the stop with the doors shut, waiting for the departure time (a layover at the
    /// first stop, or a bus that is early).
    Waiting,
    /// The doors close and the stop brake comes off, the blinker goes on: about to pull out.
    Closing,
    /// At the end of the trip: it stands until the timetable hands it the tour's next trip
    /// or lets it go.
    TripDone,
}

#[derive(Debug, Clone)]
pub struct BusService {
    pub stops: VecDeque<Stop>,
    pub phase: Phase,
    /// Seconds in the current phase.
    pub phase_t: f32,
    /// Seconds of boarding left (the people at the doors hold it open: `hold`).
    pub boarding: f32,
    /// When it may leave the stop (seconds of the day).
    pub leave_at: f64,
    /// When it came to the stop it stands at (seconds of the day).
    pub arrived_at: f64,
    /// The doors have been open at this stop already (a layover bus opens them only for
    /// the last minute before its departure).
    pub boarded: bool,
    /// Seconds behind (positive) or ahead of the timetable, as of the last stop.
    pub delay: f64,
    /// Put out before its departure: it waits for it at its first stop.
    pub layover: bool,
    /// The timetable still carries the route on as tiles bring their lanes: at the end of
    /// what it has, it waits for more.
    pub route_open: bool,
    /// The terminus of its trip, the name the waiting people read off it (Omsi.exe's bus
    /// +0x7bc) to see whether it goes their way.
    pub terminus: String,
    /// How far the front stop was last frame (m; infinite when not measured yet).
    near_d: f32,
    /// Whether it serves the front stop, once that is settled (`SKIP_DECIDE`).
    serve: Option<bool>,
    /// The trip's last station: always served.
    pub last_stop: Option<i64>,
    /// Stops the timetable has it serve in any case, and those it serves when it would be
    /// more than `EARLY_STOP_SHORT` early (`schedule::TripTimes::kinds`).
    pub always: Vec<i64>,
    pub serve_early: Vec<i64>,
}

/// The side the bus pulls out towards: left (1) from a bay on the right, else right (2).
fn out_signal(bay: f32) -> i32 {
    if bay < -0.1 {
        2
    } else {
        1
    }
}

/// `AI_Engine` at a stop (`BusService::engine_running`): off this long after arriving when
/// the departure is more than `ENGINE_OFF_WAIT` away, on again `ENGINE_ON_BEFORE` before it.
const ENGINE_OFF_AFTER: f64 = 6.0;
const ENGINE_OFF_WAIT: f64 = 60.0;
const ENGINE_ON_BEFORE: f64 = 20.0;

/// Seconds the doors stay open at a stop without anyone holding them.
fn boarding_time(id: u64) -> f32 {
    7.0 + (id % 5) as f32
}

/// An early bus waits at its stop until this long before its departure, a train at its
/// station until `EARLY_LEAVE_RAIL` before (Omsi.exe 0x7d9bdc: it stands while it is more
/// than 20 s, a train 120 s, early). Capped at 40 s, the buses no longer waited for their
/// times at the stops where a timetable holds them all for a connection (#1012).
const EARLY_LEAVE: f64 = 20.0;
const EARLY_LEAVE_RAIL: f64 = 120.0;
/// A layover waits for the departure however long (a tour's bus in on its previous trip).
const LAYOVER_WAIT: f64 = 1800.0;

/// How long a bus arriving at `now` stands at a stop it is to leave at `depart`.
fn early_wait(depart: f64, now: f64, layover: bool, rail: bool) -> f64 {
    let lead = if layover {
        0.0
    } else if rail {
        EARLY_LEAVE_RAIL
    } else {
        EARLY_LEAVE
    };
    (depart - lead - now).clamp(0.0, LAYOVER_WAIT)
}
/// On a layover, the doors open this long before the departure.
const LAYOVER_BOARDING: f64 = 45.0;
/// Pull into the bay over this distance before the stop: the stop's docking distance,
/// 30 m unless its object strings say otherwise (Omsi.exe 0x620058, string 4; the bus
/// moves over once it is that near, 0x7dac5e).
const BAY_REACH: f32 = 30.0;
/// Pulling out: at least this long after the doors were told to close (s), at most this
/// long waiting for the script to say they are shut.
const CLOSE_MIN: f32 = 1.5;
const CLOSE_MAX: f32 = 12.0;
/// Brake for the stop from this far.
const STOP_REACH: f32 = 80.0;
/// This near its stop a timetable bus settles whether it stops there at all (Omsi.exe
/// 0x7da5b5: 50 m): not when nobody aboard wants to get off and nobody waits there,
/// unless it is the trip's first or last stop or the bus is more than `EARLY_STOP` early.
const SKIP_DECIDE: f32 = 50.0;
const EARLY_STOP: f64 = 120.0;
const EARLY_STOP_SHORT: f64 = 20.0;

/// What the service needs of the world this frame.
pub struct Ctx<'a> {
    pub net: &'a Network,
    /// The car's way ahead: (lane, distance from its origin to the lane's start).
    pub way: &'a [(usize, f32)],
    pub day_time: f64,
    pub dt: f32,
    pub id: u64,
    /// Seconds it has stood still without a stop of its own.
    pub stopped: f32,
    /// Passing something (the passing manoeuvre owns the lateral position).
    pub passing: bool,
    /// Where it would swerve to round a car parked at the kerb.
    pub kerb_swerve: Option<f32>,
    /// Somebody aboard wants to get off at the front stop or somebody waits there (None:
    /// nobody knows - no passengers run - and every stop is served).
    pub wanted: Option<bool>,
    pub debug: bool,
}

impl BusService {
    /// Keep callbacks current before the next AI script frame. No trip-sized buffers
    /// are rebuilt here; tile streaming only changes the geometric stop queue.
    pub fn feed_timetable(&self, vehicle: &mut VehicleInstance, day_time: f64) {
        if vehicle.host.schedule_active < 0.5 {
            return;
        }
        if let Some(index) = timetable_stop_index(&vehicle.host, self.stops.front()) {
            vehicle.host.tt_busstop_index = index as i32;
        } else if self.stops.is_empty() && !self.route_open {
            vehicle.host.tt_busstop_index = vehicle.host.tt_stops.len().saturating_sub(1) as i32;
        }
        vehicle.host.tt_delay = if self.at_stop() {
            self.stops
                .front()
                .map(|s| day_time - s.depart)
                .unwrap_or(self.delay)
        } else {
            vehicle
                .host
                .tt_stops
                .get(vehicle.host.tt_busstop_index.max(0) as usize)
                .map(|s| self.delay.max(day_time - s.1 as f64))
                .unwrap_or(self.delay)
        } as f32;
        vehicle.set_var("schedule_active", vehicle.host.schedule_active);
    }

    pub fn new(stops: Vec<Stop>) -> BusService {
        BusService {
            stops: stops.into(),
            phase: Phase::Running,
            phase_t: 0.0,
            boarding: 0.0,
            leave_at: 0.0,
            arrived_at: 0.0,
            boarded: false,
            delay: 0.0,
            layover: false,
            route_open: false,
            terminus: String::new(),
            near_d: f32::INFINITY,
            serve: None,
            last_stop: None,
            always: Vec::new(),
            serve_early: Vec::new(),
        }
    }

    /// Doors open for people (`AI_Scheduled_AtStation`).
    pub fn at_station(&self) -> bool {
        self.phase == Phase::Boarding
    }

    /// The side's doors the bus opens at the stop it is at (`AI_Scheduled_AtStation_Side`):
    /// the front stop's while it boards, else 0 (nobody at a stop, nothing to open).
    pub fn at_station_side(&self) -> f32 {
        if self.phase == Phase::Boarding {
            self.stops.front().map(|s| s.side).unwrap_or(0.0)
        } else {
            0.0
        }
    }

    pub fn trip_done(&self) -> bool {
        self.phase == Phase::TripDone
    }

    /// Standing at a stop (boarding, waiting or pulling out).
    /// Whether its engine runs (`AI_Engine`): Omsi.exe (0x7d9128) switches a timetable
    /// bus's engine off six seconds after it came to a stop it is to leave more than a
    /// minute later, and on again twenty seconds before it leaves. The buses took their
    /// breaks at the termini with the engines running (#1404).
    pub fn engine_running(&self, day_time: f64) -> bool {
        let standing = matches!(self.phase, Phase::Boarding | Phase::Waiting);
        !(standing
            && self.leave_at - self.arrived_at > ENGINE_OFF_WAIT
            && day_time - self.arrived_at > ENGINE_OFF_AFTER
            && self.leave_at - day_time > ENGINE_ON_BEFORE)
    }

    pub fn at_stop(&self) -> bool {
        matches!(self.phase, Phase::Boarding | Phase::Waiting | Phase::Closing)
    }

    /// Seconds it expects to stand where it is yet (for the traffic behind: worth going
    /// round, or worth waiting for).
    pub fn standing_for(&self, day_time: f64) -> f32 {
        let wait = (self.leave_at - day_time).max(0.0) as f32;
        match self.phase {
            Phase::Running => 0.0,
            Phase::Boarding => self.boarding.max(0.0) + 2.0 + wait,
            Phase::Waiting => wait + 2.0,
            Phase::Closing => 1.0,
            Phase::TripDone => 600.0,
        }
    }

    /// Somebody is still at the doors: keep them open for `secs` more - for somebody
    /// coming from stop `stop` only while the bus serves that stop (Omsi.exe 0x7d9f1d: the
    /// person's stop is the bus's), not a stop it stands next to.
    pub fn hold(&mut self, stop: Option<i64>, secs: f32) {
        let here = stop.is_none_or(|s| self.stops.front().is_some_and(|f| f.id == s));
        if self.phase == Phase::Boarding && here {
            self.boarding = self.boarding.max(secs);
        }
    }

    /// A new trip (the tour's next, or the rest of a trip).
    pub fn restart(&mut self, stops: Vec<Stop>, layover: bool) {
        self.near_d = f32::INFINITY;
        self.serve = None;
        self.stops = stops.into();
        self.phase = Phase::Running;
        self.phase_t = 0.0;
        self.boarding = 0.0;
        self.boarded = false;
        self.layover = layover;
    }

    fn set_phase(&mut self, p: Phase) {
        self.phase = p;
        self.phase_t = 0.0;
    }

    /// A stop it serves whoever wants it or not: the trip's first (a layover) and last, the
    /// ones its timetable says it always serves, and any stop it would reach more than
    /// `EARLY_STOP` early (`EARLY_STOP_SHORT` at a stop marked for it).
    fn must_serve(&self, stop: &Stop, day_time: f64) -> bool {
        let last = (self.stops.len() == 1 && !self.route_open) || self.last_stop == Some(stop.id);
        let early = stop.depart - day_time;
        last || self.layover
            || early > EARLY_STOP
            || self.always.contains(&stop.id)
            || (early > EARLY_STOP_SHORT && self.serve_early.contains(&stop.id))
    }

    /// Arrived at the front stop: what now.
    fn arrive(&mut self, ctx: &Ctx, depart: f64, at: (usize, f32)) {
        if omsi_cfg::env::var_os("OMSI_DEBUG_STOPS").is_some() {
            log::info!("t={:.1}: timetable bus {} serves its stop {:?}", ctx.day_time, ctx.id, self.stops.front().map(|s| s.id));
        }
        let layover = std::mem::take(&mut self.layover);
        let rail = ctx.net.lanes.get(at.0).is_some_and(|l| l.kind == LaneKind::Rail);
        let wait = early_wait(depart, ctx.day_time, layover, rail);
        self.leave_at = ctx.day_time + wait;
        self.arrived_at = ctx.day_time;
        self.boarding = boarding_time(ctx.id);
        self.boarded = false;
        // a layover opens the doors for the last minute only; anywhere else people get off
        // straight away
        if layover && wait > LAYOVER_BOARDING + 10.0 {
            self.set_phase(Phase::Waiting);
        } else {
            self.set_phase(Phase::Boarding);
            self.boarded = true;
        }
        self.delay = (ctx.day_time + (self.boarding as f64).max(wait)) - depart;
        if ctx.debug {
            log::info!(
                "t={:.1}: timetable bus {} at its stop, {:.0} s to its departure ({:?}) at {:?}",
                ctx.day_time,
                ctx.id,
                depart - ctx.day_time,
                self.phase,
                ctx.net.lanes.get(at.0).map(|l| { let p = l.at(at.1).0; (p.x.round(), p.y.round()) })
            );
        }
    }

    /// Off from the stop: the next one is the front.
    fn depart(&mut self, st: &mut AiState, vehicle: &mut VehicleInstance, ctx: &Ctx) {
        let bay = self.stops.front().map(|s| s.bay).unwrap_or(0.0);
        if let Some(stop) = self.stops.front() {
            self.delay = ctx.day_time - stop.depart;
        }
        self.stops.pop_front();
        self.near_d = f32::INFINITY;
        self.serve = None;
        crate::traffic::ibis_to_next_stop(vehicle, self.stops.len());
        let air = ctx.net.lanes[st.lane].kind == LaneKind::Air;
        if self.stops.is_empty() && !self.route_open && !air {
            self.set_phase(Phase::TripDone);
            st.signal = 0;
            st.signal_time = 0.0;
            return;
        }
        self.set_phase(Phase::Running);
        // the blinker stays on while it pulls out
        st.signal = out_signal(bay);
        st.signal_time = 2.5;
        // out of the bay, and back in the lane before the next junction
        let lat = st.lateral;
        if lat.abs() > 0.05 && !ctx.passing {
            let junction = ctx
                .way
                .iter()
                .find(|&&(l, dl)| dl > 0.0 && !ctx.net.crossings[l].is_empty())
                .map(|w| (w.1 - st.front - 1.0).max(4.0));
            let len = (lat.abs() * 8.0).clamp(8.0, 30.0).min(junction.unwrap_or(f32::MAX));
            st.lateral_target = 0.0;
            st.lateral_ramp = (lat, 0.0, st.odometer, len);
        }
        if ctx.debug {
            log::info!(
                "t={:.1}: timetable bus {} pulls away, IBIS stop {:?}",
                ctx.day_time,
                ctx.id,
                vehicle.var("IBIS_busstop")
            );
        }
    }

    /// One frame of the service: where the bus has to stop (distance from its origin along
    /// its way), if anywhere; it also sets its blinker and its place across the lane.
    pub fn step(&mut self, st: &mut AiState, vehicle: &mut VehicleInstance, ctx: &Ctx) -> Option<f32> {
        self.phase_t += ctx.dt;
        let here = Some(st.front);
        match self.phase {
            Phase::TripDone => here,
            Phase::Waiting => {
                let board_at = self.leave_at - LAYOVER_BOARDING;
                if !self.boarded && ctx.day_time >= board_at {
                    self.boarded = true;
                    self.boarding = self.boarding.max((self.leave_at - ctx.day_time) as f32 - 3.0);
                    self.set_phase(Phase::Boarding);
                } else if self.boarded && ctx.day_time >= self.leave_at {
                    self.set_phase(Phase::Closing);
                }
                here
            }
            Phase::Boarding => {
                self.boarding -= ctx.dt;
                // an early bus waits for its departure with the doors open, as drivers do
                // (shut, it stood at the stop for half a minute for no reason anyone could
                // see); the doors close when it is time to go
                if self.boarding <= 0.0 && ctx.day_time >= self.leave_at {
                    self.set_phase(Phase::Closing);
                }
                here
            }
            Phase::Closing => {
                // blinker on towards the traffic while the doors close and the stop brake
                // comes off. The bus leaves when its script says the doors are shut (it
                // writes AI_Scheduled_AtStation back to 0), as OMSI's buses do; leaving
                // after a fixed 3.5 s drove buses off with their doors still closing, and
                // each type at its own moment. A script that never answers is not waited
                // for longer than CLOSE_MAX.
                let bay = self.stops.front().map(|s| s.bay).unwrap_or(0.0);
                st.signal = out_signal(bay);
                st.signal_time = st.signal_time.max(1.0);
                let answered = vehicle.station_released() && vehicle.var("AI_Scheduled_AtStation").map(|v| v.abs() < 0.5).unwrap_or(true);
                if (answered && self.phase_t >= CLOSE_MIN) || self.phase_t >= CLOSE_MAX {
                    self.depart(st, vehicle, ctx);
                    if self.phase == Phase::TripDone {
                        return here;
                    }
                    return None;
                }
                here
            }
            Phase::Running => self.approach(st, vehicle, ctx),
        }
    }

    /// Driving: brake for the next stop, pull into its bay.
    fn approach(&mut self, st: &mut AiState, vehicle: &mut VehicleInstance, ctx: &Ctx) -> Option<f32> {
        loop {
            let Some(stop) = self.stops.front().copied() else {
                if !ctx.passing {
                    st.lateral_target = ctx.kerb_swerve.unwrap_or(0.0);
                }
                return None;
            };
            let speed = st.speed;
            // A bus creeping the last metres to its stop point is at its stop, even when the
            // point lies past the end of the stop's lane and the lane ahead takes over first:
            // measured from the new lane the stop was suddenly 20-30 m behind, and the bus -
            // blinker on, pulled into the bay - drove on without opening its doors.
            let crept_past = self.near_d < 12.0 && speed < 4.0;
            if stop.ri < st.route_index {
                if crept_past {
                    self.near_d = f32::INFINITY;
                    self.arrive(ctx, stop.depart, (st.lane, st.s));
                    return Some(st.front);
                }
                // behind it already (the route was cut short)
                self.stops.pop_front();
                self.near_d = f32::INFINITY;
                self.serve = None;
                continue;
            }
            let d = st.route_distance(ctx.net, stop.ri, stop.s);
            // near enough to see whether anybody wants it (a train keeps to its stations)
            if self.serve.is_none() && d < SKIP_DECIDE {
                let rail = ctx.net.lanes.get(st.lane).is_some_and(|l| l.kind == LaneKind::Rail);
                self.serve = Some(rail || self.must_serve(&stop, ctx.day_time) || ctx.wanted.unwrap_or(true));
            }
            if self.serve == Some(false) {
                if ctx.debug || omsi_cfg::env::var_os("OMSI_DEBUG_STOPS").is_some() {
                    log::info!("t={:.1}: timetable bus {} passes its stop {}: nobody gets off or on", ctx.day_time, ctx.id, stop.id);
                }
                self.stops.pop_front();
                self.near_d = f32::INFINITY;
                self.serve = None;
                crate::traffic::ibis_to_next_stop(vehicle, self.stops.len());
                if !ctx.passing {
                    st.lateral_target = ctx.kerb_swerve.unwrap_or(0.0);
                }
                continue;
            }
            // into the bay over the last metres, but only once no junction lies between
            // the bus and its stop: the meeting places of a junction are laid out for
            // vehicles in the middle of their lane. Not where the stop lies too close
            // behind the junction for the S-curve into the bay (`AiState::lateral`):
            // the bus came to rest in the middle of its lane, too far from the pole for
            // anybody to get on, and stood there for good (DBC_Map, Grand and Peshtigo)
            if !ctx.passing {
                let junction_end = ctx
                    .way
                    .iter()
                    .filter(|&&(l, dl)| dl < d && !ctx.net.crossings[l].is_empty())
                    .map(|&(l, dl)| dl + ctx.net.lanes[l].length())
                    .reduce(f32::max);
                let ramp = ((stop.bay - st.lateral).abs() * 8.0).clamp(8.0, 30.0);
                let junction_first = junction_end.is_some_and(|e| d - e >= ramp);
                st.lateral_target = if d < BAY_REACH && d > -25.0 && !junction_first {
                    stop.bay
                } else {
                    ctx.kerb_swerve.unwrap_or(0.0)
                };
            }
            // queued behind something standing at its stop (another bus, the player's):
            // after a while it serves the stop where it stands, as drivers do
            let queued = speed < 0.3 && ctx.stopped > 6.0 && d < 45.0 && d >= 2.0;
            if (d < 2.0 && speed < 0.3) || queued || (d < -2.0 && crept_past) {
                self.near_d = f32::INFINITY;
                self.arrive(ctx, stop.depart, (st.lane, st.s));
                return Some(st.front);
            }
            self.near_d = d;
            if d < -2.0 {
                // missed it
                if ctx.debug || omsi_cfg::env::var_os("OMSI_DEBUG_STOPS").is_some() {
                    log::info!("t={:.1}: timetable bus {} missed its stop ({:.1} m past, {:.1} m/s, stood {:.1} s, passing {})", ctx.day_time, ctx.id, -d, speed, ctx.stopped, ctx.passing);
                }
                self.stops.pop_front();
                self.near_d = f32::INFINITY;
                self.serve = None;
                continue;
            }
            if d < STOP_REACH {
                // the stop point is where the front of the bus comes to rest
                return Some(d.max(0.0) + st.front - 0.3);
            }
            return None;
        }
    }
}

/// How far before a station's point a vehicle stops with its origin, as Omsi.exe measures
/// the way to the station (0x7da4e3): from the origin of the vehicle that leads, less half
/// its length for a train (its front comes to rest at the station, not its middle - the
/// S-Bahn stopped with half a car past the end of the platform), plus the holding point
/// offset of its `[ai_brakeperformance]` ("to correct unprecise braking").
pub fn stop_shift(ty: &omsi_sim::VehicleType, rail: bool) -> f32 {
    let hold = ty.def.ai_brake_performance.map(|b| b[4]).unwrap_or(0.0);
    let half = if rail { ty.half_length().unwrap_or(0.0) } else { 0.0 };
    half - hold
}

#[cfg(test)]
mod tests {
    /// A bus with a long wait at its stop switches its engine off after six seconds and on
    /// twenty seconds before it leaves; a short stop keeps it running (#1404).
    #[test]
    fn the_engine_rests_through_a_long_wait() {
        let mut b = super::BusService::new(Vec::new());
        b.phase = super::Phase::Waiting;
        b.arrived_at = 1000.0;
        b.leave_at = 1300.0;
        assert!(b.engine_running(1003.0), "just arrived");
        assert!(!b.engine_running(1010.0), "resting");
        assert!(b.engine_running(1285.0), "about to leave");
        b.leave_at = 1040.0;
        assert!(b.engine_running(1010.0), "a short stop");
        b.leave_at = 1300.0;
        b.phase = super::Phase::Running;
        assert!(b.engine_running(1010.0), "driving");
    }

    #[test]
    fn scheduled_ai_exposes_timetable_during_init_and_frame_ai() {
        let template = crate::schedule::tests::script_test_vehicle(
            "{init}\n(L.L.schedule_active) (S.L.init_active)\n(M.V.GetTTBusstopCount) (S.L.init_count)\n(M.V.GetTTLineString) (S.$.init_line)\n{end}\n{frame_ai}\n(L.L.schedule_active) (S.L.frame_active)\n(M.V.GetTTBusstopIndex) (S.L.frame_index)\n{end}\n",
            "schedule_active\ninit_active\ninit_count\nframe_active\nframe_index\n",
            "init_line\n",
        );
        let stop = Stop::from_tuple((0, 10.0, 0.0, 120.0, 7, 0.0));
        let timetable = AiTimetable {
            line: "042".into(),
            terminus: "terminus".into(),
            stops: vec![
                (7, "First".into(), 100.0, 120.0),
                (8, "Last".into(), 200.0, 220.0),
            ],
        };
        let mut host = omsi_sim::VehicleHost::new(Default::default());
        timetable.install(&mut host, Some(&stop));
        let mut vehicle = VehicleInstance::new(template.ty.clone(), host);
        assert_eq!(vehicle.var("init_active"), Some(1.0));
        assert_eq!(vehicle.var("init_count"), Some(2.0));
        assert_eq!(vehicle.str_var("init_line"), "042");
        vehicle.set_var("schedule_active", 0.0);
        vehicle.update_ai(0.01, &Default::default());
        assert_eq!(vehicle.var("frame_active"), Some(1.0));
        assert_eq!(vehicle.var("frame_index"), Some(0.0));
        // Ordinary traffic has no assigned timetable; it must remain inactive.
        let mut ordinary = VehicleInstance::new(
            template.ty.clone(),
            omsi_sim::VehicleHost::new(Default::default()),
        );
        ordinary.update_ai(0.01, &Default::default());
        assert_eq!(ordinary.var("init_active"), Some(0.0));
        assert_eq!(ordinary.var("frame_active"), Some(0.0));
    }

    #[test]
    fn timetable_tracks_repeated_stops_streaming_and_trip_changes() {
        let stop = |id, depart| Stop::from_tuple((0, 10.0, 0.0, depart, id, 0.0));
        let first = stop(7, 120.0);
        let second = stop(8, 220.0);
        let repeated = stop(7, 320.0);
        let timetable = AiTimetable {
            line: "42".into(),
            terminus: "terminal".into(),
            stops: vec![
                (7, "A".into(), 100.0, 120.0),
                (8, "B".into(), 200.0, 220.0),
                (7, "A".into(), 300.0, 320.0),
            ],
        };
        let mut v =
            crate::schedule::tests::script_test_vehicle("{frame_ai}\n{end}\n", "schedule_active\n", "");
        v.host.hof = Some(std::sync::Arc::new(omsi_vehicle::Hof {
            termini: vec![omsi_vehicle::hof::Terminus {
                texture_id: "terminal".into(),
                ..Default::default()
            }],
            ..Default::default()
        }));
        timetable.install(&mut v.host, Some(&first));
        assert_eq!(v.host.tt_terminus_index, 0);
        let mut service = BusService::new(vec![first, second, repeated]);
        service.phase = Phase::Boarding;
        service.feed_timetable(&mut v, 110.0);
        assert_eq!(v.host.tt_delay, -10.0);
        service.stops.pop_front();
        service.phase = Phase::Running;
        service.delay = 5.0;
        service.feed_timetable(&mut v, 210.0);
        assert_eq!(v.host.tt_busstop_index, 1);
        assert_eq!(v.host.tt_delay, 10.0);
        service.stops.pop_front();
        service.feed_timetable(&mut v, 310.0);
        assert_eq!(v.host.tt_busstop_index, 2);
        // Exhausting the loaded geometry does not exhaust the script's timetable.
        service.stops.clear();
        service.route_open = true;
        service.feed_timetable(&mut v, 315.0);
        assert_eq!(v.host.tt_stops.len(), 3);
        assert_eq!(v.host.tt_busstop_index, 2);
        assert_eq!(v.host.schedule_active, 1.0);
        let next = AiTimetable {
            line: "43".into(),
            terminus: "missing".into(),
            stops: vec![(9, "C".into(), 400.0, 420.0)],
        };
        next.install(&mut v.host, Some(&stop(9, 420.0)));
        assert_eq!(v.host.tt_line, "43");
        assert_eq!(v.host.tt_stops.len(), 1);
        assert_eq!(v.host.tt_busstop_index, 0);
        assert_eq!(v.host.tt_terminus_index, -1);
        assert_eq!(v.host.tt_delay, 0.0);
    }

    use super::*;

    /// Somebody coming from another stop than the one the bus serves does not keep it there
    /// (#767); somebody on the way out does, wherever.
    #[test]
    fn a_hold_counts_at_the_stop_served() {
        let stop = Stop { ri: 0, s: 0.0, bay: 0.0, depart: 0.0, id: 42, side: 0.0 };
        let mut s = BusService::new(vec![stop]);
        s.phase = Phase::Boarding;
        s.boarding = 0.0;
        s.hold(Some(41), 2.5);
        assert_eq!(s.boarding, 0.0);
        s.hold(Some(42), 2.5);
        assert_eq!(s.boarding, 2.5);
        s.boarding = 0.0;
        s.hold(None, 2.5);
        assert_eq!(s.boarding, 2.5);
    }

    #[test]
    fn an_early_bus_waits_for_its_time() {
        // five minutes early: until 20 s before its departure (a train: 2 min)
        assert_eq!(early_wait(400.0, 100.0, false, false), 280.0);
        assert_eq!(early_wait(400.0, 100.0, false, true), 180.0);
        // late, or nearly on time: off at once
        assert_eq!(early_wait(400.0, 390.0, false, false), 0.0);
        assert_eq!(early_wait(400.0, 500.0, false, false), 0.0);
        // a layover: to the departure itself
        assert_eq!(early_wait(400.0, 100.0, true, false), 300.0);
    }

    #[test]
    fn standing_time() {
        let mut s = BusService::new(vec![]);
        assert_eq!(s.standing_for(0.0), 0.0);
        s.phase = Phase::Waiting;
        s.leave_at = 100.0;
        assert!((s.standing_for(40.0) - 62.0).abs() < 1e-3);
        s.phase = Phase::TripDone;
        assert!(s.standing_for(0.0) > 100.0);
    }

    #[test]
    fn only_the_ends_of_the_trip_and_an_early_bus_stop_for_nobody() {
        let stop = |id: i64, depart: f64| Stop::from_tuple((0, 0.0, 0.0, depart, id, 0.0));
        let mut s = BusService::new(vec![stop(1, 100.0), stop(2, 200.0), stop(3, 300.0)]);
        s.last_stop = Some(3);
        // on time at a stop in the middle: only if somebody wants it
        assert!(!s.must_serve(&stop(2, 200.0), 150.0));
        // over two minutes early: it stops and waits
        assert!(s.must_serve(&stop(2, 200.0), 70.0));
        // the trip's terminus, and the last stop it knows of with the route complete
        assert!(s.must_serve(&stop(3, 300.0), 300.0));
        s.stops = vec![stop(2, 200.0)].into();
        assert!(s.must_serve(&stop(2, 200.0), 200.0));
        s.route_open = true;
        assert!(!s.must_serve(&stop(2, 200.0), 200.0));
        // its first stop, where it stands out its layover
        s.layover = true;
        assert!(s.must_serve(&stop(2, 200.0), 200.0));
        s.layover = false;
        // `[profile_otherstopping]` 1/4 (always) and 3 (when early)
        s.always = vec![2];
        assert!(s.must_serve(&stop(2, 200.0), 200.0));
        s.always.clear();
        s.serve_early = vec![2];
        assert!(!s.must_serve(&stop(2, 200.0), 190.0));
        assert!(s.must_serve(&stop(2, 200.0), 170.0));
    }

    #[test]
    fn station_side_comes_from_the_stop_it_boards_at() {
        let stop = |side: f32| Stop::from_tuple((0, 0.0, 0.0, 0.0, 1, side));
        let mut s = BusService::new(vec![stop(1.0)]);
        // off a stop: nothing to open
        s.phase = Phase::Running;
        assert_eq!(s.at_station_side(), 0.0);
        // boarding: the front stop's side
        s.phase = Phase::Boarding;
        assert_eq!(s.at_station_side(), 1.0);
        // waiting to pull out (doors shut): the side is not asked for any more
        s.phase = Phase::Waiting;
        assert_eq!(s.at_station_side(), 0.0);
        // an empty queue answers 0, not a panic
        s.phase = Phase::Boarding;
        s.stops.clear();
        assert_eq!(s.at_station_side(), 0.0);
    }
}
