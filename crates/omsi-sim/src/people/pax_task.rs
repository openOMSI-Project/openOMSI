//! Passengers' tasks (sub_62e42c): setting one up and what each task does every frame.

use super::*;

impl PeopleSim {
    /// sub_62e42c: a new task and what it starts with.
    pub fn set_task(&mut self, i: usize, t: Task, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>, world: &dyn World) {
        if self.pax(i).is_none_or(|p| p.task == t) {
            return;
        }
        if debug_pax() {
            log::info!("t={:.1} pax {} {} -> {}", self.time, self.people[i].label(), self.pax(i).unwrap().task.name(), t.name());
        }
        let seatheight = self.people[i].ty.def.seat_height;
        self.pax_mut(i).unwrap().task = t;
        match t {
            Task::WaitingForBus => {
                let (stop, spot) = {
                    let p = self.pax(i).unwrap();
                    (p.stop, p.spot)
                };
                let sp = stop.zip(spot).and_then(|(s, k)| self.stops.get(&s).and_then(|s| s.spots.get(k)).cloned());
                let p = self.pax_mut(i).unwrap();
                p.st = 0;
                match sp {
                    Some(sp) if sp.height != 0.0 => {
                        p.seat_h = sp.height;
                        p.pos = sp.pos - DVec3::Z * seatheight as f64;
                        p.yaw = sp.face.to_radians();
                        p.pax_state = 2.0;
                    }
                    Some(sp) => {
                        p.pos = sp.pos;
                        p.yaw = sp.face.to_radians();
                        p.pax_state = 0.0;
                    }
                    None => p.pax_state = 0.0,
                }
            }
            Task::ToBus => {
                let (stop, spot) = {
                    let p = self.pax(i).unwrap();
                    (p.stop, p.spot)
                };
                if let (Some(s), Some(k)) = (stop, spot) {
                    self.free_spot(s, k);
                }
                let gather = stop.and_then(|s| self.stops.get(&s)).map(|s| s.gather);
                let p = self.pax_mut(i).unwrap();
                p.spot = None;
                if let Some(g) = gather {
                    p.target = g;
                }
                p.target_bus = false;
                p.st = 1;
                p.pax_state = 1.0;
            }
            Task::WalkingToBus => {
                let (stop, spot) = {
                    let p = self.pax(i).unwrap();
                    (p.stop, p.spot)
                };
                if let (Some(s), Some(k)) = (stop, spot) {
                    self.free_spot(s, k);
                }
                self.pax_mut(i).unwrap().spot = None;
                self.choose_entry(i, buses, bus_ix);
                let p = self.pax_mut(i).unwrap();
                p.target_bus = true;
                p.st = 1;
                p.pax_state = 1.0;
            }
            Task::InBusToPlace => {
                let bus = self.pax(i).unwrap().bus;
                let Some(bn) = bus.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k])) else { return };
                let km = self.odometer.get(&bn.id).copied().unwrap_or(0.0);
                let detailed = bn.id == BusId::Player;
                let all = bn.cabin.all_points();
                let p = self.pax_mut(i).unwrap();
                p.pax_state = 1.0;
                p.km_start = km;
                p.door = None;
                // into the bus's frame
                let local = bn.to_local(p.pos);
                p.yaw = wrap(p.yaw - bn.heading_at(local).to_radians());
                p.pos = local.as_dvec3();
                p.inside = Some(bn.id);
                p.target_bus = true;
                p.pt = bn.cabin.omsi_nearest(local, &all, false, false, None, None);
                p.st = 5;
                if !detailed {
                    // a bus not the player's: at the place at once (sub_62a358 + task 7)
                    self.set_task(i, Task::SittingInBus, buses, bus_ix, world);
                    return;
                }
                let ticket = p.ticket;
                match ticket {
                    TICKET_STAMP => {
                        p.stamper = bn.cabin.nearest_stamper(local);
                        p.pt_target = p.stamper.and_then(|k| bn.cabin.stampers[k].0);
                    }
                    TICKET_BUY => p.pt_target = bn.cabin.in_group(vec![bn.cabin.sale.and_then(|s| s.0)], bn.cabin.group_at(p.pt))[0],
                    _ => self.route_to_place(i, bn),
                }
                let p = self.pax_mut(i).unwrap();
                if p.pt_target.is_none() {
                    // (no path point at the device: straight on to the place)
                    p.ticket = TICKET_NONE;
                    self.route_to_place(i, bn);
                }
            }
            Task::InBusToExit => {
                let bus = self.pax(i).unwrap().bus.or(self.pax(i).unwrap().inside);
                let Some(bn) = bus.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k])) else { return };
                // the stop button
                if bn.id == BusId::Player {
                    self.stop_request = true;
                }
                let seat = self.pax(i).unwrap().seat;
                let all = bn.cabin.all_points();
                let from = seat.and_then(|k| bn.cabin.seats.get(k)).map(|s| s.pos).unwrap_or(self.pax(i).unwrap().pos.as_vec3());
                let start = bn.cabin.omsi_nearest(from, &all, false, true, None, None);
                let exits = bn.cabin.exit_points();
                let p = self.pax_mut(i).unwrap();
                p.pax_state = 1.0;
                p.pt = start;
                if let Some(q) = start.and_then(|k| bn.cabin.graph.points.get(k)) {
                    p.pos = q.as_dvec3();
                }
                let here = p.pos.as_vec3();
                // the nearest exit (sub_62a49c / sub_62a5a8), of the sections they are in
                let exits = bn.cabin.in_group(exits, bn.cabin.group_at(start));
                p.pt_target = bn.cabin.omsi_nearest(here, &exits, false, false, None, None);
                p.door = p.pt_target.and_then(|t| exits.iter().position(|e| *e == Some(t)));
                if let Some(d) = p.door {
                    if let Some((_, x)) = self.pax_req.get_mut(&bn.id) {
                        if let Some(r) = x.get_mut(d) {
                            *r = true;
                        }
                    }
                }
                let p = self.pax_mut(i).unwrap();
                p.st = 5;
                if let Some(k) = p.seat.take() {
                    self.free_seat(bn.id, k);
                }
            }
            Task::WalkingToBusstop => {
                let r = self.rand_f() as f32;
                let stop = self.pax(i).unwrap().stop;
                {
                    let p = self.pax_mut(i).unwrap();
                    p.short = false;
                    p.door = None;
                    p.ride_km = r * 19.0 + 1.0;
                }
                // a free waiting place (sub_61fed0)
                let spot = match (stop, self.pax(i).unwrap().spot) {
                    (_, Some(k)) => Some(k),
                    (Some(s), None) => self.take_spot(s),
                    _ => None,
                };
                let sp = stop.zip(spot).and_then(|(s, k)| self.stops.get(&s).and_then(|s| s.spots.get(k)).cloned());
                let stop_pos = stop.and_then(|s| self.stops.get(&s)).map(|s| s.pos);
                // (no place free: at the stop's point - spread along the kerb by who they
                // are, or everybody without a place stood in one another there)
                let along = ((self.people[i].id % 7) as f64 - 3.0) * 0.7;
                let fwd = stop.and_then(|s| self.stops.get(&s)).map(|s| { let h = s.heading.to_radians(); DVec3::new(h.sin(), h.cos(), 0.0) }).unwrap_or(DVec3::ZERO);
                let stop_pos = stop_pos.map(|q| q + fwd * along);
                let p = self.pax_mut(i).unwrap();
                p.spot = spot;
                p.target_bus = false;
                match sp {
                    Some(sp) => {
                        // a seat: in front of it, the hip at its height (0x62e5d1)
                        let mut tgt = sp.pos;
                        if sp.height != 0.0 {
                            tgt.z = tgt.z.min((sp.pos.z - sp.height as f64).max(stop_pos.map(|s| s.z).unwrap_or(tgt.z)));
                        }
                        p.target = tgt;
                        p.target_yaw = sp.face.to_radians();
                    }
                    None => {
                        if let Some(sp) = stop_pos {
                            p.target = sp;
                        }
                        p.target_yaw = 0.0;
                    }
                }
                p.st = 1;
            }
            Task::SittingInBus => {
                let bus = self.pax(i).unwrap().inside;
                let Some(bn) = bus.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k])) else { return };
                let seat = self.pax(i).unwrap().seat.and_then(|k| bn.cabin.seats.get(k)).cloned();
                let p = self.pax_mut(i).unwrap();
                p.st = 0;
                if let Some(s) = seat {
                    if s.seated {
                        p.seat_h = s.height;
                        p.pos = (s.pos - Vec3::Z * seatheight).as_dvec3();
                        p.pax_state = 2.0;
                    } else {
                        p.pos = s.pos.as_dvec3();
                        p.pax_state = 0.0;
                    }
                    p.yaw = (s.rot as f64).to_radians();
                }
                p.room = OUTSIDE_ROOM;
                p.reach = false;
                p.look_driver = false;
            }
            Task::Nothing => {}
        }
    }

    /// sub_625b98: the entry to walk to, every frame on the way (the nearest open one or
    /// one with a button; one selling tickets for a buyer), and its index for the request.
    pub fn choose_entry(&mut self, i: usize, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>) {
        let p = self.pax(i).unwrap().clone();
        let Some(bn) = p.bus.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k])) else { return };
        let here = match p.inside {
            Some(_) => p.pos.as_vec3(),
            None => bn.to_local(p.pos),
        };
        // (the doors of the sections of the place reserved)
        let group = p.seat.and_then(|k| bn.cabin.seats.get(k)).map(|s| s.group);
        let list = bn.cabin.in_group(bn.cabin.entry_points(), group);
        let flags = bn.cabin.entry_flags();
        let open: Vec<bool> = (0..list.len()).map(|k| bn.entry_open.get(k).copied().unwrap_or(false)).collect();
        let pt = bn.cabin.omsi_nearest(here, &list, p.ticket == TICKET_BUY, false, Some(&flags), Some(&open));
        let p = self.pax_mut(i).unwrap();
        if let Some(q) = pt.and_then(|k| bn.cabin.graph.points.get(k)) {
            p.target = q.as_dvec3();
            p.target_bus = true;
        }
        p.door = pt.and_then(|t| list.iter().position(|e| *e == Some(t)));
    }

    /// sub_62a628: along the paths to the place reserved.
    pub fn route_to_place(&mut self, i: usize, bn: &BusNow) {
        let all = bn.cabin.all_points();
        let seat = self.pax(i).unwrap().seat.and_then(|k| bn.cabin.seats.get(k)).map(|s| s.pos);
        let p = self.pax_mut(i).unwrap();
        if let Some(s) = seat {
            p.pt_target = bn.cabin.omsi_nearest(s, &all, false, true, None, None);
        }
        p.st = 5;
        p.smooth = false;
    }

    /// The task part of the tick (sub_62a6a0 from 0x62b984).
    #[allow(clippy::too_many_arguments)]
    pub fn pax_task(
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
        let p = self.pax(i).unwrap().clone();
        let bn = p.bus.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k]));
        match p.task {
            Task::WaitingForBus => {
                let Some(stop) = p.stop else { return };
                let Some(b) = self.bus_for(i, stop, buses, bus_ix) else {
                    if super::debug_pax() && (self.time * 2.0).fract() < (dt as f64 * 2.0) {
                        let s = &self.stops[&stop];
                        let listed: Vec<String> = s.buses.iter().map(|(id, inbox)| format!("{id:?} box {inbox} shows {:?}", bus_ix.get(id).and_then(|k| buses[*k].terminus.clone()))).collect();
                        let termini = p.line.and_then(|k| s.lines.get(k)).map(|l| l.1.iter().cloned().collect::<Vec<_>>());
                        log::info!("t={:.1} pax {} at stop {stop} for {:?} (line {:?} termini {:?}): no bus; listed {:?}", self.time, self.people[i].label(), p.dest, p.line, termini, listed);
                    }
                    return;
                };
                let (b, why) = b;
                let Some(bn) = bus_ix.get(&b).map(|k| &buses[*k]) else { return };
                // (a bus they gave up on at its shut doors: once it opens one)
                if self.pax(i).unwrap().shunned == Some(b) {
                    if !bn.entry_open.iter().any(|o| *o) {
                        return;
                    }
                    self.pax_mut(i).unwrap().shunned = None;
                }
                self.pax_mut(i).unwrap().bus = Some(b);
                // still rolling in, or standing in the stop's box: to the gather point
                if bn.speed.abs() <= 2.0 && !self.in_stop_box(stop, b) {
                    return;
                }
                if super::debug_pax() {
                    log::info!("t={:.1} pax {} at stop {stop} for {:?}: bus {b:?} showing {:?} ({why:?})", self.time, self.people[i].label(), p.dest, bn.terminus);
                }
                self.set_task(i, Task::ToBus, buses, bus_ix, world);
            }
            Task::ToBus => {
                let Some(stop) = p.stop else { return };
                if let Some(bn) = bn {
                    if bn.speed.abs() < 3.0 && self.in_stop_box(stop, bn.id) {
                        if let Some(k) = self.reserve_place(bn.id, &bn.cabin.seats, &bn.places_off) {
                            let (tk, id) = self.decide_pax_ticket(i, bn);
                            let price = self.tickets.as_ref().and_then(|t| t.tickets.get(id.saturating_sub(1) as usize)).map(|t| t.value).unwrap_or(0.0);
                            let pp = self.pax_mut(i).unwrap();
                            pp.seat = Some(k);
                            pp.ticket = tk;
                            pp.ticket_id = id;
                            pp.price = if id > 0 { price } else { 0.0 };
                            self.set_task(i, Task::WalkingToBus, buses, bus_ix, world);
                        }
                    }
                }
                self.pax_mut(i).unwrap().short = true;
                // the bus is gone from the stop: back to a waiting place
                let gone = match self.pax(i).unwrap().bus {
                    Some(b) => !self.listed_at(stop, b),
                    None => true,
                };
                if gone && self.pax(i).unwrap().task == Task::ToBus {
                    self.set_task(i, Task::WalkingToBusstop, buses, bus_ix, world);
                }
            }
            Task::WalkingToBus => self.task_to_bus(i, buses, bus_ix, world),
            Task::InBusToPlace => self.task_to_place(i, dt, buses, bus_ix, world, player_bus, taken_ticket),
            Task::InBusToExit => self.task_to_exit(i, buses, bus_ix, at_stops, world, remove),
            Task::WalkingToBusstop => {
                if p.st == 3 {
                    self.set_task(i, Task::WaitingForBus, buses, bus_ix, world);
                } else {
                    self.pax_mut(i).unwrap().pax_state = 1.0;
                }
            }
            Task::SittingInBus => {
                let Some(b) = p.inside else { return };
                let reg = at_stops.get(&b).cloned().unwrap_or_default();
                let km = self.odometer.get(&b).copied().unwrap_or(0.0);
                if let Some(bn) = bn {
                    if let Some(stop) = bn.next_stop.as_ref().or(reg.request_next.as_ref()) {
                        let force_exit = reg.all_exit
                            && !bn.terminus.as_ref().is_some_and(|name| stop.is_named(name));
                        if !force_exit && p.dest.as_ref().is_some_and(|dest| stop.is_named(dest)) {
                            let departing = bn.speed.abs() > 0.1
                                && !bn.entry_open.iter().chain(&bn.exit_open).any(|open| *open);
                            let arrived = reg.next == Some(stop.id)
                                && bn.speed.abs() < 1.0
                                && bn.exit_open.iter().any(|open| *open);
                            if arrived
                                || self
                                    .pax_mut(i)
                                    .unwrap()
                                    .wants_stop_at(stop, bn.pos, departing)
                            {
                                self.set_task(i, Task::InBusToExit, buses, bus_ix, world);
                            }
                            // The boarding range must not override a later random point
                            // on short legs, including those ending at the terminus.
                            return;
                        }
                    }
                }
                if reg.all_exit {
                    self.set_task(i, Task::InBusToExit, buses, bus_ix, world);
                    return;
                }
                if let (Some(next), Some(dest)) = (reg.next, p.dest.as_ref()) {
                    let name = self.stops.get(&next).map(|s| s.name.trim().to_string()).unwrap_or_default();
                    if self.stops.get(&next).is_some_and(|s| s.is_named(dest)) {
                        self.set_task(i, Task::InBusToExit, buses, bus_ix, world);
                        return;
                    }
                    if p.alt.as_ref().is_some_and(|a| a.trim() == name) && !p.alt_seen {
                        let r = self.rand_f() as f32;
                        let pp = self.pax_mut(i).unwrap();
                        pp.alt_seen = true;
                        pp.km_start = km;
                        pp.ride_km = (pp.alt_m / 1000.0) * (0.2 + 0.6 * r);
                    }
                }
                let p = self.pax(i).unwrap();
                if (p.alt_seen || p.dest.is_none()) && p.km_start + (p.ride_km as f64) < km {
                    self.set_task(i, Task::InBusToExit, buses, bus_ix, world);
                }
            }
            Task::Nothing => {}
        }
    }

    /// Task 3 (sub_62a6a0 case 3): to the door and in.
    pub fn task_to_bus(&mut self, i: usize, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>, world: &dyn World) {
        let p = self.pax(i).unwrap().clone();
        let Some(bn) = p.bus.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k])) else {
            self.set_task(i, Task::WalkingToBusstop, buses, bus_ix, world);
            return;
        };
        let door_x = p.door.and_then(|d| bn.cabin.entries.get(d)).map(|e| e.inside.x).unwrap_or(0.0);
        let open = p.door.map(|d| bn.entry_open.get(d).copied().unwrap_or(false)).unwrap_or(false);
        {
            let pp = self.pax_mut(i).unwrap();
            pp.clamp = true;
            pp.clamp_left = door_x < 0.0;
            pp.clamp_x = if pp.clamp_left { bn.centre.x - bn.half.x - 0.5 } else { bn.centre.x + bn.half.x + 0.5 };
            pp.clamp_open = open;
        }
        // a shut door is asked for, from the moment they stand at it
        if p.st == 3 || p.st == 2 {
            if let Some(d) = p.door {
                if let Some((e, _)) = self.pax_req.get_mut(&bn.id) {
                    if let Some(r) = e.get_mut(d) {
                        *r = true;
                    }
                }
            }
        }
        if p.seat.is_none() {
            self.set_task(i, Task::WalkingToBusstop, buses, bus_ix, world);
            return;
        }
        // At a shut door that stays shut - a bus on its layover, at the end of its trip, or
        // standing in the stop's box without serving it - nobody stands pressed against it
        // for good: after `DOOR_GIVE_UP` the place is given back and the person waits at the
        // stop again, for this bus only once it opens a door (they stood at its doors for
        // ten minutes and more).
        let at_shut_door = p.st == 2 && !bn.entry_open.iter().any(|o| *o);
        let (since, give_up) = shut_door_wait(p.door_since, self.time, at_shut_door);
        self.pax_mut(i).unwrap().door_since = since;
        if give_up {
            if let Some(k) = p.seat {
                self.free_seat(bn.id, k);
            }
            let pp = self.pax_mut(i).unwrap();
            pp.seat = None;
            pp.door_since = None;
            pp.shunned = Some(bn.id);
            if super::debug_pax() {
                log::info!("t={:.1} pax {} gives up at the shut doors of {:?}", self.time, self.people[i].label(), bn.id);
            }
            self.set_task(i, Task::WalkingToBusstop, buses, bus_ix, world);
            return;
        }
        let stop = p.stop;
        let ok = bn.speed.abs() < 3.0
            && stop.is_some_and(|s| self.in_stop_box(s, bn.id))
            && !bn.cabin.graph.points.is_empty();
        if ok {
            if p.st != 3 {
                self.choose_entry(i, buses, bus_ix);
                let open = self.pax(i).unwrap().door.map(|d| bn.entry_open.get(d).copied().unwrap_or(false)).unwrap_or(false);
                let pp = self.pax_mut(i).unwrap();
                pp.short = !open;
                pp.st = 1;
                pp.pax_state = 1.0;
                return;
            }
            // in the doorway: the driver is greeted (the player's bus)
            if bn.id == BusId::Player {
                self.greet_or_complain(i, bn);
            }
            self.set_task(i, Task::InBusToPlace, buses, bus_ix, world);
            return;
        }
        // the bus pulls away again: the place is given back
        if let Some(k) = p.seat {
            self.free_seat(bn.id, k);
        }
        self.pax_mut(i).unwrap().seat = None;
        if bn.speed.abs() >= 3.0 {
            self.set_task(i, Task::ToBus, buses, bus_ix, world);
        } else {
            self.set_task(i, Task::WalkingToBusstop, buses, bus_ix, world);
        }
    }

    /// Task 4 (case 4): the validator, the cash desk, and on to the place.
    #[allow(clippy::too_many_arguments)]
    pub fn task_to_place(
        &mut self,
        i: usize,
        dt: f32,
        buses: &[BusNow],
        bus_ix: &HashMap<BusId, usize>,
        world: &dyn World,
        player_bus: Option<&VehicleInstance>,
        taken_ticket: &mut bool,
    ) {
        let p = self.pax(i).unwrap().clone();
        let Some(bn) = p.inside.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k])) else { return };
        if p.st == 7 {
            if p.ticket < TICKET_STAMP {
                self.set_task(i, Task::SittingInBus, buses, bus_ix, world);
                return;
            }
            if bn.id != BusId::Player {
                let pp = self.pax_mut(i).unwrap();
                pp.sub = 0;
                pp.ticket = TICKET_NONE;
            } else {
                let pp = self.pax_mut(i).unwrap();
                pp.st = 9;
                pp.smooth = true;
                if pp.ticket == TICKET_STAMP {
                    if let Some(&(_, dev)) = pp.stamper.and_then(|k| bn.cabin.stampers.get(k)) {
                        pp.target = dev.as_dvec3();
                        pp.target_bus = true;
                        pp.reach_at = dev;
                    }
                    pp.timer = 1.0;
                    pp.reach = true;
                    pp.sub = 1;
                } else {
                    pp.sub = 3;
                    if let Some(m) = bn.cabin.money_point {
                        pp.target = m.as_dvec3();
                        pp.target_bus = true;
                        pp.reach_at = m;
                    }
                }
            }
        }
        let p = self.pax(i).unwrap().clone();
        if p.ticket == TICKET_STAMP {
            if p.sub == 1 && p.timer < 0.5 {
                // the validator stamps
                self.stamped.push(bn.id);
                let pp = self.pax_mut(i).unwrap();
                pp.sub = 2;
                pp.reach = false;
            } else if p.sub == 2 && p.timer <= 0.0 {
                self.route_to_place(i, bn);
                let pp = self.pax_mut(i).unwrap();
                pp.pt = pp.stamper.and_then(|k| bn.cabin.stampers.get(k)).and_then(|s| s.0);
                pp.ticket = TICKET_NONE;
                pp.sub = 0;
            }
        } else if p.ticket == TICKET_BUY {
            self.desk_sale(i, dt, bn, player_bus, taken_ticket);
        }
    }

    /// Task 5 (case 5): to the exit, out.
    pub fn task_to_exit(&mut self, i: usize, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>, at_stops: &HashMap<BusId, BusAtStops>, world: &dyn World, remove: &mut Vec<usize>) {
        let p = self.pax(i).unwrap().clone();
        let Some(b) = p.inside else { return };
        let Some(bn) = bus_ix.get(&b).map(|k| &buses[*k]) else { return };
        let reg = at_stops.get(&b).cloned().unwrap_or_default();
        if bn.speed.abs() >= 1.0 {
            self.pax_mut(i).unwrap().timer = 1.0;
        }
        let door_open = p.door.map(|d| bn.exit_open.get(d).copied().unwrap_or(false)).unwrap_or(false);
        let may_leave = door_open && (reg.next.is_some() || p.complaint == 3);
        self.pax_mut(i).unwrap().short = !may_leave;
        let out = p.st == 7 && bn.speed.abs() < 1.0 && may_leave;
        if !out {
            if reg.next.is_none() {
                if bn.id == BusId::Player {
                    self.stop_request = true;
                }
                return;
            }
            if p.timer < 0.0 {
                // the bus stands: the nearest exit that is open now (0x62d6b1 passes the
                // exits' open states, +0x6e0: a shut door is skipped, none open gives the
                // first). Without them the nearest door was taken again, open or shut, and
                // people walked on to a shut front door with the others open (#493).
                // Once a second: with the timer left run out, the way was found afresh every
                // frame from the nearest point, and whoever had left a point was pulled back
                // to it - the people coming down from the upper deck never got off the stairs.
                self.pax_mut(i).unwrap().timer = 1.0;
                let all = bn.cabin.all_points();
                let pp = self.pax_mut(i).unwrap();
                let here = pp.pos.as_vec3();
                // (the exits of the sections they are in)
                let group = bn.cabin.group_at(pp.pt.or_else(|| bn.cabin.omsi_nearest(here, &all, false, false, None, None)));
                let exits = bn.cabin.in_group(bn.cabin.exit_points(), group);
                let open: Vec<bool> = (0..exits.len()).map(|k| bn.exit_open.get(k).copied().unwrap_or(false)).collect();
                // (with every exit still shut - the bus rolling in - the nearest exit, not the
                // first of the list: the people of the whole saloon gathered at one door while
                // the others stood empty, #1149)
                let any_open = open.iter().zip(&exits).any(|(o, e)| *o && e.is_some());
                let target = bn.cabin.omsi_nearest(here, &exits, false, false, None, any_open.then_some(open.as_slice()));
                if pp.st == 5 {
                    // walking: on from the point walked to, towards the new door (Omsi.exe
                    // changes only the target and the door)
                    pp.pt_target = target;
                } else if target != pp.pt_target || pp.st != 7 {
                    pp.pt = bn.cabin.omsi_nearest(here, &all, false, false, None, None);
                    pp.pt_target = target;
                    pp.st = 5;
                }
                pp.door = pp.pt_target.and_then(|t| exits.iter().position(|e| *e == Some(t)));
            }
            if bn.id == BusId::Player {
                self.stop_request = true;
            }
            if let Some(d) = self.pax(i).unwrap().door {
                if let Some((_, x)) = self.pax_req.get_mut(&b) {
                    if let Some(r) = x.get_mut(d) {
                        *r = true;
                    }
                }
            }
            return;
        }
        // out of the bus: into the world, on along the pavement (task 8)
        let Some((w, h)) = self.pax_world(&p, buses, bus_ix) else { return };
        let stop = reg.next;
        let pp = self.pax_mut(i).unwrap();
        pp.inside = None;
        pp.pos = w;
        pp.yaw = h.to_radians();
        if debug_pax() {
            log::info!("t={:.1} pax {} gets off at stop {:?} by exit {:?}", self.time, self.people[i].label(), stop, p.door);
        }
        self.walk_street(i, w, h, stop, world, remove);
    }

    /// sub_626818 / task 8: on as a pedestrian along the pavement from the stop - or gone
    /// when there is none.
    pub fn walk_street(&mut self, i: usize, at: DVec3, heading: f64, stop: Option<i64>, world: &dyn World, remove: &mut Vec<usize>) {
        let _ = world;
        self.walk_street_plain(i, at, heading, stop, remove)
    }

    pub fn walk_street_plain(&mut self, i: usize, at: DVec3, heading: f64, stop: Option<i64>, remove: &mut Vec<usize>) {
        let lane = stop.and_then(|s| self.stops.get(&s)).and_then(|s| s.lane);
        let p = &mut self.people[i];
        p.position = at;
        p.heading = heading;
        p.place = Place::Ground;
        p.interior = 0.0;
        p.vel = DVec2::ZERO;
        let _ = &remove;
        match (lane, self.ped.as_ref()) {
            (Some((l, s)), Some(_)) => {
                let leg = Leg { lane: l, a: s, b: s };
                p.state = State::Strolling(PedWalk::new(vec![leg], true, 0.0));
            }
            _ => p.state = State::Standing,
        }
    }
}
