//! `.osn` situations (unit `mc_situation`).

use omsi_cfg::CfgFile;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SituationVehicle {
    pub file: String,
    pub pos: [f64; 3],
    /// The seven numbers after the position: the body's rotation as a quaternion (x, y, z,
    /// w; Direct3D frame, y up) and three more (0 for a standing vehicle). Nine slots.
    pub orientation: [f64; 9],
    pub tile: (i32, i32),
    /// Total odometer kilometres, after the tile coordinates: Omsi.exe 0x64207E writes
    /// the starting reading plus the distance driven, and 0x643DF3 restores it.
    pub odometer_km: f64,
    pub paint: String,
    pub coupled_with: Option<i32>,
    pub is_my_vehicle: bool,
    pub vars: Vec<(String, f64)>,
    pub string_vars: Vec<(String, String)>,
    pub timetable: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Situation {
    pub path: PathBuf,
    pub name: String,
    pub description: String,
    pub map: String,
    pub weather: Option<String>,
    /// year, day of year, hour, minute, second
    pub time: (i32, i32, i32, i32, f64),
    pub center_tile: (i32, i32),
    pub map_cam: Vec<f64>,
    pub ego_pos: Vec<f64>,
    pub timetable_active: bool,
    pub my_vehicle: i32,
    pub view: i32,
    pub vehicles: Vec<SituationVehicle>,
}

impl Situation {
    /// Write the situation the way OMSI does (UTF-16 LE with a BOM, CR LF).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut t = String::new();
        t.push_str(&format!("[name]\r\n{}\r\n", self.name));
        t.push_str(&format!("[description]\r\n{}\r\n[end]\r\n\r\n", self.description));
        t.push_str(&format!("[map]\r\n{}\r\n\r\n", self.map));
        if let Some(w) = &self.weather {
            // (`[actuWeather]` as OMSI writes and reads it; `[weather]` was never read back)
            t.push_str(&format!("[actuWeather]\r\n{w}\r\n\r\n"));
        }
        let (y, doy, hh, mm, ss) = self.time;
        t.push_str(&format!("[time]\r\n{y}\r\n{doy}\r\n{hh}\r\n{mm}\r\n{ss:.6}\r\n\r\n"));
        t.push_str(&format!("[centerkachel]\r\n{}\r\n{}\r\n\r\n", self.center_tile.0, self.center_tile.1));
        if self.map_cam.len() == 6 {
            t.push_str("[mapcam]\r\n");
            for v in &self.map_cam {
                t.push_str(&format!("{v:.6}\r\n"));
            }
        }
        if self.ego_pos.len() == 5 {
            t.push_str("[egopos]\r\n");
            for v in &self.ego_pos {
                t.push_str(&format!("{v:.6}\r\n"));
            }
        }
        if self.timetable_active {
            t.push_str("\r\n[TT_active]\r\n");
        }
        for (i, v) in self.vehicles.iter().enumerate() {
            t.push_str(&format!("\r\n---------------------------------------------\r\n\r\nFahrzeug Nr. {i}:\r\n\r\n[vehicle]\r\n{}\r\n", v.file));
            for k in 0..3 {
                t.push_str(&format!("{:.3}\r\n", v.pos[k]));
            }
            for k in 0..7 {
                t.push_str(&format!("{:.3}\r\n", v.orientation[k]));
            }
            t.push_str(&format!("{}\r\n{}\r\n{:.3}\r\n{}\r\n", v.tile.0, v.tile.1, v.odometer_km, v.paint));
            if let Some(c) = v.coupled_with {
                t.push_str(&format!("\r\n[coupledWith]\r\n{c}\r\n"));
            }
            if v.is_my_vehicle {
                t.push_str("\r\n[ismyVehicle]\r\n");
            }
            t.push_str(&format!("\r\n[vars]\r\n{}\r\n", v.vars.len()));
            for (n, x) in &v.vars {
                t.push_str(&format!("{n}\r\n{x:.6}\r\n"));
            }
            t.push_str(&format!("\r\n[stringvars]\r\n{}\r\n", v.string_vars.len()));
            for (n, x) in &v.string_vars {
                t.push_str(&format!("{n}\r\n{x}\r\n"));
            }
            if !v.timetable.is_empty() {
                t.push_str("\r\n[settimetable]\r\n");
                for l in &v.timetable {
                    t.push_str(&format!("{l}\r\n"));
                }
            }
        }
        t.push_str(&format!("\r\n[myvehicle]\r\n{}\r\n\r\n[view]\r\n{}\r\n", self.my_vehicle, self.view));
        let mut bytes = vec![0xFF, 0xFE];
        for u in t.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        std::fs::write(path, bytes)
    }

    pub fn load(path: &Path) -> Result<Situation, omsi_cfg::CfgError> {
        let f = CfgFile::read(path)?;
        let mut s = Situation { path: f.path.clone(), my_vehicle: -1, ..Default::default() };
        let mut r = f.reader();
        while let Some(k) = r.next_keyword() {
            match k.as_str() {
                "name" => s.name = r.str().to_string(),
                "description" => s.description = r.until("[end]").join("\n"),
                "map" => s.map = r.str().to_string(),
                "actuweather" => s.weather = Some(r.str().to_string()),
                "time" => s.time = (r.i32(), r.i32(), r.i32(), r.i32(), r.f64()),
                "centerkachel" => s.center_tile = (r.i32(), r.i32()),
                "mapcam" => s.map_cam = (0..6).map(|_| r.f64()).collect(),
                "egopos" => s.ego_pos = (0..5).map(|_| r.f64()).collect(),
                "tt_active" => s.timetable_active = true,
                "myvehicle" => s.my_vehicle = r.i32(),
                "view" => s.view = r.i32(),
                "vehicle" => {
                    let file = r.str().to_string();
                    // x, height, y in the tile, then 7 orientation numbers (a quaternion and
                    // three more) before the tile coordinates
                    let pos = r.f64s::<3>();
                    let mut orientation = [0.0; 9];
                    for v in orientation.iter_mut().take(7) {
                        *v = r.f64();
                    }
                    let tile = (r.i32(), r.i32());
                    let odometer_km = r.f64();
                    let paint = r.str().to_string();
                    s.vehicles.push(SituationVehicle { file, pos, orientation, tile, odometer_km, paint, ..Default::default() });
                }
                "coupledwith" => {
                    let v = r.i32();
                    if let Some(veh) = s.vehicles.last_mut() {
                        veh.coupled_with = Some(v);
                    }
                }
                "ismyvehicle" => {
                    if let Some(veh) = s.vehicles.last_mut() {
                        veh.is_my_vehicle = true;
                    }
                }
                "vars" => {
                    let n = r.usize();
                    let mut vars = Vec::with_capacity(n);
                    for _ in 0..n {
                        let name = r.str().to_string();
                        let v = r.f64();
                        vars.push((name, v));
                    }
                    if let Some(veh) = s.vehicles.last_mut() {
                        veh.vars = vars;
                    }
                }
                "stringvars" => {
                    let n = r.usize();
                    let mut vars = Vec::with_capacity(n);
                    for _ in 0..n {
                        let name = r.str().to_string();
                        let v = r.line().to_string();
                        vars.push((name, v));
                    }
                    if let Some(veh) = s.vehicles.last_mut() {
                        veh.string_vars = vars;
                    }
                }
                "settimetable" => {
                    let v: Vec<String> = (0..6).map(|_| r.str().to_string()).collect();
                    if let Some(veh) = s.vehicles.last_mut() {
                        veh.timetable = v;
                    }
                }
                _ => {}
            }
        }
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn situation_round_trips_padded_device_strings_and_duty_identity() {
        let path =
            std::env::temp_dir().join(format!("omsi_situation_strings_{}.osn", std::process::id()));
        let sit = Situation {
            vehicles: vec![SituationVehicle {
                file: "Vehicles/Test.bus".into(),
                odometer_km: 75556.639,
                vars: vec![("power".into(), 1.0)],
                string_vars: vec![
                    ("line".into(), " 109  ".into()),
                    ("blank".into(), "     ".into()),
                    ("destination".into(), "  Central  ".into()),
                ],
                timetable: ["109", "65104", "16", "7", "0", "0"]
                    .map(String::from)
                    .to_vec(),
                ..Default::default()
            }],
            ..Default::default()
        };
        sit.save(&path).unwrap();
        let loaded = Situation::load(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(loaded.vehicles, sit.vehicles);
    }
}
