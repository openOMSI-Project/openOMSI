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
}

impl Stop {
    pub fn from_tuple(t: (usize, f32, f32, f64, i64)) -> Stop {
        Stop { ri: t.0, s: t.1, bay: t.2, depart: t.3, id: t.4 }
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
    /// People aboard when it was put on the road (seated by the passengers' side when the
    /// bus first comes near).
    pub riders: u8,
    /// How far the front stop was last frame (m; infinite when not measured yet).
    near_d: f32,
}

/// The side the bus pulls out towards: left (1) from a bay on the right, else right (2).
fn out_signal(bay: f32) -> i32 {
    if bay < -0.1 {
        2
    } else {
        1
    }
}

/// Seconds the doors stay open at a stop without anyone holding them.
fn boarding_time(id: u64) -> f32 {
    7.0 + (id % 5) as f32
}

/// An early bus waits for its departure, but not for more than this (the stop times of
/// most trips are the running time shared out by distance: a bus that sat out minutes of
/// "lead" at every stop held up every bus behind it).
const EARLY_WAIT: f64 = 40.0;
/// A layover waits for the departure however long (a tour's bus in on its previous trip).
const LAYOVER_WAIT: f64 = 1800.0;
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
    pub debug: bool,
}

impl BusService {
    pub fn new(stops: Vec<Stop>, riders: u8) -> BusService {
        BusService {
            stops: stops.into(),
            phase: Phase::Running,
            phase_t: 0.0,
            boarding: 0.0,
            leave_at: 0.0,
            boarded: false,
            delay: 0.0,
            layover: false,
            route_open: false,
            terminus: String::new(),
            riders,
            near_d: f32::INFINITY,
        }
    }

    /// Doors open for people (`AI_Scheduled_AtStation`).
    pub fn at_station(&self) -> bool {
        self.phase == Phase::Boarding
    }

    pub fn trip_done(&self) -> bool {
        self.phase == Phase::TripDone
    }

    /// Standing at a stop (boarding, waiting or pulling out).
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

    /// Somebody is still at the doors: keep them open for `secs` more.
    pub fn hold(&mut self, secs: f32) {
        if self.phase == Phase::Boarding {
            self.boarding = self.boarding.max(secs);
        }
    }

    /// A new trip (the tour's next, or the rest of a trip).
    pub fn restart(&mut self, stops: Vec<Stop>, layover: bool) {
        self.near_d = f32::INFINITY;
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

    /// Arrived at the front stop: what now.
    fn arrive(&mut self, ctx: &Ctx, depart: f64, at: (usize, f32)) {
        if omsi_cfg::env::var_os("OMSI_DEBUG_STOPS").is_some() {
            log::info!("t={:.1}: timetable bus {} serves its stop", ctx.day_time, ctx.id);
        }
        let layover = std::mem::take(&mut self.layover);
        let limit = if layover { LAYOVER_WAIT } else { EARLY_WAIT };
        let wait = (depart - ctx.day_time).clamp(0.0, limit);
        self.leave_at = ctx.day_time + wait;
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
        self.stops.pop_front();
        self.near_d = f32::INFINITY;
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
            Phase::Running => self.approach(st, ctx),
        }
    }

    /// Driving: brake for the next stop, pull into its bay.
    fn approach(&mut self, st: &mut AiState, ctx: &Ctx) -> Option<f32> {
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
                continue;
            }
            let d = st.route_distance(ctx.net, stop.ri, stop.s);
            // into the bay over the last metres, but only once no junction lies between
            // the bus and its stop: the meeting places of a junction are laid out for
            // vehicles in the middle of their lane
            if !ctx.passing {
                let junction_first = ctx
                    .way
                    .iter()
                    .any(|&(l, dl)| dl < d && !ctx.net.crossings[l].is_empty());
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

/// How many people ride a timetable bus put on the road at `day_time` (seconds of the
/// day): the rush hours full, the night nearly empty; `seed` spreads it between buses.
pub fn riders_at(day_time: f64, seed: u64) -> u8 {
    // (OMSI_AI_RIDERS=0 leaves them empty, =n caps them)
    let cap = omsi_cfg::env::var("OMSI_AI_RIDERS").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(40.0);
    let h = (day_time / 3600.0).rem_euclid(24.0);
    let rush = |c: f64, w: f64| (-((h - c) / w).powi(2)).exp();
    let base = if (5.0..23.5).contains(&h) { 5.0 } else { 1.0 };
    let peak = 16.0 * rush(7.8, 1.3) + 13.0 * rush(16.8, 1.8) + 6.0 * rush(12.5, 2.0);
    let spread = 0.6 + ((seed >> 17) % 100) as f64 / 100.0 * 0.8;
    ((base + peak) * spread).round().clamp(0.0, cap) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn riders_follow_the_rush_hours() {
        let at = |h: f64| (0..20).map(|s| riders_at(h * 3600.0, s * 7919 << 17) as u32).sum::<u32>();
        assert!(at(8.0) > at(11.0));
        assert!(at(17.0) > at(20.0));
        assert!(at(20.0) > at(3.0));
    }

    #[test]
    fn standing_time() {
        let mut s = BusService::new(vec![], 0);
        assert_eq!(s.standing_for(0.0), 0.0);
        s.phase = Phase::Waiting;
        s.leave_at = 100.0;
        assert!((s.standing_for(40.0) - 62.0).abs() < 1e-3);
        s.phase = Phase::TripDone;
        assert!(s.standing_for(0.0) > 100.0);
    }
}
