//! Display fonts drawn the way a destination display shows letters: in its own grid of dots.
//!
//! A display font the player chose for a bus ([`DisplayFontSpec`]) is an OMSI `.oft` font (by
//! its `[newfont]` name) or a TrueType/OpenType font (`.ttf`, `.otf`, `.ttc`; one face of the
//! file). Either becomes a font of dots first ([`DotFont`]): an `.oft` font is read in its own
//! grid - a font drawn as a picture of a matrix (the Annax fonts' 3x3 dots in 4 px cells, a
//! pixel font magnified four times) comes back as the matrix it shows - and a vector font is
//! rasterised at a number of dot rows. Bold and the letter spacing are dots too. The dots are
//! then laid into the grid of the display the font is drawn on ([`DotGrid`], read from the
//! display's own font): one dot of the font is one dot of the display, or a square of them
//! when the font has fewer rows than the display, thresholded as a matrix shows it (on or
//! off) unless the display's own font is a smooth one. The result is a [`FontAtlas`] of the
//! height of a line of the display's own font ([`DotFont::to_atlas`]): the game's text and
//! script textures (`omsi_sim::texttex`, `omsi_sim::host`) and the launcher's previews draw
//! with that same atlas.

use crate::font::{Font, FontAtlas, FontChar};
use std::path::{Path, PathBuf};

// --- the choice -------------------------------------------------------------------------------

/// A display font with the settings that matter on a display, as the launcher keeps it per
/// bus and hands it to the game (`--display-font`, see [`DisplayFontSpec::to_arg`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct DisplayFontSpec {
    /// The `[newfont]` name of an `.oft` font, or the full name of a vector font's face.
    pub name: String,
    /// The TrueType/OpenType file of a vector font (none: an `.oft` font, found by its name).
    pub file: Option<PathBuf>,
    /// The face in a font collection (`.ttc`).
    pub face: u32,
    /// How many rows of dots the font's letters are drawn in (none: as many as the display's
    /// own font has - a vector font rasterised at that, an `.oft` font in its own).
    pub rows: Option<u32>,
    /// Every stroke a dot wider.
    pub bold: bool,
    /// Dots between two letters (none: the font's own).
    pub spacing: Option<u32>,
}

impl DisplayFontSpec {
    /// An `.oft` font by its name, as it is.
    pub fn named(name: &str) -> DisplayFontSpec {
        DisplayFontSpec { name: name.trim().to_string(), ..Default::default() }
    }

    /// One face of a vector font file, as it is.
    pub fn vector(name: &str, file: &Path, face: u32) -> DisplayFontSpec {
        DisplayFontSpec { name: name.trim().to_string(), file: Some(file.to_path_buf()), face, ..Default::default() }
    }

    /// Read a `--display-font` argument: a font's name alone (as the launcher wrote it before
    /// vector fonts and the settings), or the name with `|key=value` parts after it - `file=`,
    /// `face=`, `rows=`, `bold`, `spacing=`. None for a blank one.
    pub fn parse(arg: &str) -> Option<DisplayFontSpec> {
        let mut parts = arg.split('|');
        let name = parts.next().unwrap_or_default().trim();
        if name.is_empty() {
            return None;
        }
        let mut spec = DisplayFontSpec::named(name);
        for p in parts {
            let (k, v) = p.split_once('=').map(|(k, v)| (k.trim(), v.trim())).unwrap_or((p.trim(), ""));
            match k.to_ascii_lowercase().as_str() {
                "file" if !v.is_empty() => spec.file = Some(PathBuf::from(v)),
                "face" => spec.face = v.parse().unwrap_or(0),
                "rows" => spec.rows = v.parse().ok().filter(|r| *r > 0),
                "bold" => spec.bold = !matches!(v, "0" | "false" | "no"),
                "spacing" => spec.spacing = v.parse().ok(),
                _ => {}
            }
        }
        Some(spec)
    }

    /// The argument the game reads back with [`DisplayFontSpec::parse`]: the name alone when
    /// there is nothing else to say (what a game of before understands too).
    pub fn to_arg(&self) -> String {
        let mut out = self.name.trim().to_string();
        if let Some(f) = &self.file {
            out.push_str(&format!("|file={}", f.display()));
            if self.face > 0 {
                out.push_str(&format!("|face={}", self.face));
            }
        }
        if let Some(r) = self.rows {
            out.push_str(&format!("|rows={r}"));
        }
        if self.bold {
            out.push_str("|bold");
        }
        if let Some(s) = self.spacing {
            out.push_str(&format!("|spacing={s}"));
        }
        out
    }

    /// The font itself, without the settings (what a list of fonts offers).
    pub fn plain(&self) -> DisplayFontSpec {
        DisplayFontSpec { name: self.name.clone(), file: self.file.clone(), face: self.face, ..Default::default() }
    }

    /// Whether it is a TrueType/OpenType font.
    pub fn is_vector(&self) -> bool {
        self.file.is_some()
    }

    /// The same font (whatever the settings): the same name, and the same file and face.
    pub fn same_font(&self, other: &DisplayFontSpec) -> bool {
        self.name.trim().eq_ignore_ascii_case(other.name.trim()) && self.face == other.face && self.file.as_deref().map(path_key) == other.file.as_deref().map(path_key)
    }

    /// A key for caches and previews: the argument in lower case.
    pub fn key(&self) -> String {
        self.to_arg().to_lowercase()
    }
}

fn path_key(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/").to_lowercase()
}

// --- the display's grid -----------------------------------------------------------------------

/// The grid of dots a display's own font draws in: a dot every `pitch` pixels, `dot` pixels
/// of it lit (less than the pitch: a dark seam between the dots, as the Annax fonts draw
/// them), the first one `phase_x`/`phase_y` pixels into a letter, and how many rows of dots a
/// line of the font has. `smooth`: the font is no matrix but smooth letters (anti-aliased
/// pixels at a pitch of one), which a chosen font keeps too instead of being thresholded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DotGrid {
    pub pitch: u32,
    pub dot: u32,
    pub phase_x: u32,
    pub phase_y: u32,
    pub rows: u32,
    pub smooth: bool,
}

impl DotGrid {
    /// A matrix of `rows` rows whose every pixel is a dot (what a script's matrix texture is:
    /// the script lights an LED for each of its pixels).
    pub fn plain(rows: u32) -> DotGrid {
        DotGrid { pitch: 1, dot: 1, phase_x: 0, phase_y: 0, rows: rows.max(1), smooth: false }
    }

    /// The grid `atlas`'s font draws in, read from its letters: the largest pitch at which
    /// every stretch of lit pixels starts and ends at the same place of a cell (in both
    /// directions). A font of no such grid (smooth letters, a pixel font as it is) has a
    /// pitch of one.
    pub fn of_atlas(atlas: &FontAtlas) -> DotGrid {
        let h = atlas.font.height.max(1) as u32;
        let (aw, ah) = (atlas.width as i32, atlas.height as i32);
        let alpha = |x: i32, y: i32| -> u8 {
            if x < 0 || y < 0 || x >= aw || y >= ah {
                return 0;
            }
            atlas.alpha.get(((y * aw + x) * 4) as usize).copied().unwrap_or(0)
        };
        // stretches of ink (alpha of half and more) along the rows and the columns of every
        // letter, from the letter's corner; and how much of the ink is partly covered
        let (mut h_runs, mut v_runs): (Vec<(u32, u32)>, Vec<(u32, u32)>) = (Vec::new(), Vec::new());
        let (mut inked, mut partial) = (0usize, 0usize);
        let mut seen: Vec<char> = Vec::new();
        for g in atlas.font.chars.iter().filter(|g| !g.ch.is_whitespace() && g.x1 > g.x0).take(256) {
            if seen.contains(&g.ch) {
                continue;
            }
            seen.push(g.ch);
            let gw = (g.x1 - g.x0) as u32;
            for y in 0..h {
                let mut start: Option<u32> = None;
                for x in 0..=gw {
                    let a = if x < gw { alpha(g.x0 + x as i32, g.y + y as i32) } else { 0 };
                    if x < gw && a > 0 {
                        inked += 1;
                        if (16..240).contains(&a) {
                            partial += 1;
                        }
                    }
                    match (a >= 128, start) {
                        (true, None) => start = Some(x),
                        (false, Some(s)) => {
                            h_runs.push((s, x));
                            start = None;
                        }
                        _ => {}
                    }
                }
            }
            for x in 0..gw {
                let mut start: Option<u32> = None;
                for y in 0..=h {
                    let a = if y < h { alpha(g.x0 + x as i32, g.y + y as i32) } else { 0 };
                    match (a >= 128, start) {
                        (true, None) => start = Some(y),
                        (false, Some(s)) => {
                            v_runs.push((s, y));
                            start = None;
                        }
                        _ => {}
                    }
                }
            }
        }
        let smooth = inked > 0 && partial * 4 > inked;
        let plain = DotGrid { smooth, ..DotGrid::plain(h) };
        if smooth || h_runs.len() < 12 || v_runs.len() < 12 {
            return plain;
        }
        // (the residue most stretches start - or end - at, and how many do)
        let mode = |v: &mut dyn Iterator<Item = u32>, p: u32| -> (u32, f32) {
            let mut counts = vec![0usize; p as usize];
            let mut n = 0usize;
            for x in v {
                counts[(x % p) as usize] += 1;
                n += 1;
            }
            let (r, c) = counts.iter().enumerate().max_by_key(|(i, c)| (**c, std::cmp::Reverse(*i))).map(|(i, c)| (i as u32, *c)).unwrap_or((0, 0));
            (r, if n == 0 { 0.0 } else { c as f32 / n as f32 })
        };
        for p in (2..=12u32).rev() {
            if p * 2 > h {
                continue;
            }
            let (hs, fhs) = mode(&mut h_runs.iter().map(|r| r.0), p);
            let (he, fhe) = mode(&mut h_runs.iter().map(|r| r.1), p);
            let (vs, fvs) = mode(&mut v_runs.iter().map(|r| r.0), p);
            let (ve, fve) = mode(&mut v_runs.iter().map(|r| r.1), p);
            if [fhs, fhe, fvs, fve].iter().all(|f| *f >= 0.97) {
                let dot_of = |s: u32, e: u32| match (e + p - s) % p {
                    0 => p,
                    d => d,
                };
                let dot = dot_of(hs, he).min(dot_of(vs, ve));
                let rows = if h >= vs + dot { (h - vs - dot) / p + 1 } else { (h / p).max(1) };
                return DotGrid { pitch: p, dot, phase_x: hs, phase_y: vs, rows, smooth: false };
            }
        }
        plain
    }
}

// --- a font of dots ---------------------------------------------------------------------------

/// One letter of a [`DotFont`]: `w` columns of the font's rows, each dot's coverage (0-255),
/// row by row.
#[derive(Debug, Clone, PartialEq)]
pub struct DotGlyph {
    pub ch: char,
    pub w: u32,
    pub cov: Vec<u8>,
}

/// A font as dots: its letters in `rows` rows, `gap` dots between two of them, a space
/// `space` dots wide.
#[derive(Debug, Clone, PartialEq)]
pub struct DotFont {
    pub name: String,
    pub rows: u32,
    pub gap: u32,
    pub space: u32,
    pub glyphs: Vec<DotGlyph>,
}

/// Fewer rows than this are too few for small letters with descenders: a font is made into
/// capitals there (its small letters written as capitals), as signs of so few rows write.
pub const CAPITALS_BELOW: u32 = 10;

/// The capital a small letter is written as on a sign of capitals.
fn capital_of(c: char) -> Option<char> {
    if !c.is_lowercase() {
        return None;
    }
    let mut up = c.to_uppercase();
    match (up.next(), up.next()) {
        (Some(u), None) if u != c => Some(u),
        _ => None,
    }
}

/// The letters a vector font is made into dots for: Latin with its accents (Western and
/// Central Europe), Greek, Cyrillic and the punctuation of destinations.
pub fn charset() -> Vec<char> {
    let ranges: [(u32, u32); 6] = [(0x21, 0x7e), (0xa1, 0x17f), (0x386, 0x3ce), (0x400, 0x45f), (0x490, 0x491), (0x2013, 0x2014)];
    let mut out: Vec<char> = ranges.iter().flat_map(|&(a, b)| (a..=b).filter_map(char::from_u32)).collect();
    out.extend(['‘', '’', '‚', '“', '”', '„', '…', '•', '€', '№']);
    out
}

impl DotFont {
    /// The letters of `atlas` as dots, each read at the middle of its dots in `grid` (the
    /// font's own, [`DotGrid::of_atlas`]).
    pub fn from_atlas(atlas: &FontAtlas, grid: &DotGrid) -> DotFont {
        let p = grid.pitch.max(1);
        let half = (grid.dot / 2) as i32;
        let (aw, ah) = (atlas.width as i32, atlas.height as i32);
        let mut glyphs: Vec<DotGlyph> = Vec::new();
        for g in atlas.font.chars.iter().filter(|g| !g.ch.is_whitespace()) {
            if glyphs.iter().any(|d| d.ch == g.ch) {
                continue;
            }
            let gw = (g.x1 - g.x0).max(0) as u32;
            let cols = if gw > grid.phase_x { (gw - grid.phase_x).div_ceil(p) } else { 0 };
            let mut cov = vec![0u8; (cols * grid.rows) as usize];
            for j in 0..grid.rows {
                for i in 0..cols {
                    let x = g.x0 + (grid.phase_x + i * p) as i32 + half;
                    let y = g.y + (grid.phase_y + j * p) as i32 + half;
                    if x >= g.x1 || x < 0 || y < 0 || x >= aw || y >= ah {
                        continue;
                    }
                    cov[(j * cols + i) as usize] = atlas.alpha.get(((y * aw + x) * 4) as usize).copied().unwrap_or(0);
                }
            }
            glyphs.push(DotGlyph { ch: g.ch, w: cols, cov });
        }
        let dots = |px: i32| ((px.max(0) as f32) / p as f32).round() as u32;
        DotFont { name: atlas.font.name.trim().to_string(), rows: grid.rows.max(1), gap: dots(atlas.font.gap), space: dots(atlas.font.space_width()).max(1), glyphs }
    }

    /// Face `face` of the TrueType/OpenType font `data` rasterised at `rows` rows of dots, for
    /// the letters `chars`, as a matrix sign's font fills its rows: from the top of its
    /// capitals and tall letters to the bottom of its descenders (a letter reaching further -
    /// a capital with an accent - squeezed into them, as signs do, not cut off), each letter's
    /// outline as much as it covers of every dot, its empty columns at either side left out:
    /// the letters stand `gap` dots apart. In fewer than `CAPITALS_BELOW` rows the capitals
    /// fill them and small letters are written as capitals.
    pub fn from_vector(data: &[u8], face: u32, rows: u32, chars: &[char]) -> Result<DotFont, String> {
        use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont as _};
        let font = FontRef::try_from_slice_and_index(data, face).map_err(|e| e.to_string())?;
        let rows = rows.clamp(3, 512);
        let capitals = rows < CAPITALS_BELOW;
        let has = |c: char| font.glyph_id(c).0 != 0;
        // the rows' span in the font's units: the plain letters' and digits' tops and bottoms
        let (mut top, mut bottom) = (0.0f32, 0.0f32);
        let small: Vec<char> = if capitals { Vec::new() } else { ('a'..='z').collect() };
        for c in ('A'..='Z').chain(small).chain('0'..='9') {
            let id = font.glyph_id(c);
            // (an outline's bounds run from its top, `min`, to its bottom, `max`, in units
            // counted upwards)
            if let Some(o) = (id.0 != 0).then(|| font.outline(id)).flatten() {
                top = top.max(o.bounds.min.y);
                bottom = bottom.min(o.bounds.max.y);
            }
        }
        if capitals {
            // (capitals stand on the line: a Q's tail and a J below it are squeezed in)
            bottom = 0.0;
        }
        if top <= bottom {
            (top, bottom) = (font.ascent_unscaled(), font.descent_unscaled());
        }
        let units = font.height_unscaled().max(1.0);
        // (pixels per unit; a `PxScale` is the pixels of the font's whole height)
        let f = rows as f32 / (top - bottom).max(1.0);
        let scale = PxScale::from(f * units);
        let scaled = font.as_scaled(scale);
        let mut glyphs: Vec<DotGlyph> = Vec::new();
        for &c in chars {
            if c.is_whitespace() || glyphs.iter().any(|g| g.ch == c) || (capitals && capital_of(c).is_some_and(has)) {
                continue;
            }
            let id = font.glyph_id(c);
            if id.0 == 0 {
                continue;
            }
            // (a letter taller or deeper than the rows: squeezed upright into them, from its
            // top to its bottom)
            let (gt, gb) = font.outline(id).map(|o| (o.bounds.min.y.max(top), o.bounds.max.y.min(bottom))).unwrap_or((top, bottom));
            let fy = rows as f32 / (gt - gb).max(1.0);
            let glyph_scale = PxScale { x: f * units, y: fy * units };
            let Some(outlined) = font.outline_glyph(id.with_scale_and_position(glyph_scale, ab_glyph::point(0.0, gt * fy))) else { continue };
            let b = outlined.px_bounds();
            let (w, top) = (b.width().max(0.0) as u32, b.min.y as i32);
            if w == 0 {
                continue;
            }
            let mut cov = vec![0u8; (w * rows) as usize];
            outlined.draw(|x, y, c| {
                let yy = top + y as i32;
                if x < w && yy >= 0 && (yy as u32) < rows {
                    let i = (yy as u32 * w + x) as usize;
                    cov[i] = cov[i].max((c.clamp(0.0, 1.0) * 255.0).round() as u8);
                }
            });
            // (the columns the letter does not reach: its side bearings, which the gap takes over)
            let used = |x: u32| (0..rows).any(|y| cov[(y * w + x) as usize] >= 100);
            let Some(first) = (0..w).find(|&x| used(x)) else { continue };
            let last = (0..w).rev().find(|&x| used(x)).unwrap_or(first);
            let nw = last - first + 1;
            let cut: Vec<u8> = (0..rows).flat_map(|y| (first..=last).map(move |x| (y, x))).map(|(y, x)| cov[(y * w + x) as usize]).collect();
            glyphs.push(DotGlyph { ch: c, w: nw, cov: cut });
        }
        if glyphs.is_empty() {
            return Err("the font has none of the letters".into());
        }
        if capitals {
            add_capitals_for_small(&mut glyphs, chars);
        }
        let space_id = font.glyph_id(' ');
        let space = if space_id.0 != 0 { scaled.h_advance(space_id) } else { rows as f32 / 4.0 };
        // (a dot between letters on a small sign, more on a large one, as the signs' own fonts)
        let gap = ((rows as f32 / 12.0).round() as u32).max(1);
        Ok(DotFont { name: String::new(), rows, gap, space: (space.round() as u32).max(1), glyphs })
    }

    /// The font in `rows` rows: every letter scaled by as much (the share of each dot it
    /// covers averaged), the spacing too. Made smaller than `CAPITALS_BELOW` rows, its capitals
    /// fill them (a letter reaching above or below them squeezed in) and its small letters are
    /// written as capitals.
    pub fn resized(&self, rows: u32) -> DotFont {
        let rows = rows.max(1);
        if rows == self.rows {
            return self.clone();
        }
        if rows < CAPITALS_BELOW && rows < self.rows {
            if let Some(f) = self.capitals_in(rows) {
                return f;
            }
        }
        let f = rows as f32 / self.rows.max(1) as f32;
        let glyphs = self
            .glyphs
            .iter()
            .map(|g| {
                let nw = ((g.w as f32 * f).round() as u32).max(1);
                DotGlyph { ch: g.ch, w: nw, cov: resample_plane(&g.cov, g.w, self.rows, nw, rows) }
            })
            .collect();
        DotFont { name: self.name.clone(), rows, gap: (self.gap as f32 * f).round() as u32, space: ((self.space as f32 * f).round() as u32).max(1), glyphs }
    }

    /// The rows a glyph's dots are lit in (half and more), first and last.
    fn lit_rows(&self, g: &DotGlyph) -> Option<(u32, u32)> {
        let lit = |y: u32| (0..g.w).any(|x| g.cov[(y * g.w + x) as usize] >= 112);
        let first = (0..self.rows).find(|&y| lit(y))?;
        Some((first, (0..self.rows).rev().find(|&y| lit(y)).unwrap_or(first)))
    }

    /// `resized` to a sign of capitals: the rows of the capitals (an H's) made `rows` rows.
    fn capitals_in(&self, rows: u32) -> Option<DotFont> {
        let (ct, cb) = ['H', 'E', 'I', 'T', 'N'].iter().find_map(|&c| self.glyphs.iter().find(|g| g.ch == c).and_then(|g| self.lit_rows(g)))?;
        let f = rows as f32 / (cb - ct + 1) as f32;
        let mut glyphs: Vec<DotGlyph> = Vec::new();
        for g in &self.glyphs {
            if capital_of(g.ch).is_some_and(|u| self.glyphs.iter().any(|o| o.ch == u)) {
                continue;
            }
            // the capitals' rows, and as much more as the letter reaches (an accent, a comma)
            let (gt, gb) = self.lit_rows(g).unwrap_or((ct, cb));
            let (bt, bb) = (gt.min(ct), gb.max(cb));
            let band: Vec<u8> = g.cov[(bt * g.w) as usize..((bb + 1) * g.w) as usize].to_vec();
            let nw = ((g.w as f32 * f).round() as u32).max(1);
            glyphs.push(DotGlyph { ch: g.ch, w: nw, cov: resample_plane(&band, g.w, bb - bt + 1, nw, rows) });
        }
        let small: Vec<char> = self.glyphs.iter().map(|g| g.ch).collect();
        add_capitals_for_small(&mut glyphs, &small);
        Some(DotFont { name: self.name.clone(), rows, gap: ((self.gap as f32 * f).round() as u32).max(1), space: ((self.space as f32 * f).round() as u32).max(1), glyphs })
    }

    /// The font bold (every stroke a dot wider, to the right) and with `spacing` dots between
    /// letters (none: its own).
    pub fn styled(&self, bold: bool, spacing: Option<u32>) -> DotFont {
        let mut out = self.clone();
        if bold {
            for g in out.glyphs.iter_mut() {
                let (w, nw) = (g.w, g.w + 1);
                let mut cov = vec![0u8; (nw * out.rows) as usize];
                for y in 0..out.rows {
                    for x in 0..nw {
                        let at = |x: u32| if x < w { g.cov[(y * w + x) as usize] } else { 0 };
                        cov[(y * nw + x) as usize] = at(x).max(if x > 0 { at(x - 1) } else { 0 });
                    }
                }
                g.w = nw;
                g.cov = cov;
            }
        }
        if let Some(s) = spacing {
            out.gap = s;
        }
        out
    }

    /// The share of `text`'s letters (not its spaces) the font has.
    pub fn coverage(&self, text: &str) -> f32 {
        let letters: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
        if letters.is_empty() {
            return 1.0;
        }
        letters.iter().filter(|&&c| self.glyph(c).is_some()).count() as f32 / letters.len() as f32
    }

    /// The letter drawn for `c`: its own, else (a small letter the font lacks) its capital.
    pub fn glyph(&self, c: char) -> Option<&DotGlyph> {
        self.glyphs.iter().find(|g| g.ch == c).or_else(|| {
            let up: Vec<char> = c.to_uppercase().collect();
            (up.len() == 1 && up[0] != c).then(|| self.glyphs.iter().find(|g| g.ch == up[0])).flatten()
        })
    }

    /// The font as `grid`'s display draws it, `line_h` pixels high (a line of the display's
    /// own font): as many of the display's rows as it has - shrunk to them when it has more,
    /// every dot a square of whole dots when it fits that many times - in the middle of the
    /// line, each dot drawn where the display has one; on or off, unless the display's font is
    /// a smooth one. White letters on nothing: the display's colour is laid over them as over
    /// its own font.
    pub fn to_atlas(&self, grid: &DotGrid, line_h: u32, name: &str) -> FontAtlas {
        let line_h = line_h.max(1);
        let p = grid.pitch.max(1);
        let d = grid.dot.clamp(1, p);
        let n = grid.rows.max(1);
        let font = if self.rows > n { self.resized(n) } else { self.clone() };
        let k = (n / font.rows.max(1)).max(1);
        let top = n.saturating_sub(font.rows * k) / 2;
        let lit = |c: u8| -> u8 {
            if grid.smooth {
                c
            } else if c >= 112 {
                255
            } else {
                0
            }
        };
        // letters side by side in rows of the atlas, a pitch apart (a dot past a letter's last
        // cell does not reach into the next one)
        let mut glyphs: Vec<(char, u32, &DotGlyph)> = font.glyphs.iter().map(|g| (g.ch, g.w * k * p, g)).collect();
        glyphs.sort_by_key(|g| g.0);
        let widest = glyphs.iter().map(|g| g.1).max().unwrap_or(1) + p;
        let atlas_w = widest.max(1024).min(4096).max(widest);
        let mut places: Vec<(u32, u32)> = Vec::with_capacity(glyphs.len());
        let (mut x, mut y) = (0u32, 0u32);
        for g in &glyphs {
            if x + g.1 + p > atlas_w && x > 0 {
                x = 0;
                y += line_h;
            }
            places.push((x, y));
            x += g.1 + p;
        }
        let atlas_h = y + line_h;
        let mut alpha = vec![0u8; (atlas_w * atlas_h * 4) as usize];
        let mut color = vec![0u8; (atlas_w * atlas_h * 4) as usize];
        let mut chars: Vec<FontChar> = Vec::with_capacity(glyphs.len() + 1);
        for (&(ch, w_px, g), &(gx, gy)) in glyphs.iter().zip(&places) {
            chars.push(FontChar { ch, x0: gx as i32, x1: (gx + w_px) as i32, y: gy as i32 });
            for j in 0..font.rows {
                for i in 0..g.w {
                    let a = lit(g.cov[(j * g.w + i) as usize]);
                    if a == 0 {
                        continue;
                    }
                    for kj in 0..k {
                        for ki in 0..k {
                            let px = gx + grid.phase_x + (i * k + ki) * p;
                            let py = gy + grid.phase_y + (top + j * k + kj) * p;
                            for yy in py..(py + d).min(gy + line_h) {
                                for xx in px..(px + d).min(atlas_w) {
                                    let at = ((yy * atlas_w + xx) * 4) as usize;
                                    let v = a.max(alpha[at]);
                                    alpha[at..at + 4].copy_from_slice(&[v; 4]);
                                    color[at..at + 4].copy_from_slice(&[255; 4]);
                                }
                            }
                        }
                    }
                }
            }
        }
        // the space: as wide as the font's, nothing in it
        chars.push(FontChar { ch: ' ', x0: 0, x1: (font.space * k * p) as i32, y: atlas_h as i32 });
        let font_def = Font { path: PathBuf::new(), name: name.to_string(), bitmap: String::new(), alpha: String::new(), height: line_h as i32, gap: (font.gap * k * p) as i32, chars };
        // (white where lit, black between: a script that takes the font's own colours lights
        // its LEDs from them)
        FontAtlas::new(font_def, atlas_w, atlas_h, color, alpha)
    }
}

/// The small letters among `chars` written as their capitals among `glyphs` (a sign of
/// capitals): each a copy of its capital.
fn add_capitals_for_small(glyphs: &mut Vec<DotGlyph>, chars: &[char]) {
    for &c in chars {
        if glyphs.iter().any(|g| g.ch == c) {
            continue;
        }
        if let Some(cap) = capital_of(c).and_then(|u| glyphs.iter().find(|g| g.ch == u)).cloned() {
            glyphs.push(DotGlyph { ch: c, ..cap });
        }
    }
}

/// A plane of coverage `sw` x `sh` scaled to `dw` x `dh`, every new cell the average of the
/// part of the old ones under it.
fn resample_plane(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dw * dh) as usize];
    if sw == 0 || sh == 0 {
        return out;
    }
    let (fx, fy) = (sw as f32 / dw as f32, sh as f32 / dh as f32);
    let span = |a: f32, b: f32, n: u32| -> Vec<(usize, f32)> {
        let mut v = Vec::new();
        let mut i = a.floor().max(0.0) as i64;
        while (i as f32) < b && i < n as i64 {
            let w = b.min(i as f32 + 1.0) - a.max(i as f32);
            if w > 0.0 {
                v.push((i as usize, w));
            }
            i += 1;
        }
        v
    };
    for y in 0..dh {
        let rows = span(y as f32 * fy, (y + 1) as f32 * fy, sh);
        for x in 0..dw {
            let cols = span(x as f32 * fx, (x + 1) as f32 * fx, sw);
            let (mut sum, mut total) = (0.0f32, 0.0f32);
            for &(sy, wy) in &rows {
                for &(sx, wx) in &cols {
                    sum += src[sy * sw as usize + sx] as f32 * wx * wy;
                    total += wx * wy;
                }
            }
            if total > 0.0 {
                out[(y * dw + x) as usize] = (sum / total).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

// --- TrueType and OpenType files --------------------------------------------------------------

/// One face of a TrueType/OpenType file: the file, its place in a collection, and its names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorFace {
    pub path: PathBuf,
    pub face: u32,
    pub family: String,
    pub style: String,
    /// Its full name ("Arial Bold"): what the player picks it by.
    pub name: String,
}

impl VectorFace {
    /// The choice of this face, as it is.
    pub fn spec(&self) -> DisplayFontSpec {
        DisplayFontSpec::vector(&self.name, &self.path, self.face)
    }
}

/// Whether `path` is a TrueType/OpenType font file (or a collection of them).
pub fn is_vector_font(path: &Path) -> bool {
    path.extension().and_then(|x| x.to_str()).is_some_and(|x| ["ttf", "otf", "ttc", "otc"].contains(&x.to_ascii_lowercase().as_str()))
}

/// The faces of the font file `path` and their names - read from its tables of names alone,
/// not the whole file (a list of every font of the system reads only those). None of a file
/// that is no font.
pub fn vector_faces(path: &Path) -> Vec<VectorFace> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else {
        // (a font in an archive of the content: the whole file through the content's VFS)
        return omsi_cfg::vfs::read(path).map(|d| faces_in(path, &mut |off, len| d.get(off as usize..off as usize + len).map(<[u8]>::to_vec))).unwrap_or_default();
    };
    let mut read = |off: u64, len: usize| -> Option<Vec<u8>> {
        file.seek(SeekFrom::Start(off)).ok()?;
        let mut buf = vec![0u8; len];
        file.read_exact(&mut buf).ok()?;
        Some(buf)
    };
    faces_in(path, &mut read)
}

/// `vector_faces` of a font whose bytes `read` gives (offset, length).
fn faces_in(path: &Path, read: &mut dyn FnMut(u64, usize) -> Option<Vec<u8>>) -> Vec<VectorFace> {
    let be32 = |b: &[u8], i: usize| b.get(i..i + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]));
    let Some(head) = read(0, 12) else { return Vec::new() };
    let offsets: Vec<u64> = if &head[0..4] == b"ttcf" {
        let n = be32(&head, 8).unwrap_or(0).min(64) as usize;
        let Some(list) = read(12, n * 4) else { return Vec::new() };
        (0..n).filter_map(|i| be32(&list, i * 4).map(u64::from)).collect()
    } else {
        vec![0]
    };
    offsets.iter().enumerate().filter_map(|(face, &off)| face_names(read, off).map(|(family, style, name)| VectorFace { path: path.to_path_buf(), face: face as u32, family, style, name })).collect()
}

/// The family, style and full name of the face whose table directory is at `off`.
fn face_names(read: &mut dyn FnMut(u64, usize) -> Option<Vec<u8>>, off: u64) -> Option<(String, String, String)> {
    let be16 = |b: &[u8], i: usize| b.get(i..i + 2).map(|s| u16::from_be_bytes([s[0], s[1]]) as usize);
    let be32 = |b: &[u8], i: usize| b.get(i..i + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as u64);
    let dir = read(off, 12)?;
    let tag = &dir[0..4];
    if !(tag == [0, 1, 0, 0] || tag == b"OTTO" || tag == b"true") {
        return None;
    }
    let n = be16(&dir, 4)?.min(256);
    let tables = read(off + 12, n * 16)?;
    let (name_off, name_len) = (0..n).find(|i| &tables[i * 16..i * 16 + 4] == b"name").map(|i| (be32(&tables, i * 16 + 8), be32(&tables, i * 16 + 12)))?;
    let (name_off, name_len) = (name_off?, (name_len? as usize).min(1 << 20));
    let t = read(name_off, name_len)?;
    let count = be16(&t, 2)?;
    let strings = be16(&t, 4)?;
    // (each name in the best language there is: Windows' English, any of Windows', the Mac's)
    let mut best: [(u32, String); 18] = Default::default();
    for r in 0..count {
        let at = 6 + r * 12;
        let (Some(platform), Some(encoding), Some(lang), Some(id), Some(len), Some(o)) = (be16(&t, at), be16(&t, at + 2), be16(&t, at + 4), be16(&t, at + 6), be16(&t, at + 8), be16(&t, at + 10)) else { break };
        if id >= best.len() {
            continue;
        }
        let Some(bytes) = t.get(strings + o..strings + o + len) else { continue };
        let (rank, text) = match (platform, encoding) {
            (3, 0 | 1 | 10) => (if lang == 0x409 { 3 } else { 2 }, String::from_utf16_lossy(&bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<u16>>())),
            (0, _) => (2, String::from_utf16_lossy(&bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<u16>>())),
            (1, 0) => (1, bytes.iter().map(|&b| b as char).collect()),
            _ => continue,
        };
        let text = text.trim().trim_matches('\0').to_string();
        if !text.is_empty() && rank > best[id].0 {
            best[id] = (rank, text);
        }
    }
    let pick = |a: usize, b: usize| if best[a].1.is_empty() { best[b].1.clone() } else { best[a].1.clone() };
    let family = pick(16, 1);
    let style = pick(17, 2);
    let name = if best[4].1.is_empty() { format!("{family} {style}").trim().to_string() } else { best[4].1.clone() };
    (!name.is_empty()).then_some((family, style, name))
}

/// The folders the system keeps its fonts in (those that exist): Windows' own and the
/// user's, the Mac's, Linux's, Android's.
pub fn system_font_dirs() -> Vec<PathBuf> {
    let var = |n: &str| omsi_cfg::env::var_os(n).map(PathBuf::from);
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        dirs.push(var("SystemRoot").or_else(|| var("WINDIR")).unwrap_or_else(|| PathBuf::from("C:\\Windows")).join("Fonts"));
        if let Some(l) = var("LOCALAPPDATA") {
            dirs.push(l.join("Microsoft").join("Windows").join("Fonts"));
        }
    } else if cfg!(target_os = "android") {
        dirs.push(PathBuf::from("/system/fonts"));
    } else if cfg!(target_os = "macos") {
        dirs.extend(["/System/Library/Fonts", "/Library/Fonts"].map(PathBuf::from));
        if let Some(h) = var("HOME") {
            dirs.push(h.join("Library").join("Fonts"));
        }
    } else {
        dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
        if let Some(h) = var("HOME") {
            dirs.push(h.join(".local").join("share").join("fonts"));
            dirs.push(h.join(".fonts"));
        }
    }
    dirs.retain(|d| d.is_dir());
    dirs
}

/// The TrueType/OpenType files in `dir` and the folders under it (three deep), by name.
pub fn vector_font_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<(PathBuf, bool)> = match std::fs::read_dir(dir) {
            Ok(rd) => rd.flatten().map(|e| (e.path(), e.file_type().map(|t| t.is_dir()).unwrap_or(false))).collect(),
            // (a content folder's archive: listed through the content's VFS)
            Err(_) => omsi_cfg::vfs::list_dir(dir).unwrap_or_default().into_iter().map(|(n, d)| (dir.join(n), d)).collect(),
        };
        entries.sort_by_key(|e| e.0.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default());
        for (p, is_dir) in entries {
            if is_dir {
                if depth < 3 {
                    walk(&p, depth + 1, out);
                }
            } else if is_vector_font(&p) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HANKEN: &[u8] = include_bytes!("../../../assets/fonts/HankenGrotesk/HankenGrotesk-latin-700.ttf");

    /// The rows and columns of a glyph's lit dots in an atlas, from its corner.
    fn lit(a: &FontAtlas, ch: char) -> Vec<(u32, u32)> {
        let g = a.font.chars.iter().find(|g| g.ch == ch).unwrap();
        let mut out = Vec::new();
        for y in 0..a.font.height as u32 {
            for x in 0..(g.x1 - g.x0) as u32 {
                let i = (((g.y as u32 + y) * a.width + g.x0 as u32 + x) * 4 + 3) as usize;
                if a.alpha[i] > 0 {
                    out.push((x, y));
                }
            }
        }
        out
    }

    #[test]
    fn a_choice_goes_to_the_game_and_back() {
        assert_eq!(DisplayFontSpec::parse("  "), None);
        let old = DisplayFontSpec::parse(" Annax Small ").unwrap();
        assert_eq!(old, DisplayFontSpec::named("Annax Small"));
        assert_eq!(old.to_arg(), "Annax Small", "a font alone is written as before");
        let mut s = DisplayFontSpec::vector("Arial Bold", Path::new("C:\\Windows\\Fonts\\arialbd.ttf"), 0);
        s.rows = Some(16);
        s.bold = true;
        s.spacing = Some(2);
        assert_eq!(s.to_arg(), "Arial Bold|file=C:\\Windows\\Fonts\\arialbd.ttf|rows=16|bold|spacing=2");
        assert_eq!(DisplayFontSpec::parse(&s.to_arg()), Some(s.clone()));
        let ttc = DisplayFontSpec::vector("Cambria Math", Path::new("/f/cambria.ttc"), 1);
        assert_eq!(DisplayFontSpec::parse(&ttc.to_arg()), Some(ttc.clone()));
        assert!(s.same_font(&DisplayFontSpec::vector("arial bold", Path::new("c:/windows/fonts/ARIALBD.ttf"), 0)));
        assert!(!s.same_font(&ttc));
        assert_eq!(s.plain(), DisplayFontSpec::vector("Arial Bold", Path::new("C:\\Windows\\Fonts\\arialbd.ttf"), 0));
        // (what a later version may add is left alone)
        assert_eq!(DisplayFontSpec::parse("X|colour=red|rows=x"), Some(DisplayFontSpec::named("X")));
    }

    /// A font of `w` x `h` letters 'A' and 'B' whose every dot is drawn `dot` pixels square
    /// in cells of `pitch`, `phase` pixels into the cell; the letters are 'A' a frame, 'B' a
    /// diagonal.
    fn matrix_font(cols: u32, rows: u32, pitch: u32, dot: u32, phase: u32) -> FontAtlas {
        let (gw, h) = (cols * pitch, rows * pitch + phase);
        let aw = gw * 2;
        let mut alpha = vec![0u8; (aw * h * 4) as usize];
        let mut set = |gx: u32, i: u32, j: u32| {
            for y in 0..dot {
                for x in 0..dot {
                    let (px, py) = (gx + phase + i * pitch + x, phase + j * pitch + y);
                    let at = ((py * aw + px) * 4) as usize;
                    alpha[at..at + 4].copy_from_slice(&[255; 4]);
                }
            }
        };
        for j in 0..rows {
            for i in 0..cols {
                if i == 0 || j == 0 || i == cols - 1 || j == rows - 1 {
                    set(0, i, j);
                }
                if i == j * cols / rows {
                    set(gw, i, j);
                }
            }
        }
        let font = Font { name: "Matrix".into(), height: h as i32, gap: pitch as i32, chars: vec![FontChar { ch: 'A', x0: 0, x1: gw as i32, y: 0 }, FontChar { ch: 'B', x0: gw as i32, x1: aw as i32, y: 0 }], ..Default::default() };
        FontAtlas::new(font, aw, h, alpha.clone(), alpha)
    }

    #[test]
    fn a_font_drawn_as_a_matrix_is_read_in_its_own_grid() {
        // the Annax fonts: 3x3 dots in 4 px cells, a pixel into the cell; 7 rows
        let annax = matrix_font(5, 7, 4, 3, 1);
        let g = DotGrid::of_atlas(&annax);
        assert_eq!((g.pitch, g.dot, g.phase_x, g.phase_y, g.rows, g.smooth), (4, 3, 1, 1, 7, false));
        let dots = DotFont::from_atlas(&annax, &g);
        assert_eq!(dots.rows, 7);
        let a = dots.glyph('A').unwrap();
        assert_eq!(a.w, 5);
        assert_eq!(a.cov.iter().filter(|c| **c > 0).count(), 2 * 5 + 2 * 5);
        // a pixel font magnified four times, no seams: its pixels are its dots
        let big = matrix_font(6, 8, 4, 4, 0);
        let g = DotGrid::of_atlas(&big);
        assert_eq!((g.pitch, g.dot, g.rows), (4, 4, 8));
        assert_eq!(DotFont::from_atlas(&big, &g).glyph('A').unwrap().w, 6);
        // a pixel font as it is
        let plain = matrix_font(6, 8, 1, 1, 0);
        assert_eq!(DotGrid::of_atlas(&plain), DotGrid::plain(8));
    }

    #[test]
    fn a_vector_font_becomes_dots_at_the_rows_asked_for() {
        let f = DotFont::from_vector(HANKEN, 0, 16, &charset()).unwrap();
        assert_eq!(f.rows, 16);
        let h = f.glyph('H').unwrap();
        // an H: two upright strokes and a bar, as tall as a capital is in 16 rows
        let on = |x: u32, y: u32| h.cov[(y * h.w + x) as usize] >= 112;
        let rows_lit: Vec<u32> = (0..16).filter(|&y| (0..h.w).any(|x| on(x, y))).collect();
        assert!(rows_lit.len() >= 10 && rows_lit.len() <= 14 && rows_lit[0] == 0, "{rows_lit:?}");
        // a descender reaches the last row; a capital with an accent keeps it, in the rows
        let g = f.glyph('g').unwrap();
        assert!((0..g.w).any(|x| g.cov[(15 * g.w + x) as usize] >= 112));
        let u = f.glyph('Ü').unwrap();
        let first = (0..16).find(|&y| (0..u.w).any(|x| u.cov[(y * u.w + x) as usize] >= 112)).unwrap();
        let gap_row = (first..16).find(|&y| (0..u.w).all(|x| u.cov[(y * u.w + x) as usize] < 112));
        assert!(gap_row.is_some_and(|r| r < 6), "the dots above the U, apart from it");
        assert!(rows_lit.iter().any(|&y| (0..h.w).all(|x| on(x, y))), "the bar");
        let top = rows_lit[0];
        assert!((0..h.w / 3).any(|x| on(x, top)) && (2 * h.w / 3..h.w).any(|x| on(x, top)), "a stroke at either side");
        assert!(!on(h.w / 2, top), "and nothing between them at the top");
        // letters it has, small ones as their capitals where it has none, a gap of a dot
        assert!(f.coverage("12 Hauptbahnhof") > 0.99);
        assert_eq!(f.gap, 1);
        assert!(f.space >= 2);
        // more rows: larger letters
        let big = DotFont::from_vector(HANKEN, 0, 32, &['H']).unwrap();
        assert!(big.glyph('H').unwrap().w > h.w);
        // a sign of seven rows writes capitals: an H fills them, small letters are capitals
        let seven = DotFont::from_vector(HANKEN, 0, 7, &['H', 'a', 'A', 'ü', 'Ü', 'g', 'G']).unwrap();
        let h7 = seven.glyph('H').unwrap();
        assert_eq!(seven.lit_rows(h7), Some((0, 6)));
        assert_eq!(seven.glyph('a').unwrap().cov, seven.glyph('A').unwrap().cov);
        assert_eq!(seven.glyph('ü').unwrap().cov, seven.glyph('Ü').unwrap().cov);
        // and a font of more rows made smaller than that: the same
        let shrunk = f.resized(7);
        assert_eq!(shrunk.rows, 7);
        assert_eq!(shrunk.lit_rows(shrunk.glyph('H').unwrap()), Some((0, 6)));
        assert_eq!(shrunk.glyph('g').unwrap().cov, shrunk.glyph('G').unwrap().cov);
        assert!(DotFont::from_vector(b"no font", 0, 16, &['H']).is_err());
    }

    #[test]
    fn a_vector_font_on_a_display_is_drawn_in_its_dots() {
        let f = DotFont::from_vector(HANKEN, 0, 7, &['E', 'I']).unwrap();
        // an Annax display: 7 rows of 3x3 dots in 4 px cells, a line of 30 px
        let grid = DotGrid { pitch: 4, dot: 3, phase_x: 1, phase_y: 1, rows: 7, smooth: false };
        let a = f.to_atlas(&grid, 30, "Hanken");
        assert_eq!(a.font.height, 30);
        let e = lit(&a, 'E');
        assert!(!e.is_empty());
        // every lit pixel is a dot's pixel (none in a seam), each fully lit
        assert!(e.iter().all(|&(x, y)| x % 4 != 0 && y % 4 != 0 && y < 29), "{e:?}");
        let g = a.font.chars.iter().find(|g| g.ch == 'E').unwrap();
        assert!(e.iter().all(|&(x, y)| a.alpha[(((g.y as u32 + y) * a.width + g.x0 as u32 + x) * 4 + 3) as usize] == 255));
        // the same dots as the font has, three by three
        let dots: usize = f.glyph('E').unwrap().cov.iter().filter(|c| **c >= 112).count();
        assert_eq!(e.len(), dots * 9);
        // a matrix of 14 rows whose pixels are its LEDs: the 7 rows twice
        let m = f.to_atlas(&DotGrid::plain(14), 14, "Hanken");
        assert_eq!(lit(&m, 'E').len(), dots * 4);
        // a font of more rows than the display: shrunk to them
        let tall = DotFont::from_vector(HANKEN, 0, 32, &['I']).unwrap();
        let small = tall.to_atlas(&DotGrid::plain(8), 8, "x");
        assert!(lit(&small, 'I').iter().all(|&(_, y)| y < 8));
        // bold: a dot wider; spacing: as asked
        let bold = f.styled(true, Some(3));
        assert_eq!(bold.glyph('I').unwrap().w, f.glyph('I').unwrap().w + 1);
        assert_eq!(bold.gap, 3);
        assert_eq!(bold.to_atlas(&grid, 30, "x").font.gap, 12);
    }

    #[test]
    fn the_names_of_a_font_file() {
        let dir = std::env::temp_dir().join(format!("omsi_dotfont_names_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let file = dir.join("sub").join("Hanken.ttf");
        std::fs::write(&file, HANKEN).unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        std::fs::write(dir.join("broken.otf"), "not a font").unwrap();
        let faces = vector_faces(&file);
        assert_eq!(faces.len(), 1);
        assert!(faces[0].family.starts_with("Hanken Grotesk"), "{faces:?}");
        assert!(faces[0].name.starts_with("Hanken Grotesk"), "{faces:?}");
        assert_eq!(faces[0].spec().file.as_deref(), Some(file.as_path()));
        assert!(vector_faces(&dir.join("broken.otf")).is_empty());
        assert_eq!(vector_font_files(&dir), vec![dir.join("broken.otf"), file.clone()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
