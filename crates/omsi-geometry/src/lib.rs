//! CPU geometry: spline curves, spline surface extrusion, terrain meshes, placement.
//!
//! World frame: x east, y north, z up, metres. Headings in degrees, 0 = +y, clockwise.

use glam::{DVec2, DVec3, Mat4, Quat, Vec2, Vec3};
use omsi_map::{tile_size, MapSpline, Terrain};
use omsi_scenery::Spline;

/// A renderable triangle mesh with one texture per material slot.
#[derive(Debug, Clone, Default)]
pub struct MeshData {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
    /// (first index, index count, material slot)
    pub ranges: Vec<(u32, u32, u32)>,
    pub indices: Vec<u32>,
    /// A mesh from a content file (o3d / .x): drawn like Direct3D draws it, only from the
    /// side its triangles face (clockwise in the file's frame). Generated geometry
    /// (terrain, splines, tree cards) leaves this off and is seen from both sides.
    pub one_sided: bool,
}

impl MeshData {
    /// Join static meshes with the same ordered material slots and winding. Each range
    /// keeps its place in the material order, but draws all segments in one call.
    /// Positions must already be expressed in the same coordinate frame.
    pub fn merge_static(meshes: &[&MeshData]) -> MeshData {
        let Some(first) = meshes.first() else { return MeshData::default() };
        let mut out = MeshData { one_sided: first.one_sided, ..Default::default() };
        let mut ranges = vec![Vec::new(); first.ranges.len()];
        for mesh in meshes {
            assert_eq!(mesh.one_sided, first.one_sided);
            assert_eq!(mesh.ranges.len(), first.ranges.len());
            let base = out.positions.len() as u32;
            out.positions.extend_from_slice(&mesh.positions);
            out.normals.extend_from_slice(&mesh.normals);
            out.uvs.extend_from_slice(&mesh.uvs);
            for (i, &(start, count, slot)) in mesh.ranges.iter().enumerate() {
                assert_eq!(slot, first.ranges[i].2);
                ranges[i].extend(mesh.indices[start as usize..(start + count) as usize].iter().map(|v| base + v));
            }
        }
        for (indices, &(_, _, slot)) in ranges.into_iter().zip(&first.ranges) {
            let start = out.indices.len() as u32;
            let count = indices.len() as u32;
            out.indices.extend(indices);
            // Adjacent profiles using the same slot have identical draw parameters.
            match out.ranges.last_mut() {
                Some((_, n, s)) if *s == slot => *n += count,
                _ => out.ranges.push((start, count, slot)),
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Bytes the mesh holds on the heap.
    pub fn heap_bytes(&self) -> usize {
        self.positions.capacity() * 12 + self.normals.capacity() * 12 + self.uvs.capacity() * 8 + self.indices.capacity() * 4 + self.ranges.capacity() * 12
    }
}

/// Analytic spline curve (one map segment).
#[derive(Debug, Clone, Copy)]
pub struct SplineCurve {
    pub start: DVec3,
    pub heading_deg: f64,
    pub length: f64,
    pub radius: f64,
    pub grad_start: f64,
    pub grad_end: f64,
    /// `[spline_h]`: the height gained over the length; the height is then a cubic between
    /// the two gradients instead of the gradient parabola of a plain `[spline]`.
    pub delta_h: Option<f64>,
    /// Cant in percent: the rise across per metre.
    pub cant_start: f64,
    pub cant_end: f64,
    /// The map's skew at the start and the end: a cross-section at lateral `x` is shifted
    /// along the spline by `x * skew`, the skew running linearly from start to end
    /// (Omsi.exe sub_5aaef4). Two splines meeting at a corner mitre there (a fence turning
    /// 90 degrees ends at skew 1 and the next starts at -1).
    pub skew_start: f64,
    pub skew_end: f64,
    /// Where the textures start along the spline (the map's [`MapSpline::tex_offset`]).
    pub tex_offset: f64,
    /// The spline's IDCode: the seed of its `[patchwork_chain]` order.
    pub seed: u32,
    /// How far out from the centre line the cant lifts (`[halfcantwidth]` of the .sli;
    /// beyond it the profile keeps the height it has there). See [`half_cant_width`].
    pub half_cant_width: f64,
}

/// The half cant width OMSI uses for a spline type: the .sli's `[halfcantwidth]`, else as
/// far as its `[heightprofile]`s reach either side (Omsi.exe 0x5adb3c: the width starts at
/// -1 and becomes the larger of -min(0, x0) and max(0, x1) over the height profiles, 0x5ac727
/// / 0x5ac78d) - not the drawn profile. A type without height profiles (the grass and
/// pavement pieces of a crossing kit) has none at all, and a cant the map gives it does
/// nothing: taken over the drawn width, Westcountry's crossings' `-70` tipped their pieces
/// metres into the ground and the sky, pointed spikes at every corner.
pub fn half_cant_width(def: &Spline) -> f64 {
    if let Some(w) = def.half_cant_width.filter(|w| w.is_finite() && *w >= 0.0) {
        return w as f64;
    }
    let (lo, hi) = def.height_profiles.iter().fold((0.0f64, 0.0f64), |(lo, hi), h| (lo.min(h.x0 as f64), hi.max(h.x1 as f64)));
    (-lo).max(hi).max(-1.0)
}

/// The half cant width of a spline without a .sli object (OMSI: 10 m).
pub const DEFAULT_HALF_CANT_WIDTH: f64 = 10.0;

static HALF_CANT_WIDTHS: std::sync::OnceLock<std::sync::RwLock<std::collections::HashMap<String, f64>>> = std::sync::OnceLock::new();

/// Remember a spline type's half cant width by its file name (as a tile names it), for
/// what is placed on its splines without the type at hand (objects in rows).
pub fn register_half_cant_width(file: &str, def: &Spline) {
    let key = file.trim().to_ascii_lowercase().replace('\\', "/");
    HALF_CANT_WIDTHS.get_or_init(Default::default).write().unwrap_or_else(|e| e.into_inner()).insert(key, half_cant_width(def));
}

/// The half cant width of the spline type `file` names, as registered (else the default).
pub fn half_cant_width_of(file: &str) -> f64 {
    let key = file.trim().to_ascii_lowercase().replace('\\', "/");
    HALF_CANT_WIDTHS.get().and_then(|m| m.read().ok().and_then(|m| m.get(&key).copied())).unwrap_or(DEFAULT_HALF_CANT_WIDTH)
}

impl SplineCurve {
    pub fn from_map(s: &MapSpline, tile_origin: DVec2) -> SplineCurve {
        SplineCurve {
            start: DVec3::new(s.pos[0] + tile_origin.x, s.pos[1] + tile_origin.y, s.pos[2]),
            heading_deg: s.heading,
            length: s.length,
            radius: s.radius,
            grad_start: s.grad_start,
            grad_end: s.grad_end,
            delta_h: s.delta_h,
            cant_start: s.cant_start,
            cant_end: s.cant_end,
            skew_start: s.skew_start,
            skew_end: s.skew_end,
            tex_offset: s.tex_offset,
            seed: s.id as u32,
            half_cant_width: DEFAULT_HALF_CANT_WIDTH,
        }
    }

    /// The same curve with its spline type's half cant width.
    pub fn with_sli(self, def: &Spline) -> SplineCurve {
        SplineCurve { half_cant_width: half_cant_width(def), ..self }
    }

    /// Direction vector (unit, horizontal) for a heading.
    #[inline]
    pub fn dir(heading_deg: f64) -> DVec2 {
        let h = heading_deg.to_radians();
        DVec2::new(h.sin(), h.cos())
    }

    /// Heading at distance `s`.
    pub fn heading_at(&self, s: f64) -> f64 {
        if self.radius == 0.0 {
            self.heading_deg
        } else {
            let turn = self.radius.signum();
            self.heading_deg + turn * (s / self.radius.abs()).to_degrees()
        }
    }

    /// Height at distance `s`: a plain spline's gradient (percent) changes linearly along
    /// it; a `[spline_h]` runs as the cubic that leaves with the start gradient, arrives with
    /// the end gradient and climbs exactly `delta_h`.
    pub fn height_at(&self, s: f64) -> f64 {
        let l = self.length.max(1e-6);
        let t = (s / l).clamp(0.0, 1.0);
        match self.delta_h {
            Some(dh) => {
                let (m0, m1) = (self.grad_start / 100.0 * l, self.grad_end / 100.0 * l);
                let (t2, t3) = (t * t, t * t * t);
                self.start.z + (t3 - 2.0 * t2 + t) * m0 + (3.0 * t2 - 2.0 * t3) * dh + (t3 - t2) * m1
            }
            None => {
                let g = self.grad_start + (self.grad_end - self.grad_start) * t * 0.5;
                self.start.z + s * g / 100.0
            }
        }
    }

    /// Rise per metre at distance `s`.
    pub fn slope_at(&self, s: f64) -> f64 {
        let l = self.length.max(1e-6);
        let t = (s / l).clamp(0.0, 1.0);
        match self.delta_h {
            Some(dh) => {
                let (m0, m1) = (self.grad_start / 100.0 * l, self.grad_end / 100.0 * l);
                ((3.0 * t * t - 4.0 * t + 1.0) * m0 + (6.0 * t - 6.0 * t * t) * dh + (3.0 * t * t - 2.0 * t) * m1) / l
            }
            None => (self.grad_start + (self.grad_end - self.grad_start) * t) / 100.0,
        }
    }

    pub fn cant_at(&self, s: f64) -> f64 {
        let l = self.length.max(1e-6);
        let t = (s / l).clamp(0.0, 1.0);
        self.cant_start + (self.cant_end - self.cant_start) * t
    }

    /// Centre-line point at distance `s`.
    pub fn point_at(&self, s: f64) -> DVec3 {
        let xy = if self.radius == 0.0 {
            self.start.truncate() + Self::dir(self.heading_deg) * s
        } else {
            let r = self.radius.abs();
            let turn = self.radius.signum();
            let d = Self::dir(self.heading_deg);
            let right = DVec2::new(d.y, -d.x);
            let centre = self.start.truncate() + right * r * turn;
            let ang = -turn * s / r; // clockwise = negative angle in an x-east/y-north frame
            let p = self.start.truncate() - centre;
            let (sa, ca) = ang.sin_cos();
            centre + DVec2::new(p.x * ca - p.y * sa, p.x * sa + p.y * ca)
        };
        DVec3::new(xy.x, xy.y, self.height_at(s))
    }

    /// Point at distance `s` and lateral offset `x` (right positive) and height `z`.
    pub fn offset_point(&self, s: f64, x: f64, z: f64) -> DVec3 {
        let c = self.point_at(s);
        let d = Self::dir(self.heading_at(s));
        let right = DVec2::new(d.y, -d.x);
        // (the original: the cant is a percentage and lifts only within the half
        // cant width; read as degrees it banked every curve 1.75 times too steeply)
        let hcw = self.half_cant_width.max(0.0);
        let dz = -x.clamp(-hcw, hcw) * self.cant_at(s) / 100.0;
        DVec3::new(c.x + right.x * x, c.y + right.y * x, c.z + z + dz)
    }

    pub fn end_point(&self) -> DVec3 {
        self.point_at(self.length)
    }
}

/// How many cross-sections a spline is drawn with, as Omsi.exe's TSplineSegment.Generate
/// (sub_5b1e14) and the mesh builder (sub_5aee00) count them: one every half degree of
/// turn, one every half degree the gradient changes, at least one per 10 m - and never
/// more than the spline type's buffers hold (2580 vertices, 3870 indices), which is what
/// keeps a road of many profiles to a few dozen.
pub fn spline_station_count(def: &Spline, curve: &SplineCurve) -> usize {
    const HALF_DEGREE: f64 = 0.008726647;
    let l = curve.length.max(0.0);
    // (the turn: Generate's curvature times length)
    let turn = if curve.radius != 0.0 { (l / curve.radius).abs() } else { 0.0 };
    let horizontal = (turn / HALF_DEGREE) as usize + 1;
    // the height polynomial g0 s + a s^2 + b s^3 (sub_5ab108)
    let (g0, g1) = (curve.grad_start / 100.0, curve.grad_end / 100.0);
    let (a, b) = match curve.delta_h {
        Some(dh) if l > 1e-6 => ((3.0 * (dh - g0 * l) - (g1 - g0) * l) / (l * l), ((g1 - g0) * l - 2.0 * (dh - g0 * l)) / (l * l * l)),
        _ if l > 1e-6 => ((g1 - g0) / (2.0 * l), 0.0),
        _ => (0.0, 0.0),
    };
    let vertical = if a != 0.0 || b != 0.0 {
        let bend = (2.0 * a + 3.0 * b * l).abs() + (2.0 * a).abs();
        (l * bend / HALF_DEGREE).min(1e6) as usize + 1
    } else {
        1
    };
    let n = horizontal.max(vertical.max((l / 10.0) as usize));
    n.min(station_cap(def)).max(1)
}

/// The most cross-sections a spline type's mesh may have (Omsi.exe 0x5adbf0: 2580 vertices
/// and 3870 indices over all its profiles).
fn station_cap(def: &Spline) -> usize {
    let points: usize = def.profiles.iter().map(|p| p.points.len()).sum();
    let segments: usize = def.profiles.iter().map(|p| p.points.len().saturating_sub(1)).sum();
    if points == 0 || segments == 0 {
        return usize::MAX;
    }
    (2580 / points).saturating_sub(1).min(3870 / (6 * segments))
}

/// The `[patchwork_chain]` of a spline: which part of the texture each stretch shows.
struct Patchwork {
    /// The texture slot it belongs to.
    texture: usize,
    /// Parts the texture is cut into.
    parts: usize,
    /// Cross-sections per stretch.
    per: usize,
    /// Per stretch the part, from 1 (negative: laid backwards).
    order: Vec<i32>,
}

/// OMSI's patchwork order (Omsi.exe sub_5aee00): the spline is cut into
/// `trunc(length / segment_length) + 1` stretches; each takes a part whose first letter is the
/// last letter of the part before (or, if invertible, a part laid backwards), drawn by weight
/// with the map's own random numbers: seeded by the spline's IDCode plus 100 per stretch and
/// stirred `(id + stretch) mod 7 + 1` times by `x = (x * 31415926 + 1) mod 10^8`. The last
/// stretch has to end on the chain's first letter, so the next spline joins on.
/// `stations` is made a multiple of the stretches.
fn patchwork(def: &Spline, curve: &SplineCurve, stations: &mut usize) -> Option<Patchwork> {
    // (OMSI keeps one chain per spline type, the last one defined)
    let (texture, pc) = def.textures.iter().enumerate().rev().find_map(|(i, t)| t.patchwork.as_ref().filter(|p| p.valid()).map(|p| (i, p)))?;
    let chain = pc.chain.as_bytes();
    let n_parts = pc.weights.len();
    let weight = |k: usize| pc.weights.as_bytes()[k].wrapping_sub(b'0').min(9) as usize;
    let invertible = |k: usize| pc.invertable.as_bytes()[k] == b'1';
    let seg = pc.segment_length as f64;
    if !(seg > 0.0) {
        return None;
    }
    let stretches = ((curve.length / seg).max(0.0).min(1e6) as usize) + 1;
    let cap = station_cap(def);
    let mut count = (*stations).min(cap);
    let mut per = count / stretches + 1;
    count = stretches * per;
    if count > cap {
        count = cap.min(stretches).max(cap / stretches * stretches);
        per = count / stretches;
    }
    let per = per.max(1);
    *stations = stretches * per;
    let mut state = chain[0];
    let mut order = Vec::with_capacity(stretches);
    for k in 0..stretches {
        let mut cands: Vec<i32> = Vec::new();
        if k + 1 == stretches {
            for p in 0..n_parts {
                if chain[p] == state && chain[p + 1] == chain[0] {
                    cands.extend(std::iter::repeat(p as i32 + 1).take(weight(p)));
                }
            }
            for p in 0..n_parts {
                if invertible(p) && chain[p + 1] == state && chain[p] == chain[0] {
                    cands.extend(std::iter::repeat(-(p as i32 + 1)).take(weight(p)));
                }
            }
        }
        if cands.is_empty() {
            for q in -(n_parts as i32)..=n_parts as i32 {
                let p = q.unsigned_abs() as usize;
                let fits = match q.signum() {
                    -1 => invertible(p - 1) && chain[p] == state,
                    1 => chain[p - 1] == state,
                    _ => false,
                };
                if fits {
                    cands.extend(std::iter::repeat(q).take(weight(p - 1)));
                }
            }
        }
        let mut x: i64 = curve.seed as i64 + 100 * k as i64;
        for _ in 0..=((curve.seed as i64 + k as i64) % 7) {
            x = (x * 31_415_926 + 1) % 100_000_000;
        }
        let r = x as f64 / 1e8;
        let part = if cands.is_empty() {
            // (a chain that cannot go on: OMSI logs it; take any part)
            ((r * n_parts as f64) as i32 + 1).min(n_parts as i32)
        } else {
            cands[((cands.len() as f64 * r) as usize).min(cands.len() - 1)]
        };
        state = if part < 0 { chain[(-part - 1) as usize] } else { chain[part as usize] };
        order.push(part);
    }
    Some(Patchwork { texture, parts: n_parts, per, order })
}

/// The point of a spline mesh at distance `s` along it, `x` across (right positive, already
/// mirrored) and `z` up: the cross-section shifted along the spline by the skew (Omsi.exe
/// sub_5aaef4: `x * (skew_start (1 - s/L) + skew_end s/L)`), and that shifted distance.
fn skewed_point(curve: &SplineCurve, s: f64, x: f64, z: f64) -> (DVec3, f64) {
    let t = if curve.length > 0.0 { s / curve.length } else { 0.0 };
    let skew = curve.skew_start * (1.0 - t) + curve.skew_end * t;
    let s = if skew.is_finite() { s + x * skew.clamp(-32.0, 32.0) } else { s };
    (curve.offset_point(s, x, z), s)
}

/// Extrude a spline definition along a map curve into a mesh (one material slot per texture),
/// as Omsi.exe's sub_5aee00 does: [`spline_station_count`] cross-sections, each shifted by the
/// skew; `v = v_scale * (s + tex_offset)` so a chain's textures run on across its joints
/// (`[scaleTexByLength]`: from 0 to `v_scale` over the whole spline; a `[patchwork_chain]`:
/// part by part); a mirrored spline is flipped across and its v turned round.
/// Positions are relative to `origin` (f64 subtraction keeps precision on large maps).
pub fn build_spline_mesh(def: &Spline, curve: &SplineCurve, mirror: bool, origin: DVec3) -> MeshData {
    let mut mesh = MeshData::default();
    if curve.length <= 0.0 {
        return mesh;
    }
    let mut n = spline_station_count(def, curve);
    let patch = patchwork(def, curve, &mut n);
    let mirror_sign = if mirror { -1.0 } else { 1.0 };
    let curve = &curve.with_sli(def);
    let station = |i: usize| curve.length * i as f64 / n as f64;
    for profile in &def.profiles {
        if profile.points.len() < 2 {
            continue;
        }
        let first_index = mesh.indices.len() as u32;
        let base = mesh.positions.len() as u32;
        let pn = profile.points.len() as u32;
        let tex = def.textures.get(profile.texture);
        let quad = |mesh: &mut MeshData, a: u32, b: u32, c: u32, d: u32| {
            // counter-clockwise seen from above, so the computed normals point up
            // (profile points run left to right, stations forward along the curve)
            if mirror {
                mesh.indices.extend_from_slice(&[a, c, b, b, c, d]);
            } else {
                mesh.indices.extend_from_slice(&[a, b, c, b, d, c]);
            }
        };
        match patch.as_ref().filter(|p| p.texture == profile.texture) {
            Some(pw) => {
                // every stretch between two cross-sections has vertices of its own: the
                // part changes where one stretch of the chain ends and the next begins
                for i in 0..n {
                    let block = i / pw.per;
                    let part = pw.order.get(block).copied().unwrap_or(1);
                    let p0 = (part.unsigned_abs() as usize).saturating_sub(1) as f64 / pw.parts as f64;
                    let q = base + (i as u32) * 2 * pn;
                    for e in 0..2 {
                        let r = (i + e - block * pw.per) as f64;
                        let r = if part < 0 { pw.per as f64 - r } else { r };
                        let v = r / (pw.per * pw.parts) as f64 + p0;
                        for pt in &profile.points {
                            let (p, _) = skewed_point(curve, station(i + e), pt.x as f64 * mirror_sign, pt.z as f64);
                            mesh.positions.push((p - origin).as_vec3());
                            mesh.normals.push(Vec3::Z);
                            mesh.uvs.push(Vec2::new(pt.u, (v * mirror_sign) as f32));
                        }
                    }
                    for j in 0..pn - 1 {
                        quad(&mut mesh, q + j, q + j + 1, q + pn + j, q + pn + j + 1);
                    }
                }
            }
            None => {
                let by_length = tex.is_some_and(|t| t.scale_by_length);
                // (v can run to thousands on a long chain: drop whole repeats, which change
                // nothing, where the profile's points share one scale)
                let shared = profile.points.iter().all(|p| p.v_scale == profile.points[0].v_scale);
                let whole = if shared && !by_length { (profile.points[0].v_scale as f64 * curve.tex_offset * mirror_sign).floor() } else { 0.0 };
                for i in 0..=n {
                    for pt in &profile.points {
                        let (p, s) = skewed_point(curve, station(i), pt.x as f64 * mirror_sign, pt.z as f64);
                        mesh.positions.push((p - origin).as_vec3());
                        mesh.normals.push(Vec3::Z);
                        let v = if by_length {
                            i as f64 * pt.v_scale as f64 / n as f64 * mirror_sign
                        } else {
                            pt.v_scale as f64 * (s + curve.tex_offset) * mirror_sign - whole
                        };
                        mesh.uvs.push(Vec2::new(pt.u, v as f32));
                    }
                }
                for i in 0..n as u32 {
                    for j in 0..pn - 1 {
                        let a = base + i * pn + j;
                        quad(&mut mesh, a, a + 1, a + pn, a + pn + 1);
                    }
                }
            }
        }
        let count = mesh.indices.len() as u32 - first_index;
        mesh.ranges.push((first_index, count, profile.texture as u32));
    }
    compute_normals(&mut mesh);
    // Drawn from the side a profile faces only, as OMSI draws splines: the makers orient
    // every face - 7560 of 7630 level faces in the stock and two add-on maps' splines face
    // up, the other 70 are the undersides of bridges, roofs and tunnel ceilings - and a
    // fence or guard rail is two faces a couple of centimetres apart, one per side, which
    // drawn from both sides fought in the depth buffer and flickered as the camera moved.
    // (The winding is turned to the content meshes' clockwise front; the normals, made
    // above, stay as they are.)
    for t in mesh.indices.chunks_exact_mut(3) {
        t.swap(1, 2);
    }
    mesh.one_sided = true;
    mesh
}

/// The profiles of the hole a spline laid with `[spline_terrain_align]` cuts into the ground,
/// as points (x across, height, how far that end of the hole stands off the spline's end):
/// the `.sli`'s own `[terrainholeprofile]`s, or else those Omsi.exe makes from the drawn
/// profiles when it loads the spline (0x5ab908, from 0x5adc52): the profiles are strung
/// together where one begins within a centimetre of where the last ended, and each string
/// gives a trough from its left end to its right end, 3 cm in from both and 3 mm below them,
/// whose bottom lies 10 cm under the lowest point of the string and reaches half a metre
/// past both ends of the spline. (The bottom's corners are where the lines from the edges
/// through the points below them come down to that depth; with nothing below an edge, a
/// quarter of the way across. The left edge is always the first profile's first point:
/// Omsi.exe compares every other profile's start with it but then takes the first one's
/// again.)
pub fn terrain_hole_profiles(def: &Spline) -> Vec<Vec<[f32; 3]>> {
    if !def.terrain_hole_profiles.is_empty() {
        return def.terrain_hole_profiles.clone();
    }
    const JOIN: f32 = 0.01;
    let profiles: Vec<Vec<(f32, f32)>> = def.profiles.iter().map(|p| p.points.iter().map(|q| (q.x, q.z)).collect()).collect();
    let mut used = vec![false; profiles.len()];
    let mut out = Vec::new();
    while used.iter().any(|u| !u) {
        let mut chain: Vec<usize> = Vec::new();
        loop {
            let next = (0..profiles.len()).find(|&j| {
                !used[j]
                    && match chain.last() {
                        None => true,
                        Some(&l) => match (profiles[l].last(), profiles[j].first()) {
                            (Some(a), Some(b)) => (a.0 - b.0).abs() < JOIN && (a.1 - b.1).abs() < JOIN,
                            _ => false,
                        },
                    }
            });
            match next {
                Some(j) => {
                    used[j] = true;
                    chain.push(j);
                }
                None => break,
            }
        }
        let Some((&(xl, zl), _)) = profiles[chain[0]].first().zip(profiles[chain[0]].last()) else {
            continue;
        };
        let (mut xr, mut zr) = *profiles[chain[0]].last().unwrap();
        for &j in &chain[1..] {
            if let Some(&(x, z)) = profiles[j].last() {
                if x > xr {
                    (xr, zr) = (x, z);
                }
            }
        }
        let points = || chain.iter().flat_map(|&j| profiles[j].iter().copied());
        let low = points().fold(zl.min(zr), |m, (_, z)| m.min(z));
        let bottom_left = if low < zl {
            points().filter(|p| zl > p.1).fold(xr, |m, (x, z)| m.min((x - xl) / (zl - z) * (zl - low) + xl))
        } else {
            (xr - xl) / 4.0 + xl
        };
        let bottom_right = if low < zr {
            points().filter(|p| zr > p.1).fold(xl, |m, (x, z)| m.max(xr - (xr - x) / (zr - z) * (zr - low)))
        } else {
            (xr - xl) * 3.0 / 4.0 + xl
        };
        let low = low - 0.1;
        out.push(vec![[xl + 0.03, zl - 0.003, 0.0], [bottom_left, low, -0.5], [bottom_right, low, -0.5], [xr - 0.03, zr - 0.003, 0.0]]);
    }
    out
}

/// The outlines (world x, y) a spline laid with `[spline_terrain_align]` cuts out of the
/// ground, one per [`terrain_hole_profiles`] profile, as Omsi.exe's TSplineSegment.Generate
/// makes them (0x5b1178, from 0x5b13b1): along the right edge (the profile's last point)
/// at the inner cross-sections, across the far end (the profile backwards), back along the
/// left edge (its first point) and across the near end. An end lies where each point's
/// third value puts it (the hole reaching past the spline's end) unless the map's
/// `[spline_terrain_align_2]` says otherwise: `mode` 2 and 4 keep the far end at the end,
/// 3 and 4 the near end at the start. A mirrored spline takes the profile backwards and
/// turned across.
pub fn spline_hole_outlines(def: &Spline, curve: &SplineCurve, mirror: bool, mode: u8) -> Vec<Vec<DVec2>> {
    if mode == 0 || curve.length <= 0.0 {
        return Vec::new();
    }
    let mut n = spline_station_count(def, curve);
    let _ = patchwork(def, curve, &mut n);
    let n = n.max(1);
    let l = curve.length;
    let curve = &curve.with_sli(def);
    let at = |x: f32, s: f64| skewed_point(curve, s, x as f64, 0.0).0.truncate();
    terrain_hole_profiles(def)
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|p| {
            let p: Vec<[f32; 3]> = if mirror { p.iter().rev().map(|q| [-q[0], q[1], q[2]]).collect() } else { p };
            let k = p.len();
            let mut ring = Vec::with_capacity(2 * (k + n - 1));
            for i in 1..n {
                ring.push(at(p[k - 1][0], i as f64 * l / n as f64));
            }
            for q in p.iter().rev() {
                ring.push(at(q[0], if mode & 1 == 0 { l } else { l - q[2] as f64 }));
            }
            for i in (1..n).rev() {
                ring.push(at(p[0][0], i as f64 * l / n as f64));
            }
            for q in &p {
                ring.push(at(q[0], if mode > 2 { 0.0 } else { q[2] as f64 }));
            }
            ring
        })
        .collect()
}

/// Does a closed outline cross itself? Omsi.exe cuts no hole along one that does (0x5748bc
/// and the terrain's "Terrain hole cutting" error): a tight curve folds the inner edge over.
pub fn outline_crosses_itself(ring: &[DVec2]) -> bool {
    let n = ring.len();
    if n < 4 {
        return n < 3;
    }
    let cross = |a: DVec2, b: DVec2, c: DVec2| (b - a).perp_dot(c - a);
    let hits = |p1: DVec2, p2: DVec2, q1: DVec2, q2: DVec2| {
        let (d1, d2) = (cross(q1, q2, p1), cross(q1, q2, p2));
        let (d3, d4) = (cross(p1, p2, q1), cross(p1, p2, q2));
        ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    };
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        for j in i + 2..n {
            if (j + 1) % n == i {
                continue;
            }
            if hits(a, b, ring[j], ring[(j + 1) % n]) {
                return true;
            }
        }
    }
    false
}

/// The surface a spline's `[heightprofile]` segments describe, extruded along the curve like
/// the drawn profile (the same cross-sections, the same skew): what vehicles and wheels stand
/// on. OMSI keeps it apart from the graphics - a railway's third rail or a tunnel's walls are
/// drawn but not driven on, a fence or the Berlin Wall has no height profile at all, and a
/// street's lies exactly on its carriageway and pavements. Positions are relative to `origin`.
pub fn build_height_profile_mesh(def: &Spline, curve: &SplineCurve, mirror: bool, origin: DVec3) -> MeshData {
    let mut mesh = MeshData::default();
    if curve.length <= 0.0 || def.height_profiles.is_empty() {
        return mesh;
    }
    let n = spline_station_count(def, curve);
    let sign = if mirror { -1.0 } else { 1.0 };
    let curve = &curve.with_sli(def);
    // the ordinary profiles first (material 0), then the wall tops (material 1)
    let ridge = |hp: &&omsi_scenery::sli::HeightProfile| is_wall_top(hp.x0, hp.x1, hp.z0, hp.z1);
    let (flat, tops): (Vec<_>, Vec<_>) = def.height_profiles.iter().partition(|hp| !ridge(hp));
    let mut flat_end = 0u32;
    for (pass, list) in [flat, tops].into_iter().enumerate() {
        for hp in list {
            if (hp.x1 - hp.x0).abs() < 1e-3 {
                continue;
            }
            let base = mesh.positions.len() as u32;
            let (z0, z1) = match drawn_height(def, hp.x0.min(hp.x1), hp.x0.max(hp.x1)) {
                Some(d) if hp.z0.min(hp.z1) > d + PHANTOM_LIFT => (d, d),
                _ => (hp.z0, hp.z1),
            };
            for i in 0..=n {
                let s = curve.length * i as f64 / n as f64;
                for (x, z) in [(hp.x0, z0), (hp.x1, z1)] {
                    let (p, _) = skewed_point(curve, s, x as f64 * sign, z as f64);
                    mesh.positions.push((p - origin).as_vec3());
                    mesh.normals.push(Vec3::Z);
                    mesh.uvs.push(Vec2::ZERO);
                }
            }
            for i in 0..n as u32 {
                let (a, b) = (base + i * 2, base + i * 2 + 1);
                let (c, d) = (a + 2, b + 2);
                mesh.indices.extend_from_slice(&[a, b, c, b, d, c]);
            }
        }
        if pass == 0 {
            flat_end = mesh.indices.len() as u32;
        }
    }
    mesh.ranges.push((0, flat_end, 0));
    if mesh.indices.len() as u32 > flat_end {
        mesh.ranges.push((flat_end, mesh.indices.len() as u32 - flat_end, 1));
    }
    mesh
}

/// How far a height profile may lie over everything the spline draws across it before it
/// counts as a slip of its maker and is brought down to the drawn surface.
const PHANTOM_LIFT: f32 = 0.25;

/// The highest point the spline's drawn profiles reach between `xa` and `xb` (none: nothing
/// is drawn there - an invisible footway, which keeps its height profile as it is).
/// Westcountry's yellow surface marking draws its paint 10 cm up and says 50 cm in its
/// `[heightprofile]`: every wheel met it as a 40 cm wall across the carriageway.
fn drawn_height(def: &Spline, xa: f32, xb: f32) -> Option<f32> {
    let mut best: Option<f32> = None;
    for p in &def.profiles {
        for w in p.points.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            let (lo, hi) = (a.x.min(b.x), a.x.max(b.x));
            if hi < xa - 0.05 || lo > xb + 0.05 {
                continue;
            }
            // (a vertical face, a kerb's edge, counts with its top)
            let z = a.z.max(b.z);
            best = Some(best.map_or(z, |m: f32| m.max(z)));
        }
    }
    best
}

/// A height profile that is the top of a wall: a strip narrower than a wheel could stand on
/// (under 0.6 m), 0.3 m and more over the spline. UK maps give their stone and brick walls
/// one (Westcountry's `cb_wall03`: 30 cm wide, 1.6 m up); taken as road, a wheel rolled
/// onto the wall where its top met the road and rode along it as the road fell away.
pub fn is_wall_top(x0: f32, x1: f32, z0: f32, z1: f32) -> bool {
    let w = (x1 - x0).abs();
    w >= 1e-3 && w < 0.6 && z0.min(z1) >= 0.3
}

/// Recompute smooth vertex normals from triangles.
pub fn compute_normals(mesh: &mut MeshData) {
    let mut acc = vec![Vec3::ZERO; mesh.positions.len()];
    for tri in mesh.indices.chunks_exact(3) {
        let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let n = (mesh.positions[b] - mesh.positions[a]).cross(mesh.positions[c] - mesh.positions[a]);
        acc[a] += n;
        acc[b] += n;
        acc[c] += n;
    }
    mesh.normals = acc.into_iter().map(|n| if n.length_squared() > 0.0 { n.normalize() } else { Vec3::Z }).collect();
}

/// Terrain mesh for one tile in tile-local coordinates (0..300). UVs are tile space (0..1);
/// the material scales them for the ground texture.
pub fn build_terrain_mesh(t: &Terrain) -> MeshData {
    let n = t.samples();
    let cell = tile_size() as f32 / t.cells as f32;
    let mut mesh = MeshData::default();
    mesh.positions.reserve(n * n);
    for j in 0..n {
        for i in 0..n {
            let x = i as f32 * cell;
            let y = j as f32 * cell;
            mesh.positions.push(Vec3::new(x, y, t.height_at(i, j)));
            mesh.uvs.push(Vec2::new(x / tile_size() as f32, y / tile_size() as f32));
            mesh.normals.push(Vec3::Z);
        }
    }
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            let a = (j * n + i) as u32;
            let b = a + 1;
            let c = a + n as u32;
            let d = c + 1;
            // split along the diagonal from the cell's (i, j) corner to its (i+1, j+1)
            // corner, as OMSI does (its terrain vertex cache, `.map.terrain_0.rdy`,
            // lists every cell as (i,j)-(i,j+1)-(i+1,j+1) and (i,j)-(i+1,j+1)-(i+1,j)).
            // The maps' ground is shaped against those triangles: split the other way,
            // wherever the ground steps down beside a kerb half a cell of it stood up
            // over the road as a V-shaped wedge.
            mesh.indices.extend_from_slice(&[a, d, c, a, b, d]);
        }
    }
    mesh.ranges.push((0, mesh.indices.len() as u32, 0));
    compute_normals(&mut mesh);
    mesh
}

/// Height of the terrain *as drawn* at tile-local metres: the two triangles of each cell
/// that [`build_terrain_mesh`] makes (diagonal from the cell's (i, j) corner to its
/// (i+1, j+1) corner, as OMSI splits it), not the bilinear patch `Terrain::sample`
/// interpolates. On a curved slope the
/// two differ by centimetres, and a wheel standing on the bilinear one sank into the grass
/// or floated over it.
pub fn terrain_height(t: &Terrain, x: f32, y: f32) -> f32 {
    t.sample(x, y)
}

/// Rotation of a map object: heading (Z), pitch (X), bank (Y). The position is kept apart
/// as the instance origin (f64). Pitched first, then banked, then turned, as Omsi.exe puts
/// an object down (sub_79cb18: RotationX(pitch) · RotationZ(bank) · RotationY(heading)) - a
/// rock both pitched and banked a good deal leant otherwise with the bank taken first.
pub fn object_rotation(rot_deg: [f64; 3]) -> Mat4 {
    let heading = (-rot_deg[0]).to_radians() as f32; // clockwise heading → counter-clockwise rotation about +z
    let pitch = rot_deg[1].to_radians() as f32;
    let bank = rot_deg[2].to_radians() as f32;
    let q = Quat::from_rotation_z(heading) * Quat::from_rotation_y(bank) * Quat::from_rotation_x(pitch);
    Mat4::from_quat(q)
}

/// The same angles in the order D3DX's `QuaternionRotationYawPitchRoll` applies them (the
/// bank first, then the pitch, then the heading), which is how Omsi.exe turns an attached
/// object by its own angles (0x79d547).
pub fn object_rotation_ypr(rot_deg: [f64; 3]) -> Mat4 {
    let heading = (-rot_deg[0]).to_radians() as f32;
    let pitch = rot_deg[1].to_radians() as f32;
    let bank = rot_deg[2].to_radians() as f32;
    Mat4::from_quat(Quat::from_rotation_z(heading) * Quat::from_rotation_x(pitch) * Quat::from_rotation_y(bank))
}

/// A map file's heading, pitch and bank (`[object]`, `[attachObj]`, `[splineAttachement]`)
/// in the terms [`object_rotation`] takes. The file keeps the angles of Direct3D's
/// left-handed frame, where a positive pitch lowers the nose and a positive bank raises the
/// right side; going over to the right-handed world turns every sense of rotation round.
/// `object_rotation` already turns the heading, and pitch and bank have to turn as well: taken
/// as they stand, TH_Wald's rocks tipped the other way and stood as boxes with a grass lid
/// beside the road instead of a rock face. (Pitch and bank that openOMSI works out itself -
/// a parked car on a slope, an object tilted with its spline - are world angles already.)
pub fn map_rotation(rot_deg: [f64; 3]) -> [f64; 3] {
    [rot_deg[0], -rot_deg[1], -rot_deg[2]]
}

/// Convert an `.o3d`/`.x` mesh to [`MeshData`] (one range per material).
pub fn mesh_from_o3d(m: &omsi_o3d::Mesh) -> MeshData {
    mesh_from_o3d_turning(m, true)
}

pub fn mesh_from_o3d_turning(m: &omsi_o3d::Mesh, may_turn: bool) -> MeshData {
    // Vertices are stored in the parent (object/vehicle) frame already; the matrix in the
    // file is the mesh's pivot frame used by `origin_from_mesh` animations, not a transform
    // to apply. Mesh files use Direct3D's frame (x right, y up, z forward); the world uses
    // x right, y forward, z up, so y and z are swapped.
    let swap = |v: Vec3| Vec3::new(v.x, v.z, v.y);
    let mut out = MeshData { one_sided: true, ..Default::default() };
    out.positions = m.vertices.iter().map(|v| swap(v.position)).collect();
    out.normals = m.vertices.iter().map(|v| swap(v.normal).normalize_or_zero()).collect();
    out.uvs = m.vertices.iter().map(|v| v.uv).collect();
    let turn = may_turn && turns_round(m);
    // group triangles by material, preserving material index as slot
    let mat_count = m.materials.len().max(1);
    let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); mat_count];
    for t in &m.triangles {
        let slot = (t.material as usize).min(mat_count - 1);
        if turn {
            buckets[slot].extend_from_slice(&[t.indices[0], t.indices[2], t.indices[1]]);
        } else {
            buckets[slot].extend_from_slice(&t.indices);
        }
    }
    for (slot, idx) in buckets.into_iter().enumerate() {
        if idx.is_empty() {
            continue;
        }
        let first = out.indices.len() as u32;
        out.indices.extend_from_slice(&idx);
        out.ranges.push((first, idx.len() as u32, slot as u32));
    }
    out
}


pub fn turns_round(m: &omsi_o3d::Mesh) -> bool {
    // A mesh whose faces all turn their backs on their own normals was mirrored in the
    // modeller (the winding flips, the normals are recomputed): drawn one-sided as it stands,
    // the front shows nothing - the LiAZ 5292's right mirror housing and two dashboard
    // screens were holes. Such a mesh is turned round to face where its normals face. (Only
    // a mesh that is backwards nearly throughout: smoothed normals disagree with a few faces
    // of any ordinary mesh, and two-sided parts are two faces facing apart.)
    // The normals turned by the file's matrix are counted as well: an exporter that left
    // them in the object's own frame (the Solaris Urbino's lamps and dashboard screens: a
    // half turn about x) wrote faces that are wound the right way round.
    let rot = glam::Mat3::from_mat4(m.transform.transpose());
    let (mut against, mut against_turned, mut counted) = (0usize, 0usize, 0usize);
    for t in &m.triangles {
        let v = t.indices.map(|i| &m.vertices[i as usize]);
        let g = (v[1].position - v[0].position).cross(v[2].position - v[0].position);
        let n = v[0].normal + v[1].normal + v[2].normal;
        if g.length_squared() > 1e-12 && n.length_squared() > 1e-12 {
            counted += 1;
            if g.dot(n) < 0.0 {
                against += 1;
            }
            if g.dot(rot * n) < 0.0 {
                against_turned += 1;
            }
        }
    }
    // …but only when the file's own matrix says the object was mirrored (a positive
    // determinant where the exporter writes -1 for an ordinary object: the LiAZ's right
    // mirror housing, the Solaris' display planes). OMSI culls by winding alone
    // (D3DCULL_CCW, which only its stencil-shadow pass changes, the original), so a mesh
    // with an ordinary matrix is drawn as wound, whatever its normals say: the Procity's
    // dashboard screens face the driver by their winding and their normals away, and
    // turning them round hid the pressure and trip displays.
    // Turned round, the Urbino's headlamps faced into the bus and the body showed through
    // the holes in their place.
    let mirrored = m.transform.determinant() > 0.0;
    let explained = against_turned * 10 <= counted;
    mirrored && !explained && counted >= 2 && against * 10 >= counted * 9
}

pub fn positive_det_faces_forward(m: &omsi_o3d::Mesh) -> Option<bool> {
    if m.transform.determinant() <= 0.0 {
        return None;
    }
    let (mut against, mut counted) = (0usize, 0usize);
    for t in &m.triangles {
        let v = t.indices.map(|i| &m.vertices[i as usize]);
        let g = (v[1].position - v[0].position).cross(v[2].position - v[0].position);
        let n = v[0].normal + v[1].normal + v[2].normal;
        if g.length_squared() > 1e-12 && n.length_squared() > 1e-12 {
            counted += 1;
            if g.dot(n) < 0.0 {
                against += 1;
            }
        }
    }
    match counted {
        0..=1 => None,
        _ if against * 10 <= counted => Some(true),
        _ if against * 10 >= counted * 9 => Some(false),
        _ => None,
    }
}

pub fn reverse_winding(data: &mut MeshData) {
    for t in data.indices.chunks_exact_mut(3) {
        t.swap(1, 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_mesh_merge_preserves_geometry_uvs_and_material_order() {
        let a = MeshData {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z],
            normals: vec![Vec3::Z; 4],
            uvs: vec![Vec2::ZERO, Vec2::X, Vec2::Y, Vec2::ONE],
            indices: vec![0, 1, 2, 1, 2, 3, 0, 2, 3],
            ranges: vec![(0, 3, 0), (3, 3, 1), (6, 3, 0)],
            one_sided: true,
        };
        let mut b = a.clone();
        for p in &mut b.positions { *p += Vec3::splat(20.0); }
        let merged = MeshData::merge_static(&[&a, &b]);
        assert_eq!(merged.positions, [a.positions.clone(), b.positions.clone()].concat());
        assert_eq!(merged.normals, [a.normals.clone(), b.normals.clone()].concat());
        assert_eq!(merged.uvs, [a.uvs.clone(), b.uvs.clone()].concat());
        assert_eq!(merged.indices, vec![0, 1, 2, 4, 5, 6, 1, 2, 3, 5, 6, 7, 0, 2, 3, 4, 6, 7]);
        assert_eq!(merged.ranges, vec![(0, 6, 0), (6, 6, 1), (12, 6, 0)]);
        assert!(merged.one_sided);
        // Same-slot adjacent profiles need only one draw and keep their triangle order.
        b.ranges = vec![(0, 3, 0), (3, 3, 0), (6, 3, 1)];
        let merged = MeshData::merge_static(&[&b, &b]);
        assert_eq!(merged.ranges, vec![(0, 12, 0), (12, 6, 1)]);
        assert_eq!(merged.indices.len(), b.indices.len() * 2);
    }

    /// A map object turned, pitched and banked a good deal at once stands as Omsi.exe puts
    /// it: `v · RotationX(pitch) · RotationZ(bank) · RotationY(heading)` in Direct3D's
    /// left-handed frame (x right, y up, z forward), read in the world's (x, z, y).
    #[test]
    fn map_angles_compose_as_omsi_does() {
        let d3d = |v: [f32; 3], h: f32, p: f32, b: f32| -> Vec3 {
            let (h, p, b) = (h.to_radians(), p.to_radians(), b.to_radians());
            let rx = |v: [f32; 3]| [v[0], v[1] * p.cos() - v[2] * p.sin(), v[1] * p.sin() + v[2] * p.cos()];
            let rz = |v: [f32; 3]| [v[0] * b.cos() - v[1] * b.sin(), v[0] * b.sin() + v[1] * b.cos(), v[2]];
            let ry = |v: [f32; 3]| [v[0] * h.cos() + v[2] * h.sin(), v[1], -v[0] * h.sin() + v[2] * h.cos()];
            let w = ry(rz(rx(v)));
            Vec3::new(w[0], w[2], w[1])
        };
        let (h, p, b) = (40.0, 35.0, -50.0);
        let r = object_rotation(map_rotation([h as f64, p as f64, b as f64]));
        for (lh, rh) in [([0.0, 0.0, 1.0], Vec3::Y), ([1.0, 0.0, 0.0], Vec3::X), ([0.0, 1.0, 0.0], Vec3::Z)] {
            let want = d3d(lh, h, p, b);
            let got = r.transform_vector3(rh);
            assert!((want - got).length() < 1e-4, "{lh:?}: {got:?} != {want:?}");
        }
    }

    /// A map object pitched and banked by the file's positive angles: the nose goes down
    /// and the right side up, as in Direct3D's frame the file keeps them in.
    #[test]
    fn map_pitch_lowers_the_nose_and_bank_raises_the_right() {
        let r = object_rotation(map_rotation([0.0, 30.0, 0.0]));
        let forward = r.transform_vector3(Vec3::Y);
        assert!(forward.z < -0.4, "{forward:?}");
        let r = object_rotation(map_rotation([0.0, 0.0, 30.0]));
        let up = r.transform_vector3(Vec3::Z);
        assert!(up.x < -0.4, "{up:?}");
        // the heading stays as the file has it (clockwise from north)
        let r = object_rotation(map_rotation([90.0, 0.0, 0.0]));
        assert!(r.transform_vector3(Vec3::Y).x > 0.99);
    }

    /// A kerb: road at 0, pavement at 0.15 from x = 10 on, a bridge deck at 6 m over it all.
    #[test]
    fn drive_grid_probes_the_face_under_the_axle() {
        let mut g = DriveGrid::default();
        let quad = |g: &mut DriveGrid, x0: f32, x1: f32, z: f32| {
            g.push([Vec3::new(x0, 0.0, z), Vec3::new(x1, 0.0, z), Vec3::new(x0, 20.0, z)]);
            g.push([Vec3::new(x1, 0.0, z), Vec3::new(x1, 20.0, z), Vec3::new(x0, 20.0, z)]);
        };
        quad(&mut g, 0.0, 10.0, 0.0);
        quad(&mut g, 10.0, 20.0, 0.15);
        quad(&mut g, 0.0, 20.0, 6.0);
        // the kerb face itself is a wall and never a place to stand
        g.push([Vec3::new(10.0, 0.0, 0.0), Vec3::new(10.0, 20.0, 0.0), Vec3::new(10.0, 0.0, 0.15)]);
        g.build(300.0);
        assert_eq!(g.tris.len(), 6);
        let p = g.probe(5.0, 5.0, 0.5);
        assert_eq!(p, Probe { below: Some(0.0), above: Some(6.0) });
        let p = g.probe(12.0, 5.0, 0.5);
        assert_eq!(p.below, Some(0.15));
        // a probe that starts under the pavement's top sees it as a step above
        let p = g.probe(12.0, 5.0, 0.1);
        assert_eq!(p, Probe { below: None, above: Some(0.15) });
        // on the shared edge of two triangles
        assert_eq!(g.probe(5.0, 10.0, 0.5).below, Some(0.0));
        assert_eq!(g.probe(250.0, 250.0, 0.5), Probe::default());
    }

    #[test]
    fn surface_raster_keeps_its_layers() {
        let mut ts = TileSurface::new(512);
        let cell = tile_size() as f32 / 512.0;
        let quad = |z: f32| {
            let mut m = MeshData::default();
            for (x, y) in [(10.0f32, 10.0f32), (30.0, 10.0), (30.0, 30.0), (10.0, 30.0)] {
                m.positions.push(Vec3::new(x, y, z));
            }
            m.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
            m
        };
        // a plate at 2 m, a road at 1 m under part of it, a hole cutter elsewhere
        ts.rasterize_kind(&quad(2.0), &Mat4::IDENTITY, DVec3::ZERO, 0, 0, false);
        ts.rasterize_kind(&quad(1.0), &Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0)), DVec3::ZERO, 0, 0, true);
        ts.rasterize_hole(&quad(0.5), &Mat4::from_translation(Vec3::new(100.0, 100.0, 0.0)), DVec3::ZERO, 0, 0);
        ts.finish();
        let k = |x: f32, y: f32| ts.texel(x, y);
        assert_eq!(ts.sample(12.0, 20.0), Some(2.0));
        assert_eq!(ts.sample_road(12.0, 20.0), None);
        assert_eq!(ts.sample(25.0, 20.0), Some(2.0));
        assert_eq!(ts.sample_road(25.0, 20.0), Some(1.0));
        assert_eq!(ts.low_height(k(25.0, 20.0)), 1.0);
        assert_eq!(ts.low_height(k(12.0, 20.0)), 2.0);
        assert_eq!(ts.sample(33.0, 20.0), Some(1.0));
        assert_eq!(ts.sample(200.0, 200.0), None);
        assert_eq!(ts.hole_height(k(115.0, 115.0)), Some(0.5));
        assert_eq!(ts.hole_height(k(12.0, 20.0)), None);
        assert!(ts.cut_at(115.0, 115.0, 0.0, 0.12));
        assert!(!ts.cut_at(200.0, 200.0, 0.0, 0.12));
        // a surface alone takes no ground away (only the map's holes do, as in Omsi.exe)
        // unless OMSI_ROAD_CUT asks for it
        assert_eq!(ts.cut_at(12.0, 20.0, 1.95, 0.12), road_cut());
        assert!(!ts.cut_at(12.0, 20.0, 1.5, 0.12));
        // an aligned spline's outline cuts exactly along it, whatever the heights
        ts.add_outline(&[DVec2::new(50.0, 50.0), DVec2::new(60.0, 50.0), DVec2::new(60.0, 51.0), DVec2::new(50.0, 51.0)], 0, 0);
        assert!(ts.cut_at(55.0, 50.5, 40.0, 0.12));
        assert!(!ts.cut_at(55.0, 51.3, 0.0, 0.12));
        let mask = ts.mask_image(&|_, _| 0.0, 0.12);
        let n = ts.size;
        let a = |x: f32, y: f32| mask[((y / tile_size() as f32 * n as f32) as usize * n + (x / tile_size() as f32 * n as f32) as usize) * 4 + 3];
        assert!(a(55.0, 50.5) < 128 && a(55.0, 53.0) == 255);
        // only the touched blocks hold memory: a few kilobytes, not the 5 MB of a dense raster
        assert!(ts.heap_bytes().0 < 200_000, "{}", ts.heap_bytes().0);
        let _ = cell;
    }

    #[test]
    fn height_profile_mesh_follows_the_segments() {
        let mut def = Spline::default();
        def.height_profiles.push(omsi_scenery::sli::HeightProfile { x0: -4.5, x1: 4.5, z0: 0.1, z1: 0.1 });
        def.height_profiles.push(omsi_scenery::sli::HeightProfile { x0: 4.5, x1: 7.5, z0: 0.25, z1: 0.25 });
        def.height_profiles.push(omsi_scenery::sli::HeightProfile { x0: -1.25, x1: -1.25, z0: 0.25, z1: 0.25 });
        let c = SplineCurve { start: DVec3::new(10.0, 10.0, 30.0), heading_deg: 0.0, length: 20.0, radius: 0.0, grad_start: 0.0, grad_end: 0.0, delta_h: None, cant_start: 0.0, cant_end: 0.0, skew_start: 0.0, skew_end: 0.0, tex_offset: 0.0, seed: 0, half_cant_width: DEFAULT_HALF_CANT_WIDTH };
        let m = build_height_profile_mesh(&def, &c, false, DVec3::ZERO);
        let mut g = DriveGrid::default();
        for t in m.indices.chunks_exact(3) {
            g.push([m.positions[t[0] as usize], m.positions[t[1] as usize], m.positions[t[2] as usize]]);
        }
        g.build(300.0);
        assert_eq!(g.tris.len(), 8, "the zero-width segment is no surface (two cross-sections every 10 m)");
        assert!((g.probe(10.0, 15.0, 31.0).below.unwrap() - 30.1).abs() < 1e-4);
        assert!((g.probe(16.0, 15.0, 31.0).below.unwrap() - 30.25).abs() < 1e-4);
        // mirrored, the pavement is on the left
        let m = build_height_profile_mesh(&def, &c, true, DVec3::ZERO);
        let lo = m.positions.iter().map(|p| p.x).fold(f32::MAX, f32::min);
        assert!((lo - 2.5).abs() < 1e-4, "{lo}");
    }

    #[test]
    fn terrain_height_follows_the_drawn_triangles() {
        // one cell raised at its lower right corner only: the drawn surface is a flat
        // triangle over half the cell, the bilinear patch is not
        let mut t = Terrain::flat();
        let n = t.samples();
        t.heights.iter_mut().for_each(|h| *h = 0.0);
        t.heights[1] = 1.0; // (ix 1, iy 0)
        let cell = tile_size() as f32 / t.cells as f32;
        let _ = n;
        // the diagonal runs from (0,0) to (1,1): on it the raised corner is not felt
        let h = terrain_height(&t, cell * 0.5, cell * 0.5);
        assert!(h.abs() < 1e-4, "{h}");
        // in the lower triangle, (0,0)-(1,0)-(1,1), the corner pulls linearly
        let h = terrain_height(&t, cell * 0.75, cell * 0.25);
        assert!((h - 0.5).abs() < 1e-4, "{h}");
        // the upper triangle, (0,0)-(1,1)-(0,1), does not reach it
        let h = terrain_height(&t, cell * 0.25, cell * 0.75);
        assert!(h.abs() < 1e-4, "{h}");
        // bilinear would give 0.5625 at (0.75, 0.25)
        assert!((t.sample_bilinear(cell * 0.75, cell * 0.25) - 0.5625).abs() < 1e-4);
    }

    #[test]
    fn straight_and_arc() {
        let c = SplineCurve { start: DVec3::ZERO, heading_deg: 90.0, length: 10.0, radius: 0.0, grad_start: 0.0, grad_end: 0.0, delta_h: None, cant_start: 0.0, cant_end: 0.0, skew_start: 0.0, skew_end: 0.0, tex_offset: 0.0, seed: 0, half_cant_width: DEFAULT_HALF_CANT_WIDTH };
        let e = c.end_point();
        assert!((e.x - 10.0).abs() < 1e-9 && e.y.abs() < 1e-9);
        // quarter circle to the right from heading 0 with radius 10 ends at (10, 10), heading 90
        let c = SplineCurve { start: DVec3::ZERO, heading_deg: 0.0, length: std::f64::consts::FRAC_PI_2 * 10.0, radius: 10.0, grad_start: 0.0, grad_end: 0.0, delta_h: None, cant_start: 0.0, cant_end: 0.0, skew_start: 0.0, skew_end: 0.0, tex_offset: 0.0, seed: 0, half_cant_width: DEFAULT_HALF_CANT_WIDTH };
        let e = c.end_point();
        assert!((e.x - 10.0).abs() < 1e-9 && (e.y - 10.0).abs() < 1e-9, "{e:?}");
        assert!((c.heading_at(c.length) - 90.0).abs() < 1e-9);
    }

    fn plain_curve(length: f64, radius: f64) -> SplineCurve {
        SplineCurve { start: DVec3::ZERO, heading_deg: 0.0, length, radius, grad_start: 0.0, grad_end: 0.0, delta_h: None, cant_start: 0.0, cant_end: 0.0, skew_start: 0.0, skew_end: 0.0, tex_offset: 0.0, seed: 0, half_cant_width: DEFAULT_HALF_CANT_WIDTH }
    }

    fn strip(half: f32, v_scale: f32) -> Spline {
        let mut def = Spline::default();
        def.textures.push(Default::default());
        def.profiles.push(omsi_scenery::sli::SplineProfile {
            texture: 0,
            points: vec![
                omsi_scenery::sli::SplineProfilePoint { x: -half, u: 0.0, v_scale, ..Default::default() },
                omsi_scenery::sli::SplineProfilePoint { x: half, u: 1.0, v_scale, ..Default::default() },
            ],
        });
        def
    }

    /// Omsi.exe counts a cross-section every half degree of turn, but never more than its
    /// buffers hold: an island's nose (radius 2, a quarter turn) is round, a road of many
    /// profiles stays at a few dozen.
    #[test]
    fn stations_every_half_degree_within_the_buffer() {
        let nose = plain_curve(std::f64::consts::FRAC_PI_2 * 2.0, 2.0);
        let def = strip(2.0, 0.1);
        assert_eq!(spline_station_count(&def, &nose), 180);
        let m = build_spline_mesh(&def, &nose, false, DVec3::ZERO);
        assert_eq!(m.positions.len(), 181 * 2);
        // 30 profile segments: 3870 / 180 indices = 21 cross-sections at most
        let mut road = Spline::default();
        for k in 0..30 {
            road.profiles.push(omsi_scenery::sli::SplineProfile {
                texture: 0,
                points: vec![
                    omsi_scenery::sli::SplineProfilePoint { x: k as f32, ..Default::default() },
                    omsi_scenery::sli::SplineProfilePoint { x: k as f32 + 1.0, ..Default::default() },
                ],
            });
        }
        assert_eq!(spline_station_count(&road, &plain_curve(50.0, 30.0)), 21);
        // straight and level: one every 10 m
        assert_eq!(spline_station_count(&def, &plain_curve(95.0, 0.0)), 9);
        assert_eq!(spline_station_count(&def, &plain_curve(4.0, 0.0)), 1);
    }

    /// The skew shifts every cross-section, running from its start value to its end value.
    #[test]
    fn skew_runs_along_the_spline() {
        let def = strip(2.0, 0.0);
        let c = SplineCurve { skew_end: 1.0, ..plain_curve(20.0, 0.0) };
        let m = build_spline_mesh(&def, &c, false, DVec3::ZERO);
        // two cross-sections: at 0, 10 and 20 m; the right edge (x = 2) at the end is 2 m on
        let right_end = m.positions[5];
        assert!((right_end.y - 22.0).abs() < 1e-4 && (right_end.x - 2.0).abs() < 1e-4, "{right_end:?}");
        let left_end = m.positions[4];
        assert!((left_end.y - 18.0).abs() < 1e-4, "{left_end:?}");
        // half way the skew is a half
        assert!((m.positions[3].y - 11.0).abs() < 1e-4, "{:?}", m.positions[3]);
        // the drivable surface follows
        let mut def = def;
        def.height_profiles.push(omsi_scenery::sli::HeightProfile { x0: -2.0, x1: 2.0, z0: 0.0, z1: 0.0 });
        let hp = build_height_profile_mesh(&def, &c, false, DVec3::ZERO);
        assert!((hp.positions[5].y - 22.0).abs() < 1e-4);
    }

    /// A chain's textures run on across its joints: the next spline starts where the
    /// last one's v ended (the map's texture offset); a mirrored spline turns v round.
    #[test]
    fn textures_run_on_along_a_chain() {
        let def = strip(2.0, 0.37);
        let a = plain_curve(13.0, 0.0);
        let b = SplineCurve { start: a.end_point(), tex_offset: 13.0, ..plain_curve(9.0, 0.0) };
        let ma = build_spline_mesh(&def, &a, false, DVec3::ZERO);
        let mb = build_spline_mesh(&def, &b, false, DVec3::ZERO);
        let end_a = ma.uvs[ma.uvs.len() - 1].y;
        let start_b = mb.uvs[1].y;
        assert!((end_a - start_b).rem_euclid(1.0).min(1.0 - (end_a - start_b).rem_euclid(1.0)) < 1e-4, "{end_a} {start_b}");
        let mm = build_spline_mesh(&def, &a, true, DVec3::ZERO);
        assert!((mm.uvs[mm.uvs.len() - 1].y + end_a).abs() < 1e-5);
        // [scaleTexByLength]: 0..v_scale over the whole spline
        let mut def = def;
        def.textures[0].scale_by_length = true;
        let m = build_spline_mesh(&def, &plain_curve(55.0, 0.0), false, DVec3::ZERO);
        assert!((m.uvs[m.uvs.len() - 1].y - 0.37).abs() < 1e-6);
    }

    /// Stock `str_2spur_8m_DDR_Bahnstr.sli`: 16 parts of a 512x4096 texture, 5 m each; the
    /// parts follow on letter by letter and the spline ends on the first letter again.
    #[test]
    fn patchwork_parts_join_up() {
        let mut def = strip(3.6, 0.1);
        def.textures[0].patchwork = Some(omsi_scenery::sli::PatchworkChain {
            segment_length: 5.0,
            chain: "AAAABBAACAAADAAAA".into(),
            weights: "1110001111100155".into(),
            invertable: "0000000000000000".into(),
        });
        for seed in [1u32, 4190, 3215067, 123456789] {
            let c = SplineCurve { seed, ..plain_curve(47.3, 0.0) };
            let mut n = spline_station_count(&def, &c);
            let pw = patchwork(&def, &c, &mut n).unwrap();
            assert_eq!(pw.order.len(), 10);
            assert_eq!(n % 10, 0);
            let chain = b"AAAABBAACAAADAAAA";
            let mut state = b'A';
            for &p in &pw.order {
                assert!(p > 0 && (p as usize) <= 16);
                let k = p as usize - 1;
                assert_eq!(chain[k], state, "seed {seed}: {:?}", pw.order);
                assert!(b"1110001111100155"[k] != b'0');
                state = chain[k + 1];
            }
            assert_eq!(state, b'A');
            // the same spline always gets the same order
            let mut n2 = spline_station_count(&def, &c);
            assert_eq!(patchwork(&def, &c, &mut n2).unwrap().order, pw.order);
            // each stretch shows 1/16 of the texture
            let m = build_spline_mesh(&def, &c, false, DVec3::ZERO);
            let (v0, v1) = (m.uvs[0].y, m.uvs[2].y);
            let p = pw.order[0] as f32 - 1.0;
            assert!((v0 - p / 16.0).abs() < 1e-6 && (v1 - (p / 16.0 + 1.0 / 16.0 / pw.per as f32)).abs() < 1e-6, "{v0} {v1}");
        }
    }

    #[test]
    fn spline_h_height() {
        // the Ahlheim underpass ramp: 73.86 m, leaves at 6.19 %, arrives level, 5.85 m up
        let c = SplineCurve { start: DVec3::new(0.0, 0.0, -5.79), heading_deg: 0.0, length: 73.86, radius: 0.0, grad_start: 6.19, grad_end: 0.0, delta_h: Some(5.85), cant_start: 0.0, cant_end: 0.0, skew_start: 0.0, skew_end: 0.0, tex_offset: 0.0, seed: 0, half_cant_width: DEFAULT_HALF_CANT_WIDTH };
        assert!((c.height_at(0.0) + 5.79).abs() < 1e-9);
        assert!((c.height_at(c.length) - 0.06).abs() < 1e-9, "{}", c.height_at(c.length));
        assert!((c.slope_at(0.0) - 0.0619).abs() < 1e-9);
        assert!(c.slope_at(c.length).abs() < 1e-9);
        // the slope is the derivative of the height
        let s = 30.0;
        let d = (c.height_at(s + 1e-4) - c.height_at(s - 1e-4)) / 2e-4;
        assert!((d - c.slope_at(s)).abs() < 1e-6, "{d} {}", c.slope_at(s));
        // a plain spline keeps its gradient parabola
        let p = SplineCurve { delta_h: None, ..c };
        assert!((p.height_at(p.length) - (-5.79 + 73.86 * 0.0619 / 2.0)).abs() < 1e-9);
    }

    #[test]
    fn a_backwards_quad_with_an_unmirrored_matrix_can_keep_its_winding() {
        let v = |x: f32, y: f32| omsi_o3d::Vertex { position: Vec3::new(x, y, 1.0), normal: Vec3::new(0.0, 0.0, 1.0), uv: Vec2::ZERO };
        let o3d = omsi_o3d::Mesh { vertices: vec![v(0.0, 0.0), v(0.0, 1.0), v(1.0, 0.0), v(1.0, 1.0)], triangles: vec![omsi_o3d::Triangle { indices: [0, 1, 2], material: 0 }, omsi_o3d::Triangle { indices: [2, 1, 3], material: 0 }], materials: vec![omsi_o3d::Material::default()], transform: glam::Mat4::IDENTITY, ..Default::default() };
        assert_eq!(positive_det_faces_forward(&o3d), Some(false));
        assert!(turns_round(&o3d));
        assert_eq!(mesh_from_o3d(&o3d).indices[..3], [0, 2, 1]);
        let mut kept = mesh_from_o3d_turning(&o3d, false);
        assert_eq!(kept.indices[..3], [0, 1, 2]);
        reverse_winding(&mut kept);
        assert_eq!(kept.indices, mesh_from_o3d(&o3d).indices);
    }

    #[test]
    fn o3d_front_faces_arrive_clockwise() {
        // A triangle a viewer at the origin looking along +z sees from its front in the
        // file's Direct3D frame (x right, y up, z forward): clockwise there, the normal
        // pointing back at the viewer.
        let v = |x: f32, y: f32, z: f32| omsi_o3d::Vertex { position: Vec3::new(x, y, z), normal: Vec3::new(0.0, 0.0, -1.0), uv: Vec2::ZERO };
        let o3d = omsi_o3d::Mesh { vertices: vec![v(0.0, 0.0, 1.0), v(0.0, 1.0, 1.0), v(1.0, 0.0, 1.0)], triangles: vec![omsi_o3d::Triangle { indices: [0, 1, 2], material: 0 }], materials: vec![omsi_o3d::Material::default()], ..Default::default() };
        let m = mesh_from_o3d(&o3d);
        assert!(m.one_sided);
        // the same viewer in the world frame looks along +y with z up: the screen shows
        // world x to the right and world z upwards
        let p: Vec<(f32, f32)> = m.indices.iter().map(|i| (m.positions[*i as usize].x, m.positions[*i as usize].z)).collect();
        let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
        assert!(area < 0.0, "front face must be clockwise on the screen (the renderer's front face), area {area}");
        assert_eq!(m.normals[0], Vec3::new(0.0, -1.0, 0.0));
    }
}

/// Möller-Trumbore ray/triangle test. Returns the distance along the ray.
pub fn ray_triangle(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    ray_triangle_bary(origin, dir, a, b, c).map(|(t, _, _)| t)
}

/// The same test, with the barycentric coordinates of the hit: `(t, u, v)`, the point being
/// `a + u * (b - a) + v * (c - a)`.
pub fn ray_triangle_bary(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<(f32, f32, f32)> {
    let e1 = b - a;
    let e2 = c - a;
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-8 {
        return None;
    }
    let inv = 1.0 / det;
    let t = origin - a;
    let u = t.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = t.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let d = e2.dot(q) * inv;
    if d > 1e-4 {
        Some((d, u, v))
    } else {
        None
    }
}

/// Where a ray met a mesh.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshHit {
    /// Distance along the ray.
    pub t: f32,
    /// Position of the triangle's first index in `MeshData::indices` (its material slot is the
    /// one of the `ranges` entry that holds it).
    pub index: usize,
    /// The texture coordinates at the hit.
    pub uv: Vec2,
}

impl MeshData {
    /// The material slot of the triangle whose first index is `index`.
    pub fn slot_of(&self, index: usize) -> u32 {
        self.ranges
            .iter()
            .find(|(first, count, _)| index >= *first as usize && index < (*first + *count) as usize)
            .map(|r| r.2)
            .unwrap_or(0)
    }
}

/// Closest hit of a world-space ray against a mesh under `transform`, with the triangle it
/// hit and the texture coordinates there (what a click on a screen texture needs).
pub fn ray_mesh_hit(origin: Vec3, dir: Vec3, mesh: &MeshData, transform: &Mat4) -> Option<MeshHit> {
    let inv = transform.inverse();
    let o = inv.transform_point3(origin);
    let d = inv.transform_vector3(dir);
    let scale = d.length();
    if scale < 1e-9 {
        return None;
    }
    let dn = d / scale;
    let mut best: Option<MeshHit> = None;
    for (k, tri) in mesh.indices.chunks_exact(3).enumerate() {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let (a, b, c) = (mesh.positions[i0], mesh.positions[i1], mesh.positions[i2]);
        if let Some((t, u, v)) = ray_triangle_bary(o, dn, a, b, c) {
            let t = t / scale;
            if best.map(|h| t < h.t).unwrap_or(true) {
                let uv = match (mesh.uvs.get(i0), mesh.uvs.get(i1), mesh.uvs.get(i2)) {
                    (Some(&x), Some(&y), Some(&z)) => x * (1.0 - u - v) + y * u + z * v,
                    _ => Vec2::ZERO,
                };
                best = Some(MeshHit { t, index: k * 3, uv });
            }
        }
    }
    best
}

/// Closest hit of a world-space ray against a mesh under `transform`.
/// A sphere around the points: the middle of their box and the farthest point from it.
pub fn bounding_sphere(points: &[Vec3]) -> (Vec3, f32) {
    if points.is_empty() {
        return (Vec3::ZERO, 0.0);
    }
    let (lo, hi) = points.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let c = (lo + hi) * 0.5;
    let r = points.iter().map(|p| p.distance_squared(c)).fold(0.0f32, f32::max).sqrt();
    (c, r)
}

/// Does the ray (`dir` of unit length) pass within `r` of `c`, ahead of its origin or with
/// the origin inside?
pub fn ray_near_sphere(origin: Vec3, dir: Vec3, c: Vec3, r: f32) -> bool {
    let to = c - origin;
    let along = to.dot(dir);
    let d2 = to.length_squared() - along * along;
    d2 <= r * r && (along >= 0.0 || to.length_squared() <= r * r)
}

pub fn ray_mesh(origin: Vec3, dir: Vec3, mesh: &MeshData, transform: &Mat4) -> Option<f32> {
    // transform the ray into mesh space (cheaper than transforming every vertex)
    let inv = transform.inverse();
    let o = inv.transform_point3(origin);
    let d = inv.transform_vector3(dir);
    let scale = d.length();
    if scale < 1e-9 {
        return None;
    }
    let dn = d / scale;
    let mut best: Option<f32> = None;
    for tri in mesh.indices.chunks_exact(3) {
        let (a, b, c) = (mesh.positions[tri[0] as usize], mesh.positions[tri[1] as usize], mesh.positions[tri[2] as usize]);
        if let Some(t) = ray_triangle(o, dn, a, b, c) {
            let t = t / scale;
            if best.map(|bt| t < bt).unwrap_or(true) {
                best = Some(t);
            }
        }
    }
    best
}

/// What a vertical probe found at one point: the highest face at or below the probe's top,
/// and the lowest face above it (a step the wheel cannot be under, or a bridge deck).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Probe {
    pub below: Option<f32>,
    pub above: Option<f32>,
}

impl Probe {
    /// Combine two probes of the same point (surfaces and terrain).
    pub fn merge(self, o: Probe) -> Probe {
        let pick = |a: Option<f32>, b: Option<f32>, f: fn(f32, f32) -> f32| match (a, b) {
            (Some(x), Some(y)) => Some(f(x, y)),
            (x, y) => x.or(y),
        };
        Probe { below: pick(self.below, o.below, f32::max), above: pick(self.above, o.above, f32::min) }
    }

    /// Put a single height into a probe with its top at `z_top`.
    pub fn of(z: f32, z_top: f32) -> Probe {
        if z <= z_top {
            Probe { below: Some(z), above: None }
        } else {
            Probe { below: None, above: Some(z) }
        }
    }
}

/// Upward-facing triangles of one tile (tile-local x/y in metres, absolute z), bucketed on a
/// coarse grid so that a wheel asks only the few faces around it.
#[derive(Debug, Clone, Default)]
pub struct DriveGrid {
    pub tris: Vec<[Vec3; 3]>,
    /// Per triangle: a wall top ([`is_wall_top`]), never stood on - a wall where it stands
    /// over the ground (see [`DriveGrid::probe_walls`]).
    pub ridge: Vec<bool>,
    cells: usize,
    cell: f32,
    /// Per cell, the range of `items` that lists its triangles (`cells² + 1` offsets).
    start: Vec<u32>,
    items: Vec<u32>,
}

impl DriveGrid {
    /// Edge of one bucket (m).
    pub const CELL: f32 = 4.0;

    /// Bytes the grid holds on the heap.
    pub fn heap_bytes(&self) -> usize {
        self.tris.capacity() * std::mem::size_of::<[Vec3; 3]>() + self.ridge.capacity() + self.start.capacity() * 4 + self.items.capacity() * 4
    }

    /// Add a triangle; walls (faces steeper than about 70°) are left out, they are nothing
    /// to stand on.
    pub fn push(&mut self, p: [Vec3; 3]) {
        self.push_kind(p, false);
    }

    /// Add a triangle, a wall top or not.
    pub fn push_kind(&mut self, p: [Vec3; 3], ridge: bool) {
        let nrm = (p[1] - p[0]).cross(p[2] - p[0]);
        let len = nrm.length();
        if len < 1e-6 || nrm.z.abs() / len < 0.3 {
            return;
        }
        self.tris.push(p);
        self.ridge.push(ridge);
    }

    /// Bucket the triangles of a tile `tile` metres wide; those entirely outside are dropped.
    pub fn build(&mut self, tile: f32) {
        let n = ((tile / Self::CELL).ceil() as usize).max(1);
        self.cells = n;
        self.cell = tile / n as f32;
        let mut ranges: Vec<(usize, usize, usize, usize)> = Vec::with_capacity(self.tris.len());
        let mut keep = Vec::with_capacity(self.tris.len());
        let mut keep_ridge = Vec::with_capacity(self.tris.len());
        self.ridge.resize(self.tris.len(), false);
        for (t, r) in self.tris.iter().zip(self.ridge.iter()) {
            let (lo_x, hi_x) = (t[0].x.min(t[1].x).min(t[2].x), t[0].x.max(t[1].x).max(t[2].x));
            let (lo_y, hi_y) = (t[0].y.min(t[1].y).min(t[2].y), t[0].y.max(t[1].y).max(t[2].y));
            if hi_x < 0.0 || hi_y < 0.0 || lo_x > tile || lo_y > tile {
                continue;
            }
            let c = |v: f32| ((v / self.cell).floor().max(0.0) as usize).min(n - 1);
            ranges.push((c(lo_x), c(hi_x), c(lo_y), c(hi_y)));
            keep.push(*t);
            keep_ridge.push(*r);
        }
        self.tris = keep;
        self.ridge = keep_ridge;
        let mut count = vec![0u32; n * n + 1];
        for &(x0, x1, y0, y1) in &ranges {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    count[y * n + x] += 1;
                }
            }
        }
        let mut start = vec![0u32; n * n + 1];
        for k in 0..n * n {
            start[k + 1] = start[k] + count[k];
        }
        let mut fill = start.clone();
        let mut items = vec![0u32; start[n * n] as usize];
        for (i, &(x0, x1, y0, y1)) in ranges.iter().enumerate() {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let k = y * n + x;
                    items[fill[k] as usize] = i as u32;
                    fill[k] += 1;
                }
            }
        }
        self.start = start;
        self.items = items;
    }

    /// The faces over tile-local (x, y): the highest one not above `z_top` and the lowest one
    /// above it.
    pub fn probe(&self, x: f32, y: f32, z_top: f32) -> Probe {
        self.probe_kind(x, y, z_top, false)
    }

    /// The wall tops over tile-local (x, y) alone, as [`DriveGrid::probe`] gives the rest.
    pub fn probe_walls(&self, x: f32, y: f32, z_top: f32) -> Probe {
        self.probe_kind(x, y, z_top, true)
    }

    fn probe_kind(&self, x: f32, y: f32, z_top: f32, ridges: bool) -> Probe {
        let mut out = Probe::default();
        if self.cells == 0 || x < 0.0 || y < 0.0 {
            return out;
        }
        let (cx, cy) = ((x / self.cell) as usize, (y / self.cell) as usize);
        if cx >= self.cells || cy >= self.cells {
            return out;
        }
        let k = cy * self.cells + cx;
        for &i in &self.items[self.start[k] as usize..self.start[k + 1] as usize] {
            if self.ridge.get(i as usize).copied().unwrap_or(false) != ridges {
                continue;
            }
            let [a, b, c] = self.tris[i as usize];
            let d = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
            if d.abs() < 1e-9 {
                continue;
            }
            let l1 = ((b.y - c.y) * (x - c.x) + (c.x - b.x) * (y - c.y)) / d;
            let l2 = ((c.y - a.y) * (x - c.x) + (a.x - c.x) * (y - c.y)) / d;
            let l3 = 1.0 - l1 - l2;
            // a hair of tolerance so that a point on a shared edge is never missed
            const EPS: f32 = -1e-4;
            if l1 < EPS || l2 < EPS || l3 < EPS {
                continue;
            }
            out = out.merge(Probe::of(l1 * a.z + l2 * b.z + l3 * c.z, z_top));
        }
        out
    }
}

/// Per-tile raster of the surfaces (roads, crossings) lying on the terrain: where a texel is
/// covered the terrain is not drawn (the original cuts the terrain polygons instead) and the
/// surface height is used for ground queries.
/// How far below the ground a surface may lie and still take the ground away: enough for a
/// Take the ground away under every road surface lying about its height (`OMSI_ROAD_CUT=1`).
/// Omsi.exe does not: the ground goes only where the map says (`[terrainhole]` meshes, the
/// splines' `[terrainholeprofile]`, a tile's `.hole` file), and a road lying on it wins by
/// its depth bias. Cut by a raster of 1.5-3 m texels, the ground went a metre or two past
/// the road's edge too, and the sky showed through along kerbs and car parks.
pub fn road_cut() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("OMSI_ROAD_CUT").is_some())
}

/// How far below the ground a surface may lie and still take the ground away: enough for a
/// sunken road or an underpass, not enough for a junction plate left at height zero.
const DEEP_CUT: f32 = 4.0;

/// How far around an aligned spline's outline the mask is looked at texel by texel (m).
const OUTLINE_EDGE: f32 = 1.2;
/// How far inside an outline the ground is still kept (m): the filtered mask puts a slanting
/// edge up to ~7 cm off, and past the outline (3 cm inside the road's edge) the ground gone
/// shows the sky under a kerb's top - pale slivers every two metres along the kerbs.
const OUTLINE_KEEP: f32 = 0.08;

pub struct TileSurface {
    pub size: usize,
    /// The raster in blocks of `BLOCK`² texels, made when a surface first touches one: a
    /// tile's roads cover a part of it, and seven full layers of 512² texels took 5 MB a
    /// tile (270 MB around the Ahlheim main station). Per texel: whether a surface and a
    /// surface a vehicle can stand on cover it, the highest surface's height, the *lowest*
    /// surface's height (the ground has to be cut wherever it would come up through any of
    /// them - at a junction the plate can ride a metre over the road that runs under its
    /// edge), and the drivable surface's height (a railway embankment or a bridge deck
    /// passing over a road is a surface - the ground is cut under it - but the wheels must
    /// not be lifted onto it). [`TileSurface::finish`] drops a block's lowest or drivable
    /// heights where they are the highest one's.
    blocks: Vec<Option<Box<SurfaceBlock>>>,
    /// `[terrainhole]`: the ground is cut here whatever its height - a junction, an
    /// underpass or a tunnel mouth names a cutter mesh in its model, and OMSI takes the
    /// terrain away under it instead of leaving a mound over the carriageway. With the
    /// highest point of the cutter over each texel: a hole only takes the ground away where
    /// it reaches up to it, so a junction a mapper left thirty metres down does not open a
    /// window into the sky. In blocks like the surfaces; most tiles have none.
    holes: Vec<Option<Box<[f32; BLOCK * BLOCK]>>>,
    /// The outlines the splines laid with `[spline_terrain_align]` cut out of the ground
    /// ([`spline_hole_outlines`]), in tile metres, with their bounds (x0, y0, x1, y1): cut
    /// exactly along them, as Omsi.exe cuts the terrain's triangles, not by texel.
    outlines: Vec<(Vec<Vec2>, [f32; 4])>,
    /// The faces the wheels stand on over this tile (spline height profiles, surface objects):
    /// a texel of the raster is most of a metre wide and holds one height, which put a wheel
    /// next to a kerb on top of the pavement or into the road's camber.
    pub drive: DriveGrid,
}

/// Edge of a [`TileSurface`] block (texels).
const BLOCK: usize = 16;
const COVERED: u8 = 1;
const ROAD: u8 = 2;

struct SurfaceBlock {
    flags: [u8; BLOCK * BLOCK],
    height: [f32; BLOCK * BLOCK],
    /// None: the same as `height`.
    low: Option<Box<[f32; BLOCK * BLOCK]>>,
    road: Option<Box<[f32; BLOCK * BLOCK]>>,
}

impl SurfaceBlock {
    fn new() -> Box<SurfaceBlock> {
        Box::new(SurfaceBlock { flags: [0; BLOCK * BLOCK], height: [0.0; BLOCK * BLOCK], low: Some(Box::new([0.0; BLOCK * BLOCK])), road: Some(Box::new([0.0; BLOCK * BLOCK])) })
    }
}

impl TileSurface {
    /// Bytes the rasters and the wheel grid hold on the heap: (rasters, wheel grid).
    pub fn heap_bytes(&self) -> (usize, usize) {
        let n = BLOCK * BLOCK;
        let mut rasters = self.blocks.capacity() * 8 + self.holes.capacity() * 8;
        for b in self.blocks.iter().flatten() {
            rasters += n * 5 + b.low.as_ref().map(|_| n * 4).unwrap_or(0) + b.road.as_ref().map(|_| n * 4).unwrap_or(0);
        }
        rasters += self.holes.iter().flatten().count() * n * 4;
        (rasters, self.drive.heap_bytes())
    }

    pub fn new(size: usize) -> TileSurface {
        let blocks = size.div_ceil(BLOCK);
        TileSurface { size, blocks: (0..blocks * blocks).map(|_| None).collect(), holes: (0..blocks * blocks).map(|_| None).collect(), outlines: Vec::new(), drive: DriveGrid::default() }
    }

    /// Block and index within it of texel `k`.
    #[inline]
    fn at(&self, k: usize) -> (usize, usize) {
        let (x, y) = (k % self.size, k / self.size);
        let per_row = self.size.div_ceil(BLOCK);
        ((y / BLOCK) * per_row + x / BLOCK, (y % BLOCK) * BLOCK + x % BLOCK)
    }

    /// Whether a surface covers texel `k`.
    #[inline]
    pub fn covered(&self, k: usize) -> bool {
        let (b, i) = self.at(k);
        self.blocks[b].as_ref().map(|b| b.flags[i] & COVERED != 0).unwrap_or(false)
    }

    /// Whether a drivable surface covers texel `k`.
    #[inline]
    pub fn road_covered(&self, k: usize) -> bool {
        let (b, i) = self.at(k);
        self.blocks[b].as_ref().map(|b| b.flags[i] & ROAD != 0).unwrap_or(false)
    }

    /// Height of the highest surface over texel `k` (0 where none).
    #[inline]
    pub fn height(&self, k: usize) -> f32 {
        let (b, i) = self.at(k);
        self.blocks[b].as_ref().map(|b| b.height[i]).unwrap_or(0.0)
    }

    /// Height of the lowest surface over texel `k`.
    #[inline]
    pub fn low_height(&self, k: usize) -> f32 {
        let (b, i) = self.at(k);
        self.blocks[b].as_ref().map(|b| b.low.as_ref().map(|l| l[i]).unwrap_or(b.height[i])).unwrap_or(0.0)
    }

    /// Height of the drivable surface over texel `k`.
    #[inline]
    pub fn road_height(&self, k: usize) -> f32 {
        let (b, i) = self.at(k);
        self.blocks[b].as_ref().map(|b| b.road.as_ref().map(|l| l[i]).unwrap_or(b.height[i])).unwrap_or(0.0)
    }

    /// The top of a `[terrainhole]` cutter over texel `k`, if one is there.
    #[inline]
    pub fn hole_height(&self, k: usize) -> Option<f32> {
        let (b, i) = self.at(k);
        self.holes[b].as_ref().map(|h| h[i]).filter(|h| *h > f32::MIN)
    }

    fn block_mut(&mut self, k: usize) -> (&mut SurfaceBlock, usize) {
        let (b, i) = self.at(k);
        (self.blocks[b].get_or_insert_with(SurfaceBlock::new), i)
    }

    fn add_hole(&mut self, k: usize, h: f32) {
        let (b, i) = self.at(k);
        let block = self.holes[b].get_or_insert_with(|| Box::new([f32::MIN; BLOCK * BLOCK]));
        block[i] = block[i].max(h);
    }

    /// Put a surface at height `h` over texel `k`.
    fn add_surface(&mut self, k: usize, h: f32, drivable: bool) {
        let (b, i) = self.block_mut(k);
        let first = b.flags[i] & COVERED == 0;
        if first || h > b.height[i] {
            b.height[i] = h;
        }
        if let Some(low) = b.low.as_mut() {
            if first || h < low[i] {
                low[i] = h;
            }
        }
        b.flags[i] |= COVERED;
        if drivable {
            if let Some(road) = b.road.as_mut() {
                if b.flags[i] & ROAD == 0 || h > road[i] {
                    road[i] = h;
                }
            }
            b.flags[i] |= ROAD;
        }
    }

    /// Let go of what the finished raster does not need: a block's lowest and drivable
    /// heights where every texel has them equal to the highest one.
    fn compact(&mut self) {
        for b in self.blocks.iter_mut().flatten() {
            let (flags, height) = (&b.flags, &b.height);
            if b.low.as_ref().map(|l| (0..BLOCK * BLOCK).all(|i| flags[i] & COVERED == 0 || l[i] == height[i])).unwrap_or(false) {
                b.low = None;
            }
            if b.road.as_ref().map(|r| (0..BLOCK * BLOCK).all(|i| flags[i] & ROAD == 0 || r[i] == height[i])).unwrap_or(false) {
                b.road = None;
            }
        }
    }

    /// Add a mesh the wheels stand on: a spline's height profile, a surface object, or a low
    /// object with a `[collision_mesh]` (a traffic island) that cuts no ground. The probe
    /// takes the highest face below the axle, so a bridge deck overhead is never picked.
    pub fn add_drive_mesh(&mut self, mesh: &MeshData, transform: &Mat4, origin: DVec3, tx: i32, ty: i32) {
        let ident = *transform == Mat4::IDENTITY;
        let off = (origin - DVec3::new(tx as f64 * tile_size(), ty as f64 * tile_size(), 0.0)).as_vec3();
        for tri in mesh.indices.chunks_exact(3) {
            let mut p = [Vec3::ZERO; 3];
            for k in 0..3 {
                let v = mesh.positions[tri[k] as usize];
                p[k] = (if ident { v } else { transform.transform_point3(v) }) + off;
            }
            self.drive.push(p);
        }
    }

    /// Add a spline's height profiles ([`build_height_profile_mesh`]): its wall tops (the
    /// range of material 1) go in as walls.
    pub fn add_height_profiles(&mut self, mesh: &MeshData, origin: DVec3, tx: i32, ty: i32) {
        let off = (origin - DVec3::new(tx as f64 * tile_size(), ty as f64 * tile_size(), 0.0)).as_vec3();
        let ridge_from = mesh.ranges.iter().find(|r| r.2 == 1).map(|r| r.0 as usize).unwrap_or(usize::MAX);
        for (j, tri) in mesh.indices.chunks_exact(3).enumerate() {
            let p = [0, 1, 2].map(|k| mesh.positions[tri[k] as usize] + off);
            self.drive.push_kind(p, j * 3 >= ridge_from);
        }
    }

    /// Build the wheel lookup once every mesh of the tile is in (and let go of the raster
    /// layers that repeat the top surface).
    pub fn finish(&mut self) {
        self.drive.build(tile_size() as f32);
        self.compact();
    }

    /// Rasterize a mesh into this tile at (tx, ty). Mesh positions are relative to
    /// `origin` (after `transform`).
    pub fn rasterize(&mut self, mesh: &MeshData, transform: &Mat4, origin: DVec3, tx: i32, ty: i32) {
        self.rasterize_kind(mesh, transform, origin, tx, ty, true)
    }

    /// Cut the ground inside an aligned spline's outline (world x, y) on tile (tx, ty).
    pub fn add_outline(&mut self, ring: &[DVec2], tx: i32, ty: i32) {
        let o = DVec2::new(tx as f64 * tile_size(), ty as f64 * tile_size());
        let pts: Vec<Vec2> = ring.iter().map(|p| (*p - o).as_vec2()).collect();
        let b = pts.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p.x), b[1].min(p.y), b[2].max(p.x), b[3].max(p.y)]);
        let t = tile_size() as f32;
        if pts.len() < 3 || b[2] < -OUTLINE_EDGE || b[3] < -OUTLINE_EDGE || b[0] > t + OUTLINE_EDGE || b[1] > t + OUTLINE_EDGE {
            return;
        }
        self.outlines.push((pts, b));
    }

    /// Signed distance (m) from tile point (x, y) to the aligned splines' cut, negative
    /// inside it, as far as `OUTLINE_EDGE` (farther: None).
    fn outline_distance(&self, x: f32, y: f32) -> Option<f32> {
        let p = Vec2::new(x, y);
        let mut best: Option<f32> = None;
        for (ring, b) in &self.outlines {
            if x < b[0] - OUTLINE_EDGE || y < b[1] - OUTLINE_EDGE || x > b[2] + OUTLINE_EDGE || y > b[3] + OUTLINE_EDGE {
                continue;
            }
            let mut d2 = f32::MAX;
            let mut inside = false;
            let n = ring.len();
            for i in 0..n {
                let (a, c) = (ring[i], ring[(i + 1) % n]);
                let e = c - a;
                let t = ((p - a).dot(e) / e.length_squared().max(1e-12)).clamp(0.0, 1.0);
                d2 = d2.min((a + e * t - p).length_squared());
                if (a.y > y) != (c.y > y) && x < a.x + (y - a.y) / (c.y - a.y) * (c.x - a.x) {
                    inside = !inside;
                }
            }
            let d = if inside { -d2.sqrt() } else { d2.sqrt() };
            best = Some(best.map_or(d, |b: f32| b.min(d)));
        }
        best.filter(|d| *d < OUTLINE_EDGE)
    }

    /// Is tile point (x, y) inside an aligned spline's outline?
    fn in_outlines(&self, x: f32, y: f32) -> bool {
        self.outlines.iter().any(|(ring, b)| {
            if x < b[0] || y < b[1] || x > b[2] || y > b[3] {
                return false;
            }
            let mut inside = false;
            let n = ring.len();
            for i in 0..n {
                let (a, c) = (ring[i], ring[(i + 1) % n]);
                if (a.y > y) != (c.y > y) && x < a.x + (y - a.y) / (c.y - a.y) * (c.x - a.x) {
                    inside = !inside;
                }
            }
            inside
        })
    }

    /// Mark the texels a `[terrainhole]` mesh covers: there the ground goes away entirely.
    /// Its faces may be vertical walls, so nothing is skipped by orientation here.
    pub fn rasterize_hole(&mut self, mesh: &MeshData, transform: &Mat4, origin: DVec3, tx: i32, ty: i32) {
        let n = self.size as f32;
        let scale = n / tile_size() as f32;
        let ident = *transform == Mat4::IDENTITY;
        let off = (origin - DVec3::new(tx as f64 * tile_size(), ty as f64 * tile_size(), 0.0)).as_vec3();
        let (ox, oy) = (-off.x, -off.y);
        for tri in mesh.indices.chunks_exact(3) {
            let mut p = [Vec3::ZERO; 3];
            for k in 0..3 {
                let v = mesh.positions[tri[k] as usize];
                p[k] = if ident { v } else { transform.transform_point3(v) };
                p[k].z += off.z;
            }
            let (x0, y0) = ((p[0].x - ox) * scale, (p[0].y - oy) * scale);
            let (x1, y1) = ((p[1].x - ox) * scale, (p[1].y - oy) * scale);
            let (x2, y2) = ((p[2].x - ox) * scale, (p[2].y - oy) * scale);
            let minx = x0.min(x1).min(x2).floor().max(0.0) as i32;
            let maxx = x0.max(x1).max(x2).ceil().min(n - 1.0) as i32;
            let miny = y0.min(y1).min(y2).floor().max(0.0) as i32;
            let maxy = y0.max(y1).max(y2).ceil().min(n - 1.0) as i32;
            if minx > maxx || miny > maxy {
                continue;
            }
            let det = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
            if det.abs() < 1e-9 {
                continue;
            }
            let inv = 1.0 / det;
            for py in miny..=maxy {
                let cy = py as f32 + 0.5;
                for px in minx..=maxx {
                    let cx = px as f32 + 0.5;
                    let l1 = ((x1 - x0) * (cy - y0) - (cx - x0) * (y1 - y0)) * inv;
                    let l2 = ((cx - x0) * (y2 - y0) - (x2 - x0) * (cy - y0)) * inv;
                    let l0 = 1.0 - l1 - l2;
                    let eps = -0.002;
                    if l0 >= eps && l1 >= eps && l2 >= eps {
                        let i = py as usize * self.size + px as usize;
                        let h = l0 * p[0].z + l2 * p[1].z + l1 * p[2].z;
                        self.add_hole(i, h);
                    }
                }
            }
        }
    }

    /// `drivable`: this surface also carries vehicles (a road, a crossing plate, a car park).
    pub fn rasterize_kind(&mut self, mesh: &MeshData, transform: &Mat4, origin: DVec3, tx: i32, ty: i32, drivable: bool) {
        let n = self.size as f32;
        let scale = n / tile_size() as f32;
        let ident = *transform == Mat4::IDENTITY;
        // offset from the mesh origin to this tile's corner, in f64
        let off = (origin - DVec3::new(tx as f64 * tile_size(), ty as f64 * tile_size(), 0.0)).as_vec3();
        let (ox, oy) = (-off.x, -off.y);
        for tri in mesh.indices.chunks_exact(3) {
            let mut p = [Vec3::ZERO; 3];
            for k in 0..3 {
                let v = mesh.positions[tri[k] as usize];
                p[k] = if ident { v } else { transform.transform_point3(v) };
                p[k].z += off.z;
            }
            // skip (near-)vertical faces: they are walls, not surfaces
            let nrm = (p[1] - p[0]).cross(p[2] - p[0]);
            let len = nrm.length();
            if len < 1e-6 || nrm.z.abs() / len < 0.3 {
                continue;
            }
            let (x0, y0) = ((p[0].x - ox) * scale, (p[0].y - oy) * scale);
            let (x1, y1) = ((p[1].x - ox) * scale, (p[1].y - oy) * scale);
            let (x2, y2) = ((p[2].x - ox) * scale, (p[2].y - oy) * scale);
            let minx = x0.min(x1).min(x2).floor().max(0.0) as i32;
            let maxx = x0.max(x1).max(x2).ceil().min(n - 1.0) as i32;
            let miny = y0.min(y1).min(y2).floor().max(0.0) as i32;
            let maxy = y0.max(y1).max(y2).ceil().min(n - 1.0) as i32;
            if minx > maxx || miny > maxy {
                continue;
            }
            let det = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
            if det.abs() < 1e-9 {
                continue;
            }
            let inv = 1.0 / det;
            for py in miny..=maxy {
                let cy = py as f32 + 0.5;
                for px in minx..=maxx {
                    let cx = px as f32 + 0.5;
                    let l1 = ((x1 - x0) * (cy - y0) - (cx - x0) * (y1 - y0)) * inv; // weight of p2
                    let l2 = ((cx - x0) * (y2 - y0) - (x2 - x0) * (cy - y0)) * inv; // weight of p1
                    let l0 = 1.0 - l1 - l2;
                    let eps = -0.002;
                    if l0 >= eps && l1 >= eps && l2 >= eps {
                        let i = py as usize * self.size + px as usize;
                        let h = l0 * p[0].z + l2 * p[1].z + l1 * p[2].z;
                        self.add_surface(i, h, drivable);
                    }
                }
            }
        }
    }

    /// Surface height at tile-local metres, if covered.
    pub fn sample(&self, x: f32, y: f32) -> Option<f32> {
        let i = self.texel(x, y);
        self.covered(i).then(|| self.height(i))
    }

    /// Height of the drivable surface here, if any: what the wheels stand on.
    pub fn sample_road(&self, x: f32, y: f32) -> Option<f32> {
        let i = self.texel(x, y);
        self.road_covered(i).then(|| self.road_height(i))
    }

    /// Height of the drivable surface (else of any surface) here, blended between the four
    /// nearest covered texels. A texel is 0.7 m on a Berlin tile, and a wheel reading the
    /// nearest one climbs a slope in centimetre steps - the AI cars shook on every hill.
    pub fn sample_road_smooth(&self, x: f32, y: f32) -> Option<f32> {
        self.blend(x, y, true).or_else(|| self.blend(x, y, false))
    }

    fn blend(&self, x: f32, y: f32, road: bool) -> Option<f32> {
        let n = self.size as i32;
        let cell = tile_size() as f32 / self.size as f32;
        let (fx, fy) = (x / cell - 0.5, y / cell - 0.5);
        let (ix, iy) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - ix, fy - iy);
        let (mut acc, mut sum) = (0.0f32, 0.0f32);
        for (dx, dy, w) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
            let cx = (ix as i32 + dx).clamp(0, n - 1);
            let cy = (iy as i32 + dy).clamp(0, n - 1);
            let k = (cy * n + cx) as usize;
            let (covered, height) = if road { (self.road_covered(k), self.road_height(k)) } else { (self.covered(k), self.height(k)) };
            if covered && w > 0.0 {
                acc += height * w;
                sum += w;
            }
        }
        (sum > 0.05).then(|| acc / sum)
    }

    fn texel(&self, x: f32, y: f32) -> usize {
        let n = self.size as f32;
        let fx = (x / tile_size() as f32 * n).clamp(0.0, n - 1.0);
        let fy = (y / tile_size() as f32 * n).clamp(0.0, n - 1.0);
        fy as usize * self.size + fx as usize
    }

    /// The terrain mask as an RGBA image: alpha 255 where the terrain is visible.
    /// Alpha mask that cuts the terrain away under flush surfaces (255 = keep the terrain).
    ///
    /// Only texels whose surface lies within `flush` metres of the terrain are cut: a road
    /// that sits on a kerb 25 cm above the grass needs no hole, and cutting one there left a
    /// gap you could see the sky through. The cut is eroded by one texel as well, so the
    /// hole always stays inside the surface even after the mask is filtered.
    pub fn mask_image(&self, terrain_at: &dyn Fn(f32, f32) -> f32, flush: f32) -> Vec<u8> {
        let n = self.size;
        let cell = tile_size() as f32 / n as f32;
        let mut cut = vec![false; n * n];
        for j in 0..n {
            for i in 0..n {
                let k = j * n + i;
                if let Some(top) = self.hole_height(k) {
                    // a [terrainhole] takes the ground away where it reaches up to it
                    let t = terrain_at((i as f32 + 0.5) * cell, (j as f32 + 0.5) * cell);
                    if t <= top + DEEP_CUT {
                        cut[k] = true;
                        continue;
                    }
                }
                if !self.covered(k) {
                    continue;
                }
                let t = terrain_at((i as f32 + 0.5) * cell, (j as f32 + 0.5) * cell);
                cut[k] = self.cuts(k, t, flush);
            }
        }
        let mut out = vec![255u8; n * n * 4];
        // the aligned splines' outlines: how much of each texel keeps its ground (4×4
        // samples where an edge passes through it). Filtered, the half-way value lies on the
        // outline; and a strip of ground between two holes narrower than two texels (a
        // median, 0.96 m on the Falkenseer Chaussee) keeps its full value, where a distance
        // field's ridge sank to the cut-off and let the sky through in lens-shaped patches.
        if !self.outlines.is_empty() {
            let (mut lo, mut hi) = ([usize::MAX; 2], [0usize; 2]);
            for (_, b) in &self.outlines {
                lo[0] = lo[0].min(((b[0] - OUTLINE_EDGE) / cell).floor().max(0.0) as usize);
                lo[1] = lo[1].min(((b[1] - OUTLINE_EDGE) / cell).floor().max(0.0) as usize);
                hi[0] = hi[0].max((((b[2] + OUTLINE_EDGE) / cell).ceil().max(0.0) as usize).min(n));
                hi[1] = hi[1].max((((b[3] + OUTLINE_EDGE) / cell).ceil().max(0.0) as usize).min(n));
            }
            for j in lo[1]..hi[1] {
                for i in lo[0]..hi[0] {
                    let (cx, cy) = ((i as f32 + 0.5) * cell, (j as f32 + 0.5) * cell);
                    let Some(d) = self.outline_distance(cx, cy) else { continue };
                    let a = if d < -cell * 0.75 {
                        0
                    } else if d > cell * 0.75 {
                        255
                    } else {
                        let kept = (0..16)
                            .filter(|q| {
                                let (x, y) = (cx + ((q % 4) as f32 - 1.5) * cell / 4.0, cy + ((q / 4) as f32 - 1.5) * cell / 4.0);
                                !self.in_outlines(x, y) || self.outline_distance(x, y).is_some_and(|d| d > -OUTLINE_KEEP)
                            })
                            .count();
                        (kept * 255 / 16) as u8
                    };
                    out[(j * n + i) * 4 + 3] = a;
                }
            }
        }
        for j in 0..n {
            for i in 0..n {
                // erode: cut only where the whole 3×3 neighbourhood is cut
                let all = (-1i32..=1).all(|dj| {
                    (-1i32..=1).all(|di| {
                        let (x, y) = (i as i32 + di, j as i32 + dj);
                        x < 0 || y < 0 || x >= n as i32 || y >= n as i32 || cut[y as usize * n + x as usize]
                    })
                });
                if all && cut[j * n + i] {
                    out[(j * n + i) * 4 + 3] = 0;
                }
            }
        }
        out
    }

    /// Would the ground be taken away at this point? (the per-texel decision `mask_image`
    /// makes, for checks that need to know whether a road is visible)
    pub fn cut_at(&self, x: f32, y: f32, terrain_h: f32, flush: f32) -> bool {
        if self.in_outlines(x, y) {
            return true;
        }
        let i = self.texel(x, y);
        if self.hole_height(i).map(|top| terrain_h <= top + DEEP_CUT).unwrap_or(false) {
            return true;
        }
        self.cuts(i, terrain_h, flush)
    }

    /// The ground goes where a surface crosses it: nothing more than `flush` above it (a
    /// bridge or an embankment keeps its ground). Below it, a road or footway may lie up to
    /// a storey deep - a sunken road or an underpass still shows - but anything else only
    /// takes the ground away where it is flush with it: the far slope of a railway
    /// embankment runs on under the grass (Berlin-Spandau's `Damm1.sli` down to 16 m below
    /// it), and cutting the ground over it opened a band you could see the sky through.
    /// A plate a mapper left thirty metres down takes nothing either.
    pub fn cuts(&self, k: usize, t: f32, flush: f32) -> bool {
        if !road_cut() || !self.covered(k) || self.low_height(k) - flush > t {
            return false;
        }
        if self.road_covered(k) && t <= self.road_height(k) + DEEP_CUT {
            return true;
        }
        t <= self.height(k) + flush
    }

    /// The texels some block of the raster holds (surfaces or holes), in order: the others
    /// are empty.
    fn touched(&self) -> impl Iterator<Item = usize> + '_ {
        let per_row = self.size.div_ceil(BLOCK);
        (0..self.blocks.len()).filter(|b| self.blocks[*b].is_some() || self.holes[*b].is_some()).flat_map(move |b| {
            let (bx, by) = (b % per_row, b / per_row);
            (0..BLOCK * BLOCK).filter_map(move |i| {
                let (x, y) = (bx * BLOCK + i % BLOCK, by * BLOCK + i / BLOCK);
                (x < self.size && y < self.size).then_some(y * self.size + x)
            })
        })
    }

    /// Is any texel actually cut? (`mask_image` with the same arguments would do something.)
    /// (Only the map's own holes - `[terrainhole]` meshes, the splines' `[terrainholeprofile]`
    /// - unless `road_cut`.)
    pub fn cuts_anything(&self, terrain_at: &dyn Fn(f32, f32) -> f32, flush: f32) -> bool {
        if !self.outlines.is_empty() {
            return true;
        }
        let n = self.size;
        let cell = tile_size() as f32 / n as f32;
        self.touched().any(|k| {
            let (i, j) = (k % n, k / n);
            let t = terrain_at((i as f32 + 0.5) * cell, (j as f32 + 0.5) * cell);
            self.hole_height(k).map(|top| t <= top + DEEP_CUT).unwrap_or(false) || (self.covered(k) && self.cuts(k, t, flush))
        })
    }
}

#[cfg(test)]
mod cant_tests {
    use super::*;

    #[test]
    fn cant_is_a_percentage_within_the_half_cant_width() {
        let c = SplineCurve { start: DVec3::ZERO, heading_deg: 0.0, length: 10.0, radius: 0.0, grad_start: 0.0, grad_end: 0.0, delta_h: None, cant_start: 5.0, cant_end: 5.0, skew_start: 0.0, skew_end: 0.0, tex_offset: 0.0, seed: 0, half_cant_width: 3.0 };
        // 2 m right at 5 %: 10 cm down
        assert!((c.offset_point(5.0, 2.0, 0.0).z + 0.10).abs() < 1e-9);
        // beyond the half cant width the height stays what it is at its edge
        assert!((c.offset_point(5.0, 6.0, 0.0).z + 0.15).abs() < 1e-9);
        assert!((c.offset_point(5.0, -6.0, 0.0).z - 0.15).abs() < 1e-9);
    }
}

#[cfg(test)]
mod ray_hit_tests {
    use super::*;

    /// A 2 x 2 quad in the x/z plane at y = 0 (x -1..1, z -1..1), two triangles, two material slots;
    /// u runs with x, v runs down (v = 0 at z = 1).
    fn quad() -> MeshData {
        MeshData {
            positions: vec![Vec3::new(-1.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 1.0), Vec3::new(1.0, 0.0, -1.0), Vec3::new(-1.0, 0.0, -1.0)],
            normals: vec![Vec3::Y; 4],
            uvs: vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)],
            ranges: vec![(0, 3, 0), (3, 3, 1)],
            indices: vec![0, 1, 2, 0, 2, 3],
            one_sided: false,
        }
    }

    #[test]
    fn a_ray_gives_the_texture_coordinates_it_hit() {
        let m = quad();
        let id = Mat4::IDENTITY;
        let h = ray_mesh_hit(Vec3::new(0.5, -5.0, 0.5), Vec3::Y, &m, &id).unwrap();
        assert!((h.t - 5.0).abs() < 1e-4);
        assert!((h.uv - Vec2::new(0.75, 0.25)).length() < 1e-4, "{:?}", h.uv);
        assert_eq!(m.slot_of(h.index), 0, "the upper right triangle is slot 0");
        let h = ray_mesh_hit(Vec3::new(-0.5, -5.0, -0.5), Vec3::Y, &m, &id).unwrap();
        assert!((h.uv - Vec2::new(0.25, 0.75)).length() < 1e-4, "{:?}", h.uv);
        assert_eq!(m.slot_of(h.index), 1);
        assert!(ray_mesh_hit(Vec3::new(2.0, -5.0, 0.0), Vec3::Y, &m, &id).is_none());
    }

    #[test]
    fn the_mesh_transform_is_undone() {
        let m = quad();
        // the quad moved 10 m east and doubled in size: its middle is at x = 10, z = 0
        let xf = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0)) * Mat4::from_scale(Vec3::splat(2.0));
        let h = ray_mesh_hit(Vec3::new(11.0, -3.0, 0.5), Vec3::Y, &m, &xf).unwrap();
        assert!((h.t - 3.0).abs() < 1e-4);
        assert!((h.uv - Vec2::new(0.75, 0.375)).length() < 1e-4, "{:?}", h.uv);
        assert_eq!(ray_mesh_hit(Vec3::new(11.0, -3.0, 0.5), Vec3::Y, &m, &xf).map(|h| h.t), ray_mesh(Vec3::new(11.0, -3.0, 0.5), Vec3::Y, &m, &xf));
    }
}