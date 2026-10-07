//! What else can happen on a trip the company's drivers run (the modelled day, `day`), and
//! what their courses change of it (Luc: "meer trainingen ... met een effect dat de simulatie
//! echt gebruikt"):
//!
//! - *accidents* - a scrape at a stop, a mirror, a bumper: damage to repair and a little of
//!   the reputation - and *traffic fines* (a speed camera, a red light): fewer with a better
//!   driver, half of them after the defensive driving course;
//! - *complaints* of passengers: fewer with more service, half of them after the customer
//!   service course - each costs a little of the reputation;
//! - a passenger *taken ill or hurt* on board: a first-aider at the wheel turns it from bad
//!   news into a thank-you letter (less reputation lost);
//! - *fare dodgers*: a driver trained in ticket sales checks and sells, and the trip brings
//!   4 % more fares.
//!
//! The rates are small and per trip: a company of twenty tours a day sees an accident a
//! month or so, a few complaints a week. Their own random numbers (`Rng::of(.., "incidents")`)
//! keep the rest of the day as it was.

use super::model::{BusSize, Cents, Company, Employee};
use super::rng::Rng;
use super::training::{self, CourseKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The courses of a driver that count here.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Trained {
    pub safety: bool,
    pub service: bool,
    pub first_aid: bool,
    pub ticketing: bool,
}

/// The courses done of every employee.
pub fn trained(c: &Company) -> HashMap<u32, Trained> {
    c.staff
        .iter()
        .map(|e| {
            let t = |k| training::trained(c, Some(e.id), k);
            (e.id, Trained { safety: t(CourseKind::Safety), service: t(CourseKind::CustomerService), first_aid: t(CourseKind::FirstAid), ticketing: t(CourseKind::Ticketing) })
        })
        .collect()
}

/// What ticket sales add to a trip's fares.
pub const TICKETING_GAIN: f64 = 0.04;

/// The factor on a trip's fares for its driver.
pub fn fare_factor(t: &Trained) -> f64 {
    if t.ticketing {
        1.0 + TICKETING_GAIN
    } else {
        1.0
    }
}

/// The chance of an accident on a trip, of a traffic fine, of a complaint, and of a
/// passenger taken ill or hurt (with `pax` aboard in all).
pub fn chances(e: &Employee, t: &Trained, pax: u32) -> (f64, f64, f64, f64) {
    let driving = (e.skills.driving / 100.0).clamp(0.0, 1.0);
    let service = (e.skills.service / 100.0).clamp(0.0, 1.0);
    let safe = if t.safety { 0.5 } else { 1.0 };
    let accident = 0.0006 * (1.3 - driving) * safe;
    let fine = 0.002 * (1.2 - driving) * safe;
    let complaint = 0.006 * (1.4 - service) * if t.service { 0.5 } else { 1.0 };
    let ill = 0.0003 * (pax as f64 / 20.0).min(4.0);
    (accident, fine, complaint, ill)
}

/// What went wrong on the day's trips.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Tally {
    /// The bus (its number) and the damage.
    pub accidents: Vec<(String, Cents)>,
    pub fines: u32,
    pub fine_cost: Cents,
    pub complaints: u32,
    /// Passengers taken ill or hurt on board, and how many of them had a first-aider at the
    /// wheel.
    pub taken_ill: u32,
    pub helped: u32,
}

impl Tally {
    /// What the day does to the reputation (at most two points down).
    pub fn reputation(&self) -> f64 {
        let hurt = 0.4 * self.accidents.len() as f64 + 0.04 * self.complaints as f64 + 0.3 * (self.taken_ill - self.helped) as f64 + 0.05 * self.helped as f64;
        -hurt.min(2.0)
    }
}

/// One trip of `e` on the bus `number` of `size` with `pax` aboard: what happened, into
/// `tally`.
pub fn trip(rng: &mut Rng, e: &Employee, t: &Trained, number: &str, size: BusSize, pax: u32, price_index: f64, tally: &mut Tally) {
    let (accident, fine, complaint, ill) = chances(e, t, pax);
    if rng.chance(accident) {
        let size = match size {
            BusSize::Midi => 0.8,
            BusSize::Solo => 1.0,
            _ => 1.35,
        };
        let cost = ((rng.range(600.0, 4_000.0) * size * price_index).round() as Cents) * 100;
        tally.accidents.push((number.to_string(), cost));
    }
    if rng.chance(fine) {
        tally.fines += 1;
        tally.fine_cost += ((rng.range(30.0, 120.0) * price_index).round() as Cents) * 100;
    }
    if rng.chance(complaint) {
        tally.complaints += 1;
    }
    if rng.chance(ill) {
        tally.taken_ill += 1;
        if t.first_aid {
            tally.helped += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driver(driving: f64, service: f64) -> Employee {
        let mut e = super::super::plan::agency_driver();
        e.skills.driving = driving;
        e.skills.service = service;
        e
    }

    #[test]
    fn courses_make_trips_safer_and_kinder() {
        let e = driver(50.0, 50.0);
        let none = Trained::default();
        let all = Trained { safety: true, service: true, first_aid: true, ticketing: true };
        let (a0, f0, c0, i0) = chances(&e, &none, 40);
        let (a1, f1, c1, i1) = chances(&e, &all, 40);
        assert!((a1 - a0 * 0.5).abs() < 1e-12 && (f1 - f0 * 0.5).abs() < 1e-12 && (c1 - c0 * 0.5).abs() < 1e-12);
        assert_eq!(i0, i1, "first aid does not keep people from falling ill");
        // a better driver has fewer accidents
        assert!(chances(&driver(90.0, 50.0), &none, 40).0 < a0);
        assert_eq!(fare_factor(&all), 1.04);
        assert_eq!(fare_factor(&none), 1.0);
    }

    #[test]
    fn a_tally_costs_reputation_and_first_aid_softens_it() {
        let ill = Tally { taken_ill: 2, ..Default::default() };
        let helped = Tally { taken_ill: 2, helped: 2, ..Default::default() };
        assert!(helped.reputation() > ill.reputation());
        let bad = Tally { accidents: vec![("101".into(), 1_000_00); 9], complaints: 50, ..Default::default() };
        assert_eq!(bad.reputation(), -2.0);
        // many trips: the trained driver has fewer complaints and fines
        let e = driver(40.0, 40.0);
        let (mut plain, mut trained) = (Tally::default(), Tally::default());
        let mut rng = Rng::new(7);
        for _ in 0..20_000 {
            trip(&mut rng, &e, &Trained::default(), "101", BusSize::Solo, 30, 1.0, &mut plain);
        }
        let mut rng = Rng::new(7);
        for _ in 0..20_000 {
            trip(&mut rng, &e, &Trained { safety: true, service: true, first_aid: true, ticketing: true }, "101", BusSize::Solo, 30, 1.0, &mut trained);
        }
        assert!(trained.complaints < plain.complaints && trained.fines < plain.fines, "{plain:?} {trained:?}");
        assert!(trained.accidents.len() <= plain.accidents.len());
        assert_eq!(trained.helped, trained.taken_ill);
    }
}
