//! What a decal looks like before it goes on the bus: texts set in a font, the built-in vector
//! shapes, the player's own shapes and uploaded pictures, rasterised with tiny-skia at the
//! texture's own density into masks (a colour is put in when it is painted) or colour pictures.

use super::model::PathNode;
use ab_glyph::{Font, FontVec, VariableFont};
use resvg::tiny_skia::{self, FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};
use std::sync::{Arc, Mutex, OnceLock};

/// A picture of one channel (coverage) or four (straight RGBA), row by row from the top.
#[derive(Clone, Debug, PartialEq)]
pub struct Raster {
    pub w: u32,
    pub h: u32,
    pub channels: u8,
    pub data: Vec<u8>,
}

impl Raster {
    pub fn mask(w: u32, h: u32) -> Raster {
        Raster { w, h, channels: 1, data: vec![0; (w * h) as usize] }
    }

    /// Bilinear sample at `(x, y)` (0..1 across and down): coverage, or RGBA with straight
    /// alpha (colour weighted by alpha so transparent edges do not darken it). Outside: none.
    pub fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
            return [0.0; 4];
        }
        let fx = x * self.w as f32 - 0.5;
        let fy = y * self.h as f32 - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (w, h) = (self.w as i32, self.h as i32);
        let at = |xi: i32, yi: i32| -> usize { (yi.clamp(0, h - 1) * w + xi.clamp(0, w - 1)) as usize };
        let (x0, y0) = (x0 as i32, y0 as i32);
        let wts = [(1.0 - tx) * (1.0 - ty), tx * (1.0 - ty), (1.0 - tx) * ty, tx * ty];
        let idx = [at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1)];
        if self.channels == 1 {
            let a: f32 = idx.iter().zip(wts).map(|(i, k)| self.data[*i] as f32 * k).sum::<f32>() / 255.0;
            return [1.0, 1.0, 1.0, a];
        }
        let mut c = [0.0f32; 4];
        for (i, k) in idx.iter().zip(wts) {
            let p = &self.data[i * 4..i * 4 + 4];
            let a = p[3] as f32 / 255.0 * k;
            c[0] += super::colour::to_linear(p[0]) * a;
            c[1] += super::colour::to_linear(p[1]) * a;
            c[2] += super::colour::to_linear(p[2]) * a;
            c[3] += a;
        }
        let a = c[3];
        if a > 1e-6 {
            for v in c.iter_mut().take(3) {
                *v /= a;
            }
        }
        c
    }
}

fn alpha_of(p: &Pixmap) -> Raster {
    Raster { w: p.width(), h: p.height(), channels: 1, data: p.pixels().iter().map(|c| c.alpha()).collect() }
}

fn white() -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 255, 255, 255);
    paint.anti_alias = true;
    paint
}

// --- fonts -----------------------------------------------------------------------------------

pub const DEFAULT_FONT: &str = "Hanken Grotesk";

/// The fonts offered: the two the launcher carries (SIL Open Font License) and the common ones
/// Windows has, each with its files, the bold one first.
const WINDOWS_FONTS: &[(&str, &[&str])] = &[
    ("Arial", &["arialbd.ttf", "arial.ttf"]),
    ("Arial Black", &["ariblk.ttf"]),
    ("Arial Narrow", &["ARIALNB.TTF", "ARIALN.TTF"]),
    ("Bahnschrift", &["bahnschrift.ttf"]),
    ("Calibri", &["calibrib.ttf", "calibri.ttf"]),
    ("Cambria", &["cambriab.ttf"]),
    ("Century Gothic", &["GOTHICB.TTF", "GOTHIC.TTF"]),
    ("Comic Sans MS", &["comicbd.ttf", "comic.ttf"]),
    ("Consolas", &["consolab.ttf", "consola.ttf"]),
    ("Courier New", &["courbd.ttf", "cour.ttf"]),
    ("Franklin Gothic", &["framd.ttf", "FRADM.TTF"]),
    ("Georgia", &["georgiab.ttf", "georgia.ttf"]),
    ("Gill Sans", &["GILB____.TTF", "GIL_____.TTF"]),
    ("Impact", &["impact.ttf"]),
    ("Lucida Sans", &["LSANSD.TTF", "LSANS.TTF"]),
    ("Palatino Linotype", &["palab.ttf", "pala.ttf"]),
    ("Rockwell", &["ROCKB.TTF", "ROCK.TTF"]),
    ("Segoe UI", &["segoeuib.ttf", "segoeui.ttf"]),
    ("Segoe UI Black", &["seguibl.ttf"]),
    ("Tahoma", &["tahomabd.ttf", "tahoma.ttf"]),
    ("Times New Roman", &["timesbd.ttf", "times.ttf"]),
    ("Trebuchet MS", &["trebucbd.ttf", "trebuc.ttf"]),
    ("Verdana", &["verdanab.ttf", "verdana.ttf"]),
];

fn fonts_dir() -> std::path::PathBuf {
    let windir = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
    windir.join("Fonts")
}

/// The fonts this computer has, by name (the launcher's own first).
pub fn font_names() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut v = vec![DEFAULT_FONT.to_string(), "Roboto".to_string()];
        let dir = fonts_dir();
        for (name, files) in WINDOWS_FONTS {
            if files.iter().any(|f| omsi_cfg::resolve_path(&dir, f).is_file()) {
                v.push(name.to_string());
            }
        }
        v
    })
}

/// A font and the fonts that fill in what it lacks (Hanken Grotesk's Latin Extended half).
pub struct FontSet(Vec<FontVec>);

impl FontSet {
    fn pick(&self, c: char) -> Option<(&FontVec, ab_glyph::GlyphId)> {
        self.0.iter().find_map(|f| {
            let g = f.glyph_id(c);
            (g.0 != 0).then_some((f, g))
        })
    }
}

/// The font called `name` (Hanken Grotesk when this computer has no such font).
pub fn font(name: &str) -> Arc<FontSet> {
    static CACHE: OnceLock<Mutex<std::collections::HashMap<String, Arc<FontSet>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(f) = cache.lock().unwrap().get(name) {
        return f.clone();
    }
    let hanken = || -> Vec<FontVec> {
        [include_bytes!("../../../../../assets/fonts/HankenGrotesk/HankenGrotesk-latin-700.ttf").as_slice(), include_bytes!("../../../../../assets/fonts/HankenGrotesk/HankenGrotesk-latin-ext-700.ttf").as_slice()]
            .iter()
            .filter_map(|b| FontVec::try_from_vec(b.to_vec()).ok())
            .collect()
    };
    let mut set: Vec<FontVec> = match name {
        "Roboto" => FontVec::try_from_vec(include_bytes!("../../../../../assets/fonts/Roboto-VariableFont_wdth,wght.ttf").to_vec())
            .ok()
            .map(|mut f| {
                f.set_variation(b"wght", 700.0);
                vec![f]
            })
            .unwrap_or_default(),
        n if n == DEFAULT_FONT => Vec::new(),
        n => {
            let dir = fonts_dir();
            WINDOWS_FONTS
                .iter()
                .find(|(f, _)| *f == n)
                .and_then(|(_, files)| files.iter().find_map(|f| std::fs::read(omsi_cfg::resolve_path(&dir, f)).ok()))
                .and_then(|b| FontVec::try_from_vec(b).ok())
                .into_iter()
                .collect()
        }
    };
    // (Hanken Grotesk behind every font: a letter the font lacks still comes out)
    set.extend(hanken());
    let f = Arc::new(FontSet(set));
    cache.lock().unwrap().insert(name.to_string(), f.clone());
    f
}

/// How a text is set: its width and the box it stands in, in letter heights (the em).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextMetrics {
    /// The letters' width, from the first one's start to the last one's end.
    pub width: f32,
    /// Where the baseline lies under the box's top (the box is `TEXT_BOX` high).
    pub baseline: f32,
}

fn glyph_run(fonts: &FontSet, text: &str, spacing: f32) -> (Vec<(f32, char)>, f32) {
    let mut x = 0.0;
    let mut out = Vec::new();
    let mut prev: Option<(usize, ab_glyph::GlyphId)> = None;
    for c in text.chars() {
        let Some((f, g)) = fonts.pick(c) else { continue };
        let upem = f.units_per_em().unwrap_or(1000.0);
        let fi = fonts.0.iter().position(|x| std::ptr::eq(x, f)).unwrap_or(0);
        if let Some((pf, pg)) = prev {
            if pf == fi {
                x += f.kern_unscaled(pg, g) / upem;
            }
            x += spacing;
        }
        out.push((x, c));
        x += f.h_advance_unscaled(g) / upem;
        prev = Some((fi, g));
    }
    (out, x)
}

/// A text's measures in `font` (`spacing`: percent of the letter height between letters).
pub fn text_metrics(text: &str, font_name: &str, spacing: f32) -> TextMetrics {
    let fonts = font(font_name);
    let (_, width) = glyph_run(&fonts, text, spacing / 100.0);
    let f = &fonts.0[0];
    let upem = f.units_per_em().unwrap_or(1000.0);
    let (asc, desc) = (f.ascent_unscaled() / upem, -f.descent_unscaled() / upem);
    // the letters' whole height in the middle of the box
    let baseline = (super::model::TEXT_BOX - (asc + desc)) * 0.5 + asc;
    TextMetrics { width: width.max(0.05), baseline }
}

/// The width of a text one metre high (what the quick livery measures with).
pub fn text_width(text: &str, font_name: &str) -> f32 {
    text_metrics(text, font_name, 0.0).width
}

/// A text rasterised into its box: `em_px` pixels per letter height, `outline_px` the outline's
/// width outside the letters (0: none). The box is the text's width plus the outline each side,
/// `TEXT_BOX` letter heights high. Returns the fill and the outline (the outline's mask
/// includes what lies under the letters).
pub fn text_raster(text: &str, font_name: &str, spacing: f32, em_px: f32, outline_px: f32) -> (Raster, Option<Raster>) {
    let fonts = font(font_name);
    let m = text_metrics(text, font_name, spacing);
    let pad = outline_px.max(0.0);
    let w = ((m.width * em_px + 2.0 * pad).ceil() as u32).clamp(1, 8192);
    let h = ((super::model::TEXT_BOX * em_px).ceil() as u32).clamp(1, 8192);
    let (run, _) = glyph_run(&fonts, text, spacing / 100.0);
    let mut pb = PathBuilder::new();
    for (x, c) in run {
        let Some((f, g)) = fonts.pick(c) else { continue };
        let Some(outline) = f.outline(g) else { continue };
        let k = em_px / f.units_per_em().unwrap_or(1000.0);
        let ox = pad + x * em_px;
        let oy = m.baseline * em_px;
        let tp = |p: ab_glyph::Point| (ox + p.x * k, oy - p.y * k);
        let mut last: Option<(f32, f32)> = None;
        for curve in &outline.curves {
            let (start, end) = match curve {
                ab_glyph::OutlineCurve::Line(a, b) => (*a, *b),
                ab_glyph::OutlineCurve::Quad(a, _, b) => (*a, *b),
                ab_glyph::OutlineCurve::Cubic(a, _, _, b) => (*a, *b),
            };
            let s = tp(start);
            if last.is_none_or(|l| (l.0 - s.0).abs() > 1e-3 || (l.1 - s.1).abs() > 1e-3) {
                if last.is_some() {
                    pb.close();
                }
                pb.move_to(s.0, s.1);
            }
            match curve {
                ab_glyph::OutlineCurve::Line(_, b) => {
                    let b = tp(*b);
                    pb.line_to(b.0, b.1);
                }
                ab_glyph::OutlineCurve::Quad(_, c, b) => {
                    let (c, b) = (tp(*c), tp(*b));
                    pb.quad_to(c.0, c.1, b.0, b.1);
                }
                ab_glyph::OutlineCurve::Cubic(_, c1, c2, b) => {
                    let (c1, c2, b) = (tp(*c1), tp(*c2), tp(*b));
                    pb.cubic_to(c1.0, c1.1, c2.0, c2.1, b.0, b.1);
                }
            }
            last = Some(tp(end));
        }
        if last.is_some() {
            pb.close();
        }
    }
    let Some(mut fill) = Pixmap::new(w, h) else { return (Raster::mask(1, 1), None) };
    let Some(path) = pb.finish() else { return (alpha_of(&fill), None) };
    fill.fill_path(&path, &white(), FillRule::Winding, Transform::identity(), None);
    let outline = (pad > 0.0).then(|| {
        let mut o = fill.clone();
        let stroke = Stroke { width: pad * 2.0, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
        o.stroke_path(&path, &white(), &stroke, Transform::identity(), None);
        alpha_of(&o)
    });
    (alpha_of(&fill), outline)
}

// --- the built-in shapes ---------------------------------------------------------------------

/// A piece of a shape in a box of 100 x 100: a path filled (even-odd where `evenodd`), or with
/// `stroke` a line that thick.
struct Piece {
    d: String,
    evenodd: bool,
    stroke: Option<f32>,
}

fn ring(x: f32, y: f32, r: f32) -> String {
    format!("M{} {} a{r} {r} 0 1 0 {} 0 a{r} {r} 0 1 0 {} 0 Z", x - r, y, 2.0 * r, -2.0 * r)
}

/// The shapes: Omsi-Hub's nine (`vormen.ts`) and the plain ones, by their key in the project
/// file, with their name in the interface.
pub const SHAPES: [(&str, &str); 15] = [
    ("rechthoek", "Rectangle"),
    ("afgerond", "Rounded rectangle"),
    ("ellips", "Ellipse"),
    ("driehoek", "Triangle"),
    ("chevron", "Chevron"),
    ("ster8", "Starburst"),
    ("pijl", "Arrow"),
    ("streep", "Bar"),
    ("cirkel", "Circle"),
    ("ster", "Star"),
    ("golf", "Wave"),
    ("rolstoel", "Wheelchair"),
    ("kinderwagen", "Pram"),
    ("fiets", "Bicycle"),
    ("kader", "Shield frame"),
];

fn pieces(key: &str) -> Vec<Piece> {
    let fill = |d: &str| Piece { d: d.to_string(), evenodd: false, stroke: None };
    let eo = |d: String| Piece { d, evenodd: true, stroke: None };
    let line = |d: &str, w: f32| Piece { d: d.to_string(), evenodd: false, stroke: Some(w) };
    match key {
        "rechthoek" => vec![fill("M0 0 H100 V100 H0 Z")],
        "afgerond" => vec![fill("M18 0 H82 Q100 0 100 18 V82 Q100 100 82 100 H18 Q0 100 0 82 V18 Q0 0 18 0 Z")],
        "ellips" => vec![fill("M0 50 A50 50 0 1 0 100 50 A50 50 0 1 0 0 50 Z")],
        "driehoek" => vec![fill("M50 2 L98 98 H2 Z")],
        "chevron" => vec![fill("M0 0 H45 L100 50 L45 100 H0 L55 50 Z")],
        "ster8" => {
            let mut d = String::new();
            for k in 0..16 {
                let r = if k % 2 == 0 { 50.0 } else { 30.0 };
                let a = std::f32::consts::TAU * k as f32 / 16.0 - std::f32::consts::FRAC_PI_2;
                d.push_str(&format!("{}{:.2} {:.2} ", if k == 0 { "M" } else { "L" }, 50.0 + r * a.cos(), 50.0 + r * a.sin()));
            }
            d.push('Z');
            vec![Piece { d, evenodd: false, stroke: None }]
        }
        "pijl" => vec![fill("M5 40 H65 V20 L95 50 L65 80 V60 H5 Z")],
        "streep" => vec![fill("M0 40 H100 V60 H0 Z")],
        "cirkel" => vec![Piece { d: ring(50.0, 50.0, 45.0), evenodd: false, stroke: None }],
        "ster" => vec![fill("M50 5 L61 38 H95 L67 58 L78 92 L50 72 L22 92 L33 58 L5 38 H39 Z")],
        "golf" => vec![fill("M0 45 Q25 15 50 45 T100 45 V65 Q75 95 50 65 T0 65 Z")],
        "rolstoel" => vec![
            Piece { d: ring(42.0, 12.0, 8.0), evenodd: false, stroke: None },
            line("M38 26 L40 54 H66 L76 80", 9.0),
            line("M40 38 H62", 7.0),
            line("M30 50 A26 26 0 1 0 70 76", 7.0),
        ],
        "kinderwagen" => vec![
            fill("M16 52 A34 34 0 0 1 50 18 V52 Z"),
            fill("M16 56 H84 Q82 78 50 78 Q18 78 16 56 Z"),
            line("M84 56 L90 26 H97", 5.0),
            eo(format!("{} {}", ring(32.0, 89.0, 8.0), ring(32.0, 89.0, 3.5))),
            eo(format!("{} {}", ring(68.0, 89.0, 8.0), ring(68.0, 89.0, 3.5))),
        ],
        "fiets" => vec![
            line("M40 70 A17 17 0 1 0 6 70 A17 17 0 1 0 40 70", 5.0),
            line("M94 70 A17 17 0 1 0 60 70 A17 17 0 1 0 94 70", 5.0),
            line("M23 70 L38 44 H68 L77 70 M38 44 L50 70 H23 M68 44 L63 32 H72 M38 44 L35 36 M29 36 H43", 5.0),
        ],
        "kader" => vec![eo("M10 6 H90 V48 Q90 82 50 97 Q10 82 10 48 Z M18 14 V48 Q18 76 50 88 Q82 76 82 48 V14 Z".into())],
        _ => vec![Piece { d: ring(50.0, 50.0, 45.0), evenodd: false, stroke: None }],
    }
}

/// An SVG path's outline (through usvg, which reads every command of the path syntax).
fn parse_d(d: &str) -> Option<tiny_skia::Path> {
    let doc = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100"><path d="{d}"/></svg>"#);
    let tree = resvg::usvg::Tree::from_str(&doc, &resvg::usvg::Options::default()).ok()?;
    fn first(g: &resvg::usvg::Group) -> Option<tiny_skia::Path> {
        for n in g.children() {
            match n {
                resvg::usvg::Node::Path(p) => return p.data().clone().transform(p.abs_transform()),
                resvg::usvg::Node::Group(g) => {
                    if let Some(p) = first(g) {
                        return Some(p);
                    }
                }
                _ => {}
            }
        }
        None
    }
    first(tree.root())
}

/// Paths of a shape, made once.
fn shape_paths(key: &str) -> Arc<Vec<(tiny_skia::Path, bool, Option<f32>)>> {
    static CACHE: OnceLock<Mutex<std::collections::HashMap<String, Arc<Vec<(tiny_skia::Path, bool, Option<f32>)>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(p) = cache.lock().unwrap().get(key) {
        return p.clone();
    }
    let v: Vec<_> = pieces(key).into_iter().filter_map(|p| Some((parse_d(&p.d)?, p.evenodd, p.stroke))).collect();
    let v = Arc::new(v);
    cache.lock().unwrap().insert(key.to_string(), v.clone());
    v
}

/// A shape stretched over `w` x `h` pixels, with an outline `outline_px` wide round it (0: none;
/// the box then has room for it).
pub fn shape_raster(key: &str, w: u32, h: u32, outline_px: f32) -> (Raster, Option<Raster>) {
    let pad = outline_px.max(0.0);
    let (bw, bh) = (w.max(1), h.max(1));
    let Some(mut fill) = Pixmap::new(bw, bh) else { return (Raster::mask(1, 1), None) };
    let (iw, ih) = ((bw as f32 - 2.0 * pad).max(1.0), (bh as f32 - 2.0 * pad).max(1.0));
    let ts = Transform::from_row(iw / 100.0, 0.0, 0.0, ih / 100.0, pad, pad);
    let k = (iw * ih).sqrt() / 100.0;
    let paths = shape_paths(key);
    let mut outline = (pad > 0.0).then(|| Pixmap::new(bw, bh)).flatten();
    for (path, evenodd, stroke) in paths.iter() {
        let Some(p) = path.clone().transform(ts) else { continue };
        match stroke {
            Some(sw) => {
                let s = Stroke { width: sw * k, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
                fill.stroke_path(&p, &white(), &s, Transform::identity(), None);
                if let Some(o) = outline.as_mut() {
                    let s = Stroke { width: sw * k + 2.0 * pad, ..s };
                    o.stroke_path(&p, &white(), &s, Transform::identity(), None);
                }
            }
            None => {
                let rule = if *evenodd { FillRule::EvenOdd } else { FillRule::Winding };
                fill.fill_path(&p, &white(), rule, Transform::identity(), None);
                if let Some(o) = outline.as_mut() {
                    o.fill_path(&p, &white(), rule, Transform::identity(), None);
                    let s = Stroke { width: 2.0 * pad, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
                    o.stroke_path(&p, &white(), &s, Transform::identity(), None);
                }
            }
        }
    }
    (alpha_of(&fill), outline.map(|o| alpha_of(&o)))
}

// --- the player's own shapes -----------------------------------------------------------------

/// The pen's path in a box of `w` x `h` (its nodes are 0..1 of it), as tiny-skia draws it.
pub fn own_path(nodes: &[PathNode], w: f32, h: f32, pad: f32) -> Option<tiny_skia::Path> {
    if nodes.len() < 2 {
        return None;
    }
    let (iw, ih) = ((w - 2.0 * pad).max(1.0), (h - 2.0 * pad).max(1.0));
    let tp = |p: [f32; 2]| (pad + p[0] * iw, pad + p[1] * ih);
    let mut pb = PathBuilder::new();
    let s = tp(nodes[0].p);
    pb.move_to(s.0, s.1);
    let n = nodes.len();
    for i in 0..n {
        let (a, b) = (&nodes[i], &nodes[(i + 1) % n]);
        if a.c2.is_some() || b.c1.is_some() {
            let (c1, c2, e) = (tp(a.c2.unwrap_or(a.p)), tp(b.c1.unwrap_or(b.p)), tp(b.p));
            pb.cubic_to(c1.0, c1.1, c2.0, c2.1, e.0, e.1);
        } else {
            let e = tp(b.p);
            pb.line_to(e.0, e.1);
        }
    }
    pb.close();
    pb.finish()
}

/// The player's shape over `w` x `h` pixels, filled even-odd, with an outline `outline_px` wide.
pub fn path_raster(nodes: &[PathNode], w: u32, h: u32, outline_px: f32) -> (Raster, Option<Raster>) {
    let pad = outline_px.max(0.0);
    let Some(mut fill) = Pixmap::new(w.max(1), h.max(1)) else { return (Raster::mask(1, 1), None) };
    let Some(p) = own_path(nodes, w as f32, h as f32, pad) else { return (alpha_of(&fill), None) };
    fill.fill_path(&p, &white(), FillRule::EvenOdd, Transform::identity(), None);
    let outline = (pad > 0.0).then(|| {
        let mut o = fill.clone();
        let s = Stroke { width: 2.0 * pad, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
        o.stroke_path(&p, &white(), &s, Transform::identity(), None);
        alpha_of(&o)
    });
    (alpha_of(&fill), outline)
}

/// The nodes drawn on the bus (metres in the decal's plane, y up) fitted into their box: the box's
/// middle and size (metres), and the nodes 0..1 in it (y down).
pub fn fit_nodes(points: &[(glam::Vec2, Option<glam::Vec2>, Option<glam::Vec2>)]) -> Option<(glam::Vec2, glam::Vec2, Vec<PathNode>)> {
    if points.len() < 3 {
        return None;
    }
    let mut lo = glam::Vec2::splat(f32::INFINITY);
    let mut hi = glam::Vec2::splat(f32::NEG_INFINITY);
    for (p, a, b) in points {
        for q in std::iter::once(*p).chain(*a).chain(*b) {
            lo = lo.min(q);
            hi = hi.max(q);
        }
    }
    let size = (hi - lo).max(glam::Vec2::splat(0.01));
    let to = |q: glam::Vec2| [(q.x - lo.x) / size.x, (hi.y - q.y) / size.y];
    let nodes = points.iter().map(|(p, a, b)| PathNode { p: to(*p), c1: a.map(to), c2: b.map(to) }).collect();
    Some(((lo + hi) * 0.5, size, nodes))
}

// --- pictures --------------------------------------------------------------------------------

/// The longest side a picture is kept at.
pub const IMAGE_MAX: u32 = 4096;

/// A picture from its file's bytes: PNG, JPG, BMP, TGA, DDS or SVG (drawn at 1024 to 4096 on
/// its long side), at most `IMAGE_MAX` on its long side. Straight RGBA.
pub fn decode_image(bytes: &[u8], name: &str) -> Result<Raster, String> {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).trim_start().to_ascii_lowercase();
    let svg = name.to_ascii_lowercase().ends_with(".svg") || head.starts_with("<?xml") || head.starts_with("<svg");
    let (w, h, rgba) = if svg {
        let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).map_err(|e| e.to_string())?;
        let s = tree.size();
        let long = s.width().max(s.height()).max(1.0);
        let k = (IMAGE_MAX as f32).min(long.max(1024.0)) / long;
        let (w, h) = (((s.width() * k).round() as u32).max(1), ((s.height() * k).round() as u32).max(1));
        let mut pix = Pixmap::new(w, h).ok_or("empty picture")?;
        resvg::render(&tree, Transform::from_scale(k, k), &mut pix.as_mut());
        let rgba = pix.pixels().iter().flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        });
        (w, h, rgba.collect())
    } else {
        let img = omsi_texture::decode_bytes(bytes, std::path::Path::new(name)).map_err(|e| e.to_string())?;
        (img.width, img.height, img.rgba)
    };
    let long = w.max(h);
    if long > IMAGE_MAX {
        let (nw, nh) = (((w as u64 * IMAGE_MAX as u64) / long as u64).max(1) as u32, ((h as u64 * IMAGE_MAX as u64) / long as u64).max(1) as u32);
        return Ok(Raster { w: nw, h: nh, channels: 4, data: omsi_texture::bc::resize(&rgba, w, h, nw, nh) });
    }
    Ok(Raster { w, h, channels: 4, data: rgba })
}

fn near_white(p: &[u8]) -> bool {
    p[0] >= 230 && p[1] >= 230 && p[2] >= 230
}

/// Whether "white becomes clear" is wanted at once: nothing see-through in the picture, and at
/// least nine in ten of its edge's pixels nearly white (a logo on a white sheet).
pub fn wants_white_clear(img: &Raster) -> bool {
    if img.channels != 4 || img.data.chunks(4).any(|p| p[3] < 250) {
        return false;
    }
    let (w, h) = (img.w as usize, img.h as usize);
    let px = |x: usize, y: usize| &img.data[(y * w + x) * 4..(y * w + x) * 4 + 4];
    let mut n = 0;
    let mut white = 0;
    for x in 0..w {
        for y in [0, h - 1] {
            n += 1;
            white += near_white(px(x, y)) as usize;
        }
    }
    for y in 0..h {
        for x in [0, w - 1] {
            n += 1;
            white += near_white(px(x, y)) as usize;
        }
    }
    white * 10 >= n * 9
}

/// The white round a picture made clear (Omsi-Hub's `witWeg`): the nearly white pixels that
/// reach the edge go, and the pixels along them lose their white as alpha (colour to alpha), so
/// that no white seam is left round the logo.
pub fn white_clear(img: &Raster) -> Raster {
    if img.channels != 4 {
        return img.clone();
    }
    let (w, h) = (img.w as usize, img.h as usize);
    let mut out = img.data.clone();
    let mut gone = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    for x in 0..w {
        stack.push(x);
        stack.push((h - 1) * w + x);
    }
    for y in 0..h {
        stack.push(y * w);
        stack.push(y * w + w - 1);
    }
    while let Some(i) = stack.pop() {
        if gone[i] || !near_white(&img.data[i * 4..i * 4 + 4]) {
            continue;
        }
        gone[i] = true;
        let (x, y) = (i % w, i / w);
        if x > 0 { stack.push(i - 1); }
        if x + 1 < w { stack.push(i + 1); }
        if y > 0 { stack.push(i - w); }
        if y + 1 < h { stack.push(i + w); }
    }
    // the fringe: two pixels along what went
    let mut fringe = vec![false; w * h];
    for i in 0..w * h {
        if gone[i] {
            continue;
        }
        let (x, y) = ((i % w) as i32, (i / w) as i32);
        'n: for dy in -2..=2i32 {
            for dx in -2..=2i32 {
                let (nx, ny) = (x + dx, y + dy);
                if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h && gone[ny as usize * w + nx as usize] {
                    fringe[i] = true;
                    break 'n;
                }
            }
        }
    }
    for i in 0..w * h {
        let p = &mut out[i * 4..i * 4 + 4];
        if gone[i] {
            p[3] = 0;
        } else if fringe[i] {
            // colour to alpha against white
            let a = (0..3).map(|k| 1.0 - p[k] as f32 / 255.0).fold(0.0f32, f32::max);
            if a < 1.0 {
                for k in 0..3 {
                    let c = p[k] as f32 / 255.0;
                    p[k] = if a > 1e-3 { (((c - (1.0 - a)) / a).clamp(0.0, 1.0) * 255.0 + 0.5) as u8 } else { 255 };
                }
                p[3] = (p[3] as f32 * a + 0.5) as u8;
            }
        }
    }
    Raster { w: img.w, h: img.h, channels: 4, data: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covered(r: &Raster) -> f32 {
        r.data.iter().map(|a| *a as f32 / 255.0).sum::<f32>() / (r.w * r.h) as f32
    }

    #[test]
    fn every_shape_draws_and_keeps_its_holes() {
        for (key, _) in SHAPES {
            let (m, o) = shape_raster(key, 100, 100, 0.0);
            let c = covered(&m);
            assert!(c > 0.04 && c < 1.01, "{key}: {c}");
            assert!(o.is_none());
        }
        let (rect, _) = shape_raster("rechthoek", 64, 32, 0.0);
        assert!(covered(&rect) > 0.98);
        // the ring is open in its middle, the shield frame too
        let (ring, _) = shape_raster("cirkel", 100, 100, 0.0);
        assert!(ring.data[50 * 100 + 50] > 200, "Omsi-Hub's 'cirkel' is a disc");
        let (frame, _) = shape_raster("kader", 100, 100, 0.0);
        assert_eq!(frame.data[50 * 100 + 50], 0, "the frame's inside is clear");
        let (e, _) = shape_raster("ellips", 100, 50, 0.0);
        assert_eq!(e.data[0], 0);
        assert!(e.data[25 * 100 + 50] == 255);
        // an outline lies round it
        let (f, o) = shape_raster("rechthoek", 100, 100, 10.0);
        let o = o.unwrap();
        assert_eq!(f.data[5 * 100 + 50], 0);
        assert!(o.data[5 * 100 + 50] > 200 && o.data[50 * 100 + 50] == 255);
    }

    #[test]
    fn a_text_is_set_in_its_box() {
        let m = text_metrics("Lucstad", DEFAULT_FONT, 0.0);
        assert!(m.width > 2.0 && m.width < 5.0, "{m:?}");
        assert!(m.baseline > 0.8 && m.baseline < super::super::model::TEXT_BOX, "{m:?}");
        let wider = text_metrics("Lucstad", DEFAULT_FONT, 20.0);
        assert!((wider.width - m.width - 6.0 * 0.2).abs() < 1e-3, "spacing between six pairs");
        let (fill, outline) = text_raster("Lucstad", DEFAULT_FONT, 0.0, 40.0, 3.0);
        assert_eq!(fill.h, (40.0 * super::super::model::TEXT_BOX).ceil() as u32);
        assert!((fill.w as f32 - (m.width * 40.0 + 6.0)).abs() <= 1.0);
        let c = covered(&fill);
        assert!(c > 0.1 && c < 0.6, "{c}");
        assert!(covered(&outline.unwrap()) > c * 1.2);
        // a letter outside Latin-1 comes from the extended half
        let (pl, _) = text_raster("Ł", DEFAULT_FONT, 0.0, 40.0, 0.0);
        assert!(covered(&pl) > 0.05);
        assert!(text_width("", DEFAULT_FONT) > 0.0);
        assert!(font_names().iter().any(|n| n == "Roboto"));
        let (r, _) = text_raster("A", "Roboto", 0.0, 40.0, 0.0);
        assert!(covered(&r) > 0.05);
    }

    #[test]
    fn own_shapes_are_filled_even_odd() {
        // a square with a square hole: two runs round, drawn as one path
        let sq = |a: f32, b: f32| [[a, a], [b, a], [b, b], [a, b]].map(|p| PathNode { p, c1: None, c2: None });
        let nodes: Vec<PathNode> = sq(0.0, 1.0).to_vec();
        let (m, _) = path_raster(&nodes, 50, 50, 0.0);
        assert!(covered(&m) > 0.95);
        // a bow tie: crossing itself, both halves filled
        let tie = [[0.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 1.0]].map(|p| PathNode { p, c1: None, c2: None });
        let (m, _) = path_raster(&tie, 50, 50, 0.0);
        assert!(m.data[25 * 50 + 5] > 200 && m.data[5 * 50 + 25] == 0);
        // a curve bulges out
        let mut bulge = sq(0.2, 0.8).to_vec();
        bulge[0].c2 = Some([0.5, -0.2]);
        let (m, _) = path_raster(&bulge, 50, 50, 0.0);
        assert!(m.data[3 * 50 + 25] > 100, "the curve reaches up");
        let (c, size, nodes) = fit_nodes(&[(glam::Vec2::new(1.0, 1.0), None, None), (glam::Vec2::new(3.0, 1.0), None, None), (glam::Vec2::new(3.0, 2.0), None, None)]).unwrap();
        assert_eq!((c, size), (glam::Vec2::new(2.0, 1.5), glam::Vec2::new(2.0, 1.0)));
        assert_eq!(nodes[0].p, [0.0, 1.0]);
        assert_eq!(nodes[2].p, [1.0, 0.0]);
    }

    #[test]
    fn white_round_a_logo_goes() {
        // a red disc on white
        let (w, h) = (40u32, 40u32);
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let d = ((x as f32 - 20.0).powi(2) + (y as f32 - 20.0).powi(2)).sqrt();
                data.extend_from_slice(if d < 10.0 { &[200, 20, 20, 255] } else { &[255, 255, 255, 255] });
            }
        }
        let img = Raster { w, h, channels: 4, data };
        assert!(wants_white_clear(&img));
        let clear = white_clear(&img);
        assert_eq!(clear.data[3], 0);
        assert_eq!(&clear.data[(20 * 40 + 20) * 4..(20 * 40 + 20) * 4 + 4], &[200, 20, 20, 255]);
        let s = clear.sample(0.5, 0.5);
        assert!(s[3] > 0.99 && s[0] > 0.5);
        assert_eq!(clear.sample(0.02, 0.02)[3], 0.0);
        // a photo with no white edge is left alone
        let photo = Raster { w: 4, h: 4, channels: 4, data: [10u8, 90, 30, 255].repeat(16) };
        assert!(!wants_white_clear(&photo));
    }

    #[test]
    fn pictures_are_read_from_their_files() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(8, 4, image::Rgba([1, 2, 3, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let r = decode_image(&png, "logo.png").unwrap();
        assert_eq!((r.w, r.h, r.channels), (8, 4, 4));
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="10" height="10" fill="#ff0000"/></svg>"##;
        let r = decode_image(svg, "logo.svg").unwrap();
        assert_eq!((r.w, r.h), (1024, 512));
        assert_eq!(&r.data[..4], &[255, 0, 0, 255]);
        assert_eq!(r.data[(1023) * 4 + 3], 0);
        assert!(decode_image(b"nonsense", "x.png").is_err());
    }
}
