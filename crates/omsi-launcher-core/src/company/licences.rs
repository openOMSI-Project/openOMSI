//! Licences and type training (Luc: "personeel moet worden opgeleid en rijbewijzen krijgen; als
//! je een nieuwe bus hebt, moet het personeel er een cursus voor kunnen volgen"): a driver is
//! planned only on a bus he is qualified for.
//!
//! - *The licence*: D drives every bus, D1 only midibuses (`staff::may_drive`); a D1 driver
//!   gets the D in a course of ten days.
//! - *Endorsements* (what an operator asks besides the licence): an articulated bus, a
//!   double-decker, and the high-voltage instruction for an electric bus. Each is a course of
//!   a day or two, opened by the company level that opens such buses.
//! - *Type training* per bus model (its family: the buses of one folder of `Vehicles`): a
//!   day with the model's controls, doors, kneeling, ramps and faults. A model new to the
//!   company needs it before its drivers take it; drivers hired later learn the fleet's
//!   models in their induction; the dealer's extra "Introduction" trains two drivers on
//!   delivery.
//!
//! Applicants bring endorsements of their own (and ask more for them). A company from before
//! the licences gives its people what the fleet they drive asks (`grant_fleet`, `store`).
//! Courses (Realistic, net, founding day's prices; Easy 0.7, Hard 1.3 times): the D licence
//! €4,500, an articulated or a double-decker endorsement €900, the high-voltage instruction
//! €600, a type training €250.

use super::dates;
use super::model::{BookingKind, BusKind, BusSize, Cents, Company, Difficulty, Drive, Employee, Licence};
use super::training::{Course, CourseKind};
use serde::{Deserialize, Serialize};

/// What an operator asks of a driver besides the licence for some buses.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Endorsement {
    Articulated,
    DoubleDecker,
    Electric,
}

impl Endorsement {
    pub const ALL: [Endorsement; 3] = [Endorsement::Articulated, Endorsement::DoubleDecker, Endorsement::Electric];

    pub fn label(self) -> &'static str {
        match self {
            Endorsement::Articulated => "Articulated buses",
            Endorsement::DoubleDecker => "Double-deckers",
            Endorsement::Electric => "High voltage (electric buses)",
        }
    }

    /// A short mark for lists ("G", "DD", "E").
    pub fn short(self) -> &'static str {
        match self {
            Endorsement::Articulated => "G",
            Endorsement::DoubleDecker => "DD",
            Endorsement::Electric => "E",
        }
    }

    /// The course that gives it.
    pub fn course(self) -> CourseKind {
        match self {
            Endorsement::Articulated => CourseKind::ArticulatedLicence,
            Endorsement::DoubleDecker => CourseKind::DoubleDeckerLicence,
            Endorsement::Electric => CourseKind::HighVoltage,
        }
    }

    /// Why someone without it may not drive such a bus.
    pub fn lacking(self) -> &'static str {
        match self {
            Endorsement::Articulated => "No licence for articulated buses",
            Endorsement::DoubleDecker => "No licence for double-deckers",
            Endorsement::Electric => "No high-voltage instruction for electric buses",
        }
    }
}

/// The endorsements a bus of `kind` asks for.
pub fn needed(kind: BusKind) -> Vec<Endorsement> {
    let mut v = Vec::new();
    match kind.size {
        BusSize::Articulated => v.push(Endorsement::Articulated),
        BusSize::Double => v.push(Endorsement::DoubleDecker),
        _ => {}
    }
    if kind.drive == Drive::Electric {
        v.push(Endorsement::Electric);
    }
    v
}

/// The model family of a bus file: its folder of `Vehicles` (lower case), its versions and
/// doors together.
pub fn type_key(bus: &str) -> String {
    let f = bus.trim().replace('\\', "/");
    let parts: Vec<&str> = f.split('/').filter(|p| !p.is_empty()).collect();
    let k = match parts.iter().position(|p| p.eq_ignore_ascii_case("vehicles")) {
        Some(i) if i + 2 < parts.len() => parts[i + 1],
        _ if parts.len() >= 2 => parts[parts.len() - 2],
        _ => parts.last().copied().unwrap_or(""),
    };
    k.to_lowercase()
}

/// A model family's name as the fleet calls it (the name of one of its buses, else its key).
pub fn type_name(c: &Company, key: &str) -> String {
    c.fleet.iter().find(|v| type_key(&v.bus) == key).map(|v| v.name.clone()).unwrap_or_else(|| key.to_string())
}

/// The model families of the fleet (held now), in the order the buses came.
pub fn fleet_types(c: &Company) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in c.fleet.iter().filter(|v| v.held_on(&c.date)) {
        let k = type_key(&v.bus);
        if !k.is_empty() && !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

/// What a driver lacks for a bus.
#[derive(Clone, Debug, PartialEq)]
pub enum Lack {
    /// A D1 licence and a bus bigger than a midibus.
    Licence,
    Endorsement(Endorsement),
    /// No type training for the model family (its key).
    Type(String),
}

/// What `e` lacks to drive a bus of `kind` (`bus`: its file, for the type training; None: a
/// bus not known yet, a rental) - None: qualified.
pub fn lack(e: &Employee, kind: BusKind, bus: Option<&str>) -> Option<Lack> {
    if e.licence == Licence::D1 && kind.size != BusSize::Midi {
        return Some(Lack::Licence);
    }
    if let Some(x) = needed(kind).into_iter().find(|x| !e.endorsements.contains(x)) {
        return Some(Lack::Endorsement(x));
    }
    let key = bus.map(type_key).filter(|k| !k.is_empty())?;
    (!e.types.iter().any(|t| *t == key)).then_some(Lack::Type(key))
}

/// The drivers (employed, not leaving) who may drive the bus file `bus` of `kind`, and of how
/// many.
pub fn qualified_drivers(c: &Company, kind: BusKind, bus: &str) -> (usize, usize) {
    let staff: Vec<&Employee> = c.staff.iter().filter(|e| e.employed_on(&c.date) && e.notice_until.is_none()).collect();
    (staff.iter().filter(|e| lack(e, kind, Some(bus)).is_none()).count(), staff.len())
}

fn factor(d: Difficulty) -> f64 {
    match d {
        Difficulty::Easy => 0.7,
        Difficulty::Realistic => 1.0,
        Difficulty::Hard => 1.3,
    }
}

/// A licence course's cost now (the course's own at founding prices, by difficulty).
pub fn course_cost(c: &Company, kind: CourseKind) -> Cents {
    ((kind.spec().cost as f64 * factor(c.difficulty) * c.price_index.max(0.0) / 10_00 as f64).round() as Cents) * 10_00
}

/// Someone has the course's licence, endorsement or type already.
pub fn has(e: &Employee, kind: CourseKind, subject: &str) -> bool {
    match kind {
        CourseKind::LicenceD => e.licence == Licence::D,
        CourseKind::ArticulatedLicence => e.endorsements.contains(&Endorsement::Articulated),
        CourseKind::DoubleDeckerLicence => e.endorsements.contains(&Endorsement::DoubleDecker),
        CourseKind::HighVoltage => e.endorsements.contains(&Endorsement::Electric),
        CourseKind::TypeTraining => e.types.iter().any(|t| t == subject),
        _ => false,
    }
}

/// A licence course or a type training (`subject`: the model family) running for someone.
pub fn booked<'a>(c: &'a Company, employee: u32, kind: CourseKind, subject: &str) -> Option<&'a Course> {
    c.progress.courses.iter().find(|x| x.employee == Some(employee) && x.kind == kind && !x.done && (kind != CourseKind::TypeTraining || x.subject == subject))
}

/// Book a licence course or a type training (`subject`: the model family's key) for a driver:
/// paid and begun today, the driver away for its days. Returns its last day.
pub fn enrol(c: &mut Company, employee: u32, kind: CourseKind, subject: &str) -> Result<String, &'static str> {
    if !kind.is_licence() {
        return Err("This course is not for them.");
    }
    if !super::levels::unlocked(c, kind.spec().feature) {
        return Err("Your company's level does not offer this course yet.");
    }
    let Some(e) = c.employee(employee).cloned() else { return Err("This person does not work here.") };
    if kind == CourseKind::TypeTraining && subject.trim().is_empty() {
        return Err("Which bus is the training for?");
    }
    if has(&e, kind, subject) {
        return Err("This course is done already.");
    }
    if booked(c, employee, kind, subject).is_some() {
        return Err("This course is booked already.");
    }
    // (an articulated bus or a double-decker asks the D licence under the endorsement)
    if matches!(kind, CourseKind::ArticulatedLicence | CourseKind::DoubleDeckerLicence) && e.licence == Licence::D1 {
        return Err("They need the D licence first.");
    }
    let today = c.date.clone();
    let until = dates::add(&today, kind.spec().days as i64 - 1);
    if !e.employed_on(&until) || e.notice_until.is_some() {
        return Err("They are leaving the company.");
    }
    if e.training_until.as_deref().is_some_and(|u| dates::between(&today, u) >= 0) {
        return Err("They are on another course.");
    }
    if e.absent(&today) {
        return Err("They are away today.");
    }
    let cost = course_cost(c, kind);
    if c.cash < cost {
        return Err("Not enough money for the course.");
    }
    let what = if kind == CourseKind::TypeTraining { format!("{} {} ({})", kind.label(), type_name(c, subject), e.name) } else { format!("{} ({})", kind.label(), e.name) };
    c.book(BookingKind::Training, -cost, what, false);
    if let Some(x) = c.staff.iter_mut().find(|x| x.id == employee) {
        x.training_until = Some(until.clone());
    }
    c.progress.next_course += 1;
    let id = c.progress.next_course;
    c.progress.courses.push(Course { id, kind, employee: Some(employee), name: e.name.clone(), from: today, until: until.clone(), cost, done: false, subject: subject.to_string() });
    Ok(until)
}

/// Book a course for every driver who has neither it nor a booking for it and can go today
/// (the cash allowing). Returns how many were booked, and what it cost.
pub fn enrol_all(c: &mut Company, kind: CourseKind, subject: &str) -> (usize, Cents) {
    let ids: Vec<u32> = c.staff.iter().map(|e| e.id).collect();
    let (mut n, mut spent) = (0, 0);
    for id in ids {
        let before = c.cash;
        if enrol(c, id, kind, subject).is_ok() {
            n += 1;
            spent += before - c.cash;
        }
    }
    (n, spent)
}

/// A licence course or type training finished: what it gives.
pub fn learnt(e: &mut Employee, kind: CourseKind, subject: &str) {
    let mut add = |x: Endorsement| {
        if !e.endorsements.contains(&x) {
            e.endorsements.push(x);
            e.endorsements.sort();
        }
    };
    match kind {
        CourseKind::LicenceD => e.licence = Licence::D,
        CourseKind::ArticulatedLicence => add(Endorsement::Articulated),
        CourseKind::DoubleDeckerLicence => add(Endorsement::DoubleDecker),
        CourseKind::HighVoltage => add(Endorsement::Electric),
        CourseKind::TypeTraining => {
            if !subject.is_empty() && !e.types.iter().any(|t| t == subject) {
                e.types.push(subject.to_string());
            }
        }
        _ => {}
    }
}

/// A new driver's induction: the model families of the fleet.
pub fn induction(c: &mut Company, employee: u32) {
    let types = fleet_types(c);
    if let Some(e) = c.staff.iter_mut().find(|e| e.id == employee) {
        for t in types {
            if !e.types.contains(&t) {
                e.types.push(t);
            }
        }
    }
}

/// The dealer's introduction on delivery: the type training of `bus` for the two most
/// experienced drivers without it, free. Returns their names.
pub fn introduction(c: &mut Company, bus: &str) -> Vec<String> {
    let key = type_key(bus);
    let mut who: Vec<(f64, u32)> = c.staff.iter().filter(|e| e.employed_on(&c.date) && !e.types.contains(&key)).map(|e| (e.experience, e.id)).collect();
    who.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut names = Vec::new();
    for (_, id) in who.into_iter().take(2) {
        if let Some(e) = c.staff.iter_mut().find(|e| e.id == id) {
            e.types.push(key.clone());
            names.push(e.name.clone());
        }
    }
    names
}

/// A company from before the licences: its people drive what they drove - every endorsement
/// and model family its fleet asks for.
pub fn grant_fleet(c: &mut Company) {
    let types = fleet_types(c);
    let mut ends: Vec<Endorsement> = c.fleet.iter().flat_map(|v| needed(v.kind)).collect();
    ends.sort();
    ends.dedup();
    for e in c.staff.iter_mut() {
        for x in &ends {
            if !e.endorsements.contains(x) {
                e.endorsements.push(*x);
            }
        }
        e.endorsements.sort();
        for t in &types {
            if !e.types.contains(t) {
                e.types.push(t.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::market::{self, MarketBus, Payment};
    use super::super::staff::{applicants, hire};
    use super::super::{found, store, Founding};
    use super::*;

    fn company() -> Company {
        let mut c = found(&Founding { name: "Schule".into(), date: "2024-03-04".into(), ..Default::default() }, "Luc");
        c.cash += 5_000_000_00;
        c.progress.xp = super::super::levels::LEVEL_XP[4];
        c
    }

    fn bus(c: &mut Company, file: &str, size: BusSize, drive: Drive) -> u32 {
        let b = MarketBus { file: file.into(), name: file.rsplit('/').next().unwrap_or("Bus").trim_end_matches(".bus").into(), kind: BusKind { size, drive }, ..Default::default() };
        market::buy_new(c, &b, Payment::Cash, "").unwrap()
    }

    #[test]
    fn a_driver_is_qualified_by_licence_endorsement_and_type() {
        let mut e = super::super::plan::agency_driver();
        (e.endorsements, e.types) = (Vec::new(), Vec::new());
        let solo = BusKind { size: BusSize::Solo, drive: Drive::Diesel };
        let art = BusKind { size: BusSize::Articulated, drive: Drive::Diesel };
        let ebus = BusKind { size: BusSize::Solo, drive: Drive::Electric };
        assert_eq!(type_key("Vehicles/MB_Citaro/Citaro_3T.bus"), "mb_citaro");
        assert_eq!(type_key("vehicles\\MAN_NL\\x.bus"), "man_nl");
        assert_eq!(lack(&e, solo, Some("Vehicles/MB_Citaro/C.bus")), Some(Lack::Type("mb_citaro".into())));
        assert_eq!(lack(&e, art, None), Some(Lack::Endorsement(Endorsement::Articulated)));
        assert_eq!(lack(&e, ebus, None), Some(Lack::Endorsement(Endorsement::Electric)));
        e.types.push("mb_citaro".into());
        assert_eq!(lack(&e, solo, Some("Vehicles/MB_Citaro/C2.bus")), None, "the family's other versions too");
        assert_eq!(lack(&e, solo, None), None);
        e.licence = Licence::D1;
        assert_eq!(lack(&e, solo, None), Some(Lack::Licence));
        assert_eq!(lack(&e, BusKind { size: BusSize::Midi, drive: Drive::Diesel }, None), None);
        learnt(&mut e, CourseKind::LicenceD, "");
        learnt(&mut e, CourseKind::ArticulatedLicence, "");
        assert_eq!(lack(&e, art, None), None);
    }

    #[test]
    fn courses_teach_and_new_drivers_learn_the_fleet() {
        let mut c = company();
        let citaro = bus(&mut c, "Vehicles/MB_Citaro/Citaro.bus", BusSize::Solo, Drive::Diesel);
        // hired after the Citaro came: it is in the induction
        let a = applicants(&c)[0].clone();
        let id = hire(&mut c, &a).unwrap();
        assert!(c.employee(id).unwrap().types.contains(&"mb_citaro".to_string()));
        let kind = c.vehicle(citaro).unwrap().kind;
        assert_eq!(qualified_drivers(&c, kind, "Vehicles/MB_Citaro/Citaro.bus").0, 1);
        // a new model: nobody may drive it until the type training
        let lion = bus(&mut c, "Vehicles/MAN_Lion/Lion.bus", BusSize::Solo, Drive::Diesel);
        let lk = c.vehicle(lion).unwrap().kind;
        assert_eq!(qualified_drivers(&c, lk, "Vehicles/MAN_Lion/Lion.bus").0, 0);
        let cash = c.cash;
        let until = enrol(&mut c, id, CourseKind::TypeTraining, "man_lion").unwrap();
        assert_eq!(until, c.date, "a day");
        assert_eq!(cash - c.cash, 250_00);
        assert_eq!(enrol(&mut c, id, CourseKind::TypeTraining, "man_lion"), Err("This course is booked already."));
        assert!(c.employee(id).unwrap().absent(&c.date));
        let today = c.date.clone();
        super::super::training::day_passed(&mut c, &today);
        assert_eq!(qualified_drivers(&c, lk, "Vehicles/MAN_Lion/Lion.bus").0, 1);
        assert_eq!(enrol(&mut c, id, CourseKind::TypeTraining, "man_lion"), Err("This course is done already."));
        // an endorsement for several at once
        c.date = dates::add(&c.date, 1);
        let (n, spent) = enrol_all(&mut c, CourseKind::HighVoltage, "");
        let has_it = c.staff.iter().filter(|e| e.endorsements.contains(&Endorsement::Electric)).count();
        assert_eq!(n + has_it, c.staff.len());
        assert_eq!(spent, n as Cents * course_cost(&c, CourseKind::HighVoltage));
        // the dealer's introduction: two drivers, free
        let a2 = applicants(&c)[1].clone();
        hire(&mut c, &a2).unwrap();
        let names = introduction(&mut c, "Vehicles/Solaris/Urbino.bus");
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn an_older_company_keeps_driving_what_it_drove() {
        let mut c = company();
        bus(&mut c, "Vehicles/MB_Citaro/CitaroG.bus", BusSize::Articulated, Drive::Diesel);
        let a = applicants(&c)[0].clone();
        let id = hire(&mut c, &a).unwrap();
        // (a file of before: nothing known of licences)
        if let Some(e) = c.staff.iter_mut().find(|e| e.id == id) {
            (e.endorsements, e.types) = (Vec::new(), Vec::new());
        }
        c.quals = 0;
        let c = store::migrate(c);
        let e = c.employee(id).unwrap();
        assert!(e.endorsements.contains(&Endorsement::Articulated) && e.types.contains(&"mb_citaro".to_string()));
        assert_eq!(c.quals, 1);
    }
}
