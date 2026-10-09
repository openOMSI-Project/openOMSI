//! Scripted / animated scenery objects (unit `mc_complMapObj` runtime part): each placed
//! object with a script or animations owns a script state, runs `{frame}` and drives its
//! mesh animations and `[visible]` flags.

use crate::anim::MeshAnimator;
use crate::host::VehicleHost;
use glam::Mat4;
use omsi_model::MeshDef;
use omsi_script::{compile, CompileInput, Program, State, Vm};
use omsi_scenery::sco::ScriptSet;
use std::path::Path;
use std::sync::Arc;

/// Builtin variables of scenery objects (`program/varlist_scenobj.txt`).
pub fn builtin_scenobj_vars(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = match omsi_cfg::CfgFile::read(root.join("program/varlist_scenobj.txt")) {
        Ok(f) => f.lines.iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
        Err(_) => vec!["NightlightA", "InUse", "TrafficLightPhase", "TrafficLightApproach", "Colorscheme", "Signal", "NextSignal", "Refresh_Strings", "Switch"].into_iter().map(String::from).collect(),
    };
    for n in ["NightlightA", "TrafficLightPhase", "Switch"] {
        if !v.iter().any(|x| x.eq_ignore_ascii_case(n)) {
            v.push(n.to_string());
        }
    }
    v
}

#[cfg(test)]
mod placement_tests {
    use super::*;

    #[test]
    fn busstop_frame_resolves_each_placements_texture() {
        let dir = std::env::temp_dir().join(format!("omsi_busstop_freetex_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let vars = dir.join("strings.txt");
        let script = dir.join("BusStop.osc");
        std::fs::write(&vars, "BusStop\nTexture\n").unwrap();
        std::fs::write(&script, concat!(
            "{init}\n{end}\n{frame}\n",
            "(L.$.BusStop) \"\" $= !\n{if}\n",
            "\"Busstop\\\" $+ (L.$.BusStop) $+ \".png\" $+ (S.$.Texture)\n",
            "{endif}\n{end}\n",
        )).unwrap();
        let program = Arc::new(compile(&CompileInput {
            stringvarlists: vec![vars], scripts: vec![script], ..Default::default()
        }));
        assert!(program.errors.is_empty(), "{:?}", program.errors);
        for name in ["BentenDaini_1", "BentenDaini_2", ""] {
            let mut inst = SceneryInstance::new(
                program.clone(), &[], crate::SimClock::default(), &[name.to_string()],
            );
            inst.update(0.0, &SceneryVars::default());
            let expected = if name.is_empty() { String::new() } else { format!("Busstop\\{name}.png") };
            assert_eq!(inst.str_var("Texture"), expected);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// Compile the scripts of a scenery object type (an empty program with the builtin
/// variables when it has none, so animations can still bind to `Switch` & co).
pub fn compile_scenery(root: &Path, scripts: &ScriptSet) -> Program {
    let mut input = CompileInput { builtin_vars: builtin_scenobj_vars(root), ..Default::default() };
    input.varlists = scripts.varlists.clone();
    input.stringvarlists = scripts.stringvarlists.clone();
    input.constfiles = scripts.constfiles.clone();
    input.scripts = scripts.scripts.clone();
    let p = compile(&input);
    for e in &p.errors {
        log::warn!("{e}");
    }
    p
}

/// Engine-provided values written into the builtin variables every frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct SceneryVars {
    pub nightlight: f32,
    /// `InUse`: the building is in use (its `[NightMapMode]` hours; 1 without them).
    pub in_use: f32,
    pub traffic_light_phase: f32,
    pub traffic_light_approach: f32,
    pub switch: Option<f32>,
}

pub struct SceneryInstance {
    pub program: Arc<Program>,
    pub state: State,
    pub vm: Vm,
    pub host: VehicleHost,
    animators: Vec<MeshAnimator>,
    /// `[visible] var value` per mesh (no variable: one the object does not declare, which
    /// reads 0 - Omsi.exe registers it as it reads the model).
    visible_conds: Vec<Option<(Option<omsi_script::VarId>, f32)>>,
    pub mesh_transforms: Vec<Mat4>,
    pub mesh_visible: Vec<bool>,
    v_in_use: Option<omsi_script::VarId>,
    v_night: Option<omsi_script::VarId>,
    v_phase: Option<omsi_script::VarId>,
    v_approach: Option<omsi_script::VarId>,
    v_switch: Option<omsi_script::VarId>,
    v_refresh: Option<omsi_script::VarId>,
    pub html_textures: Vec<crate::htmltex::HtmlTexture>,
}

impl SceneryInstance {
    /// `meshes`: (mesh definition, pivot) per rendered mesh; `strings`: the placed object's
    /// strings from the map, which are its string variables in order, before the `{init}`
    /// runs (Omsi.exe sub_7eea70 copies them into the object's string variables when it
    /// is placed - a sign's label naming its picture, a display's stop).
    pub fn new(program: Arc<Program>, meshes: &[(&MeshDef, Mat4)], clock: crate::SimClock, strings: &[String]) -> SceneryInstance {
        let mut state = State::new(&program);
        for (v, s) in state.str_vars.iter_mut().zip(strings) {
            v.clone_from(s);
        }
        let mut vm = Vm::new();
        let mut host = VehicleHost::new(clock);
        // `Colorscheme`: the object's paint scheme, −1 for its own textures (OMSI
        // the original draws one only for objects placed at random; placed ones show theirs)
        if let Some(id) = program.var("Colorscheme") {
            state.vars[id as usize] = -1.0;
        }
        vm.run_init(&program, &mut state, &mut host);
        let mut animators: Vec<MeshAnimator> = meshes.iter().map(|(d, pivot)| MeshAnimator::new(d, *pivot, |n| program.var(n))).collect();
        crate::anim::link_parents(&mut animators, &meshes.iter().map(|(d, _)| *d).collect::<Vec<_>>());
        let visible_conds = meshes.iter().map(|(d, _)| d.visible.as_ref().map(|(v, x)| (program.var(v), *x))).collect();
        let n = meshes.len();
        SceneryInstance {
            v_night: program.var("NightlightA"),
            v_in_use: program.var("InUse"),
            v_phase: program.var("TrafficLightPhase"),
            v_approach: program.var("TrafficLightApproach"),
            v_switch: program.var("Switch"),
            v_refresh: program.var("Refresh_Strings"),
            program,
            state,
            vm,
            host,
            animators,
            visible_conds,
            mesh_transforms: vec![Mat4::IDENTITY; n],
            mesh_visible: vec![true; n],
            html_textures: Vec::new(),
        }
    }

    /// Does the script move any of the meshes (a crossing's barrier arm)?
    pub fn animated(&self) -> bool {
        self.animators.iter().any(|a| a.has_animations())
    }

    /// Does the object need per-frame updates at all?
    pub fn is_dynamic(&self) -> bool {
        !self.program.frame.is_empty() || self.animators.iter().any(|a| a.has_animations()) || self.visible_conds.iter().any(|c| c.is_some())
    }

    fn put(&mut self, id: Option<omsi_script::VarId>, v: f32) {
        if let Some(i) = id {
            self.state.vars[i as usize] = v;
        }
    }

    pub fn trigger(&mut self, name: &str) -> bool {
        let p = self.program.clone();
        self.vm.run_trigger(&p, name, &mut self.state, &mut self.host)
    }

    pub fn update(&mut self, dt: f32, vars: &SceneryVars) {
        self.host.clock.advance(dt);
        self.put(self.v_night, vars.nightlight);
        self.put(self.v_in_use, vars.in_use);
        self.put(self.v_phase, vars.traffic_light_phase);
        self.put(self.v_approach, vars.traffic_light_approach);
        if let Some(s) = vars.switch {
            self.put(self.v_switch, s);
        }
        let p = self.program.clone();
        self.vm.run_frame(&p, &mut self.state, &mut self.host);
        for (i, a) in self.animators.iter_mut().enumerate() {
            self.mesh_transforms[i] = a.update(dt, &self.state.vars);
        }
        crate::anim::apply_parents(&self.animators, &mut self.mesh_transforms);
        for (i, c) in self.visible_conds.iter().enumerate() {
            self.mesh_visible[i] = match c {
                Some((id, x)) => (id.map(|id| self.state.vars[id as usize]).unwrap_or(0.0) - x).abs() < 0.5,
                None => true,
            };
        }
    }

    pub fn var(&self, name: &str) -> Option<f32> {
        self.program.var(name).map(|i| self.state.vars[i as usize])
    }

    /// Set a variable of the script; false when it has none of that name.
    pub fn set_var(&mut self, name: &str, v: f32) -> bool {
        match self.program.var(name) {
            Some(i) => {
                self.state.vars[i as usize] = v;
                true
            }
            None => false,
        }
    }

    /// A string variable of the script (empty when it has none of that name).
    pub fn str_var(&self, name: &str) -> &str {
        self.program.str_var(name).and_then(|i| self.state.str_vars.get(i as usize)).map(|s| s.as_str()).unwrap_or("")
    }

    /// `Refresh_Strings`: the script asks for its text textures to be drawn again from its
    /// string variables; like OMSI the flag is cleared once that is done (the stock bus
    /// stop display sets it every frame).
    pub fn take_refresh_strings(&mut self) -> bool {
        match self.v_refresh {
            Some(i) => std::mem::replace(&mut self.state.vars[i as usize], 0.0) != 0.0,
            None => false,
        }
    }

    /// Start the `[htmltexture]` pages of the object's model. `model_dir` is the folder of
    /// the model config, `object_dir` the folder of the `.sco`.
    pub fn init_html_textures(&mut self, defs: &[omsi_model::HtmlTextureDef], model_dir: &Path, object_dir: &Path) {
        self.html_textures = crate::htmltex::scenery_pages(defs, model_dir, object_dir);
    }

    /// Give the pages the object's variables and time, apply what they did (variables,
    /// triggers) and return their new pictures: (script texture index, width, height, RGBA).
    pub fn update_html_textures(&mut self) -> Vec<(usize, u32, u32, Vec<u8>)> {
        if self.html_textures.is_empty() {
            return Vec::new();
        }
        let num: Vec<(String, f32)> = self.program.var_names.iter().enumerate().map(|(i, n)| (n.clone(), self.state.vars[i])).collect();
        let strs: Vec<(String, String)> = self.program.str_var_names.iter().enumerate().map(|(i, n)| (n.clone(), self.state.str_vars[i].clone())).collect();
        // (the basic API only: no vehicle, no depot)
        let env = crate::vehicle_api::environment(&self.host.clock, &crate::vehicle_api::locale());
        let departures = (!self.host.html_departures.is_empty()).then(|| crate::vehicle_api::departures(&self.host.html_departures));
        let out = crate::htmltex::drive_pages(&mut self.html_textures, &num, &strs, None, &env, None, departures.as_ref());
        for key in out.departure_wants {
            self.host.want_departures(key);
        }
        self.apply_page_output(out.events, out.triggers);
        out.frames
    }

    /// A press, release or move on page `script_index` (`u`/`v` 0..1 across it, `v` down
    /// from the top). False when the object has no such page.
    pub fn html_pointer(&mut self, script_index: usize, u: f32, v: f32, kind: crate::htmltex::PointerKind) -> bool {
        match crate::htmltex::pointer_on(&mut self.html_textures, script_index, u, v, kind) {
            Some((events, triggers)) => {
                self.apply_page_output(events, triggers);
                true
            }
            None => false,
        }
    }

    fn apply_page_output(&mut self, events: Vec<(String, f32)>, triggers: Vec<String>) {
        for (name, value) in events {
            if !self.set_var(&name, value) {
                log::debug!("htmltexture: the page sets {name}, which the object does not have");
            }
        }
        for name in triggers {
            if !self.trigger(&name) {
                log::debug!("htmltexture: the page presses {name}, which the object does not have");
            }
        }
    }

    /// Whether the script asks for the buses due at its stop (`GetArrBus*`).
    pub fn wants_arrivals(&self) -> bool {
        self.program.names.iter().any(|n| n.get(..9).map(|p| p.eq_ignore_ascii_case("getarrbus")).unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The map's strings of a placed object are its string variables, in order, before its
    /// {init} runs; a script without that many string variables takes the first ones.
    #[test]
    fn the_map_strings_are_the_string_variables() {
        let mut p = Program::default();
        p.declare_str_var("A");
        p.declare_str_var("B");
        let inst = SceneryInstance::new(Arc::new(p), &[], crate::SimClock::default(), &["bss1\\14.jpg".into(), "x".into(), "ignored".into()]);
        assert_eq!(inst.str_var("A"), "bss1\\14.jpg");
        assert_eq!(inst.str_var("B"), "x");
    }
}
