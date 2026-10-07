//! The player's own buses (`~/.openomsi/depot.json`): each keeps its look, its counters and its wear.

use crate::install::now_secs;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The fuel in litres, as OMSI's engine scripts keep it.
pub const FUEL: &str = "engine_tank_content";
/// How dirty the body is (0..1), fed to the model by the engine.
pub const DIRT: &str = "Dirt_Norm";

/// A script variable a bus carries from one session to the next.
pub fn is_wear_var(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        "engine_tank_content" | "dirt_norm" | "kmcounter_km" | "kmcounter_m" | "elec_battery_load" | "elec_battery_q_curr" | "elec_battery_age" | "cp_zentralschmierung_nextkm"
    ) || (n.starts_with("collision_energy") && !n.ends_with("_threshold"))
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DepotBus {
    pub id: String,
    pub name: String,
    /// The `.bus` file, relative to the OMSI folder.
    pub bus: String,
    pub paint: String,
    pub plate: String,
    pub number: String,
    pub added: u64,
    pub last_used: u64,
    pub sessions: u32,
    pub metres: f64,
    pub seconds: f64,
    pub stops: i32,
    pub tickets: i32,
    pub takings: f64,
    pub crashes: i32,
    /// Litres burnt, counted between the start and the end of each session.
    pub fuel_used: f64,
    /// The wear variables as the last session left them (none yet: as the bus comes).
    pub wear: Vec<(String, f32)>,
}

impl DepotBus {
    pub fn wear_var(&self, name: &str) -> Option<f32> {
        self.wear.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| *v)
    }

    pub fn fuel(&self) -> Option<f32> {
        self.wear_var(FUEL)
    }

    pub fn dirt(&self) -> Option<f32> {
        self.wear_var(DIRT)
    }

    /// What its odometer shows (km), once a session has read it.
    pub fn odometer(&self) -> Option<f64> {
        Some(self.wear_var("kmcounter_km")? as f64 + self.wear_var("kmcounter_m").unwrap_or(0.0) as f64 / 1000.0)
    }

    /// Collision energy taken and not repaired yet.
    pub fn damage(&self) -> f32 {
        self.wear.iter().filter(|(n, _)| n.to_ascii_lowercase().starts_with("collision_energy")).map(|(_, v)| v.max(0.0)).sum()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Depot {
    pub buses: Vec<DepotBus>,
}

impl Depot {
    pub fn get(&self, id: &str) -> Option<&DepotBus> {
        self.buses.iter().find(|b| b.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut DepotBus> {
        self.buses.iter_mut().find(|b| b.id == id)
    }

    /// Adds a bus under a new id and returns that id.
    pub fn add(&mut self, mut b: DepotBus) -> String {
        let mut n = now_secs();
        while self.buses.iter().any(|x| x.id == format!("{n:x}")) {
            n += 1;
        }
        b.id = format!("{n:x}");
        b.added = now_secs();
        let id = b.id.clone();
        self.buses.push(b);
        id
    }

    pub fn remove(&mut self, id: &str) {
        self.buses.retain(|b| b.id != id);
    }
}

/// Letters of French plates (SIV): no I, O or U, which read as 1, 0 and V.
const PLATE_LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTVWXYZ";

/// A French plate (`AB-123-CD`) from `seed`, as the SIV gives them: no SS, no WW in front, no 000.
pub fn french_plate(seed: u64) -> String {
    let mut x = seed ^ 0x9e37_79b9_7f4a_7c15;
    let mut next = |n: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x % n
    };
    loop {
        let mut pair = || [PLATE_LETTERS[next(PLATE_LETTERS.len() as u64) as usize] as char, PLATE_LETTERS[next(PLATE_LETTERS.len() as u64) as usize] as char];
        let (a, c) = (pair(), pair());
        let n = next(999) + 1;
        if a == ['S', 'S'] || c == ['S', 'S'] || a == ['W', 'W'] {
            continue;
        }
        return format!("{}{}-{n:03}-{}{}", a[0], a[1], c[0], c[1]);
    }
}

impl Depot {
    /// A French plate none of the depot's buses has.
    pub fn new_plate(&self, seed: u64) -> String {
        (0..).map(|k| french_plate(seed.wrapping_add(k))).find(|p| !self.buses.iter().any(|b| b.plate.eq_ignore_ascii_case(p))).unwrap_or_default()
    }
}

/// What one session did to a bus.
#[derive(Debug, Clone, Default)]
pub struct Outing {
    pub metres: f64,
    pub seconds: f64,
    pub stops: i32,
    pub tickets: i32,
    pub takings: f64,
    pub crashes: i32,
}

pub fn path() -> PathBuf {
    crate::data_dir().join("depot.json")
}

pub fn load() -> Result<Depot> {
    load_from(&path())
}

pub fn save(d: &Depot) -> Result<()> {
    save_to(&path(), d)
}

pub fn load_from(p: &Path) -> Result<Depot> {
    match std::fs::read(p) {
        Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("reading {}", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Depot::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", p.display())),
    }
}

/// Written aside first, so that a game and the launcher saving at once never leave half a file.
pub fn save_to(p: &Path, d: &Depot) -> Result<()> {
    let tmp = p.with_extension(format!("json.{}", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec_pretty(d)?)?;
    std::fs::rename(&tmp, p).with_context(|| format!("writing {}", p.display()))
}

/// Adds a session to bus `id` in the file at `p` and keeps the wear it ended with; false when the bus is gone.
pub fn record_in(p: &Path, id: &str, outing: &Outing, wear: Vec<(String, f32)>) -> Result<bool> {
    let mut d = load_from(p)?;
    let Some(b) = d.get_mut(id) else { return Ok(false) };
    let start = b.fuel();
    let end = wear.iter().find(|(n, _)| n.eq_ignore_ascii_case(FUEL)).map(|(_, v)| *v);
    if let (Some(s), Some(e)) = (start, end) {
        b.fuel_used += (s - e).max(0.0) as f64;
    }
    b.sessions += 1;
    b.metres += outing.metres;
    b.seconds += outing.seconds;
    b.stops += outing.stops;
    b.tickets += outing.tickets;
    b.takings += outing.takings;
    b.crashes += outing.crashes;
    b.last_used = now_secs();
    b.wear = wear;
    save_to(p, &d)?;
    Ok(true)
}

pub fn record(id: &str, outing: &Outing, wear: Vec<(String, f32)>) -> Result<bool> {
    record_in(&path(), id, outing, wear)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("omsi-depot-test-{}-{name}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d.join("depot.json")
    }

    #[test]
    fn wear_is_fuel_battery_dirt_and_damage_not_thresholds() {
        for n in ["engine_tank_content", "Dirt_Norm", "kmcounter_km", "elec_battery_load", "collision_energy", "collision_energy_eng"] {
            assert!(is_wear_var(n), "{n}");
        }
        for n in ["collision_energy_eng_threshold", "engine_RPM", "door_0", "cp_odometer_1"] {
            assert!(!is_wear_var(n), "{n}");
        }
    }

    #[test]
    fn french_plates_follow_the_siv() {
        for seed in 0..5000u64 {
            let p = french_plate(seed);
            let c: Vec<char> = p.chars().collect();
            assert_eq!(c.len(), 9, "{p}");
            assert!(c[2] == '-' && c[6] == '-', "{p}");
            for i in [0, 1, 7, 8] {
                assert!(c[i].is_ascii_uppercase() && !"IOU".contains(c[i]), "{p}");
            }
            assert!(c[3..6].iter().all(|d| d.is_ascii_digit()) && &p[3..6] != "000", "{p}");
            assert!(&p[0..2] != "SS" && &p[7..9] != "SS" && &p[0..2] != "WW", "{p}");
        }
        assert_ne!(french_plate(1), french_plate(2));
        let mut d = Depot::default();
        let taken = d.new_plate(42);
        d.add(DepotBus { plate: taken.clone(), ..Default::default() });
        assert_ne!(d.new_plate(42), taken);
    }

    #[test]
    fn a_missing_file_is_an_empty_depot_and_a_saved_one_reads_back() {
        let p = scratch("roundtrip");
        let _ = std::fs::remove_file(&p);
        assert_eq!(load_from(&p).unwrap(), Depot::default());
        let mut d = Depot::default();
        let id = d.add(DepotBus { name: "Mon Citaro".into(), plate: "GP-123-MO".into(), ..Default::default() });
        save_to(&p, &d).unwrap();
        let back = load_from(&p).unwrap();
        assert_eq!(back, d);
        assert_eq!(back.get(&id).unwrap().plate, "GP-123-MO");
    }

    #[test]
    fn a_session_adds_up_and_leaves_its_wear() {
        let p = scratch("record");
        let mut d = Depot::default();
        let id = d.add(DepotBus { wear: vec![(FUEL.into(), 200.0)], metres: 1000.0, ..Default::default() });
        save_to(&p, &d).unwrap();
        let outing = Outing { metres: 12_000.0, seconds: 1800.0, stops: 14, tickets: 3, takings: 6.3, crashes: 1 };
        let wear = vec![(FUEL.into(), 188.5), ("collision_energy".into(), 40.0)];
        assert!(record_in(&p, &id, &outing, wear.clone()).unwrap());
        let b = load_from(&p).unwrap().get(&id).cloned().unwrap();
        assert_eq!((b.sessions, b.metres, b.stops, b.tickets, b.crashes), (1, 13_000.0, 14, 3, 1));
        assert!((b.fuel_used - 11.5).abs() < 1e-6);
        assert_eq!(b.wear, wear);
        assert_eq!(b.damage(), 40.0);
        assert!(!record_in(&p, "gone", &outing, Vec::new()).unwrap());
    }
}
