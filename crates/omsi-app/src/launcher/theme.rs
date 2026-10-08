//! The launcher's look: the blue of a station sign, the yellow of a bus stop.

use omsi_ui::Color;

pub const ACCENT: Color = Color::rgba(255, 213, 0, 1.0);
pub const ACCENT_2: Color = Color::rgba(122, 214, 236, 1.0);
pub const ON_ACCENT: Color = Color::rgba(9, 26, 46, 1.0);
pub const DANGER: Color = Color::rgba(255, 112, 99, 1.0);
pub const OK: Color = Color::rgba(98, 210, 136, 1.0);
pub const WARN: Color = Color::rgba(255, 158, 66, 1.0);

pub const TEXT: Color = Color::rgba(244, 247, 251, 1.0);
pub const TEXT_SOFT: Color = Color::rgba(206, 218, 232, 1.0);
pub const TEXT_DIM: Color = Color::rgba(146, 168, 194, 1.0);
pub const TEXT_FAINT: Color = Color::rgba(98, 124, 154, 1.0);

pub const BACKDROP: Color = Color::rgba(14, 37, 64, 1.0);
pub const RAIL: Color = Color::rgba(10, 27, 48, 1.0);
pub const PANEL: Color = Color::rgba(18, 46, 77, 1.0);
pub const FIELD: Color = Color::rgba(25, 58, 94, 1.0);
pub const HOVER: Color = Color::rgba(33, 72, 113, 1.0);
pub const SELECTED: Color = Color::rgba(41, 86, 133, 1.0);
pub const TRACK: Color = Color::rgba(52, 88, 128, 1.0);
pub const KNOB: Color = Color::rgba(246, 249, 252, 1.0);
pub const POPUP: Color = Color::rgba(22, 54, 89, 1.0);
pub const EDGE: Color = Color::rgba(150, 195, 255, 0.14);
pub const SHADE: Color = Color::rgba(4, 13, 25, 0.7);
pub const SHADOW: Color = Color::rgba(2, 9, 18, 0.45);

pub const RADIUS: f32 = 10.0;
pub const CTRL: f32 = 6.0;
/// Height of a control row.
pub const ROW: f32 = 36.0;
pub const GAP: f32 = 12.0;

/// What the window is cleared to.
pub fn backdrop() -> wgpu::Color {
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) } as f64;
    let [r, g, b, _] = BACKDROP.0;
    wgpu::Color { r: lin(r), g: lin(g), b: lin(b), a: 1.0 }
}
