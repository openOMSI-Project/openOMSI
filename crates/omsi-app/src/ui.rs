//! The game's own interface, drawn over the picture in Roboto with a dark outline (the
//! HUD's `.oft` fonts are OMSI's and stay for the time and speed): the chat of a LAN
//! session, the name of what the cursor points at next to the cursor, and the other
//! players' name tags above their buses.
//!
//! Every text is rendered once into a small texture and kept while it is shown; the
//! overlays are rectangles in physical pixels (`Scene::overlays`).

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use omsi_render::{Renderer, Scene, TextureId};

/// Roboto (Apache 2.0), the interface font.
const ROBOTO: &[u8] = include_bytes!("../../../assets/fonts/Roboto-VariableFont_wdth,wght.ttf");

/// A rendered text: its texture and size in pixels.
#[derive(Clone, Copy)]
struct Label {
    tex: TextureId,
    w: u32,
    h: u32,
    used: u64,
}

/// Texts rendered into textures, kept while they are used.
pub struct TextCache {
    font: FontVec,
    labels: hashbrown::HashMap<(String, u32, [u8; 4]), Label>,
    frame: u64,
}

impl TextCache {
    pub fn new() -> Option<TextCache> {
        let font = FontVec::try_from_vec(ROBOTO.to_vec()).ok()?;
        Some(TextCache { font, labels: hashbrown::HashMap::new(), frame: 0 })
    }

    /// The texture of `text` at `px` pixels in `color` (alpha = opacity of the outline), and
    /// its size.
    fn label(&mut self, r: &Renderer, scene: &mut Scene, text: &str, px: u32, color: [u8; 4]) -> Label {
        // (in the interface's language: the menu, the notes, the windows)
        let text = &*omsi_ui::tr(text);
        let key = (text.to_string(), px, color);
        if let Some(l) = self.labels.get_mut(&key) {
            l.used = self.frame;
            return *l;
        }
        let img = render_text(&self.font, text, px as f32, color);
        let tex = r.add_texture(scene, &img, false);
        let l = Label { tex, w: img.width, h: img.height, used: self.frame };
        self.labels.insert(key, l);
        l
    }

    /// Text width in pixels, without rendering it.
    pub fn width(&self, text: &str, px: f32) -> f32 {
        let text = &*omsi_ui::tr(text);
        let mut w = 0.0;
        let mut prev: Option<(ab_glyph::GlyphId, *const FontVec)> = None;
        for c in text.chars() {
            let font = font_for(&self.font, c);
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
        w + outline_px(px) * 2.0 + 2.0
    }

    /// End of a frame: labels not used for a few seconds are released.
    pub fn end_frame(&mut self, r: &Renderer, scene: &mut Scene) {
        self.frame += 1;
        if self.frame % 120 == 0 {
            let old: Vec<_> = self.labels.iter().filter(|(_, l)| self.frame - l.used > 240).map(|(k, _)| k.clone()).collect();
            for k in old {
                if let Some(l) = self.labels.remove(&k) {
                    r.free_texture(scene, l.tex);
                }
            }
        }
    }
}

/// The outline (UIStroke) around the glyphs, in pixels.
fn outline_px(px: f32) -> f32 {
    (px / 9.0).clamp(1.0, 3.0)
}

/// The font that draws `c`: Roboto, else the system's font for the script (Chinese,
/// Japanese, Korean, Thai, Hindi - the menu was a column of boxes in those languages).
fn font_for(roboto: &FontVec, c: char) -> &FontVec {
    if omsi_ui::text::needs_fallback(roboto, c) {
        if let Some(f) = omsi_ui::text::fallback_font(c) {
            return f;
        }
    }
    roboto
}

/// `text` as straight-alpha RGBA: the glyphs in `color` over a dark outline.
fn render_text(font: &FontVec, text: &str, px: f32, color: [u8; 4]) -> omsi_texture::Image {
    let f = font.as_scaled(PxScale::from(px));
    let stroke = outline_px(px);
    let pad = stroke.ceil() as i32 + 1;
    let asc = f.ascent();
    let h = (asc - f.descent()).ceil() as i32 + pad * 2;
    // lay the glyphs out
    let mut glyphs: Vec<(&FontVec, ab_glyph::Glyph)> = Vec::new();
    let mut x = pad as f32;
    let mut prev: Option<(ab_glyph::GlyphId, *const FontVec)> = None;
    for c in text.chars() {
        let gf = font_for(font, c);
        let sf = gf.as_scaled(PxScale::from(px));
        let id = sf.glyph_id(c);
        if let Some((p, pf)) = prev {
            if std::ptr::eq(pf, gf) {
                x += sf.kern(p, id);
            }
        }
        glyphs.push((gf, id.with_scale_and_position(PxScale::from(px), ab_glyph::point(x, pad as f32 + asc))));
        x += sf.h_advance(id);
        prev = Some((id, gf as *const FontVec));
    }
    let w = (x.ceil() as i32 + pad).max(1);
    let (wu, hu) = (w as usize, h.max(1) as usize);
    let mut cov = vec![0f32; wu * hu];
    for (gf, g) in glyphs {
        if let Some(o) = gf.outline_glyph(g) {
            let b = o.px_bounds();
            o.draw(|gx, gy, c| {
                let xx = b.min.x as i32 + gx as i32;
                let yy = b.min.y as i32 + gy as i32;
                if xx >= 0 && yy >= 0 && (xx as usize) < wu && (yy as usize) < hu {
                    let i = yy as usize * wu + xx as usize;
                    cov[i] = (cov[i] + c).min(1.0);
                }
            });
        }
    }
    // the outline: the coverage grown by the stroke radius
    let r = stroke;
    let ri = r.ceil() as i32;
    let mut edge = vec![0f32; wu * hu];
    for y in 0..hu as i32 {
        for x in 0..wu as i32 {
            let mut m = 0f32;
            for dy in -ri..=ri {
                for dx in -ri..=ri {
                    let d = ((dx * dx + dy * dy) as f32).sqrt();
                    if d > r + 0.5 {
                        continue;
                    }
                    let (sx, sy) = (x + dx, y + dy);
                    if sx < 0 || sy < 0 || sx >= wu as i32 || sy >= hu as i32 {
                        continue;
                    }
                    let k = (r + 0.5 - d).clamp(0.0, 1.0);
                    m = m.max(cov[sy as usize * wu + sx as usize] * k);
                }
            }
            edge[y as usize * wu + x as usize] = m;
        }
    }
    let oa = color[3] as f32 / 255.0;
    let mut rgba = vec![0u8; wu * hu * 4];
    for i in 0..wu * hu {
        let a_text = cov[i];
        let a_edge = edge[i] * oa;
        let a = a_text + a_edge * (1.0 - a_text);
        if a <= 0.0 {
            continue;
        }
        for c in 0..3 {
            let v = (color[c] as f32 * a_text + 12.0 * a_edge * (1.0 - a_text)) / a;
            rgba[i * 4 + c] = v.round().clamp(0.0, 255.0) as u8;
        }
        rgba[i * 4 + 3] = (a * 255.0).round() as u8;
    }
    omsi_texture::Image { width: wu as u32, height: hu as u32, rgba, has_alpha: true }
}

// ---------------------------------------------------------------------------------------
// the chat

/// How many lines the chat shows while it is closed, and how many it keeps to scroll back.
const CHAT_SHOWN: usize = 8;
pub const CHAT_KEEP: usize = 200;

/// What the chat widget needs from the session each frame.
pub struct ChatView<'a> {
    /// Every line, oldest first ("Name: text" or "* notice").
    pub lines: &'a [String],
    /// The line being typed (the input box is open).
    pub typing: Option<&'a str>,
    /// Why the last line was not sent.
    pub error: Option<&'a str>,
}

/// The chat's own state: shown or hidden (V), the scroll position and whether the cursor is
/// over it.
#[derive(Default)]
pub struct ChatWidget {
    pub hidden: bool,
    /// Lines scrolled back from the newest.
    pub scroll: usize,
    /// The chat's box on the screen (physical pixels) as drawn last: hovering over it shows
    /// the input box, a click there opens it.
    pub rect: [f32; 4],
    pub hovered: bool,
    pub caret_t: f32,
}

impl ChatWidget {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.rect[0] && x <= self.rect[2] && y >= self.rect[1] && y <= self.rect[3]
    }

    /// The mouse wheel over the chat (or while typing) scrolls the history.
    pub fn wheel(&mut self, lines: usize, amount: f32) {
        let max = lines.saturating_sub(CHAT_SHOWN);
        let s = self.scroll as i32 + amount.round() as i32;
        self.scroll = s.clamp(0, max as i32) as usize;
    }
}

/// Everything the interface draws in a frame.
pub struct Frame<'a> {
    /// Physical pixels per logical one.
    pub scale: f32,
    pub width: f32,
    pub height: f32,
    pub cursor: (f32, f32),
    /// An OpenXR headset is drawing this frame.
    pub vr: bool,
    /// The name of what the cursor points at (a switch, a part), shown next to it.
    pub tooltip: Option<String>,
    /// The chat, when a LAN session runs and the chat is not switched off.
    pub chat: Option<ChatView<'a>>,
    /// What the driver has to act on (why the bus does not move, a passenger's wish, the
    /// change due, a service done), top left.
    pub notes: &'a [String],
    /// The frame rate, top right (the `show_fps` setting).
    pub fps: Option<f32>,
    /// The game stands paused.
    pub paused: bool,
    /// The game menu is open, with this line chosen (labels from `GAME_MENU`).
    pub menu: Option<(usize, &'a [(&'a str, &'a str)])>,
    /// The first line shown when a finger scrolled the menu (`App::menu_top`).
    pub menu_top: Option<f32>,
    /// The timetable window: its title and per stop (name, time, 0 served / 1 next / 2 ahead).
    pub timetable: Option<(String, Vec<(String, String, u8)>)>,
    /// The information bar along the top.
    pub info: Option<String>,
    /// A tutorial page: title, text, picture, page number and count.
    pub tutorial: Option<(&'a str, &'a str, Option<&'a std::path::Path>, usize, usize)>,
    /// Name tags: a screen position (the point above a bus), the name and a second line.
    pub tags: Vec<((f32, f32), String, String, f32)>,
}

pub struct Ui {
    pub text: TextCache,
    pub chat: ChatWidget,
    /// Where the game menu's lines were drawn this frame (physical pixels), for the mouse.
    pub menu_rects: Vec<[f32; 4]>,
    pub menu_scroll_thumb: Option<[f32; 4]>,
    pub menu_scroll_track: Option<[f32; 4]>,
    /// Overlay entries belonging to the game menu.
    pub menu_overlay_range: std::ops::Range<usize>,
    /// The pointer texture, positioned separately for each headset eye.
    pub vr_cursor_overlay: Option<usize>,
    pub vr_tooltip_overlay: Option<usize>,
    /// The first line of the menu shown (a long menu scrolls: `menu_rects[k]` is line
    /// `menu_start + k`).
    pub menu_start: usize,
    /// How many lines the menu shows at once, and how high one is (physical pixels): a
    /// finger's drag is turned into lines with it.
    pub menu_rows: usize,
    pub menu_row_h: f32,
    /// Pictures shown in the interface (a tutorial page's), by file.
    images: hashbrown::HashMap<std::path::PathBuf, Option<(TextureId, u32, u32)>>,
}

impl Ui {
    pub fn new() -> Option<Ui> {
        Some(Ui { text: TextCache::new()?, chat: ChatWidget::default(), menu_rects: Vec::new(), menu_scroll_thumb: None, menu_scroll_track: None, menu_overlay_range: 0..0, vr_cursor_overlay: None, vr_tooltip_overlay: None, menu_start: 0, menu_rows: 0, menu_row_h: 1.0, images: Default::default() })
    }

    /// Draw the frame's interface: its overlays go after the HUD's in `scene.overlays`.
    pub fn draw(&mut self, r: &Renderer, scene: &mut Scene, f: &Frame, dt: f32) {
        let s = f.scale.max(0.5);
        // --- name tags above the other players' buses
        for ((x, y), name, sub, alpha) in &f.tags {
            let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
            let l = self.text.label(r, scene, name, (19.0 * s) as u32, [255, 255, 255, 220]);
            let x0 = x - l.w as f32 * 0.5;
            let y0 = y - l.h as f32;
            if a > 0 {
                scene.overlays.push((l.tex, [x0, y0, x0 + l.w as f32, y0 + l.h as f32]));
                if !sub.is_empty() {
                    let m = self.text.label(r, scene, sub, (12.0 * s) as u32, [210, 225, 255, 200]);
                    let mx = x - m.w as f32 * 0.5;
                    scene.overlays.push((m.tex, [mx, y0 + l.h as f32 - 3.0 * s, mx + m.w as f32, y0 + l.h as f32 - 3.0 * s + m.h as f32]));
                }
            }
        }
        // --- the chat, top left under the HUD, as Roblox has it
        if let Some(c) = f.chat.as_ref().filter(|_| !self.chat.hidden) {
            let px = (17.0 * s) as u32;
            let lh = px as f32 * 1.35;
            let x0 = 14.0 * s;
            let y0 = 96.0 * s;
            let width = (460.0 * s).min(f.width * 0.5);
            let open = c.typing.is_some();
            let n = c.lines.len();
            let shown = CHAT_SHOWN.min(n);
            let end = n.saturating_sub(if open || self.chat.hovered { self.chat.scroll } else { 0 });
            let start = end.saturating_sub(shown);
            let box_h = lh * CHAT_SHOWN as f32 + lh * 1.6;
            self.chat.rect = [x0 - 6.0 * s, y0 - 6.0 * s, x0 + width, y0 + box_h];
            self.chat.hovered = self.chat.contains(f.cursor.0, f.cursor.1);
            let show_box = open || self.chat.hovered;
            // the lines, newest at the bottom of the history area
            let mut y = y0 + lh * (CHAT_SHOWN - (end - start)) as f32;
            for line in &c.lines[start..end] {
                let color = if line.starts_with("* ") { [255, 226, 140, 230] } else { [255, 255, 255, 230] };
                let text = clip_to(&self.text, line, px as f32, width);
                let l = self.text.label(r, scene, &text, px, color);
                scene.overlays.push((l.tex, [x0, y, x0 + l.w as f32, y + l.h as f32]));
                y += lh;
            }
            if show_box {
                // the input box: shown when the cursor is over the chat or it is typed into
                let by = y0 + lh * CHAT_SHOWN as f32 + lh * 0.2;
                let bh = lh * 1.25;
                let plate = self.text.plate(r, scene, 0);
                scene.overlays.push((plate, [x0 - 4.0 * s, by, x0 + width, by + bh]));
                self.chat.caret_t += dt;
                let caret = if open && (self.chat.caret_t % 1.0) < 0.55 { "|" } else { "" };
                let (text, color) = match c.typing {
                    Some(t) => (format!("{t}{caret}"), [255, 255, 255, 240]),
                    None => ("Click here or press / to chat".to_string(), [190, 190, 190, 200]),
                };
                let text = clip_left(&self.text, &text, px as f32, width - 10.0 * s);
                let l = self.text.label(r, scene, &text, px, color);
                let ty = by + (bh - l.h as f32) * 0.5;
                scene.overlays.push((l.tex, [x0 + 2.0 * s, ty, x0 + 2.0 * s + l.w as f32, ty + l.h as f32]));
                if self.chat.scroll > 0 && (open || self.chat.hovered) {
                    let m = self.text.label(r, scene, &format!("{} newer below", self.chat.scroll), (11.0 * s) as u32, [200, 200, 200, 200]);
                    scene.overlays.push((m.tex, [x0 + width - m.w as f32, by - m.h as f32, x0 + width, by]));
                }
            }
            if let Some(e) = c.error {
                let l = self.text.label(r, scene, &format!("not sent: {e}"), (12.0 * s) as u32, [255, 150, 150, 220]);
                let ey = y0 + box_h;
                scene.overlays.push((l.tex, [x0, ey, x0 + l.w as f32, ey + l.h as f32]));
            }
        } else {
            self.chat.hovered = false;
            self.chat.rect = [0.0; 4];
        }
        // --- notes, top left: white on a dark outline, one line each
        {
            let px = (16.0 * s) as u32;
            let x0 = 16.0 * s;
            // (below the on-screen buttons of a phone)
            let mut y = if crate::platform::touch_controls() { 80.0 * s } else { 14.0 * s };
            for n in f.notes.iter().filter(|n| !n.trim().is_empty()).take(8) {
                let text = clip_to(&self.text, n, px as f32, f.width * 0.6);
                let l = self.text.label(r, scene, &text, px, [255, 255, 255, 235]);
                scene.overlays.push((l.tex, [x0, y, x0 + l.w as f32, y + l.h as f32]));
                y += l.h as f32 + 2.0 * s;
            }
        }
        if let Some(fps) = f.fps {
            let l = self.text.label(r, scene, &format!("{fps:.0} fps"), (13.0 * s) as u32, [255, 255, 255, 200]);
            let x = f.width - l.w as f32 - 12.0 * s;
            scene.overlays.push((l.tex, [x, 10.0 * s, x + l.w as f32, 10.0 * s + l.h as f32]));
        }
        // --- the information bar, along the top in the middle
        if let Some(info) = f.info.as_ref() {
            let l = self.text.label(r, scene, info, (15.0 * s) as u32, [255, 255, 255, 0]);
            let pad = 10.0 * s;
            let (w, h) = (l.w as f32 + pad * 2.0, l.h as f32 + pad * 0.8);
            let x = (f.width - w) * 0.5;
            let y = 8.0 * s;
            let plate = self.text.plate(r, scene, 3);
            scene.overlays.push((plate, [x, y, x + w, y + h]));
            scene.overlays.push((l.tex, [x + pad, y + pad * 0.4, x + pad + l.w as f32, y + pad * 0.4 + l.h as f32]));
        }
        // --- the timetable window, on the right
        if let Some((title, rows)) = f.timetable.as_ref() {
            let px = (14.0 * s) as u32;
            let lh = px as f32 * 1.55;
            let w = (340.0 * s).min(f.width * 0.4);
            let shown = rows.len().min(((f.height * 0.7) / lh) as usize).max(1);
            // (from the stop before the next one on)
            let next = rows.iter().position(|r| r.2 == 1).unwrap_or(0);
            let first = next.saturating_sub(1).min(rows.len().saturating_sub(shown));
            let h = lh * (shown as f32 + 1.6);
            let x = f.width - w - 16.0 * s;
            let y = 60.0 * s;
            let plate = self.text.plate(r, scene, 3);
            scene.overlays.push((plate, [x, y, x + w, y + h]));
            let t = self.text.label(r, scene, &clip_to(&self.text, title, px as f32 * 1.1, w - 20.0 * s), (px as f32 * 1.1) as u32, [255, 255, 255, 0]);
            scene.overlays.push((t.tex, [x + 10.0 * s, y + 6.0 * s, x + 10.0 * s + t.w as f32, y + 6.0 * s + t.h as f32]));
            // (the names start after the widest time: "12:03-05" of a stop with a wait)
            let time_w = rows.iter().map(|r| self.text.width(&r.1, px as f32)).fold(0.0f32, f32::max).max(40.0 * s);
            let name_x = x + 10.0 * s + time_w + 12.0 * s;
            for (k, (name, time, state)) in rows.iter().skip(first).take(shown).enumerate() {
                let ry = y + lh * (k as f32 + 1.3);
                if *state == 1 {
                    let hl = self.text.plate(r, scene, 5);
                    scene.overlays.push((hl, [x + 4.0 * s, ry - 2.0 * s, x + w - 4.0 * s, ry + lh - 4.0 * s]));
                }
                let color = match state {
                    0 => [140, 140, 140, 0],
                    1 => [255, 200, 110, 0],
                    _ => [235, 235, 235, 0],
                };
                let tl = self.text.label(r, scene, time, px, color);
                scene.overlays.push((tl.tex, [x + 10.0 * s, ry, x + 10.0 * s + tl.w as f32, ry + tl.h as f32]));
                let nl = self.text.label(r, scene, &clip_to(&self.text, name, px as f32, x + w - name_x - 10.0 * s), px, color);
                scene.overlays.push((nl.tex, [name_x, ry, name_x + nl.w as f32, ry + nl.h as f32]));
            }
        }
        // --- a tutorial page, on the right
        if let Some((title, text, image, at, count)) = f.tutorial {
            let w = (420.0 * s).min(f.width * 0.42);
            let x = f.width - w - 16.0 * s;
            let mut y = 60.0 * s;
            let top = y;
            let pad = 14.0 * s;
            let mut items: Vec<(TextureId, [f32; 4])> = Vec::new();
            if let Some(p) = image {
                let entry = self.images.entry(p.to_path_buf()).or_insert_with(|| {
                    omsi_texture::decode_file(p).ok().map(|img| {
                        let (iw, ih) = (img.width, img.height);
                        (r.add_texture(scene, &img, false), iw, ih)
                    })
                });
                if let Some((tex, iw, ih)) = *entry {
                    let dw = w - pad * 2.0;
                    let dh = dw * ih as f32 / iw.max(1) as f32;
                    items.push((tex, [x + pad, y + pad, x + pad + dw, y + pad + dh]));
                    y += dh + pad;
                }
            }
            y += pad * 0.6;
            let tp = (18.0 * s) as u32;
            if !title.is_empty() {
                for line in wrap(&self.text, title, tp as f32, w - pad * 2.0) {
                    let l = self.text.label(r, scene, &line, tp, [255, 200, 110, 0]);
                    items.push((l.tex, [x + pad, y, x + pad + l.w as f32, y + l.h as f32]));
                    y += l.h as f32;
                }
                y += 6.0 * s;
            }
            let bp = (14.0 * s) as u32;
            let max_y = f.height - 60.0 * s;
            'text: for para in text.lines() {
                for line in wrap(&self.text, para, bp as f32, w - pad * 2.0) {
                    if y > max_y {
                        break 'text;
                    }
                    let l = self.text.label(r, scene, &line, bp, [235, 235, 235, 0]);
                    items.push((l.tex, [x + pad, y, x + pad + l.w as f32, y + l.h as f32]));
                    y += l.h as f32 * 0.95;
                }
                y += 5.0 * s;
            }
            let foot = format!("Page {} of {}   ·   Enter next   ·   Page Up back   ·   Ctrl+T hide", at + 1, count);
            let l = self.text.label(r, scene, &foot, (12.0 * s) as u32, [150, 150, 150, 0]);
            y += 4.0 * s;
            items.push((l.tex, [x + pad, y, x + pad + l.w as f32, y + l.h as f32]));
            y += l.h as f32 + pad;
            let plate = self.text.plate(r, scene, 3);
            scene.overlays.push((plate, [x, top, x + w, y]));
            scene.overlays.extend(items);
        }
        // --- paused
        if f.paused && f.menu.is_none() {
            let l = self.text.label(r, scene, "Paused  ·  P to go on", (18.0 * s) as u32, [255, 255, 255, 0]);
            let pad = 14.0 * s;
            let (w, h) = (l.w as f32 + pad * 2.0, l.h as f32 + pad);
            let x = (f.width - w) * 0.5;
            let y = f.height * 0.2;
            let plate = self.text.plate(r, scene, 3);
            scene.overlays.push((plate, [x, y, x + w, y + h]));
            scene.overlays.push((l.tex, [x + pad, y + pad * 0.5, x + pad + l.w as f32, y + pad * 0.5 + l.h as f32]));
        }
        // --- the game menu, in the middle over a dimmed picture
        self.menu_rects.clear();
        self.menu_scroll_thumb = None;
        self.menu_scroll_track = None;
        let menu_overlay_start = scene.overlays.len();
        if let Some((sel, items)) = f.menu {
            let dim = self.text.plate(r, scene, 6);
            scene.overlays.push((dim, [0.0, 0.0, f.width, f.height]));
            let w = 340.0 * s;
            let title_h = if f.vr { 50.0 * s } else { 56.0 * s };
            // as many lines as fit at a readable height; a longer menu scrolls (the wheel,
            // the arrow keys), the chosen line kept in view
            let room = f.height * (if f.vr { 0.60 } else { 0.92 }) - title_h - 16.0 * s;
            let row_h = (if f.vr { 40.0 } else { 44.0 }) * s;
            let row_h = row_h
                .min(room / items.len().max(1) as f32).max(34.0 * s);
            let rows = ((room / row_h).floor() as usize).clamp(1, items.len().max(1));
            let start = match (items.len() > rows, f.menu_top) {
                (false, _) => 0,
                (true, Some(top)) => (top.max(0.0).round() as usize).min(items.len() - rows),
                (true, None) => sel.saturating_sub(rows / 2).min(items.len() - rows),
            };
            self.menu_start = start;
            self.menu_rows = rows;
            self.menu_row_h = row_h;
            let px = ((17.0 * s).min(row_h * 0.45)) as u32;
            let h = title_h + row_h * rows as f32 + 16.0 * s;
            let x = (f.width - w) * 0.5;
            let y = (f.height - h) * 0.5;
            let panel = self.text.plate(r, scene, 3);
            scene.overlays.push((panel, [x, y, x + w, y + h]));
            let title = if items.first().is_some_and(|i| i.0 == "less") { "More" } else if f.paused { "Paused" } else { "Menu" };
            let t = self.text.label(r, scene, title, (22.0 * s) as u32, [255, 255, 255, 0]);
            scene.overlays.push((t.tex, [x + 20.0 * s, y + 16.0 * s, x + 20.0 * s + t.w as f32, y + 16.0 * s + t.h as f32]));
            // the scroll bar: where the lines shown lie in the whole menu
            if items.len() > rows {
                let track = [x + w - 7.0 * s, y + title_h, x + w - 3.0 * s, y + title_h + row_h * rows as f32 - 6.0 * s];
                self.menu_scroll_track = Some(track);
                let tp = self.text.plate(r, scene, 5);
                scene.overlays.push((tp, track));
                let th = track[3] - track[1];
                let t0 = track[1] + th * start as f32 / items.len() as f32;
                let t1 = track[1] + th * (start + rows) as f32 / items.len() as f32;
                let thumb = self.text.plate(r, scene, 4);
                let thumb_rect = [track[0], t0, track[2], t1];
                scene.overlays.push((thumb, thumb_rect));
                self.menu_scroll_thumb = Some(thumb_rect);
                let more = format!("{} of {}", sel + 1, items.len());
                let l = self.text.label(r, scene, &more, (12.0 * s) as u32, [150, 150, 150, 0]);
                scene.overlays.push((l.tex, [x + w - 16.0 * s - l.w as f32, y + 22.0 * s, x + w - 16.0 * s, y + 22.0 * s + l.h as f32]));
            }
            // (the line under the mouse is the one lit; the keyboard's choice only while the
            // mouse is off the lines - both lit at once read as two choices)
            let over = |rect: [f32; 4]| f.cursor.0 >= rect[0] && f.cursor.0 <= rect[2] && f.cursor.1 >= rect[1] && f.cursor.1 <= rect[3];
            // (the whole panel: in the gaps between the lines the keyboard's choice, the top
            // line, lit up for a moment as the mouse went down the list)
            let any_hovered = over([x, y, x + w, y + h]);
            for (k, (id, label)) in items.iter().enumerate().skip(start).take(rows) {
                let ry = y + title_h + row_h * (k - start) as f32;
                let rect = [x + 8.0 * s, ry, x + w - 12.0 * s, ry + row_h - 6.0 * s];
                let hovered = over(rect);
                let lit = hovered || (k == sel && !any_hovered);
                // a thin line above "More..." / "End the session": the everyday lines apart
                if matches!(*id, "more" | "quit") && k > start {
                    let sep = self.text.plate(r, scene, 5);
                    scene.overlays.push((sep, [x + 16.0 * s, ry - 3.5 * s, x + w - 20.0 * s, ry - 2.5 * s]));
                }
                if lit {
                    let hl = self.text.plate(r, scene, 4);
                    scene.overlays.push((hl, rect));
                }
                let color = if lit { [20, 20, 20, 0] } else if *id == "quit" { [240, 150, 140, 0] } else { [235, 235, 235, 0] };
                let l = self.text.label(r, scene, label, px, color);
                let ly = ry + (row_h - 6.0 * s - l.h as f32) * 0.5;
                scene.overlays.push((l.tex, [x + 20.0 * s, ly, x + 20.0 * s + l.w as f32, ly + l.h as f32]));
                self.menu_rects.push(rect);
            }
        }
        self.menu_overlay_range = menu_overlay_start..scene.overlays.len();
        // --- the mouse-over name, right of the cursor
        self.vr_tooltip_overlay = None;
        if let Some(t) = f.tooltip.as_ref().filter(|t| !t.is_empty()) {
            let l = self.text.label(r, scene, t, (14.0 * s) as u32, [255, 255, 255, 235]);
            let mut x = f.cursor.0 + 16.0 * s;
            let mut y = f.cursor.1 + 2.0 * s;
            if x + l.w as f32 > f.width {
                x = f.cursor.0 - 8.0 * s - l.w as f32;
            }
            if y + l.h as f32 > f.height {
                y = f.height - l.h as f32;
            }
            if f.vr { self.vr_tooltip_overlay = Some(scene.overlays.len()); }
            scene.overlays.push((l.tex, [x, y, x + l.w as f32, y + l.h as f32]));
        }
        if f.vr {
            let pointer = self.text.vr_pointer(r, scene);
            self.vr_cursor_overlay = Some(scene.overlays.len());
            scene.overlays.push((pointer, [0.0, 0.0, 7.0 * s, 7.0 * s]));
        } else {
            self.vr_cursor_overlay = None;
        }
        self.text.end_frame(r, scene);
    }
}

impl TextCache {
    /// A small white circle, centred on the point that receives the click.
    fn vr_pointer(&mut self, r: &Renderer, scene: &mut Scene) -> TextureId {
        let key = ("\u{0}vr_pointer_dot".to_string(), 0, [0, 0, 0, 0]);
        if let Some(label) = self.labels.get_mut(&key) {
            label.used = self.frame;
            return label.tex;
        }
        const W: usize = 32;
        const H: usize = 32;
        let mut rgba = vec![0u8; W * H * 4];
        for y in 0..H {
            for x in 0..W {
                let dx = x as f32 + 0.5 - W as f32 * 0.5;
                let dy = y as f32 + 0.5 - H as f32 * 0.5;
                let radius = (dx * dx + dy * dy).sqrt();
                let alpha = (16.0 - radius).clamp(0.0, 1.0);
                let white = radius < 13.0;
                let color = if white {
                    [255, 255, 255, (alpha * 255.0) as u8]
                } else {
                    [0, 0, 0, (alpha * 220.0) as u8]
                };
                rgba[(y * W + x) * 4..(y * W + x + 1) * 4].copy_from_slice(&color);
            }
        }
        let image = omsi_texture::Image { width: W as u32, height: H as u32, rgba, has_alpha: true };
        let tex = r.add_texture(scene, &image, false);
        self.labels.insert(key, Label { tex, w: W as u32, h: H as u32, used: self.frame });
        tex
    }

    /// A plate of one colour: 0 the chat's dark translucent input box, 1 the loading
    /// screen's bar track, 2 its fill.
    fn plate(&mut self, r: &Renderer, scene: &mut Scene, kind: u8) -> TextureId {
        let key = ("\u{0}plate".to_string(), kind as u32, [0, 0, 0, 0]);
        if let Some(l) = self.labels.get_mut(&key) {
            l.used = self.frame;
            return l.tex;
        }
        let rgba = match kind {
            1 => vec![255, 255, 255, 38],
            2 => vec![235, 238, 242, 255],
            // an opaque panel (translucent dark panels show the sky through them)
            3 => vec![22, 22, 22, 255],
            // the chosen line: the interface's accent
            4 => vec![232, 160, 48, 255],
            // a line the mouse is over, the next stop
            5 => vec![255, 255, 255, 28],
            // the picture dimmed behind the menu
            6 => vec![0, 0, 0, 120],
            _ => vec![10, 12, 16, 150],
        };
        let img = omsi_texture::Image { width: 1, height: 1, rgba, has_alpha: true };
        let tex = r.add_texture(scene, &img, false);
        self.labels.insert(key, Label { tex, w: 1, h: 1, used: u64::MAX / 2 });
        tex
    }
}

impl Ui {
    /// The loading screen over a plain dark picture: the map's name in the middle, a thin
    /// bar of how far the start area got under it, and one quiet line (what is being done).
    pub fn loading(&mut self, r: &Renderer, scene: &mut Scene, width: f32, height: f32, scale: f32, title: &str, caption: &str, progress: f32) {
        let s = scale.max(0.5);
        let t = self.text.label(r, scene, title, (30.0 * s) as u32, [255, 255, 255, 0]);
        let cy = height * 0.5;
        let tx = (width - t.w as f32) * 0.5;
        let ty = cy - t.h as f32 - 14.0 * s;
        scene.overlays.push((t.tex, [tx, ty, tx + t.w as f32, ty + t.h as f32]));
        let bw = (320.0 * s).min(width * 0.6);
        let bh = (3.0 * s).max(2.0);
        let bx = (width - bw) * 0.5;
        let by = cy + 4.0 * s;
        let track = self.text.plate(r, scene, 1);
        scene.overlays.push((track, [bx, by, bx + bw, by + bh]));
        let fill = self.text.plate(r, scene, 2);
        let p = progress.clamp(0.0, 1.0);
        if p > 0.0 {
            scene.overlays.push((fill, [bx, by, bx + bw * p, by + bh]));
        }
        if !caption.is_empty() {
            let c = self.text.label(r, scene, caption, (13.0 * s) as u32, [200, 204, 210, 0]);
            let cx = (width - c.w as f32) * 0.5;
            let cy2 = by + bh + 14.0 * s;
            scene.overlays.push((c.tex, [cx, cy2, cx + c.w as f32, cy2 + c.h as f32]));
        }
        self.text.end_frame(r, scene);
    }
}

/// `text` broken into lines of at most `width` pixels, between words.
fn wrap(tc: &TextCache, text: &str, px: f32, width: f32) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let try_line = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if tc.width(&try_line, px) > width && !line.is_empty() {
            out.push(std::mem::take(&mut line));
            line = word.to_string();
        } else {
            line = try_line;
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// `text` cut at the end to fit `width` pixels ("…").
fn clip_to(tc: &TextCache, text: &str, px: f32, width: f32) -> String {
    if tc.width(text, px) <= width {
        return text.to_string();
    }
    let mut t: String = text.to_string();
    while !t.is_empty() && tc.width(&format!("{t}…"), px) > width {
        t.pop();
    }
    format!("{t}…")
}

/// `text` cut at the start to fit (the end of what is being typed stays visible).
fn clip_left(tc: &TextCache, text: &str, px: f32, width: f32) -> String {
    let mut t: Vec<char> = text.chars().collect();
    while t.len() > 1 && tc.width(&t.iter().collect::<String>(), px) > width {
        t.remove(0);
    }
    t.into_iter().collect()
}

/// A chat line with its bad words starred out (rustrict: profanity, slurs and the usual
/// ways of writing around a filter, without a word list to keep).
pub fn filter_chat(text: &str) -> String {
    use rustrict::CensorStr;
    text.censor()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_renders_with_an_outline() {
        let f = FontVec::try_from_vec(ROBOTO.to_vec()).unwrap();
        let img = render_text(&f, "Savva: hi", 16.0, [255, 255, 255, 220]);
        assert!(img.width > 40 && img.height > 14);
        // white text and dark outline pixels are both there
        let px: Vec<&[u8]> = img.rgba.chunks(4).collect();
        assert!(px.iter().any(|p| p[3] > 200 && p[0] > 240));
        assert!(px.iter().any(|p| p[3] > 100 && p[0] < 40));
    }

    /// The Esc menu in Chinese, Korean and Thai: the system's fonts, not boxes.
    #[test]
    fn scripts_roboto_lacks_come_from_the_system() {
        let f = FontVec::try_from_vec(ROBOTO.to_vec()).unwrap();
        for t in ["继续", "繼續", "계속", "ดำเนินการต่อ"] {
            if t.chars().next().and_then(omsi_ui::text::fallback_font).is_none() {
                continue;
            }
            for c in t.chars() {
                let g = font_for(&f, c);
                assert!(!std::ptr::eq(g, &f) && g.glyph_id(c).0 != 0, "{t}: {c}");
            }
            let img = render_text(&f, t, 20.0, [255, 255, 255, 220]);
            let ink = img.rgba.chunks(4).filter(|p| p[3] > 128 && p[0] > 128).count();
            assert!(ink > 30, "{t}: {ink}");
        }
    }

    #[test]
    fn chat_filter_stars_out_swearing() {
        assert_ne!(filter_chat("you are a fucking idiot"), "you are a fucking idiot");
        assert_eq!(filter_chat("next stop Rathaus Spandau"), "next stop Rathaus Spandau");
    }
}
