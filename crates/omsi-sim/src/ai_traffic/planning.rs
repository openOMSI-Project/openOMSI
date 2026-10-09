//! A car's plans: what stands ahead of it, lane changes, going round what stands in
//! the way on the other half of the road.

use super::*;

/// Seconds a car at `st` needs to drive `dist` metres out on the other half of the road:
/// the first `creep` metres edging out, the rest speeding up to `v_cap` (a driver standing
/// still first reacts).
pub fn pass_time(dist: f32, creep: f32, st: &AiState, v_cap: f32) -> f32 {
    let wait = if st.speed < 0.1 { st.reaction } else { 0.0 };
    let accel = st.accel * 0.85;
    if creep <= 0.0 || dist <= 0.0 {
        return wait + arrival_time(dist, st.speed, accel, v_cap);
    }
    let a0 = accel.min(PULL_OUT_ACCEL);
    let first = creep.min(dist);
    let v1 = (st.speed * st.speed + 2.0 * a0 * first)
        .sqrt()
        .min(v_cap.max(st.speed));
    wait + arrival_time(first, st.speed, a0, v_cap) + arrival_time(dist - first, v1, accel, v_cap)
}

/// Seconds a car stands at a joint where two lanes become one before it takes its turn.
pub const ZIP_WAIT: f32 = 3.0;

/// Is car `c`, its front `dist` metres short of a joint, standing there waiting for its
/// turn? `clear`: where it waits for the other lane's cars (`merge_clearance`).
pub fn zip_waiting(c: &AiCar, dist: f32, clear: f32) -> bool {
    c.state.speed < 0.5 && dist < clear + c.state.min_gap + 2.5 && c.stopped > ZIP_WAIT
}

/// How far before the joint where lane `b` runs into the lane `a` runs into a car on `b`
/// (half width `hb`) keeps its front to be out of the way of a car on `a` (half width
/// `ha`): where `b` has come within the two half widths and a margin of `a` (m, at least
/// one). Waiting a metre short of the joint, a car at a merge of 45 degrees still stood
/// with its nose in the other lane: the car it let go first could not get past it, and
/// the two lanes took turns at a car every quarter of a minute.
pub fn merge_clearance(net: &Network, b: usize, a: usize, ha: f32, hb: f32) -> f32 {
    let (lb, la) = (&net.lanes[b], &net.lanes[a]);
    let need = (ha + hb + 0.3) as f64;
    let mut x = 1.0f32;
    while x < 12.0 && x < lb.length() {
        let (p, _) = lb.at(lb.length() - x);
        if la.nearest_point(p).is_none_or(|(_, d)| d > need) {
            return x;
        }
        x += 0.5;
    }
    x.min(12.0)
}

impl TrafficSim {
    /// Nearest vehicle ahead of position `s` on `lane` (following the lanes `plan` has
    /// chosen after it, else the first `next`, for up to `look` m): (distance from `s` to
    /// its rear, its speed along the lane, its index). A vehicle beside the lane's middle
    /// far enough to be passed (a bus in its bay) does not count; one coming the other way
    /// round an obstacle does, standing.
    pub fn obstacle_from(
        &self,
        i: usize,
        lane: usize,
        s: f32,
        plan: Option<&[usize]>,
        look: f32,
        by_lane: &HashMap<usize, Vec<(usize, f32, f32, bool)>>,
    ) -> Option<(f32, f32, usize)> {
        let me = &self.cars[i];
        let mut best: Option<(f32, f32, usize)> = None;
        let mut lane = lane;
        let mut offset = 0.0f32;
        let mut s_from = s;
        let mut upcoming = plan.map(|p| p.iter().copied());
        // (as many lanes as fit into `look`: a junction is a string of short ones, and four
        // of them hid a bus standing just behind it)
        for _ in 0..10 {
            if let Some(list) = by_lane.get(&lane) {
                for &(j, os, lat, foreign) in list {
                    if j == i || os <= s_from {
                        continue;
                    }
                    let o = &self.cars[j];
                    // Two that overlap (a car that ended up beside or in a bus) each found the
                    // other ahead - one's front past the other's rear - and each waited for the
                    // other for good. One that follows this car already and whose middle is
                    // behind this one's is not its lead: the front one drives off.
                    if o.lead_car == Some(me.id) {
                        let h = me.vehicle.heading.to_radians();
                        let fwd = DVec2::new(h.sin(), h.cos());
                        if (o.vehicle.position - me.vehicle.position).truncate().dot(fwd) < 0.0 {
                            continue;
                        }
                    }
                    // (the lateral place this car will have when it gets there: pulling out
                    // round a standing bus, it is clear of it before it arrives)
                    let mine = me.state.lateral_ahead(offset + (os - s_from));
                    if !foreign && (lat - mine).abs() > me.half_width + o.half_width + 0.3 {
                        continue;
                    }
                    let (d, v) = if foreign {
                        (offset + (os - s_from) - o.state.front, 0.0)
                    } else {
                        (offset + (os - s_from) - o.state.rear, o.state.speed)
                    };
                    if best.map(|b| d < b.0).unwrap_or(true) {
                        best = Some((d.max(0.0), v, j));
                    }
                }
            }
            let l = &self.net.lanes[lane];
            offset += l.length() - s_from;
            if offset > look || best.is_some() {
                break;
            }
            let next = match upcoming.as_mut() {
                Some(u) => u.next(),
                None => l.next.first().copied(),
            };
            match next {
                Some(n) => {
                    lane = n;
                    s_from = 0.0;
                }
                None => break,
            }
        }
        best.filter(|d| d.0 < look)
    }

    /// The vehicle car `i` follows: (gap from its front bumper, its speed, its index) along
    /// its lane chain (up to `look` m); during a lane change the target lane counts too.
    pub fn obstacle_ahead(
        &self,
        i: usize,
        look: f32,
        by_lane: &HashMap<usize, Vec<(usize, f32, f32, bool)>>,
    ) -> Option<(Lead, usize)> {
        let me = &self.cars[i].state;
        // once well over into the new lane, what stands in the old one no longer matters
        // (that is the whole point of pulling out round it)
        let committed = me
            .change
            .map(|c| c.t > 0.4 || (c.bypass && c.wait <= 0.0))
            .unwrap_or(false);
        let plan: Vec<usize> = me.upcoming().collect();
        let mut best = if committed {
            None
        } else {
            self.obstacle_from(i, me.lane, me.s, Some(&plan), look, by_lane)
        };
        if let Some(c) = me.change {
            // along the way it has chosen from the new lane
            if let Some(o) =
                self.obstacle_from(i, c.to, c.s_to, Some(&me.change_plan), look, by_lane)
            {
                if best.map(|b| o.0 < b.0).unwrap_or(true) {
                    best = Some(o);
                }
            }
        }
        // Merging: a car on another lane that leads into the same lane as the next one of
        // ours, and is nearer to that joint, goes first; this car keeps behind it as if it
        // were already ahead in its own lane. (A left turn and the straight lane beside it
        // end in the same exit; taking the turn at a sensible speed, a car used to be run
        // through by the one going straight.)
        if !committed {
            let mut before = self.net.lanes[me.lane].length() - me.s;
            let mut from = me.lane;
            for next in me.upcoming().take(2) {
                if before > look {
                    break;
                }
                for &f in self.net.prev.get(next).map(|v| v.as_slice()).unwrap_or(&[]) {
                    // (two paths of one junction meeting: `junction_stop` sorts that out)
                    if f == from
                        || self.net.crossings[from]
                            .iter()
                            .any(|c| c.other == f && c.merge)
                    {
                        continue;
                    }
                    if before - me.front < 0.5 {
                        continue; // this car is at the joint already
                    }
                    for &(j, os, _, foreign) in by_lane.get(&f).map(|v| v.as_slice()).unwrap_or(&[])
                    {
                        let other = &self.cars[j];
                        if j == i
                            || foreign
                            || other.state.lane != f
                            || other.state.planned_next != Some(next)
                        {
                            continue;
                        }
                        let theirs = self.net.lanes[f].length() - os;
                        if theirs < -2.0 {
                            continue;
                        }
                        // who reaches the joint first goes first; a near tie goes to the one
                        // already let in (the order is kept, it does not flip frame by frame)
                        let t_me = (before - me.front).max(0.0) / me.speed.max(1.0);
                        let dist_them = (theirs - other.state.front).max(0.0);
                        let t_them = if other.yielding || other.light_hold {
                            f32::MAX
                        } else if other.state.speed < 0.5 {
                            time_to(dist_them, 0.0, other.state.accel) + other.state.reaction
                        } else {
                            dist_them / other.state.speed
                        };
                        let kept = self.cars[i].merge_after == Some(other.id);
                        // Taking turns (the zip): a car that has stood at the joint for a
                        // few seconds goes next, unless the other can no longer stop gently.
                        // By arrival alone the one standing there never came first while
                        // the other lane's queue kept rolling, and stood for minutes, the
                        // lanes behind it and the junctions they come out of filling up.
                        let my_dist = (before - me.front).max(0.0);
                        let my_clear = merge_clearance(&self.net, from, f, other.half_width, self.cars[i].half_width);
                        let their_clear = merge_clearance(&self.net, f, from, self.cars[i].half_width, other.half_width);
                        let me_zip = zip_waiting(&self.cars[i], my_dist, my_clear);
                        let them_zip = zip_waiting(other, dist_them, their_clear) && !other.yielding && !other.light_hold;
                        let can_stop = |st: &AiState, d: f32| d > st.speed * st.speed / (2.0 * st.decel.max(1.0)) + 0.5;
                        let first = if me_zip && them_zip {
                            other.stopped > self.cars[i].stopped + 0.1
                                || ((other.stopped - self.cars[i].stopped).abs() <= 0.1 && other.id < self.cars[i].id)
                        } else if them_zip && can_stop(me, my_dist) {
                            true
                        } else if me_zip && can_stop(&other.state, dist_them) {
                            false
                        } else {
                            t_them < t_me - 0.4
                                || (kept && t_them < t_me + 1.0)
                                || ((t_them - t_me).abs() <= 0.4
                                    && !kept
                                    && other.merge_after != Some(self.cars[i].id)
                                    && j < i)
                        };
                        if first {
                            // behind it at the joint; while it is not past yet, wait short of
                            // the joint, out of its way, rather than behind a car that is
                            // still beside
                            let d = (before - theirs) - other.state.rear;
                            let (d, v) = if d >= 0.0 {
                                (d, other.state.speed)
                            } else {
                                ((before - my_clear).max(0.0), 0.0)
                            };
                            if best.map(|b| d < b.0).unwrap_or(true) {
                                best = Some((d, v, j));
                            }
                        }
                    }
                }
                before += self.net.lanes[next].length();
                from = next;
            }
        }
        best.map(|(d, v, j)| {
            (
                Lead {
                    gap: d - me.front,
                    speed: v,
                    acc: if v > 0.1 { self.cars[j].state.acc } else { 0.0 },
                },
                j,
            )
        })
    }

    /// Is the stretch `s - back .. s + ahead` of `lane` free of cars (other than `i`)?
    pub fn lane_clear(
        &self,
        i: usize,
        lane: usize,
        s: f32,
        back: f32,
        ahead: f32,
        by_lane: &HashMap<usize, Vec<(usize, f32, f32, bool)>>,
    ) -> bool {
        // Parked scenery cars are not in `by_lane`. An otherwise empty lane must still
        // leave room for this vehicle before it starts moving over.
        if !parked_lane_clear(
            self.parked.get(&lane).map(Vec::as_slice).unwrap_or(&[]),
            s,
            back,
            ahead,
            self.cars[i].half_width,
        ) {
            return false;
        }
        let Some(list) = by_lane.get(&lane) else {
            return true;
        };
        !list
            .iter()
            .any(|&(j, os, _, _)| j != i && os > s - back && os < s + ahead)
    }

    /// The vehicles the AI does not drive (the player's bus, the LAN players') that stand in
    /// `lane` or on a lane just before or after it, going its way: (distance of their centre
    /// along `lane` - negative on a lane before it -, speed, half length). A lane change
    /// looks at these as at the cars there: they are not on the lanes, and a car moved over
    /// into the lane the player's bus was driving in, beside it.
    pub fn players_on(&self, lane: usize) -> Vec<(f32, f32, f32)> {
        let mut out = Vec::new();
        let Some(l) = self.net.lanes.get(lane) else { return out };
        let boxes = self.player.iter().chain(self.others_now.iter()).copied();
        for (centre, heading, half_len, half_w, speed) in boxes {
            let far = |p: DVec3| (p - centre).truncate().length() > l.length() as f64 + 200.0;
            if speed < -0.3 || (far(l.start()) && far(l.end())) {
                continue;
            }
            // (on the lane itself or on one joining it at either end: inside it by some of
            // its width, its heading within 50 degrees of the lane's)
            let near = |k: usize, off: f32| -> Option<f32> {
                let x = &self.net.lanes[k];
                let (s, d) = x.nearest_point(centre)?;
                let h = x.at(s).1 as f64;
                let turn = ((h - heading + 540.0).rem_euclid(360.0) - 180.0).abs();
                (d < (x.width * 0.5 + half_w - 0.3).max(1.2) as f64 && turn < 50.0 && s > 0.01 && s < x.length() - 0.01).then_some(s + off)
            };
            let at = near(lane, 0.0)
                .or_else(|| self.net.prev.get(lane).into_iter().flatten().find_map(|&p| near(p, -self.net.lanes[p].length())))
                .or_else(|| l.next.iter().find_map(|&n| near(n, l.length())));
            if let Some(s) = at {
                out.push((s, speed.max(0.0), half_len));
            }
        }
        out
    }

    /// May car `i` move over into `lane` at `s` as far as the vehicles the AI does not
    /// drive are concerned? They do not brake for it: nothing of theirs beside it, room
    /// ahead, and one coming up from behind far enough back to stay there without braking
    /// hard (its speed over a second and a fifth, and the closing speed at a gentle 1.5 m/s²).
    pub fn players_let_in(&self, i: usize, lane: usize, s: f32) -> bool {
        if self.player.is_none() && self.others_now.is_empty() {
            return true;
        }
        let me = &self.cars[i].state;
        self.players_on(lane).iter().all(|&(ps, pv, hl)| {
            if ps >= s {
                ps - hl - s - me.front > 3.0 + (me.speed - pv).max(0.0) * 1.5
            } else {
                s - me.rear - (ps + hl) > 3.0 + pv * 1.2 + (pv - me.speed).max(0.0).powi(2) / (2.0 * 1.5)
            }
        })
    }

    /// May car `i` move over into `lane` at `s` now? Nothing may be beside it or just ahead,
    /// and every car coming up behind must be able to stop comfortably behind it - the
    /// player's bus and the LAN players' included (`players_let_in`).
    pub fn can_merge(
        &self,
        i: usize,
        lane: usize,
        s: f32,
        by_lane: &HashMap<usize, Vec<(usize, f32, f32, bool)>>,
    ) -> bool {
        if !self.players_let_in(i, lane, s) {
            return false;
        }
        let me = &self.cars[i].state;
        if !parked_lane_clear(
            self.parked.get(&lane).map(Vec::as_slice).unwrap_or(&[]),
            s,
            me.rear + 1.0,
            me.front + 2.0 + me.speed * 1.5,
            self.cars[i].half_width,
        ) {
            return false;
        }
        // the cars on the lane, and those about to come onto it from the lanes before it (a
        // bus changing lanes just after a joint cut in front of a car still on the lane
        // before, which the target lane alone did not show)
        let on = by_lane
            .get(&lane)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .iter()
            .copied();
        let before = self
            .net
            .prev
            .get(lane)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .iter()
            .flat_map(|&p| {
                let len = self.net.lanes[p].length();
                let single = self.net.lanes[p].next.len() == 1;
                by_lane
                    .get(&p)
                    .map(|v| v.as_slice())
                    .unwrap_or(&[])
                    .iter()
                    .filter(move |e| {
                        !e.3 && (single || self.cars[e.0].state.planned_next == Some(lane))
                            && e.1 > len - 80.0
                    })
                    .map(move |&(j, os, lat, f)| (j, os - len, lat, f))
            });
        on.chain(before).all(|(j, os, _, foreign)| {
            if j == i {
                return true;
            }
            if foreign {
                return false;
            }
            let o = &self.cars[j].state;
            if os >= s {
                // ahead: room for this car, and it must not be much slower
                let gap = os - s - o.rear - me.front;
                gap > 2.0 + (me.speed - o.speed).max(0.0) * 1.5
            } else {
                // behind and standing for this car already (it keeps behind it): it lets it
                // in. Counted as in the way, the bus waiting at the end of its lane to move
                // over and the car stopped behind it for that bus waited on each other for
                // good, and the street behind with them (Spandau's Klosterstrasse).
                if o.speed < 0.3 && self.cars[j].lead_info.is_some_and(|(id, _)| id == self.cars[i].id) {
                    return true;
                }
                // behind: the other driver keeps a time gap and brakes gently
                let gap = s - os - me.rear - o.front;
                gap > 2.0
                    + o.speed * 0.8
                    + (o.speed - me.speed).max(0.0).powi(2) / (2.0 * o.decel.max(1.0))
            }
        })
    }

    /// A timetable vehicle whose route moves over to the next lane: signal, and move as soon
    /// as the lane is free; until then wait before the end of this one. Returns where to stop.
    pub fn plan_route_change(
        &mut self,
        i: usize,
        by_lane: &HashMap<usize, Vec<(usize, f32, f32, bool)>>,
    ) -> Option<f32> {
        let st = &self.cars[i].state;
        let (to, dir) = st.route_change_due(&self.net)?;
        let len = self.net.lanes[st.lane].length();
        let s_to = self.net.beside_s(st.lane, to, st.s);
        if self.can_merge(i, to, s_to, by_lane) {
            let net = &self.net;
            self.cars[i].state.start_route_change(net, to, dir);
            return None;
        }
        let st = &mut self.cars[i].state;
        st.signal = dir;
        st.signal_time = 1.0;
        Some(len - st.s - 1.0)
    }

    /// Is what car `i` has stopped behind going to stand there for a while (a parked car, a
    /// bus serving its stop, a broken-down or abandoned car, the player's bus waiting)?
    pub fn standing_obstacle(
        &self,
        i: usize,
        lead: Option<(Lead, Option<usize>)>,
        parked_ahead: bool,
        player_standing: f32,
    ) -> bool {
        let Some((l, who)) = lead else { return false };
        if l.speed.abs() > 0.3 {
            return false;
        }
        match who {
            Some(j) if j < self.cars.len() => {
                let o = &self.cars[j];
                // a bus at its stop, or a car that has stood for long with nothing holding
                // it (not the head of a queue that waits for a light, a junction or a car),
                // or the end of a queue standing behind a bus at its stop
                o.standing_for(self.day_time) > 4.0
                    || (o.stopped > 25.0 && !o.held && self.cars[i].stopped > 6.0)
                    || (o.stopped > 3.0 && self.standing_queue(j).1)
            }
            Some(_) => player_standing > 10.0,
            None => parked_ahead,
        }
    }

    /// The vehicles standing nose to tail from car `j` on (as far as their last steps
    /// show): the length of road they fill (m), and whether a bus serving its stop heads
    /// it - a queue that will not move for a while, which the cars behind may pass as a
    /// whole. (Behind a bus on its layover the whole street
    /// used to wait, five buses and a dozen cars for a quarter of an hour.)
    pub fn standing_queue(&self, j: usize) -> (f32, bool) {
        let mut k = j;
        let mut len = self.cars[j].state.front + self.cars[j].state.rear;
        let mut long = self.cars[j].standing_for(self.day_time) > 4.0;
        for _ in 0..8 {
            if long {
                break;
            }
            let Some((id, gap)) = self.cars[k].lead_info else {
                break;
            };
            let Some(&n) = self.index_of.get(&id) else {
                break;
            };
            let o = &self.cars[n];
            if gap > 8.0 || o.state.speed > 0.3 || o.light_hold || o.yielding {
                break;
            }
            len += gap.max(0.0) + o.state.front + o.state.rear;
            long = o.standing_for(self.day_time) > 4.0;
            k = n;
        }
        (len, long)
    }

    /// Pull out round something that has stopped in front (a bus at its stop, a car that
    /// gave up, the player standing in the lane): a car held for a few seconds behind a
    /// standing obstacle within 25 m moves to a free neighbouring lane - left first, then
    /// right. With nowhere to go it waits, like everybody else in a jam.
    pub fn plan_bypass(
        &mut self,
        i: usize,
        gap: Option<f32>,
        standing: bool,
        by_lane: &HashMap<usize, Vec<(usize, f32, f32, bool)>>,
    ) {
        let st = &self.cars[i].state;
        let stuck = self.cars[i].stopped;
        if stuck > 20.0 && standing && omsi_cfg::flags::OMSI_DEBUG_STUCK.is_set() && (self.time * 0.2).fract() < 0.01 {
            let lane = &self.net.lanes[st.lane];
            log::info!("t={:.1}: car {} behind an obstacle {:.1} m for {stuck:.0} s: change {:?} route {} cooldown {:.1} light {} yielding {} left {:?} right {:?} left clear {:?}", self.time, self.cars[i].id, gap.unwrap_or(-1.0), st.change.map(|c| (c.to, c.t, c.wait, c.bypass, c.length)), st.route.len(), st.change_cooldown, self.cars[i].light_hold, self.cars[i].yielding, lane.left, lane.right, lane.left.map(|l| { let s_side = st.s / lane.length().max(1.0) * self.net.lanes[l].length(); (self.open_to(i, l), self.lane_clear(i, l, s_side, 12.0, 30.0, by_lane), self.can_merge(i, l, s_side, by_lane)) }));
        }
        let st = &self.cars[i].state;
        if st.change.is_some()
            || !st.route.is_empty()
            || st.change_cooldown > 0.0
            || stuck < 4.0
            || !standing
            || self.cars[i].light_hold
            || self.cars[i].yielding
        {
            return;
        }
        let Some(d) = gap else { return };
        if d > 25.0 {
            return;
        }
        let lane = &self.net.lanes[st.lane];
        // (a move not over by the end of the lane carries on across the joint, `AiState::drive`:
        // it used to be started only 20 m and more before a lane's end, with the lane beside
        // running on for 30 m by itself - on a road built of 35-40 m spline pieces, Spandau's
        // Falkenseer Chaussee with its kerb lanes lined with parked cars, a car that stopped
        // behind a parked car just past a joint stood there for good with the traffic queued
        // up behind it, the lane beside free)
        if lane.length() - st.s < 9.0 && st.planned_next.is_none() {
            return;
        }
        let frac = st.s / lane.length().max(1.0);
        let (lane_idx, planned) = (st.lane, st.planned_next);
        let turn = planned.map(|n| self.net.lanes[n].turn).unwrap_or(0);
        for (side, dir) in [(lane.left, 1), (lane.right, 2)] {
            let Some(side) = side.filter(|&l| self.open_to(i, l)) else {
                continue;
            };
            // never leave a turn lane just before the junction
            if lane.length() - st.s < 150.0 && turn != 0 && dir != turn {
                continue;
            }
            let side_lane = &self.net.lanes[side];
            let s_side = frac * side_lane.length();
            // the lane beside runs on for 30 m (through its joint)
            let side_on = side_lane.length() - s_side + side_lane.next.iter().map(|&n| self.net.lanes[n].length()).fold(0.0, f32::max);
            if side_on > 30.0
                && self.lane_clear(i, side, s_side, 12.0, 30.0, by_lane)
                && self.can_merge(i, side, s_side, by_lane)
            {
                let net = &self.net;
                self.cars[i].state.start_bypass(net, side, dir);
                self.cars[i].stopped = 0.0;
                if omsi_cfg::flags::OMSI_DEBUG_TRAFFIC.is_set() {
                    log::info!("t={:.1}: car {} pulls out round an obstacle {d:.0} m ahead after {stuck:.0} s: lane {} -> {}", self.time, self.cars[i].id, lane_idx, side);
                }
                return;
            }
        }
    }

    /// On a road with one lane each way: pull out onto the other half round something
    /// standing in the lane, when the oncoming traffic leaves time enough and the car's own
    /// steering gets its front corner past the obstacle's (`AiBody::sweep_clearance`).
    /// `lead` is what it stands behind (`Some(usize::MAX)` the player's bus, `None` a
    /// parked car at `parked_box`).
    #[allow(clippy::too_many_arguments)]
    pub fn plan_pass(
        &mut self,
        i: usize,
        lead: Option<(Lead, Option<usize>)>,
        obstacle_len: f32,
        standing: bool,
        parked: bool,
        way: &[(usize, f32)],
        by_lane: &HashMap<usize, Vec<(usize, f32, f32, bool)>>,
        player: Option<PlayerBox>,
        parked_box: Option<Obb>,
        feet: &[Footprint],
    ) {
        let car = &self.cars[i];
        // Rail vehicles must never use the road-vehicle passing manoeuvre.
        if car.is_rail() {
            return;
        }
        let st = &car.state;
        // a parked car is known from afar: the driver pulls out while still rolling up to
        // it; anything else is waited behind for a moment first
        let rolling = parked && st.speed > 0.5;
        if car.passing.is_some()
            || st.change.is_some()
            || (car.stopped < 3.0 && !rolling)
            || !standing
            || car.yielding
            || car.at_stop()
            || self.time < car.pass_retry
        {
            return;
        }
        let Some((lead, who)) = lead else { return };
        let gap = lead.gap;
        // (`OMSI_DEBUG_PASS`: why a car standing behind something does not go round it,
        // twice a second)
        let debug_pass = omsi_cfg::flags::OMSI_DEBUG_PASS.is_set()
            && (self.time * 2.0).floor() != ((self.time - self.last_dt) * 2.0).floor();
        let (id_dbg, t_dbg) = (car.id, self.time);
        let skip = |why: String| {
            if debug_pass {
                log::info!("t={t_dbg:.1}: car {id_dbg} does not pass: {why}");
            }
        };
        // a timetable bus does not pass what stands at its own next stop: it queues for it
        if let Some((ri, ss)) = car.next_stop() {
            if ri >= st.route_index
                && st.route_distance(&self.net, ri, ss) < gap + obstacle_len + st.front + 15.0
            {
                return;
            }
        }
        // the distance from the front bumper to the obstacle's body (the player's box is seen
        // a little long, a parked car's gap two metres short)
        let real = match who {
            Some(usize::MAX) => gap + PLAYER_BOX_MARGIN,
            None => gap + 2.0,
            _ => gap,
        };
        let reach = if rolling {
            (st.speed * st.speed / (2.0 * st.decel) + 12.0).clamp(15.0, 40.0)
        } else {
            22.0 + (car.pass_room - 4.0).max(0.0)
        };
        // (a car still half out from a pass it gave up may start again from there)
        // (lateral towards the oncoming side: a car still half out, or pulled the other way)
        let outward = st.lateral * self.net.oncoming_sign();
        if real > reach || real < 0.3 || outward < -0.5 || outward > 1.6 {
            skip(format!(
                "gap {real:.2} (reach {reach:.1}), lateral {:.2}",
                st.lateral
            ));
            return;
        }
        let lane = &self.net.lanes[st.lane];
        if lane.left.is_some() || lane.right.is_some() {
            return; // a road with lanes to change to: `plan_bypass`
        }
        // the manoeuvre happens before the next junction, or reaches only its mouth
        let pass_len = real + obstacle_len + st.front + st.rear + 6.0;
        // and something that stands just before a light or a junction is waiting there
        let waits_there = way.iter().skip(1).any(|w| {
            (self.net.lanes[w.0].traffic_light.is_some() || !self.net.crossings[w.0].is_empty())
                && w.1 < real + st.front + obstacle_len + 25.0
        });
        if waits_there && !parked {
            skip("it waits before a light or a junction".into());
            return;
        }
        // nor where the road ends (the queue at the end of the network only waits to be taken
        // away: a car that went round it drove into its head)
        if let Some(&(last, d)) = way.last() {
            if self.net.lanes[last].next.is_empty()
                && d + self.net.lanes[last].length() < pass_len + real + 30.0
            {
                skip("the road ends".into());
                return;
            }
        }
        let junction = way
            .iter()
            .skip(1)
            .find(|w| !self.net.crossings[w.0].is_empty())
            .map(|w| w.1)
            .unwrap_or(f32::MAX);
        let open_road: f32 = way
            .iter()
            .take_while(|w| self.net.crossings[w.0].is_empty())
            .map(|w| w.1 + self.net.lanes[w.0].length())
            .fold(0.0, f32::max);
        if open_road.min(junction) < pass_len - if parked { 6.0 } else { -8.0 } {
            skip(format!(
                "a junction in {:.0} m, the pass takes {pass_len:.0} m",
                open_road.min(junction)
            ));
            return;
        }
        let Some((opp, os, side)) = self.net.opposite(st.lane, st.s) else {
            skip("no oncoming lane".into());
            return;
        };
        if !(2.3..=5.5).contains(&side) {
            skip(format!("the oncoming lane is {side:.1} m over"));
            return;
        }
        // back in: 2 m past the obstacle
        let until_d = real + obstacle_len + st.front + st.rear + 2.0;
        // Room to get back in, and to stop there if need be: nothing standing (or stopping)
        // just past the obstacle, no red light close after it. (A car that had passed at speed
        // came back in behind one braking for a red light and braked at 4.5 m/s².)
        let merge_at = st.front + real + obstacle_len;
        let v_cap =
            ((lane.speed_limit_kmh * st.desire).min(st.max_speed_kmh) / 3.6).clamp(4.0, 14.0);
        let v_back = (st.speed * st.speed + 2.0 * st.accel * 0.85 * until_d)
            .sqrt()
            .min(v_cap);
        // (the shortest S-curve back in: three and a half times the offset)
        let back_min = back_in_ramp(side, 0.0, BACK_IN_LAT_ACCEL);
        let need_room =
            (st.length + 4.0 + v_back * v_back / (2.0 * st.decel.max(1.0))).max(back_min + 2.0);
        let mut merge_room = f32::MAX;
        if let Some(l) = car.light_at.filter(|_| car.light_hold) {
            merge_room = l - merge_at - 0.6;
        }
        if let Some(k) = way.iter().rposition(|w| w.1 <= merge_at) {
            let (l, d) = way[k];
            let rest: Vec<usize> = way.iter().skip(k + 1).map(|w| w.0).collect();
            if let Some((ahead, v, j)) = self.obstacle_from(
                i,
                l,
                merge_at - d,
                Some(rest.as_slice()),
                need_room + 20.0,
                by_lane,
            ) {
                let acc = self.cars[j].state.acc;
                if v < 3.0 {
                    merge_room = merge_room.min(ahead);
                } else if acc < -1.0 || self.cars[j].light_hold {
                    // where it will come to a stop
                    merge_room = merge_room
                        .min(ahead + v * v / (2.0 * (-acc).max(self.cars[j].state.decel.max(1.0))));
                }
            }
        }
        if merge_room < need_room {
            skip(format!(
                "no room to get back in ({merge_room:.1} m, needs {need_room:.1} m)"
            ));
            return;
        }
        // back in along an S-curve as long as the speed it will have there asks for, within
        // the room there is (with something standing ahead the car is slowing down anyway)
        let back = back_in_ramp(side, v_back, st.lat_accel.min(BACK_IN_LAT_ACCEL))
            .min((merge_room - 2.0).max(back_min));
        let probe = Passing {
            lane: opp,
            side,
            until: until_d,
            block: real,
            back,
            aborted: false,
            hold: 0.0,
            creep: !rolling,
        };
        let clear_d = probe.clear_at(car.half_width);
        // Time out there: until the car is back far enough to be out of the oncoming
        // traffic's way. Nobody coming may get to where its front will be by then - on the
        // oncoming lane or on the lanes that feed it, back through the junctions beyond
        // (a car that came through the junction ahead used to meet the passer head-on).
        let t_need = pass_time(
            clear_d,
            if rolling { 0.0 } else { real + CREEP_PAST },
            st,
            v_cap,
        ) + 1.5;
        let from = os - st.front - clear_d - 2.0;
        let to = os + st.rear + 8.0;
        let parked_clear = self
            .parked
            .get(&opp)
            .map(|l| {
                l.iter()
                    .all(|&(ps, lat)| lat.abs() > 1.6 || ps < from || ps > to)
            })
            .unwrap_or(true);
        if !parked_clear {
            skip("a parked car on the oncoming lane".into());
            return;
        }
        if let Some((who_opp, t)) = self.oncoming_block(i, opp, from, to, t_need, true, by_lane) {
            skip(format!("car {who_opp} on the oncoming side is there in {t:.1} s, the pass needs {t_need:.1} s"));
            return;
        }
        let debug = omsi_cfg::flags::OMSI_DEBUG_TRAFFIC.is_set();
        let Some(ramp) = self.pull_out_ramp(i, who, real, rolling, side, player, parked_box, feet, debug) else {
            return;
        };
        let car = &self.cars[i];
        let st = &car.state;
        let id = car.id;
        let odo = st.odometer;
        let out = self.net.oncoming_sign();
        let car = &mut self.cars[i];
        car.passing = Some(Passing {
            lane: opp,
            side,
            until: odo + until_d,
            block: odo + real,
            back,
            aborted: false,
            hold: 0.0,
            creep: !rolling,
        });
        car.state.lateral_target = side * out;
        // pull out over what room there is (from a standstill a car turns out steeply)
        let lat0 = car.state.lateral;
        car.state.lateral_ramp = (lat0, side * out, odo, ramp);
        car.stopped = 0.0;
        if self.first_passer.is_none() {
            self.first_passer = Some((id, self.time));
        }
        if debug {
            log::info!("t={:.1}: car {id} passes a standing obstacle {real:.2} m ahead on the oncoming lane {opp} ({side:.1} m to the left, S-curve {ramp:.1} m, {t_need:.1} s out there)", self.time);
        }
        if omsi_cfg::flags::OMSI_DEBUG_PASS.is_set() {
            // what it saw coming on the oncoming side
            let lanes = self.net.upstream(opp, from, 150.0, 48);
            let seen: Vec<String> = lanes
                .iter()
                .flat_map(|&(l, off, _)| {
                    by_lane
                        .get(&l)
                        .into_iter()
                        .flatten()
                        .map(move |e| (l, off, *e))
                })
                .map(|(l, off, (j, sj, _, out))| {
                    format!(
                        "car {} on lane {l} at {:.1} m, {:.1} m/s{}",
                        self.cars[j].id,
                        sj + off,
                        self.cars[j].state.speed,
                        if out { " (out passing)" } else { "" }
                    )
                })
                .collect();
            log::info!(
                "  (its stretch {from:.1}..{to:.1} of lane {opp}; lanes before it {:?}; {:?})",
                lanes.iter().map(|e| (e.0, e.1.round())).collect::<Vec<_>>(),
                seen
            );
        }
    }

    /// `plan_pass`: the S-curve along which car `i` can steer out round what it stands
    /// behind (`real` m ahead) onto the oncoming lane `side` m over, None when none clears.
    #[allow(clippy::too_many_arguments)]
    fn pull_out_ramp(
        &mut self,
        i: usize,
        who: Option<usize>,
        real: f32,
        rolling: bool,
        side: f32,
        player: Option<PlayerBox>,
        parked_box: Option<Obb>,
        feet: &[Footprint],
        debug: bool,
    ) -> Option<f32> {
        let car = &self.cars[i];
        let st = &car.state;
        // Can it steer out round the corner from where it stands? The body is driven along
        // each S-curve in turn (its own wheelbase, lock and steering rate) against the
        // obstacle's box; the gentlest that clears is taken.
        let obstacles: Vec<Obb> = match who {
            Some(usize::MAX) => player
                .map(|(c, h, hl, hw, _)| {
                    vec![Obb::vehicle(
                        c.truncate(),
                        h,
                        hl as f64,
                        hl as f64,
                        hw as f64,
                    )]
                })
                .unwrap_or_default(),
            Some(j) => feet
                .iter()
                .filter(|f| f.car == j)
                .map(|f| f.obb())
                .collect(),
            None => parked_box.into_iter().collect(),
        };
        let extent = (st.front, st.rear, car.half_width);
        let (v_max, accel) = if rolling {
            (st.speed.max(4.0), 0.3)
        } else {
            (8.0, st.accel.min(PULL_OUT_ACCEL))
        };
        // (after half a minute of waiting a driver squeezes out with less to spare)
        let need = if car.stopped > 30.0 {
            0.05
        } else {
            PULL_OUT_CLEARANCE
        };
        let mut chosen = None;
        let mut best_seen = f64::MIN;
        let mut tried: Vec<String> = Vec::new();
        for ramp in pull_out_ramps(real, st.front, rolling) {
            let mut hyp = st.clone();
            hyp.lateral_target = side * self.net.oncoming_sign();
            hyp.lateral_ramp = (st.lateral, side * self.net.oncoming_sign(), st.odometer, ramp);
            // the checks that go by the lanes must let it go along that way too
            if let (Some(usize::MAX), Some(p)) = (who, player.as_ref()) {
                if let Some(l) = self.player_on_way(&hyp, car.half_width, p, 0.0) {
                    if debug {
                        tried.push(format!("{ramp:.1}: into the bus box at {:.1}", l.gap));
                    }
                    continue;
                }
            }
            if obstacles.is_empty() {
                // (nothing to measure against: the old rule of thumb)
                if real >= if rolling { 1.5 } else { 3.0 } {
                    chosen = Some(ramp);
                }
                break;
            }
            let c = car.body.sweep_clearance(
                &|d| hyp.way_point(&self.net, d),
                st.speed,
                st.reaction,
                accel,
                v_max,
                real + 6.0,
                extent,
                &obstacles,
            );
            best_seen = best_seen.max(c);
            if debug {
                tried.push(format!("{ramp:.1}: {c:.2}"));
            }
            if c >= need {
                chosen = Some(ramp);
                break;
            }
        }
        let Some(ramp) = chosen else {
            if debug {
                // where the obstacle's box is from the car (right, ahead, turned by)
                let rel = obstacles.first().map(|o| {
                    let h = car.vehicle.heading.to_radians();
                    let d = o.center - car.vehicle.position.truncate();
                    let turned = (o.heading.to_degrees() - car.vehicle.heading + 540.0).rem_euclid(360.0) - 180.0;
                    format!("box {:+.2} m right, {:.2} m ahead, turned {turned:+.1}°, half {:.2} x {:.2}", d.x * h.cos() - d.y * h.sin(), d.x * h.sin() + d.y * h.cos(), o.half.x, o.half.y)
                });
                log::info!("t={:.1}: car {} cannot steer out round the obstacle {real:.2} m ahead (best clearance {best_seen:.2} m, room {:.2} m, oncoming lane {side:.2} m over, lateral {:.2}): {}; S-curves {}", self.time, car.id, car.pass_room, st.lateral, rel.unwrap_or_default(), tried.join(", "));
            }
            self.cars[i].pass_retry = self.time + 1.0;
            return None;
        };
        Some(ramp)
    }

    /// Who on the oncoming side comes too soon for a car that will be out on lane `opp`
    /// between distances `from` and `to` (that lane's own) for `t_need` seconds: anyone in
    /// that stretch now, or anyone on the lane or on the lanes that lead into it - back
    /// over joints and through junctions, as far as the fastest of them gets in that time -
    /// whose front can get to `from` sooner. `strict`: a moving car may speed up to the
    /// limit (before starting a pass); otherwise it keeps its speed (while out there). A
    /// car waiting at a red light comes once its light changes, one giving way after its
    /// reaction time. Returns (its id, seconds until it is there).
    #[allow(clippy::too_many_arguments)]
    pub fn oncoming_block(
        &self,
        i: usize,
        opp: usize,
        from: f32,
        to: f32,
        t_need: f32,
        strict: bool,
        by_lane: &HashMap<usize, Vec<(usize, f32, f32, bool)>>,
    ) -> Option<(u64, f32)> {
        let limit = self.net.lanes[opp].speed_limit_kmh / 3.6;
        let look = (limit.clamp(8.0, 20.0) * (t_need + 1.0) + 20.0).min(300.0);
        let lanes = self.net.upstream(opp, from, look, 48);
        for &(l, off, into) in &lanes {
            let Some(list) = by_lane.get(&l) else {
                continue;
            };
            for &(j, s_j, _, foreign) in list {
                if j == i {
                    continue;
                }
                let o = &self.cars[j];
                let c = s_j + off;
                if foreign {
                    // someone from this side out on this lane round something: it is ahead and
                    // going the same way, and one may follow it out while it keeps going; not
                    // while it is slow or gave up in the stretch
                    let going =
                        o.state.speed > 2.0 && o.passing.map(|p| !p.aborted).unwrap_or(false);
                    if l == opp && c > from - 2.0 && c < to && !going {
                        return Some((o.id, 0.0));
                    }
                    continue;
                }
                // only those whose way leads on towards the stretch (a lane can lead there
                // more than one way)
                if into.is_some() && o.state.lane == l && o.state.change.is_none() {
                    if let Some(p) = o.state.planned_next {
                        if !lanes.iter().any(|e| e.0 == p) {
                            continue;
                        }
                    }
                }
                if c - o.state.rear > to {
                    continue; // past it already
                }
                let front = c + o.state.front;
                if front > from {
                    return Some((o.id, 0.0));
                }
                let dist = from - front;
                if dist > look {
                    continue;
                }
                let v = o.state.speed;
                let v_max = ((self.net.lanes[l]
                    .speed_limit_kmh
                    .max(self.net.lanes[opp].speed_limit_kmh)
                    * o.state.desire)
                    .min(o.state.max_speed_kmh)
                    / 3.6)
                    .max(v);
                let go = |d: f32| {
                    if strict || v < 0.3 {
                        arrival_time(d, v, o.state.accel, v_max)
                    } else {
                        d / v
                    }
                };
                // A red light between the car and the stretch holds it until it changes (one
                // further on does not: the car drives through the stretch first); a car that
                // gets to its light only after it has changed does not stop at all.
                let light = o
                    .light_at
                    .filter(|_| o.light_hold)
                    .map(|l| l - o.state.front)
                    .filter(|&l| l <= dist + 0.5);
                let t = match light {
                    Some(l) => {
                        let change = self.light_wait(j);
                        if go(l.max(0.0)) >= change {
                            go(dist)
                        } else {
                            change
                                + o.state.reaction
                                + arrival_time(dist - l.max(0.0), 0.0, o.state.accel, v_max)
                        }
                    }
                    None if v < 0.3 => o.state.reaction + go(dist),
                    None => go(dist),
                };
                if t < t_need {
                    return Some((o.id, t));
                }
            }
        }
        None
    }

    /// A car out on the oncoming lane round something: if somebody is coming who will be
    /// where its front is headed before it is back out of their way, it gives up while it
    /// still can - back into its lane, stopping short of what it was going round - and
    /// otherwise finishes, with the oncoming traffic stopping short of where it moves back
    /// in (`Traffic::tick` puts it there on their lane).
    pub fn guard_pass(&mut self, i: usize, by_lane: &HashMap<usize, Vec<(usize, f32, f32, bool)>>) {
        let car = &self.cars[i];
        let Some(p) = car.passing else { return };
        if p.aborted {
            return;
        }
        let st = &car.state;
        let r = p.clear_at(car.half_width) - st.odometer;
        if r <= 0.0 {
            return;
        }
        let Some((opp, os, _)) = self.net.opposite(st.lane, st.s) else {
            return;
        };
        let lane = &self.net.lanes[st.lane];
        let v_cap =
            ((lane.speed_limit_kmh * st.desire).min(st.max_speed_kmh) / 3.6).clamp(4.0, 14.0);
        let creep = if p.creep {
            (p.block + CREEP_PAST - st.odometer).max(0.0)
        } else {
            0.0
        };
        let t_me = pass_time(r, creep, st, v_cap);
        let from = os - st.front - r - 1.0;
        let to = os + st.rear;
        let Some((who, t)) = self.oncoming_block(i, opp, from, to, t_me + 0.5, false, by_lane)
        else {
            return;
        };
        // still in its lane far enough for the oncoming car to get by, and able to stop
        // before the obstacle (a car of two and a half metres needs that much of its lane)
        let v = st.speed;
        let stop_d = v * v / (2.0 * 3.5);
        let shallow = st.lateral * self.net.oncoming_sign() < p.side - car.half_width - 1.45;
        let abortable = shallow && st.odometer + stop_d + 0.4 < p.block;
        let id = car.id;
        let debug = omsi_cfg::flags::OMSI_DEBUG_TRAFFIC.is_set();
        if !abortable {
            if debug && (self.time * 2.0).floor() != ((self.time - self.last_dt) * 2.0).floor() {
                log::info!("t={:.1}: car {id} is out passing with car {who} coming in {t:.1} s, {r:.1} m to go ({t_me:.1} s): finishing", self.time);
            }
            return;
        }
        let odo = st.odometer;
        let lat = st.lateral;
        let car = &mut self.cars[i];
        let mut p = p;
        p.aborted = true;
        p.until = odo;
        // (where it would have waited anyway, if it can stop there)
        p.hold = (p.block - car.pass_room).max(odo + stop_d);
        car.passing = Some(p);
        car.state.lateral_target = 0.0;
        car.state.lateral_ramp = (lat, 0.0, odo, (p.block - odo - 0.5).clamp(2.0, 8.0));
        if debug {
            log::info!("t={:.1}: car {id} gives up passing: car {who} comes in {t:.1} s, it needed {t_me:.1} s more ({r:.1} m); stops within {stop_d:.1} m", self.time);
        }
    }

    /// Overtaking and keeping right: start a lane change when it is safe.
    pub fn plan_lane_change(
        &mut self,
        i: usize,
        by_lane: &HashMap<usize, Vec<(usize, f32, f32, bool)>>,
    ) {
        let st = &self.cars[i].state;
        if st.change.is_some()
            || !st.route.is_empty()
            || st.change_cooldown > 0.0
            || st.speed < 4.0
            || self.cars[i].passing.is_some()
        {
            return;
        }
        let lane = &self.net.lanes[st.lane];
        // not shortly before the end of the lane (junctions): the indicator, the move and a
        // little margin must fit into what is left of it
        if lane.length() - st.s < (st.speed * 5.5 + 10.0).max(40.0) {
            return;
        }
        let limit = (lane.speed_limit_kmh * st.desire).min(st.max_speed_kmh) / 3.6;
        let frac = st.s / lane.length().max(1.0);
        let (lane_idx, s, planned) = (st.lane, st.s, st.planned_next);
        // turn lanes: within 150 m of the junction get into the lane for the way out
        // (the crossing `[path]` carries the turn: 1 left, 2 right)
        let to_junction = lane.length() - s;
        let turn = planned.map(|n| self.net.lanes[n].turn).unwrap_or(0);
        if to_junction < 150.0 && turn != 0 {
            let want = if turn == 1 { lane.left } else { lane.right };
            if let Some(side) = want.filter(|&l| self.open_to(i, l)) {
                // only if that lane reaches a way out with the same turn
                let ok = self.net.lanes[side]
                    .next
                    .iter()
                    .any(|&n| self.net.lanes[n].turn == turn);
                let s_side = frac * self.net.lanes[side].length();
                if ok && self.can_merge(i, side, s_side, by_lane) {
                    let net = &self.net;
                    self.cars[i].state.turn_wish = turn;
                    self.cars[i].state.start_change(net, side, turn);
                    if omsi_cfg::flags::OMSI_DEBUG_TRAFFIC.is_set() {
                        log::info!("t={:.1}: car {} takes the {} turn lane: {} -> {} ({:.0} m to the junction)", self.time, self.cars[i].id, if turn == 1 { "left" } else { "right" }, lane_idx, side, to_junction);
                    }
                    return;
                }
            }
        }
        // never wander out of a turn lane shortly before the junction
        if to_junction < 150.0 && turn != 0 {
            return;
        }
        // a slow car ahead and a free lane on the left: overtake
        // (on a left-hand-traffic map passing is on the right and the keeping to the left)
        let lht = self.net.left_hand;
        let (pass_side, keep_side, pass_dir, keep_dir) = if lht { (lane.right, lane.left, 2, 1) } else { (lane.left, lane.right, 1, 2) };
        if let Some(left) = pass_side.filter(|&l| self.open_to(i, l)) {
            let plan: Vec<usize> = self.cars[i].state.upcoming().collect();
            if let Some((d, v, _)) = self.obstacle_from(i, lane_idx, s, Some(&plan), 45.0, by_lane)
            {
                let s_left = frac * self.net.lanes[left].length();
                if v < limit * 0.7
                    && v < st.speed + 1.0
                    && d > 8.0
                    && self.lane_clear(i, left, s_left, 20.0, 50.0, by_lane)
                    && self.can_merge(i, left, s_left, by_lane)
                {
                    let net = &self.net;
                    self.cars[i].state.start_change(net, left, pass_dir);
                    if self.last_overtaker.is_none() {
                        self.last_overtaker = Some((self.cars[i].id, self.time));
                    }
                    if omsi_cfg::flags::OMSI_DEBUG_TRAFFIC.is_set() {
                        log::info!("t={:.1}: car {} overtakes: obstacle {d:.0} m at {:.0} km/h, lane {} -> {} ({} {:?}) at ({:.1}, {:.1})", self.time, self.cars[i].id, v * 3.6, lane_idx, left, self.net.lanes[lane_idx].name, self.net.lanes[lane_idx].key, self.cars[i].vehicle.position.x, self.cars[i].vehicle.position.y);
                    }
                    return;
                }
            }
        }
        // keep right when the right lane is free (not into a parking lane: the outer lanes
        // of Spandau's six-lane roads carry `[rule] trafficdensity 0` and the parked cars)
        if let Some(right) = keep_side.filter(|&l| self.open_to(i, l)) {
            let s_right = frac * self.net.lanes[right].length();
            if self.lane_clear(i, right, s_right, 30.0, 70.0, by_lane)
                && self.can_merge(i, right, s_right, by_lane)
                && self.net.lanes[right].length() - s_right > 40.0
            {
                let net = &self.net;
                self.cars[i].state.start_change(net, right, keep_dir);
                if omsi_cfg::flags::OMSI_DEBUG_TRAFFIC.is_set() {
                    log::info!(
                        "t={:.1}: car {} keeps right: lane {} -> {} at ({:.1}, {:.1})",
                        self.time,
                        self.cars[i].id,
                        lane_idx,
                        right,
                        self.cars[i].vehicle.position.x,
                        self.cars[i].vehicle.position.y
                    );
                }
            }
        }
    }

    /// May random traffic drive on `lane` at all (`[rule] no_cars`, `trafficdensity 0`)?
    /// Whether car `i` may change onto `lane`: open to cars, open to its own traffic group
    /// (`[rule]` densities per group: a path open to bicycles only counted as open to
    /// everybody, and the trucks of Vlietlanden changed onto the cycle paths beside the
    /// road, #327) and to its `[ai_veh_type]` (`Lane::allows`).
    pub fn open_to(&self, i: usize, lane: usize) -> bool {
        let Some(l) = self.net.lanes.get(lane) else { return false };
        lane_open_to(l, &self.cars[i].state)
    }
}

/// The same vehicle/group gates used by route planning also apply to lane changes.
pub fn lane_open_to(lane: &crate::traffic::Lane, state: &AiState) -> bool {
    lane.allows(state.veh_type)
        && match state.traffic_pool.as_ref() {
            Some((pool, defaults)) => lane.pool_density(defaults, *pool) > 0.0,
            None => lane.density > 0.0,
        }
}
