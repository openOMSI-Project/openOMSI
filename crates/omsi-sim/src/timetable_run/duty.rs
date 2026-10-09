//! The player's duty: its planned trips and the progress along them.

use super::*;

impl StopDir {
    pub(crate) fn takes(self, fwd: glam::DVec2) -> bool {
        if self.inbound.is_none() && self.outbound.is_none() {
            return true;
        }
        [self.inbound, self.outbound].into_iter().flatten().any(|d| fwd.dot(d) >= DIR_COS)
    }
}

/// The unit vector of the ground plane a bus heading `deg` drives along (degrees clockwise
/// from north, as `VehicleInstance::heading`).
pub(crate) fn forward_of(deg: f64) -> glam::DVec2 {
    let h = deg.to_radians();
    glam::DVec2::new(h.sin(), h.cos())
}

/// How far apart two stops of a trip must stand before the line between them is taken as
/// the way the trip runs between them (m).
pub(crate) const DIR_REACH: f64 = 20.0;

/// How far off the way a trip runs through a stop a bus may head and still be taken as
/// running that way: the cosine of the angle, 60 degrees either side.
pub(crate) const DIR_COS: f64 = 0.5;

impl PlannedTrip {
    /// Give every stop the way the trip runs through it: in from the stop before, out to
    /// the stop after.
    pub fn set_dirs(&mut self) {
        let p: Vec<Option<glam::DVec3>> = self.stops.iter().map(|s| s.position).collect();
        let dir = |a: Option<glam::DVec3>, b: Option<glam::DVec3>| -> Option<glam::DVec2> {
            let v = (b? - a?).truncate();
            (v.length() >= DIR_REACH).then(|| v.normalize())
        };
        for (i, s) in self.stops.iter_mut().enumerate() {
            s.dir = StopDir {
                inbound: i.checked_sub(1).and_then(|k| dir(p[k], p[i])),
                outbound: p.get(i + 1).and_then(|b| dir(p[i], *b)),
            };
        }
    }
}

/// Where timetable stop `k` (called `name`) stands in the IBIS's own stop list of route
/// `route` (an index into the depot file's `info_busstop_lists`): the stop of that name
/// nearest `k`. What `IBIS_busstop` has to be for the IBIS to show that stop.
pub fn ibis_stop_index(hof: &omsi_vehicle::Hof, route: usize, name: &str, k: usize) -> Option<usize> {
    let stop = (name.trim().to_lowercase(), stop_words(name));
    let list = hof.info_busstop_lists.get(route)?;
    // (spelt as `pick_route` compares the names: "Kirchweg" is the depot file's "F_Kirchweg")
    list.iter()
        .enumerate()
        .filter(|(_, id)| same_stop(&ident_names(hof, id), &stop))
        .map(|(i, _)| i)
        .min_by_key(|i| i.abs_diff(k))
}

/// Route index the unit's stop list uses: stock `IBIS_RouteIndex`, or Aachen's `ibox_routenindex`.
pub(crate) fn script_route_index(bus: &crate::VehicleInstance) -> Option<usize> {
    bus.var("IBIS_RouteIndex")
        .or_else(|| bus.var("ibox_routenindex"))
        .filter(|r| *r >= 0.0)
        .map(|r| r.round() as usize)
}

/// Align `ibox_busstop` with timetable stop `tt` (named `name`).
///
/// (Not `IBIS_busstop`: no stock script reads the timetable's stop index, the stock IBIS
/// steps on its own keys only - set to the row before, every stock IBIS on a duty ran a
/// stop behind.) Units such as Aachen's ibox do `(L.L.ibox_busstop) 1 +` when `GetTTBusstopIndex` changes,
/// so on a forward step the value is left at the previous route index and that `+ 1` lands
/// on the new one. A backward step (or a resume) writes the index itself and freezes the
/// ibox's "last TT index" so the frame does not announce again.
pub(crate) fn sync_script_busstop(bus: &mut crate::VehicleInstance, tt: usize, name: &str, prev_tt: i32) {
    let Some(hof) = bus.host.hof.clone() else { return };
    let Some(route) = script_route_index(bus) else { return };
    let Some(idx) = ibis_stop_index(&hof, route, name, tt) else { return };
    let forward = (tt as i32) > prev_tt;
    let value = if forward { idx.saturating_sub(1) } else { idx };
    for var in ["ibox_busstop"] {
        if bus.var(var).is_some() {
            bus.set_var(var, value as f32);
        }
    }
    if !forward {
        if bus.var("ibox_TTBusstopIndexLAST").is_some() {
            bus.set_var("ibox_TTBusstopIndexLAST", tt as f32);
        }
    }
}

/// The trip the player picked: "HH:MM" - the first trip leaving at that minute or later -
/// or its number in the tour (1 = the first).
pub fn chosen_trip(trips: &[PlannedTrip], pick: &str) -> Option<usize> {
    if let Some((h, m)) = pick.split_once(':') {
        let (h, m) = (h.trim().parse::<f64>().ok()?, m.trim().parse::<f64>().ok()?);
        let at = h * 3600.0 + m * 60.0;
        return trips.iter().position(|t| t.departure >= at - 30.0);
    }
    let n = pick.parse::<usize>().ok()?;
    (n >= 1 && n <= trips.len()).then(|| n - 1)
}

/// The trip of a duty that fits the time of day: the one under way, else the next to leave
/// (the last one once all are over).
pub fn starting_trip(trips: &[PlannedTrip], now: f64) -> usize {
    trips.iter().position(|t| t.end > now).unwrap_or_else(|| {
        // every trip over for today: a tour after midnight (the night line 13N's runs from
        // 0:49) picked in the evening is tonight's, and starts at its first trip
        match trips.first() {
            Some(first) if first.departure + DAY - now < DAY / 2.0 => 0,
            _ => trips.len().saturating_sub(1),
        }
    })
}

/// GetTTTerminusIndex as Omsi.exe answers it: the first depot terminus whose name is the
/// trip's terminus (the second [trip] line), else -1.
pub(crate) fn tt_terminus_index(hof: Option<&omsi_vehicle::hof::Hof>, terminus: &str) -> i32 {
    hof.and_then(|h| h.termini.iter().position(|t| t.texture_id == terminus)).map_or(-1, |i| i as i32)
}

/// A trip run's number (`PlayerDuty::trip_run`): never the same twice in a game.
fn next_run() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl PlayerDuty {
    /// A duty of `trips` (where they begin in the tour: `first_trip`) that starts with trip
    /// `trip_index`, before the bus has been placed on it; `picked` when the player picked
    /// the trip.
    pub fn new(line: String, tour: String, trips: Vec<PlannedTrip>, trip_index: usize, first_trip: usize, picked: bool) -> PlayerDuty {
        PlayerDuty {
            line,
            tour,
            trips,
            trip_index,
            first_trip,
            next_stop: 0,
            at_stop: false,
            arrived_late: None,
            done: false,
            served_terminus: None,
            left_late: None,
            held_back: false,
            placed: false,
            trip_changed: false,
            skipped: None,
            run: next_run(),
            finished: None,
            reopened: None,
            picked,
            first_update: None,
            heading: 0.0,
            position: None,
        }
    }

    pub fn trip(&self) -> &PlannedTrip {
        &self.trips[self.trip_index]
    }

    pub fn trip_done(&self) -> bool {
        self.done
    }

    /// True while the bus stands at the next stop.
    pub fn at_stop(&self) -> bool {
        self.at_stop
    }

    /// How late (s, negative = early) the bus left the last stop it served on this trip;
    /// None while it has not left one.
    pub fn left_late(&self) -> Option<f64> {
        self.left_late
    }

    /// Service/depot legs have no public line and use the HOF's
    /// `Betriebsfahrt` destination. They remain part of the duty, but the
    /// player's IBIS should use the next public leg while the bus is waiting.
    pub fn trip_for_ibis(&self) -> (&PlannedTrip, usize) {
        let current = self.trip();
        if !current.line.trim().is_empty() {
            return (
                current,
                self.next_stop.min(current.stops.len().saturating_sub(1)),
            );
        }
        let next = self
            .trips
            .iter()
            .skip(self.trip_index + 1)
            .find(|trip| !trip.line.trim().is_empty());
        match next {
            Some(trip) => (trip, 0),
            None => (
                current,
                self.next_stop.min(current.stops.len().saturating_sub(1)),
            ),
        }
    }

    /// Whether the current trip changed since the last call (the IBIS wants the new one).
    pub fn take_trip_change(&mut self) -> bool {
        std::mem::take(&mut self.trip_changed)
    }

    /// The stops the bus passed without stopping since the last call: how many, the stop it
    /// was due at and the one it is at now (numbers in the trip, from 1). Lua plugins get
    /// it as the `stops_skipped` event.
    pub fn take_skipped(&mut self) -> Option<(usize, usize, usize)> {
        self.skipped.take()
    }

    /// The trip that ended since the last call, and how (see [`Finished`]); Lua plugins get
    /// it as the `trip_done` event. (A saved situation restored at the last stop is over, but
    /// did not end now; neither did a trip passed over unbegun.)
    pub fn take_finished(&mut self) -> Option<Finished> {
        self.finished.take()
    }

    /// The current trip's run: a number no other trip had in this game, a new one every
    /// time the duty goes on to a trip (or a duty is taken).
    pub fn trip_run(&self) -> u64 {
        self.run
    }

    /// The run (`trip_run`) whose trip a page reopened after its end was taken: it ends again
    /// later, and that end counts too.
    pub fn take_reopened(&mut self) -> Option<u64> {
        self.reopened.take()
    }

    /// The current trip ended `how` (the first ending since the last `take_finished` counts).
    fn finish(&mut self, how: TripEnd) {
        if self.finished.is_none() {
            self.finished = Some(Finished { index: self.trip_index, how, run: self.run });
        }
    }

    /// How late the bus arrived at the stop it stands at (s; negative: early), None while it
    /// stands at none: the journey's log notes the arrival (`journey`).
    pub fn arrived(&self) -> Option<f64> {
        self.arrived_late.filter(|_| self.at_stop)
    }

    /// Places of stops the timetable did not know (their tiles were not loaded when the duty
    /// was made): the navigator reads the whole map.
    pub fn learn_places(&mut self, places: &HashMap<i64, glam::DVec3>) {
        for trip in &mut self.trips {
            for s in &mut trip.stops {
                if s.position.is_none() {
                    s.position = places.get(&s.object_id).copied();
                }
            }
            // (a stop that only now has a place gives its neighbours their direction)
            trip.set_dirs();
        }
    }

    /// The places of the stops ahead as the map has them now (`World::object_positions`):
    /// a stop whose tile was not loaded when the duty began - and that no index placed -
    /// gets its place once the tile comes, and one placed roughly (an object hung on
    /// another) its exact one. A stop without a place was never reached: the next stop
    /// stayed on it for the rest of the trip (#975) and the map left it out (#1014).
    pub fn learn_loaded(&mut self, positions: &HashMap<i64, (glam::DVec3, [f64; 3])>) {
        // (the trip under way and the next: the later ones learn theirs when they come)
        let from = self.trip_index;
        for trip in self.trips[from..].iter_mut().take(2) {
            let mut changed = false;
            for s in &mut trip.stops {
                if let Some((p, _)) = positions.get(&s.object_id) {
                    if s.position != Some(*p) {
                        s.position = Some(*p);
                        changed = true;
                    }
                }
            }
            if changed {
                trip.set_dirs();
            }
        }
    }

    /// Time to drive from `pos` to `to` (s), roughly: roads are longer than the straight
    /// line, a bus in town makes some 25 km/h, and it takes a minute or two to get going.
    pub(crate) fn approach_time(pos: glam::DVec3, to: glam::DVec3) -> f64 {
        let d = (to - pos).truncate().length();
        if d < AT_STOP {
            return 0.0;
        }
        d * 1.35 / 7.0 + 60.0
    }

    /// Where the bus starts: at the stop of the trip under way it stands at; else with the
    /// first trip of the tour whose first stop it can reach before that trip leaves (a
    /// duty picked for 08:00 with the bus in the depot used to start with the trip under
    /// way at 08:00, led the driver to whatever stop that trip was due at next - halfway
    /// along the line - and ran late from the first second). When no trip of the tour
    /// can be reached in time any more, the last one is driven from its first stop that
    /// can (else its first stop), late as that is.
    pub(crate) fn place(&mut self, pos: glam::DVec3, now: f64) {
        let trip = &self.trips[self.trip_index];
        // of two stops within AT_STOP of the bus - the two sides of a street on a circular
        // route - the one the bus drives the way the trip runs through it, else the nearer
        let fwd = forward_of(self.heading);
        let near: Vec<(usize, f64)> = trip
            .stops
            .iter()
            .enumerate()
            .filter_map(|(k, s)| s.position.map(|p| (k, (p - pos).length())))
            .filter(|(_, d)| *d < AT_STOP)
            .collect();
        let nearest = |v: Vec<(usize, f64)>| v.into_iter().min_by(|a, b| a.1.total_cmp(&b.1));
        let near = nearest(near.iter().copied().filter(|&(k, _)| trip.stops[k].dir.takes(fwd)).collect()).or_else(|| nearest(near));
        if near.is_none() && !self.picked {
            let reachable = (self.trip_index..self.trips.len()).find(|&k| {
                let t = &self.trips[k];
                let first = t.stops.first().and_then(|s| s.position);
                // (a first stop nobody knows the place of: ten minutes)
                let need = first.map(|p| Self::approach_time(pos, p)).unwrap_or(600.0);
                t.departure >= now + need
            });
            match reachable {
                Some(k) if k != self.trip_index => {
                    log::info!(
                        "duty: trip {} ({}) cannot be reached in time from here; starting with trip {} ({}) at {}",
                        self.trip_index + 1,
                        trip.name,
                        k + 1,
                        self.trips[k].name,
                        hhmm(self.trips[k].departure)
                    );
                    self.set_trip(k);
                    self.trip_changed = true;
                }
                Some(_) => {}
                None => {
                    let t = &self.trips[self.trip_index];
                    self.next_stop = t
                        .stops
                        .iter()
                        .position(|s| s.stops && s.position.map(|p| s.arr >= now + Self::approach_time(pos, p)).unwrap_or(false))
                        .unwrap_or(0);
                    log::info!(
                        "duty: no trip of the tour can be reached in time; trip {} ({}) from stop {} '{}'",
                        self.trip_index + 1,
                        t.name,
                        self.next_stop,
                        t.stops.get(self.next_stop).map(|s| s.name.as_str()).unwrap_or("")
                    );
                    return;
                }
            }
        }
        let trip = &self.trips[self.trip_index];
        if trip.departure >= now {
            log::info!(
                "duty: trip {} ({}) leaves {} at {:.0} s",
                self.trip_index + 1,
                trip.name,
                trip.stops.first().map(|s| s.name.as_str()).unwrap_or(""),
                trip.departure
            );
            return;
        }
        // The first stop's tile may still be unloaded while a later stop already has a
        // place: snapping `near` to that later stop made GetTTBusstopIndex (Aachen ibox
        // Fahrplan list) start on stop #2. Keep the first stop until its place is known.
        let first_unknown = trip.stops.first().is_some_and(|s| s.position.is_none());
        self.next_stop = match near {
            Some((k, _)) if first_unknown && k > 0 => 0,
            Some((k, _)) => k,
            // a trip the player picked is driven from its first stop, late as it may be
            None if self.picked => self.next_stop,
            None => trip
                .stops
                .iter()
                .position(|s| s.arr >= now)
                .unwrap_or(trip.stops.len().saturating_sub(1)),
        };
        // standing at a stop of it, the bus is on its way (as if it had left the stop before
        // on time); elsewhere the duty goes on with the next trip when that is due
        self.left_late = near
            .filter(|&(k, _)| !(first_unknown && k > 0))
            .map(|_| 0.0);
        log::info!(
            "duty: the bus starts {} trip {} ({}) under way, next stop {} '{}'",
            if near.is_some() {
                "at a stop of"
            } else {
                "away from the stops of"
            },
            self.trip_index + 1,
            trip.name,
            self.next_stop,
            trip.stops
                .get(self.next_stop)
                .map(|s| s.name.as_str())
                .unwrap_or("")
        );
    }

    /// The trip a bus placed at its first stop starts with: the one under way or picked,
    /// else the first to leave from now on (the last when all have left).
    pub fn start_trip(&self, now: f64) -> usize {
        if self.picked {
            return self.trip_index;
        }
        (self.trip_index..self.trips.len())
            .find(|&k| self.trips[k].departure >= now)
            .unwrap_or(self.trip_index)
    }

    /// The duty starts with trip `k` at its stop `stop` (`duty_start`: the stops before it
    /// cannot be reached by road): that trip is the duty's first, driven from there.
    pub fn start_at(&mut self, k: usize, stop: usize) {
        if k < self.trips.len() {
            self.set_trip(k);
            self.picked = true;
            self.trip_changed = true;
            self.next_stop = stop.min(self.trips[k].stops.len().saturating_sub(1));
        }
    }

    /// Like `start_at`, for a bus that stays where it is (the stop was chosen in the menu,
    /// the bus is not put there): the first update does not look where the bus stands and
    /// does not move the chosen stop to one it happens to be near.
    pub fn start_at_here(&mut self, k: usize, stop: usize) {
        self.start_at(k, stop);
        self.placed = true;
    }

    /// Resume the saved trip at its saved ordinal, without choosing a fresh starting
    /// point or asking the device to retype its already restored programming. Past the
    /// first stop the bus is on its way, as if it had left the stop before on time (as
    /// `place` has it): `catch_up` may then look as far as the last stop and `delay`
    /// counts from there. Saved at the last stop, the trip is over.
    pub fn restore_progress(&mut self, stop: usize, pos: glam::DVec3) {
        self.served_terminus = None;
        let last = self.trip().stops.len().saturating_sub(1);
        self.next_stop = stop.min(last);
        self.left_late = (self.next_stop > 0).then_some(0.0);
        self.done = self.next_stop == last
            && self.trip().stops[last].position.is_some_and(|p| (p - pos).length() < AT_STOP);
        self.placed = true;
        self.picked = true;
        self.trip_changed = false;
    }

    /// The duty of a resumed situation: at its saved stop (an older save without one is
    /// placed by where the bus stands), with the timetable on the host before the first
    /// script frame.
    pub fn resume(&mut self, bus: &mut crate::VehicleInstance, day_time: f64, saved_stop: Option<usize>) {
        match saved_stop {
            Some(stop) => self.restore_progress(stop, bus.position),
            None => {
                self.update(bus, day_time);
            }
        }
        self.restore_host(bus, day_time);
    }

    /// A page sets the stop the duty goes on with (`omsi.setNextStop`), forwards or
    /// backwards: skipped stops count as not served, and going back makes the stops from
    /// `stop` on due again. Not once the trip's last stop is reached (`done`), unless the
    /// page goes back, which reopens the trip.
    pub fn skip_to(&mut self, stop: usize) -> bool {
        let last = self.trip().stops.len().saturating_sub(1);
        let stop = stop.min(last);
        log::debug!("duty: page asks for stop {stop} (next {}, at_stop {}, done {})", self.next_stop, self.at_stop, self.done);
        if stop == self.next_stop && !self.done {
            return false;
        }
        if self.done && stop >= self.next_stop {
            return false;
        }
        let back = stop < self.next_stop;
        self.next_stop = stop;
        self.at_stop = false;
        self.served_terminus = None;
        self.arrived_late = None;
        if back {
            // the trip ended reopened: an ending not yet taken is no ending, one taken is
            // undone (`take_reopened`)
            if self.done && self.finished.take().is_none() {
                self.reopened = Some(self.run);
            }
            self.done = false;
            self.held_back = true;
        }
        true
    }

    /// Whether the trip still has a stop to come ([`PlayerDuty::skip_next`]).
    pub fn stop_to_skip(&self) -> bool {
        !self.done && !self.trip().stops.is_empty()
    }

    /// The game menu's "Skip the next stop" (#1015): the stop the duty is due at is given
    /// up, not served, and the duty goes on with the one after it - for a stop the bus
    /// cannot reach, or whose object stands too far from where buses stop for it to count.
    /// The trip's last stop ends the trip: the tour's next one follows as usual. Returns the
    /// name of the stop skipped; None once the trip is over.
    pub fn skip_next(&mut self) -> Option<String> {
        if self.done {
            return None;
        }
        let last = self.trip().stops.len().checked_sub(1)?;
        let name = self.trip().stops[self.next_stop.min(last)].name.trim().to_string();
        if self.next_stop >= last {
            self.at_stop = false;
            self.arrived_late = None;
            self.done = true;
            self.finish(TripEnd::Skipped);
            return Some(name);
        }
        self.skip_to(self.next_stop + 1).then_some(name)
    }

    pub(crate) fn set_trip(&mut self, index: usize) {
        self.trip_index = index;
        self.run = next_run();
        self.next_stop = 0;
        self.at_stop = false;
        self.done = false;
        self.served_terminus = None;
        self.left_late = None;
        self.held_back = false;
        self.trip_changed = true;
        self.picked = false;
    }

    /// How late the bus is (s; negative = early), as the IBIS shows it: at a stop against
    /// its departure there, on the way against the time the timetable has it where it is -
    /// the last stop's departure and the next one's arrival shared out by how far it has
    /// come between them, as OMSI shows it while driving (it stood still between the stops
    /// at the delay the bus left the last one with, #1898, #735) - and at the end of a trip
    /// against the next trip's start.
    pub fn delay(&self, now: f64) -> f64 {
        let now = self.duty_time(now);
        let trip = self.trip();
        if self.done {
            if let Some(next) = self.trips.get(self.trip_index + 1) {
                return now - next.departure;
            }
        }
        let Some(stop) = trip.stops.get(self.next_stop) else {
            return 0.0;
        };
        if self.at_stop {
            return now - stop.dep;
        }
        let due = now - stop.arr;
        let last = self.next_stop.checked_sub(1).and_then(|k| trip.stops.get(k));
        if let (Some(last), Some(at), Some(here), true) = (last, last.and_then(|s| s.position), self.position, self.left_late.is_some()) {
            if let Some(next) = stop.position {
                let (gone, left) = ((here - at).truncate().length(), (next - here).truncate().length());
                if gone + left > 1.0 {
                    let share = gone / (gone + left);
                    return now - (last.dep + share * (stop.arr - last.dep));
                }
            }
        }
        self.left_late.map(|l| l.max(due)).unwrap_or(due)
    }

    /// The clock's time of day as the duty counts it: the day before or after when that is
    /// nearer the trip under way, so a duty across midnight (picked at 23:00 for trips from
    /// 0:49, or running from 23:40 into the night) is neither 22 hours late nor early.
    pub(crate) fn duty_time(&self, day_time: f64) -> f64 {
        let t = self.trip();
        let centre = (t.departure + t.end) / 2.0;
        [day_time - DAY, day_time, day_time + DAY]
            .into_iter()
            .min_by(|a, b| (a - centre).abs().total_cmp(&(b - centre).abs()))
            .unwrap_or(day_time)
    }

    /// Advance the duty and feed the vehicle host's timetable callbacks. Returns how late
    /// the bus left a stop, at the moment it leaves it (negative = early), which is what
    /// the personnel file counts.
    /// Returns, when the bus has just left a stop it had to serve, how late it arrived
    /// there and how late it left (seconds; negative: early).
    pub fn update(&mut self, bus: &mut crate::VehicleInstance, day_time: f64) -> Option<(f64, f64)> {
        let day_time = self.duty_time(day_time);
        self.heading = bus.heading;
        self.position = Some(bus.position);
        let served = self.advance(bus.position, day_time);
        if self.done
            && self.at_stop
            && bus.physics.velocity_kmh().abs() < 0.36
            && Self::doors_open(bus)
        {
            self.served_terminus = self.trip().stops.last().and_then(|stop| stop.position);
        }
        // (the next trip starts on leaving the terminus only when it is due within a few
        // minutes: a bus moved to its layover or across to the departure stand well before
        // then waits for it, instead of being hours early on a trip begun at once)
        if self.trip_index + 1 < self.trips.len()
            && self.trips[self.trip_index + 1].departure - day_time <= EARLY_START
            && self
                .served_terminus
                .is_some_and(|stop| (bus.position - stop).length() >= 60.0)
        {
            let terminus = self.served_terminus.unwrap();
            self.set_trip(self.trip_index + 1);
            // This trip has begun, however early it is. Do not place it again or skip it
            // as an unbegun trip on a subsequent update.
            self.picked = true;
            self.left_late = Some(day_time - self.trip().departure);
            if self
                .trip()
                .stops
                .first()
                .and_then(|stop| stop.position)
                .is_some_and(|start| (start - terminus).length() < AT_STOP)
            {
                self.next_stop = 1.min(self.trip().stops.len().saturating_sub(1));
            }
            self.advance(bus.position, day_time);
        }
        self.feed_host(bus, day_time);
        served
    }

    pub(crate) fn doors_open(bus: &crate::VehicleInstance) -> bool {
        let mut reports_passenger_doors = false;
        let mut passenger_door_open = false;
        for i in 0..16 {
            for kind in ["Entry", "Exit"] {
                let name = format!("PAX_{kind}{i}_Open");
                if bus.has_script_var(&name)
                    || bus
                        .ty
                        .program
                        .var(&name)
                        .is_some_and(|id| bus.ty.program.stores(id))
                {
                    reports_passenger_doors = true;
                    passenger_door_open |= bus.var(&name).unwrap_or(0.0) > 0.5;
                }
            }
        }
        if reports_passenger_doors {
            passenger_door_open
        } else {
            (0..8).any(|i| {
                bus.var(&format!("door_{i}"))
                    .or_else(|| bus.var(&format!("door{i}")))
                    .unwrap_or(0.0)
                    > 0.5
            })
        }
    }

    /// Supply restored timetable data before the first resumed script frame.
    pub fn restore_host(&self, bus: &mut crate::VehicleInstance, day_time: f64) {
        self.feed_host(bus, day_time);
        bus.host.schedule_active = 1.0;
        bus.set_var("schedule_active", 1.0);
    }

    pub(crate) fn feed_host(&self, bus: &mut crate::VehicleInstance, day_time: f64) {
        let delay = self.delay(day_time);
        let trip = &self.trips[self.trip_index];
        let prev_tt = bus.host.tt_busstop_index;
        let stop_name = trip.stops.get(self.next_stop).map(|s| s.name.clone());
        {
            let host = &mut bus.host;
            host.tt_line = trip.line.clone();
            host.tt_stops = trip
                .stops
                .iter()
                .map(|s| (s.name.clone(), s.arr as f32, s.dep as f32))
                .collect();
            host.tt_stop_ids = trip.stops.iter().map(|s| s.object_id).collect();
            host.tt_busstop_index = self.next_stop as i32;
            host.tt_terminus_index = tt_terminus_index(host.hof.as_deref(), &trip.terminus);
            host.tt_delay = delay as f32;
        }
        // Aachen's ibox keeps its own stop counter and does `+ 1` when
        // `GetTTBusstopIndex` changes. A jump of more than one stop, or a unit that only
        // has `ibox_busstop`, left that counter behind the timetable - the announcement
        // used the new TT name while the list still showed the old `ibox_busstop` row.
        if self.next_stop as i32 != prev_tt {
            if let Some(name) = stop_name.as_deref() {
                sync_script_busstop(bus, self.next_stop, name, prev_tt);
            }
        }
    }

    /// The bus came to a later stop of the trip than the one it is due at (it drove past
    /// some). Which stop that is cannot be told by the distance alone: a circular route, or
    /// one that turns back, calls at the same place twice, and its two stops there stand a
    /// few metres apart, so a bus at one is within [`AT_STOP`] of the other as well - the
    /// duty jumped from stop 2 to stop 18 and 3-17 were never served (#254). Three things
    /// have to agree: the trip runs through the stop the way the bus heads ([`StopDir`]);
    /// the bus stands nearer to it than to the stop it is due at; and it has driven away
    /// from the stop it served last.
    pub(crate) fn catch_up(&mut self, pos: glam::DVec3, fwd: glam::DVec2) {
        if self.held_back {
            return;
        }
        let trip = &self.trips[self.trip_index];
        let last = trip.stops.len().saturating_sub(1);
        let upto = if self.left_late.is_some() { trip.stops.len() } else { last };
        if self.next_stop + 1 >= upto {
            return;
        }
        let of = |k: usize| -> Option<f64> { trip.stops.get(k).and_then(|s| s.position).map(|p| (p - pos).length()) };
        // still at the stop it served: too early to look for a later one
        if let Some(k) = self.next_stop.checked_sub(1) {
            if of(k).is_some_and(|d| d <= AT_STOP) {
                return;
            }
        }
        let here = of(self.next_stop);
        let mut best: Option<(usize, f64)> = None;
        for k in self.next_stop + 1..upto {
            let Some(d) = of(k) else { continue };
            if d >= AT_STOP || here.is_some_and(|h| d >= h) || !trip.stops[k].dir.takes(fwd) {
                continue;
            }
            if best.is_none_or(|(_, b)| d < b) {
                best = Some((k, d));
            }
        }
        let Some((k, _)) = best else { return };
        log::info!(
            "duty: trip {}: {} stop(s) passed without stopping, the bus at stop {} '{}' (it was due at {})",
            trip.name,
            k - self.next_stop,
            k + 1,
            trip.stops[k].name.trim(),
            self.next_stop + 1
        );
        self.skipped = Some((k - self.next_stop, self.next_stop + 1, k + 1));
        self.next_stop = k;
    }

    /// The duty's progress with the bus at `pos` (see [`PlayerDuty::update`]).
    pub(crate) fn advance(&mut self, pos: glam::DVec3, day_time: f64) -> Option<(f64, f64)> {
        if !self.placed {
            // (stops beyond the loaded tiles have no place yet: a few seconds for the
            // navigator's map, unless the bus stands at a stop of its trip)
            let first = *self.first_update.get_or_insert(day_time);
            let at_a_stop = self.trip().stops.iter().any(|s| s.position.map(|p| (p - pos).length() < AT_STOP).unwrap_or(false));
            let known = self.trips[self.trip_index..].iter().all(|t| t.stops.first().map(|s| s.position.is_some()).unwrap_or(true));
            if !(at_a_stop || known || (day_time - first).abs() > 12.0) {
                return None;
            }
            self.placed = true;
            self.place(pos, day_time);
        }
        // on to the next trip a minute before it leaves, once this one is over, was never
        // begun, or was given up half an hour ago
        while self.trip_index + 1 < self.trips.len()
            && self.trips[self.trip_index + 1].departure - 60.0 <= day_time
        {
            let given_up = day_time > self.trip().end + 1800.0;
            let unbegun = self.left_late.is_none() && !self.picked;
            // on the trip's last leg and standing at the next trip's first stop: this trip is
            // over even when its last stop was never reached within AT_STOP (a terminus
            // whose stop object lies away from where the buses stand, or that has no place
            // on the map). Before, the duty stayed on the old trip until half an hour after
            // its end: the IBIS kept the old terminus, the people at the new trip's stops
            // waited for another bus, and they boarded only once that half hour was up -
            // somewhere along the route.
            let last = self.trip().stops.len().saturating_sub(1);
            // (at the last stop itself it is reached the usual way, its arrival counted)
            let at_last = self.trip().stops.get(last).and_then(|s| s.position).is_some_and(|p| (p - pos).length() < AT_STOP);
            let at_next_start = self.next_stop >= last
                && !at_last
                && self.trips[self.trip_index + 1].stops.first().and_then(|s| s.position).is_some_and(|p| (p - pos).length() < AT_STOP);
            if !(self.done || unbegun || given_up || at_next_start) {
                break;
            }
            // (one done has ended already; one the bus never left a stop of was never driven,
            // picked or not)
            let begun = self.left_late.is_some();
            if !self.done && begun && at_next_start {
                self.finish(TripEnd::Arrived);
            } else if !self.done && begun && given_up {
                self.finish(TripEnd::GivenUp);
            }
            self.set_trip(self.trip_index + 1);
            log::info!(
                "duty: trip {} {} to {}",
                self.trip_index + 1,
                self.trip().name,
                self.trip().terminus
            );
        }
        let mut served = None;
        let trip = &self.trips[self.trip_index];
        let last = trip.stops.len().saturating_sub(1);
        // the bus reached a later stop of the trip (skipped stops); before it has left a
        // stop of the trip, not its last one (where the tour's previous trip may end)
        if !self.at_stop {
            self.catch_up(pos, forward_of(self.heading));
        }
        let trip = &self.trips[self.trip_index];
        // stop progress by proximity
        if let Some(stop) = trip.stops.get(self.next_stop) {
            if let Some(p) = stop.position {
                let d = (p - pos).length();
                if d < AT_STOP {
                    self.held_back = false;
                    if !self.at_stop {
                        self.arrived_late = Some(day_time - stop.arr);
                    }
                    self.at_stop = true;
                    if self.next_stop == last && !self.done {
                        self.done = true;
                        self.finish(TripEnd::Arrived);
                    }
                } else if self.at_stop && d > LEFT_STOP {
                    self.at_stop = false;
                    let late = day_time - stop.dep;
                    self.left_late = Some(late);
                    // (OMSI counts a stop only with its arrival: the original)
                    if let (true, Some(arrived)) = (stop.stops, self.arrived_late.take()) {
                        served = Some((arrived, late));
                    }
                    self.next_stop = (self.next_stop + 1).min(last);
                    log::debug!("duty: left stop, next stop now {}", self.next_stop);
                }
            }
        }
        served
    }
}
