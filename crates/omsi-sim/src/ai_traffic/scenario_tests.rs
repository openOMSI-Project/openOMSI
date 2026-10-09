//! Whole traffic scenes run headless, for the ways traffic used to lock up: four cars at
//! a junction without priorities, a queue beyond a junction, a side road at a busy main
//! road, a queue moving off at a green light.

use super::*;
use crate::ai_traffic::setup::RandomTypes;
use crate::traffic::{LaneBuilder, LaneKey};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A vehicle type with no model and no scripts, in a folder of its own.
struct Fixture {
    dir: std::path::PathBuf,
    ty: Arc<VehicleType>,
}

impl Fixture {
    fn new() -> Fixture {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "omsi-traffic-scenario-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("car.bus"), "[model]\nmodel.cfg\n").unwrap();
        std::fs::write(dir.join("model.cfg"), "").unwrap();
        let ty = Arc::new(VehicleType::load_ai(&dir, &dir.join("car.bus")).unwrap());
        Fixture { dir, ty }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn traffic(f: &Fixture, mut net: Network) -> TrafficSim {
    net.build_grid();
    net.compute_reach();
    let random = RandomTypes { types: Vec::new(), groups: Vec::new(), group_curves: false, group_uvg: Vec::new(), uvg_defaults: Vec::new() };
    TrafficSim::assemble(&f.dir, net, random, Vec::new(), HashMap::new(), (Vec::new(), Vec::new()), Vec::new(), (1.0, 0), 0)
}

fn add_car(t: &mut TrafficSim, f: &Fixture, lane: usize, s: f32, seed: u64, speed: Option<f32>) -> u64 {
    let vehicle = VehicleInstance::new(f.ty.clone(), crate::VehicleHost::new(crate::SimClock::default()));
    t.place_car(vehicle, LaneKind::Street, lane, s, f.ty.clone(), seed, None, None, speed, None)
}

fn street(start: DVec3, heading: f64, length: f64) -> crate::traffic::Lane {
    LaneBuilder::arc(start, heading, length, 0.0, 0.0, LaneKind::Street, 3.0)
}

/// A junction object (one key, a path per direction) where four straight roads meet,
/// 150 m of road before each of its paths and `exit` m after: lanes 0..4 the roads in
/// (from the south, west, north, east), 4..8 the junction's paths, 8..12 the roads out.
/// Without priorities: everybody gives way to the right.
fn crossroads(exit: f64) -> Network {
    let half = 12.0;
    let o = 1.75;
    // (start of the junction path, heading) north, east, south, west bound
    let dirs = [
        (DVec3::new(o, -half, 0.0), 0.0),
        (DVec3::new(-half, -o, 0.0), 90.0),
        (DVec3::new(-o, half, 0.0), 180.0),
        (DVec3::new(half, o, 0.0), 270.0),
    ];
    let back = |p: DVec3, h: f64, d: f64| {
        let r = h.to_radians();
        p - DVec3::new(r.sin() * d, r.cos() * d, 0.0)
    };
    let mut lanes = Vec::new();
    for &(p, h) in &dirs {
        lanes.push(street(back(p, h, 150.0), h, 150.0));
    }
    for (k, &(p, h)) in dirs.iter().enumerate() {
        let mut l = street(p, h, 2.0 * half);
        l.source = 2;
        l.key = Some(LaneKey { tile: (0, 0), id: 1, path: k as u16 });
        lanes.push(l);
    }
    for k in 0..4 {
        let end = lanes[4 + k].end();
        lanes.push(street(end, dirs[k].1, exit));
    }
    let mut net = Network { lanes, ..Default::default() };
    net.link(1.5);
    net
}

fn run(t: &mut TrafficSim, secs: f32) {
    for _ in 0..(secs / 0.05) as usize {
        t.tick(0.05, None);
    }
}

/// The car's front, metres past the junction's middle along its own road (+ = through).
fn past_middle(t: &TrafficSim, id: u64) -> Option<f64> {
    let c = t.cars.iter().find(|c| c.id == id)?;
    let h = c.vehicle.heading.to_radians();
    let fwd = DVec2::new(h.sin(), h.cos());
    Some(c.vehicle.position.truncate().dot(fwd) + c.state.front as f64)
}

#[test]
fn four_cars_at_a_junction_without_priorities_all_get_through() {
    // everybody has somebody on the right: by the rules alone nobody would ever go
    let f = Fixture::new();
    let mut t = traffic(&f, crossroads(150.0));
    let ids: Vec<u64> = (0..4).map(|k| add_car(&mut t, &f, k, 135.0, 0x51 + k as u64 * 0x1000, Some(4.0))).collect();
    run(&mut t, 40.0);
    for id in ids {
        if let Some(p) = past_middle(&t, id) {
            assert!(p > 15.0, "car {id} has got only {p:.1} m past the junction's middle");
        }
    }
}

#[test]
fn a_car_does_not_drive_into_a_junction_it_cannot_leave() {
    // a queue crawls along the road beyond the junction with no room for another car:
    // the car coming up stops at the junction's line, not in its middle
    let f = Fixture::new();
    let mut t = traffic(&f, crossroads(14.0));
    // (the road out ends in 14 m: the queue on it stands at its end)
    let lane_out = 8;
    let front = add_car(&mut t, &f, lane_out, 10.0, 0x77, Some(0.0));
    let second = add_car(&mut t, &f, lane_out, 3.0, 0x78, Some(0.0));
    // the car that comes up from the south
    let me = add_car(&mut t, &f, 0, 90.0, 0x79, Some(8.0));
    for _ in 0..(30.0 / 0.05) as usize {
        // the two ahead stay where they are (a queue that does not move on)
        for c in t.cars.iter_mut().filter(|c| c.id == front || c.id == second) {
            c.state.speed = 0.0;
            c.state.max_speed_kmh = 0.0;
        }
        t.tick(0.05, None);
    }
    let c = t.cars.iter().find(|c| c.id == me).expect("still there");
    assert!(c.state.lane == 0, "the car drove into the junction (lane {} s {:.1})", c.state.lane, c.state.s);
    assert!(c.state.speed < 0.1, "it stands at the line ({:.1} m/s)", c.state.speed);
}

#[test]
fn the_exit_counts_a_crawling_queue_where_it_will_stop() {
    let way = [(10, -20.0), (20, 5.0), (21, 15.0)];
    // the last car of the queue 4 m beyond the exit's start, crawling at 1.8 m/s (it will
    // be 0.8 m further on when it has stopped): no room for 7 m
    assert!(queued_exit_vehicle(&way, (20, 5.0), 7.0, [(20, 4.0, 1.8)]).is_some());
    // driving off at 8 m/s: room
    assert!(queued_exit_vehicle(&way, (20, 5.0), 7.0, [(20, 4.0, 8.0)]).is_none());
}

/// A main road west to east (lane 0 in, the junction's path 1, lane 2 out) and a side road
/// from the south (lane 3) whose path through the junction (4) crosses it, `[rule]`
/// priorities as the stock maps set them.
fn side_road() -> Network {
    let key = |path: u16| Some(LaneKey { tile: (0, 0), id: 2, path });
    let main_in = street(DVec3::new(-200.0, 0.0, 0.0), 90.0, 190.0);
    let mut main_j = street(DVec3::new(-10.0, 0.0, 0.0), 90.0, 20.0);
    let main_out = street(DVec3::new(10.0, 0.0, 0.0), 90.0, 300.0);
    let side_in = street(DVec3::new(0.0, -110.0, 0.0), 0.0, 100.0);
    let mut side_j = street(DVec3::new(0.0, -10.0, 0.0), 0.0, 20.0);
    let side_out = street(DVec3::new(0.0, 10.0, 0.0), 0.0, 300.0);
    main_j.source = 2;
    main_j.key = key(0);
    main_j.priority = 192.0;
    side_j.source = 2;
    side_j.key = key(1);
    side_j.priority = 64.0;
    let mut net = Network { lanes: vec![main_in, main_j, main_out, side_in, side_j, side_out], ..Default::default() };
    net.link(1.5);
    net
}

/// When the side road's car (waiting at its line) gets across a main road whose queue
/// crawls past at `v` m/s, a new car put on whenever the last has moved `spacing` m on
/// (None: not within `secs`).
fn side_road_crossing(v: f32, spacing: f32, secs: f32) -> Option<f32> {
    let f = Fixture::new();
    let mut t = traffic(&f, side_road());
    let me = add_car(&mut t, &f, 3, 80.0, 0x99, Some(0.0));
    let mut seed = 0x1234;
    // the queue along the whole main road already
    for (lane, len) in [(0usize, 190.0f32), (1, 20.0), (2, 300.0)] {
        let mut s = 3.0;
        while s < len - 3.0 {
            seed += 0x101;
            add_car(&mut t, &f, lane, s, seed, Some(v));
            s += spacing;
        }
    }
    let mut time = 0.0f32;
    while time < secs {
        if !t.cars.iter().any(|c| c.state.lane == 0 && c.state.s < spacing) {
            seed += 0x101;
            add_car(&mut t, &f, 0, 2.0, seed, Some(v));
        }
        for c in t.cars.iter_mut().filter(|c| c.id != me) {
            c.state.max_speed_kmh = v * 3.6;
        }
        t.tick(0.05, None);
        time += 0.05;
        if t.cars.iter().find(|c| c.id == me).is_none_or(|c| c.state.lane == 5 || (c.state.lane == 4 && c.state.s > 15.0)) {
            return Some(time);
        }
    }
    None
}

#[test]
fn a_side_road_car_gets_into_a_main_road_that_never_leaves_a_gap() {
    // a queue rolling past at 3 or 5 m/s, a car every 9 m: never a gap any driver takes.
    // After its long wait the side road's car keeps a claim on its way, the cars not yet
    // at the junction let it across (it used to stand there for good: its claim was taken
    // for a stalled car's and held nobody back)
    for v in [3.0, 5.0] {
        let at = side_road_crossing(v, 9.0, 150.0);
        assert!(at.is_some_and(|t| t < LONG_WAIT_CLAIM + 40.0), "queue at {v} m/s: across after {at:?} s");
    }
    // a queue that crawls (1-1.5 m/s) leaves room enough between its cars
    assert!(side_road_crossing(1.5, 7.5, 150.0).is_some_and(|t| t < 40.0));
}

#[test]
fn a_queue_moves_off_without_closing_up_or_braking_hard() {
    // ten cars standing nose to tail move off together (the Intelligent Driver Model):
    // nobody runs into the one ahead, nobody brakes hard, and the queue is on its way
    let f = Fixture::new();
    let mut lanes = vec![street(DVec3::ZERO, 0.0, 900.0)];
    lanes[0].speed_limit_kmh = 50.0;
    let mut net = Network { lanes, ..Default::default() };
    net.link(1.5);
    let mut t = traffic(&f, net);
    let ids: Vec<u64> = (0..10).map(|k| add_car(&mut t, &f, 0, 200.0 - k as f32 * 7.0, 0x300 + k as u64 * 0x77, Some(0.0))).collect();
    let mut min_gap = f32::MAX;
    let mut hardest = 0.0f32;
    for _ in 0..(40.0 / 0.05) as usize {
        t.tick(0.05, None);
        for w in ids.windows(2) {
            let (Some(a), Some(b)) = (t.cars.iter().find(|c| c.id == w[0]), t.cars.iter().find(|c| c.id == w[1])) else { continue };
            min_gap = min_gap.min(a.state.s - a.state.rear - (b.state.s + b.state.front));
        }
        hardest = hardest.min(t.cars.iter().map(|c| c.state.acc).fold(0.0, f32::min));
    }
    assert!(min_gap > 1.0, "two cars came within {min_gap:.2} m");
    assert!(hardest > -3.0, "a car braked at {hardest:.1} m/s²");
    for id in ids {
        let c = t.cars.iter().find(|c| c.id == id).unwrap();
        assert!(c.state.speed > 8.0, "car {id} at {:.1} m/s after 40 s", c.state.speed);
    }
}

#[test]
fn a_car_does_not_change_lanes_into_the_players_bus() {
    // two lanes north, the car on the right one at 100 m; the player's bus on the left one
    let f = Fixture::new();
    let lanes = vec![street(DVec3::ZERO, 0.0, 300.0), street(DVec3::new(-3.5, 0.0, 0.0), 0.0, 300.0)];
    let mut net = Network { lanes, ..Default::default() };
    net.link(1.5);
    let mut t = traffic(&f, net);
    let id = add_car(&mut t, &f, 0, 100.0, 0x42, Some(10.0));
    let i = t.cars.iter().position(|c| c.id == id).unwrap();
    let bus_at = |y: f64, v: f32| -> PlayerBox { (DVec3::new(-3.5, y, 0.0), 0.0, 6.0, 1.25, v) };
    // beside it, or coming up fast just behind: not into that lane
    for (y, v) in [(100.0, 10.0), (88.0, 14.0)] {
        t.player = Some(bus_at(y, v));
        assert!(!t.players_let_in(i, 1, 100.0), "bus at {y} m, {v} m/s");
    }
    // far behind, or well ahead and faster: room
    for (y, v) in [(30.0, 10.0), (140.0, 12.0)] {
        t.player = Some(bus_at(y, v));
        assert!(t.players_let_in(i, 1, 100.0), "bus at {y} m, {v} m/s");
    }
}

/// When the car on a lane that runs into another (lane 1 joining lane 0, both going on as
/// lane 2) gets onto the lane beyond the joint while lane 0's queue rolls past at `v` m/s,
/// a car every `spacing` m (None: not within `secs`).
fn zip_merge(v: f32, spacing: f32, secs: f32) -> Option<f32> {
    let f = Fixture::new();
    let main = LaneBuilder::polyline(vec![DVec3::ZERO, DVec3::new(0.0, 150.0, 0.0)], LaneKind::Street, 3.0);
    let side = LaneBuilder::polyline(vec![DVec3::new(17.32, 120.0, 0.0), DVec3::new(0.0, 150.0, 0.0)], LaneKind::Street, 3.0);
    let on = LaneBuilder::polyline(vec![DVec3::new(0.0, 150.0, 0.0), DVec3::new(0.0, 450.0, 0.0)], LaneKind::Street, 3.0);
    let mut net = Network { lanes: vec![main, side, on], ..Default::default() };
    net.link(1.5);
    let mut t = traffic(&f, net);
    let side_len = t.net.lanes[1].length();
    let me = add_car(&mut t, &f, 1, side_len - 12.0, 0x5a, Some(0.0));
    let mut seed = 0x700;
    for (lane, len) in [(0usize, 150.0f32), (2, 300.0)] {
        let mut s = 3.0;
        while s < len - 3.0 {
            seed += 0x101;
            add_car(&mut t, &f, lane, s, seed, Some(v));
            s += spacing;
        }
    }
    let mut time = 0.0f32;
    while time < secs {
        if !t.cars.iter().any(|c| c.state.lane == 0 && c.state.s < spacing) {
            seed += 0x101;
            add_car(&mut t, &f, 0, 2.0, seed, Some(v));
        }
        for c in t.cars.iter_mut().filter(|c| c.id != me) {
            c.state.max_speed_kmh = v * 3.6;
        }
        t.tick(0.05, None);
        time += 0.05;
        if t.cars.iter().find(|c| c.id == me).is_none_or(|c| c.state.lane == 2) {
            return Some(time);
        }
    }
    None
}

#[test]
fn a_car_at_a_merge_gets_its_turn_in_a_rolling_queue() {
    // the zip: after a few seconds at the joint the car goes next, and the queue on the
    // other lane lets it in
    for (v, spacing) in [(1.5, 7.0), (3.0, 7.0), (4.0, 9.0)] {
        let at = zip_merge(v, spacing, 120.0);
        assert!(at.is_some_and(|t| t < 30.0), "queue at {v} m/s: merged after {at:?} s");
    }
}

#[test]
fn short_consecutive_paths_do_not_send_cars_backwards() {
    let f = Fixture::new();
    // Artificial straight road with two pieces shorter than the linking tolerance.
    // A backward link makes the planned way jump back and leaves the body following
    // a folded path, even though there is no obstacle on the road.
    let mut lanes = Vec::new();
    let mut y = 0.0;
    for len in [30.0, 0.47, 1.0, 25.0, 600.0] {
        lanes.push(street(DVec3::new(0.0, y, 0.0), 0.0, len));
        y += len;
    }
    let mut net = Network { lanes, ..Default::default() };
    net.link(1.5);
    for seed in 1..=16 {
        let mut copy = Network { lanes: net.lanes.clone(), ..Default::default() };
        copy.link(1.5);
        let mut t = traffic(&f, copy);
        let id = add_car(&mut t, &f, 0, 26.0, seed, Some(5.0));
        let mut previous = 26.0;
        for _ in 0..200 {
            t.tick(0.05, None);
            let car = t.cars.iter().find(|c| c.id == id).expect("continuous road");
            let y = car.state.way_point(&t.net, 0.0).y;
            assert!(y >= previous - 0.01, "seed {seed}: {previous} -> {y}");
            previous = y;
        }
        assert!(previous > 60.0, "seed {seed} stuck at {previous}");
        let car = t.cars.iter().find(|c| c.id == id).unwrap();
        assert!(car.vehicle.position.y > 60.0, "seed {seed}: body stuck at {:?}", car.vehicle.position);
    }
}

#[test]
fn cars_still_cross_a_finite_exit_when_its_short_return_is_not_chosen() {
    let f = Fixture::new();
    let a = street(DVec3::ZERO, 0.0, 30.0);
    let finite = street(a.end(), 10.0, 20.0);
    let short = street(finite.end(), 10.0, 0.47);
    let next = street(short.end(), 10.0, 1.0);
    let exit = street(next.end(), 10.0, 280.0);
    let long = street(a.end(), 350.0, 700.0);
    let lanes = vec![a, finite, short, next, exit, long];
    let mut crossed = [0; 2];
    for seed in 1..=32 {
        let mut net = Network { lanes: lanes.clone(), ..Default::default() };
        net.link(1.5);
        let mut t = traffic(&f, net);
        let id = add_car(&mut t, &f, 0, 26.0, seed, Some(5.0));
        run(&mut t, 15.0);
        let car = t.cars.iter().find(|c| c.id == id).expect("ample road beyond the join");
        match car.state.lane {
            4 => {
                crossed[0] += 1;
                assert!(car.state.s > 30.0, "car did not drive past the short join");
                assert!(car.vehicle.position.y > 70.0, "body did not cross the join");
            }
            5 => crossed[1] += 1,
            other => panic!("car is still on approach lane {other}"),
        }
    }
    assert!(crossed.iter().all(|&n| n > 0), "a usable exit lost its traffic: {crossed:?}");
}
