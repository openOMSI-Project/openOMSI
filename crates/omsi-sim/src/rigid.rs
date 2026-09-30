//! Rigid-body vehicle dynamics after Omsi.exe's own (0x7e2574; OMSI uses ODE only for the
//! objects a crash knocks over): a six-degree-of-freedom body with mass,
//! `[momentofintertia]` and `[schwerpunkt]`, wheels on spring/damper suspensions
//! (`achse_feder`, `achse_daempfer`, `achse_maxforce`, measured where the tyres stand
//! between `achse_minwidth` and `achse_maxwidth`, pushing from `achse_maxwidth`), every axle
//! steered towards the turning centre on `[rot_pnt_long]` with `[inv_min_turnradius]` as the
//! full lock, OMSI's grip across the tyres (the bus follows its wheels until the bend asks
//! more than the road's grip, then slides at mu x load), drive torque and brake forces from
//! the scripts, pitch and roll damped as OMSI damps them, gravity, ground contact per wheel
//! and impulse responses against obstacles. `examples/handling.rs` measures what a `.bus`
//! file makes of a bus.
//!
//! Ride height. A `.bus` file has no height for its axles: OMSI hangs each wheel so that
//! its spring is *unloaded* when the tyre stands on the model's ground plane (z = 0), and
//! the body then sinks by what the load compresses the springs - mass share over
//! `achse_feder` times the scripts' `Axle_Springfactor`. That sag is the `ai_deltaheight`
//! the same files give for their AI copies (-0.10 m for the SD202 and the NL202, whose
//! springs take 22-28 kN at 240-280 kN/m; -0.15 m for the F90 lorry, 55 kN at 370 kN/m),
//! and it is what `Axle_Suspension` reports: the model lifts the wheel into its arch by the
//! compression. Every vehicle therefore sits at its own height, set by its own data - the
//! NL202's level control even pumps its bellows until the compression reads 0.105 m.

use crate::collision::Obb;
use glam::{DVec2, DVec3, Quat, Vec2, Vec3};
use omsi_vehicle::Vehicle;

/// How far a wheel may hang below its unloaded position (m).
pub const DROOP: f32 = 0.15;
/// Compression at which the bump stop takes over (m). A deflated air suspension rests on it.
pub const BUMP: f32 = 0.24;
/// The wheel between the road and the strut: its share of the corner's mass (axle, hub,
/// brake, tyre - about an eighth on a bus), and the tyre's vertical stiffness (N/m) and
/// damping (N s/m). Taken as massless, the wheel stood wherever the road put it: a bump
/// shoved it into the arch within a substep, the strut's damper kicked the body into the
/// air, and the wheel then dropped to its full droop at once - the bus hopped over a
/// manhole cover with its wheels flicking in and out of the body.
const UNSPRUNG: f32 = 0.12;
const TYRE_K: f32 = 900_000.0;
const TYRE_C: f32 = 3_000.0;
/// The frame OMSI's per-frame damping of the body's pitch and roll is measured in (see
/// `RigidBus::step`): a thirtieth of a second, the rate its options.cfg caps OMSI at.
const OMSI_FRAME: f32 = 1.0 / 30.0;

/// The suspension as Omsi.exe has it (see `step_slice`); `OMSI_TYRE_SUSPENSION=1` gives
/// the old one with a wheel mass, a tyre and bump stops (A/B).
fn omsi_suspension() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| omsi_cfg::env::var_os("OMSI_TYRE_SUSPENSION").is_none())
}

/// What is left of the body's pitch and roll rate after `dt` seconds of OMSI's damping.
fn body_damping(body_freq: f32, dt: f32) -> f32 {
    let x0 = 1.5 * body_freq * OMSI_FRAME;
    if x0 <= 1e-6 || dt <= 0.0 {
        return 1.0;
    }
    (x0.sin() / x0).clamp(0.1, 1.0).powf(dt / OMSI_FRAME)
}

/// OMSI has no tyre between the road and the spring: `achse_feder` alone carries the body
/// (Omsi.exe 0x7e4960..0x7e4c40, force = feder x penetration - daempfer x speed). The
/// tyre here is at least this many times as stiff as the spring, so that the two in series
/// give the body 94 % of the file's spring rate instead of the 79 % a fixed 900 kN/m tyre
/// left a bus with 240 kN/m springs - that softness was a good part of its sway.
const TYRE_OVER_SPRING: f32 = 15.0;
/// Tyre samples along the envelope's length (see [`tyre_line`]).
const TYRE_SAMPLES: usize = 13;
/// Samples taken: the lattice they stand on covers the envelope with one more to spare, and
/// two more beyond each end tell the edges near the ends from the grade.
const LATTICE: usize = TYRE_SAMPLES + 5;
/// How far the tyre reaches for the ground ahead and behind, in radii. A pneumatic tyre
/// wraps round an edge and climbs it over about twice the length a rigid disc would: a
/// 15 cm kerb then costs about half the wheel load in push, not all of it, and a lowered
/// kerb of 3 cm is a nudge instead of the wall a rigid disc makes of it.
const ENVELOPE: f32 = 2.0;
/// How far (m) the rise between two samples may depart from the rises beside it before it
/// is an edge rather than the road's own grade or curve.
const EDGE: f64 = 0.01;
/// Halvings that place an edge between two samples (to a 64th of their spacing) and a face
/// the tyre cannot climb (to a 128th of the bracket).
const EDGE_HALVINGS: usize = 6;
const FACE_HALVINGS: usize = 7;
/// The highest step a tyre climbs, in radii over the ground it stands on; anything higher
/// is a wall. A step of the full radius would only give way to an infinite push (and the
/// hub and 5 cm, climbed before, took a bus over a 0.52 m step from 10 km/h). Up to 0.8 the
/// steepest slope the envelope meets (2.4) is still one the push takes in full (see
/// `step`), so a climb costs exactly the lift; and it keeps the stock maps' 30 cm gaps
/// between a side road and its junction something a bus drives over.
pub const CLIMB: f32 = 0.8;
/// Half the width of a tyre, in radii (a 275 mm bus tyre on a 0.47 m radius).
const HALF_WIDTH: f32 = 0.3;
/// How far beside the tyre (in half widths from the hub) a face is looked for.
const SIDE_REACH: f32 = 2.0;
/// How far (m) either side of a face's first point its direction is measured.
const FACE_SPAN: f32 = 0.2;
/// A face this far above the axle next to the tyre is a wall, not something far overhead.
const WALL_HEIGHT: f64 = 1.5;
/// Stiffness (N/m) and damping (N s/m) of a tyre pressed into a wall it cannot climb.
const WALL_K: f32 = 1.5e6;
const WALL_C: f32 = 6.0e4;
/// Share of the closing speed an obstacle gives back, and the sliding friction along it.
const RESTITUTION: f32 = 0.2;
const SCRAPE_FRICTION: f32 = 0.4;
/// Below this closing speed (m/s) a touch is a push, not a crash.
pub const CRASH_SPEED: f32 = 0.4;

/// The ground a wheel finds at one point: the highest face at or below the probe's top and
/// the lowest face above it (world heights).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GroundProbe {
    pub below: Option<f64>,
    pub above: Option<f64>,
}

/// What the wheels of a vehicle stand on.
pub trait Ground: Send + Sync {
    /// The faces at world (x, y): the highest at or below `top` and the lowest above it.
    fn probe(&self, x: f64, y: f64, top: f64) -> GroundProbe;

    /// A prober for one step of one vehicle - many points close together. It may keep what
    /// it looked up (the tiles under the vehicle) until it is dropped.
    fn session(&self) -> Box<dyn Fn(f64, f64, f64) -> GroundProbe + '_> {
        Box::new(move |x, y, top| self.probe(x, y, top))
    }
}

impl<F: Fn(f64, f64, f64) -> GroundProbe + Send + Sync> Ground for F {
    fn probe(&self, x: f64, y: f64, top: f64) -> GroundProbe {
        self(x, y, top)
    }
}

/// A face a tyre cannot climb, kept while the tyre is up against it: the samples that found
/// it are lost as soon as the hub is over it, and a wheel pushed that far went on through.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WallFace {
    /// A point of the face (world, ground plane) and its normal, towards the tyre.
    pub point: DVec2,
    pub normal: Vec2,
}

impl WallFace {
    /// How far the hub stands off the face (negative once it is past it).
    fn distance(&self, hub: DVec3) -> f32 {
        (hub.truncate() - self.point).dot(self.normal.as_dvec2()) as f32
    }

    /// How far a tyre of radius `r` rolling along `fwd_h` reaches from its hub towards the
    /// face: its radius straight ahead, half its width to the side.
    fn reach(&self, fwd_h: Vec3, r: f32) -> f32 {
        let n = self.normal.extend(0.0);
        let right_h = Vec3::new(fwd_h.y, -fwd_h.x, 0.0);
        r * n.dot(fwd_h).abs() + HALF_WIDTH * r * n.dot(right_h).abs()
    }
}

#[derive(Debug, Clone)]
pub struct RigidWheel {
    /// The wheel's own turning (rad/s) and its state as OMSI keeps it:
    /// gripping (turning with the ground), slipping (spinning or skidding: the tyre passes on
    /// the sliding friction and the wheel turns by its own torques), locked (a brake that
    /// stopped it). `inertia_inv`: the axle's `achse_inertia_inv` (0.002 when not given).
    pub spin: f32,
    pub slipping: bool,
    pub locked: bool,
    pub inertia_inv: f32,
    /// Hub in the body frame (x right, y forward, z up, relative to the model origin) with
    /// the spring unloaded: the model's wheel centre when `Axle_Suspension` reads zero.
    pub attach: Vec3,
    /// How much farther out than the hub the strut's force acts (m, signed like `attach.x`).
    /// OMSI measures the spring's travel where the tyres stand - halfway between
    /// `achse_minwidth` and `achse_maxwidth`, (max + min) / 4 from the middle - but turns the
    /// body with the force at `achse_maxwidth` / 2 (Omsi.exe 0x7e47b3 and 0x7e4bad).
    pub lever: f32,
    pub radius: f32,
    pub driven: bool,
    pub steered: bool,
    /// The wheel's steering angle this step (rad, positive to the right): every axle points
    /// at the turning centre on the `[rot_pnt_long]` line, atan((long - rot_pnt_long) x
    /// curvature) (Omsi.exe 0x7e3060), so an axle behind that line steers the other way.
    pub steer: f32,
    /// The tyre's vertical stiffness and damping (N/m, N s/m).
    pub tyre_k: f32,
    pub tyre_c: f32,
    /// Spring rate (N/m), damper rate (N s/m) and the load the strut is built for (N).
    pub spring: f32,
    pub damper: f32,
    pub max_force: f32,
    /// `Axle_Springfactor_*`: air suspensions scale the spring with the bellows pressure.
    pub spring_factor: f32,
    /// Static load at rest (N), from the mass split over the axles.
    pub rest_load: f32,
    /// Current compression (m, positive = the wheel pushed up into the body) and its rate.
    pub compression: f32,
    pub compression_rate: f32,
    /// The compression at which the tyre would just touch the ground, last substep (None
    /// in the air or right after [`RigidBody::place`]).
    pub touch: Option<f32>,
    pub on_ground: bool,
    pub rotation_deg: f32,
    pub rpm: f32,
    /// Load on the tyre this step (N).
    pub load: f32,
    /// World height of the ground the tyre stands on this step (the surface under the hub
    /// or the edge it is climbing).
    pub ground_z: f64,
    /// `ground_z` came from the ground (not yet after [`RigidBody::place`]): steps up to
    /// [`CLIMB`] radii over it are ground, higher ones walls.
    pub ground_seen: bool,
    /// Faces the tyre is up against.
    pub walls: Vec<WallFace>,
    /// Horizontal push (N) of a step the tyre climbs or a wall it stands against.
    pub step_force: f32,
}

fn sign(x: f32) -> f32 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

impl RigidWheel {
    /// One substep of the wheel's turning:
    /// a gripping wheel turns with the ground and starts to slip when what it has to pass
    /// on (`f_long`, before the friction circle cut it) is more than the road takes; a
    /// slipping one turns by its drive, its brake and the sliding friction, locks when the
    /// brake stops it, and grips again when its speed crosses the ground's.
    fn advance_spin(&mut self, c: &Contact, f_long_wanted: f32, friction: f32, h: f32) {
        let r = self.radius.max(0.05);
        let ground = c.v_long / r;
        let mu_n = friction * c.grip;
        // (`OMSI_NO_WHEEL_SLIP=1`: every wheel grips, as before the wheels had a turning of
        // their own - for comparison)
        let no_slip = omsi_cfg::env::var_os("OMSI_NO_WHEEL_SLIP").is_some();
        if no_slip {
            self.slipping = false;
            self.locked = false;
        }
        if !self.slipping {
            let wanted = if c.v_long.abs() > STANDING { c.drive - c.brake * c.v_long.signum() } else { f_long_wanted };
            if wanted.abs() > mu_n * 1.02 && mu_n > 0.0 && !no_slip {
                self.slipping = true;
                self.locked = false;
                self.spin = ground;
            } else {
                self.spin = ground;
            }
        }
        if self.slipping {
            let slide_before = self.spin * r - c.v_long;
            if self.locked {
                // it turns again when the road's pull and the drive beat the brake
                if (sign(-c.v_long) * mu_n - c.drive).abs() > c.brake {
                    self.locked = false;
                } else {
                    self.spin = 0.0;
                }
            }
            if !self.locked {
                let before = self.spin;
                self.spin += h * self.inertia_inv * (c.drive - sign(self.spin) * c.brake - sign(slide_before) * mu_n);
                if before != 0.0 && sign(before) != sign(self.spin) && c.brake > 0.0 {
                    self.spin = 0.0;
                    self.locked = true;
                }
            }
            let slide_after = self.spin * r - c.v_long;
            // gripping again: the wheel's speed crossed the ground's (or both stand)
            // (a slide that only now begins - zero before - has crossed nothing)
            if sign(slide_after) * sign(slide_before) < 0.0 || (c.v_long.abs() < STANDING && self.spin.abs() * r < STANDING && c.drive.abs() <= c.brake.max(mu_n)) {
                self.slipping = false;
                self.locked = false;
                self.spin = ground;
            }
        }
        self.rpm = self.spin * 60.0 / std::f32::consts::TAU;
        self.rotation_deg = (self.rotation_deg + self.spin.to_degrees() * h).rem_euclid(360.0);
    }

    /// Compression the static load gives with the current spring factor.
    pub fn rest_compression(&self) -> f32 {
        self.rest_load / (self.spring * self.spring_factor.max(0.05))
    }
}

/// The static load on each wheel of each axle (N): the weight shared so that the axle loads
/// balance the centre of gravity (a least-squares split when there are more than two axles).
pub fn wheel_rest_loads(def: &Vehicle) -> Vec<f32> {
    let mass = if def.mass < 100.0 { def.mass * 1000.0 } else { def.mass }.max(500.0);
    let cog_y = def.cog.map(|c| c[1]).unwrap_or(0.0);
    let g = 9.81;
    let n = (def.axles.len() * 2).max(1) as f32;
    let sy: f32 = def.axles.iter().map(|a| 2.0 * a.long).sum();
    let syy: f32 = def.axles.iter().map(|a| 2.0 * a.long * a.long).sum();
    let det = n * syy - sy * sy;
    let (a, b) = if det.abs() > 1e-6 {
        let rhs1 = mass * g;
        let rhs2 = mass * g * cog_y;
        ((rhs1 * syy - sy * rhs2) / det, (n * rhs2 - sy * rhs1) / det)
    } else {
        (mass * g / n, 0.0)
    };
    def.axles.iter().map(|x| (a + b * x.long).max(mass * g / n * 0.2)).collect()
}

/// A wheel on the ground in one substep, before its tyre forces are settled.
struct Contact {
    /// Contact point relative to the centre of gravity, tyre forward and right (world).
    r: Vec3,
    fwd: Vec3,
    right: Vec3,
    /// Suspension, step and wall forces (world).
    base: Vec3,
    drive: f32,
    /// What the brake (and the rolling resistance) can hold.
    brake: f32,
    grip: f32,
    v_long: f32,
    /// Takes side forces (a coupled part's joint does not).
    lateral: bool,
    /// The wheel it is (None: a coupled part's joint).
    wheel: Option<usize>,
}

/// `achse_inertia_inv` where a `[newachse]` does not say (OMSI LoadFromFile: 0.002).
pub const DEFAULT_INERTIA_INV: f32 = 0.002;

/// Below this speed (m/s) at the contact a tyre stands and its brake holds statically.
const STANDING: f32 = 0.15;

/// One hit of the body against an obstacle.
#[derive(Debug, Clone, Copy)]
pub struct Impact {
    /// Where it hit, in the body frame relative to the model origin (x right, y forward, z up).
    pub point: Vec3,
    /// Closing speed along the contact normal (m/s).
    pub speed: f32,
    /// Kinetic energy the impact took away (J).
    pub energy: f32,
    /// Index into the obstacle list the body was tested against; for a wheel stopped by a
    /// face (`RigidBody::wheel_impacts`), the wheel's.
    pub obstacle: usize,
    /// A `[crashmode_pole]` obstacle that broke off.
    pub broke: bool,
    /// Direction the obstacle was pushed (world, ground plane).
    pub push: Vec3,
}

/// A part towed behind the body (the rear section of an articulated bus). The parts follow
/// kinematically, so what acts on them along their length arrives at the joint: the drive
/// of a driven axle - the O530G/GL Facelift and the stock GN92 are pushers, whose front
/// sections roll free -, the brakes and rolling resistance, the weight on a grade and the
/// mass. The body takes the part of it along its own axis (cosine of the articulation
/// angle); the side part is what a pusher's joint damping and the rear section's tyres
/// hold. Handed on whole, it turned a pusher standing against its brakes round on the spot
/// and drove every bend into a jack-knife: the kinematic rear section has no tyres of its
/// own to resist it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoupledPart {
    /// The coupling on this body (body frame, relative to the model origin).
    pub point: Vec3,
    /// The direction of travel of the part hung on that coupling (world).
    pub dir: Vec3,
    /// Mass (kg).
    pub mass: f32,
    /// Driven wheels, their radius (m) and the static load on them (N): they pass on no
    /// more than friction times that load.
    pub driven_wheels: usize,
    pub radius: f32,
    pub driven_load: f32,
    /// What its brakes and its rolling resistance hold (N), and the static load on all its
    /// wheels (N).
    pub brake: f32,
    pub load: f32,
}

#[derive(Debug, Clone)]
pub struct RigidBody {
    pub mass: f32,
    /// Body-frame inertia (diagonal, kg m²).
    pub inertia: Vec3,
    /// Centre of gravity relative to the model origin (m).
    pub cog: Vec3,
    /// World position of the centre of gravity.
    pub position: DVec3,
    pub orientation: Quat,
    pub velocity: Vec3,
    /// Angular velocity in the body frame (rad/s).
    pub omega: Vec3,
    pub wheels: Vec<RigidWheel>,
    /// Axle index of each wheel (two per axle, left then right).
    pub wheel_axle: Vec<usize>,
    pub steer_deg: f32,
    pub max_steer_deg: f32,
    /// `[rot_pnt_long]` and `[inv_min_turnradius]`: the line the bus turns about and the
    /// curvature of the full lock.
    pub rot_pnt_long: f32,
    pub inv_min_turn_radius: f32,
    /// sqrt(sum of the springs / mass) (1/s): OMSI damps the body's pitch and roll by
    /// sin(x)/x a frame with x = 1.5 of it times the frame (Omsi.exe 0x7e4f55).
    pub body_freq: f32,
    /// OMSI's two states of the tyres across (Omsi.exe `+0x1d8`): holding - the bus follows
    /// its wheels however quickly they are turned, as long as the bend asks no more than
    /// the grip of all its tyres and no wheel spins or locks - or sliding, each tyre giving
    /// at most its own grip until the sideways speed at every tyre is under 0.1 m/s again.
    pub holding: bool,
    pub rolling_resistance: f32,
    /// Body-frame acceleration of the last step (m/s²), for `A_Trans_*`.
    pub accel_body: Vec3,
    pub friction: f32,
    /// Wheels a face stopped during the last step, one entry per wheel.
    pub wheel_impacts: Vec<Impact>,
    /// The parts coupled behind (see [`CoupledPart`]); `drive_torque` is shared by their
    /// driven wheels and the body's own.
    pub coupled: Vec<CoupledPart>,
    /// Obstacles (by id) the body was put down inside: left alone until it is out of them
    /// (a bus spawned in a shelter's or a depot hall's box was held there for good - every
    /// move towards the nearest side pushed it back in). `None` until the first collision
    /// check after `place`.
    pub spawned_inside: Option<Vec<i64>>,
    /// The faces a wheel cannot climb stop it (a platform's edge, a kerb too high for it).
    /// Off with the collisions with objects: the wheels then keep to the ground under them,
    /// as OMSI's do - its wheels know no faces - and a step nobody sees (a height profile
    /// over the drawn road, a helper object's collision box) is no invisible wall.
    pub wheel_walls: bool,
}

impl RigidBody {
    /// `hub_heights[a]`: unloaded hub height of axle `a` above the model origin; the tyre
    /// radius when the model does not say (see `VehicleType::wheel_geometry`).
    pub fn from_definition(def: &Vehicle, hub_heights: &[Option<f32>]) -> RigidBody {
        let mass = if def.mass < 100.0 { def.mass * 1000.0 } else { def.mass }.max(500.0);
        let moi = def.moment_of_inertia;
        let scale = if moi[0] < 5000.0 { 1000.0 } else { 1.0 };
        // `[momentofintertia]` goes to ODE as I11, I22, I33 of Omsi.exe's y-up body frame
        // (0x7af0a4): about the lateral axis (pitch), the vertical one (yaw) and the
        // longitudinal one (roll) - roll takes the third value (0x7e4ef3), yaw the second
        // (0x7e5110), whatever the SDK's comment says. Here (right, forward, up): pitch,
        // roll, yaw. Read in the comment's order the SD202 rolled on 80 t m² instead of
        // 300, twice as fast, and every uneven patch rocked it like a boat.
        let inertia = Vec3::new((moi[0] * scale).max(100.0), (moi[2] * scale).max(100.0), (moi[1] * scale).max(100.0));
        let cog_xy = def.cog.map(|c| (c[0], c[1])).unwrap_or((0.0, 0.0));
        let cog_z = if def.cog_height > 0.0 { def.cog_height } else { def.cog.map(|c| c[2]).filter(|z| *z > 0.0).unwrap_or(1.0) };
        let cog = Vec3::new(cog_xy.0, cog_xy.1, cog_z);
        let front_long = def.axles.iter().map(|a| a.long).fold(f32::MIN, f32::max);
        let mut wheels = Vec::new();
        let mut wheel_axle = Vec::new();
        for (ai, a) in def.axles.iter().enumerate() {
            let r = (a.wheel_diameter / 2.0).max(0.15);
            let z = hub_heights.get(ai).copied().flatten().unwrap_or(r);
            let k = if a.spring > 0.0 { a.spring * 1000.0 } else { 150_000.0 };
            let c = if a.damper > 0.0 { a.damper * 1000.0 } else { 12_000.0 };
            let max_force = if a.max_force > 0.0 { a.max_force * 1000.0 } else { 200_000.0 };
            // where the tyres stand across (the middle of the band between the two widths)
            // and where the strut pushes the body (the outer width)
            let outer = (a.max_width / 2.0).max(0.3);
            let inner = if a.min_width > 0.0 && a.min_width < a.max_width { (a.max_width + a.min_width) / 4.0 } else { outer * 0.85 };
            let tyre_k = TYRE_K.max(k * TYRE_OVER_SPRING);
            let tyre_c = TYRE_C * (tyre_k / TYRE_K).sqrt();
            for side in [-1.0f32, 1.0] {
                let inertia_inv = if a.inertia_inv > 0.0 { a.inertia_inv } else { DEFAULT_INERTIA_INV };
                wheels.push(RigidWheel { spin: 0.0, slipping: false, locked: false, inertia_inv, attach: Vec3::new(side * inner, a.long, z), lever: side * (outer - inner), radius: r, driven: a.driven, steered: (a.long - front_long).abs() < 0.01, steer: 0.0, tyre_k, tyre_c, spring: k, damper: c, max_force, spring_factor: 1.0, rest_load: 0.0, compression: 0.0, compression_rate: 0.0, touch: None, on_ground: true, rotation_deg: 0.0, rpm: 0.0, load: 0.0, ground_z: 0.0, ground_seen: false, walls: Vec::new(), step_force: 0.0 });
                wheel_axle.push(ai);
            }
        }
        let loads = wheel_rest_loads(def);
        for (w, a) in wheels.iter_mut().zip(&wheel_axle) {
            w.rest_load = loads.get(*a).copied().unwrap_or(0.0);
        }
        let s = (front_long - def.rot_pnt_long).abs().max(1.0);
        let max_steer_deg = (def.inv_min_turn_radius * s).atan().to_degrees().clamp(10.0, 60.0);
        // a file without a usable `[inv_min_turnradius]` still steers its front axle as far
        // as the old default lock
        let inv_min_turn_radius = if def.inv_min_turn_radius > 0.0 { def.inv_min_turn_radius } else { max_steer_deg.to_radians().tan() / s };
        let springs: f32 = def.axles.iter().map(|a| 2.0 * if a.spring > 0.0 { a.spring } else { 150.0 }).sum();
        let body_freq = (springs / (mass / 1000.0)).max(0.0).sqrt();
        RigidBody { mass, inertia, cog, position: DVec3::ZERO, orientation: Quat::IDENTITY, velocity: Vec3::ZERO, omega: Vec3::ZERO, wheels, wheel_axle, steer_deg: 0.0, max_steer_deg, rot_pnt_long: def.rot_pnt_long, inv_min_turn_radius, body_freq, holding: true, rolling_resistance: if def.rolling_resistance > 0.0 { def.rolling_resistance } else { 0.008 * mass * 9.81 }, accel_body: Vec3::ZERO, friction: 0.85, wheel_impacts: Vec::new(), coupled: Vec::new(), spawned_inside: None, wheel_walls: true }
    }

    /// Place the body at rest with its wheels on the ground plane at `origin.z`: heading
    /// `heading_deg`, springs already carrying the load (no drop and bounce at the start).
    pub fn place(&mut self, origin: DVec3, heading_deg: f64) {
        self.orientation = Quat::from_rotation_z((-heading_deg).to_radians() as f32);
        let n = self.wheels.len().max(1) as f32;
        let sag: f32 = self.wheels.iter().map(|w| w.radius - w.attach.z - w.rest_compression().min(BUMP)).sum::<f32>() / n;
        let origin = origin + DVec3::Z * sag as f64;
        self.position = origin + self.orientation.mul_vec3(self.cog).as_dvec3();
        self.velocity = Vec3::ZERO;
        self.omega = Vec3::ZERO;
        self.spawned_inside = None;
        for w in self.wheels.iter_mut() {
            w.compression = w.rest_compression().min(BUMP);
            w.compression_rate = 0.0;
            w.touch = None;
        }
        // standing: the ground holds the body up against gravity
        self.accel_body = Vec3::new(0.0, 0.0, 9.81);
        for w in self.wheels.iter_mut() {
            w.ground_seen = false;
            w.walls.clear();
        }
    }

    /// Model origin (the point the renderer places) in the world.
    pub fn origin(&self) -> DVec3 {
        self.position - self.orientation.mul_vec3(self.cog).as_dvec3()
    }

    /// Heading (deg, clockwise from north), pitch and bank (deg) of the body.
    pub fn heading_pitch_bank(&self) -> (f64, f32, f32) {
        let f = self.orientation.mul_vec3(Vec3::Y);
        let r = self.orientation.mul_vec3(Vec3::X);
        let heading = (f.x.atan2(f.y) as f64).to_degrees().rem_euclid(360.0);
        let pitch = f.z.clamp(-1.0, 1.0).asin().to_degrees();
        let bank = (-r.z).clamp(-1.0, 1.0).asin().to_degrees();
        (heading, pitch, bank)
    }

    /// Forward speed (m/s) in the body frame.
    pub fn forward_speed(&self) -> f32 {
        self.velocity.dot(self.orientation.mul_vec3(Vec3::Y))
    }

    pub fn kinetic_energy(&self) -> f32 {
        0.5 * self.mass * self.velocity.length_squared() + 0.5 * (self.inertia * self.omega * self.omega).element_sum()
    }

    /// Inverse of the effective mass the body shows along world direction `n` at world
    /// offset `r` from its centre of gravity.
    fn inv_mass_at(&self, r: Vec3, n: Vec3) -> f32 {
        let inv = self.orientation.inverse();
        let (rb, nb) = (inv.mul_vec3(r), inv.mul_vec3(n));
        let w = rb.cross(nb) / self.inertia;
        1.0 / self.mass + w.cross(rb).dot(nb)
    }

    /// Apply the world impulse `p` at world offset `r` from the centre of gravity.
    fn apply_impulse(&mut self, r: Vec3, p: Vec3) {
        let inv = self.orientation.inverse();
        self.velocity += p / self.mass;
        self.omega += inv.mul_vec3(r).cross(inv.mul_vec3(p)) / self.inertia;
    }

    /// The body's `[boundingbox]` in the world: its footprint turned with the heading, its
    /// height range over all eight corners (a pitched bus reaches lower at one end).
    pub fn body_box(&self, bb: [f32; 6]) -> Obb {
        let (heading, _, _) = self.heading_pitch_bank();
        let origin = self.origin();
        let mut o = Obb::from_box(bb, origin, heading);
        let centre = origin + self.orientation.mul_vec3(Vec3::new(bb[3], bb[4], bb[5])).as_dvec3();
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for sx in [-0.5f32, 0.5] {
            for sy in [-0.5f32, 0.5] {
                for sz in [-0.5f32, 0.5] {
                    let z = centre.z + self.orientation.mul_vec3(Vec3::new(bb[0] * sx, bb[1] * sy, bb[2] * sz)).z as f64;
                    lo = lo.min(z);
                    hi = hi.max(z);
                }
            }
        }
        o.z0 = lo;
        o.z1 = hi;
        o
    }

    /// The body's `[boundingbox]` as it stands, turned, pitched and banked, grown by
    /// `margin` on every side.
    pub fn body_solid(&self, bb: [f32; 6], margin: f32) -> crate::collision::Box3 {
        let r = self.orientation;
        let centre = self.origin() + r.mul_vec3(Vec3::new(bb[3], bb[4], bb[5])).as_dvec3();
        crate::collision::Box3 {
            center: centre,
            axes: [r.mul_vec3(Vec3::X).as_dvec3(), r.mul_vec3(Vec3::Y).as_dvec3(), r.mul_vec3(Vec3::Z).as_dvec3()],
            half: (Vec3::new(bb[0], bb[1], bb[2]) * 0.5 + Vec3::splat(margin)).as_dvec3(),
        }
    }

    /// One step. `drive_torque` (N m at the driven wheels, from `M_Wheel`), `brake` per
    /// wheel (N), `steer` −1..1, `probe(x, y, z_top)` the ground at a point.
    pub fn step(&mut self, dt: f32, drive_torque: f32, brake: &[f32], steer: f32, probe: &dyn Fn(f64, f64, f64) -> GroundProbe) {
        // a long frame (below 20 fps) is stepped in slices of at most 50 ms, so that the
        // body covers the whole frame the clock, the odometer and the scripts count (it was
        // cut to 50 ms: at 10 fps the bus went half as far as the time went on)
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.25) } else { 0.0 };
        let slices = (dt / 0.05).ceil().max(1.0) as usize;
        let mut impacts = Vec::new();
        for _ in 0..slices {
            self.step_slice(dt / slices as f32, drive_torque, brake, steer, probe);
            // OMSI damps the body's pitch and roll - never its yaw - by sin(x)/x each frame,
            // x = 1.5 x sqrt(springs / mass) x frame (Omsi.exe 0x7e4f55..0x7e50b0). Its
            // frames are OMSI's (its options.cfg caps them at 30 a second): taken per frame
            // of this game, a machine drawing 150 frames a second damped the body a fifth as
            // much, and the bus rocked on its springs like a boat. The same damping a second
            // as OMSI's at 30 frames, however many are drawn:
            let k = body_damping(self.body_freq, dt / slices as f32);
            self.omega.x *= k;
            self.omega.y *= k;
            // (one impact per obstacle a frame, as a single slice gives)
            for m in self.wheel_impacts.drain(..) {
                match impacts.iter_mut().find(|x: &&mut Impact| x.obstacle == m.obstacle) {
                    Some(x) if x.energy >= m.energy => {}
                    Some(x) => *x = m,
                    None => impacts.push(m),
                }
            }
        }
        self.wheel_impacts = impacts;
    }

    fn step_slice(&mut self, dt: f32, drive_torque: f32, brake: &[f32], steer: f32, probe: &dyn Fn(f64, f64, f64) -> GroundProbe) {
        // The steering sets the curvature the bus turns on (`steer` 1 = the full lock of
        // `[inv_min_turnradius]`) and every axle points at the centre of that turn on the
        // `[rot_pnt_long]` line, as OMSI does it. How fast the curvature may change is the
        // input's business (keys, mouse, wheel), not the axle's: the old fixed rate here
        // lagged every mouse and controller movement by up to 0.4 s.
        let kappa = steer.clamp(-1.0, 1.0) * self.inv_min_turn_radius;
        let front = self.wheels.iter().map(|w| w.attach.y).fold(f32::MIN, f32::max);
        for w in self.wheels.iter_mut() {
            // each tyre square to the line from the centre of the turn to itself - the inner
            // one turned further than the outer (Ackermann). With both at the axle's angle the
            // tyres of an axle pulled against each other through the side constraint: the
            // scrub ate the drive, 60 % steering left a twentieth of the push and full lock
            // none ("the engine revs but the bus gets slower the more I steer")
            let across = (1.0 - w.attach.x * kappa).max(0.2);
            w.steer = ((w.attach.y - self.rot_pnt_long) * kappa / across).atan().clamp(-1.05, 1.05);
            w.steered = w.steer != 0.0;
        }
        self.steer_deg = ((front - self.rot_pnt_long) * kappa).atan().clamp(-1.05, 1.05).to_degrees();
        // (substeps of at most ~4 ms whatever the frame: a frame held up by loading - 50 ms -
        // made 12 ms substeps, too long for the stiff tyres, and the body hopped on its
        // springs for no reason the driver could see)
        let substeps = ((dt / 0.0042).ceil() as usize).clamp(4, 16);
        let h = dt / substeps as f32;
        let mut accel_sum = Vec3::ZERO;
        self.wheel_impacts.clear();
        for _ in 0..substeps {
            let rot = self.orientation;
            let up = rot.mul_vec3(Vec3::Z);
            let mut force = Vec3::new(0.0, 0.0, -9.81 * self.mass);
            // the air: ½ ρ cw A v² (cw 0.6, the front a bus's or a car's by the mass). Without it
            // a bus rolling downhill without its brakes ran on to any speed
            {
                let v = self.velocity;
                let area = (self.mass / 2200.0).clamp(2.0, 8.5);
                force -= v * v.length() * (0.5 * 1.2 * 0.6 * area);
            }
            let mut torque = Vec3::ZERO; // body frame
            // Every driven wheel of the train takes its share of `M_Wheel`. A body without
            // driven wheels of its own (a pusher's front section) was pushed by nothing: the
            // count was clamped to one and the torque went to wheels that do not drive.
            let coupled_wheels: usize = self.coupled.iter().map(|d| d.driven_wheels).sum();
            let driven = (self.wheels.iter().filter(|w| w.driven).count() + coupled_wheels).max(1) as f32;
            // the towed mass moves with the body along its length
            let body_fwd = rot.mul_vec3(Vec3::Y);
            let towed: f32 = self.coupled.iter().map(|d| d.mass).sum();
            let n_wheels = self.wheels.len().max(1) as f32;
            // tyre plane (forward turned by the wheel's steering angle), the hub with the
            // spring unloaded and where it is now
            let (position, cog) = (self.position, self.cog);
            let tyre = move |w: &RigidWheel| {
                let ang = w.steer;
                let fwd = rot.mul_vec3(Vec3::new(ang.sin(), ang.cos(), 0.0));
                let right = rot.mul_vec3(Vec3::new(ang.cos(), -ang.sin(), 0.0));
                let hub0 = position + rot.mul_vec3(w.attach - cog).as_dvec3();
                let hub = hub0 + (up * w.compression.max(-DROOP)).as_dvec3();
                (fwd, right, Vec3::new(fwd.x, fwd.y, 0.0).normalize_or(Vec3::Y), hub0, hub)
            };
            // What each tyre finds under and around it: the hub height it needs, how steeply
            // that rises as the wheel rolls on, and the faces it cannot climb. Steps up to
            // CLIMB radii over the ground it stood on are ground; the first step after
            // `place` (which knows no ground) takes anything up to the hub, and so does a
            // tyre that finds nothing lower, to lift a body set down a little too low.
            let mut found: Vec<Option<(f64, f32, f32)>> = Vec::with_capacity(self.wheels.len());
            for i in 0..self.wheels.len() {
                let w = &self.wheels[i];
                let (_, _, fwd_h, _, hub) = tyre(w);
                let r = w.radius;
                let z_top = if w.ground_seen { w.ground_z + (CLIMB * r) as f64 } else { hub.z + 0.05 };
                let wall_top = hub.z + WALL_HEIGHT;
                // faces are looked for as far ahead as the hub moves in a substep, so that
                // the first touch is already answered before the tyre is in the face
                let v_hub = self.velocity + rot.mul_vec3(self.omega).cross((hub - position).as_vec3());
                let look = Vec2::new(v_hub.dot(fwd_h).abs(), v_hub.dot(Vec3::new(fwd_h.y, -fwd_h.x, 0.0)).abs()) * h;
                let line = tyre_line(probe, hub, fwd_h, r, z_top, wall_top, look.x);
                let ground = match line.ground {
                    None if w.ground_seen => tyre_line(probe, hub, fwd_h, r, hub.z + 0.05, wall_top, look.x).ground,
                    g => g,
                };
                // across the tread: a kerb under part of it (see `tread_step`)
                let ground = ground.map(|(need, slope, x)| (need + tread_step(probe, hub, fwd_h, r, z_top), slope, x));
                let walls = if self.wheel_walls { track_walls(probe, &w.walls, &line, hub, fwd_h, r, z_top, wall_top, look.y) } else { Vec::new() };
                found.push(ground);
                let w = &mut self.wheels[i];
                w.walls = walls;
                w.step_force = 0.0;
            }
            // A face stops the hub at once: whatever would close on it within the substep is
            // taken away by an impulse at the hub, as the body's own collisions do (and it is
            // a crash like theirs). A spring alone gave way - a front wheel met at 30 km/h
            // went through a platform's face, and once the hub was over the face nothing
            // pushed at all.
            for _ in 0..2 {
                for i in 0..self.wheels.len() {
                    let (_, _, fwd_h, _, hub) = tyre(&self.wheels[i]);
                    let r_hub = (hub - self.position).as_vec3();
                    for f in 0..self.wheels[i].walls.len() {
                        let face = self.wheels[i].walls[f];
                        let n = face.normal.extend(0.0);
                        let reach = face.reach(fwd_h, self.wheels[i].radius);
                        let gap = face.distance(hub) - reach;
                        let v_hub = self.velocity + rot.mul_vec3(self.omega).cross(r_hub);
                        let closing = -v_hub.dot(n);
                        let excess = closing - gap.max(0.0) / h;
                        if excess <= 0.0 {
                            continue;
                        }
                        let j = excess / self.inv_mass_at(r_hub, n);
                        let before = self.kinetic_energy();
                        self.apply_impulse(r_hub, n * j);
                        let lost = (before - self.kinetic_energy()).max(0.0);
                        self.wheels[i].step_force += j / h * n.dot(fwd_h);
                        match self.wheel_impacts.iter_mut().find(|m| m.obstacle == i) {
                            Some(m) => {
                                m.speed = m.speed.max(closing);
                                m.energy += lost;
                            }
                            None => {
                                let point = rot.inverse().mul_vec3(r_hub - n * reach) + self.cog;
                                self.wheel_impacts.push(Impact { point, speed: closing, energy: lost, obstacle: i, broke: false, push: -n });
                            }
                        }
                    }
                }
            }
            let omega_world = rot.mul_vec3(self.omega);
            let mut contacts: Vec<Contact> = Vec::with_capacity(self.wheels.len());
            for (i, w) in self.wheels.iter_mut().enumerate() {
                let (fwd, right, fwd_h, hub0, hub) = tyre(w);
                let r = w.radius;
                // pressed into a face: pushed back out
                let mut f_world = Vec3::ZERO;
                for face in &w.walls {
                    let n = face.normal.extend(0.0);
                    let pen = face.reach(fwd_h, r) - face.distance(hub);
                    if pen > 0.0 {
                        let v_hub = self.velocity + omega_world.cross((hub - self.position).as_vec3());
                        f_world += n * (WALL_K * pen - WALL_C * v_hub.dot(n)).max(0.0);
                    }
                }
                w.step_force += f_world.dot(fwd_h);
                // The tyre pushes the wheel up, the strut pushes it down and the body up, and
                // the wheel's own mass moves between the two. The tyre carries the static
                // load already where it just touches, so that the ride height stays the
                // springs' alone (see the module's notes).
                let (tyre_f, tyre_static, n) = if omsi_suspension() {
                    // Omsi.exe's suspension (0x7e47aa..0x7e4de8): the body hangs straight on
                    // the ground point under each wheel. The spring pushes by how far the
                    // ground lies over the wheel's unloaded place, times `Axle_Springfactor`
                    // and `achse_feder`; below it the wheel is in the air, pushes nothing and
                    // the tyres stop holding (0x7e4b13). Pressed, the damper takes
                    // `achse_daempfer` times the body's upward speed where the strut acts
                    // (the roll rate at `achse_maxwidth` / 2, 0x7e4899), and the whole never
                    // passes `achse_maxforce` (0x7e4b71) - there is no wheel of its own mass,
                    // no tyre and no bump stop between the road and the body. (Those, with a
                    // tyre envelope smoothing the road, had the bus float over what it drove
                    // on: "like a boat on the sea".)
                    let k = w.spring * w.spring_factor.max(0.0);
                    let top = hub0.z + (CLIMB * r) as f64;
                    // (nothing under it: Omsi.exe's ground query looks from 3 m over the model
                    // origin's plane, 0x7a0985 - a wheel that sank through a face at a joint of
                    // two surfaces, or into a bridge deck, found nothing within reach, the
                    // spring let go and the bus fell through the world)
                    // The spring's point is where Omsi.exe puts it (0x7e47aa): on the model's
                    // origin plane (z = 0) under the wheel, and its height over the ground is
                    // measured straight up. Measured from the hub less the `.bus` file's tyre
                    // radius instead, a mod whose tyre mesh is larger than that radius stood
                    // with its wheels drawn sunk a few centimetres into the road.
                    let plane0 = position + rot.mul_vec3(Vec3::new(w.attach.x, w.attach.y, 0.0) - cog).as_dvec3();
                    let under = probe(plane0.x, plane0.y, top).below.or_else(|| probe(plane0.x, plane0.y, plane0.z + 3.0).below);
                    let t = under.map(|g| (g - plane0.z) as f32);
                    if let Some(g) = under {
                        w.ground_z = g;
                        w.ground_seen = true;
                    }
                    let r_out = rot.mul_vec3(w.attach - self.cog + Vec3::new(w.lever, 0.0, 0.0));
                    let v_up = (self.velocity + omega_world.cross(r_out)).dot(up);
                    let mut n = 0.0f32;
                    if let Some(t) = t {
                        let spring = k * t;
                        if spring >= 0.0 {
                            n = (spring - w.damper * v_up).min(w.max_force);
                        }
                    }
                    let travel = t.unwrap_or(-DROOP);
                    let full = if k > 1.0 { w.max_force / k } else { BUMP };
                    w.compression_rate = if w.touch.is_some() { (travel.clamp(-DROOP, full) - w.compression) / h } else { 0.0 };
                    w.compression = travel.clamp(-DROOP, full);
                    w.touch = t;
                    let on = t.is_some_and(|t| k * t >= 0.0);
                    w.on_ground = on;
                    w.load = n.max(0.0);
                    (if on { n.max(0.0) } else { 0.0 }, if on { n.max(0.0) } else { 0.0 }, n)
                } else {
                    let m_w = (w.rest_load / 9.81 * UNSPRUNG).max(40.0);
                    let k = w.spring * w.spring_factor.max(0.05);
                    // How far the ground under the tyre can rise within a substep: a tyre rolls up
                    // a step along its envelope, never steeper than 2.4 (see CLIMB). A reading
                    // that leaps higher (a kerb's edge or a surface object's rim caught for one
                    // substep: +25 cm under the Urbino's front wheel at a Spandau kerb) is
                    // climbed at that pace - taken at once it struck the wheel with 260 kN and
                    // threw it into its arch.
                    let v_along = (self.velocity + omega_world.cross((hub - self.position).as_vec3())).dot(fwd_h).abs();
                    let max_rise = (v_along.max(0.5) * 2.4 + 0.3) * h;
                    let need = found[i].map(|(need, _, _)| if w.ground_seen { need.min(w.ground_z + r as f64 + max_rise as f64) } else { need });
                    let touch = need.map(|need| ((need - hub0.z) / up.z.max(0.3) as f64) as f32);
                    if let Some(need) = need {
                        w.ground_z = need - r as f64;
                        w.ground_seen = true;
                    }
                    let (tyre_f, tyre_static) = match touch {
                        // (a wheel that cannot reach the ground at full droop hangs in the air)
                        Some(t) if t >= -DROOP - 0.05 => {
                            let pen = t - w.compression;
                            let closing = w.touch.map(|p| ((t - p) / h).clamp(-3.0, 3.0)).unwrap_or(0.0) - w.compression_rate;
                            let s = w.tyre_k * pen + w.rest_load;
                            if s > 0.0 {
                                ((s + w.tyre_c * closing).clamp(0.0, w.max_force * 3.0), s)
                            } else {
                                (0.0, 0.0)
                            }
                        }
                        _ => (0.0, 0.0),
                    };
                    w.touch = touch;
                    // the strut: spring, a digressive damper (a kerb struck at speed must not fire
                    // the body into the air) and the bump stop, a stiff rubber block; as a whole it
                    // takes no more than three times the load it is built for (`achse_maxforce`)
                    // (the spring only pushes: below its unloaded length the wheel hangs on the
                    // damper, which also works on the rebound)
                    // (OMSI: spring and damper together never pass more than `achse_maxforce`;
                    // beyond it the bump stop, a rubber block of its own, takes over)
                    let c = w.compression;
                    let rate_c = w.compression_rate.clamp(-1.5, 1.5);
                    let bump = if c > BUMP { (w.spring * 10.0 * (c - BUMP)).min(w.max_force * 2.0) } else { 0.0 };
                    let n = (k * c.max(0.0) + w.damper * rate_c).clamp(-w.max_force * 0.5, w.max_force) + bump;
                    // the wheel moves against the body, which is itself accelerated by what holds
                    // it up (`accel_body.z`, 9.81 m/s² standing, 0 in the air: a wheel off a kerb
                    // drops, a wheel of a body in the air does not) - semi-implicit Euler, stable
                    // with the stiff tyre at these steps
                    w.compression_rate += ((tyre_f - n) / m_w - self.accel_body.z) * h;
                    w.compression += w.compression_rate * h;
                    if w.compression < -DROOP {
                        w.compression = -DROOP;
                        w.compression_rate = w.compression_rate.max(0.0);
                    } else if w.compression > BUMP + 0.08 {
                        w.compression = BUMP + 0.08;
                        w.compression_rate = w.compression_rate.min(0.0);
                    }
                    w.on_ground = tyre_f > 0.0;
                    w.load = tyre_f;
                    (tyre_f, tyre_static, n)
                };
                // the strut's force turns the body about its length from the outer width, not
                // from where the tyre stands (see `RigidWheel::lever`): body x cross body z
                torque += Vec3::new(0.0, -w.lever * n, 0.0);
                // and OMSI's damper reads the body's speed out there too (0x7e4899: roll rate
                // x maxwidth / 2): the part of it the hub does not see, while the tyre carries
                if tyre_f > 0.0 && !omsi_suspension() {
                    torque.y -= (w.attach.x + w.lever) * w.damper * self.omega.y * w.lever;
                }
                let Some((_, slope, contact_dx)) = found[i].filter(|_| tyre_f > 0.0) else {
                    // off the ground the wheel keeps its turning (a brake stops it)
                    let (drive_w, brake_w) = (if w.driven { drive_torque / r / driven } else { 0.0 }, brake.get(i).copied().unwrap_or(0.0).max(0.0));
                    let before = w.spin;
                    w.spin += h * w.inertia_inv * (drive_w - sign(w.spin) * brake_w);
                    if before != 0.0 && sign(before) != sign(w.spin) && brake_w > 0.0 {
                        w.spin = 0.0;
                    }
                    w.slipping = omsi_cfg::env::var_os("OMSI_NO_WHEEL_SLIP").is_none();
                    w.rpm = w.spin * 60.0 / std::f32::consts::TAU;
                    w.rotation_deg = (w.rotation_deg + w.spin.to_degrees() * h).rem_euclid(360.0);
                    // the tyre off the ground (or nothing under it: the edge of the loaded
                    // world): the strut still pushes on the body
                    let f = f_world + up * n;
                    force += f;
                    torque += rot.inverse().mul_vec3((hub - self.position).as_vec3()).cross(rot.inverse().mul_vec3(f));
                    continue;
                };
                // The ground pushes along its own normal; the strut carries the part along
                // the body's up axis, and the rest pushes the wheel back up a kerb (or on
                // down an edge). Measured against the body, a slope the bus stands on
                // already leans with it and adds nothing; the tyre's static load does the
                // pushing - the damping kick as the tyre meets the edge is taken by the
                // wheel's own mass.
                let normal = (Vec3::Z - fwd_h * slope).normalize();
                let along_up = normal.dot(up).max(0.3);
                let carried = tyre_static.min(w.max_force * 2.0);
                let step = (normal - up * normal.dot(up)) * (carried / along_up);
                w.step_force += step.dot(fwd_h);
                let contact = hub + (fwd_h * contact_dx.clamp(-r, r)).as_dvec3() - DVec3::Z * r as f64;
                let r_world = (contact - self.position).as_vec3();
                let v_point = self.velocity + omega_world.cross(r_world);
                let v_long = v_point.dot(fwd);
                // the tyre works with at most its rated load: the spike of a wheel landing on
                // its bump stop is not grip, and fed into the side force it spun a bus round
                // on a verge
                let grip = tyre_f.min(w.max_force);
                contacts.push(Contact {
                    r: r_world,
                    fwd,
                    right,
                    base: f_world + up * n + step,
                    drive: if w.driven { drive_torque / r / driven } else { 0.0 },
                    brake: brake.get(i).copied().unwrap_or(0.0).max(0.0) + self.rolling_resistance / n_wheels,
                    grip,
                    v_long,
                    lateral: true,
                    wheel: Some(i),
                });
            }
            // The coupled parts, each a tyre at the joint that pushes and brakes along the
            // body's axis and takes no side force.
            let fwd_flat = Vec3::new(body_fwd.x, body_fwd.y, 0.0).normalize_or(Vec3::Y);
            for d in &self.coupled {
                let share = Vec3::new(d.dir.x, d.dir.y, 0.0).normalize_or(fwd_flat).dot(fwd_flat).max(0.0);
                let r_world = rot.mul_vec3(d.point - self.cog);
                let cap = self.friction * d.driven_load;
                let drive = if d.driven_wheels > 0 { (drive_torque / d.radius.max(0.1) / driven * d.driven_wheels as f32).clamp(-cap, cap) } else { 0.0 };
                contacts.push(Contact {
                    r: r_world,
                    fwd: body_fwd,
                    right: Vec3::ZERO,
                    base: body_fwd * (-9.81 * d.mass * body_fwd.z),
                    drive: drive * share,
                    brake: d.brake.max(0.0) * share,
                    grip: d.load,
                    v_long: (self.velocity + omega_world.cross(r_world)).dot(body_fwd),
                    lateral: false,
                    wheel: None,
                });
            }
            // Tyre forces as OMSI has them (Omsi.exe 0x7e2dc1..0x7e44ee).
            // Along the tyre: a rolling one passes on the drive and the brake, a slipping one
            // the sliding friction. Tyres that stand hold like static friction - but
            // together: they take up everything else that pushes the body (gravity on a
            // slope, the drive, the rolling ones) and what is left of its speed, shared by
            // what each brake can bear. One wheel at a time, each cancelling only its quarter
            // of the bus, a parked bus crept down an 8 % slope at 2 mm/s.
            let mut outer = force + contacts.iter().map(|c| c.base).sum::<Vec3>();
            let mut rolling_long = vec![0.0f32; contacts.len()];
            for (k, c) in contacts.iter().enumerate() {
                // a slipping wheel passes on the sliding friction, whichever way the tyre
                // slides over the road (the wheel's own speed against the ground's)
                if let Some(w) = c.wheel.map(|i| &self.wheels[i]).filter(|w| w.slipping) {
                    let mu_n = self.friction * c.grip;
                    let slide = w.spin * w.radius - c.v_long;
                    rolling_long[k] = mu_n * sign(slide);
                    outer += c.fwd * rolling_long[k];
                    continue;
                }
                if c.v_long.abs() > STANDING {
                    rolling_long[k] = c.drive - c.brake * c.v_long.signum();
                    outer += c.fwd * rolling_long[k];
                } else {
                    outer += c.fwd * c.drive;
                }
            }
            let standing: Vec<usize> = (0..contacts.len()).filter(|&k| contacts[k].v_long.abs() <= STANDING && !contacts[k].wheel.is_some_and(|i| self.wheels[i].slipping)).collect();
            let hold = |axis: Vec3, mass: f32, cap: &dyn Fn(&Contact) -> f32| -> Vec<f32> {
                let need = -outer.dot(axis) - self.velocity.dot(axis) * mass / h;
                let total: f32 = standing.iter().map(|&k| cap(&contacts[k])).sum();
                standing.iter().map(|&k| if total > 1e-3 { (need * cap(&contacts[k]) / total).clamp(-cap(&contacts[k]), cap(&contacts[k])) } else { 0.0 }).collect()
            };
            let hold_long = hold(body_fwd, self.mass + towed, &|c| c.brake);
            // (the tyres' pull along the body, for the pitch lever below)
            let mut long_sum = 0.0f32;
            for (k, c) in contacts.iter().enumerate() {
                let mut f_long = match standing.iter().position(|&j| j == k) {
                    Some(si) => c.drive + hold_long[si],
                    None => rolling_long[k],
                };
                let max_f = self.friction * c.grip;
                f_long = f_long.clamp(-max_f, max_f);
                let f_world = c.base + c.fwd * f_long;
                long_sum += f_long * c.fwd.dot(body_fwd);
                if let Some(i) = c.wheel {
                    self.wheels[i].advance_spin(c, f_long, self.friction, h);
                }
                force += f_world;
                torque += rot.inverse().mul_vec3(c.r).cross(rot.inverse().mul_vec3(f_world));
            }
            // Driving and braking pitch the body about a lever longer than the centre of
            // gravity's height over the road: Omsi.exe takes the tyres' forces at the hubs,
            // their radius more (0x7e46a5: (drive - brake) x d/2 besides [schwerpunkt] x
            // the pull), and not while the bus stands with its brakes holding (0x7e4660).
            if self.velocity.dot(body_fwd).abs() > 0.2 {
                let r = self.wheels.iter().map(|w| w.radius).sum::<f32>() / self.wheels.len().max(1) as f32;
                torque.x += long_sum * r;
            }
            // Across the tyre: OMSI has no slip angle. Each axle takes away the sideways
            // speed it has - the bus follows its wheels exactly - with no more force than
            // the road's grip times the load on it; only beyond that does it slide
            // (0x7e4012: -0.5 x mass x side speed / frame per axle, capped at mu x load).
            // Here the same as a constraint on every tyre, solved over this substep with
            // everything else that pushes the body. The old tyre instead gave 12 x load per
            // radian of slip: every bus turned at 70 % of what its steering asked, 0.8 s
            // late, drifting 2-3 degrees and swaying - the same for every `.bus`.
            {
                let (m, mt) = (self.mass, self.mass + towed);
                let lin = |p: Vec3| (p - body_fwd * p.dot(body_fwd)) / m + body_fwd * (p.dot(body_fwd) / mt);
                let along = force.dot(body_fwd);
                let mut v = self.velocity + ((force - body_fwd * along) / m + body_fwd * (along / mt)) * h;
                let mut w = self.omega + (torque - self.omega.cross(self.inertia * self.omega)) / self.inertia * h;
                let lat: Vec<usize> = (0..contacts.len()).filter(|&k| contacts[k].lateral && contacts[k].grip > 0.0).collect();
                // (following its wheels: hardly any speed across the body)
                let gripped = self.velocity.dot(rot.mul_vec3(Vec3::X)).abs() < 0.5 && self.velocity.dot(body_fwd).abs() > 1.0;
                let arms: Vec<(Vec3, f32)> = lat
                    .iter()
                    .map(|&k| {
                        let c = &contacts[k];
                        let mut rn = rot.inverse().mul_vec3(c.r).cross(rot.inverse().mul_vec3(c.right));
                        // (holding, no roll from the tyres' hold across: Omsi.exe then sets the
                        // sideways motion outright and rolls the body by the bend's pull alone,
                        // below - taken at the ground, every turn of the wheel kicked the body
                        // over; sliding, the tyres trip it as before)
                        if self.holding && gripped {
                            rn.y = 0.0;
                        }
                        let inv = c.right.dot(lin(c.right)) + (rn / self.inertia).dot(rn);
                        (rn, inv.max(1e-9))
                    })
                    .collect();
                // holding: the bend's pull against the grip of all tyres (0x7e4dee), and
                // no wheel spinning or locked (0x7e3c3b)
                let grip_all: f32 = lat.iter().map(|&k| self.friction * contacts[k].grip).sum();
                let v_fwd = self.velocity.dot(body_fwd);
                let kappa = (self.steer_deg.to_radians().tan() / (self.wheels.iter().map(|w| w.attach.y).fold(f32::MIN, f32::max) - self.rot_pnt_long).abs().max(0.5)).abs();
                let slipping = self.wheels.iter().any(|w| w.slipping && w.on_ground);
                // (and a wheel in the air: Omsi.exe drops the holding state there, 0x7e4b13)
                let airborne = omsi_suspension() && self.wheels.iter().any(|w| !w.on_ground);
                if self.holding && (m * v_fwd * v_fwd * kappa > grip_all || slipping || airborne) {
                    self.holding = false;
                }
                // (holding, a tyre may pass several times its grip for the moment it takes
                // the body to follow a quick turn of the wheel: OMSI sets the yaw rate outright)
                let reach = if self.holding { 8.0 } else { 1.0 };
                let mut j = vec![0.0f32; lat.len()];
                for _ in 0..8 {
                    for (q, &k) in lat.iter().enumerate() {
                        let c = &contacts[k];
                        let v_lat = (v + rot.mul_vec3(w).cross(c.r)).dot(c.right);
                        let cap = self.friction * c.grip * h * reach;
                        let before = j[q];
                        j[q] = (before - v_lat / arms[q].1).clamp(-cap, cap);
                        let dj = j[q] - before;
                        v += lin(c.right) * dj;
                        w += arms[q].0 / self.inertia * dj;
                    }
                }
                for (q, &k) in lat.iter().enumerate() {
                    force += contacts[k].right * (j[q] / h);
                    torque += arms[q].0 * (j[q] / h);
                }
                if !self.holding && !slipping && lat.iter().all(|&k| (v + rot.mul_vec3(w).cross(contacts[k].r)).dot(contacts[k].right).abs() < 0.1) {
                    self.holding = true;
                }
                // the bend's pull rolls the body out of it (0x7e4629: [schwerpunkt] x mass x
                // the centripetal acceleration)
                if self.holding && gripped && !lat.is_empty() {
                    torque.y += m * self.cog.z * v_fwd * self.omega.z;
                }
            }
            // integrate
            let along = force.dot(body_fwd);
            let acc = (force - body_fwd * along) / self.mass + body_fwd * (along / (self.mass + towed));
            accel_sum += rot.inverse().mul_vec3(acc - Vec3::new(0.0, 0.0, -9.81));
            self.velocity += acc * h;
            self.position += (self.velocity * h).as_dvec3();
            let iw = self.inertia * self.omega;
            let omega_dot = (torque - self.omega.cross(iw)) / self.inertia;
            self.omega += omega_dot * h;
            let dq = Quat::from_scaled_axis(rot.mul_vec3(self.omega) * h);
            self.orientation = (dq * rot).normalize();
        }
        self.accel_body = accel_sum / substeps as f32;
        // Asleep: with the brakes holding and no drive, the last few millimetres per
        // second of horizontal drift are noise from the tyre model, not motion. Take them
        // away so the bus stands truly still (the vertical velocity stays: the suspension
        // may still be settling).
        let braked: f32 = brake.iter().sum();
        let horizontal = Vec3::new(self.velocity.x, self.velocity.y, 0.0).length();
        if horizontal < 0.03 && drive_torque.abs() < 1.0 && braked > 0.02 * self.mass * 9.81 {
            self.velocity.x = 0.0;
            self.velocity.y = 0.0;
            self.omega.z = 0.0;
        }
        // rotation keeps the body from tipping over completely (safety net for bad ground)
        let up = self.orientation.mul_vec3(Vec3::Z);
        if up.z < 0.3 {
            let (heading, _, _) = self.heading_pitch_bank();
            self.orientation = Quat::from_rotation_z((-heading).to_radians() as f32);
            self.omega = Vec3::ZERO;
        }
    }

    /// Keep the body's `[boundingbox]` out of the obstacles and answer each hit with an
    /// impulse at the point of contact: the closing speed goes (a fifth of it comes back),
    /// sliding along the obstacle is braked by friction, and an off-centre blow turns the
    /// body. Nothing is frozen - the vehicle can drive off or reverse at once.
    ///
    /// `skip(i)`: obstacles already broken off. `dt` is the frame, to tell a hit from a box
    /// the vehicle has been standing in all along (spawned in a depot hall's box): that one
    /// is deeper than any frame of motion could have taken it, and never pushes.
    ///
    /// Each obstacle is answered once per call (up to four of them, deepest first). A car
    /// that drives into the bus is still there after its answer - the bus only steps a
    /// millimetre out of its way and takes none of its speed - and was taken as the same
    /// crash four times over.
    pub fn collide(&mut self, bb: [f32; 6], obstacles: &[Obb], skip: &dyn Fn(usize) -> bool, dt: f32) -> Vec<Impact> {
        let mut impacts: Vec<Impact> = Vec::new();
        let mut answered: Vec<usize> = Vec::new();
        {
            let me = self.body_box(bb);
            let touching: Vec<i64> = obstacles.iter().enumerate().filter(|(i, o)| o.id >= 0 && o.mass <= 0.0 && !skip(*i) && me.contact(o).is_some()).map(|(_, o)| o.id).collect();
            match self.spawned_inside.as_mut() {
                None => self.spawned_inside = Some(touching),
                // (once out of one, it counts again)
                Some(list) => list.retain(|id| touching.contains(id)),
            }
        }
        let inside = self.spawned_inside.clone().unwrap_or_default();
        for _ in 0..4 {
            let me = self.body_box(bb);
            let mut deepest: Option<(usize, crate::collision::Contact)> = None;
            for (i, o) in obstacles.iter().enumerate() {
                if skip(i) || answered.contains(&i) || (o.id >= 0 && inside.contains(&o.id)) {
                    continue;
                }
                let Some(c) = me.contact(o) else { continue };
                let rel = (glam::DVec2::new(self.velocity.x as f64, self.velocity.y as f64) - o.velocity).length() + self.omega.z.abs() as f64 * 6.0;
                if c.depth > 0.5 + rel * dt as f64 * 2.0 {
                    continue;
                }
                if deepest.map(|d| c.depth > d.1.depth).unwrap_or(true) {
                    deepest = Some((i, c));
                }
            }
            let Some((i, c)) = deepest else { break };
            answered.push(i);
            let o = &obstacles[i];
            let n = Vec3::new(c.normal.x as f32, c.normal.y as f32, 0.0);
            // A wall as tall as the vehicle stops it at the height of its centre of gravity (a
            // real front is stiffest down at the frame; the middle of a double-decker's face
            // tipped it onto its rear wheels); a low bollard or a high beam acts where it is.
            let z = self.position.z.clamp(c.z0, c.z1.max(c.z0));
            let r = (DVec3::new(c.point.x, c.point.y, z) - self.position).as_vec3();
            // A moving obstacle (an AI car) has a mass of its own, which shares the impulse and
            // the way out of the overlap. But the AI moves it by itself and never gives up its
            // speed: answered with its full approach, it came back every frame and shoved the
            // bus across the road at its own pace until the bus nearly rolled over. So it blocks
            // the bus's motion into it; its own run into the bus counts as the crash it is, and
            // pushes nothing.
            let v_other = Vec3::new(o.velocity.x as f32, o.velocity.y as f32, 0.0);
            let v_body = self.velocity + self.orientation.mul_vec3(self.omega).cross(r);
            let vn_full = (v_body - v_other).dot(n);
            let v_c = v_body - (v_other - n * v_other.dot(n).max(0.0));
            let vn = v_c.dot(n);
            let inv_other = if o.mass > 0.0 { 1.0 / o.mass } else { 0.0 };
            let k_body = self.inv_mass_at(r, n);
            let k_n = k_body + inv_other;
            let before = self.kinetic_energy();
            // The scripts are told where the bodies meet, low down where the bumpers are
            // (the stock buses damage their rear engine only below 1.10 m), not where the
            // blow is taken: at the centre of gravity every wall and car read 1.2-1.3 m.
            let seen = DVec3::new(c.point.x, c.point.y, crate::collision::impact_height(c.z0, c.z1));
            let point = self.orientation.inverse().mul_vec3((seen - self.position).as_vec3()) + self.cog;
            if let Some((mass_t, load_kn)) = o.pole.filter(|_| vn < 0.0) {
                // A post breaks at its foot when stopping the vehicle would take more than it
                // can bear; the vehicle then only loses what it takes to throw the post.
                let stop = -vn / k_n / dt.max(1e-3);
                if -vn > CRASH_SPEED && stop > load_kn.max(0.1) * 1000.0 {
                    let m_pole = (mass_t * 1000.0).max(5.0);
                    let j = -(1.0 + RESTITUTION) * vn / (k_n + 1.0 / m_pole);
                    self.apply_impulse(r, n * j);
                    impacts.push(Impact { point, speed: -vn, energy: (before - self.kinetic_energy()).max(0.0), obstacle: i, broke: true, push: -n });
                    continue;
                }
            }
            // out of the obstacle along the shortest way, a millimetre clear - out of a moving
            // one only as far as the body itself ran into it this frame: a standing car is not
            // ploughed through, and one that drives into the bus does not drag it along
            let out = if o.mass > 0.0 { (c.depth as f32).min((-vn).max(0.0) * dt + 0.001) } else { c.depth as f32 + 0.001 };
            self.position += (n * out).as_dvec3();
            if vn_full >= 0.0 {
                continue;
            }
            // a gentle push is dead, a real blow gives a little back
            let e = if -vn_full > 1.0 { RESTITUTION } else { 0.0 };
            let j = -(1.0 + e) * vn.min(0.0) / k_n;
            let mut p = n * j;
            let vt = v_c - n * vn;
            let vt = Vec3::new(vt.x, vt.y, 0.0);
            if vt.length() > 1e-3 {
                let t = vt.normalize();
                let jt = (vt.length() / (self.inv_mass_at(r, t) + inv_other)).min(SCRAPE_FRICTION * j);
                p -= t * jt;
            }
            self.apply_impulse(r, p);
            // what the crash destroyed: the closing motion of the two, less what bounced back
            // (the bus alone may even gain energy when the other one does the hitting)
            let energy = (before - self.kinetic_energy()).max(0.5 * vn_full * vn_full * (1.0 - e * e) / k_n);
            impacts.push(Impact { point, speed: -vn_full, energy, obstacle: i, broke: false, push: -n });
        }
        // a blow the tyres have to catch: they slide until the body is back on its wheels
        if impacts.iter().any(|m| m.speed >= CRASH_SPEED) {
            self.holding = false;
        }
        impacts
    }
}

/// What a tyre finds along its length (see [`tyre_line`]).
struct TyreLine {
    /// The world height the hub has to stand at, how fast that rises as the hub rolls
    /// forward (m per m) and where along the tyre (m ahead of the hub) the ground carries
    /// it. None when there is nothing below the tyre at all.
    ground: Option<(f64, f32, f32)>,
    /// A face the tyre cannot climb ahead of the hub and one behind it, each as the last
    /// clear sample and the first one in the face (m ahead of the hub), when it is near
    /// enough to be touched within the substep.
    faces: [Option<(f32, f32)>; 2],
    /// The samples either side of the hub are clear of faces: it is not sunk into a step.
    clear: bool,
}

/// The ground under and around a tyre of radius `r` rolling along `fwd_h`.
///
/// The tyre is an envelope [`ENVELOPE`] radii long (a flattened circle) laid over the ground,
/// and the hub stands where the envelope first touches it: `need` = the highest ground
/// height plus the envelope's height over it. The ground is a polyline through samples
/// that stand still on the ground while the hub rolls past them (a lattice along the tyre's
/// direction), with an edge between two of them placed by halving. Anchored so, the
/// polyline is the same at every step, `need` changes smoothly as the hub rolls, and the
/// slope reported is exactly how fast it changes: the ground's own slope where the envelope
/// rests on a face of it, the envelope's where it slides over a corner. Only then does the
/// push that slope gives (see `step`) take from the body what lifting it over a step gains;
/// a slope from the samples that differed from it pumped energy into the body on steep
/// faces, and one from a comb carried with the hub read every grade over 6 % as a run of
/// kerbs.
///
/// A sample whose ground is higher than `z_top` (the most the tyre climbs) is no ground,
/// and within a radius, a sample and `look` metres of the hub it is a face the tyre runs
/// against if that face is lower than `wall_top`.
fn tyre_line(probe: &dyn Fn(f64, f64, f64) -> GroundProbe, hub: DVec3, fwd_h: Vec3, r: f32, z_top: f64, wall_top: f64, look: f32) -> TyreLine {
    const N: usize = LATTICE;
    let a = r * ENVELOPE;
    let lift = |x: f32| r * (1.0 - (x / a).powi(2)).max(0.0).sqrt();
    // d need / d hub while the envelope slides over a corner x ahead of the hub
    let corner_slope = |x: f32| r * x / (a * a * (1.0 - (x / a).powi(2)).max(1e-3).sqrt());
    let spacing = 2.0 * a / (TYRE_SAMPLES - 1) as f32;
    let fwd = fwd_h.as_dvec3();
    let along = hub.x * fwd.x + hub.y * fwd.y;
    let first = ((along - a as f64) / spacing as f64).floor() - 2.0;
    // metres ahead of the hub of lattice point k: x(2) <= -a < x(3), x(N - 4) <= a < x(N - 3)
    let x_of = |k: usize| (first + k as f64) * spacing as f64 - along;
    let at = |dx: f64| {
        let p = hub + fwd * dx;
        probe(p.x, p.y, z_top)
    };
    let xs: [f32; N] = std::array::from_fn(|k| x_of(k) as f32);
    let mut below = [None; N];
    let mut wall = [false; N];
    for k in 0..N {
        let g = at(x_of(k));
        below[k] = g.below;
        wall[k] = xs[k].abs() <= r + spacing + look && g.above.map(|z| z < wall_top).unwrap_or(false);
    }
    let rise = |k: usize| match (below.get(k).copied().flatten(), below.get(k + 1).copied().flatten()) {
        (Some(g0), Some(g1)) => Some(g1 - g0),
        _ => None,
    };
    // An edge: a rise unlike those either side of it. A grade, however steep, rises alike
    // from sample to sample; a curve changes its rise by far less than EDGE over one
    // spacing. Every rise that decides the pieces over the envelope has both neighbours
    // (and they theirs), so a step is told the same way wherever the hub stands; told from
    // one neighbour, the level ground after a step at the end of the samples was an edge
    // too, the step no longer a lone one, and the hub jumped a centimetre as the lattice
    // moved on.
    let edge: [bool; N] = std::array::from_fn(|k| match (rise(k), k.checked_sub(1).and_then(rise), rise(k + 1)) {
        (Some(d), Some(l), Some(h)) => (d - l).abs() > EDGE && (d - h).abs() > EDGE,
        _ => false,
    });

    let mut best: Option<(f64, f32, f32)> = None;
    let mut offer = |need: f64, slope: f32, x: f32| {
        if best.map_or(true, |b| need > b.0) {
            best = Some((need, slope, x));
        }
    };
    // the highest the envelope has to stand over the piece of ground from (x0, z0) to (x1, z1)
    let mut piece = |x0: f32, z0: f64, x1: f32, z1: f64| {
        if x1 < -a || x0 > a {
            return;
        }
        if x1 - x0 < 1e-6 {
            offer(z0.max(z1) + lift(x0) as f64, corner_slope(x0), x0);
            return;
        }
        let m = ((z1 - z0) / (x1 - x0) as f64) as f32;
        let (lo, hi) = (x0.max(-a), x1.min(a));
        // where the envelope's own slope matches the ground's
        let t = m * a / r;
        let free = a * t / (1.0 + t * t).sqrt();
        let x = free.clamp(lo, hi);
        let need = z0 + (m * (x - x0)) as f64 + lift(x) as f64;
        // resting on the face, or at the end of the tyre (which rolls on with the hub), the
        // need rises with the ground; at a corner of the ground the envelope slides over it
        let slope = if x == free || x <= -a || x >= a { m } else { corner_slope(x) };
        offer(need, slope, x);
    };
    for k in 0..N {
        let Some(g0) = below[k] else { continue };
        let left = k > 0 && below[k - 1].is_some();
        let Some(g1) = below.get(k + 1).copied().flatten() else {
            if !left && xs[k].abs() <= a {
                piece(xs[k], g0, xs[k], g0);
            }
            continue;
        };
        let lone_edge = edge[k] && !(k > 0 && edge[k - 1]) && !edge.get(k + 1).copied().unwrap_or(false);
        if !lone_edge || xs[k + 1] < -a || xs[k] > a {
            // no step, one beyond the envelope, or a run of them: a steep face the samples
            // outline well enough
            piece(xs[k], g0, xs[k + 1], g1);
            continue;
        }
        // A step between the two. The ground either side runs on at its own grade (that of
        // the rise beyond it) up to where the step is: the point of the gap whose height is
        // nearer the far side's.
        let grade = |j: usize| rise(j).unwrap_or(0.0) / spacing as f64;
        let (m_lo, m_hi) = (grade(k - 1), grade(k + 1));
        let (x0, x1) = (x_of(k), x_of(k + 1));
        let near = |x: f64| g0 + m_lo * (x - x0);
        let far = |x: f64| g1 + m_hi * (x - x1);
        let (mut lo, mut hi) = (x0, x1);
        for _ in 0..EDGE_HALVINGS {
            let m = 0.5 * (lo + hi);
            match at(m).below {
                Some(z) if (z - far(m)).abs() < (z - near(m)).abs() => hi = m,
                Some(_) => lo = m,
                None => break,
            }
        }
        let c = 0.5 * (lo + hi);
        piece(xs[k], g0, c as f32, near(c));
        piece(c as f32, far(c), xs[k + 1], g1);
    }

    // the faces nearest the hub ahead and behind, bracketed by the clear sample before them
    let ahead = (0..N).find(|&k| xs[k] > 0.0 && wall[k]).filter(|&k| k > 0 && !wall[k - 1]).map(|k| (xs[k - 1], xs[k]));
    let behind = (0..N).rev().find(|&k| xs[k] <= 0.0 && wall[k]).filter(|&k| k + 1 < N && !wall[k + 1]).map(|k| (xs[k + 1], xs[k]));
    let clear = (0..N - 1).find(|&k| xs[k] <= 0.0 && xs[k + 1] > 0.0).map_or(true, |k| !wall[k] && !wall[k + 1]);
    TyreLine { ground: best, faces: [ahead, behind], clear }
}

/// How much higher (or lower) the hub stands than the ground under the tread's middle line
/// says, for a step running along under the tread (the tyre driving onto a kerb at a
/// shallow angle, or with one shoulder over its edge). The lengthwise envelope of
/// [`tyre_line`] sees only the middle line: the tyre stood wholly down or wholly up on the
/// kerb and jumped its full height as the edge passed under the hub. Here the tread's width
/// is looked at: with a share of it over the higher ground the hub rises by the step times
/// that share (all of it once half the tread is up, as a tyre carries the load on the part
/// that touches), and over lower ground at one shoulder it sinks likewise.
fn tread_step(probe: &dyn Fn(f64, f64, f64) -> GroundProbe, hub: DVec3, fwd_h: Vec3, r: f32, z_top: f64) -> f64 {
    let w = (HALF_WIDTH * r) as f64;
    let across = DVec3::new(fwd_h.y as f64, -fwd_h.x as f64, 0.0);
    let g = |d: f64| {
        let p = hub + across * d;
        probe(p.x, p.y, z_top).below
    };
    let Some(gc) = g(0.0) else { return 0.0 };
    let mut best = 0.0f64;
    for side in [-1.0f64, 1.0] {
        let Some(gs) = g(side * w) else { continue };
        let h = gs - gc;
        if h.abs() < 0.02 {
            continue;
        }
        // where across the tread the ground changes: halving between the middle and the
        // shoulder
        let (mut lo, mut hi) = (0.0, w);
        for _ in 0..5 {
            let m = 0.5 * (lo + hi);
            match g(side * m) {
                Some(z) if (z - gs).abs() < (z - gc).abs() => hi = m,
                Some(_) => lo = m,
                None => break,
            }
        }
        let e = 0.5 * (lo + hi);
        // share of the tread on the other ground (from its edge to this shoulder)
        let share = ((w - e) / (2.0 * w)).clamp(0.0, 0.5);
        let t = share / 0.5;
        let k = t * t * (3.0 - 2.0 * t);
        // up: the shoulder over the kerb takes the load in proportion. (Down, with the
        // middle still on the step and a shoulder hanging over its edge, the hub stays: seen
        // from the other side once the edge passes the middle, it is the same share going
        // the other way, so the tyre comes down from the kerb as smoothly as it went up.)
        let delta = if h > 0.0 { h * k } else { 0.0 };
        if delta.abs() > best.abs() {
            best = delta;
        }
    }
    best
}

/// The faces a tyre is up against after this step: those it was against that are still
/// there and near, and new ones its samples (and a look to either side, `look` metres
/// further than the tyre reaches) found.
#[allow(clippy::too_many_arguments)]
fn track_walls(probe: &dyn Fn(f64, f64, f64) -> GroundProbe, old: &[WallFace], line: &TyreLine, hub: DVec3, fwd_h: Vec3, r: f32, z_top: f64, wall_top: f64, look: f32) -> Vec<WallFace> {
    let is_wall = |p: DVec2| probe(p.x, p.y, z_top).above.map(|z| z < wall_top).unwrap_or(false);
    let hub2 = hub.truncate();
    // A face stays while the hub is near it (as near as the samples find one) and has not
    // gone through by a whole radius, and while the ground just behind the line is still
    // the face and the ground just before it still clear (the face ends, or the line
    // drifted off a curved one).
    let mut out: Vec<WallFace> = old
        .iter()
        .copied()
        .filter(|f| {
            let d = f.distance(hub);
            let n = f.normal.as_dvec2();
            let foot = hub2 - n * d as f64;
            d > -r && d < 2.5 * r && is_wall(foot - n * 0.05) && !is_wall(foot + n * 0.03)
        })
        .collect();
    let facing = |out: &[WallFace], dir: Vec2| out.iter().any(|f| f.normal.dot(-dir) > 0.5);
    let fwd2 = fwd_h.truncate().normalize_or(Vec2::Y);
    for (clear, wall) in line.faces.iter().flatten().copied() {
        let dir = fwd2 * wall.signum();
        if !facing(&out, dir) {
            let d = halve(&is_wall, hub2, dir, clear * wall.signum(), wall.abs());
            out.push(find_face(&is_wall, hub2, dir, d));
        }
    }
    if line.clear {
        // beside the tyre (a bus sliding or turning into a platform)
        let reach = SIDE_REACH * HALF_WIDTH * r + look;
        for dir in [fwd2.perp(), -fwd2.perp()] {
            if !facing(&out, dir) && is_wall(hub2 + (dir * reach).as_dvec2()) {
                let d = halve(&is_wall, hub2, dir, 0.0, reach);
                out.push(find_face(&is_wall, hub2, dir, d));
            }
        }
    }
    out
}

/// Where between `clear` and `wall` metres from `from` along `dir` a face begins.
fn halve(is_wall: &dyn Fn(DVec2) -> bool, from: DVec2, dir: Vec2, clear: f32, wall: f32) -> f32 {
    let (mut a, mut b) = (clear, wall);
    for _ in 0..FACE_HALVINGS {
        let m = 0.5 * (a + b);
        if is_wall(from + (dir * m).as_dvec2()) {
            b = m;
        } else {
            a = m;
        }
    }
    0.5 * (a + b)
}

/// A face found `d` metres from the hub along `dir`, with its direction taken from where the
/// lines [`FACE_SPAN`] either side of that one meet it (square to `dir` where they miss it).
fn find_face(is_wall: &dyn Fn(DVec2) -> bool, hub2: DVec2, dir: Vec2, d: f32) -> WallFace {
    let across = dir.perp();
    let point = hub2 + (dir * d).as_dvec2();
    let side = |s: f32| {
        let from = hub2 + (across * s * FACE_SPAN).as_dvec2();
        let (a, b) = (d - 2.0 * FACE_SPAN, d + 2.0 * FACE_SPAN);
        let hit = !is_wall(from + (dir * a).as_dvec2()) && is_wall(from + (dir * b).as_dvec2());
        hit.then(|| from + (dir * halve(is_wall, from, dir, a, b)).as_dvec2())
    };
    let tangent = match (side(-1.0), side(1.0)) {
        (Some(p), Some(q)) => q - p,
        (Some(p), None) => point - p,
        (None, Some(q)) => q - point,
        (None, None) => across.as_dvec2(),
    };
    let mut normal = tangent.perp().normalize_or_zero().as_vec2();
    if normal.dot(dir) > 0.0 {
        normal = -normal;
    }
    if normal.dot(dir) > -0.1 {
        // a face the lines graze is no face to push the tyre along
        normal = -dir;
    }
    WallFace { point, normal }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The body's pitch and roll are damped as much a second at any frame rate: as OMSI
    /// damps them at 30 frames a second (a 12 t bus on 240-280 kN/m springs).
    #[test]
    fn body_damping_does_not_depend_on_the_frame_rate() {
        let f = ((2.0 * 240.0 + 2.0 * 280.0) / 12.0f32).sqrt();
        let second = |fps: f32| (0..fps as usize).fold(1.0f32, |k, _| k * super::body_damping(f, 1.0 / fps));
        let (a, b, c) = (second(30.0), second(60.0), second(144.0));
        assert!((a - b).abs() < 1e-3 && (a - c).abs() < 1e-3, "{a} {b} {c}");
        assert!(a > 0.2 && a < 0.5, "{a}");
    }
    use omsi_vehicle::vehicle::Axle;

    fn bus() -> Vehicle {
        let axle = |long: f32, spring: f32, max_force: f32, driven: bool| Axle { long, max_width: 2.4, min_width: 1.76, wheel_diameter: 0.94, spring, max_force, damper: 20.0, driven, inertia_inv: 0.0 };
        Vehicle { mass: 10.2, moment_of_inertia: [600.0, 90.0, 400.0], cog_height: 1.2, rolling_resistance: 1000.0, rot_pnt_long: -2.7, inv_min_turn_radius: 0.13, axles: vec![axle(3.238, 240.0, 90.0, false), axle(-2.637, 280.0, 116.0, true)], bounding_box: Some([2.472, 11.663, 2.491, 0.0, 0.001, 1.622]), ..Default::default() }
    }

    /// Flat road at height 0 with an optional step of `step` metres from y = `at` on.
    fn road(at: f64, step: f64) -> impl Fn(f64, f64, f64) -> GroundProbe {
        move |_x, y, z_top| {
            let z = if y >= at { step } else { 0.0 };
            if z <= z_top {
                GroundProbe { below: Some(z), above: None }
            } else {
                GroundProbe { below: Some(0.0), above: Some(z) }
            }
        }
    }

    fn run(rb: &mut RigidBody, secs: f32, torque: f32, brake: f32, g: &dyn Fn(f64, f64, f64) -> GroundProbe) {
        let brakes = vec![brake; rb.wheels.len()];
        for _ in 0..(secs * 60.0) as usize {
            rb.step(1.0 / 60.0, torque, &brakes, 0.0, g);
        }
    }

    /// A brake stronger than the road takes locks the wheels (the ABS scripts see the
    /// wheel stand while the bus still moves); a brake the road takes keeps them turning.
    #[test]
    fn a_brake_beyond_the_grip_locks_the_wheels() {
        for (brake, locks) in [(60_000.0, true), (8_000.0, false)] {
            let mut rb = RigidBody::from_definition(&bus(), &[]);
            rb.place(DVec3::ZERO, 0.0);
            let g = road(1e9, 0.0);
            run(&mut rb, 2.0, 0.0, 0.0, &g);
            rb.velocity = rb.orientation.mul_vec3(Vec3::Y) * 12.0;
            for w in rb.wheels.iter_mut() {
                w.spin = 12.0 / w.radius;
            }
            run(&mut rb, 0.6, 0.0, brake, &g);
            let v = rb.velocity.length();
            let locked = rb.wheels.iter().all(|w| w.spin.abs() * w.radius < 0.5);
            assert!(v > 5.0, "still moving: {v}");
            assert_eq!(locked, locks, "brake {brake}: spins {:?} at {v} m/s", rb.wheels.iter().map(|w| w.spin).collect::<Vec<_>>());
        }
    }

    /// The NL202 sits about 0.10 m below its unloaded height - its own `ai_deltaheight` -
    /// with the wheels exactly on the road.
    #[test]
    fn sags_by_its_own_springs() {
        let mut rb = RigidBody::from_definition(&bus(), &[]);
        rb.place(DVec3::ZERO, 0.0);
        let g = road(1e9, 0.0);
        run(&mut rb, 3.0, 0.0, 5000.0, &g);
        let origin = rb.origin();
        assert!((origin.z + 0.097).abs() < 0.01, "origin {origin:?}");
        for w in &rb.wheels {
            assert!((w.compression - 0.097).abs() < 0.015, "compression {}", w.compression);
            assert!(w.ground_z.abs() < 1e-6);
        }
        // a softer air spring lowers it further
        let mut soft = RigidBody::from_definition(&bus(), &[]);
        soft.wheels.iter_mut().for_each(|w| w.spring_factor = 0.5);
        soft.place(DVec3::ZERO, 0.0);
        run(&mut soft, 3.0, 0.0, 5000.0, &g);
        assert!(soft.origin().z < origin.z - 0.08, "{:?}", soft.origin());
    }

    /// A 6 cm hump 0.6 m long (a manhole cover's rim, a speed cushion's edge) crossed at
    /// 40 km/h: the wheels ride over it without leaving the road for long, the body
    /// hardly lifts, and a wheel's travel changes smoothly from frame to frame instead of
    /// jumping into the arch and out to full droop.
    #[test]
    fn a_long_frame_does_not_bounce_the_body() {
        // driving on flat road, one frame held up for 60 ms (loading): the body keeps still
        let mut rb = RigidBody::from_definition(&bus(), &[]);
        rb.place(DVec3::ZERO, 0.0);
        let g = road(1e9, 0.0);
        run(&mut rb, 3.0, 3000.0, 5000.0, &g);
        let before: Vec<f32> = rb.wheels.iter().map(|w| w.compression).collect();
        rb.step(0.1, 3000.0, &[0.0; 4], 0.0, &g);
        let mut worst = 0.0f32;
        for _ in 0..30 {
            rb.step(1.0 / 60.0, 3000.0, &[0.0; 4], 0.0, &g);
            for (w, b) in rb.wheels.iter().zip(&before) {
                worst = worst.max((w.compression - b).abs());
            }
        }
        assert!(worst < 0.02, "the suspension moved {worst} m after a long frame");
    }

    #[test]
    fn rides_over_a_hump() {
        let hump = |_x: f64, y: f64, z_top: f64| {
            let z = if (20.0..20.6).contains(&y) { 0.06 } else { 0.0 };
            if z <= z_top {
                GroundProbe { below: Some(z), above: None }
            } else {
                GroundProbe { below: Some(0.0), above: Some(z) }
            }
        };
        let mut rb = RigidBody::from_definition(&bus(), &[]);
        rb.place(DVec3::ZERO, 0.0);
        run(&mut rb, 2.0, 0.0, 5000.0, &hump);
        rb.velocity = rb.orientation.mul_vec3(Vec3::Y) * (40.0 / 3.6);
        let z0 = rb.position.z;
        let (mut lift, mut jump, mut air) = (0.0f64, 0.0f32, 0usize);
        let mut last: Vec<f32> = rb.wheels.iter().map(|w| w.compression).collect();
        for _ in 0..(3.0 * 60.0) as usize {
            rb.step(1.0 / 60.0, 0.0, &[0.0; 4], 0.0, &hump);
            lift = lift.max(rb.position.z - z0);
            for (k, w) in rb.wheels.iter().enumerate() {
                jump = jump.max((w.compression - last[k]).abs());
                last[k] = w.compression;
                if !w.on_ground {
                    air += 1;
                }
            }
        }
        assert!(rb.origin().y > 30.0, "{:?}", rb.origin());
        assert!(lift < 0.05, "the body lifted {lift:.3} m");
        // (Omsi.exe's wheel stands on the point under it: at the hump's edge its travel
        // takes the hump's height at once, no more)
        assert!(jump < 0.065, "a wheel's travel jumped {jump:.3} m in a frame");
        assert!(air < 16, "wheels off the road for {air} wheel-frames");
    }

    /// Drive at `torque` (N m) for `secs` from rest against the ground `g`; where the front
    /// axle ends up.
    fn climb(g: &dyn Fn(f64, f64, f64) -> GroundProbe, torque: f32, secs: f32) -> (RigidBody, f64) {
        let mut rb = RigidBody::from_definition(&bus(), &[]);
        rb.place(DVec3::ZERO, 0.0);
        run(&mut rb, 1.0, 0.0, 5000.0, g);
        run(&mut rb, secs, torque, 0.0, g);
        let front = rb.origin().y + 3.238;
        (rb, front)
    }

    /// A 15 cm kerb takes a firm push from a standstill (full torque: about 20 kN at the
    /// rear wheels) but not the idle creep of 3 kN; a lowered kerb of 3 cm the creep takes;
    /// a 60 cm wall stops the bus whatever it does.
    #[test]
    fn climbs_a_kerb_but_not_a_wall() {
        let kerb = road(6.0, 0.15);
        let (rb, front) = climb(&kerb, 9500.0, 10.0);
        assert!(front > 9.0, "front axle at {front}");
        assert!((rb.wheels[0].ground_z - 0.15).abs() < 0.01, "front wheel on the kerb: {}", rb.wheels[0].ground_z);
        let (_, front) = climb(&kerb, 1400.0, 10.0);
        assert!(front < 6.0, "idle creep climbed the kerb: front axle at {front}");
        let lowered = road(6.0, 0.03);
        let (_, front) = climb(&lowered, 1400.0, 15.0);
        assert!(front > 7.0, "idle creep stuck at a lowered kerb: front axle at {front}");

        let mut rb = RigidBody::from_definition(&bus(), &[]);
        rb.place(DVec3::ZERO, 0.0);
        let wall = road(6.0, 0.6);
        run(&mut rb, 1.0, 0.0, 5000.0, &wall);
        run(&mut rb, 12.0, 6000.0, 0.0, &wall);
        let front = rb.origin().y + 3.238;
        assert!(front < 6.0 && front > 5.3, "front axle at {front}");
        // and it backs off at once
        let y0 = rb.origin().y;
        run(&mut rb, 3.0, -6000.0, 0.0, &wall);
        assert!(rb.origin().y < y0 - 1.0, "{} -> {}", y0, rb.origin().y);
    }

    /// A pusher's front section has no driven wheels: the push of the rear section's axle
    /// comes through the joint and moves it like the same torque on its own axle, the towed
    /// mass slows the start, and the brakes of either part hold the train.
    #[test]
    fn pusher_front_section_is_pushed_through_the_joint() {
        let mut def = bus();
        def.axles.iter_mut().for_each(|a| a.driven = false);
        let rear = CoupledPart { point: Vec3::new(0.0, -4.456, 0.503), dir: Vec3::Y, mass: 0.0, driven_wheels: 2, radius: 0.47, driven_load: 70_000.0, brake: 0.0, load: 70_000.0 };
        let g = road(1e9, 0.0);
        let start = |def: &Vehicle, parts: Vec<CoupledPart>| {
            let mut rb = RigidBody::from_definition(def, &[]);
            rb.coupled = parts;
            rb.place(DVec3::ZERO, 0.0);
            run(&mut rb, 1.0, 0.0, 5000.0, &g);
            rb
        };
        // held by the front section's brakes, then by the rear section's alone
        let mut rb = start(&def, vec![rear]);
        run(&mut rb, 2.0, 9000.0, 20_000.0, &g);
        assert!(rb.origin().y.abs() < 0.05, "crept to {:?}", rb.origin());
        rb.coupled[0].brake = 80_000.0;
        run(&mut rb, 2.0, 9000.0, 0.0, &g);
        assert!(rb.origin().y.abs() < 0.05, "crept to {:?}", rb.origin());
        rb.coupled[0].brake = 0.0;
        run(&mut rb, 5.0, 9000.0, 0.0, &g);
        let pushed = rb.origin().y;
        // the same torque on its own rear axle
        let mut own = start(&bus(), Vec::new());
        run(&mut own, 5.0, 9000.0, 0.0, &g);
        let driven = own.origin().y;
        assert!(pushed > 5.0 && (pushed - driven).abs() < 0.1 * driven, "pushed {pushed} m, driven {driven} m");
        let (heading, _, _) = rb.heading_pitch_bank();
        assert!(rb.origin().x.abs() < 0.05 && heading.min(360.0 - heading) < 0.5, "{:?} {heading}", rb.origin());
        // the rear section's brakes stop the train
        let v = rb.forward_speed();
        rb.coupled[0].brake = 30_000.0;
        run(&mut rb, 1.0, 0.0, 0.0, &g);
        assert!(rb.forward_speed() < v - 2.0, "{v} -> {}", rb.forward_speed());
        // and backwards
        rb.coupled[0].brake = 0.0;
        run(&mut rb, 1.0, 0.0, 20_000.0, &g);
        let y0 = rb.origin().y;
        run(&mut rb, 6.0, -9000.0, 0.0, &g);
        assert!(rb.origin().y < y0 - 1.0, "{} -> {}", y0, rb.origin().y);
        // a towed mass as heavy as the body halves the acceleration
        let mut heavy = start(&def, vec![CoupledPart { mass: rb.mass, ..rear }]);
        run(&mut heavy, 5.0, 9000.0, 0.0, &g);
        let ratio = heavy.origin().y / pushed;
        assert!((ratio - 0.5).abs() < 0.08, "heavy train went {} m, light {pushed} m", heavy.origin().y);
        // a joint bent by 30 degrees pushes with its share along the body and does not turn
        // it, standing against the brakes or driving
        let bent_dir = Vec3::new(0.5, 0.866, 0.0);
        let mut bent = start(&def, vec![CoupledPart { dir: bent_dir, brake: 30_000.0, ..rear }]);
        run(&mut bent, 5.0, 9000.0, 20_000.0, &g);
        let (heading, _, _) = bent.heading_pitch_bank();
        assert!(bent.origin().truncate().length() < 0.05 && heading.min(360.0 - heading) < 0.2, "{:?} {heading}", bent.origin());
        bent.coupled[0].brake = 0.0;
        run(&mut bent, 5.0, 9000.0, 0.0, &g);
        let (heading, _, _) = bent.heading_pitch_bank();
        let ratio = bent.origin().y / pushed;
        assert!((ratio - 0.866).abs() < 0.08 && heading.min(360.0 - heading) < 0.5, "{:?} {heading} ({ratio})", bent.origin());
    }

    /// A towed part on a grade pulls the body down it, and its brakes hold it there.
    #[test]
    fn towed_weight_on_a_grade() {
        let mut def = bus();
        def.axles.iter_mut().for_each(|a| a.driven = false);
        let g = ramp(0.08);
        let part = CoupledPart { point: Vec3::new(0.0, -4.456, 0.503), dir: Vec3::Y, mass: 7000.0, driven_wheels: 0, radius: 0.47, driven_load: 0.0, brake: 0.0, load: 70_000.0 };
        let mut rb = RigidBody::from_definition(&def, &[]);
        rb.coupled = vec![CoupledPart { brake: 40_000.0, ..part }];
        rb.place(DVec3::new(0.0, 50.0, 4.0), 0.0);
        run(&mut rb, 3.0, 0.0, 0.0, &g);
        let y0 = rb.origin().y;
        run(&mut rb, 3.0, 0.0, 0.0, &g);
        assert!((rb.origin().y - y0).abs() < 0.02, "rolled from {y0} to {}", rb.origin().y);
        rb.coupled[0].brake = 0.0;
        run(&mut rb, 3.0, 0.0, 0.0, &g);
        assert!(rb.origin().y < y0 - 1.0, "stayed at {}", rb.origin().y);
    }

    /// A ramp of grade `s` along y (from y = 0), steps over `CLIMB` radii read as faces.
    fn ramp(s: f64) -> impl Fn(f64, f64, f64) -> GroundProbe {
        move |_x, y, top| {
            let z = y * s;
            if z <= top { GroundProbe { below: Some(z), above: None } } else { GroundProbe { below: None, above: Some(z) } }
        }
    }

    /// A kerb running along under the tread lifts the hub by the share of the tread over
    /// it, smoothly as the edge passes under the tyre - not all at once as the edge passes
    /// the middle - and lets it down the same way from the other side.
    #[test]
    fn kerb_along_the_tread_lifts_smoothly() {
        let r = 0.47f32;
        let h = 0.15;
        let mut last: Option<f64> = None;
        let mut max_jump = 0.0f64;
        for k in 0..=200 {
            // the kerb's edge from 0.3 m right of the hub to 0.3 m left of it
            let e = 0.3 - k as f64 * 0.003;
            let probe = move |x: f64, _y: f64, top: f64| {
                let z = if x > e { h } else { 0.0 };
                if z <= top { GroundProbe { below: Some(z), above: None } } else { GroundProbe { below: None, above: Some(z) } }
            };
            let hub = DVec3::new(0.0, 0.0, r as f64);
            let z_top = (CLIMB * r) as f64;
            let line = tyre_line(&probe, hub, Vec3::Y, r, z_top, hub.z + WALL_HEIGHT, 0.0);
            let (need, _, _) = line.ground.expect("ground");
            let need = need + tread_step(&probe, hub, Vec3::Y, r, z_top);
            if let Some(l) = last {
                max_jump = max_jump.max((need - l).abs());
            }
            last = Some(need);
        }
        assert!((last.unwrap() - (r as f64 + h)).abs() < 1e-3, "ends on the kerb: {:?}", last);
        assert!(max_jump < 0.02, "a jump of {max_jump:.3} m");
    }

    /// On a uniform grade of any steepness the tyre reports the grade itself and stands where
    /// the envelope touches the ramp, with no more probes than on the flat. The old comb read
    /// every grade over 6.4 % as a run of kerbs: 0.043 for 6.5-9 %, 0.13 for 10-15 %, and 73
    /// probes a wheel.
    #[test]
    fn tyre_reads_any_grade_as_a_grade() {
        let r = 0.47f32;
        let a = (r * ENVELOPE) as f64;
        for pct in [0.0, 2.0, 5.0, 6.3, 6.5, 8.0, 9.0, 10.0, 12.0, 15.0, 20.0, 25.0] {
            let s = pct / 100.0;
            let count = std::cell::Cell::new(0);
            let g = ramp(s);
            let probe = |x: f64, y: f64, top: f64| {
                count.set(count.get() + 1);
                g(x, y, top)
            };
            for k in 0..40 {
                let y = 100.0 + k as f64 * 0.037;
                let hub = DVec3::new(0.0, y, y * s + r as f64);
                let z_top = y * s + (CLIMB * r) as f64 + 0.05;
                count.set(0);
                let line = tyre_line(&probe, hub, Vec3::Y, r, z_top, hub.z + WALL_HEIGHT, 0.0);
                let (need, slope, x) = line.ground.expect("ground");
                // the envelope touches where its own slope is the grade
                let t = s * a / r as f64;
                let xs = a * t / (1.0 + t * t).sqrt();
                let want = s * (y + xs) + r as f64 * (1.0 - (xs / a).powi(2)).sqrt();
                assert!((slope as f64 - s).abs() < 1e-4, "{pct} %: slope {slope} at {y}");
                assert!((need - want).abs() < 1e-5, "{pct} %: need {need}, want {want}");
                assert!((x as f64 - xs).abs() < 1e-3, "{pct} %: contact at {x}, want {xs}");
                assert!(line.faces == [None, None] && line.clear);
                assert!(count.get() <= LATTICE, "{pct} %: {} probes", count.get());
            }
        }
    }

    /// Walked over a step or a chamfered face (on the level or on a grade), the push the
    /// slope gives takes exactly what the hub is lifted by (∫ slope dx = Δneed), and the hub
    /// never jumps as the samples move on: the old corner slope gave back only 0.43 of the
    /// load for 0.6 of it on a steep face and pumped the body up.
    #[test]
    fn climbing_a_face_costs_what_it_lifts() {
        let r = 0.47f32;
        for (grade, height, angle) in [(0.0, 0.15, 90.0f64), (0.0, 0.2, 72.0), (0.0, 0.2, 60.0), (0.0, 0.1, 45.0), (0.0, 0.3, 80.0), (0.0, 0.05, 20.0), (0.0, 0.37, 90.0), (0.0, 0.37, 70.0), (0.1, 0.15, 90.0), (-0.08, 0.2, 75.0)] {
            let run = height / angle.to_radians().tan();
            let face = move |_x: f64, y: f64, top: f64| {
                let z = grade * y + ((y - 5.0) / run.max(1e-9)).clamp(0.0, 1.0) * height;
                if z <= top { GroundProbe { below: Some(z), above: None } } else { GroundProbe { below: Some(0.0), above: Some(z) } }
            };
            let need_at = |y: f64| {
                let hub = DVec3::new(0.3, y, r as f64 + grade * y);
                let top = hub.z - r as f64 + (CLIMB * r) as f64;
                tyre_line(&face, hub, Vec3::Y, r, top.max(grade * 5.0 + height), hub.z + WALL_HEIGHT, 0.0).ground.expect("ground")
            };
            let (y0, y1, dy) = (2.5, 7.0, 0.0005);
            let (mut work, mut y, mut need) = (0.0, y0, need_at(y0).0);
            while y < y1 {
                let slope = need_at(y + dy / 2.0).1 as f64;
                work += slope * dy;
                y += dy;
                let next = need_at(y).0;
                assert!((next - need - slope * dy).abs() < 1e-3, "{height} m at {angle}° on {grade}: the hub jumped by {} at {y}", next - need - slope * dy);
                need = next;
            }
            let lift = need_at(y).0 - need_at(y0).0;
            let want = height + grade * (y - y0);
            assert!((lift - want).abs() < 1e-3, "{height} m at {angle}° on {grade}: lifted {lift}, want {want}");
            assert!((work - lift).abs() < 0.003 * height.max(0.1), "{height} m at {angle}° on {grade}: pushed back {work} for {lift}");
        }
    }

    /// A step asks the ground for a few points per tyre - the lattice and a look to either
    /// side - on the flat and on any grade alike (the comb read every grade over 6.4 % as a
    /// run of kerbs and asked for 73 per tyre).
    #[test]
    fn a_step_asks_for_few_points() {
        for s in [0.0, 0.05, 0.08, 0.12, 0.2] {
            let count = std::cell::Cell::new(0);
            let g = ramp(s);
            let probe = |x: f64, y: f64, top: f64| {
                count.set(count.get() + 1);
                g(x, y, top)
            };
            let mut rb = RigidBody::from_definition(&bus(), &[]);
            rb.place(DVec3::new(0.0, 10.0, 10.0 * s), 0.0);
            run(&mut rb, 1.0, 0.0, 30_000.0, &probe);
            count.set(0);
            rb.step(1.0 / 60.0, 0.0, &[30_000.0; 4], 0.0, &probe);
            // (and three across the tread, `tread_step`, and the point under the wheel
            // Omsi.exe's suspension stands on)
            assert!(count.get() <= 4 * rb.wheels.len() * (LATTICE + 6), "{s}: {} probes", count.get());
        }
    }

    /// Released on grades from 4 % to 16 %, the bus rolls down with gravity's full pull along
    /// the slope less the rolling resistance.
    #[test]
    fn rolls_down_a_grade_at_its_own_pace() {
        for pct in [4.0f64, 8.0, 12.0, 16.0] {
            let s = pct / 100.0;
            let g = ramp(s);
            let mut rb = RigidBody::from_definition(&bus(), &[]);
            rb.place(DVec3::ZERO, 0.0);
            run(&mut rb, 3.0, 0.0, 30_000.0, &g);
            run(&mut rb, 0.5, 0.0, 0.0, &g);
            let v0 = rb.forward_speed();
            run(&mut rb, 2.0, 0.0, 0.0, &g);
            let accel = (rb.forward_speed() - v0) / 2.0;
            let want = -(9.81 * s / (1.0 + s * s).sqrt()) as f32 + 1000.0 / rb.mass;
            assert!((accel - want).abs() < 0.04 * want.abs(), "{pct} %: {accel} m/s², want {want}");
        }
    }

    /// A step close to the tyre's radius (0.45 m on 0.47 m, which the hub-and-5-cm limit
    /// still climbed) is a wall, whatever the push or the run-up; one of 0.3 m is climbed
    /// with a run-up.
    #[test]
    fn a_step_near_the_radius_is_a_wall() {
        let step = road(6.0, 0.45);
        let (rb, front) = climb(&step, 9500.0, 10.0);
        assert!(front < 6.0 - 0.4, "front axle at {front}");
        assert!(rb.wheels[0].ground_z.abs() < 0.01, "on the step: {}", rb.wheels[0].ground_z);
        let mut rb = RigidBody::from_definition(&bus(), &[]);
        rb.place(DVec3::ZERO, 0.0);
        run(&mut rb, 1.0, 0.0, 5000.0, &step);
        rb.velocity = Vec3::new(0.0, 15.0 / 3.6, 0.0);
        run(&mut rb, 5.0, 9500.0, 0.0, &step);
        let front = rb.origin().y + 3.238;
        assert!(front < 6.0 - 0.4, "front axle at {front} after a run-up");
        let low = road(6.0, 0.3);
        let mut rb = RigidBody::from_definition(&bus(), &[]);
        rb.place(DVec3::ZERO, 0.0);
        run(&mut rb, 1.0, 0.0, 5000.0, &low);
        rb.velocity = Vec3::new(0.0, 15.0 / 3.6, 0.0);
        run(&mut rb, 5.0, 9500.0, 0.0, &low);
        assert!(rb.origin().y > 6.0, "stuck at {} before a 0.3 m step", rb.origin().y);
        assert!(rb.wheels.iter().all(|w| (w.ground_z - 0.3).abs() < 0.01), "{:?}", rb.wheels.iter().map(|w| w.ground_z).collect::<Vec<_>>());
    }

    /// A platform's face stops the front wheels at any speed and frame rate (the body has no
    /// collision box here: a spline's faces are not in the collision world), without
    /// throwing the bus about.
    #[test]
    fn a_face_stops_the_wheels_at_any_speed() {
        let wall = road(6.0, 0.6);
        for fps in [60.0f32, 30.0, 20.0] {
            for kmh in [10.0f32, 30.0, 50.0, 80.0] {
                let mut rb = RigidBody::from_definition(&bus(), &[]);
                rb.place(DVec3::new(0.0, -10.0, 0.0), 0.0);
                run(&mut rb, 1.0, 0.0, 5000.0, &wall);
                rb.velocity = Vec3::new(0.0, kmh / 3.6, 0.0);
                let kinetic = rb.kinetic_energy();
                let (mut front, mut top, mut crash) = (f64::MIN, f64::MIN, 0.0);
                for _ in 0..(fps * 6.0) as usize {
                    rb.step(1.0 / fps, 0.0, &[0.0; 4], 0.0, &wall);
                    front = front.max(rb.origin().y + 3.238);
                    top = top.max(rb.origin().z);
                    for hit in rb.wheel_impacts.iter().filter(|h| h.speed > CRASH_SPEED && h.energy > 1000.0) {
                        assert!(hit.obstacle < 2 && hit.point.y > 3.238 && hit.point.z < 1.1, "{hit:?}");
                        crash += hit.energy;
                    }
                }
                // the crash is the wheels' (the bus has no box here), and it took most of the
                // bus's motion
                assert!(crash > 0.5 * kinetic && crash < 1.05 * kinetic, "{kmh} km/h at {fps} fps: crash of {crash} J for {kinetic} J");
                assert!(front < 6.0 - 0.47 + 0.03, "{kmh} km/h at {fps} fps: front axle reached {front}");
                assert!(top < 0.3, "{kmh} km/h at {fps} fps: thrown up to {top}");
                // (a rebound of a metre or so: with the moments of inertia on Omsi.exe's axes
                // the body's lower yaw inertia takes a little more of the blow back)
                assert!(rb.forward_speed().abs() < 1.5, "{kmh} km/h at {fps} fps: still at {}", rb.forward_speed());
            }
        }
    }

    /// Sliding sideways into a platform along the bus, the tyres stop at its face instead of
    /// running under it.
    #[test]
    fn a_face_beside_the_tyres_stops_a_slide() {
        let platform = |x: f64, _y: f64, top: f64| {
            let z = if x >= 1.5 { 0.6 } else { 0.0 };
            if z <= top { GroundProbe { below: Some(z), above: None } } else { GroundProbe { below: Some(0.0), above: Some(z) } }
        };
        let mut rb = RigidBody::from_definition(&bus(), &[]);
        rb.place(DVec3::ZERO, 0.0);
        run(&mut rb, 1.0, 0.0, 0.0, &platform);
        rb.velocity = Vec3::new(8.0, 0.0, 0.0);
        let mut right = f64::MIN;
        for _ in 0..120 {
            rb.step(1.0 / 30.0, 0.0, &[0.0; 4], 0.0, &platform);
            for w in rb.wheels.iter().filter(|w| w.attach.x > 0.0) {
                let hub = rb.position + rb.orientation.mul_vec3(w.attach - rb.cog).as_dvec3();
                right = right.max(hub.x);
            }
        }
        let limit = 1.5 - (HALF_WIDTH * 0.47) as f64;
        assert!(right < limit + 0.02, "a right hub reached {right} (face at 1.5)");
        assert!(rb.wheels.iter().filter(|w| w.attach.x > 0.0).all(|w| !w.walls.is_empty()), "{:?}", rb.wheels.iter().map(|w| &w.walls).collect::<Vec<_>>());
        let (_, _, bank) = rb.heading_pitch_bank();
        assert!(bank.abs() < 5.0, "banked {bank}");
    }

    /// On an 8 % slope the parking brake (rear wheels only) holds the bus; released, it rolls
    /// back down; and a driven bus brakes to a stand on the flat.
    #[test]
    fn holds_on_a_slope_and_brakes_to_a_stand() {
        let slope = |_x: f64, y: f64, top: f64| {
            let z = y * 0.08;
            if z <= top { GroundProbe { below: Some(z), above: None } } else { GroundProbe { below: None, above: Some(z) } }
        };
        let mut rb = RigidBody::from_definition(&bus(), &[]);
        rb.place(DVec3::ZERO, 0.0);
        let park = [0.0, 0.0, 20_000.0, 20_000.0];
        for _ in 0..180 {
            rb.step(1.0 / 60.0, 0.0, &park, 0.0, &slope);
        }
        let y0 = rb.origin().y;
        for _ in 0..600 {
            rb.step(1.0 / 60.0, 0.0, &park, 0.0, &slope);
        }
        let y1 = rb.origin().y;
        for _ in 0..600 {
            rb.step(1.0 / 60.0, 0.0, &park, 0.0, &slope);
        }
        eprintln!("parked on 8 %: {:+.4} m in the first 10 s, {:+.4} m in the next", y1 - y0, rb.origin().y - y1);
        assert!((rb.origin().y - y1).abs() < 0.005, "crept {} m", rb.origin().y - y1);
        for _ in 0..180 {
            rb.step(1.0 / 60.0, 0.0, &[0.0; 4], 0.0, &slope);
        }
        assert!(rb.origin().y < y0 - 1.0 && rb.forward_speed() < -0.5, "rolled back to {} at {}", rb.origin().y, rb.forward_speed());

        let flat = road(1e9, 0.0);
        let mut rb = RigidBody::from_definition(&bus(), &[]);
        rb.place(DVec3::ZERO, 0.0);
        run(&mut rb, 0.5, 0.0, 0.0, &flat);
        rb.velocity = Vec3::new(0.0, 50.0 / 3.6, 0.0);
        let mut t = 0.0;
        while rb.forward_speed() > 0.01 && t < 10.0 {
            rb.step(1.0 / 60.0, 0.0, &[15_000.0; 4], 0.0, &flat);
            t += 1.0 / 60.0;
        }
        // 60 kN on 10.2 t: about 5.9 m/s², so 50 km/h are gone in some 2.4 s
        assert!(t > 2.0 && t < 3.0, "stopped after {t} s");
        let y = rb.origin().y;
        run(&mut rb, 3.0, 0.0, 15_000.0, &flat);
        assert!((rb.origin().y - y).abs() < 0.02, "moved {} m after stopping", rb.origin().y - y);
    }

    /// The same full braking from 50 km/h on a dry road and on black ice: the tyres cannot
    /// pass the brakes' 5.9 m/s² on to the ice, and the bus slides about five times as far.
    #[test]
    fn brakes_slide_on_ice() {
        let stop = |mu: f32| {
            let flat = road(1e9, 0.0);
            let mut rb = RigidBody::from_definition(&bus(), &[]);
            rb.place(DVec3::ZERO, 0.0);
            run(&mut rb, 0.5, 0.0, 0.0, &flat);
            rb.friction = mu;
            rb.velocity = Vec3::new(0.0, 50.0 / 3.6, 0.0);
            let y0 = rb.origin().y;
            let mut t = 0.0;
            while rb.forward_speed() > 0.01 && t < 30.0 {
                rb.step(1.0 / 60.0, 0.0, &[15_000.0; 4], 0.0, &flat);
                t += 1.0 / 60.0;
            }
            rb.origin().y - y0
        };
        let (dry, ice) = (stop(crate::vehicle::road_grip(0.0, 15.0)), stop(crate::vehicle::road_grip(0.4, -6.0)));
        eprintln!("50 km/h to a stand: {dry:.1} m dry, {ice:.1} m on ice");
        assert!(dry > 12.0 && dry < 20.0, "dry {dry}");
        assert!(ice > dry * 4.0, "ice {ice} dry {dry}");
    }

    /// Straight into a wall at 40 km/h: the bus stops at the wall, bounces off a little, and
    /// drives away backwards straight after.
    #[test]
    fn crash_bounces_and_releases() {
        let def = bus();
        let bb = def.bounding_box.unwrap();
        let mut rb = RigidBody::from_definition(&def, &[]);
        rb.place(DVec3::ZERO, 0.0);
        let g = road(1e9, 0.0);
        run(&mut rb, 1.0, 0.0, 5000.0, &g);
        rb.velocity = Vec3::new(0.0, 40.0 / 3.6, 0.0);
        let wall = [Obb::from_box([20.0, 1.0, 3.0, 0.0, 0.0, 1.5], DVec3::new(0.0, 12.0, 0.0), 0.0)];
        let mut impacts = Vec::new();
        for _ in 0..90 {
            rb.step(1.0 / 30.0, 0.0, &[0.0; 4], 0.0, &g);
            impacts.extend(rb.collide(bb, &wall, &|_| false, 1.0 / 30.0));
        }
        assert_eq!(impacts.iter().filter(|i| i.speed > CRASH_SPEED).count(), 1, "{impacts:?}");
        let hit = impacts[0];
        assert!(hit.speed > 10.0 && hit.energy > 400_000.0, "{hit:?}");
        assert!(hit.point.y > 5.5, "{hit:?}");
        assert!(rb.forward_speed() < 0.0, "bounced: {}", rb.forward_speed());
        let front = rb.origin().y + 5.83;
        assert!(front < 11.5 + 0.01, "{front}");
        // reverse away at once
        let y0 = rb.origin().y;
        for _ in 0..60 {
            rb.step(1.0 / 30.0, -8000.0, &[0.0; 4], 0.0, &g);
            rb.collide(bb, &wall, &|_| false, 1.0 / 30.0);
        }
        assert!(rb.origin().y < y0 - 1.0, "{} -> {}", y0, rb.origin().y);
    }

    /// A glancing blow on the front corner turns the bus away from the wall and lets it slide.
    #[test]
    fn glancing_blow_turns_the_bus() {
        let def = bus();
        let bb = def.bounding_box.unwrap();
        let mut rb = RigidBody::from_definition(&def, &[]);
        rb.place(DVec3::ZERO, 0.0);
        let g = road(1e9, 0.0);
        run(&mut rb, 1.0, 0.0, 5000.0, &g);
        // heading 20° to the right towards a wall along x = 4
        rb.place(DVec3::ZERO, 20.0);
        run(&mut rb, 0.5, 0.0, 0.0, &g);
        rb.velocity = rb.orientation.mul_vec3(Vec3::Y) * 30.0 / 3.6;
        let wall = [Obb::from_box([1.0, 60.0, 3.0, 0.0, 0.0, 1.5], DVec3::new(4.5, 10.0, 0.0), 0.0)];
        let mut hits = 0;
        for _ in 0..60 {
            rb.step(1.0 / 30.0, 0.0, &[0.0; 4], 0.0, &g);
            hits += rb.collide(bb, &wall, &|_| false, 1.0 / 30.0).len();
        }
        let (heading, _, _) = rb.heading_pitch_bank();
        assert!(hits > 0);
        assert!(heading < 15.0 || heading > 300.0, "turned away from the wall: {heading}");
        assert!(rb.velocity.y > 1.0, "still sliding along: {:?}", rb.velocity);
    }

    /// A car (1 t, kinematic: it keeps its speed) drives into the side of the standing bus
    /// at 50 km/h: the bus gets a nudge and a crash, not a shove across the road.
    #[test]
    fn a_car_does_not_bulldoze_the_bus() {
        let def = bus();
        let bb = def.bounding_box.unwrap();
        let mut rb = RigidBody::from_definition(&def, &[]);
        rb.place(DVec3::ZERO, 0.0);
        let g = road(1e9, 0.0);
        run(&mut rb, 1.0, 0.0, 5000.0, &g);
        let (x0, mut hits, mut energy) = (rb.origin().x, 0, 0.0f32);
        for k in 0..30 {
            // from the left, heading east
            let car = Obb::from_box([1.7, 4.2, 1.4, 0.0, 0.0, 0.7], DVec3::new(-4.0 + 50.0 / 3.6 * k as f64 / 30.0, 0.0, 0.0), 90.0).moving(glam::DVec2::new(50.0 / 3.6, 0.0), 1000.0, 7);
            rb.step(1.0 / 30.0, 0.0, &[5000.0; 4], 0.0, &g);
            for hit in rb.collide(bb, &[car], &|_| false, 1.0 / 30.0) {
                hits += 1;
                energy += hit.energy;
            }
        }
        run(&mut rb, 2.0, 0.0, 5000.0, &g);
        let (_, _, bank) = rb.heading_pitch_bank();
        assert!(hits > 0 && energy > 20_000.0, "{hits} hits, {energy} J");
        assert!((rb.origin().x - x0).abs() < 1.0, "shoved {} m", rb.origin().x - x0);
        assert!(bank.abs() < 3.0, "banked {bank}");
    }

    /// A car driving into the standing bus is one crash per call, however long it stays
    /// inside the bus: 0.5 v² (1 - e²) over the pair's inverse mass, not four times that.
    #[test]
    fn a_car_strike_counts_once() {
        let def = bus();
        let bb = def.bounding_box.unwrap();
        let mut rb = RigidBody::from_definition(&def, &[]);
        rb.place(DVec3::ZERO, 0.0);
        let g = road(1e9, 0.0);
        run(&mut rb, 1.0, 0.0, 5000.0, &g);
        let v = 50.0f32 / 3.6;
        let car = Obb::from_box([1.7, 4.2, 1.4, 0.0, 0.0, 0.7], DVec3::new(-2.0, 0.0, 0.0), 90.0).moving(glam::DVec2::new(v as f64, 0.0), 1000.0, 7);
        let hits = rb.collide(bb, &[car], &|_| false, 1.0 / 30.0);
        assert_eq!(hits.len(), 1, "{hits:?}");
        let n = Vec3::X;
        let r = Vec3::new(-1.236, 0.0, 0.0);
        let k = rb.inv_mass_at(r, n) + 1.0 / 1000.0;
        let want = 0.5 * v * v * (1.0 - RESTITUTION * RESTITUTION) / k;
        assert!((hits[0].energy - want).abs() < 0.15 * want, "{} J, want about {want}", hits[0].energy);
        assert!(hits[0].energy < 100_000.0, "{hits:?}");
    }

    /// The scripts hear where the bodies meet - low down at the bumpers - not the height of
    /// the centre of gravity: reversing into a wall or a car is below 1.10 m and behind
    /// -4.70 m, where the stock buses damage their engine; a high beam is hit high.
    #[test]
    fn impacts_are_reported_at_bumper_height() {
        let def = bus();
        let bb = def.bounding_box.unwrap();
        let hit = |obstacle: Obb, v: Vec3| {
            let mut rb = RigidBody::from_definition(&def, &[]);
            rb.place(DVec3::ZERO, 0.0);
            let g = road(1e9, 0.0);
            run(&mut rb, 1.0, 0.0, 5000.0, &g);
            rb.velocity = v;
            for _ in 0..60 {
                rb.step(1.0 / 30.0, 0.0, &[0.0; 4], 0.0, &g);
                if let Some(h) = rb.collide(bb, &[obstacle], &|_| false, 1.0 / 30.0).into_iter().find(|h| h.speed > CRASH_SPEED) {
                    return h;
                }
            }
            panic!("no hit on {obstacle:?}");
        };
        let back = Vec3::new(0.0, -15.0 / 3.6, 0.0);
        let wall = hit(Obb::from_box([20.0, 1.0, 5.0, 0.0, 0.0, 2.5], DVec3::new(0.0, -8.0, 0.0), 0.0), back);
        assert!(wall.point.z < 1.1 && wall.point.z > 0.5 && wall.point.y < -4.7, "{wall:?}");
        let car = hit(Obb::from_box([1.7, 4.2, 1.4, 0.0, 0.0, 0.7], DVec3::new(0.0, -9.0, 0.0), 0.0).moving(glam::DVec2::ZERO, 1000.0, 3), back);
        assert!(car.point.z < 1.1 && car.point.y < -4.7, "{car:?}");
        let front = hit(Obb::from_box([20.0, 1.0, 5.0, 0.0, 0.0, 2.5], DVec3::new(0.0, 8.0, 0.0), 0.0), -back);
        assert!(front.point.z < 1.1 && front.point.y > 4.7, "{front:?}");
        let beam = hit(Obb::from_box([20.0, 1.0, 1.0, 0.0, 0.0, 2.5], DVec3::new(0.0, 8.0, 0.0), 0.0), -back);
        assert!(beam.point.z > 2.0, "{beam:?}");
    }

    /// Into the back of a standing car at 30 km/h: the car (1 t, which the AI holds where it
    /// is) stops the bus within a few metres instead of being driven through.
    #[test]
    fn rear_ends_a_standing_car() {
        let def = bus();
        let bb = def.bounding_box.unwrap();
        let mut rb = RigidBody::from_definition(&def, &[]);
        rb.place(DVec3::ZERO, 0.0);
        let g = road(1e9, 0.0);
        run(&mut rb, 1.0, 0.0, 5000.0, &g);
        rb.velocity = Vec3::new(0.0, 30.0 / 3.6, 0.0);
        let car = [Obb::from_box([1.7, 4.2, 1.4, 0.0, 0.0, 0.7], DVec3::new(0.0, 12.0, 0.0), 0.0).moving(glam::DVec2::ZERO, 1000.0, 3)];
        let mut first = None;
        for k in 0..90 {
            rb.step(1.0 / 30.0, 0.0, &[0.0; 4], 0.0, &g);
            if !rb.collide(bb, &car, &|_| false, 1.0 / 30.0).is_empty() && first.is_none() {
                first = Some((k, rb.origin().y));
            }
        }
        let (_, y_hit) = first.expect("hit the car");
        let front = rb.origin().y + 5.83;
        assert!(rb.forward_speed().abs() < 0.3, "still at {} km/h", rb.forward_speed() * 3.6);
        assert!(front < 12.0 - 2.1 + 0.3, "drove into the car: front at {front}");
        assert!(rb.origin().y - y_hit < 3.0, "pushed on {} m", rb.origin().y - y_hit);
    }

    /// A lamp post breaks off: the bus hardly slows down.
    #[test]
    fn a_post_breaks_off() {
        let def = bus();
        let bb = def.bounding_box.unwrap();
        let mut rb = RigidBody::from_definition(&def, &[]);
        rb.place(DVec3::ZERO, 0.0);
        let g = road(1e9, 0.0);
        run(&mut rb, 1.0, 0.0, 5000.0, &g);
        rb.velocity = Vec3::new(0.0, 30.0 / 3.6, 0.0);
        let mut post = Obb::from_box([0.2, 0.2, 6.0, 0.0, 0.0, 3.0], DVec3::new(0.5, 10.0, 0.0), 0.0);
        post.pole = Some((0.05, 0.5));
        let mut broke = false;
        for _ in 0..30 {
            rb.step(1.0 / 30.0, 0.0, &[0.0; 4], 0.0, &g);
            let hits = rb.collide(bb, &[post], &|_| broke, 1.0 / 30.0);
            broke |= hits.iter().any(|h| h.broke);
        }
        assert!(broke);
        assert!(rb.forward_speed() > 25.0 / 3.6, "{}", rb.forward_speed() * 3.6);
    }
}
