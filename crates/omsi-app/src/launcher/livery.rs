//! The Livery studio: the chosen bus in the showroom, painted with layers - a base colour,
//! bands and stripes, a name and a logo on its sides - and saved as a paint scheme of its
//! own the game lists with the bus's other liveries.
//!
//! The paint goes onto the bus's body texture (the `[CTCTexture]` its repaints replace). To
//! know where on the bus a texel of it lies, the texture is laid out once per bus: every
//! triangle drawn with it is filled in in texture space with its place on the bus and which
//! way it faces (`Surface`). A layer is then a rule over those places - "lower than a third
//! of the bus's height", "on the sides, 4 m from the front" - and the texture is made again
//! from them on a worker whenever a layer changes, and put on the bus in the showroom.
//!
//! Nothing is written into the OMSI 2 folder: a livery is saved into the content folder's
//! copy of the bus's `[CTC]` folder (a `.cti` and its picture), where the game reads repaints
//! installed as mods.

use super::theme::*;
use super::ui::{id_of, ButtonKind, Ui};
use super::{showroom, Launcher, Page};
use glam::{Vec2, Vec3};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

/// What a layer paints.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    /// The whole body.
    Base,
    /// Everything under a line.
    TwoTone,
    Skirt,
    Window,
    Roof,
    Slanted,
    Wave,
    Front,
    Rear,
    /// Words on the sides.
    Text,
    /// A picture on the sides.
    Logo,
}

/// The stripes a click puts on (the presets of the right panel): kind, name.
const STRIPES: [(Kind, &str); 8] = [
    (Kind::Skirt, "Skirt band"),
    (Kind::Window, "Window band"),
    (Kind::Roof, "Roof band"),
    (Kind::Slanted, "Slanted"),
    (Kind::Wave, "Wave"),
    (Kind::TwoTone, "Two-tone"),
    (Kind::Front, "Front face"),
    (Kind::Rear, "Rear face"),
];

/// Colours to pick from at once.
const SWATCHES: [[u8; 3]; 16] = [
    [255, 255, 255], [206, 210, 214], [128, 132, 138], [24, 24, 26],
    [236, 0, 22], [176, 0, 32], [247, 168, 0], [255, 214, 0],
    [214, 40, 130], [120, 40, 140], [0, 84, 159], [41, 98, 240],
    [0, 150, 130], [0, 132, 61], [120, 190, 32], [110, 70, 40],
];

#[derive(Clone, PartialEq, Debug)]
pub struct Layer {
    pub kind: Kind,
    pub name: String,
    pub color: [u8; 3],
    pub visible: bool,
    pub locked: bool,
    pub opacity: f32,
    /// Where it is, as a share of the bus's height from the bottom (bands, text, logo).
    pub height: f32,
    /// How thick, as a share of the bus's height (bands); the text's and logo's height in
    /// metres.
    pub size: f32,
    /// The slope or the wave's height (as a share of the bus's height).
    pub amount: f32,
    /// Where along the bus, as a share of its length from the rear (text, logo).
    pub along: f32,
    pub text: String,
    /// The logo's picture (its number in `LiveryView::images`).
    pub image: u64,
}

impl Layer {
    fn new(kind: Kind, color: [u8; 3]) -> Layer {
        let name = match kind {
            Kind::Base => "Base colour",
            Kind::Text => "Name",
            Kind::Logo => "Logo",
            k => STRIPES.iter().find(|s| s.0 == k).map(|s| s.1).unwrap_or("Stripe"),
        };
        let (height, size, amount, along) = match kind {
            Kind::Base => (0.0, 1.0, 0.0, 0.5),
            Kind::TwoTone => (0.42, 1.0, 0.0, 0.5),
            Kind::Skirt => (0.13, 0.10, 0.0, 0.5),
            Kind::Window => (0.66, 0.30, 0.0, 0.5),
            Kind::Roof => (0.90, 0.2, 0.0, 0.5),
            Kind::Slanted => (0.30, 1.0, 0.35, 0.5),
            Kind::Wave => (0.30, 0.07, 0.07, 0.5),
            Kind::Front | Kind::Rear => (0.0, 1.0, 0.0, 0.5),
            Kind::Text => (0.30, 0.32, 0.0, 0.55),
            Kind::Logo => (0.30, 0.6, 0.0, 0.3),
        };
        Layer { kind, name: name.into(), color, visible: true, locked: false, opacity: 1.0, height, size, amount, along, text: String::new(), image: 0 }
    }
}

/// The body texture laid out over the bus (see the module's words).
pub struct Surface {
    /// The scene's texture it is drawn with, its `[CTCTexture]` name (None: the bus has no
    /// repaints for it - it can be painted and looked at, not saved) and its file.
    pub tex: omsi_render::TextureId,
    /// Other textures of the scene with the same picture (a rear section's): painted alike.
    pub also: Vec<omsi_render::TextureId>,
    pub ctc: Option<String>,
    pub file: String,
    /// The bus's `[CTC]` folder (where a `.cti` goes).
    pub ctc_dir: Option<PathBuf>,
    pub base: image::RgbaImage,
    /// Per texel: where on the bus it is and which way it faces (zero: no triangle there).
    pub pos: Vec<Vec3>,
    pub nrm: Vec<Vec3>,
    pub lo: Vec3,
    pub hi: Vec3,
    /// The body's mean brightness (what a painted texel is shaded against).
    pub mean: f32,
    /// What the studio says about it: the size and how many texels a metre.
    pub about: String,
}

/// What the studio waits for: the surface being laid out, a texture being painted.
#[derive(Default)]
pub struct LiveryView {
    pub layers: Vec<Layer>,
    pub selected: Option<usize>,
    /// What the colour picker colours: 0 the body, 1 the stripes, 2 the name, 3 the selected
    /// layer.
    pub target: usize,
    hsv: [f32; 3],
    hex: String,
    used: Vec<[u8; 3]>,
    pub name: String,
    pub on_bus: String,
    pub mirror: bool,
    pub before: bool,
    undo: Vec<Vec<Layer>>,
    redo: Vec<Vec<Layer>>,
    committed: Vec<Layer>,
    images: std::collections::HashMap<u64, Arc<image::RgbaImage>>,
    next_image: u64,
    /// The front part's surface (the one the panels speak of) and the rear sections'.
    surface: Option<Arc<Surface>>,
    rear: Vec<Arc<Surface>>,
    /// The bus the surfaces are of (bus, paint, map), and a layout under way for it.
    surface_for: Option<(String, String, String)>,
    laying: Option<Receiver<Result<Vec<Surface>, String>>>,
    /// The layers last painted and the texture under way.
    painted: Option<Vec<Layer>>,
    painting: Option<Receiver<Vec<image::RgbaImage>>>,
    /// The texture on the bus is the studio's (to be put back when the page is left).
    applied: bool,
    pub status: String,
    started: bool,
}

impl LiveryView {
    /// The device went with the showroom's scene: everything is laid out again.
    pub fn drop_gpu(&mut self) {
        self.surface = None;
        self.rear.clear();
        self.surface_for = None;
        self.laying = None;
        self.painted = None;
        self.painting = None;
        self.applied = false;
    }
}

/// The texture laid out over the bus: the body texture found (the repaintable one the most
/// of the bus's outside is drawn with), its picture read, and its triangles filled in.
fn lay_out(t: &showroom::PaintPart, scheme: Option<usize>, root: &std::path::Path) -> Result<Surface, String> {
    let vt = &t.vt;
    let scheme = scheme.filter(|i| *i < vt.paint_schemes.len());
    let ctc_defaults: Vec<(String, String)> = vt.model.ctc_textures.clone();
    // the area each texture covers, by the scene's texture
    let mut area: std::collections::HashMap<omsi_render::TextureId, (f32, String)> = Default::default();
    for &(mi, slot, tex) in &t.slots {
        let m = &vt.meshes[mi];
        let Some(mat) = m.materials.get(slot) else { continue };
        let name = mat.texture.trim().to_string();
        if name.is_empty() {
            continue;
        }
        let xf = t.xf.get(mi).copied().unwrap_or(glam::Mat4::IDENTITY);
        let d = &m.data;
        let mut a = 0.0;
        for &(start, count, s) in &d.ranges {
            if s as usize != slot {
                continue;
            }
            for tri in d.indices[start as usize..(start + count) as usize].chunks_exact(3) {
                let p = |i: u32| xf.transform_point3(d.positions[i as usize]);
                let (p0, p1, p2) = (p(tri[0]), p(tri[1]), p(tri[2]));
                a += (p1 - p0).cross(p2 - p0).length() * 0.5;
            }
        }
        let e = area.entry(tex).or_insert((0.0, name));
        e.0 += a;
    }
    let is_ctc = |name: &str| ctc_defaults.iter().find(|(_, d)| d.trim().eq_ignore_ascii_case(name.trim())).map(|(n, _)| n.clone());
    let best = area
        .iter()
        .map(|(tex, (a, name))| (*tex, *a * if is_ctc(name).is_some() { 4.0 } else { 1.0 }, name.clone()))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .ok_or("This bus has no textured body")?;
    let (tex, _, file) = best;
    let ctc = is_ctc(&file);
    // the picture: the worn scheme's, else the model's own
    let mut dirs = vt.texture_dirs(root);
    let mut name = file.clone();
    if let (Some(s), Some(_)) = (scheme, ctc.as_ref()) {
        let (subs, dir) = vt.scheme_substitutions(s);
        if let Some(f) = subs.get(&file.to_ascii_lowercase()) {
            name = f.clone();
            if let Some(d) = dir {
                dirs.insert(0, d);
            }
        }
    }
    let refs: Vec<&std::path::Path> = dirs.iter().map(|p| p.as_path()).collect();
    let path = omsi_texture::find_texture(&name, &refs).ok_or_else(|| format!("{name} not found"))?;
    let img = omsi_texture::decode_file(&path).map_err(|e| e.to_string())?;
    // (no larger than 2048: a livery has no use for more, and every change paints them all)
    let k = ((img.width.max(img.height) as f32 / 2048.0).ceil() as u32).max(1);
    let (w, h) = (img.width / k, img.height / k);
    let mut base = image::RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let i = (((y * k) * img.width + x * k) * 4) as usize;
            base.put_pixel(x, y, image::Rgba([img.rgba[i], img.rgba[i + 1], img.rgba[i + 2], img.rgba[i + 3]]));
        }
    }
    // the triangles filled in, in texture space
    let n = (w * h) as usize;
    let mut pos = vec![Vec3::ZERO; n];
    let mut nrm = vec![Vec3::ZERO; n];
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    let mut world_area = 0.0f32;
    let mut texels = 0usize;
    for &(mi, slot, t2) in &t.slots {
        if t2 != tex {
            continue;
        }
        let m = &vt.meshes[mi];
        let xf = t.xf.get(mi).copied().unwrap_or(glam::Mat4::IDENTITY);
        let d = &m.data;
        if d.uvs.len() != d.positions.len() {
            continue;
        }
        for &(start, count, s) in &d.ranges {
            if s as usize != slot {
                continue;
            }
            for tri in d.indices[start as usize..(start + count) as usize].chunks_exact(3) {
                let p: [Vec3; 3] = [0, 1, 2].map(|j| xf.transform_point3(d.positions[tri[j] as usize]));
                let uv: [Vec2; 3] = [0, 1, 2].map(|j| d.uvs[tri[j] as usize]);
                let normal = (p[1] - p[0]).cross(p[2] - p[0]);
                let a = normal.length() * 0.5;
                if a < 1e-7 {
                    continue;
                }
                let normal = normal / (2.0 * a);
                for q in p {
                    lo = lo.min(q);
                    hi = hi.max(q);
                }
                // (a triangle laid over the texture's edge: put whole into the tile it starts in)
                let shift = Vec2::new(uv[0].x.floor(), uv[0].y.floor());
                let t: [Vec2; 3] = uv.map(|u| (u - shift) * Vec2::new(w as f32, h as f32));
                let (x0, x1) = (t.iter().map(|v| v.x).fold(f32::MAX, f32::min).floor().max(0.0) as i32, t.iter().map(|v| v.x).fold(f32::MIN, f32::max).ceil().min(w as f32) as i32);
                let (y0, y1) = (t.iter().map(|v| v.y).fold(f32::MAX, f32::min).floor().max(0.0) as i32, t.iter().map(|v| v.y).fold(f32::MIN, f32::max).ceil().min(h as f32) as i32);
                let den = (t[1].y - t[2].y) * (t[0].x - t[2].x) + (t[2].x - t[1].x) * (t[0].y - t[2].y);
                if den.abs() < 1e-9 {
                    continue;
                }
                world_area += a;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let c = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                        let l0 = ((t[1].y - t[2].y) * (c.x - t[2].x) + (t[2].x - t[1].x) * (c.y - t[2].y)) / den;
                        let l1 = ((t[2].y - t[0].y) * (c.x - t[2].x) + (t[0].x - t[2].x) * (c.y - t[2].y)) / den;
                        let l2 = 1.0 - l0 - l1;
                        if l0 < -0.01 || l1 < -0.01 || l2 < -0.01 {
                            continue;
                        }
                        let i = (y as u32 * w + x as u32) as usize;
                        if nrm[i] == Vec3::ZERO {
                            texels += 1;
                        }
                        pos[i] = p[0] * l0 + p[1] * l1 + p[2] * l2;
                        nrm[i] = normal;
                    }
                }
            }
        }
    }
    if texels == 0 {
        return Err("The body texture's triangles could not be laid out".into());
    }
    // the islands grown by two texels: the texture's filtering reads a little past their edges
    for _ in 0..2 {
        let (p0, n0) = (pos.clone(), nrm.clone());
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let i = (y as u32 * w + x as u32) as usize;
                if n0[i] != Vec3::ZERO {
                    continue;
                }
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let j = (ny as u32 * w + nx as u32) as usize;
                    if n0[j] != Vec3::ZERO {
                        pos[i] = p0[j];
                        nrm[i] = n0[j];
                        break;
                    }
                }
            }
        }
    }
    let mut sum = 0.0f32;
    let mut cnt = 0.0f32;
    for (i, px) in base.pixels().enumerate() {
        if nrm[i] != Vec3::ZERO {
            sum += lum(px.0);
            cnt += 1.0;
        }
    }
    let per_m = (texels as f32 / world_area.max(0.01)).sqrt();
    let about = format!("{} · {w} × {h} · {:.0} {}", omsi_ui::tr(if scheme.is_some() { "Begun from the chosen livery" } else { "Begun from the model's own" }), per_m, omsi_ui::tr("texels/m"));
    let ctc_dir = vt.model.ctc.first().map(|c| omsi_cfg::resolve_path(vt.def.dir(), &c.path));
    let mut also: Vec<omsi_render::TextureId> = t.slots.iter().filter(|(mi, slot, id)| *id != tex && vt.meshes[*mi].materials.get(*slot).is_some_and(|m| m.texture.trim().eq_ignore_ascii_case(file.trim()))).map(|s| s.2).collect();
    also.sort_unstable();
    also.dedup();
    Ok(Surface { tex, also, ctc, file, ctc_dir, base, pos, nrm, lo, hi, mean: (sum / cnt.max(1.0)).max(0.05), about })
}

fn lum(c: [u8; 4]) -> f32 {
    (0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32) / 255.0
}

/// A word or a picture prepared for painting: its coverage (0..255 a texel) or colours.
pub struct Stamp {
    pub w: u32,
    pub h: u32,
    /// RGBA, straight alpha.
    pub rgba: Vec<u8>,
}

/// How much of a layer lies on a texel at `p` facing `n` (0..1), and the colour there when
/// the layer brings its own (a logo).
fn cover(l: &Layer, s: &Surface, p: Vec3, n: Vec3, stamp: Option<&Stamp>, mirror: bool) -> (f32, Option<[u8; 3]>) {
    let size = (s.hi - s.lo).max(Vec3::splat(0.01));
    let zr = (p.z - s.lo.z) / size.z;
    let yr = (p.y - s.lo.y) / size.y;
    let side = n.x.abs() > 0.45;
    // (a soft edge of a few centimetres, not a stair of texels)
    let edge = |d: f32| (0.5 + d / (0.02 / size.z)).clamp(0.0, 1.0);
    let band = |c: f32, half: f32| edge(half - (zr - c).abs());
    match l.kind {
        Kind::Base => (1.0, None),
        Kind::TwoTone => (edge(l.height - zr), None),
        Kind::Skirt | Kind::Window => (band(l.height, l.size * 0.5), None),
        Kind::Roof => (if n.z > 0.75 { 1.0 } else { edge(zr - l.height) }, None),
        Kind::Slanted => (edge(l.height + l.amount * (yr - 0.5) - zr), None),
        Kind::Wave => {
            let c = l.height + l.amount * (yr * std::f32::consts::TAU * 2.0).sin();
            (band(c, l.size * 0.5), None)
        }
        Kind::Front => (if n.y > 0.45 && yr > 0.8 { 1.0 } else { 0.0 }, None),
        Kind::Rear => (if n.y < -0.45 && yr < 0.2 { 1.0 } else { 0.0 }, None),
        Kind::Text | Kind::Logo => {
            let Some(st) = stamp else { return (0.0, None) };
            if !side || (!mirror && n.x < 0.0) {
                return (0.0, None);
            }
            let hm = l.size.max(0.05);
            let wm = hm * st.w as f32 / st.h.max(1) as f32;
            let cy = s.lo.y + l.along * size.y;
            let cz = s.lo.z + l.height * size.z;
            // read left to right on both sides: seen from the side at +x the front is to the left
            let u = if n.x > 0.0 { (cy - p.y) / wm + 0.5 } else { (p.y - cy) / wm + 0.5 };
            let v = (cz - p.z) / hm + 0.5;
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                return (0.0, None);
            }
            let (x, y) = ((u * st.w as f32) as u32, (v * st.h as f32) as u32);
            let i = ((y.min(st.h - 1) * st.w + x.min(st.w - 1)) * 4) as usize;
            let a = st.rgba[i + 3] as f32 / 255.0;
            if l.kind == Kind::Logo {
                (a, Some([st.rgba[i], st.rgba[i + 1], st.rgba[i + 2]]))
            } else {
                (a, None)
            }
        }
    }
}

/// The body texture with the layers on it, bottom layer first.
pub fn compose(s: &Surface, layers: &[Layer], stamps: &[Option<Stamp>], mirror: bool) -> image::RgbaImage {
    let mut out = s.base.clone();
    for (i, px) in out.pixels_mut().enumerate() {
        let n = s.nrm[i];
        if n == Vec3::ZERO {
            continue;
        }
        let p = s.pos[i];
        let b = px.0;
        let l0 = lum(b);
        // the paint takes on the texture's own light and shade (panel lines, rivets, dirt),
        // and leaves its black (rubber, grilles) black
        let shade = (l0 / s.mean).clamp(0.55, 1.18);
        let keep = if l0 < 0.07 { 0.0 } else { 1.0 };
        let mut c = [b[0] as f32, b[1] as f32, b[2] as f32];
        for (k, l) in layers.iter().enumerate() {
            if !l.visible {
                continue;
            }
            let (a, own) = cover(l, s, p, n, stamps.get(k).and_then(|s| s.as_ref()), mirror);
            let painted = matches!(l.kind, Kind::Text | Kind::Logo);
            let a = a * l.opacity * if painted { 1.0 } else { keep };
            if a <= 0.0 {
                continue;
            }
            let col = own.unwrap_or(l.color);
            let sh = if painted { 1.0 } else { shade };
            for j in 0..3 {
                let target = (col[j] as f32 * sh).min(255.0);
                c[j] += (target - c[j]) * a.min(1.0);
            }
        }
        px.0 = [c[0] as u8, c[1] as u8, c[2] as u8, b[3]];
    }
    out
}

/// Per frame, with the renderer: the bus laid out when it changed, the paint made again when
/// a layer did, and put on the bus (and taken off again when the page is left).
pub fn pump(l: &mut Launcher, renderer: &mut omsi_render::Renderer, _dt: f32) {
    let here = l.page == Page::Livery;
    let v = &mut l.livery;
    if !here || v.before {
        // back to the bus's own paint
        if v.applied {
            for s in v.surface.iter().chain(&v.rear) {
                show(&mut l.showroom, renderer, s, &s.base);
            }
            v.applied = false;
            v.painted = None;
        }
        if !here {
            return;
        }
    }
    let Some(target) = l.showroom.paint_target() else { return };
    let key = (target.look.bus.clone(), target.look.paint.clone(), target.look.map.clone());
    if v.surface_for.as_ref() != Some(&key) {
        v.surface_for = Some(key);
        v.surface = None;
        v.rear.clear();
        v.painted = None;
        v.applied = false;
        v.status = omsi_ui::tr("Laying the body texture out…").to_string();
        let (tx, rx) = channel();
        let root = PathBuf::from(&l.state.config.root);
        std::thread::spawn(move || {
            // the front part must be laid out; a rear section that cannot is left as it is
            let mut out: Vec<Surface> = Vec::new();
            for (k, p) in target.parts.iter().enumerate() {
                match lay_out(p, target.scheme, &root) {
                    // (a rear section drawn with the front's own texture is painted with it)
                    Ok(s) if out.iter().any(|o| o.tex == s.tex) => {}
                    Ok(s) => out.push(s),
                    Err(e) if k == 0 => {
                        let _ = tx.send(Err(e));
                        return;
                    }
                    Err(e) => log::info!("livery studio: rear section {k} not painted: {e}"),
                }
            }
            let _ = tx.send(Ok(out));
        });
        v.laying = Some(rx);
        return;
    }
    if let Some(rx) = v.laying.as_ref() {
        match rx.try_recv() {
            Ok(Ok(parts)) => {
                for s in &parts {
                    log::info!("livery studio: {} laid out ({})", s.file, s.about);
                }
                v.status = String::new();
                let mut parts = parts.into_iter().map(Arc::new);
                v.surface = parts.next();
                v.rear = parts.collect();
                v.laying = None;
            }
            Ok(Err(e)) => {
                log::warn!("livery studio: {e}");
                v.status = e;
                v.laying = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(_) => v.laying = None,
        }
    }
    let Some(lead) = v.surface.clone() else { return };
    let all: Vec<Arc<Surface>> = std::iter::once(lead).chain(v.rear.iter().cloned()).collect();
    if let Some(rx) = v.painting.as_ref() {
        if let Ok(imgs) = rx.try_recv() {
            v.painting = None;
            if !v.before {
                for (s, img) in all.iter().zip(&imgs) {
                    show(&mut l.showroom, renderer, s, img);
                }
                l.livery.applied = true;
            }
        }
        return;
    }
    if v.before || (v.painted.as_ref() == Some(&v.layers) && v.applied) {
        return;
    }
    // the words and pictures the layers stamp, made here (the fonts are the interface's)
    let stamps = stamps(v, &l.ui);
    v.painted = Some(v.layers.clone());
    let (layers, mirror) = (v.layers.clone(), v.mirror);
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(all.iter().enumerate().map(|(k, s)| compose(s, &part_layers(&layers, k == 0), &stamps, mirror)).collect());
    });
    v.painting = Some(rx);
}

/// The layers a part wears: the names and logos go on the front part only (a rear section
/// is measured on its own, and the words would stand on it a second time).
fn part_layers(layers: &[Layer], lead: bool) -> Vec<Layer> {
    layers.iter().map(|l| Layer { visible: l.visible && (lead || !matches!(l.kind, Kind::Text | Kind::Logo)), ..l.clone() }).collect()
}

/// The part drawn with this body texture.
fn show(room: &mut showroom::Showroom, renderer: &omsi_render::Renderer, s: &Surface, img: &image::RgbaImage) {
    for id in std::iter::once(s.tex).chain(s.also.iter().copied()) {
        room.set_texture(renderer, id, img);
    }
}

fn rgb_to_hsv(c: [u8; 3]) -> [f32; 3] {
    let (r, g, b) = (c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    [h, if max == 0.0 { 0.0 } else { d / max }, max]
}

fn hsv_to_rgb(h: [f32; 3]) -> [u8; 3] {
    let (h, s, v) = (h[0].rem_euclid(1.0) * 6.0, h[1], h[2]);
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [((r + m) * 255.0).round() as u8, ((g + m) * 255.0).round() as u8, ((b + m) * 255.0).round() as u8]
}

fn hex_of(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn col(c: [u8; 3]) -> Color {
    Color::rgba(c[0], c[1], c[2], 1.0)
}

/// The colour picker: saturation and value in a square, the hue in a bar under it. True
/// while it changes.
fn picker(ui: &mut Ui, r: Rect, hsv: &mut [f32; 3]) -> bool {
    let sq = Rect::new(r.x, r.y, r.w, r.h - 26.0);
    let hue = hsv_to_rgb([hsv[0], 1.0, 1.0]);
    ui.solid(r);
    ui.p().gradient_h(sq, Color::rgba(255, 255, 255, 1.0), col(hue));
    ui.p().gradient(sq, Color::rgba(0, 0, 0, 0.0), Color::rgba(0, 0, 0, 1.0));
    ui.p().rounded_border(sq, 4.0, 1.0, EDGE());
    let mut changed = false;
    let (_, held, _) = ui.interact(id_of("livery-sv"), sq);
    if held && ui.input.down {
        let m = ui.input.mouse;
        hsv[1] = ((m.x - sq.x) / sq.w).clamp(0.0, 1.0);
        hsv[2] = 1.0 - ((m.y - sq.y) / sq.h).clamp(0.0, 1.0);
        changed = true;
    }
    let knob = Vec2::new(sq.x + hsv[1] * sq.w, sq.y + (1.0 - hsv[2]) * sq.h);
    ui.p().circle(knob, 7.0, Color::rgba(255, 255, 255, 1.0));
    ui.p().circle(knob, 5.0, col(hsv_to_rgb(*hsv)));
    let bar = Rect::new(r.x, sq.bottom() + 10.0, r.w, 12.0);
    let n = 6;
    for k in 0..n {
        let a = hsv_to_rgb([k as f32 / n as f32, 1.0, 1.0]);
        let b = hsv_to_rgb([(k + 1) as f32 / n as f32, 1.0, 1.0]);
        ui.p().gradient_h(Rect::new(bar.x + bar.w * k as f32 / n as f32, bar.y, bar.w / n as f32 + 0.5, bar.h), col(a), col(b));
    }
    let (_, held, _) = ui.interact(id_of("livery-hue"), Rect::new(bar.x, bar.y - 4.0, bar.w, bar.h + 8.0));
    if held && ui.input.down {
        hsv[0] = ((ui.input.mouse.x - bar.x) / bar.w).clamp(0.0, 0.999);
        changed = true;
    }
    let hk = Vec2::new(bar.x + hsv[0] * bar.w, bar.center().y);
    ui.p().circle(hk, 8.0, Color::rgba(255, 255, 255, 1.0));
    ui.p().circle(hk, 6.0, col(hue));
    changed
}

/// The colour the picker shows now (of the quick slot or the selected layer).
fn target_color(v: &LiveryView) -> Option<[u8; 3]> {
    match v.target {
        0 => v.layers.iter().find(|l| l.kind == Kind::Base).map(|l| l.color),
        1 => v.layers.iter().find(|l| STRIPES.iter().any(|s| s.0 == l.kind)).map(|l| l.color),
        2 => v.layers.iter().find(|l| l.kind == Kind::Text).map(|l| l.color),
        _ => v.selected.and_then(|i| v.layers.get(i)).map(|l| l.color),
    }
}

/// Set the picker's colour on what it colours (a quick slot without its layer gets one).
fn set_target_color(v: &mut LiveryView, c: [u8; 3]) {
    let find = |v: &LiveryView, f: &dyn Fn(&Layer) -> bool| v.layers.iter().position(|l| f(l));
    let i = match v.target {
        0 => find(v, &|l| l.kind == Kind::Base).or_else(|| {
            v.layers.insert(0, Layer::new(Kind::Base, c));
            v.selected = v.selected.map(|s| s + 1);
            Some(0)
        }),
        1 => find(v, &|l| STRIPES.iter().any(|s| s.0 == l.kind)).or_else(|| {
            v.layers.push(Layer::new(Kind::Window, c));
            Some(v.layers.len() - 1)
        }),
        2 => find(v, &|l| l.kind == Kind::Text),
        _ => v.selected,
    };
    if let Some(l) = i.and_then(|i| v.layers.get_mut(i)) {
        if !l.locked {
            l.color = c;
        }
    }
}

fn remember(v: &mut LiveryView, c: [u8; 3]) {
    v.used.retain(|u| *u != c);
    v.used.insert(0, c);
    v.used.truncate(8);
}

/// Write the livery into the content folder: a picture for each part's body texture and a
/// `.cti` naming them, in the content folder's copy of each part's `[CTC]` folder. Returns
/// where.
fn save(l: &mut Launcher) -> Result<String, String> {
    let v = &l.livery;
    let lead = v.surface.clone().ok_or("The bus is still being laid out")?;
    lead.ctc.as_ref().ok_or("This bus's body texture is not one its repaints replace: it cannot be saved as a livery")?;
    let name = v.name.trim().to_string();
    if name.is_empty() {
        return Err("Give the livery a name first".into());
    }
    let content = omsi_launcher_lib::content_dir().ok_or("no content folder")?;
    let root = PathBuf::from(&l.state.config.root);
    // (never into the OMSI 2 folder)
    if content.starts_with(&root) {
        return Err("The content folder lies inside the OMSI 2 folder: the livery is not written there".into());
    }
    let stem: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let stem = format!("openomsi_{stem}");
    let stamps = stamps(v, &l.ui);
    let roots = omsi_cfg::content_roots();
    // per folder: its `.cti` items (a part whose texture has no [CTCTexture] keeps its paint)
    let mut items: Vec<(PathBuf, String)> = Vec::new();
    let all: Vec<Arc<Surface>> = std::iter::once(lead).chain(v.rear.iter().cloned()).collect();
    for (k, s) in all.iter().enumerate() {
        let (Some(ctc), Some(ctc_dir)) = (s.ctc.clone(), s.ctc_dir.clone()) else { continue };
        // the [CTC] folder's place under the root it is in (the game's folder, or a mod's)
        let owner = roots.iter().filter(|r| ctc_dir.starts_with(r)).max_by_key(|r| r.components().count()).cloned().unwrap_or(root.clone());
        let rel = ctc_dir.strip_prefix(&owner).map_err(|_| "The bus's [CTC] folder is outside the game's folders")?;
        let dir = content.join(rel);
        if dir.starts_with(&root) {
            return Err("The content folder lies inside the OMSI 2 folder: the livery is not written there".into());
        }
        // (one picture per texture: a rear section sharing the front's [CTCTexture] wears it)
        if items.iter().any(|(d, i)| *d == dir && i.contains(&format!("\r\n{ctc}\r\n"))) {
            continue;
        }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let file = if k == 0 { format!("{stem}.png") } else { format!("{stem}_{k}.png") };
        compose(s, &part_layers(&v.layers, k == 0), &stamps, v.mirror).save(dir.join(&file)).map_err(|e| e.to_string())?;
        let item = format!("[item]\r\n{name}\r\n{ctc}\r\n{file}\r\n\r\n");
        match items.iter_mut().find(|(d, _)| *d == dir) {
            Some((_, text)) => text.push_str(&item),
            None => items.push((dir, item)),
        }
    }
    for (dir, text) in &items {
        std::fs::write(dir.join(format!("{stem}.cti")), text).map_err(|e| e.to_string())?;
    }
    Ok(items.first().map(|(d, _)| d.display().to_string()).unwrap_or_default())
}

/// What each layer stamps: a name's letters (the interface's own font), a logo's picture.
fn stamps(v: &LiveryView, ui: &Ui) -> Vec<Option<Stamp>> {
    v.layers
        .iter()
        .map(|layer| match layer.kind {
            Kind::Text if !layer.text.trim().is_empty() => {
                let b = ui.fonts.render(layer.text.trim(), 96.0, Weight::Black);
                let mut rgba = vec![0u8; (b.w * b.h * 4) as usize];
                for (k, a) in b.alpha.iter().enumerate() {
                    rgba[k * 4 + 3] = *a;
                }
                (b.w > 0 && b.h > 0).then_some(Stamp { w: b.w, h: b.h, rgba })
            }
            Kind::Logo => v.images.get(&layer.image).map(|img| Stamp { w: img.width(), h: img.height(), rgba: img.as_raw().clone() }),
            _ => None,
        })
        .collect()
}

/// The views the bar under the bus turns to: name, yaw, pitch.
const VIEWS: [(&str, f32, f32); 6] = [("Left", 270.0, 4.0), ("Right", 90.0, 4.0), ("Front", 180.0, 6.0), ("Rear", 0.0, 6.0), ("Roof", 235.0, 55.0), ("Angled", 215.0, 12.0)];

pub fn draw(l: &mut Launcher, area: Rect) {
    if !l.livery.started {
        l.livery.started = true;
        l.livery.mirror = true;
        l.livery.hsv = [0.0, 0.0, 1.0];
        l.livery.hex = "#ffffff".into();
        // (a livery is looked at from the side)
        l.showroom.turn_to(VIEWS[0].1, VIEWS[0].2);
    }
    let gap = 12.0;
    let bar = Rect::new(area.x, area.y, area.w, 52.0);
    let body_y = bar.bottom() + gap;
    let body_h = (area.h - 52.0 - gap).max(200.0);
    let left = Rect::new(area.x, body_y, 220.0, body_h);
    let right = Rect::new(area.right() - 300.0, body_y, 300.0, body_h);
    let view = Rect::new(left.right() + gap, body_y, right.x - left.right() - gap * 2.0, body_h);

    // the bus first: the panels lie beside it and take the mouse before it
    if l.state.bus().is_none() {
        l.ui.panel(view);
        l.ui.text_in("Choose a bus first", Rect::new(view.x, view.center().y - 30.0, view.w, 24.0), 16.0, Weight::Bold, TEXT(), Align::Center);
        if l.ui.button("livery-choose", Rect::new(view.center().x - 90.0, view.center().y + 4.0, 180.0, ROW), "Bus gallery", Some("photo_library"), ButtonKind::Primary) {
            l.go(Page::Buses);
        }
    } else {
        l.preview_full(view, 0.5);
        l.ui.p().rounded_border(view, RADIUS, 1.0, EDGE());
        if !l.livery.status.is_empty() || l.livery.laying.is_some() || l.livery.painting.is_some() {
            let t = if l.livery.status.is_empty() { omsi_ui::tr("Painting…").to_string() } else { l.livery.status.clone() };
            let w = l.ui.width(&t, 12.5, Weight::Medium) + 40.0;
            let chip = Rect::new(view.center().x - w * 0.5, view.y + 14.0, w, 28.0);
            l.ui.p().rounded(chip, 14.0, PANEL().alpha(0.92));
            l.ui.text_in(&t, chip, 12.5, Weight::Medium, TEXT_SOFT(), Align::Center);
        }
        // the views and before / after along the foot of the picture
        let names: Vec<String> = VIEWS.iter().map(|v| omsi_ui::tr(v.0).to_string()).collect();
        let ba = omsi_ui::tr("Before / after").to_string();
        let ws: Vec<f32> = names.iter().map(|n| l.ui.width(n, 12.0, Weight::Medium) + 22.0).collect();
        let ba_w = l.ui.width(&ba, 12.0, Weight::Medium) + 40.0;
        let total: f32 = ws.iter().sum::<f32>() + ba_w + 12.0;
        let strip = Rect::new(view.center().x - total * 0.5 - 6.0, view.bottom() - 46.0, total + 12.0, 34.0);
        l.ui.solid(strip);
        l.ui.p().rounded(strip, 17.0, PANEL().alpha(0.94));
        l.ui.p().rounded_border(strip, 17.0, 1.0, EDGE());
        let mut x = strip.x + 6.0;
        for (k, (name, w)) in names.iter().zip(ws).enumerate() {
            let r = Rect::new(x, strip.y + 3.0, w, 28.0);
            let (h, _, c) = l.ui.interact(id_of(&format!("livery-view-{k}")), r);
            if h {
                l.ui.p().rounded(r, 14.0, HOVER());
            }
            l.ui.text_in(name, r, 12.0, Weight::Medium, if h { TEXT() } else { TEXT_SOFT() }, Align::Center);
            if c {
                l.showroom.turn_to(VIEWS[k].1, VIEWS[k].2);
            }
            x += w;
        }
        let r = Rect::new(x + 6.0, strip.y + 3.0, ba_w, 28.0);
        let (h, _, c) = l.ui.interact(id_of("livery-before"), r);
        if c {
            l.livery.before = !l.livery.before;
        }
        let on = l.livery.before;
        l.ui.p().rounded(r, 14.0, if on { SELECTED() } else if h { HOVER() } else { FIELD() });
        l.ui.icon(if on { "visibility_off" } else { "visibility" }, Vec2::new(r.x + 14.0, r.center().y), 14.0, TEXT_SOFT());
        l.ui.text_in(&ba, Rect::new(r.x + 24.0, r.y, r.w - 26.0, r.h), 12.0, Weight::Medium, TEXT_SOFT(), Align::Center);
    }

    // (the panels slide in from their sides when the page opens)
    let a = appear(l.page_t, 0.0, 0.45);
    let b = appear(l.page_t, 0.08, 0.45);
    toolbar(l, Rect::new(bar.x, bar.y - 18.0 * (1.0 - a), bar.w, bar.h));
    layers_panel(l, Rect::new(left.x - 40.0 * (1.0 - b), left.y, left.w, left.h));
    right_panel(l, Rect::new(right.x + 40.0 * (1.0 - b), right.y, right.w, right.h));

    if l.state.bus().is_some() {
        l.showroom_pointer(view);
    }
    // what changed this frame goes on the undo list once the mouse lets go
    let v = &mut l.livery;
    if !l.ui.input.down && v.layers != v.committed {
        let before = std::mem::replace(&mut v.committed, v.layers.clone());
        v.undo.push(before);
        if v.undo.len() > 60 {
            v.undo.remove(0);
        }
        v.redo.clear();
    }
}

/// The bar along the top: back, the tools, mirror, undo and redo, the livery's name and Save.
fn toolbar(l: &mut Launcher, r: Rect) {
    l.ui.panel(r);
    if l.ui.button("livery-back", Rect::new(r.x + 8.0, r.y + 8.0, 90.0, 36.0), "Back", Some("chevron_left"), ButtonKind::Ghost) {
        l.go(Page::Buses);
    }
    let bus = l.state.bus().map(|b| omsi_launcher_lib::display_bus_name(&b.name)).unwrap_or_default();
    l.ui.text_in("Livery studio", Rect::new(r.x + 110.0, r.y + 7.0, 220.0, 20.0), 14.5, Weight::Bold, TEXT(), Align::Left);
    l.ui.text_in(&bus, Rect::new(r.x + 110.0, r.y + 27.0, 220.0, 16.0), 11.0, Weight::Regular, TEXT_DIM(), Align::Left);

    // the tools: each puts its layer on
    let tools: [(&str, &str, Kind); 4] = [("Fill", "format_color_fill", Kind::Base), ("Stripe", "texture", Kind::Window), ("Text", "text_fields", Kind::Text), ("Image", "image", Kind::Logo)];
    let mut x = r.x + 340.0;
    for (name, icon, kind) in tools {
        let b = Rect::new(x, r.y + 4.0, 56.0, 44.0);
        let (h, _, c) = l.ui.interact(id_of(&format!("livery-tool-{name}")), b);
        if h {
            l.ui.p().rounded(b, 8.0, HOVER());
        }
        l.ui.icon(icon, Vec2::new(b.center().x, b.y + 15.0), 18.0, if h { TEXT() } else { TEXT_SOFT() });
        l.ui.text_in(name, Rect::new(b.x, b.y + 26.0, b.w, 14.0), 10.5, Weight::Medium, TEXT_DIM(), Align::Center);
        if c {
            add_layer(l, kind);
        }
        x += 60.0;
    }
    // mirror: words and logos on both sides
    let m = Rect::new(x + 8.0, r.y + 4.0, 56.0, 44.0);
    let (h, _, c) = l.ui.interact(id_of("livery-mirror"), m);
    if c {
        l.livery.mirror = !l.livery.mirror;
    }
    let on = l.livery.mirror;
    l.ui.p().rounded(m, 8.0, if on { ACCENT() } else if h { HOVER() } else { FIELD() });
    l.ui.icon("flip", Vec2::new(m.center().x, m.y + 15.0), 18.0, if on { ON_ACCENT() } else { TEXT_SOFT() });
    l.ui.text_in("Mirror", Rect::new(m.x, m.y + 26.0, m.w, 14.0), 10.5, Weight::Medium, if on { ON_ACCENT() } else { TEXT_DIM() }, Align::Center);
    l.ui.tooltip(m, "Names and logos on both sides of the bus");
    let u = Vec2::new(m.right() + 30.0, r.center().y);
    if l.ui.icon_button("livery-undo", u, 16.0, "undo", "Undo") {
        if let Some(prev) = l.livery.undo.pop() {
            let now = std::mem::replace(&mut l.livery.layers, prev);
            l.livery.redo.push(now);
            l.livery.committed = l.livery.layers.clone();
            l.livery.selected = None;
        }
    }
    if l.ui.icon_button("livery-redo", Vec2::new(u.x + 38.0, u.y), 16.0, "redo", "Redo") {
        if let Some(next) = l.livery.redo.pop() {
            let now = std::mem::replace(&mut l.livery.layers, next);
            l.livery.undo.push(now);
            l.livery.committed = l.livery.layers.clone();
            l.livery.selected = None;
        }
    }

    let save_r = Rect::new(r.right() - 158.0, r.y + 8.0, 150.0, 36.0);
    let name_r = Rect::new(save_r.x - 230.0, r.y + 8.0, 220.0, 36.0);
    if name_r.x > u.x + 70.0 {
        l.ui.text_input("livery-name", name_r, &mut l.livery.name, "Name of the livery", None);
    }
    if l.ui.button("livery-save", save_r, "Save to game", Some("save"), ButtonKind::Primary) {
        match save(l) {
            Ok(dir) => {
                log::info!("livery studio: '{}' saved into {dir}", l.livery.name);
                l.state.set_status(format!("{} {dir}", omsi_ui::tr("Livery saved into")), false);
            }
            Err(e) => l.state.set_status(omsi_ui::tr(&e).to_string(), true),
        }
    }
}

/// A layer put on by a tool or a preset (on top, chosen).
fn add_layer(l: &mut Launcher, kind: Kind) {
    let v = &mut l.livery;
    let c = parse_hex(&v.hex).unwrap_or([255, 255, 255]);
    let mut layer = Layer::new(kind, c);
    match kind {
        Kind::Base => {
            // (one base: Fill colours the one there is)
            if let Some(i) = v.layers.iter().position(|l| l.kind == Kind::Base) {
                v.layers[i].color = c;
                v.selected = Some(i);
                return;
            }
            v.layers.insert(0, layer);
            v.selected = Some(0);
            v.target = 0;
            return;
        }
        Kind::Text => {
            layer.text = if v.on_bus.trim().is_empty() { "openOMSI".into() } else { v.on_bus.clone() };
            layer.color = [255, 255, 255];
            v.target = 2;
        }
        Kind::Logo => {
            // (the system's own file dialog: a PNG with transparency keeps the bus around it)
            let Some(path) = omsi_launcher_lib::pick_file("A logo (PNG, JPG)") else { return };
            match image::open(&path) {
                Ok(img) => {
                    v.next_image += 1;
                    v.images.insert(v.next_image, Arc::new(img.to_rgba8()));
                    layer.image = v.next_image;
                    layer.name = std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Logo".into());
                }
                Err(e) => {
                    l.state.set_status(format!("{}: {e}", omsi_ui::tr("The logo cannot be read")), true);
                    return;
                }
            }
        }
        _ => v.target = 1,
    }
    v.layers.push(layer);
    v.selected = Some(v.layers.len() - 1);
}

/// The layers, top first, and the buttons that order them.
fn layers_panel(l: &mut Launcher, r: Rect) {
    l.ui.panel(r);
    let inner = Rect::new(r.x + 12.0, r.y + 12.0, r.w - 24.0, r.h - 24.0);
    l.ui.label(Rect::new(inner.x, inner.y, inner.w, 18.0), "Layers");
    let list = Rect::new(inner.x, inner.y + 26.0, inner.w, inner.h - 26.0 - 96.0);
    let n = l.livery.layers.len();
    if n == 0 {
        l.ui.paragraph("No layers yet. Pick a colour and a stripe on the right, or a tool above.", Vec2::new(list.x, list.y + 4.0), list.w, 12.0, Weight::Regular, TEXT_DIM());
    }
    let mut pick = None;
    let mut toggle_vis = None;
    let mut toggle_lock = None;
    for (row, k) in (0..n).rev().enumerate() {
        let y = list.y + row as f32 * 38.0;
        if y + 34.0 > list.bottom() {
            break;
        }
        let rr = Rect::new(list.x, y, list.w, 34.0);
        let sel = l.livery.selected == Some(k);
        let layer = l.livery.layers[k].clone();
        if l.ui.row(&format!("livery-layer-{k}"), rr, sel) {
            pick = Some(k);
        }
        let eye = Rect::new(rr.x + 4.0, rr.y + 5.0, 24.0, 24.0);
        let (_, _, c) = l.ui.interact(id_of(&format!("livery-eye-{k}")), eye);
        if c {
            toggle_vis = Some(k);
        }
        l.ui.icon(if layer.visible { "visibility" } else { "visibility_off" }, eye.center(), 15.0, if layer.visible { TEXT_SOFT() } else { TEXT_FAINT() });
        l.ui.text_in(&layer.name, Rect::new(rr.x + 32.0, rr.y, rr.w - 84.0, rr.h), 12.5, if sel { Weight::Bold } else { Weight::Medium }, if layer.visible { TEXT() } else { TEXT_DIM() }, Align::Left);
        let sw = Rect::new(rr.right() - 50.0, rr.y + 10.0, 14.0, 14.0);
        l.ui.p().rounded(sw, 3.0, col(layer.color));
        l.ui.p().rounded_border(sw, 3.0, 1.0, EDGE());
        let lock = Rect::new(rr.right() - 30.0, rr.y + 5.0, 24.0, 24.0);
        let (_, _, c) = l.ui.interact(id_of(&format!("livery-lock-{k}")), lock);
        if c {
            toggle_lock = Some(k);
        }
        l.ui.icon(if layer.locked { "lock" } else { "lock_open" }, lock.center(), 14.0, if layer.locked { ACCENT_2() } else { TEXT_FAINT() });
    }
    let v = &mut l.livery;
    if let Some(k) = toggle_vis {
        v.layers[k].visible = !v.layers[k].visible;
    } else if let Some(k) = toggle_lock {
        v.layers[k].locked = !v.layers[k].locked;
    } else if let Some(k) = pick {
        v.selected = Some(k);
        v.target = 3;
        let c = v.layers[k].color;
        v.hsv = rgb_to_hsv(c);
        v.hex = hex_of(c);
    }
    // up, down, copy, delete
    let by = list.bottom() + 8.0;
    let bw = (inner.w - 18.0) / 4.0;
    let icons = [("vertical_align_top", "Move up"), ("arrow_downward", "Move down"), ("content_copy", "Duplicate"), ("delete", "Delete")];
    for (k, (icon, tip)) in icons.iter().enumerate() {
        let b = Rect::new(inner.x + k as f32 * (bw + 6.0), by, bw, 34.0);
        if l.ui.button(&format!("livery-order-{k}"), b, "", Some(icon), ButtonKind::Normal) {
            let v = &mut l.livery;
            if let Some(i) = v.selected.filter(|i| *i < v.layers.len()) {
                match k {
                    0 if i + 1 < v.layers.len() => {
                        v.layers.swap(i, i + 1);
                        v.selected = Some(i + 1);
                    }
                    1 if i > 0 => {
                        v.layers.swap(i, i - 1);
                        v.selected = Some(i - 1);
                    }
                    2 => {
                        let mut c = v.layers[i].clone();
                        c.name = format!("{} 2", c.name);
                        v.layers.insert(i + 1, c);
                        v.selected = Some(i + 1);
                    }
                    3 if !v.layers[i].locked => {
                        v.layers.remove(i);
                        v.selected = None;
                    }
                    _ => {}
                }
            }
        }
        l.ui.tooltip(b, tip);
    }
    let about = l.livery.surface.as_ref().map(|s| s.about.clone()).unwrap_or_default();
    if !about.is_empty() {
        l.ui.paragraph(&about, Vec2::new(inner.x, by + 44.0), inner.w, 10.5, Weight::Regular, TEXT_FAINT());
    }
}

/// The right panel: the quick livery (three colours, a stripe, a name and a logo), the
/// colour picker, and the chosen layer's own settings.
fn right_panel(l: &mut Launcher, r: Rect) {
    l.ui.panel(r);
    let inner = Rect::new(r.x + 14.0, r.y + 12.0, r.w - 28.0, r.h - 24.0);
    l.ui.push_clip(r, RADIUS);
    let mut y = inner.y;
    l.ui.label(Rect::new(inner.x, y, inner.w, 18.0), "Quick livery");
    y += 24.0;
    // the three colours the quick livery is made of
    let slots = ["Body", "Stripe", "Name"];
    let sw = (inner.w - 16.0) / 3.0;
    for (k, name) in slots.iter().enumerate() {
        let c = Rect::new(inner.x + k as f32 * (sw + 8.0), y, sw, 26.0);
        let id = id_of(&format!("livery-slot-{k}"));
        let (h, _, clicked) = l.ui.interact(id, c);
        let colour = {
            let v = &l.livery;
            let found = match k {
                0 => v.layers.iter().find(|l| l.kind == Kind::Base),
                1 => v.layers.iter().find(|l| STRIPES.iter().any(|s| s.0 == l.kind)),
                _ => v.layers.iter().find(|l| l.kind == Kind::Text),
            };
            found.map(|l| l.color)
        };
        match colour {
            Some(cc) => l.ui.p().rounded(c, 5.0, col(cc)),
            None => l.ui.p().rounded(c, 5.0, FIELD()),
        }
        let on = l.livery.target == k;
        l.ui.p().rounded_border(c, 5.0, if on { 2.0 } else { 1.0 }, if on { ACCENT() } else if h { TEXT_DIM() } else { EDGE() });
        l.ui.text_in(name, Rect::new(c.x, c.bottom() + 2.0, c.w, 14.0), 10.5, Weight::Medium, TEXT_DIM(), Align::Center);
        if clicked {
            l.livery.target = k;
            if let Some(cc) = colour {
                l.livery.hsv = rgb_to_hsv(cc);
                l.livery.hex = hex_of(cc);
            }
        }
    }
    y += 46.0;
    // the picker
    let mut hsv = l.livery.hsv;
    if picker(&mut l.ui, Rect::new(inner.x, y, inner.w, 120.0), &mut hsv) {
        l.livery.hsv = hsv;
        let c = hsv_to_rgb(hsv);
        l.livery.hex = hex_of(c);
        set_target_color(&mut l.livery, c);
    }
    y += 128.0;
    let hex_r = Rect::new(inner.x, y, 110.0, 30.0);
    if l.ui.text_input("livery-hex", hex_r, &mut l.livery.hex, "#rrggbb", None) {
        if let Some(c) = parse_hex(&l.livery.hex) {
            l.livery.hsv = rgb_to_hsv(c);
            set_target_color(&mut l.livery, c);
        }
    }
    let now = parse_hex(&l.livery.hex).unwrap_or([255, 255, 255]);
    let cur = Rect::new(hex_r.right() + 8.0, y, 30.0, 30.0);
    l.ui.p().rounded(cur, 5.0, col(now));
    l.ui.p().rounded_border(cur, 5.0, 1.0, EDGE());
    // the colour of the bus's own paint under the mouse is not read from the 3D picture:
    // "From the bus" takes the body texture's mean colour
    let from = Rect::new(cur.right() + 8.0, y, inner.right() - cur.right() - 8.0, 30.0);
    if l.ui.button("livery-from-bus", from, "From the bus", Some("colorize"), ButtonKind::Normal) {
        if let Some(s) = l.livery.surface.clone() {
            let (mut sum, mut n) = ([0f64; 3], 0f64);
            for (i, p) in s.base.pixels().enumerate().step_by(7) {
                if s.nrm[i] != Vec3::ZERO {
                    for j in 0..3 {
                        sum[j] += p.0[j] as f64;
                    }
                    n += 1.0;
                }
            }
            if n > 0.0 {
                let c = [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8];
                l.livery.hsv = rgb_to_hsv(c);
                l.livery.hex = hex_of(c);
                set_target_color(&mut l.livery, c);
            }
        }
    }
    y += 38.0;
    // the swatches and the colours used last
    let cell = (inner.w - 7.0 * 4.0) / 8.0;
    let mut chosen = None;
    for (k, c) in SWATCHES.iter().enumerate() {
        let s = Rect::new(inner.x + (k % 8) as f32 * (cell + 4.0), y + (k / 8) as f32 * (cell * 0.7 + 4.0), cell, cell * 0.7);
        let (h, _, clicked) = l.ui.interact(id_of(&format!("livery-sw-{k}")), s);
        l.ui.p().rounded(s, 3.0, col(*c));
        l.ui.p().rounded_border(s, 3.0, if h { 2.0 } else { 1.0 }, if h { TEXT() } else { EDGE() });
        if clicked {
            chosen = Some(*c);
        }
    }
    y += 2.0 * (cell * 0.7 + 4.0) + 4.0;
    let used = l.livery.used.clone();
    if !used.is_empty() {
        l.ui.text_in("Used last", Rect::new(inner.x, y, inner.w, 14.0), 10.5, Weight::Medium, TEXT_FAINT(), Align::Left);
        y += 16.0;
        for (k, c) in used.iter().enumerate() {
            let s = Rect::new(inner.x + k as f32 * (cell + 4.0), y, cell, cell * 0.7);
            let (h, _, clicked) = l.ui.interact(id_of(&format!("livery-used-{k}")), s);
            l.ui.p().rounded(s, 3.0, col(*c));
            l.ui.p().rounded_border(s, 3.0, if h { 2.0 } else { 1.0 }, if h { TEXT() } else { EDGE() });
            if clicked {
                chosen = Some(*c);
            }
        }
        y += cell * 0.7 + 8.0;
    }
    if let Some(c) = chosen {
        l.livery.hsv = rgb_to_hsv(c);
        l.livery.hex = hex_of(c);
        set_target_color(&mut l.livery, c);
    }
    // a colour that stays chosen once the mouse lets go is remembered
    if l.ui.input.released {
        if let Some(c) = target_color(&l.livery) {
            if l.livery.used.first() != Some(&c) {
                remember(&mut l.livery, c);
            }
        }
    }
    y += 6.0;

    // the selected layer's own settings, else the stripes to put on
    let sel = l.livery.selected.filter(|i| *i < l.livery.layers.len());
    match sel {
        Some(i) if l.livery.layers[i].kind != Kind::Base => layer_settings(l, Rect::new(inner.x, y, inner.w, inner.bottom() - y), i),
        _ => {
            l.ui.label(Rect::new(inner.x, y, inner.w, 18.0), "Stripe");
            y += 22.0;
            let tw = (inner.w - 18.0) / 4.0;
            for (k, (kind, name)) in STRIPES.iter().enumerate() {
                let t = Rect::new(inner.x + (k % 4) as f32 * (tw + 6.0), y + (k / 4) as f32 * 58.0, tw, 52.0);
                let (h, _, clicked) = l.ui.interact(id_of(&format!("livery-stripe-{k}")), t);
                l.ui.p().rounded(t, 6.0, if h { HOVER() } else { FIELD() });
                stripe_icon(&mut l.ui, Rect::new(t.x + 6.0, t.y + 6.0, t.w - 12.0, 24.0), *kind);
                l.ui.text_in(name, Rect::new(t.x, t.y + 33.0, t.w, 14.0), 9.5, Weight::Medium, TEXT_DIM(), Align::Center);
                if clicked {
                    add_layer(l, *kind);
                }
            }
            y += 2.0 * 58.0 + 6.0;
            l.ui.label(Rect::new(inner.x, y, inner.w, 18.0), "Name on the bus");
            y += 22.0;
            if l.ui.text_input("livery-on-bus", Rect::new(inner.x, y, inner.w, 32.0), &mut l.livery.on_bus, "Stadtwerke …", Some("text_fields")) {
                let t = l.livery.on_bus.clone();
                if let Some(layer) = l.livery.layers.iter_mut().find(|l| l.kind == Kind::Text) {
                    layer.text = t;
                }
            }
            y += 40.0;
            l.ui.label(Rect::new(inner.x, y, inner.w, 18.0), "Logo");
            y += 22.0;
            if l.ui.button("livery-logo", Rect::new(inner.x, y, inner.w, 32.0), "Choose a logo…", Some("upload"), ButtonKind::Normal) {
                add_layer(l, Kind::Logo);
            }
            y += 42.0;
            l.ui.paragraph("Drag on the bus to turn it, the wheel zooms. A layer chosen on the left shows its settings here.", Vec2::new(inner.x, y), inner.w, 11.0, Weight::Regular, TEXT_FAINT());
        }
    }
    l.ui.pop_clip();
}

/// A stripe preset's little picture: a bus side with the stripe on it.
fn stripe_icon(ui: &mut Ui, r: Rect, kind: Kind) {
    ui.p().rounded(r, 3.0, TEXT_SOFT());
    let c = ACCENT();
    let band = |ui: &mut Ui, a: f32, b: f32| ui.p().rect(Rect::new(r.x, r.y + r.h * (1.0 - b), r.w, r.h * (b - a)), c);
    match kind {
        Kind::Skirt => band(ui, 0.08, 0.28),
        Kind::Window => band(ui, 0.5, 0.82),
        Kind::Roof => band(ui, 0.86, 1.0),
        Kind::TwoTone => band(ui, 0.0, 0.45),
        Kind::Slanted => ui.p().convex(&[Vec2::new(r.x, r.bottom()), Vec2::new(r.x, r.y + r.h * 0.85), Vec2::new(r.right(), r.y + r.h * 0.25), Vec2::new(r.right(), r.bottom())], c),
        Kind::Wave => {
            for k in 0..12 {
                let t0 = k as f32 / 12.0;
                let t1 = (k + 1) as f32 / 12.0;
                let y = |t: f32| r.y + r.h * (0.62 - 0.15 * (t * std::f32::consts::TAU * 2.0).sin());
                ui.p().line(Vec2::new(r.x + r.w * t0, y(t0)), Vec2::new(r.x + r.w * t1, y(t1)), 3.0, c);
            }
        }
        Kind::Front => ui.p().rect(Rect::new(r.right() - r.w * 0.16, r.y, r.w * 0.16, r.h), c),
        Kind::Rear => ui.p().rect(Rect::new(r.x, r.y, r.w * 0.16, r.h), c),
        _ => {}
    }
    // the windows
    for k in 0..4 {
        ui.p().rect(Rect::new(r.x + r.w * (0.08 + k as f32 * 0.22), r.y + r.h * 0.22, r.w * 0.16, r.h * 0.26), PANEL().alpha(0.7));
    }
}

/// The chosen layer's settings: where, how thick, how see-through.
fn layer_settings(l: &mut Launcher, r: Rect, i: usize) {
    let mut y = r.y;
    let kind = l.livery.layers[i].kind;
    let name = l.livery.layers[i].name.clone();
    l.ui.label(Rect::new(r.x, y, r.w - 60.0, 18.0), &name);
    if l.ui.button("livery-layer-done", Rect::new(r.right() - 56.0, y - 4.0, 56.0, 26.0), "Done", None, ButtonKind::Ghost) {
        l.livery.selected = None;
        l.livery.target = 0;
        return;
    }
    y += 26.0;
    let locked = l.livery.layers[i].locked;
    let pct = |v: f32| format!("{:.0} %", v * 100.0);
    let m = |v: f32| format!("{v:.2} m");
    let layer = &mut l.livery.layers[i];
    let row = |ui: &mut Ui, y: &mut f32, id: &str, label: &str, v: &mut f32, lo: f32, hi: f32, f: &dyn Fn(f32) -> String| {
        let mut x = *v;
        if ui.slider(id, Rect::new(r.x, *y, r.w, ROW), &mut x, lo, hi, 0.0, label, f) && !locked {
            *v = x;
        }
        *y += ROW + 6.0;
    };
    match kind {
        Kind::Text | Kind::Logo => {
            if kind == Kind::Text {
                let mut t = layer.text.clone();
                if l.ui.text_input("livery-layer-text", Rect::new(r.x, y, r.w, 32.0), &mut t, "Text", Some("text_fields")) && !locked {
                    layer.text = t;
                }
                y += 40.0;
            }
            row(&mut l.ui, &mut y, "livery-l-along", "Along", &mut layer.along, 0.0, 1.0, &pct);
            row(&mut l.ui, &mut y, "livery-l-height", "Height", &mut layer.height, 0.0, 1.0, &pct);
            row(&mut l.ui, &mut y, "livery-l-size", "Size", &mut layer.size, 0.1, 2.0, &m);
        }
        Kind::Front | Kind::Rear => {}
        _ => {
            row(&mut l.ui, &mut y, "livery-l-height", "Height", &mut layer.height, 0.0, 1.0, &pct);
            if matches!(kind, Kind::Skirt | Kind::Window | Kind::Wave) {
                row(&mut l.ui, &mut y, "livery-l-size", "Width", &mut layer.size, 0.01, 0.6, &pct);
            }
            if matches!(kind, Kind::Slanted | Kind::Wave) {
                row(&mut l.ui, &mut y, "livery-l-amount", if kind == Kind::Wave { "Wave" } else { "Slope" }, &mut layer.amount, -0.6, 0.6, &pct);
            }
        }
    }
    let layer = &mut l.livery.layers[i];
    row(&mut l.ui, &mut y, "livery-l-opacity", "Opacity", &mut layer.opacity, 0.0, 1.0, &pct);
    let _ = y;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_go_round_hsv_unchanged() {
        for c in [[255, 0, 0], [0, 84, 159], [247, 168, 0], [214, 40, 130], [17, 17, 17], [255, 255, 255]] {
            assert_eq!(hsv_to_rgb(rgb_to_hsv(c)), c);
            assert_eq!(parse_hex(&hex_of(c)), Some(c));
        }
        assert_eq!(parse_hex("nope"), None);
    }

    /// A box-shaped bus: its two sides laid out side by side in the texture.
    fn surface() -> Surface {
        let (w, h) = (64u32, 32u32);
        let mut pos = vec![Vec3::ZERO; (w * h) as usize];
        let mut nrm = vec![Vec3::ZERO; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                let right = x >= w / 2;
                let u = (x % (w / 2)) as f32 / (w / 2) as f32;
                pos[i] = Vec3::new(if right { 1.25 } else { -1.25 }, -6.0 + 12.0 * u, 3.0 * (1.0 - y as f32 / h as f32));
                nrm[i] = Vec3::new(if right { 1.0 } else { -1.0 }, 0.0, 0.0);
            }
        }
        let base = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 200, 200, 255]));
        Surface { tex: 0, also: Vec::new(), ctc: None, file: String::new(), ctc_dir: None, base, pos, nrm, lo: Vec3::new(-1.25, -6.0, 0.0), hi: Vec3::new(1.25, 6.0, 3.0), mean: 200.0 / 255.0, about: String::new() }
    }

    #[test]
    fn a_two_tone_paints_the_lower_part_only() {
        let s = surface();
        let mut l = Layer::new(Kind::TwoTone, [255, 0, 0]);
        l.height = 0.5;
        let img = compose(&s, &[l], &[None], true);
        let low = img.get_pixel(5, 30).0;
        let high = img.get_pixel(5, 2).0;
        assert!(low[0] > 200 && low[1] < 40, "{low:?}");
        assert_eq!(high, [200, 200, 200, 255]);
    }

    #[test]
    fn a_name_is_on_one_side_unless_mirrored() {
        let s = surface();
        let mut l = Layer::new(Kind::Text, [0, 0, 255]);
        l.height = 0.5;
        l.along = 0.5;
        l.size = 2.0;
        let stamp = Stamp { w: 4, h: 2, rgba: vec![255; 32] };
        let one = compose(&s, &[l.clone()], &[Some(Stamp { w: 4, h: 2, rgba: stamp.rgba.clone() })], false);
        // the middle of each side
        assert_eq!(one.get_pixel(16, 16).0[2], 200);
        assert!(one.get_pixel(48, 16).0[2] > 240);
        let both = compose(&s, &[l], &[Some(stamp)], true);
        assert!(both.get_pixel(16, 16).0[2] > 240);
    }
}
