//! Fares: the single ticket's price a line asks, and how its passengers take it.
//!
//! The fare association (the Verkehrsverbund) sets the fares of the map's lines: the operator
//! of a concession may only move within a band round the association's single ticket - ±20 %
//! on Easy and Realistic, ±10 % on Hard. An own line's fare is the company's to set, from half
//! the association's to two and a half times it. What a passenger pays on average is far
//! below a single ticket (most ride on day, week and season tickets): the fares booked per
//! passenger are the difficulty's average (`Rules::fare`) at the association's price, and move
//! with the line's fare in proportion - `TICKET_MIX` is that average as a share of a single.
//!
//! Demand answers the price with the elasticities the studies give for town buses. Balcombe
//! et al., "The demand for public transport: a practical guide" (TRL Report 593, 2004): bus
//! fares about -0.4 in the short run, -0.56 in the medium and -1.0 in the long run; Paulley et
//! al., "The demand for public transport: the effects of fares, quality of service, income and
//! car ownership" (Transport Policy 13, 2006) the same; Litman, "Transit Price Elasticities and
//! Cross-Elasticities" (VTPI, 2004, updated 2023): -0.2 to -0.5 short run, -0.6 to -0.9 long
//! run. Taken here: `SHORT_RUN` -0.35 and `LONG_RUN` -0.7.
//!
//! The curve is semi-logarithmic - passengers = base · exp(ε · (p/p₀ − 1)) - so that its point
//! elasticity is ε at the association's fare p₀ and grows with the price (ε · p/p₀): a fare
//! far above the association's loses ever more riders, and the fares' revenue is at its
//! highest at p₀/|ε| (`best_fare`), about 1.4 times the association's. A cheaper fare draws
//! riders, up to `MOST` times as many (an own line's buses still leave behind whom they
//! cannot take, `ownline::carried`).
//!
//! The answer comes with a lag (a partial adjustment, as the studies model it): a change is
//! felt at once with the short-run elasticity, and the rest towards the long run comes over
//! the next weeks, `LAG_DAYS` the time constant (`settle`, every closed day). A fare more than
//! `FAIR` above the association's also costs reputation, a little every day (`reputation_cost`).

use super::economy;
use super::model::{Cents, Company, CompanyLine, Difficulty};
use serde::{Deserialize, Serialize};

/// The short-run and the long-run fare elasticity of demand.
pub const SHORT_RUN: f64 = -0.35;
pub const LONG_RUN: f64 = -0.7;
/// The lag's time constant: after this many days the passengers have come about two thirds
/// of the way from the short-run answer to the long-run one.
pub const LAG_DAYS: f64 = 21.0;
/// What a passenger pays on average, as a share of a single ticket.
pub const TICKET_MIX: f64 = 0.55;
/// The most a cheap fare multiplies the passengers by, and the least a dear one leaves.
pub const MOST: f64 = 1.6;
pub const LEAST: f64 = 0.15;
/// Above the association's fare by more than this share, a fare costs reputation.
pub const FAIR: f64 = 0.3;

/// How a line's passengers have taken its fare so far.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub struct Demand {
    /// The fare the passengers last knew, as its offset from the association's (p/p₀ − 1).
    #[serde(default)]
    pub x: f64,
    /// The log of the passengers' share now (0: as many as at the association's fare).
    #[serde(default)]
    pub log: f64,
}

impl Demand {
    /// The share of the passengers the line has now (1: as at the association's fare).
    pub fn share(&self) -> f64 {
        self.log.exp().clamp(LEAST, MOST)
    }
}

/// Ten cents round.
fn round10(c: f64) -> Cents {
    ((c / 10.0).round() as Cents) * 10
}

/// The association's single ticket (cents).
pub fn association_fare(c: &Company) -> Cents {
    round10(economy::rules(c.difficulty).fare as f64 / TICKET_MIX)
}

/// The band a line's fare may be set in (cents): an own line's is wide, a concession's the
/// association allows.
pub fn band(c: &Company, l: &CompanyLine) -> (Cents, Cents) {
    let p0 = association_fare(c) as f64;
    let (lo, hi) = if l.own {
        (0.5, 2.5)
    } else {
        match c.difficulty {
            Difficulty::Hard => (0.9, 1.1),
            _ => (0.8, 1.2),
        }
    };
    (round10(p0 * lo), round10(p0 * hi))
}

/// The single ticket a line asks now.
pub fn fare_of(c: &Company, l: &CompanyLine) -> Cents {
    let (lo, hi) = band(c, l);
    l.fare.unwrap_or_else(|| association_fare(c)).clamp(lo, hi)
}

/// A fare's offset from the association's (p/p₀ − 1).
pub fn offset(c: &Company, fare: Cents) -> f64 {
    fare as f64 / association_fare(c).max(1) as f64 - 1.0
}

/// What a passenger of the line pays on average (cents): the difficulty's average fare, moved
/// with the line's single ticket.
pub fn per_passenger(c: &Company, l: &CompanyLine) -> f64 {
    economy::rules(c.difficulty).fare as f64 * (1.0 + offset(c, fare_of(c, l)))
}

/// The passengers' share at a fare's offset once they have fully answered it.
pub fn long_run_share(x: f64) -> f64 {
    (LONG_RUN * x).exp().clamp(LEAST, MOST)
}

/// The passengers' share at a fare's offset the day it comes in.
pub fn short_run_share(x: f64) -> f64 {
    (SHORT_RUN * x).exp().clamp(LEAST, MOST)
}

/// The passengers answer the fare of one more day (`x` its offset today): a change at once
/// with the short-run elasticity, then a day's way towards the long run.
pub fn settle(d: &mut Demand, x: f64) {
    d.log += SHORT_RUN * (x - d.x);
    d.x = x;
    let k = 1.0 - (-1.0 / LAG_DAYS).exp();
    d.log += k * (LONG_RUN * x - d.log);
}

/// The fare (cents) that brings the most fares in the long run, within the line's band.
pub fn best_fare(c: &Company, l: &CompanyLine) -> Cents {
    let (lo, hi) = band(c, l);
    round10(association_fare(c) as f64 / -LONG_RUN).clamp(lo, hi)
}

/// The reputation a day of this fare costs (points).
pub fn reputation_cost(x: f64) -> f64 {
    (x - FAIR).max(0.0) * 0.3
}

/// Set a line's single ticket (clamped to its band). Returns the fare set.
pub fn set_fare(c: &mut Company, line: &str, fare: Cents) -> Result<Cents, &'static str> {
    let Some(k) = c.lines.iter().position(|l| l.name.eq_ignore_ascii_case(line)) else { return Err("The company does not run this line.") };
    let (lo, hi) = band(c, &c.lines[k]);
    let fare = round10(fare as f64).clamp(lo, hi);
    let p0 = association_fare(c);
    c.lines[k].fare = (fare != p0).then_some(fare);
    Ok(fare)
}

/// A day of fares for every line, as the day's close runs it: the passengers answer the
/// fares, and a fare far above the association's costs reputation.
pub fn day(c: &mut Company) {
    let xs: Vec<f64> = c.lines.iter().map(|l| offset(c, fare_of(c, l))).collect();
    let mut cost = 0.0;
    for (l, x) in c.lines.iter_mut().zip(xs) {
        settle(&mut l.demand, x);
        cost += reputation_cost(x);
    }
    c.reputation = (c.reputation - cost).clamp(0.0, 100.0);
}

/// What a line would bring at a fare in the long run, from its passengers a day at the
/// association's fare: (passengers a day, fares a day in cents).
pub fn estimate(c: &Company, base_passengers: f64, fare: Cents) -> (f64, f64) {
    let x = offset(c, fare);
    let pax = base_passengers * long_run_share(x);
    let per = economy::rules(c.difficulty).fare as f64 * (1.0 + x);
    (pax, pax * per)
}

#[cfg(test)]
mod tests {
    use super::super::{found, Founding};
    use super::*;

    fn company(d: Difficulty) -> Company {
        let mut c = found(&Founding { name: "Fares".into(), difficulty: d, date: "2024-03-04".into(), ..Default::default() }, "Luc");
        c.lines.push(CompanyLine { name: "Linie5".into(), number: "5".into(), ..Default::default() });
        c.lines.push(CompanyLine { name: "oo_1".into(), number: "1".into(), own: true, ..Default::default() });
        c
    }

    #[test]
    fn the_curve_has_the_studies_elasticities_at_the_association_fare() {
        // (the point elasticity: d ln Q / d ln p at p₀)
        let h = 1e-4;
        let point = |f: fn(f64) -> f64| ((f(h)).ln() - (f(-h)).ln()) / ((1.0f64 + h).ln() - (1.0f64 - h).ln());
        assert!((point(long_run_share) - LONG_RUN).abs() < 1e-3, "{}", point(long_run_share));
        assert!((point(short_run_share) - SHORT_RUN).abs() < 1e-3);
        // dearer loses riders, cheaper draws them, both within their bounds
        assert!(long_run_share(0.2) < 1.0 && long_run_share(-0.2) > 1.0);
        assert_eq!(long_run_share(-5.0), MOST);
        // the fares' revenue is highest at p₀/|ε|
        let c = company(Difficulty::Realistic);
        let own = &c.lines[1];
        let best = best_fare(&c, own);
        let rev = |f: Cents| estimate(&c, 1000.0, f).1;
        assert!(rev(best) >= rev(best - 30) && rev(best) >= rev(best + 30), "{best}");
        assert!((best as f64 / association_fare(&c) as f64 - 1.0 / 0.7).abs() < 0.05);
    }

    #[test]
    fn passengers_answer_at_once_in_part_and_the_rest_over_weeks() {
        let mut d = Demand::default();
        // twenty per cent dearer from today
        settle(&mut d, 0.2);
        let first = d.share();
        assert!((first.ln() - SHORT_RUN * 0.2).abs() < 0.01, "{first}");
        for _ in 1..LAG_DAYS as usize {
            settle(&mut d, 0.2);
        }
        // after the lag's time constant about two thirds of the way to the long run
        let way = (d.log - SHORT_RUN * 0.2) / ((LONG_RUN - SHORT_RUN) * 0.2);
        assert!((0.55..0.75).contains(&way), "{way}");
        for _ in 0..200 {
            settle(&mut d, 0.2);
        }
        assert!((d.share() - long_run_share(0.2)).abs() < 1e-3);
        // and back: the fare as before, the passengers come back the same way
        settle(&mut d, 0.0);
        assert!(d.share() > long_run_share(0.2) && d.share() < 1.0);
    }

    #[test]
    fn fares_are_set_within_the_band() {
        let mut c = company(Difficulty::Hard);
        let p0 = association_fare(&c);
        // a concession on Hard: ±10 %
        assert_eq!(set_fare(&mut c, "Linie5", p0 * 2), Ok((p0 as f64 * 1.1 / 10.0).round() as Cents * 10));
        assert_eq!(set_fare(&mut c, "Linie5", p0), Ok(p0));
        assert_eq!(c.lines[0].fare, None, "the association's fare is kept as none");
        // an own line: half to two and a half times
        let f = set_fare(&mut c, "oo_1", p0 * 2).unwrap();
        assert_eq!(f, p0 * 2);
        assert!(set_fare(&mut c, "nope", 100).is_err());
        // far above the association's: reputation, a little a day
        let before = c.reputation;
        day(&mut c);
        assert!(c.reputation < before);
        assert!(reputation_cost(0.2) == 0.0 && reputation_cost(1.0) > 0.0);
        assert!(c.lines[1].demand.share() < 1.0);
    }
}
