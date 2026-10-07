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
use omsi_ui::paint::{rounded_outline, Align};
use omsi_ui::{Atlas, Color, Fonts, Layer, Painter, Rect, Vertex, Weight};

use super::theme::*;

pub type Id = u64;

/// How many times slower the interface's clock runs than the wall's: 1, or for looking at an
/// animation frame by frame `OMSI_LAUNCHER_SLOW=10` (every animation of the launcher - the
/// springs, the eased values, a ripple, a sheen - takes ten times as long, so that a script's
/// `shot` catches it halfway).
fn slow_motion() -> f32 {
    static SLOW: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SLOW.get_or_init(|| omsi_cfg::env::var("OMSI_LAUNCHER_SLOW").ok().and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| *v >= 1.0).unwrap_or(1.0))
}

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
    pub double_click: bool,
    /// The wheel is a finger dragged over the screen: it scrolls, it never turns a slider or
    /// a time field under the finger.
    pub touch: bool,
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
    /// The size of its options' text (the page's `widget_px` when it opened).
    px: f32,
}

impl Popup {
    /// The options shown (their places in `options`): those with the typed text in them.
    fn shown(&self) -> Vec<usize> {
        let q = self.query.to_lowercase();
        (0..self.options.len()).filter(|&k| q.is_empty() || matches(option_words(&self.options[k]), &q)).collect()
    }
}

/// The mark a dropdown option with a picture begins with (see `picture_option`).
const PICTURE_MARK: char = '\u{1}';

/// A dropdown option shown as a picture before its text (the bus step's display fonts, each
/// with its sign): the texture `tex` of `w` x `h` pixels, or none (yet).
pub fn picture_option(picture: Option<(usize, u32, u32)>, text: &str) -> String {
    match picture {
        Some((tex, w, h)) => format!("{PICTURE_MARK}{tex},{w},{h}{PICTURE_MARK}{text}"),
        None => format!("{PICTURE_MARK}{PICTURE_MARK}{text}"),
    }
}

/// A `picture_option`'s picture (if it has one yet) and text.
fn parse_picture(o: &str) -> Option<(Option<(usize, u32, u32)>, &str)> {
    let rest = o.strip_prefix(PICTURE_MARK)?;
    let (head, text) = rest.split_once(PICTURE_MARK)?;
    let mut n = head.split(',').map(|v| v.parse::<u64>().ok());
    let picture = match (n.next().flatten(), n.next().flatten(), n.next().flatten()) {
        (Some(t), Some(w), Some(h)) => Some((t as usize, w as u32, h as u32)),
        _ => None,
    };
    Some((picture, text))
}

/// What an option says, as typing into an open list finds it (a picture's numbers left out).
fn option_words(o: &str) -> &str {
    parse_picture(o).map(|p| p.1).unwrap_or(o)
}

/// Whether `text` (or its translation) holds the search `q` (in lower case).
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
    /// Sprung values (`spring`): where each is and how fast it goes.
    springs: HashMap<Id, Spring>,
    /// Tiles' clicks still rippling (`tile`): where on the tile, and when.
    ripples: HashMap<Id, (Vec2, f32)>,
    /// Primary buttons' sheen: when it began to sweep, and whether the mouse was on the button
    /// the frame before (a sweep begins as the mouse comes onto it).
    sweeps: HashMap<Id, (f32, bool)>,
    /// The row drawn last (`row`): the radio drawn in it animates with it.
    row_id: Option<Id>,
    /// Something is still moving this frame (an eased value on its way, an animation that
    /// asked with `keep_moving`): the launcher then draws at the screen's rate even when idle.
    pub moving: bool,
    /// The slider held this frame (see `slider_on_release`).
    slider_held: Option<Id>,
    /// Animations wanted (the setting `animations`): off, everything is where it ends at once.
    pub motion: bool,
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
    tooltip: Option<(String, Vec2, f32)>,
    /// The least size of the controls' words (labels, buttons, fields, lists, tooltips): 0
    /// for their own; a page that wants larger text sets it while it draws (the bus company's
    /// pages). Back to 0 at every frame's start.
    pub widget_px: f32,
    /// (tests) Where each clickable widget was this frame, by id.
    #[cfg(test)]
    pub drawn: HashMap<Id, Rect>,
}

impl Ui {
    pub fn new() -> Ui {
        Ui {
            fonts: Fonts::hanken(),
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
            springs: HashMap::new(),
            ripples: HashMap::new(),
            sweeps: HashMap::new(),
            row_id: None,
            moving: false,
            slider_held: None,
            motion: true,
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
            widget_px: 0.0,
            #[cfg(test)]
            drawn: HashMap::new(),
        }
    }

    /// Start a frame: the window's size in points and its scale.
    pub fn begin(&mut self, size: Vec2, scale: f32, dt: f32) {
        self.size = size;
        self.scale = scale;
        self.dt = (dt / slow_motion()).clamp(0.0, 0.1);
        self.time += self.dt;
        self.moving = false;
        self.slider_held = None;
        self.atlas.begin_frame();
        self.layers.clear();
        self.clip_stack.clear();
        self.hot = None;
        self.row_id = None;
        self.wheel_taken = false;
        self.cursor = winit::window::CursorIcon::Default;
        self.over_ui = false;
        self.tooltip = None;
        self.widget_px = 0.0;
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

    /// [`Ui::image`] seen through: at `alpha` (0 gone, 1 whole).
    pub fn image_faded(&mut self, r: Rect, tex: usize, radius: f32, alpha: f32) {
        let clip = intersect(self.clip_now(), r);
        self.push_layer(clip, radius);
        self.layers.last_mut().unwrap().2 = tex;
        let sprite = omsi_ui::Sprite { uv: [0.0, 0.0, 1.0, 1.0], w: r.w, h: r.h, ascent: 0.0 };
        self.p().sprite(sprite, Vec2::new(r.x, r.y), Vec2::new(r.w, r.h), Color::WHITE.alpha(alpha));
        let (c, rad) = self.clip_stack.last().copied().unwrap_or((Rect::new(0.0, 0.0, self.size.x, self.size.y), 0.0));
        self.push_layer(c, rad);
    }

    /// A picture of `tex` (`w` x `h` pixels) covering `r` as a photo covers a tile: scaled
    /// to fill it, what sticks out cut off evenly on both sides, its corners rounded.
    pub fn image_cover(&mut self, r: Rect, tex: usize, radius: f32, w: u32, h: u32) {
        self.image_cover_at(r, tex, radius, w, h, 1.0, Vec2::ZERO);
    }

    /// `image_cover` brought `zoom` times closer and its picture moved by `shift` points (as
    /// far as the picture reaches: no edge of it comes into view).
    #[allow(clippy::too_many_arguments)]
    pub fn image_cover_at(&mut self, r: Rect, tex: usize, radius: f32, w: u32, h: u32, zoom: f32, shift: Vec2) {
        let uv = cover_uv(r.w, r.h, w, h, zoom, shift);
        let clip = intersect(self.clip_now(), r);
        self.push_layer(clip, radius);
        self.layers.last_mut().unwrap().2 = tex;
        let sprite = omsi_ui::Sprite { uv, w: r.w, h: r.h, ascent: 0.0 };
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
        let k = if self.motion { 1.0 - (-self.dt / tau.max(1e-3)).exp() } else { 1.0 };
        let v = self.anims.entry(id).or_insert(to);
        *v += (to - *v) * k;
        if (to - *v).abs() > 1e-3 {
            self.moving = true;
        } else {
            *v = to;
        }
        *v
    }

    /// An animation of one's own runs this frame: keep drawing at the screen's rate.
    pub fn keep_moving(&mut self) {
        self.moving = true;
    }

    /// A sprung value for widget part `id` going to `to`: it moves as a weight on a spring
    /// does, `feel` saying how quick and how damped, and takes its speed along when `to`
    /// changes on the way. Without animations (or first seen, or back after a while unseen)
    /// it is at `to` at once.
    pub fn spring(&mut self, id: Id, to: f32, feel: Feel) -> f32 {
        let now = self.time;
        let s = self.springs.entry(id).or_insert(Spring { x: to, v: 0.0, seen: now });
        // (not drawn for a while - another page, a list scrolled past: it comes back where it
        // is going, not halfway from where it was left)
        if !self.motion || now - s.seen > 0.25 {
            (s.x, s.v) = (to, 0.0);
        }
        s.seen = now;
        let (d, v) = spring_step(s.x - to, s.v, self.dt, feel);
        if d.abs() < 2e-3 && v.abs() < 2e-2 {
            (s.x, s.v) = (to, 0.0);
        } else {
            (s.x, s.v) = (to + d, v);
            self.moving = true;
        }
        s.x
    }

    /// Where the sprung value `id` is now (without moving it), if it has been used.
    pub fn spring_at(&self, id: Id) -> Option<f32> {
        self.springs.get(&id).map(|s| s.x)
    }

    /// A tile that is clicked: `interact` on `base` (which stays where it is) and how it is
    /// drawn this frame - risen, grown, followed by the light - see [`TileMotion`].
    pub fn tile(&mut self, id: Id, base: Rect, radius: f32) -> TileMotion {
        let hit = self.interact(id, base);
        self.tile_from(id, base, radius, hit)
    }

    /// A tile that only shows (a figure of the service record): it moves under the mouse but
    /// is no button (no pointer, no press).
    pub fn card_tile(&mut self, id: Id, base: Rect, radius: f32) -> TileMotion {
        let h = self.hover(base);
        self.tile_from(id, base, radius, (h, false, false))
    }

    /// A tile's motion from its (hovered, held, clicked) as the caller worked them out (a map
    /// on a server's map step is under the mouse but cannot be chosen: all false).
    pub fn tile_from(&mut self, id: Id, base: Rect, radius: f32, (h, held, clicked): (bool, bool, bool)) -> TileMotion {
        let lift = self.spring(id ^ 0x7113_0001, if h { 1.0 } else { 0.0 }, Feel::LIFT);
        let press = self.spring(id ^ 0x7113_0002, if held { 1.0 } else { 0.0 }, Feel::PRESS);
        // where the mouse is on it, followed; when it leaves, the light stays where it left
        // and fades there
        let here = aim_on(base, self.input.mouse);
        let (ax, ay) = (id ^ 0x7113_0003, id ^ 0x7113_0004);
        let hold = Vec2::new(self.spring_at(ax).unwrap_or(here.x), self.spring_at(ay).unwrap_or(here.y));
        let want = if h { here } else { hold };
        let aim = Vec2::new(self.spring(ax, want.x, Feel::FOLLOW), self.spring(ay, want.y, Feel::FOLLOW));
        // (a tile under the mouse is only highlighted - its edge lit, evenly: no rising, no
        // tilt, no sheen following the mouse, no ripple - players found the motion too busy)
        let motion = self.motion && TILE_MOTION;
        let hover = lift.clamp(0.0, 1.0);
        let rise = (lift * (1.0 - press.clamp(0.0, 1.0))).max(-0.05);
        let r = if motion { tile_rect(base, rise, press) } else { base };
        if clicked && motion {
            // (the ripples of tiles gone since - a click that went to another page - are let go)
            let now = self.time;
            self.ripples.retain(|_, (_, t0)| now - *t0 < RIPPLE_S);
            self.ripples.insert(id, (self.input.mouse - Vec2::new(base.x, base.y), now));
        }
        let ripple = match self.ripples.get(&id).copied() {
            Some((at, t0)) if self.time - t0 < RIPPLE_S && motion => {
                self.keep_moving();
                Some((at, (self.time - t0) / RIPPLE_S))
            }
            Some(_) => {
                self.ripples.remove(&id);
                None
            }
            None => None,
        };
        let k = Vec2::new(r.w / base.w.max(1.0), r.h / base.h.max(1.0));
        TileMotion {
            hovered: h,
            clicked,
            base,
            r,
            radius,
            hover,
            rise: if motion { rise } else { 0.0 },
            tilt: aim * hover,
            light: r.center() + aim * Vec2::new(r.w, r.h) * 0.5,
            ripple: ripple.map(|(at, t)| (Vec2::new(r.x, r.y) + at * k, t)),
            motion,
        }
    }

    /// Under a tile risen by the mouse: its shadow, deeper and softer the higher it is and a
    /// little away from the mouse (the light is where the mouse is). Nothing at rest: the
    /// sheets keep their one elevation.
    pub fn tile_shadow(&mut self, t: &TileMotion) {
        if !t.motion || t.rise < 0.01 {
            return;
        }
        let k = t.rise.min(1.2);
        let r = Rect::new(t.r.x - t.tilt.x * 2.0, t.r.y + 1.0 + 4.0 * k - t.tilt.y * 1.5, t.r.w, t.r.h).inset(1.0);
        self.p().shadow(r, t.radius, 8.0 + 6.0 * k, Color::rgba(0, 0, 0, 0.42 * k.min(1.0)));
    }

    /// A tile's photo in `area` (in the tile as drawn, its corners rounded by `radius`): a
    /// little closer under the mouse and shifted against it, as if it lay deeper than the
    /// tile's edge; the zoom keeps the photo's own edges out of sight.
    pub fn tile_photo(&mut self, t: &TileMotion, area: Rect, radius: f32, tex: usize, w: u32, h: u32) {
        let (zoom, shift) = if t.motion { (1.0 + PHOTO_ZOOM * t.hover, -t.tilt * PHOTO_SHIFT) } else { (1.0, Vec2::ZERO) };
        self.image_cover_at(area, tex, radius, w, h, zoom, shift);
    }

    /// The light on a tile under the mouse: a soft sheen where the mouse is (`strength`, the
    /// white's alpha in its middle) and a click's ripple running out from where it was
    /// clicked. Both are cut to the tile's rounded shape by arithmetic rather than by a clip -
    /// a clip is a layer of its own, and a grid of tiles would use them up.
    pub fn tile_light(&mut self, t: &TileMotion, strength: f32) {
        if !t.motion {
            return;
        }
        let shape = rounded_outline(t.r, t.radius);
        if t.hover > 0.005 && strength > 0.0 {
            let reach = SHEEN_REACH * t.r.w.max(t.r.h);
            spot(self.p(), &shape, t.light, reach, LIGHT.alpha(strength * t.hover), &SHEEN);
        }
        if let Some((at, k)) = t.ripple {
            let (radius, a) = ripple_at(t.r, at, k);
            // (a soft edge of a fixed width, whatever the size it has grown to)
            let edge = ((radius - 16.0) / radius.max(1.0)).max(0.0);
            spot(self.p(), &shape, at, radius, LIGHT.alpha(0.2 * a), &[(0.0, 0.5), (edge, 1.0), (1.0, 0.0)]);
        }
    }

    /// A tile's edge, `width` wide: `rest` while the mouse is away; under it the whole edge
    /// goes towards the route's blue, brightest nearest the mouse.
    pub fn tile_edge(&mut self, t: &TileMotion, width: f32, rest: Color) {
        let k = t.hover;
        let rad = t.radius.min(t.r.w * 0.5).min(t.r.h * 0.5);
        // (without animations the hover's edge is lit evenly: no glint following the mouse)
        if k < 0.005 || rad < width + 0.5 || !t.motion {
            self.p().rounded_border(t.r, t.radius, width, rest.mix(edge_lit(), k));
            return;
        }
        let reach = EDGE_REACH * t.r.w.max(t.r.h);
        let pts = outline_dense(t.r, t.radius, 10.0);
        // (nearest the mouse the edge is brighter and half as wide again: a glint)
        let lit = |q: Vec2| {
            let near = k * smoothstep(1.0 - ((q - t.light).length() / reach).min(1.0));
            (rest.mix(edge_lit(), k).mix(edge_lit_near(), near), width * (1.0 + 0.5 * near))
        };
        let n = pts.len();
        let p = self.p();
        for j in 0..n {
            let (a, na) = pts[j];
            let (b, nb) = pts[(j + 1) % n];
            let ((ca, wa), (cb, wb)) = (lit(a), lit(b));
            let (ai, bi) = (a - na * wa, b - nb * wb);
            p.tri(ai, a, b, ca, ca, cb);
            p.tri(ai, b, bi, ca, cb, cb);
        }
    }

    /// The round mark of a list's row (`flow::radio`): a ring, and when chosen a dot that pops
    /// in - sprung with the row drawn last.
    pub fn radio(&mut self, c: Vec2, on: bool) {
        let target = if on { 1.0 } else { 0.0 };
        let k = match self.row_id {
            Some(id) => self.spring(id ^ 0x7ad1_0001, target, Feel::POP),
            None => target,
        };
        let m = k.clamp(0.0, 1.0);
        let p = self.p();
        // (a ring with the row showing through it: the row's fill fading to or from the blue
        // meets no disc of another colour)
        if m > 0.0 {
            p.circle(c, 6.5, accent().alpha(m));
        }
        p.arc(c, 6.5, 8.0, 0.0, std::f32::consts::TAU, TEXT_DIM.mix(on_accent(), m));
        if k > 0.02 {
            p.circle(c, 3.5 * k, on_accent());
        }
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
                self.tooltip = Some((text.to_string(), self.input.mouse, self.wpx(12.5)));
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

    /// Text turned by `angle` (radians) about its middle `at`.
    #[allow(clippy::too_many_arguments)]
    pub fn text_rotated(&mut self, text: &str, px: f32, weight: Weight, at: Vec2, angle: f32, c: Color) {
        let Ui { atlas, fonts, layers, .. } = self;
        layers.last_mut().unwrap().1.text_rotated(atlas, fonts, text, px, weight, at, angle, c);
    }

    pub fn icon(&mut self, name: &str, center: Vec2, size: f32, c: Color) {
        let Ui { atlas, layers, .. } = self;
        layers.last_mut().unwrap().1.icon(atlas, name, center, size, c)
    }

    pub fn width(&self, text: &str, px: f32, weight: Weight) -> f32 {
        self.fonts.width(text, px, weight)
    }

    /// A control's text size: its own, or the page's least (`widget_px`).
    pub fn wpx(&self, px: f32) -> f32 {
        px.max(self.widget_px)
    }

    // --- surfaces ------------------------------------------------------------------------

    /// A panel: flat, dark, a hairline edge.
    pub fn panel(&mut self, r: Rect) {
        self.solid(r);
        let p = self.p();
        p.rounded(r, SHEET_RADIUS, PANEL);
        p.rounded_border(r, SHEET_RADIUS, 1.0, EDGE);
    }

    /// A section of a sheet, as Omsi-Hub draws one: no fill of its own (the sheet under it is
    /// enough), only a hairline round it.
    pub fn card(&mut self, r: Rect) {
        self.solid(r);
        self.p().rounded_border(r, RADIUS, 1.0, HAIRLINE);
    }

    /// A section heading inside a panel: an accent tick and the title in capitals.
    pub fn heading(&mut self, r: Rect, title: &str, icon: Option<&str>) -> Rect {
        let _ = icon;
        self.text(&omsi_ui::tr(title).to_uppercase(), Vec2::new(r.x, r.y + 14.0), 11.0, Weight::Bold, TEXT_DIM, Align::Left);
        Rect::new(r.x, r.y + 26.0, r.w, (r.h - 26.0).max(0.0))
    }

    pub fn label(&mut self, r: Rect, text: &str) {
        let px = self.wpx(13.0);
        self.text_in(text, r, px, Weight::Medium, TEXT_DIM, Align::Left);
    }

    // --- controls ------------------------------------------------------------------------

    /// A button. `primary` is the accent-coloured one.
    pub fn button(&mut self, name: &str, r: Rect, label: &str, icon: Option<&str>, kind: ButtonKind) -> bool {
        let id = id_of(name);
        let (h, held, clicked) = self.interact(id, r);
        let t = self.anim(id, if h { 1.0 } else { 0.0 }, 0.06);
        // (the big action in the corner: its own corner and its word in capitals, as on a
        // ticket machine; a button in a sheet is a control like the others)
        let big = r.h >= 44.0;
        // under the mouse it rises (a ghost, being no surface, only lights up) and brightens;
        // pressed it goes down a little and comes back with a small bounce. The hit area is
        // `r` throughout.
        let lifts = kind != ButtonKind::Ghost;
        let rise = self.spring(id ^ 0xb077_0001, if h && !held && lifts { 1.0 } else { 0.0 }, Feel::LIFT);
        let press = self.spring(id ^ 0xb077_0002, if held { 1.0 } else { 0.0 }, Feel::PRESS);
        let (rise, press) = if self.motion && TILE_MOTION { (rise, press) } else { (0.0, 0.0) };
        let rr = button_rect(r, rise * if big { 2.0 } else { 1.0 }, press);
        let down = press.clamp(0.0, 1.0);
        let (fill, text_c, edge) = match kind {
            ButtonKind::Primary => (accent().mix(accent().lighten(0.1), t).mix(accent_deep(), down), on_accent(), Color::CLEAR),
            ButtonKind::Danger => (PANEL.mix(HOVER, t), DANGER, DANGER.alpha(0.35 + 0.3 * t)),
            ButtonKind::Normal => (PANEL.mix(HOVER, t), TEXT, EDGE.mix(Color::WHITE.alpha(0.16), t)),
            ButtonKind::Ghost => (Color::WHITE.alpha(0.05 * t), if h { TEXT } else { TEXT_DIM }, Color::CLEAR),
        };
        let rad = if big { ACTION_RADIUS } else { RADIUS };
        if big && lifts && rise > 0.01 {
            let k = rise.min(1.2);
            self.p().shadow(Rect::new(rr.x, rr.y + 2.0 + 3.0 * k, rr.w, rr.h).inset(2.0), rad, 10.0 + 6.0 * k, Color::rgba(0, 0, 0, 0.32 * k.min(1.0)));
        }
        self.p().rounded(rr, rad, fill);
        if edge.0[3] > 0.0 {
            self.p().rounded_border(rr, rad, 1.0, edge);
        }
        if kind == ButtonKind::Primary {
            self.sweep(id, rr, rad, h);
        }
        let caps = big && kind == ButtonKind::Primary && rr.h >= 50.0;
        let shown = if caps { omsi_ui::tr(label).to_uppercase() } else { label.to_string() };
        let label = shown.as_str();
        let px = self.wpx(if caps { 18.0 } else if big { 14.5 } else { 13.0 });
        let weight = if kind == ButtonKind::Ghost { Weight::Medium } else { Weight::Bold };
        let tw = self.width(label, px, weight);
        // (the gap after the icon only when a label follows it)
        let iw = if icon.is_some() { px * 1.3 + if label.is_empty() { 0.0 } else { 6.0 } } else { 0.0 };
        let x0 = rr.center().x - (tw + iw) * 0.5;
        if let Some(i) = icon {
            // (the icon a step the way it points: Next forward, Back back)
            let nudge = if lifts { icon_way(i) * 2.5 * rise.clamp(0.0, 1.2) } else { 0.0 };
            self.icon(i, Vec2::new(x0 + px * 0.65 + nudge, rr.center().y), px * 1.3, text_c);
        }
        self.text_in(label, Rect::new(x0 + iw, rr.y, tw + 2.0, rr.h), px, weight, text_c, Align::Left);
        clicked
    }

    /// A primary button's sheen: a slanted band of light crossing it once as the mouse comes
    /// onto it (it finishes its way when the mouse leaves meanwhile), cut to its corners.
    fn sweep(&mut self, id: Id, r: Rect, radius: f32, hovered: bool) {
        let now = self.time;
        let (mut start, was) = self.sweeps.get(&id).copied().unwrap_or((f32::MIN, false));
        if hovered && !was && now - start > SWEEP_S && self.motion && TILE_MOTION {
            start = now;
        }
        self.sweeps.insert(id, (start, hovered));
        let t = (now - start) / SWEEP_S;
        if !(0.0..1.0).contains(&t) || !self.motion {
            return;
        }
        self.keep_moving();
        let shape = rounded_outline(r, radius);
        let (left, right, top, bottom, half) = sweep_band(r, t);
        // (brightest along the band's slanted middle line, nothing at its sides)
        let light = Color::WHITE.alpha(0.2);
        let tint = |q: Vec2| {
            let f = ((q.y - r.y) / r.h.max(1.0)).clamp(0.0, 1.0);
            let mid = top + (bottom - top) * f;
            light.alpha((1.0 - (q.x - mid).abs() / half).clamp(0.0, 1.0))
        };
        let p = self.p();
        shade(p, &left, &shape, &tint);
        shade(p, &right, &shape, &tint);
    }

    /// A round button with only an icon.
    pub fn icon_button(&mut self, name: &str, c: Vec2, r: f32, icon: &str, tip: &str) -> bool {
        let id = id_of(name);
        let rect = Rect::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r);
        let (h, held, clicked) = self.interact(id, rect);
        let t = self.anim(id, if h { 1.0 } else { 0.0 }, 0.08);
        // (its round light grows in from the middle, and gives a little under a press)
        let press = self.spring(id ^ 0xb077_0002, if held { 1.0 } else { 0.0 }, Feel::PRESS);
        let grow = if self.motion { 0.8 + 0.2 * t - 0.08 * press } else { 1.0 };
        self.p().circle(c, r * grow, Color::WHITE.alpha(0.06 * t + 0.04 * press.clamp(0.0, 1.0)));
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
        // (the knob springs across, the track's blue coming with it; under the mouse the knob
        // swells a little)
        let on = self.spring(id ^ 0x70661e, if *value { 1.0 } else { 0.0 }, Feel::KNOB);
        let lit = self.anim(id, if h { 1.0 } else { 0.0 }, 0.08);
        let tw = 34.0;
        let th = 18.0;
        let track = Rect::new(r.right() - tw, r.y + (r.h - th) * 0.5, tw, th);
        self.p().rounded(track, th * 0.5, Color::WHITE.alpha(0.16 + 0.06 * lit).mix(accent(), on.clamp(0.0, 1.0)));
        let kx = track.x + th * 0.5 + (tw - th) * on;
        self.p().circle(Vec2::new(kx, track.center().y), th * 0.5 - 3.0 + 0.8 * lit, Color::rgba(240, 240, 240, 1.0));
        let px = self.wpx(13.0);
        self.text_in(label, Rect::new(r.x, r.y, r.w - tw - 10.0, r.h), px, Weight::Regular, if h { TEXT } else { TEXT_SOFT }, Align::Left);
        clicked
    }

    /// A slider over `[min, max]` rounded to `step`; `fmt` makes the value's text.
    #[allow(clippy::too_many_arguments)]
    /// A slider whose value is only given out once the drag ends: while it is held it shows
    /// the value it will take, let go it returns true with `value` set (a step of the wheel at
    /// once). For what changes the interface itself - the launcher's size: given out while
    /// dragged, the whole window, the slider with it, scaled under the mouse.
    pub fn slider_on_release(&mut self, name: &str, r: Rect, value: &mut f32, min: f32, max: f32, step: f32, label: &str, fmt: &dyn Fn(f32) -> String) -> bool {
        let id = id_of(name);
        let key = id ^ 0x5e71_7e5e;
        let mut v = self.anims.get(&key).copied().unwrap_or(*value);
        let changed = self.slider(name, r, &mut v, min, max, step, label, fmt);
        if self.slider_held == Some(id) {
            self.anims.insert(key, v);
            return false;
        }
        let dragged = self.anims.remove(&key).is_some();
        if (dragged || changed) && (v - *value).abs() > 1e-6 {
            *value = v;
            return true;
        }
        false
    }

    pub fn slider(&mut self, name: &str, r: Rect, value: &mut f32, min: f32, max: f32, step: f32, label: &str, fmt: &dyn Fn(f32) -> String) -> bool {
        let id = id_of(name);
        let label_w = if label.is_empty() { 0.0 } else { (r.w * 0.38).min(170.0) };
        let val_w = 58.0;
        let track_r = Rect::new(r.x + label_w, r.y, r.w - label_w - val_w, r.h);
        let (h, held, _) = self.interact(id, track_r.pad(-8.0, 0.0));
        let before = *value;
        if held {
            self.slider_held = Some(id);
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
            let px = self.wpx(13.0);
            self.text_in(label, Rect::new(r.x, r.y, label_w - 8.0, r.h), px, Weight::Regular, TEXT_SOFT, Align::Left);
        }
        let cy = track_r.center().y;
        // (Omsi-Hub's: the slider is the line, four points thick, the knob sitting on it)
        let th = 4.0;
        let track = Rect::new(track_r.x, cy - th * 0.5, track_r.w, th);
        self.p().rounded(track, th * 0.5, Color::WHITE.alpha(0.12));
        self.p().rounded(Rect::new(track.x, track.y, track.w * shown, th), th * 0.5, accent());
        let kc = Vec2::new(track.x + track.w * shown, cy);
        let swell = self.spring(id ^ 0x5110e, if held { 1.6 } else if h { 1.0 } else { 0.0 }, Feel::LIFT);
        let halo = ((swell - 1.0) / 0.6).clamp(0.0, 1.0);
        if halo > 0.005 {
            // (a halo round the knob while it is held)
            self.p().circle(kc, 9.0 + 5.0 * halo, accent().alpha(0.2 * halo));
        }
        self.p().circle(kc, 8.0 + swell * 0.9, accent());
        let txt = fmt(*value);
        let px = self.wpx(12.5);
        self.text_in(&txt, Rect::new(r.right() - val_w + 8.0, r.y, val_w - 8.0, r.h), px, Weight::Medium, TEXT, Align::Right);
        *value != before
    }

    /// Buttons side by side, one of them chosen.
    pub fn segmented(&mut self, name: &str, r: Rect, selected: &mut usize, labels: &[&str]) -> bool {
        self.segmented_some(name, r, selected, labels, &[])
    }

    /// `segmented` with segments that cannot be chosen now (`off`): faint, and not clicked
    /// (the caller may say why).
    pub fn segmented_some(&mut self, name: &str, r: Rect, selected: &mut usize, labels: &[&str], off: &[usize]) -> bool {
        let n = labels.len().max(1);
        let id = id_of(name);
        self.p().rounded(r, r.h * 0.5, FIELD);
        let w = r.w / n as f32;
        let mut changed = false;
        let mut hovered = vec![false; labels.len()];
        for (k, h) in hovered.iter_mut().enumerate() {
            if off.contains(&k) {
                continue;
            }
            let cell = Rect::new(r.x + w * k as f32, r.y, w, r.h);
            let (hk, _, clicked) = self.interact(id ^ (k as u64 + 11), cell);
            *h = hk;
            if clicked && *selected != k {
                *selected = k;
                changed = true;
            }
        }
        // the chosen segment's pill slides to the one clicked
        let at = self.spring(id ^ 0x5e6, *selected as f32, Feel::SLIDE);
        let knob = Rect::new(r.x + 3.0 + w * at, r.y + 3.0, w - 6.0, r.h - 6.0);
        self.p().rounded(knob, knob.h * 0.5, accent());
        for (k, l) in labels.iter().enumerate() {
            let cell = Rect::new(r.x + w * k as f32, r.y, w, r.h);
            let h = hovered[k];
            let c = if *selected == k { on_accent() } else if off.contains(&k) { TEXT_FAINT } else if h { TEXT } else { TEXT_DIM };
            let px = self.wpx(12.5);
            self.text_in(l, cell.pad(4.0, 0.0), px, if *selected == k { Weight::Bold } else { Weight::Medium }, c, Align::Center);
        }
        changed
    }

    /// Chips, as Omsi-Hub's tabs are: each word in a pill of its own, side by side from the
    /// left of `r`, the chosen one filled with the route's blue. Returns true when another
    /// was chosen.
    pub fn chips(&mut self, name: &str, r: Rect, selected: &mut usize, labels: &[&str]) -> bool {
        let id = id_of(name);
        let mut changed = false;
        let (mut x, mut y) = (r.x, r.y);
        let mut cells = Vec::with_capacity(labels.len());
        for (k, label) in labels.iter().enumerate() {
            let w = self.width(label, self.wpx(13.0), Weight::Bold) + 30.0;
            // (more than a row holds - a phone - goes on in the next)
            if x > r.x && x + w > r.right() {
                x = r.x;
                y += r.h + 8.0;
            }
            let cell = Rect::new(x, y, w, r.h);
            let (h, _, clicked) = self.interact(id ^ (k as u64 + 11), cell);
            if clicked && *selected != k {
                *selected = k;
                changed = true;
            }
            cells.push((cell, h));
            x = cell.right() + 8.0;
        }
        for (k, (cell, h)) in cells.iter().enumerate() {
            let t = self.anim(id ^ (k as u64 + 101), if *h { 1.0 } else { 0.0 }, 0.08);
            if *selected != k {
                self.p().rounded(*cell, cell.h * 0.5, HOVER.alpha(t));
                self.p().rounded_border(*cell, cell.h * 0.5, 1.0, HAIRLINE.mix(Color::WHITE.alpha(0.2), t));
            }
        }
        // the chosen one's pill slides over from the one chosen before, its ends each on a
        // spring of their own: the end leading the way goes first, the other follows, so the
        // pill stretches on its way and settles to the chip's width
        if let Some((cell, _)) = cells.get(*selected) {
            let pill = self.slide_span(id ^ 0xc41b, cell.x, cell.right());
            let py = self.spring(id ^ 0xc41b ^ 7, cell.y, Feel::SLIDE);
            self.p().rounded(Rect::new(pill.0, py, pill.1 - pill.0, cell.h), cell.h * 0.5, accent());
        }
        for (k, (label, (cell, _))) in labels.iter().zip(cells.iter()).enumerate() {
            let on = *selected == k;
            let px = self.wpx(13.0);
            self.text_in(label, *cell, px, if on { Weight::Bold } else { Weight::Medium }, if on { on_accent() } else { TEXT }, Align::Center);
        }
        changed
    }

    /// A span (a pill, an underline) from `x0` to `x1` sliding to where it is now from where it
    /// was: the end on the side it goes to moves on a quicker spring than the other, so that
    /// it stretches on the way and comes to its length at the end. Returns (x0, x1) this frame.
    pub fn slide_span(&mut self, id: Id, x0: f32, x1: f32) -> (f32, f32) {
        const LEAD: Feel = Feel { response: 0.2, damping: 0.86 };
        const TRAIL: Feel = Feel { response: 0.3, damping: 0.9 };
        let now = self.spring_at(id ^ 1).unwrap_or(x0);
        let right = x0 >= now;
        let a = self.spring(id ^ 1, x0, if right { TRAIL } else { LEAD });
        let b = self.spring(id ^ 2, x1, if right { LEAD } else { TRAIL });
        (a, b.max(a))
    }

    /// How high `chips` comes out in `w` with rows `h` high.
    pub fn chips_height(&self, w: f32, h: f32, labels: &[&str]) -> f32 {
        let (mut x, mut rows) = (0.0, 1.0);
        for label in labels {
            let cw = self.width(label, self.wpx(13.0), Weight::Bold) + 30.0;
            if x > 0.0 && x + cw > w {
                x = 0.0;
                rows += 1.0;
            }
            x += cw + 8.0;
        }
        rows * h + (rows - 1.0) * 8.0
    }

    /// An option of `options` picked from the list a dropdown opened, or nothing.
    fn picked(&mut self, id: Id, selected: &mut usize, options: &[String]) -> bool {
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
        changed
    }

    /// A small button with an icon that opens a dropdown's list (the bar's other languages):
    /// framed when `marked`.
    pub fn menu(&mut self, name: &str, r: Rect, icon: &str, tip: &str, marked: bool, selected: &mut usize, options: &[String]) -> bool {
        let id = id_of(name);
        let changed = self.picked(id, selected, options);
        let (h, _, clicked) = self.interact(id, r);
        let open = self.popup.as_ref().is_some_and(|p| p.id == id);
        if marked || open {
            self.p().rounded_border(r, 6.0, 1.0, accent());
        }
        self.icon(icon, r.center(), 17.0, if h || open || marked { TEXT } else { TEXT_DIM });
        if !open {
            self.tooltip(r, tip);
        }
        if clicked {
            if open {
                self.popup = None;
            } else {
                self.open_popup(id, r, *selected, options);
            }
        } else if let Some(p) = self.popup.as_mut().filter(|p| p.id == id) {
            p.options = options.to_vec();
            p.anchor = r;
        }
        changed
    }

    /// Open the list of a dropdown under (or over) `anchor`, the chosen option in view.
    fn open_popup(&mut self, id: Id, anchor: Rect, selected: usize, options: &[String]) {
        let sel = selected.min(options.len().saturating_sub(1));
        let mut p = Popup { id, anchor, options: options.to_vec(), selected: sel, scroll: 0.0, opened: 0.0, picked: None, drag: None, query: String::new(), px: self.wpx(13.0) };
        let row = 34.0;
        let visible = popup_rect(&p, self.size).h;
        p.scroll = ((sel as f32 + 0.5) * row - visible * 0.5).clamp(0.0, (options.len() as f32 * row - visible).max(0.0));
        self.popup = Some(p);
        self.date_popup = None;
    }

    /// A dropdown's option: its text, or - a player's line (`ownlines::plate_option`) - the
    /// line's plate in its colour and the text after it.
    fn option_text(&mut self, o: &str, r: Rect, px: f32, ink: Color) {
        if let Some((picture, rest)) = parse_picture(o) {
            // (a sign's picture on its dark panel, as wide as the slot lets it be)
            let slot = Rect::new(r.x - 4.0, r.y + 5.0, (r.w * 0.5).min(150.0), (r.h - 10.0).max(8.0));
            self.p().rounded(slot, 4.0, Color::rgba(8, 8, 10, 1.0));
            if let Some((tex, w, h)) = picture.filter(|p| p.1 > 0 && p.2 > 0) {
                let room = slot.inset(3.0);
                let k = (room.h / h as f32).min(room.w / w as f32);
                let (pw, ph) = (w as f32 * k, h as f32 * k);
                self.image(Rect::new(room.x, room.center().y - ph * 0.5, pw, ph), tex, 0.0);
            }
            let x = slot.right() + 10.0;
            self.text_in(rest, Rect::new(x, r.y, (r.right() - x).max(0.0), r.h), px, Weight::Regular, ink, Align::Left);
            return;
        }
        match super::ownlines::parse_plate(o) {
            Some((colour, number, rest)) => {
                let h = (r.h - 14.0).clamp(14.0, 20.0);
                let w = super::ownlines::plate(self, Vec2::new(r.x, r.center().y - h * 0.5), number, colour, h);
                self.text_in(rest, Rect::new(r.x + w + 8.0, r.y, (r.w - w - 8.0).max(0.0), r.h), px, Weight::Regular, ink, Align::Left);
            }
            None => {
                self.text_in(o, r, px, Weight::Regular, ink, Align::Left);
            }
        }
    }

    /// A dropdown: shows the chosen option, opens a list over everything else.
    pub fn select(&mut self, name: &str, r: Rect, selected: &mut usize, options: &[String]) -> bool {
        let id = id_of(name);
        let changed = self.picked(id, selected, options);
        let (h, _, clicked) = self.interact(id, r);
        let open = self.popup.as_ref().map(|p| p.id == id).unwrap_or(false);
        let t = self.anim(id, if h || open { 1.0 } else { 0.0 }, 0.08);
        self.p().rounded(r, RADIUS, FIELD.mix(HOVER, t));
        self.p().rounded_border(r, RADIUS, 1.0, if open { accent().alpha(0.7) } else { EDGE });
        let txt = options.get(*selected).cloned().unwrap_or_default();
        let px = self.wpx(13.0);
        self.option_text(&txt, Rect::new(r.x + 12.0, r.y, r.w - 40.0, r.h), px, TEXT);
        let rot = self.anim(id ^ 9, if open { 1.0 } else { 0.0 }, 0.08);
        self.icon(if rot > 0.5 { "expand_less" } else { "expand_more" }, Vec2::new(r.right() - 18.0, r.center().y), 20.0, if h { TEXT } else { TEXT_DIM });
        if clicked {
            // (a phone's tap can come twice - as a touch and as the mouse click made of it:
            // the second one closed the list it had just opened)
            if open && self.popup.as_ref().is_some_and(|p| p.opened >= 1.0) {
                self.popup = None;
            } else if open {
            } else {
                self.open_popup(id, r, *selected, options);
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
        self.p().rounded(r, RADIUS, FIELD.mix(HOVER, t * 0.5));
        self.p().rounded_border(r, RADIUS, 1.0, if focused { accent().alpha(0.7) } else { EDGE });
        let mut x = r.x + 12.0;
        if let Some(i) = icon {
            self.icon(i, Vec2::new(x + 8.0, r.center().y), 18.0, TEXT_DIM);
            x += 24.0;
        }
        let inner = Rect::new(x, r.y, r.right() - x - 10.0, r.h);
        self.push_clip(inner, 0.0);
        let px = self.wpx(13.0);

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
                        accent().alpha(0.35),
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
                    accent(),
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
        self.p().rounded(r, RADIUS, FIELD);
        self.p().rounded_border(r, RADIUS, 1.0, EDGE);
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
        self.p().rounded(r, RADIUS, FIELD.mix(HOVER, t));
        self.p().rounded_border(r, RADIUS, 1.0, if open { accent().alpha(0.7) } else { EDGE });
        self.icon("calendar_month", Vec2::new(r.x + 20.0, r.center().y), 17.0, TEXT_DIM);
        let (y, m, d) = parse_date(date);
        let shown = format!("{} {} {}", d, omsi_ui::tr(MONTHS[(m as usize).clamp(1, 12) - 1]), y);
        let px = self.wpx(13.0);
        self.text_in(&shown, Rect::new(r.x + 38.0, r.y, r.w - 60.0, r.h), px, Weight::Regular, TEXT, Align::Left);
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
        self.p().rounded(r, r.h * 0.5, Color::WHITE.alpha(0.1));
        let w = (r.w * frac.clamp(0.0, 1.0)).max(r.h);
        self.p().rounded(Rect::new(r.x, r.y, w, r.h), r.h * 0.5, accent());
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
        // (the hover's fill comes quickly and goes a little slower: a list swept by the mouse
        // leaves a short fading trail rather than a flicker)
        let t = self.anim(id, if h { 1.0 } else { 0.0 }, if h { 0.05 } else { 0.12 });
        let s = self.anim(id ^ 4, if selected { 1.0 } else { 0.0 }, 0.1);
        if t > 0.01 && s < 0.99 {
            self.p().rounded(r, RADIUS, HOVER.alpha(t));
        }
        if s > 0.01 {
            self.p().rounded(r, RADIUS, accent().alpha(s));
        }
        self.row_id = Some(id);
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
        if let Some((t, at, px)) = self.tooltip.take() {
            let w = (self.width(&t, px, Weight::Medium) + 20.0).min(360.0 * px / 12.5);
            let h = self.paragraph_height(&t, w - 20.0, px, Weight::Medium) + 12.0;
            let mut r = Rect::new(at.x + 14.0, at.y + 18.0, w, h);
            if r.right() > self.size.x - 8.0 {
                r.x = self.size.x - 8.0 - r.w;
            }
            if r.bottom() > self.size.y - 8.0 {
                r.y = at.y - 12.0 - r.h;
            }
            self.p().rounded(r, 6.0, FIELD);
            self.p().rounded_border(r, 7.0, 1.0, Color::WHITE.alpha(0.1));
            self.paragraph(&t, Vec2::new(r.x + 10.0, r.y + 4.0), w - 20.0, px, Weight::Medium, TEXT_SOFT);
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
        self.p().rounded(rr, 8.0, Color::rgba(20, 26, 38, e));
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
                self.p().rounded(cell, 5.0, accent());
                self.icon("check", Vec2::new(cell.right() - 16.0, cell.center().y), 15.0, on_accent());
            } else if h {
                self.p().rounded(cell, 5.0, HOVER);
            }
            self.option_text(o, Rect::new(cell.x + 10.0, cell.y, cell.w - 36.0, cell.h), p.px, TEXT);
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
        self.p().rounded(r, 8.0, Color::rgba(20, 26, 38, e));
        self.p().rounded_border(r, 8.0, 1.0, Color::WHITE.alpha(0.1 * e));
        let m = self.input.mouse;
        let click = self.input.released;
        // month header with arrows
        let head = Rect::new(r.x + 8.0, r.y + 8.0, r.w - 16.0, 30.0);
        self.text_in(&format!("{} {}", omsi_ui::tr(MONTHS_LONG[p.month as usize - 1]), p.year), head, 14.0, Weight::Bold, TEXT, Align::Center);
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
            self.icon(icon, b.center(), 20.0, if h { accent() } else { TEXT_DIM });
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
            self.text_in(d, Rect::new(r.x + 8.0 + cw * k as f32, r.y + 42.0, cw, 18.0), 11.0, Weight::Bold, if k >= 5 { accent().alpha(0.8) } else { TEXT_FAINT }, Align::Center);
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
                self.p().rounded(cell, 7.0, accent());
            } else if h {
                self.p().rounded(cell, 7.0, Color::WHITE.alpha(0.09));
            }
            self.text_in(&d.to_string(), cell, 12.5, Weight::Medium, if is_chosen { on_accent() } else { TEXT }, Align::Center);
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

// --- motion ---------------------------------------------------------------------------------
//
// One family of movements for the whole launcher: things under the mouse rise on a spring
// with a touch of overshoot, a part of a control going to its new place slides on a stiffer
// one, a press goes down at once and comes back with a small bounce, and the light follows
// the mouse without ever passing it. Everything lands where it is going and stays there; the
// setting "animations" off puts everything there at once.

/// How a spring moves: the time one swing takes (seconds) and its damping (1 settles without
/// passing its end; below 1 it goes a little past and comes back, as a weight does).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Feel {
    pub response: f32,
    pub damping: f32,
}

impl Feel {
    /// A tile or a button rising under the mouse: there in a sixth of a second, a few percent
    /// past and back.
    pub const LIFT: Feel = Feel { response: 0.30, damping: 0.72 };
    /// A part of a control going to its new place: the chosen chip's pill, the segment, the
    /// step bar's underline.
    pub const SLIDE: Feel = Feel { response: 0.24, damping: 0.86 };
    /// A switch's knob: a slide with a little more swing in it.
    pub const KNOB: Feel = Feel { response: 0.26, damping: 0.68 };
    /// Following the mouse across a tile: smooth, never past it.
    pub const FOLLOW: Feel = Feel { response: 0.36, damping: 1.0 };
    /// A press: down at once, back up with a small bounce.
    pub const PRESS: Feel = Feel { response: 0.14, damping: 0.58 };
    /// A mark appearing (the chosen radio's dot): a little pop.
    pub const POP: Feel = Feel { response: 0.22, damping: 0.55 };
}

/// A sprung value: where it is, how fast it goes, and when it was last used (`Ui::time`).
#[derive(Clone, Copy, Debug)]
struct Spring {
    x: f32,
    v: f32,
    seen: f32,
}

/// One step of a damped spring: its distance `x` from where it goes and its speed `v`, `dt`
/// seconds later. The closed form rather than a numerical step, so that a long frame (the
/// launcher behind another window draws ten a second) cannot make it swing more or blow up.
pub fn spring_step(x: f32, v: f32, dt: f32, feel: Feel) -> (f32, f32) {
    let w = std::f32::consts::TAU / feel.response.max(1e-3);
    let z = feel.damping.max(0.0);
    if z < 0.999 {
        let a = z * w;
        let wd = w * (1.0 - z * z).sqrt();
        let (s, c) = (wd * dt).sin_cos();
        let e = (-a * dt).exp();
        let b = (v + a * x) / wd;
        (e * (x * c + b * s), e * (v * c - (a * v + w * w * x) / wd * s))
    } else if z <= 1.001 {
        let e = (-w * dt).exp();
        let k = v + w * x;
        (e * (x + k * dt), e * (v - w * dt * k))
    } else {
        let q = (z * z - 1.0).sqrt();
        let (r1, r2) = (-w * (z - q), -w * (z + q));
        let c2 = (v - r1 * x) / (r2 - r1);
        let c1 = x - c2;
        let (e1, e2) = ((r1 * dt).exp(), (r2 * dt).exp());
        (c1 * e1 + c2 * e2, r1 * c1 * e1 + r2 * c2 * e2)
    }
}

/// Fast at first, gently into its end.
pub fn ease_out_cubic(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

/// Slowly away, fast through the middle, gently into its end.
pub fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) * 0.5
    }
}

/// 0 at 0, 1 at 1, flat at both ends.
pub fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A tile under the mouse, this frame (`Ui::tile`): what `interact` said of it and how it is
/// drawn. `base` is where it lies and where it is hit; `r` is where it is drawn - risen and
/// grown under the mouse, sunk while pressed - and everything on it is laid out in `r`.
#[derive(Clone, Copy, Debug)]
pub struct TileMotion {
    pub hovered: bool,
    pub clicked: bool,
    pub base: Rect,
    pub r: Rect,
    pub radius: f32,
    /// How far the hover has come, 0 to 1 (for colours).
    pub hover: f32,
    /// How far it has risen (sprung: a little past 1 on the way up; 0 without animations).
    pub rise: f32,
    /// Where the mouse is on it, -1 to 1 each way (followed, and gone with the hover): the
    /// photo shifts against it, the shadow away from it.
    pub tilt: Vec2,
    /// The light's middle: where the mouse is, or where it left the tile.
    pub light: Vec2,
    /// A click's ripple: from where, and how far through (0 to 1).
    pub ripple: Option<(Vec2, f32)>,
    motion: bool,
}

impl TileMotion {
    /// A point of the tile where it lies, where it is drawn.
    pub fn at(&self, p: Vec2) -> Vec2 {
        Vec2::new(self.r.x + (p.x - self.base.x) * self.r.w / self.base.w.max(1.0), self.r.y + (p.y - self.base.y) * self.r.h / self.base.h.max(1.0))
    }

    /// How much bigger it is drawn than it lies (for lengths in it).
    pub fn grown(&self) -> f32 {
        self.r.h / self.base.h.max(1.0)
    }
}

/// How high (points) a tile rises under the mouse, how much it grows, and how much it shrinks
/// while pressed.
/// Tiles and buttons move under the mouse (rise, tilt, sheen, ripple, a button's sweep): off -
/// a highlight is enough (players found the motion too busy).
const TILE_MOTION: bool = false;
const TILE_RISE: f32 = 3.0;
const TILE_GROW: f32 = 0.012;
const TILE_SINK: f32 = 0.02;
/// A tile's photo under the mouse: how much closer, and how far (points) it shifts against
/// the mouse at the tile's edge.
const PHOTO_ZOOM: f32 = 0.06;
const PHOTO_SHIFT: f32 = 10.0;
/// The sheen's reach as a part of the tile's longer side, and how it fades from its middle.
const SHEEN_REACH: f32 = 0.55;
/// The light on a tile: a cool white, as the night's ground and the route's blue light it.
const LIGHT: Color = Color::rgba(214, 228, 255, 1.0);
const SHEEN: [(f32, f32); 5] = [(0.0, 1.0), (0.22, 0.72), (0.5, 0.32), (0.76, 0.08), (1.0, 0.0)];
/// A lit edge: the whole of it under the mouse (the accent, half seen), and nearest the
/// mouse (`accent_2`), as far as this part of the tile's longer side.
fn edge_lit() -> Color {
    accent().alpha(0.4)
}
fn edge_lit_near() -> Color {
    accent_2()
}
const EDGE_REACH: f32 = 0.42;
/// How long a click's ripple runs (seconds).
pub const RIPPLE_S: f32 = 0.55;

/// The tile `base` as drawn when it has risen `rise` (0 to about 1) and is pressed `press`:
/// grown about its middle and lifted, or shrunk while pressed.
pub fn tile_rect(base: Rect, rise: f32, press: f32) -> Rect {
    let s = 1.0 + TILE_GROW * rise - TILE_SINK * press;
    let c = base.center();
    let (w, h) = (base.w * s, base.h * s);
    Rect::new(c.x - w * 0.5, c.y - h * 0.5 - TILE_RISE * rise, w, h)
}

/// Where `m` is on `r`, from -1 (left, top) to 1 (right, bottom).
pub fn aim_on(r: Rect, m: Vec2) -> Vec2 {
    let half = Vec2::new(r.w.max(2.0), r.h.max(2.0)) * 0.5;
    ((m - r.center()) / half).clamp(Vec2::splat(-1.0), Vec2::splat(1.0))
}

/// The part of a `w` x `h` picture that covers an area `aw` x `ah` (as `image_cover` cuts
/// it), `zoom` times closer and its content moved `shift` points - kept inside the picture.
/// Returns [u0, v0, u1, v1].
pub fn cover_uv(aw: f32, ah: f32, w: u32, h: u32, zoom: f32, shift: Vec2) -> [f32; 4] {
    let (iw, ih) = (w.max(1) as f32, h.max(1) as f32);
    let k = (aw / iw).max(ah / ih) * zoom.max(1.0);
    let (sw, sh) = ((aw / (iw * k)).min(1.0), (ah / (ih * k)).min(1.0));
    let u0 = ((1.0 - sw) * 0.5 - shift.x / (iw * k)).clamp(0.0, 1.0 - sw);
    let v0 = ((1.0 - sh) * 0.5 - shift.y / (ih * k)).clamp(0.0, 1.0 - sh);
    [u0, v0, u0 + sw, v0 + sh]
}

/// A ripple from `at` in `r`, `t` (0 to 1) of the way through: its radius - out to the
/// farthest corner, fast at first - and its strength, in at once and fading as it spreads.
pub fn ripple_at(r: Rect, at: Vec2, t: f32) -> (f32, f32) {
    let far = [Vec2::new(r.x, r.y), Vec2::new(r.right(), r.y), Vec2::new(r.x, r.bottom()), Vec2::new(r.right(), r.bottom())].iter().map(|c| (*c - at).length()).fold(0.0, f32::max);
    let t = t.clamp(0.0, 1.0);
    (far * ease_out_cubic(t), (1.0 - t).powi(2) * (t / 0.06).min(1.0))
}

/// Linearly between the points of `profile` ((place, value), places rising from 0 to 1) at
/// `d`; 0 past its end.
pub fn profile_at(profile: &[(f32, f32)], d: f32) -> f32 {
    let Some(first) = profile.first() else { return 0.0 };
    if d <= first.0 {
        return first.1;
    }
    for w in profile.windows(2) {
        let ((d0, a0), (d1, a1)) = (w[0], w[1]);
        if d <= d1 {
            return if d1 - d0 < 1e-6 { a1 } else { a0 + (a1 - a0) * (d - d0) / (d1 - d0) };
        }
    }
    0.0
}

/// Whether `p` lies in the convex polygon `poly` (either way round).
pub fn inside_convex(poly: &[Vec2], p: Vec2) -> bool {
    let s = area_sign(poly);
    (0..poly.len()).all(|k| {
        let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
        s * (b - a).perp_dot(p - a) >= -1e-4
    })
}

fn area_sign(poly: &[Vec2]) -> f32 {
    let a: f32 = (0..poly.len()).map(|k| poly[k].perp_dot(poly[(k + 1) % poly.len()])).sum();
    if a < 0.0 {
        -1.0
    } else {
        1.0
    }
}

/// `poly` cut to the convex polygon `clip` (Sutherland-Hodgman, one side of it at a time):
/// what of a convex shape lies inside a tile's rounded outline, without a clip layer.
pub fn clip_convex(poly: &[Vec2], clip: &[Vec2]) -> Vec<Vec2> {
    if clip.len() < 3 {
        return Vec::new();
    }
    let s = area_sign(clip);
    let mut out = poly.to_vec();
    for k in 0..clip.len() {
        if out.len() < 3 {
            return Vec::new();
        }
        let (a, b) = (clip[k], clip[(k + 1) % clip.len()]);
        let side = |p: Vec2| s * (b - a).perp_dot(p - a);
        let input = std::mem::take(&mut out);
        for j in 0..input.len() {
            let (p, q) = (input[j], input[(j + 1) % input.len()]);
            let (sp, sq) = (side(p), side(q));
            if sp >= 0.0 {
                out.push(p);
            }
            if (sp >= 0.0) != (sq >= 0.0) {
                out.push(p + (q - p) * (sp / (sp - sq)));
            }
        }
    }
    if out.len() < 3 {
        out.clear();
    }
    out
}

/// The convex `cell` cut to the convex `shape` and painted, each corner in `colour` of where
/// it is: a smooth shading (a light, a sweep) inside a rounded tile.
fn shade(p: &mut Painter, cell: &[Vec2], shape: &[Vec2], colour: &dyn Fn(Vec2) -> Color) {
    let poly = if cell.iter().all(|q| inside_convex(shape, *q)) { cell.to_vec() } else { clip_convex(cell, shape) };
    if poly.len() < 3 {
        return;
    }
    let cs: Vec<Color> = poly.iter().map(|q| colour(*q)).collect();
    for k in 1..poly.len() - 1 {
        p.tri(poly[0], poly[k], poly[k + 1], cs[0], cs[k], cs[k + 1]);
    }
}

/// A soft round light at `c`, `radius` points, `colour` faded by `profile` from its middle
/// out, cut to the convex `shape` - rings of cells, each painted by `shade`.
fn spot(p: &mut Painter, shape: &[Vec2], c: Vec2, radius: f32, colour: Color, profile: &[(f32, f32)]) {
    if radius < 0.5 || colour.0[3] <= 0.0 || shape.len() < 3 {
        return;
    }
    let (lo, hi) = shape.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), q| (lo.min(*q), hi.max(*q)));
    if c.x + radius < lo.x || c.x - radius > hi.x || c.y + radius < lo.y || c.y - radius > hi.y {
        return;
    }
    let tint = |q: Vec2| colour.alpha(profile_at(profile, (q - c).length() / radius));
    let n = ((radius * 0.25) as usize).clamp(16, 48);
    let dir = |k: usize| {
        let a = std::f32::consts::TAU * k as f32 / n as f32;
        Vec2::new(a.cos(), a.sin())
    };
    for ring in profile.windows(2) {
        let (r0, r1) = (ring[0].0 * radius, ring[1].0 * radius);
        if r1 - r0 < 0.05 || ring[0].1 <= 0.0 && ring[1].1 <= 0.0 {
            continue;
        }
        for k in 0..n {
            let (d0, d1) = (dir(k), dir(k + 1));
            let cell: Vec<Vec2> = if r0 < 0.05 { vec![c, c + d0 * r1, c + d1 * r1] } else { vec![c + d0 * r0, c + d0 * r1, c + d1 * r1, c + d1 * r0] };
            let (clo, chi) = cell.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), q| (lo.min(*q), hi.max(*q)));
            if chi.x < lo.x || clo.x > hi.x || chi.y < lo.y || clo.y > hi.y {
                continue;
            }
            shade(p, &cell, shape, &tint);
        }
    }
}

/// The outline of a rounded box as `rounded_outline` makes it, each point with the way out
/// of the box there, its straight sides cut into pieces no longer than `step` (an edge whose
/// colour changes along it).
pub fn outline_dense(r: Rect, radius: f32, step: f32) -> Vec<(Vec2, Vec2)> {
    let rad = radius.min(r.w * 0.5).min(r.h * 0.5).max(0.0);
    let n = ((rad * 0.6) as usize).clamp(3, 12);
    let corners = [(Vec2::new(r.right() - rad, r.y + rad), -90.0f32), (Vec2::new(r.right() - rad, r.bottom() - rad), 0.0), (Vec2::new(r.x + rad, r.bottom() - rad), 90.0), (Vec2::new(r.x + rad, r.y + rad), 180.0)];
    let dir = |deg: f32| Vec2::new(deg.to_radians().cos(), deg.to_radians().sin());
    let mut out = Vec::new();
    for (k, (c, a0)) in corners.iter().enumerate() {
        for j in 0..=n {
            let d = dir(a0 + 90.0 * j as f32 / n as f32);
            out.push((*c + d * rad, d));
        }
        let normal = dir(a0 + 90.0);
        let from = *c + normal * rad;
        let (c2, a2) = corners[(k + 1) % 4];
        let to = c2 + dir(a2) * rad;
        let pieces = ((to - from).length() / step.max(1.0)).ceil() as usize;
        for j in 1..pieces {
            out.push((from + (to - from) * (j as f32 / pieces as f32), normal));
        }
    }
    out
}

/// A button as drawn: risen `rise` points, shrunk while pressed.
pub fn button_rect(r: Rect, rise: f32, press: f32) -> Rect {
    let s = 1.0 - 0.03 * press;
    let c = r.center();
    Rect::new(c.x - r.w * s * 0.5, c.y - r.h * s * 0.5 - rise, r.w * s, r.h * s)
}

/// How long the sheen takes across a primary button (seconds).
const SWEEP_S: f32 = 0.7;

/// The sheen across a primary button `r`, `t` (0 to 1) of the way: its two halves - a slanted
/// band brightest along its middle - as (cell, the middle line's x at the top and the
/// bottom, the band's half width).
pub fn sweep_band(r: Rect, t: f32) -> ([Vec2; 4], [Vec2; 4], f32, f32, f32) {
    let half = (r.h * 0.6).max(12.0);
    let slant = r.h * 0.35;
    let from = r.x - half - slant;
    let to = r.right() + half + slant;
    let x = from + (to - from) * ease_in_out_cubic(t);
    let (top, bottom) = (x + slant, x - slant);
    let left = [Vec2::new(top - half, r.y), Vec2::new(top, r.y), Vec2::new(bottom, r.bottom()), Vec2::new(bottom - half, r.bottom())];
    let right = [Vec2::new(top, r.y), Vec2::new(top + half, r.y), Vec2::new(bottom + half, r.bottom()), Vec2::new(bottom, r.bottom())];
    (left, right, top, bottom, half)
}

/// Which way an icon points: forward (1), back (-1) or nowhere (0) - a button's icon is
/// nudged that way under the mouse.
fn icon_way(icon: &str) -> f32 {
    match icon {
        "play_arrow" | "chevron_right" | "arrow_forward" | "navigate_next" | "east" => 1.0,
        "chevron_left" | "arrow_back" | "navigate_before" | "west" => -1.0,
        _ => 0.0,
    }
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
    // (no further right than the window: the bar's list of languages opens at its edge)
    let w = p.anchor.w.max(200.0);
    Rect::new(p.anchor.x.min(size.x - 10.0 - w).max(10.0), y, w, h)
}

/// A flag as small as the bar's, painted rather than drawn from a font (Windows has no
/// flag emoji): the three tricolours as they are, the Union Jack simplified - its diagonals
/// straight, not counterchanged - which still reads as itself at eighteen pixels.
pub fn flag(p: &mut Painter, r: Rect, code: &str) {
    let bands = |p: &mut Painter, colours: [Color; 3], across: bool| {
        for (k, c) in colours.iter().enumerate() {
            let part = if across { Rect::new(r.x + r.w * k as f32 / 3.0, r.y, r.w / 3.0, r.h) } else { Rect::new(r.x, r.y + r.h * k as f32 / 3.0, r.w, r.h / 3.0) };
            p.rect(part, *c);
        }
    };
    match code {
        "DEU" => bands(p, [Color::rgba(0, 0, 0, 1.0), Color::rgba(221, 0, 0, 1.0), Color::rgba(255, 206, 0, 1.0)], false),
        "FRA" => bands(p, [Color::rgba(0, 35, 149, 1.0), Color::rgba(255, 255, 255, 1.0), Color::rgba(237, 41, 57, 1.0)], true),
        "NLD" => bands(p, [Color::rgba(174, 28, 40, 1.0), Color::rgba(255, 255, 255, 1.0), Color::rgba(33, 70, 139, 1.0)], false),
        _ => {
            let (white, red) = (Color::rgba(255, 255, 255, 1.0), Color::rgba(200, 16, 46, 1.0));
            p.rect(r, Color::rgba(1, 33, 105, 1.0));
            for (w, c) in [(r.h * 0.2, white), (r.h * 0.1, red)] {
                for (a, b) in [(Vec2::new(r.x, r.y), Vec2::new(r.right(), r.bottom())), (Vec2::new(r.right(), r.y), Vec2::new(r.x, r.bottom()))] {
                    let n = (b - a).normalize().perp() * w * 0.5;
                    let band = clip_to(&[a - (b - a) * 0.1 + n, b + (b - a) * 0.1 + n, b + (b - a) * 0.1 - n, a - (b - a) * 0.1 - n], r);
                    p.convex(&band, c);
                }
            }
            for (w, c) in [(r.h / 3.0, white), (r.h * 0.2, red)] {
                p.rect(Rect::new(r.x, r.center().y - w * 0.5, r.w, w), c);
                p.rect(Rect::new(r.center().x - w * 0.5, r.y, w, r.h), c);
            }
        }
    }
    // (an edge of light from inside: the white of the French and the Dutch flag ran into the
    // sheet without it)
    p.rounded_border(r, 2.0, 1.0, Color::WHITE.alpha(0.28));
}

/// A convex polygon cut to `r` (Sutherland-Hodgman, one side of the box at a time).
pub fn clip_to(poly: &[Vec2], r: Rect) -> Vec<Vec2> {
    let mut out = poly.to_vec();
    let sides: [(&dyn Fn(Vec2) -> bool, &dyn Fn(Vec2, Vec2) -> Vec2); 4] = [
        (&|p: Vec2| p.x >= r.x, &|a: Vec2, b: Vec2| a + (b - a) * ((r.x - a.x) / (b.x - a.x))),
        (&|p: Vec2| p.x <= r.right(), &|a: Vec2, b: Vec2| a + (b - a) * ((r.right() - a.x) / (b.x - a.x))),
        (&|p: Vec2| p.y >= r.y, &|a: Vec2, b: Vec2| a + (b - a) * ((r.y - a.y) / (b.y - a.y))),
        (&|p: Vec2| p.y <= r.bottom(), &|a: Vec2, b: Vec2| a + (b - a) * ((r.bottom() - a.y) / (b.y - a.y))),
    ];
    for (inside, cut) in sides {
        let input = std::mem::take(&mut out);
        for k in 0..input.len() {
            let (a, b) = (input[k], input[(k + 1) % input.len()]);
            match (inside(a), inside(b)) {
                (true, true) => out.push(b),
                (true, false) => out.push(cut(a, b)),
                (false, true) => {
                    out.push(cut(a, b));
                    out.push(b);
                }
                (false, false) => {}
            }
        }
    }
    out
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

    // --- motion -------------------------------------------------------------------------

    /// Runs a spring from 1 to 0 at `fps`: its lowest point (the overshoot, negative) and
    /// where it is after `secs`.
    fn swing(feel: Feel, fps: f32, secs: f32) -> (f32, f32) {
        let (mut x, mut v, mut low) = (1.0f32, 0.0f32, 0.0f32);
        for _ in 0..(secs * fps) as usize {
            (x, v) = spring_step(x, v, 1.0 / fps, feel);
            low = low.min(x);
        }
        (low, x)
    }

    /// Each feel lands where it goes, swings past it no more than meant, and does the same at
    /// ten frames a second as at a hundred and forty-four (the closed form: no blowing up).
    #[test]
    fn springs_land_and_swing_as_meant() {
        for fps in [10.0, 60.0, 144.0] {
            for (feel, most) in [(Feel::LIFT, 0.06), (Feel::SLIDE, 0.01), (Feel::KNOB, 0.08), (Feel::PRESS, 0.13), (Feel::POP, 0.15)] {
                let (low, end) = swing(feel, fps, 1.0);
                assert!(-low < most, "{feel:?} at {fps}: past by {}", -low);
                assert!(end.abs() < 2e-3, "{feel:?} at {fps}: still {end} after a second");
            }
            // following the mouse never passes it
            let (low, end) = swing(Feel::FOLLOW, fps, 1.0);
            assert!(low > -1e-4 && end.abs() < 2e-3, "{low} {end}");
        }
        // the lift does pass its end a little: it is a spring, not an ease
        assert!(swing(Feel::LIFT, 60.0, 1.0).0 < -0.01);
        // one step of 1/30 s is two of 1/60 s, at any damping
        for feel in [Feel::LIFT, Feel::FOLLOW, Feel { response: 0.3, damping: 1.4 }] {
            let one = spring_step(0.7, -2.0, 1.0 / 30.0, feel);
            let half = spring_step(0.7, -2.0, 1.0 / 60.0, feel);
            let two = spring_step(half.0, half.1, 1.0 / 60.0, feel);
            assert!((one.0 - two.0).abs() < 1e-4 && (one.1 - two.1).abs() < 1e-2, "{feel:?}: {one:?} against {two:?}");
        }
    }

    #[test]
    fn easings_go_from_nothing_to_all() {
        for f in [ease_out_cubic, ease_in_out_cubic, smoothstep] {
            assert_eq!(f(0.0), 0.0);
            assert!((f(1.0) - 1.0).abs() < 1e-6);
            let mut last = 0.0;
            for k in 1..=100 {
                let v = f(k as f32 / 100.0);
                assert!(v >= last - 1e-6, "rising");
                last = v;
            }
        }
        assert!(ease_out_cubic(0.3) > 0.6, "fast at first");
        assert!((ease_in_out_cubic(0.5) - 0.5).abs() < 1e-6);
    }

    /// A sprung value moves while it is on its way (and says so), lands exactly, and is where
    /// it is going at once without animations or when it comes back after a while unseen.
    #[test]
    fn a_sprung_value_lands_and_skips_when_it_should() {
        let mut ui = Ui::new();
        let frame = |ui: &mut Ui, to: f32| {
            ui.begin(Vec2::new(400.0, 300.0), 1.0, 1.0 / 60.0);
            ui.spring(1, to, Feel::SLIDE)
        };
        assert_eq!(frame(&mut ui, 5.0), 5.0, "first seen: there");
        assert!(!ui.moving);
        let v = frame(&mut ui, 100.0);
        assert!(v > 5.0 && v < 100.0 && ui.moving, "on its way: {v}");
        for _ in 0..90 {
            frame(&mut ui, 100.0);
        }
        assert_eq!(frame(&mut ui, 100.0), 100.0);
        assert!(!ui.moving);
        // a while unseen: back where it goes
        for _ in 0..20 {
            ui.begin(Vec2::new(400.0, 300.0), 1.0, 1.0 / 60.0);
        }
        assert_eq!(frame(&mut ui, 20.0), 20.0);
        // without animations: there at once
        ui.motion = false;
        assert_eq!(frame(&mut ui, 300.0), 300.0);
        assert!(!ui.moving);
    }

    /// The launcher's size is given out as its slider is let go, not while it is dragged.
    #[test]
    fn a_slider_on_release_gives_its_value_when_let_go() {
        fn frame(ui: &mut Ui, value: &mut f32) -> bool {
            ui.begin(Vec2::new(800.0, 600.0), 1.0, 1.0 / 60.0);
            let changed = ui.slider_on_release("scale", Rect::new(0.0, 0.0, 600.0, 32.0), value, 0.6, 2.0, 0.05, "Size", &|v| format!("{v}"));
            ui.finish();
            changed
        }
        let mut ui = Ui::new();
        let mut value = 1.0f32;
        // (the track runs from 170 to 542: the label takes 170, the value 58)
        ui.input.mouse = Vec2::new(200.0, 16.0);
        (ui.input.pressed, ui.input.down) = (true, true);
        assert!(!frame(&mut ui, &mut value));
        ui.input.pressed = false;
        ui.input.mouse = Vec2::new(500.0, 16.0);
        assert!(!frame(&mut ui, &mut value));
        assert_eq!(value, 1.0, "nothing given out while it is dragged");
        (ui.input.down, ui.input.released) = (false, true);
        assert!(frame(&mut ui, &mut value), "let go: the value it was dragged to");
        assert!((value - 1.85).abs() < 1e-4, "{value}");
        ui.input.released = false;
        assert!(!frame(&mut ui, &mut value), "and only once");
    }

    /// A tile under the mouse is only highlighted: it stays where it lies (no rising, tilt or
    /// ripple) while its hover - what lights its edge - goes to full and back.
    #[test]
    fn a_tile_is_only_highlighted() {
        let base = Rect::new(100.0, 100.0, 300.0, 200.0);
        let mut ui = Ui::new();
        let frame = |ui: &mut Ui| {
            ui.begin(Vec2::new(800.0, 600.0), 1.0, 1.0 / 60.0);
            let t = ui.tile(7, base, 14.0);
            ui.tile_shadow(&t);
            ui.tile_light(&t, 0.1);
            ui.tile_edge(&t, 1.0, EDGE);
            ui.finish();
            t
        };
        ui.input.mouse = Vec2::new(700.0, 550.0);
        assert_eq!(frame(&mut ui).r, base, "away from the mouse: at rest");
        ui.input.mouse = Vec2::new(380.0, 120.0);
        let mut t = frame(&mut ui);
        for _ in 0..40 {
            t = frame(&mut ui);
        }
        assert_eq!(ui.drawn[&7], base, "the hit area does not move");
        assert_eq!(t.r, base, "drawn where it lies");
        assert!(t.hover > 0.9, "highlighted: {}", t.hover);
        assert!(!t.motion && t.ripple.is_none());
        ui.input.mouse = Vec2::new(700.0, 550.0);
        for _ in 0..60 {
            t = frame(&mut ui);
        }
        assert!(t.hover < 0.05, "the highlight fades: {}", t.hover);
    }

    #[test]
    fn a_risen_tile_grows_about_its_middle() {
        let base = Rect::new(0.0, 0.0, 400.0, 200.0);
        assert_eq!(tile_rect(base, 0.0, 0.0), base);
        let up = tile_rect(base, 1.0, 0.0);
        assert!((up.w - 400.0 * (1.0 + TILE_GROW)).abs() < 1e-3);
        assert!((up.center().x - 200.0).abs() < 1e-3);
        assert!((up.center().y - (100.0 - TILE_RISE)).abs() < 1e-3);
        let down = tile_rect(base, 0.0, 1.0);
        assert!((down.w - 400.0 * (1.0 - TILE_SINK)).abs() < 1e-3 && (down.center() - base.center()).length() < 1e-3);
        assert_eq!(aim_on(base, Vec2::new(400.0, 0.0)), Vec2::new(1.0, -1.0));
        assert_eq!(aim_on(base, Vec2::new(900.0, 100.0)), Vec2::new(1.0, 0.0), "held to the edge");
    }

    /// The photo's part: as `image_cover` cut it at rest; closer and moved, it stays inside the
    /// picture, moving its content the way asked.
    #[test]
    fn a_tile_photo_shifts_inside_its_picture() {
        let rest = cover_uv(400.0, 200.0, 800, 600, 1.0, Vec2::ZERO);
        // (400 x 200 over 800 x 600: the whole width, the middle two thirds of the height)
        assert!((rest[0] - 0.0).abs() < 1e-5 && (rest[2] - 1.0).abs() < 1e-5);
        assert!((rest[1] - 1.0 / 6.0).abs() < 1e-4 && (rest[3] - 5.0 / 6.0).abs() < 1e-4, "{rest:?}");
        let near = cover_uv(400.0, 200.0, 800, 600, 1.05, Vec2::ZERO);
        assert!(((near[2] - near[0]) - 1.0 / 1.05).abs() < 1e-4, "{near:?}");
        // content moved right: the window moves left
        let moved = cover_uv(400.0, 200.0, 800, 600, 1.05, Vec2::new(6.0, 0.0));
        assert!(moved[0] < near[0]);
        for shift in [Vec2::new(500.0, 0.0), Vec2::new(-500.0, 300.0), Vec2::new(0.0, -900.0)] {
            let uv = cover_uv(400.0, 200.0, 800, 600, 1.05, shift);
            assert!(uv[0] >= 0.0 && uv[1] >= 0.0 && uv[2] <= 1.0 + 1e-6 && uv[3] <= 1.0 + 1e-6, "{shift}: {uv:?}");
        }
    }

    #[test]
    fn a_convex_shape_is_cut_to_a_convex_shape() {
        let square = [Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0), Vec2::new(0.0, 10.0)];
        let area = |p: &[Vec2]| (0..p.len()).map(|k| p[k].perp_dot(p[(k + 1) % p.len()])).sum::<f32>().abs() * 0.5;
        let half = [Vec2::new(5.0, -5.0), Vec2::new(20.0, -5.0), Vec2::new(20.0, 20.0), Vec2::new(5.0, 20.0)];
        // either way round
        for clip in [half.to_vec(), half.iter().rev().copied().collect()] {
            let cut = clip_convex(&square, &clip);
            assert!((area(&cut) - 50.0).abs() < 1e-3, "{cut:?}");
        }
        assert_eq!(clip_convex(&square, &[Vec2::new(-5.0, -5.0), Vec2::new(15.0, -5.0), Vec2::new(15.0, 15.0), Vec2::new(-5.0, 15.0)]).len(), 4, "inside: as it was");
        assert!(clip_convex(&square, &[Vec2::new(20.0, 20.0), Vec2::new(30.0, 20.0), Vec2::new(30.0, 30.0)]).is_empty());
        assert!(inside_convex(&square, Vec2::new(5.0, 5.0)) && !inside_convex(&square, Vec2::new(11.0, 5.0)));
    }

    /// The sheen and the ripple paint only inside the tile's rounded shape - its corners
    /// included - and nothing at all when they lie far from it.
    #[test]
    fn a_light_stays_in_its_tile() {
        let r = Rect::new(50.0, 50.0, 300.0, 180.0);
        let shape = rounded_outline(r, 14.0);
        let mut p = Painter::new();
        for (c, radius) in [(Vec2::new(55.0, 55.0), 250.0), (Vec2::new(340.0, 220.0), 120.0), (r.center(), 600.0)] {
            spot(&mut p, &shape, c, radius, Color::WHITE.alpha(0.2), &SHEEN);
        }
        assert!(!p.verts.is_empty());
        for v in &p.verts {
            let q = Vec2::new(v.pos[0], v.pos[1]);
            // (in the corner's round, not merely in the box)
            let corner = Vec2::new(q.x.clamp(r.x + 14.0, r.right() - 14.0), q.y.clamp(r.y + 14.0, r.bottom() - 14.0));
            assert!((q - corner).length() <= 14.0 + 0.05, "{q} lies outside the tile");
        }
        let n = p.verts.len();
        spot(&mut p, &shape, Vec2::new(900.0, 900.0), 100.0, Color::WHITE, &SHEEN);
        assert_eq!(p.verts.len(), n);
        assert_eq!(profile_at(&SHEEN, 0.0), 1.0);
        assert_eq!(profile_at(&SHEEN, 1.0), 0.0);
        assert_eq!(profile_at(&SHEEN, 2.0), 0.0);
        assert!((profile_at(&SHEEN, 0.11) - 0.86).abs() < 1e-4);
    }

    #[test]
    fn a_dense_outline_lies_on_the_rounded_box() {
        let r = Rect::new(10.0, 20.0, 400.0, 120.0);
        let pts = outline_dense(r, 14.0, 10.0);
        assert!(pts.len() > 2 * (400 + 120) / 10 - 8);
        for (k, (q, n)) in pts.iter().enumerate() {
            let corner = Vec2::new(q.x.clamp(r.x + 14.0, r.right() - 14.0), q.y.clamp(r.y + 14.0, r.bottom() - 14.0));
            let d = (*q - corner).length();
            assert!((d - 14.0).abs() < 0.05 || (q.x - r.x).abs() < 0.05 || (q.x - r.right()).abs() < 0.05 || (q.y - r.y).abs() < 0.05 || (q.y - r.bottom()).abs() < 0.05, "{q} is off the edge");
            assert!((n.length() - 1.0).abs() < 1e-4);
            // (the way out: away from the middle)
            assert!(n.dot(*q - r.center()) > 0.0);
            let next = pts[(k + 1) % pts.len()].0;
            assert!((next - *q).length() <= 10.0 + 0.05, "a gap of {} after {q}", (next - *q).length());
        }
    }

    #[test]
    fn a_ripple_runs_out_to_the_far_corner_and_fades() {
        let r = Rect::new(0.0, 0.0, 300.0, 400.0);
        let at = Vec2::new(0.0, 0.0);
        assert_eq!(ripple_at(r, at, 0.0), (0.0, 0.0));
        let (end, gone) = ripple_at(r, at, 1.0);
        assert!((end - 500.0).abs() < 1e-3 && gone == 0.0);
        let mut last = 0.0;
        for k in 1..=20 {
            let (radius, _) = ripple_at(r, at, k as f32 / 20.0);
            assert!(radius > last);
            last = radius;
        }
        assert!(ripple_at(r, at, 0.1).1 > 0.7, "strong at once");
    }

    #[test]
    fn the_sheen_crosses_the_button_from_side_to_side() {
        let r = Rect::new(100.0, 100.0, 240.0, 62.0);
        let (l0, r0, ..) = sweep_band(r, 0.0);
        assert!(l0.iter().chain(r0.iter()).all(|q| q.x <= r.x + 0.01), "it begins left of the button");
        let (l1, r1, ..) = sweep_band(r, 1.0);
        assert!(l1.iter().chain(r1.iter()).all(|q| q.x >= r.right() - 0.01), "it ends right of it");
        assert_eq!(icon_way("play_arrow"), 1.0);
        assert_eq!(icon_way("chevron_left"), -1.0);
        assert_eq!(icon_way("add"), 0.0);
    }

    /// A button under the mouse stays where it lies (it only lights up: no rising), and is
    /// hit there.
    #[test]
    fn a_button_stays_where_it_lies() {
        let r = Rect::new(100.0, 100.0, 200.0, 62.0);
        let mut ui = Ui::new();
        ui.input.mouse = Vec2::new(110.0, 158.0);
        let mut top = 0.0;
        for _ in 0..30 {
            ui.begin(Vec2::new(400.0, 300.0), 1.0, 1.0 / 60.0);
            ui.button("go", r, "Go", Some("play_arrow"), ButtonKind::Primary);
            top = ui.p().verts.iter().filter(|v| v.mode == [0.0, 0.0]).map(|v| v.pos[1]).fold(f32::MAX, f32::min);
            ui.finish();
        }
        assert_eq!(ui.drawn[&id_of("go")], r);
        assert!(top >= r.y - 0.5, "not risen: its top at {top}");
    }

    /// The chosen chip's pill slides to the one clicked - not at once, and lands on it.
    #[test]
    fn the_chosen_chip_slides_over() {
        let labels = ["One", "Two", "Three", "Four"];
        let r = Rect::new(10.0, 10.0, 600.0, 34.0);
        let mut ui = Ui::new();
        let mut sel = 0;
        let pill = |ui: &Ui| ui.spring_at(id_of("c") ^ 0xc41b ^ 1).unwrap();
        let frame = |ui: &mut Ui, sel: &mut usize| {
            ui.begin(Vec2::new(800.0, 600.0), 1.0, 1.0 / 60.0);
            ui.chips("c", r, sel, &labels);
            ui.finish();
        };
        frame(&mut ui, &mut sel);
        assert_eq!(pill(&ui), r.x, "first drawn: in place");
        let last = ui.drawn[&(id_of("c") ^ (3 + 11))];
        ui.input.mouse = last.center();
        ui.input.pressed = true;
        frame(&mut ui, &mut sel);
        (ui.input.pressed, ui.input.released) = (false, true);
        frame(&mut ui, &mut sel);
        ui.input.released = false;
        assert_eq!(sel, 3);
        let x = pill(&ui);
        assert!(x > r.x && x < last.x, "on its way: {x}");
        for _ in 0..60 {
            frame(&mut ui, &mut sel);
        }
        assert_eq!(pill(&ui), last.x);
    }

    /// The radio's dot pops: a little past its size and back.
    #[test]
    fn the_chosen_dot_pops_in() {
        let mut ui = Ui::new();
        let r = Rect::new(10.0, 10.0, 300.0, 36.0);
        let mut most: f32 = 0.0;
        for k in 0..40 {
            ui.begin(Vec2::new(400.0, 300.0), 1.0, 1.0 / 60.0);
            ui.row("r", r, k > 0);
            ui.radio(Vec2::new(26.0, 28.0), k > 0);
            most = most.max(ui.spring_at(id_of("r") ^ 0x7ad1_0001).unwrap());
            ui.finish();
        }
        assert!(most > 1.03, "it pops: {most}");
        assert_eq!(ui.spring_at(id_of("r") ^ 0x7ad1_0001), Some(1.0));
    }

    /// A grid of tiles, all lit, one rippling, takes no layers of its own (the light is cut to
    /// the tiles by arithmetic): the GPU's 256 are left for the rest.
    #[test]
    fn lit_tiles_take_no_layers() {
        let mut ui = Ui::new();
        let mut count = 0;
        for frame in 0..30 {
            ui.begin(Vec2::new(1400.0, 900.0), 1.0, 1.0 / 60.0);
            ui.input.mouse = Vec2::new(200.0, 100.0);
            (ui.input.pressed, ui.input.down) = (frame == 10, frame == 10);
            ui.input.released = frame == 11;
            for k in 0..60 {
                let base = Rect::new(20.0 + (k % 10) as f32 * 130.0, 20.0 + (k / 10) as f32 * 140.0, 120.0, 130.0);
                let t = ui.tile(1000 + k, base, 14.0);
                ui.tile_shadow(&t);
                ui.p().rounded(t.r, 14.0, PANEL);
                ui.tile_light(&t, 0.1);
                ui.tile_edge(&t, 1.0, EDGE);
            }
            count = ui.finish().0.len();
        }
        assert_eq!(count, 1, "{count} layers");
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

    /// A click on a chip chooses it; the chips go on in a second row where the first is full,
    /// and `chips_height` says so.
    #[test]
    fn chips_choose_and_wrap() {
        let labels = ["Graphics", "Driving", "Camera", "Sound", "Gameplay", "General"];
        let mut ui = Ui::new();
        let mut sel = 0;
        let r = Rect::new(10.0, 10.0, 260.0, 34.0);
        let frame = |ui: &mut Ui, sel: &mut usize| {
            ui.begin(Vec2::new(800.0, 600.0), 1.0, 1.0 / 60.0);
            let changed = ui.chips("tabs", r, sel, &labels);
            ui.finish();
            changed
        };
        frame(&mut ui, &mut sel);
        let last = ui.drawn[&(id_of("tabs") ^ (5 + 11))];
        assert!(last.y > r.y, "the last chip is in a later row");
        let h = ui.chips_height(r.w, r.h, &labels);
        assert!((last.bottom() - r.y - h).abs() < 0.5, "chips_height {h} against the last row's foot {}", last.bottom() - r.y);
        ui.input.mouse = last.center();
        ui.input.pressed = true;
        assert!(!frame(&mut ui, &mut sel));
        ui.input.released = true;
        assert!(frame(&mut ui, &mut sel));
        assert_eq!(sel, 5);
        assert_eq!(ui.chips_height(2000.0, 34.0, &labels), 34.0);
    }

    /// The Union Jack's diagonals are cut to the flag: nothing of them lies outside it.
    #[test]
    fn a_polygon_is_cut_to_the_box() {
        let r = Rect::new(0.0, 0.0, 18.0, 12.0);
        let band = [Vec2::new(-5.0, -4.0), Vec2::new(23.0, 15.0), Vec2::new(21.0, 17.0), Vec2::new(-7.0, -2.0)];
        let cut = clip_to(&band, r);
        assert!(cut.len() >= 3);
        assert!(cut.iter().all(|p| p.x >= -1e-3 && p.x <= 18.001 && p.y >= -1e-3 && p.y <= 12.001), "{cut:?}");
        assert!(clip_to(&[Vec2::new(30.0, 30.0), Vec2::new(40.0, 30.0), Vec2::new(35.0, 40.0)], r).is_empty());
        // every flag paints something, the unknown ones as the Union Jack
        let mut p = Painter::new();
        for code in ["ENG", "DEU", "FRA", "NLD"] {
            let before = p.verts.len();
            flag(&mut p, r, code);
            assert!(p.verts.len() > before);
        }
    }

    /// The bar's list of languages opens at the window's right edge: it is not cut off there.
    #[test]
    fn a_list_opened_at_the_edge_stays_in_the_window() {
        let options: Vec<String> = (0..30).map(|k| format!("Language {k}")).collect();
        let mut ui = Ui::new();
        let mut sel = 0;
        let at = Rect::new(770.0, 10.0, 26.0, 26.0);
        ui.input.mouse = at.center();
        for press in [true, false] {
            ui.begin(Vec2::new(800.0, 600.0), 1.0, 1.0 / 60.0);
            (ui.input.pressed, ui.input.released) = (press, !press);
            ui.menu("langs", at, "language", "", false, &mut sel, &options);
            ui.finish();
        }
        let r = popup_rect(ui.popup.as_ref().expect("the list is open"), ui.size);
        assert!(r.right() <= 800.0 && r.x >= 0.0, "{r:?}");
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
