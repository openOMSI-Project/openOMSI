//! Filling the map: the bus stops near the player as Omsi.exe sets them up, the people
//! waiting there and the strollers on the pavements.

use super::*;

impl PeopleSim {
    /// Put people at the bus stops near `center`: at the start everywhere, later only at
    /// stops out of sight (the others fill with people walking up).
    pub fn populate(
        &mut self,
        world: &dyn World,
        center: DVec3,
    ) {
        if self.avatar_only {
            return;
        }
        // whoever stands under the surface there (its tile's pavements and roads came
        // after them), or anyone standing still off it: onto it
        for p in self.people.iter_mut() {
            if matches!(p.place, Place::Ground) {
                if let Some(z) = world.walk_height_near(p.position.x, p.position.y, p.position.z) {
                    let d = z - p.position.z;
                    let still = p.vel.length() < 0.05;
                    if d.abs() < 3.0 && (d > 0.02 || (still && d.abs() > 0.02)) {
                        p.position.z = z;
                    }
                }
            }
        }
        self.populate_with(world, None, center);
    }

    pub fn populate_with(
        &mut self,
        world: &dyn World,
        net: Option<&Network>,
        center: DVec3,
    ) {
        self.center = center;
        let list: Vec<(i64, DVec3, f64, String)> = world
            .bus_stops()
            .iter()
            .filter(|s| (s.1 - center).length() < STOP_RANGE + 100.0)
            .map(|s| (s.0, s.1, s.2, s.3.clone()))
            .collect();
        for (id, pos, rot, name) in list {
            // only once the ground under the stop is there
            if world.walk_height(pos.x, pos.y).is_none() || self.stops.contains_key(&id) {
                continue;
            }
            let st = self.build_pax_stop(world, net, id, pos, rot, &name);
            self.stops.insert(id, st);
        }
        self.started = true;
    }

    /// A stop as Omsi.exe sets it up (sub_620058, sub_620c0c, sub_61c604): its waiting
    /// places are the `[passpos]` of every object near it - within 10 m to the platform's
    /// side and from 10 m behind to the stop's length ahead of it -, the gather point a
    /// metre to the kerb and a metre ahead, the destinations of the trips leaving it.
    pub fn build_pax_stop(&mut self, world: &dyn World, net: Option<&Network>, id: i64, pos: DVec3, heading: f64, name: &str) -> PaxStop {
        let length = world.stop_length(id);
        let side = world.stop_side(id).round().clamp(0.0, 255.0) as u8;
        let left = LEFT_HAND.load(std::sync::atomic::Ordering::Relaxed);
        let (xmax, xmin) = (if (side == 1) != left { 0.0 } else { 10.0 }, if (side == 0) != left { 0.0 } else { -10.0 });
        let h = heading.to_radians();
        let objects = world.object_positions();
        let mut spots: Vec<WaitSpot> = Vec::new();
        for (obj, p, face, height) in world.waiting_places().iter() {
            let Some((opos, _)) = objects.get(obj) else {
                if debug_pax() && (*p - pos).length() < 30.0 {
                    log::info!("stop {id}: waiting place of object {obj} at {p:?}: object position unknown");
                }
                continue;
            };
            // (sub_7f0db8 / sub_7f0d3c: the stop less the object)
            let v = pos - *opos;
            // (Omsi.exe looks at every object of the stop's tile and the ones round it; the
            // region below reaches the stop's length ahead, which a long bus station stop
            // takes past 40 m)
            if v.length() > 40.0_f64.max(length as f64 + 15.0) {
                continue;
            }
            // (0x7efb08 with -heading: in the stop's frame)
            let lat = h.cos() * v.x - h.sin() * v.y;
            let along = h.cos() * v.y + h.sin() * v.x;
            if !(-along < 10.0 && -along > -(length as f64).max(10.0) && -lat < xmax && -lat > xmin) {
                if debug_pax() && v.length() < 30.0 {
                    log::info!("stop {id}: object {obj} (waiting place {p:?}) not the stop's: across {:.1}, along {:.1}", -lat, -along);
                }
                continue;
            }
            spots.push(WaitSpot { pos: *p, face: *face, height: *height });
        }
        drop(objects);
        // the gather point (+0x48): (1, 0, 1) or (-1, 0, 1) through the stop's turn
        let x = if (side == 1) == left { 1.0 } else { -1.0 };
        let (fwd, right) = (DVec2::new(h.sin(), h.cos()), DVec2::new(h.cos(), -h.sin()));
        let g = pos.truncate() + right * x + fwd * 1.0;
        let gather = DVec3::new(g.x, g.y, pos.z);
        let lane = net.and_then(|n| self.ped.as_ref().and_then(|pn| pn.nearest(n, pos, 12.0))).map(|(l, s, _)| (l, s));
        let (enter_max, enter_min) = world.stop_enter(id);
        // the destinations: the stops the trips from here go on to, as likely as people
        // get off there; each with the termini of the buses that go there
        let lines: Vec<(String, HashSet<String>)> = self.stop_targets.as_ref().and_then(|m| m.get(&id)).cloned().unwrap_or_default();
        let weights: Vec<f32> = lines
            .iter()
            .map(|(n, _)| {
                world.bus_stops().iter().find(|s| s.3.trim() == n.trim()).map(|s| world.stop_exit_weight(s.0)).unwrap_or(0.5)
            })
            .collect();
        let total: f32 = weights.iter().sum();
        let dests: Vec<(String, f32)> = if total > 0.0 {
            lines.iter().zip(&weights).map(|((n, _), w)| (n.clone(), w / total)).collect()
        } else {
            Vec::new()
        };
        if debug_pax() {
            log::info!("stop {id} '{name}' at ({:.1}, {:.1}, {:.2}) heading {heading:.0}: {} waiting places, length {length}, side {side}, {} destinations", pos.x, pos.y, pos.z, spots.len(), dests.len());
        }
        let n = spots.len();
        // what the timetable calls it - its id when the timetable does not know it, as the
        // targets then do
        let alias = match &self.stop_names {
            Some(n) => n.get(&id).cloned().unwrap_or_else(|| id.to_string()),
            None => String::new(),
        };
        PaxStop {
            name: name.to_string(),
            alias,
            pos,
            heading,
            gather,
            spots,
            taken: vec![false; n],
            enter_max,
            enter_min,
            length,
            lane,
            was_near: false,
            near: false,
            clock_ms: 0.0,
            want: 0,
            factor: 1.0,
            buses: Vec::new(),
            dests,
            lines,
        }
    }

    /// The stops near the player fill with people (sub_61bf94, every frame): a stop coming
    /// into range gets its people at once, one in range another one every 10..15 s while
    /// it has fewer than it should - its pass_enter mean times its own random factor
    /// times the passenger density, at most one per waiting place. A stop going out of
    /// range loses the people waiting there.
    pub fn stops_tick(&mut self, dt: f32, world: &dyn World) {
        if self.mirror || self.avatar_only {
            return;
        }
        let mut ids: Vec<i64> = self.stops.keys().copied().collect();
        ids.sort_unstable();
        let forced = omsi_cfg::flags::OMSI_PAX_WAITING.parse::<usize>();
        // the people handed over to another player's bus stop counting once it has left
        // their stop (or the session)
        if !self.handed.is_empty() {
            let (stops, remote) = (&self.stops, &self.remote_now);
            self.handed.retain(|(stop, bus)| {
                let Some(s) = stops.get(stop) else { return false };
                remote.iter().any(|b| b.id == BusId::Ai(*bus) && (b.pos - s.pos).length() < 60.0)
            });
        }
        for id in ids {
            let near = {
                let s = &self.stops[&id];
                self.anchors().any(|c| (s.pos - c).length() < STOP_RANGE)
            };
            let changed = {
                let s = self.stops.get_mut(&id).unwrap();
                s.was_near = s.near;
                s.near = near;
                s.was_near != s.near
            };
            if !near {
                if changed {
                    // (sub_61be80) the people waiting there go
                    for i in (0..self.people.len()).rev() {
                        let here = matches!(&self.people[i].state, State::Pax(p) if p.stop == Some(id) && p.inside.is_none() && matches!(p.task, Task::WaitingForBus | Task::WalkingToBusstop));
                        if here {
                            self.release(i);
                            let p = self.people.swap_remove(i);
                            self.retire(&p);
                        }
                    }
                    for t in self.stops.get_mut(&id).unwrap().taken.iter_mut() {
                        *t = false;
                    }
                }
                continue;
            }
            if changed {
                let r = self.rand_f() as f32;
                let s = self.stops.get_mut(&id).unwrap();
                let mean = (s.enter_max + s.enter_min) / 2.0;
                let k = if s.enter_max == 0.0 {
                    0.0
                } else if mean == 0.0 {
                    (s.enter_max - s.enter_min) / (s.enter_max * 2.0)
                } else {
                    (s.enter_max - s.enter_min) / (mean * 2.0)
                };
                s.factor = (r * 2.0 - 1.0) * k + 1.0;
            }
            let count = self.people.iter().filter(|p| matches!(&p.state, State::Pax(x) if x.stop == Some(id))).count() + self.handed.iter().filter(|h| h.0 == id).count();
            let want = {
                let s = &self.stops[&id];
                let mean = (s.enter_max + s.enter_min) / 2.0;
                // (0x61bf94: with a timetable, times the share of the trips due there - at a
                // stop no trip leaves from, nobody)
                let none_due = self.due_dests.as_ref().is_some_and(|d| d.get(&id).is_none_or(|set| set.is_empty()));
                let served = if self.stop_targets.is_some() && (s.lines.is_empty() || none_due) { 0.0 } else { 1.0 };
                let w = (self.density.max(0.0) * mean * s.factor * served).round().max(0.0) as usize;
                forced.unwrap_or(w).min(s.spots.len())
            };
            let s = self.stops.get_mut(&id).unwrap();
            s.want = want;
            s.clock_ms += dt * 1000.0;
            if !changed {
                let r = self.rand_f() as f32;
                if self.stops[&id].clock_ms <= r * 5000.0 + 10000.0 {
                    continue;
                }
            }
            let mut count = count;
            while count < want {
                if self.spawn_waiting(world, id).is_none() {
                    break;
                }
                count += 1;
                if !changed {
                    break;
                }
            }
            if count >= want {
                self.stops.get_mut(&id).unwrap().clock_ms = 0.0;
            }
        }
    }

    /// The trips due at the stops soon (`due_dests`) made anew by `due` for game time
    /// `day_time` once a game minute has gone by since they last were (#1415). Made once
    /// only, at the game's start, they leave every stop without a trip in its first quarter
    /// of an hour without people for the rest of the session.
    pub fn keep_due_dests(
        &mut self,
        day_time: f64,
        due: impl FnOnce(f64) -> HashMap<i64, HashSet<String>>,
    ) {
        if (day_time - self.due_at).abs() >= 60.0 {
            self.due_dests = Some(due(day_time));
            self.due_at = day_time;
        }
    }

    /// A destination drawn from stop `id`'s (sub_61baa8): by weight; none when the weights
    /// leave the draw over. Also the stop's line record it matched.
    pub fn draw_dest(&mut self, id: i64) -> (Option<String>, Option<usize>) {
        let mut r = self.rand_f() as f32;
        let mut dest: Option<String> = None;
        let Some(stop) = self.stops.get(&id) else { return (None, None) };
        // (of the trips due here soon, `due_dests`; their weights made a whole again)
        let due = self.due_dests.as_ref().map(|d| d.get(&id));
        let dests: Vec<(&String, f32)> = match due {
            Some(set) => {
                let kept: Vec<(&String, f32)> = stop.dests.iter().filter(|(n, _)| set.is_some_and(|s| s.contains(n.trim()))).map(|(n, w)| (n, *w)).collect();
                let total: f32 = kept.iter().map(|k| k.1).sum();
                if total <= 0.0 {
                    return (None, None);
                }
                // (the share that drew no destination stays the stop's own: those people go
                // nowhere by bus, as before)
                let all: f32 = stop.dests.iter().map(|d| d.1).sum();
                kept.into_iter().map(|(n, w)| (n, w / total * all)).collect()
            }
            None => stop.dests.iter().map(|(n, w)| (n, *w)).collect(),
        };
        for (n, w) in dests {
            if r <= 0.0 {
                break;
            }
            r -= w;
            if r <= 0.0 {
                dest = Some(n.clone());
            }
        }
        let line = dest.as_ref().and_then(|d| stop.lines.iter().position(|(n, _)| n.trim() == d.trim()));
        (dest, line)
    }

    /// A person put at a free waiting place of stop `id` (sub_626044) with a destination
    /// drawn from the stop's (sub_61baa8); they settle there as task 6 does.
    pub fn spawn_waiting(&mut self, world: &dyn World, id: i64) -> Option<usize> {
        if self.stops.get(&id)?.taken.iter().all(|t| *t) || !self.pool_room() {
            return None;
        }
        let k = self.take_spot(id)?;
        let sp = self.stops[&id].spots[k].clone();
        let (dest, line) = self.draw_dest(id);
        let walk = 1.1 + (self.rand_f() as f32 * 2.0 - 1.0) * 0.2;
        let mut pax = Pax::new(walk, self.rand_f());
        pax.stop = Some(id);
        pax.spot = Some(k);
        pax.pos = sp.pos;
        pax.yaw = sp.face.to_radians();
        pax.dest = dest;
        pax.line = line;
        pax.st = 0;
        let Some(i) = self.spawn(world, sp.pos, sp.face, State::Pax(Box::new(pax))) else {
            self.free_spot(id, k);
            return None;
        };
        let dummy_b: Vec<BusNow> = Vec::new();
        let dummy_ix: HashMap<BusId, usize> = HashMap::new();
        self.set_task(i, Task::WalkingToBusstop, &dummy_b, &dummy_ix, world);
        if debug_pax() {
            let d = self.pax(i).and_then(|p| p.dest.clone());
            log::info!("t={:.1} pax {} waits at stop {id} place {k}, for {:?}", self.time, self.people[i].label(), d);
        }
        Some(i)
    }

    /// Keep strollers on the pavements near the player, and people walking up to the stops.
    pub fn populate_on_foot(
        &mut self,
        world: &dyn World,
        net: &Network,
        dt: f32,
    ) {
        let Some(ped) = self.ped.take() else { return };
        self.populate_on_foot_with(&ped, world, net, dt);
        self.ped = Some(ped);
    }

    pub fn populate_on_foot_with(
        &mut self,
        ped: &PedNet,
        world: &dyn World,
        net: &Network,
        _dt: f32,
    ) {
        let center = self.center;
        // strollers: as many as the pavement around carries
        let lanes: Vec<usize> = ped
            .ends
            .keys()
            .copied()
            .filter(|&l| {
                (net.lanes[l].start() - center).truncate().length() < STROLL_RADIUS * 0.9
                    && net.lanes[l].length() > 4.0
            })
            .collect();
        let crowd = (lanes.len() as f32 / 120.0).clamp(0.6, 3.0);
        let target =
            (self.pedestrians as f32 * crowd * self.density.clamp(0.0, 3.0)).round() as usize;
        // (0x62463c: walking the pavements only while fewer than half the pool do, and never
        // past the pool)
        let target = target.min(self.max_people / 2).min((self.max_people + self.people.iter().filter(|p| matches!(p.state, State::Strolling(_))).count()).saturating_sub(self.pool_used()));
        let have = self
            .people
            .iter()
            .filter(|p| matches!(p.state, State::Strolling(_)))
            // (around this player only, when a LAN host keeps people around several)
            .filter(|p| {
                self.lan_centers.is_empty() || (p.position - center).length() < STROLL_RADIUS
            })
            .count();
        if have < target && !lanes.is_empty() {
            for _ in 0..(target - have).min(4) {
                let lane = lanes[(self.rand() as usize) % lanes.len()];
                let len = net.lanes[lane].length();
                let s = (self.rand_f() as f32 * (len - 1.0)).max(0.5);
                let (p, h) = net.lanes[lane].at(s);
                if self.seen(p)
                    || (p - center).length() < 20.0
                    || (p - center).length() > STROLL_RADIUS * 0.9
                    || !world.has_ground(p.x, p.y)
                {
                    continue;
                }
                let fwd = self.rand_f() < 0.5;
                let leg = if fwd {
                    Leg { lane, a: s, b: len }
                } else {
                    Leg { lane, a: s, b: 0.0 }
                };
                let side = 0.3 + self.rand_f() as f32 * 0.4;
                let heading = if fwd { h as f64 } else { h as f64 + 180.0 };
                if let Some(i) = self.spawn(
                    world,
                                        p,
                    heading,
                    State::Strolling(PedWalk::new(vec![leg], true, side)),
                ) {
                    self.people[i].activity = Activity::Walk;
                }
            }
        }
        // OMSI_PAX_CROSS=x,y: a few pedestrians sent across the signalised crossing nearest that point
        if let Some((x, y)) = omsi_cfg::flags::OMSI_PAX_CROSS.var().and_then(|v| {
            let mut it = v.split(',').filter_map(|t| t.trim().parse::<f64>().ok());
            Some((it.next()?, it.next()?))
        }) {
            let want = DVec3::new(x, y, center.z);
            let placed = self
                .people
                .iter()
                .filter(|p| matches!(p.state, State::Strolling(ref w) if !w.roam || w.side < 0.0))
                .count();
            let lane = ped
                .ends
                .keys()
                .copied()
                .filter(|&l| net.lanes[l].traffic_light.is_some())
                .min_by(|a, b| {
                    (net.lanes[*a].start() - want)
                        .truncate()
                        .length()
                        .total_cmp(&(net.lanes[*b].start() - want).truncate().length())
                });
            if let (Some(cross), 0) = (lane, placed) {
                let (start_node, _) = ped.ends[&cross];
                let feeders: Vec<(usize, bool)> = ped.out[start_node]
                    .iter()
                    .copied()
                    .filter(|(l, _)| *l != cross)
                    .collect();
                log::info!(
                    "OMSI_PAX_CROSS: crossing path {cross} light {:?}, {} paths lead to it",
                    net.lanes[cross].traffic_light,
                    feeders.len()
                );
                for k in 0..6 {
                    let Some(&(lane, fwd)) = feeders.get(k % feeders.len().max(1)) else {
                        break;
                    };
                    let len = net.lanes[lane].length();
                    let back = (3.0 + k as f32 * 1.6).min(len);
                    let first = if fwd {
                        Leg {
                            lane,
                            a: back,
                            b: 0.0,
                        }
                    } else {
                        Leg {
                            lane,
                            a: len - back,
                            b: len,
                        }
                    };
                    let over = Leg {
                        lane: cross,
                        a: 0.0,
                        b: net.lanes[cross].length(),
                    };
                    let (p, h) = first.at(net, 0.0);
                    let mut walk = PedWalk::new(vec![first, over], true, 0.4);
                    // marked so that the knob spawns them once
                    walk.side = -0.4;
                    if let Some(i) =
                        self.spawn(world, p, h, State::Strolling(walk))
                    {
                        self.people[i].activity = Activity::Walk;
                    }
                }
            }
        }
    }
}
