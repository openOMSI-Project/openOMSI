//! The way a player's line drives from stop to stop, on the whole map's lane network (the one
//! the launcher's map reads, `mapview`), as the lanes the game's timetable names: what the
//! line editor saves as a trip's track (`core::lines::LaneStep`).
//!
//! The rules are Omsi-Hub's stop router (`routing.ts`, measured on the stock maps), put onto
//! openOMSI's own network:
//! * a stop is reached from up to `ANCHORS` lanes within `REACH` metres (`FAR` when none is),
//!   one with the stop on its kerb side preferred by `WRONG_SIDE` metres;
//! * a lane may be left for the one beside it of the same road (`LANE_CHANGE`);
//! * a lane end that the map leaves loose is joined to a lane start within `LOOSE` metres
//!   that runs the same way (the network's own joins take 1.5 m: Spandau's stop legs were
//!   two thirds routable without it) - the game bridges such a gap itself;
//! * a lane that turns round costs `U_TURN` more;
//! * the search gives up beyond five times the straight distance and 1.5 km more.

use glam::{DVec2, DVec3};
use hashbrown::HashMap;
use omsi_launcher_lib::lines::LaneStep;
use omsi_sim::traffic::{wrap_deg, Lane, LaneKind, Network};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;

pub const ANCHORS: usize = 4;
pub const REACH: f64 = 30.0;
pub const FAR: f64 = 60.0;
pub const WRONG_SIDE: f32 = 20.0;
pub const LANE_CHANGE: f32 = 25.0;
pub const LOOSE: f64 = 12.0;
pub const U_TURN: f32 = 400.0;
/// What a metre off a dragged point costs.
pub const VIA_WEIGHT: f32 = 10.0;

/// Where a stop (or a point a leg is dragged through) meets a lane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub lane: usize,
    /// Along the lane, and beside it (right of its direction positive).
    pub s: f32,
    pub lateral: f32,
    /// What reaching it here costs over the way itself (metres).
    pub penalty: f32,
    /// A lane changed onto at once: the lane the anchor was found on.
    pub beside: Option<usize>,
}

/// A way from one anchor to another: the lanes in order (the first the one it starts on, a
/// lane changed onto at once after it) and its length.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub lanes: Vec<usize>,
    pub from: Anchor,
    pub to: Anchor,
    pub length: f32,
}

/// The network with the joins the router adds to it.
pub struct Router {
    pub net: Arc<Network>,
    pub left_hand: bool,
    /// Loose lane ends → lane starts near them that run on the same way, with the gap.
    loose: HashMap<usize, Vec<(usize, f32)>>,
}

fn is_street(l: &Lane) -> bool {
    l.kind == LaneKind::Street && l.points.len() >= 2
}

/// The point of a lane nearest to `p` on the ground: (along, distance, beside - right of the
/// lane's direction positive).
fn nearest_2d(l: &Lane, p: DVec2) -> Option<(f32, f64, f32)> {
    let mut best: Option<(f32, f64, f32)> = None;
    for k in 0..l.points.len().saturating_sub(1) {
        let (a, b) = (l.points[k].truncate(), l.points[k + 1].truncate());
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
        let q = a + ab * t;
        let d = (p - q).length();
        if best.map(|x| d < x.1).unwrap_or(true) {
            let dir = ab.normalize_or_zero();
            let rel = p - q;
            let lateral = (rel.x * dir.y - rel.y * dir.x) as f32;
            best = Some((l.dist[k] + (l.dist[k + 1] - l.dist[k]) * t as f32, d, lateral));
        }
    }
    best
}

#[derive(Copy, Clone, PartialEq)]
struct State {
    cost: f32,
    lane: usize,
}
impl Eq for State {}
impl Ord for State {
    fn cmp(&self, o: &Self) -> Ordering {
        o.cost.total_cmp(&self.cost)
    }
}
impl PartialOrd for State {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

#[derive(Clone, Copy, PartialEq)]
enum From {
    None,
    Lane(usize),
    Seed(usize),
}

impl Router {
    pub fn new(net: Arc<Network>, left_hand: bool) -> Router {
        let mut loose: HashMap<usize, Vec<(usize, f32)>> = HashMap::new();
        for (i, l) in net.lanes.iter().enumerate() {
            if !is_street(l) || l.next.iter().any(|&j| net.lanes.get(j).is_some_and(is_street)) {
                continue;
            }
            let e = l.end();
            let eh = l.end_heading();
            let mut found = Vec::new();
            for j in starts_near(&net, e.truncate(), LOOSE) {
                let m = &net.lanes[j];
                if j == i || !is_street(m) {
                    continue;
                }
                let gap = (m.start() - e).truncate().length();
                if gap <= LOOSE && wrap_deg(m.start_heading() - eh).abs() <= 40.0 && (m.start().z - e.z).abs() < 3.0 {
                    found.push((j, gap as f32));
                }
            }
            if !found.is_empty() {
                loose.insert(i, found);
            }
        }
        Router { net, left_hand, loose }
    }

    /// How many loose ends the router joins (the log says it).
    pub fn joined(&self) -> usize {
        self.loose.len()
    }

    /// The lanes a stop at `p` (or a dragged point, `kerb` false) is reached from, the best
    /// first.
    pub fn anchors(&self, p: DVec2, kerb: bool) -> Vec<Anchor> {
        for reach in [REACH, FAR] {
            let mut out: Vec<Anchor> = Vec::new();
            for i in lanes_near(&self.net, p, reach) {
                let l = &self.net.lanes[i];
                if !is_street(l) {
                    continue;
                }
                let Some((s, d, lateral)) = nearest_2d(l, p) else { continue };
                if d > reach {
                    continue;
                }
                let on_kerb = if self.left_hand { lateral < -0.3 } else { lateral > 0.3 };
                // (a point the player dragged the leg through means the lane under it: the
                // one beside it costs a lane change and more)
                let penalty = if kerb { d as f32 + if on_kerb { 0.0 } else { WRONG_SIDE } } else { d as f32 * VIA_WEIGHT };
                out.push(Anchor { lane: i, s, lateral, penalty, beside: None });
            }
            if !out.is_empty() {
                out.sort_by(|a, b| a.penalty.total_cmp(&b.penalty));
                out.truncate(ANCHORS);
                return out;
            }
        }
        Vec::new()
    }

    /// The anchors with the lanes beside them added (changing lanes at once, `LANE_CHANGE`).
    fn with_beside(&self, from: &[Anchor]) -> Vec<Anchor> {
        let mut out = from.to_vec();
        for a in from {
            let l = &self.net.lanes[a.lane];
            for c in [l.left, l.right].into_iter().flatten() {
                if out.iter().any(|x| x.lane == c) {
                    continue;
                }
                let lc = &self.net.lanes[c];
                let s = if l.length() > 0.0 { a.s / l.length() * lc.length() } else { 0.0 };
                out.push(Anchor { lane: c, s, lateral: a.lateral, penalty: a.penalty + LANE_CHANGE, beside: Some(a.lane) });
            }
        }
        out
    }

    /// What it costs to go on from the end of lane `x` into `n` beyond the lane itself.
    fn turn(&self, n: usize) -> f32 {
        let l = &self.net.lanes[n];
        if wrap_deg(l.end_heading() - l.start_heading()).abs() > 150.0 {
            U_TURN
        } else {
            0.0
        }
    }

    /// The lanes after `x` with what the step costs over their length (a loose join: the gap).
    fn after(&self, x: usize) -> Vec<(usize, f32)> {
        let l = &self.net.lanes[x];
        let mut v: Vec<(usize, f32)> = l.next.iter().copied().filter(|&n| self.net.lanes.get(n).is_some_and(is_street)).map(|n| (n, self.turn(n))).collect();
        if let Some(extra) = self.loose.get(&x) {
            v.extend(extra.iter().map(|&(n, gap)| (n, gap + 5.0 + self.turn(n))));
        }
        v
    }

    /// The cheapest way from any of `from` to any of `to`, giving up beyond `cap` metres.
    pub fn route(&self, from: &[Anchor], to: &[Anchor], cap: f32) -> Option<Route> {
        let net = &self.net;
        let n = net.lanes.len();
        let from = self.with_beside(from);
        let mut best: Option<(f32, Route)> = None;
        let consider = |cost: f32, r: Route, best: &mut Option<(f32, Route)>| {
            if best.as_ref().map(|b| cost < b.0).unwrap_or(true) {
                *best = Some((cost, r));
            }
        };
        // on the same lane, ahead
        for a in &from {
            for t in to.iter().filter(|t| t.lane == a.lane && t.s >= a.s - 0.5) {
                let lanes = a.beside.into_iter().chain([a.lane]).collect();
                consider(t.s - a.s + a.penalty + t.penalty, Route { lanes, from: *a, to: *t, length: (t.s - a.s).max(0.0) }, &mut best);
            }
        }
        let mut dist = vec![f32::INFINITY; n];
        let mut prev = vec![From::None; n];
        let mut heap = BinaryHeap::new();
        for (k, a) in from.iter().enumerate() {
            let end = a.penalty + net.lanes[a.lane].length() - a.s;
            for (m, extra) in self.after(a.lane) {
                let c = end + extra;
                if c < dist[m] {
                    dist[m] = c;
                    prev[m] = From::Seed(k);
                    heap.push(State { cost: c, lane: m });
                }
            }
        }
        while let Some(State { cost, lane }) = heap.pop() {
            if cost > dist[lane] {
                continue;
            }
            if cost > cap || best.as_ref().is_some_and(|b| cost >= b.0) {
                break;
            }
            for t in to.iter().filter(|t| t.lane == lane) {
                let total = cost + t.s + t.penalty;
                if best.as_ref().map(|b| total < b.0).unwrap_or(true) {
                    // the lanes back to the anchor it started at
                    let mut lanes = vec![lane];
                    let mut at = lane;
                    let mut seed = 0;
                    let mut guard = 0;
                    loop {
                        match prev[at] {
                            From::Lane(p) => {
                                lanes.push(p);
                                at = p;
                            }
                            From::Seed(k) => {
                                seed = k;
                                break;
                            }
                            From::None => break,
                        }
                        guard += 1;
                        if guard > n + 2 {
                            break;
                        }
                    }
                    let a = from[seed];
                    lanes.push(a.lane);
                    if let Some(b) = a.beside {
                        lanes.push(b);
                    }
                    lanes.reverse();
                    best = Some((total, Route { lanes, from: a, to: *t, length: 0.0 }));
                }
            }
            let len = net.lanes[lane].length();
            for (m, extra) in self.after(lane) {
                let c = cost + len + extra;
                if c < dist[m] {
                    dist[m] = c;
                    prev[m] = From::Lane(lane);
                    heap.push(State { cost: c, lane: m });
                }
            }
            let l = &net.lanes[lane];
            for c in [l.left, l.right].into_iter().flatten() {
                let v = cost + LANE_CHANGE;
                if v < dist[c] {
                    dist[c] = v;
                    prev[c] = From::Lane(lane);
                    heap.push(State { cost: v, lane: c });
                }
            }
        }
        let (_, mut r) = best?;
        r.length = self.driven(&r);
        Some(r)
    }

    /// The metres a route drives: from where it starts on its first lane to where it ends on
    /// its last, every lane between whole (a lane left beside the next at once is not driven).
    fn driven(&self, r: &Route) -> f32 {
        let net = &self.net;
        let start = usize::from(r.from.beside.is_some());
        let lanes = &r.lanes[start.min(r.lanes.len().saturating_sub(1))..];
        if lanes.len() <= 1 {
            return (r.to.s - r.from.s).max(0.0);
        }
        let mut sum: f32 = lanes.iter().map(|&i| net.lanes[i].length()).sum();
        sum -= r.from.s;
        sum -= net.lanes[*lanes.last().unwrap()].length() - r.to.s;
        // (a lane changed onto beside the one before is driven only from there)
        for w in lanes.windows(2) {
            let (a, b) = (&net.lanes[w[0]], &net.lanes[w[1]]);
            if a.left == Some(w[1]) || a.right == Some(w[1]) {
                sum -= a.length().min(b.length());
            }
        }
        sum.max(0.0)
    }

    /// The way from `a` to `b` through `vias` (each a point the player dragged the leg
    /// through): None when some part has no way within its cap. `start` fixes the anchor
    /// the leg begins at (where the leg before ended), else `a`'s are tried.
    pub fn leg(&self, a: DVec2, b: DVec2, vias: &[DVec2], start: Option<Anchor>) -> Option<Route> {
        let mut points = vec![a];
        points.extend_from_slice(vias);
        points.push(b);
        let mut from: Vec<Anchor> = match start {
            Some(s) => vec![s],
            None => self.anchors(a, true),
        };
        let mut whole: Option<Route> = None;
        for k in 1..points.len() {
            let last = k == points.len() - 1;
            let to = self.anchors(points[k], last);
            if from.is_empty() || to.is_empty() {
                return None;
            }
            let straight = (points[k] - points[k - 1]).length() as f32;
            let r = self.route(&from, &to, straight * 5.0 + 1500.0)?;
            from = vec![Anchor { beside: None, ..r.to }];
            whole = Some(match whole {
                None => r,
                Some(mut w) => {
                    // (the point the leg was dragged through is on the last lane of the part
                    // before and the first of this one)
                    let skip = usize::from(r.lanes.first() == w.lanes.last());
                    w.lanes.extend_from_slice(&r.lanes[skip..]);
                    w.length += r.length;
                    w.to = r.to;
                    w
                }
            });
        }
        whole
    }

    /// A route as the registry keeps it: the keyed lanes in order, each once in a row (a
    /// lane without a key - a joint the network made - is left for the game to bridge).
    pub fn steps(&self, r: &Route) -> Vec<LaneStep> {
        let mut out: Vec<LaneStep> = Vec::new();
        for &i in &r.lanes {
            let l = &self.net.lanes[i];
            let Some(k) = l.key else { continue };
            let s = LaneStep { tile: [k.tile.0, k.tile.1], id: k.id, path: k.path, reversed: l.reversed, length: l.length() };
            if out.last().map(|p| (p.tile, p.id, p.path, p.reversed) == (s.tile, s.id, s.path, s.reversed)).unwrap_or(false) {
                continue;
            }
            out.push(s);
        }
        out
    }

    /// The lanes of a saved leg on this network (a lane the map no longer has is left out).
    pub fn lanes_of(&self, steps: &[LaneStep]) -> Vec<usize> {
        steps
            .iter()
            .filter_map(|s| self.net.find(omsi_sim::traffic::LaneKey { tile: (s.tile[0], s.tile[1]), id: s.id, path: s.path }, Some(s.reversed)))
            .collect()
    }

    /// The ground points of a leg: its lanes in order, the first from `from_s` and the last up
    /// to `to_s`.
    pub fn points(&self, lanes: &[usize], from_s: f32, to_s: f32) -> Vec<DVec2> {
        let mut out: Vec<DVec2> = Vec::new();
        let n = lanes.len();
        for (k, &i) in lanes.iter().enumerate() {
            let Some(l) = self.net.lanes.get(i) else { continue };
            let lo = if k == 0 { from_s } else { 0.0 };
            let hi = if k + 1 == n { to_s } else { l.length() };
            if k == 0 && n == 1 && hi < lo {
                continue;
            }
            out.push(l.at(lo).0.truncate());
            for (q, d) in l.points.iter().zip(&l.dist) {
                if *d > lo && *d < hi {
                    out.push(q.truncate());
                }
            }
            out.push(l.at(hi.max(lo)).0.truncate());
        }
        out
    }
}

/// Lanes whose geometry lies in a grid cell within `r` of `p`.
fn lanes_near(net: &Network, p: DVec2, r: f64) -> Vec<usize> {
    let (x0, y0) = Network::grid_cell(DVec3::new(p.x - r, p.y - r, 0.0));
    let (x1, y1) = Network::grid_cell(DVec3::new(p.x + r, p.y + r, 0.0));
    let mut out = Vec::new();
    for x in x0..=x1 {
        for y in y0..=y1 {
            if let Some(v) = net.grid.get(&(x, y)) {
                out.extend_from_slice(v);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Lanes that start within `r` of `p`.
fn starts_near(net: &Network, p: DVec2, r: f64) -> Vec<usize> {
    net.lanes_starting_near(DVec3::new(p.x, p.y, 0.0), r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use omsi_sim::traffic::{LaneBuilder, LaneKey};

    /// A street east and west along y = 0: lanes of 100 m, the eastbound 2 m south of the
    /// middle (heading 90), the westbound 2 m north. `gap` leaves a hole of that many metres
    /// before the third eastbound lane.
    fn street(gap: f64, extra: Vec<Lane>) -> Network {
        let mut lanes = Vec::new();
        for k in 0..3 {
            let x0 = k as f64 * 100.0 + if k == 2 { gap } else { 0.0 };
            let mut l = LaneBuilder::polyline((0..=10).map(|i| DVec3::new(x0 + i as f64 * 10.0, -2.0, 0.0)).collect(), LaneKind::Street, 3.0);
            l.key = Some(LaneKey { tile: (0, 0), id: k, path: 0 });
            lanes.push(l);
        }
        for k in 0..3 {
            let x0 = 300.0 - k as f64 * 100.0;
            let mut l = LaneBuilder::polyline((0..=10).map(|i| DVec3::new(x0 - i as f64 * 10.0, 2.0, 0.0)).collect(), LaneKind::Street, 3.0);
            l.key = Some(LaneKey { tile: (0, 0), id: 10 + k, path: 0 });
            lanes.push(l);
        }
        lanes.extend(extra);
        let mut net = Network { lanes, ..Default::default() };
        net.link(1.5);
        net
    }

    fn router(net: Network) -> Router {
        Router::new(Arc::new(net), false)
    }

    #[test]
    fn a_stop_on_the_kerb_is_reached_on_its_own_side() {
        let r = router(street(0.0, Vec::new()));
        // two stops on the south kerb (right of the eastbound lanes)
        let a = r.anchors(DVec2::new(20.0, -5.0), true);
        assert_eq!(a[0].lane, 0);
        let leg = r.leg(DVec2::new(20.0, -5.0), DVec2::new(250.0, -5.0), &[], None).expect("a way");
        assert_eq!(leg.lanes, vec![0, 1, 2]);
        assert!((leg.length - 230.0).abs() < 1.0, "{}", leg.length);
        let steps = r.steps(&leg);
        assert_eq!(steps.iter().map(|s| s.id).collect::<Vec<_>>(), vec![0, 1, 2]);
        // and back on the north kerb: the westbound lanes
        let back = r.leg(DVec2::new(250.0, 5.0), DVec2::new(20.0, 5.0), &[], None).expect("a way back");
        assert_eq!(back.lanes, vec![3, 4, 5]);
    }

    #[test]
    fn a_loose_end_is_bridged_and_a_way_that_is_none_is_none() {
        // the third eastbound lane starts 8 m after the second ends
        let net = street(8.0, Vec::new());
        assert!(net.lanes[1].next.is_empty());
        let r = router(net);
        assert_eq!(r.joined(), 1);
        let leg = r.leg(DVec2::new(20.0, -5.0), DVec2::new(280.0, -5.0), &[], None).expect("bridged");
        assert_eq!(leg.lanes, vec![0, 1, 2]);
        // to a stop behind on the same kerb, with no way round on this street: the lanes of
        // the other side, the stops reached across the road (they cost, but they are a way)
        let back = r.leg(DVec2::new(250.0, -5.0), DVec2::new(20.0, -5.0), &[], None).expect("across");
        assert_eq!(back.lanes, vec![3, 4, 5]);
        // a stop far from every road has none
        assert!(r.leg(DVec2::new(20.0, -5.0), DVec2::new(20.0, 500.0), &[], None).is_none());
    }

    #[test]
    fn a_leg_dragged_through_a_point_takes_the_lane_beside() {
        // a second eastbound lane beside the first (an overtaking lane of the same spline)
        let mut l = LaneBuilder::polyline((0..=10).map(|i| DVec3::new(i as f64 * 10.0, 1.0, 0.0)).collect(), LaneKind::Street, 3.0);
        l.key = Some(LaneKey { tile: (0, 0), id: 0, path: 1 });
        l.source = 1;
        l.offset = -3.0;
        let mut net = street(0.0, vec![l]);
        net.lanes[0].source = 1;
        net.lanes[0].left = Some(6);
        net.lanes[6].right = Some(0);
        let r = router(net);
        // through a point on the overtaking lane: the bus changes lanes there
        let leg = r.leg(DVec2::new(5.0, -5.0), DVec2::new(250.0, -5.0), &[DVec2::new(60.0, 1.0)], None).expect("a way");
        assert!(leg.lanes.contains(&6), "{:?}", leg.lanes);
        assert_eq!(*leg.lanes.last().unwrap(), 2);
        // without it, the way stays in its lane
        let plain = r.leg(DVec2::new(5.0, -5.0), DVec2::new(250.0, -5.0), &[], None).unwrap();
        assert!(!plain.lanes.contains(&6));
    }

    #[test]
    fn a_way_beyond_its_cap_is_given_up() {
        let r = router(street(0.0, Vec::new()));
        let from = r.anchors(DVec2::new(20.0, -5.0), true);
        let to = r.anchors(DVec2::new(250.0, -5.0), true);
        assert!(r.route(&from, &to, 100.0).is_none());
        assert!(r.route(&from, &to, 1000.0).is_some());
        // the saved steps find their lanes again
        let leg = r.route(&from, &to, 1000.0).unwrap();
        assert_eq!(r.lanes_of(&r.steps(&leg)), leg.lanes);
        // and its points run from the first stop's place to the second's
        let pts = r.points(&leg.lanes, leg.from.s, leg.to.s);
        assert!((pts[0] - DVec2::new(20.0, -2.0)).length() < 0.5);
        assert!((*pts.last().unwrap() - DVec2::new(250.0, -2.0)).length() < 0.5);
    }
}
