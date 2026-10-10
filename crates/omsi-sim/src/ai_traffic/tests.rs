use super::*;
use super::planning::lane_open_to;

#[cfg(test)]
mod parked_lane_tests {
    use super::*;
    use super::control::impact_zones;

    #[test]
    fn vehicle_impact_zones_cover_the_whole_body_with_distinct_front_rear_and_sides() {
        let bb = [2.0, 4.0, 2.0, 0.3, 0.7, 1.0];
        let zones = impact_zones(bb, DVec3::ZERO, 0.0, DVec2::ZERO, 1200.0, 42);
        let samples = [
            DVec2::new(0.3, 2.55),
            DVec2::new(0.3, -1.15),
            DVec2::new(-0.45, 0.7),
            DVec2::new(1.05, 0.7),
        ];
        for point in samples {
            let probe = crate::collision::Obb::point(
                DVec3::new(point.x, point.y, 1.0),
                0.02,
            );
            assert!(zones.iter().any(|zone| zone.overlaps(&probe)), "{point:?}");
        }
        assert!(zones.iter().all(|zone| zone.id == -44));
        assert!(zones[0].center.y > zones[1].center.y);
        assert!(zones[2].center.x < zones[3].center.x);
    }

    #[test]
    fn a_parked_body_blocks_an_otherwise_empty_target_lane() {
        assert!(parked_lane_clear(&[], 50.0, 30.0, 70.0, 0.9));
        assert!(!parked_lane_clear(&[(55.0, 0.0)], 50.0, 30.0, 70.0, 0.9));
        // The centre is outside the inspected strip, but the parked car's body is not.
        assert!(!parked_lane_clear(&[(121.0, 0.0)], 50.0, 30.0, 70.0, 0.9));
        assert!(!parked_lane_clear(&[(19.0, 0.0)], 50.0, 30.0, 70.0, 0.9));
        assert!(parked_lane_clear(
            &[(123.0, 0.0), (17.0, 0.0)],
            50.0,
            30.0,
            70.0,
            0.9
        ));
    }

    #[test]
    fn parked_clearance_accounts_for_vehicle_width_and_both_sides() {
        for lateral in [-2.1, 2.1] {
            // A car fits; a wider bus would overlap the parked body.
            assert!(parked_lane_clear(&[(50.0, lateral)], 50.0, 4.0, 10.0, 0.9));
            assert!(!parked_lane_clear(
                &[(50.0, lateral)],
                50.0,
                7.0,
                14.0,
                1.25
            ));
        }
        assert!(parked_lane_clear(&[(50.0, 2.7)], 50.0, 7.0, 14.0, 1.25));
    }
}

#[cfg(test)]
mod mirror_light_tests {
    use super::{mirror_light_tick, reset_light_runtime, set_mirror_light_clock};
    use crate::traffic::TrafficLightController;

    fn program() -> TrafficLightController {
        TrafficLightController::from_program(
            vec![(vec![(0, 4.0), (6, 4.0)], Some(25.0))],
            Some(8.0),
            &[[0.0, 4.0, 1.0]],
            &[[0.0, 6.0, 1.0, 1.0]],
        )
    }

    #[test]
    fn host_hold_survives_missing_local_requests() {
        let mut ctl = program();
        ctl.stops[0].if_request = false;
        ctl.start(4.0);
        set_mirror_light_clock(&mut ctl, 4.0, true);
        for _ in 0..60 {
            mirror_light_tick(&mut ctl, 0.1, 20_000.0);
        }
        assert_eq!(ctl.time, 4.0);
        assert!(ctl.held);
        assert_eq!(ctl.state(0), 6);
    }

    #[test]
    fn unheld_host_clock_crosses_local_stop_and_jump_points() {
        let mut ctl = program();
        ctl.start(3.5);
        set_mirror_light_clock(&mut ctl, 3.5, false);
        mirror_light_tick(&mut ctl, 1.0, 20_000.0);
        assert_eq!(ctl.time, 4.5);
        assert_eq!(ctl.state(0), 6);
        mirror_light_tick(&mut ctl, 2.0, 20_000.0);
        assert_eq!(ctl.time, 6.5);
        assert!(!ctl.held);
    }

    #[test]
    fn first_snapshot_is_not_replaced_by_the_day_time_seed() {
        let mut ctl = program();
        ctl.offset = 0.75;
        set_mirror_light_clock(&mut ctl, 6.5, false);
        mirror_light_tick(&mut ctl, 0.0, 20_000.0);
        assert_eq!(ctl.time, 6.5);
        assert_eq!(ctl.state(0), 6);
    }

    #[test]
    fn a_new_crossing_uses_the_day_clock_until_its_first_snapshot() {
        let mut ctl = program();
        ctl.offset = 0.75;
        mirror_light_tick(&mut ctl, 0.0, 10.0);
        assert_eq!(ctl.time, 2.75);
        set_mirror_light_clock(&mut ctl, 6.5, true);
        mirror_light_tick(&mut ctl, 1.0, 10.0);
        assert_eq!(ctl.time, 6.5);
        assert!(ctl.held);
    }

    #[test]
    fn host_seek_release_and_cycle_wrap_replace_interpolation() {
        let mut ctl = program();
        set_mirror_light_clock(&mut ctl, 7.75, false);
        mirror_light_tick(&mut ctl, 0.5, 0.0);
        assert_eq!(ctl.time, 0.25);
        assert_eq!(ctl.state(0), 0);
        set_mirror_light_clock(&mut ctl, 10.5, true);
        mirror_light_tick(&mut ctl, 2.0, 0.0);
        assert_eq!(ctl.time, 2.5);
        set_mirror_light_clock(&mut ctl, 1.5, false);
        mirror_light_tick(&mut ctl, 1.0, 0.0);
        assert_eq!(ctl.time, 2.5);
        assert!(!ctl.held);
        mirror_light_tick(&mut ctl, -1.0, 0.0);
        assert_eq!(ctl.time, 2.5);
    }

    #[test]
    fn returning_to_local_simulation_discards_a_previously_passed_stop() {
        let mut ctl = program();
        ctl.offset = 0.75;
        ctl.start(3.25);
        ctl.request[0] = true;
        ctl.advance(0.0);
        assert!(!ctl.held);
        set_mirror_light_clock(&mut ctl, 4.0, false);
        reset_light_runtime(&mut ctl, 20_000.0);
        assert_eq!(ctl.lights, vec![vec![(0, 4.0), (6, 4.0)]]);
        assert_eq!(ctl.approach, vec![Some(25.0)]);
        assert_eq!(ctl.offset, 0.75);
        assert_eq!(ctl.stops.len(), 2);
        assert_eq!(ctl.request, vec![false]);
        ctl.advance(0.0);
        assert!(ctl.held);
        assert_eq!(ctl.time, 4.0);
    }

    #[test]
    fn returning_to_local_simulation_discards_a_previous_backward_jump() {
        let mut ctl = program();
        ctl.start(5.5);
        ctl.advance(0.5);
        assert_eq!(ctl.time, 1.0);
        set_mirror_light_clock(&mut ctl, 6.0, false);
        reset_light_runtime(&mut ctl, 20_000.0);
        ctl.advance(0.0);
        assert_eq!(ctl.time, 1.0);
        assert!(!ctl.held);
    }

    #[test]
    fn returning_to_local_simulation_rechecks_a_host_hold() {
        let mut ctl = program();
        ctl.stops[0].if_request = false;
        set_mirror_light_clock(&mut ctl, 4.0, true);
        reset_light_runtime(&mut ctl, 20_000.0);
        assert!(!ctl.held);
        ctl.advance(0.5);
        assert!(!ctl.held);
        assert_eq!(ctl.time, 4.5);
    }

    #[test]
    fn a_new_crossing_keeps_its_first_seed_when_returning_to_local_simulation() {
        let mut ctl = program();
        ctl.offset = 0.75;
        reset_light_runtime(&mut ctl, 10.0);
        assert_eq!(ctl.time, 2.75);
        ctl.start(20_000.0);
        ctl.advance(0.5);
        assert_eq!(ctl.time, 3.25);
    }
}

#[cfg(test)]
mod road_scale_tests {
    use crate::ai_traffic::density::road_scale;

    #[test]
    fn path_density_scales_the_street_target() {
        assert!((road_scale(&[1.0; 100]) - 1.0).abs() < 1e-6);
        assert!((road_scale(&[0.2; 100]) - 0.2).abs() < 1e-6);
        assert_eq!(road_scale(&[0.0; 100]), 0.0);
        assert!((road_scale(&[1.0; 500]) - 2.0).abs() < 1e-6);
    }
}

#[cfg(test)]
mod junction_arrival_tests {
    use super::{crossing_arrival, queued_exit_vehicle};
    use crate::traffic::AiState;

    #[test]
    fn stopped_queue_does_not_predict_a_restart() {
        let st = AiState::new(0, 0.0, 1);
        assert_eq!(crossing_arrival(&st, 10.0, false, false, true), f32::MAX);
    }

    #[test]
    fn crawling_queue_is_measured_at_its_actual_speed() {
        let mut st = AiState::new(0, 0.0, 1);
        for speed in [0.2, 0.3, 0.4, 0.5, 0.8, 1.4] {
            st.speed = speed;
            assert!((crossing_arrival(&st, 10.0, false, false, true) - 10.0 / speed).abs() < 1e-5);
        }
    }

    #[test]
    fn crawling_queue_allows_a_gap_but_nearby_traffic_still_counts() {
        let mut st = AiState::new(0, 0.0, 1);
        st.speed = 0.4;
        let gap = 7.5;
        assert!(crossing_arrival(&st, 10.0, false, false, true) > gap);
        assert!(crossing_arrival(&st, 1.0, false, false, true) < gap);
        // Once it can move freely again, account for it accelerating towards the crossing.
        assert!(crossing_arrival(&st, 10.0, true, false, false) < gap);
    }

    #[test]
    fn queue_already_at_the_conflict_still_blocks() {
        let st = AiState::new(0, 0.0, 1);
        for distance in [-1.0, 0.0, 0.3] {
            assert_eq!(crossing_arrival(&st, distance, false, true, true), 0.0);
        }
    }

    #[test]
    fn freely_starting_car_keeps_its_accelerating_prediction() {
        let st = AiState::new(0, 0.0, 1);
        let expected = (20.0 / st.accel).sqrt() + st.reaction;
        assert!((crossing_arrival(&st, 10.0, false, false, false) - expected).abs() < 1e-5);
        assert!((crossing_arrival(&st, 10.0, true, false, false) - expected).abs() < 1e-5);
    }

    #[test]
    fn car_waiting_before_the_conflict_is_not_approaching() {
        let mut st = AiState::new(0, 0.0, 1);
        st.speed = 2.0;
        assert_eq!(crossing_arrival(&st, 10.0, false, true, false), f32::MAX);
        assert!(crossing_arrival(&st, 10.0, true, false, false) < 5.0);
        assert_eq!(crossing_arrival(&st, 10.0, false, false, false), 5.0);
    }

    #[test]
    fn a_queue_on_a_short_lane_after_the_exit_keeps_the_junction_clear() {
        // Lanes 20 and 21 are two short pieces immediately beyond the crossing. The first
        // is empty; a car whose rear is 1 m into the second leaves only 3 m beyond the exit.
        let way = [(10, -20.0), (20, 5.0), (21, 7.0), (22, 10.0), (23, 40.0)];
        assert_eq!(
            queued_exit_vehicle(&way, (20, 5.0), 8.0, [(21, 1.0, 0.0)]),
            Some((3.0, 0.0, 21))
        );
    }

    #[test]
    fn traffic_beyond_the_space_the_car_needs_does_not_close_the_exit() {
        let way = [(10, -20.0), (20, 5.0), (21, 7.0), (22, 10.0), (23, 40.0)];
        assert_eq!(
            queued_exit_vehicle(&way, (20, 5.0), 8.0, [(22, 5.0, 0.0), (23, 0.0, 0.0)]),
            None
        );
    }

    #[test]
    fn the_nearest_vehicle_across_exit_pieces_wins() {
        let way = [(10, -20.0), (20, 5.0), (21, 7.0), (22, 10.0)];
        assert_eq!(
            queued_exit_vehicle(
                &way,
                (20, 5.0),
                10.0,
                [(21, 4.0, 1.0), (20, 5.0, 0.5), (22, -1.0, 0.0)]
            ),
            Some((4.0, 0.0, 22))
        );
    }
}

#[cfg(test)]
mod group_density_tests {
    use super::player_reach_ahead;
    use crate::traffic::pool_density as uvg_density;
    use glam::DVec2;

    /// Berlin-Spandau's `unsched_vehgroups.txt`: NormalCars 1, Trucks 0, Commercials 1,
    /// Ambulance 1, GDRCars 0.
    const SPANDAU: [i32; 5] = [1, 0, 1, 1, 0];

    #[test]
    fn a_bus_under_a_bridge_is_not_in_the_way_on_it() {
        use super::in_player_box;
        use glam::DVec3;
        let (c, f, r) = (DVec3::new(0.0, 0.0, 32.0), DVec2::new(0.0, 1.0), DVec2::new(1.0, 0.0));
        // the road through the bus's box, on its level and on a bridge 5.4 m above it
        assert!(in_player_box(DVec3::new(0.5, 3.0, 32.3), c, f, r, 2.5, 6.0, 6.0));
        assert!(!in_player_box(DVec3::new(0.5, 3.0, 37.4), c, f, r, 2.5, 6.0, 6.0));
        assert!(!in_player_box(DVec3::new(0.5, 3.0, 26.0), c, f, r, 2.5, 6.0, 6.0));
        // beside it
        assert!(!in_player_box(DVec3::new(3.5, 3.0, 32.0), c, f, r, 2.5, 6.0, 6.0));
    }

    #[test]
    fn a_following_bus_is_no_bus_in_the_way() {
        let north = DVec2::new(0.0, 1.0);
        // a car ahead going the same way: only the bus itself counts
        assert_eq!(player_reach_ahead(6.0, 14.0, 1.5, north, DVec2::new(0.1, 3.0)), 6.0);
        // a car crossing its way (or coming towards it): where the bus will be counts too
        assert_eq!(player_reach_ahead(6.0, 14.0, 1.5, north, DVec2::new(3.0, 0.0)), 27.0);
        assert_eq!(player_reach_ahead(6.0, 14.0, 1.5, north, DVec2::new(0.0, -3.0)), 27.0);
    }

    #[test]
    fn a_group_off_by_default_drives_where_a_path_asks_for_it() {
        // a Falkensee path: no rule for the normal cars, the GDR cars asked for
        let rules = [(4u16, 1.0f32)];
        assert_eq!(uvg_density(&rules, &SPANDAU, 4), 1.0);
        assert_eq!(uvg_density(&rules, &SPANDAU, 0), 1.0);
        // and nowhere else
        assert_eq!(uvg_density(&[], &SPANDAU, 4), 0.0);
        assert_eq!(uvg_density(&[(0, 0.5)], &SPANDAU, 1), 0.0);
    }

    #[test]
    fn a_default_follows_the_first_group_on_the_path() {
        // commercials (default 1) take the normal cars' density of the path
        assert_eq!(uvg_density(&[(0, 0.4)], &SPANDAU, 2), 0.4);
        assert_eq!(uvg_density(&[(0, 0.0)], &SPANDAU, 2), 0.0);
        assert_eq!(uvg_density(&[], &SPANDAU, 2), 1.0);
        // an own rule wins
        assert_eq!(uvg_density(&[(0, 0.4), (2, 2.0)], &SPANDAU, 2), 2.0);
    }

    #[test]
    fn defaults_naming_each_other_end() {
        assert_eq!(uvg_density(&[], &[1, 3, 2], 1), 0.0);
    }
}


#[cfg(test)]
mod way_user_tests {
    use super::*;
    use crate::traffic::{Crossing, LaneBuilder};

    fn street(start: DVec3, heading: f64, length: f64, radius: f64) -> crate::traffic::Lane {
        LaneBuilder::arc(start, heading, length, radius, 0.0, LaneKind::Street, 3.0)
    }

    /// A road north (lane 0) forking into straight on (1) and a left turn (2).
    fn fork() -> Network {
        let a = street(DVec3::ZERO, 0.0, 50.0, 0.0);
        let b = street(a.end(), 0.0, 20.0, 0.0);
        let mut c = street(a.end(), 0.0, 15.7, -10.0);
        c.turn = 1;
        let mut net = Network { lanes: vec![a, b, c], ..Default::default() };
        net.link(1.5);
        net.build_grid();
        net
    }

    #[test]
    fn the_players_bus_is_put_onto_the_lanes_it_may_take() {
        let net = fork();
        let bus: PlayerBox = (DVec3::new(0.0, 20.0, 0.0), 0.0, 6.0, 1.25, 10.0);
        let u = way_user_on(&net, &bus, 0, 0.0, false).expect("on the road");
        let lanes: Vec<usize> = u.lanes.iter().map(|l| l.0).collect();
        assert_eq!(lanes[0], 0);
        assert!((u.lanes[0].1 + 20.0).abs() < 0.1, "{:?}", u.lanes);
        // no indicator: either way (the cars cannot know)
        assert!(lanes.contains(&1) && lanes.contains(&2), "{lanes:?}");
        assert!(u.lanes.iter().filter(|l| l.0 != 0).all(|l| (l.1 - 30.0).abs() < 0.1), "{:?}", u.lanes);
        // indicating left: the left turn only; right (no such branch): both
        let left: Vec<usize> = way_user_on(&net, &bus, 1, 0.0, false).unwrap().lanes.iter().map(|l| l.0).collect();
        assert_eq!(left, vec![0, 2]);
        assert_eq!(way_user_on(&net, &bus, 2, 0.0, false).unwrap().lanes.len(), 3);
        // reversing, or off the road: not on the lanes
        assert!(way_user_on(&net, &(bus.0, 0.0, 6.0, 1.25, -2.0), 0, 0.0, false).is_none());
        assert!(way_user_on(&net, &(DVec3::new(30.0, 20.0, 0.0), 0.0, 6.0, 1.25, 5.0), 0, 0.0, false).is_none());
    }

    fn user(lanes: Vec<(usize, f32)>, speed: f32, still: f32) -> WayUser {
        WayUser { lanes, speed, half_len: 6.0, still, prio: false }
    }

    /// A side road's car 15 m before the meeting place with the main road the bus drives on.
    fn side_road_car(speed: f32) -> AiState {
        let mut st = AiState::new(0, 0.0, 1);
        st.speed = speed;
        st.accept_gap = 5.0;
        st
    }

    #[test]
    fn a_car_on_the_side_road_gives_way_to_the_players_bus() {
        let c = Crossing { other: 1, at: 2.0, other_at: 2.0, merge: false, before: 1.5, after: 1.5, other_before: 1.5, other_after: 1.5 };
        let st = side_road_car(5.0);
        let point = 17.5; // the car's origin to the meeting point
        // the bus 50 m off on the main road at 11 m/s (there in about four seconds)
        let coming = user(vec![(7, -10.0), (1, 40.0)], 11.0, 0.0);
        assert_eq!(way_user_verdict(&st, &coming, 40.0, &c, point, false, false, false, true, 1.0).0, Verdict::Ruled);
        // the car has the right of way, or has claimed the junction already: it goes
        assert_eq!(way_user_verdict(&st, &coming, 40.0, &c, point, false, false, false, false, 1.0).0, Verdict::Free);
        assert_eq!(way_user_verdict(&st, &coming, 40.0, &c, point, false, true, false, true, 1.0).0, Verdict::Free);
        // far off (a gap any driver takes), or standing at a stop: it goes
        let far = user(vec![(1, 140.0)], 11.0, 0.0);
        assert_eq!(way_user_verdict(&st, &far, 140.0, &c, point, false, false, false, true, 1.0).0, Verdict::Free);
        let standing = user(vec![(1, 20.0)], 0.0, 30.0);
        assert_eq!(way_user_verdict(&st, &standing, 20.0, &c, point, false, false, false, true, 1.0).0, Verdict::Free);
        // in the meeting place: whoever has the right of way, the car waits
        let there = user(vec![(1, -6.0)], 0.0, 30.0);
        assert_eq!(way_user_verdict(&st, &there, -6.0, &c, point, false, false, false, false, 1.0).0, Verdict::Hard);
        // in the junction and on its way through, there first: the car waits
        let crossing = user(vec![(1, -1.0)], 8.0, 0.0);
        assert_eq!(way_user_verdict(&st, &crossing, -1.0, &c, 4.0, false, false, false, false, 1.0).0, Verdict::Hard);
        // through already
        let through = user(vec![(1, -20.0)], 8.0, 0.0);
        assert_eq!(way_user_verdict(&st, &through, -20.0, &c, point, false, false, false, true, 1.0).0, Verdict::Free);
    }

    /// Two lanes running into one (lane 0 from the south, lane 1 slanting in from the
    /// south-east) and on as lane 2.
    fn joint() -> Network {
        let main = LaneBuilder::polyline(vec![DVec3::ZERO, DVec3::new(0.0, 50.0, 0.0)], LaneKind::Street, 3.0);
        let side = LaneBuilder::polyline(vec![DVec3::new(30.0, 20.0, 0.0), DVec3::new(0.0, 50.0, 0.0)], LaneKind::Street, 3.0);
        let on = LaneBuilder::polyline(vec![DVec3::new(0.0, 50.0, 0.0), DVec3::new(0.0, 120.0, 0.0)], LaneKind::Street, 3.0);
        let mut net = Network { lanes: vec![main, side, on], ..Default::default() };
        net.link(1.5);
        net.build_grid();
        net
    }

    #[test]
    fn a_car_merging_in_keeps_behind_the_players_bus() {
        let net = joint();
        let side_len = net.lanes[1].length();
        // the car 20 m before the joint at 8 m/s
        let mut me = AiState::new(1, side_len - 20.0, 1);
        me.speed = 8.0;
        me.planned_next = Some(2);
        // the bus 20 m before the joint at 12 m/s, its way on through it: there first
        let bus = user(vec![(0, -30.0), (2, 20.0)], 12.0, 0.0);
        let l = merging_lead(&net, &me, &[bus]).expect("the bus goes first");
        assert!(l.gap > 0.0 && l.gap < 20.0, "{l:?}");
        // the bus far back, or standing: the car goes
        assert!(merging_lead(&net, &me, &[user(vec![(0, 0.0), (2, 50.0)], 12.0, 0.0)]).is_none());
        assert!(merging_lead(&net, &me, &[user(vec![(0, -30.0), (2, 20.0)], 0.0, 9.0)]).is_none());
        // a bus whose way does not go on into the lane the car takes: nothing to do with it
        assert!(merging_lead(&net, &me, &[user(vec![(0, -30.0)], 12.0, 0.0)]).is_none());
    }
}

#[cfg(test)]
mod signal_entry_tests {
    use super::*;
    use crate::traffic::{LaneBuilder, LaneKey};

    fn lane(id: Option<i64>, light: Option<(usize, usize)>) -> crate::traffic::Lane {
        let mut lane = LaneBuilder::arc(DVec3::ZERO, 0.0, 20.0, 0.0, 0.0, LaneKind::Street, 3.0);
        lane.source = 2;
        lane.key = id.map(|id| LaneKey {
            tile: (0, 0),
            id,
            path: 0,
        });
        lane.traffic_light = light;
        lane
    }

    #[test]
    fn a_signal_does_not_require_scenery_identity() {
        let mut net = Network {
            lanes: vec![lane(None, None), lane(None, Some((0, 1)))],
            ..Default::default()
        };
        let way = [(0, -5.0), (1, 15.0)];
        assert_eq!(entry_light(&net, &way, 1), Some((0, 1)));
        net.lanes[1].source = 1;
        net.lanes[1].key = Some(LaneKey {
            tile: (0, 0),
            id: 10,
            path: 0,
        });
        assert_eq!(entry_light(&net, &way, 1), Some((0, 1)));
    }

    #[test]
    fn junction_interior_deduplication_keeps_the_next_junction_signal() {
        let net = Network {
            lanes: vec![
                lane(Some(10), Some((0, 0))),
                lane(Some(10), Some((0, 1))),
                lane(Some(11), Some((1, 0))),
            ],
            ..Default::default()
        };
        let way = [(0, -5.0), (1, 15.0), (2, 35.0)];
        assert_eq!(entry_light(&net, &way, 0), Some((0, 0)));
        assert_eq!(entry_light(&net, &way, 1), None);
        assert_eq!(entry_light(&net, &way, 2), Some((1, 0)));
    }
}

#[cfg(test)]
mod lane_permission_tests {
    use super::*;
    use crate::traffic::LaneBuilder;

    #[test]
    fn lane_changes_follow_type_and_pool_permissions() {
        let mut lane = LaneBuilder::arc(DVec3::ZERO, 0.0, 20.0, 0.0, 0.0, LaneKind::Street, 3.0);
        let mut state = AiState::new(0, 0.0, 1);
        lane.no_cars = true;
        lane.rule_bus = true;
        assert!(!lane_open_to(&lane, &state));
        state.veh_type = 1;
        assert!(
            lane_open_to(&lane, &state),
            "taxis may use an explicitly open bus lane"
        );
        state.veh_type = 2;
        assert!(lane_open_to(&lane, &state));
        state.traffic_pool = Some((2, vec![1, 1, 1].into()));
        lane.group_density = vec![(2, 0.0)];
        assert!(!lane_open_to(&lane, &state));
        lane.group_density = vec![(2, 0.5)];
        lane.density = 0.0;
        assert!(
            lane_open_to(&lane, &state),
            "the specific pool rule matches route-planning semantics"
        );
        lane.group_density.clear();
        state.traffic_pool = Some((2, vec![1, 1, 0].into()));
        assert!(
            !lane_open_to(&lane, &state),
            "disabled default groups remain closed without a path override"
        );
    }
}

/// The traffic simulation on its own: no world, no renderer, a network made up here and a
/// vehicle type of two small files. `TrafficSim::assemble` and `TrafficSim::place_car` are the
/// parts of `Traffic::new` and `Traffic::create_car` that need neither.
#[cfg(test)]
mod headless_tests {
    use super::*;
    use crate::traffic::LaneBuilder;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use crate::ai_traffic::setup::RandomTypes;

    /// A vehicle type with no model and no scripts, in a folder of its own.
    struct Fixture {
        dir: std::path::PathBuf,
        ty: Arc<VehicleType>,
    }

    impl Fixture {
        fn new() -> Fixture {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "omsi-traffic-headless-{}-{}",
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

    /// A straight street north: `lengths` metres of lanes one after the other, ending in a
    /// dead end.
    fn road(lengths: &[f64]) -> Network {
        let mut lanes: Vec<crate::traffic::Lane> = Vec::new();
        let mut start = DVec3::ZERO;
        for &len in lengths {
            let l = LaneBuilder::arc(start, 0.0, len, 0.0, 0.0, LaneKind::Street, 3.0);
            start = l.end();
            lanes.push(l);
        }
        let mut net = Network { lanes, ..Default::default() };
        net.link(1.5);
        net
    }

    fn traffic(f: &Fixture, net: Network) -> TrafficSim {
        let random = RandomTypes {
            types: Vec::new(),
            groups: Vec::new(),
            group_curves: false,
            group_uvg: Vec::new(),
            uvg_defaults: Vec::new(),
        };
        TrafficSim::assemble(&f.dir, net, random, Vec::new(), HashMap::new(), (Vec::new(), Vec::new()), Vec::new(), (1.0, 0), 0)
    }

    /// A random car of the fixture's type on `lane` at `s` (as `create_car` puts one, without
    /// its picture and the ground under it).
    fn add_car(t: &mut TrafficSim, f: &Fixture, lane: usize, s: f32, seed: u64) -> u64 {
        let vehicle = VehicleInstance::new(f.ty.clone(), crate::VehicleHost::new(crate::SimClock::default()));
        t.place_car(vehicle, LaneKind::Street, lane, s, f.ty.clone(), seed, None, None, None, None)
    }

    fn car(t: &TrafficSim, id: u64) -> &AiCar {
        t.cars.iter().find(|c| c.id == id).unwrap()
    }

    #[test]
    fn cars_drive_along_the_road_one_behind_the_other() {
        let f = Fixture::new();
        let mut t = traffic(&f, road(&[300.0, 300.0, 300.0]));
        let lead = add_car(&mut t, &f, 0, 60.0, 0x1234_5678);
        let follow = add_car(&mut t, &f, 0, 20.0, 0x8765_4321);
        let start = (car(&t, lead).vehicle.position, car(&t, follow).vehicle.position);
        for _ in 0..600 {
            t.tick(0.05, None);
            let (a, b) = (car(&t, lead), car(&t, follow));
            // the follower stays behind its lead, its front clear of the lead's rear
            let gap = (a.vehicle.position.y - b.vehicle.position.y) as f32 - a.state.rear - b.state.front;
            assert!(gap > 0.5, "the cars touch: gap {gap:.2} m at t={:.2}", t.time);
        }
        let (a, b) = (car(&t, lead), car(&t, follow));
        assert!(a.vehicle.position.y - start.0.y > 100.0, "the lead drove {:.1} m", a.vehicle.position.y - start.0.y);
        assert!(b.vehicle.position.y - start.1.y > 100.0, "the follower drove {:.1} m", b.vehicle.position.y - start.1.y);
        assert_eq!(b.lead_car, Some(lead));
        assert!((t.time - 30.0).abs() < 1e-3);
    }

    #[test]
    fn a_car_at_a_dead_end_leaves_the_road() {
        let f = Fixture::new();
        let mut t = traffic(&f, road(&[120.0]));
        add_car(&mut t, &f, 0, 10.0, 42);
        let mut steps = 0;
        while !t.cars.is_empty() && steps < 1200 {
            t.tick(0.05, None);
            steps += 1;
        }
        assert!(t.cars.is_empty(), "the car still stands at s {:.1}", t.cars[0].state.s);
    }

    /// A car coming up the road to a bus standing in its bay beside it, 150 m on: the bus
    /// is the player's or a LAN player's, indicating out of the stop or not. Where the car
    /// is after `secs` (its front, m north), its speed and what held it.
    fn bus_in_its_bay(lan: bool, indicating: bool, secs: f32) -> (f64, f32, &'static str) {
        let f = Fixture::new();
        let mut t = traffic(&f, road(&[400.0]));
        let id = add_car(&mut t, &f, 0, 60.0, 0x5EED);
        // (3.5 m to the right of the lane's middle: out of the car's way)
        let bus: PlayerBox = (DVec3::new(3.5, 150.0, 0.0), 0.0, 6.0, 1.25, 0.0);
        let blinker = if indicating { 1 } else { 0 };
        for _ in 0..(secs / 0.05) as usize {
            if lan {
                t.others = vec![(7, bus)];
                t.other_blinkers = [(7, blinker)].into_iter().collect();
                t.tick(0.05, None);
            } else {
                t.player_blinker = blinker;
                t.tick(0.05, Some(bus));
            }
        }
        let c = car(&t, id);
        (
            c.vehicle.position.y + c.state.front as f64,
            c.state.speed,
            c.why.0,
        )
    }

    #[test]
    fn a_lan_players_bus_indicating_is_let_out_of_its_stop_as_the_players_is() {
        // (on a dedicated server every bus is a LAN player's: the cars let none of them out)
        for lan in [false, true] {
            let (front, speed, why) = bus_in_its_bay(lan, true, 15.0);
            assert!(
                front < 144.0 && speed < 0.1 && why == "let_out",
                "lan {lan}: the car's front at {front:.1} m, {speed:.1} m/s, held by {why:?}"
            );
            // not indicating: the bus stays in its bay, the car drives on past it
            let (front, _, _) = bus_in_its_bay(lan, false, 25.0);
            assert!(front > 160.0, "lan {lan}: the car's front at {front:.1} m");
        }
    }

    #[test]
    fn the_same_start_makes_the_same_traffic() {
        let run = || {
            let f = Fixture::new();
            let mut t = traffic(&f, road(&[200.0, 200.0]));
            for (k, s) in [15.0, 45.0, 80.0].into_iter().enumerate() {
                add_car(&mut t, &f, 0, s, 7 + k as u64 * 0x9E37_79B9);
            }
            for _ in 0..200 {
                t.tick(0.05, None);
            }
            t.cars.iter().map(|c| (c.id, c.state.lane, c.state.s, c.state.speed)).collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }
}
