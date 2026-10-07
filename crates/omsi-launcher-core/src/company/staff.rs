//! The company's people: the labour market of drivers (new applicants every week), hiring and
//! dismissing with notice, what a working day does to them (experience, satisfaction,
//! illness, holidays, leaving), and the working-time rules a duty must keep - Omsi-Hub's
//! `planregels.ts`: a duty of at most 9½ hours, a working day of at most 10 (overtime above
//! 8), 20 minutes between two pieces of work, 11 hours' rest overnight, a transfer to another
//! stop needs 45 minutes, and an articulated or double-decker bus wants some experience.

use super::dates;
use super::economy;
use super::licences::Endorsement;
use super::model::{BookingKind, BusSize, Cents, Company, Employee, Licence, Skills, Taken};
use super::rng::Rng;
use serde::{Deserialize, Serialize};

// --- working time (planregels.ts) ----------------------------------------------------------

/// A duty lasts at most this long (minutes); a longer tour is split.
pub const DUTY_MAX: i32 = 570;
/// A tour is split only where the bus stands at least this long.
pub const SPLIT_PAUSE: i32 = 3;
/// Rest between two pieces of work, overnight rest, time for a transfer between stops.
pub const REST: i32 = 20;
pub const NIGHT_REST: i32 = 660;
pub const TRANSFER: i32 = 45;
/// A working day's target (overtime above it) and its hard limit.
pub const DAY_TARGET: i32 = 480;
pub const DAY_MAX: i32 = 600;
/// Between two tours of the same bus.
pub const BUS_MARGIN: i32 = 10;
/// Under this experience an articulated bus or a double-decker is a warning.
pub const EXPERIENCE_LARGE: f64 = 25.0;
/// Days a week someone works at most.
pub const WEEK_DAYS: u32 = 5;
/// Holiday days a year (the German regional tariffs give about 30).
pub const HOLIDAYS: u32 = 30;

/// A piece of work of one driver or bus: minutes of the day, and the stops it begins and
/// ends at.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Block {
    pub key: String,
    pub from: i32,
    pub to: i32,
    pub from_stop: String,
    pub to_stop: String,
}

pub fn overlaps(a: &Block, b: &Block, margin: i32) -> bool {
    a.from < b.to + margin && b.from < a.to + margin
}

pub fn work_minutes(blocks: &[Block]) -> i32 {
    blocks.iter().map(|b| b.to - b.from).sum()
}

/// How a new piece of work fits what a driver has already (`toets`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Check {
    /// The key of a block it overlaps.
    pub double: Option<String>,
    /// The working day goes over `DAY_MAX`.
    pub too_long: bool,
    /// The shortest pause to a neighbouring block.
    pub rest: Option<i32>,
    /// That pause, when it is shorter than `TRANSFER` and the stops differ.
    pub transfer: Option<i32>,
    /// Minutes over `DAY_TARGET`.
    pub overtime: i32,
    /// The shortest rest against the day before (its last end, minutes of that day) or the
    /// day after (its first start).
    pub night_rest: Option<i32>,
}

impl Check {
    /// Allowed at all (the hard limits; the rest are warnings).
    pub fn allowed(&self) -> bool {
        self.double.is_none() && !self.too_long && self.rest.is_none_or(|r| r >= REST) && self.night_rest.is_none_or(|r| r >= NIGHT_REST)
    }
}

pub fn check(existing: &[Block], new: &Block, prev_end: Option<i32>, next_start: Option<i32>) -> Check {
    let others: Vec<&Block> = existing.iter().filter(|b| b.key != new.key).collect();
    let double = others.iter().find(|b| overlaps(b, new, 0)).map(|b| b.key.clone());
    let mut all: Vec<&Block> = others.clone();
    all.push(new);
    all.sort_by_key(|b| b.from);
    let work: i32 = all.iter().map(|b| b.to - b.from).sum();
    let at = all.iter().position(|b| std::ptr::eq(*b, new)).unwrap_or(0);
    let mut pairs = Vec::new();
    if at > 0 {
        pairs.push((all[at - 1], new));
    }
    if at + 1 < all.len() {
        pairs.push((new, all[at + 1]));
    }
    let (mut rest, mut transfer) = (None::<i32>, None::<i32>);
    for (a, b) in pairs {
        let pause = b.from - a.to;
        if pause < 0 {
            continue;
        }
        rest = Some(rest.map_or(pause, |r| r.min(pause)));
        if pause < TRANSFER && !a.to_stop.is_empty() && !b.from_stop.is_empty() && a.to_stop != b.from_stop {
            transfer = Some(transfer.map_or(pause, |t| t.min(pause)));
        }
    }
    let mut nights = Vec::new();
    if let Some(p) = prev_end {
        nights.push(1440 + all[0].from - p);
    }
    if let Some(n) = next_start {
        nights.push(1440 + n - all[all.len() - 1].to);
    }
    Check { double, too_long: work > DAY_MAX, rest, transfer, overtime: (work - DAY_TARGET).max(0), night_rest: nights.into_iter().min() }
}

/// May drive a bus of this size at all (the licence: D1 only midibuses).
pub fn may_drive(e: &Employee, size: BusSize) -> bool {
    e.licence == Licence::D || size == BusSize::Midi
}

/// Drives it without a warning (an articulated bus or a double-decker wants experience).
pub fn qualified(e: &Employee, size: BusSize) -> bool {
    may_drive(e, size) && (!matches!(size, BusSize::Articulated | BusSize::Double) || e.experience >= EXPERIENCE_LARGE)
}

// --- the labour market ---------------------------------------------------------------------

const FIRST: [&str; 40] = [
    "Anna", "Ben", "Carla", "Dennis", "Elif", "Frank", "Greta", "Hakan", "Ines", "Jan", "Klaus", "Lena", "Mehmet", "Nina", "Olaf", "Petra", "Rainer", "Sanne", "Tobias", "Ute", "Volker", "Wiebke", "Yusuf",
    "Zoë", "Bram", "Daan", "Emma", "Fleur", "Joost", "Marieke", "Pieter", "Sophie", "Agnieszka", "Piotr", "Olena", "Dmytro", "Marco", "Aylin", "Jonas", "Heike",
];
const LAST: [&str; 32] = [
    "Becker", "de Vries", "Fischer", "Hoffmann", "Jansen", "Kaya", "Krüger", "Meijer", "Müller", "Peters", "Richter", "Schmidt", "Schulz", "van Dijk", "Vogel", "Wagner", "Weber", "Wolf", "Yilmaz", "Zimmermann",
    "Bakker", "Visser", "Smit", "Mulder", "Kowalski", "Nowak", "Shevchenko", "Bondarenko", "Rossi", "Demir", "Lange", "Koch",
];

/// Someone applying this week.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Applicant {
    pub no: u32,
    pub name: String,
    pub age: u32,
    pub experience: f64,
    pub licence: Licence,
    /// The monthly gross wage they ask.
    pub wage: Cents,
    pub reliability: f64,
    pub skills: Skills,
    /// Endorsements they bring (`licences`).
    #[serde(default)]
    pub endorsements: Vec<Endorsement>,
}

/// The applicants of the company's week: the same all week, new ones on Monday, those
/// hired gone. More beginners than old hands, as on the real market; wages around the
/// market's for their experience.
pub fn applicants(c: &Company) -> Vec<Applicant> {
    let r = economy::rules(c.difficulty);
    let week = dates::week_of(&c.date);
    let mut rng = Rng::of(&[&c.id, "applicants"], week);
    let mut out = Vec::new();
    for no in 0..r.applicants as u32 {
        let name = format!("{} {}", rng.pick(&FIRST).unwrap_or(&"Alex"), rng.pick(&LAST).unwrap_or(&"Weber"));
        let experience = (rng.f64().powf(1.5) * 90.0 + 3.0).round();
        let age = (21.0 + experience * 0.35 + rng.range(0.0, 18.0)).round().min(63.0) as u32;
        let licence = if rng.chance(0.1) { Licence::D1 } else { Licence::D };
        let skill = |rng: &mut Rng| (35.0 + experience * 0.45 + rng.range(-15.0, 20.0)).clamp(10.0, 100.0).round();
        let skills = Skills { driving: skill(&mut rng), punctuality: skill(&mut rng), service: skill(&mut rng) };
        let reliability = (rng.range(0.72, 0.99) * 100.0).round() / 100.0;
        // (endorsements of their own, more with experience; each asks 3 % more)
        let mut endorsements = Vec::new();
        for (x, p) in [(Endorsement::Articulated, 0.25 + experience / 200.0), (Endorsement::DoubleDecker, 0.1 + experience / 400.0), (Endorsement::Electric, 0.15 + experience / 400.0)] {
            if licence == Licence::D && rng.chance(p) {
                endorsements.push(x);
            }
        }
        let wage = ((economy::market_wage(experience, c.price_index) as f64 * rng.range(0.94, 1.10) * (1.0 + 0.03 * endorsements.len() as f64) / 10_00 as f64).round() as Cents) * 10_00;
        let taken = c.taken.week == week && c.taken.applicants.contains(&no);
        if !taken {
            out.push(Applicant { no, name, age, experience, licence, wage, reliability, skills, endorsements });
        }
    }
    out
}

/// Hire an applicant of the week. Returns their id.
pub fn hire(c: &mut Company, a: &Applicant) -> Result<u32, &'static str> {
    let week = dates::week_of(&c.date);
    if c.taken.week == week && c.taken.applicants.contains(&a.no) {
        return Err("They have found work elsewhere.");
    }
    if c.taken.week != week {
        c.taken = Taken { week, used: Vec::new(), applicants: Vec::new() };
    }
    c.taken.applicants.push(a.no);
    c.counters.employee += 1;
    let id = c.counters.employee;
    // (the year's holidays pro rata)
    let left = dates::parse(&c.date).map(|d| dates::civil_from_days(d)).map(|(y, m, day)| {
        let year_end = dates::days_from_civil(y, 12, 31);
        let today = dates::days_from_civil(y, m, day);
        ((year_end - today + 1) as f64 / 365.0 * HOLIDAYS as f64).round() as u32
    });
    c.staff.push(Employee {
        id,
        name: a.name.clone(),
        age: a.age,
        experience: a.experience,
        licence: a.licence,
        wage: a.wage,
        reliability: a.reliability,
        skills: a.skills,
        satisfaction: 65.0,
        hired: c.date.clone(),
        notice_until: None,
        resigned: false,
        sick_until: None,
        holiday_until: None,
        training_until: None,
        holiday_left: left.unwrap_or(HOLIDAYS),
        week_days: 0,
        last_end: None,
        days_worked: 0,
        endorsements: a.endorsements.clone(),
        types: Vec::new(),
    });
    // (the induction: the fleet's models)
    super::licences::induction(c, id);
    Ok(id)
}

/// Dismiss someone: they work their notice and leave after it. Returns their last day.
pub fn dismiss(c: &mut Company, id: u32) -> Result<String, &'static str> {
    let r = economy::rules(c.difficulty);
    let today = c.date.clone();
    let Some(e) = c.staff.iter_mut().find(|e| e.id == id) else { return Err("They do not work here.") };
    if e.notice_until.is_some() {
        return Err("They are leaving already.");
    }
    let until = dates::add(&today, r.notice_days);
    e.notice_until = Some(until.clone());
    e.resigned = false;
    // (the others do not like it)
    for o in c.staff.iter_mut().filter(|o| o.id != id) {
        o.satisfaction = (o.satisfaction - 3.0).max(0.0);
    }
    Ok(until)
}

/// Take a dismissal back (not a resignation).
pub fn withdraw_notice(c: &mut Company, id: u32) -> Result<(), &'static str> {
    match c.staff.iter_mut().find(|e| e.id == id) {
        Some(e) if e.notice_until.is_some() && !e.resigned => {
            e.notice_until = None;
            Ok(())
        }
        Some(_) => Err("That cannot be taken back."),
        None => Err("They do not work here."),
    }
}

/// A raise of `share` (0.05 = 5 %), to the ten euros.
pub fn raise(c: &mut Company, id: u32, share: f64) {
    if let Some(e) = c.staff.iter_mut().find(|e| e.id == id) {
        e.wage = ((e.wage as f64 * (1.0 + share) / 10_00 as f64).round() as Cents) * 10_00;
        e.satisfaction = (e.satisfaction + 100.0 * share * 2.0).min(100.0);
    }
}

/// What someone costs a month (gross and the employer's share).
pub fn monthly_cost(e: &Employee) -> Cents {
    economy::employer_cost(e.wage)
}

/// What happened to someone tonight (for the day's report).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StaffNote {
    Sick { name: String, until: String },
    Holiday { name: String, until: String },
    Resigned { name: String, until: String },
    Unhappy { name: String },
    Left { name: String },
}

/// The wage for the days of `month` someone was employed (pro rata).
pub fn wage_for_month(e: &Employee, month: &str) -> Cents {
    let Some(first) = dates::parse(&format!("{month}-01")) else { return 0 };
    let (y, m, _) = dates::civil_from_days(first);
    let len = dates::days_in_month(y, m) as i64;
    let last = first + len - 1;
    let from = dates::parse(&e.hired).unwrap_or(first).max(first);
    let to = e.notice_until.as_deref().and_then(dates::parse).unwrap_or(last).min(last);
    let days = (to - from + 1).max(0);
    (economy::employer_cost(e.wage) as f64 * days as f64 / len as f64).round() as Cents
}

/// The night after a working day: experience for those who worked (`worked`: id, minutes and
/// the end of their last duty), satisfaction moving towards what their wage and their hours
/// make of it, illness and holidays, resignations, and those whose notice ends leave (their
/// last wage and, on Hard, a severance pay booked).
pub fn after_day(c: &mut Company, worked: &[(u32, i32, i32)], rng: &mut Rng) -> Vec<StaffNote> {
    let r = economy::rules(c.difficulty);
    let today = c.date.clone();
    let mut notes = Vec::new();
    let mut leaving = Vec::new();
    for e in c.staff.iter_mut() {
        let w = worked.iter().find(|w| w.0 == e.id);
        if let Some(&(_, minutes, end)) = w {
            e.experience = (e.experience + 0.12).min(100.0);
            e.week_days += 1;
            e.days_worked += 1;
            e.last_end = Some(end);
            let _ = minutes;
        } else {
            e.last_end = None;
        }
        let market = economy::market_wage(e.experience, c.price_index) as f64;
        let overtime = w.is_some_and(|w| w.1 > DAY_TARGET);
        let target = (60.0 + 150.0 * (e.wage as f64 / market - 1.0) - if overtime { 8.0 } else { 0.0 }).clamp(0.0, 100.0);
        let was = e.satisfaction;
        e.satisfaction = ((e.satisfaction + (target - e.satisfaction) * 0.1) * 10.0).round() / 10.0;
        if e.satisfaction < 35.0 && was >= 35.0 {
            notes.push(StaffNote::Unhappy { name: e.name.clone() });
        }
        let away = e.absent(&dates::add(&today, 1));
        if !away && e.notice_until.is_none() {
            if rng.chance(r.sickness * (1.6 - e.reliability)) {
                let until = dates::add(&today, rng.int(1, 7));
                e.sick_until = Some(until.clone());
                notes.push(StaffNote::Sick { name: e.name.clone(), until });
            } else if e.holiday_left >= 5 && rng.chance(0.012) {
                let days = rng.int(5, 10.min(e.holiday_left as i64)) as u32;
                let until = dates::add(&today, days as i64);
                e.holiday_until = Some(until.clone());
                e.holiday_left -= (days * 5).div_ceil(7);
                notes.push(StaffNote::Holiday { name: e.name.clone(), until });
            }
        }
        if e.notice_until.is_none() && e.satisfaction < 25.0 && rng.chance(0.03) {
            let until = dates::add(&today, 14);
            e.notice_until = Some(until.clone());
            e.resigned = true;
            notes.push(StaffNote::Resigned { name: e.name.clone(), until });
        }
        if e.notice_until.as_deref() == Some(today.as_str()) {
            leaving.push(e.id);
        }
    }
    for id in leaving {
        let Some(e) = c.staff.iter().find(|e| e.id == id).cloned() else { continue };
        c.book(BookingKind::Wages, -wage_for_month(&e, &dates::month_of(&today)), e.name.clone(), false);
        if !e.resigned && r.severance_months_per_year > 0.0 {
            let years = dates::years_between(&e.hired, &today);
            c.book(BookingKind::Severance, -(e.wage as f64 * r.severance_months_per_year * years).round() as Cents, e.name.clone(), false);
        }
        c.staff.retain(|x| x.id != id);
        notes.push(StaffNote::Left { name: e.name });
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::super::{found, Founding};
    use super::*;
    use crate::company::model::Difficulty;

    fn block(key: &str, from: i32, to: i32) -> Block {
        Block { key: key.into(), from, to, from_stop: "A".into(), to_stop: "B".into() }
    }

    #[test]
    fn the_working_time_rules_of_omsi_hub() {
        let morning = block("m", 6 * 60, 10 * 60);
        // overlapping
        assert_eq!(check(&[morning.clone()], &block("x", 9 * 60, 11 * 60), None, None).double.as_deref(), Some("m"));
        // a short pause and another stop: a transfer warning, but allowed
        let mut after = block("a", 10 * 60 + 25, 14 * 60);
        after.from_stop = "C".into();
        let k = check(&[morning.clone()], &after, None, None);
        assert_eq!((k.rest, k.transfer, k.too_long, k.allowed()), (Some(25), Some(25), false, true));
        // too short a rest
        assert!(!check(&[morning.clone()], &block("b", 10 * 60 + 10, 12 * 60), None, None).allowed());
        // a long day: overtime above 8 hours, too long above 10
        let k = check(&[morning.clone()], &block("c", 11 * 60, 15 * 60 + 30), None, None);
        assert_eq!((k.overtime, k.too_long), (30, false));
        assert!(check(&[morning.clone()], &block("d", 11 * 60, 17 * 60 + 30), None, None).too_long);
        // the night: yesterday ended at 22:00, today begins at 6:00 - only 8 hours
        let k = check(&[], &morning, Some(22 * 60), None);
        assert_eq!(k.night_rest, Some(480));
        assert!(!k.allowed());
        assert!(check(&[], &morning, Some(19 * 60), None).allowed());
    }

    #[test]
    fn hiring_dismissing_and_the_wages() {
        let mut c = found(&Founding { name: "Leute".into(), difficulty: Difficulty::Realistic, date: "2024-03-04".into(), ..Default::default() }, "Luc");
        let market = applicants(&c);
        assert_eq!(market.len(), 6);
        assert!(market.iter().all(|a| (2_500_00..4_500_00).contains(&a.wage) && (21..=63).contains(&a.age)));
        let a = market[0].clone();
        let id = hire(&mut c, &a).unwrap();
        assert_eq!(applicants(&c).len(), 5);
        assert!(hire(&mut c, &a).is_err());
        let e = c.employee(id).unwrap().clone();
        // hired on the 4th of March: the wage for 28 of 31 days
        assert_eq!(wage_for_month(&e, "2024-03"), (economy::employer_cost(e.wage) as f64 * 28.0 / 31.0).round() as Cents);
        assert_eq!(wage_for_month(&e, "2024-04"), economy::employer_cost(e.wage));
        assert_eq!(wage_for_month(&e, "2024-02"), 0);
        // dismissed: four weeks' notice on Realistic
        let until = dismiss(&mut c, id).unwrap();
        assert_eq!(until, "2024-04-01");
        assert!(c.employee(id).unwrap().employed_on("2024-04-01") && !c.employee(id).unwrap().employed_on("2024-04-02"));
        withdraw_notice(&mut c, id).unwrap();
        assert!(c.employee(id).unwrap().notice_until.is_none());
        // the licence and the experience for the big buses
        let mut e = c.employee(id).unwrap().clone();
        e.experience = 10.0;
        assert!(may_drive(&e, BusSize::Articulated) && !qualified(&e, BusSize::Articulated));
        e.licence = Licence::D1;
        assert!(!may_drive(&e, BusSize::Solo) && may_drive(&e, BusSize::Midi));
    }

    #[test]
    fn someone_whose_notice_ends_leaves_with_the_last_wage() {
        let mut c = found(&Founding { name: "Weg".into(), difficulty: Difficulty::Hard, date: "2024-03-04".into(), ..Default::default() }, "Luc");
        let a = applicants(&c)[0].clone();
        let id = hire(&mut c, &a).unwrap();
        c.staff[0].hired = "2022-03-04".into();
        dismiss(&mut c, id).unwrap();
        c.date = c.staff[0].notice_until.clone().unwrap();
        let cash = c.cash;
        let notes = after_day(&mut c, &[], &mut Rng::new(1));
        assert!(c.staff.is_empty());
        assert!(notes.contains(&StaffNote::Left { name: a.name.clone() }));
        // the wage of April's first day, and on Hard half a month's wage per year employed
        let severance = (a.wage as f64 * 0.5 * dates::years_between("2022-03-04", "2024-04-01")).round() as Cents;
        let wage = (economy::employer_cost(a.wage) as f64 / 30.0).round() as Cents;
        assert_eq!(cash - c.cash, severance + wage);
    }
}
