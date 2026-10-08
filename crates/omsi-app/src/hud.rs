//! Text overlay (time, speed, line/next stop/delay, fps) drawn with an OMSI `.oft` font.

use omsi_content::font::FontAtlas;
use omsi_render::{Renderer, Scene, TextureId};
use std::sync::Arc;

pub struct Hud {
    font: Option<Arc<FontAtlas>>,
    texture: Option<TextureId>,
    width: u32,
    height: u32,
    last: Vec<String>,
    /// Draw scale (the fonts are large).
    scale: f32,
}

impl Hud {
    pub fn new(fonts: &mut omsi_sim::texttex::FontLibrary) -> Hud {
        let font = fonts.load("DIN Narrow").or_else(|| fonts.load("DIN_Narrow")).or_else(|| fonts.load("DIN"));
        Hud { font, texture: None, width: 900, height: 0, last: Vec::new(), scale: 0.45 }
    }

    /// Re-render when the lines changed and register the overlay for this frame. A line
    /// wider than the overlay is broken between words (squeezed into one row, a long
    /// message could not be read).
    pub fn update(&mut self, renderer: &Renderer, scene: &mut Scene, lines: &[String]) {
        if lines.is_empty() {
            // nothing to say: the frame's overlays start empty (the interface adds its own)
            scene.overlays.clear();
            scene.subpixel_overlays.clear();
            return;
        }
        let Some(font) = self.font.clone() else { return };
        let lh = font.font.height.max(8) as u32 + 4;
        if self.last != lines || self.texture.is_none() {
            let rows: Vec<String> = lines.iter().map(|l| legible(&font, l)).flat_map(|l| wrap(&font, &l, self.width as i32 - 4)).collect();
            let h = lh * rows.len().max(1) as u32;
            let mut img = vec![0u8; (self.width * h * 4) as usize];
            for (i, l) in rows.iter().enumerate() {
                let row = font.render(l, self.width, lh, false, [255, 255, 255]);
                compose_row(&mut img, &row, self.width, lh, i as u32 * lh);
            }
            let image = omsi_texture::Image { width: self.width, height: h, rgba: img, has_alpha: true };
            match self.texture {
                Some(t) if self.height == h => renderer.update_texture(scene, t, &image),
                _ => self.texture = Some(renderer.add_texture(scene, &image, false)),
            }
            self.height = h;
            self.last = lines.to_vec();
        }
        if let Some(t) = self.texture {
            let (w, h) = (self.width as f32 * self.scale, self.height as f32 * self.scale);
            scene.overlays.clear();
            scene.subpixel_overlays.clear();
            scene.overlays.push((t, [12.0, 12.0, 12.0 + w, 12.0 + h]));
        }
    }
}

/// Outline radius around the glyphs, in texture pixels (the overlay is drawn at 0.45, so
/// the one-pixel edge it had came out under half a pixel wide and white text vanished in a
/// bright sky), and the opacity of the plate behind each row.
const OUTLINE: i32 = 3;
const PLATE_ALPHA: f32 = 0.4;

/// One rendered text row (straight-alpha RGBA, `width` x `lh`) into the overlay at row
/// `y0`: a translucent dark plate as wide as the text, a dark outline and the glyphs on top.
fn compose_row(img: &mut [u8], row: &[u8], width: u32, lh: u32, y0: u32) {
    let (w, hh) = (width as i32, lh as i32);
    // the text moves right by the outline's width, so that its left edge has one too
    let mut shifted = vec![0u8; row.len()];
    for y in 0..hh {
        let (a, b) = (((y * w) * 4) as usize, (((y + 1) * w) * 4) as usize);
        let n = ((w - OUTLINE) * 4) as usize;
        shifted[a + (OUTLINE * 4) as usize..b].copy_from_slice(&row[a..a + n]);
    }
    let row = &shifted[..];
    let mut outline = vec![0u8; (width * lh) as usize];
    let mut right = -1;
    let disc: Vec<(i32, i32)> = (-OUTLINE..=OUTLINE).flat_map(|dy| (-OUTLINE..=OUTLINE).map(move |dx| (dx, dy))).filter(|(dx, dy)| dx * dx + dy * dy <= OUTLINE * OUTLINE + 1).collect();
    for y in 0..hh {
        for x in 0..w {
            let a = row[((y * w + x) * 4 + 3) as usize];
            if a == 0 {
                continue;
            }
            right = right.max(x);
            for (dx, dy) in &disc {
                let (ox, oy) = (x + dx, y + dy);
                if ox >= 0 && oy >= 0 && ox < w && oy < hh {
                    let o = &mut outline[(oy * w + ox) as usize];
                    *o = (*o).max(a);
                }
            }
        }
    }
    if right < 0 {
        return;
    }
    let plate_right = right + OUTLINE + 6;
    for y in 0..hh {
        for x in 0..w {
            let si = ((y * w + x) * 4) as usize;
            let mut a = if x <= plate_right { PLATE_ALPHA } else { 0.0 };
            let oa = outline[(y * w + x) as usize] as f32 / 255.0 * 0.9;
            a = oa + a * (1.0 - oa);
            let ga = row[si + 3] as f32 / 255.0;
            let out_a = ga + a * (1.0 - ga);
            if out_a <= 0.0 {
                continue;
            }
            // (the plate and the outline are black: only the glyphs bring colour)
            let di = (((y0 as i32 + y) * w + x) * 4) as usize;
            for c in 0..3 {
                img[di + c] = ((row[si + c] as f32 * ga) / out_a).round().min(255.0) as u8;
            }
            img[di + 3] = (out_a * 255.0).round() as u8;
        }
    }
}

/// `line` with the few characters the HUD font has no glyph for (DIN Narrow: `&` in line
/// names like "5 & 5N", `_` in chrono folder names) written with ones it has.
fn legible(font: &FontAtlas, line: &str) -> String {
    let has = |c: char| font.font.has_glyph(c);
    line.chars()
        .map(|c| match c {
            '&' if !has('&') => '+',
            '_' if !has('_') => ' ',
            c => c,
        })
        .collect()
}

/// `line` in rows no wider than `width` pixels, broken at spaces (a single word wider than
/// that keeps its own row).
fn wrap(font: &FontAtlas, line: &str, width: i32) -> Vec<String> {
    if font.text_width(line) <= width {
        return vec![line.to_string()];
    }
    let mut rows = Vec::new();
    let mut row = String::new();
    for word in line.split(' ') {
        if row.is_empty() {
            // a row does not start with the gap it was broken at
            row.push_str(word);
            continue;
        }
        let longer = format!("{row} {word}");
        if font.text_width(longer.trim_end()) > width {
            rows.push(std::mem::take(&mut row).trim_end().to_string());
            row.push_str(word);
        } else {
            row = longer;
        }
    }
    if !row.trim().is_empty() {
        rows.push(row.trim_end().to_string());
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every glyph 10 px wide with no gap.
    fn font() -> FontAtlas {
        let chars = "abcdefghijklmnopqrstuvwxyz:".chars().map(|ch| omsi_content::font::FontChar { ch, x0: 0, x1: 10, y: 0 }).collect();
        FontAtlas::new(omsi_content::font::Font { height: 12, gap: 0, chars, ..Default::default() }, 1, 1, vec![0; 4], vec![0; 4])
    }

    #[test]
    fn long_lines_break_between_words() {
        let f = font();
        assert_eq!(wrap(&f, "short line", 200), vec!["short line"]);
        // a space without a glyph counts as the gap (1 px)
        assert_eq!(wrap(&f, "no duty:   line five does not run", 100), vec!["no duty:", "line five", "does not", "run"]);
        assert_eq!(wrap(&f, "unbreakablewordhere ok", 100), vec!["unbreakablewordhere", "ok"]);
        for row in wrap(&f, "no duty: line five does not run on that date at all", 250) {
            assert!(f.text_width(&row) <= 250, "{row}");
        }
        assert_eq!(legible(&f, "line 5 & 5N: 1000_FPW"), "line 5 + 5N: 1000 FPW");
    }

    /// A glyph pixel stays white and opaque, is ringed by a dark outline several pixels
    /// wide, and a plate runs behind the row as far as the text goes.
    #[test]
    fn rows_get_an_outline_and_a_plate() {
        let (w, lh) = (40u32, 12u32);
        let mut row = vec![0u8; (w * lh * 4) as usize];
        let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;
        row[at(10, 6)..at(10, 6) + 4].copy_from_slice(&[255, 255, 255, 255]);
        let mut img = vec![0u8; (w * lh * 4) as usize];
        compose_row(&mut img, &row, w, lh, 0);
        let x = 10 + OUTLINE as u32;
        assert_eq!(&img[at(x, 6)..at(x, 6) + 4], &[255, 255, 255, 255]);
        let ring = &img[at(x + 2, 6)..at(x + 2, 6) + 4];
        assert!(ring[0] == 0 && ring[3] > 200, "{ring:?}");
        let plate = &img[at(x + 8, 1)..at(x + 8, 1) + 4];
        assert!(plate[0] == 0 && (90..120).contains(&plate[3]), "{plate:?}");
        assert_eq!(img[at(30, 6) + 3], 0);
    }
}
