//! The company on the phone: what the companion's "Company" tab shows of it, and the few
//! things a paired phone may order (build the next level of a depot area, a workshop job for
//! a bus).
//!
//! The phone talks to the game (the companion's server runs in it), and the company belongs
//! to the launcher, which keeps it open and saves it with every change. So the phone never
//! writes the company: an order it sends is checked against the company as saved (the same
//! rules the launcher applies, on a copy) and put in a queue beside it
//! (`companies/<id>.orders.jsonl`); the launcher takes the queue in (`take`), applies each
//! order with the same rules again and saves. Orders are plain JSON of a fixed shape - no
//! names of files, no free text.

use super::day::{self, Plan};
use super::depot::{self, Area, JobKind};
use super::model::Company;
use super::{alerts, store, Alert};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// What a phone may order.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Order {
    /// Build the next level of an area of the depot.
    Build { area: Area },
    /// A workshop job for a bus of the fleet.
    Job { bus: u32, job: JobKind },
}

/// An order from the phone's JSON, strictly: an object of exactly the fields of one order.
pub fn parse(v: &Value) -> Option<Order> {
    let o = v.as_object()?;
    let has = |keys: &[&str]| o.len() == keys.len() && keys.iter().all(|k| o.contains_key(*k));
    match o.get("do")?.as_str()? {
        "build" if has(&["do", "area"]) => Some(Order::Build { area: Area::from_key(o.get("area")?.as_str()?)? }),
        "job" if has(&["do", "bus", "job"]) => {
            let bus = o.get("bus")?.as_u64().filter(|n| *n < 1_000_000)? as u32;
            Some(Order::Job { bus, job: JobKind::from_key(o.get("job")?.as_str()?)? })
        }
        _ => None,
    }
}

/// Carry out an order (the launcher's side).
pub fn apply(c: &mut Company, o: &Order) -> Result<(), &'static str> {
    match *o {
        Order::Build { area } => depot::build(c, area),
        Order::Job { bus, job } => depot::order(c, bus, job).map(|_| ()),
    }
}

/// Whether an order would be carried out, after the ones still waiting: on a copy.
pub fn check(c: &Company, waiting: &[Order], o: &Order) -> Result<(), &'static str> {
    let mut x = c.clone();
    for w in waiting {
        let _ = apply(&mut x, w);
    }
    apply(&mut x, o)
}

pub fn orders_file(data: &Path, id: &str) -> PathBuf {
    store::dir(data).join(format!("{id}.orders.jsonl"))
}

/// Put an order in the company's queue.
pub fn queue(data: &Path, id: &str, o: &Order) -> Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(store::dir(data))?;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(orders_file(data, id))?;
    writeln!(f, "{}", serde_json::to_string(o)?)?;
    Ok(())
}

/// The orders waiting for the launcher.
pub fn pending(data: &Path, id: &str) -> Vec<Order> {
    let Ok(text) = std::fs::read_to_string(orders_file(data, id)) else { return Vec::new() };
    text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter_map(|v| parse(&v)).collect()
}

/// Take the queue in: every order applied (or refused), the queue emptied.
pub fn take(data: &Path, c: &mut Company) -> Vec<(Order, Result<(), &'static str>)> {
    let path = orders_file(data, &c.id);
    if !path.is_file() {
        return Vec::new();
    }
    let orders = pending(data, &c.id);
    let _ = std::fs::remove_file(&path);
    orders.into_iter().map(|o| (o, apply(c, &o))).collect()
}

/// The company saved last (the one the launcher has open), if any.
pub fn latest(data: &Path) -> Option<Company> {
    let rd = std::fs::read_dir(store::dir(data)).ok()?;
    let newest = rd
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.file_name().to_string_lossy().to_string())))
        .max_by_key(|x| x.0)?;
    store::load(data, newest.1.strip_suffix(".json")?).ok()
}

/// The company's plan of today, from the map's timetable of its date.
pub fn plan_of(c: &Company) -> Option<Plan> {
    let (_, tours) = store::tours_today(c).ok()?;
    Some(day::assign(c, tours, &[], &[]))
}

fn hhmm(minutes: i32) -> String {
    format!("{:02}:{:02}", (minutes / 60).rem_euclid(24), minutes.rem_euclid(60))
}

/// What the phone's tab shows: money, today's dispositions, the depot and its workshop, the
/// buses that want work, the orders waiting. Amounts in cents.
pub fn summary(c: &Company, plan: Option<&Plan>, waiting: &[Order]) -> Value {
    let today = &c.date;
    let month = super::dates::month_of(today);
    let m = c.month(&month);
    let months: Vec<Value> = c.months.iter().rev().take(6).rev().map(|m| json!({ "month": m.month, "income": m.income(), "expenses": m.expenses(), "result": m.result() })).collect();
    let days: Vec<Value> = c.history.iter().rev().take(14).rev().map(|d| json!({ "date": d.date, "result": d.result })).collect();
    let held: Vec<&super::Vehicle> = c.fleet.iter().filter(|v| v.held_on(today)).collect();
    let in_workshop = held.iter().filter(|v| v.in_workshop(today)).count();
    let ours: Vec<u32> = c.site.jobs.iter().filter(|j| j.started.is_some()).map(|j| j.vehicle).collect();
    let breakdowns: Vec<Value> = held.iter().filter(|v| v.in_workshop(today) && !ours.contains(&v.id)).map(|v| json!({ "number": v.number, "until": v.workshop_until })).collect();
    let today_v = plan.map(|p| {
        let open: Vec<Value> = p
            .tours
            .iter()
            .filter(|t| !t.covered())
            .take(30)
            .map(|t| json!({ "line": t.tour.number, "tour": t.tour.tour, "from": hhmm(t.tour.from()), "to": hhmm(t.tour.to()), "why": if t.bus.is_none() { "bus" } else { "driver" } }))
            .collect();
        let (buses, duties) = p.short_of();
        json!({ "tours": p.tours.len(), "covered": p.tours.len() - p.uncovered(), "open": open, "no_bus": buses, "no_driver": duties })
    });
    let al: Vec<Value> = alerts(c, plan)
        .into_iter()
        .map(|a| match a {
            Alert::NoLines => json!({ "kind": "no_lines" }),
            Alert::NoBuses => json!({ "kind": "no_buses" }),
            Alert::NoDrivers => json!({ "kind": "no_drivers" }),
            Alert::Uncovered { tours, .. } => json!({ "kind": "uncovered", "n": tours }),
            Alert::LowCash => json!({ "kind": "low_cash" }),
            Alert::ServiceDue(n) => json!({ "kind": "service_due", "n": n }),
            Alert::Unhappy(n) => json!({ "kind": "unhappy", "n": n }),
            Alert::GoingBack { number, until } => json!({ "kind": "going_back", "number": number, "until": until }),
            Alert::NotPlanned(lines) => json!({ "kind": "not_planned", "lines": lines }),
            Alert::Tomorrow { tours, buses, duties } => json!({ "kind": "tomorrow", "tours": tours, "buses": buses, "duties": duties }),
        })
        .collect();
    let areas: Vec<Value> = Area::ALL
        .iter()
        .map(|a| {
            let next = depot::next_cost(c, *a);
            let works = c.site.works_on(*a).map(|w| w.until.clone());
            let can = check(c, waiting, &Order::Build { area: *a }).is_ok();
            json!({ "key": a.key(), "label": a.label(), "level": c.site.level(*a), "max": a.max(), "cost": next.map(|x| x.0), "days": next.map(|x| x.1), "building_until": works, "allowed": depot::area_allowed(c, *a), "can": can })
        })
        .collect();
    let jobs: Vec<Value> = c
        .site
        .jobs
        .iter()
        .map(|j| json!({ "id": j.id, "number": c.vehicle(j.vehicle).map(|v| v.number.clone()).unwrap_or_default(), "job": j.kind.label(), "started": j.started, "until": j.until, "cost": j.cost }))
        .collect();
    let buses: Vec<Value> = held
        .iter()
        .filter(|v| c.site.job_of(v.id).is_none() && !v.in_workshop(today))
        .filter(|v| v.condition < 75.0 || v.km >= v.next_service_km - 1_000.0)
        .take(20)
        .map(|v| {
            let due = v.km >= v.next_service_km - 1_000.0;
            let job = if due { JobKind::Service } else { JobKind::Repair };
            json!({ "id": v.id, "number": v.number, "name": v.name, "condition": v.condition.round(), "due": due, "job": job.key(), "job_label": job.label(), "cost": depot::job_cost(c, v.id, job) })
        })
        .collect();
    let pend: Vec<Value> = waiting.iter().map(|o| serde_json::to_value(o).unwrap_or(Value::Null)).collect();
    json!({
        "id": c.id,
        "name": c.name,
        "short": c.short,
        "colour": c.colours[0],
        "date": c.date,
        "difficulty": c.difficulty.label(),
        "cash": c.cash,
        "debt": c.debt(),
        "reputation": c.reputation.round(),
        "punctuality": c.punctuality.round(),
        "clean": c.site.clean.round(),
        "month": { "month": month, "income": m.income(), "expenses": m.expenses(), "result": m.result() },
        "months": months,
        "days": days,
        "fleet": { "buses": held.len(), "workshop": in_workshop, "spaces": c.site.spaces(), "outside": depot::outside(c), "bays": c.site.bays() },
        "staff": { "people": c.staff.iter().filter(|e| e.employed_on(today)).count(), "absent": c.staff.iter().filter(|e| e.employed_on(today) && e.absent(today)).count() },
        "today": today_v,
        "breakdowns": breakdowns,
        "alerts": al,
        "areas": areas,
        "upkeep": depot::upkeep_month(c),
        "jobs": jobs,
        "buses": buses,
        "pending": pend,
        "concessions": c.concessions.held.iter().map(|h| json!({ "number": h.number, "until": h.until, "price": h.price })).collect::<Vec<_>>(),
        "tenders": c.concessions.tenders.iter().filter(|t| t.open()).map(|t| json!({ "number": t.number, "closes": t.closes, "closes_at": super::clock::hhmm(t.closes_at), "bid": serde_json::Value::Null, "offer": t.offers.last().map(|o| o.1) })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::market::{self, MarketBus, Payment};
    use super::super::{found, Founding};
    use super::*;

    #[test]
    fn only_orders_of_the_exact_shape_are_taken() {
        assert_eq!(parse(&json!({ "do": "build", "area": "wash" })), Some(Order::Build { area: Area::Wash }));
        assert_eq!(parse(&json!({ "do": "job", "bus": 3, "job": "repair" })), Some(Order::Job { bus: 3, job: JobKind::Repair }));
        for bad in [
            json!({ "do": "build", "area": "wash", "file": "x" }),
            json!({ "do": "build", "area": "../x" }),
            json!({ "do": "job", "bus": -1, "job": "repair" }),
            json!({ "do": "job", "bus": "3", "job": "repair" }),
            json!({ "do": "sell", "bus": 3 }),
            json!(["build"]),
            json!("build"),
        ] {
            assert_eq!(parse(&bad), None, "{bad}");
        }
        // what is queued reads back the same
        let o = Order::Job { bus: 7, job: JobKind::Overhaul };
        assert_eq!(parse(&serde_json::to_value(o).unwrap()), Some(o));
    }

    #[test]
    fn an_order_is_checked_queued_and_carried_out_by_the_launcher() {
        let data = std::env::temp_dir().join(format!("openomsi-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let mut c = found(&Founding { name: "Fern".into(), date: "2024-03-04".into(), ..Default::default() }, "Luc");
        let bus = MarketBus { file: "Vehicles/Citaro/Citaro.bus".into(), name: "Citaro".into(), ..Default::default() };
        let id = market::buy_new(&mut c, &bus, Payment::Cash, "").unwrap();
        store::save(&data, &c).unwrap();
        assert_eq!(latest(&data).map(|x| x.id), Some(c.id.clone()));
        let wash = Order::Build { area: Area::Wash };
        assert!(check(&c, &[], &wash).is_ok());
        // (a second order of the same build waits behind the first and would be refused)
        assert_eq!(check(&c, &[wash], &wash), Err("This is being built already."));
        queue(&data, &c.id, &wash).unwrap();
        queue(&data, &c.id, &Order::Job { bus: id, job: JobKind::Service }).unwrap();
        assert_eq!(pending(&data, &c.id).len(), 2);
        let s = summary(&c, None, &pending(&data, &c.id));
        assert_eq!(s["pending"].as_array().map(|a| a.len()), Some(2));
        assert_eq!(s["areas"][2]["key"], "wash");
        assert_eq!(s["areas"][2]["can"], false);
        let done = take(&data, &mut c);
        assert!(done.iter().all(|x| x.1.is_ok()));
        assert!(c.site.works_on(Area::Wash).is_some() && c.site.job_of(id).is_some());
        assert!(pending(&data, &c.id).is_empty() && take(&data, &mut c).is_empty());
        let _ = std::fs::remove_dir_all(&data);
    }
}
