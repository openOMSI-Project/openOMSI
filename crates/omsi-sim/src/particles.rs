//! Particle systems: `[smoke]` and `[particle_emitter]` of vehicles and scenery objects, as
//! OMSI runs them (TRauch / TRauchInst: the original emits, the original sets a particle
//! off, the original moves it). An emitter keeps at most 100 particles. A particle leaves along
//! the emitter's direction at its speed plus a random spread; every frame its velocity is
//! multiplied by the brake factor raised to 20 times the frame's seconds (Omsi.exe keeps
//! 20 ln(brake) per emitter, 0x5a145c, and takes e to that times dt, 0x5a238c: the factor
//! is per twentieth of a second, whatever the frame rate) and gravity pulls it down (a
//! negative factor makes it rise); it grows from its start size by `size_grow` a second, and
//! its alpha goes linearly from the initial value at birth to the final one at the end of
//! its life (0x5a183c). Its picture is turned by a random angle (0x5a1d54).
//!
//! Nothing stops one at the ground: Omsi.exe lets it fall on through the road until its
//! life is over (0x5a238c moves it and tests nothing but its age). Each particle remembers
//! the height of the ground under the place it was set off from, though - the plane its
//! owner stands on - so that the renderer can let it fade out into the road instead of
//! being cut off by it in a straight line (see `omsi_render::SmokeParticle::ground`).

use glam::{DVec3, Mat4, Vec3};
use omsi_model::{ParticleSystemDef, PsRange, PsValue};
use std::sync::RwLock;

/// Particles an emitter keeps at most (OMSI's 100).
pub const MAX_PER_EMITTER: usize = 100;
/// How many times a second a particle's brake factor slows it (Omsi.exe 0x5a145c keeps
/// 20 ln(brake), of a brake no lower than 0.1).
const BRAKE_RATE: f32 = 20.0;
/// The lowest brake factor Omsi.exe takes (0x5a145c).
const BRAKE_MIN: f32 = 0.1;

/// Where the camera is: emitters farther than their `calc_dist` send no new particles.
static EYE: RwLock<Option<DVec3>> = RwLock::new(None);

pub fn set_eye(p: DVec3) {
    *EYE.write().unwrap_or_else(|e| e.into_inner()) = Some(p);
}

fn eye() -> Option<DVec3> {
    *EYE.read().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone)]
pub struct Particle {
    pub pos: DVec3,
    pub vel: Vec3,
    pub age: f32,
    pub life: f32,
    pub size0: f32,
    pub grow: f32,
    pub alpha0: f32,
    pub alpha1: f32,
    pub color: [f32; 3],
    pub brake: f32,
    pub gravity: f32,
    /// The angle its picture is turned by about the line of sight (radians, 0..2 pi; drawn
    /// so by Omsi.exe, 0x5a1d54 -> 0x5a2b5c).
    pub spin: f32,
    /// World z of the ground under where it was set off (the plane its owner stands on;
    /// minus infinity where it has none).
    pub ground: f64,
}

impl Particle {
    /// Its width (m): the start size plus the growth over its age.
    pub fn size(&self) -> f32 {
        (self.size0 + self.grow * self.age).max(0.0)
    }

    pub fn alpha(&self) -> f32 {
        let t = (self.age / self.life.max(1e-3)).clamp(0.0, 1.0);
        (self.alpha0 + (self.alpha1 - self.alpha0) * t).clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone)]
pub struct Emitter {
    pub def: ParticleSystemDef,
    pub particles: Vec<Particle>,
    /// Particles owed from fractions of frames.
    carry: f32,
    /// The burst of a free emitter has gone off.
    burst_done: bool,
    /// Where and how fast its particles were going when they ended this frame, and the
    /// ground under them (for an emitter attached in burst mode).
    ended: Vec<(DVec3, Vec3, f64)>,
}

/// The weather's wind (m/s, world, x east y north), as the air the particles move in
/// (`set_wind`): smoke drifts with it.
static WIND: std::sync::Mutex<[f32; 3]> = std::sync::Mutex::new([0.0; 3]);

/// The wind the particles drift in from now on (the weather's, every frame).
pub fn set_wind(w: Vec3) {
    *WIND.lock().unwrap_or_else(|e| e.into_inner()) = w.to_array();
}

/// The particle systems of one vehicle part or scenery object.
#[derive(Debug, Clone, Default)]
pub struct ParticleSet {
    pub emitters: Vec<Emitter>,
    rng: u64,
}

fn eval(v: &PsValue, value: &dyn Fn(&str) -> f32) -> f32 {
    match v {
        PsValue::Const(x) => *x,
        PsValue::Var(n) => value(n),
    }
}

impl ParticleSet {
    pub fn new(defs: Vec<ParticleSystemDef>, seed: u64) -> ParticleSet {
        ParticleSet {
            emitters: defs
                .into_iter()
                .map(|def| Emitter { def, particles: Vec::new(), carry: 0.0, burst_done: false, ended: Vec::new() })
                .collect(),
            rng: seed | 1,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.emitters.is_empty()
    }

    /// Live particles of every emitter, and whether each glows (`--PS_emissive--`) and its
    /// picture.
    pub fn particles(&self) -> impl Iterator<Item = (&Particle, &ParticleSystemDef)> {
        self.emitters.iter().flat_map(|e| e.particles.iter().map(move |p| (p, &e.def)))
    }

    /// -1..1
    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        ((self.rng >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    }

    fn draw(&mut self, r: &PsRange, value: &dyn Fn(&str) -> f32) -> f32 {
        let (base, spread) = (eval(&r.0, value), eval(&r.1, value));
        base + spread * self.rand()
    }

    /// One frame: age and move the particles and send new ones off. `origin` and `rot` place
    /// the owner (a vehicle's frame: x right, y forward, z up), `value` reads its variables.
    /// The ground is the owner's own z = 0 (a scenery object stands on it).
    pub fn update(&mut self, dt: f32, origin: DVec3, rot: Mat4, value: &dyn Fn(&str) -> f32) {
        self.update_over(dt, origin, rot, &|| [0.0; 3], value);
    }

    /// `update` for an owner whose ground is the plane z = p[0] + p[1] x + p[2] y of its own
    /// frame (a vehicle's: where its wheels touch the road, see
    /// `VehicleInstance::contact_plane`). `ground` is asked at most once, and only when a
    /// particle is set off from the emitter itself.
    pub fn update_over(&mut self, dt: f32, origin: DVec3, rot: Mat4, ground: &dyn Fn() -> [f32; 3], value: &dyn Fn(&str) -> f32) {
        if self.emitters.is_empty() || dt <= 0.0 {
            return;
        }
        let mut plane: Option<[f32; 3]> = None;
        let eye = eye();
        let wind = Vec3::from(*WIND.lock().unwrap_or_else(|e| e.into_inner()));
        for i in 0..self.emitters.len() {
            // move what is there
            let mut ended = Vec::new();
            {
                let e = &mut self.emitters[i];
                e.particles.retain_mut(|p| {
                    p.age += dt;
                    if p.age >= p.life {
                        ended.push((p.pos, p.vel, p.ground));
                        return false;
                    }
                    // (braked against the air, which moves with the wind: smoke drifts off
                    // with it - it rose straight up in a gale, #1798)
                    p.vel = wind + (p.vel - wind) * p.brake.clamp(BRAKE_MIN, 1.5).powf(dt * BRAKE_RATE);
                    p.vel.z -= 9.81 * p.gravity * dt;
                    p.pos += p.vel.as_dvec3() * dt as f64;
                    true
                });
                e.ended = ended;
            }
            let def = self.emitters[i].def.clone();
            let own = origin + rot.transform_vector3(Vec3::from(def.pos)).as_dvec3();
            if let Some(eye) = eye {
                if (own - eye).length() > def.calc_dist.max(50.0) as f64 {
                    continue;
                }
            }
            let dir = rot.transform_vector3(Vec3::from(def.dir)).normalize_or_zero();
            // where new particles start: the emitter itself, or the particles of the one it
            // is attached to - and the ground under them: the owner's under the emitter, a
            // parent particle's own
            let freq = eval(&def.freq.0, value).max(0.0);
            let mut sources: Vec<(DVec3, Vec3, f64)> = Vec::new();
            let mut burst_sources: Vec<(DVec3, Vec3, f64)> = Vec::new();
            match def.attach {
                Some((parent, mode)) if parent < i => {
                    let pe = &self.emitters[parent];
                    match mode {
                        1 => burst_sources = pe.ended.clone(),
                        _ => sources = pe.particles.iter().map(|p| (p.pos, if mode == 2 { -p.vel } else { dir }, p.ground)).collect(),
                    }
                }
                Some(_) => {}
                None => {
                    let bursts = !self.emitters[i].burst_done && def.burst.is_some();
                    // (asked for only when something will be set off from here)
                    let floor = if freq > 0.0 || bursts {
                        let g = *plane.get_or_insert_with(ground);
                        let under = Vec3::new(def.pos[0], def.pos[1], g[0] + g[1] * def.pos[0] + g[2] * def.pos[1]);
                        origin.z + rot.transform_vector3(under).z as f64
                    } else {
                        f64::NEG_INFINITY
                    };
                    sources.push((own, dir, floor));
                    if bursts {
                        burst_sources.push((own, dir, floor));
                        self.emitters[i].burst_done = true;
                    }
                }
            }
            // continuous emission
            let mut n = 0usize;
            if freq > 0.0 && !sources.is_empty() {
                let e = &mut self.emitters[i];
                e.carry += freq * dt;
                n = e.carry.floor() as usize;
                e.carry -= n as f32;
            }
            let mut spawn: Vec<(DVec3, Vec3, f64)> = Vec::new();
            for _ in 0..n {
                spawn.extend(sources.iter().copied());
            }
            if let Some(b) = &def.burst {
                for s in &burst_sources {
                    let count = self.draw(b, value).round().max(0.0) as usize;
                    spawn.extend(std::iter::repeat(*s).take(count));
                }
            }
            for (at, d, floor) in spawn {
                if self.emitters[i].particles.len() >= MAX_PER_EMITTER {
                    break;
                }
                let mut p = self.new_particle(&def, at, d.normalize_or_zero(), value);
                p.ground = floor;
                self.emitters[i].particles.push(p);
            }
        }
    }

    fn new_particle(&mut self, def: &ParticleSystemDef, at: DVec3, dir: Vec3, value: &dyn Fn(&str) -> f32) -> Particle {
        let speed = eval(&def.velocity.0, value);
        let spread = eval(&def.velocity.1, value);
        let vel = if def.velocity_all_round {
            let v = Vec3::new(self.rand(), self.rand(), self.rand()).normalize_or(Vec3::Z);
            v * (speed + spread * self.rand())
        } else {
            dir * speed + Vec3::new(self.rand(), self.rand(), self.rand()) * spread
        };
        let color = [
            self.draw(&def.rgb[0], value).clamp(0.0, 1.0),
            self.draw(&def.rgb[1], value).clamp(0.0, 1.0),
            self.draw(&def.rgb[2], value).clamp(0.0, 1.0),
        ];
        Particle {
            pos: at,
            vel,
            age: 0.0,
            life: self.draw(&def.life, value).max(0.05),
            size0: self.draw(&def.size_start, value),
            grow: self.draw(&def.size_grow, value),
            alpha0: self.draw(&def.alpha_initial, value),
            alpha1: self.draw(&def.alpha_final, value),
            color,
            brake: self.draw(&def.brake, value),
            gravity: self.draw(&def.gravity, value),
            spin: (self.rand() + 1.0) * std::f32::consts::PI,
            ground: f64::NEG_INFINITY,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smoke(freq: f32) -> ParticleSystemDef {
        let p: Vec<String> = ["0", "-5", "0.4", "0", "-1", "0", "2", "0.2", &freq.to_string(), "2", "0.95", "-0.2", "0.5", "3", "0.8", "0", "0.6", "0.6", "0.6"].iter().map(|s| s.to_string()).collect();
        ParticleSystemDef::from_smoke(&p)
    }

    #[test]
    fn exhaust_puffs_rise_grow_and_fade() {
        let mut s = ParticleSet::new(vec![smoke(20.0)], 7);
        for _ in 0..30 {
            s.update(1.0 / 30.0, DVec3::new(100.0, 200.0, 10.0), Mat4::IDENTITY, &|_| 0.0);
        }
        let ps: Vec<&Particle> = s.particles().map(|(p, _)| p).collect();
        assert!(ps.len() >= 18 && ps.len() <= 21, "{} particles after a second at 20/s", ps.len());
        let oldest = ps.iter().max_by(|a, b| a.age.total_cmp(&b.age)).unwrap();
        assert!(oldest.pos.y < 195.0, "blown out backwards: {:?}", oldest.pos);
        assert!(oldest.size() > 2.5, "grown to {}", oldest.size());
        assert!(oldest.alpha() < 0.5, "faded to {}", oldest.alpha());
        assert!(oldest.vel.length() < 2.0, "slowed to {}", oldest.vel.length());
    }

    /// One puff of `def` set off on the first frame (its frequency read from `f`), then
    /// `frames` more of `dt`.
    fn one_puff(mut def: ParticleSystemDef, dt: f32, frames: usize) -> Particle {
        def.freq.0 = PsValue::Var("f".into());
        let mut s = ParticleSet::new(vec![def], 11);
        s.update(dt, DVec3::ZERO, Mat4::IDENTITY, &|_| 1.01 / dt);
        for _ in 0..frames {
            s.update(dt, DVec3::ZERO, Mat4::IDENTITY, &|_| 0.0);
        }
        let ps: Vec<&Particle> = s.particles().map(|(p, _)| p).collect();
        assert_eq!(ps.len(), 1);
        ps[0].clone()
    }

    /// Omsi.exe brakes a particle by its factor every twentieth of a second (0x5a145c keeps
    /// 20 ln(brake), 0x5a238c takes e to that times dt), whatever the frame rate.
    #[test]
    fn a_brake_factor_is_per_twentieth_of_a_second() {
        let mut def = smoke(0.0);
        def.brake = (PsValue::Const(0.5), PsValue::Const(0.0));
        def.gravity = (PsValue::Const(0.0), PsValue::Const(0.0));
        def.velocity = (PsValue::Const(2.0), PsValue::Const(0.0));
        let at_60 = one_puff(def.clone(), 1.0 / 60.0, 30).vel.length();
        let at_30 = one_puff(def, 1.0 / 30.0, 15).vel.length();
        let want = 2.0 * 0.5f32.powi(10);
        assert!((at_60 - want).abs() < want * 0.01 && (at_30 - want).abs() < want * 0.01, "half a second at 60 fps {at_60}, at 30 fps {at_30}, want {want}");
    }

    /// A puff remembers the height of the ground under where it was set off: the plane its
    /// owner stands on (a vehicle's tilted with the road), or the owner's own z = 0.
    #[test]
    fn a_puff_knows_the_ground_it_was_set_off_over() {
        let asked = std::cell::Cell::new(0);
        let plane = || {
            asked.set(asked.get() + 1);
            [-0.1, 0.0, 0.02]
        };
        let rot = Mat4::from_rotation_z(1.0);
        let mut s = ParticleSet::new(vec![smoke(31.0)], 5);
        s.update_over(1.0 / 30.0, DVec3::new(100.0, 200.0, 10.0), rot, &plane, &|_| 0.0);
        let p = s.particles().next().unwrap().0;
        // the emitter at (0, -5, 0.4): the plane 0.2 m under the origin there
        assert!((p.ground - 9.8).abs() < 1e-4, "ground {}", p.ground);
        assert!(p.pos.z > p.ground, "set off {} over the ground {}", p.pos.z, p.ground);
        assert_eq!(asked.get(), 1);
        // (nothing set off: the plane is not asked for)
        let mut quiet = ParticleSet::new(vec![smoke(0.0)], 5);
        quiet.update_over(1.0 / 30.0, DVec3::ZERO, rot, &plane, &|_| 0.0);
        assert_eq!(asked.get(), 1);
        // a scenery object's: its own z = 0
        let mut o = ParticleSet::new(vec![smoke(31.0)], 5);
        o.update(1.0 / 30.0, DVec3::new(0.0, 0.0, 33.0), Mat4::IDENTITY, &|_| 0.0);
        assert_eq!(o.particles().next().unwrap().0.ground, 33.0);
    }

    /// Every puff's picture is turned its own way (Omsi.exe 0x5a1d54: a random angle).
    #[test]
    fn every_puff_is_turned_its_own_way() {
        let mut s = ParticleSet::new(vec![smoke(60.0)], 9);
        for _ in 0..30 {
            s.update(1.0 / 30.0, DVec3::ZERO, Mat4::IDENTITY, &|_| 0.0);
        }
        let spins: Vec<f32> = s.particles().map(|(p, _)| p.spin).collect();
        assert!(spins.len() > 40);
        assert!(spins.iter().all(|a| (0.0..std::f32::consts::TAU).contains(a)), "{spins:?}");
        let (lo, hi) = spins.iter().fold((f32::MAX, f32::MIN), |(lo, hi), a| (lo.min(*a), hi.max(*a)));
        assert!(lo < 1.0 && hi > 5.0, "spins {lo}..{hi}");
    }

    #[test]
    fn a_variable_frequency_of_zero_sends_nothing_and_the_cap_holds() {
        let mut p = smoke(0.0);
        p.freq.0 = PsValue::Var("auspuff_freq".into());
        let mut s = ParticleSet::new(vec![p], 3);
        for _ in 0..30 {
            s.update(1.0 / 30.0, DVec3::ZERO, Mat4::IDENTITY, &|n| if n == "auspuff_freq" { 0.0 } else { 1.0 });
        }
        assert_eq!(s.particles().count(), 0);
        for _ in 0..60 {
            s.update(1.0 / 30.0, DVec3::ZERO, Mat4::IDENTITY, &|_| 500.0);
        }
        assert!(s.particles().count() <= MAX_PER_EMITTER);
    }
}
