//! Passengers at the stops: the buses standing there, who boards which, the waiting places.

use super::*;

impl PeopleSim {
    /// The passenger of person `i`, if it is one.
    pub fn pax(&self, i: usize) -> Option<&Pax> {
        match &self.people[i].state {
            State::Pax(p) => Some(p),
            _ => None,
        }
    }

    pub fn pax_mut(&mut self, i: usize) -> Option<&mut Pax> {
        match &mut self.people[i].state {
            State::Pax(p) => Some(p),
            _ => None,
        }
    }

    /// The stops as the buses see them this frame (sub_61f93c / sub_61f238), and the
    /// odometers of the buses.
    pub fn register_buses(&mut self, buses: &[BusNow], dt: f32) -> HashMap<BusId, BusAtStops> {
        let mut out: HashMap<BusId, BusAtStops> = HashMap::new();
        for s in self.stops.values_mut() {
            s.buses.clear();
        }
        let left = LEFT_HAND.load(std::sync::atomic::Ordering::Relaxed);
        let _ = left;
        let mut ids: Vec<i64> = self.stops.keys().copied().collect();
        ids.sort_unstable();
        for bn in buses {
            let km = self.odometer.entry(bn.id).or_insert(0.0);
            *km += bn.speed.abs() * dt as f64 / 1000.0;
            let mut reg = BusAtStops::default();
            let mut request_distance = 500.0;
            for id in &ids {
                let s = &self.stops[id];
                let d = bn.pos - s.pos;
                let dist = d.length();
                // (the stop to request is wanted only by a bus whose next stop nobody knows; a
                // stop beyond that and out of reach costs nothing more, as before)
                let wants_request = bn.next_stop.is_none() && dist < request_distance;
                if !wants_request && !(dist < 60.0) {
                    continue;
                }
                let sh = s.heading.to_radians();
                let (s_fwd, s_right) = (DVec2::new(sh.sin(), sh.cos()), DVec2::new(sh.cos(), -sh.sin()));
                let same_way = bn.fwd().dot(s_fwd) > 0.0;
                // Without a route, use the nearest stop ahead, not the one just left.
                if wants_request && same_way && d.truncate().dot(s_fwd) <= 25.0 {
                    request_distance = dist;
                    reg.request_next = Some(RequestStop {
                        id: *id,
                        name: s.name.clone(),
                        alias: s.alias.clone(),
                        pos: s.pos,
                    });
                }
                if !(dist < 60.0) {
                    continue;
                }
                reg.near.push(*id);
                // A scheduled AI already knows its next stop. Nearby platforms must not
                // overwrite that identity according to their object-id sort order.
                // Keep the original geometric fallback for the player and unknown trips.
                let matches_trip = !matches!(bn.id, BusId::Ai(_))
                    || bn.next_stop.as_ref().is_none_or(|next| next.id == *id);
                if same_way && matches_trip {
                    reg.next = Some(*id);
                }
                // a bus not in service, or at its own terminus, empties and takes nobody
                // (0x61f3e3); one in free drive only lets its riders off
                match at_stop(*id, s, bn.terminus.as_deref(), &bn.takes) {
                    AtStop::Empties => {
                        reg.all_exit = true;
                        continue;
                    }
                    AtStop::Passes => continue,
                    AtStop::Serves => {}
                }
                if same_way {
                    let lateral = d.truncate().dot(s_right);
                    let along = d.truncate().dot(s_fwd);
                    let in_box = lateral.abs() < 2.0 && along.abs() < (s.length as f64 - 5.0).max(0.0);
                    self.stops.get_mut(id).unwrap().buses.push((bn.id, in_box));
                }
            }
            // a timetable bus boarding at a stop none of these know: its riders get off at
            // its timetable's stop (#1593)
            if reg.next.is_none() {
                reg.next = bn.served;
            }
            out.insert(bn.id, reg);
        }
        out
    }

    /// sub_61c33c: the bus at stop `stop` person `i` gets into, and why: with a line record,
    /// the nearest of the buses listed whose terminus goes there - else the nearest whose
    /// duty takes them there (`fit`); without, the first listed.
    pub fn bus_for(&self, i: usize, stop: i64, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>) -> Option<(BusId, Fit)> {
        let s = self.stops.get(&stop)?;
        let p = self.pax(i)?;
        match p.line.and_then(|k| s.lines.get(k)) {
            None => s.buses.first().map(|b| (b.0, Fit::Any)),
            Some((_, termini)) => best_bus(s.buses.iter().filter_map(|(id, _)| {
                let bn = bus_ix.get(id).map(|k| &buses[*k])?;
                if bn.cabin.entries.is_empty() {
                    return None;
                }
                let f = fit(stop, s, p.dest.as_deref(), termini, bn.terminus.as_deref()?, &bn.takes)?;
                Some((*id, f, (bn.pos - self.people[i].position).length()))
            })),
        }
    }

    /// Whether the bus stands in the stop's box (the flag of its entry, sub_61ee18).
    pub fn in_stop_box(&self, stop: i64, bus: BusId) -> bool {
        self.stops.get(&stop).is_some_and(|s| s.buses.iter().any(|b| b.0 == bus && b.1))
    }

    pub fn listed_at(&self, stop: i64, bus: BusId) -> bool {
        self.stops.get(&stop).is_some_and(|s| s.buses.iter().any(|b| b.0 == bus))
    }

    /// sub_7e910c: a free place of the bus (none free: nobody gets on). `off`: the places
    /// its scripts have switched off (#721), which nobody takes. Unlike OMSI's pick among
    /// all the free places, a seat comes first: a standing place only by `stand_chance`
    /// or once the seats are full.
    pub fn reserve_place(&mut self, bus: BusId, places: &[Seat], off: &[bool]) -> Option<usize> {
        let n = places.len();
        let seats = self.seats.entry(bus).or_insert_with(|| vec![false; n]);
        if seats.len() < n {
            seats.resize(n, false);
        }
        let (sit, stand): (Vec<usize>, Vec<usize>) =
            (0..n).filter(|k| !seats[*k] && !off.get(*k).copied().unwrap_or(false)).partition(|k| places[*k].seated);
        let free = if stand.is_empty() || (!sit.is_empty() && self.rand_f() as f32 >= self.stand_chance) { sit } else { stand };
        if free.is_empty() {
            return None;
        }
        let k = free[(self.rand() as usize) % free.len()];
        self.seats.get_mut(&bus).unwrap()[k] = true;
        Some(k)
    }

    /// sub_5ce4e0: stamp (stamper_prop) or buy (ticketbuy_prop) at a bus that has a
    /// validator / a cash desk, else nothing to do; the ticket bought (sub_5ce2dc).
    pub fn decide_pax_ticket(&mut self, i: usize, bn: &BusNow) -> (u8, u8) {
        let Some(tp) = self.tickets.clone() else { return (TICKET_NONE, 0) };
        let mut r = self.rand_f() as f32;
        if !bn.cabin.stampers.is_empty() {
            if r < tp.stamper_prop {
                return (TICKET_STAMP, 0);
            }
            r -= tp.stamper_prop;
        }
        // (the sale also needs the option on, `boarding` not "walk")
        if bn.cabin.sale.is_some() && r < tp.ticketbuy_prop && !self.boarding.eq_ignore_ascii_case("walk") {
            let age = self.people[i].age;
            if let Some(t) = self.pick_ticket(age) {
                return (TICKET_BUY, (t + 1).min(255) as u8);
            }
            return (TICKET_BUY, 0);
        }
        (TICKET_NONE, 0)
    }

    pub fn free_spot(&mut self, stop: i64, k: usize) {
        if let Some(t) = self.stops.get_mut(&stop).and_then(|s| s.taken.get_mut(k)) {
            *t = false;
        }
    }

    /// sub_61c8d8: a free waiting place of the stop, at random.
    pub fn take_spot(&mut self, stop: i64) -> Option<usize> {
        // Stops a few metres apart (both sides of a bus station's platform, a stop and its
        // copy for another line) find the same objects' places, and each kept its own list of
        // who stands where (as Omsi.exe's 0x61c8d8 does), so two or three people stood in
        // one another. A place somebody of another stop stands on is not free (nor one of the
        // stop's own a few centimetres from a taken one: objects placed twice).
        let s = self.stops.get(&stop)?;
        let (pos, reach) = (s.pos, 40.0_f64.max(s.length as f64 + 15.0) * 2.0);
        let elsewhere: Vec<DVec3> = self
            .stops
            .iter()
            .filter(|(_, o)| (o.pos - pos).length() < reach)
            .flat_map(|(_, o)| o.spots.iter().zip(&o.taken).filter(|(_, t)| **t).map(|(sp, _)| sp.pos))
            .collect();
        let s = self.stops.get(&stop)?;
        let free: Vec<usize> = s
            .taken
            .iter()
            .enumerate()
            .filter(|(k, t)| !**t && !elsewhere.iter().any(|q| (*q - s.spots[*k].pos).truncate().length() < 0.4))
            .map(|(k, _)| k)
            .collect();
        if free.is_empty() {
            return None;
        }
        let k = free[(self.rand() as usize) % free.len()];
        self.stops.get_mut(&stop).unwrap().taken[k] = true;
        Some(k)
    }
}
