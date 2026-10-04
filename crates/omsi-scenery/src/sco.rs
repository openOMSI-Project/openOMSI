//! `.sco` scenery object definitions (unit `mc_complMapObj`).

use crate::PathDef;
use omsi_cfg::{CfgFile, Entry};
use omsi_model::Model;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderType {
    #[default]
    Normal,
    PreSurface,
    Surface,
    OnSurface,
    BeforeNormal,
    AfterNormal,
    AfterVehicles,
}

/// Parse the OMSI `[rendertype]` spelling used by both a .sco and its model.cfg.
pub fn parse_render_type(value: &str) -> RenderType {
    match value.trim().to_ascii_lowercase().as_str() {
        "presurface" => RenderType::PreSurface,
        "surface" => RenderType::Surface,
        "on_surface" => RenderType::OnSurface,
        "1" => RenderType::BeforeNormal,
        "3" => RenderType::AfterNormal,
        "4" => RenderType::AfterVehicles,
        _ => RenderType::Normal,
    }
}

/// One `[phase]` of a light: the value the lamp scripts read as `TrafficLightPhase`
/// (0..2 red, 3..5 red and yellow, 6..8 green, 9..11 yellow, 12 dark - see the stock
/// `ampel1.osc`) and how many seconds it lasts. A last phase of 0 s lasts until the
/// `[traffic_lights_group]` cycle starts again.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrafficLightPhase {
    pub state: i32,
    pub duration: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrafficLight {
    pub name: String,
    pub phases: Vec<TrafficLightPhase>,
    /// `[approachdist]` written after this light: how far before its stop line a vehicle
    /// registers a request (m).
    pub approach_dist: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SplineHelper {
    pub spline: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub heading: f32,
    pub length: f32,
    pub radius: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Attachment {
    pub ops: Vec<(String, Vec<f32>)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReflexionCamera {
    pub values: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MapLight {
    pub pos: [f32; 3],
    pub color: [f32; 3],
    pub radius: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TriggerBox {
    pub size: [f32; 3],
    pub center: [f32; 3],
    pub reverb: Option<(f32, f32)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScriptSet {
    pub varlists: Vec<PathBuf>,
    pub stringvarlists: Vec<PathBuf>,
    pub scripts: Vec<PathBuf>,
    pub constfiles: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SceneryObject {
    pub path: PathBuf,
    pub friendly_name: String,
    pub groups: Vec<String>,
    pub only_editor: bool,
    pub complexity: i32,
    pub render_type: RenderType,
    /// Whether `[rendertype]` appeared in this .sco, which takes precedence over model.cfg.
    pub render_type_explicit: bool,
    pub light_map_mapping: bool,
    pub no_map_lighting: bool,
    pub night_map_mode: i32,
    pub fixed: bool,
    pub abs_height: bool,
    pub collision_mesh: Option<String>,
    pub crossing_height_deformation: Option<String>,
    pub no_collision: bool,
    pub surface: bool,
    /// Whether this .sco supplied `[surface]`; otherwise the model.cfg value may be inherited.
    pub surface_explicit: bool,
    pub switch: Option<i32>,
    pub switch_dir: Vec<i32>,
    /// `[switchdir]` of each `[path]` (parallel to `paths`): the position of the points
    /// (the script's `Switch` variable) that sends a train along it.
    pub path_switch_dir: Vec<Option<i32>>,
    /// `[blockpath] n mode` after a `[path]` (OMSI keeps them with the path, the original):
    /// the other paths of the object that this one blocks while it is taken.
    pub path_blocks: Vec<Vec<(i32, i32)>>,
    /// `[crossingproblem]` after a `[path]`: a vehicle on it keeps the junction clear.
    pub path_crossing_problem: Vec<bool>,
    pub traffic_lights_group: Option<f32>,
    pub traffic_lights: Vec<TrafficLight>,
    pub approach_dist: Option<f32>,
    /// `[traffic_light_stop] light time if_request`: the cycle clock halts at `time` (see
    /// `omsi_sim::traffic::LightStop`).
    pub traffic_light_stop: Vec<[f32; 3]>,
    /// `[traffic_light_jump] light time if_request jump_to`.
    pub traffic_light_jump: Vec<[f32; 4]>,
    pub spline_helpers: Vec<SplineHelper>,
    pub paths: Vec<PathDef>,
    /// `[use_traffic_light]` index per path (parallel to `paths`, -1 when none).
    pub path_traffic_light: Vec<i32>,
    pub block_paths: Vec<(i32, i32)>,
    pub crossing_problem: bool,
    pub model_file: Option<String>,
    pub script_share: bool,
    pub scripts: ScriptSet,
    pub sound: Option<String>,
    pub sound_ai: Option<String>,
    pub paths_file: Option<String>,
    pub passenger_cabin: Option<String>,
    pub is_bus_stop: bool,
    pub is_entry_point: bool,
    pub is_car_park: bool,
    pub is_traffic_light: bool,
    pub is_signal: bool,
    pub is_help_arrow: bool,
    pub is_depot: bool,
    pub is_petrol_station: bool,
    pub tree: Option<(String, f32, f32, f32, f32)>,
    pub reflexion_cameras: Vec<ReflexionCamera>,
    pub mass: Option<f32>,
    pub moment_of_inertia: Option<[f32; 3]>,
    pub cog: Option<[f32; 3]>,
    pub bounding_box: Option<[f32; 6]>,
    pub crash_mode_pole: Option<(f32, f32)>,
    pub attachments: Vec<Attachment>,
    pub map_lights: Vec<MapLight>,
    pub rail_enh: Vec<[f32; 8]>,
    pub third_rail: Vec<[f32; 6]>,
    pub trigger_boxes: Vec<TriggerBox>,
    pub joinable: bool,
    pub model: Model,
    pub unknown_keywords: Vec<(String, usize)>,
}

fn read_list(r: &mut omsi_cfg::CfgReader, base: &Path) -> Vec<PathBuf> {
    let n = r.usize();
    (0..n).map(|_| omsi_cfg::resolve_path(base, r.str())).filter(|p| !p.as_os_str().is_empty()).collect()
}

/// One variable name per line, as the original reads a varlist and as the script
/// compiler does (`omsi_script::compile::read_list` + `declare_str_var`): trimmed, empty
/// lines passed over, and a name that comes again is the variable it already named - the
/// compiler hands out one slot per name (`X10 Objekte/Cooper/Modul1_stringvarlist.txt`
/// writes its eight names three times; Omsi.exe has eight variables for it, not 24).
/// A file that is not there gives no names - the object simply has none of them.
fn read_names(files: &[PathBuf]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for p in files {
        match omsi_cfg::CfgFile::read(p) {
            Ok(f) => {
                for l in f.lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
                    if seen.insert(l.to_ascii_lowercase()) {
                        out.push(l.to_string());
                    }
                }
            }
            Err(e) => log::debug!("{e}"),
        }
    }
    out
}

impl SceneryObject {
    /// The names of the object's string variables, in the order the map fills them: the
    /// lines of the files its `[stringvarnamelist]` names, one name each. The labels of a
    /// placed copy are those variables in this order - a sign's text, a bush's texture,
    /// a stop's name, a display's countdown - and they are what OMSI's Labels dialog
    /// edits. Read exactly as the script compiler reads them, so the n-th name is the
    /// variable the n-th label fills; an object without a stringvarlist carries values
    /// that have no name.
    pub fn string_var_names(&self) -> Vec<String> {
        read_names(&self.scripts.stringvarlists)
    }

    /// The names of the object's float variables: the lines of the files its
    /// `[varnamelist]` names. The map does not fill these - the object's own script sets
    /// them (a traffic light writes its `Red_light_duration` from the string the map gave
    /// it), so there is nothing for the map to fill in.
    pub fn var_names(&self) -> Vec<String> {
        read_names(&self.scripts.varlists)
    }

    /// Whether the map stores this object's height as it stands (not over the terrain):
    /// Omsi.exe sets that flag (+0x194) at the end of loading the .sco for `[absheight]` and
    /// for every object with a traffic path (0x7b8c66: the path ends `[path]` made, built by
    /// 0x7ba0d0) - crossings, but also a road piece or an invisible AI-path object of a mod
    /// that has no `[splinehelper]`. Put on the terrain as well, such a road stood the
    /// height of the hill above (or under) the ground.
    pub fn absolute_height(&self) -> bool {
        self.abs_height || !self.paths.is_empty()
    }

    /// Cutter filenames and their source folders. A separate model.cfg does not replace
    /// the object's own [terrainhole] declarations, which are not render-mesh overrides.
    pub fn terrain_hole_sources<'a>(&'a self, model: &'a Model) -> impl Iterator<Item = (&'a Path, &'a str)> {
        let model_dir = model.path.parent().unwrap_or_else(|| Path::new(""));
        let sco_dir = self.path.parent().unwrap_or_else(|| Path::new(""));
        model.terrain_hole_meshes().map(move |f| (model_dir, f)).chain(
            self.model_file.iter().flat_map(move |_| {
                self.model.terrain_hole_meshes().map(move |f| (sco_dir, f))
            }),
        )
    }

    pub fn load(path: &Path) -> Result<SceneryObject, omsi_cfg::CfgError> {
        let f = CfgFile::read(path)?;
        Ok(Self::parse(&f))
    }

    /// Inherit render and surface tags from a referenced model.cfg when the .sco wrapper
    /// did not author the corresponding tag. This mirrors OMSI's scenery-definition merge.
    pub fn inherit_model_tags(&mut self, model: &Model) {
        if !self.render_type_explicit {
            if let Some(value) = model.render_type.as_deref() {
                self.render_type = parse_render_type(value);
            }
        }
        if !self.surface_explicit {
            if let Some(surface) = model.surface {
                self.surface = surface;
                if surface {
                    self.fixed = true;
                }
            }
        }
    }

    pub fn parse(file: &CfgFile) -> SceneryObject {
        // Tram switches ship with an indented `[surface]`. The C++ handler trims
        // this tag and applies the same surface lift as to adjacent spline rails.
        // Normalize only this placement tag; indented mesh/animation commands must
        // retain their existing semantics, including disabled blocks.
        let normalized;
        let file = if file.lines.iter().any(|line| line != "[surface]" && line.trim() == "[surface]") {
            normalized = CfgFile {
                path: file.path.clone(),
                lines: file.lines.iter().map(|line| {
                    if line.trim() == "[surface]" { "[surface]".to_string() } else { line.clone() }
                }).collect(),
            };
            &normalized
        } else {
            file
        };
        let mut o = SceneryObject { path: file.path.clone(), complexity: 0, model: Model { path: file.path.clone(), detail_factor: 1.0, tex_detail_factor: 1.0, ..Default::default() }, ..Default::default() };
        let base = file.dir().to_path_buf();
        let mut r = file.reader().disabled_blocks();
        let tokens: Vec<&str> = ATTACH_TOKENS.iter().chain(omsi_model::ANIM_TOKENS).copied().collect();
        while let Some(e) = r.next_entry(&tokens) {
            let k = match e {
                Entry::Keyword(k) => k,
                Entry::Token(t) if ATTACH_TOKENS.contains(&t) => {
                    let values = if t == "attach_trans" { r.f32s::<3>().to_vec() } else { vec![r.f32()] };
                    if let Some(a) = o.attachments.last_mut() {
                        a.ops.push((t.to_string(), values));
                    }
                    continue;
                }
                Entry::Token(t) => {
                    o.model.handle_token(t, &mut r);
                    continue;
                }
            };
            match k.as_str() {
                "friendlyname" => o.friendly_name = r.str().to_string(),
                "groups" => {
                    let n = r.usize();
                    o.groups = (0..n).map(|_| r.str().to_string()).collect();
                }
                "onlyeditor" => o.only_editor = true,
                "complexity" => o.complexity = r.i32(),
                "rendertype" => {
                    o.render_type = parse_render_type(r.word());
                    o.render_type_explicit = true;
                }
                "lightmapmapping" => o.light_map_mapping = true,
                "nomaplighting" => o.no_map_lighting = true,
                "nightmapmode" => o.night_map_mode = r.i32(),
                "fixed" => o.fixed = true,
                "absheight" => o.abs_height = true,
                // OMSI knows only this spelling: the 72 stock buildings written
                // `[collisionmesh]` (the Staaken checkpoints on the Heerstraße among them)
                // have no collision shape in OMSI and are driven through
                "collision_mesh" => o.collision_mesh = Some(r.str().to_string()),
                "crossing_heightdeformation" => o.crossing_height_deformation = Some(r.str().to_string()),
                "nocollision" => o.no_collision = true,
                // (and `[fixed]` with it, as Omsi.exe sets both at 0x7b6823)
                "surface" => {
                    // A bare tag means true. Peek so an immediately following
                    // keyword (e.g. `[mesh]`) is not consumed as its optional value.
                    o.surface = r.clone().word() != "0";
                    o.surface_explicit = true;
                    o.fixed = true;
                }
                "switch" => o.switch = Some(r.i32()),
                "switchdir" => {
                    let d = r.i32();
                    o.switch_dir.push(d);
                    if let Some(last) = o.path_switch_dir.last_mut() {
                        *last = Some(d);
                    }
                }
                "traffic_lights_group" => o.traffic_lights_group = Some(r.f32()),
                "traffic_light" => o.traffic_lights.push(TrafficLight { name: r.str().to_string(), phases: Vec::new(), approach_dist: None }),
                "phase" => {
                    let state = r.i32();
                    let duration = r.f32();
                    if let Some(t) = o.traffic_lights.last_mut() {
                        t.phases.push(TrafficLightPhase { state, duration });
                    }
                }
                "approachdist" => {
                    // belongs to the light before it (a request button or a detector loop)
                    let d = r.f32();
                    o.approach_dist = Some(d);
                    if let Some(t) = o.traffic_lights.last_mut() {
                        t.approach_dist = Some(d);
                    }
                }
                "traffic_light_stop" => o.traffic_light_stop.push(r.f32s::<3>()),
                "traffic_light_jump" => o.traffic_light_jump.push(r.f32s::<4>()),
                "splinehelper" => {
                    let spline = r.str().to_string();
                    let v = r.f32s::<6>();
                    o.spline_helpers.push(SplineHelper { spline, x: v[0], y: v[1], z: v[2], heading: v[3], length: v[4], radius: v[5] });
                }
                "path" => {
                    let v = r.f32s::<12>();
                    o.paths.push(PathDef { kind: v[8] as i32, start: [v[0], v[1], v[2]], end: [v[3], v[4], v[5]], width: v[9], direction: v[10] as i32, params: v.to_vec() });
                    o.path_traffic_light.push(-1);
                    o.path_switch_dir.push(None);
                    o.path_blocks.push(Vec::new());
                    o.path_crossing_problem.push(false);
                }
                "path_2" => {
                    let v = r.f32s::<14>();
                    o.paths.push(PathDef { kind: v[8] as i32, start: [v[0], v[1], v[2]], end: [v[3], v[4], v[5]], width: v[9], direction: v[10] as i32, params: v.to_vec() });
                    o.path_traffic_light.push(-1);
                    o.path_switch_dir.push(None);
                    o.path_blocks.push(Vec::new());
                    o.path_crossing_problem.push(false);
                }
                "use_traffic_light" => {
                    let i = r.i32();
                    if let Some(last) = o.path_traffic_light.last_mut() {
                        *last = i;
                    }
                }
                "blockpath" => {
                    let a = r.i32();
                    let b = r.i32();
                    o.block_paths.push((a, b));
                    if let Some(last) = o.path_blocks.last_mut() {
                        last.push((a, b));
                    }
                }
                "crossingproblem" => {
                    o.crossing_problem = true;
                    if let Some(last) = o.path_crossing_problem.last_mut() {
                        *last = true;
                    }
                }
                "model" => o.model_file = Some(r.str().to_string()),
                "scriptshare" => o.script_share = true,
                "varnamelist" => o.scripts.varlists.extend(read_list(&mut r, &base)),
                "stringvarnamelist" => o.scripts.stringvarlists.extend(read_list(&mut r, &base)),
                "script" => o.scripts.scripts.extend(read_list(&mut r, &base)),
                "constfile" => o.scripts.constfiles.extend(read_list(&mut r, &base)),
                "sound" => o.sound = Some(r.str().to_string()),
                "sound_ai" => o.sound_ai = Some(r.str().to_string()),
                "paths" => o.paths_file = Some(r.str().to_string()),
                "passengercabin" => o.passenger_cabin = Some(r.str().to_string()),
                "busstop" => o.is_bus_stop = true,
                "entrypoint" => o.is_entry_point = true,
                "carpark_p" => o.is_car_park = true,
                "trafficlight" => o.is_traffic_light = true,
                "signal" => o.is_signal = true,
                "helparrow" => o.is_help_arrow = true,
                "depot" => o.is_depot = true,
                "petrolstation" => o.is_petrol_station = true,
                "tree" => {
                    let t = r.str().to_string();
                    let v = r.f32s::<4>();
                    o.tree = Some((t, v[0], v[1], v[2], v[3]));
                }
                "add_camera_reflexion" => o.reflexion_cameras.push(ReflexionCamera { values: r.f32s::<7>().to_vec() }),
                "add_camera_reflexion_2" => o.reflexion_cameras.push(ReflexionCamera { values: r.f32s::<8>().to_vec() }),
                "mass" => o.mass = Some(r.f32()),
                "momentofintertia" => o.moment_of_inertia = Some(r.f32s::<3>()),
                "cog" => o.cog = Some(r.f32s::<3>()),
                "boundingbox" => {
                    // (the sizes as magnitudes, as a vehicle's: a box given negative
                    // crossed the bounds of the walkers' clamp about it, #986)
                    let mut bb = r.f32s::<6>();
                    for x in &mut bb[..3] {
                        *x = x.abs();
                    }
                    o.bounding_box = Some(bb);
                }
                "crashmode_pole" => {
                    let a = r.f32();
                    let b = r.f32();
                    o.crash_mode_pole = Some((a, b));
                }
                "new_attachment" => o.attachments.push(Attachment::default()),
                "maplight" => {
                    let v = r.f32s::<7>();
                    o.map_lights.push(MapLight { pos: [v[0], v[1], v[2]], color: [v[3], v[4], v[5]], radius: v[6] });
                }
                "rail_enh" => o.rail_enh.push(r.f32s::<8>()),
                "third_rail" => o.third_rail.push(r.f32s::<6>()),
                "triggerbox_new" => {
                    let v = r.f32s::<6>();
                    o.trigger_boxes.push(TriggerBox { size: [v[0], v[1], v[2]], center: [v[3], v[4], v[5]], reverb: None });
                }
                "triggerbox_setreverb" => {
                    let a = r.f32();
                    let b = r.f32();
                    if let Some(t) = o.trigger_boxes.last_mut() {
                        t.reverb = Some((a, b));
                    }
                }
                "joinable" => o.joinable = true,
                _ => {
                    if !o.model.handle_keyword(&k, &mut r) {
                        o.unknown_keywords.push((k, r.block_line()));
                    }
                }
            }
        }
        o
    }
}

/// The `[new_attachment]` sub-commands (`attach_trans x y z`, `attach_rot_x a` …). The
/// original compares them with whole lines like keywords and applies them to the last
/// attachment wherever they stand after it: the stock `Timetable_Terminus_Pole_S.sco` lifts
/// its point to 2.68 m with an `attach_trans` after its `[complexity]` block (read as part of
/// the `[new_attachment]` block only, the timetable hung at the foot of the pole).
pub const ATTACH_TOKENS: &[&str] = &["attach_trans", "attach_rot_x", "attach_rot_y", "attach_rot_z"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_hole_sources_keep_inline_and_referenced_declarations() {
        let inline = SceneryObject::parse(&CfgFile::from_str(
            "objects/cutting.sco",
            "[terrainhole]\ncut.o3d\n[mesh]\nvisible.o3d\n[terrainhole]\nend.o3d\n",
        ));
        assert_eq!(inline.terrain_hole_sources(&inline.model).collect::<Vec<_>>(), [
            (Path::new("objects"), "cut.o3d"),
            (Path::new("objects"), "end.o3d"),
        ]);
        let wrapper = SceneryObject::parse(&CfgFile::from_str(
            "objects/cutting.sco",
            "[terrainhole]\ncut.o3d\n[model]\nmodel/model.cfg\n",
        ));
        let model = Model::parse(&CfgFile::from_str(
            "objects/model/model.cfg",
            "[terrainhole]\nend.o3d\n[mesh]\nvisible.o3d\n",
        ));
        assert_eq!(wrapper.terrain_hole_sources(&model).collect::<Vec<_>>(), [
            (Path::new("objects/model"), "end.o3d"),
            (Path::new("objects"), "cut.o3d"),
        ]);
    }

    /// The varlists of an object name the fields its placed copies carry in the map: the
    /// labels of a placed object are its string variables, one value per name.
    #[test]
    fn the_varlists_name_the_fields_of_a_placed_object() {
        let dir = std::env::temp_dir().join(format!("omsi_sco_names_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sco = dir.join("stop.sco");
        std::fs::write(dir.join("strings.txt"), "BusStop\r\nTexture\r\n").unwrap();
        std::fs::write(dir.join("vars.txt"), "Delay\r\n\r\n").unwrap();
        let o = SceneryObject::parse(&CfgFile::from_str(
            &*sco.to_string_lossy(),
            "[stringvarnamelist]\n1\nstrings.txt\n[varnamelist]\n1\nvars.txt\n",
        ));
        assert_eq!(o.string_var_names(), vec!["BusStop".to_string(), "Texture".to_string()]);
        assert_eq!(o.var_names(), vec!["Delay".to_string()]);
        // an object without a varlist: its labels are values without a name
        let bare = SceneryObject::parse(&CfgFile::from_str(&*dir.join("tree.sco").to_string_lossy(), "[fixed]\n"));
        assert!(bare.string_var_names().is_empty() && bare.var_names().is_empty());
        // a name written again names the variable it already named: the compiler gives it
        // one slot, so the names must not outnumber the values the map writes (a real
        // object writes its eight names three times - `X10 Objekte/Cooper`)
        std::fs::write(dir.join("thrice.txt"), "Ersatzverkehr\r\nHaltstellennamen\r\nErsatzverkehr\r\nHaltstellennamen\r\n").unwrap();
        let again = SceneryObject::parse(&CfgFile::from_str(
            &*sco.to_string_lossy(),
            "[stringvarnamelist]\n1\nthrice.txt\n",
        ));
        assert_eq!(again.string_var_names(), vec!["Ersatzverkehr".to_string(), "Haltstellennamen".to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The stock timetable pole: the `attach_trans` after `[complexity]` belongs to the
    /// attachment; an indented or differently spelled one does not.
    #[test]
    fn attachment_sub_commands() {
        let text = "[new_attachment]\n\n\nComplexity:\n0 - very important object\n\n[complexity]\n2\n\nattach_trans\n0.042\n0\n2.68\n\n[mesh]\npole.o3d\n\n[new_attachment]\n\tattach_rot_z\n\t90\nAttach_rot_x\n45\nattach_rot_x\n-10\n";
        let o = SceneryObject::parse(&CfgFile::from_str("Timetable_Terminus_Pole_S.sco", text));
        assert_eq!(o.complexity, 2);
        assert_eq!(o.model.meshes.len(), 1);
        assert_eq!(o.attachments.len(), 2);
        assert_eq!(o.attachments[0].ops, vec![("attach_trans".to_string(), vec![0.042, 0.0, 2.68])]);
        assert_eq!(o.attachments[1].ops, vec![("attach_rot_x".to_string(), vec![-10.0])]);
    }

    #[test]
    fn animations_and_disabled_blocks_in_a_scenery_object() {
        let text = "[mesh]\nbarrier.o3d\n[newanim]\norigin_rot_y\n90\n[mouseevent]\nopen\nanim_rot\nbarrier\n85\n-<DISABLED>-\n[mesh]\nold.o3d\n-<ENABLED>-\n[fixed]\n";
        let o = SceneryObject::parse(&CfgFile::from_str("barrier.sco", text));
        assert_eq!(o.model.meshes.len(), 1);
        let a = &o.model.meshes[0].animations[0];
        assert_eq!(a.origins, vec![omsi_model::AnimOrigin::RotY(90.0)]);
        assert_eq!((a.variable.as_str(), a.factor), ("barrier", 85.0));
        assert!(o.fixed);
    }

    #[test]
    fn render_type_names_and_numeric_queues_match_omsi() {
        for (value, expected) in [
            ("presurface", RenderType::PreSurface),
            ("surface", RenderType::Surface),
            ("on_surface", RenderType::OnSurface),
            ("1", RenderType::BeforeNormal),
            ("3", RenderType::AfterNormal),
            ("4", RenderType::AfterVehicles),
            ("0", RenderType::Normal),
        ] {
            let text = format!("[rendertype]\n{value}\n");
            assert_eq!(SceneryObject::parse(&CfgFile::from_str("phase.sco", &text)).render_type, expected);
        }
    }

    #[test]
    fn referenced_model_cfg_render_tags_are_inherited_unless_the_sco_overrides_them() {
        let model = Model::parse(&CfgFile::from_str(
            "model.cfg",
            "[rendertype]\nsurface\n[surface]\n1\n",
        ));
        let mut inherited = SceneryObject::parse(&CfgFile::from_str("junction.sco", ""));
        inherited.inherit_model_tags(&model);
        assert_eq!(inherited.render_type, RenderType::Surface);
        assert!(inherited.surface);
        assert!(inherited.fixed);

        let mut explicit = SceneryObject::parse(&CfgFile::from_str(
            "junction.sco",
            "[rendertype]\n0\n[surface]\n0\n",
        ));
        explicit.inherit_model_tags(&model);
        assert_eq!(explicit.render_type, RenderType::Normal);
        assert!(!explicit.surface);
    }

    #[test]
    fn an_object_with_a_path_keeps_the_height_the_map_gives_it() {
        let path = "[path]\n0\n0\n0\n0\n0\n10\n0\n0\n0\n3.5\n0\n0\n";
        let road = SceneryObject::parse(&CfgFile::from_str("road.sco", &format!("[mesh]\nroad.o3d\n{path}")));
        assert!(!road.abs_height && road.spline_helpers.is_empty());
        assert!(road.absolute_height(), "a road piece without [splinehelper]");
        let house = SceneryObject::parse(&CfgFile::from_str("house.sco", "[mesh]\nhouse.o3d\n"));
        assert!(!house.absolute_height());
        let lifted = SceneryObject::parse(&CfgFile::from_str("bridge.sco", "[absheight]\n[mesh]\nb.o3d\n"));
        assert!(lifted.absolute_height());
    }

    #[test]
    fn indented_switch_surface_tag_preserves_the_surface_lift() {
        let text = "[rendertype]\nsurface\n[absheight]\n\n\t[surface]\n[mesh]\nWeiche_L_Asphalt.o3d\n\t[mesh]\nignored.o3d\n";
        let o = SceneryObject::parse(&CfgFile::from_str("switch.sco", text));
        assert!(o.surface && o.surface_explicit && o.fixed && o.abs_height);
        assert_eq!(o.render_type, RenderType::Surface);
        assert_eq!(o.model.meshes.len(), 1);
        assert_eq!(o.model.meshes[0].file, "Weiche_L_Asphalt.o3d");

        let o = SceneryObject::parse(&CfgFile::from_str("switch.sco", " \t[surface] \t\n0\n[mesh]\nswitch.o3d\n"));
        assert!(o.surface_explicit);
        assert!(!o.surface);
        assert_eq!(o.model.meshes.len(), 1);

        let o = SceneryObject::parse(&CfgFile::from_str("switch.sco", "-<DISABLED>-\n\t[surface]\n1\n-<ENABLED>-\n[mesh]\nswitch.o3d\n"));
        assert!(!o.surface_explicit && !o.surface);
        assert_eq!(o.model.meshes.len(), 1);
    }
}
