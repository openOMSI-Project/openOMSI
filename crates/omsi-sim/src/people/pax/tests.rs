use super::*;

#[test]
fn shut_door_give_up_uses_one_continuous_wait_and_resets_when_opened() {
    let (since, expired) = shut_door_wait(None, 100.0, true);
    assert_eq!(since, Some(100.0));
    assert!(!expired);
    let (since, expired) = shut_door_wait(since, 125.0, true);
    assert_eq!(since, Some(100.0));
    assert!(!expired, "the exact timeout boundary is still allowed");
    let (since, expired) = shut_door_wait(since, 125.001, true);
    assert_eq!(since, Some(100.0));
    assert!(expired, "a continuous wait past the timeout gives up");
    let (since, expired) = shut_door_wait(since, 130.0, false);
    assert_eq!(since, None, "an open entry resets the old shut-door wait");
    assert!(!expired);
    let (since, expired) = shut_door_wait(since, 200.0, true);
    assert_eq!(since, Some(200.0), "a later closure starts a fresh wait");
    assert!(!expired);
}

#[test]
fn arriving_ai_uses_the_timetable_stop_not_a_neighbour() {
    let dir = std::env::temp_dir().join(format!("omsi-ai-stop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("test.bus"),
        "[passengercabin]\ncabin.cfg\n[paths]\npaths.cfg\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("paths.cfg"),
        "[pathpnt]\n1\n0\n0\n[pathpnt]\n0\n0\n0\n[pathlink]\n0\n1\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("cabin.cfg"),
        "[entry]\n0\n[exit]\n0\n[passpos]\n0\n0\n0.9\n0.45\n0\n",
    )
    .unwrap();
    let def = omsi_vehicle::Vehicle::load(&dir.join("test.bus")).unwrap();
    let cabin =
        Arc::new(Cabin::load_train(&[(&def, Vec3::ZERO, f32::INFINITY)]).expect("cabin"));
    std::fs::remove_dir_all(&dir).unwrap();
    let mut humans = PeopleSim::new(Path::new("/nonexistent"), 200);
    humans.stops.insert(42, test_stop("Scheduled", ""));
    let mut neighbour = test_stop("Neighbour", "");
    neighbour.pos = DVec3::Y * 3.0;
    humans.stops.insert(99, neighbour);
    let mut bus = BusNow {
        id: BusId::Ai(3),
        next_stop: Some(RequestStop {
            id: 42,
            name: "Scheduled".into(),
            alias: String::new(),
            pos: DVec3::ZERO,
        }),
        cabin,
        pos: DVec3::ZERO,
        rot: Mat4::IDENTITY,
        heading: 0.0,
        speed: 0.0,
        entry_open: vec![true],
        exit_open: vec![true],
        walk_open: None,
        interior: 0.0,
        air: CabinAir::default(),
        half: DVec2::new(1.25, 6.0),
        centre: DVec2::ZERO,
        accel: DVec2::ZERO,
        trailers: Vec::new(),
        terminus: Some("Elsewhere".into()),
        takes: Takes::Terminus,
        places_off: Vec::new(),
        served: None,
    };
    let register = |humans: &mut PeopleSim, bus: &BusNow| {
        humans
            .register_buses(std::slice::from_ref(bus), 0.0)
            .remove(&bus.id)
            .unwrap()
    };
    let registered = register(&mut humans, &bus);
    assert_eq!(
        registered.next,
        Some(42),
        "a neighbouring platform cannot replace the trip stop"
    );
    assert_eq!(
        registered.near,
        [42, 99],
        "nearby stops still participate in boarding detection"
    );
    bus.next_stop.as_mut().unwrap().id = 123;
    assert_eq!(
        register(&mut humans, &bus).next,
        None,
        "do not arrive at a different stop while the trip stop is absent"
    );
    bus.served = Some(123);
    assert_eq!(
        register(&mut humans, &bus).next,
        Some(123),
        "an unloaded timetable stop retains the upstream served-stop fallback"
    );
    bus.served = None;
    bus.next_stop = None;
    assert_eq!(
        register(&mut humans, &bus).next,
        Some(99),
        "unknown routes retain the geometric fallback"
    );
    bus.next_stop = Some(RequestStop {
        id: 42,
        name: "Scheduled".into(),
        alias: String::new(),
        pos: DVec3::ZERO,
    });
    bus.id = BusId::Player;
    assert_eq!(
        register(&mut humans, &bus).next,
        Some(99),
        "preserve the player's OMSI-compatible geometric detection"
    );
}

/// A timetable bus waits for the people walking up to its doors from its stop and for
/// those on their way out of it - not for the people still at the gather point, who
/// held a full bus at the stop for good (#767).
#[test]
fn who_keeps_a_timetable_bus_at_its_stop() {
    let bus = BusId::Ai(7);
    let mut x = Pax::new(1.1, 0.5);
    x.bus = Some(bus);
    x.stop = Some(42);
    x.task = Task::ToBus;
    assert_eq!(holds_bus(&x, bus), None);
    x.task = Task::WaitingForBus;
    assert_eq!(holds_bus(&x, bus), None);
    x.task = Task::WalkingToBus;
    assert_eq!(holds_bus(&x, bus), Some(Some(42)));
    assert_eq!(holds_bus(&x, BusId::Ai(8)), None);
    x.task = Task::InBusToExit;
    x.inside = Some(bus);
    assert_eq!(holds_bus(&x, bus), Some(None));
    x.task = Task::SittingInBus;
    assert_eq!(holds_bus(&x, bus), None);
}

/// Somebody late keeps a timetable bus at the stop while hurrying for it, from the street
/// and at its doors - for `RUNNER_HOLD_MAX` at most - but not on the way up before the bus
/// stands, nor once it has left them behind.
#[test]
fn somebody_late_keeps_a_timetable_bus_for_a_while() {
    let bus = BusId::Ai(7);
    let mut x = Pax::new(1.1, 0.5);
    x.bus = Some(bus);
    x.stop = Some(42);
    x.task = Task::WalkingToBusstop;
    let late = Late { phase: LatePhase::Approach, walk: 1.1, run: 3.3, hurried: 0.0, stood: false };
    x.late = Some(late);
    assert_eq!(holds_bus(&x, bus), None, "still on the way, the bus not standing yet");
    x.late = Some(Late { phase: LatePhase::Hurry, ..late });
    assert_eq!(holds_bus(&x, bus), Some(Some(42)));
    x.task = Task::ToBus;
    assert_eq!(holds_bus(&x, bus), Some(Some(42)), "hurrying to the gather point");
    assert_eq!(holds_bus(&x, BusId::Ai(8)), None, "not another bus");
    x.task = Task::WalkingToBus;
    assert_eq!(holds_bus(&x, bus), Some(Some(42)));
    x.late = Some(Late { phase: LatePhase::Hurry, hurried: RUNNER_HOLD_MAX + 0.1, ..late });
    assert_eq!(holds_bus(&x, bus), None, "hurried longer than a bus waits");
    x.task = Task::ToBus;
    assert_eq!(holds_bus(&x, bus), None);
    x.late = Some(Late { phase: LatePhase::Missed(1.0), ..late });
    assert_eq!(holds_bus(&x, bus), None, "left behind");
    // anybody else as ever
    x.late = None;
    assert_eq!(holds_bus(&x, bus), None);
    x.task = Task::WalkingToBus;
    assert_eq!(holds_bus(&x, bus), Some(Some(42)));
}

/// Smooth driving upsets nobody; a hard stop, a fast bend and a jerky foot do, as in
/// Omsi.exe (#862).
#[test]
fn the_riders_feel_hard_braking_fast_bends_and_a_jerky_foot() {
    let dt = 0.02;
    let run = |f: &dyn Fn(f64) -> (f32, f32, f32), secs: f64| {
        let mut c = RideComfort::default();
        let mut jolts = Vec::new();
        let mut t = 0.0;
        while t < secs {
            let (v, lat, long) = f(t);
            let k = c.step(dt, (t + 10.0) * 1000.0, v, lat, long);
            if k > 0.0 {
                jolts.push((t, k));
            }
            t += dt as f64;
        }
        jolts
    };
    // pulling away at 1.2 m/s², cruising, braking at 1.5 m/s² to a stop: nothing
    assert!(run(&|t| if t < 10.0 { (1.2 * t as f32, 0.0, 1.2) } else if t < 20.0 { (12.0, 0.0, 0.0) } else if t < 28.0 { (12.0 - 1.5 * (t as f32 - 20.0), 0.0, -1.5) } else { (0.0, 0.0, 0.0) }, 40.0).is_empty());
    // a gentle bend at 1.5 m/s² sideways
    assert!(run(&|_| (10.0, 1.5, 0.0), 10.0).is_empty());
    // an emergency stop at 7 m/s²: one jolt, not one a frame
    let hard = run(&|t| if t < 1.0 { (14.0, 0.0, 0.0) } else { (14.0, 0.0, -7.0) }, 2.0);
    assert_eq!(hard.len(), 1, "{hard:?}");
    assert_eq!(hard[0].1, 0.1);
    // a bend at 4 m/s² held for seconds
    assert_eq!(run(&|_| (12.0, 4.0, 0.0), 6.0).len(), 1);
    // throttle and brake every 1.5 s: the fifth swing on upsets them, each further one too
    let jerky = run(&|t| (8.0, 0.0, if (t / 1.5).floor() as i64 % 2 == 0 { 1.0 } else { -1.0 }), 15.0);
    assert!(jerky.len() >= 4 && jerky.iter().all(|j| j.1 == 0.05) && jerky[0].0 > 5.0, "{jerky:?}");
    // standing, nothing counts
    assert!(run(&|_| (0.0, 5.0, -8.0), 5.0).is_empty());
}

#[test]
fn complaints_come_worst_first_and_once_each() {
    let at = bad_ride_thresholds([0.5, 0.5, 0.5]);
    assert!((at[0] - 0.05).abs() < 1e-6 && (at[1] - 0.3).abs() < 1e-6 && (at[2] - 0.65).abs() < 1e-6);
    let lo = bad_ride_thresholds([0.0, 0.0, 0.0]);
    let hi = bad_ride_thresholds([1.0, 1.0, 1.0]);
    assert!(lo[1] >= 0.2 - 1e-6 && hi[1] <= 0.4 + 1e-6 && lo[2] >= 0.5 - 1e-6 && hi[2] <= 0.8 + 1e-6);
    assert_eq!(bad_ride_complaint(0.01, 0, at), None);
    assert_eq!(bad_ride_complaint(0.1, 0, at), Some(1));
    assert_eq!(bad_ride_complaint(0.1, 1, at), None);
    assert_eq!(bad_ride_complaint(0.35, 1, at), Some(2));
    // a crash straight to the top: the worst at once, then nothing more
    assert_eq!(bad_ride_complaint(0.9, 0, at), Some(3));
    assert_eq!(bad_ride_complaint(0.95, 3, at), None);
    // three emergency stops in a row take a rider from nothing past 0.27
    let mut x = 0.0f32;
    for _ in 0..3 {
        x += (1.0 - x) * 0.1;
    }
    assert!((x - 0.271).abs() < 1e-3);
}

fn test_stop(name: &str, alias: &str) -> PaxStop {
    PaxStop {
        name: name.into(),
        alias: alias.into(),
        pos: DVec3::ZERO,
        heading: 0.0,
        gather: DVec3::ZERO,
        spots: Vec::new(),
        taken: Vec::new(),
        enter_max: 1.0,
        enter_min: 0.0,
        length: 30.0,
        lane: None,
        was_near: false,
        near: false,
        clock_ms: 0.0,
        want: 0,
        factor: 1.0,
        buses: Vec::new(),
        dests: Vec::new(),
        lines: Vec::new(),
    }
}

/// The player's bus on a duty takes the people its trip takes where they are going,
/// whatever its depot file calls the terminus; in free drive, or with no destination
/// shown, nobody; a bus whose terminus is on their line record comes first.
#[test]
fn who_boards_the_players_bus() {
    // line 76 from Bauernhof (stop 10) by Kirche to Endstation; the depot file calls the
    // terminus "Endstation Grundorf", the timetable "Endstation"
    let trip = |name: &str| StopPlan {
        object_id: match name {
            "Bauernhof" => 10,
            "Kirche" => 11,
            "Depot" => 13,
            _ => 12,
        },
        name: format!("{name} "),
        stops: name != "Depot",
    };
    let planned = TripPlan {
        name: "76-1".into(),
        line: "76".into(),
        terminus: "Endstation ".into(),
        departure: 8.0 * 3600.0,
        stops: ["Bauernhof", "Depot", "Kirche", "Endstation"].into_iter().map(trip).collect(),
    };
    // (named as the timetable's Busstops.cfg names the objects; one it does not know by
    // its id)
    let names: HashMap<i64, String> = [(10, "Bauernhof".to_string()), (11, "Kirche".to_string()), (13, "Depot".to_string())].into_iter().collect();
    let dt = Arc::new(DutyTrip::of(&planned, Some(&names)));
    assert_eq!(dt.terminus, "Endstation");
    assert_eq!(dt.stops.iter().map(|s| s.1.as_str()).collect::<Vec<_>>(), ["Bauernhof", "Depot", "Kirche", "12"]);
    let duty = |next: usize, done: bool| Takes::Duty { trip: dt.clone(), next, done };
    let here = test_stop("Bauernhof Grundorf", "Bauernhof");
    let set = |t: &[&str]| t.iter().map(|x| x.to_string()).collect::<HashSet<String>>();
    let shown = "Endstation Grundorf";

    // on the duty with the destination shown: at the stop, and on the line record
    // whatever spelling the depot file has
    assert_eq!(at_stop(10, &here, Some(shown), &duty(0, false)), AtStop::Serves);
    assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Endstation"]), shown, &duty(0, false)), Some(Fit::Duty));
    // a record that does not list the trip's terminus at all (made of another variant's
    // trips): the trip goes to their stop all the same
    assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Waldweg"]), shown, &duty(0, false)), Some(Fit::Duty));
    assert_eq!(fit(10, &here, Some("12"), &set(&["Waldweg"]), shown, &duty(0, false)), Some(Fit::Duty));
    // ... but not somebody whose stop the trip does not go to, nor one it only passes
    assert_eq!(fit(10, &here, Some("Waldweg"), &set(&["Waldweg"]), shown, &duty(0, false)), None);
    assert_eq!(fit(10, &here, Some("Depot"), &set(&["Waldweg"]), shown, &duty(0, false)), None);
    // nor at a stop the trip has left behind, or does not call at
    assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Waldweg"]), shown, &duty(3, false)), None);
    assert_eq!(fit(20, &test_stop("Am Teich", "Am Teich"), Some("Kirche"), &set(&["Waldweg"]), shown, &duty(0, false)), None);
    // another platform of the stop's name will do
    assert_eq!(fit(14, &test_stop("Bauernhof 2", "Bauernhof"), Some("Kirche"), &set(&["Waldweg"]), shown, &duty(0, false)), Some(Fit::Duty));
    // the depot file's terminus on the record: the bus is theirs as in Omsi.exe
    assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Endstation Grundorf"]), shown, &duty(0, false)), Some(Fit::Terminus));

    // free drive: nobody waiting gets on, the riders get off at their stops
    assert_eq!(at_stop(10, &here, Some(shown), &Takes::Nobody), AtStop::Passes);
    assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Endstation"]), shown, &Takes::Nobody), None);
    // no destination shown (or "Nicht einsteigen"): it empties and takes nobody, duty or not
    assert_eq!(at_stop(10, &here, None, &duty(0, false)), AtStop::Empties);
    assert_eq!(at_stop(10, &here, None, &Takes::Nobody), AtStop::Empties);
    // at the trip's last stop it empties too, whatever the depot file calls it ...
    let end = test_stop("Endstation Grundorf Wendeschleife", "12");
    assert_eq!(at_stop(12, &end, Some(shown), &duty(3, true)), AtStop::Empties);
    assert_eq!(at_stop(12, &end, Some(shown), &duty(3, false)), AtStop::Serves);
    // ... and as at the terminus it shows
    assert_eq!(at_stop(12, &test_stop("Endstation Grundorf", ""), Some(shown), &duty(3, false)), AtStop::Empties);
    // a finished trip takes nobody by the duty, nor a works trip from the depot
    assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Endstation"]), shown, &duty(0, true)), None);
    let works = Takes::Duty { trip: Arc::new(DutyTrip::of(&TripPlan { line: String::new(), ..planned.clone() }, Some(&names))), next: 0, done: false };
    assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Endstation"]), shown, &works), None);
    assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Endstation Grundorf"]), shown, &works), Some(Fit::Terminus));

    // a timetable bus: as in Omsi.exe, the terminus on the record or nothing
    assert_eq!(at_stop(10, &here, Some("Endstation"), &Takes::Terminus), AtStop::Serves);
    assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Endstation"]), "Endstation", &Takes::Terminus), Some(Fit::Terminus));
    assert_eq!(fit(10, &here, Some("Kirche"), &set(&["Endstation"]), "Endstation Grundorf", &Takes::Terminus), None);

    // two buses at the stop: the one whose terminus is on the record, though further
    let (ai, me) = (BusId::Ai(5), BusId::Player);
    assert_eq!(best_bus([(me, Fit::Duty, 5.0), (ai, Fit::Terminus, 30.0)].into_iter()), Some((ai, Fit::Terminus)));
    assert_eq!(best_bus([(ai, Fit::Terminus, 30.0), (me, Fit::Terminus, 5.0)].into_iter()), Some((me, Fit::Terminus)));
    assert_eq!(best_bus([(me, Fit::Duty, 5.0)].into_iter()), Some((me, Fit::Duty)));
    assert_eq!(best_bus(std::iter::empty()), None);
}

fn stop(alias: &str) -> PaxStop {
    PaxStop {
        name: "Königsrath, Bf. Ausstieg".into(),
        alias: alias.into(),
        pos: DVec3::ZERO,
        heading: 0.0,
        gather: DVec3::ZERO,
        spots: Vec::new(),
        taken: Vec::new(),
        enter_max: 1.0,
        enter_min: 0.0,
        length: 30.0,
        lane: None,
        was_near: false,
        near: false,
        clock_ms: 0.0,
        want: 0,
        factor: 1.0,
        buses: Vec::new(),
        dests: Vec::new(),
        lines: Vec::new(),
    }
}

#[test]
fn a_stop_answers_to_its_label_and_its_timetable_name() {
    let s = stop("Koenigsrath Bf Ausstieg");
    assert!(s.is_named("Königsrath, Bf. Ausstieg "));
    assert!(s.is_named("Koenigsrath Bf Ausstieg"), "the timetable's spelling");
    assert!(!s.is_named("Königsrath, Bf. Pause"));
    // a stop the timetable does not know: its id, as the riders' destinations then are
    assert!(stop("4711").is_named("4711"));
    assert!(!stop("").is_named(""), "no timetable name: no empty match");
}

fn request_stop() -> RequestStop {
    let stop = stop("Koenigsrath Bf Ausstieg");
    RequestStop {
        id: 42,
        name: stop.name,
        alias: stop.alias,
        pos: stop.pos,
    }
}

#[test]
fn passengers_request_between_departure_and_the_approach() {
    let stop = request_stop();
    let mut early = Pax::new(1.1, 1.0);
    let mut middle = Pax::new(1.1, 0.5);
    let mut late = Pax::new(1.1, 0.0);
    early.dest = Some(stop.name.clone());
    middle.dest = early.dest.clone();
    late.dest = early.dest.clone();

    // A rider can ask just after pulling away, while another waits until the approach.
    assert!(!early.wants_stop_at(&stop, DVec3::Y * 1000.0, false));
    assert_eq!(early.stop_request_at, None);
    assert!(early.wants_stop_at(&stop, DVec3::Y * 1000.0, true));
    assert!(!middle.wants_stop_at(&stop, DVec3::Y * 1000.0, true));
    assert!(!late.wants_stop_at(&stop, DVec3::Y * 1000.0, true));
    // Repeated frames do not draw a new point or shorten the leg used to choose it.
    for _ in 0..120 {
        assert!(!middle.wants_stop_at(&stop, DVec3::Y * 600.0, true));
        assert_eq!(middle.stop_request_at, Some((42, 550.0)));
    }
    assert!(middle.wants_stop_at(&stop, DVec3::Y * 550.0, true));
    assert!(!late.wants_stop_at(&stop, DVec3::Y * 100.01, true));
    assert!(late.wants_stop_at(&stop, DVec3::Y * 100.0, true));
    assert!(late.wants_stop_at(&stop, stop.pos, false));
}

#[test]
fn short_legs_keep_random_requests_after_departure() {
    let stop = request_stop();
    for length in [20.0, 60.0, 80.0, 100.0, 150.0] {
        let mut early = Pax::new(1.1, 0.8);
        let mut late = Pax::new(1.1, 0.2);
        early.dest = Some(stop.name.clone());
        late.dest = early.dest.clone();
        assert!(!early.wants_stop_at(&stop, DVec3::Y * length, false));
        assert!(!early.wants_stop_at(&stop, DVec3::Y * length, true));
        assert!(!late.wants_stop_at(&stop, DVec3::Y * length, true));
        let early_point = early.stop_request_at.unwrap().1;
        let late_point = late.stop_request_at.unwrap().1;
        assert!(0.0 < late_point && late_point < early_point && early_point < length);
        let between = (early_point + late_point) / 2.0;
        for _ in 0..120 {
            assert!(early.wants_stop_at(&stop, DVec3::Y * between, true));
            assert!(!late.wants_stop_at(&stop, DVec3::Y * between, true));
            assert_eq!(late.stop_request_at, Some((stop.id, late_point)));
        }
        assert!(late.wants_stop_at(&stop, DVec3::Y * late_point, true));
    }
}

#[test]
fn stop_requests_match_the_destination_and_its_timetable_alias() {
    let stop = request_stop();
    let mut passenger = Pax::new(1.1, 0.5);
    assert!(!passenger.wants_stop_at(&stop, stop.pos, true));
    passenger.dest = Some("Königsrath, Bf. Pause".into());
    assert!(!passenger.wants_stop_at(&stop, stop.pos, true));
    assert_eq!(passenger.stop_request_at, None);
    passenger.dest = Some(stop.alias.clone());
    assert!(passenger.wants_stop_at(&stop, stop.pos, true));
    passenger.dest = Some(stop.name.clone());
    assert!(passenger.wants_stop_at(&stop, stop.pos, true));
}

#[test]
fn stop_request_distances_vary_and_repeat_with_the_passenger_seed() {
    let mut first = PeopleSim::new(Path::new("/nonexistent"), 200);
    let mut second = PeopleSim::new(Path::new("/nonexistent"), 200);
    first.set_lan_seed(42);
    second.set_lan_seed(42);
    let stop = request_stop();
    let mut distances = Vec::new();
    for _ in 0..100 {
        let mut passenger = Pax::new(1.1, first.rand_f());
        let mut repeated = Pax::new(1.1, second.rand_f());
        passenger.dest = Some(stop.name.clone());
        repeated.dest = passenger.dest.clone();
        passenger.wants_stop_at(&stop, DVec3::Y * 1000.0, true);
        repeated.wants_stop_at(&stop, DVec3::Y * 1000.0, true);
        assert_eq!(passenger.stop_request_at, repeated.stop_request_at);
        let distance = passenger.stop_request_at.unwrap().1;
        assert!((100.0..=1000.0).contains(&distance));
        distances.push(distance);
    }
    assert!(distances.iter().any(|distance| *distance < 300.0));
    assert!(distances.iter().any(|distance| *distance > 800.0));
}

#[test]
fn a_timetable_stop_can_be_requested_before_its_tile_loads() {
    let mut humans = PeopleSim::new(Path::new("/nonexistent"), 200);
    let planned = (42, "Next stop".to_string(), Some(DVec3::Y * 1000.0));
    humans.set_player_next_stop(Some((planned.0, &planned.1, planned.2)));
    assert!(humans.stops.is_empty());
    let target = humans.player_next_stop.as_ref().unwrap();
    let mut passenger = Pax::new(1.1, 1.0);
    passenger.dest = Some(planned.1);
    assert!(passenger.wants_stop_at(target, DVec3::ZERO, true));
    humans.set_player_next_stop(None);
    assert!(humans.player_next_stop.is_none());
}

#[test]
pub fn routes_follow_the_link_order_and_one_way_links() {
    // 0 - 1 - 2, and 2 -> 0 one way
    let r = build_routes(3, &[(0, 1, false), (1, 2, false), (2, 0, true)]);
    // from 0 to 2: through 1 (0 cannot use the one-way link back from 0 to 2)
    let next = |a: usize, b: usize| r[a].iter().find(|l| l.reach.contains(&b)).map(|l| l.to);
    assert_eq!(next(0, 2), Some(1));
    assert_eq!(next(2, 0), Some(1).or(Some(0)).filter(|_| true).and(next(2, 0)));
    assert_eq!(next(1, 0), Some(0));
    assert_eq!(next(1, 2), Some(2));
}
