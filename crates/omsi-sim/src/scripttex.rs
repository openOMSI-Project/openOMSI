//! `[scripttexture]`: RGBA images the scripts draw into through the `ST*` callbacks
//! (matrix displays). Materials use them with `[useScriptTexture] n` or as a transparency
//! map `[matl_transmap] \S:n`.

use omsi_content::font::FontAtlas;
use std::sync::Arc;

pub struct ScriptTexture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Changed since the last upload.
    pub dirty: bool,
    pub locked: bool,
    /// `STFilter` was called after this texture was unlocked. OMSI then generates a mip
    /// chain, which keeps a distant matrix or IBIS screen stable instead of sampling its
    /// full-resolution pixels directly.
    pub mipmaps: bool,
    /// Current draw colour as set by `STSetColor` (r, g, b, a).
    pub color: [u8; 4],
}

impl ScriptTexture {
    pub fn new(width: i32, height: i32) -> ScriptTexture {
        let (w, h) = (width.max(1) as u32, height.max(1) as u32);
        ScriptTexture {
            width: w,
            height: h,
            rgba: vec![0; (w * h * 4) as usize],
            dirty: true,
            locked: false,
            mipmaps: false,
            color: [255, 255, 255, 255],
        }
    }

    pub fn clear(&mut self) {
        self.rgba.iter_mut().for_each(|b| *b = 0);
        self.dirty = true;
    }

    pub fn put(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let i = ((y as u32 * self.width + x as u32) * 4) as usize;
        self.rgba[i..i + 4].copy_from_slice(&c);
        self.dirty = true;
    }

    pub fn get(&self, x: i32, y: i32) -> [u8; 4] {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return [0; 4];
        }
        let i = ((y as u32 * self.width + x as u32) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    pub fn rect(&mut self, x1: i32, y1: i32, x2: i32, y2: i32) {
        let c = self.color;
        for y in y1.min(y2)..=y1.max(y2) {
            for x in x1.min(x2)..=x1.max(x2) {
                self.put(x, y, c);
            }
        }
    }

    /// Draw `text` with the font at (x, y) (top-left), `spacing` extra pixels between
    /// letters. Glyph coverage scales the colour's alpha; RGB is the draw colour.
    /// `STTextOut` as OMSI draws (0x7bb731, the original): `mode` bit 0 takes the colour
    /// from the font's colour bitmap (else the `STSetColor` colour), the alpha is the font's
    /// mask; `mode & 3 == 2` writes only the glyphs' covered pixels (a transparent
    /// background), any other mode the whole glyph cell.
    pub fn text_out(&mut self, atlas: &FontAtlas, x: i32, y: i32, spacing: i32, mode: u8, text: &str) {
        let glyph_h = atlas.font.height.max(1) as i32;
        let mut cx = x;
        let c = self.color;
        let from_font = mode & 1 != 0;
        let transparent = mode & 3 == 2;
        for ch in text.chars() {
            if ch.is_whitespace() {
                cx += atlas.font.space_width() + atlas.font.gap + spacing;
                continue;
            }
            let Some(g) = atlas.font.glyph(ch) else {
                cx += atlas.font.gap.max(1) + spacing;
                continue;
            };
            let gw = (g.x1 - g.x0).max(0);
            for gy in 0..glyph_h {
                let sy = g.y + gy;
                if sy < 0 || sy >= atlas.height as i32 {
                    continue;
                }
                for gx in 0..gw {
                    let sx = g.x0 + gx;
                    if sx < 0 || sx >= atlas.width as i32 {
                        continue;
                    }
                    let si = ((sy as u32 * atlas.width + sx as u32) * 4) as usize;
                    let cov = atlas.alpha[si];
                    if cov == 0 && transparent {
                        continue;
                    }
                    let rgb = if from_font && si + 2 < atlas.color.len() { [atlas.color[si], atlas.color[si + 1], atlas.color[si + 2]] } else { [c[0], c[1], c[2]] };
                    self.put(cx + gx, y + gy, [rgb[0], rgb[1], rgb[2], cov]);
                }
            }
            cx += gw + atlas.font.gap + spacing;
        }
    }

    /// Copy an image into the texture (top-left aligned).
    pub fn load(&mut self, w: u32, h: u32, rgba: &[u8]) {
        for y in 0..h.min(self.height) {
            for x in 0..w.min(self.width) {
                let si = ((y * w + x) * 4) as usize;
                self.put(x as i32, y as i32, [rgba[si], rgba[si + 1], rgba[si + 2], rgba[si + 3]]);
            }
        }
    }
}

/// Registered fonts of a host: index = `GetFontIndex` result.
#[derive(Default)]
pub struct FontTable {
    pub entries: Vec<(String, Option<Arc<FontAtlas>>)>,
}
