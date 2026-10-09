//! The launcher's widgets: an immediate-mode toolkit on `omsi-ui`.
//!
//! Every frame the pages call the widgets with where they go and what they change; the
//! toolkit draws them, answers whether they were clicked, and keeps the little state a
//! widget needs between frames (hover and press animations, scroll positions, the open
//! dropdown, the focused text field) under an id made from the widget's name.
//!
//! Coordinates are logical pixels (points); `Painter::scale` rasterises text and icons for
//! the display. Clipping (a scrolling list) starts a new layer with its own rounded clip.

use glam::Vec2;
use hashbrown::HashMap;
use omsi_ui::paint::Align;
use omsi_ui::{Atlas, Color, Fonts, Layer, Painter, Rect, Vertex, Weight};

use super::theme::*;

pub type Id = u64;

pub fn id_of(s: &str) -> Id {
    // FNV-1a
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Keys the widgets care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Backspace,
    Delete,
    Enter,
    Escape,
    Tab,
    SelectAll,
    Copy,
    Paste,
    Cut,
}

/// What happened since the last frame.
#[derive(Default, Clone)]
pub struct Input {
    pub mouse: Vec2,
    pub down: bool,
    pub pressed: bool,
    pub released: bool,
    pub right_down: bool,
    pub right_pressed: bool,
    pub wheel: Vec2,
    pub text: String,
    pub keys: Vec<Key>,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// A key as it was pressed (for binding keys): winit's code name.
    pub raw_key: Option<winit::keyboard::KeyCode>,
    /// Binding modifiers at key press, independent of later releases before redraw.
    pub raw_chord: i32,
    pub double_click: bool,
    /// The wheel is a finger dragged over the screen: it scrolls, it never turns a slider or
    /// a time field under the finger.
    pub touch: bool,
}

impl Input {
    pub fn capture_key(&mut self, code: winit::keyboard::KeyCode) {
        // A modifier alone is not a binding. Do not let a later modifier event
        // overwrite a letter already waiting for the controls page.
        if Modifiers::bit(code).is_some() { return; }
        self.raw_key = Some(code);
        self.raw_chord = omsi_content::input::chord(self.shift, self.ctrl, self.alt);
    }

    pub fn binding_modifiers(&self) -> i32 {
        self.raw_chord
    }
}

/// Shift, Ctrl, Alt and the logo key as the launcher knows them. The window says when they
/// change (`ModifiersChanged`) - except on Android, whose winit backend never does: Shift or
/// Ctrl held on a phone's keyboard went unseen there, a key bound with Shift was saved as the
/// letter alone and Ctrl+V typed a "v" (#634). Until the window has said it once, the
/// modifier keys' own presses and releases tell what is held.
#[derive(Default, Clone, Copy)]
pub struct Modifiers {
    /// The modifier keys held, a bit each (left and right apart: one let go of while the
    /// other is still down keeps it held).
    keys: u8,
    /// What the window last said, once it has said anything.
    told: Option<winit::keyboard::ModifiersState>,
}

impl Modifiers {
    fn bit(code: winit::keyboard::KeyCode) -> Option<u8> {
        use winit::keyboard::KeyCode as K;
        let k = [K::ShiftLeft, K::ShiftRight, K::ControlLeft, K::ControlRight, K::AltLeft, K::AltRight, K::SuperLeft, K::SuperRight];
        k.iter().position(|c| *c == code).map(|i| 1 << i)
    }

    /// A key went down or up; true when it was a modifier key.
    pub fn key(&mut self, code: winit::keyboard::KeyCode, pressed: bool) -> bool {
        let Some(b) = Self::bit(code) else { return false };
        if pressed {
            self.keys |= b;
        } else {
            self.keys &= !b;
        }
        true
    }

    /// The window's `ModifiersChanged`.
    pub fn told(&mut self, m: winit::keyboard::ModifiersState) {
        self.told = Some(m);
    }

    /// The keyboard went to another window: what is let go of there is not held here.
    pub fn release_keys(&mut self) {
        self.keys = 0;
    }

    pub fn state(&self) -> winit::keyboard::ModifiersState {
        use winit::keyboard::ModifiersState as M;
        if let Some(m) = self.told {
            return m;
        }
        let mut m = M::empty();
        for (bits, flag) in [(0b11, M::SHIFT), (0b1100, M::CONTROL), (0b11_0000, M::ALT), (0b1100_0000, M::SUPER)] {
            if self.keys & bits != 0 {
                m |= flag;
            }
        }
        m
    }

    /// Ctrl, or the logo key (Cmd on a Mac): the one for copy and paste.
    pub fn command(&self) -> bool {
        let m = self.state();
        m.control_key() || m.super_key()
    }

    /// Into the widgets' input.
    pub fn apply(&self, input: &mut Input) {
        let m = self.state();
        input.shift = m.shift_key();
        input.ctrl = self.command();
        input.alt = m.alt_key();
    }
}

/// An open dropdown: its options are drawn last, over everything.
#[derive(Clone)]
struct Popup {
    id: Id,
    anchor: Rect,
    options: Vec<String>,
    selected: usize,
    scroll: f32,
    opened: f32,
    /// Filled when an option was clicked: read by `select` next frame.
    picked: Option<usize>,
    /// The scrollbar is being dragged: where in the thumb it was taken.
    drag: Option<f32>,
    /// Typed while the list is open: only the options with it in their name are shown (a
    /// map's many entry points, #747).
    query: String,
}

impl Popup {
    /// The options shown (their places in `options`): those with the typed text in them.
    fn shown(&self) -> Vec<usize> {
        let q = self.query.to_lowercase();
        (0..self.options.len()).filter(|&k| q.is_empty() || matches(&self.options[k], &q)).collect()
    }
}

pub fn matches(text: &str, q: &str) -> bool {
    text.to_lowercase().contains(q) || omsi_ui::tr(text).to_lowercase().contains(q)
}

/// A calendar dropdown for a date field.
#[derive(Clone)]
struct DatePopup {
    id: Id,
    anchor: Rect,
    year: i32,
    month: u32,
    /// The date the field shows (marked in the calendar).
    current: (i32, u32, u32),
    picked: Option<(i32, u32, u32)>,
    opened: f32,
}

pub struct Ui {
    pub fonts: Fonts,
    pub atlas: Atlas,
    pub input: Input,
    pub scale: f32,
    pub size: Vec2,
    pub time: f32,
    pub dt: f32,
    /// Layers: how they are seen, what is drawn, and with which texture (0: the atlas).
    layers: Vec<(Layer, Painter, usize)>,
    clip_stack: Vec<(Rect, f32)>,
    pub hot: Option<Id>,
    pub active: Option<Id>,
    pub focus: Option<Id>,
    /// The widget that took the mouse wheel this frame (the innermost scroll area).
    wheel_taken: bool,
    anims: HashMap<Id, f32>,
    pub scroll: HashMap<Id, f32>,
    popup: Option<Popup>,
    date_popup: Option<DatePopup>,
    /// Text fields: caret position (chars) and the time it last moved.
    caret: HashMap<Id, (usize, f32)>,
    /// Text fields: selection anchor and caret position (chars).
    selection: HashMap<Id, (usize, usize)>,
    /// Text fields: pending mouse position for placing the caret.
    text_click: HashMap<Id, f32>,
    pub cursor: winit::window::CursorIcon,
    pub clipboard_out: Option<String>,
    pub clipboard_in: Option<String>,
    /// Rects the mouse is over UI in (the rest of the window is the 3D showroom).
    pub over_ui: bool,
    tooltip: Option<(String, Vec2)>,
    /// (tests) Where each clickable widget was this frame, by id.
    #[cfg(test)]
    pub drawn: HashMap<Id, Rect>,
}

impl Ui {
    pub fn new() -> Ui {
        Ui {
            fonts: Fonts::new(),
            atlas: Atlas::new(2048),
            input: Input::default(),
            scale: 1.0,
            size: Vec2::new(1280.0, 800.0),
            time: 0.0,
            dt: 1.0 / 60.0,
            layers: Vec::new(),
            clip_stack: Vec::new(),
            hot: None,
            active: None,
            focus: None,
            wheel_taken: false,
            anims: HashMap::new(),
            scroll: HashMap::new(),
            popup: None,
            date_popup: None,
            caret: HashMap::new(),
            selection: HashMap::new(),
            text_click: HashMap::new(),
            cursor: winit::window::CursorIcon::Default,
            clipboard_out: None,
            clipboard_in: None,
            over_ui: false,
            tooltip: None,
            #[cfg(test)]
            drawn: HashMap::new(),
        }
    }

    /// Start a frame: the window's size in points and its scale.
    pub fn begin(&mut self, size: Vec2, scale: f32, dt: f32) {
        self.size = size;
        self.scale = scale;
        self.dt = dt.clamp(0.0, 0.1);
        self.time += self.dt;
        self.atlas.begin_frame();
        self.layers.clear();
        self.clip_stack.clear();
        self.hot = None;
        self.wheel_taken = false;
        self.cursor = winit::window::CursorIcon::Default;
        self.over_ui = false;
        self.tooltip = None;
        #[cfg(test)]
        self.drawn.clear();
        self.push_layer(Rect::new(0.0, 0.0, size.x, size.y), 0.0);
        // typing into an open dropdown searches it: the keys are the list's, not the page's
        if let Some(p) = self.popup.as_mut() {
            let before = p.query.clone();
            p.query.extend(self.input.text.chars().filter(|c| !c.is_control()));
            self.input.text.clear();
            let mut close = false;
            self.input.keys.retain(|k| match k {
                Key::Backspace => {
                    p.query.pop();
                    false
                }
                Key::Escape => {
                    close = p.query.is_empty();
                    p.query.clear();
                    false
                }
                Key::Enter => {
                    p.picked = p.shown().first().copied();
                    false
                }
                _ => true,
            });
            if p.query != before {
                p.scroll = 0.0;
            }
            if close {
                self.popup = None;
            }
        }
        // a click outside the open dropdown closes it (the click does nothing else)
        if self.input.pressed {
            if let Some(p) = &self.popup {
                if !popup_rect(p, self.size).contains(self.input.mouse) && !p.anchor.contains(self.input.mouse) && p.opened >= 1.0 {
                    self.popup = None;
                    self.input.pressed = false;
                }
            }
            if let Some(p) = &self.date_popup {
                if !date_rect(p, self.size).contains(self.input.mouse) && !p.anchor.contains(self.input.mouse) {
                    self.date_popup = None;
                    self.input.pressed = false;
                }
            }
        }
    }

    fn push_layer(&mut self, clip: Rect, radius: f32) {
        let s = self.scale;
        let layer = Layer {
            view_proj: glam::Mat4::IDENTITY,
            viewport: [0.0, 0.0, self.size.x, self.size.y],
            clip: [clip.x * s, clip.y * s, clip.right() * s, clip.bottom() * s],
            radius: radius * s,
            opacity: 1.0,
            px_scale: 0.0,
        };
        self.layers.push((layer, Painter::with_scale(s), 0));
    }

    /// Whether a scroll area took this frame's wheel (what is left scrolls the page).
    pub fn wheel_taken(&self) -> bool {
        // (an open dropdown takes it where it lies, at the end of the frame: a finger
        // sliding its list moved the phone's page behind it as well, #774)
        let m = self.input.mouse;
        self.wheel_taken
            || self.popup.as_ref().is_some_and(|p| popup_rect(p, self.size).contains(m))
            || self.date_popup.as_ref().is_some_and(|p| date_rect(p, self.size).contains(m))
    }

    /// The painter of the current layer.
    pub fn p(&mut self) -> &mut Painter {
        &mut self.layers.last_mut().unwrap().1
    }

    /// Draw what follows clipped to `r` (rounded by `radius`), until `pop_clip`.
    pub fn push_clip(&mut self, r: Rect, radius: f32) {
        let r = match self.clip_stack.last() {
            Some((c, _)) => intersect(*c, r),
            None => r,
        };
        self.clip_stack.push((r, radius));
        self.push_layer(r, radius);
    }

    pub fn pop_clip(&mut self) {
        self.clip_stack.pop();
        let (r, rad) = self.clip_stack.last().copied().unwrap_or((Rect::new(0.0, 0.0, self.size.x, self.size.y), 0.0));
        self.push_layer(r, rad);
    }

    fn clip_now(&self) -> Rect {
        self.clip_stack.last().map(|c| c.0).unwrap_or(Rect::new(0.0, 0.0, self.size.x, self.size.y))
    }

    pub fn rect_visible(&self, r: Rect) -> bool {
        let visible = intersect(self.clip_now(), r);
        visible.w > 0.0 && visible.h > 0.0
    }

    /// The mouse is over `r` (and not over an open dropdown lying above it, nor outside the
    /// current clip).
    pub fn hover(&self, r: Rect) -> bool {
        let m = self.input.mouse;
        if !r.contains(m) || !self.clip_now().contains(m) {
            return false;
        }
        if let Some(p) = &self.popup {
            if popup_rect(p, self.size).contains(m) {
                return false;
            }
        }
        if let Some(p) = &self.date_popup {
            if date_rect(p, self.size).contains(m) {
                return false;
            }
        }
        true
    }

    /// A picture of texture `tex` (a render target) filling `r`, its corners rounded.
    pub fn image(&mut self, r: Rect, tex: usize, radius: f32) {
        let clip = intersect(self.clip_now(), r);
        self.push_layer(clip, radius);
        self.layers.last_mut().unwrap().2 = tex;
        let sprite = omsi_ui::Sprite { uv: [0.0, 0.0, 1.0, 1.0], w: r.w, h: r.h, ascent: 0.0 };
        self.p().sprite(sprite, Vec2::new(r.x, r.y), Vec2::new(r.w, r.h), Color::WHITE);
        let (c, rad) = self.clip_stack.last().copied().unwrap_or((Rect::new(0.0, 0.0, self.size.x, self.size.y), 0.0));
        self.push_layer(c, rad);
    }

    /// Mark `r` as interface (the showroom does not orbit under it).
    pub fn solid(&mut self, r: Rect) {
        if r.contains(self.input.mouse) {
            self.over_ui = true;
        }
    }

    /// Eased value towards `to` for widget `id` (time constant in seconds).
    pub fn anim(&mut self, id: Id, to: f32, tau: f32) -> f32 {
        let k = 1.0 - (-self.dt / tau.max(1e-3)).exp();
        let v = self.anims.entry(id).or_insert(to);
        *v += (to - *v) * k;
        *v
    }

    /// Hover/press behaviour of a clickable area: (hovered, pressed now, clicked).
    pub fn interact(&mut self, id: Id, r: Rect) -> (bool, bool, bool) {
        #[cfg(test)]
        self.drawn.insert(id, r);
        let h = self.hover(r);
        if h {
            self.hot = Some(id);
            self.cursor = winit::window::CursorIcon::Pointer;
        }
        if h && self.input.pressed {
            self.active = Some(id);
        }
        let held = self.active == Some(id) && self.input.down;
        let clicked = self.active == Some(id) && self.input.released && h;
        if self.active == Some(id) && self.input.released {
            self.active = None;
        }
        (h, held, clicked)
    }

    pub fn tooltip(&mut self, r: Rect, text: &str) {
        if self.hover(r) && !self.input.down {
            let t = self.anim(id_of(&format!("tip{}{}", r.x, r.y)) ^ 0x55, 1.0, 0.4);
            if t > 0.9 {
                self.tooltip = Some((text.to_string(), self.input.mouse));
            }
        }
    }

    // --- text -------------------------------------------------------------------------

    /// Text on its baseline; returns its width.
    pub fn text(&mut self, text: &str, at: Vec2, px: f32, weight: Weight, c: Color, align: Align) -> f32 {
        let Ui { atlas, fonts, layers, .. } = self;
        layers.last_mut().unwrap().1.text(atlas, fonts, text, px, weight, at, align, c)
    }

    /// Text vertically centred in `r`, cut to fit.
    pub fn text_in(&mut self, text: &str, r: Rect, px: f32, weight: Weight, c: Color, align: Align) -> f32 {
        let Ui { atlas, fonts, layers, .. } = self;
        layers.last_mut().unwrap().1.text_in(atlas, fonts, text, px, weight, r, align, c)
    }

    /// Several lines broken at spaces to `width`; returns the height used.
    pub fn paragraph(&mut self, text: &str, at: Vec2, width: f32, px: f32, weight: Weight, c: Color) -> f32 {
        let text = &*omsi_ui::tr(text);
        let lh = px * 1.38;
        let mut y = at.y + px;
        let mut n = 0;
        for para in text.split('\n') {
            let mut line = String::new();
            for word in para.split(' ') {
                let t = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
                if self.fonts.width(&t, px, weight) > width && !line.is_empty() {
                    self.text(&line, Vec2::new(at.x, y), px, weight, c, Align::Left);
                    y += lh;
                    n += 1;
                    line = word.to_string();
                } else {
                    line = t;
                }
            }
            self.text(&line, Vec2::new(at.x, y), px, weight, c, Align::Left);
            y += lh;
            n += 1;
        }
        n as f32 * lh
    }

    /// Height `paragraph` would take.
    pub fn paragraph_height(&self, text: &str, width: f32, px: f32, weight: Weight) -> f32 {
        let text = &*omsi_ui::tr(text);
        let lh = px * 1.38;
        let mut n = 0;
        for para in text.split('\n') {
            let mut line = String::new();
            for word in para.split(' ') {
                let t = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
                if self.fonts.width(&t, px, weight) > width && !line.is_empty() {
                    n += 1;
                    line = word.to_string();
                } else {
                    line = t;
                }
            }
            n += 1;
        }
        n as f32 * lh
    }

    pub fn icon(&mut self, name: &str, center: Vec2, size: f32, c: Color) {
        let Ui { atlas, layers, .. } = self;
        layers.last_mut().unwrap().1.icon(atlas, name, center, size, c)
    }

    pub fn width(&self, text: &str, px: f32, weight: Weight) -> f32 {
        self.fonts.width(text, px, weight)
    }

    // --- surfaces ------------------------------------------------------------------------

    /// A panel: flat, dark, a hairline edge.
    pub fn panel(&mut self, r: Rect) {
        self.solid(r);
        let p = self.p();
        p.rounded(r, RADIUS, PANEL);
        p.rounded_border(r, RADIUS, 1.0, EDGE);
    }

    /// A section heading inside a panel: an accent tick and the title in capitals.
    pub fn heading(&mut self, r: Rect, title: &str, icon: Option<&str>) -> Rect {
        let _ = icon;
        self.text(&omsi_ui::tr(title).to_uppercase(), Vec2::new(r.x, r.y + 14.0), 11.0, Weight::Bold, TEXT_DIM, Align::Left);
        Rect::new(r.x, r.y + 26.0, r.w, (r.h - 26.0).max(0.0))
    }

    pub fn label(&mut self, r: Rect, text: &str) {
        self.text_in(text, r, 13.0, Weight::Medium, TEXT_DIM, Align::Left);
    }

    // --- controls ------------------------------------------------------------------------

    /// A button. `primary` is the accent-coloured one.
    pub fn button(&mut self, name: &str, r: Rect, label: &str, icon: Option<&str>, kind: ButtonKind) -> bool {
        let id = id_of(name);
        let (h, held, clicked) = self.interact(id, r);
        let t = self.anim(id, if h { 1.0 } else { 0.0 }, 0.06);
        let rr = if held { r.inset(0.5) } else { r };
        let (fill, text_c, edge) = match kind {
            ButtonKind::Primary => (ACCENT.lighten(0.08 * t), Color::rgba(18, 14, 8, 1.0), Color::CLEAR),
            ButtonKind::Danger => (FIELD.mix(HOVER, t), DANGER, DANGER.alpha(0.35 + 0.3 * t)),
            ButtonKind::Normal => (FIELD.mix(HOVER, t), TEXT, EDGE),
            ButtonKind::Ghost => (Color::WHITE.alpha(0.05 * t), if h { TEXT } else { TEXT_DIM }, Color::CLEAR),
        };
        let rad = 6.0;
        self.p().rounded(rr, rad, fill);
        if edge.0[3] > 0.0 {
            self.p().rounded_border(rr, rad, 1.0, edge);
        }
        let px = if rr.h >= 44.0 { 14.5 } else { 13.0 };
        let weight = if kind == ButtonKind::Primary { Weight::Bold } else { Weight::Medium };
        let tw = self.width(label, px, weight);
        // (the gap after the icon only when a label follows it)
        let iw = if icon.is_some() { px * 1.3 + if label.is_empty() { 0.0 } else { 6.0 } } else { 0.0 };
        let x0 = rr.center().x - (tw + iw) * 0.5;
        if let Some(i) = icon {
            self.icon(i, Vec2::new(x0 + px * 0.65, rr.center().y), px * 1.3, text_c);
        }
        self.text_in(label, Rect::new(x0 + iw, rr.y, tw + 2.0, rr.h), px, weight, text_c, Align::Left);
        clicked
    }

    /// A round button with only an icon.
    pub fn icon_button(&mut self, name: &str, c: Vec2, r: f32, icon: &str, tip: &str) -> bool {
        let id = id_of(name);
        let rect = Rect::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r);
        let (h, _, clicked) = self.interact(id, rect);
        let t = self.anim(id, if h { 1.0 } else { 0.0 }, 0.08);
        self.p().circle(c, r, Color::WHITE.alpha(0.06 * t));
        self.icon(icon, c, r * 1.1, if h { TEXT } else { TEXT_DIM });
        if !tip.is_empty() {
            self.tooltip(rect, tip);
        }
        clicked
    }

    /// A switch: returns true when it was flipped.
    pub fn toggle(&mut self, name: &str, r: Rect, value: &mut bool, label: &str) -> bool {
        let id = id_of(name);
        let (h, _, clicked) = self.interact(id, r);
        if clicked {
            *value = !*value;
        }
        let on = self.anim(id, if *value { 1.0 } else { 0.0 }, 0.07);
        let tw = 34.0;
        let th = 18.0;
        let track = Rect::new(r.right() - tw, r.y + (r.h - th) * 0.5, tw, th);
        self.p().rounded(track, th * 0.5, Color::rgba(62, 62, 62, 1.0).mix(ACCENT, on));
        let kx = track.x + th * 0.5 + (tw - th) * on;
        self.p().circle(Vec2::new(kx, track.center().y), th * 0.5 - 3.0, Color::rgba(240, 240, 240, 1.0));
        self.text_in(label, Rect::new(r.x, r.y, r.w - tw - 10.0, r.h), 13.0, Weight::Regular, if h { TEXT } else { TEXT_SOFT }, Align::Left);
        clicked
    }

    /// A slider over `[min, max]` rounded to `step`; `fmt` makes the value's text.
    #[allow(clippy::too_many_arguments)]
    pub fn slider(&mut self, name: &str, r: Rect, value: &mut f32, min: f32, max: f32, step: f32, label: &str, fmt: &dyn Fn(f32) -> String) -> bool {
        let id = id_of(name);
        let label_w = if label.is_empty() { 0.0 } else { (r.w * 0.38).min(170.0) };
        let val_w = 58.0;
        let track_r = Rect::new(r.x + label_w, r.y, r.w - label_w - val_w, r.h);
        let (h, held, _) = self.interact(id, track_r.pad(-8.0, 0.0));
        let before = *value;
        if held {
            let t = ((self.input.mouse.x - track_r.x) / track_r.w.max(1.0)).clamp(0.0, 1.0);
            let mut v = min + t * (max - min);
            if step > 0.0 {
                v = (v / step).round() * step;
            }
            *value = v.clamp(min, max);
            self.cursor = winit::window::CursorIcon::Grabbing;
        } else if h && self.input.wheel.y.abs() > 0.0 && !self.wheel_taken && !self.input.touch {
            let k = if step > 0.0 { step } else { (max - min) / 50.0 };
            *value = (*value + self.input.wheel.y.signum() * k).clamp(min, max);
            self.wheel_taken = true;
        }
        let frac = ((*value - min) / (max - min).max(1e-6)).clamp(0.0, 1.0);
        let shown = self.anim(id ^ 3, frac, 0.06);
        if !label.is_empty() {
            self.text_in(label, Rect::new(r.x, r.y, label_w - 8.0, r.h), 13.0, Weight::Regular, TEXT_SOFT, Align::Left);
        }
        let cy = track_r.center().y;
        let th = 4.0;
        let track = Rect::new(track_r.x, cy - th * 0.5, track_r.w, th);
        self.p().rounded(track, th * 0.5, Color::rgba(58, 58, 58, 1.0));
        self.p().rounded(Rect::new(track.x, track.y, track.w * shown, th), th * 0.5, ACCENT);
        let kc = Vec2::new(track.x + track.w * shown, cy);
        self.p().circle(kc, if h || held { 7.5 } else { 6.5 }, Color::rgba(240, 240, 240, 1.0));
        let txt = fmt(*value);
        self.text_in(&txt, Rect::new(r.right() - val_w + 8.0, r.y, val_w - 8.0, r.h), 12.5, Weight::Medium, TEXT, Align::Right);
        *value != before
    }

    /// Buttons side by side, one of them chosen.
    pub fn segmented(&mut self, name: &str, r: Rect, selected: &mut usize, labels: &[&str]) -> bool {
        let n = labels.len().max(1);
        let id = id_of(name);
        self.p().rounded(r, 6.0, FIELD);
        let w = r.w / n as f32;
        let at = self.anim(id, *selected as f32, 0.07);
        let knob = Rect::new(r.x + 3.0 + w * at, r.y + 3.0, w - 6.0, r.h - 6.0);
        self.p().rounded(knob, 4.0, Color::rgba(62, 62, 62, 1.0));
        let mut changed = false;
        for (k, l) in labels.iter().enumerate() {
            let cell = Rect::new(r.x + w * k as f32, r.y, w, r.h);
            let (h, _, clicked) = self.interact(id ^ (k as u64 + 11), cell);
            if clicked && *selected != k {
                *selected = k;
                changed = true;
            }
            let c = if *selected == k { TEXT } else if h { TEXT_SOFT } else { TEXT_DIM };
            self.text_in(l, cell.pad(4.0, 0.0), 12.5, Weight::Medium, c, Align::Center);
        }
        changed
    }

    /// A dropdown: shows the chosen option, opens a list over everything else.
    pub fn select(&mut self, name: &str, r: Rect, selected: &mut usize, options: &[String]) -> bool {
        let id = id_of(name);
        let mut changed = false;
        if let Some(p) = self.popup.as_mut().filter(|p| p.id == id) {
            if let Some(k) = p.picked.take() {
                if k != *selected && k < options.len() {
                    *selected = k;
                    changed = true;
                }
                self.popup = None;
            }
        }
        let (h, _, clicked) = self.interact(id, r);
        let open = self.popup.as_ref().map(|p| p.id == id).unwrap_or(false);
        let t = self.anim(id, if h || open { 1.0 } else { 0.0 }, 0.08);
        self.p().rounded(r, 6.0, FIELD.mix(HOVER, t));
        self.p().rounded_border(r, 6.0, 1.0, if open { ACCENT.alpha(0.7) } else { EDGE });
        let txt = options.get(*selected).cloned().unwrap_or_default();
        self.text_in(&txt, Rect::new(r.x + 12.0, r.y, r.w - 40.0, r.h), 13.0, Weight::Regular, TEXT, Align::Left);
        let rot = self.anim(id ^ 9, if open { 1.0 } else { 0.0 }, 0.08);
        self.icon(if rot > 0.5 { "expand_less" } else { "expand_more" }, Vec2::new(r.right() - 18.0, r.center().y), 20.0, if h { TEXT } else { TEXT_DIM });
        if clicked {
            // (a phone's tap can come twice - as a touch and as the mouse click made of it:
            // the second one closed the list it had just opened)
            if open && self.popup.as_ref().is_some_and(|p| p.opened >= 1.0) {
                self.popup = None;
            } else if open {
            } else {
                let sel = (*selected).min(options.len().saturating_sub(1));
                let mut p = Popup { id, anchor: r, options: options.to_vec(), selected: sel, scroll: 0.0, opened: 0.0, picked: None, drag: None, query: String::new() };
                // the chosen option in view
                let row = 34.0;
                let visible = popup_rect(&p, self.size).h;
                p.scroll = ((sel as f32 + 0.5) * row - visible * 0.5).clamp(0.0, (options.len() as f32 * row - visible).max(0.0));
                self.popup = Some(p);
                self.date_popup = None;
            }
        } else if let Some(p) = self.popup.as_mut().filter(|p| p.id == id) {
            // the options may change while it is open
            p.options = options.to_vec();
            p.anchor = r;
        }
        changed
    }

    /// A text field. Returns true when the text changed.
    pub fn text_input(&mut self, name: &str, r: Rect, value: &mut String, placeholder: &str, icon: Option<&str>) -> bool {
        let id = id_of(name);
        let (h, _, _) = self.interact(id, r);
        if h {
            self.cursor = winit::window::CursorIcon::Text;
        }
        if h && self.input.pressed {
            self.focus = Some(id);
            self.text_click.insert(id, self.input.mouse.x);
        } else if self.input.pressed && self.focus == Some(id) && !h {
            self.focus = None;
        }
        let focused = self.focus == Some(id);
        let before = value.clone();
        let click_x = self.text_click.remove(&id);
        if focused {
            let (mut caret, mut moved) = self.caret.get(&id).copied().unwrap_or((value.chars().count(), self.time));
            let n = value.chars().count();
            caret = caret.min(n);
            let byte = |s: &str, c: usize| s.char_indices().nth(c).map(|(b, _)| b).unwrap_or(s.len());
            for k in self.input.keys.clone() {
                match k {
                    Key::Left => caret = caret.saturating_sub(1),
                    Key::Right => caret = (caret + 1).min(value.chars().count()),
                    Key::Home => caret = 0,
                    Key::End => caret = value.chars().count(),
                    Key::Backspace => {
                        if let Some(&(a, b)) = self.selection.get(&id) {
                            let (start, end) = (a.min(b), a.max(b));
                            if start != end {
                                let b0 = byte(value, start);
                                let b1 = byte(value, end);
                                value.replace_range(b0..b1, "");
                                caret = start;
                                self.selection.insert(id, (caret, caret));
                            } else if caret > 0 {
                                let b0 = byte(value, caret - 1);
                                let b1 = byte(value, caret);
                                value.replace_range(b0..b1, "");
                                caret -= 1;
                            }
                        } else if caret > 0 {
                            let b0 = byte(value, caret - 1);
                            let b1 = byte(value, caret);
                            value.replace_range(b0..b1, "");
                            caret -= 1;
                        }
                    }
                    Key::Delete => {
                        if let Some(&(a, b)) = self.selection.get(&id) {
                            let (start, end) = (a.min(b), a.max(b));
                            if start != end {
                                let b0 = byte(value, start);
                                let b1 = byte(value, end);
                                value.replace_range(b0..b1, "");
                                caret = start;
                                self.selection.insert(id, (caret, caret));
                            } else if caret < value.chars().count() {
                                let b0 = byte(value, caret);
                                let b1 = byte(value, caret + 1);
                                value.replace_range(b0..b1, "");
                            }
                        } else if caret < value.chars().count() {
                            let b0 = byte(value, caret);
                            let b1 = byte(value, caret + 1);
                            value.replace_range(b0..b1, "");
                        }
                    }
                    Key::SelectAll => {
                        self.selection.insert(id, (0, value.chars().count()));
                        caret = value.chars().count();
                    }
                    Key::Copy => self.clipboard_out = Some(value.clone()),
                    Key::Cut => {
                        self.clipboard_out = Some(value.clone());
                        value.clear();
                        caret = 0;
                    }
                    Key::Paste => {
                        if let Some(t) = self.clipboard_in.clone() {
                            let t: String = t.chars().filter(|c| !c.is_control()).collect();
                            let b = byte(value, caret);
                            value.insert_str(b, &t);
                            caret += t.chars().count();
                        }
                    }
                    Key::Enter | Key::Escape => self.focus = None,
                    _ => {}
                }
                moved = self.time;
            }
            if !self.input.text.is_empty() {
                let t: String = self.input.text.chars().filter(|c| !c.is_control()).collect();

                if let Some(&(a, b)) = self.selection.get(&id) {
                    let (start, end) = (a.min(b), a.max(b));

                    if start != end {
                        let b0 = byte(value, start);
                        let b1 = byte(value, end);
                        value.replace_range(b0..b1, "");
                        caret = start;
                    }
                }

                let b = byte(value, caret);
                value.insert_str(b, &t);
                caret += t.chars().count();
                self.selection.insert(id, (caret, caret));
                moved = self.time;
            }
            self.caret.insert(id, (caret, moved));
        }
        let t = self.anim(id, if focused { 1.0 } else if h { 0.5 } else { 0.0 }, 0.08);
        self.p().rounded(r, 6.0, FIELD.mix(HOVER, t * 0.5));
        self.p().rounded_border(r, 6.0, 1.0, if focused { ACCENT.alpha(0.7) } else { EDGE });
        let mut x = r.x + 12.0;
        if let Some(i) = icon {
            self.icon(i, Vec2::new(x + 8.0, r.center().y), 18.0, TEXT_DIM);
            x += 24.0;
        }
        let inner = Rect::new(x, r.y, r.right() - x - 10.0, r.h);
        self.push_clip(inner, 0.0);
        let px = 13.0;

        if let Some(click_x) = click_x {
                let local_x = (click_x - inner.x).clamp(0.0, inner.w);
                
                let mut best = 0;
                let mut best_dist = f32::MAX;

                for i in 0..=value.chars().count() {
                    let upto: String = value.chars().take(i).collect();
                    let cx = self.width(&upto, px, Weight::Regular);
                    let dist = (cx - local_x).abs();
                    
                    if dist < best_dist {
                        best_dist = dist;
                        best = i;
                    }
                }

                self.caret.insert(id, (best, self.time));
                self.selection.insert(id, (best, best));
            }
        if self.focus == Some(id) && self.input.down {
            let mouse_x = self.input.mouse.x;
            let local_x = (mouse_x - inner.x).clamp(0.0, inner.w);

            let mut best = 0;
            let mut best_dist = f32::MAX;

            for i in 0..=value.chars().count() {
                let upto: String = value.chars().take(i).collect();
                let cx = self.width(&upto, px, Weight::Regular);
                let dist = (cx - local_x).abs();
            
                if dist < best_dist {
                    best_dist = dist;
                    best = i;
                }
            }
        
            if let Some(&(anchor, _)) = self.selection.get(&id) {
                self.selection.insert(id, (anchor, best));
                self.caret.insert(id, (best, self.time));
            }
        }
        if value.is_empty() && !focused {
            self.text_in(placeholder, inner, px, Weight::Regular, TEXT_FAINT, Align::Left);
        } else {
            let (caret, moved) = self.caret.get(&id).copied().unwrap_or((0, 0.0));
            let upto: String = value.chars().take(caret).collect();
            let cw = self.width(&upto, px, Weight::Regular);
            if let Some(&(a, b)) = self.selection.get(&id) {
                let start = a.min(b);
                let end = a.max(b);

                if start != end {
                    let before: String = value.chars().take(start).collect();
                    let selected: String = value.chars().skip(start).take(end - start).collect();

                    let sx = self.width(&before, px, Weight::Regular);
                    let sw = self.width(&selected, px, Weight::Regular);

                    self.p().rect(
                        Rect::new(inner.x + sx, r.y + 5.0, sw, r.h - 10.0),
                        ACCENT.alpha(0.35),
                    );
                }
            }
            // keep the caret in view
            let shift = (cw - inner.w + 4.0).max(0.0);
            self.text_in(value, Rect::new(inner.x - shift, inner.y, inner.w + shift + 2000.0, inner.h), px, Weight::Regular, TEXT, Align::Left);
            if focused 
                && self.selection.get(&id).is_none_or(|&(a, b)| a == b)
                && ((self.time - moved) % 1.0) < 0.55
            {
                self.p().rect(
                    Rect::new(inner.x - shift + cw + 0.5, r.center().y -8.5, 1.5, 17.0),
                    ACCENT,
                );
            }
        }
        if value.is_empty() {
            self.text_in(placeholder, inner, px, Weight::Regular, TEXT_FAINT, Align::Left);
        }
        self.pop_clip();
        *value != before
    }

    /// Hours and minutes with arrows (and the wheel over either).
    pub fn time_field(&mut self, name: &str, r: Rect, minutes: &mut i32) -> bool {
        let before = *minutes;
        self.p().rounded(r, 6.0, FIELD);
        self.p().rounded_border(r, 6.0, 1.0, EDGE);
        let half = (r.w - 16.0) * 0.5;
        for (k, (unit, step)) in [(60, 60), (1, 5)].iter().enumerate() {
            let cell = Rect::new(r.x + k as f32 * (half + 16.0), r.y, half, r.h);
            let id = id_of(&format!("{name}.{k}"));
            let (h, _, _) = self.interact(id, cell);
            if h && self.input.wheel.y.abs() > 0.0 && !self.wheel_taken && !self.input.touch {
                *minutes += self.input.wheel.y.signum() as i32 * if *unit == 60 { 60 } else { 5 };
                self.wheel_taken = true;
            }
            let v = if *unit == 60 { minutes.rem_euclid(1440) / 60 } else { minutes.rem_euclid(60) };
            self.text_in(&format!("{v:02}"), Rect::new(cell.x + 8.0, cell.y, cell.w - 30.0, cell.h), 17.0, Weight::Medium, TEXT, Align::Center);
            let up = Rect::new(cell.right() - 24.0, cell.y + 3.0, 20.0, cell.h * 0.5 - 3.0);
            let down = Rect::new(cell.right() - 24.0, cell.center().y, 20.0, cell.h * 0.5 - 3.0);
            let (hu, _, cu) = self.interact(id ^ 1, up);
            let (hd, _, cd) = self.interact(id ^ 2, down);
            self.icon("expand_less", up.center(), 16.0, if hu { TEXT } else { TEXT_FAINT });
            self.icon("expand_more", down.center(), 16.0, if hd { TEXT } else { TEXT_FAINT });
            if cu {
                *minutes += step;
            }
            if cd {
                *minutes -= step;
            }
        }
        self.text_in(":", Rect::new(r.x + half, r.y, 16.0, r.h - 2.0), 17.0, Weight::Medium, TEXT_DIM, Align::Center);
        *minutes = minutes.rem_euclid(1440);
        *minutes != before
    }

    /// A date field (YYYY-MM-DD) with a calendar that opens under it.
    pub fn date_field(&mut self, name: &str, r: Rect, date: &mut String) -> bool {
        let id = id_of(name);
        let mut changed = false;
        if let Some(p) = self.date_popup.as_mut().filter(|p| p.id == id) {
            if let Some((y, m, d)) = p.picked.take() {
                *date = format!("{y:04}-{m:02}-{d:02}");
                changed = true;
                self.date_popup = None;
            }
        }
        let (h, _, clicked) = self.interact(id, r);
        let open = self.date_popup.as_ref().map(|p| p.id == id).unwrap_or(false);
        let t = self.anim(id, if h || open { 1.0 } else { 0.0 }, 0.08);
        self.p().rounded(r, 6.0, FIELD.mix(HOVER, t));
        self.p().rounded_border(r, 6.0, 1.0, if open { ACCENT.alpha(0.7) } else { EDGE });
        self.icon("calendar_month", Vec2::new(r.x + 20.0, r.center().y), 17.0, TEXT_DIM);
        let (y, m, d) = parse_date(date);
        let shown = format!("{} {} {}", d, MONTHS[(m as usize).clamp(1, 12) - 1], y);
        self.text_in(&shown, Rect::new(r.x + 38.0, r.y, r.w - 60.0, r.h), 13.0, Weight::Regular, TEXT, Align::Left);
        self.icon("expand_more", Vec2::new(r.right() - 18.0, r.center().y), 20.0, TEXT_DIM);
        if clicked {
            if open {
                self.date_popup = None;
            } else {
                self.date_popup = Some(DatePopup { id, anchor: r, year: y, month: m, current: (y, m, d), picked: None, opened: 0.0 });
                self.popup = None;
            }
        }
        changed
    }

    /// A progress bar (0..1), animated stripes while `busy`.
    pub fn progress(&mut self, r: Rect, frac: f32, busy: bool) {
        self.p().rounded(r, r.h * 0.5, Color::rgba(50, 50, 50, 1.0));
        let w = (r.w * frac.clamp(0.0, 1.0)).max(r.h);
        self.p().rounded(Rect::new(r.x, r.y, w, r.h), r.h * 0.5, ACCENT);
        if busy {
            let x = r.x + ((self.time * 0.6) % 1.0) * (w + 60.0) - 60.0;
            self.push_clip(Rect::new(r.x, r.y, w, r.h), r.h * 0.5);
            self.p().gradient_h(Rect::new(x, r.y, 60.0, r.h), Color::WHITE.alpha(0.0), Color::WHITE.alpha(0.15));
            self.pop_clip();
        }
    }

    /// A vertical list that scrolls: `body` draws the rows given the offset and returns the
    /// content height. Draws a thin scrollbar.
    pub fn scroll_area(&mut self, name: &str, r: Rect, body: &mut dyn FnMut(&mut Ui, Rect) -> f32) {
        let id = id_of(name);
        let off = self.scroll.get(&id).copied().unwrap_or(0.0);
        self.push_clip(r, 6.0);
        let content = body(self, Rect::new(r.x, r.y - off, r.w, r.h));
        self.pop_clip();
        self.scroll_keep(name, r, content);
    }

    /// The scrolling of such a view: the bar, the wheel, and its own easing towards where it
    /// was sent. `content` is what the rows came to, all of them.
    pub fn scroll_keep(&mut self, name: &str, r: Rect, content: f32) {
        let id = id_of(name);
        let off = self.scroll.get(&id).copied().unwrap_or(0.0);
        let max = (content - r.h).max(0.0);
        let mut target = self.scroll.get(&(id ^ 0xabc)).copied().unwrap_or(off);
        if self.hover(r) && self.input.wheel.y.abs() > 0.0 && !self.wheel_taken {
            // what the list cannot use (it is at its end) goes on to the list or page
            // around it: a finger on a list at its end scrolls the phone's page on
            let want = target - self.input.wheel.y * 42.0;
            let left = want - want.clamp(0.0, max);
            target = want - left;
            if left.abs() < 0.5 {
                self.wheel_taken = true;
            } else {
                self.input.wheel.y = -left / 42.0;
            }
        }
        // dragging the bar
        if max > 0.0 {
            let bar_h = (r.h * r.h / content).max(28.0);
            let bar_y = r.y + (r.h - bar_h) * (off / max);
            let bar = Rect::new(r.right() - 5.0, bar_y, 4.0, bar_h);
            let (h, held, _) = self.interact(id ^ 0xdef, Rect::new(bar.x - 6.0, bar.y, 14.0, bar.h));
            if held {
                target = ((self.input.mouse.y - r.y - bar_h * 0.5) / (r.h - bar_h).max(1.0)) * max;
            }
            let t = self.anim(id ^ 0x77, if h || held || self.hover(r) { 1.0 } else { 0.0 }, 0.15);
            self.p().rounded(bar, 2.0, Color::WHITE.alpha(0.10 + 0.25 * t));
        }
        target = target.clamp(0.0, max);
        self.scroll.insert(id ^ 0xabc, target);
        let k = 1.0 - (-self.dt / 0.07).exp();
        let now = off + (target - off) * k;
        self.scroll.insert(id, if (now - target).abs() < 0.3 { target } else { now });
    }

    /// Scroll the list `name` to show a row at `y..y+h` of its content (a newly chosen row).
    pub fn scroll_to(&mut self, name: &str, y: f32, h: f32, view_h: f32) {
        let id = id_of(name);
        let t = self.scroll.get(&(id ^ 0xabc)).copied().unwrap_or(0.0);
        let t = if y < t { y } else if y + h > t + view_h { y + h - view_h } else { t };
        self.scroll.insert(id ^ 0xabc, t.max(0.0));
    }

    /// A row of a list: hover highlight, the chosen one marked. Returns clicked.
    pub fn row(&mut self, name: &str, r: Rect, selected: bool) -> bool {
        let id = id_of(name);
        let (h, _, clicked) = self.interact(id, r);
        let t = self.anim(id, if h { 1.0 } else { 0.0 }, 0.08);
        let s = self.anim(id ^ 4, if selected { 1.0 } else { 0.0 }, 0.1);
        if s > 0.01 {
            self.p().rounded(r, 6.0, SELECTED.alpha(s));
            self.p().rounded(Rect::new(r.x, r.y + 8.0, 2.0, r.h - 16.0), 1.0, ACCENT.alpha(s));
        } else if t > 0.01 {
            self.p().rounded(r, 6.0, HOVER.alpha(t));
        }
        clicked
    }

    /// Small coloured tag.
    pub fn badge(&mut self, at: Vec2, text: &str, c: Color) -> f32 {
        let w = self.width(text, 10.0, Weight::Bold) + 10.0;
        let r = Rect::new(at.x, at.y, w, 16.0);
        self.p().rounded(r, 4.0, c.alpha(0.14));
        self.text_in(text, r, 10.0, Weight::Bold, c, Align::Center);
        w
    }

    // --- end of frame -----------------------------------------------------------------

    /// Draw the popups and tooltip over everything and hand the layers over.
    pub fn finish(&mut self) -> (Vec<Layer>, Vec<Vertex>, Vec<(std::ops::Range<u32>, usize)>) {
        self.clip_stack.clear();
        self.push_layer(Rect::new(0.0, 0.0, self.size.x, self.size.y), 0.0);
        self.draw_popup();
        self.draw_date_popup();
        if let Some((t, at)) = self.tooltip.take() {
            let w = (self.width(&t, 12.5, Weight::Medium) + 20.0).min(360.0).min((self.size.x - 16.0).max(1.0));
            let text_w = (w - 20.0).max(1.0);
            let max_h = (self.size.y - 16.0).clamp(1.0, 280.0);
            let h = (self.paragraph_height(&t, text_w, 12.5, Weight::Medium) + 12.0).min(max_h);
            let x = (at.x + 14.0).clamp(8.0, (self.size.x - w - 8.0).max(8.0));
            let below = at.y + 18.0;
            let above = at.y - 12.0 - h;
            let y = if below + h <= self.size.y - 8.0 {
                below
            } else if above >= 8.0 {
                above
            } else {
                (self.size.y - h - 8.0).max(8.0)
            };
            let r = Rect::new(x, y, w, h);
            self.p().rounded(r, 6.0, Color::rgba(34, 34, 34, 1.0));
            self.p().rounded_border(r, 7.0, 1.0, Color::WHITE.alpha(0.1));
            self.push_clip(Rect::new(r.x + 10.0, r.y + 4.0, text_w, (h - 8.0).max(0.0)), 2.0);
            self.paragraph(&t, Vec2::new(r.x + 10.0, r.y + 4.0), text_w, 12.5, Weight::Medium, TEXT_SOFT);
            self.pop_clip();
        }
        let mut layers = Vec::new();
        let mut verts = Vec::new();
        let mut ranges: Vec<(std::ops::Range<u32>, usize)> = Vec::new();
        // (the GPU side draws 256 layers at most and drops the rest: a list of 242 trips
        // made a layer for every field of every row, hidden or not, and the rail, the
        // buttons under it and the status bar were never drawn, #666. A layer whose clip
        // shows nothing is left out, one that clips as the layer before it joins it.)
        let same = |a: &Layer, b: &Layer| a.clip == b.clip && a.radius == b.radius && a.viewport == b.viewport && a.opacity == b.opacity && a.px_scale == b.px_scale && a.view_proj == b.view_proj;
        for (l, p, tex) in self.layers.drain(..) {
            if p.verts.is_empty() || l.clip[2] <= l.clip[0] || l.clip[3] <= l.clip[1] {
                continue;
            }
            let a = verts.len() as u32;
            verts.extend(p.verts);
            match (layers.last(), ranges.last_mut()) {
                (Some(last), Some((range, last_tex))) if *last_tex == tex && same(last, &l) && range.end == a => range.end = verts.len() as u32,
                _ => {
                    layers.push(l);
                    ranges.push((a..verts.len() as u32, tex));
                }
            }
        }
        // the input of this frame is used up
        self.discard_input();
        (layers, verts, ranges)
    }

    /// Forget the clicks, keys and text since the last frame (used up by a frame, or come
    /// while nothing was drawn).
    pub fn discard_input(&mut self) {
        self.input.pressed = false;
        self.input.released = false;
        self.input.right_pressed = false;
        self.input.wheel = Vec2::ZERO;
        self.input.text.clear();
        self.input.keys.clear();
        self.input.raw_key = None;
        self.input.raw_chord = 0;
        self.input.double_click = false;
    }

    fn draw_popup(&mut self) {
        let Some(mut p) = self.popup.take() else { return };
        let r = popup_rect(&p, self.size);
        // the tap that opened the list must not also pick from it (on a small screen the
        // list lies under the finger)
        let fresh = p.opened < 0.5;
        p.opened = (p.opened + self.dt / 0.3).min(1.0);
        let e = 1.0 - (1.0 - p.opened).powi(3);
        let rr = Rect::new(r.x, r.y - 6.0 * (1.0 - e), r.w, r.h);
        self.p().rounded(rr, 8.0, Color::rgba(28, 28, 28, e));
        self.p().rounded_border(rr, 8.0, 1.0, Color::WHITE.alpha(0.1 * e));
        let row = 34.0;
        let shown = p.shown();
        // (what was typed, over the options it leaves)
        let head = if p.query.is_empty() { 0.0 } else { row };
        let content = shown.len() as f32 * row + head;
        let max = (content - rr.h + 8.0).max(0.0);
        if rr.contains(self.input.mouse) && self.input.wheel.y.abs() > 0.0 {
            p.scroll = (p.scroll - self.input.wheel.y * 40.0).clamp(0.0, max);
        }
        // the scrollbar: its thumb is dragged, a press on the track beside it jumps there
        let bar_h = (rr.h * rr.h / content.max(1.0)).max(24.0);
        let track = Rect::new(rr.right() - 14.0, rr.y, 14.0, rr.h);
        if max > 0.0 {
            if self.input.pressed && !fresh && track.contains(self.input.mouse) {
                let y = rr.y + (rr.h - bar_h) * (p.scroll / max.max(1.0));
                let inside = self.input.mouse.y - y;
                p.drag = Some(if (0.0..=bar_h).contains(&inside) { inside } else { bar_h * 0.5 });
            }
            if let Some(grab) = p.drag {
                let at = (self.input.mouse.y - grab - rr.y) / (rr.h - bar_h).max(1.0);
                p.scroll = (at * max).clamp(0.0, max);
            }
        }
        let dragging = p.drag.is_some();
        if !self.input.down || self.input.released {
            p.drag = None;
        }
        self.push_clip(rr.inset(4.0), 8.0);
        if head > 0.0 {
            let y = rr.y + 4.0 - p.scroll;
            self.icon("search", Vec2::new(rr.x + 22.0, y + row * 0.5), 16.0, TEXT_DIM);
            let caret = if (self.time * 2.0) as i64 % 2 == 0 { "|" } else { "" };
            self.text_in(&format!("{}{caret}", p.query), Rect::new(rr.x + 38.0, y, rr.w - 60.0, row), 13.0, Weight::Medium, TEXT, Align::Left);
            if shown.is_empty() {
                self.text_in("Nothing found", Rect::new(rr.x + 14.0, y + row, rr.w - 28.0, row), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
            }
        }
        for (n, &k) in shown.iter().enumerate() {
            let o = &p.options[k];
            let y = rr.y + 4.0 + head + n as f32 * row - p.scroll;
            if y + row < rr.y || y > rr.bottom() {
                continue;
            }
            let cell = Rect::new(rr.x + 4.0, y, rr.w - 8.0, row);
            let h = cell.contains(self.input.mouse) && rr.contains(self.input.mouse) && !dragging && !(max > 0.0 && track.contains(self.input.mouse));
            if k == p.selected {
                self.p().rounded(cell, 5.0, SELECTED);
                self.icon("check", Vec2::new(cell.right() - 16.0, cell.center().y), 15.0, ACCENT);
            } else if h {
                self.p().rounded(cell, 5.0, HOVER);
            }
            self.text_in(o, Rect::new(cell.x + 10.0, cell.y, cell.w - 36.0, cell.h), 13.0, Weight::Regular, TEXT, Align::Left);
            if h {
                self.cursor = winit::window::CursorIcon::Pointer;
                if self.input.released && !fresh {
                    p.picked = Some(k);
                }
            }
        }
        self.pop_clip();
        if max > 0.0 {
            let y = rr.y + (rr.h - bar_h) * (p.scroll / max.max(1.0));
            let wide = dragging || track.contains(self.input.mouse);
            let w = if wide { 5.0 } else { 3.0 };
            self.p().rounded(Rect::new(rr.right() - 2.0 - w, y, w, bar_h), w * 0.5, Color::WHITE.alpha(if wide { 0.5 } else { 0.3 }));
        }
        if rr.contains(self.input.mouse) {
            self.over_ui = true;
        }
        self.popup = Some(p);
    }

    fn draw_date_popup(&mut self) {
        let Some(mut p) = self.date_popup.take() else { return };
        let r = date_rect(&p, self.size);
        p.opened = (p.opened + self.dt / 0.12).min(1.0);
        let e = 1.0 - (1.0 - p.opened).powi(3);
        self.p().rounded(r, 8.0, Color::rgba(28, 28, 28, e));
        self.p().rounded_border(r, 8.0, 1.0, Color::WHITE.alpha(0.1 * e));
        let m = self.input.mouse;
        let click = self.input.released;
        // month header with arrows
        let head = Rect::new(r.x + 8.0, r.y + 8.0, r.w - 16.0, 30.0);
        self.text_in(&format!("{} {}", MONTHS_LONG[p.month as usize - 1], p.year), head, 14.0, Weight::Bold, TEXT, Align::Center);
        let prev = Rect::new(head.x, head.y, 30.0, 30.0);
        let next = Rect::new(head.right() - 30.0, head.y, 30.0, 30.0);
        let py = Rect::new(head.x + 30.0, head.y, 30.0, 30.0);
        let ny = Rect::new(head.right() - 60.0, head.y, 30.0, 30.0);
        for (b, icon) in [(prev, "chevron_left"), (next, "chevron_right"), (py, "expand_more"), (ny, "expand_less")] {
            let h = b.contains(m);
            if h {
                self.p().rounded(b, 6.0, Color::WHITE.alpha(0.08));
                self.cursor = winit::window::CursorIcon::Pointer;
            }
            self.icon(icon, b.center(), 20.0, if h { ACCENT } else { TEXT_DIM });
        }
        if click && prev.contains(m) {
            if p.month == 1 {
                p.month = 12;
                p.year -= 1;
            } else {
                p.month -= 1;
            }
        }
        if click && next.contains(m) {
            if p.month == 12 {
                p.month = 1;
                p.year += 1;
            } else {
                p.month += 1;
            }
        }
        if click && py.contains(m) {
            p.year -= 1;
        }
        if click && ny.contains(m) {
            p.year += 1;
        }
        let cw = (r.w - 16.0) / 7.0;
        for (k, d) in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"].iter().enumerate() {
            self.text_in(d, Rect::new(r.x + 8.0 + cw * k as f32, r.y + 42.0, cw, 18.0), 11.0, Weight::Bold, if k >= 5 { ACCENT.alpha(0.8) } else { TEXT_FAINT }, Align::Center);
        }
        let first = weekday(p.year, p.month, 1);
        let days = days_in_month(p.year, p.month);
        let chosen = Some(p.current);
        for d in 1..=days {
            let cell_k = first + d as i32 - 1;
            let (col, row) = (cell_k % 7, cell_k / 7);
            let cell = Rect::new(r.x + 8.0 + cw * col as f32, r.y + 62.0 + row as f32 * 30.0, cw, 28.0).inset(1.5);
            let h = cell.contains(m);
            let is_chosen = chosen == Some((p.year, p.month, d));
            if is_chosen {
                self.p().rounded(cell, 7.0, ACCENT);
            } else if h {
                self.p().rounded(cell, 7.0, Color::WHITE.alpha(0.09));
            }
            self.text_in(&d.to_string(), cell, 12.5, Weight::Medium, if is_chosen { Color::rgba(20, 16, 8, 1.0) } else { TEXT }, Align::Center);
            if h {
                self.cursor = winit::window::CursorIcon::Pointer;
                if click {
                    p.picked = Some((p.year, p.month, d));
                }
            }
        }
        if r.contains(m) {
            self.over_ui = true;
        }
        self.date_popup = Some(p);
    }

}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Normal,
    Danger,
    Ghost,
}

fn intersect(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = a.right().min(b.right());
    let y1 = a.bottom().min(b.bottom());
    Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

fn popup_rect(p: &Popup, size: Vec2) -> Rect {
    let row = 34.0;
    let h = (p.options.len() as f32 * row + 8.0).min(320.0);
    let below = p.anchor.bottom() + 6.0;
    let y = if below + h > size.y - 10.0 { (p.anchor.y - 6.0 - h).max(10.0) } else { below };
    Rect::new(p.anchor.x, y, p.anchor.w.max(200.0), h)
}

fn date_rect(p: &DatePopup, size: Vec2) -> Rect {
    let h = 62.0 + 6.0 * 30.0 + 8.0;
    let w = 280.0;
    let below = p.anchor.bottom() + 6.0;
    let y = if below + h > size.y - 10.0 { (p.anchor.y - 6.0 - h).max(10.0) } else { below };
    Rect::new(p.anchor.x, y, w, h)
}

pub const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
pub const MONTHS_LONG: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

pub fn parse_date(s: &str) -> (i32, u32, u32) {
    let mut it = s.trim().split('-');
    let y = it.next().and_then(|x| x.parse().ok()).unwrap_or(1989);
    let m = it.next().and_then(|x| x.parse().ok()).unwrap_or(5).clamp(1, 12);
    let d = it.next().and_then(|x| x.parse().ok()).unwrap_or(30).clamp(1, 31);
    (y, m, d)
}

pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

/// Day of the week, 0 = Monday.
pub fn weekday(y: i32, m: u32, d: u32) -> i32 {
    // Sakamoto
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    let w = (y + y / 4 - y / 100 + y / 400 + t[(m - 1) as usize] + d as i32) % 7; // 0 = Sunday
    (w + 6) % 7
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_visibility_respects_nested_clips_and_partial_rows() {
        let mut ui = Ui::new();
        ui.begin(Vec2::new(400.0, 300.0), 1.0, 0.016);
        ui.push_clip(Rect::new(10.0, 50.0, 200.0, 100.0), 0.0);
        assert!(!ui.rect_visible(Rect::new(10.0, 0.0, 200.0, 50.0)));
        assert!(ui.rect_visible(Rect::new(10.0, 40.0, 200.0, 54.0)));
        assert!(!ui.rect_visible(Rect::new(10.0, 150.0, 200.0, 54.0)));
        ui.push_clip(Rect::new(10.0, 80.0, 200.0, 20.0), 0.0);
        assert!(!ui.rect_visible(Rect::new(10.0, 50.0, 200.0, 20.0)));
        assert!(ui.rect_visible(Rect::new(10.0, 90.0, 200.0, 54.0)));
        ui.pop_clip();
        assert!(ui.rect_visible(Rect::new(10.0, 50.0, 200.0, 20.0)));
    }

    #[test]
    fn calendar_arithmetic() {
        assert_eq!(weekday(2026, 9, 24), 3); // a Thursday
        assert_eq!(weekday(1989, 5, 30), 1); // a Tuesday
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(parse_date("1989-05-30"), (1989, 5, 30));
    }

    #[test]
    fn a_button_with_only_an_icon_has_it_in_the_middle() {
        let mut ui = Ui::new();
        ui.begin(Vec2::new(400.0, 300.0), 1.0, 1.0 / 60.0);
        let r = Rect::new(100.0, 100.0, 46.0, 46.0);
        ui.button("b", r, "", Some("chevron_left"), ButtonKind::Normal);
        // the icon is the only sprite drawn
        let xs: Vec<f32> = ui.p().verts.iter().filter(|v| v.mode == [0.0, 1.0]).map(|v| v.pos[0]).collect();
        assert!(!xs.is_empty());
        let middle = (xs.iter().cloned().fold(f32::MAX, f32::min) + xs.iter().cloned().fold(f32::MIN, f32::max)) * 0.5;
        assert!((middle - r.center().x).abs() <= 0.5, "the icon is at {middle}, the button's middle at {}", r.center().x);
    }

    /// A long list of fields in a scroll area (a tour of 242 trips) stays well under the
    /// 256 layers the GPU draws, so what comes after it - the rail - is drawn (#666).
    #[test]
    fn a_long_list_of_fields_leaves_layers_for_the_rest_of_the_page() {
        let mut ui = Ui::new();
        ui.begin(Vec2::new(1400.0, 900.0), 1.0, 1.0 / 60.0);
        let mut texts: Vec<String> = (0..242).map(|k| format!("{}:{:02}", 4 + k / 60, k % 60)).collect();
        let options = vec!["a".to_string(), "b".to_string()];
        ui.scroll_area("trips", Rect::new(300.0, 100.0, 900.0, 600.0), &mut |ui, v| {
            for (i, t) in texts.iter_mut().enumerate() {
                let y = v.y + i as f32 * 42.0;
                ui.text_input(&format!("dep-{i}"), Rect::new(v.x, y, 110.0, 36.0), t, "h:mm", None);
                let mut k = 0;
                ui.select(&format!("trip-{i}"), Rect::new(v.x + 120.0, y, 200.0, 36.0), &mut k, &options);
            }
            242.0 * 42.0
        });
        // the rail, last
        ui.p().rect(Rect::new(0.0, 0.0, 240.0, 900.0), Color::WHITE);
        let (layers, _, ranges) = ui.finish();
        assert_eq!(layers.len(), ranges.len());
        assert!(layers.len() < 128, "{} layers", layers.len());
    }

    /// Where the window never says which modifiers are held (Android), their keys do: a key
    /// bound with Shift or Ctrl held keeps them, and Ctrl+V is a paste (#634).
    #[test]
    fn modifier_keys_count_where_the_window_does_not_tell_them() {
        use winit::keyboard::{KeyCode as K, ModifiersState as M};
        let mut m = Modifiers::default();
        let mut input = Input::default();
        assert!(!m.key(K::KeyA, true));
        assert!(m.key(K::ShiftLeft, true) && m.key(K::ControlRight, true));
        m.apply(&mut input);
        assert!(input.shift && input.ctrl && !input.alt && m.command());
        assert_eq!(omsi_content::input::chord(input.shift, input.ctrl, input.alt), omsi_content::input::KEY_SHIFT | omsi_content::input::KEY_CTRL);
        // (both Shift keys down, one let go of: still held)
        m.key(K::ShiftRight, true);
        m.key(K::ShiftLeft, false);
        m.key(K::ControlRight, false);
        assert_eq!(m.state(), M::SHIFT);
        // (the keyboard taken away: nothing is held any more)
        m.release_keys();
        assert_eq!(m.state(), M::empty());
        // where the window does tell them, its word holds
        m.key(K::AltLeft, true);
        m.told(M::CONTROL);
        assert_eq!(m.state(), M::CONTROL);
        m.apply(&mut input);
        assert!(!input.shift && input.ctrl && !input.alt);
    }

    #[test]
    fn binding_keeps_modifiers_when_released_before_redraw() {
        use winit::keyboard::{KeyCode as K, ModifiersState as M};
        for held in [M::SHIFT, M::CONTROL, M::ALT, M::SHIFT | M::CONTROL | M::ALT] {
            let mut m = Modifiers::default();
            let mut input = Input::default();
            m.told(held);
            m.apply(&mut input);
            // KeyboardInput presses A; ModifiersChanged releases the chord before redraw.
            input.capture_key(K::KeyA);
            m.told(M::empty());
            m.apply(&mut input);
            assert_eq!(input.raw_key, Some(K::KeyA));
            assert_eq!(input.binding_modifiers(), omsi_content::input::chord(
                held.shift_key(), held.control_key(), held.alt_key()));
        }
    }

    #[test]
    fn binding_snapshot_uses_fallback_keys_and_does_not_leak_into_next_binding() {
        use winit::keyboard::KeyCode as K;
        let mut m = Modifiers::default();
        let mut ui = Ui::new();
        for key in [K::ShiftRight, K::ControlLeft, K::AltRight] { m.key(key, true); }
        m.apply(&mut ui.input);
        ui.input.capture_key(K::KeyA);
        for key in [K::ShiftRight, K::ControlLeft, K::AltRight] { m.key(key, false); }
        m.apply(&mut ui.input);
        ui.input.capture_key(K::ShiftLeft);
        assert_eq!(ui.input.raw_key, Some(K::KeyA));
        assert_eq!(ui.input.binding_modifiers(), omsi_content::input::KEY_SHIFT |
            omsi_content::input::KEY_CTRL | omsi_content::input::KEY_ALT);
        ui.discard_input();
        assert_eq!(ui.input.binding_modifiers(), 0);
        ui.input.capture_key(K::KeyB);
        assert_eq!(ui.input.raw_key, Some(K::KeyB));
        assert_eq!(ui.input.binding_modifiers(), 0);
    }

    /// What was clicked and typed while the launcher drew nothing (a game ran) is gone:
    /// the first frame drawn afterwards pressed the button under the mouse, Start again.
    #[test]
    fn input_while_nothing_is_drawn_is_not_used_afterwards() {
        let mut ui = Ui::new();
        ui.input.pressed = true;
        ui.input.released = true;
        ui.input.right_pressed = true;
        ui.input.double_click = true;
        ui.input.wheel = Vec2::new(0.0, -3.0);
        ui.input.text.push_str("abc");
        ui.input.keys.push(Key::Enter);
        ui.input.raw_key = Some(winit::keyboard::KeyCode::Enter);
        ui.discard_input();
        let i = &ui.input;
        assert!(!i.pressed && !i.released && !i.right_pressed && !i.double_click);
        assert_eq!(i.wheel, Vec2::ZERO);
        assert!(i.text.is_empty() && i.keys.is_empty() && i.raw_key.is_none());
    }

    /// A long dropdown (a bus with hundreds of fleet numbers) scrolls by dragging its bar,
    /// and letting go over an option does not pick it.
    #[test]
    fn a_long_dropdown_scrolls_by_dragging_its_bar() {
        let options: Vec<String> = (0..200).map(|k| format!("{k}")).collect();
        let mut ui = Ui::new();
        let mut sel = 0;
        let field = Rect::new(20.0, 20.0, 240.0, 30.0);
        let frame = |ui: &mut Ui, sel: &mut usize| {
            ui.begin(Vec2::new(800.0, 600.0), 1.0, 1.0 / 60.0);
            ui.select("n", field, sel, &options);
            ui.finish();
        };
        // open it and let it finish opening
        ui.input.mouse = field.center();
        ui.input.pressed = true;
        ui.input.down = true;
        frame(&mut ui, &mut sel);
        ui.input.down = false;
        ui.input.released = true;
        frame(&mut ui, &mut sel);
        for _ in 0..30 {
            frame(&mut ui, &mut sel);
        }
        let r = popup_rect(ui.popup.as_ref().unwrap(), ui.size);
        assert_eq!(ui.popup.as_ref().unwrap().scroll, 0.0);
        // take the thumb at the top and pull it down to the end of the track
        ui.input.mouse = Vec2::new(r.right() - 4.0, r.y + 5.0);
        ui.input.pressed = true;
        ui.input.down = true;
        frame(&mut ui, &mut sel);
        ui.input.mouse = Vec2::new(r.x + 40.0, r.bottom() + 50.0);
        frame(&mut ui, &mut sel);
        let max = 200.0 * 34.0 - r.h + 8.0;
        assert!((ui.popup.as_ref().unwrap().scroll - max).abs() < 0.5, "scrolled to {}", ui.popup.as_ref().unwrap().scroll);
        // let go over an option: it is not picked, the list stays open
        ui.input.mouse = Vec2::new(r.x + 40.0, r.center().y);
        ui.input.down = false;
        ui.input.released = true;
        frame(&mut ui, &mut sel);
        frame(&mut ui, &mut sel);
        assert_eq!(sel, 0);
        let p = ui.popup.as_ref().expect("the list is still open");
        assert!(p.drag.is_none() && p.picked.is_none());
    }

    /// A finger sliding an open dropdown's list scrolls the list, not the page behind it.
    #[test]
    fn an_open_dropdown_keeps_the_wheel_from_the_page() {
        let options: Vec<String> = (0..50).map(|k| format!("{k}")).collect();
        let mut ui = Ui::new();
        let mut sel = 0;
        let field = Rect::new(20.0, 20.0, 240.0, 30.0);
        ui.input.mouse = field.center();
        for press in [true, false] {
            ui.begin(Vec2::new(400.0, 800.0), 1.0, 1.0 / 60.0);
            (ui.input.pressed, ui.input.released) = (press, !press);
            ui.select("n", field, &mut sel, &options);
            ui.finish();
        }
        let r = popup_rect(ui.popup.as_ref().unwrap(), ui.size);
        ui.begin(Vec2::new(400.0, 800.0), 1.0, 1.0 / 60.0);
        ui.input.mouse = r.center();
        ui.input.wheel.y = -2.0;
        ui.select("n", field, &mut sel, &options);
        assert!(ui.wheel_taken(), "the page would scroll under the list");
        ui.finish();
        assert!(ui.popup.as_ref().unwrap().scroll > 0.0);
        // beside the list the page has it
        ui.begin(Vec2::new(400.0, 800.0), 1.0, 1.0 / 60.0);
        ui.input.mouse = Vec2::new(380.0, 780.0);
        ui.input.wheel.y = -2.0;
        ui.select("n", field, &mut sel, &options);
        assert!(!ui.wheel_taken());
    }

    /// Typing into an open dropdown leaves the options with the text in their name; Enter
    /// takes the first of them, Escape clears the text and then closes the list.
    #[test]
    fn typing_into_a_dropdown_searches_it() {
        let options: Vec<String> = ["Depot", "Bauernhof", "Hauptbahnhof", "Kirche"].iter().map(|s| s.to_string()).collect();
        let mut ui = Ui::new();
        let mut sel = 0;
        let field = Rect::new(20.0, 20.0, 240.0, 30.0);
        let frame = |ui: &mut Ui, sel: &mut usize| {
            ui.begin(Vec2::new(800.0, 600.0), 1.0, 1.0 / 60.0);
            let changed = ui.select("n", field, sel, &options);
            ui.finish();
            changed
        };
        ui.input.mouse = field.center();
        ui.input.pressed = true;
        frame(&mut ui, &mut sel);
        ui.input.released = true;
        frame(&mut ui, &mut sel);
        ui.input.mouse = Vec2::new(700.0, 500.0);
        ui.input.text.push_str("HOF");
        frame(&mut ui, &mut sel);
        assert_eq!(ui.popup.as_ref().unwrap().shown(), vec![1, 2]);
        ui.input.keys.push(Key::Backspace);
        ui.input.text.push_str("f");
        frame(&mut ui, &mut sel);
        ui.input.text.push_str("-x");
        frame(&mut ui, &mut sel);
        assert!(ui.popup.as_ref().unwrap().shown().is_empty());
        ui.input.keys.extend([Key::Backspace, Key::Backspace]);
        frame(&mut ui, &mut sel);
        ui.input.keys.push(Key::Enter);
        assert!(frame(&mut ui, &mut sel), "Enter takes the first match");
        assert_eq!(sel, 1);
        assert!(ui.popup.is_none());
        // Escape: first the text, then the list
        ui.input.mouse = field.center();
        ui.input.pressed = true;
        frame(&mut ui, &mut sel);
        ui.input.released = true;
        frame(&mut ui, &mut sel);
        ui.input.text.push_str("k");
        frame(&mut ui, &mut sel);
        ui.input.keys.push(Key::Escape);
        frame(&mut ui, &mut sel);
        assert!(ui.popup.as_ref().is_some_and(|p| p.query.is_empty()));
        ui.input.keys.push(Key::Escape);
        frame(&mut ui, &mut sel);
        assert!(ui.popup.is_none());
        assert_eq!(sel, 1);
    }
}
