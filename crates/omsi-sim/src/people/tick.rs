//! The frame: the simulation step (`tick_inner`, no renderer; omsi-app's `Humans::tick`
//! shows the renderer what it did afterwards), and everybody's animation.

use super::*;

impl PeopleSim {
    /// What follows the step and the drawing of what it did (omsi-app's `Humans::tick`
    /// runs `tick_inner`, then the view, then this): the slow ticks logged, the checks and
    /// the statistics. `ms`: how long the whole tick took.
    pub fn tick_done(&mut self, dt: f32, world: &dyn World, ms: f64) {
        if (debug_pax() || omsi_cfg::flags::OMSI_PROFILE.is_set()) && ms > 30.0 {
            log::info!(
                "t={:.1} slow people tick: {ms:.1} ms ({} people): {}",
                self.time,
                self.people.len(),
                self.tick_stages.iter().filter(|s| s.1 >= 1.0).map(|(n, t)| format!("{n} {t:.1}")).collect::<Vec<_>>().join(", ")
            );
        }
        if omsi_cfg::flags::OMSI_CHECK_WALLS.is_set() {
            self.check_walls();
        }
        // OMSI_CHECK_GROUND=1: people on foot with a walkable surface over their heads'
        // reach above them, every two seconds (people "in the ground")
        if omsi_cfg::flags::OMSI_CHECK_GROUND.is_set() && (self.time / 2.0).floor() != ((self.time - dt as f64) / 2.0).floor() {
            for p in &self.people {
                if !matches!(p.place, Place::Ground) || p.puppet.is_some() {
                    continue;
                }
                // the floor under the feet: the highest face up to a step (0.5 m) over them
                let floor = world.walk_height_near(p.position.x, p.position.y, p.position.z);
                if let Some(f) = floor {
                    if f - p.position.z > 0.05 {
                        let detail = match &p.state {
                            State::Pax(x) => format!(" st {} pax_state {} task {:?} pos.z {:.2}", x.st, x.pax_state, x.task, x.pos.z),
                            _ => String::new(),
                        };
                        log::warn!("t={:.1} person {} ({}) {:.2} m under the floor at ({:.1}, {:.1}, {:.2}), top surface {:?}{detail}", self.time, p.id, p.state.name(), f - p.position.z, p.position.x, p.position.y, p.position.z, world.walk_height(p.position.x, p.position.y));
                    }
                }
            }
        }
        self.tick_stats.0 += 1;
        self.tick_stats.1 += ms;
        self.tick_stats.2 = self.tick_stats.2.max(ms);
    }

    /// The simulation step: what everybody decides and does this frame. It does not draw;
    /// the people it makes or takes away are shown by omsi-app's view after it.
    #[allow(unused_assignments)]
    pub fn tick_inner(
        &mut self,
        dt: f32,
        world: &dyn World,
        bus: Option<&VehicleInstance>,
        traffic: Option<&TrafficSim>,
    ) -> bool {
        let mut mark = std::time::Instant::now();
        macro_rules! stage {
            ($name:expr) => {{
                let now = std::time::Instant::now();
                self.tick_stages.push(($name, (now - mark).as_secs_f64() * 1000.0));
                mark = now;
            }};
        }
        self.use_map_humans(world);
        self.time += dt as f64;
        let net = traffic.map(|t| &t.net);
        if let Some(b) = bus {
            self.center = b.position;
        } else if let Some(e) = self.eye {
            self.center = e.pos;
        }
        let generation = world.tiles_generation();
        if generation != self.tiles_seen {
            self.tiles_seen = generation;
            self.tiles_changed(world);
        }
        stage!("tiles");
        // tiles brought lanes: their pavements join the network, and stops without one look again
        if let (Some(pn), Some(n)) = (self.ped.as_mut(), net) {
            if pn.built < n.lanes.len() {
                let added = pn.extend(n);
                if added > 0 {
                    let ids: Vec<(i64, DVec3)> = self.stops.iter().filter(|(_, s)| s.lane.is_none()).map(|(k, s)| (*k, s.pos)).collect();
                    for (id, pos) in ids {
                        let lane = self.ped.as_ref().and_then(|pn| pn.nearest(n, pos, 12.0)).map(|(l, s, _)| (l, s));
                        self.stops.get_mut(&id).unwrap().lane = lane;
                    }
                }
            }
        }
        if self.ped.is_none() {
            if let Some(n) = net {
                self.ped = Some(PedNet::build(n));
                let ids: Vec<(i64, DVec3)> = self.stops.iter().map(|(k, s)| (*k, s.pos)).collect();
                for (id, pos) in ids {
                    let lane = self.ped.as_ref().and_then(|pn| pn.nearest(n, pos, 12.0)).map(|(l, s, _)| (l, s));
                    self.stops.get_mut(&id).unwrap().lane = lane;
                }
            }
        }
        stage!("pedestrian network");
        if let Some(n) = net {
            self.stroll_timer -= dt;
            if self.stroll_timer <= 0.0 {
                self.stroll_timer = 1.0;
                let c = self.center;
                self.populate_with(world, Some(n), c);
                if !self.mirror {
                    if !self.players_only {
                        self.populate_on_foot(world, n, 1.0);
                    }
                    self.populate_lan_centers(world, n);
                }
            }
        }
        stage!("populate");
        let mut buses = self.gather_buses(world, bus, traffic);
        for b in &buses {
            if b.entry_open.iter().chain(b.exit_open.iter()).any(|o| *o) {
                self.last_door_open.insert(b.id, self.time);
            }
        }
        // how the floor of each bus accelerates (for the drawing of its riders)
        if dt > 1e-4 {
            let mut motion = HashMap::new();
            for bn in buses.iter_mut() {
                let accel = match self.bus_motion.get(&bn.id) {
                    Some(&(v0, h0, a0)) => {
                        let yaw_rate = crowd::angle_diff(h0, bn.heading).to_radians() / dt as f64;
                        let raw = DVec2::new(bn.speed * yaw_rate, (bn.speed - v0) / dt as f64).clamp(DVec2::splat(-6.0), DVec2::splat(6.0));
                        a0 + (raw - a0) * (1.0 - (-(dt as f64) / 0.2).exp())
                    }
                    None => DVec2::ZERO,
                };
                bn.accel = accel;
                motion.insert(bn.id, (bn.speed, bn.heading, accel));
            }
            self.bus_motion = motion;
        }
        let buses = buses;
        self.last_buses = buses.clone();
        let bus_ix: HashMap<BusId, usize> = buses.iter().enumerate().map(|(i, b)| (b.id, i)).collect();
        // the stops: which buses stand at them (sub_61f93c), who waits there (sub_61bf94)
        let at_stops = self.register_buses(&buses, dt);
        self.claim_waiting();
        self.ride_comfort(dt, bus, &buses, &bus_ix, world);
        if !self.avatar_only {
            self.stops_tick(dt, world);
            if let Some(n) = net {
                self.runners_tick(world, n, &buses, &bus_ix);
            }
        }
        stage!("stops");
        // the passengers (sub_6ffc7c)
        let mut taken_ticket = false;
        let mut remove: Vec<usize> = Vec::new();
        self.pax_frame(dt, world, &buses, &bus_ix, &at_stops, bus, &mut taken_ticket, &mut remove);
        stage!("passengers");
        // the pedestrians: a crowd on the pavements (the cars around every player: the people
        // around the other LAN players wait for them at the kerb too)
        let mut cars: Vec<(DVec2, DVec2, f64)> = Vec::new();
        let mut blocks: Vec<Block> = Vec::new();
        if let Some(t) = traffic {
            for c in &t.cars {
                if self.far_from_players(c.vehicle.position, 320.0) {
                    continue;
                }
                let h = c.vehicle.heading.to_radians();
                let fwd = DVec2::new(h.sin(), h.cos());
                let bb = c.vehicle.ty.def.bounding_box.unwrap_or([2.0, 4.5, 1.6, 0.0, 0.0, 0.8]);
                cars.push((c.vehicle.position.truncate(), fwd * c.state.speed as f64, bb[1] as f64 * 0.5));
                if !matches!(c.vehicle.ty.def.kind, omsi_vehicle::VehicleKind::Other(3)) {
                    let o = crate::collision::Obb::from_box(bb, c.vehicle.position, c.vehicle.heading);
                    blocks.push(Block { center: o.center, half: o.half, heading: o.heading, vel: fwd * c.state.speed as f64 });
                    for t in &c.vehicle.trailers {
                        let tb = t.ty.def.bounding_box.unwrap_or([2.5, 7.0, 3.0, 0.0, 0.0, 1.5]);
                        let o = crate::collision::Obb::from_box(tb, t.position, t.heading);
                        let th = t.heading.to_radians();
                        blocks.push(Block { center: o.center, half: o.half, heading: o.heading, vel: DVec2::new(th.sin(), th.cos()) * c.state.speed as f64 });
                    }
                }
            }
        }
        for o in world.parked_boxes().iter() {
            if self.anchors().any(|c| (o.center - c.truncate()).length() < 320.0) {
                blocks.push(Block { center: o.center, half: o.half, heading: o.heading, vel: DVec2::ZERO });
            }
        }
        if let Some(pb) = bus_ix.get(&BusId::Player).map(|&i| &buses[i]) {
            cars.push((pb.pos.truncate(), pb.fwd() * pb.speed, pb.half.y));
            for t in &pb.trailers {
                let h = t.heading.to_radians();
                cars.push((t.pos.truncate(), DVec2::new(h.sin(), h.cos()) * pb.speed, t.half.y));
            }
            blocks.extend(pb.blocks());
        }
        let mut wants: Vec<Want> = Vec::with_capacity(self.people.len());
        for i in 0..self.people.len() {
            self.people[i].t_state += dt;
            let w = if self.people[i].puppet.is_some() || matches!(self.people[i].state, State::Pax(_)) {
                Want::stand(None, Activity::Stand)
            } else if self.people[i].remote {
                self.mirror_want(i, &buses)
            } else {
                self.decide(i, dt, world, net, traffic, &cars, &mut remove)
            };
            wants.push(w);
        }
        // a standing vehicle in the way: wait, then go round it
        for i in 0..self.people.len() {
            let p = &self.people[i];
            if remove.contains(&i) || p.puppet.is_some() || p.remote || !matches!(p.state, State::Strolling(_)) {
                continue;
            }
            let want = wants[i].vel;
            let speed = want.length();
            if speed < 0.2 {
                self.people[i].car_wait = 0.0;
                continue;
            }
            let ahead = p.position.truncate() + want / speed * 0.9;
            let in_way = blocks.iter().any(|b| {
                b.vel.length() < 0.5 && b.near(ahead, BODY_OUTSIDE + 0.15) && {
                    let (q, inside) = b.closest(ahead);
                    inside || (ahead - q).length() < BODY_OUTSIDE + 0.15
                }
            });
            if !in_way {
                self.people[i].car_wait = 0.0;
                continue;
            }
            self.people[i].car_wait += dt;
            if self.people[i].car_wait > 8.0 {
                self.people[i].detour = self.people[i].detour.max(4.0);
                self.people[i].car_wait = 0.0;
            } else if self.people[i].detour <= 0.0 {
                wants[i].vel = DVec2::ZERO;
            }
        }
        // the crowd of the pavements (passengers outside stand in it as they are)
        let mut walkers: Vec<Walker> = Vec::with_capacity(self.people.len());
        let mut who: Vec<usize> = Vec::with_capacity(self.people.len());
        for (i, p) in self.people.iter().enumerate() {
            if remove.contains(&i) || p.puppet.is_some() || p.remote || p.place != Place::Ground {
                continue;
            }
            let fixed = matches!(p.state, State::Pax(_));
            let w = &wants[i];
            walkers.push(Walker {
                pos: p.position.truncate(),
                vel: p.vel,
                radius: BODY_OUTSIDE,
                want: w.vel,
                give: w.give,
                space: 0,
                fixed,
                ghost: p.ghost > 0.0,
                corridor: if p.detour > 0.0 { None } else { w.corridor },
            });
            who.push(i);
        }
        let near_blocks: Vec<Block> = blocks.into_iter().filter(|b| walkers.iter().any(|w| b.near(w.pos, 25.0))).collect();
        let mut g = walkers.clone();
        crowd::step(&mut g, &near_blocks, &CrowdParams::default(), dt as f64);
        let mut ground: Vec<(usize, Walker)> = g.into_iter().enumerate().collect();
        self.keep_out_of_walls(world, &who, &mut ground);
        let mut moved = vec![false; self.people.len()];
        for (k, w) in ground {
            let i = who[k];
            if matches!(self.people[i].state, State::Pax(_)) {
                continue;
            }
            moved[i] = true;
            self.apply(i, &w, &wants[i], dt, world, net, &buses, &bus_ix);
        }
        for i in 0..self.people.len() {
            if !moved[i] && !matches!(self.people[i].state, State::Pax(_)) {
                self.carry(i, dt, &buses, &bus_ix, wants[i].face);
            }
        }
        self.animate(dt, world, &buses, &bus_ix);
        remove.sort_unstable();
        remove.dedup();
        for i in remove.into_iter().rev() {
            self.release(i);
            let p = self.people.swap_remove(i);
            if debug_pax() {
                log::info!("t={:.1} pax {} taken away ({}){}", self.time, p.label(), p.state.name(), if self.seen(p.position) { " IN SIGHT" } else { "" });
            }
            self.retire(&p);
        }
        self.give_ticket = false;
        stage!("pedestrians");
        taken_ticket
    }

    /// Everybody's animation this frame (sub_626ae8): the passengers from what their task
    /// says (`PAX_State`, speed, the room height, the seat, the hand and the head), the
    /// pedestrians from their walk.
    pub fn animate(&mut self, dt: f32, world: &dyn World, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>) {
        let dt_ms = dt * 1000.0;
        for i in 0..self.people.len() {
            if let Some(pp) = self.people[i].puppet {
                if pp.mode == PuppetMode::Avatar {
                    self.animate_avatar(i, dt, world, buses, bus_ix);
                }
                continue;
            }
            let p = &self.people[i];
            let (input, footstep) = match &p.state {
                State::Pax(x) => {
                    let bn = x.inside.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k]));
                    // a point of the bus in the person's own frame, Direct3D's axes
                    let own = |q: Vec3| -> Vec3 {
                        let v = q.as_dvec3() - x.pos;
                        let (s, c) = x.yaw.sin_cos();
                        let local = Vec3::new((v.x * c - v.y * s) as f32, (v.x * s + v.y * c) as f32, v.z as f32);
                        crate::human_omsi::d3d(local)
                    };
                    let reach = (x.reach && x.inside.is_some()).then(|| own(x.reach_at));
                    let look = match (x.look_driver, bn) {
                        (true, Some(b)) => b.cabin.data.driver_positions.first().map(|d| own(Vec3::from(d.pos) + Vec3::Z * 0.65)),
                        _ => None,
                    };
                    let kind = x.pax_state.round().clamp(0.0, 2.0) as u8;
                    let pack = match (x.step_pack, bn) {
                        (Some(k), Some(b)) => b.cabin.step_packs.get(k).cloned().map(|pk| (b.id, pk)),
                        _ => None,
                    };
                    (
                        AnimInput {
                            kind,
                            speed: x.speed,
                            moved: x.moved,
                            room_height: x.room,
                            seat_height: x.seat_h,
                            reach,
                            look,
                            smooth: x.smooth,
                            dt_ms,
                        },
                        pack,
                    )
                }
                _ => {
                    let v = p.vel.length() as f32;
                    (
                        AnimInput {
                            kind: if v > 0.05 { 1 } else { 0 },
                            speed: v,
                            moved: v * dt,
                            room_height: pax::OUTSIDE_ROOM,
                            dt_ms,
                            ..Default::default()
                        },
                        None,
                    )
                }
            };
            let p = &mut self.people[i];
            let ev = p.anim.advance(&p.ty.omsi, &input);
            // a foot down inside a vehicle: the link's step sound (outside there are none)
            if let (true, Some((bus, pack))) = (ev.step, footstep) {
                self.footfalls.push(Footfall { position: p.position, inside: true, own_bus: bus == BusId::Player, pack: Some(pack) });
            }
        }
    }
}
