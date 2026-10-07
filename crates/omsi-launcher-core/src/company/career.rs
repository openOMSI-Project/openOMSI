//! The driver's career, beside the company's (Luc: the career is the driver's own and goes
//! with him from company to company): every trip judged at its end as the Bus Company
//! Simulator judges its tours - punctuality, the passengers' comfort and safety - into a
//! score, a grade and experience points; the levels and ranks those points climb; and the
//! licences for the big buses, each won with a driving test (Omsi-Hub's `exam.ts`: one trip,
//! judged on what can be measured fairly).
//!
//! The fines of the game's drive watch are worked out here as well (`speeding_fine`,
//! `red_light_fine`), so that the game, the trip card and the company's books agree on them.
//!
//! The career is saved per driver in `~/.openomsi/careers/<driver>.json`; what it is made of
//! besides that - the trips - stays in the trip records (`crate::trips_of`), so that it is
//! worked out again from them each time and never runs apart from them.

use super::model::{BusSize, Cents};
use crate::TripRun;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// --- fines -----------------------------------------------------------------------------------

/// What the drive watch fines.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OffenceKind {
    #[default]
    RedLight,
    Speeding,
}

/// One offence on a trip: a red light run or a speed camera that flashed.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Offence {
    pub kind: OffenceKind,
    /// Seconds into the trip.
    pub at: f64,
    /// The bus's speed (km/h) and, at a camera, the limit there.
    pub speed: f32,
    pub limit: f32,
    /// At a light: how long it had been red (s).
    pub red_for: f32,
    pub fine: Cents,
}

/// What a speed camera allows over the limit before it flashes: 3 km/h up to 100 km/h, 3 %
/// above (the German cameras' tolerance).
pub fn tolerance(speed: f32) -> f32 {
    if speed <= 100.0 {
        3.0
    } else {
        (speed * 0.03).ceil()
    }
}

/// How far over the limit a camera counts `speed` (km/h): after the tolerance, in whole
/// km/h; None within it.
pub fn over_limit(speed: f32, limit: f32) -> Option<f32> {
    if !(speed.is_finite() && limit > 0.0) {
        return None;
    }
    let counted = (speed - tolerance(speed)).floor();
    (counted > limit).then(|| counted - limit)
}

/// The fine for `over` km/h too fast (the German catalogue of 2021): in town (a limit up to
/// 50) or outside it.
pub fn speeding_fine(over: f32, town: bool) -> Cents {
    const TOWN: [(f32, Cents); 10] = [(10.0, 30), (15.0, 50), (20.0, 70), (25.0, 115), (30.0, 180), (40.0, 260), (50.0, 400), (60.0, 560), (70.0, 700), (f32::MAX, 800)];
    const OUT: [(f32, Cents); 10] = [(10.0, 20), (15.0, 40), (20.0, 60), (25.0, 100), (30.0, 150), (40.0, 200), (50.0, 320), (60.0, 480), (70.0, 600), (f32::MAX, 700)];
    if over <= 0.0 {
        return 0;
    }
    let table = if town { &TOWN } else { &OUT };
    table.iter().find(|(upto, _)| over <= *upto).map(|x| x.1).unwrap_or(800) * 100
}

/// A red light run: â‚¬90, â‚¬200 when it had been red for more than a second.
pub fn red_light_fine(red_for: f32) -> Cents {
    if red_for > 1.0 {
        200_00
    } else {
        90_00
    }
}

// --- a trip judged ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Grade {
    Excellent,
    Good,
    Fair,
    Poor,
    Bad,
}

impl Grade {
    pub fn of(score: u32) -> Grade {
        match score {
            90.. => Grade::Excellent,
            75..=89 => Grade::Good,
            60..=74 => Grade::Fair,
            40..=59 => Grade::Poor,
            _ => Grade::Bad,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Grade::Excellent => "Excellent",
            Grade::Good => "Good",
            Grade::Fair => "Fair",
            Grade::Poor => "Poor",
            Grade::Bad => "Bad",
        }
    }

    /// One to five stars.
    pub fn stars(self) -> u32 {
        match self {
            Grade::Excellent => 5,
            Grade::Good => 4,
            Grade::Fair => 3,
            Grade::Poor => 2,
            Grade::Bad => 1,
        }
    }
}

/// What a trip came to: its parts, the score they make (0 - 100), its grade and the
/// experience it earned.
#[derive(Clone, Debug, PartialEq)]
pub struct Evaluation {
    /// The stops served on time, in per cent (None on a free drive: no timetable).
    pub punctuality: Option<u32>,
    /// What the passengers felt: hard braking, hard starts, jerky stops, jolts.
    pub comfort: u32,
    /// Red lights, speed cameras and collisions.
    pub safety: u32,
    pub score: u32,
    pub grade: Grade,
    pub xp: i64,
}

/// What each comfort or safety event takes off its part (of 100).
const HARD_BRAKE: f64 = 10.0;
const HARD_START: f64 = 6.0;
const ROUGH_STOP: f64 = 8.0;
const JOLT: f64 = 5.0;
const RED_LIGHT: f64 = 25.0;
const SPEEDING: f64 = 15.0;
const CRASH: f64 = 30.0;

/// Judge a trip. The punctuality weighs 40 %, comfort and safety 30 % each (a free drive:
/// half and half); a trip not driven to its last stop loses ten points.
pub fn evaluate(t: &TripRun) -> Evaluation {
    let part = |minus: f64| (100.0 - minus).clamp(0.0, 100.0);
    let comfort = part(HARD_BRAKE * t.hard_brakes.max(0) as f64 + HARD_START * t.hard_starts.max(0) as f64 + ROUGH_STOP * t.rough_stops.max(0) as f64 + JOLT * t.jolts.max(0) as f64);
    let safety = part(RED_LIGHT * t.red_lights.max(0) as f64 + SPEEDING * t.speeding.max(0) as f64 + CRASH * t.crashes.max(0) as f64);
    let punctuality = t.timed().then(|| 100.0 * t.on_time() as f64 / t.stops.max(1) as f64);
    let mut score = match punctuality {
        Some(p) => 0.4 * p + 0.3 * comfort + 0.3 * safety,
        None => 0.5 * comfort + 0.5 * safety,
    };
    if !t.completed {
        score -= 10.0;
    }
    let score = score.clamp(0.0, 100.0).round() as u32;
    let grade = Grade::of(score);
    Evaluation { punctuality: punctuality.map(|p| p.round() as u32), comfort: comfort.round() as u32, safety: safety.round() as u32, score, grade, xp: trip_xp(t, score) }
}

/// The kilometres of a trip that can have been driven in its time (as the company's books
/// believe them: at most 100 km/h, any distance up to 10 km).
pub fn believable_km(t: &TripRun) -> f64 {
    let km = t.metres / 1000.0;
    if km.is_finite() && km > 0.0 && km <= (t.seconds / 3600.0 * 100.0).max(10.0) {
        km
    } else {
        0.0
    }
}

/// Experience of a trip: eight a kilometre, four a stop, one a passenger (up to two hundred)
/// and twenty for driving it to its end, weighed by the score (a quarter at 0, all of it at
/// 100); an excellent trip earns forty on top.
pub fn trip_xp(t: &TripRun, score: u32) -> i64 {
    let base = 8.0 * believable_km(t) + 4.0 * t.stops.max(0) as f64 + t.passengers.clamp(0, 200) as f64 + if t.completed { 20.0 } else { 0.0 };
    let weighed = base * (0.25 + 0.75 * score.min(100) as f64 / 100.0);
    weighed.round() as i64 + if score >= 90 { 40 } else { 0 }
}

// --- levels and ranks ------------------------------------------------------------------------

pub const MAX_LEVEL: u32 = 30;

/// The experience a driver needs for level `n` (250 n (n - 1): 500 for the second, 1 500 for
/// the third, 22 500 for the tenth).
pub fn xp_for_level(n: u32) -> i64 {
    let n = n.clamp(1, MAX_LEVEL) as i64;
    250 * n * (n - 1)
}

pub fn level_of(xp: i64) -> u32 {
    (1..=MAX_LEVEL).rev().find(|n| xp >= xp_for_level(*n)).unwrap_or(1)
}

/// The rank a level gives (its English name: a key of the translations).
pub fn rank_of(level: u32) -> &'static str {
    match level {
        0..=2 => "Trainee driver",
        3..=5 => "Bus driver",
        6..=9 => "Experienced driver",
        10..=14 => "Senior driver",
        15..=19 => "Driving instructor",
        _ => "Master driver",
    }
}

/// How far a driver is: the level, the points into it and those the next one needs (None at
/// the last), as a share for a bar.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelProgress {
    pub xp: i64,
    pub level: u32,
    pub floor: i64,
    pub next: Option<i64>,
}

impl LevelProgress {
    pub fn of(xp: i64) -> LevelProgress {
        let level = level_of(xp);
        LevelProgress { xp, level, floor: xp_for_level(level), next: (level < MAX_LEVEL).then(|| xp_for_level(level + 1)) }
    }

    pub fn share(&self) -> f64 {
        match self.next {
            Some(n) if n > self.floor => ((self.xp - self.floor) as f64 / (n - self.floor) as f64).clamp(0.0, 1.0),
            _ => 1.0,
        }
    }
}

// --- licences and the driving test -----------------------------------------------------------

/// The buses a driver may drive: a midibus and a solo bus with the bus licence every driver
/// starts with, the articulated bus and the double-decker each after a driving test.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum LicenceClass {
    Midi,
    Solo,
    Articulated,
    Double,
}

impl LicenceClass {
    pub const ALL: [LicenceClass; 4] = [LicenceClass::Midi, LicenceClass::Solo, LicenceClass::Articulated, LicenceClass::Double];

    pub fn of(size: BusSize) -> LicenceClass {
        match size {
            BusSize::Midi => LicenceClass::Midi,
            BusSize::Solo => LicenceClass::Solo,
            BusSize::Articulated => LicenceClass::Articulated,
            BusSize::Double => LicenceClass::Double,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LicenceClass::Midi => "Midibus",
            LicenceClass::Solo => "Solo bus",
            LicenceClass::Articulated => "Articulated bus",
            LicenceClass::Double => "Double-decker",
        }
    }

    /// Held from the start (no test).
    pub fn basic(self) -> bool {
        matches!(self, LicenceClass::Midi | LicenceClass::Solo)
    }

    /// The driver level the test can be booked from.
    pub fn level(self) -> u32 {
        match self {
            LicenceClass::Midi | LicenceClass::Solo => 1,
            LicenceClass::Articulated => 4,
            LicenceClass::Double => 6,
        }
    }

    /// What a pass is worth.
    pub fn xp(self) -> i64 {
        match self {
            LicenceClass::Articulated => 400,
            LicenceClass::Double => 600,
            _ => 0,
        }
    }
}

/// One rule of the test, as measured and its limit.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Criterion {
    /// "finish", "punctual", "smooth", "safe".
    pub rule: String,
    pub passed: bool,
    pub value: f64,
    pub limit: f64,
}

/// A driving test taken.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ExamRecord {
    pub class: LicenceClass,
    /// When its trip ended (Unix time).
    pub time: u64,
    pub line: String,
    pub passed: bool,
    pub score: u32,
    pub criteria: Vec<Criterion>,
}

/// The limits of the test: generous enough to pass, strict enough to mean something
/// (Omsi-Hub's: three minutes off the timetable on average, five rough moments in all).
pub const EXAM_DELAY: f64 = 180.0;
pub const EXAM_ROUGH: f64 = 5.0;

/// Judge the trip of a driving test: driven to its end, punctual (on average within three
/// minutes, at most a quarter of the stops late), smooth (five rough moments at most) and
/// safe (no red light, no camera, no collision). All four must be met; the score counts
/// them equally.
pub fn judge_exam(t: &TripRun) -> (bool, u32, Vec<Criterion>) {
    let rough = (t.hard_brakes + t.hard_starts + t.rough_stops + t.jolts).max(0) as f64;
    let delay = t.average.unwrap_or(0.0).abs();
    let late_share = if t.stops > 0 { t.late as f64 / t.stops as f64 } else { 0.0 };
    let offences = (t.red_lights + t.speeding + t.crashes).max(0) as f64;
    let criteria = vec![
        Criterion { rule: "finish".into(), passed: t.completed && t.timed(), value: if t.completed { 1.0 } else { 0.0 }, limit: 1.0 },
        Criterion { rule: "punctual".into(), passed: delay <= EXAM_DELAY && late_share <= 0.25, value: delay.round(), limit: EXAM_DELAY },
        Criterion { rule: "smooth".into(), passed: rough <= EXAM_ROUGH, value: rough, limit: EXAM_ROUGH },
        Criterion { rule: "safe".into(), passed: offences == 0.0, value: offences, limit: 0.0 },
    ];
    let good = criteria.iter().filter(|c| c.passed).count();
    let score = (100 * good / criteria.len()) as u32;
    (good == criteria.len(), score, criteria)
}

/// A test booked: the next trip with a bus of its class after `since` is its trip.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Booking {
    pub class: LicenceClass,
    pub since: u64,
}

/// What a driver's career keeps besides the trips.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct DriverCareer {
    pub driver: String,
    /// The licences won with a test (the basic ones are always held).
    pub licences: Vec<LicenceClass>,
    pub booked: Option<Booking>,
    pub exams: Vec<ExamRecord>,
}

impl DriverCareer {
    pub fn holds(&self, class: LicenceClass) -> bool {
        class.basic() || self.licences.contains(&class)
    }

    /// The experience of the tests passed.
    pub fn exam_xp(&self) -> i64 {
        self.exams.iter().filter(|e| e.passed).map(|e| e.class.xp()).sum()
    }
}

/// May the driver take a bus of this size on a duty.
pub fn may_drive(c: &DriverCareer, size: BusSize) -> bool {
    c.holds(LicenceClass::of(size))
}

/// Book the test for `class` (`level`: the driver's level now, `now`: Unix time).
pub fn book_exam(c: &mut DriverCareer, class: LicenceClass, level: u32, now: u64) -> Result<(), &'static str> {
    if c.holds(class) {
        return Err("You hold this licence already.");
    }
    if level < class.level() {
        return Err("Your driver level is too low for this test.");
    }
    if c.booked.is_some() {
        return Err("A driving test is booked already.");
    }
    c.booked = Some(Booking { class, since: now });
    Ok(())
}

pub fn cancel_exam(c: &mut DriverCareer) {
    c.booked = None;
}

/// The test booked, judged once its trip is driven: the first trip after the booking with a
/// bus of its class (`size_of`: the size of a bus file) that ran to a timetable. Returns the
/// test taken (a pass adds the licence).
pub fn check_exam(c: &mut DriverCareer, trips: &[TripRun], size_of: &dyn Fn(&str) -> BusSize) -> Option<ExamRecord> {
    let b = c.booked.clone()?;
    let mut mine: Vec<&TripRun> = trips.iter().filter(|t| t.time > b.since && !t.free && t.stops > 0 && LicenceClass::of(size_of(&t.bus)) == b.class).collect();
    mine.sort_by_key(|t| t.time);
    let t = mine.first()?;
    let (passed, score, criteria) = judge_exam(t);
    let rec = ExamRecord { class: b.class, time: t.time, line: t.line.clone(), passed, score, criteria };
    if passed && !c.licences.contains(&b.class) {
        c.licences.push(b.class);
    }
    c.booked = None;
    c.exams.insert(0, rec.clone());
    c.exams.truncate(50);
    Some(rec)
}

// --- what a driver's trips come to -------------------------------------------------------------

/// A driver's career at a glance, from the trips and the career file.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub progress: LevelProgress,
    pub rank: &'static str,
    pub trips: usize,
    /// The trips the drive watch saw (their comfort and safety are known).
    pub watched: usize,
    pub km: f64,
    pub hours: f64,
    pub passengers: i64,
    /// Stops on time of those served, in per cent.
    pub punctuality: Option<f64>,
    pub average_score: Option<f64>,
    pub best_score: Option<u32>,
    pub fines: Cents,
    pub red_lights: i64,
    pub speeding: i64,
    pub excellent: usize,
}

pub fn summary(trips: &[TripRun], c: &DriverCareer) -> Summary {
    let evals: Vec<Evaluation> = trips.iter().map(evaluate).collect();
    let xp: i64 = evals.iter().map(|e| e.xp).sum::<i64>() + c.exam_xp();
    let progress = LevelProgress::of(xp);
    let stops: i64 = trips.iter().filter(|t| t.timed()).map(|t| t.stops as i64).sum();
    let on_time: i64 = trips.iter().filter(|t| t.timed()).map(|t| t.on_time() as i64).sum();
    let scores: Vec<u32> = evals.iter().map(|e| e.score).collect();
    Summary {
        rank: rank_of(progress.level),
        progress,
        trips: trips.len(),
        watched: trips.iter().filter(|t| t.watched).count(),
        km: trips.iter().map(believable_km).sum(),
        hours: trips.iter().map(|t| t.seconds.max(0.0)).sum::<f64>() / 3600.0,
        passengers: trips.iter().map(|t| t.passengers.max(0) as i64).sum(),
        punctuality: (stops > 0).then(|| 100.0 * on_time as f64 / stops as f64),
        average_score: (!scores.is_empty()).then(|| scores.iter().map(|s| *s as f64).sum::<f64>() / scores.len() as f64),
        best_score: scores.iter().copied().max(),
        fines: trips.iter().map(|t| t.fines.max(0)).sum(),
        red_lights: trips.iter().map(|t| t.red_lights.max(0) as i64).sum(),
        speeding: trips.iter().map(|t| t.speeding.max(0) as i64).sum(),
        excellent: evals.iter().filter(|e| e.grade == Grade::Excellent).count(),
    }
}

// --- on the disk -------------------------------------------------------------------------------

/// `careers/<driver>.json` in the data folder (the name as the trips file has it).
pub fn path_of(data: &Path, driver: &str) -> PathBuf {
    let name: String = driver.trim().to_lowercase().chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' }).collect();
    let name = if name.trim().is_empty() { "driver".to_string() } else { name };
    data.join("careers").join(format!("{name}.json"))
}

/// The driver's career (a new one when there is none).
pub fn load(data: &Path, driver: &str) -> DriverCareer {
    let mut c: DriverCareer = std::fs::read_to_string(path_of(data, driver)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    c.driver = driver.trim().to_string();
    c
}

pub fn save(data: &Path, c: &DriverCareer) -> anyhow::Result<()> {
    let path = path_of(data, &c.driver);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(c)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn trip(stops: i32, late: i32) -> TripRun {
        TripRun { time: 1_700_000_000, line: "5".into(), stops, planned: stops, late, completed: true, seconds: 1500.0, metres: 10_000.0, passengers: 30, watched: true, average: Some(30.0), ..Default::default() }
    }

    #[test]
    fn speed_cameras_fine_as_the_catalogue_does() {
        // within the tolerance: no flash
        assert_eq!(over_limit(53.9, 50.0), None);
        assert_eq!(over_limit(54.0, 50.0), Some(1.0));
        assert_eq!(over_limit(66.5, 50.0), Some(13.0));
        // over 100 km/h the tolerance is 3 %
        assert_eq!(tolerance(120.0), 4.0);
        assert_eq!(over_limit(110.0, 100.0), Some(6.0));
        assert_eq!(speeding_fine(1.0, true), 30_00);
        assert_eq!(speeding_fine(13.0, true), 50_00);
        assert_eq!(speeding_fine(13.0, false), 40_00);
        assert_eq!(speeding_fine(26.0, true), 180_00);
        assert_eq!(speeding_fine(90.0, true), 800_00);
        assert_eq!(speeding_fine(0.0, true), 0);
        assert_eq!(red_light_fine(0.6), 90_00);
        assert_eq!(red_light_fine(2.5), 200_00);
    }

    #[test]
    fn a_trip_is_judged_on_punctuality_comfort_and_safety() {
        let good = evaluate(&trip(20, 0));
        assert_eq!((good.punctuality, good.comfort, good.safety, good.score, good.grade), (Some(100), 100, 100, 100, Grade::Excellent));
        // 10 km, 20 stops, 30 passengers, to the end: (80 + 80 + 30 + 20), all of it, +40
        assert_eq!(good.xp, 250);
        // a red light, a camera, two hard brakes and a rough stop
        let bad = evaluate(&TripRun { red_lights: 1, speeding: 1, hard_brakes: 2, rough_stops: 1, late: 5, ..trip(20, 5) });
        assert_eq!((bad.punctuality, bad.comfort, bad.safety), (Some(75), 72, 60));
        assert_eq!(bad.score, (0.4f64 * 75.0 + 0.3 * 72.0 + 0.3 * 60.0).round() as u32);
        assert_eq!(bad.grade, Grade::Fair);
        assert!(bad.xp < good.xp && bad.xp > 0);
        // a free drive has no punctuality; one given up loses ten points
        let free = evaluate(&TripRun { free: true, hard_starts: 2, ..trip(8, 0) });
        assert_eq!((free.punctuality, free.score), (None, 94));
        let left = evaluate(&TripRun { completed: false, ..trip(20, 0) });
        assert_eq!(left.score, 90);
        // a trip of an old game without the watch: its jolts and collisions only
        let old = evaluate(&TripRun { watched: false, jolts: 2, crashes: 1, ..trip(10, 0) });
        assert_eq!((old.comfort, old.safety), (90, 70));
        // a broken odometer earns nothing for its kilometres
        assert_eq!(believable_km(&TripRun { metres: 9e9, ..trip(1, 0) }), 0.0);
    }

    #[test]
    fn levels_and_ranks_climb_with_the_points() {
        assert_eq!((level_of(0), level_of(499), level_of(500), level_of(1500), level_of(22_500)), (1, 1, 2, 3, 10));
        assert_eq!(level_of(i64::MAX / 4), MAX_LEVEL);
        assert_eq!((rank_of(1), rank_of(3), rank_of(10), rank_of(30)), ("Trainee driver", "Bus driver", "Senior driver", "Master driver"));
        let p = LevelProgress::of(1000);
        assert_eq!((p.level, p.floor, p.next), (2, 500, Some(1500)));
        assert!((p.share() - 0.5).abs() < 1e-9);
        assert_eq!(LevelProgress::of(xp_for_level(MAX_LEVEL)).share(), 1.0);
    }

    #[test]
    fn a_driving_test_wins_the_licence() {
        let mut c = DriverCareer { driver: "Luc".into(), ..Default::default() };
        assert!(may_drive(&c, BusSize::Solo) && !may_drive(&c, BusSize::Articulated));
        assert!(book_exam(&mut c, LicenceClass::Articulated, 2, 100).is_err());
        assert!(book_exam(&mut c, LicenceClass::Solo, 9, 100).is_err());
        book_exam(&mut c, LicenceClass::Articulated, 4, 1_700_000_000).unwrap();
        assert!(book_exam(&mut c, LicenceClass::Double, 9, 100).is_err());
        let size = |bus: &str| if bus.contains("GN") { BusSize::Articulated } else { BusSize::Solo };
        // a trip with a solo bus, or before the booking, is not the test
        let solo = TripRun { time: 1_700_000_100, bus: "Vehicles/SD/SD.bus".into(), ..trip(12, 0) };
        let before = TripRun { time: 1_600_000_000, bus: "Vehicles/GN/GN.bus".into(), ..trip(12, 0) };
        assert!(check_exam(&mut c, &[solo.clone(), before], &size).is_none());
        // failed: a red light
        let red = TripRun { time: 1_700_000_200, bus: "Vehicles/GN/GN.bus".into(), red_lights: 1, ..trip(12, 0) };
        let r = check_exam(&mut c, &[red.clone(), solo], &size).unwrap();
        assert!(!r.passed && r.score == 75 && !c.holds(LicenceClass::Articulated) && c.booked.is_none());
        assert!(!r.criteria.iter().find(|x| x.rule == "safe").unwrap().passed);
        // booked again and passed
        book_exam(&mut c, LicenceClass::Articulated, 4, 1_700_000_300).unwrap();
        let fine = TripRun { time: 1_700_000_400, bus: "Vehicles/GN/GN.bus".into(), hard_brakes: 2, ..trip(12, 1) };
        let r = check_exam(&mut c, &[fine, red], &size).unwrap();
        assert!(r.passed && r.score == 100);
        assert!(may_drive(&c, BusSize::Articulated) && c.exam_xp() == 400 && c.exams.len() == 2);
        // too rough
        let (ok, _, _) = judge_exam(&TripRun { hard_brakes: 3, hard_starts: 2, jolts: 1, ..trip(12, 0) });
        assert!(!ok);
    }

    #[test]
    fn the_career_adds_up_its_trips_and_is_saved() {
        let trips = vec![trip(20, 0), TripRun { fines: 70_00, speeding: 1, ..trip(10, 2) }, TripRun { free: true, ..trip(5, 0) }];
        let c = DriverCareer::default();
        let s = summary(&trips, &c);
        assert_eq!((s.trips, s.watched, s.speeding, s.fines), (3, 3, 1, 70_00));
        assert_eq!(s.punctuality, Some(100.0 * 28.0 / 30.0));
        assert!(s.progress.xp > 500 && s.best_score == Some(100));
        let data = std::env::temp_dir().join(format!("omsi-career-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let mut c = load(&data, "Luc");
        assert_eq!(c.driver, "Luc");
        c.licences.push(LicenceClass::Double);
        save(&data, &c).unwrap();
        assert_eq!(load(&data, "LUC").licences, vec![LicenceClass::Double]);
        let _ = std::fs::remove_dir_all(&data);
    }
}
