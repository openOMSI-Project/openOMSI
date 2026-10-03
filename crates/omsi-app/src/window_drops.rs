//! A bounded population of collector drops. Fine spray stays procedural; these
//! drops retain water and momentum, merge on contact and leave the pane at its edges.

use glam::Vec2;

const LIMIT: usize = 128;
const MAP: usize = 512;
const TRAIL: usize = 8;

pub(super) struct Drop {
    pub pos: Vec2,
    pub velocity: Vec2,
    // Volume in cubic millimetres, with the common cap-shape factor omitted.
    pub water: f32,
    radius: f32,
    tail: [Vec2; TRAIL],
}

impl Drop {
    fn new(pos: Vec2, water: f32) -> Self {
        Self {
            pos,
            water,
            radius: water.cbrt() * 0.001,
            velocity: Vec2::ZERO,
            tail: [pos; TRAIL],
        }
    }

    fn radius(&self) -> f32 {
        self.radius
    }

    pub fn displace(&mut self, pos: Vec2, velocity: Vec2) {
        self.pos = pos;
        self.velocity = velocity;
        // A blade removes the old trail too; it must not reappear behind the blade.
        self.tail.fill(pos);
    }
}

pub(super) struct Drops {
    pub drops: Vec<Drop>,
    size: Vec2,
    grid: [usize; 2],
    heads: Vec<i32>,
    next: [i32; LIMIT],
    seed: u32,
    arrivals: f32,
    dirty: Vec<usize>,
    pub image: omsi_texture::Image,
}

impl Drops {
    pub fn new(size: Vec2, wet: f32, seed: u32) -> Self {
        let grid = [
            (size.x / 0.02).ceil() as usize,
            (size.y / 0.02).ceil() as usize,
        ];
        let mut image = omsi_texture::Image {
            width: MAP as u32,
            height: MAP as u32,
            rgba: vec![0; MAP * MAP * 4],
            has_alpha: true,
        };
        for pixel in image.rgba.chunks_exact_mut(4) {
            pixel[..2].fill(128);
        }
        let mut this = Self {
            drops: Vec::with_capacity(LIMIT),
            size,
            grid,
            heads: vec![-1; grid[0] * grid[1]],
            next: [-1; LIMIT],
            seed,
            arrivals: 0.0,
            dirty: Vec::with_capacity(MAP * MAP / 8),
            image,
        };
        // Match an already wet script state without making all the drops the same age.
        for _ in 0..((wet * size.element_product() * 60.0) as usize).min(LIMIT) {
            let pos = Vec2::new(this.random(), this.random()) * size;
            let water = 0.4 + this.random().powi(3) * 14.0;
            this.drops.push(Drop::new(pos, water));
        }
        this
    }

    fn random(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.seed >> 8) as f32 / 16777216.0
    }

    fn cell(&self, pos: Vec2) -> [usize; 2] {
        let p = pos / self.size;
        [
            (p.x * self.grid[0] as f32) as usize,
            (p.y * self.grid[1] as f32) as usize,
        ]
    }

    /// Surface resistance decreases with size; drag grows with relative airspeed
    /// squared. The coefficients approximate ordinary glass, not a CFD solver.
    pub fn advance(
        &mut self,
        dt: f32,
        rain: f32,
        gravity: Vec2,
        air: Vec2,
        spread: f32,
        wetness: impl Fn(Vec2) -> f32,
    ) {
        let steps = (dt / (1.0 / 60.0)).ceil().max(1.0) as usize;
        let step = dt / steps as f32;
        for _ in 0..steps {
            self.arrivals += step * rain * self.size.element_product() * 24.0;
            while self.arrivals >= 1.0 {
                self.arrivals -= 1.0;
                let pos = Vec2::new(self.random(), self.random()) * self.size;
                // Freshly wiped glass needs time to collect visible drops again.
                if self.drops.len() < LIMIT && wetness(pos) > 0.06 {
                    let water = 0.12 + self.random() * 0.5;
                    self.drops.push(Drop::new(pos, water));
                }
            }
            for drop in &mut self.drops {
                let wet = wetness(drop.pos).clamp(0.0, 1.0);
                let r = (drop.radius() * 1000.0).max(0.4);
                let outward = (drop.pos.x / self.size.x * 2.0 - 1.0) * spread;
                let force = gravity + (air + Vec2::new(outward, 0.0)) / r;
                let strength = force.length();
                let pinning = 4.0 / (r * r);
                let drive = force.normalize_or_zero() * (strength - pinning).max(0.0) * 0.4;
                drop.velocity = (drop.velocity + drive * step) / (1.0 + step * 4.0);
                let previous = drop.pos;
                drop.pos += drop.velocity * step;
                let distance = previous.distance(drop.pos);
                // Rain feeds pinned drops; a runner also collects the fine water in its path.
                drop.water += step * rain * 0.65 + distance * drop.radius() * wet * 12000.0;
                if rain == 0.0 {
                    drop.water = (drop.water - step * 0.015).max(0.0);
                }
                drop.radius = drop.water.cbrt() * 0.001;
                if drop.pos.distance_squared(drop.tail[0]) > 0.015 * 0.015 {
                    drop.tail.copy_within(0..TRAIL - 1, 1);
                    drop.tail[0] = previous;
                }
            }
            self.drops.retain(|d| {
                d.water > 0.01
                    && d.pos.cmpge(Vec2::ZERO).all()
                    && d.pos.cmplt(self.size).all()
                    && wetness(d.pos).is_finite()
            });
            self.merge();
        }
    }

    fn merge(&mut self) {
        self.heads.fill(-1);
        for i in 0..self.drops.len() {
            let [cx, cy] = self.cell(self.drops[i].pos);
            // The small spatial grid avoids comparing every pair of drops.
            let reach = (self.drops[i].radius() / 0.01).ceil().max(1.0) as usize;
            for y in cy.saturating_sub(reach)..=(cy + reach).min(self.grid[1] - 1) {
                for x in cx.saturating_sub(reach)..=(cx + reach).min(self.grid[0] - 1) {
                    let mut j = self.heads[y * self.grid[0] + x];
                    while j >= 0 {
                        let k = j as usize;
                        let (a, b) = self.drops.split_at_mut(i);
                        let (other, drop) = (&mut a[k], &mut b[0]);
                        let contact = drop.radius() + other.radius();
                        if other.water > 0.0
                            && drop.pos.distance_squared(other.pos) < contact * contact
                        {
                            let sum = drop.water + other.water;
                            drop.pos = (drop.pos * drop.water + other.pos * other.water) / sum;
                            drop.velocity =
                                (drop.velocity * drop.water + other.velocity * other.water) / sum;
                            drop.water = sum;
                            drop.radius = sum.cbrt() * 0.001;
                            other.water = 0.0;
                        }
                        j = self.next[k];
                    }
                }
            }
            let [x, y] = self.cell(self.drops[i].pos);
            let head = &mut self.heads[y * self.grid[0] + x];
            self.next[i] = *head;
            *head = i as i32;
        }
        self.drops.retain(|d| d.water > 0.0);
    }

    /// RG is the cap slope, B a cleared runner track, A water coverage. Reuses the
    /// material's bump binding. Clear only touched pixels, not the entire map.
    pub fn paint(&mut self) -> bool {
        let changed = !self.dirty.is_empty() || !self.drops.is_empty();
        for i in self.dirty.drain(..) {
            self.image.rgba[i * 4..i * 4 + 4].copy_from_slice(&[128, 128, 0, 0]);
        }
        for i in 0..self.drops.len() {
            let drop = &self.drops[i];
            let pos = drop.pos;
            let radius = drop.radius().min(0.0035);
            let tail = drop.tail;
            // A short, narrowing filament is distinct from the rounded moving head.
            if drop.velocity.length_squared() > 0.0001 {
                let mut start = pos;
                for (k, &end) in tail.iter().enumerate() {
                    let fade = 1.0 - k as f32 / TRAIL as f32;
                    self.stamp(start, end, radius * 0.25 * fade, true);
                    start = end;
                }
            }
            self.stamp(pos, pos, radius, false);
        }
        changed
    }

    fn stamp(&mut self, a: Vec2, b: Vec2, radius: f32, trail: bool) {
        let scale = Vec2::splat(MAP as f32) / self.size;
        let aa = (self.size / MAP as f32).max_element() * 0.6;
        let lo = ((a.min(b) - Vec2::splat(radius + aa)) * scale)
            .floor()
            .max(Vec2::ZERO);
        let hi = ((a.max(b) + Vec2::splat(radius + aa)) * scale)
            .ceil()
            .min(Vec2::splat((MAP - 1) as f32));
        let edge = b - a;
        for y in lo.y as usize..=hi.y as usize {
            for x in lo.x as usize..=hi.x as usize {
                let pos = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) / scale;
                let t = ((pos - a).dot(edge) / edge.length_squared().max(1e-10)).clamp(0.0, 1.0);
                let offset = pos - a.lerp(b, t);
                let coverage = ((radius + aa - offset.length()) / (2.0 * aa)).clamp(0.0, 1.0)
                    * (radius / aa).min(1.0);
                let alpha = (coverage * 255.0).round() as u8;
                if alpha == 0 {
                    continue;
                }
                let i = y * MAP + x;
                let pixel = &mut self.image.rgba[i * 4..i * 4 + 4];
                if pixel[2] == 0 && pixel[3] == 0 {
                    self.dirty.push(i);
                }
                if trail {
                    pixel[2] = pixel[2].max(alpha);
                }
                if pixel[3] < alpha {
                    let slope = (offset / radius.max(1e-5)).clamp_length_max(1.0) * 0.5;
                    pixel[0] = (128.0 + slope.x * 127.0).round() as u8;
                    pixel[1] = (128.0 + slope.y * 127.0).round() as u8;
                    pixel[3] = alpha;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_update_keeps_all_elapsed_motion_and_empty_glass_needs_no_upload() {
        let mut whole = Drops::new(Vec2::ONE, 0.0, 1);
        let mut split = Drops::new(Vec2::ONE, 0.0, 1);
        assert!(!whole.paint());
        for pop in [&mut whole, &mut split] {
            pop.drops.push(Drop::new(Vec2::splat(0.5), 27.0));
        }
        whole.advance(1.0, 0.0, -Vec2::Y, Vec2::ZERO, 0.0, |_| 1.0);
        for _ in 0..60 {
            split.advance(1.0 / 60.0, 0.0, -Vec2::Y, Vec2::ZERO, 0.0, |_| 1.0);
        }
        assert!(whole.drops[0].pos.distance(split.drops[0].pos) < 1e-5);
        assert!(whole.drops[0].pos.y < 0.48);
        assert!(whole.paint());
        whole.drops.clear();
        assert!(whole.paint());
        assert!(!whole.paint());
    }

    #[test]
    fn coalescence_conserves_volume_and_momentum() {
        let mut drops = Drops::new(Vec2::ONE, 0.0, 1);
        let mut a = Drop::new(Vec2::splat(0.5), 8.0);
        a.velocity = Vec2::X;
        drops.drops.push(a);
        drops.drops.push(Drop::new(Vec2::new(0.501, 0.5), 1.0));
        drops.merge();
        assert_eq!(drops.drops.len(), 1);
        assert_eq!(drops.drops[0].water, 9.0);
        assert!((drops.drops[0].velocity.x - 8.0 / 9.0).abs() < 1e-6);
    }

    #[test]
    fn small_drops_pin_large_drops_fall_and_air_can_reverse_them() {
        let mut drops = Drops::new(Vec2::ONE, 0.0, 1);
        drops.drops.push(Drop::new(Vec2::new(0.25, 0.5), 0.5));
        drops.drops.push(Drop::new(Vec2::new(0.75, 0.5), 27.0));
        drops.advance(0.2, 0.0, -Vec2::Y, Vec2::ZERO, 0.0, |_| 1.0);
        assert_eq!(drops.drops[0].pos.y, 0.5);
        assert!(drops.drops[1].pos.y < 0.5);
        drops.drops[1].velocity = Vec2::ZERO;
        drops.advance(0.2, 0.0, -Vec2::Y, Vec2::Y * 9.0, 0.0, |_| 1.0);
        assert!(drops.drops[1].velocity.y > 0.0);
    }

    #[test]
    fn rain_keeps_arriving_and_large_water_leaves_the_pane() {
        let mut drops = Drops::new(Vec2::ONE, 1.0, 123);
        let original = drops.drops[0].pos;
        for _ in 0..600 {
            drops.advance(0.05, 1.0, -Vec2::Y, Vec2::ZERO, 0.0, |_| 1.0);
        }
        assert!(drops.drops.len() <= LIMIT);
        assert!(drops
            .drops
            .iter()
            .all(|d| d.pos.is_finite() && d.water.is_finite()));
        assert!(!drops.drops.iter().any(|d| d.pos == original));
        assert!(drops.drops.iter().any(|d| d.velocity.y < -0.02));
        assert!(drops.arrivals < 1.0);
        drops.paint();
        assert!(drops.image.rgba.chunks_exact(4).any(|p| p[3] > 0));
    }
}
