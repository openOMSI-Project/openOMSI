//! One RGBA picture holding every text line and icon drawn: filled on demand, packed in
//! shelves, uploaded by the region that changed, cleared when full.

use crate::text::{Fonts, Style, Weight};
use hashbrown::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Text(String, u32, Weight),
    /// A line in a `Style`: its slant, boldening and stretch in thousandths.
    Styled(String, u32, Weight, [i32; 3]),
    Icon(String, u32),
}

/// Where something is in the atlas: texture coordinates, its size in pixels, and for text
/// the baseline below its top.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Sprite {
    pub uv: [f32; 4],
    pub w: f32,
    pub h: f32,
    pub ascent: f32,
}

pub struct Atlas {
    pub size: u32,
    /// Straight-alpha RGBA (white glyphs and icons: the colour comes from the vertex).
    pub rgba: Vec<u8>,
    entries: HashMap<Key, (Sprite, u64)>,
    shelf_y: u32,
    shelf_h: u32,
    cursor_x: u32,
    dirty: Option<[u32; 4]>,
    frame: u64,
    /// Bumped whenever the atlas was cleared.
    pub generation: u64,
    /// A frame needed more room than there is: the atlas was cleared in the middle of it
    /// (what that frame had already drawn lost its pictures), so it grows at the next.
    overflowed: bool,
}

/// The biggest the atlas grows to (texels a side; every graphics chip takes it).
const MAX_SIZE: u32 = 4096;

impl Atlas {
    pub fn new(size: u32) -> Atlas {
        Atlas { size, rgba: vec![0; (size * size * 4) as usize], entries: HashMap::new(), shelf_y: 0, shelf_h: 0, cursor_x: 0, dirty: Some([0, 0, size, size]), frame: 0, generation: 0, overflowed: false }
    }

    /// Start of a frame: an atlas more than nine tenths full is cleared now, before
    /// anything of this frame was taken from it.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        // A screen of many pixels a point (a phone at 3x) draws every word and icon three
        // times as big: its frame did not fit, and whatever was drawn before the atlas ran
        // full lost its picture - the icons were missing on such phones. Twice the size then.
        if self.overflowed && self.size < MAX_SIZE {
            self.overflowed = false;
            self.size *= 2;
            self.rgba = vec![0; (self.size * self.size * 4) as usize];
            self.clear();
            return;
        }
        self.overflowed = false;
        if self.shelf_y + self.shelf_h > self.size * 9 / 10 {
            self.clear();
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.rgba.iter_mut().for_each(|b| *b = 0);
        self.shelf_y = 0;
        self.shelf_h = 0;
        self.cursor_x = 0;
        self.dirty = Some([0, 0, self.size, self.size]);
        self.generation += 1;
    }

    /// Everything to be sent again (to another GPU pipeline, or one made anew).
    pub fn mark_all_dirty(&mut self) {
        self.dirty = Some([0, 0, self.size, self.size]);
    }

    /// The region changed since the last call: x, y, w, h.
    pub fn take_dirty(&mut self) -> Option<[u32; 4]> {
        self.dirty.take()
    }

    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let (w, h) = (w + 1, h + 1);
        if w > self.size {
            return None;
        }
        if self.cursor_x + w > self.size {
            self.shelf_y += self.shelf_h;
            self.shelf_h = 0;
            self.cursor_x = 0;
        }
        if self.shelf_y + h > self.size {
            return None;
        }
        let at = (self.cursor_x, self.shelf_y);
        self.cursor_x += w;
        self.shelf_h = self.shelf_h.max(h);
        Some(at)
    }

    fn put(&mut self, key: Key, w: u32, h: u32, alpha: &[u8], ascent: f32) -> Sprite {
        let at = match self.alloc(w, h) {
            Some(a) => a,
            None => {
                self.overflowed = true;
                self.clear();
                match self.alloc(w, h) {
                    Some(a) => a,
                    None => return Sprite::default(),
                }
            }
        };
        for y in 0..h {
            for x in 0..w {
                let a = alpha[(y * w + x) as usize];
                let i = (((at.1 + y) * self.size + at.0 + x) * 4) as usize;
                self.rgba[i..i + 4].copy_from_slice(&[255, 255, 255, a]);
            }
        }
        let r = [at.0, at.1, w, h];
        self.dirty = Some(match self.dirty {
            None => r,
            Some(d) => {
                let (x0, y0) = (d[0].min(r[0]), d[1].min(r[1]));
                let (x1, y1) = ((d[0] + d[2]).max(r[0] + r[2]), (d[1] + d[3]).max(r[1] + r[3]));
                [x0, y0, x1 - x0, y1 - y0]
            }
        });
        let s = self.size as f32;
        let sprite = Sprite { uv: [at.0 as f32 / s, at.1 as f32 / s, (at.0 + w) as f32 / s, (at.1 + h) as f32 / s], w: w as f32, h: h as f32, ascent };
        self.entries.insert(key, (sprite, self.frame));
        sprite
    }

    /// A line of text at `px` pixels.
    pub fn text(&mut self, fonts: &Fonts, text: &str, px: f32, weight: Weight) -> Sprite {
        let key = Key::Text(text.to_string(), (px * 4.0).round() as u32, weight);
        if let Some(e) = self.entries.get_mut(&key) {
            e.1 = self.frame;
            return e.0;
        }
        let b = fonts.render(text, px, weight);
        self.put(key, b.w, b.h, &b.alpha, b.ascent)
    }

    /// A line of text at `px` pixels, stretched, boldened and slanted (`Style`).
    pub fn text_styled(&mut self, fonts: &Fonts, text: &str, px: f32, weight: Weight, style: Style) -> Sprite {
        let milli = |v: f32| (v * 1000.0).round() as i32;
        let key = Key::Styled(text.to_string(), (px * 4.0).round() as u32, weight, [milli(style.slant), milli(style.bold), milli(style.stretch)]);
        if let Some(e) = self.entries.get_mut(&key) {
            e.1 = self.frame;
            return e.0;
        }
        let b = fonts.render_styled(text, px, weight, style);
        self.put(key, b.w, b.h, &b.alpha, b.ascent)
    }

    /// An icon at `size` pixels (None: no such icon).
    pub fn icon(&mut self, name: &str, size: u32) -> Option<Sprite> {
        let key = Key::Icon(name.to_string(), size);
        if let Some(e) = self.entries.get_mut(&key) {
            e.1 = self.frame;
            return Some(e.0);
        }
        let a = crate::icons::rasterize(name, size)?;
        Some(self.put(key, size, size, &a, 0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_that_does_not_fit_makes_it_grow() {
        let mut a = Atlas::new(64);
        a.begin_frame();
        for i in 0..40 {
            a.put(Key::Icon(format!("i{i}"), 20), 20, 20, &[255; 400], 0.0);
        }
        a.begin_frame();
        assert_eq!(a.size, 128);
        assert_eq!(a.rgba.len(), 128 * 128 * 4);
    }

    #[test]
    fn packs_reuses_and_clears() {
        let fonts = Fonts::new();
        let mut a = Atlas::new(256);
        a.take_dirty();
        let s1 = a.text(&fonts, "08:59", 18.0, Weight::Bold);
        assert!(s1.w > 20.0 && a.take_dirty().is_some());
        assert_eq!(a.text(&fonts, "08:59", 18.0, Weight::Bold), s1);
        assert!(a.take_dirty().is_none());
        let i = a.icon("flag", 24).unwrap();
        assert_eq!((i.w, i.h), (24.0, 24.0));
        // filling it clears it and bumps the generation
        for k in 0..400 {
            a.text(&fonts, &format!("line {k}"), 18.0, Weight::Regular);
        }
        assert!(a.generation > 0);
    }
}
