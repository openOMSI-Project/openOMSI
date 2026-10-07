//! `.bus` / `.ovh` road vehicle definitions (unit `mc_roadvehicle`) and `.zug` train lists.

use omsi_cfg::CfgFile;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VehicleKind {
    /// `.bus`
    #[default]
    Bus,
    /// `.ovh` `[type]` 0 = car/other road vehicle, 1 = ?, 2 = rail, 3 = aircraft/helicopter…
    Other(i32),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Camera {
    pub pos: [f32; 3],
    pub dist: f32,
    pub fov: f32,
    pub yaw: f32,
    pub pitch: f32,
    /// `[add_camera_reflexion_2]` extra parameter.
    pub extra: Option<f32>,
    /// `[add_camera_reflexion_static]` (openOMSI): a camera for a screen (CCTV), not a mirror
    /// - it looks where its yaw and pitch point, whoever looks at its picture.
    pub fixed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Axle {
    pub long: f32,
    pub max_width: f32,
    pub min_width: f32,
    pub wheel_diameter: f32,
    pub spring: f32,
    pub max_force: f32,
    pub damper: f32,
    pub driven: bool,
    pub inertia_inv: f32,
}

impl Default for Axle {
    fn default() -> Self {
        Self { long: 0.0, max_width: 2.0, min_width: 1.5, wheel_diameter: 1.0, spring: 0.0, max_force: 0.0, damper: 0.0, driven: false, inertia_inv: 0.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Attachment {
    pub ops: Vec<(String, Vec<f32>)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScriptSet {
    pub varlists: Vec<PathBuf>,
    pub stringvarlists: Vec<PathBuf>,
    pub scripts: Vec<PathBuf>,
    pub constfiles: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Coupling {
    pub pos: [f32; 3],
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ControlCable {
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Vehicle {
    pub path: PathBuf,
    pub kind: VehicleKind,
    pub manufacturer: String,
    pub type_name: String,
    pub default_paint: String,
    /// The file has a `[friendlyname]`: OMSI offers exactly these in its vehicle list.
    /// Rear sections of articulated buses, trailers and most AI-only variants have none.
    pub has_friendly_name: bool,
    pub friendly_name_inv: Vec<String>,
    pub description: String,
    pub ai_veh_type: i32,
    /// The file has an `[ai_veh_type]`: a vehicle made for the AI traffic (a fire engine, a
    /// lorry, a taxi in a `.bus`), without a cab to drive from - the launchers do not offer it.
    pub ai_only: bool,
    pub number_file: Option<String>,
    /// `[registration_automatic]`: prefix and postfix around the fleet number.
    pub registration_automatic: Option<(String, String)>,
    /// `[registration_list]`: the file of plates (line by line beside the `[number]` list),
    /// then the prefix and postfix for a number the file has no plate for.
    pub registration_list: Option<(String, String, String)>,
    pub registration_free: bool,
    /// The plate mode the last of those keywords set (TRoadVehicle +0x28d): 0 none, 1 free,
    /// 2 list, 3 automatic; and the prefix and postfix the list and automatic modes share
    /// (+0x2a4, +0x2a8: the later keyword's).
    pub registration_mode: u8,
    pub registration_affix: (String, String),
    pub km_counter_init: Option<(i32, f32)>,
    pub sound: Option<String>,
    pub sound_ai: Option<String>,
    pub model: Option<String>,
    pub paths: Option<String>,
    pub passenger_cabin: Option<String>,
    pub scripts: ScriptSet,
    pub script_share: bool,
    pub cameras_driver: Vec<Camera>,
    pub cameras_pax: Vec<Camera>,
    pub cameras_reflexion: Vec<Camera>,
    pub view_schedule: Option<usize>,
    pub view_ticketselling: Option<usize>,
    pub camera_std: usize,
    pub camera_outside_center: [f32; 3],
    pub mass: f32,
    pub moment_of_inertia: [f32; 3],
    pub bounding_box: Option<[f32; 6]>,
    /// `[cog]`: centre of gravity (x right, y forward, z up) of the physics object.
    pub cog: Option<[f32; 3]>,
    /// `[schwerpunkt]`: height of the centre of gravity.
    pub cog_height: f32,
    pub rolling_resistance: f32,
    pub rot_pnt_long: f32,
    pub inv_min_turn_radius: f32,
    pub ai_delta_height: f32,
    pub axles: Vec<Axle>,
    pub attachments: Vec<Attachment>,
    pub coupling_front: Option<Coupling>,
    pub coupling_back: Option<Coupling>,
    pub couple_front: Option<(String, bool)>,
    pub couple_back: Option<(String, bool)>,
    pub couple_front_open_for_sound: bool,
    pub coupling_front_character: Option<[f32; 4]>,
    pub control_cable_front: Vec<ControlCable>,
    pub control_cable_back: Vec<ControlCable>,
    pub rowdy_factor: Option<(f32, f32)>,
    pub boogies: Option<f32>,
    pub sinus: Option<[f32; 4]>,
    pub rail_body_osc: Option<[f32; 7]>,
    pub contact_shoes: Vec<[f32; 6]>,
    pub ai_brake_performance: Option<[f32; 5]>,
    pub fixed: bool,
    pub unknown_keywords: Vec<(String, usize)>,
}

fn read_list(r: &mut omsi_cfg::CfgReader, base: &Path) -> Vec<PathBuf> {
    let n = r.usize();
    (0..n).map(|_| r.str().to_string()).filter(|s| !s.trim().is_empty()).map(|s| omsi_cfg::resolve_path(base, &s)).collect()
}

fn read_camera(r: &mut omsi_cfg::CfgReader, extra: bool) -> Camera {
    let pos = r.f32s::<3>();
    let dist = r.f32();
    let fov = r.f32();
    let yaw = r.f32();
    let pitch = r.f32();
    let extra = if extra { Some(r.f32()) } else { None };
    Camera { pos, dist, fov, yaw, pitch, extra, fixed: false }
}

pub fn parse_attachment(r: &mut omsi_cfg::CfgReader) -> Attachment {
    let mut a = Attachment::default();
    loop {
        let save = r.pos();
        let l = r.str();
        let w = l.trim().to_ascii_lowercase();
        match w.as_str() {
            "attach_trans" => a.ops.push((w, r.f32s::<3>().to_vec())),
            "attach_rot_x" | "attach_rot_y" | "attach_rot_z" => a.ops.push((w, vec![r.f32()])),
            _ => {
                if omsi_cfg::keyword_of(l).is_some() || r.at_end() {
                    r.seek(save);
                    break;
                }
            }
        }
    }
    a
}

fn parse_axle(r: &mut omsi_cfg::CfgReader) -> Axle {
    let mut a = Axle::default();
    // Old files (the stock AI cars, the Manta) list the values without their keywords, one
    // per line: long, max width, min width, wheel diameter, spring, max force, damper,
    // driven, inertia. Read that way, the Golf's wheels are 0.503 m, not the default 1 m
    // that made every AI car's wheels turn at half their speed.
    let first = r.lines().get(r.pos()).map(|l| l.trim().to_string()).unwrap_or_default();
    if omsi_cfg::keyword_of(&first).is_none() && first.parse::<f32>().is_ok() {
        let mut vals = Vec::new();
        while vals.len() < 9 && !r.at_end() {
            let save = r.pos();
            match r.word().parse::<f32>() {
                Ok(v) => vals.push(v),
                Err(_) => {
                    r.seek(save);
                    break;
                }
            }
        }
        let slots: [&mut f32; 7] = [&mut a.long, &mut a.max_width, &mut a.min_width, &mut a.wheel_diameter, &mut a.spring, &mut a.max_force, &mut a.damper];
        for (slot, v) in slots.into_iter().zip(vals.iter()) {
            *slot = *v;
        }
        if let Some(d) = vals.get(7) {
            a.driven = *d != 0.0;
        }
        if let Some(i) = vals.get(8) {
            a.inertia_inv = *i;
        }
        return a;
    }
    loop {
        let save = r.pos();
        let l = r.str();
        let w = l.trim().to_ascii_lowercase();
        match w.as_str() {
            "achse_long" => a.long = r.f32(),
            "achse_maxwidth" => a.max_width = r.f32(),
            "achse_minwidth" => a.min_width = r.f32(),
            "achse_raddurchmesser" => a.wheel_diameter = r.f32(),
            "achse_feder" => a.spring = r.f32(),
            "achse_maxforce" => a.max_force = r.f32(),
            "achse_daempfer" => a.damper = r.f32(),
            "achse_antrieb" => {
                let w = r.word();
                a.driven = w.parse::<f32>().map(|x| x != 0.0).unwrap_or(w.eq_ignore_ascii_case("true"));
            }
            "achse_inertia_inv" => a.inertia_inv = r.f32(),
            _ => {
                if omsi_cfg::keyword_of(l).is_some() || r.at_end() {
                    r.seek(save);
                    break;
                }
                // comment line between the parameters: skip
            }
        }
    }
    a
}

impl Vehicle {
    pub fn load(path: &Path) -> Result<Vehicle, omsi_cfg::CfgError> {
        let f = CfgFile::read(path)?;
        Ok(Self::parse(&f))
    }

    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new(""))
    }

    /// Whether the vehicle is offered for driving. OMSI 2 lists the `.bus`/`.ovh` files
    /// that carry a `[friendlyname]` (every stock bus does; the GN92's rear section, most
    /// `_KI` AI variants and the AI cars do not), so a rear section never shows up as a
    /// bus of its own.
    pub fn is_selectable(&self) -> bool {
        self.has_friendly_name
    }

    /// The file of the vehicle coupled behind this one (`[couple_back]`), wherever it lives.
    pub fn couple_back_path(&self) -> Option<PathBuf> {
        self.couple_back.as_ref().map(|(f, _)| omsi_cfg::resolve_path(self.dir(), f))
    }

    /// A part that is only ever coupled behind another vehicle: the rear section of an
    /// articulated bus (it has a front coupling and no name of its own).
    pub fn is_rear_section(&self) -> bool {
        self.coupling_front.is_some() && !self.has_friendly_name
    }

    /// A rail vehicle: a car of a `.zug` runs on rails, which Omsi.exe stands end to end by
    /// their model bodies (its cars' declared coupling points need not be at the cars' ends).
    /// A road vehicle, a trailer or an articulated-bus rear section keeps its declared
    /// `[coupling_front]` / `[coupling_back]`.
    ///
    /// Rails are what the file says they are: a `[boogies]`, a `[contact_shoe]` or a
    /// `[rail_body_osc]` (`rail_drive::is_rail` reads the same three; a rail car has all of
    /// a bogie, and Omsi.exe's `[type] 2` marks the same vehicles).
    pub fn is_rail(&self) -> bool {
        self.boogies.is_some() || !self.contact_shoes.is_empty() || self.rail_body_osc.is_some()
    }

    pub fn parse(file: &CfgFile) -> Vehicle {
        let is_ovh = file.path.extension().map(|e| e.eq_ignore_ascii_case("ovh")).unwrap_or(false);
        let mut v = Vehicle { path: file.path.clone(), kind: if is_ovh { VehicleKind::Other(0) } else { VehicleKind::Bus }, mass: 1000.0, ..Default::default() };
        let base = file.dir().to_path_buf();
        let mut r = file.reader().disabled_blocks();
        while let Some(k) = r.next_keyword() {
            match k.as_str() {
                "type" => v.kind = VehicleKind::Other(r.i32()),
                "friendlyname" => {
                    v.has_friendly_name = true;
                    v.manufacturer = r.str().to_string();
                    v.type_name = r.str().to_string();
                    v.default_paint = r.str().to_string();
                }
                "friendlyname_inv" => v.friendly_name_inv = (0..3).map(|_| r.str().to_string()).collect(),
                "description" => v.description = r.until("[end]").join("\n"),
                "ai_veh_type" => {
                    v.ai_veh_type = r.i32();
                    v.ai_only = true;
                }
                "number" => v.number_file = Some(r.str().to_string()),
                // Omsi.exe (TRoadVehicle.LoadFromFile 0x7cddf5, 0x7cde6e) reads these lines
                // as they come, whatever they say: the automatic mode's prefix and postfix
                // (the stock "B-V " with its space), the list mode's file, prefix and postfix.
                // The lines end where the next block starts, so a file that leaves them out
                // (the Urumqi AI cars' `[registration_automatic]` straight before `[model]`)
                // reads them as empty instead of taking the keyword for one of them.
                "registration_automatic" => {
                    let pre = r.param_line().to_string();
                    let post = r.param_line().to_string();
                    v.registration_automatic = Some((pre.clone(), post.clone()));
                    v.registration_mode = 3;
                    v.registration_affix = (pre, post);
                }
                "registration_list" => {
                    let file = r.param_line().trim().to_string();
                    let pre = r.param_line().to_string();
                    let post = r.param_line().to_string();
                    v.registration_list = Some((file, pre.clone(), post.clone()));
                    v.registration_mode = 2;
                    v.registration_affix = (pre, post);
                }
                "registration_free" => {
                    v.registration_free = true;
                    v.registration_mode = 1;
                }
                "kmcounter_init" => {
                    let y = r.i32();
                    let km = r.f32();
                    v.km_counter_init = Some((y, km));
                }
                "sound" => v.sound = Some(r.str().to_string()),
                "sound_ai" => v.sound_ai = Some(r.str().to_string()),
                "model" => v.model = Some(r.str().to_string()),
                "paths" => v.paths = Some(r.str().to_string()),
                "passengercabin" => v.passenger_cabin = Some(r.str().to_string()),
                "varnamelist" => v.scripts.varlists.extend(read_list(&mut r, &base)),
                "stringvarnamelist" => v.scripts.stringvarlists.extend(read_list(&mut r, &base)),
                "script" => v.scripts.scripts.extend(read_list(&mut r, &base)),
                "constfile" => v.scripts.constfiles.extend(read_list(&mut r, &base)),
                "scriptshare" => v.script_share = true,
                "add_camera_driver" => v.cameras_driver.push(read_camera(&mut r, false)),
                "add_camera_pax" => v.cameras_pax.push(read_camera(&mut r, false)),
                "add_camera_reflexion" => v.cameras_reflexion.push(read_camera(&mut r, false)),
                "add_camera_reflexion_2" => v.cameras_reflexion.push(read_camera(&mut r, true)),
                "add_camera_reflexion_static" => v.cameras_reflexion.push(Camera { fixed: true, ..read_camera(&mut r, false) }),
                "view_schedule" => v.view_schedule = Some(v.cameras_driver.len().saturating_sub(1)),
                "view_ticketselling" => v.view_ticketselling = Some(v.cameras_driver.len().saturating_sub(1)),
                "set_camera_std" => v.camera_std = r.usize(),
                "set_camera_outside_center" => v.camera_outside_center = r.f32s::<3>(),
                "mass" => v.mass = r.f32(),
                "momentofintertia" => v.moment_of_inertia = r.f32s::<3>(),
                "boundingbox" => {
                    // (the sizes as magnitudes: a mod's box given -2.62 m wide crossed the
                    // bounds of the walkers' clamp about it and the game stopped, #986)
                    let mut bb = r.f32s::<6>();
                    for x in &mut bb[..3] {
                        *x = x.abs();
                    }
                    v.bounding_box = Some(bb);
                }
                "cog" => v.cog = Some(r.f32s::<3>()),
                "schwerpunkt" => v.cog_height = r.f32(),
                "rollwiderstand" => v.rolling_resistance = r.f32(),
                "rot_pnt_long" => v.rot_pnt_long = r.f32(),
                "inv_min_turnradius" => v.inv_min_turn_radius = r.f32(),
                "ai_deltaheight" => v.ai_delta_height = r.f32(),
                "newachse" => v.axles.push(parse_axle(&mut r)),
                "new_attachment" => v.attachments.push(parse_attachment(&mut r)),
                "coupling_front" => v.coupling_front = Some(Coupling { pos: r.f32s::<3>() }),
                "coupling_back" => v.coupling_back = Some(Coupling { pos: r.f32s::<3>() }),
                "couple_front" | "couple_back" | "couple" => {
                    let f = r.str().to_string();
                    let save = r.pos();
                    let b = r.word();
                    let flag = if b.eq_ignore_ascii_case("true") {
                        true
                    } else if b.eq_ignore_ascii_case("false") {
                        false
                    } else {
                        r.seek(save);
                        false
                    };
                    if k == "couple_back" {
                        v.couple_back = Some((f, flag));
                    } else {
                        v.couple_front = Some((f, flag));
                    }
                }
                "couple_front_open_for_sound" => v.couple_front_open_for_sound = true,
                "coupling_front_character" => v.coupling_front_character = Some(r.f32s::<4>()),
                "control_cable_front" => v.control_cable_front.push(ControlCable { lines: (0..5).map(|_| r.str().to_string()).collect() }),
                "control_cable_back" => v.control_cable_back.push(ControlCable { lines: (0..5).map(|_| r.str().to_string()).collect() }),
                "rowdy_factor" => {
                    let a = r.f32();
                    let b = r.f32();
                    v.rowdy_factor = Some((a, b));
                }
                "boogies" => v.boogies = Some(r.f32()),
                "sinus" => v.sinus = Some(r.f32s::<4>()),
                "rail_body_osc" => v.rail_body_osc = Some(r.f32s::<7>()),
                "contact_shoe" => v.contact_shoes.push(r.f32s::<6>()),
                "ai_brakeperformance" => v.ai_brake_performance = Some(r.f32s::<5>()),
                "fixed" => v.fixed = true,
                _ => v.unknown_keywords.push((k, r.block_line())),
            }
        }
        v
    }
}

/// Content paths name files case-insensitively (a mod writes `[couple_back]` however it
/// likes, and a case-insensitive file system hands the spelling back unchanged).
fn same_file(a: &Path, b: &Path) -> bool {
    let (x, y) = (a.canonicalize().unwrap_or_else(|_| a.to_path_buf()), b.canonicalize().unwrap_or_else(|_| b.to_path_buf()));
    x.to_string_lossy().eq_ignore_ascii_case(&y.to_string_lossy())
}

/// Of the vehicle files of one vehicle folder (every content root's copy together), the
/// ones offered for driving, loaded and in the given order: those with a `[friendlyname]`
/// - OMSI's own rule - that no other file of the folder couples behind itself. The second
/// part keeps a rear section out of the list even when a mod copied the front section's
/// name into it: a coupled part only ever comes with its front.
pub fn offered_vehicles(files: &[PathBuf]) -> Vec<(PathBuf, Vehicle)> {
    let loaded: Vec<(PathBuf, Vehicle)> = files
        .iter()
        .filter(|f| f.extension().map(|e| e.eq_ignore_ascii_case("bus") || e.eq_ignore_ascii_case("ovh")).unwrap_or(false))
        .filter_map(|f| Vehicle::load(f).ok().map(|v| (f.clone(), v)))
        .collect();
    let coupled: Vec<PathBuf> = loaded.iter().filter_map(|(_, v)| v.couple_back_path()).collect();
    loaded.into_iter().filter(|(f, v)| v.is_selectable() && !(v.coupling_front.is_some() && coupled.iter().any(|c| same_file(c, f)))).collect()
}

/// The vehicles that couple `rear` behind them (`[couple_back]`), looked for in its folder
/// and in the same folder of every other content root; for the rear section of an
/// articulated bus, the front section it belongs to. A chain (front → middle → rear) is
/// followed to its selectable front.
pub fn front_sections_of(rear: &Path) -> Vec<PathBuf> {
    fn direct(rear: &Path) -> Vec<(PathBuf, Vehicle)> {
        let Some(dir) = rear.parent() else { return Vec::new() };
        // the folder under every content root (archives read in place too)
        let mut dirs = vec![dir.to_path_buf()];
        for d in omsi_cfg::mirrored_dirs(dir) {
            if !dirs.contains(&d) {
                dirs.push(d);
            }
        }
        let mut out = Vec::new();
        for d in dirs {
            let mut files: Vec<PathBuf> = omsi_cfg::vfs::read_dir_paths(&d).into_iter().filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("bus") || e.eq_ignore_ascii_case("ovh")).unwrap_or(false)).collect();
            files.sort();
            for f in files {
                if same_file(&f, rear) {
                    continue;
                }
                let Ok(v) = Vehicle::load(&f) else { continue };
                if v.couple_back_path().map(|p| same_file(&p, rear)).unwrap_or(false) {
                    out.push((f, v));
                }
            }
        }
        out
    }
    let mut out = Vec::new();
    let mut todo = vec![rear.to_path_buf()];
    let mut seen: Vec<PathBuf> = Vec::new();
    while let Some(p) = todo.pop() {
        if seen.iter().any(|s| same_file(s, &p)) || seen.len() > 8 {
            continue;
        }
        seen.push(p.clone());
        for (f, v) in direct(&p) {
            if v.is_selectable() {
                out.push(f);
            } else {
                todo.push(f);
            }
        }
    }
    out
}

/// `.zug`: pairs of lines (vehicle file, reverse flag).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Train {
    pub cars: Vec<(String, bool)>,
}

impl Train {
    pub fn load(path: &Path) -> Result<Train, omsi_cfg::CfgError> {
        let f = CfgFile::read(path)?;
        let mut t = Train::default();
        let lines: Vec<&String> = f.lines.iter().filter(|l| !l.trim().is_empty()).collect();
        for pair in lines.chunks(2) {
            let file = pair[0].trim().to_string();
            let rev = pair.get(1).map(|s| s.trim() == "1").unwrap_or(false);
            t.cars.push((file, rev));
        }
        Ok(t)
    }
}

/// A `[number]` list (`.org`): one fleet number a line - every line, the first as well.
/// (The stock `.bus` files describe a first line naming a repaint; Omsi.exe 2.3 reads none:
/// 0x614f90 takes each non-empty line for a number.)
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NumberList {
    pub numbers: Vec<String>,
}

impl NumberList {
    pub fn load(path: &Path) -> Result<NumberList, omsi_cfg::CfgError> {
        let f = CfgFile::read(path)?;
        let numbers = f.lines.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        Ok(NumberList { numbers })
    }
}

impl Vehicle {
    /// The fleet numbers of the `[number]` list with the plate `[registration_list]`'s file
    /// gives each - line by line beside it, empty lines counted (0x614f90) - and no plate
    /// where that file has none.
    pub fn numbers_with_plates(&self) -> Vec<(String, String)> {
        let Some(list) = self.number_file.as_ref() else { return Vec::new() };
        // (read once per bus: the AI asks it for every bus it puts on the road)
        static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, Vec<(String, String)>>>> = std::sync::OnceLock::new();
        let key = self.path.clone();
        if let Some(v) = CACHE.get_or_init(Default::default).lock().ok().and_then(|c| c.get(&key).cloned()) {
            return v;
        }
        let out = self.read_numbers_with_plates(list);
        if let Ok(mut c) = CACHE.get_or_init(Default::default).lock() {
            c.insert(key, out.clone());
        }
        out
    }

    fn read_numbers_with_plates(&self, list: &str) -> Vec<(String, String)> {
        let Ok(numbers) = CfgFile::read(&omsi_cfg::resolve_path(self.dir(), list)) else { return Vec::new() };
        let plates = self
            .registration_list
            .as_ref()
            .and_then(|(f, _, _)| CfgFile::read(&omsi_cfg::resolve_path(self.dir(), f)).ok());
        numbers
            .lines
            .iter()
            .enumerate()
            .filter(|(_, n)| !n.trim().is_empty())
            .map(|(i, n)| {
                let plate = plates.as_ref().and_then(|p| p.lines.get(i)).map(|p| p.trim_end().to_string()).unwrap_or_default();
                (n.trim().to_string(), plate)
            })
            .collect()
    }

    /// The plate of fleet number `number`, as Omsi.exe gives it to an AI bus not in the
    /// `[registration_free]` mode (0x7e7a80, from the depot buses' 0x70a174): the
    /// `[registration_list]` file's plate of that number when it has one, whatever the mode,
    /// else prefix, number and postfix of the list or automatic mode - the number alone
    /// without a mode.
    pub fn plate_of_number(&self, number: &str) -> String {
        self.plate_from(number, self.registration_list.is_some())
    }

    /// The plate the player's bus gets for fleet number `number`: the `[registration_list]`
    /// file's plate when it has one for that number, whatever keyword came last, as the AI's
    /// (#133, #1584, #1591). Taken from the list only in the list mode - a repaint's
    /// `[registration_list]` followed by the template's `[registration_automatic]` - the
    /// player's bus showed the German prefix and its fleet number where OMSI 2 shows the
    /// plate, and a Hong Kong number `ATENU1035@@UC 6645@@S` came out as `ATENU1035`.
    pub fn chosen_plate_of_number(&self, number: &str) -> String {
        self.plate_from(number, self.registration_list.is_some())
    }

    fn plate_from(&self, number: &str, list: bool) -> String {
        if list {
            if let Some((_, p)) = self.numbers_with_plates().into_iter().find(|(n, p)| n == number.trim() && !p.is_empty()) {
                return p;
            }
        }
        let (pre, post) = (&self.registration_affix.0, &self.registration_affix.1);
        format!("{pre}{}{post}", number.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axles_with_and_without_keywords() {
        let keyed = "[newachse]\nachse_long\n2.943\nachse_raddurchmesser\n1.023\n1.05\nachse_feder\n240\nachse_antrieb\n0\n[newachse]\n-2.577\n2.4\n1.4\n1.023\n280\n116\n20\n1\n0.015\n\n[mass]\n10.9\n[cog]\n0\n0.2\n0.8\n";
        let v = Vehicle::parse(&CfgFile::from_str("x.bus", keyed));
        assert_eq!(v.axles.len(), 2);
        assert_eq!((v.axles[0].long, v.axles[0].wheel_diameter, v.axles[0].spring, v.axles[0].driven), (2.943, 1.023, 240.0, false));
        let b = &v.axles[1];
        assert_eq!((b.long, b.max_width, b.min_width, b.wheel_diameter, b.spring, b.max_force, b.damper, b.driven, b.inertia_inv), (-2.577, 2.4, 1.4, 1.023, 280.0, 116.0, 20.0, true, 0.015));
        assert_eq!(v.mass, 10.9);
        assert_eq!(v.cog, Some([0.0, 0.2, 0.8]));
    }

    /// `[add_camera_reflexion_static]` is one more reflection camera, numbered with the
    /// mirrors, that keeps its direction.
    #[test]
    fn a_static_reflexion_camera_counts_with_the_mirrors() {
        let text = "[add_camera_reflexion]\n-1.2\n5.5\n2\n0\n30\n201\n-2\n\n[add_camera_reflexion_static]\n0.9\n-1\n2.6\n0\n70\n90\n-40\n\n[add_camera_reflexion_2]\n1.2\n5.5\n2\n0\n30\n159\n-2\n0.2\n";
        let v = Vehicle::parse(&CfgFile::from_str("x.bus", text));
        let c = &v.cameras_reflexion;
        assert_eq!(c.len(), 3);
        assert_eq!(c.iter().map(|c| c.fixed).collect::<Vec<_>>(), [false, true, false]);
        assert_eq!((c[1].pos, c[1].fov, c[1].yaw, c[1].pitch, c[1].extra), ([0.9, -1.0, 2.6], 70.0, 90.0, -40.0, None));
        assert_eq!(c[2].extra, Some(0.2));
    }

    #[test]
    fn a_bounding_box_given_negative_is_its_size() {
        let v = Vehicle::parse(&CfgFile::from_str("x.bus", "[boundingbox]\n-2.62\n12.2\n-3.4\n0\n-0.3\n1.7\n"));
        assert_eq!(v.bounding_box, Some([2.62, 12.2, 3.4, 0.0, -0.3, 1.7]));
    }

    #[test]
    fn a_share_of_the_drive_is_a_driven_axle() {
        let text = "[newachse]\nachse_long\n-2.9\nachse_antrieb\n0.2\n[newachse]\nachse_long\n2.9\nachse_antrieb\n0\n";
        let v = Vehicle::parse(&CfgFile::from_str("x.bus", text));
        assert_eq!(v.axles.iter().map(|a| a.driven).collect::<Vec<_>>(), vec![true, false]);
    }

    #[test]
    fn rear_sections_are_not_listed_and_lead_to_their_front() {
        let dir = std::env::temp_dir().join(format!("omsi_vehicle_couple_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("G Main.bus"), "[friendlyname]\nMB\nO530G\nDefault\n\n[coupling_back]\n0\n-4\n0.3\n\n[couple_back]\ng trail.BUS\nfalse\n").unwrap();
        std::fs::write(dir.join("G Trail.bus"), "[scriptshare]\n\n[coupling_front]\n0\n4\n0.3\n").unwrap();
        std::fs::write(dir.join("Solo.bus"), "[friendlyname]\nMB\nO530\nDefault\n").unwrap();
        std::fs::write(dir.join("Solo_KI.bus"), "[model]\nx.cfg\n").unwrap();
        let main = Vehicle::load(&dir.join("G Main.bus")).unwrap();
        let trail = Vehicle::load(&dir.join("G Trail.bus")).unwrap();
        let ki = Vehicle::load(&dir.join("Solo_KI.bus")).unwrap();
        assert!(main.is_selectable() && !main.is_rear_section());
        assert!(!trail.is_selectable() && trail.is_rear_section() && trail.script_share);
        assert!(!ki.is_selectable() && !ki.is_rear_section());
        // the case of the [couple_back] name does not matter
        assert!(main.couple_back_path().unwrap().to_string_lossy().to_ascii_lowercase().ends_with("g trail.bus"));
        let fronts = front_sections_of(&dir.join("G Trail.bus"));
        assert_eq!(fronts.len(), 1);
        assert_eq!(fronts[0].file_name().unwrap(), "G Main.bus");
        assert!(front_sections_of(&dir.join("Solo.bus")).is_empty());
        // a rear section that carries the front's [friendlyname] is still not offered
        std::fs::write(dir.join("L Main.bus"), "[friendlyname]\nMB\nO530GL\nDefault\n\n[couple_back]\nL Trail.bus\nfalse\n").unwrap();
        std::fs::write(dir.join("L Trail.bus"), "[friendlyname]\nMB\nO530GL\nDefault\n\n[coupling_front]\n0\n4\n0.3\n").unwrap();
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
        files.sort();
        let offered: Vec<String> = offered_vehicles(&files).iter().map(|(f, _)| f.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(offered, vec!["G Main.bus", "L Main.bus", "Solo.bus"]);
        assert_eq!(front_sections_of(&dir.join("L Trail.bus")).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An `.ovh` that leaves the registration affixes out (`[registration_automatic]` with
    /// nothing but a blank line before `[model]`, as the Urumqi AI cars write it): the
    /// `[model]` is a keyword, not the postfix, so the vehicle keeps its model.
    #[test]
    fn an_empty_registration_affix_keeps_the_next_keyword() {
        let v = Vehicle::parse(&CfgFile::from_str("x.ovh", "[registration_free]\n\n[registration_automatic]\n\n[model]\nmodel\\model.cfg\n\n[sound]\ns.cfg\n"));
        assert_eq!(v.registration_mode, 3);
        assert_eq!(v.registration_affix, (String::new(), String::new()));
        assert_eq!(v.model.as_deref(), Some("model\\model.cfg"));
        // the stock shape (prefix "B-V ", blank postfix) is unchanged
        let s = Vehicle::parse(&CfgFile::from_str("y.bus", "[registration_automatic]\nB-V \n\n[model]\nm.cfg\n"));
        assert_eq!(s.registration_affix, ("B-V ".to_string(), String::new()));
        assert_eq!(s.model.as_deref(), Some("m.cfg"));
    }

    /// A repaint's own `[registration_list]` followed by the template's
    /// `[registration_automatic]`: the list's plate still wins (Omsi.exe 0x7e7a80 reads the
    /// list file whatever the mode), prefix + number only where the list has none.
    #[test]
    fn list_plate_wins_over_a_later_automatic_mode() {
        let dir = std::env::temp_dir().join(format!("omsi_vehicle_regs_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Nos.org"), "E1\nE2\n").unwrap();
        std::fs::write(dir.join("Regs.org"), "AB12 CDE\n").unwrap();
        std::fs::write(dir.join("x.bus"), "[number]\nNos.org\n\n[registration_list]\nRegs.org\n\n\n\n[registration_automatic]\nB-V \n\n").unwrap();
        let v = Vehicle::load(&dir.join("x.bus")).unwrap();
        assert_eq!(v.registration_mode, 3);
        assert_eq!(v.plate_of_number("E1"), "AB12 CDE");
        assert_eq!(v.plate_of_number("E2"), "B-V E2");
        // the player's bus too (#1584): the list's plate, prefix + number where it has none
        assert_eq!(v.chosen_plate_of_number("E1"), "AB12 CDE");
        assert_eq!(v.chosen_plate_of_number("E2"), "B-V E2");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
