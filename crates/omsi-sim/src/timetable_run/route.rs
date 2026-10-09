//! A trip's route on the lanes of the network, as far as it needs the network only: how
//! its lanes join, the ways across gaps, where the bus is on it and its stops placed on it.

use super::*;
use crate::ai_traffic::TrafficSim;
use crate::traffic::Network;

/// The bus stands next to the kerb: the pole's offset less half a bus width and a gap; only
/// where the pole is clearly off the lane (a bay).
///
/// A pole further off than a bay's width stands behind the pavement or the verge (the
/// stop objects of many maps are placed there): the bus stays in its lane at the kerb then.
/// Taken as a bay up to 4 m wide, the bus pulled out over the kerb onto the grass.
///
/// Not at all, now: a timetable bus stays on its path at the stop, as OMSI's do (a
/// map's bus bay is a spline of its own that the route runs through). The pole's offset
/// says nothing about where the kerb is - most stand behind the pavement - and a bus
/// moved 1.6 m to the right of its lane drove along with its right wheels on the pavement.
/// A box on the *opposite* side or more than 8 m from its authored route lane cannot be
/// reached as a bay: following that offset sends the bus across the NCC median/platform.
/// Stops moved `shift` metres back along `route` (the lanes the stops' route indices less
/// `base` count in): where the vehicle's origin comes to rest (`bus_service::stop_shift`).
/// One that comes to lie before the route's first lane keeps a distance below zero on it.
pub(crate) fn shift_stops(net: &Network, route: &[usize], base: usize, stops: &mut [(usize, f32, f32, f64, i64, f32)], shift: f32) {
    if shift.abs() < 1e-3 {
        return;
    }
    for st in stops.iter_mut() {
        let (mut k, mut ss) = (st.0.saturating_sub(base), st.1 - shift);
        while ss < 0.0 && k > 0 && k <= route.len() - 1 {
            k -= 1;
            ss += net.lanes[route[k]].length();
        }
        while ss > 0.0 && k + 1 < route.len() && ss > net.lanes[route[k]].length() {
            ss -= net.lanes[route[k]].length();
            k += 1;
        }
        st.0 = base + k;
        st.1 = ss;
    }
}

pub fn bay_offset(lat: f32) -> f32 {
    lat
}

/// Where a timetable bus stands across its lane at a stop, as Omsi.exe puts it
/// (0x7dac5e..0x7dae81): its kerb-side flank 0.3 m past the `[busstop]` box's centre -
/// `lat` less its `[boundingbox]` lateral centre and half width plus 0.3 on the right (the other way round
/// where traffic keeps left), from the box's offset `lat` off the path (right positive);
/// a railway vehicle keeps to its track. OMSI clamps it only to the room beside other
/// vehicles, not to a kerb: the bus pulls into the bay whether or not a path leads there
/// (#241). (openOMSI kept it on its path before - a map whose box stood behind the
/// pavement had its buses on the pavement - but OMSI does the same there.)
pub(crate) fn bay_for(lat: f32, ty: &crate::VehicleType, rail: bool, left_hand: bool, side: f32) -> f32 {
    if rail || !lat.is_finite() {
        return 0.0;
    }
    let bb = ty.def.bounding_box.unwrap_or([2.5, 0.0, 0.0, 0.0, 0.0, 0.0]);
    let (hw, centre) = (bb[0] * 0.5, bb[3]);
    // Platform side is independent of traffic hand. With boarding on both sides,
    // align the flank facing this stop's box rather than assuming a right-hand kerb.
    let left = if side == 2.0 { lat < 0.0 } else { left_hand != (side == 1.0) };
    // The timetable's station step may point at a lane on the other side of a broad
    // platform. Its pole must not make the bus cross that platform to serve the stop.
    if lat.abs() > 8.0 || (left && lat > 0.3) || (!left && lat < -0.3) {
        return 0.0;
    }
    if left {
        lat - centre + hw - 0.3
    } else {
        lat - centre - hw + 0.3
    }
}

/// The stops' raw box offsets (see `bay_offset`) made the vehicle's bay offsets, and the
/// stops moved to where its origin comes to rest (`shift_stops`).
pub fn place_stops(net: &Network, route: &[usize], base: usize, stops: &mut [(usize, f32, f32, f64, i64, f32)], ty: &crate::VehicleType, rail: bool) {
    for st in stops.iter_mut() {
        st.2 = bay_for(st.2, ty, rail, net.left_hand, st.5);
    }
    shift_stops(net, route, base, stops, crate::ai_traffic::bus_service::stop_shift(ty, rail));
}

/// Where on `route` the bus stop at `pos` is: (route index, distance along that lane, lateral
/// offset); None when it is further than `reach` from the route.
/// Where the stop at `pos` lies on `route`, not before route index `from` (the stops come
/// in the trip's order): on the side of the road it stands, see
/// `Network::project_stop_on_route`.
pub fn project_stop(
    net: &Network,
    route: &[usize],
    pos: glam::DVec3,
    reach: Option<f64>,
    from: usize,
    side: f32,
    on: StopRoute,
) -> Option<(usize, f32, f32)> {
    match on {
        StopRoute::Nearest => net.project_stop_on_route_side(route, pos, reach, from, side as u8),
        StopRoute::Outside => None,
        StopRoute::Track(ri) => {
            let lane = *route.get(ri)?;
            let (_, s, lat) = net.project_on_route_lateral(&[lane], pos)?;
            let point = net.lanes[lane].at(s).0;
            if reach.is_some_and(|r| (point - pos).truncate().length() > r) {
                return None;
            }
            Some((ri, s, lat))
        }
    }
}

/// One-way paths that a route drives the other way - their end lies where the path
/// before it ends, their start where the next one begins - get a lane that way
/// (`Traffic::add_reverse_twins`). OMSI's timetable buses follow their station links
/// and tracks whichever way a path runs: Spandau's line to Kladow and a dozen Novi Sad
/// tracks run over invisible one-way helper streets backwards, and the bus drove them
/// forwards, against its route, and jumped back at their end.
pub fn add_twins(traffic: &mut TrafficSim, steps: &[Step]) {
    let net = &traffic.net;
    let cands: Vec<Option<&Vec<usize>>> = steps
        .iter()
        .map(|st| st.key.and_then(|k| net.by_key.get(&k)).filter(|c| !c.is_empty()))
        .collect();
    let ends = |c: &Vec<usize>| -> Vec<glam::DVec3> {
        c.iter().flat_map(|&l| [net.lanes[l].start(), net.lanes[l].end()]).collect()
    };
    let near = |p: glam::DVec3, pts: &[glam::DVec3]| {
        pts.iter().map(|q| (*q - p).truncate().length()).fold(f64::MAX, f64::min)
    };
    let mut want = Vec::new();
    for (i, c) in cands.iter().enumerate() {
        // a path that runs both ways has its lanes already
        let Some(c) = c.filter(|c| c.len() == 1) else { continue };
        let l = &net.lanes[c[0]];
        let prev = i.checked_sub(1).and_then(|k| cands[k]).map(ends);
        let next = cands.get(i + 1).copied().flatten().map(ends);
        if prev.is_none() && next.is_none() {
            continue;
        }
        let score = |a: glam::DVec3, b: glam::DVec3| {
            prev.as_ref().map(|p| near(a, p)).unwrap_or(0.0)
                + next.as_ref().map(|n| near(b, n)).unwrap_or(0.0)
        };
        let (fwd, bwd) = (score(l.start(), l.end()), score(l.end(), l.start()));
        if bwd + 3.0 < fwd && bwd < 6.0 {
            want.push(c[0]);
        }
    }
    if !want.is_empty() {
        traffic.add_reverse_twins(&want);
    }
}

/// Where a route's lanes do not join and the network has no way between them either,
/// a connector lane across the gap (`Traffic::add_connector`), so that `bridge_gaps`
/// finds a way to drive.
pub fn add_connectors(traffic: &mut TrafficSim, lanes: &[usize]) {
    let net = &traffic.net;
    let holes: Vec<(usize, usize)> = lanes
        .windows(2)
        .filter(|w| !joins(net, w[0], w[1]))
        .filter(|w| {
            let gap = (net.lanes[w[1]].start() - net.lanes[w[0]].end()).truncate().length();
            way_between(net, w[0], w[1], (gap * 2.5 + 60.0) as f32).is_none()
        })
        .map(|w| (w[0], w[1]))
        .collect();
    for (a, b) in holes {
        traffic.add_connector(a, b);
    }
}

/// Consecutive route lanes that a vehicle can drive from one into the other: linked, a lane
/// change beside it, or starting (almost) where the first ends.
pub fn joins(net: &Network, a: usize, b: usize) -> bool {
    net.lanes[a].next.contains(&b)
        || net.parallel(a, b)
        || (net.lanes[b].start() - net.lanes[a].end()).truncate().length() < 2.0
}

/// A station link often runs on past its station: the path search that made it went a
/// few paths beyond the stop - into a turning lane, round a corner - before the next link
/// starts back at the stop on another path (Spandau's links end so in 122 of 505 joins, the
/// extra paths mostly listed with length 0). Driven as listed, the bus turned off, then
/// jumped back and drove on the wrong side or against the traffic. Such a detour is passed
/// over (made `Absent`): where the route does not join, the lane a few steps back that
/// the next one continues from - or the lane a few steps on that continues this one - is
/// where the route really goes.
pub fn skip_detours(net: &Network, slots: &mut [Slot]) {
    const REACH: usize = 8;
    let lane_at = |slots: &[Slot], k: usize| match slots[k] {
        Slot::Lane(l) => Some(l),
        _ => None,
    };
    let mut i = 0;
    while i + 1 < slots.len() {
        let (Some(a), Some(b)) = (lane_at(slots, i), lane_at(slots, i + 1)) else {
            i += 1;
            continue;
        };
        if joins(net, a, b) {
            i += 1;
            continue;
        }
        // back: an earlier lane of the route that `b` continues
        let back = (i.saturating_sub(REACH)..i)
            .rev()
            .find(|&k| lane_at(slots, k).map(|x| joins(net, x, b)).unwrap_or(false));
        // on: a later lane that continues `a`
        let on = (i + 2..(i + 2 + REACH).min(slots.len()))
            .find(|&k| lane_at(slots, k).map(|x| joins(net, a, x)).unwrap_or(false));
        match (back, on) {
            (Some(k), Some(m)) if i - k <= m - i - 1 => slots[k + 1..=i].fill(Slot::Absent),
            (_, Some(m)) => slots[i + 1..m].fill(Slot::Absent),
            (Some(k), None) => slots[k + 1..=i].fill(Slot::Absent),
            (None, None) => {}
        }
        i += 1;
    }
}

/// Where consecutive lanes of a route do not join (a path the timetable file names that
/// the map does not have any more, a junction a mod map edited after its tracks were
/// made), the shortest way between them through the network, when there is one not much
/// longer than the gap: the bus drives it instead of jumping across. Returns the lanes and,
/// for each lane given, its index in them.
pub fn bridge_gaps(net: &Network, lanes: &[usize]) -> (Vec<usize>, Vec<usize>) {
    let mut out: Vec<usize> = Vec::with_capacity(lanes.len());
    let mut index = Vec::with_capacity(lanes.len());
    for (k, &b) in lanes.iter().enumerate() {
        if k > 0 {
            let a = lanes[k - 1];
            if !joins(net, a, b) {
                let gap = (net.lanes[b].start() - net.lanes[a].end()).truncate().length();
                if let Some(way) = way_between(net, a, b, (gap * 2.5 + 60.0) as f32) {
                    out.extend(way);
                }
            }
        }
        index.push(out.len());
        out.push(b);
    }
    (out, index)
}

/// The lanes strictly between `a` and `b` on the shortest way from the end of `a` to the
/// start of `b`, if that is at most `max` metres long.
pub(crate) fn way_between(net: &Network, a: usize, b: usize, max: f32) -> Option<Vec<usize>> {
    use std::cmp::Reverse;
    let mut best: HashMap<usize, (f32, usize)> = HashMap::new();
    let mut heap = std::collections::BinaryHeap::new();
    for &n in &net.lanes[a].next {
        heap.push((Reverse(ordered(0.0)), n, a));
    }
    while let Some((Reverse(c), l, from)) = heap.pop() {
        let c = c as f32 / 1000.0;
        if best.contains_key(&l) {
            continue;
        }
        best.insert(l, (c, from));
        if l == b {
            let mut way = Vec::new();
            let mut at = from;
            while at != a {
                way.push(at);
                at = best.get(&at)?.1;
            }
            way.reverse();
            return Some(way);
        }
        let c2 = c + net.lanes[l].length();
        if c2 > max {
            continue;
        }
        for &n in &net.lanes[l].next {
            if !best.contains_key(&n) {
                heap.push((Reverse(ordered(c2)), n, l));
            }
        }
    }
    None
}

/// A distance in millimetres, for ordering.
pub(crate) fn ordered(m: f32) -> u64 {
    (m.max(0.0) * 1000.0) as u64
}

/// A flight path: aircraft are not tied to the ground under them.
pub fn track_is_air(traffic: &TrafficSim, lane: usize) -> bool {
    traffic
        .net
        .lanes
        .get(lane)
        .map(|l| l.kind == crate::traffic::LaneKind::Air)
        .unwrap_or(false)
}

/// Where on its route a bus is: the step it is on and how far into it, from the leg it is on
/// (`leg`, `frac` of the way along) and the estimated length of every step (`est`; an absent
/// step has none, so the bus is on the next step there is). None when that is past the end.
pub fn step_at(
    steps: &[Step],
    slots: &[Slot],
    est: &[f64],
    leg: usize,
    frac: f64,
) -> Option<(usize, f64)> {
    let in_leg: Vec<usize> = (0..steps.len()).filter(|&k| steps[k].leg == leg).collect();
    let (mut at, mut offset) = match in_leg.last() {
        // a leg without a station link: the bus is at the start of the next one
        None => (steps.iter().position(|s| s.leg > leg)?, 0.0),
        Some(&last) => {
            let mut target = frac * in_leg.iter().map(|&k| est[k]).sum::<f64>();
            let mut pick = (last, est[last]);
            for &k in &in_leg {
                if est[k] > 0.0 && target <= est[k] {
                    pick = (k, target);
                    break;
                }
                target -= est[k];
            }
            pick
        }
    };
    while slots.get(at) == Some(&Slot::Absent) {
        at += 1;
        offset = 0.0;
    }
    (at < slots.len()).then_some((at, offset))
}

/// The steps around `at` that the network has, up to the steps still to come on either
/// side: (first, end).
pub fn section_around(slots: &[Slot], at: usize) -> (usize, usize) {
    let start = slots[..at]
        .iter()
        .rposition(|s| *s == Slot::Waiting)
        .map(|k| k + 1)
        .unwrap_or(0);
    let end = slots[at..]
        .iter()
        .position(|s| *s == Slot::Waiting)
        .map(|k| at + k)
        .unwrap_or(slots.len());
    (start, end)
}
