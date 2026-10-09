//! The random cars out of the player's range (see `DormantCar`).

use super::*;

/// How far from the player cars are kept (m) and how far out of sight one may be before it
/// is taken off (m); in plain view a car stays until it is too small to see.
pub const DESPAWN_FACTOR: f64 = 1.6;

/// How many cars a whole map keeps at most, as a multiple of the number asked for around the
/// player (memory: a dormant car is a few dozen bytes, but each one woken is a full vehicle).
pub const MAP_POPULATION_FACTOR: f32 = 8.0;


pub fn street_lane_weight(l: &crate::traffic::Lane) -> Option<f64> {
    (l.kind == LaneKind::Street && !l.no_cars && l.density > 0.0 && l.length() >= 8.0)
        .then(|| l.length() as f64 * l.density.clamp(0.05, 4.0) as f64)
}

impl TrafficSim {
    /// The cars out of range drive on: along their lanes at about the lanes' speed, taking
    /// a random way at every fork; one that reaches the end of the network has left the map.
    pub fn advance_dormant(&mut self) {
        let dt = (self.time - self.dormant_time).clamp(0.0, 10.0);
        self.dormant_time = self.time;
        if dt <= 0.0 || self.dormant.is_empty() {
            return;
        }
        let mut i = 0;
        while i < self.dormant.len() {
            let mut gone = false;
            {
                let d = &mut self.dormant[i];
                let lane = &self.net.lanes[d.lane];
                // (waits at lights and junctions taken as a quarter off the speed limit)
                d.speed = (lane.speed_limit_kmh.min(60.0) / 3.6 * 0.75).max(2.0);
                d.s += d.speed * dt;
                let mut guard = 0;
                while d.s > self.net.lanes[d.lane].length() && guard < 32 {
                    guard += 1;
                    let l = &self.net.lanes[d.lane];
                    // the ways a car on the road would take (`AiState::choose_after`): those
                    // of its group, else those open to cars, else any - only where the
                    // network ends does it leave the map (filtering by its group alone, the
                    // trucks of Spandau were gone at the first junction whose turn has no
                    // `trucks` rule, and hardly one of them ever came into range)
                    let pool = self.types.iter().find(|t| Arc::ptr_eq(&t.0, &d.ty)).and_then(|t| self.group_uvg[t.3]);
                    let same_kind = |n: &usize| self.net.lanes[*n].kind == d.kind;
                    let open = |pooled: bool| -> Vec<usize> {
                        l.next
                            .iter()
                            .copied()
                            .filter(same_kind)
                            .filter(|&n| {
                                let nl = &self.net.lanes[n];
                                let density = match pool.filter(|_| pooled) {
                                    Some(p) => nl.pool_density(&self.uvg_defaults, p),
                                    None => nl.density,
                                };
                                nl.allows(d.ty.def.ai_veh_type) && density > 0.0
                            })
                            .collect()
                    };
                    let mut options = if pool.is_some() { open(true) } else { Vec::new() };
                    if options.is_empty() {
                        options = open(false);
                    }
                    if options.is_empty() {
                        options = l.next.iter().copied().filter(same_kind).collect();
                    }
                    if options.is_empty() {
                        gone = true;
                        break;
                    }
                    d.s -= l.length();
                    d.walk = d.walk.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    d.lane = options[(d.walk >> 33) as usize % options.len()];
                }
            }
            if gone {
                self.dormant.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Fill the map: the streets the map has shown so far carry as many cars per metre as
    /// the ones around the player, the ones out of range as dormant cars (see `DormantCar`).
    pub fn fill_map(&mut self, center: DVec3, street_target: usize) {
        if street_target == 0 {
            return;
        }
        let far = self.spawn_radius * DESPAWN_FACTOR;
        let centers: Vec<DVec3> = std::iter::once(center).chain(self.lan_centers.iter().copied()).collect();
        let mut nearby: Vec<usize> = centers.iter()
            .flat_map(|&c| self.net.lanes_starting_near(c, self.spawn_radius))
            .collect();
        nearby.sort_unstable();
        nearby.dedup();
        let mut near = 0f64;
        for i in nearby {
            let l = &self.net.lanes[i];
            let Some(w) = street_lane_weight(l) else { continue };
            let d = centers.iter().map(|c| (l.start() - *c).truncate().length()).fold(f64::MAX, f64::min);
            if d < self.spawn_radius {
                near += w;
            }
        }
        if near < 50.0 {
            return;
        }
        let map_target = ((street_target as f64 * self.street_weight / near).min(street_target as f64 * MAP_POPULATION_FACTOR as f64)) as usize;
        // (a car that gave up counts while it is on the road: see omsi-app's `populate_kind`)
        let present = self.cars.iter().filter(|c| !c.is_bus()).count() + self.dormant.len();
        if present >= map_target {
            return;
        }
        // The full outside list is only needed while replenishing the map population.
        let outside: Vec<(usize, f32)> = self.net.lanes.iter().enumerate()
            .filter_map(|(i, l)| {
                let w = street_lane_weight(l)?;
                let d = centers.iter().map(|c| (l.start() - *c).truncate().length()).fold(f64::MAX, f64::min);
                (d > far).then_some((i, w as f32))
            })
            .collect();
        if outside.is_empty() {
            return;
        }
        let mut acc = 0.0f32;
        let cumulative: Vec<f32> = outside.iter().map(|c| { acc += c.1; acc }).collect();
        for _ in 0..(map_target - present).min(64) {
            let x = self.rand_f() as f32 * acc;
            let lane = outside[cumulative.partition_point(|&c| c < x).min(outside.len() - 1)].0;
            let s = (self.rand_f() * (self.net.lanes[lane].length() as f64 - 4.0)) as f32 + 2.0;
            let Some(ty) = self.pick_type(LaneKind::Street, Some(lane)) else {
                continue;
            };
            let seed = self.rand();
            let scheme = if ty.paint_schemes.is_empty() { None } else { Some((seed >> 8) as usize % ty.paint_schemes.len().min(AI_SCHEMES)) };
            let id = self.next_id;
            self.next_id += 1;
            self.dormant.push(DormantCar { id, ty, kind: LaneKind::Street, lane, s, speed: 8.0, seed, scheme, walk: seed | 1 });
        }
    }
}
