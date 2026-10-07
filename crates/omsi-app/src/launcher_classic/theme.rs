//! The launcher's look: flat and dark (neutral greys, no tint), one accent used sparingly -
//! after the calm menus of Euro Truck Simulator 2.
//!
//! (classic copy: the one deliberate difference from openOMSI's own launcher - the accent is
//! the player's choice, `crate::accent`, where openOMSI's was amber `rgba(232, 160, 48)`; dark
//! ink on a light accent, white on a dark one)

use omsi_ui::Color;

pub fn accent() -> Color {
    crate::accent::base()
}
/// The accent pressed, and where it has to be darker (the new launcher's `route_deep`).
pub fn accent_deep() -> Color {
    crate::accent::shades().deep
}
/// Text and marks on the accent.
pub fn on_accent() -> Color {
    crate::accent::shades().on
}
pub fn accent_2() -> Color {
    crate::accent::shades().soft
}
pub const DANGER: Color = Color::rgba(222, 78, 68, 1.0);
pub const OK: Color = Color::rgba(104, 190, 118, 1.0);
pub const WARN: Color = Color::rgba(232, 170, 70, 1.0);

pub const TEXT: Color = Color::rgba(236, 236, 236, 1.0);
pub const TEXT_SOFT: Color = Color::rgba(200, 200, 200, 1.0);
pub const TEXT_DIM: Color = Color::rgba(142, 142, 142, 1.0);
pub const TEXT_FAINT: Color = Color::rgba(96, 96, 96, 1.0);

/// Rail, panels, fields.
pub const RAIL: Color = Color::rgba(18, 18, 18, 1.0);
pub const PANEL: Color = Color::rgba(22, 22, 22, 1.0);
pub const FIELD: Color = Color::rgba(31, 31, 31, 1.0);
pub const HOVER: Color = Color::rgba(38, 38, 38, 1.0);
pub const SELECTED: Color = Color::rgba(44, 44, 44, 1.0);
pub const EDGE: Color = Color::rgba(255, 255, 255, 0.06);

pub const RADIUS: f32 = 8.0;
/// Height of a control row.
pub const ROW: f32 = 36.0;
pub const GAP: f32 = 12.0;
