//! The pixel canvas and the painter that draws laid-out boxes and text into it.

use super::*;

pub(crate) struct Canvas {
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) px: Vec<u8>,
}

impl Canvas {
    #[inline]
    pub(crate) fn blend(&mut self, x: i32, y: i32, c: [u8; 4], cov: f32) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let i = ((y as u32 * self.w + x as u32) * 4) as usize;
        // opaque colour over full coverage: a plain copy
        if c[3] == 255 && cov >= 0.999 {
            self.px[i..i + 4].copy_from_slice(&c);
            return;
        }
        let sa = c[3] as f32 / 255.0 * cov.clamp(0.0, 1.0);
        if sa <= 0.0 {
            return;
        }
        let d = &mut self.px[i..i + 4];
        match d[3] {
            // nothing below: the colour with the source alpha
            0 => {
                d[..3].copy_from_slice(&c[..3]);
                d[3] = (sa * 255.0).round().clamp(0.0, 255.0) as u8;
            }
            // opaque below: stays opaque, no division needed
            255 => {
                let inv = 1.0 - sa;
                for k in 0..3 {
                    d[k] = (c[k] as f32 * sa + d[k] as f32 * inv + 0.5) as u8;
                }
            }
            da8 => {
                let da = da8 as f32 / 255.0;
                let oa = sa + da * (1.0 - sa);
                for k in 0..3 {
                    let v = (c[k] as f32 * sa + d[k] as f32 * da * (1.0 - sa)) / oa;
                    d[k] = v.round().clamp(0.0, 255.0) as u8;
                }
                d[3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    /// Put one picture pixel `s` (RGBA, straight alpha) at an in-bounds position, `cov`
    /// scaling its alpha (the rounded corners). Opaque pixels are plain copies.
    #[inline]
    pub(crate) fn put(&mut self, x: i32, y: i32, s: &[u8], cov: f32) {
        let a = s[3];
        if a == 0 || cov <= 0.0 {
            return;
        }
        let i = ((y as u32 * self.w + x as u32) * 4) as usize;
        if cov >= 0.999 && (a == 255 || self.px[i + 3] == 0) {
            self.px[i..i + 4].copy_from_slice(&s[..4]);
            return;
        }
        self.blend(x, y, [s[0], s[1], s[2], a], cov);
    }

    /// A horizontal run of `x0..x1` on row `y` in one colour, full coverage, in bounds.
    #[inline]
    fn span(&mut self, y: i32, x0: i32, x1: i32, c: [u8; 4]) {
        if x0 >= x1 {
            return;
        }
        if c[3] == 255 {
            let row = y as usize * self.w as usize * 4;
            for p in self.px[row + x0 as usize * 4..row + x1 as usize * 4].chunks_exact_mut(4) {
                p.copy_from_slice(&c);
            }
        } else if c[3] > 0 {
            for x in x0..x1 {
                self.blend(x, y, c, 1.0);
            }
        }
    }

    pub(crate) fn fill(&mut self, r: [f32; 4], c: [u8; 4], radius: f32) {
        if !r.iter().all(|v| v.is_finite()) || !radius.is_finite() {
            return;
        }
        let (x0, y0) = (r[0].round() as i32, r[1].round() as i32);
        let (x1, y1) = ((r[0] + r[2]).round() as i32, (r[1] + r[3]).round() as i32);
        let (x0, x1) = (x0.max(0), x1.min(self.w as i32));
        let rad = radius.min(r[2] / 2.0).min(r[3] / 2.0).max(0.0);
        let round = rad > 0.5;
        // the corner zones: only there the coverage is below 1
        let (cl, cr) = {
            let (a, b) = (r[0] + rad, r[0] + r[2] - rad);
            (a.min(b), a.max(b))
        };
        let (ct, cb) = {
            let (a, b) = (r[1] + rad, r[1] + r[3] - rad);
            (a.min(b), a.max(b))
        };
        let mid_lo = ((cl.ceil() as i32).max(x0)).min(x1);
        let mid_hi = ((cr.floor() as i32).max(mid_lo)).min(x1);
        for y in y0.max(0)..y1.min(self.h as i32) {
            let ycorner = round && ((y as f32) < ct || (y as f32 + 1.0) > cb);
            if !ycorner {
                self.span(y, x0, x1, c);
                continue;
            }
            for x in x0..mid_lo {
                self.blend(x, y, c, corner_cov(r, rad, x, y));
            }
            self.span(y, mid_lo, mid_hi, c);
            for x in mid_hi..x1 {
                self.blend(x, y, c, corner_cov(r, rad, x, y));
            }
        }
    }
}

/// A rasterised glyph: coverage bytes and where its top-left lies relative to the pen.
struct GlyphBmp {
    w: u32,
    h: u32,
    ox: i32,
    oy: i32,
    cov: Vec<u8>,
}

/// Glyphs are drawn at a quarter pixel in both axes, so a page that repaints the same text
/// every frame rasterises each glyph once.
const SUB: f32 = 4.0;

/// (glyph, size, bold, face (0 Roboto, 1 the fallback), sub-pixel x, y)
type GlyphKey = (u16, u32, bool, u8, u8, u8);

thread_local! {
    static GLYPHS: std::cell::RefCell<HashMap<GlyphKey, Option<Arc<GlyphBmp>>>> = std::cell::RefCell::new(HashMap::new());
}

fn glyph_bmp(font: &FontRef<'static>, bold: bool, face: u8, id: ab_glyph::GlyphId, px: f32, sx: u8, sy: u8) -> Option<Arc<GlyphBmp>> {
    let key = (id.0, px.to_bits(), bold, face, sx, sy);
    GLYPHS.with(|g| {
        let mut g = g.borrow_mut();
        if let Some(e) = g.get(&key) {
            return e.clone();
        }
        if g.len() > 8192 {
            g.clear();
        }
        let sc = PxScale::from(px);
        let glyph = id.with_scale_and_position(sc, point(sx as f32 / SUB, sy as f32 / SUB));
        let bmp = font.outline_glyph(glyph).map(|o| {
            let b = o.px_bounds();
            let (w, h) = (b.width().ceil() as u32 + 1, b.height().ceil() as u32 + 1);
            let mut cov = vec![0u8; (w * h) as usize];
            o.draw(|gx, gy, c| {
                if gx < w && gy < h {
                    cov[(gy * w + gx) as usize] = (c.clamp(0.0, 1.0) * 255.0).round() as u8;
                }
            });
            Arc::new(GlyphBmp { w, h, ox: b.min.x.floor() as i32, oy: b.min.y.floor() as i32, cov })
        });
        g.insert(key, bmp.clone());
        bmp
    })
}

pub(crate) fn draw_text(cv: &mut Canvas, font: &FontRef<'static>, bold: bool, px: f32, x: f32, base_y: f32, color: [u8; 4], s: &str) {
    let sc = PxScale::from(px);
    let sf = font.as_scaled(sc);
    let mut cx = x;
    let mut prev = None;
    let fy = base_y.floor();
    let sy = (((base_y - fy) * SUB) as u8).min(SUB as u8 - 1);
    for ch in s.chars() {
        if ch == '\n' {
            continue;
        }
        face_for(font, bold, ch, |face, id, fi| {
        let sf = if fi == 0 { sf } else { face.as_scaled(sc) };
        if let Some((p, pf)) = prev {
            if pf == fi {
                cx += sf.kern(p, id);
            }
        }
        let fx = cx.floor();
        let sx = (((cx - fx) * SUB) as u8).min(SUB as u8 - 1);
        if let Some(g) = glyph_bmp(face, bold, fi, id, px, sx, sy) {
            let (bx, by) = (fx as i32 + g.ox, fy as i32 + g.oy);
            for gy in 0..g.h {
                let y = by + gy as i32;
                if y < 0 || y >= cv.h as i32 {
                    continue;
                }
                let row = &g.cov[(gy * g.w) as usize..((gy + 1) * g.w) as usize];
                for (gx, &v) in row.iter().enumerate() {
                    if v != 0 {
                        cv.blend(bx + gx as i32, y, color, v as f32 * (1.0 / 255.0));
                    }
                }
            }
        }
        cx += sf.h_advance(id);
        prev = Some((id, fi));
        });
    }
}

/// Coverage (0..1) of the pixel at (`x`, `y`) inside the rounded rectangle `r`.
fn corner_cov(r: [f32; 4], rad: f32, x: i32, y: i32) -> f32 {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let cx = px.max(r[0] + rad).min(r[0] + r[2] - rad);
    let cy = py.max(r[1] + rad).min(r[1] + r[3] - rad);
    let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
    (rad - d + 0.5).clamp(0.0, 1.0)
}

/// Draw `img` inside `clip` (rounded by `radius`), its top-left corner at (`ox`, `oy`),
/// tiled along the axes that repeat. Only the pixels of the clip are visited.
fn draw_tiles(cv: &mut Canvas, img: &Img, clip: [f32; 4], radius: f32, ox: i32, oy: i32, rep: (bool, bool)) {
    if !clip.iter().all(|v| v.is_finite()) || !radius.is_finite() {
        return;
    }
    let (iw, ih) = (img.w as i32, img.h as i32);
    let mut x0 = (clip[0].round() as i32).max(0);
    let mut y0 = (clip[1].round() as i32).max(0);
    let mut x1 = ((clip[0] + clip[2]).round() as i32).min(cv.w as i32);
    let mut y1 = ((clip[1] + clip[3]).round() as i32).min(cv.h as i32);
    if !rep.0 {
        x0 = x0.max(ox);
        x1 = x1.min(ox.saturating_add(iw));
    }
    if !rep.1 {
        y0 = y0.max(oy);
        y1 = y1.min(oy.saturating_add(ih));
    }
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let rad = radius.min(clip[2] / 2.0).min(clip[3] / 2.0).max(0.0);
    let round = rad > 0.5;
    let (cl, cr) = (clip[0] + rad, clip[0] + clip[2] - rad);
    let (ct, cb) = (clip[1] + rad, clip[1] + clip[3] - rad);
    let tx0 = (x0 - ox).rem_euclid(iw);
    for y in y0..y1 {
        let ty = (y - oy).rem_euclid(ih);
        let row = ty as usize * iw as usize * 4;
        let ycorner = round && ((y as f32) < ct || (y as f32 + 1.0) > cb);
        let mut tx = tx0;
        for x in x0..x1 {
            let i = row + tx as usize * 4;
            let cov = if ycorner && ((x as f32) < cl || (x as f32 + 1.0) > cr) { corner_cov(clip, rad, x, y) } else { 1.0 };
            cv.put(x, y, &img.rgba[i..i + 4], cov);
            tx += 1;
            if tx == iw {
                tx = 0;
            }
        }
    }
}

/// The `background-image` of `st` over `rect` (size, repeat and position as CSS has them).
pub(crate) fn paint_bg(cv: &mut Canvas, imgs: &ImageStore, rect: [f32; 4], radius: f32, st: &Style) {
    let Some(src) = &st.bg_img else { return };
    let (rw, rh) = (rect[2], rect[3]);
    if !rect.iter().all(|v| v.is_finite()) || rw < 1.0 || rh < 1.0 {
        return;
    }
    let Some((iw, ih)) = imgs.dims(src) else { return };
    let (iw, ih) = (iw as f32, ih as f32);
    let (tw, th) = match st.bg_size {
        BgSize::Auto => (iw, ih),
        BgSize::Cover => {
            let k = (rw / iw).max(rh / ih);
            (iw * k, ih * k)
        }
        BgSize::Contain => {
            let k = (rw / iw).min(rh / ih);
            (iw * k, ih * k)
        }
        BgSize::Dims(w, h) => match (w.map(|l| l.px(rw)), h.map(|l| l.px(rh))) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w * ih / iw),
            (None, Some(h)) => (h * iw / ih, h),
            (None, None) => (iw, ih),
        },
    };
    if !tw.is_finite() || !th.is_finite() {
        return;
    }
    let (tw, th) = ((tw.round() as i64).clamp(1, 16384) as u32, (th.round() as i64).clamp(1, 16384) as u32);
    let Some(tile) = imgs.scaled(src, tw, th) else { return };
    let ox = rect[0] + st.bg_pos[0].px(rw - tw as f32);
    let oy = rect[1] + st.bg_pos[1].px(rh - th as f32);
    if !ox.is_finite() || !oy.is_finite() {
        return;
    }
    draw_tiles(cv, &tile, rect, radius, ox.round() as i32, oy.round() as i32, st.bg_repeat);
}

/// An `<img>`: the picture stretched over the content box of the element.
fn paint_img(cv: &mut Canvas, lay: &Layouter, b: &LBox) {
    let n = &lay.dom.nodes[b.node];
    if n.tag != "img" || n.src.is_empty() {
        return;
    }
    let [pt, pr, pb, pl] = b.st.padding;
    let area = [b.rect[0] + pl, b.rect[1] + pt, b.rect[2] - pl - pr, b.rect[3] - pt - pb];
    if !area.iter().all(|v| v.is_finite()) || area[2] < 0.5 || area[3] < 0.5 {
        return;
    }
    let w = (area[2].round() as i64).clamp(1, 16384) as u32;
    let h = (area[3].round() as i64).clamp(1, 16384) as u32;
    let Some(pic) = lay.imgs.scaled(&n.src, w, h) else { return };
    draw_tiles(cv, &pic, area, b.st.radius, area[0].round() as i32, area[1].round() as i32, (false, false));
}

pub(crate) fn paint(cv: &mut Canvas, lay: &Layouter, b: &LBox) {
    if !b.st.hidden {
        if b.st.bg[3] > 0 {
            cv.fill(b.rect, b.st.bg, b.st.radius);
        }
        if b.st.bg_img.is_some() {
            paint_bg(cv, lay.imgs, b.rect, b.st.radius, &b.st);
        }
        paint_img(cv, lay, b);
    }
    for it in &b.items {
        match it {
            Item::Block(c) => paint(cv, lay, c),
            Item::Line(l) => {
                if b.st.hidden {
                    continue;
                }
                for li in &l.items {
                    let font = if li.bold { lay.bold } else { lay.reg };
                    let sf = font.as_scaled(PxScale::from(li.px));
                    let base = l.y + (l.h - (sf.ascent() - sf.descent())) / 2.0 + sf.ascent();
                    draw_text(cv, font, li.bold, li.px, l.x + li.dx, base, li.color, &li.text);
                }
            }
        }
    }
}