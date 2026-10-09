//! The animation of Omsi.exe's people (`THumanBeingInst`, sub_626ae8), as the original
//! computes it: thirty joint angles from the walk phase and what the person is doing,
//! turned into the thirteen bone matrices by plain rotations about the `[links]` joints.
//! There is no inverse kinematics for the legs: a foot goes where the angles put it. The
//! one place the original solves a limb is the right arm reaching for the validator or the
//! cash desk (and the head turning to the driver), with the law of cosines - kept here.
//!
//! Everything is done in Direct3D's frame and with D3DX's matrices (row vectors, `v' = v *
//! M`, left-handed: x right, y up, z forward) so that the signs and the order of the
//! rotations are the original's; [`OmsiAnim::bones`] hands the result over in this
//! engine's model frame (x right, y forward, z up).
//!
//! Inputs per frame (fields of the human, see [`AnimInput`]):
//! * `PAX_State` (+0x64c) rounded: 0 standing, 1 walking, 2 sitting;
//! * the speed (+0x6a4) and the distance moved this frame (`LastMovedDist`, +0x644);
//! * the room height of the path link walked on (+0x668, 50 outside a vehicle);
//! * `HeightOfSeat` (+0x648) of the seat sat on;
//! * the right hand's target (+0x665, +0x670) and the head's (+0x666, +0x688) in the
//!   person's own frame.
//!
//! The walk: the phase (+0x66c) runs 0..2 over two steps, advanced by the distance moved
//! divided by the stride (`[walk_param]` line 1 times the speed / 1.2 m/s, at most 1); the
//! thighs follow a fixed curve of the phase (0: 0.75, 0.2: 1.1, 0.8: -1.1, 1: 0.75, set up
//! in sub_624820) and the knees another (1, 0, 0, 1), the other leg half a cycle behind;
//! the pelvis bobs, the hips and the torso turn with sin(2*pi*phase) and the arms swing
//! with it.

use glam::{Affine3A, Mat4, Vec3, Vec4};
use omsi_content::Human;

/// Bone slots: the engine ids -2 .. -14 of `[setbone]` in that order (see `human.rs`).
pub const BONES: usize = 13;

/// A D3DX matrix: rows, row vectors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DMat(pub [[f32; 4]; 4]);

impl DMat {
    pub const IDENTITY: DMat = DMat([
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]);

    /// D3DXMatrixTranslation.
    pub fn translation(v: Vec3) -> DMat {
        let mut m = DMat::IDENTITY;
        m.0[3][0] = v.x;
        m.0[3][1] = v.y;
        m.0[3][2] = v.z;
        m
    }

    /// D3DXMatrixRotationX.
    pub fn rot_x(a: f32) -> DMat {
        let (s, c) = a.sin_cos();
        let mut m = DMat::IDENTITY;
        m.0[1][1] = c;
        m.0[2][2] = c;
        m.0[1][2] = s;
        m.0[2][1] = -s;
        m
    }

    /// D3DXMatrixRotationY.
    pub fn rot_y(a: f32) -> DMat {
        let (s, c) = a.sin_cos();
        let mut m = DMat::IDENTITY;
        m.0[0][0] = c;
        m.0[2][2] = c;
        m.0[0][2] = -s;
        m.0[2][0] = s;
        m
    }

    /// D3DXMatrixRotationZ.
    pub fn rot_z(a: f32) -> DMat {
        let (s, c) = a.sin_cos();
        let mut m = DMat::IDENTITY;
        m.0[0][0] = c;
        m.0[1][1] = c;
        m.0[0][1] = s;
        m.0[1][0] = -s;
        m
    }

    /// D3DXMatrixRotationAxis (the axis is normalised).
    pub fn rot_axis(axis: Vec3, a: f32) -> DMat {
        let v = axis.normalize_or_zero();
        let (s, c) = a.sin_cos();
        let d = 1.0 - c;
        let mut m = DMat::IDENTITY;
        m.0[0][0] = d * v.x * v.x + c;
        m.0[1][0] = d * v.x * v.y - s * v.z;
        m.0[2][0] = d * v.x * v.z + s * v.y;
        m.0[0][1] = d * v.y * v.x + s * v.z;
        m.0[1][1] = d * v.y * v.y + c;
        m.0[2][1] = d * v.y * v.z - s * v.x;
        m.0[0][2] = d * v.z * v.x - s * v.y;
        m.0[1][2] = d * v.z * v.y + s * v.x;
        m.0[2][2] = d * v.z * v.z + c;
        m
    }

    /// D3DXMatrixMultiply(out, self, b): `self` first, then `b`.
    pub fn mul(&self, b: &DMat) -> DMat {
        let mut o = [[0.0f32; 4]; 4];
        for (i, row) in o.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = (0..4).map(|k| self.0[i][k] * b.0[k][j]).sum();
            }
        }
        DMat(o)
    }

    /// A point through the matrix (D3DXVec3TransformCoord).
    pub fn point(&self, p: Vec3) -> Vec3 {
        let r = |j: usize| p.x * self.0[0][j] + p.y * self.0[1][j] + p.z * self.0[2][j] + self.0[3][j];
        let w = r(3);
        let w = if w.abs() > 1e-12 { w } else { 1.0 };
        Vec3::new(r(0) / w, r(1) / w, r(2) / w)
    }

    /// The same transform in this engine's model frame (x right, y forward, z up), as a
    /// column-vector affine: `S * M^T * S` with S swapping y and z.
    pub fn to_engine(&self) -> Affine3A {
        let m = &self.0;
        // column-vector form: its columns are the D3D matrix's rows
        let row = |j: usize| Vec4::new(m[j][0], m[j][1], m[j][2], m[j][3]);
        let t = Mat4::from_cols(row(0), row(1), row(2), row(3));
        let s = Mat4::from_cols(Vec4::X, Vec4::Z, Vec4::Y, Vec4::W);
        Affine3A::from_mat4(s * t * s)
    }
}

/// A point of this engine's model frame in Direct3D's (y and z swapped).
pub fn d3d(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y)
}

/// The skeleton of a `.hum` as Omsi.exe keeps it (THumanBeing, loader sub_624a90): the
/// right side's joints in Direct3D's frame and the vectors derived from them at load.
#[derive(Debug, Clone)]
pub struct OmsiRig {
    /// +0x2c0, +0x2cc, +0x2b4, +0x278, +0x284, +0x2a8, +0x290, +0x29c.
    pub hip: Vec3,
    pub knee: Vec3,
    pub waist: Vec3,
    pub shoulder: Vec3,
    pub elbow: Vec3,
    pub neck: Vec3,
    pub hand: Vec3,
    pub finger: Vec3,
    /// `[humangeom]`: feet distance (+0x274) and height (+0x270).
    pub feet_dist: f32,
    pub height: f32,
    /// `[seatheight]` (+0x26c, 0 when missing).
    pub seat_height: f32,
    /// `[walk_param]`: stride (+0x2d8, 1.4), upper arm beta (+0x2e8, 66), arm swing
    /// (+0x2dc, 1), hip turn (+0x2e0, 1), waist (+0x2e4, 0).
    pub stride: f32,
    pub beta: f32,
    pub arm_swing: f32,
    pub hip_turn: f32,
    pub waist_bend: f32,
    /// Derived at load: the right upper arm (elbow - shoulder, +0x310) and its mirror
    /// (+0x304), forearm and hand (finger - elbow, +0x2f8; +0x2ec), thigh (knee - hip,
    /// +0x328; +0x31c).
    pub upper_arm: [Vec3; 2],
    pub forearm: [Vec3; 2],
    pub thigh: [Vec3; 2],
}

impl OmsiRig {
    pub fn new(def: &Human) -> OmsiRig {
        let l = |i: usize| def.links.get(i).copied().filter(|v| v.is_finite()).unwrap_or(0.0);
        // [links] lists x, y (forward), z (up); Omsi.exe stores (x, z, y)
        let hip = Vec3::new(l(0), l(2), l(1));
        let knee = Vec3::new(l(3), l(5), l(4));
        let waist = Vec3::new(0.0, l(7), l(6));
        let shoulder = Vec3::new(l(8), l(10), l(9));
        let elbow = Vec3::new(l(11), l(13), l(12));
        let neck = Vec3::new(0.0, l(15), l(14));
        let hand = Vec3::new(l(16), l(18), l(17));
        let finger = Vec3::new(l(19), l(21), l(20));
        let mirror = |v: Vec3| Vec3::new(-v.x, v.y, v.z);
        let ua = elbow - shoulder;
        let fa = finger - elbow;
        let th = knee - hip;
        let wp = def.walk_param;
        OmsiRig {
            hip,
            knee,
            waist,
            shoulder,
            elbow,
            neck,
            hand,
            finger,
            feet_dist: def.feet_dist,
            height: def.height,
            seat_height: def.seat_height,
            stride: wp[0],
            beta: wp[1],
            arm_swing: wp[2],
            hip_turn: wp[3],
            waist_bend: wp[4],
            upper_arm: [mirror(ua), ua],
            forearm: [mirror(fa), fa],
            thigh: [mirror(th), th],
        }
    }
}

/// The walk curves of sub_624820 (piecewise linear, held at the ends): the thigh's and the
/// knee's angle over the phase's fraction, in units of the leg's swing angle.
const THIGH_CURVE: [(f32, f32); 4] = [(0.0, 0.75), (0.2, 1.1), (0.8, -1.1), (1.0, 0.75)];
const KNEE_CURVE: [(f32, f32); 4] = [(0.0, 1.0), (0.2, 0.0), (0.8, 0.0), (1.0, 1.0)];
/// The knee through a running stride: never straight - a little bent under
/// the body while the foot is down, a short stance - and folded well up behind as the leg
/// swings through (the heel kick).
const RUN_KNEE_CURVE: [(f32, f32); 5] = [(0.0, 1.0), (0.22, 0.2), (0.5, 0.15), (0.72, 0.3), (1.0, 1.0)];

/// sub_7f061c: piecewise linear, the first or last value outside the points.
fn curve(c: &[(f32, f32)], x: f32) -> f32 {
    if x < c[0].0 {
        return c[0].1;
    }
    let last = c[c.len() - 1];
    if last.0 <= x {
        return last.1;
    }
    let mut i = 1;
    while i < c.len() && c[i].0 <= x {
        i += 1;
    }
    let (a, b) = (c[i - 1], c[i]);
    if b.0 == a.0 {
        (a.1 + b.1) / 2.0
    } else {
        (x - a.0) * ((b.1 - a.1) / (b.0 - a.0)) + a.1
    }
}

/// Delphi's Trunc of a float (toward zero).
fn frac(x: f32) -> f32 {
    x - x.trunc()
}

const DEG: f32 = std::f32::consts::PI / 180.0;

/// The run (`crate::human::run_factor`), at a full run: how much longer the
/// stride gets, the most the knees bend (degrees, through `RUN_KNEE_CURVE`), the lean
/// forward (degrees), how much more the arms swing from the shoulder, the elbows' bend
/// (degrees) and how much of the walk's dip at every step is left (a runner barely dips:
/// with it doubled they crouched at every step). The leg's swing grows with the stride
/// already; on top of that the knees had folded through the thighs.
const RUN_STRIDE: f32 = 0.4;
const RUN_KNEE_MAX: f32 = 105.0;
const RUN_LEAN: f32 = 6.0;
const RUN_ARM_SWING: f32 = 0.6;
const RUN_ELBOW: f32 = 85.0;
const RUN_BOB: f32 = 0.35;
/// The original's degree-to-radian factor (a 10-byte constant, 0.01745...).
const RAD: f32 = 0.017_453_292;

/// What the animation is told about a person this frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct AnimInput {
    /// `PAX_State` rounded: 0 stand, 1 walk, 2 sit.
    pub kind: u8,
    /// m/s (the sign is ignored).
    pub speed: f32,
    /// Distance moved this frame (m).
    pub moved: f32,
    /// Room height of the path link (m; 50 outside a vehicle).
    pub room_height: f32,
    /// `HeightOfSeat`: the seat's height above the floor (m).
    pub seat_height: f32,
    /// The right hand reaches here (person's frame, Direct3D axes).
    pub reach: Option<Vec3>,
    /// The head looks here (person's frame, Direct3D axes).
    pub look: Option<Vec3>,
    /// The angles ease towards their targets instead of jumping (+0x660: standing at the
    /// validator or the cash desk, sitting).
    pub smooth: bool,
    /// Frame time (ms).
    pub dt_ms: f32,
}

/// A person's animation state: the walk phase and the thirty angles (degrees).
#[derive(Debug, Clone)]
pub struct OmsiAnim {
    /// +0x66c, 0 .. 2.
    pub phase: f32,
    /// +0x530 .. +0x5a4.
    pub angles: [f32; 30],
    /// The pelvis bob of the last frame (the translation of +0x4f0).
    bob: f32,
}

impl Default for OmsiAnim {
    fn default() -> Self {
        OmsiAnim { phase: 0.0, angles: [0.0; 30], bob: 0.0 }
    }
}

/// What [`OmsiAnim::advance`] reports.
#[derive(Debug, Clone, Copy, Default)]
pub struct AnimEvents {
    /// The phase crossed 0.2, 0.7, 1.2 or 1.7: a foot came down (Omsi.exe plays the
    /// link's step sound then).
    pub step: bool,
}

impl OmsiAnim {
    /// The thirty angles for this frame (sub_626ae8 up to 0x628c8a) and the phase.
    pub fn advance(&mut self, rig: &OmsiRig, inp: &AnimInput) -> AnimEvents {
        let mut ev = AnimEvents::default();
        let mut a = [0.0f32; 30];
        // the stride used at this speed (+0x2d8 times |v| / 1.2, at most 1)
        let rel = (inp.speed.abs() / 1.2).min(1.0);
        // (from 2 m/s the walk turns into a run - a longer stride, the knees higher, a lean
        // forward, the elbows bent; nothing changes at a walking pace)
        let run = if inp.kind == 1 { crate::human::run_factor(inp.speed.abs()) } else { 0.0 };
        let stride = rel * rig.stride * (1.0 + RUN_STRIDE * run);
        // stooping under a low ceiling
        let l20 = inp.room_height - rig.waist.y;
        let l24 = rig.height - rig.waist.y;
        let stoop = if l20 < l24 {
            let c = l20.max(0.0) / l24;
            let s = c.clamp(-1.0, 1.0).acos() / DEG;
            a[12] = s / 2.0;
            a[11] = s / 2.0;
            s
        } else {
            0.0
        };
        let kind = inp.kind;
        if kind == 1 {
            a[12] = a[12].max(3.0);
            a[11] = a[11].max(3.0);
            a[9] = a[9].min(-5.0);
        }
        // the leg's swing angle (degrees)
        let swing = (stride / (4.0 * rig.hip.y)) / DEG;
        let mut bob = 0.0;
        match kind {
            0 => {
                let spread = -(rig.feet_dist / (2.0 * rig.hip.y)) / DEG;
                a[2] = spread;
                a[3] = spread;
            }
            1 => {
                let before = self.phase;
                if stride > 0.0 {
                    self.phase += inp.moved / stride;
                }
                // (0x6275e3: a foot down at 0.2, 0.7, 1.2 and 1.7)
                if [0.2f32, 0.7, 1.2, 1.7].iter().any(|&t| t <= self.phase && before < t) {
                    ev.step = true;
                }
                if self.phase > 2.0 {
                    self.phase -= 2.0;
                }
                let p = self.phase;
                let f_half = frac(p + 0.5);
                let f = frac(p);
                // (the walk's knee; running, the knee of a run blended in)
                let knee = |x: f32| {
                    let walk = curve(&KNEE_CURVE, x) * swing * 1.5 * 2.0;
                    if run > 0.0 {
                        walk + (curve(&RUN_KNEE_CURVE, x) * RUN_KNEE_MAX - walk) * run
                    } else {
                        walk
                    }
                };
                a[0] = curve(&THIGH_CURVE, f_half) * swing;
                a[4] = knee(f_half);
                a[1] = curve(&THIGH_CURVE, f) * swing;
                a[5] = knee(f);
                let spread = -(rig.feet_dist / (3.0 * rig.hip.y)) / DEG;
                a[2] = spread;
                a[3] = spread;
                let l24 = ((4.0 * std::f32::consts::PI * p).cos() - 1.0) / 2.0;
                bob = (1.0 - (std::f32::consts::PI / 180.0 * swing).cos()) * (0.8 * rig.hip.y) * l24 * (1.0 - (1.0 - RUN_BOB) * run);
                a[11] -= swing * l24 * rig.waist_bend;
                a[12] += swing * l24 * rig.waist_bend + RUN_LEAN * run;
                a[29] = (2.0 * std::f32::consts::PI * p).sin() * 5.0;
                a[10] = (2.0 * std::f32::consts::PI * p).sin() * rig.hip_turn;
            }
            2 => {
                let thigh = rig.thigh[0].length();
                let l20 = (-(thigh - rig.seat_height) - inp.seat_height) + 0.1;
                let extra = if l20 > 0.0 {
                    let x = (l20 / thigh).min(0.99);
                    let d = x.clamp(-1.0, 1.0).asin() / DEG;
                    if d > 40.0 { 40.0 } else { d }
                } else {
                    0.0
                };
                a[9] = -20.0;
                a[0] = 60.0 + extra;
                a[1] = 60.0 + extra;
                let spread = -(rig.feet_dist / (1.5 * rig.hip.y)) / DEG;
                a[2] = spread;
                a[3] = spread;
                a[4] = 90.0 + extra;
                a[5] = 90.0 + extra;
            }
            _ => {}
        }
        // how much of the arm swing is left when stooping
        let upright = ((90.0 - stoop) / 90.0).max(0.0);
        let sin2 = (2.0 * std::f32::consts::PI * self.phase).sin();
        // the left arm (+0x661 is set for everybody: the left hand holds, the right reaches)
        match kind {
            2 => {
                a[13] = -3.0 - 0.4 * stoop;
                a[15] = 67.0;
                a[17] = -58.0;
                a[19] = 35.0 + stoop;
                a[21] = 46.0;
                a[23] = 41.0;
                a[25] = -26.0;
                a[27] = 5.0;
            }
            1 => {
                a[13] = -3.0;
                a[15] = rig.beta - 3.0;
                a[17] = -30.0;
                a[19] = (sin2 - 0.2) * (upright * rig.arm_swing * swing) * (1.0 + RUN_ARM_SWING * run) + stoop * 0.5;
                let walk_elbow = (sin2 + 1.0) * (upright * rig.arm_swing * swing) * 1.5;
                a[21] = walk_elbow + (RUN_ELBOW + 15.0 * sin2 - walk_elbow) * run;
            }
            _ => {
                a[13] = -3.0;
                a[15] = rig.beta;
                a[17] = -30.0;
                a[19] = 0.6 * stoop - 3.0;
                a[21] = 29.0;
            }
        }
        // the right arm
        if let Some(target) = inp.reach {
            // (sub_626ae8 at 0x627dc6: the law of cosines for the elbow and the shoulder)
            let l30 = rig.upper_arm[1].length();
            let l34 = rig.forearm[1].length();
            let v = target - rig.shoulder;
            let l38 = v.length();
            let c = ((l30 * l30 + l34 * l34) - l38 * l38) / (2.0 * l30 * l34);
            let c = if c > 1.0 { 1.0 } else { c };
            a[22] = if c > -1.0 { 180.0 - c.acos() / DEG } else { 0.0 };
            let c = ((l30 * l30 + l38 * l38) - l34 * l34) / (2.0 * l30 * l38);
            let c = if c > 1.0 { 1.0 } else { c };
            a[20] = if c > -1.0 { -(c.acos() / DEG) } else { 0.0 };
            let arm = rig.forearm[1] + rig.upper_arm[1];
            let flat_arm = Vec3::new(arm.x, 0.0, arm.z);
            let l44 = flat_arm.length();
            let flat_v = Vec3::new(v.x, 0.0, v.z);
            let l48 = flat_v.length();
            // The turn about the shoulder from the arm's own direction to the target's, and the
            // lift from its height to the target's - signed: taken as the law of cosines' bare
            // angle (and 180 less it) with the lift the wrong way round, a hand reaching for
            // the money tray at hip height went up beside the head, the arm stretched out
            // forward and up (a raised-arm salute at every cash desk)
            let _ = (l44, l48);
            a[14] = (flat_v.z.atan2(flat_v.x) - flat_arm.z.atan2(flat_arm.x)) / DEG;
            let l58 = arm.length();
            let l5c = v.length();
            let s1 = (-v.y / l5c).clamp(-1.0, 1.0).asin();
            let s2 = (arm.y / l58).clamp(-1.0, 1.0).asin();
            a[16] = (s1 - s2) / DEG;
        } else {
            match kind {
                2 => {
                    a[14] = -3.0 - 0.4 * stoop;
                    a[16] = 67.0;
                    a[18] = -58.0;
                    a[20] = 35.0 + stoop;
                    a[22] = 46.0;
                    a[24] = 41.0;
                    a[26] = -26.0;
                    a[28] = 5.0;
                }
                1 => {
                    a[14] = -3.0;
                    a[16] = rig.beta - 3.0;
                    a[18] = -30.0;
                    a[20] = (-sin2 - 0.2) * (upright * rig.arm_swing * swing) * (1.0 + RUN_ARM_SWING * run) + stoop * 0.5;
                    let walk_elbow = (-sin2 + 1.0) * (upright * rig.arm_swing * swing) * 1.2;
                    a[22] = walk_elbow + (RUN_ELBOW - 15.0 * sin2 - walk_elbow) * run;
                }
                _ => {
                    a[14] = -3.0;
                    a[16] = rig.beta;
                    a[18] = -30.0;
                    a[20] = 0.6 * stoop - 3.0;
                    a[22] = 29.0;
                }
            }
        }
        // the head
        if let Some(t) = inp.look {
            let v = t - (rig.neck + Vec3::new(0.0, 0.13, 0.0));
            if v.z > 0.0 {
                let l = v.length();
                a[6] = (v.x / l).clamp(-1.0, 1.0).asin() / DEG;
                a[7] = -((v.y / l).clamp(-1.0, 1.0).asin() / DEG);
            }
        }
        // to the angles: at once, or easing towards them (0x628c95)
        let k = (inp.dt_ms / 1000.0 * 10.0).min(1.0);
        let lim = 18000.0 * inp.dt_ms / 1000.0;
        for (cur, to) in self.angles.iter_mut().zip(a.iter()) {
            if inp.smooth {
                let d = ((to - *cur) * k).clamp(-lim, lim);
                *cur += d;
            } else {
                *cur = *to;
            }
        }
        self.bob = bob;
        ev
    }

    /// The thirteen bone matrices (sub_626ae8 from 0x628e78), Direct3D's.
    pub fn bones_d3d(&self, rig: &OmsiRig) -> [DMat; BONES] {
        let a = &self.angles;
        let t = DMat::translation;
        let m = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let (h, k, w, s, e, n, hd) = (rig.hip, rig.knee, rig.waist, rig.shoulder, rig.elbow, rig.neck, rig.hand);
        let mut b = [DMat::IDENTITY; BONES];
        // the pelvis: about the hip line, with the bob (+0x4f0)
        let pelvis = t(-h)
            .mul(&DMat::rot_x(RAD * a[11]))
            .mul(&t(m(0.0, self.bob, 0.0)))
            .mul(&t(h));
        // 8: the hip (lower torso), about the waist
        let waist = t(-w).mul(&DMat::rot_y(RAD * a[10])).mul(&DMat::rot_x(RAD * a[9])).mul(&t(w));
        b[8] = waist.mul(&pelvis);
        // 0, 1: the thighs, from the hip bone with its turn taken out again
        let hl = m(-h.x, h.y, h.z);
        b[0] = t(-hl)
            .mul(&DMat::rot_z(-a[2] * RAD))
            .mul(&DMat::rot_x((-a[11] - a[0]) * RAD))
            .mul(&DMat::rot_y(-a[10] * RAD))
            .mul(&t(hl))
            .mul(&b[8]);
        b[1] = t(-h)
            .mul(&DMat::rot_z(RAD * a[3]))
            .mul(&DMat::rot_x((-a[11] - a[1]) * RAD))
            .mul(&DMat::rot_y(-a[10] * RAD))
            .mul(&t(h))
            .mul(&b[8]);
        // 2, 3: the shins, about the knees
        let kl = m(-k.x, k.y, k.z);
        b[2] = t(-kl).mul(&DMat::rot_x(RAD * a[4])).mul(&t(kl)).mul(&b[0]);
        b[3] = t(-k).mul(&DMat::rot_x(RAD * a[5])).mul(&t(k)).mul(&b[1]);
        // 9: the upper body, about the waist, on the pelvis
        b[9] = t(-w).mul(&DMat::rot_x(RAD * a[12])).mul(&DMat::rot_y(RAD * a[29])).mul(&t(w)).mul(&pelvis);
        // 4, 5: the upper arms, about the shoulders
        let sl = m(-s.x, s.y, s.z);
        b[4] = t(-sl)
            .mul(&DMat::rot_axis(rig.upper_arm[0], RAD * a[17]))
            .mul(&DMat::rot_y(RAD * a[19]))
            .mul(&DMat::rot_z(RAD * a[15]))
            .mul(&DMat::rot_y(RAD * a[13]))
            .mul(&t(sl))
            .mul(&b[9]);
        b[5] = t(-s)
            .mul(&DMat::rot_axis(rig.upper_arm[1], -a[18] * RAD))
            .mul(&DMat::rot_y(-a[20] * RAD))
            .mul(&DMat::rot_z(-a[16] * RAD))
            .mul(&DMat::rot_y(-a[14] * RAD))
            .mul(&t(s))
            .mul(&b[9]);
        // 6, 7: the forearms, about the elbows
        let el = m(-e.x, e.y, e.z);
        b[6] = t(-el).mul(&DMat::rot_y(RAD * a[21])).mul(&t(el)).mul(&b[4]);
        b[7] = t(-e).mul(&DMat::rot_y(-a[22] * RAD)).mul(&t(e)).mul(&b[5]);
        // 11, 12: the hands, about the wrists
        let hl = m(-hd.x, hd.y, hd.z);
        b[11] = t(-hl)
            .mul(&DMat::rot_y(RAD * a[27]))
            .mul(&DMat::rot_z(RAD * a[25]))
            .mul(&DMat::rot_x(-a[23] * RAD))
            .mul(&t(hl))
            .mul(&b[6]);
        b[12] = t(-hd)
            .mul(&DMat::rot_y(-a[28] * RAD))
            .mul(&DMat::rot_z(-a[26] * RAD))
            .mul(&DMat::rot_x(-a[24] * RAD))
            .mul(&t(hd))
            .mul(&b[7]);
        // 10: the head, about the neck, on the upper body
        b[10] = t(-n)
            .mul(&DMat::rot_z(RAD * a[8]))
            .mul(&DMat::rot_x(RAD * a[7]))
            .mul(&DMat::rot_y((a[6] - a[29]) * RAD))
            .mul(&t(n))
            .mul(&b[9]);
        b
    }

    /// The bone matrices in this engine's model frame.
    pub fn bones(&self, rig: &OmsiRig) -> [Affine3A; BONES] {
        self.bones_d3d(rig).map(|m| m.to_engine())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig() -> OmsiRig {
        let def = Human {
            height: 1.77,
            feet_dist: 0.04,
            seat_height: 0.83,
            links: vec![
                0.09, 0.0, 0.92, 0.09, -0.03, 0.53, 0.02, 1.17, 0.18, -0.05, 1.43, 0.44, -0.04, 1.41, -0.02, 1.55, 0.69,
                -0.03, 1.43, 0.9, -0.03, 1.43,
            ],
            walk_param: [1.4, 80.0, 1.0, 1.0, 0.0],
            ..Default::default()
        };
        OmsiRig::new(&def)
    }

    #[test]
    fn curves_as_set_up_in_sub_624820() {
        assert_eq!(curve(&THIGH_CURVE, 0.0), 0.75);
        assert!((curve(&THIGH_CURVE, 0.1) - 0.925).abs() < 1e-6);
        assert!((curve(&THIGH_CURVE, 0.5) - 0.0).abs() < 1e-6);
        assert_eq!(curve(&KNEE_CURVE, 0.5), 0.0);
        assert_eq!(curve(&KNEE_CURVE, 1.0), 1.0);
    }

    #[test]
    fn d3dx_rotations_and_order() {
        // D3DX: RotationY(90°) takes +x to -z (left-handed, row vectors)
        let p = DMat::rot_y(std::f32::consts::FRAC_PI_2).point(Vec3::X);
        assert!((p - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-6, "{p:?}");
        // a translation then a rotation: the rotation acts last
        let m = DMat::translation(Vec3::X).mul(&DMat::rot_z(std::f32::consts::FRAC_PI_2));
        let p = m.point(Vec3::ZERO);
        assert!((p - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-6, "{p:?}");
    }

    #[test]
    fn the_hand_reaches_down_to_the_cash_desk() {
        // targets in front of the right shoulder (D3D: x right, y up, z forward), below it as
        // the money tray and the ticket slot are: the fingers end there, the elbow under the
        // shoulder - not the arm raised up and out
        let r = rig();
        for target in [Vec3::new(0.25, 1.0, 0.4), Vec3::new(0.1, 1.2, 0.45), Vec3::new(0.35, 1.1, 0.2)] {
            let mut an = OmsiAnim::default();
            an.advance(&r, &AnimInput { kind: 0, room_height: 50.0, dt_ms: 16.0, reach: Some(target), ..Default::default() });
            let b = an.bones_d3d(&r);
            let finger = b[7].point(r.finger);
            let elbow = b[5].point(r.elbow);
            assert!((finger - target).length() < 0.05, "{target:?}: the fingers at {finger:?}");
            assert!(elbow.y < r.shoulder.y, "{target:?}: the elbow at {elbow:?} above the shoulder");
        }
    }

    #[test]
    fn standing_pose_keeps_the_feet_on_the_floor() {
        let r = rig();
        let mut an = OmsiAnim::default();
        an.advance(&r, &AnimInput { kind: 0, room_height: 50.0, dt_ms: 16.0, ..Default::default() });
        let b = an.bones(&r);
        // the right ankle region (a point just above the floor under the knee) stays at its
        // height: the leg only turns in by feetdist / (2 x hip height) about the hip
        let foot = Vec3::new(0.09, -0.03, 0.05);
        let p = b[3].transform_point3(foot);
        assert!((p.z - foot.z).abs() < 0.01 && (p.x - foot.x).abs() < 0.03, "{p:?}");
        // the head is where the model has it (no turn)
        let head = Vec3::new(0.0, -0.02, 1.65);
        assert!((b[10].transform_point3(head) - head).length() < 1e-4);
    }

    #[test]
    fn walking_swings_the_legs_and_plays_steps() {
        let r = rig();
        let mut an = OmsiAnim::default();
        let mut steps = 0;
        let mut max_thigh: f32 = 0.0;
        for _ in 0..200 {
            let ev = an.advance(
                &r,
                &AnimInput { kind: 1, speed: 1.2, moved: 1.2 * 0.016, room_height: 50.0, dt_ms: 16.0, ..Default::default() },
            );
            steps += ev.step as u32;
            max_thigh = max_thigh.max(an.angles[0].abs());
        }
        // 200 frames at 1.2 m/s = 3.84 m = 2.74 strides of 1.4 m: a step every half stride
        assert!((4..=7).contains(&steps), "{steps}");
        // the thigh swings by 1.1 x (1.4 / (4 x 0.92)) rad = 24 degrees
        assert!((max_thigh - 24.0).abs() < 1.5, "{max_thigh}");
    }

    /// A run takes longer strides than a walk sped up would, swings the legs
    /// and the knees further, leans forward and bends the elbows; a walk stays as it was.
    #[test]
    fn running_lengthens_the_stride_and_bends_the_elbows() {
        let r = rig();
        let go = |speed: f32| {
            let mut an = OmsiAnim::default();
            let (mut steps, mut thigh, mut knee, mut elbow, mut lean) = (0u32, 0f32, 0f32, 0f32, 0f32);
            for _ in 0..300 {
                let ev = an.advance(&r, &AnimInput { kind: 1, speed, moved: speed * 0.016, room_height: 50.0, dt_ms: 16.0, ..Default::default() });
                steps += ev.step as u32;
                thigh = thigh.max(an.angles[0].abs());
                knee = knee.max(an.angles[4]);
                elbow += an.angles[21] / 300.0;
                lean = lean.max(an.angles[12]);
            }
            (steps as f32 / (speed * 300.0 * 0.016), thigh, knee, elbow, lean)
        };
        let (walk_steps, walk_thigh, walk_knee, walk_elbow, walk_lean) = go(1.2);
        let (run_steps, run_thigh, run_knee, run_elbow, run_lean) = go(3.4);
        assert!(run_steps < walk_steps * 0.75, "fewer steps a metre: {run_steps} vs {walk_steps}");
        assert!(run_thigh > walk_thigh * 1.3 && run_knee > walk_knee * 1.2, "thigh {run_thigh} knee {run_knee}");
        assert!(run_knee <= 105.0, "the knee no more than a right angle and a bit: {run_knee}");
        assert!(run_elbow > 60.0 && walk_elbow < 40.0, "elbows {run_elbow} vs {walk_elbow}");
        assert!(run_lean > walk_lean + 4.0, "lean {run_lean} vs {walk_lean}");
        // under 2 m/s nothing of the run: the thigh as `walking_swings_the_legs_and_plays_steps`
        let (_, thigh_15, ..) = go(1.9);
        assert!((thigh_15 - 24.0).abs() < 1.5, "{thigh_15}");
    }

    #[test]
    fn sitting_bends_hips_and_knees() {
        let r = rig();
        let mut an = OmsiAnim::default();
        an.advance(&r, &AnimInput { kind: 2, room_height: 50.0, seat_height: 0.45, dt_ms: 16.0, ..Default::default() });
        assert!(an.angles[0] >= 60.0 && an.angles[4] >= 90.0);
        assert_eq!(an.angles[9], -20.0);
    }
}
