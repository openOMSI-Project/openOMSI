//! The launcher's looks: a palette each, chosen under Settings.
#![allow(non_snake_case)]

use omsi_ui::Color;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Palette {
    /// The name in `settings.cfg` (`launcher_theme`) and the one shown.
    pub key: &'static str,
    pub name: &'static str,
    accent: Color,
    accent_2: Color,
    on_accent: Color,
    danger: Color,
    ok: Color,
    warn: Color,
    text: Color,
    text_soft: Color,
    text_dim: Color,
    text_faint: Color,
    backdrop: Color,
    rail: Color,
    panel: Color,
    field: Color,
    hover: Color,
    selected: Color,
    track: Color,
    knob: Color,
    popup: Color,
    edge: Color,
    shade: Color,
    shadow: Color,
    /// What a surface is tinted with under the mouse: white on a dark look, dark on a light.
    lift: Color,
    road: Color,
    road_main: Color,
    road_casing: Color,
}

const fn c(r: u8, g: u8, b: u8) -> Color {
    Color::rgba(r, g, b, 1.0)
}

pub const THEMES: [Palette; 3] = [
    Palette {
        key: "midnight",
        name: "Midnight",
        accent: c(41, 98, 240),
        accent_2: c(255, 196, 0),
        on_accent: c(255, 255, 255),
        danger: c(255, 104, 96),
        ok: c(80, 208, 140),
        warn: c(255, 170, 60),
        text: c(236, 240, 248),
        text_soft: c(192, 200, 216),
        text_dim: c(134, 146, 170),
        text_faint: c(86, 98, 122),
        backdrop: c(7, 10, 18),
        rail: c(10, 14, 24),
        panel: c(16, 21, 34),
        field: c(23, 29, 46),
        hover: c(31, 39, 60),
        selected: c(30, 50, 98),
        track: c(50, 60, 86),
        knob: c(244, 247, 252),
        popup: c(20, 26, 42),
        edge: Color::rgba(150, 175, 230, 0.12),
        shade: Color::rgba(2, 4, 10, 0.72),
        shadow: Color::rgba(0, 2, 8, 0.5),
        lift: c(255, 255, 255),
        road: c(70, 78, 98),
        road_main: c(108, 118, 142),
        road_casing: Color::rgba(4, 6, 12, 0.9),
    },
    Palette {
        key: "ersatzverkehr",
        name: "Ersatzverkehr",
        accent: c(214, 40, 130),
        accent_2: c(247, 168, 0),
        on_accent: c(255, 255, 255),
        danger: c(255, 96, 82),
        ok: c(96, 204, 124),
        warn: c(247, 168, 0),
        text: c(246, 244, 246),
        text_soft: c(212, 206, 211),
        text_dim: c(158, 148, 155),
        text_faint: c(104, 96, 102),
        backdrop: c(8, 8, 9),
        rail: c(0, 0, 0),
        panel: c(19, 18, 20),
        field: c(29, 27, 30),
        hover: c(42, 38, 42),
        selected: c(78, 22, 52),
        track: c(66, 60, 66),
        knob: c(250, 248, 250),
        popup: c(25, 23, 26),
        edge: Color::rgba(255, 210, 235, 0.11),
        shade: Color::rgba(0, 0, 0, 0.72),
        shadow: Color::rgba(0, 0, 0, 0.55),
        lift: c(255, 255, 255),
        road: c(84, 78, 84),
        road_main: c(124, 114, 122),
        road_casing: Color::rgba(0, 0, 0, 0.9),
    },
    Palette {
        key: "classic",
        name: "Classic",
        accent: c(232, 160, 48),
        accent_2: c(96, 160, 232),
        on_accent: c(18, 14, 8),
        danger: c(222, 78, 68),
        ok: c(104, 190, 118),
        warn: c(232, 170, 70),
        text: c(236, 236, 236),
        text_soft: c(200, 200, 200),
        text_dim: c(142, 142, 142),
        text_faint: c(96, 96, 96),
        backdrop: c(18, 18, 18),
        rail: c(18, 18, 18),
        panel: c(22, 22, 22),
        field: c(31, 31, 31),
        hover: c(38, 38, 38),
        selected: c(44, 44, 44),
        track: c(62, 62, 62),
        knob: c(240, 240, 240),
        popup: c(28, 28, 28),
        edge: Color::rgba(255, 255, 255, 0.06),
        shade: Color::rgba(0, 0, 0, 0.62),
        shadow: Color::rgba(0, 0, 0, 0.4),
        lift: c(255, 255, 255),
        road: c(92, 92, 92),
        road_main: c(112, 112, 112),
        road_casing: Color::rgba(30, 30, 30, 0.9),
    },
];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// Which of `THEMES` is worn.
pub fn current() -> usize {
    CURRENT.load(Ordering::Relaxed).min(THEMES.len() - 1)
}

/// Wear the look of this key; one nobody knows is the first.
pub fn set(key: &str) {
    let i = THEMES.iter().position(|t| t.key.eq_ignore_ascii_case(key.trim())).unwrap_or(0);
    CURRENT.store(i, Ordering::Relaxed);
}

fn p() -> &'static Palette {
    &THEMES[current()]
}

macro_rules! colors {
    ($($name:ident => $field:ident),* $(,)?) => {
        $(pub fn $name() -> Color { p().$field })*
    };
}

colors! {
    ACCENT => accent, ACCENT_2 => accent_2, ON_ACCENT => on_accent, DANGER => danger, OK => ok, WARN => warn,
    TEXT => text, TEXT_SOFT => text_soft, TEXT_DIM => text_dim, TEXT_FAINT => text_faint,
    BACKDROP => backdrop, RAIL => rail, PANEL => panel, FIELD => field, HOVER => hover, SELECTED => selected,
    TRACK => track, KNOB => knob, POPUP => popup, EDGE => edge, SHADE => shade, SHADOW => shadow, LIFT => lift,
    ROAD => road, ROAD_MAIN => road_main, ROAD_CASING => road_casing,
}

/// Who these looks, the Home page and the workshop's pages were designed by (the mark on
/// the Home page and in the intro).
pub const DESIGNER: &str = "SchrimpLeiche81";

/// How far something has come in (0 → 1, eased out) `t` seconds after its page opened, when
/// it starts `delay` seconds in and takes `dur`.
pub fn appear(t: f32, delay: f32, dur: f32) -> f32 {
    let x = ((t - delay) / dur.max(1e-3)).clamp(0.0, 1.0);
    1.0 - (1.0 - x).powi(3)
}

pub const RADIUS: f32 = 10.0;
pub const CTRL: f32 = 6.0;
/// Height of a control row.
pub const ROW: f32 = 36.0;
pub const GAP: f32 = 12.0;

/// What the window is cleared to.
pub fn backdrop() -> wgpu::Color {
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) } as f64;
    let [r, g, b, _] = BACKDROP().0;
    wgpu::Color { r: lin(r), g: lin(g), b: lin(b), a: 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn things_come_in_eased_and_stay() {
        assert_eq!(appear(0.0, 0.2, 0.5), 0.0);
        assert!(appear(0.45, 0.2, 0.5) > 0.5);
        assert_eq!(appear(3.0, 0.2, 0.5), 1.0);
    }

    #[test]
    fn a_look_nobody_knows_is_the_first() {
        set("Ersatzverkehr");
        assert_eq!(THEMES[current()].key, "ersatzverkehr");
        set("no such look");
        assert_eq!(current(), 0);
    }

    #[test]
    fn text_reads_on_every_surface_of_every_look() {
        let lum = |c: Color| {
            let l = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
            0.2126 * l(c.0[0]) + 0.7152 * l(c.0[1]) + 0.0722 * l(c.0[2])
        };
        let contrast = |a: Color, b: Color| {
            let (x, y) = (lum(a), lum(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        };
        for t in &THEMES {
            for surface in [t.backdrop, t.rail, t.panel, t.field, t.popup] {
                assert!(contrast(t.text, surface) >= 7.0, "{}: text", t.key);
                assert!(contrast(t.text_dim, surface) >= 4.0, "{}: dim text", t.key);
            }
            assert!(contrast(t.on_accent, t.accent) >= 4.5, "{}: text on the accent", t.key);
        }
    }
}
