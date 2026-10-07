//! A device of the bus drawn by the page itself: its 2D form.
//!
//! The live picture of a device (`super::device`) was a photograph: a mirror of the cab, a
//! little soft, lit as the cab is lit, and late by a picture. Omsi-Hub drew its ALMEX and
//! its IBIS itself instead, from the bus's files, and so does this: a device is sent once as
//! its form - what lies on its face, laid flat - and the page draws it from that, as often as
//! it likes and as sharp as the screen it is on; only what changes goes on: which parts show,
//! where the moving ones are, the strings its text textures show.
//!
//! The form is the device as a camera straight before its face sees it, but drawn flat
//! (orthographic): every triangle of the inside's meshes that lies before the face's
//! rectangle within the depth the face camera draws, facing the viewer as the renderer
//! culls it, in the face's own frame - across and down in metres from its top left corner,
//! and how far before it. Each part is one material of one mesh: its triangles, its texture
//! (a file of the bus, a text texture, a script texture), its colour and how it is blended.
//! The page draws them with a depth buffer, as the game does, the opaque ones in the
//! model's order and the blended ones from the back.
//!
//! What lives:
//! * which meshes show (`MeshProps::visible`: `[visible]`, the scripts' switches) - an
//!   ALMEX's menu pages are its meshes, one shown at a time;
//! * where they are: a mesh moved by its animation since the form was made (a key pressed
//!   in, a flap) is sent as the movement in the face's frame, the device's own movement (on
//!   a door that swings) taken out;
//! * the strings the text textures show: the page writes them in the texture's `.oft` font
//!   itself, with the game's own rules (`omsi_content::font`), into the texture the mesh
//!   shows;
//! * a script texture (what a script paints with `ST*`, an `[htmltexture]` page) is a
//!   picture, the only one: sent as it changes.
//!
//! A tap goes to the touch areas: the `[mouseevent]` meshes on the face, their footprint
//! (the outline of their corners laid flat). The nearest one under the finger is clicked as
//! the mouse clicks it in the cab (`Player::click_mesh`), and let go again with the finger.

use glam::{Mat3, Mat4, Vec2, Vec3};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// What the face camera draws before the device's face: this share of its longer side,
/// within these bounds (m)...
const FRONT: f32 = 0.6;
const FRONT_MIN: f32 = 0.08;
const FRONT_MAX: f32 = 0.3;
/// ...and behind it, at least (m).
const BEHIND_MIN: f32 = 0.5;
/// Coordinates go as whole numbers of this many per metre (a hundredth of a millimetre),
/// texture coordinates of this many per unit.
pub(crate) const QUANT: f32 = 100_000.0;
/// A mesh moved by less than this (in the face's frame) has not moved.
const STILL: f32 = 2e-5;
/// Most triangles of one form (a form is sent whole to a phone).
const MAX_TRIS: usize = 60_000;
/// A mesh that lies wholly this far before the face (m) is something of the cab between it
/// and the driver (a lamp, the steering column), not part of the device; and so is one that
/// lies this far before it and covers this share of it.
const OCCLUDER_NEAR: f32 = 0.06;
const OCCLUDER_BIG_NEAR: f32 = 0.02;
const OCCLUDER_COVER: f32 = 0.2;
/// What lives on a watched form goes this often at most (a second).
pub(crate) const LIVE_FPS: f64 = 20.0;

/// The plane a device is drawn in: its face, in the bus's own frame (unturned).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sheet {
    /// The top left corner, across (unit), down (unit), out of the face towards the viewer.
    pub top_left: Vec3,
    pub ax: Vec3,
    pub ay: Vec3,
    pub normal: Vec3,
    /// Width and height (m).
    pub w: f32,
    pub h: f32,
    /// What is drawn: from this far before the face to this far behind it (m).
    pub front: f32,
    pub behind: f32,
}

impl Sheet {
    /// The face with the corner `top_left`, `across` its width and `down` its height, `normal`
    /// out of it (the face camera's, [`super::device::View::face`]).
    pub(crate) fn of(top_left: Vec3, across: Vec3, down: Vec3, normal: Vec3) -> Option<Sheet> {
        let normal = normal.normalize_or_zero();
        if normal == Vec3::ZERO || !top_left.is_finite() || !across.is_finite() || !down.is_finite() {
            return None;
        }
        let ay = (down - normal * down.dot(normal)).normalize_or_zero();
        if ay == Vec3::ZERO {
            return None;
        }
        let ax = normal.cross(ay);
        let (w, h) = (across.dot(ax), down.dot(ay));
        if w < 1e-3 || h < 1e-3 {
            return None;
        }
        let front = (w.max(h) * FRONT).clamp(FRONT_MIN, FRONT_MAX);
        Some(Sheet { top_left, ax, ay, normal, w, h, front, behind: front.max(BEHIND_MIN) })
    }

    /// From the bus's frame to the sheet's: across, down (m from the top left corner), before
    /// the face (m).
    pub(crate) fn to_local(&self) -> Mat4 {
        let r = Mat4::from_cols(self.ax.extend(0.0), self.ay.extend(0.0), self.normal.extend(0.0), glam::Vec4::W).transpose();
        r * Mat4::from_translation(-self.top_left)
    }
}

/// What a part shows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tex {
    /// Its material's colour alone.
    Colour,
    /// A texture file of the bus (an index into [`Form::files`] once built).
    File(PathBuf),
    /// `text_textures[n]`, written by the page.
    Text(usize),
    /// `host.script_textures[n]` (an `[htmltexture]` page draws into one too): its picture.
    Script(usize),
}

/// How one material slot of a mesh is drawn.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Slot {
    pub tex: Tex,
    /// Multiplies the texture (r, g, b, a; a counts without a texture only).
    pub colour: [f32; 4],
    /// 0 opaque, 1 alpha test (at a half), 2 blended.
    pub alpha: u8,
    /// The texture repeats beyond 0..1 (else it is clamped).
    pub wrap: bool,
    pub z_write: bool,
}

/// A mesh of the inside as the form builder sees it.
pub(crate) struct MeshIn<'a> {
    /// Its index among the bus's meshes.
    pub mesh: usize,
    /// Where it is (mesh to the bus's own frame).
    pub xf: Mat4,
    pub data: &'a omsi_geometry::MeshData,
    /// Per material slot, how it is drawn (None: not at all).
    pub slots: Vec<Option<Slot>>,
    /// Its `[mouseevent]`, when it is a switch one may tap (not a whole panel).
    pub event: Option<String>,
}

/// One material of one mesh, laid flat.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Part {
    /// Index into [`Form::meshes`].
    pub mesh: usize,
    pub slot: usize,
    /// What it shows: for [`Tex::File`] an index into [`Form::files`].
    pub tex: PartTex,
    pub colour: [f32; 4],
    pub alpha: u8,
    pub wrap: bool,
    pub z_write: bool,
    /// Its corners: across, down, before the face (m); u, v (moved by whole turns into the
    /// texture's first where they lie within one).
    pub verts: Vec<[f32; 5]>,
    pub idx: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PartTex {
    Colour,
    File(usize),
    Text(usize),
    Script(usize),
}

/// A switch on the face: its mesh (index into [`Form::meshes`]), its event, its triangles
/// laid flat (across, down, before the face; m; both sides, as the cab's pick takes them) and
/// how far before the face its nearest corner is.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Touch {
    pub mesh: usize,
    pub event: String,
    pub tris: Vec<[Vec3; 3]>,
    pub z: f32,
}

/// A texture file the form shows, and the part of it its parts use (u0, v0, u1, v1; all of it
/// when one of them wraps round).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FileUse {
    pub path: PathBuf,
    pub uv: [f32; 4],
}

/// A device's form.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Form {
    pub sheet: Sheet,
    /// In drawing order: the opaque and alpha-tested ones in the model's order, then the
    /// blended ones from the back.
    pub parts: Vec<Part>,
    /// The nearest last.
    pub touches: Vec<Touch>,
    pub files: Vec<FileUse>,
    /// The bus's meshes the parts and touches are of, and where each was when the form was
    /// made (mesh to the bus's own frame).
    pub meshes: Vec<usize>,
    pub built: Vec<Mat4>,
    /// The mesh the device moves with (index into `meshes`).
    pub anchor: Option<usize>,
    /// The text and script textures shown.
    pub texts: Vec<usize>,
    pub scripts: Vec<usize>,
}

/// Where a point lies on the sheet, interpolated.
fn lerp5(a: [f32; 5], b: [f32; 5], t: f32) -> [f32; 5] {
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

/// A polygon cut to what lies no nearer than `front` (z at most `front`).
fn clip_front(poly: &[[f32; 5]], front: f32) -> Vec<[f32; 5]> {
    let mut out = Vec::with_capacity(poly.len() + 2);
    for k in 0..poly.len() {
        let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
        let (ina, inb) = (a[2] <= front, b[2] <= front);
        if ina {
            out.push(a);
        }
        if ina != inb {
            out.push(lerp5(a, b, (front - a[2]) / (b[2] - a[2])));
        }
    }
    out
}

/// Twice the signed area of a triangle on the sheet (down the sheet is +y): positive when its
/// corners go clockwise as one sees them - the side the renderer draws of a one-sided mesh
/// (`one_sided_primitive`: the visible side arrives clockwise).
fn turn(a: [f32; 5], b: [f32; 5], c: [f32; 5]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// How far before the face triangle `t` is at `q` (across, down), when `q` lies on it.
#[cfg(test)]
pub(crate) fn depth_at(t: &[Vec3; 3], q: Vec2) -> Option<f32> {
    let (a, b, c) = (t[0].truncate(), t[1].truncate(), t[2].truncate());
    let det = (b - a).perp_dot(c - a);
    if det.abs() < 1e-12 {
        return None;
    }
    let w1 = (q - a).perp_dot(c - a) / det;
    let w2 = (b - a).perp_dot(q - a) / det;
    let w0 = 1.0 - w1 - w2;
    let eps = -1e-5;
    (w0 >= eps && w1 >= eps && w2 >= eps).then(|| t[0].z * w0 + t[1].z * w1 + t[2].z * w2)
}

/// The form of the device on `sheet`, from the meshes round it; `anchor` the bus's mesh the
/// device moves with. None when nothing of it is drawn.
pub(crate) fn build(sheet: &Sheet, meshes: &[MeshIn], anchor: Option<usize>) -> Option<Form> {
    let to = sheet.to_local();
    let (w, h, front, behind) = (sheet.w, sheet.h, sheet.front, sheet.behind);
    let mut form = Form { sheet: *sheet, parts: Vec::new(), touches: Vec::new(), files: Vec::new(), meshes: Vec::new(), built: Vec::new(), anchor: None, texts: Vec::new(), scripts: Vec::new() };
    let mut blended: Vec<(f32, Part)> = Vec::new();
    let mut tris = 0usize;
    for m in meshes {
        let x = to * m.xf;
        let d = m.data;
        if d.positions.is_empty() || !x.is_finite() {
            continue;
        }
        // (a mirrored mesh is drawn from both sides, as the renderer does, and so is
        // generated geometry)
        let cull = d.one_sided && Mat3::from_mat4(m.xf).determinant() > 0.0;
        let pts: Vec<Vec3> = d.positions.iter().map(|p| x.transform_point3(*p)).collect();
        let k = form.meshes.len();
        let mut used = false;
        // its slots laid flat first: a mesh that floats before the face is no part of the
        // device but something of the cab between it and the driver
        let mut laid: Vec<(usize, &Slot, Vec<[f32; 5]>, Vec<u32>)> = Vec::new();
        for (slot, s) in m.slots.iter().enumerate() {
            let Some(s) = s else { continue };
            let mut verts: Vec<[f32; 5]> = Vec::new();
            let mut idx: Vec<u32> = Vec::new();
            let mut of_vertex: std::collections::HashMap<u32, u32> = Default::default();
            for r in d.ranges.iter().filter(|r| r.2 as usize == slot) {
                let Some(list) = d.indices.get(r.0 as usize..(r.0 + r.1) as usize) else { continue };
                for t in list.chunks_exact(3) {
                    if tris >= MAX_TRIS {
                        break;
                    }
                    let corner = |i: u32| -> Option<[f32; 5]> {
                        let p = *pts.get(i as usize)?;
                        let uv = d.uvs.get(i as usize).copied().unwrap_or(Vec2::ZERO);
                        Some([p.x, p.y, p.z, uv.x, uv.y])
                    };
                    let (Some(a), Some(b), Some(c)) = (corner(t[0]), corner(t[1]), corner(t[2])) else { continue };
                    let area = turn(a, b, c);
                    if area.abs() < 1e-10 || (cull && area < 0.0) {
                        continue;
                    }
                    // (on the face's rectangle, within the depth drawn)
                    let lo = [a[0].min(b[0]).min(c[0]), a[1].min(b[1]).min(c[1]), a[2].min(b[2]).min(c[2])];
                    let hi = [a[0].max(b[0]).max(c[0]), a[1].max(b[1]).max(c[1]), a[2].max(b[2]).max(c[2])];
                    if hi[0] <= 0.0 || lo[0] >= w || hi[1] <= 0.0 || lo[1] >= h || lo[2] > front || hi[2] < -behind {
                        continue;
                    }
                    tris += 1;
                    if hi[2] <= front {
                        for (j, q) in [(t[0], a), (t[1], b), (t[2], c)] {
                            let n = *of_vertex.entry(j).or_insert_with(|| {
                                verts.push(q);
                                verts.len() as u32 - 1
                            });
                            idx.push(n);
                        }
                    } else {
                        // (cut at the front, as the camera's near plane cuts it)
                        let poly = clip_front(&[a, b, c], front);
                        let first = verts.len() as u32;
                        verts.extend_from_slice(&poly);
                        for q in 1..poly.len().saturating_sub(1) as u32 {
                            idx.extend_from_slice(&[first, first + q, first + q + 1]);
                        }
                    }
                }
            }
            if !idx.is_empty() {
                laid.push((slot, s, verts, idx));
            }
        }
        let nearest_back = laid.iter().flat_map(|l| l.3.iter().map(|&i| l.2[i as usize][2])).fold(f32::MAX, f32::min);
        let cover: f32 = laid.iter().flat_map(|l| l.3.chunks_exact(3).map(|t| turn(l.2[t[0] as usize], l.2[t[1] as usize], l.2[t[2] as usize]).abs() * 0.5)).sum::<f32>() / (w * h);
        if !laid.is_empty() && (nearest_back > OCCLUDER_NEAR || (nearest_back > OCCLUDER_BIG_NEAR && cover > OCCLUDER_COVER)) {
            continue;
        }
        for (slot, s, mut verts, idx) in laid {
            used = true;
            // (texture coordinates of one turn moved into the first: the page's texture may
            // be only the part of the file they use)
            let (mut ulo, mut vlo, mut uhi, mut vhi) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for q in &verts {
                ulo = ulo.min(q[3]);
                vlo = vlo.min(q[4]);
                uhi = uhi.max(q[3]);
                vhi = vhi.max(q[4]);
            }
            let one_turn = (uhi - 1e-4).floor() <= ulo.floor() && (vhi - 1e-4).floor() <= vlo.floor();
            if one_turn {
                let (su, sv) = (ulo.floor(), vlo.floor());
                for q in verts.iter_mut() {
                    q[3] -= su;
                    q[4] -= sv;
                }
            }
            let tex = match &s.tex {
                Tex::Colour => PartTex::Colour,
                Tex::Text(n) => {
                    if !form.texts.contains(n) {
                        form.texts.push(*n);
                    }
                    PartTex::Text(*n)
                }
                Tex::Script(n) => {
                    if !form.scripts.contains(n) {
                        form.scripts.push(*n);
                    }
                    PartTex::Script(*n)
                }
                Tex::File(p) => {
                    let uv = if one_turn { [ulo - ulo.floor(), vlo - vlo.floor(), uhi - ulo.floor(), vhi - vlo.floor()] } else { [0.0, 0.0, 1.0, 1.0] };
                    let f = match form.files.iter().position(|f| f.path == *p) {
                        Some(f) => {
                            let o = &mut form.files[f].uv;
                            *o = [o[0].min(uv[0]), o[1].min(uv[1]), o[2].max(uv[2]), o[3].max(uv[3])];
                            f
                        }
                        None => {
                            form.files.push(FileUse { path: p.clone(), uv });
                            form.files.len() - 1
                        }
                    };
                    PartTex::File(f)
                }
            };
            // (one turn needs no repeating: its edge would sample the other edge)
            let part = Part { mesh: k, slot, tex, colour: s.colour, alpha: s.alpha, wrap: s.wrap && !one_turn, z_write: s.z_write, verts, idx };
            if s.alpha == 2 {
                let z = part.verts.iter().map(|q| q[2]).sum::<f32>() / part.verts.len() as f32;
                blended.push((z, part));
            } else {
                form.parts.push(part);
            }
        }
        // a switch on the face: its triangles over the face, from either side (a ray hits
        // them so in the cab)
        let mut touch = None;
        if let Some(event) = m.event.as_ref() {
            let mut flat: Vec<[Vec3; 3]> = Vec::new();
            for t in d.indices.chunks_exact(3) {
                let (Some(a), Some(b), Some(c)) = (pts.get(t[0] as usize), pts.get(t[1] as usize), pts.get(t[2] as usize)) else { continue };
                let (lo, hi) = (a.min(*b).min(*c), a.max(*b).max(*c));
                if hi.x <= 0.0 || lo.x >= w || hi.y <= 0.0 || lo.y >= h || lo.z > front || hi.z < -behind {
                    continue;
                }
                if (b.truncate() - a.truncate()).perp_dot(c.truncate() - a.truncate()).abs() < 1e-10 {
                    continue;
                }
                flat.push([*a, *b, *c]);
            }
            if !flat.is_empty() {
                let z = flat.iter().flatten().map(|p| p.z).fold(f32::MIN, f32::max).min(front);
                touch = Some(Touch { mesh: k, event: event.clone(), tris: flat, z });
            }
        }
        if let Some(t) = touch {
            form.touches.push(t);
            used = true;
        }
        if used {
            form.meshes.push(m.mesh);
            form.built.push(m.xf);
        }
    }
    if form.parts.is_empty() && blended.is_empty() {
        return None;
    }
    // (blended from the back; at one depth in the model's order)
    blended.sort_by(|a, b| ((a.0 * 2000.0).round()).total_cmp(&(b.0 * 2000.0).round()));
    form.parts.extend(blended.into_iter().map(|b| b.1));
    form.touches.sort_by(|a, b| a.z.total_cmp(&b.z));
    form.anchor = anchor.and_then(|a| form.meshes.iter().position(|&m| m == a));
    Some(form)
}

fn q(x: f32) -> i64 {
    (x * QUANT).round() as i64
}

impl Form {
    /// The form as the page reads it (`/api/form`): `files`, `texts` and `fonts` give the ids
    /// of the textures and fonts under which the server hands them out.
    pub(crate) fn json(&self, file_ids: &[usize], texts: &[Value], pages: &[usize]) -> Value {
        let parts: Vec<Value> = self
            .parts
            .iter()
            .map(|p| {
                let t = match p.tex {
                    PartTex::Colour => Value::Null,
                    PartTex::File(f) => json!({ "f": file_ids.get(f).copied().unwrap_or(usize::MAX) }),
                    PartTex::Text(n) => json!({ "t": n }),
                    PartTex::Script(n) => json!({ "s": n }),
                };
                let v: Vec<i64> = p.verts.iter().flat_map(|q5| q5.map(q)).collect();
                json!({
                    "m": p.mesh,
                    "tex": t,
                    "c": p.colour.map(|c| (c * 1000.0).round() / 1000.0),
                    "a": p.alpha,
                    "wrap": p.wrap,
                    "zw": p.z_write,
                    "v": v,
                    "i": p.idx,
                })
            })
            .collect();
        let touches: Vec<Value> = self
            .touches
            .iter()
            .map(|t| json!({ "m": t.mesh, "e": t.event, "t": t.tris.iter().flatten().flat_map(|p| [q(p.x), q(p.y), q(p.z)]).collect::<Vec<_>>() }))
            .collect();
        json!({
            "q": QUANT,
            "w": q(self.sheet.w),
            "h": q(self.sheet.h),
            "front": q(self.sheet.front),
            "behind": q(self.sheet.behind),
            "meshes": self.meshes.len(),
            "parts": parts,
            "touch": touches,
            "texts": texts,
            "scripts": self.scripts,
            // (the script textures that are `[htmltexture]` pages: a tap on one goes to it)
            "pages": self.scripts.iter().filter(|n| pages.contains(n)).collect::<Vec<_>>(),
        })
    }

    /// Where its meshes are now (mesh to the bus's own frame, `now`, in the order of
    /// [`Form::meshes`]) as movements in the face's frame since the form was made, the
    /// device's own taken out: (index, the movement as 3x4, by rows) for those that moved.
    pub(crate) fn moved(&self, now: &[Mat4]) -> Vec<(usize, [f32; 12])> {
        let s = self.sheet.to_local();
        let si = s.inverse();
        let device = self.anchor.and_then(|a| Some(now.get(a)?.mul_mat4(&self.built[a].inverse()))).filter(|m| m.is_finite() && m.determinant().abs() > 1e-9).unwrap_or(Mat4::IDENTITY);
        let undo = device.inverse();
        let mut out = Vec::new();
        for (k, built) in self.built.iter().enumerate() {
            let Some(n) = now.get(k) else { continue };
            let d = s * undo * *n * built.inverse() * si;
            if !d.is_finite() {
                continue;
            }
            let r = d.transpose().to_cols_array();
            let rows: [f32; 12] = std::array::from_fn(|i| r[i]);
            let id = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
            if rows.iter().zip(id).any(|(a, b)| (a - b).abs() > STILL) {
                out.push((k, rows));
            }
        }
        out
    }

    /// The touch area under `at` (across, down, m) with the meshes `shown` and moved by
    /// `moved` (as [`Form::moved`]): the one nearest there, as a ray straight at the face
    /// finds it in the cab (the page does the same).
    #[cfg(test)]
    pub(crate) fn touch_at(&self, at: Vec2, shown: &dyn Fn(usize) -> bool, moved: &[(usize, [f32; 12])]) -> Option<&Touch> {
        let mut best: Option<(f32, &Touch)> = None;
        for t in self.touches.iter().filter(|t| shown(t.mesh)) {
            let m = moved.iter().find(|m| m.0 == t.mesh).map(|m| m.1);
            let at_now = |p: Vec3| match m {
                Some(r) => Vec3::new(r[0] * p.x + r[1] * p.y + r[2] * p.z + r[3], r[4] * p.x + r[5] * p.y + r[6] * p.z + r[7], r[8] * p.x + r[9] * p.y + r[10] * p.z + r[11]),
                None => p,
            };
            for tri in &t.tris {
                if let Some(z) = depth_at(&tri.map(at_now), at) {
                    if best.is_none_or(|b| z > b.0) {
                        best = Some((z, t));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }
}

// ---------------------------------------------------------------------------------------
// the bus

/// The form of the device on `sheet` in the bus `v` (the content `root`, for its textures),
/// moving with mesh `anchor`.
pub(crate) fn of_vehicle(v: &omsi_sim::VehicleInstance, root: &Path, sheet: &Sheet, anchor: Option<usize>) -> Option<Form> {
    let ty = &v.ty;
    let unturn = v.body_rotation().inverse();
    let to = sheet.to_local();
    let mut dirs = ty.texture_dirs(root);
    let (subst, scheme_dir) = match v.host.paint_scheme.flatten() {
        Some(i) => ty.scheme_substitutions(i),
        None => (ty.default_substitutions(root), None),
    };
    if let Some(d) = scheme_dir {
        dirs.insert(0, d);
    }
    let dirs_ref: Vec<&Path> = dirs.iter().map(PathBuf::as_path).collect();
    let mut found: std::collections::HashMap<String, Option<PathBuf>> = Default::default();
    let mut ins: Vec<MeshIn> = Vec::new();
    for (i, vm) in ty.meshes.iter().enumerate() {
        let Some(def) = ty.model.meshes.get(vm.def_index) else { continue };
        if !(def.viewpoint == 0 || def.viewpoint & 2 != 0) || def.lod > 0 || def.is_shadow || !vm.skin.is_empty() || vm.data.indices.is_empty() {
            continue;
        }
        let xf = unturn * v.mesh_local_transform(i);
        // (near the face at all: its sphere)
        let Some(&(c, r)) = ty.mesh_bounds.get(i) else { continue };
        let cs = to.transform_point3(xf.transform_point3(c));
        let r = r * Mat3::from_mat4(xf).to_cols_array().iter().fold(0.0f32, |a, b| a.max(b.abs())).max(1.0) * 1.8;
        if cs.x + r < 0.0 || cs.x - r > sheet.w || cs.y + r < 0.0 || cs.y - r > sheet.h || cs.z - r > sheet.front || cs.z + r < -sheet.behind {
            continue;
        }
        let slots: Vec<Option<Slot>> = vm
            .materials
            .iter()
            .enumerate()
            .map(|(slot, m)| {
                let ov: Vec<&omsi_model::MaterialDef> = vm.overrides.iter().filter(|o| !o.item && omsi_sim::vehicle::override_slot(&vm.materials, o) == Some(slot)).collect();
                let text = ov.iter().find_map(|o| o.use_text_texture).filter(|n| *n >= 0 && (*n as usize) < v.text_textures.len());
                let script = ov.iter().find_map(|o| o.use_script_texture).filter(|n| *n >= 0 && (*n as usize) < v.host.script_textures.len());
                let name = m.texture.trim();
                if name.to_ascii_lowercase().starts_with("reflexion") {
                    // (a mirror: the cab's own picture, nothing of a device)
                    return None;
                }
                let tex = if let Some(n) = text {
                    Tex::Text(n as usize)
                } else if let Some(n) = script {
                    Tex::Script(n as usize)
                } else if crate::scene::is_null_texture(name) || ty.texchange(name).is_some() {
                    Tex::Colour
                } else {
                    let named = subst.get(&name.to_ascii_lowercase()).cloned().unwrap_or_else(|| name.to_string());
                    match found.entry(named.clone()).or_insert_with(|| omsi_texture::find_texture(&named, &dirs_ref)) {
                        Some(p) => Tex::File(p.clone()),
                        None => Tex::Colour,
                    }
                };
                let textured = tex != Tex::Colour;
                let all = ov.iter().find_map(|o| o.allcolor);
                let (diffuse, emissive) = match all {
                    Some(a) => ([a[0], a[1], a[2], a[3]], [a[10], a[11], a[12]]),
                    None => (m.diffuse, m.emissive),
                };
                let c01 = |x: f32| if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 };
                // (a texture as it is - the cab's light on it is not the page's; a plain
                // colour as its material has it)
                let colour = if textured { [1.0, 1.0, 1.0, 1.0] } else { [c01(diffuse[0] + emissive[0]), c01(diffuse[1] + emissive[1]), c01(diffuse[2] + emissive[2]), c01(diffuse[3])] };
                let alpha = match crate::scene::material_alpha(&vm.materials, slot, &vm.overrides) {
                    omsi_render::AlphaMode::Opaque => 0,
                    omsi_render::AlphaMode::Test => 1,
                    omsi_render::AlphaMode::Blend => 2,
                };
                let wrap = !matches!(ov.iter().rev().map(|o| o.tex_address).find(|a| *a != omsi_model::TexAddress::Wrap), Some(omsi_model::TexAddress::Clamp | omsi_model::TexAddress::Border));
                let z_write = !ov.iter().any(|o| o.no_z_write);
                Some(Slot { tex, colour, alpha, wrap, z_write })
            })
            .collect();
        let event = def.mouse_event.as_deref().map(str::trim).filter(|e| !e.is_empty()).map(str::to_string).filter(|_| ty.mesh_bounds.get(i).is_some_and(|b| b.1 <= super::screens::BIG));
        ins.push(MeshIn { mesh: i, xf, data: &vm.data, slots, event });
    }
    build(sheet, &ins, anchor)
}

/// Where the meshes of a form are now (mesh to the bus's own frame).
pub(crate) fn now_of(v: &omsi_sim::VehicleInstance, form: &Form) -> Vec<Mat4> {
    let unturn = v.body_rotation().inverse();
    form.meshes.iter().map(|&i| if i < v.ty.meshes.len() { unturn * v.mesh_local_transform(i) } else { Mat4::IDENTITY }).collect()
}

/// The string text texture `n` shows now.
pub(crate) fn text_now(v: &omsi_sim::VehicleInstance, n: usize) -> String {
    match v.text_textures.get(n) {
        Some(t) => t.last_text.clone().unwrap_or_else(|| v.text_texture_string(&t.def.variable)),
        None => String::new(),
    }
}

/// The live state of a form (`/api/live`): which of its meshes show (`s`, a 1 or 0 each),
/// those that moved (`m`: index and movement), the strings of its text textures (`t`), the
/// parts faded by `[alphascale]` (`a`: index and opacity).
pub(crate) fn live(v: &omsi_sim::VehicleInstance, form: &Form) -> Value {
    let shown: String = form.meshes.iter().map(|&i| if v.mesh_props.get(i).is_none_or(|p| p.visible) { '1' } else { '0' }).collect();
    let moved: Vec<Value> = form.moved(&now_of(v, form)).into_iter().map(|(k, r)| json!([k, r.map(|x| (x * 100_000.0).round() / 100_000.0)])).collect();
    let texts: serde_json::Map<String, Value> = form.texts.iter().map(|&n| (n.to_string(), Value::String(text_now(v, n)))).collect();
    let faded: Vec<Value> = form
        .parts
        .iter()
        .enumerate()
        .filter_map(|(k, p)| {
            let a = *v.mesh_props.get(*form.meshes.get(p.mesh)?)?.slot_alpha.get(p.slot)?;
            ((a - 1.0).abs() > 1e-3).then(|| json!([k, (a.clamp(0.0, 1.0) * 1000.0).round() / 1000.0]))
        })
        .collect();
    json!({ "s": shown, "m": moved, "t": texts, "a": faded })
}

// ---------------------------------------------------------------------------------------
// fonts

/// An `.oft` font as the page draws with it (`/api/font`): its height, the gap after each
/// letter, the width of a space, its glyphs in the file's order (character, x0, x1, y), and
/// for the letters of the other code pages the glyph that stands in for them
/// (`Font::glyph`); its bitmap goes apart as a picture.
pub(crate) fn font_json(a: &omsi_content::font::FontAtlas) -> Value {
    let f = &a.font;
    let glyphs: Vec<Value> = f.chars.iter().map(|g| json!([g.ch.to_string(), g.x0, g.x1, g.y])).collect();
    let mut alias = serde_json::Map::new();
    // (a letter of another code page is the same byte as one of the font's: the variants of
    // the font's own letters are all the letters that may stand in for one of them)
    let mut seen = std::collections::HashSet::new();
    for c in f.chars.iter().flat_map(|g| omsi_cfg::codepage::char_variants(g.ch)) {
        if c.is_whitespace() || !seen.insert(c) || f.chars.iter().any(|g| g.ch == c) {
            continue;
        }
        if let Some(g) = f.glyph(c) {
            if let Some(k) = f.chars.iter().position(|x| std::ptr::eq(x, g)) {
                alias.insert(c.to_string(), json!(k));
            }
        }
    }
    json!({ "h": f.height, "gap": f.gap, "space": f.space_width(), "iw": a.width, "ih": a.height, "g": glyphs, "alias": alias })
}

/// A font's bitmap for the page: its colours, and as the alpha the coverage (the alpha
/// bitmap's red).
pub(crate) fn font_rgba(a: &omsi_content::font::FontAtlas) -> Vec<u8> {
    let n = (a.width * a.height) as usize;
    let mut out = vec![0u8; n * 4];
    for i in 0..n {
        let c = a.color.get(i * 4..i * 4 + 3).unwrap_or(&[0, 0, 0]);
        out[i * 4..i * 4 + 3].copy_from_slice(c);
        out[i * 4 + 3] = a.alpha.get(i * 4).copied().unwrap_or(0);
    }
    out
}

/// A text texture's definition for the page (`texts` of the form): its size, font (the id the
/// server has it under, or -1), colour, whether the font's own colours are taken, its
/// placement, and - drawn in a display font the player chose - the height of the line of the
/// bus's own font that font is fitted to (`lh`, `FontAtlas::render_fitted`; 0: none).
pub(crate) fn text_json(s: &omsi_sim::texttex::TextTextureState, n: usize, font: Option<usize>) -> Value {
    let t = &s.def;
    json!({
        "n": n,
        "w": t.width.max(1),
        "h": t.height.max(1),
        "font": font.map_or(-1, |f| f as i64),
        "rgb": [t.color[0].clamp(0.0, 255.0) as u8, t.color[1].clamp(0.0, 255.0) as u8, t.color[2].clamp(0.0, 255.0) as u8],
        "full": t.full_color,
        "o": t.orientation,
        "g": t.grid,
        "lh": s.fit.unwrap_or(0),
    })
}

// ---------------------------------------------------------------------------------------
// the page's drawing, on the CPU (for the tests: the same rules as app.js)

#[cfg(test)]
pub(crate) mod raster {
    use super::*;

    /// A picture a part samples: RGBA, its size, and the part of the texture (u0, v0, u1, v1)
    /// it is.
    pub(crate) struct Pic<'a> {
        pub w: u32,
        pub h: u32,
        pub rgba: &'a [u8],
        pub uv: [f32; 4],
        /// Sampled by the nearest pixel (a display's), else between them.
        pub nearest: bool,
    }

    fn sample(p: &Pic, u: f32, v: f32, wrap: bool) -> [f32; 4] {
        let (mut u, mut v) = if wrap { (u - u.floor(), v - v.floor()) } else { (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0)) };
        u = (u - p.uv[0]) / (p.uv[2] - p.uv[0]).max(1e-9);
        v = (v - p.uv[1]) / (p.uv[3] - p.uv[1]).max(1e-9);
        let (fx, fy) = (u * p.w as f32 - 0.5, v * p.h as f32 - 0.5);
        let px = |x: i64, y: i64| -> [f32; 4] {
            let (x, y) = (x.clamp(0, p.w as i64 - 1) as usize, y.clamp(0, p.h as i64 - 1) as usize);
            let i = (y * p.w as usize + x) * 4;
            [p.rgba[i] as f32 / 255.0, p.rgba[i + 1] as f32 / 255.0, p.rgba[i + 2] as f32 / 255.0, p.rgba[i + 3] as f32 / 255.0]
        };
        if p.nearest {
            return px((fx + 0.5).floor() as i64, (fy + 0.5).floor() as i64);
        }
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (a, b, c, d) = (px(x0 as i64, y0 as i64), px(x0 as i64 + 1, y0 as i64), px(x0 as i64, y0 as i64 + 1), px(x0 as i64 + 1, y0 as i64 + 1));
        std::array::from_fn(|k| (a[k] * (1.0 - tx) + b[k] * tx) * (1.0 - ty) + (c[k] * (1.0 - tx) + d[k] * tx) * ty)
    }

    /// The form drawn at `scale` pixels a metre: `shown(mesh)`, `moved` as [`Form::moved`],
    /// `pic(part)` its picture. RGBA on black.
    pub(crate) fn draw<'a>(form: &Form, scale: f32, shown: &dyn Fn(usize) -> bool, moved: &[(usize, [f32; 12])], pic: &dyn Fn(&Part) -> Option<Pic<'a>>) -> (u32, u32, Vec<u8>) {
        let (w, h) = ((form.sheet.w * scale).round().max(1.0) as u32, (form.sheet.h * scale).round().max(1.0) as u32);
        let mut col = vec![[0.0f32; 3]; (w * h) as usize];
        let mut depth = vec![f32::MIN; (w * h) as usize];
        for p in &form.parts {
            if !shown(p.mesh) {
                continue;
            }
            let m = moved.iter().find(|m| m.0 == p.mesh).map(|m| m.1);
            let at = |q: &[f32; 5]| -> [f32; 5] {
                match m {
                    Some(r) => [r[0] * q[0] + r[1] * q[1] + r[2] * q[2] + r[3], r[4] * q[0] + r[5] * q[1] + r[6] * q[2] + r[7], r[8] * q[0] + r[9] * q[1] + r[10] * q[2] + r[11], q[3], q[4]],
                    None => *q,
                }
            };
            let picture = pic(p);
            for t in p.idx.chunks_exact(3) {
                let c = [at(&p.verts[t[0] as usize]), at(&p.verts[t[1] as usize]), at(&p.verts[t[2] as usize])];
                let s: [Vec2; 3] = c.map(|q| Vec2::new(q[0] * scale, q[1] * scale));
                let det = (s[1] - s[0]).perp_dot(s[2] - s[0]);
                if det.abs() < 1e-9 {
                    continue;
                }
                let (lo, hi) = (s[0].min(s[1]).min(s[2]), s[0].max(s[1]).max(s[2]));
                let (x0, y0) = (lo.x.floor().max(0.0) as u32, lo.y.floor().max(0.0) as u32);
                let (x1, y1) = (hi.x.ceil().min(w as f32) as u32, hi.y.ceil().min(h as f32) as u32);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                        let w1 = (q - s[0]).perp_dot(s[2] - s[0]) / det;
                        let w2 = (s[1] - s[0]).perp_dot(q - s[0]) / det;
                        let w0 = 1.0 - w1 - w2;
                        if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                            continue;
                        }
                        let ip = |k: usize| c[0][k] * w0 + c[1][k] * w1 + c[2][k] * w2;
                        let z = ip(2);
                        let i = (y * w + x) as usize;
                        if z < depth[i] - 1e-6 {
                            continue;
                        }
                        let mut rgba = p.colour;
                        if let Some(pc) = picture.as_ref() {
                            let s = sample(pc, ip(3), ip(4), p.wrap);
                            rgba = [rgba[0] * s[0], rgba[1] * s[1], rgba[2] * s[2], s[3]];
                        }
                        let a = match p.alpha {
                            0 => 1.0,
                            1 => {
                                if rgba[3] < 0.5 {
                                    continue;
                                }
                                1.0
                            }
                            _ => rgba[3],
                        };
                        for k in 0..3 {
                            col[i][k] = rgba[k] * a + col[i][k] * (1.0 - a);
                        }
                        if p.z_write {
                            depth[i] = z;
                        }
                    }
                }
            }
        }
        let out = col.iter().flat_map(|c| [(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, 255]).collect();
        (w, h, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device facing -y (towards a driver at y < 0), upright: 0.2 m wide, 0.1 m high, its
    /// top left corner at (0, 1, 1.5).
    fn sheet() -> Sheet {
        Sheet::of(Vec3::new(0.0, 1.0, 1.5), Vec3::X * 0.2, -Vec3::Z * 0.1, -Vec3::Y).unwrap()
    }

    /// A quad in the plane y = `y` from (x0, z0) to (x1, z1), wound to face -y (clockwise as
    /// seen from there), its texture's (0, 0) at the top left.
    fn quad(x0: f32, z0: f32, x1: f32, z1: f32, y: f32) -> omsi_geometry::MeshData {
        omsi_geometry::MeshData {
            positions: vec![Vec3::new(x0, y, z1), Vec3::new(x1, y, z1), Vec3::new(x1, y, z0), Vec3::new(x0, y, z0)],
            normals: vec![-Vec3::Y; 4],
            uvs: vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)],
            ranges: vec![(0, 6, 0)],
            // (seen from -y: top left, top right, bottom right - clockwise)
            indices: vec![0, 1, 2, 0, 2, 3],
            one_sided: true,
        }
    }

    fn plain(tex: Tex, alpha: u8) -> Option<Slot> {
        Some(Slot { tex, colour: [1.0; 4], alpha, wrap: true, z_write: alpha != 2 })
    }

    #[test]
    fn the_sheet_lays_the_face_flat_from_its_top_left_corner() {
        let s = sheet();
        let to = s.to_local();
        for (p, want) in [(Vec3::new(0.0, 1.0, 1.5), Vec3::ZERO), (Vec3::new(0.2, 1.0, 1.5), Vec3::new(0.2, 0.0, 0.0)), (Vec3::new(0.2, 1.0, 1.4), Vec3::new(0.2, 0.1, 0.0)), (Vec3::new(0.1, 0.97, 1.45), Vec3::new(0.1, 0.05, 0.03))] {
            assert!(to.transform_point3(p).distance(want) < 1e-5, "{p:?}: {:?}", to.transform_point3(p));
        }
        assert!((s.w - 0.2).abs() < 1e-6 && (s.h - 0.1).abs() < 1e-6);
        assert!(s.front >= FRONT_MIN && s.behind >= BEHIND_MIN);
        // (a face leaning back keeps its own down as the sheet's)
        let n = Vec3::new(0.0, -1.0, 0.5).normalize();
        let down = Vec3::new(0.0, -0.5, -1.0).normalize() * 0.1;
        let t = Sheet::of(Vec3::ZERO, Vec3::X * 0.2, down, n).unwrap();
        assert!(t.to_local().transform_point3(down).distance(Vec3::new(0.0, 0.1, 0.0)) < 1e-5);
        assert!(Sheet::of(Vec3::ZERO, Vec3::X * 0.2, n * 0.1, n).is_none());
    }

    #[test]
    fn a_face_and_a_key_before_it_are_laid_flat_and_the_key_is_a_touch_area() {
        let s = sheet();
        let face = quad(0.0, 1.4, 0.2, 1.5, 1.0);
        // a key 1 cm before the face, at x 0.15..0.18, z 1.42..1.44
        let key = quad(0.15, 1.42, 0.18, 1.44, 0.99);
        // (a mesh seen from behind, as the ALMEX's click spots are: not drawn, still a touch)
        let mut spot = quad(0.02, 1.46, 0.06, 1.48, 0.995);
        spot.indices = vec![0, 2, 1, 0, 3, 2];
        // (and one far off the face: not part of it)
        let far = quad(0.5, 1.0, 0.6, 1.1, 1.0);
        let ins = [
            MeshIn { mesh: 3, xf: Mat4::IDENTITY, data: &face, slots: vec![plain(Tex::File("face.png".into()), 0)], event: None },
            MeshIn { mesh: 7, xf: Mat4::IDENTITY, data: &key, slots: vec![plain(Tex::Colour, 0)], event: Some("ibis_1".into()) },
            MeshIn { mesh: 8, xf: Mat4::IDENTITY, data: &spot, slots: vec![plain(Tex::Colour, 0)], event: Some("almex_click".into()) },
            MeshIn { mesh: 9, xf: Mat4::IDENTITY, data: &far, slots: vec![plain(Tex::Colour, 0)], event: Some("far".into()) },
        ];
        let f = build(&s, &ins, Some(3)).unwrap();
        assert_eq!(f.meshes, vec![3, 7, 8]);
        assert_eq!(f.parts.len(), 2, "the face and the key; the spot from behind is not drawn");
        assert_eq!(f.anchor, Some(0));
        assert_eq!(f.files, vec![FileUse { path: "face.png".into(), uv: [0.0, 0.0, 1.0, 1.0] }]);
        // the face fills the sheet, its texture's top left at the sheet's
        let p = &f.parts[0];
        let tl = p.verts.iter().find(|v| v[3] == 0.0 && v[4] == 0.0).unwrap();
        assert!(tl[0].abs() < 1e-5 && tl[1].abs() < 1e-5 && tl[2].abs() < 1e-5, "{tl:?}");
        let br = p.verts.iter().find(|v| v[3] == 1.0 && v[4] == 1.0).unwrap();
        assert!((br[0] - 0.2).abs() < 1e-5 && (br[1] - 0.1).abs() < 1e-5, "{br:?}");
        // the key: before the face, where it is
        let k = &f.parts[1];
        assert!(k.verts.iter().all(|v| (v[2] - 0.01).abs() < 1e-5 && v[0] > 0.149 && v[0] < 0.181 && v[1] > 0.059 && v[1] < 0.081));
        // two touch areas, the nearest last; a tap on the key finds it, beside it nothing
        assert_eq!(f.touches.len(), 2);
        assert_eq!(f.touches[1].event, "ibis_1");
        let all = |_: usize| true;
        assert_eq!(f.touch_at(Vec2::new(0.165, 0.07), &all, &[]).map(|t| t.event.as_str()), Some("ibis_1"));
        assert_eq!(f.touch_at(Vec2::new(0.04, 0.03), &all, &[]).map(|t| t.event.as_str()), Some("almex_click"));
        assert!(f.touch_at(Vec2::new(0.1, 0.05), &all, &[]).is_none());
        // a hidden key is no touch area
        let shown = |m: usize| m != 1;
        assert!(f.touch_at(Vec2::new(0.165, 0.07), &shown, &[]).is_none());
        // the page's form: whole numbers of hundredths of a millimetre
        let j = f.json(&[5], &[], &[]);
        assert_eq!(j["w"], 20000);
        assert_eq!(j["parts"][0]["tex"]["f"], 5);
        assert_eq!(j["touch"][1]["e"], "ibis_1");
    }

    #[test]
    fn what_lies_far_before_the_face_is_cut_away_and_what_turns_its_back_is_not_drawn() {
        let s = sheet();
        // a panel 0.5 m before the face (the steering wheel, a pillar): not drawn
        let before = quad(0.0, 1.4, 0.2, 1.5, 0.5);
        // one reaching from behind the face to far before it: cut at the front
        let mut slant = quad(0.0, 1.4, 0.2, 1.5, 1.0);
        slant.positions[0].y = 1.0 - 0.6;
        slant.positions[1].y = 1.0 - 0.6;
        // a mesh not one-sided turned away: drawn all the same
        let mut both = quad(0.05, 1.42, 0.08, 1.45, 0.998);
        both.indices = vec![0, 2, 1, 0, 3, 2];
        both.one_sided = false;
        let ins = [
            MeshIn { mesh: 1, xf: Mat4::IDENTITY, data: &before, slots: vec![plain(Tex::Colour, 0)], event: None },
            MeshIn { mesh: 2, xf: Mat4::IDENTITY, data: &slant, slots: vec![plain(Tex::Colour, 0)], event: None },
            MeshIn { mesh: 4, xf: Mat4::IDENTITY, data: &both, slots: vec![plain(Tex::Colour, 0)], event: None },
        ];
        let f = build(&s, &ins, None).unwrap();
        assert_eq!(f.meshes, vec![2, 4]);
        assert!(f.parts[0].verts.iter().all(|v| v[2] <= s.front + 1e-5));
        assert!(f.parts[0].verts.iter().any(|v| (v[2] - s.front).abs() < 1e-4), "cut at the front");
        // (a mirrored mesh, by its transform, is drawn from both sides too)
        let face = quad(0.0, 1.4, 0.2, 1.5, 1.0);
        let mirror = Mat4::from_translation(Vec3::new(0.2, 0.0, 0.0)) * Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
        let f = build(&s, &[MeshIn { mesh: 0, xf: mirror, data: &face, slots: vec![plain(Tex::Colour, 0)], event: None }], None).unwrap();
        assert_eq!(f.parts[0].idx.len(), 6);
        // a lamp 10 cm before the face and a cover 3 cm before all of it: something of the
        // cab between the device and the driver, not drawn
        let lamp = quad(0.05, 1.42, 0.08, 1.45, 0.9);
        let cover = quad(0.0, 1.4, 0.2, 1.5, 0.97);
        let key = quad(0.15, 1.42, 0.18, 1.44, 0.99);
        let face = quad(0.0, 1.4, 0.2, 1.5, 1.0);
        let ins = [
            MeshIn { mesh: 0, xf: Mat4::IDENTITY, data: &face, slots: vec![plain(Tex::Colour, 0)], event: None },
            MeshIn { mesh: 1, xf: Mat4::IDENTITY, data: &lamp, slots: vec![plain(Tex::Colour, 0)], event: None },
            MeshIn { mesh: 2, xf: Mat4::IDENTITY, data: &cover, slots: vec![plain(Tex::Colour, 0)], event: Some("cp_lenkrad".into()) },
            MeshIn { mesh: 3, xf: Mat4::IDENTITY, data: &key, slots: vec![plain(Tex::Colour, 0)], event: Some("key".into()) },
        ];
        let f = build(&s, &ins, None).unwrap();
        assert_eq!(f.meshes, vec![0, 3]);
        assert_eq!(f.touches.len(), 1);
        // nothing on the face: no form
        assert!(build(&s, &[MeshIn { mesh: 1, xf: Mat4::IDENTITY, data: &before, slots: vec![plain(Tex::Colour, 0)], event: None }], None).is_none());
    }

    #[test]
    fn texture_coordinates_of_one_turn_come_into_the_first_and_the_file_part_used_is_kept() {
        let s = sheet();
        // a text texture's mesh addressed a turn below (v -0.57..-0.47, as the Hamburg
        // buses' are), and a picture showing a corner of a big texture
        let mut text = quad(0.02, 1.43, 0.18, 1.47, 0.999);
        text.uvs = vec![Vec2::new(0.0, -0.57), Vec2::new(1.0, -0.57), Vec2::new(1.0, -0.47), Vec2::new(0.0, -0.47)];
        let mut pic = quad(0.0, 1.4, 0.2, 1.5, 1.0);
        pic.uvs = vec![Vec2::new(0.1, 0.2), Vec2::new(0.4, 0.2), Vec2::new(0.4, 0.3), Vec2::new(0.1, 0.3)];
        let mut tiled = quad(0.0, 1.4, 0.1, 1.45, 1.001);
        tiled.uvs = vec![Vec2::new(0.0, 0.0), Vec2::new(3.0, 0.0), Vec2::new(3.0, 1.0), Vec2::new(0.0, 1.0)];
        let ins = [
            MeshIn { mesh: 0, xf: Mat4::IDENTITY, data: &pic, slots: vec![plain(Tex::File("big.dds".into()), 0)], event: None },
            MeshIn { mesh: 1, xf: Mat4::IDENTITY, data: &text, slots: vec![plain(Tex::Text(4), 2)], event: None },
            MeshIn { mesh: 2, xf: Mat4::IDENTITY, data: &tiled, slots: vec![plain(Tex::File("tile.bmp".into()), 0)], event: None },
        ];
        let f = build(&s, &ins, None).unwrap();
        let t = f.parts.iter().find(|p| p.tex == PartTex::Text(4)).unwrap();
        assert!(t.verts.iter().all(|v| v[4] > 0.42 && v[4] < 0.54), "{:?}", t.verts);
        assert_eq!(f.texts, vec![4]);
        assert!(f.files[0].uv.iter().zip([0.1, 0.2, 0.4, 0.3]).all(|(a, b)| (a - b).abs() < 1e-5), "{:?}", f.files[0].uv);
        assert_eq!(f.files[1].uv, [0.0, 0.0, 1.0, 1.0]);
        // (the blended one after the opaque ones)
        assert_eq!(f.parts.last().unwrap().tex, PartTex::Text(4));
    }

    #[test]
    fn a_key_pressed_in_moves_on_the_page_and_the_device_moving_with_its_door_does_not() {
        let s = sheet();
        let face = quad(0.0, 1.4, 0.2, 1.5, 1.0);
        let key = quad(0.15, 1.42, 0.18, 1.44, 0.99);
        let ins = [
            MeshIn { mesh: 3, xf: Mat4::IDENTITY, data: &face, slots: vec![plain(Tex::Colour, 0)], event: None },
            MeshIn { mesh: 7, xf: Mat4::IDENTITY, data: &key, slots: vec![plain(Tex::Colour, 0)], event: Some("ibis_1".into()) },
        ];
        let f = build(&s, &ins, Some(3)).unwrap();
        assert!(f.moved(&[Mat4::IDENTITY, Mat4::IDENTITY]).is_empty());
        // the key pressed 5 mm into the face (+y in the bus: away from the viewer)
        let pressed = Mat4::from_translation(Vec3::new(0.0, 0.005, 0.0));
        let m = f.moved(&[Mat4::IDENTITY, pressed]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].0, 1);
        assert!((m[0].1[11] + 0.005).abs() < 1e-6, "back into the face: {:?}", m[0].1);
        assert!(m[0].1[3].abs() < 1e-6 && m[0].1[7].abs() < 1e-6);
        // the whole device swung with its door: nothing moved on the page
        let door = Mat4::from_rotation_z(0.4) * Mat4::from_translation(Vec3::new(0.3, 0.1, 0.0));
        assert!(f.moved(&[door, door]).is_empty());
        // ...and the key pressed in on the swung door is pressed in all the same
        let m = f.moved(&[door, door * pressed]);
        assert!(m.len() == 1 && (m[0].1[11] + 0.005).abs() < 1e-5, "{m:?}");
        // a key slid 1 cm across moves its touch area with it
        let slid = [(1usize, [1.0, 0.0, 0.0, 0.01, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0])];
        let all = |_: usize| true;
        assert!(f.touch_at(Vec2::new(0.152, 0.07), &all, &slid).is_none());
        assert!(f.touch_at(Vec2::new(0.185, 0.07), &all, &slid).is_some());
    }

    #[test]
    fn the_cpu_drawing_puts_the_key_over_the_face_and_samples_the_texture_upright() {
        let s = sheet();
        let face = quad(0.0, 1.4, 0.2, 1.5, 1.0);
        let key = quad(0.15, 1.42, 0.18, 1.44, 0.99);
        let ins = [
            MeshIn { mesh: 3, xf: Mat4::IDENTITY, data: &face, slots: vec![plain(Tex::File("face.png".into()), 0)], event: None },
            MeshIn { mesh: 7, xf: Mat4::IDENTITY, data: &key, slots: vec![Some(Slot { tex: Tex::Colour, colour: [1.0, 0.0, 0.0, 1.0], alpha: 0, wrap: true, z_write: true })], event: None },
        ];
        let f = build(&s, &ins, None).unwrap();
        // a 2x2 texture: white top, blue bottom
        let tex = [255u8, 255, 255, 255, 255, 255, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255];
        let pic = |p: &Part| (p.tex == PartTex::File(0)).then(|| raster::Pic { w: 2, h: 2, rgba: &tex, uv: [0.0, 0.0, 1.0, 1.0], nearest: true });
        let (w, h, px) = raster::draw(&f, 1000.0, &|_| true, &[], &pic);
        assert_eq!((w, h), (200, 100));
        let at = |x: u32, y: u32| &px[((y * w + x) * 4) as usize..((y * w + x) * 4 + 3) as usize];
        assert_eq!(at(10, 10), &[255, 255, 255]);
        assert_eq!(at(10, 90), &[0, 0, 255]);
        assert_eq!(at(165, 70), &[255, 0, 0], "the key in front");
        // hidden, the face shows there
        let (_, _, px) = raster::draw(&f, 1000.0, &|m| m != 1, &[], &pic);
        assert_eq!(&px[((70 * w + 165) * 4) as usize..((70 * w + 165) * 4 + 3) as usize], &[0, 0, 255]);
    }

    #[test]
    fn a_tap_finds_the_switch_nearest_to_the_finger_where_it_is() {
        let s = sheet();
        // a big switch (the whole ticket table, to swing it) lying behind a key on it
        let table = quad(0.0, 1.4, 0.2, 1.5, 1.005);
        let key = quad(0.15, 1.42, 0.18, 1.44, 0.99);
        let face = quad(0.0, 1.4, 0.2, 1.5, 1.0);
        let ins = [
            MeshIn { mesh: 1, xf: Mat4::IDENTITY, data: &face, slots: vec![plain(Tex::Colour, 0)], event: None },
            MeshIn { mesh: 2, xf: Mat4::IDENTITY, data: &table, slots: vec![None], event: Some("cp_zahltisch".into()) },
            MeshIn { mesh: 3, xf: Mat4::IDENTITY, data: &key, slots: vec![plain(Tex::Colour, 0)], event: Some("btn_1".into()) },
        ];
        let f = build(&s, &ins, None).unwrap();
        let all = |_: usize| true;
        assert_eq!(f.touch_at(Vec2::new(0.165, 0.07), &all, &[]).map(|t| t.event.as_str()), Some("btn_1"));
        assert_eq!(f.touch_at(Vec2::new(0.05, 0.07), &all, &[]).map(|t| t.event.as_str()), Some("cp_zahltisch"));
        assert!(f.touch_at(Vec2::new(0.25, 0.07), &all, &[]).is_none());
        assert!(depth_at(&[Vec3::ZERO, Vec3::X, Vec3::Y], Vec2::new(0.6, 0.6)).is_none());
        assert_eq!(depth_at(&[Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 1.0), Vec3::new(0.0, 1.0, 3.0)], Vec2::new(0.25, 0.5)), Some(2.0));
    }

    /// Real buses of the installed OMSI 2 (`OMSI_ROOT`; `OMSI_FORM_BUSES`, `;` between them -
    /// the Hamburg electric bus with its ALMEX, the MAN SL with its IBIS and ticket table, the
    /// SD202 and the Hamburg city bus unless said otherwise; skipped without them): every
    /// device with a form drawn on the CPU as the page draws it (written as PNGs to
    /// `OMSI_FORM_SHOTS` when that is set, and the forms with their textures and fonts to
    /// `OMSI_FORM_DUMP` for a page without the game), not one colour all over; and the middle
    /// of each touch area shown is that area on the page and, clicked into the cab straight
    /// at the face, the same switch.
    #[test]
    fn real_buses_devices_are_drawn_flat_and_their_touch_areas_lie_on_their_switches() {
        let Some(root) = omsi_cfg::env::var_os("OMSI_ROOT").map(PathBuf::from) else {
            eprintln!("skipped: no OMSI_ROOT");
            return;
        };
        let buses = std::env::var("OMSI_FORM_BUSES").unwrap_or_else(|_| "Vehicles/HH20_EBus2021/HHEBus2021_main.bus;Vehicles/MAN_SL_SG/MAN_SL_standard.bus;Vehicles/MAN_SD202/MAN_D92.bus;Vehicles/HH109_Stadtbus_HHA/HHStadtbus_HHA.bus".into());
        let shots = std::env::var_os("OMSI_FORM_SHOTS").map(PathBuf::from);
        let dump = std::env::var_os("OMSI_FORM_DUMP").map(PathBuf::from);
        // (the devices on, as their scripts have them once the driver switched them on)
        let set = std::env::var("OMSI_FORM_SET").unwrap_or_else(|_| "almex_ein=1,almex_standby=1,almex_standby_zeit=1000000,almex_menu=0,almex_menu_req=0,elec_busbar_main=1,bremse_halte=1,cockpit_batterietrennschalter=1,ibis_power=1".into());
        let (mut drawn, mut touches, mut hit) = (0, 0, 0);
        for bus in buses.split(';').filter(|b| !b.trim().is_empty()) {
            let path = root.join(bus.trim());
            if !path.is_file() {
                eprintln!("skipped: no {}", path.display());
                continue;
            }
            let clock = omsi_sim::SimClock { time: 6.0 * 3600.0 + 57.0 * 60.0 + 7.0, ..Default::default() };
            let vt = std::sync::Arc::new(omsi_sim::VehicleType::load(&root, &path).expect("the bus"));
            let mut vehicle = omsi_sim::VehicleInstance::new(vt.clone(), omsi_sim::VehicleHost::new(clock.clone()));
            let mut fonts = omsi_sim::texttex::FontLibrary::new(&root);
            vehicle.init_text_textures(&mut fonts, &|p| omsi_texture::decode_file(p).ok().map(|i| (i.width, i.height, i.rgba)));
            let set_all = |vehicle: &mut omsi_sim::VehicleInstance| {
                for kv in set.split(',').filter(|s| !s.trim().is_empty()) {
                    let (k, v) = kv.split_once('=').expect("name=value");
                    vehicle.set_var(k.trim(), v.trim().parse().expect("a number"));
                }
            };
            for k in 0..30 {
                if k == 10 {
                    set_all(&mut vehicle);
                }
                vehicle.update(1.0 / 30.0);
            }
            set_all(&mut vehicle);
            vehicle.mesh_props = omsi_sim::vehicle::compute_mesh_props(&vehicle.ty, &|n| vehicle.var(n));
            vehicle.update_text_textures();
            let looked = std::time::Instant::now();
            let screens = crate::companion::screens::discover(&vehicle, &root);
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            eprintln!("{stem}: {} screen(s) in {:?}", screens.len(), looked.elapsed());
            let mut pictures: std::collections::HashMap<PathBuf, Option<(Vec<u8>, u32, u32, [f32; 4])>> = Default::default();
            let mut state_screens = Vec::new();
            for s in &screens {
                let Some(f) = s.form.as_ref() else {
                    eprintln!("  {} ({}): no form", s.name, s.id);
                    continue;
                };
                let shown = |k: usize| vehicle.mesh_props.get(f.meshes[k]).is_none_or(|p| p.visible);
                let moved = f.moved(&now_of(&vehicle, f));
                for u in &f.files {
                    pictures.entry(u.path.clone()).or_insert_with(|| crate::companion::server::texture_part(&u.path, u.uv).map(|(_, _, px, w, h, uv)| (px, w, h, uv)));
                }
                // the part of a file each part uses: the same file over all forms here
                let texts: std::collections::HashMap<usize, (u32, u32, Vec<u8>)> = f.texts.iter().filter_map(|&n| {
                    let t = vehicle.text_textures.get(n)?;
                    Some((n, (t.def.width.max(1) as u32, t.def.height.max(1) as u32, t.image(&text_now(&vehicle, n)))))
                }).collect();
                let pic = |p: &Part| {
                    match p.tex {
                        PartTex::Colour => None,
                        PartTex::File(k) => {
                            let (px, w, h, uv) = pictures.get(&f.files[k].path)?.as_ref()?;
                            Some(raster::Pic { w: *w, h: *h, rgba: px, uv: *uv, nearest: false })
                        }
                        PartTex::Text(n) => {
                            let (w, h, px) = texts.get(&n)?;
                            Some(raster::Pic { w: *w, h: *h, rgba: px, uv: [0.0, 0.0, 1.0, 1.0], nearest: true })
                        }
                        PartTex::Script(n) => {
                            let st = vehicle.host.script_textures.get(n)?;
                            (st.rgba.len() == (st.width * st.height * 4) as usize).then(|| raster::Pic { w: st.width, h: st.height, rgba: &st.rgba, uv: [0.0, 0.0, 1.0, 1.0], nearest: true })
                        }
                    }
                };
                let scale = 1600.0 / f.sheet.w.max(f.sheet.h);
                let (w, h, px) = raster::draw(f, scale, &shown, &moved, &pic);
                let lum: Vec<f32> = px.chunks_exact(4).map(|p| p[0] as f32 * 0.3 + p[1] as f32 * 0.59 + p[2] as f32 * 0.11).collect();
                let mean = lum.iter().sum::<f32>() / lum.len() as f32;
                let spread = (lum.iter().map(|l| (l - mean).powi(2)).sum::<f32>() / lum.len() as f32).sqrt();
                let tris: usize = f.parts.iter().map(|p| p.idx.len() / 3).sum();
                eprintln!("  {} ({}): {:.3} x {:.3} m, {} parts ({} triangles), {} touch areas, {} files, {} texts, {} scripts, brightness {mean:.0} ± {spread:.0}", s.name, s.id, f.sheet.w, f.sheet.h, f.parts.len(), tris, f.touches.len(), f.files.len(), f.texts.len(), f.scripts.len());
                if std::env::var_os("OMSI_FORM_VERBOSE").is_some() {
                    for (k, p) in f.parts.iter().enumerate() {
                        let m = f.meshes[p.mesh];
                        let file = &vehicle.ty.model.meshes[vehicle.ty.meshes[m].def_index].file;
                        let (lo, hi) = p.verts.iter().fold((f32::MAX, f32::MIN), |a, v| (a.0.min(v[2]), a.1.max(v[2])));
                        let (x0, x1) = p.verts.iter().fold((f32::MAX, f32::MIN), |a, v| (a.0.min(v[0]), a.1.max(v[0])));
                        let (y0, y1) = p.verts.iter().fold((f32::MAX, f32::MIN), |a, v| (a.0.min(v[1]), a.1.max(v[1])));
                        let tex = match p.tex { PartTex::File(i) => f.files[i].path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), t => format!("{t:?}") };
                        eprintln!("    part {k}: mesh {m} {file} slot {} {tex} alpha {} shown {} tris {} x {x0:.3}..{x1:.3} y {y0:.3}..{y1:.3} z {lo:.3}..{hi:.3} colour {:?}", p.slot, p.alpha, shown(p.mesh), p.idx.len() / 3, p.colour);
                    }
                }
                if let Some(dir) = shots.as_ref() {
                    let _ = std::fs::create_dir_all(dir);
                    let mut img = image::RgbaImage::from_raw(w, h, px.clone()).unwrap();
                    // (the touch areas shown, outlined, in a copy)
                    image::save_buffer(dir.join(format!("{stem}_{}.png", s.id)), &img, w, h, image::ColorType::Rgba8).expect("the picture written");
                    for t in f.touches.iter().filter(|t| shown(t.mesh)) {
                        for (k, tri) in t.tris.iter().flat_map(|tri| (0..3).map(move |k| (k, tri))) {
                            let (a, b) = (tri[k].truncate() * scale, tri[(k + 1) % 3].truncate() * scale);
                            for j in 0..=100 {
                                let q = a + (b - a) * (j as f32 / 100.0);
                                if q.x >= 0.0 && q.y >= 0.0 && (q.x as u32) < w && (q.y as u32) < h {
                                    img.put_pixel(q.x as u32, q.y as u32, image::Rgba([255, 0, 255, 255]));
                                }
                            }
                        }
                    }
                    image::save_buffer(dir.join(format!("{stem}_{}_touch.png", s.id)), &img, w, h, image::ColorType::Rgba8).expect("the picture written");
                }
                drawn += 1;
                // the middle of each touch area shown: that area on the page, the same switch in
                // the cab
                let rot = vehicle.body_rotation();
                let event = |i: usize| vehicle.ty.meshes.get(i).and_then(|m| vehicle.ty.model.meshes.get(m.def_index)).and_then(|d| d.mouse_event.clone());
                for t in f.touches.iter().filter(|t| shown(t.mesh)) {
                    // (the middle of its nearest triangles: its face)
                    let near = t.tris.iter().filter(|tri| tri.iter().map(|p| p.z).fold(f32::MIN, f32::max) >= t.z - 1e-4).max_by(|a, b| (a[1].truncate() - a[0].truncate()).perp_dot(a[2].truncate() - a[0].truncate()).abs().total_cmp(&(b[1].truncate() - b[0].truncate()).perp_dot(b[2].truncate() - b[0].truncate()).abs()));
                    let Some(near) = near else { continue };
                    let c = (near[0].truncate() + near[1].truncate() + near[2].truncate()) / 3.0;
                    if c.x < 0.0 || c.y < 0.0 || c.x > f.sheet.w || c.y > f.sheet.h {
                        continue;
                    }
                    touches += 1;
                    let mine = f.meshes[t.mesh];
                    let on_page = f.touch_at(c, &shown, &moved).map(|o| f.meshes[o.mesh]);
                    let at = f.sheet.top_left + f.sheet.ax * c.x + f.sheet.ay * c.y + f.sheet.normal * (t.z + 0.02);
                    let got = crate::player::pick_in(&vehicle, vehicle.position + rot.transform_point3(at).as_dvec3(), rot.transform_vector3(-f.sheet.normal), 0.0);
                    let same = |g: Option<usize>| g == Some(mine) || g.is_some_and(|g| event(g) == event(mine));
                    if same(on_page) && same(got) {
                        hit += 1;
                    } else {
                        eprintln!("    {} at ({:.3}, {:.3}): the page has {:?}, the cab {:?}", t.event, c.x, c.y, on_page.and_then(event), got.and_then(event));
                    }
                }
                if let Some(dir) = dump.as_ref() {
                    let dir = dir.join(&stem);
                    let _ = std::fs::create_dir_all(&dir);
                    let base = state_screens.len() * 100;
                    let ids: Vec<usize> = (0..f.files.len()).map(|k| base + k).collect();
                    for (k, u) in f.files.iter().enumerate() {
                        if let Some((bytes, ctype, uv)) = crate::companion::server::encode_part(&u.path, u.uv) {
                            std::fs::write(dir.join(format!("tex_{}.{}", base + k, if ctype == "image/png" { "png" } else { "jpg" })), bytes.as_slice()).unwrap();
                            std::fs::write(dir.join(format!("tex_{}.uv", base + k)), format!("{},{},{},{}", uv[0], uv[1], uv[2], uv[3])).unwrap();
                        }
                    }
                    let mut texts_json = Vec::new();
                    for &n in &f.texts {
                        let t = &vehicle.text_textures[n];
                        let font = t.atlas.as_ref().map(|a| {
                            let id = base + n;
                            std::fs::write(dir.join(format!("font_{id}.json")), font_json(a).to_string()).unwrap();
                            let (png, _) = crate::companion::server::encode(&font_rgba(a), a.width, a.height, [0, 0, a.width, a.height], true).unwrap();
                            std::fs::write(dir.join(format!("font_{id}.png")), png).unwrap();
                            id
                        });
                        texts_json.push(text_json(t, n, font));
                    }
                    for &n in &f.scripts {
                        if let Some(st) = vehicle.host.script_textures.get(n).filter(|st| st.rgba.len() == (st.width * st.height * 4) as usize) {
                            let (png, _) = crate::companion::server::encode(&st.rgba, st.width, st.height, [0, 0, st.width, st.height], true).unwrap();
                            std::fs::write(dir.join(format!("x{n}.png")), png).unwrap();
                        }
                    }
                    std::fs::write(dir.join(format!("form_{}.json", s.id)), f.json(&ids, &texts_json, &vehicle.html_textures.iter().map(|h| h.script_index).collect::<Vec<_>>()).to_string()).unwrap();
                    std::fs::write(dir.join(format!("live_{}.json", s.id)), live(&vehicle, f).to_string()).unwrap();
                    state_screens.push(json!({ "id": s.id, "name": s.name, "html": s.html, "panel": s.source == crate::companion::screens::Source::Panel, "fields": s.fields.len(), "size": s.size, "keys": s.keys.iter().map(|k| json!({ "label": k.label, "title": k.event, "r": k.rect })).collect::<Vec<_>>(), "view": s.view.map(|v| json!({ "w": v.size.0, "h": v.size.1, "keys": [] })), "form": format!("{:016x}", base) }));
                }
            }
            if let Some(dir) = dump.as_ref() {
                std::fs::write(dir.join(&stem).join("screens.json"), Value::Array(state_screens).to_string()).unwrap();
            }
        }
        eprintln!("{drawn} device(s) drawn; {hit} of {touches} touch areas lie on their switches");
        if drawn > 0 {
            assert!(hit * 10 >= touches * 8, "only {hit} of {touches} touch areas lie on their switches");
        }
    }
}
