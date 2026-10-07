//! Roboto, the interface font (Apache 2.0), in several weights of its variable font - and,
//! for the launcher, Hanken Grotesk (SIL OFL 1.1) before it, with Roboto for what it has not.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, VariableFont};

const ROBOTO: &[u8] = include_bytes!("../../../assets/fonts/Roboto-VariableFont_wdth,wght.ttf");

/// Hanken Grotesk per weight: the Latin alphabet, then the rest of the Latin scripts (Polish,
/// Czech, Turkish ...). Cyrillic, Greek and the others come from Roboto after it.
const HANKEN: [(Weight, [&[u8]; 2]); 4] = [
    (Weight::Regular, [include_bytes!("../../../assets/fonts/HankenGrotesk/HankenGrotesk-latin-400.ttf"), include_bytes!("../../../assets/fonts/HankenGrotesk/HankenGrotesk-latin-ext-400.ttf")]),
    (Weight::Medium, [include_bytes!("../../../assets/fonts/HankenGrotesk/HankenGrotesk-latin-500.ttf"), include_bytes!("../../../assets/fonts/HankenGrotesk/HankenGrotesk-latin-ext-500.ttf")]),
    (Weight::Bold, [include_bytes!("../../../assets/fonts/HankenGrotesk/HankenGrotesk-latin-700.ttf"), include_bytes!("../../../assets/fonts/HankenGrotesk/HankenGrotesk-latin-ext-700.ttf")]),
    (Weight::Black, [include_bytes!("../../../assets/fonts/HankenGrotesk/HankenGrotesk-latin-800.ttf"), include_bytes!("../../../assets/fonts/HankenGrotesk/HankenGrotesk-latin-ext-800.ttf")]),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Weight {
    Regular,
    Medium,
    Bold,
    Black,
    /// Bold and narrow (Roboto's `wdth` 75): figures in a tight space.
    Condensed,
}

impl Weight {
    const ALL: [Weight; 5] = [Weight::Regular, Weight::Medium, Weight::Bold, Weight::Black, Weight::Condensed];
    fn axes(self) -> (f32, f32) {
        match self {
            Weight::Regular => (400.0, 100.0),
            Weight::Medium => (500.0, 100.0),
            Weight::Bold => (700.0, 100.0),
            Weight::Black => (900.0, 100.0),
            Weight::Condensed => (700.0, 75.0),
        }
    }
}

/// A line of text as coverage: `w` x `h` alpha values; the baseline is `ascent` pixels
/// below the top.
#[derive(Debug, Clone)]
pub struct Bitmap {
    pub w: u32,
    pub h: u32,
    pub alpha: Vec<u8>,
    pub ascent: f32,
}

/// A character Roboto has in place of one it has not (a box would show).
fn substitute(c: char) -> char {
    match c {
        '→' | '▸' | '➜' | '⟶' | '►' | '▶' => '›',
        '←' | '◂' | '◀' => '‹',
        '★' | '☆' => '•',
        '⚠' => '!',
        '✓' | '✔' => '•',
        '✕' | '✖' => '×',
        c => c,
    }
}

/// Letters written as a base and a combining mark (Unicode's decomposed form, which is how
/// macOS stores file names: "Eiseska\u{308}lte.owt") as the one letter Roboto draws - the
/// mark alone was a box after a plain "a" in the launcher's weather list.
pub fn composed(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.chars().any(|c| ('\u{300}'..='\u{36f}').contains(&c)) {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if ('\u{300}'..='\u{36f}').contains(&c) {
            if let Some(base) = out.pop() {
                match compose(base, c) {
                    Some(k) => out.push(k),
                    None => out.push(base),
                }
            }
            continue;
        }
        out.push(c);
    }
    std::borrow::Cow::Owned(out)
}

/// The precomposed letter for `base` + combining `mark` (the Latin letters of the maps'
/// countries: German, French, Polish, Czech, Hungarian, Serbian/Croatian latin).
fn compose(base: char, mark: char) -> Option<char> {
    let table: &[(char, &str, &str)] = &[
        ('\u{308}', "aeiouyAEIOUY", "äëïöüÿÄËÏÖÜŸ"),
        ('\u{301}', "aeiouyAEIOUYcnszlrCNSZLR", "áéíóúýÁÉÍÓÚÝćńśźĺŕĆŃŚŹĹŔ"),
        ('\u{300}', "aeiouAEIOU", "àèìòùÀÈÌÒÙ"),
        ('\u{302}', "aeiouAEIOU", "âêîôûÂÊÎÔÛ"),
        ('\u{303}', "anoANO", "ãñõÃÑÕ"),
        ('\u{30c}', "cdenrstzCDENRSTZ", "čďěňřšťžČĎĚŇŘŠŤŽ"),
        ('\u{30a}', "auAU", "åůÅŮ"),
        ('\u{327}', "cstCST", "çşţÇŞŢ"),
        ('\u{328}', "aeAE", "ąęĄĘ"),
        ('\u{307}', "zZ", "żŻ"),
        ('\u{30b}', "ouOU", "őűŐŰ"),
    ];
    let (_, from, to) = table.iter().find(|(m, _, _)| *m == mark)?;
    let k = from.chars().position(|c| c == base)?;
    to.chars().nth(k)
}

/// Padding around a rendered line (pixels), so that linear filtering does not bleed.
pub const PAD: u32 = 1;

/// A line drawn otherwise than upright and as its face has it: stretched - every letter
/// `stretch` wider (0.1: a tenth), its advance with it - emboldened - every outline pushed out
/// by `bold` of the size (pulled in when it is below 0), each letter's advance as much wider -
/// and slanted - an oblique, for a face without an italic: every point moved right by `slant`
/// times its height over the baseline. All is done to the outlines before they are rasterised,
/// so the line is as crisp as any other (a sheared picture of the upright line went soft).
/// Boldening closes a letter's narrow openings as it fills them: a few hundredths at most (an
/// S of the launcher's heaviest face kept its shape at 0.03, not at 0.05).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Style {
    pub slant: f32,
    pub bold: f32,
    pub stretch: f32,
}

/// How far below the baseline a descender may reach (of the size): what a slant pushes left.
const DESCENT_SHARE: f32 = 0.3;

impl Style {
    /// How much further left than an upright line's the picture of a line in this style
    /// starts, in pixels at `px`: its pen stands `PAD` + this from the picture's left.
    pub fn lead(&self, px: f32) -> f32 {
        (self.slant.abs() * DESCENT_SHARE * px).ceil()
    }

    /// What each letter's advance grows by besides its stretch (pixels at `px`): its outline,
    /// pushed out on both sides, keeps its distance to the next.
    pub fn widen(&self, px: f32) -> f32 {
        2.0 * self.bold * px
    }
}

/// The outlines of a glyph pushed out by `e` on every side, as FreeType emboldens: each point
/// moves along the bisector of the normals of its two edges, so that a straight edge moves out
/// by `e` exactly (a corner's point by the mitre), held in where an edge is too short for that;
/// then everything moves right and up by `e`, so the glyph still stands on the baseline and
/// starts where it did. In pixels, y up.
fn embolden(curves: &[ab_glyph::OutlineCurve], e: f32) -> Vec<ab_glyph::OutlineCurve> {
    use ab_glyph::OutlineCurve as C;
    let v = |p: &ab_glyph::Point| glam::Vec2::new(p.x, p.y);
    let ends = |c: &C| match c {
        C::Line(a, b) | C::Quad(a, _, b) | C::Cubic(a, _, _, b) => (v(a), v(b)),
    };
    // the contours: each a run of curves, end to end, back to where it began
    let mut contours: Vec<Vec<&C>> = Vec::new();
    for c in curves {
        let (a, b) = ends(c);
        if a == b && matches!(c, C::Line(..)) {
            continue;
        }
        // (on from the last contour while that has not come back to where it began)
        match contours.last_mut() {
            Some(k) if ends(k[k.len() - 1]).1 == a && ends(k[0]).0 != a => k.push(c),
            _ => contours.push(vec![c]),
        }
    }
    // each contour's points in order, its control points with them (each curve takes its
    // own from there, in turn)
    let points: Vec<Vec<glam::Vec2>> = contours
        .iter()
        .map(|k| {
            k.iter()
                .flat_map(|c| match c {
                    C::Line(a, _) => vec![v(a)],
                    C::Quad(a, b, _) => vec![v(a), v(b)],
                    C::Cubic(a, b, c, _) => vec![v(a), v(b), v(c)],
                })
                .collect()
        })
        .collect();
    // TrueType's outer contours run clockwise (y up), PostScript's against it: which side of
    // the way along an edge is outside the ink, from the area they all enclose together
    let area: f32 = points.iter().map(|p| (0..p.len()).map(|i| p[i].perp_dot(p[(i + 1) % p.len()])).sum::<f32>()).sum();
    let clockwise = area < 0.0;
    let moved: Vec<Vec<glam::Vec2>> = points
        .iter()
        .map(|p| {
            let n = p.len();
            (0..n)
                .map(|i| {
                    // the nearest other points before and after (a point may stand twice)
                    let prev = (1..n).map(|k| p[(i + n - k) % n]).find(|q| q.distance_squared(p[i]) > 1e-8);
                    let next = (1..n).map(|k| p[(i + k) % n]).find(|q| q.distance_squared(p[i]) > 1e-8);
                    let (Some(prev), Some(next)) = (prev, next) else { return p[i] + glam::Vec2::splat(e) };
                    let (l_in, l_out) = (p[i].distance(prev), next.distance(p[i]));
                    let (din, dout) = ((p[i] - prev) / l_in, (next - p[i]) / l_out);
                    let d = din.dot(dout);
                    let mut shift = glam::Vec2::ZERO;
                    // (a spike turning back on itself is left as it is)
                    if d > -0.9375 {
                        let d = d + 1.0;
                        let (mut q, sum) = (dout.x * din.y - dout.y * din.x, din + dout);
                        shift = if clockwise { glam::Vec2::new(-sum.y, sum.x) } else { glam::Vec2::new(sum.y, -sum.x) };
                        if clockwise {
                            q = -q;
                        }
                        let l = l_in.min(l_out);
                        shift *= if e * q <= l * d { e / d } else { l / q };
                    }
                    p[i] + shift + glam::Vec2::splat(e)
                })
                .collect()
        })
        .collect();
    let pt = |q: glam::Vec2| ab_glyph::point(q.x, q.y);
    let mut out = Vec::with_capacity(curves.len());
    for (k, m) in contours.iter().zip(&moved) {
        let mut i = 0;
        for c in k {
            let at = |j: usize| pt(m[j % m.len()]);
            match c {
                C::Line(..) => {
                    out.push(C::Line(at(i), at(i + 1)));
                    i += 1;
                }
                C::Quad(..) => {
                    out.push(C::Quad(at(i), at(i + 1), at(i + 2)));
                    i += 2;
                }
                C::Cubic(..) => {
                    out.push(C::Cubic(at(i), at(i + 1), at(i + 2), at(i + 3)));
                    i += 3;
                }
            }
        }
    }
    out
}

pub struct Fonts {
    /// Per weight the fonts a character is looked for in, in order: Roboto alone, or a
    /// typeface before it (`Fonts::hanken`). The first one's metrics are the line's.
    faces: Vec<(Weight, Vec<FontVec>)>,
}

/// The system's fonts for the scripts Roboto has not (Chinese, Japanese, Korean, the
/// Devanagari of Hindi, Arabic, Thai ...): read the first time such a character is drawn,
/// from where each system keeps them - nothing is shipped, and nothing is read for a
/// language Roboto covers. Each carries the interface language it is best for: Chinese,
/// Japanese and Korean share thousands of characters and each font draws them its own
/// country's way, so the interface's own language picks first.
fn fallback_fonts() -> &'static [(&'static str, FontVec)] {
    static FALLBACK: std::sync::OnceLock<Vec<(&'static str, FontVec)>> = std::sync::OnceLock::new();
    FALLBACK.get_or_init(|| {
        const PATHS: &[(&str, &str)] = &[
            // Windows
            ("ja", "C:\\Windows\\Fonts\\YuGothM.ttc"),
            ("zh", "C:\\Windows\\Fonts\\msyh.ttc"),
            ("zh-tw", "C:\\Windows\\Fonts\\msjh.ttc"),
            ("ja", "C:\\Windows\\Fonts\\meiryo.ttc"),
            ("ko", "C:\\Windows\\Fonts\\malgun.ttf"),
            // (Thai from fonts whose marks sit right without a shaper: Tahoma's do, those of
            // Leelawadee and macOS's Thonburi wait for one - Thonburi even draws them on
            // dotted circles)
            ("th", "C:\\Windows\\Fonts\\tahoma.ttf"),
            ("hi", "C:\\Windows\\Fonts\\Nirmala.ttf"),
            ("hi", "C:\\Windows\\Fonts\\NirmalaUI.ttf"),
            // (Windows 11 has Nirmala UI as a collection only)
            ("hi", "C:\\Windows\\Fonts\\Nirmala.ttc"),
            ("", "C:\\Windows\\Fonts\\segoeui.ttf"),
            // macOS
            ("zh", "/System/Library/Fonts/Hiragino Sans GB.ttc"),
            ("ja", "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc"),
            ("zh-tw", "/System/Library/Fonts/STHeiti Light.ttc"),
            ("ko", "/System/Library/Fonts/AppleSDGothicNeo.ttc"),
            ("th", "/System/Library/Fonts/Supplemental/SukhumvitSet.ttc"),
            ("th", "/System/Library/Fonts/Supplemental/Silom.ttf"),
            ("hi", "/System/Library/Fonts/Kohinoor.ttc"),
            ("hi", "/System/Library/Fonts/Supplemental/Devanagari Sangam MN.ttc"),
            ("", "/System/Library/Fonts/Supplemental/Arial Unicode.ttf"),
            ("", "/System/Library/Fonts/GeezaPro.ttc"),
            // Android
            ("", "/system/fonts/NotoSansCJK-Regular.ttc"),
            ("", "/system/fonts/NotoSerifCJK-Regular.ttc"),
            ("th", "/system/fonts/NotoSansThai-Regular.ttf"),
            ("hi", "/system/fonts/NotoSansDevanagari-Regular.otf"),
            ("hi", "/system/fonts/NotoSansDevanagariUI-VF.ttf"),
            ("hi", "/system/fonts/NotoSansDevanagari-VF.ttf"),
            ("", "/system/fonts/NotoNaskhArabic-Regular.ttf"),
            ("", "/system/fonts/DroidSansFallback.ttf"),
            // Linux
            ("", "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
            ("", "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc"),
            ("", "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc"),
            ("th", "/usr/share/fonts/truetype/noto/NotoSansThai-Regular.ttf"),
            ("th", "/usr/share/fonts/truetype/tlwg/Garuda.ttf"),
            ("hi", "/usr/share/fonts/truetype/noto/NotoSansDevanagari-Regular.ttf"),
            ("hi", "/usr/share/fonts/noto/NotoSansDevanagari-Regular.ttf"),
            ("", "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf"),
            ("", "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc"),
            ("", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
        ];
        let mut out = Vec::new();
        for (lang, p) in PATHS {
            let Ok(data) = std::fs::read(p) else { continue };
            if let Ok(f) = FontVec::try_from_vec_and_index(data, 0) {
                out.push((*lang, f));
            }
        }
        out
    })
}

/// The system font that draws `c` when Roboto has it not: one meant for the interface's
/// language first, then any. For the game's own texts as much as the launcher's.
pub fn fallback_font(c: char) -> Option<&'static FontVec> {
    let fonts = fallback_fonts();
    let lang = crate::i18n::language();
    let has = |f: &FontVec| f.glyph_id(c).0 != 0;
    if !lang.is_empty() {
        if let Some((_, f)) = fonts.iter().find(|(l, f)| *l == lang && has(f)) {
            return Some(f);
        }
    }
    fonts.iter().find(|(_, f)| has(f)).map(|(_, f)| f)
}

/// Whether `c` has to come from another font than Roboto (`main`).
pub fn needs_fallback(main: &impl Font, c: char) -> bool {
    !(main.glyph_id(c).0 != 0 || (c as u32) < 0x2000 && !((c as u32) >= 0x0590 && (c as u32) < 0x1100))
}

/// The font that draws `c`: the first of `chain` that has it, else the first system font
/// that has it (a character of the common scripts that none has stays with the first: it
/// draws its own placeholder).
fn font_for(chain: &[FontVec], c: char) -> &FontVec {
    if let Some(f) = chain.iter().find(|f| f.glyph_id(c).0 != 0) {
        return f;
    }
    let main = &chain[0];
    if !needs_fallback(main, c) {
        return main;
    }
    fallback_font(c).unwrap_or(main)
}

/// Roboto at weight `w` (its variable font set to the weight's axes).
fn roboto(w: Weight) -> Option<FontVec> {
    let mut f = FontVec::try_from_vec(ROBOTO.to_vec()).ok()?;
    let (wght, wdth) = w.axes();
    f.set_variation(b"wght", wght);
    f.set_variation(b"wdth", wdth);
    Some(f)
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

impl Fonts {
    pub fn new() -> Fonts {
        let faces = Weight::ALL.iter().filter_map(|&w| Some((w, vec![roboto(w)?]))).collect();
        Fonts { faces }
    }

    /// The launcher's typeface: Hanken Grotesk, with Roboto after it for the scripts it has
    /// not and for the narrow figures (`Condensed` is Roboto's own).
    pub fn hanken() -> Fonts {
        let faces = Weight::ALL
            .iter()
            .filter_map(|&w| {
                let mut chain: Vec<FontVec> = HANKEN.iter().filter(|(k, _)| *k == w).flat_map(|(_, files)| files.iter()).filter_map(|b| FontVec::try_from_vec(b.to_vec()).ok()).collect();
                chain.push(roboto(w)?);
                Some((w, chain))
            })
            .collect();
        Fonts { faces }
    }

    fn chain(&self, w: Weight) -> &[FontVec] {
        &self.faces.iter().find(|(k, _)| *k == w).unwrap_or(&self.faces[0]).1
    }

    fn face(&self, w: Weight) -> &FontVec {
        &self.chain(w)[0]
    }

    /// Width of `text` in pixels at `px`.
    pub fn width(&self, text: &str, px: f32, weight: Weight) -> f32 {
        self.width_as_is(&crate::i18n::tr(text), px, weight)
    }

    /// Width of `text` as it is, not translated (a name: the launcher's own, letter by letter).
    pub fn width_as_is(&self, text: &str, px: f32, weight: Weight) -> f32 {
        let comp = composed(text);
        let text = &*comp;
        let main = self.chain(weight);
        let mut w = 0.0;
        let mut prev: Option<(ab_glyph::GlyphId, *const FontVec)> = None;
        for c in text.chars().map(substitute) {
            let font = font_for(main, c);
            let f = font.as_scaled(PxScale::from(px));
            let id = f.glyph_id(c);
            if let Some((p, pf)) = prev {
                if std::ptr::eq(pf, font) {
                    w += f.kern(p, id);
                }
            }
            w += f.h_advance(id);
            prev = Some((id, font as *const FontVec));
        }
        w
    }

    /// Width of `text` as it is, set in a `Style`.
    pub fn width_styled(&self, text: &str, px: f32, weight: Weight, style: Style) -> f32 {
        self.width_as_is(text, px, weight) * (1.0 + style.stretch) + style.widen(px) * composed(text).chars().count() as f32
    }

    /// Line height (ascent − descent) at `px`.
    pub fn line_height(&self, px: f32, weight: Weight) -> f32 {
        let f = self.face(weight).as_scaled(PxScale::from(px));
        f.ascent() - f.descent()
    }

    /// Height of capitals above the baseline at `px` (for centring a line on a box).
    pub fn cap_height(&self, px: f32, weight: Weight) -> f32 {
        let f = self.face(weight).as_scaled(PxScale::from(px));
        let id = f.glyph_id('H');
        let g = id.with_scale(PxScale::from(px));
        self.face(weight).outline_glyph(g).map(|o| -o.px_bounds().min.y).unwrap_or(f.ascent() * 0.7)
    }

    /// The longest start of `text` that fits in `max` pixels, with an ellipsis when cut.
    pub fn fit(&self, text: &str, px: f32, weight: Weight, max: f32) -> String {
        let translated = crate::i18n::tr(text);
        let comp = composed(&translated);
        let text = &*comp;
        if self.width(text, px, weight) <= max {
            return text.to_string();
        }
        let chars: Vec<char> = text.chars().collect();
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let s: String = chars[..mid].iter().collect::<String>().trim_end().to_string() + "…";
            if self.width(&s, px, weight) <= max {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        chars[..lo].iter().collect::<String>().trim_end().to_string() + "…"
    }

    /// Rasterise one line.
    pub fn render(&self, text: &str, px: f32, weight: Weight) -> Bitmap {
        let comp = composed(text);
        let text = &*comp;
        let chain = self.chain(weight);
        let font = &chain[0];
        let f = font.as_scaled(PxScale::from(px));
        let pad = PAD as f32;
        let asc = f.ascent();
        let h = ((asc - f.descent()).ceil() as u32 + 2 * PAD).max(1);
        let mut glyphs: Vec<(&FontVec, ab_glyph::Glyph)> = Vec::new();
        let mut x = pad;
        let mut prev: Option<(ab_glyph::GlyphId, *const FontVec)> = None;
        for c in text.chars().map(substitute) {
            let gf = font_for(chain, c);
            let sf = gf.as_scaled(PxScale::from(px));
            let id = sf.glyph_id(c);
            if let Some((p, pf)) = prev {
                if std::ptr::eq(pf, gf) {
                    x += sf.kern(p, id);
                }
            }
            glyphs.push((gf, id.with_scale_and_position(PxScale::from(px), ab_glyph::point(x, pad + asc))));
            x += sf.h_advance(id);
            prev = Some((id, gf as *const FontVec));
        }
        let _ = &f;
        let w = (x.ceil() as u32 + PAD).max(1);
        let mut cov = vec![0f32; (w * h) as usize];
        for (gf, g) in glyphs {
            if let Some(o) = gf.outline_glyph(g) {
                let b = o.px_bounds();
                o.draw(|gx, gy, c| {
                    let xx = b.min.x as i32 + gx as i32;
                    let yy = b.min.y as i32 + gy as i32;
                    if xx >= 0 && yy >= 0 && (xx as u32) < w && (yy as u32) < h {
                        let i = (yy as u32 * w + xx as u32) as usize;
                        cov[i] = (cov[i] + c).min(1.0);
                    }
                });
            }
        }
        Bitmap { w, h, alpha: cov.iter().map(|c| (c * 255.0).round() as u8).collect(), ascent: asc + pad }
    }

    /// Rasterise one line in a `Style`: as `render`, its pen `PAD` + `style.lead(px)` from the
    /// picture's left; as tall as the slant and the boldening make it.
    pub fn render_styled(&self, text: &str, px: f32, weight: Weight, style: Style) -> Bitmap {
        if style == Style::default() {
            return self.render(text, px, weight);
        }
        let comp = composed(text);
        let chain = self.chain(weight);
        let f = chain[0].as_scaled(PxScale::from(px));
        let pad = PAD as f32;
        let e = style.bold * px;
        let wide = 1.0 + style.stretch;
        // (the boldening grows the letters upwards - thinning shrinks them down: they keep
        // standing on the baseline)
        let asc = f.ascent() + 2.0 * e;
        let base = pad + asc;
        let h = ((asc - f.descent()).ceil() as u32 + 2 * PAD).max(1);
        let mut x = pad + style.lead(px);
        let mut prev: Option<(ab_glyph::GlyphId, *const FontVec)> = None;
        let mut glyphs = Vec::new();
        let mut right = x;
        for c in comp.chars().map(substitute) {
            let gf = font_for(chain, c);
            let sf = gf.as_scaled(PxScale::from(px));
            let id = sf.glyph_id(c);
            if let Some((p, pf)) = prev {
                if std::ptr::eq(pf, gf) {
                    x += sf.kern(p, id) * wide;
                }
            }
            if let Some(o) = gf.outline(id) {
                // in pixels, y up from the baseline: stretched, boldened, then slanted
                let (hs, vs) = (sf.h_scale_factor() * wide, sf.v_scale_factor());
                let scaled: Vec<ab_glyph::OutlineCurve> = o.curves.iter().map(|c| map_curve(c, |p| ab_glyph::point(p.x * hs, p.y * vs))).collect();
                let bold = if e != 0.0 { embolden(&scaled, e) } else { scaled };
                let curves: Vec<ab_glyph::OutlineCurve> = bold.iter().map(|c| map_curve(c, |p| ab_glyph::point(p.x + style.slant * p.y, p.y))).collect();
                // (the control points' hull holds the curves)
                let (mut lo, mut hi) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
                for p in curves.iter().flat_map(points_of) {
                    lo = lo.min(p);
                    hi = hi.max(p);
                }
                if lo.x < hi.x && lo.y < hi.y {
                    let outline = ab_glyph::Outline { bounds: ab_glyph::Rect { min: ab_glyph::point(lo.x, hi.y), max: ab_glyph::point(hi.x, lo.y) }, curves };
                    let g = id.with_scale_and_position(PxScale::from(px), ab_glyph::point(x, base));
                    let og = ab_glyph::OutlinedGlyph::new(g, outline, ab_glyph::PxScaleFactor { horizontal: 1.0, vertical: 1.0 });
                    right = right.max(og.px_bounds().max.x);
                    glyphs.push(og);
                }
            }
            x += sf.h_advance(id) * wide + style.widen(px);
            right = right.max(x);
            prev = Some((id, gf as *const FontVec));
        }
        let w = (right.ceil() as u32 + PAD).max(1);
        let mut cov = vec![0f32; (w * h) as usize];
        for o in glyphs {
            let b = o.px_bounds();
            o.draw(|gx, gy, c| {
                let xx = b.min.x as i32 + gx as i32;
                let yy = b.min.y as i32 + gy as i32;
                if xx >= 0 && yy >= 0 && (xx as u32) < w && (yy as u32) < h {
                    let i = (yy as u32 * w + xx as u32) as usize;
                    cov[i] = (cov[i] + c).min(1.0);
                }
            });
        }
        Bitmap { w, h, alpha: cov.iter().map(|c| (c * 255.0).round() as u8).collect(), ascent: base }
    }
}

/// A curve's points, its control points with them.
fn points_of(c: &ab_glyph::OutlineCurve) -> Vec<glam::Vec2> {
    use ab_glyph::OutlineCurve as C;
    let v = |p: &ab_glyph::Point| glam::Vec2::new(p.x, p.y);
    match c {
        C::Line(a, b) => vec![v(a), v(b)],
        C::Quad(a, b, d) => vec![v(a), v(b), v(d)],
        C::Cubic(a, b, d, g) => vec![v(a), v(b), v(d), v(g)],
    }
}

/// A curve with `f` done to each of its points.
fn map_curve(c: &ab_glyph::OutlineCurve, mut f: impl FnMut(ab_glyph::Point) -> ab_glyph::Point) -> ab_glyph::OutlineCurve {
    use ab_glyph::OutlineCurve as C;
    match *c {
        C::Line(a, b) => C::Line(f(a), f(b)),
        C::Quad(a, b, d) => C::Quad(f(a), f(b), f(d)),
        C::Cubic(a, b, d, g) => C::Cubic(f(a), f(b), f(d), f(g)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The launcher's Hanken Grotesk draws the Latin scripts itself and leaves the rest to
    /// Roboto: a Russian word is not boxes, and the narrow figures stay Roboto's.
    #[test]
    fn hanken_falls_back_to_roboto() {
        let h = Fonts::hanken();
        let r = Fonts::new();
        assert_eq!(h.chain(Weight::Regular).len(), 3);
        assert!(h.width("Bauernhof", 20.0, Weight::Regular) != r.width("Bauernhof", 20.0, Weight::Regular), "another typeface");
        for word in ["Łódź", "Şişli", "Haltestelle Größe"] {
            assert!(word.chars().all(|c| std::ptr::eq(font_for(h.chain(Weight::Bold), c), &h.chain(Weight::Bold)[0]) || std::ptr::eq(font_for(h.chain(Weight::Bold), c), &h.chain(Weight::Bold)[1])), "{word}");
        }
        let ru = h.chain(Weight::Regular);
        assert!(std::ptr::eq(font_for(ru, 'Ж'), &ru[2]), "Cyrillic from Roboto");
        assert!(h.render("Привет", 16.0, Weight::Regular).alpha.iter().any(|&a| a > 128));
        assert_eq!(h.chain(Weight::Condensed).len(), 1);
    }

    /// A square, clockwise with y up as TrueType has its outer contours, pushed out by 2 on
    /// every side and moved right and up by as much: it stands where it stood, 4 larger.
    #[test]
    fn emboldening_pushes_outlines_out_evenly() {
        use ab_glyph::{point, OutlineCurve as C};
        let sq = [(0.0, 0.0), (0.0, 10.0), (10.0, 10.0), (10.0, 0.0)];
        let curves: Vec<C> = (0..4).map(|i| C::Line(point(sq[i].0, sq[i].1), point(sq[(i + 1) % 4].0, sq[(i + 1) % 4].1))).collect();
        let b = embolden(&curves, 2.0);
        let pts: Vec<(f32, f32)> = b.iter().map(|c| if let C::Line(a, _) = c { (a.x, a.y) } else { unreachable!() }).collect();
        assert_eq!(pts, vec![(0.0, 0.0), (0.0, 14.0), (14.0, 14.0), (14.0, 0.0)]);
        // a hole (against the clock inside it) gets smaller: the ink round it grows into it
        let hole = [(3.0, 3.0), (7.0, 3.0), (7.0, 7.0), (3.0, 7.0)];
        let mut both = curves.clone();
        both.extend((0..4).map(|i| C::Line(point(hole[i].0, hole[i].1), point(hole[(i + 1) % 4].0, hole[(i + 1) % 4].1))));
        let b = embolden(&both, 1.0);
        let inner: Vec<(f32, f32)> = b[4..].iter().map(|c| if let C::Line(a, _) = c { (a.x, a.y) } else { unreachable!() }).collect();
        assert_eq!(inner, vec![(5.0, 5.0), (7.0, 5.0), (7.0, 7.0), (5.0, 7.0)]);
    }

    /// A slanted, boldened I: heavier, its top further right than its foot by the slant, on
    /// the baseline; plain, the same as an upright line.
    #[test]
    fn a_styled_line_leans_and_is_heavier() {
        let f = Fonts::hanken();
        let up = f.render("I", 100.0, Weight::Black);
        assert_eq!(f.render_styled("I", 100.0, Weight::Black, Style::default()).alpha, up.alpha);
        let st = Style { slant: 0.27, bold: 0.03, stretch: 0.0 };
        let it = f.render_styled("I", 100.0, Weight::Black, st);
        let ink = |b: &Bitmap| b.alpha.iter().map(|&a| a as u64).sum::<u64>();
        assert!(ink(&it) > ink(&up) * 13 / 10, "{} {}", ink(&it), ink(&up));
        let left = |b: &Bitmap, y: u32| (0..b.w).find(|&x| b.alpha[(y * b.w + x) as usize] > 128);
        let rows: Vec<u32> = (0..it.h).filter(|&y| left(&it, y).is_some()).collect();
        let (top, foot) = (rows[3], rows[rows.len() - 4]);
        let lean = (left(&it, top).unwrap() as f32 - left(&it, foot).unwrap() as f32) / (foot - top) as f32;
        assert!((lean - 0.27).abs() < 0.03, "{lean}");
        assert!(((rows[rows.len() - 1] + 1) as f32 - it.ascent).abs() < 1.0, "{rows:?} {}", it.ascent);
        // the pen where it says: the foot's left edge as far past it as the upright I's
        let foot_up = (0..up.h).filter(|&y| left(&up, y).is_some()).last().unwrap();
        let from_pen = |b: &Bitmap, y: u32, lead: f32| left(b, y).unwrap() as f32 - PAD as f32 - lead;
        assert!((from_pen(&it, rows[rows.len() - 1], st.lead(100.0)) - from_pen(&up, foot_up, 0.0)).abs() <= 1.5);
        assert!(it.w as f32 >= up.w as f32 + st.widen(100.0) + st.lead(100.0) - 1.0);
        // stretched: wider by as much, and its width says so
        let wide = Style { stretch: 0.2, ..st };
        let ink_w = |b: &Bitmap, y: u32| (0..b.w).filter(|&x| b.alpha[(y * b.w + x) as usize] > 128).count() as f32;
        let (o, ow) = (f.render_styled("O", 100.0, Weight::Black, st), f.render_styled("O", 100.0, Weight::Black, wide));
        let mid = |b: &Bitmap| (b.ascent - 0.35 * f.cap_height(100.0, Weight::Black)) as u32;
        assert!((ink_w(&ow, mid(&ow)) / ink_w(&o, mid(&o)) - 1.2).abs() < 0.08, "{} {}", ink_w(&ow, mid(&ow)), ink_w(&o, mid(&o)));
        assert!((f.width_styled("OMSI", 100.0, Weight::Black, wide) - (f.width_as_is("OMSI", 100.0, Weight::Black) * 1.2 + 4.0 * wide.widen(100.0))).abs() < 1e-3);
    }

    #[test]
    fn weights_differ_and_text_fits() {
        let f = Fonts::new();
        let regular = f.render("Bauernhof", 20.0, Weight::Regular);
        let bold = f.render("Bauernhof", 20.0, Weight::Bold);
        let ink = |b: &Bitmap| b.alpha.iter().map(|&a| a as u64).sum::<u64>();
        assert!(ink(&bold) > ink(&regular) * 11 / 10, "bold {} regular {}", ink(&bold), ink(&regular));
        assert!(f.width("Bauernhof", 20.0, Weight::Condensed) < f.width("Bauernhof", 20.0, Weight::Bold));
        let cut = f.fit("Krankenhaus Grundorf Nord", 16.0, Weight::Regular, 100.0);
        assert!(cut.ends_with('…') && f.width(&cut, 16.0, Weight::Regular) <= 100.0, "{cut}");
        assert_eq!(f.fit("Kurz", 16.0, Weight::Regular, 100.0), "Kurz");
    }
}

#[cfg(test)]
mod fallback_tests {
    use super::*;
    /// Scripts Roboto lacks are drawn with a system font where the system has one.
    #[test]
    fn cjk_and_devanagari_come_from_the_system() {
        if fallback_fonts().is_empty() {
            return;
        }
        let f = Fonts::new();
        for t in ["日本語", "中文", "हिन्दी"] {
            let b = f.render(t, 20.0, Weight::Regular);
            let ink: u64 = b.alpha.iter().map(|&a| a as u64).sum();
            assert!(ink > 1000, "{t}: {ink}");
        }
    }
}

#[cfg(test)]
mod glyph_tests {
    use super::*;
    #[test]
    fn portuguese_letters_are_present_in_roboto() {
        let f = ab_glyph::FontRef::try_from_slice(ROBOTO).unwrap();
        // PT-BR + PT-PT: acute, grave, circumflex, tilde and cedilla, both cases.
        for c in "áàâãéêíóôõúçÁÀÂÃÉÊÍÓÔÕÚÇ".chars() {
            assert_ne!(f.glyph_id(c).0, 0, "missing Portuguese glyph {c}");
        }
    }

    #[test]
    fn missing_symbols_are_substituted() {
        let f = ab_glyph::FontRef::try_from_slice(ROBOTO).unwrap();
        for c in "→★⚠✓▸ Bauernhof · 12 °C - ДёЖ".chars() {
            assert!(f.glyph_id(substitute(c)).0 != 0 || substitute(c) == ' ', "{c}");
        }
    }
}
