//! The company's own liveries (Luc: "vanuit het busbedrijf moet je ook een lakontwerp voor je
//! bus kunnen maken, wat natuurlijk geld kost"): a livery painted in the livery studio for a
//! bus model of the fleet becomes the company's design, and a bus of that model is painted in
//! it in the workshop.
//!
//! What it costs (Realistic, net, founding day's prices; Easy 0.6 times, Hard 1.3 times):
//! - *The design* (`design_fee`): a designer's corporate livery for a bus - the drawings, the
//!   colours, the proofs - €2,500. Saved again under its name (a change) a fifth of that.
//! - *The painting* (`paint_cost`): a foil wrap or a repaint of a 12 m bus in the company's
//!   livery €4,000 - a midibus three quarters of that, an articulated bus half again, a
//!   double-decker 1.4 times. The bus is in the workshop for a day (`depot::JobKind::Paint`),
//!   and wears the livery from then on, in the game too (`Vehicle::house_livery`).

use super::depot;
use super::model::{BookingKind, BusSize, Cents, Company, Difficulty, Vehicle};
use serde::{Deserialize, Serialize};

/// A livery of the company's own: its name in the game (the livery studio's `.cti`), the bus
/// file it was painted for, when it was made, and what its design cost so far.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Design {
    pub name: String,
    pub bus: String,
    pub made: String,
    pub paid: Cents,
}

/// The design of a livery, and the painting of a 12 m bus (Realistic, founding day's prices).
pub const DESIGN_FEE: Cents = 2_500_00;
pub const PAINT_BASE: Cents = 4_000_00;

fn factor(d: Difficulty) -> f64 {
    match d {
        Difficulty::Easy => 0.6,
        Difficulty::Realistic => 1.0,
        Difficulty::Hard => 1.3,
    }
}

/// What painting a bus of a size costs against a solo bus.
pub fn size_factor(size: BusSize) -> f64 {
    match size {
        BusSize::Midi => 0.75,
        BusSize::Solo => 1.0,
        BusSize::Articulated => 1.5,
        BusSize::Double => 1.4,
    }
}

/// In whole euros.
fn euros(x: f64) -> Cents {
    ((x / 100.0).round() as Cents) * 100
}

/// What the design of a new livery costs the company now.
pub fn design_fee(c: &Company) -> Cents {
    euros(DESIGN_FEE as f64 * factor(c.difficulty) * c.price_index)
}

/// What painting a bus of the fleet in a livery of the company's costs.
pub fn paint_cost(c: &Company, v: &Vehicle) -> Cents {
    euros(PAINT_BASE as f64 * size_factor(v.kind.size) * factor(c.difficulty) * c.price_index)
}

fn same_file(a: &str, b: &str) -> bool {
    a.trim().replace('\\', "/").eq_ignore_ascii_case(&b.trim().replace('\\', "/"))
}

/// The company's design of that name (in any case).
pub fn design<'a>(c: &'a Company, name: &str) -> Option<&'a Design> {
    c.designs.iter().find(|d| d.name.trim().eq_ignore_ascii_case(name.trim()))
}

/// The company's designs for a bus file.
pub fn designs_for<'a>(c: &'a Company, bus: &str) -> Vec<&'a Design> {
    c.designs.iter().filter(|d| same_file(&d.bus, bus)).collect()
}

/// What saving a design under `name` costs: the design of a new livery, a fifth of it for one
/// the company has already (a change).
pub fn save_cost(c: &Company, name: &str) -> Cents {
    if design(c, name).is_some() {
        euros(design_fee(c) as f64 / 5.0)
    } else {
        design_fee(c)
    }
}

/// A livery painted in the studio for `bus` kept as the company's design: its cost booked.
/// Returns what was booked.
pub fn save_design(c: &mut Company, name: &str, bus: &str) -> Result<Cents, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("The livery needs a name.");
    }
    let cost = save_cost(c, name);
    if c.cash < cost {
        return Err("Not enough cash.");
    }
    c.book(BookingKind::Livery, -cost, format!("Livery design \"{name}\""), false);
    let date = c.date.clone();
    match c.designs.iter_mut().find(|d| d.name.trim().eq_ignore_ascii_case(name)) {
        Some(d) => {
            d.paid += cost;
            d.bus = bus.to_string();
        }
        None => c.designs.push(Design { name: name.to_string(), bus: bus.to_string(), made: date, paid: cost }),
    }
    Ok(cost)
}

/// Paint a bus of the fleet in one of the company's designs: a day in the workshop (from
/// tomorrow, when a bay is free), paid when the work starts. Returns the workshop's job.
pub fn paint(c: &mut Company, vehicle: u32, name: &str) -> Result<u32, &'static str> {
    let Some(v) = c.vehicle(vehicle).cloned() else { return Err("This bus is not in the fleet.") };
    let Some(d) = design(c, name).cloned() else { return Err("The company has no such livery.") };
    if !same_file(&d.bus, &v.bus) {
        return Err("This livery was made for another bus.");
    }
    if c.site.job_of(vehicle).is_some() {
        return Err("The workshop has a job for this bus already.");
    }
    if v.house_livery.as_deref().is_some_and(|h| h.trim().eq_ignore_ascii_case(name.trim())) && v.livery.trim().eq_ignore_ascii_case(name.trim()) {
        return Err("The bus wears this livery already.");
    }
    let cost = paint_cost(c, &v);
    depot::order_paint(c, vehicle, &d.name, cost)
}

#[cfg(test)]
mod tests {
    use super::super::market::{self, MarketBus, Payment};
    use super::super::{found, Founding};
    use super::*;
    use crate::company::model::{BusKind, Drive};

    fn company(d: Difficulty) -> Company {
        let mut c = found(&Founding { name: "Lack".into(), difficulty: d, date: "2024-03-04".into(), ..Default::default() }, "Luc");
        c.cash += 1_000_000_00;
        let bus = MarketBus { file: "Vehicles/Citaro/Citaro.bus".into(), name: "Citaro".into(), kind: BusKind { size: BusSize::Solo, drive: Drive::Diesel }, ..Default::default() };
        market::buy_new(&mut c, &bus, Payment::Cash, "").unwrap();
        c
    }

    #[test]
    fn a_design_costs_its_fee_and_a_bus_its_painting() {
        let mut c = company(Difficulty::Realistic);
        assert_eq!(design_fee(&c), 2_500_00);
        assert_eq!(paint_cost(&c, &c.fleet[0]), 4_000_00);
        assert!(design_fee(&company(Difficulty::Easy)) < design_fee(&c) && design_fee(&company(Difficulty::Hard)) > design_fee(&c));
        // saved: the fee booked, the design the company's
        let cash = c.cash;
        assert_eq!(save_design(&mut c, " Stadtbus Orange ", "Vehicles/Citaro/Citaro.bus"), Ok(2_500_00));
        assert_eq!(c.cash, cash - 2_500_00);
        assert!(c.ledger.last().is_some_and(|b| b.kind == BookingKind::Livery && !b.kind.is_capital()));
        assert_eq!(designs_for(&c, "vehicles\\citaro\\citaro.bus").len(), 1);
        // saved again under its name: a change, a fifth
        assert_eq!(save_cost(&c, "stadtbus orange"), 500_00);
        assert_eq!(save_design(&mut c, "Stadtbus Orange", "Vehicles/Citaro/Citaro.bus"), Ok(500_00));
        assert_eq!((c.designs.len(), c.designs[0].paid), (1, 3_000_00));
        assert_eq!(save_design(&mut c, "  ", "x"), Err("The livery needs a name."));
        // not without the money
        let mut poor = c.clone();
        poor.cash = 100_00;
        assert_eq!(save_design(&mut poor, "Neu", "x"), Err("Not enough cash."));
    }

    #[test]
    fn a_bus_is_painted_in_the_workshop() {
        let mut c = company(Difficulty::Realistic);
        let id = c.fleet[0].id;
        save_design(&mut c, "Stadtbus Orange", "Vehicles/Citaro/Citaro.bus").unwrap();
        save_design(&mut c, "Other", "Vehicles/MAN/Lion.bus").unwrap();
        assert_eq!(paint(&mut c, id, "Other"), Err("This livery was made for another bus."));
        assert_eq!(paint(&mut c, id, "Nothing"), Err("The company has no such livery."));
        let today = c.date.clone();
        let job = paint(&mut c, id, "Stadtbus Orange").unwrap();
        let j = c.site.jobs.iter().find(|j| j.id == job).cloned().unwrap();
        assert_eq!((j.kind, j.livery.as_deref(), j.cost), (depot::JobKind::Paint, Some("Stadtbus Orange"), 4_000_00));
        assert_eq!(paint(&mut c, id, "Stadtbus Orange"), Err("The workshop has a job for this bus already."));
        // (the work starts tomorrow, when a bay is free: then it is paid, and the bus wears the
        // livery)
        if j.started.is_none() {
            c.date = super::super::dates::add(&today, 1);
            depot::after_day(&mut c, &today);
        }
        let v = c.vehicle(id).unwrap();
        assert_eq!((v.livery.as_str(), v.house_livery.as_deref()), ("Stadtbus Orange", Some("Stadtbus Orange")));
        assert!(c.ledger.iter().any(|b| b.kind == BookingKind::Livery && b.amount == -4_000_00 && b.text.contains("Painting")));
        assert!(v.in_workshop(&super::super::dates::add(&today, 1)));
        // painted: not again
        let mut later = c.clone();
        later.site.jobs.clear();
        assert_eq!(paint(&mut later, id, "Stadtbus Orange"), Err("The bus wears this livery already."));
    }
}
