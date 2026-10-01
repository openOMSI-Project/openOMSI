//! The driver of the player's bus: a person on the bus's `[drivpos]` with both hands on the
//! steering wheel, turning it as the wheel turns - seen from outside, from the passengers'
//! places and in the mirrors, left out of the driver's own view (the cab view shows him
//! only in the mirrors, as OMSI does).
//!
//! OMSI keeps driver figures of its own among the people (`Humans/*/..._driver.hum`, which
//! the passenger crowd leaves out). The wheel is the mesh the model turns with
//! `Axle_Steering_*` by a large factor (the LiAZ's -1680, the MANs' 1450); its turning axis
//! is its `[newanim]` origin frame, its rim the farthest ring of its vertices round that
//! axis and its centre the middle of that ring on the axis (the origin is often the foot of
//! the column: the Urbino's lies 12 cm under the hub, and the hands held the air under and
//! past the rim). Both hands hold the rim at ten to two, closed round it in fists whose
//! wrists continue the forearms (the hand turned onto a fixed frame on the rim bent the
//! wrists sharply), and turn with it, the wheel's angle read from its own animation
//! variable. Turned out of its reach a hand lets go and takes the rim again further back
//! while the other holds on, as drivers shuffle a bus's wheel through their hands; held
//! still, the wheel gets the hands back at their rest. (Before, the hands stopped at the
//! end of a small range and the rim slid on through them: the wheel seemed to turn by
//! itself under hands frozen in the air.)

use glam::{Mat4, Vec3};
use omsi_render::{AlphaMode, MeshId, Renderer, Scene};
use omsi_sim::human::{curl_hands, grip_centres, hand_slot, skin_from, Activity, HumanType, Pose, PoseInput};
use omsi_sim::VehicleInstance;
use std::sync::Arc;

/// Where the hands rest on the rim, from the top, clockwise seen by the driver (degrees):
/// a little above the sides, ten to two as bus drivers hold a flat wheel.
const REST: [f32; 2] = [-70.0, 70.0];
/// Where each hand can hold the rim (degrees from the top): the hands turn with the wheel
/// within it; a hand turned out of it lets go and takes the rim again further back (the
/// other hand holding on meanwhile), as a driver shuffles the wheel through his hands. (The
/// figure used to keep its hands still and let the rim slide through them past a small
/// range: the wheel seemed to turn by itself under hands frozen in the air.)
const RANGE: [(f32, f32); 2] = [(-150.0, -20.0), (20.0, 150.0)];
/// A hand lets go this far (degrees) past its range at most before the rim slides.
const SLIP: f32 = 25.0;
/// How far a fist rolls round the rim (degrees, see `hand_targets`) and the time constant
/// (s) it rolls with.
const ROLL: (f32, f32) = (-30.0, 120.0);
/// The roll where the forearm says nothing about it.
const ROLL_PLAIN: f32 = 30.0;
const ROLL_EASE: f32 = 0.12;
/// How fast the wrists' targets are moved so that the fists hold the rim (time constant, s)
/// and the most they move for it in a frame (m).
const FIX_EASE: f32 = 0.25;
const FIX_STEP: f32 = 0.002;
/// Time constant (s) a hand turns into the frame its hold asks for.
const FRAME_EASE: f32 = 0.07;
/// How far a hand that lets go reaches back (degrees short of the far end of its range).
const REGRIP_BACK: f32 = 40.0;
/// Lifted off the rim while moving to its new hold (m).
const LIFT: f32 = 0.06;
/// The wheel held still this long (s): the hands go back to their rest one after the other.
const SETTLE_AFTER: f32 = 0.6;
/// The rim does not run exactly across the fist where the forearm comes along it: the hand
/// holds it diagonally (the rim from the base of the forefinger to the heel of the hand)
/// with up to this angle (degrees) between the rim and the knuckles, the wrist straight.
const DIAGONAL: f32 = 40.0;
/// A hip over the seat point stands this far in front of it (the feet under the knees).
const SEAT_FRONT: f32 = 0.34;
/// Radius the fingers close round: the rim's own (measured, see `find_wheel`; this one when
/// there is no wheel) and the fingers' half thickness.
const GRIP_RADIUS: f32 = 0.026;
const FINGER_HALF: f32 = 0.009;
/// The most the seat is slid forward to bring the hands to the wheel (m); what is still
/// missing is made up by leaning forward. Slid further, the driver sat in front of his seat
/// (the interior mirror showed the seat empty).
const SLIDE_MAX: f32 = 0.16;

struct Wheel {
    /// The steering wheel mesh (index into the vehicle's meshes).
    mesh: usize,
    /// Its `[newanim]` variable and factor: the wheel's angle in degrees is their product.
    var: String,
    factor: f32,
    /// The turning axis, pointing at the driver.
    axis: Vec3,
    /// Centre, axis (towards the driver) and the rim's up and right in the wheel's plane,
    /// at rest, in the model frame; the rim radius the hands hold.
    centre: Vec3,
    up: Vec3,
    right: Vec3,
    radius: f32,
    /// The rim's own thickness (its tube's radius, m).
    tube: f32,
}

pub struct DriverFigure {
    ty: Arc<HumanType>,
    /// The figure's meshes with the fingers closed round the rim (`curl_hands`).
    curled: Vec<(Vec<Vec3>, Vec<Vec3>)>,
    /// Wrist to knuckles (m).
    knuckles: f32,
    /// Where the closed fingers hold their bar, rest frame (see `grip_centres`), and how far
    /// the wrists' targets are moved (person frame) so that the bar they hold is the rim:
    /// the hand does not always turn as far as the grip asks (its bend at the wrist is
    /// limited), and the fingers closed round the air beside the rim.
    grip_rest: [Option<Vec3>; 2],
    grip_fix: [Vec3; 2],
    /// The radius the fingers are closed round (the rim's tube and the fingers' half
    /// thickness) that `curled` and `grip_rest` are made for.
    grip_radius: f32,
    pose: Pose,
    meshes: Vec<(MeshId, usize)>,
    skins: Vec<(Vec<Vec3>, Vec<Vec3>)>,
    /// Hip point, floor point in front of the seat, heading (model frame).
    hip: Vec3,
    floor: Vec3,
    heading: f32,
    wheel: Option<Wheel>,
    /// The sign that turns the wheel variable's angle into the angle seen from the seat
    /// (found by comparing it with the mesh as turned; 0 until then).
    sign: f32,
    /// Extra forward lean that brings the hands to the rim (degrees, see SLIDE_MAX), and
    /// what it is with the hands at their places.
    lean: f32,
    base_lean: f32,
    /// Whether hands/arms should remain visible in cab (first-person) view behind settings.
    pub show_hands_in_cab: bool,
    shown: bool,
    /// Posed at least once (the first pose is settled, not eased in from standing).
    settled: bool,
    /// How far the seat is slid forward so that the hands reach the rim (m): a figure of
    /// the stock size on the LiAZ's `[drivpos]` sat 0.7 m behind its wheel with its arms
    /// stretched out in the air.
    slide: f32,
    /// Where each hand holds the rim, or is on its way to (see `steer_hands`).
    hands: [Hand; 2],
    hands_placed: bool,
    /// The wheel's angle seen from the seat (degrees, clockwise) now and a frame ago, and
    /// how long it has been held still (s).
    theta: f32,
    last_theta: f32,
    still: f32,
    /// How fast the wheel turns (degrees/s, smoothed).
    rate: f32,
    /// The elbows as last posed (model frame): the forearms the hands continue.
    elbows: Option<[Vec3; 2]>,
    /// How each hand lies now (wrist to knuckles, palm; model frame), eased.
    frames: [Option<(Vec3, Vec3)>; 2],
    /// How far each fist is rolled round the rim (degrees), eased.
    rolls: [Option<f32>; 2],
    /// Per mesh and vertex, the hand it belongs to (0 left, 1 right, -1 none), and a
    /// scratch copy of a mesh with a hand opening.
    hand_of: Vec<Vec<i8>>,
    blend: (Vec<Vec3>, Vec<Vec3>),
}

/// A hand on the rim: the point it holds, as an angle on the wheel (the angle seen from the
/// seat less the wheel's), or its way to a new hold.
#[derive(Clone, Copy, Default)]
struct Hand {
    on_rim: f32,
    mv: Option<Regrip>,
}

/// A hand let go of the rim and moving to hold it again: from and to angles seen from the
/// seat (degrees), progress 0..1 and duration (s).
#[derive(Clone, Copy)]
struct Regrip {
    from: f32,
    to: f32,
    t: f32,
    dur: f32,
    /// How fast the hand moves along the rim as it lets go and as it takes hold (degrees
    /// over the whole move): it leaves the rim moving with it and meets it the same way,
    /// not stopping dead in one frame.
    v0: f32,
    v1: f32,
}

impl Regrip {
    fn new(from: f32, to: f32, dur: f32, rate: f32) -> Regrip {
        let v = (rate * dur).clamp(-60.0, 60.0);
        Regrip { from, to, t: 0.0, dur, v0: v, v1: v }
    }
}

impl Hand {
    /// The angle the hand is at, seen from the seat, and how far it is lifted off the rim
    /// (0..1).
    fn seen(&self, theta: f32) -> (f32, f32) {
        match self.mv {
            Some(m) => {
                // a Hermite curve: from and to, leaving and arriving with the rim's speed
                let t = m.t.clamp(0.0, 1.0);
                let (t2, t3) = (t * t, t * t * t);
                let a = m.from * (2.0 * t3 - 3.0 * t2 + 1.0) + m.v0 * (t3 - 2.0 * t2 + t) + m.to * (3.0 * t2 - 2.0 * t3) + m.v1 * (t3 - t2);
                (a, (t * std::f32::consts::PI).sin().powi(2))
            }
            None => (self.on_rim + theta, 0.0),
        }
    }

    /// How far its fingers are open (0 closed round the rim).
    fn open(&self) -> f32 {
        self.mv.map(|m| (m.t * std::f32::consts::PI).sin().powi(2) * 0.45).unwrap_or(0.0)
    }
}

/// Ease in and out with no jolt at either end (smootherstep).
fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Where the hands go this frame (model frame): the wrists, how each hand lies (wrist to
/// knuckles, the way the palm faces) and the points of the rim they hold.
struct Targets {
    grips: [Vec3; 2],
    frames: [(Vec3, Vec3); 2],
    tubes: [Vec3; 2],
}

impl DriverFigure {
    /// The driver for the player's bus, when it has a `[drivpos]` and a driver figure is
    /// installed.
    /// `pick` chooses among the map's drivers (`drivers.txt`): 0 for the player, a vehicle's
    /// own number for the traffic.
    pub fn new(
        world: &crate::scene::World,
        renderer: &Renderer,
        scene: &mut Scene,
        v: &VehicleInstance,
        pick: u64,
    ) -> Option<DriverFigure> {
        let ty = driver_type(world, pick)?;
        Self::new_with(world, renderer, scene, v, ty)
    }

    /// The driver as another game has them (LAN): the figure it names by its `.hum` file
    /// (relative to the installation), else one of the map's as `new` picks.
    pub fn new_named(
        world: &crate::scene::World,
        renderer: &Renderer,
        scene: &mut Scene,
        v: &VehicleInstance,
        hum: &str,
        pick: u64,
    ) -> Option<DriverFigure> {
        let named = omsi_net::human_path(hum)
            .map(|rel| omsi_cfg::resolve_path(&world.root, &rel))
            .filter(|p| omsi_cfg::vfs::exists(p))
            .and_then(|p| cached_type(&p));
        let ty = named.or_else(|| driver_type(world, pick))?;
        Self::new_with(world, renderer, scene, v, ty)
    }

    fn new_with(
        world: &crate::scene::World,
        renderer: &Renderer,
        scene: &mut Scene,
        v: &VehicleInstance,
        ty: Arc<HumanType>,
    ) -> Option<DriverFigure> {
        let seat = seat_of(v)?;
        let mut meshes = Vec::new();
        let dirs = ty.texture_dirs(&world.root);
        for hm in &ty.meshes {
            let mut mats = Vec::new();
            for (k, m) in hm.materials.iter().enumerate() {
                let look: Vec<&std::path::Path> = dirs.iter().map(|p| p.as_path()).collect();
                let tex = omsi_texture::find_texture(&m.texture, &look).and_then(|path| {
                    let t = world
                        .textures
                        .get_gpu_fast(&path)
                        .map(|(img, _)| renderer.add_texture_data(scene, &img));
                    world.textures.release(&path);
                    t
                });
                let alpha = match hm.alpha.get(k).copied().unwrap_or(0) {
                    1 => AlphaMode::Test,
                    2 => AlphaMode::Blend,
                    _ => AlphaMode::Opaque,
                };
                mats.push(renderer.add_material(scene, tex, alpha, [1.0; 4], false));
            }
            let id = renderer.add_mesh(scene, &hm.data);
            let inst = renderer.add_instance(scene, id, v.position, Mat4::IDENTITY, mats);
            meshes.push((id, inst));
        }
        let curled = curl_hands(&ty, GRIP_RADIUS);
        let knuckles = (ty.joints.finger - ty.joints.hand).length().clamp(0.12, 0.3) * 0.58;
        let hand_of = ty
            .meshes
            .iter()
            .map(|m| {
                m.skin
                    .iter()
                    .map(|inf| {
                        (0..2)
                            .find(|&side| (0..inf.n as usize).any(|j| inf.slot[j] as usize == hand_slot(side) && inf.weight[j] > 0.5))
                            .map(|side| side as i8)
                            .unwrap_or(-1)
                    })
                    .collect()
            })
            .collect();
        let grip_rest = grip_centres(&ty, GRIP_RADIUS);
        let mut f = DriverFigure {
            ty,
            curled,
            knuckles,
            grip_rest,
            grip_fix: [Vec3::ZERO; 2],
            grip_radius: GRIP_RADIUS,
            pose: Pose::new(0x5eed_d71e),
            meshes,
            skins: Vec::new(),
            hip: Vec3::ZERO,
            floor: Vec3::ZERO,
            heading: 0.0,
            wheel: None,
            sign: 0.0,
            lean: 0.0,
            base_lean: 0.0,
            show_hands_in_cab: false,
            shown: true,
            settled: false,
            slide: 0.0,
            hands: [Hand::default(); 2],
            hands_placed: false,
            theta: 0.0,
            last_theta: 0.0,
            still: 0.0,
            rate: 0.0,
            elbows: None,
            frames: [None; 2],
            rolls: [None; 2],
            hand_of,
            blend: Default::default(),
        };
        f.seat_in(v, seat);
        Some(f)
    }

    /// Put the figure into (another) vehicle's driver's seat: `false` when it has none.
    /// The traffic keeps a few figures and moves them from bus to bus.
    pub fn attach(&mut self, v: &VehicleInstance) -> bool {
        match seat_of(v) {
            Some(seat) => {
                self.seat_in(v, seat);
                true
            }
            None => false,
        }
    }

    fn seat_in(&mut self, v: &VehicleInstance, seat: omsi_vehicle::cabin::PassPos) {
        let hip = Vec3::from(seat.pos);
        let r = seat.rot.to_radians();
        self.hip = hip;
        self.floor = Vec3::new(
            hip.x + r.sin() * SEAT_FRONT,
            hip.y + r.cos() * SEAT_FRONT,
            hip.z - seat.height.max(0.3),
        );
        self.heading = seat.rot;
        self.wheel = find_wheel(v, hip);
        let r = self.wheel.as_ref().map(|w| w.tube + FINGER_HALF).unwrap_or(GRIP_RADIUS);
        if (r - self.grip_radius).abs() > 1e-4 {
            self.curled = curl_hands(&self.ty, r);
            self.grip_rest = grip_centres(&self.ty, r);
            self.grip_radius = r;
        }
        self.sign = 0.0;
        self.grip_fix = [Vec3::ZERO; 2];
        self.pose = Pose::new(0x5eed_d71e);
        self.settled = false;
        self.slide = 0.0;
        self.lean = 0.0;
        self.base_lean = 0.0;
        self.hands_placed = false;
        self.elbows = None;
        self.frames = [None; 2];
        self.rolls = [None; 2];
        log::debug!(
            "driver: {} on [drivpos] ({:.2}, {:.2}, {:.2}){}",
            self.ty.def.path.file_name().unwrap_or_default().to_string_lossy(),
            hip.x,
            hip.y,
            hip.z,
            self.wheel
                .as_ref()
                .map(|w| format!(", hands on the wheel (rim {:.2} m at ({:.2}, {:.2}, {:.2}))", w.radius, w.centre.x, w.centre.y, w.centre.z))
                .unwrap_or_else(|| ", no steering wheel found: hands on the lap".into())
        );
    }

    /// Hide the figure (kept for another bus).
    pub fn hide(&mut self, renderer: &Renderer, scene: &mut Scene) {
        for (_, inst) in &self.meshes {
            renderer.set_params(scene, *inst, &[], false, &[]);
        }
        self.shown = false;
    }

    /// Turn the hands with the wheel, pose, skin and place the figure; `show` false hides it,
    /// `mirror_only` keeps it out of the window's picture but in the mirrors (the driver's
    /// own view: OMSI shows the driver in the mirrors while one looks from his seat).
    pub fn update(&mut self, renderer: &Renderer, scene: &mut Scene, v: &VehicleInstance, dt: f32, show: bool, mirror_only: bool) {
        if show != self.shown {
            for (_, inst) in &self.meshes {
                renderer.set_params(scene, *inst, &[], show, &[]);
            }
            self.shown = show;
        }

        // (in the cab with the hands shown: the figure is drawn in the window's picture too,
        // with everything but the hands folded away, see the skinning below)
        let force_visible_in_cab = mirror_only && self.show_hands_in_cab;
        for (_, inst) in &self.meshes {
            renderer.set_mirror_only(scene, *inst, if force_visible_in_cab { false } else { mirror_only });
        }

        if !show {
            return;
        }
        if let Some(theta) = self.wheel_angle(v) {
            self.steer_hands(theta, if self.settled { dt } else { 0.0 });
        }
        // the person's own frame: feet at `floor` (slid forward by `slide`), facing `heading`
        let h = self.heading.to_radians();
        let fwd = Vec3::new(h.sin(), h.cos(), 0.0);
        if !self.settled {
            // sat down already when first seen (an offscreen picture is one frame), the
            // seat slid up until the hands reach the rim
            for round in 0..20 {
                let mut p = Pose::new(0x5eed_d71e);
                let targets = self.hand_targets(v, 0.0);
                let try_input = self.pose_input(targets.as_ref(), fwd);
                for _ in 0..90 {
                    p.advance(&self.ty.rig, &try_input, 1.0 / 30.0);
                }
                let posed = p.bones(&self.ty.rig);
                let miss = match (try_input.grips, posed.ok) {
                    (Some(g), true) => (0..2).map(|k| (posed.wrist[k] - g[k]).length()).fold(0.0f32, f32::max),
                    _ => 0.0,
                };
                if omsi_cfg::env::var_os("OMSI_DEBUG_DRIVER").is_some() {
                    log::info!("driver settle {round}: slide {:.2} grips {:?} wrists {:?} elbows {:?} neck {:?} hip {:?}", self.slide, try_input.grips, posed.wrist, posed.elbow, posed.neck, posed.hip);
                }
                // the hand's frame follows the forearm: pose again until both settle
                let moved = if posed.ok { self.keep_elbows(&posed.elbow) } else { 0.0 };
                let off = match (&targets, posed.ok) {
                    (Some(t), true) => {
                        let tubes = t.tubes.map(|q| self.to_person(q));
                        self.correct_grips(&posed.bones, tubes, 1.0, [true; 2])
                    }
                    _ => 0.0,
                };
                if (miss < 0.02 && off < 0.01 && moved < 0.01) || (self.slide >= SLIDE_MAX && self.lean >= 30.0) || round == 19 {
                    self.pose = p;
                    break;
                }
                if miss < 0.02 {
                    // only the fingers' hold or the forearm moved: pose again
                } else if self.slide < SLIDE_MAX {
                    self.slide = (self.slide + miss * 0.8).min(SLIDE_MAX);
                } else {
                    // about 1.1 cm of reach per degree of lean for a seated adult
                    self.lean = (self.lean + (miss / 0.011).max(2.0)).min(30.0);
                }
            }
            self.settled = true;
            self.base_lean = self.lean;
            log::debug!("driver: seat slid {:.2} m forward and {:.0} deg of lean to reach the wheel", self.slide, self.lean);
            return self.update(renderer, scene, v, dt, show, mirror_only);
        }
        let targets = self.hand_targets(v, dt);
        if let (Some(t), true) = (&targets, omsi_cfg::env::var_os("OMSI_DEBUG_DRIVER").is_some()) {
            log::info!("HANDT {dt:.4} {:?} {:?} {:?} {:?} seen {:.1} {}", t.grips[0].to_array(), t.frames[0].0.to_array(), t.frames[0].1.to_array(), t.grips[1].to_array(), self.hands[0].seen(self.theta).0, self.hands[0].mv.is_some());
        }
        let input = self.pose_input(targets.as_ref(), fwd);
        let floor = self.floor + fwd * self.slide;
        self.pose.advance(&self.ty.rig, &input, dt);
        let posed = self.pose.bones(&self.ty.rig);
        if posed.ok {
            self.keep_elbows(&posed.elbow);
        }
        if omsi_cfg::env::var_os("OMSI_DEBUG_DRIVER").is_some() {
            log::info!("HANDP {dt:.4} {:?} {:?} {:?} lean {:.2}", posed.wrist[0].to_array(), posed.elbow[0].to_array(), self.grip_fix[0].to_array(), self.lean);
        }
        if let (Some(t), true) = (&targets, posed.ok) {
            // (drawn this frame as posed; the next frame holds the rim) - a hand on its way
            // to a new hold keeps the correction it had
            let tubes = t.tubes.map(|q| self.to_person(q));
            let holding = [0, 1].map(|k| self.hands[k].mv.is_none());
            let off = self.correct_grips(&posed.bones, tubes, 1.0 - (-dt / FIX_EASE).exp(), holding);
            if omsi_cfg::env::var_os("OMSI_DEBUG_DRIVER").is_some() {
                log::info!("driver: grip off the rim {off:.3} m, fix {:?}", self.grip_fix);
            }
        }
        // a hold the arms do not quite reach (the top of a tilted wheel is further off than
        // its sides): lean towards it, and back again when the hands are nearer
        if let (Some(g), true) = (input.grips, posed.ok && dt > 0.0) {
            let miss = (0..2)
                .map(|k| (posed.wrist[k] - g[k]).length())
                .fold(0.0f32, f32::max);
            if miss > 0.015 {
                self.lean = (self.lean + (miss / 0.011) * dt * 4.0).min(34.0).min(self.base_lean + 6.0);
            } else if miss < 0.006 {
                self.lean = (self.lean - 4.0 * dt).max(self.base_lean);
            }
        }
        if omsi_cfg::env::var_os("OMSI_DEBUG_DRIVER").is_some() {
            if let Some(g) = input.grips {
                log::info!(
                    "driver: wheel {:.0} deg (sign {}), hands at {:.0} {:.0}{}{}, lean {:.1}, miss {:.3} {:.3}",
                    self.theta,
                    self.sign,
                    self.hands[0].seen(self.theta).0,
                    self.hands[1].seen(self.theta).0,
                    if self.hands[0].mv.is_some() { " (left regrips)" } else { "" },
                    if self.hands[1].mv.is_some() { " (right regrips)" } else { "" },
                    self.lean,
                    (posed.wrist[0] - g[0]).length(),
                    (posed.wrist[1] - g[1]).length()
                );
            }
        }
        if !posed.ok && !self.skins.is_empty() {
            return;
        }
        self.skins.resize_with(self.ty.meshes.len(), Default::default);
        let open = [0, 1].map(|k| self.hands[k].open());
        for (k, m) in self.ty.meshes.iter().enumerate() {
            let (pos, nrm) = &mut self.skins[k];
            if open.iter().any(|&o| o > 0.01) {
                // a hand on its way to a new hold opens its fingers
                let blend = &mut self.blend;
                blend.0.clone_from(&self.curled[k].0);
                blend.1.clone_from(&self.curled[k].1);
                for (i, side) in self.hand_of[k].iter().enumerate() {
                    if *side >= 0 && open[*side as usize] > 0.01 {
                        let o = open[*side as usize];
                        blend.0[i] = blend.0[i].lerp(m.data.positions[i], o);
                        blend.1[i] = blend.1[i].lerp(m.data.normals[i], o).normalize_or_zero();
                    }
                }
                skin_from(m, blend, &posed.bones, pos, nrm);
            } else {
                skin_from(m, &self.curled[k], &posed.bones, pos, nrm);
            }

            // In the cab view only the hands show: every other vertex is folded onto the
            // nearer wrist (the middle of that hand's vertices), so that the body's triangles
            // shrink to nothing and those joining a hand close it at the wrist. (Folded onto
            // the figure's origin, the triangles from the wrists stretched to the seat.)
            if mirror_only && self.show_hands_in_cab {
                let mut sum = [Vec3::ZERO; 2];
                let mut n = [0.0f32; 2];
                for (i, side) in self.hand_of[k].iter().enumerate() {
                    if *side >= 0 && i < pos.len() {
                        sum[*side as usize] += pos[i];
                        n[*side as usize] += 1.0;
                    }
                }
                let wrist: Vec<Vec3> = (0..2).filter(|h| n[*h] > 0.0).map(|h| sum[h] / n[h]).collect();
                let fold = wrist.first().copied().or_else(|| pos.first().copied()).unwrap_or(Vec3::ZERO);
                for (i, side) in self.hand_of[k].iter().enumerate() {
                    if *side < 0 && i < pos.len() {
                        pos[i] = wrist.iter().copied().min_by(|a, b| a.distance_squared(pos[i]).total_cmp(&b.distance_squared(pos[i]))).unwrap_or(fold);
                    }
                }
            }

            renderer.update_mesh(scene, self.meshes[k].0, pos, nrm, &m.data.uvs);
        }
        let body = v.body_rotation();
        let at = v.position + body.transform_point3(floor).as_dvec3();
        let xf = body * Mat4::from_rotation_z(-h);
        // lit by the lamps near the seat as they are (the driver's lamp, the saloon lamps
        // over the front door), not by the brightest lamp anywhere in the bus: taken as the
        // saloon's strongest light at full strength the driver glowed evenly all night as
        // soon as any circuit was on, twice as bright as the passengers (who get half)
        let interior = v.interior_light_at(self.hip + Vec3::new(0.0, 0.0, 0.55)) * 0.5;
        for (_, inst) in &self.meshes {
            renderer.set_transform(scene, *inst, at, xf);
            renderer.set_interior(scene, *inst, interior);
        }
    }
}

impl DriverFigure {
    /// The wheel's frame as it stands now (model frame): centre, axis towards the driver, up
    /// and right in its plane. An adjustable column (the SD202's) tilts the whole wheel, and
    /// holds reckoned in the resting frame put the fists beside the tilted rim.
    fn wheel_frame(w: &Wheel, v: &VehicleInstance) -> (Mat4, Vec3, Vec3, Vec3, Vec3) {
        let turn = v.mesh_transforms.get(w.mesh).copied().unwrap_or(Mat4::IDENTITY);
        let centre = turn.transform_point3(w.centre);
        let axis = turn.transform_vector3(w.axis).normalize_or(w.axis);
        let mut up = (Vec3::Z - axis * axis.dot(Vec3::Z)).normalize_or_zero();
        if up.length_squared() < 0.5 {
            up = w.up;
        }
        let right = up.cross(axis).normalize_or(w.right);
        let right = if right.dot(w.right) < 0.0 { -right } else { right };
        (turn, centre, axis, up, right)
    }

    /// How far the wheel is turned, seen from the driver (degrees clockwise): from its
    /// variable (all the turns of a lock to lock), its sign found from the mesh as turned now
    /// (within one turn).
    fn wheel_angle(&mut self, v: &VehicleInstance) -> Option<f32> {
        let w = self.wheel.as_ref()?;
        let (turn, _, _, up, right) = Self::wheel_frame(w, v);
        let up_now = turn.transform_vector3(w.up).normalize_or(w.up);
        let seen = up_now.dot(right).atan2(up_now.dot(up)).to_degrees();
        let by_var = v.var(&w.var).unwrap_or(0.0) * w.factor;
        if seen.abs() > 10.0 && seen.abs() < 170.0 {
            self.sign = if wrap(by_var - seen).abs() <= wrap(-by_var - seen).abs() { 1.0 } else { -1.0 };
        }
        let theta = if self.sign != 0.0 { by_var * self.sign } else { seen };
        self.theta = theta.clamp(-3600.0, 3600.0);
        Some(self.theta)
    }

    /// Move the hands with the wheel turned to `theta`: each turns with the rim while it
    /// stays within its range; one turned out of it lets go and takes the rim again further
    /// back while the other holds on (the rim slides through a hand only while the other is
    /// off it); held still a while, the wheel gets its hands back at their rest.
    fn steer_hands(&mut self, theta: f32, dt: f32) {
        // OMSI_DRIVER_HANDS=<left>,<right>: both hands held at these angles (grip close-ups)
        if let Some(a) = omsi_cfg::env::var("OMSI_DRIVER_HANDS").ok().and_then(|s| {
            let v: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            (v.len() == 2).then(|| [v[0], v[1]])
        }) {
            self.hands = [0, 1].map(|k| Hand { on_rim: a[k] - theta, mv: None });
            self.hands_placed = true;
            return;
        }
        if !self.hands_placed {
            for k in 0..2 {
                self.hands[k] = Hand { on_rim: REST[k] - theta, mv: None };
            }
            self.hands_placed = true;
            self.last_theta = theta;
            self.still = 0.0;
            return;
        }
        let turned = theta - self.last_theta;
        self.last_theta = theta;
        if dt > 0.0 {
            self.rate += (turned / dt - self.rate) * (dt / 0.15).min(1.0);
        }
        if turned.abs() < 12.0 * dt.max(1e-3) {
            self.still += dt;
        } else {
            self.still = 0.0;
        }
        for k in 0..2 {
            if let Some(m) = &mut self.hands[k].mv {
                m.t += dt / m.dur;
                if m.t >= 1.0 {
                    self.hands[k] = Hand { on_rim: m.to - theta, mv: None };
                }
            }
        }
        let inside = |k: usize, a: f32| a >= RANGE[k].0 && a <= RANGE[k].1;
        // the hand furthest out of its range goes first
        let mut order = [0usize, 1];
        let out_by = |h: &Hand, k: usize| {
            let a = h.on_rim + theta;
            (RANGE[k].0 - a).max(a - RANGE[k].1)
        };
        if out_by(&self.hands[1], 1) > out_by(&self.hands[0], 0) {
            order = [1, 0];
        }
        for k in order {
            if self.hands[k].mv.is_some() {
                continue;
            }
            let a = self.hands[k].on_rim + theta;
            if inside(k, a) {
                continue;
            }
            // (the other hand keeps hold meanwhile, sliding if it has to)
            if self.hands[1 - k].mv.is_none() {
                // back against the turn, towards the other end of the range
                let (lo, hi) = RANGE[k];
                let to = if a > hi { lo + REGRIP_BACK } else { hi - REGRIP_BACK };
                let to = if (to - REST[k]).abs() > 90.0 { REST[k] } else { to };
                // (from where the hand is now: a hand that slid is already past the range)
                let from = a;
                // an unhurried reach back, only a little quicker the faster the wheel turns
                // (at a tenth of a second a hand flew back to its hold)
                let dur = (0.38 + (to - from).abs() / 350.0) / (1.0 + self.rate.abs() / 2000.0);
                self.hands[k] = Hand { on_rim: self.hands[k].on_rim, mv: Some(Regrip::new(from, to, dur.max(0.32), self.rate)) };
            } else {
                // the rim slides through the hand past its range: the hand goes on with it
                // less and less over SLIP degrees (stopped dead at a limit, it jumped)
                let (lo, hi) = RANGE[k];
                let before = self.hands[k].on_rim + theta - turned;
                let over = (lo - before).max(before - hi).max(0.0);
                let outward = (before > hi && turned > 0.0) || (before < lo && turned < 0.0);
                let follow = if outward { 1.0 - smooth(over / SLIP) } else { 1.0 };
                let now = (before + turned * follow).clamp(lo - SLIP, hi + SLIP);
                self.hands[k].on_rim = now - theta;
            }
        }
        if self.still > SETTLE_AFTER && self.hands.iter().all(|h| h.mv.is_none()) {
            let far = |k: usize| (self.hands[k].on_rim + theta - REST[k]).abs();
            let k = if far(0) >= far(1) { 0 } else { 1 };
            if far(k) > 22.0 {
                let from = self.hands[k].on_rim + theta;
                self.hands[k].mv = Some(Regrip::new(from, REST[k], 0.45 + (REST[k] - from).abs() / 300.0, 0.0));
                self.still = 0.0;
            }
        }
    }

    /// Where the wrists go and how the hands lie: each fist closed round the rim at the angle
    /// its hand is at, the wrist continuing the forearm; a hand on its way to a new hold
    /// lifted off the rim towards the driver.
    /// `dt` eases each hand's frame towards the one its hold asks for (0 takes it at once).
    fn hand_targets(&mut self, v: &VehicleInstance, dt: f32) -> Option<Targets> {
        let w = self.wheel.as_ref()?;
        let (_, centre, axis, up, right) = Self::wheel_frame(w, v);
        let h = self.heading.to_radians();
        let fwd = Vec3::new(h.sin(), h.cos(), 0.0);
        let mut t = Targets { grips: [Vec3::ZERO; 2], frames: [(Vec3::ZERO, Vec3::ZERO); 2], tubes: [Vec3::ZERO; 2] };
        for k in 0..2 {
            let (seen, lift) = self.hands[k].seen(self.theta);
            let a = seen.to_radians();
            let radial = (up * a.cos() + right * a.sin()).normalize_or(up);
            let along = (right * a.cos() - up * a.sin()).normalize_or(right);
            let tube = centre + radial * w.radius + (axis * 0.85 + radial * 0.3) * (LIFT * lift);
            // the forearm: from the elbow as last posed, else from about where it will be
            let elbow = self.elbows.map(|e| e[k]).unwrap_or(self.hip + Vec3::Z * 0.2 - fwd * 0.05 + (tube - centre).with_z(0.0) * 0.5);
            let fore = (tube - elbow).normalize_or(fwd);
            // How far the fist is rolled round the rim (0: the fingers out over the rim's
            // outer edge, the palm on it from the driver's side; 90: the knuckles turned away
            // from the driver, the palm towards the wheel's middle, as round an upright
            // wheel's sides): as near the forearm's line as it goes, within what a wrist does.
            // Where the forearm runs along the rim (the sides of a flat wheel) its line says
            // nothing about the roll and the fist keeps to the plain grip over the outer edge
            // (taken from the line alone, the palm flipped over there from frame to frame).
            let (e1, e2) = (radial, -axis);
            let proj = fore - along * along.dot(fore);
            let want = proj.dot(e2).atan2(proj.dot(e1)).to_degrees().clamp(ROLL.0, ROLL.1);
            let want = ROLL_PLAIN + (want - ROLL_PLAIN) * smooth((proj.length() - 0.2) / 0.4);
            let roll = match self.rolls[k] {
                Some(r) if dt > 0.0 => r + (want - r) * (1.0 - (-dt / ROLL_EASE).exp()),
                _ => want,
            };
            self.rolls[k] = Some(roll);
            let (sr, cr) = roll.to_radians().sin_cos();
            let across = (e1 * cr + e2 * sr).normalize_or(e1);
            // the rim held diagonally, the knuckles turned towards the forearm's line
            let dir = turn_towards(across, fore, DIAGONAL.to_radians());
            // a right hand's thumb lies towards the top of the wheel on its right side, a
            // left hand's likewise on its left: the fingers close round the rim the way that
            // puts it there (the other way round was a mirrored hand, the thumb pointing down
            // the rim and the fingers held in over the top)
            let palm = dir.cross(along).normalize_or(-axis);
            let palm = (palm - dir * dir.dot(palm)).normalize_or(palm);
            // the hand turns into its new frame over a moment, not in one frame
            let (dir, palm) = match self.frames[k] {
                Some((d0, p0)) if dt > 0.0 => {
                    let q0 = glam::Quat::from_mat3(&glam::Mat3::from_cols(d0, p0, d0.cross(p0)));
                    let q1 = glam::Quat::from_mat3(&glam::Mat3::from_cols(dir, palm, dir.cross(palm)));
                    let q = q0.slerp(q1, 1.0 - (-dt / FRAME_EASE).exp()).normalize();
                    (q * Vec3::X, q * Vec3::Y)
                }
                _ => (dir, palm),
            };
            self.frames[k] = Some((dir, palm));
            let knuckle = tube - palm * self.grip_radius;
            t.grips[k] = knuckle - dir * self.knuckles;
            t.frames[k] = (dir, palm);
            t.tubes[k] = tube;
        }
        Some(t)
    }

    /// The pose input of the driver at the wheel (the grips in the person's frame).
    fn pose_input(&self, targets: Option<&Targets>, fwd: Vec3) -> PoseInput<'static> {
        let h = self.heading.to_radians();
        let turn_person = move |d: Vec3| Vec3::new(d.x * h.cos() - d.y * h.sin(), d.x * h.sin() + d.y * h.cos(), d.z);
        let floor = self.floor + fwd * self.slide;
        let hip = self.hip + fwd * self.slide;
        PoseInput {
            activity: Activity::Sit,
            origin: floor.as_dvec3(),
            heading: self.heading as f64,
            frame: 1,
            seat: Some(self.to_person(hip)),
            look: Some(self.to_person(hip + fwd * 20.0 + Vec3::Z * 0.4)),
            grips: targets.map(|t| [0, 1].map(|k| self.to_person(t.grips[k]) + self.grip_fix[k])),
            grip_frames: targets.map(|t| t.frames.map(|(d, p)| (turn_person(d), turn_person(p)))),
            grip_lean: self.lean,
            ..Default::default()
        }
    }

    /// A model-frame point in the person's frame (feet at the slid floor point, facing the
    /// seat's heading), and back.
    fn to_person(&self, q: Vec3) -> Vec3 {
        let h = self.heading.to_radians();
        let d = q - (self.floor + Vec3::new(h.sin(), h.cos(), 0.0) * self.slide);
        Vec3::new(d.x * h.cos() - d.y * h.sin(), d.x * h.sin() + d.y * h.cos(), d.z)
    }

    fn from_person(&self, d: Vec3) -> Vec3 {
        let h = self.heading.to_radians();
        let floor = self.floor + Vec3::new(h.sin(), h.cos(), 0.0) * self.slide;
        floor + Vec3::new(d.x * h.cos() + d.y * h.sin(), -d.x * h.sin() + d.y * h.cos(), d.z)
    }

    /// Keep the posed elbows (person frame) for the next hands' frames; how far they moved.
    fn keep_elbows(&mut self, posed: &[Vec3; 2]) -> f32 {
        let now = posed.map(|e| self.from_person(e));
        let moved = self.elbows.map(|e| (0..2).map(|k| (e[k] - now[k]).length()).fold(0.0, f32::max)).unwrap_or(1.0);
        self.elbows = Some(now);
        moved
    }

    /// Move the wrists' targets by `gain` of what separates the bar the posed fingers hold
    /// from the rim (`tubes`, person frame); the largest distance left.
    fn correct_grips(&mut self, bones: &[glam::Affine3A], tubes: [Vec3; 2], gain: f32, which: [bool; 2]) -> f32 {
        let mut worst = 0.0f32;
        for k in (0..2).filter(|&k| which[k]) {
            let Some(rest) = self.grip_rest[k] else { continue };
            let Some(b) = bones.get(hand_slot(k)) else { continue };
            let held = Vec3::from(b.transform_point3a(rest.into()));
            let err = tubes[k] - held;
            worst = worst.max(err.length());
            // (a few millimetres a frame at most: a larger step read as the hand jumping)
            let step = err * gain;
            let step = if gain < 1.0 { step.clamp_length_max(FIX_STEP) } else { step };
            let fix = self.grip_fix[k] + step;
            self.grip_fix[k] = fix.clamp_length_max(0.15);
        }
        worst
    }
}

/// The vehicle's first `[drivpos]` (its passenger cabin is read once per type).
fn seat_of(v: &VehicleInstance) -> Option<omsi_vehicle::cabin::PassPos> {
    static SEATS: std::sync::Mutex<Option<std::collections::HashMap<std::path::PathBuf, Option<omsi_vehicle::cabin::PassPos>>>> =
        std::sync::Mutex::new(None);
    let key = v.ty.def.path.clone();
    let mut seats = SEATS.lock().unwrap_or_else(|e| e.into_inner());
    seats
        .get_or_insert_with(Default::default)
        .entry(key)
        .or_insert_with(|| {
            let rel = v.ty.def.passenger_cabin.as_ref()?;
            let cabin = omsi_vehicle::PassengerCabin::load(&omsi_cfg::resolve_path(v.ty.def.dir(), rel))
                .map_err(|e| log::warn!("driver: {e}"))
                .ok()?;
            cabin.driver_positions.first().cloned()
        })
        .clone()
}

/// `a` turned towards `b` by `max` radians at most (both unit vectors).
fn turn_towards(a: Vec3, b: Vec3, max: f32) -> Vec3 {
    let angle = a.angle_between(b);
    if angle <= max || !angle.is_finite() {
        return b;
    }
    let axis = a.cross(b).normalize_or_zero();
    if axis == Vec3::ZERO {
        return a;
    }
    glam::Quat::from_axis_angle(axis, max) * a
}

impl DriverFigure {
    /// The figure at the wheel (the player's own when getting up).
    pub fn human_type(&self) -> Arc<HumanType> {
        self.ty.clone()
    }
}

fn wrap(a: f32) -> f32 {
    (a + 540.0).rem_euclid(360.0) - 180.0
}

/// The driver figure: one of the map's `drivers.txt` (the human files OMSI draws at the
/// wheel of its buses - Spandau and Grundorf name `humans\\axyz\\man01.hum`; OMSI
/// reads the list with the map, the original), chosen by `pick`; without the list OMSI's own
/// driver figure `Humans/*/DBC_man04_driver.hum`. Each file is read once.
fn driver_type(world: &crate::scene::World, pick: u64) -> Option<Arc<HumanType>> {
    let listed: Vec<std::path::PathBuf> = omsi_map::ailists::load_list(&world.map_dir.join("drivers.txt"))
        .iter()
        .map(|l| omsi_cfg::resolve_path(&world.root, l))
        .collect();
    let path = if listed.is_empty() {
        let mut found: Vec<std::path::PathBuf> = Vec::new();
        for r in omsi_cfg::content_dirs("Humans") {
            for (group, is_dir) in omsi_cfg::vfs::list_dir(&r).unwrap_or_default() {
                if !is_dir {
                    continue;
                }
                let d = r.join(&group);
                for (n, _) in omsi_cfg::vfs::list_dir(&d).unwrap_or_default() {
                    let lower = n.to_string_lossy().to_ascii_lowercase();
                    if lower.ends_with(".hum") && lower.contains("driver") {
                        found.push(d.join(&n));
                    }
                }
            }
        }
        found.sort_by_key(|p| {
            let n = p.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
            (!n.starts_with("dbc_man04"), n)
        });
        found.into_iter().next()?
    } else {
        listed[(pick % listed.len() as u64) as usize].clone()
    };
    cached_type(&path)
}

/// A driver figure's type, read once per file.
pub fn cached_type(path: &std::path::Path) -> Option<Arc<HumanType>> {
    static TYPES: std::sync::Mutex<Option<std::collections::HashMap<std::path::PathBuf, Option<Arc<HumanType>>>>> =
        std::sync::Mutex::new(None);
    let path = path.to_path_buf();
    let mut types = TYPES.lock().unwrap_or_else(|e| e.into_inner());
    types
        .get_or_insert_with(Default::default)
        .entry(path.clone())
        .or_insert_with(|| {
            HumanType::load(&path)
                .map_err(|e| log::warn!("driver {}: {e:#}", path.display()))
                .ok()
                .map(Arc::new)
        })
        .clone()
}

/// The steering wheel: the mesh turned by `Axle_Steering_*` with the largest factor (a
/// road wheel turns by the steering angle itself, a steering wheel by 15-20 times it) that
/// lies within arm's reach of the seat.
fn find_wheel(v: &VehicleInstance, hip: Vec3) -> Option<Wheel> {
    let mut best: Option<(f32, usize, Mat4, String, f32)> = None;
    for (i, vm) in v.ty.meshes.iter().enumerate() {
        let def = &v.ty.model.meshes[vm.def_index];
        for a in &def.animations {
            if !a.variable.to_ascii_lowercase().starts_with("axle_steering")
                || a.factor.abs() < 200.0
                || a.kind != Some(omsi_model::AnimKind::Rot)
            {
                continue;
            }
            let origin = omsi_sim::anim::origin_matrix(&a.origins, vm.pivot);
            let c = origin.transform_point3(Vec3::ZERO);
            if (c - hip).length() > 1.2 {
                continue;
            }
            if best.as_ref().map(|b| a.factor.abs() > b.0).unwrap_or(true) {
                best = Some((a.factor.abs(), i, origin, a.variable.clone(), a.factor));
            }
        }
    }
    let (_, mesh, origin, var, factor) = best?;
    let origin_point = origin.transform_point3(Vec3::ZERO);
    let centre = origin_point;
    let mut axis = origin.transform_vector3(Vec3::X).normalize_or_zero();
    // the axis points at the driver: at his shoulders, not his hips - a bus's wheel lies
    // nearly flat at the height of the hips, whose direction then says nothing (the SD202's
    // axis came out pointing down and the hands held the air under the rim); a wheel that
    // lies anywhere near flat faces up
    let shoulders = hip + Vec3::Z * 0.5;
    if (axis.z.abs() > 0.4 && axis.z < 0.0) || (axis.z.abs() <= 0.4 && axis.dot(shoulders - centre) < 0.0) {
        axis = -axis;
    }
    let mut up = (Vec3::Z - axis * axis.dot(Vec3::Z)).normalize_or_zero();
    if up.length_squared() < 0.5 {
        up = (Vec3::Y - axis * axis.dot(Vec3::Y)).normalize_or_zero();
    }
    // right as the driver sees it: facing the wheel along -axis
    let right = up.cross(axis).normalize_or_zero();
    let right = if right.x < 0.0 { -right } else { right };
    // the rim: the ring the farthest vertices form round the axis (a spoke or the hub is
    // nearer; a few stray vertices are left out). An AI copy of a bus keeps no vertices on
    // the CPU: its wheel is read from the file (a guessed size left the AI drivers' hands
    // floating over the rim).
    let vm = &v.ty.meshes[mesh];
    let loaded;
    let positions: &[Vec3] = if vm.data.positions.len() >= 12 {
        &vm.data.positions
    } else {
        loaded = omsi_o3d::load_mesh(&vm.file)
            .ok()
            .map(|m| omsi_geometry::mesh_from_o3d(&m).positions)
            .unwrap_or_default();
        &loaded
    };
    let radius_of = |p: &Vec3| {
        let d = *p - centre;
        (d - axis * axis.dot(d)).length()
    };
    let mut radii: Vec<f32> = positions.iter().map(radius_of).collect();
    radii.sort_by(|a, b| a.total_cmp(b));
    // (an AI copy of a bus keeps no vertices on the CPU: a bus wheel's size then)
    let rim = if radii.len() < 12 { 0.26 } else { radii[(radii.len() as f32 * 0.93) as usize] };
    // The rim's cross-section, from the ring of its vertices (a spoke or the hub is nearer
    // the axis): the middle of the tube across and along the axis, and how thick it is. The
    // animation's origin may be anywhere on the axis, the foot of the column as often as the
    // hub; and a guessed tube - a fixed 93 % of the outer radius, 8 mm over the ring's mean
    // height - put the fingers' fist 2-3 cm beside the SD202's rim, closed round the air.
    let pct = |v: &mut Vec<f32>, f: f32| {
        v.sort_by(|a, b| a.total_cmp(b));
        v[((v.len() - 1) as f32 * f) as usize]
    };
    let ring: Vec<&Vec3> = positions.iter().filter(|p| radius_of(p) > rim * 0.8).collect();
    let (radius, along, tube) = if ring.len() >= 8 {
        let mut rs: Vec<f32> = ring.iter().map(|p| radius_of(p)).collect();
        let mut zs: Vec<f32> = ring.iter().map(|p| axis.dot(**p - origin_point)).collect();
        let (r0, r1) = (pct(&mut rs, 0.02), pct(&mut rs, 0.98));
        let (z0, z1) = (pct(&mut zs, 0.02), pct(&mut zs, 0.98));
        ((r0 + r1) * 0.5, (z0 + z1) * 0.5, ((r1 - r0).max(z1 - z0) * 0.5).clamp(0.01, 0.03))
    } else {
        (rim * 0.93, 0.0, 0.017)
    };
    let centre = origin_point + axis * along.clamp(-0.3, 0.3);
    let radius = radius.clamp(0.14, 0.32);
    log::debug!("driver: steering wheel rim {radius:.3} m round the axis, tube {tube:.3} m thick (radius)");
    Some(Wheel { mesh, var, factor, axis, centre, up, right, radius, tube })
}
