//! The launcher's look, after Omsi-Hub's: the map is the ground - night blue, edge to edge -
//! and the choices float on it as dark sheets. Colour has three jobs and no others: the
//! accent (the player's choice, the logo's orange unless they chose another - see
//! `crate::accent`) is the route and the one action that goes on, yellow is the line number
//! (a printed plate), and red, green and blue are punctuality. Everything else is ground,
//! sheet and ink.

use omsi_ui::Color;

/// The accent: the route on the map, the step you are on, the chosen row and the main action.
pub fn accent() -> Color {
    crate::accent::base()
}
/// The main action under the mouse; nowhere else.
pub fn accent_deep() -> Color {
    crate::accent::shades().deep
}
/// The line number's plate, and its ink.
pub const LINE: Color = Color::rgba(255, 210, 63, 1.0);
pub const ON_LINE: Color = Color::rgba(26, 20, 0, 1.0);
/// Punctuality: late, on time, early (blue, whatever the accent: it is a meaning).
pub const LATE: Color = Color::rgba(217, 58, 48, 1.0);
pub const ON_TIME: Color = Color::rgba(26, 138, 79, 1.0);
pub const EARLY: Color = Color::rgba(42, 117, 247, 1.0);
/// Early as text on a dark sheet.
pub const EARLY_SOFT: Color = Color::rgba(122, 168, 255, 1.0);

/// Text on the accent: white, or dark ink on a light one.
pub fn on_accent() -> Color {
    crate::accent::shades().on
}
/// The accent as text or a mark on a dark sheet: lighter.
pub fn accent_2() -> Color {
    crate::accent::shades().soft
}
pub const DANGER: Color = LATE;
/// (lighter than `ON_TIME`: it is read as text on a dark sheet)
pub const OK: Color = Color::rgba(52, 178, 110, 1.0);
pub const WARN: Color = Color::rgba(232, 170, 70, 1.0);

/// The sheet's ink, and its one quiet grade (with a softer and a fainter step between).
pub const TEXT: Color = Color::rgba(232, 235, 242, 1.0);
pub const TEXT_SOFT: Color = Color::rgba(196, 202, 214, 1.0);
pub const TEXT_DIM: Color = Color::rgba(149, 157, 176, 1.0);
pub const TEXT_FAINT: Color = Color::rgba(96, 104, 122, 1.0);

/// The ground (the map when there is none to show), the sheets on it, and the fields in them.
pub const GROUND: Color = Color::rgba(9, 12, 24, 1.0);
pub const RAIL: Color = GROUND;
pub const PANEL: Color = Color::rgba(20, 26, 38, 1.0);
pub const FIELD: Color = Color::rgba(27, 34, 49, 1.0);
pub const HOVER: Color = Color::rgba(33, 40, 54, 1.0);
/// A sheet's edge and the hairlines inside it.
pub const EDGE: Color = Color::rgba(255, 255, 255, 0.08);
pub const HAIRLINE: Color = Color::rgba(255, 255, 255, 0.09);
/// What lies on the map without being a sheet (its buttons): dark, a little see-through.
pub const ON_MAP: Color = Color::rgba(18, 26, 38, 0.86);

/// A control inside a sheet; a sheet; the main action.
pub const RADIUS: f32 = 10.0;
pub const SHEET_RADIUS: f32 = 14.0;
pub const ACTION_RADIUS: f32 = 12.0;
/// Height of a control row.
pub const ROW: f32 = 36.0;
pub const GAP: f32 = 12.0;
