//! The route helpers: bays, and where a bus is on a partly loaded route.

use super::*;

#[test]
fn off_centre_bounding_boxes_align_the_physical_flank_on_either_side() {
    let mut bus = script_test_vehicle("{frame}\n{end}\n", "", "");
    let ty = std::sync::Arc::get_mut(&mut bus.ty).unwrap();
    ty.def.bounding_box = Some([2.5, 12.0, 3.0, 0.6, 0.0, 1.5]);
    let right = bay_for(4.0, ty, false, false, 0.0);
    assert!((right + 0.6 + 1.25 - 4.3).abs() < 1e-5);
    let left = bay_for(-4.0, ty, false, false, 1.0);
    assert!((left + 0.6 - 1.25 + 4.3).abs() < 1e-5);
    assert_eq!(bay_for(4.0, ty, true, false, 0.0), 0.0);
}

#[test]
fn bay_alignment_follows_the_platform_side_without_moving_rail_vehicles() {
    let bus = script_test_vehicle("{frame}\n{end}\n", "", "");
    let ty = &bus.ty;
    assert!((bay_for(4.0, ty, false, false, 0.0) - 3.05).abs() < 1e-5);
    assert!((bay_for(-4.0, ty, false, false, 1.0) + 3.05).abs() < 1e-5);
    assert!((bay_for(-4.0, ty, false, true, 0.0) + 3.05).abs() < 1e-5);
    assert!((bay_for(4.0, ty, false, true, 1.0) - 3.05).abs() < 1e-5);
    for hand in [false, true] {
        assert!((bay_for(-4.0, ty, false, hand, 2.0) + 3.05).abs() < 1e-5);
        assert!((bay_for(4.0, ty, false, hand, 2.0) - 3.05).abs() < 1e-5);
        for side in [0.0, 1.0, 2.0] {
            assert_eq!(bay_for(-4.0, ty, true, hand, side), 0.0);
            assert_eq!(bay_for(f32::NAN, ty, false, hand, side), 0.0);
        }
    }
}

#[test]
fn distant_or_opposite_side_stop_does_not_pull_bus_across_platform() {
    let bus = script_test_vehicle("{frame}\n{end}\n", "", "");
    let ty = &bus.ty;
    assert_eq!(bay_for(-14.7, ty, false, false, 0.0), 0.0);
    assert_eq!(bay_for(14.7, ty, false, false, 0.0), 0.0);
    assert_eq!(bay_for(-4.0, ty, false, false, 0.0), 0.0);
    assert_eq!(bay_for(4.0, ty, false, false, 1.0), 0.0);
    assert_eq!(bay_for(-4.0, ty, false, false, 2.0), -3.05);
}

#[test]
fn where_a_bus_is_on_a_partly_loaded_route() {
    let key = |id: i64| {
        Some(LaneKey {
            tile: (0, 0),
            id,
            path: 0,
        })
    };
    let legs = [0, 0, 1, 1, 1, 2];
    let steps: Vec<Step> = legs
        .iter()
        .enumerate()
        .map(|(i, &leg)| Step {
            key: key(i as i64),
            leg,
            length: 0.0,
        })
        .collect();
    let slots = [
        Slot::Lane(10),
        Slot::Lane(11),
        Slot::Lane(12),
        Slot::Waiting,
        Slot::Lane(14),
        Slot::Absent,
    ];
    let est = [100.0, 100.0, 50.0, 70.0, 50.0, 0.0];
    // a layover bus stands at the start
    assert_eq!(step_at(&steps, &slots, &est, 0, 0.0), Some((0, 0.0)));
    // 17 m into leg 1: on its first lane, whose part of the route ends at the gap
    let (at, off) = step_at(&steps, &slots, &est, 1, 0.1).unwrap();
    assert!(at == 2 && (off - 17.0).abs() < 1e-9, "{at} {off}");
    assert_eq!(section_around(&slots, 2), (0, 3));
    // half way: on the step still to come - the bus has to wait
    assert_eq!(step_at(&steps, &slots, &est, 1, 0.5), Some((3, 35.0)));
    // near the end of the leg: after the gap
    let (at, off) = step_at(&steps, &slots, &est, 1, 0.9).unwrap();
    assert_eq!(at, 4);
    assert!((off - 33.0).abs() < 1e-9);
    assert_eq!(section_around(&slots, 4), (4, 6));
    // a leg of absent steps only, and a leg without steps: past the end
    assert_eq!(step_at(&steps, &slots, &est, 2, 0.5), None);
    assert_eq!(step_at(&steps, &slots, &est, 3, 0.5), None);
    // a leg without a station link: at the start of the next leg
    let steps2: Vec<Step> = [0, 2, 2]
        .iter()
        .enumerate()
        .map(|(i, &leg)| Step {
            key: key(i as i64),
            leg,
            length: 0.0,
        })
        .collect();
    let slots2 = [Slot::Lane(1), Slot::Absent, Slot::Lane(3)];
    assert_eq!(
        step_at(&steps2, &slots2, &[10.0, 0.0, 10.0], 1, 0.5),
        Some((2, 0.0))
    );
    assert_eq!(section_around(&slots2, 2), (0, 3));
}

#[test]
fn bays() {
    // the stop's box offset is kept as it is until the vehicle is known
    for lat in [0.0, 2.0, -4.0] {
        assert_eq!(bay_offset(lat), lat);
    }
}
