//! The pedestrians' walk: what each wants, the crossings, the crowd's result applied, and
//! keeping out of walls.

use super::*;

/// Continue from a stop in the middle or at either end of a pavement lane.
/// A zero-length first leg at the start of a lane must not turn into another
/// zero-length leg: the passenger would stand motionless after disembarking.
fn pavement_continuation(leg: Leg, lane_len: f32, pick: u64) -> Leg {
    let end = if leg.b <= 0.05 {
        lane_len
    } else if leg.b >= lane_len - 0.05 {
        0.0
    } else if leg.len() > 0.05 {
        if leg.b > leg.a { lane_len } else { 0.0 }
    } else if pick % 2 == 0 {
        lane_len
    } else {
        0.0
    };
    Leg { lane: leg.lane, a: leg.b, b: end }
}

impl PeopleSim {
    /// Nobody on foot walks into a wall: the scenery's collision boxes and meshes (shelters,
    /// fences, walls, buildings with a collision mesh) between knee and head height stop a
    /// step that would enter one, keeping the part of it along the wall. Somebody already
    /// inside one (a waiting place the map put in a shelter's box) is left alone - pushed
    /// out, they jumped. People used to walk through everything but the vehicles.
    pub fn keep_out_of_walls(&mut self, world: &dyn World, who: &[usize], ground: &mut [(usize, Walker)]) {
        const R: f64 = 0.22;
        const CELL: f64 = 12.0;
        let collision = world.collision();
        let places: Vec<DVec2> = world.waiting_places().iter().map(|w| w.1.truncate()).collect();
        let (boxes, meshes, since) = (collision.boxes.len(), collision.meshes.len(), self.wall_key.3);
        if (boxes, meshes, places.len()) != (self.wall_key.0, self.wall_key.1, self.wall_key.2) || self.time - since > 2.0 || self.time < since {
            self.wall_cells.clear();
            self.wall_key = (boxes, meshes, places.len(), self.time);
        }
        let mut cells = std::mem::take(&mut self.wall_cells);
        for (k, w) in ground.iter_mut() {
            let i = who[*k];
            if w.fixed || self.people[i].place != Place::Ground {
                continue;
            }
            let p0 = self.people[i].position.truncate();
            if (w.pos - p0).length_squared() < 1e-8 {
                continue;
            }
            let z = self.people[i].position.z;
            let key = ((p0.x / CELL).floor() as i32, (p0.y / CELL).floor() as i32, z.floor() as i32);
            let walls = cells.entry(key).or_insert_with(|| {
                let c = DVec2::new((key.0 as f64 + 0.5) * CELL, (key.1 as f64 + 0.5) * CELL);
                let probe = crate::collision::Obb {
                    center: c,
                    half: DVec2::splat(CELL * 0.5 + 2.0),
                    heading: 0.0,
                    z0: key.2 as f64 - 1.0,
                    z1: key.2 as f64 + 3.5,
                    velocity: DVec2::ZERO,
                    mass: 0.0,
                    pole: None,
                    id: -1,
                };
                let near = collision.obstacles_near(&probe);
                let reach = near.iter().map(|o| (o.center - c).length() + o.half.length() + 1.0).fold(0.0, f64::max);
                let local: Vec<DVec2> = places.iter().copied().filter(|q| (*q - c).length() < reach).collect();
                near.into_iter()
                    .filter(|o| {
                        // a shelter given as one solid box has its waiting places inside:
                        // people go in there
                        let b = Block { center: o.center, half: o.half, heading: o.heading, vel: DVec2::ZERO };
                        !local.iter().any(|q| (*q - o.center).length() < o.half.length() + 1.0 && b.near(*q, 0.3))
                    })
                    .map(|o| {
                        (
                            Block {
                                center: o.center,
                                half: o.half + DVec2::splat(R),
                                heading: o.heading,
                                vel: DVec2::ZERO,
                            },
                            o.z0,
                            o.z1,
                        )
                    })
                    .collect()
            });
            for (b, z0, z1) in walls.iter() {
                // between the knees and the head of somebody standing here
                if *z0 > z + 1.6 || *z1 < z + 0.5 {
                    continue;
                }
                if (w.pos - b.center).length_squared() >= b.half.length_squared() || !b.near(w.pos, 0.0) || b.near(p0, -0.01) {
                    continue;
                }
                let (q, inside) = b.closest(w.pos);
                if !inside {
                    continue;
                }
                // onto the wall's face, keeping the step along it
                if omsi_cfg::flags::OMSI_DEBUG_WALLS.is_set() {
                    log::info!("t={:.1} pax {} ({}) kept out of a wall ({:.1} x {:.1} m, heights {:.1}..{:.1}) at ({:.2}, {:.2}), its centre ({:.2}, {:.2}), want ({:.2}, {:.2}) vel ({:.2}, {:.2})", self.time, self.people[i].label(), self.people[i].state.name(), b.half.x * 2.0, b.half.y * 2.0, z0 - z, z1 - z, w.pos.x, w.pos.y, b.center.x, b.center.y, w.want.x, w.want.y, w.vel.x, w.vel.y);
                }
                let n = (q - w.pos).try_normalize().unwrap_or(DVec2::ZERO);
                w.pos = q + n * 0.005;
                let vn = w.vel.dot(n);
                if vn < 0.0 {
                    w.vel -= n * vn;
                }
                let fresh = self.people[i].detour <= 0.0;
                self.people[i].detour = 2.0;
                w.corridor = None;
                // walking straight at it: round it, the way that turns least from where they
                // want to go (a lamp post or a pillar stopped people dead)
                let speed = w.want.length();
                let t = DVec2::new(-n.y, n.x);
                if fresh || self.people[i].detour_side == 0.0 {
                    let along = w.want.dot(t);
                    self.people[i].detour_side = if along.abs() > 0.05 * speed {
                        along.signum()
                    } else if i % 2 == 0 {
                        1.0
                    } else {
                        -1.0
                    };
                }
                if speed > 0.2 && w.vel.dot(t) * self.people[i].detour_side < 0.4 * speed {
                    w.vel = t * self.people[i].detour_side * speed * 0.8;
                }
            }
        }
        self.wall_cells = cells;
    }

    /// OMSI_CHECK_WALLS: everybody inside a bus who stands away from its walkways (more
    /// than 0.45 m from every path link, not on a seat): through a seat back or a wall.
    pub fn check_walls(&self) {
        for p in &self.people {
            let Place::Bus(bus, local) = p.place else { continue };
            if matches!(&p.state, State::Pax(x) if x.task == Task::SittingInBus || x.st == 9) {
                continue;
            }
            let Some(bn) = self.last_buses.iter().find(|b| b.id == bus) else { continue };
            let pts = &bn.cabin.graph.points;
            let mut best = f32::INFINITY;
            for &(a, b, _) in &bn.cabin.links {
                let (Some(pa), Some(pb)) = (pts.get(a.max(0) as usize), pts.get(b.max(0) as usize)) else { continue };
                let ab = *pb - *pa;
                let t = if ab.length_squared() > 1e-6 { ((local - *pa).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                let q = *pa + ab * t;
                best = best.min((q.truncate() - local.truncate()).length() + (q.z - local.z).abs());
            }
            let near_seat = bn.cabin.seats.iter().any(|s| (s.floor - local).truncate().length() < 0.35 || (s.pos - local).truncate().length() < 0.35);
            if best > 0.45 && !near_seat && !bn.cabin.links.is_empty() {
                log::warn!("t={:.1} person {} in bus {:?} off the walkways by {best:.2} m at ({:.2}, {:.2}, {:.2}), state {}", self.time, p.id, bus, local.x, local.y, local.z, p.state.name());
            }
        }
    }

    /// What pedestrian `i` wants this frame (task 8, `WalkStreet`).
    #[allow(clippy::too_many_arguments)]
    pub fn decide(
        &mut self,
        i: usize,
        dt: f32,
        world: &dyn World,
        net: Option<&Network>,
        traffic: Option<&TrafficSim>,
        cars: &[(DVec2, DVec2, f64)],
        remove: &mut Vec<usize>,
    ) -> Want {
        let state = self.people[i].state.clone();
        let pos2 = self.people[i].position.truncate();
        // somebody walking on towards ground that is not loaded goes
        if self.people[i].place == Place::Ground && self.people[i].vel.length_squared() > 1e-4 && !world.has_ground(pos2.x, pos2.y) {
            remove.push(i);
            return Want::stand(None, Activity::Stand);
        }
        match state {
            State::Strolling(mut walk) => {
                let seen = self.seen(self.people[i].position);
                // (a stroller goes only once well out of everybody's range and out of sight -
                // not somebody put out to run for a bus, `runners`, which may be that far off)
                let far = !self.put_out(self.people[i].id) && self.far_from_players(self.people[i].position, STROLL_RADIUS * 2.0);
                let Some(net) = net else {
                    remove.push(i);
                    return Want::stand(None, Activity::Stand);
                };
                if far && !seen {
                    remove.push(i);
                    return Want::stand(None, Activity::Stand);
                }
                let w = self.walk_want(i, &mut walk, net, traffic, cars, dt);
                self.people[i].state = State::Strolling(walk);
                w
            }
            State::Standing => {
                // A person may get off before the stop has a pavement lane, or
                // the lane may arrive later with a streamed tile. Do not leave
                // them rooted forever: recover onto the closest usable path.
                // (looked for once a second: nobody waits for it, and somebody with no
                // lane in reach looked every frame)
                let look = self.people[i].t_state % 1.0 < dt.max(1e-3);
                if let Some((net, leg)) = net.filter(|_| look).and_then(|net| {
                    self.ped.as_ref()
                        .and_then(|ped| ped.nearest(net, self.people[i].position, 16.0))
                        .filter(|&(lane, at, _)| {
                            let (target, _) = net.lanes[lane].at(at);
                            net.lanes[lane].length() > 0.35
                                && !crosses_street(net, self.people[i].position.truncate(), target.truncate())
                        })
                        .map(|(lane, at, _)| (net, Leg { lane, a: at, b: at }))
                }) {
                    let mut walk = PedWalk::new(vec![leg], true, 0.0);
                    let want = self.walk_want(i, &mut walk, net, traffic, cars, dt);
                    self.people[i].state = State::Strolling(walk);
                    return want;
                }
                if !self.seen(self.people[i].position) && self.far_from_players(self.people[i].position, STROLL_RADIUS) {
                    remove.push(i);
                }
                Want::stand(None, Activity::Stand)
            }
            _ => Want::stand(None, Activity::Stand),
        }
    }

    /// Where a walk along the pavement takes somebody next.
    pub fn walk_want(
        &mut self,
        i: usize,
        walk: &mut PedWalk,
        net: &Network,
        traffic: Option<&TrafficSim>,
        cars: &[(DVec2, DVec2, f64)],
        dt: f32,
    ) -> Want {
        let mut ped = self.ped.take();
        let w = self.walk_want_with(ped.as_mut(), i, walk, net, traffic, cars, dt);
        self.ped = ped;
        w
    }

    #[allow(clippy::too_many_arguments)]
    pub fn walk_want_with(
        &mut self,
        mut ped: Option<&mut PedNet>,
        i: usize,
        walk: &mut PedWalk,
        net: &Network,
        traffic: Option<&TrafficSim>,
        cars: &[(DVec2, DVec2, f64)],
        dt: f32,
    ) -> Want {
        let pos2 = self.people[i].position.truncate();
        let pace = self.people[i].pace;
        if walk.leg >= walk.legs.len() {
            return Want::stand(None, Activity::Stand);
        }
        let leg = walk.legs[walk.leg];
        if walk.s >= leg.len() - 0.35 || walk.held > 0.0 {
            // at the end of the leg: which way on
            if walk.leg + 1 >= walk.legs.len() {
                if !walk.roam {
                    walk.leg += 1;
                    return Want::stand(None, Activity::Stand);
                }
                let pick = self.rand();
                let next = ped
                    .as_ref()
                    .and_then(|p| {
                        p.end_node(net, &leg)
                            .and_then(|n| p.next_leg(net, n, leg.lane, pick))
                    })
                    .unwrap_or_else(|| {
                        // a leg that ends in the middle of its path (the point of a stop
                        // somebody got off at) goes on to one of its ends: turned round
                        // there, the people off a bus were sent back to the same point
                        // every frame and milled round each other at the stop (#913)
                        pavement_continuation(leg, net.lanes[leg.lane].length(), pick)
                    });
                walk.legs.push(next);
                if walk.leg > 6 {
                    walk.legs.drain(..walk.leg);
                    walk.leg = 0;
                }
            }
            let next = walk.legs[walk.leg + 1];
            match self.may_cross(
                ped.as_deref_mut(),
                net,
                &next,
                traffic,
                cars,
                pace,
                walk.held,
            ) {
                Ok(()) => {
                    if walk.held > 0.0 && debug_pax() {
                        let light = net.lanes[next.lane].traffic_light.and_then(|(c, li)| {
                            traffic
                                .and_then(|t| t.light_state(c, li))
                                .map(|(st, left)| {
                                    format!(", light {c}.{li} state {st} for {left:.1} s more")
                                })
                        });
                        log::info!(
                            "t={:.1} pax {} crosses path {} after waiting {:.0} s{}",
                            self.time,
                            self.people[i].label(),
                            next.lane,
                            walk.held,
                            light.unwrap_or_default()
                        );
                    }
                    walk.s = (walk.s - leg.len()).max(0.0);
                    walk.leg += 1;
                    walk.held = 0.0;
                }
                Err(why) => {
                    walk.held += dt;
                    self.people[i].why = why;
                    // a light that stays red (nobody crosses on red any more): a stroller
                    // gives up after three minutes and walks back the way they came
                    if walk.roam && why == "red light" && walk.held > 180.0 {
                        if debug_pax() {
                            log::info!(
                                "t={:.1} pax {} gives up waiting at the red light and turns back",
                                self.time,
                                self.people[i].label()
                            );
                        }
                        walk.legs.truncate(walk.leg + 1);
                        walk.legs.push(Leg {
                            lane: leg.lane,
                            a: leg.b,
                            b: leg.a,
                        });
                        walk.held = 0.0;
                        return Want::stand(None, Activity::Stand);
                    }
                    // at the kerb, facing the way across, spread along it and a step back
                    let (end, _) = leg.at(net, leg.len());
                    let (_, h) = next.at(net, 0.3);
                    let hr = h.to_radians();
                    let (fwd, right) = (
                        DVec2::new(hr.sin(), hr.cos()),
                        DVec2::new(hr.cos(), -hr.sin()),
                    );
                    let id = self.people[i].id;
                    let spread = ((id % 5) as f64 - 2.0) * 0.45;
                    let back = 0.25 + (id % 3) as f64 * 0.45;
                    let spot = end.truncate() + right * spread - fwd * back;
                    return Want {
                        vel: arrive(pos2, spot, pace * 0.6),
                        face: Some(h),
                        give: 0.5,
                        corridor: None,
                        idle: Activity::Stand,
                    };
                }
            }
        }
        let leg = walk.legs[walk.leg];
        let len = leg.len();
        let (p, h) = leg.at(net, (walk.s + 1.3).min(len));
        let lane = &net.lanes[leg.lane];
        let width = (lane.width as f64).max(1.0);
        let crossing = lane.traffic_light.is_some()
            || ped
                .as_ref()
                .map(|p| {
                    p.crossings
                        .get(&leg.lane)
                        .map(|x| !x.is_empty())
                        .unwrap_or(false)
                })
                .unwrap_or(false);
        // keep to the right of the pavement (less so on a crossing) - the left where the
        // traffic drives on the left
        let side = if crossing {
            (walk.side.abs() as f64).min(0.3)
        } else {
            (walk.side.abs() as f64).min(width * 0.5 - 0.3).max(0.0)
        };
        let side = if net.left_hand { -side } else { side };
        let hr = h.to_radians();
        let right = DVec2::new(hr.cos(), -hr.sin());
        let target = p.truncate() + right * side;
        let vel = (target - pos2).normalize_or_zero() * pace;
        // stay on the path: the lane locally, as wide as it is
        let (a, _) = leg.at(net, (walk.s - 2.0).max(0.0));
        let (b, _) = leg.at(net, (walk.s + 2.5).min(len));
        let (m, _) = leg.at(net, (walk.s + 0.25).min(len));
        let bow = crowd::project_on_segment(m.truncate(), a.truncate(), b.truncate())
            .0
            .distance(m.truncate());
        let corridor = ((b - a).truncate().length() > 0.5).then(|| {
            (
                a.truncate() + right * side,
                b.truncate() + right * side,
                (width * 0.5 - side).max(0.35) + bow,
            )
        });
        let corridor = path_corridor(pos2, corridor);
        if self.people[i].why != "queueing behind somebody" {
            self.people[i].why = "";
        }
        Want {
            vel,
            face: None,
            give: 1.0,
            corridor,
            idle: Activity::Stand,
        }
    }

    /// May a pedestrian at the kerb start along `next`? A pedestrian light must show green,
    /// and the time left to get across - the green and then the clearance until a light of
    /// the carriageway turns green - must do; without a light no car may be about to pass
    /// the crossing. Somebody who has waited very long takes any green (never a red).
    #[allow(clippy::too_many_arguments)]
    pub fn may_cross(
        &self,
        mut ped: Option<&mut PedNet>,
        net: &Network,
        next: &Leg,
        traffic: Option<&TrafficSim>,
        cars: &[(DVec2, DVec2, f64)],
        pace: f64,
        held: f32,
    ) -> Result<(), &'static str> {
        if !next.from_end(net) {
            return Ok(());
        }
        let lane = &net.lanes[next.lane];
        let t_cross = next.len() as f64 / pace.max(0.5) + 1.0;
        if let (Some((c, li)), Some(t)) = (lane.traffic_light, traffic) {
            if let Some((state, left)) = t.light_state(c, li) {
                if !crate::traffic::TrafficLightController::allows_go(state) {
                    return Err("red light");
                }
                if held > 150.0 {
                    return Ok(());
                }
                // A pedestrian green is short (8 s at Grundorf for an 11.6 m crossing that
                // takes 10.7 s): who starts on green crosses in the clearance time after
                // it, until the cars get their green. Only the green alone was counted, so
                // nobody ever started on green and everybody went across on red after
                // 150 s, in front of moving cars.
                let window = pedestrian_window(ped.as_deref_mut(), net, t, next.lane, left);
                if (window as f64) < t_cross {
                    return Err("the green ends before they would be across");
                }
                return Ok(());
            }
        }
        let Some(ped) = ped else { return Ok(()) };
        // somebody who has waited long accepts a shorter gap (down to the time the crossing
        // takes, never less): a steady stream does not hold them for ever, but nobody walks
        // out in front of a car that is about to be there (after 45 s they used to ignore
        // the cars altogether)
        let margin = if held > 45.0 { 0.0 } else if held > 20.0 { 1.0 } else { 2.5 };
        for x in ped.crossings(net, next.lane) {
            for (p, v, half) in cars {
                let rel = *x - *p;
                let dist = rel.length();
                if dist > 90.0 {
                    continue;
                }
                if dist < half + 1.5 {
                    return Err("a vehicle stands on the crossing");
                }
                let speed = v.length();
                if speed < 0.5 {
                    continue;
                }
                let dir = *v / speed;
                let along = rel.dot(dir);
                let lateral = rel.perp_dot(dir).abs();
                if along > -half && lateral < 3.5 && along / speed < t_cross + margin {
                    return Err("waits for a car to pass");
                }
            }
        }
        Ok(())
    }

    /// Take over where the crowd moved person `i`.
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &mut self,
        i: usize,
        w: &Walker,
        want: &Want,
        dt: f32,
        world: &dyn World,
        net: Option<&Network>,
        buses: &[BusNow],
        bus_ix: &HashMap<BusId, usize>,
    ) {
        let dt64 = dt as f64;
        let time = self.time;
        let p = &mut self.people[i];
        let speed = w.vel.length();
        // somebody pressed against somebody else for seconds slips past them
        if want.vel.length() > 0.2 && speed < 0.08 {
            p.stuck += dt;
        } else if speed > 0.2 {
            p.stuck = 0.0;
        }
        p.detour = (p.detour - dt).max(0.0);
        if p.ghost > 0.0 {
            p.ghost -= dt;
        } else if p.stuck > 2.5 {
            p.ghost = 1.5;
            p.stuck = 0.0;
            if debug_pax() {
                log::info!(
                    "t={time:.1} pax {} ({}) is stuck and slips past",
                    p.label(),
                    p.state.name()
                );
            }
        }
        p.vel = w.vel;
        match p.place {
            Place::Ground => {
                p.position.x = w.pos.x;
                p.position.y = w.pos.y;
                if let Some(z) = world.walk_height_near(p.position.x, p.position.y, p.position.z) {
                    // up a kerb quickly, down it smoothly (the feet find the kerb themselves);
                    // more than a kerb below the surface is no step but a wrong height (the
                    // pavement's tile came after them): straight onto it
                    // (and whoever stands still simply stands on it: waiting people sank
                    // into a pavement that came after them and rose only when they walked)
                    p.position.z = if z - p.position.z > 0.35 || speed < 0.05 {
                        z
                    } else if z > p.position.z {
                        z.min(p.position.z + 1.5 * dt64)
                    } else {
                        z.max(p.position.z - 2.0 * dt64)
                    };
                }
            }
            Place::Bus(b, l) => {
                let here = Vec3::new(w.pos.x as f32, w.pos.y as f32, l.z);
                let z = l.z;
                let local = Vec3::new(here.x, here.y, z);
                p.place = Place::Bus(b, local);
                if let Some(bn) = bus_ix.get(&b).map(|k| &buses[*k]) {
                    p.position = bn.world(local);
                    p.interior = bn.interior;
                    p.tilt = bn.tilt_at(local);
                }
            }
        }
        // progress along the pavement
        if let Some(net) = net {
            let pos = p.position;
            match &mut p.state {
                State::Strolling(walk) => {
                    if let Some(leg) = walk.legs.get(walk.leg) {
                        walk.s = leg.project(net, pos, walk.s).max(walk.s - 0.3);
                    }
                }
                _ => {}
            }
        }
        let walking = if p.activity == Activity::Walk {
            speed > 0.12
        } else {
            speed > 0.3
        };
        let activity = if walking { Activity::Walk } else { want.idle };
        let inside = matches!(p.place, Place::Bus(..));
        let bus_heading = match p.place {
            Place::Bus(b, l) => bus_ix
                .get(&b)
                .map(|k| buses[*k].heading_at(l))
                .unwrap_or(0.0),
            Place::Ground => 0.0,
        };
        let current = if inside { p.lheading } else { p.heading };
        let target = if speed > 0.25 {
            Some(crowd::heading_of(w.vel))
        } else {
            want.face
        };
        // turning eases in and out (a constant rate started and stopped with a jerk): the
        // rate follows the angle still to go, up to the most a walker or a stander turns
        let turned = match target {
            Some(t) => {
                let left = crowd::angle_diff(current, t);
                let max_rate = if walking { 260.0 } else { 140.0 };
                let rate = (left.abs() * 5.0).min(max_rate).max(12.0);
                crowd::turn_towards(current, t, rate, dt64)
            }
            None => current,
        };
        if inside {
            p.lheading = turned;
            p.heading = bus_heading + turned;
        } else {
            p.heading = turned;
        }
        p.activity = activity;
    }

    /// People carried by a bus in their seat.
    pub fn carry(
        &mut self,
        i: usize,
        dt: f32,
        buses: &[BusNow],
        bus_ix: &HashMap<BusId, usize>,
        face: Option<f64>,
    ) {
        let p = &mut self.people[i];
        let Place::Bus(b, l) = p.place else { return };
        let Some(bn) = bus_ix.get(&b).map(|k| &buses[*k]) else {
            return;
        };
        p.position = bn.world(l);
        p.tilt = bn.tilt_at(l);
        p.vel = DVec2::ZERO;
        if let Some(f) = face {
            p.lheading = crowd::turn_towards(p.lheading, f, 150.0, dt as f64);
        }
        p.heading = bn.heading_at(l) + p.lheading;
        p.interior = bn.interior;
    }
}

#[cfg(test)]
mod pavement_recovery_tests {
    use super::*;

    #[test]
    fn passenger_leaving_from_either_lane_end_keeps_walking() {
        let start = pavement_continuation(Leg { lane: 7, a: 0.0, b: 0.0 }, 20.0, 0);
        assert_eq!((start.a, start.b), (0.0, 20.0));
        let end = pavement_continuation(Leg { lane: 7, a: 20.0, b: 20.0 }, 20.0, 0);
        assert_eq!((end.a, end.b), (20.0, 0.0));
    }

    #[test]
    fn passenger_leaving_midway_takes_a_nonzero_path() {
        for pick in 0..2 {
            let leg = pavement_continuation(Leg { lane: 2, a: 7.0, b: 7.0 }, 20.0, pick);
            assert!(leg.len() >= 7.0);
        }
    }
}
