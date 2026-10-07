//! What the studio knows of the bus's shape: its triangles in the bus's frame, which of them
//! carry a paint texture, where each texel of that texture lies on the bus (a bake of the
//! triangles into texture space: a position and a normal per texel), which texels are on the
//! outside (seen from somewhere round the bus, not through a window), the colour zones of the
//! base and the window line. All on the CPU, in parallel.

use super::colour;
use super::model::BusDims;
use glam::{Mat4, Vec2, Vec3};
use rayon::prelude::*;

/// One triangle of the bus, in the bus's frame.
#[derive(Clone, Copy, Debug)]
pub struct Tri {
    pub p: [Vec3; 3],
    pub vn: [Vec3; 3],
    pub uv: [Vec2; 3],
    /// Its face normal (unit).
    pub n: Vec3,
    /// Which painted texture it shows (an index into the targets), if any.
    pub target: Option<u8>,
    /// A window (paint goes onto it only from the layers that go over the windows).
    pub glass: bool,
    /// Never painted, nor what shares its texels: a wheel, a lamp, a rubber, a mirror's glass.
    pub keep: bool,
}

/// A mesh as it comes from the model, for `BusGeom::build`.
pub struct MeshIn<'a> {
    pub positions: &'a [Vec3],
    pub normals: &'a [Vec3],
    pub uvs: &'a [Vec2],
    pub indices: &'a [u32],
    /// (first index, count, slot) as `MeshData::ranges`.
    pub ranges: &'a [(u32, u32, u32)],
    /// Per slot: the painted texture it shows, whether it is glass and whether it is kept.
    pub slots: Vec<(Option<u8>, bool, bool)>,
    pub transform: Mat4,
}

// --- what a mesh is to the paint ---------------------------------------------------------------

/// What a mesh of the bus is to the paint, from its file's name and its part in the model
/// (Omsi-Hub's protection: `lakdoek.ts` keeps the wheels like glass, `WIEL`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Body,
    /// A sticker on the body (lettering, a logo): see-through round its print, but paint.
    Decal,
    Glass,
    Wheel,
    Lamp,
    Rubber,
    /// A mirror: its glass, housing and arm.
    Mirror,
    /// A wiper, a number plate or a badge: never painted. (A grille is told by its colour: the
    /// Citaro's `front_grill` is the panel below its windscreen, in the body's colour.)
    Trim,
    /// The articulation's bellows and joint.
    Bellows,
    Interior,
    Driver,
}

/// What a material slot of a mesh does with the paint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotUse {
    Paint,
    /// Painted only by the layers that go over the windows.
    Glass,
    /// Never painted, nor what shares its texels.
    Keep,
    /// Not painted itself (its texels are painted only where the body shares them).
    Skip,
}

/// The words of a mesh file's name, lower case: `21_wheel_T_HL_#low.o3d` is `wheel t hl low`.
fn name_words(file: &str) -> Vec<String> {
    let name = file.rsplit(['\\', '/']).next().unwrap_or(file);
    let stem = match name.rsplit_once('.') {
        Some((s, e)) if e.len() <= 4 => s,
        _ => name,
    };
    stem.to_lowercase().split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

impl Role {
    /// The role of a mesh: `file` its o3d, `turns_as_wheel` whether the model turns it by a
    /// `Wheel_Rotation_*`, `seen_outside` whether it is drawn in the outside view (`[viewpoint]`).
    pub fn of_mesh(file: &str, turns_as_wheel: bool, seen_outside: bool) -> Role {
        let words = name_words(file);
        let any = |f: &dyn Fn(&str) -> bool| words.iter().any(|w| f(w));
        // (whole words: `Rad_VR` is a wheel, `Lenkrad` and `Radio` are not)
        if turns_as_wheel || any(&|w| matches!(w, "rad" | "raeder" | "räder" | "rader" | "wheel" | "wheels" | "reifen" | "tyre" | "tyres" | "tire" | "tires" | "felge" | "felgen" | "rim" | "rims" | "radkappe" | "hubcap")) {
            Role::Wheel
        } else if any(&|w| w.contains("balg") || w.contains("bellow") || w.contains("gelenk") || w.starts_with("articul") || w.contains("drehteller") || w.contains("turntable")) {
            Role::Bellows
        } else if any(&|w| w.contains("leuchte") || w.contains("scheinwerfer") || w.contains("blinker") || w.contains("lamp") || w.contains("light") || w.starts_with("licht") || w.ends_with("licht") || matches!(w, "indicator" | "indicators")) {
            Role::Lamp
        } else if any(&|w| matches!(w, "lippe" | "lippen" | "rubber" | "rubbers" | "seal" | "seals") || w.contains("gummi") || w.contains("dichtung")) {
            Role::Rubber
        } else if any(&|w| w.contains("spiegel") || w.contains("mirror")) {
            Role::Mirror
        } else if any(&|w| w.contains("wischer") || w.contains("wiper") || w.starts_with("kennz") || w.contains("nummernschild") || w.contains("plate") || w.starts_with("stern") || matches!(w, "logo" | "logos" | "badge" | "badges" | "emblem")) {
            Role::Trim
        } else if any(&|w| matches!(w, "window" | "windows" | "windscreen" | "windshield") || w.ends_with("glas") || w.ends_with("glass") || w.ends_with("scheibe") || w.ends_with("scheiben")) {
            Role::Glass
        } else if any(&|w| matches!(w, "driver" | "fahrer" | "fahrerin" | "fahrerfigur" | "chauffeur")) {
            Role::Driver
        } else if !seen_outside || any(&|w| matches!(w, "innen" | "interior" | "inside" | "int" | "cockpit" | "cp")) {
            Role::Interior
        } else if any(&|w| w.starts_with("decal") || w.starts_with("aufkleber") || w.starts_with("sticker") || w.contains("werbung") || w.contains("schrift")) {
            Role::Decal
        } else {
            Role::Body
        }
    }

    /// What a slot of a mesh of this role does; `blended` a slot drawn see-through
    /// (`[matl_alpha] 2`, or a material not fully opaque).
    pub fn slot(self, blended: bool) -> SlotUse {
        match self {
            Role::Body if blended => SlotUse::Glass,
            Role::Body | Role::Decal => SlotUse::Paint,
            Role::Glass => SlotUse::Glass,
            Role::Wheel | Role::Lamp | Role::Rubber | Role::Mirror | Role::Trim => SlotUse::Keep,
            Role::Bellows | Role::Interior | Role::Driver => SlotUse::Skip,
        }
    }
}

pub struct BusGeom {
    pub tris: Vec<Tri>,
    /// Runs of `tris` with their box, for the mouse's ray.
    pub runs: Vec<(usize, usize, Vec3, Vec3)>,
    pub dims: BusDims,
}

impl BusGeom {
    pub fn build(meshes: &[MeshIn]) -> BusGeom {
        let mut tris = Vec::new();
        let mut runs = Vec::new();
        for m in meshes {
            let start = tris.len();
            let nm = m.transform.inverse().transpose();
            let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            for &(first, count, slot) in m.ranges {
                let (target, glass, keep) = m.slots.get(slot as usize).copied().unwrap_or((None, false, false));
                for t in m.indices[first as usize..(first + count) as usize].chunks_exact(3) {
                    let i = [t[0] as usize, t[1] as usize, t[2] as usize];
                    if i.iter().any(|&k| k >= m.positions.len()) {
                        continue;
                    }
                    let p = i.map(|k| m.transform.transform_point3(m.positions[k]));
                    let n = (p[1] - p[0]).cross(p[2] - p[0]);
                    if n.length_squared() < 1e-14 {
                        continue;
                    }
                    let n = n.normalize();
                    let vn = i.map(|k| m.normals.get(k).map(|v| nm.transform_vector3(*v).normalize_or(n)).unwrap_or(n));
                    let uv = i.map(|k| m.uvs.get(k).copied().unwrap_or(Vec2::ZERO));
                    for q in p {
                        lo = lo.min(q);
                        hi = hi.max(q);
                    }
                    tris.push(Tri { p, vn, uv, n, target, glass, keep });
                }
            }
            if tris.len() > start {
                runs.push((start, tris.len(), lo, hi));
            }
        }
        let dims = dims_of(&tris);
        BusGeom { tris, runs, dims }
    }

    /// The nearest triangle the ray hits: its index, the distance and the barycentric weights.
    pub fn ray(&self, o: Vec3, d: Vec3) -> Option<(usize, f32, Vec3)> {
        let inv = Vec3::new(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
        let mut best: Option<(usize, f32, Vec3)> = None;
        for &(a, b, lo, hi) in &self.runs {
            // (the box first)
            let t0 = (lo - o) * inv;
            let t1 = (hi - o) * inv;
            let tmin = t0.min(t1).max_element();
            let tmax = t0.max(t1).min_element();
            if tmax < tmin.max(0.0) || best.is_some_and(|b| tmin > b.1) {
                continue;
            }
            for (k, t) in self.tris[a..b].iter().enumerate() {
                if let Some((dist, w)) = ray_tri(o, d, &t.p) {
                    if best.is_none_or(|b| dist < b.1) {
                        best = Some((a + k, dist, w));
                    }
                }
            }
        }
        best
    }

    /// The point and the normal (facing the viewer at `o`) where the ray hits.
    pub fn hit(&self, o: Vec3, d: Vec3) -> Option<(usize, Vec3, Vec3, Vec3)> {
        let (i, t, w) = self.ray(o, d)?;
        let tri = &self.tris[i];
        let p = o + d * t;
        let mut n = (tri.vn[0] * w.x + tri.vn[1] * w.y + tri.vn[2] * w.z).normalize_or(tri.n);
        if n.dot(d) > 0.0 {
            n = -n;
        }
        Some((i, p, n, w))
    }
}

/// Möller-Trumbore: the distance along the ray and the barycentric weights.
fn ray_tri(o: Vec3, d: Vec3, p: &[Vec3; 3]) -> Option<(f32, Vec3)> {
    let e1 = p[1] - p[0];
    let e2 = p[2] - p[0];
    let h = d.cross(e2);
    let a = e1.dot(h);
    if a.abs() < 1e-9 {
        return None;
    }
    let f = 1.0 / a;
    let s = o - p[0];
    let u = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = f * d.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = f * e2.dot(q);
    (t > 1e-4).then_some((t, Vec3::new(1.0 - u - v, u, v)))
}

/// The bus's box from the painted triangles (all of them when none is painted; the wheels and
/// lamps not, which reach below and beyond the body), and the window line from its side windows.
fn dims_of(tris: &[Tri]) -> BusDims {
    let paints = |t: &&Tri| t.target.is_some() && !t.keep;
    let painted = tris.iter().any(|t| paints(&t));
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for t in tris.iter().filter(|t| !painted || paints(t)) {
        for p in t.p {
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    if !lo.is_finite() {
        (lo, hi) = (Vec3::new(-1.25, -6.0, 0.0), Vec3::new(1.25, 6.0, 3.0));
    }
    BusDims { min: lo, max: hi, window: window_line(tris, lo, hi) }
}

/// The windows' lower edge (Omsi-Hub's `raamlijn`): the height where most of the side glass
/// begins, weighed by how long its lower edge runs along the bus.
pub fn window_line(tris: &[Tri], lo: Vec3, hi: Vec3) -> Option<f32> {
    let h = hi.z - lo.z;
    let (z0, z1) = (lo.z + 0.25 * h, lo.z + 0.75 * h);
    let bins = 200usize;
    let mut hist = vec![0.0f32; bins];
    for t in tris.iter().filter(|t| t.glass && t.n.x.abs() > 0.7) {
        let z = t.p.iter().map(|p| p.z).fold(f32::INFINITY, f32::min);
        if !(z0..z1).contains(&z) {
            continue;
        }
        // (the length of its lower edge along the bus: a pane's upper triangle, which touches the
        // bottom only with a corner, counts nothing)
        let low: Vec<f32> = t.p.iter().filter(|p| p.z < z + 0.01).map(|p| p.y).collect();
        let len = low.iter().fold(f32::NEG_INFINITY, |a, b| a.max(*b)) - low.iter().fold(f32::INFINITY, |a, b| a.min(*b));
        let k = (((z - z0) / (z1 - z0)) * bins as f32) as usize;
        hist[k.min(bins - 1)] += len;
    }
    // (bins of a centimetre or so, smoothed over three; the lowest that comes near the most - the
    // panes above a transom run as long as the windows' lower edge)
    let smooth: Vec<f32> = (0..bins).map(|k| hist[k.saturating_sub(1)..(k + 2).min(bins)].iter().sum()).collect();
    let most = smooth.iter().fold(0.0f32, |a, b| a.max(*b));
    let k = smooth.iter().position(|w| *w >= most * 0.6).unwrap_or(0);
    (most > 0.5).then(|| z0 + (k as f32 + 0.5) / bins as f32 * (z1 - z0))
}

// --- seen from outside -------------------------------------------------------------------------

/// The bus seen from 26 directions round it (every combination of -1, 0, 1 but none): for each, how
/// far towards the viewer the nearest surface reaches in each cell of a grid across the view.
/// A point is on the outside when some direction it faces sees it there (Omsi-Hub's `buiten`):
/// the inside of the body, and what lies behind the windows, is not.
pub struct Outside {
    maps: Vec<DepthMap>,
}

struct DepthMap {
    d: Vec3,
    a: Vec3,
    b: Vec3,
    lo: Vec2,
    w: usize,
    h: usize,
    front: Vec<f32>,
}

/// The grid's cell (metres) and how far behind the nearest surface a point still counts as seen.
const CELL: f32 = 0.025;
const TOLERANCE: f32 = 0.04;

impl DepthMap {
    fn cell(&self, q: Vec2) -> Option<usize> {
        let c = ((q - self.lo) / CELL).floor();
        (c.x >= 0.0 && c.y >= 0.0 && (c.x as usize) < self.w && (c.y as usize) < self.h).then(|| c.y as usize * self.w + c.x as usize)
    }
}

impl Outside {
    pub fn build(tris: &[Tri]) -> Outside {
        let mut dirs = Vec::new();
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    if (x, y, z) != (0, 0, 0) {
                        dirs.push(Vec3::new(x as f32, y as f32, z as f32).normalize());
                    }
                }
            }
        }
        let maps = dirs.par_iter().map(|&d| Self::map(tris, d)).collect();
        Outside { maps }
    }

    fn map(tris: &[Tri], d: Vec3) -> DepthMap {
        let a = if d.z.abs() > 0.9 { Vec3::X } else { Vec3::Z.cross(d).normalize() };
        let b = d.cross(a).normalize();
        let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for t in tris {
            for p in t.p {
                let q = Vec2::new(p.dot(a), p.dot(b));
                lo = lo.min(q);
                hi = hi.max(q);
            }
        }
        if !lo.is_finite() {
            return DepthMap { d, a, b, lo: Vec2::ZERO, w: 1, h: 1, front: vec![f32::NEG_INFINITY] };
        }
        lo -= Vec2::splat(CELL);
        let w = (((hi.x - lo.x) / CELL).ceil() as usize + 2).min(4096);
        let h = (((hi.y - lo.y) / CELL).ceil() as usize + 2).min(4096);
        let mut front = vec![f32::NEG_INFINITY; w * h];
        for t in tris {
            let q = t.p.map(|p| (Vec2::new(p.dot(a), p.dot(b)) - lo) / CELL);
            let z = t.p.map(|p| p.dot(d));
            // the corners themselves (a triangle smaller than a cell still counts)
            for k in 0..3 {
                let (x, y) = (q[k].x as usize, q[k].y as usize);
                if x < w && y < h {
                    let f = &mut front[y * w + x];
                    *f = f.max(z[k]);
                }
            }
            let area = (q[1] - q[0]).perp_dot(q[2] - q[0]);
            if area.abs() < 1e-9 {
                continue;
            }
            let bl = q[0].min(q[1]).min(q[2]).floor().max(Vec2::ZERO);
            let br = q[0].max(q[1]).max(q[2]).ceil().min(Vec2::new(w as f32 - 1.0, h as f32 - 1.0));
            for y in bl.y as usize..=br.y as usize {
                for x in bl.x as usize..=br.x as usize {
                    let c = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = (q[1] - c).perp_dot(q[2] - c) / area;
                    let w1 = (q[2] - c).perp_dot(q[0] - c) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                        continue;
                    }
                    let zz = z[0] * w0 + z[1] * w1 + z[2] * w2;
                    let f = &mut front[y * w + x];
                    *f = f.max(zz);
                }
            }
        }
        DepthMap { d, a, b, lo, w, h, front }
    }

    /// Whether the point `p` with normal `n` is seen from outside the bus.
    pub fn sees(&self, p: Vec3, n: Vec3) -> bool {
        self.maps.iter().any(|m| {
            if n.dot(m.d) < 0.1 {
                return false;
            }
            let Some(c) = m.cell(Vec2::new(p.dot(m.a), p.dot(m.b))) else { return true };
            p.dot(m.d) >= m.front[c] - TOLERANCE
        })
    }
}

// --- the texture's texels on the bus ------------------------------------------------------------

/// Where a texel lies on the bus.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    pub p: Vec3,
    pub n: [i8; 3],
    /// `COVERED`, `OUTSIDE`, `KEEP`, `GLASS`
    pub flags: u8,
}

pub const COVERED: u8 = 1;
pub const OUTSIDE: u8 = 2;
/// A wheel, a lamp, a rubber or a mirror's glass lies on the texel: never painted.
pub const KEEP: u8 = 4;
/// A window lies on the texel: painted only by the layers that go over the windows.
pub const GLASS: u8 = 8;

impl Sample {
    pub fn normal(&self) -> Vec3 {
        Vec3::new(self.n[0] as f32, self.n[1] as f32, self.n[2] as f32) / 127.0
    }
    fn of(p: Vec3, n: Vec3) -> Sample {
        let q = |v: f32| (v.clamp(-1.0, 1.0) * 127.0).round() as i8;
        Sample { p, n: [q(n.x), q(n.y), q(n.z)], flags: COVERED }
    }
}

/// The texels of rows `y0..y1` of a `w` x `h` texture: their first place on the bus, and a second
/// where the texture is used twice (left and right sharing it), and for the texels no triangle
/// covers but one lies near, where to take their colour from (the seams' bleed).
#[derive(Clone)]
pub struct Bake {
    pub w: u32,
    pub y0: u32,
    pub y1: u32,
    pub first: Vec<Sample>,
    pub second: Vec<u32>,
    pub extra: Vec<Sample>,
    pub bleed: Vec<(u32, u32)>,
    /// Texels per metre on the bus (the median of the covered texels').
    pub density: f32,
}

/// How far (texels) the colour bleeds past the edges of what the triangles cover.
pub const BLEED: u32 = 4;

impl Bake {
    /// The samples of texel `i` of the band.
    pub fn samples(&self, i: usize) -> (Sample, Option<Sample>) {
        let s = self.second[i];
        (self.first[i], (s != u32::MAX).then(|| self.extra[s as usize]))
    }

    /// Bake the triangles of `target` into rows `y0..y1` (and `BLEED` rows round them, which the
    /// bleed needs but which are not kept) of a `w` x `h` texture.
    pub fn build(tris: &[Tri], target: u8, w: u32, h: u32, y0: u32, y1: u32, outside: &Outside) -> Bake {
        let (ry0, ry1) = (y0, y1.min(h));
        let rows = (ry1 - ry0) as usize;
        let wu = w as usize;
        let n = wu * rows;
        let mut first = vec![Sample::default(); n];
        let mut second = vec![u32::MAX; n];
        let mut extra: Vec<Sample> = Vec::new();
        let mine: Vec<&Tri> = tris.iter().filter(|t| t.target == Some(target)).collect();
        // per triangle: its texel corners, moved into the texture's 0..1 by its middle's tile
        let corners: Vec<[Vec2; 3]> = mine
            .iter()
            .map(|t| {
                let c = (t.uv[0] + t.uv[1] + t.uv[2]) / 3.0;
                let shift = c.floor();
                t.uv.map(|uv| (uv - shift) * Vec2::new(w as f32, h as f32))
            })
            .collect();
        let mut density = Vec::new();
        for pass in 0..2 {
            // the strict pass, then one a little wider for what slivers missed
            let margin = if pass == 0 { 0.0 } else { 0.75 };
            for (t, q) in mine.iter().zip(&corners) {
                let area = (q[1] - q[0]).perp_dot(q[2] - q[0]);
                if area.abs() < 1e-9 {
                    continue;
                }
                if pass == 0 && density.len() < 4096 {
                    let world = (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).length();
                    if world > 1e-8 {
                        density.push((area.abs() / world).sqrt());
                    }
                }
                let class = if t.keep { KEEP } else { 0 } | if t.glass { GLASS } else { 0 };
                let edge = |a: Vec2, b: Vec2| (b - a).length().max(1e-6);
                let lens = [edge(q[1], q[2]), edge(q[2], q[0]), edge(q[0], q[1])];
                let bl = (q[0].min(q[1]).min(q[2]) - Vec2::splat(margin)).floor();
                let br = (q[0].max(q[1]).max(q[2]) + Vec2::splat(margin)).ceil();
                for y in bl.y as i64..br.y as i64 {
                    let wy = y.rem_euclid(h as i64) as u32;
                    if wy < ry0 || wy >= ry1 {
                        continue;
                    }
                    for x in bl.x as i64..br.x as i64 {
                        let c = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                        let e = [(q[1] - c).perp_dot(q[2] - c), (q[2] - c).perp_dot(q[0] - c), (q[0] - c).perp_dot(q[1] - c)];
                        let s = area.signum();
                        // signed distances to the edges, in texels (inside positive)
                        let dist = [e[0] * s / lens[0], e[1] * s / lens[1], e[2] * s / lens[2]];
                        if dist.iter().any(|d| *d < -margin - 1e-5) {
                            continue;
                        }
                        let mut bw = Vec3::new(e[0] / area, e[1] / area, e[2] / area).max(Vec3::ZERO);
                        bw /= (bw.x + bw.y + bw.z).max(1e-9);
                        let p = t.p[0] * bw.x + t.p[1] * bw.y + t.p[2] * bw.z;
                        let nn = (t.vn[0] * bw.x + t.vn[1] * bw.y + t.vn[2] * bw.z).normalize_or(t.n);
                        let i = (wy - ry0) as usize * wu + x.rem_euclid(w as i64) as usize;
                        if first[i].flags & COVERED == 0 {
                            first[i] = Sample::of(p, nn);
                            first[i].flags |= class;
                        } else if pass == 0 {
                            // (a wheel or a window on a texel the body shares keeps it as it is)
                            first[i].flags |= class;
                            if second[i] == u32::MAX && first[i].p.distance(p) > 0.25 {
                                second[i] = extra.len() as u32;
                                extra.push(Sample::of(p, nn));
                            }
                        }
                    }
                }
            }
        }
        // which are seen from outside
        first.par_iter_mut().chain(extra.par_iter_mut()).for_each(|s| {
            if s.flags & COVERED != 0 && outside.sees(s.p, s.normal()) {
                s.flags |= OUTSIDE;
            }
        });
        // the bleed: the uncovered texels near covered ones take the nearest one's colour
        let mut src: Vec<u32> = (0..n as u32).map(|i| if first[i as usize].flags & COVERED != 0 { i } else { u32::MAX }).collect();
        let mut bleed = Vec::new();
        for _ in 0..BLEED {
            let before = src.clone();
            for y in 0..rows {
                for x in 0..wu {
                    let i = y * wu + x;
                    if before[i] != u32::MAX {
                        continue;
                    }
                    let mut found = u32::MAX;
                    for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
                        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                        if nx < 0 || ny < 0 || nx >= wu as i64 || ny >= rows as i64 {
                            continue;
                        }
                        let s = before[ny as usize * wu + nx as usize];
                        if s != u32::MAX {
                            found = s;
                            break;
                        }
                    }
                    if found != u32::MAX {
                        src[i] = found;
                        bleed.push((i as u32, found));
                    }
                }
            }
        }
        density.sort_by(|a, b| a.total_cmp(b));
        let density = density.get(density.len() / 2).copied().unwrap_or(100.0);
        Bake { w, y0: ry0, y1: ry1, first, second, extra, bleed, density }
    }
}

// --- the base's colour zones ---------------------------------------------------------------------

/// The colour zones of one painted texture: k-means in CIELAB over its outside texels (Omsi-Hub's
/// `zones.ts`), k 8, centres closer than ΔE 8 merged, each with its share of the texture's
/// outside, largest first.
///
/// A zone is paint when it covers a tenth of the outside or more and is no trim colour - black,
/// very dark, grey or chrome stay what they are (window surrounds, the band above the windows,
/// rubbers, bumpers, grilles, arch liners) unless it is the largest zone of the bus's main
/// texture and no other colour is paint (a black or a silver bus) - when it has the colour of a paint zone of the bus's main
/// texture (one livery over all the textures: a parts atlas keeps its greys, its body-coloured
/// parts follow the body), or when it is a shade of a paint zone (its hue a little more or less
/// saturated, or, no trim colour itself, within ΔE 20 of it: the shading and the dirt).
///
/// The paint zones fall into groups, the same over all the textures: 0 is the body's colour (the
/// largest paint zone of the main texture and its shades), 1 the second colour (the roof of a
/// two-tone livery, a skirt), and so on.
#[derive(Clone, Debug, Default)]
pub struct Zones {
    pub centres: Vec<[f32; 3]>,
    /// The centres in linear light.
    pub lin: Vec<[f32; 3]>,
    pub share: Vec<f32>,
    pub painted: Vec<bool>,
    /// Per zone its paint group (`NO_GROUP` for no paint).
    pub group: Vec<u8>,
    /// Per zone its light against its group's colour (a shade of the body is darker than the
    /// body, and a new colour there as much darker); 1 for no paint.
    pub rel: Vec<f32>,
}

pub const NO_GROUP: u8 = u8::MAX;

/// How far (ΔE) a shade of a paint zone still counts as that paint.
const SHADE: f32 = 20.0;

pub fn chroma(lab: [f32; 3]) -> f32 {
    lab[1].hypot(lab[2])
}

/// Black, very dark, grey or chrome: a trim's colour, which a livery leaves as it is.
pub fn trim_like(lab: [f32; 3]) -> bool {
    let c = chroma(lab);
    (lab[0] < 32.0 && c < 14.0) || (lab[0] < 70.0 && c < 12.0)
}

/// The relative luminance Y (linear) of a CIELAB colour.
pub fn luminance(lab: [f32; 3]) -> f32 {
    let f = (lab[0] + 16.0) / 116.0;
    if f.powi(3) > 0.008856 { f.powi(3) } else { (f - 16.0 / 116.0) / 7.787 }
}

/// k-means of `pts` into `k` (k-means++ seeds from `rnd`): the centres, each point's centre and
/// the summed squared distance.
fn kmeans(pts: &[[f32; 3]], k: usize, rnd: &mut impl FnMut() -> f32) -> (Vec<[f32; 3]>, Vec<u8>, f64) {
    let d2 = |a: &[f32; 3], b: &[f32; 3]| (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2);
    let mut centres = vec![pts[((rnd() * pts.len() as f32) as usize).min(pts.len() - 1)]];
    let mut near = vec![f32::INFINITY; pts.len()];
    while centres.len() < k {
        let last = *centres.last().unwrap();
        let mut sum = 0.0f64;
        for (p, n) in pts.iter().zip(near.iter_mut()) {
            *n = n.min(d2(p, &last));
            sum += *n as f64;
        }
        let mut r = rnd() as f64 * sum;
        let mut pick = pts.len() - 1;
        for (i, n) in near.iter().enumerate() {
            r -= *n as f64;
            if r <= 0.0 {
                pick = i;
                break;
            }
        }
        centres.push(pts[pick]);
    }
    let mut of = vec![u8::MAX; pts.len()];
    let mut err = 0.0f64;
    for _ in 0..16 {
        let mut sum = vec![[0.0f64; 4]; k];
        let mut moved = 0;
        err = 0.0;
        for (p, o) in pts.iter().zip(of.iter_mut()) {
            let (i, d) = centres.iter().enumerate().map(|(i, c)| (i, d2(p, c))).fold((0, f32::INFINITY), |a, b| if b.1 < a.1 { b } else { a });
            if *o != i as u8 {
                moved += 1;
                *o = i as u8;
            }
            err += d as f64;
            for c in 0..3 {
                sum[i][c] += p[c] as f64;
            }
            sum[i][3] += 1.0;
        }
        for (c, s) in centres.iter_mut().zip(&sum) {
            if s[3] > 0.0 {
                *c = [(s[0] / s[3]) as f32, (s[1] / s[3]) as f32, (s[2] / s[3]) as f32];
            }
        }
        if moved < pts.len() / 200 {
            break;
        }
    }
    (centres, of, err)
}

/// The colour clusters of `colours`: centres (CIELAB) and shares, largest first.
fn clusters(colours: &[[u8; 3]]) -> Vec<([f32; 3], f32)> {
    if colours.is_empty() {
        return Vec::new();
    }
    let step = (colours.len() / 6000).max(1);
    let pts: Vec<[f32; 3]> = colours.iter().step_by(step).map(|c| colour::lab(*c)).collect();
    // (a seeded randomness: the same bus gives the same zones)
    let mut seed = 7u32;
    let mut rnd = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    // (eight, the close ones merged after: an elbow stopped at two on a yellow bus with grey,
    // red and black, and took their mean for a colour of the livery)
    let (centres, of, _) = kmeans(&pts, 8.min(pts.len()), &mut rnd);
    let mut zones: Vec<([f32; 3], usize)> = centres.iter().enumerate().map(|(i, c)| (*c, of.iter().filter(|o| **o as usize == i).count())).filter(|z| z.1 > 0).collect();
    // merge what lies closer than ΔE 8
    loop {
        let pair = (0..zones.len()).flat_map(|a| (a + 1..zones.len()).map(move |b| (a, b))).find(|&(a, b)| colour::delta_e(zones[a].0, zones[b].0) < 8.0);
        let Some((a, b)) = pair else { break };
        let (ca, na) = zones[a];
        let (cb, nb) = zones.remove(b);
        let t = (na + nb).max(1) as f32;
        zones[a] = (std::array::from_fn(|k| (ca[k] * na as f32 + cb[k] * nb as f32) / t), na + nb);
    }
    zones.sort_by_key(|z| std::cmp::Reverse(z.1));
    let total = pts.len() as f32;
    zones.into_iter().map(|(c, n)| (c, n as f32 / total)).collect()
}

/// Whether `c` is a tint of the paint `p`: its hue (within 12°), 0.4..1.6 times as saturated.
fn tint_of(c: [f32; 3], p: [f32; 3]) -> bool {
    let hue = |l: [f32; 3]| l[2].atan2(l[1]);
    chroma(c) >= 12.0 && chroma(p) >= 12.0 && {
        let d = hue(c) - hue(p);
        let r = chroma(c) / chroma(p);
        d.sin().atan2(d.cos()).abs() < 12f32.to_radians() && (0.4..=1.6).contains(&r)
    }
}

/// Whether `c` is a shade of the paint `p`: a tint of it, or no trim colour within ΔE 20 of it.
fn shade_of(c: [f32; 3], p: [f32; 3]) -> bool {
    tint_of(c, p) || ((!trim_like(c) || trim_like(p)) && colour::delta_e(c, p) < SHADE)
}

impl Zones {
    /// The zones of one texture alone (as the bus's main texture).
    pub fn of(colours: &[[u8; 3]]) -> Zones {
        Zones::of_targets(&[colours.to_vec()]).pop().unwrap_or_default()
    }

    /// The zones of each painted texture of a bus from its outside colours; the texture with the
    /// most outside texels is the main one, whose paint zones say the livery's colours.
    pub fn of_targets(colours: &[Vec<[u8; 3]>]) -> Vec<Zones> {
        let main = (0..colours.len()).max_by_key(|i| colours[*i].len());
        let raw: Vec<Vec<([f32; 3], f32)>> = colours.iter().map(|c| clusters(c)).collect();
        // the groups' colours, from the main texture: its large zones that are no trim (its
        // largest when no other is), a shade of a group in that group; another texture's zones
        // are paint only in the main texture's colours - a parts atlas's greys and the yellow of
        // the handrails' fittings are no livery, its parts in the body's colour follow the body
        let mut groups: Vec<[f32; 3]> = Vec::new();
        let mut assigned: Vec<Vec<u8>> = raw.iter().map(|z| vec![NO_GROUP; z.len()]).collect();
        let order: Vec<usize> = main.into_iter().chain((0..raw.len()).filter(|k| Some(*k) != main)).collect();
        for &k in &order {
            // (a black or a silver bus: its largest zone is paint when no other colour is)
            let coloured = raw[k].iter().any(|(c, s)| *s >= 0.1 && !trim_like(*c));
            for (i, &(c, share)) in raw[k].iter().enumerate() {
                let like = groups.iter().enumerate().filter(|(_, g)| colour::delta_e(**g, c) < 12.0).min_by(|a, b| colour::delta_e(*a.1, c).total_cmp(&colour::delta_e(*b.1, c))).map(|(g, _)| g);
                let own = Some(k) == main && ((share >= 0.1 && !trim_like(c)) || (i == 0 && !coloured));
                assigned[k][i] = match like {
                    Some(g) => g as u8,
                    None if own => match groups.iter().position(|g| shade_of(c, *g)) {
                        Some(g) => g as u8,
                        None => {
                            groups.push(c);
                            (groups.len() - 1).min(250) as u8
                        }
                    },
                    None => NO_GROUP,
                };
            }
        }
        let mut out = Vec::new();
        for (k, zones) in raw.iter().enumerate() {
            let centres: Vec<[f32; 3]> = zones.iter().map(|z| z.0).collect();
            let share: Vec<f32> = zones.iter().map(|z| z.1).collect();
            let mut group = assigned[k].clone();
            // and the shades of the paint, in the group they shade: 2 % of the texture or more, a
            // tint of it (the paint in a grille's shadow) from 0.3 %
            let paint: Vec<usize> = (0..centres.len()).filter(|i| group[*i] != NO_GROUP).collect();
            for i in 0..centres.len() {
                if group[i] != NO_GROUP || share[i] < 0.003 {
                    continue;
                }
                let like = |j: usize| if share[i] >= 0.02 { shade_of(centres[i], centres[j]) } else { tint_of(centres[i], centres[j]) };
                if let Some(&j) = paint.iter().filter(|&&j| like(j)).min_by(|a, b| colour::delta_e(centres[**a], centres[i]).total_cmp(&colour::delta_e(centres[**b], centres[i]))) {
                    group[i] = group[j];
                }
            }
            let lin = centres.iter().map(|c| lab_to_linear(*c)).collect();
            let painted = group.iter().map(|g| *g != NO_GROUP).collect();
            let rel = centres.iter().zip(&group).map(|(c, g)| if *g == NO_GROUP { 1.0 } else { (luminance(*c) / luminance(groups[*g as usize]).max(1e-4)).clamp(0.3, 1.4) }).collect();
            out.push(Zones { centres, lin, share, painted, group, rel });
        }
        out
    }

    pub fn is_paint(&self, i: usize) -> bool {
        self.painted.get(i).copied().unwrap_or(false)
    }

    #[cfg(test)]
    /// The paint mask Z of a texel of colour `lab` (Omsi-Hub §4.4), for a look at the zones:
    /// how much it belongs to the paint, soft over ±6 ΔE between the nearest paint zone and the
    /// nearest other.
    pub fn paint(&self, lab: [f32; 3]) -> f32 {
        let (mut dp, mut dn) = (f32::INFINITY, f32::INFINITY);
        for (i, c) in self.centres.iter().enumerate() {
            let d = colour::delta_e(*c, lab);
            if self.is_paint(i) { dp = dp.min(d) } else { dn = dn.min(d) }
        }
        if !dn.is_finite() || !dp.is_finite() {
            return if dp.is_finite() || self.centres.is_empty() { 1.0 } else { 0.0 };
        }
        ((dn - dp) / 12.0 + 0.5).clamp(0.0, 1.0)
    }

    /// The detail ratio for a layer that paints over everything (Omsi-Hub §4.4): the texel's
    /// luminance over its zone's, gently (a quarter of a darkening let go) and within 0.55..1.15,
    /// so that dark dirt never turns into black blotches.
    pub fn detail(&self, lab: [f32; 3]) -> f32 {
        let near = self.centres.iter().enumerate().map(|(i, c)| (i, colour::delta_e(*c, lab))).fold(None, |a: Option<(usize, f32)>, b| if a.is_none_or(|a| b.1 < a.1) { Some(b) } else { a });
        let Some((i, _)) = near else { return 1.0 };
        let r = luminance(lab) / luminance(self.centres[i]).max(1e-4);
        let r = if r < 1.0 { 1.0 - (1.0 - r) * 0.75 } else { r };
        r.clamp(0.55, 1.15)
    }

    /// The radius for a fill from the zone at `lab` (Omsi-Hub's `vulStraal`): its shades together
    /// (within ΔE 30) as long as another zone lies farther, else half way to the nearest.
    pub fn fill_radius(&self, lab: [f32; 3]) -> f32 {
        let d: Vec<f32> = self.centres.iter().map(|c| colour::delta_e(*c, lab)).collect();
        let group = d.iter().filter(|x| **x > 0.5 && **x <= 30.0).fold(0.0f32, |a, b| a.max(*b));
        let other = d.iter().filter(|x| **x > 30.0).fold(f32::INFINITY, |a, b| a.min(*b));
        if !other.is_finite() {
            return 1000.0;
        }
        if group > 0.0 && group < other - 4.0 {
            return (group + other) * 0.5;
        }
        let near = d.iter().filter(|x| **x > 0.5).fold(f32::INFINITY, |a, b| a.min(*b));
        if near.is_finite() { (near * 0.5).clamp(4.0, 40.0) } else { 1000.0 }
    }

    /// How a texel of colour `c` (linear) is made of the zones (the recolour's unmixing): a mix
    /// `t·A + (1 - t)·B` of two zones or of a zone and black (`BLACK`) - a texel on the edge
    /// between the body and a trim is half of each, a shaded or dirtied texel of the body is the
    /// body mixed with black, a highlight more than the body (t up to 1.3, against black only).
    /// Of the three zones nearest in CIELAB, the pair that leaves the least.
    pub fn unmix(&self, c: [f32; 3], lab: [f32; 3]) -> (u8, u8, f32) {
        if self.centres.is_empty() {
            return (BLACK, BLACK, 0.0);
        }
        let mut near: [(usize, f32); 3] = [(usize::MAX, f32::INFINITY); 3];
        for (i, z) in self.centres.iter().enumerate() {
            let d = colour::delta_e(*z, lab);
            if d < near[2].1 {
                near[2] = (i, d);
                near.sort_by(|a, b| a.1.total_cmp(&b.1));
            }
        }
        let black = [0.0f32; 3];
        let col = |k: usize| if k == usize::MAX { black } else { self.lin[k] };
        let mut cand: [usize; 4] = [usize::MAX; 4];
        for (n, (i, _)) in near.iter().enumerate() {
            cand[n] = *i;
        }
        let mut best = (near[0].0, usize::MAX, 1.0f32, f32::INFINITY);
        // (a zone alone: t = 1 against itself, the residue its distance)
        for a in 0..4 {
            let ia = cand[a];
            if ia == usize::MAX {
                continue;
            }
            for b in 0..4 {
                let ib = if b == 3 { usize::MAX } else { cand[b] };
                if b != 3 && (ib == usize::MAX || ib <= ia) {
                    continue;
                }
                let (pa, pb) = (col(ia), col(ib));
                let d = [pa[0] - pb[0], pa[1] - pb[1], pa[2] - pb[2]];
                let dd = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                if dd < 1e-8 {
                    continue;
                }
                let e = [c[0] - pb[0], c[1] - pb[1], c[2] - pb[2]];
                let hi = if ib == usize::MAX { 1.3 } else { 1.0 };
                let t = ((e[0] * d[0] + e[1] * d[1] + e[2] * d[2]) / dd).clamp(0.0, hi);
                let r = (0..3).map(|k| (c[k] - (t * pa[k] + (1.0 - t) * pb[k])).powi(2)).sum::<f32>();
                // (relative to the texel's brightness: a dark texel's residue is small anyway;
                // what fits as well is a zone shaded rather than two zones mixed, and a zone at
                // its own strength rather than another one shaded - a grey trim on a white bus
                // is the trim, not the body in shadow)
                let r = r / (c[0] + c[1] + c[2] + 0.05).powi(2) + if ib == usize::MAX { 0.006 * (1.0 - t.min(1.0)).powi(2) } else { 0.002 };
                if r < best.3 - 1e-7 {
                    best = (ia, ib, t, r);
                }
            }
        }
        let id = |k: usize| if k == usize::MAX { BLACK } else { k as u8 };
        (id(best.0), id(best.1), best.2)
    }
}

/// Black in an unmixing: not a zone.
pub const BLACK: u8 = u8::MAX;

/// CIELAB → linear light.
pub fn lab_to_linear(lab: [f32; 3]) -> [f32; 3] {
    let fy = (lab[0] + 16.0) / 116.0;
    let fx = fy + lab[1] / 500.0;
    let fz = fy - lab[2] / 200.0;
    let g = |t: f32| if t.powi(3) > 0.008856 { t.powi(3) } else { (t - 16.0 / 116.0) / 7.787 };
    let (x, y, z) = (g(fx) * 0.95047, g(fy), g(fz) * 1.08883);
    [(3.2406 * x - 1.5372 * y - 0.4986 * z).max(0.0), (-0.9689 * x + 1.8758 * y + 0.0415 * z).max(0.0), (0.0557 * x - 0.2040 * y + 1.0570 * z).max(0.0)]
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A box-shaped bus `w` x `l` x `h` (outward faces, two triangles each), its sides painted
    /// through texture 0: the left side in the left half of the texture, the right in the right
    /// half (the front, rear and roof unpainted); and a window pane on the left side.
    pub fn box_bus() -> BusGeom {
        let (x0, x1, y0, y1, z0, z1) = (-1.25f32, 1.25, -6.0, 6.0, 0.3, 3.3);
        let quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3, uv: [Vec2; 4], target: Option<u8>, glass: bool| -> [Tri; 2] {
            let n = (b - a).cross(c - a).normalize();
            [Tri { p: [a, b, c], vn: [n; 3], uv: [uv[0], uv[1], uv[2]], n, target, glass, keep: false }, Tri { p: [a, c, d], vn: [n; 3], uv: [uv[0], uv[2], uv[3]], n, target, glass, keep: false }]
        };
        let v = Vec3::new;
        let uvq = |u0: f32, u1: f32| [Vec2::new(u0, 1.0), Vec2::new(u1, 1.0), Vec2::new(u1, 0.0), Vec2::new(u0, 0.0)];
        let mut tris = Vec::new();
        // the left side (normal -x), seen from outside: rear at the right
        tris.extend(quad(v(x0, y1, z0), v(x0, y0, z0), v(x0, y0, z1), v(x0, y1, z1), uvq(0.0, 0.5), Some(0), false));
        // the right side (normal +x)
        tris.extend(quad(v(x1, y0, z0), v(x1, y1, z0), v(x1, y1, z1), v(x1, y0, z1), uvq(0.5, 1.0), Some(0), false));
        // front, rear, roof, floor: unpainted
        tris.extend(quad(v(x1, y1, z0), v(x0, y1, z0), v(x0, y1, z1), v(x1, y1, z1), uvq(0.0, 0.1), None, false));
        tris.extend(quad(v(x0, y0, z0), v(x1, y0, z0), v(x1, y0, z1), v(x0, y0, z1), uvq(0.0, 0.1), None, false));
        tris.extend(quad(v(x0, y0, z1), v(x1, y0, z1), v(x1, y1, z1), v(x0, y1, z1), uvq(0.0, 0.1), None, false));
        tris.extend(quad(v(x0, y1, z0), v(x1, y1, z0), v(x1, y0, z0), v(x0, y0, z0), uvq(0.0, 0.1), None, false));
        // a window pane outside the left side, 1.4 m up (a hand's breadth off it, as glass in a frame)
        tris.extend(quad(v(x0 - 0.1, y1 - 1.0, 1.4), v(x0 - 0.1, y0 + 1.0, 1.4), v(x0 - 0.1, y0 + 1.0, 2.8), v(x0 - 0.1, y1 - 1.0, 2.8), uvq(0.0, 0.1), None, true));
        let runs = vec![(0, tris.len(), v(x0 - 0.1, y0, z0), v(x1, y1, z1))];
        let dims = dims_of(&tris);
        BusGeom { tris, runs, dims }
    }

    #[test]
    fn the_box_and_the_window_line_come_from_the_shape() {
        let g = box_bus();
        assert_eq!(g.dims.min, Vec3::new(-1.25, -6.0, 0.3));
        assert_eq!(g.dims.max, Vec3::new(1.25, 6.0, 3.3));
        let w = g.dims.window.unwrap();
        assert!((w - 1.4).abs() < 0.02, "{w}");
    }

    #[test]
    fn the_mouse_finds_the_side_it_points_at() {
        let g = box_bus();
        let (i, p, n, _) = g.hit(Vec3::new(10.0, 1.0, 1.0), Vec3::NEG_X).unwrap();
        assert_eq!(g.tris[i].target, Some(0));
        assert!((p - Vec3::new(1.25, 1.0, 1.0)).length() < 1e-4);
        assert!(n.dot(Vec3::X) > 0.99);
        // through the window pane first on the left
        let (i, _, _, _) = g.hit(Vec3::new(-10.0, 1.0, 2.0), Vec3::X).unwrap();
        assert!(g.tris[i].glass);
        assert!(g.hit(Vec3::new(10.0, 1.0, 10.0), Vec3::X).is_none());
    }

    #[test]
    fn texels_know_their_place_and_whether_they_are_outside() {
        let g = box_bus();
        let out = Outside::build(&g.tris);
        let b = Bake::build(&g.tris, 0, 64, 32, 0, 32, &out);
        // a texel in the right half lies on the right side, up from the bottom of the texture
        let (x, y) = (48usize, 28usize);
        let s = b.first[y * 64 + x];
        assert!(s.flags & COVERED != 0);
        assert!((s.p.x - 1.25).abs() < 1e-3, "{:?}", s.p);
        let v = 1.0 - (y as f32 + 0.5) / 32.0;
        assert!((s.p.z - (0.3 + 3.0 * v)).abs() < 0.01, "{:?}", s.p);
        assert!(s.normal().x > 0.95);
        assert!(s.flags & OUTSIDE != 0, "the side is seen");
        // every texel is covered by one side or the other
        assert!(b.first.iter().all(|s| s.flags & COVERED != 0));
        assert!(b.density > 4.0 && b.density < 12.0, "{}", b.density);
        // behind the window pane the left side is not outside (it is seen through glass); below it is
        let behind = b.first.iter().filter(|s| s.p.x < 0.0 && s.p.z > 1.6 && s.p.z < 2.6 && s.p.y.abs() < 4.0);
        assert!(behind.clone().count() > 10 && behind.clone().all(|s| s.flags & OUTSIDE == 0));
        assert!(b.first.iter().filter(|s| s.p.x < 0.0 && s.p.z < 1.2).all(|s| s.flags & OUTSIDE != 0));
        // a band of rows alone
        let band = Bake::build(&g.tris, 0, 64, 32, 8, 16, &out);
        assert_eq!(band.first.len(), 64 * 8);
        assert_eq!(band.first[0].p, b.first[8 * 64].p);
    }

    #[test]
    fn a_texture_used_on_both_sides_keeps_both_places() {
        let mut g = box_bus();
        // both sides on the whole texture
        for t in g.tris.iter_mut().filter(|t| t.target == Some(0)) {
            let right = t.n.x > 0.0;
            for uv in t.uv.iter_mut() {
                uv.x = if right { (uv.x - 0.5) * 2.0 } else { uv.x * 2.0 };
            }
        }
        let out = Outside::build(&g.tris);
        let b = Bake::build(&g.tris, 0, 32, 16, 0, 16, &out);
        let (one, two) = b.samples(8 * 32 + 10);
        let two = two.expect("a second place");
        assert!((one.p.x * two.p.x) < 0.0, "one on each side");
    }

    #[test]
    fn meshes_are_told_apart_by_their_name_and_their_part_in_the_model() {
        let r = |f: &str| Role::of_mesh(f, false, true);
        // the HH20's and others' wheels; not a steering wheel, a radio or a wheel arch
        for f in ["21_wheel_HL.o3d", "21_wheel_T_HR_#low.o3d", "model\\Rad_VR.o3d", "Reifen_hinten.o3d", "felge_l.o3d", "tyres.o3d"] {
            assert_eq!(r(f), Role::Wheel, "{f}");
        }
        assert_eq!(Role::of_mesh("achse2.o3d", true, true), Role::Wheel, "turned by Wheel_Rotation_*");
        for f in ["17_lenkrad.o3d", "radio.o3d", "radkasten.o3d", "21_aussen_weich1.o3d", "21_T_aussen_hart.o3d"] {
            assert_eq!(r(f), Role::Body, "{f}");
        }
        // stickers: see-through round their print, but paint (a livery covers the old lettering)
        assert_eq!(r("21_decals_aussen_1.o3d"), Role::Decal);
        assert_eq!(r("21_T_aufkleber_aussen_hochbahn.o3d"), Role::Decal);
        assert_eq!(r("21_decals_int.o3d"), Role::Interior);
        assert_eq!(Role::Decal.slot(true), SlotUse::Paint);
        assert_eq!(r("19_leuchten_abblend.o3d"), Role::Lamp);
        assert_eq!(r("77_T_leuchten_rueck_base_#low.o3d"), Role::Lamp);
        assert_eq!(r("21_tuer1_oberlicht.o3d"), Role::Lamp);
        assert_eq!(r("77_tuer4_lippe1.o3d"), Role::Rubber);
        assert_eq!(r("fenstergummi.o3d"), Role::Rubber);
        assert_eq!(r("21_gelenkbuegel_aussen.o3d"), Role::Bellows);
        assert_eq!(r("faltenbalg.o3d"), Role::Bellows);
        assert_eq!(r("77_articul_basis.o3d"), Role::Bellows);
        assert_eq!(r("17_aussenspiegel.o3d"), Role::Mirror);
        assert_eq!(r("21_scheiben_aussen.o3d"), Role::Glass);
        assert_eq!(r("19_tuer1_glas.o3d"), Role::Glass);
        assert_eq!(r("21_scheiben_innen.o3d"), Role::Glass, "a window's inside is glass, not the interior");
        assert_eq!(r("21_innen_hart.o3d"), Role::Interior);
        assert_eq!(r("17_cp_kmh.o3d"), Role::Interior);
        assert_eq!(Role::of_mesh("21_T_AI_int.o3d", false, false), Role::Interior);
        assert_eq!(Role::of_mesh("body.o3d", false, false), Role::Interior, "drawn only inside ([viewpoint] 2)");
        assert_eq!(r("fahrer.o3d"), Role::Driver);
        // what the slots do with the paint
        assert_eq!(Role::Body.slot(false), SlotUse::Paint);
        assert_eq!(Role::Body.slot(true), SlotUse::Glass, "a see-through material of the body is a window");
        assert_eq!(Role::Glass.slot(false), SlotUse::Glass);
        assert_eq!(Role::Wheel.slot(false), SlotUse::Keep);
        assert_eq!(Role::Lamp.slot(true), SlotUse::Keep);
        assert_eq!(Role::Rubber.slot(false), SlotUse::Keep);
        assert_eq!(Role::Mirror.slot(true), SlotUse::Keep, "a mirror's glass");
        assert_eq!(Role::Mirror.slot(false), SlotUse::Keep, "its housing and arm");
        assert_eq!(r("front_grill_s_e3_3.o3d"), Role::Body, "a grille by its colour");
        for f in ["scheibenwischer_l.o3d", "wiper_front.o3d", "kennzeichen_v.o3d", "number_plate.o3d", "mb_stern.o3d", "logo.o3d"] {
            assert_eq!(r(f), Role::Trim, "{f}");
            assert_eq!(r(f).slot(false), SlotUse::Keep, "{f}");
        }
        for role in [Role::Bellows, Role::Interior, Role::Driver] {
            assert_eq!(role.slot(false), SlotUse::Skip);
        }
    }

    #[test]
    fn a_wheel_and_a_window_mark_their_texels() {
        let mut g = box_bus();
        // the right side's two triangles: one a window, one a wheel
        g.tris[2].glass = true;
        g.tris[3].keep = true;
        let out = Outside::build(&g.tris);
        let b = Bake::build(&g.tris, 0, 64, 32, 0, 32, &out);
        let at = |x: usize, y: usize| b.first[y * 64 + x].flags;
        assert!(at(60, 28) & GLASS != 0 && at(60, 28) & KEEP == 0);
        assert!(at(34, 3) & KEEP != 0 && at(34, 3) & GLASS == 0);
        assert!(at(10, 10) & (GLASS | KEEP) == 0, "the left side is paint");
        // the wheel's and the window's texels do not count in the colour zones' paint
        let base = vec![128u8; 64 * 32 * 4];
        let colours = super::super::paint::outside_colours(&b, &base);
        assert!(colours.len() < 64 * 32 / 2 + 8, "{}", colours.len());
    }

    #[test]
    fn the_shades_of_a_paint_zone_are_paint() {
        // a white bus whose lower side is dirtier, and black rubbers
        let mut cs = vec![[235u8, 235, 235]; 700];
        cs.extend(vec![[200u8, 200, 196]; 60]);
        cs.extend(vec![[25u8, 25, 25]; 60]);
        let z = Zones::of(&cs);
        assert!(z.centres.len() >= 2, "{z:?}");
        assert_eq!(z.paint(colour::lab([235, 235, 235])), 1.0);
        assert!(z.paint(colour::lab([200, 200, 196])) > 0.9, "the dirt is paint: no holes in a band");
        assert_eq!(z.paint(colour::lab([25, 25, 25])), 0.0, "the rubbers keep their black");
    }

    /// A body colour (yellow, most of it), a black trim (the window surrounds, a fifth) and a
    /// grey part (a bumper, an eighth).
    fn yellow_black_grey() -> Vec<[u8; 3]> {
        let mut cs = vec![[240u8, 200, 20]; 650];
        cs.extend(vec![[200u8, 166, 16]; 60]);
        cs.extend(vec![[22u8, 22, 24]; 200]);
        cs.extend(vec![[120u8, 120, 118]; 125]);
        cs
    }

    #[test]
    fn the_zones_tell_the_body_from_its_black_and_grey_trims() {
        let z = Zones::of(&yellow_black_grey());
        let at = |c: [u8; 3]| (0..z.centres.len()).min_by(|a, b| colour::delta_e(z.centres[*a], colour::lab(c)).total_cmp(&colour::delta_e(z.centres[*b], colour::lab(c)))).unwrap();
        let (y, k, g) = (at([240, 200, 20]), at([22, 22, 24]), at([120, 120, 118]));
        assert!(y != k && k != g && y != g, "{z:?}");
        assert_eq!(z.group[y], 0, "the yellow is the body's colour: {z:?}");
        assert_eq!(z.group[at([200, 166, 16])], 0, "its shade too");
        assert!(z.share[k] >= 0.1 && !z.painted[k], "the black is a trim though large: {z:?}");
        assert!(z.share[g] >= 0.1 && !z.painted[g], "the grey too: {z:?}");
        // a black bus (no other colour on a tenth of it) is black paint
        let mut black = vec![[20u8, 20, 22]; 800];
        black.extend(vec![[230u8, 230, 230]; 40]);
        black.extend(vec![[90u8, 90, 90]; 100]);
        let z = Zones::of(&black);
        assert_eq!(z.group[0], 0, "{z:?}");
        assert!(trim_like(z.centres[0]));
    }

    #[test]
    fn another_texture_follows_the_main_one_s_paint() {
        // the body texture is yellow and black; a parts atlas has a little yellow, much grey
        let mut atlas = vec![[110u8, 110, 112]; 500];
        atlas.extend(vec![[40u8, 40, 40]; 300]);
        atlas.extend(vec![[238u8, 199, 22]; 40]);
        let zs = Zones::of_targets(&[atlas, yellow_black_grey().repeat(2)]);
        let a = &zs[0];
        assert!(!a.painted[0], "the atlas's largest zone, grey, is no paint: {a:?}");
        let y = a.centres.iter().position(|c| colour::delta_e(*c, colour::lab([238, 199, 22])) < 8.0).expect("a yellow zone");
        assert_eq!(a.group[y], 0, "its yellow parts are the body's: {a:?}");
    }

    #[test]
    fn a_texel_unmixes_into_its_zones() {
        let z = Zones::of(&yellow_black_grey());
        let at = |c: [u8; 3]| (0..z.centres.len()).min_by(|a, b| colour::delta_e(z.centres[*a], colour::lab(c)).total_cmp(&colour::delta_e(z.centres[*b], colour::lab(c)))).unwrap() as u8;
        let lin = |c: [u8; 3]| [colour::to_linear(c[0]), colour::to_linear(c[1]), colour::to_linear(c[2])];
        let un = |c: [f32; 3]| z.unmix(c, colour::lab_of_linear(c));
        let (y, k) = (z.lin[at([240, 200, 20]) as usize], z.lin[at([22, 22, 24]) as usize]);
        // the body alone, the body in shadow (mixed with black), the black trim alone
        let (a, _, t) = un(y);
        assert_eq!(a, at([240, 200, 20]));
        assert!((t - 1.0).abs() < 0.02, "{t}");
        let (a, b, t) = un(y.map(|v| v * 0.6));
        assert!(z.group[a as usize] == 0 && b == BLACK, "{a} {b} {t}");
        assert!((t * z.lin[a as usize][0] - 0.6 * y[0]).abs() < 0.02, "{t}");
        let (a, b, t) = un(lin([22, 22, 24]));
        assert!(a == at([22, 22, 24]) && t > 0.9 || b == at([22, 22, 24]) && t < 0.1, "{a} {b} {t}");
        // the edge between the body and the black: half of each
        let edge: [f32; 3] = std::array::from_fn(|i| y[i] * 0.5 + k[i] * 0.5);
        // (or the body half in shadow, which is the same colour)
        let (a, b, t) = un(edge);
        let paint = |i: u8| if i != BLACK && z.group[i as usize] == 0 { z.lin[i as usize][1] } else { 0.0 };
        let part = t.min(1.0) * paint(a) + (1.0 - t.min(1.0)) * paint(b);
        assert!((part - 0.5 * y[1]).abs() < 0.05, "{a} {b} {t}");
    }
}
