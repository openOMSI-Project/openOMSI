//! Painting the layers into the bus's texture: per texel, where it lies on the bus (the bake)
//! says what each layer puts there - a fill by the base's colour zone, a stripe by its height, a
//! decal by its place on its side - blended in linear light over the base, the base's alpha (the
//! reflection mask) kept, only on the outside and (unless a layer says otherwise) only in the
//! paint zones, with the base's seams and shading kept as far as the layer's detail asks. The
//! texels round what the triangles cover take the colour next to them (the bleed).
//!
//! While editing this runs on a worker at the editing size; the export runs the same at the full
//! size, a band of rows at a time.

use super::bake::{Bake, Sample, Zones, BLACK, COVERED, GLASS, KEEP, OUTSIDE};
use super::colour::{self, to_linear, to_srgb};
use super::model::{self, BusDims, Gradient, Kind, Layer, Place};
use super::shapes::{self, Raster};
use glam::{Vec2, Vec3};
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

/// One painted texture at one size (or rows of it): its bake and what the base says per texel.
pub struct Canvas {
    pub bake: Bake,
    /// sRGB and alpha of the base.
    pub base: Vec<[u8; 4]>,
    /// How each texel is made of the zones (`Zones::unmix`): zone A, zone B (or `BLACK`) and
    /// A's part t (×50 000).
    pub mix: Vec<(u8, u8, u16)>,
    /// The detail ratio (×100) for the layers that paint over everything.
    pub detail: Vec<u8>,
    pub zones: Zones,
    /// The texture has an alpha channel: a window's texel that is opaque in it is paint seen
    /// through the window's mesh (the Citaro's front below its windscreen), not glass.
    pub alpha: bool,
    /// The bus maker's template for this texture, if it has one.
    pub template: Option<Template>,
}

/// A maker's template (the Repaint-Tool's `.rpc`: its `MA` and `MU`) for a canvas's rows: where
/// the paint goes (MA, white) and how a colour shows there (MU, see `show`) - the seams and the
/// shading as the bus's maker drew them. Where a texture has one, the stripes and fills paint
/// where both its mask and the colour zones say paint, with its shading; round the mask (a
/// rubber's rim it leaves out in the body's colour) the zones alone recolour.
pub struct Template {
    pub ma: Vec<u8>,
    pub mu: Vec<[u8; 3]>,
    /// MU where the mask paints, its median per channel: the paint's own light.
    pub mu_ref: [f32; 3],
}

impl Template {
    /// Rows `y0..y1` of the template's MA and MU (RGBA, `w` wide, the texture's size).
    pub fn rows(ma: &[u8], mu: &[u8], w: u32, y0: u32, y1: u32) -> Template {
        let r = |img: &[u8]| -> Vec<[u8; 3]> { img[(y0 * w * 4) as usize..(y1 * w * 4) as usize].chunks_exact(4).map(|p| [p[0], p[1], p[2]]).collect() };
        // (over the whole picture, so that every band of the export shows a colour the same)
        let mut m: Vec<[u8; 3]> = ma.chunks_exact(4).zip(mu.chunks_exact(4)).step_by(3).filter(|(a, _)| a[0] > 128).map(|(_, u)| [u[0], u[1], u[2]]).collect();
        let mut mu_ref = [255.0f32; 3];
        if !m.is_empty() {
            for (k, v) in mu_ref.iter_mut().enumerate() {
                m.sort_by_key(|c| c[k]);
                *v = m[m.len() / 2][k].max(16) as f32;
            }
        }
        Template { ma: r(ma).into_iter().map(|c| ((c[0] as u16 + c[1] as u16 + c[2] as u16) / 3) as u8).collect(), mu: r(mu), mu_ref }
    }

    /// The colour `n` (linear) as the template shows it at texel `i`: its shading MU against the
    /// paint's own light, `detail` of the way from flat. (The Repaint-Tool adds AD, highlights and
    /// wear drawn for a light paint: on a dark blue they came out as white grime all over.)
    fn show(&self, i: usize, n: [f32; 3], detail: f32) -> [f32; 3] {
        std::array::from_fn(|k| {
            let c = colour::linear_to_srgb(n[k].clamp(0.0, 1.0));
            // (its wear softened: the seams and shading are what make the paint real)
            let r = self.mu[i][k] as f32 / self.mu_ref[k];
            let d = c * (1.0 + (r - 1.0) * 0.6).clamp(0.5, 1.1);
            colour::srgb_to_linear((c + (d - c) * detail).clamp(0.0, 1.0))
        })
    }
}

/// t's scale in `Canvas::mix`.
const T_SCALE: f32 = 50_000.0;

/// How far (texels) round a stray speck the paint is looked for.
const SPECK: usize = 5;

impl Canvas {
    /// Rows `y0..y1` of target `target` at `w` x `h`, its base `base` (RGBA at that size).
    #[cfg(test)]
    pub fn build(tris: &[super::bake::Tri], target: u8, w: u32, h: u32, y0: u32, y1: u32, base: &[u8], outside: &super::bake::Outside, zones: &Zones) -> Canvas {
        Canvas::with(Bake::build(tris, target, w, h, y0, y1, outside), base, zones)
    }

    /// A canvas of a bake made before (the zones needed it first); `base` is the whole texture,
    /// `zones` this texture's.
    pub fn with(bake: Bake, base: &[u8], zones: &Zones) -> Canvas {
        let (w, y0, y1) = (bake.w as usize, bake.y0 as usize, bake.y1 as usize);
        let rows = &base[y0 * w * 4..y1 * w * 4];
        // (a texture without zones of its own - nothing of it seen from outside - takes those
        // of its rows)
        let own;
        let zones = if zones.centres.is_empty() {
            own = Zones::of(&rows.chunks_exact(4).step_by(7).map(|p| [p[0], p[1], p[2]]).collect::<Vec<_>>());
            &own
        } else {
            zones
        };
        // (the rows round the band too, for the specks below: a band of the export paints as the
        // whole does)
        let h = base.len() / 4 / w.max(1);
        let (a0, a1) = (y0.saturating_sub(SPECK), (y1 + SPECK).min(h));
        let (mut mix, detail): (Vec<(u8, u8, u16)>, Vec<u8>) = base[a0 * w * 4..a1 * w * 4]
            .par_chunks_exact(4)
            .map(|p| {
                let c = [to_linear(p[0]), to_linear(p[1]), to_linear(p[2])];
                let lab = colour::lab_of_linear(c);
                let (a, b, t) = zones.unmix(c, lab);
                ((a, b, (t * T_SCALE + 0.5) as u16), (zones.detail(lab) * 100.0 + 0.5) as u8)
            })
            .unzip();
        // specks and streaks of a stray colour in the body - dirt, grime, a scratch, old thin
        // lettering - are the body's paint: a texel of a small zone that is no paint and not
        // black, with paint round most of it, takes the paint round it at its own light (grey and
        // white streaks of grime stayed on a cream bus painted blue; a grey bumper or a chrome
        // strip three texels wide has too little paint round it)
        let dominant = |m: (u8, u8, u16)| if m.2 as f32 >= 0.5 * T_SCALE { m.0 } else { m.1 };
        let stray = |z: u8| z != BLACK && !zones.is_paint(z as usize) && zones.share.get(z as usize).is_some_and(|s| *s < 0.03) && zones.centres[z as usize][0] >= 25.0;
        let rows_n = a1 - a0;
        // (a texel of the body half mixed with such a colour too: grime over cream came out
        // half grey under the new colour)
        let fixed: Vec<(usize, (u8, u8, u16))> = (0..mix.len())
            .into_par_iter()
            .filter(|&i| {
                let (a, b, _) = mix[i];
                let paint = |z: u8| z != BLACK && zones.is_paint(z as usize);
                stray(dominant(mix[i])) || (stray(a) && (paint(b) || b == BLACK)) || (stray(b) && paint(a))
            })
            .filter_map(|i| {
                let (x, y) = (i % w, i / w);
                let mut votes = [0u16; 32];
                let (mut paint, mut all) = (0, 0);
                for yy in y.saturating_sub(SPECK)..(y + SPECK + 1).min(rows_n) {
                    for xx in x.saturating_sub(SPECK)..(x + SPECK + 1).min(w) {
                        let d = dominant(mix[yy * w + xx]);
                        all += 1;
                        if d != BLACK && zones.is_paint(d as usize) {
                            paint += 1;
                            votes[(d as usize).min(31)] += 1;
                        }
                    }
                }
                if paint * 10 < all * 7 {
                    return None;
                }
                let z = (0..32).max_by_key(|k| votes[*k])? as u8;
                let o = (a0 * w + i) * 4;
                let c = [to_linear(base[o]), to_linear(base[o + 1]), to_linear(base[o + 2])];
                let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
                let t = (lum(c) / lum(zones.lin[z as usize]).max(1e-3)).clamp(0.3, 1.3);
                Some((i, (z, BLACK, (t * T_SCALE + 0.5) as u16)))
            })
            .collect();
        for (i, m) in fixed {
            mix[i] = m;
        }
        let keep = (y0 - a0) * w..(y1 - a0) * w;
        let mix = mix[keep.clone()].to_vec();
        let detail = detail[keep].to_vec();
        let alpha = base.par_chunks_exact(4).any(|p| p[3] < 255);
        let base: Vec<[u8; 4]> = rows.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
        Canvas { bake, base, mix, detail, zones: zones.clone(), alpha, template: None }
    }

    /// The canvas with the maker's template of its texture.
    pub fn with_template(mut self, t: Option<Template>) -> Canvas {
        self.template = t.filter(|t| t.ma.len() == self.base.len());
        self
    }

    #[cfg(test)]
    /// How much of texel `i` is paint (0..1): its zones' paint by their parts.
    pub fn paint_share(&self, i: usize) -> f32 {
        let (a, b, t) = self.mix[i];
        let t = (t as f32 / T_SCALE).min(1.0);
        let p = |z: u8| if z != BLACK && self.zones.is_paint(z as usize) { 1.0 } else { 0.0 };
        t * p(a) + (1.0 - t) * p(b)
    }
}

/// The base's colours at its outside texels (for the zones): not the windows' nor the wheels'.
pub fn outside_colours(bake: &Bake, base: &[u8]) -> Vec<[u8; 3]> {
    let row0 = (bake.y0 * bake.w) as usize;
    bake.first.iter().enumerate().filter(|(_, s)| s.flags & OUTSIDE != 0 && s.flags & (KEEP | GLASS) == 0).map(|(i, _)| {
        let o = (row0 + i) * 4;
        [base[o], base[o + 1], base[o + 2]]
    }).collect()
}

// --- the layers, ready to paint ----------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum Colour {
    Solid([f32; 3]),
    Gradient([f32; 3], [f32; 3], Gradient),
}

impl Colour {
    fn of(c: &str, g: Option<&Gradient>) -> Colour {
        match g {
            Some(g) => Colour::Gradient(colour::linear_of(c), colour::linear_of(&g.colour2), g.clone()),
            None => Colour::Solid(colour::linear_of(c)),
        }
    }
    fn at(&self, box01: Vec2) -> [f32; 3] {
        match self {
            Colour::Solid(c) => *c,
            Colour::Gradient(a, b, g) => {
                let t = g.t(box01);
                [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Decal {
    pub centre: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    pub n: Vec3,
    pub sin: f32,
    pub cos: f32,
    pub w: f32,
    pub h: f32,
    /// Its content flipped across its own box (the copy on the other side as a mirror image).
    pub flip: bool,
    pub lo: Vec3,
    pub hi: Vec3,
}

/// How deep (metres, either way along its side's normal) a decal reaches into the bus.
const DECAL_DEPTH: f32 = 0.8;

impl Decal {
    pub fn new(p: &Place, w: f32, h: f32, flip: bool) -> Decal {
        let (u, v, n) = p.side.axes();
        let (sin, cos) = p.rotation.to_radians().sin_cos();
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for c in p.corners(w, h) {
            for k in [-DECAL_DEPTH, DECAL_DEPTH] {
                lo = lo.min(c + n * k);
                hi = hi.max(c + n * k);
            }
        }
        Decal { centre: p.centre, u, v, n, sin, cos, w, h, flip, lo, hi }
    }

    /// Where `s` lies in the decal's box (0..1 across, 0..1 down) and how much it faces it.
    fn at(&self, s: &Sample) -> Option<(Vec2, f32)> {
        if s.p.cmplt(self.lo).any() || s.p.cmpgt(self.hi).any() {
            return None;
        }
        let d = s.p - self.centre;
        let facing = ramp(s.normal().dot(self.n), 0.17, 0.5);
        if facing <= 0.0 {
            return None;
        }
        let du = d.dot(self.u);
        let dv = d.dot(self.v);
        let a = du * self.cos + dv * self.sin;
        let b = -du * self.sin + dv * self.cos;
        // (the place is already mirrored: a mirror image reads its content from the other end)
        let a = if self.flip { -a } else { a };
        let q = Vec2::new(a / self.w + 0.5, 0.5 - b / self.h);
        (q.x >= 0.0 && q.x <= 1.0 && q.y >= 0.0 && q.y <= 1.0).then_some((q, facing))
    }
}

#[derive(Clone, Debug)]
pub enum Op {
    /// A fill: the paint group `group`, else the zones within `radius` of `lab` (all the paint
    /// at radius 1000).
    Fill { lab: [f32; 3], radius: f32, group: Option<u8>, colour: Colour },
    Stripe { h1: f32, h2: f32, angle: f32, wave: f32, sides: model::Sides, colour: Colour },
    /// A mask with a colour (text, shape, own shape, an outline under them).
    Mask { decal: Decal, mask: Arc<Raster>, colour: Colour },
    Picture { decal: Decal, picture: Arc<Raster> },
    /// The brush's strokes, in order (the eraser's take away what the brush put before them).
    Brush { strokes: Arc<Vec<BrushStroke>>, colour: Colour },
}

/// A stroke ready to paint: its points and normals on the bus, its radius (metres), hardness,
/// cover and whether it erases; cut into pieces of a few points, each with its box (grown by
/// the radius), so that a texel looks only at the pieces near it.
#[derive(Clone, Debug)]
pub struct BrushStroke {
    pub points: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub radius: f32,
    pub hardness: f32,
    pub opacity: f32,
    pub erase: bool,
    pub pieces: Vec<(Vec3, Vec3, usize, usize)>,
}

impl BrushStroke {
    pub fn new(st: &model::Stroke, mirror: Option<f32>) -> BrushStroke {
        let flip = |v: [f32; 3], at: Option<f32>| match at {
            Some(x) => Vec3::new(2.0 * x - v[0], v[1], v[2]),
            None => Vec3::from(v),
        };
        let points: Vec<Vec3> = st.points.iter().map(|p| flip(*p, mirror)).collect();
        let normals: Vec<Vec3> = (0..points.len()).map(|i| st.normals.get(i).map(|n| if mirror.is_some() { Vec3::new(-n[0], n[1], n[2]) } else { Vec3::from(*n) }).unwrap_or(Vec3::ZERO)).collect();
        let radius = (st.radius_cm / 100.0).max(0.002);
        let mut pieces = Vec::new();
        let mut a = 0;
        while a < points.len() {
            let b = (a + 16).min(points.len() - 1);
            let (mut lo, mut hi) = (points[a], points[a]);
            for p in &points[a..=b] {
                lo = lo.min(*p);
                hi = hi.max(*p);
            }
            pieces.push((lo - Vec3::splat(radius), hi + Vec3::splat(radius), a, b));
            if b == points.len() - 1 {
                break;
            }
            a = b;
        }
        BrushStroke { points, normals, radius, hardness: st.hardness.clamp(0.0, 1.0), opacity: st.opacity.clamp(0.0, 1.0), erase: st.erase, pieces }
    }

    /// How much of the stroke lies on `p` (facing `n`): 1 within its hard core, falling off to 0
    /// at its radius; nothing on a surface that faces away from where it was painted.
    fn cover(&self, p: Vec3, n: Vec3) -> f32 {
        let mut best = f32::INFINITY;
        for &(lo, hi, a, b) in &self.pieces {
            if p.cmplt(lo).any() || p.cmpgt(hi).any() {
                continue;
            }
            for i in a..=b {
                let j = (i + 1).min(b);
                let (q0, q1) = (self.points[i], self.points[j]);
                let e = q1 - q0;
                let t = if e.length_squared() > 1e-10 { ((p - q0).dot(e) / e.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                let d = p.distance(q0 + e * t);
                if d < best {
                    let nn = self.normals[i] + self.normals[j];
                    if nn.length_squared() < 1e-6 || nn.normalize().dot(n) > 0.25 {
                        best = d;
                    }
                }
            }
        }
        if !best.is_finite() {
            return 0.0;
        }
        let x = best / self.radius;
        let core = self.hardness.min(0.98);
        let t = ((x - core) / (1.0 - core)).clamp(0.0, 1.0);
        (1.0 - t * t * (3.0 - 2.0 * t)) * self.opacity
    }
}

#[derive(Clone, Debug)]
pub struct Prepared {
    pub op: Op,
    /// Which layer it comes from (its index in the list).
    pub layer: usize,
    pub opacity: f32,
    pub detail: f32,
    pub over_trim: bool,
    pub over_glass: bool,
    /// The copy of a decal on the other side: not painted on texels both sides share (there the
    /// original shows on both, and the copy over it garbled it).
    pub copy: bool,
}

/// What the layers need besides themselves: the bus's measures, the pictures by hash, where the
/// mirror plane lies, and the texture's density (texels a metre) the decals are drawn at.
#[derive(Clone)]
pub struct Context {
    pub pictures: Arc<HashMap<String, Arc<Raster>>>,
    pub mirror: Option<f32>,
    pub density: f32,
    /// The longest side a decal is drawn at.
    pub max_px: u32,
}

/// Decals drawn before, by what they show and their size.
#[derive(Default)]
pub struct RasterCache(HashMap<String, (Arc<Raster>, Option<Arc<Raster>>)>);

impl RasterCache {
    fn get(&mut self, key: String, make: impl FnOnce() -> (Raster, Option<Raster>)) -> (Arc<Raster>, Option<Arc<Raster>>) {
        if let Some(v) = self.0.get(&key) {
            return v.clone();
        }
        if self.0.len() > 96 {
            self.0.clear();
        }
        let (a, b) = make();
        let v = (Arc::new(a), b.map(Arc::new));
        self.0.insert(key, v.clone());
        v
    }
}

/// A picture's own height over its width: the picture's, else what the layer kept of it.
pub fn picture_aspect(image: &str, aspect: Option<f32>, pictures: &HashMap<String, Arc<Raster>>) -> f32 {
    pictures.get(image).map(|r| r.h as f32 / r.w.max(1) as f32).or(aspect).unwrap_or(0.5)
}

/// A decal's size on the bus (metres) for a layer: a text's from its letters, a picture's from its
/// picture (unless it was stretched), a shape's from its place.
pub fn decal_size(kind: &Kind, pictures: &HashMap<String, Arc<Raster>>) -> Option<(f32, f32)> {
    match kind {
        Kind::Text { text, font, height_cm, spacing, outline, .. } => {
            let em = height_cm / 100.0;
            let m = shapes::text_metrics(text, font, *spacing);
            let pad = outline.as_ref().map(|o| o.width_cm / 100.0).unwrap_or(0.0);
            Some((m.width * em + 2.0 * pad, model::TEXT_BOX * em))
        }
        Kind::Image { image, aspect, place, .. } => {
            let a = picture_aspect(image, *aspect, pictures);
            Some((place.width_m, place.height_m.unwrap_or(place.width_m * a)))
        }
        Kind::Shape { place, .. } | Kind::Path { place, .. } => Some((place.width_m, place.height_m.unwrap_or(place.width_m))),
        _ => None,
    }
}

/// The layers as paint operations, the decals drawn (or taken from `cache`).
pub fn prepare(layers: &[Layer], cx: &Context, cache: &mut RasterCache) -> Vec<Prepared> {
    let mut out = Vec::new();
    for (i, l) in layers.iter().enumerate() {
        if !l.visible || l.opacity <= 0.0 {
            continue;
        }
        let mut push_as = |op: Op, copy: bool| out.push(Prepared { op, layer: i, opacity: l.opacity.clamp(0.0, 1.0), detail: l.detail.clamp(0.0, 1.0), over_trim: l.over_trim, over_glass: l.over_glass, copy });
        let mut push = |op: Op| push_as(op, false);
        match &l.kind {
            Kind::Fill { centre, radius, group, colour, gradient } => push(Op::Fill { lab: *centre, radius: *radius, group: *group, colour: Colour::of(colour, gradient.as_ref()) }),
            Kind::Brush { colour, strokes } => {
                // (copied onto the other side as the decals are)
                let mut all: Vec<BrushStroke> = strokes.iter().map(|st| BrushStroke::new(st, None)).collect();
                if let Some(plane) = cx.mirror {
                    all.extend(strokes.iter().map(|st| BrushStroke::new(st, Some(plane))));
                    // (the copy of each stroke right after it: an eraser takes both)
                    let n = strokes.len();
                    let mut paired = Vec::with_capacity(all.len());
                    for k in 0..n {
                        paired.push(all[k].clone());
                        paired.push(all[n + k].clone());
                    }
                    all = paired;
                }
                if !all.is_empty() {
                    push(Op::Brush { strokes: Arc::new(all), colour: Colour::of(colour, None) });
                }
            }
            Kind::Stripe { h1, h2, angle, wave, sides, colour, gradient, .. } => {
                push(Op::Stripe { h1: h1.min(*h2), h2: h1.max(*h2), angle: *angle, wave: *wave, sides: *sides, colour: Colour::of(colour, gradient.as_ref()) })
            }
            kind => {
                let Some(place) = kind.place() else { continue };
                let Some((w, h)) = decal_size(kind, &cx.pictures) else { continue };
                if w <= 0.0 || h <= 0.0 {
                    continue;
                }
                // drawn at the texture's density and a quarter more, within reason
                let k = (cx.density * 1.25).min(cx.max_px as f32 / w.max(h));
                let (pw, ph) = (((w * k).ceil() as u32).clamp(4, cx.max_px), ((h * k).ceil() as u32).clamp(4, cx.max_px));
                let drawn: Option<(Option<Arc<Raster>>, Arc<Raster>, Option<Arc<Raster>>, Option<[f32; 3]>)> = match kind {
                    Kind::Text { text, font, height_cm, outline, spacing, .. } => {
                        let em_px = height_cm / 100.0 * k;
                        let o_px = outline.as_ref().map(|o| o.width_cm / 100.0 * k).unwrap_or(0.0);
                        let key = format!("t|{text}|{font}|{spacing}|{em_px:.2}|{o_px:.2}");
                        let (f, o) = cache.get(key, || shapes::text_raster(text, font, *spacing, em_px, o_px));
                        Some((None, f, o, outline.as_ref().map(|o| colour::linear_of(&o.colour))))
                    }
                    Kind::Shape { shape, outline, .. } => {
                        let o_px = outline.as_ref().map(|o| o.width_cm / 100.0 * k).unwrap_or(0.0);
                        let key = format!("v|{shape}|{pw}|{ph}|{o_px:.2}");
                        let (f, o) = cache.get(key, || shapes::shape_raster(shape, pw, ph, o_px));
                        Some((None, f, o, outline.as_ref().map(|o| colour::linear_of(&o.colour))))
                    }
                    Kind::Path { nodes, outline, .. } => {
                        let o_px = outline.as_ref().map(|o| o.width_cm / 100.0 * k).unwrap_or(0.0);
                        let key = format!("p|{nodes:?}|{pw}|{ph}|{o_px:.2}");
                        let (f, o) = cache.get(key, || shapes::path_raster(nodes, pw, ph, o_px));
                        Some((None, f, o, outline.as_ref().map(|o| colour::linear_of(&o.colour))))
                    }
                    Kind::Image { image, white_clear, .. } => cx.pictures.get(image).map(|pic| {
                        let key = format!("a|{image}|{white_clear}");
                        let (p, _) = cache.get(key, || (if *white_clear { shapes::white_clear(pic) } else { (**pic).clone() }, None));
                        (Some(p.clone()), p, None, None)
                    }),
                    _ => None,
                };
                let Some((picture, fill, outline, outline_colour)) = drawn else { continue };
                let col = Colour::of(kind.colour().unwrap_or("#ffffff"), kind.gradient());
                let mut places = vec![(place.clone(), false)];
                if let Some(plane) = cx.mirror {
                    if let Some((p, flip)) = model::mirror_place(place, plane, kind.mirror_image()) {
                        places.push((p, flip));
                    }
                }
                for (k, (p, flip)) in places.into_iter().enumerate() {
                    let decal = Decal::new(&p, w, h, flip);
                    let copy = k > 0;
                    if let Some(pic) = &picture {
                        push_as(Op::Picture { decal: decal.clone(), picture: pic.clone() }, copy);
                        continue;
                    }
                    if let (Some(o), Some(oc)) = (&outline, outline_colour) {
                        push_as(Op::Mask { decal: decal.clone(), mask: o.clone(), colour: Colour::Solid(oc) }, copy);
                    }
                    push_as(Op::Mask { decal, mask: fill.clone(), colour: col.clone() }, copy);
                }
            }
        }
    }
    out
}

fn ramp(x: f32, a: f32, b: f32) -> f32 {
    ((x - a) / (b - a)).clamp(0.0, 1.0)
}

/// What an operation puts on a texel at `s`: coverage and colour.
fn eval(op: &Op, s: &Sample, dims: &BusDims, aa: f32) -> (f32, [f32; 3]) {
    match op {
        Op::Fill { colour, .. } => {
            let q = Vec2::new((s.p.y - dims.min.y) / dims.length(), (s.p.z - dims.min.z) / dims.height());
            (1.0, colour.at(q))
        }
        Op::Stripe { h1, h2, angle, wave, sides, colour } => {
            let side = model::stripe_side_weight(*sides, s.normal());
            if side <= 0.0 {
                return (0.0, [0.0; 3]);
            }
            let z = s.p.z - dims.min.z;
            let (e1, e2) = model::stripe_edges(*h1, *h2, *angle, *wave, s.p.y, dims);
            let a = ramp(z, e1 - aa, e1 + aa) * (1.0 - ramp(z, e2 - aa, e2 + aa)) * side;
            if a <= 0.0 {
                return (0.0, [0.0; 3]);
            }
            let q = Vec2::new((s.p.y - dims.min.y) / dims.length(), ((z - e1) / (e2 - e1).max(1e-3)).clamp(0.0, 1.0));
            (a, colour.at(q))
        }
        Op::Mask { decal, mask, colour } => {
            let Some((q, facing)) = decal.at(s) else { return (0.0, [0.0; 3]) };
            let m = mask.sample(q.x, q.y)[3] * facing;
            (m, if m > 0.0 { colour.at(Vec2::new(q.x, 1.0 - q.y)) } else { [0.0; 3] })
        }
        Op::Picture { decal, picture } => {
            let Some((q, facing)) = decal.at(s) else { return (0.0, [0.0; 3]) };
            let c = picture.sample(q.x, q.y);
            (c[3] * facing, [c[0], c[1], c[2]])
        }
        Op::Brush { strokes, colour } => {
            let n = s.normal();
            let mut a = 0.0f32;
            for st in strokes.iter() {
                let c = st.cover(s.p, n);
                if c <= 0.0 {
                    continue;
                }
                a = if st.erase { a * (1.0 - c) } else { a + (1.0 - a) * c };
            }
            (a, if a > 0.0 { colour.at(Vec2::ZERO) } else { [0.0; 3] })
        }
    }
}

/// How much an operation recolours zone `z` of `zones` (`BLACK`: the black a zone is shaded
/// with): a fill its group or the zones near its colour, every other layer the paint.
fn takes(op: &Op, zones: &Zones, z: u8) -> f32 {
    match op {
        Op::Fill { lab, radius, group, .. } => {
            let centre = if z == BLACK { [0.0; 3] } else { zones.centres[z as usize] };
            match group {
                Some(g) => (z != BLACK && zones.group[z as usize] == *g) as u8 as f32,
                None if *radius >= 999.0 => (z != BLACK && zones.is_paint(z as usize)) as u8 as f32,
                None => ramp(*radius + 2.0 - colour::delta_e(centre, *lab), 0.0, 4.0),
            }
        }
        _ => (z != BLACK && zones.is_paint(z as usize)) as u8 as f32,
    }
}

/// What the layers made of a texel so far (linear light): the colours of the two zones' parts it
/// is made of (`Canvas::mix`), how much of the base's own detail is left - the seams, the grain,
/// what no zone explains - and on glass how much a print covers it.
pub type Paint = [f32; 8];

/// A texel's zones' parts: (A's part t, A's colour, B's colour, A's and B's light against their
/// group's).
fn parts(zones: &Zones, mix: (u8, u8, u16)) -> (f32, [f32; 3], [f32; 3], f32, f32) {
    let (za, zb, t) = mix;
    let lin = |z: u8| if z == BLACK { [0.0; 3] } else { zones.lin[z as usize] };
    let rel = |z: u8| if z == BLACK { 1.0 } else { zones.rel[z as usize] };
    (t as f32 / T_SCALE, lin(za), lin(zb), rel(za), rel(zb))
}

/// The base as a paint: each part its zone's colour, all the detail left.
fn unpainted(zones: &Zones, mix: (u8, u8, u16)) -> Paint {
    let (_, pa, pb, _, _) = parts(zones, mix);
    [pa[0], pa[1], pa[2], pb[0], pb[1], pb[2], 1.0, 0.0]
}

/// The texel's colour (and a print's cover) from its paint: the parts by their share, and what
/// is left of the base's own detail - as it was where nothing changed, as light and dark only
/// where a new colour came (the grey-white streaks of dirt on a cream bus were white streaks on
/// a blue one).
fn finish(zones: &Zones, mix: (u8, u8, u16), c0: [f32; 3], p: &Paint) -> [f32; 4] {
    let (t, pa, pb, _, _) = parts(zones, mix);
    let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let detail: [f32; 3] = std::array::from_fn(|k| c0[k] - t * pa[k] - (1.0 - t) * pb[k]);
    let mixed: [f32; 3] = std::array::from_fn(|k| t * p[k] + (1.0 - t) * p[3 + k]);
    let was: [f32; 3] = std::array::from_fn(|k| t * pa[k] + (1.0 - t) * pb[k]);
    let moved = (0..3).map(|k| (mixed[k] - was[k]).powi(2)).sum::<f32>().sqrt();
    let w = (moved / 0.05).clamp(0.0, 1.0);
    let ratio = (1.0 + 0.6 * p[6] * lum(detail) / lum(was).max(0.02)).clamp(0.6, 1.2);
    let c: [f32; 3] = std::array::from_fn(|k| {
        let exact = p[6] * detail[k] + mixed[k];
        let light = mixed[k] * ratio;
        (exact + (light - exact) * w).clamp(0.0, 1.0)
    });
    [c[0], c[1], c[2], p[7]]
}

/// A layer's colour `n` over a texel's paint with cover `a`: on the parts of the zones `op` takes
/// (as light or as dark as they were against their group's: the shading kept), the trims as they
/// are; `detail` 0 lets the base's own detail go where it paints.
fn recolour(op: &Op, zones: &Zones, mix: (u8, u8, u16), p: &mut Paint, n: [f32; 3], a: f32, detail: f32) {
    let (t, _, _, ra, rb) = parts(zones, mix);
    let (wa, wb) = (takes(op, zones, mix.0) * a, takes(op, zones, mix.1) * a);
    if wa <= 0.0 && wb <= 0.0 {
        return;
    }
    let (ra, rb) = (1.0 + (ra - 1.0) * detail, 1.0 + (rb - 1.0) * detail);
    for k in 0..3 {
        p[k] += (n[k] * ra - p[k]) * wa;
        p[3 + k] += (n[k] * rb - p[3 + k]) * wb;
    }
    let share = (t.min(1.0) * wa + (1.0 - t.min(1.0)) * wb).clamp(0.0, 1.0);
    p[6] *= 1.0 - share * (1.0 - detail);
}

/// A layer that paints the whole texel (over the trims, or a print on glass): both parts `n`,
/// none of the base's detail left where it covers.
fn cover(p: &mut Paint, n: [f32; 3], a: f32) {
    for k in 0..3 {
        p[k] += (n[k] - p[k]) * a;
        p[3 + k] += (n[k] - p[3 + k]) * a;
    }
    p[6] *= 1.0 - a;
}

/// The layers painted over the canvas: RGBA (sRGB, the base's alpha) for its rows. `from`: the
/// paint of the operations before `skip` already painted, to start from; `keep`: hand back the
/// paint before that operation.
///
/// A layer recolours the zones it takes (`takes`): what of a texel was body becomes the new
/// colour, its shading kept by the zone's light, its seams and grain by the base's own detail;
/// what was a trim stays (an edge texel between them, half of each, half). The protected
/// texels (wheels, lamps, rubbers, mirrors: `KEEP`) keep the base. A layer over the trims, and
/// a decal (a text, a shape, a picture: where the player put it), paints the whole texel, with
/// the base's shading. The windows (`GLASS`) take only the layers
/// that go over them, as a print on the glass: in full, without the base's shading, and less
/// see-through where the print covers them (the glass's alpha is its transparency).
pub fn composite(cv: &Canvas, ops: &[Prepared], dims: &BusDims, from: Option<(&[Paint], usize)>, keep: Option<usize>) -> (Vec<u8>, Option<Vec<Paint>>) {
    let n = cv.base.len();
    let aa = 0.6 / cv.bake.density.max(1.0);
    let (start, first_op) = match from {
        Some((c, k)) if c.len() == n => (Some(c), k),
        _ => (None, 0),
    };
    // linear colour and, on glass, how much of it the paint covers
    let mut lin: Vec<[f32; 4]> = vec![[0.0; 4]; n];
    let mut kept: Option<Vec<Paint>> = keep.map(|_| vec![[0.0; 8]; n]);
    let paint_one = |i: usize, c: &mut [f32; 4], k: Option<&mut Paint>| {
        let (s1, s2) = cv.bake.samples(i);
        let b = cv.base[i];
        let c0 = [to_linear(b[0]), to_linear(b[1]), to_linear(b[2])];
        let mix = cv.mix[i];
        let mut kk = k;
        let mut pt = match start {
            Some(st) => st[i],
            None => unpainted(&cv.zones, mix),
        };
        if let Some(k) = kk.as_deref_mut() {
            *k = pt;
        }
        if s1.flags & COVERED != 0 && s1.flags & KEEP == 0 {
            let glass = s1.flags & GLASS != 0 && !(cv.alpha && b[3] >= 240);
            let outside = |s: &Sample| if s.flags & OUTSIDE != 0 { 1.0 } else { 0.0 };
            let ratio = cv.detail[i] as f32 / 100.0;
            for (j, p) in ops.iter().enumerate().skip(first_op) {
                if let (Some(stop), Some(k)) = (keep, kk.as_deref_mut()) {
                    if j == stop {
                        *k = pt;
                    }
                }
                if glass && !p.over_glass || p.copy && s2.is_some() {
                    continue;
                }
                let (mut a, mut cc) = eval(&p.op, &s1, dims, aa);
                a *= outside(&s1);
                if let Some(s2) = &s2 {
                    let (a2, c2) = eval(&p.op, s2, dims, aa);
                    let a2 = a2 * outside(s2);
                    if a2 > a {
                        (a, cc) = (a2, c2);
                    }
                }
                let a = a * p.opacity;
                if a <= 0.0 {
                    continue;
                }
                if glass {
                    cover(&mut pt, cc, a);
                    pt[7] += (1.0 - pt[7]) * a;
                } else if p.over_trim || matches!(p.op, Op::Mask { .. } | Op::Picture { .. } | Op::Brush { .. }) {
                    // (a text, a shape, a picture or the brush goes where the player put it, over
                    // a trim too)
                    let d = 1.0 + (ratio - 1.0) * p.detail;
                    cover(&mut pt, cc.map(|v| (v * d).min(1.0)), a);
                } else if let Some(tp) = cv.template.as_ref() {
                    // outside the maker's mask the zones alone (a rubber's rim in the body's
                    // colour); inside it the maker's shading where the zones say paint too (a
                    // badge the mask takes in keeps its colours)
                    let ma = tp.ma[i] as f32 / 255.0;
                    recolour(&p.op, &cv.zones, mix, &mut pt, cc, a * (1.0 - ma), p.detail);
                    // (black with a zone is that zone in shadow, which the template draws itself)
                    let t = (mix.2 as f32 / T_SCALE).min(1.0);
                    let ta = takes(&p.op, &cv.zones, mix.0);
                    let tb = if mix.1 == BLACK { ta } else { takes(&p.op, &cv.zones, mix.1) };
                    let w = a * ma * (t * ta + (1.0 - t) * tb);
                    if w > 0.0 {
                        cover(&mut pt, tp.show(i, cc, p.detail), w);
                    }
                } else {
                    recolour(&p.op, &cv.zones, mix, &mut pt, cc, a, p.detail);
                }
            }
        }
        if let (Some(stop), Some(k)) = (keep, kk) {
            if stop >= ops.len() && s1.flags & COVERED != 0 {
                *k = pt;
            }
        }
        *c = finish(&cv.zones, mix, c0, &pt);
    };
    match kept.as_mut() {
        Some(kept) => lin.par_iter_mut().zip(kept.par_iter_mut()).enumerate().for_each(|(i, (c, k))| paint_one(i, c, Some(k))),
        None => lin.par_iter_mut().enumerate().for_each(|(i, c)| paint_one(i, c, None)),
    }
    // the bleed: round the covered texels the colour next to them (and a print's cover)
    let mut alpha_from: Vec<(u32, u32)> = Vec::new();
    for &(dst, src) in &cv.bake.bleed {
        lin[dst as usize] = lin[src as usize];
        if lin[src as usize][3] > 0.0 {
            alpha_from.push((dst, src));
        }
    }
    let mut out = vec![0u8; n * 4];
    out.par_chunks_mut(4).enumerate().for_each(|(i, px)| {
        let c = lin[i];
        px[0] = to_srgb(c[0]);
        px[1] = to_srgb(c[1]);
        px[2] = to_srgb(c[2]);
        px[3] = over_alpha(cv.base[i][3], c[3]);
    });
    // (a bled texel takes the alpha its source got: the print's edge on the glass)
    for (dst, src) in alpha_from {
        out[dst as usize * 4 + 3] = over_alpha(cv.base[src as usize][3], lin[src as usize][3]);
    }
    (out, kept)
}

/// The glass's alpha (its opacity) under a print that covers `cover` of it.
fn over_alpha(base: u8, cover: f32) -> u8 {
    if cover <= 0.0 {
        return base;
    }
    let b = base as f32 / 255.0;
    ((b + (1.0 - b) * cover.min(1.0)) * 255.0 + 0.5) as u8
}

// --- the worker ------------------------------------------------------------------------------

/// What to paint: the layers and what goes with them, and the layer being dragged (the layers
/// under it are kept painted between frames).
pub struct Job {
    pub seq: u64,
    pub layers: Vec<Layer>,
    pub cx: Context,
    /// (index of the layer being changed, a key of everything under it)
    pub moving: Option<(usize, u64)>,
}

pub struct Done {
    pub seq: u64,
    /// RGBA per canvas.
    pub pictures: Vec<Vec<u8>>,
}

/// The painter's thread: takes the latest job, paints every canvas, sends the pictures back.
pub struct Worker {
    tx: std::sync::mpsc::Sender<Job>,
    pub rx: std::sync::mpsc::Receiver<Done>,
}

impl Worker {
    pub fn start(canvases: Arc<Vec<Canvas>>, dims: BusDims) -> Worker {
        let (tx, jobs) = std::sync::mpsc::channel::<Job>();
        let (done, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("livery painter".into())
            .spawn(move || {
                let mut cache = RasterCache::default();
                // the paint under the layer being dragged, per canvas: (key, op count, colours)
                let mut under: Vec<Option<(u64, usize, Vec<Paint>)>> = (0..canvases.len()).map(|_| None).collect();
                while let Ok(mut job) = jobs.recv() {
                    while let Ok(j) = jobs.try_recv() {
                        job = j;
                    }
                    let ops = prepare(&job.layers, &job.cx, &mut cache);
                    let mut pictures = Vec::with_capacity(canvases.len());
                    for (ci, cv) in canvases.iter().enumerate() {
                        let split = job.moving.map(|(layer, key)| (ops.iter().position(|p| p.layer >= layer).unwrap_or(ops.len()), key));
                        let reuse = match (&under[ci], split) {
                            (Some((k, count, c)), Some((at, key))) if *k == key && *count == at => Some((c.as_slice(), at)),
                            _ => None,
                        };
                        if reuse.is_some() {
                            let (rgba, _) = composite(cv, &ops, &dims, reuse, None);
                            pictures.push(rgba);
                        } else {
                            let (rgba, kept) = composite(cv, &ops, &dims, None, split.map(|s| s.0));
                            if let (Some((at, key)), Some(kept)) = (split, kept) {
                                under[ci] = Some((key, at, kept));
                            } else {
                                under[ci] = None;
                            }
                            pictures.push(rgba);
                        }
                    }
                    if done.send(Done { seq: job.seq, pictures }).is_err() {
                        break;
                    }
                }
            })
            .ok();
        Worker { tx, rx }
    }

    pub fn send(&self, job: Job) {
        let _ = self.tx.send(job);
    }
}

#[cfg(test)]
mod tests {
    use super::super::bake::tests::box_bus;
    use super::super::bake::Outside;
    use super::super::model::{layer, Coupling, Side};
    use super::*;

    fn setup(w: u32, h: u32) -> (Canvas, BusDims, Vec<u8>) {
        let g = box_bus();
        let out = Outside::build(&g.tris);
        // a blue base with a black strip (rubber) along the bottom rows, alpha 100 (a mask)
        let mut base = Vec::new();
        for y in 0..h {
            for _ in 0..w {
                base.extend_from_slice(if y >= h - 2 { &[20, 20, 20, 100] } else { &[30, 60, 160, 100] });
            }
        }
        let probe = Bake::build(&g.tris, 0, w, h, 0, h, &out);
        let zones = Zones::of(&outside_colours(&probe, &base));
        (Canvas::build(&g.tris, 0, w, h, 0, h, &base, &out, &zones), g.dims, base)
    }

    fn cx(dims: BusDims) -> Context {
        let _ = dims;
        Context { pictures: Arc::new(HashMap::new()), mirror: Some(0.0), density: 6.0, max_px: 512 }
    }

    fn px(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let o = ((y * w + x) * 4) as usize;
        [rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3]]
    }

    #[test]
    fn a_base_colour_paints_the_paint_and_keeps_the_alpha_and_the_rubbers() {
        let (cv, dims, _) = setup(64, 32);
        let mut l = layer("Base", model::base_colour("#ff0000"));
        l.detail = 0.0;
        let ops = prepare(&[l], &cx(dims), &mut RasterCache::default());
        let (rgba, _) = composite(&cv, &ops, &dims, None, None);
        assert_eq!(px(&rgba, 64, 40, 10), [255, 0, 0, 100], "red, its alpha the base's");
        assert_eq!(px(&rgba, 64, 40, 31), [20, 20, 20, 100], "the rubber strip is no paint zone");
        // over the trim too when asked
        let mut l = layer("Base", model::base_colour("#ff0000"));
        l.over_trim = true;
        l.detail = 0.0;
        let ops = prepare(&[l], &cx(dims), &mut RasterCache::default());
        let (rgba, _) = composite(&cv, &ops, &dims, None, None);
        assert_eq!(px(&rgba, 64, 40, 31), [255, 0, 0, 100]);
    }

    #[test]
    fn a_stripe_lies_at_its_height_and_the_detail_keeps_the_shading() {
        let (cv, dims, _) = setup(64, 32);
        // 0.3 .. 1.5 m above the box's bottom
        let mut l = layer("Band", Kind::Stripe { template: model::StripeTemplate::Skirt, h1: 0.3, h2: 1.5, angle: 0.0, wave: 0.0, sides: model::Sides::All, colour: "#ffffff".into(), gradient: None });
        l.detail = 0.0;
        let ops = prepare(&[l], &cx(dims), &mut RasterCache::default());
        let (rgba, _) = composite(&cv, &ops, &dims, None, None);
        // texel row y: height above the bottom = 3 * (1 - (y + 0.5) / 32)
        let row = |hgt: f32| (32.0 * (1.0 - hgt / 3.0) - 0.5).round() as u32;
        assert_eq!(px(&rgba, 64, 40, row(0.9)), [255, 255, 255, 100]);
        assert_eq!(px(&rgba, 64, 40, row(2.2)), [30, 60, 160, 100]);
        assert_eq!(px(&rgba, 64, 10, row(0.9)), [255, 255, 255, 100], "on both sides");
    }

    #[test]
    fn a_decal_lies_on_its_side_and_its_mirror_on_the_other() {
        let (cv, dims, _) = setup(128, 64);
        let mut place = Place::new(Side::R, Vec3::new(1.25, 2.0, 1.0), 2.0);
        place.height_m = Some(1.0);
        let mut l = layer("Box", Kind::Shape { shape: "rechthoek".into(), colour: "#00ff00".into(), outline: None, gradient: None, place: place.clone() });
        l.detail = 0.0;
        let ops = prepare(std::slice::from_ref(&l), &cx(dims), &mut RasterCache::default());
        assert_eq!(ops.len(), 2, "the shape and its mirror image");
        let (rgba, _) = composite(&cv, &ops, &dims, None, None);
        // on the right side: u from rear (-6) to front (+6) over x 64..128; 2.0 m along → x 106.7
        let x = (64.0 + (2.0 + 6.0) / 12.0 * 64.0) as u32;
        let y = (64.0 * (1.0 - (1.0 - 0.3) / 3.0)) as u32;
        assert_eq!(px(&rgba, 128, x, y), [0, 255, 0, 100]);
        assert_eq!(px(&rgba, 128, x - 20, y), [30, 60, 160, 100]);
        // the left side runs from the front (+6) at u 0 to the rear (-6): y = 2.0 at x 21.3
        let xl = ((6.0 - 2.0) / 12.0 * 64.0) as u32;
        assert_eq!(px(&rgba, 128, xl, y), [0, 255, 0, 100]);
        // uncoupled: one side only
        if let Some(p) = l.kind.place_mut() {
            p.mirror = Coupling::Single;
        }
        let ops = prepare(&[l], &cx(dims), &mut RasterCache::default());
        let (rgba, _) = composite(&cv, &ops, &dims, None, None);
        assert_eq!(px(&rgba, 128, xl, y), [30, 60, 160, 100]);
    }

    #[test]
    fn the_paint_under_a_dragged_layer_is_kept_and_gives_the_same_picture() {
        let (cv, dims, _) = setup(64, 32);
        let mut ls = vec![layer("Base", model::base_colour("#204060"))];
        ls.push(layer("Band", model::stripe(model::StripeTemplate::WindowBand, "#ffcc00", &dims)));
        let mut place = Place::new(Side::R, Vec3::new(1.25, 0.0, 1.0), 1.5);
        place.height_m = Some(0.5);
        ls.push(layer("Star", Kind::Shape { shape: "ster".into(), colour: "#ff0000".into(), outline: Some(model::Outline { colour: "#000000".into(), width_cm: 5.0 }), gradient: None, place }));
        let mut cache = RasterCache::default();
        let ops = prepare(&ls, &cx(dims), &mut cache);
        let split = ops.iter().position(|p| p.layer >= 2).unwrap();
        let (full, kept) = composite(&cv, &ops, &dims, None, Some(split));
        let kept = kept.unwrap();
        let (again, _) = composite(&cv, &ops, &dims, Some((&kept, split)), None);
        assert_eq!(full, again);
    }

    /// Where (0..1 across its own box) the content of `d` lies at `p` on its side.
    fn across(d: &Decal, p: Vec3, n: Vec3) -> f32 {
        let q = |v: f32| (v * 127.0) as i8;
        let s = Sample { p, n: [q(n.x), q(n.y), q(n.z)], flags: COVERED | OUTSIDE };
        d.at(&s).expect("on the decal").0.x
    }

    #[test]
    fn the_copy_of_a_text_or_a_picture_reads_the_right_way_a_shape_s_is_mirrored() {
        let (_, dims, _) = setup(16, 8);
        let mut place = Place::new(Side::R, Vec3::new(1.25, 2.0, 1.4), 2.0);
        place.height_m = Some(0.5);
        let text = Kind::Text { text: "Bus".into(), font: shapes::DEFAULT_FONT.into(), height_cm: 30.0, colour: "#fff".into(), outline: None, spacing: 0.0, gradient: None, place: place.clone() };
        let shape = Kind::Shape { shape: "pijl".into(), colour: "#fff".into(), outline: None, gradient: None, place: place.clone() };
        let ops = prepare(&[layer("T", text), layer("S", shape)], &cx(dims), &mut RasterCache::default());
        let decals: Vec<&Decal> = ops.iter().filter_map(|o| match &o.op {
            Op::Mask { decal, .. } => Some(decal),
            _ => None,
        }).collect();
        assert_eq!(decals.len(), 4, "each with its copy");
        let (t, tc, s, sc) = (decals[0], decals[1], decals[2], decals[3]);
        let (rear, front) = (2.0 - 0.4 * t.w, 2.0 + 0.4 * t.w);
        // the original on the right, seen from outside: its first letter at the rear (-y)
        assert!(across(t, Vec3::new(1.25, rear, 1.4), Vec3::X) < 0.2);
        // the text's copy on the left, seen from outside: its first letter at the front (+y), the
        // viewer's left there - readable
        assert!(!tc.flip && tc.n == Vec3::NEG_X);
        assert!(across(tc, Vec3::new(-1.25, front, 1.4), Vec3::NEG_X) < 0.2);
        assert!(across(tc, Vec3::new(-1.25, rear, 1.4), Vec3::NEG_X) > 0.8);
        // the shape's copy is its mirror image: the arrow's tip at the same end of the bus
        assert!(sc.flip);
        let front = 2.0 + 0.4 * s.w;
        assert!(across(s, Vec3::new(1.25, front, 1.4), Vec3::X) > 0.8);
        assert!(across(sc, Vec3::new(-1.25, front, 1.4), Vec3::NEG_X) > 0.8);
    }

    /// The box bus with its right side a window (one triangle) and a wheel (the other).
    fn glass_and_wheel() -> (Canvas, BusDims) {
        let mut g = box_bus();
        g.tris[2].glass = true;
        g.tris[3].keep = true;
        let out = Outside::build(&g.tris);
        let (w, h) = (64, 32);
        let base: Vec<u8> = (0..w * h).flat_map(|_| [30u8, 60, 160, 100]).collect();
        let zones = Zones::of(&[[30, 60, 160]]);
        (Canvas::build(&g.tris, 0, w, h, 0, h, &base, &out, &zones), g.dims)
    }

    #[test]
    fn a_base_colour_leaves_the_windows_and_the_wheels_a_print_goes_onto_the_glass() {
        let (cv, dims) = glass_and_wheel();
        let mut l = layer("Base", model::base_colour("#ff0000"));
        l.detail = 0.0;
        l.over_trim = true;
        let (rgba, _) = composite(&cv, &prepare(std::slice::from_ref(&l), &cx(dims), &mut RasterCache::default()), &dims, None, None);
        assert_eq!(px(&rgba, 64, 10, 30), [255, 0, 0, 100], "the body (under the window pane)");
        assert_eq!(px(&rgba, 64, 60, 28), [30, 60, 160, 100], "the window keeps its own");
        assert_eq!(px(&rgba, 64, 34, 3), [30, 60, 160, 100], "the wheel keeps its own, even over the trim");
        // over the windows: painted, less see-through; the wheel still not
        l.over_glass = true;
        let (rgba, _) = composite(&cv, &prepare(&[l], &cx(dims), &mut RasterCache::default()), &dims, None, None);
        assert_eq!(px(&rgba, 64, 60, 28), [255, 0, 0, 255], "a full print on the glass");
        assert_eq!(px(&rgba, 64, 34, 3), [30, 60, 160, 100]);
        // half a print: half way to opaque
        let mut half = layer("Foil", model::base_colour("#ffffff"));
        half.over_glass = true;
        half.opacity = 0.5;
        half.detail = 0.0;
        let (rgba, _) = composite(&cv, &prepare(&[half], &cx(dims), &mut RasterCache::default()), &dims, None, None);
        let a = px(&rgba, 64, 60, 28)[3];
        assert!((175..=180).contains(&a), "{a}");
    }

    /// A yellow body, shaded at one end (from 0.6 of its light to all of it), with a black trim (the
    /// top rows), an anti-aliased edge between them (half of each, in linear light) and a grey
    /// part (rows 24..28).
    fn yellow_bus(w: u32, h: u32) -> (Canvas, BusDims, Vec<u8>) {
        let g = box_bus();
        let out = Outside::build(&g.tris);
        let yellow = [to_linear(240), to_linear(200), to_linear(20)];
        let black = [to_linear(22), to_linear(22), to_linear(24)];
        let mut base = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let shade = 0.6 + 0.4 * ((x % (w / 2)) as f32 / 7.0).min(1.0);
                let c: [f32; 3] = match y {
                    0..4 => black,
                    4 => std::array::from_fn(|k| yellow[k] * 0.5 + black[k] * 0.5),
                    24..28 => [to_linear(120), to_linear(120), to_linear(118)],
                    _ => yellow.map(|v| v * shade),
                };
                base.extend_from_slice(&[to_srgb(c[0]), to_srgb(c[1]), to_srgb(c[2]), 255]);
            }
        }
        let probe = Bake::build(&g.tris, 0, w, h, 0, h, &out);
        let zones = Zones::of(&outside_colours(&probe, &base));
        (Canvas::build(&g.tris, 0, w, h, 0, h, &base, &out, &zones), g.dims, base)
    }

    #[test]
    fn a_base_colour_recolours_the_body_keeps_its_shading_and_leaves_the_trims() {
        let (cv, dims, base) = yellow_bus(64, 32);
        let ops = prepare(&[layer("Base", model::base_colour("#2147b8"))], &cx(dims), &mut RasterCache::default());
        let (rgba, _) = composite(&cv, &ops, &dims, None, None);
        let lin = |p: [u8; 4]| [to_linear(p[0]), to_linear(p[1]), to_linear(p[2])];
        let blue = colour::linear_of("#2147b8");
        // the trims as they were
        for (x, y) in [(10, 1), (40, 2), (10, 25), (50, 26)] {
            assert_eq!(px(&rgba, 64, x, y), px(&base, 64, x, y), "{x},{y}");
        }
        // the body blue, its shading kept: dark where it was dark
        let lit = lin(px(&rgba, 64, 31, 12));
        let dark = lin(px(&rgba, 64, 0, 12));
        for k in 0..3 {
            assert!((lit[k] - blue[k]).abs() < 0.03, "lit {lit:?} {blue:?}");
            assert!((dark[k] - blue[k] * 0.6).abs() < 0.03, "shaded {dark:?}");
        }
        // the edge half blue, half black: no yellow left round the trim
        let edge = lin(px(&rgba, 64, 31, 4));
        for k in 0..3 {
            assert!((edge[k] - (blue[k] * 0.5 + to_linear(22) * 0.5)).abs() < 0.04, "edge {edge:?}");
        }
        // and a soft step down from the body through the edge into the trim, no hard one
        let rows: Vec<f32> = (2..7).map(|y| lin(px(&rgba, 64, 31, y))[2]).collect();
        assert!(rows.windows(2).all(|w| w[1] >= w[0] - 1e-3), "{rows:?}");
        assert!(rows[2] > rows[1] + 0.05 && rows[3] > rows[2] + 0.05, "{rows:?}");
    }

    #[test]
    fn a_stripe_skips_the_trims_unless_it_goes_over_them() {
        let (cv, dims, base) = yellow_bus(64, 32);
        // over the whole height
        let band = |over: bool| {
            let mut l = layer("Band", Kind::Stripe { template: model::StripeTemplate::Skirt, h1: -1.0, h2: 5.0, angle: 0.0, wave: 0.0, sides: model::Sides::All, colour: "#ffffff".into(), gradient: None });
            l.over_trim = over;
            l
        };
        let (rgba, _) = composite(&cv, &prepare(&[band(false)], &cx(dims), &mut RasterCache::default()), &dims, None, None);
        assert_eq!(px(&rgba, 64, 40, 1), px(&base, 64, 40, 1), "the black trim kept");
        assert_eq!(px(&rgba, 64, 40, 25), px(&base, 64, 40, 25), "the grey part kept");
        assert!(px(&rgba, 64, 63, 12)[2] > 240, "the body white");
        let (rgba, _) = composite(&cv, &prepare(&[band(true)], &cx(dims), &mut RasterCache::default()), &dims, None, None);
        assert!(px(&rgba, 64, 40, 25)[2] > 200, "over the trims: the grey part white too");
    }

    #[test]
    fn the_brush_paints_along_its_stroke_and_the_eraser_takes_it_away() {
        let (cv, dims, base) = setup(128, 64);
        // a stroke along the right side, 1.0 m up, from y -2 to 2, 20 cm wide
        let stroke = |erase: bool, y0: f32, y1: f32| model::Stroke { points: vec![[1.25, y0, 1.3], [1.25, y1, 1.3]], normals: vec![[1.0, 0.0, 0.0]; 2], radius_cm: 10.0, hardness: 1.0, opacity: 1.0, erase };
        let mut l = layer("Brush", Kind::Brush { colour: "#ff0000".into(), strokes: vec![stroke(false, -2.0, 2.0)] });
        l.detail = 0.0;
        let ops = prepare(std::slice::from_ref(&l), &Context { mirror: None, ..cx(dims) }, &mut RasterCache::default());
        let (rgba, _) = composite(&cv, &ops, &dims, None, None);
        // the right side: u from rear (-6) to front (+6) over x 64..128; z = 0.3 + 3 (1 - (y + 0.5) / 64)
        let (x, y) = (96, (64.0 * (1.0 - (1.3 - 0.3) / 3.0)) as u32);
        assert_eq!(px(&rgba, 128, x, y)[..3], [255, 0, 0], "on the stroke");
        assert_eq!(px(&rgba, 128, x, y - 12), px(&base, 128, x, y - 12), "half a metre above it");
        assert_eq!(px(&rgba, 128, 10, y), px(&base, 128, 10, y), "not on the left side");
        // the eraser over its front half
        if let Kind::Brush { strokes, .. } = &mut l.kind {
            strokes.push(stroke(true, 0.0, 2.5));
        }
        let ops = prepare(std::slice::from_ref(&l), &Context { mirror: None, ..cx(dims) }, &mut RasterCache::default());
        let (rgba, _) = composite(&cv, &ops, &dims, None, None);
        let front = 64 + ((1.0 + 6.0) / 12.0 * 64.0) as u32;
        let rear = 64 + ((-1.0 + 6.0) / 12.0 * 64.0) as u32;
        assert_eq!(px(&rgba, 128, front, y), px(&base, 128, front, y), "erased");
        assert_eq!(px(&rgba, 128, rear, y)[..3], [255, 0, 0], "kept");
        // with the mirror on, on the left side too
        let ops = prepare(std::slice::from_ref(&l), &cx(dims), &mut RasterCache::default());
        let (rgba, _) = composite(&cv, &ops, &dims, None, None);
        let left_rear = ((6.0 - -1.0) / 12.0 * 64.0) as u32;
        assert_eq!(px(&rgba, 128, left_rear, y)[..3], [255, 0, 0]);
    }

    #[test]
    fn a_text_is_drawn_once_for_the_same_letters() {
        let (_, dims, _) = setup(16, 8);
        let place = Place::new(Side::R, Vec3::new(1.25, 0.0, 1.0), 1.0);
        let l = layer("T", Kind::Text { text: "Bus".into(), font: shapes::DEFAULT_FONT.into(), height_cm: 30.0, colour: "#fff".into(), outline: None, spacing: 0.0, gradient: None, place });
        let mut cache = RasterCache::default();
        let a = prepare(std::slice::from_ref(&l), &cx(dims), &mut cache);
        let b = prepare(&[l], &cx(dims), &mut cache);
        let (Op::Mask { mask: ma, decal: da, .. }, Op::Mask { mask: mb, .. }) = (&a[0].op, &b[0].op) else { panic!() };
        assert!(Arc::ptr_eq(ma, mb));
        assert!(!da.flip, "a text's copy is not mirrored");
        let Op::Mask { decal: copy, .. } = &a[1].op else { panic!() };
        assert!(!copy.flip && copy.n == Vec3::NEG_X);
    }
}
