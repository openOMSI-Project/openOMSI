//! `.oft` bitmap fonts (unit `mc_font`).

use omsi_cfg::CfgFile;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct FontChar {
    pub ch: char,
    pub x0: i32,
    pub x1: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Font {
    pub path: PathBuf,
    pub name: String,
    pub bitmap: String,
    pub alpha: String,
    pub height: i32,
    pub gap: i32,
    pub chars: Vec<FontChar>,
}

impl Font {
    /// One `.oft` may define several fonts (`[newfont]` blocks).
    pub fn load_all(path: &Path) -> Result<Vec<Font>, omsi_cfg::CfgError> {
        let f = CfgFile::read(path)?;
        let mut out: Vec<Font> = Vec::new();
        let mut r = f.reader();
        while let Some(k) = r.next_keyword() {
            match k.as_str() {
                "newfont" => {
                    let name = r.str().to_string();
                    let bitmap = r.str().to_string();
                    let alpha = r.str().to_string();
                    let height = r.i32();
                    let gap = r.i32();
                    // Most stock .oft files document the format with a dummy block whose
                    // lines are placeholders ("{name}", "{Höhe in Pixeln …}"), so they read
                    // as a font of no height. Loading those as fonts only gives a lookup
                    // something wrong to land on.
                    if height > 0 && !name.starts_with('{') {
                        out.push(Font { path: f.path.clone(), name, bitmap, alpha, height, gap, chars: Vec::new() });
                    }
                }
                "char" => {
                    let c = r.line();
                    let ch = c.chars().next().unwrap_or(' ');
                    let x0 = r.i32();
                    let x1 = r.i32();
                    let y = r.i32();
                    if let Some(font) = out.last_mut() {
                        font.chars.push(FontChar { ch, x0, x1, y });
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// Width of a space character in pixels: the font's own space glyph if defined,
    /// else the width of '0' (for digit-only fonts), else the font's first glyph
    /// (the font's default advance), or half the font height.
    pub fn space_width(&self) -> i32 {
        self.exact_glyph(' ')
            .map(|g| (g.x1 - g.x0).max(0))
            .or_else(|| self.exact_glyph('0').map(|g| (g.x1 - g.x0).max(0)))
            .or_else(|| self.chars.first().map(|g| (g.x1 - g.x0).max(0)))
            .unwrap_or_else(|| (self.height / 2).max(1))
    }

    /// The glyph Omsi.exe draws for `c` (0x5d66a4): the character itself (or the same
    /// character read in another code page: a font and the text it shows need not have
    /// been read in the same one - a Russian font's `Л` is the byte 0xCB, which a font file
    /// without other Cyrillic reads as `Ë`), else for a small Latin letter a-z its capital,
    /// else nothing: its lookup (0x5d660c) gives -1 and the text is drawn and measured
    /// without it (its callers skip a negative index), only the font's gap moves on.
    /// Whitespace the font lacks is still given a width ([`Font::space_width`]) so words
    /// keep their gaps - the font's first glyph, drawn in its place before, put a `|` in
    /// front of the MAN Lion's City's odometer (#360).
    pub fn glyph(&self, c: char) -> Option<&FontChar> {
        if c.is_whitespace() {
            return self.exact_glyph(' ').or_else(|| {
                omsi_cfg::codepage::char_variants(c)
                    .into_iter()
                    .find_map(|v| self.chars.iter().find(|g| g.ch == v))
            });
        }
        self.exact_glyph(c)
            .or_else(|| c.is_ascii_lowercase().then(|| self.exact_glyph(c.to_ascii_uppercase())).flatten())
    }

    /// Whether the font has a glyph of its own for `c`.
    pub fn has_glyph(&self, c: char) -> bool {
        self.exact_glyph(c).is_some()
    }

    fn exact_glyph(&self, c: char) -> Option<&FontChar> {
        self.chars.iter().find(|g| g.ch == c).or_else(|| {
            omsi_cfg::codepage::char_variants(c)
                .into_iter()
                .find_map(|v| self.chars.iter().find(|g| g.ch == v))
        })
    }

    /// A text's width as Omsi.exe measures it (0x5d6c00, the scripts' `TextLength` too):
    /// its glyphs' widths and the font's gap between each two of them.
    pub fn text_width(&self, text: &str) -> i32 {
        let n = text.chars().count() as i32;
        let w: i32 = text
            .chars()
            .map(|c| {
                if c.is_whitespace() {
                    self.space_width()
                } else {
                    self.glyph(c).map(|g| (g.x1 - g.x0).max(0)).unwrap_or(0)
                }
            })
            .sum();
        w + (n - 1).max(0) * self.gap
    }
}

/// Horizontal placement of a text in its texture (`[texttexture_enh]` orientation and
/// grid; a plain `[texttexture]` is centred).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextAlign {
    /// 0 and 4 centred over the letters (the gap behind the last one left out), leaning left
    /// when the centre falls between two pixels; 1 left, 2 right, 3 centred and rounded,
    /// 5 centred leaning right.
    pub orientation: i32,
    /// The text starts on a multiple of this many pixels (0 and 1: anywhere).
    pub grid: i32,
}

impl Default for TextAlign {
    fn default() -> Self {
        TextAlign { orientation: 0, grid: 1 }
    }
}

impl TextAlign {
    /// Left edge of a text `advance` pixels wide (gaps included) in a `width`-pixel texture.
    pub fn offset(&self, width: i32, advance: i32, gap: i32) -> i32 {
        // "over the spacing": the gap after the last letter is not part of the text
        let visible = (advance - gap.max(0)).max(0);
        let x = match self.orientation {
            1 => 0.0,
            2 => (width - visible) as f32,
            3 => ((width - visible) as f32 / 2.0).round(),
            5 => ((width - visible) as f32 / 2.0).ceil(),
            // a plain [texttexture] (0) is centred like 4: over the letters without the gap
            // behind the last one, halved downwards. The LiAZ 5292's line display maps its
            // three digit cells at u 0.078/0.306/0.535 of a 166 px texture (13, 51, 89 px)
            // and its letter cell at u 0.554 of the 512 px one (283.6 px) - exactly where
            // "092 " and "092D" land this way; centring the gap too put every cell 5-6 px
            // (2 px) to the left, the "0" lost its left side and read as "D92".
            _ => ((width - visible) as f32 / 2.0).floor(),
        };
        let g = self.grid.max(1) as f32;
        let x = match self.orientation {
            2 | 5 => (x / g).ceil() * g,
            3 => (x / g).round() * g,
            _ => (x / g).floor() * g,
        };
        (x as i32).max(0)
    }
}

/// A loaded font with its glyph bitmaps, ready to draw text into RGBA images.
pub struct FontAtlas {
    pub font: Font,
    pub width: u32,
    pub height: u32,
    /// Colour bitmap (RGBA8) and alpha bitmap (RGBA8; the red channel is the coverage).
    pub color: Vec<u8>,
    pub alpha: Vec<u8>,
}

impl FontAtlas {
    /// `color`/`alpha` are the decoded font bitmaps (same size). When the font has no
    /// separate colour bitmap, pass the alpha image for both.
    pub fn new(font: Font, width: u32, height: u32, color: Vec<u8>, alpha: Vec<u8>) -> FontAtlas {
        FontAtlas { font, width, height, color, alpha }
    }

    /// Pixel width of `text` in this font (glyph advances including the gap after each).
    pub fn text_width(&self, text: &str) -> i32 {
        text.chars()
            .map(|ch| {
                let w = if ch.is_whitespace() {
                    self.font.space_width()
                } else {
                    self.font.glyph(ch).map(|g| (g.x1 - g.x0).max(0)).unwrap_or(0)
                };
                w + self.font.gap
            })
            .sum()
    }

    /// Render `text` centred into a `w`×`h` RGBA image (a text wider than the image is
    /// clipped at its edge, as OMSI's text textures are).
    /// `full_color` uses the font's colour bitmap, otherwise glyphs are filled with `rgb`;
    /// the alpha channel holds the coverage.
    pub fn render(&self, text: &str, w: u32, h: u32, full_color: bool, rgb: [u8; 3]) -> Vec<u8> {
        self.render_aligned(text, w, h, full_color, rgb, TextAlign::default())
    }

    /// `render` with the horizontal placement of `[texttexture_enh]`.
    pub fn render_aligned(&self, text: &str, w: u32, h: u32, full_color: bool, rgb: [u8; 3], align: TextAlign) -> Vec<u8> {
        // '@' breaks the text into lines, one glyph height each, from the top: the SD202's
        // matrix hands over "   NORDSPITZE   @   BAUERNHOF    @NORDSP.BAUERNH. " for a
        // 512x128 texture whose meshes map the lines separately. Drawn as one line and
        // squeezed to fit, that came out as a row of unreadable dots.
        if text.contains('@') {
            let glyph_h = self.font.height.max(1) as u32;
            let mut out = vec![0u8; (w * h * 4) as usize];
            let lines: Vec<&str> = text.split('@').collect();
            let natural_extent = glyph_h.saturating_mul(lines.len() as u32);
            let pitch = if natural_extent > h {
                let visible = self.visible_glyph_height().max(1);
                (h / lines.len() as u32).max(visible)
            } else { glyph_h };
            let extent = pitch.saturating_mul(lines.len().saturating_sub(1) as u32) + glyph_h;
            let top = if extent > h { 0 } else { (h - extent) / 2 };
            for (i, line) in lines.iter().enumerate() {
                let y0 = top + i as u32 * pitch;
                self.render_unscaled_at(line, w, h, full_color, rgb, align, y0 as i32, &mut out);
            }
            return out;
        }
        // (a text wider than the texture runs off its edge, as Omsi.exe draws it: the
        // start is not left of the texture and the rest is clipped, 0x5fb79c / 0x5d67bc)
        self.render_unscaled(text, w, h, full_color, rgb, align)
    }

    /// `render_aligned` in a font that is not the display's own (a display font the player
    /// chose): every line keeps the place a line of the display's own font has - `line_h`
    /// pixels high, the block of lines centred as `render_aligned` centres it - and this
    /// font's letters are scaled to fill it ([`fit_scale`]: whole times for a font that is
    /// smaller, so that a pixel font stays crisp; shrunk only when it is taller than the line).
    /// A line that would then run off the texture's edge is drawn smaller, as a real sign
    /// writes a long destination in a narrower size.
    #[allow(clippy::too_many_arguments)]
    pub fn render_fitted(&self, text: &str, w: u32, h: u32, full_color: bool, rgb: [u8; 3], align: TextAlign, line_h: u32) -> Vec<u8> {
        let mut out = vec![0u8; (w * h * 4) as usize];
        let line_h = line_h.max(1);
        let lines: Vec<&str> = text.split('@').collect();
        let block = line_h as i32 * lines.len() as i32;
        // (one line is centred as `render_unscaled` centres its glyphs, rounding to nought;
        // a block of lines from its top as `render_aligned` places them)
        let top = if lines.len() == 1 { (h as i32 - block) / 2 } else { (h as i32 - block).max(0) / 2 };
        let scale = fit_scale(line_h as f32, self.font.height.max(1) as f32);
        for (i, line) in lines.iter().enumerate() {
            let y0 = top + i as i32 * line_h as i32;
            if y0 >= h as i32 {
                break;
            }
            self.draw_line_scaled(line, &mut out, w, h, y0, line_h, scale, full_color, rgb, align);
        }
        out
    }

    /// One line of `render_fitted` into `out` (`w` x `h`), in the slot of `line_h` rows from
    /// row `slot`.
    #[allow(clippy::too_many_arguments)]
    fn draw_line_scaled(&self, line: &str, out: &mut [u8], w: u32, h: u32, slot: i32, line_h: u32, scale: f32, full_color: bool, rgb: [u8; 3], align: TextAlign) {
        let advance = self.text_width(line);
        let gap = self.font.gap;
        let visible = (advance - gap.max(0)).max(0);
        if visible <= 0 {
            return;
        }
        // too wide for the texture at this size: as large as fits
        let mut s = scale;
        if visible as f32 * s > w as f32 {
            s = fit_scale(w as f32, visible as f32).min(s);
        }
        // the line in the font's own size, from its left edge
        let (tw, th) = (advance.max(1) as u32, self.font.height.max(1) as u32);
        let src = self.render_unscaled(line, tw, th, full_color, rgb, TextAlign { orientation: 1, grid: 1 });
        let dw = (tw as f32 * s).round().max(1.0) as i32;
        let dh = (th as f32 * s).round().max(1.0) as i32;
        let x0 = align.offset(w as i32, dw, (gap as f32 * s).round() as i32);
        let y0 = slot + (line_h as i32 - dh) / 2;
        resample_into(&src, tw, th, out, w, h, x0, y0, dw, dh);
    }

    fn visible_glyph_height(&self) -> u32 {
        let mut max_rows = 0u32;
        for glyph in &self.font.chars {
            let y0 = glyph.y.max(0) as u32;
            let y1 = (glyph.y + self.font.height).min(self.height as i32).max(0) as u32;
            let x0 = glyph.x0.max(0) as u32;
            let x1 = glyph.x1.min(self.width as i32).max(0) as u32;
            let mut first = None;
            let mut last = None;
            for y in y0..y1 {
                if (x0..x1).any(|x| self.alpha[((y * self.width + x) * 4) as usize] != 0) {
                    first.get_or_insert(y);
                    last = Some(y);
                }
            }
            if let (Some(first), Some(last)) = (first, last) { max_rows = max_rows.max(last - first + 1); }
        }
        max_rows
    }

    fn render_unscaled(&self, text: &str, w: u32, h: u32, full_color: bool, rgb: [u8; 3], align: TextAlign) -> Vec<u8> {
        let mut out = vec![0u8; (w * h * 4) as usize];
        let glyph_h = self.font.height.max(1) as i32;
        let y0 = (h as i32 - glyph_h) / 2;
        self.render_unscaled_at(text, w, h, full_color, rgb, align, y0, &mut out);
        out
    }

    fn render_unscaled_at(&self, text: &str, w: u32, h: u32, full_color: bool, rgb: [u8; 3], align: TextAlign, y0: i32, out: &mut [u8]) {
        let glyph_h = self.font.height.max(1) as i32;
        let mut x = align.offset(w as i32, self.text_width(text), self.font.gap);
        for ch in text.chars() {
            if ch.is_whitespace() {
                x += self.font.space_width() + self.font.gap;
                continue;
            }
            let Some(g) = self.font.glyph(ch) else {
                x += self.font.gap;
                continue;
            };
            let gw = (g.x1 - g.x0).max(0);
            for gy in 0..glyph_h {
                let sy = g.y + gy;
                let dy = y0 + gy;
                if sy < 0 || sy >= self.height as i32 || dy < 0 || dy >= h as i32 {
                    continue;
                }
                for gx in 0..gw {
                    let sx = g.x0 + gx;
                    let dx = x + gx;
                    if sx < 0 || sx >= self.width as i32 || dx < 0 || dx >= w as i32 {
                        continue;
                    }
                    let si = ((sy as u32 * self.width + sx as u32) * 4) as usize;
                    let a = self.alpha[si];
                    if a == 0 {
                        continue;
                    }
                    let di = ((dy as u32 * w + dx as u32) * 4) as usize;
                    let (r, gcol, b) = if full_color { (self.color[si], self.color[si + 1], self.color[si + 2]) } else { (rgb[0], rgb[1], rgb[2]) };
                    // alpha-over compositing onto the transparent target
                    let af = a as f32 / 255.0;
                    let inv = 1.0 - af;
                    out[di] = (r as f32 * af + out[di] as f32 * inv) as u8;
                    out[di + 1] = (gcol as f32 * af + out[di + 1] as f32 * inv) as u8;
                    out[di + 2] = (b as f32 * af + out[di + 2] as f32 * inv) as u8;
                    out[di + 3] = out[di + 3].max(a);
                }
            }
            x += gw + self.font.gap;
        }
    }
}

/// How many times a font `font_h` pixels high is drawn to fill a line `line_h` high: as many
/// whole times as fit (a pixel font magnified by a whole number stays crisp - every pixel of
/// it becomes a square of pixels - where 1.5 times would smear every other row), and only a
/// font taller than the line shrunk to exactly its height (the one case where no whole
/// number fits; a smaller size of the same font is the better choice then, see
/// `omsi_sim::texttex::FontLibrary::display_atlas`).
pub fn fit_scale(line_h: f32, font_h: f32) -> f32 {
    if !(line_h > 0.0 && font_h > 0.0) {
        return 1.0;
    }
    let s = line_h / font_h;
    if s >= 1.0 {
        // (a hair under a whole number from rounding is still that number)
        (s + 1e-4).floor()
    } else {
        s
    }
}

/// `src` (`sw` x `sh` RGBA) scaled to `dw` x `dh` and laid over `out` (`w` x `h`) at (`x0`,
/// `y0`), clipped at its edges. Every pixel of the result is the average of the part of `src`
/// under it (weighted by its coverage, the colours by their alpha): for a whole-number
/// magnification exactly the source pixel, so a pixel font comes out with hard edges; shrunk,
/// the letters stay as heavy as they were instead of losing rows.
#[allow(clippy::too_many_arguments)]
pub fn resample_into(src: &[u8], sw: u32, sh: u32, out: &mut [u8], w: u32, h: u32, x0: i32, y0: i32, dw: i32, dh: i32) {
    if sw == 0 || sh == 0 || dw <= 0 || dh <= 0 {
        return;
    }
    let (fx, fy) = (sw as f32 / dw as f32, sh as f32 / dh as f32);
    // the source pixels under a destination pixel's span [a, b) and how much of each
    let span = |a: f32, b: f32, n: u32| -> Vec<(usize, f32)> {
        let mut v = Vec::new();
        let mut i = a.floor().max(0.0) as i64;
        while (i as f32) < b && i < n as i64 {
            let lo = a.max(i as f32);
            let hi = b.min(i as f32 + 1.0);
            if hi > lo {
                v.push((i as usize, hi - lo));
            }
            i += 1;
        }
        v
    };
    let cols: Vec<Vec<(usize, f32)>> = (0..dw).map(|x| span(x as f32 * fx, (x + 1) as f32 * fx, sw)).collect();
    for y in 0..dh {
        let oy = y0 + y;
        if oy < 0 || oy >= h as i32 {
            continue;
        }
        let rows = span(y as f32 * fy, (y + 1) as f32 * fy, sh);
        for (x, col) in cols.iter().enumerate() {
            let ox = x0 + x as i32;
            if ox < 0 || ox >= w as i32 {
                continue;
            }
            let (mut a, mut r, mut g, mut b, mut total) = (0.0f32, 0.0, 0.0, 0.0, 0.0);
            for &(sy, wy) in &rows {
                for &(sx, wx) in col {
                    let k = wx * wy;
                    let si = (sy * sw as usize + sx) * 4;
                    // (the colours as the text textures hold them: already laid over nothing,
                    // so weighted by their area alone)
                    a += src[si + 3] as f32 * k;
                    r += src[si] as f32 * k;
                    g += src[si + 1] as f32 * k;
                    b += src[si + 2] as f32 * k;
                    total += k;
                }
            }
            if a <= 0.0 || total <= 0.0 {
                continue;
            }
            let cover = (a / total).round().clamp(0.0, 255.0) as u8;
            if cover == 0 {
                continue;
            }
            let di = ((oy as u32 * w + ox as u32) * 4) as usize;
            let inv = 1.0 - cover as f32 / 255.0;
            let px = |c: f32, o: u8| (c / total + o as f32 * inv).round().clamp(0.0, 255.0) as u8;
            out[di] = px(r, out[di]);
            out[di + 1] = px(g, out[di + 1]);
            out[di + 2] = px(b, out[di + 2]);
            out[di + 3] = out[di + 3].max(cover);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_does_not_draw_visible_first_glyph() {
        // A font like MAN New Lion's City odometer font: first character is '|',
        // digits follow, and there is no space character defined in the font.
        let font = Font {
            path: PathBuf::new(),
            name: "LCD_Test".into(),
            bitmap: "lcd.bmp".into(),
            alpha: "lcd_alpha.bmp".into(),
            height: 10,
            gap: 1,
            chars: vec![
                FontChar { ch: '|', x0: 0, x1: 2, y: 0 },
                FontChar { ch: '0', x0: 2, x1: 10, y: 0 },
                FontChar { ch: '1', x0: 10, x1: 18, y: 0 },
            ],
        };
        // Alpha bitmap where '|' has opaque pixels
        let mut alpha = vec![0u8; 18 * 10 * 4];
        for y in 0..10 {
            for x in 0..2 {
                let idx = (y * 18 + x) * 4;
                alpha[idx] = 255;
            }
        }
        let atlas = FontAtlas::new(font.clone(), 18, 10, alpha.clone(), alpha);

        // Leading spaces (as used in odometer padding e.g. "  1") must not draw '|'
        let rendered = atlas.render_aligned("  1", 40, 10, false, [255, 255, 255], TextAlign { orientation: 1, grid: 1 });
        // The first 10 pixels horizontally (where spaces sit) must have 0 alpha!
        for y in 0..10 {
            for x in 0..10 {
                let idx = (y * 40 + x) * 4;
                assert_eq!(rendered[idx + 3], 0, "space pixel at ({x}, {y}) must be transparent");
            }
        }

        // glyph(' ') must not return the '|' character
        assert_eq!(font.glyph(' '), None);
        // space_width should fall back to '0' width (8)
        assert_eq!(font.space_width(), 8);
    }

    /// A font of one solid letter `w` x `h`, its glyph a checkerboard when `checker`.
    fn block_font(w: i32, h: i32, gap: i32, checker: bool) -> FontAtlas {
        let font = Font { name: format!("Block {h}"), height: h, gap, chars: vec![FontChar { ch: 'A', x0: 0, x1: w, y: 0 }], ..Default::default() };
        let alpha: Vec<u8> = (0..h).flat_map(|y| (0..w).flat_map(move |x| if !checker || (x + y) % 2 == 0 { [255u8; 4] } else { [0u8; 4] })).collect();
        FontAtlas::new(font, w as u32, h as u32, alpha.clone(), alpha)
    }

    /// The rows and columns with ink, and every alpha value drawn.
    fn ink(img: &[u8], w: u32, h: u32) -> (Vec<u32>, Vec<u32>, Vec<u8>) {
        let mut rows = Vec::new();
        let mut cols = Vec::new();
        let mut alphas = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let a = img[((y * w + x) * 4 + 3) as usize];
                if a > 0 {
                    if !rows.contains(&y) {
                        rows.push(y);
                    }
                    if !cols.contains(&x) {
                        cols.push(x);
                    }
                    if !alphas.contains(&a) {
                        alphas.push(a);
                    }
                }
            }
        }
        cols.sort();
        (rows, cols, alphas)
    }

    #[test]
    fn a_display_font_is_magnified_by_whole_numbers_and_shrunk_only_to_fit() {
        assert_eq!(fit_scale(16.0, 7.0), 2.0);
        assert_eq!(fit_scale(14.0, 7.0), 2.0);
        assert_eq!(fit_scale(21.0, 7.0), 3.0);
        assert_eq!(fit_scale(7.0, 7.0), 1.0);
        assert_eq!(fit_scale(13.0, 7.0), 1.0, "1.86 times would smear a pixel font");
        assert_eq!(fit_scale(8.0, 16.0), 0.5);
        assert_eq!(fit_scale(0.0, 7.0), 1.0);
    }

    #[test]
    fn a_smaller_pixel_font_fills_the_line_and_stays_crisp() {
        let a = block_font(5, 7, 1, true);
        let img = a.render_fitted("A", 40, 16, false, [255, 200, 0], TextAlign::default(), 16);
        let (rows, cols, alphas) = ink(&img, 40, 16);
        // twice the size, in the middle of the line, every pixel a whole 2x2 square
        assert_eq!(rows, (1..15).collect::<Vec<_>>());
        assert_eq!(cols, (15..25).collect::<Vec<_>>());
        assert_eq!(alphas, [255]);
        for y in 1..15u32 {
            for x in 15..25u32 {
                let lit = img[((y * 40 + x) * 4 + 3) as usize] > 0;
                assert_eq!(lit, ((x - 15) / 2 + (y - 1) / 2) % 2 == 0, "({x}, {y})");
            }
        }
    }

    #[test]
    fn a_taller_font_is_shrunk_into_the_line_without_overflowing() {
        let a = block_font(20, 32, 2, false);
        let img = a.render_fitted("A", 64, 16, false, [255, 255, 255], TextAlign::default(), 16);
        let (rows, cols, _) = ink(&img, 64, 16);
        assert_eq!(rows.len(), 16);
        assert_eq!(cols.len(), 10);
        // a line of two: each in its own slot of the texture, as the display's own font has them
        let small = block_font(5, 7, 1, false);
        let two = small.render_fitted("A@A", 20, 40, false, [255, 255, 255], TextAlign::default(), 16);
        let (rows, _, _) = ink(&two, 20, 40);
        let expect: Vec<u32> = (5..19).chain(21..35).collect();
        assert_eq!(rows, expect);
    }

    #[test]
    fn a_line_too_wide_at_that_size_is_drawn_smaller_to_fit() {
        let a = block_font(5, 7, 1, false);
        // "AAAAA" is 29 px wide in the font: twice that does not fit 40 px, once does
        let img = a.render_fitted("AAAAA", 40, 16, false, [255, 255, 255], TextAlign { orientation: 1, grid: 1 }, 16);
        let (rows, cols, _) = ink(&img, 40, 16);
        assert_eq!(rows.len(), 7);
        assert_eq!(cols.first().copied(), Some(0));
        assert!(cols.last().is_some_and(|x| *x < 40));
        // the bus's own placement: right aligned
        let right = a.render_fitted("A", 40, 16, false, [255, 255, 255], TextAlign { orientation: 2, grid: 1 }, 16);
        let (_, cols, _) = ink(&right, 40, 16);
        assert_eq!(cols.last().copied(), Some(39));
    }

    #[test]
    fn overflowing_multiline_text_keeps_the_last_line_visible() {
        let font = Font {
            path: PathBuf::new(),
            name: "overflow-test".into(),
            height: 6,
            gap: 0,
            chars: vec![FontChar { ch: 'A', x0: 0, x1: 1, y: 0 }],
            ..Default::default()
        };
        let mut alpha = vec![0u8; 6 * 6 * 4];
        for y in 1..5 {
            alpha[(y * 6 * 4) as usize] = 255;
        }
        let atlas = FontAtlas::new(font, 6, 6, alpha.clone(), alpha);
        let image = atlas.render("A@A@A", 1, 12, false, [255, 255, 255]);
        assert!(image[(10 * 4) + 3] != 0, "last line must not be pushed below the texture");
    }
}
