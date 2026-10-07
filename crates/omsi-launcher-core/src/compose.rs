//! Duties put together from a map's timetable: not one tour of one line, but trips that
//! follow each other at the termini as a driver's shift runs - the rest of the same tour,
//! or another line leaving from where the bus stands.
//!
//! Which trip may follow which comes from the tours themselves: what one bus drives one
//! after the other ends and begins at the same place, whatever the stops are called (on
//! Thüringer Wald a stop's id matches between such trips in 19% of cases, its name in
//! 78%). A trip that two trips are both followed by makes their ends one place, and so
//! does a trip ending at the very stop object another begins at - that is where a driver
//! changes lines.
//!
//! A duty is a walk through that: from a first trip, at each terminus one of what leaves
//! there next, until the length asked for is reached. The walk is random (the same wish
//! gives another duty the next time) and it prefers another line to the one driven: three
//! hours of the same loop is not much of a shift.

use crate::LineInfo;
use std::collections::{HashMap, HashSet};

/// A trip with fewer stops is no bus line: a train, a ferry, a depot shunt.
const MIN_STOPS: usize = 3;
/// A bus standing longer than this is relieved there, not waiting for its next trip (s).
const MAX_LAYOVER: f64 = 45.0 * 60.0;
/// Time to take over another tour at a terminus (s): with a minute the bus leaves late.
const SWITCH_LAYOVER: f64 = 4.0 * 60.0;
/// Shorter than this is a round, not a duty (s).
pub const MIN_DUTY: f64 = 30.0 * 60.0;
/// A duty has a way back: one trip out is not one.
const MIN_LEGS: usize = 2;
/// Duties found before one of them is chosen (see `compose_one`).
const POOL: usize = 6;
/// Walks tried for one duty (a walk can run into a terminus nothing leaves from in time).
const ATTEMPTS: usize = 60;

/// One trip of a duty: the tour it belongs to - what the game drives - and what it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Leg {
    /// The line (`.ttl` name) and the tour, as `--line` and `--tour` take them.
    pub line: String,
    pub tour: String,
    /// The trip's place in its tour, 1 = the first (as `--trip` takes it), and its name.
    pub index: usize,
    pub trip: String,
    /// The line its displays show (the line's name when the trip has none).
    pub shown: String,
    pub from: String,
    pub terminus: String,
    /// Seconds of the day.
    pub departure: f64,
    pub arrival: f64,
}

/// Trips of one tour driven one after the other: what the game takes as one tour from a
/// trip on.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub line: String,
    pub tour: String,
    /// The place of its first trip in the tour (1 = the first) and how many trips.
    pub first: usize,
    pub trips: usize,
    pub departure: f64,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ComposedDuty {
    pub legs: Vec<Leg>,
}

impl ComposedDuty {
    pub fn start(&self) -> f64 {
        self.legs.first().map(|l| l.departure).unwrap_or(0.0)
    }

    pub fn end(&self) -> f64 {
        self.legs.last().map(|l| l.arrival).unwrap_or(0.0)
    }

    pub fn seconds(&self) -> f64 {
        self.end() - self.start()
    }

    /// The lines shown, each once, in the order they are driven.
    pub fn lines(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for l in &self.legs {
            if !out.iter().any(|x| x == &l.shown) {
                out.push(l.shown.clone());
            }
        }
        out
    }

    /// The tours driven one after the other.
    pub fn blocks(&self) -> Vec<Block> {
        let mut out: Vec<Block> = Vec::new();
        for l in &self.legs {
            match out.last_mut() {
                Some(b) if b.line == l.line && b.tour == l.tour && b.first + b.trips == l.index => b.trips += 1,
                _ => out.push(Block { line: l.line.clone(), tour: l.tour.clone(), first: l.index, trips: 1, departure: l.departure }),
            }
        }
        out
    }

    /// The tours to take over at a terminus: one fewer than the blocks.
    pub fn changes(&self) -> usize {
        self.blocks().len().saturating_sub(1)
    }
}

impl Block {
    /// As `--duty-leg` takes it: `line|tour|first|trips`. A line is a file's name, which
    /// has no `|`; a tour is read between the first `|` and the last two (see `parse`).
    pub fn arg(&self) -> String {
        format!("{}|{}|{}|{}", self.line, self.tour, self.first, self.trips)
    }

    pub fn parse(s: &str) -> Option<(String, String, usize, usize)> {
        let (line, rest) = s.split_once('|')?;
        let mut back = rest.rsplitn(3, '|');
        let trips = back.next()?.trim().parse().ok()?;
        let first = back.next()?.trim().parse().ok()?;
        let tour = back.next()?;
        Some((line.to_string(), tour.to_string(), first, trips))
    }
}

/// What the player asks for.
#[derive(Clone, Debug)]
pub struct Wish {
    /// The length wanted (s).
    pub seconds: f64,
    /// The duty's first trip leaves in this part of the day (s; `to` may lie past
    /// midnight, 30 h for the night).
    pub from: Option<f64>,
    pub to: Option<f64>,
    /// Only these lines (`.ttl` names); empty: every line the player may drive.
    pub lines: Vec<String>,
    pub seed: u64,
}

/// A small random generator (xorshift64*): the same seed gives the same duties, which is
/// what the tests need, and a new seed another set.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9e37_79b9_7f4a_7c15 | 1)
    }

    /// In [0, 1).
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        ((self.next() * n as f64) as usize).min(n.saturating_sub(1))
    }
}

/// A trip as it runs in one tour.
struct Run {
    leg: Leg,
    /// The trip (by name): its end is node `2 * trip`, its start `2 * trip + 1`.
    trip: usize,
    tour: usize,
}

/// Places joined, with path compression.
#[derive(Default)]
struct Places(Vec<usize>);

impl Places {
    fn find(&mut self, n: usize) -> usize {
        if n >= self.0.len() {
            let from = self.0.len();
            self.0.extend(from..=n);
        }
        let mut root = n;
        while self.0[root] != root {
            root = self.0[root];
        }
        let mut at = n;
        while self.0[at] != root {
            let up = self.0[at];
            self.0[at] = root;
            at = up;
        }
        root
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[ra] = rb;
        }
    }
}

/// The trips that run on the day, and what leaves from where.
struct Net {
    runs: Vec<Run>,
    /// Where each run ends.
    end: Vec<usize>,
    /// The runs leaving each place, by departure.
    leaving: HashMap<usize, Vec<usize>>,
}

fn build(lines: &[LineInfo]) -> Net {
    let mut names: HashMap<String, usize> = HashMap::new();
    let mut places = Places::default();
    let mut runs: Vec<Run> = Vec::new();
    // (first and last stop object of each run)
    let mut ends: Vec<(i64, i64)> = Vec::new();
    let mut tour_count = 0;
    for l in lines.iter().filter(|l| l.user_allowed) {
        for t in l.tours.iter().filter(|t| t.runs) {
            let tour = tour_count;
            tour_count += 1;
            let mut trips: Vec<_> = t.trips.iter().collect();
            trips.sort_by(|a, b| a.departure.total_cmp(&b.departure));
            let mut previous: Option<usize> = None;
            for trip in trips {
                if trip.stops.len() < MIN_STOPS {
                    // (what follows a depot shunt does not follow the trip before it)
                    previous = None;
                    continue;
                }
                let n = names.len();
                let key = *names.entry(trip.name.to_lowercase()).or_insert(n);
                let shown = if trip.line.trim().is_empty() { l.name.clone() } else { trip.line.trim().to_string() };
                let r = runs.len();
                runs.push(Run {
                    leg: Leg {
                        line: l.name.clone(),
                        tour: t.number.clone(),
                        index: trip.index,
                        trip: trip.name.clone(),
                        shown,
                        from: trip.from.clone(),
                        terminus: trip.terminus.clone(),
                        departure: trip.departure,
                        arrival: trip.arrival,
                    },
                    trip: key,
                    tour,
                });
                ends.push((trip.stops.first().map(|s| s.id).unwrap_or(0), trip.stops.last().map(|s| s.id).unwrap_or(0)));
                if let Some(p) = previous {
                    let gap = trip.departure - runs[p].leg.arrival;
                    if (0.0..=MAX_LAYOVER).contains(&gap) {
                        places.union(2 * runs[p].trip, 2 * key + 1);
                    }
                }
                previous = Some(r);
            }
        }
    }
    // a trip ending at the stop object another begins at: the bus is there already
    let mut starting_at: HashMap<i64, Vec<usize>> = HashMap::new();
    for (r, run) in runs.iter().enumerate() {
        if ends[r].0 != 0 {
            starting_at.entry(ends[r].0).or_default().push(2 * run.trip + 1);
        }
    }
    for (r, run) in runs.iter().enumerate() {
        if let Some(starts) = starting_at.get(&ends[r].1) {
            for &s in starts {
                places.union(2 * run.trip, s);
            }
        }
    }
    let end: Vec<usize> = runs.iter().map(|r| places.find(2 * r.trip)).collect();
    let mut leaving: HashMap<usize, Vec<usize>> = HashMap::new();
    for (r, run) in runs.iter().enumerate() {
        leaving.entry(places.find(2 * run.trip + 1)).or_default().push(r);
    }
    for list in leaving.values_mut() {
        list.sort_by(|a, b| runs[*a].leg.departure.total_cmp(&runs[*b].leg.departure));
    }
    Net { runs, end, leaving }
}

/// What may follow run `r`: the rest of its own tour, and another tour leaving from where it
/// ends with time to take it over.
fn next_of(net: &Net, r: usize) -> impl Iterator<Item = usize> + '_ {
    let run = &net.runs[r];
    net.leaving.get(&net.end[r]).into_iter().flatten().copied().filter(move |&n| {
        let next = &net.runs[n];
        let gap = next.leg.departure - run.leg.arrival;
        (0.0..=MAX_LAYOVER).contains(&gap) && (next.tour == run.tour || gap >= SWITCH_LAYOVER)
    })
}

/// How much another line is wanted: one not driven yet most, the same line least - but
/// still, when it is all there is.
fn appetite(line: &str, current: &str, driven: &HashSet<&str>) -> f64 {
    if line == current {
        1.0
    } else if driven.contains(line) {
        3.0
    } else {
        6.0
    }
}

/// The line first, then the trip: at a terminus the own line often has twenty trips
/// leaving and another line one, and weighed by trip the own line would win nearly always.
fn pick_next(net: &Net, options: &[usize], current: &str, driven: &HashSet<&str>, rng: &mut Rng) -> usize {
    let mut per_line: Vec<(&str, Vec<usize>)> = Vec::new();
    for &o in options {
        let line = net.runs[o].leg.shown.as_str();
        match per_line.iter_mut().find(|(l, _)| *l == line) {
            Some((_, list)) => list.push(o),
            None => per_line.push((line, vec![o])),
        }
    }
    let total: f64 = per_line.iter().map(|(l, _)| appetite(l, current, driven)).sum();
    let mut ticket = rng.next() * total;
    let mut chosen = &per_line[per_line.len() - 1].1;
    for (l, list) in &per_line {
        ticket -= appetite(l, current, driven);
        if ticket <= 0.0 {
            chosen = list;
            break;
        }
    }
    chosen[rng.below(chosen.len())]
}

fn walk(net: &Net, start: usize, target: f64, tolerance: f64, allowed: &dyn Fn(usize) -> bool, rng: &mut Rng) -> Vec<usize> {
    let begin = net.runs[start].leg.departure;
    let mut legs = vec![start];
    let mut driven: HashSet<&str> = HashSet::new();
    driven.insert(&net.runs[start].leg.shown);
    loop {
        let last = *legs.last().unwrap();
        if legs.len() >= MIN_LEGS && net.runs[last].leg.arrival - begin >= target - tolerance {
            break;
        }
        let options: Vec<usize> = next_of(net, last).filter(|&n| net.runs[n].leg.arrival - begin <= target + tolerance && allowed(n)).collect();
        if options.is_empty() {
            break;
        }
        let next = pick_next(net, &options, &net.runs[last].leg.shown, &driven, rng);
        driven.insert(&net.runs[next].leg.shown);
        legs.push(next);
    }
    legs
}

fn duty_of(net: &Net, legs: &[usize]) -> ComposedDuty {
    ComposedDuty { legs: legs.iter().map(|&r| net.runs[r].leg.clone()).collect() }
}

fn line_count(net: &Net, legs: &[usize]) -> usize {
    legs.iter().map(|&r| net.runs[r].leg.shown.as_str()).collect::<HashSet<_>>().len()
}

/// A part of the day: a departure at 01:00 lies in a night that began at 22:00.
fn in_window(departure: f64, from: Option<f64>, to: Option<f64>) -> bool {
    let inside = |d: f64| from.is_none_or(|f| d >= f) && to.is_none_or(|t| d <= t);
    inside(departure) || inside(departure + 86400.0)
}

/// One duty of about the length wanted. A few are walked and one of them chosen, the more
/// lines in it the likelier (by the square: two lines four times as likely as one), so a
/// map's variety shows without every duty being the same.
fn compose_one(net: &Net, starts: &[usize], wish: &Wish, allowed: &dyn Fn(usize) -> bool, rng: &mut Rng) -> Option<ComposedDuty> {
    if starts.is_empty() {
        return None;
    }
    let target = wish.seconds.max(MIN_DUTY);
    // (two trips seldom fit half an hour exactly: a short duty needs more room)
    let tolerance = (target * 0.2).max(12.0 * 60.0);
    let floor = (target - tolerance).max(MIN_DUTY);
    let mut good: Vec<Vec<usize>> = Vec::new();
    let mut best: Option<Vec<usize>> = None;
    let length = |legs: &[usize]| net.runs[*legs.last().unwrap()].leg.arrival - net.runs[legs[0]].leg.departure;
    for _ in 0..ATTEMPTS {
        if good.len() >= POOL {
            break;
        }
        let legs = walk(net, starts[rng.below(starts.len())], target, tolerance, allowed, rng);
        if legs.len() < MIN_LEGS {
            continue;
        }
        if length(&legs) >= floor {
            good.push(legs);
        } else if best.as_ref().is_none_or(|b| length(&legs) > length(b)) {
            best = Some(legs);
        }
    }
    if !good.is_empty() {
        let weights: Vec<f64> = good.iter().map(|g| (line_count(net, g) as f64).powi(2)).collect();
        let mut ticket = rng.next() * weights.iter().sum::<f64>();
        for (g, w) in good.iter().zip(&weights) {
            ticket -= w;
            if ticket <= 0.0 {
                return Some(duty_of(net, g));
            }
        }
        return Some(duty_of(net, good.last().unwrap()));
    }
    best.filter(|b| length(b) >= MIN_DUTY).map(|b| duty_of(net, &b))
}

/// Up to `count` different duties for the wish, by their start. Two are the same when they
/// begin with the same trip at the same time.
pub fn compose(lines: &[LineInfo], wish: &Wish, count: usize) -> Vec<ComposedDuty> {
    let net = build(lines);
    let only: HashSet<String> = wish.lines.iter().map(|l| l.to_lowercase()).collect();
    let allowed = |r: usize| only.is_empty() || only.contains(&net.runs[r].leg.line.to_lowercase());
    let starts: Vec<usize> = (0..net.runs.len()).filter(|&r| allowed(r) && in_window(net.runs[r].leg.departure, wish.from, wish.to)).collect();
    let mut rng = Rng::new(wish.seed);
    let mut found: Vec<ComposedDuty> = Vec::new();
    for _ in 0..count * 12 {
        if found.len() >= count {
            break;
        }
        let Some(d) = compose_one(&net, &starts, wish, &allowed, &mut rng) else { continue };
        let same = |x: &ComposedDuty| x.legs[0].trip == d.legs[0].trip && x.start() == d.start();
        if !found.iter().any(same) {
            found.push(d);
        }
    }
    found.sort_by(|a, b| a.start().total_cmp(&b.start()));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StopInfo, TourInfo, TripInfo};

    fn trip(name: &str, index: usize, line: &str, stops: &[(i64, &str)], dep_min: f64, minutes: f64) -> TripInfo {
        let departure = dep_min * 60.0;
        TripInfo {
            name: name.into(),
            index,
            line: line.into(),
            from: stops[0].1.into(),
            terminus: stops[stops.len() - 1].1.into(),
            departure,
            arrival: departure + minutes * 60.0,
            stops: stops.iter().map(|(id, n)| StopInfo { name: n.to_string(), id: *id, arr: departure, dep: departure }).collect(),
            km: 5.0,
        }
    }

    fn tour(number: &str, runs: bool, trips: Vec<TripInfo>) -> TourInfo {
        TourInfo { number: number.into(), ai_group: String::new(), first: trips[0].departure, last: trips[trips.len() - 1].arrival, days: "daily".into(), runs, next_run: None, trips }
    }

    fn line(name: &str, tours: Vec<TourInfo>) -> LineInfo {
        LineInfo { name: name.into(), user_allowed: true, termini: Vec::new(), tours }
    }

    const NORTH: (i64, &str) = (1, "North");
    const MIDDLE: (i64, &str) = (2, "Middle");
    const SOUTH: (i64, &str) = (3, "South");
    const EAST: (i64, &str) = (4, "East");
    const WEST: (i64, &str) = (5, "West");

    /// Line 1 goes North-South and back all morning; line 2 East-West, with its own bus.
    /// Line 2's trips begin at the stop object line 1's trips to the South end at.
    fn map() -> Vec<LineInfo> {
        let mut one = Vec::new();
        for k in 0..6 {
            let at = 6.0 * 60.0 + k as f64 * 60.0;
            one.push(trip("1 North-South", 2 * k + 1, "1", &[NORTH, MIDDLE, SOUTH], at, 25.0));
            one.push(trip("1 South-North", 2 * k + 2, "1", &[SOUTH, MIDDLE, NORTH], at + 30.0, 25.0));
        }
        let mut two = Vec::new();
        for k in 0..6 {
            let at = 6.0 * 60.0 + 35.0 + k as f64 * 60.0;
            two.push(trip("2 South-West", 2 * k + 1, "2", &[SOUTH, EAST, WEST], at, 20.0));
            two.push(trip("2 West-South", 2 * k + 2, "2", &[WEST, EAST, SOUTH], at + 25.0, 20.0));
        }
        vec![line("Line 1", vec![tour("1", true, one)]), line("Line 2", vec![tour("1", true, two)])]
    }

    fn wish(minutes: f64, seed: u64) -> Wish {
        Wish { seconds: minutes * 60.0, from: None, to: None, lines: Vec::new(), seed }
    }

    #[test]
    fn a_duty_follows_on_at_each_terminus() {
        let duties = compose(&map(), &wish(120.0, 7), 8);
        assert!(!duties.is_empty());
        for d in &duties {
            assert!(d.legs.len() >= MIN_LEGS);
            for w in d.legs.windows(2) {
                let gap = w[1].departure - w[0].arrival;
                assert!((0.0..=MAX_LAYOVER).contains(&gap), "{} then {}: {gap} s", w[0].trip, w[1].trip);
                // the next trip starts where the last one ended
                assert_eq!(w[0].terminus, w[1].from);
                if w[0].tour != w[1].tour || w[0].line != w[1].line {
                    assert!(gap >= SWITCH_LAYOVER);
                }
            }
            assert!(d.seconds() <= (120.0 + 24.0) * 60.0, "{} min", d.seconds() / 60.0);
        }
    }

    #[test]
    fn duties_change_lines_where_they_meet() {
        // over a few seeds, some duty takes line 2 at the South
        let changed = (0..20).flat_map(|s| compose(&map(), &wish(150.0, s), 8)).any(|d| d.lines().len() > 1 && d.changes() > 0);
        assert!(changed);
    }

    #[test]
    fn the_same_seed_gives_the_same_duties() {
        assert_eq!(compose(&map(), &wish(90.0, 3), 6), compose(&map(), &wish(90.0, 3), 6));
    }

    #[test]
    fn only_the_lines_asked_for() {
        let mut w = wish(90.0, 5);
        w.lines = vec!["line 2".into()];
        let duties = compose(&map(), &w, 6);
        assert!(!duties.is_empty());
        assert!(duties.iter().all(|d| d.legs.iter().all(|l| l.line == "Line 2")));
    }

    #[test]
    fn the_first_trip_leaves_in_the_part_of_the_day_asked_for() {
        let mut w = wish(60.0, 9);
        w.from = Some(9.0 * 3600.0);
        w.to = Some(10.0 * 3600.0);
        let duties = compose(&map(), &w, 6);
        assert!(!duties.is_empty());
        assert!(duties.iter().all(|d| (9.0 * 3600.0..=10.0 * 3600.0).contains(&d.start())));
    }

    #[test]
    fn tours_not_running_on_the_day_and_short_trips_are_left_out() {
        let mut lines = map();
        lines[1].tours[0].runs = false;
        // a depot shunt of two stops in line 1
        lines[0].tours[0].trips.push(trip("1 Depot", 13, "1", &[NORTH, MIDDLE], 12.0 * 60.0, 5.0));
        let duties = compose(&lines, &wish(120.0, 1), 8);
        assert!(duties.iter().all(|d| d.legs.iter().all(|l| l.line == "Line 1" && l.trip != "1 Depot")));
    }

    #[test]
    fn blocks_group_the_trips_of_one_tour() {
        let leg = |line: &str, index: usize| Leg { line: line.into(), tour: "1".into(), index, trip: String::new(), shown: String::new(), from: String::new(), terminus: String::new(), departure: index as f64, arrival: index as f64 };
        let d = ComposedDuty { legs: vec![leg("A", 3), leg("A", 4), leg("B", 2), leg("B", 3), leg("A", 7)] };
        let b = d.blocks();
        assert_eq!(b.iter().map(|b| (b.line.as_str(), b.first, b.trips)).collect::<Vec<_>>(), [("A", 3, 2), ("B", 2, 2), ("A", 7, 1)]);
        assert_eq!(d.changes(), 2);
    }

    #[test]
    fn a_block_reads_back_with_bars_in_its_tour() {
        let b = Block { line: "4 Liman-ZS".into(), tour: "Mo|Fr 3".into(), first: 2, trips: 5, departure: 0.0 };
        assert_eq!(Block::parse(&b.arg()), Some(("4 Liman-ZS".into(), "Mo|Fr 3".into(), 2, 5)));
        assert_eq!(Block::parse("x|y"), None);
    }

    /// A real map's duties, printed: `OMSI_MAP=".../maps/Berlin-Spandau" cargo test -p
    /// omsi-launcher-core compose::tests::real_map -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_map() {
        let dir = std::path::PathBuf::from(std::env::var("OMSI_MAP").expect("OMSI_MAP"));
        let date = std::env::var("OMSI_DATE").unwrap_or_else(|_| "2024-05-15".into());
        let lines = crate::lines_on(&dir, &date).unwrap();
        for minutes in [60.0, 120.0, 240.0] {
            let t = std::time::Instant::now();
            let duties = compose(&lines, &wish(minutes, 42), 8);
            println!("{minutes} min: {} duties in {:?}", duties.len(), t.elapsed());
            for d in &duties {
                let hm = |s: f64| format!("{:02}:{:02}", (s / 3600.0) as i32 % 24, (s % 3600.0 / 60.0) as i32);
                println!("  {}-{} lines {:?}, {} trips, {} changes", hm(d.start()), hm(d.end()), d.lines(), d.legs.len(), d.changes());
            }
        }
    }

    #[test]
    fn a_night_window_takes_trips_after_midnight() {
        assert!(in_window(1.0 * 3600.0, Some(22.0 * 3600.0), Some(30.0 * 3600.0)));
        assert!(in_window(23.0 * 3600.0, Some(22.0 * 3600.0), Some(30.0 * 3600.0)));
        assert!(!in_window(12.0 * 3600.0, Some(22.0 * 3600.0), Some(30.0 * 3600.0)));
    }
}
