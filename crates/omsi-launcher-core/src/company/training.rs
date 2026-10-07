//! Training: courses for the staff (punctuality, customer service, eco driving, defensive
//! driving, first aid, ticket sales, the workshop, the big buses - what they change of the
//! day: `day`, `incidents`), each costing money and days away, and the two workshop courses
//! of the player himself, who may then service and repair the company's buses with his own
//! hands; and those hands' work: the workshop task, a short game in the launcher - diagnose
//! the bus's parts and fix what is wrong before the time runs out - whose quality decides
//! what the job saves (the Bus Company Simulator's repair and maintenance mini-games).
//!
//! The courses run in company days: booked today, the person is away from today on for the
//! course's days (`Employee::training_until`) and has learnt it the night its last day
//! closes (`day_passed`).

use super::dates;
use super::levels::{self, Feature};
use super::market;
use super::model::{BookingKind, Cents, Company, Vehicle};
use super::rng::Rng;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CourseKind {
    Punctuality,
    EcoDriving,
    CustomerService,
    /// Defensive driving: half the accidents and traffic fines (`incidents`).
    Safety,
    /// First aid: a passenger taken ill or hurt on board costs less reputation.
    FirstAid,
    /// Ticket sales and fare checks: more fares on their trips.
    Ticketing,
    /// A mechanic: some of the maintenance and repairs are done in the company's own
    /// workshop (`mechanic_share`).
    Workshop,
    /// The articulated bus and the double-decker without a warning.
    LargeBuses,
    /// The player: services in the workshop.
    PlayerService,
    /// The player: repairs after a breakdown.
    PlayerRepairs,
    /// The licences and the type training (`licences`): the D licence for a D1 driver, the
    /// articulated and the double-decker endorsement, the high-voltage instruction, and the
    /// type training of a model family (`Course::subject`).
    LicenceD,
    ArticulatedLicence,
    DoubleDeckerLicence,
    HighVoltage,
    TypeTraining,
}

/// What a course is: its cost (at founding prices), its days, what opens it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
    pub kind: CourseKind,
    pub cost: Cents,
    pub days: u32,
    pub feature: Feature,
    pub for_player: bool,
}

impl CourseKind {
    pub const STAFF: [CourseKind; 8] = [CourseKind::Punctuality, CourseKind::CustomerService, CourseKind::Ticketing, CourseKind::FirstAid, CourseKind::EcoDriving, CourseKind::Safety, CourseKind::Workshop, CourseKind::LargeBuses];
    pub const PLAYER: [CourseKind; 2] = [CourseKind::PlayerService, CourseKind::PlayerRepairs];

    pub fn spec(self) -> Spec {
        let (cost, days, feature, for_player) = match self {
            CourseKind::Punctuality => (650_00, 2, Feature::TrainingCentre, false),
            CourseKind::CustomerService => (480_00, 1, Feature::TrainingCentre, false),
            CourseKind::EcoDriving => (900_00, 2, Feature::EcoCourse, false),
            CourseKind::Safety => (700_00, 2, Feature::SafetyCourse, false),
            CourseKind::FirstAid => (300_00, 1, Feature::TrainingCentre, false),
            CourseKind::Ticketing => (350_00, 1, Feature::TrainingCentre, false),
            CourseKind::Workshop => (2_400_00, 5, Feature::Workshop, false),
            CourseKind::LargeBuses => (1_500_00, 3, Feature::AdvancedCourses, false),
            CourseKind::PlayerService => (1_200_00, 2, Feature::Workshop, true),
            CourseKind::PlayerRepairs => (2_000_00, 3, Feature::AdvancedCourses, true),
            CourseKind::LicenceD => (4_500_00, 10, Feature::TrainingCentre, false),
            CourseKind::ArticulatedLicence => (900_00, 2, Feature::ArticulatedBuses, false),
            CourseKind::DoubleDeckerLicence => (900_00, 2, Feature::DoubleDeckers, false),
            CourseKind::HighVoltage => (600_00, 1, Feature::ElectricBuses, false),
            CourseKind::TypeTraining => (250_00, 1, Feature::TrainingCentre, false),
        };
        Spec { kind: self, cost, days, feature, for_player }
    }

    pub fn label(self) -> &'static str {
        match self {
            CourseKind::Punctuality => "Punctuality",
            CourseKind::CustomerService => "Customer service",
            CourseKind::EcoDriving => "Eco driving",
            CourseKind::Safety => "Defensive driving",
            CourseKind::FirstAid => "First aid",
            CourseKind::Ticketing => "Ticket sales",
            CourseKind::Workshop => "Workshop skills",
            CourseKind::LargeBuses => "Large buses",
            CourseKind::PlayerService => "Workshop basics",
            CourseKind::PlayerRepairs => "Repairs",
            CourseKind::LicenceD => "D licence",
            CourseKind::ArticulatedLicence => "Articulated licence",
            CourseKind::DoubleDeckerLicence => "Double-decker licence",
            CourseKind::HighVoltage => "High-voltage instruction",
            CourseKind::TypeTraining => "Type training",
        }
    }

    /// The licence courses (`licences::enrol`), booked per driver on the Staff page.
    pub const LICENCES: [CourseKind; 4] = [CourseKind::LicenceD, CourseKind::ArticulatedLicence, CourseKind::DoubleDeckerLicence, CourseKind::HighVoltage];

    /// A licence, an endorsement or a type training.
    pub fn is_licence(self) -> bool {
        matches!(self, CourseKind::LicenceD | CourseKind::ArticulatedLicence | CourseKind::DoubleDeckerLicence | CourseKind::HighVoltage | CourseKind::TypeTraining)
    }

    /// What it does, in a line.
    pub fn effect(self) -> &'static str {
        match self {
            CourseKind::Punctuality => "Punctuality +15: fewer late trips",
            CourseKind::CustomerService => "Service +15 and half the complaints: more passengers, a better reputation",
            CourseKind::EcoDriving => "Driving +8, and 8 % less fuel and power on their share of the tours",
            CourseKind::Safety => "Half the accidents and traffic fines on their trips",
            CourseKind::FirstAid => "A passenger taken ill or hurt on board costs far less reputation",
            CourseKind::Ticketing => "Fewer fare dodgers: 4 % more fares on their trips",
            CourseKind::Workshop => "A mechanic: 6 % of maintenance and repairs done in-house (up to four), a broken-down bus back a day sooner",
            CourseKind::LargeBuses => "Articulated buses and double-deckers without a warning",
            CourseKind::PlayerService => "You may service the company's buses yourself",
            CourseKind::PlayerRepairs => "You may repair broken-down buses yourself",
            CourseKind::LicenceD => "A D1 driver may drive every bus",
            CourseKind::ArticulatedLicence => "May drive articulated buses",
            CourseKind::DoubleDeckerLicence => "May drive double-deckers",
            CourseKind::HighVoltage => "May drive electric buses",
            CourseKind::TypeTraining => "May drive the buses of one model",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            CourseKind::Punctuality => "schedule",
            CourseKind::CustomerService => "group",
            CourseKind::EcoDriving => "air",
            CourseKind::Safety => "warning",
            CourseKind::FirstAid => "emergency_home",
            CourseKind::Ticketing => "confirmation_number",
            CourseKind::Workshop => "settings",
            CourseKind::LargeBuses => "airport_shuttle",
            CourseKind::PlayerService => "tune",
            CourseKind::PlayerRepairs => "construction",
            CourseKind::LicenceD => "badge",
            CourseKind::ArticulatedLicence => "airport_shuttle",
            CourseKind::DoubleDeckerLicence => "directions_bus",
            CourseKind::HighVoltage => "bolt",
            CourseKind::TypeTraining => "key",
        }
    }
}

/// A course booked.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Course {
    pub id: u32,
    pub kind: CourseKind,
    /// Who takes it (None: the player).
    pub employee: Option<u32>,
    pub name: String,
    /// The days away, both included.
    pub from: String,
    pub until: String,
    pub cost: Cents,
    pub done: bool,
    /// A type training's model family (`licences::type_key`).
    #[serde(default)]
    pub subject: String,
}

/// What a course costs the company now.
pub fn cost_of(c: &Company, kind: CourseKind) -> Cents {
    ((kind.spec().cost as f64 * c.price_index.max(0.0) / 10_00 as f64).round() as Cents) * 10_00
}

/// The course of this kind someone has booked (`employee` None: the player), running or done.
pub fn course_of(c: &Company, employee: Option<u32>, kind: CourseKind) -> Option<&Course> {
    c.progress.courses.iter().find(|x| x.employee == employee && x.kind == kind)
}

/// Has done the course.
pub fn trained(c: &Company, employee: Option<u32>, kind: CourseKind) -> bool {
    course_of(c, employee, kind).is_some_and(|x| x.done)
}

/// The player may service (`PlayerService`) or repair (`PlayerRepairs`) the buses himself.
pub fn player_can(c: &Company, kind: CourseKind) -> bool {
    levels::unlocked(c, Feature::Workshop) && trained(c, None, kind)
}

/// Book a course for `employee` (None: the player): paid and begun today. Returns its last
/// day.
pub fn enrol(c: &mut Company, kind: CourseKind, employee: Option<u32>) -> Result<String, &'static str> {
    // (a licence: per driver, with its own rules)
    if kind.is_licence() {
        return match employee {
            Some(id) => super::licences::enrol(c, id, kind, ""),
            None => Err("This course is not for them."),
        };
    }
    let spec = kind.spec();
    if spec.for_player != employee.is_none() {
        return Err("This course is not for them.");
    }
    if !levels::unlocked(c, spec.feature) {
        return Err("Your company's level does not offer this course yet.");
    }
    if let Some(x) = course_of(c, employee, kind) {
        return Err(if x.done { "This course is done already." } else { "This course is booked already." });
    }
    let today = c.date.clone();
    let from = today.clone();
    let until = dates::add(&from, spec.days as i64 - 1);
    let name = match employee {
        Some(id) => {
            let Some(e) = c.employee(id) else { return Err("This person does not work here.") };
            if !e.employed_on(&until) || e.notice_until.is_some() {
                return Err("They are leaving the company.");
            }
            if e.training_until.as_deref().is_some_and(|u| dates::between(&today, u) >= 0) {
                return Err("They are on another course.");
            }
            if e.absent(&today) {
                return Err("They are away today.");
            }
            e.name.clone()
        }
        None => String::new(),
    };
    let cost = cost_of(c, kind);
    if c.cash < cost {
        return Err("Not enough money for the course.");
    }
    let who = if name.is_empty() { kind.label().to_string() } else { format!("{} ({name})", kind.label()) };
    c.book(BookingKind::Training, -cost, who, false);
    if let Some(e) = employee.and_then(|id| c.staff.iter_mut().find(|e| e.id == id)) {
        e.training_until = Some(until.clone());
    }
    c.progress.next_course += 1;
    let id = c.progress.next_course;
    c.progress.courses.push(Course { id, kind, employee, name, from, until: until.clone(), cost, done: false, subject: String::new() });
    Ok(until)
}

/// Book a staff course for everyone who has neither done nor booked it and can go today (the
/// cash allowing). Returns how many were booked, and what it cost.
pub fn enrol_all(c: &mut Company, kind: CourseKind) -> (usize, Cents) {
    let ids: Vec<u32> = c.staff.iter().map(|e| e.id).collect();
    let (mut n, mut spent) = (0, 0);
    for id in ids {
        let before = c.cash;
        if enrol(c, kind, Some(id)).is_ok() {
            n += 1;
            spent += before - c.cash;
        }
    }
    (n, spent)
}

/// The night after `date`: the courses whose last day it was are learnt. Returns them.
pub fn day_passed(c: &mut Company, date: &str) -> Vec<Course> {
    let mut finished = Vec::new();
    for k in 0..c.progress.courses.len() {
        let x = &c.progress.courses[k];
        if x.done || dates::between(&x.until, date) < 0 {
            continue;
        }
        c.progress.courses[k].done = true;
        let x = c.progress.courses[k].clone();
        if let Some(e) = x.employee.and_then(|id| c.staff.iter_mut().find(|e| e.id == id)) {
            match x.kind {
                CourseKind::Punctuality => e.skills.punctuality = (e.skills.punctuality + 15.0).min(100.0),
                CourseKind::CustomerService => e.skills.service = (e.skills.service + 15.0).min(100.0),
                CourseKind::EcoDriving => e.skills.driving = (e.skills.driving + 8.0).min(100.0),
                CourseKind::LargeBuses => e.experience = e.experience.max(super::staff::EXPERIENCE_LARGE + 5.0).min(100.0),
                _ => {}
            }
            super::licences::learnt(e, x.kind, &x.subject);
            // (people like being trained)
            e.satisfaction = (e.satisfaction + 4.0).min(100.0);
            e.training_until = None;
        }
        c.progress.xp += 15;
        finished.push(x);
    }
    finished
}

/// The employees trained as mechanics working on `date` (at most four count).
pub fn mechanics(c: &Company, date: &str) -> usize {
    c.staff.iter().filter(|e| e.employed_on(date) && trained(c, Some(e.id), CourseKind::Workshop)).count().min(4)
}

/// The share of the maintenance and repairs the company's own mechanics do (6 % each).
pub fn mechanic_share(c: &Company, date: &str) -> f64 {
    if !levels::unlocked(c, Feature::Workshop) {
        return 0.0;
    }
    0.06 * mechanics(c, date) as f64
}

/// What eco driving saves of the fuel and power: 8 % on the share of the drivers trained.
pub fn eco_share(c: &Company, date: &str) -> f64 {
    let drivers: Vec<_> = c.staff.iter().filter(|e| e.employed_on(date)).collect();
    if drivers.is_empty() {
        return 0.0;
    }
    let eco = drivers.iter().filter(|e| trained(c, Some(e.id), CourseKind::EcoDriving)).count();
    0.08 * eco as f64 / drivers.len() as f64
}

// --- the workshop task --------------------------------------------------------------------------

/// The parts the player looks at.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Part {
    Lights,
    Wipers,
    Doors,
    Engine,
    Brakes,
    Tyres,
    Suspension,
    Battery,
}

impl Part {
    pub const ALL: [Part; 8] = [Part::Lights, Part::Wipers, Part::Doors, Part::Engine, Part::Brakes, Part::Tyres, Part::Suspension, Part::Battery];

    pub fn label(self) -> &'static str {
        match self {
            Part::Lights => "Lights",
            Part::Wipers => "Wipers",
            Part::Doors => "Doors",
            Part::Engine => "Engine oil",
            Part::Brakes => "Brakes",
            Part::Tyres => "Tyres",
            Part::Suspension => "Suspension",
            Part::Battery => "Battery",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Part::Lights => "light_mode",
            Part::Wipers => "water_drop",
            Part::Doors => "door_sliding",
            Part::Engine => "thermostat",
            Part::Brakes => "stop_circle",
            Part::Tyres => "autorenew",
            Part::Suspension => "unfold_more",
            Part::Battery => "bolt",
        }
    }

    /// What a check finds: fine, or one of its two faults (and the fix each wants).
    pub fn reading(self, fault: Option<Fix>) -> &'static str {
        match (self, fault) {
            (Part::Lights, None) => "All lamps work, the headlights are aimed right",
            (Part::Lights, Some(Fix::Replace)) => "The left headlight is out",
            (Part::Lights, Some(_)) => "The headlights are aimed too high",
            (Part::Wipers, None) => "The blades wipe clean, the washer is full",
            (Part::Wipers, Some(Fix::Replace)) => "The blades are torn and streak",
            (Part::Wipers, Some(_)) => "The washer fluid is empty",
            (Part::Doors, None) => "The doors open and close smoothly",
            (Part::Doors, Some(Fix::Replace)) => "The seal of door 2 is torn",
            (Part::Doors, Some(_)) => "Door 2 closes slowly and jerks",
            (Part::Engine, None) => "The oil is clean and at the mark",
            (Part::Engine, Some(Fix::Replace)) => "The oil is black and past its interval",
            (Part::Engine, Some(_)) => "The oil is below the minimum",
            (Part::Brakes, None) => "The pads have 9 mm left",
            (Part::Brakes, Some(Fix::Replace)) => "The pads are down to 3 mm",
            (Part::Brakes, Some(_)) => "The brake travel is too long",
            (Part::Tyres, None) => "Tread 8 mm, pressure 8.5 bar",
            (Part::Tyres, Some(Fix::Replace)) => "The front tread is down to 1.5 mm",
            (Part::Tyres, Some(_)) => "The pressure is 6 bar instead of 8.5",
            (Part::Suspension, None) => "The bellows hold, the ride height is even",
            (Part::Suspension, Some(Fix::Replace)) => "A rear air bellows leaks",
            (Part::Suspension, Some(_)) => "The bus leans to one side",
            (Part::Battery, None) => "27.6 V while charging: good",
            (Part::Battery, Some(Fix::Replace)) => "The battery holds no charge any more",
            (Part::Battery, Some(_)) => "The battery's acid is low",
        }
    }

    /// The fix the part's second fault wants (the first always wants a new part).
    fn second_fix(self) -> Fix {
        match self {
            Part::Wipers | Part::Engine | Part::Tyres | Part::Battery => Fix::TopUp,
            _ => Fix::Adjust,
        }
    }
}

/// What the player does to a part.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Fix {
    Replace,
    Adjust,
    TopUp,
    /// It is fine as it is.
    Leave,
}

impl Fix {
    pub const ALL: [Fix; 4] = [Fix::Replace, Fix::Adjust, Fix::TopUp, Fix::Leave];

    pub fn label(self) -> &'static str {
        match self {
            Fix::Replace => "Replace",
            Fix::Adjust => "Adjust",
            Fix::TopUp => "Top up",
            Fix::Leave => "It is fine",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Service,
    Repair,
}

impl JobKind {
    /// The seconds the task gives.
    pub fn seconds(self) -> f32 {
        match self {
            JobKind::Service => 75.0,
            JobKind::Repair => 90.0,
        }
    }
}

/// One part on the bench: what is wrong with it (the fix it wants), whether the player has
/// looked at it and what he did.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub part: Part,
    pub fault: Option<Fix>,
    pub inspected: bool,
    pub done: Option<Fix>,
}

/// A workshop task: a bus's parts, some of them faulty.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub kind: JobKind,
    pub vehicle: u32,
    pub checks: Vec<Check>,
}

impl Job {
    /// The task for a bus today: a service finds two to four faults (more the worse its
    /// condition), a repair four or five. The same bus on the same day gets the same task.
    pub fn new(c: &Company, v: &Vehicle, kind: JobKind) -> Job {
        let mut rng = Rng::of(&[&c.id, "workshop", &v.number, if kind == JobKind::Repair { "repair" } else { "service" }], dates::parse(&c.date).unwrap_or(0));
        let faults = match kind {
            JobKind::Service => 2 + ((100.0 - v.condition) / 30.0).clamp(0.0, 2.0) as usize,
            JobKind::Repair => 4 + rng.int(0, 1) as usize,
        };
        let mut parts: Vec<Part> = Part::ALL.to_vec();
        let mut faulty: Vec<Part> = Vec::new();
        while faulty.len() < faults && !parts.is_empty() {
            let k = rng.int(0, parts.len() as i64 - 1) as usize;
            faulty.push(parts.remove(k));
        }
        let checks = Part::ALL.iter().map(|&p| Check { part: p, fault: faulty.contains(&p).then(|| if rng.chance(0.5) { Fix::Replace } else { p.second_fix() }), inspected: false, done: None }).collect();
        Job { kind, vehicle: v.id, checks }
    }

    pub fn faults(&self) -> usize {
        self.checks.iter().filter(|c| c.fault.is_some()).count()
    }

    /// Look at a part.
    pub fn inspect(&mut self, part: Part) {
        if let Some(c) = self.checks.iter_mut().find(|c| c.part == part) {
            c.inspected = true;
        }
    }

    /// Do `fix` to a part (once; it can be changed while the task runs). Returns whether it
    /// was the right thing.
    pub fn apply(&mut self, part: Part, fix: Fix) -> bool {
        let Some(c) = self.checks.iter_mut().find(|c| c.part == part) else { return false };
        c.inspected = true;
        c.done = Some(fix);
        c.fault.unwrap_or(Fix::Leave) == fix
    }

    /// Every part has been dealt with.
    pub fn complete(&self) -> bool {
        self.checks.iter().all(|c| c.done.is_some())
    }

    /// Faults fixed right, fixed wrong, missed; good parts worked on for nothing.
    pub fn tally(&self) -> (usize, usize, usize, usize) {
        let mut t = (0, 0, 0, 0);
        for c in &self.checks {
            match (c.fault, c.done) {
                (Some(f), Some(d)) if f == d => t.0 += 1,
                (Some(_), Some(_)) => t.1 += 1,
                (Some(_), None) => t.2 += 1,
                (None, Some(d)) if d != Fix::Leave => t.3 += 1,
                _ => {}
            }
        }
        t
    }

    /// How well the job was done, 0 to 1: a fault fixed right counts one, one fixed wrong
    /// takes half off, a good part worked on for nothing a quarter; finished with time to
    /// spare is worth up to 15 % more than at the bell.
    pub fn quality(&self, seconds_left: f32) -> f64 {
        let (right, wrong, _, wasted) = self.tally();
        let n = self.faults().max(1) as f64;
        let work = ((right as f64 - 0.5 * wrong as f64 - 0.25 * wasted as f64) / n).clamp(0.0, 1.0);
        let spare = (seconds_left / self.kind.seconds()).clamp(0.0, 1.0) as f64;
        (work * (0.85 + 0.15 * spare)).clamp(0.0, 1.0)
    }
}

/// A job the player did, as the company keeps it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct JobDone {
    pub date: String,
    pub vehicle: u32,
    pub number: String,
    pub repair: bool,
    /// 0 to 1.
    pub quality: f64,
    pub saved: Cents,
}

/// What a workshop charges for a service's work (the labour the player saves).
pub fn service_labour(c: &Company) -> Cents {
    ((600_00 as f64 * c.price_index.max(0.0)) / 100.0).round() as Cents * 100
}

/// A bus the player may service today: his course, the workshop, a bus not in the workshop
/// that is due within 5 000 km or below 90 %, not serviced by him today.
pub fn can_service(c: &Company, v: &Vehicle) -> bool {
    player_can(c, CourseKind::PlayerService) && !v.in_workshop(&c.date) && (v.km >= v.next_service_km - 5_000.0 || v.condition < 90.0) && !c.progress.jobs.iter().any(|j| j.vehicle == v.id && j.date == c.date)
}

/// The repair a broken-down bus waits for: what the workshop booked for it lately (within
/// three days), not yet done by the player.
pub fn repair_due(c: &Company, v: &Vehicle) -> Option<Cents> {
    if !v.in_workshop(&c.date) || v.breakdowns == 0 {
        return None;
    }
    let text = format!("{} {}", v.number, v.name);
    let booked = c.ledger.iter().rev().find(|b| b.kind == BookingKind::Repair && b.amount < 0 && b.text == text && dates::between(&b.date, &c.date) <= 3)?;
    if c.progress.jobs.iter().any(|j| j.vehicle == v.id && j.repair && dates::between(&booked.date, &j.date) >= 0) {
        return None;
    }
    Some(-booked.amount)
}

/// The player finished a job of `quality` (0 - 1) on a bus. A service: done without a day in
/// the workshop, the labour saved as far as the work was good (under 40 % the bus goes to the
/// workshop after all, and the parts were wasted). A repair: up to 55 % of the repair bill
/// back, and from 50 % the bus is out of the workshop tomorrow. Returns the job.
pub fn finish_job(c: &mut Company, vehicle: u32, kind: JobKind, quality: f64) -> Result<JobDone, &'static str> {
    let q = quality.clamp(0.0, 1.0);
    let Some(v) = c.vehicle(vehicle).cloned() else { return Err("This bus is not in the fleet.") };
    let today = c.date.clone();
    let saved = match kind {
        JobKind::Service => {
            if !can_service(c, &v) {
                return Err("This bus cannot be serviced by you today.");
            }
            let labour = service_labour(c);
            let vm = c.fleet.iter_mut().find(|x| x.id == vehicle).expect("the bus");
            if q < 0.4 {
                vm.workshop_until = Some(dates::add(&today, 1));
                let parts = labour / 4;
                c.book(BookingKind::Maintenance, -parts, format!("{} {} (own service, redone)", v.number, v.name), true);
                -parts
            } else {
                let serviced = market::serviced_condition(dates::years_between(&v.built, &today));
                vm.condition = vm.condition.max(serviced * (0.85 + 0.15 * q)).min(100.0);
                vm.next_service_km = ((vm.km / market::SERVICE_KM).floor() + 1.0) * market::SERVICE_KM;
                let saved = (labour as f64 * q).round() as Cents;
                c.book(BookingKind::Maintenance, saved, format!("{} {} (own service)", v.number, v.name), true);
                saved
            }
        }
        JobKind::Repair => {
            if !player_can(c, CourseKind::PlayerRepairs) {
                return Err("You need the repairs course first.");
            }
            let Some(bill) = repair_due(c, &v) else { return Err("This bus has no repair waiting.") };
            let saved = (bill as f64 * 0.55 * q).round() as Cents;
            if q >= 0.5 {
                let vm = c.fleet.iter_mut().find(|x| x.id == vehicle).expect("the bus");
                vm.workshop_until = Some(today.clone());
                vm.condition = (vm.condition + 5.0 * q).min(100.0);
            }
            c.book(BookingKind::Repair, saved, format!("{} {} (own repair)", v.number, v.name), true);
            saved
        }
    };
    c.progress.saved += saved.max(0);
    c.progress.xp += (10.0 + 20.0 * q).round() as i64;
    let job = JobDone { date: today, vehicle, number: v.number.clone(), repair: kind == JobKind::Repair, quality: q, saved };
    c.progress.jobs.push(job.clone());
    if c.progress.jobs.len() > 200 {
        c.progress.jobs.remove(0);
    }
    Ok(job)
}

#[cfg(test)]
mod tests {
    use super::super::market::{self, Payment};
    use super::super::model::{BusKind, BusSize, Difficulty, Drive};
    use super::super::staff::{applicants, hire};
    use super::super::{found, Founding};
    use super::*;

    fn company() -> Company {
        let mut c = found(&Founding { name: "Stadtbus".into(), difficulty: Difficulty::Realistic, date: "2024-05-06".into(), ..Default::default() }, "Luc");
        for a in applicants(&c).into_iter().take(3) {
            hire(&mut c, &a).unwrap();
        }
        c
    }

    fn with_bus(c: &mut Company) -> u32 {
        let bus = market::MarketBus { file: "Vehicles/SD/SD.bus".into(), name: "SD".into(), kind: BusKind { size: BusSize::Solo, drive: Drive::Diesel }, paints: vec![], default_paint: String::new() };
        market::buy_new(c, &bus, Payment::Cash, "").unwrap()
    }

    #[test]
    fn a_course_costs_money_and_days_and_teaches() {
        let mut c = company();
        let id = c.staff[0].id;
        let before = c.staff[0].skills;
        let cash = c.cash;
        // the eco course wants level 2
        assert!(enrol(&mut c, CourseKind::EcoDriving, Some(id)).is_err());
        assert!(enrol(&mut c, CourseKind::PlayerService, Some(id)).is_err());
        assert!(enrol(&mut c, CourseKind::Punctuality, None).is_err());
        let until = enrol(&mut c, CourseKind::Punctuality, Some(id)).unwrap();
        assert_eq!(until, "2024-05-07");
        assert_eq!(c.cash, cash - 650_00);
        assert!(c.ledger.last().is_some_and(|b| b.kind == BookingKind::Training));
        assert!(enrol(&mut c, CourseKind::Punctuality, Some(id)).is_err());
        assert!(enrol(&mut c, CourseKind::CustomerService, Some(id)).is_err(), "on a course already");
        // away on the course's days, from today
        assert!(c.staff[0].absent("2024-05-06") && c.staff[0].absent("2024-05-07") && !c.staff[0].absent("2024-05-08"));
        assert!(day_passed(&mut c, "2024-05-06").is_empty());
        let done = day_passed(&mut c, "2024-05-07");
        assert_eq!(done.len(), 1);
        assert!(trained(&c, Some(id), CourseKind::Punctuality));
        assert_eq!(c.staff[0].skills.punctuality, (before.punctuality + 15.0).min(100.0));
        assert!(c.staff[0].training_until.is_none());
        assert_eq!(c.progress.xp, 15);
    }

    #[test]
    fn mechanics_and_eco_drivers_save() {
        let mut c = company();
        assert_eq!(mechanic_share(&c, &c.date.clone()), 0.0);
        c.progress.xp = levels::LEVEL_XP[1];
        let ids: Vec<u32> = c.staff.iter().map(|e| e.id).collect();
        enrol(&mut c, CourseKind::Workshop, Some(ids[0])).unwrap();
        enrol(&mut c, CourseKind::EcoDriving, Some(ids[1])).unwrap();
        day_passed(&mut c, "2024-05-20");
        let d = c.date.clone();
        assert_eq!(mechanics(&c, &d), 1);
        assert!((mechanic_share(&c, &d) - 0.06).abs() < 1e-9);
        assert!((eco_share(&c, &d) - 0.08 / 3.0).abs() < 1e-9);
        // the day close books the savings against the day's bills
        c.book(BookingKind::Maintenance, -1_000_00, "Line 5", false);
        c.book(BookingKind::Energy, -3_000_00, "Line 5", false);
        let cash = c.cash;
        levels::day_closed(&mut c, &d, 0, 0, None, 0);
        assert_eq!(c.cash - cash, 60_00 + 80_00);
    }

    #[test]
    fn the_workshop_task_is_judged_by_what_was_fixed() {
        let mut c = company();
        let id = with_bus(&mut c);
        let v = c.vehicle(id).unwrap().clone();
        let job = Job::new(&c, &v, JobKind::Service);
        assert_eq!(job, Job::new(&c, &v, JobKind::Service), "the same task for the same bus and day");
        assert_eq!(job.checks.len(), Part::ALL.len());
        assert_eq!(job.faults(), 2);
        let repair = Job::new(&c, &v, JobKind::Repair);
        assert!((4..=5).contains(&repair.faults()));
        // all right, with time to spare
        let mut j = job.clone();
        for ch in job.checks.iter() {
            assert!(j.apply(ch.part, ch.fault.unwrap_or(Fix::Leave)) );
        }
        assert!(j.complete());
        assert!((j.quality(75.0) - 1.0).abs() < 1e-9);
        assert!((j.quality(0.0) - 0.85).abs() < 1e-9);
        // one fault fixed the wrong way, one missed, a good part replaced for nothing
        let mut j = job.clone();
        let faulty: Vec<&Check> = job.checks.iter().filter(|c| c.fault.is_some()).collect();
        let wrong = if faulty[0].fault == Some(Fix::Replace) { Fix::Leave } else { Fix::Replace };
        assert!(!j.apply(faulty[0].part, wrong));
        let good = job.checks.iter().find(|c| c.fault.is_none()).unwrap().part;
        j.apply(good, Fix::Replace);
        assert_eq!(j.tally(), (0, 1, 1, 1));
        assert_eq!(j.quality(75.0), 0.0);
        j.apply(faulty[1].part, faulty[1].fault.unwrap());
        assert!((j.quality(75.0) - (1.0 - 0.5 - 0.25) / 2.0).abs() < 1e-9);
    }

    #[test]
    fn the_players_own_work_saves_money() {
        let mut c = company();
        let id = with_bus(&mut c);
        // without the course nothing
        assert!(finish_job(&mut c, id, JobKind::Service, 1.0).is_err());
        c.progress.xp = levels::LEVEL_XP[3];
        enrol(&mut c, CourseKind::PlayerService, None).unwrap();
        enrol(&mut c, CourseKind::PlayerRepairs, None).unwrap();
        day_passed(&mut c, "2024-05-31");
        assert!(player_can(&c, CourseKind::PlayerService) && player_can(&c, CourseKind::PlayerRepairs));
        // a new bus is not due
        assert!(!can_service(&c, c.vehicle(id).unwrap()));
        c.fleet[0].condition = 70.0;
        assert!(can_service(&c, c.vehicle(id).unwrap()));
        let cash = c.cash;
        let job = finish_job(&mut c, id, JobKind::Service, 0.8).unwrap();
        assert_eq!(job.saved, 480_00);
        assert_eq!(c.cash - cash, 480_00);
        assert!(c.fleet[0].condition > 70.0 && !c.fleet[0].in_workshop(&c.date));
        // once a day
        assert!(finish_job(&mut c, id, JobKind::Service, 0.8).is_err());
        // a bad job: into the workshop after all, the parts paid
        c.date = "2024-06-01".into();
        c.fleet[0].condition = 60.0;
        let job = finish_job(&mut c, id, JobKind::Service, 0.2).unwrap();
        assert!(job.saved < 0 && c.fleet[0].in_workshop("2024-06-02"));
        // a breakdown: the repair bill, part of it back, the bus out tomorrow
        c.date = "2024-06-05".into();
        c.fleet[0].breakdowns = 1;
        c.fleet[0].workshop_until = Some("2024-06-07".into());
        let text = format!("{} {}", c.fleet[0].number, c.fleet[0].name);
        c.book(BookingKind::Repair, -2_000_00, text, false);
        assert_eq!(repair_due(&c, &c.fleet[0].clone()), Some(2_000_00));
        let job = finish_job(&mut c, id, JobKind::Repair, 1.0).unwrap();
        assert_eq!(job.saved, 1_100_00);
        assert_eq!(c.fleet[0].workshop_until.as_deref(), Some("2024-06-05"));
        assert_eq!(repair_due(&c, &c.fleet[0].clone()), None);
        assert!(finish_job(&mut c, id, JobKind::Repair, 1.0).is_err());
    }
}
