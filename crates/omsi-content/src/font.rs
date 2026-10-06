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
