//! The player's bus, the AI traffic and the people for the plugin API (`bus.*`,
//! `traffic.*`, `people.*`).

use crate::App;
use omsi_plugin as op;
use omsi_sim::people::{pax::Task, BusId, Place, State};
use omsi_sim::VehicleInstance;

pub(crate) fn veh(app: &App) -> Option<&VehicleInstance> {
    app.player.as_ref().map(|p| &p.vehicle)
}

/// The bus's velocity in the world (m/s): the rigid body's, else along its heading.
pub(crate) fn velocity(app: &App) -> Option<[f64; 3]> {
    let v = veh(app)?;
    Some(match v.rigid.as_ref() {
        Some(rb) => [rb.velocity.x as f64, rb.velocity.y as f64, rb.velocity.z as f64],
        None => {
            let (s, h) = (v.physics.speed as f64, v.heading.to_radians());
            [s * h.sin(), s * h.cos(), 0.0]
        }
    })
}

/// Each door leaf, `door_0`, `door_1`, ... until the first the bus does not have.
pub(crate) fn doors(app: &App) -> Vec<f32> {
    let Some(v) = veh(app) else { return Vec::new() };
    (0..32).map_while(|n| v.var(&format!("door_{n}"))).collect()
}

/// Any door open: what the passengers are told where the bus has that (`PAX_*_Open`), else
/// the leaves - the rule of the game's own hints (`omsi_sim::vehicle_api`).
pub(crate) fn doors_open(app: &App) -> Option<bool> {
    let v = veh(app)?;
    let mut flags = false;
    for kind in ["Entry", "Exit"] {
        for n in 0..8 {
            if let Some(x) = v.var(&format!("PAX_{kind}{n}_Open")) {
                flags = true;
                if x > 0.5 {
                    return Some(true);
                }
            }
        }
    }
    Some(!flags && doors(app).iter().any(|d| *d > 0.05))
}

/// Press and let go a door key (0: all doors).
pub(crate) fn door_key(app: &mut App, n: usize) -> bool {
    let Some(p) = app.player.as_mut() else { return false };
    if n > crate::player::door_keys(&p.vehicle.ty).len() {
        return false;
    }
    let fired = p.door_key(n);
    p.door_key_off(&fired);
    !fired.is_empty()
}

/// The headlights: 0 off, 1 side lights, 2 dipped, 3 high beam (`vehicle_api`'s rule).
pub(crate) fn headlights(app: &App) -> Option<u8> {
    let v = veh(app)?;
    let on = |n: &str| v.var(n).unwrap_or(0.0) > 0.5;
    Some(if on("lights_fern") {
        3
    } else if on("lights_abbl") || on("lights_main") || v.var("Spot_Select").is_some_and(|s| s >= 0.0) {
        2
    } else if on("lights_stand") {
        1
    } else {
        0
    })
}

pub(crate) fn engine(app: &App) -> Option<(bool, Option<f32>, bool)> {
    let v = veh(app)?;
    Some((omsi_sim::startup::engine_running(v), omsi_sim::startup::engine_rpm(v), omsi_sim::startup::power_on(v)))
}

pub(crate) fn gear(app: &App) -> Option<f32> {
    let v = veh(app)?;
    v.var("antrieb_getr_aktugang").or_else(|| v.var("gear"))
}

pub(crate) fn destinations(app: &App) -> (Vec<op::Terminus>, Option<i64>) {
    let Some(v) = veh(app) else { return (Vec::new(), None) };
    let Some(hof) = v.host.hof.as_ref() else { return (Vec::new(), None) };
    let list = hof.termini.iter().map(|t| op::Terminus { code: t.code as i64, name: t.display_name(), all_exit: t.all_exit }).collect();
    let shown = v.var("target_index_int").filter(|i| i.is_finite() && *i >= 0.0).map(|i| i.round() as i64);
    (list, shown)
}

/// Destination `ti` of the depot file, as the destination list sets it (the IBIS line stays).
pub(crate) fn set_destination(app: &mut App, ti: usize) -> bool {
    let Some(p) = app.player.as_mut() else { return false };
    let Some(hof) = p.vehicle.host.hof.clone() else { return false };
    let Some(t) = hof.termini.get(ti) else { return false };
    let line = crate::game_lists::destination_line(&p.vehicle);
    let name = t.menu_name();
    p.set_destination_by_hand(&hof, &line, ti);
    log::info!("destination display set by a plugin: {} {}", t.code, name.trim());
    true
}

/// People aboard the player's bus: all, seated, standing.
pub(crate) fn passengers(app: &App) -> Option<(usize, usize, usize)> {
    veh(app)?;
    let Some(h) = app.session.humans.as_ref() else { return Some((0, 0, 0)) };
    let (mut seated, mut standing) = (0, 0);
    for p in h.people.iter().filter(|p| p.inside(BusId::Player)) {
        match &p.state {
            State::Pax(x) if x.task == Task::SittingInBus => seated += 1,
            _ => standing += 1,
        }
    }
    Some((seated + standing, seated, standing))
}

pub(crate) fn tickets(app: &App) -> (Vec<op::Ticket>, Option<(String, f32)>) {
    let list = veh(app)
        .and_then(|v| v.host.tickets.as_ref())
        .map(|t| t.tickets.iter().map(|t| op::Ticket { name: t.name.trim().to_string(), price: t.value, day_ticket: t.day_ticket }).collect())
        .unwrap_or_default();
    let request = app.session.humans.as_ref().and_then(|h| h.request.clone()).map(|(n, p)| (n.trim().to_string(), p));
    (list, request)
}

pub(crate) fn wheels(app: &App) -> Vec<op::Wheel> {
    let Some(v) = veh(app) else { return Vec::new() };
    v.physics
        .wheels
        .iter()
        .enumerate()
        .flat_map(|(axle, pair)| pair.iter().enumerate().map(move |(side, w)| op::Wheel { axle, side, rpm: w.rpm, radius: w.radius, suspension: w.suspension, driven: w.driven }))
        .collect()
}

/// Play the bus's own sound of an event: handed to its sound set with the next tick, as a
/// script's trigger is.
pub(crate) fn sound(app: &mut App, event: &str) -> bool {
    let Some(p) = app.player.as_mut() else { return false };
    if event.trim().is_empty() {
        return false;
    }
    p.vehicle.host.fired_triggers.push(event.to_string());
    true
}

// --- the AI traffic ----------------------------------------------------------------------

fn kind(c: &omsi_sim::ai_traffic::model::AiCar) -> &'static str {
    if c.is_rail() {
        return "tram";
    }
    if c.vehicle.ty.def.mass > 0.0 && c.vehicle.ty.def.mass <= 0.3 {
        return "bicycle";
    }
    match c.state.veh_type {
        -1 => "timetable_bus",
        1 => "taxi",
        2 => "bus",
        3 => "truck",
        _ if c.is_bus() => "timetable_bus",
        _ => "car",
    }
}

/// A vehicle's manufacturer and type, else its file's name (many AI cars name neither).
fn vehicle_name(v: &VehicleInstance) -> String {
    let n = format!("{} {}", v.ty.def.manufacturer, v.ty.def.type_name).trim().to_string();
    if !n.is_empty() {
        return n;
    }
    v.ty.def.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

pub(crate) fn traffic_list(app: &App) -> Vec<op::AiCar> {
    let Some(t) = app.session.traffic.as_ref() else { return Vec::new() };
    t.cars
        .iter()
        .filter(|c| !c.gone)
        .map(|c| {
            let v = &c.vehicle;
            op::AiCar {
                id: c.id,
                kind: kind(c),
                name: vehicle_name(v),
                pos: [v.position.x, v.position.y, v.position.z, v.heading.rem_euclid(360.0)],
                speed_kmh: (c.state.speed * 3.6) as f64,
                max_speed_kmh: c.state.max_speed_kmh as f64,
                why: if c.state.speed > 0.5 && c.why.0 == "lead" { "" } else { c.why.0 },
                standing: c.stopped,
                braking: c.state.braking,
                blinker: c.state.blinker.clamp(0, 2) as u8,
                line: c.is_bus().then(|| v.host.tt_line.trim().to_string()).filter(|l| !l.is_empty()),
            }
        })
        .collect()
}

pub(crate) fn traffic_remove(app: &mut App, id: u64) -> bool {
    let (Some(t), Some(w), Some(r), Some(sc)) = (app.session.traffic.as_mut(), app.world.as_ref(), app.renderer.as_ref(), app.scene.as_mut()) else { return false };
    if !t.remove_car(&mut app.gfx.sim_view.traffic, w, r, sc, id) {
        return false;
    }
    if let Some(s) = app.session.schedule.as_mut() {
        s.sim.forget_car(id);
    }
    if let Some(h) = app.session.humans.as_mut() {
        h.evict(BusId::Ai(id), w);
    }
    log::info!("traffic: car {id} taken off the road by a plugin");
    true
}

pub(crate) fn traffic_clear(app: &mut App) -> Option<usize> {
    let (Some(t), Some(w), Some(r), Some(sc)) = (app.session.traffic.as_mut(), app.world.as_ref(), app.renderer.as_ref(), app.scene.as_mut()) else { return None };
    Some(t.clear_random(&mut app.gfx.sim_view.traffic, w, r, sc))
}

/// The light on the street ahead of the player's bus: the first lane within `reach` that
/// has one (the lane the bus is on is past its stop line).
pub(crate) fn light_ahead(app: &App, reach: f64) -> Option<op::Light> {
    let t = app.session.traffic.as_ref()?;
    let v = veh(app)?;
    let lanes = t.lanes_ahead_of(v.position, v.heading, reach as f32);
    let (lane, dist) = lanes.into_iter().skip(1).filter(|(l, _)| t.net.lanes[*l].traffic_light.is_some()).min_by(|a, b| a.1.total_cmp(&b.1))?;
    let (c, li) = t.net.lanes[lane].traffic_light?;
    let (state, _) = t.light_state(c, li)?;
    use omsi_sim::traffic::{Aspect, TrafficLightController};
    let aspect = match TrafficLightController::aspect(state) {
        Aspect::Red => "red",
        Aspect::RedYellow => "red_yellow",
        Aspect::Green => "green",
        Aspect::GreenYellow => "green_yellow",
        Aspect::Yellow => "yellow",
        Aspect::Dark => "dark",
    };
    let change_in = t.lights.get(c).and_then(|ctl| ctl.time_to_change(li)).unwrap_or(f32::INFINITY);
    Some(op::Light { aspect, change_in: if change_in.is_finite() { change_in } else { -1.0 }, distance: dist as f64 })
}

// --- the people --------------------------------------------------------------------------

fn person_state(p: &omsi_sim::people::Person) -> &'static str {
    match &p.state {
        State::Strolling(_) => "strolling",
        State::Idle => "idle",
        State::Standing => "standing",
        State::Pax(x) => match x.task {
            Task::WaitingForBus => "waiting",
            Task::ToBus => "to_bus",
            Task::WalkingToBus => "boarding",
            Task::InBusToPlace => "riding",
            Task::InBusToExit => "leaving",
            Task::WalkingToBusstop => "to_stop",
            Task::SittingInBus => "seated",
            Task::Nothing if x.inside.is_some() => "riding",
            Task::Nothing => "waiting",
        },
    }
}

pub(crate) fn people(app: &App) -> Vec<op::Person> {
    let Some(h) = app.session.humans.as_ref() else { return Vec::new() };
    h.people
        .iter()
        .map(|p| {
            let pax = match &p.state {
                State::Pax(x) => Some(x),
                _ => None,
            };
            op::Person {
                id: p.id,
                pos: [p.position.x, p.position.y, p.position.z],
                state: person_state(p),
                aboard: p.inside(BusId::Player),
                in_ai_bus: match p.place {
                    Place::Bus(BusId::Ai(id), _) => Some(id),
                    _ => None,
                },
                stop: pax.and_then(|x| x.stop),
                destination: pax.and_then(|x| x.dest.clone()).map(|d| d.trim().to_string()),
                ticket: None,
                complaint: pax.map_or(0, |x| x.complaint),
            }
        })
        .collect()
}

pub(crate) fn pax_stops(app: &App) -> Vec<op::PaxStop> {
    let Some(h) = app.session.humans.as_ref() else { return Vec::new() };
    let mut out: Vec<op::PaxStop> = h
        .stops
        .iter()
        .map(|(id, s)| {
            let waiting = h.people.iter().filter(|p| matches!(&p.state, State::Pax(x) if x.inside.is_none() && x.stop == Some(*id) && matches!(x.task, Task::WaitingForBus | Task::ToBus | Task::WalkingToBus))).count();
            op::PaxStop { id: *id, name: s.name.trim().to_string(), pos: [s.pos.x, s.pos.y, s.pos.z], waiting }
        })
        .collect();
    out.sort_by_key(|s| s.id);
    out
}
