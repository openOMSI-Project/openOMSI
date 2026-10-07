//! Choosing the interface's accent colour (`crate::accent`) on the Settings page: the presets
//! as round swatches and a picker for any other colour. A choice recolours the whole launcher
//! at once and is saved as the setting `accent`; the game takes it when it starts.
//!
//! (classic copy of the new launcher's `accent_pick`, without the bar's palette: this launcher
//! has no bar)

use std::cell::RefCell;

use glam::Vec2;
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};
use serde_json::{json, Value};

use super::theme::*;
use super::ui::{id_of, Ui};
use crate::accent;
use crate::launcher::livery::colour;

/// A swatch's size and the room it takes along the row.
const SWATCH: f32 = 26.0;
const SWATCH_STEP: f32 = 34.0;
/// The "Custom…" chip's width.
const CUSTOM_W: f32 = 112.0;
/// The picker's square, the hue strip under it and the code's row.
const SQUARE_H: f32 = 120.0;
const PICKER_H: f32 = SQUARE_H + 8.0 + 14.0 + 10.0 + 32.0;

/// A picker's own state: open or not, its hue, saturation and value (the hue stays where it was
/// while the colour is grey), the code as typed, and the colour it last showed.
#[derive(Default)]
struct Picker {
    open: bool,
    hsv: [f32; 3],
    hex: String,
    shown: Option<u32>,
}

thread_local! {
    static PICKERS: RefCell<hashbrown::HashMap<String, Picker>> = RefCell::default();
}

/// The colour the settings `s` choose (`0xRRGGBB`).
pub fn current(s: &Value) -> u32 {
    s.get("accent").and_then(|v| v.as_str()).and_then(accent::parse_hex).unwrap_or(accent::DEFAULT)
}

/// Choose `rgb`: the setting (saved a moment later) and the accent, at once.
fn choose(s: &mut Value, dirty: &mut f32, rgb: u32) {
    s["accent"] = json!(accent::hex(rgb));
    *dirty = 0.3;
    accent::set(rgb);
}

/// How tall [`chooser`] comes out in `w` pixels: the swatches (one row, or two where the
/// "Custom…" chip does not fit after them) and the picker while it is open.
pub fn chooser_height(name: &str, w: f32) -> f32 {
    let rows = if swatches_fit(w) { 1.0 } else { 2.0 };
    let open = PICKERS.with(|p| p.borrow().get(name).is_some_and(|p| p.open));
    rows * SWATCH_STEP + if open { PICKER_H + 12.0 } else { 0.0 }
}

fn swatches_fit(w: f32) -> bool {
    accent::PRESETS.len() as f32 * SWATCH_STEP + CUSTOM_W <= w
}

/// The presets and "Custom…" from (`x`, `y`) across `w`, and the picker under them while it is
/// open. Returns the height taken.
pub fn chooser(ui: &mut Ui, name: &str, x: f32, y: f32, w: f32, s: &mut Value, dirty: &mut f32) -> f32 {
    let cur = current(s);
    let mut picked = None;
    for (k, (word, rgb)) in accent::PRESETS.iter().enumerate() {
        let c = Vec2::new(x + k as f32 * SWATCH_STEP + SWATCH * 0.5, y + SWATCH_STEP * 0.5);
        if swatch(ui, &format!("{name}-sw{k}"), c, *rgb, *rgb == cur, word) {
            picked = Some(*rgb);
        }
    }
    let custom_on = !accent::PRESETS.iter().any(|p| p.1 == cur);
    let (cx, cy) = if swatches_fit(w) { (x + accent::PRESETS.len() as f32 * SWATCH_STEP + 4.0, y) } else { (x, y + SWATCH_STEP) };
    let open = PICKERS.with(|p| p.borrow().get(name).is_some_and(|p| p.open));
    let chip = Rect::new(cx, cy + (SWATCH_STEP - 30.0) * 0.5, CUSTOM_W - 8.0, 30.0);
    if custom_chip(ui, &format!("{name}-custom"), chip, cur, custom_on, open) {
        PICKERS.with(|p| {
            let mut p = p.borrow_mut();
            let e = p.entry(name.to_string()).or_default();
            e.open = !e.open;
        });
    }
    let mut h = chooser_height(name, w);
    if let Some(rgb) = picked {
        choose(s, dirty, rgb);
    }
    if PICKERS.with(|p| p.borrow().get(name).is_some_and(|p| p.open)) {
        let top = y + if swatches_fit(w) { SWATCH_STEP } else { 2.0 * SWATCH_STEP } + 8.0;
        if let Some(rgb) = picker(ui, name, Rect::new(x, top, w.min(360.0), PICKER_H), current(s)) {
            choose(s, dirty, rgb);
        }
        h = top + PICKER_H + 4.0 - y;
    }
    h
}

/// A round swatch of `rgb` centred on `c`; the chosen one ringed and ticked.
fn swatch(ui: &mut Ui, name: &str, c: Vec2, rgb: u32, on: bool, word: &str) -> bool {
    let hit = Rect::new(c.x - SWATCH_STEP * 0.5, c.y - SWATCH_STEP * 0.5, SWATCH_STEP, SWATCH_STEP);
    let (h, _, clicked) = ui.interact(id_of(name), hit);
    let t = ui.anim(id_of(name) ^ 0x5a7c, if h || on { 1.0 } else { 0.0 }, 0.08);
    let sh = accent::Shades::of(rgb);
    let [r, g, b] = accent::unpack(rgb);
    if on {
        ui.p().circle(c, SWATCH * 0.5 + 3.5, TEXT);
        ui.p().circle(c, SWATCH * 0.5 + 1.5, PANEL);
    }
    ui.p().circle(c, SWATCH * 0.5 * (0.88 + 0.12 * t), Color::rgba(r, g, b, 1.0));
    if on {
        ui.icon("check", c, 16.0, sh.on);
    }
    ui.tooltip(hit, word);
    clicked
}

/// "Custom…": a chip with the palette icon; the colour chosen in it as a dot while it is not
/// one of the presets.
fn custom_chip(ui: &mut Ui, name: &str, r: Rect, cur: u32, on: bool, open: bool) -> bool {
    let (h, _, clicked) = ui.interact(id_of(name), r);
    let t = ui.anim(id_of(name), if h { 1.0 } else { 0.0 }, 0.08);
    ui.p().rounded(r, r.h * 0.5, FIELD.mix(HOVER, t));
    ui.p().rounded_border(r, r.h * 0.5, if on || open { 1.5 } else { 1.0 }, if on || open { accent() } else { EDGE });
    let mut x = r.x + 12.0;
    if on {
        let [cr, cg, cb] = accent::unpack(cur);
        ui.p().circle(Vec2::new(x + 7.0, r.center().y), 7.0, Color::rgba(cr, cg, cb, 1.0));
    } else {
        ui.icon("palette", Vec2::new(x + 7.0, r.center().y), 16.0, if h { TEXT } else { TEXT_SOFT });
    }
    x += 20.0;
    ui.text_in("Custom…", Rect::new(x, r.y, r.right() - x - 8.0, r.h), 12.5, Weight::Medium, if h || open { TEXT } else { TEXT_SOFT }, Align::Left);
    clicked
}

/// The picker in `r`: the square of saturation (across) and value (up), the hue's strip, the
/// code and the colour itself. Returns a colour chosen in it.
fn picker(ui: &mut Ui, name: &str, r: Rect, cur: u32) -> Option<u32> {
    let mut st = PICKERS.with(|p| p.borrow_mut().remove(name)).unwrap_or_default();
    st.open = true;
    // (it follows a colour chosen elsewhere: a swatch, the other picker)
    if st.shown != Some(cur) {
        let hsv = colour::hsv(colour::unit(accent::unpack(cur)));
        st.hsv = if hsv[1] < 1e-3 || hsv[2] < 1e-3 { [st.hsv[0], hsv[1], hsv[2]] } else { hsv };
        st.hex = accent::hex(cur);
        st.shown = Some(cur);
    }
    let mut changed = None;
    let sq = Rect::new(r.x, r.y, r.w, SQUARE_H);
    let hue = colour::bytes(colour::rgb_of_hsv([st.hsv[0], 1.0, 1.0]));
    ui.p().rounded(sq, 8.0, Color::rgba(hue[0], hue[1], hue[2], 1.0));
    ui.p().gradient_h(sq, Color::WHITE, Color::WHITE.alpha(0.0));
    ui.p().gradient(sq, Color::BLACK.alpha(0.0), Color::BLACK);
    let (_, held, _) = ui.interact(id_of(&format!("{name}-sv")), sq);
    if held {
        let m = ui.input.mouse;
        st.hsv[1] = ((m.x - sq.x) / sq.w).clamp(0.0, 1.0);
        st.hsv[2] = 1.0 - ((m.y - sq.y) / sq.h).clamp(0.0, 1.0);
        changed = Some(accent::pack(colour::bytes(colour::rgb_of_hsv(st.hsv))));
    }
    let [cr, cg, cb] = accent::unpack(cur);
    let knob = Vec2::new(sq.x + st.hsv[1] * sq.w, sq.y + (1.0 - st.hsv[2]) * sq.h);
    ui.p().circle(knob, 8.0, Color::WHITE);
    ui.p().circle(knob, 6.0, Color::rgba(cr, cg, cb, 1.0));
    // the hues
    let bar = Rect::new(r.x, sq.bottom() + 8.0, r.w, 14.0);
    for k in 0..6 {
        let a = colour::bytes(colour::rgb_of_hsv([k as f32 * 60.0, 1.0, 1.0]));
        let b = colour::bytes(colour::rgb_of_hsv([(k + 1) as f32 * 60.0, 1.0, 1.0]));
        ui.p().gradient_h(Rect::new(bar.x + bar.w * k as f32 / 6.0, bar.y, bar.w / 6.0 + 0.5, bar.h), Color::rgba(a[0], a[1], a[2], 1.0), Color::rgba(b[0], b[1], b[2], 1.0));
    }
    let (_, held, _) = ui.interact(id_of(&format!("{name}-hue")), bar.pad(0.0, -4.0));
    if held {
        st.hsv[0] = ((ui.input.mouse.x - bar.x) / bar.w).clamp(0.0, 0.9999) * 360.0;
        // (a grey has no hue to turn: it is given some colour)
        if st.hsv[1] < 0.05 || st.hsv[2] < 0.05 {
            st.hsv[1] = st.hsv[1].max(0.8);
            st.hsv[2] = st.hsv[2].max(0.85);
        }
        changed = Some(accent::pack(colour::bytes(colour::rgb_of_hsv(st.hsv))));
    }
    let hx = bar.x + st.hsv[0] / 360.0 * bar.w;
    ui.p().rounded_border(Rect::new(hx - 4.0, bar.y - 3.0, 8.0, bar.h + 6.0), 3.0, 2.0, Color::WHITE);
    // the code and the colour itself
    let row = bar.bottom() + 10.0;
    let mut hex = st.hex.clone();
    if ui.text_input(&format!("{name}-hex"), Rect::new(r.x, row, 118.0, 32.0), &mut hex, "#f58620", None) {
        st.hex = hex.clone();
        if let Some(v) = accent::parse_hex(&hex).filter(|_| hex.trim().trim_start_matches('#').len() == 6) {
            st.hsv = colour::hsv(colour::unit(accent::unpack(v)));
            changed = Some(v);
        }
    }
    let shown = changed.unwrap_or(cur);
    let sh = accent::Shades::of(shown);
    let sample = Rect::new(r.x + 126.0, row, (r.w - 126.0).max(40.0), 32.0);
    ui.p().rounded(sample, RADIUS, sh.base);
    ui.text_in("Aa", sample, 13.0, Weight::Bold, sh.on, Align::Center);
    if let Some(v) = changed {
        if ui.focus != Some(id_of(&format!("{name}-hex"))) {
            st.hex = accent::hex(v);
        }
        st.shown = Some(v);
    }
    PICKERS.with(|p| p.borrow_mut().insert(name.to_string(), st));
    changed
}
