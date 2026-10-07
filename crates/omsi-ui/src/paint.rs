//! Shapes as triangles: a list of [`Vertex`] the [`crate::gpu`] pipeline draws.
//!
//! A vertex is a point plus an extrusion: the pipeline moves it by `ext * width`. In
//! pixel space `width.x` is pixels. In world space `width` is (metres, pixels) and the
//! larger of the two at the point's depth is used, so a road keeps its true width near
//! the camera and never thins below a few pixels far away, a marker can be sized in
//! pixels and still stand on its spot - without rebuilding anything when the view zooms.

use glam::{Vec2, Vec3};

use crate::atlas::{Atlas, Sprite};
use crate::text::{Fonts, Weight};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub ext: [f32; 2],
    pub width: [f32; 2],
    pub uv: [f32; 2],
    /// sRGB, straight alpha.
    pub color: [f32; 4],
    /// x: 0 pixels, 1 world; y: 1 = multiplied by the texture.
    pub mode: [f32; 2],
}

/// sRGB colour with straight alpha, 0..1.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Color(pub [f32; 4]);

impl Color {
    pub const WHITE: Color = Color([1.0, 1.0, 1.0, 1.0]);
    pub const BLACK: Color = Color([0.0, 0.0, 0.0, 1.0]);
    pub const CLEAR: Color = Color([0.0, 0.0, 0.0, 0.0]);

    /// From `0xRRGGBB`.
    pub const fn hex(rgb: u32) -> Color {
        Color([((rgb >> 16) & 255) as f32 / 255.0, ((rgb >> 8) & 255) as f32 / 255.0, (rgb & 255) as f32 / 255.0, 1.0])
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
        Color([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a])
    }
    pub fn alpha(self, a: f32) -> Color {
        Color([self.0[0], self.0[1], self.0[2], self.0[3] * a])
    }
    pub fn mix(self, o: Color, t: f32) -> Color {
        let m = |a: f32, b: f32| a + (b - a) * t;
        Color([m(self.0[0], o.0[0]), m(self.0[1], o.0[1]), m(self.0[2], o.0[2]), m(self.0[3], o.0[3])])
    }
    pub fn lighten(self, t: f32) -> Color {
        self.mix(Color([1.0, 1.0, 1.0, self.0[3]]), t)
    }
    pub fn darken(self, t: f32) -> Color {
        self.mix(Color([0.0, 0.0, 0.0, self.0[3]]), t)
    }
}

/// Axis-aligned box in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn center(&self) -> Vec2 {
        Vec2::new(self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.right() && p.y < self.bottom()
    }
    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
    pub fn pad(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, (self.w - 2.0 * dx).max(0.0), (self.h - 2.0 * dy).max(0.0))
    }
    /// Split off the left `w` pixels: (left, rest).
    pub fn cut_left(&self, w: f32) -> (Rect, Rect) {
        (Rect::new(self.x, self.y, w, self.h), Rect::new(self.x + w, self.y, (self.w - w).max(0.0), self.h))
    }
    pub fn cut_right(&self, w: f32) -> (Rect, Rect) {
        (Rect::new(self.right() - w, self.y, w, self.h), Rect::new(self.x, self.y, (self.w - w).max(0.0), self.h))
    }
    pub fn cut_top(&self, h: f32) -> (Rect, Rect) {
        (Rect::new(self.x, self.y, self.w, h), Rect::new(self.x, self.y + h, self.w, (self.h - h).max(0.0)))
    }
    pub fn cut_bottom(&self, h: f32) -> (Rect, Rect) {
        (Rect::new(self.x, self.bottom() - h, self.w, h), Rect::new(self.x, self.y, self.w, (self.h - h).max(0.0)))
    }
}

/// Horizontal placement of text relative to the point given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// The outline of a rounded box (clockwise in screen space), `n` points per corner.
pub fn rounded_outline(r: Rect, radius: f32) -> Vec<Vec2> {
    let rad = radius.min(r.w * 0.5).min(r.h * 0.5).max(0.0);
    if rad < 0.5 {
        return vec![Vec2::new(r.x, r.y), Vec2::new(r.right(), r.y), Vec2::new(r.right(), r.bottom()), Vec2::new(r.x, r.bottom())];
    }
    let n = ((rad * 0.6) as usize).clamp(3, 12);
    let corners = [
        (Vec2::new(r.right() - rad, r.y + rad), -90.0f32),
        (Vec2::new(r.right() - rad, r.bottom() - rad), 0.0),
        (Vec2::new(r.x + rad, r.bottom() - rad), 90.0),
        (Vec2::new(r.x + rad, r.y + rad), 180.0),
    ];
    let mut out = Vec::with_capacity(4 * (n + 1));
    for (c, a0) in corners {
        for k in 0..=n {
            let a = (a0 + 90.0 * k as f32 / n as f32).to_radians();
            out.push(c + Vec2::new(a.cos(), a.sin()) * rad);
        }
    }
    out
}

/// A path in pixels for [`Painter::stroke`]: a polyline (curves are flattened as they are
/// added, to a tenth of a pixel) with the distance along it at every point - so a stroke can
/// be drawn up to a share of its length (a line drawing itself), and a thing put where the
/// pen is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    pts: Vec<Vec2>,
    dist: Vec<f32>,
}

/// How far a flattened curve may stray from the true one (pixels).
const FLATNESS: f32 = 0.1;

impl Path {
    pub fn new(start: Vec2) -> Path {
        Path { pts: vec![start], dist: vec![0.0] }
    }
    pub fn points(&self) -> &[Vec2] {
        &self.pts
    }
    /// The length (pixels).
    pub fn length(&self) -> f32 {
        self.dist.last().copied().unwrap_or(0.0)
    }
    /// Where the path is now (its last point).
    pub fn end(&self) -> Vec2 {
        self.pts.last().copied().unwrap_or(Vec2::ZERO)
    }
    fn push(&mut self, p: Vec2) {
        let last = self.end();
        let d = last.distance(p);
        if d > 1e-4 || self.pts.is_empty() {
            let at = self.length() + d;
            self.pts.push(p);
            self.dist.push(at);
        }
    }
    /// On to `p` in a straight line.
    pub fn line_to(&mut self, p: Vec2) -> &mut Path {
        self.push(p);
        self
    }
    /// On to `p` along the cubic Bézier curve with control points `c1` and `c2` (Wang's bound
    /// on the segments it takes to stay within `FLATNESS`).
    pub fn cubic_to(&mut self, c1: Vec2, c2: Vec2, p: Vec2) -> &mut Path {
        let p0 = self.end();
        let m = (p0 - 2.0 * c1 + c2).length().max((c1 - 2.0 * c2 + p).length());
        let n = ((0.75 * m / FLATNESS).sqrt().ceil() as usize).clamp(1, 512);
        for k in 1..=n {
            let t = k as f32 / n as f32;
            let u = 1.0 - t;
            self.push(p0 * (u * u * u) + c1 * (3.0 * u * u * t) + c2 * (3.0 * u * t * t) + p * (t * t * t));
        }
        self
    }
    /// Round `center` from where the path is, turning by `sweep` radians (clockwise on the
    /// screen when positive).
    pub fn arc_around(&mut self, center: Vec2, sweep: f32) -> &mut Path {
        let from = self.end() - center;
        let r = from.length();
        if r < 1e-4 {
            return self;
        }
        // (the most a chord may turn and stay within `FLATNESS` of the circle)
        let step = 2.0 * (1.0 - FLATNESS.min(r) / r).acos().max(1e-3);
        let n = ((sweep.abs() / step).ceil() as usize).clamp(1, 2048);
        for k in 1..=n {
            self.push(center + Vec2::from_angle(sweep * k as f32 / n as f32).rotate(from));
        }
        self
    }
    /// The segment `s` pixels along lies on, and how far into it.
    fn locate(&self, s: f32) -> (usize, f32) {
        if self.pts.len() < 2 {
            return (0, 0.0);
        }
        let s = s.clamp(0.0, self.length());
        let i = self.dist.partition_point(|&d| d <= s).clamp(1, self.pts.len() - 1) - 1;
        let len = self.dist[i + 1] - self.dist[i];
        (i, if len > 0.0 { (s - self.dist[i]) / len } else { 0.0 })
    }
    /// The point `s` pixels along (held at the ends), and the way the path goes there.
    pub fn at(&self, s: f32) -> (Vec2, Vec2) {
        if self.pts.len() < 2 {
            return (self.end(), Vec2::X);
        }
        let (i, f) = self.locate(s);
        let (a, b) = (self.pts[i], self.pts[i + 1]);
        (a.lerp(b, f), (b - a).normalize_or(Vec2::X))
    }
    /// The path from `a` to `b` pixels along, cut exactly at both.
    pub fn part(&self, a: f32, b: f32) -> Vec<Vec2> {
        let (a, b) = (a.clamp(0.0, self.length()), b.clamp(0.0, self.length()));
        if self.pts.len() < 2 || b < a {
            return Vec::new();
        }
        let mut out = vec![self.at(a).0];
        let (ia, _) = self.locate(a);
        for i in ia + 1..self.pts.len() {
            if self.dist[i] >= b {
                break;
            }
            if self.dist[i] > a {
                out.push(self.pts[i]);
            }
        }
        out.push(self.at(b).0);
        out
    }
}

pub struct Painter {
    pub verts: Vec<Vertex>,
    /// Physical pixels per unit (a Retina window draws in points at 2): text and icons are
    /// rasterised at this many times their size and snapped to physical pixels.
    pub scale: f32,
}

impl Default for Painter {
    fn default() -> Self {
        Painter::new()
    }
}

impl Painter {
    pub fn new() -> Painter {
        Painter { verts: Vec::new(), scale: 1.0 }
    }
    pub fn with_scale(scale: f32) -> Painter {
        Painter { verts: Vec::new(), scale: scale.max(0.25) }
    }
    fn snap(&self, v: f32) -> f32 {
        (v * self.scale).round() / self.scale
    }
    pub fn len(&self) -> u32 {
        self.verts.len() as u32
    }
    pub fn is_empty(&self) -> bool {
        self.verts.is_empty()
    }
    pub fn clear(&mut self) {
        self.verts.clear();
    }

    fn px(p: Vec2, c: Color) -> Vertex {
        Vertex { pos: [p.x, p.y, 0.0], color: c.0, ..Default::default() }
    }

    /// A triangle in pixels with a colour per corner.
    pub fn tri(&mut self, a: Vec2, b: Vec2, c: Vec2, ca: Color, cb: Color, cc: Color) {
        self.verts.extend([Self::px(a, ca), Self::px(b, cb), Self::px(c, cc)]);
    }

    /// A convex polygon in pixels.
    pub fn convex(&mut self, pts: &[Vec2], c: Color) {
        for k in 1..pts.len().saturating_sub(1) {
            self.tri(pts[0], pts[k], pts[k + 1], c, c, c);
        }
    }

    pub fn rect(&mut self, r: Rect, c: Color) {
        self.convex(&[Vec2::new(r.x, r.y), Vec2::new(r.right(), r.y), Vec2::new(r.right(), r.bottom()), Vec2::new(r.x, r.bottom())], c);
    }

    /// A box with a vertical gradient, top to bottom.
    pub fn gradient(&mut self, r: Rect, top: Color, bottom: Color) {
        let (a, b, c, d) = (Vec2::new(r.x, r.y), Vec2::new(r.right(), r.y), Vec2::new(r.right(), r.bottom()), Vec2::new(r.x, r.bottom()));
        self.tri(a, b, c, top, top, bottom);
        self.tri(a, c, d, top, bottom, bottom);
    }

    /// A box with a horizontal gradient, left to right.
    pub fn gradient_h(&mut self, r: Rect, left: Color, right: Color) {
        let (a, b, c, d) = (Vec2::new(r.x, r.y), Vec2::new(r.right(), r.y), Vec2::new(r.right(), r.bottom()), Vec2::new(r.x, r.bottom()));
        self.tri(a, b, c, left, right, right);
        self.tri(a, c, d, left, right, left);
    }

    pub fn rounded(&mut self, r: Rect, radius: f32, c: Color) {
        let o = rounded_outline(r, radius);
        let m = r.center();
        for k in 0..o.len() {
            self.tri(m, o[k], o[(k + 1) % o.len()], c, c, c);
        }
    }

    /// A rounded box shaded from `top` to `bottom`.
    pub fn rounded_gradient(&mut self, r: Rect, radius: f32, top: Color, bottom: Color) {
        let o = rounded_outline(r, radius);
        let m = r.center();
        let col = |p: Vec2| top.mix(bottom, ((p.y - r.y) / r.h.max(1.0)).clamp(0.0, 1.0));
        for k in 0..o.len() {
            let (a, b) = (o[k], o[(k + 1) % o.len()]);
            self.tri(m, a, b, col(m), col(a), col(b));
        }
    }

    /// The band between two outlines of the same number of points.
    fn band(&mut self, inner: &[Vec2], outer: &[Vec2], ci: Color, co: Color) {
        let n = inner.len().min(outer.len());
        for k in 0..n {
            let j = (k + 1) % n;
            self.tri(inner[k], outer[k], outer[j], ci, co, co);
            self.tri(inner[k], outer[j], inner[j], ci, co, ci);
        }
    }

    /// A rounded frame `t` pixels wide inside `r`.
    pub fn rounded_border(&mut self, r: Rect, radius: f32, t: f32, c: Color) {
        let outer = rounded_outline(r, radius);
        let inner = rounded_outline(r.inset(t), (radius - t).max(0.0));
        if outer.len() == inner.len() {
            self.band(&inner, &outer, c, c);
        } else {
            // the inner corners are too small to be round: the outer one's points, pulled in
            let m = r.center();
            let inner: Vec<Vec2> = outer.iter().map(|p| *p + (m - *p).signum() * t).collect();
            self.band(&inner, &outer, c, c);
        }
    }

    /// A soft shadow under a rounded box: `c` inside, fading out over `blur` pixels.
    pub fn shadow(&mut self, r: Rect, radius: f32, blur: f32, c: Color) {
        let inner_r = r.inset(blur * 0.5);
        let outer_r = r.inset(-blur * 0.5);
        let rad_i = (radius - blur * 0.5).max(0.0);
        let inner = rounded_outline(inner_r, rad_i.max(blur * 0.25));
        let outer = rounded_outline(outer_r, rad_i.max(blur * 0.25) + blur);
        if inner.len() == outer.len() {
            self.band(&inner, &outer, c, c.alpha(0.0));
            let m = r.center();
            for k in 0..inner.len() {
                self.tri(m, inner[k], inner[(k + 1) % inner.len()], c, c, c);
            }
        } else {
            self.rounded(r, radius, c);
        }
    }

    pub fn circle(&mut self, center: Vec2, radius: f32, c: Color) {
        let n = ((radius * 0.9) as usize).clamp(12, 72);
        // (the points once, the last triangle closing on the first point itself: worked out
        // again at a full turn it lay a hair off it, and a sample on that seam fell through -
        // a hairline across a filled circle)
        let pts: Vec<Vec2> = (0..n)
            .map(|k| {
                let a = std::f32::consts::TAU * k as f32 / n as f32;
                center + Vec2::new(a.cos(), a.sin()) * radius
            })
            .collect();
        for k in 0..n {
            self.tri(center, pts[k], pts[(k + 1) % n], c, c, c);
        }
    }

    /// A ring between `r0` and `r1`, from angle `a0` to `a1` (radians, 0 = right,
    /// clockwise on screen) - a full circle for 0..TAU.
    pub fn arc(&mut self, center: Vec2, r0: f32, r1: f32, a0: f32, a1: f32, c: Color) {
        let n = (((a1 - a0).abs() * r1 * 0.35) as usize).clamp(6, 96);
        for k in 0..n {
            let t0 = a0 + (a1 - a0) * k as f32 / n as f32;
            let t1 = a0 + (a1 - a0) * (k + 1) as f32 / n as f32;
            let (d0, d1) = (Vec2::new(t0.cos(), t0.sin()), Vec2::new(t1.cos(), t1.sin()));
            let (i0, o0, i1, o1) = (center + d0 * r0, center + d0 * r1, center + d1 * r0, center + d1 * r1);
            self.tri(i0, o0, o1, c, c, c);
            self.tri(i0, o1, i1, c, c, c);
        }
    }

    /// A line `w` pixels wide with round ends.
    pub fn line(&mut self, a: Vec2, b: Vec2, w: f32, c: Color) {
        let d = (b - a).normalize_or_zero();
        let n = Vec2::new(-d.y, d.x) * w * 0.5;
        self.convex(&[a + n, b + n, b - n, a - n], c);
        self.circle(a, w * 0.5, c);
        self.circle(b, w * 0.5, c);
    }

    /// A disc whose colour runs from `inner` at `center` to `outer` at its rim, softly at both
    /// ends (a glow, a light on the ground): rings eased with a smoothstep, so neither the
    /// middle nor the rim shows an edge.
    pub fn radial(&mut self, center: Vec2, radius: f32, inner: Color, outer: Color) {
        let n = ((radius * 0.5) as usize).clamp(24, 96);
        // (enough rings that the eye sees a curve and not the bands of straight ramps)
        let rings = ((radius / 40.0) as usize).clamp(6, 24);
        let dir = |k: usize| {
            let a = std::f32::consts::TAU * (k % n) as f32 / n as f32;
            Vec2::new(a.cos(), a.sin())
        };
        let col = |f: f32| inner.mix(outer, f * f * (3.0 - 2.0 * f));
        for j in 0..rings {
            let (f0, f1) = (j as f32 / rings as f32, (j + 1) as f32 / rings as f32);
            let (c0, c1) = (col(f0), col(f1));
            for k in 0..n {
                let (d0, d1) = (dir(k), dir(k + 1));
                if j == 0 {
                    self.tri(center, center + d0 * radius * f1, center + d1 * radius * f1, c0, c1, c1);
                } else {
                    let (i0, i1, o0, o1) = (center + d0 * radius * f0, center + d1 * radius * f0, center + d0 * radius * f1, center + d1 * radius * f1);
                    self.tri(i0, o0, o1, c0, c1, c1);
                    self.tri(i0, o1, i1, c0, c1, c0);
                }
            }
        }
    }

    /// A line `width` pixels wide along `pts` with round ends and joins - a pen's stroke. The
    /// gentle bends of a flattened curve are mitred and a sharp corner (over 30 degrees) gets
    /// a round join, so along a smooth path no triangle lies over another: the stroke can be
    /// drawn see-through without its joins showing as beads. One point draws a dot.
    pub fn stroke(&mut self, pts: &[Vec2], width: f32, c: Color) {
        self.stroke_band(pts, 0.0, width * 0.5, c);
    }

    /// A stroke on a darker edge, as a route lies on a map: `width` of `fill`, and `edge`
    /// pixels of `edge_c` along both sides and round both ends. The edge and the fill lie
    /// beside each other, not one over the other (see-through, the edge does not show through
    /// the fill); the edge is drawn first, so where the path crosses itself its fill runs on
    /// over its own edge, as the map's route does.
    pub fn stroke_edged(&mut self, pts: &[Vec2], width: f32, edge: f32, fill: Color, edge_c: Color) {
        if edge > 0.0 {
            self.stroke_band(pts, width * 0.5, width * 0.5 + edge, edge_c);
        }
        self.stroke_band(pts, 0.0, width * 0.5, fill);
    }

    /// The part of a stroke along `pts` between `r0` and `r1` pixels from its middle, on both
    /// sides and round both ends (`r0` 0: the stroke itself, else a band along its edges).
    fn stroke_band(&mut self, pts: &[Vec2], r0: f32, r1: f32, c: Color) {
        let mut p: Vec<Vec2> = Vec::with_capacity(pts.len());
        for &q in pts {
            if p.last().is_none_or(|l: &Vec2| l.distance_squared(q) > 1e-6) {
                p.push(q);
            }
        }
        if p.is_empty() || r1 <= r0 {
            return;
        }
        if p.len() == 1 {
            // (no length: the pen only touched the paper)
            self.fan(p[0], r0, r1, Vec2::X, Vec2::Y, -Vec2::X, c);
            self.fan(p[0], r0, r1, -Vec2::X, -Vec2::Y, Vec2::X, c);
            return;
        }
        let perp = |d: Vec2| Vec2::new(-d.y, d.x);
        let n = p.len();
        let dirs: Vec<Vec2> = (0..n - 1).map(|i| (p[i + 1] - p[i]).normalize_or_zero()).collect();
        // per point: the offset the segment ending there uses, and the one starting there
        // (the same mitre for a gentle bend)
        let mut end_o = vec![Vec2::ZERO; n];
        let mut start_o = vec![Vec2::ZERO; n];
        start_o[0] = perp(dirs[0]);
        end_o[n - 1] = perp(dirs[n - 2]);
        let mut joins = Vec::new();
        for i in 1..n - 1 {
            let (d0, d1) = (dirs[i - 1], dirs[i]);
            let (n0, n1) = (perp(d0), perp(d1));
            if d0.dot(d1) > 0.866 {
                let m = (n0 + n1).normalize_or(n1);
                let o = m / m.dot(n1).max(0.5);
                end_o[i] = o;
                start_o[i] = o;
            } else {
                end_o[i] = n0;
                start_o[i] = n1;
                // the outer side of the corner: away from where it turns
                let side = if d0.perp_dot(d1) > 0.0 { -1.0 } else { 1.0 };
                joins.push((p[i], n0 * side, n1 * side, d0 * side));
            }
        }
        for i in 0..n - 1 {
            let (a, b) = (p[i], p[i + 1]);
            let (oa, ob) = (start_o[i], end_o[i + 1]);
            if r0 == 0.0 {
                let (a0, a1, b0, b1) = (a - oa * r1, a + oa * r1, b - ob * r1, b + ob * r1);
                self.tri(a0, a1, b1, c, c, c);
                self.tri(a0, b1, b0, c, c, c);
            } else {
                for s in [1.0, -1.0] {
                    let (a0, a1, b0, b1) = (a + oa * (s * r0), a + oa * (s * r1), b + ob * (s * r0), b + ob * (s * r1));
                    self.tri(a0, a1, b1, c, c, c);
                    self.tri(a0, b1, b0, c, c, c);
                }
            }
        }
        // a sharp corner: round on its outer side (on the inner the two segments overlap)
        for (q, from, to, along) in joins {
            let mid = (from + to).normalize_or(along);
            self.fan(q, r0, r1, from, mid, to, c);
        }
        // the round ends, each a half turn from one side over the end to the other
        let (d0, d1) = (dirs[0], dirs[n - 2]);
        self.fan(p[0], r0, r1, perp(d0), -d0, -perp(d0), c);
        self.fan(p[n - 1], r0, r1, -perp(d1), d1, perp(d1), c);
    }

    /// The ring between `r0` and `r1` round `c`, from the unit direction `from` over `mid` to
    /// `to` (each under a half turn from the next); its first and last spokes are exactly
    /// `from` and `to`, so it closes on the bands beside it without a crack.
    #[allow(clippy::too_many_arguments)]
    fn fan(&mut self, c: Vec2, r0: f32, r1: f32, from: Vec2, mid: Vec2, to: Vec2, col: Color) {
        for (a, b) in [(from, mid), (mid, to)] {
            let ang = a.angle_to(b);
            let steps = ((ang.abs() * r1.max(1.0)).sqrt() * 1.6).ceil().clamp(2.0, 32.0) as usize;
            let dir = |k: usize| if k == 0 { a } else if k == steps { b } else { Vec2::from_angle(ang * k as f32 / steps as f32).rotate(a) };
            for k in 0..steps {
                let (u, v) = (dir(k), dir(k + 1));
                if r0 == 0.0 {
                    self.tri(c, c + u * r1, c + v * r1, col, col, col);
                } else {
                    let (i0, i1, o0, o1) = (c + u * r0, c + v * r0, c + u * r1, c + v * r1);
                    self.tri(i0, o0, o1, col, col, col);
                    self.tri(i0, o1, i1, col, col, col);
                }
            }
        }
    }

    /// A sprite of the atlas with its top left at `at`, tinted `c`.
    pub fn sprite(&mut self, s: Sprite, at: Vec2, size: Vec2, c: Color) {
        let v = |p: Vec2, u: f32, w: f32| Vertex { pos: [p.x, p.y, 0.0], uv: [u, w], color: c.0, mode: [0.0, 1.0], ..Default::default() };
        let (a, b, cc, d) = (at, at + Vec2::new(size.x, 0.0), at + size, at + Vec2::new(0.0, size.y));
        self.verts.extend([v(a, s.uv[0], s.uv[1]), v(b, s.uv[2], s.uv[1]), v(cc, s.uv[2], s.uv[3]), v(a, s.uv[0], s.uv[1]), v(cc, s.uv[2], s.uv[3]), v(d, s.uv[0], s.uv[3])]);
    }

    /// Text on the baseline at `at` (pixels): returns its width.
    #[allow(clippy::too_many_arguments)]
    pub fn text(&mut self, atlas: &mut Atlas, fonts: &Fonts, text: &str, px: f32, weight: Weight, at: Vec2, align: Align, c: Color) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let text = &*crate::i18n::tr(text);
        let k = self.scale;
        let s = atlas.text(fonts, text, px * k, weight);
        let pad = crate::text::PAD as f32 / k;
        let w = s.w / k - 2.0 * pad;
        let x = match align {
            Align::Left => at.x,
            Align::Center => at.x - w * 0.5,
            Align::Right => at.x - w,
        } - pad;
        // (whole physical pixels: text sampled between texels goes soft)
        let top = Vec2::new(self.snap(x), self.snap(at.y - s.ascent / k));
        self.sprite(s, top, Vec2::new(s.w / k, s.h / k), c);
        w
    }

    /// Text centred vertically in `r` (capitals centred), aligned as asked, cut with an
    /// ellipsis to fit.
    #[allow(clippy::too_many_arguments)]
    pub fn text_in(&mut self, atlas: &mut Atlas, fonts: &Fonts, text: &str, px: f32, weight: Weight, r: Rect, align: Align, c: Color) -> f32 {
        let text = crate::i18n::tr(text);
        let t = fonts.fit(&text, px, weight, r.w);
        let base = r.y + r.h * 0.5 + fonts.cap_height(px, weight) * 0.5;
        let x = match align {
            Align::Left => r.x,
            Align::Center => r.x + r.w * 0.5,
            Align::Right => r.right(),
        };
        self.text(atlas, fonts, &t, px, weight, Vec2::new(x, base), align, c)
    }

    /// An icon centred at `center`, `size` pixels.
    pub fn icon(&mut self, atlas: &mut Atlas, name: &str, center: Vec2, size: f32, c: Color) {
        let k = self.scale;
        let px = (size * k).round().max(4.0) as u32;
        if let Some(s) = atlas.icon(name, px) {
            let d = px as f32 / k;
            let at = Vec2::new(self.snap(center.x - d * 0.5), self.snap(center.y - d * 0.5));
            self.sprite(s, at, Vec2::splat(d), c);
        }
    }

    // --- world space -----------------------------------------------------------------

    fn wv(p: Vec3, ext: Vec2, w_m: f32, w_px: f32, c: Color) -> Vertex {
        Vertex { pos: p.to_array(), ext: ext.to_array(), width: [w_m, w_px], color: c.0, mode: [1.0, 0.0], ..Default::default() }
    }

    /// A band along `pts` (world), `w_m` metres wide but at least `w_px` pixels, with
    /// round ends when `caps`. Gentle bends are mitred; sharper ones get a round joint (a
    /// clamped mitre left notches in hairpins and turning loops).
    pub fn ribbon(&mut self, pts: &[Vec3], w_m: f32, w_px: f32, c: Color, caps: bool) {
        // (points closer than a centimetre make no direction)
        let mut clean: Vec<Vec3> = Vec::with_capacity(pts.len());
        for &q in pts {
            if clean.last().map(|l: &Vec3| (q - *l).truncate().length_squared() > 1e-4).unwrap_or(true) {
                clean.push(q);
            }
        }
        let pts = &clean[..];
        if pts.len() < 2 {
            return;
        }
        let (hm, hp) = (w_m * 0.5, w_px * 0.5);
        let dir = |a: Vec3, b: Vec3| (b - a).truncate().normalize_or_zero();
        let perp = |d: Vec2| Vec2::new(-d.y, d.x);
        let n = pts.len();
        // per point: the extrusion the segment ending there uses, and the one starting there
        let mut end_n = vec![Vec2::ZERO; n];
        let mut start_n = vec![Vec2::ZERO; n];
        let mut joints = Vec::new();
        for i in 0..n {
            let d0 = if i > 0 { dir(pts[i - 1], pts[i]) } else { dir(pts[0], pts[1]) };
            let d1 = if i + 1 < n { dir(pts[i], pts[i + 1]) } else { d0 };
            let (n0, n1) = (perp(d0), perp(d1));
            let cos = d0.dot(d1);
            if cos > 0.94 {
                // under 20 degrees: a mitre, at most 3 % longer
                let m = (n0 + n1).normalize_or(n1);
                let k = 1.0 / m.dot(n1).max(0.5);
                end_n[i] = m * k;
                start_n[i] = m * k;
            } else {
                end_n[i] = n0;
                start_n[i] = n1;
                joints.push((i, n0, n1));
            }
        }
        for i in 0..n - 1 {
            let (a, b) = (pts[i], pts[i + 1]);
            let (na, nb) = (start_n[i], end_n[i + 1]);
            let (a0, a1) = (Self::wv(a, na, hm, hp, c), Self::wv(a, -na, hm, hp, c));
            let (b0, b1) = (Self::wv(b, nb, hm, hp, c), Self::wv(b, -nb, hm, hp, c));
            self.verts.extend([a0, b0, b1, a0, b1, a1]);
        }
        // round joints: a fan on both sides from one segment's edge to the next one's
        for (i, n0, n1) in joints {
            let p = pts[i];
            for (from, to) in [(n0, n1), (-n0, -n1)] {
                let a0 = from.y.atan2(from.x);
                let mut da = to.y.atan2(to.x) - a0;
                if da > std::f32::consts::PI {
                    da -= std::f32::consts::TAU;
                } else if da < -std::f32::consts::PI {
                    da += std::f32::consts::TAU;
                }
                let steps = ((da.abs() / 0.35).ceil() as usize).clamp(1, 12);
                for k in 0..steps {
                    let t0 = a0 + da * k as f32 / steps as f32;
                    let t1 = a0 + da * (k + 1) as f32 / steps as f32;
                    self.verts.extend([Self::wv(p, Vec2::ZERO, hm, hp, c), Self::wv(p, Vec2::new(t0.cos(), t0.sin()), hm, hp, c), Self::wv(p, Vec2::new(t1.cos(), t1.sin()), hm, hp, c)]);
                }
            }
        }
        if caps {
            let first = dir(pts[0], pts[1]);
            let last = dir(pts[n - 2], pts[n - 1]);
            self.cap(pts[0], -first, hm, hp, c);
            self.cap(pts[n - 1], last, hm, hp, c);
        }
    }

    /// Text turned by `angle` (radians, clockwise on screen) about its centre `at`.
    #[allow(clippy::too_many_arguments)]
    pub fn text_rotated(&mut self, atlas: &mut Atlas, fonts: &Fonts, text: &str, px: f32, weight: Weight, at: Vec2, angle: f32, c: Color) -> Vec2 {
        let text = &*crate::i18n::tr(text);
        if text.is_empty() {
            return Vec2::ZERO;
        }
        let k = self.scale;
        let s = atlas.text(fonts, text, px * k, weight);
        let (w, h) = (s.w / k, s.h / k);
        let (sn, cs) = angle.sin_cos();
        let rot = |v: Vec2| at + Vec2::new(v.x * cs - v.y * sn, v.x * sn + v.y * cs);
        let v = |p: Vec2, u: f32, t: f32| Vertex { pos: [p.x, p.y, 0.0], uv: [u, t], color: c.0, mode: [0.0, 1.0], ..Default::default() };
        let (hw, hh) = (w * 0.5, h * 0.5);
        let (a, b, cc, d) = (rot(Vec2::new(-hw, -hh)), rot(Vec2::new(hw, -hh)), rot(Vec2::new(hw, hh)), rot(Vec2::new(-hw, hh)));
        self.verts.extend([v(a, s.uv[0], s.uv[1]), v(b, s.uv[2], s.uv[1]), v(cc, s.uv[2], s.uv[3]), v(a, s.uv[0], s.uv[1]), v(cc, s.uv[2], s.uv[3]), v(d, s.uv[0], s.uv[3])]);
        Vec2::new(w, h)
    }

    /// A half disc at `p` bulging towards `d`.
    fn cap(&mut self, p: Vec3, d: Vec2, hm: f32, hp: f32, c: Color) {
        let n = 5;
        let base = d.y.atan2(d.x) - std::f32::consts::FRAC_PI_2;
        for k in 0..n {
            let a0 = base + std::f32::consts::PI * k as f32 / n as f32;
            let a1 = base + std::f32::consts::PI * (k + 1) as f32 / n as f32;
            self.verts.extend([Self::wv(p, Vec2::ZERO, hm, hp, c), Self::wv(p, Vec2::new(a0.cos(), a0.sin()), hm, hp, c), Self::wv(p, Vec2::new(a1.cos(), a1.sin()), hm, hp, c)]);
        }
    }

    /// A disc on the ground: radius `r_m` metres but at least `r_px` pixels.
    pub fn world_disc(&mut self, p: Vec3, r_m: f32, r_px: f32, c: Color) {
        let n = 24;
        for k in 0..n {
            let a0 = std::f32::consts::TAU * k as f32 / n as f32;
            let a1 = std::f32::consts::TAU * (k + 1) as f32 / n as f32;
            self.verts.extend([Self::wv(p, Vec2::ZERO, r_m, r_px, c), Self::wv(p, Vec2::new(a0.cos(), a0.sin()), r_m, r_px, c), Self::wv(p, Vec2::new(a1.cos(), a1.sin()), r_m, r_px, c)]);
        }
    }

    /// A flat shape standing on `p`: `shape` is its outline (convex, any unit), scaled by
    /// `s_m` metres but at least `s_px` pixels.
    pub fn world_shape(&mut self, p: Vec3, shape: &[Vec2], s_m: f32, s_px: f32, c: Color) {
        for k in 1..shape.len().saturating_sub(1) {
            self.verts.extend([Self::wv(p, shape[0], s_m, s_px, c), Self::wv(p, shape[k], s_m, s_px, c), Self::wv(p, shape[k + 1], s_m, s_px, c)]);
        }
    }

    /// A convex polygon on the ground (world points).
    pub fn world_poly(&mut self, pts: &[Vec3], c: Color) {
        for k in 1..pts.len().saturating_sub(1) {
            self.verts.extend([Self::wv(pts[0], Vec2::ZERO, 0.0, 0.0, c), Self::wv(pts[k], Vec2::ZERO, 0.0, 0.0, c), Self::wv(pts[k + 1], Vec2::ZERO, 0.0, 0.0, c)]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_make_whole_triangles() {
        let mut p = Painter::new();
        p.rounded(Rect::new(0.0, 0.0, 100.0, 40.0), 8.0, Color::WHITE);
        p.shadow(Rect::new(0.0, 0.0, 100.0, 40.0), 8.0, 12.0, Color::BLACK.alpha(0.5));
        p.rounded_border(Rect::new(0.0, 0.0, 100.0, 40.0), 8.0, 1.0, Color::WHITE);
        p.arc(Vec2::ZERO, 10.0, 12.0, 0.0, 3.0, Color::WHITE);
        p.ribbon(&[Vec3::ZERO, Vec3::X * 10.0, Vec3::new(10.0, 10.0, 0.0)], 3.0, 4.0, Color::WHITE, true);
        assert_eq!(p.verts.len() % 3, 0);
        // a ribbon's corners are extruded both ways from the centre line
        let r: Vec<&Vertex> = p.verts.iter().filter(|v| v.mode[0] == 1.0 && v.ext != [0.0, 0.0]).collect();
        assert!(r.iter().any(|v| v.ext[1] > 0.5) && r.iter().any(|v| v.ext[1] < -0.5));
    }

    /// The area the triangles cover, counting what lies twice twice.
    fn area(v: &[Vertex]) -> f32 {
        v.chunks(3).map(|t| {
            let (a, b, c) = (Vec2::new(t[0].pos[0], t[0].pos[1]), Vec2::new(t[1].pos[0], t[1].pos[1]), Vec2::new(t[2].pos[0], t[2].pos[1]));
            (b - a).perp_dot(c - a).abs() * 0.5
        }).sum()
    }

    #[test]
    fn paths_measure_and_sample() {
        let mut p = Path::new(Vec2::ZERO);
        p.line_to(Vec2::new(100.0, 0.0)).line_to(Vec2::new(100.0, 50.0));
        assert_eq!(p.length(), 150.0);
        assert_eq!(p.at(25.0), (Vec2::new(25.0, 0.0), Vec2::X));
        assert_eq!(p.at(125.0), (Vec2::new(100.0, 25.0), Vec2::Y));
        // held at the ends
        assert_eq!(p.at(-5.0).0, Vec2::ZERO);
        assert_eq!(p.at(1e6).0, Vec2::new(100.0, 50.0));
        // a part: cut exactly, the corner kept
        assert_eq!(p.part(90.0, 110.0), vec![Vec2::new(90.0, 0.0), Vec2::new(100.0, 0.0), Vec2::new(100.0, 10.0)]);
        assert_eq!(p.part(10.0, 20.0), vec![Vec2::new(10.0, 0.0), Vec2::new(20.0, 0.0)]);
        assert!(p.part(20.0, 10.0).is_empty());
        // a half circle, clockwise on the screen: through the bottom, its length pi r
        let mut c = Path::new(Vec2::new(50.0, 0.0));
        c.arc_around(Vec2::ZERO, std::f32::consts::PI);
        assert!((c.length() / (std::f32::consts::PI * 50.0) - 1.0).abs() < 1e-3, "{}", c.length());
        let (mid, dir) = c.at(c.length() * 0.5);
        // (within the flatness: the middle may lie on a chord)
        assert!(mid.distance(Vec2::new(0.0, 50.0)) <= FLATNESS + 1e-3 && dir.distance(-Vec2::X) < 0.05, "{mid} {dir}");
        assert!(c.points().iter().all(|q| (q.length() - 50.0).abs() < 1e-3));
        // a cubic: straight when its handles are on the line; a quarter circle's length
        let mut s = Path::new(Vec2::ZERO);
        s.cubic_to(Vec2::new(30.0, 0.0), Vec2::new(70.0, 0.0), Vec2::new(100.0, 0.0));
        assert!((s.length() - 100.0).abs() < 1e-3);
        let k = 0.552_284_8 * 80.0;
        let mut q = Path::new(Vec2::new(80.0, 0.0));
        q.cubic_to(Vec2::new(80.0, k), Vec2::new(k, 80.0), Vec2::new(0.0, 80.0));
        assert!((q.length() / (std::f32::consts::FRAC_PI_2 * 80.0) - 1.0).abs() < 1e-3, "{}", q.length());
        assert!(q.points().windows(2).all(|w| w[0].distance(w[1]) < 8.0), "flattened finely");
    }

    #[test]
    fn strokes_cover_their_area_once() {
        // a smooth half ring: the band, its two round ends, and nothing twice
        let (r, w, e) = (60.0, 10.0, 3.0);
        let mut c = Path::new(Vec2::new(r, 0.0));
        c.arc_around(Vec2::ZERO, std::f32::consts::PI);
        let mut p = Painter::new();
        p.stroke(c.points(), w, Color::WHITE);
        let want = std::f32::consts::PI * r * w + std::f32::consts::PI * (w * 0.5) * (w * 0.5);
        assert!((area(&p.verts) / want - 1.0).abs() < 0.01, "{} {want}", area(&p.verts));
        // with an edge: the edge beside the fill, together the area of the wider stroke
        let mut q = Painter::new();
        q.stroke_edged(c.points(), w, e, Color::WHITE, Color::BLACK);
        let edged = std::f32::consts::PI * r * (w + 2.0 * e) + std::f32::consts::PI * (w * 0.5 + e) * (w * 0.5 + e);
        assert!((area(&q.verts) / edged - 1.0).abs() < 0.01, "{} {edged}", area(&q.verts));
        let fill: Vec<Vertex> = q.verts.iter().copied().filter(|v| v.color == Color::WHITE.0).collect();
        assert!((area(&fill) / want - 1.0).abs() < 0.01);
        // the edge's inner side is the fill's outer side, point for point
        let fill_pts: Vec<[f32; 3]> = fill.iter().map(|v| v.pos).collect();
        assert!(q.verts.iter().filter(|v| v.color == Color::BLACK.0).any(|v| fill_pts.contains(&v.pos)));
        // a sharp corner and a dot still make whole triangles
        let mut s = Painter::new();
        s.stroke(&[Vec2::ZERO, Vec2::new(40.0, 0.0), Vec2::new(0.0, 10.0)], 6.0, Color::WHITE);
        s.stroke_edged(&[Vec2::new(5.0, 5.0)], 6.0, 2.0, Color::WHITE, Color::BLACK);
        assert_eq!(s.verts.len() % 3, 0);
        let mut dot = Painter::new();
        dot.stroke(&[Vec2::new(5.0, 5.0), Vec2::new(5.0, 5.0)], 6.0, Color::WHITE);
        assert!((area(&dot.verts) / (std::f32::consts::PI * 9.0) - 1.0).abs() < 0.03, "{}", area(&dot.verts));
        // a glow is a full disc
        let mut g = Painter::new();
        g.radial(Vec2::ZERO, 100.0, Color::WHITE, Color::CLEAR);
        assert!((area(&g.verts) / (std::f32::consts::PI * 1e4) - 1.0).abs() < 0.01);
    }

    #[test]
    fn rects_split() {
        let r = Rect::new(10.0, 10.0, 100.0, 50.0);
        let (l, rest) = r.cut_left(30.0);
        assert_eq!((l.w, rest.x, rest.w), (30.0, 40.0, 70.0));
        assert!(r.contains(Vec2::new(50.0, 30.0)) && !r.contains(Vec2::new(5.0, 30.0)));
    }
}
