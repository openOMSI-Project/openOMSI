//! The fleet on the clock: where the bus of a tour is at a time of the company's day, from the
//! timetable and its route on the map (Omsi-Hub's `plekOpKlok` in `shared/vloot.ts`). Only
//! the timetable and a delay are used - what the game reports of the company's buses moves
//! the delay, not the place - so the launcher's fleet map can be drawn at any clock and these
//! rules tested on their own.
//!
//! A trip's route is a line in map metres (`Track`); its stops lie on it where the map has
//! them (`TripShape`). Between two stops the bus moves along the line by their times; before
//! its first trip it stands at the first stop (the depot run is a trip too), between trips at
//! the end of the last, after its last at the end.

use super::rng::Rng;
use crate::TripInfo;

/// A point in map metres (x east, y north).
pub type P = [f64; 2];

/// A route as a polyline with the distance along it at every point.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub points: Vec<P>,
    pub cum: Vec<f64>,
}

fn dist(a: P, b: P) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// A heading in degrees, north 0, clockwise.
fn heading(dx: f64, dy: f64) -> f64 {
    if dx == 0.0 && dy == 0.0 {
        return 0.0;
    }
    (dx.atan2(dy).to_degrees() + 360.0) % 360.0
}

impl Track {
    /// A track through `points` (points on top of each other once); None under two points.
    pub fn new(points: &[P]) -> Option<Track> {
        let mut pts: Vec<P> = Vec::with_capacity(points.len());
        for p in points.iter().filter(|p| p[0].is_finite() && p[1].is_finite()) {
            if pts.last().is_none_or(|q| dist(*q, *p) > 0.01) {
                pts.push(*p);
            }
        }
        if pts.len() < 2 {
            return None;
        }
        let mut cum = Vec::with_capacity(pts.len());
        let mut s = 0.0;
        cum.push(0.0);
        for w in pts.windows(2) {
            s += dist(w[0], w[1]);
            cum.push(s);
        }
        Some(Track { points: pts, cum })
    }

    pub fn length(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// The point `along` metres from the start (held to the track), and the heading there.
    pub fn at(&self, along: f64) -> (P, f64) {
        let a = along.clamp(0.0, self.length());
        let i = self.cum.partition_point(|c| *c <= a).clamp(1, self.points.len() - 1);
        let (p, q) = (self.points[i - 1], self.points[i]);
        let seg = self.cum[i] - self.cum[i - 1];
        let f = if seg > 0.0 { (a - self.cum[i - 1]) / seg } else { 0.0 };
        ([p[0] + (q[0] - p[0]) * f, p[1] + (q[1] - p[1]) * f], heading(q[0] - p[0], q[1] - p[1]))
    }

    /// The place along the track nearest to `p`, not before `from` (a trip's stops come in
    /// order, and a route that comes back past a stop must not take it the second time),
    /// and how far it is.
    pub fn nearest(&self, p: P, from: f64) -> (f64, f64) {
        let mut best = (from.max(0.0), f64::MAX);
        for i in 1..self.points.len() {
            if self.cum[i] < from {
                continue;
            }
            let (a, b) = (self.points[i - 1], self.points[i]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len2 = dx * dx + dy * dy;
            let t = if len2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let along = (self.cum[i - 1] + t * (self.cum[i] - self.cum[i - 1])).max(from);
            let (q, _) = self.at(along);
            let d = dist(q, p);
            if d < best.1 {
                best = (along, d);
            }
        }
        best
    }
}

/// A stop further from its trip's route than this is not on it.
pub const ON_ROUTE: f64 = 80.0;

/// A trip as a bus is moved along it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TripShape {
    /// Seconds of the day: its departure, its arrival, and each stop's time.
    pub dep: f64,
    pub arr: f64,
    pub times: Vec<f64>,
    pub names: Vec<String>,
    pub track: Option<Track>,
    /// Each stop's place along the track, and on the map (None: the map does not have it).
    pub along: Vec<Option<f64>>,
    pub places: Vec<Option<P>>,
    /// A depot run or a positioning trip (fewer than three stops: no passengers).
    pub empty: bool,
}

impl TripShape {
    /// A trip of the timetable with its route on the map (`route`, None: unknown) and its
    /// stops' places (`places`, one a stop).
    pub fn new(trip: &TripInfo, route: Option<&[P]>, places: Vec<Option<P>>) -> TripShape {
        let n = trip.stops.len();
        let times: Vec<f64> = trip.stops.iter().enumerate().map(|(k, s)| if k == 0 { trip.departure.max(s.dep) } else { s.arr }).collect();
        let track = route.and_then(Track::new);
        let mut along = vec![None; n];
        if let Some(t) = &track {
            let mut from = 0.0;
            for (k, p) in places.iter().enumerate().take(n) {
                let Some(p) = p else { continue };
                let (a, d) = t.nearest(*p, from);
                if d <= ON_ROUTE {
                    along[k] = Some(a);
                    from = a;
                }
            }
        }
        let mut places = places;
        places.resize(n, None);
        TripShape { dep: trip.departure, arr: trip.arrival.max(trip.departure), times, names: trip.stops.iter().map(|s| s.name.clone()).collect(), track, along, places, empty: n < 3 }
    }

    /// Where the bus is at `t` on this trip (a time outside it: its start or its end).
    pub fn at(&self, t: f64) -> Option<(P, f64)> {
        if let Some(track) = &self.track {
            let mut prev: Option<(f64, f64)> = None;
            let mut next: Option<(f64, f64)> = None;
            for (k, a) in self.along.iter().enumerate() {
                let (Some(a), Some(tk)) = (a, self.times.get(k)) else { continue };
                if *tk <= t {
                    prev = Some((*tk, *a));
                } else {
                    next = Some((*tk, *a));
                    break;
                }
            }
            let along = match (prev, next) {
                (None, None) => {
                    let d = self.arr - self.dep;
                    if d > 0.0 { track.length() * ((t - self.dep) / d).clamp(0.0, 1.0) } else if t > self.dep { track.length() } else { 0.0 }
                }
                (None, Some((_, a))) => {
                    // (before the first stop the map has: from the start of the route)
                    let d = self.times.iter().zip(&self.along).find(|x| x.1.is_some()).map(|x| *x.0).unwrap_or(self.arr) - self.dep;
                    if d > 0.0 { a * ((t - self.dep) / d).clamp(0.0, 1.0) } else { a }
                }
                (Some((tp, a)), None) => {
                    // (after the last stop the map has: on to the end of the route)
                    let d = self.arr - tp;
                    if d > 0.0 && t > tp { a + (track.length() - a) * ((t - tp) / d).clamp(0.0, 1.0) } else { a }
                }
                (Some((tp, ap)), Some((tn, an))) => {
                    let f = if tn > tp { ((t - tp) / (tn - tp)).clamp(0.0, 1.0) } else { 1.0 };
                    ap + f * (an - ap)
                }
            };
            return Some(track.at(along));
        }
        // without a route: straight from stop to stop
        let mut prev: Option<(f64, P)> = None;
        let mut next: Option<(f64, P)> = None;
        for (k, p) in self.places.iter().enumerate() {
            let (Some(p), Some(tk)) = (p, self.times.get(k)) else { continue };
            if *tk <= t {
                prev = Some((*tk, *p));
            } else {
                next = Some((*tk, *p));
                break;
            }
        }
        match (prev, next) {
            (None, None) => None,
            (None, Some((_, p))) | (Some((_, p)), None) => Some((p, 0.0)),
            (Some((tp, a)), Some((tn, b))) => {
                let f = if tn > tp { ((t - tp) / (tn - tp)).clamp(0.0, 1.0) } else { 1.0 };
                Some(([a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f], heading(b[0] - a[0], b[1] - a[1])))
            }
        }
    }
}

/// What a tour's bus is doing at a clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Before its first trip (at the depot, or on its way to the first stop).
    Depot,
    /// On a trip with passengers.
    Trip,
    /// On a depot run or a positioning trip.
    Empty,
    /// Standing between two trips.
    Pause,
    /// Its day is done.
    Done,
}

/// Where a tour's bus is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub state: State,
    /// The trip it drives, the next one (at the depot, in a pause) or the last (done).
    pub trip: usize,
    /// None: none of its trip's stops is on the map.
    pub at: Option<P>,
    pub heading: f64,
    /// The next stop (of `trip`) and when the bus is there, the delay counted in.
    pub next_stop: Option<usize>,
    pub next_time: Option<f64>,
}

/// Where the bus of a tour (`trips`, in their order) is at `clock` (seconds of the day),
/// `delay` seconds behind its timetable: it is where the timetable had it `delay` earlier.
pub fn place_at(trips: &[&TripShape], clock: f64, delay: f64) -> Place {
    let mut out = Place { state: State::Depot, trip: 0, at: None, heading: 0.0, next_stop: None, next_time: None };
    let Some(first) = trips.first() else { return out };
    let t = clock - delay;
    let set = |out: &mut Place, p: Option<(P, f64)>| {
        out.at = p.map(|x| x.0);
        out.heading = p.map(|x| x.1).unwrap_or(0.0);
    };
    if t < first.dep {
        set(&mut out, first.at(f64::NEG_INFINITY));
        out.next_stop = Some(0);
        out.next_time = Some(first.dep + delay);
        return out;
    }
    for (i, trip) in trips.iter().enumerate() {
        if t >= trip.dep && t <= trip.arr {
            out.state = if trip.empty { State::Empty } else { State::Trip };
            out.trip = i;
            set(&mut out, trip.at(t));
            let k = trip.times.iter().position(|tk| *tk > t).unwrap_or(trip.times.len().saturating_sub(1));
            out.next_stop = (!trip.times.is_empty()).then_some(k);
            out.next_time = Some(trip.times.get(k).copied().unwrap_or(trip.arr) + delay);
            return out;
        }
        if let Some(next) = trips.get(i + 1) {
            if t > trip.arr && t < next.dep {
                out.state = State::Pause;
                out.trip = i + 1;
                set(&mut out, trip.at(f64::INFINITY));
                out.next_stop = Some(0);
                out.next_time = Some(next.dep + delay);
                return out;
            }
        }
    }
    let last = trips.len() - 1;
    out.state = State::Done;
    out.trip = last;
    set(&mut out, trips[last].at(f64::INFINITY));
    out
}

/// How far behind its timetable a bus shows on a trip, in seconds: a minute early to a few
/// late, the same every time for the same day, tour and trip; a driver of little experience
/// (under 50) and a bus in a poor condition (under 80) fall further behind. Only for the map:
/// nothing is booked with it.
pub fn delay_of(date: &str, tour: &str, trip: usize, experience: f64, condition: f64) -> f64 {
    let mut r = Rng::of(&[date, tour], trip as i64);
    let mut minutes = -1.0 + r.f64().powi(2) * 5.0;
    if experience < 50.0 {
        minutes += (50.0 - experience) / 50.0 * 2.0;
    }
    if condition < 80.0 {
        minutes += (80.0 - condition) / 80.0 * 2.0;
    }
    minutes * 60.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StopInfo;

    fn trip(dep: f64, stops: &[(f64, &str)], empty: bool) -> TripInfo {
        let mut s: Vec<StopInfo> = stops.iter().enumerate().map(|(k, (t, n))| StopInfo { name: n.to_string(), id: k as i64, arr: *t, dep: *t }).collect();
        if empty {
            s.truncate(2);
        }
        TripInfo { name: "t".into(), index: 1, line: "5".into(), from: String::new(), terminus: String::new(), departure: dep, arrival: s.last().map(|x| x.arr).unwrap_or(dep), stops: s, km: 1.0 }
    }

    #[test]
    fn a_bus_moves_along_its_route_by_the_times_of_its_stops() {
        // a route east 1 km, then north; stops at 0, 500 and at the corner, 60 s apart
        let route: Vec<P> = vec![[0.0, 0.0], [1000.0, 0.0], [1000.0, 500.0]];
        let t = trip(0.0, &[(0.0, "A"), (60.0, "B"), (120.0, "C")], false);
        let s = TripShape::new(&t, Some(&route), vec![Some([0.0, 5.0]), Some([500.0, -4.0]), Some([1000.0, 0.0])]);
        assert_eq!(s.along, vec![Some(0.0), Some(500.0), Some(1000.0)]);
        let at = |c: f64| s.at(c).unwrap().0;
        assert!((at(30.0)[0] - 250.0).abs() < 1e-6);
        assert!((at(90.0)[0] - 750.0).abs() < 1e-6);
        // after the last stop the map has, on to the end of the route
        let (p, h) = s.at(110.0).unwrap();
        assert!((p[0] - 916.666_666).abs() < 1e-3 && h == 90.0);
        assert_eq!(s.at(120.0).unwrap().0, [1000.0, 0.0]);
        // a stop far from the route is not on it
        let far = TripShape::new(&t, Some(&route), vec![None, Some([500.0, 300.0]), None]);
        assert_eq!(far.along, vec![None, None, None]);
        assert!((far.at(60.0).unwrap().0[0] - 750.0).abs() < 1e-6, "by the time over the whole route");
        // without a route: straight from stop to stop
        let bare = TripShape::new(&t, None, vec![Some([0.0, 0.0]), None, Some([0.0, 1200.0])]);
        let (p, h) = bare.at(60.0).unwrap();
        assert_eq!((p, h), ([0.0, 600.0], 0.0));
    }

    #[test]
    fn a_tour_stands_at_the_depot_drives_pauses_and_is_done_and_a_delay_holds_it_back() {
        let route: Vec<P> = vec![[0.0, 0.0], [1200.0, 0.0]];
        let back: Vec<P> = vec![[1200.0, 0.0], [0.0, 0.0]];
        let out = TripShape::new(&trip(3600.0, &[(3600.0, "A"), (3660.0, "B"), (3720.0, "C")], false), Some(&route), vec![Some([0.0, 0.0]), Some([600.0, 0.0]), Some([1200.0, 0.0])]);
        let home = TripShape::new(&trip(3900.0, &[(3900.0, "C"), (4020.0, "A")], true), Some(&back), vec![Some([1200.0, 0.0]), Some([0.0, 0.0])]);
        let tour = [&out, &home];
        let p = place_at(&tour, 3000.0, 0.0);
        assert_eq!((p.state, p.trip, p.at, p.next_time), (State::Depot, 0, Some([0.0, 0.0]), Some(3600.0)));
        let p = place_at(&tour, 3630.0, 0.0);
        assert_eq!((p.state, p.trip, p.next_stop), (State::Trip, 0, Some(1)));
        assert!((p.at.unwrap()[0] - 300.0).abs() < 1e-6);
        let p = place_at(&tour, 3800.0, 0.0);
        assert_eq!((p.state, p.trip, p.at, p.next_time), (State::Pause, 1, Some([1200.0, 0.0]), Some(3900.0)));
        let p = place_at(&tour, 3960.0, 0.0);
        assert_eq!(p.state, State::Empty);
        assert!((p.at.unwrap()[0] - 600.0).abs() < 1e-6);
        assert_eq!(place_at(&tour, 5000.0, 0.0).state, State::Done);
        // two minutes late: at 3750 it is where the timetable had it at 3630
        let p = place_at(&tour, 3750.0, 120.0);
        assert_eq!(p.state, State::Trip);
        assert!((p.at.unwrap()[0] - 300.0).abs() < 1e-6);
        assert_eq!(p.next_time, Some(3660.0 + 120.0));
        assert_eq!(place_at(&[], 10.0, 0.0).at, None);
    }

    #[test]
    fn the_delay_is_fixed_per_trip_and_worse_with_a_new_driver_and_a_tired_bus() {
        let a = delay_of("2024-03-04", "5/1", 2, 80.0, 95.0);
        assert_eq!(a, delay_of("2024-03-04", "5/1", 2, 80.0, 95.0));
        assert!((-60.0..=240.0).contains(&a));
        assert!(delay_of("2024-03-04", "5/1", 2, 10.0, 40.0) > a + 60.0);
    }
}
