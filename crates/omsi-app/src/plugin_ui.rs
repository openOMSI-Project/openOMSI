//! The Lua plugins' panels and notifications on the screen (`omsi.ui`; the tables, their
//! owners and the clicks are `omsi_plugin::ui`'s).
//!
//! * A panel is laid out in the plugin's logical pixels (a vertical stack; rows side by
//!   side, a `grow` child taking what the others leave; texts wrapped to their width) and
//!   drawn with `omsi-ui` into a texture of its own, shown as a premultiplied overlay over
//!   the picture and under the game's own interface. Only a panel that changed - its
//!   table, the interface's size, the button under the mouse - is laid out and drawn again;
//!   every other frame just puts the overlays where they belong.
//! * Rounded shapes are cut by the pipeline's rounded clip, one layer each, rather than
//!   smoothed by multisampling: panels come in every size, and a multisampled texture
//!   would be made anew for each one drawn.
//! * Notifications are cards of the same kind, top right (under the navigator when it is
//!   there), newest first; they slide in and out by moving, not by being drawn again.
//! * While the panels have the mouse (`omsi.ui.focus`), a click is looked up in the
//!   clickable parts of the panel under it and handed to its plugin (`ui_click`).

use glam::Vec2;
use hashbrown::HashMap;
use omsi_plugin::ui::{self as pui, Align, Element, Kind, Panel, Rgba, RowAlign, Toast, UiState};
use omsi_render::{Renderer, Scene, TextureId};
use omsi_ui::paint::Align as TextAlign;
use omsi_ui::{Atlas, Color, Draw, Fonts, Gpu, Layer, Painter, Rect, Vertex, Weight};

// --- the game's look: the notifications' card, the menus' amber accent ------------------

const CARD: Color = Color::rgba(14, 16, 20, 225.0 / 255.0);
const INK: Color = Color::rgba(236, 236, 236, 1.0);
const DARK_INK: Color = Color::rgba(18, 14, 8, 1.0);
const ACCENT: Color = Color::rgba(232, 160, 48, 1.0);
// (solid greys, the menus' own: the pipeline blends in linear light, where a tenth of white
// over the dark card came out a light grey)
const TRACK: Color = Color::rgba(62, 62, 62, 1.0);
const DIVIDER: Color = Color::rgba(52, 52, 52, 1.0);
const BUTTON: Color = Color::rgba(44, 44, 44, 1.0);
const SHADOW: Color = Color::rgba(0, 0, 0, 0.35);

/// A text line's height, times its size.
pub(crate) const LINE: f32 = 1.3;
pub(crate) const BADGE_H: f32 = 20.0;
const BADGE_PX: f32 = 12.0;
const BADGE_PAD: f32 = 8.0;
pub(crate) const BUTTON_H: f32 = 34.0;
const BUTTON_PX: f32 = 14.0;
const BUTTON_PAD: f32 = 14.0;
const BUTTON_ICON: f32 = 18.0;
const BUTTON_R: f32 = 8.0;
/// A bar in a row that does not grow.
const BAR_W: f32 = 60.0;
const STRIPE: f32 = 4.0;
/// Room round a card in its texture for the shadow (logical pixels).
const MARGIN: f32 = 14.0;
const SHADOW_BLUR: f32 = 12.0;
/// The pipeline's layers: one goes to the card, one to each rounded shape; past this the
/// shapes are drawn as triangles.
const MAX_LAYERS: usize = 250;

const TOAST_W: f32 = 320.0;
/// Where the notifications start, below the corner's window line (`ui`'s `corner_top`).
const TOAST_TOP: f32 = 60.0;
const TOAST_GAP: f32 = 8.0;
const TOAST_IN: f32 = 0.25;
const TOAST_OUT: f32 = 0.3;

fn color(c: Rgba) -> Color {
    Color::rgba(c.0[0], c.0[1], c.0[2], c.0[3] as f32 / 255.0)
}

fn weight(w: pui::Weight) -> Weight {
    match w {
        pui::Weight::Regular => Weight::Regular,
        pui::Weight::Medium => Weight::Medium,
        pui::Weight::Bold => Weight::Bold,
    }
}

/// A shape under the mouse: lighter, and a see-through one less see-through.
fn lit(c: Color) -> Color {
    let l = c.lighten(0.15);
    Color([l.0[0], l.0[1], l.0[2], (c.0[3] * 1.8).min(1.0)])
}

/// Dark ink on a light fill, light ink on a dark one.
fn ink_on(fill: Color) -> Color {
    let [r, g, b, _] = fill.0;
    if 0.299 * r + 0.587 * g + 0.114 * b > 0.6 {
        DARK_INK
    } else {
        INK
    }
}

// --- layout ------------------------------------------------------------------------------

/// What a card is drawn from, in its own logical pixels (its top left at 0, 0).
#[derive(Debug, Clone, Default)]
pub(crate) struct Laid {
    pub w: f32,
    pub h: f32,
    pub radius: f32,
    pub background: Color,
    pub accent: Option<Color>,
    pub items: Vec<Item>,
    /// The clickable parts, in the order they are drawn (the last one under the mouse wins).
    pub hits: Vec<Hit>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Item {
    /// A box with round corners; `hot`: the hit that lights it under the mouse.
    Rounded {
        r: Rect,
        radius: f32,
        color: Color,
        hot: Option<usize>,
    },
    Rect {
        r: Rect,
        color: Color,
    },
    /// A line of text, `at` its baseline's left end.
    Text {
        text: String,
        px: f32,
        weight: Weight,
        at: Vec2,
        color: Color,
    },
    Icon {
        name: String,
        center: Vec2,
        size: f32,
        color: Color,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Hit {
    pub r: Rect,
    /// None: the panel itself.
    pub element: Option<String>,
}

struct Layout<'a> {
    fonts: &'a Fonts,
    out: Laid,
}

impl Layout<'_> {
    fn width(&self, text: &str, px: f32, w: Weight) -> f32 {
        self.fonts.width_as_is(text, px, w)
    }

    /// The longest start of `text` that fits `max`, with an ellipsis when cut.
    fn fit(&self, text: &str, px: f32, w: Weight, max: f32) -> String {
        if self.width(text, px, w) <= max {
            return text.to_string();
        }
        let chars: Vec<char> = text.chars().collect();
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let s: String = chars[..mid]
                .iter()
                .collect::<String>()
                .trim_end()
                .to_string()
                + "…";
            if self.width(&s, px, w) <= max {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        chars[..lo]
            .iter()
            .collect::<String>()
            .trim_end()
            .to_string()
            + "…"
    }

    /// `text` in lines of at most `max` (a word longer than that is broken where it must).
    fn wrap(&self, text: &str, px: f32, w: Weight, max: f32) -> Vec<String> {
        let mut out = Vec::new();
        for para in text.split('\n') {
            let mut line = String::new();
            for word in para.split_whitespace() {
                let joined = if line.is_empty() {
                    word.to_string()
                } else {
                    format!("{line} {word}")
                };
                if self.width(&joined, px, w) <= max {
                    line = joined;
                    continue;
                }
                if !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                }
                for c in word.chars() {
                    let longer = format!("{line}{c}");
                    if !line.is_empty() && self.width(&longer, px, w) > max {
                        out.push(std::mem::replace(&mut line, c.to_string()));
                    } else {
                        line = longer;
                    }
                }
            }
            out.push(line);
        }
        out
    }

    fn lines(&self, e: &Element, w: f32) -> Vec<String> {
        let Kind::Text {
            text,
            size,
            weight: wt,
            wrap,
            ..
        } = &e.kind
        else {
            return Vec::new();
        };
        let wt = weight(*wt);
        if *wrap {
            self.wrap(text, *size, wt, w.max(1.0))
        } else {
            text.split('\n')
                .map(|l| self.fit(l, *size, wt, w.max(1.0)))
                .collect()
        }
    }

    fn button_content(&self, text: &str, icon: &Option<String>) -> f32 {
        let t = self.width(text, BUTTON_PX, Weight::Medium);
        match icon {
            Some(_) if text.is_empty() => BUTTON_ICON,
            Some(_) => BUTTON_ICON + 6.0 + t,
            None => t,
        }
    }

    /// The width an element takes in a row when nothing makes it smaller.
    fn natural(&self, e: &Element) -> f32 {
        match &e.kind {
            Kind::Text {
                text,
                size,
                weight: wt,
                ..
            } => text
                .split('\n')
                .map(|l| self.width(l, *size, weight(*wt)))
                .fold(0.0, f32::max)
                .ceil(),
            Kind::Icon { size, .. } => *size,
            Kind::Row { children, gap, .. } => {
                children.iter().map(|c| self.natural(c)).sum::<f32>()
                    + gap * children.len().saturating_sub(1) as f32
            }
            Kind::Bar { .. } => BAR_W,
            Kind::Badge { text, .. } => {
                (self.width(text, BADGE_PX, Weight::Medium) + 2.0 * BADGE_PAD).ceil()
            }
            Kind::Divider => 1.0,
            Kind::Space { size } => *size,
            Kind::Button { text, icon } => {
                (self.button_content(text, icon) + 2.0 * BUTTON_PAD).ceil()
            }
        }
    }

    /// The widths of a row's children in `w`: their own, the `grow` ones sharing what is
    /// left; when they do not fit, the texts, labels and rows give up width alike.
    fn row_widths(&self, children: &[Element], gap: f32, w: f32) -> Vec<f32> {
        let gaps = gap * children.len().saturating_sub(1) as f32;
        let mut ws: Vec<f32> = children
            .iter()
            .map(|c| if c.grow { 0.0 } else { self.natural(c) })
            .collect();
        let room = w - gaps - ws.iter().sum::<f32>();
        let growing = children.iter().filter(|c| c.grow).count();
        if room < 0.0 {
            let shrinks = |c: &Element| {
                !matches!(
                    c.kind,
                    Kind::Icon { .. } | Kind::Space { .. } | Kind::Divider
                )
            };
            let soft: f32 = children
                .iter()
                .zip(&ws)
                .filter(|(c, _)| shrinks(c))
                .map(|(_, w)| w)
                .sum();
            if soft > 0.0 {
                let k = ((soft + room) / soft).max(0.0);
                for (c, w) in children.iter().zip(ws.iter_mut()) {
                    if shrinks(c) {
                        *w *= k;
                    }
                }
            }
        } else if growing > 0 {
            for (c, w) in children.iter().zip(ws.iter_mut()) {
                if c.grow {
                    *w = room / growing as f32;
                }
            }
        }
        ws
    }

    /// An element's height at width `w` (a space or a divider in a row has none of its own).
    fn height(&self, e: &Element, w: f32, in_row: bool) -> f32 {
        match &e.kind {
            Kind::Text { size, .. } => self.lines(e, w).len() as f32 * size * LINE,
            Kind::Icon { size, .. } => *size,
            Kind::Row { children, gap, .. } => {
                let ws = self.row_widths(children, *gap, w);
                children
                    .iter()
                    .zip(&ws)
                    .map(|(c, cw)| self.height(c, *cw, true))
                    .fold(0.0, f32::max)
            }
            Kind::Bar { height, .. } => *height,
            Kind::Badge { .. } => BADGE_H,
            Kind::Button { .. } => BUTTON_H,
            Kind::Divider => {
                if in_row {
                    0.0
                } else {
                    1.0
                }
            }
            Kind::Space { size } => {
                if in_row {
                    0.0
                } else {
                    *size
                }
            }
        }
    }

    fn rounded(&mut self, r: Rect, radius: f32, color: Color, hot: Option<usize>) {
        self.out.items.push(Item::Rounded {
            r,
            radius,
            color,
            hot,
        });
    }

    fn text(&mut self, text: String, px: f32, weight: Weight, at: Vec2, color: Color) {
        if !text.is_empty() {
            self.out.items.push(Item::Text {
                text,
                px,
                weight,
                at,
                color,
            });
        }
    }

    /// The baseline of a line `px` high whose box starts at `top`, `h` high: capitals centred.
    fn baseline(&self, top: f32, h: f32, px: f32, w: Weight) -> f32 {
        top + (h + self.fonts.cap_height(px, w)) * 0.5
    }

    /// Put `e` into `r` (its width; in a row the row's height, in the stack its own).
    fn emit(&mut self, e: &Element, r: Rect, in_row: bool) {
        // (before what is in it: a clickable row's buttons are found first, as drawn later)
        if e.takes_clicks() && !matches!(e.kind, Kind::Button { .. }) {
            self.out.hits.push(Hit {
                r,
                element: e.id.clone(),
            });
        }
        match &e.kind {
            Kind::Text {
                size,
                weight: wt,
                align,
                ..
            } => {
                let wt = weight(*wt);
                let lines = self.lines(e, r.w);
                let lh = size * LINE;
                let top = r.y + (r.h - lh * lines.len() as f32) * 0.5;
                let c = e.color.map(color).unwrap_or(INK);
                for (k, line) in lines.into_iter().enumerate() {
                    let lw = self.width(&line, *size, wt);
                    let x = match align {
                        Align::Left => r.x,
                        Align::Center => r.x + (r.w - lw) * 0.5,
                        Align::Right => r.right() - lw,
                    };
                    let y = self.baseline(top + k as f32 * lh, lh, *size, wt);
                    self.text(line, *size, wt, Vec2::new(x, y), c);
                }
            }
            Kind::Icon { name, size } => {
                self.out.items.push(Item::Icon {
                    name: name.clone(),
                    center: Vec2::new(r.x + size * 0.5, r.y + r.h * 0.5),
                    size: *size,
                    color: e.color.map(color).unwrap_or(INK),
                });
            }
            Kind::Row {
                children,
                gap,
                align,
            } => {
                let ws = self.row_widths(children, *gap, r.w);
                let used = ws.iter().sum::<f32>() + gap * children.len().saturating_sub(1) as f32;
                let free = (r.w - used).max(0.0);
                let (mut x, step) = match align {
                    RowAlign::Start => (r.x, *gap),
                    RowAlign::Center => (r.x + free * 0.5, *gap),
                    RowAlign::End => (r.x + free, *gap),
                    RowAlign::Between if children.len() > 1 => {
                        (r.x, gap + free / (children.len() - 1) as f32)
                    }
                    RowAlign::Between => (r.x, *gap),
                };
                for (c, cw) in children.iter().zip(ws) {
                    let ch = self.height(c, cw, true);
                    let cell = if ch == 0.0 {
                        Rect::new(x, r.y, cw, r.h)
                    } else {
                        Rect::new(x, r.y + (r.h - ch) * 0.5, cw, ch)
                    };
                    self.emit(c, cell, true);
                    x += cw + step;
                }
            }
            Kind::Bar {
                value,
                height,
                background,
            } => {
                let track = Rect::new(r.x, r.y + (r.h - height) * 0.5, r.w, *height);
                self.rounded(
                    track,
                    height * 0.5,
                    background.map(color).unwrap_or(TRACK),
                    None,
                );
                if *value > 0.0 {
                    let fill = (r.w * value).max(*height).min(r.w);
                    self.rounded(
                        Rect::new(track.x, track.y, fill, *height),
                        height * 0.5,
                        e.color.map(color).unwrap_or(ACCENT),
                        None,
                    );
                }
            }
            Kind::Badge { text, text_color } => {
                let w = self.natural(e).min(r.w);
                let b = Rect::new(r.x, r.y + (r.h - BADGE_H) * 0.5, w, BADGE_H);
                let fill = e.color.map(color).unwrap_or(ACCENT);
                self.rounded(b, BADGE_H * 0.5, fill, None);
                let t = self.fit(text, BADGE_PX, Weight::Medium, w - 2.0 * BADGE_PAD);
                let tw = self.width(&t, BADGE_PX, Weight::Medium);
                let y = self.baseline(b.y, b.h, BADGE_PX, Weight::Medium);
                self.text(
                    t,
                    BADGE_PX,
                    Weight::Medium,
                    Vec2::new(b.x + (b.w - tw) * 0.5, y),
                    text_color.map(color).unwrap_or(ink_on(fill)),
                );
            }
            Kind::Divider => {
                let c = e.color.map(color).unwrap_or(DIVIDER);
                let line = if in_row {
                    Rect::new(r.x + r.w * 0.5 - 0.5, r.y, 1.0, r.h)
                } else {
                    Rect::new(r.x, r.y + (r.h - 1.0) * 0.5, r.w, 1.0)
                };
                self.out.items.push(Item::Rect { r: line, color: c });
            }
            Kind::Space { .. } => {}
            Kind::Button { text, icon } => {
                let b = Rect::new(r.x, r.y + (r.h - BUTTON_H) * 0.5, r.w, BUTTON_H);
                let hit = self.out.hits.len();
                self.out.hits.push(Hit {
                    r: b,
                    element: e.id.clone(),
                });
                let fill = e.color.map(color).unwrap_or(BUTTON);
                self.rounded(b, BUTTON_R, fill, Some(hit));
                // (on the see-through default the card's own light ink; on a colour, what reads on it)
                let ink = if e.color.is_some() { ink_on(fill) } else { INK };
                let room = (b.w - 2.0 * BUTTON_PAD).max(0.0);
                let icon_w = if icon.is_some() {
                    BUTTON_ICON + if text.is_empty() { 0.0 } else { 6.0 }
                } else {
                    0.0
                };
                let t = self.fit(text, BUTTON_PX, Weight::Medium, (room - icon_w).max(0.0));
                let content = icon_w + self.width(&t, BUTTON_PX, Weight::Medium);
                let x = b.x + (b.w - content) * 0.5;
                if let Some(name) = icon {
                    self.out.items.push(Item::Icon {
                        name: name.clone(),
                        center: Vec2::new(x + BUTTON_ICON * 0.5, b.y + b.h * 0.5),
                        size: BUTTON_ICON,
                        color: ink,
                    });
                }
                let y = self.baseline(b.y, b.h, BUTTON_PX, Weight::Medium);
                self.text(t, BUTTON_PX, Weight::Medium, Vec2::new(x + icon_w, y), ink);
            }
        }
    }
}

/// Lay a panel out: its size, what it is drawn from and where it takes clicks.
/// `backdrop`: the opacity setting's strength for the game's own card colour (`ui::backdrop`).
pub(crate) fn layout(p: &Panel, fonts: &Fonts, backdrop: f32) -> Laid {
    let mut l = Layout {
        fonts,
        out: Laid::default(),
    };
    let stripe = if p.accent.is_some() { STRIPE } else { 0.0 };
    let x0 = stripe + p.padding;
    let w = (p.width - x0 - p.padding).max(1.0);
    if p.clickable {
        l.out.hits.push(Hit {
            r: Rect::default(),
            element: None,
        });
    }
    let mut y = p.padding;
    for (i, c) in p.children.iter().enumerate() {
        if i > 0 {
            y += p.gap;
        }
        let h = l.height(c, w, false);
        l.emit(c, Rect::new(x0, y, w, h), false);
        y += h;
    }
    let mut out = l.out;
    out.w = p.width;
    out.h = (y + p.padding).max(2.0 * p.padding).max(1.0).ceil();
    if p.clickable {
        out.hits[0].r = Rect::new(0.0, 0.0, out.w, out.h);
    }
    out.radius = p.radius.min(out.w * 0.5).min(out.h * 0.5);
    out.background = p
        .background
        .map(color)
        .unwrap_or(CARD.alpha(backdrop.min(1.0 / CARD.0[3])));
    out.accent = p.accent.map(color);
    out
}

/// A notification as a panel: the stripe in its colour, the icon and the title in a row
/// above the text.
pub(crate) fn toast_panel(t: &Toast) -> Panel {
    let accent = t.color.unwrap_or(Rgba([232, 160, 48, 255]));
    let element = |kind: Kind, grow: bool, c: Option<Rgba>| Element {
        id: None,
        color: c,
        grow,
        clickable: false,
        kind,
    };
    let text = |text: &str, size: f32, w: pui::Weight| Kind::Text {
        text: text.to_string(),
        size,
        weight: w,
        align: Align::Left,
        wrap: true,
    };
    let mut children = Vec::new();
    let mut head = Vec::new();
    if let Some(name) = t.icon.as_ref() {
        head.push(element(
            Kind::Icon {
                name: name.clone(),
                size: 20.0,
            },
            false,
            Some(accent),
        ));
    }
    match t.title.as_ref() {
        Some(title) => head.push(element(
            text(title, 13.0, pui::Weight::Bold),
            true,
            Some(accent),
        )),
        // (an icon with no title stands beside the text)
        None if !head.is_empty() => head.push(element(
            text(&t.text, 14.0, pui::Weight::Regular),
            true,
            None,
        )),
        None => {}
    }
    let text_in_head = t.title.is_none() && t.icon.is_some();
    if !head.is_empty() {
        children.push(element(
            Kind::Row {
                children: head,
                gap: 8.0,
                align: RowAlign::Start,
            },
            false,
            None,
        ));
    }
    if !text_in_head {
        children.push(element(
            text(&t.text, 14.0, pui::Weight::Regular),
            false,
            None,
        ));
    }
    Panel {
        anchor: pui::Anchor::TopRight,
        x: 16.0,
        y: TOAST_TOP,
        width: TOAST_W,
        padding: 12.0,
        gap: 4.0,
        background: None,
        radius: 10.0,
        accent: Some(accent),
        visible: true,
        clickable: false,
        children,
    }
}

/// Where a card `w` x `h` goes on a screen of `screen` (logical): its anchor, moved by its
/// offset towards the middle (right and down from a middle), kept on the screen.
pub(crate) fn place(p: &Panel, w: f32, h: f32, screen: Vec2) -> Vec2 {
    let (ax, ay) = p.anchor.factors();
    let along = |a: f32, size: f32, room: f32, off: f32| {
        let at = a * (room - size) + if a == 1.0 { -off } else { off };
        at.clamp(0.0, (room - size).max(0.0))
    };
    Vec2::new(along(ax, w, screen.x, p.x), along(ay, h, screen.y, p.y))
}

/// The hit under `p` (card pixels): the last one drawn there.
pub(crate) fn hit_at(laid: &Laid, p: Vec2) -> Option<usize> {
    laid.hits.iter().rposition(|h| h.r.contains(p))
}

// --- drawing -----------------------------------------------------------------------------

/// Draw a laid-out card into `target` (`size` physical pixels, `k` of them a logical one):
/// the card's top left at `at` (logical) with its shadow round it; `hot` lights that hit's
/// shape. `clear`: the target is cleared first, else drawn over.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_card(
    gpu: &mut Gpu,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &mut Atlas,
    fonts: &Fonts,
    target: &wgpu::TextureView,
    size: (u32, u32),
    k: f32,
    laid: &Laid,
    at: Vec2,
    hot: Option<usize>,
    clear: bool,
) {
    let mut p = Painter::with_scale(k);
    let (w, h) = (size.0 as f32 / k, size.1 as f32 / k);
    let card = Rect::new(at.x, at.y, laid.w, laid.h);
    let phys = |r: Rect| [r.x * k, r.y * k, r.right() * k, r.bottom() * k];
    // layer 0: the whole target; 1: the card, cut round; then one per rounded shape
    let flat = |clip: [f32; 4], radius: f32| Layer {
        viewport: [0.0, 0.0, w, h],
        ..Layer::flat(clip, radius, 1.0)
    };
    let mut layers = vec![
        flat([0.0, 0.0, size.0 as f32, size.1 as f32], 0.0),
        flat(phys(card), laid.radius * k),
    ];
    let mut draws: Vec<Draw> = Vec::new();
    let mut put = |p: &Painter, from: u32, layer: usize| {
        if p.len() > from {
            draws.push(Draw {
                buffer: 0,
                range: from..p.len(),
                layer,
                texture: 0,
            });
        }
    };
    let n = p.len();
    p.shadow(
        Rect::new(card.x, card.y + 2.0, card.w, card.h),
        laid.radius,
        SHADOW_BLUR,
        SHADOW,
    );
    put(&p, n, 0);
    let n = p.len();
    p.rect(card, laid.background);
    if let Some(a) = laid.accent {
        p.rect(Rect::new(card.x, card.y, STRIPE, card.h), a);
    }
    put(&p, n, 1);
    for item in &laid.items {
        let n = p.len();
        match item {
            Item::Rounded {
                r,
                radius,
                color,
                hot: me,
            } => {
                let c = if hot.is_some() && *me == hot {
                    lit(*color)
                } else {
                    *color
                };
                let r = Rect::new(r.x + at.x, r.y + at.y, r.w, r.h);
                if layers.len() < MAX_LAYERS {
                    p.rect(r, c);
                    layers.push(flat(phys(r), radius * k));
                    put(&p, n, layers.len() - 1);
                } else {
                    p.rounded(r, *radius, c);
                    put(&p, n, 1);
                }
            }
            Item::Rect { r, color } => {
                p.rect(Rect::new(r.x + at.x, r.y + at.y, r.w, r.h), *color);
                put(&p, n, 1);
            }
            Item::Text {
                text,
                px,
                weight,
                at: base,
                color,
            } => {
                p.text_as_is(
                    atlas,
                    fonts,
                    text,
                    *px,
                    *weight,
                    *base + at,
                    TextAlign::Left,
                    *color,
                );
                put(&p, n, 1);
            }
            Item::Icon {
                name,
                center,
                size,
                color,
            } => {
                p.icon(atlas, name, *center + at, *size, *color);
                put(&p, n, 1);
            }
        }
    }
    let verts: Vec<Vertex> = p.verts;
    gpu.upload(device, queue, 0, &verts);
    gpu.upload_atlas(queue, atlas);
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("plugin panel"),
    });
    gpu.render(
        device,
        queue,
        &mut enc,
        target,
        size,
        clear.then_some(wgpu::Color::TRANSPARENT),
        &layers,
        &draws,
    );
    // (now: the next card's vertices and layers go into the same buffers)
    queue.submit([enc.finish()]);
}

/// A card on the screen: what it was laid out from, its texture and where it is.
struct Card {
    revision: u64,
    scale: f32,
    backdrop: f32,
    laid: Laid,
    tex: Option<(TextureId, u32, u32)>,
    /// The hit lit when it was drawn (None: to be drawn).
    drawn: Option<Option<usize>>,
    /// The card's top left on the window, physical pixels.
    origin: Vec2,
    /// A notification's place in the stack (logical), eased.
    y: Option<f32>,
    seen: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Panel(u64, String),
    Toast(u64),
}

/// What the frame tells the panels.
pub(crate) struct PanelsFrame {
    /// The interface's part of the window (x, y, w, h; physical pixels).
    pub hud: [f32; 4],
    /// Physical pixels per logical one: the screen's scale times the interface size.
    pub scale: f32,
    /// The game's menus are open (or VR): nothing shows, the mouse is the game's.
    pub hidden: bool,
    /// The mouse, physical pixels on the window.
    pub cursor: (f32, f32),
    pub dt: f32,
    /// `ui::backdrop` of the opacity setting.
    pub backdrop: f32,
    /// The navigator's panel (physical x0, y0, x1, y1), which the notifications stay clear of.
    pub navigator: Option<[f32; 4]>,
}

/// The plugins' panels on the screen.
#[derive(Default)]
pub(crate) struct PluginPanels {
    gpu: Option<Gpu>,
    fonts: Option<Fonts>,
    atlas: Option<Atlas>,
    cards: HashMap<Key, Card>,
    frame: u64,
    scale: f32,
}

impl PluginPanels {
    /// Lay out, draw what changed and put every card on the screen (`scene.overlays`).
    pub fn frame(&mut self, r: &Renderer, scene: &mut Scene, ui: &mut UiState, f: &PanelsFrame) {
        self.frame += 1;
        let k = f.scale.max(0.25);
        self.scale = k;
        let screen = Vec2::new(f.hud[2] / k, f.hud[3] / k);
        ui.set_screen(screen.x, screen.y, k);
        ui.set_shown(!f.hidden);
        ui.tick(f.dt);
        if !f.hidden && (!ui.panels().is_empty() || !ui.toasts().is_empty()) {
            self.show(r, scene, ui, f, k, screen);
        }
        // what is not there any more gives its texture back (hidden under a menu, a card
        // keeps it for when the menu closes)
        let frame = self.frame;
        let exists = |key: &Key| match key {
            Key::Panel(owner, id) => ui
                .panels()
                .iter()
                .any(|e| e.owner == *owner && e.id == *id && e.panel.visible),
            Key::Toast(serial) => ui.toasts().iter().any(|t| t.serial == *serial),
        };
        let gone: Vec<Key> = self
            .cards
            .iter()
            .filter(|(key, c)| c.seen != frame && (!f.hidden || !exists(key)))
            .map(|(key, _)| key.clone())
            .collect();
        for key in gone {
            if let Some((t, _, _)) = self.cards.remove(&key).and_then(|c| c.tex) {
                r.free_texture(scene, t);
                scene.premultiplied.remove(&t);
            }
        }
    }

    fn show(
        &mut self,
        r: &Renderer,
        scene: &mut Scene,
        ui: &mut UiState,
        f: &PanelsFrame,
        k: f32,
        screen: Vec2,
    ) {
        let fonts = self.fonts.get_or_insert_with(Fonts::new);
        let atlas = self.atlas.get_or_insert_with(|| Atlas::new(1024));
        let gpu = self
            .gpu
            .get_or_insert_with(|| Gpu::new(&r.device, r.format(), 1, atlas.size));
        atlas.begin_frame();
        let generation = atlas.generation;
        let focused = ui.focused();
        let cursor = Vec2::new(f.cursor.0, f.cursor.1);
        let margin = (MARGIN * k).ceil();
        let mut cards: Vec<Key> = Vec::new();
        for e in ui.panels().iter().filter(|e| e.panel.visible) {
            let key = Key::Panel(e.owner, e.id.clone());
            let card = self.cards.entry(key.clone()).or_insert_with(|| Card {
                revision: u64::MAX,
                scale: 0.0,
                backdrop: 0.0,
                laid: Laid::default(),
                tex: None,
                drawn: None,
                origin: Vec2::ZERO,
                y: None,
                seen: 0,
            });
            if card.revision != e.revision || card.scale != k || card.backdrop != f.backdrop {
                card.laid = layout(&e.panel, fonts, f.backdrop);
                card.revision = e.revision;
                card.scale = k;
                card.backdrop = f.backdrop;
                card.drawn = None;
            }
            let at = place(&e.panel, card.laid.w, card.laid.h, screen);
            card.origin = Vec2::new(f.hud[0] + (at.x * k).round(), f.hud[1] + (at.y * k).round());
            cards.push(key);
        }
        // the notifications, newest at the top, under the navigator when it is in the way
        let right = screen.x - 16.0;
        let mut top = TOAST_TOP;
        if let Some(n) = f.navigator {
            let (x0, y1) = ((n[0] - f.hud[0]) / k, n[3] / k);
            if x0 < right && (n[2] - f.hud[0]) / k > right - TOAST_W && n[1] / k < screen.y * 0.5 {
                top = top.max(y1 + TOAST_GAP);
            }
        }
        let ease = 1.0 - (-f.dt.max(0.0) * 12.0).exp();
        for t in ui.toasts().iter().rev() {
            let key = Key::Toast(t.serial);
            let card = self.cards.entry(key.clone()).or_insert_with(|| Card {
                revision: 0,
                scale: 0.0,
                backdrop: 0.0,
                laid: Laid::default(),
                tex: None,
                drawn: None,
                origin: Vec2::ZERO,
                y: None,
                seen: 0,
            });
            if card.scale != k || card.backdrop != f.backdrop {
                card.laid = layout(&toast_panel(t), fonts, f.backdrop);
                card.scale = k;
                card.backdrop = f.backdrop;
                card.drawn = None;
            }
            if top + card.laid.h > screen.y {
                break;
            }
            let y = match card.y {
                Some(y) => y + (top - y) * ease,
                None => top,
            };
            card.y = Some(y);
            // sliding in from the right edge, and out again at the end
            let away = card.laid.w + MARGIN + 16.0;
            let left = t.seconds - t.age;
            let slide = if t.age < TOAST_IN {
                1.0 - (t.age / TOAST_IN).powf(0.5)
            } else if left < TOAST_OUT {
                1.0 - left / TOAST_OUT
            } else {
                0.0
            };
            let x = right - card.laid.w + slide.clamp(0.0, 1.0) * away;
            card.origin = Vec2::new(f.hud[0] + (x * k).round(), f.hud[1] + (y * k).round());
            top += card.laid.h + TOAST_GAP;
            cards.push(key);
        }
        for key in cards {
            let Some(card) = self.cards.get_mut(&key) else {
                continue;
            };
            card.seen = self.frame;
            let hot = match key {
                Key::Panel(..) if focused => {
                    let local = (cursor - card.origin) / k;
                    hit_at(&card.laid, local).filter(|&i| card.laid.hits[i].element.is_some())
                }
                _ => None,
            };
            let (tw, th) = (
                ((card.laid.w * k).ceil() + 2.0 * margin) as u32,
                ((card.laid.h * k).ceil() + 2.0 * margin) as u32,
            );
            // (a card larger than the graphics chip's largest texture is not drawn: the
            // texture would come out smaller than it was drawn for)
            if tw.max(th) > r.device.limits().max_texture_dimension_2d {
                continue;
            }
            if card.tex.is_some_and(|t| (t.1, t.2) != (tw, th)) {
                let (t, _, _) = card.tex.take().unwrap();
                r.free_texture(scene, t);
                scene.premultiplied.remove(&t);
            }
            if card.tex.is_none() {
                let t = r.add_render_texture(scene, tw, th);
                scene.premultiplied.insert(t);
                card.tex = Some((t, tw, th));
                card.drawn = None;
            }
            let (tex, _, _) = card.tex.unwrap();
            if card.drawn != Some(hot) {
                if let Some(view) = r.texture_view(scene, tex) {
                    draw_card(
                        gpu,
                        &r.device,
                        &r.queue,
                        atlas,
                        fonts,
                        &view,
                        (tw, th),
                        k,
                        &card.laid,
                        Vec2::splat(margin / k),
                        hot,
                        true,
                    );
                    card.drawn = Some(hot);
                }
            }
            let (x, y) = (card.origin.x - margin, card.origin.y - margin);
            scene
                .overlays
                .push((tex, [x, y, x + tw as f32, y + th as f32]));
        }
        // (the atlas ran full in the middle of it: what was drawn then is drawn again)
        if atlas.generation != generation {
            for c in self.cards.values_mut() {
                c.drawn = None;
            }
        }
    }

    /// What a click at `(x, y)` (physical pixels on the window) is on: the plugin, the panel
    /// and the element (None: the panel itself) - the topmost panel's clickable part there.
    pub fn click_at(&self, ui: &UiState, x: f32, y: f32) -> Option<(u64, String, Option<String>)> {
        let k = self.scale.max(0.25);
        for e in ui.panels().iter().rev().filter(|e| e.panel.visible) {
            let Some(card) = self.cards.get(&Key::Panel(e.owner, e.id.clone())) else {
                continue;
            };
            if card.seen != self.frame {
                continue;
            }
            let local = (Vec2::new(x, y) - card.origin) / k;
            if !Rect::new(0.0, 0.0, card.laid.w, card.laid.h).contains(local) {
                continue;
            }
            // (on a panel, a click on no clickable part of it is nobody's - nor the bus's)
            return hit_at(&card.laid, local)
                .map(|i| (e.owner, e.id.clone(), card.laid.hits[i].element.clone()));
        }
        None
    }
}

/// The line the game shows while the panels have the mouse.
pub(crate) const FOCUS_NOTE: &str = "The mouse is on the plugin panels · Esc gives it back";

/// Whether the plugins' panels have the mouse (`omsi.ui.focus`): from the plugins alone, for
/// where the rest of the game is borrowed.
pub(crate) fn focused(plugins: &Option<omsi_plugin::Plugins>) -> bool {
    plugins.as_ref().is_some_and(|p| p.ui.borrow().focused())
}

impl crate::App {
    /// Whether the plugins' panels have the mouse (`omsi.ui.focus`).
    pub(crate) fn plugin_focus(&self) -> bool {
        focused(&self.plugins)
    }

    /// Esc while the panels have the mouse: it goes back to the bus (and not on to the
    /// menu). True when it did.
    pub(crate) fn release_plugin_focus(&mut self) -> bool {
        let Some(p) = self.plugins.as_ref().filter(|p| p.ui.borrow().focused()) else {
            return false;
        };
        p.ui.borrow_mut().release_focus();
        true
    }

    /// The left button while the panels have the mouse: a press on a clickable part goes to
    /// its plugin as `ui_click`; the bus gets none of them.
    pub(crate) fn plugin_click(&mut self, pressed: bool) {
        let Some(p) = self.plugins.as_ref() else {
            return;
        };
        if !pressed {
            return;
        }
        let hit = self
            .plugin_panels
            .click_at(&p.ui.borrow(), self.cursor.0, self.cursor.1);
        if let Some((owner, panel, element)) = hit {
            p.ui.borrow_mut().click(owner, &panel, element.as_deref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omsi_plugin::ui::{panel_from_source, Anchor};

    fn panel(src: &str) -> Panel {
        panel_from_source(src).unwrap()
    }

    #[test]
    fn a_stack_wraps_its_texts_and_grows_down() {
        let fonts = Fonts::new();
        let p = panel(
            r#"{ width = 200, padding = 10, gap = 5, children = {
            { type = "text", text = "Ein ziemlich langer Satz, der umbrochen werden muss", size = 14 },
            { type = "divider" },
            { type = "button", id = "go", text = "Los" },
        } }"#,
        );
        let l = layout(&p, &fonts, 1.0);
        let texts = l
            .items
            .iter()
            .filter(|i| matches!(i, Item::Text { .. }))
            .count();
        // several lines and the button's label
        assert!(texts >= 3, "{:?}", l.items);
        let lines = texts - 1;
        let want = 10.0 + lines as f32 * 14.0 * LINE + 5.0 + 1.0 + 5.0 + BUTTON_H + 10.0;
        assert!((l.h - want.ceil()).abs() < 1.0, "{} {want}", l.h);
        for i in &l.items {
            if let Item::Text {
                text,
                px,
                weight,
                at,
                ..
            } = i
            {
                assert!(
                    at.x >= 10.0 && at.x + fonts.width_as_is(text, *px, *weight) <= 190.5,
                    "{text} at {at}"
                );
            }
        }
        // (the card's height is whole pixels, rounded up)
        let [b] = &l.hits[..] else {
            panic!("{:?}", l.hits)
        };
        assert_eq!(
            (b.r.x, b.r.w, b.r.h, b.element.as_deref()),
            (10.0, 180.0, BUTTON_H, Some("go"))
        );
        assert!(
            (b.r.bottom() + 10.0 - l.h).abs() < 1.0,
            "{:?} in {}",
            b.r,
            l.h
        );
    }

    #[test]
    fn a_row_gives_the_rest_to_what_grows() {
        let fonts = Fonts::new();
        let p = panel(
            r#"{ width = 300, padding = 10, children = { { type = "row", gap = 8, children = {
            { type = "icon", name = "schedule", size = 20 },
            { type = "bar", value = 0.5, grow = true },
            { type = "badge", text = "+2" },
        } } } }"#,
        );
        let l = layout(&p, &fonts, 1.0);
        let rounded: Vec<Rect> = l
            .items
            .iter()
            .filter_map(|i| {
                if let Item::Rounded { r, .. } = i {
                    Some(*r)
                } else {
                    None
                }
            })
            .collect();
        // the track from after the icon to before the badge, the badge at the right edge
        let (track, badge) = (rounded[0], rounded[2]);
        assert_eq!(track.x, 10.0 + 20.0 + 8.0);
        assert!(
            (badge.right() - 290.0).abs() < 0.01 && (track.right() + 8.0 - badge.x).abs() < 0.01,
            "{track:?} {badge:?}"
        );
        assert_eq!(l.h, (10.0 + BADGE_H + 10.0f32).ceil());
    }

    #[test]
    fn panels_are_placed_by_their_anchor_and_stay_on_the_screen() {
        let screen = Vec2::new(1920.0, 1080.0);
        let mut p = panel("{ x = 16, y = 20 }");
        assert_eq!(place(&p, 300.0, 100.0, screen), Vec2::new(16.0, 20.0));
        p.anchor = Anchor::BottomRight;
        assert_eq!(
            place(&p, 300.0, 100.0, screen),
            Vec2::new(1920.0 - 316.0, 1080.0 - 120.0)
        );
        p.anchor = Anchor::Center;
        assert_eq!(
            place(&p, 300.0, 100.0, screen),
            Vec2::new(810.0 + 16.0, 490.0 + 20.0)
        );
        p.anchor = Anchor::TopLeft;
        p.x = -50.0;
        assert_eq!(place(&p, 300.0, 100.0, screen).x, 0.0);
    }

    #[test]
    fn clicks_find_the_innermost_part() {
        let fonts = Fonts::new();
        let p = panel(
            r#"{ width = 300, clickable = true, children = {
            { type = "row", id = "line", clickable = true, children = {
                { type = "text", text = "Linie 5", grow = true },
                { type = "button", id = "take", text = "Nehmen" },
            } },
            { type = "text", text = "nicht klickbar" },
        } }"#,
        );
        let l = layout(&p, &fonts, 1.0);
        let at = |i: usize| l.hits[i].r.center();
        let element = |p: Vec2| hit_at(&l, p).and_then(|i| l.hits[i].element.clone());
        let button = l
            .hits
            .iter()
            .position(|h| h.element.as_deref() == Some("take"))
            .unwrap();
        assert_eq!(element(at(button)).as_deref(), Some("take"));
        assert_eq!(
            element(Vec2::new(20.0, at(button).y)).as_deref(),
            Some("line")
        );
        // below the row: the panel itself
        assert_eq!(hit_at(&l, Vec2::new(20.0, l.h - 14.0)), Some(0));
        assert_eq!(element(Vec2::new(20.0, l.h - 14.0)), None);
    }

    /// A picture of a few panels and notifications over a stand-in for the road, drawn as the
    /// game draws them (needs a graphics adapter):
    /// `cargo test -p omsi-app --lib plugin_ui::tests::preview -- --ignored`, written to
    /// `OMSI_UI_PREVIEW` or `target/ui-preview.png`.
    #[test]
    #[ignore]
    fn preview() {
        let (w, h, k) = (1600u32, 900u32, 1.0f32);
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("device");
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut gpu = Gpu::new(&device, format, 1, 1024);
        let fonts = Fonts::new();
        let mut atlas = Atlas::new(1024);
        atlas.begin_frame();
        let screen = Vec2::new(w as f32 / k, h as f32 / k);
        let panels = [
            (
                r##"{ anchor = "top_left", x = 16, y = 60, width = 340, accent = "#F47F30", children = {
                    { type = "row", children = { { type = "icon", name = "directions_bus", color = "#F47F30" }, { type = "text", text = "Linie 42 · Kurs 3", size = 16, weight = "bold", grow = true }, { type = "badge", text = "+1:20", color = "#C62828" } } },
                    { type = "text", text = "Nächster Halt: Grundorf, Krankenhaus Nord (Wendeschleife am Haupteingang)", color = "#C8C8C8" },
                    { type = "row", gap = 6, children = { { type = "icon", name = "schedule", size = 16, color = "#8E8E8E" }, { type = "text", text = "ab 08:59", size = 13, color = "#8E8E8E" }, { type = "space", size = 8 }, { type = "icon", name = "group", size = 16, color = "#8E8E8E" }, { type = "text", text = "23 Fahrgäste", size = 13, color = "#8E8E8E" } } },
                    { type = "bar", value = 0.62 },
                } }"##,
                None,
            ),
            (
                r##"{ anchor = "bottom_left", x = 16, y = 16, width = 260, children = {
                    { type = "row", align = "between", children = { { type = "text", text = "Tagesverdienst", color = "#8E8E8E" }, { type = "text", text = "184,50 €", size = 18, weight = "bold" } } },
                    { type = "divider" },
                    { type = "row", align = "between", children = { { type = "text", text = "Pünktlichkeit" }, { type = "badge", text = "94 %", color = "#2E7D32" } } },
                    { type = "row", align = "between", children = { { type = "text", text = "Fahrstil" }, { type = "badge", text = "B", color = "#F9A825" } } },
                } }"##,
                None,
            ),
            (
                r##"{ anchor = "center", width = 380, padding = 16, gap = 10, clickable = true, children = {
                    { type = "text", text = "Schicht beendet", size = 20, weight = "bold", align = "center" },
                    { type = "text", text = "Du hast 7 von 7 Fahrten gefahren. Möchtest du die nächste Schicht direkt annehmen?", align = "center", color = "#C8C8C8" },
                    { type = "space", size = 4 },
                    { type = "row", gap = 8, children = {
                        { type = "button", id = "later", text = "Später", grow = true },
                        { type = "button", id = "take", text = "Annehmen", icon = "check", color = "#E8A030", grow = true },
                    } },
                    { type = "button", id = "details", text = "Details", icon = "receipt_long" },
                } }"##,
                Some("details"),
            ),
        ];
        let mut first = true;
        for (src, hot) in panels {
            let p = panel(src);
            let laid = layout(&p, &fonts, 1.0);
            let at = place(&p, laid.w, laid.h, screen);
            let hot = hot.and_then(|id| {
                laid.hits
                    .iter()
                    .position(|h| h.element.as_deref() == Some(id))
            });
            draw_card(
                &mut gpu,
                &device,
                &queue,
                &mut atlas,
                &fonts,
                &view,
                (w, h),
                k,
                &laid,
                at,
                hot,
                first,
            );
            first = false;
        }
        let toasts = [
            Toast {
                owner: 1,
                serial: 1,
                text: "Fahrt 3 pünktlich beendet: +12,40 €".into(),
                title: Some("Karriere".into()),
                icon: Some("payments".into()),
                color: Some(Rgba([46, 125, 50, 255])),
                seconds: 5.0,
                age: 1.0,
            },
            Toast {
                owner: 1,
                serial: 2,
                text: "Rote Ampel überfahren".into(),
                title: None,
                icon: Some("warning".into()),
                color: Some(Rgba([198, 40, 40, 255])),
                seconds: 5.0,
                age: 1.0,
            },
            Toast {
                owner: 1,
                serial: 3,
                text: "Schichtbeginn in 10 Minuten am Betriebshof".into(),
                title: None,
                icon: None,
                color: None,
                seconds: 5.0,
                age: 1.0,
            },
        ];
        let mut y = TOAST_TOP;
        for t in &toasts {
            let laid = layout(&toast_panel(t), &fonts, 1.0);
            draw_card(
                &mut gpu,
                &device,
                &queue,
                &mut atlas,
                &fonts,
                &view,
                (w, h),
                k,
                &laid,
                Vec2::new(screen.x - 16.0 - laid.w, y),
                None,
                false,
            );
            y += laid.h + TOAST_GAP;
        }
        // read back and laid over a stand-in for the road: sky, buildings, the street
        let stride = (w * 4).div_ceil(256) * 256;
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (stride * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let data = buf.slice(..).get_mapped_range();
        let mut img = image::RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let t = y as f32 / h as f32;
                let bg: [f32; 3] = if t < 0.45 {
                    [120.0 + 80.0 * t, 165.0 + 60.0 * t, 225.0]
                } else if t < 0.7 && (x / 140) % 3 != 0 {
                    [150.0 + (x % 140) as f32 * 0.3, 140.0, 128.0]
                } else if t < 0.7 {
                    [205.0, 200.0, 190.0]
                } else {
                    [70.0, 72.0, 76.0]
                };
                let i = (y * stride + x * 4) as usize;
                let a = data[i + 3] as f32 / 255.0;
                let px = |c: usize| (data[i + c] as f32 + bg[c] * (1.0 - a)).min(255.0) as u8;
                img.put_pixel(x, y, image::Rgba([px(0), px(1), px(2), 255]));
            }
        }
        let out = std::env::var("OMSI_UI_PREVIEW")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-preview.png")
            });
        img.save(&out).unwrap();
        println!("wrote {}", out.display());
    }

    #[test]
    fn a_toast_is_a_card_with_its_colour() {
        let fonts = Fonts::new();
        let t = Toast {
            owner: 1,
            serial: 1,
            text: "Fahrt beendet".into(),
            title: Some("Karriere".into()),
            icon: Some("payments".into()),
            color: None,
            seconds: 5.0,
            age: 0.0,
        };
        let l = layout(&toast_panel(&t), &fonts, 1.0);
        assert_eq!(l.w, TOAST_W);
        assert!(l.accent.is_some());
        assert_eq!(
            l.items
                .iter()
                .filter(|i| matches!(i, Item::Icon { .. }))
                .count(),
            1
        );
        assert_eq!(
            l.items
                .iter()
                .filter(|i| matches!(i, Item::Text { .. }))
                .count(),
            2
        );
    }
}
