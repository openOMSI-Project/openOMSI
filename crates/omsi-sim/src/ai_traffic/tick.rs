//! One step of the traffic simulation: who is where, the light requests, every car's
//! plan and its drive, then the bodies and scripts on the workers.

use super::*;

pub const AI_JOB_SECS: f32 = 50e-6;

/// Seconds a timetable bus waits where the route it has ends for the rest of its route
/// (tiles bring it as they load) before it gives the trip up (see `Traffic::tick`).
pub const ROUTE_WAIT_MAX: f32 = 40.0;

/// A car edging out round something standing keeps to `PULL_OUT_ACCEL` until its front is
/// this far past the obstacle's rear (m).
pub const CREEP_PAST: f32 = 2.0;
/// How far ahead an emergency vehicle warns what holds it up (`TrafficPriorityWarningNeeded`,
/// m).
pub const PRIORITY_WARN_GAP: f32 = 60.0;

/// Where every car is (see `Traffic::occupancy`): per lane, (car, distance along the lane,
/// lateral place, out on that lane passing something).
pub type ByLane = HashMap<usize, Vec<(usize, f32, f32, bool)>>;

/// What a car follows: the gap to it and who it is (a car's index, `usize::MAX` the player's bus or a
/// LAN player's, None a parked car).
pub type LeadOf = Option<(Lead, Option<usize>)>;
/// The cars coming to each junction lane: (car, distance from its origin to the lane start).
pub type Coming = HashMap<usize, Vec<(usize, f32)>>;
/// The cars that have claimed each junction lane.
pub type Claims = HashMap<usize, Vec<usize>>;
/// `Traffic::right_of_way`: where the merge holds a car, its way ahead, where a light holds it,
/// where it gives way.
type RightOfWay = (Option<f32>, Vec<(usize, f32)>, Option<f32>, Option<f32>);

/// What every car's plan in a tick reads of the tick's start: who is where, the player's
/// vehicle and the LAN players'.
pub struct TickScene {
    pub dt: f32,
    pub debug: bool,
    pub player: Option<PlayerBox>,
    /// Seconds the player's vehicle has stood.
    pub player_standing: f32,
    pub others: Vec<(u32, PlayerBox)>,
    pub feet: Vec<Footprint>,
    pub by_lane: ByLane,
    /// Cars coming to a junction lane: (car, distance from its origin to the lane start).
    pub coming: Coming,
    /// Pedestrians on the footpaths by lane (distance along it).
    pub walkers: HashMap<usize, Vec<f32>>,
}

/// A bus's indicator towards the traffic over one tick: (seconds since it last showed,
/// seconds it has been indicating) after `dt` with the indicator `on` or not. One second of
/// memory bridges the dark half of the lamps' cycle.
fn signal_step((age, signalling): (f32, f32), on: bool, dt: f32) -> (f32, f32) {
    let age = if on { 0.0 } else { age + dt };
    (age, if age < 1.0 { signalling + dt } else { 0.0 })
}

impl TrafficSim {
    /// Advance all cars.
    /// `player`: (centre, heading in degrees, half length, half width, speed) of the
    /// player's vehicle.
    pub fn tick(&mut self, dt: f32, player: Option<PlayerBox>) {
        self.lamp_dt += dt;
        if self.mirror {
            self.mirror_tick(dt);
            return;
        }
        let t_start = std::time::Instant::now();
        self.time += dt;
        self.day_time += dt as f64 * self.time_scale;
        self.last_dt = dt;
        self.held_at_red = 0;
        self.player = player;
        self.geo_prev = self.cars.iter_mut().map(|c| c.geo_block.take()).collect();
        self.index_of = self
            .cars
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id, i))
            .collect();
        let debug = omsi_cfg::flags::OMSI_DEBUG_TRAFFIC.is_set();
        self.stats_before();
        let (by_lane, coming, mut reservations) = self.occupancy();
        self.request_lights(player);
        let walkers = self.walker_requests();
        self.run_light_programs(dt);
        let (others, player_standing) = self.track_players(dt, player);
        self.others_now = others.iter().map(|o| o.1).collect();
        let t_plan = std::time::Instant::now();
        let mut remove = Vec::new();
        let mut frames: Vec<Option<AiFrame>> = vec![None; self.cars.len()];
        let feet = self.footprints();
        self.break_lead_pairs();
        self.break_rings();
        let ts = TickScene { dt, debug, player, player_standing, others, feet, by_lane, coming, walkers };
        for (i, frame) in frames.iter_mut().enumerate() {
            match self.plan_car(i, &ts, &mut reservations) {
                Some(f) => *frame = Some(f),
                None => remove.push(i),
            }
        }
        let others = ts.others;
        self.set_visibility();
        let t_par = std::time::Instant::now();
        self.step_bodies(dt, &mut frames);
        self.tick_split = [
            (t_plan - t_start).as_secs_f64(),
            (t_par - t_plan).as_secs_f64(),
            t_par.elapsed().as_secs_f64(),
        ];
        self.debug_tick(debug, player, &others, &frames);
        self.stats_after(dt, &remove);
        for i in remove.into_iter().rev() {
            let c = self.cars.swap_remove(i);
            // its sounds stop and the renders go back to the world at the next sync
            self.retired.push(c.id);
        }
    }

    /// Where every car is: its lane with its lateral place (and the lane a passing car is
    /// over on, where it counts for the oncoming traffic), the cars coming to each junction
    /// lane and the junction lanes each has claimed.
    fn occupancy(&self) -> (ByLane, Coming, Claims) {
        let mut by_lane: HashMap<usize, Vec<(usize, f32, f32, bool)>> = HashMap::new();
        // cars coming to a junction lane: (car, distance from its origin to the lane start)
        let mut coming: HashMap<usize, Vec<(usize, f32)>> = HashMap::new();
        let mut reservations: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, c) in self.cars.iter().enumerate() {
            by_lane
                .entry(c.state.lane)
                .or_default()
                .push((i, c.state.s, c.state.lateral, false));
            // the rear of the vehicle still stands in the lane it came from: whoever crosses
            // or follows that lane waits until it is out (a bus that had turned into the next
            // lane with its front used to count as gone from the junction it still filled)
            if let Some(p) = c.state.prev_lane {
                if c.state.s < c.state.rear + 1.0 && c.state.change.is_none() {
                    by_lane.entry(p).or_default().push((
                        i,
                        self.net.lanes[p].length() + c.state.s,
                        c.state.lateral,
                        false,
                    ));
                }
            }
            if let Some(ch) = c.state.change {
                by_lane
                    .entry(ch.to)
                    .or_default()
                    .push((i, ch.s_to, 0.0, false));
            }
            if let Some(p) = c.passing {
                // Out on the oncoming lane: the traffic there stops short of the place where
                // the car will be back out of their way (it is headed there, and that place
                // does not move). One that gave up counts only while it still stands too far
                // out for them to get by.
                let deep = c.state.lateral * self.net.oncoming_sign();
                let out = if p.aborted {
                    deep > p.side - c.half_width - 1.45
                } else {
                    deep > 1.2
                };
                if out {
                    if let Some((l, s, _)) = self
                        .net
                        .opposite(c.state.lane, c.state.s)
                        .filter(|o| o.0 == p.lane || (o.2 - p.side).abs() < 1.5)
                    {
                        let r = if p.aborted {
                            0.0
                        } else {
                            (p.clear_at(c.half_width) - c.state.odometer).max(0.0)
                        };
                        let at = s - r;
                        if at >= 0.0 {
                            by_lane.entry(l).or_default().push((i, at, 0.0, true));
                        } else {
                            // (that place lies on the lanes before it)
                            for (ul, off, _) in
                                self.net.upstream(l, at, 1.0, 12).into_iter().skip(1)
                            {
                                let x = at - off;
                                if x >= 0.0 && x <= self.net.lanes[ul].length() {
                                    by_lane.entry(ul).or_default().push((i, x, 0.0, true));
                                }
                            }
                        }
                    }
                }
            }
            for (l, d) in self
                .way_lanes(&c.state, LOOK_AHEAD + 30.0)
                .into_iter()
                .skip(1)
            {
                if !self.net.crossings[l].is_empty() {
                    coming.entry(l).or_default().push((i, d));
                }
            }
            for &l in &c.reserved {
                reservations.entry(l).or_default().push(i);
            }
        }
        (by_lane, coming, reservations)
    }

    /// The light programs' requests: of every car, of the player's bus and the other
    /// players' vehicles.
    fn request_lights(&mut self, player: Option<PlayerBox>) {
        // the light programs: requests of whoever is coming, then the cycle clocks
        for c in self.lights.iter_mut() {
            c.request.iter_mut().for_each(|r| *r = false);
        }
        // How far ahead a vehicle asks for a light or a crossing: as far as the farthest
        // `[approachdist]` of the map's lights says (OMSI takes them up to 1000 m), at least
        // the 160 m it always was. Held to 160, a railway crossing set to ring 300 m before
        // the train never rang (#1421).
        let reach = self
            .lights
            .iter()
            .flat_map(|c| c.approach.iter().flatten().copied())
            .fold(160.0f32, f32::max)
            .min(1200.0);
        let mut indicating: Vec<(PlayerBox, u8)> = Vec::new();
        for c in &self.cars {
            // (a car indicating stands on the paths marked with its turn: the rear sections
            // of an articulated bus too)
            if matches!(c.state.blinker, 1 | 2) {
                let v = &c.vehicle;
                let bb = v.ty.def.bounding_box.unwrap_or(model::DEFAULT_BOX);
                indicating.push((light_paths::box_outline(v.position, v.heading, bb, c.state.speed), c.state.blinker as u8));
                for t in &v.trailers {
                    if let Some(bb) = t.ty.def.bounding_box {
                        indicating.push((light_paths::box_outline(t.position, t.heading, bb, c.state.speed), c.state.blinker as u8));
                    }
                }
            }
            for (l, d) in self.way_lanes(&c.state, reach) {
                if let Some((ci, li)) = self.net.lanes[l].traffic_light {
                    if let Some(ctl) = self.lights.get_mut(ci) {
                        let gap = d - c.state.front;
                        // (asked until its rear has left the lane, not its middle)
                        if gap <= ctl.approach_dist(li) && d + self.net.lanes[l].length() + c.state.rear >= 0.0 {
                            if let Some(r) = ctl.request.get_mut(li) {
                                *r = true;
                            }
                        }
                    }
                }
            }
        }
        // the player's bus and the other players' vehicles ask too: a depot gate (Spandau's
        // `Omnibushof_S_1`, the exit arm on light 1) opens only for whoever asks, and the
        // player driving out of the depot at the start of a duty found it shut
        let askers: Vec<(DVec3, f64)> = player
            .iter()
            .map(|p| (p.0, p.1))
            .chain(self.others.iter().map(|(_, b)| (b.0, b.1)))
            .collect();
        indicating.extend(player.iter().map(|p| (*p, self.player_blinker)));
        indicating.extend(self.others.iter().map(|(id, b)| (*b, self.other_blinkers.get(id).copied().unwrap_or(0))));
        for (b, blinker) in &indicating {
            self.request_indicated(b, *blinker);
        }
        for &(pos, heading) in &askers {
            // (off the lanes - a depot yard, a car park - a gate's lane that starts just
            // ahead, the way the bus is facing, is asked all the same: standing a few metres
            // beside every lane there, the bus never opened the barrier in front of it.
            // Only off the lanes: on the road it asked the lights of every lane up to 6 m
            // beside it - Winsenburg's bus light jumped for a bus driving past on the road
            // next to the bus bays, #1790; on a lane, the lanes ahead below ask)
            if self.net.lane_along(pos, heading, LaneKind::Street, 2.5, 45.0).is_some() {
                continue;
            }
            let h = heading.to_radians();
            let fwd = glam::DVec2::new(h.sin(), h.cos());
            for l in 0..self.net.lanes.len() {
                let lane = &self.net.lanes[l];
                let Some((ci, li)) = lane.traffic_light else { continue };
                let (p0, h0) = lane.at(0.0);
                let d = (p0 - pos).truncate();
                let (along, across) = (d.dot(fwd), d.perp_dot(fwd).abs());
                let turn = ((h0 as f64 - heading + 540.0).rem_euclid(360.0) - 180.0).abs();
                if (-2.0..25.0).contains(&along) && across < 6.0 && turn < 60.0 && (p0.z - pos.z).abs() < 4.0 {
                    if let Some(r) = self.lights.get_mut(ci).and_then(|c| c.request.get_mut(li)) {
                        *r = true;
                    }
                }
            }
        }
        for (pos, heading) in askers {
            for (l, d) in self.lanes_ahead_of(pos, heading, reach) {
                if let Some((ci, li)) = self.net.lanes[l].traffic_light {
                    if let Some(ctl) = self.lights.get_mut(ci) {
                        if d <= ctl.approach_dist(li) {
                            if let Some(r) = ctl.request.get_mut(li) {
                                *r = true;
                            }
                        }
                    }
                }
            }
        }
    }

    /// The pedestrians by lane, and the push buttons of the pedestrian lights they come to.
    fn walker_requests(&mut self) -> HashMap<usize, Vec<f32>> {
        let mut walkers: HashMap<usize, Vec<f32>> = HashMap::new();
        for &(l, s) in &self.walkers {
            walkers.entry(l).or_default().push(s);
            let Some(lane) = self.net.lanes.get(l) else {
                continue;
            };
            // the push button of a pedestrian light on the way
            for (ahead, dist) in lane
                .next
                .iter()
                .map(|&n| (n, lane.length() - s))
                .chain(self.net.prev.get(l).into_iter().flatten().map(|&p| (p, s)))
                .chain(std::iter::once((l, 0.0)))
            {
                if let Some((ci, li)) = self.net.lanes.get(ahead).and_then(|x| x.traffic_light) {
                    if let Some(ctl) = self.lights.get_mut(ci) {
                        if dist <= ctl.approach_dist(li).min(10.0) {
                            if let Some(r) = ctl.request.get_mut(li) {
                                *r = true;
                            }
                        }
                    }
                }
            }
        }
        walkers
    }

    /// The light programs' clocks move on.
    fn run_light_programs(&mut self, dt: f32) {
        let day_time = self.day_time;
        for c in self.lights.iter_mut() {
            c.start(day_time);
            c.advance(dt);
        }
        self.log_lights();
    }

    /// How long the player's vehicle and the LAN players' have stood, the player's
    /// indicator, and all of them on the lanes for the right of way. Returns the LAN
    /// players' vehicles (taken from `others`) and the seconds the player's has stood.
    fn track_players(&mut self, dt: f32, player: Option<PlayerBox>) -> (Vec<(u32, PlayerBox)>, f32) {
        self.player_still = match player {
            Some(p) if p.4.abs() < 0.3 => self.player_still + dt,
            _ => 0.0,
        };
        let player_standing = self.player_still;
        let others = std::mem::take(&mut self.others);
        let mut others_still: HashMap<u32, f32> = HashMap::new();
        for (id, b) in &others {
            let before = self.others_still.get(id).copied().unwrap_or(0.0);
            others_still.insert(*id, if b.4.abs() < 0.3 { before + dt } else { 0.0 });
        }
        self.others_still = others_still;
        // the indicator towards the traffic (left, or right on a left-hand-traffic map),
        // remembered across the dark half of the lamps' cycle
        let out_side = if self.net.left_hand { 2 } else { 1 };
        (self.player_signal_age, self.player_signalling) = signal_step(
            (self.player_signal_age, self.player_signalling),
            self.player_blinker == out_side,
            dt,
        );
        // ... and the LAN players' (their buses' front sections), for letting them out of
        // their stops as the player's: on a dedicated server every bus is a LAN player's,
        // and with the player's indicator alone the cars let none of them out
        let mut others_signal: HashMap<u32, (f32, f32)> = HashMap::new();
        for (id, _) in others.iter().filter(|(id, _)| *id < 0xFFF0_0000) {
            let before = self
                .others_signal
                .get(id)
                .copied()
                .unwrap_or((f32::MAX, 0.0));
            let on = self.other_blinkers.get(id).copied() == Some(out_side);
            others_signal.insert(*id, signal_step(before, on, dt));
        }
        self.others_signal = others_signal;
        // the player's vehicle and the LAN players' on the lanes, for the right of way (the
        // rear sections and the vehicles placed by hand stand, they do not come)
        let mut users: Vec<WayUser> = Vec::new();
        if let Some(p) = player.as_ref() {
            users.extend(way_user_on(&self.net, p, self.player_blinker, player_standing, self.player_priority));
        }
        for (id, b) in others.iter().filter(|(id, _)| *id < 0xFFF0_0000) {
            users.extend(way_user_on(&self.net, b, 0, self.others_still.get(id).copied().unwrap_or(0.0), false));
        }
        self.way_users = users;
        (others, player_standing)
    }

    /// Car `i`'s plan for this tick and its drive: its frame for the body and the scripts,
    /// None when it ran out of road (it is taken off).
    fn plan_car(&mut self, i: usize, ts: &TickScene, reservations: &mut Claims) -> Option<AiFrame> {
        let (lead, player, player_standing) = self.car_lead(i, ts);
        let (lead, parked_ahead, parked_box, kerb_swerve) = self.kerb_and_squeeze(i, lead, &ts.by_lane);
        let (standing, obstacle_len, at_stop, keep_back) = self.keep_back(i, lead, parked_ahead, player, player_standing);
        self.pass_step(i, lead, obstacle_len, standing, parked_ahead, at_stop, &ts.by_lane, player, parked_box, &ts.feet);
        let (merge_wait, way, light, yield_at) = self.right_of_way(i, ts, lead, reservations);
        let (stop_at, why) = self.stop_points(i, ts, &way, lead, light, yield_at, merge_wait, keep_back);
        let (stop_at, why) = self.service_and_park(i, ts, &way, kerb_swerve, stop_at, why);
        let (stop_at, why) = self.end_of_way(i, ts, &way, stop_at, why);
        self.drive_car(i, ts, lead, stop_at, why, light, yield_at, merge_wait)
    }

    /// What car `i` follows: the car ahead (it may change lanes first), the player's bus or
    /// a LAN player's where it is in the way, another body off the lanes. Also the player's
    /// vehicle that stands for "the player's bus" from here on, and how long it has stood.
    fn car_lead(&mut self, i: usize, ts: &TickScene) -> (LeadOf, Option<PlayerBox>, f32) {
        let (by_lane, feet, others, debug) = (&ts.by_lane, &ts.feet, &ts.others, ts.debug);
        let (player, player_standing) = (ts.player, ts.player_standing);
        self.plan_lane_change(i, by_lane);
        let ahead = self.obstacle_ahead(i, look_ahead(self.cars[i].state.speed), by_lane);
        // remember whom it lets in at a merge (a car on another lane)
        let merging = ahead
            .filter(|(_, j)| {
                self.cars[*j].state.lane != self.cars[i].state.lane
                    && !self.cars[i]
                        .state
                        .upcoming()
                        .any(|u| u == self.cars[*j].state.lane)
            })
            .map(|(_, j)| self.cars[j].id);
        self.cars[i].merge_after = merging;
        let mut lead = ahead.map(|(l, j)| (l, Some(j)));
        // the player's bus, wherever it overlaps this car's way, or a LAN player's (the
        // nearest in the way stands for "the player's bus" in what follows)
        let (mut player, mut player_standing) = (player, player_standing);
        if let Some(p) = player.as_ref() {
            if let Some(l) = self.player_in_way(i, p) {
                if lead.map(|x| l.gap < x.0.gap).unwrap_or(true) {
                    lead = Some((l, Some(usize::MAX)));
                }
            }
        }
        // (or coming to the joint where this car's lane runs into its own)
        if let Some(l) = merging_lead(&self.net, &self.cars[i].state, &self.way_users) {
            if lead.map(|x| l.gap < x.0.gap).unwrap_or(true) {
                lead = Some((l, Some(usize::MAX)));
            }
        }
        for (id, o) in others.iter() {
            if let Some(l) = self.player_in_way(i, o) {
                if lead.map(|x| l.gap < x.0.gap).unwrap_or(true) {
                    lead = Some((l, Some(usize::MAX)));
                    player = Some(*o);
                    player_standing = self.others_still.get(id).copied().unwrap_or(0.0);
                }
            }
        }
        // other vehicles' bodies in the way off the lanes
        if let Some((l, j)) = self.body_in_way(i, feet, by_lane) {
            self.cars[i].geo_block = Some(self.cars[j].id);
            if lead.map(|x| l.gap < x.0.gap - 0.5).unwrap_or(true) {
                if debug
                    && l.gap < 3.0
                    && l.speed < 0.5
                    && self.cars[i].stopped == 0.0
                    && self.cars[i].state.speed > 0.5
                {
                    log::info!("t={:.1}: car {} stops for the body of car {} in its way ({:.1} m) off the lanes", self.time, self.cars[i].id, self.cars[j].id, l.gap);
                }
                lead = Some((l, Some(j)));
            }
        }
        if let Some((_, Some(j))) = lead {
            if j < self.cars.len() && self.cars[i].ignore_lead.is_some_and(|(id, until)| id == self.cars[j].id && (self.time as f64) < until) {
                lead = None;
            }
        }
        (lead, player, player_standing)
    }

    /// Parked cars: stop behind one in the lane, swerve round one at the kerb; squeeze past
    /// a bus standing half in its bay.
    fn kerb_and_squeeze(&mut self, i: usize, mut lead: LeadOf, by_lane: &ByLane) -> (LeadOf, bool, Option<Obb>, Option<f32>) {
        // parked cars: stop behind one in the middle of the lane, swerve round one at
        // the kerb (a parked car eats the right half of the lane; the passing car
        // moves left by what is missing, and back once it is past)
        let mut parked_ahead = false;
        let mut parked_box: Option<Obb> = None;
        let kerb_swerve: Option<f32>;
        let mut squeeze: Option<u64> = None;
        {
            let car = &self.cars[i];
            let st = &car.state;
            let near_way = self.way_lanes(st, 100.0);
            let passing = car.passing.map(|p| !p.aborted).unwrap_or(false);
            let mut swerve: Option<f32> = None;
            let mut stand: Option<(f32, usize, f32, f32)> = None;
            let mut check = |along: f32, lat: f32, lane: usize, at: f32| {
                if !(-6.0..=100.0).contains(&along) {
                    return;
                }
                let a = lat.abs();
                // in the way at the side the car is on now (a car pulled out onto the
                // other half passes it)
                let blocks = if passing {
                    (lat - st.lateral_ahead(along)).abs() < car.half_width + 0.9 + 0.2
                } else {
                    a < 0.9
                };
                if blocks {
                    if along > 0.0 {
                        let gap = along - 2.3 - st.front;
                        if stand.map(|o| gap < o.0).unwrap_or(true) {
                            stand = Some((gap, lane, at, lat));
                        }
                    }
                } else if !passing && a < car.half_width + 0.9 + 0.15 && along < 30.0 {
                    // (only as far as the two bodies would touch: OMSI's cars keep to
                    // their paths, and moved out by a margin of our own round every car
                    // at the kerb - 2.7 m from the lane's middle - the traffic of a
                    // narrow British street lined with parked cars wove to and fro
                    // across the road instead of keeping to its lane)
                    let need = (car.half_width + 0.9 + 0.15 - a) * -lat.signum();
                    swerve = Some(
                        swerve
                            .map(|w| if w.abs() > need.abs() { w } else { need })
                            .unwrap_or(need),
                    );
                }
            };
            // (once committed to a lane change - well over, or pulling out round what
            // stands in the way - the parked cars of the lane it leaves hold it no more,
            // as the cars standing there do not, `obstacle_ahead`: counted still, the car
            // that had begun to pull out round a row of them stopped with its nose on the
            // first, and a lane change that moves on with the car never got anywhere -
            // six cars queued for good behind the parked row on the Heerstraße)
            let leaving = st
                .change
                .filter(|c| c.t > 0.4 || (c.bypass && c.wait <= 0.0))
                .map(|_| st.lane);
            for &(l, d) in &near_way {
                if Some(l) == leaving {
                    continue;
                }
                for &(s, lat) in self.parked.get(&l).map(|v| v.as_slice()).unwrap_or(&[]) {
                    check(d + s, lat, l, s);
                }
            }
            // a bus standing half in its bay: squeeze past on
            // the other side when a metre is enough, instead of queueing behind it -
            // and stay out until past its front (moving back in while still beside it
            // steered the car into the bus)
            if !passing {
                let swerving = st.lateral_target.abs() > 0.1;
                for &(l, d) in &near_way {
                    for &(j, os, lat, foreign) in
                        by_lane.get(&l).map(|v| v.as_slice()).unwrap_or(&[])
                    {
                        let o = &self.cars[j];
                        let along = d + os;
                        if foreign
                            || j == i
                            || lat.abs() < 0.5
                            || !(-(o.state.front + st.rear + 1.0)..=40.0).contains(&along)
                        {
                            continue;
                        }
                        // a bus that is about to pull away (its last seconds at the stop, the
                        // indicator on) is not started round; one the car is already going
                        // round is passed, unless the car can still stop behind it gently
                        // (only a bus at its stop stands out of the lane on purpose: a car
                        // off the middle is squeezing past something itself)
                        let standing = o.state.speed < 0.3 && o.standing_for(self.day_time) > 3.0;
                        let keep = swerving
                            && car.squeeze == Some(o.id)
                            && (o.state.speed < 2.0
                                || along
                                    < o.state.front
                                        + st.front
                                        + st.speed * st.speed / (2.0 * st.decel.max(1.0)));
                        if !standing && !keep {
                            continue;
                        }
                        let need = car.half_width + o.half_width + 0.35 - lat.abs();
                        if need > 0.0 && need <= 1.1 {
                            let w = need * -lat.signum();
                            if swerve.map(|v: f32| w.abs() > v.abs()).unwrap_or(true) {
                                swerve = Some(w);
                                squeeze = Some(o.id);
                            }
                        }
                    }
                }
            }
            if let Some((gap, pl, ps, lat)) = stand {
                // stop a little further back than behind a car that will move on
                let l = Lead {
                    gap: (gap - 2.0).max(0.0),
                    speed: 0.0,
                    acc: 0.0,
                };
                if lead.map(|x| l.gap < x.0.gap).unwrap_or(true) {
                    lead = Some((l, None));
                    parked_ahead = true;
                    // (a parked car's box, for pulling out round it)
                    let (q, h) = self.net.lanes[pl].at(ps);
                    let hr = (h as f64).to_radians();
                    let centre = q.truncate() + DVec2::new(hr.cos(), -hr.sin()) * lat as f64;
                    parked_box = Some(Obb::vehicle(centre, h as f64, 2.3, 2.3, 0.9));
                }
            }
            kerb_swerve = swerve;
        }
        if squeeze.is_some()
            && self.cars[i].squeeze.is_none()
            && self.first_passer.is_none()
            && !self.cars[i].is_bus()
        {
            self.first_passer = Some((self.cars[i].id, self.time));
        }
        self.cars[i].squeeze = squeeze;
        (lead, parked_ahead, parked_box, kerb_swerve)
    }

    /// Something standing ahead: how long it is, whether it is a queue at a stop, and how
    /// far behind it car `i` stops to be able to pull out round it later.
    fn keep_back(&mut self, i: usize, lead: LeadOf, parked_ahead: bool, player: Option<PlayerBox>, player_standing: f32) -> (bool, f32, bool, Option<f32>) {
        let standing = self.standing_obstacle(i, lead, parked_ahead, player_standing);
        // (a queue at a stop is passed as a whole)
        let (obstacle_len, at_stop) = match lead.and_then(|l| l.1) {
            Some(usize::MAX) => (
                player.map(|p| p.2 * 2.0).unwrap_or(12.0),
                player_standing > 10.0,
            ),
            Some(j) if j < self.cars.len() => self.standing_queue(j),
            _ => (4.8, false),
        };
        self.cars[i].lead_info = lead.and_then(|(l, who)| {
            who.filter(|&j| j < self.cars.len())
                .map(|j| (self.cars[j].id, l.gap))
        });
        // Something that may stand for a while (a bus at its stop, the player's bus that
        // has stopped) is waited behind with room to pull out round it later: a car that
        // had stopped a metre behind the player's bus scraped its corner when it went
        // round, and no car can steer out of that.
        let may_stand = lead
            .map(|(l, who)| {
                l.speed.abs() < 0.3
                    && match who {
                        Some(usize::MAX) => player.map(|p| p.4.abs() < 0.3).unwrap_or(false),
                        Some(j) if j < self.cars.len() => {
                            self.cars[j].at_stop()
                        }
                        _ => false,
                    }
            })
            .unwrap_or(false);
        // It stops `pass_room` short of it (the room its own steering needs to get out
        // round it), or as far back as it can without braking hard. The two metres taken
        // off the gap it keeps to such a thing were not enough: the car still crept up to
        // under three metres behind the player's bus and never got round it.
        let mut keep_back: Option<f32> = None;
        if standing || may_stand {
            if let Some((l, who)) = lead.filter(|_| !parked_ahead) {
                let car = &self.cars[i];
                let st = &car.state;
                let real = l.gap
                    + if who == Some(usize::MAX) {
                        PLAYER_BOX_MARGIN
                    } else {
                        0.0
                    };
                // (a timetable bus queueing for its own stop is not going round it)
                let queues = car
                    .next_stop()
                    .map(|(ri, ss)| {
                        ri >= st.route_index
                            && st.route_distance(&self.net, ri, ss)
                                < real + obstacle_len + st.front + 15.0
                    })
                    .unwrap_or(false);
                let want = if queues {
                    st.min_gap + 2.0
                } else {
                    car.pass_room.max(st.min_gap)
                };
                let comfortable = st.speed * st.speed / (2.0 * st.decel.max(1.0));
                let stop_gap = if real - want >= comfortable {
                    want
                } else {
                    // (a gap wanted under half a metre is the floor itself: clamp
                    // panicked with its bounds the wrong way round, #138)
                    (real - comfortable).clamp(real.min(0.5).min(want), want)
                };
                keep_back = Some(st.front + (real - stop_gap).max(0.0) + 0.6);
            }
        }
        // a pass it gave up: stop where it said it would
        if let Some(p) = self.cars[i].passing.filter(|p| p.aborted) {
            let st = &self.cars[i].state;
            let at = st.front + (p.hold - st.odometer).max(0.0) + 0.6;
            keep_back = Some(keep_back.map(|k| k.min(at)).unwrap_or(at));
        }
        (standing, obstacle_len, at_stop, keep_back)
    }

    /// Going round what stands in the way: another lane, the other half of the road, and
    /// back in once past.
    #[allow(clippy::too_many_arguments)]
    fn pass_step(&mut self, i: usize, lead: LeadOf, obstacle_len: f32, standing: bool, parked_ahead: bool, at_stop: bool, by_lane: &ByLane, player: Option<PlayerBox>, parked_box: Option<Obb>, feet: &[Footprint]) {
        self.plan_bypass(i, lead.map(|l| l.0.gap), standing, by_lane);
        let way_now = self.way_lanes(&self.cars[i].state, 120.0);
        self.guard_pass(i, by_lane);
        self.plan_pass(
            i,
            lead,
            obstacle_len,
            standing,
            parked_ahead || at_stop,
            &way_now,
            by_lane,
            player,
            parked_box,
            feet,
        );
        // passing: back into the lane once past (and give up if the way out closes
        // before the car has moved)
        {
            let car = &mut self.cars[i];
            if let Some(mut p) = car.passing {
                if !p.aborted
                    && car.stopped > 8.0
                    && car.state.odometer < p.until
                    && car.state.lateral.abs() < p.side * 0.5
                {
                    // held before it got out: back in behind the obstacle rather than
                    // wait half in the oncoming lane
                    p.until = car.state.odometer;
                    p.aborted = true;
                    p.hold = car.state.odometer;
                    car.passing = Some(p);
                }
                if p.aborted {
                    // standing behind what it gave up going round: free to look again
                    // (from where it stands, half out or not)
                    if car.state.speed < 0.1
                        && (car.state.odometer >= p.hold - 0.3 || car.stopped > 1.0)
                    {
                        car.passing = None;
                    }
                } else if car.state.odometer >= p.until {
                    car.state.lateral_target = 0.0;
                    let odo = car.state.odometer;
                    let from = car.state.lateral;
                    if (car.state.lateral_ramp.1 - 0.0).abs() > 1e-3 {
                        // back in gently, as a driver does once past: over the S-curve
                        // planned for the speed it has here and the room ahead
                        car.state.lateral_ramp = (from, 0.0, odo, p.back);
                    }
                    if car.state.lateral.abs() < 0.05 {
                        car.passing = None;
                    }
                }
            }
        }
    }

    /// Merging, the lights and the right of way at the junction ahead: (where the merge
    /// holds car `i`, its way ahead, where a light holds it, where it gives way).
    fn right_of_way(&mut self, i: usize, ts: &TickScene, lead: LeadOf, reservations: &mut Claims) -> RightOfWay {
        let (by_lane, coming, walkers, dt) = (&ts.by_lane, &ts.coming, &ts.walkers, ts.dt);
        let merge_wait = self.plan_route_change(i, by_lane);
        let way = self.way_lanes(&self.cars[i].state, 200.0);
        // traffic lights
        let light = if self.net.lanes[self.cars[i].state.lane].kind == LaneKind::Air {
            None
        } else {
            self.light_stop(i, &way)
        };
        self.cars[i].light_hold = light.is_some();
        self.cars[i].light_at = light;
        if light.is_some() && self.cars[i].state.speed < 0.5 {
            self.held_at_red += 1;
            if self.first_red.is_none()
                && self.cars[i].state.speed < 0.2
                && !self.cars[i].is_bus()
            {
                self.first_red = Some((self.cars[i].id, self.time));
            }
        }
        // right of way: at every junction before the red light's line (and in the one the
        // car is in already) - skipping them all whenever some light ahead was red let a
        // car cross another's path unchecked on its way to a light further on
        let junction = if self.net.lanes[self.cars[i].state.lane].kind == LaneKind::Air {
            None
        } else {
            self.junction_ahead(&way).filter(|jn| {
                light
                    .map(|l| jn.inside || jn.lanes[0].1 < l - 0.5)
                    .unwrap_or(true)
            })
        };
        let yield_at = match &junction {
            Some(jn) => self.junction_stop(
                i,
                jn,
                &way,
                lead.map(|l| l.0),
                by_lane,
                coming,
                reservations,
                walkers,
            ),
            None => {
                self.cars[i].exit_wait = false;
                let old = std::mem::take(&mut self.cars[i].reserved);
                for l in old {
                    if let Some(list) = reservations.get_mut(&l) {
                        list.retain(|&c| c != i);
                    }
                }
                None
            }
        };
        // what it has claimed and is through no longer counts
        {
            let car = &mut self.cars[i];
            let on_way: Vec<usize> = way.iter().map(|w| w.0).collect();
            car.reserved.retain(|l| on_way.contains(l));
            car.yielding = yield_at.is_some();
            car.wait_at = yield_at;
            let st = &mut car.state;
            if yield_at.is_some() && st.speed < 0.3 {
                st.yield_time += dt;
            } else if yield_at.is_none() {
                st.yield_time = 0.0;
            }
            // (`--follow yield`: a car that has stood for a couple of seconds giving way at
            // a junction without lights)
            if car.yielding
                && st.yield_time >= 2.0
                && st.yield_time - dt < 2.0
                && self.first_yield.is_none()
                && !car.is_bus()
                && junction
                    .as_ref()
                    .map(|j| {
                        j.lanes.iter().all(|l| {
                            self.net.lanes[l.0].traffic_light.is_none()
                                && self.net.prev[l.0]
                                    .iter()
                                    .all(|&p| self.net.lanes[p].traffic_light.is_none())
                        })
                    })
                    .unwrap_or(false)
            {
                self.first_yield = Some((car.id, self.time));
            }
        }
        (merge_wait, way, light, yield_at)
    }

    /// Where car `i` has to stop (distance from its origin) and what holds it: the light,
    /// the junction, the merge, what it keeps back from, people on foot, its own pulling
    /// out of a parking space, the player's bus leaving its stop.
    #[allow(clippy::too_many_arguments)]
    fn stop_points(&mut self, i: usize, ts: &TickScene, way: &[(usize, f32)], lead: LeadOf, light: Option<f32>, yield_at: Option<f32>, merge_wait: Option<f32>, keep_back: Option<f32>) -> (Option<f32>, (&'static str, f32)) {
        let (debug, dt) = (ts.debug, ts.dt);
        let for_people = if self.net.lanes[self.cars[i].state.lane].kind == LaneKind::Air {
            None
        } else {
            self.people_stop(i, way)
        };
        if let Some((at, who)) = for_people {
            let car = &self.cars[i];
            if debug && car.state.speed > 0.5 {
                log::info!(
                    "t={:.2}: car {} stops for somebody on foot {:.1} m ahead",
                    self.time,
                    car.id,
                    at - car.state.front
                );
            }
            // somebody who never moves out of the way (standing in the carriageway)
            if car.stopped >= 20.0 && car.stopped - dt < 20.0 {
                log::info!(
                    "car {} has stood 20 s for somebody on foot at ({:.1}, {:.1})",
                    car.id,
                    who.x,
                    who.y
                );
            }
        }
        let people = for_people.map(|x| x.0);
        // a car that has just left its parking space waits a moment before pulling out
        let parked_wait = (self.cars[i].pull_out > 0.0).then(|| self.cars[i].state.front + 0.1);
        self.cars[i].pull_out = (self.cars[i].pull_out - dt).max(0.0);
        // the player's bus or a LAN player's indicating out of its stop (only the LAN
        // players' front sections have an `others_signal`: a rear section does not pull out)
        let player_signal = (self.player_signal_age, self.player_signalling);
        let let_out = self
            .player
            .and_then(|p| self.letting_out(i, &p, player_signal))
            .into_iter()
            .chain(
                ts.others
                    .iter()
                    .filter_map(|(id, b)| self.letting_out(i, b, *self.others_signal.get(id)?)),
            )
            .reduce(f32::min)
            .map(|g| self.cars[i].state.front + (g - 1.0).max(0.0));
        let stop_at = [light, yield_at, merge_wait, keep_back, people, parked_wait, let_out]
            .into_iter()
            .flatten()
            .reduce(f32::min);
        let mut why: (&'static str, f32) = ("", f32::MAX);
        for (name, v) in [("light", light), ("yield", yield_at), ("merge", merge_wait), ("keep_back", keep_back), ("people", people), ("pull_out", parked_wait), ("let_out", let_out)] {
            if let Some(v) = v {
                if v < why.1 {
                    why = (name, v);
                }
            }
        }
        self.cars[i].held = stop_at.is_some() || lead.map(|l| l.0.gap < 12.0).unwrap_or(false);
        (stop_at, why)
    }

    /// A timetable bus's service (its stops), any other car's place in its lane; parking.
    fn service_and_park(&mut self, i: usize, ts: &TickScene, way: &[(usize, f32)], kerb_swerve: Option<f32>, mut stop_at: Option<f32>, mut why: (&'static str, f32)) -> (Option<f32>, (&'static str, f32)) {
        let (debug, dt) = (ts.debug, ts.dt);
        // a timetable bus: its stops (see `bus_service`); any other car keeps to the middle
        // of its lane, or swerves round a car parked at the kerb
        {
            let car = &mut self.cars[i];
            if let Some(service) = car.bus.as_mut() {
                let wanted = self.stop_wishes.as_ref().map(|(alighting, waiting)| {
                    alighting.contains(&car.id) || service.stops.front().is_some_and(|s| waiting.contains(&s.id))
                });
                let ctx = super::bus_service::Ctx {
                    wanted,
                    net: &self.net,
                    way,
                    day_time: self.day_time,
                    dt,
                    id: car.id,
                    stopped: car.stopped,
                    passing: car.passing.is_some(),
                    kerb_swerve,
                    debug: debug || omsi_cfg::flags::OMSI_DEBUG_PAX.is_set(),
                    timed_waits_only: self.timed_waits_only,
                };
                if let Some(at) = service.step(&mut car.state, &mut car.vehicle, &ctx) {
                    stop_at = Some(stop_at.map(|x| x.min(at)).unwrap_or(at));
                    if at < why.1 {
                        why = ("service", at);
                    }
                }
                service.feed_timetable(&mut car.vehicle, self.day_time);
            } else if car.passing.is_none() && car.park.is_none() {
                // round a car parked at the kerb, else in the middle of the lane
                let target = kerb_swerve.unwrap_or(0.0);
                if debug && (target - car.state.lateral_target).abs() > 0.3 {
                    log::info!(
                        "t={:.2}: car {} swerves to {target:+.2} (was {:+.2}, now at {:+.2})",
                        self.time,
                        car.id,
                        car.state.lateral_target,
                        car.state.lateral
                    );
                }
                car.state.lateral_target = target;
            }
        }
        // parking: stop beside the space, move over into it, and stand there
        {
            let car = &mut self.cars[i];
            if let Some(mut plan) = car.park {
                let st = &mut car.state;
                match way.iter().find(|w| w.0 == plan.lane) {
                    None if st.lane != plan.lane => {
                        // (its way went elsewhere after all)
                        car.park = None;
                    }
                    found => {
                        let dl = found.map(|w| w.1).unwrap_or(-st.s);
                        let d = dl + plan.s; // origin to the space's middle
                        if d < 80.0 {
                            let at = d + st.front + 0.3;
                            stop_at = Some(stop_at.map(|x| x.min(at)).unwrap_or(at));
                            if at < why.1 {
                                why = ("park", at);
                            }
                            st.signal = 2;
                            st.signal_time = st.signal_time.max(1.0);
                        }
                        // over into the space along the last metres, the move ending
                        // where the car stops (the lateral place follows the distance
                        // driven: started too late, the car stood half out of the space)
                        if car.passing.is_none() {
                            let len = (plan.lat.abs() * 6.0).clamp(10.0, 22.0);
                            if !plan.ramped && d < len + 1.0 && d > 2.0 {
                                plan.ramped = true;
                                st.lateral_target = plan.lat;
                                st.lateral_ramp = (st.lateral, plan.lat, st.odometer, (d - 0.6).max(4.0));
                            } else if plan.ramped {
                                st.lateral_target = plan.lat;
                            } else {
                                st.lateral_target = 0.0;
                            }
                        }
                        if d.abs() < 1.5 && st.speed < 0.2 && (st.lateral - plan.lat).abs() < 0.3 {
                            plan.done = true;
                        } else if d < -4.0 {
                            car.park = None;
                            st.lateral_target = 0.0;
                        }
                        if car.park.is_some() {
                            car.park = Some(plan);
                        }
                    }
                }
            }
        }
        (stop_at, why)
    }

    /// The end of the way: a timetable bus at the end of its trip drives on as ordinary
    /// traffic until it is out of sight; a dead end is a place to stop.
    fn end_of_way(&mut self, i: usize, ts: &TickScene, way: &[(usize, f32)], mut stop_at: Option<f32>, mut why: (&'static str, f32)) -> (Option<f32>, (&'static str, f32)) {
        let debug = ts.debug;
        // the end of the way: a timetable bus at the end of its trip drives on as
        // ordinary traffic until it is out of sight; a dead end is a place to stop
        {
            let car = &mut self.cars[i];
            let st = &mut car.state;
            let (last, end) = way
                .last()
                .map(|&(l, d)| (l, d + self.net.lanes[l].length()))
                .unwrap_or((st.lane, 0.0));
            let exhausted = if st.route.is_empty() {
                self.net.lanes[last].next.is_empty()
            } else {
                st.route.last() == Some(&last)
            };
            let air = self.net.lanes[st.lane].kind == LaneKind::Air;
            if exhausted && st.change.is_none() && end < 150.0 {
                let service = car.bus.as_deref_mut();
                let in_service = service
                    .as_ref()
                    .map(|b| b.route_open || (b.stops.is_empty() && !b.at_stop()))
                    .unwrap_or(false);
                let stops_left = service.as_ref().map(|b| !b.stops.is_empty() || b.at_stop()).unwrap_or(false);
                let waited_out = car.stopped > ROUTE_WAIT_MAX
                    && service.as_ref().map(|b| b.route_open).unwrap_or(false);
                if waited_out && !air {
                    // it has waited long for more route where the tiles are loaded: the
                    // rest of its trip does not join what it has (a track the map does
                    // not have any more). It stood in the carriageway for good, the
                    // traffic queued behind it; now it drives on as ordinary traffic
                    // and leaves once out of sight.
                    if let Some(b) = service {
                        b.route_open = false;
                        b.stops.clear();
                    }
                    st.route.clear();
                    st.route_index = 0;
                    st.planned_next = None;
                    st.ahead.clear();
                    st.plan_next(&self.net);
                    car.gone = true;
                    log::info!("timetable bus {} waited {:.0} s for the rest of its route at ({:.0}, {:.0}): it does not join; the bus drives on and leaves", car.id, car.stopped, car.vehicle.position.x, car.vehicle.position.y);
                } else if !st.route.is_empty() && in_service && !air && !car.gone {
                    // the end of the route it has: where the loaded tiles end, it waits
                    // for more route (or to be taken off out of sight); at the end of its
                    // trip, it stops there and waits for the timetable
                    let at = end - 0.5;
                    stop_at = Some(stop_at.map(|x| x.min(at)).unwrap_or(at));
                    if at < why.1 {
                        why = ("end", at);
                    }
                    let b = service.unwrap();
                    if !b.route_open && st.speed < 0.3 && !b.trip_done() {
                        b.phase = Phase::TripDone;
                        b.phase_t = 0.0;
                        if debug {
                            log::info!("t={:.1}: timetable bus {} at the end of its trip", self.time, car.id);
                        }
                    }
                } else if !st.route.is_empty() && !stops_left {
                    st.route.clear();
                    st.route_index = 0;
                    st.planned_next = None;
                    st.ahead.clear();
                    st.plan_next(&self.net);
                    car.gone = true;
                    if debug {
                        log::info!(
                            "t={:.1}: car {} finished its trip, drives on until out of sight",
                            self.time,
                            car.id
                        );
                    }
                } else if st.route.is_empty() {
                    // a dead end (the map's edge, the end of a street spline): Omsi.exe
                    // drives on at speed and deletes the car the frame it runs out of
                    // road (0x71dc9c finds no next segment, 0x6fe3fc deletes it), and
                    // `drive` takes it off there. Braking for the end, the cars stopped
                    // there one by one and those behind queued into a stop-and-go (an
                    // aircraft flies on in any case)
                    car.gone = true;
                }
            }
        }
        (stop_at, why)
    }

    /// Car `i` drives: what it waited for, its speed, and its frame for the body and the
    /// scripts (None: it ran out of road).
    #[allow(clippy::too_many_arguments)]
    fn drive_car(&mut self, i: usize, ts: &TickScene, lead: LeadOf, stop_at: Option<f32>, why: (&'static str, f32), light: Option<f32>, yield_at: Option<f32>, merge_wait: Option<f32>) -> Option<AiFrame> {
        let (debug, dt) = (ts.debug, ts.dt);
        let lead_id = lead
            .and_then(|l| l.1)
            .filter(|&j| j < self.cars.len())
            .map(|j| self.cars[j].id);
        let car = &mut self.cars[i];
        car.lead_car = lead_id;
        if car.state.speed.abs() < 0.1 && !car.at_stop() {
            car.stopped += dt;
        } else {
            car.stopped = 0.0;
        }
        if car.state.speed.abs() < 1.0 && !car.at_stop() {
            car.crawl += dt;
        } else {
            car.crawl = 0.0;
        }
        if (car.state.odometer - car.progress.0).abs() > 2.0 || car.at_stop() {
            car.progress = (car.state.odometer, 0.0);
        } else {
            car.progress.1 += dt;
        }
        let stood = car.stopped.max(car.progress.1);
        // a random car that has stood for a minute without a light or a junction
        // holding it has given up: it leaves as soon as nobody sees it
        // (one yielding for minutes is in a gridlock nobody else will end)
        if (stood > 60.0 && !car.yielding || stood > 150.0) && !car.is_bus() && !car.light_hold && !car.gone
        {
            car.gone = true;
            if let Some(s) = self.stats.as_mut() {
                s.window.gave_up += 1;
            }
            if debug {
                log::info!(
                    "t={:.1}: car {} stood for {:.0} s: taken off once out of sight",
                    self.time,
                    car.id,
                    stood
                );
            }
        }
        let lane_before = car.state.lane;
        let lead_now = lead.map(|l| l.0);
        car.why = match (why.1 < f32::MAX, lead_now) {
            (_, Some(l)) if l.gap + car.state.front < why.1 => (
                match lead.and_then(|l| l.1) {
                    Some(usize::MAX) => "player",
                    Some(_) => "lead",
                    None => "parked",
                },
                l.gap,
            ),
            (true, _) => (why.0, why.1 - car.state.front),
            _ => ("", 0.0),
        };
        // whom it stands for (the waits-for graph of `stats`)
        car.waits_on = match car.why.0 {
            "lead" | "keep_back" => lead_id,
            "player" => Some(stats::WAITS_ON_PLAYER),
            "yield" => car.yield_to,
            _ => None,
        };
        if car.fresh > 0.0 {
            car.fresh -= dt;
            // placed moving before a queue or a red light: arrive slower rather than
            // start with an emergency stop
            for _ in 0..16 {
                if car.state.speed < 0.5
                    || car.state.desired_accel(&self.net, lead_now, stop_at) >= -car.state.decel
                {
                    break;
                }
                car.state.speed *= 0.8;
            }
            if car.state.speed < 0.5 {
                car.state.speed = 0.0;
            }
        }
        // edging out round something standing close ahead
        car.state.accel_cap = car
            .passing
            .filter(|p| p.creep && !p.aborted && car.state.odometer < p.block + CREEP_PAST)
            .map(|_| PULL_OUT_ACCEL);
        let (speed_before, lane_now, s_now) = (car.state.speed, car.state.lane, car.state.s);
        if debug && car.stopped > 30.0 {
            car.holding = Some(format!("lead {:?} (car {:?}), stop {:?} (light {:?}, junction {:?}, merge {:?}), bus {:?}, lane {} s {:.1} of {:.1}, next {:?}, lateral {:.2}, stops {:?}", lead_now, lead_id, stop_at.map(|x| x - car.state.front), light, yield_at, merge_wait, car.bus.as_ref().map(|b| (b.phase, b.phase_t as i32)), car.state.lane, car.state.s, self.net.lanes[car.state.lane].length(), car.state.planned_next, car.state.lateral, car.next_stop()));
            if car.stopped - dt <= 30.0 {
                log::info!(
                    "t={:.1}: car {} ({}) has stood for 30 s at ({:.0}, {:.0}): {}",
                    self.time,
                    car.id,
                    car.vehicle.ty.def.type_name,
                    car.vehicle.position.x,
                    car.vehicle.position.y,
                    car.holding.as_deref().unwrap_or("-")
                );
            }
        }
        if omsi_cfg::flags::OMSI_DEBUG_CAR.parse::<u64>() == Some(car.id) {
            let up: Vec<usize> = car.state.upcoming().take(4).collect();
            log::info!("t={:.2} car {}: v {:.2} lane {} s {:.1}/{:.1} upcoming {:?} bend {:.2} desired {:.2} lead {:?} stop {:?} why {:?}", self.time, car.id, car.state.speed, car.state.lane, car.state.s, self.net.lanes[car.state.lane].length(), up, car.state.curve_speed(&self.net), car.state.desired_accel(&self.net, lead_now, stop_at), lead_now.map(|l| l.gap), stop_at.map(|x| x - car.state.front), car.why);
        }
        if !car.state.drive(&self.net, dt, lead_now, stop_at) {
            if debug {
                log::info!("t={:.1}: car {} ran out of road at {:.1} m/s: taken off", self.time, car.id, car.state.speed);
            }
            return None;
        }
        if debug && car.state.acc < -4.5 {
            // hard braking is for emergencies: say what asked for it
            let who = match lead.and_then(|l| l.1) {
                Some(usize::MAX) => "the player".to_string(),
                Some(_) => format!("car {}", lead_id.unwrap_or(0)),
                None if lead.is_some() => "a parked car".to_string(),
                None => "-".to_string(),
            };
            log::info!("t={:.2}: car {} brakes {:.1} m/s² at {:.1} m/s on lane {lane_now} s {s_now:.2} (len {:.1}): lead {:?} ({who}), stop {:?} (light {:?}, junction {:?}, merge {:?})", self.time, car.id, car.state.acc, speed_before, self.net.lanes[lane_now].length(), lead_now, stop_at.map(|x| x - car.state.front), light.map(|x| x - car.state.front), yield_at.map(|x| x - car.state.front), merge_wait);
        }
        if car.state.lane != lane_before
            && self.first_turner.is_none()
            && !car.is_bus()
            && self.net.lanes[car.state.lane].turn != 0
        {
            self.first_turner = Some((car.id, self.time));
        }
        car.state.update_blinker(&self.net);
        if matches!(car.bus.as_ref().map(|b| b.phase), Some(Phase::Boarding | Phase::Waiting)) {
            // waiting at a stop: dark until it is about to pull away
            car.state.blinker = 0;
        }
        if omsi_cfg::flags::OMSI_DEBUG_DOORS.is_set() && car.is_bus() && (self.time * 2.0).floor() != ((self.time - dt) * 2.0).floor() {
            let v = &car.vehicle;
            let g = |n: &str| v.var(n).map(|x| format!("{x:.2}")).unwrap_or("-".into());
            let st = g("AI_Scheduled_AtStation");
            if st != "0.00" || car.bus.as_ref().is_some_and(|b| b.at_stop()) {
                log::info!("doors t={:.1} car {} {} phase {:?} speed {:.1}: AtStation {st} door {} {} {} {} target {} {} {} halte {} timer {}", self.time, car.id, v.ty.def.type_name, car.bus.as_ref().map(|b| b.phase), car.state.speed, g("door_0"), g("door_1"), g("door_2"), g("door_3"), g("doorTarget_0"), g("doorTarget_1"), g("doorTarget_2"), g("bremse_halte_sw"), g("door_AI_timer"));
            }
        }
        // An emergency vehicle (its script sets `TrafficPriority`, the stock ambulance)
        // is told `TrafficPriorityWarningNeeded` while something holds it up close ahead:
        // a car or the player's bus it catches up with or has to follow, a red light, a
        // junction it has to wait at. Its script sounds the siren on it; without the
        // variable it drove silent all day. (Behind a car at the same speed the siren
        // flickered on a strict "slower".)
        let priority_warning = car.vehicle.var("TrafficPriority").is_some_and(|v| v > 0.5)
            && (lead_now.is_some_and(|l| l.gap < PRIORITY_WARN_GAP && l.speed < car.state.speed + 0.5)
                || stop_at.is_some_and(|x| x - car.state.front < PRIORITY_WARN_GAP));
        Some(AiFrame {
            speed: car.state.speed,
            odometer: car.state.odometer,
            steer_deg: 0.0,
            blinker: car.state.blinker,
            brake: car.state.braking,
            lights: self.night,
            at_station: car.at_station() as i32,
            at_station_side: car.at_station_side(),
            priority_warning,
            engine_off: car.bus.as_ref().is_some_and(|b| !b.engine_running(self.day_time)),
        })
    }

    /// Who can be seen: a car out of the view (and farther than the mirrors and the
    /// shadows reach) leaves its animations as they are and is not drawn at all.
    fn set_visibility(&mut self) {
        if let Some(v) = self.viewer {
            for c in &mut self.cars {
                let p = c.vehicle.position;
                let r = (c.state.front + c.state.rear).abs().max(4.0) as f64 + 2.0;
                c.vehicle.ai_visuals =
                    (p - v.pos).length() < UNSEEN_NEAR || v.frames(p, r);
            }
        }
    }

    /// The bodies and the scripts of the AI vehicles, on the workers.
    fn step_bodies(&mut self, dt: f32, frames: &mut [Option<AiFrame>]) {
        // The bodies and the scripts of the AI vehicles run in parallel: each car follows
        // its own way and its OMSI script is its own little machine reading only its own
        // state; with thirty cars and a dozen timetable buses they were the largest single
        // cost of a frame.
        {
            use rayon::prelude::*;
            let net = &self.net;
            type Work<'a> = (&'a AiState, &'a mut AiBody, &'a mut VehicleInstance, &'a mut AiFrame, &'a mut std::collections::VecDeque<(f64, DVec3)>, &'a mut f32);
            let mut work: Vec<Work> = self
                .cars
                .iter_mut()
                .zip(frames.iter_mut())
                .filter_map(|(c, f)| {
                    let f = f.as_mut()?;
                    Some((&c.state, &mut c.body, &mut c.vehicle, f, &mut c.rail_trail, &mut c.ai_secs))
                })
                .collect();
            work.sort_by(|a, b| b.5.total_cmp(a.5));
            let mut jobs: Vec<Vec<Work>> = Vec::new();
            for w in work {
                match jobs.last_mut() {
                    Some(job) if *w.5 < AI_JOB_SECS && *job[0].5 < AI_JOB_SECS && job.len() < 4 => job.push(w),
                    _ => jobs.push(vec![w]),
                }
            }
            let profile = omsi_cfg::flags::OMSI_PROFILE.is_set();
            // (a few cars per job: every job handed out wakes a worker, and the waking cost
            // the main thread more than a car's work)
            jobs.into_par_iter().flatten_iter()
                .for_each(|(state, body, vehicle, frame, trail, secs)| {
                    let t0 = std::time::Instant::now();
                    let ground = vehicle.ground.clone();
                    let contact = vehicle.contact.clone();
                    let rail = body.kind == MotionKind::Rail;
                    if rail {
                        record_rail_trail(trail, state.odometer as f64, state.way_point(net, 0.0));
                    }
                    let trail = &*trail;
                    let behind = |d: f64| rail_behind(trail, state, net, d);
                    body.step(
                        dt,
                        state.speed,
                        &|d| if rail && d < 0.0 { behind(-d as f64) } else { state.way_point(net, d) },
                        ground
                            .as_ref()
                            .map(|g| g.as_ref() as &dyn Fn(f64, f64) -> Option<f64>),
                        contact.as_deref(),
                    );
                    body.apply(vehicle);
                    if rail && !vehicle.trailers.is_empty() {
                        // the coupled cars (a train's, a tram's sections) on the track it
                        // came along, not dragged round the bends like a road trailer
                        vehicle.retrail(0.0, &|d| Some(behind(d)));
                    }
                    frame.steer_deg = body.steer;
                    let t1 = std::time::Instant::now();
                    vehicle.update_ai(dt, frame);
                    *secs = t0.elapsed().as_secs_f32();
                    if profile && t0.elapsed().as_secs_f64() > 0.01 {
                        log::info!(
                            "  slow AI frame: {} body {:.1} ms, scripts {:.1} ms",
                            vehicle.ty.def.path.display(),
                            (t1 - t0).as_secs_f64() * 1000.0,
                            t1.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                });
        }
    }

    /// The debug output of a tick (`OMSI_DEBUG_TRAILERS`, `OMSI_DEBUG_TRAFFIC`,
    /// `OMSI_TRACE_AI`, `OMSI_CHECK_OVERLAP`).
    fn debug_tick(&mut self, debug: bool, player: Option<PlayerBox>, others: &[(u32, PlayerBox)], frames: &[Option<AiFrame>]) {
        if omsi_cfg::flags::OMSI_DEBUG_TRAILERS.is_set() {
            // coupled parts off the level of what pulls them (#140: trains' and articulated
            // buses' rear parts under bridges)
            for c in &self.cars {
                let mut lead_z = c.vehicle.position.z;
                for (k, t) in c.vehicle.trailers.iter().enumerate() {
                    let (pitch, axle, track) = t.debug_pose();
                    if pitch.abs() > 4.0 || (t.position.z - lead_z).abs() > 1.2 {
                        log::info!("trailer: car {} {} part {k} at ({:.1}, {:.1}, {:.2}) lead z {:.2} pitch {pitch:.1} axle {:?} track {:?} lane {} kind {:?}", c.id, c.vehicle.ty.def.type_name, t.position.x, t.position.y, t.position.z, lead_z, axle, track.map(|p| p.z), c.state.lane, self.net.lanes[c.state.lane].kind);
                    }
                    lead_z = t.position.z;
                }
            }
        }
        if debug {
            // a car pulled round harder than a driver would: what way was it given?
            for (c, fr) in self.cars.iter().zip(frames) {
                if fr.is_none() || c.body.a_lat.abs() < 4.0 || !self.logged_hard.insert(c.id) {
                    continue;
                }
                let st = &c.state;
                let lanes: Vec<String> = std::iter::once(st.lane)
                    .chain(st.upcoming())
                    .take(5)
                    .map(|l| {
                        let l_ = &self.net.lanes[l];
                        format!(
                            "{l} ({} turn {} len {:.1} h {:.0}->{:.0} k {:.3}->{:.3})",
                            l_.name,
                            l_.turn,
                            l_.length(),
                            l_.start_heading(),
                            l_.end_heading(),
                            l_.curvature.first().copied().unwrap_or(0.0),
                            l_.curvature.last().copied().unwrap_or(0.0)
                        )
                    })
                    .collect();
                log::info!("t={:.1}: car {} {} at {:.1} m/s pulled {:.1} m/s² sideways (steering {:.1}°), bend speed {:.1}, s {:.1}, way {}", self.time, c.id, c.vehicle.ty.def.path.file_stem().unwrap_or_default().to_string_lossy(), st.speed, c.body.a_lat, c.body.steer, st.curve_speed(&self.net), st.s, lanes.join(" / "));
            }
        }
        if let Some(f) = self.trace.as_mut() {
            use std::io::Write;
            // the player's vehicle as id 0 (its box centre, half length both ways)
            if let Some((c, h, hl, hw, v)) = player {
                let _ = writeln!(f, "{:.3},0,player,{:.3},{:.3},{:.3},{:.3},0,0,0,{:.3},-1,0,0,0,0,0,0,0,0,0,0,{hl:.2},{hl:.2},{hw:.2},0,,0,None", self.time, c.x, c.y, c.z, h, v);
            }
            // (`OMSI_TRACE_AI_BUSES=1`: the timetable buses only)
            let buses_only = omsi_cfg::flags::OMSI_TRACE_AI_BUSES.is_set();
            for (c, fr) in self.cars.iter().zip(frames) {
                let Some(fr) = fr else { continue };
                if buses_only && !c.is_bus() {
                    continue;
                }
                let v = &c.vehicle;
                let lane_heading = self.net.lanes[c.state.lane]
                    .at(c.state.s)
                    .1
                    .rem_euclid(360.0);
                let _ = writeln!(f, "{:.3},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{},{:.2},{},{},{:.2},{:.2},{},{:.2},{},{},{},{:.2},{:.2},{:.2},{},{},{:.1},{:?},{:.3}", self.time, c.id, v.ty.def.path.file_stem().unwrap_or_default().to_string_lossy(), v.position.x, v.position.y, v.position.z, v.heading, v.pitch, v.bank, fr.steer_deg, c.state.speed, c.state.lane, c.state.s, fr.blinker, self.net.lanes[c.state.lane].turn, lane_heading, c.state.lateral, c.at_station() as i32, c.state.acc, c.yielding as i32, c.light_hold as i32, c.passing.is_some() as i32, c.state.front, c.state.rear, c.half_width, c.is_bus() as i32, c.why.0, c.why.1.min(999.0), c.bus.as_ref().map(|b| b.phase), self.net.lanes[c.state.lane].at(c.state.s).0.z);
            }
        }
        if omsi_cfg::flags::OMSI_CHECK_OVERLAP.is_set() {
            self.check_overlaps(player, others);
        }
    }

    /// What holds each car that has stood for over a minute (the offscreen traffic health
    /// report): why, its blinker, its light and the lane.
    pub fn stuck_report(&self) -> Vec<String> {
        self.cars
            .iter()
            .filter(|c| c.stopped > 60.0 || (c.stopped > 15.0 && c.state.signal != 0 && c.yielding))
            .map(|c| {
                let st = &c.state;
                let light = self.way_lanes(st, 60.0).into_iter().find_map(|(l, d)| {
                    self.net.lanes[l].traffic_light.and_then(|(ci, li)| self.lights.get(ci).map(|ctl| format!("light {ci}/{li} state {} at {d:.1} (time {:.0}, held {}, cycle {:.0}, phases {:?}, stops {:?})", ctl.state(li), ctl.time, ctl.held, ctl.cycle, ctl.lights, ctl.stops)))
                });
                format!(
                    "car {} {} stood {:.0} s: why {:?} blinker {} yielding {} light_hold {} lane {} ({}) s {:.1}/{:.1} next {:?} {} lead {:?} bus {:?} pos ({:.1}, {:.1}) junction {} [geo_block {:?} squeeze {:?} wait_at {:?} held {} start_timer {:.2} accel_cap {:?} crawl {:.1} passing {} park {} pull_out {:.1} acc {:.2}]",
                    c.id,
                    c.vehicle.ty.def.path.file_stem().unwrap_or_default().to_string_lossy(),
                    c.stopped,
                    c.why,
                    st.signal,
                    c.yielding,
                    c.light_hold,
                    st.lane,
                    self.net.lanes[st.lane].name,
                    st.s,
                    self.net.lanes[st.lane].length(),
                    st.upcoming().take(3).collect::<Vec<_>>(),
                    light.unwrap_or_default(),
                    c.lead_info,
                    c.bus.as_ref().map(|b| b.phase),
                    c.vehicle.position.x,
                    c.vehicle.position.y,
                    c.junction_why,
                    c.geo_block,
                    c.squeeze,
                    c.wait_at,
                    c.held,
                    st.start_timer,
                    st.accel_cap,
                    c.crawl,
                    c.passing.is_some(),
                    c.park.is_some(),
                    c.pull_out,
                    st.acc
                )
            })
            .collect()
    }

    /// `OMSI_CHECK_OVERLAP`: every AI vehicle whose body has got into another's or into a
    /// player's bus (by more than 20 cm), once per pair and 10 s, with what each was doing.
    pub fn check_overlaps(&mut self, player: Option<PlayerBox>, others: &[(u32, PlayerBox)]) {
        static SEEN: std::sync::OnceLock<parking_lot::Mutex<HashMap<(u64, u64), f32>>> = std::sync::OnceLock::new();
        let seen = SEEN.get_or_init(|| parking_lot::Mutex::new(HashMap::new()));
        let feet = self.footprints();
        let boxes: Vec<(u64, Footprint)> = player
            .iter()
            .map(|b| (u64::MAX, *b))
            .chain(others.iter().map(|(id, b)| (u64::MAX - 1 - *id as u64, *b)))
            .map(|(id, (c, h, hl, hw, v))| {
                let hr = h.to_radians();
                (id, Footprint { car: usize::MAX, center: c.truncate(), fwd: DVec2::new(hr.sin(), hr.cos()), right: DVec2::new(hr.cos(), -hr.sin()), half_len: hl as f64, half_w: hw as f64, speed: v, z: c.z })
            })
            .collect();
        let why = |c: &AiCar| format!("{} v {:.1} lane {} s {:.1} lat {:.2} why {:?} passing {} change {}", c.vehicle.ty.def.path.file_stem().unwrap_or_default().to_string_lossy(), c.state.speed, c.state.lane, c.state.s, c.state.lateral, c.why, c.passing.is_some(), c.state.change.is_some());
        for (a, fa) in feet.iter().enumerate() {
            let ca = &self.cars[fa.car];
            let hit = |other: u64, fb: &Footprint, desc: String| {
                if (fa.z - fb.z).abs() > 3.0 || !fa.overlaps(fb, -0.2) {
                    return;
                }
                let key = (ca.id.min(other), ca.id.max(other));
                let mut m = seen.lock();
                if m.get(&key).is_some_and(|t| self.time - *t < 10.0) {
                    return;
                }
                m.insert(key, self.time);
                log::info!("t={:.1}: OVERLAP car {} ({}) with {} at ({:.1}, {:.1})", self.time, ca.id, why(ca), desc, fa.center.x, fa.center.y);
            };
            for fb in feet.iter().skip(a + 1) {
                if fb.car == fa.car {
                    continue;
                }
                let cb = &self.cars[fb.car];
                hit(cb.id, fb, format!("car {} ({})", cb.id, why(cb)));
            }
            for (id, fb) in &boxes {
                hit(*id, fb, format!("player box {} v {:.1}", u64::MAX - id, fb.speed));
            }
        }
    }
}
