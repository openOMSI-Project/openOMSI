//! `OMSI_TRAFFIC_STATS=<file.csv>`: how well the traffic flows, measured as it runs - for a
//! long offscreen run (`--drive 1800`) to compare one build's traffic with another's.
//!
//! Every second of game time it looks at all cars: how many stand and for how long, the
//! waits-for graph (who stands for whom: the car ahead, the car it gives way to at a
//! junction) with its cycles - a ring of cars each waiting for the next never clears by
//! itself - and, for every car that has stood for over a minute, the head of the chain it
//! stands in and why that one stands (a light, a junction, nothing visible ...). Every tick
//! it counts what happens: entries into junctions, the road driven, lights run on red,
//! emergency braking, cars that gave up after standing for long, bodies inside each other.
//! A line of the CSV sums up each minute, and `summary` the whole run.

use super::*;
use std::io::Write;

/// Seconds between two looks at the whole traffic.
const SAMPLE: f32 = 1.0;
/// Seconds summed up in one line of the CSV.
const WINDOW: f32 = 60.0;
/// A car standing longer than this (s) counts as waiting in the waits-for graph.
const WAITING: f32 = 5.0;
/// Braking harder than this (m/s²) is an emergency stop: something came too close.
pub const HARD_BRAKE: f32 = -4.5;
/// ... from at least this speed (m/s); below it, a stop jerk (see `Counts::stop_jerks`).
pub const BRAKE_FROM: f32 = 1.5;

/// A car stands for this car (its id): the player's vehicle or a LAN player's.
pub const WAITS_ON_PLAYER: u64 = u64::MAX;

/// What one window (or the whole run) counted.
#[derive(Debug, Clone, Default)]
pub struct Counts {
    pub samples: u32,
    /// Sums over the samples: cars on the road, their speeds (m/s), cars that stood more
    /// than 30, 60 and 120 s, cars in waits-for cycles, the cycles.
    pub cars: u64,
    /// ... of them given up (`AiCar::gone`) and still on the road.
    pub gone: u64,
    pub speed_sum: f64,
    pub moving_speed_sum: f64,
    pub moving: u64,
    pub stood30: u64,
    pub stood60: u64,
    pub stood120: u64,
    pub in_cycles: u64,
    pub cycles: u64,
    /// The most at any sample.
    pub max_stood60: u64,
    pub max_in_cycles: u64,
    /// Events: cars into a junction, metres driven, lights run on red, emergency braking,
    /// cars that gave up after standing long, new pairs of bodies inside each other, new
    /// pairs within half a metre of each other closing fast.
    pub junction_entries: u64,
    pub metres: f64,
    pub red_runs: u64,
    pub hard_brakes: u64,
    pub gave_up: u64,
    pub overlaps: u64,
    pub close_calls: u64,
    /// Cars creeping at walking pace that stopped short again (an emergency braking below
    /// `BRAKE_FROM`): hesitation at a line.
    pub stop_jerks: u64,
    /// Ticks, and the seconds their planning took (who is where, the lights, every car's
    /// plan: `TrafficSim::tick_split` without the bodies and scripts).
    pub ticks: u64,
    pub plan_secs: f64,
    /// Why the heads of the chains stand that cars stood more than a minute in (summed over
    /// the samples).
    pub heads: HashMap<&'static str, u64>,
}

impl Counts {
    fn add(&mut self, o: &Counts) {
        self.samples += o.samples;
        self.cars += o.cars;
        self.gone += o.gone;
        self.speed_sum += o.speed_sum;
        self.moving_speed_sum += o.moving_speed_sum;
        self.moving += o.moving;
        self.stood30 += o.stood30;
        self.stood60 += o.stood60;
        self.stood120 += o.stood120;
        self.in_cycles += o.in_cycles;
        self.cycles += o.cycles;
        self.max_stood60 = self.max_stood60.max(o.max_stood60);
        self.max_in_cycles = self.max_in_cycles.max(o.max_in_cycles);
        self.junction_entries += o.junction_entries;
        self.metres += o.metres;
        self.red_runs += o.red_runs;
        self.hard_brakes += o.hard_brakes;
        self.gave_up += o.gave_up;
        self.overlaps += o.overlaps;
        self.close_calls += o.close_calls;
        self.stop_jerks += o.stop_jerks;
        self.ticks += o.ticks;
        self.plan_secs += o.plan_secs;
        for (k, v) in &o.heads {
            *self.heads.entry(k).or_default() += v;
        }
    }

    /// Mean over the samples.
    fn mean(&self, sum: u64) -> f64 {
        sum as f64 / self.samples.max(1) as f64
    }

    pub fn mean_cars(&self) -> f64 {
        self.mean(self.cars)
    }

    pub fn mean_kmh(&self) -> f64 {
        self.speed_sum / self.cars.max(1) as f64 * 3.6
    }

    /// The heads of the long waits, most frequent first.
    pub fn head_list(&self) -> String {
        let mut v: Vec<(&&str, &u64)> = self.heads.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        v.iter().map(|(k, n)| format!("{k}:{n}")).collect::<Vec<_>>().join(" ")
    }
}

/// The traffic's flow statistics (see the module).
pub struct TrafficStats {
    out: Option<std::io::BufWriter<std::fs::File>>,
    /// Game seconds since the start, and when the next sample and line are due.
    t: f32,
    next_sample: f32,
    next_line: f32,
    pub window: Counts,
    pub total: Counts,
    /// Per car (id): its lane, odometer and acceleration as of the tick before.
    before: HashMap<u64, (usize, f32, f32)>,
    /// Pairs of cars (ids) inside each other / close at the last sample.
    touching: hashbrown::HashSet<(u64, u64)>,
    close: hashbrown::HashSet<(u64, u64)>,
}

impl TrafficStats {
    /// The statistics into the file `OMSI_TRAFFIC_STATS` names (None without it).
    pub fn from_env() -> Option<Box<TrafficStats>> {
        let path = omsi_cfg::flags::OMSI_TRAFFIC_STATS.os()?;
        let mut f = std::io::BufWriter::new(
            std::fs::File::create(&path)
                .map_err(|e| log::warn!("OMSI_TRAFFIC_STATS: {e}"))
                .ok()?,
        );
        writeln!(f, "t,cars,gone,dormant,mean_kmh,moving_kmh,stood30,stood60,stood120,max_stood60,in_cycles,max_in_cycles,cycles,junction_entries,km,red_runs,hard_brakes,gave_up,overlaps,close_calls,heads").ok()?;
        Some(Box::new(TrafficStats::new(Some(f))))
    }

    pub fn new(out: Option<std::io::BufWriter<std::fs::File>>) -> TrafficStats {
        TrafficStats {
            out,
            t: 0.0,
            next_sample: SAMPLE,
            next_line: WINDOW,
            window: Counts::default(),
            total: Counts::default(),
            before: HashMap::new(),
            touching: Default::default(),
            close: Default::default(),
        }
    }

    /// The whole run so far, in one line.
    pub fn summary(&self) -> String {
        let mut all = self.total.clone();
        all.add(&self.window);
        let mins = (self.t / 60.0).max(1e-3) as f64;
        format!(
            "traffic stats over {:.1} min: {:.1} cars ({:.1} given up), mean {:.1} km/h ({:.1} km/h moving), stood >30 s {:.2} >60 s {:.2} >120 s {:.2} (most {}), in waits-for cycles {:.2} (most {}), {:.1} junction entries/min, {:.1} km/min, red runs {}, emergency brakes {}, stop jerks {}, gave up {}, overlaps {}, close calls {}, planning {:.3} ms/tick; long waits headed by {}",
            self.t / 60.0,
            all.mean_cars(),
            all.mean(all.gone),
            all.mean_kmh(),
            all.moving_speed_sum / all.moving.max(1) as f64 * 3.6,
            all.mean(all.stood30),
            all.mean(all.stood60),
            all.mean(all.stood120),
            all.max_stood60,
            all.mean(all.in_cycles),
            all.max_in_cycles,
            all.junction_entries as f64 / mins,
            all.metres / 1000.0 / mins,
            all.red_runs,
            all.hard_brakes,
            all.stop_jerks,
            all.gave_up,
            all.overlaps,
            all.close_calls,
            all.plan_secs * 1000.0 / all.ticks.max(1) as f64,
            all.head_list(),
        )
    }
}

/// The cycles of a waits-for graph in which every car waits for at most one other
/// (`waits[k]` the index it waits for): the members of each cycle.
pub fn waits_for_cycles(waits: &[Option<usize>]) -> Vec<Vec<usize>> {
    // 0 not seen, 1 on the path being followed, 2 done
    let mut mark = vec![0u8; waits.len()];
    let mut cycles = Vec::new();
    for start in 0..waits.len() {
        if mark[start] != 0 {
            continue;
        }
        let mut path = Vec::new();
        let mut k = start;
        loop {
            if mark[k] == 1 {
                // back on the path: from where it was first met on, a cycle
                let from = path.iter().position(|&p| p == k).unwrap_or(0);
                cycles.push(path[from..].to_vec());
                break;
            }
            if mark[k] == 2 {
                break;
            }
            mark[k] = 1;
            path.push(k);
            match waits[k] {
                Some(n) if n < waits.len() => k = n,
                _ => break,
            }
        }
        for p in path {
            mark[p] = 2;
        }
    }
    cycles
}

/// The head of the chain car `k` waits in (the first that waits for nobody waiting, or a
/// member of the cycle the chain runs into: then None).
pub fn chain_head(waits: &[Option<usize>], k: usize) -> Option<usize> {
    let mut seen = hashbrown::HashSet::new();
    let mut at = k;
    loop {
        if !seen.insert(at) {
            return None;
        }
        match waits[at] {
            Some(n) if n < waits.len() => at = n,
            _ => return Some(at),
        }
    }
}

impl TrafficSim {
    /// Before the cars move (with `OMSI_TRAFFIC_STATS`): where each one is.
    pub fn stats_before(&mut self) {
        let Some(s) = self.stats.as_mut() else { return };
        s.before.clear();
        for c in &self.cars {
            s.before.insert(c.id, (c.state.lane, c.state.odometer, c.state.acc));
        }
    }

    /// After the cars have moved (`dt` s; `removed` the indices taken off this tick): what
    /// happened, and every `SAMPLE` seconds a look at the whole traffic.
    pub fn stats_after(&mut self, dt: f32, removed: &[usize]) {
        let Some(mut s) = self.stats.take() else { return };
        s.t += dt;
        s.window.ticks += 1;
        s.window.plan_secs += self.tick_split[0] + self.tick_split[1];
        for (i, c) in self.cars.iter().enumerate() {
            if removed.contains(&i) {
                continue;
            }
            let Some(&(lane, odo, acc)) = s.before.get(&c.id) else { continue };
            s.window.metres += (c.state.odometer - odo).max(0.0) as f64;
            // (from walking pace on: a car creeping at its line that stops again shows -8
            // m/s² for a frame, which is a jerk but no near miss; counted apart)
            if c.state.acc < HARD_BRAKE && acc >= HARD_BRAKE {
                if c.state.speed < BRAKE_FROM {
                    s.window.stop_jerks += 1;
                } else {
                    s.window.hard_brakes += 1;
                }
                if omsi_cfg::flags::OMSI_DEBUG_STUCK.is_set() {
                    log::info!("stats t={:.1}: car {} brakes {:.1} m/s² at {:.1} m/s on lane {} for {:?} (lead {:?}, junction {})", s.t, c.id, c.state.acc, c.state.speed, c.state.lane, c.why, c.lead_info, c.junction_why);
                }
            }
            if c.state.lane != lane && self.net.lanes[c.state.lane].kind == LaneKind::Street {
                let into = c.state.lane;
                // into a junction: a lane that crosses others, from one that does not
                if !self.net.crossings[into].is_empty() && self.net.crossings[lane].is_empty() {
                    s.window.junction_entries += 1;
                }
                if let Some((ci, li)) = entry_light(&self.net, &[(lane, 0.0), (into, 0.0)], 1) {
                    if let Some(ctl) = self.lights.get(ci) {
                        let red = matches!(TrafficLightController::aspect(ctl.state(li)), Aspect::Red | Aspect::RedYellow);
                        if red && c.amber != Some((ci, li)) {
                            s.window.red_runs += 1;
                            if omsi_cfg::flags::OMSI_DEBUG_STUCK.is_set() {
                                log::info!("stats t={:.1}: car {} into lane {into} on red (light {ci}/{li}) at {:.1} m/s", s.t, c.id, c.state.speed);
                            }
                        }
                    }
                }
            }
        }
        if s.t >= s.next_sample {
            s.next_sample += SAMPLE;
            self.stats_sample(&mut s);
        }
        if s.t >= s.next_line {
            s.next_line += WINDOW;
            let w = std::mem::take(&mut s.window);
            if let Some(f) = s.out.as_mut() {
                let _ = writeln!(
                    f,
                    "{:.0},{:.1},{:.1},{},{:.1},{:.1},{:.2},{:.2},{:.2},{},{:.2},{},{},{},{:.2},{},{},{},{},{},{}",
                    s.t,
                    w.mean_cars(),
                    w.mean(w.gone),
                    self.dormant.len(),
                    w.mean_kmh(),
                    w.moving_speed_sum / w.moving.max(1) as f64 * 3.6,
                    w.mean(w.stood30),
                    w.mean(w.stood60),
                    w.mean(w.stood120),
                    w.max_stood60,
                    w.mean(w.in_cycles),
                    w.max_in_cycles,
                    w.cycles,
                    w.junction_entries,
                    w.metres / 1000.0,
                    w.red_runs,
                    w.hard_brakes,
                    w.gave_up,
                    w.overlaps,
                    w.close_calls,
                    w.head_list(),
                );
                let _ = f.flush();
            }
            s.total.add(&w);
        }
        self.stats = Some(s);
    }

    /// One look at the whole traffic: who stands, the waits-for graph, the bodies.
    fn stats_sample(&self, s: &mut TrafficStats) {
        let w = &mut s.window;
        w.samples += 1;
        let n = self.cars.len();
        w.cars += n as u64;
        w.gone += self.cars.iter().filter(|c| c.gone).count() as u64;
        let mut stood60 = 0;
        for c in &self.cars {
            let v = c.state.speed.max(0.0) as f64;
            w.speed_sum += v;
            if v > 0.5 {
                w.moving += 1;
                w.moving_speed_sum += v;
            }
            let stood = c.stopped.max(c.progress.1);
            w.stood30 += (stood > 30.0) as u64;
            stood60 += (stood > 60.0) as usize;
            w.stood120 += (stood > 120.0) as u64;
        }
        w.stood60 += stood60 as u64;
        w.max_stood60 = w.max_stood60.max(stood60 as u64);
        // the waits-for graph among the cars that stand
        let index: HashMap<u64, usize> = self.cars.iter().enumerate().map(|(k, c)| (c.id, k)).collect();
        let standing = |c: &AiCar| c.stopped.max(c.progress.1) > WAITING;
        let waits: Vec<Option<usize>> = self
            .cars
            .iter()
            .map(|c| {
                if !standing(c) {
                    return None;
                }
                c.waits_on.and_then(|id| index.get(&id).copied()).filter(|&k| standing(&self.cars[k]))
            })
            .collect();
        let cycles = waits_for_cycles(&waits);
        let in_cycles: usize = cycles.iter().map(|c| c.len()).sum();
        w.cycles += cycles.len() as u64;
        w.in_cycles += in_cycles as u64;
        w.max_in_cycles = w.max_in_cycles.max(in_cycles as u64);
        let debug = omsi_cfg::flags::OMSI_DEBUG_STUCK.is_set();
        let mut logged = 0;
        for (k, c) in self.cars.iter().enumerate() {
            if c.stopped.max(c.progress.1) <= 60.0 {
                continue;
            }
            let reason: &'static str = match chain_head(&waits, k) {
                None => "cycle",
                Some(h) => {
                    let head = &self.cars[h];
                    match head.why.0 {
                        "lead" if head.waits_on.is_some() => "lead-moving",
                        "" if head.at_stop() => "bus-stop",
                        "" => "nothing",
                        r => r,
                    }
                }
            };
            *w.heads.entry(reason).or_default() += 1;
            // (the heads of the longest waits, once a minute)
            if debug && logged < 4 && (s.t / WINDOW).fract() * WINDOW < SAMPLE {
                if let Some(h) = chain_head(&waits, k).filter(|&h| h == k) {
                    let head = &self.cars[h];
                    logged += 1;
                    log::info!("stats t={:.0}: car {} stood {:.0} s at ({:.1}, {:.1}) lane {}: {:?} waits on {:?} yield_to {:?} junction {}", s.t, head.id, head.stopped.max(head.progress.1), head.vehicle.position.x, head.vehicle.position.y, head.state.lane, head.why, head.waits_on, head.yield_to, head.junction_why);
                }
            }
        }
        if debug && !cycles.is_empty() && (s.t / WINDOW).fract() * WINDOW < SAMPLE {
            for cyc in cycles.iter().take(3) {
                let desc: Vec<String> = cyc.iter().map(|&k| { let c = &self.cars[k]; format!("{} ({} lane {} at {:.0},{:.0})", c.id, c.why.0, c.state.lane, c.vehicle.position.x, c.vehicle.position.y) }).collect();
                log::info!("stats t={:.0}: waits-for cycle of {}: {}", s.t, cyc.len(), desc.join(" -> "));
            }
        }
        // bodies inside each other, or closing fast within half a metre (a grid of 10 m
        // cells: only neighbours are compared)
        let feet = self.footprints();
        let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        let cell = |p: DVec2| ((p.x / 10.0).floor() as i64, (p.y / 10.0).floor() as i64);
        for (k, f) in feet.iter().enumerate() {
            grid.entry(cell(f.center)).or_default().push(k);
        }
        let mut touching = hashbrown::HashSet::new();
        let mut close = hashbrown::HashSet::new();
        for (a, fa) in feet.iter().enumerate() {
            let (cx, cy) = cell(fa.center);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for &b in grid.get(&(cx + dx, cy + dy)).map(|v| v.as_slice()).unwrap_or(&[]) {
                        let fb = &feet[b];
                        if b <= a || fb.car == fa.car || (fa.z - fb.z).abs() > 3.0 {
                            continue;
                        }
                        // (only a long vehicle reaches farther than its neighbour cells)
                        let (ia, ib) = (self.cars[fa.car].id, self.cars[fb.car].id);
                        let key = (ia.min(ib), ia.max(ib));
                        if fa.overlaps(fb, -0.2) {
                            touching.insert(key);
                        } else if fa.overlaps(fb, 0.25) {
                            let rel = fa.fwd * fa.speed as f64 - fb.fwd * fb.speed as f64;
                            if rel.length() > 3.0 {
                                close.insert(key);
                            }
                        }
                    }
                }
            }
        }
        w.overlaps += touching.difference(&s.touching).count() as u64;
        w.close_calls += close.difference(&s.close).count() as u64;
        if debug {
            for p in touching.difference(&s.touching).take(3) {
                if let (Some(&a), Some(&b)) = (index.get(&p.0), index.get(&p.1)) {
                    let (ca, cb) = (&self.cars[a], &self.cars[b]);
                    log::info!("stats t={:.0}: overlap of car {} ({} lane {} v {:.1}) and car {} ({} lane {} v {:.1}) at ({:.1}, {:.1})", s.t, ca.id, ca.why.0, ca.state.lane, ca.state.speed, cb.id, cb.why.0, cb.state.lane, cb.state.speed, ca.vehicle.position.x, ca.vehicle.position.y);
                }
            }
        }
        s.touching = touching;
        s.close = close;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ring_of_waiting_cars_is_one_cycle() {
        // 0 -> 1 -> 2 -> 0, 3 -> 1 (queued into the ring), 4 alone, 5 -> 4
        let waits = vec![Some(1), Some(2), Some(0), Some(1), None, Some(4)];
        let cycles = waits_for_cycles(&waits);
        assert_eq!(cycles.len(), 1);
        let mut c = cycles[0].clone();
        c.sort();
        assert_eq!(c, vec![0, 1, 2]);
        assert_eq!(chain_head(&waits, 3), None, "queued into a cycle");
        assert_eq!(chain_head(&waits, 5), Some(4));
        assert_eq!(chain_head(&waits, 4), Some(4));
    }

    #[test]
    fn two_cars_waiting_on_each_other_are_a_cycle() {
        let cycles = waits_for_cycles(&[Some(1), Some(0)]);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].len(), 2);
        assert!(waits_for_cycles(&[None, Some(0), Some(1)]).is_empty());
    }
}
