//! The company's level: experience points from the days it runs and from the player's own
//! tours, and the levels they climb, each opening something new (the Bus Company
//! Simulator's level system: new benefits and new areas of the company). The other parts of
//! the company ask `unlocked` whether a feature is open - the depot its halls and yards, the
//! market its bus sizes, the bank its rates - and `extra_places`, `max_concessions` and
//! `loan_discount` for what grows with the level.
//!
//! The day close hands each of the player's trips to `book_trip` (its fines are the
//! company's costs, a good tour earns a quality bonus on Realistic and Hard) and the day to
//! `day_closed` (the day's experience, the courses that end, what trained mechanics and eco
//! drivers save).

use super::career::{self, Evaluation};
use super::model::{BookingKind, Cents, Company, Difficulty};
use super::training;
use crate::TripRun;
use serde::{Deserialize, Serialize};

/// What the company keeps of its progress (`Company::progress`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Progress {
    pub xp: i64,
    /// The training courses booked, running and done.
    pub courses: Vec<training::Course>,
    pub next_course: u32,
    /// The player's own trips judged, the newest last (at most `JUDGED_KEPT`).
    pub judged: Vec<Judged>,
    /// All fines paid and bonuses earned.
    pub fines: Cents,
    pub bonuses: Cents,
    /// What the workshop's own work saved: the player's jobs and the mechanics.
    pub saved: Cents,
    /// The player's workshop jobs, the newest last.
    pub jobs: Vec<training::JobDone>,
    /// The highest level the pages have announced.
    pub level_seen: u32,
}

/// One of the player's trips as the company booked it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Judged {
    pub date: String,
    pub line: String,
    pub score: u32,
    pub fines: Cents,
    pub bonus: Cents,
    pub offences: u32,
}

pub const JUDGED_KEPT: usize = 300;

// --- the levels --------------------------------------------------------------------------------

/// The experience each level starts at (the first at 0).
pub const LEVEL_XP: [i64; 10] = [0, 600, 1_600, 3_200, 5_500, 8_500, 12_500, 18_000, 25_000, 34_000];

pub fn level_of(xp: i64) -> u32 {
    LEVEL_XP.iter().rposition(|x| xp >= *x).map(|i| i as u32 + 1).unwrap_or(1)
}

pub fn max_level() -> u32 {
    LEVEL_XP.len() as u32
}

/// The company's level now.
pub fn level(c: &Company) -> u32 {
    level_of(c.progress.xp)
}

/// The points into the level and the next level's start (None at the top).
pub fn progress(c: &Company) -> (i64, i64, Option<i64>) {
    let l = level(c) as usize;
    (c.progress.xp, LEVEL_XP[l - 1], LEVEL_XP.get(l).copied())
}

/// What the company's title is at a level.
pub fn title_of(level: u32) -> &'static str {
    match level {
        0..=1 => "Start-up",
        2 => "Local operator",
        3 => "Town operator",
        4..=5 => "Regional operator",
        6..=7 => "City operator",
        8..=9 => "Metropolitan operator",
        _ => "Transport group",
    }
}

/// What a level opens.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    /// The training centre: the first courses for the staff.
    TrainingCentre,
    /// The company's own workshop: the player's service jobs, the mechanics' course.
    Workshop,
    ArticulatedBuses,
    EcoCourse,
    ElectricBuses,
    /// The depot's second hall (more places, see `extra_places`).
    SecondHall,
    DoubleDeckers,
    /// Repairs by the player, the course for the big buses.
    AdvancedCourses,
    /// The defensive driving course (`incidents`).
    SafetyCourse,
    /// A lower rate on new loans.
    CheaperLoans,
    WashBay,
    ChargingYard,
    ThirdHall,
    BetterLoans,
    /// Advertising contracts: posters on the buses' rears, their side panels, whole buses
    /// wrapped (`adverts`).
    RearAdverts,
    SideAdverts,
    FullWraps,
}

impl Feature {
    pub const ALL: [Feature; 17] = [
        Feature::TrainingCentre,
        Feature::Workshop,
        Feature::ArticulatedBuses,
        Feature::EcoCourse,
        Feature::ElectricBuses,
        Feature::SecondHall,
        Feature::DoubleDeckers,
        Feature::AdvancedCourses,
        Feature::SafetyCourse,
        Feature::CheaperLoans,
        Feature::WashBay,
        Feature::ChargingYard,
        Feature::ThirdHall,
        Feature::BetterLoans,
        Feature::RearAdverts,
        Feature::SideAdverts,
        Feature::FullWraps,
    ];

    /// The level that opens it.
    pub fn level(self) -> u32 {
        match self {
            Feature::TrainingCentre => 1,
            Feature::Workshop | Feature::ArticulatedBuses | Feature::EcoCourse | Feature::RearAdverts => 2,
            Feature::ElectricBuses | Feature::SecondHall | Feature::SafetyCourse => 3,
            Feature::DoubleDeckers | Feature::AdvancedCourses | Feature::CheaperLoans | Feature::SideAdverts => 4,
            Feature::WashBay | Feature::ChargingYard => 5,
            Feature::ThirdHall | Feature::FullWraps => 6,
            Feature::BetterLoans => 7,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Feature::TrainingCentre => "Training centre",
            Feature::Workshop => "Own workshop",
            Feature::ArticulatedBuses => "Articulated buses",
            Feature::EcoCourse => "Eco driving course",
            Feature::ElectricBuses => "Electric buses",
            Feature::SecondHall => "Second depot hall",
            Feature::DoubleDeckers => "Double-deckers",
            Feature::AdvancedCourses => "Advanced courses",
            Feature::SafetyCourse => "Defensive driving course",
            Feature::CheaperLoans => "Cheaper loans",
            Feature::WashBay => "Wash bay",
            Feature::ChargingYard => "Charging yard",
            Feature::ThirdHall => "Third depot hall",
            Feature::BetterLoans => "Better loan terms",
            Feature::RearAdverts => "Rear adverts",
            Feature::SideAdverts => "Side adverts",
            Feature::FullWraps => "Full-wrap adverts",
        }
    }

    /// Its icon (a Material Symbol the interface has).
    pub fn icon(self) -> &'static str {
        match self {
            Feature::TrainingCentre => "badge",
            Feature::Workshop => "construction",
            Feature::ArticulatedBuses => "airport_shuttle",
            Feature::EcoCourse => "air",
            Feature::ElectricBuses => "bolt",
            Feature::SecondHall | Feature::ThirdHall => "garage",
            Feature::DoubleDeckers => "directions_bus",
            Feature::AdvancedCourses => "military_tech",
            Feature::SafetyCourse => "warning",
            Feature::CheaperLoans | Feature::BetterLoans => "payments",
            Feature::WashBay => "water_drop",
            Feature::ChargingYard => "power_settings_new",
            Feature::RearAdverts | Feature::SideAdverts | Feature::FullWraps => "campaign",
        }
    }
}

/// Is `feature` open to the company (its level has reached the feature's).
pub fn unlocked(c: &Company, feature: Feature) -> bool {
    level(c) >= feature.level()
}

/// What opens at a level.
pub fn opens_at(level: u32) -> Vec<Feature> {
    Feature::ALL.iter().copied().filter(|f| f.level() == level).collect()
}

/// More places in the depot than its first hall has: six with the second hall, eight more
/// with the third.
pub fn extra_places(c: &Company) -> u32 {
    (if unlocked(c, Feature::SecondHall) { 6 } else { 0 }) + if unlocked(c, Feature::ThirdHall) { 8 } else { 0 }
}

/// How many concessions (line contracts) the company may hold: one, and one more at levels
/// 3, 6, 8 and 10.
pub fn max_concessions(c: &Company) -> usize {
    let l = level(c);
    1 + [3, 6, 8, 10].iter().filter(|x| l >= **x).count()
}

/// What comes off a new loan's yearly rate (0.005 = half a point).
pub fn loan_discount(c: &Company) -> f64 {
    if unlocked(c, Feature::BetterLoans) {
        0.01
    } else if unlocked(c, Feature::CheaperLoans) {
        0.005
    } else {
        0.0
    }
}

/// The level the company has reached and not yet announced (the page says it once).
pub fn take_new_level(c: &mut Company) -> Option<u32> {
    let l = level(c);
    if l > c.progress.level_seen.max(1) {
        c.progress.level_seen = l;
        Some(l)
    } else {
        c.progress.level_seen = c.progress.level_seen.max(l);
        None
    }
}

// --- what the day close books -------------------------------------------------------------------

/// The quality bonus of a tour judged `score` (the authority's, on Realistic and Hard): €12
/// from 85 points, €20 from 95, half as much again on Hard (`book_trip` gives none to a tour
/// that was fined).
pub fn quality_bonus(d: Difficulty, score: u32, price_index: f64) -> Cents {
    let base = match score {
        95.. => 20_00,
        85..=94 => 12_00,
        _ => 0,
    } as f64;
    let factor = match d {
        Difficulty::Easy => 0.0,
        Difficulty::Realistic => 1.0,
        Difficulty::Hard => 1.5,
    };
    (base * factor * price_index.max(0.0)).round() as Cents
}

/// One of the player's own trips on a line of the company (`number`), as the day close books
/// it: its fines as the company's costs, the quality bonus, the experience. Returns its
/// evaluation.
pub fn book_trip(c: &mut Company, t: &TripRun, number: &str) -> Evaluation {
    let ev = career::evaluate(t);
    let fines = t.fines.max(0);
    let offences = (t.red_lights + t.speeding).max(0) as u32;
    if fines > 0 {
        c.book(BookingKind::Fine, -fines, format!("Line {number} (own trip)"), true);
        c.progress.fines += fines;
    }
    // (a tour that was fined or hit something earns no bonus, however punctual)
    let bonus = if offences == 0 && t.crashes <= 0 { quality_bonus(c.difficulty, ev.score, c.price_index) } else { 0 };
    if bonus > 0 {
        c.book(BookingKind::Bonus, bonus, format!("Line {number} (own trip)"), true);
        c.progress.bonuses += bonus;
    }
    c.progress.xp += ev.score as i64 / 2 + if ev.score >= 90 { 20 } else { 0 };
    let date = c.date.clone();
    c.progress.judged.push(Judged { date, line: number.to_string(), score: ev.score, fines, bonus, offences });
    if c.progress.judged.len() > JUDGED_KEPT {
        let extra = c.progress.judged.len() - JUDGED_KEPT;
        c.progress.judged.drain(..extra);
    }
    ev
}

/// The experience of a closed day: four a tour covered, one for every 25 passengers, 25 for a
/// punctual day (90 % and better; 10 from 80 %), two off for every trip dropped.
pub fn day_xp(covered: u32, dropped: u32, punctuality: Option<f64>, passengers: u32) -> i64 {
    let punctual = match punctuality {
        Some(p) if p >= 90.0 => 25,
        Some(p) if p >= 80.0 => 10,
        _ => 0,
    };
    (4 * covered as i64 + passengers as i64 / 25 + punctual - 2 * dropped as i64).max(0)
}

/// The day `date` closes (called by the day close before its figures are added up): the
/// day's experience, the courses that end, and what the workshop and the eco drivers saved.
pub fn day_closed(c: &mut Company, date: &str, covered: u32, dropped: u32, punctuality: Option<f64>, passengers: u32) {
    c.progress.xp += day_xp(covered, dropped, punctuality, passengers);
    training::day_passed(c, date);
    let spent = |c: &Company, kinds: &[BookingKind]| -> Cents { -c.ledger.iter().filter(|b| b.date == date && !b.measured && kinds.contains(&b.kind) && b.amount < 0).map(|b| b.amount).sum::<Cents>() };
    let workshop = (spent(c, &[BookingKind::Maintenance, BookingKind::Repair]) as f64 * training::mechanic_share(c, date)).round() as Cents;
    if workshop > 0 {
        c.book(BookingKind::Maintenance, workshop, "Own mechanics", false);
        c.progress.saved += workshop;
    }
    let eco = (spent(c, &[BookingKind::Energy]) as f64 * training::eco_share(c, date)).round() as Cents;
    if eco > 0 {
        c.book(BookingKind::Energy, eco, "Eco driving", false);
        c.progress.saved += eco;
    }
}

#[cfg(test)]
mod tests {
    use super::super::career::tests::trip;
    use super::super::{found, Founding};
    use super::*;

    fn company(d: Difficulty) -> Company {
        found(&Founding { name: "Stadtbus".into(), difficulty: d, date: "2024-05-06".into(), ..Default::default() }, "Luc")
    }

    #[test]
    fn levels_open_the_company_step_by_step() {
        let mut c = company(Difficulty::Realistic);
        assert_eq!((level(&c), title_of(1)), (1, "Start-up"));
        assert!(unlocked(&c, Feature::TrainingCentre) && !unlocked(&c, Feature::ArticulatedBuses) && !unlocked(&c, Feature::Workshop));
        assert_eq!((extra_places(&c), max_concessions(&c), loan_discount(&c)), (0, 1, 0.0));
        c.progress.xp = 600;
        assert_eq!(level(&c), 2);
        assert!(unlocked(&c, Feature::ArticulatedBuses) && unlocked(&c, Feature::Workshop) && !unlocked(&c, Feature::DoubleDeckers));
        assert_eq!(take_new_level(&mut c), Some(2));
        assert_eq!(take_new_level(&mut c), None);
        c.progress.xp = 5_500;
        assert_eq!(level(&c), 5);
        assert_eq!((extra_places(&c), max_concessions(&c), loan_discount(&c)), (6, 2, 0.005));
        c.progress.xp = 1_000_000;
        assert_eq!(level(&c), max_level());
        assert_eq!((extra_places(&c), max_concessions(&c), loan_discount(&c)), (14, 5, 0.01));
        assert_eq!(progress(&c).2, None);
        // every feature opens at some level, and that level lists it
        for f in Feature::ALL {
            assert!(opens_at(f.level()).contains(&f));
        }
    }

    #[test]
    fn the_players_tours_bring_fines_bonuses_and_experience() {
        let mut c = company(Difficulty::Hard);
        let cash = c.cash;
        let ev = book_trip(&mut c, &trip(20, 0), "5");
        assert_eq!(ev.score, 100);
        // the bonus: €20, half again on Hard
        assert_eq!(c.cash - cash, 30_00);
        assert_eq!(c.progress.xp, 70);
        let cash = c.cash;
        // a punctual tour with a red light: the fine, no bonus
        let ev = book_trip(&mut c, &TripRun { red_lights: 1, fines: 90_00, ..trip(20, 0) }, "5");
        assert!(ev.score >= 85);
        assert_eq!(c.cash - cash, -90_00);
        assert!(c.ledger.iter().any(|b| b.kind == BookingKind::Fine && b.amount == -90_00 && b.measured));
        assert_eq!((c.progress.fines, c.progress.bonuses, c.progress.judged.len()), (90_00, 30_00, 2));
        // no bonus on Easy
        assert_eq!(quality_bonus(Difficulty::Easy, 100, 1.0), 0);
        assert_eq!(quality_bonus(Difficulty::Realistic, 88, 1.1), 13_20);
    }

    #[test]
    fn a_day_brings_experience() {
        assert_eq!(day_xp(10, 0, Some(95.0), 500), 40 + 20 + 25);
        assert_eq!(day_xp(10, 3, Some(85.0), 0), 40 + 10 - 6);
        assert_eq!(day_xp(0, 30, None, 0), 0);
        let mut c = company(Difficulty::Realistic);
        day_closed(&mut c, "2024-05-06", 10, 0, Some(95.0), 500);
        assert_eq!(c.progress.xp, 85);
    }
}
