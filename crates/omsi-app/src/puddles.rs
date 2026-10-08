//! Tyre spray on wet roads: what every vehicle's wheels throw up from the water on the road.
//! The puddle itself - the glass-flat, reflective patches on a wet `[moisture]` road, with
//! the raindrop ripples crossing it - is the renderer's (`road_puddle_coverage` in
//! `shader.wgsl`, shared by Enhanced); this file only works out *where* one lies (the same
//! mask, evaluated here so a tyre can be asked whether it stands in one) and makes the
//! spray, drawn as the renderer's smoke particles: soft, lit by the scene, blended.
//!
//! What a tyre does with water is what the spray does here. The tread lifts the water at the
//! back of the contact patch and flings it backwards and up; the heavier part comes down
//! again within a metre or two (a fan behind each wheel, the sheets going a little
//! outwards), the rest breaks up into a fine mist that the vehicle's wake drags along for a
//! moment and then leaves hanging behind it, where it drifts with the air, spreads and
//! slowly settles. All of it grows with the square of the speed - next to nothing at walking
//! pace, a cloud at 50 km/h and more - and with the water there is: standing water throws
//! by far the most, a road that is merely wet through only a thin haze at speed, and a bus
//! or a lorry throws more than a car. A tyre that hits a puddle hard adds a short splash out
//! to the side.
//!
//! Some vehicles spray through their own `[smoke]` systems as well, at their wheels, fed by
//! `tire_wet_freq` / `tire_wet_live`: the stock AI cars (set by their `main_AI` scripts) and
//! the stock SD200, SD202 and NL/NG buses, the AI SD84 among them (set by their `spray.osc`),
//! only once the road is wet through (`StreetCond` 1). Those keep spraying as OMSI has them;
//! such a vehicle gets no haze of its own here on the wet road (that is what its `[smoke]`
//! draws), only what standing water throws, which OMSI knows nothing of, on top.

use glam::{DVec3, Mat4, Vec3};
use omsi_render::SmokeParticle;

/// How far the puddle threshold drops with the wetness (`PUDDLE_SPREAD` in `enhanced.wgsl`
/// and `shader.wgsl`): a road wet through has standing water on about a fifth of it, the
/// rest is wet asphalt.
const PUDDLE_SPREAD: f32 = 0.37;
/// The procedural patterns repeat every `PATTERN_PERIOD` metres (`shader.wgsl`).
const PATTERN_PERIOD: f64 = 1000.0;

/// The water on the road at a point, as the renderer draws it there.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Water {
    /// The puddle mask (0 none .. 1 standing water), the shader's `road_puddle_coverage`.
    pub puddle: f32,
    /// How deep into the puddle the point lies: 0 at its rim .. 1 well inside it.
    pub depth: f32,
    /// How wet the road is there (0 dry or no `[moisture]` road .. 1 wet through).
    pub wet: f32,
}

/// The water at world (x, y): `wet_road` is the moisture-weighted wetness there (global
/// wetness where the surface is a `[moisture]` road, 0 elsewhere - a puddle never lies on
/// bare terrain). The mask is `road_puddle_coverage` of `shader.wgsl` evaluated on the CPU -
/// the same two octaves of the same integer-hashed value noise over the same map-space
/// coordinate - so a tyre throws water exactly where the reflection shows it.
pub fn water_at(x: f64, y: f64, wet_road: f32) -> Water {
    if wet_road <= 0.0 {
        return Water::default();
    }
    // `world_pattern_xy`: the map coordinate, which the shader takes modulo the pattern's
    // period (a whole number of cells of every octave)
    let (px, py) = (x.rem_euclid(PATTERN_PERIOD) as f32, y.rem_euclid(PATTERN_PERIOD) as f32);
    let pn = vnoise_f(px, py, 0.35, 17.3, -9.1) * 0.6 + vnoise_f(px, py, 1.6, -4.0, 8.0) * 0.3 + vnoise_f(px, py, 5.0, 2.7, 11.3) * 0.1;
    let t = 1.0 - wet_road * PUDDLE_SPREAD;
    Water { puddle: smoothstep(t - 0.02, t + 0.02, pn), depth: ((pn - t) / 0.15).clamp(0.0, 1.0), wet: wet_road }
}

/// `hash_cell` of `shader.wgsl`: the lattice cell as an exact integer modulo the lattice's
/// cells per period, through two rounds of a PCG-style mix.
fn hash_cell(cx: f32, cy: f32, cells: f32) -> f32 {
    let wx = cx - cells * (cx / cells).floor();
    let wy = cy - cells * (cy / cells).floor();
    const M: u32 = 1664525;
    let mut x = (wx as u32).wrapping_mul(M).wrapping_add(1013904223);
    let mut y = (wy as u32).wrapping_mul(M).wrapping_add(1013904223);
    x = x.wrapping_add(y.wrapping_mul(M));
    y = y.wrapping_add(x.wrapping_mul(M));
    x ^= x >> 16;
    y ^= y >> 16;
    x = x.wrapping_add(y.wrapping_mul(M));
    x ^= x >> 16;
    (x >> 8) as f32 / 16777215.0
}

/// `vnoise_f` of `shader.wgsl`: `freq` lattice cells a metre, shifted by the offset.
fn vnoise_f(px: f32, py: f32, freq: f32, ox: f32, oy: f32) -> f32 {
    let (qx, qy) = (px * freq + ox, py * freq + oy);
    let cells = (PATTERN_PERIOD as f32 * freq).round();
    let (ix, iy) = (qx.floor(), qy.floor());
    let (fx, fy) = (qx - ix, qy - iy);
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hash_cell(ix, iy, cells);
    let b = hash_cell(ix + 1.0, iy, cells);
    let c = hash_cell(ix, iy + 1.0, cells);
    let d = hash_cell(ix + 1.0, iy + 1.0, cells);
    let ab = a + (b - a) * ux;
    let cd = c + (d - c) * ux;
    ab + (cd - ab) * uy
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One tyre on the road, as the spray needs it.
#[derive(Debug, Clone, Copy)]
pub struct Tyre {
    /// Its stable identity from frame to frame (the vehicle's key and the wheel's index).
    pub key: u64,
    /// Where it touches the road (world).
    pub contact: DVec3,
    /// The way the vehicle is going over the ground (unit, level): forward, or backward when
    /// it reverses.
    pub travel: Vec3,
    /// Out of the vehicle's side the tyre is on (unit, level).
    pub out: Vec3,
    /// Speed over the ground (m/s).
    pub speed: f32,
    /// How much water the vehicle's tyres throw against a car's (1): wider and twin tyres
    /// under a heavy vehicle throw more, up to twice as much.
    pub heavy: f32,
    /// The vehicle sprays on a wet road through its own `[smoke]` (see the module's notes).
    pub own_spray: bool,
}

/// A vehicle's body (or a coupled part's): its `[boundingbox]` placed in the world, so that
/// spray does not show inside it.
#[derive(Debug, Clone, Copy)]
pub struct Body {
    pub origin: DVec3,
    /// Heading (deg, clockwise from north), as `rain::vehicle_boxes` gives it.
    pub heading: f64,
    /// Size x y z and centre x y z in the body's frame (x right, y forward, z up).
    pub bb: [f32; 6],
}

impl Body {
    /// A point in the body's frame, relative to the box's centre.
    fn local(&self, p: DVec3) -> Vec3 {
        let d = p - self.origin;
        let h = self.heading.to_radians();
        let (sh, ch) = (h.sin(), h.cos());
        Vec3::new(
            (d.x * ch - d.y * sh) as f32 - self.bb[3],
            (d.x * sh + d.y * ch) as f32 - self.bb[4],
            d.z as f32 - self.bb[5],
        )
    }

    /// How far a point is outside the box (m; 0 inside it).
    pub fn outside(&self, p: DVec3) -> f32 {
        let l = self.local(p);
        let d = (l.abs() - Vec3::new(self.bb[0], self.bb[1], self.bb[2]) * 0.5).max(Vec3::ZERO);
        d.length()
    }

    /// Whether a point is inside the box, `margin` metres round it included.
    pub fn contains(&self, p: DVec3, margin: f32) -> bool {
        let l = self.local(p).abs();
        l.x < self.bb[0] * 0.5 + margin && l.y < self.bb[1] * 0.5 + margin && l.z < self.bb[2] * 0.5 + margin
    }
}

/// The water's colour for the smoke shading, which lights a puff with the weather's ambient
/// light and a share of the sun's in every graphics mode alike, and how much of what is
/// behind a puff it covers (a factor on its opacity). What that comes to on the screen
/// differs: Vanilla writes it into a picture whose other colours are OMSI's encoded ones, so
/// it came out far too light, and Enhanced's exposure carries it above the sky it is lit by;
/// and the many puffs over one another stack up to a plume two to three times as strong in
/// Vanilla's and Vanilla+'s plain picture as under Enhanced's exposure and tone mapping. The
/// grey and the opacity are chosen so that the mist shows about as light, and as thin, as in
/// Enhanced (in the same overcast rain: the light it adds to the road behind a car is about a
/// third of the road's own in each mode), and the light it is given scales it from there -
/// darker at dusk and at night.
fn spray_look() -> ([f32; 3], f32) {
    use std::sync::atomic::Ordering::Relaxed;
    let (grey, opacity) = if crate::startup::CLASSIC.load(Relaxed) {
        (0.32, 0.26)
    } else if crate::startup::ENHANCED.load(Relaxed) {
        (0.5, 1.0)
    } else {
        (0.65, 0.42)
    };
    ([grey, grey * 1.02, grey * 1.06], opacity)
}

/// Whether a set of particle systems sprays from the tyres itself: a `[smoke]` that the
/// stock vehicles' `tire_wet_freq` / `tire_wet_live` variables drive.
pub fn has_own_tyre_spray(set: &omsi_sim::particles::ParticleSet) -> bool {
    let wet = |v: &omsi_model::PsValue| matches!(v, omsi_model::PsValue::Var(n) if n.to_ascii_lowercase().starts_with("tire_wet"));
    set.emitters.iter().any(|e| wet(&e.def.freq.0) || wet(&e.def.life.0))
}

/// How much more water a vehicle of this mass throws than a car: 1 up to 1.3 t, twice as
/// much from about 5 t (a bus, a lorry: wider tyres, twin ones at the back).
fn heaviness(mass_kg: f32) -> f32 {
    (mass_kg.max(1.0) / 1300.0).sqrt().clamp(1.0, 2.0)
}

/// The level forward and right axes of a body's rotation.
fn level_axes(rot: Mat4) -> (Vec3, Vec3) {
    let f = rot.transform_vector3(Vec3::Y);
    let f = Vec3::new(f.x, f.y, 0.0).normalize_or(Vec3::Y);
    (f, Vec3::new(f.y, -f.x, 0.0))
}

/// A vehicle's tyres on the road this frame, its coupled parts' included. `key` tells the
/// vehicle apart from the others (the tyres' keys are made from it).
pub fn vehicle_tyres(v: &omsi_sim::VehicleInstance, key: u64, out: &mut Vec<Tyre>) {
    let speed = v.physics.speed;
    let heavy = heaviness(v.physics.mass_kg);
    let mut n = 0u64;
    // (every wheel keeps its number, whether it touches the road this frame or not)
    let mut part = |origin: DVec3, rot: Mat4, own_spray: bool, wheels: &mut dyn Iterator<Item = Option<Vec3>>, out: &mut Vec<Tyre>| {
        let (fwd, right) = level_axes(rot);
        let travel = if speed < 0.0 { -fwd } else { fwd };
        for local in wheels {
            n += 1;
            let Some(local) = local else { continue };
            out.push(Tyre {
                key: (key << 8) | n.min(255),
                contact: origin + rot.transform_vector3(local).as_dvec3(),
                travel,
                out: if local.x < 0.0 { -right } else { right },
                speed: speed.abs(),
                heavy,
                own_spray,
            });
        }
    };
    let rot = v.body_rotation();
    let own = has_own_tyre_spray(&v.particles);
    match v.rigid.as_ref() {
        // the player's: the wheels that stand on the road, where they touch it
        Some(rb) => part(
            v.position,
            rot,
            own,
            &mut rb.wheels.iter().map(|w| w.on_ground.then(|| w.attach + Vec3::Z * (w.compression.max(-omsi_sim::rigid::DROOP) - w.radius))),
            out,
        ),
        // an AI vehicle's (a LAN player's copy too): its axles on the plane its body is
        // placed over
        None => {
            let lift = v.ai_rest_offset().0;
            part(v.position, rot, own, &mut v.physics.wheels.iter().flatten().map(|w| Some(Vec3::new(w.lat, w.long, -lift))), out)
        }
    }
    for t in &v.trailers {
        let lift = t.ground_lift();
        let axles = &t.ty.def.axles;
        part(
            t.position,
            t.body_rotation(),
            has_own_tyre_spray(&t.particles),
            &mut axles.iter().flat_map(|a| [-0.5, 0.5].map(|s| Some(Vec3::new(a.max_width * s, a.long, -lift)))),
            out,
        );
    }
}

/// The wetness the renderer draws the puddles with: the roads' own (`OMSI_WETNESS` in its
/// place, as the picture takes it), none under snow (a snowy road has no standing water).
pub fn road_wetness(wetness: f32, snow: bool) -> f32 {
    let w = omsi_cfg::env::var("OMSI_WETNESS").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(wetness);
    if snow {
        0.0
    } else {
        w.clamp(0.0, 1.0)
    }
}

/// The key of a LAN player's vehicle (`| id`) beside the AI's (their ids + 1) and the
/// player's own (0).
pub const REMOTE_KEY: u64 = 1 << 40;
/// The weather's wind at the height of the spray, in the lee of the street and the traffic.
pub const GROUND_WIND: f32 = 0.4;

/// Tyres farther than this from the camera throw nothing (m).
pub const SPAWN_RANGE: f64 = 100.0;
/// Within this distance a tyre throws all its spray; beyond it less and less, down to a
/// quarter at `SPAWN_RANGE` (in somewhat bigger puffs).
const FULL_DETAIL: f64 = 30.0;
/// At most this many puffs alive at once, every vehicle's together.
pub const MAX_PUFFS: usize = 1400;
/// The speed the amounts below are given for (m/s, 50 km/h): the spray goes with the
/// square of the speed.
const REF_SPEED: f32 = 13.9;
/// Mist puffs a second from one car tyre at `REF_SPEED` in a full, deep puddle.
const MIST_RATE: f32 = 65.0;
/// Heavy drops a second (the fan behind the tyre), likewise.
const SHEET_RATE: f32 = 45.0;
/// What a road wet through without standing water gives against a puddle.
const FILM: f32 = 0.12;
/// The mist settles at most this fast (m/s).
const SETTLE: f32 = 0.22;
/// A mist puff's middle stays at least this many of its half widths above the road.
const GROUND_HOLD: f32 = 0.6;
/// Over how many metres round a body a puff goes from what it is under the body to what it
/// is in the open.
const UNDER_BODY_BLEND: f32 = 0.3;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// The fine mist that hangs behind the vehicle.
    Mist,
    /// Heavy drops thrown in a fan behind the tyre (and out to the side when it hits a
    /// puddle hard): they fly in an arc and are gone when they land.
    Sheet,
}

#[derive(Debug, Clone)]
struct Puff {
    kind: Kind,
    pos: DVec3,
    vel: Vec3,
    age: f32,
    life: f32,
    size0: f32,
    size1: f32,
    alpha: f32,
    /// The road's height where it was thrown up.
    ground: f64,
    /// How quickly the air takes it along (1/s).
    drag: f32,
}

impl Puff {
    fn t(&self) -> f32 {
        (self.age / self.life).clamp(0.0, 1.0)
    }

    /// Half its width (m): it spreads quickly at first, then slower.
    fn size(&self) -> f32 {
        let t = self.t();
        self.size0 + (self.size1 - self.size0) * (1.0 - (1.0 - t) * (1.0 - t))
    }

    /// Faded in over its first frames (no puff pops up out of nothing) and out over the
    /// rest of its life, thinning as it spreads; a drop coming down fades as it reaches the
    /// road.
    fn opacity(&self) -> f32 {
        let t = self.t();
        let a = self.alpha * smoothstep(0.0, 0.08, self.age) * (1.0 - t).powf(1.3) * (0.4 + 0.6 * self.size0 / self.size().max(0.01));
        match self.kind {
            Kind::Mist => a,
            Kind::Sheet => a * smoothstep(0.2, 0.7, (self.pos.z - self.ground) as f32 / self.size().max(0.01)),
        }
    }
}

/// What a tyre left behind last frame.
#[derive(Debug, Clone, Copy, Default)]
struct TyreState {
    /// Fractional puffs owed (mist, sheet), so the spray is continuous rather than one pop.
    debt: [f32; 2],
    /// The puddle under it.
    puddle: f32,
    seen: bool,
}

pub struct Spray {
    puffs: Vec<Puff>,
    tyres: hashbrown::HashMap<u64, TyreState>,
    /// The bodies near the camera this frame (see [`Body`]).
    bodies: Vec<Body>,
    rng: u64,
    /// Tyres throwing water this frame, and how many of them in a puddle (`OMSI_DEBUG_RAIN`).
    pub tyres_wet: u32,
    pub tyres_in_puddle: u32,
}

impl Default for Spray {
    fn default() -> Self {
        Spray::new()
    }
}

impl Spray {
    pub fn new() -> Spray {
        Spray { puffs: Vec::new(), tyres: hashbrown::HashMap::new(), bodies: Vec::new(), rng: 0xD00D_F00D_1234_5678, tyres_wet: 0, tyres_in_puddle: 0 }
    }

    /// 0..1
    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Puffs alive now.
    pub fn len(&self) -> usize {
        self.puffs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.puffs.is_empty()
    }

    /// One frame of the vehicles near the camera: their tyres and bodies (see
    /// [`vehicle_tyres`], [`rain::vehicle_boxes`](crate::rain::vehicle_boxes)), then
    /// [`Spray::update`].
    pub fn frame(&mut self, dt: f32, vehicles: &[(u64, &omsi_sim::VehicleInstance)], eye: DVec3, wind: Vec3, water: &dyn Fn(f64, f64) -> Water) {
        let mut tyres = Vec::new();
        let mut bodies = std::mem::take(&mut self.bodies);
        bodies.clear();
        for &(key, v) in vehicles {
            if (v.position - eye).length() > SPAWN_RANGE + 40.0 {
                continue;
            }
            vehicle_tyres(v, key, &mut tyres);
            bodies.extend(crate::rain::vehicle_boxes(v).into_iter().map(|(origin, heading, bb)| Body { origin, heading, bb }));
        }
        self.bodies = bodies;
        self.update(dt, &tyres, eye, wind, water);
    }

    /// One frame: age and move the spray in the air, and throw new spray from `tyres`.
    /// `eye` is the camera (the tyres near it throw the most), `wind` the air's motion at
    /// the ground, `water` the water on the road at a world (x, y) - see [`water_at`].
    pub fn update(&mut self, dt: f32, tyres: &[Tyre], eye: DVec3, wind: Vec3, water: &dyn Fn(f64, f64) -> Water) {
        if dt <= 0.0 {
            return;
        }
        self.age(dt, wind);
        for s in self.tyres.values_mut() {
            s.seen = false;
        }
        self.tyres_wet = 0;
        self.tyres_in_puddle = 0;
        for tyre in tyres {
            self.throw(dt, tyre, eye, water);
        }
        // a tyre that left (a vehicle gone, a wheel off the road) owes nothing
        self.tyres.retain(|_, s| s.seen);
        if omsi_cfg::env::var_os("OMSI_DEBUG_RAIN").is_some() && (self.tyres_wet > 0 || !self.puffs.is_empty()) {
            log::info!("spray: {} of {} tyres throwing water, {} in a puddle, {} puffs", self.tyres_wet, tyres.len(), self.tyres_in_puddle, self.puffs.len());
        }
    }

    fn age(&mut self, dt: f32, wind: Vec3) {
        let mut i = 0;
        while i < self.puffs.len() {
            let p = &mut self.puffs[i];
            p.age += dt;
            let k = (p.drag * dt).min(1.0);
            match p.kind {
                Kind::Mist => {
                    // taken along by the air, and settling slowly
                    p.vel.x += (wind.x - p.vel.x) * k;
                    p.vel.y += (wind.y - p.vel.y) * k;
                    p.vel.z += (-SETTLE - p.vel.z) * k;
                }
                Kind::Sheet => {
                    p.vel.x += (wind.x - p.vel.x) * k;
                    p.vel.y += (wind.y - p.vel.y) * k;
                    p.vel.z -= 9.81 * dt;
                }
            }
            p.pos += p.vel.as_dvec3() * dt as f64;
            // (the picture of a puff is empty in its lowest fifth and faint up to half way:
            // held this high, the road cuts it where it has next to no mist left, not in a
            // hard line through the middle)
            let floor = p.ground + (p.size() * GROUND_HOLD) as f64;
            let landed = match p.kind {
                // a drop that has come down is part of the puddle again
                Kind::Sheet => p.pos.z < p.ground + (p.size() * 0.3) as f64 && p.vel.z < 0.0,
                // the mist lies on the road at the lowest, never in it
                Kind::Mist => {
                    if p.pos.z < floor {
                        p.pos.z = floor;
                        p.vel.z = p.vel.z.max(0.0);
                    }
                    false
                }
            };
            if landed || p.age >= p.life || !p.pos.is_finite() {
                self.puffs.swap_remove(i);
                continue;
            }
            i += 1;
        }
    }

    fn throw(&mut self, dt: f32, tyre: &Tyre, eye: DVec3, water: &dyn Fn(f64, f64) -> Water) {
        let dist = (tyre.contact - eye).length();
        if dist > SPAWN_RANGE || !tyre.contact.is_finite() {
            return;
        }
        let w = water(tyre.contact.x, tyre.contact.y);
        let state = self.tyres.entry(tyre.key).or_default();
        let entered = state.puddle;
        state.puddle = w.puddle;
        state.seen = true;
        let v = tyre.speed;
        // standing water, more of it deeper in; a road wet through only lends a film, which
        // a vehicle with its own `[smoke]` spray already throws
        let puddle = w.puddle * (0.45 + 0.55 * w.depth);
        let film = if tyre.own_spray { 0.0 } else { smoothstep(0.55, 1.0, w.wet) * FILM };
        // with the square of the speed: next to nothing at walking pace; the film needs
        // real speed before a tyre lifts it at all
        let sp = (v / REF_SPEED).powi(2).min(3.0) * smoothstep(1.5, 5.0, v);
        let amount = (puddle * sp + film * sp * smoothstep(6.0, 12.0, v)) * tyre.heavy;
        if amount <= 0.0 {
            state.debt = [0.0; 2];
            return;
        }
        self.tyres_wet += 1;
        if w.puddle > 0.05 {
            self.tyres_in_puddle += 1;
        }
        // fewer, bigger puffs far from the camera
        let lod = if dist < FULL_DETAIL { 1.0 } else { 1.0 - 0.75 * ((dist - FULL_DETAIL) / (SPAWN_RANGE - FULL_DETAIL)) as f32 };
        let grow = 1.0 + (1.0 - lod) * 0.6;
        // the far tyres may only fill half the budget: the near ones always have room
        let limit = ((MAX_PUFFS as f64) * (1.0 - 0.5 * dist / SPAWN_RANGE)) as usize;
        state.debt[0] += MIST_RATE * amount * lod * dt;
        state.debt[1] += SHEET_RATE * puddle * sp * smoothstep(3.0, 6.0, v) * tyre.heavy * lod * dt;
        let (mut mist, mut sheet) = (state.debt[0].floor(), state.debt[1].floor());
        state.debt[0] -= mist;
        state.debt[1] -= sheet;
        // a tyre that hits a puddle hard throws a splash out to the side
        let mut splash = if entered < 0.25 && w.puddle >= 0.5 && v > 7.0 {
            ((3.0 + 6.0 * ((v - 7.0) / 10.0).min(1.0)) * tyre.heavy * lod).round()
        } else {
            0.0
        };
        let back = -tyre.travel;
        let up = 0.85 + 0.15 * tyre.heavy;
        while mist >= 1.0 && self.puffs.len() < limit {
            mist -= 1.0;
            let r: [f32; 6] = std::array::from_fn(|_| self.rand());
            // from the back of the contact patch, just outside the tread
            let pos = tyre.contact + (back * (0.3 + 0.4 * r[0]) + tyre.out * (0.05 + 0.2 * r[1])).as_dvec3() + DVec3::Z * (0.15 + 0.25 * r[2]) as f64;
            // the wake behind the vehicle carries it along for a moment - it lags the vehicle
            // from the start, but stays close behind it - lifts it and draws it in behind
            // the body as much as out to the side
            let vel = tyre.travel * v * (0.3 + 0.4 * r[3]) + tyre.out * v * (-0.08 + 0.14 * r[4]) + Vec3::Z * (0.4 + 0.1 * v) * (0.5 + 0.5 * r[5]) * up;
            let r2: [f32; 4] = std::array::from_fn(|_| self.rand());
            let fast = (v / REF_SPEED).min(1.6);
            self.puffs.push(Puff {
                kind: Kind::Mist,
                pos,
                vel,
                age: 0.0,
                life: 1.0 + 1.0 * r2[0] + 0.05 * v,
                size0: 0.25 + 0.15 * r2[1],
                size1: (0.55 + 0.45 * r2[2]) * (0.75 + 0.25 * fast) * (0.85 + 0.15 * tyre.heavy) * grow,
                // (the smoke picture is faint: a puff at its most covers a sixth of what is
                // behind it, and the spray is the many of them together)
                alpha: (0.7 + 0.3 * r2[3]) * (0.55 + 0.45 * (amount / tyre.heavy).min(1.0)),
                ground: tyre.contact.z,
                drag: 1.3,
            });
        }
        while (sheet >= 1.0 || splash >= 1.0) && self.puffs.len() < limit {
            let side = splash >= 1.0;
            if side {
                splash -= 1.0;
            } else {
                sheet -= 1.0;
            }
            let r: [f32; 8] = std::array::from_fn(|_| self.rand());
            let pos = tyre.contact + (back * (0.2 + 0.25 * r[0]) * (!side as i32 as f32) + tyre.out * (0.05 + 0.1 * r[1])).as_dvec3() + DVec3::Z * 0.05;
            let vel = if side {
                // the bow wave: out and forward, with the vehicle but slower than it
                tyre.travel * v * (0.25 + 0.25 * r[2]) + tyre.out * v * (0.15 + 0.2 * r[3]) + Vec3::Z * v * (0.1 + 0.15 * r[4]) * up
            } else {
                // the fan behind the tyre: back (it hardly keeps up with the vehicle), up,
                // and a little outwards
                tyre.travel * v * (0.05 + 0.2 * r[2]) + tyre.out * v * (0.02 + 0.1 * r[3]) + Vec3::Z * v * (0.12 + 0.16 * r[4]) * up
            };
            self.puffs.push(Puff {
                kind: Kind::Sheet,
                pos,
                vel,
                age: 0.0,
                life: 0.55 + 0.35 * r[5],
                size0: 0.15 + 0.07 * r[6],
                size1: (0.4 + 0.25 * r[7]) * grow,
                alpha: 0.6,
                ground: tyre.contact.z,
                drag: 0.6,
            });
        }
    }

    /// The spray as the renderer's smoke particles, for `scene.smoke`. Nothing shows inside
    /// the body the camera sits in (spray only shows behind and beside the cab, through its
    /// windows and in its mirrors, never in it); none inside another vehicle's body pokes up
    /// above the wheels; and a puff the camera is about to pass through fades away rather
    /// than filling the picture.
    pub fn sprites(&self, eye: DVec3, out: &mut Vec<SmokeParticle>) {
        let cab: Vec<&Body> = self.bodies.iter().filter(|b| b.contains(eye, 0.0)).collect();
        let near: Vec<&Body> = self.bodies.iter().filter(|b| (b.origin - eye).length() < SPAWN_RANGE + 40.0).collect();
        let (color, strength) = spray_look();
        out.reserve(self.puffs.len());
        for p in &self.puffs {
            let mut size = p.size();
            let mut alpha = p.opacity();
            for b in &near {
                // (a box a few metres off cannot hold it: no need to turn the point round)
                let reach = 0.5 * (b.bb[0] + b.bb[1] + b.bb[2]) as f64 + size as f64 + 2.0;
                if (b.origin - p.pos).length_squared() > reach * reach {
                    continue;
                }
                let outside = b.outside(p.pos);
                if cab.iter().any(|c| std::ptr::eq(*c, *b)) {
                    // a billboard faces the eye: one that reaches into the box from beside
                    // it is drawn in the cabin, so it has to be clear of it by its size
                    alpha *= smoothstep(0.6 * size, size + 0.2, outside);
                } else if outside < UNDER_BODY_BLEND {
                    // under the body: kept below the floor, half seen - and so, fading out,
                    // over the 30 cm round it, not in a step where it comes out
                    let under = 1.0 - smoothstep(0.0, UNDER_BODY_BLEND, outside);
                    let low = size.min(((p.pos.z - p.ground) as f32 + 0.25).max(0.2));
                    size += (low - size) * under;
                    alpha *= 1.0 - 0.4 * under;
                }
            }
            // (a puff the eye is in covers the whole picture and costs as much to draw: it
            // goes before the camera reaches it, and one too faint to see is left out)
            let d = (p.pos - eye).length() as f32;
            alpha *= smoothstep(0.5, 1.0, d / (size * 2.0 + 0.5));
            if alpha <= 0.012 {
                continue;
            }
            // (it keeps its own way of lying on the road, see `age`: no fade into the ground)
            out.push(SmokeParticle { position: p.pos, size, color, alpha: alpha * strength, ..Default::default() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn puddle_coverage(x: f64, y: f64, wet_road: f32) -> f32 {
        water_at(x, y, wet_road).puddle
    }

    /// A car tyre at the origin going north (+y) at `speed`, its outside to the east.
    fn tyre(speed: f32) -> Tyre {
        Tyre { key: 1, contact: DVec3::new(0.0, 0.0, 33.0), travel: Vec3::Y, out: Vec3::X, speed, heavy: 1.0, own_spray: false }
    }

    fn puddle(_: f64, _: f64) -> Water {
        Water { puddle: 1.0, depth: 1.0, wet: 1.0 }
    }

    fn wet_road(_: f64, _: f64) -> Water {
        Water { puddle: 0.0, depth: 0.0, wet: 1.0 }
    }

    /// Puffs thrown by `tyres` over `secs` (the tyre standing where it is, the camera beside it).
    fn run(tyres: &[Tyre], secs: f32, water: &dyn Fn(f64, f64) -> Water) -> Spray {
        let mut s = Spray::new();
        for _ in 0..(secs * 30.0) as usize {
            s.update(1.0 / 30.0, tyres, DVec3::new(5.0, 0.0, 34.0), Vec3::ZERO, water);
        }
        s
    }

    #[test]
    fn coverage_is_zero_off_the_moisture_mask_and_grows_with_wetness() {
        assert_eq!(puddle_coverage(1000.0, 2000.0, 0.0), 0.0, "no [moisture] under the wheel: never a puddle");
        // scan a stretch of road for a spot the mask calls a puddle at full wetness, and
        // check it grows monotonically as the road dries back out from there
        let mut best = None;
        for i in 0..400 {
            let x = i as f64 * 1.3;
            let c = puddle_coverage(x, 500.0, 1.0);
            if c > 0.4 {
                best = Some((x, c));
                break;
            }
        }
        let (x, full) = best.expect("some spot along 520 m of road should read as a puddle at full wetness");
        let half = puddle_coverage(x, 500.0, 0.5);
        let dry = puddle_coverage(x, 500.0, 0.05);
        assert!(full >= half && half >= dry && full > dry, "a puddle should shrink as the road dries: {full} (wet) vs {half} (half) vs {dry} (drier)");
    }

    #[test]
    fn a_road_wet_through_has_puddles_in_patches() {
        // water stands in the low spots; the road between them is wet asphalt, not a mirror
        let covered = |wet: f32| (0..4000).map(|i| puddle_coverage(i as f64 * 0.77, i as f64 * 1.9, wet)).sum::<f32>() / 4000.0;
        let (soaked, half) = (covered(1.0), covered(0.5));
        assert!((0.15..0.5).contains(&soaked), "puddles cover {soaked} of a soaked road");
        assert!(half < soaked * 0.5, "puddles cover {half} of a half wet road, {soaked} of a soaked one");
    }

    #[test]
    fn the_mask_repeats_with_the_shaders_pattern_period() {
        // the shader sees the map coordinate modulo 1000 m: Spandau's 892 km and the same
        // spot a period on must read the same water
        for (x, y) in [(891_700.3, 4_193_450.8), (12.5, 640.0), (-37.0, 999.9)] {
            let a = water_at(x, y, 1.0);
            let b = water_at(x + 1000.0, y - 3000.0, 1.0);
            assert!((a.puddle - b.puddle).abs() < 1e-3 && (a.depth - b.depth).abs() < 1e-3, "{x}, {y}: {a:?} vs {b:?}");
        }
        // and the hash is the shader's integer one: cells a period apart are the same cell
        assert_eq!(hash_cell(3.0, 5.0, 220.0), hash_cell(223.0, -215.0, 220.0));
        assert!((0.0..=1.0).contains(&hash_cell(17.0, 4.0, 900.0)));
    }

    #[test]
    fn spray_grows_with_the_square_of_the_speed() {
        let walk = run(&[tyre(1.4)], 1.0, &puddle).len();
        let slow = run(&[tyre(7.0)], 1.0, &puddle).len();
        let city = run(&[tyre(13.9)], 1.0, &puddle).len();
        assert!(walk == 0, "a tyre at walking pace throws nothing: {walk}");
        assert!(city > 40, "a tyre at 50 km/h in a puddle throws a cloud: {city} puffs");
        // twice the speed, about four times the spray (the fan's drops land meanwhile)
        assert!(city as f32 > 2.8 * slow as f32, "50 km/h: {city} puffs, 25 km/h: {slow}");
    }

    /// A tyre rolling on along its `travel` for `secs`, the camera beside it.
    fn run_moving(mut t: Tyre, secs: f32) -> (Spray, Tyre) {
        let mut s = Spray::new();
        let dt = 1.0 / 30.0;
        for _ in 0..(secs * 30.0) as usize {
            t.contact += (t.travel * t.speed * dt).as_dvec3();
            s.update(dt, &[t], t.contact + DVec3::new(5.0, 0.0, 1.0), Vec3::ZERO, &puddle);
        }
        (s, t)
    }

    #[test]
    fn the_spray_goes_backwards_from_the_tyre() {
        for travel in [Vec3::Y, -Vec3::Y, Vec3::new(0.6, 0.8, 0.0)] {
            let mut t = tyre(13.9);
            t.travel = travel;
            t.out = Vec3::new(travel.y, -travel.x, 0.0);
            let (s, t) = run_moving(t, 0.6);
            assert!(s.len() > 30);
            let n = s.len() as f32;
            // against the vehicle, which goes on at 13.9 m/s
            let rel = s.puffs.iter().map(|p| p.vel - travel * 13.9).sum::<Vec3>() / n;
            let (along, side) = (rel.dot(travel), rel.dot(t.out));
            assert!(along < -8.0, "going {travel}, the spray falls back from the vehicle: mean relative velocity {rel:?}");
            assert!(along.abs() > 4.0 * side.abs(), "back, not out to the side: {along} along, {side} sideways");
            // and all of it is behind the tyre that threw it, a fan just outside its track
            for p in &s.puffs {
                let d = (p.pos - t.contact).as_vec3();
                assert!(d.dot(travel) < 0.0, "a puff {:.2} m ahead of the tyre", d.dot(travel));
                assert!(d.dot(t.out) > -1.0, "a puff {:.2} m inside the track", -d.dot(t.out));
            }
            let mean_out = s.puffs.iter().map(|p| (p.pos - t.contact).as_vec3().dot(t.out)).sum::<f32>() / n;
            assert!(mean_out > 0.0, "it spreads outwards rather than in: {mean_out}");
        }
    }

    #[test]
    fn a_wet_road_gives_a_thin_haze_and_a_puddle_a_cloud() {
        let haze = run(&[tyre(16.0)], 2.0, &wet_road).len();
        let cloud = run(&[tyre(16.0)], 2.0, &puddle).len();
        assert!(haze > 0, "a road wet through hazes at 58 km/h");
        assert!(cloud > 5 * haze, "a puddle throws far more: {cloud} puffs vs {haze}");
        // only at speed on a mere film, and not at all on a damp road
        assert_eq!(run(&[tyre(5.0)], 2.0, &wet_road).len(), 0, "a film at 18 km/h throws nothing");
        let damp = |_: f64, _: f64| Water { puddle: 0.0, depth: 0.0, wet: 0.4 };
        assert_eq!(run(&[tyre(16.0)], 2.0, &damp).len(), 0, "a damp road throws nothing");
        // a vehicle spraying through its own [smoke] keeps only what the puddles add
        let mut own = tyre(16.0);
        own.own_spray = true;
        assert_eq!(run(&[own], 2.0, &wet_road).len(), 0, "no second haze over its own [smoke] spray");
        assert!(run(&[own], 2.0, &puddle).len() as f32 > 0.8 * cloud as f32, "but the puddles' spray all the same");
    }

    #[test]
    fn a_vehicle_facing_east_throws_to_the_west() {
        // a body turned as `VehicleInstance::body_rotation` turns one with heading 90°
        let (fwd, right) = level_axes(Mat4::from_rotation_z((-90f32).to_radians()));
        assert!((fwd - Vec3::X).length() < 1e-5 && (right + Vec3::Y).length() < 1e-5, "{fwd} {right}");
        // nose up on a hill, the travel stays level
        let (fwd, _) = level_axes(Mat4::from_rotation_x(0.2));
        assert!((fwd - Vec3::Y).length() < 1e-5, "{fwd}");
    }

    #[test]
    fn a_bus_throws_more_than_a_car() {
        let mut bus = tyre(13.9);
        bus.heavy = heaviness(11_000.0);
        let car = run(&[tyre(13.9)], 1.0, &puddle).len();
        let bus = run(&[bus], 1.0, &puddle).len();
        assert!(bus as f32 > 1.6 * car as f32, "bus tyre {bus} puffs, car tyre {car}");
        assert_eq!(heaviness(1100.0), 1.0);
    }

    #[test]
    fn far_tyres_throw_less_and_the_farthest_none() {
        let at = |d: f64| {
            let mut t = tyre(13.9);
            t.contact.x = d;
            let mut s = Spray::new();
            for _ in 0..30 {
                s.update(1.0 / 30.0, &[t], DVec3::new(0.0, 0.0, 34.0), Vec3::ZERO, &puddle);
            }
            s.len()
        };
        let (near, mid, far) = (at(5.0), at(80.0), at(130.0));
        assert_eq!(far, 0, "nothing beyond {SPAWN_RANGE} m");
        assert!(mid > 0 && (mid as f32) < 0.6 * near as f32, "at 80 m {mid} puffs, at 5 m {near}");
    }

    #[test]
    fn the_puffs_stay_under_their_cap() {
        // a whole jam of lorries at motorway speed through deep water
        let tyres: Vec<Tyre> = (0..120)
            .map(|i| {
                let mut t = tyre(25.0);
                t.key = i;
                t.heavy = 2.0;
                t.contact.x = (i % 12) as f64 * 3.0;
                t.contact.y = (i / 12) as f64 * 6.0;
                t
            })
            .collect();
        let s = run(&tyres, 4.0, &puddle);
        assert!(s.len() <= MAX_PUFFS, "{} puffs", s.len());
        assert!(s.len() > MAX_PUFFS / 2, "the cap is used: {} puffs", s.len());
    }

    #[test]
    fn the_mist_lies_on_the_road_not_in_it() {
        let s = run(&[tyre(20.0)], 3.0, &puddle);
        let mut sprites = Vec::new();
        s.sprites(DVec3::new(30.0, 0.0, 35.0), &mut sprites);
        assert!(!sprites.is_empty());
        for p in &s.puffs {
            assert!(p.pos.z >= 33.0 - 1e-6, "a puff under the road at {:?}", p.pos);
            if p.kind == Kind::Mist {
                assert!(p.pos.z - 33.0 >= (GROUND_HOLD * p.size()) as f64 - 1e-4, "a mist puff sunk into the road: {:?} size {}", p.pos, p.size());
                assert!(p.pos.z - 33.0 < 3.5, "a mist puff floating away at {:?}", p.pos);
            }
        }
        for sp in &sprites {
            assert!(sp.alpha > 0.0 && sp.alpha <= 1.0 && sp.size < 1.6, "{sp:?}");
        }
    }

    #[test]
    fn no_spray_in_the_cab_the_camera_sits_in() {
        let mut s = Spray::new();
        // a bus 12 m long facing north round the origin, its rear wheels 3 m from its tail
        s.bodies = vec![Body { origin: DVec3::new(0.0, 0.0, 33.0), heading: 0.0, bb: [2.5, 12.0, 3.2, 0.0, 0.0, 1.6] }];
        let mut t = tyre(13.9);
        t.contact = DVec3::new(1.1, -3.0, 33.0);
        for _ in 0..45 {
            s.update(1.0 / 30.0, &[t], DVec3::new(0.0, 4.0, 35.0), Vec3::ZERO, &puddle);
        }
        let mut cab = Vec::new();
        s.sprites(DVec3::new(-0.5, 4.5, 35.0), &mut cab);
        for p in &cab {
            let outside = s.bodies[0].outside(p.position);
            assert!(outside > 0.6 * p.size, "a puff {outside:.2} m outside the cab, {:.2} m wide, seen from the driver's seat", p.size * 2.0);
        }
        // the same spray from outside: what trails behind the bus is seen
        let mut street = Vec::new();
        s.sprites(DVec3::new(6.0, -20.0, 34.5), &mut street);
        assert!(street.len() > cab.len() && !cab.is_empty(), "{} puffs from the street, {} from the cab", street.len(), cab.len());
    }

    #[test]
    fn a_tyre_that_leaves_the_puddle_stops_throwing() {
        let mut s = Spray::new();
        let t = [tyre(13.9)];
        for _ in 0..10 {
            s.update(1.0 / 30.0, &t, DVec3::new(5.0, 0.0, 34.0), Vec3::ZERO, &puddle);
        }
        assert_eq!(s.tyres_in_puddle, 1);
        let n = s.len();
        s.update(1.0 / 30.0, &t, DVec3::new(5.0, 0.0, 34.0), Vec3::ZERO, &|_, _| Water::default());
        assert_eq!(s.tyres_wet, 0);
        assert!(s.len() <= n, "nothing new off the water");
        // and a vehicle gone from the list leaves no debt behind
        s.update(1.0 / 30.0, &[], DVec3::new(5.0, 0.0, 34.0), Vec3::ZERO, &puddle);
        assert!(s.tyres.is_empty());
    }
}
