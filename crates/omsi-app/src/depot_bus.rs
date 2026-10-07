//! The game's side of the player's depot: a depot bus starts and ends with its own wear.

use omsi_launcher_lib::depot;
use omsi_sim::VehicleInstance;

/// Puts back the wear bus `id` was left with (a first outing keeps the bus as it comes).
pub(crate) fn restore(v: &mut VehicleInstance, id: &str) {
    let d = match depot::load() {
        Ok(d) => d,
        Err(e) => return log::warn!("depot: {e:#}"),
    };
    let Some(b) = d.get(id) else {
        return log::warn!("depot bus {id} not found in {}", depot::path().display());
    };
    if !is_this_bus(v, &b.bus) {
        return log::warn!("depot bus {} is a {}, not this vehicle: its wear is left alone", b.name, b.bus);
    }
    if b.wear.is_empty() {
        return log::info!("depot bus {} ({id}): first outing", b.name);
    }
    if let Some(km) = b.odometer() {
        v.host.km_base = km;
    }
    let (n, _) = v.restore_script_state(&b.wear, &[]);
    log::info!("depot bus {} ({id}): {n} of {} wear variables restored", b.name, b.wear.len());
}

/// The vehicle's wear variables as they are now.
pub(crate) fn wear(v: &VehicleInstance) -> Vec<(String, f32)> {
    let mut out: Vec<(String, f32)> = v
        .ty
        .program
        .var_names
        .iter()
        .enumerate()
        .filter(|(_, n)| depot::is_wear_var(n) && !n.eq_ignore_ascii_case(depot::DIRT))
        .map(|(i, n)| (n.clone(), v.state.vars.get(i).copied().unwrap_or(0.0)))
        .collect();
    for k in ["kmcounter_km", "kmcounter_m"] {
        if !out.iter().any(|(n, _)| n.eq_ignore_ascii_case(k)) {
            out.extend(v.var(k).map(|x| (k.to_string(), x)));
        }
    }
    out.push((depot::DIRT.into(), v.dirt));
    out
}

/// Adds the session to bus `id`, if the player still drives it.
pub(crate) fn record(v: &VehicleInstance, id: &str, outing: &depot::Outing) {
    let bus = depot::load().ok().and_then(|d| d.get(id).map(|b| b.bus.clone()));
    match bus {
        Some(b) if !is_this_bus(v, &b) => log::info!("depot bus {id}: the session ended in another vehicle, not recorded"),
        _ => match depot::record(id, outing, wear(v)) {
            Ok(true) => log::info!("depot bus {id}: {:.1} km and its wear recorded", outing.metres / 1000.0),
            Ok(false) => log::warn!("depot bus {id} is no longer in the depot"),
            Err(e) => log::warn!("depot: {e:#}"),
        },
    }
}

fn is_this_bus(v: &VehicleInstance, bus: &str) -> bool {
    let norm = |s: &str| s.replace('\\', "/").to_ascii_lowercase();
    norm(&v.ty.def.path.to_string_lossy()).ends_with(&norm(bus.trim_start_matches(['/', '\\'])))
}
