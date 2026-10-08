//! Live telemetry for outside tools (bus information systems such as OBIS): the player's
//! bus position and its timetable progress, written about twice a second as JSON to
//! `<home>/.openomsi/telemetry.json` (written atomically on a thread of its own).
//!
//! Set `OPENOMSI_TELEMETRY=0` to switch it off.

use crate::schedule::PlayerDuty;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const EVERY: Duration = Duration::from_millis(500);

fn path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".openomsi").join("telemetry.json"))
}

fn writer() -> &'static Option<Sender<(PathBuf, Vec<u8>)>> {
    static W: OnceLock<Option<Sender<(PathBuf, Vec<u8>)>>> = OnceLock::new();
    W.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<(PathBuf, Vec<u8>)>();
        std::thread::Builder::new()
            .name("telemetry".into())
            .spawn(move || {
                while let Ok(mut job) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() {
                        job = newer;
                    }
                    let (p, bytes) = job;
                    if let Some(dir) = p.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let tmp = p.with_extension("json.tmp");
                    if std::fs::write(&tmp, bytes).is_ok() {
                        let _ = std::fs::rename(&tmp, &p);
                    }
                }
            })
            .ok()
            .map(|_| tx)
    })
}

/// Called every frame; writes at most twice a second.
pub fn publish(player: Option<&crate::player::Player>, duty: Option<&PlayerDuty>, passengers: Option<usize>, paused: bool, schedule: Option<&crate::schedule::Schedule>, traffic: Option<&crate::traffic::Traffic>) {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| t.elapsed() < EVERY) {
            return;
        }
        *last = Some(Instant::now());
    }
    if omsi_cfg::env::var("OPENOMSI_TELEMETRY").map(|v| v == "0").unwrap_or(false) {
        return;
    }
    let (Some(p), Some(tx)) = (path(), writer().as_ref()) else { return };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let mut v = serde_json::json!({
        "version": 2,
        "pid": std::process::id(),
        "updated": now,
        "paused": paused,
        "player": serde_json::Value::Null,
        "duty": serde_json::Value::Null,
        // the timetable (AI) buses on the road: for the gaps to the buses ahead and behind
        "ai_buses": schedule.map(|s| s.telemetry_ai(traffic)).unwrap_or(serde_json::Value::Array(Vec::new())),
    });
    if let Some(pl) = player {
        let pos = pl.vehicle.position;
        let ts = omsi_map::tile_size();
        let (tx_, ty_) = ((pos.x / ts).floor(), (pos.y / ts).floor());
        v["player"] = serde_json::json!({
            "x": pos.x, "y": pos.y, "z": pos.z,
            "tile_x": tx_ as i64, "tile_y": ty_ as i64,
            "local_x": pos.x - tx_ * ts, "local_y": pos.y - ty_ * ts,
            "heading": pl.vehicle.heading,
            "speed_kmh": pl.vehicle.physics.velocity_kmh(),
            "delay_s": pl.vehicle.host.tt_delay,
            "passengers": passengers,
        });
        if let Some(d) = duty {
            if let Some(trip) = d.trips.get(d.trip_index) {
                let stops: Vec<serde_json::Value> = trip
                    .stops
                    .iter()
                    .map(|s| serde_json::json!({ "object_id": s.object_id, "name": s.name.trim(), "stops": s.stops, "arr": s.arr, "dep": s.dep, "x": s.position.map(|q| q.x), "y": s.position.map(|q| q.y) }))
                    .collect();
                let next = trip.stops.get(d.next_stop);
                let dist = next.and_then(|s| s.position).map(|q| ((q.x - pos.x).powi(2) + (q.y - pos.y).powi(2)).sqrt());
                // (the previous stop the bus actually calls at)
                let prev = trip.stops.iter().take(d.next_stop).rev().find(|s| s.stops);
                v["duty"] = serde_json::json!({
                    "line": if trip.line.trim().is_empty() { d.line.trim() } else { trip.line.trim() },
                    "tour": d.tour,
                    "trip": trip.name,
                    "trip_index": d.trip_index,
                    "trip_count": d.trips.len(),
                    "terminus": trip.terminus.trim(),
                    "next_stop_index": d.next_stop,
                    "next_stop_id": next.map(|s| s.object_id),
                    "next_stop_name": next.map(|s| s.name.trim().to_string()),
                    "next_stop_dist": dist,
                    "prev_stop_id": prev.map(|s| s.object_id),
                    "at_stop": d.at_stop(),
                    "stops": stops,
                });
            }
        }
    }
    let _ = tx.send((p, serde_json::to_vec(&v).unwrap_or_default()));
}
