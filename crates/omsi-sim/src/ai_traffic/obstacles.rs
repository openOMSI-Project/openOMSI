//! Bodies in a car's way: other cars by geometry, the player's bus, people on foot.

use super::*;

/// The player's box as `player_in_way` sees it is this much longer at each end (m).
pub const PLAYER_BOX_MARGIN: f32 = 0.5;
/// Somebody on foot more than this far above or below a car's way is not in it (m).
pub const PEOPLE_LEVEL: f64 = 2.5;

/// Another vehicle more than this far above or below a car's way (where the way passes it)
/// is not in it (m): the road under a bridge is some 4.5 m below the deck.
pub const BODY_LEVEL: f64 = 3.0;

/// How far ahead a car looks for other vehicles at least (m).
pub const LOOK_AHEAD: f32 = 70.0;
/// ... and at most, when it is fast.
pub const LOOK_AHEAD_MAX: f32 = 150.0;

/// How far ahead a driver at `speed` watches for something standing in the way: far enough
/// to slow down gently for it. With a fixed 70 m a car at 50-65 km/h first saw the player's
/// bus standing (or a bus at its stop) so late that the following model braked at 4-5 m/s².
pub fn look_ahead(speed: f32) -> f32 {
    (speed * speed / 3.0 + speed * 2.0 + 20.0).clamp(LOOK_AHEAD, LOOK_AHEAD_MAX)
}

/// How far ahead of the player's bus centre a car looks for it (m): the bus's half length,
/// and where it will be in `horizon` seconds for a car whose way crosses the bus's. Not
/// for one going the same way (`way_dir` within 60 degrees of the bus's heading): with the
/// bus behind it, that stretch ahead of the bus reached over the car itself and it braked
/// for a bus that was only following it (#139).
pub fn player_reach_ahead(half_len: f32, speed: f32, horizon: f32, fwd: DVec2, way_dir: DVec2) -> f64 {
    let same_way = way_dir.length() > 0.5 && way_dir.normalize().dot(fwd) > 0.5;
    half_len as f64 + if same_way { 0.0 } else { (speed.max(0.0) * horizon) as f64 }
}

impl TrafficSim {
    /// Where car `i` has to stop for somebody on foot (the distance of its front from its
    /// origin, as the other stops): anybody standing in the strip it is about to sweep, or
    /// stepping into it by the time the car gets there. Only the zebras and signalled
    /// crossings used to count, and only for people strolling the footpaths, so a car
    /// drove at full speed through a passenger walking off a bus across the road, through
    /// somebody leaving a stop, or through anybody standing in the carriageway. A
    /// timetable bus ignores the people waiting at the kerb for it unless they stand well
    /// inside its path (it pulls up right beside them).
    pub fn people_stop(&self, i: usize, way: &[(usize, f32)]) -> Option<(f32, DVec2)> {
        if self.people.is_empty() {
            return None;
        }
        let car = &self.cars[i];
        let st = &car.state;
        let v = st.speed.max(0.0);
        // as far as the car needs to stop without a jolt, and never less than a car length
        let reach = (v * v / 5.0 + v + 6.0).clamp(8.0, 45.0);
        let origin = car.vehicle.position.truncate();
        let near: Vec<&(DVec3, DVec2, bool)> = self
            .people
            .iter()
            .filter(|(p, _, _)| (p.truncate() - origin).length() < (st.front + reach) as f64 + 6.0)
            .collect();
        if near.is_empty() {
            return None;
        }
        let from = st.front - 1.0;
        let mut first = true;
        for &(l, dl) in way {
            let lane = &self.net.lanes[l];
            let len = lane.length();
            let mut s = (from - dl).max(0.0);
            while s <= len {
                let d = dl + s;
                if d > st.front + reach {
                    return None;
                }
                let (q, h) = lane.at(s);
                let hr = (h as f64).to_radians();
                let right = DVec2::new(hr.cos(), -hr.sin());
                let fwd = DVec2::new(hr.sin(), hr.cos());
                // the car's own offset from the lane counts on the lane it is on
                let lat = if first { st.lateral as f64 } else { 0.0 };
                let c = q.truncate() + right * lat;
                // when the car's front gets here, at most two seconds on
                let t = (((d - st.front).max(0.0)) / v.max(1.0)).min(2.0) as f64;
                for (p, pv, waiting) in &near {
                    // on the same level only: somebody on a footbridge over the road, in a
                    // subway under it or on a platform above it is in nobody's way here (a
                    // car would stand in front of nothing anybody could see)
                    if (p.z - q.z).abs() > PEOPLE_LEVEL {
                        continue;
                    }
                    let half = if *waiting && car.is_bus() {
                        car.half_width as f64 - 0.3
                    } else {
                        car.half_width as f64 + 0.3
                    };
                    for at in [p.truncate(), p.truncate() + *pv * t] {
                        let rel = at - c;
                        if rel.dot(fwd).abs() <= 0.55 && rel.dot(right).abs() < half {
                            return Some((d - 1.5, p.truncate()));
                        }
                    }
                }
                s += 1.0;
            }
            first = false;
        }
        None
    }

    /// Two cars that have each other for their lead - a car that ended up in a bus's body,
    /// each finding the other in its way - wait for each other for good. The one further
    /// along its lane (on the same lane; else the lower number) stops taking the other for
    /// its lead for a few seconds and drives off.
    pub fn break_lead_pairs(&mut self) {
        let index: HashMap<u64, usize> = self.cars.iter().enumerate().map(|(k, c)| (c.id, k)).collect();
        let mut pairs: Vec<(usize, u64)> = Vec::new();
        for (a, c) in self.cars.iter().enumerate() {
            let Some(bid) = c.lead_car else { continue };
            let Some(&b) = index.get(&bid) else { continue };
            if b <= a || self.cars[b].lead_car != Some(c.id) {
                continue;
            }
            let (sa, sb) = (&self.cars[a].state, &self.cars[b].state);
            let a_goes = if sa.lane == sb.lane { sa.s > sb.s } else { c.id < bid };
            let (go, other) = if a_goes { (a, bid) } else { (b, c.id) };
            pairs.push((go, other));
        }
        for (go, other) in pairs {
            if omsi_cfg::flags::OMSI_DEBUG_STUCK.is_set() {
                log::info!("t={:.1}: cars {} and {} each waited for the other: {} drives off", self.time, self.cars[go].id, other, self.cars[go].id);
            }
            self.cars[go].ignore_lead = Some((other, self.time as f64 + 5.0));
        }
    }

    /// The footprints of all AI vehicles, rear sections and trailers included.
    pub fn footprints(&self) -> Vec<Footprint> {
        let mut out = Vec::with_capacity(self.cars.len() + 8);
        for (i, c) in self.cars.iter().enumerate() {
            let st = &c.state;
            let h = c.vehicle.heading.to_radians();
            let (fwd, right) = (DVec2::new(h.sin(), h.cos()), DVec2::new(h.cos(), -h.sin()));
            let center = c.vehicle.position.truncate() + fwd * ((st.front - st.rear) * 0.5) as f64;
            out.push(Footprint {
                car: i,
                center,
                fwd,
                right,
                half_len: ((st.front + st.rear) * 0.5) as f64,
                half_w: c.half_width as f64,
                speed: st.speed,
                z: c.vehicle.position.z,
            });
            for t in &c.vehicle.trailers {
                if let Some(bb) = t.ty.def.bounding_box {
                    out.push(Footprint::from_obb(
                        i,
                        &crate::collision::Obb::from_box(bb, t.position, t.body_heading()),
                        st.speed,
                    ));
                }
            }
        }
        out
    }

    /// May a vehicle of `ty` be put on the road at `pos` facing `heading` (deg)? Not onto
    /// (or right up against) another vehicle or the player's: a timetable bus used to be
    /// checked only for a vehicle origin within 9 m, and a layover bus appeared inside the
    /// articulated bus waiting at the same stand.
    pub fn spawn_clear(&self, ty: &VehicleType, pos: DVec3, heading: f64) -> bool {
        let (front, rear, half_w) = extents(ty, 12.0);
        let h = heading.to_radians();
        let (fwd, right) = (DVec2::new(h.sin(), h.cos()), DVec2::new(h.cos(), -h.sin()));
        let me = Footprint {
            car: usize::MAX,
            center: pos.truncate() + fwd * ((front - rear) * 0.5) as f64,
            fwd,
            right,
            half_len: ((front + rear) * 0.5) as f64,
            half_w: half_w as f64,
            speed: 0.0,
            z: pos.z,
        };
        if self.footprints().iter().any(|f| {
            (f.center - me.center).length() < f.half_len + me.half_len + 5.0
                && (f.z - me.z).abs() < 4.0
                && f.overlaps(&me, 1.0)
        }) {
            return false;
        }
        // nor just in front of a car driving up to that place (it would have to stop hard)
        let in_front = self.cars.iter().any(|c| {
            let rel = pos - c.vehicle.position;
            let h = c.vehicle.heading.to_radians();
            let (along, across) = (
                rel.x * h.sin() + rel.y * h.cos(),
                (rel.x * h.cos() - rel.y * h.sin()).abs(),
            );
            let v = c.state.speed;
            along > 0.0
                && along
                    < (c.state.front + rear + 15.0 + v * v / (2.0 * c.state.decel.max(1.0)) * 1.5)
                        as f64
                && across < 3.0
                && (rel.z).abs() < 4.0
        });
        if in_front {
            return false;
        }
        match self.player {
            Some((c, ph, hl, hw, _)) => {
                let h = ph.to_radians();
                let p = Footprint {
                    car: usize::MAX,
                    center: c.truncate(),
                    fwd: DVec2::new(h.sin(), h.cos()),
                    right: DVec2::new(h.cos(), -h.sin()),
                    half_len: hl as f64,
                    half_w: hw as f64,
                    speed: 0.0,
                    z: c.z,
                };
                (p.z - me.z).abs() > 4.0 || !p.overlaps(&me, 1.5)
            }
            None => true,
        }
    }

    /// Another AI vehicle's body in car `i`'s way where the lanes do not show it: a bus
    /// standing in its bay across a turning path, a car stopped half inside a junction, the
    /// rear section of an articulated bus still swinging round, a car cutting in. The way
    /// ahead is swept with the car's width against every footprint near it (the lanes alone
    /// let a car turn right into the side of a bus that stood 1.7 m out in its bay).
    /// Two vehicles that each stand in the other's way are sorted out by `geo_prev`: the one
    /// with the higher id goes, the other waits.
    pub fn body_in_way(
        &self,
        i: usize,
        feet: &[Footprint],
        by_lane: &HashMap<usize, Vec<(usize, f32, f32, bool)>>,
    ) -> Option<(Lead, usize)> {
        let car = &self.cars[i];
        let st = &car.state;
        if self.net.lanes[st.lane].kind == LaneKind::Air {
            return None;
        }
        let pos = car.vehicle.position.truncate();
        let z = car.vehicle.position.z;
        let look = (st.speed * st.speed / (2.0 * st.decel.max(1.0)) + st.speed * 2.0 + 12.0)
            .clamp(12.0, LOOK_AHEAD);
        let reach = st.front + look;
        // what the car is pulling out round does not stop it
        let rounding: Vec<usize> = if car.passing.map(|p| !p.aborted).unwrap_or(false)
            || st.change.map(|c| c.bypass).unwrap_or(false)
        {
            by_lane
                .get(&st.lane)
                .map(|v| {
                    v.iter()
                        .filter(|e| self.cars[e.0].state.speed < 0.5)
                        .map(|e| e.0)
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let me = car.id;
        let near: Vec<&Footprint> = feet
            .iter()
            .filter(|f| f.car != i && !rounding.contains(&f.car) && (f.z - z).abs() < BODY_LEVEL + 6.0)
            .filter(|f| (f.center - pos).length() < reach as f64 + f.half_len + f.half_w + 2.0)
            .filter(|f| {
                let o = &self.cars[f.car];
                // it waits for this car already: the higher id goes - but never into it: a
                // body within reach of the bumper stops the car whoever waits for whom (a car
                // drove straight into the side of a bus that stood waiting for it)
                !(self.geo_prev.get(f.car).copied().flatten() == Some(me)
                    && me > o.id
                    && !f.overlaps(&car_foot(car, 1.5), 0.0))
            })
            .collect();
        if near.is_empty() {
            return None;
        }
        let hw = car.half_width as f64;
        let mut d = st.front + 0.2;
        let mut p3 = st.way_point(&self.net, d);
        while d <= reach {
            // (finer close by, where the gap matters)
            let step = if d < st.front + 20.0 { 0.75 } else { 1.5 };
            let q3 = st.way_point(&self.net, d + step);
            let (p, q) = (p3.truncate(), q3.truncate());
            let dir = (q - p).normalize_or_zero();
            let across = DVec2::new(dir.y, -dir.x);
            for f in &near {
                // on the level of the way there, not of the car now: by the car's own
                // height a car on a bridge counted as in the way of one on the ramp down to
                // the road under it (they differed by under 4 m until right below it)
                if (f.z - p3.z).abs() > BODY_LEVEL {
                    continue;
                }
                let rel = p - f.center;
                // the footprint grown by this car's half width across its way, less a
                // little so that a car on the lane beside does not count (10 cm: at 20 cm
                // the corners of two bodies at a shallow merge ran a hand's breadth into
                // each other, the sampled centre line never quite inside the grown box)
                let gx = f.half_w + hw * across.dot(f.right).abs() - 0.1;
                let gy = f.half_len + hw * across.dot(f.fwd).abs() - 0.1;
                if rel.dot(f.right).abs() <= gx && rel.dot(f.fwd).abs() <= gy {
                    let along = (f.fwd.dot(dir) as f32 * f.speed).max(0.0);
                    let acc = if along > 0.1 {
                        self.cars[f.car].state.acc
                    } else {
                        0.0
                    };
                    return Some((
                        Lead {
                            gap: (d - st.front).max(0.0),
                            speed: along,
                            acc,
                        },
                        f.car,
                    ));
                }
            }
            p3 = q3;
            d += step;
        }
        None
    }

    /// Where the player's vehicle is in car `i`'s way: the gap to it and how fast it moves
    /// along that way. The bus's box is stretched along its motion for the next second and
    /// a half, so a bus pulling out of a stop, turning across or reversing is seen before
    /// it is in the lane - the lanes alone saw it only once it stood in them.
    pub fn player_in_way(&self, i: usize, player: &PlayerBox) -> Option<Lead> {
        let car = &self.cars[i];
        if (car.vehicle.position - player.0).length() > LOOK_AHEAD_MAX as f64 + 30.0 {
            return None;
        }
        self.player_on_way(&car.state, car.half_width, player, 0.0)
    }


    /// The player's bus standing (or just starting) at a stop with its indicator out
    /// towards the traffic: a car coming up behind it in the lane beside lets it out - it
    /// stops before the room the bus pulls out into (the bus's box grown by 2.5 m towards
    /// the road), as OMSI's traffic lets a bus leave its stop. Only a car going the bus's
    /// way that can still stop comfortably: one already beside the bus, or too close to
    /// stop, drives on. The gap from the car's front, or None.
    /// `signal`: the bus's indicator towards the traffic, (seconds since it last showed,
    /// seconds it has been indicating) - the player's or a LAN player's (`others_signal`).
    pub fn letting_out(&self, i: usize, player: &PlayerBox, signal: (f32, f32)) -> Option<f32> {
        let car = &self.cars[i];
        let st = &car.state;
        let (centre, heading, half_len, _, speed) = *player;
        let (signal_age, signalling) = signal;
        // (a bus that indicates and stays for long is not waited for: it is passed)
        if signal_age > 1.0
            || speed.abs() > 3.0
            || (signalling > 20.0 && speed.abs() < 0.3)
            || (car.vehicle.position - centre).length() > 120.0
        {
            return None;
        }
        let h = heading.to_radians();
        let fwd = DVec2::new(h.sin(), h.cos());
        let way_dir = (st.way_point(&self.net, 3.0) - st.way_point(&self.net, 0.0)).truncate();
        if way_dir.length() < 0.5 || way_dir.normalize().dot(fwd) < 0.8 {
            return None;
        }
        // (behind the bus: a car passing it already, or past it, is not held)
        let rel = (car.vehicle.position - centre).truncate();
        if rel.dot(fwd) > -(half_len as f64) - st.front as f64 {
            return None;
        }
        let l = self.player_on_way(st, car.half_width, player, 2.5)?;
        let comfortable = st.speed * st.speed / (2.0 * st.decel.max(1.0));
        (l.gap > 0.5 && l.gap >= comfortable).then_some(l.gap)
    }

    /// `player_in_way` for a car of half width `half_width` on the way `st` lays out (also a
    /// way it only considers taking).
    /// `widen`: the box grown by this much (m) towards the traffic (the left, or the right
    /// on a left-hand-traffic map).
    pub fn player_on_way(&self, st: &AiState, half_width: f32, player: &PlayerBox, widen: f64) -> Option<Lead> {
        let (centre, heading, half_len, half_w, speed) = *player;
        let h = heading.to_radians();
        let fwd = DVec2::new(h.sin(), h.cos());
        let right = DVec2::new(h.cos(), -h.sin());
        let out = if self.net.left_hand { 1.0 } else { -1.0 };
        let centre = centre + (right * (out * widen * 0.5)).extend(0.0);
        let half_w = half_w + (widen * 0.5) as f32;
        let horizon = if self.player_priority { 5.0 } else { 1.5 };
        let way_dir = (st.way_point(&self.net, 3.0) - st.way_point(&self.net, 0.0)).truncate();
        let ahead = player_reach_ahead(half_len, speed, horizon, fwd, way_dir);
        let behind = half_len as f64 + ((-speed).max(0.0) * horizon) as f64;
        let wide = (half_w + half_width + 0.35) as f64;
        let margin = PLAYER_BOX_MARGIN as f64;
        let inside = |p: DVec3| in_player_box(p, centre, fwd, right, wide, ahead + margin, behind + margin);
        let look = (st.speed * st.speed / (2.0 * st.decel) + st.speed * 2.0 + 15.0)
            .clamp(15.0, look_ahead(st.speed));
        let mut d = 0.0f32;
        let mut step = 1.5f32;
        while d <= st.front + look {
            let p = st.way_point(&self.net, d.max(0.0));
            if inside(p) {
                // where between the samples the way enters the box: the gap to a standing bus
                // used to come in steps of a metre and a half (and up to that much too long,
                // so that cars stopped closer than they meant to)
                let (mut lo, mut hi) = ((d - step).max(0.0), d);
                if d > 0.0 {
                    for _ in 0..5 {
                        let mid = 0.5 * (lo + hi);
                        if inside(st.way_point(&self.net, mid)) {
                            hi = mid;
                        } else {
                            lo = mid;
                        }
                    }
                }
                let q = st.way_point(&self.net, hi + 1.0);
                let dir = (q - st.way_point(&self.net, hi))
                    .truncate()
                    .normalize_or_zero();
                let along = (fwd.dot(dir) as f32 * speed).max(0.0);
                return Some(Lead {
                    gap: (hi - st.front).max(0.0),
                    speed: along,
                    acc: 0.0,
                });
            }
            // (coarser far off: the entry is found by halving anyway)
            step = if d < st.front + 30.0 { 1.5 } else { 3.0 };
            d += step;
        }
        None
    }
}

/// Whether the point `p` of a car's way lies in the player's box round `centre` (`wide` to
/// either side, `ahead` in front and `behind` behind it) - on the same level only: a bus
/// under a bridge held up the traffic on the bridge above it (#753). 4 m, as for the other
/// vehicles' bodies.
pub fn in_player_box(p: DVec3, centre: DVec3, fwd: DVec2, right: DVec2, wide: f64, ahead: f64, behind: f64) -> bool {
    let rel = p.truncate() - centre.truncate();
    let (x, y) = (rel.dot(right), rel.dot(fwd));
    x.abs() <= wide && y <= ahead && y >= -behind && (p.z - centre.z).abs() < 4.0
}
