//! The player's own destination and waypoints: pins set on the city map (its pin tool, or a
//! right-click) that the navigator routes through - on a free drive to a place of the
//! player's choosing, on a duty as a diversion for the player's navigation only (the
//! timetable, the AI, the IBIS, the stop list, the delay and the trip report never hear of
//! it).
//!
//! * A pin snaps to the nearest drivable street lane ([`snap`]); with no road within
//!   [`SNAP_REACH`] there is no pin.
//! * On a free drive the first pin is the destination and later ones are vias before it, in
//!   the order they were set ([`add`]); on a duty every pin is a via, and after the last one
//!   the route goes back onto the trip's at the first lane a way reaches no earlier on it than
//!   the via ([`rejoin`]) - a stop the diversion leaves out stays left out.
//! * The route is made of legs, one search per pin from where the last one ended ([`plan`]):
//!   Dijkstra over the street lanes by metres driven, as the navigator's way back is. Off it,
//!   the navigator plans again from the bus through the pins still ahead.
//! * A pin counts as reached within [`REACHED`] of the bus, in order ([`Pins::pass`]); at the
//!   destination the drive is over after a moment.
//!
//! The navigator keeps the state ([`Pins`]) and draws the route; everything worked out is
//! here, in pure functions tested on made-up networks - the card that lists the pins on the
//! city map ([`draw_card`]) and the pins' markers too.

use glam::{DVec2, DVec3, Vec2};
use hashbrown::HashMap;
use omsi_sim::traffic::{Lane, LaneKind, Network};
use omsi_ui::paint::Align;
use omsi_ui::{Atlas, Color, Fonts, Painter, Rect, Weight};

use crate::nav_duty::{tr_with, Pen, EDGE, FIELD, HAIRLINE, ON_TIME_INK, SHEET, TEXT, TEXT_DIM, TEXT_FAINT, TEXT_SOFT};

/// No road this near a click: no pin (m).
pub(crate) const SNAP_REACH: f64 = 60.0;
/// A stop this near a pin gives it its name (m).
const NAME_REACH: f64 = 200.0;
/// A pin counts as reached with the bus this near (m).
pub(crate) const REACHED: f64 = 25.0;
/// How long "You have arrived" shows before the route and the pins are cleared (s).
pub(crate) const ARRIVED_FOR: f32 = 5.0;
/// How long a note shows (s).
pub(crate) const NOTE_FOR: f32 = 3.5;
/// Pins at most (the card has room for them in a 720p window's city map).
pub(crate) const MAX_PINS: usize = 10;
/// The other lanes of a pin's road - beside it, the other way - lie this near its point (m).
const ROAD_HALF: f64 = 12.0;
/// One leg is searched for this far (m of road: across any map).
const LEG_REACH: f32 = 60_000.0;
/// Turning back costs this much more (m), as in the navigator's way back.
const U_TURN: f32 = 400.0;

/// The pins' colour: a violet nothing else on the maps has (the stop signs are yellow, red
/// and blue, the traffic and the route blue, green, yellow and red).
pub(crate) const PIN: Color = Color::hex(0x9B5CF6);
pub(crate) const PIN_LIGHT: Color = Color::hex(0xC9B0FF);
const PIN_EDGE: Color = Color::rgba(22, 10, 44, 0.92);
/// The markers' sizes on the city map at scale 1: the destination's head, a via's disc.
pub(crate) const DEST_R: f32 = 11.0;
pub(crate) const VIA_R: f32 = 10.0;

/// A pin on the city map: where it stands (on the road it snapped to: world x, y), what it
/// is called, and its number among the pins made (a pin with no name of its own is "Point n").
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pin {
    pub at: DVec2,
    pub name: String,
    pub n: u32,
}

/// What a pin is to the route: a via (numbered from 1) or the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Via(usize),
    Destination,
}

impl Role {
    pub(crate) fn label(self) -> String {
        match self {
            Role::Via(n) => tr_with("Via %{n}", &[("n", n.to_string())]),
            Role::Destination => omsi_ui::tr("Destination").into_owned(),
        }
    }
}

// --- the pins in order ----------------------------------------------------------------------

/// Where a new pin goes in `list`: on a free drive the first is the destination and later
/// ones are vias in the order they are set, before it; on a diversion each goes last. None
/// when there are [`MAX_PINS`] already.
pub(crate) fn add(list: &mut Vec<Pin>, pin: Pin, diversion: bool) -> Option<usize> {
    if list.len() >= MAX_PINS {
        return None;
    }
    let k = if diversion || list.is_empty() { list.len() } else { list.len() - 1 };
    list.insert(k, pin);
    Some(k)
}

/// Pin `k` one place earlier on the route (`up`) or later; false when it is at that end.
pub(crate) fn shift(list: &mut [Pin], k: usize, up: bool) -> bool {
    let j = if up { k.checked_sub(1) } else { Some(k + 1) };
    match j {
        Some(j) if j < list.len() && k < list.len() => {
            list.swap(k, j);
            true
        }
        _ => false,
    }
}

// --- the road under a pin -------------------------------------------------------------------

/// A point put onto the road network: the lane, how far along it, the point on it and how far
/// the point given was from it (m).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Snap {
    pub lane: usize,
    pub s: f32,
    pub at: DVec2,
    pub dist: f64,
}

/// The point of a lane nearest `p` seen from above: how far along the lane, how far from
/// `p`, and the point (a pin is set on a map, it has no height).
pub(crate) fn nearest_2d(l: &Lane, p: DVec2) -> Option<(f32, f64, DVec2)> {
    let mut best: Option<(f32, f64, DVec2)> = None;
    for k in 0..l.points.len().saturating_sub(1) {
        let (a, b) = (l.points[k].truncate(), l.points[k + 1].truncate());
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
        let q = a + ab * t;
        let d = (q - p).length();
        if best.map(|b| d < b.1).unwrap_or(true) {
            best = Some((l.dist[k] + (l.dist[k + 1] - l.dist[k]) * t as f32, d, q));
        }
    }
    best
}

/// The nearest drivable street lane to `p` within `reach` metres (a drawn road before an
/// editor-only path beside it).
pub(crate) fn snap(net: &Network, p: DVec2, reach: f64) -> Option<Snap> {
    let mut best: Option<(Snap, f64)> = None;
    for i in crate::navigator::lanes_near(net, p, reach) {
        let l = &net.lanes[i];
        if l.kind != LaneKind::Street || l.points.len() < 2 {
            continue;
        }
        let Some((s, d, q)) = nearest_2d(l, p) else { continue };
        if d > reach {
            continue;
        }
        let score = d + if l.invisible { 15.0 } else { 0.0 };
        if best.map(|b| score < b.1).unwrap_or(true) {
            best = Some((Snap { lane: i, s, at: q, dist: d }, score));
        }
    }
    best.map(|b| b.0)
}

/// The lanes a pin at `at` is reached on, with how far along each it lies: the lane under it
/// and the other lanes of its road at that point - beside it, and the other way - so that a
/// route comes in on whichever is nearest and needs no turn round the block to approach from
/// one side.
pub(crate) fn targets(net: &Network, at: DVec2) -> Vec<(usize, f32)> {
    let Some(own) = snap(net, at, SNAP_REACH) else { return Vec::new() };
    let lane = &net.lanes[own.lane];
    let (q, h) = lane.at(own.s);
    let mut out = vec![(own.lane, own.s)];
    for i in crate::navigator::lanes_near(net, own.at, ROAD_HALF) {
        let m = &net.lanes[i];
        if i == own.lane || m.kind != LaneKind::Street || m.points.len() < 2 {
            continue;
        }
        let Some((s, d, _)) = nearest_2d(m, own.at) else { continue };
        if d > ROAD_HALF {
            continue;
        }
        let (mq, mh) = m.at(s);
        // (the same road: along it either way, not a bridge over it or a street across)
        let turn = crate::navigator::angle_diff(h as f64, mh as f64).abs();
        if (mq.z - q.z).abs() < 4.0 && !(30.0..=150.0).contains(&turn) {
            out.push((i, s));
        }
    }
    out
}

// --- the way through them -------------------------------------------------------------------

/// The way from `from` (a lane and how far along it) to the nearest of `targets` (lanes and
/// how far along each), as metres driven: the lanes after `from`'s up to and with the
/// target's, and which target - no lanes when one lies ahead on `from`'s own lane. Dijkstra
/// over the street lanes; turning back costs [`U_TURN`] more. The lane it starts on is no node
/// of the search, so a way round the block back onto it is found too.
pub(crate) fn leg(net: &Network, from: (usize, f32), targets: &[(usize, f32)], max: f32) -> Option<(Vec<usize>, usize)> {
    use std::cmp::Ordering;
    use std::collections::BinaryHeap;
    let ahead = targets.iter().enumerate().filter(|(_, t)| t.0 == from.0 && t.1 + 1.0 >= from.1).min_by(|a, b| a.1 .1.total_cmp(&b.1 .1));
    if let Some((k, _)) = ahead {
        return Some((Vec::new(), k));
    }
    let mut on: HashMap<usize, Vec<usize>> = HashMap::new();
    for (k, t) in targets.iter().enumerate() {
        on.entry(t.0).or_default().push(k);
    }
    #[derive(PartialEq)]
    struct Node(f32, usize);
    impl Eq for Node {}
    impl Ord for Node {
        fn cmp(&self, o: &Self) -> Ordering {
            o.0.total_cmp(&self.0)
        }
    }
    impl PartialOrd for Node {
        fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
            Some(self.cmp(o))
        }
    }
    let turn_cost = |a: &Lane, b: &Lane| if crate::navigator::angle_diff(a.end_heading() as f64, b.start_heading() as f64).abs() > 150.0 { U_TURN } else { 0.0 };
    // (metres to the start of each lane; None: entered straight from where the leg begins)
    let mut dist: HashMap<usize, f32> = HashMap::new();
    let mut parent: HashMap<usize, Option<usize>> = HashMap::new();
    let mut heap = BinaryHeap::new();
    let first = net.lanes.get(from.0)?;
    let base = (first.length() - from.1).max(0.0);
    for &n in &first.next {
        let Some(nl) = net.lanes.get(n).filter(|l| l.kind == LaneKind::Street) else { continue };
        let c = base + turn_cost(first, nl);
        if c < dist.get(&n).copied().unwrap_or(f32::INFINITY) {
            dist.insert(n, c);
            parent.insert(n, None);
            heap.push(Node(c, n));
        }
    }
    let mut best: Option<(f32, usize, usize)> = None;
    while let Some(Node(cost, lane)) = heap.pop() {
        if cost > dist.get(&lane).copied().unwrap_or(f32::INFINITY) {
            continue;
        }
        if best.is_some_and(|b| cost >= b.0) || cost > max {
            break;
        }
        if let Some(ks) = on.get(&lane) {
            for &k in ks {
                let total = cost + targets[k].1;
                if best.map(|b| total < b.0).unwrap_or(true) {
                    best = Some((total, lane, k));
                }
            }
        }
        let Some(l) = net.lanes.get(lane) else { continue };
        for &n in &l.next {
            let Some(nl) = net.lanes.get(n).filter(|l| l.kind == LaneKind::Street) else { continue };
            let c = cost + l.length() + turn_cost(l, nl);
            if c < dist.get(&n).copied().unwrap_or(f32::INFINITY) {
                dist.insert(n, c);
                parent.insert(n, Some(lane));
                heap.push(Node(c, n));
            }
        }
    }
    let (_, lane, k) = best?;
    let mut path = vec![lane];
    let mut c = lane;
    while let Some(Some(p)) = parent.get(&c) {
        path.push(*p);
        c = *p;
        if path.len() > net.lanes.len() {
            return None;
        }
    }
    path.reverse();
    Some((path, k))
}

/// Where a diversion goes back onto the trip's route `rest` (its lanes ahead) after its last
/// via, from `from` there: no earlier on the route than the lane of it nearest the via at
/// `via` (the stops the diversion leaves out are not gone back to), at the first lane from
/// there a way reaches. The lanes to it (the last is the route's) and its index in `rest`;
/// no lanes when the via lies on the route itself.
pub(crate) fn rejoin(net: &Network, from: (usize, f32), via: DVec2, rest: &[usize]) -> Option<(Vec<usize>, usize)> {
    let lo = rest
        .iter()
        .enumerate()
        .take(6000)
        .filter_map(|(k, &l)| net.lanes.get(l).and_then(|lane| nearest_2d(lane, via)).map(|n| (k, n.1)))
        .min_by(|a, b| a.1.total_cmp(&b.1))?
        .0;
    if let Some(j) = rest[lo..].iter().position(|&l| l == from.0) {
        return Some((Vec::new(), lo + j));
    }
    let mut first: HashMap<usize, usize> = HashMap::new();
    for (k, &l) in rest.iter().enumerate().skip(lo) {
        first.entry(l).or_insert(k);
    }
    let mut targets: Vec<(usize, f32)> = first.keys().map(|&l| (l, 0.0)).collect();
    targets.sort_unstable_by_key(|t| first[&t.0]);
    let (path, k) = leg(net, from, &targets, LEG_REACH)?;
    Some((path, first[&targets[k].0]))
}

/// A route through the pins: its lanes from the bus's on, how far along the first the bus
/// is, where on it each pin lies (an index into `lanes` and metres along that lane; None for
/// a pin no way reaches, and for those after it), how far along its last lane it ends (a
/// destination; None when it goes on along the trip's route), and the index in `lanes` of
/// the trip's lane it rejoins.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Plan {
    pub lanes: Vec<usize>,
    pub s0: f32,
    pub marks: Vec<Option<(usize, f32)>>,
    pub end: Option<f32>,
    pub join: Option<usize>,
}

/// The route from `start` (the lane under the bus going its way, and how far along it)
/// through the pins at `pins` in order - one leg each, from where the last one ended - and,
/// for a diversion, back onto the trip's route `rest`. It ends at the last pin a way reaches.
pub(crate) fn plan(net: &Network, start: (usize, f32), pins: &[DVec2], rest: Option<&[usize]>) -> Plan {
    let mut lanes = vec![start.0];
    let mut cur = start;
    let mut marks = Vec::with_capacity(pins.len());
    let mut reached_all = true;
    for &p in pins {
        if !reached_all {
            marks.push(None);
            continue;
        }
        let t = targets(net, p);
        // (a pin the network has no road for any more is reached by nothing)
        let found = if t.is_empty() { None } else { leg(net, cur, &t, LEG_REACH) };
        match found {
            Some((path, k)) => {
                lanes.extend(path);
                cur = t[k];
                marks.push(Some((lanes.len() - 1, cur.1)));
            }
            None => {
                reached_all = false;
                marks.push(None);
            }
        }
    }
    if marks.first().is_some_and(|m| m.is_none()) {
        return Plan { marks, s0: start.1, ..Plan::default() };
    }
    let mut out = Plan { s0: start.1, end: Some(cur.1), ..Plan::default() };
    if let (Some(rest), true, Some(via)) = (rest.filter(|r| !r.is_empty()), reached_all, pins.last()) {
        if let Some((path, j)) = rejoin(net, cur, *via, rest) {
            lanes.extend(path);
            out.join = Some(lanes.len() - 1);
            lanes.extend_from_slice(&rest[j + 1..]);
            out.end = None;
        }
    }
    out.lanes = lanes;
    out.marks = marks;
    out
}

/// How far along `lanes` it is from where the bus is (lane `progress`, `s` metres along it)
/// to `mark` (an index into `lanes` and metres along that lane), in metres; 0 behind.
pub(crate) fn along(net: &Network, lanes: &[usize], progress: usize, s: f32, mark: (usize, f32)) -> Option<f64> {
    let (idx, at) = mark;
    if idx < progress {
        return Some(0.0);
    }
    let mut d = -(s as f64);
    for &l in lanes.get(progress..idx)? {
        d += net.lanes.get(l)?.length() as f64;
    }
    Some((d + at as f64).max(0.0))
}

/// Where the bus at `bus`, heading `heading`, is on `lanes`: the lane under it running its
/// way (within 16 m and 100 degrees, as the navigator finds it on a route) and how far along
/// it; the first such lane when the route passes twice.
pub(crate) fn locate(net: &Network, lanes: &[usize], bus: DVec3, heading: f64) -> Option<(usize, f32)> {
    let mut best: Option<(usize, f32, f64)> = None;
    for (k, &l) in lanes.iter().enumerate() {
        let Some(lane) = net.lanes.get(l) else { continue };
        let Some((s, d)) = lane.nearest_point(bus) else { continue };
        if d > 16.0 || crate::navigator::angle_diff(heading, lane.at(s).1 as f64).abs() > 100.0 {
            continue;
        }
        if best.map(|b| d < b.2 - 0.5).unwrap_or(true) {
            best = Some((k, s, d));
        }
    }
    best.map(|b| (b.0, b.1))
}

/// The time to drive `dist` metres at the bus's speed over the last while (`speed`, m/s;
/// walking pace at least, as the navigator's time to the next stop), s.
pub(crate) fn eta(dist: f64, speed: f32) -> f64 {
    dist / speed.max(5.0) as f64
}

/// A distance as the navigator says it: metres to the ten, kilometres to the tenth.
pub(crate) fn distance_text(d: f64) -> String {
    if d >= 1000.0 {
        format!("{:.1} km", d / 1000.0)
    } else {
        format!("{:.0} m", (d / 10.0).round() * 10.0)
    }
}

/// A time to go as the navigator says it.
pub(crate) fn eta_text(secs: f64) -> String {
    if secs < 60.0 {
        "<1 min".to_string()
    } else {
        format!("{:.0} min", (secs / 60.0).round())
    }
}

/// A pin's name: the stop nearest it within [`NAME_REACH`] (`stops`: the timetable's, where
/// the map has them), else the street the map names there, else "Point n".
pub(crate) fn name(at: DVec2, stops: &[(DVec2, String)], street: Option<&str>, n: u32) -> String {
    stops
        .iter()
        .filter(|(_, name)| !name.trim().is_empty())
        .map(|(p, name)| ((*p - at).length(), name))
        .filter(|(d, _)| *d < NAME_REACH)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, name)| name.trim().to_string())
        .or_else(|| street.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string))
        .unwrap_or_else(|| tr_with("Point %{n}", &[("n", n.to_string())]))
}

// --- the state ------------------------------------------------------------------------------

/// What the pins have to say for a moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Note {
    /// Via n was reached.
    Reached(usize),
    Arrived,
    /// No road near where the map was clicked.
    NoRoad,
    /// No way leads to a pin.
    NoWay,
    /// [`MAX_PINS`] already.
    Full,
}

impl Note {
    pub(crate) fn text(self) -> String {
        match self {
            Note::Reached(n) => tr_with("Via %{n} reached", &[("n", n.to_string())]),
            Note::Arrived => omsi_ui::tr("You have arrived").into_owned(),
            Note::NoRoad => omsi_ui::tr("No road within 60 m").into_owned(),
            Note::NoWay => omsi_ui::tr("No way found to this point").into_owned(),
            Note::Full => omsi_ui::tr("Ten points at most").into_owned(),
        }
    }

    /// Good news (green) or a warning (amber).
    pub(crate) fn good(self) -> bool {
        matches!(self, Note::Reached(_) | Note::Arrived)
    }

    /// For the phone and tablet.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Note::Reached(_) => "reached",
            Note::Arrived => "arrived",
            Note::NoRoad => "no_road",
            Note::NoWay => "no_way",
            Note::Full => "full",
        }
    }
}

/// What the city map asks of the pins; done in the next frame, where the road network is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Op {
    /// A pin where the map was clicked.
    Add(DVec2),
    /// Pin k dragged to a place, from where it was (it goes back there with no road near).
    Move(usize, DVec2, DVec2),
    Remove(usize),
    /// Pin k one place earlier (true) or later on the route.
    Shift(usize, bool),
    Clear,
}

/// The player's pins and what the navigator made of them.
#[derive(Debug, Default)]
pub(crate) struct Pins {
    /// In the order the route takes them; on a free drive the last is the destination.
    pub list: Vec<Pin>,
    /// The trip they divert from (a duty); None: a free drive's destination.
    pub trip: Option<String>,
    /// Asked by the city map, not done yet.
    pub ops: Vec<Op>,
    /// The route has to be planned again (the pins changed).
    pub dirty: bool,
    /// Where each pin lies on the navigator's route, as the last plan made it.
    pub marks: Vec<Option<(usize, f32)>>,
    /// How far along the route each pin is (m), now and then.
    pub dist: Vec<Option<f64>>,
    /// The trip's route ahead when the diversion began: it goes back onto this.
    pub rest: Vec<usize>,
    /// Where on the navigator's route the diversion rejoined the trip's.
    pub join: Option<usize>,
    /// A note and the seconds it still shows.
    pub note: Option<(Note, f32)>,
    /// At the destination: the seconds until the route and the pins are cleared.
    pub arrived: Option<f32>,
    /// Vias reached on this route (the ones left keep their numbers).
    pub passed: usize,
    /// Pins made, for the names of those with none ("Point 3").
    pub made: u32,
}

impl Pins {
    /// The pins make the route the navigator follows.
    pub(crate) fn steers(&self) -> bool {
        !self.list.is_empty() && self.arrived.is_none()
    }

    pub(crate) fn diversion(&self) -> bool {
        self.trip.is_some()
    }

    /// A diversion of the trip `key` is under way.
    pub(crate) fn diverts(&self, key: &str) -> bool {
        !self.list.is_empty() && self.trip.as_deref() == Some(key)
    }

    pub(crate) fn role(&self, k: usize) -> Role {
        if !self.diversion() && k + 1 == self.list.len() {
            Role::Destination
        } else {
            Role::Via(self.passed + k + 1)
        }
    }

    pub(crate) fn say(&mut self, note: Note) {
        self.note = Some((note, if note == Note::Arrived { ARRIVED_FOR } else { NOTE_FOR }));
    }

    /// Time passes for the note.
    pub(crate) fn tick(&mut self, dt: f32) {
        if let Some((_, t)) = self.note.as_mut() {
            *t -= dt;
        }
        self.note = self.note.filter(|n| n.1 > 0.0);
    }

    /// The pins gone (cleared, arrived, the trip over): the note and the count of names stay.
    pub(crate) fn finish(&mut self) {
        *self = Pins { note: self.note, made: self.made, ..Pins::default() };
    }

    /// The bus at `bus`: the next pin, when it is reached, is dropped (a via: its number is
    /// said) or ends the drive (the destination: "You have arrived", and after
    /// [`ARRIVED_FOR`] the route goes). Pins count in order only.
    pub(crate) fn pass(&mut self, bus: DVec3) -> Option<Note> {
        if self.arrived.is_some() {
            return None;
        }
        let first = self.list.first()?;
        if (bus.truncate() - first.at).length() >= REACHED {
            return None;
        }
        if self.role(0) == Role::Destination {
            self.arrived = Some(ARRIVED_FOR);
            self.say(Note::Arrived);
            return Some(Note::Arrived);
        }
        self.passed += 1;
        let note = Note::Reached(self.passed);
        self.list.remove(0);
        if !self.marks.is_empty() {
            self.marks.remove(0);
        }
        if !self.dist.is_empty() {
            self.dist.remove(0);
        }
        self.say(note);
        Some(note)
    }
}

// --- the markers ------------------------------------------------------------------------------

/// The destination's marker standing on `at` (pixels): a pin in the pins' violet with a
/// chequered flag in its head and its point on the place - green with a tick once there.
/// `r` is its head's radius.
pub(crate) fn draw_destination(p: &mut Painter, atlas: &mut Atlas, at: Vec2, r: f32, arrived: bool, alpha: f32) {
    let fill = if arrived { ON_TIME_INK } else { PIN };
    // a shadow on the ground under the point
    p.circle(at, r * 0.28, Color::rgba(0, 0, 0, 0.45 * alpha));
    // (a disc of radius `r` 2.1 radii over the point, and the two lines from the point that
    // touch it)
    let drop = |r: f32, tip: f32| -> Vec<Vec2> {
        let c = Vec2::new(at.x, at.y - tip - r * 2.1);
        let g = (1.0 / 2.1f32).acos();
        let (a0, a1) = (std::f32::consts::FRAC_PI_2 + g, std::f32::consts::FRAC_PI_2 - g + std::f32::consts::TAU);
        let mut pts = vec![Vec2::new(at.x, at.y - tip)];
        let n = 20;
        for k in 0..=n {
            let a = a0 + (a1 - a0) * k as f32 / n as f32;
            pts.push(c + Vec2::new(a.cos(), a.sin()) * r);
        }
        pts
    };
    let edge = (r * 0.14).max(1.0);
    p.convex(&drop(r + edge, 0.0), PIN_EDGE.alpha(alpha));
    p.convex(&drop(r, edge * 1.4), fill.alpha(alpha));
    let head = destination_head(at, r);
    p.circle(head, r * 0.64, Color::WHITE.alpha(alpha));
    p.icon(atlas, if arrived { "check" } else { "sports_score" }, head, r * 1.05, fill.darken(0.35).alpha(alpha));
}

/// Where the head of the destination's marker on `at` with a head of radius `r` is (a press
/// there takes the pin).
pub(crate) fn destination_head(at: Vec2, r: f32) -> Vec2 {
    Vec2::new(at.x, at.y - (r * 0.14).max(1.0) * 1.4 - r * 2.1)
}

/// A via's marker on `at`: a violet disc with a white ring and its number.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_via(p: &mut Painter, atlas: &mut Atlas, fonts: &Fonts, at: Vec2, n: usize, r: f32, alpha: f32) {
    p.circle(at, r + (r * 0.16).max(1.0), PIN_EDGE.alpha(alpha));
    p.circle(at, r, Color::WHITE.alpha(alpha));
    p.circle(at, r * 0.8, PIN.alpha(alpha));
    let t = n.to_string();
    let px = r * if t.len() > 1 { 0.95 } else { 1.15 };
    p.text_in(atlas, fonts, &t, px, Weight::Black, Rect::new(at.x - r, at.y - r, r * 2.0, r * 2.0), Align::Center, Color::WHITE.alpha(alpha));
}

// --- the card on the city map -----------------------------------------------------------------

/// A pin as the card lists it: what it is, its name, how far along the route and how long
/// to it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CardRow {
    pub role: Role,
    pub name: String,
    pub dist: Option<f64>,
    pub eta: Option<f64>,
}

/// The card listing the pins: a diversion's or a destination's, its pins, to the
/// destination how far, how long and when there (s of the day), whether the bus is there,
/// and whether the pin tool is on (with no pins it says how to set one).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Card {
    pub diversion: bool,
    pub rows: Vec<CardRow>,
    pub total: Option<(f64, f64, f64)>,
    pub arrived: bool,
    pub tool: bool,
}

/// What a press on the card does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CardHit {
    Remove(usize),
    Up(usize),
    Down(usize),
    Clear,
}

const CARD_HEAD: f32 = 60.0;
const CARD_ROW: f32 = 48.0;
const CARD_HINT: f32 = 62.0;
const CARD_MORE: f32 = 22.0;
const CARD_FOOT: f32 = 50.0;
const CARD_PAD: f32 = 14.0;

/// Is there a card to show?
pub(crate) fn card_shown(c: &Card) -> bool {
    !c.rows.is_empty() || c.tool
}

/// How many of the card's pins fit in `room` pixels at scale `s` (the others are counted).
fn rows_fitting(c: &Card, room: f32, s: f32) -> usize {
    let fixed = CARD_HEAD + CARD_FOOT + 6.0;
    let all = c.rows.len();
    if (fixed + all as f32 * CARD_ROW) * s <= room {
        return all;
    }
    (((room / s - fixed - CARD_MORE) / CARD_ROW).floor().max(1.0) as usize).min(all)
}

/// The card's height at scale `s` within `room` pixels.
pub(crate) fn card_height(c: &Card, s: f32, room: f32) -> f32 {
    if c.rows.is_empty() {
        return (CARD_HEAD + CARD_HINT + 6.0) * s;
    }
    let n = rows_fitting(c, room, s);
    let more = if n < c.rows.len() { CARD_MORE } else { 0.0 };
    (CARD_HEAD + n as f32 * CARD_ROW + more + CARD_FOOT + 6.0) * s
}

/// The card's subtitle: to the destination how far, how long and when there; once there,
/// so; on a diversion that it is the player's own.
fn card_sub(c: &Card) -> (String, Color) {
    if c.arrived {
        return (omsi_ui::tr("You have arrived").into_owned(), ON_TIME_INK);
    }
    if c.diversion {
        return (omsi_ui::tr("For your navigation only").into_owned(), TEXT_DIM);
    }
    match c.total {
        Some((d, eta, at)) => (format!("{}  ·  {}  ·  {}", distance_text(d), eta_text(eta), tr_with("arrives %{time}", &[("time", crate::nav_duty::hhmm(at))])), TEXT_SOFT),
        None if c.rows.is_empty() => (omsi_ui::tr("Click the map where you want to go.").into_owned(), TEXT_SOFT),
        None => (String::new(), TEXT_DIM),
    }
}

/// Draw the card into `r` (its height [`card_height`]) at scale `s`; returns where a press
/// does what.
pub(crate) fn draw_card(pen: &mut Pen, c: &Card, r: Rect, s: f32) -> Vec<(Rect, CardHit)> {
    let mut hits = Vec::new();
    pen.p.shadow(r, 12.0 * s, 18.0 * s, Color::rgba(0, 0, 0, 0.45));
    pen.p.rounded(r, 12.0 * s, SHEET.alpha(0.96));
    pen.p.rounded_border(r, 12.0 * s, 1.0, EDGE);
    let pad = CARD_PAD * s;
    let (x0, x1) = (r.x + pad, r.right() - pad);
    // the head: the badge, the title and what it comes to
    let badge = Vec2::new(x0 + 14.0 * s, r.y + 30.0 * s);
    pen.p.circle(badge, 15.0 * s, PIN.alpha(0.2));
    pen.p.icon(pen.atlas, if c.diversion { "alt_route" } else { "flag" }, badge, 18.0 * s, PIN_LIGHT);
    let tx = x0 + 38.0 * s;
    pen.text_in(if c.diversion { "Diversion" } else { "Destination" }, 16.0 * s, Weight::Bold, Rect::new(tx, r.y + 11.0 * s, (x1 - tx).max(0.0), 20.0 * s), Align::Left, TEXT);
    let (sub, ink) = card_sub(c);
    pen.text_in(&sub, 11.5 * s, Weight::Medium, Rect::new(tx, r.y + 33.0 * s, (x1 - tx).max(0.0), 16.0 * s), Align::Left, ink);
    let mut y = r.y + CARD_HEAD * s;
    pen.p.rect(Rect::new(r.x, y - 1.0, r.w, 1.0), HAIRLINE);
    if c.rows.is_empty() {
        // (the tool on and nothing set yet: how it is done)
        let what = if c.diversion { "Click the map to add a via for your navigation." } else { "Click the map where you want to go." };
        pen.text_in(what, 12.5 * s, Weight::Medium, Rect::new(x0, y + 10.0 * s, x1 - x0, 18.0 * s), Align::Left, TEXT_SOFT);
        pen.text_in("A right-click on the map works without the tool.", 11.5 * s, Weight::Regular, Rect::new(x0, y + 32.0 * s, x1 - x0, 16.0 * s), Align::Left, TEXT_DIM);
        return hits;
    }
    let shown = rows_fitting(c, r.h + 1.0, s);
    let n = c.rows.len();
    let bs = 26.0 * s;
    for (k, row) in c.rows.iter().enumerate().take(shown) {
        let rr = Rect::new(r.x, y, r.w, CARD_ROW * s);
        let cy = rr.center().y;
        // its marker, as on the map
        let at = Vec2::new(x0 + 14.0 * s, cy);
        match row.role {
            Role::Via(v) => draw_via(pen.p, pen.atlas, pen.fonts, at, v, 10.0 * s, 1.0),
            Role::Destination => {
                pen.p.circle(at, 11.5 * s, PIN_EDGE);
                pen.p.circle(at, 10.0 * s, if c.arrived { ON_TIME_INK } else { PIN });
                pen.p.icon(pen.atlas, if c.arrived { "check" } else { "sports_score" }, at, 13.0 * s, Color::WHITE);
            }
        }
        // the buttons on the right: earlier, later, away
        let mut bx = x1 - bs;
        for (icon, hit, live) in [("close", CardHit::Remove(k), true), ("keyboard_arrow_down", CardHit::Down(k), k + 1 < n), ("keyboard_arrow_up", CardHit::Up(k), k > 0)] {
            let b = Rect::new(bx, cy - bs * 0.5, bs, bs);
            if live {
                pen.p.rounded(b, 6.0 * s, FIELD.alpha(0.75));
                hits.push((b, hit));
            }
            pen.p.icon(pen.atlas, icon, b.center(), 17.0 * s, if live { TEXT_SOFT } else { TEXT_FAINT.alpha(0.6) });
            bx -= bs + 4.0 * s;
        }
        let nx = x0 + 34.0 * s;
        let room = (bx + bs - 8.0 * s - nx).max(0.0);
        pen.text_in(&row.name, 13.5 * s, Weight::Bold, Rect::new(nx, rr.y + 7.0 * s, room, 18.0 * s), Align::Left, TEXT);
        let mut parts = vec![row.role.label()];
        match row.dist {
            Some(d) => {
                parts.push(distance_text(d));
                if let Some(e) = row.eta {
                    parts.push(eta_text(e));
                }
            }
            None => parts.push("–".to_string()),
        }
        pen.text_in(&parts.join("  ·  "), 11.5 * s, Weight::Medium, Rect::new(nx, rr.y + 26.0 * s, room, 15.0 * s), Align::Left, TEXT_DIM);
        y += CARD_ROW * s;
        if k + 1 < shown {
            pen.p.rect(Rect::new(nx, y - 1.0, x1 - nx, 1.0), HAIRLINE);
        }
    }
    if shown < n {
        pen.text_in(&tr_with("%{n} more", &[("n", (n - shown).to_string())]), 11.5 * s, Weight::Medium, Rect::new(x0 + 34.0 * s, y, x1 - x0, CARD_MORE * s), Align::Left, TEXT_DIM);
        y += CARD_MORE * s;
    }
    // the foot: clearing them all
    let b = Rect::new(x0, y + 9.0 * s, x1 - x0, 32.0 * s);
    pen.p.rounded(b, 8.0 * s, FIELD);
    let label = omsi_ui::tr(if c.diversion { "Clear diversion" } else { "Clear all" }).into_owned();
    let lw = pen.width(&label, 13.0 * s, Weight::Bold);
    let ix = b.center().x - (lw + 22.0 * s) * 0.5;
    pen.p.icon(pen.atlas, "delete", Vec2::new(ix + 8.0 * s, b.center().y), 16.0 * s, TEXT_SOFT);
    pen.text_in(&label, 13.0 * s, Weight::Bold, Rect::new(ix + 22.0 * s, b.y, lw + 4.0, b.h), Align::Left, TEXT);
    hits.push((b, CardHit::Clear));
    hits
}

/// A note in a dark pill centred on `at` (the city map's, under its header).
pub(crate) fn draw_note(pen: &mut Pen, note: Note, at: Vec2, s: f32, alpha: f32) {
    let text = note.text();
    let px = 13.0 * s;
    let w = pen.width(&text, px, Weight::Bold) + 46.0 * s;
    let r = Rect::new(at.x - w * 0.5, at.y - 16.0 * s, w, 32.0 * s);
    pen.p.shadow(r, 16.0 * s, 12.0 * s, Color::rgba(0, 0, 0, 0.35 * alpha));
    pen.p.rounded(r, 16.0 * s, Color::rgba(14, 18, 28, 0.94 * alpha));
    let ink = if note.good() { ON_TIME_INK } else { crate::nav_duty::NOW };
    let icon = match note {
        Note::Arrived => "sports_score",
        Note::Reached(_) => "check_circle",
        _ => "warning",
    };
    pen.p.icon(pen.atlas, icon, Vec2::new(r.x + 20.0 * s, r.center().y), 17.0 * s, ink.alpha(alpha));
    pen.text_in(&text, px, Weight::Bold, Rect::new(r.x + 34.0 * s, r.y, r.w - 44.0 * s, r.h), Align::Left, TEXT.alpha(alpha));
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use omsi_sim::traffic::LaneBuilder;

    pub(crate) fn lane(a: (f64, f64), b: (f64, f64)) -> Lane {
        let n = (((b.0 - a.0).hypot(b.1 - a.1)) / 10.0).ceil().max(1.0) as usize;
        let pts: Vec<DVec3> = (0..=n).map(|i| DVec3::new(a.0, a.1, 0.0).lerp(DVec3::new(b.0, b.1, 0.0), i as f64 / n as f64)).collect();
        LaneBuilder::polyline(pts, LaneKind::Street, 3.0)
    }

    /// A grid of two-way streets 200 m apart, `n` x `n` blocks, every corner joined every way
    /// (turning back too): lane pairs per side of each block. Lanes run 1.5 m right of the
    /// street's middle, as a road's lanes do.
    pub(crate) fn grid(n: i32) -> Network {
        let mut lanes = Vec::new();
        let step = 200.0;
        for i in 0..=n {
            for j in 0..n {
                let x = i as f64 * step;
                let (y0, y1) = (j as f64 * step, (j + 1) as f64 * step);
                // north on the east side of the street, south on the west
                lanes.push(lane((x + 1.5, y0), (x + 1.5, y1)));
                lanes.push(lane((x - 1.5, y1), (x - 1.5, y0)));
                let y = i as f64 * step;
                let (x0, x1) = (j as f64 * step, (j + 1) as f64 * step);
                lanes.push(lane((x0, y - 1.5), (x1, y - 1.5)));
                lanes.push(lane((x1, y + 1.5), (x0, y + 1.5)));
            }
        }
        let mut net = Network { lanes, ..Default::default() };
        net.link(1.5);
        // (square corners: every lane ending at a corner goes on into every lane starting there)
        let ends: Vec<DVec2> = net.lanes.iter().map(|l| l.end().truncate()).collect();
        let starts: Vec<DVec2> = net.lanes.iter().map(|l| l.start().truncate()).collect();
        for a in 0..net.lanes.len() {
            let corner = (ends[a] / step).round() * step;
            net.lanes[a].next = (0..net.lanes.len()).filter(|&b| b != a && ((starts[b] / step).round() * step - corner).length() < 1.0 && (starts[b] - corner).length() < 4.0).collect();
        }
        net.build_grid();
        net
    }

    /// The lane of `net` from `a` to `b` (its ends within a few metres).
    pub(crate) fn find(net: &Network, a: (f64, f64), b: (f64, f64)) -> usize {
        net.lanes.iter().position(|l| (l.start().truncate() - DVec2::new(a.0, a.1)).length() < 3.0 && (l.end().truncate() - DVec2::new(b.0, b.1)).length() < 3.0).unwrap_or_else(|| panic!("no lane {a:?} -> {b:?}"))
    }

    #[test]
    fn a_pin_snaps_to_the_nearest_road_or_is_no_pin() {
        let net = grid(2);
        // 20 m east of the street at x = 200, half-way up the first block
        let s = snap(&net, DVec2::new(220.0, 100.0), SNAP_REACH).expect("a road within 60 m");
        assert!((s.at.x - 201.5).abs() < 0.01 && (s.at.y - 100.0).abs() < 0.01, "{s:?}");
        assert!((s.dist - 18.5).abs() < 0.01);
        assert_eq!(s.lane, find(&net, (201.5, 0.0), (201.5, 200.0)));
        assert!((s.s - 100.0).abs() < 0.1);
        // the middle of a block is 100 m from every street
        assert_eq!(snap(&net, DVec2::new(100.0, 100.0), SNAP_REACH), None);
        // an editor-only path a little nearer gives way to the drawn road
        let mut net = net;
        let i = net.lanes.len();
        let mut hidden = lane((210.0, 0.0), (210.0, 200.0));
        hidden.invisible = true;
        net.lanes.push(hidden);
        net.build_grid();
        assert_ne!(snap(&net, DVec2::new(212.0, 100.0), SNAP_REACH).unwrap().lane, i);
        assert_eq!(snap(&net, DVec2::new(211.0, 100.0), 5.0).unwrap().lane, i, "nothing else that near");
    }

    #[test]
    fn a_pin_is_reached_on_either_side_of_its_road() {
        let net = grid(2);
        let t = targets(&net, DVec2::new(201.5, 100.0));
        let north = find(&net, (201.5, 0.0), (201.5, 200.0));
        let south = find(&net, (198.5, 200.0), (198.5, 0.0));
        assert!(t.iter().any(|x| x.0 == north && (x.1 - 100.0).abs() < 0.1), "{t:?}");
        assert!(t.iter().any(|x| x.0 == south && (x.1 - 100.0).abs() < 0.1), "{t:?}");
        // (the streets across at the corners are no way to it)
        assert_eq!(t.len(), 2, "{t:?}");
    }

    #[test]
    fn a_leg_goes_round_the_block_to_a_pin_behind() {
        let net = grid(2);
        let north = find(&net, (201.5, 0.0), (201.5, 200.0));
        // ahead on the same lane: no search
        assert_eq!(leg(&net, (north, 20.0), &[(north, 150.0)], LEG_REACH), Some((Vec::new(), 0)));
        // behind on it: round the block back onto it (no node of the search, still found)
        let (path, k) = leg(&net, (north, 150.0), &[(north, 20.0)], LEG_REACH).expect("a way round");
        assert_eq!((k, path.last()), (0, Some(&north)), "{path:?}");
        assert!(path.len() >= 4, "{path:?}");
        // the nearer of two targets wins (the other side of the road, by turning)
        let south = find(&net, (198.5, 200.0), (198.5, 0.0));
        let (path, k) = leg(&net, (north, 150.0), &[(north, 20.0), (south, 100.0)], LEG_REACH).unwrap();
        assert_eq!(k, 1, "{path:?}");
        assert_eq!(path.last(), Some(&south));
        // nothing reaches a lane on its own
        let mut net2 = grid(2);
        net2.lanes.push(lane((5000.0, 0.0), (5100.0, 0.0)));
        let far = net2.lanes.len() - 1;
        assert_eq!(leg(&net2, (north, 0.0), &[(far, 10.0)], LEG_REACH), None);
    }

    #[test]
    fn the_legs_chain_through_the_vias_to_the_destination() {
        let net = grid(3);
        let start = find(&net, (1.5, 0.0), (1.5, 200.0));
        // a via on the street east, one on the westbound side two blocks up (reached on the
        // eastbound one), then a destination far north-east
        let pins = [DVec2::new(100.0, 198.5), DVec2::new(300.0, 401.5), DVec2::new(598.5, 500.0)];
        let p = plan(&net, (start, 10.0), &pins, None);
        assert_eq!(p.lanes.first(), Some(&start));
        assert_eq!(p.s0, 10.0);
        assert_eq!(p.marks.len(), 3);
        // the lanes follow each other, and every pin lies on the route in order
        for w in p.lanes.windows(2) {
            assert!(net.lanes[w[0]].next.contains(&w[1]), "{w:?} not linked in {:?}", p.lanes);
        }
        let idx: Vec<usize> = p.marks.iter().map(|m| m.unwrap().0).collect();
        assert!(idx.windows(2).all(|w| w[0] <= w[1]), "{idx:?}");
        for (k, m) in p.marks.iter().enumerate() {
            let (i, s) = m.unwrap();
            let at = net.lanes[p.lanes[i]].at(s).0.truncate();
            assert!((at - pins[k]).length() < 5.0, "pin {k} at {at} not {}", pins[k]);
        }
        // a free drive's route ends at the destination
        assert_eq!(p.end, Some(p.marks[2].unwrap().1));
        assert_eq!(p.lanes.len() - 1, p.marks[2].unwrap().0);
        assert_eq!(p.join, None);
        // the distances along it grow, the first is within the block
        let d: Vec<f64> = p.marks.iter().map(|m| along(&net, &p.lanes, 0, p.s0, m.unwrap()).unwrap()).collect();
        assert!(d.windows(2).all(|w| w[0] < w[1]), "{d:?}");
        assert!((d[0] - (190.0 + 100.0 + 1.5)).abs() < 5.0, "{d:?}");
    }

    #[test]
    fn a_pin_no_way_reaches_ends_the_route_before_it() {
        let mut net = grid(2);
        net.lanes.push(lane((5000.0, 0.0), (5100.0, 0.0)));
        net.build_grid();
        let start = find(&net, (1.5, 0.0), (1.5, 200.0));
        let p = plan(&net, (start, 0.0), &[DVec2::new(100.0, 198.5), DVec2::new(5050.0, 0.0), DVec2::new(398.5, 100.0)], None);
        assert!(p.marks[0].is_some() && p.marks[1].is_none() && p.marks[2].is_none(), "{:?}", p.marks);
        assert_eq!(p.lanes.len() - 1, p.marks[0].unwrap().0, "the route ends at the last pin it reaches");
        // none reached: no route at all
        let p = plan(&net, (start, 0.0), &[DVec2::new(5050.0, 0.0)], None);
        assert!(p.lanes.is_empty() && p.marks == vec![None]);
    }

    /// The trip's route runs north up x = 0 and on north; the diversion's via is on the street
    /// one block east. It goes back onto the route no earlier than abreast of the via - not
    /// back down to the stretch it left out - and goes on along the trip's lanes from there.
    #[test]
    fn a_diversion_rejoins_the_route_after_its_last_via() {
        let net = grid(3);
        let route: Vec<usize> = (0..3).map(|j| find(&net, (1.5, j as f64 * 200.0), (1.5, (j + 1) as f64 * 200.0))).collect();
        // the bus on the first lane; the via half-way up the second block's eastern street
        let via = DVec2::new(201.5, 300.0);
        let p = plan(&net, (route[0], 50.0), &[via], Some(&route));
        let join = p.join.expect("back on the route");
        assert_eq!(p.end, None);
        // after the join the lanes are the trip's, to its end
        let j = route.iter().position(|&l| l == p.lanes[join]).unwrap();
        assert_eq!(&p.lanes[join..], &route[j..]);
        // not before the via: the second block, which the diversion skips, is not driven
        assert!(j >= 1, "rejoined at route lane {j}");
        assert!(!p.lanes[..join].contains(&route[1]) || j == 1, "{:?}", p.lanes);
        // the via on the route itself: it simply goes on along it
        let on = DVec2::new(1.5, 300.0);
        let p = plan(&net, (route[0], 50.0), &[on], Some(&route));
        assert_eq!(p.lanes, route, "{p:?}");
        assert_eq!(p.join, Some(1));
    }

    #[test]
    fn rejoining_starts_no_earlier_than_abreast_of_the_via() {
        let net = grid(3);
        let route: Vec<usize> = (0..3).map(|j| find(&net, (1.5, j as f64 * 200.0), (1.5, (j + 1) as f64 * 200.0))).collect();
        let east = find(&net, (201.5, 400.0), (201.5, 600.0));
        // from the street east of the route's last block: the route lane nearest the via is the
        // last one; the way goes there, not back down to the first
        let (path, j) = rejoin(&net, (east, 50.0), DVec2::new(201.5, 450.0), &route).expect("a way");
        assert_eq!(j, 2, "{path:?}");
        assert_eq!(path.last(), Some(&route[2]));
        assert!(rejoin(&net, (east, 50.0), DVec2::new(201.5, 450.0), &[]).is_none());
    }

    #[test]
    fn pins_take_their_places_in_order() {
        let p = |n: &str| Pin { at: DVec2::ZERO, name: n.into(), n: 1 };
        let mut list = Vec::new();
        assert_eq!(add(&mut list, p("dest"), false), Some(0));
        assert_eq!(add(&mut list, p("a"), false), Some(0));
        assert_eq!(add(&mut list, p("b"), false), Some(1));
        let names = |l: &[Pin]| l.iter().map(|x| x.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&list), ["a", "b", "dest"]);
        assert!(shift(&mut list, 2, true));
        assert_eq!(names(&list), ["a", "dest", "b"], "moved up: b is the destination now");
        assert!(!shift(&mut list, 0, true) && !shift(&mut list, 2, false) && !shift(&mut list, 7, true));
        assert!(shift(&mut list, 0, false));
        assert_eq!(names(&list), ["dest", "a", "b"]);
        // a diversion: each goes last
        let mut d = Vec::new();
        for n in ["x", "y", "z"] {
            add(&mut d, p(n), true);
        }
        assert_eq!(names(&d), ["x", "y", "z"]);
        let mut full: Vec<Pin> = (0..MAX_PINS).map(|_| p("q")).collect();
        assert_eq!(add(&mut full, p("r"), false), None);
        // the roles: vias numbered, the last of a free drive the destination
        let pins = Pins { list: names(&list).iter().map(|n| p(n)).collect(), ..Pins::default() };
        assert_eq!((pins.role(0), pins.role(1), pins.role(2)), (Role::Via(1), Role::Via(2), Role::Destination));
        let div = Pins { trip: Some("t".into()), ..pins };
        assert_eq!(div.role(2), Role::Via(3));
    }

    #[test]
    fn pins_are_reached_in_order_and_the_destination_ends_the_drive() {
        let pin = |x: f64| Pin { at: DVec2::new(x, 0.0), name: String::new(), n: 1 };
        let mut p = Pins { list: vec![pin(100.0), pin(200.0), pin(300.0)], marks: vec![Some((1, 0.0)), Some((2, 0.0)), Some((3, 0.0))], ..Pins::default() };
        let bus = |x: f64| DVec3::new(x, 0.0, 0.0);
        assert_eq!(p.pass(bus(70.0)), None, "30 m short");
        // the second is passed first: it does not count before the first
        assert_eq!(p.pass(bus(200.0)), None);
        assert_eq!(p.pass(bus(80.0)), Some(Note::Reached(1)));
        assert_eq!((p.list.len(), p.marks.len(), p.marks[0]), (2, 2, Some((2, 0.0))));
        // the one left keeps its number
        assert_eq!(p.role(0), Role::Via(2));
        assert_eq!(p.note.map(|n| n.0), Some(Note::Reached(1)));
        assert_eq!(p.pass(bus(190.0)), Some(Note::Reached(2)));
        assert_eq!(p.role(0), Role::Destination);
        assert_eq!(p.pass(bus(310.0)), Some(Note::Arrived));
        assert!(!p.steers() && p.arrived == Some(ARRIVED_FOR) && p.list.len() == 1, "the flag stays a moment");
        assert_eq!(p.pass(bus(300.0)), None, "once");
        // a diversion's last via is a via: then the diversion is done
        let mut d = Pins { list: vec![pin(0.0)], trip: Some("trip".into()), ..Pins::default() };
        assert!(d.diverts("trip") && !d.diverts("other"));
        assert_eq!(d.pass(bus(5.0)), Some(Note::Reached(1)));
        assert!(d.list.is_empty() && !d.steers());
        // the note runs out
        d.tick(NOTE_FOR + 0.1);
        assert_eq!(d.note, None);
    }

    #[test]
    fn the_bus_is_found_on_a_route_running_its_way() {
        let net = grid(2);
        let route = [find(&net, (1.5, 0.0), (1.5, 200.0)), find(&net, (1.5, 200.0), (1.5, 400.0))];
        let at = locate(&net, &route, DVec3::new(2.0, 250.0, 0.0), 0.0).unwrap();
        assert_eq!(at.0, 1);
        assert!((at.1 - 50.0).abs() < 0.1);
        assert_eq!(locate(&net, &route, DVec3::new(2.0, 250.0, 0.0), 180.0), None, "the other way");
        assert_eq!(locate(&net, &route, DVec3::new(60.0, 250.0, 0.0), 0.0), None, "beside it");
    }

    #[test]
    fn distances_and_times_along_the_route() {
        let net = grid(2);
        let lanes = [find(&net, (1.5, 0.0), (1.5, 200.0)), find(&net, (1.5, 200.0), (1.5, 400.0))];
        assert_eq!(along(&net, &lanes, 0, 50.0, (1, 30.0)), Some(180.0));
        assert_eq!(along(&net, &lanes, 1, 10.0, (1, 30.0)), Some(20.0));
        assert_eq!(along(&net, &lanes, 1, 40.0, (1, 30.0)), Some(0.0), "just past it");
        assert_eq!(along(&net, &lanes, 1, 0.0, (0, 30.0)), Some(0.0), "behind");
        assert_eq!(along(&net, &lanes, 0, 0.0, (5, 0.0)), None);
        assert_eq!(eta(1200.0, 10.0), 120.0);
        assert_eq!(eta(100.0, 0.0), 20.0, "standing: at walking pace");
        assert_eq!((distance_text(1234.0), distance_text(234.0), distance_text(4.0)), ("1.2 km".to_string(), "230 m".to_string(), "0 m".to_string()));
        assert_eq!((eta_text(30.0), eta_text(150.0)), ("<1 min".to_string(), "3 min".to_string()));
    }

    #[test]
    fn a_pin_is_named_after_a_stop_near_it_or_its_street() {
        let stops = vec![(DVec2::new(0.0, 0.0), " Rathaus ".to_string()), (DVec2::new(150.0, 0.0), "Markt".to_string()), (DVec2::new(10.0, 0.0), " ".to_string())];
        assert_eq!(name(DVec2::new(120.0, 0.0), &stops, Some("Hauptstraße"), 1), "Markt");
        assert_eq!(name(DVec2::new(40.0, 0.0), &stops, None, 1), "Rathaus");
        assert_eq!(name(DVec2::new(900.0, 0.0), &stops, Some(" Hauptstraße "), 1), "Hauptstraße");
        // (in whichever language the tests run: "Point 3", "Punt 3" ...)
        let point = name(DVec2::new(900.0, 0.0), &stops, None, 3);
        assert!(point.ends_with(" 3") && point.len() > 3, "{point}");
    }

    /// The card: its height grows with the pins and is held to the room it has (the rest
    /// counted); a press finds each pin's buttons - no "earlier" for the first, no "later" for
    /// the last - and the clearing one; everything inside the card.
    #[test]
    fn the_card_is_laid_out_within_itself() {
        let fonts = Fonts::hanken();
        let mut atlas = Atlas::new(1024);
        let row = |k: usize, last: bool| CardRow { role: if last { Role::Destination } else { Role::Via(k + 1) }, name: format!("Stop {k}"), dist: Some(300.0 * (k + 1) as f64), eta: Some(60.0 * k as f64) };
        for s in [0.95f32, 1.2, 2.0] {
            let c = Card { rows: (0..3).map(|k| row(k, k == 2)).collect(), total: Some((900.0, 120.0, 40_000.0)), ..Card::default() };
            let h = card_height(&c, s, 10_000.0);
            assert!((h - (CARD_HEAD + 3.0 * CARD_ROW + CARD_FOOT + 6.0) * s).abs() < 0.01);
            let r = Rect::new(20.0, 60.0, 320.0 * s, h);
            let mut p = Painter::new();
            let hits = draw_card(&mut Pen { p: &mut p, atlas: &mut atlas, fonts: &fonts }, &c, r, s);
            for v in &p.verts {
                let (x, y) = (v.pos[0] + v.ext[0] * v.width[0], v.pos[1] + v.ext[1] * v.width[0]);
                // (the shadow reaches a little past it)
                assert!(x >= r.x - 20.0 * s && x <= r.right() + 20.0 * s && y >= r.y - 20.0 * s && y <= r.bottom() + 20.0 * s, "({x}, {y}) at {s}");
            }
            let kinds: Vec<CardHit> = hits.iter().map(|h| h.1).collect();
            assert!(!kinds.contains(&CardHit::Up(0)) && !kinds.contains(&CardHit::Down(2)));
            for k in [CardHit::Remove(0), CardHit::Down(0), CardHit::Up(1), CardHit::Down(1), CardHit::Up(2), CardHit::Remove(2), CardHit::Clear] {
                let (b, _) = hits.iter().find(|h| h.1 == k).unwrap_or_else(|| panic!("{k:?}"));
                assert!(r.contains(b.center()));
            }
            for (i, (a, _)) in hits.iter().enumerate() {
                for (b, _) in &hits[i + 1..] {
                    assert!(!(a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()), "{a:?} on {b:?}");
                }
            }
            // ten pins in a short window: as many as fit, the rest counted
            let c = Card { rows: (0..MAX_PINS).map(|k| row(k, k + 1 == MAX_PINS)).collect(), ..Card::default() };
            let room = 420.0 * s;
            let h = card_height(&c, s, room);
            assert!(h <= room, "{h} > {room}");
            let mut p = Painter::new();
            let hits = draw_card(&mut Pen { p: &mut p, atlas: &mut atlas, fonts: &fonts }, &c, Rect::new(0.0, 0.0, 320.0 * s, h), s);
            assert!(hits.iter().any(|x| x.1 == CardHit::Clear));
            assert!(hits.iter().all(|x| x.0.bottom() <= h + 0.01), "{hits:?}");
        }
        // the tool on and no pins: how to set one, no buttons
        let c = Card { tool: true, ..Card::default() };
        assert!(card_shown(&c) && !card_shown(&Card::default()));
        let mut p = Painter::new();
        assert!(draw_card(&mut Pen { p: &mut p, atlas: &mut atlas, fonts: &fonts }, &c, Rect::new(0.0, 0.0, 320.0, card_height(&c, 1.0, 900.0)), 1.0).is_empty());
    }

    /// Every text of the pins is in the tables for the languages that matter most, the
    /// placeholders kept.
    #[test]
    fn the_pins_are_translated() {
        let keys = ["Destination", "Diversion", "Via %{n}", "Via %{n} reached", "You have arrived", "No road within 60 m", "No way found to this point", "Ten points at most", "Point %{n}", "For your navigation only", "Click the map where you want to go.", "Click the map to add a via for your navigation.", "A right-click on the map works without the tool.", "Clear all", "Clear diversion", "%{n} more", "arrives %{time}", "In the game, the city map (Shift+M) sets a destination: right-click it, or use its pin tool."];
        for language in ["nl", "de", "fr", "ru", "uk", "pl"] {
            for key in keys {
                let t = crate::_rust_i18n_try_translate(language, key);
                assert!(t.as_ref().is_some_and(|t| !t.trim().is_empty()), "{language}: {key}");
                for var in ["%{n}", "%{time}"] {
                    assert_eq!(key.contains(var), t.as_ref().unwrap().contains(var), "{language}: {key}");
                }
            }
        }
    }
}
