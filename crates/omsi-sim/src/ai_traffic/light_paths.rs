//! Light paths marked as a turn work as detectors for that indicator: a vehicle standing
//! on such a path with the matching indicator on asks its light, even where the path lies
//! over another road and is not on the vehicle's way. Platform screen doors are scenery
//! driven this way: a short path in the bus bay, marked as a right turn, linked to a light
//! whose phase the doors' script reads through `[varparent]` (a bus pulling in with its
//! right indicator opens them). The requests along the way the vehicles drive stay as they
//! are; this only adds the paths they stand on.

use super::*;

/// The box of a vehicle body (`[boundingbox]` of the part at `position`, facing `heading`)
/// as the traffic sees the player's: centre, heading, half length, half width, speed.
pub fn box_outline(position: DVec3, heading: f64, bb: [f32; 6], speed: f32) -> PlayerBox {
    let h = heading.to_radians();
    let centre = position
        + DVec3::new(
            (bb[3] as f64) * h.cos() + (bb[4] as f64) * h.sin(),
            -(bb[3] as f64) * h.sin() + (bb[4] as f64) * h.cos(),
            0.0,
        );
    (centre, heading, bb[1] * 0.5, bb[0] * 0.5, speed)
}

/// Where `pos` lies along the path (m, not held to its ends: a short path's clamped
/// nearest point alone kept a vehicle on it after it had left) and how far beside it.
/// None when the vehicle faces another way or is on another level.
fn position_on_path(lane: &crate::traffic::Lane, pos: DVec3, heading: f64) -> Option<(f32, f64)> {
    let (s, _) = lane.nearest_point(pos)?;
    let (point, path_heading) = lane.at(s);
    let turn = (path_heading as f64 - heading + 540.0).rem_euclid(360.0) - 180.0;
    if turn.abs() > 45.0 || (point.z - pos.z).abs() > 2.0 {
        return None;
    }
    let h = (path_heading as f64).to_radians();
    let forward = DVec2::new(h.sin(), h.cos());
    let delta = (pos - point).truncate();
    Some((s + delta.dot(forward) as f32, delta.perp_dot(forward).abs()))
}

/// The light paths marked as a turn the way `blinker` shows (1 left, 2 right) that the
/// vehicle box `b` stands on: its centre within the path's width (not a fixed search
/// distance, which would take the neighbouring bus bay too), from its front reaching the
/// path's start until its rear has left the end.
pub fn indicated_light_paths(net: &Network, b: &PlayerBox, blinker: u8) -> Vec<usize> {
    if !matches!(blinker, 1 | 2) {
        return Vec::new();
    }
    let (pos, heading, half_len, half_width, _) = *b;
    let (cx, cy) = Network::grid_cell(pos);
    let radius = ((half_len + half_width).max(0.0) as f64 / crate::traffic::GRID_CELL) as i32 + 1;
    let wanted = |i: usize| {
        let lane = &net.lanes[i];
        lane.kind == LaneKind::Street && lane.traffic_light.is_some() && lane.turn == blinker as i32
    };
    let mut candidates: Vec<usize> = if net.grid.is_empty() {
        (0..net.lanes.len()).filter(|&i| wanted(i)).collect()
    } else {
        let mut v = Vec::new();
        for x in cx - radius..=cx + radius {
            for y in cy - radius..=cy + radius {
                if let Some(lanes) = net.grid.get(&(x, y)) {
                    v.extend(lanes.iter().copied().filter(|&i| wanted(i)));
                }
            }
        }
        v
    };
    candidates.sort_unstable();
    candidates.dedup();
    candidates.retain(|&i| {
        let lane = &net.lanes[i];
        position_on_path(lane, pos, heading).is_some_and(|(s, across)| {
            across <= lane.width.max(0.0) as f64 * 0.5 + 0.25
                && s + half_len >= 0.0
                && s - half_len <= lane.length()
        })
    });
    candidates
}

impl TrafficSim {
    /// `indicated_light_paths` of the box asks those paths' lights.
    pub(crate) fn request_indicated(&mut self, b: &PlayerBox, blinker: u8) {
        for lane in indicated_light_paths(&self.net, b, blinker) {
            if let Some((ci, li)) = self.net.lanes[lane].traffic_light {
                if let Some(r) = self.lights.get_mut(ci).and_then(|c| c.request.get_mut(li)) {
                    *r = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::{Lane, LaneBuilder};

    fn path(x: f64, y: f64, length: f64, width: f32, turn: i32) -> Lane {
        let mut lane = LaneBuilder::arc(DVec3::new(x, y, 0.0), 0.0, length, 0.0, 0.0, LaneKind::Street, width);
        lane.turn = turn;
        lane.traffic_light = Some((0, 0));
        lane
    }

    fn network(lanes: Vec<Lane>) -> Network {
        let mut net = Network { lanes, ..Default::default() };
        net.build_grid();
        net
    }

    /// A road and, laid over it in a bus bay, a 6 m detector marked as a right turn.
    fn overlapping(road_x: f64) -> Network {
        let mut road = path(road_x, -100.0, 300.0, 3.5, 0);
        road.traffic_light = None;
        let mut detector = path(0.15, 10.0, 6.0, 2.8, 2);
        // (it detects even where its rules keep the AI from choosing it)
        detector.density = 0.0;
        network(vec![road, detector])
    }

    fn bus(x: f64, y: f64) -> PlayerBox {
        (DVec3::new(x, y, 0.0), 0.0, 6.0, 1.25, 0.0)
    }

    fn asks(net: &Network, vehicles: &[(PlayerBox, u8)]) -> bool {
        vehicles.iter().any(|(b, blinker)| !indicated_light_paths(net, b, *blinker).is_empty())
    }

    #[test]
    fn an_overlapping_detector_reads_the_indicator_whichever_road_is_nearer() {
        for road_x in [0.0, 0.4] {
            let net = overlapping(road_x);
            let nearest = net.lane_along(bus(0.0, 13.0).0, 0.0, LaneKind::Street, 2.5, 45.0);
            assert_eq!(nearest.unwrap().0, if road_x == 0.0 { 0 } else { 1 });
            for blinker in [0, 1, 3] {
                assert!(!asks(&net, &[(bus(0.0, 13.0), blinker)]));
            }
            assert!(asks(&net, &[(bus(0.0, 13.0), 2)]));
        }
    }

    #[test]
    fn a_request_lasts_from_front_entry_until_rear_exit() {
        let net = overlapping(0.0);
        assert!(!asks(&net, &[(bus(0.0, 3.9), 2)]));
        assert!(asks(&net, &[(bus(0.0, 4.1), 2)]));
        assert!(asks(&net, &[(bus(0.0, 21.9), 2)]));
        assert!(!asks(&net, &[(bus(0.0, 22.1), 2)]));
        // an articulated bus's rear section keeps it after the front has left
        assert!(asks(&net, &[(bus(0.0, 28.0), 2), (bus(0.0, 17.0), 2)]));
    }

    #[test]
    fn rotated_detectors_are_found_across_a_grid_cell_boundary() {
        let mut detector = LaneBuilder::arc(DVec3::new(49.0, 0.0, 0.0), 90.0, 6.0, 0.0, 0.0, LaneKind::Street, 3.0);
        detector.turn = 2;
        detector.traffic_light = Some((0, 0));
        let net = network(vec![detector]);
        let vehicle = (DVec3::new(52.0, 0.0, 0.0), 90.0, 6.0, 1.25, 0.0);
        assert!(asks(&net, &[(vehicle, 2)]));
        assert!(!asks(&net, &[(vehicle, 0)]));
    }

    #[test]
    fn neighbouring_opposite_and_overhead_vehicles_do_not_ask() {
        for net in [overlapping(0.0), network(vec![path(0.15, 10.0, 6.0, 2.8, 2)])] {
            assert!(!asks(&net, &[(bus(2.0, 13.0), 2)]));
            let mut opposite = bus(0.0, 13.0);
            opposite.1 = 180.0;
            assert!(!asks(&net, &[(opposite, 2)]));
            let mut overhead = bus(0.0, 13.0);
            overhead.0.z = 5.0;
            assert!(!asks(&net, &[(overhead, 2)]));
        }
    }

    #[test]
    fn only_paths_marked_with_the_indicators_turn_detect() {
        let left = network(vec![path(0.0, 10.0, 6.0, 3.0, 1)]);
        assert!(asks(&left, &[(bus(0.0, 13.0), 1)]));
        for blinker in [0, 2, 3] {
            assert!(!asks(&left, &[(bus(0.0, 13.0), blinker)]));
        }
        // unmarked light paths are asked along the way, as before, not by standing on them
        let straight = network(vec![path(0.0, 10.0, 6.0, 3.0, 0)]);
        for blinker in [0, 1, 2, 3] {
            assert!(!asks(&straight, &[(bus(0.0, 13.0), blinker)]));
        }
    }

    /// The doors' cycle: red without a request, held in its open phase while a bus with
    /// its right indicator stands on the detector, round to red once it has left.
    #[test]
    fn a_detector_drives_a_scripted_open_hold_and_close_cycle() {
        let net = overlapping(0.0);
        let mut ctl = TrafficLightController::from_program(
            vec![(vec![(0, 1.0), (6, 2.0), (0, 1.0)], None)],
            Some(4.0),
            &[[0.0, 2.0, 0.0]],
            &[[0.0, 0.75, 1.0, 0.25]],
        );
        let mut run = |b: PlayerBox, blinker: u8, frames: usize| {
            for _ in 0..frames {
                ctl.request.fill(false);
                for lane in indicated_light_paths(&net, &b, blinker) {
                    let (_, li) = net.lanes[lane].traffic_light.unwrap();
                    ctl.request[li] = true;
                }
                ctl.advance(0.1);
            }
            ctl.state(0)
        };
        assert_eq!(run(bus(0.0, 13.0), 0, 40), 0);
        assert_eq!(run(bus(0.0, 13.0), 2, 40), 6);
        assert_eq!(run(bus(0.0, 21.9), 2, 40), 6);
        assert_eq!(run(bus(0.0, 22.1), 2, 40), 0);
        assert_eq!(run(bus(0.0, 13.0), 2, 40), 6);
        assert_eq!(run(bus(0.0, 13.0), 0, 40), 0);
    }
}
