//! Runtime for a sound configuration attached to an object with script variables.
//!
//! Everything here follows Omsi.exe's `TSound` update (0x750340, 2.2.032), which plays each
//! entry through its own DirectSound buffer:
//!
//! * volume = the entry's volume x every `[volcurve]` (linear between `[pnt]`s, held at the
//!   ends), x `min(range / distance, 1)` for a `[3d]` sound, x `0.2 + Snd_OutsideVol` for an
//!   AI vehicle while the camera is in a cab, x `Snd_OutsideVol` for the own bus's outside
//!   sounds heard in the cab - as a linear amplitude, turned into hundredths of a dB
//!   (`2000 * log10`) for `SetVolume`: over 0 dB or under -100 dB the call is refused and the
//!   buffer keeps the volume it had, at -100 dB or below the sound is not started;
//! * pitch = a `[loopsound]`'s `rate * |variable| / reference` for `SetFrequency` (refused
//!   outside DirectSound's 100 to 200 000 Hz; not started below 100 Hz); a `[sound]` plays
//!   at its file's rate;
//! * a `[3d]` sound is panned by at most `50 * [sound_stereo]` hundredths of a dB (see
//!   `mixer::pan_gains`), nothing else is spatial: a sound without `[3d]` is heard at the
//!   same level wherever the listener is - on an AI vehicle too.

use crate::mixer::{AudioEngine, Clip, VoiceId, VoiceParams};
use glam::{Mat4, Vec3};
use omsi_vehicle::{SoundCfg, SoundEntry};
use std::path::Path;
use std::sync::Arc;

struct RuntimeSound {
    def: SoundEntry,
    clip: Option<Arc<Clip>>,
    voice: Option<VoiceId>,
    /// The conditions held last frame (a `[noloop]` entry without a trigger plays once
    /// when they start to hold).
    held: bool,
    /// Since when the conditions hold - what a `[volcurve] -1` reads, see
    /// [`SoundSet::curve_input`].
    active_since: Option<std::time::Instant>,
    /// The buffer's volume (linear) as DirectSound last took it: a `SetVolume` over 0 dB
    /// or under -100 dB is refused and the buffer keeps this one. A new buffer is at 0 dB.
    last_gain: f32,
    /// The buffer's playback rate relative to the clip, as last taken by `SetFrequency`.
    last_pitch: f32,
}

impl RuntimeSound {
    fn new(def: SoundEntry, clip: Option<Arc<Clip>>) -> RuntimeSound {
        RuntimeSound { def, clip, voice: None, held: false, active_since: None, last_gain: 1.0, last_pitch: 1.0 }
    }
}

pub struct SoundSet {
    sounds: Vec<RuntimeSound>,
    /// The `[sound_ai]`/`[sound_scenery]` share of this set (the settings' sliders).
    pub master: f32,
    /// Folder of the sound config: files of `(T.F.)` triggers resolve against it.
    dir: std::path::PathBuf,
    /// The listener sits in this vehicle's interior (see [`SoundSet::set_inside`]).
    inside: bool,
    /// This set belongs to an AI vehicle or another player's (Omsi.exe's view 4).
    ai: bool,
    /// The player's own vehicle moves with its listener; its 3D sounds still pan and fade,
    /// but frame timing must not turn their fixed cabin positions into Doppler pitch shifts.
    listener_vehicle: bool,
    /// The camera is in a cab view (driver or passenger: Omsi.exe's camera modes 0 and 1),
    /// set every frame on every sound set (see [`SoundSet::set_muffled`]).
    muffled: bool,
    /// The sound sets of the coupled parts (with the part's index among the vehicle's
    /// trailers): the rear section of an articulated bus has a `[sound]` of its own - on a
    /// pusher like the MB C2 G that is where the engine is - and plays it on the triggers
    /// and variables of the scripts it shares with the front (see [`SoundSet::update_parts`]).
    pub parts: Vec<(usize, SoundSet)>,
}

/// The file a `(T.F.)` trigger names, as Omsi.exe opens it (0x74f2e8 through 0x7eed78): the
/// characters a Windows file name cannot have are `%` in it, the folders' `\` and a drive's
/// `:` stay. The stock IBIS announces a stop by its name, and Spandau's "Falkenseer
/// Ch/Stadtrandstr" is `Falkenseer Ch%Stadtrandstr.wav`: read as a folder, it was not found
/// and the stop was not announced (#1096).
fn file_trigger_name(file: &str) -> String {
    file.chars().map(|c| if matches!(c, '/' | '?' | '*' | '"' | '<' | '>' | '|') { '%' } else { c }).collect()
}

/// Say once per file that a `(T.F.)` sound cannot be found: every AI bus of a type asks for
/// the same missing announcement at every stop.
fn warn_missing_once(trigger: &str, path: &Path) {
    static SEEN: std::sync::Mutex<Option<std::collections::HashSet<std::path::PathBuf>>> =
        std::sync::Mutex::new(None);
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen
        .get_or_insert_with(Default::default)
        .insert(path.to_path_buf())
    {
        log::warn!(
            "sound {} for (T.F.{trigger}) not found (further requests for it are not logged)",
            path.display()
        );
    }
}

/// A `[volcurve]` at `x`, as Omsi.exe's `TFuncClass` (0x7f061c) reads it: the first point's
/// value left of it, the last one's from the last point on, and in between the straight
/// line between the last point at or left of `x` and the first one right of it (0x7f07b4;
/// two points at the same place give their mean).
fn curve(points: &[(f32, f32)], x: f32) -> f32 {
    let Some(&(x0, y0)) = points.first() else {
        return 1.0;
    };
    if x < x0 {
        return y0;
    }
    let last = points[points.len() - 1];
    if x >= last.0 {
        return last.1;
    }
    let i = (1..points.len()).find(|&i| x < points[i].0).unwrap_or(points.len() - 1);
    let (xa, ya) = points[i - 1];
    let (xb, yb) = points[i];
    if xb == xa {
        (ya + yb) / 2.0
    } else {
        (x - xa) * ((yb - ya) / (xb - xa)) + ya
    }
}

/// What DirectSound makes of a linear volume (`SetVolume(Round(2000 * log10 v))`, the
/// buffer at `last`): the volume it plays at, and whether it is loud enough to be started
/// (over -100 dB). A request over 0 dB or under -100 dB is refused - the buffer keeps
/// `last`; the MB 412D's `[sound] start2.wav`, which carries a loop sound's lines and reads
/// "44100" as its volume, plays at full volume, not 44 100 times too loud.
fn direct_sound_volume(v: f32, last: f32) -> (f32, bool) {
    if !(v > 1.0e-10) {
        // (-10000 is a valid request: silence)
        return (0.0, false);
    }
    let hundredths = (2000.0 * v.log10()).round();
    if hundredths > 0.0 {
        (last, true)
    } else if hundredths < -10000.0 {
        (last, false)
    } else {
        (10f32.powf(hundredths / 2000.0), hundredths > -10000.0)
    }
}

/// DirectSound's frequency range (`DSBFREQUENCY_MIN`/`MAX`).
const FREQ_MIN: f32 = 100.0;
const FREQ_MAX: f32 = 200_000.0;

/// What a `[volcurve] -1` reads on a triggered entry: Omsi.exe never starts its clock (it
/// is reset only while the conditions of an untriggered entry fail), so it reads
/// `GetTickCount / 1000`, the seconds since Windows started - always past the last point.
const TRIGGERED_ACTIVE: f32 = 1.0e6;

/// One entry's state this frame, as the exe computes it.
struct Eval {
    /// What the voice is given (the distance attenuation is left to the mixer).
    gain: f32,
    audible: bool,
    pitch: f32,
    /// `[viewpoint]` (or the global stop) silences it: a playing voice is stopped.
    wrong_view: bool,
}

impl SoundSet {
    /// The files of a sound config's fixed clips (`dir` as for [`SoundSet::new`]), for
    /// [`AudioEngine::clips_ready`].
    pub fn clip_paths(cfg: &SoundCfg, dir: &Path) -> Vec<std::path::PathBuf> {
        cfg.sounds
            .iter()
            .filter(|def| def.file.trim().parse::<i32>().is_err())
            .map(|def| omsi_cfg::resolve_path(dir, &def.file))
            .collect()
    }

    /// Load the clips of a sound config; `dir` is the directory of the config file.
    pub fn new(engine: &AudioEngine, cfg: &SoundCfg, dir: &Path) -> SoundSet {
        let sounds = cfg
            .sounds
            .iter()
            .map(|def| {
                // `[sound] N`: no fixed file, the script names one with `(T.F.trigger)`
                let dynamic = def.file.trim().parse::<i32>().is_ok();
                let path = omsi_cfg::resolve_path(dir, &def.file);
                let clip = if engine.enabled && !dynamic {
                    engine.load_clip(&path)
                } else {
                    None
                };
                RuntimeSound::new(def.clone(), clip)
            })
            .collect();
        SoundSet {
            sounds,
            master: 1.0,
            dir: dir.to_path_buf(),
            inside: false,
            ai: false,
            listener_vehicle: false,
            muffled: false,
            parts: Vec::new(),
        }
    }

    /// Where the listener is, for the `[viewpoint]` of an entry: `true` while the camera is
    /// one of this vehicle's interior views. The player's bus is told every frame; a scenery
    /// object or another vehicle is always listened to from outside.
    pub fn set_inside(&mut self, inside: bool) {
        self.inside = inside;
        for (_, p) in &mut self.parts {
            p.set_inside(inside);
        }
    }

    /// Track whether the listener travels with the player's vehicle and its coupled parts.
    pub fn set_listener_vehicle(&mut self, follows: bool) {
        self.listener_vehicle = follows;
        for (_, p) in &mut self.parts {
            p.set_listener_vehicle(follows);
        }
    }

    /// The camera is in a cab view (driver or passenger, of any vehicle): called every frame
    /// on every sound set. An AI vehicle is then heard at `0.2 + Snd_OutsideVol` of the
    /// player's bus (`TSound` update 0x750a4d) - no filter, no other change.
    pub fn set_muffled(&mut self, muffled: bool) {
        self.muffled = muffled;
        for (_, p) in &mut self.parts {
            p.set_muffled(muffled);
        }
    }

    /// Attach the sound set of coupled part `index` (built like this one: [`SoundSet::new`]
    /// for the player's bus, [`SoundSet::new_exterior`] for the others).
    pub fn add_part(&mut self, index: usize, mut part: SoundSet) {
        part.inside = self.inside;
        part.muffled = self.muffled;
        part.ai = self.ai;
        part.listener_vehicle = self.listener_vehicle;
        self.parts.push((index, part));
    }

    /// Per-frame update of the coupled parts' sound sets: the same variables and triggers
    /// as the leading vehicle's, each at its part's place (`part_to_world`; a part that is
    /// gone is silenced).
    pub fn update_parts(
        &mut self,
        engine: &AudioEngine,
        var: &dyn Fn(&str) -> Option<f32>,
        part_to_world: &dyn Fn(usize) -> Option<Mat4>,
        triggers: &[String],
    ) {
        for (i, p) in &mut self.parts {
            match part_to_world(*i) {
                Some(xf) => p.update(engine, var, &xf, triggers),
                None => p.stop_all(engine),
            }
        }
    }

    /// The view an entry's `[viewpoint]` is tested against (Omsi.exe sets it per vehicle,
    /// 0x6ff86c and 0x700158): 2 for the player's bus while the camera is in a cab view, 1
    /// for it from outside, 4 for every other vehicle - an AI bus never plays its
    /// `[viewpoint] 1` sounds, which are the player's bus heard from the street.
    fn view_mask(&self) -> i32 {
        if self.ai {
            4
        } else if self.inside {
            2
        } else {
            1
        }
    }

    /// A sound set heard from outside: an AI vehicle's or another player's (view 4). Its
    /// sounds are placed exactly as the player's bus's: a `[3d]` one at its position, the
    /// others nowhere in particular.
    pub fn new_exterior(engine: &AudioEngine, cfg: &SoundCfg, dir: &Path) -> SoundSet {
        let mut s = Self::new(engine, cfg, dir);
        s.ai = true;
        s
    }

    /// Play the sound of a `(T.F.trigger)` event: the entry listening to `trigger` loads
    /// `file` (relative to the sound folder) into a new buffer and plays it as if its
    /// trigger had fired (0x74f2e8).
    pub fn play_file_trigger(
        &mut self,
        engine: &AudioEngine,
        trigger: &str,
        file: &str,
        var: &dyn Fn(&str) -> Option<f32>,
        object_to_world: &Mat4,
    ) {
        if !engine.enabled || file.trim().is_empty() {
            return;
        }
        let path = omsi_cfg::resolve_path(&self.dir, &file_trigger_name(file));
        let Some(clip) = engine.load_clip(&path) else {
            warn_missing_once(trigger, &path);
            return;
        };
        let ctx = self.ctx(engine);
        for s in self.sounds.iter_mut() {
            if !s.def.triggers.iter().any(|d| d.trim().eq_ignore_ascii_case(trigger)) {
                continue;
            }
            if let Some(id) = s.voice.take() {
                engine.stop(id);
            }
            s.clip = Some(clip.clone());
            s.last_gain = 1.0;
            s.last_pitch = 1.0;
            let e = ctx.eval(s, var, object_to_world);
            if e.audible && !e.wrong_view {
                s.voice = Some(engine.play(clip.clone(), ctx.params(s, &e, false, object_to_world)));
            }
        }
    }

    fn ctx(&self, engine: &AudioEngine) -> Ctx {
        Ctx { listener: engine.listener_position(), ..self.ctx_without_engine() }
    }

    /// What a `[volcurve]` reads. The exe (`TSound` load, 0x74e408) looks the name up in the
    /// vehicle's variables and, when it is none of them, stores `StrToInt(name)` as the
    /// index; the update (0x750584) then reads a negative index from the sound's own
    /// values: -1 = seconds since the sound became active (its conditions started to hold,
    /// GetTickCount/1000), -2 = how much a `[3d]` sound with a direction faces the listener
    /// (1 for a `[3d]` one without, 0 for one without `[3d]`), anything below is skipped
    /// with "Volume Variable not valid!". The LiAZ 5292's engine loops fade in with
    /// `[volcurve] -1` (0 at 1 s, 1 at 1.3 s).
    fn curve_input(
        vc: &omsi_vehicle::VolCurve,
        var: &dyn Fn(&str) -> Option<f32>,
        active: f32,
        facing: f32,
    ) -> Option<f32> {
        match vc.variable.trim().parse::<i32>() {
            Ok(-1) => Some(active),
            Ok(-2) => Some(facing),
            Ok(n) if n < 0 => None,
            _ => Some(var(&vc.variable).unwrap_or(0.0)),
        }
    }

    fn conditions_hold(def: &SoundEntry, var: &dyn Fn(&str) -> Option<f32>) -> bool {
        def.conditions
            .iter()
            .all(|c| c.holds(var(&c.variable).unwrap_or(0.0)))
    }

    /// A `[loopsound]`'s frequency request in Hz (`rate * |variable| / reference`), `None`
    /// for a `[sound]`, which plays at its file's own rate.
    fn frequency(def: &SoundEntry, var: &dyn Fn(&str) -> Option<f32>) -> Option<f32> {
        def.is_loop.then(|| def.sample_rate.trunc() * var(&def.pitch_variable).unwrap_or(0.0).abs() / def.pitch_ref)
    }

    /// The triggers (lower case) whose entries read script variables in a volume curve:
    /// what [`SoundSet::update_fired`] wants the values of at the moment they fire.
    pub fn curve_triggers(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for s in &self.sounds {
            if !s.def.vol_curves.iter().any(|vc| vc.variable.trim().parse::<i32>().is_err()) {
                continue;
            }
            for t in &s.def.triggers {
                let t = t.trim().to_ascii_lowercase();
                if !out.contains(&t) {
                    out.push(t);
                }
            }
        }
        out
    }

    /// Per-frame update. `triggers` are the sound triggers fired by the scripts this frame.
    pub fn update(
        &mut self,
        engine: &AudioEngine,
        var: &dyn Fn(&str) -> Option<f32>,
        object_to_world: &Mat4,
        triggers: &[String],
    ) {
        self.update_fired(engine, var, object_to_world, triggers, &|_, _| None);
    }

    /// [`SoundSet::update`] with `at_fire(trigger, variable)`: a variable as it stood when
    /// the trigger fired (None: as it is now). Omsi.exe starts a trigger's sounds in the
    /// middle of the script (0x74f2e8), so the volume they start with is that of the moment:
    /// the SD200's door hits read `doorSpeed_<n>`, which the door script reverses right
    /// after firing them - read at the frame's end they were silent (#676). From the next
    /// frame on the volume follows the curves as they stand, like every other entry's.
    pub fn update_fired(
        &mut self,
        engine: &AudioEngine,
        var: &dyn Fn(&str) -> Option<f32>,
        object_to_world: &Mat4,
        triggers: &[String],
        at_fire: &dyn Fn(&str, &str) -> Option<f32>,
    ) {
        if !engine.enabled {
            return;
        }
        let ctx = self.ctx(engine);
        for s in self.sounds.iter_mut() {
            // How the exe plays an entry (`TSound` update, 2.2.032):
            // * with a `[trigger]`: once each time the trigger fires, from the start, never
            //   looped (a `[loopsound]` too) and without looking at its conditions;
            // * without one: looped (from a random place in the clip) for as long as its
            //   conditions hold and it can be heard - `[sound]` and `[loopsound]` both loop;
            // * without one but with `[noloop]`: once, when its conditions start to hold.
            let triggered = !s.def.triggers.is_empty();
            let holds = triggered || Self::conditions_hold(&s.def, var);
            let rising = !triggered && holds && !s.held;
            s.held = holds;
            if !triggered {
                if !holds {
                    s.active_since = None;
                } else if s.active_since.is_none() {
                    s.active_since = Some(std::time::Instant::now());
                }
            }
            let Some(clip) = s.clip.clone() else { continue };
            let fired_by = if triggered {
                triggers.iter().find(|t| s.def.triggers.iter().any(|d| d.trim().eq_ignore_ascii_case(t)))
            } else {
                None
            };
            let fired = fired_by.is_some();
            // A voice that starts now is a buffer played anew, at DirectSound's full volume:
            // a request over 0 dB keeps that, not whatever an entry that was not playing had
            // been given in the frames before (a door's end sound, a parking brake or a
            // blinker with `[volume] 2` and a curve low at rest played at that low level, or
            // not at all - #1851, #611, #1268, #1311, #1515, #1900, #1418).
            let looping = !triggered && !s.def.no_loop;
            let idle = s.voice.is_none_or(|id| !engine.is_playing(id));
            if fired || rising || (looping && idle) {
                s.last_gain = 1.0;
            }
            let e = match fired_by {
                Some(t) => ctx.eval(s, &|n| at_fire(t, n).or_else(|| var(n)), object_to_world),
                None => ctx.eval(s, var, object_to_world),
            };
            if e.wrong_view {
                // "Stopped Sound ... globalstop/wrongview"
                if let Some(id) = s.voice.take() {
                    engine.stop(id);
                }
                continue;
            }
            if !triggered && !s.def.no_loop {
                let params = ctx.params(s, &e, true, object_to_world);
                match (s.voice, e.audible) {
                    (Some(id), true) => {
                        if engine.is_playing(id) {
                            engine.set_params(id, params);
                        } else {
                            s.voice = Some(engine.play_from_random_place(clip, params));
                        }
                    }
                    (Some(id), false) => {
                        // "Stopped Sound ... due to volume or pitch"
                        engine.stop(id);
                        s.voice = None;
                    }
                    (None, true) => s.voice = Some(engine.play_from_random_place(clip, params)),
                    (None, false) => {}
                }
                continue;
            }
            // one-shot: started by its trigger or by its conditions starting to hold; the
            // volume follows the curves while it plays
            let params = ctx.params(s, &e, false, object_to_world);
            if (fired || rising) && e.audible {
                if let Some(id) = s.voice {
                    if s.def.only_one && engine.is_playing(id) {
                        engine.set_params(id, params);
                        continue;
                    }
                    engine.stop(id);
                }
                s.voice = Some(engine.play(clip, params));
            } else if let Some(id) = s.voice {
                if engine.is_playing(id) {
                    engine.set_params(id, params);
                } else {
                    s.voice = None;
                }
            }
        }
    }

    /// How many `[sound]`/`[loopsound]` entries the configuration has.
    pub fn len(&self) -> usize {
        self.sounds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sounds.is_empty()
    }

    /// The sounds playing right now: (file, gain at the listener, pitch) - for the logs.
    pub fn playing(&self, engine: &AudioEngine) -> Vec<(String, f32, f32)> {
        self.sounds
            .iter()
            .filter_map(|s| {
                s.voice
                    .and_then(|id| engine.voice_state(id))
                    .map(|(p, heard)| (s.def.file.clone(), heard, p.pitch))
            })
            .collect()
    }

    /// One line per entry of the configuration, saying whether it is heard and why not
    /// (`OMSI_DEBUG_SOUND`): the only way to see which of a bus's hundred sounds the
    /// rewrite never reaches - a missing clip, a condition on a variable nobody feeds, a
    /// volume curve that stays at zero or a `[viewpoint]` the camera is not in. A heard one
    /// gives its gain at the listener (after the distance) and its playback rate.
    pub fn report(&self, engine: &AudioEngine, var: &dyn Fn(&str) -> Option<f32>) -> Vec<String> {
        let ctx = self.ctx(engine);
        let mut out = Vec::new();
        for s in &self.sounds {
            let file = s.def.file.trim();
            let mut why = String::new();
            if s.clip.is_none() {
                why = if file.parse::<i32>().is_ok() {
                    "waits for a (T.F.) file".into()
                } else {
                    "no clip".into()
                };
            } else if !ctx.view_lets_through(s.def.viewpoint) {
                why = format!("viewpoint {} (listener {})", s.def.viewpoint, ctx.view);
            } else if let Some(c) = s
                .def
                .conditions
                .iter()
                .find(|c| s.def.triggers.is_empty() && !c.holds(var(&c.variable).unwrap_or(0.0)))
            {
                why = format!("condition {} = {:?}", c.variable, var(&c.variable));
            } else if let Some(vc) = s.def.vol_curves.iter().find(|vc| {
                let active = s.active_since.map_or(0.0, |t| t.elapsed().as_secs_f32());
                Self::curve_input(vc, var, active, 1.0).is_some_and(|x| curve(&vc.points, x) <= 1.0e-5)
            }) {
                why = format!("volcurve {} = {:?}", vc.variable, var(&vc.variable));
            } else if let Some(f) = Self::frequency(&s.def, var).filter(|f| !(*f >= FREQ_MIN)) {
                why = format!("frequency {f:.0} Hz ({} = {:?})", s.def.pitch_variable, var(&s.def.pitch_variable));
            }
            let playing = s.voice.and_then(|id| engine.voice_state(id));
            match (playing, why.is_empty()) {
                (Some((p, heard)), _) => out.push(format!("{file}: {heard:.3} heard, pitch {:.2}", p.pitch)),
                (None, true) => out.push(format!("{file}: ready, silent")),
                (None, false) => out.push(format!("{file}: off - {why}")),
            }
        }
        out
    }

    /// Every entry's level for one state, without playing anything (for tools comparing
    /// sound configurations): (file, gain at a listener at `listener` - distance included,
    /// pan left out -, playback rate, whether it would be started). `view_inside`/`cab` as
    /// for [`SoundSet::set_inside`]/[`SoundSet::set_muffled`]; triggered entries as if
    /// their trigger fired now.
    pub fn levels(
        &mut self,
        var: &dyn Fn(&str) -> Option<f32>,
        object_to_world: &Mat4,
        listener: Vec3,
    ) -> Vec<(String, f32, f32, bool)> {
        let ctx = Ctx { listener, ..self.ctx_without_engine() };
        self.sounds
            .iter_mut()
            .map(|s| {
                if !s.def.triggers.is_empty() || Self::conditions_hold(&s.def, var) {
                    s.active_since.get_or_insert_with(|| std::time::Instant::now() - std::time::Duration::from_secs(5));
                }
                let e = ctx.eval(s, var, object_to_world);
                let dist = s
                    .def
                    .pos
                    .map(|p| crate::mixer::distance_gain(s.def.range, (object_to_world.transform_point3(Vec3::from_array(p)) - listener).length()))
                    .unwrap_or(1.0);
                let heard = if e.wrong_view || !e.audible { 0.0 } else { e.gain * dist };
                (s.def.file.trim().to_string(), heard, e.pitch, e.audible && !e.wrong_view)
            })
            .collect()
    }

    fn ctx_without_engine(&self) -> Ctx {
        Ctx { view: self.view_mask(), cab: self.muffled, master: self.master, doppler: !self.listener_vehicle, listener: Vec3::ZERO }
    }

    pub fn stop_all(&mut self, engine: &AudioEngine) {
        for s in self.sounds.iter_mut() {
            if let Some(id) = s.voice.take() {
                engine.stop(id);
            }
        }
        for (_, p) in &mut self.parts {
            p.stop_all(engine);
        }
    }
}

/// The per-set inputs of an entry's evaluation.
struct Ctx {
    view: i32,
    cab: bool,
    master: f32,
    doppler: bool,
    listener: Vec3,
}

impl Ctx {
    /// The `[viewpoint]` test (0x750462): the entry is heard when it names the view, names
    /// none, or is an outside sound of the player's bus heard from its cab while
    /// `Snd_OutsideVol` is over 0.01 (doors or a window open).
    fn view_lets_through(&self, vp: i32) -> bool {
        vp == 0 || vp & self.view != 0 || (vp & 2 == 0 && self.view == 2 && outside_vol() > 0.01)
    }

    /// One entry this frame (`TSound` update 0x750340): its volume and pitch as DirectSound
    /// takes them, and whether it can be started.
    fn eval(&self, s: &mut RuntimeSound, var: &dyn Fn(&str) -> Option<f32>, object_to_world: &Mat4) -> Eval {
        let def = &s.def;
        if !self.view_lets_through(def.viewpoint) {
            return Eval { gain: 0.0, audible: false, pitch: s.last_pitch, wrong_view: true };
        }
        let triggered = !def.triggers.is_empty();
        let pos = def.pos.map(|p| object_to_world.transform_point3(Vec3::from_array(p)));
        let active = if triggered {
            TRIGGERED_ACTIVE
        } else {
            s.active_since.map_or(0.0, |t| t.elapsed().as_secs_f32())
        };
        let facing = match (pos, def.dir) {
            (Some(at), Some(d)) => {
                let dir = object_to_world.transform_vector3(Vec3::from_array(d)).normalize_or_zero();
                dir.dot((self.listener - at).normalize_or_zero())
            }
            (Some(_), None) => 1.0,
            (None, _) => 0.0,
        };
        // (an untriggered entry whose conditions fail is at volume 0)
        let mut vol = if triggered || SoundSet::conditions_hold(def, var) { def.volume * self.master } else { 0.0 };
        for vc in &def.vol_curves {
            if let Some(x) = SoundSet::curve_input(vc, var, active, facing) {
                vol *= curve(&vc.points, x);
            }
        }
        // `[3d]`: full up to the range, then range / distance
        let dist_gain = pos.map(|p| crate::mixer::distance_gain(def.range, (p - self.listener).length()));
        if let Some(g) = dist_gain {
            vol *= g;
        }
        // an AI vehicle (view 4) heard from a cab: through the player's bus's bodywork
        if self.view & 4 != 0 && self.cab {
            vol *= 0.2 + outside_vol();
        }
        // the player's bus's outside sound heard in its cab: through what is open
        if def.viewpoint & 2 == 0 && def.viewpoint != 0 && self.view == 2 {
            vol *= outside_vol();
        }
        let (gain, audible) = direct_sound_volume(vol, s.last_gain);
        s.last_gain = gain;
        let mut audible = audible;
        let mut pitch = s.last_pitch;
        if let (Some(f), Some(clip)) = (SoundSet::frequency(def, var), s.clip.as_ref()) {
            // (not started below DirectSound's minimum; a request outside its range is
            // refused and the buffer keeps its rate)
            audible &= f >= FREQ_MIN;
            if (FREQ_MIN..=FREQ_MAX).contains(&f.trunc()) {
                pitch = f.trunc() / clip.sample_rate.max(1) as f32;
                s.last_pitch = pitch;
            }
        }
        // the mixer applies the distance itself, as the listener moves between frames
        let gain = match dist_gain {
            Some(g) if g > 0.0 => gain / g,
            Some(_) => 0.0,
            None => gain,
        };
        Eval { gain, audible, pitch, wrong_view: false }
    }

    fn params(&self, s: &RuntimeSound, e: &Eval, looping: bool, object_to_world: &Mat4) -> VoiceParams {
        let position = s.def.pos.map(|p| object_to_world.transform_point3(Vec3::from_array(p)));
        VoiceParams {
            gain: e.gain,
            pitch: e.pitch.max(0.001),
            looping,
            position,
            // (Omsi.exe shifts the frequency of a `[3d]` loop sound only: a `[sound]`
            // keeps its file's rate)
            doppler: self.doppler && s.def.is_loop,
            range: s.def.range,
            lowpass_hz: 0.0,
            important: s.def.important,
        }
    }
}

/// `Snd_OutsideVol` of the player's bus (bits of an f32): Omsi.exe's 0x859a04, 1 while the
/// player has no vehicle.
static OUTSIDE_VOL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

/// Set the player's bus's `Snd_OutsideVol` (0 for a bus whose scripts never write it), or
/// `None` without a bus of one's own.
pub fn set_outside_open(v: Option<f32>) {
    let v = match v {
        Some(x) if x.is_finite() => x,
        Some(_) => 0.0,
        None => 1.0,
    };
    OUTSIDE_VOL.store(v.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

fn outside_vol() -> f32 {
    f32::from_bits(OUTSIDE_VOL.load(std::sync::atomic::Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use omsi_vehicle::VolCurve;

    fn ctx(view: i32, cab: bool) -> Ctx {
        Ctx { view, cab, master: 1.0, doppler: false, listener: Vec3::ZERO }
    }

    fn eval(c: &Ctx, def: SoundEntry, var: &dyn Fn(&str) -> Option<f32>) -> Eval {
        let mut s = RuntimeSound::new(def, None);
        c.eval(&mut s, var, &Mat4::IDENTITY)
    }

    #[test]
    fn curves_are_linear_and_held_at_the_ends() {
        let p = [(0.0, 0.0), (1.0, 1.0), (1.0, 0.5), (3.0, 0.0)];
        assert_eq!(curve(&p, -1.0), 0.0);
        assert_eq!(curve(&p, 0.5), 0.5);
        // at x = 1 the segment that starts there: (1, 0.5) .. (3, 0)
        assert_eq!(curve(&p, 1.0), 0.5);
        assert_eq!(curve(&p, 2.0), 0.25);
        assert_eq!(curve(&p, 9.0), 0.0);
        assert_eq!(curve(&[], 9.0), 1.0);
    }

    #[test]
    fn direct_sound_refuses_over_0_db() {
        assert_eq!(direct_sound_volume(1.0, 0.3), (1.0, true));
        // 44100 "as a volume": the buffer keeps what it had
        assert_eq!(direct_sound_volume(44100.0, 0.3), (0.3, true));
        assert_eq!(direct_sound_volume(1.3, 1.0), (1.0, true));
        let (g, a) = direct_sound_volume(0.5, 1.0);
        assert!((g - 0.5).abs() < 1e-3 && a);
        assert_eq!(direct_sound_volume(0.0, 0.7), (0.0, false));
        // under -100 dB: refused and not started
        assert_eq!(direct_sound_volume(1.0e-6, 0.7), (0.7, false));
    }

    #[test]
    fn the_cab_hears_outside_through_what_is_open() {
        let none = |_: &str| None;
        // the own bus's outside-only entry (`[viewpoint] 5`) in the cab: at Snd_OutsideVol,
        // not at all with everything shut
        let engine_out = SoundEntry { volume: 0.8, viewpoint: 5, ..Default::default() };
        set_outside_open(Some(0.0));
        assert!(eval(&ctx(2, true), engine_out.clone(), &none).wrong_view);
        set_outside_open(Some(0.5));
        let e = eval(&ctx(2, true), engine_out.clone(), &none);
        assert!((e.gain - 0.4).abs() < 1e-3, "{}", e.gain);
        assert!((eval(&ctx(1, false), engine_out.clone(), &none).gain - 0.8).abs() < 1e-3, "outside: as it is");
        // an AI bus (view 4) heard from a cab: 0.2 + Snd_OutsideVol, no filter
        let ai = SoundEntry { volume: 0.5, viewpoint: 4, ..Default::default() };
        assert!((eval(&ctx(4, true), ai.clone(), &none).gain - 0.35).abs() < 1e-3);
        assert!((eval(&ctx(4, false), ai, &none).gain - 0.5).abs() < 1e-3, "from the street: as it is");
        // the player's bus heard from the street on an AI bus: never
        assert!(eval(&ctx(4, false), SoundEntry { volume: 1.0, viewpoint: 1, ..Default::default() }, &none).wrong_view);
        let cab = SoundEntry { volume: 0.8, viewpoint: 2, ..Default::default() };
        assert!(eval(&ctx(1, false), cab, &none).wrong_view, "a cab sound stays in");
        set_outside_open(None);
    }

    #[test]
    fn volume_curves_multiply_and_a_3d_sound_fades_with_distance() {
        let def = SoundEntry {
            volume: 1.0,
            pos: Some([0.0, 8.0, 0.0]),
            range: 2.0,
            vol_curves: vec![
                VolCurve { variable: "a".into(), points: vec![(0.0, 0.0), (1.0, 1.0)] },
                VolCurve { variable: "b".into(), points: vec![(0.0, 1.0), (1.0, 0.0)] },
            ],
            ..Default::default()
        };
        let var = |n: &str| Some(if n == "a" { 0.5 } else { 0.5 });
        let e = eval(&ctx(1, false), def, &var);
        // 0.5 * 0.5 * (2 / 8), the distance left to the mixer: 0.25 given to the voice
        assert!((e.gain - 0.25).abs() < 1e-3, "{}", e.gain);
        assert!(e.audible);
    }

    #[test]
    fn a_loop_sound_is_not_started_below_100_hz_and_keeps_its_rate_past_200_khz() {
        let def = SoundEntry { volume: 1.0, is_loop: true, sample_rate: 44100.0, pitch_variable: "n".into(), pitch_ref: 1000.0, ..Default::default() };
        let mut s = RuntimeSound::new(def, Some(Arc::new(Clip { sample_rate: 44100, channels: 1, samples: vec![0; 4] })));
        let c = ctx(1, false);
        let e = c.eval(&mut s, &|_| Some(1.0), &Mat4::IDENTITY);
        assert!(!e.audible, "44 Hz");
        let e = c.eval(&mut s, &|_| Some(2000.0), &Mat4::IDENTITY);
        assert!(e.audible && (e.pitch - 2.0).abs() < 1e-4);
        // 6 x 44100 = 264 600 Hz: refused, the rate stays at 2x
        let e = c.eval(&mut s, &|_| Some(6000.0), &Mat4::IDENTITY);
        assert!(e.audible && (e.pitch - 2.0).abs() < 1e-4, "{}", e.pitch);
    }

    /// The SD200's door hit: `[volcurve] doorSpeed_0` from 1 at -1 down to 0 at -0.5,
    /// fired while the door still closes at -1 m/s; by the frame's end the script has turned
    /// the speed round. The volume is the one of the moment it fired (#676).
    #[test]
    fn a_triggered_sound_reads_its_curve_when_it_fires() {
        let hit = SoundEntry {
            volume: 1.0,
            triggers: vec!["ev_doorhitclose_0".into()],
            vol_curves: vec![VolCurve { variable: "doorSpeed_0".into(), points: vec![(-1.0, 1.0), (-0.5, 0.0)] }],
            ..Default::default()
        };
        let now = |n: &str| (n == "doorSpeed_0").then_some(0.8);
        let at_fire = |t: &str, n: &str| (t == "ev_doorhitclose_0" && n == "doorSpeed_0").then_some(-1.0);
        assert_eq!(eval(&ctx(2, true), hit.clone(), &now).gain, 0.0, "read at the frame's end: silent");
        let fired = |n: &str| at_fire("ev_doorhitclose_0", n).or_else(|| now(n));
        assert_eq!(eval(&ctx(2, true), hit.clone(), &fired).gain, 1.0);
        let set = SoundSet { sounds: vec![RuntimeSound::new(hit, None)], master: 1.0, dir: Default::default(), inside: true, ai: false, listener_vehicle: true, muffled: false, parts: Vec::new() };
        assert_eq!(set.curve_triggers(), vec!["ev_doorhitclose_0".to_string()]);
    }

    /// The stock IBIS announces Spandau's "Falkenseer Ch/Stadtrandstr" with the file
    /// `Falkenseer Ch%Stadtrandstr.wav`, as Omsi.exe's file triggers write `/` (#1096).
    #[test]
    fn a_file_trigger_writes_what_a_file_name_cannot_have_as_percent() {
        let file = r"..\..\Announcements\Spandau\Falkenseer Ch/Stadtrandstr.wav";
        assert_eq!(file_trigger_name(file), r"..\..\Announcements\Spandau\Falkenseer Ch%Stadtrandstr.wav");
        assert_eq!(file_trigger_name(r"C:\a?b*c\d<e>f|g.wav"), r"C:\a%b%c\d%e%f%g.wav");
        // found where the announcement lies
        let root = std::env::temp_dir().join(format!("openomsi-file-trigger-{}", std::process::id()));
        let sound = root.join("Vehicles").join("MAN_NL_NG").join("Sound");
        let spandau = root.join("Vehicles").join("Announcements").join("Spandau");
        std::fs::create_dir_all(&sound).unwrap();
        std::fs::create_dir_all(&spandau).unwrap();
        std::fs::write(spandau.join("Falkenseer Ch%Stadtrandstr.wav"), b"RIFF").unwrap();
        let path = omsi_cfg::resolve_path(&sound, &file_trigger_name(file));
        assert!(path.is_file(), "{}", path.display());
        // (the name as the script wrote it: a folder "Falkenseer Ch" that is not there)
        assert!(!omsi_cfg::resolve_path(&sound, file).is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
