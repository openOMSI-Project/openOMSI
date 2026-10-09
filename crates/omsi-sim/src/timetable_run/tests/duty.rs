//! The player's duty: trips, stops, early departures and the IBIS on the way.

use super::*;

fn early_departure_duty() -> PlayerDuty {
    PlayerDuty {
        line: "5".into(),
        tour: "1".into(),
        trips: vec![
            planned(100.0, &[(0.0, 100.0, 100.0), (500.0, 200.0, 200.0)]),
            planned(600.0, &[(500.0, 600.0, 600.0), (1000.0, 700.0, 700.0)]),
            planned(900.0, &[(1000.0, 900.0, 900.0), (1500.0, 1000.0, 1000.0)]),
        ],
        trip_index: 0,
        first_trip: 0,
        next_stop: 1,
        at_stop: false,
        arrived_late: None,
        done: false,
        served_terminus: None,
        left_late: Some(0.0),
        held_back: false,
        placed: true,
        trip_changed: false,
        skipped: None,
        run: 0,
        finished: None,
        reopened: None,
        picked: true,
        first_update: None,
        heading: 90.0,
        position: None,
    }
}

/// Aachen's ibox only has `ibox_busstop` and does `+ 1` when the TT index changes. A
/// jump of several stops must pre-position that counter so the `+ 1` lands on the new
/// stop - otherwise the list stays behind the announcement.
#[test]
fn ibox_busstop_is_prepositioned_when_the_timetable_jumps() {
    let stops = ["Elsternplatz", "Bhf. Nordspitze", "Nordsp. Bauernhof"];
    let mut hof = omsi_vehicle::Hof::default();
    for s in stops {
        hof.bus_stops.push(omsi_vehicle::hof::BusStop {
            ident: s.into(),
            strings: vec![s.into(), s.into(), s.into(), s.into()],
        });
    }
    hof.info_busstop_lists.push(stops.iter().map(|s| s.to_string()).collect());
    let mut bus = script_test_vehicle(
        "{frame}\n{end}\n",
        "ibox_busstop\nibox_routenindex\nibox_TTBusstopIndexLAST\n",
        "",
    );
    bus.host.hof = Some(std::sync::Arc::new(hof));
    bus.set_var("ibox_routenindex", 0.0);
    bus.set_var("ibox_busstop", 0.0);
    bus.host.tt_busstop_index = 0;
    let planned_stops: Vec<PlannedStop> = stops
        .iter()
        .enumerate()
        .map(|(i, name)| PlannedStop {
            object_id: i as i64,
            name: (*name).into(),
            arr: i as f64 * 60.0,
            dep: i as f64 * 60.0,
            position: Some(glam::DVec3::new(i as f64 * 100.0, 0.0, 0.0)),
            dir: StopDir::default(),
            stops: true,
        })
        .collect();
    let mut duty = PlayerDuty {
        line: "76".into(),
        tour: "2".into(),
        trips: vec![PlannedTrip {
            name: "76_2".into(),
            line: "76".into(),
            terminus: "Nordsp. Bauernhof".into(),
            departure: 0.0,
            end: planned_stops.last().unwrap().arr,
            stops: planned_stops,
        }],
        trip_index: 0,
        first_trip: 0,
        next_stop: 0,
        at_stop: false,
        arrived_late: None,
        done: false,
        served_terminus: None,
        left_late: None,
        held_back: false,
        placed: true,
        trip_changed: false,
        skipped: None,
        run: 0,
        finished: None,
        reopened: None,
        picked: true,
        first_update: None,
        heading: 90.0,
        position: None,
    };
    // jump from the first stop to the last: the script will +1 once, so leave 1 behind
    duty.next_stop = 2;
    duty.feed_host(&mut bus, 0.0);
    assert_eq!(bus.host.tt_busstop_index, 2);
    assert_eq!(bus.var("ibox_busstop"), Some(1.0), "pre-position for the script's +1");
    // after the unit's own +1 the list and the announcement both show stop 2
    bus.set_var("ibox_busstop", bus.var("ibox_busstop").unwrap() + 1.0);
    assert_eq!(bus.var("ibox_busstop"), Some(2.0));
}

/// Aachen's Fahrplan list uses `GetTTBusstopIndex` as the highlighted (bottom) stop. At
/// the start of the line that must be 0. If only a later stop's place is loaded yet,
/// placing must not jump the index to that later stop.
#[test]
fn duty_place_keeps_first_stop_while_its_place_is_unknown() {
    let mut trip = planned(100.0, &[(0.0, 100.0, 100.0), (20.0, 200.0, 200.0), (40.0, 300.0, 300.0)]);
    trip.stops[0].position = None;
    let mut d = PlayerDuty {
        line: "33".into(),
        tour: "1".into(),
        trips: vec![trip],
        trip_index: 0,
        first_trip: 0,
        next_stop: 0,
        at_stop: false,
        arrived_late: None,
        done: false,
        served_terminus: None,
        left_late: None,
        held_back: false,
        placed: true,
        trip_changed: false,
        skipped: None,
        run: 0,
        finished: None,
        reopened: None,
        picked: true,
        first_update: None,
        heading: 90.0,
        position: None,
    };
    // late for the trip, standing at stop 2 whose place is known
    d.place(glam::DVec3::new(20.0, 0.0, 0.0), 250.0);
    assert_eq!(d.next_stop, 0, "do not highlight stop #2 while stop #1 has no place");
    assert!(d.left_late.is_none());
    let mut bus = timetable_test_vehicle();
    d.feed_host(&mut bus, 250.0);
    assert_eq!(bus.host.tt_busstop_index, 0);
    assert_eq!(bus.host.tt_stops[0].0, "s0");
}

#[test]
fn serving_the_terminus_then_leaving_60m_starts_the_next_trip_early() {
    for door in ["door_0", "PAX_Exit0_Open"] {
        let mut duty = early_departure_duty();
        let mut bus = timetable_test_vehicle_with_door(door);
        bus.position.x = 500.0;
        bus.set_var(door, 1.0);
        duty.update(&mut bus, 150.0);
        assert_eq!(duty.trip_index, 0, "opening a door does not depart");
        bus.set_var(door, 0.0);
        bus.set_speed(5.0);
        bus.position.x = 559.9;
        assert_eq!(duty.update(&mut bus, 160.0), Some((-50.0, -40.0)));
        assert_eq!(duty.trip_index, 0, "less than 60 m away");
        bus.position.x = 560.0;
        duty.update(&mut bus, 161.0);
        assert_eq!(duty.trip_index, 0, "a trip due in more than five minutes waits (a bus moved to its layover)");
        duty.update(&mut bus, 301.0);
        assert_eq!((duty.trip_index, duty.next_stop), (1, 1));
        assert_eq!(bus.host.tt_busstop_index, 1);
        assert_eq!(bus.host.tt_stops[0].1, 600.0);
        // (60 m of the 500 to the next stop gone: due there at 612, #1898)
        assert_eq!(duty.delay(301.0), -311.0);
        assert!(duty.take_trip_change());
        duty.update(&mut bus, 302.0);
        assert!(!duty.take_trip_change());
        assert_eq!(duty.trip_index, 1, "advance exactly one trip");
        // The previous trip's door opening cannot finish the next trip too.
        bus.position.x = 1000.0;
        duty.update(&mut bus, 400.0);
        bus.position.x = 1060.0;
        duty.update(&mut bus, 410.0);
        assert_eq!(duty.trip_index, 1);
    }
}

#[test]
fn early_trip_progress_requires_stopping_with_a_door_open_at_the_last_stop() {
    for (speed, open) in [(5.0, 1.0), (0.0, 0.0)] {
        let mut duty = early_departure_duty();
        let mut bus = timetable_test_vehicle();
        // Doors opened at another stop do not serve the terminus.
        bus.set_var("door_0", 1.0);
        duty.update(&mut bus, 140.0);
        bus.position.x = 500.0;
        bus.set_speed(speed);
        bus.set_var("door_0", open);
        duty.update(&mut bus, 150.0);
        bus.position.x = 560.0;
        bus.set_speed(5.0);
        bus.set_var("door_0", 0.0);
        duty.update(&mut bus, 160.0);
        assert_eq!(duty.trip_index, 0);
    }
}

#[test]
fn waiting_at_the_terminus_still_uses_the_scheduled_changeover() {
    let mut duty = early_departure_duty();
    let mut bus = timetable_test_vehicle();
    bus.position.x = 500.0;
    bus.set_var("door_0", 1.0);
    duty.update(&mut bus, 150.0);
    duty.update(&mut bus, 539.0);
    assert_eq!(duty.trip_index, 0);
    duty.update(&mut bus, 540.0);
    assert_eq!((duty.trip_index, duty.next_stop), (1, 0));
    assert!(duty.served_terminus.is_none());
}

#[test]
fn early_departure_does_not_run_past_the_end_of_the_duty() {
    let mut duty = early_departure_duty();
    duty.trips.truncate(1);
    let mut bus = timetable_test_vehicle();
    bus.position.x = 500.0;
    bus.set_var("door_0", 1.0);
    duty.update(&mut bus, 150.0);
    bus.position.x = 560.0;
    bus.set_speed(5.0);
    duty.update(&mut bus, 160.0);
    assert_eq!(duty.trip_index, 0);
    assert!(duty.done);
}

#[test]
fn resumed_duty_keeps_its_trip_and_stop_before_the_first_script_frame() {
    let trips = vec![
        planned(100.0, &[(0.0, 100.0, 100.0), (500.0, 200.0, 220.0)]),
        planned(
            400.0,
            &[
                (500.0, 400.0, 400.0),
                (1000.0, 500.0, 520.0),
                (1500.0, 600.0, 600.0),
            ],
        ),
        planned(800.0, &[(1500.0, 800.0, 800.0)]),
    ];
    let mut d = PlayerDuty {
        line: "5".into(),
        tour: "65104".into(),
        trips,
        trip_index: 0,
        first_trip: 0,
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
        run: 0,
        finished: None,
        reopened: None,
        picked: false,
        first_update: None,
        heading: 90.0,
        position: None,
    };
    // The same name appears twice. Its saved ordinal, rather than its name or the
    // bus's position far from any stop, selects the second occurrence.
    d.trips[1].stops[0].name = "Central".into();
    d.trips[1].stops[1].name = "Central".into();
    d.start_at(1, 0);
    d.restore_progress(1, glam::DVec3::new(-3500.0, 0.0, 0.0));
    assert_eq!(d.left_late, Some(0.0), "past its first stop the bus is on its way");
    assert!(
        !d.take_trip_change(),
        "resume must not request automatic reprogramming"
    );
    let mut v = timetable_test_vehicle();
    // Demonstrate that this device really loses its programming without callbacks.
    let vars = vec![("duty".into(), 65104.0)];
    let strings = vec![("destination".into(), "  Manual destination  ".into())];
    v.restore_script_state(&vars, &strings);
    v.update(0.02);
    assert_eq!(v.var("duty"), Some(0.0));
    v.restore_script_state(&vars, &strings);
    v.position = glam::DVec3::new(-3500.0, 0.0, 0.0);
    d.restore_host(&mut v, 542.0);
    v.update(0.02);
    assert_eq!(v.var("duty"), Some(65104.0));
    let dest = v.ty.program.str_var("destination").unwrap() as usize;
    assert_eq!(v.state.str_vars[dest], "  Manual destination  ");
    assert_eq!(v.var("observed_stop"), Some(1.0));
    assert_eq!(v.var("observed_delay"), Some(42.0));
    d.update(&mut v, 542.0);
    assert_eq!((d.trip_index, d.next_stop), (1, 1));
    assert_eq!(d.tour, "65104");
    d.restore_progress(usize::MAX, glam::DVec3::new(-3500.0, 0.0, 0.0));
    assert_eq!(d.next_stop, 2);
    assert!(!d.done, "away from the last stop the trip is not over yet");
    d.restore_progress(2, glam::DVec3::new(1500.0, 0.0, 0.0));
    assert!(d.done, "saved at the last stop, the trip is over");
    d.restore_progress(0, glam::DVec3::new(-3500.0, 0.0, 0.0));
    assert_eq!(d.next_stop, 0);
    assert_eq!(d.left_late, None);
    assert!(!d.done);
    // Ordinary duty updates retain their existing handling of schedule_active.
    v.host.schedule_active = 0.0;
    v.set_var("schedule_active", 0.0);
    d.update(&mut v, 542.0);
    assert_eq!(v.host.schedule_active, 0.0);
    assert_eq!(v.var("schedule_active"), Some(0.0));
}

#[test]
fn terminus_index_is_the_depot_terminus_of_that_name() {
    let mut hof = omsi_vehicle::hof::Hof::default();
    for name in ["A", "B", "C"] {
        hof.termini.push(omsi_vehicle::hof::Terminus { texture_id: name.into(), ..Default::default() });
    }
    assert_eq!(tt_terminus_index(Some(&hof), "B"), 1);
    assert_eq!(tt_terminus_index(Some(&hof), "b"), -1);
    assert_eq!(tt_terminus_index(None, "B"), -1);
}

#[test]
fn a_stop_placed_only_once_its_tile_loads_is_reached() {
    // stop 1's object was in no tile loaded when the duty began
    let mut t = planned(0.0, &[(0.0, 0.0, 0.0), (500.0, 100.0, 100.0), (1000.0, 200.0, 200.0)]);
    t.stops[1].position = None;
    let mut d = PlayerDuty {
        line: "5".into(),
        tour: "1".into(),
        trips: vec![t],
        trip_index: 0,
        first_trip: 0,
        next_stop: 0,
        at_stop: false,
        arrived_late: None,
        done: false,
        served_terminus: None,
        left_late: None,
        held_back: false,
        placed: true,
        trip_changed: false,
        skipped: None,
        run: 0,
        finished: None,
        reopened: None,
        picked: true,
        first_update: None,
        heading: 90.0,
        position: None,
    };
    d.advance(glam::DVec3::new(0.0, 0.0, 0.0), 0.0);
    d.advance(glam::DVec3::new(100.0, 0.0, 0.0), 10.0);
    assert_eq!(d.next_stop, 1);
    // its tile comes: the map has it now
    let mut positions = HashMap::new();
    positions.insert(1i64, (glam::DVec3::new(500.0, 0.0, 0.0), [0.0; 3]));
    d.learn_loaded(&positions);
    assert_eq!(d.trips[0].stops[1].position, Some(glam::DVec3::new(500.0, 0.0, 0.0)));
    d.advance(glam::DVec3::new(500.0, 0.0, 0.0), 100.0);
    d.advance(glam::DVec3::new(700.0, 0.0, 0.0), 130.0);
    assert_eq!(d.next_stop, 2);
}

#[test]
fn the_next_trip_starts_at_its_first_stop_though_the_last_one_was_missed() {
    // trip 1 ends at x = 1000 (a stop object the bus never comes within 25 m of: it
    // stands at x = 1040, where trip 2 leaves from)
    let t1 = planned(0.0, &[(0.0, 0.0, 0.0), (500.0, 100.0, 100.0), (1000.0, 200.0, 200.0)]);
    let t2 = planned(400.0, &[(1040.0, 400.0, 400.0), (1500.0, 500.0, 500.0)]);
    let mut d = PlayerDuty {
        line: "5".into(),
        tour: "1".into(),
        trips: vec![t1, t2],
        trip_index: 0,
        first_trip: 0,
        next_stop: 0,
        at_stop: false,
        arrived_late: None,
        done: false,
        served_terminus: None,
        left_late: None,
        held_back: false,
        placed: true,
        trip_changed: false,
        skipped: None,
        run: 0,
        finished: None,
        reopened: None,
        picked: true,
        first_update: None,
        heading: 90.0,
        position: None,
    };
    d.advance(glam::DVec3::new(0.0, 0.0, 0.0), 0.0);
    d.advance(glam::DVec3::new(100.0, 0.0, 0.0), 10.0);
    d.advance(glam::DVec3::new(500.0, 0.0, 0.0), 100.0);
    d.advance(glam::DVec3::new(700.0, 0.0, 0.0), 130.0);
    assert_eq!(d.next_stop, 2);
    d.take_trip_change();
    // at trip 2's first stop a minute before it leaves: trip 2, and the IBIS is told
    d.advance(glam::DVec3::new(1040.0, 0.0, 0.0), 300.0);
    assert_eq!(d.trip_index, 0, "not before a minute ahead of the departure");
    d.advance(glam::DVec3::new(1040.0, 0.0, 0.0), 345.0);
    assert_eq!(d.trip_index, 1);
    assert!(d.take_trip_change());
    // a bus still on its way (not at trip 2's first stop) stays on trip 1
    let t1 = planned(0.0, &[(0.0, 0.0, 0.0), (500.0, 100.0, 100.0), (1000.0, 200.0, 200.0)]);
    let t2 = planned(400.0, &[(1040.0, 400.0, 400.0), (1500.0, 500.0, 500.0)]);
    let mut d = PlayerDuty {
        line: "5".into(),
        tour: "1".into(),
        trips: vec![t1, t2],
        trip_index: 0,
        first_trip: 0,
        next_stop: 2,
        at_stop: false,
        arrived_late: None,
        done: false,
        served_terminus: None,
        left_late: Some(0.0),
        held_back: false,
        placed: true,
        trip_changed: false,
        skipped: None,
        run: 0,
        finished: None,
        reopened: None,
        picked: true,
        first_update: None,
        heading: 90.0,
        position: None,
    };
    d.advance(glam::DVec3::new(800.0, 0.0, 0.0), 345.0);
    assert_eq!(d.trip_index, 0);
}

/// #1015: the next stop given up from the menu, and at the trip's last one the trip.
#[test]
fn the_next_stop_can_be_skipped() {
    let trip = planned(0.0, &[(0.0, 0.0, 0.0), (100.0, 60.0, 60.0), (500.0, 120.0, 120.0), (1000.0, 200.0, 200.0)]);
    let next = planned(400.0, &[(1040.0, 400.0, 400.0), (1500.0, 500.0, 500.0)]);
    let mut d = PlayerDuty { line: "5".into(), tour: "1".into(), trips: vec![trip, next], trip_index: 0, first_trip: 0, next_stop: 0, at_stop: false, arrived_late: None, done: false, served_terminus: None, left_late: None, held_back: false, placed: true, trip_changed: false, skipped: None, run: 0, finished: None, reopened: None, picked: true, first_update: None, heading: 90.0, position: None };
    // at the first stop and away from it: the next is s1
    d.advance(glam::DVec3::new(0.0, 0.0, 0.0), 0.0);
    d.advance(glam::DVec3::new(50.0, 0.0, 0.0), 10.0);
    assert_eq!(d.next_stop, 1);
    assert_eq!(d.skip_next().as_deref(), Some("s1"));
    assert_eq!(d.next_stop, 2);
    // passing s1 now serves nothing: s2 is still the one due
    assert_eq!(d.advance(glam::DVec3::new(100.0, 0.0, 0.0), 60.0), None);
    assert_eq!(d.next_stop, 2);
    assert_eq!(d.skip_next().as_deref(), Some("s2"));
    // the last stop skipped: the trip is over, and the tour's next trip follows
    assert!(d.stop_to_skip());
    assert_eq!(d.skip_next().as_deref(), Some("s3"));
    assert!(!d.stop_to_skip());
    assert_eq!(d.skip_next(), None);
    d.advance(glam::DVec3::new(700.0, 0.0, 0.0), 345.0);
    assert_eq!(d.trip_index, 1);
    assert_eq!(d.next_stop, 0);
}

#[test]
fn a_page_can_go_back_to_an_earlier_stop() {
    let trip = planned(
        0.0,
        &[
            (0.0, 0.0, 0.0),
            (100.0, 60.0, 60.0),
            (500.0, 120.0, 120.0),
            (1000.0, 200.0, 200.0),
        ],
    );
    let mut d = PlayerDuty {
        line: "5".into(),
        tour: "1".into(),
        trips: vec![trip],
        trip_index: 0,
        first_trip: 0,
        next_stop: 0,
        at_stop: false,
        arrived_late: None,
        done: false,
        served_terminus: None,
        left_late: None,
        held_back: false,
        placed: true,
        trip_changed: false,
        skipped: None,
        run: 0,
        finished: None,
        reopened: None,
        picked: true,
        first_update: None,
        heading: 90.0,
        position: None,
    };
    assert!(d.skip_to(2));
    assert_eq!(d.next_stop, 2);
    // back one stop: due again
    assert!(d.skip_to(1));
    assert_eq!(d.next_stop, 1);
    // the stop it is already heading for: nothing changes
    assert!(!d.skip_to(1));
    // the bus stands at stop 2: the duty does not jump forward again by itself
    d.advance(glam::DVec3::new(500.0, 0.0, 0.0), 100.0);
    assert_eq!(d.next_stop, 1);
    // from the last stop (done) back reopens the trip, forwards does not
    d.skip_to(3);
    d.advance(glam::DVec3::new(1000.0, 0.0, 0.0), 200.0);
    assert!(d.done);
    assert!(!d.skip_to(3));
    assert!(d.skip_to(2));
    assert!(!d.done);
    assert_eq!(d.next_stop, 2);
}

#[test]
fn a_loop_does_not_jump_to_the_stop_over_the_road() {
    // out along y = 0 to x = 1000, back along y = 12: stop 1 at x = 100 going out, stop 5
    // at x = 100 coming back, 12 m apart (#254)
    let mut trip = planned(0.0, &[(0.0, 0.0, 0.0), (100.0, 60.0, 60.0), (500.0, 120.0, 120.0), (1000.0, 200.0, 200.0), (500.0, 280.0, 280.0), (100.0, 340.0, 340.0), (0.0, 400.0, 400.0)]);
    for (i, s) in trip.stops.iter_mut().enumerate() {
        if i >= 4 {
            s.position.as_mut().unwrap().y = 12.0;
        }
    }
    trip.set_dirs();
    let mut d = PlayerDuty {
        line: "5".into(),
        tour: "1".into(),
        trips: vec![trip],
        trip_index: 0,
        first_trip: 0,
        next_stop: 0,
        at_stop: false,
        arrived_late: None,
        done: false,
        served_terminus: None,
        left_late: None,
        held_back: false,
        placed: true,
        trip_changed: false,
        skipped: None,
        run: 0,
        finished: None,
        reopened: None,
        picked: true,
        first_update: None,
        heading: 90.0,
        position: None,
    };
    // at stop 0, then leaving east
    d.advance(glam::DVec3::new(0.0, 0.0, 0.0), 0.0);
    d.advance(glam::DVec3::new(60.0, 0.0, 0.0), 30.0);
    assert_eq!(d.next_stop, 1);
    // at stop 1 heading east: stop 5 (12 m away, the other way round) is not taken
    d.advance(glam::DVec3::new(100.0, 0.0, 0.0), 60.0);
    d.advance(glam::DVec3::new(140.0, 0.0, 0.0), 70.0);
    assert_eq!(d.next_stop, 2, "the duty goes on to stop 2, not over the road to stop 5");
}

#[test]
fn stops_passed_without_stopping_are_told_once() {
    let mut trip = planned(0.0, &[(0.0, 0.0, 0.0), (100.0, 60.0, 60.0), (500.0, 120.0, 120.0), (1000.0, 200.0, 200.0), (1500.0, 280.0, 280.0)]);
    trip.set_dirs();
    let mut d = PlayerDuty {
        line: "5".into(),
        tour: "1".into(),
        trips: vec![trip],
        trip_index: 0,
        first_trip: 0,
        next_stop: 0,
        at_stop: false,
        arrived_late: None,
        done: false,
        served_terminus: None,
        left_late: None,
        held_back: false,
        placed: true,
        trip_changed: false,
        skipped: None,
        run: 0,
        finished: None,
        reopened: None,
        picked: true,
        first_update: None,
        heading: 90.0,
        position: None,
    };
    d.advance(glam::DVec3::new(0.0, 0.0, 0.0), 0.0);
    d.advance(glam::DVec3::new(60.0, 0.0, 0.0), 30.0);
    assert_eq!(d.next_stop, 1);
    assert_eq!(d.take_skipped(), None, "leaving a stop served skips none");
    // the bus turns up at stop 4 (numbered from 1) heading on: stops 2 and 3 were passed
    d.advance(glam::DVec3::new(1000.0, 0.0, 0.0), 90.0);
    assert_eq!(d.next_stop, 3);
    assert_eq!(d.take_skipped(), Some((2, 2, 4)), "two stops, due at 2, now at 4");
    assert_eq!(d.take_skipped(), None, "told once");
}

#[test]
fn a_duty_starts_with_the_trip_that_fits_the_time() {
    // Spandau line 5, tour "Mo-Fr 3": a depot run 14:44-15:01, then 15:07 and 16:01
    let trips = vec![
        planned(
            53040.0,
            &[(0.0, 53040.0, 53040.0), (500.0, 54060.0, 54060.0)],
        ),
        planned(
            54420.0,
            &[(500.0, 54420.0, 54420.0), (1000.0, 56400.0, 56400.0)],
        ),
        planned(
            57660.0,
            &[(1000.0, 57660.0, 57660.0), (500.0, 59400.0, 59400.0)],
        ),
    ];
    assert_eq!(
        starting_trip(&trips, 15.0 * 3600.0 + 300.0),
        1,
        "15:05: the 15:07"
    );
    assert_eq!(
        starting_trip(&trips, 14.0 * 3600.0 + 50.0 * 60.0),
        0,
        "14:50: the depot run under way"
    );
    assert_eq!(
        starting_trip(&trips, 15.0 * 3600.0 + 1800.0),
        1,
        "15:30: the 15:07 under way"
    );
    assert_eq!(
        starting_trip(&trips, 23.0 * 3600.0),
        2,
        "after the last: the last"
    );
    let now = 15.0 * 3600.0 + 300.0;
    let mut d = PlayerDuty {
        line: "5".into(),
        tour: "3".into(),
        trips,
        trip_index: 1,
        first_trip: 0,
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
        run: 0,
        finished: None,
        reopened: None,
        picked: false,
        first_update: None,
        heading: 0.0,
        position: None,
    };
    // 200 m from the first stop two minutes before the departure: early, next stop the first
    assert_eq!(d.advance(glam::DVec3::new(300.0, 0.0, 0.0), now), None);
    assert_eq!(d.next_stop, 0);
    assert!((d.delay(now) + 120.0).abs() < 1e-9);
    // at the first stop, leaving a minute late
    d.advance(glam::DVec3::new(500.0, 0.0, 0.0), now + 60.0);
    assert!(d.at_stop);
    assert_eq!(
        d.advance(glam::DVec3::new(560.0, 0.0, 0.0), 54480.0).map(|(_, left)| left),
        Some(60.0)
    );
    assert_eq!(d.next_stop, 1);
    // on the way the delay is what it left with until the next stop is overdue
    assert!((d.delay(55000.0) - 60.0).abs() < 1e-9);
    assert!((d.delay(56600.0) - 200.0).abs() < 1e-9);
    // the next trip does not take over while this one is driven ...
    d.advance(glam::DVec3::new(800.0, 0.0, 0.0), 57620.0);
    assert_eq!(d.trip_index, 1);
    // ... but once its end is reached
    d.advance(glam::DVec3::new(1000.0, 0.0, 0.0), 57630.0);
    assert!(d.done && !d.take_trip_change());
    d.advance(glam::DVec3::new(1000.0, 0.0, 0.0), 57640.0);
    assert_eq!((d.trip_index, d.next_stop), (2, 0));
    assert!(d.take_trip_change());
}

#[test]
fn a_duty_starts_with_a_trip_the_bus_can_reach() {
    let trips = vec![
        planned(54420.0, &[(500.0, 54420.0, 54420.0), (1000.0, 56400.0, 56400.0)]),
        planned(57660.0, &[(1000.0, 57660.0, 57660.0), (500.0, 59400.0, 59400.0)]),
    ];
    let duty = |trips: Vec<PlannedTrip>| PlayerDuty {
        line: "5".into(),
        tour: "3".into(),
        trips,
        trip_index: 0,
        first_trip: 0,
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
        run: 0,
        finished: None,
        reopened: None,
        picked: false,
        first_update: None,
        heading: 0.0,
        position: None,
    };
    // 4 km away two minutes before the 15:07 leaves: the duty begins with the 16:01
    let mut d = duty(trips.clone());
    d.advance(glam::DVec3::new(-3500.0, 0.0, 0.0), 54300.0);
    assert_eq!((d.trip_index, d.next_stop), (1, 0));
    assert!(d.delay(54300.0) < -3000.0, "early for the 16:01");
    // under way already and nothing later: the last trip, from its first stop the bus
    // can still make on time
    let mut d = duty(trips[1..].to_vec());
    d.advance(glam::DVec3::new(-3500.0, 0.0, 0.0), 58000.0);
    assert_eq!((d.trip_index, d.next_stop), (0, 1));
    // ... or from its first when none can be
    let mut d = duty(trips[1..].to_vec());
    d.advance(glam::DVec3::new(-3500.0, 0.0, 0.0), 59000.0);
    assert_eq!((d.trip_index, d.next_stop), (0, 0));
}

#[test]
fn ibis_skips_a_service_leg_for_the_player_display() {
    let mut service = planned(100.0, &[(0.0, 100.0, 100.0), (100.0, 200.0, 200.0)]);
    service.line.clear();
    service.terminus = "Betriebsfahrt".into();
    let mut passenger = planned(300.0, &[(100.0, 300.0, 300.0), (200.0, 400.0, 400.0)]);
    passenger.line = "5E".into();
    let d = PlayerDuty {
        line: "5E".into(),
        tour: "1".into(),
        trips: vec![service, passenger],
        trip_index: 0,
        first_trip: 0,
        next_stop: 1,
        at_stop: false,
        arrived_late: None,
        done: false,
        served_terminus: None,
        left_late: None,
        held_back: false,
        placed: false,
        trip_changed: false,
        skipped: None,
        run: 0,
        finished: None,
        reopened: None,
        picked: false,
        first_update: None,
        heading: 0.0,
        position: None,
    };
    let (trip, stop) = d.trip_for_ibis();
    assert_eq!(trip.line, "5E");
    assert_eq!(trip.terminus, "T");
    assert_eq!(stop, 0);
}

#[test]
fn the_player_picks_the_trip_to_start_with() {
    let trips = vec![
        planned(4.0 * 3600.0 + 7.0 * 60.0, &[(0.0, 0.0, 0.0)]),
        planned(4.0 * 3600.0 + 22.0 * 60.0, &[(0.0, 0.0, 0.0)]),
        planned(4.0 * 3600.0 + 37.0 * 60.0, &[(0.0, 0.0, 0.0)]),
    ];
    assert_eq!(chosen_trip(&trips, "04:22"), Some(1));
    assert_eq!(chosen_trip(&trips, "4:30"), Some(2), "the next one leaving");
    assert_eq!(chosen_trip(&trips, "05:00"), None);
    assert_eq!(chosen_trip(&trips, "1"), Some(0));
    assert_eq!(chosen_trip(&trips, "3"), Some(2));
    assert_eq!(chosen_trip(&trips, "4"), None);
}

/// A trip ends once for the plugins' `trip_done`: when the bus reaches its last stop, or
/// the last stop is skipped; not again while the bus stands there.
#[test]
fn a_trip_ends_once_by_arriving_or_skipping_its_last_stop() {
    let ended = |d: &mut PlayerDuty| d.take_finished().map(|f| (f.index, f.how));
    let mut d = early_departure_duty();
    let run = d.trip_run();
    d.advance(glam::DVec3::new(400.0, 0.0, 0.0), 150.0);
    assert_eq!(ended(&mut d), None);
    d.advance(glam::DVec3::new(500.0, 0.0, 0.0), 200.0);
    assert!(d.trip_done());
    assert_eq!(d.take_finished(), Some(Finished { index: 0, how: TripEnd::Arrived, run }));
    d.advance(glam::DVec3::new(500.0, 0.0, 0.0), 201.0);
    assert_eq!(ended(&mut d), None);

    let mut d = early_departure_duty();
    d.advance(glam::DVec3::new(400.0, 0.0, 0.0), 150.0);
    assert_eq!(d.skip_next().as_deref(), Some("s1"));
    assert_eq!(ended(&mut d), Some((0, TripEnd::Skipped)));
}

/// A saved situation continued at the trip's last stop: the trip is over, but it did not
/// end now (no `trip_done` for it).
#[test]
fn a_trip_restored_at_its_last_stop_has_not_just_ended() {
    let mut d = early_departure_duty();
    d.restore_progress(1, glam::DVec3::new(500.0, 0.0, 0.0));
    assert!(d.trip_done());
    assert_eq!(d.take_finished(), None);
}

/// A page going back reopens an ended trip: an ending not yet taken is dropped, one taken
/// is undone, and the trip ends again; the next trip is another run.
#[test]
fn a_reopened_trip_ends_again_and_the_next_trip_is_another_run() {
    let mut d = early_departure_duty();
    let run = d.trip_run();
    d.advance(glam::DVec3::new(500.0, 0.0, 0.0), 200.0);
    assert!(d.skip_to(0));
    assert_eq!(d.take_finished(), None, "not yet taken: dropped");
    assert_eq!(d.take_reopened(), None, "nothing to undo");
    assert_eq!(d.trip_run(), run, "the same trip goes on");
    assert!(d.skip_to(1));
    d.advance(glam::DVec3::new(500.0, 0.0, 0.0), 210.0);
    assert_eq!(d.take_finished().map(|f| f.how), Some(TripEnd::Arrived));
    // taken, then reopened: undone, and it ends again
    assert!(d.skip_to(0));
    assert_eq!(d.take_reopened(), Some(run));
    assert!(d.skip_to(1));
    d.advance(glam::DVec3::new(500.0, 0.0, 0.0), 220.0);
    assert_eq!(d.take_finished().map(|f| (f.how, f.run)), Some((TripEnd::Arrived, run)));
    // on to the next trip a minute before it leaves: another run
    d.advance(glam::DVec3::new(500.0, 0.0, 0.0), 545.0);
    assert_eq!(d.trip_index, 1);
    assert_ne!(d.trip_run(), run);
    assert_eq!(d.take_finished(), None);
}

/// Two trips: the bus on the first one's last leg, and the second one's first stop away
/// from the first one's terminus (500 m against 800 m).
fn last_leg_duty(left_late: Option<f64>) -> PlayerDuty {
    PlayerDuty {
        trips: vec![
            planned(0.0, &[(0.0, 0.0, 0.0), (500.0, 200.0, 200.0)]),
            planned(400.0, &[(800.0, 400.0, 400.0), (1500.0, 500.0, 500.0)]),
        ],
        next_stop: 1,
        left_late,
        run: 7,
        ..early_departure_duty()
    }
}

/// A trip whose terminus stop the bus never comes near: standing at the next trip's first
/// stop from its last leg ends it as arrived - once begun; one never begun did not end.
#[test]
fn standing_at_the_next_trips_first_stop_ends_a_begun_trip() {
    let mut d = last_leg_duty(Some(0.0));
    d.advance(glam::DVec3::new(800.0, 0.0, 0.0), 345.0);
    assert_eq!(d.trip_index, 1);
    assert_eq!(d.take_finished(), Some(Finished { index: 0, how: TripEnd::Arrived, run: 7 }));

    let mut d = last_leg_duty(None);
    d.advance(glam::DVec3::new(800.0, 0.0, 0.0), 345.0);
    assert_eq!(d.trip_index, 1);
    assert_eq!(d.take_finished(), None);
}

/// A picked trip given up half an hour past its end: given up once begun; one the bus never
/// left a stop of did not end.
#[test]
fn only_a_begun_trip_is_given_up() {
    let mut d = last_leg_duty(Some(0.0));
    d.advance(glam::DVec3::new(3000.0, 0.0, 0.0), 2100.0);
    assert_eq!(d.take_finished().map(|f| (f.index, f.how)), Some((0, TripEnd::GivenUp)));

    let mut d = last_leg_duty(None);
    d.advance(glam::DVec3::new(3000.0, 0.0, 0.0), 2100.0);
    assert_eq!(d.take_finished(), None);
}

/// On the way between two stops the delay follows where the bus is, as OMSI's IBIS shows
/// it, instead of standing at the delay it left the last stop with (#1898, #735).
#[test]
fn the_delay_on_the_way_follows_where_the_bus_is() {
    let mut d = early_departure_duty();
    // left stop 0 (x 0, dep 100) on time; stop 1 is at x 500, due at 200
    d.left_late = Some(0.0);
    d.position = Some(glam::DVec3::new(250.0, 0.0, 0.0));
    // half way at 150: on time; half way at 170: 20 s late; at 120 (early): -30 s
    assert!((d.delay(150.0) - 0.0).abs() < 1e-6);
    assert!((d.delay(170.0) - 20.0).abs() < 1e-6);
    assert!((d.delay(120.0) + 30.0).abs() < 1e-6);
}
