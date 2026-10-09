//! Right of way at junctions: what a car crosses on its way, who comes there, and
//! who goes first (the player's bus and the LAN players' vehicles included).

use super::*;

/// Seconds a car waits at a junction's line before it keeps a claim on its way through
/// while waiting (see `TrafficSim::junction_stop`). (45 s once: a side road's queue had
/// grown long behind the driver by then, and drivers make themselves seen sooner.)
pub const LONG_WAIT_CLAIM: f32 = 25.0;
/// Seconds a car held only by a full exit waits before it squeezes in (see
/// `TrafficSim::junction_stop`).
pub const GRIDLOCK_WAIT: f32 = 45.0;

/// What `Traffic::weigh_crossings` found at a junction: somebody physically in the way
/// (`hard`), only the rules in the way (`ruled`), the cars this one waits for that wait
/// themselves (`soft`), why (for the debug output), where to stop, and whether anybody is
/// near the crossing lanes at all (`contested`).
pub struct Weighing {
    hard: bool,
    ruled: bool,
    soft: Vec<usize>,
    why: Vec<String>,
    stop_at: Option<f32>,
    contested: bool,
    /// The first vehicle (by id) found in the way or with the right of way.
    by: Option<u64>,
    /// Somebody who has waited long at the line pulls out across this car's way.
    courtesy: bool,
}

/// Where a vehicle meets a crossing lane on its way: its lane in the sequence, the distance
/// from its origin to that lane's start.
#[derive(Debug, Clone)]
pub struct Junction {
    /// (lane, distance from the car's origin to its start) of the junction's lanes on the
    /// car's way; the first is where it has to wait.
    pub lanes: Vec<(usize, f32)>,
    /// The lane after the junction and the distance to its start.
    pub exit: Option<(usize, f32)>,
    /// The car is already on one of the junction's lanes.
    pub inside: bool,
}

/// A vehicle the AI does not drive (the player's bus, a LAN player's) as the right of way
/// sees it: Omsi.exe keeps the player's vehicle on the paths like any AI vehicle, so the
/// cars give way to it by the same rules; here it is put onto the lanes where it is.
#[derive(Debug, Clone)]
pub struct WayUser {
    /// The lane it is on and those it may take next: (lane, distance from its centre to
    /// the lane's start - negative for the lane it is on).
    pub lanes: Vec<(usize, f32)>,
    /// Speed along its way (m/s), half its length (m), how long it has stood (s).
    pub speed: f32,
    pub half_len: f32,
    pub still: f32,
    /// Its script claims priority (`TrafficPriority`).
    pub prio: bool,
}

/// Seconds until a vehicle `dist` metres from a point gets its front there, from speed `v`
/// with acceleration `a`.
pub fn time_to(dist: f32, v: f32, a: f32) -> f32 {
    if dist <= 0.0 {
        return 0.0;
    }
    let a = a.max(0.3);
    // v t + a t² / 2 = dist
    (-v + (v * v + 2.0 * a * dist).sqrt()) / a
}

pub fn crossing_arrival(st: &AiState, distance: f32, claimed: bool, waits_short: bool, stalled: bool) -> f32 {
    if distance <= 0.3 {
        return 0.0;
    }
    if claimed {
        return time_to(distance, st.speed, st.accel)
            + if st.speed < 0.1 { st.reaction } else { 0.0 };
    }
    if waits_short {
        return f32::MAX;
    }
    // A queue cannot accelerate freely. Keep its actual movement in the prediction:
    // ignoring a crawling car altogether would let another drive into its path.
    if stalled {
        return if st.speed > 0.0 { distance / st.speed } else { f32::MAX };
    }
    if st.speed > 0.5 {
        distance / st.speed
    } else {
        time_to(distance, 0.0, st.accel) + st.reaction
    }
}

/// Metres of room beyond what it needs that a car waiting for a full exit wants before it
/// goes (see `TrafficSim::junction_stop`).
pub const EXIT_HYSTERESIS: f32 = 3.0;

/// Comfortable braking of a vehicle on a junction's exit, for where it will have stopped
/// (m/s², see `queued_exit_vehicle`).
pub const EXIT_BRAKE: f32 = 2.0;

/// The nearest vehicle in the part of the chosen exit that must be clear for this car:
/// (the room left before its rear, its speed, its lane).
///
/// `way` distances are measured from the car's present lane origin; `rear` is measured
/// from the occupied lane's start. Combining both lets a queue be found across short,
/// consecutive path objects instead of only on the first lane after a junction.
/// A vehicle that is still moving counts where it would have stopped braking gently now
/// (its speed² / 2 `EXIT_BRAKE` further on): a queue crawling along the exit leaves no
/// more room than one standing there. Counted only once it stood (below 1.5 m/s), a queue
/// creeping on at walking pace let car after car into a junction they could not leave;
/// the queue stopped and they stood in the crossing traffic's way, which in its turn
/// queued back into the junctions before - on Spandau's big junctions the rings of cars
/// waiting on each other never cleared.
pub fn queued_exit_vehicle(
    way: &[(usize, f32)],
    exit: (usize, f32),
    need: f32,
    occupied: impl IntoIterator<Item = (usize, f32, f32)>,
) -> Option<(f32, f32, usize)> {
    let start = way
        .iter()
        .position(|&(lane, distance)| lane == exit.0 && (distance - exit.1).abs() < 0.01)?;
    occupied
        .into_iter()
        .filter_map(|(lane, rear, speed)| {
            let offset = way[start..]
                .iter()
                .take_while(|&&(_, distance)| distance - exit.1 < need)
                .find(|&&(candidate, _)| candidate == lane)
                .map(|&(_, distance)| distance - exit.1)?;
            let space = offset + rear;
            let will = space + speed.max(0.0).powi(2) / (2.0 * EXIT_BRAKE);
            (will < need).then_some((will, space, speed, lane))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, space, speed, lane)| (space, speed, lane))
}

/// A vehicle the AI does not drive, put onto the lanes for the right of way: the street
/// lane it drives along (within 3.5 m, its heading within 45 degrees) and the lanes it
/// may take on from there, as far as it gets in about six seconds. Where the way forks,
/// an indicator picks the turns that way (when the fork has one); without one every
/// branch counts - the cars cannot know where the bus is going, and the autopilot's
/// left turn without an indicator ran into a car that had taken the bus for going
/// straight on. None off the lanes or reversing.
pub fn way_user_on(net: &Network, b: &PlayerBox, blinker: u8, still: f32, prio: bool) -> Option<WayUser> {
    let (pos, heading, half_len, _, speed) = *b;
    if speed < -0.3 {
        return None;
    }
    let (lane, s, _) = net.lane_along(pos, heading, LaneKind::Street, 3.5, 45.0)?;
    let reach = (speed.max(0.0) * 6.0 + 40.0).min(LOOK_AHEAD_MAX);
    let mut lanes = vec![(lane, -s)];
    let mut open = vec![(lane, net.lanes[lane].length() - s)];
    while let Some((l, to_start)) = open.pop() {
        if to_start > reach || lanes.len() > 48 {
            continue;
        }
        let next: Vec<usize> = net.lanes[l]
            .next
            .iter()
            .copied()
            .filter(|&n| net.lanes[n].kind == LaneKind::Street && !lanes.iter().any(|o| o.0 == n))
            .collect();
        let chosen: Vec<usize> = match blinker {
            1 | 2 if next.len() > 1 && next.iter().any(|&n| net.lanes[n].turn == blinker as i32) => {
                next.into_iter().filter(|&n| net.lanes[n].turn == blinker as i32).collect()
            }
            _ => next,
        };
        for n in chosen {
            lanes.push((n, to_start));
            open.push((n, to_start + net.lanes[n].length()));
        }
    }
    Some(WayUser { lanes, speed: speed.max(0.0), half_len, still, prio })
}

/// The player's bus (or a LAN player's) coming to the joint where the car's (`me`) lane runs
/// into the one the bus is on - a side road's right turn into the main road, two lanes
/// becoming one - as `obstacle_ahead` sorts out two AI cars there: whoever gets to the
/// joint first goes first (a near tie to the bus), and the car keeps behind the bus as
/// if it were ahead in its own lane. Before, the car only saw the bus once its box was in
/// the car's way, and pulled out in front of it.
pub fn merging_lead(net: &Network, me: &AiState, users: &[WayUser]) -> Option<Lead> {
    if users.is_empty() || me.change.is_some() {
        return None;
    }
    let mut best: Option<Lead> = None;
    let mut before = net.lanes[me.lane].length() - me.s;
    let mut from = me.lane;
    for next in me.upcoming().take(2) {
        if before > LOOK_AHEAD {
            break;
        }
        if before - me.front < 0.5 {
            // at the joint already
            before += net.lanes[next].length();
            from = next;
            continue;
        }
        for &f in net.prev.get(next).map(|v| v.as_slice()).unwrap_or(&[]) {
            if f == from || net.crossings[from].iter().any(|c| c.other == f && c.merge) {
                continue;
            }
            for u in users {
                let (Some(&(_, df)), true) = (u.lanes.iter().find(|x| x.0 == f), u.lanes.iter().any(|x| x.0 == next)) else { continue };
                // its centre to the joint
                let to_joint = df + net.lanes[f].length();
                let theirs = to_joint - u.half_len;
                if to_joint < -u.half_len || (u.speed < 0.5 && u.still > 2.0) {
                    continue; // past it (then it is ahead in the lane), or standing
                }
                let t_me = (before - me.front).max(0.0) / me.speed.max(1.0);
                let t_them = if u.speed > 0.5 { theirs.max(0.0) / u.speed } else { time_to(theirs, 0.0, 1.0) + 1.0 };
                if t_them > t_me + 0.4 {
                    continue;
                }
                // behind it at the joint; while it is not past, wait at the joint itself
                let d = before - to_joint - u.half_len;
                let l = if d >= 0.0 {
                    Lead { gap: d - me.front, speed: u.speed, acc: 0.0 }
                } else {
                    Lead { gap: (before - 1.0 - me.front).max(0.0), speed: 0.0, acc: 0.0 }
                };
                if best.map(|b| l.gap < b.gap).unwrap_or(true) {
                    best = Some(l);
                }
            }
        }
        before += net.lanes[next].length();
        from = next;
    }
    best
}

/// What a vehicle the AI does not drive means for a car at a meeting place of its junction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    /// Nothing: through already, far off, standing, or the car goes first.
    Free,
    /// It is in the meeting place, or in the junction and there first: wait (a car that
    /// cannot stop any more still goes).
    Hard,
    /// It has the right of way and comes within the gap the driver would take.
    Ruled,
}

/// `Traffic::junction_stop` for one vehicle the AI does not drive (`u`, its lane's start
/// `dm` m from its centre) at the meeting place `c` of the car's junction lane, `point` m
/// from the car's origin: the rules the cars keep among themselves. `inside`: the car is
/// in the junction already; `committed`: it has claimed it; `me_prio`: it has priority by
/// its script; `must_yield`: its lane gives way to the vehicle's; `patience` shrinks the
/// gap it wants after a long wait. Also when the vehicle arrives (s) and when the car
/// would be through (s).
#[allow(clippy::too_many_arguments)]
pub fn way_user_verdict(st: &AiState, u: &WayUser, dm: f32, c: &crate::traffic::Crossing, point: f32, inside: bool, committed: bool, me_prio: bool, must_yield: bool, patience: f32) -> (Verdict, f32, f32) {
    let (v, a_me) = (st.speed, st.accel);
    // (its centre to the meeting point, and its front to the meeting place)
    let dj = dm + c.other_at;
    let t_clear = time_to(point + c.after + st.rear + 0.3, v, a_me) + if v < 0.5 { st.reaction } else { 0.0 };
    if dj + c.other_after < -u.half_len - 0.3 {
        return (Verdict::Free, f32::MAX, t_clear); // through
    }
    // (a meeting place far into the junction is weighed once the car is in it)
    if !inside && point - c.before - st.front > 25.0 {
        return (Verdict::Free, f32::MAX, t_clear);
    }
    let theirs = dj - c.other_before - u.half_len;
    let t_mine = time_to(point - c.before - st.front, v, a_me) + if v < 0.1 { st.reaction } else { 0.0 };
    if dm <= 0.0 && theirs <= 0.3 {
        // on that lane and in the meeting place: unless this car is further in already
        // (only where it is: a bus at its stop line is not on any of the ways it may take
        // yet, and counted as in all of them it held the cross traffic it waited for)
        let mine_in = st.front - (point - c.before);
        let ahead = mine_in > 0.0 && mine_in > -theirs + 0.3;
        return (if ahead { Verdict::Free } else { Verdict::Hard }, 0.0, t_clear);
    }
    // standing (at a stop, at its own line, letting this car go): not coming
    if u.speed < 0.5 && u.still > 2.0 {
        return (Verdict::Free, f32::MAX, t_clear);
    }
    let t_j = if u.speed > 0.5 { theirs.max(0.0) / u.speed } else { time_to(theirs, u.speed, 1.0) + 1.0 };
    if dm <= 0.0 && u.speed > 0.5 {
        // in the junction already and moving: the first there goes first
        let first = !(committed || inside) || t_j < t_mine - 0.3;
        let verdict = if first && t_j < t_clear + 1.0 { Verdict::Hard } else { Verdict::Free };
        return (verdict, t_j, t_clear);
    }
    if inside || committed || (me_prio && !u.prio) || (!must_yield && !(u.prio && !me_prio)) {
        return (Verdict::Free, t_j, t_clear);
    }
    // the gap a driver takes in the main road's traffic (as for the cars)
    let verdict = if t_j < (st.accept_gap + 2.0).max(t_clear + 1.0) * patience { Verdict::Ruled } else { Verdict::Free };
    (verdict, t_j, t_clear)
}

impl TrafficSim {
    /// The lanes of a car's way with their distance from its origin: the current lane (at
    /// minus `s`) and the plan, up to `within` metres.
    pub fn way_lanes(&self, st: &AiState, within: f32) -> Vec<(usize, f32)> {
        let mut out = vec![(st.lane, -st.s)];
        let mut d = self.net.lanes[st.lane].length() - st.s;
        let plan: Vec<usize> = match st.change {
            Some(c) => std::iter::once(c.to)
                .chain(st.change_plan.iter().copied())
                .collect(),
            None => st.upcoming().collect(),
        };
        if let Some(c) = st.change {
            // over on the new lane: its distances count from the same place
            out.clear();
            out.push((c.to, -c.s_to));
            d = self.net.lanes[c.to].length() - c.s_to;
            for &l in plan.iter().skip(1) {
                if d > within {
                    break;
                }
                out.push((l, d));
                d += self.net.lanes[l].length();
            }
            return out;
        }
        for l in plan {
            if d > within {
                break;
            }
            out.push((l, d));
            d += self.net.lanes[l].length();
        }
        out
    }

    /// The junction on car `i`'s way within `within` metres: its lanes that cross or meet
    /// others (or a footpath), and the lane after it.
    pub fn junction_ahead(&self, way: &[(usize, f32)]) -> Option<Junction> {
        let has = |l: usize| !self.net.crossings[l].is_empty() || !self.net.walks[l].is_empty();
        let object = |l: usize| {
            self.net.lanes[l]
                .key
                .filter(|_| self.net.lanes[l].source == 2)
                .map(|k| (k.tile, k.id))
        };
        let mut j: Option<Junction> = None;
        for (k, &(l, d)) in way.iter().enumerate() {
            match j.as_mut() {
                None => {
                    if has(l) {
                        j = Some(Junction {
                            lanes: vec![(l, d)],
                            exit: None,
                            inside: k == 0,
                        });
                    }
                }
                Some(jn) => {
                    if object(l).is_some() && object(l) == object(jn.lanes[0].0) {
                        jn.lanes.push((l, d));
                    } else {
                        jn.exit = Some((l, d));
                        break;
                    }
                }
            }
        }
        j
    }

    /// Right of way at the junction ahead of car `i`: where it has to wait (distance from
    /// its origin), or None when it may go - in which case it claims the junction's lanes.
    /// It gives way to anyone already in the junction on a crossing path, to anyone who
    /// has claimed a crossing path and arrives before it could be through, to traffic with
    /// the right of way that is close enough in time (its `accept_gap`), to pedestrians on
    /// a crossing, and it does not drive into a junction it could not leave (a queue on
    /// the exit). Cars waiting on each other all round are resolved in favour of the one
    /// that has waited longest. A driver who has decided to go keeps to it (the claim
    /// stands) unless someone is actually in the way: weighing the gap again every frame
    /// made two cars take turns at stopping and going, a hard brake every other frame.
    /// `way` is the car's own way: where its own path crosses itself nothing is to be
    /// given way to.
    #[allow(clippy::too_many_arguments)]
    pub fn junction_stop(
        &mut self,
        i: usize,
        jn: &Junction,
        way: &[(usize, f32)],
        lead: Option<Lead>,
        on_lane: &HashMap<usize, Vec<(usize, f32, f32, bool)>>,
        coming: &HashMap<usize, Vec<(usize, f32)>>,
        reservations: &mut HashMap<usize, Vec<usize>>,
        walkers: &HashMap<usize, Vec<f32>>,
    ) -> Option<f32> {
        let car = &self.cars[i];
        let st = &car.state;
        let v = st.speed;
        let entry = jn.lanes[0].1;
        let decide = (v * v / (2.0 * st.decel) + 12.0).clamp(20.0, 70.0);
        let release = |reservations: &mut HashMap<usize, Vec<usize>>, lanes: &[usize]| {
            for l in lanes {
                if let Some(list) = reservations.get_mut(l) {
                    list.retain(|&c| c != i);
                }
            }
        };
        if !jn.inside && entry - st.front > decide {
            // too far to decide; a claim made just inside that distance stands (slowing
            // down moves the line, and letting go and claiming again in turns made the
            // cross traffic stop and go with it)
            if entry - st.front > decide + 20.0 {
                let old = std::mem::take(&mut self.cars[i].reserved);
                release(reservations, &old);
            }
            self.cars[i].exit_wait = false;
            return None;
        }
        // queued behind someone who is not through the junction yet: no claim
        let queued = !jn.inside
            && lead
                .map(|l| l.speed < 1.0 && l.gap < entry - st.front + 3.0)
                .unwrap_or(false);
        // decided already (a claim from the frames before), or past the point where it could
        // still stop without an emergency brake
        let committed = car.reserved.contains(&jn.lanes[0].0);
        // (a stop line is kept 0.6 m off)
        let room = entry - st.front - 0.6;
        // (a car creeping up to its line can always stop: at the line the room is nothing,
        // and the car standing there used to count as one that could not stop any more)
        let cannot_stop = !jn.inside && v > 1.0 && room < v * v / (2.0 * MAX_BRAKE * 0.7);
        let cannot_stop_gently = !jn.inside && v > 1.0 && room < v * v / (2.0 * st.decel * 1.5);
        // (what is only courtesy - room for the car on the exit, a driver who has waited
        // long - is given only braking no harder than the driver likes)
        let cannot_stop_comfortably = !jn.inside && v > 1.0 && room < v * v / (2.0 * st.decel);
        // (nor, a queue on the exit that only will stand, to a driver who has decided and is
        // rolling off his line: stopped again for it, the car crept off and stood by turns,
        // a jolt every few seconds)
        let started = committed && v > 0.3 && room < 3.0;
        let explain = omsi_cfg::flags::OMSI_DEBUG_JUNCTION.is_set() || omsi_cfg::flags::OMSI_DEBUG_STUCK.is_set();
        let stop_at = if jn.inside { None } else { Some(entry) };
        // A driver who has waited long accepts a shorter gap (the critical gap shrinks with
        // the wait, by up to a third after forty seconds): a bus that needed twelve seconds
        // of a busy main road stood at the mouth of its side road for minutes.
        let wait = self.cars[i].state.yield_time;
        let patience = 1.0 - (wait / 40.0).min(1.0) / 3.0;
        let Weighing { hard, mut ruled, soft, mut why, stop_at, contested, by, courtesy } =
            self.weigh_crossings(i, jn, way, on_lane, coming, reservations, walkers, committed, patience, explain, stop_at);
        // keep the junction clear: the exit must take the whole car
        let ruled_before_exit = ruled;
        let mut exit_full = false;
        let mut exit_car: Option<u64> = None;
        if !jn.inside {
            if let Some(exit) = jn.exit {
                // (a car waiting for room on the exit goes once there is clearly room: with
                // the same measure both ways it crept off and stopped again at its line
                // every few seconds as the queue beyond crawled and stood by turns)
                let need = st.length + st.min_gap + if self.cars[i].exit_wait { EXIT_HYSTERESIS } else { 0.0 };
                // A map often builds the road immediately beyond a crossing from several
                // short path objects. Looking only at the first exit lane then calls the
                // exit empty while a queue stands on the next 2 m piece, and a car enters
                // the box with nowhere to put its body. Follow this car's chosen way until
                // there is enough clear road for all of it.
                let all_cars = &self.cars;
                let occupied = on_lane.iter().flat_map(|(&lane, cars)| {
                    cars.iter().filter_map(move |&(j, s, _, passing)| {
                        (!passing && j != i).then_some((
                            lane,
                            s - all_cars[j].state.rear,
                            all_cars[j].state.speed,
                        ))
                    })
                });
                if let Some((space, speed, lane)) = queued_exit_vehicle(way, exit, need, occupied) {
                    // (a car within its distance to decide, `decide`, looks across the
                    // junction however long it is: with the exit taken only within 40 m of
                    // the car, a big junction's exit came into view when the car could no
                    // longer stop for it, and it stood in the middle behind the queue)
                    // (a queue that stands near the junction, as before, at any braking a
                    // driver calls gentle; one that is about to stand, only if the car can
                    // stop for it comfortably)
                    let standing = speed < 1.5 && space < need && exit.1 < 40.0;
                    let will_stand = space + speed.powi(2) / (2.0 * EXIT_BRAKE) < need && exit.1 - entry < 60.0;
                    if standing || (will_stand && !cannot_stop_comfortably && !started) {
                        ruled = true;
                        exit_full = true;
                        // (the last car of that queue on that lane)
                        exit_car = on_lane
                            .get(&lane)
                            .and_then(|v| v.iter().filter(|e| !e.3 && e.0 != i).min_by(|a, b| (a.1 - self.cars[a.0].state.rear).total_cmp(&(b.1 - self.cars[b.0].state.rear))))
                            .map(|e| self.cars[e.0].id);
                        if explain {
                            why.push(format!("exit {} full on lane {lane} ({space:.1} m)", exit.0));
                        }
                    }
                }
            }
        }
        let mut blocked = (hard && !cannot_stop)
            || ((ruled || !soft.is_empty()) && !cannot_stop_gently)
            || (courtesy && !cannot_stop_comfortably && !started);
        // held only by a full exit for long: a ring of queues each waiting for the next
        // junction's exit (round a block) never clears by itself - squeeze in, as drivers do
        // (but only into a junction nobody else needs: stopped in it with its exit still full,
        // a car stands across the crossing traffic's way - on a big junction behind a long
        // queue the cars of every direction squeezed in after their 45 s, each standing in
        // the others' way, and the junction was locked for good, timetable buses and all)
        if blocked && exit_full && !hard && !ruled_before_exit && soft.is_empty() && wait > GRIDLOCK_WAIT && !contested {
            blocked = false;
            if omsi_cfg::flags::OMSI_DEBUG_TRAFFIC.is_set() {
                log::info!("t={:.1}: car {} squeezes into a full exit after {wait:.0} s (gridlock)", self.time, self.cars[i].id);
            }
        }
        if !hard && !ruled && !soft.is_empty() && wait > 2.5 + st.reaction {
            // everybody is waiting for somebody: the longest waiter goes
            let wins = soft.iter().all(|&j| {
                let o = &self.cars[j];
                (o.yielding || o.state.speed < 0.3)
                    && (wait > o.state.yield_time + 0.05
                        || ((wait - o.state.yield_time).abs() <= 0.05 && self.cars[i].id < o.id))
            });
            if wins {
                blocked = false;
                if omsi_cfg::flags::OMSI_DEBUG_TRAFFIC.is_set() {
                    log::info!("t={:.1}: car {} ends a wait of {wait:.1} s at a junction ({} waiting on it)", self.time, self.cars[i].id, soft.len());
                }
            }
        }
        // a ring of cars waiting on each other that this one has been chosen to break
        // (`deadlock`): it goes, as far as its body check lets it
        if blocked && self.cars[i].deadlock_pass > self.time {
            blocked = false;
        }
        let lanes: Vec<usize> = jn.lanes.iter().map(|x| x.0).collect();
        // (and every ten seconds of a long wait)
        let long_wait =
            wait > 15.0 && (wait / 10.0).floor() != ((wait - self.last_dt) / 10.0).floor();
        if explain && (blocked != self.cars[i].yielding || (blocked && long_wait)) {
            log::info!("t={:.2}: car {} at {:.1} m/s {} the junction {:?} (entry {:.1} m, inside {}): hard {hard}, waiting for {:?}, claims {:?} {:?}", self.time, self.cars[i].id, v, if blocked { "waits at" } else { "goes into" }, lanes, entry - st.front, jn.inside, soft.iter().map(|&j| self.cars[j].id).collect::<Vec<_>>(), lanes.iter().map(|l| reservations.get(l).map(|r| r.iter().map(|&j| self.cars[j].id).collect::<Vec<_>>())).collect::<Vec<_>>(), why);
        }
        if explain {
            self.cars[i].junction_why = if blocked { format!("{why:?} soft {:?}", soft.iter().map(|&j| self.cars[j].id).collect::<Vec<_>>()) } else { String::new() };
        }
        let yield_to = if blocked {
            by.or(exit_car).or_else(|| soft.first().map(|&j| self.cars[j].id))
        } else {
            None
        };
        self.cars[i].yield_to = yield_to;
        self.cars[i].exit_wait = blocked && exit_full;
        // A driver who has waited long at the line makes himself seen: he keeps a claim on
        // his way through while still waiting, so the cars not yet committed to the
        // junction hold back for him and he goes once those already on their way are
        // through. Without it a side road's car at a busy main road waited four and a half
        // minutes while every newcomer claimed the junction first. (Two such on crossing
        // ways are sorted out by the claims' order: the first there, a tie the lower number.)
        if blocked && !jn.inside && wait > LONG_WAIT_CLAIM && !queued {
            for &l in &lanes {
                let list = reservations.entry(l).or_default();
                if !list.contains(&i) {
                    list.push(i);
                }
            }
            let car = &mut self.cars[i];
            for &l in &lanes {
                if !car.reserved.contains(&l) {
                    car.reserved.push(l);
                }
            }
            return stop_at;
        }
        if blocked && !jn.inside {
            let old = std::mem::take(&mut self.cars[i].reserved);
            release(reservations, &old);
            return stop_at;
        }
        if blocked {
            return stop_at;
        }
        if queued && !jn.inside {
            let old = std::mem::take(&mut self.cars[i].reserved);
            release(reservations, &old);
            return None;
        }
        // claim the way through
        for &l in &lanes {
            let list = reservations.entry(l).or_default();
            if !list.contains(&i) {
                list.push(i);
            }
        }
        let car = &mut self.cars[i];
        for l in lanes {
            if !car.reserved.contains(&l) {
                car.reserved.push(l);
            }
        }
        None
    }

    /// `junction_stop`'s look at everybody on or coming to the lanes that cross car `i`'s
    /// way through junction `jn`, and at the people on its crossings.
    #[allow(clippy::too_many_arguments)]
    fn weigh_crossings(
        &self,
        i: usize,
        jn: &Junction,
        way: &[(usize, f32)],
        on_lane: &HashMap<usize, Vec<(usize, f32, f32, bool)>>,
        coming: &HashMap<usize, Vec<(usize, f32)>>,
        reservations: &HashMap<usize, Vec<usize>>,
        walkers: &HashMap<usize, Vec<f32>>,
        committed: bool,
        patience: f32,
        explain: bool,
        mut stop_at: Option<f32>,
    ) -> Weighing {
        let car = &self.cars[i];
        let st = &car.state;
        let v = st.speed;
        let a_me = st.accel;
        let mut hard = false;
        // what is only a matter of the rules (right of way, a full exit) against someone
        // physically in the way
        let mut ruled = false;
        let mut soft: Vec<usize> = Vec::new();
        let mut why: Vec<String> = Vec::new();
        let me_id = car.id;
        // (Omsi.exe: a vehicle whose script sets `TrafficPriority` claims a crossing with
        // priority 1000, above any vehicle type's, FUN_007d9128 - the AI ambulance as much
        // as the player; ours honoured it for the player's bus only)
        let prio = |c: &AiCar| c.vehicle.var("TrafficPriority").is_some_and(|v| v > 0.5);
        let me_prio = prio(car);
        // somebody on, or coming to, a lane that crosses this car's way through the junction
        // (see the gridlock squeeze below)
        let mut contested = false;
        let mut by: Option<u64> = None;
        let mut courtesy = false;
        for &(l, dl) in &jn.lanes {
            for c in &self.net.crossings[l] {
                let point = dl + c.at;
                let m = c.other;
                if way.iter().any(|w| w.0 == m) {
                    continue; // its own way
                }
                // Where the bodies meet (`Crossing::before`/`after`): two paths that cross at a
                // shallow angle, or two turns bending towards each other, bring the cars
                // together metres before their centre lines cross (two turning cars used to
                // touch while the one that gave way still rolled towards "its" point). Paths
                // that run into one another (a merge) are ordered at the joint itself.
                if point + c.after < -st.rear - 0.3 {
                    continue; // passed already
                }
                // vehicles on the other lane, or coming to it
                let on = on_lane
                    .get(&m)
                    .map(|v| v.as_slice())
                    .unwrap_or(&[])
                    .iter()
                    .filter(|e| !e.3)
                    .map(|&(j, sj, _, _)| (j, c.other_at - sj, true));
                let near = coming
                    .get(&m)
                    .map(|v| v.as_slice())
                    .unwrap_or(&[])
                    .iter()
                    .map(|&(j, dj)| (j, dj + c.other_at, false));
                for (j, dj, is_on) in on.chain(near) {
                    if j == i {
                        continue;
                    }
                    let o = &self.cars[j];
                    // it waits behind this car's body: it will not come before this one moves
                    if self.geo_prev.get(j).copied().flatten() == Some(me_id) {
                        continue;
                    }
                    if dj + c.other_after < -o.state.rear - 0.3 {
                        continue; // it is through
                    }
                    if dj - c.other_before < 40.0 {
                        contested = true;
                    }
                    let t_clear = time_to(point + c.after + st.rear + 0.3, v, a_me)
                        + if v < 0.5 { st.reaction } else { 0.0 };
                    // when this car's front gets to the meeting place
                    let t_mine = time_to(point - c.before - st.front, v, a_me)
                        + if v < 0.1 { st.reaction } else { 0.0 };
                    // A meeting place far into the junction's way (the far side of a
                    // roundabout its path runs round to) is not weighed at the line: the car
                    // goes in behind the traffic already on its way and gives way there if
                    // it has to (inside the junction every meeting place counts). Weighed
                    // at the line, an entry waited for a gap of nine seconds on a ring that
                    // never had one, and the queue behind it stood for minutes.
                    if !jn.inside && point - c.before - st.front > 25.0 {
                        continue;
                    }
                    // A stalled car neither claims an imminent arrival nor accelerates
                    // freely in the arrival prediction, even without a reservation.
                    let stalled = (o.stopped > 4.0 && o.state.speed < 0.1)
                        || o.crawl >= 8.0
                        || (o.state.speed < 1.5
                            && o.lead_info.is_some_and(|(lid, gap)| gap < 8.0 && self.cars.iter().find(|x| x.id == lid).is_some_and(|x| x.state.speed < 1.0)));
                    // (one that has waited long at its line keeps a claim on its way while
                    // it stands, `junction_stop`: that claim counts, standing or not.
                    // Taken for a stalled car's, it held nobody back - a side road's car
                    // stood for minutes at a main road whose queue crawled through the
                    // junction without a gap, every newcomer claiming the way first)
                    let long_claim = o.yielding && o.state.yield_time > LONG_WAIT_CLAIM && o.wait_at.is_some();
                    let claimed = reservations
                        .get(&m)
                        .map(|r| r.contains(&j))
                        .unwrap_or(false)
                        && (!stalled || long_claim);
                    let theirs = dj - c.other_before - o.state.front;
                    // it waits for someone else before this meeting place (a car that gives
                    // way further on still rolls through here on its way to its line)
                    let waits_short = o.light_hold
                        || (o.yielding
                            && !claimed
                            && o.wait_at
                                .map(|w| w - 0.6 <= dj - c.other_before)
                                .unwrap_or(false));
                    // (it arrives in `t_j` seconds)
                    let t_j = crossing_arrival(&o.state, theirs, claimed, waits_short, stalled);
                    if is_on && theirs <= 0.3 {
                        // in the meeting place right now: unless this car is further in
                        // already (then it is the other one that has to wait)
                        let mine_in = st.front - (point - c.before);
                        let theirs_in = -theirs;
                        let ahead = mine_in > 0.0
                            && (mine_in > theirs_in + 0.3
                                || ((mine_in - theirs_in).abs() <= 0.3 && me_id > o.id));
                        if !ahead {
                            hard = true;
                            by = by.or(Some(o.id));
                            if explain {
                                why.push(format!("car {} in the crossing of lanes {l}/{m}", o.id));
                            }
                            if jn.inside {
                                stop_at = Some(stop_at.unwrap_or(f32::MAX).min(point - c.before));
                            }
                        }
                        continue;
                    }
                    if claimed || (is_on && o.state.speed > 0.5 && !waits_short) {
                        // Both have decided (or this one is in the junction already): the one
                        // that gets there first goes first, a tie goes to the lower number.
                        // Otherwise two cars standing at their lines, each with a claim,
                        // waited for each other for good.
                        let me_decided = committed || jn.inside;
                        let first = if me_decided {
                            t_j < t_mine - 0.3 || ((t_j - t_mine).abs() <= 0.3 && o.id < me_id)
                        } else {
                            true
                        };
                        if first && t_j < t_clear * if me_decided { 1.0 } else { patience } + 1.0 {
                            hard = true;
                            by = by.or(Some(o.id));
                            if explain {
                                why.push(format!("car {} ({}) arrives at {l}/{m} in {t_j:.1} s, this one in {t_mine:.1} s, clear in {t_clear:.1} s [its v {:.2} stood {:.1} crawl {:.1} yielding {} lead {:?} lane {} theirs {:.1}]", o.id, if claimed { "claimed" } else { "on it" }, o.state.speed, o.stopped, o.crawl, o.yielding, o.lead_info, o.state.lane, theirs));
                            }
                            if jn.inside {
                                stop_at = Some(stop_at.unwrap_or(f32::MAX).min(point - c.before));
                            }
                        } else if long_claim && !me_decided && claimed {
                            // A driver who has waited long at the line pulls out: whoever has
                            // not decided yet and can still stop gently lets him go, however
                            // far his way is from the meeting place. By arrival times alone a
                            // stream at 25 km/h kept a car waiting at the far side of
                            // Spandau's Rathaus junction for minutes, claim or none.
                            courtesy = true;
                            by = by.or(Some(o.id));
                            if explain {
                                why.push(format!("car {} has waited {:.0} s at its line for {l}/{m}", o.id, o.state.yield_time));
                            }
                        }
                        continue;
                    }
                    // a vehicle with priority goes before one without, whatever the lanes say;
                    // one without gives way to it
                    let o_prio = prio(o);
                    if jn.inside || committed || (me_prio && !o_prio) || (!self.net.must_yield(l, m) && !(o_prio && !me_prio)) {
                        continue;
                    }
                    // the gap a driver takes in the main road's traffic: the critical gap of
                    // 5 to 7.5 s (by driver), and time enough to be through
                    if t_j < (st.accept_gap + 2.0).max(t_clear + 1.0) * patience {
                        if t_j == f32::MAX || o.state.speed < 0.3 {
                            soft.push(j);
                        } else {
                            ruled = true;
                            by = by.or(Some(o.id));
                            if explain {
                                why.push(format!("car {} has the right of way at {l}/{m}, arrives in {t_j:.1} s, clear in {t_clear:.1} s", o.id));
                            }
                        }
                    }
                }
                // The player's bus (and a LAN player's) on that lane or coming to it, by the
                // same rules: before, the cars saw it only as a box in their way a second and
                // a half ahead, and pulled out of a side road in front of a bus coming along
                // the main road, or turned across it.
                for u in &self.way_users {
                    let Some(&(_, dm)) = u.lanes.iter().find(|x| x.0 == m) else { continue };
                    let (verdict, t_j, t_clear) = way_user_verdict(st, u, dm, c, point, jn.inside, committed, me_prio, self.net.must_yield(l, m), patience);
                    match verdict {
                        Verdict::Free => {}
                        Verdict::Hard => {
                            hard = true;
                            by = by.or(Some(super::stats::WAITS_ON_PLAYER));
                            if explain {
                                why.push(format!("the player's vehicle in or at the crossing of lanes {l}/{m} (arrives in {t_j:.1} s, this one is clear in {t_clear:.1} s)"));
                            }
                            if jn.inside {
                                stop_at = Some(stop_at.unwrap_or(f32::MAX).min(point - c.before));
                            }
                        }
                        Verdict::Ruled => {
                            ruled = true;
                            by = by.or(Some(super::stats::WAITS_ON_PLAYER));
                            if explain {
                                why.push(format!("the player's vehicle has the right of way at {l}/{m}, arrives in {t_j:.1} s, clear in {t_clear:.1} s"));
                            }
                        }
                    }
                }
            }
            // people on a zebra or a signalled crossing
            for &(w, at, w_at) in &self.net.walks[l] {
                let point = dl + at;
                if point < st.front - 1.0 {
                    continue;
                }
                if walkers
                    .get(&w)
                    .map(|ps| ps.iter().any(|&p| (p - w_at).abs() < 3.0))
                    .unwrap_or(false)
                    && point - st.front < 30.0
                {
                    hard = true;
                    if explain {
                        why.push(format!(
                            "someone on the crossing of lane {l} and footpath {w}"
                        ));
                    }
                    let before = point - 2.5;
                    stop_at = Some(stop_at.unwrap_or(before).min(before));
                }
            }
        }
        Weighing { hard, ruled, soft, why, stop_at, contested, by, courtesy }
    }
}
