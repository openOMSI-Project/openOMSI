//! Signing on for a duty, the way Omsi-Hub's phone does it: the driver types a personnel
//! number and then a code on a keypad, signs the duty order, and only then gets the duty's
//! data on the IBIS - as at a real depot, where the codes come after the signature.
//!
//! The number and the code belong to the driver (`Drivers/<name>.odr` says nothing of
//! them, so they are made the first time and kept in `~/.openomsi/personnel.json`): six and
//! four digits, neither starting with a nought (a leading nought reads as a typing error on a
//! keypad, and whoever copies the number leaves it out). Checking happens here and never on
//! a phone: a device on the network learns how many digits there are, not what they are.
//!
//! Ten wrong tries within a minute and every answer is "wrong" for the rest of that minute:
//! a four-digit code is guessed quickly over the network otherwise, and a driver who mistypes
//! loses nothing by it.
//!
//! All of it is asked for only when the player chose so (the setting `nav_signon`, off as it
//! comes): otherwise the driver counts as signed on and a duty as signed for as soon as it is
//! there, and the navigator shows the map and the duty at once ([`SignOn::auto`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Wrong tries allowed within [`MISS_WINDOW`] seconds.
pub(crate) const MAX_MISSES: u32 = 10;
pub(crate) const MISS_WINDOW: f64 = 60.0;

/// A driver's personnel number and code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Personnel {
    pub number: String,
    pub code: String,
}

impl Personnel {
    /// A new number (six digits) and code (four), from `rnd` (a source of random numbers).
    pub(crate) fn generate(rnd: &mut dyn FnMut() -> u64) -> Personnel {
        Personnel { number: digits(6, rnd), code: digits(4, rnd) }
    }

    /// Both are plain digits of a sensible length (a hand-edited file may hold anything).
    fn valid(&self) -> bool {
        let ok = |s: &str, n: std::ops::RangeInclusive<usize>| n.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
        ok(&self.number, 3..=10) && ok(&self.code, 3..=8)
    }
}

/// `n` digits, the first of them 1-9.
fn digits(n: usize, rnd: &mut dyn FnMut() -> u64) -> String {
    (0..n)
        .map(|i| {
            let d = if i == 0 { 1 + rnd() % 9 } else { rnd() % 10 };
            char::from(b'0' + d as u8)
        })
        .collect()
}

/// The personnel data of every driver this game has signed on, by driver key (see
/// [`driver_key`]), and the file they are kept in.
#[derive(Debug, Default)]
pub(crate) struct PersonnelFile {
    pub path: Option<PathBuf>,
    pub drivers: BTreeMap<String, Personnel>,
}

impl PersonnelFile {
    /// `~/.openomsi/personnel.json` (an unreadable or missing file is an empty one).
    pub(crate) fn load(path: Option<PathBuf>) -> PersonnelFile {
        let drivers = path.as_deref().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse_personnel(&t)).unwrap_or_default();
        PersonnelFile { path, drivers }
    }

    /// The data of `key`, made (and written down) the first time it is asked for. Written
    /// once and kept: made anew at every start, the number a driver learnt yesterday would
    /// be wrong today.
    pub(crate) fn of(&mut self, key: &str, rnd: &mut dyn FnMut() -> u64) -> Personnel {
        if let Some(p) = self.drivers.get(key).filter(|p| p.valid()) {
            return p.clone();
        }
        let p = Personnel::generate(rnd);
        self.drivers.insert(key.to_string(), p.clone());
        if let Some(path) = self.path.as_deref() {
            // (when it cannot be written the sign-on still works this time, and the driver
            // gets another number next time - better than none)
            if let Err(e) = write_private(path, &personnel_json(&self.drivers)) {
                log::warn!("companion: cannot write {}: {e}", path.display());
            }
        }
        log::info!("companion: personnel number made for driver '{key}'");
        p
    }
}

fn parse_personnel(text: &str) -> BTreeMap<String, Personnel> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return BTreeMap::new() };
    let Some(map) = v.get("drivers").and_then(|d| d.as_object()) else { return BTreeMap::new() };
    map.iter()
        .filter_map(|(k, e)| {
            let number = e.get("number")?.as_str()?.to_string();
            let code = e.get("code")?.as_str()?.to_string();
            Some((k.clone(), Personnel { number, code }))
        })
        .collect()
}

fn personnel_json(drivers: &BTreeMap<String, Personnel>) -> String {
    let map: serde_json::Map<String, serde_json::Value> = drivers.iter().map(|(k, p)| (k.clone(), serde_json::json!({ "number": p.number, "code": p.code }))).collect();
    serde_json::to_string_pretty(&serde_json::json!({ "drivers": map })).unwrap_or_default()
}

/// Write `text` to `path`, readable by this user only where the system has such a thing.
pub(crate) fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path)?.write_all(text.as_bytes())
}

/// The key a driver's personnel data is kept under: the name of the personnel file (what the
/// game menu's "Driver..." lists), lower case; "driver" when the game runs without one.
pub(crate) fn driver_key(file_stem: Option<&str>) -> String {
    let k = file_stem.map(|s| s.trim().to_lowercase()).unwrap_or_default();
    if k.is_empty() { "driver".into() } else { k }
}

/// What a duty is known by: map, line and tour, the time it starts, and every trip with its
/// departure. Omsi-Hub's `aanmeldSleutelVan`: map, tour and start alone are too coarse - a
/// duty taken on can match a free run before it in those, and then the free run's signature
/// would have stood for it. Empty without trips.
pub(crate) fn duty_key(map: &str, line: &str, tour: &str, trips: &[(&str, f64)]) -> String {
    let Some(first) = trips.first() else { return String::new() };
    let minute = |s: f64| (s / 60.0).round() as i64;
    let legs: Vec<String> = trips.iter().map(|(name, dep)| format!("{}@{}", name.trim(), minute(*dep))).collect();
    format!("{map}|{}/{}|{}#{map}|{}", line.trim(), tour.trim(), minute(first.1), legs.join(";"))
}

/// What a try at signing on came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Attempt {
    /// The number is right: now the code (a try without a code asks only this).
    Number,
    SignedOn,
    Wrong,
}

impl Attempt {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Attempt::Number => "number",
            Attempt::SignedOn => "signed_on",
            Attempt::Wrong => "wrong",
        }
    }
}

/// Where the driver is in the order of things.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Stage {
    /// Not signed on: the keypad.
    #[default]
    SignOn,
    /// Signed on, no duty yet: the duty menu (lines and tours), or driving without one.
    DutyMenu,
    /// A duty is there and waits for the driver's signature (the duty order).
    DutyOrder,
    /// At work: the duty signed for, or driving freely by choice.
    OnDuty,
}

impl Stage {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Stage::SignOn => "sign_on",
            Stage::DutyMenu => "duty_menu",
            Stage::DutyOrder => "duty_order",
            Stage::OnDuty => "on_duty",
        }
    }
}

/// The phone's state of a driver: signed on or not, the duty signed for, a break.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SignOn {
    pub signed_on: bool,
    /// The duty order signed (or the duty picked on the phone: picking it is signing it).
    pub accepted: bool,
    /// The driver chose to drive without a duty.
    pub free: bool,
    /// When the break began (seconds of the day, the game's clock): the scheduled break is
    /// timed in game time, as OMSI may run faster or slower than the wall clock.
    pub break_since: Option<f64>,
    /// Signing on is not asked for (the setting `nav_signon` off, as it comes): the driver
    /// counts as signed on, a duty as signed for as soon as it is there and no duty as driving
    /// freely - the navigator shows the map and the duty at once.
    pub auto: bool,
    driver: String,
    duty: String,
    misses: u32,
    misses_since: Option<f64>,
}

impl SignOn {
    /// The driver now (`driver_key`): another one starts afresh, signed off.
    pub(crate) fn follow_driver(&mut self, key: &str) {
        if self.driver != key {
            let first = self.driver.is_empty();
            *self = SignOn { driver: key.to_string(), duty: std::mem::take(&mut self.duty), auto: self.auto, misses: self.misses, misses_since: self.misses_since, ..Default::default() };
            if !first {
                log::info!("companion: driver changed to '{key}': signed off");
            }
        }
    }

    /// Whether signing on with the number and the code is asked for (the setting
    /// `nav_signon`); see [`SignOn::auto`].
    pub(crate) fn ask_sign_on(&mut self, ask: bool) {
        self.auto = !ask;
    }

    /// The duty now ([`duty_key`], empty for none): another one has to be signed for again
    /// (the driver stays signed on - the same driver in the same bus). True when it is another
    /// duty than before.
    pub(crate) fn follow_duty(&mut self, key: &str) -> bool {
        if self.duty == key {
            return false;
        }
        self.duty = key.to_string();
        self.accepted = false;
        self.break_since = None;
        if !key.is_empty() {
            self.free = false;
        }
        true
    }

    /// Signed on: with the number and the code, or without when that is not asked for.
    pub(crate) fn is_signed_on(&self) -> bool {
        self.signed_on || self.auto
    }

    /// The duty order signed (by itself when signing on is not asked for).
    pub(crate) fn is_accepted(&self) -> bool {
        self.accepted || (self.auto && !self.duty.is_empty())
    }

    /// Driving without a duty: chosen, or simply so when signing on is not asked for.
    pub(crate) fn is_free(&self) -> bool {
        self.free || (self.auto && self.duty.is_empty())
    }

    pub(crate) fn duty(&self) -> &str {
        &self.duty
    }

    /// A try with `number` and, as the second step, `code`. `now` in seconds of any steady
    /// clock (the misses are counted per minute of it).
    pub(crate) fn attempt(&mut self, p: &Personnel, number: &str, code: Option<&str>, now: f64) -> Attempt {
        if self.misses_since.is_none_or(|t| now - t > MISS_WINDOW) {
            self.misses_since = Some(now);
            self.misses = 0;
        }
        if self.misses >= MAX_MISSES {
            return Attempt::Wrong;
        }
        if number.trim() != p.number {
            self.misses += 1;
            return Attempt::Wrong;
        }
        let Some(code) = code else { return Attempt::Number };
        if code.trim() != p.code {
            self.misses += 1;
            return Attempt::Wrong;
        }
        self.misses = 0;
        if !self.signed_on {
            log::info!("companion: driver '{}' signed on", self.driver);
        }
        self.signed_on = true;
        Attempt::SignedOn
    }

    pub(crate) fn sign_off(&mut self) {
        let driver = std::mem::take(&mut self.driver);
        let duty = std::mem::take(&mut self.duty);
        *self = SignOn { driver, duty, auto: self.auto, misses: self.misses, misses_since: self.misses_since, ..Default::default() };
    }

    /// Sign the duty order: only when signed on and there is a duty.
    pub(crate) fn accept(&mut self) -> bool {
        if !self.is_signed_on() || self.duty.is_empty() {
            return false;
        }
        self.accepted = true;
        true
    }

    /// Drive without a duty (signed on, none there).
    pub(crate) fn drive_free(&mut self) -> bool {
        if !self.is_signed_on() || !self.duty.is_empty() {
            return false;
        }
        self.free = true;
        true
    }

    /// A break from `at` (seconds of the day), or none.
    pub(crate) fn set_break(&mut self, at: Option<f64>) -> bool {
        if self.stage() != Stage::OnDuty {
            return false;
        }
        self.break_since = at.filter(|t| t.is_finite());
        true
    }

    pub(crate) fn stage(&self) -> Stage {
        match (self.is_signed_on(), !self.duty.is_empty()) {
            (false, _) => Stage::SignOn,
            (true, true) if self.is_accepted() => Stage::OnDuty,
            (true, true) => Stage::DutyOrder,
            (true, false) if self.is_free() => Stage::OnDuty,
            (true, false) => Stage::DutyMenu,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counter() -> impl FnMut() -> u64 {
        let mut n = 0u64;
        move || {
            n = n.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            n >> 11
        }
    }

    fn hans() -> Personnel {
        Personnel { number: "482913".into(), code: "5821".into() }
    }

    #[test]
    fn personnel_numbers_have_six_and_four_digits_without_a_leading_nought() {
        let mut rnd = counter();
        for _ in 0..200 {
            let p = Personnel::generate(&mut rnd);
            assert_eq!(p.number.len(), 6);
            assert_eq!(p.code.len(), 4);
            assert!(p.number.bytes().all(|b| b.is_ascii_digit()) && p.code.bytes().all(|b| b.is_ascii_digit()));
            assert_ne!(p.number.as_bytes()[0], b'0');
            assert_ne!(p.code.as_bytes()[0], b'0');
        }
        // (a source that only gives noughts still gives a number that does not start with one)
        let p = Personnel::generate(&mut || 0);
        assert_eq!((p.number.as_str(), p.code.as_str()), ("100000", "1000"));
    }

    #[test]
    fn the_number_comes_first_then_the_code() {
        let mut s = SignOn::default();
        s.follow_driver("hans");
        assert_eq!(s.attempt(&hans(), "482913", None, 0.0), Attempt::Number);
        assert!(!s.signed_on);
        assert_eq!(s.attempt(&hans(), "482913", Some("1111"), 1.0), Attempt::Wrong);
        assert_eq!(s.attempt(&hans(), "482913", Some("5821"), 2.0), Attempt::SignedOn);
        assert!(s.signed_on);
        assert_eq!(s.stage(), Stage::DutyMenu);
    }

    #[test]
    fn ten_misses_in_a_minute_lock_the_keypad_until_the_minute_is_over() {
        let mut s = SignOn::default();
        for i in 0..MAX_MISSES {
            assert_eq!(s.attempt(&hans(), "000000", None, i as f64), Attempt::Wrong);
        }
        // (even the right number and code are refused now)
        assert_eq!(s.attempt(&hans(), "482913", Some("5821"), 30.0), Attempt::Wrong);
        assert!(!s.signed_on);
        assert_eq!(s.attempt(&hans(), "482913", Some("5821"), 61.0), Attempt::SignedOn);
    }

    #[test]
    fn a_duty_order_is_signed_and_another_duty_has_to_be_signed_again() {
        let mut s = SignOn::default();
        s.follow_driver("hans");
        s.follow_duty("grundorf|1/3|330#grundorf|t1@330");
        assert!(!s.accept(), "not signed on yet");
        s.attempt(&hans(), "482913", Some("5821"), 0.0);
        assert_eq!(s.stage(), Stage::DutyOrder);
        assert!(s.accept());
        assert_eq!(s.stage(), Stage::OnDuty);
        assert!(s.set_break(Some(36_000.0)));
        // the same duty again changes nothing
        s.follow_duty("grundorf|1/3|330#grundorf|t1@330");
        assert_eq!((s.stage(), s.break_since), (Stage::OnDuty, Some(36_000.0)));
        // another: still signed on, the order to be signed, the break over
        s.follow_duty("grundorf|1/4|400#grundorf|t2@400");
        assert_eq!((s.stage(), s.break_since, s.signed_on), (Stage::DutyOrder, None, true));
    }

    #[test]
    fn driving_freely_is_a_choice_that_a_new_duty_ends() {
        let mut s = SignOn::default();
        assert!(!s.drive_free());
        s.attempt(&hans(), "482913", Some("5821"), 0.0);
        assert!(s.drive_free());
        assert_eq!(s.stage(), Stage::OnDuty);
        s.follow_duty("grundorf|1/3|330#grundorf|t1@330");
        assert_eq!(s.stage(), Stage::DutyOrder);
        assert!(!s.drive_free(), "there is a duty");
    }

    #[test]
    fn another_driver_signs_off_and_signing_off_keeps_the_duty() {
        let mut s = SignOn::default();
        s.follow_driver("hans");
        s.follow_duty("k");
        s.attempt(&hans(), "482913", Some("5821"), 0.0);
        s.accept();
        s.follow_driver("hans");
        assert!(s.signed_on);
        s.follow_driver("greta");
        assert_eq!((s.signed_on, s.accepted, s.duty()), (false, false, "k"));
        s.attempt(&hans(), "482913", Some("5821"), 1.0);
        s.sign_off();
        assert_eq!((s.signed_on, s.duty()), (false, "k"));
        assert_eq!(s.stage(), Stage::SignOn);
    }

    /// Signing on not asked for (as it comes): no keypad and no duty order - a duty is driven
    /// as soon as it is there, no duty is a free drive, whoever drives.
    #[test]
    fn without_signing_on_a_duty_is_driven_at_once() {
        let mut s = SignOn::default();
        s.ask_sign_on(false);
        s.follow_driver("hans");
        assert_eq!(s.stage(), Stage::OnDuty, "no duty: a free drive");
        assert!(s.is_free() && s.is_signed_on() && !s.is_accepted());
        assert!(s.follow_duty("grundorf|1/3|330#grundorf|t1@330"), "a new duty");
        assert_eq!(s.stage(), Stage::OnDuty);
        assert!(s.is_accepted() && !s.is_free());
        assert!(s.set_break(Some(36_000.0)));
        assert!(!s.follow_duty("grundorf|1/3|330#grundorf|t1@330"), "the same duty");
        s.follow_duty("grundorf|1/4|400#grundorf|t2@400");
        assert_eq!((s.stage(), s.break_since), (Stage::OnDuty, None));
        // another driver, or signing off, changes nothing of it
        s.follow_driver("greta");
        assert_eq!(s.stage(), Stage::OnDuty);
        s.sign_off();
        assert_eq!(s.stage(), Stage::OnDuty);
        // the duty over: a free drive again
        s.follow_duty("");
        assert_eq!(s.stage(), Stage::OnDuty);
        assert!(s.is_free());
    }

    /// Signing on asked for: as it always was - the number and the code, then the duty order;
    /// switched on in the middle of a duty, both are asked for.
    #[test]
    fn with_signing_on_the_keypad_and_the_order_come_first() {
        let mut s = SignOn::default();
        s.ask_sign_on(true);
        s.follow_driver("hans");
        s.follow_duty("k");
        assert_eq!(s.stage(), Stage::SignOn);
        assert!(!s.accept() && !s.drive_free());
        s.attempt(&hans(), "482913", Some("5821"), 0.0);
        assert_eq!(s.stage(), Stage::DutyOrder);
        assert!(s.accept());
        assert_eq!(s.stage(), Stage::OnDuty);
        let mut t = SignOn::default();
        t.ask_sign_on(false);
        t.follow_driver("hans");
        t.follow_duty("k");
        assert_eq!(t.stage(), Stage::OnDuty);
        t.ask_sign_on(true);
        assert_eq!(t.stage(), Stage::SignOn);
    }

    #[test]
    fn a_duty_is_known_by_its_trips_and_their_times() {
        let a = duty_key("Grundorf", "1", "3", &[("1_hin", 19_800.0), ("1_rueck", 21_600.0)]);
        assert_eq!(a, "Grundorf|1/3|330#Grundorf|1_hin@330;1_rueck@360");
        // the same tour starting later is another duty
        assert_ne!(a, duty_key("Grundorf", "1", "3", &[("1_hin", 23_400.0), ("1_rueck", 25_200.0)]));
        assert_eq!(duty_key("Grundorf", "1", "3", &[]), "");
    }

    #[test]
    fn the_personnel_file_keeps_a_drivers_number_and_mends_a_broken_entry() {
        let dir = std::env::temp_dir().join(format!("oo-companion-personnel-{}", std::process::id()));
        let path = dir.join("personnel.json");
        let _ = std::fs::remove_file(&path);
        let mut rnd = counter();
        let mut f = PersonnelFile::load(Some(path.clone()));
        let first = f.of("hans", &mut rnd);
        let again = PersonnelFile::load(Some(path.clone())).of("hans", &mut rnd);
        assert_eq!(first, again, "read back from the file, not made anew");
        std::fs::write(&path, r#"{"drivers":{"hans":{"number":"12ab","code":""}}}"#).unwrap();
        let mended = PersonnelFile::load(Some(path.clone())).of("hans", &mut rnd);
        assert!(mended.valid());
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(driver_key(Some(" Hans Müller ")), "hans müller");
        assert_eq!(driver_key(None), "driver");
    }
}
