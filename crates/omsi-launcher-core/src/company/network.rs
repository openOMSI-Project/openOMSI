//! The company's lines: lines of the map's timetable it takes on, and the player's own from
//! the line editor (they are `.ttl` lines of the timetable too, see `crate::lines`). What a
//! line asks of the company on a day are its tours of that day, as the timetable has them -
//! each a bus from its first trip to its last, cut into duties for the drivers where it
//! stands long enough (Omsi-Hub's `omlopenVanDag` and `knipOmloop`).

use super::dates;
use super::economy;
use super::model::{BusSize, Cents, Company, CompanyLine, Difficulty, LEGACY_SERVICE};
use super::staff::{DUTY_MAX, SPLIT_PAUSE};
use crate::lines::OwnLine;
use crate::LineInfo;
use serde::{Deserialize, Serialize};

/// A trip as the day plans it: minutes of the day.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct PlannedTrip {
    pub name: String,
    /// The number its displays show.
    pub line: String,
    pub from: String,
    pub to: String,
    pub dep: i32,
    pub arr: i32,
    pub km: f64,
    pub stops: u32,
    /// A depot run, an empty run or another trip without passengers (`specials::Kind::empty`):
    /// run with its tour, booked as empty kilometres.
    #[serde(default)]
    pub empty: bool,
}

impl PlannedTrip {
    /// A trip with passengers (a depot run or a short positioning trip carries none: a duty is
    /// not cut before one).
    pub fn counts(&self) -> bool {
        !self.empty && self.stops >= 3
    }
}

/// A tour of a company line on the day.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct TourOfDay {
    /// The company line (its timetable name) and the number shown.
    pub line: String,
    pub number: String,
    pub tour: String,
    pub ai_group: String,
    pub trips: Vec<PlannedTrip>,
    /// Its line is not in service when it begins (not planned yet, or from later on): shown
    /// on the planning to be planned, but neither run nor penalised (`in_service`).
    #[serde(default)]
    pub unplanned: bool,
}

impl TourOfDay {
    pub fn from(&self) -> i32 {
        self.trips.first().map(|t| t.dep).unwrap_or(0)
    }

    pub fn to(&self) -> i32 {
        self.trips.iter().map(|t| t.arr).max().unwrap_or(0)
    }

    pub fn km(&self) -> f64 {
        self.trips.iter().map(|t| t.km).sum()
    }

    /// The bus size its depot group asks for, if it names one (`vormVanDepot`).
    pub fn wants(&self) -> Option<BusSize> {
        wants_size(&self.ai_group)
    }
}

/// The bus size an AI group or depot name asks for ("Gelenkbus", "Solo", "DD").
pub fn wants_size(group: &str) -> Option<BusSize> {
    let g = group.to_lowercase();
    let word = |w: &str| g.split(|c: char| !c.is_alphanumeric()).any(|x| x == w);
    if g.contains("gelenk") || g.contains("schlenk") || g.contains("artic") {
        Some(BusSize::Articulated)
    } else if g.contains("doppeldeck") || word("dd") {
        Some(BusSize::Double)
    } else if g.contains("midi") || g.contains("kurz") {
        Some(BusSize::Midi)
    } else if g.contains("solo") || g.contains("standard") {
        Some(BusSize::Solo)
    } else {
        None
    }
}

/// Take on a line of the map's timetable (`own`: the player's line it is, from the line
/// editor's registry).
pub fn add_line(c: &mut Company, line: &LineInfo, own: Option<&OwnLine>) -> Result<(), &'static str> {
    if c.lines.iter().any(|l| l.name.eq_ignore_ascii_case(&line.name)) {
        return Err("The company runs this line already.");
    }
    let mut numbers: Vec<String> = Vec::new();
    // (the passenger trips' numbers: not a depot run's "X")
    for t in line.tours.iter().flat_map(|t| t.trips.iter()).filter(|t| !super::specials::trip_kind(&t.line, &t.name, &t.terminus, t.stops.len(), &[&c.depot]).empty()) {
        let n = t.line.trim();
        if !n.is_empty() && !numbers.iter().any(|x| x == n) {
            numbers.push(n.to_string());
        }
    }
    let number = match own {
        Some(o) if !o.number.trim().is_empty() => o.number.trim().to_string(),
        _ => numbers.first().cloned().unwrap_or_else(|| line.name.clone()),
    };
    let caption = own.map(|o| o.caption()).filter(|c| !c.is_empty()).unwrap_or_else(|| super::specials::caption_of(line, &[&c.depot]));
    let runs: Vec<_> = line.tours.iter().filter(|t| t.runs).collect();
    c.lines.push(CompanyLine {
        name: line.name.clone(),
        number,
        numbers,
        own: own.is_some(),
        colour: own.map(|o| o.colour.clone()).unwrap_or_default(),
        caption,
        added: c.date.clone(),
        tours: runs.len() as u32,
        km: runs.iter().flat_map(|t| t.trips.iter()).map(|t| t.km).sum(),
        plan: None,
        // (a line runs only once it is planned: `start_service`)
        service_from: None,
        fare: None,
        demand: Default::default(),
        title: String::new(),
        hof: String::new(),
        pending: None,
    });
    Ok(())
}

/// The line runs at `moment` (the clock's minutes): it was put into service by then.
pub fn in_service(l: &CompanyLine, moment: i64) -> bool {
    l.service_from.is_some_and(|s| s == LEGACY_SERVICE || moment >= s)
}

/// Put a line into service from `from` (the clock's minutes; now at the soonest): its tours
/// from then on run as they are planned, and what is not covered is dropped with the
/// contract's penalty. Returns when it begins.
pub fn start_service(c: &mut Company, name: &str, from: i64) -> Result<i64, &'static str> {
    let now = super::clock::now(c);
    let Some(l) = c.lines.iter_mut().find(|l| l.name.eq_ignore_ascii_case(name)) else { return Err("The company does not run this line.") };
    let from = from.max(now);
    l.service_from = Some(from);
    Ok(from)
}

/// Take a line out of service again (its tours wait for the planning).
pub fn stop_service(c: &mut Company, name: &str) -> Result<(), &'static str> {
    let Some(l) = c.lines.iter_mut().find(|l| l.name.eq_ignore_ascii_case(name)) else { return Err("The company does not run this line.") };
    l.service_from = None;
    Ok(())
}

/// The days of grace a concession line has to start its service, and the share of the
/// authority's payment for its kilometres charged for every day after them it does not run
/// (None: never charged - Easy).
pub fn start_grace(d: Difficulty) -> Option<(i64, f64)> {
    match d {
        Difficulty::Easy => None,
        Difficulty::Realistic => Some((7, 0.2)),
        Difficulty::Hard => Some((3, 0.35)),
    }
}

/// What a concession line not in service on `date` costs that day: after its grace, a share
/// of what the authority pays for its kilometres (an own line costs nothing). (days since it
/// was taken on, the charge)
pub fn late_start(c: &Company, l: &CompanyLine, date: &str) -> Option<(i64, Cents)> {
    if l.own {
        return None;
    }
    let (grace, share) = start_grace(c.difficulty)?;
    let end = super::clock::moment(date, 24 * 60 - 1);
    if l.service_from.is_some_and(|s| s == LEGACY_SERVICE || s <= end) {
        return None;
    }
    let days = dates::between(&l.added, date);
    if days < grace {
        return None;
    }
    let r = economy::rules(c.difficulty);
    let charge = (economy::compensation_per_km(&r, c.reputation, c.contract_index) * l.km * share).round() as Cents;
    (charge > 0).then_some((days, charge))
}

pub fn remove_line(c: &mut Company, name: &str) {
    c.lines.retain(|l| !l.name.eq_ignore_ascii_case(name));
}

/// The company line a trip report names (by the number its displays showed).
pub fn line_of_number<'a>(c: &'a Company, number: &str) -> Option<&'a CompanyLine> {
    let n = number.trim();
    if n.is_empty() {
        return None;
    }
    c.lines.iter().find(|l| l.number.eq_ignore_ascii_case(n) || l.numbers.iter().any(|x| x.eq_ignore_ascii_case(n)) || l.name.eq_ignore_ascii_case(n))
}

/// The tours of the company's lines on `date` (`lines` the timetable read for it): those that
/// run that day, their trips in order of departure, each marked `unplanned` when its line is
/// not in service as it begins. The one source of a day's tours - the Lines page, the
/// planning, the clock and the day's close all take them from here.
pub fn tours_of_day(c: &Company, lines: &[LineInfo], date: &str) -> Vec<TourOfDay> {
    let mut out = Vec::new();
    for cl in &c.lines {
        let Some(line) = lines.iter().find(|l| l.name.eq_ignore_ascii_case(&cl.name)) else { continue };
        for t in line.tours.iter().filter(|t| t.runs) {
            let mut trips: Vec<PlannedTrip> = t
                .trips
                .iter()
                .map(|x| PlannedTrip {
                    name: x.name.clone(),
                    line: if x.line.trim().is_empty() { cl.number.clone() } else { x.line.trim().to_string() },
                    from: x.from.clone(),
                    to: x.terminus.clone(),
                    dep: (x.departure / 60.0).round() as i32,
                    arr: (x.arrival / 60.0).round() as i32,
                    km: x.km,
                    stops: x.stops.len() as u32,
                    empty: super::specials::trip_kind(&x.line, &x.name, &x.terminus, x.stops.len(), &[&c.depot]).empty(),
                })
                .collect();
            trips.sort_by_key(|x| x.dep);
            if trips.iter().any(PlannedTrip::counts) {
                out.push(TourOfDay { line: cl.name.clone(), number: cl.number.clone(), tour: t.number.clone(), ai_group: t.ai_group.clone(), trips, unplanned: false });
            }
        }
    }
    // (the own lines' tours with their depot runs: their timetable has none)
    super::ownline::add_depot_runs(c, &mut out);
    for t in out.iter_mut() {
        let begins = super::clock::moment(date, t.from() as i64);
        t.unplanned = !c.lines.iter().find(|l| l.name.eq_ignore_ascii_case(&t.line)).is_some_and(|l| in_service(l, begins));
    }
    out.sort_by_key(|t| t.from());
    out
}

/// The tours that run: those of lines in service.
pub fn in_service_only(tours: Vec<TourOfDay>) -> Vec<TourOfDay> {
    tours.into_iter().filter(|t| !t.unplanned).collect()
}

/// What a line asks of the company on a day, against what it has.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Needs {
    /// Buses at the line's busiest, and of them how many of each size its tours ask for
    /// (a solo bus where they name none).
    pub buses: usize,
    pub sizes: Vec<(BusSize, usize)>,
    /// Its duties, and about the drivers they need: one and a half a duty (a driver works
    /// five days a week and has holidays and ill days).
    pub duties: usize,
    pub drivers: usize,
    /// The company's buses and drivers, and what its lines in service take of them already.
    pub have_buses: usize,
    pub have_drivers: usize,
    pub busy_buses: usize,
    pub busy_drivers: usize,
}

impl Needs {
    /// Buses and drivers the company lacks for the line beside its others.
    pub fn short(&self) -> (usize, usize) {
        ((self.buses + self.busy_buses).saturating_sub(self.have_buses), (self.drivers + self.busy_drivers).saturating_sub(self.have_drivers))
    }
}

/// The most tours running at once.
fn peak(tours: &[&TourOfDay]) -> usize {
    tours.iter().map(|t| tours.iter().filter(|x| x.from() <= t.from() && t.from() < x.to()).count()).max().unwrap_or(0)
}

/// About the drivers `duties` duties a day need.
pub fn drivers_for(duties: usize) -> usize {
    (duties as f64 * 1.5).ceil() as usize
}

/// What `line` asks of the company on `date` (`all` the timetable of that day, for what its
/// lines in service take already).
pub fn needs(c: &Company, line: &LineInfo, all: &[LineInfo], date: &str) -> Needs {
    let mut probe = c.clone();
    probe.lines.retain(|l| l.name.eq_ignore_ascii_case(&line.name));
    if probe.lines.is_empty() {
        let _ = add_line(&mut probe, line, None);
    }
    for l in probe.lines.iter_mut() {
        l.service_from = Some(LEGACY_SERVICE);
    }
    let mine = tours_of_day(&probe, std::slice::from_ref(line), date);
    let refs: Vec<&TourOfDay> = mine.iter().collect();
    let mut sizes: Vec<(BusSize, usize)> = Vec::new();
    for size in [BusSize::Midi, BusSize::Solo, BusSize::Articulated, BusSize::Double] {
        let of: Vec<&TourOfDay> = mine.iter().filter(|t| super::ownline::wanted(&probe, t).unwrap_or(BusSize::Solo) == size).collect();
        let n = peak(&of);
        if n > 0 {
            sizes.push((size, n));
        }
    }
    let duties: usize = mine.iter().map(|t| duties_of(t).len()).sum();
    let others: Vec<TourOfDay> = in_service_only(tours_of_day(c, all, date)).into_iter().filter(|t| !t.line.eq_ignore_ascii_case(&line.name)).collect();
    let other_refs: Vec<&TourOfDay> = others.iter().collect();
    Needs {
        buses: peak(&refs),
        sizes,
        duties,
        drivers: drivers_for(duties),
        have_buses: c.fleet.iter().filter(|v| v.held_on(date)).count(),
        have_drivers: c.staff.iter().filter(|e| e.employed_on(date) && e.notice_until.is_none()).count(),
        busy_buses: peak(&other_refs),
        busy_drivers: drivers_for(others.iter().map(|t| duties_of(t).len()).sum()),
    }
}

/// What the timetable gives each company line on that day: tours and kilometres (kept with
/// the line for the pages).
pub fn refresh_lines(c: &mut Company, lines: &[LineInfo]) {
    for cl in c.lines.iter_mut() {
        if let Some(l) = lines.iter().find(|l| l.name.eq_ignore_ascii_case(&cl.name)) {
            let runs: Vec<_> = l.tours.iter().filter(|t| t.runs).collect();
            cl.tours = runs.len() as u32;
            cl.km = runs.iter().flat_map(|t| t.trips.iter()).map(|t| t.km).sum();
        }
    }
}

/// A tour cut into duties of at most `max` minutes (`knipOmloop`): as many parts as needed,
/// the cuts as near an even split as can be, only before a trip with passengers after the bus
/// stood at least `SPLIT_PAUSE` minutes (there a driver can take over). Where there is no
/// such place it is not cut; a part without a trip with passengers goes into the one before.
pub fn split_tour(trips: &[PlannedTrip], max: i32) -> Vec<std::ops::Range<usize>> {
    if trips.is_empty() {
        return Vec::new();
    }
    let begin = trips[0].dep;
    let span = trips.iter().map(|t| t.arr).max().unwrap_or(begin) - begin;
    let parts = ((span as f64 / max as f64).ceil() as i32).max(1);
    let mut cuts: Vec<usize> = Vec::new();
    for k in 1..parts {
        let ideal = begin as f64 + span as f64 * k as f64 / parts as f64;
        let mut best: Option<usize> = None;
        for i in 1..trips.len() {
            if i <= cuts.last().copied().unwrap_or(0) {
                continue;
            }
            if !trips[i].counts() || trips[i].dep - trips[i - 1].arr < SPLIT_PAUSE {
                continue;
            }
            if best.is_none_or(|b| (trips[i].dep as f64 - ideal).abs() < (trips[b].dep as f64 - ideal).abs()) {
                best = Some(i);
            }
        }
        if let Some(b) = best {
            cuts.push(b);
        }
    }
    let mut pieces: Vec<std::ops::Range<usize>> = Vec::new();
    let mut from = 0;
    for cut in cuts.into_iter().chain(std::iter::once(trips.len())) {
        pieces.push(from..cut);
        from = cut;
    }
    let mut out: Vec<std::ops::Range<usize>> = Vec::new();
    for p in pieces {
        let counts = trips[p.clone()].iter().any(PlannedTrip::counts);
        match out.last_mut() {
            Some(last) if !counts => last.end = p.end,
            _ => out.push(p),
        }
    }
    if out.len() > 1 && !trips[out[0].clone()].iter().any(PlannedTrip::counts) {
        let first = out.remove(0);
        out[0].start = first.start;
    }
    out
}

/// The duties of a tour (`DUTY_MAX`).
pub fn duties_of(t: &TourOfDay) -> Vec<std::ops::Range<usize>> {
    split_tour(&t.trips, DUTY_MAX)
}

/// The first day of the company's next week (for a page that says when the markets change).
pub fn next_monday(c: &Company) -> String {
    dates::fmt(dates::week_of(&c.date) + 7)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn trip(dep: i32, arr: i32, stops: u32) -> PlannedTrip {
        PlannedTrip { name: format!("t{dep}"), line: "5".into(), from: "A".into(), to: "B".into(), dep, arr, km: (arr - dep) as f64 * 0.3, stops, empty: false }
    }

    #[test]
    fn a_long_tour_is_cut_into_duties() {
        // 5:00 to 23:00 in trips of 50 minutes with 10 minutes' layover: two parts
        let trips: Vec<PlannedTrip> = (0..18).map(|k| trip(300 + k * 60, 350 + k * 60, 12)).collect();
        let d = split_tour(&trips, DUTY_MAX);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].start, 0);
        assert_eq!(d[1].end, 18);
        // the cut lies near the middle (14:00)
        assert!((trips[d[1].start].dep - 14 * 60).abs() <= 60);
        // a short tour stays whole
        assert_eq!(split_tour(&trips[..5], DUTY_MAX), vec![0..5]);
        // no cut where the bus does not stand: one long duty
        let tight: Vec<PlannedTrip> = (0..12).map(|k| trip(300 + k * 60, 360 + k * 60, 12)).collect();
        assert_eq!(split_tour(&tight, DUTY_MAX), vec![0..12]);
        // a depot run at the end goes with the last part
        let mut with_run = trips.clone();
        with_run.push(trip(1390, 1400, 2));
        let d = split_tour(&with_run, DUTY_MAX);
        assert_eq!(d.last().unwrap().end, 19);
        assert!(split_tour(&[], DUTY_MAX).is_empty());
    }

    #[test]
    fn a_line_says_what_it_needs_against_what_the_company_has() {
        use super::super::concessions::tests::line;
        use super::super::{found, Founding};
        let c = found(&Founding { name: "Needs".into(), date: "2024-03-04".into(), ..Default::default() }, "Luc");
        // three tours at the same hours (four trips of an hour from 06:00): three buses at the
        // busiest, a duty each, five drivers or so; the company has none
        let l = line("Linie5", "5", 3, true);
        let n = needs(&c, &l, std::slice::from_ref(&l), &c.date);
        assert_eq!((n.buses, n.duties, n.drivers), (3, 3, 5));
        assert_eq!(n.sizes, vec![(BusSize::Solo, 3)]);
        assert_eq!(n.short(), (3, 5));
        // a day the line does not run: nothing
        let off = line("Linie6", "6", 2, false);
        assert_eq!(needs(&c, &off, &[], &c.date).buses, 0);
    }

    #[test]
    fn an_own_line_has_its_tours_every_day_it_runs() {
        // (one source of a day's tours: an own line's timetable is a line of the map's like
        // the others, its depot runs added)
        use super::super::concessions::tests::line;
        use super::super::{found, Founding};
        let mut c = found(&Founding { name: "Own".into(), date: "2024-03-04".into(), ..Default::default() }, "Luc");
        let own = crate::lines::OwnLine { number: "1".into(), colour: "#7b1fa2".into(), ..Default::default() };
        let mut l = line("oo_1", "1", 2, true);
        l.tours[1].ai_group = "Midibus".into();
        add_line(&mut c, &l, Some(&own)).unwrap();
        for k in 0..7 {
            let date = dates::add(&c.date, k);
            let tours = tours_of_day(&c, std::slice::from_ref(&l), &date);
            assert_eq!(tours.len(), 2, "{date}");
            assert!(tours.iter().all(|t| t.unplanned), "not planned yet");
        }
        start_service(&mut c, "oo_1", 0).unwrap();
        assert!(tours_of_day(&c, std::slice::from_ref(&l), &c.date.clone()).iter().all(|t| !t.unplanned));
    }

    #[test]
    fn a_group_name_asks_for_a_size() {
        assert_eq!(wants_size("Gelenkbus"), Some(BusSize::Articulated));
        assert_eq!(wants_size("Solo_Diesel"), Some(BusSize::Solo));
        assert_eq!(wants_size("BVG DD"), Some(BusSize::Double));
        assert_eq!(wants_size("Linie 5"), None);
    }
}
