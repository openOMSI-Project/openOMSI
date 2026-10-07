//! What runs on a player's line, and when (Luc: "schoolvervoer / lijnvervoer / weekendritjes
//! ... aangepast aan de realiteit"): the kind of service a line is (`ServiceKind`), the buses
//! that run it (`LineVehicles` - by the kind of bus, and by maker, model or the very bus file)
//! and what day a date is (`DayCalendar`: its weekday, a public holiday, the school holidays).
//!
//! The kinds, as German and Dutch operators know them:
//! * *Regular* (Linienverkehr): every day as the timetable has it, paid by the fares and the
//!   authority's money per kilometre.
//! * *School transport* (Schülerverkehr, leerlingenvervoer): on school days only - Monday to
//!   Friday outside the school holidays and the public holidays -, its trips around the
//!   beginning and the end of the school day. The school authority pays a contract per trip
//!   (the pupils' passes are part of it) and is strict about punctuality.
//! * *Weekend and leisure trips* (Freizeitverkehr): Saturdays, Sundays and public holidays in
//!   the daytime, people out for the day - a higher fare (day tickets, visitors), and fewer of
//!   them when it rains.
//! * *On demand* (Rufbus, Anrufsammeltaxi, the Dutch buurtbus and taxibus): small buses on a
//!   timetable whose trips run only when somebody booked them; the passenger pays the fare
//!   and a booking fee.
//!
//! In the game a kind is the tours' day masks (`ServiceKind::mask`): a school line's tours
//! carry the school-day bit alone, so that the game's own calendar - the map's `Holidays.txt`
//! - keeps them in the depot in the holidays, as OMSI does with the maps' own school runs. A
//! map without school holidays of its own takes the periods of its line registry
//! (`lines::Registry::school_holidays`, by default `default_school_holidays`) where the
//! launcher and the bus company decide which tours run; the game itself knows only the map's.

use crate::company::dates;
use crate::company::BusSize;
use serde::{Deserialize, Serialize};
use std::path::Path;

// --- the kind of service -------------------------------------------------------------------------

/// What kind of service a line is (see the module's text).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ServiceKind {
    #[default]
    Regular,
    School,
    Leisure,
    OnDemand,
}

/// The game's school bits of a tour mask: bit 8 runs in the school holidays, bit 9 on school
/// days (`lines::SCHOOL_BITS` is both).
pub const SCHOOL_HOLIDAY_BIT: i32 = 1 << 8;
pub const SCHOOL_DAY_BIT: i32 = 1 << 9;
/// Monday to Friday, and Saturday, Sunday and the public holidays (bits 0 - 4, 5 - 7).
pub const WORKDAY_BITS: u16 = 0b0001_1111;
pub const WEEKEND_BITS: u16 = 0b1110_0000;

impl ServiceKind {
    pub const ALL: [ServiceKind; 4] = [ServiceKind::Regular, ServiceKind::School, ServiceKind::Leisure, ServiceKind::OnDemand];

    pub fn label(self) -> &'static str {
        match self {
            ServiceKind::Regular => "Regular service",
            ServiceKind::School => "School transport",
            ServiceKind::Leisure => "Weekend and leisure",
            ServiceKind::OnDemand => "On demand",
        }
    }

    /// What it means, in a sentence for the line editor.
    pub fn note(self) -> &'static str {
        match self {
            ServiceKind::Regular => "Every day as the timetable has it. Paid by the fares and the authority's money per kilometre.",
            ServiceKind::School => "Monday to Friday on school days only: not in the school holidays, not on public holidays. Trips around the start and the end of school; the school authority pays per trip and wants them on time.",
            ServiceKind::Leisure => "Saturdays, Sundays and public holidays in the daytime: people out for the day, a higher fare, fewer of them when it rains.",
            ServiceKind::OnDemand => "Small buses whose trips run only when somebody booked them. The passenger pays the fare and a booking fee.",
        }
    }

    /// Its mark in the interface (one of the interface's icons: pupils, a day out in the sun,
    /// a booking by phone).
    pub fn icon(self) -> &'static str {
        match self {
            ServiceKind::Regular => "directions_bus",
            ServiceKind::School => "groups",
            ServiceKind::Leisure => "wb_sunny",
            ServiceKind::OnDemand => "smartphone",
        }
    }

    /// The line runs the day group `group` (an index into `lines::DAY_GROUPS`: working days,
    /// Saturday, Sunday and public holidays) at all.
    pub fn allows(self, group: usize) -> bool {
        match self {
            ServiceKind::School => group == 0,
            ServiceKind::Leisure => group == 1 || group == 2,
            ServiceKind::Regular | ServiceKind::OnDemand => true,
        }
    }

    /// The tour mask a day pattern's days (`lines::DayPattern::days`) are written as: the
    /// days the kind runs on of them, and the school bits - a school line's on school days
    /// only, any other in the holidays as well.
    pub fn mask(self, days: u16) -> i32 {
        match self {
            ServiceKind::School => (days & WORKDAY_BITS) as i32 | SCHOOL_DAY_BIT,
            ServiceKind::Leisure => (days & WEEKEND_BITS) as i32 | SCHOOL_HOLIDAY_BIT | SCHOOL_DAY_BIT,
            ServiceKind::Regular | ServiceKind::OnDemand => days as i32 & 0xff | SCHOOL_HOLIDAY_BIT | SCHOOL_DAY_BIT,
        }
    }

    /// The kinds of bus that suit it, for the line editor to suggest (none: any).
    pub fn suits(self) -> &'static [VehicleClass] {
        match self {
            ServiceKind::Regular => &[],
            ServiceKind::School => &[VehicleClass::Solo, VehicleClass::Articulated, VehicleClass::Coach],
            ServiceKind::Leisure => &[VehicleClass::Midi, VehicleClass::Solo, VehicleClass::Coach],
            ServiceKind::OnDemand => &[VehicleClass::Minibus, VehicleClass::Midi],
        }
    }
}

/// Whether a tour with `mask` runs on a day whose bits (`DayType::bits`) are `day` and
/// `school`, as the game reads it.
pub fn mask_runs(mask: i32, (day, school): (i32, i32)) -> bool {
    mask & day != 0 && mask & school != 0
}

// --- the buses of a line --------------------------------------------------------------------------

/// A kind of bus as a line asks for it: the company's sizes, with the small buses and the
/// coaches told apart.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum VehicleClass {
    /// A van-sized bus (Sprinter, Crafter): the taxibus, the on-demand bus.
    Minibus,
    /// Up to about 10.5 m.
    Midi,
    #[default]
    Solo,
    Articulated,
    Double,
    /// A coach or an intercity bus (Setra, Tourismo, Intouro, the "Ü" and "UL" buses).
    Coach,
}

impl VehicleClass {
    pub const ALL: [VehicleClass; 6] = [VehicleClass::Minibus, VehicleClass::Midi, VehicleClass::Solo, VehicleClass::Articulated, VehicleClass::Double, VehicleClass::Coach];

    pub fn label(self) -> &'static str {
        match self {
            VehicleClass::Minibus => "Minibus",
            VehicleClass::Midi => "Midibus",
            VehicleClass::Solo => "Solo bus",
            VehicleClass::Articulated => "Articulated bus",
            VehicleClass::Double => "Double-decker",
            VehicleClass::Coach => "Coach",
        }
    }

    /// The company's size of it (what it costs, carries and who may drive it).
    pub fn size(self) -> BusSize {
        match self {
            VehicleClass::Minibus | VehicleClass::Midi => BusSize::Midi,
            VehicleClass::Solo | VehicleClass::Coach => BusSize::Solo,
            VehicleClass::Articulated => BusSize::Articulated,
            VehicleClass::Double => BusSize::Double,
        }
    }
}

/// Words a minibus has in its names (vans made buses, taxibuses, on-demand buses).
const MINI: [&str; 11] = ["sprinter", "crafter", "minibus", "kleinbus", "taxibus", "rufbus", "ducato", "vito", "vivaro", "jumper", "iveco daily"];
/// Words a coach or an intercity bus has in its names.
const COACH: [&str; 16] = ["reisebus", "coach", "tourismo", "travego", "intouro", "integro", "tourliner", "comfortclass", "topclass", "multiclass", "überland", "ueberland", "uberland", "intercity", "lion's coach", "lionscoach"];
/// Short words a coach's or an intercity bus's type is given ("S 415 UL", "O 303 RHD"), as
/// words of their own.
const COACH_WORDS: [&str; 10] = ["hd", "hdh", "gt", "ul", "rhd", "shd", "rl", "msd", "ü", "ue"];

/// The kind of a bus from what is known of it: its names (name, type, maker, file), whether a
/// trailer section is coupled behind it and its length and height in metres. The size is the
/// company market's (`market::guess_kind`); a short one, or one whose names say so, is a
/// minibus, and a solo bus whose names say so a coach.
pub fn classify(texts: &[&str], trailer: bool, length: Option<f32>, height: Option<f32>) -> VehicleClass {
    let size = crate::company::market::guess_kind(texts, &[], trailer, length, height).size;
    let all = texts.join(" ").to_lowercase();
    let words: Vec<String> = all.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect();
    let mini = MINI.iter().any(|k| all.contains(k)) || length.is_some_and(|l| l > 3.0 && l < 8.0);
    let coach = COACH.iter().any(|k| all.contains(k)) || words.iter().any(|w| COACH_WORDS.contains(&w.as_str()));
    match size {
        BusSize::Articulated => VehicleClass::Articulated,
        BusSize::Double => VehicleClass::Double,
        BusSize::Midi if mini => VehicleClass::Minibus,
        BusSize::Midi => VehicleClass::Midi,
        BusSize::Solo if mini && !coach => VehicleClass::Minibus,
        BusSize::Solo if coach => VehicleClass::Coach,
        BusSize::Solo => VehicleClass::Solo,
    }
}

/// The kind of an installed bus from its file (it is read: its length, its trailer section).
pub fn class_of_file(file: &str, texts: &[&str]) -> VehicleClass {
    let path = crate::resolve_content(file).unwrap_or_else(|_| std::path::PathBuf::from(file));
    let mut all: Vec<String> = texts.iter().map(|t| t.to_string()).collect();
    all.push(file.to_string());
    match omsi_vehicle::Vehicle::load(&path) {
        Ok(v) => {
            all.push(v.type_name.clone());
            all.push(v.manufacturer.clone());
            let refs: Vec<&str> = all.iter().map(String::as_str).collect();
            let bb = v.bounding_box;
            classify(&refs, v.couple_back.is_some(), bb.map(|b| b[1]), bb.map(|b| b[2]))
        }
        Err(_) => {
            let refs: Vec<&str> = all.iter().map(String::as_str).collect();
            classify(&refs, false, None, None)
        }
    }
}

/// A name to compare: lower case, letters and digits only.
pub fn fold(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).filter(|c| c.is_alphanumeric()).collect()
}

fn same_file(a: &str, b: &str) -> bool {
    a.trim().replace('\\', "/").eq_ignore_ascii_case(&b.trim().replace('\\', "/"))
}

/// The maker and the model of an installed bus as the bus step's picker groups them: the
/// maker of `[friendlyname]` (else the bus's folder), and the model - its type up to the first
/// " - " ("Gelenkbus - 18C - 3 Tuerer": Gelenkbus).
pub fn maker_model(manufacturer: &str, type_name: &str, file: &str, folder: &str) -> (String, String) {
    let maker = if manufacturer.trim().is_empty() { crate::display_bus_name(folder) } else { crate::display_bus_name(manufacturer) };
    let label = crate::vehicle_type_label(type_name, Path::new(file));
    let model = label.split(" - ").map(str::trim).find(|s| !s.is_empty()).unwrap_or(label.as_str()).to_string();
    (maker, model)
}

/// What a line's choice of buses is weighed against: a bus's file, maker, model, names and
/// kind. A bus of the company's fleet knows its file, name and size only (`BusFacts::of_fleet`):
/// its maker and model are then looked for in its name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BusFacts {
    pub file: String,
    pub maker: String,
    pub model: String,
    pub name: String,
    pub class: VehicleClass,
}

impl BusFacts {
    /// An installed bus (`class` from `class_of_file`, or a guess).
    pub fn of_info(v: &crate::VehicleInfo, class: VehicleClass) -> BusFacts {
        let (maker, model) = maker_model(&v.manufacturer, &v.type_name, &v.file, &v.folder);
        BusFacts { file: v.file.clone(), maker, model, name: v.name.clone(), class }
    }

    /// A bus of the company's fleet: its file, its name ("MAN Lion's City") and its size; the
    /// kind as its name tells it.
    pub fn of_fleet(file: &str, name: &str, size: BusSize) -> BusFacts {
        let guessed = classify(&[name, file], size == BusSize::Articulated, None, (size == BusSize::Double).then_some(4.2));
        let class = match (size, guessed) {
            (BusSize::Midi, VehicleClass::Minibus) => VehicleClass::Minibus,
            (BusSize::Midi, _) => VehicleClass::Midi,
            (BusSize::Solo, c @ (VehicleClass::Coach | VehicleClass::Minibus)) => c,
            (BusSize::Solo, _) => VehicleClass::Solo,
            (BusSize::Articulated, _) => VehicleClass::Articulated,
            (BusSize::Double, _) => VehicleClass::Double,
        };
        BusFacts { file: file.to_string(), maker: String::new(), model: String::new(), name: name.to_string(), class }
    }
}

/// A bus chosen for a line by name: a whole maker (`model` and `file` empty), a model of it
/// (`file` empty), or one bus file. `files` are the installed buses it covered when it was
/// chosen (a fleet bus is known by its file only).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct BusPick {
    pub maker: String,
    pub model: String,
    pub file: String,
    pub files: Vec<String>,
    /// What the line editor shows of it ("Setra", "Mercedes-Benz Citaro", "Citaro · 3 doors").
    pub label: String,
}

impl BusPick {
    pub fn covers(&self, b: &BusFacts) -> bool {
        if !self.file.trim().is_empty() {
            return same_file(&self.file, &b.file);
        }
        if self.files.iter().any(|f| same_file(f, &b.file)) {
            return true;
        }
        let (maker, model) = (fold(&self.maker), fold(&self.model));
        if maker.is_empty() {
            return false;
        }
        if !b.maker.trim().is_empty() {
            return fold(&b.maker) == maker && (model.is_empty() || fold(&b.model) == model);
        }
        // (a fleet bus: its name says its maker and its model)
        let name = fold(&b.name);
        name.contains(&maker) && (model.is_empty() || name.contains(&model))
    }
}

/// The buses that run a line: the kinds of bus, and buses by name; none of either - any bus
/// of the line's depot.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct LineVehicles {
    pub classes: Vec<VehicleClass>,
    pub buses: Vec<BusPick>,
}

impl LineVehicles {
    /// Nothing chosen: any bus runs it.
    pub fn open(&self) -> bool {
        self.classes.is_empty() && self.buses.is_empty()
    }

    /// The bus runs the line: one of its kinds, or one of its buses by name.
    pub fn allows(&self, b: &BusFacts) -> bool {
        self.open() || self.classes.contains(&b.class) || self.buses.iter().any(|p| p.covers(b))
    }

    /// The kind of bus switched on or off.
    pub fn toggle_class(&mut self, c: VehicleClass) {
        match self.classes.iter().position(|x| *x == c) {
            Some(k) => {
                self.classes.remove(k);
            }
            None => {
                self.classes.push(c);
                self.classes.sort();
            }
        }
    }

    /// The smallest size the line asks for (None: any): what the company's demand model may
    /// not go under.
    pub fn sizes(&self) -> Vec<BusSize> {
        let mut v: Vec<BusSize> = self.classes.iter().map(|c| c.size()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// "Minibus, Setra": what is chosen, in a few words (English: the caller translates the
    /// kinds' labels itself when it wants them translated).
    pub fn summary(&self, tr: &dyn Fn(&str) -> String) -> String {
        let mut parts: Vec<String> = self.classes.iter().map(|c| tr(c.label())).collect();
        parts.extend(self.buses.iter().map(|p| p.label.clone()).filter(|s| !s.trim().is_empty()));
        parts.join(", ")
    }
}

// --- the day -------------------------------------------------------------------------------------

/// A yearly holiday period, from one day of the year to another (`MM-DD`, both counted; one
/// that ends before it begins goes over the new year).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct HolidayPeriod {
    pub name: String,
    pub from: String,
    pub to: String,
}

/// `MM-DD` as month × 100 + day (None: not one).
pub fn month_day(s: &str) -> Option<u32> {
    let (m, d) = s.trim().split_once('-')?;
    let (m, d): (u32, u32) = (m.trim().parse().ok()?, d.trim().parse().ok()?);
    ((1..=12).contains(&m) && (1..=dates::days_in_month(2024, m)).contains(&d)).then_some(m * 100 + d)
}

impl HolidayPeriod {
    pub fn new(name: &str, from: &str, to: &str) -> HolidayPeriod {
        HolidayPeriod { name: name.into(), from: from.into(), to: to.into() }
    }

    /// Both ends can be read.
    pub fn valid(&self) -> bool {
        month_day(&self.from).is_some() && month_day(&self.to).is_some()
    }

    /// The date code (YYYYMMDD) lies in it.
    pub fn covers(&self, code: i32) -> bool {
        let (Some(a), Some(b)) = (month_day(&self.from), month_day(&self.to)) else { return false };
        let x = (code % 10000) as u32;
        if a <= b {
            a <= x && x <= b
        } else {
            x >= a || x <= b
        }
    }
}

/// School holidays as a German or Dutch school year has them, by and large: a week in
/// February, two at Easter, six in the summer, two in the autumn and two at Christmas.
pub fn default_school_holidays() -> Vec<HolidayPeriod> {
    vec![
        HolidayPeriod::new("Spring holidays", "02-17", "02-23"),
        HolidayPeriod::new("Easter holidays", "03-30", "04-12"),
        HolidayPeriod::new("Summer holidays", "07-13", "08-24"),
        HolidayPeriod::new("Autumn holidays", "10-12", "10-25"),
        HolidayPeriod::new("Christmas holidays", "12-22", "01-05"),
    ]
}

/// Easter Sunday of a year (the Gregorian computus): (month, day).
pub fn easter(y: i32) -> (u32, u32) {
    let a = y % 19;
    let b = y / 100;
    let c = y % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = (h + l - 7 * m + 114) % 31 + 1;
    (month as u32, day as u32)
}

/// The public holidays most of Germany and the Netherlands share, for a map whose calendar
/// names none: New Year, Good Friday, Easter Monday, the first of May, Ascension, Whit Monday
/// and the two days of Christmas.
pub fn common_public_holiday(code: i32) -> bool {
    let (y, md) = (code / 10000, code % 10000);
    if matches!(md, 101 | 501 | 1225 | 1226) {
        return true;
    }
    let (em, ed) = easter(y);
    let e = dates::days_from_civil(y, em, ed);
    let Some(day) = day_of_code(code) else { return false };
    [-2, 1, 39, 50].iter().any(|k| e + k == day)
}

/// The day number of a date code (YYYYMMDD).
pub fn day_of_code(code: i32) -> Option<i64> {
    let (y, m, d) = (code / 10000, (code / 100 % 100) as u32, (code % 100) as u32);
    ((1..=12).contains(&m) && d >= 1 && d <= dates::days_in_month(y, m)).then(|| dates::days_from_civil(y, m, d))
}

/// The date code (YYYYMMDD) of a day number.
pub fn code_of_day(day: i64) -> i32 {
    let (y, m, d) = dates::civil_from_days(day);
    y * 10000 + m as i32 * 100 + d as i32
}

/// What day a date is: its weekday (0 Monday), and whether it is a public holiday or in the
/// school holidays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DayType {
    pub weekday: u32,
    pub public_holiday: bool,
    pub school_holiday: bool,
}

impl DayType {
    /// Its day group (`lines::DAY_GROUPS`): 0 a working day, 1 Saturday, 2 Sunday or a public
    /// holiday.
    pub fn group(&self) -> usize {
        if self.public_holiday || self.weekday == 6 {
            2
        } else if self.weekday == 5 {
            1
        } else {
            0
        }
    }

    /// The children go to school: Monday to Friday, no public holiday, no school holidays.
    pub fn school_day(&self) -> bool {
        self.weekday < 5 && !self.public_holiday && !self.school_holiday
    }

    /// The tour mask bits it selects, as the game takes them: (the weekday's or the public
    /// holiday's, the school holidays' or the school days').
    pub fn bits(&self) -> (i32, i32) {
        let day = if self.public_holiday { 1 << 7 } else { 1 << self.weekday };
        let school = if self.school_holiday { SCHOOL_HOLIDAY_BIT } else { SCHOOL_DAY_BIT };
        (day, school)
    }

    /// A line of `kind` runs that day at all.
    pub fn runs(&self, kind: ServiceKind) -> bool {
        match kind {
            ServiceKind::School => self.school_day(),
            ServiceKind::Leisure => self.group() > 0,
            ServiceKind::Regular | ServiceKind::OnDemand => true,
        }
    }
}

/// Where the days come from: the map's calendar (`Holidays.txt`), and - for what it does not
/// name - the periods of the line registry and the common public holidays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DayCalendar {
    /// The map's school holidays and public holidays (date codes).
    pub map_ranges: Vec<(i32, i32, String)>,
    pub map_holidays: Vec<i32>,
    /// The periods taken where the map has no school holidays.
    pub periods: Vec<HolidayPeriod>,
}

impl DayCalendar {
    /// The calendar of a map's `Holidays.txt` (`periods`: the registry's, empty for the
    /// defaults).
    pub fn new(map: &omsi_map::Calendar, periods: &[HolidayPeriod]) -> DayCalendar {
        let periods: Vec<HolidayPeriod> = periods.iter().filter(|p| p.valid()).cloned().collect();
        DayCalendar {
            map_ranges: map.ranges.iter().map(|r| (r.start, r.end, r.name.clone())).collect(),
            map_holidays: map.holidays.iter().map(|h| h.date).collect(),
            periods: if periods.is_empty() { default_school_holidays() } else { periods },
        }
    }

    /// The calendar of the map in `map_dir`.
    pub fn of_map(map_dir: &Path, periods: &[HolidayPeriod]) -> DayCalendar {
        DayCalendar::new(&omsi_map::Calendar::load(&map_dir.join("Holidays.txt")).unwrap_or_default(), periods)
    }

    /// The school holidays are the map's own (else the periods').
    pub fn map_school(&self) -> bool {
        !self.map_ranges.is_empty()
    }

    pub fn public_holiday(&self, code: i32) -> bool {
        if self.map_holidays.is_empty() {
            common_public_holiday(code)
        } else {
            self.map_holidays.contains(&code)
        }
    }

    pub fn school_holiday(&self, code: i32) -> bool {
        if self.map_school() {
            self.map_ranges.iter().any(|r| code >= r.0 && code <= r.1)
        } else {
            self.periods.iter().any(|p| p.covers(code))
        }
    }

    /// The day a date code (YYYYMMDD) is.
    pub fn day(&self, code: i32) -> DayType {
        let weekday = day_of_code(code).map(dates::weekday).unwrap_or(0);
        DayType { weekday, public_holiday: self.public_holiday(code), school_holiday: self.school_holiday(code) }
    }

    /// The day `YYYY-MM-DD` is (None: not a date).
    pub fn day_of(&self, date: &str) -> Option<DayType> {
        omsi_map::date_code(date).map(|c| self.day(c))
    }

    /// The school holidays that are on at `code` or come next, within a year: their name,
    /// first and last day (date codes).
    pub fn next_school_holidays(&self, code: i32) -> Option<(String, i32, i32)> {
        let start = day_of_code(code)?;
        let mut d = start;
        while d < start + 370 {
            let c = code_of_day(d);
            if self.school_holiday(c) {
                let mut first = d;
                while first > start - 60 && self.school_holiday(code_of_day(first - 1)) {
                    first -= 1;
                }
                let mut last = d;
                while last < d + 120 && self.school_holiday(code_of_day(last + 1)) {
                    last += 1;
                }
                let name = if self.map_school() {
                    self.map_ranges.iter().find(|r| c >= r.0 && c <= r.1).map(|r| r.2.clone()).unwrap_or_default()
                } else {
                    self.periods.iter().find(|p| p.covers(c)).map(|p| p.name.clone()).unwrap_or_default()
                };
                return Some((name, code_of_day(first), code_of_day(last)));
            }
            d += 1;
        }
        None
    }

    /// School days in a year from `code` on (for what a school contract brings in a month).
    pub fn school_days_in_year(&self, code: i32) -> u32 {
        let Some(start) = day_of_code(code) else { return 0 };
        (0..365).filter(|k| self.day(code_of_day(start + k)).school_day()).count() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_runs_on_its_days() {
        // the masks as the game reads them
        assert_eq!(ServiceKind::Regular.mask(0b0001_1111), 0b11_0001_1111);
        assert_eq!(ServiceKind::School.mask(0b0001_1111), 0b10_0001_1111);
        // (a school line's Saturday is no school day)
        assert_eq!(ServiceKind::School.mask(0b0010_0000), SCHOOL_DAY_BIT);
        assert_eq!(ServiceKind::Leisure.mask(0b1100_0000), 0b11_1100_0000);
        assert_eq!(ServiceKind::Leisure.mask(0b0001_1111), 0b11_0000_0000);
        assert!(ServiceKind::School.allows(0) && !ServiceKind::School.allows(1) && !ServiceKind::School.allows(2));
        assert!(!ServiceKind::Leisure.allows(0) && ServiceKind::Leisure.allows(1) && ServiceKind::Leisure.allows(2));
        assert!((0..3).all(|g| ServiceKind::Regular.allows(g) && ServiceKind::OnDemand.allows(g)));
        // a Tuesday in term, the same Tuesday in the holidays, a public holiday, a Saturday
        let term = DayType { weekday: 1, ..Default::default() };
        let holidays = DayType { school_holiday: true, ..term };
        let public = DayType { public_holiday: true, ..term };
        let saturday = DayType { weekday: 5, ..Default::default() };
        let school = ServiceKind::School.mask(0b0001_1111);
        assert!(mask_runs(school, term.bits()) && term.runs(ServiceKind::School));
        assert!(!mask_runs(school, holidays.bits()) && !holidays.runs(ServiceKind::School));
        assert!(!mask_runs(school, public.bits()) && !public.runs(ServiceKind::School));
        assert!(!mask_runs(school, saturday.bits()));
        // the weekend line on Saturday, on the public holiday (Sunday's tours), not on Tuesday
        let sat = ServiceKind::Leisure.mask(0b0010_0000);
        let sun = ServiceKind::Leisure.mask(0b1100_0000);
        assert!(mask_runs(sat, saturday.bits()) && mask_runs(sun, public.bits()));
        assert!(!mask_runs(sat, term.bits()) && !mask_runs(sun, term.bits()));
        assert!(saturday.runs(ServiceKind::Leisure) && public.runs(ServiceKind::Leisure) && !term.runs(ServiceKind::Leisure));
        // a regular line in the holidays as in term
        let reg = ServiceKind::Regular.mask(0b0001_1111);
        assert!(mask_runs(reg, term.bits()) && mask_runs(reg, holidays.bits()));
        assert_eq!((public.group(), saturday.group(), term.group()), (2, 1, 0));
    }

    #[test]
    fn the_calendar_knows_the_holidays() {
        assert_eq!(easter(2024), (3, 31));
        assert_eq!(easter(1989), (3, 26));
        assert_eq!(easter(2025), (4, 20));
        // without a calendar of the map's: the common holidays and the default periods
        let cal = DayCalendar::new(&omsi_map::Calendar::default(), &[]);
        assert!(!cal.map_school());
        // Good Friday 2024 (29 March), Easter Monday, Ascension (9 May), Whit Monday (20 May)
        for c in [20240329, 20240401, 20240509, 20240520, 20241225, 20240101] {
            assert!(cal.public_holiday(c), "{c}");
        }
        assert!(!cal.public_holiday(20240402));
        // a Tuesday in term, one in the summer holidays, Christmas over the new year
        let d = cal.day(20240604);
        assert_eq!(d.weekday, 1);
        assert!(d.school_day());
        assert!(cal.day(20240730).school_holiday && !cal.day(20240730).school_day());
        assert!(cal.school_holiday(20241230) && cal.school_holiday(20250103) && !cal.school_holiday(20250107));
        // the next holidays from early June: the summer's
        let (name, first, last) = cal.next_school_holidays(20240604).unwrap();
        assert_eq!((name.as_str(), first, last), ("Summer holidays", 20240713, 20240824));
        // about 190 school days a year
        let n = cal.school_days_in_year(20240101);
        assert!((175..=200).contains(&n), "{n}");
        // a map's own calendar comes first; periods of the player's when it has none
        let map = omsi_map::Calendar { ranges: vec![omsi_map::calendar::HolidayRange { start: 20240601, end: 20240610, name: "Pfingstferien".into() }], holidays: vec![omsi_map::calendar::Holiday { date: 20240603, name: "X".into() }] };
        let cal = DayCalendar::new(&map, &[]);
        assert!(cal.map_school() && cal.school_holiday(20240605) && !cal.school_holiday(20240730));
        assert!(cal.public_holiday(20240603) && !cal.public_holiday(20241225));
        let own = DayCalendar::new(&omsi_map::Calendar::default(), &[HolidayPeriod::new("Kermis", "09-01", "09-07"), HolidayPeriod::new("broken", "13-01", "x")]);
        assert_eq!(own.periods.len(), 1);
        assert!(own.school_holiday(20240903) && !own.school_holiday(20240730));
    }

    #[test]
    fn a_line_takes_its_buses() {
        let setra = BusFacts { file: "Vehicles/Setra/S415UL.bus".into(), maker: "Setra".into(), model: "S 415 UL".into(), name: "Setra S 415 UL".into(), class: VehicleClass::Coach };
        let citaro = BusFacts { file: "Vehicles/MB_Citaro/Citaro.bus".into(), maker: "Mercedes-Benz".into(), model: "Citaro".into(), name: "Mercedes-Benz Citaro".into(), class: VehicleClass::Solo };
        let sprinter = BusFacts { file: "Vehicles/Sprinter/Sprinter.bus".into(), maker: "Mercedes-Benz".into(), model: "Sprinter".into(), name: "Mercedes-Benz Sprinter".into(), class: VehicleClass::Minibus };
        let open = LineVehicles::default();
        assert!(open.open() && open.allows(&setra) && open.allows(&citaro));
        // a kind of bus
        let mut v = LineVehicles::default();
        v.toggle_class(VehicleClass::Minibus);
        assert!(v.allows(&sprinter) && !v.allows(&citaro));
        // and a maker: every Setra
        v.buses.push(BusPick { maker: "Setra".into(), label: "Setra".into(), ..Default::default() });
        assert!(v.allows(&setra) && !v.allows(&citaro));
        // a model of a maker; one bus file
        let model = BusPick { maker: "Mercedes-Benz".into(), model: "Citaro".into(), ..Default::default() };
        assert!(model.covers(&citaro) && !model.covers(&sprinter));
        let file = BusPick { file: "vehicles\\mb_citaro\\citaro.bus".into(), ..Default::default() };
        assert!(file.covers(&citaro) && !file.covers(&setra));
        // a fleet bus is known by its name and file
        let fleet = BusFacts::of_fleet("Vehicles/Setra/S415UL.bus", "Setra S 415 UL", BusSize::Solo);
        assert_eq!(fleet.class, VehicleClass::Coach);
        assert!(v.allows(&fleet));
        assert!(model.covers(&BusFacts::of_fleet("Vehicles/X/C2.bus", "Mercedes-Benz Citaro C2", BusSize::Solo)));
        let picked_files = BusPick { maker: "Nobody".into(), files: vec!["Vehicles/Setra/S415UL.bus".into()], ..Default::default() };
        assert!(picked_files.covers(&fleet));
        // switched off again
        v.toggle_class(VehicleClass::Minibus);
        assert!(!v.allows(&sprinter) && v.classes.is_empty());
        assert_eq!(v.summary(&|s| s.to_string()), "Setra");
    }

    #[test]
    fn a_bus_is_told_by_its_names_and_size() {
        assert_eq!(classify(&["Mercedes-Benz Sprinter City"], false, Some(8.5), Some(2.9)), VehicleClass::Minibus);
        assert_eq!(classify(&["MAN Lion's City M"], false, Some(10.5), None), VehicleClass::Midi);
        assert_eq!(classify(&["Mercedes-Benz Sprinter"], false, Some(7.4), Some(2.9)), VehicleClass::Minibus);
        assert_eq!(classify(&["VW Crafter Taxibus"], false, None, None), VehicleClass::Minibus);
        assert_eq!(classify(&["Setra S 415 UL"], false, Some(12.2), Some(3.2)), VehicleClass::Coach);
        assert_eq!(classify(&["Mercedes-Benz Tourismo"], false, Some(12.1), Some(3.6)), VehicleClass::Coach);
        assert_eq!(classify(&["MAN Lion's City"], false, Some(12.0), Some(3.0)), VehicleClass::Solo);
        assert_eq!(classify(&["MAN Lion's City G"], true, Some(12.0), None), VehicleClass::Articulated);
        assert_eq!(classify(&["MAN SD202"], false, Some(11.0), Some(4.2)), VehicleClass::Double);
        assert_eq!(classify(&["Ulm Citaro"], false, Some(12.0), None), VehicleClass::Solo);
        assert_eq!(VehicleClass::Coach.size(), BusSize::Solo);
        assert_eq!(maker_model("", "Gelenkbus - 18C - 3 Tuerer", "Vehicles/MAN_NG/x.bus", "MAN_NG"), ("MAN NG".to_string(), "Gelenkbus".to_string()));
        assert_eq!(maker_model("Setra", "", "Vehicles/Setra/S_415_UL.bus", "Setra").1, "S 415 UL");
    }

    #[test]
    fn periods_read_their_days() {
        assert_eq!(month_day("07-13"), Some(713));
        assert_eq!(month_day("02-30"), None);
        assert_eq!(month_day("x"), None);
        let p = HolidayPeriod::new("Christmas", "12-22", "01-05");
        assert!(p.covers(20241222) && p.covers(20250105) && !p.covers(20250106) && !p.covers(20241221));
        assert!(!HolidayPeriod::new("", "", "").covers(20240101));
    }
}
