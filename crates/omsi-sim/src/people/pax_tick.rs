//! Passengers, frame by frame (sub_6ffc7c, sub_62a6a0): the movement by state and what
//! stops them.

use super::*;

impl PeopleSim {
    /// The world position and heading of a passenger.
    pub fn pax_world(&self, p: &Pax, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>) -> Option<(DVec3, f64)> {
        match p.inside {
            None => Some((p.pos, p.yaw.to_degrees())),
            Some(b) => {
                let bn = bus_ix.get(&b).map(|k| &buses[*k])?;
                let l = p.pos.as_vec3();
                Some((bn.world(l), bn.heading_at(l) + p.yaw.to_degrees()))
            }
        }
    }

    /// Everybody's passenger tick of this frame, in the order of the people (sub_6ffc7c).
    #[allow(clippy::too_many_arguments)]
    pub fn pax_frame(
        &mut self,
        dt: f32,
        world: &dyn World,
        buses: &[BusNow],
        bus_ix: &HashMap<BusId, usize>,
        at_stops: &HashMap<BusId, BusAtStops>,
        player_bus: Option<&VehicleInstance>,
        taken_ticket: &mut bool,
        remove: &mut Vec<usize>,
    ) {
        // the requests the buses' scripts read this frame
        for r in self.entry_req.iter_mut().chain(self.exit_req.iter_mut()) {
            *r = false;
        }
        let mut ai_req: HashMap<BusId, (Vec<bool>, Vec<bool>)> = HashMap::new();
        for bn in buses {
            ai_req.insert(bn.id, (vec![false; bn.cabin.entries.len()], vec![false; bn.cabin.exits.len()]));
        }
        self.pax_req = ai_req;
        for i in 0..self.people.len() {
            if self.pax(i).is_none() || remove.contains(&i) {
                continue;
            }
            self.pax_tick(i, dt, world, buses, bus_ix, at_stops, player_bus, taken_ticket, remove);
        }
        // (where every passenger stands: in a bus's frame, or the world's - and the people
        // walking the pavement, among them a rider who has just stepped off, still on the
        // step until they are clear of the door. Not those who stand where they got off
        // with no pavement to go on along: they would hold the door open for good.)
        let at: Vec<(Option<BusId>, DVec3)> = self
            .people
            .iter()
            .filter_map(|p| match &p.state {
                State::Pax(x) => Some((x.inside, x.pos)),
                State::Strolling(_) => Some((None, p.position)),
                _ => None,
            })
            .collect();
        let busy: HashMap<BusId, (Vec<bool>, Vec<bool>)> = buses.iter().map(|bn| (bn.id, doorways_taken(bn, &at))).collect();
        if debug_pax() {
            for (b, (e, x)) in &busy {
                let before = self.pax_busy.get(b);
                for (kind, now, was) in [("entry", e, before.map(|o| &o.0)), ("exit", x, before.map(|o| &o.1))] {
                    for (k, on) in now.iter().enumerate() {
                        if was.and_then(|w| w.get(k)).copied().unwrap_or(false) != *on {
                            log::info!("t={:.1} bus {b:?} {kind} {k}: {}", self.time, if *on { "somebody in the doorway" } else { "the doorway is free" });
                        }
                    }
                }
            }
        }
        self.pax_busy = busy;
        // the places' own occupancy variables (#721): the riders at their places
        let sitting: Vec<(BusId, usize)> = self
            .people
            .iter()
            .filter_map(|p| match &p.state {
                State::Pax(x) if x.task == Task::SittingInBus => Some((x.inside?, x.seat?)),
                _ => None,
            })
            .collect();
        self.pax_places = buses.iter().map(|bn| (bn.id, places_taken(bn, &sitting))).collect();
        // the player's bus reads its requests from `entry_req` / `exit_req`
        if let Some((e, x)) = self.pax_req.get(&BusId::Player) {
            self.entry_req = e.clone();
            self.exit_req = x.clone();
        }
        if let Some((e, x)) = self.pax_busy.get(&BusId::Player) {
            self.entry_busy = e.clone();
            self.exit_busy = x.clone();
        }
        self.ai_requests.clear();
        for (b, (e, x)) in &self.pax_req {
            if let BusId::Ai(id) = b {
                let (entry_busy, exit_busy) = self.pax_busy.get(b).cloned().unwrap_or_default();
                let places = self.pax_places.get(b).cloned().unwrap_or_default();
                self.ai_requests.push((*id, DoorWants { entry_req: e.clone(), exit_req: x.clone(), entry_busy, exit_busy, places }));
            }
        }
        // timetable buses wait while people still get on or off (0x7d9e8b - 0x7d9f5e):
        // somebody of this bus walking in it to an exit, or walking up to its doors from
        // the stop the bus serves - the traffic checks the stop (`hold_boarding`). People
        // still on their way to the gather point do not hold it: they walk up to the doors
        // as soon as the bus has a place for them, and with the bus full they stood there
        // and kept it at the stop with its doors open for good (#767)
        for bn in buses {
            let BusId::Ai(id) = bn.id else { continue };
            if bn.speed.abs() > 0.5 {
                continue;
            }
            let mut any_exit = false;
            let mut stops: Vec<i64> = Vec::new();
            for p in &self.people {
                let State::Pax(x) = &p.state else { continue };
                match holds_bus(x, bn.id) {
                    Some(None) => any_exit = true,
                    Some(Some(s)) if !stops.contains(&s) => stops.push(s),
                    _ => {}
                }
            }
            if any_exit {
                self.holds.push((id, None, 2.5));
            }
            for s in stops {
                self.holds.push((id, Some(s), 2.5));
            }
        }
    }

    /// One person's tick (sub_62a6a0 without the street walk).
    #[allow(clippy::too_many_arguments)]
    pub fn pax_tick(
        &mut self,
        i: usize,
        dt: f32,
        world: &dyn World,
        buses: &[BusNow],
        bus_ix: &HashMap<BusId, usize>,
        at_stops: &HashMap<BusId, BusAtStops>,
        player_bus: Option<&VehicleInstance>,
        taken_ticket: &mut bool,
        remove: &mut Vec<usize>,
    ) {
        let dt_ms = dt * 1000.0;
        // sub_62a258: the bus is gone - nothing more to do with it
        {
            let p = self.pax_mut(i).unwrap();
            if let Some(b) = p.bus {
                if !bus_ix.contains_key(&b) {
                    p.bus = None;
                }
            }
        }
        // inside a bus that is gone (a timetable bus left the map): gone with it
        if let Some(b) = self.pax(i).unwrap().inside {
            if !bus_ix.contains_key(&b) {
                remove.push(i);
                return;
            }
        }
        {
            let p = self.pax_mut(i).unwrap();
            if p.timer > 0.0 {
                p.timer -= dt;
            }
            if p.dist_timer > 0.0 {
                p.dist_timer -= p.moved;
            }
        }
        // the toll of a bad ride eases off as the bus goes on (0x62d86c: 0.2 a kilometre)
        {
            let speed = self.pax(i).unwrap().inside.and_then(|b| bus_ix.get(&b)).map(|k| buses[*k].speed.abs() as f32);
            let p = self.pax_mut(i).unwrap();
            match speed {
                Some(v) => p.discomfort = (p.discomfort - v * dt / 5000.0).max(0.0),
                None => p.discomfort = 0.0,
            }
        }
        self.runner_step(i, dt, world, buses, bus_ix);
        self.pax_move(i, dt, dt_ms, world, buses, bus_ix);
        self.pax_task(i, dt, world, buses, bus_ix, at_stops, player_bus, taken_ticket, remove);
        // (got off: a pedestrian now)
        let Some(p) = self.pax(i).cloned() else { return };
        // (0x62d75b) the stop they boarded at is forgotten once the bus has left it
        if matches!(p.task, Task::InBusToPlace | Task::InBusToExit | Task::SittingInBus) {
            if let (Some(stop), Some(b)) = (p.stop, p.bus) {
                if !at_stops.get(&b).is_some_and(|r| r.near.contains(&stop)) {
                    let p = self.pax_mut(i).unwrap();
                    p.stop = None;
                }
            }
        }
        // where the person is drawn
        if let Some((w, h)) = self.pax_world(&p, buses, bus_ix) {
            let person = &mut self.people[i];
            person.position = w;
            person.heading = h;
            match p.inside {
                Some(b) => {
                    person.place = Place::Bus(b, p.pos.as_vec3());
                    person.lheading = p.yaw.to_degrees();
                    if let Some(bn) = bus_ix.get(&b).map(|k| &buses[*k]) {
                        person.tilt = bn.tilt_at(p.pos.as_vec3());
                        person.interior = bn.interior;
                    }
                }
                None => {
                    person.place = Place::Ground;
                    person.interior = 0.0;
                }
            }
            let yaw = p.yaw;
            person.vel = if p.st == 1 || p.st == 5 {
                DVec2::new(yaw.sin(), yaw.cos()) * p.speed as f64
            } else {
                DVec2::ZERO
            };
        }
    }

    /// The movement part of the tick (sub_62a6a0, 0x62ad0b - 0x62b966).
    pub fn pax_move(&mut self, i: usize, dt: f32, dt_ms: f32, world: &dyn World, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>) {
        let p0 = self.pax(i).unwrap().clone();
        let bn_in = p0.inside.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k]));
        let bn_t = p0.bus.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k]));
        // the path point walked to is the target
        let mut target = p0.target;
        let mut target_bus = p0.target_bus;
        // (kept as the target, +0x5bd: waiting short of the point, state 6, goes on facing
        // it - with the target of before kept instead, a seat or the stop's gather point in
        // another frame, the people waiting at a shut exit were lifted 40 m up in the bus
        // and stood stacked there for good, #709)
        let mut walked_to: Option<DVec3> = None;
        if p0.st == 5 {
            if let (Some(pt), Some(bn)) = (p0.pt, bn_in) {
                if let Some(q) = bn.cabin.graph.points.get(pt) {
                    target = q.as_dvec3();
                    target_bus = true;
                    walked_to = Some(target);
                }
            }
        }
        // walking to a door from outside: keep off the bus side (0x62ad81)
        if p0.clamp && target_bus {
            if let Some(bn) = bn_t {
                let level = if p0.clamp_open && p0.inside.is_none() {
                    let l = bn.to_local(p0.pos);
                    (l.y as f64 - target.y).abs() <= 1.0
                } else {
                    false
                };
                if !level {
                    if p0.clamp_left {
                        target.x = target.x.min(p0.clamp_x);
                    } else {
                        target.x = target.x.max(p0.clamp_x);
                    }
                }
            }
        }
        // the target in the person's own frame
        let tgt = match (target_bus, p0.inside) {
            (true, Some(_)) => target,
            (true, None) => match bn_t {
                Some(bn) => bn.world(target.as_vec3()),
                None => target,
            },
            (false, Some(_)) => match bn_in {
                Some(bn) => bn.to_local(target).as_dvec3(),
                None => target,
            },
            (false, None) => target,
        };
        let mut d = tgt - p0.pos;
        let mut room = p0.room;
        let mut step_pack = p0.step_pack;
        if p0.inside.is_none() {
            d.z = 0.0;
            step_pack = None;
            room = OUTSIDE_ROOM;
        }
        let dist = d.length() as f32;
        let mut st = p0.st;
        let mut pt = p0.pt;
        let mut link = p0.link;
        match st {
            5 => {
                if p0.pt == p0.pt_target && dist <= 0.7 && p0.short {
                    st = 6;
                } else if dist <= 0.1 {
                    let next = match (p0.pt, p0.pt_target, bn_in) {
                        (Some(a), Some(b), Some(bn)) => bn.cabin.route_next(a, b),
                        _ => None,
                    };
                    match next {
                        Some((n, l)) => {
                            pt = Some(n);
                            link = Some(l);
                            if let Some(bn) = bn_in {
                                step_pack = bn.cabin.link_pack.get(l).copied().flatten();
                                room = bn.cabin.link_room.get(l).copied().unwrap_or(2.0);
                            }
                        }
                        None => st = 7,
                    }
                }
            }
            1 => {
                if dist <= 0.7 && p0.short {
                    st = 2;
                } else if dist <= 0.1 {
                    st = 3;
                }
            }
            6 => {
                if !p0.short {
                    st = 5;
                }
            }
            2 => {
                if !p0.short {
                    st = 1;
                }
            }
            _ => {}
        }
        // the people in the way (sub_626860)
        let (mut block, free_r, free_l) = if st == 1 || st == 5 { self.pax_blockers(i, buses, bus_ix) } else { (0, true, true) };
        let p = self.pax_mut(i).unwrap();
        // Inside a bus, people going opposite ways along the aisle or the stairs stood face to
        // face for good (the whole upper deck of a double-decker on its way out, the people
        // coming up stopped on the stairs): held up for two seconds, they squeeze past for a
        // second and a half, as the people on the pavements do.
        if p.inside.is_some() {
            if p.squeeze > 0.0 {
                p.squeeze -= dt;
                block = 0;
            } else if block == 2 {
                p.jam += dt;
                if p.jam > 2.0 {
                    p.jam = 0.0;
                    p.squeeze = 1.5;
                    block = 0;
                }
            } else {
                p.jam = 0.0;
            }
        }
        if let Some(t) = walked_to {
            p.target = t;
            p.target_bus = true;
        }
        p.st = st;
        p.pt = pt;
        p.link = link;
        p.room = room;
        p.step_pack = step_pack;
        p.clamp = false;
        p.clamp_open = false;
        p.clamp_left = false;
        p.clamp_x = -1e9;
        p.moved = 0.0;
        p.speed_des = 0.0;
        p.block = block;
        p.free_r = free_r;
        p.free_l = free_l;
        let mut head_des = p.yaw;
        let mut slope = f64::INFINITY;
        if st == 1 || st == 5 {
            head_des = yaw_of(d.truncate());
            if p.inside.is_some() || p.pax_state == 2.0 {
                let h = d.truncate().length();
                slope = if h > 0.0 { d.z / h } else { f64::INFINITY };
            }
            p.speed_des = if block < 2 { p.walk_speed } else { 0.0 };
        } else if st == 3 || st == 7 {
            head_des = p.target_yaw;
        } else if st == 9 {
            head_des = yaw_of(d.truncate());
        }
        if st == 0 {
            // standing: still on the ground under the feet, as Omsi.exe asks for it every
            // tick in every state but turning (0x62b852 -> 0x7aec3c, not when seated); the
            // waiting people stood at the height of their [passpos]'s object - a shelter
            // on the terrain - 25-35 cm down in the platform
            if p.inside.is_none() && p.pax_state != 2.0 {
                if let Some(g) = world.walk_height_near(p.pos.x, p.pos.y, p.pos.z) {
                    p.pos.z = g;
                }
            }
            return;
        }
        let mut dh = wrap(head_des - p.yaw);
        if st == 1 && p.free_r && p.block == 2 {
            dh = -1.745;
        }
        if st != 9 {
            if dh.abs() > 1.0 {
                p.speed = 0.0;
            }
            let diff = p.speed_des - p.speed;
            p.speed += diff.signum() * diff.abs().min(5.0 * dt_ms / 1000.0);
        }
        let turn = dh.signum() * dh.abs().min(dt_ms as f64 / 150.0);
        p.yaw = wrap(p.yaw + turn);
        if st == 9 {
            return;
        }
        let mut moved = p.speed * dt_ms / 1000.0;
        let mut step = DVec3::new(d.x, d.y, 0.0);
        let len = step.length() as f32;
        if len <= moved {
            moved = len;
        } else if len > 0.0 {
            step *= (moved / len) as f64;
        }
        p.moved = moved;
        if p.inside.is_some() || p.pax_state == 2.0 {
            step.z = if slope.is_finite() { moved as f64 * slope } else { d.z };
        } else {
            let at = p.pos + step;
            step.z = match world.walk_height_near(at.x, at.y, p.pos.z) {
                Some(g) => g - p.pos.z,
                None => 0.0,
            };
        }
        p.pos += step;
        if p.inside.is_none() && p.pax_state != 2.0 {
            if let Some(stop) = p.stop {
                if let Some(s) = self.stops.get(&stop) {
                    let floor = s.pos.z;
                    let p = self.pax_mut(i).unwrap();
                    p.pos.z = p.pos.z.max(floor);
                }
            }
        }
        let _ = dt;
    }

    /// sub_626860: whether somebody within 0.6 m stands in the way.
    pub fn pax_blockers(&self, i: usize, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>) -> (u8, bool, bool) {
        let me = self.pax(i).unwrap();
        let Some((my_pos, my_head)) = self.pax_world(me, buses, bus_ix) else { return (0, true, true) };
        let fs = {
            let h = my_head.to_radians();
            DVec2::new(h.sin(), h.cos())
        };
        let (mut block, mut free_r, mut free_l) = (0u8, true, true);
        for (j, o) in self.people.iter().enumerate() {
            if j == i || o.puppet.is_some() {
                continue;
            }
            // who counts: the passengers of the same bus or stop, and the people on foot
            // when this one has no bus yet
            let (o_task, o_st, o_bus, o_stop, o_sub, o_block) = match &o.state {
                State::Pax(x) => (Some(x.task), x.st, x.bus, x.stop, x.sub, x.block),
                // a pedestrian on a path: state 8 with a path, no bus, no stop
                State::Strolling(_) => (None, 8, None, None, 0, 0),
                _ => continue,
            };
            if o_st == 0 || o_st == 3 {
                continue;
            }
            if o_task == Some(Task::ToBus) && me.task == Task::WalkingToBus {
                continue;
            }
            if !(o_bus == me.bus || (o_stop.is_some() && o_stop == me.stop)) {
                continue;
            }
            let d = o.position - my_pos;
            if d.z >= 2.0 {
                continue;
            }
            let d2 = d.truncate();
            let dist = d2.length();
            if !(dist < 0.6) {
                continue;
            }
            let dn = if dist > 0.0 { d2 / dist } else { DVec2::ZERO };
            let fo = {
                let h = o.heading.to_radians();
                DVec2::new(h.sin(), h.cos())
            };
            if fs.dot(dn) < 0.0 {
                if block == 0 {
                    block = 1;
                }
                continue;
            }
            // (D3DXVec3Cross(fs, d).y in the left-handed frame)
            let side = fs.y * dn.x - fs.x * dn.y < 0.0;
            let facing = fo.dot(fs) < 0.2 || dn.dot(fo) <= 0.0;
            if facing && o_sub == 0 {
                if o_block == 0 {
                    block = block.max(2);
                    if side {
                        free_r = false;
                    } else {
                        free_l = false;
                    }
                }
                continue;
            }
            block = block.max(3);
        }
        (block, free_r, free_l)
    }
}
