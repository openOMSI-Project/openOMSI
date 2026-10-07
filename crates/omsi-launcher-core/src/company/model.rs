//! The company as it is saved: who it is, where it is at home, its money and its books, its
//! buses, its people and its lines. Amounts are whole cents everywhere (Omsi-Hub's rule: a
//! hundred days of additions never lose half a cent), dates `YYYY-MM-DD`.
//!
//! Every booking says whether it was measured (what the player or, later, the game's own
//! company buses really drove) or modelled (what the day close reckoned for the rest), as
//! Omsi-Hub's books do.

use super::dates;
use super::day::{DayReport, LiveEvent};
use serde::{Deserialize, Serialize};

pub type Cents = i64;

/// The version of the saved file; raised when a field changes meaning (see `store::migrate`).
pub const VERSION: u32 = 1;

/// How hard the economy is (see `economy::rules`).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Difficulty {
    Easy,
    #[default]
    Realistic,
    Hard,
}

impl Difficulty {
    pub const ALL: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Realistic, Difficulty::Hard];

    pub fn label(self) -> &'static str {
        match self {
            Difficulty::Easy => "Easy",
            Difficulty::Realistic => "Realistic",
            Difficulty::Hard => "Hard",
        }
    }
}

/// A bus's length class: what it costs, what it carries and who may drive it.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum BusSize {
    /// Up to about 10.5 m.
    Midi,
    /// The 12 m standard bus.
    #[default]
    Solo,
    /// 18 m, with a trailer section.
    Articulated,
    Double,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Drive {
    #[default]
    Diesel,
    Electric,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct BusKind {
    pub size: BusSize,
    pub drive: Drive,
}

impl BusKind {
    pub fn label(self) -> &'static str {
        match (self.size, self.drive) {
            (BusSize::Midi, Drive::Diesel) => "Midibus",
            (BusSize::Midi, Drive::Electric) => "Electric midibus",
            (BusSize::Solo, Drive::Diesel) => "Solo bus",
            (BusSize::Solo, Drive::Electric) => "Electric solo bus",
            (BusSize::Articulated, Drive::Diesel) => "Articulated bus",
            (BusSize::Articulated, Drive::Electric) => "Electric articulated bus",
            (BusSize::Double, Drive::Diesel) => "Double-decker",
            (BusSize::Double, Drive::Electric) => "Electric double-decker",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum BookingKind {
    /// The founders' starting capital.
    Capital,
    /// A loan paid out, and its repayment (the principal part of a rate).
    Loan,
    Repayment,
    Interest,
    Fares,
    /// The authority's payment per timetable kilometre (the public service contract).
    Compensation,
    /// A grant towards a new bus.
    Subsidy,
    Purchase,
    Sale,
    Lease,
    Rent,
    /// Diesel or electricity.
    Energy,
    Maintenance,
    Repair,
    Insurance,
    Depot,
    Wages,
    Severance,
    /// Contract penalties: trips dropped, trips late.
    Penalty,
    /// Traffic fines of the player's own tours (red lights, speed cameras).
    Fine,
    /// The authority's quality bonus for the player's good tours.
    Bonus,
    /// Training courses.
    Training,
    /// Building the depot's areas (an investment, like a bus).
    Construction,
    /// Tender fees and the licences of the company's own lines.
    Concession,
    /// The company's own liveries: their design, and buses painted in them.
    Livery,
    /// What advertisers pay for the adverts on the buses (and a contract ended early costs).
    Advertising,
}

impl BookingKind {
    pub const ALL: [BookingKind; 26] = [
        BookingKind::Capital,
        BookingKind::Loan,
        BookingKind::Repayment,
        BookingKind::Interest,
        BookingKind::Fares,
        BookingKind::Compensation,
        BookingKind::Subsidy,
        BookingKind::Purchase,
        BookingKind::Sale,
        BookingKind::Lease,
        BookingKind::Rent,
        BookingKind::Energy,
        BookingKind::Maintenance,
        BookingKind::Repair,
        BookingKind::Insurance,
        BookingKind::Depot,
        BookingKind::Wages,
        BookingKind::Severance,
        BookingKind::Penalty,
        BookingKind::Fine,
        BookingKind::Bonus,
        BookingKind::Training,
        BookingKind::Construction,
        BookingKind::Concession,
        BookingKind::Livery,
        BookingKind::Advertising,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BookingKind::Capital => "Starting capital",
            BookingKind::Loan => "Loan",
            BookingKind::Repayment => "Repayment",
            BookingKind::Interest => "Interest",
            BookingKind::Fares => "Fares",
            BookingKind::Compensation => "Compensation",
            BookingKind::Subsidy => "Subsidy",
            BookingKind::Purchase => "Bus bought",
            BookingKind::Sale => "Bus sold",
            BookingKind::Lease => "Leasing",
            BookingKind::Rent => "Rental",
            BookingKind::Energy => "Fuel and power",
            BookingKind::Maintenance => "Maintenance",
            BookingKind::Repair => "Repairs",
            BookingKind::Insurance => "Insurance",
            BookingKind::Depot => "Depot",
            BookingKind::Wages => "Wages",
            BookingKind::Severance => "Severance",
            BookingKind::Penalty => "Penalties",
            BookingKind::Fine => "Traffic fines",
            BookingKind::Bonus => "Quality bonus",
            BookingKind::Training => "Training",
            BookingKind::Construction => "Depot building",
            BookingKind::Concession => "Concessions and licences",
            BookingKind::Livery => "Liveries and painting",
            BookingKind::Advertising => "Advertising",
        }
    }

    /// Money that comes or goes with the company's capital (founding, loans, buying and
    /// selling buses): not part of what running the lines earns. A purchase is no bad day.
    pub fn is_capital(self) -> bool {
        matches!(self, BookingKind::Capital | BookingKind::Loan | BookingKind::Repayment | BookingKind::Purchase | BookingKind::Sale | BookingKind::Subsidy | BookingKind::Construction)
    }
}

/// One line in the books.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Booking {
    pub date: String,
    pub kind: BookingKind,
    /// Positive comes in, negative goes out.
    pub amount: Cents,
    /// What it was for (line numbers, a bus, a person: not translated).
    pub text: String,
    /// From a trip really driven (true), or from the model (false).
    pub measured: bool,
}

/// A month's bookings added up by kind (the ledger keeps only the last few thousand lines).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Month {
    /// `YYYY-MM`.
    pub month: String,
    pub by_kind: Vec<(BookingKind, Cents)>,
}

impl Month {
    pub fn get(&self, k: BookingKind) -> Cents {
        self.by_kind.iter().find(|x| x.0 == k).map(|x| x.1).unwrap_or(0)
    }

    /// What running the lines earned (capital movements left out).
    pub fn result(&self) -> Cents {
        self.by_kind.iter().filter(|x| !x.0.is_capital()).map(|x| x.1).sum()
    }

    pub fn income(&self) -> Cents {
        self.by_kind.iter().filter(|x| !x.0.is_capital() && x.1 > 0).map(|x| x.1).sum()
    }

    pub fn expenses(&self) -> Cents {
        -self.by_kind.iter().filter(|x| !x.0.is_capital() && x.1 < 0).map(|x| x.1).sum::<Cents>()
    }
}

/// How the company holds a bus.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Tenure {
    /// Bought: what was paid, and the new price its value is written off from.
    Owned { paid: Cents, new_value: Cents },
    /// Leased: the monthly rate until `until`, when it goes back (its residual value is what
    /// buying it then would cost).
    Leased { monthly: Cents, until: String, residual: Cents },
    /// Rented by the day until `until` (it goes back after that day).
    Rented { daily: Cents, until: String },
}

/// A bus of the fleet.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Vehicle {
    /// Unique in the company, never given again.
    pub id: u32,
    /// The fleet number on its sides.
    pub number: String,
    pub plate: String,
    /// The bus file (`Vehicles/.../x.bus`) and its name.
    pub bus: String,
    pub name: String,
    pub kind: BusKind,
    /// The paint it wears (empty: the bus's own), one of the bus's liveries.
    #[serde(default)]
    pub livery: String,
    /// The company's house livery for this bus, once the livery studio made one.
    #[serde(default)]
    pub house_livery: Option<String>,
    /// First registered.
    pub built: String,
    pub km: f64,
    /// 100: as new from the workshop.
    pub condition: f64,
    /// The odometer reading of its next service.
    pub next_service_km: f64,
    pub tenure: Tenure,
    pub acquired: String,
    /// In the workshop up to and including this day.
    #[serde(default)]
    pub workshop_until: Option<String>,
    #[serde(default)]
    pub breakdowns: u32,
}

impl Vehicle {
    pub fn age_years(&self, today: &str) -> f64 {
        dates::years_between(&self.built, today)
    }

    pub fn in_workshop(&self, today: &str) -> bool {
        self.workshop_until.as_deref().is_some_and(|u| dates::between(today, u) >= 0)
    }

    /// Still the company's on `today` (a lease or a rental ends).
    pub fn held_on(&self, today: &str) -> bool {
        match &self.tenure {
            Tenure::Owned { .. } => true,
            Tenure::Leased { until, .. } | Tenure::Rented { until, .. } => dates::between(today, until) >= 0,
        }
    }
}

/// A bus driving licence: D every bus, D1 only small ones.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Licence {
    #[default]
    D,
    D1,
}

/// 0 to 100 each.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug, Default)]
pub struct Skills {
    pub driving: f64,
    pub punctuality: f64,
    pub service: f64,
}

/// Someone on the payroll.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Employee {
    pub id: u32,
    pub name: String,
    pub age: u32,
    /// 0 to 100 points, growing with every day worked (Omsi-Hub's: under 25 an articulated
    /// bus is a warning, see `staff::qualified`).
    pub experience: f64,
    pub licence: Licence,
    /// Monthly gross wage; the company pays the employer's share on top
    /// (`economy::employer_cost`).
    pub wage: Cents,
    /// 0 to 1: how seldom they are ill or miss a duty.
    pub reliability: f64,
    pub skills: Skills,
    /// 0 to 100: under 25 they may hand in their notice.
    pub satisfaction: f64,
    pub hired: String,
    /// Leaves after this day (dismissed or resigned).
    #[serde(default)]
    pub notice_until: Option<String>,
    #[serde(default)]
    pub resigned: bool,
    #[serde(default)]
    pub sick_until: Option<String>,
    #[serde(default)]
    pub holiday_until: Option<String>,
    /// Away on a training course up to and including this day (`training::enrol`).
    #[serde(default)]
    pub training_until: Option<String>,
    /// Holiday days left this year.
    pub holiday_left: u32,
    /// Days worked in the running week (at most five).
    #[serde(default)]
    pub week_days: u32,
    /// When their last duty ended, in minutes from the start of the day before the one
    /// being planned (for the night's rest; None: not yesterday).
    #[serde(default)]
    pub last_end: Option<i32>,
    #[serde(default)]
    pub days_worked: u32,
    /// Besides the licence: the buses they may drive (`licences::Endorsement`), and the model
    /// families they had the type training for (`licences::type_key`).
    #[serde(default)]
    pub endorsements: Vec<super::licences::Endorsement>,
    #[serde(default)]
    pub types: Vec<String>,
}

impl Employee {
    pub fn absent(&self, today: &str) -> bool {
        let until = |u: &Option<String>| u.as_deref().is_some_and(|u| dates::between(today, u) >= 0);
        until(&self.sick_until) || until(&self.holiday_until) || until(&self.training_until)
    }

    pub fn employed_on(&self, today: &str) -> bool {
        dates::between(&self.hired, today) >= 0 && self.notice_until.as_deref().is_none_or(|u| dates::between(today, u) >= 0)
    }
}

/// A loan with a fixed monthly rate (an annuity): interest on what is left, the rest repays.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Loan {
    pub id: u32,
    pub taken: String,
    pub principal: Cents,
    pub remaining: Cents,
    /// Yearly interest, 0.045 = 4.5 %.
    pub rate: f64,
    pub monthly: Cents,
    pub months_left: u32,
    #[serde(default)]
    pub purpose: String,
}

/// A line the company runs: one of the map's timetable lines, or one of the player's own
/// from the line editor (it is a `.ttl` of the map's timetable as well).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct CompanyLine {
    /// The timetable's line (its `.ttl` name): how the tours are found.
    pub name: String,
    /// The number shown (a plate), and every number its trips' displays show (a trip
    /// report names the line by these).
    pub number: String,
    #[serde(default)]
    pub numbers: Vec<String>,
    /// Made in the line editor.
    pub own: bool,
    /// `#rrggbb` (the player's lines have one).
    #[serde(default)]
    pub colour: String,
    /// Where it goes.
    #[serde(default)]
    pub caption: String,
    pub added: String,
    /// What the timetable gave the last time it was read: tours and kilometres a day.
    #[serde(default)]
    pub tours: u32,
    #[serde(default)]
    pub km: f64,
    /// An own line the company confirmed and paid for in the line editor: what it keeps of it
    /// (its passengers through the day, its tours' bus sizes; see `ownline`).
    #[serde(default)]
    pub plan: Option<super::ownline::OwnPlan>,
    /// From when the line runs (the company clock's minutes, `clock::moment`): None while it
    /// is not planned yet - its tours are neither run nor penalised (`network::in_service`). A
    /// file of an older version, which ran every line, reads `LEGACY_SERVICE` (`store::migrate`).
    #[serde(default = "legacy_service")]
    pub service_from: Option<i64>,
    /// The single ticket's price the line asks (None: the fare association's; `fares`).
    #[serde(default)]
    pub fare: Option<Cents>,
    /// How the passengers have taken the fare so far (`fares::Demand`).
    #[serde(default)]
    pub demand: super::fares::Demand,
    /// What the line is called in public - on its card and in the company's advertising
    /// ("Shuttleverkehr Altenfeld - Wurzbach"); empty: its number and where it goes.
    #[serde(default)]
    pub title: String,
    /// The depot file the company's buses carry on it - their destinations and the IBIS
    /// (empty: the company's, `Company::depot`).
    #[serde(default)]
    pub hof: String,
    /// An own line in service changed in the line editor for a later day (`ownline::Pending`).
    #[serde(default)]
    pub pending: Option<super::ownline::Pending>,
}

impl CompanyLine {
    /// The depot file its buses carry: its own, else the company's (`depot`).
    pub fn hof_or<'a>(&'a self, depot: &'a str) -> &'a str {
        if self.hof.trim().is_empty() { depot } else { self.hof.trim() }
    }

    /// What it is called in public: its title, else its caption.
    pub fn public_name(&self) -> &str {
        if self.title.trim().is_empty() { self.caption.trim() } else { self.title.trim() }
    }
}

/// What a line of an older file says of its service until `store::migrate` decides it.
pub const LEGACY_SERVICE: i64 = i64::MIN;

fn legacy_service() -> Option<i64> {
    Some(LEGACY_SERVICE)
}

/// How a closed day ended, for the dashboard's chart.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct DayRecord {
    pub date: String,
    pub cash: Cents,
    /// Operating income and expenses (capital movements left out), and their difference.
    pub income: Cents,
    pub expenses: Cents,
    pub result: Cents,
    pub tours: u32,
    pub dropped_tours: u32,
    pub trips: u32,
    pub dropped_trips: u32,
    pub km: f64,
    pub passengers: u32,
    /// Trips on time, of the trips run (0 to 100; None without trips).
    pub punctuality: Option<f64>,
}

/// What of the week's markets is gone (bought, hired): a new week brings new offers.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Taken {
    pub week: i64,
    pub used: Vec<u32>,
    pub applicants: Vec<u32>,
}

/// Numbers that only go up: a sold bus or a person who left never gives theirs away.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Counters {
    pub vehicle: u32,
    pub employee: u32,
    pub loan: u32,
    /// The last fleet number given.
    pub fleet_number: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Company {
    pub version: u32,
    /// The file's name (`companies/<id>.json`).
    pub id: String,
    /// The driver (profile) it belongs to.
    pub profile: String,
    pub name: String,
    /// Two to four letters: the plates and the logo's monogram.
    pub short: String,
    /// `#rrggbb`: the main colour and the second.
    pub colours: [String; 2],
    /// A picture of the logo (a path), if one was chosen.
    #[serde(default)]
    pub logo: Option<String>,
    /// The home map (`maps/<folder>/global.cfg`), its name, and the depot: the `.hof` file
    /// its buses carry.
    pub map: String,
    pub map_name: String,
    pub depot: String,
    pub founded: String,
    /// The day that runs now: the next one "Close the day" settles.
    pub date: String,
    pub difficulty: Difficulty,
    pub cash: Cents,
    /// 0 to 100: the authority's view of the company (it moves the compensation ±10 %).
    pub reputation: f64,
    /// Trips on time over the last weeks, 0 to 100.
    pub punctuality: f64,
    /// Prices against founding day (inflation; 1 on an easy economy).
    pub price_index: f64,
    /// Compensation per kilometre against founding day (the contract's indexation).
    #[serde(default = "one")]
    pub contract_index: f64,
    pub ledger: Vec<Booking>,
    #[serde(default)]
    pub months: Vec<Month>,
    #[serde(default)]
    pub history: Vec<DayRecord>,
    #[serde(default)]
    pub loans: Vec<Loan>,
    #[serde(default)]
    pub fleet: Vec<Vehicle>,
    #[serde(default)]
    pub staff: Vec<Employee>,
    #[serde(default)]
    pub lines: Vec<CompanyLine>,
    #[serde(default)]
    pub taken: Taken,
    #[serde(default)]
    pub counters: Counters,
    /// The trip reports booked already: those that ended up to this time (Unix seconds).
    #[serde(default)]
    pub trips_seen: u64,
    /// What the game reported of the company's own buses today (see `day::record_live`).
    #[serde(default)]
    pub live: Vec<LiveEvent>,
    #[serde(default)]
    pub last_report: Option<DayReport>,
    /// The weekly roster and the dispatcher's choices for the day (see `plan`).
    #[serde(default)]
    pub planning: super::plan::Planning,
    /// Its level, its courses, the player's tours judged (`levels`, `training`).
    #[serde(default)]
    pub progress: super::levels::Progress,
    /// The depot's buildings and its workshop's jobs (see `depot`).
    #[serde(default)]
    pub site: super::depot::Site,
    /// The lines' concessions and the tenders (see `concessions`).
    #[serde(default)]
    pub concessions: super::concessions::Concessions,
    /// The company's now within its day, the feed of what happened, today's events (see
    /// `clock`).
    #[serde(default)]
    pub clock: super::clock::Clock,
    /// The dealer: the buying mode, offers bought, orders, talks, warranties (see `dealer`).
    #[serde(default)]
    pub dealer: super::dealer::DealerState,
    /// The company's own liveries (see `livery`).
    #[serde(default)]
    pub designs: Vec<super::livery::Design>,
    /// The advertising on its buses (see `adverts`).
    #[serde(default)]
    pub adverts: super::adverts::Adverts,
    /// 1: its people's licences and type trainings are kept (`licences`); 0: a file of before,
    /// whose people get what the fleet asks (`store::migrate`).
    #[serde(default)]
    pub quals: u32,
}

fn one() -> f64 {
    1.0
}

/// How many bookings the ledger keeps (the months keep the sums of all of them).
pub const LEDGER_KEPT: usize = 4000;
/// How many closed days the history keeps.
pub const HISTORY_KEPT: usize = 400;

impl Company {
    /// Book an amount: the cash, the ledger and the month's sums.
    pub fn book(&mut self, kind: BookingKind, amount: Cents, text: impl Into<String>, measured: bool) {
        if amount == 0 {
            return;
        }
        self.cash += amount;
        let date = self.date.clone();
        let month = dates::month_of(&date);
        match self.months.iter_mut().find(|m| m.month == month) {
            Some(m) => match m.by_kind.iter_mut().find(|x| x.0 == kind) {
                Some(x) => x.1 += amount,
                None => m.by_kind.push((kind, amount)),
            },
            None => self.months.push(Month { month, by_kind: vec![(kind, amount)] }),
        }
        self.ledger.push(Booking { date, kind, amount, text: text.into(), measured });
        if self.ledger.len() > LEDGER_KEPT {
            let extra = self.ledger.len() - LEDGER_KEPT;
            self.ledger.drain(..extra);
        }
    }

    /// The month a date is in (an empty one when nothing was booked).
    pub fn month(&self, month: &str) -> Month {
        self.months.iter().find(|m| m.month == month).cloned().unwrap_or_else(|| Month { month: month.to_string(), by_kind: Vec::new() })
    }

    pub fn vehicle(&self, id: u32) -> Option<&Vehicle> {
        self.fleet.iter().find(|v| v.id == id)
    }

    pub fn employee(&self, id: u32) -> Option<&Employee> {
        self.staff.iter().find(|e| e.id == id)
    }

    /// What is still owed on all loans.
    pub fn debt(&self) -> Cents {
        self.loans.iter().map(|l| l.remaining).sum()
    }

    /// The next fleet number: from 101 on, never one given before.
    pub fn next_fleet_number(&mut self) -> String {
        self.counters.fleet_number = self.counters.fleet_number.max(100) + 1;
        self.counters.fleet_number.to_string()
    }

    /// A plate in the company's district: its short name, a letter and the fleet number
    /// ("OHB-K 101").
    pub fn plate_for(&self, number: &str) -> String {
        let district: String = self.short.chars().filter(|c| c.is_alphabetic()).take(3).collect::<String>().to_uppercase();
        let district = if district.is_empty() { "B".to_string() } else { district };
        format!("{district}-K {number}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_booking_moves_the_cash_and_the_month() {
        let mut c = super::super::found(&super::super::Founding { name: "Test".into(), ..Default::default() }, "Luc");
        let start = c.cash;
        c.book(BookingKind::Fares, 12_345, "line 5", true);
        c.book(BookingKind::Energy, -2_000, "line 5", false);
        c.book(BookingKind::Purchase, -100_000, "bus", false);
        assert_eq!(c.cash, start + 12_345 - 2_000 - 100_000);
        let m = c.month(&dates::month_of(&c.date));
        assert_eq!(m.result(), 10_345);
        assert_eq!(m.income(), 12_345);
        assert_eq!(m.expenses(), 2_000);
        assert_eq!(m.get(BookingKind::Purchase), -100_000);
        assert!(c.ledger.last().is_some_and(|b| !b.measured && b.kind == BookingKind::Purchase));
        assert_eq!(c.plate_for("101"), "TES-K 101");
    }
}
