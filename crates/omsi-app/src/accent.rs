//! The interface's accent: the one colour the player chooses (the launcher's Settings and the
//! palette in its top bar), the orange of the "OMSI" in openOMSI's logo unless they chose
//! another. Everything that was the route's blue - the main action, the chosen row, the step
//! one is on, focus, switches and sliders - asks here, so a new choice recolours the whole
//! interface at once. The game reads the setting (`accent=#rrggbb`) when it starts.
//!
//! What a colour means stays as it is: green is on time and done, red late and danger, amber a
//! warning, yellow the line's plate.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use omsi_ui::Color;

/// The logo's orange (`assets/logos/omsi-orange.svg`).
pub const DEFAULT: u32 = 0xF58620;

/// The choices offered: a name (an English key, translated where shown) and the colour.
pub const PRESETS: [(&str, u32); 8] = [
    ("Orange", DEFAULT),
    // (Omsi-Hub's own: the launcher as it looked before)
    ("Blue", 0x2A75F7),
    ("Green", 0x23A65A),
    ("Red", 0xE43334),
    ("Purple", 0x8B5CF6),
    ("Teal", 0x14C6BC),
    ("Pink", 0xE8459A),
    ("Yellow", 0xF6C90D),
];

/// The ground the interface lies on (the launcher's `GROUND`): an accent darker than this
/// allows is lifted until it reads on it.
const GROUND: [u8; 3] = [9, 12, 24];
/// How far an accent stands off the ground at least (WCAG contrast; 3 is what a control's
/// outline or a large word needs).
const MIN_ON_GROUND: f32 = 3.0;
/// Text on the accent: white while it reads at this contrast, else dark ink.
const WHITE_ON: f32 = 3.0;
/// The dark ink on a light accent (warm, near-black; as openOMSI's amber had).
const INK: [u8; 3] = [24, 16, 6];

/// The colour chosen (as chosen) and the one the interface uses (lifted where too dark), as
/// `f` gets them. (The tests' own per test: they run side by side, and one choosing a colour
/// would change what another one draws.)
fn cells<R>(f: impl FnOnce(&AtomicU32, &AtomicU32) -> R) -> R {
    #[cfg(not(test))]
    {
        static CHOSEN: AtomicU32 = AtomicU32::new(DEFAULT);
        static BASE: AtomicU32 = AtomicU32::new(DEFAULT);
        f(&CHOSEN, &BASE)
    }
    #[cfg(test)]
    {
        thread_local! {
            static CHOSEN: AtomicU32 = const { AtomicU32::new(DEFAULT) };
            static BASE: AtomicU32 = const { AtomicU32::new(DEFAULT) };
        }
        CHOSEN.with(|c| BASE.with(|b| f(c, b)))
    }
}

/// The setting "dark mode" (Luc): the launcher's ground and glass dark - the night ground,
/// smoked glass under white words - rather than light. Off unless the player turns it on.
static DARK: AtomicBool = AtomicBool::new(false);

/// Whether the launcher is dark (the setting "dark mode").
pub fn dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

/// Turn the dark mode on or off, at once for everything drawn from now on.
pub fn set_dark(on: bool) {
    DARK.store(on, Ordering::Relaxed);
}

/// Make `rgb` (`0xRRGGBB`) the accent, at once for everything drawn from now on.
pub fn set(rgb: u32) {
    let rgb = rgb & 0xFF_FFFF;
    if chosen() == rgb {
        return;
    }
    let base = pack(readable(unpack(rgb)));
    cells(|c, b| {
        c.store(rgb, Ordering::Relaxed);
        b.store(base, Ordering::Relaxed);
    });
}

/// The accent from the settings' value (`#rrggbb`), the default when there is none or it
/// cannot be read.
pub fn set_from_setting(v: Option<&str>) {
    set(v.and_then(parse_hex).unwrap_or(DEFAULT));
}

/// The colour chosen, as chosen (`0xRRGGBB`).
pub fn chosen() -> u32 {
    cells(|c, _| c.load(Ordering::Relaxed))
}

/// The accent's shades now.
pub fn shades() -> Shades {
    Shades::of_base(unpack(cells(|_, b| b.load(Ordering::Relaxed))))
}

/// The accent itself: the route, the chosen row, the main action.
pub fn base() -> Color {
    let [r, g, b] = unpack(cells(|_, b| b.load(Ordering::Relaxed)));
    Color::rgba(r, g, b, 1.0)
}

/// The accent's shades, all from one colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shades {
    /// The accent (lifted where it was too dark for the ground).
    pub base: Color,
    /// Under the mouse: a little lighter.
    pub hover: Color,
    /// Pressed, and where it has to be darker.
    pub deep: Color,
    /// Lighter: the accent as text, an icon or a mark on a dark sheet.
    pub soft: Color,
    /// Text and marks on the accent.
    pub on: Color,
}

impl Shades {
    /// The shades of the colour `rgb` (`0xRRGGBB`) as chosen.
    pub fn of(rgb: u32) -> Shades {
        Shades::of_base(readable(unpack(rgb)))
    }

    fn of_base(c: [u8; 3]) -> Shades {
        let base = Color::rgba(c[0], c[1], c[2], 1.0);
        let on = if contrast(c, [255, 255, 255]) >= WHITE_ON { Color::WHITE } else { Color::rgba(INK[0], INK[1], INK[2], 1.0) };
        Shades { base, hover: base.lighten(0.14), deep: base.darken(0.17), soft: base.lighten(0.38), on }
    }
}

/// `c` lifted towards white until it stands off the ground (unchanged when it does).
pub fn readable(c: [u8; 3]) -> [u8; 3] {
    let mut out = c;
    let mut t = 0.0f32;
    while contrast(out, GROUND) < MIN_ON_GROUND && t < 1.0 {
        t += 0.02;
        out = std::array::from_fn(|i| (c[i] as f32 + (255.0 - c[i] as f32) * t).round() as u8);
    }
    out
}

/// WCAG relative luminance of an sRGB colour.
pub fn luminance(c: [u8; 3]) -> f32 {
    let lin = |v: u8| {
        let s = v as f32 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// WCAG contrast ratio between two colours (1 to 21).
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// `#rrggbb`, `rrggbb` or `#rgb` → `0xRRGGBB`.
pub fn parse_hex(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match h.len() {
        6 => u32::from_str_radix(h, 16).ok(),
        3 => {
            let v = u32::from_str_radix(h, 16).ok()?;
            let (r, g, b) = ((v >> 8) & 15, (v >> 4) & 15, v & 15);
            Some((r * 17) << 16 | (g * 17) << 8 | b * 17)
        }
        _ => None,
    }
}

/// `0xRRGGBB` → `#rrggbb`.
pub fn hex(rgb: u32) -> String {
    format!("#{:06x}", rgb & 0xFF_FFFF)
}

pub fn unpack(rgb: u32) -> [u8; 3] {
    [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]
}

pub fn pack(c: [u8; 3]) -> u32 {
    (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32
}

/// The accent for the phone and tablet page (its CSS variables, set by `app.js`): the colour,
/// pressed, its "r, g, b" for see-through tints, the ink on it and the map route's casing.
pub fn css() -> serde_json::Value {
    css_of(shades())
}

fn css_of(s: Shades) -> serde_json::Value {
    let h = |c: Color| {
        let b = bytes(c, 255);
        format!("#{:02x}{:02x}{:02x}", b[0], b[1], b[2])
    };
    let b = bytes(s.base, 255);
    serde_json::json!({
        "base": h(s.base),
        "deep": h(s.deep),
        "rgb": format!("{}, {}, {}", b[0], b[1], b[2]),
        "on": h(s.on),
        "casing": h(s.base.darken(0.56)),
    })
}

/// A colour as the bytes the game's overlays take (`[r, g, b, a]`, a in 0..255).
pub fn bytes(c: Color, a: u8) -> [u8; 4] {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [b(c.0[0]), b(c.0[1]), b(c.0[2]), a]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(c: Color) -> [u8; 3] {
        let b = bytes(c, 255);
        [b[0], b[1], b[2]]
    }

    #[test]
    fn hex_codes_are_read_and_written() {
        assert_eq!(parse_hex("#F58620"), Some(0xF58620));
        assert_eq!(parse_hex(" f58620 "), Some(0xF58620));
        assert_eq!(parse_hex("#fa0"), Some(0xFFAA00));
        assert_eq!(parse_hex("#f5862"), None);
        assert_eq!(parse_hex("orange"), None);
        assert_eq!(parse_hex("#+12345"), None);
        assert_eq!(hex(0xF58620), "#f58620");
        assert_eq!(parse_hex(&hex(0x0A0B0C)), Some(0x0A0B0C));
    }

    #[test]
    fn the_default_is_the_logos_orange_with_dark_ink_on_it() {
        let s = Shades::of(DEFAULT);
        assert_eq!(rgb(s.base), [0xF5, 0x86, 0x20]);
        // (white on the orange is too faint to read: the ink is dark)
        assert!(contrast([0xF5, 0x86, 0x20], [255, 255, 255]) < WHITE_ON);
        assert_eq!(rgb(s.on), INK);
    }

    #[test]
    fn omsi_hubs_blue_keeps_white_on_it() {
        let s = Shades::of(0x2A75F7);
        assert_eq!(rgb(s.base), [42, 117, 247]);
        assert_eq!(s.on, Color::WHITE);
    }

    #[test]
    fn the_shades_go_lighter_and_darker_in_order() {
        for (_, c) in PRESETS {
            let s = Shades::of(c);
            let l = |x: Color| luminance(rgb(x));
            assert!(l(s.deep) < l(s.base) && l(s.base) < l(s.hover) && l(s.hover) < l(s.soft), "{}", hex(c));
            // (the text on it reads, whichever ink it is)
            assert!(contrast(rgb(s.on), rgb(s.base)) >= 3.0, "{}", hex(c));
        }
    }

    #[test]
    fn a_colour_too_dark_for_the_ground_is_lifted() {
        let navy = [10, 20, 60];
        assert!(contrast(navy, GROUND) < MIN_ON_GROUND);
        let up = readable(navy);
        assert!(contrast(up, GROUND) >= MIN_ON_GROUND);
        // (lifted, not turned: still blue)
        assert!(up[2] > up[0] && up[2] > up[1]);
        // the presets read as they are
        for (_, c) in PRESETS {
            assert_eq!(readable(unpack(c)), unpack(c), "{}", hex(c));
        }
    }

    #[test]
    fn set_changes_what_everything_asks_for() {
        set(0x8B5CF6);
        assert_eq!(chosen(), 0x8B5CF6);
        assert_eq!(rgb(base()), [0x8B, 0x5C, 0xF6]);
        set_from_setting(Some("not a colour"));
        assert_eq!(chosen(), DEFAULT);
        set_from_setting(None);
        assert_eq!(rgb(base()), [0xF5, 0x86, 0x20]);
    }

    #[test]
    fn the_phone_page_gets_the_default_as_its_stylesheet_has_it() {
        // (assets/companion/style.css and map.js start in these, before the game's state)
        let v = css_of(Shades::of(DEFAULT));
        assert_eq!(v["base"], "#f58620");
        assert_eq!(v["deep"], "#cb6f1b");
        assert_eq!(v["rgb"], "245, 134, 32");
        assert_eq!(v["on"], "#181006");
        assert_eq!(v["casing"], "#6c3b0e");
    }

    #[test]
    fn the_setting_is_written_as_it_is_read() {
        for (_, c) in PRESETS {
            assert_eq!(parse_hex(&omsi_launcher_lib::accent_text(&hex(c))), Some(c));
        }
        assert_eq!(omsi_launcher_lib::accent_text("nonsense"), hex(DEFAULT));
    }
}
