//! A look at the studio's paint on a real bus without a window: the bus read from the OMSI 2
//! folder (only read), its textures baked and painted as the studio does, and the bus drawn from
//! both sides into PNGs with the stripes' guide lines over it. Run by hand:
//!
//! `OMSI_LIVERY_LOOK=<OMSI folder>|<bus file>|<out folder> cargo test -p omsi-app --lib livery::headless -- --ignored --nocapture`

use super::bake::{self, BusGeom, Outside, Zones};
use super::model::{self, BusDims, Kind, Layer, Place, Side, StripeTemplate};
use super::paint::{self, Canvas, Context, RasterCache};
use super::{bus_meshes, PartIn};
use glam::{Mat4, Vec2, Vec3};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Painted {
    index: u8,
    w: u32,
    h: u32,
    rgba: Vec<u8>,
}

/// The bus's parts as the showroom has them: the front and its coupled rear sections, standing
/// straight.
fn bus(root: &Path, file: &str) -> omsi_sim::VehicleInstance {
    let path = crate::spawn::player_bus_path(root, file).expect("the bus file");
    let vt = Arc::new(omsi_sim::VehicleType::load(root, &path).expect("the bus"));
    let mut host = omsi_sim::VehicleHost::new(omsi_sim::SimClock::default());
    host.paint_scheme = Some(None);
    let mut v = omsi_sim::VehicleInstance::new(vt, host);
    v.apply_paint_vars(None);
    crate::spawn::load_coupled_parts(root, &mut v);
    for _ in 0..3 {
        v.update(1.0 / 30.0);
    }
    v
}

/// The layers looked at: a white base, an orange skirt, a green wave, a text and an arrow on the
/// right side (copied to the left), a text on the rear section.
fn layers(d: &BusDims) -> Vec<Layer> {
    let mut v = vec![model::layer("Base", model::base_colour("#f4f4f4"))];
    v.push(model::layer("Skirt", model::stripe(StripeTemplate::Skirt, "#f28c28", d)));
    let mut wave = model::stripe(StripeTemplate::Wave, "#1a8a4f", d);
    if let Kind::Stripe { h1, h2, .. } = &mut wave {
        (*h1, *h2) = (d.window_height() * 0.45, d.window_height() * 0.75);
    }
    v.push(model::layer("Wave", wave));
    let text = |s: &str, along: f32| {
        let width = super::shapes::text_width(s, super::shapes::DEFAULT_FONT) * 0.35;
        Kind::Text { text: s.into(), font: super::shapes::DEFAULT_FONT.into(), height_cm: 35.0, colour: "#1d3f8f".into(), outline: None, spacing: 0.0, gradient: None, place: model::place_on(Side::R, along, d.window_height() * 0.9, width, d) }
    };
    v.push(model::layer("Name", text("Bus 42", 0.8)));
    v.push(model::layer("Rear", text("REAR", 0.15)));
    let arrow = Place { height_m: Some(0.4), ..model::place_on(Side::R, 0.55, d.window_height() * 0.9, 0.8, d) };
    v.push(model::layer("Arrow", Kind::Shape { shape: "pijl".into(), colour: "#d93a30".into(), outline: None, gradient: None, place: arrow }));
    for l in v.iter_mut() {
        l.detail = 0.6;
    }
    v
}

/// The bus seen from one side (`from` +1 the right, -1 the left), orthographic, 100 pixels a
/// metre, the painted textures on it, lit from the viewer; the guide lines of the stripes over it.
fn side_view(geom: &BusGeom, painted: &[Painted], layers: &[Layer], from: f32) -> image::RgbaImage {
    let d = geom.dims;
    let k = 100.0;
    let (w, h) = (((d.length() + 1.0) * k) as u32, ((d.height() + 0.6) * k) as u32);
    let to_px = |p: Vec3| {
        // seen from the right the front is on the right; from the left, on the left
        let x = if from > 0.0 { p.y - d.min.y + 0.5 } else { d.max.y + 0.5 - p.y };
        Vec2::new(x * k, (d.max.z + 0.3 - p.z) * k)
    };
    let mut img = image::RgbaImage::from_pixel(w, h, image::Rgba([96, 98, 104, 255]));
    let mut depth = vec![f32::NEG_INFINITY; (w * h) as usize];
    for t in &geom.tris {
        let q = t.p.map(to_px);
        let area = (q[1] - q[0]).perp_dot(q[2] - q[0]);
        if area.abs() < 1e-6 {
            continue;
        }
        let tex = t.target.and_then(|i| painted.iter().find(|p| p.index == i));
        let light = 0.35 + 0.65 * (t.n.x * from).max(0.0);
        let lo = q[0].min(q[1]).min(q[2]).max(Vec2::ZERO);
        let hi = q[0].max(q[1]).max(q[2]).min(Vec2::new(w as f32 - 1.0, h as f32 - 1.0));
        for y in lo.y as u32..=hi.y as u32 {
            for x in lo.x as u32..=hi.x as u32 {
                let c = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let b = Vec3::new((q[1] - c).perp_dot(q[2] - c), (q[2] - c).perp_dot(q[0] - c), (q[0] - c).perp_dot(q[1] - c)) / area;
                if b.min_element() < 0.0 {
                    continue;
                }
                let dep = (t.p[0].x * b.x + t.p[1].x * b.y + t.p[2].x * b.z) * from;
                let i = (y * w + x) as usize;
                if dep <= depth[i] {
                    continue;
                }
                depth[i] = dep;
                let rgb = match tex {
                    Some(p) => {
                        let uv = t.uv[0] * b.x + t.uv[1] * b.y + t.uv[2] * b.z;
                        let tx = ((uv.x.rem_euclid(1.0) * p.w as f32) as u32).min(p.w - 1);
                        let ty = ((uv.y.rem_euclid(1.0) * p.h as f32) as u32).min(p.h - 1);
                        let o = ((ty * p.w + tx) * 4) as usize;
                        [p.rgba[o], p.rgba[o + 1], p.rgba[o + 2]]
                    }
                    None => [150, 150, 150],
                };
                let l = if t.glass { 1.0 } else { light };
                img.put_pixel(x, y, image::Rgba([(rgb[0] as f32 * l) as u8, (rgb[1] as f32 * l) as u8, (rgb[2] as f32 * l) as u8, 255]));
            }
        }
    }
    // the guide lines of the stripes, as the studio draws them
    for l in layers {
        if let Kind::Stripe { h1, h2, angle, wave, .. } = &l.kind {
            for hh in [*h1, *h2] {
                let mut y = d.min.y;
                while y <= d.max.y {
                    let (e, _) = model::stripe_edges(hh, hh, *angle, *wave, y, &d);
                    let p = to_px(Vec3::new(0.0, y, d.min.z + e));
                    if p.x >= 0.0 && p.y >= 0.0 && (p.x as u32) < w && (p.y as u32) < h {
                        img.put_pixel(p.x as u32, p.y as u32, image::Rgba([255, 140, 0, 255]));
                    }
                    y += 0.004;
                }
            }
        }
    }
    img
}

#[test]
#[ignore]
fn a_real_bus_painted_from_both_sides() {
    let Ok(spec) = std::env::var("OMSI_LIVERY_LOOK") else { return };
    let mut it = spec.split('|');
    let (root, file, out) = (PathBuf::from(it.next().unwrap()), it.next().unwrap().to_string(), PathBuf::from(it.next().unwrap()));
    std::fs::create_dir_all(&out).unwrap();
    let v = bus(&root, &file);
    let mut list = vec![PartIn { ty: &v.ty, scheme: None, visible: v.mesh_props.iter().map(|p| p.visible).collect(), xf: (0..v.ty.meshes.len()).map(|i| v.mesh_local_transform(i)).collect() }];
    for t in &v.trailers {
        let off = Mat4::from_translation((t.position - v.position).as_vec3());
        list.push(PartIn { ty: &t.ty, scheme: None, visible: t.mesh_props.iter().map(|p| p.visible).collect(), xf: (0..t.ty.meshes.len()).map(|i| off * t.mesh_local_transform(i)).collect() });
    }
    let (textures, found, kept) = bus_meshes(&list);
    println!("{} part(s); textures {:?}", list.len(), textures.iter().map(|t| (&t.file, &t.slots, &t.low)).collect::<Vec<_>>());
    println!("kept from the paint: {kept:?}");
    let meshes: Vec<_> = found.into_iter().map(|f| f.2).collect();
    let geom = BusGeom::build(&meshes);
    println!("box {:?} - {:?}, window line {:?}", geom.dims.min, geom.dims.max, geom.dims.window);
    let mut area = vec![0.0f32; textures.len()];
    for t in &geom.tris {
        if let Some(k) = t.target {
            area[k as usize] += (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).length() * 0.5;
        }
    }
    let outside = Outside::build(&geom.tris);
    let mut bakes = Vec::new();
    for (k, t) in textures.iter().enumerate() {
        if area[k] < 0.5 {
            continue;
        }
        let ty = list[t.slots[0].0].ty;
        let dirs = ty.texture_dirs(&root);
        let refs: Vec<&Path> = dirs.iter().map(|d| d.as_path()).collect();
        let Some(base) = omsi_texture::find_texture(&t.file, &refs) else {
            println!("{}: not found", t.file);
            continue;
        };
        let img = omsi_texture::decode_file(&base).unwrap();
        let (ow, oh) = model::output_size(img.width, img.height);
        let (w, h) = model::edit_size(ow, oh);
        let rgba = if (w, h) != (img.width, img.height) { omsi_texture::bc::resize(&img.rgba, img.width, img.height, w, h) } else { img.rgba };
        let b = bake::Bake::build(&geom.tris, k as u8, w, h, 0, h, &outside);
        let outs = b.first.iter().filter(|s| s.flags & bake::OUTSIDE != 0).count();
        let keep = b.first.iter().filter(|s| s.flags & bake::KEEP != 0).count();
        let glass = b.first.iter().filter(|s| s.flags & bake::GLASS != 0).count();
        println!("{} {}x{}: area {:.1} m², {outs} outside, {keep} kept, {glass} glass", t.file, w, h, area[k]);
        if outs == 0 {
            continue;
        }
        bakes.push((k as u8, w, h, rgba, b));
    }
    let colours: Vec<Vec<[u8; 3]>> = bakes.iter().map(|(_, _, _, rgba, b)| paint::outside_colours(b, rgba)).collect();
    let zones = Zones::of_targets(&colours);
    let layers = layers(&geom.dims);
    let mut cache = RasterCache::default();
    let pictures = Arc::new(HashMap::new());
    let mut painted = Vec::new();
    for ((index, w, h, rgba, b), zones) in bakes.into_iter().zip(&zones) {
        println!("zones of {index}: {:?} shares {:?} groups {:?}", zones.centres, zones.share, zones.group);
        let cv = Canvas::with(b, &rgba, zones);
        let cx = Context { pictures: pictures.clone(), mirror: Some(geom.dims.middle_x()), density: cv.bake.density, max_px: 2048 };
        let ops = paint::prepare(&layers, &cx, &mut cache);
        let (out_rgba, _) = paint::composite(&cv, &ops, &geom.dims, None, None);
        image::RgbaImage::from_raw(w, h, out_rgba.clone()).unwrap().save(out.join(format!("texture_{index}.png"))).unwrap();
        painted.push(Painted { index, w, h, rgba: out_rgba });
    }
    for (name, from) in [("right", 1.0), ("left", -1.0)] {
        side_view(&geom, &painted, &layers, from).save(out.join(format!("side_{name}.png"))).unwrap();
    }
    println!("written into {}", out.display());
}

// --- before and after, bus by bus ---------------------------------------------------------------

/// A bus read for a look: its shape, its painted textures baked at the editing size with their
/// bases, and the colour zones.
struct Look {
    geom: BusGeom,
    bakes: Vec<(u8, u32, u32, Vec<u8>, bake::Bake)>,
    zones: Vec<Zones>,
    /// The maker's template of each (MA, MU at its size).
    templates: Vec<Option<[Vec<u8>; 2]>>,
}

fn look(root: &Path, file: &str, paint: Option<&str>) -> Look {
    let v = bus(root, file);
    let scheme = crate::spawn::paint_scheme(&v.ty, paint);
    let mut list = vec![PartIn { ty: &v.ty, scheme, visible: v.mesh_props.iter().map(|p| p.visible).collect(), xf: (0..v.ty.meshes.len()).map(|i| v.mesh_local_transform(i)).collect() }];
    for t in &v.trailers {
        let off = Mat4::from_translation((t.position - v.position).as_vec3());
        list.push(PartIn { ty: &t.ty, scheme: crate::spawn::part_scheme(&v.ty, scheme, &t.ty), visible: t.mesh_props.iter().map(|p| p.visible).collect(), xf: (0..t.ty.meshes.len()).map(|i| off * t.mesh_local_transform(i)).collect() });
    }
    let (textures, found, kept) = bus_meshes(&list);
    println!("  kept from the paint: {kept:?}");
    let meshes: Vec<_> = found.into_iter().map(|f| f.2).collect();
    let geom = BusGeom::build(&meshes);
    let mut area = vec![0.0f32; textures.len()];
    for t in &geom.tris {
        if let Some(k) = t.target {
            area[k as usize] += (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).length() * 0.5;
        }
    }
    let outside = Outside::build(&geom.tris);
    let mut bakes = Vec::new();
    let mut templates = Vec::new();
    for (k, t) in textures.iter().enumerate() {
        if area[k] < 0.5 {
            continue;
        }
        let PartIn { ty, scheme: sch, .. } = &list[t.slots[0].0];
        let (subs, sdir) = sch.map(|i| ty.scheme_substitutions(i)).unwrap_or_default();
        let mut dirs: Vec<PathBuf> = sdir.iter().cloned().collect();
        dirs.extend(ty.texture_dirs(root));
        let refs: Vec<&Path> = dirs.iter().map(|d| d.as_path()).collect();
        let wanted = subs.get(&t.key).cloned().unwrap_or_else(|| t.file.clone());
        let base = sdir.as_ref().map(|d| omsi_cfg::resolve_path(d, &wanted)).filter(|p| p.is_file()).or_else(|| omsi_texture::find_texture(&wanted, &refs)).or_else(|| omsi_texture::find_texture(&t.file, &refs));
        let Some(base) = base else {
            println!("  {}: not found", t.file);
            continue;
        };
        let img = omsi_texture::decode_file(&base).unwrap();
        let (ow, oh) = model::output_size(img.width, img.height);
        let (w, h) = model::edit_size(ow, oh);
        let rgba = if (w, h) != (img.width, img.height) { omsi_texture::bc::resize(&img.rgba, img.width, img.height, w, h) } else { img.rgba };
        let b = bake::Bake::build(&geom.tris, k as u8, w, h, 0, h, &outside);
        if !b.first.iter().any(|s| s.flags & bake::OUTSIDE != 0) {
            continue;
        }
        let tp = super::find_template(root, &t.file);
        println!("  texture {k}: {} ({}) {}x{}, {:.1} m²{}", t.file, base.display(), w, h, area[k], if tp.is_some() { ", maker's template" } else { "" });
        templates.push(tp.and_then(|tp| super::template_pictures(&tp, w, h)));
        bakes.push((k as u8, w, h, rgba, b));
    }
    let colours: Vec<Vec<[u8; 3]>> = bakes.iter().map(|(_, _, _, rgba, b)| paint::outside_colours(b, rgba)).collect();
    let zones = Zones::of_targets(&colours);
    for (z, (index, ..)) in zones.iter().zip(&bakes) {
        println!("  texture {index}:");
        for i in 0..z.centres.len() {
            let c = z.centres[i];
            println!("    zone {i}: L {:.0} a {:.0} b {:.0}, {:.1} %{}", c[0], c[1], c[2], z.share[i] * 100.0, if z.painted[i] { format!(", paint {}", z.group[i]) } else { String::new() });
        }
    }
    Look { geom, bakes, zones, templates }
}

/// Which way a view looks at the bus.
#[derive(Clone, Copy)]
enum View {
    Right,
    Left,
    Front,
}

/// The bus drawn orthographically from `view`, `k` pixels a metre, lit from above and in front
/// of the viewer; `painted` the textures on it.
fn draw(geom: &BusGeom, painted: &[Painted], view: View, k: f32) -> image::RgbaImage {
    let d = geom.dims;
    let w_m = match view {
        View::Right | View::Left => d.length(),
        View::Front => d.max.x - d.min.x,
    };
    let (w, h) = (((w_m + 0.6) * k) as u32, ((d.height() + 0.6) * k) as u32);
    let project = |p: Vec3| -> (Vec2, f32) {
        let (x, dep) = match view {
            View::Right => (p.y - d.min.y + 0.3, p.x),
            View::Left => (d.max.y + 0.3 - p.y, -p.x),
            View::Front => (d.max.x + 0.3 - p.x, p.y),
        };
        (Vec2::new(x * k, (d.max.z + 0.3 - p.z) * k), dep)
    };
    let toward = match view {
        View::Right => Vec3::X,
        View::Left => Vec3::NEG_X,
        View::Front => Vec3::Y,
    };
    let sun = (toward + Vec3::Z * 0.8 + toward.cross(Vec3::Z) * 0.3).normalize();
    let mut img = image::RgbaImage::from_pixel(w, h, image::Rgba([170, 190, 215, 255]));
    let mut depth = vec![f32::NEG_INFINITY; (w * h) as usize];
    // the opaque first, then the glass over them
    for glass_pass in [false, true] {
        for t in geom.tris.iter().filter(|t| t.glass == glass_pass) {
            let q = t.p.map(|p| project(p).0);
            let z = t.p.map(|p| project(p).1);
            let area = (q[1] - q[0]).perp_dot(q[2] - q[0]);
            if area.abs() < 1e-6 {
                continue;
            }
            let tex = t.target.and_then(|i| painted.iter().find(|p| p.index == i));
            let lo = q[0].min(q[1]).min(q[2]).max(Vec2::ZERO);
            let hi = q[0].max(q[1]).max(q[2]).min(Vec2::new(w as f32 - 1.0, h as f32 - 1.0));
            if lo.x > hi.x || lo.y > hi.y {
                continue;
            }
            for y in lo.y as u32..=hi.y as u32 {
                for x in lo.x as u32..=hi.x as u32 {
                    let c = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let b = Vec3::new((q[1] - c).perp_dot(q[2] - c), (q[2] - c).perp_dot(q[0] - c), (q[0] - c).perp_dot(q[1] - c)) / area;
                    if b.min_element() < 0.0 {
                        continue;
                    }
                    let dep = z[0] * b.x + z[1] * b.y + z[2] * b.z;
                    let i = (y * w + x) as usize;
                    if dep <= depth[i] {
                        continue;
                    }
                    let (rgb, a) = match tex {
                        Some(p) => {
                            let uv = t.uv[0] * b.x + t.uv[1] * b.y + t.uv[2] * b.z;
                            let tx = ((uv.x.rem_euclid(1.0) * p.w as f32) as u32).min(p.w - 1);
                            let ty = ((uv.y.rem_euclid(1.0) * p.h as f32) as u32).min(p.h - 1);
                            let o = ((ty * p.w + tx) * 4) as usize;
                            ([p.rgba[o], p.rgba[o + 1], p.rgba[o + 2]], p.rgba[o + 3])
                        }
                        None => ([120, 120, 120], if t.glass { 60 } else { 255 }),
                    };
                    if t.glass {
                        // see-through as its alpha says
                        let f = a as f32 / 255.0 * 0.7 + 0.15;
                        let px = img.get_pixel(x, y).0;
                        let mix = |k: usize| (px[k] as f32 * (1.0 - f) + rgb[k] as f32 * f) as u8;
                        img.put_pixel(x, y, image::Rgba([mix(0), mix(1), mix(2), 255]));
                        continue;
                    }
                    depth[i] = dep;
                    let n = (t.vn[0] * b.x + t.vn[1] * b.y + t.vn[2] * b.z).normalize_or(t.n);
                    let n = if n.dot(toward) < 0.0 { -n } else { n };
                    let l = 0.5 + 0.5 * n.dot(sun).max(0.0);
                    img.put_pixel(x, y, image::Rgba([(rgb[0] as f32 * l).min(255.0) as u8, (rgb[1] as f32 * l).min(255.0) as u8, (rgb[2] as f32 * l).min(255.0) as u8, 255]));
                }
            }
        }
    }
    img
}

/// The quick livery looked at: a blue base, a white skirt band, a name.
fn quick(d: &BusDims) -> Vec<Layer> {
    let q = model::Quick { colours: [Some("#2147b8".into()), Some("#ffffff".into()), None], stripe: StripeTemplate::Skirt, stripe_chosen: true, name: "Lucstad".into(), logo: None, logo_aspect: None, font: None };
    model::apply_quick(&[], &q, d, &|t, f| super::shapes::text_width(t, f))
}

fn paint_look(lk: &Look, layers: &[Layer]) -> Vec<Painted> {
    let mut cache = RasterCache::default();
    let pictures = Arc::new(HashMap::new());
    lk.bakes
        .iter()
        .zip(&lk.zones)
        .zip(&lk.templates)
        .map(|(((index, w, h, rgba, b), zones), tp)| {
            let cv = Canvas::with(b.clone(), rgba, zones).with_template(tp.as_ref().map(|[ma, mu]| paint::Template::rows(ma, mu, *w, 0, *h)));
            let cx = Context { pictures: pictures.clone(), mirror: Some(lk.geom.dims.middle_x()), density: cv.bake.density, max_px: 2048 };
            let ops = paint::prepare(layers, &cx, &mut cache);
            let (out, _) = paint::composite(&cv, &ops, &lk.geom.dims, None, None);
            Painted { index: *index, w: *w, h: *h, rgba: out }
        })
        .collect()
}

fn side_by_side(rows: &[[image::RgbaImage; 2]]) -> image::RgbaImage {
    let w = rows.iter().map(|r| r[0].width() + r[1].width() + 10).max().unwrap_or(1);
    let h = rows.iter().map(|r| r[0].height().max(r[1].height()) + 10).sum::<u32>();
    let mut out = image::RgbaImage::from_pixel(w, h, image::Rgba([40, 40, 40, 255]));
    let mut y = 0;
    for r in rows {
        image::imageops::overlay(&mut out, &r[0], 0, y as i64);
        image::imageops::overlay(&mut out, &r[1], (r[0].width() + 10) as i64, y as i64);
        y += r[0].height().max(r[1].height()) + 10;
    }
    out
}

/// Each bus with its own livery and with the quick livery, side by side, from the right, the
/// left and the front; and its first texture's paint mask. Run by hand:
///
/// `OMSI_LIVERY_ZONES=<OMSI folder>|<out folder>|<bus>[@<livery>];<bus>... cargo test -p omsi-app --lib livery::headless::buses -- --ignored --nocapture`
#[test]
#[ignore]
fn buses_before_and_after() {
    let Ok(spec) = std::env::var("OMSI_LIVERY_ZONES") else { return };
    let mut it = spec.split('|');
    let (root, out) = (PathBuf::from(it.next().unwrap()), PathBuf::from(it.next().unwrap()));
    std::fs::create_dir_all(&out).unwrap();
    for item in it.next().unwrap().split(';').filter(|s| !s.trim().is_empty()) {
        let (file, paint) = match item.split_once('@') {
            Some((f, p)) => (f.trim(), Some(p.trim())),
            None => (item.trim(), None),
        };
        println!("{file} {paint:?}");
        let lk = look(&root, file, paint);
        let before: Vec<Painted> = lk.bakes.iter().map(|(index, w, h, rgba, _)| Painted { index: *index, w: *w, h: *h, rgba: rgba.clone() }).collect();
        let after = paint_look(&lk, &quick(&lk.geom.dims));
        let stem = format!("{}{}", Path::new(file).file_stem().unwrap().to_string_lossy(), paint.map(|p| format!("_{p}")).unwrap_or_default()).replace(' ', "_");
        let k = 70.0;
        let rows = [
            [draw(&lk.geom, &before, View::Right, k), draw(&lk.geom, &after, View::Right, k)],
            [draw(&lk.geom, &before, View::Left, k), draw(&lk.geom, &after, View::Left, k)],
            [draw(&lk.geom, &before, View::Front, k * 1.6), draw(&lk.geom, &after, View::Front, k * 1.6)],
        ];
        side_by_side(&rows).save(out.join(format!("{stem}.png"))).unwrap();
        for p in &after {
            image::RgbaImage::from_raw(p.w, p.h, p.rgba.clone()).unwrap().save(out.join(format!("{stem}_after{}.png", p.index))).unwrap();
        }
        // the paint mask of the first texture (paint white, trims black; inside texels bluish)
        for ((index, w, h, rgba, b), zones) in lk.bakes.iter().zip(&lk.zones) {
            let cv = Canvas::with(b.clone(), rgba, zones);
            let mut m = image::RgbaImage::new(*w, *h);
            for (i, px) in m.pixels_mut().enumerate() {
                let z = (cv.paint_share(i) * 255.0) as u8;
                let outside = cv.bake.first[i].flags & bake::OUTSIDE != 0;
                *px = image::Rgba([if outside { z } else { z / 3 }, if outside { z } else { z / 3 }, z, 255]);
            }
            m.save(out.join(format!("{stem}_mask{index}.png"))).unwrap();
        }
    }
}
