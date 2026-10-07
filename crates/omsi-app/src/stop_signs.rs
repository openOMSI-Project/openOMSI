//! The bus stop signs on the maps: the navigator's small map and city map, and the launcher's
//! map of the duty. A stop is drawn as the sign the player looks for at the roadside, in the
//! style chosen on the launcher's Settings page (`stop_style`):
//!
//! * German: the Haltestelle sign (Zeichen 224) - a yellow disc, a green ring, a green H - on
//!   its white rim, as openOMSI's logo has it;
//! * British: London's bus stop flag - the roundel, a red ring on a white disc with the red
//!   bar across it, "BUS STOP" on the bar where there are pixels for the letters;
//! * French: the arrêt d'autobus (C6) - a white bus on a blue square with rounded corners.
//!
//! A sign is a list of flat convex shapes worked out for the size it is drawn at: the fine
//! parts (the white rim, the bus's mirrors and lamps, the bar's letters) only where there are
//! pixels for them, and no stroke thinner than a pixel - so that it reads from the small
//! navigator's corner to a 4K city map, and far out it is a dot in the sign's colours. The
//! same shapes are put on the screen (`draw`) or on a world point (`draw_world`: the
//! launcher's map, whose camera only moves a matrix), so the game and the launcher cannot draw
//! a stop differently.
//!
//! Where the bus is in the trip shows on the sign (`Kind`): the stop it heads for larger, on a
//! white edge with a soft light round it; a stop it has served smaller and faded into the map;
//! the terminus ringed in the line plate's yellow. The other public transport's line tags are
//! rounded chips in their kind's colour (the duty's own line on the plate's yellow), and the
//! stops' names wear the sign's colour as a tab.

use std::sync::atomic::{AtomicU8, Ordering};

use glam::{DVec3, Vec2, Vec3};
use hashbrown::HashMap;
use omsi_ui::paint::Align;
use omsi_ui::{Atlas, Color, Fonts, Painter, Rect, Weight};

use crate::schedule::{PlannedTrip, PlayerDuty};

// --- colours (sRGB) -------------------------------------------------------------------

/// German: the logo's Haltestelle (`launcher::flow`'s brand colours) on its white rim.
const DE_RIM: Color = Color::hex(0xFFFFFF);
const DE_YELLOW: Color = Color::hex(0xF6D60D);
const DE_GREEN: Color = Color::hex(0x0F7E3A);
/// British: the logo's red stop - the roundel's red on white.
const UK_RED: Color = Color::hex(0xE43334);
const UK_WHITE: Color = Color::hex(0xFBFBF8);
/// French: the C6 sign's blue (a shade brighter than the roadside's: the map is dark) on its
/// white border.
const FR_BLUE: Color = Color::hex(0x1F5FAE);
const FR_WHITE: Color = Color::hex(0xFFFFFF);
/// What every sign stands on: a dark edge that parts it from the roads and the route.
const EDGE: Color = Color::rgba(8, 11, 18, 0.92);
/// The next stop's soft light and its white edge (a cool white, the signs' own is warmer).
const GLOW: Color = Color::rgba(255, 255, 255, 0.09);
const HIGHLIGHT: Color = Color::rgba(246, 250, 255, 1.0);
/// The terminus's ring: the line plate's yellow (as the launcher's intro and tour ring it).
const RING: Color = crate::nav_duty::LINE;
/// What a served stop fades into: the maps' night-blue ground.
const GROUND: Color = Color::rgba(18, 24, 36, 1.0);
/// The ink on the line plate's yellow, and on a light chip.
const ON_PLATE: Color = Color::rgba(26, 20, 0, 1.0);
const ON_DARK: Color = Color::rgba(255, 255, 255, 1.0);
/// A stop's label (the next stop's lifted a little) and its text.
const LABEL: Color = Color::rgba(12, 12, 12, 0.85);
const LABEL_NEXT: Color = Color::rgba(24, 31, 46, 0.94);
const LABEL_TEXT: Color = Color::rgba(235, 235, 235, 1.0);
const LABEL_DIM: Color = Color::rgba(178, 178, 178, 1.0);

// --- the style ------------------------------------------------------------------------

/// Which country's stop sign the maps draw (`stop_style`: "de", "uk", "fr").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Style {
    #[default]
    German,
    British,
    French,
}

impl Style {
    /// The style a setting names (anything unknown is the German sign, the default).
    pub fn from_setting(v: &str) -> Style {
        match omsi_launcher_lib::stop_style(v) {
            "uk" => Style::British,
            "fr" => Style::French,
            _ => Style::German,
        }
    }

    /// The setting's value for it.
    #[cfg(test)]
    fn key(self) -> &'static str {
        match self {
            Style::German => "de",
            Style::British => "uk",
            Style::French => "fr",
        }
    }

    /// The sign's own colour, for what belongs to it on the dark map (a label's tab) - the
    /// French blue lighter there, or it would not show.
    pub fn accent(self) -> Color {
        match self {
            Style::German => DE_YELLOW,
            Style::British => UK_RED,
            Style::French => Color::hex(0x4A8BE0),
        }
    }

    /// How large the sign is for the same size asked: a square and a disc with a bar across
    /// look larger than a disc, so they are drawn a little smaller to weigh the same.
    fn weight(self) -> f32 {
        match self {
            Style::German => 1.0,
            Style::British => 0.94,
            Style::French => 0.88,
        }
    }
}

/// The style the game's maps draw (set from `Settings::stop_style` when the navigator is made;
/// one game, one setting - the launcher passes its own).
static STYLE: AtomicU8 = AtomicU8::new(Style::German as u8);

pub fn set_style(style: Style) {
    STYLE.store(style as u8, Ordering::Relaxed);
}

pub fn style() -> Style {
    match STYLE.load(Ordering::Relaxed) {
        1 => Style::British,
        2 => Style::French,
        _ => Style::German,
    }
}

// --- the kind of stop -----------------------------------------------------------------

/// Where a stop is in the trip, as its sign shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A stop still to come.
    Ahead,
    /// The stop the bus heads for: larger, on a white edge, a soft light round it.
    Next,
    /// The trip's last stop: ringed in the line plate's yellow.
    Terminus,
    /// The next stop is the last: both.
    NextTerminus,
    /// A stop the bus has served: smaller, faded into the map.
    Passed,
}

impl Kind {
    /// The kind of the `k`th of the `n` stops ahead (0 = the next).
    pub fn ahead(k: usize, n: usize) -> Kind {
        match (k == 0, k + 1 == n) {
            (true, true) => Kind::NextTerminus,
            (true, false) => Kind::Next,
            (false, true) => Kind::Terminus,
            _ => Kind::Ahead,
        }
    }

    fn next(self) -> bool {
        matches!(self, Kind::Next | Kind::NextTerminus)
    }

    fn ringed(self) -> bool {
        matches!(self, Kind::Terminus | Kind::NextTerminus)
    }

    fn scale(self) -> f32 {
        match self {
            Kind::Ahead => 1.0,
            Kind::Next | Kind::NextTerminus => 1.28,
            Kind::Terminus => 1.1,
            Kind::Passed => 0.8,
        }
    }
}

// --- shapes ---------------------------------------------------------------------------

/// A flat convex shape of a sign, in pixels round the sign's middle (y down, as the screen).
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub pts: Vec<Vec2>,
    pub color: Color,
}

/// How much of a sign there are pixels for, by its radius: all of it, its bold strokes only,
/// or a dot in its colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lod {
    Dot,
    Small,
    Full,
}

impl Lod {
    fn of(r: f32) -> Lod {
        if r < 3.6 {
            Lod::Dot
        } else if r < 7.5 {
            Lod::Small
        } else {
            Lod::Full
        }
    }
}

/// Points round a circle (enough that its edge never shows a corner).
fn circle(c: Vec2, r: f32) -> Vec<Vec2> {
    let n = ((r * 1.1) as usize).clamp(14, 64);
    (0..n).map(|k| c + Vec2::from_angle(std::f32::consts::TAU * k as f32 / n as f32) * r).collect()
}

/// The band between two circles, as quads (each convex; the launcher's world shapes take
/// nothing else).
fn ring(out: &mut Vec<Shape>, c: Vec2, r0: f32, r1: f32, color: Color) {
    let n = ((r1 * 1.1) as usize).clamp(16, 64);
    let d = |k: usize| Vec2::from_angle(std::f32::consts::TAU * k as f32 / n as f32);
    for k in 0..n {
        let (a, b) = (d(k), d(k + 1));
        out.push(Shape { pts: vec![c + a * r0, c + a * r1, c + b * r1, c + b * r0], color });
    }
}

/// Points per corner of a rounded box: the same for every radius, so two outlines can be
/// joined point for point into a frame.
const CORNER: usize = 6;

/// A rounded box's outline, clockwise on the screen.
fn rounded(r: Rect, rad: f32) -> Vec<Vec2> {
    let rad = rad.min(r.w * 0.5).min(r.h * 0.5).max(0.0);
    let corners = [(Vec2::new(r.right() - rad, r.y + rad), -90.0f32), (Vec2::new(r.right() - rad, r.bottom() - rad), 0.0), (Vec2::new(r.x + rad, r.bottom() - rad), 90.0), (Vec2::new(r.x + rad, r.y + rad), 180.0)];
    let mut out = Vec::with_capacity(4 * (CORNER + 1));
    for (c, a0) in corners {
        for k in 0..=CORNER {
            out.push(c + Vec2::from_angle((a0 + 90.0 * k as f32 / CORNER as f32).to_radians()) * rad);
        }
    }
    out
}

/// A box `hw` by `hh` either side of `c`.
fn boxed(c: Vec2, hw: f32, hh: f32) -> Rect {
    Rect::new(c.x - hw, c.y - hh, hw * 2.0, hh * 2.0)
}

fn quad(r: Rect) -> Vec<Vec2> {
    vec![Vec2::new(r.x, r.y), Vec2::new(r.right(), r.y), Vec2::new(r.right(), r.bottom()), Vec2::new(r.x, r.bottom())]
}

/// The band between a rounded box and the same box `w` larger all round, as quads.
fn frame(out: &mut Vec<Shape>, r: Rect, rad: f32, w: f32, color: Color) {
    let inner = rounded(r, rad);
    let outer = rounded(r.inset(-w), rad + w);
    let n = inner.len();
    for k in 0..n {
        let j = (k + 1) % n;
        out.push(Shape { pts: vec![inner[k], outer[k], outer[j], inner[j]], color });
    }
}

/// The British sign's bar: how far it reaches out either side and how tall half of it is, in
/// the sign's radius (London's bar runs out past the ring).
const UK_BAR: (f32, f32) = (1.16, 0.2);
/// The French sign's corners, in its half width.
const FR_CORNER: f32 = 0.3;

/// The sign's outline `e` pixels out from its edge (its dark edge, the next stop's white one).
fn silhouette(style: Style, lod: Lod, r: f32, e: f32) -> Vec<Vec<Vec2>> {
    match style {
        Style::German => vec![circle(Vec2::ZERO, r + e)],
        Style::British if lod == Lod::Dot => vec![circle(Vec2::ZERO, r + e)],
        Style::British => vec![circle(Vec2::ZERO, r + e), quad(boxed(Vec2::ZERO, UK_BAR.0 * r + e, UK_BAR.1 * r + e))],
        Style::French => vec![rounded(boxed(Vec2::ZERO, r + e, r + e), FR_CORNER * r + e)],
    }
}

/// How far from its middle the sign reaches at its farthest (the terminus's ring clears it).
fn reach(style: Style, lod: Lod, r: f32) -> f32 {
    match style {
        Style::German => r,
        Style::British if lod == Lod::Dot => r,
        Style::British => (UK_BAR.0 * r).hypot(UK_BAR.1 * r).max(r),
        Style::French => (r - FR_CORNER * r) * std::f32::consts::SQRT_2 + FR_CORNER * r,
    }
}

/// The sign of `style` for a stop of `kind`, `size` pixels across where it is an ordinary stop
/// ahead (the kind and the style make it larger or smaller): every shape in pixels round its
/// middle, to be drawn in order.
pub fn sign(style: Style, kind: Kind, size: f32) -> Vec<Shape> {
    let r = (size * 0.5 * kind.scale() * style.weight()).max(1.0);
    let lod = Lod::of(r);
    // (the dark edge: a pixel at least, never heavy)
    let o = (r * 0.13).clamp(1.0, 2.2);
    // (the next stop's white edge lies on the sign itself, its dark edge outside it: a dark
    // line between them made the white rims of the signs a target's rings)
    let white = if kind.next() { (r * 0.16).clamp(1.4, 2.6) } else { 0.0 };
    let edge = o + white;
    let mut out = Vec::new();
    if kind.next() {
        // a soft light round it (two steps of it: one edge would be a disc of its own)
        let glow = reach(style, lod, r) + edge;
        let reach_out = (r * 0.5).max(3.0);
        out.push(Shape { pts: circle(Vec2::ZERO, glow + reach_out), color: GLOW });
        out.push(Shape { pts: circle(Vec2::ZERO, glow + reach_out * 0.5), color: GLOW });
    }
    if kind.ringed() {
        let gap = (r * 0.25).max(1.5);
        let w = (r * 0.22).max(1.6);
        if style == Style::French {
            // (round a square sign a rounded square: a circle would stand far off its sides)
            let at = r + edge + gap;
            frame(&mut out, boxed(Vec2::ZERO, at - 1.0, at - 1.0), FR_CORNER * r + edge + gap - 1.0, w + 2.0, EDGE);
            frame(&mut out, boxed(Vec2::ZERO, at, at), FR_CORNER * r + edge + gap, w, RING);
        } else {
            let at = reach(style, lod, r) + edge + gap;
            ring(&mut out, Vec2::ZERO, at - 1.0, at + w + 1.0, EDGE);
            ring(&mut out, Vec2::ZERO, at, at + w, RING);
        }
    }
    let first = out.len();
    for pts in silhouette(style, lod, r, edge) {
        out.push(Shape { pts, color: EDGE });
    }
    if kind.next() {
        for pts in silhouette(style, lod, r, white) {
            out.push(Shape { pts, color: HIGHLIGHT });
        }
    }
    match style {
        Style::German => german(&mut out, lod, r),
        Style::British => british(&mut out, lod, r),
        Style::French => french(&mut out, lod, r),
    }
    if kind == Kind::Passed {
        for s in &mut out[first..] {
            s.color = faded(s.color);
        }
    }
    out
}

/// A colour faded into the map: greyed and half sunk into the ground.
fn faded(c: Color) -> Color {
    let [r, g, b, a] = c.0;
    let l = 0.3 * r + 0.59 * g + 0.11 * b;
    let ground = Color([GROUND.0[0], GROUND.0[1], GROUND.0[2], a]);
    c.mix(Color([l, l, l, a]), 0.6).mix(ground, 0.42)
}

/// The Haltestelle sign: a yellow disc, a green ring and a green H (the logo's measures, as
/// shares of its white rim).
fn german(out: &mut Vec<Shape>, lod: Lod, r: f32) {
    let o = Vec2::ZERO;
    match lod {
        Lod::Dot => {
            out.push(Shape { pts: circle(o, r), color: DE_GREEN });
            out.push(Shape { pts: circle(o, r * 0.6), color: DE_YELLOW });
        }
        Lod::Small => {
            out.push(Shape { pts: circle(o, r), color: DE_YELLOW });
            let outer = r * 0.9;
            ring(out, o, outer - (r * 0.24).max(1.0), outer, DE_GREEN);
            h(out, r * 0.14, (r * 0.2).max(1.0), r * 0.4, (r * 0.075).max(0.5), DE_GREEN);
        }
        Lod::Full => {
            out.push(Shape { pts: circle(o, r), color: DE_RIM });
            out.push(Shape { pts: circle(o, r * 0.9), color: DE_YELLOW });
            ring(out, o, r * 0.68, r * 0.86, DE_GREEN);
            h(out, r * 0.17, r * 0.16, r * 0.43, r * 0.075, DE_GREEN);
        }
    }
}

/// An H round the middle: its bars `bw` wide from `inner` out, `hh` up and down, the bar
/// across `ch` up and down.
fn h(out: &mut Vec<Shape>, inner: f32, bw: f32, hh: f32, ch: f32, color: Color) {
    for side in [-1.0, 1.0] {
        let x = side * (inner + bw * 0.5);
        out.push(Shape { pts: quad(boxed(Vec2::new(x, 0.0), bw * 0.5, hh)), color });
    }
    // (into the bars a little: no hairline between them)
    out.push(Shape { pts: quad(boxed(Vec2::ZERO, inner + bw * 0.25, ch)), color });
}

/// London's bus stop flag: the roundel - a red ring on a white disc, the red bar across it
/// and out past the ring, "BUS STOP" on the bar when there are pixels for it.
fn british(out: &mut Vec<Shape>, lod: Lod, r: f32) {
    let o = Vec2::ZERO;
    match lod {
        Lod::Dot => {
            out.push(Shape { pts: circle(o, r), color: UK_RED });
            out.push(Shape { pts: circle(o, r * 0.55), color: UK_WHITE });
        }
        Lod::Small => {
            out.push(Shape { pts: circle(o, r), color: UK_WHITE });
            let outer = r * 0.9;
            ring(out, o, outer - (r * 0.36).max(1.2), outer, UK_RED);
            out.push(Shape { pts: quad(boxed(o, UK_BAR.0 * r, (UK_BAR.1 * r).max(0.75))), color: UK_RED });
        }
        Lod::Full => {
            out.push(Shape { pts: circle(o, r), color: UK_WHITE });
            ring(out, o, r * 0.58, r * 0.86, UK_RED);
            out.push(Shape { pts: quad(boxed(o, UK_BAR.0 * r, UK_BAR.1 * r)), color: UK_RED });
            // (a letter of five rows wants a pixel a row)
            let cell = r * 0.054;
            if cell >= 0.8 {
                letters(out, "BUS STOP", o, cell, UK_WHITE);
            }
        }
    }
}

/// The French sign C6: a white bus on blue, in a rounded square with a white border.
fn french(out: &mut Vec<Shape>, lod: Lod, r: f32) {
    let o = Vec2::ZERO;
    match lod {
        Lod::Dot => {
            out.push(Shape { pts: rounded(boxed(o, r, r), r * 0.35), color: FR_WHITE });
            out.push(Shape { pts: rounded(boxed(o, r * 0.6, r * 0.6), r * 0.22), color: FR_BLUE });
        }
        Lod::Small => {
            out.push(Shape { pts: rounded(boxed(o, r, r), FR_CORNER * r), color: FR_WHITE });
            let b = r - (r * 0.1).max(0.9);
            out.push(Shape { pts: rounded(boxed(o, b, b), FR_CORNER * b), color: FR_BLUE });
            bus(out, o, b * 0.66, FR_WHITE, FR_BLUE, false);
        }
        Lod::Full => {
            out.push(Shape { pts: rounded(boxed(o, r, r), FR_CORNER * r), color: FR_WHITE });
            out.push(Shape { pts: rounded(boxed(o, r * 0.88, r * 0.88), r * 0.24), color: FR_BLUE });
            // (its mirrors and lamps once it is a dozen pixels tall)
            bus(out, o, r * 0.56, FR_WHITE, FR_BLUE, r * 0.56 >= 6.0);
        }
    }
}

/// The logo's bus, front on (`assets/logos`): `k` pixels a unit, 2 units tall round `c`, in
/// `ink` with its windows cut out in `hole`; `detail` adds the mirrors, the destination
/// display and the lamps.
fn bus(out: &mut Vec<Shape>, c: Vec2, k: f32, ink: Color, hole: Color, detail: bool) {
    let b = |x0: f32, y0: f32, x1: f32, y1: f32, rad: f32| rounded(Rect::new(c.x + x0 * k, c.y + y0 * k, (x1 - x0) * k, (y1 - y0) * k), rad * k);
    if detail {
        for s in [-1.0, 1.0] {
            let (x0, x1) = if s < 0.0 { (-1.11, -0.96) } else { (0.96, 1.11) };
            out.push(Shape { pts: b(x0, -0.57, x1, -0.13, 0.07), color: ink });
        }
    }
    out.push(Shape { pts: b(-0.89, -1.0, 0.89, 0.76, 0.2), color: ink });
    out.push(Shape { pts: b(-0.77, 0.6, -0.49, 1.0, 0.08), color: ink });
    out.push(Shape { pts: b(0.49, 0.6, 0.77, 1.0, 0.08), color: ink });
    if detail {
        out.push(Shape { pts: b(-0.43, -0.88, 0.43, -0.73, 0.07), color: hole });
        out.push(Shape { pts: b(-0.74, -0.61, 0.74, 0.22, 0.12), color: hole });
        for s in [-1.0, 1.0] {
            out.push(Shape { pts: circle(c + Vec2::new(s * 0.6, 0.49) * k, 0.12 * k), color: hole });
        }
    } else {
        out.push(Shape { pts: b(-0.7, -0.72, 0.7, 0.18, 0.1), color: hole });
    }
}

/// The few letters a sign carries, three cells by five (a row a bit, the left column high).
const GLYPHS: [(char, [u8; 5]); 6] = [
    ('B', [0b110, 0b101, 0b110, 0b101, 0b110]),
    ('U', [0b101, 0b101, 0b101, 0b101, 0b111]),
    ('S', [0b111, 0b100, 0b111, 0b001, 0b111]),
    ('T', [0b111, 0b010, 0b010, 0b010, 0b010]),
    ('O', [0b111, 0b101, 0b101, 0b101, 0b111]),
    ('P', [0b111, 0b101, 0b111, 0b100, 0b100]),
];

/// How many cells `text` takes across: a glyph three and a cell after it, a space two.
fn letters_width(text: &str) -> usize {
    text.chars().map(|ch| if ch == ' ' { 2 } else { 4 }).sum::<usize>().saturating_sub(1)
}

/// `text` in block letters `cell` pixels a cell, centred on `c` (each row's run of cells one
/// box).
fn letters(out: &mut Vec<Shape>, text: &str, c: Vec2, cell: f32, color: Color) {
    let mut x = c.x - letters_width(text) as f32 * cell * 0.5;
    let top = c.y - 2.5 * cell;
    for ch in text.chars() {
        if ch == ' ' {
            x += 2.0 * cell;
            continue;
        }
        if let Some((_, rows)) = GLYPHS.iter().find(|g| g.0 == ch) {
            for (j, &bits) in rows.iter().enumerate() {
                let on = |i: usize| (bits >> (2 - i)) & 1 == 1;
                let mut i = 0;
                while i < 3 {
                    if !on(i) {
                        i += 1;
                        continue;
                    }
                    let start = i;
                    while i < 3 && on(i) {
                        i += 1;
                    }
                    out.push(Shape { pts: quad(Rect::new(x + start as f32 * cell, top + j as f32 * cell, (i - start) as f32 * cell, cell)), color });
                }
            }
        }
        x += 4.0 * cell;
    }
}

/// How far from its middle a sign reaches with everything round it (its light, its ring).
#[cfg(test)]
fn extent(style: Style, kind: Kind, size: f32) -> f32 {
    sign(style, kind, size).iter().flat_map(|s| s.pts.iter()).map(|p| p.length()).fold(0.0, f32::max)
}

/// A sign on the screen, its middle at `at` (pixels).
pub fn draw(p: &mut Painter, style: Style, kind: Kind, at: Vec2, size: f32) {
    for s in sign(style, kind, size) {
        let pts: Vec<Vec2> = s.pts.iter().map(|q| at + *q).collect();
        p.convex(&pts, s.color);
    }
}

/// A sign standing on a world point, sized in pixels whatever the zoom (the launcher's map:
/// north is up there, so the shapes are turned the screen's way up).
pub fn draw_world(p: &mut Painter, style: Style, kind: Kind, at: Vec3, size: f32) {
    for s in sign(style, kind, size) {
        let pts: Vec<Vec2> = s.pts.iter().map(|q| Vec2::new(q.x, -q.y)).collect();
        p.world_shape(at, &pts, 0.0, 1.0, s.color);
    }
}

// --- the stops served -----------------------------------------------------------------

/// Where the stops the bus has served on the trip under way stand, the latest last: the
/// timetable's place, else the map's (`places`: a stop beyond the loaded tiles).
pub fn served(duty: Option<&PlayerDuty>, places: &HashMap<i64, DVec3>) -> Vec<DVec3> {
    let Some(d) = duty else { return Vec::new() };
    let Some(trip) = d.trips.get(d.trip_index) else { return Vec::new() };
    served_of(trip, d.next_stop, places)
}

fn served_of(trip: &PlannedTrip, next: usize, places: &HashMap<i64, DVec3>) -> Vec<DVec3> {
    trip.stops
        .iter()
        .take(next.min(trip.stops.len()))
        .filter(|s| s.stops)
        .filter_map(|s| s.position.filter(|p| *p != DVec3::ZERO).or_else(|| places.get(&s.object_id).copied()))
        .collect()
}

// --- line tags and stop names ---------------------------------------------------------

/// WCAG's relative luminance of an sRGB colour.
fn luminance(c: Color) -> f32 {
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c.0[0]) + 0.7152 * lin(c.0[1]) + 0.0722 * lin(c.0[2])
}

/// The colours of a line's tag on the map (fill, ink): the duty's own line on the line plate's
/// yellow, as the duty board prints it; any other in its kind's colour (the bus's, the
/// trolleybus's, the tram's dot) with the ink that reads best on it.
fn chip_colours(kind: Color, own: bool) -> (Color, Color) {
    if own {
        return (crate::nav_duty::LINE, ON_PLATE);
    }
    // (the luminance where black and white read alike on it is 0.18; white is London's red's)
    (kind, if luminance(kind) > 0.22 { ON_PLATE } else { ON_DARK })
}

/// A line's tag over a vehicle on the map, in `r` (its pointer reaches down to the vehicle's
/// dot under the middle of `r`): another line a rounded chip in its kind's colour `kind`, the
/// duty's own line (`own`) the line plate the duty board prints - square-cornered and yellow,
/// so that it stands apart from a tram's yellow chip.
#[allow(clippy::too_many_arguments)]
pub fn draw_chip(p: &mut Painter, atlas: &mut Atlas, fonts: &Fonts, r: Rect, text: &str, px: f32, kind: Color, own: bool, s: f32) {
    let (fill, ink) = chip_colours(kind, own);
    let rad = if own { 3.5 * s } else { r.h * 0.5 };
    let (cx, b) = (r.center().x, r.bottom());
    let tip = 3.5 * s;
    p.convex(&[Vec2::new(cx - 4.6 * s, b - 1.0), Vec2::new(cx + 4.6 * s, b - 1.0), Vec2::new(cx, b + tip + 1.4 * s)], EDGE);
    p.rounded(r.inset(-1.0 * s), rad + 1.0 * s, EDGE);
    p.convex(&[Vec2::new(cx - 3.2 * s, b - 1.0), Vec2::new(cx + 3.2 * s, b - 1.0), Vec2::new(cx, b + tip)], fill.darken(0.08));
    p.rounded_gradient(r, rad, fill.lighten(0.1), fill.darken(0.06));
    p.text_in(atlas, fonts, text, px, if own { Weight::Black } else { Weight::Bold }, r, Align::Center, ink);
}

/// How much wider than its text a line's chip is (its round ends), at scale 1.
pub const CHIP_PAD: f32 = 10.0;

/// A stop's name on the map: on its dark label with the sign's colour as a tab at its left -
/// the next stop's lifted and bright, the others quieter. `r` leaves 6 pixels (at scale 1)
/// either side of the text, as the navigator measures it.
#[allow(clippy::too_many_arguments)]
pub fn draw_label(p: &mut Painter, atlas: &mut Atlas, fonts: &Fonts, r: Rect, text: &str, px: f32, style: Style, next: bool, s: f32) {
    p.rounded(r, 4.0 * s, if next { LABEL_NEXT } else { LABEL });
    let tab = Rect::new(r.x + 2.0 * s, r.y + 4.0 * s, 2.5 * s, (r.h - 8.0 * s).max(2.0));
    p.rounded(tab, 1.25 * s, if next { style.accent() } else { style.accent().alpha(0.55) });
    let weight = if next { Weight::Bold } else { Weight::Medium };
    p.text_in(atlas, fonts, text, px, weight, Rect::new(r.x + 7.5 * s, r.y, (r.w - 13.0 * s).max(0.0), r.h), Align::Left, if next { LABEL_TEXT } else { LABEL_DIM });
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Style {
        const ALL: [Style; 3] = [Style::German, Style::British, Style::French];
    }

    impl Kind {
        const ALL: [Kind; 5] = [Kind::Passed, Kind::Ahead, Kind::Next, Kind::Terminus, Kind::NextTerminus];
    }

    const SIZES: [f32; 6] = [5.0, 8.0, 12.0, 16.0, 26.0, 44.0];

    fn convex(pts: &[Vec2]) -> bool {
        // every turn the same way (or none: points on a line, a corner of no radius)
        let n = pts.len();
        let mut sign = 0.0f32;
        for k in 0..n {
            let (a, b, c) = (pts[k], pts[(k + 1) % n], pts[(k + 2) % n]);
            let t = (b - a).perp_dot(c - b);
            if t.abs() > 1e-3 {
                if sign != 0.0 && t.signum() != sign {
                    return false;
                }
                sign = t.signum();
            }
        }
        true
    }

    fn area(pts: &[Vec2]) -> f32 {
        (0..pts.len()).map(|k| pts[k].perp_dot(pts[(k + 1) % pts.len()])).sum::<f32>().abs() * 0.5
    }

    fn has(shapes: &[Shape], c: Color) -> bool {
        shapes.iter().any(|s| s.color == c)
    }

    /// Every shape of every sign is a convex polygon with an area (the world painter fans them
    /// out from their first point), at every size and of every kind.
    #[test]
    fn signs_are_convex_shapes() {
        for style in Style::ALL {
            for kind in Kind::ALL {
                for size in SIZES {
                    let shapes = sign(style, kind, size);
                    assert!(shapes.len() >= 3, "{style:?} {kind:?} {size}");
                    for s in &shapes {
                        assert!(s.pts.len() >= 3 && convex(&s.pts), "{style:?} {kind:?} {size}: {:?}", s.pts);
                        assert!(area(&s.pts) > 1e-4, "{style:?} {kind:?} {size}");
                        assert!(s.pts.iter().all(|p| p.is_finite()));
                    }
                }
            }
        }
    }

    /// Each style is its country's sign: its colours, and the parts there are pixels for.
    #[test]
    fn each_style_wears_its_colours() {
        for size in [12.0, 30.0] {
            let de = sign(Style::German, Kind::Ahead, size);
            assert!(has(&de, DE_YELLOW) && has(&de, DE_GREEN), "{size}");
            let uk = sign(Style::British, Kind::Ahead, size);
            assert!(has(&uk, UK_RED) && has(&uk, UK_WHITE), "{size}");
            let fr = sign(Style::French, Kind::Ahead, size);
            assert!(has(&fr, FR_BLUE) && has(&fr, FR_WHITE), "{size}");
            for s in [&de, &uk, &fr] {
                assert!(!has(s, RING) && !has(s, GLOW) && !has(s, HIGHLIGHT), "an ordinary stop is not dressed up");
            }
        }
        // the German sign gets its white rim, the bus its mirrors, the bar its letters only
        // where there are pixels for them
        assert!(!has(&sign(Style::German, Kind::Ahead, 10.0), DE_RIM) && has(&sign(Style::German, Kind::Ahead, 20.0), DE_RIM));
        assert!(sign(Style::French, Kind::Ahead, 12.0).len() < sign(Style::French, Kind::Ahead, 40.0).len());
        let bar = |size: f32| sign(Style::British, Kind::Ahead, size).iter().filter(|s| s.color == UK_WHITE).count();
        assert_eq!(bar(16.0), 1, "the disc alone");
        assert!(bar(40.0) > 10, "and the letters on the bar");
        // far out each is a dot in its colours
        assert!(has(&sign(Style::British, Kind::Ahead, 5.0), UK_RED) && has(&sign(Style::French, Kind::Ahead, 5.0), FR_BLUE));
    }

    /// The next stop is larger than one ahead, a served one smaller; the terminus wears the
    /// plate's ring clear of its sign; the next stop its white edge and light.
    #[test]
    fn the_kind_of_stop_shows() {
        for style in Style::ALL {
            for size in [8.0, 14.0, 30.0] {
                let e = |k| extent(style, k, size);
                assert!(e(Kind::Next) > e(Kind::Ahead) && e(Kind::Ahead) > e(Kind::Passed), "{style:?} {size}");
                assert!(e(Kind::Terminus) > e(Kind::Ahead), "{style:?} {size}");
                let next = sign(style, Kind::Next, size);
                assert!(has(&next, HIGHLIGHT) && has(&next, GLOW) && !has(&next, RING));
                let both = sign(style, Kind::NextTerminus, size);
                assert!(has(&both, HIGHLIGHT) && has(&both, RING));
                // the ring lies outside everything of the sign itself (its edges included)
                let term = sign(style, Kind::Terminus, size);
                let ring_in = term.iter().filter(|s| s.color == RING).flat_map(|s| s.pts.iter()).map(|p| p.length()).fold(f32::MAX, f32::min);
                let body_out = term.iter().filter(|s| s.color != RING && s.color != EDGE).flat_map(|s| s.pts.iter()).map(|p| p.length()).fold(0.0, f32::max);
                assert!(ring_in > body_out, "{style:?} {size}: ring at {ring_in}, sign to {body_out}");
                for k in [Kind::Ahead, Kind::Passed] {
                    assert!(!has(&sign(style, k, size), RING));
                }
            }
        }
        assert_eq!([Kind::ahead(0, 3), Kind::ahead(1, 3), Kind::ahead(2, 3), Kind::ahead(0, 1)], [Kind::Next, Kind::Ahead, Kind::Terminus, Kind::NextTerminus]);
    }

    /// A served stop is faded: greyer and darker than the same stop ahead, never see-through
    /// in parts (a shape over another would show through).
    #[test]
    fn a_served_stop_fades_into_the_map() {
        let chroma = |c: Color| c.0[..3].iter().cloned().fold(0.0, f32::max) - c.0[..3].iter().cloned().fold(1.0, f32::min);
        let light = |c: Color| c.0[0] + c.0[1] + c.0[2];
        for style in Style::ALL {
            let (ahead, passed) = (sign(style, Kind::Ahead, 14.0 * 0.8), sign(style, Kind::Passed, 14.0));
            assert_eq!(ahead.len(), passed.len(), "the same sign, {style:?}");
            for (a, p) in ahead.iter().zip(&passed) {
                if a.color == EDGE {
                    continue;
                }
                assert!(chroma(p.color) < chroma(a.color) || chroma(a.color) < 0.05, "{style:?}");
                assert!(light(p.color) < light(a.color) + 1e-4, "{style:?}");
                assert_eq!(p.color.0[3], a.color.0[3]);
            }
        }
    }

    /// No stroke of the small signs is thinner than a pixel: the H's bars and the ring of the
    /// German sign, the ring and the bar of the British.
    #[test]
    fn small_signs_keep_whole_pixel_strokes() {
        for size in [8.0, 10.0, 14.0] {
            for (style, ink) in [(Style::German, DE_GREEN), (Style::British, UK_RED)] {
                let shapes = sign(style, Kind::Ahead, size);
                let strokes: Vec<&Shape> = shapes.iter().filter(|s| s.color == ink && s.pts.len() == 4).collect();
                assert!(strokes.len() > 3, "{style:?} {size}");
                for b in strokes {
                    let w = (b.pts[1] - b.pts[0]).length().min((b.pts[2] - b.pts[1]).length());
                    assert!(w >= 0.999, "{style:?} {size}: {w}");
                }
            }
        }
    }

    /// The launcher's map stands the sign on its world point with north up: the bus's wheels
    /// (down on the screen) point south, and every corner is a pixel offset.
    #[test]
    fn the_world_sign_stands_the_right_way_up() {
        let size = 40.0;
        let mut flat = Painter::new();
        draw(&mut flat, Style::French, Kind::Ahead, Vec2::new(100.0, 50.0), size);
        let mut world = Painter::new();
        draw_world(&mut world, Style::French, Kind::Ahead, Vec3::new(7.0, 9.0, 0.0), size);
        assert_eq!(flat.verts.len(), world.verts.len());
        for (f, w) in flat.verts.iter().zip(&world.verts) {
            assert_eq!((w.pos, w.width, w.mode[0]), ([7.0, 9.0, 0.0], [0.0, 1.0], 1.0));
            assert!((f.pos[0] - 100.0 - w.ext[0]).abs() < 1e-4 && (f.pos[1] - 50.0 + w.ext[1]).abs() < 1e-4);
        }
        // the lowest white on the screen (the border's foot) is the southernmost in the world
        let low = flat.verts.iter().filter(|v| v.color == FR_WHITE.0).map(|v| v.pos[1]).fold(f32::MIN, f32::max);
        let south = world.verts.iter().filter(|v| v.color == FR_WHITE.0).map(|v| v.ext[1]).fold(f32::MAX, f32::min);
        assert!((low - 50.0 + south).abs() < 1e-4);
    }

    /// "BUS STOP" fits on the bar it is written on.
    #[test]
    fn the_letters_fit_the_bar() {
        assert_eq!(letters_width("BUS STOP"), 29);
        let r = 30.0;
        let mut out = Vec::new();
        letters(&mut out, "BUS STOP", Vec2::ZERO, r * 0.054, UK_WHITE);
        for p in out.iter().flat_map(|s| s.pts.iter()) {
            assert!(p.x.abs() <= UK_BAR.0 * r && p.y.abs() <= UK_BAR.1 * r, "{p}");
        }
        // B, U, S, S, T, O, P: every row of every letter has a cell
        assert!(out.len() >= 7 * 5);
    }

    #[test]
    fn the_setting_names_the_style() {
        assert_eq!(["de", "uk", "UK", "fr", "xx", ""].map(Style::from_setting), [Style::German, Style::British, Style::British, Style::French, Style::German, Style::German]);
        for s in Style::ALL {
            assert_eq!(Style::from_setting(s.key()), s);
        }
        let was = style();
        set_style(Style::French);
        assert_eq!(style(), Style::French);
        set_style(was);
    }

    /// A tag reads on its colour; the duty's own line is on the plate's yellow.
    #[test]
    fn line_chips_read_on_their_colour() {
        assert_eq!(chip_colours(Color::rgba(226, 58, 52, 1.0), false), (Color::rgba(226, 58, 52, 1.0), ON_DARK));
        assert_eq!(chip_colours(Color::rgba(46, 184, 92, 1.0), false).1, ON_PLATE);
        assert_eq!(chip_colours(Color::rgba(240, 190, 30, 1.0), false).1, ON_PLATE);
        assert_eq!(chip_colours(Color::rgba(226, 58, 52, 1.0), true), (crate::nav_duty::LINE, ON_PLATE));
        // the chip and its pointer stay within the room it was given and the dot below it
        let fonts = Fonts::hanken();
        let mut atlas = Atlas::new(512);
        let mut p = Painter::new();
        let r = Rect::new(40.0, 20.0, 30.0, 13.0);
        for own in [false, true] {
            draw_chip(&mut p, &mut atlas, &fonts, r, "301", 9.5, UK_RED, own, 1.0);
        }
        for v in &p.verts {
            assert!(v.pos[0] >= r.x - 1.01 && v.pos[0] <= r.right() + 1.01 && v.pos[1] >= r.y - 1.01 && v.pos[1] <= r.bottom() + 5.0, "{:?}", v.pos);
        }
        assert!(p.verts.iter().any(|v| v.color == UK_RED.lighten(0.1).0) && p.verts.iter().any(|v| v.color == crate::nav_duty::LINE.lighten(0.1).0));
    }

    /// The stops served on the trip under way, from the timetable's places or the map's, the
    /// stations passed without stopping left out.
    #[test]
    fn served_stops_are_found_where_they_stand() {
        let mut trip = crate::nav_duty::tests::trip("35", "Park", 600.0, &[("A", 0.0, true), ("Depot gate", 1.0, false), ("B", 3.0, true), ("C", 5.0, true), ("D", 7.0, true)]);
        trip.stops[0].position = Some(DVec3::new(1.0, 2.0, 0.0));
        let places: HashMap<i64, DVec3> = [(2, DVec3::new(5.0, 5.0, 0.0)), (1, DVec3::new(9.0, 9.0, 0.0))].into_iter().collect();
        assert_eq!(served_of(&trip, 3, &places), vec![DVec3::new(1.0, 2.0, 0.0), DVec3::new(5.0, 5.0, 0.0)]);
        assert!(served_of(&trip, 0, &places).is_empty());
        // (past the end: all of them, the one without a place left out)
        assert_eq!(served_of(&trip, 99, &places).len(), 2);
        assert!(served(None, &places).is_empty());
    }

    /// Pictures of the three styles at the navigator's sizes - the small map's, the city
    /// map's, a 4K city map's, and far out - in every kind, on the city map's ground with the
    /// route through them, with line tags and stop names:
    /// `OMSI_STOP_SIGNS_PREVIEW=<folder> cargo test -p omsi-app --lib stop_signs -- --ignored`.
    /// Drawn four times over and scaled down, as the navigator's 4x MSAA would.
    #[test]
    #[ignore]
    fn preview_pictures() {
        let Ok(dir) = std::env::var("OMSI_STOP_SIGNS_PREVIEW") else { return };
        let ss = 4.0;
        let fonts = Fonts::hanken();
        let mut atlas = Atlas::new(4096);
        let sizes: [(&str, f32); 4] = [("far", 6.0), ("small map", 13.5), ("city map", 15.0), ("4K city map", 26.0)];
        let (cw, rh) = (100.0f32, 96.0f32);
        let (w, h) = (40.0 + 150.0 + cw * Kind::ALL.len() as f32 * sizes.len() as f32 * 0.5, 60.0 + rh * 6.0 + 140.0);
        let mut img = image::RgbaImage::from_pixel((w * ss) as u32, (h * ss) as u32, image::Rgba([12, 16, 26, 255]));
        let mut p = Painter::new();
        let mut y = 60.0;
        for (row, style) in Style::ALL.iter().enumerate() {
            for half in 0..2 {
                let cy = y + rh * 0.5;
                p.text_in(&mut atlas, &fonts, &format!("{} {}", style.key(), if half == 0 { "far / small" } else { "city / 4K" }), 13.0 * ss, Weight::Bold, Rect::new(20.0 * ss, (cy - 10.0) * ss, 140.0 * ss, 20.0 * ss), Align::Left, LABEL_TEXT);
                // the route through the row: casing and blue
                p.line(Vec2::new(170.0 * ss, cy * ss), Vec2::new((w - 20.0) * ss, cy * ss), 9.0 * ss, Color::rgba(18, 58, 107, 1.0));
                p.line(Vec2::new(170.0 * ss, cy * ss), Vec2::new((w - 20.0) * ss, cy * ss), 5.0 * ss, Color::rgba(74, 144, 255, 1.0));
                let mut x = 200.0;
                for (_, size) in &sizes[half * 2..half * 2 + 2] {
                    let step = (size * 2.6).max(cw * 0.5);
                    for kind in Kind::ALL {
                        // (worked out at its own size - which parts it has - and only then
                        // drawn four times over)
                        for s in sign(*style, kind, *size) {
                            p.convex(&s.pts.iter().map(|q| (Vec2::new(x, cy) + *q) * ss).collect::<Vec<_>>(), s.color);
                        }
                        x += step;
                    }
                    x += 20.0;
                }
                y += rh;
            }
            let _ = row;
        }
        // a label of each style, and the line tags
        let mut x = 30.0;
        for (k, style) in Style::ALL.iter().enumerate() {
            let t = "Koenigsring  19:06";
            let tw = fonts.width(t, 12.5 * ss, if k == 0 { Weight::Bold } else { Weight::Medium });
            draw_label(&mut p, &mut atlas, &fonts, Rect::new(x * ss, (y + 20.0) * ss, tw + 14.0 * ss, 20.0 * ss), t, 12.5 * ss, *style, k == 0, ss);
            x += tw / ss + 40.0;
        }
        let mut x = 30.0;
        for (line, kind, own) in [("301", Color::rgba(226, 58, 52, 1.0), false), ("X", Color::rgba(226, 58, 52, 1.0), false), ("307", Color::rgba(226, 58, 52, 1.0), true), ("4", Color::rgba(240, 190, 30, 1.0), false), ("62", Color::rgba(46, 184, 92, 1.0), false)] {
            let px = 11.0 * ss;
            let tw = fonts.width(line, px, Weight::Bold) + CHIP_PAD * ss;
            let dot = Vec2::new(x + 20.0, y + 90.0) * ss;
            p.circle(dot, 3.4 * 1.4 * ss, Color::rgba(8, 8, 8, 0.9));
            p.circle(dot, 2.3 * 1.4 * ss, kind);
            let r = Rect::new(dot.x - tw * 0.5, dot.y - 22.0 * ss, tw, 15.0 * ss);
            draw_chip(&mut p, &mut atlas, &fonts, r, line, px, kind, own, ss);
            x += 60.0;
        }
        crate::nav_duty::tests::raster(&p.verts, &atlas, &mut img, Rect::new(0.0, 0.0, 1e6, 1e6));
        let small = image::imageops::resize(&img, w as u32, h as u32, image::imageops::FilterType::Triangle);
        small.save(format!("{dir}/stop_signs.png")).unwrap();
        // one sign of each, large, to look at its drawing
        let mut img = image::RgbaImage::from_pixel(3 * 160, 160, image::Rgba([12, 16, 26, 255]));
        let mut p = Painter::new();
        for (k, style) in Style::ALL.iter().enumerate() {
            draw(&mut p, *style, Kind::Ahead, Vec2::new(80.0 + 160.0 * k as f32, 80.0), 100.0);
        }
        crate::nav_duty::tests::raster(&p.verts, &atlas, &mut img, Rect::new(0.0, 0.0, 1e6, 1e6));
        img.save(format!("{dir}/stop_signs_large.png")).unwrap();
    }
}
